//! Terminal configuration and translation to alacritty types.

use std::collections::HashMap;
use std::path::PathBuf;

use alacritty_terminal::term::Config as AlacrittyConfig;
use alacritty_terminal::tty::{Options as PtyOptions, Shell};
use alacritty_terminal::vte::ansi::{
    CursorShape as AlacCursorShape, CursorStyle as AlacCursorStyle,
};
use serde::Deserialize;

use crate::grid::{CursorShape, RgbColor};

/// Terminal configuration provided by the frontend (deserialized from JSON).
#[derive(Deserialize)]
pub struct TerminalConfig {
    pub scrollback_lines: usize,
    pub cursor_shape: CursorShape,
    pub cursor_blink: bool,
    pub shell_path: String,
    pub shell_args: Vec<String>,
    pub working_directory: Option<String>,
    pub env_vars: HashMap<String, String>,
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
        Self {
            foreground: RgbColor::new(220, 215, 186),
            background: RgbColor::new(31, 31, 40),
            palette: [
                RgbColor::new(0, 0, 0),
                RgbColor::new(205, 49, 49),
                RgbColor::new(13, 188, 121),
                RgbColor::new(229, 229, 16),
                RgbColor::new(36, 114, 200),
                RgbColor::new(188, 63, 188),
                RgbColor::new(17, 168, 205),
                RgbColor::new(229, 229, 229),
                RgbColor::new(102, 102, 102),
                RgbColor::new(241, 76, 76),
                RgbColor::new(35, 209, 139),
                RgbColor::new(245, 245, 67),
                RgbColor::new(59, 142, 234),
                RgbColor::new(214, 112, 214),
                RgbColor::new(41, 184, 219),
                RgbColor::new(229, 229, 229),
            ],
        }
    }
}

impl TerminalConfig {
    /// Convert to alacritty's term Config.
    pub(crate) fn to_alacritty_config(&self) -> AlacrittyConfig {
        AlacrittyConfig {
            scrolling_history: self.scrollback_lines,
            default_cursor_style: AlacCursorStyle {
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

    /// Convert to alacritty's PTY Options.
    pub(crate) fn to_pty_options(&self) -> PtyOptions {
        let shell = if self.shell_path.is_empty() {
            None
        } else {
            Some(Shell::new(self.shell_path.clone(), self.shell_args.clone()))
        };
        PtyOptions {
            shell,
            working_directory: self.working_directory.as_ref().map(PathBuf::from),
            drain_on_exit: false,
            env: self.env_vars.clone(),
        }
    }
}
