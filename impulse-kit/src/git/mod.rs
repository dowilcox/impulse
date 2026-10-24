//! The git rules that need no repository (the Swift `ImpulseKit/Git`
//! folder): task worktrees and their registry, the commit graph's lanes,
//! ref names and decorations, History's filter, diff scopes, what a shell
//! command does with git, where workspaces change the same files, conflict
//! prompts, what a server says during a push, and the editor's change
//! marks, conflict blocks and blame text. Reading and writing a
//! repository is another crate's job; nothing here runs git.

pub mod command_inspector;
pub mod commit_graph;
pub mod conflict_prompt;
pub mod diff_scope;
pub mod editor_marks;
pub mod history_query;
pub mod refs;
pub mod server_message;
pub mod task_overlap;
pub mod task_registry;
pub mod worktree_tasks;

/// `fnmatch(3)` with no flags: `*` and `?` match any character, a slash
/// and a leading dot included; `[...]` is a set (`!` or `^` negates it,
/// `a-z` is a range); a backslash makes the next character literal.
pub fn fnmatch(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    matches_from(&pattern, &name)
}

fn matches_from(pattern: &[char], name: &[char]) -> bool {
    let (mut p, mut n) = (0, 0);
    // Where to resume after the last `*`: the pattern after it, and the
    // name position it has consumed up to.
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        let step = match pattern.get(p) {
            Some('*') => {
                star = Some((p + 1, n));
                p += 1;
                continue;
            }
            Some('?') => Some(1),
            Some('[') => match bracket(&pattern[p..], name[n]) {
                Some((true, len)) => Some(len),
                Some((false, _)) => None,
                // An unterminated set is a literal `[`.
                None => (name[n] == '[').then_some(1),
            },
            Some('\\') if p + 1 < pattern.len() => (pattern[p + 1] == name[n]).then_some(2),
            Some(&c) => (c == name[n]).then_some(1),
            None => None,
        };
        match step {
            Some(len) => {
                p += len;
                n += 1;
            }
            None => match star {
                Some((after, consumed)) => {
                    p = after;
                    n = consumed + 1;
                    star = Some((after, consumed + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

/// Whether the set at the start of `pattern` matches `c`, and how many
/// pattern characters it spans; `None` when it isn't closed.
fn bracket(pattern: &[char], c: char) -> Option<(bool, usize)> {
    let mut i = 1;
    let negated = matches!(pattern.get(i), Some('!') | Some('^'));
    if negated {
        i += 1;
    }
    let mut matched = false;
    let mut first = true;
    loop {
        let &item = pattern.get(i)?;
        if item == ']' && !first {
            return Some((matched != negated, i + 1));
        }
        first = false;
        let (low, mut next) = if item == '\\' {
            (*pattern.get(i + 1)?, i + 2)
        } else {
            (item, i + 1)
        };
        let mut high = low;
        if pattern.get(next) == Some(&'-') && pattern.get(next + 1).is_some_and(|&h| h != ']') {
            high = pattern[next + 1];
            next += 2;
        }
        if low <= c && c <= high {
            matched = true;
        }
        i = next;
    }
}

/// The last part of a path, as `NSString.lastPathComponent` gives it:
/// trailing slashes don't count.
pub(crate) fn last_path_component(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.is_empty() { "" } else { "/" };
    }
    match trimmed.rfind('/') {
        Some(slash) => &trimmed[slash + 1..],
        None => trimmed,
    }
}

/// Blanks as Swift's `.whitespaces` has them: spaces and tabs, not line
/// breaks.
pub(crate) fn is_blank(c: char) -> bool {
    c == '\t'
        || (c.is_whitespace()
            && !matches!(
                c,
                '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
            ))
}

pub(crate) fn trim_blanks(text: &str) -> &str {
    text.trim_matches(is_blank)
}

/// Line breaks as Swift's `.newlines` has them.
pub(crate) fn trim_newlines(text: &str) -> &str {
    text.trim_matches(|c| {
        matches!(
            c,
            '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnmatch_as_the_c_library_does_it() {
        assert!(fnmatch("*.lock", "composer.lock"));
        assert!(fnmatch("*", "a/b"), "no FNM_PATHNAME");
        assert!(fnmatch("src/*", "src/a/b.ts"));
        assert!(fnmatch(".env*", ".env.local"));
        assert!(fnmatch("a?c", "abc"));
        assert!(fnmatch("[a-c]x", "bx"));
        assert!(!fnmatch("[!a-c]x", "bx"));
        assert!(fnmatch("[^a-c]x", "dx"));
        assert!(fnmatch("[]]", "]"));
        assert!(fnmatch("[a", "[a"), "an unterminated set is literal");
        assert!(fnmatch(r"\*", "*"));
        assert!(!fnmatch(r"\*", "a"));
        assert!(!fnmatch("*.lock", "lock.txt"));
        assert!(fnmatch("*a*b", "xaxxb"));
        assert!(!fnmatch("", "a"));
        assert!(fnmatch("", ""));
    }

    #[test]
    fn last_components() {
        assert_eq!(last_path_component("a/b/"), "b");
        assert_eq!(last_path_component("b"), "b");
        assert_eq!(last_path_component("/"), "/");
        assert_eq!(last_path_component(""), "");
    }
}
