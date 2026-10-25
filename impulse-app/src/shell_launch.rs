//! How a terminal starts its shell, as `TerminalTab.spawnShell` does: the
//! user's login shell, with Impulse's shell integration (OSC 133 marks,
//! OSC 7 directory, OSC 6973 metadata) added the way each shell takes it:
//! bash through an `--rcfile` that reads the login files first, zsh through
//! a `ZDOTDIR` of startup files that read the user's, fish through
//! `--init-command`.

use std::collections::HashMap;
use std::path::PathBuf;

use impulse_kit::shell::{integration, login, ShellType};

/// The shell, its arguments, the variables to add, and temporary files to
/// remove when the terminal closes.
pub struct Launch {
    pub shell: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub temp_paths: Vec<PathBuf>,
}

/// A random number from the OS's randomness (std's hasher seeds).
fn random() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(std::process::id() as u64);
    hasher.finish()
}

/// This run's private folder for shell startup files: made fresh with a
/// random name (never one that's already there, which another user could
/// have planted in a shared `/tmp`), readable by the user only, in
/// `$XDG_RUNTIME_DIR` when there is one (the Swift app uses the per-user
/// temporary folder and a UUID).
pub(crate) fn private_dir() -> Option<&'static PathBuf> {
    static DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute() && p.is_dir())
            .unwrap_or_else(std::env::temp_dir);
        for _ in 0..16 {
            let dir = base.join(format!("impulse-{:016x}", random()));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
            match builder.create(&dir) {
                Ok(()) => return Some(dir),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => return None,
            }
        }
        None
    })
    .as_ref()
}

/// Remove this run's private folder (at quit).
pub fn remove_private_dir() {
    if let Some(dir) = private_dir() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// A new path in the private folder.
fn unique(prefix: &str) -> Option<PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let next = NEXT.fetch_add(1, Ordering::Relaxed);
    Some(private_dir()?.join(format!("{prefix}-{next}")))
}

/// Write `text` to a file that must not exist yet, readable by the user
/// only.
fn write_new(path: &std::path::Path, text: &str) -> bool {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
        .open(path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .is_ok()
}

/// A UTF-8 locale for programs when the app has none (apps started from a
/// desktop launcher often don't).
fn utf8_locale() -> &'static str {
    if cfg!(target_os = "macos") {
        "en_US.UTF-8"
    } else {
        "C.UTF-8"
    }
}

pub fn launch(extra_env: &[(String, String)]) -> Launch {
    let shell = login::default_shell_path();
    let mut env: HashMap<String, String> = [
        ("TERM", "xterm-256color"),
        ("TERM_PROGRAM", "Impulse"),
        ("COLORTERM", "truecolor"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    if ["LANG", "LC_ALL", "LC_CTYPE"]
        .iter()
        .all(|k| std::env::var_os(k).is_none())
    {
        env.insert("LANG".into(), utf8_locale().into());
    }
    for (k, v) in extra_env {
        env.insert(k.clone(), v.clone());
    }
    let mut args = Vec::new();
    let mut temp_paths = Vec::new();
    match ShellType::detect(&shell) {
        ShellType::Fish => {
            args.extend([
                "--login".to_string(),
                "--init-command".to_string(),
                integration::script(ShellType::Fish).to_string(),
            ]);
        }
        ShellType::Zsh => {
            let user = std::env::var("ZDOTDIR").ok().filter(|z| !z.is_empty());
            let files = integration::zsh_startup_files(
                integration::script(ShellType::Zsh),
                user.as_deref(),
            );
            let written = unique("zsh").filter(|zdotdir| {
                std::fs::create_dir(zdotdir).is_ok()
                    && files
                        .iter()
                        .all(|(name, text)| write_new(&zdotdir.join(name), text))
            });
            if let Some(zdotdir) = written {
                env.insert("ZDOTDIR".into(), zdotdir.to_string_lossy().into_owned());
                temp_paths.push(zdotdir);
            }
            args.push("--login".into());
        }
        ShellType::Bash => {
            let rc = unique("bash-rc");
            // `--rcfile` makes bash non-login, so read what a login shell
            // would, then the integration.
            let text = format!(
                r#"[ -f /etc/profile ] && source /etc/profile
if [ -f "$HOME/.bash_profile" ]; then source "$HOME/.bash_profile"
elif [ -f "$HOME/.bash_login" ]; then source "$HOME/.bash_login"
elif [ -f "$HOME/.profile" ]; then source "$HOME/.profile"
elif [ -f "$HOME/.bashrc" ]; then source "$HOME/.bashrc"
fi
{}
"#,
                integration::script(ShellType::Bash)
            );
            if let Some(rc) = rc.filter(|rc| write_new(rc, &text)) {
                args.extend(["--rcfile".to_string(), rc.to_string_lossy().into_owned()]);
                temp_paths.push(rc);
            } else {
                args.push("--login".into());
            }
        }
    }
    Launch {
        shell,
        args,
        env,
        temp_paths,
    }
}
