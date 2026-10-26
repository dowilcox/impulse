use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, BorderStyle, Bounds, Corners, CursorStyle, Edges, Element, ElementId, EntityId,
    GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    SharedString, StyledText, Subscription, TextLayout, TextRun, TextStyleRefinement, Window,
    transparent_black,
};

use crate::{
    TextSelection, TextSelectionEvent, TextSelectionHandle, TextSelectionRegistration,
    TextSelectionRun,
};

// Impulse: what a link click calls, with the index of the range clicked.
type ClickListener = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// Plain text that participates in the window-scoped [`TextSelection`].
///
/// Use [`Self::new`] for an independent run, or [`Self::with_handle`] when
/// several elements form one selectable document.
///
/// Applications must render one [`crate::TextSelectionLayer`] above their
/// content and call [`crate::init`] during startup. Selection and copy then
/// work without depending on `gpui-component`.
pub struct SelectableText {
    id: ElementId,
    handle: Option<TextSelectionHandle>,
    text: SharedString,
    styled_text: StyledText,
    document_order: u64,
    text_style: Option<TextStyleRefinement>,
    selection_color: Option<Hsla>,
    // Impulse: links, a group of pieces laid out as one text, and the
    // document select-all selects.
    links: Option<(Vec<Range<usize>>, ClickListener)>,
    group: Option<SelectableTextGroup>,
    document: Option<SelectableTextDocument>,
}

impl SelectableText {
    /// Creates a run that owns its own selection.
    pub fn new(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Self {
        Self::build(id.into(), None, text.into())
    }

    /// Creates a run that joins the document `handle` belongs to.
    pub fn with_handle(
        id: impl Into<ElementId>,
        handle: TextSelectionHandle,
        text: impl Into<SharedString>,
    ) -> Self {
        Self::build(id.into(), Some(handle), text.into())
    }

    fn build(id: ElementId, handle: Option<TextSelectionHandle>, text: SharedString) -> Self {
        Self {
            id,
            handle,
            styled_text: StyledText::new(text.clone()),
            text,
            document_order: 0,
            text_style: None,
            selection_color: None,
            links: None,
            group: None,
            document: None,
        }
    }

    /// Places this run in reading order among the others sharing its handle.
    pub fn document_order(mut self, order: u64) -> Self {
        self.document_order = order;
        self
    }

    /// Sets the text style the run is laid out and painted with.
    pub fn text_style(mut self, style: TextStyleRefinement) -> Self {
        self.text_style = Some(style);
        self
    }

    /// Overrides the selection background, which defaults to the theme's
    /// `colors.selection` token.
    pub fn selection_color(mut self, color: Hsla) -> Self {
        self.selection_color = Some(color);
        self
    }

    /// Impulse: lays the text out in `runs` (fonts, colors, decorations), as
    /// [`StyledText::with_runs`] does. Their lengths add up to the text's.
    pub fn runs(mut self, runs: Vec<TextRun>) -> Self {
        self.styled_text = StyledText::new(self.text.clone()).with_runs(runs);
        self
    }

    /// Impulse: makes `ranges` of the text links, as
    /// `InteractiveText::on_click` does, with a pointing hand over them.
    /// A press and release on the same link that doesn't drag calls
    /// `listener` with the link's index; a drag from a link selects.
    pub fn on_click(
        mut self,
        ranges: Vec<Range<usize>>,
        listener: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.links = Some((ranges, Rc::new(listener)));
        self
    }

    /// Impulse: makes this run one piece of a text laid out as several
    /// elements (a paragraph wrapped word by word), selected, copied and
    /// double-clicked as one participant: the pieces' text joins with
    /// nothing between, where separate runs copy a line apart. Give every
    /// piece the same [`Self::document_order`], in reading order.
    pub fn group(mut self, group: &SelectableTextGroup) -> Self {
        self.group = Some(group.clone());
        self
    }

    /// Impulse: makes this run part of `document`, whose
    /// [`SelectableTextDocument::select_all`] selects it whole.
    pub fn document(mut self, document: &SelectableTextDocument) -> Self {
        self.document = Some(document.clone());
        self
    }

    fn paint_selection(
        layout: &gpui::TextLayout,
        range: Range<usize>,
        color: Hsla,
        window: &mut Window,
    ) {
        let (Some(start), Some(end)) = (
            layout.position_for_index(range.start),
            layout.position_for_index(range.end),
        ) else {
            return;
        };
        for bounds in selection_quad_bounds(start, end, layout.bounds(), layout.line_height()) {
            window.paint_quad(PaintQuad {
                bounds,
                background: color.into(),
                corner_radii: Corners::default(),
                border_widths: Edges::default(),
                border_color: transparent_black(),
                border_style: BorderStyle::default(),
            });
        }
    }

    /// Impulse: the selected part of the text, projected for this frame.
    fn project(
        &self,
        state: &SelectableTextFrame,
        layout: &TextLayout,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Range<usize>> {
        let handle = &state.handle;
        // Select-all: the whole text, copied as it is (the participant's
        // fallback text), not as projected from a geometry selection.
        if self
            .document
            .as_ref()
            .is_some_and(SelectableTextDocument::is_all_selected)
        {
            if !handle.has_local_selection(cx) {
                handle.set_local_selection(true, cx);
            }
            let text = match &self.group {
                Some(group) => group.add_text(&self.text),
                None => self.text.to_string(),
            };
            handle.set_fallback_copy_text(text, cx);
            return (!self.text.is_empty()).then(|| 0..self.text.len());
        }
        let run = TextSelectionRun::new(self.text.clone(), layout.clone(), bounds);
        if let Some(group) = &self.group {
            // Each piece projects the pieces so far, so the last one leaves
            // the participant with them all; the window's selected text
            // grows piece by piece, so a change is seen in this piece's own
            // selection from the frame before.
            let runs = group.add_run(run);
            let projection = handle.update_runs(&runs, cx);
            let range = projection.ranges().last().cloned().flatten();
            if state.links.painted.replace(range.clone()) != range {
                window.refresh();
            }
            return range;
        }
        // Only a run in the selection can change the selected text, and
        // asking for it walks every participant: skip that for the others.
        let selected = handle.snapshot(cx).is_some() || handle.has_local_selection(cx);
        let selected_text_before = selected.then(|| TextSelection::selected_text(window, cx));
        let projection = handle.update_runs(&[run.with_document_order(self.document_order)], cx);
        if selected_text_before
            .is_some_and(|before| before != TextSelection::selected_text(window, cx))
        {
            window.refresh();
        }
        projection.ranges().first().cloned().flatten()
    }

    /// Impulse: the links' pointing hand, and their clicks.
    fn paint_links(
        &mut self,
        state: &SelectableTextFrame,
        layout: &TextLayout,
        hitbox: &Hitbox,
        window: &mut Window,
    ) {
        let Some((ranges, listener)) = self.links.take() else {
            return;
        };
        let ranges: Rc<[Range<usize>]> = ranges.into();
        let link_at = {
            let (layout, ranges) = (layout.clone(), ranges.clone());
            move |position: Point<Pixels>| {
                layout
                    .index_for_position(position)
                    .ok()
                    .filter(|index| ranges.iter().any(|range| range.contains(index)))
            }
        };
        let view = window.current_view();
        let links = state.links.clone();

        let over = hitbox.is_hovered(window) && link_at(window.mouse_position()).is_some();
        links.hovered.set(over);
        if over {
            window.set_cursor_style(CursorStyle::PointingHand, hitbox);
        }
        window.on_mouse_event({
            let (hitbox, links, link_at) = (hitbox.clone(), links.clone(), link_at.clone());
            move |event: &MouseMoveEvent, phase, window, cx| {
                if !phase.bubble() {
                    return;
                }
                let over = hitbox.is_hovered(window) && link_at(event.position).is_some();
                // The cursor is set as the run paints: paint again.
                if links.hovered.replace(over) != over {
                    cx.notify(view);
                }
            }
        });
        window.on_mouse_event({
            let (hitbox, links, link_at) = (hitbox.clone(), links.clone(), link_at.clone());
            move |event: &MouseDownEvent, phase, window, _| {
                if !phase.bubble() || event.button != MouseButton::Left {
                    return;
                }
                // A double click selects a word; the first click followed
                // the link already.
                let pressed = (event.click_count == 1 && hitbox.is_hovered(window))
                    .then(|| link_at(event.position))
                    .flatten();
                links
                    .pressed
                    .set(pressed.map(|index| (index, event.position)));
            }
        });
        let (hitbox, layout) = (hitbox.clone(), layout.clone());
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if !phase.bubble() || event.button != MouseButton::Left {
                return;
            }
            let Some((down, at)) = links.pressed.take() else {
                return;
            };
            if !hitbox.is_hovered(window) || !is_click(at, event.position) {
                return;
            }
            let Ok(up) = layout.index_for_position(event.position) else {
                return;
            };
            let Some(index) = ranges
                .iter()
                .position(|range| range.contains(&down) && range.contains(&up))
            else {
                return;
            };
            // A click that moved a pixel leaves a sliver of selection.
            if TextSelection::has_selection(window, cx) {
                TextSelection::clear(window, cx);
            }
            listener(index, window, cx);
        });
    }
}

/// Impulse: whether a press at `down` released at `up` is a click rather
/// than a drag.
fn is_click(down: Point<Pixels>, up: Point<Pixels>) -> bool {
    const SLOP: f32 = 4.;
    f32::from(up.x - down.x).abs() <= SLOP && f32::from(up.y - down.y).abs() <= SLOP
}

fn selection_quad_bounds(
    start: Point<Pixels>,
    end: Point<Pixels>,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
) -> Vec<Bounds<Pixels>> {
    if start.y == end.y {
        return vec![Bounds::from_corners(
            start,
            Point::new(end.x, end.y + line_height),
        )];
    }

    let mut quads = vec![Bounds::from_corners(
        start,
        Point::new(bounds.right(), start.y + line_height),
    )];
    if end.y > start.y + line_height {
        quads.push(Bounds::from_corners(
            Point::new(bounds.left(), start.y + line_height),
            Point::new(bounds.right(), end.y),
        ));
    }
    quads.push(Bounds::from_corners(
        Point::new(bounds.left(), end.y),
        Point::new(end.x, end.y + line_height),
    ));
    quads
}

/// Impulse: the pieces of one text laid out as separate
/// [`SelectableText`]s, which select as one participant
/// ([`SelectableText::group`]).
///
/// It gathers one frame's pieces: make a new one each time the pieces are
/// built. The participant itself is kept by the first piece.
#[derive(Clone, Default)]
pub struct SelectableTextGroup(Rc<RefCell<GroupFrame>>);

#[derive(Default)]
struct GroupFrame {
    handle: Option<TextSelectionHandle>,
    bounds: Option<Bounds<Pixels>>,
    text_bounds: Vec<Bounds<Pixels>>,
    runs: Vec<TextSelectionRun>,
    text: String,
}

impl SelectableTextGroup {
    /// An empty group.
    pub fn new() -> Self {
        Self::default()
    }

    fn handle(&self) -> Option<TextSelectionHandle> {
        self.0.borrow().handle.clone()
    }

    fn set_handle(&self, handle: TextSelectionHandle) {
        self.0.borrow_mut().handle = Some(handle);
    }

    /// Adds a piece's bounds: the bounds of the pieces so far, and each.
    fn add_bounds(&self, bounds: Bounds<Pixels>) -> (Bounds<Pixels>, Vec<Bounds<Pixels>>) {
        let mut frame = self.0.borrow_mut();
        let union = frame.bounds.map_or(bounds, |union| union.union(&bounds));
        frame.bounds = Some(union);
        frame.text_bounds.push(bounds);
        (union, frame.text_bounds.clone())
    }

    /// Adds a piece's run, after the others: the runs so far.
    fn add_run(&self, run: TextSelectionRun) -> Vec<TextSelectionRun> {
        let mut frame = self.0.borrow_mut();
        let order = frame.runs.len() as u64;
        frame.runs.push(run.with_document_order(order));
        frame.runs.clone()
    }

    /// Adds a piece's text: the text so far.
    fn add_text(&self, text: &str) -> String {
        let mut frame = self.0.borrow_mut();
        frame.text.push_str(text);
        frame.text.clone()
    }
}

/// Impulse: the [`SelectableText`]s of one document, for select-all
/// ([`SelectableText::document`]). Keep it as long as the document.
///
/// [`Self::select_all`] clears the window's selection and selects every
/// run of the document, which copy then joins a line apart in document
/// order; the next press anywhere in the window clears it, as it clears
/// any selection.
#[derive(Clone, Default)]
pub struct SelectableTextDocument(Rc<Cell<bool>>);

impl SelectableTextDocument {
    /// A document with nothing selected.
    pub fn new() -> Self {
        Self::default()
    }

    /// Selects all of the document. The runs paint it when they next paint:
    /// a cached view drawing them has to be notified.
    pub fn select_all(&self, window: &mut Window, cx: &mut App) {
        TextSelection::clear(window, cx);
        self.0.set(true);
        window.refresh();
    }

    /// Whether all of the document is selected.
    pub fn is_all_selected(&self) -> bool {
        self.0.get()
    }
}

/// Impulse: what a run keeps from frame to frame.
#[derive(Default)]
struct Retained {
    /// Its own participant, when it's given none.
    handle: Option<TextSelectionHandle>,
    /// The participant whose selection changes redraw the view the run is
    /// in, and the subscription doing it.
    watched: Option<(EntityId, Subscription)>,
    /// The document whose select-all clearing the participant ends.
    clears: Option<SelectableTextDocument>,
    links: Rc<LinkState>,
}

#[derive(Default)]
struct LinkState {
    /// A press on a link: the index and the position pressed.
    pressed: Cell<Option<(usize, Point<Pixels>)>>,
    /// Whether the pointer was over a link when the run last painted.
    hovered: Cell<bool>,
    /// The piece of a group painted selected last frame.
    painted: RefCell<Option<Range<usize>>>,
}

/// Impulse: what a run takes from layout to paint.
#[doc(hidden)]
pub struct SelectableTextFrame {
    handle: TextSelectionHandle,
    links: Rc<LinkState>,
}

/// Impulse: notifies `view` when `handle`'s selection changes. The run
/// paints its selection, and a cached view would replay the frame before.
fn watch(handle: &TextSelectionHandle, view: EntityId, cx: &mut App) -> Subscription {
    let selected = Cell::new(false);
    handle.subscribe(
        move |event, cx| {
            if let TextSelectionEvent::SelectionChanged(snapshot) = event {
                let was = selected.replace(snapshot.is_some());
                if was || snapshot.is_some() {
                    cx.notify(view);
                }
            }
        },
        cx,
    )
}

impl IntoElement for SelectableText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectableText {
    type RequestLayoutState = SelectableTextFrame;
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let view = window.current_view();
        let frame = window.with_element_state(
            global_id.expect("SelectableText must have a stable element id"),
            |retained: Option<Retained>, _| {
                let mut retained = retained.unwrap_or_default();
                // Impulse: a group's later pieces join the participant its
                // first piece keeps.
                let joined = self.group.as_ref().and_then(SelectableTextGroup::handle);
                let owner = joined.is_none();
                let handle = joined.or_else(|| self.handle.clone()).unwrap_or_else(|| {
                    retained
                        .handle
                        .get_or_insert_with(|| TextSelectionHandle::new(self.text.clone(), cx))
                        .clone()
                });
                if owner {
                    if let Some(group) = &self.group {
                        group.set_handle(handle.clone());
                    }
                    if retained
                        .watched
                        .as_ref()
                        .is_none_or(|(id, _)| *id != handle.entity_id())
                    {
                        retained.watched = Some((handle.entity_id(), watch(&handle, view, cx)));
                    }
                    if let Some(document) = &self.document
                        && retained
                            .clears
                            .as_ref()
                            .is_none_or(|clears| !Rc::ptr_eq(&clears.0, &document.0))
                    {
                        let selected = document.0.clone();
                        handle.clear_with(
                            move |cx| {
                                if selected.replace(false) {
                                    cx.notify(view);
                                }
                            },
                            cx,
                        );
                        retained.clears = Some(document.clone());
                    }
                }
                let frame = SelectableTextFrame {
                    handle,
                    links: retained.links.clone(),
                };
                (frame, retained)
            },
        );
        let (layout_id, ()) = if let Some(style) = self.text_style.clone() {
            window.with_text_style(Some(style), |window| {
                self.styled_text
                    .request_layout(global_id, inspector_id, window, cx)
            })
        } else {
            self.styled_text
                .request_layout(global_id, inspector_id, window, cx)
        };
        (layout_id, frame)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        frame: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.styled_text
            .prepaint(global_id, inspector_id, bounds, &mut (), window, cx);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let registration = match &self.group {
            // Impulse: a group's pieces register as one text, each with the
            // pieces so far, so the last one's registration holds them all.
            Some(group) => {
                let (union, text_bounds) = group.add_bounds(bounds);
                let union_hitbox = window.insert_hitbox(union, HitboxBehavior::Normal);
                TextSelectionRegistration::new(union_hitbox, union).with_text_bounds(text_bounds)
            }
            None => TextSelectionRegistration::new(hitbox.clone(), bounds)
                .with_text_bounds(vec![bounds]),
        };
        let registration = registration
            .with_document_order(self.document_order)
            .with_rendered_element(&frame.handle, window, cx);
        frame.handle.register(registration, window, cx);
        hitbox
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        frame: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.styled_text.layout().clone();
        let selected = self.project(frame, &layout, bounds, window, cx);
        let color = self
            .selection_color
            .unwrap_or_else(|| crate::Theme::global(cx).tokens.colors.selection);
        if let Some(range) = selected {
            Self::paint_selection(&layout, range, color, window);
        }
        self.styled_text.paint(
            global_id,
            inspector_id,
            bounds,
            &mut (),
            &mut (),
            window,
            cx,
        );
        self.paint_links(frame, &layout, hitbox, window);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{
        AppContext as _, Bounds, Context, IntoElement, Modifiers, MouseButton, ParentElement as _,
        Render, Styled as _, TestAppContext, VisualTestContext, Window, div, point, px, size,
    };

    use super::{SelectableText, SelectableTextDocument, SelectableTextGroup};
    use crate::{TextSelection, TextSelectionHandle, TextSelectionLayer};

    struct SelectableTextTestView;

    impl Render for SelectableTextTestView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(TextSelectionLayer).child(
                div()
                    .w(px(240.))
                    .h(px(32.))
                    .child(SelectableText::new("local", "alpha beta")),
            )
        }
    }

    /// Impulse: a paragraph in three pieces, a line of plain text under it
    /// with a link, both in one document.
    struct ImpulseTestView {
        document: SelectableTextDocument,
        clicks: Rc<Cell<usize>>,
    }

    impl Render for ImpulseTestView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let group = SelectableTextGroup::new();
            let clicks = self.clicks.clone();
            div()
                .size_full()
                .child(TextSelectionLayer)
                .child(
                    div().w(px(400.)).h(px(32.)).flex().children(
                        ["alpha ", "beta ", "gamma"]
                            .into_iter()
                            .enumerate()
                            .map(|(i, piece)| {
                                SelectableText::new(("piece", i), piece)
                                    .group(&group)
                                    .document_order(1)
                                    .document(&self.document)
                            }),
                    ),
                )
                .child(
                    div().w(px(400.)).h(px(32.)).child(
                        SelectableText::new("link", "docs")
                            .document_order(2)
                            .document(&self.document)
                            .on_click(vec![0..4], move |_, _, _| clicks.set(clicks.get() + 1)),
                    ),
                )
        }
    }

    fn impulse_view(
        cx: &mut TestAppContext,
    ) -> (
        SelectableTextDocument,
        Rc<Cell<usize>>,
        &mut VisualTestContext,
    ) {
        let document = SelectableTextDocument::new();
        let clicks = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view({
            let (document, clicks) = (document.clone(), clicks.clone());
            move |_, _| ImpulseTestView { document, clicks }
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        (document, clicks, cx)
    }

    fn drag(cx: &mut VisualTestContext, from: (f32, f32), to: (f32, f32)) {
        cx.simulate_mouse_down(
            point(px(from.0), px(from.1)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_move(
            point(px(to.0), px(to.1)),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        cx.simulate_mouse_up(
            point(px(to.0), px(to.1)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    }

    #[gpui::test]
    fn explicit_handle_constructor_preserves_document_contract(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let handle = TextSelectionHandle::new("alpha beta", cx);
            let _ = SelectableText::with_handle("plain", handle, "alpha beta").document_order(42);
        });
    }

    #[gpui::test]
    fn local_handle_participates_in_window_selection(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| SelectableTextTestView);
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        cx.simulate_mouse_down(
            gpui::point(px(1.), px(12.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_move(
            gpui::point(px(220.), px(12.)),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        cx.simulate_mouse_up(
            gpui::point(px(220.), px(12.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.update(|window, cx| {
            let _ = window.draw(cx);
            assert_eq!(TextSelection::selected_text(window, cx), "alpha beta");
        });
    }

    #[gpui::test]
    fn group_pieces_select_and_copy_as_one_text(cx: &mut TestAppContext) {
        let (_, _, cx) = impulse_view(cx);
        drag(cx, (1., 12.), (390., 12.));
        cx.update(|window, cx| {
            assert_eq!(TextSelection::selected_text(window, cx), "alpha beta gamma");
        });
    }

    #[gpui::test]
    fn selection_across_runs_copies_them_a_line_apart(cx: &mut TestAppContext) {
        let (_, _, cx) = impulse_view(cx);
        drag(cx, (1., 12.), (390., 44.));
        cx.update(|window, cx| {
            assert_eq!(
                TextSelection::selected_text(window, cx),
                "alpha beta gamma\ndocs"
            );
        });
    }

    #[gpui::test]
    fn document_select_all_selects_every_run_until_a_press(cx: &mut TestAppContext) {
        let (document, _, cx) = impulse_view(cx);
        cx.update(|window, cx| {
            document.select_all(window, cx);
            let _ = window.draw(cx);
            assert!(document.is_all_selected());
            assert_eq!(
                TextSelection::selected_text(window, cx),
                "alpha beta gamma\ndocs"
            );
        });
        drag(cx, (380., 80.), (380., 80.));
        cx.update(|window, cx| {
            assert!(!document.is_all_selected());
            assert_eq!(TextSelection::selected_text(window, cx), "");
        });
    }

    #[gpui::test]
    fn link_click_follows_and_drag_selects(cx: &mut TestAppContext) {
        let (_, clicks, cx) = impulse_view(cx);
        drag(cx, (2., 44.), (3., 44.));
        assert_eq!(clicks.get(), 1);
        cx.update(|window, cx| assert!(!TextSelection::has_selection(window, cx)));

        drag(cx, (2., 44.), (300., 44.));
        assert_eq!(clicks.get(), 1);
        cx.update(|window, cx| {
            assert_eq!(TextSelection::selected_text(window, cx), "docs");
        });
    }

    /// Impulse: a selectable text in a cached view, counting its renders.
    struct CachedText(Rc<Cell<usize>>);

    impl Render for CachedText {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.0.set(self.0.get() + 1);
            div()
                .w(px(240.))
                .h(px(32.))
                .child(SelectableText::new("cached", "alpha beta"))
        }
    }

    struct CachedHost(gpui::Entity<CachedText>);

    impl Render for CachedHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(TextSelectionLayer)
                .child(self.0.clone().cached(gpui::StyleRefinement::default()))
        }
    }

    #[gpui::test]
    fn selection_change_redraws_a_cached_view(cx: &mut TestAppContext) {
        let renders = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view({
            let renders = renders.clone();
            move |_, cx| CachedHost(cx.new(|_| CachedText(renders)))
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let before = renders.get();
        drag(cx, (1., 12.), (220., 12.));
        assert!(renders.get() > before);
        cx.update(|window, cx| {
            assert_eq!(TextSelection::selected_text(window, cx), "alpha beta");
        });
    }

    #[test]
    fn click_allows_a_few_pixels_of_movement() {
        assert!(super::is_click(
            point(px(10.), px(10.)),
            point(px(13.), px(7.))
        ));
        assert!(!super::is_click(
            point(px(10.), px(10.)),
            point(px(20.), px(10.))
        ));
    }

    #[test]
    fn wrapped_selection_paints_full_width_middle_lines() {
        let bounds = Bounds::new(point(px(10.), px(20.)), size(px(100.), px(100.)));
        let quads = super::selection_quad_bounds(
            point(px(40.), px(20.)),
            point(px(30.), px(80.)),
            bounds,
            px(20.),
        );

        assert_eq!(
            quads,
            vec![
                Bounds::from_corners(point(px(40.), px(20.)), point(px(110.), px(40.))),
                Bounds::from_corners(point(px(10.), px(40.)), point(px(110.), px(80.))),
                Bounds::from_corners(point(px(10.), px(80.)), point(px(30.), px(100.))),
            ]
        );
    }
}
