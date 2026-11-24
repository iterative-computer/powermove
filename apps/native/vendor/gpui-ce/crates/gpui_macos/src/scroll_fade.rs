//! Match the GPUI content mask for AppKit slots that sit above its scene.
use objc2::{rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSColor, NSView};
use objc2_foundation::{NSArray, NSNumber, NSPoint, NSRect, NSSize};
use objc2_quartz_core::CAGradientLayer;
use std::cell::Cell;

#[derive(Default)]
pub(crate) struct ScrollFade {
    distances: Cell<[f64; 2]>,
    installed: Cell<Option<(NSRect, [f64; 2])>>,
}

impl ScrollFade {
    /// Takes its mask off `view` (it moved into a container that fades).
    pub(crate) fn clear(&self, view: &NSView) {
        if self.installed.take().is_some()
            && let Some(layer) = view.layer()
        {
            unsafe { layer.setMask(None) };
        }
    }

    pub(crate) fn stage(&self, distances: [f64; 2]) {
        self.distances.set(distances);
    }

    /// Geometry is in top-down GPUI coordinates. Give the mask the original
    /// clip's height and origin relative to the visible part of the control.
    pub(crate) fn place(&self, view: &NSView, visible: NSRect, clip: NSRect, apply: bool) -> bool {
        let [top, bottom] = self.distances.get();
        let near_top = top > 0. && visible.origin.y < clip.origin.y + top;
        let near_bottom = bottom > 0.
            && visible.origin.y + visible.size.height > clip.origin.y + clip.size.height - bottom;
        let next = (clip.size.height > 0. && visible.size.height > 0. && (near_top || near_bottom))
            .then(|| {
                let y = visible.origin.y + visible.size.height - clip.origin.y - clip.size.height;
                (NSRect::new(NSPoint::new(0., y), NSSize::new(visible.size.width, clip.size.height)), [top, bottom])
            });
        if self.installed.get() == next {
            return false;
        }
        if apply && let Some(layer) = view.layer() {
            let mask = next.map(|(frame, distances)| gradient(frame, distances));
            unsafe { layer.setMask(mask.as_deref().map(|mask| &**mask)); }
            self.installed.set(next);
        }
        true
    }
}

fn gradient(frame: NSRect, [top, bottom]: [f64; 2]) -> Retained<CAGradientLayer> {
    let gradient = CAGradientLayer::new();
    let color = |alpha| {
        let color = NSColor::colorWithSRGBRed_green_blue_alpha(0., 0., 0., alpha).CGColor();
        unsafe { Retained::retain(&*color as *const _ as *mut AnyObject).unwrap() }
    };
    let colors = [if top > 0. { 0. } else { 1. }, 1., 1., if bottom > 0. { 0. } else { 1. }].map(color);
    unsafe { gradient.setColors(Some(&NSArray::from_retained_slice(&colors))); }
    let top_end = (top / frame.size.height).min(1.);
    let bottom_start = (1. - bottom / frame.size.height).max(top_end);
    gradient.setLocations(Some(&NSArray::from_retained_slice(
        &[0., top_end, bottom_start, 1.].map(NSNumber::new_f64),
    )));
    gradient.setStartPoint(NSPoint::new(0.5, 1.));
    gradient.setEndPoint(NSPoint::new(0.5, 0.));
    gradient.setFrame(frame);
    gradient
}
