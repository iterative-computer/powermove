//! SVG stills through usvg and resvg (plan section 13 "Images").
//!
//! An SVG is vector data: it is rasterized at the size the renderer asks
//! for, as today's compositor rasterizes it at a size chosen from the
//! layer's display transform (`svgRasterDimensions`), stretched to that
//! size from its intrinsic size as `drawImage` stretches it. Like an SVG
//! drawn through an `<img>`, it never loads anything outside the file:
//! embedded (`data:`) images render, external references do not. Text uses
//! the system fonts. Output is 8-bit premultiplied RGBA in sRGB (E).

use super::{ImageBackend, ImageColor, ImageFormat, Rgba, StillImage, pool_rgba};
use crate::error::{MediaError, Result};
use crate::pool::FramePool;
use pm_protocol::color::{AlphaMode, Transfer};
use resvg::{tiny_skia, usvg};
use std::path::Path;
use std::sync::{Arc, OnceLock};

/// The largest raster side (today's limit, `svgRasterDimensions`).
pub const MAX_DIMENSION: u32 = 8192;

fn fonts() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    })
    .clone()
}

fn options() -> usvg::Options<'static> {
    usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
            // No external files or URLs.
            resolve_string: Box::new(|_, _| None),
        },
        fontdb: fonts(),
        ..usvg::Options::default()
    }
}

/// A parsed SVG document.
pub struct SvgDocument {
    tree: usvg::Tree,
}

impl SvgDocument {
    pub fn open(path: &Path) -> Result<SvgDocument> {
        let data = std::fs::read(path)?;
        Self::from_data(&data)
    }

    pub fn from_data(data: &[u8]) -> Result<SvgDocument> {
        let tree = usvg::Tree::from_data(data, &options())
            .map_err(|e| MediaError::Image(format!("SVG: {e}")))?;
        Ok(SvgDocument { tree })
    }

    /// The intrinsic size in CSS pixels (width and height, or the viewBox).
    pub fn size(&self) -> (f64, f64) {
        let s = self.tree.size();
        (f64::from(s.width()), f64::from(s.height()))
    }

    /// Rasterizes the whole document stretched to `width` × `height`.
    pub fn render(&self, width: u32, height: u32, pool: &FramePool) -> Result<StillImage> {
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(MediaError::invalid(format!(
                "SVG raster of {width}×{height} is outside 1..{MAX_DIMENSION}"
            )));
        }
        let mut pixmap = tiny_skia::Pixmap::new(width, height)
            .ok_or(MediaError::OutOfMemory((width * height * 4) as usize))?;
        let (sw, sh) = self.size();
        let transform = tiny_skia::Transform::from_scale(
            (f64::from(width) / sw) as f32,
            (f64::from(height) / sh) as f32,
        );
        resvg::render(&self.tree, transform, &mut pixmap.as_mut());
        let frame = pool_rgba(
            pool,
            width as usize,
            height as usize,
            Rgba::U8(pixmap.data()),
            true,
            8,
            "resvg rgba8 premultiplied".into(),
        )?;
        Ok(StillImage {
            frame,
            format: ImageFormat::Svg,
            backend: ImageBackend::Resvg,
            bit_depth: 8,
            float: false,
            color: ImageColor {
                icc: None,
                icc_bytes: None,
                icc_class: None,
                transfer: Transfer::DisplayE,
                alpha: AlphaMode::Premultiplied,
                chromaticities: None,
            },
            orientation: 1,
        })
    }
}

/// Whether a file's first bytes look like SVG (or gzipped SVGZ).
pub fn sniff(head: &[u8]) -> bool {
    if head.starts_with(&[0x1f, 0x8b]) {
        return false;
    }
    let text = String::from_utf8_lossy(head);
    let t = text.trim_start_matches('\u{feff}').trim_start();
    t.starts_with("<svg")
        || ((t.starts_with("<?xml") || t.starts_with("<!--")) && text.contains("<svg"))
}

/// Rasterizes an SVG file at its intrinsic size (rounded up), the still
/// [`super::read`] returns; renderers ask [`SvgDocument::render`] for the
/// size they draw at.
pub fn read(path: &Path, pool: &FramePool) -> Result<StillImage> {
    let doc = SvgDocument::open(path)?;
    let (w, h) = doc.size();
    doc.render(
        (w.ceil() as u32).clamp(1, MAX_DIMENSION),
        (h.ceil() as u32).clamp(1, MAX_DIMENSION),
        pool,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARES: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20" viewBox="0 0 4 2">
  <rect x="0" y="0" width="2" height="2" fill="#ff0000"/>
  <rect x="2" y="0" width="2" height="2" fill="#0000ff" fill-opacity="0.5"/>
  <image href="/etc/hosts" width="4" height="2"/>
</svg>"##;

    fn px(s: &StillImage, x: usize, y: usize) -> [u8; 4] {
        s.frame
            .with_planes(|p| unsafe {
                let row = p[0].ptr.add(y * p[0].stride);
                [0, 1, 2, 3].map(|c| *row.add(x * 4 + c))
            })
            .unwrap()
    }

    #[test]
    fn renders_stretched_premultiplied_without_external_files() {
        let doc = SvgDocument::from_data(SQUARES.as_bytes()).unwrap();
        assert_eq!(doc.size(), (40.0, 20.0));
        let pool = FramePool::new(0);
        let s = doc.render(80, 10, &pool).unwrap();
        assert_eq!((s.frame.format.width, s.frame.format.height), (80, 10));
        assert!(s.frame.format.rgba_premultiplied);
        assert_eq!(px(&s, 10, 5), [255, 0, 0, 255]);
        // Half-transparent blue, premultiplied.
        let b = px(&s, 70, 5);
        assert_eq!((b[0], b[1]), (0, 0));
        assert!(
            (127..=128).contains(&b[2]) && (127..=128).contains(&b[3]),
            "{b:?}"
        );
        assert!(doc.render(0, 10, &pool).is_err());
    }

    #[test]
    fn sniffs_svg() {
        assert!(sniff(b"<svg xmlns='http://www.w3.org/2000/svg'/>"));
        assert!(sniff(b"<?xml version=\"1.0\"?>\n<svg>"));
        assert!(sniff("\u{feff}  <svg>".as_bytes()));
        assert!(!sniff(b"\x89PNG"));
    }
}
