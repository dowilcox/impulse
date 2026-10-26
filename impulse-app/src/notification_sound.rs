//! Impulse's notification sounds: two console chirps and a soft chime
//! (`assets/sounds/`, made by `scripts/sounds/make-notification-sound.py`),
//! compiled in so no install step has to copy them. `agent_notify::post`
//! plays the one `notification_sound` names with each desktop
//! notification, in place of the desktop's own sound: NSSound from memory
//! on macOS; on Linux PulseAudio's or PipeWire's player, else libcanberra's,
//! from a copy in the cache folder. Nothing when none of them is there.

/// A notification sound (`notification_sound`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sound {
    /// Two chirps flicking upward, then a settled blip (the default).
    Ack,
    /// A chirp sliding up, answered by one sliding down.
    TwoTone,
    /// Two soft notes rising a fifth.
    Chime,
}

impl Sound {
    const ALL: [Sound; 3] = [Sound::Ack, Sound::TwoTone, Sound::Chime];

    /// The setting's value as a sound; none for "off".
    pub(crate) fn named(name: &str) -> Option<Sound> {
        Sound::ALL.into_iter().find(|sound| sound.name() == name)
    }

    fn name(self) -> &'static str {
        match self {
            Sound::Ack => "ack",
            Sound::TwoTone => "two-tone",
            Sound::Chime => "chime",
        }
    }

    /// 16-bit mono PCM WAV, peaking at -3 dBFS at most.
    fn wav(self) -> &'static [u8] {
        match self {
            Sound::Ack => include_bytes!("../../assets/sounds/ack.wav"),
            Sound::TwoTone => include_bytes!("../../assets/sounds/two-tone.wav"),
            Sound::Chime => include_bytes!("../../assets/sounds/chime.wav"),
        }
    }
}

/// The sound the settings choose, if any.
pub(crate) fn chosen(cx: &gpui_kit::App) -> Option<Sound> {
    Sound::named(&crate::settings_store::SettingsStore::get(cx).notification_sound)
}

/// Play the sound when it's chosen (in Settings or settings.json), so
/// picking one lets you hear it.
pub(crate) fn preview_on_change(cx: &mut gpui_kit::App) {
    use crate::settings_store::SettingsStore;
    let mut current = SettingsStore::get(cx).notification_sound.clone();
    cx.observe_global::<SettingsStore>(move |cx| {
        let now = &SettingsStore::get(cx).notification_sound;
        if *now == current {
            return;
        }
        current = now.clone();
        #[cfg(debug_assertions)]
        if crate::debug_snapshot::is_active() {
            return;
        }
        if let Some(sound) = Sound::named(&current) {
            play(sound);
        }
    })
    .detach();
}

/// The playback volume (0 to 1). The files are no louder than the chime,
/// which peaks at -3 dBFS; at 0.2 (-14 dB) they sit level with the
/// system's alert sounds.
const VOLUME: f32 = 0.2;

/// Play `sound` once, without waiting for it. On the main thread on
/// macOS.
pub(crate) fn play(sound: Sound) {
    #[cfg(target_os = "macos")]
    mac::play(sound);
    #[cfg(not(target_os = "macos"))]
    std::thread::spawn(move || {
        let Some(file) = cached_file(sound) else {
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
        /// Each sound's NSSound, made when first played (retained for the
        /// run), in `Sound::ALL`'s order.
        static SOUNDS: [std::cell::Cell<Option<*mut Object>>; 3] = Default::default();
    }

    fn make(wav: &'static [u8]) -> *mut Object {
        let (Some(data_class), Some(sound_class)) = (Class::get("NSData"), Class::get("NSSound"))
        else {
            return std::ptr::null_mut();
        };
        // SAFETY: NSData over the static bytes (never freed, so no copy),
        // and an NSSound made from it; both stay alive for the run.
        unsafe {
            let data: *mut Object = msg_send![data_class,
                dataWithBytesNoCopy: wav.as_ptr() as *mut std::ffi::c_void
                length: wav.len()
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

    pub(super) fn play(which: super::Sound) {
        let index = super::Sound::ALL.iter().position(|s| *s == which);
        SOUNDS.with(|sounds| {
            let Some(cell) = index.map(|i| &sounds[i]) else {
                return;
            };
            let sound = cell.get().unwrap_or_else(|| make(which.wav()));
            cell.set(Some(sound));
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

/// `sound` written to `<cache>/impulse/<name>.wav` for players that take
/// a file (once a run, and only when it differs).
#[cfg(not(target_os = "macos"))]
fn cached_file(sound: Sound) -> Option<std::path::PathBuf> {
    use std::sync::OnceLock;
    static FILES: [OnceLock<Option<std::path::PathBuf>>; 3] =
        [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    let index = Sound::ALL.iter().position(|s| *s == sound)?;
    FILES[index]
        .get_or_init(|| {
            let dir = cache_dir(std::env::var_os("XDG_CACHE_HOME"), std::env::var_os("HOME"))?
                .join("impulse");
            let name = format!("{}.wav", sound.name());
            let file = dir.join(&name);
            if std::fs::read(&file).is_ok_and(|bytes| bytes == sound.wav()) {
                return Some(file);
            }
            std::fs::create_dir_all(&dir).ok()?;
            let partial = dir.join(format!("{name}.{}", std::process::id()));
            std::fs::write(&partial, sound.wav()).ok()?;
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
    fn the_setting_names_the_sounds() {
        assert_eq!(Sound::named("ack"), Some(Sound::Ack));
        assert_eq!(Sound::named("two-tone"), Some(Sound::TwoTone));
        assert_eq!(Sound::named("chime"), Some(Sound::Chime));
        assert_eq!(Sound::named("off"), None);
        // Every choice the settings offer is a sound or off.
        let item = impulse_kit::settings::catalog()
            .into_iter()
            .find(|item| item.key == "notification_sound")
            .unwrap();
        let impulse_kit::settings::Control::Choice(options) = item.control else {
            panic!("a choice");
        };
        for (value, _) in options {
            assert!(value == "off" || Sound::named(value).is_some(), "{value}");
        }
    }

    #[test]
    fn the_sounds_are_short_and_quiet() {
        for sound in Sound::ALL {
            check(sound.wav(), sound.name());
        }
    }

    fn check(wav: &[u8], name: &str) {
        let (fmt, data) = chunks(wav);
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
        assert!((0.4..=0.9).contains(&seconds), "{name}: {seconds} s");

        // Peak at -3 dBFS at most, nothing clipped, and none much quieter
        // than the others.
        let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
        let peak_db = 20.0 * (peak as f64 / 32768.0).log10();
        assert!(
            (-8.0..=-2.5).contains(&peak_db),
            "{name}: peak {peak_db} dBFS"
        );
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
