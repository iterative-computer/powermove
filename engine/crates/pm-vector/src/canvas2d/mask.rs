//! Today's bezier path mask route on the emulated Canvas2D context
//! (`applyPathMasks` in `legacy/gl/compositor.ts`; docs/engine/composite.md
//! "Path masks").
//!
//! Today every enabled mask of a layer is filled white on its own canvas
//! of the output size (the *item*), and each item is drawn onto one
//! coverage canvas through `ctx.filter = blur(σ px)` with `source-over`
//! (add) or `destination-out` (subtract). [`item`] produces an item's
//! coverage; [`super::filter::BlurPlan`] plans Skia's blur of it, which
//! pm-render runs on the GPU.

use super::context::{Context, Unsupported};
use super::shape::trace;
use crate::mask::{MaskMode, MaskOutline, PathMask, PathMasks};

/// One item canvas: a mask filled white, as coverage (the canvas's alpha,
/// equal to every premultiplied channel), top-down rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub width: u32,
    pub height: u32,
    pub alpha: Vec<u8>,
}

/// The item canvas of `mask` for a layer whose scaled world matrix is
/// `world`, on a `width` × `height` canvas:
///
/// ```js
/// c.transform(...world); c.fillStyle = 'white';
/// c.translate(x, y); c.rotate(rotation);
/// // a path: c.translate(v.x, v.y); c.rotate(v.rotation); c.scale(…); tracePath(c, …)
/// // else:   c.beginPath(); c.ellipse(…) or c.rect(…)
/// c.fill();
/// ```
pub fn item(mask: &PathMask, world: [f64; 6], width: u32, height: u32) -> Result<Item, Unsupported> {
    let mut c = Context::new(width, height);
    c.transform(world);
    c.set_fill_style("white");
    for op in &mask.ops {
        match *op {
            crate::canvas::TransformOp::Transform(m) => c.transform(m),
            crate::canvas::TransformOp::Translate(x, y) => c.translate(x, y),
            crate::canvas::TransformOp::Rotate(r) => c.rotate(r),
            crate::canvas::TransformOp::Scale(x, y) => c.scale(x, y),
        }
    }
    match mask.outline {
        MaskOutline::Path => trace(&mut c, &mask.path),
        MaskOutline::Ellipse { rx, ry } => {
            c.begin_path();
            // Absolute radii never throw.
            let _ = c.path.ellipse(0.0, 0.0, rx, ry, 0.0, core::f64::consts::TAU, false);
        }
        MaskOutline::Rect { w, h } => {
            c.begin_path();
            c.path.rect(-w / 2.0, -h / 2.0, w, h);
        }
    }
    c.fill()?;
    let rgba = c.finish();
    Ok(Item {
        width,
        height,
        alpha: rgba.chunks_exact(4).map(|p| p[3]).collect(),
    })
}

/// How an item is drawn onto the coverage canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Composite {
    /// `source-over` (an add mask).
    SourceOver,
    /// `destination-out` (a subtract mask).
    DestinationOut,
}

impl From<MaskMode> for Composite {
    fn from(m: MaskMode) -> Self {
        match m {
            MaskMode::Add => Composite::SourceOver,
            MaskMode::Subtract => Composite::DestinationOut,
        }
    }
}

/// The blur `ctx.filter` applies to an item: CSS `blur(σ px)` with
/// `σ = max(0, feather) · hypot(world[0], world[1]) / 2`, as the string
/// round trip leaves it (a JavaScript number printed and parsed back, then
/// held as a float).
pub fn sigma(mask: &PathMask, world: [f64; 6]) -> f32 {
    let s = mask.feather * crate::js::hypot(world[0], world[1]) / 2.0;
    if s.is_finite() { s as f32 } else { f32::NAN }
}

/// `src` drawn onto `dst` (8-bit premultiplied coverage) with `op`, as the
/// GPU's unorm8 blending does: `src + dst · (1 − src)` or `dst · (1 − src)`.
pub fn composite(dst: &mut [u8], src: &[u8], op: Composite) {
    for (d, s) in dst.iter_mut().zip(src) {
        let (sf, df) = (f32::from(*s) / 255.0, f32::from(*d) / 255.0);
        let v = match op {
            Composite::SourceOver => sf + df * (1.0 - sf),
            Composite::DestinationOut => df * (1.0 - sf),
        };
        *d = (v * 255.0 + 0.5) as u8;
    }
}

/// The coverage canvas's starting state: `fillRect(0, 0, W, H)` in white
/// when no mask adds, else transparent.
pub fn start(masks: &PathMasks, width: u32, height: u32) -> Vec<u8> {
    vec![if masks.start_covered { 255 } else { 0 }; (width * height) as usize]
}
