//! Sampling and antialiasing (plan section 8, "Sampling and
//! antialiasing"; section 2's antialiasing bar).
//!
//! Level 1 content (`compat` 1, every layer created before cutover) keeps
//! today's sampling: no mipmaps, bilinear, hard edges at the UV window, the
//! rasterizer's pixel-center coverage (`composite.wgsl`, `fs_draw`).
//!
//! Level 2 raster content (image, video, and precomp layers with `compat`
//! ≥ 2) is resampled with a declared kernel and analytic coverage edges:
//!
//! - **Playback** ([`Sampling::Playback`], frames while playing and scrub
//!   frames): trilinear filtering over a box-filtered mip pyramid, the
//!   level of detail from the exact Jacobian of the mapping.
//! - **Final** ([`Sampling::Final`], paused frames, exports, captures):
//!   Lanczos-3 in the principal axes of the mapping, its support widened by
//!   the minification (`max(1, σ)` per axis, σ the singular values of the
//!   output-to-source Jacobian) and taken from the pyramid level that keeps
//!   at most 4 source texels per kernel unit, with the anti-ringing clamp
//!   to the minimum and maximum of the kernel's central texels, in
//!   premultiplied space.
//! - **Edges.** Each output pixel's coverage is the exact area of the pixel
//!   square inside the layer's quad (the projected quad clipped at the
//!   camera plane), so a quad edge is antialiased analytically instead of
//!   at pixel centers.
//!
//! The draw itself (`shaders/resample.wgsl`, [`gpu`]) covers the quad's
//! pixel bounds and maps each pixel back through the inverse homography,
//! so affine and 2.5D quads take the same path.

use pm_eval::{JsResult, Scope, node};
use pm_model::Node;

pub mod gpu;

/// Which resampler level 2 content uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Sampling {
    /// Trilinear (mipmapped) during playback.
    Playback,
    /// Lanczos-3 with the anti-ringing clamp: paused frames and exports.
    #[default]
    Final,
}

/// The layer's content is `compat` level 2 or later.
pub fn is_level2(layer: &Node) -> bool {
    let c = node::to_number(node::member(Some(layer), "compat"));
    c >= 2.0
}

/// Level 2 layer types whose raster content this module samples (vectors,
/// text, and solids at level 2 belong to their own renderers).
pub fn samples_layer_type(ty: &str) -> bool {
    matches!(ty, "image" | "video" | "precomp")
}

/// The largest edge of a continuously rasterized precomp target.
pub const MAX_TARGET: f64 = 16384.0;

/// Output pixels per composition pixel of a level 2 precomp's quad where
/// it is densest, per axis of the quad: the column lengths of its world
/// matrix scaled to output pixels, or for a 2.5D plane the largest over the
/// quad's corners. Capped so the target stays within [`MAX_TARGET`]; a
/// plane with a corner behind the camera keeps the output scale.
#[allow(clippy::too_many_arguments)]
pub fn precomp_density<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
    scale: (f64, f64),
    w: f64,
    h: f64,
) -> JsResult<(f64, f64)> {
    let offset = scope.size_anchor_offset(layer, time)?;
    let (ax, ay) = (-offset.x, -offset.y);
    let density = match crate::space25d::projected(scope, layer, time, scale, w, h, ax, ay)? {
        Some(plane) => match crate::space25d::corner_density(&plane) {
            Some((du, dv)) => (du / w.max(1e-9), dv / h.max(1e-9)),
            None => scale,
        },
        None => {
            let m = scope.world_matrix(layer, time)?;
            (
                (m[0] * scale.0).hypot(m[1] * scale.1),
                (m[2] * scale.0).hypot(m[3] * scale.1),
            )
        }
    };
    let cap = |d: f64, size: f64| {
        if d.is_finite() {
            d.min(MAX_TARGET / size.max(1.0))
        } else {
            0.0
        }
    };
    Ok((cap(density.0, w), cap(density.1, h)))
}
