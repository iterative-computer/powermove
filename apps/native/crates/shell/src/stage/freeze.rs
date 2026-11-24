//! A subtree that can be pictured on demand: the frame a `Freeze` is asked
//! to capture, it paints its child as usual and keeps an offscreen picture
//! of it (GPUI's `paint_snapshot`, as the wave does). A beat holds
//! the editor as it was while the real change is applied, then waves from
//! that picture into the live editor.
use gpui::{prelude::*, *};
use std::{cell::RefCell, rc::Rc, sync::Arc};

/// The picture last captured, shared with whoever asked for it.
#[derive(Clone, Default)]
pub struct Frozen(Rc<RefCell<Option<Arc<RenderImage>>>>);
impl Frozen {
    pub fn get(&self) -> Option<Arc<RenderImage>> {
        self.0.borrow().clone()
    }
    pub fn clear(&self) {
        self.0.borrow_mut().take();
    }
}

pub struct Freeze {
    child: AnyElement,
    into: Frozen,
    capture: bool,
}
/// `child`, captured into `into` this frame when `capture` is set.
pub fn freeze(child: impl IntoElement, into: Frozen, capture: bool) -> Freeze {
    Freeze {
        child: child.into_any_element(),
        into,
        capture,
    }
}
impl IntoElement for Freeze {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for Freeze {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let child = self.child.request_layout(window, cx);
        let style = Style {
            size: size(relative(1.).into(), relative(1.).into()),
            ..Style::default()
        };
        (window.request_layout(style, [child], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if !self.capture {
            self.child.paint(window, cx);
            return;
        }
        match window.paint_snapshot(bounds, cx, |w, c| self.child.paint(w, c)) {
            Ok(mut picture) => {
                // GPUI's sprites are BGRA.
                for p in picture.pixels_mut() {
                    p.0.swap(0, 2);
                }
                // The child is in this frame as painted; the picture is kept.
                *self.into.0.borrow_mut() =
                    Some(Arc::new(RenderImage::new([image::Frame::new(picture)])));
            }
            Err(error) => {
                eprintln!("The stage could not picture the editor: {error:#}");
                self.into.clear();
            }
        }
    }
}
