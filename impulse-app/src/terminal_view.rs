//! A terminal pane: `impulse-terminal` drives the PTY and grid, and this
//! paints it the way `TerminalRenderer.swift` does: cell backgrounds in
//! runs, text on a fixed cell grid, the selection and search matches, the
//! cursor in the shape the settings give it, and command-block
//! decorations (a hairline above each block, a wash and left stripe for
//! failed or running ones, exit status and duration at the right of the
//! block's first row). Keys go through `impulse_kit::key_encoder`; typed
//! text and input methods through GPUI's text input; the mouse selects,
//! scrolls history, or reports to programs that ask for it.
//!
//! Around the grid: the input bar (`terminal_input_bar`), which stands in
//! for the shell's own prompt, so the prompt isn't drawn and the last
//! output sits against the bar; find (`terminal_find_bar`); and command
//! blocks, hints and links (`terminal_blocks`).

use std::cell::RefCell;
use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::{
    actions, canvas, div, fill, point, prelude::*, px, size, AnyElement, App, Bounds,
    ClipboardItem, Context, ElementInputHandler, Entity, EntityId, EntityInputHandler, FocusHandle,
    Font, FontStyle, FontWeight, Hsla, KeyBinding, KeyDownEvent, Modifiers, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Rgba, ScrollDelta,
    ScrollWheelEvent, ShapedLine, SharedString, TextAlign, TextRun, UTF16Selection, UnderlineStyle,
    Window,
};
use impulse_kit::chrome::{ChromePalette, Rgba as KitRgba};
use impulse_kit::key_encoder::{self, KeyPress};
use impulse_kit::kitty_keyboard;
use impulse_kit::shell::lookup::CommandLookup;
use impulse_kit::shell::ShellType;
use impulse_kit::terminal_find::TerminalFindQuery;
use impulse_kit::theme::ResolvedTheme;
use impulse_terminal::{
    BlockOverlay, CellFlags, RgbColor, SelectionKind, TerminalBackend, TerminalColors,
    TerminalConfig, TerminalEvent, TerminalMode, CELL_STRIDE, EXTRA_IMAGE_CELL,
    EXTRA_UNDERLINE_COLOR, EXTRA_ZERO_WIDTH, FIXED_HEADER_SIZE, RANGE_ENTRY_SIZE,
};

use crate::command_editor::{CommandEditor, EditorColors};
use crate::command_history::CommandHistory;
use crate::find_bar::FindBar;
use crate::repository_state::RepositoryStore;
use crate::settings_store::SettingsStore;
use crate::terminal_blocks::{
    Blocks, FileChecks, Hints, HoverLink, LinkTarget, RowText, ToolbarButton,
};
use crate::terminal_images::{self, FrameImages, ImageCell, Textures};
use crate::terminal_input_bar::InputBar;

/// Same inset as the Swift renderer's `padding`.
const PADDING: f32 = 12.0;
/// How long the pointer rests on a link before its target shows (a
/// system tooltip's delay).
const LINK_TIP_DELAY: std::time::Duration = std::time::Duration::from_millis(700);
/// The rule under replayed output from a previous session.
const RESTORED_RULE: &str = "restored from the previous session";

/// The columns and rows that fit in `area` with the grid's padding.
fn grid_size(area: gpui_kit::Size<Pixels>, m: Metrics) -> (u16, u16) {
    let cols = ((f32::from(area.width) - PADDING * 2.0) / f32::from(m.width))
        .floor()
        .max(2.0) as u16;
    let rows = ((f32::from(area.height) - PADDING * 2.0) / f32::from(m.height))
        .floor()
        .max(1.0) as u16;
    (cols, rows)
}

/// A transcript without the "restored" rule it ends with when no command
/// ran since it was restored, so restoring it again doesn't stack rules.
fn without_rule(transcript: &str) -> &str {
    let body = transcript.trim_end_matches(['\r', '\n']);
    match body.rfind('\n') {
        Some(at) if body[at..].contains(RESTORED_RULE) => &transcript[..at + 1],
        None if body.contains(RESTORED_RULE) => "",
        _ => transcript,
    }
}

actions!(
    terminal,
    [
        Copy,
        Paste,
        SelectAll,
        Clear,
        ScrollPageUp,
        ScrollPageDown,
        ScrollToTop,
        ScrollToBottom
    ]
);

/// The key context of a terminal pane.
pub const CONTEXT: &str = "Terminal";

pub fn bind_keys(cx: &mut App) {
    let c = Some(CONTEXT);
    let mut bindings = vec![
        KeyBinding::new("shift-pageup", ScrollPageUp, c),
        KeyBinding::new("shift-pagedown", ScrollPageDown, c),
        KeyBinding::new("shift-home", ScrollToTop, c),
        KeyBinding::new("shift-end", ScrollToBottom, c),
        // Tab is the shell's (the window would move focus with it).
        KeyBinding::new("tab", gpui_kit::NoAction, c),
        KeyBinding::new("shift-tab", gpui_kit::NoAction, c),
    ];
    // Copy and Paste are bound with the commands, at the keys the settings
    // give them (`commands::command_bindings`).
    if cfg!(target_os = "macos") {
        bindings.extend([
            KeyBinding::new("cmd-a", SelectAll, c),
            KeyBinding::new("cmd-k", Clear, c),
        ]);
    } else {
        // No Clear key: Ctrl+L is the shell's own, and Ctrl+Shift+K is
        // Bookmark Block's.
        bindings.extend([KeyBinding::new("ctrl-shift-a", SelectAll, c)]);
    }
    cx.bind_keys(bindings);
}

/// A process's working directory, for shells that don't say it (Swift's
/// `TerminalBackend.queryCwd`).
#[cfg(target_os = "linux")]
fn process_cwd(pid: u32) -> Option<String> {
    (pid > 0)
        .then(|| std::fs::read_link(format!("/proc/{pid}/cwd")).ok())
        .flatten()
        .map(|path| path.to_string_lossy().into_owned())
}

#[cfg(target_os = "macos")]
fn process_cwd(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    // SAFETY: proc_pidinfo fills at most `size` bytes of `info`, which is
    // plain data; the path is a NUL-terminated C string inside it.
    unsafe {
        let mut info: libc::proc_vnodepathinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as i32;
        let filled = libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            (&mut info as *mut libc::proc_vnodepathinfo).cast(),
            size,
        );
        if filled != size {
            return None;
        }
        let path = std::ffi::CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr().cast());
        let path = path.to_string_lossy().into_owned();
        (!path.is_empty()).then_some(path)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn process_cwd(_: u32) -> Option<String> {
    None
}

/// Whether Alt (Option) is Meta for the key being handled, by
/// `terminal_option_as_meta` and, on the Mac, the side held.
fn alt_meta(cx: &App) -> bool {
    key_encoder::alt_is_meta_held(
        &SettingsStore::get(cx).terminal_option_as_meta,
        cfg!(target_os = "macos"),
        option_sides(),
    )
}

/// The Option keys held now (left, right), from AppKit's device-dependent
/// modifier flags: GPUI's modifiers don't say which side.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
fn option_sides() -> Option<(bool, bool)> {
    use objc::runtime::Class;
    use objc::{msg_send, sel, sel_impl};
    const LEFT_OPTION: u64 = 0x20; // NX_DEVICELALTKEYMASK
    const RIGHT_OPTION: u64 = 0x40; // NX_DEVICERALTKEYMASK
    let class = Class::get("NSEvent")?;
    // SAFETY: a class property read; NSEventModifierFlags is NSUInteger.
    let flags: u64 = unsafe { msg_send![class, modifierFlags] };
    Some((flags & LEFT_OPTION != 0, flags & RIGHT_OPTION != 0))
}

#[cfg(not(target_os = "macos"))]
fn option_sides() -> Option<(bool, bool)> {
    None
}

/// One mouse report (Swift's `mouseReport`): SGR (`CSI < Cb ; Cx ; Cy M`,
/// `m` for a release) or the normal form (`CSI M Cb Cx Cy`, each value plus
/// 32, a release as button 3, positions past 223 clamped).
/// What a left press starts.
#[derive(Debug)]
enum Press {
    /// A plain click: a selection of this kind once the drag leaves the
    /// cell.
    Pending(SelectionKind),
    /// A double or triple click: its word or line, selected now.
    Now(SelectionKind),
}

/// Swift's `mouseDown`: double and triple clicks select at once; a plain
/// click (Option for a block) waits for a drag.
fn press_selection(click_count: usize, alt: bool) -> Press {
    match click_count {
        2 => Press::Now(SelectionKind::Semantic),
        n if n >= 3 => Press::Now(SelectionKind::Lines),
        _ if alt => Press::Pending(SelectionKind::Block),
        _ => Press::Pending(SelectionKind::Simple),
    }
}

/// A pending selection starts once the drag reaches another cell.
fn drag_begins(anchor: (usize, usize), cell: (usize, usize)) -> bool {
    anchor != cell
}

/// Wheel scroll in lines: a mouse wheel's notches go 3 lines each, as in
/// Swift's `scrollWheel`; a trackpad's pixels go by the cell height.
fn wheel_lines(delta: ScrollDelta, cell_height: Pixels) -> f32 {
    match delta {
        ScrollDelta::Lines(delta) => delta.y * 3.0,
        ScrollDelta::Pixels(delta) => f32::from(delta.y) / f32::from(cell_height),
    }
}

fn mouse_report(button: u8, col: usize, row: usize, release: bool, sgr: bool) -> Vec<u8> {
    if sgr {
        let end = if release { 'm' } else { 'M' };
        return format!("\x1b[<{button};{};{}{end}", col + 1, row + 1).into_bytes();
    }
    let code = if release { 3 } else { button };
    vec![
        0x1b,
        b'[',
        b'M',
        code.saturating_add(32),
        (col + 33).min(255) as u8,
        (row + 33).min(255) as u8,
    ]
}

/// What a grid snapshot was read at: the scroll place and the size.
type GridKey = (usize, (u16, u16));

pub fn rgba(c: KitRgba) -> Rgba {
    Rgba {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
}

fn cell_hsla(c: RgbColor) -> Hsla {
    Rgba {
        r: c.r as f32 / 255.0,
        g: c.g as f32 / 255.0,
        b: c.b as f32 / 255.0,
        a: 1.0,
    }
    .into()
}

fn rgb_color(hex: &str) -> RgbColor {
    let c = KitRgba::from_hex(hex);
    RgbColor::new(
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
    )
}

/// The most an image file's preview reads (`image_peek`).
const MAX_PEEK_BYTES: u64 = 20 * 1024 * 1024;
/// The largest an image preview shows.
const PEEK_SIZE: (f32, f32) = (320., 240.);

/// A hovered image path's preview: where the pointer was, the file and
/// the size it shows at.
#[derive(Clone)]
struct ImagePeek {
    at: Point<Pixels>,
    path: PathBuf,
    size: gpui_kit::Size<Pixels>,
}

/// Cmd on the Mac, Ctrl elsewhere: the modifier that opens links.
fn secondary(modifiers: &Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        modifiers.platform
    } else {
        modifiers.control
    }
}

/// One visible cell, decoded from the grid snapshot.
#[derive(Clone, Copy)]
pub(crate) struct Cell {
    ch: char,
    fg: RgbColor,
    bg: RgbColor,
    flags: CellFlags,
    /// Its underline's own color (SGR 58), from the snapshot's extras.
    underline: Option<RgbColor>,
}

/// A highlighted span of one row, columns inclusive.
#[derive(Clone, Copy)]
struct Span {
    row: usize,
    start: usize,
    end: usize,
}

/// The grid snapshot `write_grid_to_buffer` fills (the same bytes the Swift
/// `GridBufferReader` parses).
pub(crate) struct Grid {
    pub cols: usize,
    pub rows: usize,
    cursor: Option<(usize, usize, u8)>,
    /// Where the cursor is, shown or not (input methods compose there).
    cursor_at: (usize, usize),
    selection: Vec<Span>,
    matches: Vec<Span>,
    cells: Vec<Cell>,
    /// Combining marks and other zero-width characters after a cell's own
    /// (by cell index), from the snapshot's extras.
    zero_width: std::collections::HashMap<usize, String>,
    /// Blank cells that show part of an image (by cell index).
    images: std::collections::HashMap<usize, ImageCell>,
}

impl Grid {
    fn read(buf: &[u8]) -> Option<Grid> {
        if buf.len() < FIXED_HEADER_SIZE {
            return None;
        }
        let u16_at = |at: usize| u16::from_le_bytes([buf[at], buf[at + 1]]) as usize;
        let (cols, rows) = (u16_at(0), u16_at(2));
        let (cursor_row, cursor_col, shape, visible) = (u16_at(4), u16_at(6), buf[8], buf[9] != 0);
        let (selections, searches) = (u16_at(12), u16_at(14));
        let span = |i: usize| {
            let at = FIXED_HEADER_SIZE + i * RANGE_ENTRY_SIZE;
            Span {
                row: u16_at(at),
                start: u16_at(at + 2),
                end: u16_at(at + 4),
            }
        };
        let start = FIXED_HEADER_SIZE + (selections + searches) * RANGE_ENTRY_SIZE;
        if buf.len() < start + cols * rows * CELL_STRIDE {
            return None;
        }
        let cells = (0..cols * rows)
            .map(|i| {
                let at = start + i * CELL_STRIDE;
                let code = u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]]);
                Cell {
                    ch: char::from_u32(code).unwrap_or(' '),
                    fg: RgbColor::new(buf[at + 4], buf[at + 5], buf[at + 6]),
                    bg: RgbColor::new(buf[at + 7], buf[at + 8], buf[at + 9]),
                    flags: CellFlags::from_bits_truncate(u16::from_le_bytes([
                        buf[at + 10],
                        buf[at + 11],
                    ])),
                    underline: None,
                }
            })
            .collect::<Vec<_>>();
        let mut cells = cells;
        // The extras after the cells: a count, then (cell, kind, length,
        // bytes) entries.
        let mut zero_width = std::collections::HashMap::new();
        let mut images = std::collections::HashMap::new();
        let mut at = start + cols * rows * CELL_STRIDE;
        let count = buf
            .get(at..at + 4)
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        at += 4;
        for _ in 0..count {
            let Some(head) = buf.get(at..at + 6) else {
                break;
            };
            let index = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
            let (kind, len) = (head[4], head[5] as usize);
            let Some(payload) = buf.get(at + 6..at + 6 + len) else {
                break;
            };
            at += 6 + len;
            match kind {
                EXTRA_ZERO_WIDTH if index < cells.len() => {
                    if let Ok(text) = std::str::from_utf8(payload) {
                        zero_width.insert(index, text.to_string());
                    }
                }
                EXTRA_UNDERLINE_COLOR if index < cells.len() && len == 3 => {
                    cells[index].underline =
                        Some(RgbColor::new(payload[0], payload[1], payload[2]));
                }
                EXTRA_IMAGE_CELL if index < cells.len() => {
                    if let Some(cell) = ImageCell::read(payload) {
                        images.insert(index, cell);
                    }
                }
                _ => {}
            }
        }
        Some(Grid {
            cols,
            rows,
            cursor: visible.then_some((cursor_row, cursor_col, shape)),
            cursor_at: (cursor_row, cursor_col),
            selection: (0..selections).map(span).collect(),
            matches: (selections..selections + searches).map(span).collect(),
            cells,
            zero_width,
            images,
        })
    }

    /// What the cell at `row`, `col` shows of an image, if it's a
    /// placeholder.
    fn image_cell(&self, row: usize, col: usize) -> Option<ImageCell> {
        self.images.get(&(row * self.cols + col)).copied()
    }

    /// The ids of the images placeholder cells show.
    fn image_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.images.values().map(|cell| cell.image)
    }

    /// The zero-width characters drawn with the cell at `row`, `col`.
    fn combining(&self, row: usize, col: usize) -> Option<&str> {
        self.zero_width
            .get(&(row * self.cols + col))
            .map(String::as_str)
    }

    fn cell(&self, row: usize, col: usize) -> &Cell {
        &self.cells[row * self.cols + col]
    }

    fn row_cells(&self, row: usize) -> &[Cell] {
        &self.cells[row * self.cols..(row + 1) * self.cols]
    }

    /// No visible glyph on `row` (only spaces and NULs).
    fn is_blank_row(&self, row: usize) -> bool {
        (0..self.cols).all(|col| {
            let cell = self.cell(row, col);
            (cell.flags.contains(CellFlags::WIDE_CHAR_SPACER) || cell.ch == ' ' || cell.ch == '\0')
                && (self.images.is_empty() || self.image_cell(row, col).is_none())
        })
    }

    /// The last row at or above `up_to` with a glyph on it.
    /// The last row up to `up_to` with something on it: text, or an image
    /// (`covered`, the rows under direct placements).
    fn last_non_blank_row(&self, up_to: i32, covered: &[bool]) -> i32 {
        let mut row = up_to.min(self.rows as i32 - 1);
        while row >= 0
            && self.is_blank_row(row as usize)
            && !covered.get(row as usize).copied().unwrap_or(false)
        {
            row -= 1;
        }
        row
    }

    /// Row `row` as text (NUL reads as a space; wide-character spacers
    /// are skipped).
    pub fn row_text(&self, row: usize) -> RowText {
        let mut text = String::new();
        let mut col_for_byte = Vec::new();
        let mut byte_for_col = Vec::new();
        let mut hyperlink = Vec::new();
        for col in 0..self.cols {
            let cell = self.cell(row, col);
            hyperlink.push(cell.flags.contains(CellFlags::HYPERLINK));
            if cell.flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                byte_for_col.push(None);
                continue;
            }
            byte_for_col.push(Some(text.len()));
            let ch = if cell.ch == '\0' { ' ' } else { cell.ch };
            text.push(ch);
            col_for_byte.extend(std::iter::repeat_n(col, ch.len_utf8()));
            if let Some(marks) = self.combining(row, col) {
                text.push_str(marks);
                col_for_byte.extend(std::iter::repeat_n(col, marks.len()));
            }
        }
        RowText {
            text,
            col_for_byte,
            byte_for_col,
            hyperlink,
        }
    }
}

/// Cell size and where text sits in a cell.
#[derive(Clone, Copy)]
struct Metrics {
    width: Pixels,
    height: Pixels,
    font_size: Pixels,
    /// The baseline is `ascent` below a row's top (Swift's `drawTextRun`),
    /// the font's leading below the descent.
    ascent: Pixels,
    descent: Pixels,
}

/// Hiding the shell's prompt below the last output once a command ends
/// (the Swift renderer's prompt tuck): with a screenful of output the grid
/// scrolls up until the last output meets the input bar, and scrolling
/// back down stops there.
#[derive(Default)]
struct Tuck {
    /// Armed when a command finishes; done once the prompt is out of view.
    armed: bool,
    pending: bool,
    scrolled: bool,
    /// The display offset scrolling down stops at, at an idle prompt.
    floor: Option<usize>,
}

pub struct TerminalView {
    pub(crate) backend: TerminalBackend,
    /// The grid's keyboard focus.
    pub(crate) focus: FocusHandle,
    /// The whole pane's: the grid, the input bar and the find bar.
    root_focus: FocusHandle,
    pub(crate) entity_id: EntityId,
    pub(crate) theme: ResolvedTheme,
    pub(crate) palette: ChromePalette,
    font: Font,
    metrics: Metrics,
    size: (u16, u16),
    /// The grid was sized to its pane (see `fit_pane`).
    pub(crate) fitted: bool,
    buffer: Vec<u8>,
    /// The last grid snapshot, with the scroll place and size it was read
    /// at; read again only after damage (output, selection, search).
    grid_cache: Option<(Rc<Grid>, GridKey)>,
    grid_dirty: bool,
    /// The images programs show, as GPU textures.
    textures: Textures,
    /// Damage a snapshot took before the wake handler saw it.
    damage_seen: bool,
    /// The block overlay by `block_overlay_key`.
    overlay_cache: RefCell<Option<(u64, BlockOverlay)>>,
    pub title: String,
    pub cwd: String,
    pub last_duration_ms: Option<u64>,
    /// The shell's exit status, once it has exited.
    pub exited: Option<i32>,
    /// A drag is selecting text (rather than reporting to the program).
    selecting: bool,
    /// Where a plain click pressed: a selection starts there only once a
    /// drag reaches another cell (Swift's `pendingSelectionAnchor`).
    pending_selection: Option<((usize, usize), SelectionKind)>,
    /// The button whose press went to the program (its drags and release
    /// go there too).
    press_reported: Option<u8>,
    /// The last cell a reported drag reached.
    motion_cell: Option<(usize, usize)>,
    /// Wheel movement not yet a whole line.
    scroll_remainder: f32,
    /// Lines of wheel scroll toward the next page key in a full-screen
    /// program without mouse reporting.
    page_scroll: f32,
    /// Where the pointer last moved over the grid.
    pointer: Point<Pixels>,
    /// The hovered web link's target, shown after `LINK_TIP_DELAY`.
    link_tip: Option<(Point<Pixels>, String)>,
    _link_tip_timer: Option<gpui_kit::Task<()>>,
    /// The image file the hovered path names: its preview shows while
    /// Cmd (Ctrl on Linux) is held.
    image_peek: Option<ImagePeek>,
    _image_peek_check: Option<gpui_kit::Task<()>>,
    /// Cmd (Ctrl on Linux) is held.
    secondary_held: bool,
    /// Its window, to ask the system for attention (`request_attention`).
    pub(crate) window_handle: gpui_kit::AnyWindowHandle,
    /// Text being composed by an input method, shown at the cursor.
    marked: Option<String>,
    blink_on: bool,
    /// Whether the grid had the focus when last drawn: only then does its
    /// cursor blink (each blink redraws the window).
    shown_focused: bool,
    last_input: Instant,
    /// `IMPULSE_PANE_TOKEN`: how `impulse` in this terminal names it.
    pub token: String,
    /// The shell's startup files, removed when the terminal closes.
    temp_paths: Vec<std::path::PathBuf>,

    // The input bar.
    pub(crate) editor: Entity<CommandEditor>,
    pub(crate) bar: InputBar,
    /// How the shell quotes (completions, inserted paths).
    pub(crate) shell: ShellType,
    pub(crate) shell_name: String,
    /// What the shell can run, from its integration.
    pub(crate) lookup: CommandLookup,
    pub(crate) branch: Option<String>,
    /// The repository state `branch` follows, when something holds the
    /// folder's repository: its id and the observation.
    followed_repository: Option<(EntityId, gpui_kit::Subscription)>,
    /// Between a command block's start and end (OSC 133).
    pub(crate) command_running: bool,
    /// Commands to run at the shell's first prompt.
    start_commands: Vec<String>,
    /// The shell reports its prompts, so `command_running` false means it
    /// waits at one.
    pub(crate) tracks_prompts: bool,
    /// The program reads a password (echo off): the bar masks input.
    pub(crate) password: bool,
    pub(crate) last_exit: Option<i32>,
    /// A full-screen program, or a running command that asked for raw
    /// input (bracketed paste, the mouse), owns the grid: the bar hides and
    /// keys go straight to it.
    pub(crate) direct: bool,
    /// No input bar at all (the Quick Terminal): type at the shell's prompt.
    pub(crate) no_bar: bool,
    /// What the program reports (OSC 9;4): its state and percentage, while
    /// shown. Cleared when its command ends.
    pub(crate) progress: Option<(impulse_terminal::osc_scanner::ProgressState, Option<u8>)>,
    /// The program's own status (iTerm2's OSC 21337). Cleared when its
    /// command ends.
    pub(crate) session_status: SessionStatus,
    /// The command sent from the bar began with a space (kept out of the
    /// history).
    pub(crate) hidden_pending: bool,
    current_hidden: bool,
    current_command: Option<String>,
    /// Blocks whose command was typed with a leading space.
    pub(crate) hidden_blocks: HashSet<u64>,
    session_id: String,
    /// Give the bar the keyboard at the next frame (a command finished).
    focus_bar_pending: bool,

    // Blocks, hints and links.
    pub(crate) blocks: Blocks,
    pub(crate) hints: Option<Hints>,
    hover_link: Option<HoverLink>,
    hover_cell: Option<(usize, usize)>,
    pub(crate) file_checks: FileChecks,
    /// A hovered path was still being looked up (`FileChecks`).
    pub(crate) file_check_pending: bool,
    /// Tries at labelling hints while paths are looked up.
    pub(crate) hint_attempts: u8,
    _file_check_retry: Option<gpui_kit::Task<()>>,

    // Find.
    pub(crate) find: Option<FindBar>,
    pub(crate) last_find: Option<TerminalFindQuery>,

    // The prompt the bar stands in for.
    tuck: Tuck,
    /// The last row drawn (`None`: every row). Rows below it are the
    /// shell's prompt, which the bar replaces.
    pub(crate) last_content_row: Option<usize>,
    /// How far the rows are pushed down so the last one meets the bar.
    y_offset: Pixels,
    /// Where each row is drawn, with the prompt padding between blocks
    /// collapsed.
    row_layout: RowLayout,
    /// Rows drawn this frame.
    draw_rows: usize,
    /// The agent running in it, and its attention mark (`agent_tracker`).
    pub(crate) agent: crate::agent_tracker::AgentTracker,
    _wake: gpui_kit::Task<()>,
    _blink: gpui_kit::Task<()>,
    /// The shell's own folder, read every few seconds while it sends no
    /// OSC 7 (Swift's `startCwdPolling`).
    _cwd_poll: gpui_kit::Task<()>,
    /// The shell has told its folder with OSC 7 (then it's trusted alone:
    /// over ssh it names a remote folder).
    osc7_seen: bool,
}

/// What a terminal reports to whoever shows it.
pub enum TerminalViewEvent {
    TitleChanged,
    /// The shell exited (its status is `TerminalView::exited`).
    Exited,
    Bell,
    /// Open a file in an editor tab (a link or hint), at a 1-based line and
    /// column.
    OpenFile {
        path: PathBuf,
        line: Option<usize>,
        column: Option<usize>,
    },
    /// Run a window command (Command History from the bar).
    RunCommand(String),
    /// Show commit `sha` (a hint) in History, for the repository at `cwd`.
    OpenCommit {
        sha: String,
        cwd: String,
    },
    /// A message for the status bar.
    Notice(String),
    /// A command finished, with its exit status when the shell said.
    CommandEnded(Option<i32>),
    /// The shell's directory changed (OSC 7).
    DirectoryChanged(String),
}

impl gpui_kit::EventEmitter<TerminalViewEvent> for TerminalView {}

impl TerminalView {
    pub fn new(
        theme: ResolvedTheme,
        palette: ChromePalette,
        cwd: String,
        commands: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::restoring(theme, palette, cwd, commands, None, None, window, cx)
    }

    /// A terminal that first shows `transcript` (a previous session's
    /// output) under a dim "restored" rule, as the Swift app does. Its
    /// shell starts at the size `pane` holds (80×24 without one) until the
    /// pane is drawn.
    #[allow(clippy::too_many_arguments)]
    pub fn restoring(
        theme: ResolvedTheme,
        palette: ChromePalette,
        cwd: String,
        commands: Vec<String>,
        transcript: Option<String>,
        pane: Option<gpui_kit::Size<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = SettingsStore::get(cx).clone();
        let font = terminal_font(&settings.terminal_font_family);
        let metrics = measure(&font, settings.terminal_font_size as f32, window);
        let token = crate::control_server::new_token();
        let mut env = crate::control_server::terminal_env(
            &token,
            SettingsStore::get(cx).terminal_editor_integration,
        );
        // In a task: which one, its slot and the main checkout.
        env.extend(crate::tasks::identity(&cwd));
        let launch = crate::shell_launch::launch(&env);
        let config = TerminalConfig {
            shell_path: launch.shell.clone(),
            shell_args: launch.args.clone(),
            working_directory: Some(cwd.clone()),
            env_vars: launch.env.clone(),
            colors: colors(&theme),
            scrollback_lines: settings.terminal_scrollback.max(0) as usize,
            minimum_contrast: settings.terminal_minimum_contrast as f32,
            // Both reach the app, which asks the settings as each request
            // comes, so a change applies to open terminals too.
            allow_clipboard_write: true,
            allow_clipboard_read: true,
            images: settings.terminal_images,
            restored_transcript: transcript
                .map(|t| without_rule(&t).to_string())
                .filter(|t| !t.is_empty())
                .map(|t| format!("{t}\u{1b}[0;2m── {RESTORED_RULE} ──\u{1b}[0m\r\n")),
            ..Default::default()
        };
        let size = pane.map_or((80, 24), |pane| grid_size(pane, metrics));
        let backend = TerminalBackend::new(
            config,
            size.0,
            size.1,
            f32::from(metrics.width) as u16,
            f32::from(metrics.height) as u16,
        )
        .expect("terminal backend");
        // Commands to start with wait for the shell's first prompt, so they
        // run as typed ones do (written before it, the tty echoes them and
        // their output can go missing); a shell without integration gets
        // them after a few seconds.
        if !commands.is_empty() {
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(3))
                    .await;
                let _ = this.update(cx, |view, _| {
                    for command in std::mem::take(&mut view.start_commands) {
                        view.backend.write(format!("{command}\r").as_bytes());
                    }
                });
            })
            .detach();
        }

        // Sleep until the backend says there's output (the Swift renderer's
        // wake callback), then take events and damage on the main thread.
        let (wake_tx, wake_rx) = async_channel::bounded::<()>(1);
        backend.set_wakeup_callback(Some(Box::new(move || {
            let _ = wake_tx.try_send(());
        })));
        let wake = cx.spawn(async move |this, cx| {
            while wake_rx.recv().await.is_ok() {
                if this.update(cx, |view, cx| view.drain(cx)).is_err() {
                    break;
                }
            }
        });
        let cwd_poll = cx.spawn(async move |view, cx| loop {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            let Ok(Some(pid)) = view.update(cx, |view, _| {
                (!view.osc7_seen && view.exited.is_none()).then(|| view.backend.child_pid())
            }) else {
                if view.upgrade().is_none() {
                    break;
                }
                continue;
            };
            let dir = cx
                .background_executor()
                .spawn(async move { process_cwd(pid) })
                .await;
            let Some(dir) = dir else { continue };
            let _ = view.update(cx, |view, cx| {
                if !view.osc7_seen && dir != view.cwd {
                    view.cwd = dir.clone();
                    view.refresh_branch(cx);
                    cx.emit(TerminalViewEvent::DirectoryChanged(dir));
                    cx.notify();
                }
            });
        });
        let blink = cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(530))
                .await;
            let alive = this.update(cx, |view, cx| {
                // Solid while typing, as macOS text cursors are, and not
                // blinking at all without the focus, in a window that isn't
                // the active one (Swift's `shouldBlink`), or while the input
                // bar has the caret.
                let blinks = view.shown_focused
                    && cx.active_window() == Some(view.window_handle)
                    && !view.bar_shown(cx)
                    && SettingsStore::get(cx).terminal_cursor_blink
                    && view.last_input.elapsed() > Duration::from_millis(600);
                let on = !blinks || !view.blink_on;
                if on != view.blink_on {
                    view.blink_on = on;
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        });

        let shell_path = impulse_kit::shell::login::default_shell_path();
        let lookup = CommandLookup::new();
        let editor = cx.new(|cx| {
            let mut editor = CommandEditor::new(false, &theme, cx);
            editor.font = font.clone();
            editor.font_size = px((settings.terminal_font_size - 1).max(6) as f32);
            editor.placeholder = "Run a command…".into();
            editor.lookup = Some((lookup.clone(), Some(cwd.clone())));
            editor
        });
        cx.subscribe_in(&editor, window, Self::on_editor_event)
            .detach();
        let session_id = format!(
            "{:x}-{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
            cx.entity_id().as_u64()
        );

        let mut view = Self {
            backend,
            focus: cx.focus_handle(),
            root_focus: cx.focus_handle(),
            entity_id: cx.entity_id(),
            theme,
            palette,
            font,
            metrics,
            size,
            fitted: false,
            buffer: Vec::new(),
            grid_cache: None,
            grid_dirty: true,
            textures: Textures::default(),
            damage_seen: false,
            overlay_cache: RefCell::default(),
            title: String::new(),
            temp_paths: launch.temp_paths,
            token,
            cwd,
            last_duration_ms: None,
            exited: None,
            selecting: false,
            pending_selection: None,
            press_reported: None,
            motion_cell: None,
            scroll_remainder: 0.0,
            page_scroll: 0.0,
            pointer: Point::default(),
            link_tip: None,
            _link_tip_timer: None,
            image_peek: None,
            _image_peek_check: None,
            secondary_held: false,
            window_handle: window.window_handle(),
            marked: None,
            blink_on: true,
            shown_focused: false,
            last_input: Instant::now(),
            editor,
            bar: InputBar::default(),
            shell: ShellType::detect(&shell_path),
            shell_name: impulse_kit::shell::login::default_shell_name(),
            lookup,
            branch: None,
            followed_repository: None,
            command_running: false,
            start_commands: commands,
            tracks_prompts: false,
            password: false,
            last_exit: None,
            direct: false,
            no_bar: false,
            progress: None,
            session_status: SessionStatus::default(),
            hidden_pending: false,
            current_hidden: false,
            current_command: None,
            hidden_blocks: HashSet::new(),
            session_id,
            focus_bar_pending: false,
            blocks: Blocks::default(),
            hints: None,
            hover_link: None,
            hover_cell: None,
            file_checks: FileChecks::default(),
            file_check_pending: false,
            hint_attempts: 0,
            _file_check_retry: None,
            find: None,
            last_find: None,
            tuck: Tuck::default(),
            last_content_row: None,
            y_offset: px(0.),
            row_layout: RowLayout::default(),
            draw_rows: usize::MAX,
            agent: Default::default(),
            _wake: wake,
            _blink: blink,
            _cwd_poll: cwd_poll,
            osc7_seen: false,
        };
        view.refresh_branch(cx);
        CommandHistory::preload(cx);
        view
    }

    /// The pane's focus (it contains the grid's and the input bar's).
    pub fn focus_handle(&self) -> &FocusHandle {
        &self.root_focus
    }

    /// Give the terminal the keyboard: the input bar when it's showing,
    /// otherwise the grid.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.note_active();
        let grid = self.hints.is_some() || !self.blocks.selection.is_empty() || !self.bar_shown(cx);
        if grid {
            window.focus(&self.focus, cx);
        } else {
            let editor = self.editor.read(cx).focus_handle().clone();
            window.focus(&editor, cx);
        }
    }

    /// At a prompt with the bar showing, typing in the grid belongs in the
    /// bar: in the shell's own line it would sit unseen and run with the
    /// next command.
    fn typing_goes_to_bar(&self, cx: &App) -> bool {
        self.bar_shown(cx) && !self.command_running && self.tracks_prompts
    }

    /// The git branch of the terminal's folder: from its repository's state
    /// when a workspace holds it, followed so a checkout made anywhere else
    /// (the palette, another terminal, an agent) shows at once; else looked
    /// up in the background.
    fn refresh_branch(&mut self, cx: &mut Context<Self>) {
        let state = RepositoryStore::root_for(std::path::Path::new(&self.cwd), cx)
            .and_then(|root| RepositoryStore::existing(&root, cx));
        match &state {
            Some(state)
                if self.followed_repository.as_ref().map(|(id, _)| *id)
                    != Some(state.entity_id()) =>
            {
                let observing = cx.observe(state, |view, state, cx| {
                    let state = state.read(cx);
                    if state.loaded && view.branch != state.branch_label() {
                        view.branch = state.branch_label();
                        cx.notify();
                    }
                });
                self.followed_repository = Some((state.entity_id(), observing));
            }
            Some(_) => {}
            None => self.followed_repository = None,
        }
        if let Some(state) = state.filter(|state| state.read(cx).loaded) {
            let branch = state.read(cx).branch_label();
            if self.branch != branch {
                self.branch = branch;
                cx.notify();
            }
            return;
        }
        let dir = self.cwd.clone();
        let task = cx.background_spawn(async move {
            let branch = crate::workspace::git_branch(std::path::Path::new(&dir));
            (!branch.is_empty()).then_some(branch)
        });
        cx.spawn(async move |this, cx| {
            let branch = task.await;
            let _ = this.update(cx, |view, cx| {
                if view.branch != branch {
                    view.branch = branch;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// A fresh grid snapshot (rows for links and hints).
    pub(crate) fn grid_now(&mut self) -> Option<Rc<Grid>> {
        self.snapshot().0
    }

    /// About the last `max_rows` rows of output, with colors, for the
    /// session.
    pub fn transcript(&self, max_rows: usize) -> String {
        self.backend.transcript(max_rows, true)
    }

    /// The shell waits at its prompt in the foreground, so a command typed
    /// into it reaches the shell and not a running program (Swift's
    /// `isAtPrompt`). Without shell integration commands aren't marked;
    /// then the terminal's foreground process group tells.
    /// What the workbench's chrome shows of this terminal (tab title and
    /// glyphs, progress, status, folder, agent state), as one number: the
    /// workbench redraws for a terminal it isn't showing only when this
    /// changes, not for every line of output.
    pub(crate) fn chrome_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.title.hash(&mut hasher);
        self.cwd.hash(&mut hasher);
        self.direct.hash(&mut hasher);
        self.command_running.hash(&mut hasher);
        self.at_prompt().hash(&mut hasher);
        format!("{:?}{:?}", self.progress, self.session_status).hash(&mut hasher);
        self.agent.agent.hash(&mut hasher);
        self.agent.machine.state().hash(&mut hasher);
        self.agent.needs_attention.hash(&mut hasher);
        self.agent.status_message.hash(&mut hasher);
        hasher.finish()
    }

    pub(crate) fn at_prompt(&self) -> bool {
        if self.command_running
            || self
                .backend
                .mode()
                .contains(impulse_terminal::TerminalMode::ALT_SCREEN)
        {
            return false;
        }
        self.backend
            .foreground_pid()
            .is_none_or(|foreground| foreground == self.shell_pid())
    }

    /// The shell's process id.
    pub fn shell_pid(&self) -> i32 {
        self.backend.child_pid() as i32
    }

    /// New theme or terminal settings.
    pub fn set_theme(
        &mut self,
        theme: ResolvedTheme,
        palette: ChromePalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = SettingsStore::get(cx).clone();
        let mut config = TerminalConfig {
            colors: colors(&theme),
            minimum_contrast: settings.terminal_minimum_contrast as f32,
            ..Default::default()
        };
        config.scrollback_lines = settings.terminal_scrollback.max(0) as usize;
        self.backend.set_colors(&config);
        self.backend.set_images_enabled(settings.terminal_images);
        self.font = terminal_font(&settings.terminal_font_family);
        self.metrics = measure(&self.font, settings.terminal_font_size as f32, window);
        let colors = EditorColors::new(&theme);
        self.editor
            .update(cx, |editor, _| editor.colors = colors.clone());
        if let Some(find) = &self.find {
            find.set_colors(colors, cx);
        }
        self.theme = theme;
        self.palette = palette;
        cx.notify();
    }

    /// Events and damage since the last wake.
    fn drain(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        for event in self.backend.poll_events() {
            changed = true;
            self.agent_observe(&event, cx);
            match event {
                TerminalEvent::CwdChanged(path) => {
                    self.osc7_seen = true;
                    if path != self.cwd {
                        self.cwd = path.clone();
                        self.refresh_branch(cx);
                        cx.emit(TerminalViewEvent::DirectoryChanged(path));
                    }
                }
                TerminalEvent::PromptStart => {
                    // A prompt means nothing runs, even if a command's end
                    // mark was lost (the bar would otherwise stay hidden at
                    // a bare prompt), and nothing reads a password.
                    self.command_running = false;
                    self.tracks_prompts = true;
                    self.password = false;
                    for command in std::mem::take(&mut self.start_commands) {
                        self.run_command(&command);
                    }
                }
                TerminalEvent::CommandBlockStarted(block) => {
                    self.command_running = true;
                    self.current_command = block.command.clone();
                    self.current_hidden = std::mem::take(&mut self.hidden_pending);
                    if self.current_hidden {
                        self.hidden_blocks.insert(block.id.0);
                    }
                    self.blocks.highlighted = None;
                    self.blocks.selection.clear();
                    self.close_menu(cx);
                }
                TerminalEvent::PasswordInputChanged(active) => self.password = active,
                TerminalEvent::ShellNames(names) => self.lookup.set_names(names),
                TerminalEvent::ShellPath(path) => self.lookup.set_path(&path),
                TerminalEvent::TitleChanged(title) => {
                    self.title = title;
                    cx.emit(TerminalViewEvent::TitleChanged);
                }
                TerminalEvent::ResetTitle => {
                    self.title.clear();
                    cx.emit(TerminalViewEvent::TitleChanged);
                }
                TerminalEvent::CommandBlockEnded(block) => {
                    self.command_running = false;
                    self.password = false;
                    self.bar.session_revision += 1;
                    self.last_exit = block.exit_code;
                    self.last_duration_ms = block
                        .ended_at_ms
                        .map(|end| end.saturating_sub(block.started_at_ms));
                    if let Some(command) = block.command.clone().or(self.current_command.clone()) {
                        if !self.current_hidden {
                            let cwd = block
                                .cwd
                                .clone()
                                .or_else(|| (!self.cwd.is_empty()).then(|| self.cwd.clone()));
                            CommandHistory::record(
                                &command,
                                cwd,
                                block.exit_code,
                                self.last_duration_ms.map(|d| d as i64),
                                Some(self.session_id.clone()),
                                cx,
                            );
                        }
                    }
                    self.blocks.highlighted = None;
                    // The program that reported them is gone.
                    self.progress = None;
                    self.session_status = SessionStatus::default();
                    // Tuck the prompt away once, when it appears after this
                    // command; the shell is back for the next command.
                    self.tuck = Tuck {
                        armed: true,
                        ..Tuck::default()
                    };
                    self.focus_bar_pending = true;
                    self.refresh_branch(cx);
                    cx.emit(TerminalViewEvent::CommandEnded(block.exit_code));
                }
                TerminalEvent::ClipboardStore(text) => {
                    if SettingsStore::get(cx).terminal_allow_osc52_write {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                }
                TerminalEvent::ClipboardLoad => {
                    if !SettingsStore::get(cx).terminal_allow_osc52_read {
                        continue;
                    }
                    if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        use std::io::Write as _;
                        let mut reply = Vec::new();
                        let _ = write!(reply, "\x1b]52;c;{}\x07", base64(text.as_bytes()));
                        self.backend.write(&reply);
                    }
                }
                TerminalEvent::Bell => cx.emit(TerminalViewEvent::Bell),
                TerminalEvent::Progress { state, percent } => {
                    self.progress = (state != impulse_terminal::osc_scanner::ProgressState::Hidden)
                        .then_some((state, percent));
                }
                TerminalEvent::SessionStatus(fields) => self.session_status.apply(&fields),
                TerminalEvent::ChildExited(code) => {
                    self.exited = Some(code);
                    cx.emit(TerminalViewEvent::Exited);
                }
                _ => {}
            }
        }
        let damaged = !matches!(self.backend.take_damage(), Some(rows) if rows.is_empty());
        if damaged || std::mem::take(&mut self.damage_seen) {
            self.grid_dirty = true;
            changed = true;
            self.agent_output(cx);
            // Output brings the view to the bottom unless the user is
            // reading back past where the prompt was tucked away.
            let offset = self.backend.display_offset();
            if offset > 0
                && self.tuck.floor.is_some_and(|floor| offset <= floor)
                && SettingsStore::get(cx).terminal_scroll_on_output
            {
                self.backend.scroll_to_bottom();
            }
        }
        if changed {
            cx.notify();
        }
    }

    /// Fit the grid to the pane. The PTY is told the cell's size in device
    /// pixels (`scale` to a point), which programs size images by.
    fn fit(&mut self, bounds: Bounds<Pixels>, scale: f32) {
        let m = self.metrics;
        let (cols, rows) = grid_size(bounds.size, m);
        if (cols, rows) != self.size {
            self.size = (cols, rows);
        }
        self.fitted = true;
        let cell = (
            (f32::from(m.width) * scale).round().max(1.0) as u16,
            (f32::from(m.height) * scale).round().max(1.0) as u16,
        );
        // Unchanged sizes are ignored there.
        self.backend.resize(cols, rows, cell.0, cell.1);
    }

    /// Size the grid for `pane` before it's drawn there (a terminal in a
    /// hidden tab or workspace), so its shell doesn't print a greeting at
    /// 80×24 that stays wrapped, or filled with a background color where
    /// it scrolled, once the grid widens.
    pub(crate) fn fit_pane(&mut self, mut pane: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        if self.bar_shown(cx) {
            pane.size.height -= self.bar_height(window, cx);
        }
        self.fit(pane, window.scale_factor());
    }

    /// The grid, the block overlay and the modes, read again only when
    /// they changed (a blink or a hover reuses the last ones).
    fn snapshot(&mut self) -> (Option<Rc<Grid>>, BlockOverlay, TerminalMode) {
        if !matches!(self.backend.take_damage(), Some(rows) if rows.is_empty()) {
            self.grid_dirty = true;
            self.damage_seen = true;
        }
        let key = (self.backend.display_offset(), self.backend.dimensions());
        let grid = match &self.grid_cache {
            Some((grid, at)) if !self.grid_dirty && *at == key => Some(grid.clone()),
            _ => {
                let needed = self.backend.grid_buffer_size();
                if self.buffer.len() < needed {
                    self.buffer.resize(needed, 0);
                }
                let written = self.backend.write_grid_to_buffer(&mut self.buffer);
                let grid = Grid::read(&self.buffer[..written]).map(Rc::new);
                self.grid_cache = grid.clone().map(|grid| (grid, key));
                self.grid_dirty = false;
                grid
            }
        };
        (grid, self.overlay(), self.backend.mode())
    }

    /// The images this frame shows: the direct placements in view and the
    /// grid's placeholder cells, with their textures. `None` when there
    /// are none.
    fn frame_images(&mut self, grid: Option<&Grid>, scale: f32) -> Option<FrameImages> {
        let layer = self.backend.image_layer();
        let placeholders = grid.is_some_and(|grid| !grid.images.is_empty());
        if layer.placements.is_empty() && !placeholders {
            // Textures of images no longer shown go.
            self.textures.update(&self.backend, []);
            return None;
        }
        let ids: Vec<u32> = layer
            .placements
            .iter()
            .map(|p| p.image_id)
            .chain(grid.into_iter().flat_map(Grid::image_ids))
            .collect();
        let textures = self.textures.update(&self.backend, ids);
        Some(FrameImages {
            layer,
            textures,
            scale,
        })
    }

    /// The block overlay, kept until `block_overlay_key` changes.
    fn overlay(&self) -> BlockOverlay {
        let key = self.backend.block_overlay_key();
        let mut cache = self.overlay_cache.borrow_mut();
        match &*cache {
            Some((at, overlay)) if *at == key => overlay.clone(),
            _ => {
                let overlay = self.backend.block_overlay();
                *cache = Some((key, overlay.clone()));
                overlay
            }
        }
    }

    /// Typing goes to the bottom of the output, as in other terminals.
    fn typed(&mut self) {
        self.last_input = Instant::now();
        self.blink_on = true;
        if self.backend.display_offset() > 0 {
            self.backend.scroll_to_bottom();
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        let m = &k.modifiers;
        if self.hints.is_some() {
            self.hint_key(event, window, cx);
            return cx.stop_propagation();
        }
        if !self.blocks.selection.is_empty() && self.block_selection_key(event, window, cx) {
            return cx.stop_propagation();
        }
        let secondary = if cfg!(target_os = "macos") {
            m.platform
        } else {
            m.control
        };
        // Cmd+Up selects blocks from the grid too, as it does in the bar.
        if !self.direct && k.key == "up" && secondary && !m.shift && !m.alt {
            self.begin_block_selection(window, cx);
            return cx.stop_propagation();
        }
        if self.typing_goes_to_bar(cx) {
            // Return and Escape go back to the bar; text goes in through
            // the input handler (`replace_text_in_range`); other keys mean
            // nothing here and mustn't reach the shell's line.
            if matches!(k.key.as_str(), "enter" | "escape") {
                self.focus(window, cx);
                cx.stop_propagation();
            } else if m.control || m.platform || k.key_char.is_none() {
                cx.stop_propagation();
            }
            return;
        }
        let meta = alt_meta(cx);
        // A program that asked for the kitty keyboard protocol gets keys it
        // can tell apart (Ctrl-I vs Tab, Esc vs Alt); plain typing still goes
        // through text input.
        let event_kind = if event.is_held {
            kitty_keyboard::Event::Repeat
        } else {
            kitty_keyboard::Event::Press
        };
        if let Some(bytes) = self.kitty_bytes(k, event_kind, meta) {
            if !bytes.is_empty() {
                self.backend.clear_selection();
                self.backend.write(&bytes);
                self.typed();
                self.agent_key(k.key == "enter", cx);
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        let press = KeyPress {
            key: &k.key,
            text: k.key_char.as_deref(),
            ctrl: m.control,
            alt: m.alt,
            shift: m.shift,
            platform: m.platform,
        };
        let app_cursor = self.backend.mode().contains(TerminalMode::APP_CURSOR);
        if let Some(bytes) = key_encoder::encode(&press, app_cursor, meta) {
            self.backend.clear_selection();
            self.backend.write(&bytes);
            self.typed();
            self.agent_key(k.key == "enter", cx);
            cx.stop_propagation();
            cx.notify();
        }
    }

    /// The kitty keyboard protocol's bytes for a key, when the program asked
    /// for the protocol and the key isn't text for an input method or the
    /// legacy path; empty when nothing is sent.
    fn kitty_bytes(
        &self,
        k: &gpui_kit::Keystroke,
        event: kitty_keyboard::Event,
        alt_is_meta: bool,
    ) -> Option<Vec<u8>> {
        let m = &k.modifiers;
        // Cmd (Super on Linux) shortcuts stay the app's and the desktop's.
        if m.platform || self.marked.as_ref().is_some_and(|t| !t.is_empty()) {
            return None;
        }
        let flags = kitty_keyboard::Flags(((self.backend.mode().bits() >> 11) & 0x1F) as u8);
        if flags.0 == 0 {
            return None;
        }
        let mut modifiers = kitty_keyboard::Modifiers::NONE;
        for (held, bit) in [
            (m.shift, kitty_keyboard::Modifiers::SHIFT),
            (m.alt, kitty_keyboard::Modifiers::ALT),
            (m.control, kitty_keyboard::Modifiers::CONTROL),
        ] {
            if held {
                modifiers = modifiers | bit;
            }
        }
        let (key, text) =
            kitty_keyboard::key_for(&k.key, k.key_char.as_deref(), modifiers, alt_is_meta)?;
        kitty_keyboard::encode(key, modifiers, event, text, flags)
    }

    /// Releases, for programs that asked for kitty event types.
    fn key_up(&mut self, event: &gpui_kit::KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.typing_goes_to_bar(cx) {
            return;
        }
        let meta = alt_meta(cx);
        if let Some(bytes) =
            self.kitty_bytes(&event.keystroke, kitty_keyboard::Event::Release, meta)
        {
            if !bytes.is_empty() {
                self.backend.write(&bytes);
            }
        }
    }

    /// The cell under `position` (window coordinates), clamped to the grid.
    fn cell_at(&self, position: Point<Pixels>, bounds: Bounds<Pixels>) -> (usize, usize) {
        let x = f32::from(position.x - bounds.origin.x) - PADDING;
        let y = position.y - bounds.origin.y - self.y_offset - px(PADDING);
        let col = (x / f32::from(self.metrics.width)).floor().max(0.0) as usize;
        let row = self.row_layout.row_at(y.max(px(0.)));
        (
            col.min(self.size.0.saturating_sub(1) as usize),
            row.min(self.size.1.saturating_sub(1) as usize),
        )
    }

    /// Right-click: the terminal's menu, with the clicked block's actions
    /// first (Swift's `rightMouseDown`, which always shows it).
    fn context_menu_at(
        &mut self,
        event: &MouseDownEvent,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus(window, cx);
        let (_, row) = self.cell_at(event.position, bounds);
        let block = self.block_at_row(row, cx);
        cx.stop_propagation();
        self.open_block_menu(block, event.position, true, window, cx);
    }

    /// Mouse reporting is on (any of the click, drag and motion modes).
    fn mouse_reporting(&self) -> bool {
        self.backend.mode().intersects(
            TerminalMode::MOUSE_REPORT_CLICK
                | TerminalMode::MOUSE_DRAG
                | TerminalMode::MOUSE_MOTION,
        )
    }

    /// Shift or Option (Alt) held: the mouse selects text even while a
    /// program reports it (Swift's `selectsDespiteMouseReporting`).
    fn selects_despite_reporting(modifiers: &Modifiers) -> bool {
        modifiers.shift || modifiers.alt
    }

    /// Send the program one mouse event if its mode asks for it (Swift's
    /// `reportMouseEvent`): `button` 0 left, 1 middle; a drag only when it
    /// reaches another cell; Shift, Alt and Control as bits 4, 8 and 16.
    /// True when it was sent (or swallowed as a repeat).
    fn report_mouse(
        &mut self,
        button: u8,
        modifiers: &Modifiers,
        (col, row): (usize, usize),
        motion: bool,
        release: bool,
    ) -> bool {
        let mode = self.backend.mode();
        if !self.mouse_reporting() {
            return false;
        }
        if motion && !mode.intersects(TerminalMode::MOUSE_DRAG | TerminalMode::MOUSE_MOTION) {
            return false;
        }
        if motion {
            if self.motion_cell == Some((col, row)) {
                return true;
            }
            self.motion_cell = Some((col, row));
        } else {
            self.motion_cell = None;
        }
        let mut code = button;
        if motion {
            code += 32;
        }
        if modifiers.shift {
            code += 4;
        }
        if modifiers.alt {
            code += 8;
        }
        if modifiers.control {
            code += 16;
        }
        let sgr = mode.contains(TerminalMode::MOUSE_SGR);
        self.backend
            .write(&mouse_report(code, col, row, release, sgr));
        true
    }

    fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocks.menu.take().is_some() {
            cx.notify();
        }
        if self.hints.is_some() {
            self.hints = None;
            cx.notify();
        }
        // A plain click leaves a block selection.
        let opens = if cfg!(target_os = "macos") {
            event.modifiers.platform
        } else {
            event.modifiers.control
        };
        if !opens && !self.blocks.selection.is_empty() {
            self.end_block_selection(false, window, cx);
        }
        // The grid at a prompt is read-only: the bar keeps the keyboard,
        // and the mouse still selects.
        self.focus(window, cx);
        let (col, row) = self.cell_at(event.position, bounds);
        if opens && event.button == MouseButton::Left {
            // Cmd-click (Ctrl-click on Linux) opens a link; elsewhere on a
            // block it adds the block to the selection or drops it.
            if SettingsStore::get(cx).terminal_allow_hyperlink {
                if let Some(link) = self.link_at(col, row) {
                    self.open_link(link.target, window, cx);
                    return;
                }
            }
            if let Some(id) = self.block_at_row(row, cx) {
                self.toggle_block_selection(id, cx);
                if !self.blocks.selection.is_empty() {
                    window.focus(&self.focus, cx);
                }
                return;
            }
        }
        // The press goes to a program reporting the mouse; its drags and
        // release follow it there.
        if !Self::selects_despite_reporting(&event.modifiers)
            && self.report_mouse(0, &event.modifiers, (col, row), false, false)
        {
            self.press_reported = Some(0);
            return;
        }
        if event.button != MouseButton::Left {
            return;
        }
        self.backend.clear_selection();
        self.pending_selection = None;
        match press_selection(event.click_count, event.modifiers.alt) {
            Press::Pending(kind) => {
                // A plain click selects nothing (and copies nothing) until
                // it drags.
                self.selecting = false;
                self.pending_selection = Some(((col, row), kind));
            }
            Press::Now(kind) => {
                self.backend.start_selection(col, row, kind);
                self.backend.update_selection(col, row);
                self.selecting = true;
            }
        }
        cx.notify();
    }

    fn mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        bounds: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let (col, row) = self.cell_at(event.position, bounds);
        self.modifiers_changed(&event.modifiers, cx);
        if event.pressed_button.is_none() {
            self.pointer = event.position;
            self.update_hover(col, row, cx);
        }
        if let Some(button) = self.press_reported {
            if event.pressed_button.is_some() {
                self.report_mouse(button, &event.modifiers, (col, row), true, false);
            }
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        if !self.selecting {
            let Some((anchor, kind)) = self.pending_selection else {
                return;
            };
            if !drag_begins(anchor, (col, row)) {
                return;
            }
            self.pending_selection = None;
            self.backend.start_selection(anchor.0, anchor.1, kind);
            self.selecting = true;
        }
        // Past the top or bottom: scroll history while selecting.
        let y = event.position.y - bounds.origin.y;
        if y < px(0.) {
            self.backend.scroll(1);
        } else if y > bounds.size.height {
            self.backend.scroll(-1);
        }
        self.backend.update_selection(col, row);
        cx.notify();
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        if let Some(button) = self.press_reported.take() {
            let cell = self.cell_at(event.position, bounds);
            self.report_mouse(button, &event.modifiers, cell, false, true);
            return;
        }
        self.pending_selection = None;
        if !self.selecting {
            return;
        }
        self.selecting = false;
        // Only a drag or a double or triple click gets here: a plain click
        // leaves the clipboard alone.
        if SettingsStore::get(cx).terminal_copy_on_select {
            if let Some(text) = self.backend.selected_text().filter(|t| !t.is_empty()) {
                #[cfg(target_os = "linux")]
                cx.write_to_primary(ClipboardItem::new_string(text.clone()));
                #[cfg(target_os = "macos")]
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        }
    }

    /// The middle button: to a program reporting the mouse, else (on
    /// Linux) a paste of the primary selection, as X terminals do.
    fn middle_down(
        &mut self,
        event: &MouseDownEvent,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cell = self.cell_at(event.position, bounds);
        if self.report_mouse(1, &event.modifiers, cell, false, false) {
            self.press_reported = Some(1);
            return;
        }
        #[cfg(target_os = "linux")]
        if let Some(text) = cx.read_from_primary().and_then(|item| item.text()) {
            if !text.is_empty() {
                self.focus(window, cx);
                self.paste_payload(&text, window, cx);
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (window, cx);
    }

    fn middle_up(&mut self, event: &MouseUpEvent, bounds: Bounds<Pixels>) {
        if self.press_reported == Some(1) {
            self.press_reported = None;
            let cell = self.cell_at(event.position, bounds);
            self.report_mouse(1, &event.modifiers, cell, false, true);
        }
    }

    /// The command block on viewport row `row` (not the live prompt).
    pub(crate) fn block_at_row(&self, row: usize, cx: &App) -> Option<u64> {
        if self.direct || !SettingsStore::get(cx).terminal_blocks {
            return None;
        }
        let overlay = self.overlay();
        if overlay.alt_screen {
            return None;
        }
        let row = row as i32;
        if overlay.prompt_row.is_some_and(|p| row >= p) {
            return None;
        }
        overlay
            .blocks
            .iter()
            .find(|b| row >= b.start_row && row <= b.end_row)
            .map(|b| b.id)
    }

    /// The pointer moved to a cell: the block under it (hover wash and
    /// toolbar) and the link under it (underline, Cmd-click).
    fn update_hover(&mut self, col: usize, row: usize, cx: &mut Context<Self>) {
        let hovered = self.block_at_row(row, cx);
        if hovered != self.blocks.hovered {
            self.blocks.hovered = hovered;
            cx.notify();
        }
        if self.hover_cell == Some((col, row)) {
            return;
        }
        self.hover_cell = Some((col, row));
        let link = if SettingsStore::get(cx).terminal_allow_hyperlink {
            self.link_at(col, row)
        } else {
            None
        };
        // A path still being looked up: look again shortly, if the pointer
        // stays.
        if std::mem::take(&mut self.file_check_pending) {
            self._file_check_retry = Some(cx.spawn(async move |view, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                let _ = view.update(cx, |view, cx| {
                    if view.hover_cell == Some((col, row)) {
                        view.hover_cell = None;
                        view.update_hover(col, row, cx);
                    }
                });
            }));
        }
        if link != self.hover_link {
            // A web link's target shows after a tooltip's delay: an OSC 8
            // link's text needn't say where it goes.
            self.link_tip = None;
            self._link_tip_timer = None;
            if let Some(LinkTarget::Url(url)) = link.as_ref().map(|l| &l.target) {
                let (url, at) = (url.clone(), self.pointer);
                self._link_tip_timer = Some(cx.spawn(async move |view, cx| {
                    cx.background_executor().timer(LINK_TIP_DELAY).await;
                    let _ = view.update(cx, |view, cx| {
                        view.link_tip = Some((at, url));
                        cx.notify();
                    });
                }));
            }
            self.image_peek = None;
            self._image_peek_check = match link.as_ref().map(|l| &l.target) {
                Some(LinkTarget::File { path, .. })
                    if crate::image_view::has_image_extension(path) =>
                {
                    Some(self.check_image_peek(path.clone(), cx))
                }
                _ => None,
            };
            self.hover_link = link;
            cx.notify();
        }
    }

    /// Look at the hovered image file off the main thread: one that isn't
    /// too big to read for a glance gets a preview, at its size.
    fn check_image_peek(&self, path: PathBuf, cx: &mut Context<Self>) -> gpui_kit::Task<()> {
        let at = self.pointer;
        cx.spawn(async move |view, cx| {
            let file = path.clone();
            let dimensions = cx
                .background_executor()
                .spawn(async move {
                    let small = std::fs::metadata(&file)
                        .is_ok_and(|m| m.is_file() && m.len() <= MAX_PEEK_BYTES);
                    small.then(|| image::image_dimensions(&file).ok()).flatten()
                })
                .await;
            let Some((width, height)) = dimensions.filter(|(w, h)| *w > 0 && *h > 0) else {
                return;
            };
            // Scaled down to fit, never up.
            let fit = (PEEK_SIZE.0 / width as f32)
                .min(PEEK_SIZE.1 / height as f32)
                .min(1.0);
            let _ = view.update(cx, |view, cx| {
                view.image_peek = Some(ImagePeek {
                    at,
                    path,
                    size: size(px(width as f32 * fit), px(height as f32 * fit)),
                });
                cx.notify();
            });
        })
    }

    /// The hovered image path's preview, while Cmd (Ctrl) is held.
    fn image_peek_view(&self) -> Option<AnyElement> {
        if !self.secondary_held {
            return None;
        }
        let peek = self.image_peek.as_ref()?;
        let p = &self.palette;
        Some(
            gpui_kit::deferred(
                gpui_kit::anchored()
                    .position(peek.at + point(px(12.), px(18.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .p(px(4.))
                            .rounded(px(6.))
                            .bg(crate::workbench_chrome::hsla(p.overlay))
                            .border_1()
                            .border_color(crate::workbench_chrome::hsla(p.hairline_strong))
                            .child(
                                gpui_kit::img(peek.path.clone())
                                    .w(peek.size.width)
                                    .h(peek.size.height)
                                    .object_fit(gpui_kit::ObjectFit::Contain),
                            ),
                    ),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }

    /// Cmd (Ctrl on Linux) went down or up: the image preview follows.
    fn modifiers_changed(&mut self, modifiers: &Modifiers, cx: &mut Context<Self>) {
        let held = secondary(modifiers);
        if held != self.secondary_held {
            self.secondary_held = held;
            if self.image_peek.is_some() {
                cx.notify();
            }
        }
    }

    /// The hovered link's target, as Swift's tooltip says it.
    fn link_tooltip(&self) -> Option<AnyElement> {
        let (at, url) = self.link_tip.clone()?;
        self.hover_link.as_ref()?;
        let p = &self.palette;
        let click = if cfg!(target_os = "macos") {
            "⌘-click"
        } else {
            "Ctrl-click"
        };
        Some(
            gpui_kit::deferred(
                gpui_kit::anchored()
                    .position(at + point(px(0.), px(18.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .max_w(px(480.))
                            .px(px(8.))
                            .py(px(4.))
                            .rounded(px(5.))
                            .bg(crate::workbench_chrome::hsla(p.overlay))
                            .border_1()
                            .border_color(crate::workbench_chrome::hsla(p.hairline_strong))
                            .text_size(px(11.))
                            .font_family(crate::workbench::UI_FONT)
                            .text_color(crate::workbench_chrome::hsla(p.text))
                            .child(format!("{click} to open {url}")),
                    ),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }

    fn mouse_left(&mut self, cx: &mut Context<Self>) {
        self.hover_cell = None;
        self.image_peek = None;
        self._image_peek_check = None;
        if self.blocks.hovered.take().is_some() | self.hover_link.take().is_some() {
            cx.notify();
        }
    }

    fn scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        bounds: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let lines = wheel_lines(event.delta, self.metrics.height) + self.scroll_remainder;
        let whole = lines.trunc();
        self.scroll_remainder = lines - whole;
        if whole == 0.0 {
            return;
        }
        let mode = self.backend.mode();
        if self.mouse_reporting() {
            // Buttons 64 and 65, one per line, at most 20 at once (Swift).
            let (col, row) = self.cell_at(event.position, bounds);
            let button = if whole > 0.0 { 64 } else { 65 };
            let sgr = mode.contains(TerminalMode::MOUSE_SGR);
            let one = mouse_report(button, col, row, false, sgr);
            self.backend
                .write(&one.repeat((whole.abs() as usize).min(20)));
        } else if mode.contains(TerminalMode::ALT_SCREEN) {
            // Full-screen programs without mouse support page, one page key
            // per four lines, as Swift's renderer does: arrow keys are input
            // history to agents such as Claude Code, which would hide the
            // transcript instead of scrolling it.
            self.page_scroll += whole;
            let pages = (self.page_scroll / 4.0).trunc();
            if pages == 0.0 {
                return;
            }
            self.page_scroll -= pages * 4.0;
            let key: &[u8] = if pages > 0.0 { b"\x1b[5~" } else { b"\x1b[6~" };
            for _ in 0..(pages.abs() as usize).min(10) {
                self.backend.write(key);
            }
        } else {
            let mut delta = whole as i32;
            // Scrolling down stops where the tucked-away prompt begins.
            if let (true, Some(floor), false) = (delta < 0, self.tuck.floor, self.command_running) {
                let offset = self.backend.display_offset() as i32;
                delta = delta.max(floor as i32 - offset).min(0);
            }
            if delta != 0 {
                self.backend.scroll(delta);
            }
        }
        self.hover_cell = None;
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selection(cx)
    }

    /// Copy the selected blocks, else the selected text.
    pub(crate) fn copy_selection(&mut self, cx: &mut Context<Self>) {
        // Selected blocks: each command and its output.
        if !self.blocks.selection.is_empty() {
            return self.copy_selected_blocks(cx);
        }
        // Nothing selected: nothing happens (Swift's `copySelection`), not
        // an interrupt the program didn't ask for.
        if let Some(text) = self.backend.selected_text().filter(|t| !t.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// Paste text (its trailing line breaks dropped), else an image saved
    /// as a temporary file, else copied files: their paths. An image's path
    /// goes as it is, inside a bracketed paste, which is how TUI agents
    /// (Claude Code) recognize an attached image.
    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        self.paste_clipboard(window, cx)
    }

    pub(crate) fn paste_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let payload = match item.text() {
            Some(text) => text.trim_end_matches(['\n', '\r']).to_string(),
            None => match item.entries().first() {
                Some(gpui_kit::ClipboardEntry::Image(image)) => {
                    let millis = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or(0);
                    let path = std::env::temp_dir().join(format!(
                        "impulse-clipboard-{millis}.{}",
                        image.format.extension()
                    ));
                    if let Err(error) =
                        impulse_kit::settings::write_private_file(&path, &image.bytes)
                    {
                        log::error!("couldn't save the clipboard's image: {error}");
                        return;
                    }
                    path.to_string_lossy().into_owned()
                }
                Some(gpui_kit::ClipboardEntry::ExternalPaths(paths)) => {
                    self.quoted_paths(paths.paths())
                }
                _ => return,
            },
        };
        if !payload.is_empty() {
            self.paste_payload(&payload, window, cx);
        }
    }

    /// Paths for the shell's line, quoted for this terminal's shell, with a
    /// space after (files dropped on the terminal, or copied ones pasted).
    fn quoted_paths(&self, paths: &[std::path::PathBuf]) -> String {
        let quoted: Vec<String> = paths
            .iter()
            .map(|p| impulse_kit::shell::quoting::escaped(&p.to_string_lossy(), self.shell))
            .collect();
        format!("{} ", quoted.join(" "))
    }

    /// Text pasted or dropped: into the bar when it owns input (in the
    /// shell's line it would sit unseen and run with the next command),
    /// else to the program, bracketed when it asked for that.
    fn paste_payload(&mut self, payload: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.bar_shown(cx) {
            return self.insert_into_bar(payload, window, cx);
        }
        let bracketed = self.backend.mode().contains(TerminalMode::BRACKETED_PASTE);
        self.backend
            .write(&key_encoder::paste_bytes(payload, bracketed));
        self.typed();
        cx.notify();
    }

    /// Files dropped on the terminal: their quoted paths, as typed.
    fn drop_paths(
        &mut self,
        paths: &gpui_kit::ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let payload = self.quoted_paths(paths.paths());
        self.paste_payload(&payload, window, cx);
        self.focus(window, cx);
    }

    /// A file tree row dropped here: its path, quoted, as a file from the
    /// file manager would be (Swift's tree drags a file URL).
    fn drop_tree_path(
        &mut self,
        dragged: &crate::file_tree_actions::DraggedPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let payload = self.quoted_paths(std::slice::from_ref(&dragged.0));
        self.paste_payload(&payload, window, cx);
        self.focus(window, cx);
    }

    /// The screen, not the scrollback (Swift's `selectVisibleGrid`).
    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let (cols, rows) = self.backend.dimensions();
        self.backend.start_selection(0, 0, SelectionKind::Simple);
        self.backend.update_selection(
            usize::from(cols.saturating_sub(1)),
            usize::from(rows.saturating_sub(1)),
        );
        cx.notify();
    }

    /// ⌘K: Ctrl+L for the shell, which clears the screen (Swift's
    /// `contextClear`).
    fn clear(&mut self, _: &Clear, _: &mut Window, cx: &mut Context<Self>) {
        self.backend.write(b"\x0c");
        cx.notify();
    }

    /// A program owns the screen (the alternate screen, or one in raw mode
    /// at the prompt like Claude Code): Shift with PageUp, PageDown, Home
    /// and End is its (selecting in an editor, paging in a pager), as the
    /// Swift app sends them; at the shell they scroll the scrollback.
    fn send_to_program(&mut self, keys: &str, cx: &mut Context<Self>) -> bool {
        let owned = self.direct || self.backend.mode().contains(TerminalMode::ALT_SCREEN);
        if !owned {
            return false;
        }
        let Ok(keystroke) = gpui_kit::Keystroke::parse(keys) else {
            return false;
        };
        let meta = alt_meta(cx);
        let bytes = self
            .kitty_bytes(&keystroke, kitty_keyboard::Event::Press, meta)
            .or_else(|| {
                let press = KeyPress {
                    key: &keystroke.key,
                    text: None,
                    ctrl: false,
                    alt: false,
                    shift: true,
                    platform: false,
                };
                let app_cursor = self.backend.mode().contains(TerminalMode::APP_CURSOR);
                key_encoder::encode(&press, app_cursor, meta)
            });
        if let Some(bytes) = bytes.filter(|b| !b.is_empty()) {
            self.backend.write(&bytes);
            self.typed();
        }
        true
    }

    fn page_up(&mut self, _: &ScrollPageUp, _: &mut Window, cx: &mut Context<Self>) {
        if !self.send_to_program("shift-pageup", cx) {
            self.backend.scroll(self.size.1 as i32 - 1);
        }
        cx.notify();
    }

    fn page_down(&mut self, _: &ScrollPageDown, _: &mut Window, cx: &mut Context<Self>) {
        if !self.send_to_program("shift-pagedown", cx) {
            self.backend.scroll(-(self.size.1 as i32 - 1));
        }
        cx.notify();
    }

    fn scroll_top(&mut self, _: &ScrollToTop, _: &mut Window, cx: &mut Context<Self>) {
        if !self.send_to_program("shift-home", cx) {
            self.backend.scroll(i32::MAX / 2);
        }
        cx.notify();
    }

    fn scroll_bottom(&mut self, _: &ScrollToBottom, _: &mut Window, cx: &mut Context<Self>) {
        if !self.send_to_program("shift-end", cx) {
            self.backend.scroll_to_bottom();
        }
        cx.notify();
    }
}

fn colors(theme: &ResolvedTheme) -> TerminalColors {
    let mut palette16: [RgbColor; 16] = [RgbColor::new(0, 0, 0); 16];
    for (slot, hex) in palette16.iter_mut().zip(theme.terminal_palette.iter()) {
        *slot = rgb_color(hex);
    }
    TerminalColors {
        foreground: rgb_color(&theme.terminal_fg),
        background: rgb_color(&theme.terminal_bg),
        palette: palette16,
    }
}

/// The terminal's font, without ligatures: each cell shows its own
/// character, as the Swift renderer draws them one by one.
fn terminal_font(family: &str) -> Font {
    let mut font = gpui_kit::font(crate::fonts::family(family));
    font.features = gpui_kit::FontFeatures(std::sync::Arc::new(vec![
        ("calt".into(), 0),
        ("liga".into(), 0),
    ]));
    font
}

/// `TerminalFontMetrics`: the cell is the width of "W" and the font's
/// ascent, descent and leading, rounded up.
fn measure(font: &Font, font_size: f32, window: &Window) -> Metrics {
    let size = px(font_size);
    let text = window.text_system();
    let id = text.resolve_font(font);
    let width = text
        .advance(id, size, 'W')
        .map(|s| s.width)
        .unwrap_or(px(font_size * 0.6));
    let (ascent, descent) = (text.ascent(id, size), text.descent(id, size).abs());
    Metrics {
        width: px(f32::from(width).ceil()),
        height: font_line_height(font, size, window),
        font_size: size,
        ascent,
        descent,
    }
}

/// A font's line height as AppKit lays it out (`CTFontGetAscent` +
/// descent + leading, rounded up): the terminal's rows and the input bar's
/// lines.
pub(crate) fn font_line_height(font: &Font, size: Pixels, window: &Window) -> Pixels {
    let text = window.text_system();
    let id = text.resolve_font(font);
    let height = text.ascent(id, size)
        + text.descent(id, size).abs()
        + px(line_gap(&font.family, f32::from(size)).unwrap_or(0.0));
    px(f32::from(height).ceil())
}

/// The leading (line gap) of `family` at `size`, which GPUI's text system
/// keeps to itself: CoreText's for the family on macOS, as Swift's
/// `TerminalFontMetrics` reads it; None for a family CoreText doesn't
/// have (fonts GPUI loaded from memory) and off macOS. Remembered per
/// family and size.
pub(crate) fn line_gap(family: &str, size: f32) -> Option<f32> {
    thread_local! {
        static GAPS: RefCell<std::collections::HashMap<(String, u32), Option<f32>>> =
            RefCell::default();
    }
    let key = (family.to_string(), size.to_bits());
    if let Some(gap) = GAPS.with(|g| g.borrow().get(&key).copied()) {
        return gap;
    }
    #[cfg(target_os = "macos")]
    let gap = core_text::leading(family, size);
    #[cfg(not(target_os = "macos"))]
    let gap = None;
    GAPS.with(|g| g.borrow_mut().insert(key, gap));
    gap
}

/// The family's fonts as CoreText matches them by family name (what
/// font-kit's `select_family_by_name` asks for, so the font GPUI draws
/// with), and the first one's leading.
#[cfg(target_os = "macos")]
mod core_text {
    use std::ffi::c_void;

    type Ref = *const c_void;
    #[repr(C)]
    struct Opaque([u8; 0]);
    const UTF8: u32 = 0x0800_0100;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFTypeDictionaryKeyCallBacks: Opaque;
        static kCFTypeDictionaryValueCallBacks: Opaque;
        fn CFStringCreateWithBytes(
            alloc: Ref,
            bytes: *const u8,
            len: isize,
            encoding: u32,
            external: u8,
        ) -> Ref;
        fn CFDictionaryCreate(
            alloc: Ref,
            keys: *const Ref,
            values: *const Ref,
            count: isize,
            key_callbacks: *const Opaque,
            value_callbacks: *const Opaque,
        ) -> Ref;
        fn CFArrayGetCount(array: Ref) -> isize;
        fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
        fn CFRelease(object: Ref);
    }

    #[link(name = "CoreText", kind = "framework")]
    extern "C" {
        static kCTFontFamilyNameAttribute: Ref;
        fn CTFontDescriptorCreateWithAttributes(attributes: Ref) -> Ref;
        fn CTFontDescriptorCreateMatchingFontDescriptors(descriptor: Ref, mandatory: Ref) -> Ref;
        fn CTFontCreateWithFontDescriptor(descriptor: Ref, size: f64, matrix: *const c_void)
            -> Ref;
        fn CTFontGetLeading(font: Ref) -> f64;
    }

    pub(super) fn leading(family: &str, size: f32) -> Option<f32> {
        // SAFETY: every object made here is released once, after its last
        // use; the array's values are borrowed (the Get rule).
        unsafe {
            let name = CFStringCreateWithBytes(
                std::ptr::null(),
                family.as_ptr(),
                family.len() as isize,
                UTF8,
                0,
            );
            if name.is_null() {
                return None;
            }
            let keys = [kCTFontFamilyNameAttribute];
            let values = [name];
            let attributes = CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            );
            CFRelease(name);
            if attributes.is_null() {
                return None;
            }
            let descriptor = CTFontDescriptorCreateWithAttributes(attributes);
            CFRelease(attributes);
            if descriptor.is_null() {
                return None;
            }
            let matches =
                CTFontDescriptorCreateMatchingFontDescriptors(descriptor, std::ptr::null());
            CFRelease(descriptor);
            if matches.is_null() {
                return None;
            }
            let leading = (CFArrayGetCount(matches) > 0).then(|| {
                let font = CTFontCreateWithFontDescriptor(
                    CFArrayGetValueAtIndex(matches, 0),
                    size as f64,
                    std::ptr::null(),
                );
                if font.is_null() {
                    return 0.0;
                }
                let leading = CTFontGetLeading(font) as f32;
                CFRelease(font);
                leading
            });
            CFRelease(matches);
            leading
        }
    }
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `TerminalRenderer.blockChipText`.
fn block_chip_text(exit_code: Option<i32>, duration_ms: Option<u64>) -> Option<String> {
    let exit_code = exit_code?;
    let mark = if exit_code == 0 {
        "✓".to_string()
    } else {
        format!("✗ {exit_code}")
    };
    Some(match duration_ms {
        Some(ms) => format!("{mark} · {}", format_block_duration(ms)),
        None => mark,
    })
}

/// `TerminalRenderer.formatBlockDuration`.
pub(crate) fn format_block_duration(ms: u64) -> String {
    if ms < 1000 {
        return format!("{ms}ms");
    }
    let seconds = ms as f64 / 1000.0;
    if seconds < 60.0 {
        return format!("{seconds:.1}s");
    }
    let minutes = seconds as u64 / 60;
    if minutes < 60 {
        return format!("{minutes}m {}s", seconds as u64 % 60);
    }
    format!("{}h {}m", minutes / 60, minutes % 60)
}

/// The gap a collapsed row leaves between two blocks (Swift's
/// `blockPadPixels`), and how many blank rows between blocks count as the
/// shell's prompt padding.
const BLOCK_GAP: f32 = 6.;
const MAX_PADDING_ROWS: i32 = 3;

/// Where each row is drawn, below the padding: the blank lines a shell
/// prints around its prompt, between two blocks, shrink to one `BLOCK_GAP`
/// (the Swift renderer's `collapsedRows`), and a block's separator sits in
/// the middle of that gap, so every block has the same room above and
/// below. Swift only collapses rows between a block's end mark and the
/// next prompt mark; blank rows ending the block above (a shell printing
/// its spacer before Impulse's marks) or starting the block below (a
/// prompt that begins with a newline) count too, so the spacing is even
/// whichever way the shell prints it.
#[derive(Clone, Default)]
pub(crate) struct RowLayout {
    /// Each row's top, and the content's bottom edge last.
    tops: Vec<Pixels>,
    collapsed: Vec<bool>,
    height: Pixels,
}

impl RowLayout {
    fn new(height: Pixels, collapsed: Vec<bool>) -> Self {
        let mut tops = Vec::with_capacity(collapsed.len() + 1);
        let mut y = px(0.);
        let mut previous = false;
        for &gap in &collapsed {
            tops.push(y);
            // A run of collapsed rows takes one gap in all.
            y += match (gap, previous) {
                (false, _) => height,
                (true, false) => px(BLOCK_GAP),
                (true, true) => px(0.),
            };
            previous = gap;
        }
        tops.push(y);
        Self {
            tops,
            collapsed,
            height,
        }
    }

    /// The blank padding rows between blocks that collapse, among those
    /// up to `last`: the blank rows ending the block above, the rows
    /// between the two and the blank rows starting the block below, when
    /// there are at most `MAX_PADDING_ROWS` of them.
    /// Rows under an image (`covered`) aren't blank.
    fn collapsed_rows(
        grid: &Grid,
        overlay: &BlockOverlay,
        last: i32,
        covered: &[bool],
    ) -> Vec<bool> {
        let mut collapsed = vec![false; grid.rows];
        if last < 0 {
            return collapsed;
        }
        let rows = grid.rows as i32;
        let blank = |row: i32| {
            row >= 0
                && row < rows
                && grid.is_blank_row(row as usize)
                && !covered.get(row as usize).copied().unwrap_or(false)
        };
        for pair in overlay.blocks.windows(2) {
            let (above, below) = (&pair[0], &pair[1]);
            // Each block keeps its first and last row.
            let mut from = above.end_row + 1;
            while from - 1 > above.start_row && blank(from - 1) {
                from -= 1;
            }
            let mut to = below.start_row;
            while to < below.end_row && blank(to) {
                to += 1;
            }
            if to <= from || to - from > MAX_PADDING_ROWS {
                continue;
            }
            for row in from.max(0)..to.min(last + 1) {
                if blank(row) {
                    collapsed[row as usize] = true;
                }
            }
        }
        collapsed
    }

    fn is_collapsed(&self, row: usize) -> bool {
        self.collapsed.get(row).copied().unwrap_or(false)
    }

    /// A block's first and last rows that are drawn, from its rows
    /// `start..=end` (its blank padding may be collapsed).
    fn first_shown(&self, start: usize, end: usize) -> usize {
        (start..=end)
            .find(|row| !self.is_collapsed(*row))
            .unwrap_or(start)
    }

    fn last_shown(&self, start: usize, end: usize) -> usize {
        (start..=end)
            .rev()
            .find(|row| !self.is_collapsed(*row))
            .unwrap_or(end)
    }

    /// Row `row`'s top.
    fn top(&self, row: usize) -> Pixels {
        match self.tops.len() {
            0 => self.height * row as f32,
            n if row < n => self.tops[row],
            n => self.tops[n - 1] + self.height * (row + 1 - n) as f32,
        }
    }

    /// A block's top edge, where its separator goes: in the middle of the
    /// collapsed gap above it, if there is one.
    fn border(&self, row: usize) -> Pixels {
        if row > 0 && self.is_collapsed(row - 1) {
            self.top(row) - px(BLOCK_GAP / 2.)
        } else {
            self.top(row)
        }
    }

    /// A block's bottom edge, when its last row is `row`: down to the
    /// middle of the collapsed gap below it, so its wash meets the next
    /// block's separator.
    fn bottom(&self, row: usize) -> Pixels {
        let bottom = self.top(row) + self.height;
        if self.is_collapsed(row + 1) {
            bottom + px(BLOCK_GAP / 2.)
        } else {
            bottom
        }
    }

    /// The row drawn at `y`; a collapsed gap counts as the row below it.
    fn row_at(&self, y: Pixels) -> usize {
        let rows = self.tops.len().saturating_sub(1);
        if rows == 0 {
            return (f32::from(y) / f32::from(self.height)).floor().max(0.) as usize;
        }
        let row = self.tops[..rows]
            .partition_point(|top| *top <= y)
            .saturating_sub(1);
        if self.is_collapsed(row) {
            (row..rows).find(|r| !self.is_collapsed(*r)).unwrap_or(row)
        } else if row + 1 == rows && y >= self.tops[rows] {
            // Below the content, rows keep their height.
            row + 1 + (f32::from(y - self.tops[rows]) / f32::from(self.height)).floor() as usize
        } else {
            row
        }
    }
}

/// Everything a frame paints, worked out before painting.
struct Frame {
    grid: Option<Rc<Grid>>,
    overlay: BlockOverlay,
    mode: TerminalMode,
    metrics: Metrics,
    font: Font,
    theme: ResolvedTheme,
    accent: KitRgba,
    selection: KitRgba,
    focused: bool,
    hide_cursor: bool,
    blink_on: bool,
    cursor_shape: String,
    marked: Option<String>,
    /// Block decorations are on and no program owns the grid.
    decorate: bool,
    /// Rows drawn: those above the prompt the input bar stands in for.
    draw_rows: usize,
    y_offset: Pixels,
    layout: RowLayout,
    /// Blocks moved to or selected (accent wash and stripe).
    emphasized: HashSet<u64>,
    bookmarks: HashSet<u64>,
    hovered: Option<u64>,
    hover_link: Option<HoverLink>,
    /// Hint targets: row, columns, label, and whether it fits what's typed.
    hints: Vec<(usize, usize, usize, String, bool)>,
    /// The 16 ANSI colors, when bold text in one of the first 8 takes its
    /// bright twin (`terminal_bold_is_bright`).
    bright_bold: Option<[RgbColor; 16]>,
    /// Images programs show, when there are any.
    images: Option<FrameImages>,
    /// Image textures no longer shown, to give back to the GPU.
    stale_images: Vec<std::sync::Arc<gpui_kit::RenderImage>>,
}

impl Frame {
    /// The sticky header: "❯ command" (cut to fit) and the block's status
    /// on a bar over the top row, with the failed or running stripe.
    fn paint_sticky_header(
        &self,
        block: &impulse_terminal::BlockOverlayRegion,
        command: &str,
        bounds: Bounds<Pixels>,
        (muted, red): (KitRgba, KitRgba),
        window: &mut Window,
        cx: &mut App,
    ) {
        let m = self.metrics;
        let bar_height = m.height + px(6.);
        let top = bounds.origin.y;
        window.paint_quad(fill(
            Bounds::new(bounds.origin, size(bounds.size.width, bar_height)),
            cell_hsla(rgb_color(&self.theme.terminal_bg)),
        ));
        window.paint_quad(fill(
            Bounds::new(
                point(bounds.origin.x, top + bar_height - px(1.)),
                size(bounds.size.width, px(1.)),
            ),
            rgba(muted.opacity(0.28)),
        ));
        if block.failed || block.is_running {
            let color = if block.failed { red } else { self.accent };
            window.paint_quad(fill(
                Bounds::new(
                    point(bounds.origin.x + px(3.), top),
                    size(px(3.), bar_height),
                ),
                rgba(color),
            ));
        }
        let shape = |text: String, font_size: Pixels, color: Hsla, window: &mut Window| {
            let text: SharedString = text.into();
            let run = TextRun {
                len: text.len(),
                font: self.font.clone(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(text, font_size, &[run], None)
        };
        // The command and its status share the bar's baseline, a row's
        // text's distance down.
        let baseline = top + px(3.) + m.ascent;
        let mut status_width = px(0.);
        if let Some(status) = block_chip_text(block.exit_code, block.duration_ms) {
            let color: Hsla = if block.failed {
                rgba(red).into()
            } else {
                rgba(muted).into()
            };
            let line = shape(status, chip_font_size(m.font_size), color, window);
            status_width = line.width;
            let x = bounds.origin.x + bounds.size.width - px(PADDING) - line.width;
            paint_on_baseline(&line, point(x, baseline), window, cx);
        }
        let room = bounds.size.width - px(PADDING * 2.) - status_width - px(16.);
        let full = format!("\u{276F} {command}");
        let mut chars: Vec<char> = full.chars().collect();
        let mut line = shape(full.clone(), m.font_size, rgba(muted).into(), window);
        while line.width > room && chars.len() > 3 {
            chars.pop();
            let cut: String = chars.iter().collect::<String>() + "\u{2026}";
            line = shape(cut, m.font_size, rgba(muted).into(), window);
        }
        paint_on_baseline(
            &line,
            point(bounds.origin.x + px(PADDING), baseline),
            window,
            cx,
        );
    }

    fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let bg = rgb_color(&self.theme.terminal_bg);
        window.paint_quad(fill(bounds, cell_hsla(bg)));
        let Some(grid) = &self.grid else { return };
        let rows = self.draw_rows.min(grid.rows);
        let m = self.metrics;
        let origin = bounds.origin + point(px(PADDING), px(PADDING) + self.y_offset);
        let layout = &self.layout;
        let cell_origin =
            |row: usize, col: usize| origin + point(m.width * col as f32, layout.top(row));
        // A block's whole height, separator to separator.
        let block_bounds = |start: usize, end: usize| {
            let top = origin.y + layout.border(start);
            Bounds::new(
                point(bounds.origin.x, top),
                size(bounds.size.width, origin.y + layout.bottom(end) - top),
            )
        };
        let muted = KitRgba::from_hex(&self.theme.fg_muted);
        let red = KitRgba::from_hex(&self.theme.red);
        let fg = KitRgba::from_hex(&self.theme.fg);
        let decorate = self.decorate
            && !self.overlay.alt_screen
            && !self.mode.contains(TerminalMode::ALT_SCREEN);
        let block_rows = |start: i32, end: i32| {
            let start = start.max(0) as usize;
            let end = (end.max(0) as usize).min(rows.saturating_sub(1));
            (start < rows).then_some(start..=end)
        };

        // Block washes under everything: the hovered block, the one moved
        // to or selected, failed ones.
        if decorate {
            for block in &self.overlay.blocks {
                let emphasized = self.emphasized.contains(&block.id);
                let color = if emphasized {
                    self.accent.opacity(0.09)
                } else if block.failed {
                    red.opacity(0.07)
                } else if self.hovered == Some(block.id) {
                    fg.opacity(0.05)
                } else {
                    continue;
                };
                if let Some(range) = block_rows(block.start_row, block.end_row) {
                    let (start, end) = (*range.start(), *range.end());
                    window.paint_quad(fill(
                        block_bounds(
                            layout.first_shown(start, end),
                            layout.last_shown(start, end),
                        ),
                        rgba(color),
                    ));
                }
            }
        }

        // Cell backgrounds, in runs.
        for row in (0..rows).filter(|r| !layout.is_collapsed(*r)) {
            let mut col = 0;
            while col < grid.cols {
                let cell = grid.cell(row, col);
                let span = if cell.flags.contains(CellFlags::WIDE_CHAR) {
                    2
                } else {
                    1
                };
                let cell_bg = if cell.flags.contains(CellFlags::INVERSE) {
                    cell.fg
                } else {
                    cell.bg
                };
                if cell.flags.contains(CellFlags::WIDE_CHAR_SPACER) || cell_bg == bg {
                    col += 1;
                    continue;
                }
                let start = col;
                col += span;
                while col < grid.cols {
                    let next = grid.cell(row, col);
                    let next_bg = if next.flags.contains(CellFlags::INVERSE) {
                        next.fg
                    } else {
                        next.bg
                    };
                    if next_bg != cell_bg {
                        break;
                    }
                    col += 1;
                }
                window.paint_quad(fill(
                    Bounds::new(
                        cell_origin(row, start),
                        size(m.width * (col - start) as f32, m.height),
                    ),
                    cell_hsla(cell_bg),
                ));
            }
        }

        // Images under the text (a negative z-index).
        let cell = (f32::from(m.width), f32::from(m.height));
        // A viewport row's top, rows above the viewport included.
        let row_top = |row: i32| {
            if row < 0 {
                origin.y + layout.top(0) + m.height * row as f32
            } else {
                origin.y + layout.top(row as usize)
            }
        };
        // From the first row drawn to the last: what's scrolled out of
        // view stays out, even where the rows are anchored to the bar.
        let image_clip = Bounds::from_corners(
            point(bounds.origin.x, origin.y),
            point(
                bounds.origin.x + bounds.size.width,
                origin.y + layout.top(rows),
            ),
        );
        let paint_placements = |under: bool, window: &mut Window| {
            let Some(images) = &self.images else { return };
            for p in images
                .layer
                .placements
                .iter()
                .filter(|p| (p.z < 0) == under)
            {
                let at = point(origin.x + m.width * p.col as f32, row_top(p.row));
                terminal_images::paint_placement(p, images, at, cell, image_clip, window);
            }
        };
        paint_placements(true, window);

        // Search matches, then the selection.
        let paint_spans = |spans: &[Span], color: Hsla, window: &mut Window| {
            for span in spans
                .iter()
                .filter(|s| s.row < rows && !layout.is_collapsed(s.row))
            {
                let end = span.end.min(grid.cols.saturating_sub(1));
                if end < span.start {
                    continue;
                }
                window.paint_quad(fill(
                    Bounds::new(
                        cell_origin(span.row, span.start),
                        size(m.width * (end - span.start + 1) as f32, m.height),
                    ),
                    color,
                ));
            }
        };
        let yellow = KitRgba::from_hex(&self.theme.yellow);
        paint_spans(&grid.matches, rgba(yellow.opacity(0.35)).into(), window);
        paint_spans(&grid.selection, rgba(self.selection).into(), window);

        // Block separators, stripes and bookmark ribbons.
        if decorate {
            for block in &self.overlay.blocks {
                if let Some(range) =
                    block_rows(block.start_row, block.end_row).filter(|_| block.start_row > 0)
                {
                    // A hairline just above the block's top edge.
                    let row = layout.first_shown(*range.start(), *range.end());
                    let y = px(f32::from(origin.y + layout.border(row)).round()) - px(1.);
                    window.paint_quad(fill(
                        Bounds::new(point(bounds.origin.x, y), size(bounds.size.width, px(1.))),
                        rgba(muted.opacity(0.28)),
                    ));
                }
                let emphasized = self.emphasized.contains(&block.id);
                if block.failed || block.is_running || emphasized {
                    let color = if block.failed { red } else { self.accent };
                    if let Some(range) = block_rows(block.start_row, block.end_row) {
                        let (start, end) = (
                            layout.first_shown(*range.start(), *range.end()),
                            layout.last_shown(*range.start(), *range.end()),
                        );
                        window.paint_quad(fill(
                            Bounds::new(
                                point(bounds.origin.x + px(3.), origin.y + layout.top(start)),
                                size(px(3.), layout.top(end) + m.height - layout.top(start)),
                            ),
                            rgba(color),
                        ));
                    }
                }
                if self.bookmarks.contains(&block.id)
                    && block.start_row >= 0
                    && (block.start_row as usize) < rows
                {
                    // A ribbon hanging from the block's top edge in the
                    // right margin.
                    let end = (block.end_row.max(0) as usize).min(rows.saturating_sub(1));
                    let row = layout.first_shown(block.start_row as usize, end);
                    let top = origin.y + layout.border(row);
                    let width = px(7.0f32.min((PADDING - 2.).max(4.)));
                    let left = bounds.origin.x + bounds.size.width - (px(PADDING) + width) / 2.;
                    let mut ribbon = gpui_kit::PathBuilder::fill();
                    ribbon.move_to(point(left, top));
                    ribbon.line_to(point(left + width, top));
                    ribbon.line_to(point(left + width, top + m.height));
                    ribbon.line_to(point(left + width / 2., top + m.height - px(4.)));
                    ribbon.line_to(point(left, top + m.height));
                    ribbon.close();
                    if let Ok(path) = ribbon.build() {
                        window.paint_path(path, rgba(self.accent));
                    }
                }
            }
        }

        // Text: each row shaped once, each glyph painted at its own cell
        // on the row's baseline (Swift's `drawTextRun`), whatever the
        // font's advances; then underlines and strikethroughs, cell by
        // cell. Lines and blocks are drawn, so neighbouring cells meet.
        let mut shaped = ShapedRow::default();
        for row in (0..rows).filter(|r| !layout.is_collapsed(*r)) {
            for col in 0..grid.cols {
                let cell = grid.cell(row, col);
                if crate::terminal_glyphs::is_geometric(cell.ch) {
                    crate::terminal_glyphs::paint(
                        cell.ch,
                        Bounds::new(cell_origin(row, col), size(m.width, m.height)),
                        cell_color(cell, self.bright_bold.as_ref()),
                        window,
                    );
                }
            }
            self.paint_text(&mut shaped, grid, row, 0..grid.cols, None, origin, window);
            self.paint_decorations(grid, row, origin, bounds, window);
        }

        // Images over the text: placeholder cells' parts of them, then the
        // direct placements.
        if let Some(images) = self.images.as_ref().filter(|_| !grid.images.is_empty()) {
            let mut cells = Vec::with_capacity(grid.cols);
            for row in (0..rows).filter(|r| !layout.is_collapsed(*r)) {
                cells.clear();
                cells.extend((0..grid.cols).map(|col| grid.image_cell(row, col)));
                for (col, len, first) in terminal_images::placeholder_runs(&cells) {
                    let at = cell_origin(row, col);
                    terminal_images::paint_placeholders(
                        &first, len, images, at, cell, image_clip, window,
                    );
                }
            }
        }
        paint_placements(false, window);

        // The link under the pointer, underlined in the cursor color.
        if let Some(link) = self.hover_link.as_ref().filter(|l| l.row < rows) {
            let at = cell_origin(link.row, link.start_col) + point(px(0.), m.height - px(1.));
            window.paint_quad(fill(
                Bounds::new(
                    at,
                    size(m.width * (link.end_col - link.start_col) as f32, px(1.)),
                ),
                cell_hsla(rgb_color(&self.theme.cursor)),
            ));
        }

        // The cursor: a block behind its character (drawn above as the
        // background color), a beam or underline, as the settings say,
        // focused or not (Swift's renderer has no hollow cursor); it blinks
        // only where typing goes. None while the input bar stands in for
        // the prompt.
        if let Some((row, col, _)) = grid
            .cursor
            .filter(|(r, c, _)| !self.hide_cursor && *r < rows && *c < grid.cols)
        {
            let cursor = cell_hsla(rgb_color(&self.theme.cursor));
            let at = cell_origin(row, col);
            let wide = grid.cell(row, col).flags.contains(CellFlags::WIDE_CHAR);
            let width = if wide { m.width * 2. } else { m.width };
            if self.blink_on || !self.focused {
                match self.cursor_shape.as_str() {
                    "beam" => {
                        window.paint_quad(fill(Bounds::new(at, size(px(2.), m.height)), cursor))
                    }
                    "underline" => window.paint_quad(fill(
                        Bounds::new(at + point(px(0.), m.height - px(2.)), size(width, px(2.))),
                        cursor,
                    )),
                    _ => {
                        window.paint_quad(fill(Bounds::new(at, size(width, m.height)), cursor));
                        // The character under it in its own weight and
                        // slant, in its cell's background color (the
                        // foreground when it's inverse).
                        let cell = grid.cell(row, col);
                        let color = cell_hsla(if cell.flags.contains(CellFlags::INVERSE) {
                            cell.fg
                        } else {
                            cell.bg
                        });
                        if crate::terminal_glyphs::is_geometric(cell.ch) {
                            crate::terminal_glyphs::paint(
                                cell.ch,
                                Bounds::new(at, size(m.width, m.height)),
                                color,
                                window,
                            );
                        } else {
                            let mut shaped = ShapedRow::default();
                            self.paint_text(
                                &mut shaped,
                                grid,
                                row,
                                col..col + 1,
                                Some(color),
                                origin,
                                window,
                            );
                        }
                    }
                }
            }
        }

        // Text an input method is composing, underlined at the cursor, even
        // where the program hides it (TUIs such as Claude Code do).
        let (row, col) = grid.cursor_at;
        if let Some(marked) = self
            .marked
            .as_ref()
            .filter(|t| !t.is_empty() && !self.hide_cursor && row < rows && col < grid.cols)
        {
            let at = cell_origin(row, col);
            let text: SharedString = marked.clone().into();
            let color = cell_hsla(rgb_color(&self.theme.terminal_fg));
            let run = TextRun {
                len: text.len(),
                font: self.font.clone(),
                color,
                background_color: Some(cell_hsla(bg)),
                underline: Some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(color),
                    wavy: false,
                }),
                strikethrough: None,
            };
            let line = window
                .text_system()
                .shape_line(text, m.font_size, &[run], None);
            let _ = line.paint(at, m.height, TextAlign::Left, None, window, cx);
        }

        // Exit status and duration at the right of each block's first row
        // (the hovered block shows its toolbar there instead), unless
        // glyphs are already there.
        if decorate {
            for block in &self.overlay.blocks {
                if block.start_row < 0
                    || block.start_row as usize >= rows
                    || self.hovered == Some(block.id)
                {
                    continue;
                }
                let Some(chip) = block_chip_text(block.exit_code, block.duration_ms) else {
                    continue;
                };
                let color: Hsla = if block.failed {
                    rgba(red).into()
                } else {
                    rgba(muted.opacity(0.75)).into()
                };
                let text: SharedString = chip.into();
                let run = TextRun {
                    len: text.len(),
                    font: self.font.clone(),
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line = window.text_system().shape_line(
                    text,
                    chip_font_size(m.font_size),
                    &[run],
                    None,
                );
                let span = (f32::from(line.width) / f32::from(m.width)).ceil() as usize + 2;
                let end = (block.end_row.max(0) as usize).min(rows.saturating_sub(1));
                let row = layout.first_shown(block.start_row as usize, end);
                if span >= grid.cols / 2
                    || (grid.cols - span..grid.cols).any(|col| {
                        let ch = grid.cell(row, col).ch;
                        ch != ' ' && ch != '\0'
                    })
                {
                    continue;
                }
                let x = bounds.origin.x + bounds.size.width - px(PADDING) - line.width;
                let baseline = origin.y + layout.top(row) + m.ascent;
                paint_on_baseline(&line, point(x, baseline), window, cx);
            }
        }

        // Warp-style sticky header: when a block starts above the viewport,
        // its command stays along the top edge so you know whose output
        // you're reading.
        if decorate && rows > 1 {
            let pinned = self
                .overlay
                .blocks
                .iter()
                .find(|b| b.start_row < 0 && b.end_row >= 1);
            if let Some((block, command)) = pinned.and_then(|b| {
                let command = b.command.as_deref().filter(|c| !c.is_empty())?;
                Some((b, command))
            }) {
                self.paint_sticky_header(block, command, bounds, (muted, red), window, cx);
            }
        }

        // Hints: each target underlined, its label in a pill over its start.
        for (row, start, end, label, fits) in &self.hints {
            if *row >= rows {
                continue;
            }
            let top = origin.y + layout.top(*row);
            let x = origin.x + m.width * *start as f32;
            window.paint_quad(fill(
                Bounds::new(
                    point(x, top + m.height - px(1.5)),
                    size(m.width * (end - start) as f32, px(1.5)),
                ),
                rgba(self.accent.opacity(if *fits { 0.9 } else { 0.25 })),
            ));
            if !fits {
                continue;
            }
            let text: SharedString = label.to_uppercase().into();
            let run = TextRun {
                len: text.len(),
                font: Font {
                    weight: FontWeight::BOLD,
                    ..self.font.clone()
                },
                color: cell_hsla(bg),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line = window
                .text_system()
                .shape_line(text, m.font_size * 0.85, &[run], None);
            let pill = Bounds::new(
                point(x - px(1.), top + px(1.)),
                size(line.width + px(8.), m.height - px(2.)),
            );
            window.paint_quad(fill(pill, rgba(self.accent)).corner_radii(px(3.)));
            let _ = line.paint(
                point(pill.origin.x + px(4.), top),
                m.height,
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
    }
}

impl Frame {
    /// Paint the text of `row`'s cells in `cols`: shaped as one line in
    /// runs by font (GPUI picks fallback fonts and emoji glyph by glyph),
    /// each glyph then put at its own cell's origin on the row's baseline
    /// (a wide character at its first cell), and a combining mark at its
    /// shaped offset from its base. In each cell's color, or in `color`
    /// (the character under a block cursor).
    #[allow(clippy::too_many_arguments)]
    fn paint_text(
        &self,
        shaped: &mut ShapedRow,
        grid: &Grid,
        row: usize,
        cols: Range<usize>,
        color: Option<Hsla>,
        origin: Point<Pixels>,
        window: &mut Window,
    ) {
        if !shaped.gather(grid, row, cols, &self.font) {
            return;
        }
        let m = self.metrics;
        let line = window
            .text_system()
            .layout_line(&shaped.text, m.font_size, &shaped.runs, None);
        // Each cell's own character's x in the shaped line, which its
        // marks are placed from (bidirectional text can put a mark's glyph
        // before its base's).
        shaped.base_x.clear();
        shaped.base_x.resize(grid.cols, None);
        for run in &line.runs {
            for glyph in &run.glyphs {
                if let Some((col, true)) = shaped.column(glyph.index) {
                    shaped.base_x[col].get_or_insert(glyph.position.x);
                }
            }
        }
        let baseline = origin.y + self.layout.top(row) + m.ascent;
        for run in &line.runs {
            for glyph in &run.glyphs {
                let Some((col, _)) = shaped.column(glyph.index) else {
                    continue;
                };
                let offset = shaped.base_x[col].map_or(px(0.), |x| glyph.position.x - x);
                let at = point(
                    origin.x + m.width * col as f32 + offset,
                    baseline + glyph.position.y,
                );
                let _ = if glyph.is_emoji {
                    window.paint_emoji(at, run.font_id, glyph.id, m.font_size)
                } else {
                    let color = color.unwrap_or_else(|| {
                        cell_color(grid.cell(row, col), self.bright_bold.as_ref())
                    });
                    window.paint_glyph(at, run.font_id, glyph.id, m.font_size, color)
                };
            }
        }
    }

    /// `row`'s underlines and strikethroughs, a run of cells at a time,
    /// where Swift draws them: an underline a point above the bottom of
    /// the descent, in the style SGR 4 gave it and SGR 58's color when
    /// set; a strikethrough through the middle of the row.
    fn paint_decorations(
        &self,
        grid: &Grid,
        row: usize,
        origin: Point<Pixels>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
    ) {
        let cells = grid.row_cells(row);
        let bright = self.bright_bold.as_ref();
        let m = self.metrics;
        let top = origin.y + self.layout.top(row);
        let x = |col: usize| origin.x + m.width * col as f32;
        let underlines = line_spans(cells, |cell| {
            let kind = underline_kind(cell.flags)?;
            let color = cell_color(cell, bright);
            let color = cell.underline.map_or(color, |c| Hsla {
                a: color.a,
                ..cell_hsla(c)
            });
            Some((kind, color))
        });
        let y = (top + m.ascent + m.descent - px(1.)).min(top + m.height - px(1.5));
        for (start, end, (kind, color)) in underlines {
            paint_underline(kind, x(start)..x(end), y, bounds.origin.x, color, window);
        }
        let strikes = line_spans(cells, |cell| {
            cell.flags
                .contains(CellFlags::STRIKETHROUGH)
                .then(|| cell_color(cell, bright))
        });
        for (start, end, color) in strikes {
            window.paint_quad(fill(
                Bounds::new(
                    point(x(start), top + m.height / 2. - px(0.5)),
                    size(x(end) - x(start), px(1.)),
                ),
                color,
            ));
        }
    }
}

/// One row's text to shape: the cells with something to draw as text
/// (blank, hidden and drawn cells left out), each with its combining
/// marks, in runs by font. Its buffers are kept from row to row.
#[derive(Default)]
struct ShapedRow {
    text: String,
    runs: Vec<TextRun>,
    /// Where each cell's text starts, and its column, in text order.
    starts: Vec<(usize, usize)>,
    /// Each column's own character's x in the shaped line.
    base_x: Vec<Option<Pixels>>,
}

impl ShapedRow {
    /// Gather the text of `row`'s cells in `cols`; false when none of
    /// them draws any.
    fn gather(&mut self, grid: &Grid, row: usize, cols: Range<usize>, font: &Font) -> bool {
        self.text.clear();
        self.runs.clear();
        self.starts.clear();
        for col in cols.start..cols.end.min(grid.cols) {
            let cell = grid.cell(row, col);
            let marks = grid.combining(row, col);
            if cell
                .flags
                .intersects(CellFlags::WIDE_CHAR_SPACER | CellFlags::HIDDEN)
                || (cell.ch == ' ' || cell.ch == '\0') && marks.is_none()
                || crate::terminal_glyphs::is_geometric(cell.ch)
            {
                continue;
            }
            let start = self.text.len();
            self.text.push(if cell.ch == '\0' { ' ' } else { cell.ch });
            if let Some(marks) = marks {
                self.text.push_str(marks);
            }
            self.starts.push((start, col));
            let len = self.text.len() - start;
            let weight = if cell.flags.contains(CellFlags::BOLD) {
                FontWeight::BOLD
            } else {
                FontWeight::NORMAL
            };
            let style = if cell.flags.contains(CellFlags::ITALIC) {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            };
            match self.runs.last_mut() {
                Some(run) if run.font.weight == weight && run.font.style == style => run.len += len,
                // One color for every run: glyphs are painted in their
                // cells' colors, so a color change doesn't shape the row
                // again.
                _ => self.runs.push(TextRun {
                    len,
                    font: Font {
                        weight,
                        style,
                        ..font.clone()
                    },
                    color: gpui_kit::black(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }),
            }
        }
        !self.text.is_empty()
    }

    /// The column whose text holds byte `index`, and whether it's where
    /// the cell's own character starts (not one of its marks).
    fn column(&self, index: usize) -> Option<(usize, bool)> {
        let i = self
            .starts
            .partition_point(|(start, _)| *start <= index)
            .checked_sub(1)?;
        let (start, col) = self.starts[i];
        Some((col, start == index))
    }
}

/// A cell's text color: the background when it's inverse, bold in one of
/// the 8 normal ANSI colors in its bright twin (`terminal_bold_is_bright`),
/// dimmed, or clear when hidden.
fn cell_color(cell: &Cell, bright_bold: Option<&[RgbColor; 16]>) -> Hsla {
    let f = cell.flags;
    let mut fg = if f.contains(CellFlags::INVERSE) {
        cell.bg
    } else {
        cell.fg
    };
    if let Some(palette) = bright_bold.filter(|_| f.contains(CellFlags::BOLD)) {
        if let Some(i) = palette[..8].iter().position(|c| *c == fg) {
            fg = palette[i + 8];
        }
    }
    let mut color = cell_hsla(fg);
    if f.contains(CellFlags::DIM) {
        color.a *= 0.6;
    }
    if f.contains(CellFlags::HIDDEN) {
        color.a = 0.0;
    }
    color
}

/// SGR 4's underline styles (4:1 to 4:5). An OSC 8 link isn't one: it's
/// underlined only under the pointer, as in Swift.
#[derive(Clone, Copy, Debug, PartialEq)]
enum UnderlineKind {
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

fn underline_kind(flags: CellFlags) -> Option<UnderlineKind> {
    Some(if flags.contains(CellFlags::UNDERCURL) {
        UnderlineKind::Curly
    } else if flags.contains(CellFlags::DOTTED_UNDERLINE) {
        UnderlineKind::Dotted
    } else if flags.contains(CellFlags::DASHED_UNDERLINE) {
        UnderlineKind::Dashed
    } else if flags.contains(CellFlags::DOUBLE_UNDERLINE) {
        UnderlineKind::Double
    } else if flags.contains(CellFlags::UNDERLINE) {
        UnderlineKind::Single
    } else {
        return None;
    })
}

/// Runs of neighbouring cells drawing the same line: (first column,
/// column after the last, the line). A wide character's spacer goes with
/// it.
fn line_spans<T: PartialEq>(
    cells: &[Cell],
    line: impl Fn(&Cell) -> Option<T>,
) -> Vec<(usize, usize, T)> {
    let mut spans: Vec<(usize, usize, T)> = Vec::new();
    for (col, cell) in cells.iter().enumerate() {
        if cell.flags.contains(CellFlags::WIDE_CHAR_SPACER) {
            if let Some(last) = spans.last_mut().filter(|s| s.1 == col) {
                last.1 = col + 1;
            }
            continue;
        }
        let Some(value) = line(cell) else {
            continue;
        };
        match spans.last_mut() {
            Some(last) if last.1 == col && last.2 == value => last.1 = col + 1,
            _ => spans.push((col, col + 1, value)),
        }
    }
    spans
}

/// One run of cells' underline, a point thick and centred on `y`, as
/// Swift's `drawUnderline` strokes it: a second line 2 points above for
/// double; dots, dashes and the wave laid out from the view's left edge
/// (`x0`), so they carry on across runs.
fn paint_underline(
    kind: UnderlineKind,
    span: Range<Pixels>,
    y: Pixels,
    x0: Pixels,
    color: Hsla,
    window: &mut Window,
) {
    let line = |l: Pixels, r: Pixels, y: Pixels, window: &mut Window| {
        window.paint_quad(fill(
            Bounds::new(point(l, y - px(0.5)), size(r - l, px(1.))),
            color,
        ))
    };
    match kind {
        UnderlineKind::Single => line(span.start, span.end, y, window),
        UnderlineKind::Double => {
            line(span.start, span.end, y, window);
            line(span.start, span.end, y - px(2.), window);
        }
        UnderlineKind::Dotted | UnderlineKind::Dashed => {
            let (on, period) = if kind == UnderlineKind::Dotted {
                (1., 3.)
            } else {
                (4., 6.)
            };
            let (l, r) = (f32::from(span.start - x0), f32::from(span.end - x0));
            for (l, r) in dashes(l..r, on, period) {
                line(x0 + px(l), x0 + px(r), y, window);
            }
        }
        UnderlineKind::Curly => {
            // Half waves 2 points long, 1.25 points high, clipped to the run.
            const AMPLITUDE: f32 = 1.25;
            const HALF: f32 = 2.;
            let mut x = x0 + px((f32::from(span.start - x0) / (HALF * 2.)).floor() * HALF * 2.);
            let mut wave = gpui_kit::PathBuilder::stroke(px(1.));
            wave.move_to(point(x, y));
            let mut up = true;
            while x < span.end {
                let peak = if up { -AMPLITUDE } else { AMPLITUDE } * 2.;
                wave.curve_to(
                    point(x + px(HALF), y),
                    point(x + px(HALF / 2.), y + px(peak)),
                );
                x += px(HALF);
                up = !up;
            }
            let clip = Bounds::new(
                point(span.start, y - px(AMPLITUDE + 1.)),
                size(span.end - span.start, px(AMPLITUDE * 2. + 2.)),
            );
            if let Ok(path) = wave.build() {
                window.with_content_mask(Some(gpui_kit::ContentMask { bounds: clip }), |window| {
                    window.paint_path(path, color)
                });
            }
        }
    }
}

/// A dash pattern's dashes, `on` long every `period` from 0, clipped to
/// `span`.
fn dashes(span: Range<f32>, on: f32, period: f32) -> impl Iterator<Item = (f32, f32)> {
    let first = (span.start / period).floor() * period;
    (0..)
        .map(move |i| first + i as f32 * period)
        .take_while(move |dash| *dash < span.end)
        .filter_map(move |dash| {
            let (l, r) = (dash.max(span.start), (dash + on).min(span.end));
            (l < r).then_some((l, r))
        })
}

/// A block status chip's font size: 85% of the terminal's, in whole
/// points (Swift's `chipFont`).
fn chip_font_size(font_size: Pixels) -> Pixels {
    px((f32::from(font_size) * 0.85).round())
}

/// Paint `line` with its baseline at `at.y`: with a line height of just
/// its ascent plus descent, GPUI puts the baseline `ascent` down.
fn paint_on_baseline(line: &ShapedLine, at: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let height = line.ascent + line.descent;
    let _ = line.paint(
        point(at.x, at.y - line.ascent),
        height,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|t| 0..t.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = None;
        if text.is_empty() || self.hints.is_some() {
            return;
        }
        if self.typing_goes_to_bar(cx) || !self.blocks.selection.is_empty() && self.bar_shown(cx) {
            // An unbound Ctrl key can arrive as its control character.
            let typed: String = text
                .chars()
                .filter(|c| !impulse_kit::shell::quoting::is_control(*c))
                .collect();
            self.end_block_selection(false, window, cx);
            if !typed.is_empty() {
                self.insert_into_bar(&typed, window, cx);
            }
            return;
        }
        self.backend.clear_selection();
        self.backend.write(&key_encoder::paste_bytes(text, false));
        self.typed();
        self.agent_key(false, cx);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = Some(text.to_string());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // At the cursor, so candidate windows open beside it.
        let (row, col) = self
            .buffer
            .get(4..8)
            .map(|b| {
                (
                    u16::from_le_bytes([b[0], b[1]]),
                    u16::from_le_bytes([b[2], b[3]]),
                )
            })
            .unwrap_or((0, 0));
        let m = self.metrics;
        Some(Bounds::new(
            element_bounds.origin
                + point(
                    px(PADDING) + m.width * col as f32,
                    px(PADDING) + self.y_offset + self.row_layout.top(row as usize),
                ),
            size(m.width, m.height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl TerminalView {
    /// Which rows to draw and how far down: with the input bar standing in
    /// for the prompt, an idle shell's prompt isn't drawn and the last
    /// output meets the bar (pushed down while it's shorter than the pane;
    /// with a screenful, the grid scrolls up once to tuck the prompt away).
    /// While a command runs, its output meets the bar until the pane fills.
    fn layout_rows(
        &mut self,
        grid: &Grid,
        overlay: &BlockOverlay,
        suppress: bool,
        decorate: bool,
        covered: &[bool],
        cx: &mut Context<Self>,
    ) {
        let rows = grid.rows as i32;
        let mut last = rows - 1;
        let mut anchor = false;
        // An image hanging below the last output is content too: one placed
        // by screen position leaves the cursor above it (`kitten icat
        // --place`), as kitty does.
        let below_images = covered
            .iter()
            .rposition(|&c| c)
            .map_or(-1, |row| row as i32);
        if !overlay.alt_screen && !self.direct {
            let fills = overlay.blocks.iter().any(|b| b.start_row < 0);
            let at_prompt = overlay.prompt_row.is_some();
            if suppress && at_prompt {
                last = if let Some(block) = overlay.blocks.last() {
                    block.end_row
                } else if overlay.has_blocks {
                    overlay.prompt_row.map_or(rows - 1, |p| p - 1)
                } else if let Some(rule) = overlay.restored_row {
                    // Before the first command, a restored session's output
                    // (ending at its "restored" rule) is the content, not
                    // the shell's greeting under it.
                    rule
                } else {
                    -1
                };
                // Without the blank lines shells print before a prompt.
                last = grid.last_non_blank_row(last, covered).max(below_images);
                anchor = !fills;
                if fills && self.tuck.armed {
                    let hidden = (rows - 1) - last;
                    if hidden <= 0 {
                        self.tuck.armed = false;
                    } else if !self.tuck.pending {
                        self.tuck.pending = true;
                        self.tuck.scrolled = true;
                        cx.spawn(async move |this, cx| {
                            let _ = this.update(cx, |view, cx| {
                                view.backend.scroll(hidden);
                                view.tuck.pending = false;
                                cx.notify();
                            });
                        })
                        .detach();
                    }
                }
            } else if let Some(cursor) = overlay.cursor_row {
                last = cursor.max(below_images);
                anchor = !fills;
                self.tuck.floor = None;
            }
            // Tucked: the prompt is out of view. Scrolling down stops here.
            if self.tuck.armed && self.tuck.scrolled && !self.tuck.pending && !at_prompt {
                self.tuck.armed = false;
                self.tuck.floor = Some(overlay.display_offset.max(0) as usize);
            }
        }
        let last = last.clamp(-1, rows - 1);
        self.last_content_row = (last < rows - 1).then_some(last.max(0) as usize);
        let draw_rows = (last + 1) as usize;
        let collapsed = if decorate && !overlay.alt_screen && !self.direct {
            RowLayout::collapsed_rows(grid, overlay, last, covered)
        } else {
            vec![false; grid.rows]
        };
        let collapsing = collapsed.contains(&true);
        let ch = self.metrics.height;
        self.row_layout = RowLayout::new(ch, collapsed);
        // The last row meets the bar; collapsed rows free space at the top.
        self.y_offset = if (anchor || collapsing) && last >= 0 {
            (ch * rows as f32 - (self.row_layout.top(last as usize) + ch)).max(px(0.))
        } else {
            px(0.)
        };
        self.draw_rows = draw_rows;
    }

    /// The toolbar over the hovered block: copy output, run again, more.
    fn block_toolbar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.blocks.hovered?;
        if self.direct || self.hints.is_some() {
            return None;
        }
        let overlay = self.overlay();
        let block = overlay.blocks.iter().find(|b| b.id == id)?;
        let row = block.start_row.max(0) as usize;
        if row >= self.draw_rows {
            return None;
        }
        let end = (block.end_row.max(0) as usize).min(self.draw_rows - 1);
        let row = self.row_layout.first_shown(row, end.max(row));
        let ch = self.metrics.height;
        let muted = KitRgba::from_hex(&self.theme.fg_muted);
        let bg = KitRgba::from_hex(&self.theme.terminal_bg);
        let mut buttons = vec![(ToolbarButton::CopyOutput, "copy", "Copy output")];
        if block.command.as_ref().is_some_and(|c| !c.is_empty()) && !block.is_running {
            buttons.push((ToolbarButton::Rerun, "rotate-ccw", "Rerun command"));
        }
        buttons.push((ToolbarButton::SendToAgent, "bot", "Send to agent"));
        buttons.push((ToolbarButton::Menu, "ellipsis", "More"));
        let top = px(PADDING) + self.y_offset + self.row_layout.top(row) + ch * 0.55;
        Some(
            div()
                .absolute()
                .top(top)
                .right(px(PADDING))
                .h(ch)
                .px(px(4.))
                .flex()
                .rounded(ch.min(px(14.)) / 2.)
                .bg(rgba(bg.opacity(0.96)))
                .border_1()
                .border_color(rgba(muted.opacity(0.28)))
                .children(buttons.into_iter().map(|(button, icon, help)| {
                    div()
                        .id(icon)
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(help).build(window, cx)
                        })
                        .w(px(26.))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|d| d.bg(rgba(muted.opacity(0.15))))
                        .child(crate::workbench_chrome::icon(icon, 12., muted.opacity(0.8)))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |view, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                match button {
                                    ToolbarButton::CopyOutput => {
                                        view.copy_block(id, false, true, cx)
                                    }
                                    ToolbarButton::Rerun => view.rerun_block(id, cx),
                                    ToolbarButton::SendToAgent => {
                                        view.send_blocks_to_agent(&[id], cx)
                                    }
                                    ToolbarButton::Menu => {
                                        view.open_block_menu(
                                            Some(id),
                                            e.position,
                                            false,
                                            window,
                                            cx,
                                        );
                                    }
                                }
                            }),
                        )
                }))
                .into_any_element(),
        )
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The window it's in now (the Quick Terminal's comes back in a new
        // one; a tab can move): blinking and attention follow it.
        self.window_handle = window.window_handle();
        let view = cx.entity();
        let focused = self.focus.is_focused(window);
        self.shown_focused = focused;
        let contains_focus = self.focus_handle().contains_focused(window, cx);
        if contains_focus {
            self.note_active();
        }
        self.agent_note_watched(window, cx);
        // A program taking the grid (or giving it back) moves the keyboard
        // between the grid and the bar.
        let mode = self.backend.mode();
        let raw = mode.intersects(
            TerminalMode::BRACKETED_PASTE
                | TerminalMode::MOUSE_REPORT_CLICK
                | TerminalMode::MOUSE_MOTION
                | TerminalMode::MOUSE_DRAG,
        );
        let direct = mode.contains(TerminalMode::ALT_SCREEN) || self.command_running && raw;
        if direct != self.direct {
            self.direct = direct;
            if direct {
                self.close_menu(cx);
            }
            if contains_focus {
                cx.defer_in(window, |view, window, cx| view.focus(window, cx));
            }
        }
        // Back at the prompt after a command: the bar takes the keyboard.
        if std::mem::take(&mut self.focus_bar_pending)
            && focused
            && self.find.is_none()
            && self.bar_shown(cx)
        {
            cx.defer_in(window, |view, window, cx| view.focus(window, cx));
        }
        let bottom = if self.bar_shown(cx) {
            Some(if self.has_live_bar() {
                self.input_bar(window, cx)
            } else {
                let height = self.bar_height(window, cx);
                self.bar_placeholder(height)
            })
        } else {
            None
        };
        let link_cursor = self.hover_link.is_some();
        let toolbar = self.block_toolbar(cx);
        let bounds_cell = std::rc::Rc::new(std::cell::Cell::new(Bounds::<Pixels>::default()));
        let (middle_bounds, middle_up_bounds) = (bounds_cell.clone(), bounds_cell.clone());
        let (down_bounds, move_bounds, up_bounds, wheel_bounds, menu_bounds) = (
            bounds_cell.clone(),
            bounds_cell.clone(),
            bounds_cell.clone(),
            bounds_cell.clone(),
            bounds_cell.clone(),
        );
        // For screen readers (Swift's renderer is an accessible read-only
        // text area): the screen's lines, worked out only while assistive
        // technology is attached.
        let screen_text = window
            .is_a11y_active()
            .then(|| self.grid_now())
            .flatten()
            .map(|grid| {
                let lines: Vec<String> =
                    (0..grid.rows).map(|row| grid.row_text(row).text).collect();
                impulse_kit::accessible_text::AccessibleText::new(&lines).string
            });
        let grid = div()
            .id("terminal")
            .role(gpui_kit::Role::Terminal)
            .aria_label(format!("Terminal, {}", self.title))
            .when_some(screen_text, |d, text| d.aria_value(text))
            .track_focus(&self.focus)
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .when(link_cursor, |d| d.cursor_pointer())
            .when(!link_cursor, |d| d.cursor(gpui_kit::CursorStyle::IBeam))
            .on_key_down(cx.listener(Self::key_down))
            .on_key_up(cx.listener(Self::key_up))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, e: &MouseDownEvent, window, cx| {
                    view.mouse_down(e, down_bounds.get(), window, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |view, e: &MouseDownEvent, window, cx| {
                    view.context_menu_at(e, menu_bounds.get(), window, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |view, e: &MouseDownEvent, window, cx| {
                    view.middle_down(e, middle_bounds.get(), window, cx)
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(move |view, e: &MouseUpEvent, _, _| {
                    view.middle_up(e, middle_up_bounds.get())
                }),
            )
            .on_mouse_move(cx.listener(move |view, e: &MouseMoveEvent, _, cx| {
                view.mouse_move(e, move_bounds.get(), cx)
            }))
            .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
                if !hovered {
                    view.mouse_left(cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |view, e: &MouseUpEvent, _, cx| {
                    view.mouse_up(e, up_bounds.get(), cx)
                }),
            )
            .on_scroll_wheel(cx.listener(move |view, e: &ScrollWheelEvent, _, cx| {
                view.scroll_wheel(e, wheel_bounds.get(), cx)
            }))
            .child(
                canvas(
                    {
                        let view = view.clone();
                        move |bounds, window, cx| {
                            bounds_cell.set(bounds);
                            let scale = window.scale_factor();
                            view.update(cx, |view, cx| {
                                view.fit(bounds, scale);
                                let (grid, overlay, mode) = view.snapshot();
                                let settings = SettingsStore::get(cx).clone();
                                let suppress = settings.terminal_context_bar && !view.no_bar;
                                let images = view.frame_images(grid.as_deref(), scale);
                                if let Some(grid) = &grid {
                                    let decorate = settings.terminal_blocks && !view.direct;
                                    let covered = images.as_ref().map_or_else(Vec::new, |images| {
                                        terminal_images::covered_rows(&images.layer, grid.rows)
                                    });
                                    view.layout_rows(
                                        grid, &overlay, suppress, decorate, &covered, cx,
                                    );
                                }
                                let mut emphasized: HashSet<u64> =
                                    view.blocks.selection.iter().copied().collect();
                                emphasized.extend(view.blocks.highlighted);
                                Frame {
                                    grid,
                                    overlay,
                                    mode,
                                    metrics: view.metrics,
                                    font: view.font.clone(),
                                    theme: view.theme.clone(),
                                    accent: view.palette.accent,
                                    selection: view.palette.selection_strong,
                                    focused,
                                    // The input bar owns the caret while
                                    // it stands in for the prompt.
                                    hide_cursor: view.bar_shown(cx),
                                    blink_on: view.blink_on,
                                    cursor_shape: settings.terminal_cursor_shape.clone(),
                                    marked: view.marked.clone(),
                                    decorate: settings.terminal_blocks && !view.direct,
                                    bright_bold: settings
                                        .terminal_bold_is_bright
                                        .then(|| colors(&view.theme).palette),
                                    draw_rows: view.draw_rows,
                                    y_offset: view.y_offset,
                                    layout: view.row_layout.clone(),
                                    emphasized,
                                    bookmarks: view.blocks.bookmarks.clone(),
                                    hovered: view.blocks.hovered,
                                    hover_link: view.hover_link.clone(),
                                    hints: view
                                        .hints
                                        .as_ref()
                                        .map(|h| {
                                            h.targets
                                                .iter()
                                                .map(|t| {
                                                    (
                                                        t.row,
                                                        t.start_col,
                                                        t.end_col,
                                                        t.label.clone(),
                                                        t.label.starts_with(&h.typed),
                                                    )
                                                })
                                                .collect()
                                        })
                                        .unwrap_or_default(),
                                    images,
                                    stale_images: view.textures.take_stale(),
                                }
                            })
                        }
                    },
                    move |bounds, mut frame: Frame, window, cx| {
                        let focus = view.read(cx).focus.clone();
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, view.clone()),
                            cx,
                        );
                        for texture in std::mem::take(&mut frame.stale_images) {
                            let _ = window.drop_image(texture);
                        }
                        frame.paint(bounds, window, cx);
                    },
                )
                .size_full(),
            )
            .children(toolbar);
        div()
            .id("terminal-pane")
            .track_focus(&self.root_focus)
            .key_context(CONTEXT)
            .size_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_drop(cx.listener(Self::drop_paths))
            .on_drop(cx.listener(Self::drop_tree_path))
            .on_modifiers_changed(cx.listener(
                |view, event: &gpui_kit::ModifiersChangedEvent, _, cx| {
                    view.modifiers_changed(&event.modifiers, cx)
                },
            ))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::clear))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::scroll_top))
            .on_action(cx.listener(Self::scroll_bottom))
            .children(self.find_bar(cx))
            .child(grid)
            .children(self.link_tooltip())
            .children(self.image_peek_view())
            .children(self.agent_bar(window, cx))
            .children(bottom)
            .children(self.block_menu(cx))
    }
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        // Not waiting for the shell: closing a tab of splits (or quitting)
        // would block on each one in turn.
        self.backend.close();
        for path in &self.temp_paths {
            let _ = if path.is_dir() {
                std::fs::remove_dir_all(path)
            } else {
                std::fs::remove_file(path)
            };
        }
    }
}

impl gpui_kit::Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root_focus.clone()
    }
}

/// A program's status (iTerm2's OSC 21337, the Swift
/// `TerminalSessionStatus`): its text, the indicator's and the status's
/// colors (`#RRGGBB`) and a detail.
#[derive(Clone, Default, PartialEq, Debug)]
pub(crate) struct SessionStatus {
    pub status: String,
    pub indicator: Option<String>,
    pub status_color: Option<String>,
    pub detail: String,
}

impl SessionStatus {
    /// An update: keys present replace (empty clears), others stay.
    pub(crate) fn apply(&mut self, fields: &std::collections::BTreeMap<String, String>) {
        if let Some(value) = fields.get("status") {
            self.status = value.clone();
        }
        if let Some(value) = fields.get("detail") {
            self.detail = value.clone();
        }
        if let Some(value) = fields.get("indicator") {
            self.indicator = Self::hex(value);
        }
        if let Some(value) = fields.get("status-color") {
            self.status_color = Self::hex(value);
        }
    }

    /// The tab's dot: the indicator's color, else the status's.
    pub(crate) fn dot_color(&self) -> Option<&str> {
        self.indicator.as_deref().or(self.status_color.as_deref())
    }

    /// "status: detail" for a tooltip, when there's a status.
    pub(crate) fn summary(&self) -> Option<String> {
        (!self.status.is_empty()).then(|| {
            if self.detail.is_empty() {
                self.status.clone()
            } else {
                format!("{}: {}", self.status, self.detail)
            }
        })
    }

    /// `#RRGGBB` or xterm's `rgb:RR/GG/BB` as `#RRGGBB`; `None` when empty or
    /// odd.
    fn hex(value: &str) -> Option<String> {
        if value.starts_with('#') && value.len() == 7 {
            return Some(value.to_string());
        }
        let parts: Vec<&str> = value.strip_prefix("rgb:")?.split('/').collect();
        let parts: Vec<&str> = parts.iter().map(|p| p.get(..2).unwrap_or(p)).collect();
        (parts.len() == 3
            && parts
                .iter()
                .all(|p| p.len() == 2 && u8::from_str_radix(p, 16).is_ok()))
        .then(|| format!("#{}", parts.concat()))
    }
}

#[cfg(test)]
mod session_status_tests {
    use super::SessionStatus;

    #[test]
    fn colors_and_updates() {
        let mut status = SessionStatus::default();
        let fields = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        status.apply(&fields(&[("status", "Building"), ("indicator", "#ffa500")]));
        assert_eq!(status.dot_color(), Some("#ffa500"));
        assert_eq!(status.summary().as_deref(), Some("Building"));
        status.apply(&fields(&[
            ("indicator", ""),
            ("status-color", "rgb:ff/00/7f"),
            ("detail", "step 2"),
        ]));
        assert_eq!(status.dot_color(), Some("#ff007f"));
        assert_eq!(status.summary().as_deref(), Some("Building: step 2"));
        status.apply(&fields(&[("status-color", "rgb:zz/00/00")]));
        assert_eq!(status.dot_color(), None);
    }
}

#[cfg(test)]
mod restore_tests {
    use super::{without_rule, RESTORED_RULE};

    #[test]
    fn a_saved_rule_is_left_for_the_new_one() {
        let rule = format!("\u{1b}[0;2m── {RESTORED_RULE} ──\u{1b}[0m\r\n");
        assert_eq!(
            without_rule(&format!("one\r\ntwo\r\n{rule}")),
            "one\r\ntwo\r\n"
        );
        assert_eq!(without_rule(&rule), "");
        // Commands ran after it: it stays where it was.
        let later = format!("one\r\n{rule}$ ls\r\nfile\r\n");
        assert_eq!(without_rule(&later), later);
        assert_eq!(without_rule("plain\r\n"), "plain\r\n");
    }
}

#[cfg(test)]
mod mouse_report_tests {
    use super::mouse_report;

    #[test]
    fn sgr_and_normal_reports() {
        // A drag of the middle button with Control, then its release.
        assert_eq!(
            mouse_report(1 + 32 + 16, 4, 2, false, true),
            b"\x1b[<49;5;3M"
        );
        assert_eq!(mouse_report(1, 4, 2, true, true), b"\x1b[<1;5;3m");
        // The normal form: plus 32, a release as button 3, far cells clamped.
        assert_eq!(mouse_report(0, 0, 0, false, false), b"\x1b[M\x20\x21\x21");
        assert_eq!(mouse_report(1, 0, 0, true, false), b"\x1b[M\x23\x21\x21");
        assert_eq!(mouse_report(0, 400, 0, false, false)[4], 255);
    }
}

#[cfg(test)]
mod pointer_tests {
    use super::*;

    #[test]
    fn a_plain_click_selects_only_once_it_drags() {
        assert!(matches!(
            press_selection(1, false),
            Press::Pending(SelectionKind::Simple)
        ));
        assert!(matches!(
            press_selection(1, true),
            Press::Pending(SelectionKind::Block)
        ));
        assert!(matches!(
            press_selection(2, false),
            Press::Now(SelectionKind::Semantic)
        ));
        assert!(matches!(
            press_selection(3, true),
            Press::Now(SelectionKind::Lines)
        ));
        assert!(!drag_begins((4, 2), (4, 2)));
        assert!(drag_begins((4, 2), (5, 2)));
        assert!(drag_begins((4, 2), (4, 3)));
    }

    #[test]
    fn wheel_notches_scroll_three_lines() {
        let cell = px(18.);
        assert_eq!(wheel_lines(ScrollDelta::Lines(point(0., 1.)), cell), 3.0);
        assert_eq!(wheel_lines(ScrollDelta::Lines(point(0., -2.)), cell), -6.0);
        assert_eq!(
            wheel_lines(ScrollDelta::Pixels(point(px(0.), px(36.))), cell),
            2.0
        );
    }
}

#[cfg(test)]
mod text_tests {
    use super::*;

    fn cell(ch: char, flags: CellFlags) -> Cell {
        Cell {
            ch,
            fg: RgbColor::new(200, 200, 200),
            bg: RgbColor::new(0, 0, 0),
            flags,
            underline: None,
        }
    }

    fn grid(cells: Vec<Cell>, zero_width: &[(usize, &str)]) -> Grid {
        Grid {
            cols: cells.len(),
            rows: 1,
            cursor: None,
            cursor_at: (0, 0),
            selection: Vec::new(),
            matches: Vec::new(),
            cells,
            zero_width: zero_width
                .iter()
                .map(|(i, s)| (*i, s.to_string()))
                .collect(),
            images: Default::default(),
        }
    }

    #[test]
    fn rows_shape_by_font_and_map_glyphs_back_to_cells() {
        let none = CellFlags::empty();
        let g = grid(
            vec![
                cell('a', none),
                cell('e', none),
                cell('中', CellFlags::WIDE_CHAR),
                cell(' ', CellFlags::WIDE_CHAR_SPACER),
                cell(' ', none),
                cell('\u{2500}', none),
                cell('x', CellFlags::HIDDEN),
                cell('b', CellFlags::BOLD),
                cell('c', CellFlags::BOLD),
            ],
            &[(1, "\u{301}")],
        );
        let mut shaped = ShapedRow::default();
        assert!(shaped.gather(&g, 0, 0..g.cols, &gpui_kit::font("Menlo")));
        // Blanks, spacers, drawn lines and hidden cells aren't text; marks
        // stay with their base.
        assert_eq!(shaped.text, "ae\u{301}中bc");
        assert_eq!(
            shaped.runs.iter().map(|r| r.len).collect::<Vec<_>>(),
            vec!["ae\u{301}中".len(), 2]
        );
        assert_eq!(shaped.runs[1].font.weight, FontWeight::BOLD);
        assert_eq!(shaped.column(0), Some((0, true)));
        assert_eq!(shaped.column(1), Some((1, true)));
        // The mark is in column 1, but not its own character.
        assert_eq!(shaped.column(2), Some((1, false)));
        // The wide character starts at byte 4 and takes columns 2 and 3.
        assert_eq!(shaped.column(4), Some((2, true)));
        assert_eq!(shaped.column(7), Some((7, true)));
        assert_eq!(shaped.column(8), Some((8, true)));
        // One cell alone (under a block cursor).
        assert!(shaped.gather(&g, 0, 7..8, &gpui_kit::font("Menlo")));
        assert_eq!(shaped.text, "b");
        assert!(!shaped.gather(&g, 0, 4..6, &gpui_kit::font("Menlo")));
    }

    #[test]
    fn decorations_run_cell_by_cell() {
        let under = CellFlags::UNDERLINE;
        let mut colored = cell('d', under);
        colored.underline = Some(RgbColor::new(255, 0, 0));
        let cells = vec![
            cell('a', under),
            cell('b', under),
            cell('中', under | CellFlags::WIDE_CHAR),
            cell(' ', under | CellFlags::WIDE_CHAR_SPACER),
            cell('c', CellFlags::UNDERCURL),
            colored,
            cell('e', CellFlags::HYPERLINK),
            cell('f', CellFlags::DOUBLE_UNDERLINE),
        ];
        let spans = line_spans(&cells, |cell| {
            let color = cell.underline.map_or(cell_color(cell, None), cell_hsla);
            Some((underline_kind(cell.flags)?, color))
        });
        let kinds: Vec<_> = spans.iter().map(|(s, e, (k, _))| (*s, *e, *k)).collect();
        assert_eq!(
            kinds,
            vec![
                // Through the wide character's second cell.
                (0, 4, UnderlineKind::Single),
                (4, 5, UnderlineKind::Curly),
                // Its own color starts a new run.
                (5, 6, UnderlineKind::Single),
                // A link isn't underlined until the pointer is on it.
                (7, 8, UnderlineKind::Double),
            ]
        );
        assert_eq!(
            underline_kind(CellFlags::DOTTED_UNDERLINE),
            Some(UnderlineKind::Dotted)
        );
        assert_eq!(
            underline_kind(CellFlags::DASHED_UNDERLINE),
            Some(UnderlineKind::Dashed)
        );
    }

    #[test]
    fn dashes_follow_the_view_not_the_run() {
        // Dots 1 long every 3, from 0: a run from 4 to 11 gets 6..7 and
        // 9..10; dashes 4 long every 6 are cut at both ends.
        assert_eq!(
            dashes(4.0..11.0, 1.0, 3.0).collect::<Vec<_>>(),
            vec![(6.0, 7.0), (9.0, 10.0)]
        );
        assert_eq!(
            dashes(2.0..15.0, 4.0, 6.0).collect::<Vec<_>>(),
            vec![(2.0, 4.0), (6.0, 10.0), (12.0, 15.0)]
        );
    }

    #[test]
    fn chips_use_whole_point_sizes() {
        assert_eq!(chip_font_size(px(14.)), px(12.));
        assert_eq!(chip_font_size(px(13.)), px(11.));
        assert_eq!(chip_font_size(px(15.)), px(13.));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod line_gap_tests {
    use super::line_gap;

    #[test]
    fn leading_comes_from_core_text_by_family() {
        // Menlo and SF Mono have none; a family CoreText lacks has no say.
        assert_eq!(line_gap("Menlo", 13.), Some(0.0));
        assert_eq!(line_gap(".AppleSystemUIFontMonospaced", 13.), Some(0.0));
        assert_eq!(line_gap("No Such Family 4f2a", 13.), None);
        // Times has one.
        assert!(line_gap("Times New Roman", 13.).is_some_and(|gap| gap > 0.0));
    }
}
