use portable_pty::CommandBuilder;
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

const BASH_INTEGRATION: &str = include_str!("shell_integration/bash.sh");
const ZSH_INTEGRATION: &str = include_str!("shell_integration/zsh.sh");
const FISH_INTEGRATION: &str = include_str!("shell_integration/fish.sh");

#[derive(Debug, Clone, PartialEq)]
pub enum ShellType {
    Bash,
    Zsh,
    Fish,
}

/// Get the user's login shell from /etc/passwd.
pub fn get_user_login_shell() -> Option<String> {
    let username = std::env::var("USER").ok()?;
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    for line in passwd.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() >= 7 && fields[0] == username {
            let shell = fields[6].to_string();
            if std::path::Path::new(&shell).exists() {
                return Some(shell);
            }
        }
    }
    None
}

/// Detect shell type from the shell path.
pub fn detect_shell_type(shell_path: &str) -> ShellType {
    let shell_name = std::path::Path::new(shell_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("bash");

    match shell_name {
        "zsh" => ShellType::Zsh,
        "fish" => ShellType::Fish,
        _ => ShellType::Bash,
    }
}

/// Get the default shell path, preferring /etc/passwd over $SHELL.
pub fn get_default_shell_path() -> String {
    get_user_login_shell()
        .or_else(|| std::env::var("SHELL").ok())
        .unwrap_or_else(|| "/bin/bash".to_string())
}

/// Return the name of the user's default shell (e.g. "fish", "zsh", "bash").
pub fn get_default_shell_name() -> String {
    let shell_path = get_default_shell_path();
    std::path::Path::new(&shell_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("shell")
        .to_string()
}

/// Return the user's home directory from $HOME.
pub fn get_home_directory() -> Result<String, String> {
    std::env::var("HOME").map_err(|e| format!("Failed to get HOME: {}", e))
}

/// Write a file with owner-only permissions (0600) to prevent other users
/// from reading shell integration scripts that may reveal path information.
#[cfg(unix)]
fn write_secure_file(path: &std::path::Path, content: &str) -> Result<(), std::io::Error> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(content.as_bytes())
}

/// Create a directory with owner-only permissions (0700).
#[cfg(unix)]
fn create_secure_dir(path: &std::path::Path) -> Result<(), std::io::Error> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

/// Build the shell command with integration scripts injected.
/// Returns the CommandBuilder and a list of temp files that must be kept alive.
pub fn build_shell_command(
    shell_path: &str,
    shell_type: &ShellType,
) -> Result<(CommandBuilder, Vec<PathBuf>), std::io::Error> {
    let mut temp_files = Vec::new();

    let mut cmd = match shell_type {
        ShellType::Bash => {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            let user_bashrc = format!("{}/.bashrc", home);

            let rc_content = format!(
                "# Source user's bashrc\n\
                 if [ -f \"{}\" ]; then\n\
                     source \"{}\"\n\
                 fi\n\
                 # Impulse shell integration\n\
                 {}\n",
                user_bashrc, user_bashrc, BASH_INTEGRATION
            );

            let rc_path = std::env::temp_dir().join(format!(
                "impulse-bash-rc-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4().as_simple()
            ));
            write_secure_file(&rc_path, &rc_content)?;
            temp_files.push(rc_path.clone());

            let mut cmd = CommandBuilder::new(shell_path);
            cmd.arg("--rcfile");
            cmd.arg(rc_path.to_string_lossy().as_ref());
            cmd
        }
        ShellType::Zsh => {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());

            let zdotdir = std::env::temp_dir().join(format!(
                "impulse-zsh-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4().as_simple()
            ));
            create_secure_dir(&zdotdir)?;

            let zshenv_content = format!(
                "if [ -f \"{}/.zshenv\" ]; then\n\
                     source \"{}/.zshenv\"\n\
                 fi\n",
                home, home
            );
            let zshenv_path = zdotdir.join(".zshenv");
            write_secure_file(&zshenv_path, &zshenv_content)?;
            temp_files.push(zshenv_path);

            let zprofile_content = format!(
                "if [ -f \"{}/.zprofile\" ]; then\n\
                     source \"{}/.zprofile\"\n\
                 fi\n",
                home, home
            );
            let zprofile_path = zdotdir.join(".zprofile");
            write_secure_file(&zprofile_path, &zprofile_content)?;
            temp_files.push(zprofile_path);

            let zlogin_content = format!(
                "if [ -f \"{}/.zlogin\" ]; then\n\
                     source \"{}/.zlogin\"\n\
                 fi\n",
                home, home
            );
            let zlogin_path = zdotdir.join(".zlogin");
            write_secure_file(&zlogin_path, &zlogin_content)?;
            temp_files.push(zlogin_path);

            let rc_content = format!(
                "# Restore original ZDOTDIR\n\
                 export ZDOTDIR=\"{}\"\n\
                 # Source user's zshrc\n\
                 if [ -f \"{}/.zshrc\" ]; then\n\
                     source \"{}/.zshrc\"\n\
                 fi\n\
                 # Impulse shell integration\n\
                 {}\n",
                home, home, home, ZSH_INTEGRATION
            );

            let zshrc_path = zdotdir.join(".zshrc");
            write_secure_file(&zshrc_path, &rc_content)?;
            temp_files.push(zshrc_path);

            let mut cmd = CommandBuilder::new(shell_path);
            cmd.arg("--login");
            cmd.env("ZDOTDIR", zdotdir.to_string_lossy().as_ref());
            cmd
        }
        ShellType::Fish => {
            let mut cmd = CommandBuilder::new(shell_path);
            cmd.arg("--login");
            cmd.arg("--init-command");
            cmd.arg(FISH_INTEGRATION);
            cmd
        }
    };

    cmd.env("TERM_PROGRAM", "Impulse");
    cmd.env("TERM_PROGRAM_VERSION", "0.1.0");
    cmd.env("TERM", "xterm-256color");

    Ok((cmd, temp_files))
}

/// Get the shell integration script content for a given shell type.
/// Useful when the caller (e.g. VTE, SwiftTerm) manages its own PTY
/// and just needs the script text to inject via env vars.
pub fn get_integration_script(shell_type: &ShellType) -> &'static str {
    match shell_type {
        ShellType::Bash => BASH_INTEGRATION,
        ShellType::Zsh => ZSH_INTEGRATION,
        ShellType::Fish => FISH_INTEGRATION,
    }
}

/// Prepare environment variables and arguments for spawning a shell with integration.
/// This is the high-level API for frontends that manage their own PTY (VTE, SwiftTerm).
/// Returns (shell_path, args, env_vars, temp_files).
pub fn prepare_shell_spawn() -> Result<ShellSpawnConfig, std::io::Error> {
    let shell_path = get_default_shell_path();
    let shell_type = detect_shell_type(&shell_path);
    let (cmd, temp_files) = build_shell_command(&shell_path, &shell_type)?;

    Ok(ShellSpawnConfig {
        shell_path,
        shell_type,
        command: cmd,
        temp_files,
    })
}

/// Configuration for spawning a shell with integration.
pub struct ShellSpawnConfig {
    pub shell_path: String,
    pub shell_type: ShellType,
    pub command: CommandBuilder,
    pub temp_files: Vec<PathBuf>,
}
