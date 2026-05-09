use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};

pub const SESSION_STATE_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(default)]
pub struct SessionState {
    pub version: u32,
    pub windows: Vec<SessionWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_window_index: Option<usize>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(default)]
pub struct SessionWindow {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
    pub tabs: Vec<SessionTab>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_tab_index: Option<usize>,
    pub layout: SessionLayout,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionTab {
    Editor(SessionEditorTab),
    Terminal(SessionTerminalTab),
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(default)]
pub struct SessionEditorTab {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_column: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll_line: Option<u32>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(default)]
pub struct SessionTerminalTab {
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panes: Vec<SessionTerminalPane>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_pane_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_layout: Option<SessionTerminalPaneLayout>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(default)]
pub struct SessionTerminalPane {
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionTerminalPaneLayout {
    Pane(SessionTerminalPaneLeaf),
    Split(SessionTerminalPaneSplit),
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(default)]
pub struct SessionTerminalPaneLeaf {
    pub pane_index: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(default)]
pub struct SessionTerminalPaneSplit {
    pub axis: SessionSplitAxis,
    pub ratio: f32,
    pub first: Box<SessionTerminalPaneLayout>,
    pub second: Box<SessionTerminalPaneLayout>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionLayout {
    TabGroup(SessionTabGroupLayout),
    Split(SessionSplitLayout),
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(default)]
pub struct SessionTabGroupLayout {
    pub tab_indices: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_tab_index: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(default)]
pub struct SessionSplitLayout {
    pub axis: SessionSplitAxis,
    pub ratio: f32,
    pub first: Box<SessionLayout>,
    pub second: Box<SessionLayout>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionSplitAxis {
    #[default]
    Horizontal,
    Vertical,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            version: SESSION_STATE_VERSION,
            windows: Vec::new(),
            active_window_index: None,
        }
    }
}

impl Default for SessionLayout {
    fn default() -> Self {
        SessionLayout::TabGroup(SessionTabGroupLayout::default())
    }
}

impl Default for SessionSplitLayout {
    fn default() -> Self {
        Self {
            axis: SessionSplitAxis::Horizontal,
            ratio: 0.5,
            first: Box::new(SessionLayout::default()),
            second: Box::new(SessionLayout::default()),
        }
    }
}

impl Default for SessionTerminalPaneLayout {
    fn default() -> Self {
        SessionTerminalPaneLayout::Pane(SessionTerminalPaneLeaf::default())
    }
}

impl Default for SessionTerminalPaneSplit {
    fn default() -> Self {
        Self {
            axis: SessionSplitAxis::Horizontal,
            ratio: 0.5,
            first: Box::new(SessionTerminalPaneLayout::default()),
            second: Box::new(SessionTerminalPaneLayout::default()),
        }
    }
}

impl SessionState {
    pub fn from_json(json: &str) -> Result<Self, String> {
        let mut state: SessionState = serde_json::from_str(json)
            .map_err(|e| format!("Failed to parse session state: {e}"))?;
        state.validate()?;
        Ok(state)
    }

    pub fn to_json(&self) -> Result<String, String> {
        let mut state = self.clone();
        state.validate()?;
        serde_json::to_string_pretty(&state)
            .map_err(|e| format!("Failed to serialize session state: {e}"))
    }

    pub fn schema_json() -> String {
        serde_json::to_string_pretty(&schema_for!(SessionState))
            .expect("session state schema must serialize")
    }

    pub fn validate(&mut self) -> Result<(), String> {
        if self.version != SESSION_STATE_VERSION {
            return Err(format!(
                "Unsupported session state version {}; expected {}",
                self.version, SESSION_STATE_VERSION
            ));
        }

        if !index_in_bounds(self.active_window_index, self.windows.len()) {
            self.active_window_index = last_index(self.windows.len());
        }

        for window in &mut self.windows {
            window.validate();
        }

        Ok(())
    }
}

impl SessionWindow {
    fn validate(&mut self) {
        trim_empty_option(&mut self.project_root);
        if !index_in_bounds(self.active_tab_index, self.tabs.len()) {
            self.active_tab_index = last_index(self.tabs.len());
        }

        for tab in &mut self.tabs {
            tab.validate();
        }
        self.layout.validate(self.tabs.len());
    }
}

impl SessionTab {
    fn validate(&mut self) {
        match self {
            SessionTab::Editor(tab) => tab.validate(),
            SessionTab::Terminal(tab) => tab.validate(),
        }
    }
}

impl SessionEditorTab {
    fn validate(&mut self) {
        self.path = self.path.trim().to_string();
    }
}

impl SessionTerminalTab {
    fn validate(&mut self) {
        self.cwd = self.cwd.trim().to_string();
        trim_empty_option(&mut self.title);
        trim_empty_option(&mut self.shell);

        for pane in &mut self.panes {
            pane.validate();
        }

        if self.panes.is_empty() {
            self.active_pane_index = None;
            self.pane_layout = None;
            return;
        }

        if !index_in_bounds(self.active_pane_index, self.panes.len()) {
            self.active_pane_index = last_index(self.panes.len());
        }

        if let Some(layout) = &mut self.pane_layout {
            layout.validate(self.panes.len());
        } else {
            self.pane_layout = Some(SessionTerminalPaneLayout::Pane(SessionTerminalPaneLeaf {
                pane_index: self.active_pane_index.unwrap_or(0),
            }));
        }
    }
}

impl SessionTerminalPane {
    fn validate(&mut self) {
        self.cwd = self.cwd.trim().to_string();
        trim_empty_option(&mut self.title);
        trim_empty_option(&mut self.shell);
    }
}

impl SessionTerminalPaneLayout {
    fn validate(&mut self, pane_count: usize) {
        match self {
            SessionTerminalPaneLayout::Pane(leaf) => leaf.validate(pane_count),
            SessionTerminalPaneLayout::Split(split) => split.validate(pane_count),
        }
    }
}

impl SessionTerminalPaneLeaf {
    fn validate(&mut self, pane_count: usize) {
        if self.pane_index >= pane_count {
            self.pane_index = last_index(pane_count).unwrap_or(0);
        }
    }
}

impl SessionTerminalPaneSplit {
    fn validate(&mut self, pane_count: usize) {
        if !(0.1..=0.9).contains(&self.ratio) {
            self.ratio = 0.5;
        }
        self.first.validate(pane_count);
        self.second.validate(pane_count);
    }
}

impl SessionLayout {
    fn validate(&mut self, tab_count: usize) {
        match self {
            SessionLayout::TabGroup(group) => group.validate(tab_count),
            SessionLayout::Split(split) => split.validate(tab_count),
        }
    }
}

impl SessionTabGroupLayout {
    fn validate(&mut self, tab_count: usize) {
        self.tab_indices.retain(|index| *index < tab_count);
        self.tab_indices.sort_unstable();
        self.tab_indices.dedup();
        if !index_in_bounds(self.active_tab_index, tab_count)
            || !self
                .active_tab_index
                .is_some_and(|index| self.tab_indices.contains(&index))
        {
            self.active_tab_index = self.tab_indices.last().copied();
        }
    }
}

impl SessionSplitLayout {
    fn validate(&mut self, tab_count: usize) {
        if !(0.1..=0.9).contains(&self.ratio) {
            self.ratio = 0.5;
        }
        self.first.validate(tab_count);
        self.second.validate(tab_count);
    }
}

fn last_index(len: usize) -> Option<usize> {
    len.checked_sub(1)
}

fn index_in_bounds(index: Option<usize>, len: usize) -> bool {
    index.is_some_and(|index| index < len)
}

fn trim_empty_option(value: &mut Option<String>) {
    if let Some(text) = value {
        *text = text.trim().to_string();
        if text.is_empty() {
            *value = None;
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_uses_current_version() {
        let state = SessionState::default();
        assert_eq!(state.version, SESSION_STATE_VERSION);
        assert!(state.windows.is_empty());
        assert_eq!(state.active_window_index, None);
    }

    #[test]
    fn roundtrips_editor_and_terminal_tabs() {
        let state = SessionState {
            version: SESSION_STATE_VERSION,
            active_window_index: Some(0),
            windows: vec![SessionWindow {
                project_root: Some("/repo".to_string()),
                active_tab_index: Some(1),
                tabs: vec![
                    SessionTab::Editor(SessionEditorTab {
                        path: "/repo/src/main.rs".to_string(),
                        cursor_line: Some(12),
                        cursor_column: Some(4),
                        scroll_line: Some(8),
                        pinned: true,
                    }),
                    SessionTab::Terminal(SessionTerminalTab {
                        cwd: "/repo".to_string(),
                        title: Some("server".to_string()),
                        shell: Some("zsh".to_string()),
                        pinned: false,
                        panes: vec![
                            SessionTerminalPane {
                                cwd: "/repo".to_string(),
                                title: Some("server".to_string()),
                                shell: Some("zsh".to_string()),
                            },
                            SessionTerminalPane {
                                cwd: "/repo/logs".to_string(),
                                title: Some("logs".to_string()),
                                shell: Some("zsh".to_string()),
                            },
                        ],
                        active_pane_index: Some(1),
                        pane_layout: Some(SessionTerminalPaneLayout::Split(
                            SessionTerminalPaneSplit {
                                axis: SessionSplitAxis::Horizontal,
                                ratio: 0.4,
                                first: Box::new(SessionTerminalPaneLayout::Pane(
                                    SessionTerminalPaneLeaf { pane_index: 0 },
                                )),
                                second: Box::new(SessionTerminalPaneLayout::Pane(
                                    SessionTerminalPaneLeaf { pane_index: 1 },
                                )),
                            },
                        )),
                    }),
                ],
                layout: SessionLayout::TabGroup(SessionTabGroupLayout {
                    tab_indices: vec![0, 1],
                    active_tab_index: Some(1),
                }),
            }],
        };

        let json = state.to_json().unwrap();
        let parsed = SessionState::from_json(&json).unwrap();
        assert_eq!(parsed, state);
    }

    #[test]
    fn rejects_unsupported_versions() {
        let err = SessionState::from_json(r#"{"version":999,"windows":[]}"#).unwrap_err();
        assert!(err.contains("Unsupported session state version 999"));
    }

    #[test]
    fn schema_serializes() {
        let schema: serde_json::Value = serde_json::from_str(&SessionState::schema_json()).unwrap();
        assert_eq!(schema["title"], "SessionState");
    }

    #[test]
    fn fills_missing_fields_for_forward_compatible_reads() {
        let state = SessionState::from_json(
            r#"{
              "version": 1,
              "windows": [
                {
                  "tabs": [
                    { "kind": "editor", "path": "/tmp/a.rs" },
                    { "kind": "terminal", "cwd": "/tmp" }
                  ]
                }
              ]
            }"#,
        )
        .unwrap();

        assert_eq!(state.windows.len(), 1);
        assert_eq!(state.windows[0].active_tab_index, Some(1));
        assert_eq!(
            state.windows[0].layout,
            SessionLayout::TabGroup(SessionTabGroupLayout::default())
        );
    }

    #[test]
    fn validation_clamps_indices_and_split_ratio() {
        let mut state = SessionState {
            version: SESSION_STATE_VERSION,
            active_window_index: Some(9),
            windows: vec![SessionWindow {
                project_root: Some(" /repo ".to_string()),
                active_tab_index: Some(9),
                tabs: vec![SessionTab::Terminal(SessionTerminalTab {
                    cwd: " /repo ".to_string(),
                    title: Some("  ".to_string()),
                    shell: Some(" zsh ".to_string()),
                    pinned: false,
                    panes: vec![SessionTerminalPane {
                        cwd: " /repo/a ".to_string(),
                        title: Some(" logs ".to_string()),
                        shell: Some(" fish ".to_string()),
                    }],
                    active_pane_index: Some(12),
                    pane_layout: Some(SessionTerminalPaneLayout::Split(SessionTerminalPaneSplit {
                        axis: SessionSplitAxis::Vertical,
                        ratio: 1.5,
                        first: Box::new(SessionTerminalPaneLayout::Pane(SessionTerminalPaneLeaf {
                            pane_index: 99,
                        })),
                        second: Box::new(SessionTerminalPaneLayout::Pane(
                            SessionTerminalPaneLeaf { pane_index: 0 },
                        )),
                    })),
                })],
                layout: SessionLayout::Split(SessionSplitLayout {
                    axis: SessionSplitAxis::Vertical,
                    ratio: 1.5,
                    first: Box::new(SessionLayout::TabGroup(SessionTabGroupLayout {
                        tab_indices: vec![0, 99, 0],
                        active_tab_index: Some(99),
                    })),
                    second: Box::new(SessionLayout::TabGroup(SessionTabGroupLayout {
                        tab_indices: vec![99],
                        active_tab_index: Some(99),
                    })),
                }),
            }],
        };

        state.validate().unwrap();

        assert_eq!(state.active_window_index, Some(0));
        assert_eq!(state.windows[0].project_root.as_deref(), Some("/repo"));
        assert_eq!(state.windows[0].active_tab_index, Some(0));
        match &state.windows[0].tabs[0] {
            SessionTab::Terminal(tab) => {
                assert_eq!(tab.cwd, "/repo");
                assert_eq!(tab.title, None);
                assert_eq!(tab.shell.as_deref(), Some("zsh"));
                assert_eq!(tab.active_pane_index, Some(0));
                assert_eq!(tab.panes[0].cwd, "/repo/a");
                assert_eq!(tab.panes[0].title.as_deref(), Some("logs"));
                assert_eq!(tab.panes[0].shell.as_deref(), Some("fish"));
                match tab.pane_layout.as_ref().expect("pane layout") {
                    SessionTerminalPaneLayout::Split(split) => {
                        assert_eq!(split.ratio, 0.5);
                        match split.first.as_ref() {
                            SessionTerminalPaneLayout::Pane(leaf) => {
                                assert_eq!(leaf.pane_index, 0);
                            }
                            SessionTerminalPaneLayout::Split(_) => panic!("expected pane leaf"),
                        }
                    }
                    SessionTerminalPaneLayout::Pane(_) => panic!("expected split"),
                }
            }
            SessionTab::Editor(_) => panic!("expected terminal tab"),
        }
        match &state.windows[0].layout {
            SessionLayout::Split(split) => {
                assert_eq!(split.ratio, 0.5);
                match split.first.as_ref() {
                    SessionLayout::TabGroup(group) => {
                        assert_eq!(group.tab_indices, vec![0]);
                        assert_eq!(group.active_tab_index, Some(0));
                    }
                    SessionLayout::Split(_) => panic!("expected tab group"),
                }
                match split.second.as_ref() {
                    SessionLayout::TabGroup(group) => {
                        assert!(group.tab_indices.is_empty());
                        assert_eq!(group.active_tab_index, None);
                    }
                    SessionLayout::Split(_) => panic!("expected tab group"),
                }
            }
            SessionLayout::TabGroup(_) => panic!("expected split"),
        }
    }
}
