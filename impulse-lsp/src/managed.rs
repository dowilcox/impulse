//! Language servers Impulse installs itself (the Swift `ManagedServers`):
//! the web servers, installed with npm into Impulse's own folder, and the
//! system servers it only looks for on the login `PATH`. The Settings tab's
//! Language Servers page shows both and installs the managed ones.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use impulse_kit::shell::login::{is_executable_file, login_path, merge_paths, which};

/// The npm packages, each held to one major version (for 0.x, one minor)
/// so a reinstall never moves to one that works differently. TypeScript
/// stays on 6: 7 is the native compiler, without the tsserver that
/// typescript-language-server runs.
pub const PACKAGES: &[&str] = &[
    "typescript@^6.0.2",
    "typescript-language-server@^6.0.2",
    "intelephense@^1.18.5",
    "vscode-langservers-extracted@^4.10.0",
    "@tailwindcss/language-server@^0.16.0",
    "@vue/language-server@^3.3.13",
    "svelte-language-server@^0.18.4",
    "graphql-language-service-cli@^3.5.0",
    "emmet-ls@^0.4.2",
    "yaml-language-server@^2.0.0",
    "dockerfile-language-server-nodejs@^0.15.0",
    "bash-language-server@^5.8.1",
];

/// The commands those packages install.
pub const MANAGED_COMMANDS: &[&str] = &[
    "typescript-language-server",
    "intelephense",
    "vscode-html-language-server",
    "vscode-css-language-server",
    "vscode-json-language-server",
    "vscode-eslint-language-server",
    "tailwindcss-language-server",
    "vue-language-server",
    "svelteserver",
    "graphql-lsp",
    "emmet-ls",
    "yaml-language-server",
    "docker-langserver",
    "bash-language-server",
];

/// Servers installed with the system's own tools: (id, command).
pub const SYSTEM_SERVERS: &[(&str, &str)] = &[
    ("rust-analyzer", "rust-analyzer"),
    ("pyright", "pyright-langserver"),
    ("clangd", "clangd"),
    // Ships with Xcode and the Command Line Tools (Swift files use it).
    ("sourcekit-lsp", "sourcekit-lsp"),
];

/// One server's install state, as the Settings tab lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerStatus {
    pub command: String,
    pub resolved_path: Option<PathBuf>,
}

impl ServerStatus {
    pub fn installed(&self) -> bool {
        self.resolved_path.is_some()
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Where the managed servers live: `$XDG_DATA_HOME/impulse/lsp` (or
/// `~/.local/share/impulse/lsp`), or on the Mac `~/Library/Application
/// Support/impulse/lsp`; whichever holds an installation, else the
/// platform's own.
pub fn managed_root_dir() -> PathBuf {
    let xdg_base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"));
    let xdg = xdg_base.join("impulse/lsp");
    let native = if cfg!(target_os = "macos") {
        home().join("Library/Application Support/impulse/lsp")
    } else {
        xdg.clone()
    };
    if xdg.join("node_modules").is_dir() {
        xdg
    } else {
        native
    }
}

pub fn managed_bin_dir() -> PathBuf {
    managed_root_dir().join("node_modules/.bin")
}

/// `command` on the login `PATH`, then in the managed bin folder.
pub fn resolve(command: &str) -> Option<PathBuf> {
    which(command).map(PathBuf::from).or_else(|| {
        let candidate = managed_bin_dir().join(command);
        is_executable_file(&candidate).then_some(candidate)
    })
}

/// The managed servers and where each is installed.
pub fn managed_status() -> Vec<ServerStatus> {
    MANAGED_COMMANDS
        .iter()
        .map(|command| ServerStatus {
            command: command.to_string(),
            resolved_path: resolve(command),
        })
        .collect()
}

/// The system servers (by id), found on the login `PATH` only.
pub fn system_status() -> Vec<ServerStatus> {
    SYSTEM_SERVERS
        .iter()
        .map(|(id, command)| ServerStatus {
            command: id.to_string(),
            resolved_path: which(command).map(PathBuf::from),
        })
        .collect()
}

/// The login shell's `PATH` with the managed bin folder first, so `node`,
/// `npm` and servers installed with Homebrew, cargo or nvm are found.
pub fn tool_path() -> String {
    let bin = managed_bin_dir().to_string_lossy().into_owned();
    merge_paths(&[&bin, &login_path()])
}

/// `npm --version` succeeds.
pub fn npm_is_available() -> bool {
    Command::new("npm")
        .arg("--version")
        .env("PATH", tool_path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// npm-install `PACKAGES` into the managed folder (blocking). The bin
/// folder on success, else why not.
pub fn install() -> Result<PathBuf, String> {
    if !npm_is_available() {
        return Err(
            "npm is required but was not found in PATH. Install Node.js + npm first.".into(),
        );
    }
    let root = managed_root_dir();
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("Failed to create {}: {e}", root.display()))?;
    write_package_json(&root)?;
    // No install scripts: none of these servers needs one, and skipping them
    // means installing runs no package's code.
    let status = Command::new("npm")
        .args(["install", "--prefix"])
        .arg(&root)
        .args(["--no-audit", "--no-fund", "--ignore-scripts"])
        .args(PACKAGES)
        .env("PATH", tool_path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Failed to run npm install: {e}"))?;
    if !status.success() {
        return Err(format!(
            "npm install failed with status {status} while installing managed LSP servers"
        ));
    }
    Ok(managed_bin_dir())
}

fn write_package_json(root: &Path) -> Result<(), String> {
    let path = root.join("package.json");
    if path.exists() {
        return Ok(());
    }
    let document = serde_json::json!({
        "description": "Managed web LSP dependencies for Impulse",
        "license": "UNLICENSED",
        "name": "impulse-lsp-servers",
        "private": true,
    });
    let text = serde_json::to_string_pretty(&document).unwrap_or_default();
    std::fs::write(&path, text).map_err(|e| format!("Failed to write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_every_server() {
        assert_eq!(managed_status().len(), MANAGED_COMMANDS.len());
        let system = system_status();
        assert_eq!(system.len(), SYSTEM_SERVERS.len());
        assert_eq!(system[0].command, "rust-analyzer");
    }

    #[test]
    fn writes_package_json_once() {
        let dir = tempfile::tempdir().unwrap();
        write_package_json(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join("package.json")).unwrap();
        assert!(text.contains("impulse-lsp-servers"));
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        write_package_json(dir.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("package.json")).unwrap(),
            "{}"
        );
    }
}
