//! Impulse in Rust on GPUI: one codebase for macOS and Linux. This is the
//! migration spike (docs/plans/rust-migration.md): the workbench chrome
//! drawn from the theme the way `ChromePalette` draws it, with a live
//! terminal from `impulse-terminal`.
//!
//! `impulse-app [--theme <id>] [--cwd <dir>] [--run <command>]... [--open <file>]... [--trust]`
//! (`--install-lsp-servers` installs the managed language servers and
//! quits; debug builds take `debug_snapshot`'s `--impulse-snapshot` flags)

mod agent_checkpoints;
mod agent_composer;
mod agent_hooks;
mod agent_integration;
mod agent_notify;
mod agent_process;
mod agent_send;
mod agent_tasks;
mod agent_toolbelt;
mod agent_tracker;
mod agent_views;
mod bell;
mod branch_manager;
mod branch_switcher;
mod changes_panel;
mod close_flow;
mod command_editor;
mod command_history;
mod commands;
mod component_gallery;
mod control_commands;
mod control_review;
mod control_server;
#[cfg(debug_assertions)]
mod debug_snapshot;
mod dependency_prompts;
mod editor_diff;
mod editor_git;
mod editor_keys;
mod editor_minimap;
mod editor_peek;
mod editor_tab;
mod editor_vim;
mod editor_watch;
mod file_icons;
mod file_tree_actions;
mod git_actions;
mod history_actions;
mod history_view;
mod image_view;
mod keybindings_view;
mod kit_theme;
mod lsp_hub;
mod markdown_view;
mod menus;
mod palette;
mod port_scan;
mod problems_view;
mod project_actions;
mod project_setup;
mod project_setup_model;
mod quick_terminal;
mod repository_state;
mod resources;
mod review_view;
mod save_pipeline;
mod search_panel;
mod session;
mod settings_store;
mod settings_view;
mod shell_launch;
mod task_cleanup;
mod task_finish;
mod task_sheet;
mod task_sidebar;
mod tasks;
mod terminal_blocks;
mod terminal_find_bar;
mod terminal_glyphs;
mod terminal_input_bar;
mod terminal_view;
mod tree_watch;
mod untitled;
mod update_check;
mod window_frame;
mod workbench;
mod workbench_chrome;
mod workbench_panes;
mod workbench_review;
mod workbench_search;
mod workbench_trust;
mod workbench_workspaces;
mod workspace;
mod workspace_edits;

use std::borrow::Cow;

use gpui_kit::{
    px, size, App, AppContext, AssetSource, Bounds, SharedString, TitlebarOptions, WindowBounds,
    WindowOptions,
};

/// Icons from the vendored lucide set, by name (`icons/<name>.svg`), and
/// the material file icons (`material/<file>.svg`).
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        let resources = resources::get();
        let file = if let Some(name) = path.strip_prefix("material/") {
            resources.material.join(name)
        } else if let Some(name) = path.strip_prefix("icons/") {
            resources.lucide.join(name)
        } else {
            return gpui_kit::assets::Assets.load(path);
        };
        match std::fs::read(file) {
            Ok(bytes) => Ok(Some(Cow::Owned(bytes))),
            // GPUI Kit's own icons (the editor's search panel, menus).
            Err(_) => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        gpui_kit::assets::Assets.list(path)
    }
}

/// The same bundled fonts on every platform, so text looks the same
/// everywhere: Inter for the chrome (SF Pro can't ship on Linux) and
/// JetBrains Mono, the default terminal font.
fn bundled_fonts() -> Vec<Cow<'static, [u8]>> {
    let fonts = &resources::get().fonts;
    [
        "inter/Inter-Regular.ttf",
        "inter/Inter-Medium.ttf",
        "inter/Inter-SemiBold.ttf",
        "jetbrains-mono/JetBrainsMono-Regular.ttf",
        "jetbrains-mono/JetBrainsMono-Bold.ttf",
    ]
    .iter()
    .filter_map(|name| std::fs::read(fonts.join(name)).ok().map(Cow::Owned))
    .collect()
}

pub struct Args {
    /// A theme for this run, instead of `color_scheme`.
    pub theme: Option<String>,
    pub cwd: String,
    pub commands: Vec<String>,
    /// Files to open in editor tabs; the last one has the focus.
    pub open: Vec<String>,
    /// Trust the folder without asking (for scripted runs).
    pub trust: bool,
    /// The trusted folders: `trusted-folders.json` in the data directory
    /// (`impulse-dev` for debug builds, as for the Swift dev build).
    pub trust_store: std::sync::Arc<impulse_kit::trust::WorkspaceTrust>,
    /// A saved window to bring back instead of the default one.
    pub session: Option<impulse_kit::session::SessionWindow>,
}

/// The arguments, and whether any asked for a particular folder, command
/// or file (then the saved session isn't restored).
fn parse_args() -> (Args, bool) {
    let mut args = Args {
        theme: None,
        cwd: std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "/".into()),
        commands: Vec::new(),
        open: Vec::new(),
        trust: false,
        session: None,
        trust_store: std::sync::Arc::new(impulse_kit::trust::WorkspaceTrust::new(Some(
            impulse_kit::paths::trusted_folders_file(cfg!(debug_assertions)),
        ))),
    };
    let mut explicit = false;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        if flag == "--trust" {
            args.trust = true;
            continue;
        }
        // The snapshot flags without a value (`debug_snapshot`).
        if flag == "--impulse-snapshot-no-lsp" || flag == "--impulse-snapshot-persist" {
            continue;
        }
        match (flag.as_str(), it.next()) {
            ("--theme", Some(v)) => args.theme = Some(v),
            ("--cwd", Some(v)) => {
                explicit = true;
                args.cwd = v
            }
            ("--run", Some(v)) => {
                explicit = true;
                args.commands.push(v)
            }
            ("--open", Some(v)) => {
                explicit = true;
                args.open.push(v)
            }
            _ => {}
        }
    }
    (args, explicit)
}

fn main() {
    env_logger::init();
    // Install the managed language servers and quit (the smoke checks;
    // the Swift app's flag).
    if std::env::args().any(|a| a == "--install-lsp-servers") {
        match impulse_lsp::managed::install() {
            Ok(bin) => println!("Installed the language servers in {}", bin.display()),
            Err(error) => {
                eprintln!("Error: {error}");
                std::process::exit(1);
            }
        }
        return;
    }
    #[cfg(debug_assertions)]
    debug_snapshot::configure();
    #[allow(unused_mut)]
    let (mut args, explicit) = parse_args();
    #[cfg(debug_assertions)]
    if let Some(config) = debug_snapshot::config() {
        if let Some(cwd) = config.cwd.clone() {
            args.cwd = cwd;
        }
        // Folders aren't asked about in snapshots unless an action wants it
        // (`trust-on`).
        args.trust_store.set_enabled(false);
    }
    gpui_kit::application()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            settings_store::SettingsStore::init(cx);
            workbench::bind_keys(cx);
            editor_keys::bind_keys(cx);
            editor_vim::bind_keys(cx);
            terminal_view::bind_keys(cx);
            palette::bind_keys(cx);
            command_editor::bind_keys(cx);
            cx.text_system()
                .add_fonts(bundled_fonts())
                .expect("bundled fonts");
            let settings = settings_store::SettingsStore::get(cx).clone();
            commands::bind_keys(&settings, cx);
            commands::rebind_on_change(cx);
            menus::init(cx);
            session::init(cx);
            quick_terminal::init(cx);
            repository_state::start_auto_fetch(cx);
            update_check::start(cx);
            control_server::start(cx, control_commands::dispatch);
            cx.on_app_quit(|_| {
                control_server::stop();
                async {}
            })
            .detach();
            let saved = (!explicit).then(|| session::saved(cx)).flatten();
            // A snapshot opens its own session file, or the default window.
            #[cfg(debug_assertions)]
            let saved = match debug_snapshot::is_active() {
                true => debug_snapshot::session(cx),
                false => saved,
            };
            match saved {
                Some(state) => {
                    for window in state.windows {
                        open_main_window(
                            Args {
                                session: Some(window),
                                theme: args.theme.clone(),
                                cwd: args.cwd.clone(),
                                commands: Vec::new(),
                                open: Vec::new(),
                                trust: args.trust,
                                trust_store: args.trust_store.clone(),
                            },
                            cx,
                        );
                    }
                }
                None => open_main_window(args, cx),
            }
            #[cfg(debug_assertions)]
            if debug_snapshot::is_active() {
                // Never the active app: keys meant for another stay there.
                return debug_snapshot::run(cx);
            }
            cx.activate(true);
        });
}

/// A main window, sized from the settings.
pub fn open_main_window(args: Args, cx: &mut App) {
    let settings = settings_store::SettingsStore::get(cx);
    let (width, height) = (settings.window_width as f32, settings.window_height as f32);
    #[allow(unused_mut)]
    let mut bounds = Bounds::centered(None, size(px(width), px(height)), cx);
    // Snapshots are the same size run to run, at the screen's top left.
    #[cfg(debug_assertions)]
    if let Some(config) = debug_snapshot::config() {
        bounds = Bounds::new(
            gpui_kit::point(px(0.), px(0.)),
            size(px(config.size.0), px(config.size.1)),
        );
    }
    let opened = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Impulse".into()),
                appears_transparent: true,
                traffic_light_position: Some(gpui_kit::point(px(14.), px(13.))),
            }),
            // Impulse's own titlebar is the frame (window_frame.rs); a
            // window manager that won't allow it decorates the window.
            window_decorations: cfg!(target_os = "linux")
                .then_some(gpui_kit::WindowDecorations::Client),
            ..Default::default()
        },
        cx,
        |window, cx| cx.new(|cx| workbench::Workbench::new(args, window, cx)),
    );
    if let Err(error) = opened {
        log::error!("couldn't open a window: {error}");
    }
}
