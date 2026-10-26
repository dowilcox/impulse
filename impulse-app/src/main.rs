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
mod alert;
mod badge;
mod base_rules;
mod bell;
mod branch_manager;
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
mod display_options;
mod editor_actions;
mod editor_brackets;
mod editor_diff;
mod editor_git;
mod editor_keys;
mod editor_links;
mod editor_minimap;
mod editor_peek;
mod editor_tab;
mod editor_vim;
mod editor_watch;
mod file_icons;
mod file_tree_actions;
mod fonts;
mod git_actions;
mod history_actions;
mod history_view;
mod image_view;
mod instance;
mod keybindings_view;
mod kit_theme;
mod lsp_hub;
mod markdown_view;
mod menus;
mod overlays;
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
mod settings_json;
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
mod toasts;
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
            // The completion menu's kind icons (vendored GPUI Kit's), then
            // GPUI Kit's own (the editor's search panel, menus).
            Err(_) => match gpui_kit::component::input::CompletionIconAssets.load(path)? {
                Some(bytes) => Ok(Some(bytes)),
                None => gpui_kit::assets::Assets.load(path),
            },
        }
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        gpui_kit::assets::Assets.list(path)
    }
}

/// The same bundled fonts on every platform, so text looks the same
/// everywhere: Inter for the chrome (SF Pro can't ship on Linux) and
/// JetBrains Mono, the default terminal font. Each comes with its italic
/// faces: GPUI doesn't slant a face itself, so without them italic text
/// (preview tabs, a terminal's italic cells) would draw upright.
fn bundled_fonts() -> Vec<Cow<'static, [u8]>> {
    let fonts = &resources::get().fonts;
    [
        "inter/Inter-Regular.ttf",
        "inter/Inter-Italic.ttf",
        "inter/Inter-Medium.ttf",
        "inter/Inter-MediumItalic.ttf",
        "inter/Inter-SemiBold.ttf",
        "inter/Inter-SemiBoldItalic.ttf",
        "jetbrains-mono/JetBrainsMono-Regular.ttf",
        "jetbrains-mono/JetBrainsMono-Italic.ttf",
        "jetbrains-mono/JetBrainsMono-Bold.ttf",
        "jetbrains-mono/JetBrainsMono-BoldItalic.ttf",
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
/// What a launch asked for beyond restoring the session: a folder
/// (`--cwd`), commands (`--run`) and files (`--open`).
struct Launch {
    cwd: Option<String>,
    commands: Vec<String>,
    open: Vec<String>,
}

fn parse_args() -> (Args, Launch) {
    let mut args = Args {
        theme: None,
        // A launch from Finder, the Dock or a desktop entry starts in `/`,
        // which isn't a folder anyone means to work in: start at home then,
        // as the Swift app does.
        cwd: std::env::current_dir()
            .ok()
            .filter(|p| p.parent().is_some())
            .unwrap_or_else(crate::workspace::home)
            .to_string_lossy()
            .into_owned(),
        commands: Vec::new(),
        open: Vec::new(),
        trust: false,
        session: None,
        trust_store: std::sync::Arc::new(impulse_kit::trust::WorkspaceTrust::new(Some(
            impulse_kit::paths::trusted_folders_file(cfg!(debug_assertions)),
        ))),
    };
    let mut cwd_given = false;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        if flag == "--trust" {
            args.trust = true;
            continue;
        }
        // A path on its own (the desktop entry's %F, `impulse-app file`): a
        // folder to work in, or a file to open, made absolute so another
        // instance can take it.
        if !flag.starts_with("--") {
            let path = std::path::Path::new(&flag);
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir().unwrap_or_default().join(path)
            };
            if path.is_dir() {
                cwd_given = true;
                args.cwd = path.to_string_lossy().into_owned();
            } else {
                args.open.push(path.to_string_lossy().into_owned());
            }
            continue;
        }
        // The snapshot flags without a value (`debug_snapshot`).
        if flag == "--impulse-snapshot-no-lsp" || flag == "--impulse-snapshot-persist" {
            continue;
        }
        match (flag.as_str(), it.next()) {
            ("--theme", Some(v)) => args.theme = Some(v),
            ("--cwd", Some(v)) => {
                cwd_given = true;
                args.cwd = v
            }
            ("--run", Some(v)) => args.commands.push(v),
            ("--open", Some(v)) => args.open.push(v),
            _ => {}
        }
    }
    workbench_trust::share_trust(args.trust_store.clone());
    let launch = Launch {
        cwd: cwd_given.then(|| args.cwd.clone()),
        commands: args.commands.clone(),
        open: args.open.clone(),
    };
    (args, launch)
}

/// `path` from the launch's folder, when it's relative.
#[cfg(target_os = "linux")]
fn absolute(cwd: &str, path: &str) -> String {
    let path = std::path::Path::new(path);
    if path.is_absolute() {
        path.to_string_lossy().into_owned()
    } else {
        std::path::Path::new(cwd)
            .join(path)
            .to_string_lossy()
            .into_owned()
    }
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
    let (mut args, launch) = parse_args();
    #[cfg(debug_assertions)]
    if let Some(config) = debug_snapshot::config() {
        if let Some(cwd) = config.cwd.clone() {
            args.cwd = cwd;
        }
        // Folders aren't asked about in snapshots unless an action wants it
        // (`trust-on`).
        args.trust_store.set_enabled(false);
    }
    // An Impulse already running takes this launch (Linux; macOS runs one
    // app per bundle itself). Not for snapshots, which run on their own.
    #[cfg(target_os = "linux")]
    {
        #[cfg(debug_assertions)]
        let snapshot = debug_snapshot::is_active();
        #[cfg(not(debug_assertions))]
        let snapshot = false;
        if !snapshot {
            let request = instance::Request {
                cwd: launch.cwd.clone(),
                open: launch.open.iter().map(|p| absolute(&args.cwd, p)).collect(),
                commands: launch.commands.clone(),
            };
            if instance::forward(&request) {
                return;
            }
            instance::listen();
        }
    }
    let app = gpui_kit::application().with_assets(Assets);
    // Files the system asks to open (Finder, the Dock): in the front window.
    app.on_open_urls(|urls| {
        let open: Vec<String> = urls
            .iter()
            .filter_map(|url| url.strip_prefix("file://"))
            .map(|path| impulse_kit::uri::uri_to_file_path(&format!("file://{path}")))
            .collect();
        if !open.is_empty() {
            instance::send(instance::Request {
                open,
                ..Default::default()
            });
        }
    });
    app.run(move |cx: &mut App| {
        gpui_kit::init(cx);
        base_rules::register();
        settings_store::SettingsStore::init(cx);
        display_options::init(cx);
        workbench::bind_keys(cx);
        editor_keys::bind_keys(cx);
        editor_vim::bind_keys(cx);
        terminal_view::bind_keys(cx);
        palette::bind_keys(cx);
        command_editor::bind_keys(cx);
        cx.text_system()
            .add_fonts(bundled_fonts())
            .expect("bundled fonts");
        fonts::init(cx);
        let settings = settings_store::SettingsStore::get(cx).clone();
        commands::bind_keys(&settings, cx);
        commands::rebind_on_change(cx);
        menus::init(cx);
        session::init(cx);
        quick_terminal::init(cx);
        repository_state::start_auto_fetch(cx);
        update_check::start(cx);
        control_server::start(cx, control_commands::dispatch);
        instance::serve(cx);
        cx.on_app_quit(|cx| {
            control_server::stop();
            #[cfg(target_os = "linux")]
            instance::stop();
            shell_launch::remove_private_dir();
            workbench_trust::shutdown_servers(cx)
        })
        .detach();
        // The saved session comes back even when launched to open
        // something, which then opens on top of it (as the Swift app
        // does): otherwise the next quit would save over it.
        let saved = session::saved(cx);
        // A snapshot opens its own session file, or the default window.
        #[cfg(debug_assertions)]
        let saved = match debug_snapshot::is_active() {
            true => debug_snapshot::session(cx),
            false => saved,
        };
        let mut front = None;
        match saved {
            Some(state) => {
                let active = state.active_window_index.unwrap_or(0).max(0) as usize;
                for (index, window) in state.windows.into_iter().enumerate() {
                    let opened = open_main_window(
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
                    if index == active {
                        front = opened;
                    }
                }
                let target = front.or_else(|| session::workbenches(cx).first().map(|w| w.1));
                if let Some(workbench) = target.and_then(|handle| {
                    session::workbenches(cx)
                        .into_iter()
                        .find(|(_, h)| *h == handle)
                        .map(|(w, _)| w)
                }) {
                    let _ = target.map(|handle| {
                        handle.update(cx, |_, window, cx| {
                            workbench.update(cx, |workbench, cx| {
                                workbench.apply_launch(
                                    launch.cwd.clone(),
                                    launch.commands.clone(),
                                    launch.open.clone(),
                                    window,
                                    cx,
                                )
                            })
                        })
                    });
                }
            }
            None => {
                // No session: the files that were open last time
                // (`open_files`, as the Swift app reopens them).
                let reopen: Vec<String> = if launch.open.is_empty()
                    && settings_store::SettingsStore::get(cx).restore_session
                {
                    settings_store::SettingsStore::get(cx)
                        .open_files
                        .iter()
                        .filter(|p| std::path::Path::new(p).is_file())
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                };
                #[cfg(debug_assertions)]
                let reopen = if debug_snapshot::is_active() {
                    Vec::new()
                } else {
                    reopen
                };
                let mut args = args;
                args.open.extend(reopen);
                open_main_window(args, cx);
            }
        }
        #[cfg(debug_assertions)]
        if debug_snapshot::is_active() {
            // Never the active app: keys meant for another stay there.
            return debug_snapshot::run(cx);
        }
        cx.activate(true);
        // The window that was in front at quit comes to the front again.
        if let Some(front) = front {
            let _ = front.update(cx, |_, window, _| window.activate_window());
        }
    });
}

/// A main window, sized from the settings, or where a restored one was.
pub fn open_main_window(args: Args, cx: &mut App) -> Option<gpui_kit::AnyWindowHandle> {
    let settings = settings_store::SettingsStore::get(cx);
    let (width, height) = (settings.window_width as f32, settings.window_height as f32);
    let saved = args
        .session
        .as_ref()
        .and_then(|s| s.frame.as_deref())
        .and_then(|frame| session::saved_bounds(frame, cx));
    #[allow(unused_mut)]
    let mut bounds =
        saved.unwrap_or_else(|| Bounds::centered(None, size(px(width), px(height)), cx));
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
            // On macOS too the titlebar's empty stretches move the window
            // (window_frame.rs), so dragging or double-clicking a tab doesn't.
            app_owns_titlebar_drag: cfg!(target_os = "macos"),
            // The desktop entry's name (impulse.desktop): docks and window
            // switchers show its icon (Wayland's app_id, X11's WM_CLASS).
            app_id: Some("impulse".into()),
            ..Default::default()
        },
        cx,
        |window, cx| cx.new(|cx| workbench::Workbench::new(args, window, cx)),
    );
    match opened {
        Ok((handle, _)) => Some(handle),
        Err(error) => {
            log::error!("couldn't open a window: {error}");
            None
        }
    }
}
