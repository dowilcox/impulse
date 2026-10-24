//! Workspace commands beyond opening and closing: recent folders (noted
//! when a folder opens, listed by the palette's workspace switcher, cleared
//! with Undo), Rename Workspace, and the one-field prompt it uses.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{div, prelude::*, px, Context, Window};
use impulse_kit::recent_workspaces;

use crate::workbench::Workbench;
use crate::workspace::Kind;

type Submit = Rc<dyn Fn(&mut Workbench, String, &mut Window, &mut Context<Workbench>)>;

/// Where the recent folders are kept.
pub(crate) fn recents_file() -> PathBuf {
    impulse_kit::paths::data_dir(cfg!(debug_assertions)).join("recent-workspaces.json")
}

impl Workbench {
    /// Recent folders that aren't open here and still exist, newest first.
    pub(crate) fn recent_folders(&self) -> Vec<String> {
        recent_workspaces::load(&recents_file())
            .into_iter()
            .filter(|folder| {
                !self
                    .workspaces
                    .iter()
                    .any(|w| w.kind == Kind::Folder && w.root.to_string_lossy() == folder.as_str())
                    && std::path::Path::new(folder).is_dir()
            })
            .collect()
    }

    pub(crate) fn clear_recent_workspaces(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let file = recents_file();
        let cleared = recent_workspaces::clear(&file);
        if cleared.is_empty() {
            self.toast(
                Notification::info("No recent workspaces to clear"),
                window,
                cx,
            );
            return;
        }
        let count = cleared.len();
        self.toast_with_action(
            format!(
                "Cleared {count} recent workspace{}",
                if count == 1 { "" } else { "s" }
            ),
            None,
            "Undo",
            window,
            cx,
            move |_, _, _| recent_workspaces::restore(&file, &cleared),
        );
    }

    /// Rename Workspace: an empty name goes back to the folder's.
    pub(crate) fn rename_workspace(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspaces.get(index) else {
            return;
        };
        let id = workspace.id;
        let current = workspace.name();
        self.text_prompt(
            "Rename Workspace",
            "Leave empty to use the folder name.",
            &current,
            "Rename",
            window,
            cx,
            move |this, name, _, cx| {
                if let Some(workspace) = this.workspaces.iter_mut().find(|w| w.id == id) {
                    let name = name.trim().to_string();
                    workspace.custom_name = (!name.is_empty()).then_some(name);
                    cx.notify();
                }
            },
        );
    }

    /// A dialog asking for one line of text, starting as `initial`
    /// (selected); `submit` gets it when confirmed with the button or
    /// Return.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn text_prompt(
        &mut self,
        title: &str,
        message: &str,
        initial: &str,
        confirm: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
        submit: impl Fn(&mut Workbench, String, &mut Window, &mut Context<Workbench>) + 'static,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(initial.to_string()));
        let submit: Submit = Rc::new(submit);
        let workbench = cx.entity().downgrade();
        let done: Rc<dyn Fn(&mut Window, &mut gpui_kit::App)> = {
            let (input, submit) = (input.clone(), submit.clone());
            Rc::new(move |window, cx| {
                let text = input.read(cx).value().to_string();
                window.close_dialog(cx);
                let submit = submit.clone();
                let _ = workbench.update(cx, |this, cx| submit(this, text, window, cx));
            })
        };
        // Return arrives while the workbench is already being updated.
        cx.subscribe_in(
            &input,
            window,
            move |this, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let text = input.read(cx).value().to_string();
                    window.close_dialog(cx);
                    submit(this, text, window, cx);
                }
            },
        )
        .detach();
        let (title, message) = (title.to_string(), message.to_string());
        let first = std::cell::Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if first.replace(false) {
                input.update(cx, |field, cx| {
                    field.focus(window, cx);
                    field.select_all(window, cx);
                });
            }
            let done = done.clone();
            dialog
                .title(title.clone())
                .w(px(420.))
                .close_button(false)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .when(!message.is_empty(), |d| d.child(message.clone()))
                        .child(Input::new(&input)),
                )
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            Button::new("prompt-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("prompt-ok")
                                .primary()
                                .label(confirm)
                                .on_click(move |_, window, cx| done(window, cx)),
                        ),
                )
        });
    }
}
