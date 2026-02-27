use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;
use webkit6::prelude::*;

use crate::editor_webview::{self, MonacoEditorHandle};
use crate::settings::Settings;
use crate::theme::ThemeColors;
use impulse_editor::markdown;
use impulse_editor::protocol::EditorEvent;

// Global handle map keyed by file path.
// All Monaco editor handles are stored here so that any code path
// can look up the handle for a given file.
thread_local! {
    static HANDLES: RefCell<HashMap<String, Rc<MonacoEditorHandle>>> = RefCell::new(HashMap::new());
}

pub fn register_handle(file_path: &str, handle: Rc<MonacoEditorHandle>) {
    HANDLES.with(|h| h.borrow_mut().insert(file_path.to_string(), handle));
}

pub fn unregister_handle(file_path: &str) {
    if let Some(handle) = HANDLES.with(|h| h.borrow_mut().remove(file_path)) {
        handle.cleanup();
    }
}

pub fn get_handle(file_path: &str) -> Option<Rc<MonacoEditorHandle>> {
    HANDLES.with(|h| h.borrow().get(file_path).cloned())
}

pub fn get_handle_for_widget(widget: &gtk4::Widget) -> Option<Rc<MonacoEditorHandle>> {
    if !is_editor(widget) {
        return None;
    }
    let name = widget.widget_name();
    get_handle(name.as_str())
}

/// Maximum file size (in bytes) before the editor opens in read-only mode.
const LARGE_FILE_THRESHOLD: u64 = 5 * 1024 * 1024; // 5 MB

/// Create a Monaco editor for the given file.
///
/// The `on_event` callback receives editor events (content changes,
/// cursor moves, save requests, LSP requests, etc.).
pub fn create_editor<F>(
    file_path: &str,
    settings: &Settings,
    theme: &ThemeColors,
    on_event: F,
) -> (gtk4::Box, Rc<MonacoEditorHandle>)
where
    F: Fn(&MonacoEditorHandle, EditorEvent) + 'static,
{
    let large_file = std::fs::metadata(file_path)
        .map(|m| m.len() > LARGE_FILE_THRESHOLD)
        .unwrap_or(false);
    let contents = match std::fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to read file {}: {}", file_path, e);
            String::new()
        }
    };
    let language = guess_language(file_path);

    let (container, handle) = editor_webview::create_monaco_editor(
        file_path, &contents, &language, settings, theme, on_event,
    );

    if large_file {
        log::warn!(
            "File {} exceeds {}MB, opening in read-only mode",
            file_path,
            LARGE_FILE_THRESHOLD / (1024 * 1024)
        );
        handle.set_read_only(true);
    }

    register_handle(file_path, handle.clone());
    handle.setup_file_watcher();

    (container, handle)
}

/// Retrieve the cached text content from a Monaco editor widget.
pub fn get_editor_text(widget: &gtk4::Widget) -> Option<String> {
    let handle = get_handle_for_widget(widget)?;
    Some(handle.get_content())
}

/// Check if a widget is an editor container.
pub fn is_editor(widget: &gtk4::Widget) -> bool {
    if let Some(bx) = widget.downcast_ref::<gtk4::Box>() {
        let name = bx.widget_name();
        let name_str = name.as_str();
        !name_str.is_empty()
            && name_str != "GtkBox"
            && name_str.contains('/')
            && !bx.has_css_class("image-preview")
    } else {
        false
    }
}

/// Check if a widget is an image preview container.
pub fn is_image_preview(widget: &gtk4::Widget) -> bool {
    if let Some(bx) = widget.downcast_ref::<gtk4::Box>() {
        bx.has_css_class("image-preview")
    } else {
        false
    }
}

/// Check whether the editor has unsaved changes.
pub fn is_modified(widget: &gtk4::Widget) -> bool {
    get_handle_for_widget(widget)
        .map(|h| h.is_modified.get())
        .unwrap_or(false)
}

/// Mark the editor as unmodified (after saving).
pub fn set_unmodified(widget: &gtk4::Widget) {
    if let Some(h) = get_handle_for_widget(widget) {
        h.is_modified.set(false);
    }
}

/// Get the language name for status bar display.
pub fn get_editor_language(widget: &gtk4::Widget) -> Option<String> {
    let handle = get_handle_for_widget(widget)?;
    let lang = handle.language.borrow().clone();
    if lang.is_empty() || lang == "plaintext" {
        None
    } else {
        Some(lang)
    }
}

/// Get indentation info for status bar display.
pub fn get_editor_indent_info(widget: &gtk4::Widget) -> Option<String> {
    let handle = get_handle_for_widget(widget)?;
    let info = handle.indent_info.borrow().clone();
    Some(info)
}

/// Apply settings changes to an existing Monaco editor.
pub fn apply_settings(widget: &gtk4::Widget, settings: &Settings) {
    if let Some(handle) = get_handle_for_widget(widget) {
        handle.apply_settings(settings);
    }
}

/// Apply theme changes to an existing Monaco editor.
pub fn apply_theme(widget: &gtk4::Widget, theme: &ThemeColors) {
    if let Some(handle) = get_handle_for_widget(widget) {
        handle.set_theme(theme);
    }
}

/// Navigate to a specific position in the editor.
pub fn go_to_position(widget: &gtk4::Widget, line: u32, column: u32) {
    if let Some(handle) = get_handle_for_widget(widget) {
        handle.go_to_position(line, column);
    }
}

// ---------------------------------------------------------------------------
// File preview (markdown, SVG)
// ---------------------------------------------------------------------------

/// Check whether a file path is an SVG file.
fn is_svg_file(path: &str) -> bool {
    impulse_editor::svg::is_svg_file(path)
}

/// Check whether a file path is a previewable type (markdown or SVG).
pub fn is_previewable_file(path: &str) -> bool {
    impulse_editor::is_previewable_file(path)
}

/// Map the application theme to markdown preview colors.
pub fn theme_to_markdown_colors(theme: &ThemeColors) -> markdown::MarkdownThemeColors {
    markdown::MarkdownThemeColors {
        bg: theme.bg.to_string(),
        fg: theme.fg.to_string(),
        heading: theme.cyan.to_string(),
        link: theme.blue.to_string(),
        code_bg: theme.bg_dark.to_string(),
        border: theme.bg_highlight.to_string(),
        blockquote_fg: theme.comment.to_string(),
        hljs_keyword: theme.magenta.to_string(),
        hljs_string: theme.green.to_string(),
        hljs_number: theme.orange.to_string(),
        hljs_comment: theme.comment.to_string(),
        hljs_function: theme.blue.to_string(),
        hljs_type: theme.yellow.to_string(),
        font_family: "Inter, system-ui, sans-serif".to_string(),
        code_font_family: "'JetBrains Mono', monospace".to_string(),
    }
}

/// Toggle preview for an editor widget (markdown or SVG).
///
/// Returns the new `is_previewing` state, or `None` if the widget is not a
/// previewable file type or doesn't have a stack.
pub fn toggle_preview(widget: &gtk4::Widget, theme: &ThemeColors) -> Option<bool> {
    let handle = get_handle_for_widget(widget)?;
    let file_path = handle.file_path.borrow().clone();
    if !is_previewable_file(&file_path) {
        return None;
    }

    let stack_ref = handle.stack.borrow();
    let stack = stack_ref.as_ref()?;

    let currently_previewing = handle.is_previewing.get();
    if currently_previewing {
        // Switch back to editor
        stack.set_visible_child_name("editor");
        handle.is_previewing.set(false);
        return Some(false);
    }

    // Switch to preview: render current content
    let content = handle.get_content();

    let html = if is_svg_file(&file_path) {
        // SVG preview — embed raw SVG in themed HTML
        match impulse_editor::svg::render_svg_preview(&content, theme.bg) {
            Some(h) => h,
            None => return None,
        }
    } else {
        // Markdown preview
        let md_colors = theme_to_markdown_colors(theme);
        let hljs_path = match impulse_editor::assets::ensure_monaco_extracted() {
            Ok(dir) => format!("file://{}/highlight/highlight.min.js", dir.display()),
            Err(e) => {
                log::warn!("Failed to resolve highlight.js path: {}", e);
                String::new()
            }
        };
        match markdown::render_markdown_preview(&content, &md_colors, &hljs_path) {
            Some(h) => h,
            None => return None,
        }
    };

    // Create or reuse the preview WebView
    if stack.child_by_name("preview").is_none() {
        let preview_wv = webkit6::WebView::builder()
            .hexpand(true)
            .vexpand(true)
            .build();
        let bg_rgba =
            gtk4::gdk::RGBA::parse(theme.bg).unwrap_or(gtk4::gdk::RGBA::new(0.17, 0.14, 0.27, 1.0));
        preview_wv.set_background_color(&bg_rgba);

        if let Some(wk_settings) = webkit6::prelude::WebViewExt::settings(&preview_wv) {
            wk_settings.set_enable_javascript(true);
            // Explicitly disable file-from-file access — the base URI handles
            // relative image resolution and the CSP restricts scripts.
            wk_settings.set_allow_file_access_from_file_urls(false);
        }

        // Block navigation to external URLs; open them in the default browser.
        preview_wv.connect_decide_policy(|_wv, decision, decision_type| {
            if decision_type == webkit6::PolicyDecisionType::NavigationAction {
                if let Some(nav) = decision.downcast_ref::<webkit6::NavigationPolicyDecision>() {
                    if let Some(mut action) = nav.navigation_action() {
                        if let Some(request) = action.request() {
                            if let Some(uri) = request.uri() {
                                let scheme = uri.split(':').next().unwrap_or("");
                                if scheme != "file" && scheme != "about" && scheme != "data" {
                                    // Open in default browser and block in-WebView navigation
                                    let _ = gtk4::gio::AppInfo::launch_default_for_uri(
                                        &uri,
                                        gtk4::gio::AppLaunchContext::NONE,
                                    );
                                    decision.ignore();
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
            false // use default policy
        });

        stack.add_named(&preview_wv, Some("preview"));
    }

    // Load HTML into the preview WebView with the file's parent as base URI
    // so relative image paths resolve correctly.
    if let Some(preview_widget) = stack.child_by_name("preview") {
        if let Some(preview_wv) = preview_widget.downcast_ref::<webkit6::WebView>() {
            let base_uri = std::path::Path::new(&file_path)
                .parent()
                .map(|p| format!("file://{}/", p.display()));
            preview_wv.load_html(&html, base_uri.as_deref());
        }
    }

    stack.set_visible_child_name("preview");
    handle.is_previewing.set(true);
    Some(true)
}

/// Re-render the preview with new theme colors (for theme changes).
pub fn refresh_preview(widget: &gtk4::Widget, theme: &ThemeColors) {
    let handle = match get_handle_for_widget(widget) {
        Some(h) => h,
        None => return,
    };
    if !handle.is_previewing.get() {
        return;
    }

    let stack_ref = handle.stack.borrow();
    let stack = match stack_ref.as_ref() {
        Some(s) => s,
        None => return,
    };

    let content = handle.get_content();
    let file_path = handle.file_path.borrow().clone();

    let html = if is_svg_file(&file_path) {
        match impulse_editor::svg::render_svg_preview(&content, theme.bg) {
            Some(h) => h,
            None => return,
        }
    } else {
        let md_colors = theme_to_markdown_colors(theme);
        let hljs_path = match impulse_editor::assets::ensure_monaco_extracted() {
            Ok(dir) => format!("file://{}/highlight/highlight.min.js", dir.display()),
            Err(_) => String::new(),
        };
        match markdown::render_markdown_preview(&content, &md_colors, &hljs_path) {
            Some(h) => h,
            None => return,
        }
    };

    if let Some(preview_widget) = stack.child_by_name("preview") {
        if let Some(preview_wv) = preview_widget.downcast_ref::<webkit6::WebView>() {
            let file_path = handle.file_path.borrow().clone();
            let base_uri = std::path::Path::new(&file_path)
                .parent()
                .map(|p| format!("file://{}/", p.display()));
            preview_wv.load_html(&html, base_uri.as_deref());

            let bg_rgba = gtk4::gdk::RGBA::parse(theme.bg)
                .unwrap_or(gtk4::gdk::RGBA::new(0.17, 0.14, 0.27, 1.0));
            preview_wv.set_background_color(&bg_rgba);
        }
    }
}

// ---------------------------------------------------------------------------
// File type utilities (unchanged from original)
// ---------------------------------------------------------------------------

pub fn is_image_file(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tiff" | "tif"
    )
}

pub fn create_image_preview(file_path: &str) -> gtk4::Box {
    let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    container.set_hexpand(true);
    container.set_vexpand(true);
    container.set_widget_name(file_path);
    container.add_css_class("image-preview");

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_hexpand(true);

    let picture = gtk4::Picture::for_filename(file_path);
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk4::ContentFit::Contain);
    picture.set_margin_start(20);
    picture.set_margin_end(20);
    picture.set_margin_top(20);
    picture.set_margin_bottom(20);

    scroll.set_child(Some(&picture));
    container.append(&scroll);

    container
}

pub fn is_binary_file(path: &str) -> bool {
    if let Ok(metadata) = std::fs::metadata(path) {
        if metadata.len() > 10 * 1024 * 1024 {
            return true;
        }
    }
    if let Ok(mut file) = std::fs::File::open(path) {
        use std::io::Read;
        let mut buf = [0u8; 8192];
        if let Ok(n) = file.read(&mut buf) {
            return buf[..n].contains(&0);
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Language detection
// ---------------------------------------------------------------------------

fn guess_language(file_path: &str) -> String {
    let ext = file_path.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "jsx" => "javascript",
        "tsx" => "typescript",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "less" => "less",
        "json" | "jsonc" => "json",
        "xml" | "svg" | "xsl" | "xslt" => "xml",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" | "markdown" => "markdown",
        "sh" | "bash" | "zsh" => "shell",
        "fish" => "shell",
        "c" | "h" => "cpp",
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh" => "cpp",
        "java" => "java",
        "go" => "go",
        "rb" => "ruby",
        "php" => "php",
        "lua" => "lua",
        "sql" => "sql",
        "r" | "R" => "r",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "cs" => "csharp",
        "fs" | "fsx" => "fsharp",
        "ex" | "exs" => "elixir",
        "erl" | "hrl" => "plaintext",
        "hs" => "plaintext",
        "ml" | "mli" => "plaintext",
        "pl" | "pm" => "perl",
        "dart" => "dart",
        "m" => "objective-c",
        "scala" => "scala",
        "clj" | "cljs" | "cljc" => "clojure",
        "coffee" => "coffee",
        "pug" => "pug",
        "vue" => "plaintext",
        "svelte" => "plaintext",
        "dockerfile" | "Dockerfile" => "dockerfile",
        "graphql" | "gql" => "graphql",
        "tf" | "tfvars" => "hcl",
        "proto" => "protobuf",
        "ini" | "cfg" | "conf" => "ini",
        "bat" | "cmd" => "bat",
        "ps1" | "psm1" => "powershell",
        _ => {
            // Check filename without extension
            let filename = file_path
                .rsplit('/')
                .next()
                .unwrap_or(file_path)
                .to_lowercase();
            match filename.as_str() {
                "dockerfile" => "dockerfile",
                "makefile" | "gnumakefile" => "plaintext",
                "cmakelists.txt" => "plaintext",
                ".gitignore" | ".dockerignore" => "ini",
                ".env" | ".env.local" | ".env.example" => "ini",
                _ => "plaintext",
            }
        }
    }
    .to_string()
}
