//! Linked editing of HTML and XML tags (Monaco's `linkedEditing`, on in the
//! Swift editor): with the cursor in a tag's name, the name of the tag that
//! pairs with it changes along with it.

use std::ops::Range;

/// Languages whose tags are linked.
pub fn applies(language: &str) -> bool {
    matches!(
        language,
        "html" | "xml" | "vue" | "svelte" | "javascriptreact" | "typescriptreact"
    )
}

/// HTML's elements that never close.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | ':' | '.' | '_')
}

/// One tag's name in the text.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Tag {
    name: Range<usize>,
    closing: bool,
}

/// Every opening and closing tag (self-closing and void ones left out),
/// skipping comments, CDATA, doctypes and processing instructions.
fn tags(text: &str) -> Vec<Tag> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(at) = text[i..].find('<').map(|p| i + p) {
        let rest = &text[at..];
        if rest.starts_with("<!--") {
            i = rest.find("-->").map_or(text.len(), |p| at + p + 3);
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            i = rest.find('>').map_or(text.len(), |p| at + p + 1);
            continue;
        }
        let closing = bytes.get(at + 1) == Some(&b'/');
        let start = at + 1 + usize::from(closing);
        let end = start
            + text[start..]
                .char_indices()
                .find(|(_, c)| !is_name_char(*c))
                .map_or(text.len() - start, |(p, _)| p);
        if end == start {
            i = at + 1;
            continue;
        }
        // The tag's end, past quoted attribute values.
        let mut j = end;
        let mut quote = None;
        let mut self_closing = false;
        while j < bytes.len() {
            let b = bytes[j];
            match quote {
                Some(q) if b == q => quote = None,
                Some(_) => {}
                None if b == b'"' || b == b'\'' => quote = Some(b),
                None if b == b'>' => {
                    self_closing = j > 0 && bytes[j - 1] == b'/';
                    break;
                }
                None => {}
            }
            j += 1;
        }
        let name = &text[start..end];
        let void = VOID.iter().any(|v| v.eq_ignore_ascii_case(name));
        if !self_closing && !(void && !closing) {
            out.push(Tag {
                name: start..end,
                closing,
            });
        }
        i = (j + 1).min(text.len()).max(at + 1);
    }
    out
}

/// With `offset` in (or at either end of) a tag's name: that name's range
/// and the range of its partner's name, when it has one with the same
/// name.
pub fn linked_names(text: &str, offset: usize) -> Option<(Range<usize>, Range<usize>)> {
    let tags = tags(text);
    let here = tags
        .iter()
        .position(|t| t.name.start <= offset && offset <= t.name.end)?;
    let name = &text[tags[here].name.clone()];
    let partner = if tags[here].closing {
        // Back to the opening tag this closes.
        let mut depth = 0;
        tags[..here].iter().rev().find(|t| {
            if &text[t.name.clone()] != name {
                return false;
            }
            if t.closing {
                depth += 1;
                false
            } else if depth == 0 {
                true
            } else {
                depth -= 1;
                false
            }
        })
    } else {
        let mut depth = 0;
        tags[here + 1..].iter().find(|t| {
            if &text[t.name.clone()] != name {
                return false;
            }
            if !t.closing {
                depth += 1;
                false
            } else if depth == 0 {
                true
            } else {
                depth -= 1;
                false
            }
        })
    }?;
    Some((tags[here].name.clone(), partner.name.clone()))
}

/// Whether `name` can stand as a tag name (an empty one is allowed while
/// it's being retyped).
pub fn is_name(name: &str) -> bool {
    name.chars().all(is_name_char)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_opening_and_closing_tags() {
        let text = "<div class=\"a>b\"><p>x</p><br><img/><div>y</div></div>";
        let open = text.find("div").unwrap();
        let (here, there) = linked_names(text, open + 1).unwrap();
        assert_eq!(&text[here.clone()], "div");
        assert_eq!(there.start, text.rfind("div").unwrap());
        // From the closing tag back to its opening one.
        assert_eq!(linked_names(text, there.start), Some((there, here)));
        // The inner p.
        let p = text.find("<p>").unwrap() + 1;
        let (_, close) = linked_names(text, p).unwrap();
        assert_eq!(close.start, text.find("</p>").unwrap() + 2);
        // A void tag and a self-closing one have no partner.
        assert!(linked_names(text, text.find("br").unwrap()).is_none());
        assert!(linked_names(text, text.find("img").unwrap()).is_none());
        // Outside any tag name.
        assert!(linked_names(text, text.find('x').unwrap()).is_none());
        // Comments are skipped.
        assert!(linked_names("<!-- <a> --><b></b>", 6).is_none());
    }
}
