use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// Status bar at the bottom of the window showing CWD, git branch, shell name, and cursor position.
pub struct StatusBar {
    pub widget: gtk4::Box,
    cwd_label: gtk4::Label,
    branch_label: gtk4::Label,
    #[allow(dead_code)] // Kept alive to maintain widget hierarchy
    shell_label: gtk4::Label,
    cursor_label: gtk4::Label,
    language_label: gtk4::Label,
    encoding_label: gtk4::Label,
    indent_label: gtk4::Label,
    blame_label: gtk4::Label,
    pub preview_button: gtk4::Button,
}

impl StatusBar {
    pub fn new() -> Self {
        let widget = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        widget.add_css_class("status-bar");

        let shell_label = gtk4::Label::new(Some(&impulse_core::shell::get_default_shell_name()));
        shell_label.add_css_class("shell-name");

        let branch_label = gtk4::Label::new(None);
        branch_label.add_css_class("git-branch");

        let cwd_label = gtk4::Label::new(None);
        cwd_label.add_css_class("cwd");
        cwd_label.set_hexpand(true);
        cwd_label.set_halign(gtk4::Align::Start);
        cwd_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);

        let cursor_label = gtk4::Label::new(None);
        cursor_label.add_css_class("cursor-pos");
        cursor_label.set_visible(false); // hidden by default, shown for editor tabs

        let language_label = gtk4::Label::new(None);
        language_label.add_css_class("language-name");
        language_label.set_visible(false);

        let encoding_label = gtk4::Label::new(Some("UTF-8"));
        encoding_label.add_css_class("encoding");
        encoding_label.set_visible(false);

        let indent_label = gtk4::Label::new(None);
        indent_label.add_css_class("indent-info");
        indent_label.set_visible(false);

        let blame_label = gtk4::Label::new(None);
        blame_label.add_css_class("blame-info");
        blame_label.set_visible(false);
        blame_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);

        let preview_button = gtk4::Button::with_label("Preview");
        preview_button.add_css_class("status-bar-preview-btn");
        preview_button.set_tooltip_text(Some("Toggle Markdown Preview (Ctrl+Shift+M)"));
        preview_button.set_visible(false);

        widget.append(&shell_label);
        widget.append(&branch_label);
        widget.append(&cwd_label);
        widget.append(&blame_label);
        widget.append(&encoding_label);
        widget.append(&indent_label);
        widget.append(&language_label);
        widget.append(&cursor_label);
        widget.append(&preview_button);

        StatusBar {
            widget,
            cwd_label,
            branch_label,
            shell_label,
            cursor_label,
            language_label,
            encoding_label,
            indent_label,
            blame_label,
            preview_button,
        }
    }

    pub fn update_cwd(&self, path: &str) {
        // Shorten home directory to ~
        let display_path = match impulse_core::shell::get_home_directory() {
            Ok(home) => {
                if path.starts_with(&home) {
                    format!("~{}", &path[home.len()..])
                } else {
                    path.to_string()
                }
            }
            Err(_) => path.to_string(),
        };
        self.cwd_label.set_text(&display_path);

        // Update git branch
        match impulse_core::filesystem::get_git_branch(path) {
            Ok(Some(branch)) => {
                self.branch_label.set_text(&format!(" {}", branch));
                self.branch_label.set_visible(true);
            }
            _ => {
                self.branch_label.set_visible(false);
            }
        }
    }

    pub fn update_cursor_position(&self, line: i32, col: i32) {
        self.cursor_label
            .set_text(&format!("Ln {}, Col {}", line + 1, col + 1));
        self.cursor_label.set_visible(true);
    }

    pub fn update_language(&self, lang: &str) {
        self.language_label.set_text(lang);
        self.language_label.set_visible(true);
    }

    pub fn update_encoding(&self, enc: &str) {
        self.encoding_label.set_text(enc);
        self.encoding_label.set_visible(true);
    }

    pub fn update_indent_info(&self, info: &str) {
        self.indent_label.set_text(info);
        self.indent_label.set_visible(true);
    }

    pub fn update_blame(&self, info: &str) {
        self.blame_label.set_text(info);
        self.blame_label.set_visible(true);
    }

    pub fn clear_blame(&self) {
        self.blame_label.set_visible(false);
    }

    pub fn show_preview_button(&self, is_previewing: bool) {
        if is_previewing {
            self.preview_button.add_css_class("previewing");
        } else {
            self.preview_button.remove_css_class("previewing");
        }
        self.preview_button.set_visible(true);
    }

    pub fn hide_preview_button(&self) {
        self.preview_button.set_visible(false);
        self.preview_button.remove_css_class("previewing");
    }

    pub fn hide_editor_info(&self) {
        self.language_label.set_visible(false);
        self.encoding_label.set_visible(false);
        self.cursor_label.set_visible(false);
        self.indent_label.set_visible(false);
        self.blame_label.set_visible(false);
        self.preview_button.set_visible(false);
    }
}

/// Shared status bar state that can be updated from terminal CWD change signals.
pub type SharedStatusBar = Rc<RefCell<StatusBar>>;

pub fn new_shared() -> SharedStatusBar {
    Rc::new(RefCell::new(StatusBar::new()))
}
