//! The control socket for the `impulse` command-line tool (the Swift
//! `ControlServer`): each connection carries one request line and gets one
//! response line, possibly much later for requests that wait (`impulse
//! edit`). Only the same user can connect (the socket is 0600, and the
//! peer's uid is checked). Every terminal gets `IMPULSE_SOCKET`,
//! `IMPULSE_PANE_TOKEN` and `IMPULSE_CLI`, so `impulse` finds its window
//! and pane; requests are answered on the main thread by the window that
//! holds the pane (`control_commands`).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use gpui_kit::App;
use impulse_protocol::control::{
    self, ControlRequest, ControlResponse, CLI_KEY, MALFORMED_REQUEST, MAX_REQUEST_LINE,
    REQUEST_DEADLINE, SOCKET_KEY, TOKEN_KEY,
};

static SOCKET: OnceLock<PathBuf> = OnceLock::new();

/// A request, who sent it, and where its answer goes.
pub struct Incoming {
    pub request: ControlRequest,
    /// The connected process, when the kernel says.
    pub caller: Option<i32>,
    pub reply: async_channel::Sender<ControlResponse>,
}

/// A fresh pane token.
pub fn new_token() -> String {
    let mut bytes = [0u8; 16];
    if getrandom(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        bytes.copy_from_slice(&nanos.to_le_bytes());
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn getrandom(bytes: &mut [u8]) -> std::io::Result<()> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")?.read_exact(bytes)
}

/// The `impulse` tool beside the app's own executable, if it's there.
pub(crate) fn cli_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join("impulse"), dir.join("bin").join("impulse")]
        .into_iter()
        .find(|p| p.is_file())
}

/// What a terminal with `token` gets in its environment. With
/// `editor_integration` (Use Impulse as $EDITOR), `git commit`, `crontab -e`
/// and the like open an Impulse tab and wait for it to close.
pub fn terminal_env(token: &str, editor_integration: bool) -> Vec<(String, String)> {
    let mut env = Vec::new();
    if let Some(socket) = SOCKET.get() {
        env.push((
            SOCKET_KEY.to_string(),
            socket.to_string_lossy().into_owned(),
        ));
        env.push((TOKEN_KEY.to_string(), token.to_string()));
    }
    if let Some(cli) = cli_path() {
        let folder = cli.parent().map(|p| p.to_string_lossy().into_owned());
        env.push((CLI_KEY.to_string(), cli.to_string_lossy().into_owned()));
        // `impulse` on PATH inside Impulse terminals.
        if let Some(folder) = folder {
            let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into());
            env.push(("PATH".into(), format!("{folder}:{path}")));
        }
        if editor_integration {
            // Run by `sh -c` (git, crontab), so POSIX single quotes.
            let editor = format!("'{}' edit", cli.to_string_lossy().replace('\'', "'\\''"));
            env.push(("EDITOR".into(), editor.clone()));
            env.push(("VISUAL".into(), editor));
        }
    }
    env
}

/// Whether a server answers at `path` (a stale socket refuses).
fn is_serving(path: &std::path::Path) -> bool {
    UnixStream::connect(path).is_ok()
}

/// Listen, and hand each request to `handle` on the main thread.
pub fn start(cx: &mut App, handle: impl Fn(Incoming, &mut App) + 'static) {
    let persistence = std::env::var_os("IMPULSE_NO_PERSISTENCE").is_none();
    let private = std::env::temp_dir().join(format!("impulse-{}.sock", std::process::id()));
    let mut path = if !persistence {
        private.clone()
    } else if cfg!(target_os = "macos") {
        impulse_kit::paths::data_dir(cfg!(debug_assertions)).join("impulse.sock")
    } else {
        // Linux keeps sockets in the runtime directory.
        let name = if cfg!(debug_assertions) {
            "impulse-dev.sock"
        } else {
            "impulse.sock"
        };
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|d| d.is_dir())
            .map(|d| d.join(name))
            .unwrap_or_else(|| private.clone())
    };
    if is_serving(&path) {
        // Another copy of the app answers there: leave its socket alone.
        log::info!("Impulse is already listening on {}", path.display());
        path = private.clone();
    }
    let _ = std::fs::remove_file(&path);
    // A path too long for a socket address (a deep home) falls back to a
    // short private one.
    let bound = UnixListener::bind(&path).or_else(|error| {
        log::warn!("Couldn't listen on {}: {error}", path.display());
        path = private.clone();
        let _ = std::fs::remove_file(&path);
        UnixListener::bind(&path)
    });
    let listener = match bound {
        Ok(listener) => listener,
        Err(error) => {
            log::error!("Couldn't listen on {}: {error}", path.display());
            return;
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    let _ = SOCKET.set(path);
    let (tx, rx) = async_channel::unbounded::<Incoming>();
    std::thread::Builder::new()
        .name("impulse-control".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let tx = tx.clone();
                let _ = std::thread::Builder::new()
                    .name("impulse-control-client".into())
                    .spawn(move || serve(stream, tx));
            }
        })
        .ok();
    cx.spawn(async move |cx| {
        while let Ok(incoming) = rx.recv().await {
            cx.update(|cx| handle(incoming, cx));
        }
    })
    .detach();
}

/// One client: read its line, wait for the answer, send it.
fn serve(stream: UnixStream, tx: async_channel::Sender<Incoming>) {
    let Some((uid, pid)) = peer(&stream) else {
        return;
    };
    // Only the same user (the socket is 0600 anyway).
    if uid != unsafe { libc::getuid() } {
        return;
    }
    let _ = stream.set_read_timeout(Some(REQUEST_DEADLINE));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(_) => return,
    };
    let send = |writer: &mut UnixStream, response: &ControlResponse| {
        let _ = writer.write_all(&control::encode_line(response));
    };
    let mut line = Vec::new();
    let mut reader = BufReader::new(stream.take_limit());
    match reader.read_until(b'\n', &mut line) {
        Ok(n) if n > 0 && line.ends_with(b"\n") => {}
        _ => return send(&mut writer, &ControlResponse::failure(MALFORMED_REQUEST)),
    }
    let Ok(request) = control::decode::<ControlRequest>(&line[..line.len() - 1]) else {
        return send(&mut writer, &ControlResponse::failure(MALFORMED_REQUEST));
    };
    let (reply_tx, reply_rx) = async_channel::bounded(1);
    if tx
        .send_blocking(Incoming {
            request,
            caller: pid,
            reply: reply_tx,
        })
        .is_err()
    {
        return;
    }
    if let Ok(response) = reply_rx.recv_blocking() {
        send(&mut writer, &response);
    }
}

trait TakeLimit {
    fn take_limit(self) -> std::io::Take<UnixStream>;
}

impl TakeLimit for UnixStream {
    /// Requests are one short line; past a megabyte, it isn't one.
    fn take_limit(self) -> std::io::Take<UnixStream> {
        use std::io::Read;
        self.take(MAX_REQUEST_LINE as u64)
    }
}

/// The peer's user id and process id.
fn peer(stream: &UnixStream) -> Option<(u32, Option<i32>)> {
    use std::os::fd::AsRawFd;
    let fd = stream.as_raw_fd();
    #[cfg(target_os = "linux")]
    {
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let ok = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut cred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        } == 0;
        ok.then_some((cred.uid, (cred.pid > 0).then_some(cred.pid)))
    }
    #[cfg(target_os = "macos")]
    {
        let (mut uid, mut gid) = (0, 0);
        if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {
            return None;
        }
        let mut pid: libc::pid_t = 0;
        let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
        let ok = unsafe {
            libc::getsockopt(
                fd,
                0, // SOL_LOCAL
                libc::LOCAL_PEERPID,
                &mut pid as *mut _ as *mut libc::c_void,
                &mut len,
            )
        } == 0;
        Some((uid, (ok && pid > 0).then_some(pid)))
    }
}

/// The parent of a process.
pub(crate) fn parent_of(pid: i32) -> Option<i32> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // "pid (comm) state ppid ...": comm may hold spaces and parens.
        let rest = &stat[stat.rfind(')')? + 2..];
        rest.split_whitespace().nth(1)?.parse().ok()
    }
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::kinfo_proc = unsafe { std::mem::zeroed() };
        let mut size = std::mem::size_of::<libc::kinfo_proc>();
        let mut mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, pid];
        let ok = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as u32,
                &mut info as *mut _ as *mut libc::c_void,
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        } == 0;
        (ok && size > 0).then_some(info.kp_eproc.e_ppid)
    }
}

/// Whether `caller` is `ancestor` or was started from it. A process that
/// left the tree (a tmux server, a daemon) isn't.
pub fn descends(caller: Option<i32>, ancestor: i32) -> bool {
    let Some(mut current) = caller else {
        return false;
    };
    if ancestor <= 1 {
        return false;
    }
    for _ in 0..64 {
        if current == ancestor {
            return true;
        }
        match parent_of(current) {
            Some(parent) if current > 1 && parent != current => current = parent,
            _ => return false,
        }
    }
    false
}

/// Remove the socket at quit, if it's still ours.
pub fn stop() {
    if let Some(path) = SOCKET.get() {
        let _ = std::fs::remove_file(path);
    }
}
