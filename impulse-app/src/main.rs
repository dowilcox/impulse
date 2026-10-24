//! Impulse in Rust on GPUI: one codebase for macOS and Linux. This is the
//! migration spike (docs/plans/rust-migration.md): the workbench chrome
//! drawn from the theme the way `ChromePalette` draws it, with a live
//! terminal from `impulse-terminal`.
//!
//! `impulse-app [--theme <id>] [--cwd <dir>] [--run <command>]... [--open <file>]... [--trust]`

mod editor_tab;
mod kit_theme;
mod lsp_hub;
mod terminal_view;
mod workbench;

use std::borrow::Cow;
use std::path::PathBuf;

use gpui_kit::{
    px, size, App, AppContext, AssetSource, Bounds, SharedString, TitlebarOptions, WindowBounds,
    WindowOptions,
};

/// Icons from the vendored lucide set, by name (`icons/<name>.svg`).
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        let file = repo_root().join("vendor/lucide").join(path);
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

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// The same bundled fonts on every platform, so text looks the same
/// everywhere: Inter for the chrome (SF Pro can't ship on Linux) and
/// JetBrains Mono, the default terminal font.
fn bundled_fonts() -> Vec<Cow<'static, [u8]>> {
    let fonts = repo_root().join("vendor/fonts");
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
    pub theme: String,
    pub cwd: String,
    pub commands: Vec<String>,
    /// Files to open in editor tabs; the last one has the focus.
    pub open: Vec<String>,
    /// Trust the folder without asking (for scripted runs).
    pub trust: bool,
    /// The trusted folders: `trusted-folders.json` in the data directory
    /// (`impulse-dev` for debug builds, as for the Swift dev build).
    pub trust_store: std::sync::Arc<impulse_kit::trust::WorkspaceTrust>,
}

fn parse_args() -> Args {
    let mut args = Args {
        theme: "nord".into(),
        cwd: std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "/".into()),
        commands: Vec::new(),
        open: Vec::new(),
        trust: false,
        trust_store: std::sync::Arc::new(impulse_kit::trust::WorkspaceTrust::new(Some(
            impulse_kit::paths::trusted_folders_file(cfg!(debug_assertions)),
        ))),
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        if flag == "--trust" {
            args.trust = true;
            continue;
        }
        match (flag.as_str(), it.next()) {
            ("--theme", Some(v)) => args.theme = v,
            ("--cwd", Some(v)) => args.cwd = v,
            ("--run", Some(v)) => args.commands.push(v),
            ("--open", Some(v)) => args.open.push(v),
            _ => {}
        }
    }
    args
}

fn main() {
    env_logger::init();
    let args = parse_args();
    gpui_kit::application()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            workbench::bind_keys(cx);
            cx.text_system()
                .add_fonts(bundled_fonts())
                .expect("bundled fonts");
            let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Impulse".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(gpui_kit::point(px(14.), px(13.))),
                    }),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| workbench::Workbench::new(args, window, cx)),
            )
            .unwrap();
            cx.activate(true);
        });
}
