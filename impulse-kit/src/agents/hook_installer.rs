//! Edits agent configuration so the agent tells Impulse what it's doing:
//! Claude Code hooks in settings.json, Codex's `notify` program in
//! config.toml, and a plugin folder in opencode's plugins folder. Every
//! hook calls the bundled CLI through $IMPULSE_CLI, which only Impulse
//! terminals set: anywhere else the hook is a no-op.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Map, Value};

/// The Claude Code events Impulse listens to, each with the tools a tool
/// event is limited to: before shell commands (a merge of a branch still
/// being worked on asks first) and after edits and shell commands (notes
/// about files other tasks change, and dependencies that changed).
/// StopFailure ends a turn an API error stopped, which sends no Stop.
pub const CLAUDE_HOOKS: [(&str, Option<&str>); 7] = [
    ("SessionStart", None),
    ("UserPromptSubmit", None),
    ("Notification", None),
    ("Stop", None),
    ("StopFailure", None),
    ("PreToolUse", Some("Bash")),
    (
        "PostToolUse",
        Some("Edit|Write|MultiEdit|NotebookEdit|Bash"),
    ),
];

pub fn claude_events() -> Vec<&'static str> {
    CLAUDE_HOOKS.iter().map(|(event, _)| *event).collect()
}

/// Whether Impulse's hooks (or its plugin, or its MCP server) are in an
/// agent's configuration: all of them, some or an older version
/// (installed by an older Impulse), or none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    NotInstalled,
    Outdated,
    Installed,
}

/// The shell command each Claude Code hook runs.
pub const CLAUDE_COMMAND: &str = r#"[ -n "$IMPULSE_CLI" ] && "$IMPULSE_CLI" hook claude || true"#;

/// Codex appends its notification JSON as the last argument.
pub const CODEX_NOTIFY: [&str; 4] = [
    "sh",
    "-c",
    r#"[ -n "$IMPULSE_CLI" ] && "$IMPULSE_CLI" hook codex "$1" || true"#,
    "impulse-hook",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// The existing file isn't the JSON/TOML shape expected.
    Unreadable(String),
    /// The place is taken: Codex already runs another notify program, or
    /// something Impulse didn't write is where its entry would go.
    InUse(String),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Unreadable(message) | InstallError::InUse(message) => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for InstallError {}

// Claude Code (settings.json)

fn settings_object(json: Option<&[u8]>) -> Option<Map<String, Value>> {
    match serde_json::from_slice(json?).ok()? {
        Value::Object(object) => Some(object),
        _ => None,
    }
}

/// An array whose items are all objects, as Swift's `as? [[String: Any]]`
/// reads one.
fn objects(value: Option<&Value>) -> Option<Vec<&Map<String, Value>>> {
    value?.as_array()?.iter().map(Value::as_object).collect()
}

fn is_our_claude_entry(entry: &Map<String, Value>) -> bool {
    objects(entry.get("hooks")).is_some_and(|hooks| {
        hooks
            .iter()
            .any(|hook| hook.get("command").and_then(Value::as_str) == Some(CLAUDE_COMMAND))
    })
}

pub fn claude_hooks_installed(json: Option<&[u8]>) -> bool {
    claude_hook_status(json) == Status::Installed
}

pub fn claude_hook_status(json: Option<&[u8]>) -> Status {
    let Some(settings) = settings_object(json) else {
        return Status::NotInstalled;
    };
    let Some(hooks) = settings.get("hooks").and_then(Value::as_object) else {
        return Status::NotInstalled;
    };
    let events = claude_events();
    let present = events
        .iter()
        .filter(|event| {
            objects(hooks.get(**event))
                .is_some_and(|entries| entries.into_iter().any(is_our_claude_entry))
        })
        .count();
    if present == 0 {
        Status::NotInstalled
    } else if present == events.len() {
        Status::Installed
    } else {
        Status::Outdated
    }
}

/// settings.json with Impulse's hooks added (other settings and hooks
/// kept). Already-installed hooks aren't duplicated.
pub fn installing_claude_hooks(json: Option<&[u8]>) -> Result<Vec<u8>, InstallError> {
    let mut settings = Map::new();
    if let Some(json) = json.filter(|j| !j.is_empty()) {
        settings = settings_object(Some(json))
            .ok_or_else(|| InstallError::Unreadable("settings.json isn't a JSON object".into()))?;
    }
    let mut hooks = match settings.remove("hooks") {
        None => Map::new(),
        Some(Value::Object(hooks)) => hooks,
        Some(_) => return Err(InstallError::Unreadable("\"hooks\" isn't an object".into())),
    };
    for (event, matcher) in CLAUDE_HOOKS {
        let mut entries: Vec<Value> = match objects(hooks.get(event)) {
            Some(entries) => entries
                .into_iter()
                .map(|e| Value::Object(e.clone()))
                .collect(),
            None => Vec::new(),
        };
        if !entries
            .iter()
            .filter_map(Value::as_object)
            .any(is_our_claude_entry)
        {
            let mut entry = json!({ "hooks": [{ "type": "command", "command": CLAUDE_COMMAND }] });
            if let Some(matcher) = matcher {
                entry["matcher"] = Value::from(matcher);
            }
            entries.push(entry);
        }
        hooks.insert(event.into(), Value::Array(entries));
    }
    settings.insert("hooks".into(), Value::Object(hooks));
    Ok(serialize(Value::Object(settings)))
}

/// settings.json without Impulse's hooks (empty events and an empty "hooks"
/// object are removed).
pub fn removing_claude_hooks(json: &[u8]) -> Result<Vec<u8>, InstallError> {
    let mut settings = settings_object(Some(json))
        .ok_or_else(|| InstallError::Unreadable("settings.json isn't a JSON object".into()))?;
    let Some(Value::Object(hooks)) = settings.get("hooks").cloned() else {
        return Ok(json.to_vec());
    };
    let mut kept_hooks = Map::new();
    for (event, value) in hooks {
        let Some(entries) = objects(Some(&value)) else {
            kept_hooks.insert(event, value);
            continue;
        };
        let kept: Vec<Value> = entries
            .into_iter()
            .filter_map(|entry| {
                let Some(inner) = objects(entry.get("hooks")) else {
                    return Some(Value::Object(entry.clone()));
                };
                let inner: Vec<Value> = inner
                    .into_iter()
                    .filter(|hook| {
                        hook.get("command").and_then(Value::as_str) != Some(CLAUDE_COMMAND)
                    })
                    .map(|hook| Value::Object(hook.clone()))
                    .collect();
                if inner.is_empty() {
                    return None;
                }
                let mut copy = entry.clone();
                copy.insert("hooks".into(), Value::Array(inner));
                Some(Value::Object(copy))
            })
            .collect();
        if !kept.is_empty() {
            kept_hooks.insert(event, Value::Array(kept));
        }
    }
    if kept_hooks.is_empty() {
        settings.remove("hooks");
    } else {
        settings.insert("hooks".into(), Value::Object(kept_hooks));
    }
    Ok(serialize(Value::Object(settings)))
}

/// Pretty JSON with sorted keys and a final newline, byte for byte as the
/// Swift app writes it (`JSONSerialization`), so a file either app wrote
/// isn't rewritten by the other.
pub(crate) fn serialize(value: Value) -> Vec<u8> {
    let mut data = super::foundation_json::pretty(&value).into_bytes();
    data.push(b'\n');
    data
}

// Codex (config.toml)

pub fn codex_notify_installed(toml: Option<&str>) -> bool {
    toml.is_some_and(|text| text.contains("hook codex"))
}

const CODEX_COMMENT: &str = "# Tells Impulse when Codex finishes a turn (no-op outside Impulse).";

/// config.toml with Impulse as the `notify` program. Top-level keys must
/// come before any table, so it's added at the top. Refuses when another
/// notify program is configured.
pub fn installing_codex_notify(toml: Option<&str>) -> Result<String, InstallError> {
    let text = toml.unwrap_or("");
    if codex_notify_installed(Some(text)) {
        return Ok(text.to_string());
    }
    let has_notify = text
        .split(['\n', '\r'])
        .any(|line| crate::git::trim_blanks(line).starts_with("notify") && line.contains('='));
    if has_notify {
        return Err(InstallError::InUse(
            "config.toml already sets notify; add Impulse's command by hand".into(),
        ));
    }
    let array = CODEX_NOTIFY
        .iter()
        .map(|part| format!("\"{}\"", part.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(", ");
    let line = format!("{CODEX_COMMENT}\nnotify = [{array}]\n");
    Ok(if text.is_empty() {
        line
    } else {
        format!("{line}\n{text}")
    })
}

/// config.toml without Impulse's `notify` line, the comment above it and
/// the blank line installing put after it. Another program's `notify` is
/// left alone.
pub fn removing_codex_notify(toml: &str) -> String {
    let mut lines: Vec<&str> = toml.split('\n').collect();
    let Some(mut index) = lines
        .iter()
        .position(|l| crate::git::trim_blanks(l).starts_with("notify") && l.contains("hook codex"))
    else {
        return toml.to_string();
    };
    lines.remove(index);
    if index > 0 && lines[index - 1] == CODEX_COMMENT {
        index -= 1;
        lines.remove(index);
    }
    if index == 0 && lines.len() > 1 && lines[0].is_empty() {
        lines.remove(0);
    }
    lines.join("\n")
}

// opencode (a plugin folder)

/// Impulse's plugin: a folder in opencode's plugins folder. opencode loads
/// its server.js in its background service and its tui.js in each terminal
/// running opencode (rpc.js is how the two talk).
pub const OPENCODE_PLUGIN_FOLDER: &str = "impulse";

/// The single-file plugin Impulse installed for opencode 1, which opencode
/// 2 can't load: installing the folder removes it.
pub const OPENCODE_LEGACY_PLUGIN_NAME: &str = "impulse.js";

/// Each file's first line, which tells an older version of it from a file
/// that isn't Impulse's.
pub const OPENCODE_PLUGIN_MARKER: &str = "// Impulse's opencode plugin";

/// The plugin's files by name, from the Swift app's resources (each
/// replaced or removed as a whole).
pub fn opencode_plugin_files() -> BTreeMap<String, String> {
    [
        ("rpc.js", include_str!("../../../impulse-macos/Sources/ImpulseKit/Resources/Agents/opencode-plugin/rpc.js")),
        (
            "server.js",
            include_str!("../../../impulse-macos/Sources/ImpulseKit/Resources/Agents/opencode-plugin/server.js"),
        ),
        ("tui.js", include_str!("../../../impulse-macos/Sources/ImpulseKit/Resources/Agents/opencode-plugin/tui.js")),
    ]
    .into_iter()
    .map(|(name, text)| (name.to_string(), text.to_string()))
    .collect()
}

/// What's in opencode's plugins folder: the files in Impulse's folder, by
/// name, and the older single-file plugin (`None` when there's none).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpencodePlugin {
    pub files: BTreeMap<String, String>,
    pub legacy: Option<String>,
}

impl OpencodePlugin {
    /// Whether the single-file plugin is Impulse's.
    pub fn legacy_is_impulses(&self) -> bool {
        self.legacy.as_deref().is_some_and(is_opencode_plugin_file)
    }
}

/// Whether a file in opencode's plugins folder is one of Impulse's (this
/// version's or an older one's).
pub fn is_opencode_plugin_file(text: &str) -> bool {
    text.starts_with(OPENCODE_PLUGIN_MARKER)
}

/// Whether `found` is this version of the plugin, an older one (an older
/// folder, or the single file opencode 1 loaded), or not there (or not
/// Impulse's).
pub fn opencode_plugin_status(found: &OpencodePlugin) -> Status {
    if found.files == opencode_plugin_files() && !found.legacy_is_impulses() {
        return Status::Installed;
    }
    let older = found.legacy_is_impulses()
        || found
            .files
            .values()
            .any(|text| is_opencode_plugin_file(text));
    if older {
        Status::Outdated
    } else {
        Status::NotInstalled
    }
}

/// The files to write into Impulse's folder; refuses a folder holding a
/// file that isn't Impulse's.
pub fn installing_opencode_plugin(
    found: &OpencodePlugin,
) -> Result<BTreeMap<String, String>, InstallError> {
    if let Some((theirs, _)) = found
        .files
        .iter()
        .find(|(_, text)| !is_opencode_plugin_file(text))
    {
        return Err(InstallError::InUse(format!(
            "{OPENCODE_PLUGIN_FOLDER}/{theirs} in opencode's plugins folder isn't Impulse's; rename or remove the \
             {OPENCODE_PLUGIN_FOLDER} folder first"
        )));
    }
    Ok(opencode_plugin_files())
}

/// The plugin's files as one text, each under its path, for the preview.
pub fn opencode_plugin_text(found: &OpencodePlugin) -> String {
    let mut parts: Vec<String> = found
        .files
        .iter()
        .map(|(name, text)| format!("// {OPENCODE_PLUGIN_FOLDER}/{name}\n{text}"))
        .collect();
    if let Some(legacy) = &found.legacy {
        parts.push(format!("// {OPENCODE_LEGACY_PLUGIN_NAME}\n{legacy}"));
    }
    parts.join("\n")
}

// Preview

/// A unified-style line diff of a config change, for confirmation: kept
/// lines indented by two spaces, removed ones after `- `, added ones after
/// `+ `.
pub fn diff_lines(before: &str, after: &str) -> Vec<String> {
    let old: Vec<&str> = before.split('\n').collect();
    let new: Vec<&str> = after.split('\n').collect();
    // The longest common subsequence, from the ends: common[i][j] is its
    // length for old[i..] and new[j..].
    let mut common = vec![vec![0usize; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            common[i][j] = if old[i] == new[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let mut removed = BTreeSet::new();
    let mut inserted = BTreeSet::new();
    let (mut i, mut j) = (0, 0);
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            i += 1;
            j += 1;
        } else if j == new.len() || (i < old.len() && common[i + 1][j] >= common[i][j + 1]) {
            removed.insert(i);
            i += 1;
        } else {
            inserted.insert(j);
            j += 1;
        }
    }
    let mut lines = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < old.len() || j < new.len() {
        if i < old.len() && removed.contains(&i) {
            lines.push(format!("- {}", old[i]));
            i += 1;
        } else if j < new.len() && inserted.contains(&j) {
            lines.push(format!("+ {}", new[j]));
            j += 1;
        } else {
            if i < old.len() {
                lines.push(format!("  {}", old[i]));
            }
            i += 1;
            j += 1;
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::hook_events;

    fn object(data: &[u8]) -> Map<String, Value> {
        settings_object(Some(data)).unwrap_or_default()
    }

    #[test]
    fn installs_claude_hooks_into_empty_or_missing_settings() {
        let fresh = installing_claude_hooks(None).unwrap();
        assert!(claude_hooks_installed(Some(&fresh)));
        let settings = object(&fresh);
        let keys: BTreeSet<&str> = settings["hooks"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, claude_events().into_iter().collect());
        assert!(!claude_hooks_installed(None));
    }

    #[test]
    fn keeps_other_settings_and_hooks_and_does_not_duplicate() {
        let existing = br#"{"model": "opus", "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}],
 "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "audit"}]}]}}"#;
        let once = installing_claude_hooks(Some(existing)).unwrap();
        let twice = installing_claude_hooks(Some(&once)).unwrap();
        assert_eq!(once, twice);
        let settings = object(&twice);
        assert_eq!(settings["model"], "opus");
        let hooks = settings["hooks"].as_object().unwrap();
        assert_eq!(hooks["Stop"].as_array().unwrap().len(), 2);
        // Impulse's own PreToolUse hook (for shell commands) goes beside the
        // user's.
        let pre_tool_use = hooks["PreToolUse"].as_array().unwrap();
        assert_eq!(pre_tool_use.len(), 2);
        assert_eq!(pre_tool_use[0]["hooks"][0]["command"], "audit");
        assert_eq!(pre_tool_use[1]["matcher"], "Bash");
        assert_eq!(
            hooks["PostToolUse"][0]["matcher"],
            "Edit|Write|MultiEdit|NotebookEdit|Bash"
        );
    }

    #[test]
    fn hooks_from_an_older_impulse_are_out_of_date() {
        let command = CLAUDE_COMMAND.replace('"', "\\\"");
        let older = format!(
            r#"{{"hooks": {{"SessionStart": [{{"hooks": [{{"type": "command", "command": "{command}"}}]}}],
 "Stop": [{{"hooks": [{{"type": "command", "command": "{command}"}}]}}]}}}}"#
        );
        assert_eq!(claude_hook_status(Some(older.as_bytes())), Status::Outdated);
        assert_eq!(claude_hook_status(None), Status::NotInstalled);
        let updated = installing_claude_hooks(Some(older.as_bytes())).unwrap();
        assert_eq!(claude_hook_status(Some(&updated)), Status::Installed);
        // Everything but StopFailure (Impulse 0.35 and before).
        let mut settings = object(&updated);
        settings["hooks"]
            .as_object_mut()
            .unwrap()
            .remove("StopFailure");
        let before_stop_failure = serde_json::to_vec(&settings).unwrap();
        assert_eq!(
            claude_hook_status(Some(&before_stop_failure)),
            Status::Outdated
        );
    }

    #[test]
    fn uninstall_restores_the_rest() {
        let existing = br#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}, "x": 1}"#;
        let installed = installing_claude_hooks(Some(existing)).unwrap();
        let removed = removing_claude_hooks(&installed).unwrap();
        let settings = object(&removed);
        assert_eq!(settings["x"], 1);
        let keys: Vec<&str> = settings["hooks"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["Stop"]);
        assert!(!claude_hooks_installed(Some(&removed)));
        // Removing from a file that only had ours drops "hooks" entirely.
        let only_ours = installing_claude_hooks(None).unwrap();
        assert!(!object(&removing_claude_hooks(&only_ours).unwrap()).contains_key("hooks"));
    }

    /// The testdata files are what the Swift app's `AgentHookInstaller`
    /// wrote for the same input: Impulse writes settings.json the same way
    /// whichever app does it.
    #[test]
    fn writes_settings_as_the_swift_app_does() {
        let fresh = include_str!("testdata/claude-settings-fresh.json");
        let installed = include_str!("testdata/claude-settings-installed.json");
        let removed = include_str!("testdata/claude-settings-removed.json");
        let existing = r#"{"$schema":"https://json.schemastore.org/claude-code-settings.json","env":{"CLAUDEX":"1","CLAUDE_CODE_X":"2","A_b":"3","AB":"4"},"model":"opus","alwaysThinkingEnabled":true,"cleanupPeriodDays":30,"ratio":0.1,"permissions":{"allow":["Bash(git log:*)","Read(//tmp/**)"],"deny":[]},"hooks":{"Stop":[{"hooks":[{"type":"command","command":"say \"done\" \/ ok","timeout":5}]}]},"statusLine":{"type":"command","command":"~/bin/status"},"enabledPlugins":{"a@b":true,"A@c":false,"plugin10@x":true,"plugin9@x":true}}"#;
        let text = |bytes: Vec<u8>| String::from_utf8(bytes).unwrap();
        assert_eq!(text(installing_claude_hooks(None).unwrap()), fresh);
        let ours = text(installing_claude_hooks(Some(existing.as_bytes())).unwrap());
        assert_eq!(ours, installed);
        assert_eq!(
            text(removing_claude_hooks(ours.as_bytes()).unwrap()),
            removed
        );
        // Installing over what the Swift app installed changes nothing.
        assert_eq!(
            text(installing_claude_hooks(Some(installed.as_bytes())).unwrap()),
            installed
        );
    }

    /// Detection reads the JSON, not its layout: hooks the Swift app wrote
    /// and hooks an earlier Rust build wrote (serde's `"key": value`) are
    /// both found, and installing again only reformats the latter.
    #[test]
    fn finds_hooks_whichever_app_wrote_them() {
        let swift = include_bytes!("testdata/claude-settings-installed.json");
        assert_eq!(claude_hook_status(Some(swift)), Status::Installed);
        let value: Value = serde_json::from_slice(swift).unwrap();
        let serde_pretty = serde_json::to_vec_pretty(&value).unwrap();
        assert_ne!(serde_pretty.as_slice(), swift.as_slice());
        assert_eq!(claude_hook_status(Some(&serde_pretty)), Status::Installed);
        assert_eq!(
            installing_claude_hooks(Some(&serde_pretty)).unwrap(),
            swift.to_vec()
        );
        let removed = removing_claude_hooks(&serde_pretty).unwrap();
        assert_eq!(
            removed,
            include_bytes!("testdata/claude-settings-removed.json").to_vec()
        );
        assert_eq!(claude_hook_status(Some(&removed)), Status::NotInstalled);
    }

    #[test]
    fn rejects_unexpected_shapes() {
        assert_eq!(
            installing_claude_hooks(Some(b"[1,2]")),
            Err(InstallError::Unreadable(
                "settings.json isn't a JSON object".into()
            ))
        );
        assert_eq!(
            installing_claude_hooks(Some(br#"{"hooks": 3}"#)),
            Err(InstallError::Unreadable("\"hooks\" isn't an object".into()))
        );
    }

    #[test]
    fn codex_notify_goes_at_the_top_and_respects_existing_programs() {
        let config = "model = \"o3\"\n\n[profiles.fast]\nmodel = \"mini\"\n";
        let installed = installing_codex_notify(Some(config)).unwrap();
        assert!(installed.starts_with("# Tells Impulse"));
        assert!(installed.contains(r#"notify = ["sh", "-c", "[ -n \"$IMPULSE_CLI\" ]"#));
        assert!(installed.ends_with(config));
        assert!(codex_notify_installed(Some(&installed)));
        assert_eq!(
            installing_codex_notify(Some(&installed)).unwrap(),
            installed
        );
        assert_eq!(
            installing_codex_notify(Some("notify = [\"growl\"]\n")),
            Err(InstallError::InUse(
                "config.toml already sets notify; add Impulse's command by hand".into()
            ))
        );
    }

    #[test]
    fn codex_notify_removal_restores_the_file() {
        let config = "model = \"o3\"\n\n[profiles.fast]\nmodel = \"mini\"\n";
        let installed = installing_codex_notify(Some(config)).unwrap();
        let removed = removing_codex_notify(&installed);
        assert_eq!(removed, config);
        assert!(!codex_notify_installed(Some(&removed)));
        // A file that only had Impulse's notify ends up empty.
        let only_ours = installing_codex_notify(None).unwrap();
        assert_eq!(removing_codex_notify(&only_ours), "");
        // Someone else's notify program stays.
        let other = "notify = [\"growl\"]\nmodel = \"o3\"\n";
        assert_eq!(removing_codex_notify(other), other);
    }

    #[test]
    fn opencode_plugin_is_a_folder_replaced_whole_and_only_if_its_impulses() {
        let files = opencode_plugin_files();
        assert_eq!(
            files.keys().map(String::as_str).collect::<Vec<_>>(),
            ["rpc.js", "server.js", "tui.js"]
        );
        for (name, text) in &files {
            assert!(is_opencode_plugin_file(text), "{name}");
        }
        let found = |files: BTreeMap<String, String>, legacy: Option<String>| OpencodePlugin {
            files,
            legacy,
        };
        assert_eq!(
            opencode_plugin_status(&OpencodePlugin::default()),
            Status::NotInstalled
        );
        assert_eq!(
            opencode_plugin_status(&found(files.clone(), None)),
            Status::Installed
        );
        // opencode 1's single file, from an older Impulse: replaced by the
        // folder.
        let single =
            format!("{OPENCODE_PLUGIN_MARKER}, from an older Impulse\nexport const X = 1\n");
        assert_eq!(
            opencode_plugin_status(&found(BTreeMap::new(), Some(single.clone()))),
            Status::Outdated
        );
        assert_eq!(
            opencode_plugin_status(&found(files.clone(), Some(single.clone()))),
            Status::Outdated
        );
        let theirs = "export const Mine = async () => ({})\n".to_string();
        assert_eq!(
            opencode_plugin_status(&found(files.clone(), Some(theirs.clone()))),
            Status::Installed
        );
        // An older folder: updated whole.
        let mut older = files.clone();
        older.insert(
            "server.js".into(),
            format!("{OPENCODE_PLUGIN_MARKER}, older\n"),
        );
        older.insert(
            "gone.js".into(),
            format!("{OPENCODE_PLUGIN_MARKER}, a file this version doesn't have\n"),
        );
        assert_eq!(
            opencode_plugin_status(&found(older.clone(), None)),
            Status::Outdated
        );
        assert_eq!(
            installing_opencode_plugin(&found(older, None)).unwrap(),
            files
        );
        assert_eq!(
            installing_opencode_plugin(&OpencodePlugin::default()).unwrap(),
            files
        );
        // A folder holding someone else's file is left alone.
        let someone_elses: BTreeMap<String, String> =
            [("index.js".to_string(), theirs)].into_iter().collect();
        assert_eq!(
            opencode_plugin_status(&found(someone_elses.clone(), None)),
            Status::NotInstalled
        );
        assert!(installing_opencode_plugin(&found(someone_elses, None)).is_err());
        // The preview names each file.
        let text = opencode_plugin_text(&found(files, Some(single)));
        for name in [
            "impulse/rpc.js",
            "impulse/server.js",
            "impulse/tui.js",
            "impulse.js",
        ] {
            assert!(text.contains(&format!("// {name}\n")), "{name}");
        }
    }

    #[test]
    fn opencode_plugin_sends_the_events_impulse_knows() {
        let files = opencode_plugin_files();
        let tui = &files["tui.js"];
        let server = &files["server.js"];
        // The terminal's half is a no-op outside Impulse and calls the CLI
        // by its path (no shell); the service's half never trusts its own
        // environment, which is whichever terminal started the service.
        assert!(tui.contains("process.env.IMPULSE_CLI"));
        assert!(tui.contains("if (!cli) return;"));
        assert!(tui.contains(r#"spawn(cli, ["hook", "opencode", event]"#));
        assert!(!server.contains("process.env.IMPULSE"));
        assert!(
            !server.contains("spawn("),
            "only the terminal's half runs impulse"
        );
        for event in [
            "start",
            "session.start",
            "session.execution.started",
            "session.execution.succeeded",
            "session.execution.failed",
            "session.execution.interrupted",
            "permission.asked",
            "permission.replied",
            "form.created",
            "form.replied",
            "form.cancelled",
            "tool.execute.before",
            "tool.execute.after",
        ] {
            assert!(tui.contains(&format!("\"{event}\"")), "{event}");
            let known = hook_events::hook(Some("opencode"), event, Some("shell"), None).is_some()
                || hook_events::reply(Some("opencode"), event, Some("shell")).is_some();
            assert!(known, "{event}");
        }
        for tool in hook_events::OPENCODE_EDIT_TOOLS {
            assert!(server.contains(&format!("\"{tool}\"")), "{tool}");
        }
        // The variables a shell command gets from its terminal are the ones
        // the terminal sends.
        fn variables(text: &str) -> Vec<String> {
            let Some(start) = text.find("const paneVariables = [") else {
                return Vec::new();
            };
            let rest = &text[start + "const paneVariables = [".len()..];
            let Some(end) = rest.find("];") else {
                return Vec::new();
            };
            rest[..end]
                .split('"')
                .enumerate()
                .filter(|(i, _)| i % 2 == 1)
                .map(|(_, v)| v.to_string())
                .collect()
        }
        assert_eq!(variables(server), variables(tui));
        assert!(variables(server).iter().any(|v| v == "IMPULSE_PANE_TOKEN"));
        // opencode loads a plugin module's default export.
        for text in [server, tui] {
            assert_eq!(text.matches("export default").count(), 1);
        }
    }

    #[test]
    fn diff_shows_added_and_removed_lines() {
        assert_eq!(
            diff_lines("a\nb\nc", "a\nB\nc\nd"),
            ["  a", "- b", "+ B", "  c", "+ d"]
        );
        assert_eq!(diff_lines("", "x"), ["- ", "+ x"]);
        assert_eq!(diff_lines("same", "same"), ["  same"]);
    }
}
