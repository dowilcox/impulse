//! The Quick Terminal (the Swift `QuickTerminal`): a terminal that drops
//! down from the top of the screen on a system-wide shortcut, over other
//! apps, and hides again on the same key or when you click away. Off until
//! enabled in Settings (`quick_terminal_enabled`, `quick_terminal_shortcut`);
//! Toggle Quick Terminal in the palette works either way.
//!
//! The shortcut is registered with the system through `global-hotkey`
//! (Carbon on macOS, XGrabKey on X11). Wayland has no way for an app to
//! take a system-wide key, so there the desktop's own shortcut settings
//! have to run it.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use gpui_kit::{
    div, prelude::*, px, size, AnyWindowHandle, App, Bounds, Context, Entity, Global, Subscription,
    Window, WindowBounds, WindowKind, WindowOptions,
};
use impulse_kit::chrome::ChromePalette;
use impulse_kit::keybindings::Shortcut;

use crate::settings_store::SettingsStore;
use crate::terminal_view::{TerminalView, TerminalViewEvent};

/// How much of the screen's height it takes.
const HEIGHT_FRACTION: f32 = 0.42;

/// How long focus may be away before the window tucks itself up.
const FOCUS_GRACE: Duration = Duration::from_millis(250);

#[derive(Default)]
pub struct QuickTerminal {
    terminal: Option<Entity<TerminalView>>,
    window: Option<AnyWindowHandle>,
    manager: Option<GlobalHotKeyManager>,
    hotkey: Option<HotKey>,
    /// What the hotkey was last set up for (enabled, shortcut).
    registered: Option<(bool, String)>,
    /// When the window last lost focus. Pressing a system-wide shortcut
    /// briefly takes the keyboard from it (X11's key grab), so a moment ago
    /// still counts as focused.
    deactivated_at: Option<Instant>,
}

impl Global for QuickTerminal {}

/// The `global-hotkey` form of a shortcut ("Ctrl+`" → "ctrl+`"); `None`
/// without Ctrl, Alt or Cmd (a bare key can't be system-wide), as Swift's
/// `carbonKey` refuses it.
pub fn hotkey_text(shortcut: &str) -> Option<String> {
    let parsed = Shortcut::parse(shortcut)?;
    if !(parsed.ctrl || parsed.alt || parsed.cmd) {
        return None;
    }
    let mut text = String::new();
    for (on, name) in [
        (parsed.shift, "shift+"),
        (parsed.ctrl, "control+"),
        (parsed.alt, "alt+"),
        (parsed.cmd, "super+"),
    ] {
        if on {
            text.push_str(name);
        }
    }
    text.push_str(&parsed.key);
    Some(text)
}

/// Set up the hotkey from Settings now and whenever they change.
pub fn init(cx: &mut App) {
    cx.set_global(QuickTerminal::default());
    let (sender, receiver) = async_channel::unbounded::<u32>();
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.state == HotKeyState::Pressed {
            let _ = sender.try_send(event.id);
        }
    }));
    cx.spawn(async move |cx| {
        while let Ok(id) = receiver.recv().await {
            cx.update(|cx| {
                let ours = cx.global::<QuickTerminal>().hotkey.map(|h| h.id());
                if ours == Some(id) {
                    toggle(cx);
                }
            });
        }
    })
    .detach();
    configure(cx);
    cx.observe_global::<SettingsStore>(configure).detach();
}

/// Register or drop the hotkey for the current settings.
fn configure(cx: &mut App) {
    let settings = SettingsStore::get(cx);
    let wanted = (
        settings.quick_terminal_enabled,
        settings.quick_terminal_shortcut.clone(),
    );
    let quick = cx.global_mut::<QuickTerminal>();
    if quick.registered.as_ref() == Some(&wanted) {
        return;
    }
    quick.registered = Some(wanted.clone());
    if let (Some(manager), Some(hotkey)) = (&quick.manager, quick.hotkey.take()) {
        let _ = manager.unregister(hotkey);
    }
    let (enabled, shortcut) = wanted;
    if !enabled {
        hide(cx);
        return;
    }
    let Some(hotkey) = hotkey_text(&shortcut).and_then(|t| t.parse::<HotKey>().ok()) else {
        log::warn!("Quick Terminal: can't use the shortcut {shortcut:?}");
        return;
    };
    let quick = cx.global_mut::<QuickTerminal>();
    if quick.manager.is_none() {
        match GlobalHotKeyManager::new() {
            Ok(manager) => quick.manager = Some(manager),
            Err(error) => {
                log::warn!("Quick Terminal: no system-wide shortcuts here ({error})");
                return;
            }
        }
    }
    let registered = quick.manager.as_ref().map(|m| m.register(hotkey));
    match registered {
        Some(Ok(())) => quick.hotkey = Some(hotkey),
        Some(Err(error)) => log::warn!("Quick Terminal: couldn't register {shortcut:?} ({error})"),
        None => {}
    }
}

/// The Quick Terminal's terminal, if it has this pane token (the `impulse`
/// CLI runs in it too).
pub fn terminal_with_token(token: &str, cx: &App) -> Option<Entity<TerminalView>> {
    let quick = cx.try_global::<QuickTerminal>()?;
    quick.terminal.clone().filter(|t| t.read(cx).token == token)
}

pub fn toggle(cx: &mut App) {
    let quick = cx.global::<QuickTerminal>();
    let (window, recently) = (
        quick.window,
        quick
            .deactivated_at
            .is_some_and(|t| t.elapsed() < FOCUS_GRACE * 2),
    );
    let active = window.is_some_and(|w| {
        w.update(cx, |_, window, _| window.is_window_active())
            .unwrap_or(false)
            || recently
    });
    if active {
        hide(cx);
    } else {
        show(cx);
    }
}

fn hide(cx: &mut App) {
    if let Some(window) = cx.global_mut::<QuickTerminal>().window.take() {
        let _ = window.update(cx, |_, window, _| window.remove_window());
    }
}

/// The front window's folder (the one its file tree shows), else home.
fn directory(cx: &App) -> PathBuf {
    crate::session::workbenches(cx)
        .first()
        .map(|(workbench, _)| workbench.read(cx).tree_root(cx))
        .filter(|d| d.is_dir())
        .unwrap_or_else(crate::workspace::home)
}

fn show(cx: &mut App) {
    if let Some(window) = cx.global::<QuickTerminal>().window {
        // Already up but behind something: bring it forward.
        if window
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
        cx.global_mut::<QuickTerminal>().window = None;
    }
    let Some(display) = cx.primary_display() else {
        return;
    };
    let screen = display.visible_bounds();
    let height = (screen.size.height * HEIGHT_FRACTION).round();
    let bounds = Bounds {
        origin: screen.origin,
        size: size(screen.size.width, height),
    };
    let existing = cx.global::<QuickTerminal>().terminal.clone();
    let opened = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            focus: true,
            show: true,
            display_id: Some(display.id()),
            app_id: Some("impulse-quick-terminal".into()),
            ..Default::default()
        },
        cx,
        move |window, cx| cx.new(|cx| QuickTerminalView::new(existing, window, cx)),
    );
    match opened {
        Ok((handle, view)) => {
            let terminal = view.read(cx).terminal.clone();
            let quick = cx.global_mut::<QuickTerminal>();
            quick.terminal = Some(terminal);
            quick.window = Some(handle);
        }
        Err(error) => log::error!("couldn't open the Quick Terminal: {error}"),
    }
}

pub struct QuickTerminalView {
    terminal: Entity<TerminalView>,
    _subscriptions: Vec<Subscription>,
}

impl QuickTerminalView {
    fn new(
        existing: Option<Entity<TerminalView>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = SettingsStore::get(cx).clone();
        let theme = impulse_kit::theme::get_theme(&settings.color_scheme);
        let palette = ChromePalette::new(&theme, crate::increase_contrast::get(cx));
        // The shell exited (exit, Ctrl+D): start a fresh one rather than
        // show a dead terminal.
        let terminal = match existing.filter(|t| t.read(cx).exited.is_none()) {
            Some(terminal) => {
                terminal.update(cx, |t, cx| t.set_theme(theme, palette, window, cx));
                terminal
            }
            None => {
                let dir = directory(cx).to_string_lossy().into_owned();
                cx.new(|cx| {
                    let mut terminal =
                        TerminalView::new(theme, palette, dir, Vec::new(), window, cx);
                    terminal.no_bar = true;
                    terminal
                })
            }
        };
        terminal.update(cx, |t, cx| t.focus(window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&terminal, window, |_, _, event, window, cx| match event {
                TerminalViewEvent::Exited => {
                    cx.global_mut::<QuickTerminal>().terminal = None;
                    cx.global_mut::<QuickTerminal>().window = None;
                    window.remove_window();
                }
                TerminalViewEvent::OpenFile { path, .. } => {
                    if let Some((workbench, handle)) =
                        crate::session::workbenches(cx).first().cloned()
                    {
                        let path = path.clone();
                        let _ = handle.update(cx, |_, window, cx| {
                            workbench.update(cx, |w, cx| w.open_file(&path, window, cx));
                            window.activate_window();
                        });
                    }
                }
                _ => {}
            }),
            // Click away and it tucks itself back up.
            cx.observe_window_activation(window, |_, window, cx| {
                if window.is_window_active() {
                    cx.global_mut::<QuickTerminal>().deactivated_at = None;
                    return;
                }
                cx.global_mut::<QuickTerminal>().deactivated_at = Some(Instant::now());
                cx.spawn_in(window, async move |_, cx| {
                    cx.background_executor().timer(FOCUS_GRACE).await;
                    let _ = cx.update(|window, cx| {
                        if !window.is_window_active() {
                            cx.global_mut::<QuickTerminal>().window = None;
                            window.remove_window();
                        }
                    });
                })
                .detach();
            }),
            cx.observe(&terminal, |_, _, cx| cx.notify()),
        ];
        QuickTerminalView {
            terminal,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for QuickTerminalView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().pt(px(4.)).child(self.terminal.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::hotkey_text;

    #[test]
    fn shortcuts_become_hotkeys() {
        assert_eq!(hotkey_text("Ctrl+`").as_deref(), Some("control+`"));
        assert_eq!(hotkey_text("Alt+Space").as_deref(), Some("alt+space"));
        assert_eq!(hotkey_text("Cmd+Shift+T").as_deref(), Some("shift+super+t"));
        assert_eq!(hotkey_text("F12"), None);
        assert_eq!(hotkey_text("Shift+A"), None);
        for text in ["control+`", "alt+space", "shift+super+t"] {
            assert!(
                text.parse::<global_hotkey::hotkey::HotKey>().is_ok(),
                "{text}"
            );
        }
    }
}
