use fs2::FileExt;
use include_dir::{include_dir, Dir};
use std::path::PathBuf;

pub const EDITOR_HTML: &str = include_str!("../web/editor.html");
pub const EDITOR_JS: &str = include_str!("../web/editor.js");

pub const MONACO_VERSION: &str = "0.55.1+fonts1";

static MONACO_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/vendor/monaco");
static FONTS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/vendor/fonts");

/// Ensure Monaco editor files are extracted to the local data directory.
///
/// Returns the path to the extraction directory
/// (e.g. `~/.local/share/impulse/monaco/0.52.2/` on Linux,
/// `~/Library/Application Support/impulse/monaco/0.52.2/` on macOS).
pub fn ensure_monaco_extracted() -> Result<PathBuf, String> {
    let data_dir =
        dirs::data_dir().ok_or_else(|| "Cannot determine data home directory".to_string())?;

    let monaco_dir = data_dir.join("impulse").join("monaco").join(MONACO_VERSION);

    // Acquire exclusive lock to make check-and-extract atomic
    let lock_path = data_dir
        .join("impulse")
        .join("monaco")
        .join(".extract.lock");
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create lock directory: {}", e))?;
    }
    let lock_file = std::fs::File::create(&lock_path)
        .map_err(|e| format!("Failed to create lock file: {}", e))?;
    lock_file
        .lock_exclusive()
        .map_err(|e| format!("Failed to acquire extraction lock: {}", e))?;

    let marker = monaco_dir.join(".complete");

    // Check if already extracted with matching version
    if marker.is_file() {
        if let Ok(version) = std::fs::read_to_string(&marker) {
            if version.trim() == MONACO_VERSION {
                // Always overwrite editor.html and editor.js (may change between builds)
                std::fs::write(monaco_dir.join("editor.html"), EDITOR_HTML)
                    .map_err(|e| format!("Failed to write editor.html: {}", e))?;
                std::fs::write(monaco_dir.join("editor.js"), EDITOR_JS)
                    .map_err(|e| format!("Failed to write editor.js: {}", e))?;
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

    // Restrict directory permissions to owner-only on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&monaco_dir, std::fs::Permissions::from_mode(0o700));
    }

    // Extract all embedded files
    extract_dir_recursive(&MONACO_DIR, &monaco_dir)?;

    // Extract bundled fonts into the Monaco directory (for editor @font-face)
    extract_dir_recursive(&FONTS_DIR, &monaco_dir.join("fonts"))?;

    // Write editor.html and editor.js
    std::fs::write(monaco_dir.join("editor.html"), EDITOR_HTML)
        .map_err(|e| format!("Failed to write editor.html: {}", e))?;
    std::fs::write(monaco_dir.join("editor.js"), EDITOR_JS)
        .map_err(|e| format!("Failed to write editor.js: {}", e))?;

    // Install fonts to user font directory for the terminal
    install_user_fonts();

    // Write completion marker last (incomplete extraction = retry next time)
    std::fs::write(&marker, MONACO_VERSION)
        .map_err(|e| format!("Failed to write completion marker: {}", e))?;

    Ok(monaco_dir)
}

/// Install bundled TTF fonts to the user's system font directory so the
/// terminal (VTE/SwiftTerm) can use them without manual installation.
///
/// - Linux: `~/.local/share/fonts/JetBrainsMono/`
/// - macOS: `~/Library/Fonts/`
fn install_user_fonts() {
    let font_dir = match dirs::font_dir() {
        Some(d) => d,
        None => {
            log::warn!("Cannot determine user font directory; skipping font installation");
            return;
        }
    };

    // On Linux, install into a subdirectory; on macOS, ~/Library/Fonts/ is flat
    let target_dir = if cfg!(target_os = "macos") {
        font_dir.clone()
    } else {
        font_dir.join("JetBrainsMono")
    };

    if let Err(e) = std::fs::create_dir_all(&target_dir) {
        log::warn!("Failed to create font directory {:?}: {}", target_dir, e);
        return;
    }

    let font_subdir = match FONTS_DIR.get_dir("jetbrains-mono") {
        Some(d) => d,
        None => {
            log::warn!("Bundled jetbrains-mono font directory not found");
            return;
        }
    };

    for file in font_subdir.files() {
        let name = match file.path().file_name() {
            Some(n) => n,
            None => continue,
        };
        // Only install .ttf files
        if !name.to_string_lossy().ends_with(".ttf") {
            continue;
        }
        let dest = target_dir.join(name);
        if dest.exists() {
            continue;
        }
        if let Err(e) = std::fs::write(&dest, file.contents()) {
            log::warn!("Failed to install font {:?}: {}", dest, e);
        } else {
            log::info!("Installed font: {:?}", dest);
        }
    }
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
