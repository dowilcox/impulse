//! What the Project Setup tab holds (the Swift `ProjectSetupModel` and the
//! static parts of `MainWindowController+ProjectSetup`): the settings as the
//! screen shows them, filled from what's saved and what the repository
//! suggests, the entries that can't be saved as shown, and the settings
//! they make. No UI here; `project_setup` draws it and saves it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use impulse_kit::project::config::{Action, Landing, LoadError, ProjectConfig, PORT_OFFSETS};
use impulse_kit::project::detector::{self, ContainerName, FixedPort, ProjectSuggestions};
use impulse_kit::project::{settings_file, task_environment};

/// A file or folder to copy or clone into tasks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub path: String,
    pub is_on: bool,
    pub note: Option<String>,
    pub size: Option<String>,
    /// Added by the user, or saved but not found: its path is edited, and
    /// it can be removed.
    pub is_custom: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pair {
    pub id: u64,
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionRow {
    pub id: u64,
    pub is_on: bool,
    pub action: Action,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Database {
    None,
    Clone,
    Dump,
    Empty,
}

/// The local `.git/impulse/project.toml`, when there is one. Impulse's own
/// (an earlier save, Finish Task's answer) is shown folded in and removed
/// on save; `dropped` names its settings the project's replace. One edited
/// by hand is left alone, and wins in `overrides`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalFile {
    Impulses {
        text: String,
        dropped: Vec<String>,
    },
    HandEdited {
        overrides: Vec<String>,
    },
    /// It doesn't parse: the message says where.
    Unreadable(String),
}

/// A text the screen edits, by the row it belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    CopyPath(u64),
    ClonePath(u64),
    PortName(u64),
    PortValue(u64),
    ValueName(u64),
    ValueValue(u64),
    RuleName(u64),
    RuleCommand(u64),
    ActionName(u64),
    ActionCommand(u64),
    PortOffset,
    Setup,
    Check,
    Archive,
    OverlapIgnore,
    DatabaseFolder,
    DatabaseService,
}

/// The tasks every port must have room for below 65535.
pub const TASKS_WITH_ROOM: i64 = 10;

#[derive(Clone, Debug)]
pub struct SetupModel {
    /// The repository's main checkout.
    pub root: String,
    pub is_loading: bool,
    /// `.impulse/project.toml` as the screen read it (`None`: there was
    /// none), so saving can tell it was changed meanwhile.
    pub loaded_text: Option<String>,
    /// The settings as last read or saved, to tell what's changed since.
    pub saved_config: Option<ProjectConfig>,
    /// Why `.impulse/project.toml` can't be read, when it can't: the screen
    /// says so instead of showing (and saving over) its settings.
    pub load_error: Option<String>,
    pub is_saving: bool,
    /// What the repository suggests. Fields the saved settings leave empty
    /// offer it rather than being filled with it.
    pub found: Option<ProjectSuggestions>,
    pub compose_file_name: Option<String>,
    pub fixed_ports: Vec<FixedPort>,
    pub container_names: Vec<ContainerName>,
    pub compose_override: bool,
    pub copies: Vec<Row>,
    pub clones: Vec<Row>,
    pub ports: Vec<Pair>,
    pub port_offset: String,
    pub values: Vec<Pair>,
    pub env_file: String,
    pub setup: String,
    pub check: String,
    pub archive: String,
    pub actions: Vec<ActionRow>,
    /// `[on_change]`: file to command.
    pub rules: Vec<Pair>,
    /// `overlap_ignore`, comma-separated.
    pub overlap_ignore: String,
    /// How Finish Task lands work (`None`: it asks the first time).
    pub landing: Option<Landing>,
    /// What the Database row offers, and what's chosen.
    pub database_options: Vec<Database>,
    pub database: Database,
    /// The data folder cloned into tasks, and the Compose service stopped
    /// in the main checkout meanwhile, as typed.
    pub database_folder: String,
    pub database_service: String,
    pub dump_command: Option<String>,
    pub empty_command: Option<String>,
    /// Tasks Impulse made that have a slot, for Re-apply.
    pub task_count: usize,
    pub local_file: Option<LocalFile>,
    /// Saving drops comments in the sections of `.impulse/project.toml` the
    /// screen manages, and keys there it doesn't know (`scripts.lint`).
    pub drops_comments: bool,
    pub unknown_keys: Vec<String>,
    next_id: u64,
}

impl Default for SetupModel {
    fn default() -> Self {
        SetupModel {
            root: String::new(),
            is_loading: true,
            loaded_text: None,
            saved_config: None,
            load_error: None,
            is_saving: false,
            found: None,
            compose_file_name: None,
            fixed_ports: Vec::new(),
            container_names: Vec::new(),
            compose_override: false,
            copies: Vec::new(),
            clones: Vec::new(),
            ports: Vec::new(),
            port_offset: "100".into(),
            values: Vec::new(),
            env_file: ".env".into(),
            setup: String::new(),
            check: String::new(),
            archive: String::new(),
            actions: Vec::new(),
            rules: Vec::new(),
            overlap_ignore: String::new(),
            landing: None,
            database_options: Vec::new(),
            database: Database::None,
            database_folder: String::new(),
            database_service: String::new(),
            dump_command: None,
            empty_command: None,
            task_count: 0,
            local_file: None,
            drops_comments: false,
            unknown_keys: Vec::new(),
            next_id: 0,
        }
    }
}

/// Trim spaces and tabs, as Swift's `.whitespaces`.
fn trimmed(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_whitespace() && !is_newline(c))
}

fn is_newline(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// The text of a field, `None` when it's blank.
fn text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// "a, b and c".
pub fn list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

impl SetupModel {
    pub fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub fn name(&self) -> String {
        Path::new(&self.root)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.clone())
    }

    /// Whether the screen holds changes not saved yet.
    pub fn is_modified(&self) -> bool {
        if self.is_loading || self.load_error.is_some() {
            return false;
        }
        self.saved_config
            .as_ref()
            .is_some_and(|saved| self.config() != *saved)
    }

    /// What saving drops from `.impulse/project.toml` because the screen
    /// doesn't show it ("comments and scripts.lint"), if anything.
    pub fn dropped(&self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if self.drops_comments {
            parts.push("comments".into());
        }
        parts.extend(self.unknown_keys.iter().cloned());
        (!parts.is_empty()).then(|| list(&parts))
    }

    /// What task n adds to each port, n times; the default while the field
    /// holds something else.
    pub fn offset(&self) -> i64 {
        trimmed(&self.port_offset)
            .parse::<i64>()
            .ok()
            .filter(|o| PORT_OFFSETS.contains(o))
            .unwrap_or(ProjectConfig::default().port_offset)
    }

    /// Saved: the file holds `config` (what the screen showed) as `text`,
    /// with `local` what's now in the local file. Nothing is read again, so
    /// the screen keeps its place and focus.
    pub fn mark_saved(&mut self, config: ProjectConfig, text: &str, local: Option<LocalFile>) {
        self.saved_config = Some(config);
        self.loaded_text = Some(text.to_string());
        self.local_file = local;
        self.drops_comments = settings_file::managed_sections_have_comments(text);
        self.unknown_keys = settings_file::unknown_managed_keys(text);
    }

    /// Choose what the Database row does. Dump-and-load and an empty
    /// database are commands in the setup script, so they're shown (and can
    /// be edited) there.
    pub fn choose(&mut self, choice: Database) {
        for command in [self.dump_command.clone(), self.empty_command.clone()]
            .into_iter()
            .flatten()
        {
            self.setup = removing(&command, &self.setup);
        }
        self.database = choice;
        let added = match choice {
            Database::Dump => self.dump_command.clone(),
            Database::Empty => self.empty_command.clone(),
            _ => None,
        };
        if let Some(added) = added {
            self.setup = if self.setup.is_empty() {
                added
            } else {
                format!("{} && {added}", self.setup)
            };
        }
    }

    // MARK: Fields

    /// A field's text, `None` once its row is gone.
    pub fn text(&self, field: Field) -> Option<String> {
        let pair = |list: &[Pair], id: u64, name: bool| {
            list.iter().find(|p| p.id == id).map(|p| {
                if name {
                    p.name.clone()
                } else {
                    p.value.clone()
                }
            })
        };
        let row = |list: &[Row], id: u64| list.iter().find(|r| r.id == id).map(|r| r.path.clone());
        let action = |id: u64, name: bool| {
            self.actions.iter().find(|a| a.id == id).map(|a| {
                if name {
                    a.action.name.clone()
                } else {
                    a.action.command.clone()
                }
            })
        };
        match field {
            Field::CopyPath(id) => row(&self.copies, id),
            Field::ClonePath(id) => row(&self.clones, id),
            Field::PortName(id) => pair(&self.ports, id, true),
            Field::PortValue(id) => pair(&self.ports, id, false),
            Field::ValueName(id) => pair(&self.values, id, true),
            Field::ValueValue(id) => pair(&self.values, id, false),
            Field::RuleName(id) => pair(&self.rules, id, true),
            Field::RuleCommand(id) => pair(&self.rules, id, false),
            Field::ActionName(id) => action(id, true),
            Field::ActionCommand(id) => action(id, false),
            Field::PortOffset => Some(self.port_offset.clone()),
            Field::Setup => Some(self.setup.clone()),
            Field::Check => Some(self.check.clone()),
            Field::Archive => Some(self.archive.clone()),
            Field::OverlapIgnore => Some(self.overlap_ignore.clone()),
            Field::DatabaseFolder => Some(self.database_folder.clone()),
            Field::DatabaseService => Some(self.database_service.clone()),
        }
    }

    pub fn set_text(&mut self, field: Field, value: String) {
        fn pair(list: &mut [Pair], id: u64, name: bool, value: String) {
            if let Some(p) = list.iter_mut().find(|p| p.id == id) {
                if name {
                    p.name = value;
                } else {
                    p.value = value;
                }
            }
        }
        fn row(list: &mut [Row], id: u64, value: String) {
            if let Some(r) = list.iter_mut().find(|r| r.id == id) {
                r.path = value;
            }
        }
        match field {
            Field::CopyPath(id) => row(&mut self.copies, id, value),
            Field::ClonePath(id) => row(&mut self.clones, id, value),
            Field::PortName(id) => pair(&mut self.ports, id, true, value),
            Field::PortValue(id) => pair(&mut self.ports, id, false, value),
            Field::ValueName(id) => pair(&mut self.values, id, true, value),
            Field::ValueValue(id) => pair(&mut self.values, id, false, value),
            Field::RuleName(id) => pair(&mut self.rules, id, true, value),
            Field::RuleCommand(id) => pair(&mut self.rules, id, false, value),
            Field::ActionName(id) | Field::ActionCommand(id) => {
                if let Some(a) = self.actions.iter_mut().find(|a| a.id == id) {
                    if matches!(field, Field::ActionName(_)) {
                        a.action.name = value;
                    } else {
                        a.action.command = value;
                    }
                }
            }
            Field::PortOffset => self.port_offset = value,
            Field::Setup => self.setup = value,
            Field::Check => self.check = value,
            Field::Archive => self.archive = value,
            Field::OverlapIgnore => self.overlap_ignore = value,
            Field::DatabaseFolder => self.database_folder = value,
            Field::DatabaseService => self.database_service = value,
        }
    }

    pub fn pair(&mut self, name: &str, value: &str) -> Pair {
        Pair {
            id: self.new_id(),
            name: name.into(),
            value: value.into(),
        }
    }

    // MARK: Problems

    /// Why a port row can't be saved as shown; `None` when it can, or when
    /// it's left empty (skipped).
    pub fn problem_port(&self, pair: &Pair) -> Option<String> {
        if let Some(problem) = problem_name(pair, &self.ports, true, "Needs a name") {
            return Some(problem);
        }
        if is_blank(pair) {
            return None;
        }
        if names(&self.values).contains(trimmed(&pair.name)) {
            return Some("Also a value for each task".into());
        }
        match trimmed(&pair.value).parse::<i64>() {
            Ok(port) if task_environment::PORT_RANGE.contains(&port) => None,
            _ => Some("Not a port number (1–65535)".into()),
        }
    }

    pub fn problem_value(&self, pair: &Pair) -> Option<String> {
        if let Some(problem) = problem_name(pair, &self.values, true, "Needs a name") {
            return Some(problem);
        }
        if is_blank(pair) {
            return None;
        }
        if names(&self.ports).contains(trimmed(&pair.name)) {
            return Some("Also a port".into());
        }
        // The env file holds one value per line, so a line break would be
        // left out.
        if pair.value.chars().any(is_newline) {
            return Some("Can't hold a line break".into());
        }
        let mut known: BTreeSet<String> = ["task", "task_", "slot"].map(String::from).into();
        known.extend(names(&self.ports).into_iter().map(String::from));
        placeholders(&pair.value)
            .into_iter()
            .find(|p| !known.contains(p))
            .map(|unknown| {
                format!("No {{{unknown}}}: use {{task}}, {{task_}}, {{slot}} or a port's name")
            })
    }

    pub fn problem_rule(&self, pair: &Pair) -> Option<String> {
        if let Some(problem) = problem_name(pair, &self.rules, false, "Needs a file name") {
            return Some(problem);
        }
        (!is_blank(pair) && trimmed(&pair.value).is_empty()).then(|| "Needs a command".into())
    }

    pub fn problem_action(&self, row: &ActionRow) -> Option<String> {
        let name = trimmed(&row.action.name);
        let command = trimmed(&row.action.command);
        if !row.is_on || (name.is_empty() && command.is_empty()) {
            return None;
        }
        if name.is_empty() {
            return Some("Needs a name".into());
        }
        if command.is_empty() {
            return Some("Needs a command".into());
        }
        let same = self
            .actions
            .iter()
            .filter(|a| a.is_on && trimmed(&a.action.name) == name)
            .count();
        (same > 1).then(|| format!("{name} is listed twice"))
    }

    /// Empty is the default. Otherwise a whole number that leaves every
    /// port (and with the override on, every fixed Compose port) a port for
    /// `TASKS_WITH_ROOM` tasks.
    pub fn offset_problem(&self) -> Option<String> {
        let value = trimmed(&self.port_offset);
        let offset = if value.is_empty() {
            Some(ProjectConfig::default().port_offset)
        } else {
            value.parse::<i64>().ok()
        };
        let Some(offset) = offset.filter(|o| PORT_OFFSETS.contains(o)) else {
            return Some(format!("A whole number from 1 to {}", PORT_OFFSETS.end()));
        };
        let mut named: Vec<(String, i64)> = self
            .ports
            .iter()
            .filter_map(|pair| {
                let port = trimmed(&pair.value).parse::<i64>().ok()?;
                task_environment::PORT_RANGE
                    .contains(&port)
                    .then(|| (trimmed(&pair.name).to_string(), port))
            })
            .collect();
        if self.compose_override {
            named.extend(self.fixed_ports.iter().filter_map(|fixed| {
                fixed
                    .port
                    .host
                    .map(|host| (format!("{} {host}", fixed.service), host))
            }));
        }
        let (name, port) = named
            .into_iter()
            .find(|(_, port)| task_environment::port(*port, TASKS_WITH_ROOM, offset).is_none())?;
        Some(format!(
            "Too big: task {TASKS_WITH_ROOM}'s {} would be past 65535",
            if name.is_empty() {
                port.to_string()
            } else {
                name
            }
        ))
    }

    pub fn database_problem(&self) -> Option<String> {
        (self.database == Database::Clone && trimmed(&self.database_folder).is_empty())
            .then(|| "Choose the folder that holds the data".into())
    }

    /// How many entries can't be saved as shown. Save waits for none, so
    /// nothing on the screen is dropped without a word.
    pub fn problem_count(&self) -> usize {
        self.ports
            .iter()
            .filter(|p| self.problem_port(p).is_some())
            .count()
            + self
                .values
                .iter()
                .filter(|p| self.problem_value(p).is_some())
                .count()
            + self
                .rules
                .iter()
                .filter(|p| self.problem_rule(p).is_some())
                .count()
            + self
                .actions
                .iter()
                .filter(|a| self.problem_action(a).is_some())
                .count()
            + usize::from(self.offset_problem().is_some())
            + usize::from(self.database_problem().is_some())
    }

    /// The settings as the screen has them.
    pub fn config(&self) -> ProjectConfig {
        let mut config = ProjectConfig {
            actions: self
                .actions
                .iter()
                .filter(|a| a.is_on)
                .map(|a| Action {
                    name: trimmed(&a.action.name).to_string(),
                    command: trimmed(&a.action.command).to_string(),
                    cwd: a.action.cwd.clone(),
                    open: a.action.open.clone(),
                })
                .filter(|a| !a.name.is_empty() && !a.command.is_empty())
                .collect(),
            setup_script: text(&self.setup),
            archive_script: text(&self.archive),
            check_script: text(&self.check),
            worktree_copy: self
                .copies
                .iter()
                .filter(|r| r.is_on)
                .filter_map(|r| text(&r.path))
                .collect(),
            worktree_clone: self
                .clones
                .iter()
                .filter(|r| r.is_on)
                .filter_map(|r| text(&r.path))
                .collect(),
            env_file: text(&self.env_file).unwrap_or_else(|| ProjectConfig::default().env_file),
            port_offset: self.offset(),
            compose_override: self.compose_override,
            landing: self.landing,
            overlap_ignore: self
                .overlap_ignore
                .split(',')
                .map(trimmed)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect(),
            ..ProjectConfig::default()
        };
        for pair in &self.ports {
            let name = trimmed(&pair.name);
            if let (false, Ok(port)) = (name.is_empty(), trimmed(&pair.value).parse::<i64>()) {
                config.ports.insert(name.to_string(), port);
            }
        }
        for pair in &self.values {
            let name = trimmed(&pair.name);
            if !name.is_empty() {
                config
                    .worktree_env
                    .insert(name.to_string(), pair.value.clone());
            }
        }
        if self.database == Database::Clone {
            if let Some(folder) = text(&self.database_folder) {
                config.database_folder = Some(folder);
                config.database_service = text(&self.database_service);
            }
        }
        for pair in &self.rules {
            let (name, command) = (trimmed(&pair.name), trimmed(&pair.value));
            if !name.is_empty() && !command.is_empty() {
                config
                    .on_change
                    .insert(name.to_string(), command.to_string());
            }
        }
        config
    }

    // MARK: Filling it in

    /// Fill the screen from the saved settings (`existing`) and what the
    /// repository suggests (`found`).
    pub fn fill(
        &mut self,
        existing: Option<&ProjectConfig>,
        found: ProjectSuggestions,
        dump: Option<String>,
        empty: Option<String>,
    ) {
        let settings = existing.cloned().unwrap_or_default();
        // Suggestions fill in a project with no settings yet. Once it has
        // some, the screen shows what's saved, so a field emptied and saved
        // stays empty; the view offers the suggestion beside it instead.
        let has_settings = existing.is_some();
        self.copies = self.rows(&found.copies, &settings.worktree_copy, has_settings);
        self.clones = self.rows(&found.clones, &settings.worktree_clone, has_settings);
        // Ports and an offset the file sets but that can't be used are
        // shown as written, marked, so they're fixed rather than dropped
        // unseen.
        let ports: BTreeMap<String, i64> = if has_settings {
            let mut ports = settings.ports.clone();
            ports.extend(settings.unusable_ports.clone());
            ports
        } else {
            found.ports.clone()
        };
        self.ports = ports
            .iter()
            .map(|(name, port)| self.pair(name, &port.to_string()))
            .collect();
        let values = if has_settings {
            settings.worktree_env.clone()
        } else {
            found.values.clone()
        };
        self.values = values
            .iter()
            .map(|(name, value)| self.pair(name, value))
            .collect();
        self.port_offset = settings
            .unusable_port_offset
            .unwrap_or(settings.port_offset)
            .to_string();
        self.env_file = settings.env_file.clone();
        let script = |saved: &Option<String>, suggested: &Option<String>| {
            if has_settings { saved } else { suggested }
                .clone()
                .unwrap_or_default()
        };
        self.setup = script(&settings.setup_script, &found.setup);
        self.check = script(&settings.check_script, &found.check);
        self.archive = script(&settings.archive_script, &found.archive);
        self.landing = settings.landing;
        let names: BTreeSet<&str> = settings.actions.iter().map(|a| a.name.as_str()).collect();
        let mut actions: Vec<ActionRow> = Vec::new();
        for (is_on, action) in settings.actions.iter().map(|a| (true, a)).chain(
            found
                .actions
                .iter()
                .filter(|a| !names.contains(a.name.as_str()))
                .map(|a| (false, a)),
        ) {
            actions.push(ActionRow {
                id: self.new_id(),
                is_on,
                action: action.clone(),
            });
        }
        self.actions = actions;
        let rules = if has_settings {
            settings.on_change.clone()
        } else {
            found.on_change.clone()
        };
        self.rules = rules
            .iter()
            .map(|(name, command)| self.pair(name, command))
            .collect();
        self.overlap_ignore = settings.overlap_ignore.join(", ");
        self.compose_file_name = found.compose_file_name.clone();
        self.fixed_ports = found.fixed_ports.clone();
        self.container_names = found.container_names.clone();
        self.compose_override = settings.compose_override;
        self.database_folder = settings
            .database_folder
            .clone()
            .or_else(|| found.database_folder.clone())
            .unwrap_or_default();
        self.database_service = if settings.database_folder.is_some() {
            settings.database_service.clone()
        } else {
            found.database_service.clone()
        }
        .unwrap_or_default();
        self.database_options = [Database::None, Database::Clone]
            .into_iter()
            .chain(dump.as_ref().map(|_| Database::Dump))
            .chain(empty.as_ref().map(|_| Database::Empty))
            .collect();
        self.database = if settings.database_folder.is_some() {
            Database::Clone
        } else if dump
            .as_ref()
            .is_some_and(|d| self.setup.contains(d.as_str()))
        {
            Database::Dump
        } else if empty
            .as_ref()
            .is_some_and(|e| self.setup.contains(e.as_str()))
        {
            Database::Empty
        } else {
            Database::None
        };
        self.dump_command = dump;
        self.empty_command = empty;
        self.found = Some(found);
    }

    /// Rows for the detected entries plus any chosen ones not detected;
    /// ticked as chosen when the project has settings, else as suggested.
    fn rows(
        &mut self,
        found: &[detector::Entry],
        chosen: &[String],
        has_settings: bool,
    ) -> Vec<Row> {
        let mut rows: Vec<Row> = Vec::new();
        for entry in found {
            rows.push(Row {
                id: self.new_id(),
                path: entry.path.clone(),
                is_on: if has_settings {
                    chosen.contains(&entry.path)
                } else {
                    entry.suggested
                },
                note: entry.note.clone(),
                size: None,
                is_custom: false,
            });
        }
        for path in chosen {
            if !found.iter().any(|e| &e.path == path) {
                rows.push(self.custom_row(path));
            }
        }
        rows
    }

    pub fn custom_row(&mut self, path: &str) -> Row {
        Row {
            id: self.new_id(),
            path: path.into(),
            is_on: true,
            note: None,
            size: None,
            is_custom: true,
        }
    }
}

/// `script` without `command` and the `&&` that joined it to the rest,
/// wherever it stands.
fn removing(command: &str, script: &str) -> String {
    let joined = [format!(" && {command}"), format!("{command} && ")]
        .into_iter()
        .find(|j| script.contains(j.as_str()))
        .unwrap_or_else(|| command.to_string());
    trimmed(&script.replace(&joined, "")).to_string()
}

/// A row's name: given, a variable name where it goes into an env file,
/// and not given twice.
fn problem_name(pair: &Pair, list: &[Pair], variable: bool, missing: &str) -> Option<String> {
    if is_blank(pair) {
        return None;
    }
    let name = trimmed(&pair.name);
    if name.is_empty() {
        return Some(missing.into());
    }
    if variable && !is_variable_name(name) {
        return Some("Not a variable name (letters, digits and _)".into());
    }
    let same = list.iter().filter(|p| trimmed(&p.name) == name).count();
    (same > 1).then(|| format!("{name} is listed twice"))
}

fn is_blank(pair: &Pair) -> bool {
    trimmed(&pair.name).is_empty() && trimmed(&pair.value).is_empty()
}

fn names(list: &[Pair]) -> BTreeSet<&str> {
    list.iter()
        .map(|p| trimmed(&p.name))
        .filter(|n| !n.is_empty())
        .collect()
}

pub fn is_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// The `{NAME}` placeholders in a value (not other braces, as in JSON).
pub fn placeholders(value: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = value;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('}') else {
            break;
        };
        let name = &rest[..close];
        if is_variable_name(name) {
            names.push(name.to_string());
        }
        rest = &rest[close + 1..];
    }
    names
}

/// The saved settings the screen shows, and what it does about the local
/// file. Impulse's own local file (an earlier save, Finish Task's answer)
/// is folded in under the project's settings, which win, and goes on save;
/// one edited by hand stays, and the screen shows the project's own
/// settings beside a note of what it overrides.
pub fn saved_settings(
    committed: Option<&str>,
    local: Option<&str>,
) -> (Option<ProjectConfig>, Option<LocalFile>) {
    let project = committed.and_then(|c| ProjectConfig::parse(c).ok());
    let Some(local) = local else {
        return (project, None);
    };
    if let Err(LoadError::Invalid(message)) = ProjectConfig::parse(local) {
        return (project, Some(LocalFile::Unreadable(message)));
    }
    let layers = |texts: [Option<&str>; 2]| {
        let texts: Vec<&str> = texts.into_iter().flatten().collect();
        ProjectConfig::parse_layers(&texts).ok()
    };
    let applied = layers([committed, Some(local)]);
    let names = |list: Vec<&'static str>| list.into_iter().map(String::from).collect();
    if settings_file::is_written_by_impulse(local, project.as_ref()) {
        if let Some(folded) = layers([Some(local), committed]) {
            let dropped = applied
                .map(|a| names(a.differences(&folded)))
                .unwrap_or_default();
            return (
                Some(folded),
                Some(LocalFile::Impulses {
                    text: local.to_string(),
                    dropped,
                }),
            );
        }
    }
    let overrides = applied
        .map(|a| names(a.differences(&project.clone().unwrap_or_default())))
        .unwrap_or_default();
    (project, Some(LocalFile::HandEdited { overrides }))
}

/// The bytes a folder takes on disk (its files' allocated blocks), not
/// following symbolic links.
pub fn folder_size(path: &Path) -> u64 {
    let mut total = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(folder) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            total += allocated(&metadata);
            if metadata.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    total
}

#[cfg(unix)]
fn allocated(metadata: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.blocks() * 512
}

#[cfg(not(unix))]
fn allocated(metadata: &std::fs::Metadata) -> u64 {
    metadata.len()
}

/// A size as Foundation's `ByteCountFormatter` writes it for files: powers
/// of 1000, whole kilobytes, one decimal for megabytes, two beyond.
pub fn byte_count(bytes: u64) -> String {
    if bytes == 0 {
        return "Zero KB".into();
    }
    if bytes < 1000 {
        return format!("{bytes} byte{}", if bytes == 1 { "" } else { "s" });
    }
    let units = ["KB", "MB", "GB", "TB", "PB"];
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 999.5 && unit + 1 < units.len() {
        value /= 1000.0;
        unit += 1;
    }
    let decimals = match unit {
        0 => 0,
        1 => 1,
        _ => 2,
    };
    let mut number = format!("{value:.decimals$}");
    if number.contains('.') {
        number = number
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string();
    }
    format!("{number} {}", units[unit])
}

#[cfg(test)]
mod tests {
    use super::*;
    use impulse_kit::project::compose_file;

    fn model() -> SetupModel {
        SetupModel {
            dump_command: Some("DUMP".into()),
            empty_command: Some("MIGRATE".into()),
            ..Default::default()
        }
    }

    fn pairs(model: &mut SetupModel, items: &[(&str, &str)]) -> Vec<Pair> {
        items.iter().map(|(n, v)| model.pair(n, v)).collect()
    }

    fn action(model: &mut SetupModel, is_on: bool, name: &str, command: &str) -> ActionRow {
        ActionRow {
            id: model.new_id(),
            is_on,
            action: Action::new(name, command),
        }
    }

    #[test]
    fn choosing_a_database_leaves_the_rest_of_setup_whole() {
        let mut model = model();
        model.choose(Database::Dump);
        assert_eq!(model.setup, "DUMP");
        model.setup += " && composer install";
        model.choose(Database::None);
        assert_eq!(
            model.setup, "composer install",
            "not \"&& composer install\""
        );

        model.setup = "npm ci && DUMP && composer install".into();
        model.choose(Database::Empty);
        assert_eq!(model.setup, "npm ci && composer install && MIGRATE");
        model.choose(Database::None);
        assert_eq!(model.setup, "npm ci && composer install");
    }

    #[test]
    fn changes_are_told_from_what_was_saved() {
        let mut model = model();
        model.is_loading = false;
        model.setup = "npm ci".into();
        model.ports = pairs(&mut model, &[("APP_PORT", "8000")]);
        assert!(!model.is_modified(), "nothing read or saved yet");
        model.saved_config = Some(model.config());
        assert!(!model.is_modified());
        model.check = "npm test".into();
        assert!(model.is_modified());
        model.check = "  ".into();
        assert!(!model.is_modified(), "blank is no script, as saved");
        let empty = model.pair("", "");
        model.ports.push(empty);
        assert!(!model.is_modified(), "an empty row isn't saved");
        let config = model.config();
        model.mark_saved(config, "", None);
        model.ports[0].value = "8001".into();
        assert!(model.is_modified());
    }

    #[test]
    fn entries_that_cant_be_saved_are_named() {
        let mut model = model();
        model.ports = pairs(
            &mut model,
            &[
                ("APP_PORT", "8000"),
                ("VITE_PORT", "51a3"),
                ("", "9000"),
                ("WEB PORT", "80"),
                ("", " "),
            ],
        );
        let problems: Vec<Option<String>> =
            model.ports.iter().map(|p| model.problem_port(p)).collect();
        assert_eq!(
            problems,
            vec![
                None,
                Some("Not a port number (1–65535)".into()),
                Some("Needs a name".into()),
                Some("Not a variable name (letters, digits and _)".into()),
                None,
            ]
        );
        model.values = pairs(
            &mut model,
            &[
                ("APP_URL", "http://localhost:{APP_PORT}"),
                ("DB", "app_{tsk}"),
                ("JSON", r#"{"a": 1}"#),
                ("APP_PORT", "1"),
            ],
        );
        let problems: Vec<Option<String>> = model
            .values
            .iter()
            .map(|p| model.problem_value(p))
            .collect();
        assert_eq!(
            problems,
            vec![
                None,
                Some("No {tsk}: use {task}, {task_}, {slot} or a port's name".into()),
                None,
                Some("Also a port".into()),
            ]
        );
        assert_eq!(
            model.problem_port(&model.ports[0]).as_deref(),
            Some("Also a value for each task")
        );
        model.rules = pairs(&mut model, &[("composer.lock", ""), ("a", "x"), ("a", "y")]);
        let problems: Vec<Option<String>> =
            model.rules.iter().map(|p| model.problem_rule(p)).collect();
        assert_eq!(
            problems,
            vec![
                Some("Needs a command".into()),
                Some("a is listed twice".into()),
                Some("a is listed twice".into()),
            ]
        );
        model.actions = vec![
            action(&mut model, true, "test", ""),
            action(&mut model, false, "", "x"),
            action(&mut model, true, "", ""),
        ];
        let problems: Vec<Option<String>> = model
            .actions
            .iter()
            .map(|a| model.problem_action(a))
            .collect();
        assert_eq!(
            problems,
            vec![Some("Needs a command".into()), None, None],
            "unticked and empty rows are skipped"
        );
        model.port_offset = "0".into();
        assert!(model.offset_problem().is_some());
        model.port_offset = String::new();
        assert!(model.offset_problem().is_none(), "empty means the default");
        model.database = Database::Clone;
        assert!(model.database_problem().is_some());
        assert_eq!(model.problem_count(), 11);
    }

    /// Typed or read from a file, numbers that aren't ports never crash the
    /// screen, and can't be saved.
    #[test]
    fn offsets_and_ports_past_the_last_port_are_problems() {
        let mut model = model();
        model.ports = pairs(&mut model, &[("APP_PORT", "8000")]);
        for offset in [
            "0",
            "-5",
            "10001",
            "9223372036854775807",
            "99999999999999999999",
            "x",
        ] {
            model.port_offset = offset.into();
            assert_eq!(
                model.offset_problem().as_deref(),
                Some("A whole number from 1 to 10000"),
                "{offset}"
            );
            assert_eq!(model.offset(), 100);
        }
        let too_big = Some("Too big: task 10's APP_PORT would be past 65535");
        model.port_offset = "6000".into();
        assert_eq!(model.offset_problem().as_deref(), too_big);
        model.port_offset = "5754".into();
        assert_eq!(model.offset_problem().as_deref(), too_big);
        model.port_offset = "5753".into();
        assert_eq!(model.offset_problem(), None, "8000 + 10 × 5753 = 65530");
        model.port_offset = String::new();
        model.ports = pairs(
            &mut model,
            &[("HIGH", "65000"), ("BIG", "9223372036854775807")],
        );
        assert_eq!(
            model.offset_problem().as_deref(),
            Some("Too big: task 10's HIGH would be past 65535"),
            "the default offset too"
        );
        assert_eq!(
            model.problem_port(&model.ports[1]).as_deref(),
            Some("Not a port number (1–65535)")
        );
        let _ = model.config();

        // With the override on, the Compose file's fixed ports count too.
        model.ports = Vec::new();
        model.fixed_ports = vec![FixedPort {
            service: "web".into(),
            port: compose_file::port("65400:80"),
        }];
        assert_eq!(model.offset_problem(), None);
        model.compose_override = true;
        assert_eq!(
            model.offset_problem().as_deref(),
            Some("Too big: task 10's web 65400 would be past 65535")
        );
    }

    #[test]
    fn values_cant_hold_a_line_break() {
        let mut model = model();
        model.values = pairs(
            &mut model,
            &[
                ("APP_NAME", "Bob's {task}"),
                ("NOTE", "a\nb"),
                ("DB", "app_{task_}"),
            ],
        );
        let problems: Vec<Option<String>> = model
            .values
            .iter()
            .map(|p| model.problem_value(p))
            .collect();
        assert_eq!(
            problems,
            vec![None, Some("Can't hold a line break".into()), None]
        );
    }

    #[test]
    fn suggestions_fill_a_project_without_settings_and_saved_ones_win() {
        let found = ProjectSuggestions {
            copies: vec![detector::Entry::new(".env", true)],
            ports: BTreeMap::from([("APP_PORT".to_string(), 8000)]),
            setup: Some("npm ci".into()),
            actions: vec![Action::new("dev", "npm run dev")],
            ..Default::default()
        };
        let mut fresh = SetupModel::default();
        fresh.fill(None, found.clone(), None, None);
        assert!(fresh.copies[0].is_on);
        assert_eq!(fresh.setup, "npm ci");
        assert_eq!(fresh.ports.len(), 1);
        assert!(
            !fresh.actions[0].is_on,
            "found actions are offered, not chosen"
        );
        assert_eq!(
            fresh.database_options,
            vec![Database::None, Database::Clone]
        );

        let saved = ProjectConfig {
            worktree_copy: vec!["config/local.json".into()],
            actions: vec![Action::new("test", "npm test")],
            ..Default::default()
        };
        let mut model = SetupModel::default();
        model.fill(Some(&saved), found, Some("DUMP".into()), None);
        assert!(!model.copies[0].is_on, "not chosen in the saved settings");
        assert!(model.copies[1].is_custom && model.copies[1].is_on);
        assert_eq!(model.setup, "", "an emptied field stays empty");
        assert!(model.ports.is_empty());
        let names: Vec<(&str, bool)> = model
            .actions
            .iter()
            .map(|a| (a.action.name.as_str(), a.is_on))
            .collect();
        assert_eq!(names, vec![("test", true), ("dev", false)]);
        assert_eq!(model.database_options.last(), Some(&Database::Dump));
        model.is_loading = false;
        model.saved_config = Some(model.config());
        assert_eq!(model.config().worktree_copy, saved.worktree_copy);
        assert_eq!(model.config().actions, saved.actions);
    }

    #[test]
    fn the_local_file_is_folded_in_when_impulse_wrote_it() {
        let committed = "[scripts]\nsetup = \"npm ci\"\n";
        let local = settings_file::text(
            &ProjectConfig {
                landing: Some(Landing::Review),
                ..Default::default()
            },
            None,
        );
        let (config, file) = saved_settings(Some(committed), Some(&local));
        let config = config.unwrap();
        assert_eq!(config.setup_script.as_deref(), Some("npm ci"));
        assert_eq!(config.landing, Some(Landing::Review));
        assert!(matches!(file, Some(LocalFile::Impulses { .. })));

        let hand = "# mine\n[scripts]\ncheck = \"make\"\n";
        let (config, file) = saved_settings(Some(committed), Some(hand));
        assert_eq!(config.unwrap().check_script, None, "the project's own");
        assert_eq!(
            file,
            Some(LocalFile::HandEdited {
                overrides: vec!["check script".into()]
            })
        );

        let (_, file) = saved_settings(Some(committed), Some("[scripts"));
        assert!(matches!(file, Some(LocalFile::Unreadable(_))));
        assert_eq!(saved_settings(None, None), (None, None));
    }

    #[test]
    fn placeholders_and_names() {
        assert_eq!(
            placeholders("{task}-{ x }{APP_PORT}{\"a\": 1}{"),
            vec!["task", "APP_PORT"]
        );
        assert!(is_variable_name("_A1"));
        assert!(!is_variable_name("1A"));
        assert!(!is_variable_name("É"));
        assert!(!is_variable_name(""));
        assert_eq!(list(&["a".into(), "b".into(), "c".into()]), "a, b and c");
    }

    #[test]
    fn sizes_read_as_finder_shows_them() {
        assert_eq!(byte_count(0), "Zero KB");
        assert_eq!(byte_count(1), "1 byte");
        assert_eq!(byte_count(999), "999 bytes");
        assert_eq!(byte_count(1_500), "2 KB");
        assert_eq!(byte_count(212_000_000), "212 MB");
        assert_eq!(byte_count(212_340_000), "212.3 MB");
        assert_eq!(byte_count(1_234_567_890), "1.23 GB");
    }
}
