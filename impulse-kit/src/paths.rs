//! Where Impulse keeps its files (the Swift `AppPaths`): the data directory
//! is `~/Library/Application Support/impulse` on macOS, as the Swift app
//! has it, and `$XDG_DATA_HOME/impulse` (`~/.local/share/impulse`) on Linux.

use std::path::PathBuf;

/// The data directory, created (private to the user) if it's missing.
/// `dev` builds keep theirs apart, as `impulse-dev`.
pub fn data_dir(dev: bool) -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join(if dev { "impulse-dev" } else { "impulse" });
    if std::fs::create_dir_all(&dir).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    dir
}

/// `trusted-folders.json`, shared with the Swift app on the Mac.
pub fn trusted_folders_file(dev: bool) -> PathBuf {
    data_dir(dev).join("trusted-folders.json")
}
