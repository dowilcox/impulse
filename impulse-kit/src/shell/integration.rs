//! Shell integration (the Swift `ShellIntegration`): the scripts each
//! terminal's shell loads, which emit OSC 133 (prompt and command marks),
//! OSC 7 (the working directory) and OSC 6973 (the command text, the
//! shell's own names and its PATH). The scripts are the Swift app's, read
//! from its resources at build time, so both apps load the same ones.
//!
//! zsh has no option to load an extra file, so a terminal starts it with
//! `ZDOTDIR` pointing at a folder of Impulse's startup files, which read the
//! user's own (`zsh_startup_files`).

use std::collections::BTreeMap;

use super::ShellType;

const BASH: &str = include_str!("../../resources/shell-integration/bash.sh");
const ZSH: &str = include_str!("../../resources/shell-integration/zsh.sh");
const FISH: &str = include_str!("../../resources/shell-integration/fish.sh");

/// The integration script for a shell type.
pub fn script(shell: ShellType) -> &'static str {
    match shell {
        ShellType::Bash => BASH,
        ShellType::Zsh => ZSH,
        ShellType::Fish => FISH,
    }
}

/// The integration script for a shell name or path (`zsh`, `/bin/bash`).
pub fn script_for_shell(shell: &str) -> &'static str {
    script(ShellType::detect(shell))
}

/// Startup files (file name to contents) for a zsh started with ZDOTDIR
/// pointing at a folder holding them. Each reads the user's own file from
/// where zsh would have, their ZDOTDIR (`user_zdotdir`, from before Impulse
/// set it) or home, with the user's ZDOTDIR in place while it runs, and
/// `integration` loads after the user's .zshrc.
///
/// A .zshenv (or .zprofile) that sets ZDOTDIR moves the rest of the user's
/// files: Impulse notes the new folder and puts its own ZDOTDIR back so zsh
/// keeps reading these files. The .zshrc gives ZDOTDIR back to the user for
/// good, so .zlogin comes straight from their folder and programs started in
/// the shell see their value.
pub fn zsh_startup_files(
    integration: &str,
    user_zdotdir: Option<&str>,
) -> BTreeMap<&'static str, String> {
    // Files run at the top level, never inside a function: a `typeset` in
    // them would otherwise be local to it.
    let use_users = "if (( ${+__impulse_user_zdotdir} )); then export ZDOTDIR=$__impulse_user_zdotdir; else unset ZDOTDIR; fi\n";
    let back_to_impulse = "if (( ${+ZDOTDIR} )); then __impulse_user_zdotdir=$ZDOTDIR; else unset __impulse_user_zdotdir; fi\n\
                           export ZDOTDIR=$__impulse_zdotdir\n";
    let source_users = |name: &str| {
        format!(
            "if [[ -f \"${{ZDOTDIR:-$HOME}}/{name}\" ]]; then\n    source \"${{ZDOTDIR:-$HOME}}/{name}\"\nfi\n"
        )
    };
    let initial = match user_zdotdir {
        Some(dir) => format!("__impulse_user_zdotdir={}\n", single_quoted(dir)),
        None => "unset __impulse_user_zdotdir\n".to_string(),
    };
    // macOS's /etc/zshrc sets HISTFILE=${ZDOTDIR:-$HOME}/.zsh_history while
    // ZDOTDIR is still Impulse's folder, which is deleted with the terminal:
    // history goes where it would have without Impulse.
    let user_history = "if [[ -n $__impulse_zdotdir && $HISTFILE == \"$__impulse_zdotdir\"/* ]]; then\n    HISTFILE=${ZDOTDIR:-$HOME}/.zsh_history\nfi\n";
    BTreeMap::from([
        (
            ".zshenv",
            format!(
                "__impulse_zdotdir=$ZDOTDIR\n{initial}{use_users}{}{back_to_impulse}",
                source_users(".zshenv")
            ),
        ),
        (
            ".zprofile",
            format!("{use_users}{}{back_to_impulse}", source_users(".zprofile")),
        ),
        (
            ".zshrc",
            format!(
                "{use_users}{user_history}unset __impulse_zdotdir __impulse_user_zdotdir\n{}{integration}",
                source_users(".zshrc")
            ),
        ),
    ])
}

fn single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::child_process::ChildProcess;
    use std::path::Path;
    use std::time::Duration;

    #[test]
    fn scripts_emit_the_marks() {
        for shell in [ShellType::Bash, ShellType::Zsh, ShellType::Fish] {
            let script = script(shell);
            assert!(script.contains("133"), "{shell:?}");
            assert!(script.contains("6973"), "{shell:?}");
        }
        assert_eq!(script_for_shell("/usr/bin/fish"), FISH);
        assert_eq!(script_for_shell("sh"), BASH);
    }

    fn zsh() -> Option<&'static str> {
        ["/bin/zsh", "/usr/bin/zsh"]
            .into_iter()
            .find(|p| Path::new(p).exists())
    }

    /// A scratch home with the given files, Impulse's startup files in their
    /// own folder, and the lines a login shell printed while starting.
    /// `system_zshrc` runs just before Impulse's .zshrc, where macOS's
    /// /etc/zshrc runs (the test can't rely on that file's contents).
    fn start(
        zsh: &str,
        files: &[(&str, String)],
        inherited_zdotdir: Option<&dyn Fn(&str) -> String>,
        system_zshrc: &str,
        command: &str,
    ) -> (String, Vec<String>) {
        let root = tempfile::tempdir().unwrap();
        let root_path = std::fs::canonicalize(root.path()).unwrap();
        let home = root_path.join("home").to_str().unwrap().to_string();
        let impulse = root_path.join("impulse");
        for (path, contents) in files {
            let file = Path::new(&home).join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, contents).unwrap();
        }
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&impulse).unwrap();
        let user_zdotdir = inherited_zdotdir.map(|f| f(&home));
        let startup = zsh_startup_files(
            "print -r -- \"integration ZDOTDIR=${ZDOTDIR-unset}\"\n",
            user_zdotdir.as_deref(),
        );
        for (name, contents) in startup {
            let prefix = if name == ".zshrc" { system_zshrc } else { "" };
            std::fs::write(impulse.join(name), format!("{prefix}{contents}")).unwrap();
        }
        let output = ChildProcess::new(zsh)
            .args(["-i", "-l", "-c", command])
            .env_clear()
            .env("HOME", &home)
            .env("ZDOTDIR", &impulse)
            .env("PATH", "/usr/bin:/bin")
            .env("TERM", "dumb")
            .timeout(Duration::from_secs(20))
            .run()
            .unwrap();
        let lines = output
            .stdout_text()
            .split('\n')
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect();
        (home, lines)
    }

    const END: &str = "print -r -- \"end ZDOTDIR=${ZDOTDIR-unset}\"";

    fn announce(name: &str) -> String {
        format!("print -r -- \"{name} ZDOTDIR=${{ZDOTDIR-unset}}\"\n")
    }

    #[test]
    fn reads_the_usual_files_from_home() {
        let Some(zsh) = zsh() else { return };
        let (_, lines) = start(
            zsh,
            &[
                (".zshenv", announce("zshenv")),
                (".zprofile", announce("zprofile")),
                (".zshrc", announce("zshrc")),
                (".zlogin", announce("zlogin")),
            ],
            None,
            "",
            END,
        );
        assert_eq!(
            lines,
            [
                "zshenv ZDOTDIR=unset",
                "zprofile ZDOTDIR=unset",
                "zshrc ZDOTDIR=unset",
                "integration ZDOTDIR=unset",
                "zlogin ZDOTDIR=unset",
                "end ZDOTDIR=unset",
            ]
        );
    }

    #[test]
    fn follows_a_zdotdir_set_in_the_users_zshenv() {
        let Some(zsh) = zsh() else { return };
        let (home, lines) = start(
            zsh,
            &[
                (
                    ".zshenv",
                    announce("zshenv") + "export ZDOTDIR=$HOME/.config/zsh\n",
                ),
                (".zshrc", announce("home-zshrc")),
                (".config/zsh/.zprofile", announce("zprofile")),
                (
                    ".config/zsh/.zshrc",
                    announce("zshrc") + "typeset -g FROM_ZSHRC=yes\n",
                ),
                (
                    ".config/zsh/.zlogin",
                    announce("zlogin") + "print -r -- \"zshrc variable $FROM_ZSHRC\"\n",
                ),
            ],
            None,
            "",
            END,
        );
        let dots = format!("{home}/.config/zsh");
        assert_eq!(
            lines,
            [
                "zshenv ZDOTDIR=unset".to_string(),
                format!("zprofile ZDOTDIR={dots}"),
                format!("zshrc ZDOTDIR={dots}"),
                format!("integration ZDOTDIR={dots}"),
                format!("zlogin ZDOTDIR={dots}"),
                "zshrc variable yes".to_string(),
                format!("end ZDOTDIR={dots}"),
            ]
        );
    }

    #[test]
    fn uses_a_zdotdir_impulse_inherited() {
        let Some(zsh) = zsh() else { return };
        let (home, lines) = start(
            zsh,
            &[
                (".zshrc", announce("home-zshrc")),
                ("dots/.zshenv", announce("zshenv")),
                ("dots/.zshrc", announce("zshrc")),
            ],
            Some(&|home: &str| format!("{home}/dots")),
            "",
            END,
        );
        let dots = format!("{home}/dots");
        assert_eq!(
            lines,
            [
                format!("zshenv ZDOTDIR={dots}"),
                format!("zshrc ZDOTDIR={dots}"),
                format!("integration ZDOTDIR={dots}"),
                format!("end ZDOTDIR={dots}"),
            ]
        );
    }

    #[test]
    fn history_is_kept_where_zsh_would_keep_it() {
        let Some(zsh) = zsh() else { return };
        // What /etc/zshrc does, while ZDOTDIR is still Impulse's folder.
        let system_zshrc = "HISTFILE=${ZDOTDIR:-$HOME}/.zsh_history\n";
        let print_histfile = "print -r -- \"HISTFILE=$HISTFILE\"";
        let (home, lines) = start(zsh, &[], None, system_zshrc, print_histfile);
        assert_eq!(
            lines.last().map(String::as_str),
            Some(format!("HISTFILE={home}/.zsh_history").as_str())
        );

        let (dots_home, dots_lines) = start(
            zsh,
            &[(".zshenv", "export ZDOTDIR=$HOME/.config/zsh\n".to_string())],
            None,
            system_zshrc,
            print_histfile,
        );
        assert_eq!(
            dots_lines.last().map(String::as_str),
            Some(format!("HISTFILE={dots_home}/.config/zsh/.zsh_history").as_str())
        );

        // A HISTFILE the user's .zshrc sets is theirs.
        let (own_home, own_lines) = start(
            zsh,
            &[(".zshrc", "HISTFILE=$HOME/.history/zsh\n".to_string())],
            None,
            system_zshrc,
            print_histfile,
        );
        assert_eq!(
            own_lines.last().map(String::as_str),
            Some(format!("HISTFILE={own_home}/.history/zsh").as_str())
        );
    }

    #[test]
    fn quotes_the_inherited_folder() {
        let files = zsh_startup_files("", Some("/tmp/it's here"));
        assert!(files[".zshenv"].contains("__impulse_user_zdotdir='/tmp/it'\\''s here'"));
        assert_eq!(
            files.keys().copied().collect::<Vec<_>>(),
            [".zprofile", ".zshenv", ".zshrc"]
        );
    }
}
