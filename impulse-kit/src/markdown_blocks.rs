//! Markdown as blocks and styled inline spans, for the native preview (the
//! Swift app renders comrak's HTML in a web view; the Rust app draws these
//! with the same page's styles). Parsed with `comrak` as
//! `markdown_preview::render_html_body` does: GitHub tables,
//! strikethrough and task lists, raw HTML left out.

use comrak::nodes::{AstNode, ListType, NodeValue, TableAlignment};

/// One run of text and how it looks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    /// Inline code.
    pub code: bool,
    /// A link's target.
    pub link: Option<String>,
}

/// A paragraph's (or heading's, or cell's) text.
pub type Inlines = Vec<Span>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    /// A task item: checked or not.
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        text: Inlines,
    },
    Paragraph(Inlines),
    /// A fenced or indented code block; `language` is the info string's
    /// first word.
    Code {
        language: String,
        text: String,
    },
    Quote(Vec<Block>),
    List {
        ordered: bool,
        start: usize,
        items: Vec<ListItem>,
    },
    Table {
        aligns: Vec<Align>,
        header: Vec<Inlines>,
        rows: Vec<Vec<Inlines>>,
    },
    /// An image on its own line (inside text, an image shows its alt text).
    Image {
        url: String,
        alt: String,
    },
    Rule,
}

/// The shells a code block's language names: these get a Run button.
pub fn is_shell(language: &str) -> bool {
    matches!(
        language,
        "bash" | "sh" | "shell" | "zsh" | "fish" | "console" | "shell-session" | "terminal"
    )
}

/// A shell block's command for its Run button: `$ ` prompts stripped, and
/// when a block has any, only the prompted lines.
pub fn shell_command(text: &str) -> String {
    let lines: Vec<&str> = text.trim_end_matches('\n').split('\n').collect();
    let prompted: Vec<&str> = lines
        .iter()
        .filter_map(|l| l.trim_start().strip_prefix("$ "))
        .collect();
    if prompted.is_empty() {
        lines.join("\n")
    } else {
        prompted.join("\n")
    }
}

/// `source` as blocks.
pub fn parse(source: &str) -> Vec<Block> {
    let arena = comrak::Arena::new();
    let mut options = comrak::Options::default();
    options.extension.table = true;
    options.extension.strikethrough = true;
    options.extension.tasklist = true;
    let root = comrak::parse_document(&arena, source, &options);
    blocks(root)
}

fn blocks<'a>(parent: &'a AstNode<'a>) -> Vec<Block> {
    parent.children().filter_map(block).collect()
}

fn block<'a>(node: &'a AstNode<'a>) -> Option<Block> {
    let value = node.data().value.clone();
    Some(match value {
        NodeValue::Heading(heading) => Block::Heading {
            level: heading.level,
            text: inlines(node),
        },
        NodeValue::Paragraph => {
            // An image alone in its paragraph is drawn as an image.
            let children: Vec<_> = node.children().collect();
            if let [only] = children.as_slice() {
                if let NodeValue::Image(link) = &only.data().value {
                    return Some(Block::Image {
                        url: link.url.clone(),
                        alt: plain_text(only),
                    });
                }
            }
            Block::Paragraph(inlines(node))
        }
        NodeValue::CodeBlock(code) => Block::Code {
            language: code
                .info
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_lowercase(),
            text: code.literal.trim_end_matches('\n').to_string(),
        },
        NodeValue::BlockQuote | NodeValue::MultilineBlockQuote(_) | NodeValue::Alert(_) => {
            Block::Quote(blocks(node))
        }
        NodeValue::List(list) => Block::List {
            ordered: list.list_type == ListType::Ordered,
            start: list.start.max(1),
            items: node
                .children()
                .map(|item| {
                    let task = match &item.data().value {
                        NodeValue::TaskItem(task) => Some(task.symbol.is_some()),
                        _ => None,
                    };
                    ListItem {
                        task,
                        blocks: blocks(item),
                    }
                })
                .collect(),
        },
        NodeValue::Table(table) => {
            let aligns = table
                .alignments
                .iter()
                .map(|a| match a {
                    TableAlignment::Left => Align::Left,
                    TableAlignment::Center => Align::Center,
                    TableAlignment::Right => Align::Right,
                    TableAlignment::None => Align::None,
                })
                .collect();
            let mut header = Vec::new();
            let mut rows = Vec::new();
            for row in node.children() {
                let cells: Vec<Inlines> = row.children().map(inlines).collect();
                match row.data().value {
                    NodeValue::TableRow(true) => header = cells,
                    _ => rows.push(cells),
                }
            }
            Block::Table {
                aligns,
                header,
                rows,
            }
        }
        NodeValue::ThematicBreak => Block::Rule,
        // Raw HTML is left out (cmark-gfm's safe mode in the Swift app).
        _ => return None,
    })
}

/// The text of `node`'s inline children as styled spans.
fn inlines<'a>(node: &'a AstNode<'a>) -> Inlines {
    let mut spans = Vec::new();
    collect(node, &Span::default(), &mut spans);
    // Neighbours that look the same join up.
    let mut merged: Inlines = Vec::new();
    for span in spans {
        match merged.last_mut() {
            Some(last)
                if (last.bold, last.italic, last.strike, last.code, &last.link)
                    == (span.bold, span.italic, span.strike, span.code, &span.link) =>
            {
                last.text.push_str(&span.text)
            }
            _ if !span.text.is_empty() => merged.push(span),
            _ => {}
        }
    }
    merged
}

fn collect<'a>(node: &'a AstNode<'a>, style: &Span, out: &mut Inlines) {
    for child in node.children() {
        let value = child.data().value.clone();
        let text = |text: &str| Span {
            text: text.to_string(),
            ..style.clone()
        };
        match value {
            NodeValue::Text(t) => out.push(text(&t)),
            NodeValue::SoftBreak => out.push(text(" ")),
            NodeValue::LineBreak => out.push(text("\n")),
            NodeValue::Code(code) => out.push(Span {
                text: code.literal.clone(),
                code: true,
                ..style.clone()
            }),
            NodeValue::Emph => collect(
                child,
                &Span {
                    italic: true,
                    ..style.clone()
                },
                out,
            ),
            NodeValue::Strong => collect(
                child,
                &Span {
                    bold: true,
                    ..style.clone()
                },
                out,
            ),
            NodeValue::Strikethrough => collect(
                child,
                &Span {
                    strike: true,
                    ..style.clone()
                },
                out,
            ),
            NodeValue::Link(link) => collect(
                child,
                &Span {
                    link: Some(link.url.clone()),
                    ..style.clone()
                },
                out,
            ),
            // Inside text an image is its alt text.
            NodeValue::Image(_) => collect(child, style, out),
            NodeValue::HtmlInline(_) | NodeValue::Raw(_) => {}
            _ => collect(child, style, out),
        }
    }
}

fn plain_text<'a>(node: &'a AstNode<'a>) -> String {
    let mut spans = Vec::new();
    collect(node, &Span::default(), &mut spans);
    spans.into_iter().map(|s| s.text).collect()
}

/// Where a link in the preview goes.
#[derive(Debug, PartialEq)]
pub enum LinkTarget {
    /// A web or mail link, for the browser.
    Web(String),
    /// A `#section` of the page (`heading_slugs` names them).
    Anchor(String),
    /// A file, relative to the Markdown file's folder or absolute.
    File(std::path::PathBuf),
    /// Anything else, which stays put (as the Swift preview's other schemes).
    Other,
}

/// What clicking `href` in a preview of a file in `folder` does: web and
/// mail links open in the browser, `#anchors` scroll the page, and paths
/// (decoded, without their own `#fragment`) open in the editor.
pub fn resolve_link(href: &str, folder: &std::path::Path) -> LinkTarget {
    if let Some(fragment) = href.strip_prefix('#') {
        let decoded = url::form_urlencoded::parse(format!("x={fragment}").as_bytes())
            .next()
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default();
        return LinkTarget::Anchor(decoded.to_lowercase());
    }
    let lower = href.to_ascii_lowercase();
    if ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
    {
        return LinkTarget::Web(href.to_string());
    }
    let Ok(base) = url::Url::from_directory_path(folder) else {
        return LinkTarget::Other;
    };
    match base.join(href) {
        Ok(url) if url.scheme() == "file" => url
            .to_file_path()
            .map(LinkTarget::File)
            .unwrap_or(LinkTarget::Other),
        _ => LinkTarget::Other,
    }
}

/// Each top-level heading's index in `blocks` and its anchor, as GitHub
/// names them: lowercased, punctuation dropped, spaces as hyphens, and a
/// repeated name numbered (`-1`, `-2`).
pub fn heading_slugs(blocks: &[Block]) -> Vec<(usize, String)> {
    let mut seen: std::collections::HashMap<String, usize> = Default::default();
    blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| match block {
            Block::Heading { text, .. } => Some((index, text)),
            _ => None,
        })
        .map(|(index, text)| {
            let words: String = text.iter().map(|span| span.text.as_str()).collect();
            let base: String = words
                .to_lowercase()
                .chars()
                .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
                .map(|c| if c == ' ' { '-' } else { c })
                .collect();
            let count = seen.entry(base.clone()).or_insert(0);
            let slug = match *count {
                0 => base.clone(),
                n => format!("{base}-{n}"),
            };
            *count += 1;
            (index, slug)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(text: &str) -> Span {
        Span {
            text: text.into(),
            ..Default::default()
        }
    }

    #[test]
    fn reads_headings_paragraphs_and_inline_styles() {
        let blocks = parse("# Title\n\nSome **bold** and `code`, a [link](https://x.dev).\n");
        assert_eq!(
            blocks[0],
            Block::Heading {
                level: 1,
                text: vec![span("Title")]
            }
        );
        let Block::Paragraph(text) = &blocks[1] else {
            panic!("{blocks:?}")
        };
        assert_eq!(text[0], span("Some "));
        assert!(text[1].bold && text[1].text == "bold");
        assert!(text[3].code && text[3].text == "code");
        assert_eq!(text[5].link.as_deref(), Some("https://x.dev"));
    }

    #[test]
    fn reads_tables_lists_code_and_rules() {
        let source = "| A | B |\n|---|--:|\n| 1 | 2 |\n\n- [x] done\n- todo\n\n3. three\n\n```sh\n$ npm test\nok\n```\n\n---\n";
        let blocks = parse(source);
        let Block::Table {
            aligns,
            header,
            rows,
        } = &blocks[0]
        else {
            panic!("{blocks:?}")
        };
        assert_eq!(aligns, &vec![Align::None, Align::Right]);
        assert_eq!(header[1], vec![span("B")]);
        assert_eq!(rows[0][0], vec![span("1")]);
        let Block::List { ordered, items, .. } = &blocks[1] else {
            panic!("{blocks:?}")
        };
        assert!(!ordered);
        assert_eq!(items[0].task, Some(true));
        assert_eq!(items[1].task, None);
        assert!(matches!(
            blocks[2],
            Block::List {
                ordered: true,
                start: 3,
                ..
            }
        ));
        assert_eq!(
            blocks[3],
            Block::Code {
                language: "sh".into(),
                text: "$ npm test\nok".into()
            }
        );
        assert_eq!(blocks[4], Block::Rule);
    }

    #[test]
    fn leaves_raw_html_out_and_takes_lone_images() {
        let blocks = parse("<div>raw</div>\n\n![logo](img/logo.png)\n\ntext <b>bold</b>\n");
        assert_eq!(
            blocks[0],
            Block::Image {
                url: "img/logo.png".into(),
                alt: "logo".into()
            }
        );
        assert_eq!(blocks[1], Block::Paragraph(vec![span("text bold")]));
    }

    #[test]
    fn shell_blocks_run_their_prompted_lines() {
        assert!(is_shell("sh") && is_shell("zsh") && !is_shell("rust"));
        assert_eq!(
            shell_command("npm install\nnpm run dev\n"),
            "npm install\nnpm run dev"
        );
        assert_eq!(shell_command("$ npm test\nok\n  $ ls\n"), "npm test\nls");
    }

    #[test]
    fn links_resolve_as_the_preview_follows_them() {
        let folder = std::path::Path::new("/repo/docs");
        assert_eq!(
            resolve_link("https://example.com", folder),
            LinkTarget::Web("https://example.com".into())
        );
        assert_eq!(
            resolve_link("#Get%20Started", folder),
            LinkTarget::Anchor("get started".into())
        );
        assert_eq!(
            resolve_link("guide%20one.md#setup", folder),
            LinkTarget::File("/repo/docs/guide one.md".into())
        );
        assert_eq!(
            resolve_link("../README.md", folder),
            LinkTarget::File("/repo/README.md".into())
        );
        assert_eq!(
            resolve_link("javascript:alert(1)", folder),
            LinkTarget::Other
        );
    }

    #[test]
    fn headings_get_github_anchors() {
        let blocks = vec![
            Block::Heading {
                level: 1,
                text: vec![span("Get Started!")],
            },
            Block::Paragraph(vec![span("x")]),
            Block::Heading {
                level: 2,
                text: vec![span("Get Started")],
            },
        ];
        assert_eq!(
            heading_slugs(&blocks),
            vec![
                (0, "get-started".to_string()),
                (2, "get-started-1".to_string())
            ]
        );
    }
}
