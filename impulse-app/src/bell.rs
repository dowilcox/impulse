//! The audible bell (the Swift app's `NSSound.beep()`): the system alert on
//! macOS; on Linux the desktop's bell sound, through libcanberra's player
//! when it's installed, else PulseAudio's or PipeWire's with the
//! freedesktop sound theme's bell. Nothing when neither is there.

/// Ring the bell once (the caller limits how often).
pub fn ring() {
    #[cfg(target_os = "macos")]
    {
        #[link(name = "AppKit", kind = "framework")]
        extern "C" {
            fn NSBeep();
        }
        // SAFETY: NSBeep takes no arguments and only plays the alert sound.
        unsafe { NSBeep() };
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some((program, args)) = player() {
            let _ = std::process::Command::new(program)
                .args(args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map(|mut child| {
                    // Reaped off the main thread.
                    std::thread::spawn(move || child.wait());
                });
        }
    }
}

/// The program that plays the bell, and its arguments; looked for once.
#[cfg(not(target_os = "macos"))]
fn player() -> Option<&'static (String, Vec<String>)> {
    use std::sync::OnceLock;
    static PLAYER: OnceLock<Option<(String, Vec<String>)>> = OnceLock::new();
    PLAYER
        .get_or_init(|| {
            let on_path = |name: &str| {
                std::env::var_os("PATH").and_then(|path| {
                    std::env::split_paths(&path)
                        .map(|dir| dir.join(name))
                        .find(|p| p.is_file())
                })
            };
            if let Some(canberra) = on_path("canberra-gtk-play") {
                return Some((
                    canberra.to_string_lossy().into_owned(),
                    vec!["--id=bell".into(), "--description=Impulse".into()],
                ));
            }
            let sound = [
                "/usr/share/sounds/freedesktop/stereo/bell.oga",
                "/usr/share/sounds/freedesktop/stereo/dialog-warning.oga",
            ]
            .into_iter()
            .find(|p| std::path::Path::new(p).is_file())?;
            ["paplay", "pw-play"].into_iter().find_map(|name| {
                on_path(name)
                    .map(|program| (program.to_string_lossy().into_owned(), vec![sound.into()]))
            })
        })
        .as_ref()
}
