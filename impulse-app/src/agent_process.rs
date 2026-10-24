//! What a terminal's foreground program is, for agent detection (the Swift
//! `ProcessInspector` and `proc_listchildpids`): its executable and argv,
//! and its children for wrappers (`caffeinate claude`). `/proc` on Linux,
//! libproc and `KERN_PROCARGS2` on macOS.

/// Executable path and arguments of a running process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessDetails {
    pub executable_path: String,
    /// argv, including argv[0].
    pub arguments: Vec<String>,
}

/// The process's executable and argv; `None` when it's gone or unreadable.
pub fn details(pid: i32) -> Option<ProcessDetails> {
    if pid <= 0 {
        return None;
    }
    let executable_path = executable(pid).unwrap_or_default();
    let arguments = arguments(pid).unwrap_or_default();
    if executable_path.is_empty() && arguments.is_empty() {
        return None;
    }
    Some(ProcessDetails {
        executable_path,
        arguments,
    })
}

#[cfg(target_os = "linux")]
fn executable(pid: i32) -> Option<String> {
    let path = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    Some(path.to_string_lossy().into_owned())
}

#[cfg(target_os = "linux")]
fn arguments(pid: i32) -> Option<Vec<String>> {
    let bytes = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(split_nul(&bytes))
}

/// The direct children of a process.
#[cfg(target_os = "linux")]
pub fn children(pid: i32) -> Vec<i32> {
    // Each thread lists the children it started; the main thread's are
    // the usual case, the others are read for completeness.
    let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) else {
        return Vec::new();
    };
    let mut pids = Vec::new();
    let mut listed = false;
    for task in tasks.flatten() {
        if let Ok(text) = std::fs::read_to_string(task.path().join("children")) {
            listed = true;
            pids.extend(
                text.split_whitespace()
                    .filter_map(|p| p.parse::<i32>().ok()),
            );
        }
    }
    if !listed {
        // A kernel without `children` files: every process's parent.
        let Ok(all) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        pids = all
            .flatten()
            .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
            .filter(|child| crate::control_server::parent_of(*child) == Some(pid))
            .collect();
    }
    pids.retain(|p| *p > 0);
    pids
}

#[cfg(target_os = "macos")]
fn executable(pid: i32) -> Option<String> {
    let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let length = unsafe {
        libc::proc_pidpath(
            pid,
            buffer.as_mut_ptr() as *mut libc::c_void,
            buffer.len() as u32,
        )
    };
    (length > 0).then(|| String::from_utf8_lossy(&buffer[..length as usize]).into_owned())
}

/// argv from `KERN_PROCARGS2`.
#[cfg(target_os = "macos")]
fn arguments(pid: i32) -> Option<Vec<String>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size: libc::size_t = 0;
    let ok = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } == 0;
    if !ok || size <= 4 {
        return None;
    }
    let mut buffer = vec![0u8; size];
    let ok = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buffer.as_mut_ptr() as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } == 0;
    ok.then(|| parse_proc_args(&buffer[..size.min(buffer.len())]))
        .flatten()
}

/// The direct children of a process.
#[cfg(target_os = "macos")]
pub fn children(pid: i32) -> Vec<i32> {
    let count = unsafe { libc::proc_listchildpids(pid, std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Vec::new();
    }
    let mut pids = vec![0 as libc::pid_t; count as usize];
    let size = (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
    // Returns a count of pids (not bytes).
    let filled =
        unsafe { libc::proc_listchildpids(pid, pids.as_mut_ptr() as *mut libc::c_void, size) };
    pids.truncate(filled.clamp(0, count) as usize);
    pids.retain(|p| *p > 0);
    pids
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn executable(_pid: i32) -> Option<String> {
    None
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn arguments(_pid: i32) -> Option<Vec<String>> {
    None
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn children(_pid: i32) -> Vec<i32> {
    Vec::new()
}

/// `/proc/<pid>/cmdline`: NUL-separated arguments, the last one ending in a
/// NUL too.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn split_nul(bytes: &[u8]) -> Vec<String> {
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    if bytes.is_empty() {
        return Vec::new();
    }
    bytes
        .split(|b| *b == 0)
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect()
}

/// `KERN_PROCARGS2`'s layout: an argc int, the exec path, NUL padding, then
/// argc NUL-terminated strings (the environment follows).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_proc_args(buffer: &[u8]) -> Option<Vec<String>> {
    let int_size = std::mem::size_of::<i32>();
    if buffer.len() <= int_size {
        return None;
    }
    let argc = i32::from_ne_bytes(buffer[..int_size].try_into().ok()?);
    if !(1..4096).contains(&argc) {
        return None;
    }
    let mut index = int_size;
    // Skip the exec path and the padding after it.
    while index < buffer.len() && buffer[index] != 0 {
        index += 1;
    }
    while index < buffer.len() && buffer[index] == 0 {
        index += 1;
    }
    let mut arguments = Vec::new();
    while arguments.len() < argc as usize && index < buffer.len() {
        let start = index;
        while index < buffer.len() && buffer[index] != 0 {
            index += 1;
        }
        arguments.push(String::from_utf8_lossy(&buffer[start..index]).into_owned());
        index += 1;
    }
    Some(arguments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmdline_splits_on_nul() {
        assert_eq!(
            split_nul(b"node\0/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js\0"),
            vec![
                "node",
                "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"
            ]
        );
        assert!(split_nul(b"").is_empty());
        assert_eq!(split_nul(b"a\0\0b\0"), vec!["a", "", "b"]);
    }

    #[test]
    fn procargs_skip_the_exec_path_and_stop_at_argc() {
        let mut buffer = 2i32.to_ne_bytes().to_vec();
        buffer.extend_from_slice(b"/usr/local/bin/node\0\0\0\0node\0/opt/codex.js\0HOME=/x\0");
        assert_eq!(
            parse_proc_args(&buffer),
            Some(vec!["node".to_string(), "/opt/codex.js".to_string()])
        );
        assert_eq!(parse_proc_args(&0i32.to_ne_bytes()), None);
        let mut zero = 0i32.to_ne_bytes().to_vec();
        zero.extend_from_slice(b"/bin/sh\0");
        assert_eq!(parse_proc_args(&zero), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_this_process() {
        let me = details(std::process::id() as i32).expect("own details");
        assert!(!me.executable_path.is_empty());
        assert!(!me.arguments.is_empty());
    }
}
