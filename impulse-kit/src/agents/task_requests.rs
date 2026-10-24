//! Tasks an agent asks for (`impulse tasks new`, the MCP server's
//! create_task): where their brief is kept, and whether Impulse creates one
//! straight away, asks you first, or refuses. Creating a task starts
//! processes (a setup script, an agent that spends money and edits files),
//! so the answer is "ask" unless the user chose otherwise, and some cases
//! always ask whatever they chose.

use std::fs;
use std::io;

use super::known_agents::AgentKind;
use crate::git::task_registry::{join, parent, TaskRegistry};
use crate::git::trim_blanks;

/// `.git/impulse/tasks/<task>/brief.md`: beside the task's Compose
/// override, outside its folder, so it's never committed by accident.
pub fn brief_path(task: &str, common_git_directory: &str) -> String {
    let folder = join(
        &TaskRegistry::directory(common_git_directory),
        &format!("tasks/{task}"),
    );
    join(&folder, "brief.md")
}

pub fn read_brief(task: &str, common_git_directory: &str) -> Option<String> {
    let data = fs::read(brief_path(task, common_git_directory)).ok()?;
    Some(String::from_utf8_lossy(&data).into_owned())
}

pub fn write_brief(brief: &str, task: &str, common_git_directory: &str) -> io::Result<()> {
    let file = brief_path(task, common_git_directory);
    fs::create_dir_all(parent(&file))?;
    let temp = format!("{file}.tmp");
    fs::write(&temp, brief)?;
    fs::rename(&temp, &file)
}

/// The brief's first non-empty line, shortened to `limit` characters, for
/// listings.
pub fn brief_summary(brief: &str, limit: usize) -> Option<String> {
    let line = brief
        .split([
            '\n', '\r', '\u{0B}', '\u{0C}', '\u{85}', '\u{2028}', '\u{2029}',
        ])
        .map(trim_blanks)
        .find(|line| !line.is_empty())?;
    if line.chars().count() > limit {
        let kept: String = line.chars().take(limit.saturating_sub(1)).collect();
        Some(format!("{kept}…"))
    } else {
        Some(line.to_string())
    }
}

/// How long a brief's summary is by default.
pub const BRIEF_SUMMARY_LIMIT: usize = 120;

/// `agent_task_creation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Ask,
    /// Create without asking in a trusted repository whose project settings
    /// are trusted; otherwise ask.
    TrustedProjects,
    Off,
}

impl Setting {
    pub const ALL: [Setting; 3] = [Setting::Ask, Setting::TrustedProjects, Setting::Off];

    pub fn as_str(self) -> &'static str {
        match self {
            Setting::Ask => "ask",
            Setting::TrustedProjects => "trusted_projects",
            Setting::Off => "off",
        }
    }

    /// The setting's value as written; anything unknown is `Ask`.
    pub fn from_setting(value: &str) -> Setting {
        Setting::ALL
            .into_iter()
            .find(|s| s.as_str() == value)
            .unwrap_or(Setting::Ask)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Show the New Task sheet, prefilled, and wait for the user.
    Ask,
    /// Create it now, with a toast and Undo.
    Create,
    Refuse(String),
}

/// The default for `agent_task_limit`: open tasks agents asked for, per
/// repository.
pub const DEFAULT_LIMIT: usize = 4;

/// What Impulse knows about an agent's request when deciding on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    /// The repository is a trusted folder.
    pub repository_trusted: bool,
    /// Every project settings file with commands is trusted (or there are
    /// none).
    pub project_trusted: bool,
    /// Open tasks agents asked for in the repository.
    pub agent_tasks: usize,
    /// The request comes from inside a task an agent asked for: a chain,
    /// which always asks.
    pub from_agent_task: bool,
    /// It takes the main checkout's uncommitted files.
    pub moves_changes: bool,
}

/// What to do with an agent's request, given `agent_task_creation` and
/// `agent_task_limit` (0: no limit).
pub fn decide(setting: Setting, limit: usize, request: Request) -> Decision {
    if setting == Setting::Off {
        return Decision::Refuse(
            "Starting tasks from agents is turned off in Impulse's settings (agent_task_creation)."
                .into(),
        );
    }
    let tasks = request.agent_tasks;
    if limit > 0 && tasks >= limit {
        let plural = if tasks == 1 { "" } else { "s" };
        return Decision::Refuse(format!(
            "This repository already has {tasks} open task{plural} agents asked for (the limit is {limit}). \
             Finish or archive one first, or ask the user to raise agent_task_limit."
        ));
    }
    let create = setting == Setting::TrustedProjects
        && request.repository_trusted
        && request.project_trusted
        && !request.from_agent_task
        && !request.moves_changes;
    if create {
        Decision::Create
    } else {
        Decision::Ask
    }
}

/// The shells Impulse types commands into.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShellType {
    Bash,
    Zsh,
    Fish,
}

/// How the command lines that take a first prompt and stay interactive
/// take it: after `--` (Claude Code, Codex), or as the value of opencode's
/// `--prompt` (its bare argument is the folder to open).
pub fn prompt_argument(agent_id: &str) -> Option<&'static str> {
    match agent_id {
        "claude" | "codex" => Some("-- "),
        "opencode" => Some("--prompt="),
        _ => None,
    }
}

/// Agents whose command line takes a first prompt and stays interactive.
pub const TAKES_PROMPT: [&str; 3] = ["claude", "codex", "opencode"];

/// The first terminal's command for a task started with `agent`, with the
/// brief saved at `brief_path` as its first prompt for the agents whose
/// command takes one. The brief is read from its file when the command
/// runs, so a long or many-line brief is never typed into the shell (or
/// kept in its history), after `--` or as an option's `=` value so a brief
/// that starts with a dash (a Markdown list) isn't taken for an option,
/// with `command cat` so a function or alias called cat (`cat=bat`) can't
/// change or lose it. `None` when the agent's command takes no prompt: the
/// brief is typed into the agent once it's up.
pub fn command(agent: &AgentKind, brief_path: Option<&str>, shell: ShellType) -> Option<String> {
    let name = agent.names.first()?;
    let Some(brief_path) = brief_path else {
        return Some(name.clone());
    };
    let prompt = prompt_argument(&agent.id)?;
    let file = quoted(brief_path, shell);
    Some(match shell {
        ShellType::Fish => format!("{name} {prompt}(command cat {file} | string collect)"),
        ShellType::Bash | ShellType::Zsh => format!("{name} {prompt}\"$(command cat {file})\""),
    })
}

/// `text` as one word for `shell`: as is when every character is safe
/// bare, otherwise in single quotes. sh, bash and zsh close the quotes
/// around a `'` (`'\''`); fish reads `\\` and `\'` as escapes inside single
/// quotes, so those two get a backslash. Control characters other than tab
/// and newline go outside the quotes as escapes (`$'\x0d'`, fish
/// `\u000d`), so they never reach a line editor as keys.
fn quoted(text: &str, shell: ShellType) -> String {
    if text.is_empty() {
        return "''".into();
    }
    if text.chars().all(is_safe) {
        return text.into();
    }
    let mut out = String::new();
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.is_empty() {
            return;
        }
        let literal = std::mem::take(run);
        match shell {
            ShellType::Fish => out.push_str(&format!(
                "'{}'",
                literal.replace('\\', "\\\\").replace('\'', "\\'")
            )),
            ShellType::Bash | ShellType::Zsh => {
                out.push_str(&format!("'{}'", literal.replace('\'', "'\\''")))
            }
        }
    };
    for c in text.chars() {
        if is_encoded(c) {
            flush(&mut run, &mut out);
            match shell {
                ShellType::Fish => out.push_str(&format!("\\u{:04x}", c as u32)),
                ShellType::Bash | ShellType::Zsh => {
                    let mut bytes = [0; 4];
                    let escaped: String = c
                        .encode_utf8(&mut bytes)
                        .bytes()
                        .map(|b| format!("\\x{b:02x}"))
                        .collect();
                    out.push_str(&format!("$'{escaped}'"));
                }
            }
        } else {
            run.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// Control characters that can't go in quotes as they are: C0, DEL and C1,
/// but tab and newline.
fn is_encoded(c: char) -> bool {
    let value = c as u32;
    (value < 0x20 || (0x7F..=0x9F).contains(&value)) && c != '\n' && c != '\t'
}

/// Read literally, bare, by every supported shell anywhere in a word: ASCII
/// letters and digits, `/ _ . - + , : @`, and anything beyond ASCII (none of
/// the shells gives those a meaning).
fn is_safe(c: char) -> bool {
    c.is_ascii_alphanumeric() || "+,-./:@_".contains(c) || (c as u32) > 0x9F
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::known_agents::built_in_kind;

    fn decide_with(
        setting: Setting,
        trusted: bool,
        project: bool,
        tasks: usize,
        limit: usize,
        chain: bool,
        moves: bool,
    ) -> Decision {
        decide(
            setting,
            limit,
            Request {
                repository_trusted: trusted,
                project_trusted: project,
                agent_tasks: tasks,
                from_agent_task: chain,
                moves_changes: moves,
            },
        )
    }

    #[test]
    fn when_an_agents_task_asks_first() {
        let simple = |setting| decide_with(setting, true, true, 0, 4, false, false);
        assert_eq!(simple(Setting::Ask), Decision::Ask);
        assert_eq!(simple(Setting::TrustedProjects), Decision::Create);
        assert_eq!(
            decide_with(Setting::TrustedProjects, false, true, 0, 4, false, false),
            Decision::Ask
        );
        assert_eq!(
            decide_with(Setting::TrustedProjects, true, false, 0, 4, false, false),
            Decision::Ask
        );
        assert_eq!(
            decide_with(Setting::TrustedProjects, true, true, 0, 4, true, false),
            Decision::Ask
        );
        assert_eq!(
            decide_with(Setting::TrustedProjects, true, true, 0, 4, false, true),
            Decision::Ask
        );
        assert!(
            matches!(simple(Setting::Off), Decision::Refuse(_)),
            "off refuses"
        );
        match decide_with(Setting::Ask, true, true, 4, 4, false, false) {
            Decision::Refuse(why) => assert!(why.contains("the limit is 4"), "{why}"),
            other => panic!("the limit refuses: {other:?}"),
        }
        assert_eq!(
            decide_with(Setting::Ask, true, true, 40, 0, false, false),
            Decision::Ask,
            "0: no limit"
        );
        assert_eq!(Setting::from_setting("nonsense"), Setting::Ask);
        assert_eq!(
            Setting::from_setting("trusted_projects"),
            Setting::TrustedProjects
        );
    }

    #[test]
    fn the_brief_starts_the_agent_from_its_file() {
        let claude = built_in_kind("claude").unwrap();
        let aider = built_in_kind("aider").unwrap();
        let path = "/repo/.git/impulse/tasks/fix it/brief.md";
        assert_eq!(
            command(claude, Some(path), ShellType::Zsh).as_deref(),
            Some(r#"claude -- "$(command cat '/repo/.git/impulse/tasks/fix it/brief.md')""#)
        );
        assert_eq!(
            command(claude, Some(path), ShellType::Fish).as_deref(),
            Some("claude -- (command cat '/repo/.git/impulse/tasks/fix it/brief.md' | string collect)")
        );
        assert_eq!(
            command(claude, None, ShellType::Zsh).as_deref(),
            Some("claude")
        );
        assert_eq!(
            command(aider, Some(path), ShellType::Zsh),
            None,
            "typed in once it's up"
        );
        // opencode's bare argument is the folder to open: the brief is
        // --prompt's value.
        let opencode = built_in_kind("opencode").unwrap();
        assert_eq!(
            command(opencode, Some(path), ShellType::Bash).as_deref(),
            Some(
                r#"opencode --prompt="$(command cat '/repo/.git/impulse/tasks/fix it/brief.md')""#
            )
        );
        assert_eq!(
            command(opencode, Some(path), ShellType::Fish).as_deref(),
            Some("opencode --prompt=(command cat '/repo/.git/impulse/tasks/fix it/brief.md' | string collect)")
        );
        assert!(TAKES_PROMPT.iter().all(|id| prompt_argument(id).is_some()));
    }

    #[test]
    fn odd_paths_are_quoted_for_each_shell() {
        assert_eq!(quoted("/plain/path.md", ShellType::Zsh), "/plain/path.md");
        assert_eq!(quoted("it's", ShellType::Bash), r"'it'\''s'");
        assert_eq!(quoted(r"it's\x", ShellType::Fish), r"'it\'s\\x'");
        assert_eq!(quoted("a\rb", ShellType::Zsh), r"'a'$'\x0d''b'");
        assert_eq!(quoted("a\rb", ShellType::Fish), r"'a'\u000d'b'");
        assert_eq!(quoted("", ShellType::Fish), "''");
    }

    #[test]
    fn briefs_are_kept_beside_the_registry() {
        let dir = tempfile::tempdir().unwrap();
        let common = format!("{}/.git", dir.path().display());
        write_brief("Do the thing.\nCarefully.", "t", &common).unwrap();
        assert_eq!(
            brief_path("t", &common),
            format!("{common}/impulse/tasks/t/brief.md")
        );
        assert_eq!(
            read_brief("t", &common).as_deref(),
            Some("Do the thing.\nCarefully.")
        );
        assert_eq!(
            brief_summary("\n\n  First line  \nsecond", BRIEF_SUMMARY_LIMIT).as_deref(),
            Some("First line")
        );
        assert_eq!(
            brief_summary(&"x".repeat(200), 10).as_deref(),
            Some("xxxxxxxxx…")
        );
        assert_eq!(brief_summary(" \n", 10), None);
    }
}
