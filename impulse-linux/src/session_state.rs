use std::path::PathBuf;

pub fn load() -> Option<impulse_core::session_state::SessionState> {
    let path = session_state_path()?;
    let json = match std::fs::read_to_string(&path) {
        Ok(json) => json,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!(
                    "Failed to read session state from {}: {}",
                    path.display(),
                    e
                );
            }
            return None;
        }
    };
    match impulse_core::session_state::SessionState::from_json(&json) {
        Ok(state) => Some(state),
        Err(e) => {
            log::warn!(
                "Failed to parse session state from {}: {}",
                path.display(),
                e
            );
            None
        }
    }
}

pub fn save(state: &impulse_core::session_state::SessionState) {
    let Some(path) = session_state_path() else {
        log::warn!("Cannot determine state directory; session state will not be saved");
        return;
    };
    let Some(parent) = path.parent() else {
        log::warn!("Cannot determine session state directory");
        return;
    };
    if let Err(e) = std::fs::create_dir_all(parent) {
        log::warn!(
            "Failed to create session state directory {}: {}",
            parent.display(),
            e
        );
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)) {
            log::warn!(
                "Failed to set permissions on session state directory {}: {}",
                parent.display(),
                e
            );
        }
    }

    let json = match state.to_json() {
        Ok(json) => json,
        Err(e) => {
            log::warn!("Failed to serialize session state: {}", e);
            return;
        }
    };
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
            Ok(file) => file,
            Err(e) => {
                log::warn!(
                    "Failed to write session state to {}: {}",
                    tmp_path.display(),
                    e
                );
                return;
            }
        };
        if let Err(e) = file.write_all(json.as_bytes()) {
            log::warn!(
                "Failed to write session state to {}: {}",
                tmp_path.display(),
                e
            );
            return;
        }
    }
    if let Err(e) = std::fs::rename(&tmp_path, &path) {
        log::warn!(
            "Failed to move session state into {}: {}",
            path.display(),
            e
        );
    }
}

fn session_state_path() -> Option<PathBuf> {
    state_dir().map(|dir| dir.join("session-state.json"))
}

fn state_dir() -> Option<PathBuf> {
    if let Ok(xdg_state_home) = std::env::var("XDG_STATE_HOME") {
        if !xdg_state_home.is_empty() {
            return Some(PathBuf::from(xdg_state_home).join("impulse"));
        }
    }

    std::env::var("HOME").ok().map(|home| {
        PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("impulse")
    })
}
