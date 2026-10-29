//! Impulse's built-in shortcuts (the Swift `Keybindings`): one id per
//! command, its description and category for the Keyboard Shortcuts tab,
//! and its default on each platform. `keybinding_overrides` in
//! settings.json replaces a default ("Cmd+Shift+B"), or removes it
//! ("none").
//!
//! On the Mac the defaults are the Swift app's. On Linux, Cmd becomes Ctrl,
//! as the v0.29 Linux app had it, except where Ctrl would take a chord the
//! Mac spells with both Ctrl and Cmd, or a key a shell needs (Ctrl+B, Ctrl+I,
//! Ctrl+D): those get Shift or Alt instead. In a terminal, the app layer
//! leaves plain Ctrl+letter to the shell.

use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Linux,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::Mac
        } else {
            Platform::Linux
        }
    }
}

/// A built-in shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Builtin {
    pub id: &'static str,
    pub description: &'static str,
    pub category: &'static str,
    /// The Mac default, as the Swift app writes it ("Cmd+Shift+T"); empty
    /// when the command has none.
    pub mac: &'static str,
}

const fn b(
    id: &'static str,
    description: &'static str,
    category: &'static str,
    mac: &'static str,
) -> Builtin {
    Builtin {
        id,
        description,
        category,
        mac,
    }
}

/// Every built-in shortcut, in the Swift table's order.
pub const BUILTINS: &[Builtin] = &[
    b("new_tab", "New Tab", "Tabs", "Cmd+T"),
    b("close_tab", "Close Tab", "Tabs", "Cmd+W"),
    b("reopen_tab", "Reopen Closed Tab", "Tabs", "Cmd+Shift+T"),
    b("next_tab", "Show Next Tab", "Tabs", "Ctrl+Tab"),
    b("prev_tab", "Show Previous Tab", "Tabs", "Ctrl+Shift+Tab"),
    b("split_right", "Split Right", "Panes", "Cmd+D"),
    b("split_down", "Split Down", "Panes", "Cmd+Shift+D"),
    b(
        "focus_pane_left",
        "Focus Pane Left",
        "Panes",
        "Cmd+Alt+Left",
    ),
    b(
        "focus_pane_right",
        "Focus Pane Right",
        "Panes",
        "Cmd+Alt+Right",
    ),
    b("focus_pane_up", "Focus Pane Above", "Panes", "Cmd+Alt+Up"),
    b(
        "focus_pane_down",
        "Focus Pane Below",
        "Panes",
        "Cmd+Alt+Down",
    ),
    b(
        "resize_pane_left",
        "Grow Pane Left",
        "Panes",
        "Ctrl+Cmd+Left",
    ),
    b(
        "resize_pane_right",
        "Grow Pane Right",
        "Panes",
        "Ctrl+Cmd+Right",
    ),
    b("resize_pane_up", "Grow Pane Up", "Panes", "Ctrl+Cmd+Up"),
    b(
        "resize_pane_down",
        "Grow Pane Down",
        "Panes",
        "Ctrl+Cmd+Down",
    ),
    b("next_pane", "Next Pane", "Panes", ""),
    b("prev_pane", "Previous Pane", "Panes", ""),
    b("zoom_pane", "Zoom Pane", "Panes", "Cmd+Shift+Return"),
    b("terminal_hints", "Show Hints", "Blocks", "Cmd+Shift+Space"),
    b(
        "toggle_block_bookmark",
        "Bookmark Block",
        "Blocks",
        "Cmd+Shift+K",
    ),
    b("previous_block_bookmark", "Previous Bookmark", "Blocks", ""),
    b("next_block_bookmark", "Next Bookmark", "Blocks", ""),
    b("previous_block", "Previous Block", "Blocks", ""),
    b("next_block", "Next Block", "Blocks", ""),
    b("last_failed_block", "Last Failed Block", "Blocks", ""),
    b("select_blocks", "Select Blocks", "Blocks", ""),
    b("equalize_panes", "Even Out Panes", "Panes", "Ctrl+Cmd+="),
    b("move_pane_to_tab", "Move Pane to New Tab", "Panes", ""),
    b("copy", "Copy", "Terminal", "Cmd+C"),
    b("paste", "Paste", "Terminal", "Cmd+V"),
    b("new_file", "New File", "Editor", "Cmd+N"),
    b("save", "Save", "Editor", "Cmd+S"),
    b("find", "Find…", "Editor", "Cmd+F"),
    b("go_to_line", "Go to Line…", "Editor", "Cmd+G"),
    b(
        "toggle_markdown_preview",
        "Toggle Markdown Preview",
        "Editor",
        "Cmd+Shift+M",
    ),
    b("toggle_sidebar", "Toggle Sidebar", "Navigation", "Cmd+B"),
    b(
        "agent_composer",
        "Compose Message to Agent",
        "Terminal",
        "Cmd+I",
    ),
    b(
        "next_agent",
        "Next Agent Needing You",
        "Navigation",
        "Cmd+Shift+U",
    ),
    b(
        "switch_workspace",
        "Switch Workspace…",
        "Navigation",
        "Ctrl+Cmd+O",
    ),
    b(
        "scratch_workspace",
        "Scratch Workspace",
        "Navigation",
        "Ctrl+Cmd+N",
    ),
    b("switch_branch", "Switch Branch…", "Git", "Ctrl+Cmd+B"),
    b("git_fetch", "Fetch", "Git", ""),
    b("git_pull", "Pull", "Git", ""),
    b("git_push", "Push", "Git", ""),
    b("git_create_tag", "Create Tag…", "Git", ""),
    b("git_fetch_all", "Fetch All Remotes", "Git", ""),
    b("git_pull_rebase", "Pull (Rebase)", "Git", ""),
    b("git_force_push", "Force Push (With Lease)…", "Git", ""),
    b("git_push_tags", "Push All Tags", "Git", ""),
    b("git_stash", "Stash All Changes", "Git", ""),
    b("git_pop_stash", "Pop Latest Stash", "Git", ""),
    b("git_undo_commit", "Undo Last Commit", "Git", ""),
    b("git_open_remote", "Open Repository in Browser", "Git", ""),
    b("git_copy_remote_url", "Copy Remote URL", "Git", ""),
    b("git_history", "Show Git History", "Git", "Cmd+Shift+H"),
    b(
        "file_history",
        "Show History of This File",
        "Git",
        "Ctrl+Shift+Cmd+H",
    ),
    b("diff_view", "Toggle Diff View", "Git", "Alt+Cmd+G"),
    b("show_problems", "Show Problems", "Editor", "Ctrl+Cmd+M"),
    b(
        "go_to_project_symbol",
        "Go to Symbol in Project…",
        "Navigation",
        "Alt+Cmd+O",
    ),
    b(
        "project_actions",
        "Run Project Action…",
        "Navigation",
        "Ctrl+Cmd+R",
    ),
    b("new_task", "New Task…", "Navigation", "Alt+Cmd+N"),
    b(
        "new_task_from_branch",
        "New Task from Branch…",
        "Navigation",
        "",
    ),
    b("finish_task", "Finish Task…", "Navigation", ""),
    b("project_setup", "Project Setup…", "Navigation", ""),
    b(
        "review_agent_turn",
        "Review Last Agent Turn",
        "Navigation",
        "Cmd+Shift+I",
    ),
    b(
        "open_keybindings",
        "Keyboard Shortcuts…",
        "Navigation",
        "Alt+Cmd+,",
    ),
    b("manage_branches", "Manage Branches…", "Git", ""),
    b("show_changes", "Show Changes", "Navigation", "Ctrl+Shift+G"),
    b(
        "go_to_symbol",
        "Go to Symbol in File…",
        "Navigation",
        "Cmd+Shift+O",
    ),
    b("quick_open", "Go to File…", "Navigation", "Cmd+P"),
    b("show_files", "Show Files", "Navigation", "Cmd+Shift+E"),
    b(
        "project_search",
        "Find in Project",
        "Navigation",
        "Cmd+Shift+F",
    ),
    b(
        "command_palette",
        "Command Palette",
        "Navigation",
        "Cmd+Shift+P",
    ),
    b(
        "review_changes",
        "Review Changes",
        "Navigation",
        "Cmd+Shift+G",
    ),
    b("open_settings", "Settings…", "Navigation", "Cmd+,"),
    b("font_increase", "Increase Font Size", "Font", "Cmd+="),
    b("font_decrease", "Decrease Font Size", "Font", "Cmd+-"),
    b("font_reset", "Reset Font Size", "Font", "Cmd+0"),
    b("new_window", "New Window", "App", "Cmd+Shift+N"),
    b("fullscreen", "Toggle Full Screen", "App", "Ctrl+Cmd+F"),
];

/// Linux defaults that aren't the Mac's with Cmd as Ctrl.
const LINUX: &[(&str, &str)] = &[
    // The v0.29 Linux app's: the terminal keeps Ctrl+C, Ctrl+V and Ctrl+B.
    ("copy", "Ctrl+Shift+C"),
    ("paste", "Ctrl+Shift+V"),
    ("toggle_sidebar", "Ctrl+Shift+B"),
    ("fullscreen", "F11"),
    // Ctrl+D is end of input, Ctrl+I is Tab.
    ("split_right", "Ctrl+Shift+D"),
    ("split_down", "Ctrl+Alt+Shift+D"),
    ("agent_composer", "Ctrl+Alt+I"),
    // The Mac's Cmd+Alt and Ctrl+Cmd arrows would be Ctrl+Alt(+Shift)
    // arrows here, GNOME's for switching workspaces and moving windows
    // between them (and Super is the desktop's): no keys by default, the
    // Keyboard Shortcuts tab can give them some.
    ("resize_pane_left", ""),
    ("resize_pane_right", ""),
    ("resize_pane_up", ""),
    ("resize_pane_down", ""),
    ("focus_pane_left", ""),
    ("focus_pane_right", ""),
    ("focus_pane_up", ""),
    ("focus_pane_down", ""),
    // Ctrl+Shift+U is IBus's Unicode entry.
    ("next_agent", "Ctrl+Alt+U"),
    ("equalize_panes", "Ctrl+Alt+="),
    ("switch_workspace", "Ctrl+Alt+O"),
    // Ctrl+N is New File, Ctrl+Alt+N New Task.
    ("scratch_workspace", "Ctrl+Alt+Shift+N"),
    ("switch_branch", "Ctrl+Alt+B"),
    ("file_history", "Ctrl+Alt+Shift+H"),
    ("show_problems", "Ctrl+Alt+M"),
    ("project_actions", "Ctrl+Alt+R"),
    ("go_to_project_symbol", "Ctrl+Alt+Shift+O"),
    // The Mac's Ctrl+Shift+G is Review Changes' Ctrl+Shift+G here.
    ("show_changes", "Ctrl+Alt+Shift+G"),
];

/// The value of `keybinding_overrides` that removes a shortcut.
pub const UNBOUND: &str = "none";

/// Categories in the Keyboard Shortcuts tab's order.
pub fn categories() -> &'static [&'static str] {
    &[
        "Tabs",
        "Panes",
        "Blocks",
        "Terminal",
        "Editor",
        "Navigation",
        "Git",
        "Font",
        "App",
    ]
}

pub fn builtin(id: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.id == id)
}

/// A parsed shortcut: modifiers and one key, lowercased ("t", "left",
/// "return", "f5", "=").
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Shortcut {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
    pub key: String,
}

impl Shortcut {
    /// "Cmd+Shift+B", case-insensitive; Command, Control, Option and Opt
    /// work too, and "Cmd++" is the + key. `None` without a key.
    pub fn parse(text: &str) -> Option<Shortcut> {
        let text = text.trim();
        let (mods, key) = if text == "+" {
            ("", "+")
        } else if let Some(mods) = text.strip_suffix("++") {
            (mods, "+")
        } else {
            match text.rsplit_once('+') {
                Some((mods, key)) => (mods, key),
                None => ("", text),
            }
        };
        let key = key.trim();
        if key.is_empty() {
            return None;
        }
        let mut shortcut = Shortcut {
            ctrl: false,
            alt: false,
            shift: false,
            cmd: false,
            key: normalize_key(key),
        };
        for part in mods.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_lowercase().as_str() {
                "cmd" | "command" | "super" | "meta" => shortcut.cmd = true,
                "ctrl" | "control" => shortcut.ctrl = true,
                "shift" => shortcut.shift = true,
                "alt" | "option" | "opt" => shortcut.alt = true,
                _ => {}
            }
        }
        Some(shortcut)
    }

    /// The text form, in the Swift app's order: "Ctrl+Alt+Shift+Cmd+K".
    pub fn text(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".into());
        }
        if self.alt {
            parts.push("Alt".into());
        }
        if self.shift {
            parts.push("Shift".into());
        }
        if self.cmd {
            parts.push("Cmd".into());
        }
        parts.push(key_name(&self.key));
        parts.join("+")
    }

    /// The keystroke GPUI binds: "cmd-shift-t", "ctrl-alt-left".
    pub fn gpui(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if self.ctrl {
            parts.push("ctrl");
        }
        if self.alt {
            parts.push("alt");
        }
        if self.shift {
            parts.push("shift");
        }
        if self.cmd {
            parts.push(if cfg!(target_os = "macos") {
                "cmd"
            } else {
                "super"
            });
        }
        let key = match self.key.as_str() {
            "return" => "enter",
            "delete" => "backspace",
            "esc" => "escape",
            other => other,
        };
        parts.push(key);
        parts.join("-")
    }

    /// How menus and the palette show it: "⇧⌘P" on the Mac, "Ctrl+Shift+P"
    /// elsewhere.
    pub fn display(&self, platform: Platform) -> String {
        match platform {
            Platform::Mac => {
                let mut out = String::new();
                if self.ctrl {
                    out.push('⌃');
                }
                if self.alt {
                    out.push('⌥');
                }
                if self.shift {
                    out.push('⇧');
                }
                if self.cmd {
                    out.push('⌘');
                }
                out.push_str(&key_symbol(&self.key));
                out
            }
            Platform::Linux => {
                let mut parts: Vec<String> = Vec::new();
                if self.ctrl {
                    parts.push("Ctrl".into());
                }
                if self.alt {
                    parts.push("Alt".into());
                }
                if self.shift {
                    parts.push("Shift".into());
                }
                if self.cmd {
                    parts.push("Super".into());
                }
                parts.push(key_name(&self.key));
                parts.join("+")
            }
        }
    }
}

fn normalize_key(key: &str) -> String {
    match key.to_lowercase().as_str() {
        "enter" => "return".into(),
        "backspace" => "delete".into(),
        "esc" => "escape".into(),
        "comma" => ",".into(),
        "period" => ".".into(),
        "equal" | "plus" if key.len() > 1 => "=".into(),
        "minus" => "-".into(),
        other => other.into(),
    }
}

/// "Return", "Left", "F5", "K".
fn key_name(key: &str) -> String {
    match key {
        "return" => "Return".into(),
        "tab" => "Tab".into(),
        "space" => "Space".into(),
        "escape" => "Escape".into(),
        "delete" => "Delete".into(),
        "left" => "Left".into(),
        "right" => "Right".into(),
        "up" => "Up".into(),
        "down" => "Down".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "pageup" => "PageUp".into(),
        "pagedown" => "PageDown".into(),
        other => other.to_uppercase(),
    }
}

fn key_symbol(key: &str) -> String {
    match key {
        "left" => "←".into(),
        "right" => "→".into(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        "return" => "↩".into(),
        "tab" => "⇥".into(),
        "escape" => "⎋".into(),
        "delete" => "⌫".into(),
        "space" => "Space".into(),
        other => key_name(other),
    }
}

/// The default shortcut of `builtin` on `platform`, if it has one.
pub fn default_shortcut(builtin: &Builtin, platform: Platform) -> Option<Shortcut> {
    if builtin.mac.is_empty() {
        return None;
    }
    match platform {
        Platform::Mac => Shortcut::parse(builtin.mac),
        Platform::Linux => {
            if let Some((_, linux)) = LINUX.iter().find(|(id, _)| *id == builtin.id) {
                return Shortcut::parse(linux);
            }
            let mut shortcut = Shortcut::parse(builtin.mac)?;
            if shortcut.cmd {
                shortcut.cmd = false;
                shortcut.ctrl = true;
            }
            Some(shortcut)
        }
    }
}

/// The shortcut command `id` has: the user's override, else the default.
/// `None` when it has none or the override removes it.
pub fn effective(
    id: &str,
    overrides: &BTreeMap<String, String>,
    platform: Platform,
) -> Option<Shortcut> {
    match overrides.get(id) {
        Some(value) if value.eq_ignore_ascii_case(UNBOUND) => None,
        Some(value) if !value.trim().is_empty() => Shortcut::parse(value),
        _ => default_shortcut(builtin(id)?, platform),
    }
}

/// Every command with a shortcut, and that shortcut.
pub fn bindings(
    overrides: &BTreeMap<String, String>,
    platform: Platform,
) -> Vec<(&'static str, Shortcut)> {
    BUILTINS
        .iter()
        .filter_map(|b| effective(b.id, overrides, platform).map(|s| (b.id, s)))
        .collect()
}

/// Commands that share a shortcut, by its display form; only shortcuts
/// used more than once. `extra` adds custom shortcuts ("custom_<name>").
pub fn conflicts(
    overrides: &BTreeMap<String, String>,
    extra: &[(String, String)],
    platform: Platform,
) -> BTreeMap<String, Vec<String>> {
    let mut by: HashMap<Shortcut, Vec<String>> = HashMap::new();
    for (id, shortcut) in bindings(overrides, platform) {
        by.entry(shortcut).or_default().push(id.into());
    }
    for (id, text) in extra {
        if let Some(shortcut) = Shortcut::parse(text) {
            by.entry(shortcut).or_default().push(id.clone());
        }
    }
    by.into_iter()
        .filter(|(_, ids)| ids.len() > 1)
        .map(|(shortcut, ids)| (shortcut.display(platform), ids))
        .collect()
}

/// What a shortcut shows next to command `id` in menus and the palette.
pub fn display_for(
    id: &str,
    overrides: &BTreeMap<String, String>,
    platform: Platform,
) -> Option<String> {
    effective(id, overrides, platform).map(|s| s.display(platform))
}

/// On Linux, a shell needs its Ctrl+letter keys: in a terminal those go to
/// the shell, and a command bound to one is reached with Shift added, when
/// that's free (Ctrl+W closes the tab elsewhere, Ctrl+Shift+W in a
/// terminal). The (command, shortcut) pairs to bind in terminals.
///
/// Commands in `TERMINAL_FIRST` get their alias even when another command
/// has it outside terminals, where Linux terminals put them: Find is
/// Ctrl+Shift+F and New Tab Ctrl+Shift+T in a terminal, and Project Search
/// and Reopen Closed Tab everywhere else.
pub const TERMINAL_FIRST: &[&str] = &["find", "new_tab"];

pub fn terminal_aliases(bindings: &[(&'static str, Shortcut)]) -> Vec<(&'static str, Shortcut)> {
    let taken: std::collections::HashSet<&Shortcut> = bindings.iter().map(|(_, s)| s).collect();
    bindings
        .iter()
        .filter(|(_, s)| is_shell_chord(s))
        .filter_map(|(id, s)| {
            let alias = Shortcut {
                shift: true,
                ..s.clone()
            };
            (!taken.contains(&alias) || TERMINAL_FIRST.contains(id)).then_some((*id, alias))
        })
        .collect()
}

/// Ctrl+letter (or Ctrl+[ \ ] ^ _ @ space), with no other modifier: a
/// control character the shell reads.
pub fn is_shell_chord(shortcut: &Shortcut) -> bool {
    shortcut.ctrl
        && !shortcut.alt
        && !shortcut.shift
        && !shortcut.cmd
        && (shortcut.key.len() == 1
            && shortcut
                .key
                .chars()
                .all(|c| c.is_ascii_lowercase() || "[\\]^_@".contains(c))
            || shortcut.key == "space")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_takes_ctrl_shift_f_in_terminals() {
        let bindings = vec![
            ("find", Shortcut::parse("ctrl+f").unwrap()),
            ("project_search", Shortcut::parse("ctrl+shift+f").unwrap()),
            ("close_tab", Shortcut::parse("ctrl+w").unwrap()),
            ("reopen", Shortcut::parse("ctrl+shift+w").unwrap()),
            ("new_tab", Shortcut::parse("ctrl+t").unwrap()),
            ("reopen_tab", Shortcut::parse("ctrl+shift+t").unwrap()),
        ];
        let aliases = terminal_aliases(&bindings);
        assert_eq!(
            aliases,
            vec![
                ("find", Shortcut::parse("ctrl+shift+f").unwrap()),
                ("new_tab", Shortcut::parse("ctrl+shift+t").unwrap()),
            ]
        );
    }

    fn none() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    #[test]
    fn parses_and_prints() {
        let s = Shortcut::parse("cmd+shift+t").unwrap();
        assert_eq!(s.text(), "Shift+Cmd+T");
        assert_eq!(s.display(Platform::Mac), "⇧⌘T");
        assert_eq!(s.display(Platform::Linux), "Shift+Super+T");
        assert_eq!(Shortcut::parse("Cmd++").unwrap().key, "+");
        assert_eq!(Shortcut::parse("Ctrl+Cmd+=").unwrap().key, "=");
        assert_eq!(
            Shortcut::parse("Alt+Cmd+,").unwrap().display(Platform::Mac),
            "⌥⌘,"
        );
        assert_eq!(
            Shortcut::parse("Cmd+Shift+Return")
                .unwrap()
                .display(Platform::Mac),
            "⇧⌘↩"
        );
        let enter = Shortcut::parse("Ctrl+Enter").unwrap();
        assert_eq!(enter.key, "return");
        assert!(enter.gpui().ends_with("-enter"));
        assert_eq!(Shortcut::parse(""), None);
        assert_eq!(Shortcut::parse("Cmd+"), None);
    }

    #[test]
    fn defaults_per_platform() {
        let get = |id, p| effective(id, &none(), p).map(|s| s.display(Platform::Linux));
        assert_eq!(get("new_tab", Platform::Linux).as_deref(), Some("Ctrl+T"));
        assert_eq!(
            get("copy", Platform::Linux).as_deref(),
            Some("Ctrl+Shift+C")
        );
        assert_eq!(get("fullscreen", Platform::Linux).as_deref(), Some("F11"));
        assert_eq!(
            get("next_tab", Platform::Linux).as_deref(),
            Some("Ctrl+Tab")
        );
        assert_eq!(
            effective("command_palette", &none(), Platform::Mac)
                .unwrap()
                .display(Platform::Mac),
            "⇧⌘P"
        );
        assert_eq!(effective("git_fetch", &none(), Platform::Mac), None);
    }

    #[test]
    fn overrides() {
        let mut o = none();
        o.insert("split_right".into(), "Cmd+Alt+D".into());
        o.insert("new_tab".into(), "none".into());
        assert_eq!(
            display_for("split_right", &o, Platform::Mac).as_deref(),
            Some("⌥⌘D")
        );
        assert_eq!(display_for("new_tab", &o, Platform::Mac), None);
        o.insert("save".into(), "Cmd+P".into());
        let c = conflicts(&o, &[], Platform::Mac);
        assert_eq!(c.get("⌘P").map(Vec::len), Some(2));
    }

    #[test]
    fn no_default_conflicts() {
        for platform in [Platform::Mac, Platform::Linux] {
            let c = conflicts(&none(), &[], platform);
            assert!(c.is_empty(), "{platform:?}: {c:?}");
        }
    }

    #[test]
    fn linux_terminal_aliases() {
        let all = bindings(&none(), Platform::Linux);
        let aliases = terminal_aliases(&all);
        let alias = |id| {
            aliases
                .iter()
                .find(|(i, _)| *i == id)
                .map(|(_, s)| s.text())
        };
        assert_eq!(alias("close_tab").as_deref(), Some("Ctrl+Shift+W"));
        // In a terminal, as in other Linux terminals; it reopens a closed
        // tab elsewhere.
        assert_eq!(alias("new_tab").as_deref(), Some("Ctrl+Shift+T"));
        assert_eq!(alias("command_palette"), None, "not a shell chord");
        for builtin in BUILTINS {
            if let Some(s) = effective(builtin.id, &none(), Platform::Linux) {
                assert!(!s.cmd, "{} uses Super on Linux", builtin.id);
            }
        }
    }
}
