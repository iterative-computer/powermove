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
    geometry: Cell<Option<(u64, NSRect, NSRect)>>,
    staged: Cell<Option<(NSRect, NSRect)>>,
}
impl State {
    fn key(&self) -> usize {
        Retained::as_ptr(&self.gpui) as usize
    }
    fn stage(&self, geometry: Option<(NSRect, NSRect)>) {
        let paint = crate::composition::open_paint(self.key());
        self.geometry.set(geometry.map(|(bounds, clip)| (paint, bounds, clip)));
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
    fn placed(&self) -> Option<(NSRect, NSRect)> {
        let last = crate::composition::last_paint(self.key());
        self.geometry
            .get()
            .filter(|(paint, ..)| Some(*paint) == last)
            .map(|(_, bounds, clip)| (bounds, clip))
    }
    /// The wrapper's frame, the control's frame inside it and whether the
    /// wrapper is hidden, for what the last paint staged.
    fn target(&self) -> Option<(NSRect, NSRect, bool)> {
        let (bounds, clip) = self.placed()?;
        let x = bounds.origin.x.max(clip.origin.x);
        let y = bounds.origin.y.max(clip.origin.y);
        let visible = NSSize::new(
            ((bounds.origin.x + bounds.size.width).min(clip.origin.x + clip.size.width) - x)
                .max(0.),
            ((bounds.origin.y + bounds.size.height).min(clip.origin.y + clip.size.height) - y)
                .max(0.),
        );
        // In GPUI's view's parent (the window's content view, or the detail
        // view of a split view that hosts GPUI's view), from GPUI's frame.
        let g = self.gpui.frame();
        let frame = NSRect::new(
            NSPoint::new(
                g.origin.x + x,
                g.origin.y + g.size.height - y - visible.height,
            ),
            visible,
        );
        let local = NSRect::new(
            NSPoint::new(
                bounds.origin.x - x,
                y + visible.height - bounds.origin.y - bounds.size.height,
            ),
            bounds.size,
        );
        Some((frame, local, visible.width <= 0. || visible.height <= 0.))
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
            staged: Cell::new(None),
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
        self.state.stage(Some((bounds, clip)));
        self.state.staged.set(Some((bounds, clip)));
    }
    /// A cached GPUI view reuses its paint, so its slot keeps the last geometry.
    pub fn keep(&self) {
        self.state.stage(self.state.staged.get());
    }
    /// The control's wrapper, for tests that read where a present put it.
    #[doc(hidden)]
    pub fn wrapper(&self) -> &NSView {
        &self.state.wrapper
    }
}
/// A control as AppKit draws it now: premultiplied BGRA rows from the top,
/// `width` by `height` pixels at the window's backing scale, BGRA with
/// straight (not premultiplied) alpha, rows from the top.
pub struct SlotPicture {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}
impl NativeControlSlot {
    /// Powermove P-SLOT-PICTURE: the control drawn into a bitmap in its
    /// current state and appearance (light or dark, its value, enabled or
    /// not), without moving or showing it: while a scroll moves, the panel
    /// draws this picture with its rows instead of moving the AppKit view
    /// (each move made the frame a synchronous present). `None` when AppKit
    /// gives no 32-bit bitmap or the control has no size. `size` is where
    /// the paint puts it, in points: the size a present would give it.
    pub fn picture(&self, size: NSSize) -> Option<SlotPicture> {
        if size.width < 1. || size.height < 1. {
            return None;
        }
        // A control never shown yet joins the window (still hidden), so it
        // draws in the window's appearance and key state, as it will show.
        if unsafe { self.state.wrapper.superview() }.is_none() {
            self.state.attach();
        }
        picture_of(&self.state.control, size)
    }
    /// Stages nothing: the next present hides the control (a picture of it
    /// stands in the scene). [`Self::keep`] keeps it hidden.
    pub fn hide(&self) {
        self.state.stage(None);
        self.state.staged.set(None);
    }
}
/// `view` drawn by AppKit into a bitmap at `size` points, wherever it is
/// and whether shown or not (P-SLOT-PICTURE).
pub(crate) fn picture_of(view: &NSView, size: NSSize) -> Option<SlotPicture> {
    use objc2::{msg_send, runtime::AnyObject};
    if size.width < 1. || size.height < 1. {
        return None;
    }
    if view.frame().size != size {
        view.setFrameSize(size);
    }
    let control = view;
    let bounds = view.bounds();
    objc2::rc::autoreleasepool(|_| unsafe {
        let rep: *mut AnyObject =
            msg_send![control, bitmapImageRepForCachingDisplayInRect: bounds];
        let rep = rep.as_ref()?;
        let _: () = msg_send![control, cacheDisplayInRect: bounds, toBitmapImageRep: rep];
        let width: isize = msg_send![rep, pixelsWide];
        let height: isize = msg_send![rep, pixelsHigh];
        let row: isize = msg_send![rep, bytesPerRow];
        let bits: isize = msg_send![rep, bitsPerPixel];
        let samples: isize = msg_send![rep, samplesPerPixel];
        let format: usize = msg_send![rep, bitmapFormat];
        let data: *const u8 = msg_send![rep, bitmapData];
        if data.is_null() || bits != 32 || samples != 4 || width <= 0 || height <= 0 {
            return None;
        }
        // NSBitmapFormat: alpha first (1 << 0), not premultiplied (1 << 1),
        // floating point (1 << 2).
        if format & 0b100 != 0 {
            return None;
        }
        let alpha_first = format & 0b1 != 0;
        let straight = format & 0b10 != 0;
        let (w, h, row) = (width as usize, height as usize, row as usize);
        let src = std::slice::from_raw_parts(data, row * h);
        let mut bgra = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let p = &src[y * row + x * 4..y * row + x * 4 + 4];
                let (r, g, b, a) = if alpha_first {
                    (p[1], p[2], p[3], p[0])
                } else {
                    (p[0], p[1], p[2], p[3])
                };
                // Straight alpha, GPUI's sprite format.
                let out = |c: u8| {
                    if straight || a == 0 || a == 255 {
                        c
                    } else {
                        ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8
                    }
                };
                bgra.extend_from_slice(&[out(b), out(g), out(r), a]);
            }
        }
        Some(SlotPicture {
            width: w as u32,
            height: h as u32,
            bgra,
        })
    })
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
        .any(|state| match state.target() {
            None => !state.wrapper.isHidden(),
            Some((frame, local, hidden)) => {
                state.wrapper.frame() != frame
                    || state.control.frame() != local
                    || state.wrapper.isHidden() != hidden
            }
        })
}
pub(crate) fn present(key: usize) -> crate::composition::SlotChange {
    let slots = SLOTS.with(|s| s.borrow().get(&key).cloned().unwrap_or_default());
    let mut moved = false;
    for state in slots.into_iter().filter_map(|s| s.upgrade()) {
        let target = state.target();
        state.geometry.set(None);
        let Some((frame, local, hidden)) = target else {
            if !state.wrapper.isHidden() {
                state.wrapper.setHidden(true);
                moved = true;
                if std::env::var_os("PM_SLOT_TRACE").is_some() { eprintln!("PM_SLOT control hide-untargeted"); }
            }
            continue;
        };
        // P-HIDDEN-MOVES: a wrapper hidden before and after (scrolled out
        // of its clip) follows its row unseen, with no synchronous present.
        let unseen = hidden && state.wrapper.isHidden();
        if state.wrapper.frame() != frame {
            state.wrapper.setFrame(frame);
            moved |= !unseen;
            if !unseen && std::env::var_os("PM_SLOT_TRACE").is_some() { eprintln!("PM_SLOT control frame hidden={hidden} was_hidden={}", state.wrapper.isHidden()); }
        }
        if state.control.frame() != local {
            state.control.setFrame(local);
            moved |= !unseen;
        }
        if !hidden && unsafe { state.wrapper.superview() }.is_none() {
            state.attach();
        }
        if state.wrapper.isHidden() != hidden {
            if std::env::var_os("PM_SLOT_TRACE").is_some() { eprintln!("PM_SLOT control show={}", !hidden); }
            state.wrapper.setHidden(hidden);
            moved = true;
            crate::composition_accessibility::changed(key);
        }
    }
    moved.into()
}
