//! P-SLOT-REGION: after every wheel step, with no full redraw in between, a
//! hosted view placed from its region (as the platform places it: the
//! region's container chain at the offsets the frame shows, the slot at the
//! content position its last paint staged) stands exactly on the row the
//! frame draws, for a scroll layer replaying its steps, nested layers, a
//! cached view inside a layer and a plain scroller that draws again, paused
//! or while another view draws every frame (playback).
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyView, AppContext, Bounds, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    Pixels, Point, Render, ScrollDelta, ScrollHandle, ScrollWheelEvent, SlotRegion,
    StatefulInteractiveElement, StyleRefinement, Styled, TestAppContext, VisualTestContext, Window,
    canvas, div, point, px, rgb, size,
};
use std::{cell::RefCell, rc::Rc};

/// The marker row's width: its quad is found by it in the drawn scene.
const MARK: f32 = 37.;

#[derive(Default)]
struct Staged {
    /// The region and window bounds of the slot's last paint.
    at: Option<(Option<SlotRegion>, Bounds<Pixels>)>,
    paints: usize,
}

/// The row a slot stands on: a quad of width `MARK` and a canvas that
/// stages the slot where it paints.
fn marked_row(staged: Rc<RefCell<Staged>>) -> impl IntoElement {
    div().flex_none().h(px(40.)).w_full().child(
        div().w(px(MARK)).h(px(20.)).bg(rgb(0xff0000)).child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let mut s = staged.borrow_mut();
                    s.at = Some((window.slot_region(), bounds));
                    s.paints += 1;
                },
            )
            .size_full(),
        ),
    )
}

fn rows(n: usize) -> Vec<impl IntoElement> {
    (0..n)
        .map(|_| div().flex_none().h(px(40.)).w_full().bg(rgb(0x202020)))
        .collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    /// One scroll layer replaying its steps.
    Layer,
    /// A plain scroller: each step draws the view again.
    Plain,
    /// A layer inside a layer; the marker in the inner one.
    Nested,
    /// The marker in a cached view inside a layer.
    CachedInLayer,
}

struct Marker(Rc<RefCell<Staged>>);
impl Render for Marker {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().flex().flex_col().child(marked_row(self.0.clone()))
    }
}

struct Panel {
    shape: Shape,
    outer: ScrollHandle,
    inner: ScrollHandle,
    staged: Rc<RefCell<Staged>>,
    marker: Entity<Marker>,
}
impl Render for Panel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let column = |id: &'static str, h: f32| {
            div()
                .id(id)
                .flex()
                .flex_col()
                .w(px(100.))
                .h(px(h))
                .overflow_y_scroll()
        };
        let outer = column("outer", 200.).track_scroll(&self.outer);
        let outer = match self.shape {
            Shape::Plain => outer,
            _ => outer.scroll_layer(),
        };
        match self.shape {
            Shape::Layer | Shape::Plain => outer
                .children(rows(3))
                .child(marked_row(self.staged.clone()))
                .children(rows(20)),
            Shape::CachedInLayer => outer
                .children(rows(3))
                .child(
                    self.marker
                        .clone()
                        .cached(StyleRefinement::default().w_full().h(px(40.))),
                )
                .children(rows(20)),
            Shape::Nested => outer
                .children(rows(2))
                .child(
                    column("inner", 120.)
                        .flex_none()
                        .track_scroll(&self.inner)
                        .scroll_layer()
                        .children(rows(2))
                        .child(marked_row(self.staged.clone()))
                        .children(rows(12)),
                )
                .children(rows(20)),
        }
    }
}

/// Another view that draws every frame, as the viewer does while playing.
struct Ticker(u64);
impl Render for Ticker {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(10.)).h(px(10.)).child(format!("{}", self.0))
    }
}

struct Root {
    panel: AnyView,
    ticker: Entity<Ticker>,
}
impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .child(
                div().w(px(100.)).h(px(200.)).child(
                    self.panel
                        .clone()
                        .cached(StyleRefinement::default().size_full()),
                ),
            )
            .child(self.ticker.clone())
    }
}

/// Where the platform puts the slot, in window points: its staged content
/// position through its region's container chain as the last draw placed it.
fn placed(window: &Window, staged: &Staged) -> Point<Pixels> {
    let (region, bounds) = staged.at.expect("the slot was painted");
    let Some(region) = region else {
        return bounds.origin;
    };
    let regions = window.slot_region_geometry();
    let find = |id| {
        *regions
            .iter()
            .find(|r| r.id == id)
            .expect("the region is placed")
    };
    // The content position the slot staged at its paint.
    let mut at = bounds.origin - region.content_origin;
    let mut r = find(region.id);
    loop {
        at = at + r.frame.origin + r.origin;
        match r.parent {
            Some(parent) => r = find(parent),
            None => return at,
        }
    }
}

/// Where the frame draws the marker row.
fn drawn(window: &Window) -> Point<Pixels> {
    let scale = window.scale_factor();
    let quads: Vec<_> = window
        .painted_quads()
        .into_iter()
        .filter(|q| (q.bounds.size.width.0 / scale - MARK).abs() < 0.01)
        .collect();
    assert_eq!(quads.len(), 1, "one marker row in the frame");
    let b = quads[0].bounds;
    point(px(b.origin.x.0 / scale), px(b.origin.y.0 / scale))
}

fn run(cx: &mut TestAppContext, shape: Shape, playing: bool) {
    let staged = Rc::new(RefCell::new(Staged::default()));
    let (outer, inner) = (ScrollHandle::new(), ScrollHandle::new());
    let (root, vcx) = cx.add_window_view(|_, cx| {
        let marker = cx.new(|_| Marker(staged.clone()));
        let panel = cx.new(|_| Panel {
            shape,
            outer: outer.clone(),
            inner: inner.clone(),
            staged: staged.clone(),
            marker,
        });
        Root {
            panel: panel.into(),
            ticker: cx.new(|_| Ticker(0)),
        }
    });
    let mut vcx: VisualTestContext = vcx.clone();
    vcx.run_until_parked();
    let check = |vcx: &mut VisualTestContext, what: &str| {
        let s = staged.borrow();
        vcx.update(|window, _| {
            assert_eq!(
                placed(window, &s),
                drawn(window),
                "{what}: the slot is off its row"
            );
        });
    };
    check(&mut vcx, "at rest");
    let ticker = root.read_with(&vcx, |r, _| r.ticker.clone());
    // Over the outer column's top rows, or over the inner column.
    let over = |inner: bool| {
        if inner {
            point(px(50.), px(100.))
        } else {
            point(px(50.), px(20.))
        }
    };
    let mut steps = vec![(false, -12.), (false, -30.), (false, 17.), (false, -25.)];
    if shape == Shape::Nested {
        steps.extend([(true, -15.), (true, -20.), (false, -10.), (true, 9.)]);
    }
    for (i, (inner_step, dy)) in steps.into_iter().enumerate() {
        let before = drawn_now(&mut vcx);
        let paints = staged.borrow().paints;
        vcx.simulate_event(ScrollWheelEvent {
            position: over(inner_step),
            delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
            ..Default::default()
        });
        if playing {
            ticker.update(&mut vcx, |t, cx| {
                t.0 += 1;
                cx.notify();
            });
        }
        vcx.run_until_parked();
        let what = format!(
            "step {i} ({dy} on the {} column)",
            if inner_step { "inner" } else { "outer" }
        );
        // The step showed: the row moved by it.
        assert_eq!(
            drawn_now(&mut vcx).y - before.y,
            px(dy),
            "{what}: the row did not move"
        );
        if matches!(shape, Shape::Layer | Shape::CachedInLayer) && !inner_step {
            assert_eq!(
                staged.borrow().paints,
                paints,
                "{what}: replayed, not painted again"
            );
        }
        check(&mut vcx, &what);
    }
    // The steps stop: the view draws in full once, the slot where it was.
    vcx.executor()
        .advance_clock(std::time::Duration::from_millis(400));
    vcx.run_until_parked();
    check(&mut vcx, "settled");
}

fn drawn_now(vcx: &mut VisualTestContext) -> Point<Pixels> {
    vcx.update(|window, _| drawn(window))
}

#[gpui::test]
fn a_slot_follows_a_replayed_scroll_layer(cx: &mut TestAppContext) {
    run(cx, Shape::Layer, false);
}
#[gpui::test]
fn a_slot_follows_a_replayed_scroll_layer_while_playing(cx: &mut TestAppContext) {
    run(cx, Shape::Layer, true);
}
#[gpui::test]
fn a_slot_follows_a_scroller_that_draws_again(cx: &mut TestAppContext) {
    run(cx, Shape::Plain, false);
}
#[gpui::test]
fn a_slot_follows_a_scroller_that_draws_again_while_playing(cx: &mut TestAppContext) {
    run(cx, Shape::Plain, true);
}
/// Fails without the nested-layer refusal: an inner layer's offset-only step
/// replayed the outer layer as recorded, and nothing moved.
#[gpui::test]
fn a_slot_follows_nested_scroll_layers(cx: &mut TestAppContext) {
    run(cx, Shape::Nested, false);
}
#[gpui::test]
fn a_slot_follows_nested_scroll_layers_while_playing(cx: &mut TestAppContext) {
    run(cx, Shape::Nested, true);
}
#[gpui::test]
fn a_slot_in_a_cached_view_follows_its_scroll_layer(cx: &mut TestAppContext) {
    run(cx, Shape::CachedInLayer, false);
}
#[gpui::test]
fn a_slot_in_a_cached_view_follows_its_scroll_layer_while_playing(cx: &mut TestAppContext) {
    run(cx, Shape::CachedInLayer, true);
}

/// PM-696 review: a region nested behind a view that clips (a card with
/// overflow hidden between the outer scroller and the inner one) gets that
/// clip on its container. A row inside the inner scroller but below the card
/// is clipped away by GPUI and must not show natively either; scrolled into
/// the card, it shows.
struct CardPanel {
    outer: ScrollHandle,
    inner: ScrollHandle,
    staged: Rc<RefCell<Staged>>,
}
impl Render for CardPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("outer")
            .flex()
            .flex_col()
            .w(px(100.))
            .h(px(200.))
            .overflow_y_scroll()
            .track_scroll(&self.outer)
            .scroll_layer()
            .children(rows(1))
            .child(
                div().flex_none().w_full().h(px(40.)).overflow_hidden().child(
                    div()
                        .id("inner")
                        .flex()
                        .flex_col()
                        .w_full()
                        .h(px(120.))
                        .overflow_y_scroll()
                        .track_scroll(&self.inner)
                        .children(rows(1))
                        .child(marked_row(self.staged.clone()))
                        .children(rows(4)),
                ),
            )
            .children(rows(20))
    }
}

#[gpui::test]
fn a_nested_region_keeps_the_clip_of_a_view_between_it_and_its_parent(cx: &mut TestAppContext) {
    let staged = Rc::new(RefCell::new(Staged::default()));
    let (outer, inner) = (ScrollHandle::new(), ScrollHandle::new());
    let (_, vcx) = cx.add_window_view(|_, _| CardPanel {
        outer: outer.clone(),
        inner: inner.clone(),
        staged: staged.clone(),
    });
    let mut vcx: VisualTestContext = vcx.clone();
    vcx.run_until_parked();
    // Whether the platform shows the slot: its staged content rect against its
    // region's view, cut by the regions around it.
    let shows = |vcx: &mut VisualTestContext| {
        let s = staged.borrow();
        let (region, bounds) = s.at.expect("painted");
        let region = region.expect("in a region");
        vcx.update(|window, _| {
            let regions = window.slot_region_geometry();
            let find = |id| *regions.iter().find(|r| r.id == id).expect("placed");
            // The view of a region in its own content, then of its parent.
            let mut view = {
                let r = find(region.id);
                Bounds::new(-r.origin, r.frame.size)
            };
            let mut r = find(region.id);
            while let Some(parent) = r.parent {
                let p = find(parent);
                let pv = Bounds::new(-p.origin, p.frame.size);
                let pv = Bounds::new(pv.origin - r.frame.origin - r.origin, pv.size);
                view = view.intersect(&pv);
                r = p;
            }
            let rect = Bounds::new(bounds.origin - region.content_origin, bounds.size);
            let cut = view.intersect(&rect);
            (cut.size.height > px(0.), find(region.id).frame.size.height)
        })
    };
    // The row sits at 40..60 of the inner content; the card shows 0..40 of it.
    assert_eq!(shows(&mut vcx), (false, px(40.)), "below the card: not shown");
    // A step of the outer scroller (replayed) changes nothing of that.
    vcx.simulate_event(ScrollWheelEvent {
        position: point(px(50.), px(10.)),
        delta: ScrollDelta::Pixels(point(px(0.), px(-10.))),
        ..Default::default()
    });
    vcx.run_until_parked();
    assert_eq!(shows(&mut vcx), (false, px(40.)), "after an outer step");
    // The inner scroller brings the row into the card.
    vcx.simulate_event(ScrollWheelEvent {
        position: point(px(50.), px(45.)),
        delta: ScrollDelta::Pixels(point(px(0.), px(-30.))),
        ..Default::default()
    });
    vcx.run_until_parked();
    assert_eq!(shows(&mut vcx).0, true, "scrolled into the card: shown");
}

/// PM-696 review: a region exists for a frame only while the frame draws it.
/// A panel taken out of the window keeps its scroll handle (and its slot's
/// staging) alive, but its region is no longer published, so a kept slot
/// cannot show there; put back, the region returns, once, in the window that
/// draws it.
struct Shelf {
    shown: bool,
    panel: Entity<Panel>,
}
impl Render for Shelf {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let shelf = div().size_full();
        if self.shown {
            shelf.child(self.panel.clone())
        } else {
            shelf
        }
    }
}

#[gpui::test]
fn a_region_the_frame_no_longer_draws_is_not_published(cx: &mut TestAppContext) {
    let staged = Rc::new(RefCell::new(Staged::default()));
    let outer = ScrollHandle::new();
    let (shelf, vcx) = cx.add_window_view(|_, cx| {
        let marker = cx.new(|_| Marker(staged.clone()));
        let panel = cx.new(|_| Panel {
            shape: Shape::Layer,
            outer: outer.clone(),
            inner: ScrollHandle::new(),
            staged: staged.clone(),
            marker,
        });
        Shelf { shown: true, panel }
    });
    let mut vcx: VisualTestContext = vcx.clone();
    vcx.run_until_parked();
    let region = staged.borrow().at.and_then(|(r, _)| r).expect("in a region").id;
    let published = |vcx: &mut VisualTestContext| {
        vcx.update(|window, _| window.slot_region_geometry().iter().filter(|r| r.id == region).count())
    };
    assert_eq!(published(&mut vcx), 1);
    // Taken out; the handle and the staging stay alive.
    shelf.update(&mut vcx, |s, cx| {
        s.shown = false;
        cx.notify();
    });
    vcx.run_until_parked();
    assert_eq!(published(&mut vcx), 0, "a region no frame draws is not published");
    // Put back.
    shelf.update(&mut vcx, |s, cx| {
        s.shown = true;
        cx.notify();
    });
    vcx.run_until_parked();
    assert_eq!(published(&mut vcx), 1, "back once");
}

/// A scroll layer that fades its content at the edges it can still scroll
/// past (a dock panel's body).
struct Fading {
    handle: ScrollHandle,
    staged: Rc<RefCell<Staged>>,
}
impl Render for Fading {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("fading")
            .flex()
            .flex_col()
            .w(px(100.))
            .h(px(200.))
            .overflow_y_scroll()
            .track_scroll(&self.handle)
            .scroll_edge_fade_y(px(FADE))
            .scroll_layer()
            .children(rows(3))
            .child(marked_row(self.staged.clone()))
            .children(rows(20))
    }
}
const FADE: f32 = 16.;

/// The marker row's edge fades (top, bottom) in points, and the region's.
fn fades(vcx: &mut VisualTestContext) -> ([f32; 2], [f32; 2]) {
    vcx.update(|window, _| {
        let scale = window.scale_factor();
        let quad = window
            .painted_quads()
            .into_iter()
            .find(|q| (q.bounds.size.width.0 / scale - MARK).abs() < 0.01)
            .expect("the marker row is drawn");
        let regions = window.slot_region_geometry();
        assert_eq!(regions.len(), 1, "one region");
        (
            [
                quad.content_mask.fade_out.top.0 / scale,
                quad.content_mask.fade_out.bottom.0 / scale,
            ],
            [f32::from(regions[0].fade[0]), f32::from(regions[0].fade[1])],
        )
    })
}

/// PM-710: a wheel step that leaves the top edge (or comes back to it)
/// changes where the body fades. It replays the layer with the fades moved,
/// as a full draw places them, instead of drawing the view again (which
/// redrew a dock panel's whole body on the first step of every scroll down
/// from the top and on the step that reached it again).
#[gpui::test]
fn a_scroll_layer_moves_its_edge_fades_without_drawing_again(cx: &mut TestAppContext) {
    let staged = Rc::new(RefCell::new(Staged::default()));
    let handle = ScrollHandle::new();
    let (_, vcx) = cx.add_window_view(|_, cx| {
        let panel = cx.new(|_| Fading {
            handle: handle.clone(),
            staged: staged.clone(),
        });
        Root {
            panel: panel.into(),
            ticker: cx.new(|_| Ticker(0)),
        }
    });
    let mut vcx: VisualTestContext = vcx.clone();
    vcx.run_until_parked();
    // A second frame records the layer (the first may draw it plainly).
    vcx.update(|window, cx| window.simulate_next_frame(cx));
    vcx.run_until_parked();
    let at_top = fades(&mut vcx);
    assert_eq!(
        at_top,
        ([0., FADE], [0., FADE]),
        "at the top only the bottom fades"
    );
    let wheel = |vcx: &mut VisualTestContext, dy: f32| {
        vcx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(20.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
            ..Default::default()
        });
        vcx.run_until_parked();
    };
    let paints = staged.borrow().paints;
    wheel(&mut vcx, -12.);
    assert_eq!(
        staged.borrow().paints,
        paints,
        "leaving the top: replayed, not painted again"
    );
    let scrolled = fades(&mut vcx);
    assert_eq!(
        scrolled,
        ([FADE, FADE], [FADE, FADE]),
        "both edges fade once scrolled"
    );
    wheel(&mut vcx, 12.);
    assert_eq!(staged.borrow().paints, paints, "back at the top: replayed");
    assert_eq!(fades(&mut vcx), at_top, "the top fade goes again");
    // Scrolled and settled, a full draw shows the same fades as the replay.
    wheel(&mut vcx, -12.);
    vcx.executor()
        .advance_clock(std::time::Duration::from_millis(400));
    vcx.run_until_parked();
    assert!(staged.borrow().paints > paints, "settled: drawn in full");
    assert_eq!(
        fades(&mut vcx),
        scrolled,
        "the full draw fades as the replay did"
    );
}

/// A panel's own scroller inside a body that fits its slot: the body is a
/// scroll layer with nothing to scroll (a dock panel's body around the
/// Project panel).
struct FitsBody {
    body: ScrollHandle,
    list: ScrollHandle,
    staged: Rc<RefCell<Staged>>,
}
impl Render for FitsBody {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("body")
            .flex()
            .flex_col()
            .w(px(100.))
            .h(px(200.))
            .overflow_y_scroll()
            .track_scroll(&self.body)
            .scroll_layer()
            .child(
                div()
                    .id("list")
                    .flex()
                    .flex_col()
                    .flex_none()
                    .w_full()
                    .h(px(160.))
                    .overflow_y_scroll()
                    .track_scroll(&self.list)
                    .scroll_layer()
                    .children(rows(2))
                    .child(marked_row(self.staged.clone()))
                    .children(rows(12)),
            )
    }
}

/// PM-710: the inner scroller of a body with nothing to scroll replays its
/// steps; it was a nested layer (recorded inside the body's recording),
/// whose every step drew the panel again (the Project panel's first wheel
/// step of each scroll).
#[gpui::test]
fn a_scroll_layer_inside_a_body_that_fits_replays_its_steps(cx: &mut TestAppContext) {
    let staged = Rc::new(RefCell::new(Staged::default()));
    let (body, list) = (ScrollHandle::new(), ScrollHandle::new());
    let (_, vcx) = cx.add_window_view(|_, cx| {
        let panel = cx.new(|_| FitsBody {
            body: body.clone(),
            list: list.clone(),
            staged: staged.clone(),
        });
        Root {
            panel: panel.into(),
            ticker: cx.new(|_| Ticker(0)),
        }
    });
    let mut vcx: VisualTestContext = vcx.clone();
    vcx.run_until_parked();
    vcx.update(|window, cx| window.simulate_next_frame(cx));
    vcx.run_until_parked();
    for dy in [-15., -20., 9.] {
        let (before, paints) = (drawn_now(&mut vcx), staged.borrow().paints);
        vcx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(60.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
            ..Default::default()
        });
        vcx.run_until_parked();
        assert_eq!(
            drawn_now(&mut vcx).y - before.y,
            px(dy),
            "the row moved by the step"
        );
        assert_eq!(
            staged.borrow().paints,
            paints,
            "{dy}: replayed, not painted again"
        );
    }
}

/// The thumbnail's quad: found by its width in the drawn scene.
const THUMB: f32 = 41.;

/// A scroll layer of rows whose thumbnails clip to rounded corners (the
/// Project grid's), in a viewport that may clip to rounded corners itself.
struct Thumbs {
    handle: ScrollHandle,
    rounded_viewport: bool,
    /// False: a plain scroller, drawn in full at every offset (the reference).
    layer: bool,
}
impl Render for Thumbs {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("thumbs")
            .flex()
            .flex_col()
            .w(px(100.))
            .h(px(200.))
            // Rounded corners clip only where both axes do.
            .when(self.rounded_viewport, |d| {
                d.rounded(px(12.)).overflow_x_hidden()
            })
            .overflow_y_scroll()
            .track_scroll(&self.handle)
            .when(self.layer, |d| d.scroll_layer())
            .children(rows(3))
            .child(
                div().flex_none().h(px(40.)).w_full().child(
                    div()
                        .w(px(30.))
                        .h(px(30.))
                        .rounded(px(8.))
                        .overflow_hidden()
                        .child(div().w(px(THUMB)).h(px(30.)).bg(rgb(0x00ff00))),
                ),
            )
            .children(rows(20))
    }
}

/// The thumbnail as drawn: its bounds and its clip (rectangle and rounded
/// shape), in device pixels.
fn thumb(
    vcx: &mut VisualTestContext,
) -> (
    Bounds<gpui::ScaledPixels>,
    gpui::ContentMask<gpui::ScaledPixels>,
) {
    vcx.update(|window, _| {
        let scale = window.scale_factor();
        let quad = window
            .painted_quads()
            .into_iter()
            .find(|q| (q.bounds.size.width.0 / scale - THUMB).abs() < 0.01)
            .expect("the thumbnail is drawn");
        (quad.bounds, quad.content_mask)
    })
}

/// The same rows in a window of their own, drawn at `offset` as a plain
/// scroller (no layer): the picture a replay must reproduce.
fn plain_draw(
    cx: &mut TestAppContext,
    rounded_viewport: bool,
    offset: Point<Pixels>,
) -> (
    Bounds<gpui::ScaledPixels>,
    gpui::ContentMask<gpui::ScaledPixels>,
) {
    let handle = ScrollHandle::new();
    let (_, vcx) = cx.add_window_view(|_, cx| {
        let panel = cx.new(|_| Thumbs {
            handle: handle.clone(),
            rounded_viewport,
            layer: false,
        });
        Root {
            panel: panel.into(),
            ticker: cx.new(|_| Ticker(0)),
        }
    });
    let mut vcx: VisualTestContext = vcx.clone();
    vcx.run_until_parked();
    handle.set_offset(offset);
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    assert_eq!(handle.offset(), offset);
    thumb(&mut vcx)
}

/// PM-710 review: a replayed step draws a thumbnail with rounded corners
/// exactly as a plain draw at the same offset draws it, at a 2x window:
/// - a whole step: its rounded shape moves with it (it stayed where it was
///   recorded, and the thumbnail was sliced or vanished);
/// - a step of a fraction of a device pixel (-0.3 pt is -0.6 device pixels,
///   which a plain draw snaps): the replay moves by whole device pixels;
/// - half outside a viewport that clips to rounded corners itself: it keeps
///   its own corners, as a plain draw chooses.
/// A layer's own full draw (after the steps stop) draws so too.
#[gpui::test]
fn a_replayed_step_moves_rounded_clips_with_the_content(cx: &mut TestAppContext) {
    for (rounded_viewport, dy, what) in [
        (false, -30., "a whole step"),
        (false, -0.3, "a fractional step"),
        (true, -130., "half outside a rounded viewport"),
    ] {
        let handle = ScrollHandle::new();
        let (_, vcx) = cx.add_window_view(|_, cx| {
            let panel = cx.new(|_| Thumbs {
                handle: handle.clone(),
                rounded_viewport,
                layer: true,
            });
            Root {
                panel: panel.into(),
                ticker: cx.new(|_| Ticker(0)),
            }
        });
        let mut vcx: VisualTestContext = vcx.clone();
        assert_eq!(vcx.update(|window, _| window.scale_factor()), 2.);
        vcx.run_until_parked();
        vcx.update(|window, cx| window.simulate_next_frame(cx));
        vcx.run_until_parked();
        let (before, mask) = thumb(&mut vcx);
        assert!(
            mask.corner_radii.top_left.0 > 0.,
            "{what}: the thumbnail clips to rounded corners"
        );
        vcx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(20.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
            ..Default::default()
        });
        vcx.run_until_parked();
        let replayed = thumb(&mut vcx);
        assert!(
            replayed.0.origin.y < before.origin.y,
            "{what}: the step moved the thumbnail"
        );
        let reference = plain_draw(cx, rounded_viewport, handle.offset());
        assert_eq!(
            replayed, reference,
            "{what}: replayed as a plain draw draws it"
        );
        // The steps stop: the layer draws in full at the same offset.
        vcx.executor()
            .advance_clock(std::time::Duration::from_millis(400));
        vcx.run_until_parked();
        assert_eq!(
            thumb(&mut vcx),
            reference,
            "{what}: the layer's full draw as a plain draw"
        );
    }
}

/// A driven scroll layer (the timeline's rows) whose content fits.
struct DrivenFits {
    handle: ScrollHandle,
    staged: Rc<RefCell<Staged>>,
}
impl Render for DrivenFits {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("driven")
            .flex()
            .flex_col()
            .w(px(100.))
            .h(px(200.))
            .overflow_y_scroll()
            .track_scroll(&self.handle)
            .scroll_layer_driven()
            .children(rows(1))
            .child(marked_row(self.staged.clone()))
    }
}

/// PM-710 review: a driven layer is kept whatever its extent; its owner
/// steps it on every wheel event, and a step it can no longer replay drew
/// the owner's view again each time (the timeline with a few tracks).
#[gpui::test]
fn a_driven_layer_with_nothing_to_scroll_still_replays_its_owners_steps(cx: &mut TestAppContext) {
    let staged = Rc::new(RefCell::new(Staged::default()));
    let handle = ScrollHandle::new();
    let (_, vcx) = cx.add_window_view(|_, cx| {
        let panel = cx.new(|_| DrivenFits {
            handle: handle.clone(),
            staged: staged.clone(),
        });
        Root {
            panel: panel.into(),
            ticker: cx.new(|_| Ticker(0)),
        }
    });
    let mut vcx: VisualTestContext = vcx.clone();
    vcx.run_until_parked();
    vcx.update(|window, cx| window.simulate_next_frame(cx));
    vcx.run_until_parked();
    let paints = staged.borrow().paints;
    let stepped =
        vcx.update(|window, cx| window.step_scroll_layer(&handle, point(px(0.), px(0.)), cx));
    assert!(stepped, "the owner's step replays");
    vcx.run_until_parked();
    assert_eq!(staged.borrow().paints, paints, "not painted again");
}

/// A Project grid of missing-media tiles: each a rounded tile that clips,
/// grey, with its red-brown tint over it (an absolutely placed fill drawn
/// at an opacity).
struct Tiles {
    handle: ScrollHandle,
    layer: bool,
}
impl Render for Tiles {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let tile = |i: usize| {
            div().flex_none().h(px(70.)).w_full().p(px(5.)).child(
                div()
                    .id(("tile", i))
                    .relative()
                    .overflow_hidden()
                    .rounded(px(8.))
                    .w(px(80.))
                    .h(px(60.))
                    .bg(rgb(0x2a2a2a))
                    .child(div().absolute().inset_0().bg(rgb(0x5a2a20)).opacity(0.6)),
            )
        };
        div()
            .id("tiles")
            .flex()
            .flex_col()
            .w(px(100.))
            .h(px(200.))
            .overflow_y_scroll()
            .track_scroll(&self.handle)
            .when(self.layer, |d| d.scroll_layer())
            .children((0..20).map(tile))
    }
}

/// Every quad drawn, in device pixels: bounds, clip and colour.
fn scene(vcx: &mut VisualTestContext) -> Vec<String> {
    vcx.update(|window, _| {
        let mut quads: Vec<String> = window
            .painted_quads()
            .into_iter()
            .map(|q| {
                // A clip's shape matters only where it rounds corners.
                let m = &q.content_mask;
                let rounded = m.corner_radii.top_left.0 > 0.
                    || m.corner_radii.top_right.0 > 0.
                    || m.corner_radii.bottom_right.0 > 0.
                    || m.corner_radii.bottom_left.0 > 0.;
                let shape = rounded.then(|| format!("{:?} {:?}", m.shape, m.corner_radii));
                format!(
                    "{:?} clip {:?} fade {:?} {shape:?} radii {:?} {:?}",
                    q.bounds, m.bounds, m.fade_out, q.corner_radii, q.background
                )
            })
            .collect();
        quads.extend(window.painted_filter_boundaries().into_iter().map(|b| {
            format!(
                "group {:?} clip {:?} opacity {} start {}",
                b.bounds, b.content_mask.bounds, b.opacity, b.is_start
            )
        }));
        quads.sort();
        quads
    })
}

/// The tiles in a window of their own, drawn at `offset` as a plain
/// scroller: the picture each replayed step must reproduce.
fn plain_tiles(cx: &mut TestAppContext, offset: Point<Pixels>) -> Vec<String> {
    let handle = ScrollHandle::new();
    let (_, vcx) = cx.add_window_view(|_, cx| {
        let panel = cx.new(|_| Tiles {
            handle: handle.clone(),
            layer: false,
        });
        Root {
            panel: panel.into(),
            ticker: cx.new(|_| Ticker(0)),
        }
    });
    let mut vcx: VisualTestContext = vcx.clone();
    vcx.run_until_parked();
    handle.set_offset(offset);
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    scene(&mut vcx)
}

/// PM-710 on screen: an upward flick over the Project grid's missing-media
/// tiles replays each step exactly as a plain draw at its offset draws the
/// tiles, their tint included (it was drawn about 40 pt off its tile).
#[gpui::test]
fn an_upward_flick_replays_missing_media_tiles_as_drawn(cx: &mut TestAppContext) {
    let handle = ScrollHandle::new();
    let (_, vcx) = cx.add_window_view(|_, cx| {
        let panel = cx.new(|_| Tiles {
            handle: handle.clone(),
            layer: true,
        });
        Root {
            panel: panel.into(),
            ticker: cx.new(|_| Ticker(0)),
        }
    });
    let mut vcx: VisualTestContext = vcx.clone();
    vcx.run_until_parked();
    let wheel = |vcx: &mut VisualTestContext, dy: f32| {
        vcx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
            ..Default::default()
        });
        vcx.run_until_parked();
    };
    // Down the grid, then settled: drawn in full there.
    wheel(&mut vcx, -150.);
    vcx.executor()
        .advance_clock(std::time::Duration::from_millis(400));
    vcx.run_until_parked();
    for dy in [25.5, 18.3, 12.1, 7.7, 3.2, 1.4] {
        wheel(&mut vcx, dy);
        let offset = handle.offset();
        let (replayed, plain) = (scene(&mut vcx), plain_tiles(cx, offset));
        let only = |a: &Vec<String>, b: &Vec<String>| -> Vec<String> {
            a.iter().filter(|q| !b.contains(q)).cloned().collect()
        };
        assert_eq!(
            (only(&replayed, &plain), only(&plain, &replayed)),
            (vec![], vec![]),
            "a step of {dy} up, at {offset:?}: replayed as a plain draw draws it"
        );
    }
}

/// Rows reporting the pointer entering them, in a scroll layer its owner
/// steps (driven, as the timeline's) or the wheel steps.
struct HoverRows {
    handle: ScrollHandle,
    driven: bool,
    entered: Rc<RefCell<Vec<usize>>>,
}
impl Render for HoverRows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let rows = (0..20).map(|i| {
            let entered = self.entered.clone();
            div()
                .id(("hover-row", i))
                .flex_none()
                .h(px(20.))
                .w_full()
                .on_hover(move |hovered, _, _| {
                    if *hovered {
                        entered.borrow_mut().push(i);
                    }
                })
        });
        let list = div()
            .id("hover-rows")
            .flex()
            .flex_col()
            .w(px(100.))
            .h(px(200.))
            .overflow_y_scroll()
            .track_scroll(&self.handle);
        if self.driven {
            list.scroll_layer_driven().children(rows)
        } else {
            list.scroll_layer().children(rows)
        }
    }
}

/// PM-710 on screen: a wheel-stepped layer's rows take the hover at once as
/// they move under a still pointer (it was held until the layer settled, and
/// a stop showed no hover); a driven layer's (the timeline's) keep waiting
/// for the settle, as their owner draws its hover then.
#[gpui::test]
fn the_hover_follows_a_wheeled_layer_at_once_and_a_driven_one_once_settled(
    cx: &mut TestAppContext,
) {
    for driven in [false, true] {
        let entered = Rc::new(RefCell::new(Vec::new()));
        let handle = ScrollHandle::new();
        let (_, vcx) = cx.add_window_view(|_, cx| {
            let panel = cx.new(|_| HoverRows {
                handle: handle.clone(),
                driven,
                entered: entered.clone(),
            });
            Root {
                panel: panel.into(),
                ticker: cx.new(|_| Ticker(0)),
            }
        });
        let mut vcx: VisualTestContext = vcx.clone();
        vcx.run_until_parked();
        let at = point(px(50.), px(50.));
        vcx.simulate_mouse_move(at, None, gpui::Modifiers::none());
        vcx.update(|window, cx| window.simulate_next_frame(cx));
        vcx.run_until_parked();
        assert_eq!(entered.borrow().last(), Some(&2), "row 2 under the pointer");
        // The next row comes under the still pointer.
        if driven {
            let stepped = vcx.update(|window, cx| {
                window.step_scroll_layer(&handle, point(px(0.), px(-20.)), cx)
            });
            assert!(stepped);
        } else {
            vcx.simulate_event(ScrollWheelEvent {
                position: at,
                delta: ScrollDelta::Pixels(point(px(0.), px(-20.))),
                ..Default::default()
            });
        }
        vcx.update(|window, cx| window.simulate_next_frame(cx));
        vcx.run_until_parked();
        let now = entered.borrow().last().copied();
        if driven {
            assert_eq!(now, Some(2), "a driven layer's hover waits for its settle");
            vcx.executor()
                .advance_clock(std::time::Duration::from_millis(400));
            vcx.run_until_parked();
            assert_eq!(entered.borrow().last(), Some(&3), "settled: row 3 hovered");
        } else {
            assert_eq!(now, Some(3), "a wheeled layer's hover follows at once");
        }
    }
}
