//! Files with unsaved edits (`impulse unsaved`, `list_unsaved_files`) and
//! the project's settings (`impulse project`, `get_project_config`).
//!
//! The summary reads [`ProjectScripts`], the part of the Swift
//! `ProjectConfig` it shows, and works out a task's ports as the Swift
//! `TaskEnvironment` does ([`task_ports`]); both are ported here until
//! project settings are.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::task_detail::ports_text;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsavedList {
    pub files: Vec<UnsavedFile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsavedFile {
    pub path: String,
    /// The file on disk changed too since it was opened or last saved.
    pub changed_on_disk: bool,
}

impl UnsavedList {
    /// Sorted by path.
    pub fn new(mut files: Vec<UnsavedFile>) -> Self {
        files.sort_by(|a, b| a.path.cmp(&b.path));
        UnsavedList { files }
    }

    pub fn text(&self) -> String {
        if self.files.is_empty() {
            return "No files with unsaved edits.".to_string();
        }
        let files: Vec<String> = self
            .files
            .iter()
            .map(|file| {
                format!(
                    "  {}{}",
                    file.path,
                    if file.changed_on_disk {
                        " (also changed on disk)"
                    } else {
                        ""
                    }
                )
            })
            .collect();
        format!(
            "Open with unsaved edits (ask the user to save before changing them):\n{}",
            files.join("\n")
        )
    }
}

/// What the project summary shows of a repository's project settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectScripts {
    pub check_script: Option<String>,
    pub setup_script: Option<String>,
    pub archive_script: Option<String>,
    /// Palette actions: name and command.
    pub actions: Vec<(String, String)>,
    /// `[on_change]`: a file, and the command to run when it changes.
    pub on_change: BTreeMap<String, String>,
    pub env_file: String,
    /// `[worktrees.ports]`: the main checkout's ports, moved by
    /// `port_offset` per slot.
    pub ports: BTreeMap<String, i64>,
    pub port_offset: i64,
}

impl Default for ProjectScripts {
    fn default() -> Self {
        ProjectScripts {
            check_script: None,
            setup_script: None,
            archive_script: None,
            actions: Vec::new(),
            on_change: BTreeMap::new(),
            env_file: ".env".to_string(),
            ports: BTreeMap::new(),
            port_offset: 100,
        }
    }
}

/// `port` for `slot`: plus `slot` × `offset`. `None` when that isn't a
/// port (past 65535, or too big to work out).
pub fn task_port(port: i64, slot: i64, offset: i64) -> Option<i64> {
    let moved = port.checked_add(slot.checked_mul(offset)?)?;
    (1..=65535).contains(&moved).then_some(moved)
}

/// The ports for `slot`: each of the main checkout's plus `slot` ×
/// `offset`. Slot 0 is the main checkout. `None` when one of them isn't a
/// port, so the slot can't be used.
pub fn task_ports(
    base: &BTreeMap<String, i64>,
    slot: i64,
    offset: i64,
) -> Option<BTreeMap<String, i64>> {
    base.iter()
        .map(|(name, port)| Some((name.clone(), task_port(*port, slot, offset)?)))
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    /// The settings' commands are listed: every file with commands is
    /// trusted.
    pub trusted: bool,
    /// Settings files found (relative to the repository).
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<String>,
    pub actions: Vec<Action>,
    pub on_change: Vec<Rule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<i64>,
    pub ports: BTreeMap<String, i64>,
    pub detected: Detected,
    /// Why commands aren't listed, or that there are no settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub name: String,
    pub command: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub file: String,
    pub command: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Detected {
    pub lock_files: Vec<String>,
    pub install: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compose_file: Option<String>,
}

impl ProjectSummary {
    /// From settings the user trusted (`config`), or only what Impulse found
    /// itself (`config` `None`, or not `trusted`).
    pub fn new(
        config: Option<&ProjectScripts>,
        trusted: bool,
        files: Vec<String>,
        slot: Option<i64>,
        detected: Detected,
        note: Option<String>,
    ) -> Self {
        let shown = if trusted { config } else { None };
        let ports = match (config, slot) {
            (Some(config), Some(slot)) if trusted => {
                task_ports(&config.ports, slot, config.port_offset).unwrap_or_default()
            }
            _ => BTreeMap::new(),
        };
        ProjectSummary {
            trusted,
            files,
            check: shown.and_then(|c| c.check_script.clone()),
            setup: shown.and_then(|c| c.setup_script.clone()),
            archive: shown.and_then(|c| c.archive_script.clone()),
            actions: shown
                .map(|c| {
                    c.actions
                        .iter()
                        .map(|(name, command)| Action {
                            name: name.clone(),
                            command: command.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            on_change: shown
                .map(|c| {
                    c.on_change
                        .iter()
                        .map(|(file, command)| Rule {
                            file: file.clone(),
                            command: command.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            env_file: config.map(|c| c.env_file.clone()),
            slot,
            ports,
            detected,
            note,
        }
    }

    pub fn text(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        if let Some(note) = &self.note {
            lines.push(note.clone());
        }
        if let Some(check) = &self.check {
            lines.push(format!("check: {check}"));
        }
        if let Some(setup) = &self.setup {
            lines.push(format!("setup: {setup}"));
        }
        if let Some(archive) = &self.archive {
            lines.push(format!("archive: {archive}"));
        }
        if !self.actions.is_empty() {
            lines.push("actions:".to_string());
            lines.extend(
                self.actions
                    .iter()
                    .map(|action| format!("  {}: {}", action.name, action.command)),
            );
        }
        if !self.on_change.is_empty() {
            lines.push("when these change:".to_string());
            lines.extend(
                self.on_change
                    .iter()
                    .map(|rule| format!("  {}: {}", rule.file, rule.command)),
            );
        }
        if let Some(env_file) = &self.env_file {
            lines.push(format!("env file: {env_file}"));
        }
        if let Some(slot) = self.slot {
            let mut line = format!("this task's slot: {slot}");
            if !self.ports.is_empty() {
                line += &format!(" ({})", ports_text(&self.ports));
            }
            lines.push(line);
        }
        if !self.detected.lock_files.is_empty() {
            lines.push(format!(
                "lock files: {} (install: {})",
                self.detected.lock_files.join(", "),
                self.detected.install.join(" && ")
            ));
        }
        if let Some(compose) = &self.detected.compose_file {
            lines.push(format!("compose file: {compose}"));
        }
        if lines.is_empty() {
            "No project settings.".to_string()
        } else {
            lines.join("\n")
        }
    }
}
