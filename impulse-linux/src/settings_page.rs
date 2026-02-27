use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::keybindings;
use crate::settings::{self, CommandOnSave, CustomKeybinding, FileTypeOverride, Settings};
use crate::theme;

fn override_summary(o: &FileTypeOverride) -> String {
    let mut parts = Vec::new();
    if let Some(tw) = o.tab_width {
        parts.push(format!("Tab: {tw}"));
    }
    if let Some(spaces) = o.use_spaces {
        parts.push(if spaces { "Spaces" } else { "Tabs" }.to_string());
    }
    if parts.is_empty() {
        "No overrides set".to_string()
    } else {
        parts.join(", ")
    }
}

fn command_summary(c: &CommandOnSave) -> String {
    let suffix = if c.reload_file { " (formatter)" } else { "" };
    if c.command.is_empty() {
        format!("{}{}", c.file_pattern, suffix)
    } else {
        format!("{} on {}{}", c.command, c.file_pattern, suffix)
    }
}

fn rebuild_overrides_group(
    group: &adw::PreferencesGroup,
    tracked: &Rc<RefCell<Vec<gtk4::Widget>>>,
    settings: &Rc<RefCell<Settings>>,
    on_changed: &Rc<dyn Fn(&Settings)>,
    generation: &Rc<Cell<u64>>,
) {
    // Increment generation so stale closures from previous rebuilds become no-ops
    generation.set(generation.get() + 1);
    let gen = generation.get();

    for row in tracked.borrow().iter() {
        group.remove(row);
    }
    tracked.borrow_mut().clear();

    let count = settings.borrow().file_type_overrides.len();
    for i in 0..count {
        let (pattern, tab_width_val, use_spaces_val, summary) = {
            let s = settings.borrow();
            let o = &s.file_type_overrides[i];
            (
                o.pattern.clone(),
                o.tab_width.unwrap_or(0) as f64,
                o.use_spaces.unwrap_or(true),
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
            let generation = Rc::clone(generation);
            delete_btn.connect_clicked(move |_| {
                {
                    let mut s = settings.borrow_mut();
                    if i >= s.file_type_overrides.len() {
                        return;
                    }
                    s.file_type_overrides.remove(i);
                    settings::save(&s);
                    on_changed(&s);
                }
                rebuild_overrides_group(&group, &tracked, &settings, &on_changed, &generation);
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
            let generation = Rc::clone(generation);
            pattern_row.connect_changed(move |row| {
                if generation.get() != gen {
                    return;
                }
                let mut s = settings.borrow_mut();
                if i >= s.file_type_overrides.len() {
                    return;
                }
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
            let generation = Rc::clone(generation);
            tw_row.connect_value_notify(move |row| {
                if generation.get() != gen {
                    return;
                }
                let val = row.value() as u32;
                let mut s = settings.borrow_mut();
                if i >= s.file_type_overrides.len() {
                    return;
                }
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
            let generation = Rc::clone(generation);
            spaces_row.connect_active_notify(move |row| {
                if generation.get() != gen {
                    return;
                }
                let mut s = settings.borrow_mut();
                if i >= s.file_type_overrides.len() {
                    return;
                }
                s.file_type_overrides[i].use_spaces = Some(row.is_active());
                expander.set_subtitle(&override_summary(&s.file_type_overrides[i]));
                settings::save(&s);
                on_changed(&s);
            });
        }
        expander.add_row(&spaces_row);

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
        let generation = Rc::clone(generation);
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
            rebuild_overrides_group(&group, &tracked, &settings, &on_changed, &generation);
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
    generation: &Rc<Cell<u64>>,
) {
    // Increment generation so stale closures from previous rebuilds become no-ops
    generation.set(generation.get() + 1);
    let gen = generation.get();

    for row in tracked.borrow().iter() {
        group.remove(row);
    }
    tracked.borrow_mut().clear();

    let count = settings.borrow().commands_on_save.len();
    for i in 0..count {
        let (name, command, args, file_pattern, reload_file, summary) = {
            let s = settings.borrow();
            let c = &s.commands_on_save[i];
            (
                c.name.clone(),
                c.command.clone(),
                c.args.join(" "),
                c.file_pattern.clone(),
                c.reload_file,
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
            let generation = Rc::clone(generation);
            delete_btn.connect_clicked(move |_| {
                {
                    let mut s = settings.borrow_mut();
                    if i >= s.commands_on_save.len() {
                        return;
                    }
                    s.commands_on_save.remove(i);
                    settings::save(&s);
                    on_changed(&s);
                }
                rebuild_commands_group(&group, &tracked, &settings, &on_changed, &generation);
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
            let generation = Rc::clone(generation);
            name_row.connect_changed(move |row| {
                if generation.get() != gen {
                    return;
                }
                let mut s = settings.borrow_mut();
                if i >= s.commands_on_save.len() {
                    return;
                }
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
            let generation = Rc::clone(generation);
            cmd_row.connect_changed(move |row| {
                if generation.get() != gen {
                    return;
                }
                let mut s = settings.borrow_mut();
                if i >= s.commands_on_save.len() {
                    return;
                }
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
            let generation = Rc::clone(generation);
            args_row.connect_changed(move |row| {
                if generation.get() != gen {
                    return;
                }
                let mut s = settings.borrow_mut();
                if i >= s.commands_on_save.len() {
                    return;
                }
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
            let generation = Rc::clone(generation);
            pattern_row.connect_changed(move |row| {
                if generation.get() != gen {
                    return;
                }
                let mut s = settings.borrow_mut();
                if i >= s.commands_on_save.len() {
                    return;
                }
                s.commands_on_save[i].file_pattern = row.text().to_string();
                expander.set_subtitle(&command_summary(&s.commands_on_save[i]));
                settings::save(&s);
                on_changed(&s);
            });
        }
        expander.add_row(&pattern_row);

        let reload_row = adw::SwitchRow::new();
        reload_row.set_title("Reload File After Run");
        reload_row.set_subtitle("Reload the editor buffer after the command succeeds");
        reload_row.set_active(reload_file);
        {
            let settings = Rc::clone(settings);
            let on_changed = Rc::clone(on_changed);
            let expander = expander.clone();
            let generation = Rc::clone(generation);
            reload_row.connect_active_notify(move |row| {
                if generation.get() != gen {
                    return;
                }
                let mut s = settings.borrow_mut();
                if i >= s.commands_on_save.len() {
                    return;
                }
                s.commands_on_save[i].reload_file = row.is_active();
                expander.set_subtitle(&command_summary(&s.commands_on_save[i]));
                settings::save(&s);
                on_changed(&s);
            });
        }
        expander.add_row(&reload_row);

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
        let generation = Rc::clone(generation);
        add_row.connect_activated(move |_| {
            {
                let mut s = settings.borrow_mut();
                s.commands_on_save.push(CommandOnSave {
                    name: "new command".to_string(),
                    command: String::new(),
                    args: Vec::new(),
                    file_pattern: "*".to_string(),
                    reload_file: false,
                });
                settings::save(&s);
                on_changed(&s);
            }
            rebuild_commands_group(&group, &tracked, &settings, &on_changed, &generation);
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

    let available_themes = theme::get_available_themes();
    let theme_labels: Vec<String> = available_themes
        .iter()
        .map(|id| theme::theme_display_name(id))
        .collect();
    let theme_label_refs: Vec<&str> = theme_labels.iter().map(|s| s.as_str()).collect();
    let theme_model = gtk4::StringList::new(&theme_label_refs);

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
    overrides_group.set_description(Some("Per-file-type indentation settings"));
    let tracked_overrides: Rc<RefCell<Vec<gtk4::Widget>>> = Rc::new(RefCell::new(Vec::new()));
    let overrides_generation: Rc<Cell<u64>> = Rc::new(Cell::new(0));
    rebuild_overrides_group(
        &overrides_group,
        &tracked_overrides,
        settings,
        &on_changed,
        &overrides_generation,
    );
    automation_page.add(&overrides_group);

    let commands_group = adw::PreferencesGroup::new();
    commands_group.set_title("Commands on Save");
    commands_group.set_description(Some("Shell commands that run after saving matching files"));
    let tracked_commands: Rc<RefCell<Vec<gtk4::Widget>>> = Rc::new(RefCell::new(Vec::new()));
    let commands_generation: Rc<Cell<u64>> = Rc::new(Cell::new(0));
    rebuild_commands_group(
        &commands_group,
        &tracked_commands,
        settings,
        &on_changed,
        &commands_generation,
    );
    automation_page.add(&commands_group);

    preferences_window.add(&automation_page);

    // ── Page 5: Keybindings ────────────────────────────────────────────
    let keybindings_page = adw::PreferencesPage::new();
    keybindings_page.set_title("Keybindings");
    keybindings_page.set_icon_name(Some("preferences-desktop-keyboard-symbolic"));

    // Built-in shortcuts group
    let builtin_group = adw::PreferencesGroup::new();
    builtin_group.set_title("Built-in Shortcuts");
    builtin_group.set_description(Some(
        "Click a shortcut to change it. Press Backspace to reset to default.\n\
         Keybinding changes take effect after restarting Impulse.",
    ));

    for category in keybindings::categories() {
        // Category header
        let header = adw::ActionRow::new();
        header.set_title(category);
        header.add_css_class("heading");
        builtin_group.add(&header);

        for kb in keybindings::BUILTIN_KEYBINDINGS
            .iter()
            .filter(|kb| kb.category == *category)
        {
            let overrides = settings.borrow().keybinding_overrides.clone();
            let current_accel = keybindings::get_accel(kb.id, &overrides);
            let display_text = keybindings::accel_to_display(&current_accel);
            let is_overridden = overrides.contains_key(kb.id);

            let row = adw::ActionRow::new();
            row.set_title(kb.description);
            if is_overridden {
                row.set_subtitle(&format!(
                    "Default: {}",
                    keybindings::accel_to_display(kb.default_accel)
                ));
            }

            let btn = gtk4::Button::with_label(&display_text);
            btn.set_valign(gtk4::Align::Center);
            btn.add_css_class("flat");
            row.add_suffix(&btn);

            {
                let settings = Rc::clone(settings);
                let on_changed = Rc::clone(&on_changed);
                let kb_id = kb.id.to_string();
                let kb_default = kb.default_accel.to_string();
                let kb_desc = kb.description.to_string();
                let row = row.clone();
                let preferences_window_ref = preferences_window.clone();
                btn.connect_clicked(move |btn| {
                    show_key_capture_dialog(
                        &preferences_window_ref,
                        &kb_desc,
                        &kb_id,
                        &kb_default,
                        btn,
                        &row,
                        &settings,
                        &on_changed,
                    );
                });
            }

            builtin_group.add(&row);
        }
    }

    keybindings_page.add(&builtin_group);

    // Custom keybindings group
    let custom_kb_group = adw::PreferencesGroup::new();
    custom_kb_group.set_title("Custom Keybindings");
    custom_kb_group.set_description(Some("Shortcuts that run shell commands"));
    let tracked_custom_kb: Rc<RefCell<Vec<gtk4::Widget>>> = Rc::new(RefCell::new(Vec::new()));
    let custom_kb_generation: Rc<Cell<u64>> = Rc::new(Cell::new(0));
    rebuild_custom_keybindings_group(
        &custom_kb_group,
        &tracked_custom_kb,
        settings,
        &on_changed,
        &custom_kb_generation,
    );
    keybindings_page.add(&custom_kb_group);

    preferences_window.add(&keybindings_page);

    // ── Page 6: Language Servers ────────────────────────────────────────
    let lsp_page = adw::PreferencesPage::new();
    lsp_page.set_title("Language Servers");
    lsp_page.set_icon_name(Some("network-server-symbolic"));

    // -- npm status group --
    let npm_group = adw::PreferencesGroup::new();
    npm_group.set_title("npm");
    let npm_row = adw::ActionRow::new();
    if impulse_core::lsp::npm_is_available() {
        npm_row.set_title("npm");
        npm_row.set_subtitle("Available");
        let icon = gtk4::Image::from_icon_name("emblem-ok-symbolic");
        icon.set_valign(gtk4::Align::Center);
        npm_row.add_suffix(&icon);
    } else {
        npm_row.set_title("npm");
        npm_row.set_subtitle("Not found — install Node.js to manage web language servers");
        let icon = gtk4::Image::from_icon_name("dialog-warning-symbolic");
        icon.set_valign(gtk4::Align::Center);
        npm_row.add_suffix(&icon);
    }
    npm_group.add(&npm_row);
    lsp_page.add(&npm_group);

    // -- Managed Web Language Servers group --
    let managed_group = adw::PreferencesGroup::new();
    managed_group.set_title("Managed Web Language Servers");
    managed_group.set_description(Some("Installed and managed by Impulse via npm."));

    // Helper: rebuild managed server rows from fresh data
    fn rebuild_managed_lsp_rows(
        group: &adw::PreferencesGroup,
        tracked: &Rc<RefCell<Vec<gtk4::Widget>>>,
    ) {
        let mut rows = tracked.borrow_mut();
        for row in rows.drain(..) {
            group.remove(&row);
        }
        let statuses = impulse_core::lsp::managed_web_lsp_status();
        for status in &statuses {
            let row = adw::ActionRow::new();
            row.set_title(&status.command);
            if let Some(ref path) = status.resolved_path {
                row.set_subtitle(&path.to_string_lossy());
                let icon = gtk4::Image::from_icon_name("emblem-ok-symbolic");
                icon.set_valign(gtk4::Align::Center);
                row.add_suffix(&icon);
            } else {
                row.set_subtitle("Not installed");
                let icon = gtk4::Image::from_icon_name("window-close-symbolic");
                icon.set_valign(gtk4::Align::Center);
                row.add_suffix(&icon);
            }
            group.add(&row);
            rows.push(row.upcast());
        }
    }

    let tracked_managed: Rc<RefCell<Vec<gtk4::Widget>>> = Rc::new(RefCell::new(Vec::new()));
    rebuild_managed_lsp_rows(&managed_group, &tracked_managed);

    // Install button row
    let install_row = adw::ActionRow::new();
    install_row.set_title("Install All Web Language Servers");
    install_row.set_subtitle("Downloads and installs via npm");

    let install_spinner = gtk4::Spinner::new();
    install_spinner.set_visible(false);
    install_spinner.set_valign(gtk4::Align::Center);

    let install_button = gtk4::Button::with_label("Install");
    install_button.set_valign(gtk4::Align::Center);
    install_button.add_css_class("suggested-action");
    if !impulse_core::lsp::npm_is_available() {
        install_button.set_sensitive(false);
        install_button.set_tooltip_text(Some("npm is not available"));
    }

    install_row.add_suffix(&install_spinner);
    install_row.add_suffix(&install_button);
    managed_group.add(&install_row);

    {
        let managed_group = managed_group.clone();
        let tracked_managed = Rc::clone(&tracked_managed);
        let install_spinner = install_spinner.clone();
        let install_button = install_button.clone();
        let preferences_window_weak = preferences_window.downgrade();
        install_button.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            install_spinner.set_visible(true);
            install_spinner.set_spinning(true);

            let (tx, rx) = std::sync::mpsc::channel::<Result<std::path::PathBuf, String>>();

            std::thread::spawn(move || {
                let result = impulse_core::lsp::install_managed_web_lsp_servers();
                let _ = tx.send(result);
            });

            let managed_group = managed_group.clone();
            let tracked_managed = Rc::clone(&tracked_managed);
            let install_spinner = install_spinner.clone();
            let btn = btn.clone();
            let preferences_window_weak = preferences_window_weak.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
                match rx.try_recv() {
                    Ok(result) => {
                        install_spinner.set_spinning(false);
                        install_spinner.set_visible(false);
                        btn.set_sensitive(true);

                        rebuild_managed_lsp_rows(&managed_group, &tracked_managed);

                        if let Some(win) = preferences_window_weak.upgrade() {
                            let toast = match result {
                                Ok(_) => adw::Toast::new("Language servers installed successfully"),
                                Err(ref e) => adw::Toast::new(&format!("Install failed: {e}")),
                            };
                            win.add_toast(toast);
                        }
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        if preferences_window_weak.upgrade().is_none() {
                            return glib::ControlFlow::Break;
                        }
                        glib::ControlFlow::Continue
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        install_spinner.set_spinning(false);
                        install_spinner.set_visible(false);
                        btn.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                }
            });
        });
    }

    lsp_page.add(&managed_group);

    // -- System Language Servers group --
    let system_group = adw::PreferencesGroup::new();
    system_group.set_title("System Language Servers");
    system_group.set_description(Some("Install these via your system package manager."));

    let system_statuses = impulse_core::lsp::system_lsp_status();
    for status in &system_statuses {
        let row = adw::ActionRow::new();
        row.set_title(&status.command);
        if let Some(ref path) = status.resolved_path {
            row.set_subtitle(&path.to_string_lossy());
            let icon = gtk4::Image::from_icon_name("emblem-ok-symbolic");
            icon.set_valign(gtk4::Align::Center);
            row.add_suffix(&icon);
        } else {
            row.set_subtitle("Not found in PATH");
            let icon = gtk4::Image::from_icon_name("window-close-symbolic");
            icon.set_valign(gtk4::Align::Center);
            row.add_suffix(&icon);
        }
        system_group.add(&row);
    }

    lsp_page.add(&system_group);

    preferences_window.add(&lsp_page);

    preferences_window.present();
}

#[allow(clippy::too_many_arguments)]
fn show_key_capture_dialog(
    parent: &adw::PreferencesWindow,
    description: &str,
    kb_id: &str,
    kb_default: &str,
    btn: &gtk4::Button,
    row: &adw::ActionRow,
    settings: &Rc<RefCell<Settings>>,
    on_changed: &Rc<dyn Fn(&Settings)>,
) {
    let dialog = gtk4::Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .default_width(400)
        .default_height(120)
        .build();
    dialog.add_css_class("quick-open");

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    vbox.set_margin_top(24);
    vbox.set_margin_bottom(24);
    vbox.set_margin_start(24);
    vbox.set_margin_end(24);

    let label = gtk4::Label::new(Some(&format!(
        "Press a key combination for \"{}\"\nPress Escape to cancel, Backspace to reset",
        description
    )));
    label.set_halign(gtk4::Align::Center);
    vbox.append(&label);

    dialog.set_child(Some(&vbox));

    let key_controller = gtk4::EventControllerKey::new();
    {
        let dialog = dialog.clone();
        let kb_id = kb_id.to_string();
        let kb_default = kb_default.to_string();
        let btn = btn.clone();
        let row = row.clone();
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(on_changed);
        key_controller.connect_key_pressed(move |_, key, _keycode, modifiers| {
            // Escape cancels
            if key == gtk4::gdk::Key::Escape {
                dialog.close();
                return gtk4::glib::Propagation::Stop;
            }

            // Backspace/Delete resets to default
            if key == gtk4::gdk::Key::BackSpace || key == gtk4::gdk::Key::Delete {
                {
                    let mut s = settings.borrow_mut();
                    s.keybinding_overrides.remove(&kb_id);
                    settings::save(&s);
                    on_changed(&s);
                }
                btn.set_label(&keybindings::accel_to_display(&kb_default));
                row.set_subtitle("");
                dialog.close();
                return gtk4::glib::Propagation::Stop;
            }

            // Ignore lone modifier keys
            if matches!(
                key,
                gtk4::gdk::Key::Shift_L
                    | gtk4::gdk::Key::Shift_R
                    | gtk4::gdk::Key::Control_L
                    | gtk4::gdk::Key::Control_R
                    | gtk4::gdk::Key::Alt_L
                    | gtk4::gdk::Key::Alt_R
                    | gtk4::gdk::Key::Super_L
                    | gtk4::gdk::Key::Super_R
                    | gtk4::gdk::Key::Meta_L
                    | gtk4::gdk::Key::Meta_R
            ) {
                return gtk4::glib::Propagation::Stop;
            }

            // Build display string from modifiers + key
            let mut parts = Vec::new();
            if modifiers.contains(gtk4::gdk::ModifierType::CONTROL_MASK) {
                parts.push("Ctrl");
            }
            if modifiers.contains(gtk4::gdk::ModifierType::SHIFT_MASK) {
                parts.push("Shift");
            }
            if modifiers.contains(gtk4::gdk::ModifierType::ALT_MASK) {
                parts.push("Alt");
            }
            if modifiers.contains(gtk4::gdk::ModifierType::SUPER_MASK) {
                parts.push("Super");
            }

            let key_name = key_to_display_name(key);
            if key_name.is_empty() {
                return gtk4::glib::Propagation::Stop;
            }
            parts.push(&key_name);

            let display_str = parts.join("+");

            // Store the override
            {
                let mut s = settings.borrow_mut();
                s.keybinding_overrides
                    .insert(kb_id.clone(), display_str.clone());
                settings::save(&s);
                on_changed(&s);
            }
            btn.set_label(&display_str);
            row.set_subtitle(&format!(
                "Default: {}",
                keybindings::accel_to_display(&kb_default)
            ));
            dialog.close();
            gtk4::glib::Propagation::Stop
        });
    }
    dialog.add_controller(key_controller);
    dialog.present();
}

fn key_to_display_name(key: gtk4::gdk::Key) -> String {
    match key {
        gtk4::gdk::Key::Tab => "Tab".to_string(),
        gtk4::gdk::Key::Return | gtk4::gdk::Key::KP_Enter => "Return".to_string(),
        gtk4::gdk::Key::space => "Space".to_string(),
        gtk4::gdk::Key::Left => "Left".to_string(),
        gtk4::gdk::Key::Right => "Right".to_string(),
        gtk4::gdk::Key::Up => "Up".to_string(),
        gtk4::gdk::Key::Down => "Down".to_string(),
        gtk4::gdk::Key::Home => "Home".to_string(),
        gtk4::gdk::Key::End => "End".to_string(),
        gtk4::gdk::Key::Page_Up => "Page_Up".to_string(),
        gtk4::gdk::Key::Page_Down => "Page_Down".to_string(),
        gtk4::gdk::Key::F1 => "F1".to_string(),
        gtk4::gdk::Key::F2 => "F2".to_string(),
        gtk4::gdk::Key::F3 => "F3".to_string(),
        gtk4::gdk::Key::F4 => "F4".to_string(),
        gtk4::gdk::Key::F5 => "F5".to_string(),
        gtk4::gdk::Key::F6 => "F6".to_string(),
        gtk4::gdk::Key::F7 => "F7".to_string(),
        gtk4::gdk::Key::F8 => "F8".to_string(),
        gtk4::gdk::Key::F9 => "F9".to_string(),
        gtk4::gdk::Key::F10 => "F10".to_string(),
        gtk4::gdk::Key::F11 => "F11".to_string(),
        gtk4::gdk::Key::F12 => "F12".to_string(),
        gtk4::gdk::Key::comma => ",".to_string(),
        gtk4::gdk::Key::period => ".".to_string(),
        gtk4::gdk::Key::equal => "=".to_string(),
        gtk4::gdk::Key::minus => "-".to_string(),
        gtk4::gdk::Key::semicolon => ";".to_string(),
        gtk4::gdk::Key::slash => "/".to_string(),
        gtk4::gdk::Key::bracketleft => "[".to_string(),
        gtk4::gdk::Key::bracketright => "]".to_string(),
        gtk4::gdk::Key::backslash => "\\".to_string(),
        gtk4::gdk::Key::grave => "`".to_string(),
        gtk4::gdk::Key::apostrophe => "'".to_string(),
        _ => {
            // Try to get the key name from the key value
            let name = key.name().map(|n| n.to_string()).unwrap_or_default();
            if name.len() == 1 {
                name.to_uppercase()
            } else {
                name
            }
        }
    }
}

fn rebuild_custom_keybindings_group(
    group: &adw::PreferencesGroup,
    tracked: &Rc<RefCell<Vec<gtk4::Widget>>>,
    settings: &Rc<RefCell<Settings>>,
    on_changed: &Rc<dyn Fn(&Settings)>,
    generation: &Rc<Cell<u64>>,
) {
    // Increment generation so stale closures from previous rebuilds become no-ops
    generation.set(generation.get() + 1);
    let gen = generation.get();

    for row in tracked.borrow().iter() {
        group.remove(row);
    }
    tracked.borrow_mut().clear();

    let count = settings.borrow().custom_keybindings.len();
    for i in 0..count {
        let (name, key, command, args) = {
            let s = settings.borrow();
            let kb = &s.custom_keybindings[i];
            (
                kb.name.clone(),
                kb.key.clone(),
                kb.command.clone(),
                kb.args.join(" "),
            )
        };

        let expander = adw::ExpanderRow::new();
        expander.set_title(&name);
        expander.set_subtitle(&key);

        let delete_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
        delete_btn.set_valign(gtk4::Align::Center);
        delete_btn.add_css_class("flat");
        {
            let group = group.clone();
            let tracked = Rc::clone(tracked);
            let settings = Rc::clone(settings);
            let on_changed = Rc::clone(on_changed);
            let generation = Rc::clone(generation);
            delete_btn.connect_clicked(move |_| {
                {
                    let mut s = settings.borrow_mut();
                    if i >= s.custom_keybindings.len() {
                        return;
                    }
                    s.custom_keybindings.remove(i);
                    settings::save(&s);
                    on_changed(&s);
                }
                rebuild_custom_keybindings_group(
                    &group,
                    &tracked,
                    &settings,
                    &on_changed,
                    &generation,
                );
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
            let generation = Rc::clone(generation);
            name_row.connect_changed(move |row| {
                if generation.get() != gen {
                    return;
                }
                let mut s = settings.borrow_mut();
                if i >= s.custom_keybindings.len() {
                    return;
                }
                s.custom_keybindings[i].name = row.text().to_string();
                expander.set_title(&row.text());
                settings::save(&s);
                on_changed(&s);
            });
        }
        expander.add_row(&name_row);

        let key_row = adw::EntryRow::new();
        key_row.set_title("Key");
        key_row.set_text(&key);
        {
            let settings = Rc::clone(settings);
            let on_changed = Rc::clone(on_changed);
            let expander = expander.clone();
            let generation = Rc::clone(generation);
            key_row.connect_changed(move |row| {
                if generation.get() != gen {
                    return;
                }
                let mut s = settings.borrow_mut();
                if i >= s.custom_keybindings.len() {
                    return;
                }
                s.custom_keybindings[i].key = row.text().to_string();
                expander.set_subtitle(&row.text());
                settings::save(&s);
                on_changed(&s);
            });
        }
        expander.add_row(&key_row);

        let cmd_row = adw::EntryRow::new();
        cmd_row.set_title("Command");
        cmd_row.set_text(&command);
        {
            let settings = Rc::clone(settings);
            let on_changed = Rc::clone(on_changed);
            let generation = Rc::clone(generation);
            cmd_row.connect_changed(move |row| {
                if generation.get() != gen {
                    return;
                }
                let mut s = settings.borrow_mut();
                if i >= s.custom_keybindings.len() {
                    return;
                }
                s.custom_keybindings[i].command = row.text().to_string();
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
            let generation = Rc::clone(generation);
            args_row.connect_changed(move |row| {
                if generation.get() != gen {
                    return;
                }
                let mut s = settings.borrow_mut();
                if i >= s.custom_keybindings.len() {
                    return;
                }
                s.custom_keybindings[i].args = row
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

        group.add(&expander);
        tracked.borrow_mut().push(expander.upcast());
    }

    let add_row = adw::ActionRow::new();
    add_row.set_title("Add Custom Keybinding");
    add_row.set_activatable(true);
    add_row.add_prefix(&gtk4::Image::from_icon_name("list-add-symbolic"));
    {
        let group = group.clone();
        let tracked = Rc::clone(tracked);
        let settings = Rc::clone(settings);
        let on_changed = Rc::clone(on_changed);
        let generation = Rc::clone(generation);
        add_row.connect_activated(move |_| {
            {
                let mut s = settings.borrow_mut();
                s.custom_keybindings.push(CustomKeybinding {
                    name: "new keybinding".to_string(),
                    key: String::new(),
                    command: String::new(),
                    args: Vec::new(),
                });
                settings::save(&s);
                on_changed(&s);
            }
            rebuild_custom_keybindings_group(&group, &tracked, &settings, &on_changed, &generation);
        });
    }
    group.add(&add_row);
    tracked.borrow_mut().push(add_row.upcast());
}
