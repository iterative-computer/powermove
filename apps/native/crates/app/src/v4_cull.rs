//! The rows of a long scrolling page that are nowhere near what shows are
//! not built: each stands in as an empty box of the height it had (Settings ›
//! Shortcuts lists 185 commands in about 2,600 nodes, of which some 25 rows
//! show). Every frame of a page builds, lays out and paints what it builds;
//! a hover's wash or a click redrew all of it at about 60 ms a frame.
//!
//! Only rows stacked in a vertical scroller are left out, and only those a
//! whole viewport away from it (above or below), so a scroll never shows an
//! empty box; scrolling redraws the page, which brings them back as they
//! come near. A row is measured the first time it is built (where its top
//! sits in the scroller's content, and its height); rows not measured yet
//! stand in at an estimate and are built a few a frame until all are, so a
//! row never changes height near what shows. A native control is made when
//! its row is first built. Everything is built while the keyboard is in the
//! scroller (Tab moves only through what is built) and while VoiceOver runs
//! (it reads only what is built).
use super::SurfaceView;
use crate::v4_control::Kind;
use gpui::{
    AnyElement, Display, FlexDirection, IntoElement, ParentElement, Position, ScrollHandle,
    Styled, Window, canvas, div, px,
};
use pm_surface::Id;
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
};

/// Panels with more nodes than this make a row's native controls when the
/// row is first built (Settings › Shortcuts has about 2,600; other pages
/// 100 to 300).
const LARGE: usize = 1000;

/// Rows not measured yet that a frame builds anyway, to measure them.
const MEASURE_PER_FRAME: usize = 4;

#[derive(Default)]
pub(super) struct Cull {
    /// Where each measured node sat in its scroller's content in the last
    /// frame drawn: its top and its height.
    extents: Rc<RefCell<HashMap<Id, (f32, f32)>>>,
    /// This frame: nodes left out, and the height each stands in at.
    pub(super) culled: HashMap<Id, f32>,
    /// This frame: scrollers with rows left out.
    culled_in: std::collections::HashSet<Id>,
    /// This frame: nodes whose place is recorded, and their scroller.
    measured: HashMap<Id, ScrollHandle>,
    /// Native controls not made yet: made when their row is first built.
    pub(super) deferred: HashMap<Id, Kind>,
    /// Some row stands in at an estimate: another frame measures more.
    pending: bool,
    /// The scroller of a body that scrolls around the panel (Settings in a
    /// panel), and its offset when the panel was last laid out: a wheel step
    /// moves the panel before it is drawn again, by the difference.
    outer: Option<(ScrollHandle, Rc<std::cell::Cell<f32>>)>,
}

impl Cull {
    /// Marks the offset of the body around the panel as the panel is laid
    /// out (with [`SurfaceView::scrolls_in`]).
    pub(super) fn outer_mark(&self) -> Option<AnyElement> {
        let (handle, then) = self.outer.clone()?;
        Some(
            canvas(
                move |_, _, _| then.set(clamped(&handle)),
                |_, _, _, _| {},
            )
            .absolute()
            .into_any_element(),
        )
    }
    /// Forgets nodes that are gone.
    pub(super) fn retain(&mut self, live: impl Fn(&Id) -> bool) {
        self.extents.borrow_mut().retain(|id, _| live(id));
        self.deferred.retain(|id, _| live(id));
    }
    /// A new tree: where rows were is the old one's, so tops are forgotten
    /// (NaN) and heights kept as guesses until the next layout records both.
    pub(super) fn forget_tops(&self) {
        for extent in self.extents.borrow_mut().values_mut() {
            extent.0 = f32::NAN;
        }
    }
    /// What stands in for a node left out: a box of its height, keeping its
    /// place recorded when it was measured.
    pub(super) fn stand_in(&self, id: Id) -> Option<AnyElement> {
        let height = *self.culled.get(&id)?;
        let mut stand_in = div().flex_none().w_full().h(px(height));
        if let Some(record) = self.record(id) {
            stand_in = stand_in.relative().child(record);
        }
        Some(stand_in.into_any_element())
    }
    /// Records where a built node sits, when it is one whose place is kept.
    pub(super) fn record(&self, id: Id) -> Option<AnyElement> {
        let handle = self.measured.get(&id)?.clone();
        let extents = self.extents.clone();
        let known = self.culled.get(&id).is_none() || extents.borrow().contains_key(&id);
        // A box standing in at an estimate is not a measurement.
        known.then(|| {
            canvas(
                move |bounds, _, _| {
                    let scroller = handle.bounds();
                    let top = f32::from(bounds.origin.y - scroller.origin.y - handle.offset().y);
                    extents
                        .borrow_mut()
                        .insert(id, (top, f32::from(bounds.size.height)));
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0()
            .into_any_element()
        })
    }
}

impl SurfaceView {
    /// Decides, before a frame is built, which rows of each vertical
    /// scroller it leaves out, and makes the native controls of the rows it
    /// builds for the first time.
    pub(super) fn plan_cull(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        self.cull.culled.clear();
        self.cull.culled_in.clear();
        self.cull.measured.clear();
        self.cull.pending = false;
        if !voiceover_running() {
            let focused = self
                .focus
                .iter()
                .find(|(_, handle)| handle.is_focused(window))
                .map(|(id, _)| *id);
            let extents = self.cull.extents.borrow().clone();
            let scrollers: Vec<(Id, ScrollHandle)> = self
                .prepared
                .nodes
                .values()
                .filter(|n| n.scroll_y && !n.hidden)
                .filter_map(|n| self.scrolls.get(&n.id).map(|h| (n.id, h.clone())))
                .collect();
            let window_height = f32::from(window.viewport_size().height);
            for (scroller, handle) in scrollers {
                let bounds = handle.bounds();
                if bounds.size.height <= px(0.) || focused.is_some_and(|f| self.inside(f, scroller)) {
                    continue;
                }
                // What shows of the scroller's content: its part inside the
                // window. A panel whose body scrolls around it (Settings in
                // a panel) moves the scroller itself; its own offset moves
                // its content.
                // Where the scroller is now: where the last layout left it,
                // moved by the steps of a body around the panel since.
                // Offsets as the layout will clamp them: a step can leave one
                // past its end (or above the top) until the next layout.
                let moved = self.cull.outer.as_ref().map_or(0., |(outer, then)| {
                    clamped(outer) - then.get()
                });
                let (top, offset) = (f32::from(bounds.origin.y) + moved, clamped(&handle));
                let from = top.max(0.) - top - offset;
                let to = (top + f32::from(bounds.size.height)).min(window_height) - top - offset;
                let shown = (to - from).max(0.);
                // A viewport kept on either side: never less than what a
                // scroll layer records past its edges (`scroll_layer::REACH`,
                // one viewport), so a step it replays shows only rows built,
                // and a step past it draws the panel again.
                let near = (from - shown, to + shown);
                let mut budget = MEASURE_PER_FRAME;
                let before = self.cull.culled.len();
                self.flow(scroller, &handle, 0., near, &extents, &mut budget);
                if self.cull.culled.len() > before {
                    self.cull.culled_in.insert(scroller);
                }
            }
        }
        if self.cull.pending {
            window.request_animation_frame();
        }
        if crate::diag::enabled() {
            let scrollers: Vec<(f32, f32, f32)> = self
                .prepared
                .nodes
                .values()
                .filter(|n| n.scroll_y)
                .filter_map(|n| self.scrolls.get(&n.id))
                .map(|h| (f32::from(h.bounds().origin.y), f32::from(h.bounds().size.height), -f32::from(h.offset().y)))
                .collect();
            crate::diag::line(serde_json::json!({"ui": "cull", "nodes": self.prepared.nodes.len(),
                "culled": self.cull.culled.len(), "measured": self.cull.measured.len(),
                "known": self.cull.extents.borrow().len(), "pending": self.cull.pending,
                "scrollers": format!("{scrollers:?}"), "grows": self.grows,
                "window": f32::from(window.viewport_size().height), "panel": self.publication.panel.as_str()}));
        }
        // The controls of rows built now that were never built before.
        let due: Vec<(Id, Kind)> = self
            .cull
            .deferred
            .iter()
            .filter(|(id, _)| !self.left_out(**id))
            .map(|(id, kind)| (*id, *kind))
            .collect();
        let started = std::time::Instant::now();
        let made = due.len();
        for (id, kind) in due {
            self.cull.deferred.remove(&id);
            self.mount_deferred(id, kind, window, cx);
        }
        if crate::diag::enabled() && made > 0 {
            crate::diag::line(serde_json::json!({"ui": "cull_mount", "controls": made,
                "ms": started.elapsed().as_secs_f64() * 1000.}));
        }
    }
    /// Lays out the children of node `id` (at `top` in its scroller's
    /// content) as far as deciding which are left out, and returns the
    /// node's height.
    fn flow(
        &mut self,
        id: Id,
        handle: &ScrollHandle,
        top: f32,
        near: (f32, f32),
        extents: &HashMap<Id, (f32, f32)>,
        budget: &mut usize,
    ) -> f32 {
        let prepared = self.prepared.clone();
        let Some(node) = prepared.node(id) else {
            return 0.;
        };
        // A top from before the newest publication is no longer known (NaN):
        // the node goes where its siblings' heights put it.
        let known = extents
            .get(&id)
            .copied()
            .map(|(t, h)| (if t.is_nan() { top } else { t }, h));
        if !stacks(&node.style) {
            // Side by side (a row wrapping a page's column): each child starts
            // where the node does, and is laid out the same way, none left out.
            let top = known.map_or(top, |(t, _)| t);
            let mut tallest = 0f32;
            for &child in &node.children {
                if prepared.node(child).is_some_and(|c| {
                    !c.hidden && !c.is_plain_text && c.virtual_window.is_none() && c.native.is_none()
                }) {
                    tallest = tallest.max(self.flow(child, handle, top, near, extents, budget));
                }
            }
            return known.map_or_else(|| tallest.max(self.estimate(id, extents)), |(_, h)| h);
        }
        let mut cursor = known.map_or(top, |(t, _)| t);
        for &child in &node.children {
            let Some(c) = prepared.node(child) else {
                continue;
            };
            if c.hidden {
                continue;
            }
            // A virtual list places its own rows; a native view draws itself.
            if c.virtual_window.is_some() || c.native.is_some() {
                cursor += extents
                    .get(&child)
                    .map_or_else(|| self.estimate(child, extents), |(_, h)| *h);
                continue;
            }
            if c.is_plain_text || c.style.position == Some(Position::Absolute) {
                cursor += if c.is_plain_text { self.leaf() } else { 0. };
                continue;
            }
            self.cull.measured.insert(child, handle.clone());
            let measured = extents.get(&child).copied();
            let (child_top, height) = match measured {
                Some((t, h)) if !t.is_nan() => (t, h),
                Some((_, h)) => (cursor, h),
                None => (cursor, self.estimate(child, extents)),
            };
            let away = child_top + height < near.0 || child_top > near.1;
            let leave_out = away
                && (measured.is_some() || {
                    // Built anyway to be measured, a few a frame.
                    let spare = *budget == 0;
                    *budget = budget.saturating_sub(1);
                    spare
                });
            if leave_out {
                if measured.is_none() {
                    self.cull.pending = true;
                }
                self.cull.culled.insert(child, height);
                cursor = child_top + height;
            } else {
                let built = self.flow(child, handle, child_top, near, extents, budget);
                cursor = child_top + built;
            }
        }
        known.map_or(cursor - top, |(_, h)| h)
    }
    /// A node's height before it was ever built: its stacked children's,
    /// or one row.
    fn estimate(&self, id: Id, extents: &HashMap<Id, (f32, f32)>) -> f32 {
        if let Some(&(_, h)) = extents.get(&id) {
            return h;
        }
        let Some(node) = self.prepared.node(id) else {
            return 0.;
        };
        if node.children.is_empty() || node.is_plain_text {
            return self.leaf();
        }
        if stacks(&node.style) {
            node.children
                .iter()
                .filter(|c| self.prepared.node(**c).is_some_and(|c| !c.hidden))
                .map(|c| self.estimate(*c, extents))
                .sum()
        } else {
            self.table.tokens.metrics.form_row
        }
    }
    fn leaf(&self) -> f32 {
        self.table.tokens.metrics.row
    }
    /// The panel sits in a body that scrolls around it (`builtins`): its
    /// steps move the panel's scrollers before the panel draws again.
    pub(crate) fn scrolls_in(&mut self, body: &ScrollHandle) {
        // One body per panel, kept for the panel's life (`builtins::scrolls`).
        if self.cull.outer.is_none() {
            self.cull.outer = Some((body.clone(), Default::default()));
        }
    }
    /// Whether this frame leaves rows of scroller `id` out.
    pub(super) fn culls_in(&self, id: Id) -> bool {
        self.cull.culled_in.contains(&id)
    }
    /// A panel large enough that its controls wait for their rows: a small
    /// page makes its few before the frame, as any panel does (a pop-up is a
    /// few milliseconds to make, which a frame should not pay).
    pub(super) fn cull_large(&self) -> bool {
        self.prepared.nodes.len() > LARGE
    }
    /// Whether the last frame left rows out: a body that scrolls around the
    /// panel draws it again on each step, so rows come back as they near.
    pub(crate) fn culls(&self) -> bool {
        !self.cull.culled.is_empty()
    }
    /// Whether node `id` or one of its ancestors is left out this frame.
    pub(super) fn left_out(&self, mut id: Id) -> bool {
        loop {
            if self.cull.culled.contains_key(&id) {
                return true;
            }
            match self.prepared.node(id).and_then(|n| n.parent) {
                Some(parent) => id = parent,
                None => return false,
            }
        }
    }
    /// Whether node `id` is `ancestor` or inside it.
    fn inside(&self, mut id: Id, ancestor: Id) -> bool {
        loop {
            if id == ancestor {
                return true;
            }
            match self.prepared.node(id).and_then(|n| n.parent) {
                Some(parent) => id = parent,
                None => return false,
            }
        }
    }
    /// Nodes inside a vertical scroller (their controls can wait for their
    /// row to be built).
    pub(super) fn in_scroller(&self, mut id: Id) -> bool {
        while let Some(parent) = self.prepared.node(id).and_then(|n| n.parent) {
            if self.prepared.node(parent).is_some_and(|p| p.scroll_y) {
                return true;
            }
            id = parent;
        }
        false
    }
}

/// Whether a node stacks its children top to bottom (a block, or a flex
/// column).
fn stacks(style: &gpui::StyleRefinement) -> bool {
    match style.display {
        Some(Display::Flex) => matches!(
            style.flex_direction,
            Some(FlexDirection::Column)
        ),
        Some(Display::Block) | None => true,
        _ => false,
    }
}

/// A scroller's vertical offset within its range, as its layout leaves it.
fn clamped(handle: &ScrollHandle) -> f32 {
    let max = f32::from(handle.max_offset().y).max(0.);
    f32::from(handle.offset().y).clamp(-max, 0.)
}

/// Whether VoiceOver is running: it reads only what is built.
fn voiceover_running() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().isVoiceOverEnabled()
}
