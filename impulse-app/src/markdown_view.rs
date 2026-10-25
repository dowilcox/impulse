//! The Markdown preview, drawn natively from `impulse_kit::markdown_blocks`
//! with the Swift preview page's styles (`markdown_preview::PAGE`): 15 pt
//! text at 1.6 lines, headings in the theme's heading color (h1 and h2
//! ruled), code in its own background with tree-sitter colors, tables with
//! a shaded header, quotes with a left rule, task lists, images from beside
//! the file. Shell blocks get a Run button, as the Swift page's
//! `run_button_script` adds. Web and mail links open in the browser,
//! links to files open in the editor and `#anchors` scroll to the heading.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::highlighter::SyntaxHighlighter;
use gpui_kit::component::input::Rope;
use gpui_kit::{
    div, prelude::*, px, AnyElement, App, Context, FontStyle, FontWeight, Hsla, InteractiveText,
    SharedString, StrikethroughStyle, StyledText, TextRun,
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
}

impl Page<'_> {
    fn id(&mut self) -> usize {
        self.next_id += 1;
        self.next_id
    }
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
    let mut page = Page {
        colors,
        folder: folder.to_path_buf(),
        font: crate::workbench::UI_FONT,
        next_id: 0,
        editor: cx.weak_entity(),
        scroll: scroll.clone(),
        anchors: std::rc::Rc::new(markdown_blocks::heading_slugs(blocks)),
    };
    let body = render_blocks(blocks, &mut page, cx);
    div()
        .id("markdown-preview")
        .flex_1()
        .h_full()
        .min_w_0()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .flex()
        .flex_col()
        .bg(colors.bg)
        .text_color(colors.fg)
        .font_family(crate::workbench::UI_FONT)
        .text_size(px(BODY))
        .line_height(gpui_kit::relative(1.6))
        .px(px(28.))
        .py(px(24.))
        // Each block a child of the scroll, so an anchor scrolls to its
        // heading by index.
        .children(body)
        .into_any_element()
}

fn render_blocks(
    blocks: &[Block],
    page: &mut Page,
    cx: &mut Context<EditorTab>,
) -> Vec<AnyElement> {
    blocks
        .iter()
        .map(|block| render_block(block, page, cx))
        .collect()
}

fn render_block(block: &Block, page: &mut Page, cx: &mut Context<EditorTab>) -> AnyElement {
    let c = page.colors;
    match block {
        Block::Heading { level, text } => {
            let size = match level {
                1 => 2.,
                2 => 1.5,
                3 => 1.25,
                4 => 1.,
                5 => 0.83,
                _ => 0.67,
            } * BODY;
            div()
                // The page's body padding doesn't swallow the first
                // heading's margin.
                .mt(px(size * 1.2))
                .mb(px(size * 0.4))
                .text_size(px(size))
                .line_height(gpui_kit::relative(1.3))
                .font_weight(FontWeight::BOLD)
                .text_color(c.heading)
                .when(*level <= 2, |d| {
                    d.pb(px(size * 0.3)).border_b_1().border_color(c.border)
                })
                .child(inline_text(text, page, c.heading, true))
                .into_any_element()
        }
        Block::Paragraph(text) => div()
            .my(px(BODY * 0.6))
            .child(inline_text(text, page, c.fg, false))
            .into_any_element(),
        Block::Code { language, text } => code_block(language, text, page, cx),
        Block::Quote(blocks) => div()
            .my(px(BODY * 0.8))
            .pl(px(16.))
            .border_l(px(3.))
            .border_color(c.border)
            .text_color(c.quote)
            .children(render_blocks(blocks, page, cx))
            .into_any_element(),
        Block::List {
            ordered,
            start,
            items,
        } => div()
            .my(px(BODY * 0.6))
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
                            .children(item.blocks.iter().map(|b| match b {
                                Block::Paragraph(text) => {
                                    inline_text(text, page, c.fg, false).into_any_element()
                                }
                                other => render_block(other, page, cx),
                            })),
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
                    .child(inline_text(text, page, c.fg, head))
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
                .my(px(BODY * 0.8))
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
                    .my(px(BODY * 0.6))
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
                    .my(px(BODY * 0.6))
                    .text_color(c.quote)
                    .child(if alt.is_empty() {
                        url.clone()
                    } else {
                        alt.clone()
                    })
                    .into_any_element()
            }
        }
        Block::Rule => div()
            .my(px(BODY * 1.5))
            .h(px(1.))
            .bg(c.border)
            .into_any_element(),
    }
}

/// `text` as one paragraph: bold, italic, struck-through and code runs,
/// and links that open when clicked.
fn inline_text(text: &Inlines, page: &mut Page, color: Hsla, bold: bool) -> AnyElement {
    let c = page.colors;
    let mut string = String::new();
    let mut runs = Vec::new();
    let mut links: Vec<(Range<usize>, String)> = Vec::new();
    // Swift's `code` padding (0.4em each side), as a space of the page's
    // font in the code's background: runs take no padding of their own.
    let pad = |string: &mut String, runs: &mut Vec<TextRun>| {
        string.push(' ');
        runs.push(TextRun {
            len: 1,
            font: gpui_kit::font(page.font),
            color,
            background_color: Some(c.code_bg),
            underline: None,
            strikethrough: None,
        });
    };
    for span in text {
        if span.code {
            pad(&mut string, &mut runs);
        }
        let start = string.len();
        string.push_str(&span.text);
        let mut font = gpui_kit::font(if span.code { MONO } else { page.font });
        font.features = gpui_kit::FontFeatures::disable_ligatures();
        if span.bold || bold {
            font.weight = FontWeight::BOLD;
        }
        if span.italic {
            font.style = FontStyle::Italic;
        }
        let run_color = if span.link.is_some() { c.link } else { color };
        runs.push(TextRun {
            len: span.text.len(),
            font,
            color: run_color,
            background_color: span.code.then_some(c.code_bg),
            underline: None,
            strikethrough: span.strike.then_some(StrikethroughStyle {
                thickness: px(1.),
                color: Some(run_color),
            }),
        });
        if let Some(link) = &span.link {
            links.push((start..string.len(), link.clone()));
        }
        if span.code {
            pad(&mut string, &mut runs);
        }
    }
    let styled = StyledText::new(SharedString::from(string)).with_runs(runs);
    if links.is_empty() {
        return styled.into_any_element();
    }
    let (ranges, targets): (Vec<_>, Vec<_>) = links.into_iter().unzip();
    let (folder, editor, scroll, anchors) = (
        page.folder.clone(),
        page.editor.clone(),
        page.scroll.clone(),
        page.anchors.clone(),
    );
    InteractiveText::new(("md-text", page.id()), styled)
        .on_click(ranges, move |index, _, cx: &mut App| {
            let Some(url) = targets.get(index) else {
                return;
            };
            match markdown_blocks::resolve_link(url, &folder) {
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
                // A page is never navigated elsewhere, as the Swift
                // preview keeps it.
                markdown_blocks::LinkTarget::Other => {}
            }
        })
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
    div()
        .relative()
        .my(px(BODY * 0.8))
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
                .when(!shell, |d| d.whitespace_nowrap())
                .child(StyledText::new(SharedString::from(text.to_string())).with_runs(runs)),
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
