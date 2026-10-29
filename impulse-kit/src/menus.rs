//! The menu bar (the Swift `MenuBuilder`), described once: the macOS menu
//! bar is built from it, and on Linux the same menus open from the
//! titlebar's menu button. Items run a palette command by id, so their
//! shortcuts are the commands' own, or one of the fixed items below.

use crate::keybindings::Platform;

/// Items that aren't palette commands: the standard ones every app has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fixed {
    About,
    Hide,
    HideOthers,
    ShowAll,
    Quit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    /// Paste as plain text (macOS only); every paste here is plain text.
    PasteAndMatchStyle,
    SelectAll,
    Open,
    CloseWindow,
    Minimize,
    Zoom,
    /// Window ▸ Bring All to Front (macOS only).
    BringAllToFront,
    /// The app menu's Services submenu, filled by macOS.
    Services,
    /// Tab 1 to 9 of the active workspace (9 is the last).
    Tab(u8),
    Help,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Command(&'static str, &'static str),
    Fixed(&'static str, Fixed),
    Submenu(&'static str, Vec<Item>),
    Separator,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Menu {
    pub title: &'static str,
    pub items: Vec<Item>,
}

/// The fixed items' shortcuts (`Keybindings`' text form), which can't be
/// changed. On Linux a terminal keeps its Ctrl+letter keys, so only keys
/// a shell doesn't read are taken there.
pub fn fixed_shortcut(fixed: Fixed, platform: Platform) -> Option<String> {
    let mac = platform == Platform::Mac;
    Some(match fixed {
        Fixed::Hide if mac => "Cmd+H".into(),
        Fixed::HideOthers if mac => "Alt+Cmd+H".into(),
        Fixed::Quit if mac => "Cmd+Q".into(),
        Fixed::Quit => "Ctrl+Shift+Q".into(),
        Fixed::Open if mac => "Cmd+O".into(),
        Fixed::CloseWindow if mac => "Shift+Cmd+W".into(),
        // Ctrl+Shift+W closes the tab from a terminal; the desktop's own.
        Fixed::CloseWindow => "Alt+F4".into(),
        Fixed::Minimize if mac => "Cmd+M".into(),
        Fixed::Tab(n) if mac => format!("Cmd+{n}"),
        Fixed::Tab(n) => format!("Alt+{n}"),
        Fixed::Help if mac => "Shift+Cmd+?".into(),
        Fixed::Undo => if mac { "Cmd+Z" } else { "Ctrl+Z" }.into(),
        Fixed::Redo => if mac { "Shift+Cmd+Z" } else { "Ctrl+Shift+Z" }.into(),
        Fixed::Cut => if mac { "Cmd+X" } else { "Ctrl+X" }.into(),
        Fixed::Copy => if mac { "Cmd+C" } else { "Ctrl+Shift+C" }.into(),
        Fixed::Paste => if mac { "Cmd+V" } else { "Ctrl+Shift+V" }.into(),
        Fixed::PasteAndMatchStyle if mac => "Alt+Shift+Cmd+V".into(),
        Fixed::SelectAll => if mac { "Cmd+A" } else { "Ctrl+A" }.into(),
        _ => return None,
    })
}

fn command(title: &'static str, id: &'static str) -> Item {
    Item::Command(title, id)
}

fn fixed(title: &'static str, fixed: Fixed) -> Item {
    Item::Fixed(title, fixed)
}

/// The menus, in order. On Linux the app menu's items move where Linux
/// apps keep them: Settings and Quit to File, About to Help.
pub fn main_menu(platform: Platform) -> Vec<Menu> {
    use Item::Separator;
    let mac = platform == Platform::Mac;
    let mut menus = Vec::new();
    if mac {
        menus.push(Menu {
            title: "Impulse",
            items: vec![
                fixed("About Impulse", Fixed::About),
                Separator,
                command("Settings…", "open_settings"),
                command("Keyboard Shortcuts…", "open_keybindings"),
                Separator,
                fixed("Services", Fixed::Services),
                Separator,
                fixed("Hide Impulse", Fixed::Hide),
                fixed("Hide Others", Fixed::HideOthers),
                fixed("Show All", Fixed::ShowAll),
                Separator,
                fixed("Quit Impulse", Fixed::Quit),
            ],
        });
    }
    let mut file = vec![
        command("New Tab", "new_tab"),
        command("New File", "new_file"),
        command("New Window", "new_window"),
        Separator,
        fixed("Open…", Fixed::Open),
        command("Open Folder as Workspace…", "open_workspace"),
        command("New Task…", "new_task"),
        command("New Task from Branch…", "new_task_from_branch"),
        command("Finish Task…", "finish_task"),
        command("Project Setup…", "project_setup"),
        command("Compose Message to Agent", "agent_composer"),
        command("Next Agent Needing You", "next_agent"),
        command("Review Last Agent Turn", "review_agent_turn"),
        command("Switch Workspace…", "switch_workspace"),
        command("Scratch Workspace", "scratch_workspace"),
        Separator,
        command("Close Tab", "close_tab"),
        command("Reopen Closed Tab", "reopen_tab"),
        fixed("Close Window", Fixed::CloseWindow),
        Separator,
        command("Save", "save"),
    ];
    if !mac {
        file.extend([
            Separator,
            command("Settings…", "open_settings"),
            command("Keyboard Shortcuts…", "open_keybindings"),
            Separator,
            fixed("Quit Impulse", Fixed::Quit),
        ]);
    }
    menus.push(Menu {
        title: "File",
        items: file,
    });
    let mut edit = vec![
        fixed("Undo", Fixed::Undo),
        fixed("Redo", Fixed::Redo),
        Separator,
        fixed("Cut", Fixed::Cut),
        fixed("Copy", Fixed::Copy),
        fixed("Paste", Fixed::Paste),
    ];
    if mac {
        edit.push(fixed("Paste and Match Style", Fixed::PasteAndMatchStyle));
    }
    edit.extend([
        fixed("Select All", Fixed::SelectAll),
        Separator,
        command("Find…", "find"),
        command("Go to Line…", "go_to_line"),
    ]);
    menus.push(Menu {
        title: "Edit",
        items: edit,
    });
    menus.push(Menu {
        title: "View",
        items: vec![
            command("Toggle Sidebar", "toggle_sidebar"),
            command("Show Files", "show_files"),
            Separator,
            command("Command Palette", "command_palette"),
            command("Go to File…", "quick_open"),
            command("Go to Symbol in File…", "go_to_symbol"),
            command("Find in Project", "project_search"),
            command("Go to Symbol in Project…", "go_to_project_symbol"),
            command("Show Problems", "show_problems"),
            command("Run Project Action…", "project_actions"),
            command("Toggle Markdown Preview", "toggle_markdown_preview"),
            Separator,
            Item::Submenu(
                "Panes",
                vec![
                    command("Split Right", "split_right"),
                    command("Split Down", "split_down"),
                    Separator,
                    command("Focus Pane Left", "focus_pane_left"),
                    command("Focus Pane Right", "focus_pane_right"),
                    command("Focus Pane Above", "focus_pane_up"),
                    command("Focus Pane Below", "focus_pane_down"),
                    command("Next Pane", "next_pane"),
                    command("Previous Pane", "prev_pane"),
                    Separator,
                    command("Grow Pane Left", "resize_pane_left"),
                    command("Grow Pane Right", "resize_pane_right"),
                    command("Grow Pane Up", "resize_pane_up"),
                    command("Grow Pane Down", "resize_pane_down"),
                    command("Even Out Panes", "equalize_panes"),
                    Separator,
                    command("Zoom Pane", "zoom_pane"),
                    command("Move Pane to New Tab", "move_pane_to_tab"),
                ],
            ),
            Item::Submenu(
                "Command Blocks",
                vec![
                    command("Show Hints", "terminal_hints"),
                    command("Select Blocks", "select_blocks"),
                    Separator,
                    command("Previous Block", "previous_block"),
                    command("Next Block", "next_block"),
                    command("Last Failed Block", "last_failed_block"),
                    Separator,
                    command("Bookmark Block", "toggle_block_bookmark"),
                    command("Previous Bookmark", "previous_block_bookmark"),
                    command("Next Bookmark", "next_block_bookmark"),
                ],
            ),
            Separator,
            command("Increase Font Size", "font_increase"),
            command("Decrease Font Size", "font_decrease"),
            command("Reset Font Size", "font_reset"),
            Separator,
            command("Toggle Full Screen", "fullscreen"),
        ],
    });
    menus.push(Menu {
        title: "Git",
        items: vec![
            command("Show Changes", "show_changes"),
            command("Review Changes", "review_changes"),
            command("Show Git History", "git_history"),
            command("Show History of This File", "file_history"),
            command("Toggle Diff View", "diff_view"),
            Separator,
            command("Switch Branch…", "switch_branch"),
            command("Manage Branches…", "manage_branches"),
            Separator,
            command("Fetch", "git_fetch"),
            command("Fetch All Remotes", "git_fetch_all"),
            command("Pull", "git_pull"),
            command("Pull (Rebase)", "git_pull_rebase"),
            command("Push", "git_push"),
            command("Force Push (With Lease)…", "git_force_push"),
            Separator,
            command("Create Tag…", "git_create_tag"),
            command("Push All Tags", "git_push_tags"),
            Separator,
            command("Stash All Changes", "git_stash"),
            command("Pop Latest Stash", "git_pop_stash"),
            command("Undo Last Commit", "git_undo_commit"),
            Separator,
            command("Open Repository in Browser", "git_open_remote"),
            command("Copy Remote URL", "git_copy_remote_url"),
        ],
    });
    let mut window = Vec::new();
    if mac {
        window.extend([
            fixed("Minimize", Fixed::Minimize),
            fixed("Zoom", Fixed::Zoom),
            Separator,
            fixed("Bring All to Front", Fixed::BringAllToFront),
            Separator,
        ]);
    }
    window.extend([
        command("Show Next Tab", "next_tab"),
        command("Show Previous Tab", "prev_tab"),
        Separator,
    ]);
    const TABS: [&str; 9] = [
        "Tab 1", "Tab 2", "Tab 3", "Tab 4", "Tab 5", "Tab 6", "Tab 7", "Tab 8", "Tab 9",
    ];
    window.extend((1..=9u8).map(|n| fixed(TABS[n as usize - 1], Fixed::Tab(n))));
    menus.push(Menu {
        title: "Window",
        items: window,
    });
    let mut help = vec![fixed("Impulse Help", Fixed::Help)];
    if !mac {
        help.extend([Separator, fixed("About Impulse", Fixed::About)]);
    }
    menus.push(Menu {
        title: "Help",
        items: help,
    });
    menus
}

/// Every command item's id, submenus included.
pub fn command_ids(items: &[Item], out: &mut Vec<&'static str>) {
    for item in items {
        match item {
            Item::Command(_, id) => out.push(id),
            Item::Submenu(_, items) => command_ids(items, out),
            _ => {}
        }
    }
}

/// Help ▸ Impulse Help: the user guide.
pub const HELP_URL: &str = "https://github.com/dowilcox/impulse/blob/main/docs/README.md";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_menus_on_both_but_the_app_menu() {
        let titles = |p| main_menu(p).iter().map(|m| m.title).collect::<Vec<_>>();
        assert_eq!(titles(Platform::Mac)[1..], titles(Platform::Linux)[..]);
        let mut mac = Vec::new();
        let mut linux = Vec::new();
        for menu in main_menu(Platform::Mac) {
            command_ids(&menu.items, &mut mac);
        }
        for menu in main_menu(Platform::Linux) {
            command_ids(&menu.items, &mut linux);
        }
        mac.sort();
        linux.sort();
        assert_eq!(mac, linux);
    }

    #[test]
    fn the_mac_only_items_stay_off_linux() {
        fn fixed_items(items: &[Item], out: &mut Vec<Fixed>) {
            for item in items {
                match item {
                    Item::Fixed(_, fixed) => out.push(*fixed),
                    Item::Submenu(_, items) => fixed_items(items, out),
                    _ => {}
                }
            }
        }
        let fixed = |p| {
            let mut out = Vec::new();
            for menu in main_menu(p) {
                fixed_items(&menu.items, &mut out);
            }
            out
        };
        let (mac, linux) = (fixed(Platform::Mac), fixed(Platform::Linux));
        for item in [
            Fixed::Services,
            Fixed::BringAllToFront,
            Fixed::PasteAndMatchStyle,
        ] {
            assert!(mac.contains(&item), "{item:?}");
            assert!(!linux.contains(&item), "{item:?}");
        }
        // Services sits between Keyboard Shortcuts and Hide, as in AppKit.
        let app = &main_menu(Platform::Mac)[0].items;
        let at = |wanted: &Item| app.iter().position(|i| i == wanted).unwrap();
        let services = at(&Item::Fixed("Services", Fixed::Services));
        assert!(at(&Item::Command("Keyboard Shortcuts…", "open_keybindings")) < services);
        assert!(services < at(&Item::Fixed("Hide Impulse", Fixed::Hide)));
    }

    #[test]
    fn linux_takes_no_shell_keys() {
        for fixed in [Fixed::Quit, Fixed::Open, Fixed::Tab(3), Fixed::CloseWindow] {
            if let Some(text) = fixed_shortcut(fixed, Platform::Linux) {
                let shortcut = crate::keybindings::Shortcut::parse(&text).unwrap();
                assert!(!crate::keybindings::is_shell_chord(&shortcut), "{text}");
            }
        }
    }
}
