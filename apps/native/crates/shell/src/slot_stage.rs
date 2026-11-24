//! Where an AppKit slot painted now is staged (P-SLOT-REGION): inside a
//! scrolling region, in that region's content coordinates (its container
//! moves with the scroll, the slot does not); elsewhere in the window, with
//! its clip and edge fades.
use gpui::{Bounds, ContentMask, Pixels, Window};
use objc2_foundation::NSRect;

pub enum Place {
    Window {
        bounds: NSRect,
        clip: NSRect,
        fade: [f64; 2],
    },
    Region {
        id: usize,
        bounds: NSRect,
        clip: NSRect,
    },
}

/// How far an edge the region clips reaches in the slot's own clip: the
/// region's container clips there, at whatever offset it scrolls to.
const OPEN: f32 = 1.0e6;

pub fn place(bounds: Bounds<Pixels>, mask: ContentMask<Pixels>, window: &Window) -> Place {
    let rect = crate::native_control::ns_rect;
    let Some(region) = window.slot_region() else {
        return Place::Window {
            bounds: rect(bounds),
            clip: rect(mask.bounds),
            fade: [mask.fade_out.top, mask.fade_out.bottom].map(|v| f64::from(f32::from(v))),
        };
    };
    // Only clip edges inside the region's are the slot's own (a card inside
    // the scroll content); the rest are the region's, or a scroll layer's
    // recording past it.
    let (m, r) = (mask.bounds, region.clip);
    let edge = |own: Pixels, region: Pixels, inside: bool| {
        if inside {
            f32::from(own)
        } else if own <= region {
            -OPEN
        } else {
            OPEN
        }
    };
    let left = edge(m.left(), r.left(), m.left() > r.left());
    let top = edge(m.top(), r.top(), m.top() > r.top());
    let right = edge(m.right(), r.right(), m.right() < r.right());
    let bottom = edge(m.bottom(), r.bottom(), m.bottom() < r.bottom());
    let o = region.content_origin;
    let (ox, oy) = (f32::from(o.x), f32::from(o.y));
    let clip = Bounds::from_corners(
        gpui::point(gpui::px(left - ox), gpui::px(top - oy)),
        gpui::point(gpui::px(right - ox), gpui::px(bottom - oy)),
    );
    Place::Region {
        id: region.id,
        bounds: rect(Bounds::new(bounds.origin - o, bounds.size)),
        clip: rect(clip),
    }
}

/// [`place`] for a native text slot.
pub fn stage_text(
    slot: &gpui_macos::native_text::NativeTextSlot,
    bounds: Bounds<Pixels>,
    mask: ContentMask<Pixels>,
    window: &Window,
) {
    match place(bounds, mask, window) {
        Place::Window { bounds, clip, fade } => slot.stage_with_scroll_fade(bounds, clip, fade),
        Place::Region { id, bounds, clip } => slot.stage_in_region(id, bounds, clip),
    }
}
