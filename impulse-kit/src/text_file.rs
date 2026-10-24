//! Editor files on disk, read and written without losing anything (the Swift
//! `TextFile`, plus the open checks in `TabManager`):
//! - strictly UTF-8: a file in another encoding isn't opened, rather than
//!   opening empty and being overwritten on save;
//! - a UTF-8 byte-order mark is remembered and written back;
//! - writes go through a symlink to its target instead of replacing the
//!   link, and keep the file's permissions.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Files larger than this don't open in the editor.
pub const MAX_EDITOR_FILE_SIZE: u64 = 10 * 1024 * 1024;
/// Files larger than this open read-only and aren't reloaded on every write.
pub const LARGE_FILE_SIZE: u64 = 5 * 1024 * 1024;

const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contents {
    pub text: String,
    pub bom: bool,
}

/// Identity of what's on disk; changes whenever anything rewrites it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub modified: Option<SystemTime>,
    pub size: u64,
    pub inode: u64,
}

/// Why a file can't open in the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenRefusal {
    /// Over `MAX_EDITOR_FILE_SIZE`.
    TooLarge,
    /// Not a regular file, or a NUL byte in its first 8 KB.
    Binary,
    /// Not valid UTF-8.
    NotUtf8,
    Unreadable(String),
}

impl OpenRefusal {
    /// What the editor says, as the Swift app's toasts say it.
    pub fn message(&self, name: &str) -> String {
        match self {
            OpenRefusal::TooLarge => format!("{name} is too large for the editor."),
            OpenRefusal::Binary => format!("{name} is a binary file the editor can't show."),
            OpenRefusal::NotUtf8 => {
                format!("{name} isn't UTF-8 text, so the editor won't open it.")
            }
            OpenRefusal::Unreadable(error) => format!("{name} can't be read: {error}"),
        }
    }
}

/// A file ready for the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    pub contents: Contents,
    /// The stamp it had while it was read; `None` when it changed during the
    /// read, so a save compares the text itself.
    pub stamp: Option<Stamp>,
    /// Over `LARGE_FILE_SIZE`: opens read-only.
    pub large: bool,
}

pub fn decode(bytes: &[u8]) -> Option<Contents> {
    let bom = bytes.starts_with(&UTF8_BOM);
    let body = if bom { &bytes[UTF8_BOM.len()..] } else { bytes };
    let text = std::str::from_utf8(body).ok()?.to_string();
    Some(Contents { text, bom })
}

/// The file's text, or `None` when it's unreadable or not valid UTF-8.
pub fn read(path: &Path) -> Option<Contents> {
    decode(&fs::read(path).ok()?)
}

/// Where a write to `path` should land: the real file behind any symlinks.
pub fn target(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

pub fn stamp(path: &Path) -> Option<Stamp> {
    let meta = fs::metadata(target(path)).ok()?;
    #[cfg(unix)]
    let inode = std::os::unix::fs::MetadataExt::ino(&meta);
    #[cfg(not(unix))]
    let inode = 0;
    Some(Stamp {
        modified: meta.modified().ok(),
        size: meta.len(),
        inode,
    })
}

/// Not a regular file, larger than the editor's limit, or a NUL byte in the
/// first 8 KB. Opening never blocks (a FIFO with no writer).
pub fn is_binary(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return true;
    }
    if meta.len() > MAX_EDITOR_FILE_SIZE {
        return true;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut buffer = [0u8; 8192];
    let count = file.read(&mut buffer).unwrap_or(0);
    buffer[..count].contains(&0)
}

/// Read `path` for the editor, with every check the Swift app makes before
/// opening a tab. A path that doesn't exist yet opens empty.
pub fn open(path: &Path) -> Result<Opened, OpenRefusal> {
    if !path.exists() {
        return Ok(Opened {
            contents: Contents {
                text: String::new(),
                bom: false,
            },
            stamp: None,
            large: false,
        });
    }
    if stamp(path).is_some_and(|s| s.size > MAX_EDITOR_FILE_SIZE) {
        return Err(OpenRefusal::TooLarge);
    }
    if is_binary(path) {
        return Err(OpenRefusal::Binary);
    }
    let before = stamp(path);
    let bytes = fs::read(path).map_err(|e| OpenRefusal::Unreadable(e.to_string()))?;
    let contents = decode(&bytes).ok_or(OpenRefusal::NotUtf8)?;
    let after = stamp(path);
    let size = contents.text.len() as u64;
    if size > MAX_EDITOR_FILE_SIZE {
        return Err(OpenRefusal::TooLarge);
    }
    Ok(Opened {
        contents,
        stamp: if before.is_some() && before == after {
            after
        } else {
            None
        },
        large: size > LARGE_FILE_SIZE,
    })
}

/// Write `text` (with the BOM if it had one) atomically to the real file
/// behind `path`, keeping its permissions.
pub fn write(text: &str, bom: bool, path: &Path) -> io::Result<()> {
    let target = target(path);
    let dir = target.parent().unwrap_or(Path::new("."));
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = dir.join(format!(".{name}.impulse-{}", std::process::id()));
    {
        let mut file = fs::File::create(&temp)?;
        if bom {
            file.write_all(&UTF8_BOM)?;
        }
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    if let Ok(meta) = fs::metadata(&target) {
        fs::set_permissions(&temp, meta.permissions())?;
    }
    fs::rename(&temp, &target).inspect_err(|_| {
        let _ = fs::remove_file(&temp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_text_that_is_not_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let latin1 = dir.path().join("latin1.txt");
        fs::write(&latin1, [0x63, 0x61, 0x66, 0xE9, 0x0A]).unwrap(); // "café\n" in Latin-1
        assert_eq!(read(&latin1), None);
        assert_eq!(read(&dir.path().join("missing.txt")), None);
        assert_eq!(open(&latin1), Err(OpenRefusal::NotUtf8));
    }

    #[test]
    fn keeps_the_byte_order_mark() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bom.txt");
        fs::write(&file, [&UTF8_BOM[..], b"hello\n"].concat()).unwrap();
        let contents = read(&file).unwrap();
        assert_eq!(
            contents,
            Contents {
                text: "hello\n".into(),
                bom: true
            }
        );
        write("bye\n", contents.bom, &file).unwrap();
        assert_eq!(fs::read(&file).unwrap(), [&UTF8_BOM[..], b"bye\n"].concat());
    }

    #[cfg(unix)]
    #[test]
    fn writes_through_symlinks_and_keeps_permissions() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let target_file = dir.path().join("AGENTS.md");
        let link = dir.path().join("CLAUDE.md");
        fs::write(&target_file, "old\n").unwrap();
        fs::set_permissions(&target_file, fs::Permissions::from_mode(0o755)).unwrap();
        symlink("AGENTS.md", &link).unwrap();

        write("new\n", false, &link).unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("AGENTS.md"));
        assert_eq!(fs::read_to_string(&target_file).unwrap(), "new\n");
        let mode = fs::metadata(&target_file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
        assert_eq!(stamp(&link), stamp(&target_file));
    }

    #[test]
    fn binary_and_size_checks() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("a.bin");
        fs::write(&binary, [b'a', 0, b'b']).unwrap();
        assert_eq!(open(&binary), Err(OpenRefusal::Binary));
        assert_eq!(open(dir.path()), Err(OpenRefusal::Binary));

        let text = dir.path().join("a.txt");
        fs::write(&text, "fine\n").unwrap();
        let opened = open(&text).unwrap();
        assert_eq!(opened.contents.text, "fine\n");
        assert!(opened.stamp.is_some());
        assert!(!opened.large);

        let missing = open(&dir.path().join("new.rs")).unwrap();
        assert_eq!(missing.contents.text, "");
    }
}
