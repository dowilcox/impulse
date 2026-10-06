//! Terminal configuration and translation to alacritty types.

use std::collections::HashMap;
use std::path::PathBuf;

use alacritty_terminal::term::{Config as AlacrittyConfig, Osc52};
use alacritty_terminal::tty::{Options as PtyOptions, Shell};

use alacritty_terminal::vte::ansi::{
    CursorShape as AlacCursorShape, CursorStyle as AlacCursorStyle,
};
use serde::Deserialize;

use crate::grid::{CursorShape, RgbColor};

/// Variables a terminal's shell doesn't inherit from the app: color
/// overrides from wherever Impulse was launched, and the window ids
/// alacritty_terminal always adds, which make every shell look like it runs
/// in Alacritty.
const UNSET_IN_CHILD: &[&str] = &[
    "NO_COLOR",
    "CLICOLOR",
    "CLICOLOR_FORCE",
    "FORCE_COLOR",
    "ALACRITTY_WINDOW_ID",
    "WINDOWID",
];

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
    /// Minimum WCAG contrast ratio (1.0–21.0) enforced between every cell's
    /// foreground and its background at render time. Foregrounds below it are
    /// nudged toward black or white until they comply; 1.0 disables the
    /// adjustment. This fixes app-chosen low-contrast pairs (e.g. dark text
    /// on a dark selection bar) that no theme palette can prevent.
    #[serde(default = "default_minimum_contrast")]
    pub minimum_contrast: f32,
    /// Output from a previous session (see `transcript`), replayed into the
    /// grid before the shell starts so its scrollback comes back.
    #[serde(default)]
    pub restored_transcript: Option<String>,
    /// Programs may set the clipboard (OSC 52 store).
    #[serde(default = "default_true")]
    pub allow_clipboard_write: bool,
    /// Programs may read the clipboard (OSC 52 load). Off unless asked for.
    #[serde(default)]
    pub allow_clipboard_read: bool,
}

fn default_true() -> bool {
    true
}

fn default_minimum_contrast() -> f32 {
    1.0
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
            minimum_contrast: 1.0,
            restored_transcript: None,
            allow_clipboard_write: true,
            allow_clipboard_read: false,
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
            // Programs can ask for the kitty keyboard protocol (CSI > u).
            kitty_keyboard: true,
            osc52: match (self.allow_clipboard_write, self.allow_clipboard_read) {
                (true, true) => Osc52::CopyPaste,
                (true, false) => Osc52::OnlyCopy,
                (false, true) => Osc52::OnlyPaste,
                (false, false) => Osc52::Disabled,
            },
            ..Default::default()
        }
    }

    /// Convert to alacritty's PTY Options.
    ///
    /// The shell starts through `env -u`, which removes `UNSET_IN_CHILD` and
    /// then execs it (same process). alacritty_terminal can only add
    /// variables to the child, and changing the app's own environment around
    /// the spawn isn't safe while other threads read it.
    pub(crate) fn to_pty_options(&self) -> PtyOptions {
        let shell = if self.shell_path.is_empty() {
            None
        } else if self.shell_path.contains('=') {
            // `env` would take it for an assignment.
            Some(Shell::new(self.shell_path.clone(), self.shell_args.clone()))
        } else {
            let mut args = Vec::new();
            for key in UNSET_IN_CHILD {
                if !self.env_vars.contains_key(*key) {
                    args.push("-u".to_string());
                    args.push(key.to_string());
                }
            }
            args.push(self.shell_path.clone());
            args.extend(self.shell_args.iter().cloned());
            Some(Shell::new("/usr/bin/env".to_string(), args))
        };
        PtyOptions {
            shell,
            working_directory: self.working_directory.as_ref().map(PathBuf::from),
            drain_on_exit: false,
            env: self.env_vars.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shell_starts_without_inherited_overrides() {
        let mut config = TerminalConfig {
            shell_path: "/bin/zsh".into(),
            shell_args: vec!["-l".into()],
            ..TerminalConfig::default()
        };
        config.env_vars.insert("FORCE_COLOR".into(), "1".into());
        // FORCE_COLOR is set on purpose, so it stays.
        let args = [
            "-u",
            "NO_COLOR",
            "-u",
            "CLICOLOR",
            "-u",
            "CLICOLOR_FORCE",
            "-u",
            "ALACRITTY_WINDOW_ID",
            "-u",
            "WINDOWID",
            "/bin/zsh",
            "-l",
        ];
        assert_eq!(
            config.to_pty_options().shell,
            Some(Shell::new(
                "/usr/bin/env".into(),
                args.iter().map(|arg| arg.to_string()).collect()
            ))
        );
    }
}
