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
    let o = region.content_origin;
    let clip = open_clip(mask.bounds, region.clip);
    let clip = Bounds::new(clip.origin - o, clip.size);
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

/// A slot's clip inside its region: only edges strictly inside the region's
/// are the slot's own (a card inside the scroll content); an edge at or past
/// the region's (the region's own, or a scroll layer's recording past it) is
/// opened outwards on its side, as the region's container clips there.
fn open_clip(mask: Bounds<Pixels>, region: Bounds<Pixels>) -> Bounds<Pixels> {
    let open = gpui::px(OPEN);
    let left = if mask.left() > region.left() {
        mask.left()
    } else {
        -open
    };
    let top = if mask.top() > region.top() {
        mask.top()
    } else {
        -open
    };
    let right = if mask.right() < region.right() {
        mask.right()
    } else {
        open
    };
    let bottom = if mask.bottom() < region.bottom() {
        mask.bottom()
    } else {
        open
    };
    Bounds::from_corners(gpui::point(left, top), gpui::point(right, bottom))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px, size};
    /// PM-696: a control as wide as its region (a section header's
    /// checkbox row) kept its right edge at the region's and was opened to
    /// the wrong side: its clip was empty and the control never showed.
    #[test]
    fn a_clip_edge_on_the_region_edge_opens_outwards_on_its_side() {
        let region = Bounds::new(point(px(1092.), px(90.)), size(px(340.), px(500.)));
        let same = open_clip(region, region);
        assert!(same.left() < px(-1e5) && same.top() < px(-1e5));
        assert!(same.right() > px(1e5) && same.bottom() > px(1e5));
        // A card inside the content keeps its own edges.
        let card = Bounds::new(point(px(1100.), px(120.)), size(px(200.), px(40.)));
        assert_eq!(open_clip(card, region), card);
        // A scroll layer's recording past the region opens too.
        let recording = Bounds::new(point(px(1092.), px(-410.)), size(px(340.), px(1500.)));
        let r = open_clip(recording, region);
        assert!(r.top() < px(-1e5) && r.bottom() > px(1e5));
    }
}
