use include_dir::{include_dir, Dir};
use std::path::PathBuf;

pub const EDITOR_HTML: &str = include_str!("../web/editor.html");

pub const MONACO_VERSION: &str = "0.52.2";

static MONACO_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/vendor/monaco");

fn data_home_dir() -> Option<PathBuf> {
    if let Ok(xdg_data_home) = std::env::var("XDG_DATA_HOME") {
        if !xdg_data_home.is_empty() {
            return Some(PathBuf::from(xdg_data_home));
        }
    }
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".local").join("share"))
}

/// Ensure Monaco editor files are extracted to the local data directory.
///
/// Returns the path to the extraction directory
/// (e.g. `~/.local/share/impulse/monaco/0.52.2/`).
pub fn ensure_monaco_extracted() -> Result<PathBuf, String> {
    let data_dir =
        data_home_dir().ok_or_else(|| "Cannot determine data home directory".to_string())?;

    let monaco_dir = data_dir.join("impulse").join("monaco").join(MONACO_VERSION);
    let marker = monaco_dir.join(".complete");

    // Check if already extracted with matching version
    if marker.is_file() {
        if let Ok(version) = std::fs::read_to_string(&marker) {
            if version.trim() == MONACO_VERSION {
                // Always overwrite editor.html (may change between builds)
                std::fs::write(monaco_dir.join("editor.html"), EDITOR_HTML)
                    .map_err(|e| format!("Failed to write editor.html: {}", e))?;
                return Ok(monaco_dir);
            }
        }
        // Version mismatch — remove and re-extract
        log::info!("Monaco version mismatch, re-extracting...");
        let _ = std::fs::remove_dir_all(&monaco_dir);
    }

    log::info!(
        "Extracting Monaco Editor v{} to {:?}",
        MONACO_VERSION,
        monaco_dir
    );

    // Create directory
    std::fs::create_dir_all(&monaco_dir)
        .map_err(|e| format!("Failed to create Monaco directory: {}", e))?;

    // Extract all embedded files
    extract_dir_recursive(&MONACO_DIR, &monaco_dir)?;

    // Write editor.html
    std::fs::write(monaco_dir.join("editor.html"), EDITOR_HTML)
        .map_err(|e| format!("Failed to write editor.html: {}", e))?;

    // Write completion marker last (incomplete extraction = retry next time)
    std::fs::write(&marker, MONACO_VERSION)
        .map_err(|e| format!("Failed to write completion marker: {}", e))?;

    Ok(monaco_dir)
}

fn extract_dir_recursive(dir: &Dir<'_>, target: &std::path::Path) -> Result<(), String> {
    for file in dir.files() {
        let path = target.join(file.path());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory {:?}: {}", parent, e))?;
        }
        std::fs::write(&path, file.contents())
            .map_err(|e| format!("Failed to write {:?}: {}", path, e))?;
    }
    for subdir in dir.dirs() {
        extract_dir_recursive(subdir, target)?;
    }
    Ok(())
}
