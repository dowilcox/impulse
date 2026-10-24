//! Project actions (the Swift `ProjectActions`): the commands a repository's
//! `.impulse/project.toml` lists under `[[actions]]`, offered in the
//! palette's `a:` mode and run in a new terminal tab, or a split when the
//! action says `open = "right"` or `"down"`, from the repository root or
//! its `cwd`. They run only once the project settings are trusted, and
//! what runs is the trusted settings' action of that name, not the one the
//! palette listed: the file can change in between. Edit Project Actions
//! opens Project Setup at its Actions.

use std::path::Path;

use gpui_kit::{App, Context, Window};
use impulse_kit::layout_tree::SplitAxis;
use impulse_kit::project::config::Action;

use crate::tasks::{self, toast, ToastKind};
use crate::workbench::Workbench;

impl Workbench {
    /// The repository root for project actions: the active workspace's
    /// repository (the task's own checkout in a task).
    pub(crate) fn project_root(&self, cx: &App) -> Option<String> {
        self.git
            .active
            .as_ref()
            .map(|s| s.read(cx).root.clone())
            .or_else(|| impulse_git::repo_root(&self.tree_root(cx).to_string_lossy()))
            .map(|r| r.trim_end_matches('/').to_string())
    }

    /// The actions the palette lists (no trust needed to list them), or why
    /// the settings can't be read.
    pub(crate) fn palette_project_actions(&self, cx: &App) -> Result<Vec<Action>, String> {
        let Some(root) = self.project_root(cx) else {
            return Ok(Vec::new());
        };
        match tasks::load_project_config(&root).map(|l| l.config) {
            Some(Ok(config)) => Ok(config.actions),
            Some(Err(error)) => Err(error.to_string()),
            None => Ok(Vec::new()),
        }
    }

    /// Run the action the palette listed, in the active workspace's
    /// repository.
    pub(crate) fn palette_run_project_action(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(root) = self.project_root(cx) {
            self.run_project_action(action, root, window, cx);
        }
    }

    /// Run a project action in a new terminal (a tab, or a split right or
    /// down) once the settings are trusted.
    pub(crate) fn run_project_action(
        &mut self,
        listed: Action,
        root: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let trusted_root = root.clone();
        self.trust_project_config(
            &trusted_root,
            window,
            cx,
            Box::new(move |workbench, config, window, cx| {
                let Some(config) = config else {
                    return;
                };
                let Some(action) = config.actions.into_iter().find(|a| a.name == listed.name)
                else {
                    return toast(
                        window,
                        cx,
                        ToastKind::Info,
                        format!(
                            "The project settings have no action named {} anymore.",
                            listed.name
                        ),
                        None,
                        None,
                    );
                };
                let directory = match &action.cwd {
                    Some(cwd) => Path::new(&root).join(cwd),
                    None => Path::new(&root).to_path_buf(),
                };
                let terminal = workbench.new_terminal(directory, vec![action.command], window, cx);
                match action.open.as_deref() {
                    Some("right") => {
                        workbench.split_focused(terminal, SplitAxis::Horizontal, window, cx)
                    }
                    Some("down") => {
                        workbench.split_focused(terminal, SplitAxis::Vertical, window, cx)
                    }
                    _ => {
                        let active = workbench.active;
                        workbench.add_tab(active, terminal, window, cx);
                    }
                }
            }),
        );
    }

    /// "Edit Project Actions": Project Setup at its Actions.
    pub(crate) fn edit_project_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_project_setup(None, Some("actions"), window, cx);
    }
}
