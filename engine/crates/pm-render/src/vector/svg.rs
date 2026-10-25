//! SVG image layers (plan section 10, `image` (SVG); docs/engine/vector.md,
//! "SVG images").
//!
//! Every SVG image (any `compat` level) is drawn at output resolution: the
//! plan places the SVG's user space in the image box as the image quad
//! places a bitmap (`fit`, anchor at the box center) and clips it to the
//! box and the SVG's viewport; the cache draws it in float on the GPU, or
//! resvg ([`super::cache`]).

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
) -> Result<Option<Box<VectorQuad>>, String> {
    let (doc, stamp) = document(path)?;
    let (w, h) = doc.size();
    let [u0, v0, us, vs] = uv;
    if !(bw > 0.0 && bh > 0.0 && us > 0.0 && vs > 0.0) {
        return Ok(None);
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
        return Ok(None);
    }
    let Some(bounds) = doc.user_bounds() else {
        return Ok(None);
    };
    let visible = bounds.intersect(clip);
    if visible.width() <= 0.0 || visible.height() <= 0.0 {
        return Ok(None);
    }
    let Some(rect) = PixelRect::round_out(m.transform_rect_bbox(visible)).intersect(target) else {
        return Ok(None);
    };
    let corners = [
        Point::new(clip.x0, clip.y0),
        Point::new(clip.x1, clip.y0),
        Point::new(clip.x1, clip.y1),
        Point::new(clip.x0, clip.y1),
    ]
    .map(|p| m * p);
    Ok(Some(Box::new(VectorQuad::Device {
        source: VectorSource::SvgRaster {
            path: path.to_path_buf(),
            stamp,
            transform: m.as_coeffs(),
            rect,
            clip: corners.map(|p| [p.x, p.y]),
        },
        rect,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestSvg(PathBuf);

    impl std::ops::Deref for TestSvg {
        type Target = std::path::Path;
        fn deref(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for TestSvg {
        fn drop(&mut self) {
            std::fs::remove_dir_all(self.0.parent().unwrap()).unwrap();
        }
    }
    fn write(name: &str, svg: &str) -> TestSvg {
        let dir = std::env::temp_dir().join(format!("pm-render-svg-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, svg).unwrap();
        TestSvg(p)
    }

    const SOLID: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50"><rect x="10" y="10" width="80" height="30" fill="#4C8DFF"/></svg>"##;

    fn planned(
        p: &Path,
        bw: f64,
        bh: f64,
        uv: [f64; 4],
        world: &Mat2d,
    ) -> (PixelRect, [f64; 6], [[f64; 2]; 4]) {
        let target = PixelRect::new(0, 0, 400, 200);
        let quad = plan_svg(p, bw, bh, uv, world, target)
            .unwrap()
            .expect("drawn");
        let VectorQuad::Device {
            source:
                VectorSource::SvgRaster {
                    transform,
                    rect: r,
                    clip,
                    ..
                },
            rect,
        } = *quad
        else {
            panic!("an SVG raster")
        };
        assert_eq!(r, rect);
        (rect, transform, clip)
    }

    #[test]
    fn the_svg_fills_the_box_at_output_resolution() {
        let p = write("solid.svg", SOLID);
        // A 200 × 100 box centered at (200, 100) in a 400 × 200 target: the
        // 100 × 50 user space spans it at 2×.
        let world = [1.0, 0.0, 0.0, 1.0, 200.0, 100.0];
        let (rect, transform, clip) = planned(&p, 200.0, 100.0, [0.0, 0.0, 1.0, 1.0], &world);
        // The rect spans user (10..90, 10..40) × 2 from (100, 50).
        assert_eq!(rect, PixelRect::new(120, 70, 160, 60));
        assert_eq!(transform, [2.0, 0.0, 0.0, 2.0, 100.0, 50.0]);
        assert_eq!(
            clip,
            [[100.0, 50.0], [300.0, 50.0], [300.0, 150.0], [100.0, 150.0]]
        );
    }

    #[test]
    fn cover_crops_to_the_box() {
        let p = write("crop.svg", SOLID);
        let world = [1.0, 0.0, 0.0, 1.0, 200.0, 100.0];
        // `cover` in a square box crops the sides: the clip is the box,
        // 100 × 100 centered at (200, 100).
        let uv = crate::plan::fit_uv("cover", 100.0, 50.0, 100.0, 100.0);
        let (rect, _, clip) = planned(&p, 100.0, 100.0, uv, &world);
        assert_eq!(clip[0], [150.0, 50.0]);
        assert_eq!(clip[2], [250.0, 150.0]);
        assert_eq!(rect, PixelRect::new(150, 70, 100, 60));
    }

    #[test]
    fn text_is_planned_like_any_content() {
        let t = write(
            "text.svg",
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><text x="1" y="8" font-size="8">a</text></svg>"##,
        );
        let world = [1.0, 0.0, 0.0, 1.0, 200.0, 100.0];
        planned(&t, 10.0, 10.0, [0.0, 0.0, 1.0, 1.0], &world);
    }
}
