//! The Component Gallery (debug builds; the Swift `ComponentGalleryView`):
//! the chrome's components in every built-in theme, dark, light or all,
//! with and without Increase Contrast, to check a theme or a component
//! change at a glance. "Component Gallery" in the palette, and the
//! snapshot's `gallery`, `gallery=dark`, `gallery=light+contrast` actions.

use std::f32::consts::PI;

use gpui_kit::{
    canvas, div, point, prelude::*, px, size, uniform_list, App, Bounds, Context, PathBuilder,
    Pixels, Window, WindowBounds, WindowOptions,
};
use impulse_kit::chrome::{metrics, ChromePalette, Rgba};
use impulse_kit::theme;

use crate::workbench_chrome::{hsla, icon};

/// Which themes it shows.
#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Dark,
    Light,
}

pub struct ComponentGallery {
    filter: Filter,
    increase_contrast: bool,
}

/// Open the gallery in a window of its own. `light`: only light (true) or
/// dark (false) themes; `None`: all of them.
pub fn show(light: Option<bool>, increase_contrast: bool, cx: &mut App) {
    let filter = match light {
        None => Filter::All,
        Some(true) => Filter::Light,
        Some(false) => Filter::Dark,
    };
    let bounds = Bounds::centered(None, size(px(1180.), px(820.)), cx);
    let opened = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui_kit::TitlebarOptions {
                title: Some("Component Gallery".into()),
                ..Default::default()
            }),
            focus: true,
            show: true,
            app_id: Some("impulse-component-gallery".into()),
            ..Default::default()
        },
        cx,
        move |_, cx| {
            cx.new(|_| ComponentGallery {
                filter,
                increase_contrast,
            })
        },
    );
    if let Err(error) = opened {
        log::error!("couldn't open the Component Gallery: {error}");
    }
}

impl ComponentGallery {
    fn themes(&self) -> Vec<&'static str> {
        theme::builtin_theme_names()
            .into_iter()
            .filter(|name| {
                let light = theme::builtin_theme(name).is_some_and(|t| t.is_light);
                match self.filter {
                    Filter::All => true,
                    Filter::Dark => !light,
                    Filter::Light => light,
                }
            })
            .collect()
    }
}

impl Render for ComponentGallery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The gallery's own frame takes the first theme's chrome.
        let themes = self.themes();
        let frame = ChromePalette::new(
            &theme::get_theme(themes.first().copied().unwrap_or("")),
            false,
        );
        let segment = |id: &'static str, label: &'static str, on: bool| {
            div()
                .id(id)
                .px(px(10.))
                .h(px(20.))
                .flex()
                .items_center()
                .rounded(px(metrics::RADIUS_SMALL))
                .text_size(px(11.5))
                .cursor_pointer()
                .text_color(hsla(if on { frame.text } else { frame.text_secondary }))
                .when(on, |d| d.bg(hsla(frame.raised)))
                .child(label)
        };
        let set = |filter: Filter| {
            cx.listener(move |gallery: &mut ComponentGallery, _, _, cx| {
                gallery.filter = filter;
                cx.notify();
            })
        };
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .p(px(12.))
            .border_b_1()
            .border_color(hsla(frame.hairline))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(format!("{} built-in themes", themes.len())),
            )
            .child(
                div()
                    .flex()
                    .gap(px(2.))
                    .p(px(2.))
                    .rounded(px(metrics::RADIUS_SMALL + 2.))
                    .bg(hsla(frame.hover))
                    .child(
                        segment("all", "All", self.filter == Filter::All)
                            .on_click(set(Filter::All)),
                    )
                    .child(
                        segment("dark", "Dark", self.filter == Filter::Dark)
                            .on_click(set(Filter::Dark)),
                    )
                    .child(
                        segment("light", "Light", self.filter == Filter::Light)
                            .on_click(set(Filter::Light)),
                    ),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("contrast")
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .on_click(cx.listener(|gallery, _, _, cx| {
                        gallery.increase_contrast = !gallery.increase_contrast;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .size(px(14.))
                            .rounded(px(3.))
                            .border_1()
                            .border_color(hsla(frame.hairline_strong))
                            .when(self.increase_contrast, |d| {
                                d.bg(hsla(frame.accent)).child(icon(
                                    "check",
                                    12.,
                                    frame.text_on_accent,
                                ))
                            }),
                    )
                    .child("Increase Contrast"),
            );
        // Two cards a row.
        let rows: Vec<Vec<&'static str>> = themes.chunks(2).map(<[_]>::to_vec).collect();
        let contrast = self.increase_contrast;
        let list = uniform_list("gallery", rows.len(), move |range, _, _| {
            range
                .map(|i| {
                    div()
                        .w_full()
                        .flex()
                        .gap(px(12.))
                        .px(px(12.))
                        .pb(px(12.))
                        .children(
                            rows[i]
                                .iter()
                                .map(|name| div().flex_1().min_w_0().child(card(name, contrast))),
                        )
                        .when(rows[i].len() == 1, |d| d.child(div().flex_1()))
                        .into_any_element()
                })
                .collect()
        })
        .flex_1()
        .w_full()
        .pt(px(12.));
        div()
            .size_full()
            .flex()
            .flex_col()
            .font_family(crate::workbench::UI_FONT)
            .text_size(px(13.))
            .line_height(gpui_kit::relative(crate::workbench::UI_LINE_HEIGHT))
            .bg(hsla(frame.window))
            .text_color(hsla(frame.text))
            .child(header)
            .child(list)
    }
}

/// One theme's chrome: surfaces, controls, list rows, text, status and git
/// colors, and a toast (`ThemeSampleCard`).
fn card(name: &str, increase_contrast: bool) -> impl IntoElement {
    let resolved = theme::get_theme(name);
    let p = ChromePalette::new(&resolved, increase_contrast);
    let mono = crate::settings_view::MONO_FONT;
    let chip = |children: Vec<gpui_kit::AnyElement>| {
        div()
            .flex()
            .items_center()
            .gap(px(5.))
            .px(px(7.))
            .h(px(20.))
            .rounded(px(metrics::RADIUS_SMALL + 1.))
            .bg(hsla(p.raised))
            .font_family(mono)
            .text_size(px(11.))
            .text_color(hsla(p.text_secondary))
            .children(children)
    };
    let icon_button = |name: &str, active: bool| {
        div()
            .size(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(metrics::RADIUS_SMALL + 1.))
            .when(active, |d| d.bg(hsla(p.pressed)))
            .child(icon(
                name,
                metrics::ICON,
                if active { p.text } else { p.text_secondary },
            ))
    };
    let titlebar = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(10.))
        .h(px(36.))
        .bg(hsla(p.chrome))
        .border_b_1()
        .border_color(hsla(p.hairline))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(hsla(p.text))
                .child(theme::theme_display_name(name)),
        )
        .child(
            div()
                .text_size(px(10.))
                .text_color(hsla(p.text_tertiary))
                .child(if resolved.is_light { "light" } else { "dark" }),
        )
        .child(div().flex_1())
        .child(chip(vec![
            icon("git-branch", 11., p.text_secondary).into_any_element(),
            "main".into_any_element(),
        ]))
        .child(chip(vec![
            div()
                .text_color(hsla(p.git_added))
                .child("+12")
                .into_any_element(),
            div()
                .text_color(hsla(p.git_deleted))
                .child("\u{2212}3")
                .into_any_element(),
        ]))
        .child(icon_button("search", false))
        .child(icon_button("panel-right", true));
    let row = |file: &'static str,
               status: &'static str,
               color: Rgba,
               selected: bool,
               hovered: bool,
               focused: bool| {
        div().px(px(6.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(8.))
                .h(px(24.))
                .rounded(px(metrics::RADIUS_SMALL + 1.))
                .when(selected, |d| d.bg(hsla(p.selection)))
                .when(!selected && hovered, |d| d.bg(hsla(p.hover)))
                .when(selected && focused, |d| {
                    d.border(px(1.5)).border_color(hsla(p.focus_ring))
                })
                .child(icon("file", 12., p.text_tertiary))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.))
                        .text_color(hsla(p.text))
                        .child(file),
                )
                .child(
                    div()
                        .font_family(mono)
                        .text_size(px(11.))
                        .font_weight(gpui_kit::FontWeight::BOLD)
                        .text_color(hsla(color))
                        .child(status),
                ),
        )
    };
    let panel = div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .py(px(6.))
        .w(px(220.))
        .flex_none()
        .bg(hsla(p.panel))
        .border_r_1()
        .border_color(hsla(p.hairline))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(12.))
                .h(px(26.))
                .text_size(px(10.5))
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(hsla(p.text_tertiary))
                .child("CHANGES")
                .child("3"),
        )
        .child(row("cache.swift", "M", p.git_modified, false, false, false))
        .child(row("new.txt", "U", p.git_untracked, false, true, false))
        .child(row("README.md", "A", p.git_added, true, false, false))
        .child(row("old.swift", "D", p.git_deleted, true, false, true))
        .child(row("moved.swift", "R", p.git_renamed, false, false, false))
        .child(row(
            "conflict.swift",
            "C",
            p.git_conflict,
            false,
            false,
            false,
        ));
    let button = |label: &'static str, kind: u8| {
        // 0 primary, 1 secondary, 2 ghost, 3 danger.
        div()
            .flex()
            .items_center()
            .gap(px(5.))
            .px(px(9.))
            .h(px(22.))
            .rounded(px(metrics::RADIUS - 1.))
            .text_size(px(11.5))
            .font_weight(gpui_kit::FontWeight::MEDIUM)
            .text_color(hsla(match kind {
                0 => p.text_on_accent,
                3 => p.danger,
                _ => p.text,
            }))
            .when(kind == 0, |d| {
                d.bg(hsla(p.accent))
                    .child(icon("check", 12., p.text_on_accent))
            })
            .when(kind == 1, |d| {
                d.bg(hsla(p.raised))
                    .border_1()
                    .border_color(hsla(p.hairline_strong))
            })
            .child(label)
    };
    let dots = [
        ("ok", p.success),
        ("warn", p.warning),
        ("err", p.danger),
        ("info", p.info),
        ("busy", p.working),
        ("you", p.attention),
    ];
    let badge = |text: &'static str, color: Option<Rgba>| {
        div()
            .px(px(5.))
            .min_w(px(16.))
            .h(px(15.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(hsla(color.unwrap_or(p.raised)))
            .text_size(px(10.))
            .font_weight(gpui_kit::FontWeight::BOLD)
            .text_color(hsla(if color.is_some() {
                p.text_on_accent
            } else {
                p.text_secondary
            }))
            .child(text)
    };
    let content = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(10.))
        .p(px(10.))
        .bg(hsla(p.content))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .text_size(px(12.))
                .child(div().text_color(hsla(p.text)).child("Primary text"))
                .child(
                    div()
                        .text_color(hsla(p.text_secondary))
                        .child("Secondary text"),
                )
                .child(
                    div()
                        .text_color(hsla(p.text_tertiary))
                        .child("Tertiary text"),
                ),
        )
        .child(
            div()
                .flex()
                .gap(px(6.))
                .child(button("Commit", 0))
                .child(button("Stage", 1))
                .child(button("Later", 2))
                .child(button("Discard", 3)),
        )
        .child(
            div()
                .flex()
                .gap(px(10.))
                .children(dots.map(|(label, color)| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(div().size(px(7.)).rounded_full().bg(hsla(color)))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(hsla(p.text_secondary))
                                .child(label),
                        )
                })),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(badge("3", None))
                .child(badge("12", Some(p.accent)))
                .child(
                    div()
                        .px(px(4.))
                        .h(px(16.))
                        .flex()
                        .items_center()
                        .rounded(px(3.))
                        .border_1()
                        .border_color(hsla(p.hairline_strong))
                        .text_size(px(10.5))
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(hsla(p.text_tertiary))
                        .child(if cfg!(target_os = "macos") {
                            "\u{21e7}\u{2318}P"
                        } else {
                            "Ctrl+Shift+P"
                        }),
                )
                .child(ring(Some(0.6), p.working))
                .child(ring(None, p.text_secondary))
                .child(
                    div()
                        .px(px(6.))
                        .py(px(2.))
                        .rounded_full()
                        .bg(hsla(p.accent_soft))
                        .text_size(px(11.))
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(hsla(p.accent))
                        .child("accent"),
                ),
        )
        .child(
            // A toast (`ToastView`).
            div()
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
                .child(icon("circle-check", 14., p.success))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .text_color(hsla(p.text))
                                .child("Committed 3 files"),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(hsla(p.text_secondary))
                                .child("on main"),
                        ),
                )
                .child(button("Undo", 1))
                .child(icon("x", 12., p.text_tertiary)),
        );
    let status = div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(10.))
        .h(px(24.))
        .bg(hsla(p.chrome))
        .border_t_1()
        .border_color(hsla(p.hairline))
        .text_size(px(11.))
        .text_color(hsla(p.text_secondary))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(icon("git-branch", 11., p.text_secondary))
                .child("main"),
        )
        .child("3 changes")
        .child(div().flex_1())
        .child("Ln 12, Col 4");
    div()
        .flex()
        .flex_col()
        .rounded(px(8.))
        .overflow_hidden()
        .border_1()
        .border_color(hsla(p.hairline_strong))
        .child(titlebar)
        .child(div().flex().child(panel).child(content))
        .child(status)
}

/// `ProgressRing`: a faint circle and an arc for `progress` (a quarter
/// when unknown).
fn ring(progress: Option<f32>, color: Rgba) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, _| {
            let width = px(2.);
            let radius = bounds.size.width / 2. - width / 2.;
            let center = bounds.center();
            let arc = |from: f32, to: f32| {
                let mut path = PathBuilder::stroke(width);
                let steps = 32;
                for step in 0..=steps {
                    let angle = from + (to - from) * step as f32 / steps as f32;
                    let at = point(
                        center.x + radius * angle.cos(),
                        center.y + radius * angle.sin(),
                    );
                    if step == 0 {
                        path.move_to(at);
                    } else {
                        path.line_to(at);
                    }
                }
                path.build().ok()
            };
            if let Some(path) = arc(0., 2. * PI) {
                window.paint_path(path, hsla(color.opacity(0.25)));
            }
            let start = -PI / 2.;
            let sweep = progress.unwrap_or(0.28).clamp(0.02, 1.) * 2. * PI;
            if let Some(path) = arc(start, start + sweep) {
                window.paint_path(path, hsla(color));
            }
        },
    )
    .size(px(14.))
    .flex_none()
}
