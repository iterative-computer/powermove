//! Powermove P-SLOT-REGION: where content placed outside the scene (an AppKit
//! control's slot) sits inside the scrolling divs around it.
//!
//! Each scrolling div records, while it paints, a region: its frame (in its
//! parent region's content coordinates, or the window's for the outermost),
//! its edge fades and the offset its content was drawn at. A slot painted
//! inside records its bounds in the innermost region's content coordinates,
//! which a scroll does not change. The platform keeps one clipping container
//! per region and moves only that container's content by the region's offset,
//! with the frame that draws it: a wheel step moves one view, not one per
//! slot, and a scroll layer's replayed step (which paints nothing) moves its
//! container by the offset it replays at.
use crate::{Bounds, Pixels, Point, Window};
use std::{cell::RefCell, rc::Weak};

/// A scrolling div's region, for a slot painted inside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlotRegion {
    /// The region's id (stable while its div's scroll state lives).
    pub id: usize,
    /// Where the region's content coordinates start, in window coordinates,
    /// as this paint draws it.
    pub content_origin: Point<Pixels>,
    /// The region's own clip in window coordinates: a slot's clip edges at or
    /// past it are the region's (the container clips there).
    pub clip: Bounds<Pixels>,
}

/// One region as the platform places it, after a draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlotRegionGeometry {
    /// The region's id ([`SlotRegion::id`]).
    pub id: usize,
    /// The region it sits in; `None` for one placed in the window.
    pub parent: Option<usize>,
    /// The clip, in the parent region's content coordinates (the window's
    /// top-left points for an outermost region).
    pub frame: Bounds<Pixels>,
    /// Where the region's content coordinates start inside `frame`, at the
    /// offset this frame shows (the div's offset is already in it).
    pub origin: Point<Pixels>,
    /// Edge fades (top, bottom) of the clip, in points.
    pub fade: [Pixels; 2],
}

pub(crate) struct RegionRecord {
    cell: Weak<RefCell<Point<Pixels>>>,
    geometry: SlotRegionGeometry,
    /// The div's origin inside `frame` (the offset not added).
    inset: Point<Pixels>,
    /// The offset the content was drawn at in the frame last drawn.
    offset: Point<Pixels>,
    /// The draw frame that last painted it.
    painted: u64,
}

pub(crate) struct OpenRegion {
    id: usize,
    content_origin: Point<Pixels>,
    clip: Bounds<Pixels>,
}

impl Window {
    /// A scrolling div begins painting its children: its region opens.
    /// `bounds` is the div's own, `own_clip` its overflow clip (not yet
    /// intersected with its ancestors') and `offset` its scroll offset.
    pub(crate) fn begin_slot_region(
        &mut self,
        cell: &std::rc::Rc<RefCell<Point<Pixels>>>,
        bounds: Bounds<Pixels>,
        own_clip: Bounds<Pixels>,
        offset: Point<Pixels>,
    ) {
        let id = std::rc::Rc::as_ptr(cell) as usize;
        let mask = self.content_mask();
        let parent = self.slot_regions_open.last();
        // The outermost region keeps its ancestors' clip (they do not scroll).
        // A nested one keeps its own and those of the views between it and its
        // parent region (a card that clips, PM-696 review), but not the
        // parent's view: the parent's container clips that, and it moves in
        // the parent's content as it scrolls.
        let (clip, parent_id) = match parent {
            Some(p) => (own_clip.intersect(&within(mask.bounds, p.clip)), Some(p.id)),
            None => (mask.bounds, None),
        };
        let frame = match parent {
            Some(p) => Bounds::new(clip.origin - p.content_origin, clip.size),
            None => clip,
        };
        let window_frame_origin = clip.origin;
        let inset = bounds.origin - window_frame_origin;
        let fade = [mask.fade_out.top, mask.fade_out.bottom];
        self.slot_regions.insert(
            id,
            RegionRecord {
                cell: std::rc::Rc::downgrade(cell),
                geometry: SlotRegionGeometry {
                    id,
                    parent: parent_id,
                    frame,
                    origin: inset + offset,
                    fade,
                },
                inset,
                offset,
                painted: self.scroll_layer_frame,
            },
        );
        self.next_frame.slot_regions.push(id);
        self.slot_regions_open.push(OpenRegion {
            id,
            content_origin: bounds.origin + offset,
            clip: mask.bounds,
        });
    }

    pub(crate) fn end_slot_region(&mut self) {
        self.slot_regions_open.pop();
    }

    /// The innermost scrolling region being painted here, if any: a slot
    /// painted now places itself in its content coordinates.
    pub fn slot_region(&self) -> Option<SlotRegion> {
        self.slot_regions_open.last().map(|r| SlotRegion {
            id: r.id,
            content_origin: r.content_origin,
            clip: r.clip,
        })
    }

    /// After a draw: each living region at the offset this frame shows (a
    /// replayed scroll layer at the offset it replayed at), for the platform.
    pub(crate) fn finish_slot_regions(&mut self) {
        let frame = self.scroll_layer_frame;
        let layers = &self.scroll_layers;
        // Only the regions this frame draws: a retained scroll handle keeps a
        // region's state alive, not its place on screen (PM-696 review).
        let drawn: std::collections::HashSet<usize> =
            self.next_frame.slot_regions.iter().copied().collect();
        self.slot_regions.retain(|key, record| {
            if record.cell.strong_count() == 0 || !drawn.contains(key) {
                return false;
            }
            if record.painted != frame
                && let Some(layer) = layers.get(key)
                && layer.frame == frame
                && layer.shiftable
            {
                record.offset = layer.current();
            }
            record.geometry.origin = record.inset + record.offset;
            true
        });
        if self.slot_regions.is_empty() && !self.slot_regions_sent {
            return;
        }
        self.slot_regions_sent = !self.slot_regions.is_empty();
        let regions: Vec<_> = self.slot_regions.values().map(|r| r.geometry).collect();
        self.platform_window.set_slot_regions(&regions);
    }

    /// The regions the last draw handed to the platform (tests).
    #[doc(hidden)]
    pub fn slot_region_geometry(&self) -> Vec<SlotRegionGeometry> {
        self.slot_regions.values().map(|r| r.geometry).collect()
    }
}

/// `mask` with each edge at or past `region`'s opened outwards on its side:
/// what clips inside a region, apart from the region's own view.
fn within(mask: Bounds<Pixels>, region: Bounds<Pixels>) -> Bounds<Pixels> {
    let open = crate::px(1.0e6);
    let left = if mask.left() > region.left() { mask.left() } else { -open };
    let top = if mask.top() > region.top() { mask.top() } else { -open };
    let right = if mask.right() < region.right() { mask.right() } else { open };
    let bottom = if mask.bottom() < region.bottom() { mask.bottom() } else { open };
    Bounds::from_corners(crate::point(left, top), crate::point(right, bottom))
}
