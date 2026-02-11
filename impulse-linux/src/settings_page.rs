use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::settings::{self, CommandOnSave, FileTypeOverride, FormatOnSave, Settings};
use crate::theme;

fn override_summary(o: &FileTypeOverride) -> String {
    let mut parts = Vec::new();
    if let Some(tw) = o.tab_width {
        parts.push(format!("Tab: {tw}"));
    }
    if let Some(spaces) = o.use_spaces {
        parts.push(if spaces { "Spaces" } else { "Tabs" }.to_string());
    }
    if let Some(ref fmt) = o.format_on_save {
        parts.push(format!("Format: {}", fmt.command));
    }
    if parts.is_empty() {
        "No overrides set".to_string()
    } else {
        parts.join(", ")
    }
}

fn command_summary(c: &CommandOnSave) -> String {
    if c.command.is_empty() {
        c.file_pattern.clone()
    } else {
        format!("{} on {}", c.command, c.file_pattern)
    }
}

fn rebuild_overrides_group(
    group: &adw::PreferencesGroup,
    tracked: &Rc<RefCell<Vec<gtk4::Widget>>>,
    settings: &Rc<RefCell<Settings>>,
    on_changed: &Rc<dyn Fn(&Settings)>,
) {
    for row in tracked.borrow().iter() {
        group.remove(row);
    }
    tracked.borrow_mut().clear();

    let count = settings.borrow().file_type_overrides.len();
    for i in 0..count {
        let (pattern, tab_width_val, use_spaces_val, fmt_cmd, fmt_args, summary) = {
            let s = settings.borrow();
            let o = &s.file_type_overrides[i];
            (
                o.pattern.clone(),
                o.tab_width.unwrap_or(0) as f64,
                o.use_spaces.unwrap_or(true),
                o.format_on_save
                    .as_ref()
                    .map(|f| f.command.clone())
                    .unwrap_or_default(),
                o.format_on_save
                    .as_ref()
                    .map(|f| f.args.join(" "))
                    .unwrap_or_default(),
                override_summary(o),
            )
        };

        let expander = adw::ExpanderRow::new();
        expander.set_title(&pattern);
        expander.set_subtitle(&summary);

        let delete_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
        delete_btn.set_valign(gtk4::Align::Center);
        delete_btn.add_css_class("flat");
        {
            let group = group.clone();
            let tracked = Rc::clone(tracked);
            let settings = Rc::clone(settings);
            let on_changed = Rc::clone(on_changed);
            delete_btn.connect_clicked(move |_| {
                {
                    let mut s = settings.borrow_mut();
                    s.file_type_overrides.remove(i);
                    settings::save(&s);
                    on_changed(&s);
                }
                rebuild_overrides_group(&group, &tracked, &settings, &on_changed);
            });
        }
        expander.add_suffix(&delete_btn);

        let pattern_row = adw::EntryRow::new();
        pattern_row.set_title("Pattern");
        pattern_row.set_text(&pattern);
        {
            let settings = Rc::clone(settings);
            let on_changed = Rc::clone(on_changed);
            let expander = expander.clone();
            pattern_row.connect_changed(move |row| {
                let mut s = settings.borrow_mut();
                s.file_type_overrides[i].pattern = row.text().to_string();
                expander.set_title(&row.text());
                expander.set_subtitle(&override_summary(&s.file_type_overrides[i]));
                settings::save(&s);
                on_changed(&s);
            });
        }
        expander.add_row(&pattern_row);

        let tw_adj = gtk4::Adjustment::new(tab_width_val, 0.0, 16.0, 1.0, 1.0, 0.0);
        let tw_row = adw::SpinRow::new(Some(&tw_adj), 1.0, 0);
        tw_row.set_title("Tab Width");
        tw_row.set_subtitle("0 = use auto-detection");
        {
            let settings = Rc::clone(settings);
            let on_changed = Rc::clone(on_changed);
            let expander = expander.clone();
            tw_row.connect_value_notify(move |row| {
                let val = row.value() as u32;
                let mut s = settings.borrow_mut();
                s.file_type_overrides[i].tab_width = if val == 0 { None } else { Some(val) };
                expander.set_subtitle(&override_summary(&s.file_type_overrides[i]));
                settings::save(&s);
                on_changed(&s);
            });
        }
        expander.add_row(&tw_row);

        let spaces_row = adw::SwitchRow::new();
        spaces_row.set_title("Use Spaces");
        spaces_row.set_active(use_spaces_val);
        {
            let settings = Rc::clone(settings);
            let on_changed = Rc::clone(on_changed);
            let expander = expander.clone();
            spaces_row.connect_active_notify(move |row| {
                let mut s = settings.borrow_mut();
                s.file_type_overrides[i].use_spaces = Some(row.is_active());
                expander.set_subtitle(&override_summary(&s.file_type_overrides[i]));
                settings::save(&s);
                on_changed(&s);
            });
        }
        expander.add_row(&spaces_row);

        let fmt_cmd_row = adw::EntryRow::new();
        fmt_cmd_row.set_title("Format Command");
        fmt_cmd_row.set_text(&fmt_cmd);
        {
            let settings = Rc::clone(settings);
            let on_changed = Rc::clone(on_changed);
            let expander = expander.clone();
            fmt_cmd_row.connect_changed(move |row| {
                let cmd = row.text().to_string();
                let mut s = settings.borrow_mut();
                let o = &mut s.file_type_overrides[i];
                if cmd.is_empty() {
                    o.format_on_save = None;
                } else {
                    match o.format_on_save.as_mut() {
                        Some(f) => f.command = cmd,
                        None => {
                            o.format_on_save = Some(FormatOnSave {
                                command: cmd,
                                args: Vec::new(),
                            });
                        }
                    }
                }
                expander.set_subtitle(&override_summary(&s.file_type_overrides[i]));
                settings::save(&s);
                on_changed(&s);
            });
        }
        expander.add_row(&fmt_cmd_row);

        let fmt_args_row = adw::EntryRow::new();
        fmt_args_row.set_title("Format Arguments");
        fmt_args_row.set_text(&fmt_args);
        {
            let settings = Rc::clone(settings);
            let on_changed = Rc::clone(on_changed);
            fmt_args_row.connect_changed(move |row| {
                let args_str = row.text().to_string();
                let mut s = settings.borrow_mut();
                if let Some(ref mut f) = s.file_type_overrides[i].format_on_save {
                    f.args = args_str
                        .split_whitespace()
                        .map(String::from)
                        .collect();
                    settings::save(&s);
                    on_changed(&s);
                }
            });
        }
        expander.add_row(&fmt_args_row);

        group.add(&expander);
        tracked.borrow_mut().push(expander.upcast());
    }

    let add_row = adw::ActionRow::new();
    add_row.set_title("Add File Type Override");
    add_row.set_activatable(true);
    add_row.add_prefix(&gtk4::Image::from_icon_name("list-add-symbolic"));
    {
        let group = group.clone();
        let tracked = Rc::clone(tracked);
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(on_changed);
        add_row.connect_activated(move |_| {
            {
                let mut s = settings.borrow_mut();
                s.file_type_overrides.push(FileTypeOverride {
                    pattern: "*.ext".to_string(),
                    tab_width: None,
                    use_spaces: Some(true),
                    format_on_save: None,
                });
                settings::save(&s);
                on_changed(&s);
            }
            rebuild_overrides_group(&group, &tracked, &settings, &on_changed);
        });
    }
    group.add(&add_row);
    tracked.borrow_mut().push(add_row.upcast());
}

fn rebuild_commands_group(
    group: &adw::PreferencesGroup,
    tracked: &Rc<RefCell<Vec<gtk4::Widget>>>,
    settings: &Rc<RefCell<Settings>>,
    on_changed: &Rc<dyn Fn(&Settings)>,
) {
    for row in tracked.borrow().iter() {
        group.remove(row);
    }
    tracked.borrow_mut().clear();

    let count = settings.borrow().commands_on_save.len();
    for i in 0..count {
        let (name, command, args, file_pattern, summary) = {
            let s = settings.borrow();
            let c = &s.commands_on_save[i];
            (
                c.name.clone(),
                c.command.clone(),
                c.args.join(" "),
                c.file_pattern.clone(),
                command_summary(c),
            )
        };

        let expander = adw::ExpanderRow::new();
        expander.set_title(&name);
        expander.set_subtitle(&summary);

        let delete_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
        delete_btn.set_valign(gtk4::Align::Center);
        delete_btn.add_css_class("flat");
        {
            let group = group.clone();
            let tracked = Rc::clone(tracked);
            let settings = Rc::clone(settings);
            let on_changed = Rc::clone(on_changed);
            delete_btn.connect_clicked(move |_| {
                {
                    let mut s = settings.borrow_mut();
                    s.commands_on_save.remove(i);
                    settings::save(&s);
                    on_changed(&s);
                }
                rebuild_commands_group(&group, &tracked, &settings, &on_changed);
            });
        }
        expander.add_suffix(&delete_btn);

        let name_row = adw::EntryRow::new();
        name_row.set_title("Name");
        name_row.set_text(&name);
        {
            let settings = Rc::clone(settings);
            let on_changed = Rc::clone(on_changed);
            let expander = expander.clone();
            name_row.connect_changed(move |row| {
                let mut s = settings.borrow_mut();
                s.commands_on_save[i].name = row.text().to_string();
                expander.set_title(&row.text());
                expander.set_subtitle(&command_summary(&s.commands_on_save[i]));
                settings::save(&s);
                on_changed(&s);
            });
        }
        expander.add_row(&name_row);

        let cmd_row = adw::EntryRow::new();
        cmd_row.set_title("Command");
        cmd_row.set_text(&command);
        {
            let settings = Rc::clone(settings);
            let on_changed = Rc::clone(on_changed);
            let expander = expander.clone();
            cmd_row.connect_changed(move |row| {
                let mut s = settings.borrow_mut();
                s.commands_on_save[i].command = row.text().to_string();
                expander.set_subtitle(&command_summary(&s.commands_on_save[i]));
                settings::save(&s);
                on_changed(&s);
            });
        }
        expander.add_row(&cmd_row);

        let args_row = adw::EntryRow::new();
        args_row.set_title("Arguments");
        args_row.set_text(&args);
        {
            let settings = Rc::clone(settings);
            let on_changed = Rc::clone(on_changed);
            args_row.connect_changed(move |row| {
                let mut s = settings.borrow_mut();
                s.commands_on_save[i].args = row
                    .text()
                    .to_string()
                    .split_whitespace()
                    .map(String::from)
                    .collect();
                settings::save(&s);
                on_changed(&s);
            });
        }
        expander.add_row(&args_row);

        let pattern_row = adw::EntryRow::new();
        pattern_row.set_title("File Pattern");
        pattern_row.set_text(&file_pattern);
        {
            let settings = Rc::clone(settings);
            let on_changed = Rc::clone(on_changed);
            let expander = expander.clone();
            pattern_row.connect_changed(move |row| {
                let mut s = settings.borrow_mut();
                s.commands_on_save[i].file_pattern = row.text().to_string();
                expander.set_subtitle(&command_summary(&s.commands_on_save[i]));
                settings::save(&s);
                on_changed(&s);
            });
        }
        expander.add_row(&pattern_row);

        group.add(&expander);
        tracked.borrow_mut().push(expander.upcast());
    }

    let add_row = adw::ActionRow::new();
    add_row.set_title("Add Command on Save");
    add_row.set_activatable(true);
    add_row.add_prefix(&gtk4::Image::from_icon_name("list-add-symbolic"));
    {
        let group = group.clone();
        let tracked = Rc::clone(tracked);
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(on_changed);
        add_row.connect_activated(move |_| {
            {
                let mut s = settings.borrow_mut();
                s.commands_on_save.push(CommandOnSave {
                    name: "new command".to_string(),
                    command: String::new(),
                    args: Vec::new(),
                    file_pattern: "*".to_string(),
                });
                settings::save(&s);
                on_changed(&s);
            }
            rebuild_commands_group(&group, &tracked, &settings, &on_changed);
        });
    }
    group.add(&add_row);
    tracked.borrow_mut().push(add_row.upcast());
}

/// Creates and presents an `adw::PreferencesWindow` that allows the user to
/// edit all application settings. Changes are applied immediately, persisted
/// to disk, and forwarded to the caller via `on_settings_changed`.
pub fn show_settings_window(
    parent: &impl gtk4::prelude::IsA<gtk4::Window>,
    settings: &Rc<RefCell<Settings>>,
    on_settings_changed: impl Fn(&Settings) + 'static,
) {
    let preferences_window = adw::PreferencesWindow::new();
    preferences_window.set_transient_for(Some(parent.upcast_ref()));
    preferences_window.set_modal(true);
    preferences_window.set_search_enabled(true);
    preferences_window.set_title(Some("Settings"));

    let on_changed: Rc<dyn Fn(&Settings)> = Rc::new(on_settings_changed);

    // ── Page 1: Editor ───────────────────────────────────────────────────
    let editor_page = adw::PreferencesPage::new();
    editor_page.set_title("Editor");
    editor_page.set_icon_name(Some("text-editor-symbolic"));

    // -- Font group --
    let font_group = adw::PreferencesGroup::new();
    font_group.set_title("Font");

    let font_family_row = adw::EntryRow::new();
    font_family_row.set_title("Font Family");
    font_family_row.set_text(&settings.borrow().font_family);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        font_family_row.connect_changed(move |row| {
            let mut s = settings.borrow_mut();
            s.font_family = row.text().to_string();
            settings::save(&s);
            on_changed(&s);
        });
    }
    font_group.add(&font_family_row);

    let font_size_adj = gtk4::Adjustment::new(
        settings.borrow().font_size as f64,
        6.0,
        72.0,
        1.0,
        10.0,
        0.0,
    );
    let font_size_row = adw::SpinRow::new(Some(&font_size_adj), 1.0, 0);
    font_size_row.set_title("Font Size");
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        font_size_row.connect_value_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.font_size = row.value() as i32;
            settings::save(&s);
            on_changed(&s);
        });
    }
    font_group.add(&font_size_row);
    editor_page.add(&font_group);

    // -- Indentation group --
    let indent_group = adw::PreferencesGroup::new();
    indent_group.set_title("Indentation");

    let tab_width_adj = gtk4::Adjustment::new(
        settings.borrow().tab_width as f64,
        1.0,
        16.0,
        1.0,
        10.0,
        0.0,
    );
    let tab_width_row = adw::SpinRow::new(Some(&tab_width_adj), 1.0, 0);
    tab_width_row.set_title("Tab Width");
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        tab_width_row.connect_value_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.tab_width = row.value() as u32;
            settings::save(&s);
            on_changed(&s);
        });
    }
    indent_group.add(&tab_width_row);

    let use_spaces_row = adw::SwitchRow::new();
    use_spaces_row.set_title("Use Spaces Instead of Tabs");
    use_spaces_row.set_active(settings.borrow().use_spaces);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        use_spaces_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.use_spaces = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    indent_group.add(&use_spaces_row);
    editor_page.add(&indent_group);

    // -- Display group --
    let display_group = adw::PreferencesGroup::new();
    display_group.set_title("Display");

    let line_numbers_row = adw::SwitchRow::new();
    line_numbers_row.set_title("Show Line Numbers");
    line_numbers_row.set_active(settings.borrow().show_line_numbers);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        line_numbers_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.show_line_numbers = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    display_group.add(&line_numbers_row);

    let highlight_line_row = adw::SwitchRow::new();
    highlight_line_row.set_title("Highlight Current Line");
    highlight_line_row.set_active(settings.borrow().highlight_current_line);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        highlight_line_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.highlight_current_line = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    display_group.add(&highlight_line_row);

    let word_wrap_row = adw::SwitchRow::new();
    word_wrap_row.set_title("Word Wrap");
    word_wrap_row.set_active(settings.borrow().word_wrap);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        word_wrap_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.word_wrap = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    display_group.add(&word_wrap_row);

    let show_margin_row = adw::SwitchRow::new();
    show_margin_row.set_title("Show Right Margin");
    show_margin_row.set_active(settings.borrow().show_right_margin);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        show_margin_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.show_right_margin = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    display_group.add(&show_margin_row);

    let margin_pos_adj = gtk4::Adjustment::new(
        settings.borrow().right_margin_position as f64,
        40.0,
        200.0,
        1.0,
        10.0,
        0.0,
    );
    let margin_pos_row = adw::SpinRow::new(Some(&margin_pos_adj), 1.0, 0);
    margin_pos_row.set_title("Right Margin Column");
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        margin_pos_row.connect_value_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.right_margin_position = row.value() as u32;
            settings::save(&s);
            on_changed(&s);
        });
    }
    display_group.add(&margin_pos_row);

    let minimap_row = adw::SwitchRow::new();
    minimap_row.set_title("Minimap");
    minimap_row.set_active(settings.borrow().minimap_enabled);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        minimap_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.minimap_enabled = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    display_group.add(&minimap_row);

    let sticky_scroll_row = adw::SwitchRow::new();
    sticky_scroll_row.set_title("Sticky Scroll");
    sticky_scroll_row.set_active(settings.borrow().sticky_scroll);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        sticky_scroll_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.sticky_scroll = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    display_group.add(&sticky_scroll_row);

    let bracket_color_row = adw::SwitchRow::new();
    bracket_color_row.set_title("Bracket Pair Colorization");
    bracket_color_row.set_active(settings.borrow().bracket_pair_colorization);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        bracket_color_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.bracket_pair_colorization = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    display_group.add(&bracket_color_row);

    let indent_guides_row = adw::SwitchRow::new();
    indent_guides_row.set_title("Indentation Guides");
    indent_guides_row.set_active(settings.borrow().indent_guides);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        indent_guides_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.indent_guides = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    display_group.add(&indent_guides_row);

    let font_ligatures_row = adw::SwitchRow::new();
    font_ligatures_row.set_title("Font Ligatures");
    font_ligatures_row.set_active(settings.borrow().font_ligatures);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        font_ligatures_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.font_ligatures = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    display_group.add(&font_ligatures_row);

    let folding_row = adw::SwitchRow::new();
    folding_row.set_title("Code Folding");
    folding_row.set_active(settings.borrow().folding);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        folding_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.folding = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    display_group.add(&folding_row);

    let scroll_beyond_row = adw::SwitchRow::new();
    scroll_beyond_row.set_title("Scroll Beyond Last Line");
    scroll_beyond_row.set_active(settings.borrow().scroll_beyond_last_line);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        scroll_beyond_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.scroll_beyond_last_line = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    display_group.add(&scroll_beyond_row);

    let smooth_scrolling_row = adw::SwitchRow::new();
    smooth_scrolling_row.set_title("Smooth Scrolling");
    smooth_scrolling_row.set_active(settings.borrow().smooth_scrolling);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        smooth_scrolling_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.smooth_scrolling = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    display_group.add(&smooth_scrolling_row);

    let whitespace_labels = ["None", "Boundary", "Selection", "Trailing", "All"];
    let whitespace_values = ["none", "boundary", "selection", "trailing", "all"];
    let whitespace_model = gtk4::StringList::new(&whitespace_labels);

    let current_whitespace = settings.borrow().render_whitespace.clone();
    let whitespace_index = whitespace_values
        .iter()
        .position(|v| *v == current_whitespace)
        .unwrap_or(2) as u32;

    let whitespace_row = adw::ComboRow::new();
    whitespace_row.set_title("Render Whitespace");
    whitespace_row.set_model(Some(&whitespace_model));
    whitespace_row.set_selected(whitespace_index);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        whitespace_row.connect_selected_notify(move |row| {
            let idx = row.selected() as usize;
            if let Some(&val) = whitespace_values.get(idx) {
                let mut s = settings.borrow_mut();
                s.render_whitespace = val.to_string();
                settings::save(&s);
                on_changed(&s);
            }
        });
    }
    display_group.add(&whitespace_row);

    let line_height_adj = gtk4::Adjustment::new(
        settings.borrow().editor_line_height as f64,
        0.0,
        100.0,
        1.0,
        10.0,
        0.0,
    );
    let line_height_row = adw::SpinRow::new(Some(&line_height_adj), 1.0, 0);
    line_height_row.set_title("Line Height");
    line_height_row.set_subtitle("0 = auto");
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        line_height_row.connect_value_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.editor_line_height = row.value() as u32;
            settings::save(&s);
            on_changed(&s);
        });
    }
    display_group.add(&line_height_row);

    editor_page.add(&display_group);

    // -- Cursor group --
    let cursor_group = adw::PreferencesGroup::new();
    cursor_group.set_title("Cursor");

    let cursor_style_labels = [
        "Line",
        "Block",
        "Underline",
        "Line Thin",
        "Block Outline",
        "Underline Thin",
    ];
    let cursor_style_values = [
        "line",
        "block",
        "underline",
        "line-thin",
        "block-outline",
        "underline-thin",
    ];
    let cursor_style_model = gtk4::StringList::new(&cursor_style_labels);

    let current_cursor_style = settings.borrow().editor_cursor_style.clone();
    let cursor_style_index = cursor_style_values
        .iter()
        .position(|v| *v == current_cursor_style)
        .unwrap_or(0) as u32;

    let cursor_style_row = adw::ComboRow::new();
    cursor_style_row.set_title("Cursor Style");
    cursor_style_row.set_model(Some(&cursor_style_model));
    cursor_style_row.set_selected(cursor_style_index);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        cursor_style_row.connect_selected_notify(move |row| {
            let idx = row.selected() as usize;
            if let Some(&val) = cursor_style_values.get(idx) {
                let mut s = settings.borrow_mut();
                s.editor_cursor_style = val.to_string();
                settings::save(&s);
                on_changed(&s);
            }
        });
    }
    cursor_group.add(&cursor_style_row);

    let cursor_blink_labels = ["Blink", "Smooth", "Phase", "Expand", "Solid"];
    let cursor_blink_values = ["blink", "smooth", "phase", "expand", "solid"];
    let cursor_blink_model = gtk4::StringList::new(&cursor_blink_labels);

    let current_cursor_blink = settings.borrow().editor_cursor_blinking.clone();
    let cursor_blink_index = cursor_blink_values
        .iter()
        .position(|v| *v == current_cursor_blink)
        .unwrap_or(1) as u32;

    let editor_cursor_blink_row = adw::ComboRow::new();
    editor_cursor_blink_row.set_title("Cursor Blinking");
    editor_cursor_blink_row.set_model(Some(&cursor_blink_model));
    editor_cursor_blink_row.set_selected(cursor_blink_index);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        editor_cursor_blink_row.connect_selected_notify(move |row| {
            let idx = row.selected() as usize;
            if let Some(&val) = cursor_blink_values.get(idx) {
                let mut s = settings.borrow_mut();
                s.editor_cursor_blinking = val.to_string();
                settings::save(&s);
                on_changed(&s);
            }
        });
    }
    cursor_group.add(&editor_cursor_blink_row);

    editor_page.add(&cursor_group);

    // -- Behavior group --
    let behavior_group = adw::PreferencesGroup::new();
    behavior_group.set_title("Behavior");

    let auto_save_row = adw::SwitchRow::new();
    auto_save_row.set_title("Auto Save");
    auto_save_row.set_active(settings.borrow().auto_save);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        auto_save_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.auto_save = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    behavior_group.add(&auto_save_row);

    let auto_close_labels = ["Always", "Language Defined", "Before Whitespace", "Never"];
    let auto_close_values = ["always", "languageDefined", "beforeWhitespace", "never"];
    let auto_close_model = gtk4::StringList::new(&auto_close_labels);

    let current_auto_close = settings.borrow().editor_auto_closing_brackets.clone();
    let auto_close_index = auto_close_values
        .iter()
        .position(|v| *v == current_auto_close)
        .unwrap_or(1) as u32;

    let auto_close_row = adw::ComboRow::new();
    auto_close_row.set_title("Auto-Close Brackets");
    auto_close_row.set_model(Some(&auto_close_model));
    auto_close_row.set_selected(auto_close_index);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        auto_close_row.connect_selected_notify(move |row| {
            let idx = row.selected() as usize;
            if let Some(&val) = auto_close_values.get(idx) {
                let mut s = settings.borrow_mut();
                s.editor_auto_closing_brackets = val.to_string();
                settings::save(&s);
                on_changed(&s);
            }
        });
    }
    behavior_group.add(&auto_close_row);

    editor_page.add(&behavior_group);

    preferences_window.add(&editor_page);

    // ── Page 2: Terminal ─────────────────────────────────────────────────
    let terminal_page = adw::PreferencesPage::new();
    terminal_page.set_title("Terminal");
    terminal_page.set_icon_name(Some("utilities-terminal-symbolic"));

    // -- Font group --
    let term_font_group = adw::PreferencesGroup::new();
    term_font_group.set_title("Font");

    let term_font_row = adw::EntryRow::new();
    term_font_row.set_title("Font Family");
    term_font_row.set_text(&settings.borrow().terminal_font_family);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        term_font_row.connect_changed(move |row| {
            let mut s = settings.borrow_mut();
            s.terminal_font_family = row.text().to_string();
            settings::save(&s);
            on_changed(&s);
        });
    }
    term_font_group.add(&term_font_row);

    let term_font_size_adj = gtk4::Adjustment::new(
        settings.borrow().terminal_font_size as f64,
        6.0,
        72.0,
        1.0,
        10.0,
        0.0,
    );
    let term_font_size_row = adw::SpinRow::new(Some(&term_font_size_adj), 1.0, 0);
    term_font_size_row.set_title("Font Size");
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        term_font_size_row.connect_value_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.terminal_font_size = row.value() as i32;
            settings::save(&s);
            on_changed(&s);
        });
    }
    term_font_group.add(&term_font_size_row);
    terminal_page.add(&term_font_group);

    // -- Appearance group --
    let term_appearance_group = adw::PreferencesGroup::new();
    term_appearance_group.set_title("Appearance");

    let cursor_shape_labels = ["Block", "IBeam", "Underline"];
    let cursor_shape_values = ["block", "ibeam", "underline"];
    let cursor_model = gtk4::StringList::new(&cursor_shape_labels);

    let current_cursor = settings.borrow().terminal_cursor_shape.clone();
    let cursor_index = cursor_shape_values
        .iter()
        .position(|v| *v == current_cursor)
        .unwrap_or(0) as u32;

    let cursor_row = adw::ComboRow::new();
    cursor_row.set_title("Cursor Shape");
    cursor_row.set_model(Some(&cursor_model));
    cursor_row.set_selected(cursor_index);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        cursor_row.connect_selected_notify(move |row| {
            let idx = row.selected() as usize;
            if let Some(&val) = cursor_shape_values.get(idx) {
                let mut s = settings.borrow_mut();
                s.terminal_cursor_shape = val.to_string();
                settings::save(&s);
                on_changed(&s);
            }
        });
    }
    term_appearance_group.add(&cursor_row);

    let cursor_blink_row = adw::SwitchRow::new();
    cursor_blink_row.set_title("Cursor Blink");
    cursor_blink_row.set_active(settings.borrow().terminal_cursor_blink);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        cursor_blink_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.terminal_cursor_blink = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    term_appearance_group.add(&cursor_blink_row);

    let bell_row = adw::SwitchRow::new();
    bell_row.set_title("Audible Bell");
    bell_row.set_active(settings.borrow().terminal_bell);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        bell_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.terminal_bell = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    term_appearance_group.add(&bell_row);
    terminal_page.add(&term_appearance_group);

    // -- Behavior group --
    let term_behavior_group = adw::PreferencesGroup::new();
    term_behavior_group.set_title("Behavior");

    let copy_on_select_row = adw::SwitchRow::new();
    copy_on_select_row.set_title("Copy on Select");
    copy_on_select_row.set_subtitle("Copy selected text to clipboard automatically");
    copy_on_select_row.set_active(settings.borrow().terminal_copy_on_select);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        copy_on_select_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.terminal_copy_on_select = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    term_behavior_group.add(&copy_on_select_row);

    let scroll_on_output_row = adw::SwitchRow::new();
    scroll_on_output_row.set_title("Scroll on Output");
    scroll_on_output_row.set_subtitle("Auto-scroll when new output appears");
    scroll_on_output_row.set_active(settings.borrow().terminal_scroll_on_output);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        scroll_on_output_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.terminal_scroll_on_output = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    term_behavior_group.add(&scroll_on_output_row);

    let allow_hyperlink_row = adw::SwitchRow::new();
    allow_hyperlink_row.set_title("Clickable Links");
    allow_hyperlink_row.set_subtitle("Make URLs in terminal output clickable");
    allow_hyperlink_row.set_active(settings.borrow().terminal_allow_hyperlink);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        allow_hyperlink_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.terminal_allow_hyperlink = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    term_behavior_group.add(&allow_hyperlink_row);

    let bold_is_bright_row = adw::SwitchRow::new();
    bold_is_bright_row.set_title("Bold is Bright");
    bold_is_bright_row.set_subtitle("Map bold text to bright color variants");
    bold_is_bright_row.set_active(settings.borrow().terminal_bold_is_bright);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        bold_is_bright_row.connect_active_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.terminal_bold_is_bright = row.is_active();
            settings::save(&s);
            on_changed(&s);
        });
    }
    term_behavior_group.add(&bold_is_bright_row);
    terminal_page.add(&term_behavior_group);

    // -- Scrollback group --
    let scrollback_group = adw::PreferencesGroup::new();
    scrollback_group.set_title("Scrollback");

    let scrollback_adj = gtk4::Adjustment::new(
        settings.borrow().terminal_scrollback as f64,
        100.0,
        1_000_000.0,
        1000.0,
        10.0,
        0.0,
    );
    let scrollback_row = adw::SpinRow::new(Some(&scrollback_adj), 1.0, 0);
    scrollback_row.set_title("Scrollback Lines");
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        scrollback_row.connect_value_notify(move |row| {
            let mut s = settings.borrow_mut();
            s.terminal_scrollback = row.value() as i64;
            settings::save(&s);
            on_changed(&s);
        });
    }
    scrollback_group.add(&scrollback_row);
    terminal_page.add(&scrollback_group);

    preferences_window.add(&terminal_page);

    // ── Page 3: Appearance ───────────────────────────────────────────────
    let appearance_page = adw::PreferencesPage::new();
    appearance_page.set_title("Appearance");
    appearance_page.set_icon_name(Some("applications-graphics-symbolic"));

    let theme_group = adw::PreferencesGroup::new();
    theme_group.set_title("Theme");

    let theme_labels = [
        "Kanagawa",
        "Rosé Pine",
        "Nord",
        "Gruvbox",
        "Tokyo Night",
        "Catppuccin Mocha",
    ];
    let available_themes = theme::get_available_themes();
    let theme_model = gtk4::StringList::new(&theme_labels);

    let current_theme = settings.borrow().color_scheme.clone();
    let theme_index = available_themes
        .iter()
        .position(|t| *t == current_theme)
        .unwrap_or(0) as u32;

    let theme_row = adw::ComboRow::new();
    theme_row.set_title("Color Scheme");
    theme_row.set_model(Some(&theme_model));
    theme_row.set_selected(theme_index);
    {
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(&on_changed);
        theme_row.connect_selected_notify(move |row| {
            let idx = row.selected() as usize;
            if let Some(&val) = available_themes.get(idx) {
                let mut s = settings.borrow_mut();
                s.color_scheme = val.to_string();
                settings::save(&s);
                on_changed(&s);
            }
        });
    }
    theme_group.add(&theme_row);
    appearance_page.add(&theme_group);

    preferences_window.add(&appearance_page);

    // ── Page 4: Automation ──────────────────────────────────────────────
    let automation_page = adw::PreferencesPage::new();
    automation_page.set_title("Automation");
    automation_page.set_icon_name(Some("system-run-symbolic"));

    let overrides_group = adw::PreferencesGroup::new();
    overrides_group.set_title("File Type Overrides");
    overrides_group.set_description(Some(
        "Per-file-type editor settings and format-on-save commands",
    ));
    let tracked_overrides: Rc<RefCell<Vec<gtk4::Widget>>> = Rc::new(RefCell::new(Vec::new()));
    rebuild_overrides_group(&overrides_group, &tracked_overrides, settings, &on_changed);
    automation_page.add(&overrides_group);

    let commands_group = adw::PreferencesGroup::new();
    commands_group.set_title("Commands on Save");
    commands_group.set_description(Some(
        "Shell commands that run after saving matching files",
    ));
    let tracked_commands: Rc<RefCell<Vec<gtk4::Widget>>> = Rc::new(RefCell::new(Vec::new()));
    rebuild_commands_group(&commands_group, &tracked_commands, settings, &on_changed);
    automation_page.add(&commands_group);

    preferences_window.add(&automation_page);

    preferences_window.present();
}
