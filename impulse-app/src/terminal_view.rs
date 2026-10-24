//! A terminal pane: `impulse-terminal` drives the PTY and grid, GPUI draws
//! it. Command-block decorations follow `TerminalRenderer.swift`: a hairline
//! above each block, a wash and left stripe for failed blocks, and the exit
//! status and duration at the right of the block's first row.

use std::time::Duration;

use gpui::{div, prelude::*, px, Context, FontWeight, Hsla, Pixels, Rgba, SharedString, Window};
use impulse_kit::chrome::{ChromePalette, Rgba as KitRgba};
use impulse_kit::theme::ResolvedTheme;
use impulse_terminal::{
    BlockOverlay, CellFlags, RgbColor, TerminalBackend, TerminalColors, TerminalConfig,
    TerminalEvent, CELL_STRIDE, FIXED_HEADER_SIZE, RANGE_ENTRY_SIZE,
};

/// Same inset as the Swift renderer's `padding`.
const PADDING: f32 = 12.0;

pub fn rgba(c: KitRgba) -> Rgba {
    Rgba {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
}

fn cell_rgba(c: RgbColor) -> Rgba {
    Rgba {
        r: c.r as f32 / 255.0,
        g: c.g as f32 / 255.0,
        b: c.b as f32 / 255.0,
        a: 1.0,
    }
}

fn rgb_color(hex: &str) -> RgbColor {
    let c = KitRgba::from_hex(hex);
    RgbColor::new(
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
    )
}

/// One visible cell, decoded from the grid snapshot.
#[derive(Clone, Copy)]
struct Cell {
    ch: char,
    fg: RgbColor,
    bg: RgbColor,
    flags: CellFlags,
}

/// The grid snapshot `write_grid_to_buffer` fills (the same bytes the Swift
/// `GridBufferReader` parses).
struct Grid {
    cols: usize,
    rows: usize,
    cursor: Option<(usize, usize)>,
    cells: Vec<Cell>,
}

impl Grid {
    fn read(buf: &[u8]) -> Option<Grid> {
        if buf.len() < FIXED_HEADER_SIZE {
            return None;
        }
        let u16_at = |at: usize| u16::from_le_bytes([buf[at], buf[at + 1]]) as usize;
        let (cols, rows) = (u16_at(0), u16_at(2));
        let (cursor_row, cursor_col, cursor_visible) = (u16_at(4), u16_at(6), buf[9] != 0);
        let ranges = u16_at(12) + u16_at(14);
        let start = FIXED_HEADER_SIZE + ranges * RANGE_ENTRY_SIZE;
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
                }
            })
            .collect();
        Some(Grid {
            cols,
            rows,
            cursor: cursor_visible.then_some((cursor_row, cursor_col)),
            cells,
        })
    }
}

pub struct TerminalView {
    backend: TerminalBackend,
    buffer: Vec<u8>,
    theme: ResolvedTheme,
    palette: ChromePalette,
    font_family: SharedString,
    font_size: f32,
    cell_width: f32,
    line_height: f32,
    size: (u16, u16),
    pub cwd: String,
    pub last_duration_ms: Option<u64>,
    _poll: gpui::Task<()>,
}

impl TerminalView {
    pub fn new(
        theme: ResolvedTheme,
        palette: ChromePalette,
        cwd: String,
        commands: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let font_family: SharedString = "JetBrains Mono".into();
        let font_size = 14.0;
        let font_id = window
            .text_system()
            .resolve_font(&gpui::font(font_family.clone()));
        let cell_width = window
            .text_system()
            .advance(font_id, px(font_size), 'm')
            .map(|s| f32::from(s.width))
            .unwrap_or(font_size * 0.6);
        let line_height = (font_size * 1.4).round();

        let mut palette16: [RgbColor; 16] = [RgbColor::new(0, 0, 0); 16];
        for (slot, hex) in palette16.iter_mut().zip(theme.terminal_palette.iter()) {
            *slot = rgb_color(hex);
        }
        let config = TerminalConfig {
            shell_path: "/bin/bash".into(),
            shell_args: vec!["--rcfile".into(), write_bash_rc()],
            working_directory: Some(cwd.clone()),
            colors: TerminalColors {
                foreground: rgb_color(&theme.terminal_fg),
                background: rgb_color(&theme.terminal_bg),
                palette: palette16,
            },
            ..Default::default()
        };
        let size = (100, 30);
        let backend = TerminalBackend::new(
            config,
            size.0,
            size.1,
            cell_width as u16,
            line_height as u16,
        )
        .expect("terminal backend");
        for command in commands {
            backend.write(format!("{command}\r").as_bytes());
        }

        // The Swift app redraws from a wake callback; polling at the frame
        // rate is enough for the spike.
        let poll = cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            let alive = this.update(cx, |view, cx| {
                let mut changed = false;
                for event in view.backend.poll_events() {
                    match event {
                        TerminalEvent::CwdChanged(path) => view.cwd = path,
                        TerminalEvent::CommandBlockEnded(block) => {
                            view.last_duration_ms = block
                                .ended_at_ms
                                .map(|end| end.saturating_sub(block.started_at_ms));
                        }
                        _ => {}
                    }
                    changed = true;
                }
                if !matches!(view.backend.take_damage(), Some(rows) if rows.is_empty()) {
                    changed = true;
                }
                if changed {
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        });

        Self {
            backend,
            buffer: Vec::new(),
            theme,
            palette,
            font_family,
            font_size,
            cell_width,
            line_height,
            size,
            cwd,
            last_duration_ms: None,
            _poll: poll,
        }
    }

    pub fn write(&self, bytes: &[u8]) {
        self.backend.write(bytes);
    }

    /// Fit the grid to the pane's size.
    pub fn fit(&mut self, width: Pixels, height: Pixels) {
        let cols = ((f32::from(width) - PADDING * 2.0) / self.cell_width)
            .floor()
            .max(20.0) as u16;
        let rows = ((f32::from(height) - PADDING) / self.line_height)
            .floor()
            .max(5.0) as u16;
        if (cols, rows) != self.size {
            self.size = (cols, rows);
            self.backend
                .resize(cols, rows, self.cell_width as u16, self.line_height as u16);
        }
    }

    fn row_element(&self, grid: &Grid, row: usize, overlay: &BlockOverlay) -> impl IntoElement {
        let p = &self.palette;
        let muted = KitRgba::from_hex(&self.theme.fg_muted);
        let red = KitRgba::from_hex(&self.theme.red);
        let block = overlay
            .blocks
            .iter()
            .find(|b| b.start_row <= row as i32 && row as i32 <= b.end_row);
        let starts_block = overlay.blocks.iter().find(|b| b.start_row == row as i32);

        // Runs of cells sharing colors and weight.
        let mut runs: Vec<(String, usize, Cell)> = Vec::new();
        for col in 0..grid.cols {
            let mut cell = grid.cells[row * grid.cols + col];
            if cell.flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                continue;
            }
            if cell.flags.contains(CellFlags::INVERSE) {
                std::mem::swap(&mut cell.fg, &mut cell.bg);
            }
            if grid.cursor == Some((row, col)) {
                let cursor = rgb_color(&self.theme.cursor);
                cell.fg = cell.bg;
                cell.bg = cursor;
            }
            let width = if cell.flags.contains(CellFlags::WIDE_CHAR) {
                2
            } else {
                1
            };
            let ch = if cell.ch == '\0' { ' ' } else { cell.ch };
            match runs.last_mut() {
                Some((text, cells, last))
                    if last.fg == cell.fg
                        && last.bg == cell.bg
                        && last.flags.contains(CellFlags::BOLD)
                            == cell.flags.contains(CellFlags::BOLD) =>
                {
                    text.push(ch);
                    *cells += width;
                }
                _ => runs.push((ch.to_string(), width, cell)),
            }
        }
        let default_bg = rgb_color(&self.theme.terminal_bg);

        let mut line = div().flex().flex_row().h(px(self.line_height)).relative();
        if block.is_some_and(|b| b.failed) {
            line = line.bg(rgba(red.opacity(0.07)));
        }
        if starts_block.is_some() && row > 0 {
            line = line.border_t_1().border_color(rgba(muted.opacity(0.28)));
        }
        if block.is_some_and(|b| b.failed || b.is_running) {
            let stripe = if block.unwrap().failed { red } else { p.accent };
            line = line.child(
                div()
                    .absolute()
                    .left(px(3.0 - PADDING))
                    .top_0()
                    .w(px(3.0))
                    .h_full()
                    .bg(rgba(stripe)),
            );
        }
        for (text, cells, cell) in runs {
            let mut run = div()
                .w(px(cells as f32 * self.cell_width))
                .flex_none()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(cell_rgba(cell.fg))
                .child(text);
            if cell.bg != default_bg {
                run = run.bg(cell_rgba(cell.bg));
            }
            if cell.flags.contains(CellFlags::BOLD) {
                run = run.font_weight(FontWeight::BOLD);
            }
            line = line.child(run);
        }
        if let Some(chip) = starts_block
            .and_then(|b| block_chip_text(b.exit_code, b.duration_ms).map(|t| (t, b.failed)))
        {
            let color: Hsla = if chip.1 {
                rgba(red).into()
            } else {
                rgba(muted.opacity(0.75)).into()
            };
            line = line.child(
                div()
                    .absolute()
                    .right_0()
                    .top_0()
                    .text_size(px(12.0))
                    .text_color(color)
                    .child(chip.0),
            );
        }
        line
    }
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
fn format_block_duration(ms: u64) -> String {
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

/// bash started the way `TerminalTab` starts it: `--rcfile` with the user's
/// login files and then Impulse's shell integration. The prompt imitates the
/// docs' demo prompt.
fn write_bash_rc() -> String {
    let integration =
        include_str!("../../impulse-macos/Sources/ImpulseKit/Resources/ShellIntegration/bash.sh");
    let rc = format!(
        r#"[ -f /etc/profile ] && source /etc/profile
__branch() {{ git rev-parse --abbrev-ref HEAD 2>/dev/null; }}
PS1='\[\e[34m\]\w \[\e[90m\]$(__branch)\n\[\e[35m\]❯\[\e[0m\] '
{integration}
"#
    );
    let path = std::env::temp_dir().join(format!("impulse-bash-rc-{}", std::process::id()));
    std::fs::write(&path, rc).expect("write rc");
    path.to_string_lossy().into_owned()
}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let needed = self.backend.grid_buffer_size();
        if self.buffer.len() < needed {
            self.buffer.resize(needed, 0);
        }
        let written = self.backend.write_grid_to_buffer(&mut self.buffer);
        let grid = Grid::read(&self.buffer[..written]);
        let overlay = self.backend.block_overlay();
        let mut column = div()
            .size_full()
            .bg(rgba(KitRgba::from_hex(&self.theme.terminal_bg)))
            .pt(px(PADDING / 2.0))
            .px(px(PADDING))
            .font_family(self.font_family.clone())
            .text_size(px(self.font_size))
            .line_height(px(self.line_height))
            .flex()
            .flex_col()
            .overflow_hidden();
        if let Some(grid) = grid {
            for row in 0..grid.rows {
                column = column.child(self.row_element(&grid, row, &overlay));
            }
        }
        column
    }
}
