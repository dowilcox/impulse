//! Where a repository's workspaces change the same files: the main checkout
//! and every task Impulse made, each with the files it changes (its commits
//! since it left its base, plus its uncommitted files). Two agents rewriting
//! the same files only find out when the work is merged; this tells them,
//! and you, while it's still small.

use std::collections::BTreeSet;

use super::{fnmatch, last_path_component};

/// One workspace's changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changes {
    /// Its folder.
    pub path: String,
    pub name: String,
    pub files: BTreeSet<String>,
}

impl Changes {
    pub fn new(path: impl Into<String>, name: impl Into<String>, files: BTreeSet<String>) -> Self {
        Self {
            path: path.into(),
            name: name.into(),
            files,
        }
    }
}

/// Two workspaces that change the same files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pair {
    pub a: Changes,
    pub b: Changes,
    pub files: Vec<String>,
}

impl Pair {
    /// Identifies the pair, whichever order its workspaces come in.
    pub fn key(&self) -> String {
        let mut paths = [self.a.path.as_str(), self.b.path.as_str()];
        paths.sort();
        paths.join("\n")
    }

    /// The other workspace of the pair, seen from `path`.
    pub fn other(&self, path: &str) -> &Changes {
        if self.a.path == path {
            &self.b
        } else {
            &self.a
        }
    }

    pub fn contains(&self, path: &str) -> bool {
        self.a.path == path || self.b.path == path
    }
}

/// Every pair of workspaces with files in common, most shared files first.
/// Files matching `ignoring` (a name in any folder, a path, or a pattern)
/// don't count.
pub fn pairs(workspaces: &[Changes], ignoring: &[String]) -> Vec<Pair> {
    let mut pairs = Vec::new();
    for (index, a) in workspaces.iter().enumerate() {
        for b in &workspaces[index + 1..] {
            let shared: Vec<String> = a
                .files
                .intersection(&b.files)
                .filter(|file| !is_ignored(file, ignoring))
                .cloned()
                .collect();
            if !shared.is_empty() {
                pairs.push(Pair {
                    a: a.clone(),
                    b: b.clone(),
                    files: shared,
                });
            }
        }
    }
    pairs.sort_by(|x, y| {
        y.files
            .len()
            .cmp(&x.files.len())
            .then_with(|| x.key().cmp(&y.key()))
    });
    pairs
}

/// Whether `path` matches one of `patterns`: one with a slash against the
/// whole path, any other against the file's name.
pub fn is_ignored(path: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        if pattern.contains('/') {
            fnmatch(pattern, path)
        } else {
            fnmatch(pattern, last_path_component(path))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changes(path: &str, name: &str, files: &[&str]) -> Changes {
        Changes::new(path, name, files.iter().map(|f| f.to_string()).collect())
    }

    fn workspaces() -> [Changes; 4] {
        [
            changes(
                "/code/app",
                "app",
                &["src/page.tsx", "src/link.tsx", "composer.lock", "README.md"],
            ),
            changes(
                "/code/app.worktrees/upgrade",
                "upgrade",
                &["src/page.tsx", "src/link.tsx", "composer.lock"],
            ),
            changes("/code/app.worktrees/docs", "docs", &["README.md"]),
            changes("/code/app.worktrees/quiet", "quiet", &["other.txt"]),
        ]
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn pairs_share_files_most_first() {
        let [main, upgrade, docs, quiet] = workspaces();
        let found = pairs(&[main.clone(), upgrade.clone(), docs, quiet], &[]);
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[0].files,
            ["composer.lock", "src/link.tsx", "src/page.tsx"],
            "lock files count"
        );
        assert_eq!(found[0].other("/code/app").name, "upgrade");
        assert_eq!(found[1].files, ["README.md"]);
        let reversed = Pair {
            a: upgrade,
            b: main,
            files: vec![],
        };
        assert_eq!(found[0].key(), reversed.key(), "the key ignores order");
        assert!(!found
            .iter()
            .any(|p| p.contains("/code/app.worktrees/quiet")));
    }

    #[test]
    fn ignored_files_dont_count() {
        let [main, upgrade, docs, _] = workspaces();
        let found = pairs(
            &[main.clone(), upgrade.clone(), docs],
            &strings(&["*.lock", "README.md"]),
        );
        let files: Vec<Vec<String>> = found.into_iter().map(|p| p.files).collect();
        assert_eq!(files, [strings(&["src/link.tsx", "src/page.tsx"])]);
        let found = pairs(&[main, upgrade], &strings(&["src/*"]));
        let files: Vec<Vec<String>> = found.into_iter().map(|p| p.files).collect();
        assert_eq!(files, [strings(&["composer.lock"])]);
    }
}
