//! Select part of the screen for the agent (v1's composer, PM-399): a
//! crosshair over a window's content; dragging draws the box over a light
//! wash, releasing answers it, Escape cancels. A click without a drag
//! answers a box of no size at that point (the composition or panel there).
//!
//! The box is in the window's content, in top-left points.
use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::*;
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize};
use std::cell::{Cell, RefCell};

/// The box, in the window's content (top-left points), or `None` (Escape).
pub type Picked = Option<(f64, f64, f64, f64)>;

pub struct Pick {
    tx: async_channel::Sender<Picked>,
    start: Cell<Option<NSPoint>>,
    end: Cell<Option<NSPoint>>,
    done: Cell<bool>,
    overlay: RefCell<Option<Retained<NSPanel>>>,
    parent: RefCell<Option<Retained<NSWindow>>>,
}

define_class!(
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMRegionPickView"]
    #[ivars = Pick]
    pub struct PickView;
    unsafe impl NSObjectProtocol for PickView {}
    impl PickView {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }
        #[unsafe(method(acceptsFirstMouse:))]
        fn first_mouse(&self, _: Option<&NSEvent>) -> bool { true }
        #[unsafe(method(resetCursorRects))]
        fn cursor_rects(&self) {
            self.addCursorRect_cursor(self.bounds(), &NSCursor::crosshairCursor());
        }
        #[unsafe(method(mouseDown:))]
        fn down(&self, event: &NSEvent) {
            let at = self.convertPoint_fromView(event.locationInWindow(), None);
            self.ivars().start.set(Some(at));
            self.ivars().end.set(Some(at));
            self.setNeedsDisplay(true);
        }
        #[unsafe(method(mouseDragged:))]
        fn dragged(&self, event: &NSEvent) {
            self.ivars().end.set(Some(self.convertPoint_fromView(event.locationInWindow(), None)));
            self.setNeedsDisplay(true);
        }
        #[unsafe(method(mouseUp:))]
        fn up(&self, event: &NSEvent) {
            self.ivars().end.set(Some(self.convertPoint_fromView(event.locationInWindow(), None)));
            let rect = self.rect();
            finish(self, rect.map(|r| (r.origin.x, r.origin.y, r.size.width, r.size.height)));
        }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _: NSRect) {
            // Dim enough to read over a dark editor as well as a light one.
            NSColor::colorWithWhite_alpha(0., 0.35).setFill();
            NSRectFill(self.bounds());
            if let Some(r) = self.rect().filter(|r| r.size.width > 0. && r.size.height > 0.) {
                NSRectFillUsingOperation(r, NSCompositingOperation::Clear);
                NSColor::colorWithWhite_alpha(1., 0.9).setStroke();
                let path = NSBezierPath::bezierPathWithRect(NSRect::new(
                    NSPoint::new(r.origin.x + 0.5, r.origin.y + 0.5),
                    NSSize::new((r.size.width - 1.).max(0.), (r.size.height - 1.).max(0.)),
                ));
                path.setLineWidth(1.);
                path.stroke();
            }
        }
    }
);

impl PickView {
    fn rect(&self) -> Option<NSRect> {
        let (a, b) = (self.ivars().start.get()?, self.ivars().end.get()?);
        let (x0, y0) = (a.x.min(b.x), a.y.min(b.y));
        let (w, h) = ((a.x - b.x).abs(), (a.y - b.y).abs());
        // A click, not a drag: a box of no size where it was.
        Some(if w < 3. && h < 3. {
            NSRect::new(NSPoint::new(a.x, a.y), NSSize::new(0., 0.))
        } else {
            NSRect::new(NSPoint::new(x0, y0), NSSize::new(w, h))
        })
    }
}

fn finish(view: &PickView, picked: Picked) {
    let state = view.ivars();
    if state.done.replace(true) {
        return;
    }
    if let (Some(overlay), Some(parent)) = (state.overlay.borrow_mut().take(), state.parent.borrow_mut().take()) {
        parent.removeChildWindow(&overlay);
        overlay.orderOut(None);
    }
    let _ = state.tx.try_send(picked);
    PICKING.with(|p| p.borrow_mut().take());
}

thread_local! {
    /// The pick in progress (one at a time), kept until it answers.
    static PICKING: RefCell<Option<Retained<PickView>>> = const { RefCell::new(None) };
}

/// Cancels the pick in progress (Escape); whether there was one.
pub fn cancel() -> bool {
    let Some(view) = PICKING.with(|p| p.borrow().clone()) else {
        return false;
    };
    finish(&view, None);
    true
}

/// Starts a pick over `window`'s content; the receiver gets the box once.
/// A pick already in progress is cancelled first.
pub fn pick(window: &NSWindow) -> async_channel::Receiver<Picked> {
    let (tx, rx) = async_channel::bounded(1);
    let Some(mtm) = MainThreadMarker::new() else {
        let _ = tx.try_send(None);
        return rx;
    };
    if let Some(old) = PICKING.with(|p| p.borrow().clone()) {
        finish(&old, None);
    }
    let frame = window.contentRectForFrameRect(window.frame());
    let overlay = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        frame,
        NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
        NSBackingStoreType::Buffered,
        false,
    );
    unsafe { overlay.setReleasedWhenClosed(false) };
    overlay.setOpaque(false);
    overlay.setBackgroundColor(Some(&NSColor::clearColor()));
    overlay.setHasShadow(false);
    overlay.setIgnoresMouseEvents(false);
    let view = PickView::alloc(mtm).set_ivars(Pick {
        tx,
        start: Cell::new(None),
        end: Cell::new(None),
        done: Cell::new(false),
        overlay: RefCell::new(Some(overlay.clone())),
        parent: RefCell::new(Some(window.retain())),
    });
    let view: Retained<PickView> = unsafe {
        msg_send![super(view), initWithFrame: NSRect::new(NSPoint::new(0., 0.), frame.size)]
    };
    overlay.setContentView(Some(&view));
    // Escape cancels through the shell's key monitor (`cancel`): the
    // editor keeps the keyboard, the overlay never becomes key.
    unsafe { window.addChildWindow_ordered(&overlay, NSWindowOrderingMode::Above) };
    overlay.invalidateCursorRectsForView(&view);
    PICKING.with(|p| *p.borrow_mut() = Some(view));
    rx
}
