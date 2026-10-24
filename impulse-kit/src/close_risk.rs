use serde::{Deserialize, Serialize};

/// User action that will destroy app state if confirmed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseRiskAction {
    Quit,
    CloseWindow,
    CloseTab,
}

/// One command that is currently running in a terminal.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunningCommandRisk {
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub started_at_ms: u64,
}

/// Inputs a frontend can collect before closing a window or quitting.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CloseRiskInput {
    pub action: CloseRiskAction,
    #[serde(default)]
    pub unsaved_editor_count: usize,
    #[serde(default)]
    pub running_terminal_process_count: usize,
    #[serde(default)]
    pub running_commands: Vec<RunningCommandRisk>,
    #[serde(default)]
    pub now_ms: u64,
    #[serde(default = "default_long_command_threshold_seconds")]
    pub long_command_threshold_seconds: u64,
}

/// One normalized command entry suitable for display.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CloseRiskCommandSummary {
    pub command: String,
    pub cwd: Option<String>,
    pub duration_seconds: u64,
    pub is_long_running: bool,
}

/// Summary used by frontends to decide whether to prompt and what to show.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CloseRiskSummary {
    pub has_risk: bool,
    pub title: String,
    pub informative_text: String,
    pub detail_lines: Vec<String>,
    pub destructive_action_title: String,
    pub cancel_title: String,
    pub unsaved_editor_count: usize,
    pub running_terminal_process_count: usize,
    pub running_command_count: usize,
    pub long_running_command_count: usize,
    pub commands: Vec<CloseRiskCommandSummary>,
}

impl CloseRiskInput {
    pub fn summarize(&self) -> CloseRiskSummary {
        summarize_close_risk(self)
    }
}

pub fn summarize_close_risk(input: &CloseRiskInput) -> CloseRiskSummary {
    let threshold = input.long_command_threshold_seconds.max(1);
    let commands = summarize_commands(input, threshold);
    let running_command_count = commands.len();
    let long_running_command_count = commands
        .iter()
        .filter(|command| command.is_long_running)
        .count();
    let has_risk = input.unsaved_editor_count > 0
        || input.running_terminal_process_count > 0
        || running_command_count > 0;

    let destructive_action_title = match input.action {
        CloseRiskAction::Quit => "Quit",
        CloseRiskAction::CloseWindow => "Close Window",
        CloseRiskAction::CloseTab => "Close Tab",
    }
    .to_string();

    if !has_risk {
        return CloseRiskSummary {
            has_risk: false,
            title: String::new(),
            informative_text: String::new(),
            detail_lines: Vec::new(),
            destructive_action_title,
            cancel_title: "Cancel".to_string(),
            unsaved_editor_count: 0,
            running_terminal_process_count: 0,
            running_command_count: 0,
            long_running_command_count: 0,
            commands,
        };
    }

    let title = close_title(input);
    let informative_text = close_informative_text(input, running_command_count);
    let detail_lines = close_detail_lines(input, &commands);

    CloseRiskSummary {
        has_risk,
        title,
        informative_text,
        detail_lines,
        destructive_action_title,
        cancel_title: "Cancel".to_string(),
        unsaved_editor_count: input.unsaved_editor_count,
        running_terminal_process_count: input.running_terminal_process_count,
        running_command_count,
        long_running_command_count,
        commands,
    }
}

fn default_long_command_threshold_seconds() -> u64 {
    30
}

fn summarize_commands(input: &CloseRiskInput, threshold: u64) -> Vec<CloseRiskCommandSummary> {
    input
        .running_commands
        .iter()
        .map(|command| {
            let duration_seconds = input.now_ms.saturating_sub(command.started_at_ms) / 1000;
            CloseRiskCommandSummary {
                command: display_command(command.command.as_deref()),
                cwd: command.cwd.clone(),
                duration_seconds,
                is_long_running: duration_seconds >= threshold,
            }
        })
        .collect()
}

fn close_title(input: &CloseRiskInput) -> String {
    let action = match input.action {
        CloseRiskAction::Quit => "Quit Impulse",
        CloseRiskAction::CloseWindow => "Close window",
        CloseRiskAction::CloseTab => "Close tab",
    };
    let has_unsaved = input.unsaved_editor_count > 0;
    let has_terminal =
        input.running_terminal_process_count > 0 || !input.running_commands.is_empty();

    match (has_unsaved, has_terminal) {
        (true, true) => format!("{action} with unsaved changes and running terminal work?"),
        (true, false) => format!("{action} with unsaved changes?"),
        (false, true) => format!("{action} with running terminal work?"),
        (false, false) => String::new(),
    }
}

fn close_informative_text(input: &CloseRiskInput, running_command_count: usize) -> String {
    let mut sentences = Vec::new();
    if input.unsaved_editor_count == 1 {
        sentences.push("1 editor has unsaved changes that may be lost.".to_string());
    } else if input.unsaved_editor_count > 1 {
        sentences.push(format!(
            "{} editors have unsaved changes that may be lost.",
            input.unsaved_editor_count
        ));
    }

    if input.running_terminal_process_count == 1 {
        sentences.push("1 terminal process will be terminated.".to_string());
    } else if input.running_terminal_process_count > 1 {
        sentences.push(format!(
            "{} terminal processes will be terminated.",
            input.running_terminal_process_count
        ));
    }

    if running_command_count == 1 {
        sentences.push("1 running command will be stopped.".to_string());
    } else if running_command_count > 1 {
        sentences.push(format!(
            "{running_command_count} running commands will be stopped."
        ));
    }

    sentences.join(" ")
}

fn close_detail_lines(input: &CloseRiskInput, commands: &[CloseRiskCommandSummary]) -> Vec<String> {
    let mut lines = Vec::new();
    if input.unsaved_editor_count > 0 {
        lines.push(plural_line(input.unsaved_editor_count, "unsaved editor"));
    }
    if input.running_terminal_process_count > 0 {
        lines.push(plural_line(
            input.running_terminal_process_count,
            "running terminal process",
        ));
    }

    for command in commands.iter().take(3) {
        lines.push(format!(
            "{} running for {}",
            command.command,
            format_duration(command.duration_seconds)
        ));
    }
    if commands.len() > 3 {
        lines.push(format!("{} more running commands", commands.len() - 3));
    }

    lines
}

fn plural_line(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else if noun.ends_with("process") {
        format!("{count} {noun}es")
    } else {
        format!("{count} {noun}s")
    }
}

fn display_command(command: Option<&str>) -> String {
    let trimmed = command.unwrap_or("").trim();
    if trimmed.is_empty() {
        return "Running command".to_string();
    }
    const MAX_COMMAND_CHARS: usize = 80;
    if trimmed.chars().count() <= MAX_COMMAND_CHARS {
        return trimmed.to_string();
    }
    let mut result: String = trimmed.chars().take(MAX_COMMAND_CHARS - 3).collect();
    result.push_str("...");
    result
}

fn format_duration(seconds: u64) -> String {
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    let remaining_seconds = seconds % 60;
    if minutes < 60 {
        return format!("{minutes}m {remaining_seconds:02}s");
    }
    let hours = minutes / 60;
    let remaining_minutes = minutes % 60;
    format!("{hours}h {remaining_minutes:02}m")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_risk_returns_empty_summary() {
        let summary = summarize_close_risk(&CloseRiskInput {
            action: CloseRiskAction::Quit,
            unsaved_editor_count: 0,
            running_terminal_process_count: 0,
            running_commands: Vec::new(),
            now_ms: 10_000,
            long_command_threshold_seconds: 30,
        });

        assert!(!summary.has_risk);
        assert_eq!(summary.title, "");
        assert!(summary.detail_lines.is_empty());
    }

    #[test]
    fn summarizes_quit_with_running_command() {
        let summary = summarize_close_risk(&CloseRiskInput {
            action: CloseRiskAction::Quit,
            unsaved_editor_count: 0,
            running_terminal_process_count: 1,
            running_commands: vec![RunningCommandRisk {
                command: Some("cargo test -p impulse-core".to_string()),
                cwd: Some("/tmp/project".to_string()),
                started_at_ms: 1_000,
            }],
            now_ms: 66_000,
            long_command_threshold_seconds: 30,
        });

        assert!(summary.has_risk);
        assert_eq!(summary.title, "Quit Impulse with running terminal work?");
        assert_eq!(summary.running_command_count, 1);
        assert_eq!(summary.long_running_command_count, 1);
        assert_eq!(
            summary.detail_lines,
            vec![
                "1 running terminal process",
                "cargo test -p impulse-core running for 1m 05s",
            ]
        );
    }

    #[test]
    fn summarizes_window_close_with_unsaved_and_processes() {
        let summary = summarize_close_risk(&CloseRiskInput {
            action: CloseRiskAction::CloseWindow,
            unsaved_editor_count: 2,
            running_terminal_process_count: 3,
            running_commands: Vec::new(),
            now_ms: 0,
            long_command_threshold_seconds: 30,
        });

        assert_eq!(
            summary.title,
            "Close window with unsaved changes and running terminal work?"
        );
        assert_eq!(
            summary.informative_text,
            "2 editors have unsaved changes that may be lost. 3 terminal processes will be terminated."
        );
        assert_eq!(
            summary.detail_lines,
            vec!["2 unsaved editors", "3 running terminal processes"]
        );
    }

    #[test]
    fn summarizes_tab_close_with_running_command() {
        let summary = summarize_close_risk(&CloseRiskInput {
            action: CloseRiskAction::CloseTab,
            unsaved_editor_count: 0,
            running_terminal_process_count: 1,
            running_commands: vec![RunningCommandRisk {
                command: Some("lazygit".to_string()),
                cwd: None,
                started_at_ms: 0,
            }],
            now_ms: 31_000,
            long_command_threshold_seconds: 30,
        });

        assert!(summary.has_risk);
        assert_eq!(summary.title, "Close tab with running terminal work?");
        assert_eq!(summary.destructive_action_title, "Close Tab");
        assert_eq!(summary.long_running_command_count, 1);
    }

    #[test]
    fn formats_unknown_and_short_running_command() {
        let summary = summarize_close_risk(&CloseRiskInput {
            action: CloseRiskAction::CloseWindow,
            unsaved_editor_count: 0,
            running_terminal_process_count: 0,
            running_commands: vec![RunningCommandRisk {
                command: Some("  ".to_string()),
                cwd: None,
                started_at_ms: 12_000,
            }],
            now_ms: 20_000,
            long_command_threshold_seconds: 30,
        });

        assert_eq!(summary.commands[0].command, "Running command");
        assert_eq!(summary.commands[0].duration_seconds, 8);
        assert!(!summary.commands[0].is_long_running);
    }
}
