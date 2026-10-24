//! `.impulse/project.toml`: what a repository tells Impulse about itself,
//! that is actions to run from the palette, scripts for new and archived
//! task worktrees, and files to copy into them. Commands from it only run
//! after the user trusts that exact file.
//!
//! The same settings can also live in `.git/impulse/project.toml`, inside
//! the repository's shared git folder: settings for this machine only,
//! never committed, seen by every worktree at once. Where both files set a
//! key, the local one wins.

use std::collections::BTreeMap;
use std::fs;
use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::task_environment::PORT_RANGE;
use crate::git::task_registry::{join, TaskRegistry};
use crate::trust;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Action {
    pub name: String,
    pub command: String,
    /// Folder to run in, relative to the repository root.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Where the terminal opens: "tab" (default), "right" or "down".
    #[serde(default)]
    pub open: Option<String>,
}

impl Action {
    pub fn new(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
            cwd: None,
            open: None,
        }
    }
}

/// How Finish Task lands a task's work (`[finish] land`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Landing {
    /// Merge the branch into its base and push the base.
    Merge,
    /// Push the branch for review on the git host.
    Review,
}

impl Landing {
    pub const ALL: [Landing; 2] = [Landing::Merge, Landing::Review];

    pub fn as_str(self) -> &'static str {
        match self {
            Landing::Merge => "merge",
            Landing::Review => "review",
        }
    }

    pub fn parse(text: &str) -> Option<Landing> {
        Landing::ALL.into_iter().find(|l| l.as_str() == text)
    }
}

/// The offsets read. A bigger one is a mistake: a few tasks in, their ports
/// would pass 65535.
pub const PORT_OFFSETS: RangeInclusive<i64> = 1..=10000;

/// The committed file, relative to the repository root.
pub const RELATIVE_PATH: &str = ".impulse/project.toml";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectConfig {
    pub actions: Vec<Action>,
    /// Run in a new task worktree after it's created.
    pub setup_script: Option<String>,
    /// Run in a task worktree before it's archived.
    pub archive_script: Option<String>,
    /// Checks that a task is ready to land: types, tests (`[scripts] check`).
    pub check_script: Option<String>,
    /// Commands for when files change in a pull, merge or checkout, by file
    /// name (`[on_change]`: `"composer.lock" = "composer install"`).
    pub on_change: BTreeMap<String, String>,
    /// Untracked files to copy into new task worktrees (globs allowed).
    pub worktree_copy: Vec<String>,
    /// The dotenv file a new task's own values are written into
    /// (`[worktrees] env_file`): the main checkout's copy, then updated.
    pub env_file: String,
    /// What task n adds to every port: n times this
    /// (`[worktrees] port_offset`).
    pub port_offset: i64,
    /// The main checkout's ports, by their name in the env file
    /// (`[worktrees.ports]`).
    pub ports: BTreeMap<String, i64>,
    /// Values that differ per task, with placeholders (`[worktrees.env]`).
    pub worktree_env: BTreeMap<String, String>,
    /// Folders cloned into new tasks from the main checkout
    /// (`[worktrees] clone`).
    pub worktree_clone: Vec<String>,
    /// A database's data folder cloned into new tasks
    /// (`[worktrees.database] clone`), with the Compose service that writes
    /// it stopped in the main checkout meanwhile (`service`).
    pub database_folder: Option<String>,
    pub database_service: Option<String>,
    /// Give each new task a Compose override that renames its containers
    /// and moves its fixed ports (`[worktrees] compose_override`).
    pub compose_override: bool,
    /// Files that don't count when workspaces change the same files
    /// (`[worktrees] overlap_ignore`): names, paths or patterns.
    pub overlap_ignore: Vec<String>,
    /// How Finish Task lands work; `None` until chosen (the first Finish
    /// asks).
    pub landing: Option<Landing>,
    /// What the files set that can't be used, so `ports` and `port_offset`
    /// leave it out: a port outside 1-65535, an offset outside
    /// `PORT_OFFSETS`. Project Setup shows it, to be fixed.
    pub unusable_ports: BTreeMap<String, i64>,
    pub unusable_port_offset: Option<i64>,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            actions: Vec::new(),
            setup_script: None,
            archive_script: None,
            check_script: None,
            on_change: BTreeMap::new(),
            worktree_copy: Vec::new(),
            env_file: ".env".into(),
            port_offset: 100,
            ports: BTreeMap::new(),
            worktree_env: BTreeMap::new(),
            worktree_clone: Vec::new(),
            database_folder: None,
            database_service: None,
            compose_override: false,
            overlap_ignore: Vec::new(),
            landing: None,
            unusable_ports: BTreeMap::new(),
            unusable_port_offset: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    Invalid(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Invalid(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for LoadError {}

/// One file's settings. Keys the file doesn't set stay `None`, so a later
/// file only overrides what it sets.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Layer {
    actions: Option<Vec<Action>>,
    scripts: Option<Scripts>,
    worktrees: Option<Worktrees>,
    on_change: Option<BTreeMap<String, String>>,
    finish: Option<Finish>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct Scripts {
    setup: Option<String>,
    archive: Option<String>,
    check: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct Worktrees {
    copy: Option<Vec<String>>,
    env_file: Option<String>,
    port_offset: Option<i64>,
    ports: Option<BTreeMap<String, i64>>,
    env: Option<BTreeMap<String, String>>,
    clone: Option<Vec<String>>,
    database: Option<Database>,
    compose_override: Option<bool>,
    overlap_ignore: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct Database {
    clone: Option<String>,
    service: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct Finish {
    land: Option<String>,
}

impl ProjectConfig {
    /// Every command the file can run, for the trust prompt.
    pub fn commands(&self) -> Vec<String> {
        let mut commands: Vec<String> = self.actions.iter().map(|a| a.command.clone()).collect();
        commands.extend(
            [&self.setup_script, &self.archive_script, &self.check_script]
                .into_iter()
                .flatten()
                .cloned(),
        );
        commands.extend(self.on_change.values().cloned());
        if let (Some(_), Some(service)) = (&self.database_folder, &self.database_service) {
            commands.push(format!("docker compose stop {service}"));
            commands.push(format!("docker compose start {service}"));
        }
        commands
    }

    /// Whether new tasks get values of their own (ports, env).
    pub fn has_task_values(&self) -> bool {
        !self.ports.is_empty() || !self.worktree_env.is_empty()
    }

    /// The names a new task's values are written under, for the trust
    /// prompt.
    pub fn task_value_names(&self) -> Vec<String> {
        self.ports
            .keys()
            .chain(self.worktree_env.keys())
            .cloned()
            .collect()
    }

    /// The local file: settings for this machine only, in the repository's
    /// shared git folder (`.git/impulse/project.toml`).
    pub fn local_path(common_git_directory: &str) -> String {
        join(
            &TaskRegistry::directory(common_git_directory),
            "project.toml",
        )
    }

    pub fn parse(text: &str) -> Result<ProjectConfig, LoadError> {
        parse_layer(text).map(|layer| resolve(&[layer]))
    }

    /// The settings of several files' texts, later ones winning key by key,
    /// as `load` layers the local file over the committed one.
    pub fn parse_layers<S: AsRef<str>>(texts: &[S]) -> Result<ProjectConfig, LoadError> {
        let layers = texts
            .iter()
            .map(|text| parse_layer(text.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(resolve(&layers))
    }

    /// The settings that differ from `other`'s, named as Project Setup shows
    /// them ("setup script", "ports"), in its order.
    pub fn differences(&self, other: &ProjectConfig) -> Vec<&'static str> {
        let settings = [
            ("files to copy", self.worktree_copy == other.worktree_copy),
            (
                "folders to clone",
                self.worktree_clone == other.worktree_clone,
            ),
            (
                "database",
                self.database_folder == other.database_folder
                    && self.database_service == other.database_service,
            ),
            (
                "ports",
                self.ports == other.ports
                    && self.port_offset == other.port_offset
                    && self.compose_override == other.compose_override,
            ),
            (
                "values",
                self.worktree_env == other.worktree_env && self.env_file == other.env_file,
            ),
            ("setup script", self.setup_script == other.setup_script),
            ("check script", self.check_script == other.check_script),
            (
                "archive script",
                self.archive_script == other.archive_script,
            ),
            ("how tasks finish", self.landing == other.landing),
            (
                "commands for changed files",
                self.on_change == other.on_change,
            ),
            (
                "files ignored for overlaps",
                self.overlap_ignore == other.overlap_ignore,
            ),
            ("actions", self.actions == other.actions),
        ];
        settings
            .into_iter()
            .filter(|(_, same)| !same)
            .map(|(name, _)| name)
            .collect()
    }

    /// The settings for the checkout at `root`: its own
    /// `.impulse/project.toml` and, given the repository's shared git
    /// folder, the local file. `None` when neither exists.
    pub fn load(root: &str, common_git_directory: Option<&str>) -> Option<Loaded> {
        let committed_path = join(root, RELATIVE_PATH);
        let local_path = common_git_directory.map(Self::local_path);
        let mut layers = Vec::new();
        let mut sources: [Option<Source>; 2] = [None, None];
        let mut failure: Option<LoadError> = None;
        for (index, path) in [Some(committed_path), local_path].into_iter().enumerate() {
            let Some(path) = path else { continue };
            let Ok(data) = fs::read(&path) else { continue };
            match parse_layer(&String::from_utf8_lossy(&data)) {
                Ok(layer) => {
                    let alone = resolve(std::slice::from_ref(&layer));
                    layers.push(layer);
                    sources[index] = Some(Source {
                        path,
                        digest: digest(&data),
                        commands: alone.commands(),
                        values: alone.task_value_names(),
                    });
                }
                Err(LoadError::Invalid(message)) => {
                    let name = if index == 0 {
                        RELATIVE_PATH
                    } else {
                        ".git/impulse/project.toml"
                    };
                    failure.get_or_insert(LoadError::Invalid(format!("{name}: {message}")));
                    sources[index] = Some(Source {
                        path,
                        digest: digest(&data),
                        commands: Vec::new(),
                        values: Vec::new(),
                    });
                }
            }
        }
        if sources.iter().all(Option::is_none) {
            return None;
        }
        let [committed, local] = sources;
        Some(Loaded {
            config: match failure {
                Some(error) => Err(error),
                None => Ok(resolve(&layers)),
            },
            committed,
            local,
        })
    }
}

pub(crate) fn parse_layer(text: &str) -> Result<Layer, LoadError> {
    toml::from_str(text).map_err(|e| LoadError::Invalid(e.to_string()))
}

/// The settings of `layers`, later layers winning key by key. Actions are
/// keyed by name: a later action replaces an earlier one of the same name.
/// An empty script (`setup = ""`) clears an earlier one.
pub(crate) fn resolve(layers: &[Layer]) -> ProjectConfig {
    let mut config = ProjectConfig::default();
    let mut setup: Option<&String> = None;
    let mut archive: Option<&String> = None;
    let mut check: Option<&String> = None;
    for layer in layers {
        for action in layer.actions.iter().flatten() {
            if action.name.is_empty() || action.command.is_empty() {
                continue;
            }
            match config.actions.iter_mut().find(|a| a.name == action.name) {
                Some(existing) => *existing = action.clone(),
                None => config.actions.push(action.clone()),
            }
        }
        if let Some(scripts) = &layer.scripts {
            setup = scripts.setup.as_ref().or(setup);
            archive = scripts.archive.as_ref().or(archive);
            check = scripts.check.as_ref().or(check);
        }
        if let Some(on_change) = &layer.on_change {
            config
                .on_change
                .extend(on_change.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        if let Some(worktrees) = &layer.worktrees {
            if let Some(copy) = &worktrees.copy {
                config.worktree_copy = copy.clone();
            }
            if let Some(file) = worktrees.env_file.as_ref().filter(|f| !f.is_empty()) {
                config.env_file = file.clone();
            }
            // Ports are worked out from these values unasked (the New Task
            // sheet, a task's summary), before the file is trusted: ones
            // that aren't ports are kept aside, not used.
            if let Some(offset) = worktrees.port_offset {
                if PORT_OFFSETS.contains(&offset) {
                    config.port_offset = offset;
                    config.unusable_port_offset = None;
                } else {
                    config.unusable_port_offset = Some(offset);
                }
            }
            for (name, &port) in worktrees.ports.iter().flatten() {
                if PORT_RANGE.contains(&port) {
                    config.ports.insert(name.clone(), port);
                    config.unusable_ports.remove(name);
                } else {
                    config.unusable_ports.insert(name.clone(), port);
                }
            }
            if let Some(env) = &worktrees.env {
                config
                    .worktree_env
                    .extend(env.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
            if let Some(clone) = &worktrees.clone {
                config.worktree_clone = clone.clone();
            }
            if let Some(database) = &worktrees.database {
                if let Some(folder) = &database.clone {
                    config.database_folder = (!folder.is_empty()).then(|| folder.clone());
                }
                if let Some(service) = &database.service {
                    config.database_service = (!service.is_empty()).then(|| service.clone());
                }
            }
            if let Some(value) = worktrees.compose_override {
                config.compose_override = value;
            }
            if let Some(ignore) = &worktrees.overlap_ignore {
                config.overlap_ignore = ignore.clone();
            }
        }
        if let Some(land) = layer.finish.as_ref().and_then(|f| f.land.as_ref()) {
            config.landing = Landing::parse(land);
        }
    }
    let script = |value: Option<&String>| value.filter(|s| !s.is_empty()).cloned();
    config.setup_script = script(setup);
    config.archive_script = script(archive);
    config.check_script = script(check);
    config.on_change.retain(|_, command| !command.is_empty());
    config
}

/// One settings file as read: where it is, the SHA-256 of its bytes (trust
/// is for exactly this content), the commands it can run and the per-task
/// values it sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub path: String,
    pub digest: String,
    pub commands: Vec<String>,
    pub values: Vec<String>,
}

/// The settings for a checkout, from both files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded {
    /// Both files together, local winning; a failure names the file.
    pub config: Result<ProjectConfig, LoadError>,
    /// `.impulse/project.toml` in the checkout.
    pub committed: Option<Source>,
    /// `.git/impulse/project.toml`.
    pub local: Option<Source>,
}

impl Loaded {
    /// The files that need trusting: they run commands or set values.
    pub fn sources(&self) -> Vec<&Source> {
        [&self.committed, &self.local]
            .into_iter()
            .flatten()
            .filter(|s| !s.commands.is_empty() || !s.values.is_empty())
            .collect()
    }
}

/// SHA-256 of the file's bytes, in hex: trust is for exactly this content.
pub fn digest(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Which project.toml files the user has trusted, by repository and
/// content digest. Editing the file asks again. Stored as the Swift app
/// stores it (`{"trusted": {root: digest}}`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectTrust {
    #[serde(default)]
    pub trusted: BTreeMap<String, String>,
}

impl ProjectTrust {
    pub fn is_trusted(&self, root: &str, digest: &str) -> bool {
        self.trusted.get(root).map(String::as_str) == Some(digest)
    }

    pub fn trust(&mut self, root: &str, digest: &str) {
        self.trusted.insert(root.to_string(), digest.to_string());
    }

    pub fn revoke(&mut self, root: &str) {
        self.trusted.remove(root);
    }

    /// Forget the files of `folder` and of every repository inside it.
    pub fn revoke_within(&mut self, folder: &str) {
        let folder = trust::normalize(folder);
        self.trusted
            .retain(|root, _| !contains(&folder, &trust::normalize(root)));
    }

    pub fn revoke_all(&mut self) {
        self.trusted.clear();
    }
}

/// `folder` is `path` or one of its ancestors.
fn contains(folder: &str, path: &str) -> bool {
    folder == "/" || path == folder || path.starts_with(&format!("{folder}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_actions_scripts_and_copy_list() {
        let text = r#"
[scripts]
setup = "npm ci"
archive = ""

[worktrees]
copy = ["local.config", "config/*.yml"]

[[actions]]
name = "Dev server"
command = "npm run dev"
cwd = "web"
open = "right"

[[actions]]
name = "Tests"
command = "npm test"

[[actions]]
name = ""
command = "ignored"
"#;
        let config = ProjectConfig::parse(text).unwrap();
        let names: Vec<&str> = config.actions.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Dev server", "Tests"]);
        assert_eq!(config.actions[0].cwd.as_deref(), Some("web"));
        assert_eq!(config.actions[0].open.as_deref(), Some("right"));
        assert_eq!(config.setup_script.as_deref(), Some("npm ci"));
        assert_eq!(config.archive_script, None, "empty scripts are dropped");
        assert_eq!(config.worktree_copy, ["local.config", "config/*.yml"]);
        assert_eq!(config.commands(), ["npm run dev", "npm test", "npm ci"]);
    }

    #[test]
    fn empty_and_broken_files() {
        assert_eq!(ProjectConfig::parse("").unwrap(), ProjectConfig::default());
        assert!(
            ProjectConfig::parse("[[actions]\nname = ").is_err(),
            "broken TOML should fail"
        );
    }

    #[test]
    fn trust_is_per_content() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        fs::create_dir_all(format!("{root}/.impulse")).unwrap();
        let file = format!("{root}/.impulse/project.toml");
        assert_eq!(ProjectConfig::load(&root, None), None);
        fs::write(&file, "[[actions]]\nname = \"A\"\ncommand = \"echo a\"\n").unwrap();
        let first = ProjectConfig::load(&root, None).unwrap().committed.unwrap();
        let mut trust = ProjectTrust::default();
        assert!(!trust.is_trusted(&root, &first.digest));
        trust.trust(&root, &first.digest);
        assert!(trust.is_trusted(&root, &first.digest));
        fs::write(&file, "[[actions]]\nname = \"A\"\ncommand = \"echo b\"\n").unwrap();
        let edited = ProjectConfig::load(&root, None).unwrap().committed.unwrap();
        assert!(
            !trust.is_trusted(&root, &edited.digest),
            "editing asks again"
        );
    }

    #[test]
    fn local_file_wins_key_by_key() {
        let committed = r#"
[scripts]
setup = "npm ci"
archive = "docker compose down"

[worktrees]
copy = [".env"]

[[actions]]
name = "dev"
command = "npm run dev"

[[actions]]
name = "test"
command = "npm test"
"#;
        let local = r#"
[scripts]
archive = ""

[worktrees]
copy = [".env", "config/*.local.json"]

[[actions]]
name = "test"
command = "npm run test:run"

[[actions]]
name = "lint"
command = "npm run lint"
"#;
        let config = resolve(&[parse_layer(committed).unwrap(), parse_layer(local).unwrap()]);
        assert_eq!(
            config.setup_script.as_deref(),
            Some("npm ci"),
            "a key the local file doesn't set comes from the committed one"
        );
        assert_eq!(
            config.archive_script, None,
            "an empty local script clears the committed one"
        );
        assert_eq!(
            config.worktree_copy,
            [".env", "config/*.local.json"],
            "a local list replaces the committed one"
        );
        let names: Vec<&str> = config.actions.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["dev", "test", "lint"]);
        assert_eq!(
            config.actions[1].command, "npm run test:run",
            "a local action replaces one of the same name"
        );
    }

    #[test]
    fn layers_and_what_differs() {
        let committed = "[scripts]\nsetup = \"docker-composer install\"\n\n[worktrees]\ncompose_override = true\n";
        let local = "[scripts]\nsetup = \"composer install\"\n\n[finish]\nland = \"merge\"\n";
        let applied = ProjectConfig::parse_layers(&[committed, local]).unwrap();
        let project_wins = ProjectConfig::parse_layers(&[local, committed]).unwrap();
        assert_eq!(applied.setup_script.as_deref(), Some("composer install"));
        assert_eq!(
            project_wins.setup_script.as_deref(),
            Some("docker-composer install")
        );
        assert_eq!(
            project_wins.landing,
            Some(Landing::Merge),
            "a key only the earlier file sets stays"
        );
        assert_eq!(applied.differences(&project_wins), ["setup script"]);
        assert_eq!(
            applied.differences(&ProjectConfig::parse(committed).unwrap()),
            ["setup script", "how tasks finish"]
        );
        assert!(applied.differences(&applied).is_empty());
        assert!(ProjectConfig::parse_layers(&[committed, "[scripts"]).is_err());
    }

    #[test]
    fn loads_both_files_and_names_the_broken_one() {
        let dir = tempfile::tempdir().unwrap();
        let root = format!("{}/repo", dir.path().display());
        let common = format!("{root}/.git");
        fs::create_dir_all(format!("{root}/.impulse")).unwrap();
        fs::create_dir_all(format!("{common}/impulse")).unwrap();

        let local_path = ProjectConfig::local_path(&common);
        assert_eq!(local_path, format!("{common}/impulse/project.toml"));
        fs::write(&local_path, "[scripts]\nsetup = \"composer install\"\n").unwrap();
        let only_local = ProjectConfig::load(&root, Some(&common)).unwrap();
        assert_eq!(only_local.committed, None);
        assert_eq!(
            only_local.local.as_ref().unwrap().commands,
            ["composer install"]
        );
        assert_eq!(
            only_local.config.as_ref().unwrap().setup_script.as_deref(),
            Some("composer install")
        );
        let paths: Vec<&str> = only_local
            .sources()
            .iter()
            .map(|s| s.path.as_str())
            .collect();
        assert_eq!(paths, [local_path.as_str()]);

        let committed_path = format!("{root}/{RELATIVE_PATH}");
        fs::write(
            &committed_path,
            "[[actions]]\nname = \"dev\"\ncommand = \"npm run dev\"\n",
        )
        .unwrap();
        let both = ProjectConfig::load(&root, Some(&common)).unwrap();
        let paths: Vec<&str> = both.sources().iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths, [committed_path.as_str(), local_path.as_str()]);
        assert_eq!(
            both.config.unwrap().commands(),
            ["npm run dev", "composer install"]
        );

        fs::write(&local_path, "[scripts\n").unwrap();
        let broken = ProjectConfig::load(&root, Some(&common)).unwrap();
        match broken.config {
            Err(LoadError::Invalid(message)) => {
                assert!(
                    message.starts_with(".git/impulse/project.toml: "),
                    "{message}"
                )
            }
            Ok(_) => panic!("a broken local file fails the whole config"),
        }
    }

    #[test]
    fn trust_can_be_revoked_by_folder_or_all() {
        let mut trust = ProjectTrust {
            trusted: [
                ("/Users/me/Code/app", "a"),
                ("/Users/me/Code/api", "b"),
                ("/Users/me/Code/apps", "c"),
                ("/Users/me/Other/x", "d"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        };
        trust.revoke_within("/Users/me/Code/app");
        let keys: Vec<&str> = trust.trusted.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "/Users/me/Code/api",
                "/Users/me/Code/apps",
                "/Users/me/Other/x"
            ]
        );
        trust.revoke_within("/Users/me/Code/");
        let keys: Vec<&str> = trust.trusted.keys().map(String::as_str).collect();
        assert_eq!(keys, ["/Users/me/Other/x"]);
        trust.revoke_all();
        assert!(trust.trusted.is_empty());
    }
}
