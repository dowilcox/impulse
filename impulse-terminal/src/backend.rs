//! Terminal backend — owns the alacritty_terminal::Term and PTY event loop.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

use alacritty_terminal::event::{Event as AlacEvent, EventListener, OnResize, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags as AlacFlags;
use alacritty_terminal::term::{Term, TermDamage, TermMode};
use alacritty_terminal::tty::{self, EventedPty, EventedReadWrite, Options as PtyOptions};
use alacritty_terminal::vte::ansi::Processor;
use alacritty_terminal::vte::ansi::{
    Color as AlacColor, CursorShape as AlacCursorShape, NamedColor, Rgb as AlacRgb,
};
use crossbeam_channel::{Receiver, Sender};
use polling::{Event as PollingEvent, Events, PollMode, Poller};

use crate::blocks::{CommandBlockTracker, TerminalBlockId, TerminalCommandBlock};
use crate::buffer::{self, HighlightRange};
use crate::config::TerminalConfig;
use crate::event::TerminalEvent;
use crate::grid::{CellFlags, CursorShape, CursorState, RgbColor, TerminalMode};
use crate::history::{
    command_history_rerun_input, CommandHistoryContext, CommandHistoryQuery, CommandHistoryRecord,
    CommandHistorySearchResult, CommandHistoryStore,
};
use crate::search::{SearchResult, TerminalSearch};

const FILTERED_CHILD_ENV_VARS: &[&str] = &["NO_COLOR", "CLICOLOR", "CLICOLOR_FORCE", "FORCE_COLOR"];

static CHILD_ENV_LOCK: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// Event proxy — bridges alacritty events to our channel
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct EventProxy {
    event_tx: Sender<TerminalEvent>,
    wakeup_pending: Arc<AtomicBool>,
    /// Snapshot of the configured palette for answering OSC 4/10/11/12 color
    /// queries (TUI apps use these to detect light vs dark backgrounds).
    /// Shared with the backend, which refreshes it on theme changes. Runtime
    /// OSC 4 color overrides are not reflected here: this is called from the
    /// PTY reader thread while the Term is locked, so it must not touch Term.
    query_colors: Arc<RwLock<[RgbColor; 269]>>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: AlacEvent) {
        match event {
            AlacEvent::PtyWrite(text) => {
                let _ = self.event_tx.send(TerminalEvent::PtyWrite(text));
            }
            AlacEvent::Wakeup => {
                send_wakeup(&self.event_tx, &self.wakeup_pending);
            }
            AlacEvent::Title(title) => {
                let _ = self
                    .event_tx
                    .send(TerminalEvent::TitleChanged(sanitize_title(&title)));
            }
            AlacEvent::ResetTitle => {
                let _ = self.event_tx.send(TerminalEvent::ResetTitle);
            }
            AlacEvent::Bell => {
                let _ = self.event_tx.send(TerminalEvent::Bell);
            }
            AlacEvent::Exit => {
                let _ = self.event_tx.send(TerminalEvent::Exit);
            }
            AlacEvent::ChildExit(status) => {
                // ExitStatus -> i32: use code() which returns Option<i32>, defaulting to -1.
                let code = status.code().unwrap_or(-1);
                let _ = self.event_tx.send(TerminalEvent::ChildExited(code));
            }
            AlacEvent::ClipboardStore(_, text) => {
                let _ = self.event_tx.send(TerminalEvent::ClipboardStore(text));
            }
            AlacEvent::ClipboardLoad(_, _) => {
                let _ = self.event_tx.send(TerminalEvent::ClipboardLoad);
            }
            AlacEvent::CursorBlinkingChange => {
                let _ = self.event_tx.send(TerminalEvent::CursorBlinkingChange);
            }
            AlacEvent::ColorRequest(index, format) => {
                let color = {
                    let palette = self.query_colors.read().unwrap();
                    palette[index.min(palette.len() - 1)]
                };
                let response = format(AlacRgb {
                    r: color.r,
                    g: color.g,
                    b: color.b,
                });
                let _ = self.event_tx.send(TerminalEvent::PtyWrite(response));
            }
            AlacEvent::TextAreaSizeRequest(_) | AlacEvent::MouseCursorDirty => {}
        }
    }
}

fn send_wakeup(event_tx: &Sender<TerminalEvent>, wakeup_pending: &AtomicBool) {
    if wakeup_pending
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
        && event_tx.send(TerminalEvent::Wakeup).is_err()
    {
        wakeup_pending.store(false, Ordering::Release);
    }
}

/// Absolute grid row of the cursor: eviction estimate + history depth +
/// on-screen line. Stable across scrolling, exact under line wrapping.
fn absolute_cursor_row<T: EventListener>(term: &Term<T>, row_base: i64) -> i64 {
    let grid = term.grid();
    row_base + grid.history_size() as i64 + i64::from(grid.cursor.point.line.0)
}

/// Read and reset the term's accumulated damage.
///
/// Returns `None` for full damage, or `Some(rows)` with the damaged viewport
/// row indices (top = 0).
fn collect_term_damage<T: EventListener>(term: &mut Term<T>) -> Option<Vec<u16>> {
    let result = match term.damage() {
        TermDamage::Full => None,
        TermDamage::Partial(iter) => Some(
            iter.map(|bounds| bounds.line.min(u16::MAX as usize) as u16)
                .collect::<Vec<u16>>(),
        ),
    };
    term.reset_damage();
    result
}

// ---------------------------------------------------------------------------
// Terminal size helper
// ---------------------------------------------------------------------------

struct TermSize {
    columns: usize,
    screen_lines: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }
    fn screen_lines(&self) -> usize {
        self.screen_lines
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

// ---------------------------------------------------------------------------
// Color resolution
// ---------------------------------------------------------------------------

struct ConfiguredColors {
    foreground: RgbColor,
    background: RgbColor,
    palette: [RgbColor; 269],
    /// Minimum WCAG contrast ratio enforced between cell fg and bg.
    minimum_contrast: f32,
}

impl ConfiguredColors {
    fn from_config(config: &TerminalConfig) -> Self {
        let mut palette = [RgbColor::new(0, 0, 0); 269];

        // 16 ANSI colors from config.
        for (i, c) in config.colors.palette.iter().enumerate() {
            palette[i] = *c;
        }

        // 6x6x6 color cube (indices 16-231).
        for i in 16u16..232 {
            let idx = i - 16;
            let r = (idx / 36) as u8;
            let g = ((idx % 36) / 6) as u8;
            let b = (idx % 6) as u8;
            let to_val = |v: u8| if v == 0 { 0u8 } else { 55 + 40 * v };
            palette[i as usize] = RgbColor::new(to_val(r), to_val(g), to_val(b));
        }

        // Grayscale ramp (indices 232-255).
        for i in 232u16..256 {
            let val = (8 + 10 * (i - 232)) as u8;
            palette[i as usize] = RgbColor::new(val, val, val);
        }

        // Named colors.
        palette[NamedColor::Foreground as usize] = config.colors.foreground;
        palette[NamedColor::Background as usize] = config.colors.background;
        palette[NamedColor::Cursor as usize] = config.colors.foreground;
        palette[NamedColor::BrightForeground as usize] = config.colors.foreground;
        palette[NamedColor::DimForeground as usize] = config.colors.foreground;

        // Dim colors.
        palette[NamedColor::DimBlack as usize] = RgbColor::new(0, 0, 0);
        for i in 1..8usize {
            let base = config.colors.palette[i];
            palette[NamedColor::DimBlack as usize + i] = RgbColor::new(
                ((base.r as u16) * 3 / 4) as u8,
                ((base.g as u16) * 3 / 4) as u8,
                ((base.b as u16) * 3 / 4) as u8,
            );
        }

        Self {
            foreground: config.colors.foreground,
            background: config.colors.background,
            palette,
            minimum_contrast: config.minimum_contrast.clamp(1.0, 21.0),
        }
    }

    fn resolve(
        &self,
        color: AlacColor,
        term_colors: &alacritty_terminal::term::color::Colors,
    ) -> RgbColor {
        match color {
            AlacColor::Spec(rgb) => RgbColor::new(rgb.r, rgb.g, rgb.b),
            AlacColor::Named(named) => {
                if let Some(rgb) = term_colors[named] {
                    RgbColor::new(rgb.r, rgb.g, rgb.b)
                } else {
                    self.palette[named as usize]
                }
            }
            AlacColor::Indexed(idx) => {
                if let Some(rgb) = term_colors[idx as usize] {
                    RgbColor::new(rgb.r, rgb.g, rgb.b)
                } else {
                    self.palette[idx as usize]
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Minimum contrast (accessibility)
// ---------------------------------------------------------------------------

/// WCAG 2.1 relative luminance of an sRGB color.
fn relative_luminance(c: RgbColor) -> f32 {
    fn channel(v: u8) -> f32 {
        let v = v as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
}

/// WCAG 2.1 contrast ratio between two colors (1.0–21.0).
fn contrast_ratio(a: RgbColor, b: RgbColor) -> f32 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

fn mix(a: RgbColor, b: RgbColor, t: f32) -> RgbColor {
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    RgbColor::new(lerp(a.r, b.r), lerp(a.g, b.g), lerp(a.b, b.b))
}

/// Nudge `fg` toward black or white (whichever moves away from `bg`) until it
/// reaches `min_ratio` contrast against `bg`. Cells where fg == bg are left
/// untouched: apps use identical colors to intentionally hide text.
fn apply_minimum_contrast(fg: RgbColor, bg: RgbColor, min_ratio: f32) -> RgbColor {
    if min_ratio <= 1.0 || fg == bg || contrast_ratio(fg, bg) >= min_ratio {
        return fg;
    }
    let target = if relative_luminance(bg) > 0.179 {
        RgbColor::new(0, 0, 0)
    } else {
        RgbColor::new(255, 255, 255)
    };
    // Binary search the smallest blend toward the target that satisfies the
    // ratio, preserving as much of the original hue as possible.
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..7 {
        let mid = (lo + hi) / 2.0;
        if contrast_ratio(mix(fg, target, mid), bg) >= min_ratio {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let adjusted = mix(fg, target, hi);
    if contrast_ratio(adjusted, bg) >= min_ratio {
        adjusted
    } else {
        // Even a full blend can fall short (e.g. mid-gray backgrounds);
        // return the extreme as the best achievable.
        target
    }
}

// ---------------------------------------------------------------------------
// Selection kind
// ---------------------------------------------------------------------------

/// Selection kind for `start_selection()`.
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum SelectionKind {
    Simple = 0,
    Block = 1,
    Semantic = 2,
    Lines = 3,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CommandBlockFlags {
    pub has_command: bool,
    pub has_output: bool,
    pub has_failed: bool,
}

/// Exit codes that don't count as failures for block decoration:
/// 130 (SIGINT, user pressed Ctrl+C) and 141 (SIGPIPE, e.g. `cmd | head`).
const NON_FAILURE_EXIT_CODES: [i32; 2] = [130, 141];

/// One command block mapped into current viewport row coordinates.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BlockOverlayRegion {
    pub id: u64,
    /// Viewport row of the block's first line (its prompt). May be negative
    /// when the block starts above the visible area.
    pub start_row: i32,
    /// Viewport row of the block's last line, inclusive. May extend past the
    /// bottom of the visible area.
    pub end_row: i32,
    pub command: Option<String>,
    pub exit_code: Option<i32>,
    pub is_running: bool,
    /// Failed by Warp's rule: non-zero exit excluding SIGINT/SIGPIPE.
    pub failed: bool,
    pub duration_ms: Option<u64>,
}

/// Viewport-mapped snapshot of command blocks plus the live prompt region,
/// for Warp-style block decorations. Frontends fetch this per frame and only
/// receive blocks intersecting the viewport.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct BlockOverlay {
    pub blocks: Vec<BlockOverlayRegion>,
    /// Viewport row where the live input prompt begins (shell idle at a
    /// prompt). `None` while a command is running or when unknown.
    pub prompt_row: Option<i32>,
    /// Viewport row of the cursor, `None` when scrolled out of view.
    pub cursor_row: Option<i32>,
    pub rows: u16,
    /// True while the alternate screen (vim, htop, ...) is active; block
    /// decorations should not be drawn.
    pub alt_screen: bool,
    /// Whether any command block has been tracked at all this session. Lets
    /// the frontend tell a genuinely fresh shell (safe to blank the redundant
    /// prompt) from one where blocks merely scrolled out of view.
    pub has_blocks: bool,
}

/// Longest command text included in overlay regions; headers only need a
/// summary line, not multi-kilobyte scripts.
const OVERLAY_COMMAND_MAX_LEN: usize = 200;

impl SelectionKind {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Block,
            2 => Self::Semantic,
            3 => Self::Lines,
            _ => Self::Simple,
        }
    }
}

fn spawn_pty(pty_options: &PtyOptions, window_size: WindowSize) -> io::Result<tty::Pty> {
    let _guard = CHILD_ENV_LOCK
        .lock()
        .expect("child environment lock poisoned");
    let saved_env = save_and_remove_child_env();
    let result = tty::new(pty_options, window_size, 0);
    restore_child_env(saved_env);
    result
}

fn save_and_remove_child_env() -> Vec<(&'static str, Option<OsString>)> {
    let mut saved_env = Vec::with_capacity(FILTERED_CHILD_ENV_VARS.len());
    for key in FILTERED_CHILD_ENV_VARS {
        saved_env.push((*key, std::env::var_os(key)));
        // alacritty_terminal merges `PtyOptions.env` with the parent process
        // environment, so omitted keys must be removed before spawning.
        unsafe {
            std::env::remove_var(key);
        }
    }
    saved_env
}

fn restore_child_env(saved_env: Vec<(&'static str, Option<OsString>)>) {
    for (key, value) in saved_env {
        match value {
            Some(value) => unsafe {
                std::env::set_var(key, value);
            },
            None => unsafe {
                std::env::remove_var(key);
            },
        }
    }
}

// ---------------------------------------------------------------------------
// BackendMsg — commands sent to the PTY read thread
// ---------------------------------------------------------------------------

/// Messages sent from the main thread to the PTY read thread.
enum BackendMsg {
    Input(Vec<u8>),
    Resize {
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    },
    Shutdown,
}

fn flush_pending_input<W: Write>(writer: &mut W, pending: &mut VecDeque<u8>) -> io::Result<()> {
    while !pending.is_empty() {
        let (front, _) = pending.as_slices();
        if front.is_empty() {
            break;
        }

        match writer.write(front) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "failed to write PTY input",
                ));
            }
            Ok(written) => {
                pending.drain(..written);
            }
            Err(ref err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break,
            Err(err) => return Err(err),
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// TerminalBackend
// ---------------------------------------------------------------------------

const READ_THREAD_JOIN_TIMEOUT: Duration = Duration::from_secs(2);

/// The main terminal backend. One instance per terminal tab/split.
pub struct TerminalBackend {
    term: Arc<FairMutex<Term<EventProxy>>>,
    cmd_tx: Sender<BackendMsg>,
    poller: Arc<Poller>,
    event_rx: Receiver<TerminalEvent>,
    read_thread: Mutex<Option<JoinHandle<()>>>,
    cols: u16,
    rows: u16,
    cell_width: u16,
    cell_height: u16,
    colors: ConfiguredColors,
    /// Palette snapshot shared with the `EventProxy` for OSC color queries.
    query_colors: Arc<RwLock<[RgbColor; 269]>>,
    child_pid: u32,
    search: Mutex<TerminalSearch>,
    blocks: Arc<Mutex<CommandBlockTracker>>,
    history: Arc<Mutex<CommandHistoryStore>>,
    wakeup_pending: Arc<AtomicBool>,
    /// Forces the next `take_damage()` to report full damage. Set by state
    /// changes alacritty's damage tracker does not cover (selection, search
    /// highlights, colors, focus). Starts true so the first frame paints fully.
    force_full_damage: AtomicBool,
}

impl TerminalBackend {
    /// Create a new terminal backend and spawn a shell process.
    pub fn new(
        config: TerminalConfig,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    ) -> Result<Self, String> {
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<BackendMsg>();
        let wakeup_pending = Arc::new(AtomicBool::new(false));

        let colors = ConfiguredColors::from_config(&config);
        let query_colors = Arc::new(RwLock::new(colors.palette));
        let proxy = EventProxy {
            event_tx: event_tx.clone(),
            wakeup_pending: Arc::clone(&wakeup_pending),
            query_colors: Arc::clone(&query_colors),
        };

        let alac_config = config.to_alacritty_config();
        let pty_options = config.to_pty_options();

        let size = TermSize {
            columns: cols as usize,
            screen_lines: rows as usize,
        };
        let term = Term::new(alac_config, &size, proxy);
        let term = Arc::new(FairMutex::new(term));

        let window_size = WindowSize {
            num_lines: rows,
            num_cols: cols,
            cell_width,
            cell_height,
        };
        let pty = spawn_pty(&pty_options, window_size)
            .map_err(|e| format!("Failed to create PTY: {e}"))?;
        let child_pid = pty.child().id();
        let history_context = CommandHistoryContext {
            session_id: Some(format!("terminal:{child_pid}")),
            shell: shell_name_from_path(&config.shell_path),
            git_branch: None,
        };

        let poller =
            Arc::new(Poller::new().map_err(|e| format!("Failed to create PTY poller: {e}"))?);

        let blocks = Arc::new(Mutex::new(CommandBlockTracker::new()));
        let history = Arc::new(Mutex::new(CommandHistoryStore::new()));
        let term_clone = Arc::clone(&term);
        let blocks_clone = Arc::clone(&blocks);
        let history_clone = Arc::clone(&history);
        let wakeup_pending_clone = Arc::clone(&wakeup_pending);
        let poller_clone = Arc::clone(&poller);
        let max_scrollback = config.scrollback_lines;
        let read_thread = std::thread::Builder::new()
            .name("impulse-pty-reader".into())
            .spawn(move || {
                Self::read_loop(
                    pty,
                    term_clone,
                    event_tx,
                    cmd_rx,
                    poller_clone,
                    blocks_clone,
                    history_clone,
                    history_context,
                    wakeup_pending_clone,
                    max_scrollback,
                );
            })
            .map_err(|e| format!("Failed to spawn read thread: {e}"))?;

        Ok(Self {
            term,
            cmd_tx,
            poller,
            event_rx,
            read_thread: Mutex::new(Some(read_thread)),
            cols,
            rows,
            cell_width,
            cell_height,
            colors,
            query_colors,
            child_pid,
            search: Mutex::new(TerminalSearch::new()),
            blocks,
            history,
            wakeup_pending,
            force_full_damage: AtomicBool::new(true),
        })
    }

    /// The PTY read loop — runs on a dedicated thread.
    ///
    /// Reads bytes from the PTY, scans for OSC sequences, feeds data to
    /// alacritty's terminal state machine, and processes commands from the
    /// main thread (input, resize, shutdown).
    #[allow(clippy::too_many_arguments)]
    fn read_loop(
        mut pty: tty::Pty,
        term: Arc<FairMutex<Term<EventProxy>>>,
        event_tx: Sender<TerminalEvent>,
        cmd_rx: Receiver<BackendMsg>,
        poller: Arc<Poller>,
        blocks: Arc<Mutex<CommandBlockTracker>>,
        history: Arc<Mutex<CommandHistoryStore>>,
        history_context: CommandHistoryContext,
        wakeup_pending: Arc<AtomicBool>,
        max_scrollback: usize,
    ) {
        let mut buf = [0u8; 0x10000]; // 64KB read buffer
        let mut processor: Processor = Processor::new();
        let mut scanner = crate::osc_scanner::OscScanner::new();

        let mut pending_input: VecDeque<u8> = VecDeque::new();

        // Helper closure: process a single BackendMsg. Returns false on Shutdown.
        let handle_cmd =
            |msg: BackendMsg, pty: &mut tty::Pty, pending_input: &mut VecDeque<u8>| -> bool {
                match msg {
                    BackendMsg::Input(data) => {
                        pending_input.extend(data);
                        true
                    }
                    BackendMsg::Resize {
                        cols,
                        rows,
                        cell_width,
                        cell_height,
                    } => {
                        let ws = WindowSize {
                            num_lines: rows,
                            num_cols: cols,
                            cell_width,
                            cell_height,
                        };
                        pty.on_resize(ws);
                        let size = TermSize {
                            columns: cols as usize,
                            screen_lines: rows as usize,
                        };
                        term.lock().resize(size);
                        true
                    }
                    BackendMsg::Shutdown => false,
                }
            };

        let poll_opts = PollMode::Level;
        let mut interest = PollingEvent::readable(0);
        if let Err(err) = unsafe { pty.register(&poller, interest, poll_opts) } {
            log::error!("failed to register PTY poller: {err}");
            let _ = event_tx.send(TerminalEvent::Exit);
            return;
        }

        let mut events = Events::with_capacity(NonZeroUsize::new(1024).unwrap());
        let mut writable_registered = false;

        'event_loop: loop {
            // Drain all pending commands first (non-blocking).
            while let Ok(msg) = cmd_rx.try_recv() {
                if !handle_cmd(msg, &mut pty, &mut pending_input) {
                    break 'event_loop;
                }
            }

            if !pending_input.is_empty() {
                if let Err(err) = flush_pending_input(pty.writer(), &mut pending_input) {
                    log::warn!("failed to write PTY input: {err}");
                    pending_input.clear();
                }
            }

            let needs_write = !pending_input.is_empty();
            if needs_write != writable_registered {
                interest.writable = needs_write;
                if let Err(err) = pty.reregister(&poller, interest, poll_opts) {
                    log::error!("failed to update PTY poll interest: {err}");
                    let _ = event_tx.send(TerminalEvent::Exit);
                    break 'event_loop;
                }
                writable_registered = needs_write;
            }

            events.clear();
            if let Err(err) = poller.wait(&mut events, None) {
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                log::error!("PTY poll failed: {err}");
                let _ = event_tx.send(TerminalEvent::Exit);
                break 'event_loop;
            }

            // A command-channel notify wakes the poller with no events. Drain
            // commands immediately instead of waiting for the next PTY event.
            while let Ok(msg) = cmd_rx.try_recv() {
                if !handle_cmd(msg, &mut pty, &mut pending_input) {
                    break 'event_loop;
                }
            }

            if let Some(tty::ChildEvent::Exited(status)) = pty.next_child_event() {
                let code = status.and_then(|s| s.code()).unwrap_or(-1);
                let _ = event_tx.send(TerminalEvent::ChildExited(code));
                let _ = event_tx.send(TerminalEvent::Exit);
                break 'event_loop;
            }

            let mut readable = false;
            let mut writable = false;
            for event in events.iter() {
                if event.is_interrupt() {
                    continue;
                }
                readable |= event.readable;
                writable |= event.writable;
            }

            if writable && !pending_input.is_empty() {
                if let Err(err) = flush_pending_input(pty.writer(), &mut pending_input) {
                    log::warn!("failed to write PTY input: {err}");
                    pending_input.clear();
                }
            }

            if readable {
                loop {
                    // Read from PTY until it would block, then wait for the
                    // next readiness notification from the OS.
                    match pty.reader().read(&mut buf) {
                        Ok(0) => {
                            let _ = event_tx.send(TerminalEvent::Exit);
                            break 'event_loop;
                        }
                        Ok(n) => {
                            // Scan for OSC sequences, then use their offsets to capture
                            // command output without including shell prompt markers.
                            scanner.scan(&buf[..n]);
                            let osc_events = scanner.drain_event_spans();
                            let mut output_cursor = 0usize;
                            let mut advance_cursor = 0usize;
                            {
                                // Hold the term lock across the chunk so block row
                                // marks are recorded against the exact grid state at
                                // each shell-integration mark.
                                let mut term_locked = term.lock();
                                let hs_before = term_locked.grid().history_size();
                                let nl_before = blocks
                                    .lock()
                                    .map(|blocks| blocks.current_output_line())
                                    .unwrap_or(0);

                                for osc_event in osc_events {
                                    if let Some(start_offset) = osc_event.start_offset {
                                        let start_offset = start_offset.min(n);
                                        if start_offset > output_cursor {
                                            if let Ok(mut blocks) = blocks.lock() {
                                                blocks.observe_output(
                                                    &buf[output_cursor..start_offset],
                                                );
                                            }
                                        }
                                        // Advance the parser up to the mark so the
                                        // cursor row reflects the bytes preceding it.
                                        if start_offset > advance_cursor {
                                            processor.advance(
                                                &mut *term_locked,
                                                &buf[advance_cursor..start_offset],
                                            );
                                            advance_cursor = start_offset;
                                        }
                                    }

                                    match osc_event.event {
                                        crate::osc_scanner::OscEvent::CwdChanged(path) => {
                                            if let Ok(mut blocks) = blocks.lock() {
                                                blocks.set_cwd(path.clone());
                                            }
                                            let _ = event_tx.send(TerminalEvent::CwdChanged(path));
                                        }
                                        crate::osc_scanner::OscEvent::CommandText(command) => {
                                            if let Ok(mut blocks) = blocks.lock() {
                                                blocks.set_pending_command(command);
                                            }
                                        }
                                        crate::osc_scanner::OscEvent::PromptStart => {
                                            if let Ok(mut blocks) = blocks.lock() {
                                                let row = absolute_cursor_row(
                                                    &term_locked,
                                                    blocks.row_base(),
                                                );
                                                blocks.prompt_marked(row);
                                            }
                                            let _ = event_tx.send(TerminalEvent::PromptStart);
                                        }
                                        crate::osc_scanner::OscEvent::CommandStart => {
                                            if let Ok(mut blocks) = blocks.lock() {
                                                let row = absolute_cursor_row(
                                                    &term_locked,
                                                    blocks.row_base(),
                                                );
                                                let block = blocks.command_started(Some(row));
                                                let _ = event_tx.send(
                                                    TerminalEvent::CommandBlockStarted(block),
                                                );
                                            }
                                            let _ = event_tx.send(TerminalEvent::CommandStart);
                                        }
                                        crate::osc_scanner::OscEvent::CommandEnd(code) => {
                                            if let Ok(mut blocks) = blocks.lock() {
                                                let row = absolute_cursor_row(
                                                    &term_locked,
                                                    blocks.row_base(),
                                                );
                                                if let Some(block) =
                                                    blocks.command_ended(code, Some(row))
                                                {
                                                    if let Ok(mut history) = history.lock() {
                                                        history.record_completed_block(
                                                            &block,
                                                            history_context.clone(),
                                                        );
                                                    }
                                                    let _ = event_tx.send(
                                                        TerminalEvent::CommandBlockEnded(block),
                                                    );
                                                }
                                            }
                                            let _ = event_tx.send(TerminalEvent::CommandEnd(code));
                                        }
                                        crate::osc_scanner::OscEvent::AttentionRequest(value) => {
                                            let _ = event_tx
                                                .send(TerminalEvent::AttentionRequest(value));
                                        }
                                        crate::osc_scanner::OscEvent::Notification {
                                            title,
                                            body,
                                        } => {
                                            let _ = event_tx
                                                .send(TerminalEvent::Notification { title, body });
                                        }
                                    }

                                    output_cursor = output_cursor.max(osc_event.end_offset.min(n));
                                }
                                if output_cursor < n {
                                    if let Ok(mut blocks) = blocks.lock() {
                                        blocks.observe_output(&buf[output_cursor..n]);
                                    }
                                }
                                // Feed the remaining bytes to alacritty's state machine.
                                if advance_cursor < n {
                                    processor.advance(&mut *term_locked, &buf[advance_cursor..n]);
                                }

                                // At the scrollback cap alacritty evicts one history
                                // line per scrolled line and history_size() stops
                                // growing; estimate evictions from observed newlines
                                // so absolute block rows keep tracking the grid. The
                                // alternate screen never scrolls into history.
                                let hs_after = term_locked.grid().history_size();
                                if hs_after >= max_scrollback
                                    && !term_locked.mode().contains(TermMode::ALT_SCREEN)
                                {
                                    if let Ok(mut blocks) = blocks.lock() {
                                        let newlines =
                                            blocks.current_output_line().saturating_sub(nl_before);
                                        let grown = hs_after.saturating_sub(hs_before) as u64;
                                        let evicted = newlines.saturating_sub(grown);
                                        if evicted > 0 {
                                            blocks.bump_row_base(evicted);
                                        }
                                    }
                                }
                            }

                            // Notify frontend of content change.
                            send_wakeup(&event_tx, &wakeup_pending);
                        }
                        Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                        Err(err) => {
                            #[cfg(target_os = "linux")]
                            if err.raw_os_error() == Some(libc::EIO) {
                                let _ = event_tx.send(TerminalEvent::Exit);
                                break 'event_loop;
                            }

                            log::warn!("failed to read PTY: {err}");
                            let _ = event_tx.send(TerminalEvent::Exit);
                            break 'event_loop;
                        }
                    };
                }
            }
        }

        let _ = pty.deregister(&poller);
    }

    /// Send input bytes to the PTY.
    pub fn write(&self, data: &[u8]) {
        if !data.is_empty() {
            let _ = self.cmd_tx.send(BackendMsg::Input(data.to_vec()));
            let _ = self.poller.notify();
        }
    }

    /// Resize the terminal grid and PTY.
    pub fn resize(&mut self, cols: u16, rows: u16, cell_width: u16, cell_height: u16) {
        if cols == self.cols
            && rows == self.rows
            && cell_width == self.cell_width
            && cell_height == self.cell_height
        {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        self.cell_width = cell_width;
        self.cell_height = cell_height;
        // The term itself is resized asynchronously on the read thread (which
        // marks full damage); force full here too so a take_damage() in the
        // window before that lands doesn't under-report.
        self.mark_force_full_damage();
        let _ = self.cmd_tx.send(BackendMsg::Resize {
            cols,
            rows,
            cell_width,
            cell_height,
        });
        let _ = self.poller.notify();
    }

    /// Poll for terminal events (non-blocking).
    pub fn poll_events(&self) -> Vec<TerminalEvent> {
        let mut events = Vec::new();
        let mut emitted_wakeup = false;
        while let Ok(ev) = self.event_rx.try_recv() {
            match &ev {
                TerminalEvent::PtyWrite(text) => {
                    // PtyWrite responses need to go back to the PTY.
                    let _ = self
                        .cmd_tx
                        .send(BackendMsg::Input(text.as_bytes().to_vec()));
                    let _ = self.poller.notify();
                }
                TerminalEvent::Wakeup => {
                    self.wakeup_pending.store(false, Ordering::Release);
                    if !emitted_wakeup {
                        events.push(ev);
                        emitted_wakeup = true;
                    }
                }
                _ => events.push(ev),
            }
        }
        events
    }

    /// Return command block metadata observed for this terminal session.
    pub fn command_blocks(&self) -> Vec<TerminalCommandBlock> {
        self.blocks
            .lock()
            .map(|blocks| blocks.blocks())
            .unwrap_or_default()
    }

    /// Map command blocks into viewport rows for block-decoration rendering.
    ///
    /// Only blocks intersecting the viewport are returned. Returns an empty,
    /// `alt_screen = true` snapshot while the alternate screen is active.
    pub fn block_overlay(&self) -> BlockOverlay {
        let (mode, rows, display_offset, history_size, cursor_line) = {
            let term = self.term.lock();
            let grid = term.grid();
            (
                *term.mode(),
                term.screen_lines(),
                grid.display_offset() as i64,
                grid.history_size() as i64,
                i64::from(grid.cursor.point.line.0),
            )
        };

        let mut overlay = BlockOverlay {
            rows: rows.min(u16::MAX as usize) as u16,
            ..Default::default()
        };
        if mode.contains(TermMode::ALT_SCREEN) {
            overlay.alt_screen = true;
            return overlay;
        }

        let Ok(blocks) = self.blocks.lock() else {
            return overlay;
        };
        let base = blocks.row_base();
        let rows_i = rows as i64;
        // Viewport row of an absolute grid row, mirroring the cell mapping in
        // write_grid_to_buffer (grid line + display_offset).
        let to_viewport = |abs: i64| abs - base - history_size + display_offset;
        let cursor_abs = base + history_size + cursor_line;

        let cursor_row = cursor_line + display_offset;
        if (0..rows_i).contains(&cursor_row) {
            overlay.cursor_row = Some(cursor_row as i32);
        }
        if let Some(prompt_abs) = blocks.pending_prompt_row() {
            let row = to_viewport(prompt_abs);
            if row < rows_i {
                overlay.prompt_row = Some(row.clamp(i32::MIN as i64, i32::MAX as i64) as i32);
            }
        }

        let all: Vec<&TerminalCommandBlock> = blocks.iter_blocks().collect();
        overlay.has_blocks = !all.is_empty();

        // `clear` (and other full-screen erases) reset the cursor to the top
        // without growing scrollback, so a post-clear block can be assigned an
        // absolute row that collides with a stale, now-erased earlier block.
        // Block tops normally increase monotonically (each command sits below
        // the previous one); a non-increasing step marks a screen reset. Only
        // decorate the contiguous run since the last such reset — earlier
        // blocks are no longer on screen and would otherwise paint chips and
        // separators over unrelated output.
        let tops: Vec<Option<i64>> = all
            .iter()
            .map(|block| block.prompt_row.or(block.output_row))
            .collect();
        let mut decorate_from = 0usize;
        let mut last_top: Option<i64> = None;
        for (i, top) in tops.iter().enumerate() {
            if let Some(top) = top {
                if let Some(prev) = last_top {
                    if *top <= prev {
                        decorate_from = i;
                    }
                }
                last_top = Some(*top);
            }
        }

        for (i, block) in all.iter().enumerate().skip(decorate_from) {
            let Some(top_abs) = block.prompt_row.or(block.output_row) else {
                continue;
            };
            let next_top = all[i + 1..]
                .iter()
                .find_map(|next| next.prompt_row.or(next.output_row));
            let is_running = block.exit_code.is_none() && block.ended_at_ms.is_none();
            // The OSC 133;D mark lands on the fresh line after the output, so
            // a completed block's content ends one row above its end mark —
            // without this, no-output commands tint the blank line after them.
            let own_end = block.end_row.map(|end| end - 1);
            let end_abs = if is_running {
                cursor_abs
            } else if let Some(next_top) = next_top {
                own_end.unwrap_or(next_top - 1).min(next_top - 1)
            } else {
                // Last completed block: it also ends where the live prompt begins.
                let bound = match blocks.pending_prompt_row() {
                    Some(prompt_abs) if prompt_abs > top_abs => prompt_abs - 1,
                    _ => cursor_abs,
                };
                own_end.unwrap_or(bound).min(bound)
            };

            let start_row = to_viewport(top_abs);
            let end_row = to_viewport(end_abs.max(top_abs));
            if end_row < 0 || start_row >= rows_i {
                continue;
            }

            let failed = block
                .exit_code
                .is_some_and(|code| code != 0 && !NON_FAILURE_EXIT_CODES.contains(&code));
            let command = block.command.as_ref().map(|command| {
                let command = command.trim();
                if command.len() <= OVERLAY_COMMAND_MAX_LEN {
                    command.to_string()
                } else {
                    let mut end = OVERLAY_COMMAND_MAX_LEN;
                    while end > 0 && !command.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}…", &command[..end])
                }
            });

            overlay.blocks.push(BlockOverlayRegion {
                id: block.id.0,
                start_row: start_row.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
                end_row: end_row.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
                command,
                exit_code: block.exit_code,
                is_running,
                failed,
                duration_ms: block
                    .ended_at_ms
                    .map(|ended| ended.saturating_sub(block.started_at_ms)),
            });
        }

        overlay
    }

    /// Return lightweight command-block availability flags without cloning block output.
    pub fn command_block_flags(&self) -> CommandBlockFlags {
        self.blocks
            .lock()
            .map(|blocks| CommandBlockFlags {
                has_command: blocks.has_command_text(),
                has_output: blocks.has_output(),
                has_failed: blocks.has_failed_command(),
            })
            .unwrap_or_default()
    }

    /// Return completed command history records observed for this terminal session.
    pub fn command_history(&self) -> Vec<CommandHistoryRecord> {
        self.history
            .lock()
            .map(|history| history.records())
            .unwrap_or_default()
    }

    /// Return whether this terminal has any completed command history records.
    pub fn has_command_history(&self) -> bool {
        self.history
            .lock()
            .map(|history| !history.is_empty())
            .unwrap_or(false)
    }

    /// Search completed command history records observed for this terminal session.
    pub fn search_command_history(
        &self,
        query: &CommandHistoryQuery,
    ) -> Vec<CommandHistorySearchResult> {
        self.history
            .lock()
            .map(|history| history.search(query))
            .unwrap_or_default()
    }

    /// Rerun a stored command by writing it back to the interactive PTY.
    pub fn rerun_command(&self, command: &str) -> bool {
        let Some(input) = command_history_rerun_input(command) else {
            return false;
        };
        self.write(input.as_bytes());
        true
    }

    /// Write the visible grid into a pre-allocated binary buffer.
    /// Returns the number of bytes written.
    pub fn write_grid_to_buffer(&self, buf: &mut [u8]) -> usize {
        let term = self.term.lock();
        let content = term.renderable_content();
        let term_colors = content.colors;
        let mode = content.mode;
        let cursor = content.cursor;
        let num_cols = term.columns();
        let num_lines = term.screen_lines();
        // When the viewport is scrolled into history, `display_iter` emits
        // points with negative line numbers. Shift by `display_offset` so
        // each cell maps to a 0-based viewport row.
        let display_offset = term.grid().display_offset() as i32;

        let mode_flags = convert_mode(mode);

        // Cursor state.
        let cursor_row = cursor.point.line.0 + display_offset;
        let cursor_visible = mode.contains(TermMode::SHOW_CURSOR)
            && cursor_row >= 0
            && (cursor_row as usize) < num_lines;
        let cursor_state = CursorState {
            row: cursor_row.max(0) as usize,
            col: cursor.point.column.0,
            shape: match cursor.shape {
                AlacCursorShape::Block => CursorShape::Block,
                AlacCursorShape::Beam => CursorShape::Beam,
                AlacCursorShape::Underline => CursorShape::Underline,
                AlacCursorShape::HollowBlock => CursorShape::HollowBlock,
                AlacCursorShape::Hidden => CursorShape::Hidden,
            },
            visible: cursor_visible,
        };

        // Selection ranges.
        let mut selection_ranges = Vec::new();
        if let Some(sel) = &content.selection {
            let viewport_top = -display_offset;
            let viewport_bottom = num_lines as i32 - 1 - display_offset;
            let start_line = sel.start.line.0.max(viewport_top);
            let end_line = sel.end.line.0.min(viewport_bottom);
            if start_line <= end_line {
                for grid_line in start_line..=end_line {
                    let row = grid_line + display_offset;
                    if row < 0 {
                        continue;
                    }
                    let sc = if grid_line == sel.start.line.0 {
                        sel.start.column.0
                    } else {
                        0
                    };
                    let ec = if grid_line == sel.end.line.0 {
                        sel.end.column.0
                    } else {
                        num_cols.saturating_sub(1)
                    };
                    selection_ranges.push(HighlightRange {
                        row: row as u16,
                        start_col: sc as u16,
                        end_col: ec as u16,
                    });
                }
            }
        }

        let search_ranges = self
            .search
            .lock()
            .map(|mut s| s.visible_matches(&term))
            .unwrap_or_default();

        let required = buffer::buffer_size(
            num_cols as u16,
            num_lines as u16,
            selection_ranges.len() as u16,
            search_ranges.len() as u16,
        );
        if buf.len() < required {
            return 0;
        }

        // Write header.
        let cell_offset = buffer::write_header(
            buf,
            num_cols as u16,
            num_lines as u16,
            &cursor_state,
            mode_flags,
            &selection_ranges,
            &search_ranges,
        );

        // Initialize all cells to space with default colors.
        let default_fg = self.colors.foreground;
        let default_bg = self.colors.background;
        for i in 0..(num_cols * num_lines) {
            buffer::write_cell(
                buf,
                cell_offset + i * buffer::CELL_STRIDE,
                ' ',
                default_fg,
                default_bg,
                CellFlags::empty(),
            );
        }

        // Fill from display iterator.
        for indexed in content.display_iter {
            let row_i32 = indexed.point.line.0 + display_offset;
            let col = indexed.point.column.0;
            if row_i32 >= 0 && (row_i32 as usize) < num_lines && col < num_cols {
                let row = row_i32 as usize;
                let offset = cell_offset + (row * num_cols + col) * buffer::CELL_STRIDE;
                let bg = self.colors.resolve(indexed.cell.bg, term_colors);
                let fg = apply_minimum_contrast(
                    self.colors.resolve(indexed.cell.fg, term_colors),
                    bg,
                    self.colors.minimum_contrast,
                );
                let mut flags = convert_flags(indexed.cell.flags);
                if indexed.cell.hyperlink().is_some() {
                    flags |= CellFlags::HYPERLINK;
                }
                buffer::write_cell(buf, offset, indexed.cell.c, fg, bg, flags);
            }
        }

        required
    }

    /// Calculate the buffer size needed for a grid snapshot.
    pub fn grid_buffer_size(&self) -> usize {
        // Use self.cols/rows (updated synchronously in resize()) rather than
        // querying the term (which is resized asynchronously on the read
        // thread). This prevents a race where the buffer is allocated for the
        // old grid size, then write_grid_to_buffer() sees the new (larger)
        // grid and returns 0 because the buffer is too small.
        let lines = self.rows;
        let cols = self.cols;
        // Selection can produce at most one range per visible line. Search can
        // produce one range per visible cell for dense or single-character
        // queries, so reserve the viewport worst case.
        let search_ranges = cols.saturating_mul(lines);
        buffer::buffer_size(cols, lines, lines, search_ranges)
    }

    /// Take the damage accumulated since the last call and reset tracking.
    ///
    /// Returns `None` when the entire viewport must be repainted, or
    /// `Some(rows)` with the viewport row indices that changed. Selection,
    /// search highlights, color, and focus changes are not covered by
    /// alacritty's damage tracker, so the methods mutating that state set
    /// `force_full_damage` and this method honours it.
    pub fn take_damage(&self) -> Option<Vec<u16>> {
        let force_full = self.force_full_damage.swap(false, Ordering::Relaxed);
        let mut term = self.term.lock();
        let result = collect_term_damage(&mut term);
        if force_full {
            None
        } else {
            result
        }
    }

    /// Force the next `take_damage()` to report full damage.
    fn mark_force_full_damage(&self) {
        self.force_full_damage.store(true, Ordering::Relaxed);
    }

    /// Start a text selection.
    pub fn start_selection(&self, col: usize, row: usize, kind: SelectionKind) {
        self.mark_force_full_damage();
        let mut term = self.term.lock();
        let point = viewport_point(&term, col, row);
        let ty = match kind {
            SelectionKind::Simple => SelectionType::Simple,
            SelectionKind::Block => SelectionType::Block,
            SelectionKind::Semantic => SelectionType::Semantic,
            SelectionKind::Lines => SelectionType::Lines,
        };
        term.selection = Some(Selection::new(
            ty,
            point,
            alacritty_terminal::index::Side::Left,
        ));
    }

    /// Update the current selection endpoint.
    pub fn update_selection(&self, col: usize, row: usize) {
        self.mark_force_full_damage();
        let mut term = self.term.lock();
        let point = viewport_point(&term, col, row);
        if let Some(ref mut sel) = term.selection {
            sel.update(point, alacritty_terminal::index::Side::Right);
        }
    }

    /// Clear the current selection.
    pub fn clear_selection(&self) {
        self.mark_force_full_damage();
        self.term.lock().selection = None;
    }

    /// Select the entire terminal buffer, including scrollback.
    pub fn select_all(&self) {
        self.mark_force_full_damage();
        let mut term = self.term.lock();
        let grid = term.grid();
        if grid.columns() == 0 || grid.total_lines() == 0 {
            return;
        }

        let start = alacritty_terminal::index::Point::new(
            grid.topmost_line(),
            alacritty_terminal::index::Column(0),
        );
        let end = alacritty_terminal::index::Point::new(grid.bottommost_line(), grid.last_column());
        let mut selection = Selection::new(
            SelectionType::Simple,
            start,
            alacritty_terminal::index::Side::Left,
        );
        selection.update(end, alacritty_terminal::index::Side::Right);
        term.selection = Some(selection);
    }

    /// Get the selected text.
    pub fn selected_text(&self) -> Option<String> {
        self.term.lock().selection_to_string()
    }

    /// Scroll the viewport. Positive = up (towards history), negative = down.
    pub fn scroll(&self, delta: i32) {
        self.term
            .lock()
            .scroll_display(alacritty_terminal::grid::Scroll::Delta(delta));
    }

    /// Scroll the viewport to the bottom.
    pub fn scroll_to_bottom(&self) {
        self.term
            .lock()
            .scroll_display(alacritty_terminal::grid::Scroll::Bottom);
    }

    /// Scroll the viewport to the start of a command block.
    ///
    /// Uses the exact absolute-row mark recorded at the shell-integration
    /// prompt when available, falling back to the newline-counted estimate
    /// for blocks recorded without marks.
    pub fn scroll_to_command_block(&self, id: TerminalBlockId) -> bool {
        let (top_row, row_base, fallback) = match self.blocks.lock() {
            Ok(blocks) => (
                blocks.block_top_row(id),
                blocks.row_base(),
                blocks
                    .block_start_line(id)
                    .map(|start_line| (blocks.current_output_line(), start_line)),
            ),
            Err(_) => return false,
        };

        let mut term = self.term.lock();
        if let Some(abs) = top_row {
            // Viewport row 0 == abs requires display_offset = base + hs - abs.
            let history_size = term.grid().history_size() as i64;
            let delta = (row_base + history_size - abs).clamp(0, i32::MAX as i64) as i32;
            term.scroll_display(alacritty_terminal::grid::Scroll::Bottom);
            if delta > 0 {
                term.scroll_display(alacritty_terminal::grid::Scroll::Delta(delta));
            }
            return true;
        }

        let Some((current_line, start_line)) = fallback else {
            return false;
        };
        let delta = current_line.saturating_sub(start_line).min(i32::MAX as u64) as i32;
        term.scroll_display(alacritty_terminal::grid::Scroll::Bottom);
        if delta > 0 {
            term.scroll_display(alacritty_terminal::grid::Scroll::Delta(delta));
        }
        true
    }

    /// Update the terminal's color palette at runtime (for live theme changes).
    pub fn set_colors(&mut self, config: &TerminalConfig) {
        self.mark_force_full_damage();
        self.colors = ConfiguredColors::from_config(config);
        if let Ok(mut palette) = self.query_colors.write() {
            *palette = self.colors.palette;
        }
    }

    /// Return the hyperlink URI at the given grid cell, if any.
    /// Used by the frontend for hover/click handling on OSC 8 hyperlinks.
    pub fn hyperlink_at(&self, col: usize, row: usize) -> Option<String> {
        let term = self.term.lock();
        let grid = term.grid();
        if row >= grid.screen_lines() || col >= grid.columns() {
            return None;
        }
        let point = viewport_point(&term, col, row);
        let cell = &grid[point];
        cell.hyperlink().map(|h| h.uri().to_string())
    }

    /// Get the current terminal mode flags.
    pub fn mode(&self) -> TerminalMode {
        let mode = *self.term.lock().mode();
        convert_mode(mode)
    }

    /// Get the current terminal dimensions.
    pub fn dimensions(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }

    /// Get the PID of the child shell process.
    pub fn child_pid(&self) -> u32 {
        self.child_pid
    }

    /// Notify the terminal about focus change.
    ///
    /// When the PTY has DECSET 1004 (FOCUS_IN_OUT) enabled, emit the
    /// corresponding `\x1b[I` / `\x1b[O` sequence so TUIs like neovim and tmux
    /// can react to the application gaining or losing focus.
    pub fn set_focus(&self, focused: bool) {
        self.mark_force_full_damage();
        let emit = {
            let mut term = self.term.lock();
            term.is_focused = focused;
            term.mode().contains(TermMode::FOCUS_IN_OUT)
        };
        if emit {
            let seq: &[u8] = if focused { b"\x1b[I" } else { b"\x1b[O" };
            let _ = self.cmd_tx.send(BackendMsg::Input(seq.to_vec()));
            let _ = self.poller.notify();
        }
    }

    /// Shut down the terminal. The reader thread is joined when it exits
    /// promptly, but shutdown does not block the UI indefinitely if a child
    /// process or PTY close hangs.
    pub fn shutdown(&self) {
        let _ = self.cmd_tx.send(BackendMsg::Shutdown);
        let _ = self.poller.notify();
        let handle = self.read_thread.lock().ok().and_then(|mut h| h.take());
        if let Some(handle) = handle {
            let (done_tx, done_rx) = mpsc::channel();
            match std::thread::Builder::new()
                .name("impulse-pty-reader-join".into())
                .spawn(move || {
                    let _ = handle.join();
                    let _ = done_tx.send(());
                }) {
                Ok(joiner) => {
                    if done_rx.recv_timeout(READ_THREAD_JOIN_TIMEOUT).is_ok() {
                        let _ = joiner.join();
                    } else {
                        log::warn!("timed out waiting for PTY reader thread shutdown");
                    }
                }
                Err(err) => {
                    log::warn!("failed to spawn PTY reader joiner thread: {err}");
                }
            }
        }
    }

    /// Search for a regex pattern in the terminal. Returns the first match.
    pub fn search(&self, pattern: &str) -> SearchResult {
        self.mark_force_full_damage();
        let term = self.term.lock();
        let Ok(mut search) = self.search.lock() else {
            return SearchResult::no_match();
        };
        search.search(&term, pattern)
    }

    /// Find the next search match after the current one.
    pub fn search_next(&self) -> SearchResult {
        self.mark_force_full_damage();
        let term = self.term.lock();
        let Ok(mut search) = self.search.lock() else {
            return SearchResult::no_match();
        };
        search.search_next(&term)
    }

    /// Find the previous search match before the current one.
    pub fn search_prev(&self) -> SearchResult {
        self.mark_force_full_damage();
        let term = self.term.lock();
        let Ok(mut search) = self.search.lock() else {
            return SearchResult::no_match();
        };
        search.search_prev(&term)
    }

    /// Clear the current search state.
    pub fn search_clear(&self) {
        self.mark_force_full_damage();
        if let Ok(mut search) = self.search.lock() {
            search.clear();
        }
    }
}

impl Drop for TerminalBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn convert_mode(mode: TermMode) -> TerminalMode {
    let mut flags = TerminalMode::empty();
    if mode.contains(TermMode::SHOW_CURSOR) {
        flags |= TerminalMode::SHOW_CURSOR;
    }
    if mode.contains(TermMode::APP_CURSOR) {
        flags |= TerminalMode::APP_CURSOR;
    }
    if mode.contains(TermMode::APP_KEYPAD) {
        flags |= TerminalMode::APP_KEYPAD;
    }
    if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
        flags |= TerminalMode::MOUSE_REPORT_CLICK;
    }
    if mode.contains(TermMode::MOUSE_MOTION) {
        flags |= TerminalMode::MOUSE_MOTION;
    }
    if mode.contains(TermMode::MOUSE_DRAG) {
        flags |= TerminalMode::MOUSE_DRAG;
    }
    if mode.contains(TermMode::SGR_MOUSE) {
        flags |= TerminalMode::MOUSE_SGR;
    }
    if mode.contains(TermMode::BRACKETED_PASTE) {
        flags |= TerminalMode::BRACKETED_PASTE;
    }
    if mode.contains(TermMode::FOCUS_IN_OUT) {
        flags |= TerminalMode::FOCUS_IN_OUT;
    }
    if mode.contains(TermMode::ALT_SCREEN) {
        flags |= TerminalMode::ALT_SCREEN;
    }
    if mode.contains(TermMode::LINE_WRAP) {
        flags |= TerminalMode::LINE_WRAP;
    }
    flags
}

/// Convert a frontend viewport coordinate into alacritty's grid coordinate.
fn viewport_point<T>(term: &Term<T>, col: usize, row: usize) -> alacritty_terminal::index::Point {
    let display_offset = term.grid().display_offset() as i32;
    alacritty_terminal::index::Point::new(
        alacritty_terminal::index::Line(row as i32 - display_offset),
        alacritty_terminal::index::Column(col),
    )
}

fn shell_name_from_path(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

/// Strip ASCII control characters (except tab and newline) from a shell-set
/// title so embedded NULs don't silently truncate strings on the Swift side.
fn sanitize_title(title: &str) -> String {
    title
        .chars()
        .filter(|c| *c == '\t' || *c == '\n' || (*c >= ' ' && *c != '\x7f'))
        .collect()
}

/// Convert alacritty cell flags to our CellFlags.
fn convert_flags(flags: AlacFlags) -> CellFlags {
    let mut result = CellFlags::empty();
    if flags.contains(AlacFlags::BOLD) {
        result |= CellFlags::BOLD;
    }
    if flags.contains(AlacFlags::ITALIC) {
        result |= CellFlags::ITALIC;
    }
    if flags.contains(AlacFlags::UNDERLINE) {
        result |= CellFlags::UNDERLINE;
    }
    if flags.contains(AlacFlags::STRIKEOUT) {
        result |= CellFlags::STRIKETHROUGH;
    }
    if flags.contains(AlacFlags::DIM) {
        result |= CellFlags::DIM;
    }
    if flags.contains(AlacFlags::INVERSE) {
        result |= CellFlags::INVERSE;
    }
    if flags.contains(AlacFlags::HIDDEN) {
        result |= CellFlags::HIDDEN;
    }
    if flags.contains(AlacFlags::WIDE_CHAR) {
        result |= CellFlags::WIDE_CHAR;
    }
    if flags.contains(AlacFlags::WIDE_CHAR_SPACER) {
        result |= CellFlags::WIDE_CHAR_SPACER;
    }
    if flags.contains(AlacFlags::DOUBLE_UNDERLINE) {
        result |= CellFlags::DOUBLE_UNDERLINE;
    }
    if flags.contains(AlacFlags::UNDERCURL) {
        result |= CellFlags::UNDERCURL;
    }
    if flags.contains(AlacFlags::DOTTED_UNDERLINE) {
        result |= CellFlags::DOTTED_UNDERLINE;
    }
    if flags.contains(AlacFlags::DASHED_UNDERLINE) {
        result |= CellFlags::DASHED_UNDERLINE;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{
        apply_minimum_contrast, collect_term_damage, contrast_ratio, flush_pending_input,
        send_wakeup, ConfiguredColors, EventProxy, TermSize,
    };
    use crate::config::{TerminalColors, TerminalConfig};
    use crate::event::TerminalEvent;
    use crate::grid::RgbColor;
    use alacritty_terminal::event::{Event as AlacEvent, EventListener};
    use alacritty_terminal::vte::ansi::NamedColor;
    use std::collections::VecDeque;
    use std::io::{self, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, RwLock};

    struct FlakyWriter {
        output: Vec<u8>,
        calls: usize,
    }

    impl FlakyWriter {
        fn new() -> Self {
            Self {
                output: Vec::new(),
                calls: 0,
            }
        }
    }

    impl Write for FlakyWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            match self.calls {
                1 => {
                    let count = buf.len().min(3);
                    self.output.extend_from_slice(&buf[..count]);
                    Ok(count)
                }
                2 => Err(io::Error::from(io::ErrorKind::WouldBlock)),
                3 => Err(io::Error::from(io::ErrorKind::Interrupted)),
                _ => {
                    let count = buf.len().min(4);
                    self.output.extend_from_slice(&buf[..count]);
                    Ok(count)
                }
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct ZeroWriter;

    impl Write for ZeroWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Ok(0)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn flush_pending_input_preserves_bytes_across_would_block() {
        let mut writer = FlakyWriter::new();
        let mut pending = VecDeque::from(Vec::from(&b"large paste payload"[..]));

        flush_pending_input(&mut writer, &mut pending).unwrap();
        assert_eq!(writer.output, b"lar");
        assert_eq!(pending.len(), b"ge paste payload".len());

        flush_pending_input(&mut writer, &mut pending).unwrap();
        assert_eq!(writer.output, b"large paste payload");
        assert!(pending.is_empty());
    }

    #[test]
    fn flush_pending_input_reports_write_zero() {
        let mut writer = ZeroWriter;
        let mut pending = VecDeque::from(Vec::from(&b"payload"[..]));
        let err = flush_pending_input(&mut writer, &mut pending).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WriteZero);
    }

    #[test]
    fn send_wakeup_coalesces_until_pending_flag_is_cleared() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let pending = AtomicBool::new(false);

        send_wakeup(&tx, &pending);
        send_wakeup(&tx, &pending);

        assert_eq!(rx.try_iter().count(), 1);
        assert!(pending.load(Ordering::Acquire));

        pending.store(false, Ordering::Release);
        send_wakeup(&tx, &pending);

        assert_eq!(rx.try_iter().count(), 1);
    }

    #[test]
    fn contrast_ratio_extremes() {
        let black = RgbColor::new(0, 0, 0);
        let white = RgbColor::new(255, 255, 255);
        assert!((contrast_ratio(black, white) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(white, white) - 1.0).abs() < 0.01);
    }

    #[test]
    fn minimum_contrast_fixes_dark_on_dark() {
        // The lazygit case: Harbor's dark green text on its indigo selection
        // bar (~1.2:1) must be raised to at least the configured ratio.
        let green = RgbColor::new(0x09, 0x5c, 0x34);
        let indigo = RgbColor::new(0x39, 0x59, 0xa6);
        let adjusted = apply_minimum_contrast(green, indigo, 3.0);
        assert!(contrast_ratio(adjusted, indigo) >= 3.0);
    }

    #[test]
    fn minimum_contrast_preserves_compliant_and_hidden_text() {
        let ink = RgbColor::new(0x22, 0x29, 0x35);
        let cream = RgbColor::new(0xf8, 0xfa, 0xfd);
        // Already compliant — untouched.
        assert_eq!(apply_minimum_contrast(ink, cream, 4.5), ink);
        // fg == bg is intentional hiding — untouched.
        assert_eq!(apply_minimum_contrast(cream, cream, 4.5), cream);
        // Ratio 1.0 disables the feature.
        let red = RgbColor::new(200, 60, 60);
        assert_eq!(apply_minimum_contrast(red, cream, 1.0), red);
    }

    #[test]
    fn color_request_replies_with_configured_palette() {
        let config = TerminalConfig::default();
        let colors = ConfiguredColors::from_config(&config);
        let (tx, rx) = crossbeam_channel::unbounded();
        let proxy = EventProxy {
            event_tx: tx,
            wakeup_pending: Arc::new(AtomicBool::new(false)),
            query_colors: Arc::new(RwLock::new(colors.palette)),
        };

        // OSC 11 (background) arrives as a request for NamedColor::Background.
        let index = NamedColor::Background as usize;
        proxy.send_event(AlacEvent::ColorRequest(
            index,
            Arc::new(|rgb| format!("11;rgb:{:02x}/{:02x}/{:02x}", rgb.r, rgb.g, rgb.b)),
        ));

        let bg = config.colors.background;
        match rx.try_recv() {
            Ok(TerminalEvent::PtyWrite(s)) => {
                assert_eq!(s, format!("11;rgb:{:02x}/{:02x}/{:02x}", bg.r, bg.g, bg.b));
            }
            other => panic!("expected PtyWrite, got {other:?}"),
        }
    }

    #[test]
    fn configured_colors_dims_bright_palette_without_overflow() {
        let config = TerminalConfig {
            colors: TerminalColors {
                foreground: RgbColor::new(255, 255, 255),
                background: RgbColor::new(0, 0, 0),
                palette: [RgbColor::new(255, 240, 224); 16],
            },
            ..TerminalConfig::default()
        };

        let colors = ConfiguredColors::from_config(&config);

        let dim_red = alacritty_terminal::vte::ansi::NamedColor::DimBlack as usize + 1;
        assert_eq!(colors.palette[dim_red].r, 191);
        assert_eq!(colors.palette[dim_red].g, 180);
        assert_eq!(colors.palette[dim_red].b, 168);
    }

    #[test]
    fn collect_term_damage_reports_written_rows_and_resets() {
        use alacritty_terminal::event::VoidListener;
        use alacritty_terminal::term::{Config, Term};
        use alacritty_terminal::vte::ansi::Processor;

        let size = TermSize {
            columns: 20,
            screen_lines: 5,
        };
        let mut term = Term::new(Config::default(), &size, VoidListener);

        // A fresh term starts fully damaged.
        assert!(collect_term_damage(&mut term).is_none());

        // After a reset, plain output damages only the written row (plus the
        // cursor row, which is the same here).
        let mut processor: Processor = Processor::new();
        processor.advance(&mut term, b"hello");
        let rows = collect_term_damage(&mut term).expect("expected partial damage");
        assert!(rows.contains(&0), "row 0 should be damaged, got {rows:?}");
        assert!(rows.iter().all(|&r| r < 5));

        // Damage was reset: with no changes, only the cursor row reports.
        let rows = collect_term_damage(&mut term).expect("expected partial damage");
        assert!(rows.iter().all(|&r| r < 5));
    }
}
