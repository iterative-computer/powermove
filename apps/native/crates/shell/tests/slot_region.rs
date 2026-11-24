//! P-SLOT-REGION: after every wheel step, with no full redraw in between, a
//! hosted view placed from its region (as the platform places it: the
//! region's container chain at the offsets the frame shows, the slot at the
//! content position its last paint staged) stands exactly on the row the
//! frame draws, for a scroll layer replaying its steps, nested layers, a
//! cached view inside a layer and a plain scroller that draws again, paused
//! or while another view draws every frame (playback).
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
