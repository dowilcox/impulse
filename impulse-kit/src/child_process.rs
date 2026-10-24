//! Runs a program to completion and collects its output (the Swift
//! `ChildProcess`).
//!
//! The child gets its own process group, so a timeout stops everything it
//! started (hooks, ssh, credential helpers, LFS) rather than only the
//! program itself. Both output pipes are drained while it runs, so a chatty
//! program can't fill a pipe and stall. Standard input is `/dev/null` unless
//! bytes are given for it. The standard library already opens every file
//! descriptor it makes close-on-exec and resets the signal mask and SIGPIPE
//! in the child.

use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How long the whole group gets between SIGTERM and SIGKILL.
const KILL_GRACE: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// Exit status, or 128 + the signal number when a signal ended it.
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
}

impl Output {
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

type ChunkHandler = Box<dyn FnMut(&[u8]) + Send>;

/// A program to run: `ChildProcess::new(path).args(...).timeout(...).run()`.
pub struct ChildProcess {
    command: Command,
    stdin: Option<Vec<u8>>,
    timeout: Option<Duration>,
    on_stderr: Option<ChunkHandler>,
}

impl ChildProcess {
    pub fn new(executable: impl AsRef<OsStr>) -> Self {
        ChildProcess {
            command: Command::new(executable),
            stdin: None,
            timeout: None,
            on_stderr: None,
        }
    }

    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.command.arg(arg);
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command.args(args);
        self
    }

    /// Where it runs (otherwise the app's working directory).
    pub fn dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.command.current_dir(dir);
        self
    }

    /// Set one variable on top of the environment it gets.
    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.command.env(key, value);
        self
    }

    pub fn envs<I, K, V>(mut self, vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.command.envs(vars);
        self
    }

    /// Start from an empty environment instead of the app's own.
    pub fn env_clear(mut self) -> Self {
        self.command.env_clear();
        self
    }

    /// Written to the program's standard input, which is otherwise
    /// `/dev/null`.
    pub fn stdin(mut self, input: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(input.into());
        self
    }

    /// Wall-clock limit. When it passes, the whole process group gets
    /// SIGTERM, then SIGKILL five seconds later.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn timeout_opt(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = timeout;
        self
    }

    /// Each chunk of standard error as it arrives (on a background thread),
    /// e.g. for progress lines.
    pub fn on_stderr(mut self, handler: impl FnMut(&[u8]) + Send + 'static) -> Self {
        self.on_stderr = Some(Box::new(handler));
        self
    }

    /// Run it to the end (or the timeout). Fails only when it can't start.
    pub fn run(self) -> io::Result<Output> {
        let ChildProcess {
            mut command,
            stdin,
            timeout,
            on_stderr,
        } = self;
        command
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);

        let mut child = command.spawn()?;
        let pid = child.id();
        let out = Drain::start(child.stdout.take().expect("piped stdout"), None);
        let err = Drain::start(child.stderr.take().expect("piped stderr"), on_stderr);
        if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
            // The program may exit without reading it; the write then fails
            // (Rust ignores SIGPIPE), and that's fine.
            thread::spawn(move || {
                let _ = pipe.write_all(&input);
            });
        }

        let (exited_tx, exited) = mpsc::channel();
        thread::spawn(move || {
            let _ = exited_tx.send(child.wait());
        });

        let mut timed_out = false;
        let status = match timeout {
            None => exited.recv(),
            Some(limit) => match exited.recv_timeout(limit) {
                Ok(status) => Ok(status),
                Err(_) => {
                    timed_out = true;
                    signal_group(pid, Signal::Terminate);
                    match exited.recv_timeout(KILL_GRACE) {
                        Ok(status) => Ok(status),
                        Err(_) => {
                            signal_group(pid, Signal::Kill);
                            exited.recv()
                        }
                    }
                }
            },
        };
        let status = status
            .map_err(|_| io::Error::other("lost the child process"))
            .and_then(|s| s)?;

        // Output ends when every holder of the pipes has exited. Something
        // that left the process group (a daemonized ssh master) can keep
        // them open; don't wait for it.
        let deadline = Instant::now()
            + if timed_out {
                Duration::from_secs(1)
            } else {
                Duration::from_secs(5)
            };
        out.wait_until(deadline);
        err.wait_until(deadline);
        Ok(Output {
            status: decode(status),
            stdout: out.finish(),
            stderr: err.finish(),
            timed_out,
        })
    }
}

/// Exit code, or 128 + signal for a program a signal ended.
fn decode(status: std::process::ExitStatus) -> i32 {
    #[cfg(unix)]
    if let Some(signal) = std::os::unix::process::ExitStatusExt::signal(&status) {
        return 128 + signal;
    }
    status.code().unwrap_or(-1)
}

enum Signal {
    Terminate,
    Kill,
}

#[cfg(unix)]
fn signal_group(pid: u32, signal: Signal) {
    let signal = match signal {
        Signal::Terminate => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    };
    // SAFETY: kill only sends a signal; a negative pid names the group.
    unsafe {
        libc::kill(-(pid as libc::pid_t), signal);
    }
}

#[cfg(not(unix))]
fn signal_group(_pid: u32, _signal: Signal) {}

#[derive(Default)]
struct DrainState {
    data: Vec<u8>,
    stopped: bool,
}

/// Reads a pipe until it closes, on a thread of its own.
struct Drain {
    state: Arc<Mutex<DrainState>>,
    done: Receiver<()>,
}

impl Drain {
    fn start(mut pipe: impl Read + Send + 'static, mut on_chunk: Option<ChunkHandler>) -> Drain {
        let state = Arc::new(Mutex::new(DrainState::default()));
        let (done_tx, done) = mpsc::channel();
        let shared = state.clone();
        thread::spawn(move || {
            let mut buffer = vec![0u8; 64 * 1024];
            loop {
                let count = match pipe.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => count,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                };
                let chunk = &buffer[..count];
                {
                    let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
                    if state.stopped {
                        break;
                    }
                    state.data.extend_from_slice(chunk);
                }
                if let Some(handler) = on_chunk.as_mut() {
                    handler(chunk);
                }
            }
            let _ = done_tx.send(());
        });
        Drain { state, done }
    }

    /// Wait for the pipe to close, until `deadline`. A reader still going
    /// then is left to end on its own when the pipe finally closes.
    fn wait_until(&self, deadline: Instant) {
        let left = deadline.saturating_duration_since(Instant::now());
        let _ = self.done.recv_timeout(left);
    }

    /// What was read so far; nothing more is collected after this.
    fn finish(self) -> Vec<u8> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.stopped = true;
        std::mem::take(&mut state.data)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn sh(script: &str) -> ChildProcess {
        ChildProcess::new("/bin/sh").args(["-c", script])
    }

    #[test]
    fn collects_output_and_status() {
        let output = sh("echo out; echo err >&2; exit 3").run().unwrap();
        assert_eq!(output.stdout, b"out\n");
        assert_eq!(output.stderr, b"err\n");
        assert_eq!(output.status, 3);
        assert!(!output.timed_out);
    }

    #[test]
    fn a_signal_ending_the_program_shows_in_the_status() {
        assert_eq!(
            sh("kill -TERM $$").run().unwrap().status,
            128 + libc::SIGTERM
        );
    }

    #[test]
    fn feeds_standard_input() {
        let input: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let output = ChildProcess::new("/bin/cat")
            .stdin(input.clone())
            .run()
            .unwrap();
        assert_eq!(output.stdout, input);
    }

    #[test]
    fn standard_input_is_empty_otherwise() {
        let output = ChildProcess::new("/bin/cat")
            .timeout(Duration::from_secs(5))
            .run()
            .unwrap();
        assert!(output.stdout.is_empty());
    }

    #[test]
    fn lots_of_output_on_both_streams_does_not_stall() {
        let output = sh("head -c 300000 /dev/zero; head -c 300000 /dev/zero >&2")
            .timeout(Duration::from_secs(20))
            .run()
            .unwrap();
        assert_eq!(output.stdout.len(), 300_000);
        assert_eq!(output.stderr.len(), 300_000);
        assert!(!output.timed_out);
    }

    #[test]
    fn runs_in_the_given_directory() {
        let dir = tempfile::tempdir().unwrap();
        let output = ChildProcess::new("/bin/pwd")
            .arg("-P")
            .dir(dir.path())
            .run()
            .unwrap();
        let expected = std::fs::canonicalize(dir.path()).unwrap();
        assert_eq!(
            output.stdout_text(),
            format!("{}\n", expected.to_str().unwrap())
        );
    }

    #[test]
    fn a_timeout_stops_everything_the_program_started() {
        let started = Instant::now();
        // The background sleep is a grandchild; killing only sh would leave it.
        let output = sh("sleep 30 & echo $!; wait")
            .timeout(Duration::from_millis(500))
            .run()
            .unwrap();
        assert!(output.timed_out);
        assert!(started.elapsed() < Duration::from_secs(5));
        let grandchild: libc::pid_t = output.stdout_text().trim().parse().unwrap_or(0);
        assert!(grandchild > 0);
        // Gone, or a zombie its new parent hasn't reaped yet.
        let gone = || {
            // SAFETY: signal 0 only checks that the process exists.
            let missing = unsafe { libc::kill(grandchild, 0) } == -1
                && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
            missing || is_zombie(grandchild)
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        while !gone() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(gone(), "the grandchild is gone too");
    }

    /// Whether `pid` has exited but not been reaped (Linux; elsewhere the
    /// check above has to see it gone).
    fn is_zombie(pid: libc::pid_t) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| {
                let state = stat.rsplit_once(") ")?.1.chars().next()?;
                Some(state == 'Z')
            })
            .unwrap_or(false)
    }

    #[test]
    fn a_daemon_holding_the_pipe_does_not_hold_up_the_result() {
        if !Path::new("/usr/bin/perl").exists() {
            return;
        }
        let started = Instant::now();
        // perl leaves the process group and keeps stdout open after sh exits.
        let output = sh("perl -e 'setpgrp(0, 0); sleep 8' & echo started")
            .run()
            .unwrap();
        assert_eq!(output.stdout_text(), "started\n");
        assert!(started.elapsed() < Duration::from_millis(7500));
    }

    #[test]
    fn a_missing_program_fails() {
        assert!(ChildProcess::new("/nonexistent/tool").run().is_err());
    }

    #[test]
    fn standard_error_streams_as_it_arrives() {
        let chunks = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
        let collected = chunks.clone();
        ChildProcess::new("/bin/sh")
            .args(["-c", "echo one >&2; sleep 0.2; echo two >&2"])
            .timeout(Duration::from_secs(5))
            .on_stderr(move |chunk| collected.lock().unwrap().push(chunk.to_vec()))
            .run()
            .unwrap();
        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.concat(), b"one\ntwo\n");
        assert!(chunks.len() >= 2);
    }
}
