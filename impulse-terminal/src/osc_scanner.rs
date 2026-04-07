//! Minimal OSC 7/133 byte-stream scanner.
//!
//! Watches a byte stream for OSC escape sequences and emits events.
//! Does NOT modify or buffer the byte stream — all bytes pass through
//! unchanged. alacritty_terminal ignores OSC 7/133 harmlessly.

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
}

/// Scanner state machine.
#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Normal,
    Escape,   // Saw ESC (0x1B), expecting ']' for OSC
    OscBody,  // Inside OSC payload, collecting until BEL or ST
}

/// Maximum OSC payload size before we reset (prevents unbounded growth).
const MAX_OSC_LEN: usize = 4096;

/// Scans a byte stream for OSC 7 and OSC 133 sequences.
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
        }

        self.buf.clear();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_osc7_cwd() {
        let mut scanner = OscScanner::new();
        let seq = b"\x1b]7;file://myhost/Users/test\x07";
        scanner.scan(seq);
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::CwdChanged("/Users/test".to_string())]);
    }

    #[test]
    fn test_osc7_url_encoded() {
        let mut scanner = OscScanner::new();
        let seq = b"\x1b]7;file://host/Users/my%20dir/foo%2Fbar\x07";
        scanner.scan(seq);
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::CwdChanged("/Users/my dir/foo/bar".to_string())]);
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
        assert_eq!(events, vec![
            OscEvent::PromptStart,
            OscEvent::CwdChanged("/tmp".to_string()),
        ]);
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
