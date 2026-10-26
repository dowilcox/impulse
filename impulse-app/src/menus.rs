//! The menu bar from `impulse_kit::menus`: the macOS menu bar (rebuilt when
//! shortcuts change, so it shows the current ones), and on Linux the same
//! menus from the titlebar's menu button. Command items run palette
//! commands; the fixed ones (Quit, Open…, Tab 1 to 9…) are actions here.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::{actions, prelude::*, App, Context, KeyBinding, Keystroke, Menu, MenuItem, Window};
use impulse_kit::keybindings::{Platform, Shortcut};
use impulse_kit::menus::{self, Fixed, Item};

use crate::commands::RunCommand;
use crate::settings_store::SettingsStore;
use crate::workbench::Workbench;

actions!(
    impulse_menu,
    [
        About,
        Hide,
        HideOthers,
        ShowAll,
        Quit,
        MenuCopy,
        MenuPaste,
        MenuPasteAndMatchStyle,
        MenuSelectAll,
        Open,
        CloseWindow,
        Minimize,
        Zoom,
        BringAllToFront,
        Help,
    ]
);

/// A key context no element has: bindings in it only label menu items.
const MENU_ONLY: &str = "ImpulseMenuOnly";

/// Tab 1 to 9 (Window menu).
#[derive(Clone, PartialEq, Debug, serde::Deserialize, gpui_kit::Action)]
#[action(namespace = impulse_menu, no_json)]
pub struct SelectTab {
    pub number: u8,
}

fn action(fixed: Fixed) -> Box<dyn gpui_kit::Action> {
    use gpui_kit::component::input::{Cut, Redo, Undo};
    match fixed {
        Fixed::About => Box::new(About),
        Fixed::Hide => Box::new(Hide),
        Fixed::HideOthers => Box::new(HideOthers),
        Fixed::ShowAll => Box::new(ShowAll),
        Fixed::Quit => Box::new(Quit),
        Fixed::Undo => Box::new(Undo),
        Fixed::Redo => Box::new(Redo),
        Fixed::Cut => Box::new(Cut),
        Fixed::Copy => Box::new(MenuCopy),
        Fixed::Paste => Box::new(MenuPaste),
        Fixed::PasteAndMatchStyle => Box::new(MenuPasteAndMatchStyle),
        Fixed::SelectAll => Box::new(MenuSelectAll),
        Fixed::Open => Box::new(Open),
        Fixed::CloseWindow => Box::new(CloseWindow),
        Fixed::Minimize => Box::new(Minimize),
        Fixed::Zoom => Box::new(Zoom),
        Fixed::BringAllToFront => Box::new(BringAllToFront),
        // A submenu macOS fills (`native_items`), never run.
        Fixed::Services => Box::new(gpui_kit::NoAction),
        Fixed::Tab(number) => Box::new(SelectTab { number }),
        Fixed::Help => Box::new(Help),
    }
}

/// The fixed items' keys, and the app-wide actions. The menu takes an
/// item's key equivalent from the keymap, but the editor and terminal bind
/// their own Copy, Paste and Select All: those are bound in a context no
/// element has, only to label the menu.
pub fn init(cx: &mut App) {
    let platform = Platform::current();
    let mut keys = Vec::new();
    let mut fixed = vec![
        Fixed::Quit,
        Fixed::Hide,
        Fixed::HideOthers,
        Fixed::Open,
        Fixed::CloseWindow,
        Fixed::Minimize,
        Fixed::Help,
    ];
    fixed.extend((1..=9).map(Fixed::Tab));
    let labels = [
        Fixed::Copy,
        Fixed::Paste,
        Fixed::PasteAndMatchStyle,
        Fixed::SelectAll,
    ];
    let items = fixed
        .into_iter()
        .map(|item| (item, None))
        .chain(labels.into_iter().map(|item| (item, Some(MENU_ONLY))));
    for (item, context) in items {
        let Some(shortcut) =
            menus::fixed_shortcut(item, platform).and_then(|t| Shortcut::parse(&t))
        else {
            continue;
        };
        let text = shortcut.gpui();
        if Keystroke::parse(&text).is_ok() {
            keys.push(KeyBinding::load(
                &text,
                action(item),
                context
                    .and_then(|c| gpui_kit::KeyBindingContextPredicate::parse(c).ok())
                    .map(Into::into),
                false,
                None,
                cx.keyboard_mapper().as_ref(),
            ));
        }
    }
    cx.bind_keys(keys.into_iter().flatten());
    cx.on_action(|_: &Quit, cx| crate::close_flow::quit(cx));
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Help, cx| cx.open_url(menus::HELP_URL));
    #[cfg(target_os = "macos")]
    {
        cx.on_action(|_: &About, _| mac::about_panel());
        cx.on_action(|_: &BringAllToFront, _| mac::arrange_in_front());
    }
    set_app_menus(cx);
    // Shortcuts changed: show the new ones (after the keymap is rebound).
    let mut overrides = SettingsStore::get(cx).keybinding_overrides.clone();
    cx.observe_global::<SettingsStore>(move |cx| {
        let now = SettingsStore::get(cx).keybinding_overrides.clone();
        if now != overrides {
            overrides = now;
            cx.defer(set_app_menus);
        }
    })
    .detach();
}

/// NSApplication's own About panel and Bring All to Front, as the Swift
/// menu's `orderFrontStandardAboutPanel:` and `arrangeInFront:`.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
mod mac {
    use objc::runtime::{Class, Object};
    use objc::{msg_send, sel, sel_impl};

    fn app() -> Option<*mut Object> {
        let class = Class::get("NSApplication")?;
        // SAFETY: AppKit's shared application lives for the whole process.
        let app: *mut Object = unsafe { msg_send![class, sharedApplication] };
        (!app.is_null()).then_some(app)
    }

    /// The name and version given, so they show without a bundle's
    /// Info.plist too (a `cargo run` build).
    pub fn about_panel() {
        let (Some(app), Some(string), Some(dictionary)) =
            (app(), Class::get("NSString"), Class::get("NSDictionary"))
        else {
            return;
        };
        let text = |s: &str| -> *mut Object {
            let s = std::ffi::CString::new(s).unwrap_or_default();
            // SAFETY: an autoreleased NSString copied from the C string.
            unsafe { msg_send![string, stringWithUTF8String: s.as_ptr()] }
        };
        let keys = [text("ApplicationName"), text("ApplicationVersion")];
        let values = [text("Impulse"), text(env!("CARGO_PKG_VERSION"))];
        // SAFETY: both arrays hold `count` live NSStrings, which the
        // dictionary retains; the panel copies what it shows.
        unsafe {
            let options: *mut Object = msg_send![dictionary,
                dictionaryWithObjects: values.as_ptr()
                forKeys: keys.as_ptr()
                count: keys.len()];
            let _: () = msg_send![app, orderFrontStandardAboutPanelWithOptions: options];
        }
    }

    pub fn arrange_in_front() {
        if let Some(app) = app() {
            // SAFETY: a standard NSApplication action; the sender may be nil.
            unsafe {
                let _: () = msg_send![app, arrangeInFront: std::ptr::null_mut::<Object>()];
            }
        }
    }
}

fn native_items(items: &[Item]) -> Vec<MenuItem> {
    items
        .iter()
        .map(|item| match item {
            Item::Command(title, id) => MenuItem::action(*title, RunCommand::new(*id)),
            Item::Fixed(title, Fixed::Services) => {
                MenuItem::os_submenu(*title, gpui_kit::SystemMenuType::Services)
            }
            Item::Fixed(title, fixed) => MenuItem::Action {
                name: (*title).into(),
                action: action(*fixed),
                os_action: None,
                checked: false,
                disabled: false,
            },
            Item::Submenu(title, items) => MenuItem::submenu(Menu {
                name: (*title).into(),
                items: native_items(items),
                disabled: false,
            }),
            Item::Separator => MenuItem::separator(),
        })
        .collect()
}

/// The macOS menu bar (Linux has none: the titlebar button shows it).
fn set_app_menus(cx: &mut App) {
    if Platform::current() != Platform::Mac {
        return;
    }
    let menus: Vec<Menu> = menus::main_menu(Platform::Mac)
        .iter()
        .map(|menu| Menu {
            name: menu.title.into(),
            items: native_items(&menu.items),
            disabled: false,
        })
        .collect();
    cx.set_menus(menus);
}

/// A popup menu's items, submenus opening beside it.
fn popup_items(
    mut menu: PopupMenu,
    items: &[Item],
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    for item in items {
        menu = match item {
            Item::Command(title, id) => menu.menu(*title, Box::new(RunCommand::new(*id))),
            // macOS's alone.
            Item::Fixed(_, Fixed::Services) => continue,
            Item::Fixed(title, fixed) => menu.menu(*title, action(*fixed)),
            Item::Submenu(title, items) => {
                let items = items.clone();
                menu.submenu(*title, window, cx, move |menu, window, cx| {
                    popup_items(menu, &items, window, cx)
                })
            }
            Item::Separator => menu.separator(),
        };
    }
    menu
}

/// Linux's menu button, at the start of the titlebar.
pub fn menu_button(cx: &mut Context<Workbench>) -> Option<impl IntoElement> {
    if Platform::current() == Platform::Mac {
        return None;
    }
    let _ = cx;
    Some(
        Button::new("app-menu")
            .ghost()
            .small()
            .icon(Icon::empty().path("icons/menu.svg"))
            .dropdown_menu(|mut menu: PopupMenu, window, cx| {
                for top in menus::main_menu(Platform::Linux) {
                    let items = top.items.clone();
                    menu = menu.submenu(top.title, window, cx, move |menu, window, cx| {
                        popup_items(menu, &items, window, cx)
                    });
                }
                menu
            }),
    )
}

impl Workbench {
    /// The menus' window actions, on the workbench's root.
    pub(crate) fn menu_actions(
        &self,
        root: gpui_kit::Stateful<gpui_kit::Div>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        root.on_action(cx.listener(|this, _: &Open, window, cx| this.open_files_dialog(window, cx)))
            .on_action(cx.listener(|w, _: &CloseWindow, window, cx| w.close_window(window, cx)))
            .on_action(cx.listener(|_, _: &Minimize, window, _| window.minimize_window()))
            .on_action(cx.listener(|_, _: &Zoom, window, _| window.zoom_window()))
            // macOS shows its own About panel (`menus::init`).
            .when(!cfg!(target_os = "macos"), |root| {
                root.on_action(
                    cx.listener(|this, _: &About, window, cx| this.show_about(window, cx)),
                )
            })
            .on_action(cx.listener(|this, tab: &SelectTab, window, cx| {
                let count = this.workspace().tabs.len();
                if count == 0 {
                    return;
                }
                // 9 is the last tab, as in browsers.
                let index = if tab.number == 9 {
                    count - 1
                } else {
                    tab.number as usize - 1
                };
                if index < count {
                    this.activate_tab(index, window, cx);
                }
            }))
            .on_action(cx.listener(|_, _: &MenuCopy, window, cx| {
                window.dispatch_action(Box::new(gpui_kit::component::input::Copy), cx);
                window.dispatch_action(Box::new(crate::terminal_view::Copy), cx);
            }))
            .on_action(cx.listener(|_, _: &MenuPaste, window, cx| {
                window.dispatch_action(Box::new(gpui_kit::component::input::Paste), cx);
                window.dispatch_action(Box::new(crate::terminal_view::Paste), cx);
            }))
            // Every paste here is plain text already.
            .on_action(cx.listener(|_, _: &MenuPasteAndMatchStyle, window, cx| {
                window.dispatch_action(Box::new(gpui_kit::component::input::Paste), cx);
                window.dispatch_action(Box::new(crate::terminal_view::Paste), cx);
            }))
            .on_action(cx.listener(|_, _: &MenuSelectAll, window, cx| {
                window.dispatch_action(Box::new(gpui_kit::component::input::SelectAll), cx);
                window.dispatch_action(Box::new(crate::terminal_view::SelectAll), cx);
            }))
    }

    /// File ▸ Open…: files to open in editor tabs, or a folder to open as
    /// a workspace (Swift's Open… takes either).
    fn open_files_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                for path in paths {
                    if path.is_dir() {
                        this.open_workspace(&path, window, cx);
                    } else {
                        this.open_file(&path, window, cx);
                    }
                }
            });
        })
        .detach();
    }

    /// About Impulse on Linux: the name and version.
    fn show_about(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let version = env!("CARGO_PKG_VERSION");
        crate::overlays::open_dialog(window, cx, move |dialog, _, _| {
            dialog.title("Impulse").w(gpui_kit::px(340.)).child(
                gpui_kit::div()
                    .flex()
                    .flex_col()
                    .gap(gpui_kit::px(6.))
                    .child(format!("Version {version}"))
                    .child("A terminal IDE for macOS and Linux."),
            )
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_menu_command_is_in_the_catalog() {
        let catalog: Vec<String> = crate::commands::catalog(&Default::default())
            .into_iter()
            .map(|c| c.id.to_string())
            .collect();
        for platform in [Platform::Mac, Platform::Linux] {
            let mut ids = Vec::new();
            for menu in menus::main_menu(platform) {
                menus::command_ids(&menu.items, &mut ids);
            }
            for id in ids {
                assert!(catalog.iter().any(|c| c == id), "{id} isn't a command");
            }
        }
    }
}
