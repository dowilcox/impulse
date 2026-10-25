//! Every user-facing command (the Swift `CommandRegistry`): the palette's
//! catalog, and one action, `RunCommand { id }`, that menus, the palette
//! and shortcuts all send to the workbench. Shortcuts come from
//! `impulse_kit::keybindings` with the user's `keybinding_overrides`.

use gpui_kit::{Action, App, KeyBinding, KeyBindingMetaIndex, Keystroke, NoAction};
use impulse_kit::keybindings::{self, Platform, Shortcut};
use impulse_kit::settings::Settings;

use crate::settings_store::SettingsStore;

/// Run the command with this id (a `keybindings` id, a palette id, or
/// `custom:…`).
#[derive(Clone, PartialEq, Debug, serde::Deserialize, Action)]
#[action(namespace = impulse, no_json)]
pub struct RunCommand {
    pub id: String,
}

impl RunCommand {
    pub fn new(id: impl Into<String>) -> Self {
        RunCommand { id: id.into() }
    }
}

/// A command as the palette lists it.
#[derive(Clone, Debug)]
pub struct AppCommand {
    pub id: String,
    pub title: String,
    pub category: String,
    pub keywords: Vec<String>,
    pub icon: Option<&'static str>,
    /// The shortcut shown beside it.
    pub shortcut: Option<String>,
}

/// Workbench commands beyond ImpulseKit's built-in palette items: (id,
/// title, category, keywords, icon).
const WORKBENCH: &[(&str, &str, &str, &[&str], &str)] = &[
    (
        "show_changes",
        "Show Changes",
        "Git",
        &["git", "stage", "commit", "status"],
        "git-branch",
    ),
    (
        "show_files",
        "Show Files",
        "Navigation",
        &["explorer", "file tree", "sidebar", "folders"],
        "folder-tree",
    ),
    (
        "switch_branch",
        "Switch Branch…",
        "Git",
        &["checkout", "git", "create branch"],
        "git-branch",
    ),
    (
        "agent_hooks",
        "Install Agent Hooks…",
        "Agents",
        &[
            "claude", "codex", "opencode", "hooks", "plugin", "status", "setup",
        ],
        "plug",
    ),
    (
        "agent_mcp_server",
        "Install Impulse's MCP Server…",
        "Agents",
        &[
            "mcp",
            "claude",
            "codex",
            "opencode",
            "tools",
            "tasks",
            "model context protocol",
        ],
        "plug",
    ),
    (
        "editor_integration",
        "Use Impulse as $EDITOR in Terminals",
        "Terminal",
        &["git commit", "visual", "editor", "wait"],
        "pencil",
    ),
    (
        "manage_branches",
        "Manage Branches…",
        "Git",
        &["delete", "rename", "publish", "merged", "stale"],
        "git-branch",
    ),
    (
        "git_fetch",
        "Fetch",
        "Git",
        &["git", "remote", "update", "sync"],
        "refresh-cw",
    ),
    (
        "git_fetch_all",
        "Fetch All Remotes",
        "Git",
        &["git", "remote", "update", "sync", "upstream"],
        "refresh-cw",
    ),
    (
        "git_pull",
        "Pull",
        "Git",
        &["git", "update", "sync", "download", "merge"],
        "arrow-down",
    ),
    (
        "git_pull_rebase",
        "Pull (Rebase)",
        "Git",
        &["git", "update", "sync", "rebase"],
        "arrow-down",
    ),
    (
        "git_push",
        "Push",
        "Git",
        &["git", "upload", "sync", "publish"],
        "arrow-up",
    ),
    (
        "git_force_push",
        "Force Push (With Lease)…",
        "Git",
        &["git", "overwrite", "rebase", "force-with-lease"],
        "arrow-up",
    ),
    (
        "git_create_tag",
        "Create Tag…",
        "Git",
        &["git", "release", "version", "annotated"],
        "tag",
    ),
    (
        "git_push_tags",
        "Push All Tags",
        "Git",
        &["git", "release", "version", "upload"],
        "tag",
    ),
    (
        "git_stash",
        "Stash All Changes",
        "Git",
        &["git", "save", "shelve", "wip"],
        "archive",
    ),
    (
        "git_pop_stash",
        "Pop Latest Stash",
        "Git",
        &["git", "restore", "unshelve", "apply"],
        "archive",
    ),
    (
        "git_undo_commit",
        "Undo Last Commit",
        "Git",
        &["git", "uncommit", "reset", "soft"],
        "undo-2",
    ),
    (
        "git_open_remote",
        "Open Repository in Browser",
        "Git",
        &["git", "web", "remote", "url"],
        "external-link",
    ),
    (
        "git_copy_remote_url",
        "Copy Remote URL",
        "Git",
        &["git", "remote", "origin", "clone", "url"],
        "copy",
    ),
    (
        "git_history",
        "Show Git History",
        "Git",
        &["log", "commits", "graph", "blame"],
        "history",
    ),
    (
        "file_history",
        "Show History of This File",
        "Git",
        &["log", "commits", "blame"],
        "history",
    ),
    (
        "new_task",
        "New Task…",
        "Workspaces",
        &["worktree", "branch", "agent", "parallel"],
        "git-branch-plus",
    ),
    (
        "project_setup",
        "Project Setup…",
        "Workspaces",
        &[
            "task",
            "worktree",
            "ports",
            "env",
            "clone",
            "setup script",
            "docker",
            "project.toml",
        ],
        "settings",
    ),
    (
        "new_task_from_branch",
        "New Task from Branch…",
        "Workspaces",
        &[
            "worktree", "branch", "checkout", "review", "existing", "remote",
        ],
        "git-branch",
    ),
    (
        "finish_task",
        "Finish Task…",
        "Workspaces",
        &[
            "worktree", "merge", "land", "push", "review", "done", "check",
        ],
        "git-merge",
    ),
    (
        "archive_task",
        "Archive Task…",
        "Workspaces",
        &["worktree", "remove", "done"],
        "archive",
    ),
    (
        "review_agent_turn",
        "Review Last Agent Turn",
        "Agents",
        &[
            "claude",
            "codex",
            "opencode",
            "diff",
            "checkpoint",
            "changes",
        ],
        "file-diff",
    ),
    (
        "send_selection_to_agent",
        "Send Selection to Agent",
        "Agents",
        &["claude", "codex", "opencode", "code", "ask"],
        "message-square-plus",
    ),
    (
        "agent_composer",
        "Compose Message to Agent",
        "Agents",
        &["prompt", "claude", "codex", "opencode", "write"],
        "message-square",
    ),
    (
        "next_agent",
        "Next Agent Needing You",
        "Agents",
        &["claude", "codex", "opencode", "inbox", "waiting"],
        "bot",
    ),
    (
        "command_history",
        "Command History…",
        "Terminal",
        &["history", "recent", "ctrl-r"],
        "history",
    ),
    (
        "preview_beside",
        "Open Preview to the Side",
        "Editor",
        &["markdown", "svg", "split", "live"],
        "columns-2",
    ),
    (
        "diff_view",
        "Toggle Diff View",
        "Editor",
        &["changes", "git", "compare", "staged", "side by side"],
        "file-diff",
    ),
    (
        "go_to_symbol",
        "Go to Symbol in File…",
        "Editor",
        &["outline", "function", "class", "@"],
        "code",
    ),
    (
        "go_to_project_symbol",
        "Go to Symbol in Project…",
        "Editor",
        &["workspace", "function", "class", "#"],
        "code",
    ),
    (
        "project_actions",
        "Run Project Action…",
        "Workspaces",
        &["task", "script", "run", "project.toml", "a:"],
        "play",
    ),
    (
        "trust_folder",
        "Trust This Folder…",
        "Workspaces",
        &[
            "workspace trust",
            "restricted",
            "security",
            "language servers",
        ],
        "shield-check",
    ),
    (
        "restrict_folder",
        "Restrict This Folder",
        "Workspaces",
        &["workspace trust", "untrust", "security", "language servers"],
        "shield-alert",
    ),
    (
        "forget_trusted_folders",
        "Forget Trusted Folders",
        "Workspaces",
        &["workspace trust", "restricted", "security", "reset"],
        "shield-alert",
    ),
    (
        "edit_project_config",
        "Edit Project Actions",
        "Workspaces",
        &["project.toml", "scripts", "worktree"],
        "pencil",
    ),
    (
        "show_problems",
        "Show Problems",
        "Editor",
        &["diagnostics", "errors", "warnings", "lint"],
        "triangle-alert",
    ),
    (
        "quick_terminal",
        "Toggle Quick Terminal",
        "Terminal",
        &["dropdown", "hotkey", "quake", "global"],
        "square-terminal",
    ),
    (
        "open_keybindings",
        "Keyboard Shortcuts…",
        "Impulse",
        &["keybindings", "shortcuts", "keys", "hotkeys"],
        "keyboard",
    ),
    (
        "open_settings_json",
        "Open settings.json",
        "Impulse",
        &["preferences", "config", "json"],
        "file-code",
    ),
    (
        "find_setting",
        "Find a Setting…",
        "Impulse",
        &["preferences", "search"],
        "settings",
    ),
    (
        "terminal_hints",
        "Show Hints",
        "Terminal",
        &["link", "url", "path", "sha", "port", "open", "copy"],
        "keyboard",
    ),
    (
        "import_shell_history",
        "Import Shell History",
        "Terminal",
        &["zsh", "bash", "fish", "history"],
        "history",
    ),
    (
        "clear_command_history",
        "Clear Command History",
        "Terminal",
        &["history", "forget", "delete", "privacy"],
        "trash-2",
    ),
    (
        "switch_workspace",
        "Switch Workspace…",
        "Workspaces",
        &["project", "folder", "recent"],
        "folder-git-2",
    ),
    (
        "open_workspace",
        "Open Folder as Workspace…",
        "Workspaces",
        &["project", "folder", "open"],
        "folder-open",
    ),
    (
        "clear_recent_workspaces",
        "Clear Recent Workspaces",
        "Workspaces",
        &["project", "folder", "recent", "history", "forget"],
        "trash-2",
    ),
    (
        "rename_workspace",
        "Rename Workspace…",
        "Workspaces",
        &["project", "name"],
        "pencil",
    ),
    (
        "close_workspace",
        "Close Workspace",
        "Workspaces",
        &["project", "folder"],
        "x",
    ),
    (
        "switch_tab",
        "Switch Tab…",
        "Tabs",
        &["go to tab"],
        "layers",
    ),
    (
        "search_text",
        "Search Text in Project…",
        "Navigation",
        &["grep", "find"],
        "search",
    ),
];

const PANES: &[(&str, &str, &[&str], &str)] = &[
    (
        "split_right",
        "Split Right",
        &["pane", "vertical split", "side by side"],
        "columns-2",
    ),
    (
        "split_down",
        "Split Down",
        &["pane", "horizontal split", "stack"],
        "rows-2",
    ),
    (
        "focus_pane_left",
        "Focus Pane Left",
        &["pane", "move"],
        "chevron-left",
    ),
    (
        "focus_pane_right",
        "Focus Pane Right",
        &["pane", "move"],
        "chevron-right",
    ),
    (
        "focus_pane_up",
        "Focus Pane Above",
        &["pane", "move"],
        "chevron-up",
    ),
    (
        "focus_pane_down",
        "Focus Pane Below",
        &["pane", "move"],
        "chevron-down",
    ),
    (
        "next_pane",
        "Next Pane",
        &["pane", "cycle"],
        "chevron-right",
    ),
    (
        "prev_pane",
        "Previous Pane",
        &["pane", "cycle"],
        "chevron-left",
    ),
    (
        "resize_pane_left",
        "Grow Pane Left",
        &["pane", "resize", "wider"],
        "chevron-left",
    ),
    (
        "resize_pane_right",
        "Grow Pane Right",
        &["pane", "resize", "wider"],
        "chevron-right",
    ),
    (
        "resize_pane_up",
        "Grow Pane Up",
        &["pane", "resize", "taller"],
        "chevron-up",
    ),
    (
        "resize_pane_down",
        "Grow Pane Down",
        &["pane", "resize", "taller"],
        "chevron-down",
    ),
    (
        "zoom_pane",
        "Zoom Pane",
        &["pane", "maximize", "fullscreen"],
        "maximize-2",
    ),
    (
        "equalize_panes",
        "Even Out Panes",
        &["pane", "equalize", "balance"],
        "columns-2",
    ),
    (
        "move_pane_to_tab",
        "Move Pane to New Tab",
        &["pane", "pop out", "detach"],
        "external-link",
    ),
];

const BLOCKS: &[(&str, &str, &[&str], &str)] = &[
    (
        "select_blocks",
        "Select Blocks",
        &["block", "copy", "output"],
        "square-terminal",
    ),
    (
        "previous_block",
        "Previous Block",
        &["block", "command", "scroll"],
        "chevron-up",
    ),
    (
        "next_block",
        "Next Block",
        &["block", "command", "scroll"],
        "chevron-down",
    ),
    (
        "last_failed_block",
        "Last Failed Block",
        &["block", "error", "exit"],
        "triangle-alert",
    ),
    (
        "toggle_block_bookmark",
        "Bookmark Block",
        &["block", "mark", "pin"],
        "bookmark",
    ),
    (
        "previous_block_bookmark",
        "Previous Bookmark",
        &["block", "mark"],
        "chevron-up",
    ),
    (
        "next_block_bookmark",
        "Next Bookmark",
        &["block", "mark"],
        "chevron-down",
    ),
];

fn builtin_icon(id: &str) -> Option<&'static str> {
    Some(match id {
        "new_tab" => "plus",
        "close_tab" => "x",
        "reopen_tab" => "undo-2",
        "next_tab" => "chevron-right",
        "prev_tab" => "chevron-left",
        "copy" => "copy",
        "paste" => "clipboard",
        "review_changes" => "file-diff",
        "new_file" => "file-plus",
        "save" => "check",
        "find" => "search",
        "go_to_line" => "corner-down-left",
        "toggle_markdown_preview" => "eye",
        "toggle_sidebar" => "panel-left",
        "quick_open" => "file",
        "project_search" => "search",
        "command_palette" => "command",
        "open_settings" => "settings",
        "font_increase" | "font_decrease" | "font_reset" => "sliders-horizontal",
        "new_window" => "external-link",
        "fullscreen" => "maximize-2",
        "install_lsp" => "package",
        _ => return None,
    })
}

/// The palette's commands for these settings.
pub fn catalog(settings: &Settings) -> Vec<AppCommand> {
    let platform = Platform::current();
    let overrides = &settings.keybinding_overrides;
    let shortcut = |id: &str| keybindings::display_for(id, overrides, platform);
    let mut out: Vec<AppCommand> = impulse_kit::command_palette::builtin_items()
        .into_iter()
        .map(|item| AppCommand {
            icon: builtin_icon(&item.id),
            shortcut: shortcut(&item.id),
            id: item.id,
            title: item.title,
            category: item.category,
            keywords: item.keywords,
        })
        .collect();
    let mut add = |id: &str, title: &str, category: &str, keywords: &[&str], icon: &'static str| {
        out.push(AppCommand {
            id: id.into(),
            title: title.into(),
            category: category.into(),
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
            icon: Some(icon),
            shortcut: shortcut(id),
        })
    };
    for (id, title, category, keywords, icon) in WORKBENCH {
        add(id, title, category, keywords, icon);
    }
    for (id, title, keywords, icon) in PANES {
        add(id, title, "Panes", keywords, icon);
    }
    for (id, title, keywords, icon) in BLOCKS {
        add(id, title, "Terminal", keywords, icon);
    }
    if cfg!(debug_assertions) {
        add(
            "component_gallery",
            "Component Gallery",
            "Developer",
            &["themes", "design", "components"],
            "palette",
        );
    }
    for custom in settings
        .custom_keybindings
        .iter()
        .filter(|c| !c.name.is_empty())
    {
        out.push(AppCommand {
            id: format!("custom_{}", custom.name),
            title: custom.name.clone(),
            category: "Custom".into(),
            keywords: vec![custom.command.clone()],
            icon: Some("square-terminal"),
            shortcut: keybindings::Shortcut::parse(&custom.key).map(|s| s.display(platform)),
        });
    }
    out
}

/// Marks the bindings `bind_keys` makes, so they can be swapped for new
/// ones when the shortcuts change.
const COMMAND_KEYS: KeyBindingMetaIndex = KeyBindingMetaIndex(0x4b45_5953);

/// Bind every command's shortcut.
pub fn bind_keys(settings: &Settings, cx: &mut App) {
    let others: Vec<KeyBinding> = cx.key_bindings().borrow().bindings().cloned().collect();
    cx.bind_keys(command_bindings(settings, &others));
}

/// The editor's context, for the keys it keeps.
const EDITOR_INPUT: &str = "ImpulseEditor > Input";

/// Keys the editor keeps while it has the keyboard, whatever command they
/// name, as the Swift app's `EditorWebView.appCommandWins` decides: the
/// terminal's commands (they'd do nothing there; ⇧⌘K deletes a line), and
/// Monaco's editing keys everyone knows from VS Code that Impulse's
/// commands also use (⌘D adds the next occurrence, ⌥⌘↑/↓ add a cursor,
/// ⇧⌘↩ inserts a line above).
fn editor_keeps(settings: &Settings, bound: &[(&str, keybindings::Shortcut)]) -> Vec<String> {
    let mut keys: Vec<String> = if cfg!(target_os = "macos") {
        vec!["cmd-d", "alt-cmd-up", "alt-cmd-down", "shift-cmd-enter"]
    } else {
        vec!["ctrl-d", "ctrl-shift-enter"]
    }
    .into_iter()
    .map(String::from)
    .collect();
    let terminal: std::collections::HashSet<String> = catalog(settings)
        .into_iter()
        .filter(|c| c.category == "Terminal" || c.category == "Blocks")
        .map(|c| c.id)
        .collect();
    for (id, shortcut) in bound {
        if terminal.contains(*id) {
            keys.push(shortcut.gpui());
        }
    }
    // As GPUI writes them, to compare with bindings.
    keys.iter()
        .filter_map(|k| Keystroke::parse(k).ok().map(|k| k.unparse()))
        .collect()
}

/// For the keys the editor keeps: the commands' bindings turned off in the
/// editor, then its own bindings for them (GPUI Kit's and Impulse's)
/// again, after the commands', so they win there (at the same depth the
/// later binding does).
fn editor_priority(keys: &[String], others: &[KeyBinding]) -> Vec<KeyBinding> {
    let mut out = Vec::new();
    for key in keys {
        out.extend(
            KeyBinding::load(
                key,
                Box::new(NoAction),
                gpui_kit::KeyBindingContextPredicate::parse(EDITOR_INPUT)
                    .ok()
                    .map(Into::into),
                false,
                None,
                &gpui_kit::DummyKeyboardMapper,
            )
            .ok(),
        );
        for binding in others.iter().filter(|b| b.meta() != Some(COMMAND_KEYS)) {
            let [stroke] = binding.keystrokes() else {
                continue;
            };
            if &stroke.inner().unparse() != key {
                continue;
            }
            let context = binding
                .predicate()
                .map(|p| p.to_string())
                .unwrap_or_default();
            let editor = context.contains("ImpulseEditor");
            if !editor && context != "Input" {
                continue;
            }
            // GPUI Kit's text-input keys, scoped to the editor here.
            let context = if editor {
                context
            } else {
                EDITOR_INPUT.to_string()
            };
            out.extend(
                KeyBinding::load(
                    key,
                    binding.action().boxed_clone(),
                    gpui_kit::KeyBindingContextPredicate::parse(&context)
                        .ok()
                        .map(Into::into),
                    false,
                    binding.action_input(),
                    &gpui_kit::DummyKeyboardMapper,
                )
                .ok(),
            );
        }
    }
    out
}

/// Bind the commands' shortcuts again whenever `keybinding_overrides` or
/// `custom_keybindings` change (the Keyboard Shortcuts tab, settings.json),
/// keeping every other binding, and the commands' place among them.
pub fn rebind_on_change(cx: &mut App) {
    let shortcuts = |s: &Settings| (s.keybinding_overrides.clone(), s.custom_keybindings.clone());
    let mut bound = shortcuts(SettingsStore::get(cx));
    cx.observe_global::<SettingsStore>(move |cx| {
        let settings = SettingsStore::get(cx).clone();
        let now = shortcuts(&settings);
        if now == bound {
            return;
        }
        bound = now;
        let keymap = cx.key_bindings();
        {
            let mut keymap = keymap.borrow_mut();
            let mut all: Vec<KeyBinding> = keymap.bindings().cloned().collect();
            let at = all
                .iter()
                .position(|b| b.meta() == Some(COMMAND_KEYS))
                .unwrap_or(all.len());
            all.retain(|b| b.meta() != Some(COMMAND_KEYS));
            let commands = command_bindings(&settings, &all);
            all.splice(at..at, commands);
            keymap.clear();
            keymap.add_bindings(all);
        }
        cx.refresh_windows();
    })
    .detach();
}

/// A binding of `shortcut`, unless GPUI can't read it (a hand-edited
/// settings.json).
fn binding(shortcut: &Shortcut, action: impl Action, context: Option<&str>) -> Option<KeyBinding> {
    let keys = shortcut.gpui();
    Keystroke::parse(&keys)
        .ok()
        .map(|_| KeyBinding::new(&keys, action, context).with_meta(COMMAND_KEYS))
}

/// Every command's shortcut. In a terminal on Linux, plain Ctrl+letter
/// chords go to the shell, and the commands bound to them are reached with
/// Shift added where that's free.
fn command_bindings(settings: &Settings, others: &[KeyBinding]) -> Vec<KeyBinding> {
    let platform = Platform::current();
    let bindings = keybindings::bindings(&settings.keybinding_overrides, platform);
    let mut keys: Vec<KeyBinding> = bindings
        .iter()
        .filter_map(|(id, shortcut)| binding(shortcut, RunCommand::new(*id), None))
        .collect();
    for custom in &settings.custom_keybindings {
        if let Some(shortcut) = keybindings::Shortcut::parse(&custom.key) {
            keys.extend(binding(
                &shortcut,
                RunCommand::new(format!("custom_{}", custom.name)),
                None,
            ));
        }
    }
    // GPUI ranks a binding without a context as matching at the deepest
    // level, so the input bar and find field (command editors inside the
    // terminal) need these bound in their own context too.
    if platform == Platform::Linux {
        for terminal in [
            crate::terminal_view::CONTEXT,
            crate::command_editor::CONTEXT,
        ] {
            for (_, shortcut) in bindings
                .iter()
                .filter(|(_, s)| keybindings::is_shell_chord(s))
            {
                keys.extend(binding(shortcut, NoAction, Some(terminal)));
            }
            for (id, alias) in keybindings::terminal_aliases(&bindings) {
                keys.extend(binding(&alias, RunCommand::new(id), Some(terminal)));
            }
        }
    }
    // Copy and Paste are the terminal's own actions, at the keys the
    // settings give them (in its input bar too); the editor keeps GPUI
    // Kit's.
    keys.retain(|k| {
        !matches!(
            k.action()
                .as_any()
                .downcast_ref::<RunCommand>()
                .map(|c| c.id.as_str()),
            Some("copy" | "paste")
        )
    });
    for (id, shortcut) in &bindings {
        let terminal = Some(crate::terminal_view::CONTEXT);
        let bar = Some(crate::command_editor::CONTEXT);
        match *id {
            "copy" => {
                keys.extend(binding(shortcut, crate::terminal_view::Copy, terminal));
                keys.extend(binding(shortcut, crate::command_editor::Copy, bar));
            }
            "paste" => {
                keys.extend(binding(shortcut, crate::terminal_view::Paste, terminal));
                keys.extend(binding(shortcut, crate::command_editor::Paste, bar));
            }
            _ => {}
        }
    }
    let kept = editor_keeps(settings, &bindings);
    keys.extend(
        editor_priority(&kept, others)
            .into_iter()
            .map(|b| b.with_meta(COMMAND_KEYS)),
    );
    // Vim's keys outrank the commands' while it reads them.
    keys.extend(
        crate::editor_vim::command_overrides()
            .into_iter()
            .map(|b| b.with_meta(COMMAND_KEYS)),
    );
    keys
}
