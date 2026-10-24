//! Listening ports (the Swift `PortScanner` and `scanPorts`): every 3
//! seconds while the window is active, the TCP ports the processes under
//! each workspace's terminals listen on, for the port chip on its sidebar
//! row and the status bar's items (a click opens `http://localhost:<port>`).
//! Linux reads `/proc`; macOS asks `lsof`. Parsing is
//! `impulse_kit::listening_ports`.

use std::collections::HashMap;
use std::time::Duration;

use gpui_kit::{Context, Window};
use impulse_kit::listening_ports::{self, ListeningPort};

use crate::workbench::Workbench;

/// How often the ports are looked at.
const INTERVAL: Duration = Duration::from_secs(3);

/// `pid` and every process under it.
fn tree(pid: i32) -> Vec<i32> {
    let mut all = Vec::new();
    let mut pending = vec![pid];
    while let Some(pid) = pending.pop() {
        if pid <= 0 || all.contains(&pid) {
            continue;
        }
        all.push(pid);
        pending.extend(crate::agent_process::children(pid));
    }
    all
}

/// The TCP ports `pids` listen on.
#[cfg(target_os = "linux")]
fn listening(pids: &[i32]) -> Vec<ListeningPort> {
    let mut sockets: HashMap<u64, u16> = HashMap::new();
    for table in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(text) = std::fs::read_to_string(table) {
            sockets.extend(listening_ports::proc_net_listeners(&text));
        }
    }
    if sockets.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    for &pid in pids {
        let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
            continue;
        };
        let process = std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .map(|c| c.trim().to_string())
            .unwrap_or_default();
        for fd in fds.flatten() {
            let Ok(link) = std::fs::read_link(fd.path()) else {
                continue;
            };
            let inode = listening_ports::socket_inode(&link.to_string_lossy());
            if let Some(&port) = inode.and_then(|i| sockets.get(&i)) {
                found.push(ListeningPort {
                    port,
                    pid,
                    process: process.clone(),
                });
            }
        }
    }
    listening_ports::merge(found)
}

/// The TCP ports `pids` listen on.
#[cfg(not(target_os = "linux"))]
fn listening(pids: &[i32]) -> Vec<ListeningPort> {
    if pids.is_empty() {
        return Vec::new();
    }
    let list = pids
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let output = std::process::Command::new("lsof")
        .args([
            "-nP",
            "-a",
            "-iTCP",
            "-sTCP:LISTEN",
            "-F",
            "pcn",
            "-p",
            &list,
        ])
        .output();
    match output {
        Ok(output) => listening_ports::lsof_listeners(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => Vec::new(),
    }
}

impl Workbench {
    pub(crate) fn start_port_scan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |workbench, cx| loop {
            let trees = workbench.update_in(cx, |workbench, window, cx| {
                // Only while the window is in front (Swift: while visible;
                // the headless snapshot's windows never are).
                let looking =
                    window.is_window_active() || crate::debug_snapshot::config().is_some();
                looking.then(|| workbench.process_trees(cx))
            });
            let Ok(trees) = trees else {
                break;
            };
            if let Some(trees) = trees {
                let ports = cx
                    .background_executor()
                    .spawn(async move {
                        trees
                            .into_iter()
                            .map(|(id, pids)| (id, listening(&pids)))
                            .collect::<HashMap<u64, Vec<ListeningPort>>>()
                    })
                    .await;
                if workbench
                    .update(cx, |workbench, cx| workbench.set_ports(ports, cx))
                    .is_err()
                {
                    break;
                }
            }
            cx.background_executor().timer(INTERVAL).await;
        })
        .detach();
    }

    /// Each workspace's terminal processes, by workspace id.
    fn process_trees(&self, cx: &gpui_kit::App) -> Vec<(u64, Vec<i32>)> {
        self.workspaces
            .iter()
            .map(|workspace| {
                let pids = workspace
                    .all_surfaces()
                    .filter_map(|s| s.terminal())
                    .flat_map(|t| tree(t.read(cx).shell_pid()))
                    .collect();
                (workspace.id, pids)
            })
            .collect()
    }

    fn set_ports(&mut self, mut ports: HashMap<u64, Vec<ListeningPort>>, cx: &mut Context<Self>) {
        let mut changed = false;
        for workspace in &mut self.workspaces {
            let next = ports.remove(&workspace.id).unwrap_or_default();
            if workspace.ports != next {
                workspace.ports = next;
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
    }
}
