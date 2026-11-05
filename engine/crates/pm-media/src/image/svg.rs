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
        let tree = usvg::Tree::from_data(&utf8_source(data), &options())
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
            color: ImageColor::plain(Transfer::DisplayE, AlphaMode::Premultiplied),
            orientation: 1,
            oriented: true,
        })
    }
}

/// The character encoding Chromium's XML parser reads an SVG image in
/// (Blink `TextResourceDecoder` for XML with no transport charset, as a
/// `blob:` or `file:` image has): a byte order mark; else UTF-16 recognized
/// from `<?` in 16-bit units; else the XML declaration's `encoding` label
/// (a UTF-16 label on 8-bit bytes means UTF-8, an unknown label is ignored);
/// else UTF-8.
pub fn source_encoding(data: &[u8]) -> &'static encoding_rs::Encoding {
    use encoding_rs::{UTF_8, UTF_16BE, UTF_16LE};
    if let Some((encoding, _)) = encoding_rs::Encoding::for_bom(data) {
        return encoding;
    }
    if data.starts_with(b"<\0?\0") {
        return UTF_16LE;
    }
    if data.starts_with(b"\0<\0?") {
        return UTF_16BE;
    }
    declared_encoding(data)
        .map(|e| {
            if e == UTF_16LE || e == UTF_16BE {
                UTF_8
            } else {
                e
            }
        })
        .unwrap_or(UTF_8)
}

/// The encoding an XML declaration at the start of `data` names.
fn declared_encoding(data: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    let head = &data[..data.len().min(1024)];
    if !head.starts_with(b"<?xml") {
        return None;
    }
    let end = head.windows(2).position(|w| w == b"?>")?;
    let decl = &head[5..end];
    let at = decl.windows(8).position(|w| w == b"encoding")?;
    let rest = &decl[at + 8..];
    let rest = &rest[rest.iter().position(|b| !b.is_ascii_whitespace())?..];
    let rest = rest.strip_prefix(b"=")?;
    let rest = &rest[rest.iter().position(|b| !b.is_ascii_whitespace())?..];
    let quote = *rest.first().filter(|q| matches!(q, b'"' | b'\''))?;
    let len = rest[1..].iter().position(|&b| b == quote)?;
    encoding_rs::Encoding::for_label(&rest[1..1 + len])
}

/// `data` as UTF-8 for usvg: decoded from [`source_encoding`] with
/// malformed sequences replaced (U+FFFD), as Chromium decodes before
/// parsing, and the byte order mark removed. Gzipped data is returned as is
/// (usvg inflates SVGZ itself).
pub fn utf8_source(data: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    use std::borrow::Cow;
    if data.starts_with(&[0x1f, 0x8b]) {
        return Cow::Borrowed(data);
    }
    match source_encoding(data).decode_with_bom_removal(data).0 {
        Cow::Borrowed(s) => Cow::Borrowed(s.as_bytes()),
        Cow::Owned(s) => Cow::Owned(s.into_bytes()),
    }
}

/// Whether a file's first bytes look like SVG (or gzipped SVGZ), in any
/// encoding [`source_encoding`] reads.
pub fn sniff(head: &[u8]) -> bool {
    if head.starts_with(&[0x1f, 0x8b]) {
        return false;
    }
    // A head may end inside a character; the lossy decode only affects
    // the last one.
    let text = source_encoding(head)
        .decode_without_bom_handling(
            head.strip_prefix(&[0xEF, 0xBB, 0xBF][..])
                .or_else(|| head.strip_prefix(&[0xFF, 0xFE][..]))
                .or_else(|| head.strip_prefix(&[0xFE, 0xFF][..]))
                .unwrap_or(head),
        )
        .0;
    let t = text.trim_start_matches('\u{feff}').trim_start();
    t.starts_with("<svg")
        || ((t.starts_with("<?xml") || t.starts_with("<!--")) && text.contains("<svg"))
}

pub use super::svg_broken::{MAX_ELEMENT_DEPTH, broken_image, broken_image_file};

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
        let utf16: Vec<u8> = "<?xml version=\"1.0\"?><svg/>"
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect();
        assert!(sniff(&utf16));
    }

    /// Valid SVGs in the encodings Chromium honors parse; their text keeps
    /// its characters.
    #[test]
    fn reads_declared_and_marked_encodings() {
        use encoding_rs::{UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252};
        let body = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"2\"><title>Café</title><rect width=\"4\" height=\"2\" fill=\"red\"/></svg>";
        let latin1 = |decl: &str| {
            let mut v = decl.as_bytes().to_vec();
            v.extend(body.chars().map(|c| c as u32 as u8));
            v
        };
        let l1 = latin1("<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>\n");
        assert_eq!(source_encoding(&l1), WINDOWS_1252);
        assert!(std::str::from_utf8(&l1).is_err());
        assert!(
            String::from_utf8(utf8_source(&l1).into_owned())
                .unwrap()
                .contains("Café")
        );
        assert_eq!(SvgDocument::from_data(&l1).unwrap().size(), (4.0, 2.0));
        let single = latin1("<?xml version='1.0' encoding = 'windows-1252' ?>");
        assert_eq!(source_encoding(&single), WINDOWS_1252);

        let decl = "<?xml version=\"1.0\" encoding=\"UTF-16\"?>";
        let text = format!("{decl}{body}");
        let le: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let be: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let mut be_bom = vec![0xFE, 0xFF];
        be_bom.extend_from_slice(&be);
        assert_eq!(source_encoding(&le), UTF_16LE);
        assert_eq!(source_encoding(&be), UTF_16BE);
        assert_eq!(source_encoding(&be_bom), UTF_16BE);
        for data in [&le, &be, &be_bom] {
            assert_eq!(SvgDocument::from_data(data).unwrap().size(), (4.0, 2.0));
        }
        // A UTF-16 label on 8-bit bytes, an unknown label, and no
        // declaration all read as UTF-8; malformed UTF-8 is replaced.
        assert_eq!(source_encoding(format!("{decl}{body}").as_bytes()), UTF_8);
        assert_eq!(
            source_encoding(b"<?xml version=\"1.0\" encoding=\"x-nope\"?><svg/>"),
            UTF_8
        );
        let mut bad = body.as_bytes().to_vec();
        let at = body.find("Caf").unwrap() + 3;
        bad.splice(at..at + 2, [0xFF]);
        assert_eq!(SvgDocument::from_data(&bad).unwrap().size(), (4.0, 2.0));
        // Valid UTF-8 is not copied.
        assert!(matches!(
            utf8_source(body.as_bytes()),
            std::borrow::Cow::Borrowed(_)
        ));
    }
}
