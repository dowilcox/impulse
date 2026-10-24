//! When a checkout's HEAD moves and dependency files come with it (a lock
//! file, a Dockerfile), the checkout's installed dependencies are stale:
//! tests then fail for the wrong reason. These rules tell which moves
//! matter and what to run for the files that changed, from the project's
//! `[on_change]` settings.

use std::collections::BTreeMap;

use super::detector::INSTALL_COMMANDS;
use crate::git::{fnmatch, last_path_component, trim_blanks};

/// How HEAD moved, from its reflog entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    /// Another branch or commit checked out (or reset to).
    Checkout,
    /// Changes brought in: a pull, merge or rebase.
    Merge,
    /// The checkout's own commit: whoever made it has what it needs.
    OwnCommit,
}

/// The move a reflog subject (`git reflog -1 --format=%gs`) describes.
pub fn move_of(reflog_subject: &str) -> Move {
    let action = reflog_subject
        .split(':')
        .find(|part| !part.is_empty())
        .map(trim_blanks)
        .unwrap_or("");
    if ["commit", "cherry-pick", "revert"]
        .iter()
        .any(|p| action.starts_with(p))
    {
        return Move::OwnCommit;
    }
    if ["pull", "merge", "rebase"]
        .iter()
        .any(|p| action.starts_with(p))
    {
        return Move::Merge;
    }
    Move::Checkout
}

/// Files whose change usually means dependencies to install or images to
/// rebuild.
pub fn is_known_file(name: &str) -> bool {
    INSTALL_COMMANDS
        .iter()
        .any(|(lock_file, _)| *lock_file == name)
        || [
            "Dockerfile",
            "docker-compose.yml",
            "docker-compose.yaml",
            "compose.yml",
            "compose.yaml",
        ]
        .contains(&name)
}

/// The `[on_change]` rules that match `changed` paths, as (file, command)
/// in the rules' order of names; a command shared by several files once. A
/// rule names a file (`composer.lock`, matched in any folder), a path
/// (`web/package-lock.json`) or a pattern (`*.lock`).
pub fn matches(rules: &BTreeMap<String, String>, changed: &[String]) -> Vec<(String, String)> {
    let mut result: Vec<(String, String)> = Vec::new();
    for (name, command) in rules {
        if command.is_empty() || result.iter().any(|(_, c)| c == command) {
            continue;
        }
        let Some(file) = changed.iter().find(|path| rule_matches(name, path)) else {
            continue;
        };
        result.push((file.clone(), command.clone()));
    }
    result
}

/// The known dependency files among `changed`.
pub fn known_changed(changed: &[String]) -> Vec<String> {
    changed
        .iter()
        .filter(|path| is_known_file(last_path_component(path)))
        .cloned()
        .collect()
}

fn rule_matches(rule: &str, path: &str) -> bool {
    if rule.contains('/') {
        fnmatch(rule, path)
    } else {
        fnmatch(rule, last_path_component(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn moves_are_read_from_the_reflog() {
        assert_eq!(move_of("commit: Add the cache"), Move::OwnCommit);
        assert_eq!(move_of("commit (amend): Add the cache"), Move::OwnCommit);
        assert_eq!(move_of("cherry-pick: Fix units"), Move::OwnCommit);
        assert_eq!(move_of("pull: Fast-forward"), Move::Merge);
        assert_eq!(
            move_of("pull --rebase (finish): returning to refs/heads/main"),
            Move::Merge
        );
        assert_eq!(
            move_of("merge origin/main: Merge made by the 'ort' strategy."),
            Move::Merge
        );
        assert_eq!(
            move_of("rebase (finish): returning to refs/heads/topic"),
            Move::Merge
        );
        assert_eq!(
            move_of("checkout: moving from main to upgrade"),
            Move::Checkout
        );
        assert_eq!(move_of("reset: moving to HEAD~1"), Move::Checkout);
    }

    #[test]
    fn rules_match_files_paths_and_patterns() {
        let rules: BTreeMap<String, String> = [
            ("composer.lock", "composer install"),
            ("web/package-lock.json", "cd web && npm ci"),
            ("*.gemspec", "bundle install"),
            ("Dockerfile", "docker compose build"),
            ("unused.lock", "nothing"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let changed = strings(&[
            "composer.lock",
            "web/package-lock.json",
            "app/Http/Kernel.php",
            "lib/thing.gemspec",
        ]);
        let found = matches(&rules, &changed);
        let files: Vec<&str> = found.iter().map(|(f, _)| f.as_str()).collect();
        let commands: Vec<&str> = found.iter().map(|(_, c)| c.as_str()).collect();
        assert_eq!(
            files,
            [
                "lib/thing.gemspec",
                "composer.lock",
                "web/package-lock.json"
            ]
        );
        assert_eq!(
            commands,
            ["bundle install", "composer install", "cd web && npm ci"]
        );
        let same: BTreeMap<String, String> = [("a", "same"), ("b", "same")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert_eq!(matches(&same, &strings(&["a", "b"])).len(), 1);
    }

    #[test]
    fn known_dependency_files() {
        assert_eq!(
            known_changed(&strings(&[
                "src/a.ts",
                "package-lock.json",
                "docker/Dockerfile",
                "compose.yaml"
            ])),
            ["package-lock.json", "docker/Dockerfile", "compose.yaml"]
        );
    }
}
