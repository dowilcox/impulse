//! The Markdown preview, drawn natively from `impulse_kit::markdown_blocks`
//! with the Swift preview page's styles (`markdown_preview::PAGE`): 15 pt
//! text at 1.6 lines, headings in the theme's heading color (h1 and h2
//! ruled), code in its own background with tree-sitter colors, tables with
//! a shaded header, quotes with a left rule, task lists, images from beside
//! the file. Shell blocks get a Run button, as the Swift page's
//! `run_button_script` adds. Web and mail links open in the browser,
//! links to files open in the editor and `#anchors` scroll to the heading.
//! Block margins collapse between neighbors, as the page's CSS margins do,
//! and inline code is set at 0.9em of its text, as the page's `code` is.
//! Its text selects as a page's does (GPUI Kit's `SelectableText`): a drag
//! across blocks, a double click on a word, ⌘A once a click in the preview
//! has given it the keyboard, and ⌘C copies the selection as plain text.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::{SelectableText, SelectableTextDocument, SelectableTextGroup, TextSelection};
use gpui_kit::component::highlighter::SyntaxHighlighter;
use gpui_kit::component::input::{Copy, Rope, SelectAll};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::{
    div, prelude::*, px, AnyElement, App, Context, FontStyle, FontWeight, Hsla, KeyBinding,
    SharedString, StrikethroughStyle, TextRun,
};
use impulse_kit::markdown_blocks::{self, Align, Block, Inlines};

use crate::editor_tab::EditorTab;
use crate::workbench_chrome::hsla;

/// The preview page's colors (`markdown_theme::theme_to_markdown_colors`).
#[derive(Clone, Copy)]
pub(crate) struct MarkdownColors {
    pub bg: Hsla,
    pub fg: Hsla,
    pub heading: Hsla,
    pub link: Hsla,
    pub code_bg: Hsla,
    pub border: Hsla,
    pub quote: Hsla,
}

impl gpui_kit::Global for MarkdownColors {}

impl MarkdownColors {
    pub(crate) fn new(theme: &impulse_kit::theme::ResolvedTheme) -> Self {
        let c = impulse_kit::markdown_theme::theme_to_markdown_colors(theme);
        let color = |hex: &str| hsla(impulse_kit::chrome::Rgba::from_hex(hex));
        MarkdownColors {
            bg: color(&c.bg),
            fg: color(&c.fg),
            heading: color(&c.heading),
            link: color(&c.link),
            code_bg: color(&c.code_bg),
            border: color(&c.border),
            quote: color(&c.blockquote_fg),
        }
    }
}

const BODY: f32 = 15.;
const MONO: &str = "JetBrains Mono";

/// What a preview needs while it's built.
struct Page<'a> {
    colors: MarkdownColors,
    /// The Markdown file's folder, for images.
    folder: PathBuf,
    font: &'a str,
    /// Ids for interactive text and buttons.
    next_id: usize,
    /// Where links go: the editor (to open files) and the page's scroll,
    /// with each heading's anchor and block.
    editor: gpui_kit::WeakEntity<EditorTab>,
    scroll: gpui_kit::ScrollHandle,
    anchors: std::rc::Rc<Vec<(usize, String)>>,
    /// The page's selectable text, for ⌘A, and where its reading order
    /// starts among the window's selectable text.
    document: SelectableTextDocument,
    order_base: u64,
    /// The code blocks' sideways scrolls, by block id, for their scrollbars.
    code_scrolls: CodeScrolls,
}

/// A preview's code blocks' sideways scrolls, kept between renders.
type CodeScrolls = Rc<RefCell<HashMap<usize, gpui_kit::ScrollHandle>>>;

impl Page<'_> {
    fn id(&mut self) -> usize {
        self.next_id += 1;
        self.next_id
    }

    /// The sideways scroll of the code block `id`.
    fn code_scroll(&self, id: usize) -> gpui_kit::ScrollHandle {
        self.code_scrolls
            .borrow_mut()
            .entry(id)
            .or_default()
            .clone()
    }

    /// The next place in reading order, for a block's text.
    fn order(&mut self) -> u64 {
        let id = self.id();
        document_order(self.order_base, id)
    }

    /// `text` as a selectable block of the page, in reading order.
    fn text(&mut self, text: impl Into<SharedString>) -> SelectableText {
        let id = self.id();
        SelectableText::new(("md-text", id), text)
            .document_order(document_order(self.order_base, id))
            .document(&self.document)
    }
}

/// A block's place in the window's reading order: each preview's blocks
/// in a range of their own, past the small numbers GPUI Kit's own
/// selectable text counts in, so a drag in one preview never takes in
/// another's text (or a hover's) lying between the same numbers.
fn document_order(base: u64, id: usize) -> u64 {
    base + id as u64
}

/// The key context ⌘A and ⌘C reach the preview in, once a click has given
/// it the keyboard.
const CONTEXT: &str = "MarkdownPreview";

/// What a preview keeps between renders: the focus a click in it takes,
/// its select-all, its range of reading order and its code blocks'
/// scrolls.
struct PreviewState {
    editor: gpui_kit::WeakEntity<EditorTab>,
    focus: gpui_kit::FocusHandle,
    document: SelectableTextDocument,
    order_base: u64,
    code_scrolls: CodeScrolls,
}

thread_local! {
    /// The previews' state, by editor tab.
    static PREVIEWS: RefCell<HashMap<gpui_kit::EntityId, PreviewState>> = RefCell::default();
    /// Previews made so far, for their reading-order ranges.
    static PREVIEW_COUNT: Cell<u64> = const { Cell::new(0) };
}

/// Where the `n`th preview's reading order starts (from 0): 2³² blocks
/// apart, above any order GPUI Kit counts to.
fn order_base(n: u64) -> u64 {
    (n + 1) << 32
}

/// The state of the editor tab's preview, made the first time it renders
/// (and its keys bound the first time any does).
fn preview_state(
    cx: &mut Context<EditorTab>,
) -> (
    gpui_kit::FocusHandle,
    SelectableTextDocument,
    u64,
    CodeScrolls,
) {
    static BOUND: std::sync::Once = std::sync::Once::new();
    BOUND.call_once(|| {
        let c = Some(CONTEXT);
        cx.bind_keys([
            KeyBinding::new("secondary-a", SelectAll, c),
            KeyBinding::new("secondary-c", Copy, c),
        ]);
    });
    let id = cx.entity_id();
    PREVIEWS.with(|previews| {
        let mut previews = previews.borrow_mut();
        if !previews.contains_key(&id) {
            // Closed tabs' previews go as another opens.
            previews.retain(|_, state| state.editor.upgrade().is_some());
            let n = PREVIEW_COUNT.with(|count| count.replace(count.get() + 1));
            previews.insert(
                id,
                PreviewState {
                    editor: cx.weak_entity(),
                    focus: cx.focus_handle(),
                    document: SelectableTextDocument::new(),
                    order_base: order_base(n),
                    code_scrolls: CodeScrolls::default(),
                },
            );
        }
        let state = &previews[&id];
        (
            state.focus.clone(),
            state.document.clone(),
            state.order_base,
            state.code_scrolls.clone(),
        )
    })
}

/// The preview of `blocks`, for the editor tab of a file in `folder`.
pub(crate) fn preview(
    blocks: &[Block],
    folder: &Path,
    scroll: &gpui_kit::ScrollHandle,
    cx: &mut Context<EditorTab>,
) -> AnyElement {
    let colors = cx
        .try_global::<MarkdownColors>()
        .copied()
        .unwrap_or_else(|| MarkdownColors::new(&impulse_kit::theme::get_theme("")));
    let (focus, document, order_base, code_scrolls) = preview_state(cx);
    let mut page = Page {
        colors,
        folder: folder.to_path_buf(),
        font: crate::workbench::UI_FONT,
        next_id: 0,
        editor: cx.weak_entity(),
        scroll: scroll.clone(),
        anchors: std::rc::Rc::new(markdown_blocks::heading_slugs(blocks)),
        document: document.clone(),
        order_base,
        code_scrolls,
    };
    let body = render_blocks(blocks, &mut page, cx);
    let content = div()
        .id("markdown-preview")
        // A click takes the keyboard, as the Swift preview's web view does,
        // so ⌘A selects the page and ⌘C copies from it.
        .key_context(CONTEXT)
        .track_focus(&focus)
        .on_mouse_down(gpui_kit::MouseButton::Left, move |_, window, cx| {
            window.focus(&focus, cx)
        })
        .on_action(cx.listener(move |_, _: &SelectAll, window, cx| {
            document.select_all(window, cx);
            cx.notify();
        }))
        .on_action(|_: &Copy, window, cx| {
            let text = TextSelection::selected_text(window, cx);
            if text.is_empty() {
                cx.propagate();
            } else {
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
            }
        })
        .size_full()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .flex()
        .flex_col()
        .bg(colors.bg)
        .text_color(colors.fg)
        .font_family(crate::workbench::UI_FONT)
        .text_size(px(BODY))
        .line_height(gpui_kit::relative(1.6))
        // Swift's page CSS: `padding: 24px clamp(16px, 3vw, 40px)`, from
        // the preview's width as last laid out.
        .px(px(
            (f32::from(scroll.bounds().size.width) * 0.03).clamp(16., 40.)
        ))
        .py(px(24.))
        // Each block a child of the scroll, so an anchor scrolls to its
        // heading by index.
        .children(body);
    div()
        .relative()
        .flex_1()
        .h_full()
        .min_w_0()
        .child(content)
        .vertical_scrollbar(scroll)
        .into_any_element()
}

fn render_blocks(
    blocks: &[Block],
    page: &mut Page,
    cx: &mut Context<EditorTab>,
) -> Vec<AnyElement> {
    flow_blocks(blocks, Flow::Page, page, cx)
}

/// Where blocks are laid out: their margins and paragraphs' differ.
#[derive(Clone, Copy, PartialEq)]
enum Flow {
    /// The page: the first block's top margin and the last's bottom one
    /// stay (the body's padding keeps them from collapsing).
    Page,
    /// Inside a quote: the outer margins collapse into the quote's.
    Nested,
    /// A list item: tight, its paragraphs without margins.
    Item,
}

/// A block's top and bottom margins, the page's CSS (`h1` 1.2em and
/// 0.4em of its size, `p` 0.6em, `pre`, `blockquote` and `table` 0.8em,
/// `hr` 1.5em).
fn margins(block: &Block, flow: Flow) -> (f32, f32) {
    match block {
        Block::Heading { level, .. } => {
            let size = heading_size(*level);
            (size * 1.2, size * 0.4)
        }
        Block::Paragraph(_) if flow == Flow::Item => (0., 0.),
        Block::Paragraph(_) | Block::List { .. } | Block::Image { .. } => (BODY * 0.6, BODY * 0.6),
        Block::Code { .. } | Block::Quote(_) | Block::Table { .. } => (BODY * 0.8, BODY * 0.8),
        Block::Rule => (BODY * 1.5, BODY * 1.5),
    }
}

/// `blocks`, each spaced from the one before by the larger of the two
/// margins between them (CSS's collapsing margins), not their sum.
fn flow_blocks(
    blocks: &[Block],
    flow: Flow,
    page: &mut Page,
    cx: &mut Context<EditorTab>,
) -> Vec<AnyElement> {
    let mut out = Vec::with_capacity(blocks.len());
    let mut above: Option<f32> = None;
    for (i, block) in blocks.iter().enumerate() {
        let (top, bottom) = margins(block, flow);
        let gap = match above {
            Some(above) => above.max(top),
            None if flow == Flow::Page => top,
            None => 0.,
        };
        above = Some(bottom);
        let last = i + 1 == blocks.len() && flow == Flow::Page;
        let element = match block {
            Block::Paragraph(text) if flow == Flow::Item => {
                inline_text(text, page, page.colors.fg, false, BODY, false)
            }
            other => render_block(other, page, cx),
        };
        out.push(
            div()
                .pt(px(gap))
                .when(last, |d| d.pb(px(bottom)))
                .child(element)
                .into_any_element(),
        );
    }
    out
}

fn heading_size(level: u8) -> f32 {
    BODY * match level {
        1 => 2.,
        2 => 1.5,
        3 => 1.25,
        4 => 1.,
        5 => 0.83,
        _ => 0.67,
    }
}

fn render_block(block: &Block, page: &mut Page, cx: &mut Context<EditorTab>) -> AnyElement {
    let c = page.colors;
    match block {
        Block::Heading { level, text } => {
            let size = heading_size(*level);
            div()
                .text_size(px(size))
                .line_height(gpui_kit::relative(1.3))
                .font_weight(FontWeight::BOLD)
                .text_color(c.heading)
                .when(*level <= 2, |d| {
                    d.pb(px(size * 0.3)).border_b_1().border_color(c.border)
                })
                .child(inline_text(text, page, c.heading, true, size, false))
                .into_any_element()
        }
        Block::Paragraph(text) => inline_text(text, page, c.fg, false, BODY, false),
        Block::Code { language, text } => code_block(language, text, page, cx),
        Block::Quote(blocks) => div()
            .pl(px(16.))
            .border_l(px(3.))
            .border_color(c.border)
            .text_color(c.quote)
            .children(flow_blocks(blocks, Flow::Nested, page, cx))
            .into_any_element(),
        Block::List {
            ordered,
            start,
            items,
        } => div()
            .flex()
            .flex_col()
            .children(items.iter().enumerate().map(|(n, item)| {
                let marker: AnyElement = match item.task {
                    Some(done) => div()
                        .mt(px(5.))
                        .size(px(13.))
                        .rounded(px(3.))
                        .border_1()
                        .border_color(c.border)
                        .when(done, |d| {
                            d.bg(c.link).child(crate::workbench_chrome::icon(
                                "check",
                                11.,
                                impulse_kit::chrome::Rgba::WHITE,
                            ))
                        })
                        .into_any_element(),
                    None if *ordered => div().child(format!("{}.", start + n)).into_any_element(),
                    None => div().child("\u{2022}").into_any_element(),
                };
                div()
                    .my(px(BODY * 0.2))
                    .flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_none()
                            .w(px(BODY * 2. - 8.))
                            .flex()
                            .justify_end()
                            .child(marker),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            // A tight item's paragraph has no margins.
                            .children(flow_blocks(&item.blocks, Flow::Item, page, cx)),
                    )
                    .into_any_element()
            }))
            .into_any_element(),
        Block::Table {
            aligns,
            header,
            rows,
        } => {
            let cell = |text: &Inlines, align: Align, head: bool, page: &mut Page| {
                div()
                    .flex_1()
                    .min_w_0()
                    .px(px(12.))
                    .py(px(8.))
                    .border_r_1()
                    .border_color(c.border)
                    .when(head, |d| d.bg(c.code_bg).font_weight(FontWeight::SEMIBOLD))
                    .when(align == Align::Center, |d| d.text_center())
                    .when(align == Align::Right, |d| d.text_right())
                    .child(inline_text(text, page, c.fg, head, BODY, true))
            };
            let row = |cells: &[Inlines], head: bool, page: &mut Page| {
                div().flex().border_b_1().border_color(c.border).children(
                    cells.iter().enumerate().map(|(i, text)| {
                        cell(
                            text,
                            aligns.get(i).copied().unwrap_or(Align::None),
                            head,
                            page,
                        )
                    }),
                )
            };
            let mut table = div()
                .flex()
                .flex_col()
                .border_t_1()
                .border_l_1()
                .border_color(c.border);
            if !header.is_empty() {
                table = table.child(row(header, true, page));
            }
            for cells in rows {
                table = table.child(row(cells, false, page));
            }
            table.into_any_element()
        }
        Block::Image { url, alt } => {
            let local = !url.contains("://") && !url.starts_with("data:");
            let path = page.folder.join(url);
            if local && path.is_file() {
                div()
                    .child(
                        gpui_kit::img(path)
                            .max_w_full()
                            .rounded(px(4.))
                            .object_fit(gpui_kit::ObjectFit::ScaleDown),
                    )
                    .into_any_element()
            } else {
                // Remote images aren't loaded (the Swift page's CSP allows
                // only file: and data:).
                div()
                    .text_color(c.quote)
                    .child(if alt.is_empty() {
                        url.clone()
                    } else {
                        alt.clone()
                    })
                    .into_any_element()
            }
        }
        Block::Rule => div().h(px(1.)).bg(c.border).into_any_element(),
    }
}

/// `text` as one paragraph at `size`: bold, italic, struck-through and code
/// runs, and links that open when clicked (a drag from one selects). With
/// inline code it's laid out word by word (`flowed_text`), since a text's
/// runs share one size.
fn inline_text(
    text: &Inlines,
    page: &mut Page,
    color: Hsla,
    bold: bool,
    size: f32,
    in_table: bool,
) -> AnyElement {
    if text.iter().any(|span| span.code) {
        return flowed_text(text, page, color, bold, size, in_table);
    }
    let mut string = String::new();
    let mut runs = Vec::new();
    let mut links: Vec<(Range<usize>, String)> = Vec::new();
    for span in text {
        let start = string.len();
        string.push_str(&span.text);
        runs.push(span_run(span, page, color, bold));
        if let Some(link) = &span.link {
            links.push((start..string.len(), link.clone()));
        }
    }
    let selectable = page.text(string).runs(runs);
    if links.is_empty() {
        return selectable.into_any_element();
    }
    let (ranges, targets): (Vec<_>, Vec<_>) = links.into_iter().unzip();
    let open = link_opener(page);
    selectable
        .on_click(ranges, move |index, _, cx: &mut App| {
            if let Some(url) = targets.get(index) {
                open(url, cx);
            }
        })
        .into_any_element()
}

/// A span's run (its whole text) in the page's font, or the code font.
fn span_run(span: &markdown_blocks::Span, page: &Page, color: Hsla, bold: bool) -> TextRun {
    let c = page.colors;
    let mut font = gpui_kit::font(if span.code { MONO } else { page.font });
    font.features = gpui_kit::FontFeatures::disable_ligatures();
    if span.bold || bold {
        font.weight = FontWeight::BOLD;
    }
    if span.italic {
        font.style = FontStyle::Italic;
    }
    let run_color = if span.link.is_some() { c.link } else { color };
    TextRun {
        len: span.text.len(),
        font,
        color: run_color,
        background_color: None,
        underline: None,
        strikethrough: span.strike.then_some(StrikethroughStyle {
            thickness: px(1.),
            color: Some(run_color),
        }),
    }
}

/// What a click on a link does: web and mail links open in the browser,
/// files in the editor, `#anchors` scroll to their heading.
fn link_opener(page: &Page) -> Rc<dyn Fn(&str, &mut App)> {
    let (folder, editor, scroll, anchors) = (
        page.folder.clone(),
        page.editor.clone(),
        page.scroll.clone(),
        page.anchors.clone(),
    );
    Rc::new(
        move |url, cx| match markdown_blocks::resolve_link(url, &folder) {
            markdown_blocks::LinkTarget::Web(url) => cx.open_url(&url),
            markdown_blocks::LinkTarget::Anchor(anchor) => {
                if let Some((block, _)) = anchors.iter().find(|(_, slug)| *slug == anchor) {
                    scroll.scroll_to_top_of_item(*block);
                    let _ = editor.update(cx, |_, cx| cx.notify());
                }
            }
            markdown_blocks::LinkTarget::File(path) => {
                let _ = editor.update(cx, |_, cx| {
                    cx.emit(crate::editor_git::EditorGitAction {
                        action: format!("open:{}", path.display()),
                        line: 0,
                    })
                });
            }
            // A page is never navigated elsewhere, as the Swift preview
            // keeps it.
            markdown_blocks::LinkTarget::Other => {}
        },
    )
}

/// `text` split after each run of whitespace: the places a line may break.
fn words(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_space = false;
    for (i, ch) in text.char_indices() {
        let space = ch.is_whitespace();
        if in_space && !space {
            out.push(&text[start..i]);
            start = i;
        }
        in_space = space;
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// What a piece of a flowed paragraph shows: a hard break's newline as a
/// space at the end of its line, which copies the line apart from the next
/// rather than run into it. Code keeps its text.
fn piece_text(piece: &str, code: bool) -> String {
    if code {
        piece.to_string()
    } else {
        piece.replace('\n', " ")
    }
}

/// A paragraph with inline code, word by word in a wrapping row: each word
/// with what's attached to it (a code span and the punctuation after it
/// break together), code at 0.9em of `size` in its rounded background
/// (padded 0.4em at its ends, 0.3em in a table), broken across lines the
/// way a browser breaks an inline box, padding and corners only at its
/// ends. Its pieces select and copy as one text (a `SelectableTextGroup`).
fn flowed_text(
    text: &Inlines,
    page: &mut Page,
    color: Hsla,
    bold: bool,
    size: f32,
    in_table: bool,
) -> AnyElement {
    let c = page.colors;
    let code_size = size * 0.9;
    let pad = code_size * if in_table { 0.3 } else { 0.4 };
    let open = link_opener(page);
    // Words: pieces that don't break apart (span, its text, the code
    // span's ends), and line breaks.
    enum Word {
        Pieces(Vec<(usize, String, bool, bool)>),
        Break,
    }
    let mut flow: Vec<Word> = vec![Word::Pieces(Vec::new())];
    for (i, span) in text.iter().enumerate() {
        let parts = words(&span.text);
        let count = parts.len();
        for (k, part) in parts.into_iter().enumerate() {
            if let Some(Word::Pieces(pieces)) = flow.last_mut() {
                pieces.push((i, part.to_string(), k == 0, k + 1 == count));
            }
            if part.ends_with(char::is_whitespace) {
                if part.contains('\n') && !span.code {
                    flow.push(Word::Break);
                }
                flow.push(Word::Pieces(Vec::new()));
            }
        }
    }
    let group = SelectableTextGroup::new();
    let order = page.order();
    let mut elements = Vec::with_capacity(flow.len());
    for word in flow {
        let pieces = match word {
            Word::Break => {
                elements.push(div().w_full().h(px(0.)).into_any_element());
                continue;
            }
            Word::Pieces(pieces) if pieces.is_empty() => continue,
            Word::Pieces(pieces) => pieces,
        };
        let mut row = div().flex().flex_none().items_center();
        for (i, piece, first, last) in pieces {
            let span = &text[i];
            let shown = piece_text(&piece, span.code);
            let mut run = span_run(span, page, color, bold);
            run.len = shown.len();
            let len = shown.len();
            let mut styled = SelectableText::new(("md-piece", page.id()), shown)
                .runs(vec![run])
                .group(&group)
                .document_order(order)
                .document(&page.document);
            if let Some(url) = span.link.clone() {
                let open = open.clone();
                styled = styled.on_click(vec![0..len], move |_, _, cx| open(&url, cx));
            }
            let element = if span.code {
                div()
                    .bg(c.code_bg)
                    .font_family(MONO)
                    .text_size(px(code_size))
                    .line_height(gpui_kit::relative(1.5))
                    .when(first, |d| d.pl(px(pad)).rounded_l(px(4.)))
                    .when(last, |d| d.pr(px(pad)).rounded_r(px(4.)))
                    .child(styled)
            } else {
                div().child(styled)
            };
            row = row.child(element);
        }
        elements.push(row.into_any_element());
    }
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .children(elements)
        .into_any_element()
}

/// The tree-sitter grammar for a fence's language.
fn grammar(language: &str) -> &str {
    match language {
        "sh" | "shell" | "zsh" | "bash" | "console" | "shell-session" | "terminal" => "bash",
        "ts" => "typescript",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" | "tsx" => "tsx",
        "golang" => "go",
        "c#" => "csharp",
        "html" | "htm" | "xhtml" => "html",
        "py" => "python",
        "rs" => "rust",
        "yml" => "yaml",
        "jsonc" => "json",
        "md" => "markdown",
        other => other,
    }
}

type BlockStyles = Rc<Vec<(std::ops::Range<usize>, Option<gpui_kit::Hsla>, bool)>>;

thread_local! {
    /// Highlighted code blocks by grammar, theme and text.
    static HIGHLIGHTS: RefCell<HashMap<(String, usize, SharedString), BlockStyles>> =
        RefCell::default();
}

fn code_block(
    language: &str,
    text: &str,
    page: &mut Page,
    cx: &mut Context<EditorTab>,
) -> AnyElement {
    let c = page.colors;
    let size = BODY * 0.88;
    let mut runs = Vec::new();
    let mut font = gpui_kit::font(MONO);
    font.features = gpui_kit::FontFeatures::disable_ligatures();
    let plain = |len: usize| TextRun {
        len,
        font: font.clone(),
        color: c.fg,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    // highlight.js's colors: function names colored, except a shell's
    // commands, which it leaves plain (the editor's colors).
    let theme = if markdown_blocks::is_shell(language) {
        Some(
            gpui_kit::component::ActiveTheme::theme(&**cx)
                .highlight_theme
                .clone(),
        )
    } else {
        cx.try_global::<crate::kit_theme::ReviewHighlight>()
            .map(|h| h.0.clone())
    };
    let grammar = grammar(language);
    if let (Some(theme), false) = (theme, grammar.is_empty() || text.len() > 200_000) {
        // Highlighting compiles the grammar's queries and parses the
        // block: done once per block text and theme, not every frame.
        let key = (
            grammar.to_string(),
            Arc::as_ptr(&theme) as usize,
            SharedString::from(text.to_string()),
        );
        let styles = HIGHLIGHTS.with(|cache| cache.borrow().get(&key).cloned());
        let styles = styles.unwrap_or_else(|| {
            let mut highlighter = SyntaxHighlighter::new(grammar);
            highlighter.update(None, &Rope::from(text), None);
            let styles: Rc<Vec<_>> = Rc::new(
                highlighter
                    .styles(&(0..text.len()), &*theme)
                    .into_iter()
                    .map(|(range, style)| {
                        (
                            range,
                            style.color,
                            style.font_style == Some(FontStyle::Italic),
                        )
                    })
                    .collect(),
            );
            HIGHLIGHTS.with(|cache| {
                let mut cache = cache.borrow_mut();
                if cache.len() >= 256 {
                    cache.clear();
                }
                cache.insert(key, styles.clone());
            });
            styles
        });
        let mut at = 0;
        for (range, color, italic) in styles.iter().cloned() {
            if range.start > at {
                runs.push(plain(range.start - at));
            }
            if range.end <= at {
                continue;
            }
            let start = range.start.max(at);
            let mut run = plain(range.end - start);
            if let Some(color) = color {
                run.color = color;
            }
            if italic {
                run.font.style = FontStyle::Italic;
            }
            runs.push(run);
            at = range.end;
        }
        if at < text.len() {
            runs.push(plain(text.len() - at));
        }
    } else {
        runs.push(plain(text.len()));
    }
    let shell = markdown_blocks::is_shell(language);
    let command = markdown_blocks::shell_command(text);
    let id = page.id();
    let scroll = (!shell).then(|| page.code_scroll(id));
    div()
        .relative()
        .px(px(16.))
        .py(px(14.))
        .when(shell, |d| d.pr(px(72.)))
        .rounded(px(6.))
        .border_1()
        .border_color(c.border)
        .bg(c.code_bg)
        .font_family(MONO)
        .text_size(px(size))
        // The pre's own 1.6 lines at the body size hold the code's lines.
        .line_height(px(BODY * 1.6))
        .child(
            div()
                .relative()
                .child(
                    div()
                        .id(("md-code", id))
                        // Long lines scroll sideways rather than being cut
                        // off (Swift's `pre { overflow-x: auto }`); shell
                        // blocks wrap beside their Run button. A vertical
                        // swipe stays the page's (GPUI turns it sideways
                        // otherwise).
                        .when_some(scroll.as_ref(), |d, scroll| {
                            d.whitespace_nowrap()
                                .overflow_x_scroll()
                                .restrict_scroll_to_axis()
                                .track_scroll(scroll)
                        })
                        .child(page.text(text.to_string()).runs(runs)),
                )
                .when_some(scroll.as_ref(), |d, scroll| d.horizontal_scrollbar(scroll)),
        )
        .when(shell, |d| {
            d.child(
                div()
                    .id(("md-run", id))
                    .absolute()
                    .top(px(6.))
                    .right(px(6.))
                    .px(px(8.))
                    .py(px(2.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(c.fg.opacity(0.4))
                    .bg(c.fg.opacity(0.15))
                    .font_family(crate::workbench::UI_FONT)
                    .text_size(px(12.))
                    .line_height(gpui_kit::relative(1.3))
                    .opacity(0.75)
                    .hover(|s| s.opacity(1.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(crate::editor_git::EditorGitAction {
                            action: format!("run:{command}"),
                            line: 0,
                        })
                    }))
                    .child("\u{25b6} Run"),
            )
        })
        .into_any_element()
}

impl crate::workbench::Workbench {
    /// A Run button in a preview (the Swift `runFromPreview`): in a trusted
    /// folder a plain command runs in a terminal of the preview's tab that
    /// waits at its prompt, else in a new terminal below; anything else is
    /// put in the input bar to check first, or copied when there's no
    /// input bar.
    pub(crate) fn run_from_preview(
        &mut self,
        editor: &gpui_kit::Entity<EditorTab>,
        raw: &str,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        use crate::tasks::{toast, ToastKind};
        let Some(command) = impulse_kit::markdown_preview::run_command(raw) else {
            return;
        };
        let path = editor.read(cx).path.clone();
        let directory = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(crate::workspace::home);
        let trusted = self.trust.is_trusted(&path.to_string_lossy());
        let run_now = trusted && command.is_plain;
        let note = if trusted {
            "Check the command, then press Return to run it."
        } else {
            "This folder isn't trusted: check the command, then press Return to run it."
        };
        let input_bar = crate::settings_store::SettingsStore::get(cx).terminal_context_bar;
        // A terminal at its prompt in the preview's own tab.
        let editor_id = editor.entity_id();
        let waiting = self
            .workspace()
            .tabs
            .iter()
            .find(|t| {
                t.surfaces()
                    .iter()
                    .any(|(_, s)| s.editor().is_some_and(|e| e.entity_id() == editor_id))
            })
            .and_then(|t| {
                t.surfaces().into_iter().find_map(|(_, s)| {
                    s.terminal()
                        .filter(|term| !term.read(cx).command_running && !term.read(cx).direct)
                        .cloned()
                })
            })
            .filter(|_| input_bar);
        if let Some(terminal) = waiting {
            if run_now {
                terminal.update(cx, |t, cx| {
                    t.run_command(&command.text);
                    t.focus(window, cx);
                });
            } else {
                terminal.update(cx, |t, cx| {
                    let text = command.text.clone();
                    t.editor.update(cx, |e, cx| e.set_text(&text, cx));
                    t.bar_text_changed(cx);
                    t.focus(window, cx);
                });
                toast(window, cx, ToastKind::Info, note.into(), None, None);
            }
            return;
        }
        if !run_now && !input_bar {
            // Nowhere to put it for checking first.
            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(command.text));
            return toast(
                window,
                cx,
                ToastKind::Info,
                "Copied the command: paste it into a terminal to check and run it.".into(),
                None,
                None,
            );
        }
        let commands = if run_now {
            vec![command.text.clone()]
        } else {
            Vec::new()
        };
        let surface = self.new_terminal(directory, commands, window, cx);
        if !run_now {
            if let Some(terminal) = surface.terminal() {
                let text = command.text.clone();
                terminal.update(cx, |t, cx| {
                    t.editor.update(cx, |e, cx| e.set_text(&text, cx));
                    t.bar_text_changed(cx);
                });
            }
        }
        self.split_focused(
            surface,
            impulse_kit::layout_tree::SplitAxis::Vertical,
            window,
            cx,
        );
        if !run_now {
            toast(window, cx, ToastKind::Info, note.into(), None, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_breaks_copy_as_spaces_and_code_keeps_its_text() {
        assert_eq!(piece_text("one\n", false), "one ");
        assert_eq!(piece_text("a\nb", true), "a\nb");
        assert_eq!(piece_text("word ", false), "word ");
    }

    #[test]
    fn previews_read_in_ranges_of_their_own() {
        let (first, second) = (order_base(0), order_base(1));
        // Above the small numbers GPUI Kit's own text counts in.
        assert!(first > u32::MAX as u64);
        assert!(document_order(first, 1) < document_order(first, 2));
        assert!(document_order(first, u32::MAX as usize) < document_order(second, 0));
    }

    #[test]
    fn words_break_after_whitespace() {
        assert_eq!(words("a  b\nc"), vec!["a  ", "b\n", "c"]);
    }
}
