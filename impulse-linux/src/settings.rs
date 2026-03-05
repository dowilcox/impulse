pub use impulse_core::settings::*;

use std::path::PathBuf;

pub fn matches_file_pattern(path: &str, pattern: &str) -> bool {
    impulse_core::util::matches_file_pattern(path, pattern)
}

fn settings_path() -> Option<PathBuf> {
    let config_dir = dirs::config_dir()?;
    let impulse_dir = config_dir.join("impulse");
    let _ = std::fs::create_dir_all(&impulse_dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) =
            std::fs::set_permissions(&impulse_dir, std::fs::Permissions::from_mode(0o700))
        {
            log::warn!("Failed to set permissions on {:?}: {}", impulse_dir, e);
        }
    }
    Some(impulse_dir.join("settings.json"))
}

pub fn load() -> Settings {
    let path = match settings_path() {
        Some(p) => p,
        None => {
            log::warn!("Cannot determine config directory; using default settings");
            return Settings::default();
        }
    };
    let mut settings = match std::fs::read_to_string(&path) {
        Ok(contents) => match Settings::from_json(&contents) {
            Ok(s) => s,
            Err(e) => {
                log::error!(
                    "Failed to parse settings from {}: {}; using defaults",
                    path.display(),
                    e
                );
                Settings::default()
            }
        },
        Err(_) => Settings::default(),
    };

    // Check if migrations changed anything and save if so.
    // from_json already calls migrate + validate, but the old code saved on
    // migration — replicate that behaviour by re-saving when the font was
    // migrated (the most common migration path).
    let default_font = "JetBrains Mono";
    let needs_save = settings.font_family == default_font
        && std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| {
                serde_json::from_str::<serde_json::Value>(&raw)
                    .ok()
                    .and_then(|v| {
                        v.get("font_family")
                            .and_then(|f| f.as_str().map(String::from))
                    })
            })
            .is_some_and(|old| old != default_font);
    if needs_save {
        save(&settings);
    }

    settings
}

pub fn save(settings: &Settings) {
    let path = match settings_path() {
        Some(p) => p,
        None => {
            log::error!("Cannot determine config directory; settings will not be saved");
            return;
        }
    };
    let json = match settings.to_json() {
        Ok(j) => j,
        Err(e) => {
            log::error!("Failed to serialize settings: {}", e);
            return;
        }
    };
    // Atomic write: write to temp file with restrictive permissions, then rename
    let tmp_path = path.with_extension("json.tmp");
    {
        use std::io::Write;
        #[cfg(unix)]
        use std::os::unix::fs::OpenOptionsExt;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        opts.mode(0o600);
        let mut file = match opts.open(&tmp_path) {
            Ok(f) => f,
            Err(e) => {
                log::error!("Failed to write settings to {}: {}", tmp_path.display(), e);
                return;
            }
        };
        if let Err(e) = file.write_all(json.as_bytes()) {
            log::error!("Failed to write settings to {}: {}", tmp_path.display(), e);
            return;
        }
    }
    if let Err(e) = std::fs::rename(&tmp_path, &path) {
        log::error!("Failed to rename settings file: {}", e);
    }
}
