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

fn unique(prefix: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
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
            let zdotdir = unique("impulse-zsh");
            let user = std::env::var("ZDOTDIR").ok().filter(|z| !z.is_empty());
            let files = integration::zsh_startup_files(
                integration::script(ShellType::Zsh),
                user.as_deref(),
            );
            let written = std::fs::create_dir_all(&zdotdir).is_ok()
                && files
                    .iter()
                    .all(|(name, text)| std::fs::write(zdotdir.join(name), text).is_ok());
            if written {
                env.insert("ZDOTDIR".into(), zdotdir.to_string_lossy().into_owned());
                temp_paths.push(zdotdir);
            }
            args.push("--login".into());
        }
        ShellType::Bash => {
            let rc = unique("impulse-bash-rc");
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
            if std::fs::write(&rc, text).is_ok() {
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
