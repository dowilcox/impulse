//! Minimal OSC byte-stream scanner.
//!
//! Watches a byte stream for OSC escape sequences and emits events.
//! Does NOT modify or buffer the byte stream — all bytes pass through
//! unchanged. alacritty_terminal ignores unsupported OSCs harmlessly.

/// Events emitted by the OSC scanner.
#[derive(Clone, Debug, PartialEq)]
pub enum OscEvent {
    /// OSC 7: working directory changed.
    CwdChanged(String),
    /// OSC 133;A: shell prompt started.
    PromptStart,
    /// OSC 133;B: command execution started.
    CommandStart,
    /// OSC 133;D;{code}: command execution ended with exit code.
    CommandEnd(i32),
    /// iTerm2 OSC 1337;RequestAttention={yes|once|no}.
    AttentionRequest(String),
    /// OSC 9 / OSC 777 notification request.
    Notification { title: String, body: String },
}

/// Scanner state machine.
#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Normal,
    Escape,  // Saw ESC (0x1B), expecting ']' for OSC
    OscBody, // Inside OSC payload, collecting until BEL or ST
}

/// Maximum OSC payload size before we reset (prevents unbounded growth).
const MAX_OSC_LEN: usize = 4096;

/// Scans a byte stream for OSC sequences used by Impulse.
pub struct OscScanner {
    state: State,
    buf: Vec<u8>,
    events: Vec<OscEvent>,
}

impl OscScanner {
    pub fn new() -> Self {
        Self {
            state: State::Normal,
            buf: Vec::with_capacity(256),
            events: Vec::new(),
        }
    }

    /// Scan a chunk of bytes. Call `drain_events()` after to collect results.
    pub fn scan(&mut self, bytes: &[u8]) {
        for &b in bytes {
            match self.state {
                State::Normal => {
                    if b == 0x1B {
                        self.state = State::Escape;
                    }
                }
                State::Escape => {
                    if b == b']' {
                        self.state = State::OscBody;
                        self.buf.clear();
                    } else {
                        // Not an OSC sequence, back to normal.
                        self.state = State::Normal;
                    }
                }
                State::OscBody => {
                    if b == 0x07 {
                        // BEL terminates the OSC sequence.
                        self.dispatch_osc();
                        self.state = State::Normal;
                    } else if b == 0x1B {
                        // Could be ST (ESC \). Dispatch what we have and
                        // go to Escape state to handle the next byte.
                        self.dispatch_osc();
                        self.state = State::Escape;
                    } else if self.buf.len() < MAX_OSC_LEN {
                        self.buf.push(b);
                    } else {
                        // Overflow, reset.
                        self.buf.clear();
                        self.state = State::Normal;
                    }
                }
            }
        }
    }

    /// Drain all events emitted since the last drain.
    pub fn drain_events(&mut self) -> Vec<OscEvent> {
        std::mem::take(&mut self.events)
    }

    fn dispatch_osc(&mut self) {
        if self.buf.is_empty() {
            return;
        }

        // Check for OSC 7 (CWD): "7;file://..."
        if self.buf.starts_with(b"7;") {
            if let Some(path) = Self::parse_osc7(&self.buf[2..]) {
                self.events.push(OscEvent::CwdChanged(path));
            }
        }
        // Check for OSC 133 (shell integration): "133;X" or "133;D;code"
        else if self.buf.starts_with(b"133;") && self.buf.len() >= 5 {
            match self.buf[4] {
                b'A' => self.events.push(OscEvent::PromptStart),
                b'B' => self.events.push(OscEvent::CommandStart),
                b'C' => {} // Ignored (redundant with B)
                b'D' => {
                    let code = if self.buf.len() > 6 && self.buf[5] == b';' {
                        std::str::from_utf8(&self.buf[6..])
                            .ok()
                            .and_then(|s| s.parse::<i32>().ok())
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    self.events.push(OscEvent::CommandEnd(code));
                }
                _ => {}
            }
        } else if self.buf.starts_with(b"1337;") {
            if let Some(value) = Self::parse_iterm2_attention(&self.buf[5..]) {
                self.events.push(OscEvent::AttentionRequest(value));
            }
        } else if self.buf.starts_with(b"9;") {
            if let Ok(body) = std::str::from_utf8(&self.buf[2..]) {
                self.events.push(OscEvent::Notification {
                    title: "Terminal".to_string(),
                    body: Self::sanitize_text(body),
                });
            }
        } else if self.buf.starts_with(b"777;") {
            if let Some((title, body)) = Self::parse_rxvt_notify(&self.buf[4..]) {
                self.events.push(OscEvent::Notification { title, body });
            }
        }

        self.buf.clear();
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

        // Validate: must be absolute and non-empty.
        if decoded.starts_with('/') && !decoded.is_empty() {
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
        let mut scanner = OscScanner::new();
        let seq = b"\x1b]7;file://myhost/Users/test\x07";
        scanner.scan(seq);
        let events = scanner.drain_events();
        assert_eq!(
            events,
            vec![OscEvent::CwdChanged("/Users/test".to_string())]
        );
    }

    #[test]
    fn test_osc7_url_encoded() {
        let mut scanner = OscScanner::new();
        let seq = b"\x1b]7;file://host/Users/my%20dir/foo%2Fbar\x07";
        scanner.scan(seq);
        let events = scanner.drain_events();
        assert_eq!(
            events,
            vec![OscEvent::CwdChanged("/Users/my dir/foo/bar".to_string())]
        );
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
        scanner.scan(b"\x1b]133;B\x07");
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::CommandStart]);
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
    fn test_osc133_c_ignored() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]133;C\x07");
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
        scanner.scan(b";B\x07");
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
