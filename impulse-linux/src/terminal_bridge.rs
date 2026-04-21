// SPDX-License-Identifier: GPL-3.0-only
//
// Terminal bridge QObject for the Linux QML frontend. Owns one
// impulse-terminal backend instance and exposes a rendered grid snapshot
// as JSON for a QML canvas-based renderer.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        include!("helpers.h");
        type QString = cxx_qt_lib::QString;

        fn impulse_clipboard_image_to_temp_png() -> QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, grid_json)]
        #[qproperty(QString, title)]
        #[qproperty(QString, current_directory)]
        #[qproperty(bool, is_running)]
        #[qproperty(i32, mode_bits)]
        #[qproperty(QString, error_message)]
        type TerminalBridge = super::TerminalBridgeRust;

        #[qinvokable]
        fn start(
            self: Pin<&mut TerminalBridge>,
            working_directory: &QString,
            theme_id: &QString,
            scrollback: i32,
            cursor_shape: &QString,
            cursor_blink: bool,
        );

        #[qinvokable]
        fn poll(self: Pin<&mut TerminalBridge>);

        #[qinvokable]
        fn resize_terminal(
            self: Pin<&mut TerminalBridge>,
            cols: i32,
            rows: i32,
            cell_width: i32,
            cell_height: i32,
        );

        #[qinvokable]
        fn send_text(self: Pin<&mut TerminalBridge>, text: &QString);

        #[qinvokable]
        fn scroll(self: Pin<&mut TerminalBridge>, delta: i32);

        #[qinvokable]
        fn set_focused(self: Pin<&mut TerminalBridge>, focused: bool);

        #[qinvokable]
        fn apply_theme(self: Pin<&mut TerminalBridge>, theme_id: &QString);

        #[qinvokable]
        fn start_selection(self: Pin<&mut TerminalBridge>, col: i32, row: i32, kind: i32);

        #[qinvokable]
        fn update_selection(self: Pin<&mut TerminalBridge>, col: i32, row: i32);

        #[qinvokable]
        fn clear_selection(self: Pin<&mut TerminalBridge>);

        #[qinvokable]
        fn select_all(self: Pin<&mut TerminalBridge>);

        #[qinvokable]
        fn selected_text(self: &TerminalBridge) -> QString;

        #[qinvokable]
        fn hyperlink_at(self: &TerminalBridge, col: i32, row: i32) -> QString;

        #[qinvokable]
        fn clipboard_image_path(self: Pin<&mut TerminalBridge>) -> QString;

        #[qinvokable]
        fn shutdown(self: Pin<&mut TerminalBridge>);
    }
}

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use impulse_terminal::{
    CellFlags, CursorShape, RgbColor, TerminalBackend, TerminalConfig, CELL_STRIDE,
    FIXED_HEADER_SIZE, RANGE_ENTRY_SIZE,
};
use serde::Serialize;
use std::path::Path;
use std::pin::Pin;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SegmentStyle {
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    dim: bool,
}

#[derive(Debug)]
struct PendingSegment {
    style: SegmentStyle,
    text: String,
    columns: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotRange {
    row: u16,
    start_col: u16,
    end_col: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotSegment {
    text: String,
    columns: u16,
    fg: String,
    bg: String,
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    dim: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotRow {
    segments: Vec<SnapshotSegment>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalSnapshot {
    cols: u16,
    rows: u16,
    cursor_row: u16,
    cursor_col: u16,
    cursor_shape: u8,
    cursor_visible: bool,
    selection_ranges: Vec<SnapshotRange>,
    search_ranges: Vec<SnapshotRange>,
    rows_data: Vec<SnapshotRow>,
}

pub struct TerminalBridgeRust {
    grid_json: QString,
    title: QString,
    current_directory: QString,
    is_running: bool,
    mode_bits: i32,
    error_message: QString,
    backend: Option<TerminalBackend>,
    grid_buffer: Vec<u8>,
    default_title: String,
    shell_temp_paths: Vec<std::path::PathBuf>,
}

impl Default for TerminalBridgeRust {
    fn default() -> Self {
        Self {
            grid_json: QString::from("{}"),
            title: QString::from("Terminal"),
            current_directory: QString::default(),
            is_running: false,
            mode_bits: 0,
            error_message: QString::default(),
            backend: None,
            grid_buffer: Vec::new(),
            default_title: "Terminal".to_string(),
            shell_temp_paths: Vec::new(),
        }
    }
}

impl Drop for TerminalBridgeRust {
    fn drop(&mut self) {
        self.cleanup_backend();
    }
}

impl TerminalBridgeRust {
    fn cleanup_backend(&mut self) {
        if let Some(backend) = self.backend.take() {
            backend.shutdown();
        }
        for path in self.shell_temp_paths.drain(..) {
            let _ = if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
        }
        self.grid_buffer.clear();
    }

    fn terminal_colors(theme_id: &str) -> impulse_terminal::TerminalColors {
        let theme = impulse_core::theme::get_theme(theme_id);
        let mut palette = [RgbColor::new(0, 0, 0); 16];
        for (idx, hex) in theme.terminal_palette.iter().take(16).enumerate() {
            palette[idx] = hex_to_rgb(hex);
        }
        impulse_terminal::TerminalColors {
            foreground: hex_to_rgb(&theme.fg),
            background: hex_to_rgb(&theme.bg),
            palette,
        }
    }

    fn build_terminal_config(
        working_directory: &str,
        theme_id: &str,
        scrollback: i32,
        cursor_shape: &str,
        cursor_blink: bool,
    ) -> Result<(TerminalConfig, Vec<std::path::PathBuf>, String), String> {
        let launch = impulse_core::shell::prepare_shell_launch_config()
            .map_err(|e| format!("Failed to prepare shell launch: {}", e))?;

        let mut config = TerminalConfig::default();
        config.scrollback_lines = scrollback.max(100) as usize;
        config.cursor_shape = parse_cursor_shape(cursor_shape);
        config.cursor_blink = cursor_blink;
        config.shell_path = launch.shell_path.clone();
        config.shell_args = launch.shell_args;
        config.env_vars = launch.env_vars;
        config.working_directory = if working_directory.is_empty() {
            None
        } else {
            Some(working_directory.to_string())
        };
        config.colors = Self::terminal_colors(theme_id);

        let title = Path::new(&launch.shell_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Terminal")
            .to_string();

        Ok((config, launch.temp_files, title))
    }

    fn ensure_buffer_for_backend(&mut self) {
        if let Some(backend) = self.backend.as_ref() {
            let required = backend.grid_buffer_size();
            if self.grid_buffer.len() < required {
                self.grid_buffer.resize(required, 0);
            }
        }
    }

    fn rebuild_snapshot(&mut self) -> Option<String> {
        self.ensure_buffer_for_backend();
        let backend = self.backend.as_mut()?;

        let written = backend.write_grid_to_buffer(&mut self.grid_buffer);
        if written == 0 {
            return None;
        }

        let buf = &self.grid_buffer[..written];
        let cols = read_u16(buf, 0);
        let rows = read_u16(buf, 2);
        let selection_count = read_u16(buf, 12) as usize;
        let search_count = read_u16(buf, 14) as usize;

        let mut offset = FIXED_HEADER_SIZE;
        let mut selection_ranges = Vec::with_capacity(selection_count);
        for _ in 0..selection_count {
            selection_ranges.push(SnapshotRange {
                row: read_u16(buf, offset),
                start_col: read_u16(buf, offset + 2),
                end_col: read_u16(buf, offset + 4),
            });
            offset += RANGE_ENTRY_SIZE;
        }

        let mut search_ranges = Vec::with_capacity(search_count);
        for _ in 0..search_count {
            search_ranges.push(SnapshotRange {
                row: read_u16(buf, offset),
                start_col: read_u16(buf, offset + 2),
                end_col: read_u16(buf, offset + 4),
            });
            offset += RANGE_ENTRY_SIZE;
        }

        let cell_data_offset = offset;
        let mut rows_data = Vec::with_capacity(rows as usize);

        for row in 0..rows as usize {
            let mut segments: Vec<SnapshotSegment> = Vec::new();
            let mut pending: Option<PendingSegment> = None;

            for col in 0..cols as usize {
                let cell_offset = cell_data_offset + (row * cols as usize + col) * CELL_STRIDE;
                let codepoint = read_u32(buf, cell_offset);
                let fg = (
                    buf[cell_offset + 4],
                    buf[cell_offset + 5],
                    buf[cell_offset + 6],
                );
                let bg = (
                    buf[cell_offset + 7],
                    buf[cell_offset + 8],
                    buf[cell_offset + 9],
                );
                let flags = CellFlags::from_bits_truncate(read_u16(buf, cell_offset + 10));
                if flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                    continue;
                }

                let mut fg_rgb = fg;
                let mut bg_rgb = bg;
                if flags.contains(CellFlags::INVERSE) {
                    std::mem::swap(&mut fg_rgb, &mut bg_rgb);
                }

                let style = SegmentStyle {
                    fg: fg_rgb,
                    bg: bg_rgb,
                    bold: flags.contains(CellFlags::BOLD),
                    italic: flags.contains(CellFlags::ITALIC),
                    underline: flags.intersects(
                        CellFlags::UNDERLINE
                            | CellFlags::DOUBLE_UNDERLINE
                            | CellFlags::UNDERCURL
                            | CellFlags::DOTTED_UNDERLINE
                            | CellFlags::DASHED_UNDERLINE,
                    ),
                    strikethrough: flags.contains(CellFlags::STRIKETHROUGH),
                    dim: flags.contains(CellFlags::DIM),
                };

                let mut ch = char::from_u32(codepoint).unwrap_or(' ');
                if codepoint == 0 || flags.contains(CellFlags::HIDDEN) {
                    ch = ' ';
                }
                let columns = if flags.contains(CellFlags::WIDE_CHAR) {
                    2
                } else {
                    1
                };

                match pending.as_mut() {
                    Some(segment) if segment.style == style => {
                        segment.text.push(ch);
                        segment.columns += columns;
                    }
                    Some(segment) => {
                        let replacement = PendingSegment {
                            style,
                            text: ch.to_string(),
                            columns,
                        };
                        let previous = std::mem::replace(segment, replacement);
                        segments.push(finalize_segment(previous));
                    }
                    None => {
                        pending = Some(PendingSegment {
                            style,
                            text: ch.to_string(),
                            columns,
                        });
                    }
                }
            }

            if let Some(segment) = pending.take() {
                segments.push(finalize_segment(segment));
            }

            rows_data.push(SnapshotRow { segments });
        }

        let snapshot = TerminalSnapshot {
            cols,
            rows,
            cursor_row: read_u16(buf, 4),
            cursor_col: read_u16(buf, 6),
            cursor_shape: buf[8],
            cursor_visible: buf[9] != 0,
            selection_ranges,
            search_ranges,
            rows_data,
        };

        serde_json::to_string(&snapshot).ok()
    }
}

impl qobject::TerminalBridge {
    pub fn start(
        mut self: Pin<&mut Self>,
        working_directory: &QString,
        theme_id: &QString,
        scrollback: i32,
        cursor_shape: &QString,
        cursor_blink: bool,
    ) {
        self.as_mut().shutdown();

        let cwd = working_directory.to_string();
        let theme = theme_id.to_string();
        let cursor = cursor_shape.to_string();

        match TerminalBridgeRust::build_terminal_config(
            &cwd,
            &theme,
            scrollback,
            &cursor,
            cursor_blink,
        ) {
            Ok((config, temp_files, default_title)) => {
                match TerminalBackend::new(config, 80, 24, 8, 16) {
                    Ok(backend) => {
                        let mode_bits = backend.mode().bits() as i32;
                        let current_dir = if cwd.is_empty() {
                            QString::from("/")
                        } else {
                            QString::from(cwd.as_str())
                        };

                        {
                            let mut rust = self.as_mut().rust_mut();
                            rust.backend = Some(backend);
                            rust.shell_temp_paths = temp_files;
                            rust.default_title = default_title.clone();
                            rust.current_directory = current_dir.clone();
                            rust.title = QString::from(default_title.as_str());
                            rust.is_running = true;
                            rust.mode_bits = mode_bits;
                            rust.error_message = QString::default();
                            rust.ensure_buffer_for_backend();
                        }

                        self.as_mut().set_current_directory(current_dir);
                        self.as_mut()
                            .set_title(QString::from(default_title.as_str()));
                        self.as_mut().set_is_running(true);
                        self.as_mut().set_mode_bits(mode_bits);
                        self.as_mut().set_error_message(QString::default());
                        if let Some(json) = self.as_mut().rust_mut().rebuild_snapshot() {
                            self.as_mut().set_grid_json(QString::from(json.as_str()));
                        }
                    }
                    Err(err) => {
                        let msg = QString::from(err.as_str());
                        self.as_mut().set_error_message(msg);
                        self.as_mut().set_is_running(false);
                    }
                }
            }
            Err(err) => {
                self.as_mut().set_error_message(QString::from(err.as_str()));
                self.as_mut().set_is_running(false);
            }
        }
    }

    pub fn poll(mut self: Pin<&mut Self>) {
        let events = match self.as_ref().rust().backend.as_ref() {
            Some(backend) => backend.poll_events(),
            None => return,
        };

        let mut needs_snapshot = false;
        let mut next_title: Option<String> = None;
        let mut next_cwd: Option<String> = None;
        let mut running = *self.is_running();

        for event in events {
            match event {
                impulse_terminal::TerminalEvent::Wakeup => {
                    needs_snapshot = true;
                }
                impulse_terminal::TerminalEvent::TitleChanged(title) => {
                    next_title = Some(title);
                }
                impulse_terminal::TerminalEvent::ResetTitle => {
                    next_title = Some(self.as_ref().rust().default_title.clone());
                }
                impulse_terminal::TerminalEvent::CwdChanged(path) => {
                    next_cwd = Some(path);
                }
                impulse_terminal::TerminalEvent::Exit
                | impulse_terminal::TerminalEvent::ChildExited(_) => {
                    running = false;
                }
                impulse_terminal::TerminalEvent::Bell
                | impulse_terminal::TerminalEvent::ClipboardStore(_)
                | impulse_terminal::TerminalEvent::ClipboardLoad
                | impulse_terminal::TerminalEvent::CursorBlinkingChange
                | impulse_terminal::TerminalEvent::PromptStart
                | impulse_terminal::TerminalEvent::CommandStart
                | impulse_terminal::TerminalEvent::CommandEnd(_)
                | impulse_terminal::TerminalEvent::PtyWrite(_) => {}
            }
        }

        if let Some(title) = next_title {
            self.as_mut().set_title(QString::from(title.as_str()));
        }
        if let Some(path) = next_cwd {
            self.as_mut()
                .set_current_directory(QString::from(path.as_str()));
        }
        if *self.is_running() != running {
            self.as_mut().set_is_running(running);
        }

        if let Some(backend) = self.as_ref().rust().backend.as_ref() {
            let mode_bits = backend.mode().bits() as i32;
            if *self.mode_bits() != mode_bits {
                self.as_mut().set_mode_bits(mode_bits);
            }
        }

        if needs_snapshot {
            if let Some(json) = self.as_mut().rust_mut().rebuild_snapshot() {
                self.as_mut().set_grid_json(QString::from(json.as_str()));
            }
        }
    }

    pub fn resize_terminal(
        mut self: Pin<&mut Self>,
        cols: i32,
        rows: i32,
        cell_width: i32,
        cell_height: i32,
    ) {
        let cols = cols.max(2) as u16;
        let rows = rows.max(1) as u16;
        let cell_width = cell_width.max(1) as u16;
        let cell_height = cell_height.max(1) as u16;

        {
            let mut rust = self.as_mut().rust_mut();
            if let Some(backend) = rust.backend.as_mut() {
                backend.resize(cols, rows, cell_width, cell_height);
            } else {
                return;
            }
        }

        self.as_mut().rust_mut().ensure_buffer_for_backend();
        if let Some(json) = self.as_mut().rust_mut().rebuild_snapshot() {
            self.as_mut().set_grid_json(QString::from(json.as_str()));
        }
    }

    pub fn send_text(self: Pin<&mut Self>, text: &QString) {
        let text = text.to_string();
        if text.is_empty() {
            return;
        }
        if let Some(backend) = self.rust().backend.as_ref() {
            backend.write(text.as_bytes());
        }
    }

    pub fn scroll(mut self: Pin<&mut Self>, delta: i32) {
        if let Some(backend) = self.as_ref().rust().backend.as_ref() {
            backend.scroll(delta);
        }
        if let Some(json) = self.as_mut().rust_mut().rebuild_snapshot() {
            self.as_mut().set_grid_json(QString::from(json.as_str()));
        }
    }

    pub fn set_focused(self: Pin<&mut Self>, focused: bool) {
        if let Some(backend) = self.rust().backend.as_ref() {
            backend.set_focus(focused);
        }
    }

    pub fn apply_theme(mut self: Pin<&mut Self>, theme_id: &QString) {
        let theme = theme_id.to_string();
        if let Some(backend) = self.as_mut().rust_mut().backend.as_mut() {
            let mut config = TerminalConfig::default();
            config.colors = TerminalBridgeRust::terminal_colors(&theme);
            backend.set_colors(&config);
            if let Some(json) = self.as_mut().rust_mut().rebuild_snapshot() {
                self.as_mut().set_grid_json(QString::from(json.as_str()));
            }
        }
    }

    pub fn start_selection(mut self: Pin<&mut Self>, col: i32, row: i32, kind: i32) {
        if let Some(backend) = self.as_ref().rust().backend.as_ref() {
            backend.start_selection(
                col.max(0) as usize,
                row.max(0) as usize,
                impulse_terminal::SelectionKind::from_u8(kind.max(0) as u8),
            );
        }
        if let Some(json) = self.as_mut().rust_mut().rebuild_snapshot() {
            self.as_mut().set_grid_json(QString::from(json.as_str()));
        }
    }

    pub fn select_all(mut self: Pin<&mut Self>) {
        if let Some(backend) = self.as_ref().rust().backend.as_ref() {
            backend.select_all();
        }
        if let Some(json) = self.as_mut().rust_mut().rebuild_snapshot() {
            self.as_mut().set_grid_json(QString::from(json.as_str()));
        }
    }

    pub fn hyperlink_at(&self, col: i32, row: i32) -> QString {
        if col < 0 || row < 0 {
            return QString::default();
        }
        self.rust()
            .backend
            .as_ref()
            .and_then(|backend| backend.hyperlink_at(col as usize, row as usize))
            .map(|uri| QString::from(uri.as_str()))
            .unwrap_or_default()
    }

    pub fn update_selection(mut self: Pin<&mut Self>, col: i32, row: i32) {
        if let Some(backend) = self.as_ref().rust().backend.as_ref() {
            backend.update_selection(col.max(0) as usize, row.max(0) as usize);
        }
        if let Some(json) = self.as_mut().rust_mut().rebuild_snapshot() {
            self.as_mut().set_grid_json(QString::from(json.as_str()));
        }
    }

    pub fn clear_selection(mut self: Pin<&mut Self>) {
        if let Some(backend) = self.as_ref().rust().backend.as_ref() {
            backend.clear_selection();
        }
        if let Some(json) = self.as_mut().rust_mut().rebuild_snapshot() {
            self.as_mut().set_grid_json(QString::from(json.as_str()));
        }
    }

    pub fn selected_text(&self) -> QString {
        self.rust()
            .backend
            .as_ref()
            .and_then(|backend| backend.selected_text())
            .map(|text| QString::from(text.as_str()))
            .unwrap_or_default()
    }

    pub fn clipboard_image_path(mut self: Pin<&mut Self>) -> QString {
        let path = qobject::impulse_clipboard_image_to_temp_png();
        let path_string = path.to_string();
        if path_string.is_empty() {
            return QString::default();
        }

        self.as_mut()
            .rust_mut()
            .shell_temp_paths
            .push(std::path::PathBuf::from(path_string.as_str()));
        path
    }

    pub fn shutdown(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().cleanup_backend();
        self.as_mut().set_is_running(false);
        self.as_mut().set_mode_bits(0);
    }
}

fn read_u16(buf: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([buf[offset], buf[offset + 1]])
}

fn read_u32(buf: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
    ])
}

fn hex_to_rgb(hex: &str) -> RgbColor {
    let cleaned = hex.trim().trim_start_matches('#');
    if cleaned.len() != 6 {
        return RgbColor::new(0, 0, 0);
    }

    let parse = |start| u8::from_str_radix(&cleaned[start..start + 2], 16).ok();
    match (parse(0), parse(2), parse(4)) {
        (Some(r), Some(g), Some(b)) => RgbColor::new(r, g, b),
        _ => RgbColor::new(0, 0, 0),
    }
}

fn rgb_to_hex(rgb: (u8, u8, u8)) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb.0, rgb.1, rgb.2)
}

fn finalize_segment(segment: PendingSegment) -> SnapshotSegment {
    SnapshotSegment {
        text: segment.text,
        columns: segment.columns,
        fg: rgb_to_hex(segment.style.fg),
        bg: rgb_to_hex(segment.style.bg),
        bold: segment.style.bold,
        italic: segment.style.italic,
        underline: segment.style.underline,
        strikethrough: segment.style.strikethrough,
        dim: segment.style.dim,
    }
}

fn parse_cursor_shape(shape: &str) -> CursorShape {
    match shape.to_ascii_lowercase().as_str() {
        "beam" | "line" => CursorShape::Beam,
        "underline" => CursorShape::Underline,
        "hollowblock" => CursorShape::HollowBlock,
        "hidden" => CursorShape::Hidden,
        _ => CursorShape::Block,
    }
}
