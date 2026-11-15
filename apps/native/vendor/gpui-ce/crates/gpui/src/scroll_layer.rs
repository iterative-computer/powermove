//! Powermove P-SCROLL-LAYER: a scroll step that changes only the offset
//! repaints at the new offset without rendering or laying out the view again.
//!
//! A scrolling div that declares its content independent of the offset
//! ([`crate::InteractiveElement::scroll_layer`]) records what its children
//! drew, at the offset they were drawn at, beyond its own edges by about its
//! size each way along the axes it scrolls. A wheel step that stays inside
//! that recording moves the offset and asks for a frame without notifying the
//! view: the view's cached frame is reused, and its recorded content is
//! replayed shifted to the new offset and clipped to the div again (the scene
//! and the hitboxes, so hover and clicks land on what is drawn). A step past
//! the recording, a focused text input, a deferred draw or a transformed
//! glyph inside it, or the gesture ending notifies the view as before, which
//! records the content again.
use crate::{
    Bounds, ContentMask, Edges, EntityId, Hitbox, Pixels, Point, ScaledPixels,
    scene::{PaintOperation, Primitive, TransformationMatrix},
};
use std::{cell::RefCell, ops::Range, rc::Rc};

/// A layer's key: the address of its div's offset cell, which lives as long
/// as the div's element state.
pub(crate) type ScrollLayerKey = usize;

pub(crate) fn key(offset: &Rc<RefCell<Point<Pixels>>>) -> ScrollLayerKey {
    Rc::as_ptr(offset) as usize
}

/// A scroll layer, for content drawn outside the scene (an AppKit control's
/// slot) to follow the layer's offset: see [`crate::Window::scroll_layer`]
/// and [`crate::Window::scroll_layer_shift`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScrollLayerId(pub(crate) ScrollLayerKey);

/// Off with `PM_SCROLL_LAYER=0` (a measurement: every step renders the view
/// and truncations are shaped each time, as before the layer).
pub(crate) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("PM_SCROLL_LAYER").is_none_or(|v| v != "0"))
}

/// Recorded this many frames in a row with no step between, a layer rests
/// (its div draws as a plain scroller) for `REST` frames.
pub(crate) const IDLE_RECORDS: u32 = 3;
pub(crate) const REST: u64 = 60;

/// How far past its edges a layer records, in its own sizes.
pub(crate) const REACH: f32 = 1.0;

/// How long after the last offset-only step the view draws again in full
/// (hover, accessibility and text input positions follow the content then).
pub(crate) const SETTLE: std::time::Duration = std::time::Duration::from_millis(150);

pub(crate) struct ScrollLayer {
    /// The view whose cached frame holds the layer.
    pub view: EntityId,
    pub offset: Rc<RefCell<Point<Pixels>>>,
    /// The offset the content was drawn at.
    pub recorded: Point<Pixels>,
    /// How far the offset may move from `recorded` and stay inside the recording.
    pub reach: Point<Pixels>,
    /// The least and most offset (both axes; the offset is zero or negative).
    pub max: Point<Pixels>,
    /// The div's own clip, where the content shows.
    pub clip: ContentMask<Pixels>,
    /// The content as drawn, clipped only to the recording's reach.
    pub ops: Vec<PaintOperation>,
    pub hitboxes: Vec<Hitbox>,
    /// Where the layer's hitboxes sit in the frame last drawn.
    pub hitbox_range: Range<usize>,
    /// False when something inside can't be shifted (see the module docs).
    pub shiftable: bool,
    /// What made it unshiftable, for the draw trace.
    pub why: &'static str,
    /// The frame that last drew the layer.
    pub frame: u64,
    /// Bumped by every offset-only step; the settle timer acts only on the last.
    pub generation: u64,
    /// `generation` when last recorded, and how many frames in a row it was
    /// recorded with no step between: a layer its view draws again anyway
    /// (a panel that redraws while it scrolls) only pays for recording.
    pub recorded_at: u64,
    pub idle_records: u32,
}

impl ScrollLayer {
    /// The offset now, clamped to the layer's extent.
    pub fn current(&self) -> Point<Pixels> {
        *self.offset.borrow()
    }

    /// Whether `offset` keeps the visible content inside the recording.
    pub fn covers(&self, offset: Point<Pixels>) -> bool {
        (offset.x - self.recorded.x).abs() <= self.reach.x
            && (offset.y - self.recorded.y).abs() <= self.reach.y
    }

    /// The recorded hitboxes shifted to the current offset and clipped.
    pub fn shifted_hitboxes(&self) -> impl Iterator<Item = Hitbox> + '_ {
        let delta = self.current() - self.recorded;
        self.hitboxes.iter().map(move |hitbox| {
            let mut hitbox = hitbox.clone();
            hitbox.bounds.origin += delta;
            if let Some(fragments) = &hitbox.fragments {
                hitbox.fragments = Some(
                    fragments
                        .iter()
                        .map(|b| Bounds::new(b.origin + delta, b.size))
                        .collect(),
                );
            }
            hitbox.content_mask.bounds.origin += delta;
            hitbox.content_mask = hitbox.content_mask.intersect(&self.clip);
            hitbox
        })
    }

    /// The recorded content shifted to the current offset and clipped to the
    /// div, ready to insert into a scene.
    pub fn shifted_ops(&self, scale_factor: f32) -> impl Iterator<Item = PaintOperation> + '_ {
        let delta = (self.current() - self.recorded).scale(scale_factor);
        let clip = self.clip.scale(scale_factor);
        self.ops.iter().filter_map(move |op| match op {
            PaintOperation::Primitive(primitive) => {
                let mut primitive = primitive.clone();
                shift(&mut primitive, delta);
                clip_to(&mut primitive, &clip);
                Some(PaintOperation::Primitive(primitive))
            }
            PaintOperation::StartLayer(bounds) => Some(PaintOperation::StartLayer(
                Bounds::new(bounds.origin + delta, bounds.size).intersect(&clip.bounds),
            )),
            PaintOperation::EndLayer => Some(PaintOperation::EndLayer),
            // A nested layer's content was recorded into this one at its own
            // offset; the markers carry no content.
            PaintOperation::ScrollLayerStart(_) | PaintOperation::ScrollLayerEnd => None,
        })
    }
}

/// Whether a primitive can be shifted by moving its bounds (a glyph drawn
/// with a rotation carries its own placement).
pub(crate) fn shiftable(op: &PaintOperation) -> bool {
    match op {
        PaintOperation::Primitive(Primitive::MonochromeSprite(s)) => {
            s.transformation == TransformationMatrix::unit()
        }
        PaintOperation::Primitive(Primitive::SubpixelSprite(s)) => {
            s.transformation == TransformationMatrix::unit()
        }
        _ => true,
    }
}

fn shift_bounds(bounds: &mut Bounds<ScaledPixels>, delta: Point<ScaledPixels>) {
    bounds.origin.x += delta.x;
    bounds.origin.y += delta.y;
}

fn shift(primitive: &mut Primitive, delta: Point<ScaledPixels>) {
    match primitive {
        Primitive::Shadow(s) => {
            shift_bounds(&mut s.bounds, delta);
            shift_bounds(&mut s.element_bounds, delta);
            shift_bounds(&mut s.content_mask.bounds, delta);
        }
        Primitive::Quad(q) => {
            shift_bounds(&mut q.bounds, delta);
            shift_bounds(&mut q.content_mask.bounds, delta);
        }
        Primitive::Path(p) => {
            shift_bounds(&mut p.bounds, delta);
            shift_bounds(&mut p.content_mask.bounds, delta);
            for v in &mut p.vertices {
                v.xy_position.x += delta.x;
                v.xy_position.y += delta.y;
                shift_bounds(&mut v.content_mask.bounds, delta);
            }
        }
        Primitive::Underline(u) => {
            shift_bounds(&mut u.bounds, delta);
            shift_bounds(&mut u.content_mask.bounds, delta);
        }
        Primitive::MonochromeSprite(s) => {
            shift_bounds(&mut s.bounds, delta);
            shift_bounds(&mut s.content_mask.bounds, delta);
        }
        Primitive::SubpixelSprite(s) => {
            shift_bounds(&mut s.bounds, delta);
            shift_bounds(&mut s.content_mask.bounds, delta);
        }
        Primitive::PolychromeSprite(s) => {
            shift_bounds(&mut s.bounds, delta);
            shift_bounds(&mut s.content_mask.bounds, delta);
        }
        Primitive::Surface(s) => {
            shift_bounds(&mut s.bounds, delta);
            shift_bounds(&mut s.content_mask.bounds, delta);
        }
        Primitive::BackdropFilter(f) => {
            shift_bounds(&mut f.bounds, delta);
            shift_bounds(&mut f.content_mask.bounds, delta);
        }
        Primitive::FilterBoundary(b) => {
            shift_bounds(&mut b.bounds, delta);
            shift_bounds(&mut b.content_mask.bounds, delta);
        }
    }
}

fn clip_to(primitive: &mut Primitive, clip: &ContentMask<ScaledPixels>) {
    let mask = match primitive {
        Primitive::Shadow(s) => &mut s.content_mask,
        Primitive::Quad(q) => &mut q.content_mask,
        Primitive::Path(p) => {
            for v in &mut p.vertices {
                v.content_mask = intersect(&v.content_mask, clip);
            }
            &mut p.content_mask
        }
        Primitive::Underline(u) => &mut u.content_mask,
        Primitive::MonochromeSprite(s) => &mut s.content_mask,
        Primitive::SubpixelSprite(s) => &mut s.content_mask,
        Primitive::PolychromeSprite(s) => &mut s.content_mask,
        Primitive::Surface(s) => &mut s.content_mask,
        Primitive::BackdropFilter(f) => &mut f.content_mask,
        Primitive::FilterBoundary(b) => &mut b.content_mask,
    };
    *mask = intersect(mask, clip);
}

/// [`ContentMask::intersect`] in device pixels: each edge keeps the fade of
/// the mask that clips there.
pub(crate) fn intersect(
    a: &ContentMask<ScaledPixels>,
    b: &ContentMask<ScaledPixels>,
) -> ContentMask<ScaledPixels> {
    let fade = |a_edge: f32, b_edge: f32, a_fade, b_fade, a_closer: fn(f32, f32) -> bool| {
        if a_closer(a_edge, b_edge) {
            a_fade
        } else if a_closer(b_edge, a_edge) {
            b_fade
        } else if a_fade > b_fade {
            a_fade
        } else {
            b_fade
        }
    };
    let (ab, bb) = (&a.bounds, &b.bounds);
    ContentMask {
        bounds: ab.intersect(bb),
        fade_out: Edges {
            top: fade(ab.top().0, bb.top().0, a.fade_out.top, b.fade_out.top, |x, y| x > y),
            right: fade(ab.right().0, bb.right().0, a.fade_out.right, b.fade_out.right, |x, y| x < y),
            bottom: fade(ab.bottom().0, bb.bottom().0, a.fade_out.bottom, b.fade_out.bottom, |x, y| x < y),
            left: fade(ab.left().0, bb.left().0, a.fade_out.left, b.fade_out.left, |x, y| x > y),
        },
    }
}

/// The recording's mask: the div's clip grown by its reach along the axes it
/// scrolls, without the clip's fades (they apply when the content is shown).
pub(crate) fn recording_mask(clip: &ContentMask<Pixels>, reach: Point<Pixels>) -> ContentMask<Pixels> {
    let b = clip.bounds;
    ContentMask {
        bounds: Bounds::from_corners(
            Point::new(b.left() - reach.x, b.top() - reach.y),
            Point::new(b.right() + reach.x, b.bottom() + reach.y),
        ),
        fade_out: Edges::default(),
    }
}
