//! What a git server said during a push: the `remote:` lines git prints.
//! Hosts use them for links ("To create a merge request for x, visit:"
//! followed by the address), and Impulse shows them as they are. It's git's
//! own channel, so it works the same with any host, and Impulse never needs
//! to know which host it is.

use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitServerMessage {
    /// The server's lines, without the `remote:` prefix, blank lines and
    /// pack progress ("Counting objects: 100%").
    pub lines: Vec<String>,
    /// The first web address in them, as written.
    pub link: Option<String>,
    /// What the server said about the link: the text before it on its
    /// line, or the line before.
    pub link_caption: Option<String>,
}

impl GitServerMessage {
    /// The message in a push's output lines; `None` when the server said
    /// nothing worth showing.
    pub fn parse<S: AsRef<str>>(output: &[S]) -> Option<GitServerMessage> {
        let mut lines: Vec<String> = Vec::new();
        for raw in output {
            let Some(text) = raw.as_ref().trim().strip_prefix("remote:") else {
                continue;
            };
            let text = super::trim_blanks(text);
            if text.is_empty() || is_progress(text) {
                continue;
            }
            lines.push(text.to_string());
        }
        if lines.is_empty() {
            return None;
        }
        let mut link = None;
        let mut link_caption = None;
        for (index, line) in lines.iter().enumerate() {
            let Some((start, address)) = first_address(line) else {
                continue;
            };
            if Url::parse(address).is_err() {
                continue;
            }
            link = Some(address.to_string());
            let before = super::trim_blanks(&line[..start]);
            link_caption = if before.is_empty() {
                index.checked_sub(1).map(|i| lines[i].clone())
            } else {
                Some(before.to_string())
            };
            break;
        }
        Some(GitServerMessage {
            lines,
            link,
            link_caption,
        })
    }
}

/// The first `https?://` address in `line` and where it starts.
fn first_address(line: &str) -> Option<(usize, &str)> {
    for (start, _) in line.match_indices("http") {
        let rest = &line[start + 4..];
        let scheme = if rest.starts_with("s://") {
            8
        } else if rest.starts_with("://") {
            7
        } else {
            continue;
        };
        let tail = &line[start + scheme..];
        let length = tail.find(char::is_whitespace).unwrap_or(tail.len());
        if length > 0 {
            return Some((start, &line[start..start + scheme + length]));
        }
    }
    None
}

/// git's pack progress, relayed from the server.
fn is_progress(text: &str) -> bool {
    [
        "Enumerating objects",
        "Counting objects",
        "Compressing objects",
        "Total ",
        "Resolving deltas",
    ]
    .iter()
    .any(|p| text.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_on_the_next_line() {
        let output = [
            "Enumerating objects: 5, done.",
            "remote: Counting objects: 100% (5/5), done.",
            "remote: ",
            "remote: To create a merge request for fix-elevation, visit:",
            "remote:   https://git.example.edu/web/trailhead/-/merge_requests/new?merge_request%5Bsource_branch%5D=fix-elevation",
            "remote: ",
            "To git.example.edu:web/trailhead.git",
            " * [new branch]      fix-elevation -> fix-elevation",
        ];
        let message = GitServerMessage::parse(&output).unwrap();
        assert_eq!(message.lines.len(), 2);
        assert_eq!(
            message.link_caption.as_deref(),
            Some("To create a merge request for fix-elevation, visit:")
        );
        assert_eq!(
            message.link.as_deref(),
            Some("https://git.example.edu/web/trailhead/-/merge_requests/new?merge_request%5Bsource_branch%5D=fix-elevation")
        );
    }

    #[test]
    fn a_link_after_text_on_the_same_line() {
        let message =
            GitServerMessage::parse(&["remote: View it at https://example.com/r/12 now"]).unwrap();
        assert_eq!(message.link_caption.as_deref(), Some("View it at"));
        assert_eq!(message.link.as_deref(), Some("https://example.com/r/12"));
    }

    #[test]
    fn text_without_a_link() {
        let message = GitServerMessage::parse(&["remote: warning: large files detected"]).unwrap();
        assert_eq!(message.lines, ["warning: large files detected"]);
        assert_eq!(message.link, None);
        assert_eq!(message.link_caption, None);
    }

    #[test]
    fn only_progress_is_nothing() {
        assert_eq!(GitServerMessage::parse::<&str>(&[]), None);
        assert_eq!(
            GitServerMessage::parse(&[
                "remote: Resolving deltas: 100% (2/2), completed with 2 local objects.",
                "remote: Total 3 (delta 1)",
                "To example.com:r.git",
                "   abc..def  main -> main",
            ]),
            None
        );
    }
}
