//! Recognizing coding agents (Claude Code, Codex, ...) from the process a
//! terminal is running. Many are scripts, so the interpreter's arguments are
//! checked too.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::git::last_path_component;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentKind {
    /// Stable id ("claude", "codex", ...).
    pub id: String,
    pub display_name: String,
    /// Executable or script basenames that identify it.
    pub names: Vec<String>,
    /// Path fragments of its installed package (for `node .../cli.js` and
    /// the like).
    #[serde(default)]
    pub packages: Vec<String>,
}

impl AgentKind {
    pub fn new(id: &str, display_name: &str, names: &[&str], packages: &[&str]) -> Self {
        Self {
            id: id.into(),
            display_name: display_name.into(),
            names: names.iter().map(|n| n.to_string()).collect(),
            packages: packages.iter().map(|p| p.to_string()).collect(),
        }
    }
}

/// The command that resumes an agent's session, when the agent supports it.
pub fn resume_command(agent_id: &str, session: &str) -> Option<String> {
    let safe = session
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    if !safe || session.is_empty() {
        return None;
    }
    match agent_id {
        "claude" => Some(format!("claude --resume {session}")),
        "codex" => Some(format!("codex resume {session}")),
        "opencode" => Some(format!("opencode --session {session}")),
        _ => None,
    }
}

static BUILT_IN: LazyLock<Vec<AgentKind>> = LazyLock::new(|| {
    vec![
        AgentKind::new(
            "claude",
            "Claude Code",
            &["claude"],
            &["@anthropic-ai/claude-code"],
        ),
        AgentKind::new("codex", "Codex", &["codex"], &["@openai/codex"]),
        AgentKind::new("gemini", "Gemini CLI", &["gemini"], &["@google/gemini-cli"]),
        AgentKind::new("aider", "Aider", &["aider"], &["aider-chat", "/aider/"]),
        AgentKind::new("opencode", "opencode", &["opencode"], &["opencode-ai"]),
        AgentKind::new("amp", "Amp", &["amp"], &["@sourcegraph/amp"]),
        AgentKind::new("copilot", "Copilot CLI", &["copilot"], &["@github/copilot"]),
        AgentKind::new("cursor", "Cursor Agent", &["cursor-agent"], &[]),
        AgentKind::new("goose", "Goose", &["goose"], &[]),
        AgentKind::new("qwen", "Qwen Code", &["qwen"], &["@qwen-code/qwen-code"]),
        AgentKind::new("crush", "Crush", &["crush"], &[]),
    ]
});

/// The agents Impulse knows.
pub fn built_in() -> &'static [AgentKind] {
    &BUILT_IN
}

/// The built-in agent with `id`.
pub fn built_in_kind(id: &str) -> Option<&'static AgentKind> {
    BUILT_IN.iter().find(|kind| kind.id == id)
}

/// Interpreters whose script argument names the real program.
const INTERPRETERS: [&str; 15] = [
    "node", "bun", "deno", "python", "python3", "ruby", "uv", "uvx", "npx", "pnpm", "bunx",
    // Wrapper scripts (e.g. ~/.claude/local/claude) run under a shell.
    "sh", "bash", "zsh", "dash",
];

/// The agent a process is, given its executable path and argv (argv[0]
/// included). `extra` extends the built-in table (user settings).
pub fn match_process(
    executable_path: &str,
    arguments: &[String],
    extra: &[AgentKind],
) -> Option<AgentKind> {
    let kinds: Vec<&AgentKind> = extra.iter().chain(BUILT_IN.iter()).collect();
    let executable = last_path_component(executable_path);
    let mut candidates = vec![executable];
    if let Some(first) = arguments.first() {
        candidates.push(last_path_component(first));
    }
    let is_interpreter = INTERPRETERS.contains(&executable)
        || executable.starts_with("python")
        || arguments
            .first()
            .is_some_and(|first| INTERPRETERS.contains(&last_path_component(first)));
    if is_interpreter {
        // The first argument that isn't an option is the script.
        if let Some(script) = arguments.iter().skip(1).find(|a| !a.starts_with('-')) {
            candidates.push(last_path_component(script));
            if let Some(kind) = kinds
                .iter()
                .find(|kind| kind.packages.iter().any(|p| script.contains(p.as_str())))
            {
                return Some((*kind).clone());
            }
        }
    }
    for candidate in candidates {
        let name = strip_extension(candidate);
        if let Some(kind) = kinds
            .iter()
            .find(|kind| kind.names.iter().any(|n| n == name))
        {
            return Some((*kind).clone());
        }
    }
    // A package path anywhere in the executable (e.g. a node_modules shim).
    kinds
        .iter()
        .find(|kind| {
            kind.packages
                .iter()
                .any(|p| executable_path.contains(p.as_str()))
        })
        .map(|kind| (*kind).clone())
}

fn strip_extension(name: &str) -> &str {
    [".js", ".mjs", ".cjs", ".py", ".exe"]
        .iter()
        .find_map(|ext| name.strip_suffix(ext))
        .unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(executable: &str, arguments: &[&str]) -> Option<String> {
        let arguments: Vec<String> = arguments.iter().map(|a| a.to_string()).collect();
        match_process(executable, &arguments, &[]).map(|kind| kind.id)
    }

    #[test]
    fn matches_native_binaries_and_scripts() {
        assert_eq!(
            id("/opt/homebrew/bin/claude", &["claude"]).as_deref(),
            Some("claude")
        );
        assert_eq!(
            id(
                "/usr/local/bin/node",
                &[
                    "node",
                    "/usr/local/lib/node_modules/@anthropic-ai/claude-code/cli.js",
                    "--resume"
                ]
            )
            .as_deref(),
            Some("claude")
        );
        assert_eq!(
            id(
                "/Users/me/.nvm/versions/node/v22/bin/node",
                &[
                    "node",
                    "--no-warnings",
                    "/Users/me/.nvm/versions/node/v22/bin/codex"
                ]
            )
            .as_deref(),
            Some("codex")
        );
        assert_eq!(
            id(
                "/opt/homebrew/Cellar/python@3.12/3.12.4/bin/python3.12",
                &["python3.12", "/Users/me/.local/bin/aider", "--model", "x"]
            )
            .as_deref(),
            Some("aider")
        );
        let gemini = match_process("/usr/local/bin/gemini", &["gemini".to_string()], &[]).unwrap();
        assert_eq!(gemini.display_name, "Gemini CLI");
        assert_eq!(
            id("/bin/sh", &["/bin/sh", "/Users/me/.claude/local/claude"]).as_deref(),
            Some("claude")
        );
        assert_eq!(id("/bin/bash", &["bash", "deploy.sh"]), None);
    }

    #[test]
    fn matches_opencode_however_its_installed() {
        // npm (opencode-ai): its postinstall links the native binary in as
        // bin/opencode.exe.
        assert_eq!(
            id(
                "/Users/me/.npm-global/lib/node_modules/opencode-ai/bin/opencode.exe",
                &["opencode"]
            )
            .as_deref(),
            Some("opencode")
        );
        // Older npm releases: a node script that starts the platform
        // package's binary.
        assert_eq!(
            id(
                "/usr/local/bin/node",
                &[
                    "node",
                    "/usr/local/lib/node_modules/opencode-ai/bin/opencode"
                ]
            )
            .as_deref(),
            Some("opencode")
        );
        assert_eq!(
            id(
                "/usr/local/lib/node_modules/opencode-ai/node_modules/opencode-darwin-arm64/bin/opencode",
                &["opencode"]
            )
            .as_deref(),
            Some("opencode")
        );
        // Homebrew, and the curl installer.
        assert_eq!(
            id(
                "/opt/homebrew/Cellar/opencode/1.18.34/bin/opencode",
                &["opencode"]
            )
            .as_deref(),
            Some("opencode")
        );
        assert_eq!(
            id(
                "/Users/me/.opencode/bin/opencode",
                &["/Users/me/.opencode/bin/opencode", "--continue"]
            )
            .as_deref(),
            Some("opencode")
        );
        // Crush (once the Go project called opencode) is its own agent.
        assert_eq!(
            id("/opt/homebrew/bin/crush", &["crush"]).as_deref(),
            Some("crush")
        );
    }

    #[test]
    fn ignores_ordinary_programs() {
        assert_eq!(id("/usr/bin/vim", &["vim", "claude.md"]), None);
        assert_eq!(id("/usr/local/bin/node", &["node", "server.js"]), None);
        assert_eq!(id("/bin/zsh", &["-zsh"]), None);
    }

    #[test]
    fn resume_commands() {
        assert_eq!(
            resume_command("claude", "3f2a-91bc").as_deref(),
            Some("claude --resume 3f2a-91bc")
        );
        assert_eq!(
            resume_command("codex", "abc_1").as_deref(),
            Some("codex resume abc_1")
        );
        assert_eq!(
            resume_command("opencode", "ses_3a1f9c2e7ffeQ2xVbN8kLm4pRt").as_deref(),
            Some("opencode --session ses_3a1f9c2e7ffeQ2xVbN8kLm4pRt")
        );
        assert_eq!(resume_command("claude", "x; rm -rf ~"), None);
        assert_eq!(resume_command("aider", "s"), None);
    }

    #[test]
    fn user_agents_extend_the_table() {
        let custom = AgentKind::new("mine", "Mine", &["my-agent"], &[]);
        let found = match_process("/x/my-agent", &["my-agent".to_string()], &[custom]);
        assert_eq!(found.map(|k| k.id).as_deref(), Some("mine"));
    }
}
