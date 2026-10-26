//! Alerts drawn as macOS 26's `NSAlert` sheet, which the Swift app uses for
//! its questions: 260 wide, a bold title over the message, an optional
//! accessory (a checkbox, fields) centered under it, and pill buttons, the
//! first being the default (Return) one, in a 228-wide block: stacked, or
//! two side by side (the default on the right) when both titles fit half of
//! it. A wide accessory widens the alert. Escape picks the button titled
//! "Cancel", and ⌘D the one titled "Don't Save", as in `NSAlert`.
//!
//! The colors are the system's, not the theme's, as `NSAlert` has them.

use std::rc::Rc;

use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::{
    actions, div, prelude::*, px, AnyElement, App, Entity, Hsla, KeyBinding, MouseButton,
    SharedString, Window,
};

use crate::workbench_chrome::hsla;
use impulse_kit::chrome::Rgba;

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

/// Whether alerts take the light system colors (the theme is light).
pub(crate) fn is_light(cx: &App) -> bool {
    use gpui_kit::component::ActiveTheme as _;
    !cx.theme().mode.is_dark()
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
        let colors = Colors::new(is_light(cx));
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
    /// A destructive button's fill and title: red, as `NSAlert` tints it.
    danger_fill: Hsla,
    danger_fill_hover: Hsla,
    danger_text: Hsla,
}

impl Colors {
    fn new(light: bool) -> Self {
        let rgba = |hex: &str| hsla(Rgba::from_hex(hex));
        let ink = if light { Rgba::BLACK } else { Rgba::WHITE };
        Colors {
            background: rgba(if light { "#F5F5F5" } else { "#1C1C1C" }),
            border: hsla(ink.opacity(if light { 0.12 } else { 0.16 })),
            // NSAlert's title and message: #FAFAFA on its dark background.
            text: hsla(ink.opacity(if light { 0.88 } else { 0.98 })),
            button: hsla(ink.opacity(if light { 0.06 } else { 0.08 })),
            button_hover: hsla(ink.opacity(if light { 0.1 } else { 0.12 })),
            button_pressed: hsla(ink.opacity(if light { 0.14 } else { 0.16 })),
            button_text: hsla(ink.opacity(0.85)),
            default: rgba(if light { "#007AFF" } else { "#0A84FF" }),
            default_hover: rgba(if light { "#1A87FF" } else { "#2490FF" }),
            danger_fill: rgba(if light { "#F6D3D1" } else { "#562927" }),
            danger_fill_hover: rgba(if light { "#F2C4C1" } else { "#63302D" }),
            danger_text: rgba(if light { "#E0241B" } else { "#F01D15" }),
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
                    gpui_kit::white()
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
    light: bool,
    toggle: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let ink = if light { Rgba::BLACK } else { Rgba::WHITE };
    div()
        .id(id)
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
                .bg(if checked {
                    hsla(Rgba::from_hex(if light { "#007AFF" } else { "#0A84FF" }))
                } else {
                    hsla(ink.opacity(0.1))
                })
                .when(checked, |d| {
                    d.child(crate::workbench_chrome::icon("check", 12., Rgba::WHITE))
                }),
        )
        .child(
            div()
                .min_w_0()
                .text_size(px(13.))
                .line_height(px(16.))
                .text_color(hsla(ink.opacity(0.88)))
                .child(title.into()),
        )
}

/// `NSTextField` in an alert: the field color, a hairline that turns into
/// the accent's focus ring, text at 13.
pub(crate) fn field(
    state: &Entity<gpui_kit::component::input::InputState>,
    width: f32,
    height: f32,
    light: bool,
) -> impl IntoElement {
    Field {
        state: state.clone(),
        width,
        height,
        light,
    }
}

/// `field`, drawn when the window is at hand to say whether it's focused.
#[derive(IntoElement)]
struct Field {
    state: Entity<gpui_kit::component::input::InputState>,
    width: f32,
    height: f32,
    light: bool,
}

impl RenderOnce for Field {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        use gpui_kit::component::input::Input;
        use gpui_kit::component::Sizable as _;
        use gpui_kit::Focusable as _;
        let light = self.light;
        let ink = if light { Rgba::BLACK } else { Rgba::WHITE };
        let focused = self.state.read(cx).focus_handle(cx).is_focused(window);
        let ring = Colors::new(light).default.opacity(0.75);
        div()
            .w(px(self.width))
            .h(px(self.height))
            .flex()
            .items_center()
            .px(px(6.))
            .rounded(px(7.))
            .border_1()
            .border_color(if focused {
                ring
            } else {
                hsla(ink.opacity(0.1))
            })
            // NSTextField's fill: opaque, not a tint of what's behind.
            .bg(if light {
                gpui_kit::white()
            } else {
                hsla(Rgba::from_hex("#171717"))
            })
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
pub(crate) fn command_list(text: String, lines: usize, light: bool) -> impl IntoElement {
    let ink = if light { Rgba::BLACK } else { Rgba::WHITE };
    let line = 13.;
    div()
        .id("alert-commands")
        .w(px(440.))
        .h(px(lines.clamp(2, 14) as f32 * line + 12.))
        .border_1()
        .border_color(hsla(ink.opacity(0.25)))
        .font_family(crate::settings_view::MONO_FONT)
        .text_size(px(11.))
        .line_height(px(line))
        .text_color(hsla(ink.opacity(0.88)))
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
