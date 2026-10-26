//! Impulse's notification sound: a soft two-note chime
//! (`assets/sounds/notification.wav`, made by
//! `scripts/sounds/make-notification-sound.py`), compiled in so no install
//! step has to copy it. `agent_notify::post` plays it with each desktop
//! notification while `notification_sound` is on, in place of the
//! desktop's own sound: NSSound from memory on macOS; on Linux PulseAudio's
//! or PipeWire's player, else libcanberra's, from a copy in the cache
//! folder. Nothing when none of them is there.

/// The sound: 16-bit mono PCM WAV, peaking at -3 dBFS.
pub(crate) const WAV: &[u8] = include_bytes!("../../assets/sounds/notification.wav");

/// The playback volume (0 to 1). The file is normalized to -3 dBFS; at
/// 0.2 (-14 dB) it sits level with the system's alert sounds.
const VOLUME: f32 = 0.2;

/// Play the sound once, without waiting for it. On the main thread on
/// macOS.
pub(crate) fn play() {
    #[cfg(target_os = "macos")]
    mac::play();
    #[cfg(not(target_os = "macos"))]
    std::thread::spawn(|| {
        let Some(file) = cached_file() else {
            return;
        };
        let Some((program, args)) = player(&file) else {
            return;
        };
        let _ = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
mod mac {
    use objc::runtime::{Class, Object, BOOL, NO};
    use objc::{msg_send, sel, sel_impl};

    thread_local! {
        /// The NSSound, made once (retained for the run).
        static SOUND: *mut Object = make();
    }

    fn make() -> *mut Object {
        let (Some(data_class), Some(sound_class)) = (Class::get("NSData"), Class::get("NSSound"))
        else {
            return std::ptr::null_mut();
        };
        // SAFETY: NSData over the static bytes (never freed, so no copy),
        // and an NSSound made from it; both stay alive for the run.
        unsafe {
            let data: *mut Object = msg_send![data_class,
                dataWithBytesNoCopy: super::WAV.as_ptr() as *mut std::ffi::c_void
                length: super::WAV.len()
                freeWhenDone: NO];
            if data.is_null() {
                return std::ptr::null_mut();
            }
            let sound: *mut Object = msg_send![sound_class, alloc];
            let sound: *mut Object = msg_send![sound, initWithData: data];
            if !sound.is_null() {
                let _: () = msg_send![sound, setVolume: super::VOLUME];
            }
            sound
        }
    }

    pub(super) fn play() {
        SOUND.with(|&sound| {
            if sound.is_null() {
                return;
            }
            // SAFETY: the NSSound made above, on the thread that made it.
            // One still ringing starts over rather than overlapping.
            unsafe {
                let playing: BOOL = msg_send![sound, isPlaying];
                if playing != NO {
                    let _: BOOL = msg_send![sound, stop];
                }
                let _: BOOL = msg_send![sound, play];
            }
        });
    }
}

/// The sound written to `<cache>/impulse/notification.wav` for players that
/// take a file (once a run, and only when it differs).
#[cfg(not(target_os = "macos"))]
fn cached_file() -> Option<std::path::PathBuf> {
    use std::sync::OnceLock;
    static FILE: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    FILE.get_or_init(|| {
        let dir = cache_dir(std::env::var_os("XDG_CACHE_HOME"), std::env::var_os("HOME"))?
            .join("impulse");
        let file = dir.join("notification.wav");
        if std::fs::read(&file).is_ok_and(|bytes| bytes == WAV) {
            return Some(file);
        }
        std::fs::create_dir_all(&dir).ok()?;
        let partial = dir.join(format!("notification.wav.{}", std::process::id()));
        std::fs::write(&partial, WAV).ok()?;
        std::fs::rename(&partial, &file)
            .inspect_err(|_| {
                let _ = std::fs::remove_file(&partial);
            })
            .ok()?;
        Some(file)
    })
    .clone()
}

/// The user's cache folder: `$XDG_CACHE_HOME` when it's absolute, else
/// `~/.cache`.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn cache_dir(
    xdg_cache_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    xdg_cache_home
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| {
            home.map(PathBuf::from)
                .filter(|dir| dir.is_absolute())
                .map(|home| home.join(".cache"))
        })
}

/// The first player on `PATH`, and its arguments for `file`.
#[cfg(not(target_os = "macos"))]
fn player(file: &std::path::Path) -> Option<(std::path::PathBuf, Vec<String>)> {
    let path = std::env::var_os("PATH")?;
    ["paplay", "pw-play", "canberra-gtk-play"]
        .into_iter()
        .find_map(|name| {
            let program = std::env::split_paths(&path)
                .map(|dir| dir.join(name))
                .find(|p| p.is_file())?;
            Some((program, player_arguments(name, &file.to_string_lossy())))
        })
}

/// A player's arguments: the file, at `VOLUME`.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn player_arguments(player: &str, file: &str) -> Vec<String> {
    match player {
        // PulseAudio's volume is cubic (its gain is (v / 65536)³).
        "paplay" => vec![
            format!("--volume={}", (VOLUME.cbrt() * 65536.0).round() as u32),
            "--client-name=Impulse".into(),
            file.into(),
        ],
        // PipeWire's stream volume is linear.
        "pw-play" => vec![format!("--volume={VOLUME}"), file.into()],
        // libcanberra's is in decibels.
        _ => vec![
            format!("--file={file}"),
            format!("--volume={:.1}", 20.0 * VOLUME.log10()),
            "--description=Impulse".into(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `fmt ` and `data` chunks of a RIFF WAVE file.
    fn chunks(wav: &[u8]) -> (Option<&[u8]>, Option<&[u8]>) {
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        let riff_len = u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff_len + 8, wav.len(), "the RIFF length covers the file");
        let (mut fmt, mut data) = (None, None);
        let mut at = 12;
        while at + 8 <= wav.len() {
            let id = &wav[at..at + 4];
            let len = u32::from_le_bytes(wav[at + 4..at + 8].try_into().unwrap()) as usize;
            let body = &wav[at + 8..at + 8 + len];
            match id {
                b"fmt " => fmt = Some(body),
                b"data" => data = Some(body),
                _ => {}
            }
            at += 8 + len + len % 2;
        }
        (fmt, data)
    }

    #[test]
    fn the_sound_is_a_short_quiet_chime() {
        let (fmt, data) = chunks(WAV);
        let fmt = fmt.expect("a fmt chunk");
        let field = |at: usize| u16::from_le_bytes(fmt[at..at + 2].try_into().unwrap());
        let rate = u32::from_le_bytes(fmt[4..8].try_into().unwrap());
        assert_eq!(field(0), 1, "PCM");
        assert_eq!(field(2), 1, "mono");
        assert_eq!(rate, 44_100);
        assert_eq!(field(14), 16, "16-bit");

        let samples: Vec<i16> = data
            .expect("a data chunk")
            .chunks_exact(2)
            .map(|s| i16::from_le_bytes([s[0], s[1]]))
            .collect();
        let seconds = samples.len() as f64 / rate as f64;
        assert!((0.4..=0.9).contains(&seconds), "{seconds} s");

        // Peak about -3 dBFS, nothing clipped.
        let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
        let peak_db = 20.0 * (peak as f64 / 32768.0).log10();
        assert!((-3.5..=-2.5).contains(&peak_db), "peak {peak_db} dBFS");
        // It starts and ends at silence (no click), with no DC offset.
        assert_eq!(samples[0], 0);
        assert_eq!(*samples.last().unwrap(), 0);
        let tail = &samples[samples.len() - rate as usize / 100..];
        assert!(tail.iter().all(|s| s.unsigned_abs() < 64));
        let mean = samples.iter().map(|&s| s as f64).sum::<f64>() / samples.len() as f64;
        assert!(mean.abs() < 1.0, "DC offset {mean}");
    }

    #[test]
    fn players_play_the_file_quietly() {
        assert_eq!(
            player_arguments("paplay", "/c/n.wav"),
            vec!["--volume=38326", "--client-name=Impulse", "/c/n.wav"]
        );
        assert_eq!(
            player_arguments("pw-play", "/c/n.wav"),
            vec!["--volume=0.2", "/c/n.wav"]
        );
        assert_eq!(
            player_arguments("canberra-gtk-play", "/c/n.wav"),
            vec!["--file=/c/n.wav", "--volume=-14.0", "--description=Impulse"]
        );
    }

    #[test]
    fn the_cache_folder_follows_xdg() {
        use std::path::PathBuf;
        assert_eq!(
            cache_dir(Some("/x/cache".into()), Some("/home/u".into())),
            Some(PathBuf::from("/x/cache"))
        );
        // A relative $XDG_CACHE_HOME is ignored, as the spec says.
        assert_eq!(
            cache_dir(Some("cache".into()), Some("/home/u".into())),
            Some(PathBuf::from("/home/u/.cache"))
        );
        assert_eq!(cache_dir(None, None), None);
    }
}
