//! A task's own environment: the values that have to differ between the
//! main checkout and each task so their dev stacks don't collide (ports,
//! database names), worked out from the project settings and the task's
//! slot, and written into the task's copy of its dotenv file. The file is
//! the source of truth: the app in its container, `docker compose` from any
//! terminal and Vite all read it.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, TcpListener};
use std::ops::RangeInclusive;

use super::config::ProjectConfig;
use crate::git::trim_blanks;

/// One value to write: `APP_PORT=8100`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvValue {
    pub key: String,
    pub value: String,
}

impl EnvValue {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

/// The port numbers there are.
pub const PORT_RANGE: RangeInclusive<i64> = 1..=65535;

/// `port` for `slot`: plus `slot` times `offset`. `None` when that isn't a
/// port (past 65535, or too big to work out): the slot has no such port.
pub fn port(port: i64, slot: i64, offset: i64) -> Option<i64> {
    let moved = port.checked_add(slot.checked_mul(offset)?)?;
    PORT_RANGE.contains(&moved).then_some(moved)
}

/// The ports for `slot`: each of the main checkout's ports plus `slot`
/// times `offset`. Slot 0 is the main checkout. `None` when one of them
/// isn't a port, so the slot can't be used.
pub fn ports(
    base: &BTreeMap<String, i64>,
    slot: i64,
    offset: i64,
) -> Option<BTreeMap<String, i64>> {
    base.iter()
        .map(|(name, &value)| Some((name.clone(), port(value, slot, offset)?)))
        .collect()
}

/// `template` with `{task}` (the task's folder name), `{task_}` (the same
/// with dashes as underscores, for database names), `{slot}` and each
/// port's `{NAME}` filled in. Unknown placeholders are left as they are.
pub fn expand(template: &str, task: &str, slot: i64, ports: &BTreeMap<String, i64>) -> String {
    let mut result = template
        .replace("{task_}", &underscored(task))
        .replace("{task}", &plain_name(task))
        .replace("{slot}", &slot.to_string());
    for (name, port) in ports {
        result = result.replace(&format!("{{{name}}}"), &port.to_string());
    }
    result
}

/// A task's values: its ports, then the `[worktrees.env]` values, each
/// sorted by name. A port the slot doesn't have (the settings changed since
/// the slot was given) is left out.
pub fn values(config: &ProjectConfig, task: &str, slot: i64) -> Vec<EnvValue> {
    let ports: BTreeMap<String, i64> = config
        .ports
        .iter()
        .filter_map(|(name, &value)| Some((name.clone(), port(value, slot, config.port_offset)?)))
        .collect();
    let mut values: Vec<EnvValue> = ports
        .iter()
        .map(|(name, port)| EnvValue::new(name.clone(), port.to_string()))
        .collect();
    values.extend(
        config.worktree_env.iter().map(|(name, template)| {
            EnvValue::new(name.clone(), expand(template, task, slot, &ports))
        }),
    );
    values
}

/// `text` (a dotenv file) with `values` set: a key already in it is changed
/// in place (every line that sets it), a missing one is added at the end
/// under `comment`. Everything else stays as it is.
pub fn applying(values: &[EnvValue], text: &str, comment: &str) -> String {
    let mut lines: Vec<String> = if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').map(String::from).collect()
    };
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let mut missing: Vec<&EnvValue> = Vec::new();
    for value in values {
        let mut found = false;
        for line in lines.iter_mut() {
            if key_of(line).as_deref() != Some(value.key.as_str()) {
                continue;
            }
            let export = if trim_blanks(line).starts_with("export ") {
                "export "
            } else {
                ""
            };
            *line = format!("{export}{}={}", value.key, quoted(&value.value));
            found = true;
        }
        if !found {
            missing.push(value);
        }
    }
    if !missing.is_empty() {
        if lines.last().is_some_and(|l| !trim_blanks(l).is_empty()) {
            lines.push(String::new());
        }
        lines.push(format!("# {comment}"));
        lines.extend(
            missing
                .iter()
                .map(|v| format!("{}={}", v.key, quoted(&v.value))),
        );
    }
    lines.join("\n") + "\n"
}

/// The key a dotenv line sets (`KEY=...` or `export KEY=...`), `None` for
/// comments and blank lines.
pub fn key_of(line: &str) -> Option<String> {
    let mut text = trim_blanks(line);
    if text.is_empty() || text.starts_with('#') {
        return None;
    }
    if let Some(rest) = text.strip_prefix("export ") {
        text = trim_blanks(rest);
    }
    let equals = text.find('=')?;
    let key = trim_blanks(&text[..equals]);
    (!key.is_empty()).then(|| key.to_string())
}

/// A value as dotenv needs it: bare when it's simple, double-quoted when it
/// has spaces, `#`, quotes or `$` (so `${OTHER}` still expands where the
/// reader expands it). Line breaks and other control characters are
/// dropped: the file holds one value per line.
pub fn quoted(value: &str) -> String {
    let value: String = value
        .chars()
        .filter(|&c| c == '\t' || (c >= ' ' && c != '\u{7F}'))
        .collect();
    if !value.contains([' ', '\t', '#', '"', '\'', '$', '\\']) {
        return value;
    }
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// `interia-upgrade` to `interia_upgrade`; anything but letters, digits
/// and underscores becomes an underscore.
pub fn underscored(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// A task's folder name as `{task}` gives it: letters, digits, `.`, `_` and
/// `-`, anything else as a dash. A branch from a remote can be named
/// anything (`feat$(curl ...)`), and the name goes into env files and the
/// commands that read them.
pub fn plain_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Whether a TCP port is free on this machine: nothing can be bound to it
/// on any address, as a dev server or a container's published port would
/// be.
pub fn port_is_free(port: i64) -> bool {
    let Ok(port) = u16::try_from(port) else {
        return false;
    };
    port != 0
        && TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).is_ok()
        && TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::config::{parse_layer, resolve};
    use crate::project::folder_clone;
    use std::fs;
    use std::process::Command;

    const SETTINGS: &str = r#"
[worktrees]
port_offset = 100

[worktrees.ports]
APP_PORT = 8000
VITE_PORT = 5173

[worktrees.env]
DB_DATABASE = "pulseboard_{task_}"
APP_URL = "http://localhost:{APP_PORT}"
COMPOSE_PROJECT_NAME = "pulseboard-{task}"
"#;

    fn map<V: Clone>(items: &[(&str, V)]) -> BTreeMap<String, V> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn the_settings_are_read() {
        let config = ProjectConfig::parse(SETTINGS).unwrap();
        assert_eq!(config.port_offset, 100);
        assert_eq!(
            config.ports,
            map(&[("APP_PORT", 8000), ("VITE_PORT", 5173)])
        );
        assert_eq!(config.worktree_env["DB_DATABASE"], "pulseboard_{task_}");
        assert_eq!(config.env_file, ".env", "the default");
        assert!(config.has_task_values());
        assert_eq!(
            config.task_value_names(),
            [
                "APP_PORT",
                "VITE_PORT",
                "APP_URL",
                "COMPOSE_PROJECT_NAME",
                "DB_DATABASE"
            ]
        );
        assert!(!ProjectConfig::parse("").unwrap().has_task_values());
    }

    #[test]
    fn a_local_file_merges_ports_and_values_key_by_key() {
        let local = "[worktrees]\nenv_file = \".env.local\"\n\n[worktrees.ports]\nVITE_PORT = 5200\nREDIS_PORT = 6379\n";
        let config = resolve(&[parse_layer(SETTINGS).unwrap(), parse_layer(local).unwrap()]);
        assert_eq!(
            config.ports,
            map(&[
                ("APP_PORT", 8000),
                ("VITE_PORT", 5200),
                ("REDIS_PORT", 6379)
            ])
        );
        assert_eq!(config.worktree_env.len(), 3);
        assert_eq!(config.env_file, ".env.local");
    }

    #[test]
    fn a_tasks_values_come_from_its_slot() {
        let config = ProjectConfig::parse(SETTINGS).unwrap();
        assert_eq!(
            values(&config, "interia-upgrade", 1),
            [
                EnvValue::new("APP_PORT", "8100"),
                EnvValue::new("VITE_PORT", "5273"),
                EnvValue::new("APP_URL", "http://localhost:8100"),
                EnvValue::new("COMPOSE_PROJECT_NAME", "pulseboard-interia-upgrade"),
                EnvValue::new("DB_DATABASE", "pulseboard_interia_upgrade"),
            ]
        );
        assert_eq!(
            ports(&config.ports, 0, 100).as_ref(),
            Some(&config.ports),
            "slot 0 is the main checkout"
        );
        assert_eq!(
            expand("db_{slot}_{UNKNOWN}", "x", 3, &BTreeMap::new()),
            "db_3_{UNKNOWN}"
        );
    }

    #[test]
    fn values_are_changed_in_place_or_added_at_the_end() {
        let text = "APP_NAME=PulseBoard
APP_KEY=base64:secret
APP_PORT=8000
# the database
DB_DATABASE=pulseboard
export VITE_PORT=5173";
        let result = applying(
            &[
                EnvValue::new("APP_PORT", "8100"),
                EnvValue::new("VITE_PORT", "5273"),
                EnvValue::new("DB_DATABASE", "pulseboard_x"),
                EnvValue::new("APP_URL", "http://localhost:8100"),
            ],
            &format!("{text}\n"),
            "Impulse task x",
        );
        assert_eq!(
            result,
            "APP_NAME=PulseBoard
APP_KEY=base64:secret
APP_PORT=8100
# the database
DB_DATABASE=pulseboard_x
export VITE_PORT=5273

# Impulse task x
APP_URL=http://localhost:8100
"
        );
    }

    #[test]
    fn a_missing_file_gets_only_the_values() {
        assert_eq!(
            applying(&[EnvValue::new("APP_PORT", "8100")], "", "Impulse task x"),
            "# Impulse task x\nAPP_PORT=8100\n"
        );
    }

    #[test]
    fn odd_values_are_quoted() {
        assert_eq!(quoted("plain_value-1"), "plain_value-1");
        assert_eq!(quoted("http://localhost:8100"), "http://localhost:8100");
        assert_eq!(
            quoted("postgres://u@db/app?sslmode=disable"),
            "postgres://u@db/app?sslmode=disable"
        );
        assert_eq!(quoted("two words"), "\"two words\"");
        assert_eq!(quoted("a\"b"), "\"a\\\"b\"");
        assert_eq!(quoted("it's"), "\"it's\"");
        assert_eq!(
            quoted("${APP_URL}/api"),
            "\"${APP_URL}/api\"",
            "left for the reader to expand"
        );
        assert_eq!(quoted("a\nb=1\r"), "ab=1", "one value per line");
        assert_eq!(quoted(""), "");
        assert_eq!(key_of("  export  KEY = value").as_deref(), Some("KEY"));
        assert_eq!(key_of("# KEY=value"), None);
        assert_eq!(underscored("feat-search.ui"), "feat_search_ui");
    }

    /// A branch from a remote names the task folder, and so `{task}`.
    #[test]
    fn a_branch_name_cant_run_commands_from_the_env_file() {
        let task = "feat$(touch${IFS}PWNED)`touch PWNED2`";
        assert_eq!(
            expand("app-{task}", task, 1, &BTreeMap::new()),
            "app-feat--touch--IFS-PWNED--touch-PWNED2-"
        );
        assert_eq!(
            expand("app_{task_}", task, 1, &BTreeMap::new()),
            "app_feat__touch__IFS_PWNED__touch_PWNED2_"
        );

        // Sourced by a shell, the task's values run nothing; the settings'
        // own `${NAME}` still expands.
        let dir = tempfile::tempdir().unwrap();
        let config = ProjectConfig {
            ports: map(&[("APP_PORT", 8000)]),
            worktree_env: map(&[
                ("NAME", "app-{task}".to_string()),
                ("DB", "app_{task_}".to_string()),
                ("URL", "http://localhost:{APP_PORT}/${NAME}".to_string()),
                ("DSN", "postgres://u@db/app?sslmode=disable".to_string()),
                ("QUOTE", "it's\nfine".to_string()),
            ]),
            ..ProjectConfig::default()
        };
        let file = dir.path().join(".env");
        fs::write(
            &file,
            applying(&values(&config, task, 1), "", "Impulse task x"),
        )
        .unwrap();
        let output = Command::new("/bin/sh")
            .args([
                "-c",
                r#"set -a; . ./.env; printf '%s\n' "$NAME" "$DB" "$URL" "$DSN" "$QUOTE""#,
            ])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "app-feat--touch--IFS-PWNED--touch-PWNED2-
app_feat__touch__IFS_PWNED__touch_PWNED2_
http://localhost:8100/app-feat--touch--IFS-PWNED--touch-PWNED2-
postgres://u@db/app?sslmode=disable
it'sfine
"
        );
        let made = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("PWNED"))
            .count();
        assert_eq!(made, 0);
    }

    #[test]
    fn ports_past_the_last_one_arent_ports() {
        assert_eq!(port(8000, 1, 100), Some(8100));
        assert_eq!(port(65500, 1, 100), None);
        assert_eq!(port(i64::MAX, 1, 100), None, "no overflow");
        assert_eq!(port(8000, i64::MAX, 2), None);
        assert_eq!(port(8000, 3, i64::MAX), None);
        assert_eq!(port(i64::MIN, 1, -1), None);
        assert_eq!(
            ports(&map(&[("A", 8000), ("B", 65500)]), 1, 100),
            None,
            "the slot can't be used"
        );
        assert_eq!(
            ports(&map(&[("A", 8000), ("B", 5173)]), 2, 100),
            Some(map(&[("A", 8200), ("B", 5373)]))
        );

        let config = ProjectConfig {
            ports: map(&[("A", 8000), ("B", 65500)]),
            ..ProjectConfig::default()
        };
        assert_eq!(
            values(&config, "x", 1),
            [EnvValue::new("A", "8100")],
            "B has no port in slot 1"
        );
    }

    #[test]
    fn ports_and_offsets_that_arent_ports_are_set_aside() {
        let config = ProjectConfig::parse(
            "[worktrees]
port_offset = 9223372036854775807

[worktrees.ports]
APP_PORT = 8000
HUGE = 9223372036854775807
ZERO = 0
HIGH = 70000
NEGATIVE = -9223372036854775808
",
        )
        .unwrap();
        assert_eq!(config.ports, map(&[("APP_PORT", 8000)]));
        assert_eq!(
            config.unusable_ports,
            map(&[
                ("HUGE", i64::MAX),
                ("ZERO", 0),
                ("HIGH", 70000),
                ("NEGATIVE", i64::MIN)
            ])
        );
        assert_eq!(config.port_offset, 100, "the default");
        assert_eq!(config.unusable_port_offset, Some(i64::MAX));
        assert_eq!(values(&config, "x", 1), [EnvValue::new("APP_PORT", "8100")]);
        let offset = |text: &str| ProjectConfig::parse(text).unwrap();
        assert_eq!(
            offset("[worktrees]\nport_offset = 10001").unusable_port_offset,
            Some(10001)
        );
        assert_eq!(
            offset("[worktrees]\nport_offset = 0").unusable_port_offset,
            Some(0)
        );
        assert_eq!(
            offset("[worktrees]\nport_offset = 10000").port_offset,
            10000
        );

        // A later file's usable value replaces an earlier one that wasn't.
        let layered = ProjectConfig::parse_layers(&[
            "[worktrees]\nport_offset = -1\n[worktrees.ports]\nA = 70000",
            "[worktrees]\nport_offset = 50\n[worktrees.ports]\nA = 7000",
        ])
        .unwrap();
        assert!(layered.ports == map(&[("A", 7000)]) && layered.unusable_ports.is_empty());
        assert!(layered.port_offset == 50 && layered.unusable_port_offset.is_none());
    }

    #[test]
    fn clones_and_the_database_folder_are_read() {
        let config = ProjectConfig::parse(
            r#"
[worktrees]
clone = ["vendor", "public/build"]

[worktrees.database]
clone = "docker/data/mysql"
service = "mysql"
"#,
        )
        .unwrap();
        assert_eq!(config.worktree_clone, ["vendor", "public/build"]);
        assert_eq!(config.database_folder.as_deref(), Some("docker/data/mysql"));
        assert_eq!(config.database_service.as_deref(), Some("mysql"));
        assert_eq!(
            config.commands(),
            ["docker compose stop mysql", "docker compose start mysql"],
            "trusting covers them"
        );
    }

    #[test]
    fn folders_are_cloned_with_their_contents() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_string_lossy().into_owned();
        let main = format!("{base}/repo");
        let task = format!("{base}/repo.worktrees/fix");
        fs::create_dir_all(format!("{main}/vendor/pkg")).unwrap();
        fs::create_dir_all(format!("{main}/public/build")).unwrap();
        fs::create_dir_all(format!("{task}/public/build")).unwrap();
        fs::write(format!("{main}/vendor/pkg/a.php"), "lib").unwrap();
        fs::write(format!("{main}/public/build/manifest.json"), "{}").unwrap();
        fs::write(format!("{task}/public/build/.gitkeep"), "tracked").unwrap();

        assert!(folder_clone::clone("vendor", &main, &task));
        assert_eq!(
            fs::read_to_string(format!("{task}/vendor/pkg/a.php")).unwrap(),
            "lib"
        );
        assert!(
            folder_clone::clone("public/build", &main, &task),
            "an existing folder gets what it lacks"
        );
        assert_eq!(
            fs::read_to_string(format!("{task}/public/build/manifest.json")).unwrap(),
            "{}"
        );
        assert_eq!(
            fs::read_to_string(format!("{task}/public/build/.gitkeep")).unwrap(),
            "tracked"
        );
        assert!(
            !folder_clone::clone("node_modules", &main, &task),
            "missing in the main checkout"
        );
        assert!(!folder_clone::clone("../outside", &main, &task));
        // A change in the clone doesn't reach the original.
        fs::write(format!("{task}/vendor/pkg/a.php"), "changed").unwrap();
        assert_eq!(
            fs::read_to_string(format!("{main}/vendor/pkg/a.php")).unwrap(),
            "lib"
        );
    }

    #[test]
    fn a_bound_port_is_not_free() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = i64::from(listener.local_addr().unwrap().port());
        assert!(!port_is_free(port));
        assert!(!port_is_free(0));
        assert!(!port_is_free(70000));
    }
}
