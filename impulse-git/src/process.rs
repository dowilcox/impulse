//! Running a program to completion and collecting its output (the Swift
//! `ChildProcess`, which lives in ImpulseKit there; so far only the git layer
//! needs it in Rust).
//!
//! The child gets a process group of its own, so a timeout stops everything
//! it started (hooks, ssh, credential helpers, LFS) rather than only the
//! program itself: SIGTERM to the group, then SIGKILL five seconds later.
//! Both output pipes are drained while it runs, so a chatty program can't fill
//! a pipe and stall, and standard input (/dev/null unless bytes are given) is
//! written from a thread of its own.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// What a finished program left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    /// Exit status, or 128 + the signal number when a signal ended it.
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
}

/// Called with each chunk of standard error.
pub type OnStderr<'a> = &'a mut dyn FnMut(&[u8]);

#[derive(Default)]
pub struct Options<'a> {
    pub directory: Option<&'a str>,
    /// The complete environment (`None`: this process's own).
    pub environment: Option<&'a HashMap<String, String>>,
    pub stdin: Option<&'a [u8]>,
    /// Wall-clock limit (`None`: none).
    pub timeout: Option<Duration>,
    /// Each chunk of standard error as it arrives (on the calling thread),
    /// e.g. for progress lines.
    pub on_stderr: Option<OnStderr<'a>>,
}

enum Event {
    Stderr(Vec<u8>),
    StdoutDone,
    StderrDone,
    Exited(io::Result<ExitStatus>),
}

pub fn run<S: AsRef<str>>(
    executable: &str,
    arguments: &[S],
    options: Options,
) -> io::Result<Output> {
    let Options {
        directory,
        environment,
        stdin,
        timeout,
        mut on_stderr,
    } = options;

    let mut command = Command::new(executable);
    command
        .args(arguments.iter().map(AsRef::as_ref))
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    if let Some(environment) = environment {
        command.env_clear().envs(environment);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn()?;
    let pid = child.id();

    let (events, received) = mpsc::channel();
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stderr = Arc::new(Mutex::new(Vec::new()));
    if let Some(pipe) = child.stdout.take() {
        drain(pipe, stdout.clone(), events.clone(), false);
    } else {
        let _ = events.send(Event::StdoutDone);
    }
    if let Some(pipe) = child.stderr.take() {
        drain(pipe, stderr.clone(), events.clone(), true);
    } else {
        let _ = events.send(Event::StderrDone);
    }
    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        // The program may exit without reading it; the write then fails
        // (Rust programs ignore SIGPIPE), which is fine.
        let bytes = bytes.to_vec();
        thread::spawn(move || {
            let _ = pipe.write_all(&bytes);
        });
    }
    let waiter = events.clone();
    thread::spawn(move || {
        let _ = waiter.send(Event::Exited(child.wait()));
    });
    drop(events);

    let mut status = None;
    let mut timed_out = false;
    let mut stdout_done = false;
    let mut stderr_done = false;
    let mut deadline = timeout.map(|limit| Instant::now() + limit);
    let mut handle = |event: Event,
                      status: &mut Option<io::Result<ExitStatus>>,
                      stdout_done: &mut bool,
                      stderr_done: &mut bool| match event {
        Event::Stderr(chunk) => {
            if let Some(callback) = on_stderr.as_mut() {
                callback(&chunk);
            }
        }
        Event::StdoutDone => *stdout_done = true,
        Event::StderrDone => *stderr_done = true,
        Event::Exited(result) => *status = Some(result),
    };

    while status.is_none() {
        let next = match deadline {
            Some(at) => received.recv_timeout(at.saturating_duration_since(Instant::now())),
            None => received.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match next {
            Ok(event) => handle(event, &mut status, &mut stdout_done, &mut stderr_done),
            Err(RecvTimeoutError::Timeout) if !timed_out => {
                timed_out = true;
                signal_group(pid, libc::SIGTERM);
                deadline = Some(Instant::now() + Duration::from_secs(5));
            }
            Err(RecvTimeoutError::Timeout) => {
                signal_group(pid, libc::SIGKILL);
                deadline = None;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    // Output ends when every holder of the pipes has exited. Something that
    // left the process group (a daemonized ssh master) can keep them open;
    // don't wait for it.
    let drained = Instant::now() + Duration::from_secs(if timed_out { 1 } else { 5 });
    while !(stdout_done && stderr_done) {
        match received.recv_timeout(drained.saturating_duration_since(Instant::now())) {
            Ok(event) => handle(event, &mut status, &mut stdout_done, &mut stderr_done),
            Err(_) => break,
        }
    }

    let status = match status {
        Some(result) => decode(result?),
        None => return Err(io::Error::other("the program's exit was never seen")),
    };
    let stdout = std::mem::take(&mut *stdout.lock().unwrap());
    let stderr = std::mem::take(&mut *stderr.lock().unwrap());
    Ok(Output {
        status,
        stdout,
        stderr,
        timed_out,
    })
}

/// Read `pipe` until it closes into `buffer`, telling `events` about each
/// chunk of standard error and about the end.
fn drain(
    mut pipe: impl Read + Send + 'static,
    buffer: Arc<Mutex<Vec<u8>>>,
    events: mpsc::Sender<Event>,
    is_stderr: bool,
) {
    thread::spawn(move || {
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => {
                    buffer.lock().unwrap().extend_from_slice(&chunk[..count]);
                    if is_stderr {
                        let _ = events.send(Event::Stderr(chunk[..count].to_vec()));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        let _ = events.send(if is_stderr {
            Event::StderrDone
        } else {
            Event::StdoutDone
        });
    });
}

#[cfg(unix)]
fn signal_group(pid: u32, signal: libc::c_int) {
    // SAFETY: kill(2) on our own child's process group; it has no memory
    // effects in this process.
    unsafe {
        libc::kill(-(pid as libc::pid_t), signal);
    }
}

#[cfg(not(unix))]
fn signal_group(_pid: u32, _signal: i32) {}

/// Exit code, or 128 + signal for a program a signal ended.
fn decode(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    -1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_output_status_and_stdin() {
        let output = run(
            "/bin/sh",
            &["-c", "cat; echo err >&2; exit 3"],
            Options {
                stdin: Some(b"hello"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(output.status, 3);
        assert_eq!(output.stdout, b"hello");
        assert_eq!(output.stderr, b"err\n");
        assert!(!output.timed_out);
    }

    #[test]
    fn a_timeout_stops_the_whole_group() {
        let started = Instant::now();
        let output = run(
            "/bin/sh",
            &["-c", "sleep 30 & sleep 30"],
            Options {
                timeout: Some(Duration::from_millis(200)),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(output.timed_out);
        assert_eq!(output.status, 128 + libc::SIGTERM);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn large_output_does_not_stall() {
        let output = run(
            "/bin/sh",
            &[
                "-c",
                "head -c 1000000 /dev/zero; head -c 300000 /dev/zero >&2",
            ],
            Options::default(),
        )
        .unwrap();
        assert_eq!(output.stdout.len(), 1_000_000);
        assert_eq!(output.stderr.len(), 300_000);
    }
}
