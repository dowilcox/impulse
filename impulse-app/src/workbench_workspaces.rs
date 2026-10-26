//! Workspace commands beyond opening and closing: recent folders (noted
//! when a folder opens, listed by the palette's workspace switcher, cleared
//! with Undo), Rename Workspace, and the one-field prompt it uses.

use std::path::PathBuf;

use crate::toasts::Toast;
use gpui_kit::component::input::InputState;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{prelude::*, Context, Window};
use impulse_kit::recent_workspaces;

use crate::workbench::Workbench;
use crate::workspace::Kind;

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
            self.toast(Toast::info("No recent workspaces to clear"), window, cx);
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
        let workbench = cx.entity().downgrade();
        // Return reaches the alert as its default button (a one-line field
        // passes Enter on), so the field itself doesn't submit.
        let done = {
            let input = input.clone();
            move |window: &mut Window, cx: &mut gpui_kit::App| {
                let text = input.read(cx).value().to_string();
                window.close_dialog(cx);
                let _ = workbench.update(cx, |this, cx| submit(this, text, window, cx));
            }
        };
        let first = std::cell::Cell::new(true);
        let mut alert = crate::alert::Alert::new(title.to_string());
        if !message.is_empty() {
            alert = alert.message(message.to_string());
        }
        alert
            .accessory(Some(260.), move |window, cx| {
                if first.replace(false) {
                    input.update(cx, |field, cx| {
                        field.focus(window, cx);
                        field.select_all(window, cx);
                    });
                }
                crate::alert::field(&input, 260., 24.).into_any_element()
            })
            .button(confirm, done)
            .button("Cancel", |window, cx| window.close_dialog(cx))
            .show(window, cx);
    }
}
