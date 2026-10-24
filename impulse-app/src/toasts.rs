//! Toasts, as the Swift app's `ToastCenter` and `ToastView`: up to three
//! at a time, stacked over the bottom of the window's middle above the
//! status bar, each an icon in its kind's color, the message and an
//! optional detail, its actions as secondary `ChromeButton`s and an x.
//! They go after six seconds unless told otherwise (`Toast::lifetime`);
//! one with an action stays fifteen, long enough to reach the button.
//!
//! `Toast` keeps the builder names of GPUI Kit's `Notification`, which the
//! app used before: the constructors take the text, and `title` puts the
//! message line above it, the text becoming its detail.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{
    div, prelude::*, px, AnyElement, App, BoxShadow, FontWeight, Global, SharedString, Window,
    WindowId,
};
use impulse_kit::chrome::{metrics, ChromePalette, Rgba};

use crate::tasks::Run;
use crate::workbench_chrome::{chrome_button, hsla, icon, ButtonKind};

/// How long a toast stays when nothing says otherwise.
const LIFETIME: Duration = Duration::from_secs(6);
/// How long one with an action stays.
const ACTION_LIFETIME: Duration = Duration::from_secs(15);
/// At most this many at once; a new one drops the oldest.
const MAX: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Info,
    Success,
    Warning,
    Error,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Info => "info",
            Kind::Success => "success",
            Kind::Warning => "warning",
            Kind::Error => "error",
        }
    }
}

/// A toast to show (`show`).
#[derive(Clone)]
pub(crate) struct Toast {
    kind: Kind,
    text: SharedString,
    title: Option<SharedString>,
    actions: Vec<(SharedString, Run)>,
    lifetime: Option<Option<Duration>>,
}

impl Toast {
    pub(crate) fn new(kind: Kind, text: impl Into<SharedString>) -> Self {
        Toast {
            kind,
            text: text.into(),
            title: None,
            actions: Vec::new(),
            lifetime: None,
        }
    }

    pub(crate) fn info(text: impl Into<SharedString>) -> Self {
        Self::new(Kind::Info, text)
    }

    pub(crate) fn success(text: impl Into<SharedString>) -> Self {
        Self::new(Kind::Success, text)
    }

    pub(crate) fn warning(text: impl Into<SharedString>) -> Self {
        Self::new(Kind::Warning, text)
    }

    /// A line above the text, which then reads as its detail.
    pub(crate) fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// A button; using it runs `run` and closes the toast.
    pub(crate) fn action(mut self, title: impl Into<SharedString>, run: Run) -> Self {
        self.actions.push((title.into(), run));
        self
    }

    /// How long it stays (`None`: until it's closed).
    pub(crate) fn lifetime(mut self, lifetime: Option<Duration>) -> Self {
        self.lifetime = Some(lifetime);
        self
    }

    /// The bold line and the one under it.
    fn lines(&self) -> (SharedString, Option<SharedString>) {
        match &self.title {
            Some(title) => (
                title.clone(),
                Some(self.text.clone()).filter(|t| !t.is_empty()),
            ),
            None => (self.text.clone(), None),
        }
    }
}

struct Shown {
    id: u64,
    toast: Toast,
}

/// The toasts up in each window.
#[derive(Default)]
struct Toasts {
    next: u64,
    windows: HashMap<WindowId, Vec<Shown>>,
}

impl Global for Toasts {}

/// Show `toast` in `window`.
pub(crate) fn show(window: &mut Window, cx: &mut App, toast: Toast) {
    #[cfg(debug_assertions)]
    {
        let (message, detail) = toast.lines();
        crate::debug_snapshot::note_toast(
            toast.kind.name(),
            &message,
            detail.as_deref(),
            toast
                .actions
                .first()
                .map(|(title, run)| (title.as_ref(), run.clone())),
        );
    }
    let lifetime = toast.lifetime.unwrap_or(Some(if toast.actions.is_empty() {
        LIFETIME
    } else {
        ACTION_LIFETIME
    }));
    let window_id = window.window_handle().window_id();
    let state = cx.default_global::<Toasts>();
    state.next += 1;
    let id = state.next;
    let list = state.windows.entry(window_id).or_default();
    list.push(Shown { id, toast });
    if list.len() > MAX {
        list.drain(..list.len() - MAX);
    }
    window.refresh();
    if let Some(lifetime) = lifetime {
        window
            .spawn(cx, async move |cx| {
                cx.background_executor().timer(lifetime).await;
                let _ = cx.update(|window, cx| dismiss(id, window, cx));
            })
            .detach();
    }
}

fn dismiss(id: u64, window: &mut Window, cx: &mut App) {
    let window_id = window.window_handle().window_id();
    if let Some(list) = cx.default_global::<Toasts>().windows.get_mut(&window_id) {
        list.retain(|s| s.id != id);
    }
    window.refresh();
}

/// The window's toasts, to be the last child of its root (which must be
/// relatively positioned).
pub(crate) fn layer(p: &ChromePalette, window: &Window, cx: &App) -> Option<AnyElement> {
    let window_id = window.window_handle().window_id();
    let list = cx.try_global::<Toasts>()?.windows.get(&window_id)?;
    if list.is_empty() {
        return None;
    }
    let stack = div()
        .absolute()
        .left_0()
        .right_0()
        // The Swift panel sits 18 above the status bar, with 10 of padding.
        .bottom(px(metrics::STATUS_BAR_HEIGHT + 18. + 10.))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(8.))
        .children(list.iter().map(|shown| view(shown, p)));
    Some(
        gpui_kit::deferred(stack)
            .with_priority(2)
            .into_any_element(),
    )
}

fn view(shown: &Shown, p: &ChromePalette) -> AnyElement {
    let toast = &shown.toast;
    let id = shown.id;
    let (name, tint) = match toast.kind {
        Kind::Info => ("info", p.info),
        Kind::Success => ("circle-check", p.success),
        Kind::Warning => ("triangle-alert", p.warning),
        Kind::Error => ("circle-x", p.danger),
    };
    let (message, detail) = toast.lines();
    let mut row = div()
        .id(("toast", id as usize))
        .relative()
        .flex()
        .items_center()
        .gap(px(10.))
        .pl(px(12.))
        .pr(px(6.))
        .py(px(8.))
        .rounded(px(metrics::RADIUS_LARGE))
        .bg(hsla(p.overlay))
        .border_1()
        .border_color(hsla(p.hairline_strong))
        .shadow(vec![BoxShadow {
            color: hsla(Rgba::BLACK.opacity(0.25)),
            offset: gpui_kit::point(px(0.), px(4.)),
            blur_radius: px(12.),
            spread_radius: px(0.),
            inset: false,
        }])
        .font_family(crate::workbench::UI_FONT)
        .line_height(gpui_kit::relative(crate::workbench::UI_LINE_HEIGHT))
        // Clicks stay on the toast, not the pane under it.
        .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
            cx.stop_propagation()
        })
        .child(icon(name, 14., tint))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .min_w(px(200.))
                .max_w(px(380.))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(hsla(p.text))
                        .line_clamp(2)
                        .child(message),
                )
                .children(detail.map(|detail| {
                    div()
                        .text_size(px(11.))
                        .text_color(hsla(p.text_secondary))
                        .line_clamp(3)
                        .child(detail)
                })),
        );
    for (index, (title, run)) in toast.actions.iter().enumerate() {
        let run = run.clone();
        row = row.child(
            chrome_button(
                SharedString::from(format!("toast-{id}-action-{index}")),
                title.clone(),
                None,
                ButtonKind::Secondary,
                false,
                p,
            )
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                run(window, cx);
                dismiss(id, window, cx);
            }),
        );
    }
    let close: Run = Rc::new(move |window, cx| dismiss(id, window, cx));
    row.child(
        div()
            .id(SharedString::from(format!("toast-{id}-close")))
            .size(px(20.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(metrics::RADIUS_SMALL))
            .cursor_pointer()
            .hover(|s| s.bg(hsla(p.hover)))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                close(window, cx)
            })
            .child(icon("x", 11., p.text_secondary)),
    )
    .child(crate::overlays::probe())
    .into_any_element()
}
