//! When a workspace's HEAD moves and dependency files come with it, offer
//! the steps that bring the checkout up to date (the Swift
//! `DependencyPrompts`): the project's `[on_change]` commands for the files
//! that changed (or its setup script when a known dependency file changed
//! and no rule names it), and after a merge or pull, its check script,
//! since a clean merge can still break code. `RepositoryState` says when
//! HEAD moved (`RepositoryEvent::HeadMoved`).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{Context, Window};

use crate::toasts::Toast;
use impulse_kit::project::config::ProjectConfig;
use impulse_kit::project::dependency_changes::{self, Move};

use crate::repository_state::{RepositoryState, RepositoryStore};
use crate::settings_store::SettingsStore;
use crate::tasks::{self, toast, ToastKind};
use crate::workbench::Workbench;
use crate::workspace::Kind;

/// How long the toast stays up, as the Swift app's does.
const TOAST_SECONDS: u64 = 20;

/// What a move brought in that needs a step.
struct Steps {
    kind: Move,
    changed: Vec<String>,
    known: Vec<String>,
    /// Read here, not yet trusted: they only decide what the toast says and
    /// offers. What runs is worked out again from the trusted settings.
    settings: Option<ProjectConfig>,
}

/// What a toast's button runs, picked from the trusted settings.
type Pick = Box<dyn FnOnce(&ProjectConfig) -> Option<String>>;

/// What brings the checkout up to date after `changed`: the `[on_change]`
/// commands for the files that changed, else the setup script when a known
/// dependency file changed.
pub(crate) fn update_command(
    settings: Option<&ProjectConfig>,
    changed: &[String],
    known: &[String],
) -> Option<String> {
    let rules = settings
        .map(|s| dependency_changes::matches(&s.on_change, changed))
        .unwrap_or_default();
    if !rules.is_empty() {
        return Some(
            rules
                .into_iter()
                .map(|(_, command)| command)
                .collect::<Vec<_>>()
                .join(" && "),
        );
    }
    if known.is_empty() {
        None
    } else {
        settings.and_then(|s| s.setup_script.clone())
    }
}

/// The files the toast names: "composer.lock and package-lock.json", or
/// "a, b and 3 more".
pub(crate) fn listed_files(files: &[String]) -> String {
    let unique: BTreeSet<&String> = files.iter().collect();
    let names: Vec<String> = unique
        .into_iter()
        .map(|f| {
            Path::new(f)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| f.clone())
        })
        .collect();
    if names.len() <= 2 {
        names.join(" and ")
    } else {
        format!("{}, {} and {} more", names[0], names[1], names.len() - 2)
    }
}

/// How HEAD moved and what changed, off the main thread; `None` when
/// there's nothing to offer.
fn look_at_move(root: &str, from: &str, to: &str, trusted: bool) -> Option<Steps> {
    let subject =
        RepositoryState::read_git(root, trusted, &["reflog", "-1", "--format=%gs", "HEAD"])?;
    let subject = subject.trim();
    if subject.is_empty() {
        return None;
    }
    let kind = dependency_changes::move_of(subject);
    if kind == Move::OwnCommit {
        return None;
    }
    let changed: Vec<String> = RepositoryState::read_git(
        root,
        trusted,
        &["diff", "--name-only", "-z", "--no-renames", from, to, "--"],
    )
    .unwrap_or_default()
    .split('\0')
    .filter(|p| !p.is_empty())
    .map(String::from)
    .collect();
    let settings = tasks::load_project_config(root).and_then(|l| l.config.ok());
    let rules = settings
        .as_ref()
        .map(|s| dependency_changes::matches(&s.on_change, &changed))
        .unwrap_or_default();
    let known = dependency_changes::known_changed(&changed);
    if rules.is_empty() && known.is_empty() {
        return None;
    }
    Some(Steps {
        kind,
        changed,
        known,
        settings,
    })
}

impl Workbench {
    /// The repository at `root` moved HEAD from `from` to `to`: offer what
    /// brings its workspace in this window up to date.
    pub(crate) fn offer_dependency_steps(
        &mut self,
        root: String,
        from: String,
        to: String,
        cx: &mut Context<Self>,
    ) {
        let folders: Vec<(u64, PathBuf)> = self
            .workspaces
            .iter()
            .filter(|w| w.kind == Kind::Folder)
            .map(|w| (w.id, w.root.clone()))
            .collect();
        let Some(workspace) = folders
            .into_iter()
            .find(|(_, dir)| RepositoryStore::root_for(dir, cx).as_deref() == Some(root.as_str()))
            .map(|(id, _)| id)
        else {
            return;
        };
        let trusted = !SettingsStore::get(cx).ask_to_trust_folders || self.trust.is_trusted(&root);
        let work = {
            let root = root.clone();
            cx.background_executor()
                .spawn(async move { look_at_move(&root, &from, &to, trusted) })
        };
        cx.spawn(async move |workbench, cx| {
            let Some(steps) = work.await else {
                return;
            };
            let id = workbench.entity_id();
            let handle = cx.update(|cx| {
                crate::session::workbenches(cx)
                    .into_iter()
                    .find(|(w, _)| w.entity_id() == id)
                    .map(|(_, handle)| handle)
            });
            let Some(handle) = handle else {
                return;
            };
            let _ = handle.update(cx, |_, window, cx| {
                let _ = workbench.update(cx, |w, cx| {
                    w.show_dependency_toast(workspace, root, steps, window, cx)
                });
            });
        })
        .detach();
    }

    fn show_dependency_toast(
        &mut self,
        workspace: u64,
        root: String,
        steps: Steps,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = steps.settings.as_ref();
        let rules = settings
            .map(|s| dependency_changes::matches(&s.on_change, &steps.changed))
            .unwrap_or_default();
        let mut files: Vec<String> = rules.iter().map(|(file, _)| file.clone()).collect();
        files.extend(steps.known.iter().cloned());
        let command = update_command(settings, &steps.changed, &steps.known);
        let check = (steps.kind == Move::Merge)
            .then(|| settings.and_then(|s| s.check_script.clone()))
            .flatten();
        let detail = match &command {
            Some(command) => format!("Update with: {command}"),
            None => "Reinstall dependencies before running tests.".into(),
        };
        type Run = Rc<dyn Fn(&mut Workbench, &mut Window, &mut Context<Workbench>)>;
        let mut buttons: Vec<(&'static str, Run)> = Vec::new();
        if command.is_some() {
            let (root, changed, known) = (root.clone(), steps.changed.clone(), steps.known.clone());
            buttons.push((
                if rules.is_empty() {
                    "Run Setup"
                } else {
                    "Update Dependencies"
                },
                Rc::new(move |w, window, cx| {
                    let (changed, known) = (changed.clone(), known.clone());
                    w.run_project_command(
                        root.clone(),
                        workspace,
                        Box::new(move |config| update_command(Some(config), &changed, &known)),
                        window,
                        cx,
                    )
                }),
            ));
        }
        if check.is_some() {
            let root = root.clone();
            buttons.push((
                "Run Check",
                Rc::new(move |w, window, cx| {
                    w.run_project_command(
                        root.clone(),
                        workspace,
                        Box::new(|config| config.check_script.clone()),
                        window,
                        cx,
                    )
                }),
            ));
        }
        let workbench = cx.entity().downgrade();
        let mut note = Toast::info(detail)
            .title(format!("{} changed", listed_files(&files)))
            .lifetime(Some(Duration::from_secs(TOAST_SECONDS)));
        for (label, run) in buttons {
            let workbench = workbench.clone();
            note = note.action(
                label,
                Rc::new(move |window, cx| {
                    let run = run.clone();
                    let _ = workbench.update(cx, |w, cx| run(w, window, cx));
                }),
            );
        }
        crate::toasts::show(window, cx, note);
    }

    /// Run a command from the project settings in a new terminal tab of
    /// `workspace`, once they're trusted. `pick` takes it from the trusted
    /// settings, so what runs is what was trusted.
    fn run_project_command(
        &mut self,
        root: String,
        workspace: u64,
        pick: Pick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let trusted_root = root.clone();
        self.trust_project_config(
            &trusted_root,
            window,
            cx,
            Box::new(move |w, config, window, cx| {
                let Some(config) = config else {
                    return;
                };
                let Some(command) = pick(&config) else {
                    return toast(
                        window,
                        cx,
                        ToastKind::Info,
                        "The project settings have nothing to run for this anymore.".into(),
                        None,
                        None,
                    );
                };
                let index = w
                    .workspaces
                    .iter()
                    .position(|ws| ws.id == workspace)
                    .unwrap_or(w.active);
                w.activate_workspace(index, window, cx);
                let terminal = w.new_terminal(PathBuf::from(&root), vec![command], window, cx);
                w.add_tab(index, terminal, window, cx);
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn rules_win_over_the_setup_script() {
        let settings = ProjectConfig {
            setup_script: Some("make setup".into()),
            on_change: BTreeMap::from([
                ("composer.lock".to_string(), "composer install".to_string()),
                ("package-lock.json".to_string(), "npm ci".to_string()),
            ]),
            ..Default::default()
        };
        let changed = strings(&["web/package-lock.json", "composer.lock", "src/a.php"]);
        let known = dependency_changes::known_changed(&changed);
        assert_eq!(
            update_command(Some(&settings), &changed, &known).as_deref(),
            Some("composer install && npm ci")
        );
        let changed = strings(&["yarn.lock"]);
        let known = dependency_changes::known_changed(&changed);
        assert_eq!(
            update_command(Some(&settings), &changed, &known).as_deref(),
            Some("make setup"),
            "a known file no rule names"
        );
        assert_eq!(update_command(None, &changed, &known), None);
        assert_eq!(
            update_command(Some(&settings), &strings(&["README.md"]), &[]),
            None
        );
    }

    #[test]
    fn the_toast_names_two_files_then_counts() {
        assert_eq!(
            listed_files(&strings(&["a/composer.lock"])),
            "composer.lock"
        );
        assert_eq!(
            listed_files(&strings(&["yarn.lock", "composer.lock", "yarn.lock"])),
            "composer.lock and yarn.lock"
        );
        assert_eq!(
            listed_files(&strings(&["a", "b", "c", "d"])),
            "a, b and 2 more"
        );
    }
}
