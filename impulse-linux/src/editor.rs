use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::editor_webview::{self, MonacoEditorHandle};
use crate::settings::Settings;
use crate::theme::ThemeColors;
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
    HANDLES.with(|h| h.borrow_mut().remove(file_path));
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
    let contents = std::fs::read_to_string(file_path).unwrap_or_default();
    let language = guess_language(file_path);

    let (container, handle) = editor_webview::create_monaco_editor(
        file_path, &contents, &language, settings, theme, on_event,
    );

    register_handle(file_path, handle.clone());

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
        !name_str.is_empty() && name_str != "GtkBox" && name_str.contains('/')
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
// File type utilities (unchanged from original)
// ---------------------------------------------------------------------------

pub fn is_image_file(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "bmp" | "ico" | "tiff" | "tif"
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
    let ext = file_path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_lowercase();
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
        "c" | "h" => "c",
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
        "erl" | "hrl" => "erlang",
        "hs" => "haskell",
        "ml" | "mli" => "ocaml",
        "pl" | "pm" => "perl",
        "dart" => "dart",
        "vue" => "vue",
        "svelte" => "svelte",
        "dockerfile" | "Dockerfile" => "dockerfile",
        "makefile" | "Makefile" => "makefile",
        "graphql" | "gql" => "graphql",
        "tf" | "tfvars" => "terraform",
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
                "makefile" | "gnumakefile" => "makefile",
                "cmakelists.txt" => "cmake",
                ".gitignore" | ".dockerignore" => "ignore",
                ".env" | ".env.local" | ".env.example" => "dotenv",
                _ => "plaintext",
            }
        }
    }
    .to_string()
}
