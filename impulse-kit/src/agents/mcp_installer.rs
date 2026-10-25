//! Registering Impulse's MCP server (`impulse mcp`) with an agent. The entry
//! runs the bundled CLI through $IMPULSE_CLI, which only Impulse terminals
//! set, falling back to the app's own copy so the server still starts (with
//! no tools) in any other terminal: registered for every project, it never
//! shows up as broken.
//!
//! Claude Code keeps its servers in ~/.claude.json, a large file it rewrites
//! itself while it runs, so Impulse only reads it (for the status) and
//! changes it with `claude mcp add-json` / `claude mcp remove`. Codex's
//! config.toml gets an `[mcp_servers.impulse]` table, with the same preview,
//! backup and Remove as its notify hook.
//!
//! opencode 2 starts MCP servers in its background service, which every
//! terminal shares, so a server there can't tell which pane is asking:
//! Impulse's opencode plugin gives it the same tools instead. All that's
//! left here is taking out the `impulse` entry under `mcp` that older
//! Impulses added to opencode's config (JSON, or JSONC with comments),
//! edited in place so nothing else in the file changes.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

use serde_json::{json, Map, Value};

use super::hook_installer::{InstallError, Status};
use super::jsonc_edit::{self, Malformed, Member, Object};
use crate::git::task_registry::{join, sorted_keys};
use crate::git::{trim_blanks, trim_newlines};

pub const SERVER_NAME: &str = "impulse";

/// What the server needs to find the window and pane it's called from.
pub const FORWARDED_VARIABLES: [&str; 3] = ["IMPULSE_SOCKET", "IMPULSE_PANE_TOKEN", "IMPULSE_CLI"];

/// Where the app usually is, when the running copy can't say.
pub const DEFAULT_CLI: &str = "/Applications/Impulse.app/Contents/Resources/bin/impulse";

/// The `sh -c` script that starts the server: the CLI of the Impulse the
/// terminal belongs to, else `fallback_cli`. A `}` would end the default
/// of `${IMPULSE_CLI:-…}`, so a path with one is set aside in a variable
/// first.
pub fn launcher(fallback_cli: &str) -> String {
    if fallback_cli.contains('}') {
        let quoted = fallback_cli.replace('\'', "'\\''");
        return format!("cli='{quoted}'; exec \"${{IMPULSE_CLI:-$cli}}\" mcp");
    }
    let mut escaped = String::new();
    for c in fallback_cli.chars() {
        // Inside double quotes, these four are special to sh.
        if "\"\\$`".contains(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    format!("exec \"${{IMPULSE_CLI:-{escaped}}}\" mcp")
}

// Claude Code

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaudeScope {
    /// Every project (`--scope user`).
    User,
    /// This project only, for you alone (`--scope local`); never
    /// `.mcp.json`, which would give every clone a server that only works
    /// in Impulse.
    Local,
}

impl ClaudeScope {
    pub fn as_str(self) -> &'static str {
        match self {
            ClaudeScope::User => "user",
            ClaudeScope::Local => "local",
        }
    }
}

/// The server's entry, as `claude mcp add-json` takes it.
pub fn claude_server(fallback_cli: &str) -> Value {
    let env: Map<String, Value> = FORWARDED_VARIABLES
        .iter()
        .map(|name| (name.to_string(), Value::from(format!("${{{name}:-}}"))))
        .collect();
    json!({
        "type": "stdio",
        "command": "sh",
        "args": ["-c", launcher(fallback_cli)],
        "env": env,
    })
}

pub fn claude_server_json(fallback_cli: &str) -> String {
    serde_json::to_string(&sorted_keys(claude_server(fallback_cli))).unwrap_or_else(|_| "{}".into())
}

pub fn claude_add_arguments(scope: ClaudeScope, fallback_cli: &str) -> Vec<String> {
    vec![
        "mcp".into(),
        "add-json".into(),
        "--scope".into(),
        scope.as_str().into(),
        SERVER_NAME.into(),
        claude_server_json(fallback_cli),
    ]
}

pub fn claude_remove_arguments(scope: ClaudeScope) -> Vec<String> {
    vec![
        "mcp".into(),
        "remove".into(),
        "--scope".into(),
        scope.as_str().into(),
        SERVER_NAME.into(),
    ]
}

/// Whether ~/.claude.json has Impulse's server for `scope` (`project`: the
/// folder a local server belongs to): the current entry, an older one
/// (another fallback path, a hand-edited entry), or none.
pub fn claude_status(
    json: Option<&[u8]>,
    scope: ClaudeScope,
    project: Option<&str>,
    fallback_cli: &str,
) -> Status {
    let Some(config) = json.and_then(|j| serde_json::from_slice::<Value>(j).ok()) else {
        return Status::NotInstalled;
    };
    let servers = match scope {
        ClaudeScope::User => config.get("mcpServers"),
        ClaudeScope::Local => {
            project.and_then(|p| config.get("projects")?.get(p)?.get("mcpServers"))
        }
    };
    let Some(entry) = servers
        .and_then(|s| s.get(SERVER_NAME))
        .and_then(Value::as_object)
    else {
        return Status::NotInstalled;
    };
    let args: Option<Vec<&str>> = entry
        .get("args")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().map(Value::as_str).collect());
    let current = entry.get("command").and_then(Value::as_str) == Some("sh")
        && args == Some(vec!["-c", launcher(fallback_cli).as_str()]);
    if current {
        Status::Installed
    } else {
        Status::Outdated
    }
}

// Codex

const CODEX_COMMENT: &str =
    "# Impulse's MCP server: tasks, review comments, problems (no tools outside Impulse).";
const CODEX_HEADER: &str = "[mcp_servers.impulse]";

/// Codex gives up on a tool call after 5 minutes (progress doesn't reset
/// it), and create_task and finish_task wait for the user: an hour.
pub const CODEX_TOOL_TIMEOUT: u32 = 3600;

/// The table Impulse adds to config.toml.
pub fn codex_table(fallback_cli: &str) -> String {
    let quoted = |text: &str| format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""));
    let variables: Vec<String> = FORWARDED_VARIABLES.iter().map(|v| quoted(v)).collect();
    [
        CODEX_COMMENT.to_string(),
        CODEX_HEADER.to_string(),
        "command = \"sh\"".to_string(),
        format!("args = [\"-c\", {}]", quoted(&launcher(fallback_cli))),
        format!("env_vars = [{}]", variables.join(", ")),
        format!("tool_timeout_sec = {CODEX_TOOL_TIMEOUT}"),
    ]
    .join("\n")
        + "\n"
}

/// A line without its line ending, surrounding blanks and trailing comment
/// (a `#` outside quotes is rare enough in these lines to cut at).
fn bare(line: &str) -> &str {
    line.split('#').next().unwrap_or("").trim()
}

/// `[mcp_servers.impulse]`, however it's quoted or spaced.
fn is_impulse_header(line: &str) -> bool {
    let compact: String = bare(line)
        .chars()
        .filter(|&c| c != ' ' && c != '\t')
        .collect();
    [
        "[mcp_servers.impulse]",
        "[mcp_servers.\"impulse\"]",
        "[mcp_servers.'impulse']",
    ]
    .contains(&compact.as_str())
}

fn is_blank_line(line: &str) -> bool {
    line.trim().is_empty()
}

/// The lines of `toml` that hold the `[mcp_servers.impulse]` table (with
/// Impulse's comment above it, when it's there), and whether it's
/// Impulse's; `None` without one.
fn codex_table_range(lines: &[&str]) -> Option<(Range<usize>, bool)> {
    let header = lines.iter().position(|l| is_impulse_header(l))?;
    let mut end = header + 1;
    while end < lines.len() && !trim_blanks(lines[end]).starts_with('[') {
        end += 1;
    }
    // Blank lines before the next table aren't the table's, but the one
    // after it is (installing put it there).
    while end > header + 1 && is_blank_line(lines[end - 1]) {
        end -= 1;
    }
    let is_ours = header > 0
        && lines[header - 1].trim() == CODEX_COMMENT
        && lines[header].trim() == CODEX_HEADER;
    Some((if is_ours { header - 1 } else { header }..end, is_ours))
}

/// `mcp_servers.impulse` defined some other way than its own table: a
/// dotted key, or `impulse = { ... }` in `[mcp_servers]`. A second
/// definition would stop Codex from reading the file.
fn defines_server_elsewhere(lines: &[&str]) -> bool {
    let mut in_servers = false;
    for line in lines {
        let content = bare(line);
        if content.starts_with('[') {
            in_servers =
                content.chars().filter(|&c| c != ' ').collect::<String>() == "[mcp_servers]";
            continue;
        }
        let key: String = content.chars().filter(|&c| c != ' ' && c != '\t').collect();
        if key.starts_with("mcp_servers.impulse") || key.starts_with("mcp_servers.\"impulse\"") {
            return true;
        }
        if in_servers
            && (key.starts_with("impulse=")
                || key.starts_with("impulse.")
                || key.starts_with("\"impulse\""))
        {
            return true;
        }
    }
    false
}

pub fn codex_status(toml: Option<&str>, fallback_cli: &str) -> Status {
    let Some(toml) = toml else {
        return Status::NotInstalled;
    };
    let lines: Vec<&str> = toml.split('\n').collect();
    let Some((range, is_ours)) = codex_table_range(&lines) else {
        return if defines_server_elsewhere(&lines) {
            Status::Outdated
        } else {
            Status::NotInstalled
        };
    };
    let table = lines[range]
        .iter()
        .map(|l| trim_newlines(l))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    if is_ours && table == codex_table(fallback_cli) {
        Status::Installed
    } else {
        Status::Outdated
    }
}

/// config.toml with Impulse's table at the end (an older one of Impulse's
/// replaced). Refuses when `mcp_servers.impulse` is defined in a way
/// Impulse didn't write.
pub fn installing_codex_mcp(
    toml: Option<&str>,
    fallback_cli: &str,
) -> Result<String, InstallError> {
    let mut text = toml.unwrap_or("").to_string();
    let lines: Vec<&str> = text.split('\n').collect();
    if let Some((_, is_ours)) = codex_table_range(&lines) {
        if !is_ours {
            return Err(InstallError::InUse(
                "config.toml already has an [mcp_servers.impulse] table; change it by hand".into(),
            ));
        }
        text = removing_codex_mcp(&text).unwrap_or(text);
    } else if defines_server_elsewhere(&lines) {
        return Err(InstallError::InUse(
            "config.toml already defines mcp_servers.impulse; change it by hand".into(),
        ));
    }
    let table = codex_table(fallback_cli);
    if text.is_empty() {
        return Ok(table);
    }
    if !text.ends_with('\n') {
        text.push('\n');
    }
    Ok(format!("{text}\n{table}"))
}

/// config.toml without Impulse's table (and the blank line before it);
/// `None` when the table isn't Impulse's (or isn't there).
pub fn removing_codex_mcp(toml: &str) -> Option<String> {
    let mut lines: Vec<&str> = toml.split('\n').collect();
    let (mut range, is_ours) = codex_table_range(&lines)?;
    if !is_ours {
        return None;
    }
    // The blank line installing put before it; at the top of the file, the
    // one after it instead.
    if range.start > 0 && is_blank_line(lines[range.start - 1]) {
        range.start -= 1;
    } else if range.start == 0 && range.end + 1 < lines.len() && is_blank_line(lines[range.end]) {
        range.end += 1;
    }
    lines.drain(range);
    Some(lines.join("\n"))
}

// opencode

/// opencode's global config folder: `$XDG_CONFIG_HOME/opencode`, else
/// `~/.config/opencode` (on macOS too).
pub fn opencode_config_directory(environment: &HashMap<String, String>, home: &str) -> String {
    match environment
        .get("XDG_CONFIG_HOME")
        .filter(|x| x.starts_with('/'))
    {
        Some(xdg) => join(xdg, "opencode"),
        None => join(home, ".config/opencode"),
    }
}

/// The global config file opencode itself would write to (it reads all
/// three): opencode.jsonc, opencode.json or config.json, whichever exists
/// first; a new opencode.json when there's none.
pub fn opencode_config_file(directory: &str, exists: impl Fn(&str) -> bool) -> String {
    let candidates: Vec<String> = ["opencode.jsonc", "opencode.json", "config.json"]
        .iter()
        .map(|name| join(directory, name))
        .collect();
    candidates
        .iter()
        .find(|path| exists(path))
        .unwrap_or(&candidates[1])
        .clone()
}

/// `opencode_config_file` looking on disk.
pub fn opencode_config_file_on_disk(directory: &str) -> String {
    opencode_config_file(directory, |path| Path::new(path).exists())
}

struct OpencodeEntry {
    root: Object,
    mcp: Option<Member>,
    servers: Option<Object>,
    entry: Option<Member>,
}

/// The `mcp` object and Impulse's entry in it, if the text has them.
fn opencode_entry(bytes: &[u8]) -> Result<OpencodeEntry, Malformed> {
    let root = jsonc_edit::root(bytes)?;
    let Some(mcp) = root.member("mcp").cloned() else {
        return Ok(OpencodeEntry {
            root,
            mcp: None,
            servers: None,
            entry: None,
        });
    };
    if bytes.get(mcp.value_start) != Some(&b'{') {
        return Err(Malformed);
    }
    let servers = jsonc_edit::object(bytes, mcp.value_start)?;
    let entry = servers.member(SERVER_NAME).cloned();
    Ok(OpencodeEntry {
        root,
        mcp: Some(mcp),
        servers: Some(servers),
        entry,
    })
}

/// An entry an older Impulse wrote (with any fallback path), rather than
/// another server someone named impulse.
fn is_our_opencode_entry(value: Option<Value>) -> bool {
    let Some(entry) = value.as_ref().and_then(Value::as_object) else {
        return false;
    };
    if entry.get("type").and_then(Value::as_str) != Some("local") {
        return false;
    }
    let Some(command) = entry
        .get("command")
        .and_then(Value::as_array)
        .and_then(|c| c.iter().map(Value::as_str).collect::<Option<Vec<&str>>>())
    else {
        return false;
    };
    command.len() == 3
        && command[0] == "sh"
        && command[1] == "-c"
        && command[2].contains("IMPULSE_CLI")
        && command[2].ends_with(" mcp")
}

/// Whether opencode's config (`None` when there's none) still has the MCP
/// entry an older Impulse added.
pub fn opencode_has_old_entry(text: Option<&str>) -> bool {
    let bytes = text.unwrap_or("").as_bytes();
    let Ok(found) = opencode_entry(bytes) else {
        return false;
    };
    let Some(entry) = found.entry else {
        return false;
    };
    is_our_opencode_entry(jsonc_edit::value(bytes, entry.value_start, entry.value_end))
}

/// The config without that entry (and without an `mcp` it leaves empty);
/// `None` when the entry isn't Impulse's (or isn't there).
pub fn removing_opencode_mcp(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let found = opencode_entry(bytes).ok()?;
    let (servers, entry) = (found.servers?, found.entry?);
    if !is_our_opencode_entry(jsonc_edit::value(bytes, entry.value_start, entry.value_end)) {
        return None;
    }
    let mut result = jsonc_edit::applying(&jsonc_edit::removing(&entry, &servers, bytes), bytes);
    if let Ok(left) = opencode_entry(&result) {
        if let (Some(mcp), Some(servers)) = (&left.mcp, &left.servers) {
            let empty = servers.members.is_empty()
                && result[servers.open + 1..servers.close]
                    .iter()
                    .all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'));
            if empty {
                result =
                    jsonc_edit::applying(&jsonc_edit::removing(mcp, &left.root, &result), &result);
            }
        }
    }
    String::from_utf8(result).ok()
}

// Other agents

/// The entry for agents Impulse doesn't configure itself, in the
/// `mcpServers` form most of them read (their own config files differ).
pub fn generic_snippet(fallback_cli: &str) -> String {
    let object = json!({ "mcpServers": { SERVER_NAME: claude_server(fallback_cli) } });
    serde_json::to_string_pretty(&sorted_keys(object)).unwrap_or_else(|_| "{}".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLI: &str = DEFAULT_CLI;

    #[test]
    fn the_launcher_falls_back_to_the_apps_cli() {
        assert_eq!(
            launcher(CLI),
            r#"exec "${IMPULSE_CLI:-/Applications/Impulse.app/Contents/Resources/bin/impulse}" mcp"#
        );
        assert_eq!(
            launcher(r#"/A "b"/$x"#),
            r#"exec "${IMPULSE_CLI:-/A \"b\"/\$x}" mcp"#
        );
        // sh runs the CLI a `}` (or a quote) in the path names.
        let dir = std::env::temp_dir().join(format!("impulse-launcher-{}", std::process::id()));
        let cli = dir.join("a}b's").join("impulse");
        std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
        std::fs::write(&cli, "#!/bin/sh\necho \"ran $1\"\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(launcher(&cli.to_string_lossy()))
            .env_remove("IMPULSE_CLI")
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(String::from_utf8_lossy(&output.stdout), "ran mcp\n");
        let args = claude_add_arguments(ClaudeScope::User, CLI);
        assert_eq!(args[..5], ["mcp", "add-json", "--scope", "user", "impulse"]);
        let entry: Value = serde_json::from_str(&args[5]).unwrap();
        assert_eq!(entry["command"], "sh");
        assert_eq!(
            entry["env"]["IMPULSE_PANE_TOKEN"],
            "${IMPULSE_PANE_TOKEN:-}"
        );
        assert_eq!(
            claude_remove_arguments(ClaudeScope::Local),
            ["mcp", "remove", "--scope", "local", "impulse"]
        );
        let snippet: Value = serde_json::from_str(&generic_snippet(CLI)).unwrap();
        assert_eq!(snippet["mcpServers"]["impulse"], claude_server(CLI));
    }

    #[test]
    fn claude_status_reads_its_config() {
        let current = claude_server(CLI);
        let config = |servers: Value, project: Option<&str>| -> Vec<u8> {
            let mut object = json!({ "numStartups": 3 });
            match project {
                Some(project) => object["projects"] = json!({ project: { "mcpServers": servers } }),
                None => object["mcpServers"] = servers,
            }
            serde_json::to_vec(&object).unwrap()
        };
        let servers = json!({ "impulse": current });
        assert_eq!(
            claude_status(None, ClaudeScope::User, None, CLI),
            Status::NotInstalled
        );
        assert_eq!(
            claude_status(
                Some(&config(servers.clone(), None)),
                ClaudeScope::User,
                None,
                CLI
            ),
            Status::Installed
        );
        assert_eq!(
            claude_status(
                Some(&config(servers.clone(), None)),
                ClaudeScope::User,
                None,
                "/elsewhere"
            ),
            Status::Outdated
        );
        assert_eq!(
            claude_status(
                Some(&config(servers.clone(), Some("/r"))),
                ClaudeScope::Local,
                Some("/r"),
                CLI
            ),
            Status::Installed
        );
        assert_eq!(
            claude_status(
                Some(&config(servers, Some("/r"))),
                ClaudeScope::User,
                Some("/r"),
                CLI
            ),
            Status::NotInstalled
        );
    }

    #[test]
    fn codex_table_installs_and_removes_cleanly() {
        let original = "model = \"o3\"\n\n[profiles.fast]\nmodel = \"o4-mini\"\n";
        let installed = installing_codex_mcp(Some(original), CLI).unwrap();
        assert!(installed.starts_with(original));
        assert!(installed.contains("[mcp_servers.impulse]\ncommand = \"sh\""));
        assert!(installed
            .contains(r#"env_vars = ["IMPULSE_SOCKET", "IMPULSE_PANE_TOKEN", "IMPULSE_CLI"]"#));
        assert!(
            installed.contains("\ntool_timeout_sec = 3600\n"),
            "create_task waits for the user longer than Codex's 5 minutes"
        );
        assert_eq!(codex_status(Some(&installed), CLI), Status::Installed);
        assert_eq!(
            codex_status(Some(&installed), "/elsewhere"),
            Status::Outdated
        );
        assert_eq!(removing_codex_mcp(&installed).as_deref(), Some(original));
        // Installing again replaces Impulse's own table.
        let again = installing_codex_mcp(Some(&installed), "/elsewhere").unwrap();
        assert_eq!(again.matches("[mcp_servers.impulse]").count(), 1);
        // From nothing, and back.
        let fresh = installing_codex_mcp(None, CLI).unwrap();
        assert_eq!(removing_codex_mcp(&fresh).as_deref(), Some(""));
        // A table the user wrote is theirs.
        let theirs = "[mcp_servers.impulse]\ncommand = \"impulse\"\n";
        assert!(installing_codex_mcp(Some(theirs), CLI).is_err());
        assert_eq!(removing_codex_mcp(theirs), None);
        assert_eq!(codex_status(Some(theirs), CLI), Status::Outdated);
        assert_eq!(codex_status(None, CLI), Status::NotInstalled);
    }

    #[test]
    fn codex_server_defined_otherwise_is_left_alone() {
        // Another spelling of the table, a dotted key, or a key in
        // [mcp_servers]: a second definition would stop Codex reading the
        // file.
        for theirs in [
            "[mcp_servers.\"impulse\"]\ncommand = \"impulse\"\n",
            "[ mcp_servers.impulse ] # mine\ncommand = \"impulse\"\n",
            "mcp_servers.impulse.command = \"impulse\"\n",
            "[mcp_servers]\nimpulse = { command = \"impulse\" }\n",
        ] {
            assert!(installing_codex_mcp(Some(theirs), CLI).is_err(), "{theirs}");
            assert_eq!(
                codex_status(Some(theirs), CLI),
                Status::Outdated,
                "{theirs}"
            );
            assert_eq!(removing_codex_mcp(theirs), None, "{theirs}");
        }
        // Other servers are someone else's business.
        let others = "[mcp_servers]\nimpulsive = { command = \"x\" }\n\n[mcp_servers.docs]\ncommand = \"d\"\n";
        assert_eq!(codex_status(Some(others), CLI), Status::NotInstalled);
        assert!(installing_codex_mcp(Some(others), CLI).is_ok());
    }

    #[test]
    fn codex_table_in_a_crlf_file() {
        let original = "model = \"o3\"\r\n";
        let installed = installing_codex_mcp(Some(original), CLI).unwrap();
        let crlf = installed.replace("\r\n", "\n").replace('\n', "\r\n");
        assert_eq!(codex_status(Some(&crlf), CLI), Status::Installed);
        let removed = removing_codex_mcp(&crlf).unwrap();
        assert!(!removed.contains("mcp_servers"));
        assert!(removed.starts_with("model = \"o3\""));
        // Installing again doesn't add a second table.
        let again = installing_codex_mcp(Some(&crlf), "/elsewhere").unwrap();
        assert_eq!(again.matches("[mcp_servers.impulse]").count(), 1);
    }

    /// What an older Impulse added under `mcp` (for opencode 1).
    fn old_entry(cli: &str) -> String {
        format!(
            r#"{{ "type": "local", "command": ["sh", "-c", "exec \"${{IMPULSE_CLI:-{cli}}}\" mcp"] }}"#
        )
    }

    #[test]
    fn opencode_config_loses_the_old_entry_and_nothing_else() {
        let original = r#"{
  // Shared with the desktop app.
  "$schema": "https://opencode.ai/config.json",
  "model": "anthropic/claude-sonnet", /* for now */
  "mcp": {
    "docs": {
      "type": "remote",
      "url": "https://docs.example.com//mcp", // not a comment inside the string
      "headers": { "X-Key": "{env:DOCS_KEY}" },
    },
  },
}
"#;
        assert!(!opencode_has_old_entry(Some(original)));
        assert_eq!(removing_opencode_mcp(original), None);
        let with_entry = original.replace(
            "    },\n  },\n}",
            &format!(
                "    }},\n    \"impulse\": {},\n  }},\n}}",
                old_entry("/Old/impulse")
            ),
        );
        assert!(
            opencode_has_old_entry(Some(&with_entry)),
            "any fallback path"
        );
        assert_eq!(
            removing_opencode_mcp(&with_entry).as_deref(),
            Some(original)
        );
    }

    #[test]
    fn opencode_config_with_only_the_old_entry_keeps_the_rest() {
        let fresh = format!(
            "{{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"mcp\": {{\n    \"impulse\": {}\n  }}\n}}\n",
            old_entry(CLI)
        );
        assert!(opencode_has_old_entry(Some(&fresh)));
        assert_eq!(
            removing_opencode_mcp(&fresh).as_deref(),
            Some("{\n  \"$schema\": \"https://opencode.ai/config.json\"\n}\n")
        );
        let one_line = format!(
            r#"{{"theme": "opencode", "mcp": {{"impulse": {}}}}}"#,
            old_entry(CLI)
        );
        let removed = removing_opencode_mcp(&one_line).unwrap();
        assert!(
            serde_json::from_str::<Value>(&removed).is_ok(),
            "plain JSON still: {removed}"
        );
        assert!(!opencode_has_old_entry(Some(&removed)));
    }

    #[test]
    fn opencode_config_keeps_someone_elses_impulse_server() {
        let theirs = "{\n  \"mcp\": {\n    \"impulse\": { \"type\": \"remote\", \"url\": \"https://impulse.example\" }\n  }\n}\n";
        assert!(!opencode_has_old_entry(Some(theirs)));
        assert_eq!(removing_opencode_mcp(theirs), None);
        for broken in [
            "{ \"mcp\": { \"a\": 1 }",
            "[1, 2]",
            "{ \"mcp\": [] }",
            "{ /* unterminated }",
            "{ \"a\": tru e }",
        ] {
            assert!(!opencode_has_old_entry(Some(broken)), "{broken}");
            assert_eq!(removing_opencode_mcp(broken), None, "{broken}");
        }
        assert!(!opencode_has_old_entry(None));
    }

    #[test]
    fn opencode_config_lives_where_opencode_looks() {
        let env = |pairs: &[(&str, &str)]| -> HashMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        assert_eq!(
            opencode_config_directory(&env(&[]), "/Users/me"),
            "/Users/me/.config/opencode"
        );
        assert_eq!(
            opencode_config_directory(&env(&[("XDG_CONFIG_HOME", "/x/config")]), "/Users/me"),
            "/x/config/opencode"
        );
        assert_eq!(
            opencode_config_directory(&env(&[("XDG_CONFIG_HOME", "relative")]), "/Users/me"),
            "/Users/me/.config/opencode"
        );
        let dir = "/Users/me/.config/opencode";
        assert_eq!(
            opencode_config_file(dir, |_| false),
            format!("{dir}/opencode.json")
        );
        assert_eq!(
            opencode_config_file(dir, |p| p.ends_with("config.json")),
            format!("{dir}/config.json")
        );
        assert_eq!(
            opencode_config_file(dir, |p| p.ends_with(".json") || p.ends_with(".jsonc")),
            format!("{dir}/opencode.jsonc")
        );
    }
}
