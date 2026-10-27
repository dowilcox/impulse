//! Minimal OSC byte-stream scanner.
//!
//! Watches a byte stream for OSC escape sequences and emits events. It
//! follows vte's state machine, so it agrees with alacritty's parser on
//! where each OSC starts and ends. All bytes pass through to the parser
//! unchanged, except an OSC over its size limit: the scanner reports it (see
//! `OscSkip`) so the caller can make the parser abandon it and drop the rest,
//! since vte buffers an OSC without bound.

/// This machine's host name. Read each time: macOS can rename the host
/// (network changes) while the app runs, and shells report the new name.
fn local_hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for its length; gethostname writes a
    // NUL-terminated name into it (or fails, leaving it zeroed).
    let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
    let len = buf.iter().position(|&b| b == 0).unwrap_or(0);
    if ok {
        String::from_utf8_lossy(&buf[..len]).into_owned()
    } else {
        String::new()
    }
}

/// Cap on an assembled kitty notification's title or body (OSC 99 chunks).
const KITTY_NOTIFICATION_MAX_BYTES: usize = 8 * 1024;

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
    /// iTerm2 OSC 21337 session status: the keys it set (`status`,
    /// `indicator`, `status-color`, `detail`); an empty value clears one.
    SessionStatus(std::collections::BTreeMap<String, String>),
    /// OSC 6973;Names={space-separated}: the shell's aliases, functions,
    /// builtins and keywords (commands that aren't files on PATH).
    ShellNames(Vec<String>),
    /// OSC 6973;Path={PATH}: the shell's search path.
    ShellPath(String),
    /// A kitty graphics command: an APC from its `G`. Reported with its
    /// offsets only (`drain_event_spans`), not by `drain_events`.
    Graphics(Vec<u8>),
    /// CSI 2 J or CSI 3 J: the screen, or the scrollback, erased.
    EraseDisplay(u16),
    /// CSI ? 1049, 1047 or 47 h or l: the alternate screen entered (true)
    /// or left.
    AltScreen(bool),
    /// ESC c: a full reset.
    FullReset,
    /// CSI 16 t: the program asks a cell's size in pixels.
    CellSizeRequest,
    /// CSI c: the program asks the terminal what it is (primary device
    /// attributes). Reported only with a skip over the sequence, so the
    /// parser doesn't answer it too.
    DeviceAttributes,
}

/// OSC event with byte offsets in the most recently scanned chunk.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OscEventSpan {
    pub(crate) event: OscEvent,
    pub(crate) start_offset: Option<usize>,
    pub(crate) end_offset: usize,
}

/// Bytes of the most recently scanned chunk that must not reach the parser:
/// the part of an OSC past its size limit, through its terminator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OscSkip {
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// The OSC went over its limit at `start` (rather than continuing from
    /// the last chunk): the parser, which has its start, must abandon it.
    pub(crate) abort: bool,
}

/// Scanner state machine (vte's, reduced to what finds OSCs).
#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Normal,
    Escape,    // Saw ESC (0x1B), expecting ']' for OSC
    OscBody,   // Inside OSC payload, collecting until BEL or ST
    OscEscape, // Saw ESC inside an OSC payload: ST if '\', else a new escape
    OscSkip,   // An OSC over its size limit, dropped until it ends
    Csi,       // Inside a CSI sequence, watched for the few images care about
    ApcStart,  // Saw ESC _: a graphics command if `G` follows
    ApcBody,   // Inside a graphics command, collecting until ST
    ApcEscape, // Saw ESC inside a graphics command: ST if '\', else a new escape
    ApcSkip,   // An APC that isn't a graphics command, or is over its limit
}

/// Longest graphics command (APC) kept; longer ones are dropped. Programs
/// send images in chunks of 4096 bytes, but needn't.
const MAX_APC_LEN: usize = 4 * 1024 * 1024;
/// Parameters kept from one CSI sequence.
const MAX_CSI_PARAMS: usize = 16;

/// Longest OSC payload, unless one of the limits below applies. Longer ones
/// are dropped whole.
const MAX_OSC_LEN: usize = 4096;
/// Impulse's own OSC 6973 payloads can be larger (the shell's command names).
const MAX_IMPULSE_OSC_LEN: usize = 64 * 1024;
/// Window titles (OSC 0/1/2). alacritty keeps up to 4096 of them on its
/// title stack (CSI 22 t), so each must stay small.
pub(crate) const MAX_TITLE_OSC_LEN: usize = 1024;
/// OSC 52 clipboard writes (base64, so about 3 MB of text).
pub(crate) const MAX_CLIPBOARD_OSC_LEN: usize = 4 * 1024 * 1024;
/// At most this many names from one OSC 6973;Names.
const MAX_SHELL_NAMES: usize = 20_000;

/// How long an OSC 7 folder's check is waited for.
const DIRECTORY_CHECK_WAIT: std::time::Duration = std::time::Duration::from_millis(500);

/// Scans a byte stream for OSC sequences used by Impulse.
pub struct OscScanner {
    state: State,
    /// The OSC's payload, kept up to what Impulse parses.
    buf: Vec<u8>,
    /// The OSC's whole length so far, as vte stores it.
    osc_len: usize,
    events: Vec<OscEvent>,
    event_spans: Vec<OscEventSpan>,
    skips: Vec<OscSkip>,
    /// Where the skip in progress began in this chunk, and whether it aborts.
    skip_start: Option<(usize, bool)>,
    escape_start_offset: Option<usize>,
    osc_start_offset: Option<usize>,
    csi_start_offset: Option<usize>,
    /// The CSI sequence so far: its parameters, the one being read, its
    /// private marker (`?`), and whether it has intermediates or anything
    /// else that makes it none of the sequences watched for.
    csi_params: [u16; MAX_CSI_PARAMS],
    csi_count: usize,
    csi_current: u16,
    csi_private: u8,
    csi_started: bool,
    csi_ignore: bool,
    /// The graphics command being collected, from its `G`.
    apc: Vec<u8>,
    apc_start_offset: Option<usize>,
    /// OSC 99 notifications still being assembled, by id.
    kitty_pending: Vec<(String, KittyNotification)>,
    /// The terminal's shell-integration secret. When set, command text
    /// (6973;Command=) must carry it: anything a program prints (`cat` of a
    /// log or a hostile file) can't put commands into history.
    nonce: Option<String>,
    /// A mark carrying the nonce has arrived: the shell integration is
    /// running, so every other mark (OSC 133, 7, 6973 Names/Path) must carry
    /// it too. Until then they're taken without it, so a shell with its own
    /// integration keeps working.
    nonce_seen: bool,
    /// An OSC 7 folder is being checked (`is_directory`) and hasn't
    /// answered yet.
    checking_directory: std::sync::Arc<std::sync::atomic::AtomicBool>,
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
            osc_len: 0,
            events: Vec::new(),
            event_spans: Vec::new(),
            skips: Vec::new(),
            skip_start: None,
            escape_start_offset: None,
            osc_start_offset: None,
            csi_start_offset: None,
            csi_params: [0; MAX_CSI_PARAMS],
            csi_count: 0,
            csi_current: 0,
            csi_private: 0,
            csi_started: false,
            csi_ignore: false,
            apc: Vec::new(),
            apc_start_offset: None,
            kitty_pending: Vec::new(),
            nonce: None,
            nonce_seen: false,
            checking_directory: Default::default(),
        }
    }

    /// A scanner that trusts shell-integration marks only with `nonce` (see
    /// `nonce` and `nonce_seen`).
    pub fn with_nonce(nonce: Option<String>) -> Self {
        Self {
            nonce,
            ..Self::new()
        }
    }

    /// Scan a chunk of bytes. Call `drain_events()` after to collect results.
    pub fn scan(&mut self, bytes: &[u8]) {
        if self.state != State::Normal {
            self.escape_start_offset = None;
            self.osc_start_offset = None;
            self.csi_start_offset = None;
            self.apc_start_offset = None;
        }
        if self.state == State::OscSkip {
            // Still dropping an oversized OSC from the last chunk.
            self.skip_start = Some((0, false));
        }

        for (offset, &b) in bytes.iter().enumerate() {
            match self.state {
                State::Normal => {
                    if b == 0x1B {
                        self.state = State::Escape;
                        self.escape_start_offset = Some(offset);
                    }
                }
                State::Escape => self.escape_byte(b, offset),
                State::OscBody => match b {
                    // BEL terminates the OSC sequence.
                    0x07 => {
                        self.dispatch_osc(offset + 1);
                        self.state = State::Normal;
                    }
                    0x1B => self.state = State::OscEscape,
                    // CAN and SUB cancel it.
                    0x18 | 0x1A => self.cancel_osc(),
                    // vte ignores other C0 controls inside an OSC.
                    0x00..=0x1F => {}
                    _ => {
                        self.osc_len += 1;
                        if self.osc_len > self.osc_limit() {
                            self.cancel_osc();
                            self.state = State::OscSkip;
                            self.skip_start = Some((offset, true));
                        } else if self.buf.len() < self.kept_len() {
                            self.buf.push(b);
                        }
                    }
                },
                State::OscEscape => {
                    if b == b'\\' {
                        self.dispatch_osc(offset + 1);
                        self.state = State::Normal;
                    } else {
                        // The ESC ended the OSC and begins a new sequence, as
                        // in vte: `ESC ]` here opens the next OSC.
                        self.dispatch_osc(offset);
                        self.state = State::Escape;
                        self.escape_start_offset = offset.checked_sub(1);
                        self.escape_byte(b, offset);
                    }
                }
                State::OscSkip => match b {
                    0x07 => self.end_skip(offset + 1, State::Normal),
                    // Not dropped: the parser takes these as it would at the
                    // end of an OSC (ESC starting the next sequence).
                    0x1B => {
                        self.end_skip(offset, State::Escape);
                        self.escape_start_offset = Some(offset);
                    }
                    0x18 | 0x1A => self.end_skip(offset, State::Normal),
                    _ => {}
                },
                State::Csi => self.csi_byte(b, offset),
                State::ApcStart => {
                    if b == b'G' {
                        self.apc.clear();
                        self.apc.push(b);
                        self.state = State::ApcBody;
                    } else {
                        // vte ignores other APCs (and SOS and PM strings)
                        // to their end, as this does.
                        self.apc_start_offset = None;
                        self.state = State::ApcSkip;
                        self.apc_skip_byte(b, offset);
                    }
                }
                State::ApcBody => match b {
                    0x1B => self.state = State::ApcEscape,
                    0x18 | 0x1A => {
                        self.apc.clear();
                        self.apc_start_offset = None;
                        self.state = State::Normal;
                    }
                    _ if self.apc.len() >= MAX_APC_LEN => {
                        self.apc = Vec::new();
                        self.apc_start_offset = None;
                        self.state = State::ApcSkip;
                    }
                    _ => self.apc.push(b),
                },
                State::ApcEscape => {
                    if b == b'\\' {
                        let apc = std::mem::take(&mut self.apc);
                        let start = self.apc_start_offset.take();
                        self.emit(OscEvent::Graphics(apc), start, offset + 1);
                        self.state = State::Normal;
                    } else {
                        // Unterminated: dropped, and the ESC starts the
                        // next sequence, as in vte.
                        self.apc.clear();
                        self.apc_start_offset = None;
                        self.state = State::Escape;
                        self.escape_start_offset = offset.checked_sub(1);
                        self.escape_byte(b, offset);
                    }
                }
                State::ApcSkip => self.apc_skip_byte(b, offset),
            }
        }

        if let Some((start, abort)) = self.skip_start.take() {
            self.skips.push(OscSkip {
                start,
                end: bytes.len(),
                abort,
            });
        }
    }

    /// A byte after ESC, outside an OSC. Like vte, another ESC or a C0
    /// control (other than CAN/SUB) keeps the escape open, so `ESC ESC ]`
    /// still starts an OSC.
    fn escape_byte(&mut self, b: u8, offset: usize) {
        match b {
            b']' => {
                self.state = State::OscBody;
                self.buf.clear();
                self.osc_len = 0;
                self.osc_start_offset = self.escape_start_offset.take();
            }
            b'[' => {
                self.state = State::Csi;
                self.csi_count = 0;
                self.csi_current = 0;
                self.csi_private = 0;
                self.csi_started = false;
                self.csi_ignore = false;
                self.csi_start_offset = self.escape_start_offset.take();
            }
            b'_' => {
                self.state = State::ApcStart;
                self.apc_start_offset = self.escape_start_offset.take();
            }
            // SOS and PM strings, ignored to their end like other APCs.
            b'X' | b'^' => {
                self.state = State::ApcSkip;
                self.escape_start_offset = None;
            }
            b'c' => {
                let start = self.escape_start_offset.take();
                self.emit(OscEvent::FullReset, start, offset + 1);
                self.state = State::Normal;
            }
            0x18 | 0x1A | 0x20..=0x7E => {
                self.state = State::Normal;
                self.escape_start_offset = None;
            }
            _ => {}
        }
    }

    /// A byte of a string vte ignores (APC, SOS, PM): ESC ends it and starts
    /// the next sequence, CAN and SUB cancel it.
    fn apc_skip_byte(&mut self, b: u8, offset: usize) {
        match b {
            0x1B => {
                self.state = State::Escape;
                self.escape_start_offset = Some(offset);
            }
            0x18 | 0x1A => self.state = State::Normal,
            _ => {}
        }
    }

    /// A byte of a CSI sequence, following vte's states: C0 controls run
    /// inside it, ESC starts the next sequence, CAN and SUB cancel it.
    fn csi_byte(&mut self, b: u8, offset: usize) {
        match b {
            0x18 | 0x1A => {
                self.state = State::Normal;
                self.csi_start_offset = None;
            }
            0x1B => {
                self.state = State::Escape;
                self.escape_start_offset = Some(offset);
                self.csi_start_offset = None;
            }
            0x20..=0x2F | b':' => self.csi_ignore = true,
            b'0'..=b'9' => {
                self.csi_started = true;
                self.csi_current = self
                    .csi_current
                    .saturating_mul(10)
                    .saturating_add(u16::from(b - b'0'));
            }
            b';' => {
                self.csi_started = true;
                self.csi_param_done();
            }
            0x3C..=0x3F => {
                if !self.csi_started && self.csi_private == 0 {
                    self.csi_private = b;
                } else {
                    self.csi_ignore = true;
                }
            }
            0x40..=0x7E => {
                self.csi_param_done();
                self.state = State::Normal;
                let start = self.csi_start_offset.take();
                match self.csi_event(b) {
                    // Answered instead of alacritty, which then mustn't see
                    // it; one begun in an earlier read it has partly seen,
                    // so it answers that one.
                    Some(OscEvent::DeviceAttributes) => {
                        if let Some(start) = start {
                            self.skips.push(OscSkip {
                                start,
                                end: offset + 1,
                                abort: false,
                            });
                            self.emit(OscEvent::DeviceAttributes, Some(start), offset + 1);
                        }
                    }
                    Some(event) => self.emit(event, start, offset + 1),
                    None => {}
                }
            }
            // C0 controls run; DEL and bytes past ASCII are ignored.
            _ => {}
        }
    }

    fn csi_param_done(&mut self) {
        if self.csi_count < MAX_CSI_PARAMS {
            self.csi_params[self.csi_count] = self.csi_current;
            self.csi_count += 1;
        }
        self.csi_current = 0;
    }

    /// The event a finished CSI sequence with final byte `b` makes, if any.
    fn csi_event(&self, b: u8) -> Option<OscEvent> {
        if self.csi_ignore {
            return None;
        }
        let params = &self.csi_params[..self.csi_count];
        let first = params.first().copied().unwrap_or(0);
        match (self.csi_private, b) {
            (0, b'J') if first == 2 || first == 3 => Some(OscEvent::EraseDisplay(first)),
            (b'?', b'h' | b'l') if params.iter().any(|p| matches!(p, 47 | 1047 | 1049)) => {
                Some(OscEvent::AltScreen(b == b'h'))
            }
            (0, b't') if first == 16 => Some(OscEvent::CellSizeRequest),
            (0, b'c') if first == 0 && params.len() <= 1 => Some(OscEvent::DeviceAttributes),
            _ => None,
        }
    }

    /// Record an event with where it starts and ends in this chunk.
    /// Graphics commands, which can be large, aren't kept for
    /// `drain_events`.
    fn emit(&mut self, event: OscEvent, start_offset: Option<usize>, end_offset: usize) {
        if !matches!(event, OscEvent::Graphics(_)) {
            self.events.push(event.clone());
        }
        self.event_spans.push(OscEventSpan {
            event,
            start_offset,
            end_offset,
        });
    }

    /// The longest the current OSC may be, by its number.
    fn osc_limit(&self) -> usize {
        let buf = self.buf.as_slice();
        if buf.starts_with(b"6973;") {
            MAX_IMPULSE_OSC_LEN
        } else if buf.starts_with(b"52;") {
            MAX_CLIPBOARD_OSC_LEN
        } else if buf.starts_with(b"0;") || buf.starts_with(b"1;") || buf.starts_with(b"2;") {
            MAX_TITLE_OSC_LEN
        } else {
            MAX_OSC_LEN
        }
    }

    /// How much of the current OSC to keep for parsing.
    fn kept_len(&self) -> usize {
        if self.buf.starts_with(b"6973;") {
            MAX_IMPULSE_OSC_LEN
        } else {
            MAX_OSC_LEN
        }
    }

    /// Drop the OSC in progress without an event.
    fn cancel_osc(&mut self) {
        self.buf.clear();
        self.state = State::Normal;
        self.osc_start_offset = None;
        self.escape_start_offset = None;
    }

    fn end_skip(&mut self, end: usize, next: State) {
        if let Some((start, abort)) = self.skip_start.take() {
            self.skips.push(OscSkip { start, end, abort });
        }
        self.state = next;
    }

    /// Drain all events emitted since the last drain.
    pub fn drain_events(&mut self) -> Vec<OscEvent> {
        self.event_spans.clear();
        self.skips.clear();
        std::mem::take(&mut self.events)
    }

    pub(crate) fn drain_event_spans(&mut self) -> Vec<OscEventSpan> {
        self.events.clear();
        std::mem::take(&mut self.event_spans)
    }

    /// The oversized-OSC bytes of the last chunk (see `OscSkip`).
    pub(crate) fn drain_skips(&mut self) -> Vec<OscSkip> {
        std::mem::take(&mut self.skips)
    }

    fn dispatch_osc(&mut self, end_offset: usize) {
        let buf = std::mem::take(&mut self.buf);
        let event = match buf.strip_prefix(b"99;") {
            Some(payload) => self.parse_kitty_notification(payload),
            None => self.parse_osc(&buf),
        };
        self.buf = buf;
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

    /// Whether a shell-integration mark carrying `nonce` may be trusted.
    /// Command text (`required`) always needs it; other marks need it once
    /// one carrying it has arrived. A wrong nonce is never trusted.
    fn trusts(&mut self, nonce: Option<&[u8]>, required: bool) -> bool {
        let Some(expected) = self.nonce.as_deref() else {
            return true;
        };
        match nonce.filter(|nonce| !nonce.is_empty()) {
            Some(nonce) if nonce == expected.as_bytes() => {
                self.nonce_seen = true;
                true
            }
            Some(_) => false,
            None => !required && !self.nonce_seen,
        }
    }

    fn parse_osc(&mut self, buf: &[u8]) -> Option<OscEvent> {
        if buf.is_empty() {
            return None;
        }

        // OSC 7 (CWD): "7;file://host/path[;Nonce=...]"
        if let Some(payload) = buf.strip_prefix(b"7;") {
            let (payload, nonce) = split_nonce(payload);
            if !self.trusts(nonce, false) {
                return None;
            }
            return self.parse_osc7(payload).map(OscEvent::CwdChanged);
        }

        // OSC 133 (shell integration): "133;X[;params]", "133;D;code[;params]"
        if let Some(payload) = buf.strip_prefix(b"133;") {
            let mut params = payload.split(|&b| b == b';');
            let kind = params.next().unwrap_or_default();
            let (mut code, mut nonce) = (0, None);
            for (index, param) in params.enumerate() {
                if let Some(value) = param.strip_prefix(b"Nonce=") {
                    nonce = Some(value);
                } else if index == 0 {
                    code = std::str::from_utf8(param)
                        .ok()
                        .and_then(|s| s.parse::<i32>().ok())
                        .unwrap_or(0);
                }
            }
            let event = match kind {
                b"A" => Some(OscEvent::PromptStart),
                b"B" => None, // Prompt end. Not currently exposed to frontends.
                b"C" => Some(OscEvent::CommandStart),
                b"D" => Some(OscEvent::CommandEnd(code)),
                _ => return None,
            };
            return if self.trusts(nonce, false) {
                event
            } else {
                None
            };
        }

        if let Some(payload) = buf.strip_prefix(b"6973;Command=") {
            return self
                .parse_impulse_command(payload)
                .map(OscEvent::CommandText);
        }

        if let Some(payload) = buf.strip_prefix(b"6973;Names=") {
            let (payload, nonce) = split_nonce(payload);
            if !self.trusts(nonce, false) {
                return None;
            }
            let text = std::str::from_utf8(payload).ok()?;
            let names = text
                .split_whitespace()
                .filter(|name| name.len() <= 256 && !name.chars().any(char::is_control))
                .take(MAX_SHELL_NAMES)
                .map(str::to_string)
                .collect();
            return Some(OscEvent::ShellNames(names));
        }

        if let Some(payload) = buf.strip_prefix(b"6973;Path=") {
            let (payload, nonce) = split_nonce(payload);
            if !self.trusts(nonce, false) {
                return None;
            }
            let text = std::str::from_utf8(payload).ok()?;
            let path: String = text.chars().filter(|c| !c.is_control()).collect();
            return Some(OscEvent::ShellPath(path));
        }

        if let Some(payload) = buf.strip_prefix(b"21337;") {
            return Self::parse_session_status(payload);
        }

        if let Some(payload) = buf.strip_prefix(b"1337;") {
            return Self::parse_iterm2_attention(payload).map(OscEvent::AttentionRequest);
        }

        // OSC 9;4 is the ConEmu progress protocol, not a notification. It must
        // be matched before the generic OSC 9 notification below.
        if buf == b"9;4" || buf.starts_with(b"9;4;") {
            return Self::parse_progress(&buf[3..]);
        }

        if let Some(payload) = buf.strip_prefix(b"9;") {
            if let Ok(body) = std::str::from_utf8(payload) {
                return Some(OscEvent::Notification {
                    title: "Terminal".to_string(),
                    body: Self::sanitize_text(body),
                });
            }
        }

        if let Some(payload) = buf.strip_prefix(b"777;") {
            return Self::parse_rxvt_notify(payload)
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

    /// Parse an iTerm2 session status payload after "21337;":
    /// `key=value` pairs separated by ';'. Unknown keys are ignored; values
    /// are capped at 200 characters.
    fn parse_session_status(payload: &[u8]) -> Option<OscEvent> {
        let s = std::str::from_utf8(payload).ok()?;
        let mut fields = std::collections::BTreeMap::new();
        for pair in s.split(';') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            if matches!(key, "status" | "indicator" | "status-color" | "detail") {
                let value: String = Self::sanitize_text(value).chars().take(200).collect();
                fields.insert(key.to_string(), value);
            }
        }
        (!fields.is_empty()).then_some(OscEvent::SessionStatus(fields))
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
        let part = if kind == "body" {
            &mut entry.body
        } else {
            &mut entry.title
        };
        // A notification is a few lines; endless `d=0` chunks would grow
        // without bound, so anything past the cap is dropped.
        let room = KITTY_NOTIFICATION_MAX_BYTES.saturating_sub(part.len());
        if text.len() <= room {
            part.push_str(&text);
        } else {
            let mut end = room;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            part.push_str(&text[..end]);
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

    /// `<percent-encoded command>[;Nonce=<nonce>]`. The encoding leaves no
    /// raw `;` in the command.
    fn parse_impulse_command(&mut self, payload: &[u8]) -> Option<String> {
        let text = std::str::from_utf8(payload).ok()?;
        let (encoded, nonce) = match text.split_once(';') {
            Some((encoded, rest)) => (encoded, rest.strip_prefix("Nonce=")),
            None => (text, None),
        };
        if !self.trusts(nonce.map(str::as_bytes), true) {
            return None;
        }
        let decoded = Self::url_decode(encoded)?;
        Some(Self::sanitize_text(&decoded))
    }

    /// Parse OSC 7 payload: "file://hostname/path" → URL-decoded path.
    fn parse_osc7(&self, payload: &[u8]) -> Option<String> {
        let s = std::str::from_utf8(payload).ok()?;

        // Strip "file://" prefix.
        let rest = s.strip_prefix("file://")?;

        // The host is everything up to the first '/'. A directory on another
        // machine (a shell over ssh reporting its own cwd) isn't a local
        // folder, even when a folder of that name exists here.
        let path_start = rest.find('/')?;
        if !Self::is_local_host(&rest[..path_start]) {
            return None;
        }
        let encoded_path = &rest[path_start..];

        // URL-decode the path.
        let decoded = Self::url_decode(encoded_path)?;

        // Validate: must be an existing absolute directory and safe to pass
        // across FFI boundaries.
        if decoded.starts_with('/') && !decoded.contains('\0') && self.is_directory(&decoded) {
            Some(decoded)
        } else {
            None
        }
    }

    /// Whether `path` is a directory, asked on a helper thread so a folder
    /// on a mount that stopped answering can't stall the terminal's output:
    /// past `DIRECTORY_CHECK_WAIT`, or while an earlier check still hangs,
    /// it counts as not one.
    fn is_directory(&self, path: &str) -> bool {
        use std::sync::atomic::Ordering;
        let checking = self.checking_directory.clone();
        if checking.swap(true, Ordering::SeqCst) {
            return false;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        let path = path.to_string();
        let spawned = std::thread::Builder::new()
            .name("impulse-osc7-check".into())
            .spawn(move || {
                let answer = std::path::Path::new(&path).is_dir();
                // Before the answer, so the next check never sees this one
                // still running.
                checking.store(false, Ordering::SeqCst);
                let _ = sender.send(answer);
            });
        if spawned.is_err() {
            self.checking_directory.store(false, Ordering::SeqCst);
            return false;
        }
        receiver.recv_timeout(DIRECTORY_CHECK_WAIT).unwrap_or(false)
    }

    /// An OSC 7 host that means this Mac: none, `localhost`, or its name
    /// (compared without the domain: `Mac` and `Mac.local`).
    fn is_local_host(host: &str) -> bool {
        if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
            return true;
        }
        let short = |name: &str| name.split('.').next().unwrap_or("").to_ascii_lowercase();
        short(host) == short(&local_hostname())
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

/// `payload[;Nonce=value]` → (payload, value). The last `;Nonce=` counts,
/// so a payload containing one (a PATH entry) stays whole.
fn split_nonce(payload: &[u8]) -> (&[u8], Option<&[u8]>) {
    const MARKER: &[u8] = b";Nonce=";
    match payload
        .windows(MARKER.len())
        .rposition(|window| window == MARKER)
    {
        Some(at) => (&payload[..at], Some(&payload[at + MARKER.len()..])),
        None => (payload, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_iterm2_session_status() {
        let mut scanner = OscScanner::new();
        scanner
            .scan(b"\x1b]21337;status=Working;indicator=#ffa500;bogus=1;detail=Editing files\x07");
        let events = scanner.drain_events();
        let mut expected = std::collections::BTreeMap::new();
        expected.insert("status".to_string(), "Working".to_string());
        expected.insert("indicator".to_string(), "#ffa500".to_string());
        expected.insert("detail".to_string(), "Editing files".to_string());
        assert_eq!(events, vec![OscEvent::SessionStatus(expected)]);

        scanner.scan(b"\x1b]21337;status=;indicator=\x1b\\");
        let cleared = scanner.drain_events();
        let mut empty = std::collections::BTreeMap::new();
        empty.insert("indicator".to_string(), String::new());
        empty.insert("status".to_string(), String::new());
        assert_eq!(cleared, vec![OscEvent::SessionStatus(empty)]);

        scanner.scan(b"\x1b]21337;nothing=here\x07");
        assert!(scanner.drain_events().is_empty());
        // OSC 1337 still means attention.
        scanner.scan(b"\x1b]1337;RequestAttention=yes\x07");
        assert_eq!(
            scanner.drain_events(),
            vec![OscEvent::AttentionRequest("yes".to_string())]
        );
    }

    #[test]
    fn test_osc7_cwd() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().to_string_lossy();
        let mut scanner = OscScanner::new();
        let seq = format!("\x1b]7;file://{}{cwd}\x07", local_hostname());
        scanner.scan(seq.as_bytes());
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::CwdChanged(cwd.to_string())]);
    }

    #[test]
    fn test_osc7_ignores_other_hosts() {
        // An ssh session's shell reports its own cwd under its own name.
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().to_string_lossy();
        let mut scanner = OscScanner::new();
        scanner.scan(format!("\x1b]7;file://build-server.example.com{cwd}\x07").as_bytes());
        assert!(scanner.drain_events().is_empty());
        scanner.scan(format!("\x1b]7;file://localhost{cwd}\x07").as_bytes());
        scanner.scan(format!("\x1b]7;file://{cwd}\x07").as_bytes());
        assert_eq!(scanner.drain_events().len(), 2);
    }

    #[test]
    fn test_osc7_url_encoded() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("my dir").join("foo").join("bar");
        std::fs::create_dir_all(&cwd).unwrap();
        let encoded = cwd.to_string_lossy().replace(' ', "%20");
        let mut scanner = OscScanner::new();
        let seq = format!("\x1b]7;file://localhost{encoded}\x07");
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
        let seq = format!("\x1b]7;file://localhost{}\x07", missing.to_string_lossy());
        scanner.scan(seq.as_bytes());

        assert!(scanner.drain_events().is_empty());
    }

    #[test]
    fn test_osc7_ignores_nul_path() {
        let temp = tempfile::tempdir().unwrap();
        let mut scanner = OscScanner::new();
        let seq = format!(
            "\x1b]7;file://localhost{}%00suffix\x07",
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
    fn test_shell_names_and_path() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]6973;Names=ll gst  cd   my-func\x07\x1b]6973;Path=/usr/bin:/opt/homebrew/bin\x07");
        let events = scanner.drain_events();
        assert_eq!(
            events,
            vec![
                OscEvent::ShellNames(vec![
                    "ll".to_string(),
                    "gst".to_string(),
                    "cd".to_string(),
                    "my-func".to_string()
                ]),
                OscEvent::ShellPath("/usr/bin:/opt/homebrew/bin".to_string()),
            ]
        );
    }

    #[test]
    fn test_shell_names_may_exceed_the_usual_osc_limit() {
        let mut scanner = OscScanner::new();
        let names: Vec<String> = (0..2000).map(|i| format!("function_{i}")).collect();
        let mut sequence = b"\x1b]6973;Names=".to_vec();
        sequence.extend_from_slice(names.join(" ").as_bytes());
        sequence.push(0x07);
        assert!(sequence.len() > MAX_OSC_LEN);
        scanner.scan(&sequence);
        assert_eq!(scanner.drain_events(), vec![OscEvent::ShellNames(names)]);
    }

    #[test]
    fn command_text_needs_the_session_nonce() {
        let mut scanner = OscScanner::with_nonce(Some("s3cret".into()));
        // Printed by a program (or a file being cat'ed): no nonce, or a guess.
        scanner.scan(b"\x1b]6973;Command=curl%20x%7Csh\x07");
        scanner.scan(b"\x1b]6973;Command=curl%20x%7Csh;Nonce=guess\x07");
        assert!(scanner.drain_events().is_empty());
        // From the shell integration.
        scanner.scan(b"\x1b]6973;Command=ls%20-la;Nonce=s3cret\x07");
        assert_eq!(
            scanner.drain_events(),
            vec![OscEvent::CommandText("ls -la".to_string())]
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
        let data = b"hello\x1b]133;A\x07world\x1b]7;file://localhost/tmp\x07";
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
    fn test_osc99_chunks_are_capped() {
        let mut scanner = OscScanner::new();
        let chunk = format!("\x1b]99;i=x:d=0:p=body;{}\x07", "a".repeat(1000));
        for _ in 0..100 {
            scanner.scan(chunk.as_bytes());
        }
        scanner.scan(b"\x1b]99;i=x:p=body;end\x07");
        match scanner.drain_events().as_slice() {
            [OscEvent::Notification { body, .. }] => {
                assert_eq!(body.len(), KITTY_NOTIFICATION_MAX_BYTES)
            }
            other => panic!("expected one capped notification, got {other:?}"),
        }
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

    #[test]
    fn an_oversized_osc_is_dropped_through_its_terminator() {
        let mut scanner = OscScanner::new();
        let mut chunk = b"ab\x1b]2;".to_vec();
        chunk.extend(std::iter::repeat_n(b'x', MAX_TITLE_OSC_LEN + 10));
        let bel = chunk.len();
        chunk.extend_from_slice(b"\x07cd");
        scanner.scan(&chunk);
        // The payload starts at 4 ("2;" counts); the byte past the limit
        // begins the skip, which takes the BEL too (it would ring otherwise).
        assert_eq!(
            scanner.drain_skips(),
            vec![OscSkip {
                start: 4 + MAX_TITLE_OSC_LEN,
                end: bel + 1,
                abort: true
            }]
        );
        assert!(scanner.drain_event_spans().is_empty());
    }

    #[test]
    fn a_skip_carries_over_to_the_next_chunk_and_ends_at_esc() {
        let mut scanner = OscScanner::new();
        let mut chunk = b"\x1b]".to_vec();
        chunk.extend(std::iter::repeat_n(b'y', MAX_OSC_LEN + 5));
        scanner.scan(&chunk);
        assert_eq!(
            scanner.drain_skips(),
            vec![OscSkip {
                start: 2 + MAX_OSC_LEN,
                end: chunk.len(),
                abort: true
            }]
        );
        // ESC ends it (and isn't dropped): here it opens the next OSC.
        scanner.scan(b"yyy\x1b]133;A\x07");
        assert_eq!(
            scanner.drain_skips(),
            vec![OscSkip {
                start: 0,
                end: 3,
                abort: false
            }]
        );
        let spans = scanner.drain_event_spans();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].event, OscEvent::PromptStart);
        assert_eq!(spans[0].start_offset, Some(3));
    }

    #[test]
    fn clipboard_writes_and_impulse_marks_have_room() {
        let mut scanner = OscScanner::new();
        let mut chunk = b"\x1b]52;c;".to_vec();
        chunk.extend(std::iter::repeat_n(b'A', 100_000));
        chunk.push(0x07);
        scanner.scan(&chunk);
        assert!(scanner.drain_skips().is_empty());
        // A title that long is dropped.
        let mut title = b"\x1b]0;".to_vec();
        title.extend(std::iter::repeat_n(b'A', 100_000));
        title.push(0x07);
        scanner.scan(&title);
        assert_eq!(scanner.drain_skips().len(), 1);
    }

    #[test]
    fn an_esc_that_ends_an_osc_can_open_the_next() {
        // vte ends the title at ESC and reads `]` as a new OSC.
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]0;title\x1b]133;A\x07");
        let spans = scanner.drain_event_spans();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].event, OscEvent::PromptStart);
        assert_eq!(spans[0].start_offset, Some(9));
    }

    #[test]
    fn repeated_esc_and_c0_controls_keep_an_escape_open() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b\x1b]133;A\x07");
        scanner.scan(b"\x1b\n]133;C\x07");
        // Inside the OSC, C0 controls other than BEL/CAN/SUB/ESC are ignored.
        scanner.scan(b"\x1b]133;\r\nD;2\x07");
        assert_eq!(
            scanner.drain_events(),
            vec![
                OscEvent::PromptStart,
                OscEvent::CommandStart,
                OscEvent::CommandEnd(2)
            ]
        );
        // Anything else after ESC is another sequence.
        scanner.scan(b"\x1b(]133;A\x07");
        assert!(scanner.drain_events().is_empty());
    }

    #[test]
    fn can_and_sub_cancel_an_osc() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]133;A\x18");
        // The rest is plain text, not the OSC's continuation.
        scanner.scan(b"133;C\x07");
        scanner.scan(b"\x1b]133;A\x1a;D\x07");
        assert!(scanner.drain_events().is_empty());
        scanner.scan(b"\x1b\x18]133;A\x07");
        assert!(scanner.drain_events().is_empty());
        scanner.scan(b"\x1b]133;C\x07");
        assert_eq!(scanner.drain_events(), vec![OscEvent::CommandStart]);
    }

    #[test]
    fn marks_need_the_nonce_once_the_integration_sent_it() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().to_string_lossy().to_string();
        let mut scanner = OscScanner::with_nonce(Some("s3cret".into()));
        // A shell with its own integration (no nonce) works...
        scanner.scan(b"\x1b]133;A\x07\x1b]133;C\x07\x1b]133;D;1\x07");
        scanner.scan(format!("\x1b]7;file://{cwd}\x07").as_bytes());
        assert_eq!(
            scanner.drain_events(),
            vec![
                OscEvent::PromptStart,
                OscEvent::CommandStart,
                OscEvent::CommandEnd(1),
                OscEvent::CwdChanged(cwd.clone()),
            ]
        );
        // ...but a wrong nonce never does.
        scanner.scan(b"\x1b]133;A;Nonce=guess\x07");
        assert!(scanner.drain_events().is_empty());

        // Impulse's integration: every mark carries the nonce.
        scanner.scan(b"\x1b]133;A;Nonce=s3cret\x07\x1b]133;D;2;Nonce=s3cret\x07");
        scanner.scan(format!("\x1b]7;file://{cwd};Nonce=s3cret\x07").as_bytes());
        scanner.scan(b"\x1b]6973;Names=ll gst;Nonce=s3cret\x07");
        scanner.scan(b"\x1b]6973;Path=/bin:/a;Nonce=x;Nonce=s3cret\x07");
        assert_eq!(
            scanner.drain_events(),
            vec![
                OscEvent::PromptStart,
                OscEvent::CommandEnd(2),
                OscEvent::CwdChanged(cwd.clone()),
                OscEvent::ShellNames(vec!["ll".into(), "gst".into()]),
                OscEvent::ShellPath("/bin:/a;Nonce=x".into()),
            ]
        );

        // From then on, output can't forge marks.
        scanner.scan(b"\x1b]133;A\x07\x1b]133;C\x07\x1b]133;D;1\x07\x1b]133;D;1;aid=1\x07");
        scanner.scan(format!("\x1b]7;file://{cwd}\x07").as_bytes());
        scanner.scan(b"\x1b]6973;Names=evil\x07\x1b]6973;Path=/tmp\x07");
        scanner.scan(b"\x1b]6973;Path=/tmp;Nonce=\x07");
        assert!(scanner.drain_events().is_empty());
    }

    #[test]
    fn command_end_reads_its_code_beside_other_params() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]133;D;130;aid=7\x07\x1b]133;D;;aid=7\x07\x1b]133;Dx\x07");
        assert_eq!(
            scanner.drain_events(),
            vec![OscEvent::CommandEnd(130), OscEvent::CommandEnd(0)]
        );
    }

    fn spans(scanner: &mut OscScanner) -> Vec<(OscEvent, Option<usize>, usize)> {
        scanner
            .drain_event_spans()
            .into_iter()
            .map(|s| (s.event, s.start_offset, s.end_offset))
            .collect()
    }

    #[test]
    fn graphics_commands_with_their_offsets() {
        let mut scanner = OscScanner::new();
        let bytes = b"ab\x1b_Ga=T,f=100;AAAA\x1b\\cd";
        scanner.scan(bytes);
        assert_eq!(
            spans(&mut scanner),
            vec![(
                OscEvent::Graphics(b"Ga=T,f=100;AAAA".to_vec()),
                Some(2),
                bytes.len() - 2
            )]
        );
        // Not kept for drain_events.
        scanner.scan(b"\x1b_Gi=1;\x1b\\");
        assert!(scanner.drain_events().is_empty());
    }

    #[test]
    fn graphics_commands_across_reads() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b_Ga=t,");
        assert!(spans(&mut scanner).is_empty());
        scanner.scan(b"i=1;QUJD\x1b");
        assert!(spans(&mut scanner).is_empty());
        scanner.scan(b"\\x");
        assert_eq!(
            spans(&mut scanner),
            vec![(OscEvent::Graphics(b"Ga=t,i=1;QUJD".to_vec()), None, 1)]
        );
    }

    #[test]
    fn other_strings_and_cancelled_commands_are_ignored() {
        let mut scanner = OscScanner::new();
        // Another APC, SOS, PM, a cancelled command and an unterminated
        // one (whose ESC starts the OSC after it).
        scanner.scan(b"\x1b_Xyz\x1b\\\x1bXsos\x1b\\\x1b^pm\x1b\\\x1b_Gi=1\x18");
        scanner.scan(b"\x1b_Gi=2\x1b]133;A\x07");
        assert_eq!(scanner.drain_events(), vec![OscEvent::PromptStart]);
        assert!(spans(&mut scanner).is_empty());
    }

    #[test]
    fn oversized_graphics_commands_are_dropped() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b_Gi=1;");
        scanner.scan(&vec![b'A'; MAX_APC_LEN + 1]);
        scanner.scan(b"\x1b\\\x1b_Gi=2;\x1b\\");
        let events: Vec<_> = spans(&mut scanner).into_iter().map(|s| s.0).collect();
        assert_eq!(events, vec![OscEvent::Graphics(b"Gi=2;".to_vec())]);
    }

    #[test]
    fn watched_csi_sequences() {
        let mut scanner = OscScanner::new();
        let bytes =
            b"\x1b[2J\x1b[3J\x1b[J\x1b[1J\x1b[?1049h\x1b[?25;1049l\x1b[?1h\x1b[16t\x1b[14t\x1bc";
        scanner.scan(bytes);
        assert_eq!(
            scanner.drain_events(),
            vec![
                OscEvent::EraseDisplay(2),
                OscEvent::EraseDisplay(3),
                OscEvent::AltScreen(true),
                OscEvent::AltScreen(false),
                OscEvent::CellSizeRequest,
                OscEvent::FullReset,
            ]
        );
        scanner.scan(b"x\x1b[2J");
        assert_eq!(
            spans(&mut scanner),
            vec![(OscEvent::EraseDisplay(2), Some(1), 5)]
        );
    }

    #[test]
    fn csi_follows_vte() {
        let mut scanner = OscScanner::new();
        // Intermediates, a private marker after parameters and a cancel
        // make none of them; a C0 control inside runs and the CSI goes on;
        // ESC inside starts a new sequence.
        scanner.scan(b"\x1b[2 J\x1b[2?J\x1b[2\x18J\x1b[2\nJ\x1b[2\x1b[3J");
        assert_eq!(
            scanner.drain_events(),
            vec![OscEvent::EraseDisplay(2), OscEvent::EraseDisplay(3)]
        );
        // Split across reads.
        scanner.scan(b"\x1b[?10");
        scanner.scan(b"49h");
        assert_eq!(
            spans(&mut scanner),
            vec![(OscEvent::AltScreen(true), None, 3)]
        );
        // An OSC right after still parses.
        scanner.scan(b"\x1b[0m\x1b]133;A\x07");
        assert_eq!(scanner.drain_events(), vec![OscEvent::PromptStart]);
    }

    #[test]
    fn device_attributes_are_skipped_for_the_parser() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"ab\x1b[c\x1b[0c\x1b[>c\x1b[?1;2c");
        assert_eq!(
            spans(&mut scanner),
            vec![
                (OscEvent::DeviceAttributes, Some(2), 5),
                (OscEvent::DeviceAttributes, Some(5), 9),
            ]
        );
        // Secondary attributes and replies aren't requests Impulse answers.
        let skips: Vec<_> = scanner
            .drain_skips()
            .iter()
            .map(|s| (s.start, s.end))
            .collect();
        assert_eq!(skips, [(2, 5), (5, 9)]);
        // Begun in an earlier read, it's left to the parser.
        scanner.scan(b"\x1b[");
        scanner.scan(b"c");
        assert!(spans(&mut scanner).is_empty());
        assert!(scanner.drain_skips().is_empty());
    }
}
