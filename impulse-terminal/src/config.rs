//! Terminal configuration and settings translation.

use std::collections::HashMap;
use std::path::PathBuf;

use alacritty_terminal::term::Config as AlacrittyConfig;
use alacritty_terminal::tty::{Options as PtyOptions, Shell};
use alacritty_terminal::vte::ansi::CursorShape as AlacCursorShape;

use serde::Deserialize;

use crate::grid::{CursorShape, RgbColor};

/// Terminal configuration provided by the frontend.
#[derive(Deserialize)]
pub struct TerminalConfig {
    /// Number of scrollback history lines.
    pub scrollback_lines: usize,
    /// Cursor shape.
    pub cursor_shape: CursorShape,
    /// Whether the cursor should blink.
    pub cursor_blink: bool,
    /// Path to the shell executable.
    pub shell_path: String,
    /// Arguments to pass to the shell.
    pub shell_args: Vec<String>,
    /// Working directory for the shell.
    pub working_directory: Option<String>,
    /// Extra environment variables to set.
    pub env_vars: HashMap<String, String>,
    /// Terminal colors (foreground, background, 16-color ANSI palette).
    pub colors: TerminalColors,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            scrollback_lines: 10_000,
            cursor_shape: CursorShape::Block,
            cursor_blink: true,
            shell_path: String::new(),
            shell_args: Vec::new(),
            working_directory: None,
            env_vars: HashMap::new(),
            colors: TerminalColors::default(),
        }
    }
}

/// Terminal color palette.
#[derive(Deserialize)]
pub struct TerminalColors {
    pub foreground: RgbColor,
    pub background: RgbColor,
    /// 16-color ANSI palette (indices 0-15).
    pub palette: [RgbColor; 16],
}

impl Default for TerminalColors {
    fn default() -> Self {
        // Default to a reasonable dark theme (similar to xterm defaults).
        Self {
            foreground: RgbColor::new(220, 215, 186),
            background: RgbColor::new(31, 31, 40),
            palette: [
                RgbColor::new(0, 0, 0),       // Black
                RgbColor::new(205, 49, 49),    // Red
                RgbColor::new(13, 188, 121),   // Green
                RgbColor::new(229, 229, 16),   // Yellow
                RgbColor::new(36, 114, 200),   // Blue
                RgbColor::new(188, 63, 188),   // Magenta
                RgbColor::new(17, 168, 205),   // Cyan
                RgbColor::new(229, 229, 229),  // White
                RgbColor::new(102, 102, 102),  // Bright Black
                RgbColor::new(241, 76, 76),    // Bright Red
                RgbColor::new(35, 209, 139),   // Bright Green
                RgbColor::new(245, 245, 67),   // Bright Yellow
                RgbColor::new(59, 142, 234),   // Bright Blue
                RgbColor::new(214, 112, 214),  // Bright Magenta
                RgbColor::new(41, 184, 219),   // Bright Cyan
                RgbColor::new(229, 229, 229),  // Bright White
            ],
        }
    }
}

impl TerminalConfig {
    /// Convert to alacritty_terminal's term Config.
    pub(crate) fn to_alacritty_config(&self) -> AlacrittyConfig {
        AlacrittyConfig {
            scrolling_history: self.scrollback_lines,
            default_cursor_style: alacritty_terminal::vte::ansi::CursorStyle {
                shape: match self.cursor_shape {
                    CursorShape::Block => AlacCursorShape::Block,
                    CursorShape::Beam => AlacCursorShape::Beam,
                    CursorShape::Underline => AlacCursorShape::Underline,
                    CursorShape::HollowBlock => AlacCursorShape::HollowBlock,
                    CursorShape::Hidden => AlacCursorShape::Hidden,
                },
                blinking: self.cursor_blink,
            },
            ..Default::default()
        }
    }

    /// Convert to alacritty_terminal's PTY Options.
    pub(crate) fn to_pty_options(&self) -> PtyOptions {
        let shell = if self.shell_path.is_empty() {
            None
        } else {
            Some(Shell::new(self.shell_path.clone(), self.shell_args.clone()))
        };

        let working_directory = self.working_directory.as_ref().map(PathBuf::from);

        PtyOptions {
            shell,
            working_directory,
            drain_on_exit: false,
            env: self.env_vars.clone(),
        }
    }

}
