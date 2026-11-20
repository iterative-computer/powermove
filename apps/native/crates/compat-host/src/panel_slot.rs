//! AppKit owns the rectangular aperture of one panel. A document can keep a
//! larger layout for a scrolled body, but its native clip stays in its slot.
use crate::Rect;
use objc2::{MainThreadMarker, MainThreadOnly, Message, rc::Retained};
use objc2_app_kit::{NSView, NSWindowOrderingMode};
use objc2_foundation::{NSPoint, NSRect, NSSize};

pub(crate) fn editor_rect(rect: Rect, editor: &NSView, destination: &NSView) -> NSRect {
    let bounds = editor.bounds();
    let y = if editor.isFlipped() {
        bounds.origin.y + rect.y
    } else {
        bounds.origin.y + bounds.size.height - rect.y - rect.h
    };
    editor.convertRect_toView(
        NSRect::new(
            NSPoint::new(bounds.origin.x + rect.x, y),
            NSSize::new(rect.w, rect.h),
        ),
        Some(destination),
    )
}

pub(crate) struct PanelAttachment {
    parent: Retained<NSView>,
    editor: Retained<NSView>,
    aperture: Retained<NSView>,
}

impl PanelAttachment {
    pub(crate) fn new(
        mtm: MainThreadMarker,
        parent: &NSView,
        editor: &NSView,
        body: &NSView,
    ) -> Self {
        let aperture = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
        aperture.setWantsLayer(true);
        aperture
            .layer()
            .expect("panel aperture layer")
            .setMasksToBounds(true);
        aperture.addSubview(body);
        Self {
            parent: parent.retain(),
            editor: editor.retain(),
            aperture,
        }
    }

    /// Attach, size and clip the renderer in the current composition
    /// transaction. Entirely clipped panels have no window hierarchy.
    pub(crate) fn place(&self, body: Rect, clip: Rect) -> Option<NSRect> {
        let clip = body.intersection(clip);
        if clip.w <= 0. || clip.h <= 0. {
            self.detach();
            return None;
        }
        let outer = editor_rect(clip, &self.editor, &self.parent);
        if self.aperture.frame() != outer {
            self.aperture.setFrame(outer);
        }
        if unsafe { self.aperture.superview() }.is_none() {
            // Below the editor's branch in the window, even when AppKit
            // has wrapped the editor in a split view or tab container.
            let mut sibling = self.editor.clone();
            while let Some(parent) = unsafe { sibling.superview() } {
                if std::ptr::eq(&*parent, &*self.parent) {
                    break;
                }
                sibling = parent;
            }
            let relative = unsafe { sibling.superview() }
                .filter(|parent| std::ptr::eq(&**parent, &*self.parent))
                .map(|_| &*sibling);
            self.parent.addSubview_positioned_relativeTo(
                &self.aperture,
                NSWindowOrderingMode::Below,
                relative,
            );
        }
        let frame = editor_rect(body, &self.editor, &self.parent);
        Some(NSRect::new(
            NSPoint::new(
                frame.origin.x - outer.origin.x,
                frame.origin.y - outer.origin.y,
            ),
            frame.size,
        ))
    }

    pub(crate) fn editor(&self) -> Retained<NSView> {
        self.editor.clone()
    }

    pub(crate) fn detach(&self) {
        self.aperture.removeFromSuperview();
    }
}

impl Drop for PanelAttachment {
    fn drop(&mut self) {
        self.detach();
    }
}
