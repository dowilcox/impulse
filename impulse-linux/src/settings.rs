pub use impulse_core::settings::*;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Debug)]
pub struct SettingsLoadWarning {
    pub settings_path: PathBuf,
    pub backup_path: Option<PathBuf>,
    pub message: String,
}

#[derive(Clone, Debug)]
struct SettingsFileSnapshot {
    path: PathBuf,
    content_hash: String,
}

static SETTINGS_LOAD_WARNING: OnceLock<Mutex<Option<SettingsLoadWarning>>> = OnceLock::new();
static SETTINGS_FILE_SNAPSHOT: OnceLock<Mutex<Option<SettingsFileSnapshot>>> = OnceLock::new();

pub fn matches_file_pattern(path: &str, pattern: &str) -> bool {
    impulse_core::util::matches_file_pattern(path, pattern)
}

pub fn settings_load_warning() -> Option<SettingsLoadWarning> {
    settings_load_warning_cell()
        .lock()
        .ok()
        .and_then(|warning| warning.clone())
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
            set_settings_load_warning(None);
            set_settings_file_snapshot(None);
            return Settings::default();
        }
    };
    let settings = match std::fs::read(&path) {
        Ok(contents) => match std::str::from_utf8(&contents) {
            Ok(json) => match Settings::from_json(json) {
                Ok(s) => {
                    set_settings_load_warning(None);
                    set_settings_file_snapshot(Some(SettingsFileSnapshot {
                        path: path.clone(),
                        content_hash: stable_content_hash(&contents),
                    }));
                    s
                }
                Err(e) => {
                    let existing_warning =
                        settings_load_warning().filter(|warning| warning.settings_path == path);
                    let backup_path = match existing_warning {
                        Some(warning) => warning.backup_path,
                        None => backup_invalid_settings_file(&path, &contents),
                    };
                    set_settings_load_warning(Some(SettingsLoadWarning {
                        settings_path: path.clone(),
                        backup_path: backup_path.clone(),
                        message: e.clone(),
                    }));
                    set_settings_file_snapshot(None);
                    log::error!(
                        "Failed to parse settings from {}: {}; using defaults{}",
                        path.display(),
                        e,
                        backup_path
                            .as_ref()
                            .map(|path| format!("; invalid file backed up to {}", path.display()))
                            .unwrap_or_default()
                    );
                    Settings::default()
                }
            },
            Err(e) => {
                let message = format!("Failed to read settings as UTF-8: {e}");
                let existing_warning =
                    settings_load_warning().filter(|warning| warning.settings_path == path);
                let backup_path = match existing_warning {
                    Some(warning) => warning.backup_path,
                    None => backup_invalid_settings_file(&path, &contents),
                };
                set_settings_load_warning(Some(SettingsLoadWarning {
                    settings_path: path.clone(),
                    backup_path: backup_path.clone(),
                    message: message.clone(),
                }));
                set_settings_file_snapshot(None);
                log::error!(
                    "{} from {}; using defaults{}",
                    message,
                    path.display(),
                    backup_path
                        .as_ref()
                        .map(|path| format!("; invalid file backed up to {}", path.display()))
                        .unwrap_or_default()
                );
                Settings::default()
            }
        },
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                set_settings_load_warning(None);
                set_settings_file_snapshot(None);
            } else {
                let message = e.to_string();
                set_settings_load_warning(Some(SettingsLoadWarning {
                    settings_path: path.clone(),
                    backup_path: None,
                    message: message.clone(),
                }));
                set_settings_file_snapshot(None);
                log::error!(
                    "Failed to read settings from {}: {}; using defaults",
                    path.display(),
                    message
                );
            }
            Settings::default()
        }
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
    if let Some(warning) = settings_load_warning() {
        log::warn!(
            "Skipping settings save to preserve invalid settings file {}: {}",
            warning.settings_path.display(),
            warning.message
        );
        return;
    }
    if let Some(message) = settings_file_changed_since_load(&path) {
        log::warn!(
            "Skipping settings save because {}. Open settings.json and reload Impulse before saving settings.",
            message
        );
        return;
    }
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
    } else {
        set_settings_file_snapshot(Some(SettingsFileSnapshot {
            path,
            content_hash: stable_content_hash(json.as_bytes()),
        }));
    }
}

fn settings_load_warning_cell() -> &'static Mutex<Option<SettingsLoadWarning>> {
    SETTINGS_LOAD_WARNING.get_or_init(|| Mutex::new(None))
}

fn set_settings_load_warning(warning: Option<SettingsLoadWarning>) {
    if let Ok(mut cell) = settings_load_warning_cell().lock() {
        *cell = warning;
    }
}

fn settings_file_snapshot_cell() -> &'static Mutex<Option<SettingsFileSnapshot>> {
    SETTINGS_FILE_SNAPSHOT.get_or_init(|| Mutex::new(None))
}

fn settings_file_snapshot() -> Option<SettingsFileSnapshot> {
    settings_file_snapshot_cell()
        .lock()
        .ok()
        .and_then(|snapshot| snapshot.clone())
}

fn set_settings_file_snapshot(snapshot: Option<SettingsFileSnapshot>) {
    if let Ok(mut cell) = settings_file_snapshot_cell().lock() {
        *cell = snapshot;
    }
}

fn settings_file_changed_since_load(path: &Path) -> Option<String> {
    match std::fs::read(path) {
        Ok(contents) => {
            let current_hash = stable_content_hash(&contents);
            match settings_file_snapshot() {
                Some(snapshot)
                    if snapshot.path.as_path() == path && snapshot.content_hash == current_hash =>
                {
                    None
                }
                Some(snapshot) if snapshot.path.as_path() == path => Some(format!(
                    "{} changed on disk since it was loaded",
                    path.display()
                )),
                Some(_) => Some(format!(
                    "{} was not the file loaded at startup",
                    path.display()
                )),
                None => Some(format!("{} was created after startup", path.display())),
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if settings_file_snapshot().is_some() {
                Some(format!(
                    "{} was removed after it was loaded",
                    path.display()
                ))
            } else {
                None
            }
        }
        Err(e) => Some(format!("{} could not be checked: {}", path.display(), e)),
    }
}

fn stable_content_hash(contents: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in contents {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{}:{hash:016x}", contents.len())
}

fn backup_invalid_settings_file(path: &Path, contents: &[u8]) -> Option<PathBuf> {
    let parent = path.parent()?;
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("settings");
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("json");
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);

    for attempt in 0..100 {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{}", attempt)
        };
        let backup = parent.join(format!("{stem}.invalid-{timestamp}{suffix}.{extension}"));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        match opts.open(&backup) {
            Ok(mut file) => {
                use std::io::Write;
                if file.write_all(contents).is_ok() {
                    return Some(backup);
                }
                log::error!(
                    "Failed to write invalid settings backup {}",
                    backup.display()
                );
                return None;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                log::error!(
                    "Failed to create invalid settings backup {}: {}",
                    backup.display(),
                    e
                );
                return None;
            }
        }
    }

    log::error!("Failed to choose a unique invalid settings backup path");
    None
}
