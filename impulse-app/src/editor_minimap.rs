//! The minimap beside the editor (Monaco's `minimap`, the `minimap_enabled`
//! setting): the document's lines as runs of one-pixel characters, the
//! visible part under a slider while the pointer is over it, and a click
//! or drag scrolling there. Layout comes from `impulse_kit::minimap`;
//! characters take the text color rather than their syntax colors.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, RopeExt as _};
use gpui_kit::{
    canvas, div, fill, point, px, size, AnyElement, Bounds, Context, DispatchPhase, Entity,
    HitboxBehavior, InteractiveElement as _, IntoElement as _, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Styled as _,
};
use impulse_kit::minimap::{self, COLUMN_WIDTH, MAX_COLUMNS, ROW_HEIGHT};

use crate::editor_tab::EditorTab;
use gpui_kit::component::input::EditorState;

/// The pointer's state over one editor's minimap.
#[derive(Clone, Default)]
pub(crate) struct MinimapState {
    hovered: Rc<Cell<bool>>,
    dragging: Rc<Cell<bool>>,
}

/// Monaco's minimap is as wide as its columns, plus a little air.
const WIDTH: f32 = MAX_COLUMNS as f32 * COLUMN_WIDTH + 8.0;

/// GPUI Kit's vertical scrollbar, at the editor's right edge (`WIDTH` in
/// gpui-base's scrollbar.rs).
// Until editor_tab.rs places the overlay (then drop these allows and
// `minimap`).
#[allow(dead_code)]
const SCROLLBAR_WIDTH: f32 = 16.0;

impl EditorTab {
    /// The minimap beside the editor, when the setting is on.
    pub(crate) fn minimap(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        Some(self.minimap_body(cx)?.flex_none().into_any_element())
    }

    /// What the editor keeps free at its right edge for the minimap
    /// overlay (`Editor::scrollbar_gutter`), when the setting is on: the
    /// minimap and the scrollbar beside it.
    #[allow(dead_code)]
    pub(crate) fn minimap_gutter(&self, cx: &gpui_kit::App) -> Option<Pixels> {
        crate::settings_store::SettingsStore::get(cx)
            .minimap_enabled
            .then(|| px(WIDTH + SCROLLBAR_WIDTH))
    }

    /// The minimap laid over the editor's right edge, as Monaco places it:
    /// between the text and the vertical scrollbar, in the room
    /// `minimap_gutter` keeps. The editor's parent must be `relative()`.
    /// The wheel still scrolls the editor through it.
    #[allow(dead_code)]
    pub(crate) fn minimap_overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        Some(
            self.minimap_body(cx)?
                .absolute()
                .top_0()
                .bottom_0()
                .right(px(SCROLLBAR_WIDTH))
                .block_mouse_except_scroll()
                .into_any_element(),
        )
    }

    fn minimap_body(&self, cx: &mut Context<Self>) -> Option<gpui_kit::Div> {
        if !crate::settings_store::SettingsStore::get(cx).minimap_enabled {
            return None;
        }
        let state = self.state.clone();
        let pointer = self.minimap.clone();
        let tab_size = self.tab_size;
        let theme = cx.theme();
        let ink = theme.foreground.opacity(0.45);
        let slider = theme.foreground.opacity(0.08);
        let slider_active = theme.foreground.opacity(0.14);
        // Monaco's `minimap.background` is the editor's.
        let background = cx
            .try_global::<crate::kit_theme::EditorBackdrop>()
            .map_or(theme.background, |b| b.background);
        Some(
            div().h_full().w(px(WIDTH)).bg(background).child(
                canvas(
                    |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                    move |bounds, hitbox, window, cx| {
                        let layout = Layout::of(&state, bounds, cx);
                        let Some(layout) = layout else {
                            return;
                        };
                        paint_lines(&state, &layout, bounds, tab_size, ink, window, cx);
                        let hovered = hitbox.is_hovered(window);
                        if pointer.hovered.replace(hovered) != hovered {
                            window.refresh();
                        }
                        if hovered || pointer.dragging.get() {
                            let color = if pointer.dragging.get() {
                                slider_active
                            } else {
                                slider
                            };
                            window.paint_quad(fill(layout.slider(bounds), color));
                        }
                        mouse(&state, &pointer, &hitbox, layout, bounds, window);
                    },
                )
                .size_full(),
            ),
        )
    }
}

/// Where the document and the editor's view sit on the minimap.
#[derive(Clone, Copy)]
struct Layout {
    total: usize,
    first: usize,
    first_visible: usize,
    visible: usize,
    line_height: Pixels,
}

impl Layout {
    fn of(state: &Entity<EditorState>, bounds: Bounds<Pixels>, cx: &gpui_kit::App) -> Option<Self> {
        let state = state.read(cx);
        let rows = state.visible_row_range()?;
        let line_height = state.line_height()?;
        let total = state.text().lines_len().max(1);
        let fits = (f32::from(bounds.size.height) / ROW_HEIGHT) as usize;
        let visible = rows.len().max(1);
        Some(Self {
            total,
            first: minimap::first_row(total, fits, rows.start, visible),
            first_visible: rows.start,
            visible,
            line_height,
        })
    }

    /// The editor's view on the minimap.
    fn slider(&self, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
        let top = (self.first_visible as f32 - self.first as f32) * ROW_HEIGHT;
        Bounds::new(
            point(bounds.left(), bounds.top() + px(top)),
            size(bounds.size.width, px(self.visible as f32 * ROW_HEIGHT)),
        )
    }

    /// The document row at `y`.
    fn row_at(&self, y: Pixels, bounds: Bounds<Pixels>) -> usize {
        let offset = f32::from(y - bounds.top()).max(0.) / ROW_HEIGHT;
        (self.first + offset as usize).min(self.total - 1)
    }
}

fn paint_lines(
    state: &Entity<EditorState>,
    layout: &Layout,
    bounds: Bounds<Pixels>,
    tab_size: usize,
    ink: gpui_kit::Hsla,
    window: &mut gpui_kit::Window,
    cx: &gpui_kit::App,
) {
    let text = state.read(cx).text();
    let fits = (f32::from(bounds.size.height) / ROW_HEIGHT) as usize + 1;
    let left = bounds.left() + px(4.);
    for (i, row) in (layout.first..layout.total.min(layout.first + fits)).enumerate() {
        let start = text.line_start_offset(row);
        // Only as much of the line as can show.
        // (cut on a character boundary).
        let end = text.floor_char_boundary(start + text.line_len(row).min(MAX_COLUMNS * 4));
        let line = text.slice(start..end).to_string();
        let y = bounds.top() + px(i as f32 * ROW_HEIGHT);
        for (column, columns) in minimap::runs(&line, tab_size, MAX_COLUMNS) {
            window.paint_quad(fill(
                Bounds::new(
                    point(left + px(column as f32 * COLUMN_WIDTH), y),
                    size(px(columns as f32 * COLUMN_WIDTH), px(ROW_HEIGHT)),
                ),
                ink,
            ));
        }
    }
}

/// A press on the minimap scrolls the clicked line to the middle of the
/// editor, and dragging keeps doing so until the button comes up.
fn mouse(
    state: &Entity<EditorState>,
    pointer: &MinimapState,
    hitbox: &gpui_kit::Hitbox,
    layout: Layout,
    bounds: Bounds<Pixels>,
    window: &mut gpui_kit::Window,
) {
    let scroll = {
        let state = state.clone();
        move |y: Pixels, cx: &mut gpui_kit::App| {
            let row = layout.row_at(y, bounds);
            let top = minimap::scroll_target(row, layout.total, layout.visible);
            state.update(cx, |state, cx| {
                let x = state.scroll_offset().x;
                state.set_scroll_offset(point(x, -layout.line_height * top as f32), cx);
            });
        }
    };
    {
        let hitbox = hitbox.clone();
        let pointer = pointer.clone();
        let scroll = scroll.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble
                && event.button == MouseButton::Left
                && hitbox.is_hovered(window)
            {
                pointer.dragging.set(true);
                scroll(event.position.y, cx);
                cx.stop_propagation();
            }
        });
    }
    {
        let pointer = pointer.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble && pointer.dragging.get() {
                scroll(event.position.y, cx);
            }
        });
    }
    let pointer = pointer.clone();
    window.on_mouse_event(move |_: &MouseUpEvent, _, window, _| {
        if pointer.dragging.replace(false) {
            window.refresh();
        }
    });
}
