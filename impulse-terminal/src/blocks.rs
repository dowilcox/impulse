use std::collections::VecDeque;

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

/// A block without its captured output (cheap to list and serialize).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TerminalCommandBlockSummary {
    pub id: TerminalBlockId,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub output_start_line: u64,
    pub output_end_line: Option<u64>,
    pub has_output: bool,
}

impl From<&TerminalCommandBlock> for TerminalCommandBlockSummary {
    fn from(block: &TerminalCommandBlock) -> Self {
        Self {
            id: block.id,
            command: block.command.clone(),
            cwd: block.cwd.clone(),
            started_at_ms: block.started_at_ms,
            ended_at_ms: block.ended_at_ms,
            exit_code: block.exit_code,
            output_start_line: block.output_start_line,
            output_end_line: block.output_end_line,
            has_output: !block.output.is_empty(),
        }
    }
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
    /// Absolute grid row just below a previous session's output put back
    /// before the shell started (where the shell's own output began).
    restored_end: Option<i64>,
    /// Estimated number of history lines evicted at the scrollback cap.
    /// Added to grid coordinates so absolute rows stay monotonic.
    row_base: i64,
    current: Option<TerminalCommandBlock>,
    completed: VecDeque<TerminalCommandBlock>,
    completed_output_bytes: usize,
    /// Bumped whenever block boundaries or prompt marks change, so callers
    /// can cache anything derived from them (the viewport overlay).
    version: u64,
    /// Plain-text conversion of the running block's output, which arrives in
    /// reads that can split escape sequences and characters.
    stripper: PlainTextStripper,
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

    pub(crate) fn version(&self) -> u64 {
        self.version
    }

    fn touch(&mut self) {
        self.version = self.version.wrapping_add(1);
    }

    pub(crate) fn observe_output(&mut self, bytes: &[u8]) {
        self.output_line += bytes.iter().filter(|b| **b == b'\n').count() as u64;
        if let Some(block) = &mut self.current {
            append_plain_text_output(block, &mut self.stripper, bytes);
        }
    }

    /// Record the absolute grid row of an OSC 133;A prompt-start mark.
    pub(crate) fn prompt_marked(&mut self, abs_row: i64) {
        self.pending_prompt_row = Some(abs_row);
        self.touch();
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

    /// The row just below the previous session's output the terminal
    /// started with (a row that starts a line, so reflowing keeps it
    /// exact).
    pub(crate) fn set_restored_end(&mut self, abs_row: i64) {
        self.restored_end = Some(abs_row);
        self.touch();
    }

    pub(crate) fn restored_end(&self) -> Option<i64> {
        self.restored_end
    }

    /// Grow the eviction estimate once the scrollback cap is reached, so
    /// absolute rows recorded before and after eviction stay comparable.
    pub(crate) fn bump_row_base(&mut self, evicted_lines: u64) {
        self.row_base += evicted_lines.min(i64::MAX as u64) as i64;
        self.touch();
    }

    pub(crate) fn row_base(&self) -> i64 {
        self.row_base
    }

    /// Move every recorded row through `map` (a resize reflowed the grid,
    /// so soft-wrapped lines now take more or fewer rows).
    pub(crate) fn remap_rows(&mut self, mut map: impl FnMut(i64) -> i64) {
        let mut remap = |row: &mut Option<i64>| {
            if let Some(row) = row {
                *row = map(*row);
            }
        };
        remap(&mut self.pending_prompt_row);
        remap(&mut self.restored_end);
        for block in self.completed.iter_mut().chain(self.current.iter_mut()) {
            remap(&mut block.prompt_row);
            remap(&mut block.output_row);
            remap(&mut block.end_row);
        }
        self.touch();
    }

    pub(crate) fn command_started(&mut self, abs_row: Option<i64>) -> TerminalCommandBlock {
        self.command_started_at(current_time_ms(), abs_row)
    }

    pub(crate) fn command_started_at(
        &mut self,
        started_at_ms: u64,
        abs_row: Option<i64>,
    ) -> TerminalCommandBlock {
        self.touch();
        if let Some(mut current) = self.current.take() {
            // Shells repaint the prompt (re-emitting OSC 133;A/C) before the
            // real command's marks arrive, producing an empty stub block with
            // no command and no output. Absorb its prompt row into the new
            // block instead of orphaning it, so command + output stay one block.
            let is_empty_stub = current.command.is_none()
                && current.output.is_empty()
                && current.output_end_line.is_none()
                && self.output_line == current.output_start_line;
            if is_empty_stub {
                if self.pending_prompt_row.is_none() {
                    self.pending_prompt_row = current.prompt_row;
                }
            } else {
                current.ended_at_ms = Some(started_at_ms);
                current.output_end_line = Some(self.output_line);
                current.end_row = abs_row;
                self.push_completed(current);
            }
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
        self.stripper = PlainTextStripper::default();
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
        self.touch();
        block.ended_at_ms = Some(ended_at_ms);
        block.exit_code = Some(exit_code);
        block.output_end_line = Some(self.output_line);
        block.end_row = abs_row;
        self.push_completed(block.clone());
        Some(block)
    }

    pub(crate) fn blocks(&self) -> Vec<TerminalCommandBlock> {
        self.iter_blocks().cloned().collect()
    }

    /// Every block, oldest first, without output.
    pub(crate) fn summaries(&self) -> Vec<TerminalCommandBlockSummary> {
        self.iter_blocks()
            .map(TerminalCommandBlockSummary::from)
            .collect()
    }

    /// One block with its output.
    pub(crate) fn block(&self, id: TerminalBlockId) -> Option<TerminalCommandBlock> {
        self.iter_blocks().find(|block| block.id == id).cloned()
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
        self.completed.push_back(block);
        while self.completed.len() > MAX_COMMAND_BLOCKS {
            let Some(block) = self.completed.pop_front() else {
                break;
            };
            self.completed_output_bytes = self
                .completed_output_bytes
                .saturating_sub(block.output.len());
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

fn append_plain_text_output(
    block: &mut TerminalCommandBlock,
    stripper: &mut PlainTextStripper,
    bytes: &[u8],
) {
    if stripper.full {
        return;
    }

    stripper.push(bytes, &mut block.output);

    if block.output.len() > MAX_BLOCK_OUTPUT_BYTES {
        let mut end = MAX_BLOCK_OUTPUT_BYTES;
        while end > 0 && !block.output.is_char_boundary(end) {
            end -= 1;
        }
        block.output.truncate(end);
        stripper.full = true;
    }
}

/// Longest stretch of a line that `\r` and backspace can go back over, in
/// characters. A line that long has wrapped many times on screen, where
/// `\r` only goes back to the start of its last row.
const MAX_LINE_CHARS: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum StripState {
    #[default]
    Normal,
    Escape,
    Csi,
    Osc,
    OscEscape,
    SkipOne,
}

/// Terminal output → plain text, across reads: an escape sequence or a
/// UTF-8 character split between two reads carries over to the next one
/// instead of leaking its tail (`08mfoo`) or turning into U+FFFD.
///
/// The line being written stays editable, as on screen: `\r` and backspace
/// move back over it, new text overwrites, and erase in line (CSI K) clears
/// it, so a progress bar reads as its last state, not `10%20%100%`.
#[derive(Debug, Default)]
pub(crate) struct PlainTextStripper {
    state: StripState,
    /// The start of a UTF-8 character the last read cut off.
    pending: Vec<u8>,
    /// The first parameter of the CSI sequence being read.
    csi_param: u32,
    /// The CSI has a private marker, more parameters or intermediates, so
    /// it isn't a plain erase in line.
    csi_other: bool,
    /// The unfinished line and the cursor's column in it.
    line: Vec<char>,
    column: usize,
    /// How much of the output is finished lines; `line` follows them.
    finished_len: usize,
    /// How many of `line`'s characters the output shows, and the first of
    /// those changed since: an edit at the end is appended, one further
    /// back redraws from there.
    shown: usize,
    stale_from: usize,
    /// The output reached its limit; nothing more is added.
    full: bool,
    /// The last character was an image placeholder, whose marks follow.
    after_placeholder: bool,
}

impl PlainTextStripper {
    /// Feed output, appending to `out`, which ends with the unfinished line
    /// as it stands.
    pub(crate) fn push(&mut self, bytes: &[u8], out: &mut String) {
        let mut text = std::mem::take(&mut self.pending);
        text.reserve(bytes.len());
        for &byte in bytes {
            self.state = match self.state {
                StripState::Normal => match byte {
                    0x1b => {
                        self.write(&mut text, out);
                        StripState::Escape
                    }
                    b'\n' => {
                        self.write(&mut text, out);
                        self.end_line(out, true);
                        StripState::Normal
                    }
                    b'\r' => {
                        self.write(&mut text, out);
                        self.column = 0;
                        StripState::Normal
                    }
                    0x08 => {
                        self.write(&mut text, out);
                        self.column = self.column.saturating_sub(1);
                        StripState::Normal
                    }
                    b'\t' => {
                        text.push(byte);
                        StripState::Normal
                    }
                    0x00..=0x1f | 0x7f => StripState::Normal,
                    _ => {
                        text.push(byte);
                        StripState::Normal
                    }
                },
                StripState::Escape => self.escape(byte),
                StripState::Csi => match byte {
                    b'0'..=b'9' => {
                        self.csi_param = self
                            .csi_param
                            .saturating_mul(10)
                            .saturating_add(u32::from(byte - b'0'));
                        StripState::Csi
                    }
                    0x40..=0x7e => {
                        if byte == b'K' && !self.csi_other {
                            self.erase_in_line();
                        }
                        StripState::Normal
                    }
                    0x18 | 0x1a => StripState::Normal,
                    0x1b => StripState::Escape,
                    _ => {
                        self.csi_other = true;
                        StripState::Csi
                    }
                },
                StripState::Osc => match byte {
                    0x07 | 0x18 | 0x1a => StripState::Normal,
                    0x1b => StripState::OscEscape,
                    _ => StripState::Osc,
                },
                // The ESC that ends a string also starts the next sequence.
                StripState::OscEscape => self.escape(byte),
                StripState::SkipOne => StripState::Normal,
            };
        }
        let keep = incomplete_utf8_tail(&text);
        self.pending = text.split_off(text.len() - keep);
        self.write(&mut text, out);
        self.show(out);
    }

    /// Bring the end of `out` up to date with `line`.
    fn show(&mut self, out: &mut String) {
        let current = self.stale_from.min(self.shown).min(self.line.len());
        if current < self.shown {
            let bytes: usize = self.line[..current].iter().map(|c| c.len_utf8()).sum();
            out.truncate(self.finished_len + bytes);
        }
        out.extend(&self.line[current..]);
        self.shown = self.line.len();
        self.stale_from = self.shown;
    }

    /// Finish the line (with `\n`, or without when it's only too long).
    fn end_line(&mut self, out: &mut String, newline: bool) {
        self.show(out);
        if newline {
            out.push('\n');
        }
        self.finished_len = out.len();
        self.line.clear();
        self.column = 0;
        self.shown = 0;
        self.stale_from = 0;
    }

    /// The state after ESC and `byte`.
    fn escape(&mut self, byte: u8) -> StripState {
        match byte {
            b'[' => {
                self.csi_param = 0;
                self.csi_other = false;
                StripState::Csi
            }
            b']' | b'P' | b'X' | b'^' | b'_' => StripState::Osc,
            b'(' | b')' | b'*' | b'+' | b'-' | b'.' | b'/' => StripState::SkipOne,
            0x1b => StripState::Escape,
            _ => StripState::Normal,
        }
    }

    /// Write `text` (UTF-8) at the cursor, over what's there.
    fn write(&mut self, text: &mut Vec<u8>, out: &mut String) {
        if text.is_empty() {
            return;
        }
        for c in String::from_utf8_lossy(text).chars() {
            // Image placeholders and their marks aren't text.
            if c == crate::graphics::PLACEHOLDER {
                self.after_placeholder = true;
                continue;
            }
            if self.after_placeholder && crate::graphics::is_diacritic(c) {
                continue;
            }
            self.after_placeholder = false;
            if self.column >= MAX_LINE_CHARS {
                // Kept, but out of reach of `\r` from here on.
                self.end_line(out, false);
            }
            if self.column < self.line.len() {
                self.line[self.column] = c;
                self.stale_from = self.stale_from.min(self.column);
            } else {
                self.line.resize(self.column, ' ');
                self.line.push(c);
            }
            self.column += 1;
        }
        text.clear();
    }

    /// CSI K: 0 clears from the cursor to the end, 1 from the start through
    /// the cursor, 2 the whole line. The cursor stays.
    fn erase_in_line(&mut self) {
        let first_changed = match self.csi_param {
            0 => {
                self.line.truncate(self.column);
                self.column
            }
            1 => {
                let end = (self.column + 1).min(self.line.len());
                self.line[..end].fill(' ');
                0
            }
            2 => {
                self.line.clear();
                0
            }
            _ => return,
        };
        self.stale_from = self.stale_from.min(first_changed);
    }
}

/// Length of a UTF-8 character cut off at the end of `bytes` (0 if none).
fn incomplete_utf8_tail(bytes: &[u8]) -> usize {
    for back in 1..=bytes.len().min(3) {
        let byte = bytes[bytes.len() - back];
        if byte & 0xC0 == 0x80 {
            continue; // a continuation byte: keep looking for the lead
        }
        let needed = match byte {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => return 0,
        };
        return if back < needed { back } else { 0 };
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_survives_reads_that_split_sequences_and_characters() {
        let mut stripper = PlainTextStripper::default();
        let mut text = String::new();
        // An SGR sequence and "日本" (3 bytes each) cut across four reads.
        for chunk in [
            &b"ok \x1b[38;5;2"[..],
            b"08mfoo \xE6\x97",
            b"\xA5\xE6",
            b"\x9C\xAC\n",
        ] {
            stripper.push(chunk, &mut text);
        }
        assert_eq!(text, "ok foo 日本\n");
    }

    #[test]
    fn plain_text_leaves_out_images() {
        let mut stripper = PlainTextStripper::default();
        let mut text = String::new();
        // A graphics command, then two placeholder cells with marks (one
        // of them, U+0305, a mark that's also ordinary text after "x").
        stripper.push(b"a\x1b_Ga=T;AAAA\x1b\\b ", &mut text);
        stripper.push(
            "\u{10EEEE}\u{0305}\u{0305}\u{10EEEE}x\u{0305}\n".as_bytes(),
            &mut text,
        );
        assert_eq!(text, "ab x\u{0305}\n");
    }

    #[test]
    fn carriage_returns_and_backspaces_overwrite_the_line() {
        let mut tracker = CommandBlockTracker::new();
        tracker.command_started_at(10, None);
        // A progress bar redrawn in place, across reads.
        tracker.observe_output(b"10%\r20%");
        assert_eq!(tracker.blocks()[0].output, "20%");
        tracker.observe_output(b"\r100%\ndone");
        assert_eq!(tracker.blocks()[0].output, "100%\ndone");
        // Backspace moves back one; what follows overwrites.
        tracker.observe_output(b"x\x08!\r\n");
        tracker.observe_output(b"abc\x08\x08X\n");
        assert_eq!(tracker.blocks()[0].output, "100%\ndone!\naXc\n");
    }

    #[test]
    fn erase_in_line_clears_what_a_shorter_redraw_leaves() {
        let mut tracker = CommandBlockTracker::new();
        tracker.command_started_at(10, None);
        tracker.observe_output(b"Compiling something long\r\x1b[2K");
        tracker.observe_output(b"Building 3/9\r\x1b[K Done\n");
        tracker.observe_output(b"abcdef\x08\x08\x08\x1b[1Kx\x1b[0K\n");
        // Other CSIs (colors, private modes) leave the line alone.
        tracker.observe_output(b"\x1b[?25lred\x1b[31;1m!\x1b[0m\x1b[?25h\n");
        let block = tracker.command_ended_at(0, 20, None).unwrap();
        assert_eq!(block.output, " Done\n   x\nred!\n");
    }

    #[test]
    fn output_reads_the_same_however_it_is_split() {
        let mut stream = Vec::new();
        for i in 0..50 {
            stream.extend_from_slice(format!("step {i}/50 é\r").as_bytes());
            if i % 7 == 0 {
                stream.extend_from_slice(b"\x1b[2Kdone \xE6\x97\xA5\x08\x08x\x1b[K\n");
            }
            if i % 11 == 0 {
                stream.extend_from_slice(b"\x1b[31mlong\x1b[0m line\x08\x08\x08\x1b[1K!\r");
            }
            if i % 5 == 0 {
                stream.extend_from_slice(b"\n");
            }
        }
        stream.extend_from_slice("tail é\x08!".as_bytes());
        let read = |size: usize| {
            let mut tracker = CommandBlockTracker::new();
            tracker.command_started_at(10, None);
            for chunk in stream.chunks(size) {
                tracker.observe_output(chunk);
            }
            tracker.blocks().pop().unwrap().output
        };
        let whole = read(stream.len());
        assert!(
            whole.starts_with("donex\n      !ne\nstep 5/50 é\n"),
            "{whole:?}"
        );
        assert!(
            whole.ends_with("\nstep 45/50 é\ndonex\ntail !"),
            "{whole:?}"
        );
        for size in [1, 2, 3, 5, 64] {
            assert_eq!(read(size), whole, "reads of {size}");
        }
    }

    #[test]
    fn a_very_long_line_is_kept_whole() {
        let mut tracker = CommandBlockTracker::new();
        tracker.command_started_at(10, None);
        let long = "a".repeat(40_000);
        for chunk in long.as_bytes().chunks(4096) {
            tracker.observe_output(chunk);
        }
        tracker.observe_output(b"\n");
        assert_eq!(tracker.blocks()[0].output, format!("{long}\n"));

        // `\r` only goes back over the last stretch of it, as over the last
        // row on screen.
        tracker.observe_output("b".repeat(20_000).as_bytes());
        tracker.observe_output(b"\rc\n");
        let tail = format!(
            "{}c{}\n",
            "b".repeat(MAX_LINE_CHARS),
            "b".repeat(20_000 - MAX_LINE_CHARS - 1)
        );
        assert_eq!(tracker.blocks()[0].output, format!("{long}\n{tail}"));
    }

    #[test]
    fn the_oldest_blocks_go_past_the_limit() {
        let mut tracker = CommandBlockTracker::new();
        for i in 0..(MAX_COMMAND_BLOCKS as u64 + 5) {
            tracker.command_started_at(i, None);
            tracker.observe_output(b"x\n");
            tracker.command_ended_at(0, i, None);
        }
        let summaries = tracker.summaries();
        assert_eq!(summaries.len(), MAX_COMMAND_BLOCKS);
        assert_eq!(summaries[0].id, TerminalBlockId(6));
    }

    #[test]
    fn summaries_leave_output_out() {
        let mut tracker = CommandBlockTracker::new();
        tracker.set_pending_command("ls".into());
        let block = tracker.command_started_at(10, Some(1));
        tracker.observe_output(b"a.txt\n");
        tracker.command_ended_at(0, 20, Some(3));
        let summaries = tracker.summaries();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].command.as_deref(), Some("ls"));
        assert!(summaries[0].has_output);
        let full = tracker.block(block.id).expect("block");
        assert_eq!(full.output, "a.txt\n");
        assert!(tracker.block(TerminalBlockId(99)).is_none());
    }

    #[test]
    fn version_tracks_boundary_changes_but_not_output() {
        let mut tracker = CommandBlockTracker::new();
        let start = tracker.version();
        tracker.observe_output(b"hello\n");
        assert_eq!(tracker.version(), start, "plain output doesn't move blocks");
        tracker.prompt_marked(0);
        let marked = tracker.version();
        assert_ne!(marked, start);
        tracker.command_started_at(10, Some(1));
        let started = tracker.version();
        assert_ne!(started, marked);
        tracker.command_ended_at(0, 20, Some(3));
        assert_ne!(tracker.version(), started);
        let ended = tracker.version();
        assert!(tracker.command_ended_at(0, 30, Some(4)).is_none());
        assert_eq!(tracker.version(), ended, "nothing running, nothing changed");
    }

    #[test]
    fn absorbs_empty_prompt_stub_into_the_real_command_block() {
        // Shells repaint the prompt, emitting a bare OSC 133;C with no command
        // before the real one. The stub must not orphan the prompt row.
        let mut tracker = CommandBlockTracker::new();
        tracker.prompt_marked(0);

        // Phantom command-start: no pending command, consumes the prompt row.
        let stub = tracker.command_started_at(10, Some(2));
        assert_eq!(stub.command, None);
        assert_eq!(stub.prompt_row, Some(0));

        // Real command-start (after the OSC 6973 command text arrived).
        tracker.set_pending_command("echo hello".to_string());
        let real = tracker.command_started_at(20, Some(2));

        // One block, spanning the prompt through the output.
        let blocks = tracker.blocks();
        assert_eq!(blocks.len(), 1);
        assert_eq!(real.command.as_deref(), Some("echo hello"));
        assert_eq!(
            real.prompt_row,
            Some(0),
            "prompt row absorbed from the stub"
        );
        assert_eq!(real.output_row, Some(2));
    }

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
