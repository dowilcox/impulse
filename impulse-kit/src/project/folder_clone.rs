//! Folders copied into a new task: on macOS as APFS clones
//! (`clonefile(2)`), a whole tree in one call, taking no time and no extra
//! disk space until something in it changes; elsewhere (and across volumes)
//! as an ordinary copy. Dependency folders (`vendor`), build output
//! (`public/build`) and a database's data folder come over this way instead
//! of being rebuilt.

use std::fs;
use std::io;
use std::path::Path;

use crate::git::task_registry::{join, parent};

/// Clone `relative` from `source` into the same place under `destination`.
/// A folder the task already has (it holds a tracked file, say) gets the
/// entries it lacks. False when `relative` leaves the folder, is missing in
/// `source`, or nothing could be copied.
pub fn clone(relative: &str, source: &str, destination: &str) -> bool {
    if !is_inside(relative) {
        return false;
    }
    let from = join(source, relative);
    let to = join(destination, relative);
    let Ok(metadata) = fs::metadata(&from) else {
        return false;
    };
    let _ = fs::create_dir_all(parent(&to));
    if fs::symlink_metadata(&to).is_err() {
        return clone_item(Path::new(&from), Path::new(&to));
    }
    if !metadata.is_dir() {
        return false;
    }
    let Ok(entries) = fs::read_dir(&from) else {
        return false;
    };
    let mut cloned = false;
    for entry in entries.flatten() {
        let target = Path::new(&to).join(entry.file_name());
        if fs::symlink_metadata(&target).is_ok() {
            continue;
        }
        cloned = clone_item(&entry.path(), &target) || cloned;
    }
    cloned
}

fn clone_item(from: &Path, to: &Path) -> bool {
    if clone_file(from, to) {
        return true;
    }
    copy_item(from, to).is_ok()
}

#[cfg(target_os = "macos")]
fn clone_file(from: &Path, to: &Path) -> bool {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int};
    use std::os::unix::ffi::OsStrExt;

    extern "C" {
        fn clonefile(src: *const c_char, dst: *const c_char, flags: u32) -> c_int;
    }
    const CLONE_NOFOLLOW: u32 = 0x0001;
    let (Ok(src), Ok(dst)) = (
        CString::new(from.as_os_str().as_bytes()),
        CString::new(to.as_os_str().as_bytes()),
    ) else {
        return false;
    };
    // SAFETY: both are valid NUL-terminated paths for the call's duration.
    unsafe { clonefile(src.as_ptr(), dst.as_ptr(), CLONE_NOFOLLOW) == 0 }
}

#[cfg(not(target_os = "macos"))]
fn clone_file(_from: &Path, _to: &Path) -> bool {
    false
}

/// A copy of a file, link or whole folder, links copied as links.
pub fn copy_item(from: &Path, to: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(from)?;
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(from)?;
        #[cfg(unix)]
        return std::os::unix::fs::symlink(target, to);
        #[cfg(not(unix))]
        return fs::copy(from, to).map(|_| drop(target));
    }
    if metadata.is_dir() {
        fs::create_dir(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_item(&entry.path(), &to.join(entry.file_name()))?;
        }
        fs::set_permissions(to, metadata.permissions())?;
        return Ok(());
    }
    fs::copy(from, to).map(|_| ())
}

/// A relative path that stays inside the folder it's relative to.
pub fn is_inside(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.starts_with('~')
        && !path.split('/').any(|part| part == "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_paths_inside_the_folder() {
        assert!(is_inside("vendor"));
        assert!(is_inside("public/build"));
        assert!(!is_inside(""));
        assert!(!is_inside("/etc"));
        assert!(!is_inside("~/x"));
        assert!(!is_inside("a/../../b"));
    }

    #[cfg(unix)]
    #[test]
    fn links_are_copied_as_links() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main");
        fs::create_dir_all(main.join("vendor/bin")).unwrap();
        fs::write(main.join("vendor/tool"), "x").unwrap();
        std::os::unix::fs::symlink("../tool", main.join("vendor/bin/tool")).unwrap();
        let task = dir.path().join("task");
        assert!(clone(
            "vendor",
            &main.to_string_lossy(),
            &task.to_string_lossy()
        ));
        let link = task.join("vendor/bin/tool");
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("../tool"));
        assert_eq!(fs::read_to_string(link).unwrap(), "x");
    }
}
