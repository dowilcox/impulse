//! TCP ports that processes under a workspace's terminals listen on (the
//! Swift `PortScanner`, which asks libproc): shown on the workspace's
//! sidebar row and in the status bar. Linux reads them from `/proc`
//! (`/proc/<pid>/fd` socket inodes matched in `/proc/net/tcp{,6}`), macOS
//! from `lsof`; this module parses both and merges what it finds.

use std::collections::BTreeMap;

/// A TCP port some process is listening on.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ListeningPort {
    pub port: u16,
    pub pid: i32,
    /// The listening process's name ("node", "python3.12").
    pub process: String,
}

/// TCP's LISTEN state in `/proc/net/tcp`.
const LISTEN: &str = "0A";

/// The listening sockets in a `/proc/net/tcp` or `tcp6` table, as
/// (socket inode, port).
pub fn proc_net_listeners(table: &str) -> Vec<(u64, u16)> {
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            // sl local_address rem_address st tx:rx tr:when retrnsmt uid timeout inode
            if fields.len() < 10 || fields[3] != LISTEN {
                return None;
            }
            let port = fields[1].rsplit(':').next()?;
            let port = u16::from_str_radix(port, 16).ok()?;
            let inode = fields[9].parse::<u64>().ok()?;
            (port > 0 && inode > 0).then_some((inode, port))
        })
        .collect()
}

/// The socket inode a `/proc/<pid>/fd` link points at ("socket:[12345]").
pub fn socket_inode(link: &str) -> Option<u64> {
    link.strip_prefix("socket:[")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

/// `lsof -nP -a -iTCP -sTCP:LISTEN -F pcn -p …` output: `p<pid>`,
/// `c<command>` and `n<address>` lines ("*:3000", "127.0.0.1:5173",
/// "[::1]:8080").
pub fn lsof_listeners(output: &str) -> Vec<ListeningPort> {
    let mut found = Vec::new();
    let (mut pid, mut process) = (0, String::new());
    for line in output.lines() {
        let Some(kind) = line.chars().next() else {
            continue;
        };
        let value = &line[kind.len_utf8()..];
        match kind {
            'p' => {
                pid = value.parse().unwrap_or(0);
                process.clear();
            }
            'c' => process = value.to_string(),
            'n' => {
                if let Some(port) = value.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()) {
                    if port > 0 && pid > 0 {
                        found.push(ListeningPort {
                            port,
                            pid,
                            process: process.clone(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    merge(found)
}

/// One entry per port (the first process found for it), by port.
pub fn merge(ports: impl IntoIterator<Item = ListeningPort>) -> Vec<ListeningPort> {
    let mut by_port: BTreeMap<u16, ListeningPort> = BTreeMap::new();
    for port in ports {
        by_port.entry(port.port).or_insert(port);
    }
    by_port.into_values().collect()
}

/// A workspace row's port chip: ":3000", or ":3000+" when there are more.
pub fn chip(ports: &[ListeningPort]) -> Option<String> {
    let first = ports.first()?;
    Some(if ports.len() > 1 {
        format!(":{}+", first.port)
    } else {
        format!(":{}", first.port)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_listening_sockets_from_proc() {
        let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000:0BB8 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 41234 1 0000000000000000 100 0 0 10 0
   1: 0100007F:1435 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 41250 1 0000000000000000 100 0 0 10 0
   2: 0100007F:9C40 0100007F:0BB8 01 00000000:00000000 00:00000000 00000000  1000        0 41300 1 0000000000000000 20 4 30 10 -1";
        assert_eq!(
            proc_net_listeners(table),
            vec![(41234, 3000), (41250, 5173)]
        );
        let tcp6 = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000000000000:1F90 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 50001 1 0000000000000000 100 0 0 10 0";
        assert_eq!(proc_net_listeners(tcp6), vec![(50001, 8080)]);
        assert_eq!(socket_inode("socket:[41234]"), Some(41234));
        assert_eq!(socket_inode("pipe:[9]"), None);
        assert_eq!(socket_inode("/dev/null"), None);
    }

    #[test]
    fn reads_lsof_and_merges_by_port() {
        let output = "p4242\ncnode\nn*:3000\nn[::1]:3000\np5151\ncpython3.12\nn127.0.0.1:8000\n";
        let ports = lsof_listeners(output);
        assert_eq!(
            ports,
            vec![
                ListeningPort {
                    port: 3000,
                    pid: 4242,
                    process: "node".into()
                },
                ListeningPort {
                    port: 8000,
                    pid: 5151,
                    process: "python3.12".into()
                },
            ]
        );
        assert_eq!(chip(&ports).as_deref(), Some(":3000+"));
        assert_eq!(chip(&ports[1..]).as_deref(), Some(":8000"));
        assert_eq!(chip(&[]), None);
    }
}
