//! The system's display options the app follows, as the Swift app does:
//! Increase Contrast strengthens the chrome's own colors
//! (`ChromePalette::new`'s `increase_contrast`; Swift's `ChromePalette`
//! reads `accessibilityDisplayShouldIncreaseContrast` and re-applies the
//! theme when it changes), and Reduce Motion stops spinners
//! (`App::reduce_motion`). On macOS they're the Accessibility display
//! options; on Linux, GNOME's high-contrast setting and its animations
//! switch, read with `gsettings`. Where neither is there they're off.
//!
//! Windows observe the `IncreaseContrast` global to rebuild their palette.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::{App, Global};

pub struct IncreaseContrast(pub bool);

impl Global for IncreaseContrast {}

/// Whether the chrome should use its stronger colors.
pub fn increase_contrast(cx: &App) -> bool {
    cx.try_global::<IncreaseContrast>().is_some_and(|c| c.0)
}

/// Reduce Motion, for drawing code without the app at hand.
static REDUCE_MOTION: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether spinners and other non-essential animation stay still.
pub fn reduce_motion() -> bool {
    REDUCE_MOTION.load(Ordering::Relaxed)
}

/// The two options, as bits (1: increase contrast, 2: reduce motion).
#[derive(Clone, Copy, PartialEq)]
struct Options {
    increase_contrast: bool,
    reduce_motion: bool,
}

impl Options {
    fn bits(self) -> u8 {
        u8::from(self.increase_contrast) | (u8::from(self.reduce_motion) << 1)
    }

    fn from_bits(bits: u8) -> Self {
        Options {
            increase_contrast: bits & 1 != 0,
            reduce_motion: bits & 2 != 0,
        }
    }
}

/// Read the options and follow their changes.
pub fn init(cx: &mut App) {
    let Some(options) = read() else {
        cx.set_global(IncreaseContrast(false));
        return;
    };
    apply(options, cx);
    // Followed off the main thread, checked here once a second.
    let shared = Arc::new(AtomicU8::new(options.bits()));
    follow(shared.clone());
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(Duration::from_secs(1)).await;
        let options = Options::from_bits(shared.load(Ordering::Relaxed));
        cx.update(|cx| apply(options, cx));
    })
    .detach();
}

fn apply(options: Options, cx: &mut App) {
    if increase_contrast(cx) != options.increase_contrast || !cx.has_global::<IncreaseContrast>() {
        cx.set_global(IncreaseContrast(options.increase_contrast));
    }
    if cx.reduce_motion() != options.reduce_motion {
        REDUCE_MOTION.store(options.reduce_motion, Ordering::Relaxed);
        cx.set_reduce_motion(options.reduce_motion);
        cx.refresh_windows();
    }
}

/// Poll AppKit's flags (cheap property reads).
#[cfg(target_os = "macos")]
fn follow(shared: Arc<AtomicU8>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        if let Some(options) = read() {
            shared.store(options.bits(), Ordering::Relaxed);
        }
    });
}

/// One `gsettings monitor` per key, each printing a line as its value
/// changes (rather than starting two processes every few seconds). They
/// end with the app.
#[cfg(not(target_os = "macos"))]
fn follow(shared: Arc<AtomicU8>) {
    use std::io::BufRead as _;
    use std::os::unix::process::CommandExt as _;
    let keys = [
        (
            "org.gnome.desktop.a11y.interface",
            "high-contrast",
            1u8,
            false,
        ),
        (
            "org.gnome.desktop.interface",
            "enable-animations",
            2u8,
            true,
        ),
    ];
    for (schema, key, bit, inverted) in keys {
        let shared = shared.clone();
        std::thread::spawn(move || {
            let mut command = std::process::Command::new("gsettings");
            command
                .args(["monitor", schema, key])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null());
            #[cfg(target_os = "linux")]
            // SAFETY: prctl is async-signal-safe; it only asks for SIGTERM
            // when the app (the parent) exits.
            unsafe {
                command.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
            let Ok(mut child) = command.spawn() else {
                return;
            };
            let Some(stdout) = child.stdout.take() else {
                return;
            };
            // "high-contrast: true"
            for line in std::io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
            {
                let on = line.rsplit(':').next().map(str::trim) == Some("true");
                let set = on != inverted;
                let _ = shared.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |bits| {
                    Some(if set { bits | bit } else { bits & !bit })
                });
            }
            let _ = child.wait();
        });
    }
}

/// The options now, or `None` where the system has none to read.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
fn read() -> Option<Options> {
    use objc::runtime::{Class, Object, BOOL, YES};
    use objc::{msg_send, sel, sel_impl};
    let class = Class::get("NSWorkspace")?;
    // SAFETY: `sharedWorkspace` and the BOOL properties are plain getters
    // on AppKit's shared object, which lives for the whole process.
    unsafe {
        let workspace: *mut Object = msg_send![class, sharedWorkspace];
        if workspace.is_null() {
            return None;
        }
        let contrast: BOOL = msg_send![workspace, accessibilityDisplayShouldIncreaseContrast];
        let motion: BOOL = msg_send![workspace, accessibilityDisplayShouldReduceMotion];
        Some(Options {
            increase_contrast: contrast == YES,
            reduce_motion: motion == YES,
        })
    }
}

#[cfg(not(target_os = "macos"))]
fn read() -> Option<Options> {
    let get = |schema: &str, key: &str| -> Option<bool> {
        let output = std::process::Command::new("gsettings")
            .args(["get", schema, key])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim() == "true")
    };
    let contrast = get("org.gnome.desktop.a11y.interface", "high-contrast");
    let animations = get("org.gnome.desktop.interface", "enable-animations");
    if contrast.is_none() && animations.is_none() {
        return None;
    }
    Some(Options {
        increase_contrast: contrast.unwrap_or(false),
        reduce_motion: animations.is_some_and(|on| !on),
    })
}
