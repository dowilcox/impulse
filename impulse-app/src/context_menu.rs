//! Right-click menus and the dropdowns buttons open, described once
//! (`MenuSpec`) and shown the platform's way: on macOS an `NSMenu` through
//! GPUI Kit's `NativeMenu`, as the Swift app's `.contextMenu` and
//! `ChromeMenuButton` (`NSMenu.popUp` below the button); elsewhere GPUI
//! Kit's drawn `PopupMenu`.
//!
//! `NativeMenu` items carry actions, not closures, so each item is a
//! `MenuPick` naming its place in the menu shown last; the handlers wait
//! here until one is picked.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui_kit::component::button::Button;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, Context, InteractiveElement, ParentElement, Pixels, Point,
    SharedString, Styled, Window,
};

/// What a menu item does when chosen.
pub(crate) type Run = Rc<dyn Fn(&mut Window, &mut App)>;

/// One entry of a menu.
#[derive(Clone)]
pub(crate) enum MenuEntry {
    Item {
        label: SharedString,
        disabled: bool,
        checked: bool,
        run: Run,
    },
    Separator,
    Submenu {
        label: SharedString,
        items: MenuSpec,
    },
}

/// A menu's items, in order. Separators are dropped at the start, the end
/// and next to another, as `PopupMenu` drops them.
#[derive(Clone, Default)]
pub(crate) struct MenuSpec {
    entries: Vec<MenuEntry>,
}

impl MenuSpec {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// An item that runs `run` when chosen.
    pub(crate) fn item(
        self,
        label: impl Into<SharedString>,
        run: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.entry(label, false, false, Rc::new(run))
    }

    /// An item, greyed out unless `enabled`.
    pub(crate) fn item_enabled(
        self,
        label: impl Into<SharedString>,
        enabled: bool,
        run: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.entry(label, !enabled, false, Rc::new(run))
    }

    /// An item with a check mark when `checked`.
    #[allow(dead_code)]
    pub(crate) fn item_checked(
        self,
        label: impl Into<SharedString>,
        checked: bool,
        run: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.entry(label, false, checked, Rc::new(run))
    }

    /// Any item: `disabled` greys it out, `checked` marks it.
    pub(crate) fn entry(
        mut self,
        label: impl Into<SharedString>,
        disabled: bool,
        checked: bool,
        run: Run,
    ) -> Self {
        self.entries.push(MenuEntry::Item {
            label: label.into(),
            disabled,
            checked,
            run,
        });
        self
    }

    pub(crate) fn separator(mut self) -> Self {
        if !matches!(self.entries.last(), None | Some(MenuEntry::Separator)) {
            self.entries.push(MenuEntry::Separator);
        }
        self
    }

    /// A submenu; an empty one is left out.
    pub(crate) fn submenu(mut self, label: impl Into<SharedString>, items: MenuSpec) -> Self {
        if !items.is_empty() {
            self.entries.push(MenuEntry::Submenu {
                label: label.into(),
                items,
            });
        }
        self
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries().is_empty()
    }

    /// The entries, without a trailing separator.
    pub(crate) fn entries(&self) -> &[MenuEntry] {
        match self.entries.last() {
            Some(MenuEntry::Separator) => &self.entries[..self.entries.len() - 1],
            _ => &self.entries,
        }
    }

    /// The menu as GPUI Kit's drawn `PopupMenu`.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub(crate) fn into_popup(
        self,
        mut menu: PopupMenu,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        for entry in self.entries() {
            menu = match entry.clone() {
                MenuEntry::Separator => menu.separator(),
                MenuEntry::Item {
                    label,
                    disabled,
                    checked,
                    run,
                } => menu.item(popup_item(label, disabled, checked, run)),
                MenuEntry::Submenu { label, items } => {
                    menu.submenu(label, window, cx, move |menu, window, cx| {
                        items.clone().into_popup(menu, window, cx)
                    })
                }
            };
        }
        menu
    }

    /// Show the menu at `position` (in the window) with `NativeMenu`: an
    /// `NSMenu` on macOS. The chosen item runs once the menu has closed.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn show_native(self, position: Point<Pixels>, window: &mut Window, cx: &mut App) {
        if self.is_empty() {
            return;
        }
        if !LISTENING.with(|l| l.replace(true)) {
            cx.on_action(picked);
        }
        let generation = GENERATION.with(|g| {
            g.set(g.get().wrapping_add(1));
            g.get()
        });
        let mut runs = Vec::new();
        let menu = native(self.entries(), generation, &mut runs);
        PENDING.with(|p| {
            *p.borrow_mut() = Some(Pending {
                generation,
                window: window.window_handle(),
                runs,
            })
        });
        menu.show(position, window, cx);
    }
}

fn popup_item(label: SharedString, disabled: bool, checked: bool, run: Run) -> PopupMenuItem {
    PopupMenuItem::new(label)
        .disabled(disabled)
        .checked(checked)
        .on_click(move |_, window, cx| run(window, cx))
}

/// A `NativeMenu` of `entries`, each item's handler added to `runs` at
/// the index its `MenuPick` names.
fn native(entries: &[MenuEntry], generation: u64, runs: &mut Vec<Run>) -> NativeMenu {
    let mut menu = NativeMenu::new();
    for entry in entries {
        menu = match entry {
            MenuEntry::Separator => menu.separator(),
            MenuEntry::Item {
                label,
                disabled,
                checked,
                run,
            } => {
                let pick = Box::new(MenuPick {
                    generation,
                    index: runs.len(),
                });
                runs.push(run.clone());
                if *checked && !*disabled {
                    menu.menu_with_check(label.clone(), true, pick)
                } else {
                    menu.menu_with_disabled(label.clone(), *disabled, pick)
                }
            }
            MenuEntry::Submenu { label, items } => {
                menu.submenu(label.clone(), native(items.entries(), generation, runs))
            }
        };
    }
    menu
}

/// A native menu's item was chosen: the `index`th handler of the menu
/// shown as `generation`.
#[derive(Clone, PartialEq, Debug, serde::Deserialize, gpui_kit::Action)]
#[action(namespace = impulse_context_menu, no_json)]
pub(crate) struct MenuPick {
    generation: u64,
    index: usize,
}

/// The handlers of the native menu shown last.
struct Pending {
    generation: u64,
    window: AnyWindowHandle,
    runs: Vec<Run>,
}

thread_local! {
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
    static GENERATION: Cell<u64> = const { Cell::new(0) };
    static LISTENING: Cell<bool> = const { Cell::new(false) };
}

/// `NativeMenu` dispatches the chosen item's action to the window it was
/// shown in, where nothing handles a `MenuPick`, so it reaches this
/// app-wide listener. The window is mid-dispatch then, so the handler runs
/// right after, in it.
fn picked(pick: &MenuPick, cx: &mut App) {
    let chosen = PENDING.with(|p| {
        let mut p = p.borrow_mut();
        if p.as_ref()?.generation != pick.generation {
            return None;
        }
        let pending = p.take()?;
        let run = pending.runs.get(pick.index)?.clone();
        Some((pending.window, run))
    });
    if let Some((window, run)) = chosen {
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| run(window, cx));
        });
    }
}

/// On macOS the element itself, which opens the menu on a right-click or
/// a Ctrl-click; elsewhere GPUI Kit's `ContextMenu` around it.
#[cfg(target_os = "macos")]
pub(crate) type WithMenu<E> = E;
#[cfg(not(target_os = "macos"))]
pub(crate) type WithMenu<E> = gpui_kit::component::menu::ContextMenu<E>;

/// A right-click menu on an element, built when it opens.
pub(crate) trait RightClickMenu:
    InteractiveElement + ParentElement + Styled + Sized
{
    /// On macOS a Ctrl-click opens it too, as AppKit does, without also
    /// clicking the element (see `ctrl_click_shield`). Call it after the
    /// element's last `child`.
    #[track_caller]
    fn right_click_menu(
        self,
        build: impl Fn(&mut Window, &mut App) -> MenuSpec + 'static,
    ) -> WithMenu<Self> {
        #[cfg(target_os = "macos")]
        {
            let build = Rc::new(build);
            let on_ctrl = build.clone();
            self.on_mouse_down(gpui_kit::MouseButton::Right, move |event, window, cx| {
                cx.stop_propagation();
                build(window, cx).show_native(event.position, window, cx);
            })
            .capture_any_mouse_down(move |event, _, _| {
                if is_ctrl_click(event) {
                    let (build, at) = (on_ctrl.clone(), event.position);
                    set_ctrl_click(Rc::new(move |window, cx| {
                        build(window, cx).show_native(at, window, cx)
                    }));
                }
            })
            .child(ctrl_click_shield())
        }
        #[cfg(not(target_os = "macos"))]
        {
            use gpui_kit::component::menu::ContextMenuExt as _;
            self.context_menu(move |menu, window, cx| {
                let spec = build(window, cx);
                spec.into_popup(menu, window, cx)
            })
        }
    }
}

impl<E: InteractiveElement + ParentElement + Styled> RightClickMenu for E {}

// MARK: Ctrl-click

// GPUI has no Ctrl-click as a right-click, and a mouse-down handler on the
// element itself comes too late: GPUI runs an element's bubble-phase
// handlers after its children's and after noting the press for its own
// `on_click` (and drag), so the row would also be clicked, or start a drag
// once the menu closes, its mouse-up eaten by the menu. So it takes two
// steps. In the capture phase, which runs outer elements first, each
// element with a menu under the mouse notes how to open it, the innermost
// last. Then a transparent child laid over the element, painted after
// everything else in it, takes the press in the bubble phase before the
// element or any ancestor notes it: it opens the innermost menu and stops
// the event there. Elements without a menu (the terminal, the editor)
// never get the shield, so a Ctrl-click there means what it always did.

#[cfg(target_os = "macos")]
thread_local! {
    /// The innermost menu under a Ctrl-click, noted in the capture phase.
    static CTRL_CLICK: RefCell<Option<Run>> = const { RefCell::new(None) };
}

/// A left press with Ctrl held, which macOS takes as a right-click.
#[cfg(target_os = "macos")]
fn is_ctrl_click(event: &gpui_kit::MouseDownEvent) -> bool {
    event.button == gpui_kit::MouseButton::Left && event.modifiers.control
}

#[cfg(target_os = "macos")]
fn set_ctrl_click(open: Run) {
    CTRL_CLICK.with(|c| *c.borrow_mut() = Some(open));
}

/// The transparent child over an element with a menu that opens the noted
/// menu on a Ctrl-click (see above). It doesn't block the mouse, so
/// whatever is under it still hovers and clicks.
#[cfg(target_os = "macos")]
fn ctrl_click_shield() -> gpui_kit::Div {
    gpui_kit::div().absolute().inset_0().on_mouse_down(
        gpui_kit::MouseButton::Left,
        |event, window, cx| {
            if !is_ctrl_click(event) {
                return;
            }
            if let Some(open) = CTRL_CLICK.with(|c| c.borrow_mut().take()) {
                cx.stop_propagation();
                open(window, cx);
            }
        },
    )
}

// MARK: Dropdowns

/// How far below its button a dropdown opens, as Swift's `MenuAnchor`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const DROPDOWN_GAP: f32 = 4.;

/// A menu a button opens, built when it opens: on macOS an `NSMenu` shown
/// on mouse-down below the button's bottom-left corner, as Swift's
/// `ChromeMenuButton`; elsewhere GPUI Kit's drawn dropdown.
pub(crate) trait DropdownMenuButton: Sized {
    fn dropdown(self, build: impl Fn(&mut Window, &mut App) -> MenuSpec + 'static) -> AnyElement;
}

impl DropdownMenuButton for Button {
    /// On macOS the button's own `on_click` opens the menu from the
    /// keyboard (Space or Enter), so don't give it another.
    fn dropdown(self, build: impl Fn(&mut Window, &mut App) -> MenuSpec + 'static) -> AnyElement {
        use gpui_kit::IntoElement as _;
        #[cfg(target_os = "macos")]
        {
            use gpui_kit::{point, px, Bounds};
            // The button's bounds, from the last frame it was painted in.
            let bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
            let open: Run = {
                let (bounds, build) = (bounds.clone(), Rc::new(build));
                Rc::new(move |window, cx| {
                    let Some(b) = bounds.get() else {
                        return;
                    };
                    let at = point(b.left(), b.bottom() + px(DROPDOWN_GAP));
                    build(window, cx).show_native(at, window, cx);
                })
            };
            let (on_key, on_ctrl) = (open.clone(), open.clone());
            gpui_kit::div()
                .child(self.on_click(move |event, window, cx| {
                    if event.is_keyboard() {
                        on_key(window, cx);
                    }
                }))
                // A Ctrl-click opens the dropdown, not the menu of the row
                // it's in (whose shield takes the press first).
                .capture_any_mouse_down(move |event, _, _| {
                    if is_ctrl_click(event) {
                        set_ctrl_click(on_ctrl.clone());
                    }
                })
                // After the button's own handlers (which keep it from taking
                // focus), before any ancestor notes a click.
                .on_mouse_down(gpui_kit::MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    CTRL_CLICK.with(|c| c.borrow_mut().take());
                    open(window, cx);
                })
                .on_children_prepainted(move |children, _, _| bounds.set(children.first().copied()))
                .into_any_element()
        }
        #[cfg(not(target_os = "macos"))]
        {
            use gpui_kit::component::menu::DropdownMenu as _;
            self.dropdown_menu(move |menu, window, cx| {
                let spec = build(window, cx);
                spec.into_popup(menu, window, cx)
            })
            .into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(spec: &MenuSpec) -> Vec<String> {
        spec.entries()
            .iter()
            .map(|e| match e {
                MenuEntry::Item { label, .. } => label.to_string(),
                MenuEntry::Separator => "-".into(),
                MenuEntry::Submenu { label, items } => {
                    format!("{label}[{}]", labels(items).join(","))
                }
            })
            .collect()
    }

    #[test]
    fn separators_collapse_as_popup_menu_does() {
        let spec = MenuSpec::new()
            .separator()
            .item("A", |_, _| {})
            .separator()
            .separator()
            .item_enabled("B", false, |_, _| {})
            .submenu("Empty", MenuSpec::new())
            .submenu("Sub", MenuSpec::new().separator().item("C", |_, _| {}))
            .separator();
        assert_eq!(labels(&spec), ["A", "-", "B", "Sub[C]"]);
        assert!(MenuSpec::new().separator().is_empty());
    }

    #[test]
    fn native_items_pick_their_handlers_in_order() {
        let spec = MenuSpec::new()
            .item("A", |_, _| {})
            .submenu("Sub", MenuSpec::new().item("B", |_, _| {}))
            .item_enabled("C", false, |_, _| {});
        let mut runs = Vec::new();
        let menu = native(spec.entries(), 7, &mut runs);
        assert!(!menu.is_empty());
        assert_eq!(runs.len(), 3);
    }
}
