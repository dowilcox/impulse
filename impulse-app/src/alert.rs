//! Alerts drawn as macOS 26's `NSAlert` sheet, which the Swift app uses for
//! its questions: 260 wide, a bold title over the message, an optional
//! accessory (a checkbox, fields) centered under it, and pill buttons, the
//! first being the default (Return) one, in a 228-wide block: stacked, or
//! two side by side (the default on the right) when both titles fit half of
//! it. A wide accessory widens the alert. Escape picks the button titled
//! "Cancel", and ⌘D the one titled "Don't Save", as in `NSAlert`.
//!
//! The colors are the theme's (its overlay surface, accent and danger), so
//! an alert sits with the panels and popovers around it.

use std::rc::Rc;

use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::{
    actions, div, prelude::*, px, AnyElement, App, Entity, Hsla, KeyBinding, MouseButton,
    SharedString, Window,
};

use crate::workbench_chrome::hsla;
use impulse_kit::chrome::ChromePalette;

actions!(alert, [DontSave]);

/// The key context of an alert that has a "Don't Save" button.
const CONTEXT: &str = "Alert";

pub(crate) type AlertAction = Rc<dyn Fn(&mut Window, &mut App)>;
pub(crate) type Accessory = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// The width of an alert, and of its text column.
const WIDTH: f32 = 260.;
const INSET: f32 = 22.;
const WIDE_INSET: f32 = 16.;

pub(crate) struct Alert {
    pub title: SharedString,
    pub message: Option<SharedString>,
    pub accessory: Option<Accessory>,
    /// The accessory's width, when it's wider than the text column: the
    /// alert grows around it and lays its buttons out in a row.
    pub accessory_width: Option<f32>,
    /// In `NSAlert`'s order: the first is the default.
    pub buttons: Vec<(SharedString, AlertAction)>,
    /// The first button does something that can't be undone (red title).
    pub destructive: bool,
    /// The button Escape picks: the one titled "Cancel" unless set.
    pub escape: Option<usize>,
}

/// The chrome colors alerts are drawn in (`kit_theme::apply` sets them).
pub(crate) fn palette(cx: &App) -> ChromePalette {
    match cx.try_global::<crate::find_bar::ChromeTheme>() {
        Some(chrome) => chrome.palette.clone(),
        None => ChromePalette::new(&impulse_kit::theme::get_theme("nord"), false),
    }
}

impl Alert {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Alert {
            title: title.into(),
            message: None,
            accessory: None,
            accessory_width: None,
            buttons: Vec::new(),
            destructive: false,
            escape: None,
        }
    }

    /// Escape picks button `index`.
    pub fn escape(mut self, index: usize) -> Self {
        self.escape = Some(index);
        self
    }

    /// The first button's action is destructive (`hasDestructiveAction`).
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    pub fn message(mut self, message: impl Into<SharedString>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn accessory(
        mut self,
        width: Option<f32>,
        build: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        self.accessory = Some(Rc::new(build));
        self.accessory_width = width;
        self
    }

    pub fn button(
        mut self,
        title: impl Into<SharedString>,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.buttons.push((title.into(), Rc::new(action)));
        self
    }

    /// Show it over `window`'s content. A button's action closes it
    /// (`window.close_dialog`), as every answer to an `NSAlert` does.
    pub fn show(self, window: &mut Window, cx: &mut App) {
        let colors = Colors::new(&palette(cx));
        // A wide accessory sets the width, 16 in from each side.
        let width = self
            .accessory_width
            .filter(|w| *w > WIDTH - 2. * INSET)
            .map_or(WIDTH, |w| w + 2. * WIDE_INSET);
        let alert = Rc::new(self);
        let default = alert.buttons.first().map(|(_, action)| action.clone());
        let cancel = alert
            .escape
            .or_else(|| {
                alert
                    .buttons
                    .iter()
                    .position(|(title, _)| title.as_ref() == "Cancel")
            })
            .and_then(|index| alert.buttons.get(index))
            .map(|(_, action)| action.clone());
        let dont_save = alert
            .buttons
            .iter()
            .find(|(title, _)| title.as_ref() == "Don't Save")
            .map(|(_, action)| action.clone());
        // ⌘D reaches the alert while its body has the focus (inside the
        // dialog's, so Return and Escape still reach the dialog).
        let focus = cx.focus_handle();
        let first = std::cell::Cell::new(true);
        if dont_save.is_some() && cfg!(target_os = "macos") {
            static BOUND: std::sync::Once = std::sync::Once::new();
            BOUND.call_once(|| cx.bind_keys([KeyBinding::new("cmd-d", DontSave, Some(CONTEXT))]));
        }
        crate::overlays::open_dialog(window, cx, move |dialog, window, cx| {
            let (default, cancel) = (default.clone(), cancel.clone());
            let body = body(&alert, colors, width, window, cx);
            let body = match dont_save.clone() {
                Some(action) => {
                    if first.replace(false) {
                        window.focus(&focus, cx);
                    }
                    div()
                        .track_focus(&focus)
                        .key_context(CONTEXT)
                        .on_action(move |_: &DontSave, window, cx| action(window, cx))
                        .child(body)
                        .into_any_element()
                }
                None => body,
            };
            dialog
                .w(px(width))
                .p_0()
                .bg(colors.background)
                .border_color(colors.border)
                .rounded(px(14.))
                .close_button(false)
                .overlay_closable(false)
                .keyboard(true)
                .on_ok(move |_, window, cx| {
                    if let Some(action) = default.clone() {
                        action(window, cx);
                    }
                    false
                })
                .on_cancel(move |_, window, cx| {
                    if let Some(action) = cancel.clone() {
                        action(window, cx);
                    }
                    false
                })
                .child(body)
        });
    }
}

#[derive(Clone, Copy)]
struct Colors {
    background: Hsla,
    border: Hsla,
    text: Hsla,
    button: Hsla,
    button_hover: Hsla,
    button_pressed: Hsla,
    /// A plain button's title (`labelColor`).
    button_text: Hsla,
    default: Hsla,
    default_hover: Hsla,
    default_text: Hsla,
    /// A destructive button's fill and title: tinted with the theme's red.
    danger_fill: Hsla,
    danger_fill_hover: Hsla,
    danger_text: Hsla,
}

impl Colors {
    /// The popovers' surface (`kit_theme`'s `popover`), raised buttons and
    /// the accent's default button, as the rest of the chrome has them.
    fn new(p: &ChromePalette) -> Self {
        let surface = p.overlay.mix(p.text, 0.02);
        Colors {
            background: hsla(surface),
            border: hsla(p.hairline_strong),
            text: hsla(p.text),
            button: hsla(p.raised),
            button_hover: hsla(p.raised.mix(p.text, 0.06)),
            button_pressed: hsla(p.raised.mix(p.text, 0.1)),
            button_text: hsla(p.text),
            default: hsla(p.accent),
            default_hover: hsla(p.accent.mix(p.text, 0.12)),
            default_text: hsla(p.text_on_accent),
            danger_fill: hsla(surface.mix(p.danger, 0.22)),
            danger_fill_hover: hsla(surface.mix(p.danger, 0.3)),
            danger_text: hsla(p.danger),
        }
    }
}

/// The width of the button block: 16 in from a 260-wide alert's sides, and
/// centered in a wider one.
const BUTTONS: f32 = 228.;
const SIDE_BY_SIDE_GAP: f32 = 12.;

/// How wide `text` is in an alert's 13-point UI font.
pub(crate) fn text_width(
    text: &SharedString,
    weight: gpui_kit::FontWeight,
    window: &mut Window,
) -> f32 {
    let run = gpui_kit::TextRun {
        len: text.len(),
        font: gpui_kit::Font {
            weight,
            ..gpui_kit::font(crate::workbench::UI_FONT)
        },
        color: gpui_kit::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    f32::from(
        window
            .text_system()
            .shape_line(text.clone(), px(13.), &[run], None)
            .width,
    )
}

fn body(
    alert: &Alert,
    colors: Colors,
    width: f32,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let mut measure =
        |text: &SharedString, weight: gpui_kit::FontWeight| text_width(text, weight, window);
    let text = |content: SharedString| {
        div()
            .text_size(px(13.))
            .line_height(px(16.))
            .text_color(colors.text)
            .child(content)
    };
    // Two buttons whose titles fit half the block go side by side.
    let half = (BUTTONS - SIDE_BY_SIDE_GAP) / 2.;
    let side_by_side = alert.buttons.len() == 2
        && alert
            .buttons
            .iter()
            .all(|(title, _)| measure(title, gpui_kit::FontWeight::NORMAL) + 28. <= half);
    let buttons: Vec<AnyElement> = alert
        .buttons
        .iter()
        .enumerate()
        .map(|(index, (title, action))| {
            let action = action.clone();
            let destructive = index == 0 && alert.destructive;
            let default = index == 0 && !destructive;
            let (fill, hover) = if default {
                (colors.default, colors.default_hover)
            } else if destructive {
                (colors.danger_fill, colors.danger_fill_hover)
            } else {
                (colors.button, colors.button_hover)
            };
            div()
                .id(("alert-button", index))
                .h(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .px(px(14.))
                .rounded_full()
                .bg(fill)
                .hover(move |s| s.bg(hover))
                .when(!default && !destructive, |d| {
                    d.active(move |s| s.bg(colors.button_pressed))
                })
                .text_size(px(13.))
                .text_color(if default {
                    colors.default_text
                } else if destructive {
                    colors.danger_text
                } else {
                    colors.button_text
                })
                .when(side_by_side, |d| d.flex_1())
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .on_click(move |_, window, cx| action(window, cx))
                .child(title.clone())
                .into_any_element()
        })
        .collect();
    // The text keeps its inset; a wide accessory comes 16 from the sides.
    let inset = INSET;
    let accessory_inset = if width > WIDTH { WIDE_INSET } else { INSET };
    // NSAlert leaves 9 points under a one-line title, 16 under a wrapped one.
    let title_gap = if measure(&alert.title, gpui_kit::FontWeight::BOLD) > width - 2. * inset {
        16.
    } else {
        9.
    };
    div()
        .flex()
        .flex_col()
        .font_family(crate::workbench::UI_FONT)
        .pt(px(22.))
        .pb(px(16.))
        .child(
            div()
                .px(px(inset))
                .flex()
                .flex_col()
                .gap(px(title_gap))
                .child(text(alert.title.clone()).font_weight(gpui_kit::FontWeight::BOLD))
                .when_some(alert.message.clone(), |d, message| d.child(text(message))),
        )
        .when_some(alert.accessory.clone(), |d, accessory| {
            d.child(
                div()
                    .mt(px(16.))
                    .px(px(accessory_inset))
                    .flex()
                    .justify_center()
                    .child(accessory(window, cx)),
            )
        })
        .child(
            div().mt(px(14.)).flex().justify_center().child(
                div()
                    .w(px(BUTTONS))
                    .flex()
                    .when(side_by_side, |d| d.gap(px(SIDE_BY_SIDE_GAP)))
                    .when(!side_by_side, |d| d.flex_col().gap(px(6.)))
                    // Side by side, the default goes on the right.
                    .children(if side_by_side {
                        buttons.into_iter().rev().collect::<Vec<_>>()
                    } else {
                        buttons
                    }),
            ),
        )
        .into_any_element()
}

/// How wide `checkbox` is with `title`: for `Alert::accessory`, so a long
/// title widens the alert as `NSAlert` grows around its accessory.
pub(crate) fn checkbox_width(title: &SharedString, window: &mut Window) -> f32 {
    16. + 9. + text_width(title, gpui_kit::FontWeight::NORMAL, window).ceil()
}

/// `NSButton`'s checkbox in an alert: a 16-point box, then its title
/// (wrapping when it's wider than the alert).
pub(crate) fn checkbox(
    id: &'static str,
    title: impl Into<SharedString>,
    checked: bool,
    toggle: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    Checkbox {
        id,
        title: title.into(),
        checked,
        toggle: Rc::new(toggle),
    }
}

/// `checkbox`, drawn when the theme's colors are at hand.
#[derive(IntoElement)]
struct Checkbox {
    id: &'static str,
    title: SharedString,
    checked: bool,
    toggle: AlertAction,
}

impl RenderOnce for Checkbox {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = palette(cx);
        let (checked, toggle) = (self.checked, self.toggle);
        div()
            .id(self.id)
            .flex()
            .items_center()
            .gap(px(9.))
            .max_w_full()
            .cursor_pointer()
            .on_click(move |_, window, cx| toggle(window, cx))
            .child(
                div()
                    .flex_none()
                    .size(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .when(checked, |d| {
                        d.bg(hsla(p.accent)).child(crate::workbench_chrome::icon(
                            "check",
                            12.,
                            p.text_on_accent,
                        ))
                    })
                    .when(!checked, |d| {
                        d.bg(hsla(p.raised))
                            .border_1()
                            .border_color(hsla(p.hairline_strong))
                    }),
            )
            .child(
                div()
                    .min_w_0()
                    .text_size(px(13.))
                    .line_height(px(16.))
                    .text_color(hsla(p.text))
                    .child(self.title),
            )
    }
}

/// `NSTextField` in an alert: the content's color, a hairline that turns
/// into the theme's focus ring, text at 13.
pub(crate) fn field(
    state: &Entity<gpui_kit::component::input::InputState>,
    width: f32,
    height: f32,
) -> impl IntoElement {
    Field {
        state: state.clone(),
        width,
        height,
    }
}

/// `field`, drawn when the window is at hand to say whether it's focused.
#[derive(IntoElement)]
struct Field {
    state: Entity<gpui_kit::component::input::InputState>,
    width: f32,
    height: f32,
}

impl RenderOnce for Field {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        use gpui_kit::component::input::Input;
        use gpui_kit::component::Sizable as _;
        use gpui_kit::Focusable as _;
        let p = palette(cx);
        let focused = self.state.read(cx).focus_handle(cx).is_focused(window);
        div()
            .w(px(self.width))
            .h(px(self.height))
            .flex()
            .items_center()
            .px(px(6.))
            .rounded(px(7.))
            .border_1()
            .border_color(hsla(if focused {
                p.focus_ring
            } else {
                p.hairline_strong
            }))
            .bg(hsla(p.content))
            .text_color(hsla(p.text))
            .text_size(px(13.))
            .child(
                Input::new(&self.state)
                    .small()
                    .appearance(false)
                    .px_0()
                    .text_size(px(13.)),
            )
    }
}

/// The project trust prompt's list of commands (an `NSTextView` in a bezel,
/// 11-point mono): all of them, scrolling past fourteen lines.
pub(crate) fn command_list(text: String, lines: usize, p: &ChromePalette) -> impl IntoElement {
    let line = 13.;
    div()
        .id("alert-commands")
        .w(px(440.))
        .h(px(lines.clamp(2, 14) as f32 * line + 12.))
        .border_1()
        .border_color(hsla(p.hairline_strong))
        .bg(hsla(p.content))
        .font_family(crate::settings_view::MONO_FONT)
        .text_size(px(11.))
        .line_height(px(line))
        .text_color(hsla(p.text))
        .whitespace_normal()
        // The bezel stays put while the text scrolls inside it.
        .child(
            div()
                .size_full()
                .overflow_y_scrollbar()
                .px(px(10.))
                .py(px(5.))
                .child(text),
        )
}
