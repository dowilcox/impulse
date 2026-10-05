//! Minimal OSC byte-stream scanner.
//!
//! Watches a byte stream for OSC escape sequences and emits events.
//! Does NOT modify or buffer the byte stream — all bytes pass through
//! unchanged. alacritty_terminal ignores unsupported OSCs harmlessly.

/// Progress state reported via ConEmu-style OSC 9;4 (also used by Ghostty,
/// kitty, Windows Terminal and agent CLIs such as Claude Code).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum ProgressState {
    /// `9;4;0` — clear any progress indicator.
    Hidden,
    /// `9;4;1;pct` — normal progress.
    Normal,
    /// `9;4;2[;pct]` — error state.
    Error,
    /// `9;4;3` — indeterminate (busy, no percentage).
    Indeterminate,
    /// `9;4;4[;pct]` — paused / warning state.
    Paused,
}

/// Events emitted by the OSC scanner.
#[derive(Clone, Debug, PartialEq)]
pub enum OscEvent {
    /// OSC 7: working directory changed.
    CwdChanged(String),
    /// OSC 133;A: shell prompt started.
    PromptStart,
    /// OSC 133;C: command execution started.
    CommandStart,
    /// OSC 133;D;{code}: command execution ended with exit code.
    CommandEnd(i32),
    /// OSC 6973;Command={percent-encoded command}: Impulse command text.
    CommandText(String),
    /// iTerm2 OSC 1337;RequestAttention={yes|once|no}.
    AttentionRequest(String),
    /// OSC 9 / OSC 777 notification request.
    Notification { title: String, body: String },
    /// OSC 9;4 progress report. `percent` is clamped to 0...100.
    Progress {
        state: ProgressState,
        percent: Option<u8>,
    },
}

/// OSC event with byte offsets in the most recently scanned chunk.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OscEventSpan {
    pub(crate) event: OscEvent,
    pub(crate) start_offset: Option<usize>,
    pub(crate) end_offset: usize,
}

/// Scanner state machine.
#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Normal,
    Escape,    // Saw ESC (0x1B), expecting ']' for OSC
    OscBody,   // Inside OSC payload, collecting until BEL or ST
    OscEscape, // Saw ESC inside an OSC payload, expecting '\' for ST
}

/// Maximum OSC payload size before we reset (prevents unbounded growth).
const MAX_OSC_LEN: usize = 4096;

/// Scans a byte stream for OSC sequences used by Impulse.
pub struct OscScanner {
    state: State,
    buf: Vec<u8>,
    events: Vec<OscEvent>,
    event_spans: Vec<OscEventSpan>,
    escape_start_offset: Option<usize>,
    osc_start_offset: Option<usize>,
    /// OSC 99 notifications still being assembled, by id.
    kitty_pending: Vec<(String, KittyNotification)>,
}

/// A kitty (OSC 99) notification arriving in chunks.
#[derive(Default)]
struct KittyNotification {
    title: String,
    body: String,
}

impl OscScanner {
    pub fn new() -> Self {
        Self {
            state: State::Normal,
            buf: Vec::with_capacity(256),
            events: Vec::new(),
            event_spans: Vec::new(),
            escape_start_offset: None,
            osc_start_offset: None,
            kitty_pending: Vec::new(),
        }
    }

    /// Scan a chunk of bytes. Call `drain_events()` after to collect results.
    pub fn scan(&mut self, bytes: &[u8]) {
        if self.state != State::Normal {
            self.escape_start_offset = None;
            self.osc_start_offset = None;
        }

        for (offset, &b) in bytes.iter().enumerate() {
            match self.state {
                State::Normal => {
                    if b == 0x1B {
                        self.state = State::Escape;
                        self.escape_start_offset = Some(offset);
                    }
                }
                State::Escape => {
                    if b == b']' {
                        self.state = State::OscBody;
                        self.buf.clear();
                        self.osc_start_offset = self.escape_start_offset.take();
                    } else {
                        // Not an OSC sequence, back to normal.
                        self.state = State::Normal;
                        self.escape_start_offset = None;
                    }
                }
                State::OscBody => {
                    if b == 0x07 {
                        // BEL terminates the OSC sequence.
                        self.dispatch_osc(offset + 1);
                        self.state = State::Normal;
                    } else if b == 0x1B {
                        self.state = State::OscEscape;
                    } else if self.buf.len() < MAX_OSC_LEN {
                        self.buf.push(b);
                    } else {
                        // Overflow, reset.
                        self.buf.clear();
                        self.state = State::Normal;
                        self.osc_start_offset = None;
                    }
                }
                State::OscEscape => {
                    if b == b'\\' {
                        self.dispatch_osc(offset + 1);
                    } else {
                        self.dispatch_osc(offset);
                    }
                    self.state = State::Normal;
                }
            }
        }
    }

    /// Drain all events emitted since the last drain.
    pub fn drain_events(&mut self) -> Vec<OscEvent> {
        self.event_spans.clear();
        std::mem::take(&mut self.events)
    }

    pub(crate) fn drain_event_spans(&mut self) -> Vec<OscEventSpan> {
        self.events.clear();
        std::mem::take(&mut self.event_spans)
    }

    fn dispatch_osc(&mut self, end_offset: usize) {
        let event = if self.buf.starts_with(b"99;") {
            let payload = self.buf[3..].to_vec();
            self.parse_kitty_notification(&payload)
        } else {
            self.parse_osc()
        };
        if let Some(event) = event {
            self.events.push(event.clone());
            self.event_spans.push(OscEventSpan {
                event,
                start_offset: self.osc_start_offset,
                end_offset,
            });
        }
        self.osc_start_offset = None;
        self.escape_start_offset = None;
        self.buf.clear();
    }

    fn parse_osc(&self) -> Option<OscEvent> {
        if self.buf.is_empty() {
            return None;
        }

        // Check for OSC 7 (CWD): "7;file://..."
        if self.buf.starts_with(b"7;") {
            return Self::parse_osc7(&self.buf[2..]).map(OscEvent::CwdChanged);
        }

        // Check for OSC 133 (shell integration): "133;X" or "133;D;code"
        if self.buf.starts_with(b"133;") && self.buf.len() >= 5 {
            return match self.buf[4] {
                b'A' => Some(OscEvent::PromptStart),
                b'B' => None, // Prompt end. Not currently exposed to frontends.
                b'C' => Some(OscEvent::CommandStart),
                b'D' => {
                    let code = if self.buf.len() > 6 && self.buf[5] == b';' {
                        std::str::from_utf8(&self.buf[6..])
                            .ok()
                            .and_then(|s| s.parse::<i32>().ok())
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    Some(OscEvent::CommandEnd(code))
                }
                _ => None,
            };
        }

        if self.buf.starts_with(b"6973;Command=") {
            return Self::parse_impulse_command(&self.buf[13..]).map(OscEvent::CommandText);
        }

        if self.buf.starts_with(b"1337;") {
            return Self::parse_iterm2_attention(&self.buf[5..]).map(OscEvent::AttentionRequest);
        }

        // OSC 9;4 is the ConEmu progress protocol, not a notification. It must
        // be matched before the generic OSC 9 notification below.
        if self.buf == b"9;4" || self.buf.starts_with(b"9;4;") {
            return Self::parse_progress(&self.buf[3..]);
        }

        if self.buf.starts_with(b"9;") {
            if let Ok(body) = std::str::from_utf8(&self.buf[2..]) {
                return Some(OscEvent::Notification {
                    title: "Terminal".to_string(),
                    body: Self::sanitize_text(body),
                });
            }
        }

        if self.buf.starts_with(b"777;") {
            return Self::parse_rxvt_notify(&self.buf[4..])
                .map(|(title, body)| OscEvent::Notification { title, body });
        }

        None
    }

    /// Parse iTerm2 attention payload after "1337;".
    fn parse_iterm2_attention(payload: &[u8]) -> Option<String> {
        let s = std::str::from_utf8(payload).ok()?;
        let value = s.strip_prefix("RequestAttention=")?;
        match value {
            "yes" | "once" | "no" => Some(value.to_string()),
            _ => None,
        }
    }

    /// Parse an OSC 9;4 payload after "9;4": `;state[;percent]` or empty.
    fn parse_progress(payload: &[u8]) -> Option<OscEvent> {
        let s = std::str::from_utf8(payload).ok()?;
        let mut parts = s.trim_start_matches(';').split(';');
        let state = match parts.next().unwrap_or("0") {
            "" | "0" => ProgressState::Hidden,
            "1" => ProgressState::Normal,
            "2" => ProgressState::Error,
            "3" => ProgressState::Indeterminate,
            "4" => ProgressState::Paused,
            _ => return None,
        };
        let percent = parts
            .next()
            .and_then(|p| p.trim().parse::<u32>().ok())
            .map(|p| p.min(100) as u8);
        let percent = match state {
            ProgressState::Hidden | ProgressState::Indeterminate => None,
            _ => percent,
        };
        Some(OscEvent::Progress { state, percent })
    }

    /// Parse a kitty desktop notification after "99;": `metadata;payload`,
    /// where metadata is `key=value` pairs joined by ':' — `i` (id), `d`
    /// (0 while more chunks follow), `p` (`title` or `body`). Chunks with the
    /// same id accumulate until `d` isn't 0. Base64 (`e=1`) and other payload
    /// kinds are ignored.
    fn parse_kitty_notification(&mut self, payload: &[u8]) -> Option<OscEvent> {
        let s = std::str::from_utf8(payload).ok()?;
        let (metadata, text) = s.split_once(';')?;
        let mut id = String::new();
        let mut done = true;
        let mut kind = "title";
        for pair in metadata.split(':').filter(|p| !p.is_empty()) {
            let (key, value) = pair.split_once('=')?;
            match key {
                "i" => id = value.to_string(),
                "d" => done = value != "0",
                "p" => {
                    kind = if value == "body" {
                        "body"
                    } else if value == "title" {
                        "title"
                    } else {
                        return None;
                    }
                }
                "e" if value == "1" => return None,
                _ => {}
            }
        }
        let index = match self
            .kitty_pending
            .iter()
            .position(|(pending, _)| *pending == id)
        {
            Some(index) => index,
            None => {
                // Bound memory if a program never finishes its chunks.
                if self.kitty_pending.len() >= 8 {
                    self.kitty_pending.remove(0);
                }
                self.kitty_pending
                    .push((id.clone(), KittyNotification::default()));
                self.kitty_pending.len() - 1
            }
        };
        let text = Self::sanitize_text(text);
        let entry = &mut self.kitty_pending[index].1;
        if kind == "body" {
            entry.body.push_str(&text);
        } else {
            entry.title.push_str(&text);
        }
        if !done {
            return None;
        }
        let (_, finished) = self.kitty_pending.remove(index);
        if finished.title.is_empty() && finished.body.is_empty() {
            return None;
        }
        Some(OscEvent::Notification {
            title: if finished.title.is_empty() {
                "Terminal".to_string()
            } else {
                finished.title
            },
            body: finished.body,
        })
    }

    /// Parse rxvt/WezTerm notification payload after "777;".
    fn parse_rxvt_notify(payload: &[u8]) -> Option<(String, String)> {
        let s = std::str::from_utf8(payload).ok()?;
        let mut parts = s.splitn(3, ';');
        if parts.next()? != "notify" {
            return None;
        }
        let title = Self::sanitize_text(parts.next().unwrap_or("Terminal"));
        let body = Self::sanitize_text(parts.next().unwrap_or(""));
        Some((title, body))
    }

    fn sanitize_text(input: &str) -> String {
        input.chars().filter(|&c| c != '\0').collect()
    }

    fn parse_impulse_command(payload: &[u8]) -> Option<String> {
        let encoded = std::str::from_utf8(payload).ok()?;
        let decoded = Self::url_decode(encoded)?;
        Some(Self::sanitize_text(&decoded))
    }

    /// Parse OSC 7 payload: "file://hostname/path" → URL-decoded path.
    fn parse_osc7(payload: &[u8]) -> Option<String> {
        let s = std::str::from_utf8(payload).ok()?;

        // Strip "file://" prefix.
        let rest = s.strip_prefix("file://")?;

        // Skip hostname (everything up to the first '/').
        let path_start = rest.find('/')?;
        let encoded_path = &rest[path_start..];

        // URL-decode the path.
        let decoded = Self::url_decode(encoded_path)?;

        // Validate: must be an existing absolute directory and safe to pass
        // across FFI boundaries.
        if decoded.starts_with('/')
            && !decoded.contains('\0')
            && std::path::Path::new(&decoded).is_dir()
        {
            Some(decoded)
        } else {
            None
        }
    }

    /// Decode percent-encoded UTF-8 string.
    fn url_decode(input: &str) -> Option<String> {
        let mut bytes = Vec::with_capacity(input.len());
        let mut chars = input.bytes();
        while let Some(b) = chars.next() {
            if b == b'%' {
                let hi = chars.next()?;
                let lo = chars.next()?;
                let hex = [hi, lo];
                let val = u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?;
                bytes.push(val);
            } else {
                bytes.push(b);
            }
        }
        String::from_utf8(bytes).ok()
    }
}

impl Default for OscScanner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_osc7_cwd() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().to_string_lossy();
        let mut scanner = OscScanner::new();
        let seq = format!("\x1b]7;file://myhost{cwd}\x07");
        scanner.scan(seq.as_bytes());
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::CwdChanged(cwd.to_string())]);
    }

    #[test]
    fn test_osc7_url_encoded() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("my dir").join("foo").join("bar");
        std::fs::create_dir_all(&cwd).unwrap();
        let encoded = cwd.to_string_lossy().replace(' ', "%20");
        let mut scanner = OscScanner::new();
        let seq = format!("\x1b]7;file://host{encoded}\x07");
        scanner.scan(seq.as_bytes());
        let events = scanner.drain_events();
        assert_eq!(
            events,
            vec![OscEvent::CwdChanged(cwd.to_string_lossy().to_string())]
        );
    }

    #[test]
    fn test_osc7_ignores_nonexistent_directory() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing");
        let mut scanner = OscScanner::new();
        let seq = format!("\x1b]7;file://host{}\x07", missing.to_string_lossy());
        scanner.scan(seq.as_bytes());

        assert!(scanner.drain_events().is_empty());
    }

    #[test]
    fn test_osc7_ignores_nul_path() {
        let temp = tempfile::tempdir().unwrap();
        let mut scanner = OscScanner::new();
        let seq = format!(
            "\x1b]7;file://host{}%00suffix\x07",
            temp.path().to_string_lossy()
        );
        scanner.scan(seq.as_bytes());

        assert!(scanner.drain_events().is_empty());
    }

    #[test]
    fn test_osc133_prompt_start() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]133;A\x07");
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::PromptStart]);
    }

    #[test]
    fn test_osc133_command_start() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]133;C\x07");
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::CommandStart]);
    }

    #[test]
    fn test_impulse_command_text() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]6973;Command=cargo%20test%20-p%20impulse-terminal\x07");
        let events = scanner.drain_events();
        assert_eq!(
            events,
            vec![OscEvent::CommandText(
                "cargo test -p impulse-terminal".to_string()
            )]
        );
    }

    #[test]
    fn test_impulse_command_text_strips_nul() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]6973;Command=echo%00hi\x07");
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::CommandText("echohi".to_string())]);
    }

    #[test]
    fn test_osc133_command_end_with_code() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]133;D;0\x07");
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::CommandEnd(0)]);
    }

    #[test]
    fn test_osc133_command_end_nonzero() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]133;D;127\x07");
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::CommandEnd(127)]);
    }

    #[test]
    fn test_osc133_prompt_end_ignored() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]133;B\x07");
        let events = scanner.drain_events();
        assert!(events.is_empty());
    }

    #[test]
    fn test_mixed_bytes_and_osc() {
        let mut scanner = OscScanner::new();
        let data = b"hello\x1b]133;A\x07world\x1b]7;file://h/tmp\x07";
        scanner.scan(data);
        let events = scanner.drain_events();
        assert_eq!(
            events,
            vec![
                OscEvent::PromptStart,
                OscEvent::CwdChanged("/tmp".to_string()),
            ]
        );
    }

    #[test]
    fn test_event_spans_include_offsets() {
        let mut scanner = OscScanner::new();
        let data = b"out\x1b]133;D;0\x07prompt";
        scanner.scan(data);
        let spans = scanner.drain_event_spans();

        assert_eq!(
            spans,
            vec![OscEventSpan {
                event: OscEvent::CommandEnd(0),
                start_offset: Some(3),
                end_offset: 13,
            }]
        );
    }

    #[test]
    fn test_split_event_span_has_no_current_chunk_start() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]133;");
        scanner.scan(b"C\x07prompt");
        let spans = scanner.drain_event_spans();

        assert_eq!(
            spans,
            vec![OscEventSpan {
                event: OscEvent::CommandStart,
                start_offset: None,
                end_offset: 2,
            }]
        );
    }

    #[test]
    fn test_st_terminator() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]133;A\x1b\\");
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::PromptStart]);
    }

    #[test]
    fn test_split_across_chunks() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]133");
        assert!(scanner.drain_events().is_empty());
        scanner.scan(b";C\x07");
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::CommandStart]);
    }

    #[test]
    fn test_iterm2_attention_once() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]1337;RequestAttention=once\x07");
        assert_eq!(
            scanner.drain_events(),
            vec![OscEvent::AttentionRequest("once".to_string())]
        );
    }

    #[test]
    fn test_iterm2_attention_ignores_unknown_value() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]1337;RequestAttention=fireworks\x07");
        assert!(scanner.drain_events().is_empty());
    }

    #[test]
    fn test_osc9_notification() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]9;hello there\x07");
        assert_eq!(
            scanner.drain_events(),
            vec![OscEvent::Notification {
                title: "Terminal".to_string(),
                body: "hello there".to_string(),
            }]
        );
    }

    #[test]
    fn test_osc9_4_progress_is_not_a_notification() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]9;4;1;50\x07");
        scanner.scan(b"\x1b]9;4;3\x1b\\");
        scanner.scan(b"\x1b]9;4;0\x07");
        scanner.scan(b"\x1b]9;4;2;250\x07");
        scanner.scan(b"\x1b]9;4;4\x07");
        scanner.scan(b"\x1b]9;4\x07");
        let events = scanner.drain_events();
        assert_eq!(
            events,
            vec![
                OscEvent::Progress {
                    state: ProgressState::Normal,
                    percent: Some(50)
                },
                OscEvent::Progress {
                    state: ProgressState::Indeterminate,
                    percent: None
                },
                OscEvent::Progress {
                    state: ProgressState::Hidden,
                    percent: None
                },
                OscEvent::Progress {
                    state: ProgressState::Error,
                    percent: Some(100)
                },
                OscEvent::Progress {
                    state: ProgressState::Paused,
                    percent: None
                },
                OscEvent::Progress {
                    state: ProgressState::Hidden,
                    percent: None
                },
            ]
        );
    }

    #[test]
    fn test_osc9_4_unknown_state_is_ignored_and_9_40_is_text() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]9;4;9;10\x07");
        scanner.scan(b"\x1b]9;40 tests passed\x07");
        let events = scanner.drain_events();
        assert_eq!(
            events,
            vec![OscEvent::Notification {
                title: "Terminal".to_string(),
                body: "40 tests passed".to_string()
            }]
        );
    }

    #[test]
    fn test_osc99_simple_notification_is_a_title() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]99;;Hello world\x1b\\");
        assert_eq!(
            scanner.drain_events(),
            vec![OscEvent::Notification {
                title: "Hello world".to_string(),
                body: String::new(),
            }]
        );
    }

    #[test]
    fn test_osc99_chunks_assemble_by_id() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]99;i=1:d=0;Build\x1b\\");
        scanner.scan(b"\x1b]99;i=2:d=0;Other\x07");
        assert!(scanner.drain_events().is_empty());
        scanner.scan(b"\x1b]99;i=1:d=0:p=body;All tests\x1b\\");
        scanner.scan(b"\x1b]99;i=1:p=body; passed\x1b\\");
        assert_eq!(
            scanner.drain_events(),
            vec![OscEvent::Notification {
                title: "Build".to_string(),
                body: "All tests passed".to_string(),
            }]
        );
    }

    #[test]
    fn test_osc99_ignores_base64_and_unknown_payloads() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]99;e=1;SGVsbG8=\x07");
        scanner.scan(b"\x1b]99;p=icon;x\x07");
        scanner.scan(b"\x1b]99;garbage\x07");
        assert!(scanner.drain_events().is_empty());
    }

    #[test]
    fn test_osc777_notification_preserves_body_semicolons() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]777;notify;Build;done;with warnings\x07");
        assert_eq!(
            scanner.drain_events(),
            vec![OscEvent::Notification {
                title: "Build".to_string(),
                body: "done;with warnings".to_string(),
            }]
        );
    }

    #[test]
    fn test_overflow_resets() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]");
        let overflow = vec![b'x'; MAX_OSC_LEN + 1];
        scanner.scan(&overflow);
        assert!(scanner.drain_events().is_empty());
        scanner.scan(b"\x1b]133;A\x07");
        assert_eq!(scanner.drain_events(), vec![OscEvent::PromptStart]);
    }
}
