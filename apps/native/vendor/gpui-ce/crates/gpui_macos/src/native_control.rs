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
    geometry: Cell<Option<(NSRect, NSRect)>>,
    staged: Cell<Option<(NSRect, NSRect)>>,
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
            staged: Cell::new(None),
        });
        if let Some(content) = gpui.window().and_then(|w| w.contentView()) {
            content.addSubview_positioned_relativeTo(
                &state.wrapper,
                NSWindowOrderingMode::Above,
                Some(gpui),
            );
        }
        SLOTS.with(|s| {
            s.borrow_mut()
                .entry(gpui as *const NSView as usize)
                .or_default()
                .push(Rc::downgrade(&state))
        });
        Self { state }
    }
    pub fn stage(&self, bounds: NSRect, clip: NSRect) {
        self.state.geometry.set(Some((bounds, clip)));
        self.state.staged.set(Some((bounds, clip)));
    }
    /// A cached GPUI view reuses its paint, so its slot keeps the last geometry.
    pub fn keep(&self) {
        self.state.geometry.set(self.state.staged.get());
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
pub(crate) fn present(key: usize) -> crate::composition::SlotChange {
    let slots = SLOTS.with(|s| s.borrow().get(&key).cloned().unwrap_or_default());
    let mut moved = false;
    for state in slots.into_iter().filter_map(|s| s.upgrade()) {
        let Some((bounds, clip)) = state.geometry.take() else {
            if !state.wrapper.isHidden() {
                state.wrapper.setHidden(true);
                moved = true;
            }
            continue;
        };
        let x = bounds.origin.x.max(clip.origin.x);
        let y = bounds.origin.y.max(clip.origin.y);
        let visible = NSRect::new(
            NSPoint::new(x, y),
            NSSize::new(
                ((bounds.origin.x + bounds.size.width).min(clip.origin.x + clip.size.width) - x)
                    .max(0.),
                ((bounds.origin.y + bounds.size.height).min(clip.origin.y + clip.size.height) - y)
                    .max(0.),
            ),
        );
        let h = state
            .gpui
            .window()
            .and_then(|w| w.contentView())
            .map_or(0., |v| v.bounds().size.height);
        let frame = NSRect::new(NSPoint::new(x, h - y - visible.size.height), visible.size);
        if state.wrapper.frame() != frame {
            state.wrapper.setFrame(frame);
            moved = true;
        }
        let local = NSRect::new(
            NSPoint::new(
                bounds.origin.x - x,
                y + visible.size.height - bounds.origin.y - bounds.size.height,
            ),
            bounds.size,
        );
        if state.control.frame() != local {
            state.control.setFrame(local);
            moved = true;
        }
        let hidden = visible.size.width <= 0. || visible.size.height <= 0.;
        if state.wrapper.isHidden() != hidden {
            state.wrapper.setHidden(hidden);
            moved = true;
            crate::composition_accessibility::changed(key);
        }
    }
    moved.into()
}
