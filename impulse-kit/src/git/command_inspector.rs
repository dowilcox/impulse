//! What a shell command line does with git, for agent hooks: which branches
//! it merges, rebases onto or pulls (`git merge interia-upgrade`,
//! `git -C ../x rebase origin/main && npm test`), and whether it moves
//! HEAD. Read with the shell parser, so quoting and `&&` chains don't fool
//! it, and neither do scripts: several lines, subshells and groups,
//! `if`/`while` bodies, and wrappers such as `env`, `sudo` and `time`.

use crate::shell_parser::{parse_shell_input, ShellTokenRole};

/// One git invocation in a command line: its subcommand and arguments
/// (global options like `-C dir` removed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    pub subcommand: String,
    pub arguments: Vec<String>,
}

/// The git invocations in `command_line`, in order.
pub fn invocations(command_line: &str) -> Vec<Invocation> {
    let script = separating_commands(command_line);
    let parsed = parse_shell_input(&script, script.len());
    let mut result = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut finish = |words: &mut Vec<String>| {
        let all = std::mem::take(words);
        let command = &all[leading_words(&all)..];
        let Some(first) = command.first() else { return };
        if first != "git" && !first.ends_with("/git") {
            return;
        }
        let mut rest = &command[1..];
        // Global options before the subcommand.
        while let Some(option) = rest.first().filter(|o| o.starts_with('-')) {
            rest = &rest[1..];
            let takes_value = [
                "-C",
                "-c",
                "--git-dir",
                "--work-tree",
                "--namespace",
                "--config-env",
            ]
            .contains(&option.as_str());
            if takes_value && !rest.is_empty() {
                rest = &rest[1..];
            }
        }
        let Some(subcommand) = rest.first() else {
            return;
        };
        result.push(Invocation {
            subcommand: subcommand.clone(),
            arguments: rest[1..].to_vec(),
        });
    };
    for token in parsed.tokens {
        match token.role {
            ShellTokenRole::Command | ShellTokenRole::Argument => words.push(token.text),
            ShellTokenRole::ControlOperator | ShellTokenRole::PipelineSeparator => {
                finish(&mut words)
            }
            _ => {}
        }
    }
    finish(&mut words);
    result
}

/// `command_line` with each unquoted newline, parenthesis and backtick made
/// a `;`, so every command in a script, subshell or command substitution
/// ends up a command of its own for the parser, and line continuations
/// (backslash, newline) made spaces.
fn separating_commands(command_line: &str) -> String {
    let mut out: Vec<char> = Vec::with_capacity(command_line.len());
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in command_line.chars() {
        if escaped {
            escaped = false;
            if c == '\n' {
                if let Some(last) = out.last_mut() {
                    *last = ' ';
                }
                out.push(' ');
            } else {
                out.push(c);
            }
            continue;
        }
        if let Some(open) = quote {
            if c == open {
                quote = None;
            } else if open == '"' && c == '\\' {
                escaped = true;
            }
        } else if c == '\\' {
            escaped = true;
        } else if c == '\'' || c == '"' {
            quote = Some(c);
        } else if matches!(c, '\n' | '(' | ')' | '`') {
            out.push(';');
            continue;
        }
        out.push(c);
    }
    out.into_iter().collect()
}

/// Words that start a command without being one.
const KEYWORDS: [&str; 12] = [
    "!", "{", "}", "if", "then", "else", "elif", "fi", "do", "done", "while", "until",
];

/// Commands that run the rest of the line as a command, with those of their
/// options that take a value.
fn wrapper_options(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "env" => &["-u", "-C", "-P", "-S", "--unset", "--chdir"],
        "command" | "time" | "nohup" => &[],
        "sudo" => &[
            "-u", "-g", "-C", "-D", "-h", "-p", "-r", "-t", "-U", "-T", "--user", "--group",
            "--chdir", "--prompt",
        ],
        "nice" => &["-n", "--adjustment"],
        "exec" => &["-a"],
        _ => return None,
    })
}

/// How many of a command's words come before the program it runs: shell
/// keywords (`if`, `then`, `do`, `!`, `{` ...), assignments, and wrappers
/// that run the rest as a command (`env A=b`, `command`, `time -p`,
/// `sudo -u x`, `nice -n 5`, `nohup`) with their options.
fn leading_words(words: &[String]) -> usize {
    let mut index = 0;
    while index < words.len() {
        let word = &words[index];
        let name = word.split('/').rfind(|p| !p.is_empty()).unwrap_or(word);
        if KEYWORDS.contains(&word.as_str()) || is_assignment(word) {
            index += 1;
        } else if let Some(valued) = wrapper_options(name) {
            index += 1;
            while index < words.len() && words[index].starts_with('-') {
                let option = words[index].as_str();
                index += if valued.contains(&option) { 2 } else { 1 };
                if option == "--" {
                    break;
                }
            }
        } else {
            break;
        }
    }
    index.min(words.len())
}

/// `NAME=value`, the way a shell recognizes an assignment.
fn is_assignment(word: &str) -> bool {
    let Some(equals) = word.find('=') else {
        return false;
    };
    let Some(first) = word.chars().next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && word[..equals]
            .chars()
            .all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// The refs a command line merges into the current branch: `merge`'s
/// commits, `rebase`'s upstream, `pull`'s branches (as `remote/branch`, or
/// the branch itself when pulling from `.`).
pub fn merged_refs(command_line: &str) -> Vec<String> {
    let mut refs = Vec::new();
    for invocation in invocations(command_line) {
        let positional = positionals(
            &invocation.arguments,
            valued_options(&invocation.subcommand),
        );
        match invocation.subcommand.as_str() {
            "merge" => refs.extend(positional),
            "rebase" => {
                if let Some(onto) = value_of("--onto", &invocation.arguments) {
                    refs.push(onto);
                }
                if let Some(upstream) = positional.into_iter().next() {
                    refs.push(upstream);
                }
            }
            "pull" => {
                let Some((remote, branches)) = positional.split_first() else {
                    continue;
                };
                for branch in branches {
                    refs.push(if remote == "." {
                        branch.clone()
                    } else {
                        format!("{remote}/{branch}")
                    });
                }
            }
            _ => {}
        }
    }
    refs
}

/// Whether the command line can move HEAD to another commit (so dependency
/// files may have changed under the checkout).
pub fn moves_head(command_line: &str) -> bool {
    invocations(command_line).iter().any(|i| {
        [
            "merge",
            "pull",
            "rebase",
            "checkout",
            "switch",
            "reset",
            "cherry-pick",
            "revert",
        ]
        .contains(&i.subcommand.as_str())
    })
}

/// Options that take a value, per subcommand, so the value isn't read as a
/// branch.
fn valued_options(subcommand: &str) -> &'static [&'static str] {
    match subcommand {
        "merge" => &[
            "-m",
            "-F",
            "-s",
            "-X",
            "--strategy",
            "--strategy-option",
            "--file",
            "--into-name",
            "--cleanup",
        ],
        "rebase" => &[
            "--onto",
            "-s",
            "-X",
            "--strategy",
            "--strategy-option",
            "-x",
            "--exec",
        ],
        "pull" => &[
            "-s",
            "-X",
            "--strategy",
            "--strategy-option",
            "--depth",
            "--deepen",
            "--shallow-since",
            "-j",
            "--jobs",
        ],
        _ => &[],
    }
}

fn positionals(arguments: &[String], valued: &[&str]) -> Vec<String> {
    let mut result = Vec::new();
    let mut skip_next = false;
    let mut options_ended = false;
    for argument in arguments {
        if skip_next {
            skip_next = false;
            continue;
        }
        if !options_ended && argument == "--" {
            options_ended = true;
            continue;
        }
        if !options_ended && argument.starts_with('-') {
            if valued.contains(&argument.as_str()) {
                skip_next = true;
            }
            continue;
        }
        result.push(argument.clone());
    }
    result
}

fn value_of(option: &str, arguments: &[String]) -> Option<String> {
    for (index, argument) in arguments.iter().enumerate() {
        if argument == option && index + 1 < arguments.len() {
            return Some(arguments[index + 1].clone());
        }
        if let Some(value) = argument.strip_prefix(&format!("{option}=")) {
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(command: &str) -> Vec<String> {
        merged_refs(command)
    }

    #[test]
    fn merges_rebases_and_pulls() {
        assert_eq!(refs("git merge interia-upgrade"), ["interia-upgrade"]);
        assert_eq!(
            refs(r#"git merge --no-ff -m "Merge it" interia-upgrade"#),
            ["interia-upgrade"]
        );
        assert_eq!(
            refs("git -C ../pulse merge origin/interia-upgrade && npm test"),
            ["origin/interia-upgrade"]
        );
        assert_eq!(refs("git rebase main"), ["main"]);
        assert_eq!(
            refs("git rebase --onto upgrade main topic"),
            ["upgrade", "main"]
        );
        assert_eq!(refs("git pull origin feature-x"), ["origin/feature-x"]);
        assert_eq!(refs("git pull . feature-x"), ["feature-x"]);
        assert!(refs("git pull").is_empty());
        assert!(
            refs("echo 'git merge x'; git status").is_empty(),
            "quoted text isn't a command"
        );
        assert!(refs("git log --oneline main..upgrade").is_empty());
    }

    #[test]
    fn head_moves() {
        assert!(moves_head("git pull --rebase"));
        assert!(moves_head("cd x && git switch main"));
        assert!(!moves_head("git status && git diff"));
        assert!(!moves_head("npm install"));
    }

    #[test]
    fn scripts_are_read_command_by_command() {
        assert_eq!(refs("npm test\ngit merge feature"), ["feature"]);
        assert_eq!(
            refs("git fetch\n  git merge feature\nnpm test"),
            ["feature"]
        );
        assert_eq!(refs("(cd ../app && git merge feature)"), ["feature"]);
        assert_eq!(refs("{ git merge feature; npm test; }"), ["feature"]);
        assert_eq!(refs("! git merge feature"), ["feature"]);
        assert_eq!(refs("if git merge feature; then npm test; fi"), ["feature"]);
        assert_eq!(
            refs("for b in a; do git pull origin feature; done"),
            ["origin/feature"]
        );
        assert_eq!(refs("echo $(git merge feature)"), ["feature"]);
        assert_eq!(refs("echo `git rebase main`"), ["main"]);
        assert_eq!(
            refs("git merge \\\n  feature"),
            ["feature"],
            "a continued line is one command"
        );
        assert_eq!(
            refs("echo 'é' && git merge feature"),
            ["feature"],
            "non-ASCII text before it"
        );
        // Quoted, it's text.
        assert!(refs("git commit -m \"fix (merge it)\ngit merge x\"").is_empty());
        assert!(refs("echo 'a\ngit merge x'").is_empty());
        assert!(!moves_head(
            "git commit -m \"subject\n\ngit checkout body\""
        ));
    }

    #[test]
    fn wrappers_are_seen_through() {
        assert_eq!(refs("env GIT_EDITOR=true git merge feature"), ["feature"]);
        assert_eq!(
            refs("env -i -u HOME A=b /usr/bin/git merge feature"),
            ["feature"]
        );
        assert_eq!(refs("GIT_EDITOR=true git merge feature"), ["feature"]);
        assert_eq!(refs("command git merge feature"), ["feature"]);
        assert_eq!(refs("time -p git merge feature"), ["feature"]);
        assert_eq!(refs("sudo -u sam git merge feature"), ["feature"]);
        assert_eq!(refs("nice -n 10 git merge feature"), ["feature"]);
        assert_eq!(refs("nohup git pull origin feature"), ["origin/feature"]);
        assert_eq!(
            refs("if FOO=1 nohup env git rebase main; then :; fi"),
            ["main"]
        );
        assert!(refs("command -v git").is_empty());
        assert!(refs("env git").is_empty());
    }

    #[test]
    fn global_options_with_values() {
        assert_eq!(
            refs("git --config-env core.editor=EDITOR merge feature"),
            ["feature"]
        );
        assert_eq!(
            refs("git --config-env=core.editor=EDITOR merge feature"),
            ["feature"]
        );
        assert_eq!(
            refs("git -c merge.ff=false -C ../app merge feature"),
            ["feature"]
        );
    }
}
