//! The Agent Integration sheet (the Swift `AgentHooksSheet`): installs (or
//! removes) the hooks that let agents report their state to Impulse, and
//! Impulse's MCP server (`impulse mcp`), showing the exact change to the
//! agent's config first. What's written comes from `impulse_kit::agents`'
//! installers; this is the model around them and its dialog.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{div, prelude::*, px, ClipboardItem, Context, FontWeight, Window};
use impulse_kit::agents::hook_installer::{self as hooks, InstallError, OpencodePlugin, Status};
use impulse_kit::agents::mcp_installer::{self as mcp, ClaudeScope};
use impulse_kit::chrome::{metrics, ChromePalette};
use impulse_kit::shell::quoting::quoted;
use impulse_kit::shell::ShellType;

use crate::workbench::Workbench;
use crate::workbench_chrome::{abbreviate_home, hsla, icon};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
    Opencode,
    /// Agents Impulse doesn't configure: the MCP entry to copy.
    Other,
}

impl Agent {
    pub const ALL: [Agent; 4] = [Agent::Claude, Agent::Codex, Agent::Opencode, Agent::Other];

    pub fn name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
            Agent::Opencode => "opencode",
            Agent::Other => "Other",
        }
    }
}

/// What's being set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Hooks,
    Mcp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    User,
    Project,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Written,
    /// The file changed since the preview: it's been redone, for another
    /// look.
    Changed,
    Failed(String),
}

/// The sheet's state: the chosen part, agent and scope, and the previewed
/// change to that agent's config.
pub struct Model {
    pub part: Part,
    pub agent: Agent,
    pub scope: Scope,
    /// The bundled CLI's path, for the MCP entry's fallback outside Impulse.
    pub fallback_cli: String,
    /// The project for project-local Claude hooks, if any.
    pub project_root: Option<String>,
    /// The repository's main checkout, when `project_root` is a task
    /// worktree (whose project hooks go away when it's archived).
    pub main_checkout: Option<String>,
    home: PathBuf,
    environment: HashMap<String, String>,
    /// For Claude Code's MCP server: the `claude` commands that make the
    /// change, in order (Claude Code rewrites its own config while it runs,
    /// so Impulse doesn't edit it); `None` when `claude` isn't on PATH.
    pub claude_commands: Option<Vec<Vec<String>>>,
    claude_executable: Option<String>,
    pub path: String,
    pub before: String,
    /// The file's bytes when the preview was made (`None`: no file).
    previewed: Option<Vec<u8>>,
    pub after: Option<String>,
    pub installed: bool,
    /// Some of Impulse's hooks, from an older Impulse: Install adds the rest.
    pub outdated: bool,
    pub error: Option<String>,
    /// The file changed after the preview was made.
    pub refreshed: bool,
    /// Something to know about this change, shown above the preview.
    pub notice: Option<String>,
    opencode_plugin: OpencodePlugin,
    /// The plugin's files to write (`None` when removing).
    opencode_files: Option<BTreeMap<String, String>>,
}

fn read(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

fn text(data: &Option<Vec<u8>>) -> String {
    data.as_deref()
        .map(|d| String::from_utf8_lossy(d).into_owned())
        .unwrap_or_default()
}

impl Model {
    pub fn new(
        part: Part,
        project_root: Option<String>,
        main_checkout: Option<String>,
        fallback_cli: String,
        home: PathBuf,
        environment: HashMap<String, String>,
    ) -> Self {
        let mut model = Self {
            part,
            agent: Agent::Claude,
            scope: Scope::User,
            fallback_cli,
            project_root,
            main_checkout,
            home,
            environment,
            claude_commands: None,
            claude_executable: None,
            path: String::new(),
            before: String::new(),
            previewed: None,
            after: None,
            installed: false,
            outdated: false,
            error: None,
            refreshed: false,
            notice: None,
            opencode_plugin: OpencodePlugin::default(),
            opencode_files: None,
        };
        model.reload();
        model
    }

    pub fn set_part(&mut self, part: Part) {
        self.part = part;
        if part == Part::Hooks && self.agent == Agent::Other {
            self.agent = Agent::Claude;
        }
        self.reload();
    }

    pub fn set_agent(&mut self, agent: Agent) {
        self.agent = agent;
        self.reload();
    }

    pub fn set_scope(&mut self, scope: Scope) {
        self.scope = scope;
        self.reload();
    }

    /// The agents the current part offers.
    pub fn agents(&self) -> Vec<Agent> {
        Agent::ALL
            .into_iter()
            .filter(|a| self.part == Part::Mcp || *a != Agent::Other)
            .collect()
    }

    /// What the sheet installs, in its messages.
    pub fn what(&self) -> &'static str {
        match self.part {
            Part::Hooks => "hooks",
            Part::Mcp => "MCP server",
        }
    }

    /// The preview: the change as diff lines, `claude` commands, or the
    /// entry to copy.
    pub fn diff(&self) -> Vec<String> {
        if self.part == Part::Mcp && self.agent == Agent::Other {
            return mcp::generic_snippet(&self.fallback_cli)
                .split('\n')
                .map(String::from)
                .collect();
        }
        if self.part == Part::Mcp && self.agent == Agent::Claude {
            return self
                .claude_commands
                .iter()
                .flatten()
                .map(|args| {
                    let words: Vec<String> = std::iter::once("claude".to_string())
                        .chain(args.iter().cloned())
                        .map(|w| quoted(&w, ShellType::Zsh))
                        .collect();
                    format!("$ {}", words.join(" "))
                })
                .collect();
        }
        match &self.after {
            Some(after) => hooks::diff_lines(&self.before, after),
            None => Vec::new(),
        }
    }

    fn home_str(&self) -> String {
        self.home.to_string_lossy().into_owned()
    }

    fn opencode_directory(&self) -> String {
        mcp::opencode_config_directory(&self.environment, &self.home_str())
    }

    fn opencode_plugins(&self) -> PathBuf {
        Path::new(&self.opencode_directory()).join("plugins")
    }

    /// opencode's hooks are a plugin folder of Impulse's own, written and
    /// removed file by file.
    fn writes_opencode_plugin(&self) -> bool {
        self.part == Part::Hooks && self.agent == Agent::Opencode
    }

    /// What's in opencode's plugins folder now: Impulse's folder and the
    /// older single-file plugin.
    fn read_opencode_plugin(&self) -> OpencodePlugin {
        let folder = self.opencode_plugins().join(hooks::OPENCODE_PLUGIN_FOLDER);
        let mut files = BTreeMap::new();
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(data) = read(&path) {
                    files.insert(
                        entry.file_name().to_string_lossy().into_owned(),
                        String::from_utf8_lossy(&data).into_owned(),
                    );
                }
            }
        }
        let legacy = read(
            &self
                .opencode_plugins()
                .join(hooks::OPENCODE_LEGACY_PLUGIN_NAME),
        )
        .map(|d| String::from_utf8_lossy(&d).into_owned());
        OpencodePlugin { files, legacy }
    }

    /// The bytes the preview was made from, to tell whether they changed.
    fn current(&self) -> Option<Vec<u8>> {
        if self.writes_opencode_plugin() {
            return Some(hooks::opencode_plugin_text(&self.read_opencode_plugin()).into_bytes());
        }
        read(Path::new(&self.path))
    }

    fn fail(&mut self, failure: InstallError) {
        self.after = None;
        self.error = Some(failure.to_string());
    }

    pub fn reload(&mut self) {
        self.notice = None;
        self.error = None;
        self.refreshed = false;
        // Other agents only have the MCP entry to copy.
        if self.part == Part::Mcp || self.agent == Agent::Other {
            return self.reload_mcp();
        }
        if self.agent == Agent::Opencode {
            return self.reload_opencode_plugin();
        }
        let path = match (self.agent, self.scope, &self.project_root) {
            (Agent::Claude, Scope::Project, Some(root)) => {
                Path::new(root).join(".claude/settings.local.json")
            }
            (Agent::Claude, _, _) => self.home.join(".claude/settings.json"),
            _ => self.home.join(".codex/config.toml"),
        };
        self.path = path.to_string_lossy().into_owned();
        let data = read(&path);
        self.before = text(&data);
        self.previewed = data.clone();
        match self.agent {
            Agent::Claude => {
                let status = hooks::claude_hook_status(data.as_deref());
                self.installed = status == Status::Installed;
                self.outdated = status == Status::Outdated;
                let result = if self.installed {
                    hooks::removing_claude_hooks(data.as_deref().unwrap_or_default())
                } else {
                    hooks::installing_claude_hooks(data.as_deref())
                };
                match result {
                    Ok(bytes) => self.after = Some(String::from_utf8_lossy(&bytes).into_owned()),
                    Err(failure) => self.fail(failure),
                }
            }
            _ => {
                self.installed = hooks::codex_notify_installed(Some(&self.before));
                self.outdated = false;
                if self.installed {
                    let removed = hooks::removing_codex_notify(&self.before);
                    self.after = (removed != self.before).then_some(removed);
                    if self.after.is_none() {
                        self.error = Some(
                            "Impulse's notify isn't on a line of its own; remove it from config.toml by hand."
                                .into(),
                        );
                    }
                } else {
                    let existing = data.is_some().then_some(self.before.as_str());
                    match hooks::installing_codex_notify(existing) {
                        Ok(text) => self.after = Some(text),
                        Err(failure) => self.fail(failure),
                    }
                }
            }
        }
    }

    /// opencode's hooks: Impulse's plugin folder (replacing the single file
    /// older Impulses installed for opencode 1).
    fn reload_opencode_plugin(&mut self) {
        self.path = self
            .opencode_plugins()
            .join(hooks::OPENCODE_PLUGIN_FOLDER)
            .to_string_lossy()
            .into_owned();
        let found = self.read_opencode_plugin();
        self.before = hooks::opencode_plugin_text(&found);
        self.previewed = Some(self.before.clone().into_bytes());
        let status = hooks::opencode_plugin_status(&found);
        self.installed = status == Status::Installed;
        self.outdated = status == Status::Outdated;
        if self.installed {
            self.opencode_files = None;
            self.after = Some(String::new());
        } else {
            match hooks::installing_opencode_plugin(&found) {
                Ok(files) => {
                    self.after = Some(hooks::opencode_plugin_text(&OpencodePlugin {
                        files: files.clone(),
                        legacy: None,
                    }));
                    self.opencode_files = Some(files);
                }
                Err(failure) => {
                    self.opencode_files = None;
                    self.fail(failure);
                }
            }
        }
        self.opencode_plugin = found;
        let config = mcp::opencode_config_file_on_disk(&self.opencode_directory());
        let config = read(Path::new(&config)).map(|d| String::from_utf8_lossy(&d).into_owned());
        if mcp::opencode_has_old_entry(config.as_deref()) {
            self.notice = Some(
                "opencode's config still has the MCP server an older Impulse added. Remove it under MCP Server: the plugin gives opencode Impulse's tools now."
                    .into(),
            );
        }
    }

    fn claude_scope(&self) -> ClaudeScope {
        if self.scope == Scope::Project && self.project_root.is_some() {
            ClaudeScope::Local
        } else {
            ClaudeScope::User
        }
    }

    fn reload_mcp(&mut self) {
        self.outdated = false;
        self.claude_commands = None;
        match self.agent {
            Agent::Claude => {
                let path = self.home.join(".claude.json");
                self.path = path.to_string_lossy().into_owned();
                let data = read(&path);
                self.previewed = data.clone();
                self.before.clear();
                let scope = self.claude_scope();
                let status = mcp::claude_status(
                    data.as_deref(),
                    scope,
                    self.project_root.as_deref(),
                    &self.fallback_cli,
                );
                self.installed = status == Status::Installed;
                self.outdated = status == Status::Outdated;
                self.claude_executable = impulse_kit::shell::login::which("claude");
                let add = mcp::claude_add_arguments(scope, &self.fallback_cli);
                if self.claude_executable.is_none() {
                    self.after = None;
                    let words: Vec<String> =
                        add.iter().map(|w| quoted(w, ShellType::Zsh)).collect();
                    self.error = Some(format!(
                        "The claude command isn't on PATH. Install Claude Code, or run: claude {}",
                        words.join(" ")
                    ));
                } else {
                    let remove = mcp::claude_remove_arguments(scope);
                    // `add-json` refuses a name that's taken: an older entry
                    // goes first.
                    self.claude_commands = Some(if self.installed {
                        vec![remove]
                    } else if self.outdated {
                        vec![remove, add]
                    } else {
                        vec![add]
                    });
                    self.after = Some(String::new());
                }
            }
            Agent::Codex => {
                let path = self.home.join(".codex/config.toml");
                self.path = path.to_string_lossy().into_owned();
                let data = read(&path);
                self.before = text(&data);
                self.previewed = data.clone();
                let existing = data.is_some().then_some(self.before.as_str());
                let status = mcp::codex_status(existing, &self.fallback_cli);
                self.installed = status == Status::Installed;
                self.outdated = status == Status::Outdated;
                if self.installed {
                    self.after = mcp::removing_codex_mcp(&self.before);
                    if self.after.is_none() {
                        self.error = Some(
                            "The [mcp_servers.impulse] table isn't the one Impulse wrote; remove it from config.toml by hand."
                                .into(),
                        );
                    }
                } else {
                    match mcp::installing_codex_mcp(existing, &self.fallback_cli) {
                        Ok(text) => self.after = Some(text),
                        Err(failure) => self.fail(failure),
                    }
                }
            }
            Agent::Opencode => {
                // opencode gets Impulse's tools from the plugin: all there
                // is to do here is take out the MCP entry older Impulses
                // added, which opencode's shared service would run for
                // whichever terminal started it.
                self.path = mcp::opencode_config_file_on_disk(&self.opencode_directory());
                let data = read(Path::new(&self.path));
                self.before = text(&data);
                self.previewed = data.clone();
                let existing = data.is_some().then_some(self.before.as_str());
                self.installed = mcp::opencode_has_old_entry(existing);
                if self.installed {
                    self.after = mcp::removing_opencode_mcp(&self.before);
                    self.notice = Some(
                        "opencode runs MCP servers in its background service, which every terminal shares, so this entry can't tell which terminal is asking. Impulse's plugin (under Hooks) gives opencode the same tools."
                            .into(),
                    );
                } else {
                    self.after = None;
                    self.notice = Some(
                        "opencode gets Impulse's tools from Impulse's plugin: install it under Hooks."
                            .into(),
                    );
                }
            }
            Agent::Other => {
                self.path.clear();
                self.previewed = None;
                self.before.clear();
                self.installed = false;
                self.after = None;
            }
        }
    }

    /// The `claude` commands for Claude Code's MCP server (run off the
    /// main thread), or `None` when this change is a file to write.
    pub fn claude_run(&self) -> Option<Result<ClaudeRun, String>> {
        if self.part != Part::Mcp || self.agent != Agent::Claude {
            return None;
        }
        Some(match (&self.claude_executable, &self.claude_commands) {
            (Some(executable), Some(commands)) if !commands.is_empty() => Ok((
                executable.clone(),
                commands.clone(),
                self.project_root.clone().unwrap_or_else(|| self.home_str()),
            )),
            _ => Err(self
                .error
                .clone()
                .unwrap_or_else(|| "Nothing to change.".into())),
        })
    }

    /// Write the previewed change, unless the file changed since the
    /// preview was made (the agent rewrote its settings, say): then the
    /// preview is made again from what's there now, and nothing is written.
    pub fn apply(&mut self) -> Outcome {
        let Some(after) = self.after.clone() else {
            return Outcome::Failed(
                self.error
                    .clone()
                    .unwrap_or_else(|| "Nothing to change.".into()),
            );
        };
        if self.current() != self.previewed {
            self.reload();
            self.refreshed = true;
            return Outcome::Changed;
        }
        let result = if self.writes_opencode_plugin() {
            self.write_opencode_plugin()
        } else {
            self.write_config(&after)
        };
        match result {
            Ok(()) => Outcome::Written,
            Err(error) => Outcome::Failed(error.to_string()),
        }
    }

    /// The agent's own config: written through a symlink (dotfile managers
    /// link these) rather than replacing it, keeping the previous version.
    fn write_config(&self, after: &str) -> std::io::Result<()> {
        let path = Path::new(&self.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let target = impulse_kit::text_file::target(path);
        if target.exists() {
            let backup = PathBuf::from(format!("{}.impulse-backup", target.display()));
            let _ = std::fs::remove_file(&backup);
            std::fs::copy(&target, &backup)?;
        }
        let temporary = PathBuf::from(format!("{}.impulse-new", target.display()));
        std::fs::write(&temporary, after)?;
        std::fs::rename(&temporary, &target)
    }

    /// Install: Impulse's files written into its folder (older ones it no
    /// longer has removed), and the single-file plugin removed. Remove: its
    /// files, and the folder once empty. Only Impulse's files are touched.
    fn write_opencode_plugin(&self) -> std::io::Result<()> {
        let folder = PathBuf::from(&self.path);
        let ours: Vec<&String> = self
            .opencode_plugin
            .files
            .iter()
            .filter(|(_, text)| hooks::is_opencode_plugin_file(text))
            .map(|(name, _)| name)
            .collect();
        if let Some(files) = &self.opencode_files {
            std::fs::create_dir_all(&folder)?;
            for (name, text) in files {
                std::fs::write(folder.join(name), text)?;
            }
            for name in ours.iter().filter(|n| !files.contains_key(n.as_str())) {
                std::fs::remove_file(folder.join(name))?;
            }
        } else {
            for name in &ours {
                std::fs::remove_file(folder.join(name))?;
            }
            if std::fs::read_dir(&folder).is_ok_and(|mut d| d.next().is_none()) {
                std::fs::remove_dir(&folder)?;
            }
        }
        if self.opencode_plugin.legacy_is_impulses() {
            std::fs::remove_file(
                self.opencode_plugins()
                    .join(hooks::OPENCODE_LEGACY_PLUGIN_NAME),
            )?;
        }
        Ok(())
    }

    /// The status line above the preview.
    pub fn status_text(&self) -> &'static str {
        let old_entry = self.part == Part::Mcp && self.agent == Agent::Opencode;
        if self.agent == Agent::Other {
            "Add this to the agent's own MCP settings (each agent keeps them in its own file)"
        } else if old_entry {
            if self.installed {
                "An older Impulse's MCP server entry"
            } else {
                "No MCP server entry"
            }
        } else if self.installed {
            "Installed"
        } else if self.outdated {
            match self.part {
                Part::Hooks => "Out of date: install again to add the newer hooks",
                Part::Mcp => "Out of date: install again to update it",
            }
        } else {
            "Not installed"
        }
    }
}

/// The `claude` executable, its argument lists and the folder to run in.
pub type ClaudeRun = (String, Vec<Vec<String>>, String);

/// Run `claude` with each argument list in turn, stopping at a failure.
fn run_claude(executable: &str, commands: &[Vec<String>], directory: &str) -> Outcome {
    let path = impulse_kit::shell::login::login_path();
    for arguments in commands {
        let output = impulse_kit::child_process::ChildProcess::new(executable)
            .args(arguments)
            .dir(directory)
            .env("PATH", &path)
            .timeout(std::time::Duration::from_secs(30))
            .run();
        match output {
            Ok(output) if output.status == 0 => continue,
            Ok(output) => {
                let bytes = if output.stderr.is_empty() {
                    &output.stdout
                } else {
                    &output.stderr
                };
                let message = String::from_utf8_lossy(bytes).trim().to_string();
                return Outcome::Failed(if message.is_empty() {
                    format!("claude exited with status {}", output.status)
                } else {
                    message
                });
            }
            Err(error) => return Outcome::Failed(error.to_string()),
        }
    }
    Outcome::Written
}

// MARK: The sheet

pub struct AgentIntegration {
    model: Model,
    palette: ChromePalette,
    workbench: gpui_kit::WeakEntity<Workbench>,
    busy: bool,
    /// Claude Code's scope, a pop-up as the Swift sheet's.
    scope: crate::settings_view::ChoiceSelect,
    _scope_changed: gpui_kit::Subscription,
}

impl AgentIntegration {
    fn close(&mut self, message: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
        if let Some(message) = message {
            window.push_notification(Notification::success(message), cx);
        }
        let _ = self.workbench.update(cx, |_, cx| cx.notify());
    }

    fn primary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.agent == Agent::Other {
            cx.write_to_clipboard(ClipboardItem::new_string(self.model.diff().join("\n")));
            return self.close(
                Some("Copied Impulse's MCP server entry.".into()),
                window,
                cx,
            );
        }
        let was_installed = self.model.installed;
        let (agent, what) = (self.model.agent.name(), self.model.what());
        let path = abbreviate_home(&self.model.path);
        let done = move |outcome: Outcome| {
            #[cfg(debug_assertions)]
            crate::debug_snapshot::note_result(
                "agent-sheet",
                match &outcome {
                    Outcome::Written => "written".into(),
                    Outcome::Changed => "changed".into(),
                    Outcome::Failed(failure) => format!("failed({failure})"),
                },
            );
            match outcome {
                Outcome::Failed(failure) => {
                    Some(Some(format!("Couldn't update {path}: {failure}")))
                }
                // The sheet stays open on the new preview.
                Outcome::Changed => None,
                Outcome::Written if was_installed => {
                    Some(Some(format!("Removed Impulse's {agent} {what}.")))
                }
                Outcome::Written => Some(Some(format!(
                    "Installed the {agent} {what}. Restart running agents to pick it up."
                ))),
            }
        };
        match self.model.claude_run() {
            None => {
                let outcome = self.model.apply();
                match done(outcome) {
                    Some(message) => self.close(message, window, cx),
                    None => cx.notify(),
                }
            }
            Some(Err(failure)) => self.close(done(Outcome::Failed(failure)).flatten(), window, cx),
            Some(Ok((executable, commands, directory))) => {
                self.busy = true;
                cx.notify();
                let task =
                    cx.background_spawn(
                        async move { run_claude(&executable, &commands, &directory) },
                    );
                cx.spawn_in(window, async move |this, cx| {
                    let outcome = task.await;
                    let _ = this.update_in(cx, |sheet, window, cx| {
                        sheet.busy = false;
                        match done(outcome) {
                            Some(message) => sheet.close(message, window, cx),
                            None => cx.notify(),
                        }
                    });
                })
                .detach();
            }
        }
    }
}

impl Render for AgentIntegration {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The pop-up follows the scope when it's set another way (the
        // headless snapshots).
        let scope = if self.model.scope == Scope::Project {
            "project"
        } else {
            "user"
        };
        if self.scope.read(cx).selected_value().map(String::as_str) != Some(scope) {
            self.scope.update(cx, |select, cx| {
                select.set_selected_value(&scope.to_string(), window, cx)
            });
        }
        let p = self.palette.clone();
        let m = &self.model;
        let info = |text: String, warning: bool| {
            div()
                .flex()
                .items_start()
                .gap(px(6.))
                .child(div().pt(px(2.)).child(icon(
                    "info",
                    11.,
                    if warning { p.warning } else { p.text_secondary },
                )))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11.5))
                        .text_color(hsla(p.text_secondary))
                        .child(text),
                )
        };
        let light = p.is_light;
        let parts = [Part::Hooks, Part::Mcp];
        let sheet = cx.entity().downgrade();
        let part_picker = crate::workbench_chrome::native_segmented(
            "agent-part",
            vec!["Hooks".into(), "MCP Server".into()],
            parts.iter().position(|part| *part == m.part).unwrap_or(0),
            94.,
            light,
            false,
            move |index, _, cx| {
                let _ = sheet.update(cx, |sheet, cx| {
                    sheet.model.set_part(parts[index]);
                    cx.notify();
                });
            },
        );
        let agents = m.agents();
        let sheet = cx.entity().downgrade();
        let agent_picker = crate::workbench_chrome::native_segmented(
            if m.part == Part::Hooks {
                "agent-hooks-agents"
            } else {
                "agent-mcp-agents"
            },
            agents.iter().map(|agent| agent.name().into()).collect(),
            agents
                .iter()
                .position(|agent| *agent == m.agent)
                .unwrap_or(0),
            100.,
            light,
            false,
            {
                let agents = agents.clone();
                move |index, _, cx| {
                    let _ = sheet.update(cx, |sheet, cx| {
                        sheet.model.set_agent(agents[index]);
                        cx.notify();
                    });
                }
            },
        );
        let scope_picker = (m.agent == Agent::Claude).then(|| {
            div()
                .w(px(170.))
                .h(px(24.))
                .flex()
                .items_center()
                .when(m.project_root.is_none(), |d| d.opacity(0.5))
                .child(
                    crate::settings_view::popup_frame(&p).w_full().child(
                        crate::settings_view::popup(&self.scope)
                            .disabled(m.project_root.is_none())
                            .menu_width(px(200.)),
                    ),
                )
        });
        let old_entry = m.part == Part::Mcp && m.agent == Agent::Opencode;
        let dot = if old_entry {
            if m.installed {
                p.warning
            } else {
                p.text_tertiary
            }
        } else if m.installed {
            p.success
        } else if m.outdated {
            p.warning
        } else {
            p.text_tertiary
        };
        let diff = m.diff();
        let mut preview = div()
            .id("agent-preview")
            .flex()
            .flex_col()
            .h(px(220.))
            .overflow_y_scroll()
            .p(px(8.))
            .rounded(px(metrics::RADIUS))
            .bg(hsla(p.content))
            .font_family("JetBrains Mono")
            .text_size(px(11.));
        if diff.is_empty() {
            preview = preview.child(
                div()
                    .font_family(crate::workbench::UI_FONT)
                    .text_size(px(11.5))
                    .text_color(hsla(if m.error.is_some() {
                        p.danger
                    } else {
                        p.text_tertiary
                    }))
                    .child(
                        m.error
                            .clone()
                            .unwrap_or_else(|| "No change needed.".into()),
                    ),
            );
        }
        for line in diff {
            let color = if line.starts_with("+ ") {
                p.git_added
            } else if line.starts_with("- ") {
                p.git_deleted
            } else {
                p.text_tertiary
            };
            preview = preview.child(div().flex_none().text_color(hsla(color)).child(
                if line.is_empty() {
                    " ".to_string()
                } else {
                    line
                },
            ));
        }
        let primary: Option<Button> = if m.agent == Agent::Other {
            Some(
                Button::new("agent-copy")
                    .primary()
                    .icon(gpui_kit::component::Icon::empty().path("icons/copy.svg"))
                    .label("Copy"),
            )
        } else if m.after.is_some() {
            let label = format!(
                "{} {}",
                if m.installed { "Remove" } else { "Install" },
                if m.part == Part::Hooks {
                    "Hooks"
                } else {
                    "MCP Server"
                }
            );
            let button = Button::new("agent-apply")
                .icon(gpui_kit::component::Icon::empty().path(if m.installed {
                    "icons/trash-2.svg"
                } else {
                    "icons/check.svg"
                }))
                .label(label)
                .loading(self.busy);
            Some(if m.installed {
                button
            } else {
                button.primary()
            })
        } else {
            None
        };
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .text_color(hsla(p.text))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Agent Integration"),
                    )
                    .child(part_picker),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(hsla(p.text_secondary))
                    .child(match m.part {
                        Part::Hooks => "Hooks make the agent tell Impulse exactly when it starts working, needs you, and finishes, with no guessing from output. They only do anything inside Impulse terminals.",
                        Part::Mcp => "The MCP server gives the agent Impulse's tools: the repository's tasks (and starting one, which asks you first), your review comments, the language servers' problems, other terminals' output, unsaved files and the project's scripts. It only has tools inside Impulse terminals.",
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(agent_picker)
                    .children(scope_picker),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().size(px(7.)).rounded_full().bg(hsla(dot)))
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(hsla(p.text_secondary))
                            .child(m.status_text()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .font_family("JetBrains Mono")
                            .text_size(px(11.))
                            .text_color(hsla(p.text_tertiary))
                            .child(abbreviate_home(&m.path)),
                    ),
            )
            .children(m.notice.clone().map(|n| info(n, false)))
            .children(m.refreshed.then(|| {
                info(
                    "The file changed since this preview, so nothing was written. Check the change again.".into(),
                    true,
                )
            }))
            .children(
                (m.agent == Agent::Claude && m.scope == Scope::Project)
                    .then_some(m.main_checkout.as_ref())
                    .flatten()
                    .map(|main| {
                        info(
                            format!(
                                "This is a task folder: project hooks installed here are deleted when the task is archived. To keep them, install them in the main checkout ({}); new tasks copy them from there unless a .worktreeinclude leaves them out.",
                                abbreviate_home(main)
                            ),
                            true,
                        )
                    }),
            )
            .child(preview)
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("agent-close")
                            .label("Close")
                            .on_click(cx.listener(|sheet, _, window, cx| sheet.close(None, window, cx))),
                    )
                    .children(primary.map(|button| {
                        button.on_click(cx.listener(|sheet, _, window, cx| {
                            if !sheet.busy {
                                sheet.primary(window, cx)
                            }
                        }))
                    })),
            )
    }
}

impl Workbench {
    /// Agent Integration, on the hooks or the MCP server.
    pub(crate) fn present_agent_integration(
        &mut self,
        part: Part,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dir = self
            .focused_surface()
            .and_then(|s| s.directory(cx))
            .unwrap_or_else(|| self.workspace().default_directory());
        let project_root = impulse_git::branches::repo_root(&dir.to_string_lossy())
            .map(|r| r.trim_end_matches('/').to_string());
        let main_checkout = project_root
            .as_deref()
            .map(crate::tasks::main_checkout_root)
            .filter(|main| Some(main) != project_root.as_ref());
        let fallback_cli = crate::control_server::cli_path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| mcp::DEFAULT_CLI.to_string());
        let model = Model::new(
            part,
            project_root,
            main_checkout,
            fallback_cli,
            crate::workspace::home(),
            std::env::vars().collect(),
        );
        let palette = self.palette.clone();
        let workbench = cx.entity().downgrade();
        let scope = crate::settings_view::choice_select(
            vec![
                crate::settings_view::Choice::new("user", "All projects"),
                crate::settings_view::Choice::new("project", "This project only"),
            ],
            if model.scope == Scope::Project {
                "project"
            } else {
                "user"
            },
            false,
            window,
            cx,
        );
        let sheet = cx.new(|cx| {
            let changed = cx.subscribe(
                &scope,
                |sheet: &mut AgentIntegration,
                 _,
                 event: &gpui_kit::component::select::SelectEvent<
                    gpui_kit::component::select::SearchableVec<crate::settings_view::Choice>,
                >,
                 cx| {
                    if let gpui_kit::component::select::SelectEvent::Confirm(Some(value)) = event {
                        sheet.model.set_scope(if value == "project" {
                            Scope::Project
                        } else {
                            Scope::User
                        });
                        cx.notify();
                    }
                },
            );
            AgentIntegration {
                model,
                palette,
                workbench,
                busy: false,
                scope: scope.clone(),
                _scope_changed: changed,
            }
        });
        #[cfg(debug_assertions)]
        crate::debug_snapshot::note_agent_sheet(&sheet);
        // The sheet's own color, as the Swift sheet's `overlay` background.
        let background = hsla(self.palette.overlay);
        crate::overlays::open_dialog(window, cx, move |dialog, _, _| {
            dialog
                .w(px(660.))
                .bg(background)
                .close_button(false)
                .child(sheet.clone())
        });
    }
}

/// For the headless snapshots (`debug_snapshot`): choose, press Install or
/// Remove, and say what the sheet shows.
#[cfg(debug_assertions)]
impl AgentIntegration {
    /// Agents by the start of their name, lowercased (`mcp`, `claude`).
    pub(crate) fn debug_choose(
        &mut self,
        part: Option<&str>,
        agent: Option<&str>,
        project: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(part) = part {
            self.model.set_part(if part == "mcp" {
                Part::Mcp
            } else {
                Part::Hooks
            });
        }
        if let Some(agent) = agent {
            let found = Agent::ALL
                .into_iter()
                .find(|a| a.name().to_lowercase().starts_with(agent));
            self.model.set_agent(found.unwrap_or(Agent::Claude));
        }
        if project {
            self.model.set_scope(Scope::Project);
        }
        cx.notify();
    }

    pub(crate) fn debug_apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.primary(window, cx);
    }

    pub(crate) fn debug_state(&self) -> serde_json::Value {
        let m = &self.model;
        serde_json::json!({
            "part": match m.part { Part::Hooks => "Hooks", Part::Mcp => "MCP Server" },
            "agent": m.agent.name(), "installed": m.installed, "outdated": m.outdated,
            "path": m.path, "diff": m.diff(), "error": m.error, "notice": m.notice,
        })
    }
}

/// For `Workbench::run_command`: the sheet's commands.
pub(crate) fn part_for(command: &str) -> Option<Part> {
    match command {
        "agent_hooks" => Some(Part::Hooks),
        "agent_mcp_server" => Some(Part::Mcp),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "impulse-agent-sheet-{}-{}",
            std::process::id(),
            crate::control_server::new_token()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn model(part: Part, home: &Path) -> Model {
        Model::new(
            part,
            None,
            None,
            "/opt/impulse".into(),
            home.to_path_buf(),
            HashMap::new(),
        )
    }

    #[test]
    fn claude_hooks_install_and_remove() {
        let home = home();
        let mut sheet = model(Part::Hooks, &home);
        assert_eq!(sheet.agent, Agent::Claude);
        assert!(!sheet.installed);
        assert!(sheet.path.ends_with(".claude/settings.json"));
        assert!(sheet.diff().iter().any(|l| l.starts_with("+ ")));
        assert_eq!(sheet.apply(), Outcome::Written);
        let written = std::fs::read(home.join(".claude/settings.json")).unwrap();
        assert!(hooks::claude_hooks_installed(Some(&written)));

        sheet.reload();
        assert!(sheet.installed);
        assert_eq!(sheet.status_text(), "Installed");
        assert_eq!(sheet.apply(), Outcome::Written);
        let removed = std::fs::read(home.join(".claude/settings.json")).unwrap();
        assert!(!hooks::claude_hooks_installed(Some(&removed)));
        assert!(home.join(".claude/settings.json.impulse-backup").exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_file_changed_since_the_preview_is_not_written() {
        let home = home();
        let mut sheet = model(Part::Hooks, &home);
        sheet.set_agent(Agent::Codex);
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::write(home.join(".codex/config.toml"), "model = \"o3\"\n").unwrap();
        assert_eq!(sheet.apply(), Outcome::Changed);
        assert!(sheet.refreshed);
        assert_eq!(sheet.apply(), Outcome::Written);
        let text = std::fs::read_to_string(home.join(".codex/config.toml")).unwrap();
        assert!(text.contains("model = \"o3\""));
        assert!(hooks::codex_notify_installed(Some(&text)));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn opencode_plugin_is_a_folder_of_impulses_own() {
        let home = home();
        let mut sheet = model(Part::Hooks, &home);
        sheet.set_agent(Agent::Opencode);
        assert!(sheet.path.ends_with(".config/opencode/plugins/impulse"));
        assert_eq!(sheet.apply(), Outcome::Written);
        sheet.reload();
        assert!(sheet.installed);
        assert_eq!(sheet.apply(), Outcome::Written);
        assert!(!home.join(".config/opencode/plugins/impulse").exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn other_agents_only_get_the_entry_to_copy() {
        let home = home();
        let mut sheet = model(Part::Mcp, &home);
        sheet.set_agent(Agent::Other);
        assert!(sheet.diff().join("\n").contains("mcpServers"));
        assert!(sheet.after.is_none());
        // Back to hooks, which Other doesn't have.
        sheet.set_part(Part::Hooks);
        assert_eq!(sheet.agent, Agent::Claude);
        assert!(!sheet.agents().contains(&Agent::Other));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn codex_mcp_server_is_a_table_in_its_config() {
        let home = home();
        let mut sheet = model(Part::Mcp, &home);
        sheet.set_agent(Agent::Codex);
        assert_eq!(sheet.apply(), Outcome::Written);
        let text = std::fs::read_to_string(home.join(".codex/config.toml")).unwrap();
        assert_eq!(
            mcp::codex_status(Some(&text), "/opt/impulse"),
            Status::Installed
        );
        let _ = std::fs::remove_dir_all(&home);
    }
}
