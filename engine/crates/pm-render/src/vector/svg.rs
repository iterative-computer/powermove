//! SVG image layers (plan section 10, `image` (SVG); docs/engine/vector.md,
//! "SVG images").
//!
//! `compat` level 2 SVG images are drawn at output resolution: as vectors
//! (Vello coverage, the float paint pass) when the SVG is solid fills and
//! strokes inside its box, otherwise by resvg into the device rectangle
//! with the box and viewport as an antialiased clip. Level 1 SVG images
//! (today's `<img>` per size bucket) stay routed: resvg's bucket rasters
//! are outside the 2D parity class against Chromium's
//! (`pm-vector/tests/svg.rs`). SVGs with text stay routed at every level
//! (usvg's text feature is off; SVG text goes through `pm-text`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use pm_eval::transform::Mat2d;
use pm_vector::kurbo::{Affine, Point, Rect};
use pm_vector::render::scene::PixelRect;
use pm_vector::render::svg::SvgDocument;

use super::{VectorQuad, VectorSource};

/// A file's identity for caching: its length and modification time (ns).
pub type Stamp = (u64, u128);

/// The stamp of `path`.
pub fn stamp(path: &Path) -> Result<Stamp, String> {
    let m = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let modified = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    Ok((m.len(), modified))
}

struct Cached {
    path: PathBuf,
    stamp: Stamp,
    doc: Arc<SvgDocument>,
}

/// Parsed documents, most recently used last.
static DOCUMENTS: Mutex<Vec<Cached>> = Mutex::new(Vec::new());
const MAX_DOCUMENTS: usize = 16;

/// The parsed SVG at `path` (cached by path and stamp).
pub fn document(path: &Path) -> Result<(Arc<SvgDocument>, Stamp), String> {
    let stamp = stamp(path)?;
    let mut docs = DOCUMENTS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = docs.iter().position(|c| c.path == path && c.stamp == stamp) {
        let c = docs.remove(i);
        let doc = c.doc.clone();
        docs.push(c);
        return Ok((doc, stamp));
    }
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // usvg reads UTF-8 only; Chromium honors the declared encoding.
    let data = pm_media::image::svg::utf8_source(&data);
    let doc = Arc::new(SvgDocument::parse(&data).map_err(|e| e.to_string())?);
    docs.retain(|c| c.path != path);
    if docs.len() >= MAX_DOCUMENTS {
        docs.remove(0);
    }
    docs.push(Cached {
        path: path.to_path_buf(),
        stamp,
        doc: doc.clone(),
    });
    Ok((doc, stamp))
}

/// How a level 2 SVG image layer is drawn.
#[derive(Debug)]
pub enum SvgPlan {
    /// The SVG has text: routed.
    Text,
    /// Drawn (or nothing to draw).
    Drawn(Option<Box<VectorQuad>>),
}

/// The image box as the legacy quad places it: `bw` × `bh` layer units
/// centered on the layer origin, sampling the texture window `uv`
/// (`[u0, v0, us, vs]`, from `fit`). The SVG's user space spans the texture
/// (`drawImage` stretches it), so user units map to layer units by
/// `(x / W − u0) / us · bw − bw / 2`.
pub fn plan_svg(
    path: &Path,
    bw: f64,
    bh: f64,
    uv: [f64; 4],
    world: &Mat2d,
    target: PixelRect,
) -> Result<SvgPlan, String> {
    let (doc, stamp) = document(path)?;
    if doc.has_text() {
        return Ok(SvgPlan::Text);
    }
    let (w, h) = doc.size();
    let [u0, v0, us, vs] = uv;
    if !(bw > 0.0 && bh > 0.0 && us > 0.0 && vs > 0.0) {
        return Ok(SvgPlan::Drawn(None));
    }
    let to_layer = Affine::translate((-bw / 2.0 - u0 * bw / us, -bh / 2.0 - v0 * bh / vs))
        * Affine::scale_non_uniform(bw / (us * w), bh / (vs * h));
    let m = Affine::new(*world) * to_layer;
    // What shows: the SVG viewport within the box's texture window.
    let clip = Rect::new(
        (u0 * w).max(0.0),
        (v0 * h).max(0.0),
        ((u0 + us) * w).min(w),
        ((v0 + vs) * h).min(h),
    );
    if clip.width() <= 0.0 || clip.height() <= 0.0 {
        return Ok(SvgPlan::Drawn(None));
    }
    let Some(bounds) = doc.user_bounds() else {
        return Ok(SvgPlan::Drawn(None));
    };
    let inside = bounds.x0 >= clip.x0
        && bounds.y0 >= clip.y0
        && bounds.x1 <= clip.x1
        && bounds.y1 <= clip.y1;
    if inside && let Some(scene) = doc.scene(m) {
        return Ok(SvgPlan::Drawn(scene.pixel_bounds(target).map(|rect| {
            Box::new(VectorQuad::Device {
                source: VectorSource::Scene { scene, rect },
                rect,
            })
        })));
    }
    let visible = bounds.intersect(clip);
    if visible.width() <= 0.0 || visible.height() <= 0.0 {
        return Ok(SvgPlan::Drawn(None));
    }
    let Some(rect) = PixelRect::round_out(m.transform_rect_bbox(visible)).intersect(target) else {
        return Ok(SvgPlan::Drawn(None));
    };
    let corners = [
        Point::new(clip.x0, clip.y0),
        Point::new(clip.x1, clip.y0),
        Point::new(clip.x1, clip.y1),
        Point::new(clip.x0, clip.y1),
    ]
    .map(|p| m * p);
    Ok(SvgPlan::Drawn(Some(Box::new(VectorQuad::Device {
        source: VectorSource::SvgRaster {
            path: path.to_path_buf(),
            stamp,
            transform: m.as_coeffs(),
            rect,
            clip: corners.map(|p| [p.x, p.y]),
        },
        rect,
    }))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(name: &str, svg: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pm-render-svg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, svg).unwrap();
        p
    }

    const SOLID: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50"><rect x="10" y="10" width="80" height="30" fill="#4C8DFF"/></svg>"##;

    #[test]
    fn solid_svgs_inside_their_box_are_vectors() {
        let p = write("solid.svg", SOLID);
        // A 200 × 100 box at (100, 50) in a 400 × 200 target.
        let world = [1.0, 0.0, 0.0, 1.0, 200.0, 100.0];
        let target = PixelRect::new(0, 0, 400, 200);
        let SvgPlan::Drawn(Some(quad)) =
            plan_svg(&p, 200.0, 100.0, [0.0, 0.0, 1.0, 1.0], &world, target).unwrap()
        else {
            panic!("drawn");
        };
        let VectorQuad::Device { source, rect } = *quad else {
            panic!("device")
        };
        assert!(matches!(source, VectorSource::Scene { .. }));
        // The rect spans user (10..90, 10..40) × 2 from (100, 50).
        assert_eq!(rect, PixelRect::new(120, 70, 160, 60));
    }

    #[test]
    fn cropped_and_text_svgs() {
        let p = write("crop.svg", SOLID);
        let world = [1.0, 0.0, 0.0, 1.0, 200.0, 100.0];
        let target = PixelRect::new(0, 0, 400, 200);
        // `cover` in a square box crops the sides: resvg with a clip.
        let uv = crate::plan::fit_uv("cover", 100.0, 50.0, 100.0, 100.0);
        let SvgPlan::Drawn(Some(quad)) = plan_svg(&p, 100.0, 100.0, uv, &world, target).unwrap()
        else {
            panic!("drawn");
        };
        let VectorQuad::Device { source, .. } = *quad else {
            panic!("device")
        };
        let VectorSource::SvgRaster { clip, .. } = source else {
            panic!("raster")
        };
        // The clip is the box: 100 × 100 centered at (200, 100).
        assert_eq!(clip[0], [150.0, 50.0]);
        assert_eq!(clip[2], [250.0, 150.0]);
        let t = write(
            "text.svg",
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><text x="1" y="8">a</text></svg>"##,
        );
        assert!(matches!(
            plan_svg(&t, 10.0, 10.0, [0.0, 0.0, 1.0, 1.0], &world, target).unwrap(),
            SvgPlan::Text
        ));
    }
}
