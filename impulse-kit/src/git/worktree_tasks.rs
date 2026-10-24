//! Naming and setup rules for task worktrees: a branch per task, checked
//! out next to the repository in `<repo>.worktrees/<branch>`, with the
//! untracked files a checkout lacks (.env and friends) copied over.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

use super::task_registry::{join, parent, standardize};
use super::{fnmatch, last_path_component};

/// A branch name for a task title: lowercase words joined by dashes, made
/// unique against `taken`.
pub fn branch_name(title: &str, taken: &HashSet<String>) -> String {
    let folded: String = title
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase();
    let mut slug = String::new();
    for c in folded.chars() {
        if c.is_ascii_alphanumeric() || c == '/' {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') && !slug.ends_with('/') {
            slug.push('-');
        }
    }
    let mut slug: String = slug
        .trim_end_matches(['-', '/'])
        .chars()
        .take(48)
        .collect::<String>()
        .trim_end_matches('-')
        .to_string();
    if slug.is_empty() {
        slug = "task".to_string();
    }
    let mut candidate = slug.clone();
    let mut suffix = 2;
    while taken.contains(&candidate) {
        candidate = format!("{slug}-{suffix}");
        suffix += 1;
    }
    candidate
}

/// `<parent>/<repo>.worktrees/<branch>` (slashes in the branch become
/// dashes so each task is one folder).
pub fn worktree_path(repo_root: &str, branch: &str) -> String {
    let root = standardize(repo_root);
    let folder = format!("{}.worktrees", last_path_component(&root));
    join(&join(&parent(&root), &folder), &branch.replace('/', "-"))
}

/// What a new task starts from, given the main checkout's branch: the
/// remote branch it tracks (`origin/main`), so a task starts from what's on
/// the remote and not from a local branch that may be behind; the local
/// branch when it tracks nothing; `HEAD` when detached. The note says when
/// the local branch has commits the remote one lacks.
pub fn default_base(
    branch: Option<&str>,
    upstream: Option<&str>,
    ahead: usize,
) -> (String, Option<String>) {
    let Some(branch) = branch else {
        return ("HEAD".into(), None);
    };
    let Some(upstream) = upstream else {
        return (branch.into(), None);
    };
    if ahead == 0 {
        return (upstream.into(), None);
    }
    let (commits, verb, them) = if ahead == 1 {
        ("1 unpushed commit".to_string(), "isn't", "it")
    } else {
        (format!("{ahead} unpushed commits"), "aren't", "them")
    };
    (
        upstream.into(),
        Some(format!(
            "{branch} has {commits} that {verb} included; type {branch} to include {them}."
        )),
    )
}

/// What's copied when a repository has no `.worktreeinclude`: local
/// secrets, and project-only agent hooks (so a task keeps the project's
/// hooks, and archiving it doesn't take the only copy with it).
pub const DEFAULT_INCLUDE_PATTERNS: [&str; 3] =
    [".env", ".env.local", ".claude/settings.local.json"];

/// Files to copy into a new worktree: the patterns in `.worktreeinclude`
/// (one per line, `#` comments), or `DEFAULT_INCLUDE_PATTERNS` when there's
/// no such file.
pub fn include_patterns(file: Option<&str>) -> Vec<String> {
    let Some(text) = file else {
        return DEFAULT_INCLUDE_PATTERNS
            .iter()
            .map(|p| p.to_string())
            .collect();
    };
    text.split(['\n', '\r'])
        .map(super::trim_blanks)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.strip_prefix('/').unwrap_or(line).to_string())
        .collect()
}

/// Relative paths under `root` matching `patterns`. A pattern names a file
/// or uses shell wildcards within one folder (`.env*`, `config/*.json`);
/// folders aren't searched recursively.
pub fn matching_files(patterns: &[String], root: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    // Only files inside the repository: a pattern from a cloned repo's
    // project.toml mustn't copy `../../.ssh/id_ed25519` into a task.
    for pattern in patterns {
        if pattern.starts_with('/')
            || pattern.starts_with('~')
            || pattern.split('/').any(|p| p == "..")
        {
            continue;
        }
        let folder = parent(pattern);
        let leaf = last_path_component(pattern);
        let base = if folder.is_empty() {
            root.to_string()
        } else {
            join(root, &folder)
        };
        let candidates: Vec<String> = if leaf.contains(['*', '?', '[']) {
            let mut names: Vec<String> = fs::read_dir(&base)
                .map(|entries| {
                    entries
                        .flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .filter(|name| fnmatch(leaf, name))
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
        } else {
            vec![leaf.to_string()]
        };
        for name in candidates {
            let relative = if folder.is_empty() {
                name
            } else {
                join(&folder, &name)
            };
            let absolute = join(root, &relative);
            if Path::new(&absolute).is_file() && !found.contains(&relative) {
                found.push(relative);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn taken(names: &[&str]) -> HashSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn branch_names_are_slugs_and_unique() {
        assert_eq!(
            branch_name("Fix the login bug!", &taken(&[])),
            "fix-the-login-bug"
        );
        assert_eq!(
            branch_name("  Ünïcode — dashes  ", &taken(&[])),
            "unicode-dashes"
        );
        assert_eq!(branch_name("feat/Search UI", &taken(&[])), "feat/search-ui");
        assert_eq!(branch_name("!!!", &taken(&[])), "task");
        assert_eq!(
            branch_name("fix bug", &taken(&["fix-bug", "fix-bug-2"])),
            "fix-bug-3"
        );
        let long = "word ".repeat(30);
        assert!(branch_name(&long, &taken(&[])).chars().count() <= 48);
    }

    #[test]
    fn tasks_start_from_the_remote_branch() {
        assert_eq!(
            default_base(Some("main"), Some("origin/main"), 0),
            ("origin/main".into(), None)
        );
        assert_eq!(default_base(Some("main"), None, 0), ("main".into(), None));
        assert_eq!(default_base(None, None, 0), ("HEAD".into(), None));
        let one = default_base(Some("main"), Some("origin/main"), 1);
        assert_eq!(one.0, "origin/main");
        assert_eq!(
            one.1.as_deref(),
            Some("main has 1 unpushed commit that isn't included; type main to include it.")
        );
        assert_eq!(
            default_base(Some("main"), Some("upstream/main"), 2)
                .1
                .as_deref(),
            Some("main has 2 unpushed commits that aren't included; type main to include them.")
        );
    }

    #[test]
    fn worktrees_live_beside_the_repository() {
        assert_eq!(
            worktree_path("/Users/me/Code/app", "feat/search"),
            "/Users/me/Code/app.worktrees/feat-search"
        );
        assert_eq!(
            worktree_path("/Users/me/Code/app/", "fix"),
            "/Users/me/Code/app.worktrees/fix"
        );
    }

    #[test]
    fn include_patterns_default_to_env_files_and_project_hooks() {
        assert_eq!(
            include_patterns(None),
            [".env", ".env.local", ".claude/settings.local.json"]
        );
        assert_eq!(
            include_patterns(Some("# secrets\n.env*\n\n/config/local.json\n")),
            [".env*", "config/local.json"]
        );
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn matches_files_and_single_folder_wildcards() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        fs::create_dir_all(format!("{root}/config/nested")).unwrap();
        for file in [
            ".env",
            ".env.test",
            "config/a.local.json",
            "config/b.json",
            "config/nested/c.local.json",
        ] {
            fs::write(format!("{root}/{file}"), "x").unwrap();
        }
        let mut env = matching_files(&strings(&[".env*"]), &root);
        env.sort();
        assert_eq!(env, [".env", ".env.test"]);
        assert_eq!(
            matching_files(&strings(&["config/*.local.json"]), &root),
            ["config/a.local.json"]
        );
        assert_eq!(
            matching_files(&strings(&[".env", "missing.txt", "config"]), &root),
            [".env"]
        );
        // Nothing outside the repository.
        let outside = last_path_component(&root).to_string();
        assert!(matching_files(
            &strings(&[
                &format!("../{outside}/.env"),
                "/etc/hosts",
                "config/../.env"
            ]),
            &root
        )
        .is_empty());
    }

    #[test]
    fn defaults_copy_project_agent_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        fs::create_dir_all(format!("{root}/.claude")).unwrap();
        for file in [
            ".env",
            ".claude/settings.local.json",
            ".claude/settings.json",
        ] {
            fs::write(format!("{root}/{file}"), "x").unwrap();
        }
        assert_eq!(
            matching_files(&include_patterns(None), &root),
            [".env", ".claude/settings.local.json"]
        );
    }
}
