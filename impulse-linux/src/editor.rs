use gtk4::glib;
use gtk4::prelude::*;
use sourceview5::prelude::*;

/// Detect indentation style from file content.
/// Returns (use_spaces, indent_width).
fn detect_indentation(content: &str) -> (bool, u32) {
    let mut space_lines = 0;
    let mut tab_lines = 0;
    let mut indent_widths = std::collections::HashMap::new();

    for line in content.lines().take(100) {
        // Sample first 100 lines
        if line.starts_with('\t') {
            tab_lines += 1;
        } else if line.starts_with(' ') {
            space_lines += 1;
            // Count leading spaces
            let spaces = line.len() - line.trim_start_matches(' ').len();
            if spaces >= 2 {
                // Record possible indent widths
                for width in &[2u32, 4, 8] {
                    if spaces % (*width as usize) == 0 {
                        *indent_widths.entry(*width).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    let use_spaces = space_lines >= tab_lines;
    let indent_width = if use_spaces {
        // Pick the most common indent width
        indent_widths
            .into_iter()
            .max_by_key(|&(_, count)| count)
            .map(|(width, _)| width)
            .unwrap_or(4)
    } else {
        4 // tab width display
    };

    (use_spaces, indent_width)
}

/// Create an editor widget for the given file path.
/// Returns the top-level widget (a Box containing a scrolled SourceView) and the
/// underlying sourceview5::Buffer so the caller can connect modification signals.
pub fn create_editor(
    file_path: &str,
    settings: &crate::settings::Settings,
) -> (gtk4::Box, sourceview5::Buffer) {
    let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    container.set_hexpand(true);
    container.set_vexpand(true);

    // Store the file path on the widget for identification
    container.set_widget_name(file_path);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_hexpand(true);

    let buffer = sourceview5::Buffer::new(None);

    // Load file contents
    let contents = std::fs::read_to_string(file_path).ok();
    if let Some(ref text) = contents {
        buffer.set_text(text);
    }

    // Set up syntax highlighting based on file extension
    let lang_manager = sourceview5::LanguageManager::default();
    if let Some(language) = lang_manager.guess_language(Some(file_path), None) {
        buffer.set_language(Some(&language));
    }

    // Use a dark color scheme — try user preference first, then fallbacks
    let scheme_manager = sourceview5::StyleSchemeManager::default();
    let mut scheme_set = false;
    if !settings.editor_color_scheme.is_empty() {
        if let Some(scheme) = scheme_manager.scheme(&settings.editor_color_scheme) {
            buffer.set_style_scheme(Some(&scheme));
            scheme_set = true;
        }
    }
    if !scheme_set {
        for scheme_name in &["Adwaita-dark", "classic-dark", "oblivion", "cobalt"] {
            if let Some(scheme) = scheme_manager.scheme(scheme_name) {
                buffer.set_style_scheme(Some(&scheme));
                break;
            }
        }
    }

    buffer.set_highlight_syntax(true);
    buffer.set_highlight_matching_brackets(true);

    // Mark buffer as unmodified after loading file contents
    buffer.set_modified(false);

    let view = sourceview5::View::with_buffer(&buffer);
    view.set_show_line_numbers(settings.show_line_numbers);
    view.set_highlight_current_line(settings.highlight_current_line);
    // Detect and apply indentation style from file content, falling back to settings
    if let Some(ref text) = contents {
        let (use_spaces, indent_width) = detect_indentation(text);
        view.set_insert_spaces_instead_of_tabs(use_spaces);
        view.set_tab_width(indent_width);
        view.set_indent_width(indent_width as i32);
    } else {
        view.set_tab_width(settings.tab_width);
        view.set_insert_spaces_instead_of_tabs(settings.use_spaces);
        view.set_indent_width(settings.tab_width as i32);
    }
    view.set_auto_indent(true);
    view.set_monospace(true);
    view.set_show_right_margin(settings.show_right_margin);
    view.set_right_margin_position(settings.right_margin_position);
    if settings.word_wrap {
        view.set_wrap_mode(gtk4::WrapMode::Word);
    } else {
        view.set_wrap_mode(gtk4::WrapMode::None);
    }
    view.set_smart_home_end(sourceview5::SmartHomeEndType::Before);
    view.set_indent_on_tab(true);
    view.set_smart_backspace(true);

    // Show whitespace characters
    let drawer = view.space_drawer();
    drawer.set_enable_matrix(true);
    // Show leading and trailing spaces/tabs
    let matrix = sourceview5::SpaceTypeFlags::SPACE | sourceview5::SpaceTypeFlags::TAB;
    drawer.set_types_for_locations(sourceview5::SpaceLocationFlags::LEADING, matrix);
    drawer.set_types_for_locations(sourceview5::SpaceLocationFlags::TRAILING, matrix);

    // Set up line mark categories for future LSP diagnostics
    view.set_show_line_marks(true);

    let error_attrs = sourceview5::MarkAttributes::new();
    error_attrs.set_icon_name("dialog-error-symbolic");
    view.set_mark_attributes("error", &error_attrs, 100);

    let warning_attrs = sourceview5::MarkAttributes::new();
    warning_attrs.set_icon_name("dialog-warning-symbolic");
    view.set_mark_attributes("warning", &warning_attrs, 90);

    let info_attrs = sourceview5::MarkAttributes::new();
    info_attrs.set_icon_name("dialog-information-symbolic");
    view.set_mark_attributes("info", &info_attrs, 80);

    // Bracket auto-close
    {
        let view_clone = view.clone();
        buffer.connect_insert_text(move |buf, location, text| {
            if text.len() != 1 {
                return;
            }
            let closing = match text {
                "(" => Some(")"),
                "[" => Some("]"),
                "{" => Some("}"),
                "\"" => Some("\""),
                "'" => Some("'"),
                "`" => Some("`"),
                _ => None,
            };
            if let Some(close_char) = closing {
                // Check if next char is already the closing char (to avoid doubling)
                let next_iter = *location;
                if next_iter.char() == close_char.chars().next().unwrap() {
                    return;
                }
                // Insert closing char at the cursor position after the opening char
                let offset = location.offset();
                glib::idle_add_local_once({
                    let buf = buf.clone();
                    let view = view_clone.clone();
                    let close = close_char.to_string();
                    move || {
                        let mut iter = buf.iter_at_offset(offset);
                        buf.insert(&mut iter, &close);
                        // Move cursor back between the brackets
                        let cursor = buf.iter_at_offset(offset);
                        buf.place_cursor(&cursor);
                        view.scroll_mark_onscreen(&buf.get_insert());
                    }
                });
            }
        });
    }

    scroll.set_child(Some(&view));

    // Add minimap
    let map = sourceview5::Map::new();
    map.set_view(&view);
    let mut map_font = gtk4::pango::FontDescription::new();
    map_font.set_family("monospace");
    map_font.set_size(gtk4::pango::SCALE); // tiny font for overview
    map.set_font_desc(Some(&map_font));
    map.set_width_request(100);

    let editor_hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    editor_hbox.set_hexpand(true);
    editor_hbox.set_vexpand(true);
    editor_hbox.append(&scroll);
    editor_hbox.append(&map);

    container.append(&editor_hbox);

    (container, buffer)
}

/// Walk the widget tree to find the sourceview5::Buffer within an editor container.
/// Used by the save handler to reset the modified flag after saving.
pub fn get_editor_buffer(widget: &gtk4::Widget) -> Option<sourceview5::Buffer> {
    if let Some(view) = widget.downcast_ref::<sourceview5::View>() {
        return view.buffer().downcast::<sourceview5::Buffer>().ok();
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(buf) = get_editor_buffer(&c) {
            return Some(buf);
        }
        child = c.next_sibling();
    }
    None
}

/// Retrieve the text content from an editor widget tree by walking children
/// to find a sourceview5::View and extracting its buffer text.
pub fn get_editor_text(widget: &gtk4::Widget) -> Option<String> {
    if let Some(view) = widget.downcast_ref::<sourceview5::View>() {
        let buffer = view.buffer();
        let start = buffer.start_iter();
        let end = buffer.end_iter();
        return Some(buffer.text(&start, &end, true).to_string());
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(text) = get_editor_text(&c) {
            return Some(text);
        }
        child = c.next_sibling();
    }
    None
}

/// Walk the widget tree to find the sourceview5::View within an editor container.
/// Used by the editor search bar to access the view for cursor positioning.
pub fn get_editor_view(widget: &gtk4::Widget) -> Option<sourceview5::View> {
    if let Some(view) = widget.downcast_ref::<sourceview5::View>() {
        return Some(view.clone());
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(view) = get_editor_view(&c) {
            return Some(view);
        }
        child = c.next_sibling();
    }
    None
}

/// Get the language name from an editor widget's buffer.
pub fn get_editor_language(widget: &gtk4::Widget) -> Option<String> {
    let buf = get_editor_buffer(widget)?;
    let lang = buf.language()?;
    Some(lang.name().to_string())
}

/// Check if a widget is an editor container (as opposed to a terminal container).
/// Editor containers are Box widgets with a file path stored in widget_name.
pub fn is_editor(widget: &gtk4::Widget) -> bool {
    if let Some(bx) = widget.downcast_ref::<gtk4::Box>() {
        let name = bx.widget_name();
        let name_str = name.as_str();
        !name_str.is_empty() && name_str != "GtkBox" && name_str.contains('/')
    } else {
        false
    }
}

/// Get indentation info for display in the status bar.
/// Returns a string like "Spaces: 4" or "Tab Size: 4".
pub fn get_editor_indent_info(widget: &gtk4::Widget) -> Option<String> {
    if let Some(view) = get_editor_view(widget) {
        let spaces = view.is_insert_spaces_instead_of_tabs();
        let width = view.tab_width();
        if spaces {
            Some(format!("Spaces: {}", width))
        } else {
            Some(format!("Tab Size: {}", width))
        }
    } else {
        None
    }
}

/// Check if a file path refers to an image based on its extension.
pub fn is_image_file(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "bmp" | "ico" | "tiff" | "tif"
    )
}

/// Create an image preview widget for the given file path.
/// Returns a Box containing a scrollable picture view.
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
    // Add some margin around the image
    picture.set_margin_start(20);
    picture.set_margin_end(20);
    picture.set_margin_top(20);
    picture.set_margin_bottom(20);

    scroll.set_child(Some(&picture));
    container.append(&scroll);

    container
}

/// Check if a file appears to be binary (contains null bytes) or is too large to edit.
pub fn is_binary_file(path: &str) -> bool {
    // Check file size first
    if let Ok(metadata) = std::fs::metadata(path) {
        if metadata.len() > 10 * 1024 * 1024 {
            return true;
        }
    }

    // Check first 8KB for null bytes (binary indicator)
    if let Ok(mut file) = std::fs::File::open(path) {
        use std::io::Read;
        let mut buf = [0u8; 8192];
        if let Ok(n) = file.read(&mut buf) {
            return buf[..n].contains(&0);
        }
    }
    false
}
