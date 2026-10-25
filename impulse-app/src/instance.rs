//! One app per user, as macOS gives the Swift app: a second launch on Linux
//! hands its folder, files and commands to the running one (over a socket
//! in `$XDG_RUNTIME_DIR`) and exits, and files the system asks to open
//! (macOS's open-URLs, the desktop entry's `%F`) open in the front window.

use std::path::PathBuf;
use std::sync::OnceLock;

use gpui_kit::App;

/// What a launch asks for.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Request {
    pub cwd: Option<String>,
    pub open: Vec<String>,
    pub commands: Vec<String>,
}

static REQUESTS: OnceLock<async_channel::Sender<Request>> = OnceLock::new();

/// Queue a request for the running app (from any thread).
pub fn send(request: Request) {
    if let Some(sender) = REQUESTS.get() {
        let _ = sender.try_send(request);
    }
}

/// Carry out queued requests in the front window, bringing it forward.
pub fn serve(cx: &mut App) {
    let (sender, receiver) = async_channel::unbounded::<Request>();
    if REQUESTS.set(sender).is_err() {
        return;
    }
    cx.spawn(async move |cx| {
        while let Ok(request) = receiver.recv().await {
            let _ = cx.update(|cx| {
                let front = cx.active_window();
                let workbenches = crate::session::workbenches(cx);
                let Some((workbench, handle)) = workbenches
                    .iter()
                    .find(|(_, h)| Some(*h) == front)
                    .or(workbenches.first())
                    .cloned()
                else {
                    return;
                };
                let _ = handle.update(cx, |_, window, cx| {
                    window.activate_window();
                    workbench.update(cx, |workbench, cx| {
                        workbench.apply_launch(
                            request.cwd.clone(),
                            request.commands.clone(),
                            request.open.clone(),
                            window,
                            cx,
                        )
                    })
                });
                cx.activate(true);
            });
        }
    })
    .detach();
}

/// The socket the first instance listens on: in the user's own runtime
/// folder only (never a shared temp folder).
#[cfg(target_os = "linux")]
fn socket_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_dir())?;
    let dir = runtime.join("impulse");
    let mut builder = std::fs::DirBuilder::new();
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.recursive(true);
    builder.create(&dir).ok()?;
    let name = if cfg!(debug_assertions) {
        "instance-dev.sock"
    } else {
        "instance.sock"
    };
    Some(dir.join(name))
}

/// Hand `request` to an Impulse already running; true when one took it.
#[cfg(target_os = "linux")]
pub fn forward(request: &Request) -> bool {
    use std::io::{Read, Write};
    let Some(path) = socket_path() else {
        return false;
    };
    let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&path) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    let Ok(mut line) = serde_json::to_vec(request) else {
        return false;
    };
    line.push(b'\n');
    if stream.write_all(&line).is_err() {
        return false;
    }
    let mut ack = [0u8; 2];
    stream.read_exact(&mut ack).is_ok() && &ack == b"ok"
}

/// Take later launches' requests (`forward`) for this app.
#[cfg(target_os = "linux")]
pub fn listen() {
    use std::io::{BufRead, Write};
    let Some(path) = socket_path() else {
        return;
    };
    // Nothing answered `forward`: a socket left there is stale.
    let _ = std::fs::remove_file(&path);
    let Ok(listener) = std::os::unix::net::UnixListener::bind(&path) else {
        return;
    };
    std::thread::Builder::new()
        .name("impulse-instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = std::io::BufReader::new(&stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                if let Ok(request) = serde_json::from_str::<Request>(&line) {
                    send(request);
                    let _ = (&stream).write_all(b"ok");
                }
            }
        })
        .ok();
}

/// Stop listening (at quit), so the next launch starts afresh.
#[cfg(target_os = "linux")]
pub fn stop() {
    if let Some(path) = socket_path() {
        let _ = std::fs::remove_file(path);
    }
}
