//! NSView/CAMetalLayer slot with matching native hit and shape masks.
use crate::contract::{Clip, Rect};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::NSView;
use objc2_core_graphics::CGMutablePath;
use objc2_foundation::{NSPoint, NSRect};
use std::cell::RefCell;

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMS2SlotView"]
    #[ivars = RefCell<Clip>]
    pub struct SlotView;
    impl SlotView {
        #[unsafe(method_id(hitTest:))]
        fn hit(&self,point:NSPoint)->Option<Retained<NSView>> {
            let local=self.convertPoint_fromView(point,unsafe {self.superview()}.as_deref());
            if self.ivars().borrow().contains([local.x,self.bounds().size.height-local.y]){unsafe {msg_send![super(self),hitTest:point]}}else{None}
        }
    }
);
impl SlotView {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let view = Self::alloc(mtm).set_ivars(RefCell::new(Clip::Rect(Rect::new(0., 0., 0., 0.))));
        unsafe { msg_send![super(view),initWithFrame:NSRect::ZERO] }
    }
    pub fn clip(&self, clip: &Clip) {
        *self.ivars().borrow_mut() = clip.clone();
        self.setWantsLayer(true);
        let path = CGMutablePath::new();
        let h = self.bounds().size.height;
        let add_rect = |r: &Rect| {
            let points = [
                [r.x, r.y],
                [r.x + r.w, r.y],
                [r.x + r.w, r.y + r.h],
                [r.x, r.y + r.h],
            ];
            add_polygon(&path, &points, h);
        };
        match clip {
            Clip::Rect(r) => add_rect(r),
            Clip::Polygon(p) => add_polygon(&path, p, h),
            Clip::Union(rs) => {
                for r in rs {
                    add_rect(r)
                }
            }
        }
        let mask = objc2_quartz_core::CAShapeLayer::new();
        mask.setPath(Some(&path));
        if let Some(layer) = self.layer() {
            unsafe {
                layer.setMask(Some(&mask));
            }
        }
    }
}
fn add_polygon(path: &CGMutablePath, points: &[[f64; 2]], h: f64) {
    if let Some(first) = points.first() {
        unsafe {
            CGMutablePath::move_to_point(Some(path), std::ptr::null(), first[0], h - first[1]);
            for p in &points[1..] {
                CGMutablePath::add_line_to_point(Some(path), std::ptr::null(), p[0], h - p[1]);
            }
        }
        unsafe {
            CGMutablePath::close_subpath(Some(path));
        }
    }
}
