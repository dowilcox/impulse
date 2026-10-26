//! The Settings tab, as the Swift `SettingsSurface` draws it: categories on
//! the left, the settings on the right, search across all of them, a marker
//! on everything changed from its default and a reset per setting. Edits
//! apply immediately through `SettingsStore::update`. Automation (commands
//! on save, file types) and Language Servers are their own pages, as in
//! `SettingsSections.swift`; Advanced links to Keyboard Shortcuts and
//! settings.json.

use std::cell::RefCell;
use std::collections::HashMap;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState, NumberInputEvent, StepAction};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{IndexPath, Sizable as _};
use gpui_kit::{
    div, prelude::*, px, AnyElement, App, Context, ElementId, Entity, FocusHandle, Focusable as _,
    FontWeight, SharedString, Subscription, Window,
};
use impulse_kit::chrome::{metrics, ChromePalette, Rgba};
use impulse_kit::settings::{
    catalog, Category, CommandOnSave, Control, FileTypeOverride, FormatOnSave, SettingItem,
    Settings,
};
use impulse_lsp::managed::ServerStatus;
use serde_json::{json, Value};

use crate::commands::RunCommand;
use crate::settings_store::SettingsStore;
use crate::workbench::Workbench;
use crate::workbench_chrome::{
    abbreviate_home, chrome_button, hsla, icon, ButtonKind, ChromeCheckbox,
};
use crate::workspace::Surface;

pub(crate) const MONO_FONT: &str = "JetBrains Mono";

/// One option of a select: the value saved, the label shown.
#[derive(Clone)]
pub(crate) struct Choice {
    pub value: String,
    pub title: SharedString,
}

impl Choice {
    pub fn new(value: impl Into<String>, title: impl Into<SharedString>) -> Self {
        Choice {
            value: value.into(),
            title: title.into(),
        }
    }
}

impl SelectItem for Choice {
    type Value = String;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &String {
        &self.value
    }
}

pub(crate) type ChoiceSelect = Entity<SelectState<SearchableVec<Choice>>>;

/// A select over `choices`, `selected` chosen.
pub(crate) fn choice_select(
    choices: Vec<Choice>,
    selected: &str,
    searchable: bool,
    window: &mut Window,
    cx: &mut App,
) -> ChoiceSelect {
    let index = choices
        .iter()
        .position(|c| c.value == selected)
        .map(|row| IndexPath::default().row(row));
    cx.new(|cx| {
        SelectState::new(SearchableVec::new(choices), index, window, cx).searchable(searchable)
    })
}

/// The control behind a catalog setting.
enum Field {
    Number(Entity<InputState>),
    Text(Entity<InputState>),
    Choice(ChoiceSelect),
}

/// A row of Commands on save.
struct CommandRow {
    name: Entity<InputState>,
    pattern: Entity<InputState>,
    line: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

/// A row of File types.
struct FileTypeRow {
    pattern: Entity<InputState>,
    tab_width: ChoiceSelect,
    indent: ChoiceSelect,
    formatter: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

/// The Language Servers page: managed and system servers, and npm.
struct Servers {
    managed: Vec<ServerStatus>,
    system: Vec<ServerStatus>,
    npm: bool,
}

pub struct SettingsView {
    pub(crate) palette: ChromePalette,
    pub(crate) focus: FocusHandle,
    category: Category,
    search: Entity<InputState>,
    only_modified: bool,
    items: Vec<SettingItem>,
    fields: HashMap<&'static str, Field>,
    commands: Vec<CommandRow>,
    file_types: Vec<FileTypeRow>,
    servers: Option<Servers>,
    installing: bool,
    /// What the last install said, and whether it failed.
    install_message: Option<(String, bool)>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsView {
    pub fn new(palette: ChromePalette, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search settings")
                // A search field's placeholder, as NSSearchField draws it.
                .placeholder_color(Some(crate::workbench_chrome::hsla(palette.text_secondary)))
                .clean_on_escape()
        });
        let mut subscriptions = vec![
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.observe_global_in::<SettingsStore>(window, |view, window, cx| {
                view.sync(window, cx);
                cx.notify();
            }),
        ];
        let settings = SettingsStore::get(cx).clone();
        let items = catalog();
        let mut fields = HashMap::new();
        for item in &items {
            let key = item.key;
            let field = match &item.control {
                // Decimals show their value as text, with no field.
                Control::Toggle | Control::Decimal { .. } => continue,
                Control::Integer { zero_label, .. } => {
                    let zero_label = zero_label.unwrap_or("");
                    let state = cx.new(|cx| {
                        let mut state = InputState::new(window, cx)
                            .placeholder(zero_label)
                            .default_value(field_text(item, &settings));
                        // The steppers go through `step`, which saves.
                        state.set_step(None, window, cx);
                        state
                    });
                    subscriptions.push(cx.subscribe_in(
                        &state,
                        window,
                        move |view, _, event: &NumberInputEvent, window, cx| {
                            let NumberInputEvent::Step(action) = event;
                            view.step(key, *action, window, cx);
                        },
                    ));
                    subscriptions.push(cx.subscribe_in(
                        &state,
                        window,
                        move |view, _, event: &InputEvent, window, cx| {
                            if is_commit(event) {
                                view.commit_number(key, window, cx);
                            }
                        },
                    ));
                    Field::Number(state)
                }
                Control::Text { placeholder } | Control::Folder { placeholder } => {
                    let state = cx.new(|cx| {
                        InputState::new(window, cx)
                            .placeholder(*placeholder)
                            .default_value(field_text(item, &settings))
                    });
                    subscriptions.push(cx.subscribe(&state, move |_, state, event, cx| {
                        if is_commit(event) {
                            let text = state.read(cx).value().to_string();
                            SettingsStore::update(cx, |s| {
                                s.set(key, &Value::String(text));
                            });
                        }
                    }));
                    Field::Text(state)
                }
                Control::Choice(_) | Control::Theme | Control::Font => {
                    let current = field_text(item, &settings);
                    let (choices, selected) = choices_for(item, &current, cx);
                    let searchable = !matches!(item.control, Control::Choice(_));
                    let state = choice_select(choices, &selected, searchable, window, cx);
                    subscriptions.push(cx.subscribe(
                        &state,
                        move |_, _, event: &SelectEvent<SearchableVec<Choice>>, cx| {
                            if let SelectEvent::Confirm(Some(value)) = event {
                                let value = value.clone();
                                SettingsStore::update(cx, |s| {
                                    s.set(key, &Value::String(value));
                                });
                            }
                        },
                    ));
                    Field::Choice(state)
                }
            };
            fields.insert(key, field);
        }
        let mut view = SettingsView {
            palette,
            focus: cx.focus_handle(),
            category: Category::General,
            search,
            only_modified: false,
            items,
            fields,
            commands: Vec::new(),
            file_types: Vec::new(),
            servers: None,
            installing: false,
            install_message: None,
            _subscriptions: subscriptions,
        };
        view.sync(window, cx);
        view
    }

    /// The search field takes the keyboard, as `focusTool` does.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.search
            .update(cx, |search, cx| search.focus(window, cx));
    }

    /// Show one setting (the palette's deep link): search for it.
    pub fn reveal(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |search, cx| {
            search.set_value(query.to_string(), window, cx);
            search.focus(window, cx);
        });
        cx.notify();
    }

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().trim().to_string()
    }

    fn is_searching(&self, cx: &App) -> bool {
        !self.query(cx).is_empty() || self.only_modified
    }

    /// What the right side lists: search results across categories, else
    /// the selected category.
    fn visible_items(&self, cx: &App) -> Vec<&SettingItem> {
        let settings = SettingsStore::get(cx);
        let query = self.query(cx);
        self.items
            .iter()
            .filter(|item| {
                if self.only_modified && !item.is_modified(settings) {
                    return false;
                }
                if !query.is_empty() {
                    return item.matches(&query);
                }
                self.only_modified || item.category == self.category
            })
            .collect()
    }

    fn modified_count(&self, category: Category, cx: &App) -> usize {
        let settings = SettingsStore::get(cx);
        self.items
            .iter()
            .filter(|i| i.category == category && i.is_modified(settings))
            .count()
    }

    // MARK: Keeping the fields current

    /// Show the settings' values in fields that aren't being edited, and
    /// keep the Automation rows in step with their lists.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = SettingsStore::get(cx).clone();
        for item in &self.items {
            let Some(field) = self.fields.get(item.key) else {
                continue;
            };
            let text = field_text(item, &settings);
            match field {
                Field::Number(state) | Field::Text(state) => {
                    set_unless_editing(state, &text, window, cx)
                }
                Field::Choice(state) => {
                    if state.read(cx).selected_value() == Some(&text) {
                        continue;
                    }
                    let (choices, selected) = choices_for(item, &text, cx);
                    state.update(cx, |state, cx| {
                        state.set_items(SearchableVec::new(choices), window, cx);
                        state.set_selected_value(&selected, window, cx);
                    });
                }
            }
        }
        self.sync_commands(&settings.commands_on_save, window, cx);
        self.sync_file_types(&settings.file_type_overrides, window, cx);
    }

    fn sync_commands(
        &mut self,
        commands: &[CommandOnSave],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commands.truncate(commands.len());
        while self.commands.len() < commands.len() {
            let index = self.commands.len();
            let name = text_input("Name", window, cx);
            let pattern = text_input("*.swift", window, cx);
            let line = text_input("command --flag {file}", window, cx);
            let subscriptions = vec![
                commit_on(&name, cx, move |text, s| {
                    if let Some(c) = s.commands_on_save.get_mut(index) {
                        c.name = text;
                    }
                }),
                commit_on(&pattern, cx, move |text, s| {
                    if let Some(c) = s.commands_on_save.get_mut(index) {
                        c.file_pattern = text;
                    }
                }),
                commit_on(&line, cx, move |text, s| {
                    let mut parts = split_command_line(&text).into_iter();
                    if let Some(c) = s.commands_on_save.get_mut(index) {
                        c.command = parts.next().unwrap_or_default();
                        c.args = parts.collect();
                    }
                }),
            ];
            self.commands.push(CommandRow {
                name,
                pattern,
                line,
                _subscriptions: subscriptions,
            });
        }
        for (row, command) in self.commands.iter().zip(commands) {
            let line = join_command_line(
                &std::iter::once(command.command.clone())
                    .chain(command.args.iter().cloned())
                    .collect::<Vec<_>>(),
            );
            set_unless_editing(&row.name, &command.name, window, cx);
            set_unless_editing(&row.pattern, &command.file_pattern, window, cx);
            set_unless_editing(&row.line, &line, window, cx);
        }
    }

    fn sync_file_types(
        &mut self,
        overrides: &[FileTypeOverride],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_types.truncate(overrides.len());
        while self.file_types.len() < overrides.len() {
            let index = self.file_types.len();
            let pattern = text_input("*.py", window, cx);
            let formatter = text_input("formatter (optional)", window, cx);
            let tab_widths = std::iter::once(Choice::new("0", "Tab width: default"))
                .chain(
                    [2, 4, 8]
                        .iter()
                        .map(|w| Choice::new(w.to_string(), format!("Tab width: {w}"))),
                )
                .collect();
            let tab_width = choice_select(tab_widths, "0", false, window, cx);
            let indents = vec![
                Choice::new("default", "Indent: default"),
                Choice::new("spaces", "Spaces"),
                Choice::new("tabs", "Tabs"),
            ];
            let indent = choice_select(indents, "default", false, window, cx);
            let subscriptions = vec![
                commit_on(&pattern, cx, move |text, s| {
                    if let Some(o) = s.file_type_overrides.get_mut(index) {
                        o.pattern = text;
                    }
                }),
                commit_on(&formatter, cx, move |text, s| {
                    let mut parts = split_command_line(&text).into_iter();
                    if let Some(o) = s.file_type_overrides.get_mut(index) {
                        o.format_on_save = parts.next().map(|command| FormatOnSave {
                            command,
                            args: parts.collect(),
                        });
                    }
                }),
                cx.subscribe(
                    &tab_width,
                    move |_, _, event: &SelectEvent<SearchableVec<Choice>>, cx| {
                        if let SelectEvent::Confirm(Some(value)) = event {
                            let width = value.parse::<i64>().unwrap_or(0);
                            SettingsStore::update(cx, |s| {
                                if let Some(o) = s.file_type_overrides.get_mut(index) {
                                    o.tab_width = (width != 0).then_some(width);
                                }
                            });
                        }
                    },
                ),
                cx.subscribe(
                    &indent,
                    move |_, _, event: &SelectEvent<SearchableVec<Choice>>, cx| {
                        if let SelectEvent::Confirm(Some(value)) = event {
                            let spaces = match value.as_str() {
                                "spaces" => Some(true),
                                "tabs" => Some(false),
                                _ => None,
                            };
                            SettingsStore::update(cx, |s| {
                                if let Some(o) = s.file_type_overrides.get_mut(index) {
                                    o.use_spaces = spaces;
                                }
                            });
                        }
                    },
                ),
            ];
            self.file_types.push(FileTypeRow {
                pattern,
                tab_width,
                indent,
                formatter,
                _subscriptions: subscriptions,
            });
        }
        for (row, entry) in self.file_types.iter().zip(overrides) {
            set_unless_editing(&row.pattern, &entry.pattern, window, cx);
            let formatter = entry
                .format_on_save
                .as_ref()
                .map(|f| {
                    join_command_line(
                        &std::iter::once(f.command.clone())
                            .chain(f.args.iter().cloned())
                            .collect::<Vec<_>>(),
                    )
                })
                .unwrap_or_default();
            set_unless_editing(&row.formatter, &formatter, window, cx);
            let width = entry.tab_width.unwrap_or(0).to_string();
            let indent = match entry.use_spaces {
                Some(true) => "spaces",
                Some(false) => "tabs",
                None => "default",
            }
            .to_string();
            for (state, value) in [(&row.tab_width, width), (&row.indent, indent)] {
                if state.read(cx).selected_value() != Some(&value) {
                    state.update(cx, |state, cx| state.set_selected_value(&value, window, cx));
                }
            }
        }
    }

    // MARK: Edits

    /// The stepper of a number field.
    fn step(&mut self, key: &str, action: StepAction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(control) = self
            .items
            .iter()
            .find(|i| i.key == key)
            .map(|i| i.control.clone())
        else {
            return;
        };
        let up = action == StepAction::Increment;
        SettingsStore::update(cx, |s| match control {
            Control::Integer { min, max, step, .. } => {
                let value = s.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
                let value = if up { value + step } else { value - step };
                s.set(key, &json!(value.clamp(min, max)));
            }
            Control::Decimal { min, max, step } => {
                let value = s.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0);
                let value = if up { value + step } else { value - step };
                let value = (value.clamp(min, max) * 100.0).round() / 100.0;
                s.set(key, &json!(value));
            }
            _ => {}
        });
        self.show_value(key, window, cx);
    }

    /// Put the setting's value in its number field, even while it has the
    /// keyboard (stepping focuses it; a commit may have clamped the value).
    fn show_value(&self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(item), Some(Field::Number(state))) = (
            self.items.iter().find(|i| i.key == key),
            self.fields.get(key),
        ) else {
            return;
        };
        let text = field_text(item, SettingsStore::get(cx));
        if state.read(cx).value() != text {
            state.update(cx, |state, cx| state.set_value(text, window, cx));
        }
    }

    /// Typed into a number field: on Return or when focus leaves, the value
    /// (clamped to its range); empty is 0 when 0 has a label.
    fn commit_number(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.items.iter().find(|i| i.key == key).cloned() else {
            return;
        };
        let Some(Field::Number(state)) = self.fields.get(key) else {
            return;
        };
        let text = state.read(cx).value().to_string();
        let settings = SettingsStore::get(cx).clone();
        let value = match item.control {
            Control::Integer {
                min,
                max,
                zero_label,
                ..
            } => {
                let current = settings.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
                let digits: String = text.chars().filter(char::is_ascii_digit).collect();
                let parsed = if digits.is_empty() {
                    if zero_label.is_some() {
                        0
                    } else {
                        current
                    }
                } else {
                    digits.parse().unwrap_or(current)
                };
                json!(parsed.clamp(min, max))
            }
            _ => return,
        };
        if settings.get(key) != Some(value.clone()) {
            SettingsStore::update(cx, |s| {
                s.set(key, &value);
            });
        }
        // Unchanged, clamped or not a number: show the value as saved.
        self.show_value(key, window, cx);
    }

    /// Choose a folder in an open panel; stored with `~`.
    fn choose_folder(&mut self, key: &'static str, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |_, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                if let Some(path) = paths.into_iter().next() {
                    let value = abbreviate_home(&path.to_string_lossy());
                    cx.update(|cx| {
                        SettingsStore::update(cx, |s| {
                            s.set(key, &Value::String(value));
                        })
                    });
                }
            }
        })
        .detach();
    }

    /// Run a workbench command from here (Open settings.json, Keyboard
    /// Shortcuts): it goes up from this tab to the workbench.
    fn run(&self, id: &str, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
        window.dispatch_action(Box::new(RunCommand::new(id)), cx);
    }

    pub(crate) fn select_category(
        &mut self,
        category: Category,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search
            .update(cx, |search, cx| search.set_value("", window, cx));
        self.only_modified = false;
        self.category = category;
        if category == Category::LanguageServers {
            self.reload_servers(cx);
        }
        cx.notify();
    }

    fn reload_servers(&mut self, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async {
            Servers {
                managed: impulse_lsp::managed::managed_status(),
                system: impulse_lsp::managed::system_status(),
                npm: impulse_lsp::managed::npm_is_available(),
            }
        });
        cx.spawn(async move |view, cx| {
            let servers = task.await;
            let _ = view.update(cx, |view, cx| {
                view.servers = Some(servers);
                cx.notify();
            });
        })
        .detach();
    }

    fn install_servers(&mut self, cx: &mut Context<Self>) {
        self.installing = true;
        self.install_message = None;
        cx.notify();
        let task = cx.background_spawn(async { impulse_lsp::managed::install() });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |view, cx| {
                view.installing = false;
                view.install_message = Some(match result {
                    Ok(_) => ("Language servers installed.".into(), false),
                    Err(message) => (message, true),
                });
                view.reload_servers(cx);
                cx.notify();
            });
        })
        .detach();
    }

    // MARK: Drawing

    fn sidebar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        let searching = self.is_searching(cx);
        let mut column = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .w(px(210.))
            .flex_none()
            .h_full()
            .p(px(12.))
            .bg(hsla(p.panel))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .h(px(28.))
                    .pl(px(8.))
                    .mb(px(8.))
                    .rounded(px(metrics::RADIUS))
                    .bg(hsla(p.raised))
                    // The focus ring, as `chrome_text_field` draws it.
                    .border_1()
                    .border_color(hsla(
                        if self.search.read(cx).focus_handle(cx).is_focused(window) {
                            p.focus_ring
                        } else {
                            p.raised
                        },
                    ))
                    .child(icon("search", 12., p.text_tertiary))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.5))
                            .child(Input::new(&self.search).appearance(false).small()),
                    ),
            );
        for (index, category) in Category::ALL.into_iter().enumerate() {
            let selected = !searching && self.category == category;
            let modified = self.modified_count(category, cx);
            column = column.child(
                div()
                    .id(("category", index))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(26.))
                    .px(px(8.))
                    .rounded(px(metrics::RADIUS))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(hsla(p.selection)))
                    .when(!selected, |d| d.hover(|s| s.bg(hsla(p.hover))))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.select_category(category, window, cx)
                    }))
                    .child(icon(
                        category.icon(),
                        13.,
                        if selected { p.accent } else { p.text_secondary },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.5))
                            .text_color(hsla(p.text))
                            .when(selected, |d| d.font_weight(FontWeight::SEMIBOLD))
                            .child(category.title()),
                    )
                    .when(modified > 0, |d| {
                        d.child(
                            div()
                                .id(("modified-count", index))
                                .font_family(MONO_FONT)
                                .text_size(px(10.))
                                .text_color(hsla(p.accent))
                                .tooltip(move |window, cx| {
                                    Tooltip::new(format!("{modified} changed from the default"))
                                        .build(window, cx)
                                })
                                .child(modified.to_string()),
                        )
                    }),
            );
        }
        column
            .child(div().flex_1())
            .child(
                ChromeCheckbox::new("only-modified")
                    .checked(self.only_modified)
                    .label("Only changed settings")
                    .small()
                    .label_style(11.5, self.palette.text_secondary)
                    .on_click(cx.listener(|view, checked: &bool, _, cx| {
                        view.only_modified = *checked;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("open-settings-json")
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .pt(px(4.))
                    .text_size(px(11.5))
                    .text_color(hsla(p.accent))
                    .cursor_pointer()
                    .on_click(
                        cx.listener(|view, _, window, cx| {
                            view.run("open_settings_json", window, cx)
                        }),
                    )
                    .child(icon("file-code", 11., p.accent))
                    .child("Open settings.json"),
            )
    }

    fn content(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.palette;
        let searching = self.is_searching(cx);
        let page = div()
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(820.))
            .px(px(28.))
            .py(px(22.));
        let body = if !searching && self.category == Category::Advanced {
            self.advanced(page, cx)
        } else if !searching && self.category == Category::Automation {
            self.automation(page.max_w(px(900.)), cx)
        } else if !searching && self.category == Category::LanguageServers {
            self.language_servers(page, cx)
        } else {
            self.settings_list(page, window, cx)
        };
        div()
            .id("settings-content")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scrollbar()
            .bg(hsla(p.content))
            .child(body)
            .into_any_element()
    }

    fn page_title(&self, title: impl Into<SharedString>) -> impl IntoElement {
        div()
            .text_size(px(18.))
            // SF Pro's line at 18 points.
            .line_height(px(21.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(hsla(self.palette.text))
            .child(title.into())
    }

    fn section_title(&self, title: String) -> gpui_kit::Div {
        div()
            .text_size(px(10.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(hsla(self.palette.text_tertiary))
            .child(crate::workbench_chrome::tracked(&title.to_uppercase()))
    }

    fn settings_list(
        &self,
        mut page: gpui_kit::Div,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let p = &self.palette;
        let searching = self.is_searching(cx);
        let items = self.visible_items(cx);
        page = page.child(div().pb(px(10.)).child(self.page_title(if searching {
            "Results".to_string()
        } else {
            self.category.title().to_string()
        })));
        if items.is_empty() {
            page = page.child(
                div()
                    .text_size(px(12.5))
                    .text_color(hsla(p.text_secondary))
                    .child(if self.only_modified {
                        "Everything is at its default.".to_string()
                    } else {
                        format!("No settings match \u{201c}{}\u{201d}.", self.query(cx))
                    }),
            );
        }
        // Items grouped by section (with the category in search results).
        let mut last_section = String::new();
        for item in items {
            let section = if searching {
                format!("{} \u{203a} {}", item.category.title(), item.section)
            } else {
                item.section.to_string()
            };
            if section != last_section {
                page = page.child(self.section_title(section.clone()).pt(px(14.)).pb(px(4.)));
                last_section = section;
            }
            page = page
                .child(self.setting_row(item, window, cx))
                .child(div().h(px(1.)).bg(hsla(p.hairline)));
        }
        page
    }

    /// One setting: title, key and description, its control, and a reset
    /// when it differs from the default.
    fn setting_row(
        &self,
        item: &SettingItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = &self.palette;
        let modified = item.is_modified(SettingsStore::get(cx));
        let key = item.key;
        div()
            .relative()
            .flex()
            .items_center()
            .gap(px(12.))
            .py(px(9.))
            .pl(px(14.))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(9.))
                    .bottom(px(9.))
                    .w(px(2.))
                    .when(modified, |d| d.bg(hsla(p.accent))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(hsla(p.text))
                                    .child(item.title),
                            )
                            .child(
                                div()
                                    .font_family(MONO_FONT)
                                    .text_size(px(10.))
                                    .text_color(hsla(p.text_tertiary))
                                    .child(item.key),
                            ),
                    )
                    .children(item.detail.map(|detail| {
                        div()
                            .text_size(px(11.5))
                            .text_color(hsla(p.text_secondary))
                            .child(detail)
                    })),
            )
            .child(div().flex_none().child(self.control(item, window, cx)))
            .child(
                icon_button(named("reset", key), "rotate-ccw", "Reset to default", p)
                    .when(!modified, |d| d.invisible())
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if let Some(item) = view.items.iter().find(|i| i.key == key) {
                            let item = item.clone();
                            SettingsStore::update(cx, |s| item.reset(s));
                        }
                    })),
            )
    }

    /// Up and down arrows, one above the other (`NSStepper`).
    /// Swift's `Stepper`: an 18x24 pill, the halves split by a hairline,
    /// with chevrons 8 wide.
    fn stepper(&self, key: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.palette;
        let (fill, line, chevron) = if p.is_light {
            (
                Rgba::BLACK.opacity(0.06),
                Rgba::BLACK.opacity(0.12),
                Rgba::BLACK.opacity(0.55),
            )
        } else {
            (
                Rgba::WHITE.opacity(0.075),
                Rgba::WHITE.opacity(0.18),
                Rgba::WHITE.opacity(0.6),
            )
        };
        let half = |id: &'static str, name: &'static str, action: StepAction| {
            div()
                .id(named(id, key))
                .h(px(11.5))
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(hsla(p.hover)))
                .on_click(
                    cx.listener(move |view, _, window, cx| view.step(key, action, window, cx)),
                )
                .child(icon(name, 16., chevron))
        };
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(18.))
            .h(px(24.))
            .rounded(px(6.))
            .bg(hsla(fill))
            .overflow_hidden()
            .child(half("step-up", "chevron-up", StepAction::Increment))
            .child(div().mx(px(2.)).h(px(1.)).flex_none().bg(hsla(line)))
            .child(half("step-down", "chevron-down", StepAction::Decrement))
            .into_any_element()
    }

    /// An on/off switch the size and colors of Swift's small `NSSwitch`:
    /// a 44×20 track and a pill knob, the accent when on.
    fn switch(&self, key: &'static str, on: bool) -> AnyElement {
        let p = &self.palette;
        let track = if on {
            p.accent
        } else if p.is_light {
            Rgba::BLACK.opacity(0.09)
        } else {
            Rgba::WHITE.opacity(0.12)
        };
        let knob = if on || p.is_light {
            Rgba::WHITE
        } else {
            Rgba::new(0.886, 0.89, 0.894, 1.0)
        };
        div()
            .id(named("switch", key))
            .flex_none()
            .w(px(44.))
            .h(px(20.))
            .p(px(2.))
            .flex()
            .when(on, |d| d.justify_end())
            .rounded_full()
            .bg(hsla(track))
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                SettingsStore::update(cx, |s| {
                    s.set(key, &json!(!on));
                })
            })
            .child(
                div()
                    .w(px(26.))
                    .h_full()
                    .rounded_full()
                    .bg(hsla(knob))
                    .shadow_xs(),
            )
            .into_any_element()
    }

    fn control(
        &self,
        item: &SettingItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = item.key;
        let field = self.fields.get(key);
        match (&item.control, field) {
            (Control::Toggle, _) => {
                let on = SettingsStore::get(cx)
                    .get(key)
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                self.switch(key, on)
            }
            // The value as text and a stepper, as Swift's decimal control.
            (Control::Decimal { .. }, _) => {
                let value = SettingsStore::get(cx)
                    .get(key)
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        div()
                            .w(px(40.))
                            .flex()
                            .justify_end()
                            .font_family(MONO_FONT)
                            .text_size(px(12.))
                            .child(format!("{value:.1}")),
                    )
                    .child(self.stepper(key, cx))
                    .into_any_element()
            }
            // A field and an up/down stepper beside it, as Swift's
            // TextField and Stepper.
            (Control::Integer { .. }, Some(Field::Number(state))) => {
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    // A rounded-border NSTextField: the text field color,
                    // digits on the right in the mono font.
                    .child(
                        div()
                            .w(px(84.))
                            .h(px(22.))
                            .flex()
                            .items_center()
                            .px(px(6.))
                            .rounded(px(metrics::RADIUS))
                            .border_1()
                            .border_color(hsla(
                                if state.read(cx).focus_handle(cx).is_focused(window) {
                                    self.palette.focus_ring
                                } else {
                                    self.palette.hairline
                                },
                            ))
                            .bg(hsla(if self.palette.is_light {
                                Rgba::WHITE
                            } else {
                                Rgba::new(0.09, 0.09, 0.09, 1.0)
                            }))
                            .child(
                                Input::new(state)
                                    .small()
                                    .appearance(false)
                                    .px_0()
                                    .text_right()
                                    .font_family(MONO_FONT)
                                    .text_size(px(12.)),
                            ),
                    )
                    .child(self.stepper(key, cx))
                    .into_any_element()
            }
            (Control::Text { .. }, Some(Field::Text(state))) => {
                text_field(state).w(px(210.)).into_any_element()
            }
            (Control::Folder { .. }, Some(Field::Text(state))) => div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(text_field(state).w(px(150.)))
                .child(
                    Button::new(named("choose", key))
                        .label("Choose\u{2026}")
                        .small()
                        .on_click(cx.listener(move |view, _, _, cx| view.choose_folder(key, cx))),
                )
                .into_any_element(),
            // A pop-up button as wide as its title, centered where Swift's
            // 210-wide Picker frame is.
            (_, Some(Field::Choice(state))) => {
                let p = &self.palette;
                let current = SettingsStore::get(cx)
                    .get(key)
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default();
                let title = choice_title(item, &current);
                let run = gpui_kit::TextRun {
                    len: title.len(),
                    font: gpui_kit::font(crate::workbench::UI_FONT),
                    color: hsla(p.text),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let width = window
                    .text_system()
                    .shape_line(title.into(), px(13.), &[run], None)
                    .width;
                // NSPopUpButton's 24-point frame around its 22-point face.
                div()
                    .w(px(210.))
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        popup_frame(p).w(width + px(40.)).max_w(px(210.)).child(
                            popup(state)
                                .menu_width(px(240.))
                                .accessibility_label(item.title),
                        ),
                    )
                    .into_any_element()
            }
            _ => div().into_any_element(),
        }
    }

    /// Settings edited elsewhere: keyboard shortcuts and settings.json.
    fn advanced(&self, page: gpui_kit::Div, cx: &mut Context<Self>) -> gpui_kit::Div {
        let p = &self.palette;
        let link = |id: &'static str,
                    title: &'static str,
                    detail: &'static str,
                    name: &'static str,
                    command: &'static str,
                    cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(12.))
                .py(px(10.))
                .border_b_1()
                .border_color(hsla(p.hairline))
                .cursor_pointer()
                .on_click(cx.listener(move |view, _, window, cx| view.run(command, window, cx)))
                .child(
                    div()
                        .w(px(22.))
                        .flex()
                        .justify_center()
                        .child(icon(name, 15., p.accent)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .flex_1()
                        .child(
                            div()
                                .text_size(px(12.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(hsla(p.text))
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(hsla(p.text_secondary))
                                .child(detail),
                        ),
                )
                .child(icon("chevron-right", 12., p.text_tertiary))
        };
        page.child(div().pb(px(14.)).child(self.page_title("Advanced")))
            .child(link(
                "advanced-keybindings",
                "Keyboard shortcuts",
                "Rebind any command, and add shortcuts that run shell commands.",
                "keyboard",
                "open_keybindings",
                cx,
            ))
            .child(link(
                "advanced-settings-json",
                "settings.json",
                "Every setting as JSON, with completion and validation in the editor.",
                "file-code",
                "open_settings_json",
                cx,
            ))
    }

    fn list_header(
        &self,
        id: &'static str,
        title: &str,
        detail: &str,
        add: impl Fn(&mut Settings) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = &self.palette;
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .pt(px(18.))
            .pb(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(self.section_title(title.to_string()).flex_1())
                    .child(add_button(id, p).on_click(cx.listener(move |_, _, _, cx| {
                        SettingsStore::update(cx, |s| add(s));
                    }))),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(hsla(p.text_secondary))
                    .child(detail.to_string()),
            )
    }

    fn empty_note(&self, text: &'static str) -> impl IntoElement {
        div()
            .py(px(6.))
            .text_size(px(12.))
            .text_color(hsla(self.palette.text_tertiary))
            .child(text)
    }

    /// Commands on save and per-file-type overrides.
    fn automation(&self, mut page: gpui_kit::Div, cx: &mut Context<Self>) -> gpui_kit::Div {
        let p = self.palette.clone();
        let settings = SettingsStore::get(cx).clone();
        page = page
            .child(div().pb(px(4.)).child(self.page_title("Automation")))
            .child(self.list_header(
                "add-command",
                "Commands on save",
                "Run after saving files that match a pattern, e.g. a linter or code generator.",
                |s| {
                    s.commands_on_save.push(CommandOnSave {
                        name: "New command".into(),
                        ..Default::default()
                    })
                },
                cx,
            ));
        if settings.commands_on_save.is_empty() {
            page = page.child(self.empty_note("No commands yet."));
        }
        for (index, (row, entry)) in self
            .commands
            .iter()
            .zip(&settings.commands_on_save)
            .enumerate()
        {
            let reload = entry.reload_file;
            page = page
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .py(px(6.))
                        .child(mono_field(&row.name).w(px(130.)).flex_none())
                        .child(mono_field(&row.pattern).w(px(110.)).flex_none())
                        .child(mono_field(&row.line).flex_1().min_w_0())
                        .child(
                            div()
                                .id(("reload", index))
                                .tooltip(|window, cx| {
                                    Tooltip::new(
                                        "Reload the file afterwards (for commands that rewrite it)",
                                    )
                                    .build(window, cx)
                                })
                                .child(
                                    ChromeCheckbox::new(("reload-check", index))
                                        .checked(reload)
                                        .label("Reload")
                                        .label_style(11.5, p.text_secondary)
                                        .on_click(move |checked, _, cx| {
                                            let checked = *checked;
                                            SettingsStore::update(cx, |s| {
                                                if let Some(c) = s.commands_on_save.get_mut(index) {
                                                    c.reload_file = checked;
                                                }
                                            })
                                        }),
                                ),
                        )
                        .child(
                            icon_button(("remove-command", index), "trash-2", "Remove", &p)
                                .on_click(move |_, _, cx| {
                                    SettingsStore::update(cx, |s| {
                                        if index < s.commands_on_save.len() {
                                            s.commands_on_save.remove(index);
                                        }
                                    })
                                }),
                        ),
                )
                .child(div().h(px(1.)).bg(hsla(p.hairline)));
        }
        page = page.child(self.list_header(
            "add-file-type",
            "File types",
            "Indentation and a formatter per file pattern, overriding the editor defaults.",
            |s| {
                s.file_type_overrides.push(FileTypeOverride {
                    pattern: "*.ext".into(),
                    ..Default::default()
                })
            },
            cx,
        ));
        if settings.file_type_overrides.is_empty() {
            page = page.child(self.empty_note("No overrides yet."));
        }
        for (index, row) in self
            .file_types
            .iter()
            .take(settings.file_type_overrides.len())
            .enumerate()
        {
            page = page
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .py(px(6.))
                        .child(mono_field(&row.pattern).w(px(110.)).flex_none())
                        .child(
                            popup_frame(&p)
                                .w(px(150.))
                                .flex_none()
                                .child(popup(&row.tab_width)),
                        )
                        .child(
                            popup_frame(&p)
                                .w(px(130.))
                                .flex_none()
                                .child(popup(&row.indent)),
                        )
                        .child(mono_field(&row.formatter).flex_1().min_w_0())
                        .child(
                            icon_button(("remove-file-type", index), "trash-2", "Remove", &p)
                                .on_click(move |_, _, cx| {
                                    SettingsStore::update(cx, |s| {
                                        if index < s.file_type_overrides.len() {
                                            s.file_type_overrides.remove(index);
                                        }
                                    })
                                }),
                        ),
                )
                .child(div().h(px(1.)).bg(hsla(p.hairline)));
        }
        page
    }

    fn server_row(&self, status: &ServerStatus) -> impl IntoElement {
        let p = &self.palette;
        let installed = status.installed();
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .py(px(7.))
                    .child(icon(
                        if installed { "circle-check" } else { "circle" },
                        13.,
                        if installed {
                            p.success
                        } else {
                            p.text_tertiary
                        },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .font_family(MONO_FONT)
                            .text_size(px(12.5))
                            .text_color(hsla(p.text))
                            .child(status.command.clone()),
                    )
                    .child(
                        div()
                            .max_w(px(360.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(11.5))
                            .text_color(hsla(p.text_tertiary))
                            .child(match &status.resolved_path {
                                Some(path) => abbreviate_home(&path.to_string_lossy()),
                                None => "Not installed".into(),
                            }),
                    ),
            )
            .child(div().h(px(1.)).bg(hsla(p.hairline)))
    }

    fn language_servers(&self, mut page: gpui_kit::Div, cx: &mut Context<Self>) -> gpui_kit::Div {
        let p = &self.palette;
        let npm = self.servers.as_ref().is_none_or(|s| s.npm);
        page = page
            .child(self.page_title("Language Servers"))
            .child(
                div()
                    .pt(px(2.))
                    .text_size(px(11.5))
                    .text_color(hsla(p.text_secondary))
                    .child(
                        "Completions, diagnostics, go to definition and formatting come from these.",
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pt(px(18.))
                    .pb(px(4.))
                    .child(self.section_title("Managed by Impulse".into()).flex_1())
                    .when(self.installing, |d| {
                        d.child(
                            div()
                                .text_size(px(11.5))
                                .text_color(hsla(p.text_tertiary))
                                .child("Installing\u{2026}"),
                        )
                    })
                    .child(
                        div()
                            .id("install-all-wrap")
                            .tooltip(move |window, cx| {
                                Tooltip::new(if npm {
                                    "Install or update the web language servers with npm"
                                } else {
                                    "npm is not installed"
                                })
                                .build(window, cx)
                            })
                            .child(
                                chrome_button(
                                    "install-all",
                                    "Install All",
                                    None,
                                    ButtonKind::Secondary,
                                    !npm || self.installing,
                                    p,
                                )
                                .when(npm && !self.installing, |b| {
                                    b.on_click(
                                        cx.listener(|view, _, _, cx| view.install_servers(cx)),
                                    )
                                }),
                            ),
                    ),
            )
            .child(
                div()
                    .pb(px(6.))
                    .text_size(px(11.5))
                    .text_color(hsla(if npm { p.text_secondary } else { p.warning }))
                    .child(if npm {
                        "Installed with npm into Impulse's own folder."
                    } else {
                        "Install Node.js (npm) to manage these."
                    }),
            )
            .children(self.install_message.as_ref().map(|(text, failed)| {
                div()
                    .pb(px(6.))
                    .text_size(px(11.5))
                    .text_color(hsla(if *failed { p.danger } else { p.success }))
                    .child(text.clone())
            }));
        if let Some(servers) = &self.servers {
            for status in &servers.managed {
                page = page.child(self.server_row(status));
            }
        }
        page = page
            .child(
                self.section_title("From your system".into())
                    .pt(px(18.))
                    .pb(px(4.)),
            )
            .child(
                div()
                    .pb(px(6.))
                    .text_size(px(11.5))
                    .text_color(hsla(p.text_secondary))
                    .child(if cfg!(target_os = "macos") {
                        "Install these with your package manager (Homebrew, rustup, go, \u{2026})."
                    } else {
                        "Install these with your package manager (apt, dnf, rustup, go, \u{2026})."
                    }),
            );
        if let Some(servers) = &self.servers {
            for status in &servers.system {
                page = page.child(self.server_row(status));
            }
        }
        page
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.category == Category::LanguageServers && self.servers.is_none() {
            self.reload_servers(cx);
        }
        let p = &self.palette;
        div()
            .id("settings")
            .track_focus(&self.focus)
            .key_context("Settings")
            .flex()
            .size_full()
            .bg(hsla(p.content))
            .child(self.sidebar(window, cx))
            .child(div().w(px(1.)).h_full().flex_none().bg(hsla(p.hairline)))
            .child(self.content(window, cx))
    }
}

// MARK: Helpers

/// An element id for one setting's part.
fn named(part: &str, key: &str) -> ElementId {
    ElementId::Name(format!("{part}-{key}").into())
}

fn is_commit(event: &InputEvent) -> bool {
    matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur)
}

/// Put `text` in the field, unless it has the keyboard (someone's typing).
pub(crate) fn set_unless_editing(
    state: &Entity<InputState>,
    text: &str,
    window: &mut Window,
    cx: &mut App,
) {
    let field = state.read(cx);
    if field.value() == text || field.focus_handle(cx).is_focused(window) {
        return;
    }
    let text = text.to_string();
    state.update(cx, |state, cx| state.set_value(text, window, cx));
}

pub(crate) fn text_input(
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
}

/// Save a field's text with `apply` on Return or when focus leaves.
fn commit_on<V: 'static>(
    state: &Entity<InputState>,
    cx: &mut Context<V>,
    apply: impl Fn(String, &mut Settings) + 'static,
) -> Subscription {
    cx.subscribe(state, move |_, state, event: &InputEvent, cx| {
        if is_commit(event) {
            let text = state.read(cx).value().to_string();
            SettingsStore::update(cx, |s| apply(text, s));
        }
    })
}

/// A rounded-border `NSTextField` with mono text (the Swift settings' and
/// project setup's fields): 22 high, the text field color.
pub(crate) fn mono_field(state: &Entity<InputState>) -> TextField {
    TextField {
        state: state.clone(),
        mono: true,
        style: gpui_kit::StyleRefinement::default(),
    }
}

/// The same with the UI font.
pub(crate) fn text_field(state: &Entity<InputState>) -> TextField {
    TextField {
        mono: false,
        ..mono_field(state)
    }
}

/// A pop-up button's frame (Swift's `Picker`): filled, 22 high, with
/// NSPopUpButton's up-and-down chevrons at its trailing edge in the label
/// color (GPUI Kit's `Select` draws its icon extra small in the muted
/// color, so `popup` leaves that slot empty and the frame draws them); the
/// `Select` inside it goes without its own look.
pub(crate) fn popup_frame(p: &ChromePalette) -> gpui_kit::Div {
    div()
        .relative()
        .h(px(22.))
        .flex()
        .items_center()
        .rounded(px(5.))
        .bg(hsla(if p.is_light {
            Rgba::BLACK.opacity(0.06)
        } else {
            Rgba::WHITE.opacity(0.075)
        }))
        // Lucide's chevrons fill 10 by 16 of its 24: at 16 points they're
        // SF Symbols' 7 by 11, their trailing edge 9 in from the frame's.
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .right(px(4.))
                .flex()
                .items_center()
                .child(icon("chevrons-up-down", 16., p.text)),
        )
}

/// An icon that draws nothing (`popup`'s chevron slot).
const EMPTY_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"></svg>"#;

/// The `Select` for a `popup_frame`.
pub(crate) fn popup<D>(state: &Entity<SelectState<D>>) -> Select<D>
where
    D: gpui_kit::component::select::SelectDelegate + 'static,
{
    Select::new(state)
        .small()
        .appearance(false)
        // An empty slot where `popup_frame` draws the chevrons.
        .icon(gpui_kit::component::Icon::empty().data(EMPTY_SVG))
        .w_full()
        .h_full()
        .text_size(px(13.))
}

#[derive(IntoElement)]
pub(crate) struct TextField {
    state: Entity<InputState>,
    mono: bool,
    style: gpui_kit::StyleRefinement,
}

impl Styled for TextField {
    fn style(&mut self) -> &mut gpui_kit::StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for TextField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        use gpui_kit::component::ActiveTheme as _;
        let focused = self.state.read(cx).focus_handle(cx).is_focused(window);
        let theme = cx.theme();
        let background = if theme.is_dark() {
            gpui_kit::rgb(0x171717)
        } else {
            gpui_kit::rgb(0xffffff)
        };
        let mut field = div()
            .h(px(22.))
            .flex()
            .items_center()
            .px(px(6.))
            // `.roundedBorder` on macOS 26: 6-point corners and a faint
            // hairline (`ChromePalette.hairline`, GPUI Kit's `border`).
            .rounded(px(metrics::RADIUS))
            .border_1()
            // The hairline turns into the focus ring (`ChromePalette`'s,
            // GPUI Kit's `ring`).
            .border_color(if focused { theme.ring } else { theme.border })
            .bg(background)
            .child(
                Input::new(&self.state)
                    .small()
                    .appearance(false)
                    .px_0()
                    .when(self.mono, |input| {
                        input.font_family(MONO_FONT).text_size(px(12.))
                    }),
            );
        gpui_kit::Refineable::refine(field.style(), &self.style);
        field
    }
}

/// A 22pt icon button (the Swift `ChromeIconButton`).
pub(crate) fn icon_button(
    id: impl Into<gpui_kit::ElementId>,
    name: &'static str,
    help: &'static str,
    p: &ChromePalette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(22.))
        .rounded(px(metrics::RADIUS_SMALL))
        .cursor_pointer()
        .hover(|s| s.bg(hsla(p.hover)))
        .tooltip(move |window, cx| Tooltip::new(help).build(window, cx))
        .child(icon(name, 12., p.text_secondary))
}

/// "+ Add" in the accent color.
pub(crate) fn add_button(id: &'static str, p: &ChromePalette) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(4.))
        .text_size(px(11.5))
        .text_color(hsla(p.accent))
        .cursor_pointer()
        .child(icon("plus", 11., p.accent))
        .child("Add")
}

/// A setting's value as its field shows it.
fn field_text(item: &SettingItem, settings: &Settings) -> String {
    let value = settings.get(item.key).unwrap_or(Value::Null);
    match &item.control {
        Control::Integer { zero_label, .. } => {
            let number = value.as_i64().unwrap_or(0);
            if number == 0 && zero_label.is_some() {
                String::new()
            } else {
                number.to_string()
            }
        }
        Control::Decimal { .. } => format!("{:.1}", value.as_f64().unwrap_or(0.0)),
        _ => value.as_str().unwrap_or_default().to_string(),
    }
}

/// A select's options and the value to select: the catalog's choices, the
/// themes, or the monospaced fonts, plus the current value when it's none
/// of them (an unknown value from settings.json stays selectable).
/// What a choice's pop-up shows for `current`.
fn choice_title(item: &SettingItem, current: &str) -> String {
    match &item.control {
        Control::Choice(options) => options
            .iter()
            .find(|(value, _)| *value == current)
            .map_or(current, |(_, label)| *label)
            .to_string(),
        Control::Theme => impulse_kit::theme::theme_display_name(current),
        _ => current.to_string(),
    }
}

fn choices_for(item: &SettingItem, current: &str, cx: &App) -> (Vec<Choice>, String) {
    let (mut choices, selected) = match &item.control {
        Control::Choice(options) => (
            options
                .iter()
                .map(|(value, label)| Choice::new(*value, *label))
                .collect::<Vec<_>>(),
            current.to_string(),
        ),
        Control::Theme => {
            let themes = impulse_kit::theme::available_themes();
            // Names typed by hand ("Tokyo_Night") select their theme.
            let normalize = |s: &str| s.to_lowercase().replace(['_', ' '], "-");
            let selected = themes
                .iter()
                .find(|t| normalize(t) == normalize(current))
                .cloned()
                .unwrap_or_else(|| current.to_string());
            (
                themes
                    .iter()
                    .map(|id| Choice::new(id.clone(), impulse_kit::theme::theme_display_name(id)))
                    .collect(),
                selected,
            )
        }
        _ => (
            monospace_fonts(cx)
                .into_iter()
                .map(|family| Choice::new(family.clone(), family))
                .collect(),
            current.to_string(),
        ),
    };
    if !choices.iter().any(|c| c.value == selected) {
        choices.insert(0, Choice::new(selected.clone(), selected.clone()));
    }
    (choices, selected)
}

/// Monospaced font families installed (and bundled): every family whose
/// narrow and wide glyphs advance alike. Worked out once.
fn monospace_fonts(cx: &App) -> Vec<String> {
    thread_local! {
        static FONTS: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
    }
    if let Some(fonts) = FONTS.with(|f| f.borrow().clone()) {
        return fonts;
    }
    let system = cx.text_system();
    let size = px(16.);
    let fonts: Vec<String> = system
        .all_font_names()
        .into_iter()
        .filter(|name| !name.starts_with('.') && !name.is_empty())
        .filter(|name| {
            let id = system.resolve_font(&gpui_kit::font(name.clone()));
            let width = |c| system.advance(id, size, c).map(|s| s.width).ok();
            match (width('i'), width('W'), width('.')) {
                (Some(i), Some(w), Some(dot)) => i > px(0.) && i == w && i == dot,
                _ => false,
            }
        })
        .collect();
    FONTS.with(|f| *f.borrow_mut() = Some(fonts.clone()));
    fonts
}

/// Shell-style split for a "command arg arg" field (quotes group words).
pub(crate) fn split_command_line(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut in_token = false;
    for c in text.chars() {
        if let Some(open) = quote {
            if c == open {
                quote = None;
            } else {
                current.push(c);
            }
        } else if c == '"' || c == '\'' {
            quote = Some(c);
            in_token = true;
        } else if c.is_whitespace() {
            if in_token {
                parts.push(std::mem::take(&mut current));
            }
            in_token = false;
        } else {
            current.push(c);
            in_token = true;
        }
    }
    if in_token {
        parts.push(current);
    }
    parts
}

/// The inverse of `split_command_line`, for display.
pub(crate) fn join_command_line(parts: &[String]) -> String {
    if parts.iter().all(String::is_empty) {
        return String::new();
    }
    parts
        .iter()
        .map(|p| {
            if p.contains(' ') || p.is_empty() {
                format!("\"{p}\"")
            } else {
                p.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// MARK: Opening the tool tabs

impl Workbench {
    /// Show the active workspace's tool tab that `is_tool` picks, if it has
    /// one (a tool is one per workspace, as `TabManager.openTool` finds it).
    pub(crate) fn reveal_tool(
        &mut self,
        is_tool: impl Fn(&Surface) -> bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Surface> {
        let workspace = self.workspace_mut();
        let (index, pane) = workspace
            .tabs
            .iter()
            .enumerate()
            .find_map(|(i, tab)| tab.pane_of(&is_tool).map(|pane| (i, pane)))?;
        workspace.active = index;
        workspace.tabs[index].focused = pane;
        let surface = workspace.tabs[index].panes[&pane].clone();
        self.notice = None;
        self.focus_active(window, cx);
        cx.notify();
        Some(surface)
    }

    /// Settings as a tab, searching for `query` when given.
    pub(crate) fn open_settings(
        &mut self,
        query: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = match self.reveal_tool(|s| matches!(s, Surface::Settings(_)), window, cx) {
            Some(Surface::Settings(view)) => view,
            _ => {
                let palette = self.palette.clone();
                let view = cx.new(|cx| SettingsView::new(palette, window, cx));
                let index = self.active;
                self.add_tab(index, Surface::Settings(view.clone()), window, cx);
                view
            }
        };
        view.update(cx, |view, cx| match query {
            Some(query) => view.reveal(query, window, cx),
            None => view.focus(window, cx),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_and_joins_command_lines() {
        assert_eq!(
            split_command_line("swiftlint --fix \"{file}\" 'a b'"),
            vec!["swiftlint", "--fix", "{file}", "a b"]
        );
        assert_eq!(split_command_line("  "), Vec::<String>::new());
        assert_eq!(split_command_line("x \"\""), vec!["x", ""]);
        assert_eq!(
            join_command_line(&["fmt".into(), "a b".into(), String::new()]),
            "fmt \"a b\" \"\""
        );
    }
}
