//! The Keyboard Shortcuts tab, as the Swift `KeybindingsSurface` draws it:
//! every command with its shortcut, search by name or by pressing the keys,
//! click a shortcut to record a new one, conflicts flagged, reset per
//! command, and shortcuts that run a command line in the terminal. Every
//! change is saved into `keybinding_overrides` (or `custom_keybindings`),
//! and `commands::rebind_on_change` binds it right away.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::Sizable as _;
use gpui_kit::{
    div, prelude::*, px, AnyElement, App, Context, Entity, FocusHandle, FontWeight, Keystroke,
    KeystrokeEvent, MouseButton, SharedString, Subscription, Window,
};
use impulse_kit::chrome::{metrics, ChromePalette};
use impulse_kit::keybindings::{self, Builtin, Platform, Shortcut, BUILTINS, UNBOUND};
use impulse_kit::settings::CustomKeybinding;

use crate::settings_store::SettingsStore;
use crate::settings_view::{add_button, icon_button, mono_field, set_unless_editing, MONO_FONT};
use crate::workbench::Workbench;
use crate::workbench_chrome::{hsla, icon};
use crate::workspace::Surface;

/// What the next key press is recorded for.
const SEARCH: &str = "search";

/// Shortcuts that can't be changed: on the Mac the standard menu items
/// (and the Window menu's Tab 1 to 9), elsewhere the editing keys every
/// text field and the editor keep, Quit and Tab 1 to 9. A command given one is flagged.
fn fixed_shortcuts() -> Vec<(String, String)> {
    let fixed: &[(&str, &str)] = if cfg!(target_os = "macos") {
        &[
            ("Hide Impulse", "Cmd+H"),
            ("Hide Others", "Alt+Cmd+H"),
            ("Quit Impulse", "Cmd+Q"),
            ("Open\u{2026}", "Cmd+O"),
            ("Close Window", "Shift+Cmd+W"),
            ("Undo", "Cmd+Z"),
            ("Redo", "Shift+Cmd+Z"),
            ("Cut", "Cmd+X"),
            ("Paste and Match Style", "Alt+Shift+Cmd+V"),
            ("Select All", "Cmd+A"),
            ("Minimize", "Cmd+M"),
            ("Impulse Help", "Shift+Cmd+?"),
        ]
    } else {
        &[
            ("Undo", "Ctrl+Z"),
            ("Redo", "Ctrl+Shift+Z"),
            ("Cut", "Ctrl+X"),
            ("Select All", "Ctrl+A"),
            ("Quit Impulse", "Ctrl+Shift+Q"),
        ]
    };
    let mut out: Vec<(String, String)> = fixed
        .iter()
        .map(|(name, key)| (name.to_string(), key.to_string()))
        .collect();
    let platform = impulse_kit::keybindings::Platform::current();
    out.extend((1..=9).filter_map(|n| {
        let key = impulse_kit::menus::fixed_shortcut(impulse_kit::menus::Fixed::Tab(n), platform)?;
        Some((format!("Tab {n}"), key))
    }));
    out
}

/// The command line a custom shortcut runs, as typed.
fn command_line(custom: &CustomKeybinding) -> String {
    let shell = impulse_kit::shell::login::default_shell_type();
    std::iter::once(custom.command.trim().to_string())
        .chain(
            custom
                .args
                .iter()
                .map(|a| impulse_kit::shell::quoting::quoted(a, shell)),
        )
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The shortcut a key press makes; `None` for a key that can't be one.
fn shortcut_from(keystroke: &Keystroke) -> Option<Shortcut> {
    let key = keystroke.key.as_str();
    // Forward delete would read as Backspace ("delete" in Impulse's names).
    if key.is_empty()
        || matches!(
            key,
            "shift" | "control" | "alt" | "platform" | "function" | "capslock" | "delete" | "fn"
        )
    {
        return None;
    }
    let m = &keystroke.modifiers;
    let shortcut = Shortcut {
        ctrl: m.control,
        alt: m.alt,
        shift: m.shift,
        cmd: m.platform,
        key: Shortcut::parse(key)?.key,
    };
    // Only what can be bound.
    gpui_kit::Keystroke::parse(&shortcut.gpui())
        .is_ok()
        .then_some(shortcut)
}

fn is_function_key(key: &str) -> bool {
    key.strip_prefix('f')
        .and_then(|n| n.parse::<u32>().ok())
        .is_some_and(|n| (1..=12).contains(&n))
}

/// A row of the Run in Terminal list.
struct CustomRow {
    name: Entity<InputState>,
    command: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

pub struct KeybindingsView {
    pub(crate) palette: ChromePalette,
    pub(crate) focus: FocusHandle,
    search: Entity<InputState>,
    /// The shortcut typed into the search (display form).
    key_filter: Option<String>,
    /// What the next key press is recorded for: a command id, "custom:N",
    /// or "search".
    recording: Option<String>,
    /// Why the last key pressed while recording wasn't taken; recording
    /// goes on.
    notice: Option<String>,
    custom: Vec<CustomRow>,
    _subscriptions: Vec<Subscription>,
}

impl KeybindingsView {
    pub fn new(palette: ChromePalette, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search commands")
                .clean_on_escape()
        });
        let view = cx.entity().downgrade();
        let subscriptions = vec![
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.observe_global_in::<SettingsStore>(window, |view, window, cx| {
                view.sync(window, cx);
                cx.notify();
            }),
            // The keyboard moving elsewhere stops recording.
            cx.on_blur(&focus, window, |view, _, cx| {
                if view.recording.take().is_some() {
                    cx.notify();
                }
            }),
            // While recording, the next key press is the shortcut, before
            // any binding sees it.
            cx.intercept_keystrokes(move |event, window, cx| {
                if let Some(view) = view.upgrade() {
                    view.update(cx, |view, cx| view.intercept(event, window, cx));
                }
            }),
        ];
        let mut view = KeybindingsView {
            palette,
            focus,
            search,
            key_filter: None,
            recording: None,
            notice: None,
            custom: Vec::new(),
            _subscriptions: subscriptions,
        };
        view.sync(window, cx);
        view
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let custom = SettingsStore::get(cx).custom_keybindings.clone();
        self.custom.truncate(custom.len());
        while self.custom.len() < custom.len() {
            let index = self.custom.len();
            let name = crate::settings_view::text_input("Name", window, cx);
            let command = crate::settings_view::text_input("Command", window, cx);
            let commit = |state: &Entity<InputState>,
                          cx: &mut Context<Self>,
                          apply: fn(&mut CustomKeybinding, String)| {
                cx.subscribe(state, move |_, state, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        let text = state.read(cx).value().to_string();
                        SettingsStore::update(cx, |s| {
                            if let Some(c) = s.custom_keybindings.get_mut(index) {
                                apply(c, text);
                            }
                        });
                    }
                })
            };
            let subscriptions = vec![
                commit(&name, cx, |c, text| c.name = text),
                // The whole command line, kept as typed (the shell reads it).
                commit(&command, cx, |c, text| {
                    let line = text.trim().to_string();
                    if line != command_line(c) {
                        c.command = line;
                        c.args = Vec::new();
                    }
                }),
            ];
            self.custom.push(CustomRow {
                name,
                command,
                _subscriptions: subscriptions,
            });
        }
        for (row, binding) in self.custom.iter().zip(&custom) {
            set_unless_editing(&row.name, &binding.name, window, cx);
            set_unless_editing(&row.command, &command_line(binding), window, cx);
        }
    }

    // MARK: Recording

    fn start_recording(&mut self, target: String, window: &mut Window, cx: &mut Context<Self>) {
        self.recording = if self.recording.as_deref() == Some(target.as_str()) {
            None
        } else {
            Some(target)
        };
        self.notice = None;
        // Recording takes the keyboard from text fields and other panes.
        if self.recording.is_some() {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    fn intercept(&mut self, event: &KeystrokeEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.recording.is_none() {
            return;
        }
        if !self.focus.is_focused(window) {
            self.recording = None;
            cx.notify();
            return;
        }
        cx.stop_propagation();
        self.record(&event.keystroke, cx);
        cx.notify();
    }

    /// Esc cancels; Backspace alone removes the shortcut; anything with a
    /// key becomes it. A key that can't be a shortcut is refused with the
    /// reason, and recording goes on.
    fn record(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        let Some(target) = self.recording.clone() else {
            return;
        };
        let m = &keystroke.modifiers;
        let plain = !m.control && !m.alt && !m.shift && !m.platform;
        let platform = Platform::current();
        if keystroke.key == "escape" && plain {
            self.recording = None;
            return;
        }
        if target == SEARCH {
            self.key_filter = shortcut_from(keystroke).map(|s| s.display(platform));
            self.recording = None;
            return;
        }
        if keystroke.key == "backspace" && plain {
            set_shortcut(None, &target, cx);
            self.recording = None;
            return;
        }
        let Some(shortcut) = shortcut_from(keystroke) else {
            self.notice = Some("That key can't be used in a shortcut".into());
            return;
        };
        // A plain key (or Shift + key) would swallow that character
        // everywhere you type; only function keys stand alone.
        if !(m.control || m.alt || m.platform) && !is_function_key(&shortcut.key) {
            let keys = shortcut.display(platform);
            self.notice = Some(if cfg!(target_os = "macos") {
                format!(
                    "{keys} needs \u{2318}, \u{2303} or \u{2325} (only F1\u{2013}F12 work alone)"
                )
            } else {
                format!("{keys} needs Ctrl, Alt or Super (only F1\u{2013}F12 work alone)")
            });
            return;
        }
        set_shortcut(Some(shortcut), &target, cx);
        self.recording = None;
    }

    // MARK: Model

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().trim().to_lowercase()
    }

    /// Commands shown, in their categories.
    fn groups(&self, cx: &App) -> Vec<(&'static str, Vec<(usize, &'static Builtin)>)> {
        let overrides = &SettingsStore::get(cx).keybinding_overrides;
        let platform = Platform::current();
        let query = self.query(cx);
        let mut groups: Vec<(&'static str, Vec<(usize, &'static Builtin)>)> = Vec::new();
        for (index, binding) in BUILTINS.iter().enumerate() {
            let shown =
                keybindings::display_for(binding.id, overrides, platform).unwrap_or_default();
            if self.key_filter.as_ref().is_some_and(|k| *k != shown) {
                continue;
            }
            if !query.is_empty()
                && !(binding.description.to_lowercase().contains(&query)
                    || binding.id.contains(&query)
                    || binding.category.to_lowercase().contains(&query)
                    || shown.to_lowercase().contains(&query))
            {
                continue;
            }
            match groups.iter_mut().find(|(c, _)| *c == binding.category) {
                Some((_, items)) => items.push((index, binding)),
                None => groups.push((binding.category, vec![(index, binding)])),
            }
        }
        groups
    }

    /// Shared shortcuts (by display form), including the ones that can't
    /// be changed.
    fn conflicts(&self, cx: &App) -> std::collections::BTreeMap<String, Vec<String>> {
        let settings = SettingsStore::get(cx);
        let extra: Vec<(String, String)> = settings
            .custom_keybindings
            .iter()
            .enumerate()
            .map(|(i, c)| (format!("custom:{i}"), c.key.clone()))
            .chain(
                fixed_shortcuts()
                    .into_iter()
                    .enumerate()
                    .map(|(i, (_, key))| (format!("fixed:{i}"), key)),
            )
            .collect();
        keybindings::conflicts(&settings.keybinding_overrides, &extra, Platform::current())
    }

    fn display_name(id: &str, cx: &App) -> String {
        if let Some(index) = id
            .strip_prefix("custom:")
            .and_then(|i| i.parse::<usize>().ok())
        {
            return SettingsStore::get(cx)
                .custom_keybindings
                .get(index)
                .map(|c| {
                    if c.name.is_empty() {
                        "Custom shortcut".to_string()
                    } else {
                        c.name.clone()
                    }
                })
                .unwrap_or_else(|| id.to_string());
        }
        if let Some(index) = id
            .strip_prefix("fixed:")
            .and_then(|i| i.parse::<usize>().ok())
        {
            if let Some((name, _)) = fixed_shortcuts().get(index) {
                return format!("{name} (can't be changed)");
            }
        }
        keybindings::builtin(id)
            .map(|b| b.description.to_string())
            .unwrap_or_else(|| id.to_string())
    }

    // MARK: Drawing

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        let recording_search = self.recording.as_deref() == Some(SEARCH);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .h(px(52.))
            .px(px(28.))
            .bg(hsla(p.panel))
            .border_b_1()
            .border_color(hsla(p.hairline))
            .child(
                div()
                    .flex_1()
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(hsla(p.text))
                    .child("Keyboard Shortcuts"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(28.))
                    .pl(px(8.))
                    .rounded(px(metrics::RADIUS))
                    .bg(hsla(p.raised))
                    .child(icon("search", 12., p.text_tertiary))
                    .children(self.key_filter.clone().map(|filter| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .child(
                                div()
                                    .font_family(MONO_FONT)
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(hsla(p.accent))
                                    .child(filter),
                            )
                            .child(
                                div()
                                    .id("clear-key-filter")
                                    .cursor_pointer()
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.key_filter = None;
                                        cx.notify();
                                    }))
                                    .child(icon("x", 10., p.text_tertiary)),
                            )
                    }))
                    .child(
                        div()
                            .w(px(200.))
                            .text_size(px(12.5))
                            .child(Input::new(&self.search).appearance(false).small()),
                    ),
            )
            .child(
                div()
                    .id("search-by-keys")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _, window, cx| {
                            cx.stop_propagation();
                            view.start_recording(SEARCH.into(), window, cx);
                        }),
                    )
                    .child(
                        Button::new("search-by-keys-button")
                            .label(if recording_search {
                                "Press keys\u{2026}"
                            } else {
                                "Search by keys"
                            })
                            .small()
                            .when(recording_search, |b| b.primary()),
                    ),
            )
    }

    /// The recorded (or recording) shortcut; click to record.
    fn chip(
        &self,
        id: impl Into<gpui_kit::ElementId>,
        target: String,
        shortcut: Option<String>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = &self.palette;
        let recording = self.recording.as_deref() == Some(target.as_str());
        let notice = recording
            .then(|| self.notice.clone())
            .flatten()
            .map(|n| format!("{n}. Press other keys (esc cancels)"));
        let unbound = shortcut.is_none();
        let text = notice.clone().unwrap_or_else(|| {
            if recording {
                if cfg!(target_os = "macos") {
                    "Press keys\u{2026} (\u{232b} removes, esc cancels)".into()
                } else {
                    "Press keys\u{2026} (Backspace removes, Esc cancels)".into()
                }
            } else {
                shortcut.unwrap_or_else(|| "Unbound".into())
            }
        });
        let color = if notice.is_some() {
            p.warning
        } else if recording {
            p.accent
        } else if unbound {
            p.text_tertiary
        } else {
            p.text
        };
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .px(px(8.))
            .min_w(px(70.))
            .min_h(px(24.))
            .rounded(px(5.))
            .border_1()
            .cursor_pointer()
            .bg(hsla(if recording {
                p.accent.opacity(0.14)
            } else {
                p.raised
            }))
            .border_color(hsla(if recording { p.accent } else { p.hairline }))
            .text_color(hsla(color))
            .when(recording || unbound, |d| d.text_size(px(11.5)))
            .when(!(recording || unbound), |d| {
                d.font_family(MONO_FONT)
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
            })
            .tooltip(|window, cx| {
                Tooltip::new("Click, then press the new shortcut").build(window, cx)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, _, window, cx| {
                    cx.stop_propagation();
                    view.start_recording(target.clone(), window, cx);
                }),
            )
            .child(text)
    }

    fn conflict_badge(
        &self,
        id: impl Into<gpui_kit::ElementId>,
        own: &str,
        shown: Option<&String>,
        conflicts: &std::collections::BTreeMap<String, Vec<String>>,
        cx: &App,
    ) -> Option<AnyElement> {
        let others: Vec<String> = conflicts
            .get(shown?)?
            .iter()
            .filter(|other| *other != own)
            .map(|other| Self::display_name(other, cx))
            .collect();
        if others.is_empty() {
            return None;
        }
        let help: SharedString = format!("Also bound to {}", others.join(", ")).into();
        Some(
            div()
                .id(id)
                .flex_none()
                .tooltip(move |window, cx| Tooltip::new(help.clone()).build(window, cx))
                .child(icon("triangle-alert", 12., self.palette.warning))
                .into_any_element(),
        )
    }

    fn command_row(
        &self,
        index: usize,
        binding: &'static Builtin,
        conflicts: &std::collections::BTreeMap<String, Vec<String>>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = &self.palette;
        let overrides = &SettingsStore::get(cx).keybinding_overrides;
        let shown = keybindings::display_for(binding.id, overrides, Platform::current());
        let modified = overrides.contains_key(binding.id);
        let id = binding.id;
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .py(px(6.))
            .child(
                div()
                    .w(px(2.))
                    .h(px(22.))
                    .flex_none()
                    .when(modified, |d| d.bg(hsla(p.accent))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .flex_1()
                    .min_w_0()
                    // SF Pro's and SF Mono's lines at these sizes.
                    .child(
                        div()
                            .text_size(px(12.5))
                            .line_height(px(15.))
                            .text_color(hsla(p.text))
                            .child(binding.description),
                    )
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(px(10.))
                            .line_height(px(13.))
                            .text_color(hsla(p.text_tertiary))
                            .child(binding.id),
                    ),
            )
            .children(self.conflict_badge(("conflict", index), id, shown.as_ref(), conflicts, cx))
            .child(self.chip(("chip", index), id.to_string(), shown, cx))
            .child(
                icon_button(("reset", index), "rotate-ccw", "Reset to default", p)
                    .when(!modified, |d| d.invisible())
                    .on_click(move |_, _, cx| {
                        SettingsStore::update(cx, |s| {
                            s.keybinding_overrides.remove(id);
                        })
                    }),
            )
    }

    fn custom_section(
        &self,
        conflicts: &std::collections::BTreeMap<String, Vec<String>>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = self.palette.clone();
        let platform = Platform::current();
        let custom = SettingsStore::get(cx).custom_keybindings.clone();
        let mut out = vec![
            div()
                .flex()
                .items_center()
                .pt(px(22.))
                .pb(px(4.))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(hsla(p.text_tertiary))
                        .child(crate::workbench_chrome::tracked("RUN IN TERMINAL")),
                )
                .child(add_button("add-custom", &p).on_click(|_, _, cx| {
                    SettingsStore::update(cx, |s| {
                        s.custom_keybindings.push(CustomKeybinding {
                            name: "New shortcut".into(),
                            ..Default::default()
                        })
                    })
                }))
                .into_any_element(),
            div()
                .pb(px(6.))
                .text_size(px(11.5))
                .text_color(hsla(p.text_secondary))
                .child(
                    "Shortcuts that run a command line in the focused terminal, or in a new \
                     terminal tab when it's busy or the input bar is off.",
                )
                .into_any_element(),
        ];
        for (index, (row, binding)) in self.custom.iter().zip(&custom).enumerate() {
            let target = format!("custom:{index}");
            let shown = Shortcut::parse(&binding.key).map(|s| s.display(platform));
            out.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .py(px(6.))
                    .child(mono_field(&row.name).w(px(150.)).flex_none())
                    .child(mono_field(&row.command).flex_1().min_w_0())
                    .children(self.conflict_badge(
                        ("custom-conflict", index),
                        &target,
                        shown.as_ref(),
                        conflicts,
                        cx,
                    ))
                    .child(self.chip(("custom-chip", index), target, shown, cx))
                    .child(
                        icon_button(("remove-custom", index), "trash-2", "Remove", &p).on_click(
                            move |_, _, cx| {
                                SettingsStore::update(cx, |s| {
                                    if index < s.custom_keybindings.len() {
                                        s.custom_keybindings.remove(index);
                                    }
                                })
                            },
                        ),
                    )
                    .into_any_element(),
            );
            out.push(div().h(px(1.)).bg(hsla(p.hairline)).into_any_element());
        }
        out
    }
}

/// Save `shortcut` for `target` (a command id or "custom:N"); `None`
/// removes it. A command's own default is stored as no override.
fn set_shortcut(shortcut: Option<Shortcut>, target: &str, cx: &mut App) {
    SettingsStore::update(cx, |settings| {
        if let Some(index) = target
            .strip_prefix("custom:")
            .and_then(|i| i.parse::<usize>().ok())
        {
            if let Some(custom) = settings.custom_keybindings.get_mut(index) {
                custom.key = shortcut.map(|s| s.text()).unwrap_or_default();
            }
            return;
        }
        let Some(builtin) = keybindings::builtin(target) else {
            return;
        };
        let default = keybindings::default_shortcut(builtin, Platform::current());
        if shortcut == default {
            settings.keybinding_overrides.remove(target);
        } else {
            settings.keybinding_overrides.insert(
                target.to_string(),
                shortcut
                    .map(|s| s.text())
                    .unwrap_or_else(|| UNBOUND.to_string()),
            );
        }
    });
}

impl Render for KeybindingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette.clone();
        let conflicts = self.conflicts(cx);
        let mut list = div()
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(860.))
            .px(px(28.))
            .pb(px(24.));
        for (category, items) in self.groups(cx) {
            list = list.child(
                div()
                    .pt(px(16.))
                    .pb(px(4.))
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(hsla(p.text_tertiary))
                    .child(crate::workbench_chrome::tracked(&category.to_uppercase())),
            );
            for (index, binding) in items {
                list = list
                    .child(self.command_row(index, binding, &conflicts, cx))
                    .child(div().h(px(1.)).bg(hsla(p.hairline)));
            }
        }
        list = list.children(self.custom_section(&conflicts, cx));
        div()
            .id("keybindings")
            .track_focus(&self.focus)
            .key_context("Keybindings")
            .flex()
            .flex_col()
            .size_full()
            .bg(hsla(p.content))
            // A click anywhere but the recording chip stops recording.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| {
                    if view.recording.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .child(self.header(cx))
            .child(
                div()
                    .id("keybindings-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(list),
            )
    }
}

impl Workbench {
    /// Keyboard Shortcuts as a tab (one per workspace).
    pub(crate) fn open_keybindings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .reveal_tool(|s| matches!(s, Surface::Keybindings(_)), window, cx)
            .is_some()
        {
            return;
        }
        let palette = self.palette.clone();
        let view = cx.new(|cx| KeybindingsView::new(palette, window, cx));
        let index = self.active;
        self.add_tab(index, Surface::Keybindings(view), window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keystroke(text: &str) -> Keystroke {
        Keystroke::parse(text).unwrap()
    }

    #[test]
    fn key_presses_become_shortcuts() {
        let s = shortcut_from(&keystroke("ctrl-shift-k")).unwrap();
        assert_eq!(s.text(), "Ctrl+Shift+K");
        assert_eq!(
            shortcut_from(&keystroke("ctrl-enter")).unwrap().key,
            "return"
        );
        assert_eq!(shortcut_from(&keystroke("delete")), None);
        assert!(is_function_key("f5"));
        assert!(!is_function_key("f13"));
        assert!(!is_function_key("a"));
    }

    #[test]
    fn fixed_shortcuts_parse() {
        for (_, key) in fixed_shortcuts() {
            assert!(Shortcut::parse(&key).is_some(), "{key}");
        }
    }
}
