use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::blocks::TerminalCommandBlock;

const DEFAULT_MAX_HISTORY_RECORDS: usize = 10_000;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommandHistoryContext {
    pub session_id: Option<String>,
    pub shell: Option<String>,
    pub git_branch: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommandHistoryRecord {
    pub id: u64,
    pub command: String,
    pub cwd: Option<String>,
    pub shell: Option<String>,
    pub exit_code: Option<i32>,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub git_branch: Option<String>,
    pub session_id: Option<String>,
    pub block_id: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommandHistoryQuery {
    pub text: String,
    pub cwd: Option<String>,
    pub session_id: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CommandHistoryMatchKind {
    Recent,
    Prefix,
    Fuzzy,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommandHistorySearchResult {
    pub record: CommandHistoryRecord,
    pub kind: CommandHistoryMatchKind,
}

#[derive(Clone, Debug)]
pub struct CommandHistoryStore {
    next_id: u64,
    max_records: usize,
    records: VecDeque<CommandHistoryRecord>,
}

/// Build the exact PTY input used to rerun a stored command.
///
/// The command is sent back to the interactive shell as typed text, not wrapped
/// in `sh -c` and not shell-escaped. Rejecting control bytes keeps a corrupted
/// history record from injecting terminal control sequences while still
/// allowing tabs and multiline shell input.
pub fn command_history_rerun_input(command: &str) -> Option<String> {
    let normalized = command.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = normalized.trim_matches('\n');
    if trimmed.trim().is_empty() {
        return None;
    }
    if trimmed.chars().any(is_disallowed_rerun_control) {
        return None;
    }
    Some(format!("{trimmed}\n"))
}

fn is_disallowed_rerun_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{0000}'..='\u{0008}'
            | '\u{000b}'..='\u{000c}'
            | '\u{000e}'..='\u{001f}'
            | '\u{007f}'
    )
}

impl Default for CommandHistoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHistoryStore {
    pub fn new() -> Self {
        Self::with_max_records(DEFAULT_MAX_HISTORY_RECORDS)
    }

    pub fn with_max_records(max_records: usize) -> Self {
        Self {
            next_id: 1,
            max_records,
            records: VecDeque::new(),
        }
    }

    pub fn record_completed_block(
        &mut self,
        block: &TerminalCommandBlock,
        context: CommandHistoryContext,
    ) -> Option<CommandHistoryRecord> {
        let command = block.command.as_deref()?.trim();
        if command.is_empty() {
            return None;
        }
        let ended_at_ms = block.ended_at_ms?;

        let record = CommandHistoryRecord {
            id: self.next_record_id(),
            command: command.to_string(),
            cwd: block.cwd.clone(),
            shell: context.shell,
            exit_code: block.exit_code,
            started_at_ms: block.started_at_ms,
            ended_at_ms,
            git_branch: context.git_branch,
            session_id: context.session_id,
            block_id: block.id.0,
        };
        self.push_record(record.clone());
        Some(record)
    }

    pub fn push_record(&mut self, record: CommandHistoryRecord) {
        self.next_id = self.next_id.max(record.id.saturating_add(1).max(1));
        self.records.push_back(record);
        self.enforce_capacity();
    }

    pub fn records(&self) -> Vec<CommandHistoryRecord> {
        self.records.iter().cloned().collect()
    }

    pub fn recent_records(&self) -> Vec<CommandHistoryRecord> {
        self.records.iter().rev().cloned().collect()
    }

    pub fn search(&self, query: &CommandHistoryQuery) -> Vec<CommandHistorySearchResult> {
        let text = query.text.trim().to_lowercase();
        let limit = query.limit.unwrap_or(usize::MAX);
        if limit == 0 {
            return Vec::new();
        }
        if text.is_empty() {
            let mut results = Vec::new();
            for score in (0..=3).rev() {
                for record in self.records.iter().rev() {
                    if context_score(record, query) != score {
                        continue;
                    }
                    results.push(CommandHistorySearchResult {
                        record: record.clone(),
                        kind: CommandHistoryMatchKind::Recent,
                    });
                    if results.len() >= limit {
                        return results;
                    }
                }
            }
            return results;
        }

        let mut matches: Vec<_> = self
            .records
            .iter()
            .filter_map(|record| {
                let (kind, match_score) = match_command(&record.command, &text)?;
                Some(CommandHistoryCandidate {
                    record: record.clone(),
                    kind,
                    context_score: context_score(record, query),
                    match_score,
                })
            })
            .collect();

        matches.sort_by(|left, right| {
            right
                .context_score
                .cmp(&left.context_score)
                .then_with(|| match_kind_rank(right.kind).cmp(&match_kind_rank(left.kind)))
                .then_with(|| right.match_score.cmp(&left.match_score))
                .then_with(|| right.record.id.cmp(&left.record.id))
        });

        matches
            .into_iter()
            .take(limit)
            .map(|candidate| CommandHistorySearchResult {
                record: candidate.record,
                kind: candidate.kind,
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn clear(&mut self) {
        self.records.clear();
    }

    fn next_record_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1).max(1);
        id
    }

    fn enforce_capacity(&mut self) {
        if self.max_records == 0 {
            self.records.clear();
            return;
        }

        while self.records.len() > self.max_records {
            self.records.pop_front();
        }
    }
}

struct CommandHistoryCandidate {
    record: CommandHistoryRecord,
    kind: CommandHistoryMatchKind,
    context_score: u8,
    match_score: i64,
}

fn context_score(record: &CommandHistoryRecord, query: &CommandHistoryQuery) -> u8 {
    let mut score = 0;
    if query
        .session_id
        .as_ref()
        .is_some_and(|session_id| record.session_id.as_ref() == Some(session_id))
    {
        score += 2;
    }
    if query
        .cwd
        .as_ref()
        .is_some_and(|cwd| record.cwd.as_ref() == Some(cwd))
    {
        score += 1;
    }
    score
}

fn match_command(command: &str, query: &str) -> Option<(CommandHistoryMatchKind, i64)> {
    if query.is_empty() {
        return Some((CommandHistoryMatchKind::Recent, 0));
    }

    let command = command.to_lowercase();
    if command.starts_with(query) {
        return Some((
            CommandHistoryMatchKind::Prefix,
            1_000_i64.saturating_sub(command.len() as i64),
        ));
    }

    fuzzy_score(&command, query).map(|score| (CommandHistoryMatchKind::Fuzzy, score))
}

fn fuzzy_score(command: &str, query: &str) -> Option<i64> {
    let mut query_chars = query.chars();
    let mut needle = query_chars.next()?;
    let mut first_match = None;
    let mut matched = 0usize;

    for (index, ch) in command.char_indices() {
        if ch != needle {
            continue;
        }
        first_match.get_or_insert(index);
        matched += 1;
        match query_chars.next() {
            Some(next) => needle = next,
            None => {
                let span = index.saturating_sub(first_match.unwrap_or(index)) + 1;
                return Some((matched as i64 * 16) - span as i64);
            }
        }
    }

    None
}

fn match_kind_rank(kind: CommandHistoryMatchKind) -> u8 {
    match kind {
        CommandHistoryMatchKind::Recent => 0,
        CommandHistoryMatchKind::Fuzzy => 1,
        CommandHistoryMatchKind::Prefix => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::{TerminalBlockId, TerminalCommandBlock};

    fn block(command: Option<&str>, ended_at_ms: Option<u64>) -> TerminalCommandBlock {
        TerminalCommandBlock {
            id: TerminalBlockId(42),
            command: command.map(str::to_string),
            cwd: Some("/repo".to_string()),
            started_at_ms: 100,
            ended_at_ms,
            exit_code: Some(0),
            output_start_line: 3,
            output_end_line: Some(7),
            output: "ok\n".to_string(),
            prompt_row: None,
            output_row: None,
            end_row: None,
        }
    }

    #[test]
    fn records_completed_block_metadata_without_output() {
        let mut store = CommandHistoryStore::new();
        let context = CommandHistoryContext {
            session_id: Some("session-1".to_string()),
            shell: Some("zsh".to_string()),
            git_branch: Some("main".to_string()),
        };

        let record = store.record_completed_block(&block(Some(" cargo test "), Some(250)), context);

        let record = record.expect("completed command should be recorded");
        assert_eq!(record.id, 1);
        assert_eq!(record.command, "cargo test");
        assert_eq!(record.cwd.as_deref(), Some("/repo"));
        assert_eq!(record.shell.as_deref(), Some("zsh"));
        assert_eq!(record.exit_code, Some(0));
        assert_eq!(record.started_at_ms, 100);
        assert_eq!(record.ended_at_ms, 250);
        assert_eq!(record.git_branch.as_deref(), Some("main"));
        assert_eq!(record.session_id.as_deref(), Some("session-1"));
        assert_eq!(record.block_id, 42);
        assert_eq!(store.records(), vec![record]);
    }

    #[test]
    fn ignores_running_or_blank_blocks() {
        let mut store = CommandHistoryStore::new();

        assert_eq!(
            store.record_completed_block(&block(Some("sleep 10"), None), Default::default()),
            None
        );
        assert_eq!(
            store.record_completed_block(&block(Some("   "), Some(200)), Default::default()),
            None
        );
        assert_eq!(
            store.record_completed_block(&block(None, Some(200)), Default::default()),
            None
        );
        assert!(store.is_empty());
    }

    #[test]
    fn evicts_oldest_records_when_capacity_is_exceeded() {
        let mut store = CommandHistoryStore::with_max_records(2);

        for command in ["one", "two", "three"] {
            store.record_completed_block(&block(Some(command), Some(200)), Default::default());
        }

        let records = store.records();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].command, "two");
        assert_eq!(records[1].command, "three");
        assert_eq!(records[0].id, 2);
        assert_eq!(records[1].id, 3);
    }

    #[test]
    fn recent_records_are_newest_first() {
        let mut store = CommandHistoryStore::new();
        for command in ["build", "test"] {
            store.record_completed_block(&block(Some(command), Some(200)), Default::default());
        }

        let recent = store.recent_records();
        assert_eq!(recent[0].command, "test");
        assert_eq!(recent[1].command, "build");
    }

    #[test]
    fn search_matches_prefix_before_fuzzy() {
        let mut store = CommandHistoryStore::new();
        for command in ["cargo build", "git checkout feature", "cargo test"] {
            store.record_completed_block(&block(Some(command), Some(200)), Default::default());
        }

        let results = store.search(&CommandHistoryQuery {
            text: "cargo".to_string(),
            ..Default::default()
        });

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].record.command, "cargo test");
        assert_eq!(results[0].kind, CommandHistoryMatchKind::Prefix);
        assert_eq!(results[1].record.command, "cargo build");
        assert_eq!(results[1].kind, CommandHistoryMatchKind::Prefix);
    }

    #[test]
    fn search_supports_fuzzy_matching() {
        let mut store = CommandHistoryStore::new();
        store.record_completed_block(
            &block(Some("docker compose up"), Some(200)),
            Default::default(),
        );

        let results = store.search(&CommandHistoryQuery {
            text: "dcu".to_string(),
            ..Default::default()
        });

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].record.command, "docker compose up");
        assert_eq!(results[0].kind, CommandHistoryMatchKind::Fuzzy);
    }

    #[test]
    fn search_prefers_current_session_and_cwd() {
        let mut store = CommandHistoryStore::new();
        store.record_completed_block(
            &block(Some("cargo test global"), Some(200)),
            Default::default(),
        );

        let mut local = block(Some("cargo test local"), Some(200));
        local.cwd = Some("/repo/app".to_string());
        store.record_completed_block(
            &local,
            CommandHistoryContext {
                session_id: Some("session-2".to_string()),
                shell: None,
                git_branch: None,
            },
        );

        let results = store.search(&CommandHistoryQuery {
            text: "cargo".to_string(),
            cwd: Some("/repo/app".to_string()),
            session_id: Some("session-2".to_string()),
            limit: Some(1),
        });

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].record.command, "cargo test local");
    }

    #[test]
    fn empty_search_returns_recent_records_with_limit() {
        let mut store = CommandHistoryStore::new();
        for command in ["one", "two", "three"] {
            store.record_completed_block(&block(Some(command), Some(200)), Default::default());
        }

        let results = store.search(&CommandHistoryQuery {
            limit: Some(2),
            ..Default::default()
        });

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].record.command, "three");
        assert_eq!(results[0].kind, CommandHistoryMatchKind::Recent);
        assert_eq!(results[1].record.command, "two");
    }

    #[test]
    fn rerun_input_appends_single_newline_without_shell_escaping() {
        assert_eq!(
            command_history_rerun_input("echo '$HOME'"),
            Some("echo '$HOME'\n".to_string())
        );
        assert_eq!(
            command_history_rerun_input("printf one\r\nprintf two\n"),
            Some("printf one\nprintf two\n".to_string())
        );
    }

    #[test]
    fn rerun_input_rejects_empty_and_control_sequences() {
        assert_eq!(command_history_rerun_input(" \n\t "), None);
        assert_eq!(command_history_rerun_input("echo hi\u{1b}[2J"), None);
        assert_eq!(command_history_rerun_input("echo hi\u{7f}"), None);
    }
}
