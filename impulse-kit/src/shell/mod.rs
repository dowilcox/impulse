//! The shells Impulse runs in its terminals: which kind a shell is, how to
//! quote text typed into it (`quoting`), the user's login shell and its PATH
//! (`login`), the integration scripts that report prompts, commands and the
//! working directory (`integration`), and whether a command word names
//! something the shell can run (`lookup`).

use serde::{Deserialize, Serialize};

pub mod integration;
pub mod login;
pub mod lookup;
pub mod quoting;

/// How a shell quotes, and which integration script it gets. Every shell
/// that isn't zsh or fish is treated as bash.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ShellType {
    #[default]
    Bash,
    Zsh,
    Fish,
}

impl ShellType {
    /// The shell type for a shell path or name (`/bin/zsh`, `fish`).
    pub fn detect(shell_path: &str) -> ShellType {
        match shell_path.rsplit('/').next().unwrap_or(shell_path) {
            "zsh" => ShellType::Zsh,
            "fish" => ShellType::Fish,
            _ => ShellType::Bash,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ShellType::Bash => "bash",
            ShellType::Zsh => "zsh",
            ShellType::Fish => "fish",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_shell_from_its_path() {
        assert_eq!(ShellType::detect("/bin/zsh"), ShellType::Zsh);
        assert_eq!(ShellType::detect("/opt/homebrew/bin/fish"), ShellType::Fish);
        assert_eq!(ShellType::detect("fish"), ShellType::Fish);
        assert_eq!(ShellType::detect("/bin/bash"), ShellType::Bash);
        assert_eq!(ShellType::detect("/bin/dash"), ShellType::Bash);
        assert_eq!(ShellType::detect(""), ShellType::Bash);
    }
}
