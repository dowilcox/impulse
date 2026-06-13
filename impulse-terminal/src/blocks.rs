use serde::Serialize;

const MAX_COMMAND_BLOCKS: usize = 10_000;
const MAX_BLOCK_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_COMPLETED_BLOCK_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

/// Stable identifier for a command block in a terminal session.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
pub struct TerminalBlockId(pub u64);

/// Metadata for one shell command observed through shell integration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TerminalCommandBlock {
    pub id: TerminalBlockId,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub output_start_line: u64,
    pub output_end_line: Option<u64>,
    pub output: String,
    /// Absolute grid row (history + screen + eviction base) of the prompt
    /// that precedes this command (OSC 133;A). Exact even with wrapped
    /// lines, unlike the newline-counted `output_start_line`.
    pub prompt_row: Option<i64>,
    /// Absolute grid row where command output begins (OSC 133;C).
    pub output_row: Option<i64>,
    /// Absolute grid row of the cursor when the command finished (OSC 133;D).
    pub end_row: Option<i64>,
}

#[derive(Debug, Default)]
pub(crate) struct CommandBlockTracker {
    next_id: u64,
    pending_command: Option<String>,
    cwd: Option<String>,
    output_line: u64,
    /// Absolute grid row of the most recent OSC 133;A prompt-start mark.
    /// Consumed by the next command block; while no command is running it
    /// identifies the live input-prompt region.
    pending_prompt_row: Option<i64>,
    /// Estimated number of history lines evicted at the scrollback cap.
    /// Added to grid coordinates so absolute rows stay monotonic.
    row_base: i64,
    current: Option<TerminalCommandBlock>,
    completed: Vec<TerminalCommandBlock>,
    completed_output_bytes: usize,
}

impl CommandBlockTracker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn set_cwd(&mut self, cwd: String) {
        self.cwd = Some(cwd);
    }

    pub(crate) fn set_pending_command(&mut self, command: String) {
        self.pending_command = Some(command);
    }

    pub(crate) fn observe_output(&mut self, bytes: &[u8]) {
        self.output_line += bytes.iter().filter(|b| **b == b'\n').count() as u64;
        if let Some(block) = &mut self.current {
            append_plain_text_output(block, bytes);
        }
    }

    /// Record the absolute grid row of an OSC 133;A prompt-start mark.
    pub(crate) fn prompt_marked(&mut self, abs_row: i64) {
        self.pending_prompt_row = Some(abs_row);
    }

    /// Absolute grid row of the live input prompt, when the shell is idle at
    /// a prompt (no command running).
    pub(crate) fn pending_prompt_row(&self) -> Option<i64> {
        if self.current.is_some() {
            None
        } else {
            self.pending_prompt_row
        }
    }

    /// Grow the eviction estimate once the scrollback cap is reached, so
    /// absolute rows recorded before and after eviction stay comparable.
    pub(crate) fn bump_row_base(&mut self, evicted_lines: u64) {
        self.row_base += evicted_lines.min(i64::MAX as u64) as i64;
    }

    pub(crate) fn row_base(&self) -> i64 {
        self.row_base
    }

    pub(crate) fn command_started(&mut self, abs_row: Option<i64>) -> TerminalCommandBlock {
        self.command_started_at(current_time_ms(), abs_row)
    }

    pub(crate) fn command_started_at(
        &mut self,
        started_at_ms: u64,
        abs_row: Option<i64>,
    ) -> TerminalCommandBlock {
        if let Some(mut current) = self.current.take() {
            current.ended_at_ms = Some(started_at_ms);
            current.output_end_line = Some(self.output_line);
            current.end_row = abs_row;
            self.push_completed(current);
        }

        self.next_id += 1;
        let block = TerminalCommandBlock {
            id: TerminalBlockId(self.next_id),
            command: self.pending_command.take(),
            cwd: self.cwd.clone(),
            started_at_ms,
            ended_at_ms: None,
            exit_code: None,
            output_start_line: self.output_line,
            output_end_line: None,
            output: String::new(),
            prompt_row: self.pending_prompt_row.take(),
            output_row: abs_row,
            end_row: None,
        };
        self.current = Some(block.clone());
        block
    }

    pub(crate) fn command_ended(
        &mut self,
        exit_code: i32,
        abs_row: Option<i64>,
    ) -> Option<TerminalCommandBlock> {
        self.command_ended_at(exit_code, current_time_ms(), abs_row)
    }

    pub(crate) fn command_ended_at(
        &mut self,
        exit_code: i32,
        ended_at_ms: u64,
        abs_row: Option<i64>,
    ) -> Option<TerminalCommandBlock> {
        let mut block = self.current.take()?;
        block.ended_at_ms = Some(ended_at_ms);
        block.exit_code = Some(exit_code);
        block.output_end_line = Some(self.output_line);
        block.end_row = abs_row;
        self.push_completed(block.clone());
        Some(block)
    }

    pub(crate) fn blocks(&self) -> Vec<TerminalCommandBlock> {
        let mut blocks = self.completed.clone();
        if let Some(current) = &self.current {
            blocks.push(current.clone());
        }
        blocks
    }

    /// Iterate blocks oldest-first without cloning their captured output.
    pub(crate) fn iter_blocks(&self) -> impl Iterator<Item = &TerminalCommandBlock> {
        self.completed.iter().chain(self.current.iter())
    }

    /// Absolute grid row of a block's first line (prompt if marked,
    /// otherwise the start of its output).
    pub(crate) fn block_top_row(&self, id: TerminalBlockId) -> Option<i64> {
        self.iter_blocks()
            .find(|block| block.id == id)
            .and_then(|block| block.prompt_row.or(block.output_row))
    }

    pub(crate) fn has_command_text(&self) -> bool {
        self.completed
            .iter()
            .chain(self.current.iter())
            .any(|block| {
                block
                    .command
                    .as_ref()
                    .is_some_and(|command| !command.trim().is_empty())
            })
    }

    pub(crate) fn has_output(&self) -> bool {
        self.completed
            .iter()
            .chain(self.current.iter())
            .any(|block| !block.output.is_empty())
    }

    pub(crate) fn has_failed_command(&self) -> bool {
        self.completed
            .iter()
            .chain(self.current.iter())
            .any(|block| block.exit_code.is_some_and(|code| code != 0))
    }

    pub(crate) fn current_output_line(&self) -> u64 {
        self.output_line
    }

    pub(crate) fn block_start_line(&self, id: TerminalBlockId) -> Option<u64> {
        self.completed
            .iter()
            .chain(self.current.iter())
            .find(|block| block.id == id)
            .map(|block| block.output_start_line)
    }

    fn push_completed(&mut self, block: TerminalCommandBlock) {
        self.completed_output_bytes += block.output.len();
        self.completed.push(block);
        if self.completed.len() > MAX_COMMAND_BLOCKS {
            let excess = self.completed.len() - MAX_COMMAND_BLOCKS;
            for block in self.completed.drain(0..excess) {
                self.completed_output_bytes = self
                    .completed_output_bytes
                    .saturating_sub(block.output.len());
            }
        }
        while self.completed_output_bytes > MAX_COMPLETED_BLOCK_OUTPUT_BYTES {
            let Some(block) = self
                .completed
                .iter_mut()
                .find(|block| !block.output.is_empty())
            else {
                self.completed_output_bytes = 0;
                break;
            };
            self.completed_output_bytes = self
                .completed_output_bytes
                .saturating_sub(block.output.len());
            block.output.clear();
        }
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn append_plain_text_output(block: &mut TerminalCommandBlock, bytes: &[u8]) {
    if block.output.len() >= MAX_BLOCK_OUTPUT_BYTES {
        return;
    }

    let text = plain_text_from_terminal_bytes(bytes);
    if text.is_empty() {
        return;
    }

    let remaining = MAX_BLOCK_OUTPUT_BYTES - block.output.len();
    if text.len() <= remaining {
        block.output.push_str(&text);
        return;
    }

    let mut end = remaining;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    block.output.push_str(&text[..end]);
}

fn plain_text_from_terminal_bytes(bytes: &[u8]) -> String {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum State {
        Normal,
        Escape,
        Csi,
        Osc,
        OscEscape,
        SkipOne,
    }

    let mut state = State::Normal;
    let mut out = Vec::with_capacity(bytes.len());

    for &byte in bytes {
        match state {
            State::Normal => match byte {
                0x1b => state = State::Escape,
                b'\n' | b'\t' => out.push(byte),
                b'\r' | 0x08 | 0x00..=0x07 | 0x0b..=0x1f | 0x7f => {}
                _ => out.push(byte),
            },
            State::Escape => match byte {
                b'[' => state = State::Csi,
                b']' => state = State::Osc,
                b'P' | b'^' | b'_' => state = State::Osc,
                b'(' | b')' | b'*' | b'+' | b'-' | b'.' | b'/' => state = State::SkipOne,
                _ => state = State::Normal,
            },
            State::Csi => {
                if (0x40..=0x7e).contains(&byte) {
                    state = State::Normal;
                }
            }
            State::Osc => match byte {
                0x07 => state = State::Normal,
                0x1b => state = State::OscEscape,
                _ => {}
            },
            State::OscEscape => {
                state = State::Normal;
            }
            State::SkipOne => {
                state = State::Normal;
            }
        }
    }

    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_completed_command_block_with_command_cwd_exit_and_lines() {
        let mut tracker = CommandBlockTracker::new();
        tracker.set_cwd("/tmp/project".to_string());
        tracker.set_pending_command("cargo test".to_string());

        tracker.prompt_marked(38);
        let started = tracker.command_started_at(100, Some(40));
        assert_eq!(started.id, TerminalBlockId(1));
        assert_eq!(started.command.as_deref(), Some("cargo test"));
        assert_eq!(started.cwd.as_deref(), Some("/tmp/project"));
        assert_eq!(started.output_start_line, 0);
        assert_eq!(started.output, "");
        assert_eq!(started.prompt_row, Some(38));
        assert_eq!(started.output_row, Some(40));
        assert_eq!(tracker.pending_prompt_row(), None);

        tracker.observe_output(b"running 1 test\nok\n");
        let ended = tracker.command_ended_at(0, 250, Some(43)).unwrap();

        assert_eq!(ended.id, TerminalBlockId(1));
        assert_eq!(ended.exit_code, Some(0));
        assert_eq!(ended.started_at_ms, 100);
        assert_eq!(ended.ended_at_ms, Some(250));
        assert_eq!(ended.output_start_line, 0);
        assert_eq!(ended.output_end_line, Some(2));
        assert_eq!(ended.output, "running 1 test\nok\n");
        assert_eq!(ended.end_row, Some(43));
        assert_eq!(tracker.blocks(), vec![ended]);
    }

    #[test]
    fn tracks_live_prompt_region_and_row_base() {
        let mut tracker = CommandBlockTracker::new();
        tracker.prompt_marked(5);
        assert_eq!(tracker.pending_prompt_row(), Some(5));

        tracker.command_started_at(10, Some(6));
        assert_eq!(tracker.pending_prompt_row(), None);
        tracker.command_ended_at(0, 20, Some(9));

        tracker.prompt_marked(9);
        assert_eq!(tracker.pending_prompt_row(), Some(9));

        assert_eq!(tracker.row_base(), 0);
        tracker.bump_row_base(12);
        assert_eq!(tracker.row_base(), 12);
    }

    #[test]
    fn includes_running_block_in_snapshot() {
        let mut tracker = CommandBlockTracker::new();
        tracker.set_pending_command("sleep 10".to_string());
        tracker.command_started_at(10, None);
        tracker.observe_output(b"waiting\n");

        let blocks = tracker.blocks();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].command.as_deref(), Some("sleep 10"));
        assert_eq!(blocks[0].exit_code, None);
        assert_eq!(blocks[0].output_start_line, 0);
        assert_eq!(blocks[0].output_end_line, None);
        assert_eq!(blocks[0].output, "waiting\n");
    }

    #[test]
    fn closes_unfinished_block_when_a_new_command_starts() {
        let mut tracker = CommandBlockTracker::new();
        tracker.set_pending_command("first".to_string());
        tracker.command_started_at(10, None);
        tracker.observe_output(b"line\n");

        tracker.set_pending_command("second".to_string());
        let second = tracker.command_started_at(20, None);

        let blocks = tracker.blocks();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].command.as_deref(), Some("first"));
        assert_eq!(blocks[0].ended_at_ms, Some(20));
        assert_eq!(blocks[0].exit_code, None);
        assert_eq!(blocks[0].output_end_line, Some(1));
        assert_eq!(blocks[0].output, "line\n");
        assert_eq!(blocks[1], second);
    }

    #[test]
    fn strips_terminal_escape_sequences_from_copied_output() {
        let mut tracker = CommandBlockTracker::new();
        tracker.command_started_at(10, None);
        tracker.observe_output(
            b"\x1b[31mred\x1b[0m\n\x1b]6973;Command=secret\x07visible\x1b]133;D;0\x07",
        );

        let block = tracker.blocks().pop().unwrap();
        assert_eq!(block.output, "red\nvisible");
    }

    #[test]
    fn output_observed_after_command_end_is_not_captured() {
        let mut tracker = CommandBlockTracker::new();
        tracker.command_started_at(10, None);
        tracker.observe_output(b"result\n");
        let ended = tracker.command_ended_at(0, 20, None).unwrap();

        tracker.observe_output(b"prompt$ ");

        assert_eq!(ended.output, "result\n");
        assert_eq!(tracker.current_output_line(), 1);
        assert_eq!(tracker.block_start_line(TerminalBlockId(1)), Some(0));
    }
}
