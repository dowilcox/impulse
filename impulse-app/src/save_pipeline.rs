//! Saving an editor (the Swift `saveEditorTab`): a file changed on disk
//! asks first; then the write, the file type's formatter and the commands
//! on save, both only in trusted folders, run off the main thread with the
//! login shell's PATH. What a formatter (or a command with `reload_file`)
//! leaves in the file is shown unless it failed or typing moved on.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{div, prelude::*, px, App, Context, Entity, Window};
use impulse_kit::child_process::ChildProcess;
use impulse_kit::save_command;

use crate::editor_tab::EditorTab;
use crate::settings_store::SettingsStore;
use crate::tasks::{toast, ToastKind};
use crate::workbench::Workbench;

/// A formatter (or a command on save that rewrites the file) that runs
/// longer is stopped.
const FORMATTER_TIMEOUT: Duration = Duration::from_secs(60);

impl Workbench {
    /// Save the editor: Save As for an untitled one, a question when the
    /// file changed on disk since it was loaded.
    pub(crate) fn save_editor(
        &mut self,
        editor: Entity<EditorTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if editor.read(cx).untitled.is_some() {
            return self.save_untitled(editor, window, cx);
        }
        if editor.read(cx).read_only {
            return;
        }
        if !editor.read(cx).changed_on_disk() {
            return self.write_editor(editor, window, cx);
        }
        let name = editor.read(cx).title.clone();
        let workbench = cx.entity().downgrade();
        changed_on_disk_dialog(&name, window, cx, move |answer, window, cx| {
            let _ = workbench.update(cx, |w, cx| match answer {
                Answer::Save => w.write_editor(editor.clone(), window, cx),
                Answer::Reload => editor.update(cx, |e, cx| e.take_disk(window, cx)),
                Answer::Cancel => {}
            });
        });
    }

    /// Write the file, then its formatter and the commands on save.
    fn write_editor(
        &mut self,
        editor: Entity<EditorTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(saved) = editor.update(cx, |e, cx| e.write(cx)) else {
            return;
        };
        let path = editor.read(cx).path.clone();
        let file = path.to_string_lossy().into_owned();
        // Formatters load the project's own config (prettier runs
        // .prettierrc.js): trusted folders only.
        let formatter = self
            .trust
            .is_trusted(&file)
            .then(|| SettingsStore::get(cx).format_on_save(&file).cloned())
            .flatten()
            .filter(|f| !f.command.is_empty());
        let Some(formatter) = formatter else {
            return self.after_save(&editor, &path, window, cx);
        };
        editor.update(cx, |e, _| e.begin_rewrite());
        let failure = format!(
            "Formatter “{}” failed on {}.",
            formatter.command,
            file_name(&path)
        );
        let args = save_command::expand_arguments(&formatter.args, &file);
        let task = cx.background_spawn({
            let (command, cwd) = (formatter.command.clone(), parent(&path));
            async move { run(&command, &args, &cwd, Some(FORMATTER_TIMEOUT)) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |w, window, cx| {
                if let Err(problem) = &result {
                    toast(
                        window,
                        cx,
                        ToastKind::Warning,
                        failure,
                        Some(problem.clone()),
                        None,
                    );
                }
                editor.update(cx, |e, cx| {
                    e.adopt_rewrite(&saved, result.is_ok(), window, cx)
                });
                w.after_save(&editor, &path, window, cx);
            });
        })
        .detach();
    }

    /// After the write (and formatter): the commands on save, in trusted
    /// folders, and the file tree's git marks.
    fn after_save(
        &mut self,
        editor: &Entity<EditorTab>,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reload_tree(cx);
        let file = path.to_string_lossy().into_owned();
        if !self.trust.is_trusted(&file) {
            return;
        }
        let commands: Vec<_> = SettingsStore::get(cx)
            .commands_on_save_for(&file)
            .filter(|c| !c.command.is_empty())
            .cloned()
            .collect();
        for command in commands {
            let label = if command.name.is_empty() {
                command.command.clone()
            } else {
                command.name.clone()
            };
            let failure = format!("Command on save “{label}” failed on {}.", file_name(path));
            let args = save_command::expand_arguments(&command.args, &file);
            let saved = command.reload_file.then(|| editor.read(cx).disk_text(cx));
            if saved.is_some() {
                editor.update(cx, |e, _| e.begin_rewrite());
            }
            // It rewrites the file like a formatter, so it gets the same
            // limit.
            let timeout = command.reload_file.then_some(FORMATTER_TIMEOUT);
            let task = cx.background_spawn({
                let (program, cwd) = (command.command.clone(), parent(path));
                async move { run(&program, &args, &cwd, timeout) }
            });
            let editor = editor.clone();
            cx.spawn_in(window, async move |_, cx| {
                let result = task.await;
                let _ = cx.update(|window, cx| {
                    if let Err(problem) = &result {
                        toast(
                            window,
                            cx,
                            ToastKind::Warning,
                            failure,
                            Some(problem.clone()),
                            None,
                        );
                    }
                    if let Some(saved) = saved {
                        editor.update(cx, |e, cx| {
                            e.adopt_rewrite(&saved, result.is_ok(), window, cx)
                        });
                    }
                });
            })
            .detach();
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn parent(path: &Path) -> PathBuf {
    path.parent().map(Path::to_path_buf).unwrap_or_default()
}

/// Run a formatter or command on save (blocking): `Err` with a one-line
/// reason when it can't run, fails or takes too long. A bare name is looked
/// up on the login shell's PATH through `env`.
fn run(
    command: &str,
    args: &[String],
    cwd: &Path,
    timeout: Option<Duration>,
) -> Result<(), String> {
    if !save_command::is_safe_command(command) {
        return Err(format!(
            "“{command}” must be a program name or an absolute path."
        ));
    }
    if args.iter().any(|a| a.contains('\0')) {
        return Err("Its arguments contain a null byte.".into());
    }
    let process = if command.starts_with('/') {
        ChildProcess::new(command).args(args)
    } else {
        ChildProcess::new("/usr/bin/env").arg(command).args(args)
    };
    let output = process
        .dir(cwd)
        .env("PATH", impulse_kit::shell::login::login_path())
        .timeout_opt(timeout)
        .run()
        .map_err(|e| e.to_string())?;
    if output.status != 0 || output.timed_out {
        return Err(save_command::failure_summary(
            output.status,
            &output.stdout,
            &output.stderr,
            output.timed_out,
        ));
    }
    Ok(())
}

const DISK_CHANGED: &str = "It was changed outside this editor (by an agent, git or another app) \
                            since you opened it. Saving replaces those changes with your version.";

#[derive(Clone, Copy)]
enum Answer {
    Save,
    Reload,
    Cancel,
}

type Answered = Rc<RefCell<Option<Box<dyn FnOnce(Answer, &mut Window, &mut App)>>>>;

/// “X changed on disk”: save over it, take the disk's version instead, or
/// don't save.
fn changed_on_disk_dialog(
    name: &str,
    window: &mut Window,
    cx: &mut App,
    answer: impl FnOnce(Answer, &mut Window, &mut App) + 'static,
) {
    let answered: Answered = Rc::new(RefCell::new(Some(Box::new(answer))));
    let title = format!("“{name}” changed on disk");
    #[cfg(debug_assertions)]
    {
        let press = |answer: Answer| -> crate::debug_snapshot::Press {
            let answered = answered.clone();
            Rc::new(move |window, cx| {
                window.close_dialog(cx);
                if let Some(answered) = answered.borrow_mut().take() {
                    answered(answer, window, cx)
                }
            })
        };
        crate::debug_snapshot::note_sheet(
            vec![title.clone(), DISK_CHANGED.into()],
            vec![
                ("Cancel", press(Answer::Cancel)),
                ("Reload from Disk", press(Answer::Reload)),
                ("Save Anyway", press(Answer::Save)),
            ],
        );
    }
    window.open_dialog(cx, move |dialog, _, _| {
        let reply = |answer: Answer| {
            let answered = answered.clone();
            move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
                window.close_dialog(cx);
                if let Some(answered) = answered.borrow_mut().take() {
                    answered(answer, window, cx)
                }
            }
        };
        let escape = answered.clone();
        dialog
            .title(title.clone())
            .w(px(480.))
            .overlay_closable(false)
            .close_button(false)
            .on_cancel(move |_, window, cx| {
                window.close_dialog(cx);
                if let Some(answered) = escape.borrow_mut().take() {
                    answered(Answer::Cancel, window, cx)
                }
                false
            })
            .child(div().whitespace_normal().child(DISK_CHANGED))
            .footer(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("disk-cancel")
                            .label("Cancel")
                            .on_click(reply(Answer::Cancel)),
                    )
                    .child(
                        Button::new("disk-reload")
                            .label("Reload from Disk")
                            .on_click(reply(Answer::Reload)),
                    )
                    .child(
                        Button::new("disk-save")
                            .danger()
                            .label("Save Anyway")
                            .on_click(reply(Answer::Save)),
                    ),
            )
    });
}
