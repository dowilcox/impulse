//! The Project Setup tab (the Swift `ProjectSetupSurface` and
//! `MainWindowController+ProjectSetup`): what a repository's tasks need
//! (files to copy, folders and a database to clone, ports and values of
//! their own, scripts, actions), proposed from what's in the repository by
//! `impulse_kit::project::detector` and saved into the main checkout's
//! `.impulse/project.toml` with `settings_file`, which every task reads.
//! Saving trusts what it wrote (unless the file then holds settings the
//! screen didn't show) and removes a local `.git/impulse/project.toml`
//! Impulse wrote, whose settings the screen folded in. Try in a New Task
//! saves and opens New Task; Re-apply writes the values into the tasks
//! that already exist. The model is `project_setup_model`.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::{Sizable as _, WindowExt as _};
use gpui_kit::{
    div, prelude::*, px, AnyElement, App, ClickEvent, Context, Entity, FocusHandle, FontWeight,
    ScrollHandle, SharedString, Subscription, WeakEntity, Window,
};
use impulse_git::{branches, ignored};
use impulse_kit::chrome::ChromePalette;
use impulse_kit::project::config::{self, Landing, LoadError, ProjectConfig, RELATIVE_PATH};
use impulse_kit::project::{compose_file, detector, settings_file, task_environment};

use crate::project_setup_model::{
    byte_count, folder_size, list, saved_settings, Database, Field, LocalFile, Pair, SetupModel,
};
use crate::settings_view::{icon_button, mono_field, MONO_FONT};
use crate::tasks::{self, toast, Then, ToastKind};
use crate::workbench::Workbench;
use crate::workbench_chrome::{chrome_button, hsla, icon, ButtonKind};
use crate::workspace::{Kind, Surface};

/// The local file, as the notes name it.
const LOCAL_PATH: &str = ".git/impulse/project.toml";

/// The scroll view's children, in order: a section is revealed by its
/// place here.
const SECTIONS: [&str; 12] = [
    "notes",
    "copy",
    "clone",
    "database",
    "ports",
    "values",
    "scripts",
    "finish",
    "on_change",
    "overlap",
    "actions",
    "try",
];

pub struct ProjectSetupView {
    pub palette: ChromePalette,
    pub(crate) focus: FocusHandle,
    workbench: WeakEntity<Workbench>,
    this: WeakEntity<ProjectSetupView>,
    pub(crate) model: SetupModel,
    lines: HashMap<Field, (Entity<InputState>, Subscription)>,
    areas: HashMap<Field, (Entity<TextareaState>, Subscription)>,
    scroll: ScrollHandle,
}

/// The placeholder of a field.
fn placeholder(field: Field) -> &'static str {
    match field {
        Field::CopyPath(_) => "config/local.json",
        Field::ClonePath(_) => "storage/app",
        Field::PortName(_) => "APP_PORT",
        Field::PortValue(_) => "8000",
        Field::ValueName(_) => "DB_DATABASE",
        Field::ValueValue(_) => "myapp_{task_}",
        Field::RuleName(_) => "composer.lock",
        Field::RuleCommand(_) => "composer install",
        Field::ActionName(_) => "Name",
        Field::ActionCommand(_) => "Command",
        Field::PortOffset => "100",
        Field::OverlapIgnore => "CHANGELOG.md, *.snap",
        Field::DatabaseFolder => "docker/data/mysql",
        Field::DatabaseService => "mysql",
        Field::Setup | Field::Check | Field::Archive => "",
    }
}

impl ProjectSetupView {
    pub fn new(
        palette: ChromePalette,
        workbench: WeakEntity<Workbench>,
        cx: &mut Context<Self>,
    ) -> Self {
        ProjectSetupView {
            palette,
            focus: cx.focus_handle(),
            workbench,
            this: cx.entity().downgrade(),
            model: SetupModel::default(),
            lines: HashMap::new(),
            areas: HashMap::new(),
            scroll: ScrollHandle::new(),
        }
    }

    pub fn is_modified(&self) -> bool {
        self.model.is_modified()
    }

    /// Scroll to a section ("actions") once it's drawn.
    pub fn reveal(&mut self, section: &str, cx: &mut Context<Self>) {
        if let Some(index) = SECTIONS.iter().position(|s| *s == section) {
            self.scroll.scroll_to_top_of_item(index);
            cx.notify();
        }
    }

    /// Every field the model has rows for, with a text field each: those of
    /// rows that went are dropped.
    fn ensure_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let m = &self.model;
        let mut lines = vec![
            Field::PortOffset,
            Field::OverlapIgnore,
            Field::DatabaseFolder,
            Field::DatabaseService,
        ];
        lines.extend(
            m.copies
                .iter()
                .filter(|r| r.is_custom)
                .map(|r| Field::CopyPath(r.id)),
        );
        lines.extend(
            m.clones
                .iter()
                .filter(|r| r.is_custom)
                .map(|r| Field::ClonePath(r.id)),
        );
        for p in &m.ports {
            lines.extend([Field::PortName(p.id), Field::PortValue(p.id)]);
        }
        for p in &m.values {
            lines.extend([Field::ValueName(p.id), Field::ValueValue(p.id)]);
        }
        for p in &m.rules {
            lines.extend([Field::RuleName(p.id), Field::RuleCommand(p.id)]);
        }
        for a in &m.actions {
            lines.extend([Field::ActionName(a.id), Field::ActionCommand(a.id)]);
        }
        self.lines.retain(|field, _| lines.contains(field));
        for field in lines {
            if self.lines.contains_key(&field) {
                continue;
            }
            let value = self.model.text(field).unwrap_or_default();
            let state = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder(field))
                    .default_value(value)
            });
            let subscription = cx.subscribe(&state, move |view, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = state.read(cx).value().to_string();
                    if view.model.text(field).as_ref() != Some(&value) {
                        view.model.set_text(field, value);
                        cx.notify();
                    }
                }
            });
            self.lines.insert(field, (state, subscription));
        }
        for field in [Field::Setup, Field::Check, Field::Archive] {
            if self.areas.contains_key(&field) {
                continue;
            }
            let value = self.model.text(field).unwrap_or_default();
            let state = cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(1, 4)
                    .default_value(value)
            });
            let subscription = cx.subscribe(&state, move |view, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = state.read(cx).value().to_string();
                    if view.model.text(field).as_ref() != Some(&value) {
                        view.model.set_text(field, value);
                        cx.notify();
                    }
                }
            });
            self.areas.insert(field, (state, subscription));
        }
    }

    /// Show the model's texts in the fields, after the model changed other
    /// than by typing (a reload, a suggestion used, a database chosen).
    pub(crate) fn sync_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lines: Vec<(Field, Entity<InputState>)> = self
            .lines
            .iter()
            .map(|(f, (s, _))| (*f, s.clone()))
            .collect();
        for (field, state) in lines {
            let Some(text) = self.model.text(field) else {
                continue;
            };
            if state.read(cx).value() != text {
                state.update(cx, |s, cx| s.set_value(text, window, cx));
            }
        }
        let areas: Vec<(Field, Entity<TextareaState>)> = self
            .areas
            .iter()
            .map(|(f, (s, _))| (*f, s.clone()))
            .collect();
        for (field, state) in areas {
            let Some(text) = self.model.text(field) else {
                continue;
            };
            if state.read(cx).value() != text {
                state.update(cx, |s, cx| s.set_value(text, window, cx));
            }
        }
        cx.notify();
    }

    /// A click that changes the model, then shows it in the fields.
    fn change(
        &self,
        f: impl Fn(&mut SetupModel) + 'static,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        let view = self.this.clone();
        move |_, window, cx| {
            let _ = view.update(cx, |view, cx| {
                f(&mut view.model);
                view.sync_fields(window, cx);
            });
        }
    }

    /// A click that runs on the workbench with this view.
    fn act(
        &self,
        f: impl Fn(&mut Workbench, Entity<ProjectSetupView>, &mut Window, &mut Context<Workbench>)
            + 'static,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        let (workbench, view) = (self.workbench.clone(), self.this.clone());
        move |_, window, cx| {
            let Some(view) = view.upgrade() else { return };
            let _ = workbench.update(cx, |workbench, cx| f(workbench, view, window, cx));
        }
    }

    fn line(&self, field: Field) -> AnyElement {
        match self.lines.get(&field) {
            Some((state, _)) => mono_field(state).into_any_element(),
            None => div().into_any_element(),
        }
    }

    /// A field in the interface font (an action's name).
    fn ui_line(&self, field: Field) -> AnyElement {
        match self.lines.get(&field) {
            Some((state, _)) => Input::new(state).small().into_any_element(),
            None => div().into_any_element(),
        }
    }

    fn area(&self, field: Field) -> AnyElement {
        match self.areas.get(&field) {
            Some((state, _)) => div()
                .font_family(MONO_FONT)
                .text_size(px(12.))
                .child(Textarea::new(state).small())
                .into_any_element(),
            None => div().into_any_element(),
        }
    }
}

// MARK: Drawing

/// One of the screen's buttons: Swift's `ChromeButton`, secondary unless
/// made `ghost` or `primary`, drawn by `on_click`.
struct SetupButton {
    id: gpui_kit::ElementId,
    label: SharedString,
    icon: Option<&'static str>,
    kind: ButtonKind,
    disabled: bool,
    tooltip: Option<SharedString>,
    palette: ChromePalette,
}

impl SetupButton {
    fn ghost(mut self) -> Self {
        self.kind = ButtonKind::Ghost;
        self
    }

    fn primary(mut self) -> Self {
        self.kind = ButtonKind::Primary;
        self
    }

    fn icon(mut self, name: &'static str) -> Self {
        self.icon = Some(name);
        self
    }

    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    fn on_click(
        self,
        handler: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let disabled = self.disabled;
        chrome_button(
            self.id,
            self.label,
            self.icon,
            self.kind,
            disabled,
            &self.palette,
        )
        .when_some(self.tooltip, |b, text| {
            b.tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(text.clone()).build(window, cx)
            })
        })
        .when(!disabled, |b| b.on_click(handler))
    }
}

impl ProjectSetupView {
    fn button(
        &self,
        id: impl Into<gpui_kit::ElementId>,
        label: impl Into<SharedString>,
    ) -> SetupButton {
        SetupButton {
            id: id.into(),
            label: label.into(),
            icon: None,
            kind: ButtonKind::Secondary,
            disabled: false,
            tooltip: None,
            palette: self.palette.clone(),
        }
    }

    fn caption(&self, text: impl Into<String>) -> gpui_kit::Div {
        div()
            .text_size(px(11.))
            .text_color(hsla(self.palette.text_tertiary))
            .whitespace_normal()
            .child(text.into())
    }

    fn problem(&self, message: Option<String>) -> Option<gpui_kit::Div> {
        message.map(|m| {
            div()
                .text_size(px(11.))
                .text_color(hsla(self.palette.danger))
                .whitespace_normal()
                .child(m)
        })
    }

    fn note(&self, text: String) -> gpui_kit::Div {
        let p = &self.palette;
        div()
            .flex()
            .items_start()
            .gap(px(6.))
            .flex_1()
            .min_w_0()
            .text_size(px(11.5))
            .text_color(hsla(p.text_secondary))
            .child(div().pt(px(2.)).child(icon("info", 12., p.text_secondary)))
            .child(div().flex_1().min_w_0().whitespace_normal().child(text))
    }

    fn section(
        &self,
        id: &'static str,
        title: &str,
        detail: String,
        content: Vec<AnyElement>,
    ) -> AnyElement {
        let p = &self.palette;
        div()
            .id(id)
            .flex()
            .flex_col()
            .gap(px(8.))
            .max_w(px(764.))
            .pb(px(22.))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(hsla(p.text))
                    .child(title.to_string()),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(hsla(p.text_secondary))
                    .whitespace_normal()
                    .child(detail),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .pt(px(2.))
                    .children(content),
            )
            .into_any_element()
    }

    fn subheading(&self, title: String, detail: String) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .pt(px(4.))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(title),
            )
            .child(self.caption(detail))
            .into_any_element()
    }

    /// A suggestion offered beside a field, not filled into it.
    fn suggestion(
        &self,
        id: (&'static str, usize),
        text: String,
        label: &'static str,
        apply: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        let p = &self.palette;
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .min_w_0()
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(hsla(p.text_tertiary))
                    .child("Suggested:"),
            )
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .font_family(MONO_FONT)
                    .text_size(px(11.))
                    .text_color(hsla(p.text_secondary))
                    .child(text),
            )
            .child(self.button(id, label).ghost().on_click(apply))
            .into_any_element()
    }

    /// The suggested pairs whose names `list` lacks, offered to add.
    fn suggested(
        &self,
        id: &'static str,
        found: BTreeMap<String, String>,
        list: impl Fn(&mut SetupModel) -> &mut Vec<Pair> + 'static,
        current: &[Pair],
    ) -> Option<AnyElement> {
        let names: Vec<&str> = current.iter().map(|p| p.name.trim()).collect();
        let missing: Vec<(String, String)> = found
            .into_iter()
            .filter(|(name, _)| !names.contains(&name.as_str()))
            .collect();
        if missing.is_empty() {
            return None;
        }
        let text = missing
            .iter()
            .map(|(n, v)| format!("{n} = {v}"))
            .collect::<Vec<_>>()
            .join(", ");
        let apply = self.change(move |m| {
            for (name, value) in &missing {
                let pair = m.pair(name, value);
                list(m).push(pair);
            }
        });
        Some(self.suggestion((id, 0), text, "Add", apply))
    }

    fn header(&self) -> AnyElement {
        let p = &self.palette;
        let m = &self.model;
        let problems = m.problem_count();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .h(px(58.))
            .px(px(20.))
            .bg(hsla(p.panel))
            .border_b_1()
            .border_color(hsla(p.hairline))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(hsla(p.text))
                            .child(format!("Project Setup for {}", m.name())),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(hsla(p.text_secondary))
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(format!(
                                "What new tasks get: files, folders, ports and values of their own, \
                                 and scripts. Saved in {RELATIVE_PATH}."
                            )),
                    ),
            )
            .child(
                self.button("setup-open-file", "Open File")
                    .ghost()
                    .tooltip(format!("Open {RELATIVE_PATH} in the editor"))
                    .on_click(self.act(|w, view, window, cx| {
                        let root = view.read(cx).model.root.clone();
                        w.open_project_settings_file(&root, window, cx)
                    })),
            )
            .when(problems > 0, |d| {
                d.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(hsla(p.danger))
                        .child(if problems == 1 {
                            "1 entry to fix".to_string()
                        } else {
                            format!("{problems} entries to fix")
                        }),
                )
            })
            .child(
                self.button(
                    "setup-save",
                    if m.is_saving { "Saving…" } else { "Save" },
                )
                .primary()
                .icon("check")
                .disabled(m.is_loading || m.is_saving || m.load_error.is_some() || problems > 0)
                .on_click(self.act(|w, view, window, cx| {
                    w.save_project_setup(view, false, None, window, cx)
                })),
            )
            .into_any_element()
    }

    /// In place of the settings when the file doesn't parse.
    fn unreadable(&self, message: String) -> AnyElement {
        let p = &self.palette;
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .max_w(px(764.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(icon("triangle-alert", 14., p.warning))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{RELATIVE_PATH} can't be read")),
                    ),
            )
            .child(self.caption(
                "New tasks get none of its settings until it's fixed, and this screen can't show or \
                 save them.",
            ))
            .child(
                div()
                    .font_family(MONO_FONT)
                    .text_size(px(11.5))
                    .text_color(hsla(p.text_secondary))
                    .whitespace_normal()
                    .child(message),
            )
            .child(
                div()
                    .flex()
                    .gap(px(10.))
                    .child(self.button("setup-unreadable-open", "Open File").primary().on_click(
                        self.act(|w, view, window, cx| {
                            let root = view.read(cx).model.root.clone();
                            w.open_project_settings_file(&root, window, cx)
                        }),
                    ))
                    .child(self.button("setup-reload", "Reload").on_click(self.act(
                        |w, view, window, cx| {
                            let root = view.read(cx).model.root.clone();
                            w.load_project_setup(view, root, None, window, cx)
                        },
                    ))),
            )
            .into_any_element()
    }

    fn notes(&self) -> AnyElement {
        let mut notes: Vec<AnyElement> = Vec::new();
        let open_local = || {
            self.button("setup-open-local", "Open")
                .ghost()
                .on_click(self.act(|w, view, window, cx| {
                    let root = view.read(cx).model.root.clone();
                    if let Some(common) = branches::common_git_directory(&root) {
                        w.open_file(Path::new(&ProjectConfig::local_path(&common)), window, cx);
                    }
                }))
        };
        match &self.model.local_file {
            Some(LocalFile::Impulses { dropped, .. }) => notes.push(
                self.note(format!(
                    "Settings kept on this computer only, in {LOCAL_PATH}, are included here. Saving \
                     moves them into {RELATIVE_PATH} and removes that file{}",
                    if dropped.is_empty() {
                        ".".to_string()
                    } else {
                        format!(
                            "; where the two differed ({}), the project's are shown.",
                            list(dropped)
                        )
                    }
                ))
                .into_any_element(),
            ),
            Some(LocalFile::HandEdited { overrides }) => notes.push(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .child(self.note(format!(
                        "On this computer, {LOCAL_PATH} {}. It was edited by hand, so saving leaves \
                         it alone.",
                        if overrides.is_empty() {
                            "has settings of its own".to_string()
                        } else {
                            format!("overrides the project's {}", list(overrides))
                        }
                    )))
                    .child(open_local())
                    .into_any_element(),
            ),
            Some(LocalFile::Unreadable(message)) => notes.push(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .child(self.note(format!(
                        "On this computer, {LOCAL_PATH} can't be read, so new tasks get none of the \
                         project's settings until it's fixed: {message}"
                    )))
                    .child(open_local())
                    .into_any_element(),
            ),
            None => {}
        }
        if let Some(dropped) = self.model.dropped() {
            notes.push(
                self.note(format!(
                    "Saving rewrites the sections this screen manages in {RELATIVE_PATH}, dropping \
                     what it doesn't show there: {dropped}."
                ))
                .into_any_element(),
            );
        }
        div()
            .id("notes")
            .flex()
            .flex_col()
            .gap(px(8.))
            .max_w(px(764.))
            .when(!notes.is_empty(), |d| d.pb(px(22.)))
            .children(notes)
            .into_any_element()
    }

    /// Detected paths as checkboxes; the user's own as fields to edit,
    /// pick or remove.
    fn paths(&self, clones: bool, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = &self.palette;
        let rows = if clones {
            &self.model.clones
        } else {
            &self.model.copies
        };
        let tag = if clones { "clone" } else { "copy" };
        let mut elements = Vec::new();
        for row in rows {
            let id = row.id;
            let toggle = cx.listener(move |view, checked: &bool, _, cx| {
                let rows = if clones {
                    &mut view.model.clones
                } else {
                    &mut view.model.copies
                };
                if let Some(row) = rows.iter_mut().find(|r| r.id == id) {
                    row.is_on = *checked;
                }
                cx.notify();
            });
            let element = if row.is_custom {
                let field = if clones {
                    Field::ClonePath(id)
                } else {
                    Field::CopyPath(id)
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        Checkbox::new((tag, id as usize))
                            .checked(row.is_on)
                            .on_change(toggle),
                    )
                    .child(div().flex_1().child(self.line(field)))
                    .child(
                        icon_button(
                            ("path-choose", id as usize),
                            "folder-open",
                            if clones {
                                "Choose a folder"
                            } else {
                                "Choose a file"
                            },
                            p,
                        )
                        .on_click(self.act(move |w, view, window, cx| {
                            w.choose_project_path(view, field, clones, window, cx)
                        })),
                    )
                    .child(
                        icon_button(("path-remove", id as usize), "trash-2", "Remove", p).on_click(
                            self.change(move |m| {
                                m.copies.retain(|r| r.id != id);
                                m.clones.retain(|r| r.id != id);
                            }),
                        ),
                    )
            } else {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .min_w_0()
                    .child(
                        div().font_family(MONO_FONT).child(
                            Checkbox::new((tag, id as usize))
                                .label(row.path.clone())
                                .checked(row.is_on)
                                .on_change(toggle),
                        ),
                    )
                    .children(row.size.clone().map(|size| {
                        div()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(hsla(p.text_tertiary))
                            .child(size)
                    }))
                    .children(row.note.clone().map(|note| {
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_size(px(11.))
                            .text_color(hsla(p.text_tertiary))
                            .child(note)
                    }))
            };
            elements.push(element.into_any_element());
        }
        elements
    }

    /// Rows of name and value, each with its problem under it.
    fn pairs(
        &self,
        kind: &'static str,
        pairs: &[Pair],
        problem: impl Fn(&SetupModel, &Pair) -> Option<String>,
        fields: impl Fn(u64) -> (Field, Field),
    ) -> Vec<AnyElement> {
        let p = &self.palette;
        pairs
            .iter()
            .map(|pair| {
                let id = pair.id;
                let (name, value) = fields(id);
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().w(px(200.)).flex_none().child(self.line(name)))
                            .child(div().flex_1().child(self.line(value)))
                            .child(
                                icon_button((kind, id as usize), "trash-2", "Remove", p).on_click(
                                    self.change(move |m| {
                                        m.values.retain(|p| p.id != id);
                                        m.rules.retain(|p| p.id != id);
                                    }),
                                ),
                            ),
                    )
                    .children(self.problem(problem(&self.model, pair)))
                    .into_any_element()
            })
            .collect()
    }

    fn script(
        &self,
        field: Field,
        title: &'static str,
        help: &'static str,
        suggested: Option<String>,
    ) -> AnyElement {
        let p = &self.palette;
        let current = self.model.text(field).unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .gap(px(3.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(hsla(p.text_tertiary))
                            .child(help),
                    ),
            )
            .child(self.area(field))
            .children(
                suggested
                    .filter(|_| current.trim().is_empty())
                    .map(|suggested| {
                        let text = suggested.clone();
                        let apply = self.change(move |m| m.set_text(field, text.clone()));
                        self.suggestion(("script-use", field_index(field)), suggested, "Use", apply)
                    }),
            )
            .into_any_element()
    }

    fn copy_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut content = Vec::new();
        if self.model.copies.is_empty() {
            content.push(self.caption("No ignored files found.").into_any_element());
        }
        content.extend(self.paths(false, cx));
        content.push(
            div()
                .flex()
                .child(
                    self.button("add-copy", "Add File")
                        .ghost()
                        .icon("plus")
                        .on_click(self.change(|m| {
                            let row = m.custom_row("");
                            m.copies.push(row);
                        })),
                )
                .into_any_element(),
        );
        self.section(
            "copy",
            "Copy into tasks",
            "Files a fresh checkout lacks (ignored or untracked), copied from the main checkout. \
             Patterns work: config/*.local.json."
                .into(),
            content,
        )
    }

    fn clone_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut content = Vec::new();
        if self.model.clones.is_empty() {
            content.push(
                self.caption("No ignored dependency or build folders found.")
                    .into_any_element(),
            );
        }
        content.extend(self.paths(true, cx));
        content.push(
            div()
                .flex()
                .child(
                    self.button("add-clone", "Add Folder")
                        .ghost()
                        .icon("plus")
                        .on_click(self.change(|m| {
                            let row = m.custom_row("");
                            m.clones.push(row);
                        })),
                )
                .into_any_element(),
        );
        self.section(
            "clone",
            "Clone into tasks",
            "Folders cloned in at once, with no extra disk until they change.".into(),
            content,
        )
    }

    fn database_section(&self) -> AnyElement {
        let p = &self.palette;
        let m = &self.model;
        let summary = if m.dump_command.is_some() {
            "Each task's own stack starts with an empty database unless it gets one."
        } else {
            "What a task's database starts with."
        };
        let mut content = Vec::new();
        for option in m.database_options.clone() {
            let chosen = m.database == option;
            let (title, detail) = match option {
                Database::Clone => (
                    "Clone a data folder".to_string(),
                    "A folder holding the database's data is cloned from the main checkout into \
                     each task."
                        .to_string(),
                ),
                Database::Dump => (
                    "Dump and load".into(),
                    "The setup script loads a dump of the main checkout's database into the task's."
                        .into(),
                ),
                Database::Empty => (
                    "Empty, with migrations and seed data".into(),
                    format!(
                        "The setup script runs {}.",
                        m.empty_command.as_deref().unwrap_or("the migrations")
                    ),
                ),
                Database::None => (
                    "Nothing".into(),
                    "Each task starts with an empty database.".into(),
                ),
            };
            content.push(
                div()
                    .id(("database", option as usize))
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .cursor_pointer()
                    .on_click(self.change(move |m| m.choose(option)))
                    .child(div().pt(px(1.)).child(icon(
                        if chosen { "circle-dot" } else { "circle" },
                        13.,
                        if chosen { p.accent } else { p.text_tertiary },
                    )))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(title),
                            )
                            .child(self.caption(detail)),
                    )
                    .into_any_element(),
            );
            if option == Database::Clone && chosen {
                content.push(self.database_fields().into_any_element());
            }
        }
        self.section("database", "Database", summary.into(), content)
    }

    fn database_fields(&self) -> gpui_kit::Div {
        let p = &self.palette;
        let label = |text: &'static str| {
            div()
                .w(px(52.))
                .flex_none()
                .text_size(px(11.5))
                .text_color(hsla(p.text_secondary))
                .child(text)
        };
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .pl(px(22.))
            .pb(px(4.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(label("Folder"))
                    .child(div().flex_1().child(self.line(Field::DatabaseFolder)))
                    .child(
                        icon_button("database-choose", "folder-open", "Choose a folder", p)
                            .on_click(self.act(|w, view, window, cx| {
                                w.choose_project_path(view, Field::DatabaseFolder, true, window, cx)
                            })),
                    ),
            )
            .children(
                self.problem(self.model.database_problem())
                    .map(|d| d.pl(px(60.))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(label("Service"))
                    .child(
                        div()
                            .w(px(160.))
                            .flex_none()
                            .child(self.line(Field::DatabaseService)),
                    )
                    .child(self.caption(
                        "Optional: the Compose service that writes it, stopped in the main checkout \
                         while it's cloned.",
                    )),
            )
    }

    fn ports_section(&self) -> AnyElement {
        let p = &self.palette;
        let m = &self.model;
        let offset = m.offset();
        let has_compose = !m.fixed_ports.is_empty() || !m.container_names.is_empty();
        let mut content = Vec::new();
        if has_compose {
            content.extend(self.compose_ports());
            content.push(self.subheading(
                format!("In {}", m.env_file),
                format!(
                    "Variables each task gets its own value of, in its {}. Compose ports written as \
                     ${{NAME:-8000}} come from here.",
                    m.env_file
                ),
            ));
        } else {
            content.push(
                self.caption(format!(
                    "Each task gets its own value of these in its {}.",
                    m.env_file
                ))
                .into_any_element(),
            );
        }
        if m.ports.is_empty() && m.found.as_ref().is_none_or(|f| f.ports.is_empty()) {
            content.push(
                self.caption(
                    "None found. Add the variables your app or Compose file reads its ports from.",
                )
                .into_any_element(),
            );
        }
        for pair in &m.ports {
            let id = pair.id;
            let task_one = pair
                .value
                .trim()
                .parse::<i64>()
                .ok()
                .filter(|v| task_environment::PORT_RANGE.contains(v))
                .and_then(|v| task_environment::port(v, 1, offset))
                .map(|v| format!("task 1: {v}"))
                .unwrap_or_default();
            content.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .w(px(200.))
                                    .flex_none()
                                    .child(self.line(Field::PortName(id))),
                            )
                            .child(
                                div()
                                    .w(px(90.))
                                    .flex_none()
                                    .child(self.line(Field::PortValue(id))),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(11.))
                                    .text_color(hsla(p.text_tertiary))
                                    .child(task_one),
                            )
                            .child(
                                icon_button(("port-remove", id as usize), "trash-2", "Remove", p)
                                    .on_click(self.change(move |m| m.ports.retain(|p| p.id != id))),
                            ),
                    )
                    .children(self.problem(m.problem_port(pair)))
                    .into_any_element(),
            );
        }
        let found = m
            .found
            .as_ref()
            .map(|f| {
                f.ports
                    .iter()
                    .map(|(k, v)| (k.clone(), v.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        content.extend(self.suggested("ports-add", found, |m| &mut m.ports, &m.ports));
        content.push(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    self.button("add-port", "Add Port")
                        .ghost()
                        .icon("plus")
                        .on_click(self.change(|m| {
                            let pair = m.pair("", "");
                            m.ports.push(pair);
                        })),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(hsla(p.text_secondary))
                        .child("Offset per task"),
                )
                .child(div().w(px(70.)).child(self.line(Field::PortOffset)))
                .into_any_element(),
        );
        if let Some(problem) = self.problem(m.offset_problem()) {
            content.push(div().flex().justify_end().child(problem).into_any_element());
        }
        self.section(
            "ports",
            "Ports",
            format!(
                "Two checkouts can't listen on the same port, so task n moves each one up by n × \
                 {offset}: task 1 gets {} for 8000.",
                8000 + offset
            ),
            content,
        )
    }

    /// The Compose file's fixed ports and container names, which a task's
    /// override moves and renames.
    fn compose_ports(&self) -> Vec<AnyElement> {
        let p = &self.palette;
        let m = &self.model;
        let file = m
            .compose_file_name
            .clone()
            .unwrap_or_else(|| "the Compose file".into());
        let mut content = vec![self.subheading(
            format!("In {file}"),
            "Written as plain numbers, so only one checkout's stack can run at a time.".into(),
        )];
        let row = |service: &str, value: String, task: String| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .w(px(110.))
                        .flex_none()
                        .text_size(px(12.))
                        .text_color(hsla(p.text_secondary))
                        .child(service.to_string()),
                )
                .child(
                    div()
                        .font_family(MONO_FONT)
                        .text_size(px(11.5))
                        .child(value),
                )
                .when(m.compose_override, |d| {
                    d.child(
                        div()
                            .text_size(px(11.))
                            .text_color(hsla(p.text_tertiary))
                            .child(format!("task 1: {task}")),
                    )
                })
                .into_any_element()
        };
        for fixed in &m.fixed_ports {
            content.push(row(
                &fixed.service,
                fixed.port.raw.clone(),
                compose_file::moved(&fixed.port, 1, m.offset())
                    .unwrap_or_else(|| "past 65535".into()),
            ));
        }
        for container in &m.container_names {
            content.push(row(
                &container.service,
                format!("container_name {}", container.name),
                format!("{}-<task>", container.name),
            ));
        }
        content.push(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    Checkbox::new("compose-override")
                        .label("Move them in each task")
                        .checked(m.compose_override)
                        .on_change(self.act_bool(|m, on| m.compose_override = on)),
                )
                .child(
                    self.button("show-compose", "Show File")
                        .ghost()
                        .on_click(self.act(|w, view, window, cx| {
                            let m = &view.read(cx).model;
                            if let Some(name) = &m.compose_file_name {
                                let path = Path::new(&m.root).join(name);
                                w.open_file(&path, window, cx);
                            }
                        })),
                )
                .into_any_element(),
        );
        if m.compose_override {
            content.push(
                self.caption(format!(
                    "{file} itself isn't changed. Each task gets an override file (in .git/impulse, \
                     never committed) that moves these ports and renames the containers, and its {} \
                     sets COMPOSE_FILE so docker compose uses it. Needs Docker Compose 2.24 or later.",
                    m.env_file
                ))
                .into_any_element(),
            );
        } else {
            content.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(icon("triangle-alert", 12., p.warning))
                    .child(self.caption(
                        "Off: a task's stack can't start while another checkout's is running.",
                    ))
                    .into_any_element(),
            );
        }
        content
    }

    fn act_bool(
        &self,
        set: impl Fn(&mut SetupModel, bool) + 'static,
    ) -> impl Fn(&bool, &mut Window, &mut App) + 'static {
        let view = self.this.clone();
        move |checked, _, cx| {
            let _ = view.update(cx, |v, cx| {
                set(&mut v.model, *checked);
                cx.notify();
            });
        }
    }

    fn values_section(&self) -> AnyElement {
        let m = &self.model;
        let mut content = self.pairs("value-remove", &m.values, SetupModel::problem_value, |id| {
            (Field::ValueName(id), Field::ValueValue(id))
        });
        let found = m
            .found
            .as_ref()
            .map(|f| f.values.clone())
            .unwrap_or_default();
        content.extend(self.suggested("values-add", found, |m| &mut m.values, &m.values));
        content.push(
            div()
                .flex()
                .child(
                    self.button("add-value", "Add Value")
                        .ghost()
                        .icon("plus")
                        .on_click(self.change(|m| {
                            let pair = m.pair("", "");
                            m.values.push(pair);
                        })),
                )
                .into_any_element(),
        );
        self.section(
            "values",
            "Values for each task",
            format!(
                "Written into each new task's {}. Use {{task}}, {{task_}}, {{slot}} and port names.",
                m.env_file
            ),
            content,
        )
    }

    fn scripts_section(&self) -> AnyElement {
        let found = self.model.found.clone().unwrap_or_default();
        let content = vec![
            self.script(
                Field::Setup,
                "Setup",
                "Runs in a new task's first terminal, before the agent",
                found.setup,
            ),
            self.script(
                Field::Check,
                "Check",
                "Checks a task is ready to land (types, tests)",
                found.check,
            ),
            self.script(
                Field::Archive,
                "Archive",
                "Runs before a task's folder is removed",
                found.archive,
            ),
        ];
        self.section(
            "scripts",
            "Scripts",
            "Commands run in tasks: once trusted, and asked again whenever they change.".into(),
            content,
        )
    }

    fn finish_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.palette;
        let landing = self.model.landing;
        let options: [(Option<Landing>, &str); 3] = [
            (None, "Ask the first time"),
            (Some(Landing::Merge), "Merge and push"),
            (Some(Landing::Review), "Push for review"),
        ];
        let mut bar = crate::workbench_chrome::segmented(p);
        for (i, (option, label)) in options.into_iter().enumerate() {
            let selected = landing == option;
            bar = bar.child(
                crate::workbench_chrome::segment(("landing", i), label, selected, p).on_click(
                    cx.listener(move |view, _, _, cx| {
                        view.model.landing = option;
                        cx.notify();
                    }),
                ),
            );
        }
        let caption = match landing {
            None => "The first Finish in this repository asks, and remembers the answer.",
            Some(Landing::Review) => {
                "Finish pushes the branch; you open a merge request on your git host. Once it's \
                 merged there, Impulse offers to clean up."
            }
            Some(Landing::Merge) => {
                "Finish merges the branch into its base with a merge commit, made in a throwaway \
                 folder, and pushes the base. The main checkout isn't touched."
            }
        };
        self.section(
            "finish",
            "Finishing tasks",
            "How Finish Task lands a task's branch.".into(),
            vec![
                div().flex().child(bar).into_any_element(),
                self.caption(caption).into_any_element(),
            ],
        )
    }

    fn rules_section(&self) -> AnyElement {
        let m = &self.model;
        let mut content = self.pairs("rule-remove", &m.rules, SetupModel::problem_rule, |id| {
            (Field::RuleName(id), Field::RuleCommand(id))
        });
        let found = m
            .found
            .as_ref()
            .map(|f| f.on_change.clone())
            .unwrap_or_default();
        content.extend(self.suggested("rules-add", found, |m| &mut m.rules, &m.rules));
        content.push(
            div()
                .flex()
                .child(
                    self.button("add-rule", "Add Rule")
                        .ghost()
                        .icon("plus")
                        .on_click(self.change(|m| {
                            let pair = m.pair("", "");
                            m.rules.push(pair);
                        })),
                )
                .into_any_element(),
        );
        self.section(
            "on_change",
            "When files change",
            "After a pull, merge or checkout brings in a change to one of these files, Impulse \
             offers to run its command."
                .into(),
            content,
        )
    }

    fn actions_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let m = &self.model;
        let mut content = Vec::new();
        if m.actions.is_empty() {
            content.push(self.caption("No actions yet.").into_any_element());
        }
        for row in &m.actions {
            let id = row.id;
            content.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                Checkbox::new(("action-on", id as usize))
                                    .checked(row.is_on)
                                    .on_change(cx.listener(move |view, checked: &bool, _, cx| {
                                        if let Some(row) =
                                            view.model.actions.iter_mut().find(|a| a.id == id)
                                        {
                                            row.is_on = *checked;
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .w(px(160.))
                                    .flex_none()
                                    .child(self.ui_line(Field::ActionName(id))),
                            )
                            .child(div().flex_1().child(self.line(Field::ActionCommand(id)))),
                    )
                    .children(self.problem(m.problem_action(row)).map(|d| d.pl(px(22.))))
                    .into_any_element(),
            );
        }
        content.push(
            div()
                .flex()
                .child(
                    self.button("add-action", "Add Action")
                        .ghost()
                        .icon("plus")
                        .on_click(self.change(|m| {
                            let id = m.new_id();
                            m.actions.push(crate::project_setup_model::ActionRow {
                                id,
                                is_on: true,
                                action: config::Action::new("", ""),
                            });
                        })),
                )
                .into_any_element(),
        );
        self.section(
            "actions",
            "Actions",
            "Commands in the palette's a: list.".into(),
            content,
        )
    }

    fn overlap_section(&self) -> AnyElement {
        self.section(
            "overlap",
            "Ignore when tasks overlap",
            "Files that don't count when two workspaces change the same files: names, paths or \
             patterns, separated by commas."
                .into(),
            vec![self.line(Field::OverlapIgnore)],
        )
    }

    fn try_section(&self) -> AnyElement {
        let count = self.model.task_count;
        self.section(
            "try",
            "Try it",
            "Save, then see the settings work.".into(),
            vec![
                div()
                    .flex()
                    .gap(px(10.))
                    .child(
                        self.button("setup-try", "Try in a New Task")
                            .icon("git-branch-plus")
                            .on_click(self.act(|w, view, window, cx| w.try_project_setup(view, window, cx))),
                    )
                    .when(count > 0, |d| {
                        d.child(
                            self.button(
                                "setup-reapply",
                                format!(
                                    "Re-apply Values to {count} Task{}",
                                    if count == 1 { "" } else { "s" }
                                ),
                            )
                            .on_click(self.act(|w, view, window, cx| {
                                w.reapply_project_setup(view, window, cx)
                            })),
                        )
                    })
                    .into_any_element(),
                self.caption(
                    "Try saves the settings and opens New Task. Re-apply writes the current ports and \
                     values into the env files of tasks that already exist.",
                )
                .into_any_element(),
            ],
        )
    }
}

/// A distinct number per script field, for element ids.
fn field_index(field: Field) -> usize {
    match field {
        Field::Setup => 0,
        Field::Check => 1,
        _ => 2,
    }
}

impl Render for ProjectSetupView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_fields(window, cx);
        let p = self.palette.clone();
        let body: AnyElement = if self.model.is_loading {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .text_color(hsla(p.text_tertiary))
                .child(format!("Looking through {}…", self.model.name()))
                .into_any_element()
        } else if let Some(error) = self.model.load_error.clone() {
            div()
                .flex_1()
                .px(px(28.))
                .py(px(20.))
                .child(self.unreadable(error))
                .into_any_element()
        } else {
            let sections = vec![
                self.notes(),
                self.copy_section(cx),
                self.clone_section(cx),
                self.database_section(),
                self.ports_section(),
                self.values_section(),
                self.scripts_section(),
                self.finish_section(cx),
                self.rules_section(),
                self.overlap_section(),
                self.actions_section(cx),
                self.try_section(),
            ];
            div()
                .id("project-setup-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .px(px(28.))
                .py(px(20.))
                .flex()
                .flex_col()
                .children(sections)
                .into_any_element()
        };
        div()
            .id("project-setup")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .bg(hsla(p.content))
            .text_color(hsla(p.text))
            .child(self.header())
            .child(body)
    }
}

// MARK: Opening, loading and saving

/// What loading found, off the main thread.
struct Found {
    committed: Option<String>,
    unreadable: Option<String>,
    saved: (Option<ProjectConfig>, Option<LocalFile>),
    suggestions: detector::ProjectSuggestions,
    dump: Option<String>,
    earlier_dump: Option<String>,
    empty: Option<String>,
    task_count: usize,
}

fn look_through(root: &str, trusted: bool) -> Found {
    let common = branches::common_git_directory(root);
    let local = common
        .as_deref()
        .and_then(|c| std::fs::read_to_string(ProjectConfig::local_path(c)).ok());
    let committed = std::fs::read_to_string(Path::new(root).join(RELATIVE_PATH)).ok();
    let unreadable = committed
        .as_deref()
        .and_then(|text| match ProjectConfig::parse(text) {
            Err(LoadError::Invalid(message)) => Some(message),
            Ok(_) => None,
        });
    let saved = saved_settings(committed.as_deref(), local.as_deref());
    let suggestions = detector::suggest(root, &ignored::ignored_entries(root, trusted));
    let services = compose_file::find(root)
        .map(|(_, text)| compose_file::ComposeFile::parse(&text).services)
        .unwrap_or_default();
    let database = detector::database_service(&services);
    let image = database.and_then(|s| s.image.clone().map(|i| (s.name.clone(), i)));
    let task_count = tasks::registry(root)
        .map(|r| r.tasks.iter().filter(|t| t.slot.is_some()).count())
        .unwrap_or(0);
    Found {
        committed,
        unreadable,
        saved,
        suggestions,
        dump: image
            .as_ref()
            .and_then(|(name, image)| detector::dump_and_load(name, image)),
        earlier_dump: image
            .as_ref()
            .and_then(|(name, image)| detector::earlier_dump_and_load(name, image)),
        empty: detector::fresh_database(root).map(String::from),
        task_count,
    }
}

/// What saving did.
enum SaveOutcome {
    /// What was written, the local file's state after, and the digest to
    /// trust: `None` when the file reads as more than the screen saved.
    Saved {
        digest: Option<String>,
        text: String,
        local: Option<LocalFile>,
    },
    /// The file isn't what the screen read.
    Changed,
    Failed(String),
}

/// Write `config` into the main checkout's `.impulse/project.toml`, and
/// remove the local file when it still holds `removing_local`. Given
/// `unless_changed_from`, only while the file still holds that text
/// (`None` inside: there's still no file).
fn write_project_settings(
    config: &ProjectConfig,
    root: &str,
    removing_local: Option<&str>,
    unless_changed_from: Option<Option<String>>,
) -> SaveOutcome {
    let path = Path::new(root).join(RELATIVE_PATH);
    let existing = std::fs::read_to_string(&path).ok();
    if let Some(loaded) = unless_changed_from {
        if existing != loaded {
            return SaveOutcome::Changed;
        }
    }
    let text = settings_file::merging(config, existing.as_deref().unwrap_or(""));
    if let Err(error) = path
        .parent()
        .map(std::fs::create_dir_all)
        .unwrap_or(Ok(()))
        .and_then(|_| write_atomically(&path, text.as_bytes()))
    {
        return SaveOutcome::Failed(error.to_string());
    }
    let local_path = branches::common_git_directory(root).map(|c| ProjectConfig::local_path(&c));
    if let (Some(local), Some(local_path)) = (removing_local, &local_path) {
        if std::fs::read_to_string(local_path).ok().as_deref() == Some(local) {
            let _ = std::fs::remove_file(local_path);
        }
    }
    let local_text = local_path.and_then(|p| std::fs::read_to_string(p).ok());
    SaveOutcome::Saved {
        digest: settings_file::holds_only(config, &text).then(|| config::digest(text.as_bytes())),
        local: saved_settings(Some(&text), local_text.as_deref()).1,
        text,
    }
}

/// What the committed settings file of the repository at `root` is
/// trusted under: the main repository (as `tasks::trust_key` has it).
fn trust_key(root: &str) -> String {
    branches::common_git_directory(root)
        .and_then(|c| {
            Path::new(&c)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| root.to_string())
}

impl Workbench {
    /// Project Setup for the repository of the workspace at `index` (else
    /// the active one's), scrolled to `section` ("actions").
    pub(crate) fn open_project_setup(
        &mut self,
        index: Option<usize>,
        section: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let from_workspace = self
            .workspaces
            .get(index.unwrap_or(self.active))
            .filter(|w| w.kind == Kind::Folder)
            .and_then(|w| branches::repo_root(&w.root.to_string_lossy()));
        let repository =
            from_workspace.or_else(|| self.git.active.as_ref().map(|s| s.read(cx).root.clone()));
        let Some(repository) = repository else {
            return toast(
                window,
                cx,
                ToastKind::Info,
                "Open a folder in a git repository first.".into(),
                None,
                None,
            );
        };
        let root = tasks::main_checkout_root(repository.trim_end_matches('/'));
        self.open_project_setup_at(root, section, window, cx);
    }

    /// Project Setup for the repository whose main checkout is `root`.
    pub(crate) fn open_project_setup_at(
        &mut self,
        root: String,
        section: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = match self.reveal_tool(|s| matches!(s, Surface::ProjectSetup(_)), window, cx) {
            Some(Surface::ProjectSetup(view)) => view,
            _ => {
                let (palette, workbench) = (self.palette.clone(), cx.entity().downgrade());
                let view = cx.new(|cx| ProjectSetupView::new(palette, workbench, cx));
                cx.observe(&view, |_, _, cx| cx.notify()).detach();
                let index = self.active;
                self.add_tab(index, Surface::ProjectSetup(view.clone()), window, cx);
                view
            }
        };
        // Changes not saved yet stay: opening the tab again shows them, and
        // another repository's setup only replaces them once the user says
        // so.
        let (modified, same, name) = {
            let m = &view.read(cx).model;
            (m.is_modified(), m.root == root, m.name())
        };
        if !modified {
            return self.load_project_setup(view, root, section, window, cx);
        }
        if same {
            if let Some(section) = section {
                view.update(cx, |v, cx| v.reveal(section, cx));
            }
            return;
        }
        self.git_confirm(
            format!("Discard your changes to {name}'s setup?"),
            format!(
                "Project Setup shows one repository at a time, and the changes made to {name}'s \
                 aren't saved."
            ),
            "Discard Changes",
            true,
            window,
            cx,
            move |w, window, cx| {
                w.load_project_setup(view.clone(), root.clone(), section, window, cx)
            },
        );
    }

    /// Fill the tab from the repository at `root` and what's in it (off the
    /// main thread).
    pub(crate) fn load_project_setup(
        &mut self,
        view: Entity<ProjectSetupView>,
        root: String,
        section: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Reading the same repository again keeps the screen, and its
        // place, until the settings are in.
        view.update(cx, |v, cx| {
            if v.model.root != root || v.model.load_error.is_some() {
                v.model.is_loading = true;
            }
            v.model.root = root.clone();
            cx.notify();
        });
        let trusted = self.trust.is_trusted(&root);
        let work = {
            let root = root.clone();
            cx.background_executor()
                .spawn(async move { look_through(&root, trusted) })
        };
        cx.spawn_in(window, async move |workbench, cx| {
            let found = work.await;
            let _ = view.update_in(cx, |v, window, cx| {
                if v.model.root != root {
                    return;
                }
                let m = &mut v.model;
                m.loaded_text = found.committed.clone();
                m.load_error = found.unreadable.clone();
                if m.load_error.is_some() {
                    m.is_loading = false;
                    cx.notify();
                    return;
                }
                m.fill(
                    found.saved.0.as_ref(),
                    found.suggestions,
                    found.dump.clone(),
                    found.empty,
                );
                m.saved_config = Some(m.config());
                // 0.33's dump and load ran only in bash and zsh and could
                // wait forever.
                if let (Some(earlier), Some(dump)) = (&found.earlier_dump, &found.dump) {
                    if m.setup.contains(earlier.as_str()) {
                        m.setup = m.setup.replace(earlier.as_str(), dump);
                        m.database = Database::Dump;
                    }
                }
                m.local_file = found.saved.1;
                m.drops_comments = found
                    .committed
                    .as_deref()
                    .is_some_and(settings_file::managed_sections_have_comments);
                m.unknown_keys = found
                    .committed
                    .as_deref()
                    .map(settings_file::unknown_managed_keys)
                    .unwrap_or_default();
                m.task_count = found.task_count;
                m.is_loading = false;
                v.ensure_fields(window, cx);
                v.sync_fields(window, cx);
                if let Some(section) = section {
                    v.reveal(section, cx);
                }
            });
            let _ = workbench.update(cx, |w, cx| w.measure_clones(view, cx));
        })
        .detach();
    }

    /// Each clone row's size ("212 MB"), measured off the main thread.
    fn measure_clones(&mut self, view: Entity<ProjectSetupView>, cx: &mut Context<Self>) {
        let (root, paths): (String, Vec<String>) = {
            let m = &view.read(cx).model;
            (
                m.root.clone(),
                m.clones.iter().map(|r| r.path.clone()).collect(),
            )
        };
        if paths.is_empty() {
            return;
        }
        let work = cx.background_executor().spawn(async move {
            paths
                .into_iter()
                .map(|path| {
                    let size = byte_count(folder_size(&Path::new(&root).join(&path)));
                    (path, size)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |_, cx| {
            let sizes = work.await;
            view.update(cx, |v, cx| {
                for (path, size) in sizes {
                    if let Some(row) = v.model.clones.iter_mut().find(|r| r.path == path) {
                        row.size = Some(size);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Pick a file or folder inside the repository for a path field.
    fn choose_project_path(
        &mut self,
        view: Entity<ProjectSetupView>,
        field: Field,
        folders: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = view.read(cx).model.root.clone();
        let name = view.read(cx).model.name();
        let paths = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: !folders,
            directories: folders,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn_in(window, async move |workbench, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(chosen) = paths.into_iter().next() else {
                return;
            };
            let resolve = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
            let base = resolve(Path::new(&root));
            let chosen: PathBuf = resolve(&chosen);
            let _ = workbench.update_in(cx, |_, window, cx| {
                let Ok(relative) = chosen.strip_prefix(&base) else {
                    return toast(
                        window,
                        cx,
                        ToastKind::Warning,
                        format!("Choose something inside {name}."),
                        None,
                        None,
                    );
                };
                let relative = relative.to_string_lossy().into_owned();
                if relative.is_empty() {
                    return;
                }
                view.update(cx, |v, cx| {
                    v.model.set_text(field, relative);
                    v.sync_fields(window, cx);
                });
            });
        })
        .detach();
    }

    /// Save what the screen has into `.impulse/project.toml` and trust what
    /// was written when the file reads back as just that. Impulse's own
    /// local file, folded into the screen, is removed: its settings are in
    /// the project's now. A file changed since the screen read it is only
    /// written over once the user says so (`overwriting`). `then` hears
    /// whether the settings were saved.
    pub(crate) fn save_project_setup(
        &mut self,
        view: Entity<ProjectSetupView>,
        overwriting: bool,
        then: Option<Then<bool>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let finish = |w: &mut Workbench,
                      then: Option<Then<bool>>,
                      saved,
                      window: &mut Window,
                      cx: &mut Context<Workbench>| {
            if let Some(then) = then {
                then(w, saved, window, cx)
            }
        };
        let m = &view.read(cx).model;
        if m.load_error.is_some() || m.is_saving {
            return finish(self, then, false, window, cx);
        }
        if m.problem_count() > 0 {
            toast(
                window,
                cx,
                ToastKind::Warning,
                "Fix the entries marked in red first: they can't be saved as they are.".into(),
                None,
                None,
            );
            return finish(self, then, false, window, cx);
        }
        let root = m.root.clone();
        let config = m.config();
        let loaded = m.loaded_text.clone();
        let folded = match &m.local_file {
            Some(LocalFile::Impulses { text, .. }) => Some(text.clone()),
            _ => None,
        };
        let dropped = m.dropped();
        let write = {
            let view = view.clone();
            move |_: &mut Workbench,
                  then: Option<Then<bool>>,
                  window: &mut Window,
                  cx: &mut Context<Workbench>| {
                view.update(cx, |v, cx| {
                    v.model.is_saving = true;
                    cx.notify();
                });
                let work = {
                    let (config, root) = (config.clone(), root.clone());
                    cx.background_executor().spawn(async move {
                        let unless = (!overwriting).then_some(loaded);
                        write_project_settings(&config, &root, folded.as_deref(), unless)
                    })
                };
                cx.spawn_in(window, async move |workbench, cx| {
                    let outcome = work.await;
                    let _ = workbench.update_in(cx, |w, window, cx| {
                        view.update(cx, |v, cx| {
                            v.model.is_saving = false;
                            cx.notify();
                        });
                        match outcome {
                            SaveOutcome::Saved {
                                digest,
                                text,
                                local,
                            } => {
                                if let Some(digest) = digest {
                                    let mut store = tasks::project_trust();
                                    store.trust(&trust_key(&root), &digest);
                                    tasks::save_project_trust(&store);
                                    toast(
                                        window,
                                        cx,
                                        ToastKind::Success,
                                        format!("Saved {RELATIVE_PATH}"),
                                        None,
                                        None,
                                    );
                                } else {
                                    toast(
                                        window,
                                        cx,
                                        ToastKind::Success,
                                        format!("Saved {RELATIVE_PATH}"),
                                        Some(
                                            "It also holds settings this screen didn't write, so \
                                             it asks before running its commands."
                                                .into(),
                                        ),
                                        None,
                                    );
                                }
                                view.update(cx, |v, cx| {
                                    v.model.mark_saved(config, &text, local);
                                    cx.notify();
                                });
                                finish(w, then, true, window, cx);
                            }
                            SaveOutcome::Changed => {
                                w.ask_about_changed_settings(view, then, window, cx)
                            }
                            SaveOutcome::Failed(error) => {
                                toast(
                                    window,
                                    cx,
                                    ToastKind::Warning,
                                    format!("Couldn't save the project settings: {error}"),
                                    None,
                                    None,
                                );
                                finish(w, then, false, window, cx);
                            }
                        }
                    });
                })
                .detach();
            }
        };
        match dropped.filter(|_| !overwriting) {
            Some(dropped) => {
                let workbench = cx.entity().downgrade();
                tasks::confirm(
                    window,
                    cx,
                    format!("Rewrite {RELATIVE_PATH}'s settings?"),
                    format!(
                        "What this screen doesn't show in the sections it manages is dropped: \
                         {dropped}. The rest of the file is kept."
                    ),
                    "Save",
                    "Cancel",
                    false,
                    move |yes, window, cx| {
                        let _ = workbench.update(cx, |w, cx| {
                            if yes {
                                write(w, then, window, cx)
                            } else {
                                finish(w, then, false, window, cx)
                            }
                        });
                    },
                );
            }
            None => write(self, then, window, cx),
        }
    }

    /// `.impulse/project.toml` changed since the screen read it (edited in
    /// the editor, brought in by a pull): show it as it is now, losing the
    /// screen's changes, or write over it.
    fn ask_about_changed_settings(
        &mut self,
        view: Entity<ProjectSetupView>,
        then: Option<Then<bool>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[derive(Clone, Copy)]
        enum Answer {
            Reload,
            SaveAnyway,
            Cancel,
        }
        type Pending = Rc<RefCell<Option<(Entity<ProjectSetupView>, Option<Then<bool>>)>>>;
        let pending: Pending = Rc::new(RefCell::new(Some((view, then))));
        let workbench = cx.entity().downgrade();
        let answer = Rc::new(move |answer: Answer, window: &mut Window, cx: &mut App| {
            window.close_dialog(cx);
            let Some((view, then)) = pending.borrow_mut().take() else {
                return;
            };
            let _ = workbench.update(cx, |w, cx| match answer {
                Answer::Reload => {
                    let root = view.read(cx).model.root.clone();
                    w.load_project_setup(view, root, None, window, cx);
                    if let Some(then) = then {
                        then(w, false, window, cx)
                    }
                }
                Answer::SaveAnyway => w.save_project_setup(view, true, then, window, cx),
                Answer::Cancel => {
                    if let Some(then) = then {
                        then(w, false, window, cx)
                    }
                }
            });
        });
        let (reload, save, cancel) = (answer.clone(), answer.clone(), answer);
        crate::alert::Alert::new(format!(
            "{RELATIVE_PATH} changed since Project Setup read it"
        ))
        .message(
            "Reload shows the file as it is now, discarding your changes here. Save Anyway \
                 replaces the sections this screen manages with what it shows, discarding the \
                 changes made to them in the file.",
        )
        .button("Reload", move |window, cx| {
            reload(Answer::Reload, window, cx)
        })
        .button("Save Anyway", move |window, cx| {
            save(Answer::SaveAnyway, window, cx)
        })
        .button("Cancel", move |window, cx| {
            cancel(Answer::Cancel, window, cx)
        })
        .show(window, cx);
    }

    /// Save, then New Task from the repository, to see the settings work.
    fn try_project_setup(
        &mut self,
        view: Entity<ProjectSetupView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = view.read(cx).model.root.clone();
        self.save_project_setup(
            view,
            false,
            Some(Box::new(move |w, saved, window, cx| {
                if saved {
                    let index = w.workspace_at(&root);
                    w.present_new_task(index, false, window, cx);
                }
            })),
            window,
            cx,
        );
    }

    /// Save, then write the current ports and values into the env file of
    /// every task Impulse made, once the settings are trusted.
    fn reapply_project_setup(
        &mut self,
        view: Entity<ProjectSetupView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = view.read(cx).model.root.clone();
        self.save_project_setup(
            view,
            false,
            Some(Box::new(move |_, saved, window, cx| {
                if !saved {
                    return;
                }
                let work = cx.background_executor().spawn(async move {
                    let tasks = tasks::registry(&root).map(|r| r.tasks).unwrap_or_default();
                    tasks
                        .iter()
                        .filter(|task| {
                            let (Some(slot), Some(config)) =
                                (task.slot, tasks::already_trusted_project_config(&task.path))
                            else {
                                return false;
                            };
                            tasks::write_task_values(&config, &task.path, slot)
                        })
                        .count()
                });
                cx.spawn_in(window, async move |_, cx| {
                    let count = work.await;
                    let _ = cx.update(|window, cx| {
                        toast(
                            window,
                            cx,
                            ToastKind::Success,
                            if count == 0 {
                                "No task needed new values".into()
                            } else {
                                format!(
                                    "Re-applied values to {count} task{}",
                                    if count == 1 { "" } else { "s" }
                                )
                            },
                            None,
                            None,
                        )
                    });
                })
                .detach();
            })),
            window,
            cx,
        );
    }

    /// Open `.impulse/project.toml`, creating it if needed.
    pub(crate) fn open_project_settings_file(
        &mut self,
        root: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = Path::new(root).join(RELATIVE_PATH);
        if !path.exists() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&path, "");
        }
        self.open_file(&path, window, cx);
    }
}

/// Write `data` to `path` through a file beside it, so a reader never sees
/// half of it.
fn write_atomically(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = path.with_file_name(format!(".{name}.impulse-{}", std::process::id()));
    std::fs::write(&temporary, data)?;
    std::fs::rename(&temporary, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}
