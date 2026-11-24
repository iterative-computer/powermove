//! AppKit control slots staged with the same frame transaction and AX ordering
//! as native text. A slot owns no callbacks, model, focus policy or persistence.
use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSView, NSWindowOrderingMode};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
};
struct State {
    gpui: Retained<NSView>,
    wrapper: Retained<NSView>,
    control: Retained<NSView>,
    /// What a paint staged, with that paint's number: placed by the present
    /// that follows that paint, and by no later one.
    /// The region is the scrolling region whose container holds the slot
    /// (P-SLOT-REGION): bounds and clip are then in its content coordinates.
    geometry: Cell<Option<(u64, crate::slot_region::Placement)>>,
    scroll_fade: crate::scroll_fade::ScrollFade,
    staged: Cell<Option<crate::slot_region::Placement>>,
    alpha: Cell<Option<(u64, f64)>>,
    staged_alpha: Cell<f64>,
    /// A region's slot as the last present left it: its placement, its
    /// container's content view and alpha. A frame that stages the same
    /// touches no AppKit view of it (a scroll moves only the container).
    settled: Cell<Option<(crate::slot_region::Placement, u64, f64, bool)>>,
}
impl State {
    /// The region slot's settled key for this present, when its placement is
    /// a region's (`apply` places the container to find its content view).
    fn region_key(&self, apply: bool) -> Option<(crate::slot_region::Placement, u64, f64, bool)> {
        let placement = self.placed()?;
        let region = placement.region?;
        crate::slot_region::content_view(&self.gpui, region, apply)?;
        let serial = crate::slot_region::serial(&self.gpui, region)?;
        let last = crate::composition::last_paint(self.key());
        let alpha = self
            .alpha
            .get()
            .filter(|(paint, _)| Some(*paint) == last)
            .map_or(self.staged_alpha.get(), |(_, a)| a);
        let visible = Self::visible_rect(placement.bounds, placement.clip);
        let shows = crate::slot_region::shows(&self.gpui, region, visible);
        Some((placement, serial, alpha, shows))
    }
    fn key(&self) -> usize {
        Retained::as_ptr(&self.gpui) as usize
    }
    fn stage(&self, geometry: Option<crate::slot_region::Placement>) {
        let paint = crate::composition::open_paint(self.key());
        self.geometry.set(geometry.map(|placement| (paint, placement)));
    }
    /// Powermove P-SLOT-LAZY: a control joins the window the first time a
    /// paint shows it. AppKit lays out every view in the window, hidden or
    /// not, and on macOS 27 a control's first layout builds its SwiftUI
    /// graph (a millisecond or more each): a list of a few hundred rows made
    /// them all at once though only those on screen were ever shown.
    /// It joins GPUI's view's parent: the window's content view, or the
    /// detail view of a split view hosting GPUI's view (P-EMBED).
    fn attach(&self) {
        if let Some(content) = unsafe { self.gpui.superview() } {
            content.addSubview_positioned_relativeTo(
                &self.wrapper,
                NSWindowOrderingMode::Above,
                Some(&self.gpui),
            );
        }
    }
    /// Where the last complete paint put this control, if it staged it.
    fn placed(&self) -> Option<crate::slot_region::Placement> {
        let last = crate::composition::last_paint(self.key());
        self.geometry
            .get()
            .filter(|(paint, _)| Some(*paint) == last)
            .map(|(_, placement)| placement)
    }
    fn visible_rect(bounds: NSRect, clip: NSRect) -> NSRect {
        let x = bounds.origin.x.max(clip.origin.x);
        let y = bounds.origin.y.max(clip.origin.y);
        NSRect::new(
            NSPoint::new(x, y),
            NSSize::new(
                ((bounds.origin.x + bounds.size.width).min(clip.origin.x + clip.size.width) - x).max(0.),
                ((bounds.origin.y + bounds.size.height).min(clip.origin.y + clip.size.height) - y).max(0.),
            ),
        )
    }

    fn place_fade(&self, apply: bool) -> bool {
        // In a region its container's clip fades.
        let Some(placement) = self.placed().filter(|p| p.region.is_none()) else {
            return false;
        };
        let (bounds, clip) = (placement.bounds, placement.clip);
        self.scroll_fade.place(&self.wrapper, Self::visible_rect(bounds, clip), clip, apply)
    }
    /// The wrapper's frame, the control's frame inside it, whether the
    /// wrapper is hidden and the view it belongs in (a region's content view,
    /// or `None` for GPUI's view's parent), for what the last paint staged.
    /// `apply` places the region's containers to find that view; otherwise
    /// one not placed yet reads as a move.
    fn target(&self, apply: bool) -> Option<(NSRect, NSRect, bool, Option<Retained<NSView>>)> {
        let placement = self.placed()?;
        let (bounds, clip) = (placement.bounds, placement.clip);
        let visible = Self::visible_rect(bounds, clip);
        let x = visible.origin.x;
        let y = visible.origin.y;
        let (frame, parent, outside) = match placement.region {
            None => (crate::composition::hosted_rect(&self.gpui, visible), None, false),
            // Top-down content coordinates; hidden while its region's view
            // (cut by the regions around it) shows none of it, so it takes no
            // keys and is not read out while scrolled away (PM-696 review).
            Some(region) => (
                visible,
                Some(crate::slot_region::content_view(&self.gpui, region, apply)?),
                !crate::slot_region::shows(&self.gpui, region, visible),
            ),
        };
        let visible = visible.size;
        let local = NSRect::new(
            NSPoint::new(
                bounds.origin.x - x,
                y + visible.height - bounds.origin.y - bounds.size.height,
            ),
            bounds.size,
        );
        Some((frame, local, outside || visible.width <= 0. || visible.height <= 0., parent))
    }
}
thread_local! {static SLOTS:RefCell<HashMap<usize,Vec<Weak<State>>>>=RefCell::new(HashMap::new());}
pub struct NativeControlSlot {
    state: Rc<State>,
}
impl NativeControlSlot {
    pub fn new(gpui: &NSView, control: Retained<NSView>) -> Self {
        let mtm = MainThreadMarker::new().expect("native controls require main");
        let wrapper = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
        wrapper.setWantsLayer(true);
        wrapper.setHidden(true);
        // GPUI lays out from the top: between a change of the window's
        // height and the next paint, a control keeps its distance from the
        // top edge, not from the bottom one.
        wrapper.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
        if let Some(layer) = wrapper.layer() {
            layer.setMasksToBounds(true);
        }
        wrapper.addSubview(&control);
        let state = Rc::new(State {
            gpui: unsafe { Retained::retain(gpui as *const NSView as *mut NSView).unwrap() },
            wrapper,
            control,
            geometry: Cell::new(None),
            scroll_fade: Default::default(),
            staged: Cell::new(None),
            alpha: Cell::new(None),
            staged_alpha: Cell::new(1.),
            settled: Cell::new(None),
        });
        SLOTS.with(|s| {
            s.borrow_mut()
                .entry(gpui as *const NSView as usize)
                .or_default()
                .push(Rc::downgrade(&state))
        });
        Self { state }
    }
    pub fn stage(&self, bounds: NSRect, clip: NSRect) {
        self.stage_with_scroll_fade(bounds, clip, [0., 0.]);
    }
    pub fn stage_with_scroll_fade(&self, bounds: NSRect, clip: NSRect, fade: [f64; 2]) {
        let placement = crate::slot_region::Placement { bounds, clip, region: None };
        self.state.stage(Some(placement));
        self.state.staged.set(Some(placement));
        self.state.scroll_fade.stage(fade);
    }
    /// P-SLOT-REGION: staged in scrolling region `region`'s container, with
    /// bounds and clip in its content coordinates (a scroll moves the
    /// container, not the slot).
    pub fn stage_in_region(&self, region: usize, bounds: NSRect, clip: NSRect) {
        let placement = crate::slot_region::Placement { bounds, clip, region: Some(region) };
        self.state.stage(Some(placement));
        self.state.staged.set(Some(placement));
    }
    pub fn stage_alpha(&self, alpha: f64) {
        let alpha = alpha.clamp(0., 1.);
        let paint = crate::composition::open_paint(self.state.key());
        self.state.alpha.set(Some((paint, alpha)));
        self.state.staged_alpha.set(alpha);
    }
    /// A cached GPUI view reuses its paint, so its slot keeps the last geometry.
    pub fn keep(&self) {
        self.state.stage(self.state.staged.get());
        self.stage_alpha(self.state.staged_alpha.get());
    }
    /// The control's wrapper, for tests that read where a present put it.
    #[doc(hidden)]
    pub fn wrapper(&self) -> &NSView {
        &self.state.wrapper
    }
}
impl NativeControlSlot {
    /// Stages nothing: the next present hides the control; [`Self::keep`]
    /// keeps it hidden.
    pub fn hide(&self) {
        self.state.stage(None);
        self.state.staged.set(None);
    }
}
impl Drop for NativeControlSlot {
    fn drop(&mut self) {
        self.state.wrapper.removeFromSuperview();
        let key = Retained::as_ptr(&self.state.gpui) as usize;
        crate::composition::clear_parents(&[crate::composition::AccessibilityItem::Native(
            self.state.control.clone(),
        )]);
        crate::composition_accessibility::changed(key);
        SLOTS.with(|s| {
            if let Some(slots) = s.borrow_mut().get_mut(&key) {
                slots.retain(|s| s.upgrade().is_some_and(|s| !Rc::ptr_eq(&s, &self.state)));
            }
        });
    }
}
pub(crate) fn has_slots(key: usize) -> bool {
    SLOTS.with(|s| {
        s.borrow()
            .get(&key)
            .is_some_and(|s| s.iter().any(|s| s.strong_count() > 0))
    })
}
pub(crate) fn accessibility(key: usize) -> Vec<crate::composition::AccessibilityItem> {
    SLOTS.with(|s| {
        s.borrow()
            .get(&key)
            .into_iter()
            .flatten()
            .filter_map(Weak::upgrade)
            .filter(|s| !s.wrapper.isHidden())
            .map(|s| crate::composition::AccessibilityItem::Native(s.control.clone()))
            .collect()
    })
}
/// Whether [`present`] would move a control. Nothing is touched.
pub(crate) fn pending(key: usize) -> bool {
    let slots = SLOTS.with(|s| s.borrow().get(&key).cloned().unwrap_or_default());
    slots
        .into_iter()
        .filter_map(|s| s.upgrade())
        .any(|state| {
            if let Some(key) = state.region_key(false)
                && state.settled.get() == Some(key)
            {
                return false;
            }
            match state.target(false) {
            None => !state.wrapper.isHidden() || state.placed().is_some_and(|p| p.region.is_some()),
            Some((frame, local, hidden, parent)) => {
                crate::slot_region::misplaced(&state.gpui, &state.wrapper, parent.as_deref())
                    || state.wrapper.frame() != frame
                    || state.control.frame() != local
                    || state.wrapper.isHidden() != hidden
                    || (!hidden && state.place_fade(false))
                    || state.alpha.get().is_some_and(|(paint, alpha)| {
                        Some(paint) == crate::composition::last_paint(key)
                            && state.control.alphaValue() != alpha
                    })
            }
        }
        })
}
pub(crate) fn present(key: usize) -> crate::composition::SlotChange {
    let slots = SLOTS.with(|s| s.borrow().get(&key).cloned().unwrap_or_default());
    let mut moved = false;
    for state in slots.into_iter().filter_map(|s| s.upgrade()) {
        let settled = state.region_key(true);
        if settled.is_some() && state.settled.get() == settled {
            state.geometry.set(None);
            continue;
        }
        state.settled.set(None);
        let target = state.target(true);
        let Some((frame, local, hidden, parent)) = target else {
            state.geometry.set(None);
            if !state.wrapper.isHidden() {
                state.wrapper.setHidden(true);
                moved = true;
            }
            continue;
        };
        // P-HIDDEN-MOVES: a wrapper hidden before and after (scrolled out
        // of its clip) follows its row unseen, with no synchronous present.
        let unseen = hidden && state.wrapper.isHidden();
        // Into its region's container, or back out of one.
        let (adopted, frame) =
            crate::slot_region::reparent(&state.gpui, &state.wrapper, parent.as_deref(), frame);
        if adopted {
            moved = true;
            // In a region its container's clip fades, not its own mask.
            if parent.is_some() {
                state.scroll_fade.clear(&state.wrapper);
            }
        }
        if state.wrapper.frame() != frame {
            state.wrapper.setFrame(frame);
            moved |= !unseen;
        }
        if state.control.frame() != local {
            state.control.setFrame(local);
            moved |= !unseen;
        }
        if let Some((paint, alpha)) = state.alpha.get() {
            if Some(paint) == crate::composition::last_paint(key)
                && state.control.alphaValue() != alpha
            {
                state.control.setAlphaValue(alpha);
                moved = true;
            }
        }
        if !hidden {
            moved |= state.place_fade(true);
        }
        state.geometry.set(None);
        if !hidden && unsafe { state.wrapper.superview() }.is_none() {
            state.attach();
        }
        if state.wrapper.isHidden() != hidden {
            state.wrapper.setHidden(hidden);
            moved = true;
            crate::composition_accessibility::changed(key);
        }
        // Settled only where it belongs: one held in its old parent for an
        // edit is placed again by the frames after the edit ends.
        let home = !crate::slot_region::misplaced(&state.gpui, &state.wrapper, parent.as_deref())
            && !crate::slot_region::holds_first_responder(&state.wrapper);
        // Shown or hidden alike: whether its region's view shows it is part
        // of the key.
        if settled.is_some() && home {
            state.settled.set(settled);
        }
    }
    moved.into()
}
