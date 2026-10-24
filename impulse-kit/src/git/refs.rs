//! Names around refs: which branch and tag names git accepts, what a
//! `git log --decorate=short` decoration is, a remote's web address, and
//! the next tag's name.

use url::Url;

/// Whether git accepts `name` for a branch or tag (the
/// `git check-ref-format` rules that matter for a single name typed by a
/// person).
pub fn is_valid_ref_name(name: &str) -> bool {
    if name.is_empty()
        || name == "@"
        || name.starts_with('-')
        || name.starts_with('/')
        || name.ends_with('/')
        || name.ends_with(".lock")
        || name.ends_with('.')
        || name.contains("..")
        || name.contains("//")
        || name.contains("@{")
    {
        return false;
    }
    // No component may start with a dot.
    if name.split('/').any(|part| part.starts_with('.')) {
        return false;
    }
    !name
        .chars()
        .any(|c| " ~^:?*[\\".contains(c) || is_control(c))
}

/// Control and format characters (Swift's `CharacterSet.controlCharacters`:
/// categories Cc and Cf).
fn is_control(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{AD}' | '\u{600}'..='\u{605}' | '\u{61C}' | '\u{6DD}' | '\u{70F}' | '\u{890}'..='\u{891}'
            | '\u{8E2}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}'
            | '\u{110BD}' | '\u{110CD}' | '\u{13430}'..='\u{1343F}' | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}' | '\u{E0001}' | '\u{E0020}'..='\u{E007F}')
}

/// One `git log --decorate=short` decoration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefDecoration {
    /// `HEAD -> main`: the checked-out branch.
    Head {
        branch: String,
    },
    /// `HEAD` alone: detached.
    DetachedHead,
    LocalBranch(String),
    RemoteBranch {
        remote: String,
        branch: String,
    },
    Tag(String),
}

impl RefDecoration {
    /// Parse a decoration; `remotes` tells `origin/main` from a local
    /// `feature/x`.
    pub fn parse(text: &str, remotes: &[String]) -> RefDecoration {
        if text == "HEAD" {
            return RefDecoration::DetachedHead;
        }
        if let Some(branch) = text.strip_prefix("HEAD -> ") {
            return RefDecoration::Head {
                branch: branch.to_string(),
            };
        }
        if let Some(tag) = text.strip_prefix("tag: ") {
            return RefDecoration::Tag(tag.to_string());
        }
        // The longest remote name that prefixes it (remotes may contain "/").
        let remote = remotes
            .iter()
            .filter(|r| text.starts_with(&format!("{r}/")))
            .max_by_key(|r| r.chars().count());
        match remote {
            Some(remote) => RefDecoration::RemoteBranch {
                remote: remote.clone(),
                branch: text[remote.len() + 1..].to_string(),
            },
            None => RefDecoration::LocalBranch(text.to_string()),
        }
    }

    /// The name shown on a chip.
    pub fn label(&self) -> String {
        match self {
            RefDecoration::Head { branch } => branch.clone(),
            RefDecoration::DetachedHead => "HEAD".into(),
            RefDecoration::LocalBranch(name) | RefDecoration::Tag(name) => name.clone(),
            RefDecoration::RemoteBranch { remote, branch } => format!("{remote}/{branch}"),
        }
    }
}

/// A remote's repository page, from its URL alone: `git@host:owner/repo.git`
/// becomes `https://host/owner/repo`. Impulse uses only git, so it knows
/// nothing about the host behind the address (no per-host page layouts).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteWebUrl {
    /// `https://host/owner/repo`, no trailing slash or `.git`.
    pub base: String,
}

impl RemoteWebUrl {
    /// Parse `git@host:owner/repo.git`, `ssh://git@host[:port]/owner/repo`,
    /// `https://user@host/owner/repo.git` and the like. `None` for local
    /// paths.
    pub fn parse(remote: &str) -> Option<Self> {
        let text = remote.trim();
        if text.is_empty() {
            return None;
        }
        let mut host: &str;
        let mut path: &str;
        // Plain-http servers stay http; everything else is browsed over https.
        let mut web_scheme = "https";
        if let Some(scheme_end) = text.find("://") {
            let scheme = text[..scheme_end].to_lowercase();
            if scheme == "http" {
                web_scheme = "http";
            }
            if !["https", "http", "ssh", "git", "git+ssh", "ssh+git"].contains(&scheme.as_str()) {
                return None;
            }
            let rest = &text[scheme_end + 3..];
            let slash = rest.find('/')?;
            host = &rest[..slash];
            path = &rest[slash + 1..];
            if let Some(at) = host.rfind('@') {
                host = &host[at + 1..];
            }
            // An ssh port isn't the web port.
            if let Some(colon) = host.find(':') {
                if scheme != "https" && scheme != "http" {
                    host = &host[..colon];
                }
            }
        } else {
            // scp-like: [user@]host:path
            let colon = text
                .find(':')
                .filter(|&colon| !text[..colon].contains('/'))?;
            host = &text[..colon];
            path = &text[colon + 1..];
            if let Some(at) = host.rfind('@') {
                host = &host[at + 1..];
            }
        }
        let host = host.to_lowercase();
        path = path.trim_matches('/');
        let path = path.strip_suffix(".git").unwrap_or(path);
        // A Windows drive ("C:\repo") isn't a host.
        if host.is_empty() || path.is_empty() || path.contains('\\') || host.chars().count() <= 1 {
            return None;
        }
        Some(Self {
            base: format!("{web_scheme}://{host}/{path}"),
        })
    }

    pub fn repository(&self) -> Option<Url> {
        Url::parse(&self.base).ok()
    }
}

/// A name for the next tag, given tags newest first: the newest
/// version-like one with its last number bumped ("v1.4.2" to "v1.4.3",
/// "release-7" to "release-8").
pub fn next_tag_name(tags: &[String]) -> Option<String> {
    for tag in tags {
        // The last run of digits, with a version-ish prefix before it.
        let Some(last_digit) = tag.rfind(|c: char| c.is_ascii_digit()) else {
            continue;
        };
        // Pre-release suffixes ("-rc1", "-beta") aren't bumped into.
        if last_digit + 1 != tag.len() {
            continue;
        }
        let start = tag[..last_digit]
            .rfind(|c: char| !c.is_ascii_digit())
            .map_or(0, |i| i + tag[i..].chars().next().map_or(1, char::len_utf8));
        let Some(number) = tag[start..]
            .parse::<u64>()
            .ok()
            .and_then(|n| n.checked_add(1))
        else {
            continue;
        };
        let prefix = &tag[..start];
        if prefix.is_empty() || prefix.ends_with(['.', 'v', '-', '_']) {
            return Some(format!("{prefix}{number}"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_names() {
        for name in [
            "main",
            "feature/login",
            "v1.2.3",
            "release-2026.10",
            "fix_#12",
            "ünïcode",
        ] {
            assert!(is_valid_ref_name(name), "{name}");
        }
        for name in [
            "",
            "@",
            "-x",
            "/x",
            "x/",
            "a..b",
            "a//b",
            "x.lock",
            "x.",
            "a b",
            "a~1",
            "a^",
            "a:b",
            "a?",
            "a*",
            "a[b",
            "a\\b",
            "a@{1}",
            ".hidden",
            "x/.y",
            "tab\there",
            "zero\u{200B}width",
        ] {
            assert!(!is_valid_ref_name(name), "{name}");
        }
    }

    #[test]
    fn decorations() {
        let remotes: Vec<String> = ["origin", "upstream", "my/fork"]
            .iter()
            .map(|r| r.to_string())
            .collect();
        let parse = |text: &str| RefDecoration::parse(text, &remotes);
        assert_eq!(
            parse("HEAD -> main"),
            RefDecoration::Head {
                branch: "main".into()
            }
        );
        assert_eq!(parse("HEAD"), RefDecoration::DetachedHead);
        assert_eq!(parse("tag: v1.0"), RefDecoration::Tag("v1.0".into()));
        assert_eq!(
            parse("origin/feature/x"),
            RefDecoration::RemoteBranch {
                remote: "origin".into(),
                branch: "feature/x".into()
            }
        );
        assert_eq!(
            parse("my/fork/main"),
            RefDecoration::RemoteBranch {
                remote: "my/fork".into(),
                branch: "main".into()
            }
        );
        assert_eq!(
            parse("feature/x"),
            RefDecoration::LocalBranch("feature/x".into())
        );
        assert_eq!(
            parse("origin/HEAD"),
            RefDecoration::RemoteBranch {
                remote: "origin".into(),
                branch: "HEAD".into()
            }
        );
        assert_eq!(parse("my/fork/main").label(), "my/fork/main");
    }

    #[test]
    fn remote_web_urls() {
        let cases = [
            (
                "git@github.com:owner/repo.git",
                "https://github.com/owner/repo",
            ),
            (
                "https://github.com/owner/repo",
                "https://github.com/owner/repo",
            ),
            (
                "https://user:token@github.com/owner/repo.git/",
                "https://github.com/owner/repo",
            ),
            (
                "ssh://git@github.com:22/owner/repo.git",
                "https://github.com/owner/repo",
            ),
            (
                "git@gitlab.com:group/sub/repo.git",
                "https://gitlab.com/group/sub/repo",
            ),
            (
                "git@git.example.edu:web/site.git",
                "https://git.example.edu/web/site",
            ),
            (
                "https://git.example.com:8443/team/repo.git",
                "https://git.example.com:8443/team/repo",
            ),
            ("http://intranet/team/repo.git", "http://intranet/team/repo"),
        ];
        for (remote, base) in cases {
            let url = RemoteWebUrl::parse(remote).unwrap_or_else(|| panic!("{remote}"));
            assert_eq!(url.base, base, "{remote}");
            assert_eq!(url.repository().unwrap().as_str(), base, "{remote}");
        }
        for remote in [
            "/srv/git/repo.git",
            "../repo",
            "file:///tmp/repo",
            "C:\\repos\\x",
            "",
        ] {
            assert_eq!(RemoteWebUrl::parse(remote), None, "{remote}");
        }
    }

    #[test]
    fn next_tag_names() {
        let next =
            |tags: &[&str]| next_tag_name(&tags.iter().map(|t| t.to_string()).collect::<Vec<_>>());
        assert_eq!(next(&["v1.4.2", "v1.4.1"]).as_deref(), Some("v1.4.3"));
        assert_eq!(next(&["1.9"]).as_deref(), Some("1.10"));
        assert_eq!(next(&["release-7"]).as_deref(), Some("release-8"));
        assert_eq!(
            next(&["v2.0.0-rc1", "v1.9.9"]).as_deref(),
            Some("v1.9.10"),
            "pre-releases are skipped"
        );
        assert_eq!(next(&["latest", "v3"]).as_deref(), Some("v4"));
        assert_eq!(next(&["latest"]), None);
        assert_eq!(next(&[]), None);
    }
}
