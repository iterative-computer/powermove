//! The still-image classes this renderer draws (docs/engine/pm-render.md,
//! "Image classes").
//!
//! Capability routing must be conservative (plan section 21): a still is
//! drawn only when its container, encoding, depth, and color tagging fall in
//! a class that matched the legacy renderer on the parity corpus
//! (`render/still-*`, generated with detailed content, and the fixture
//! stills). Everything else is routed to the legacy engine. The comparison
//! (2026-10-14, Chromium 152) found these classes outside section 25's
//! Standard or alpha class, so they are unsupported until the cause is
//! fixed or decided:
//!
//! - Y'CbCr JPEG: pm-media decodes with zune-jpeg; Chromium with
//!   libjpeg-turbo (islow IDCT, fancy upsampling), which reproduces the
//!   legacy frames exactly. 4:4:4 differs by up to 3 CV, 4:2:0 by 4, 4:2:2
//!   by 26; only 90 to 99.7% of pixels within 1 CV.
//! - Display P3 (PNG, JPEG, HEIF): converted values match within 1 CV on
//!   texel centers, but section 9 keeps out-of-gamut values until output,
//!   while Chromium clips at decode; filtered edges of out-of-gamut colors
//!   differ by up to 83 CV.
//! - 16-bit PNG: Chromium reduces to 8 bits before premultiplying; up to
//!   2 CV, 99.66% within 1, transparent frames 1.99 CV.
//! - WebP with alpha: libwebp's premultiplication rounding; transparent
//!   frames 1.12 CV (alpha class 1 CV).
//! - AVIF: ImageIO's and Chromium's decodes differ by up to 94 CV.
//!
//! The class is decided from the decoded [`StillImage`] facts plus a header
//! parse of the container for what the decoder does not report (JPEG frame
//! type and components, PNG color chunks, WebP chunk layout).

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use pm_media::image::icc::IccClass;
use pm_media::image::{ImageFormat, StillImage};
use pm_protocol::color::AlphaMode;

use crate::capability::Feature;

fn unsupported(what: impl Into<String>) -> Result<(), Feature> {
    Err(Feature::Media(what.into()))
}

fn feature(what: impl Into<String>) -> Feature {
    Feature::Media(what.into())
}

/// Embedded metadata (EXIF payloads) read whole is bounded; larger
/// payloads are unsupported rather than read.
const METADATA_LIMIT: u64 = 1 << 20;

/// Facts from a container header parse that the decoder may not report.
#[derive(Debug, Default, PartialEq)]
struct Header {
    /// A geometric transform the container asks for: EXIF orientation
    /// other than 1, HEIF `irot`/`imir`/`clap`.
    transform: Option<String>,
}

impl Header {
    fn exif(&mut self, orientation: Option<u16>) {
        if let Some(o) = orientation.filter(|&o| o != 1) {
            self.transform
                .get_or_insert(format!("EXIF orientation {o}"));
        }
    }
}

/// Whether a decoded still is in a tested class.
pub fn check(path: &Path, still: &StillImage) -> Result<(), Feature> {
    if still.float {
        return unsupported("floating-point stills");
    }
    // No oriented still of any drawn class has been compared with the
    // legacy renderer (which reaches HEIF and TIFF through a transcode), so
    // any orientation other than 1, reported by the decoder or found in the
    // container, is unsupported.
    if still.orientation != 1 {
        return unsupported(orientation_message(&format!(
            "EXIF orientation {}",
            still.orientation
        )));
    }
    let icc = still.color.icc_class;
    if icc == Some(IccClass::DisplayP3) {
        return unsupported(
            "Display P3 stills (out-of-gamut handling differs from the legacy renderer)",
        );
    }
    match still.format {
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP => {}
        ImageFormat::Heif => {
            if still.bit_depth != 8 {
                return unsupported("HEIF stills deeper than 8 bits");
            }
            if still.color.alpha != AlphaMode::Opaque {
                return unsupported("HEIF stills with alpha");
            }
        }
        ImageFormat::Tiff => {
            if still.bit_depth != 8 {
                return unsupported("TIFF stills deeper than 8 bits");
            }
            if still.color.alpha != AlphaMode::Opaque {
                return unsupported("TIFF stills with alpha");
            }
            if icc.is_some() {
                return unsupported("TIFF stills with an ICC profile");
            }
        }
        ImageFormat::Avif => return unsupported("AVIF stills"),
        ImageFormat::Bmp => return unsupported("BMP stills"),
        ImageFormat::Ico => return unsupported("ICO stills"),
        ImageFormat::Gif => return unsupported("GIF stills"),
        ImageFormat::Exr => return unsupported("OpenEXR stills"),
        ImageFormat::Dpx => return unsupported("DPX stills"),
        ImageFormat::Other => return unsupported("stills of an unrecognized format"),
    }
    let file = File::open(path).map_err(|e| feature(format!("{}: {e}", path.display())))?;
    let header = parse(&mut BufReader::new(file), still.format)?;
    if let Some(t) = header.transform {
        return unsupported(orientation_message(&t));
    }
    match still.format {
        ImageFormat::Png if still.bit_depth != 8 => unsupported("16-bit PNG stills"),
        ImageFormat::WebP if still.color.alpha != AlphaMode::Opaque => {
            unsupported("WebP stills with alpha")
        }
        _ => Ok(()),
    }
}

/// Whether an image asset's recorded format (or file name extension) is a
/// container this renderer may draw, for the static check that opens no
/// media. JPEG is listed (gray JPEG is drawable, but nearly every JPEG is
/// Y'CbCr) and so are AVIF and unknown containers; the depth, profile, and
/// alpha facts of the other containers need [`check`]
/// (`unsupported_with_media`).
pub fn static_format(format: &str, name: &str) -> Result<(), Feature> {
    let ext = if format.is_empty() {
        name.rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default()
    } else {
        format.to_ascii_lowercase()
    };
    match ext.as_str() {
        "png" | "webp" | "heic" | "heif" | "tif" | "tiff" => Ok(()),
        "jpg" | "jpeg" | "jpe" | "jfif" => unsupported("JPEG stills"),
        "" => unsupported("an image of unrecorded format"),
        other => unsupported(format!("{other} stills")),
    }
}

fn orientation_message(what: &str) -> String {
    format!("oriented stills ({what}; not compared with the legacy renderer)")
}

/// Parses the container header of a format [`check`] draws. Reads only
/// headers and metadata, seeking past image data.
fn parse<R: Read + Seek>(r: &mut R, format: ImageFormat) -> Result<Header, Feature> {
    let name = match format {
        ImageFormat::Png => "PNG",
        ImageFormat::Jpeg => "JPEG",
        ImageFormat::WebP => "WebP",
        ImageFormat::Tiff => "TIFF",
        _ => "HEIF",
    };
    let parsed = match format {
        ImageFormat::Png => png(r),
        ImageFormat::Jpeg => jpeg(r),
        ImageFormat::WebP => webp(r),
        ImageFormat::Tiff => tiff(r),
        _ => heif(r),
    };
    match parsed {
        Ok(h) => h,
        Err(e) => Err(feature(format!("malformed {name} headers ({e})"))),
    }
}

fn malformed(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_string())
}

fn read_vec<R: Read>(r: &mut R, len: u64) -> io::Result<Vec<u8>> {
    if len > METADATA_LIMIT {
        return Err(malformed("metadata larger than 1 MiB"));
    }
    let mut v = vec![0; len as usize];
    r.read_exact(&mut v)?;
    Ok(v)
}

fn array<const N: usize, R: Read>(r: &mut R) -> io::Result<[u8; N]> {
    let mut b = [0; N];
    r.read_exact(&mut b)?;
    Ok(b)
}

/// Reads up to `N` bytes; fewer only at the end of the stream.
fn array_or_eof<const N: usize, R: Read>(r: &mut R) -> io::Result<Option<[u8; N]>> {
    let mut b = [0; N];
    let mut got = 0;
    while got < N {
        match r.read(&mut b[got..])? {
            0 => break,
            n => got += n,
        }
    }
    Ok((got == N).then_some(b))
}

/// The scalar entries (count 1, SHORT or LONG) of a TIFF structure's first
/// IFD (a TIFF file or an EXIF payload), as (tag, value). Entries with
/// several values or of other types are listed with `None`.
fn ifd0<R: Read + Seek>(r: &mut R) -> io::Result<Vec<(u16, Option<u32>)>> {
    let head: [u8; 8] = array(r)?;
    let le = match &head[..4] {
        b"II*\0" => true,
        b"MM\0*" => false,
        b"II+\0" | b"MM\0+" => return Err(malformed("BigTIFF")),
        _ => return Err(malformed("not a TIFF structure")),
    };
    let u16_ = |b: [u8; 2]| {
        if le {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        }
    };
    let u32_ = |b: [u8; 4]| {
        if le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        }
    };
    let ifd = u32_(head[4..8].try_into().unwrap());
    r.seek(SeekFrom::Start(ifd as u64))?;
    let count = u16_(array(r)?);
    let entries = read_vec(r, count as u64 * 12)?;
    Ok(entries
        .chunks_exact(12)
        .map(|e| {
            let tag = u16_([e[0], e[1]]);
            let value = match (u16_([e[2], e[3]]), u32_(e[4..8].try_into().unwrap())) {
                (3, 1) => Some(u16_([e[8], e[9]]) as u32),
                (4, 1) => Some(u32_(e[8..12].try_into().unwrap())),
                _ => None,
            };
            (tag, value)
        })
        .collect())
}

/// The orientation tag (274) of a TIFF structure's first IFD; `None` when
/// absent.
fn ifd0_orientation(entries: &[(u16, Option<u32>)]) -> io::Result<Option<u16>> {
    match entries.iter().find(|(tag, _)| *tag == 274) {
        None => Ok(None),
        Some((_, Some(v))) => Ok(Some((*v).min(u16::MAX as u32) as u16)),
        Some((_, None)) => Err(malformed("orientation tag of an unexpected type")),
    }
}

/// The orientation of an EXIF payload (with or without the `Exif\0\0`
/// prefix JPEG uses).
fn exif_orientation(payload: &[u8]) -> io::Result<Option<u16>> {
    let tiff = payload.strip_prefix(b"Exif\0\0").unwrap_or(payload);
    ifd0_orientation(&ifd0(&mut io::Cursor::new(tiff))?)
}

/// PNG: 8 or 16-bit samples; no HDR or CICP chunks; gamma and
/// chromaticity chunks only where an sRGB chunk or ICC profile overrides
/// them (as in Chromium and libpng); the orientation of an `eXIf` chunk
/// (anywhere in the file).
fn png<R: Read + Seek>(r: &mut R) -> io::Result<Result<Header, Feature>> {
    if &array::<8, _>(r)? != b"\x89PNG\r\n\x1a\n" {
        return Err(malformed("no PNG signature"));
    }
    let mut header = Header::default();
    let (mut depth, mut srgb, mut iccp, mut gama, mut chrm) = (0u8, false, false, false, false);
    let mut before_idat = true;
    while let Some(head) = array_or_eof::<8, _>(r)? {
        let len = u32::from_be_bytes(head[..4].try_into().unwrap()) as u64;
        let ty: [u8; 4] = head[4..].try_into().unwrap();
        let mut body_read = 0;
        match &ty {
            b"IHDR" if len >= 13 => {
                depth = array::<13, _>(r)?[8];
                body_read = 13;
            }
            b"sRGB" => srgb |= before_idat,
            b"iCCP" => iccp |= before_idat,
            b"gAMA" => gama |= before_idat,
            b"cHRM" => chrm |= before_idat,
            b"cICP" | b"mDCv" | b"cLLi" | b"mDCV" | b"cLLI" => {
                return Ok(unsupported("PNG stills with CICP or HDR metadata").map(|_| header));
            }
            b"eXIf" => {
                header.exif(exif_orientation(&read_vec(r, len)?)?);
                body_read = len;
            }
            b"IDAT" => before_idat = false,
            b"IEND" => break,
            _ => {}
        }
        r.seek_relative((len - body_read + 4) as i64)?;
    }
    if !matches!(depth, 8 | 16) {
        return Ok(unsupported(format!("{depth}-bit PNG stills")).map(|_| header));
    }
    if (gama || chrm) && !srgb && !iccp {
        return Ok(
            unsupported("PNG stills with gamma or chromaticity chunks (gAMA, cHRM)")
                .map(|_| header),
        );
    }
    Ok(Ok(header))
}

/// JPEG: baseline or progressive Huffman, 8-bit, one (gray) component;
/// the orientation of an `Exif` APP1 segment. Y'CbCr is unsupported (see
/// the module notes), as are RGB, CMYK, arithmetic, lossless, and 12-bit
/// JPEG.
fn jpeg<R: Read + Seek>(r: &mut R) -> io::Result<Result<Header, Feature>> {
    if array::<2, _>(r)? != [0xFF, 0xD8] {
        return Err(malformed("no start of image"));
    }
    let mut header = Header::default();
    let mut adobe_transform: Option<u8> = None;
    let mut sof: Option<Vec<u8>> = None;
    loop {
        if array::<1, _>(r)?[0] != 0xFF {
            return Err(malformed("marker expected"));
        }
        let mut marker = array::<1, _>(r)?[0];
        while marker == 0xFF {
            marker = array::<1, _>(r)?[0];
        }
        match marker {
            0x01 | 0xD0..=0xD7 => continue,
            0xD9 | 0xDA => break,
            _ => {}
        }
        let len = u16::from_be_bytes(array(r)?) as u64;
        if len < 2 {
            return Err(malformed("segment length"));
        }
        let body_len = len - 2;
        match marker {
            0xEE | 0xE1 | 0xC0 | 0xC2 => {
                let body = read_vec(r, body_len)?;
                match marker {
                    0xEE if body.len() >= 12 && body.starts_with(b"Adobe") => {
                        adobe_transform = Some(body[11]);
                    }
                    0xE1 if body.starts_with(b"Exif\0\0") => {
                        header.exif(exif_orientation(&body)?);
                    }
                    0xC0 | 0xC2 => sof = Some(body),
                    _ => {}
                }
            }
            0xC1 | 0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                return Ok(
                    unsupported("JPEG stills other than baseline or progressive Huffman")
                        .map(|_| header),
                );
            }
            _ => r.seek_relative(body_len as i64)?,
        }
    }
    let sof = sof.ok_or_else(|| malformed("no frame header"))?;
    Ok(jpeg_frame(&sof, adobe_transform).map(|_| header))
}

fn jpeg_frame(sof: &[u8], adobe_transform: Option<u8>) -> Result<(), Feature> {
    if sof.len() < 6 {
        return unsupported("malformed JPEG headers");
    }
    if sof[0] != 8 {
        return unsupported(format!("{}-bit JPEG stills", sof[0]));
    }
    let n = sof[5] as usize;
    let comps: Vec<&[u8]> = sof[6..].chunks_exact(3).take(n).collect();
    if comps.len() != n {
        return unsupported("malformed JPEG headers");
    }
    match n {
        1 => Ok(()),
        3 if adobe_transform == Some(0) || comps.iter().map(|c| c[0]).eq(*b"RGB") => {
            unsupported("RGB-encoded JPEG stills")
        }
        3 => unsupported(
            "Y'CbCr JPEG stills (pm-media's decoder differs from Chromium's libjpeg-turbo)",
        ),
        _ => unsupported("CMYK and other JPEG stills with neither 1 nor 3 components"),
    }
}

/// WebP: lossy (`VP8 `) or lossless (`VP8L`), not animated, no ICC
/// profile (alpha is decided from the decoded still); the orientation of an
/// `EXIF` chunk.
fn webp<R: Read + Seek>(r: &mut R) -> io::Result<Result<Header, Feature>> {
    let riff: [u8; 12] = array(r)?;
    if &riff[..4] != b"RIFF" || &riff[8..] != b"WEBP" {
        return Err(malformed("no RIFF WEBP header"));
    }
    let mut header = Header::default();
    while let Some(head) = array_or_eof::<8, _>(r)? {
        let len = u32::from_le_bytes(head[4..].try_into().unwrap()) as u64;
        let mut body_read = 0;
        match &head[..4] {
            b"ICCP" => {
                return Ok(unsupported("WebP stills with an ICC profile").map(|_| header));
            }
            b"ANIM" | b"ANMF" => return Ok(unsupported("animated WebP").map(|_| header)),
            b"EXIF" => {
                header.exif(exif_orientation(&read_vec(r, len)?)?);
                body_read = len;
            }
            _ => {}
        }
        r.seek_relative((len - body_read + (len & 1)) as i64)?;
    }
    Ok(Ok(header))
}

/// TIFF, first IFD (the image FFmpeg converts for the legacy renderer and
/// pm-media decodes): chunky RGB, three samples, no predictor other than
/// horizontal differencing, compressed without loss by a scheme whose
/// decode was shown identical to the uncompressed file
/// (`tests/stills.rs`, `tiff_compressions_decode_alike`): none, LZW,
/// Deflate, PackBits. Depth, alpha, and ICC are decided from the decoded
/// still; the orientation tag (274) other than 1 is a transform.
fn tiff<R: Read + Seek>(r: &mut R) -> io::Result<Result<Header, Feature>> {
    let entries = ifd0(r)?;
    let mut header = Header::default();
    header.exif(ifd0_orientation(&entries)?);
    // A tag of the image structure: present and scalar, or absent (the
    // TIFF default); several values (or another type) is outside the class.
    let scalar = |tag: u16, default: u32| -> Result<u32, ()> {
        match entries.iter().find(|(t, _)| *t == tag) {
            None => Ok(default),
            Some((_, Some(v))) => Ok(*v),
            Some((_, None)) => Err(()),
        }
    };
    let class = (|| -> Result<(), String> {
        let compression = scalar(259, 1).map_err(|_| "TIFF compression tag".to_string())?;
        if !matches!(compression, 1 | 5 | 8 | 32946 | 32773) {
            return Err(format!("TIFF stills with compression {compression}"));
        }
        // Photometric interpretation has no default; RGB only (gray,
        // palette, CMYK, and Y'CbCr were not compared).
        let photometric = scalar(262, u32::MAX).map_err(|_| "TIFF photometric tag".to_string())?;
        if photometric != 2 {
            return Err(format!(
                "TIFF stills with photometric interpretation {}",
                if photometric == u32::MAX {
                    "missing".to_string()
                } else {
                    photometric.to_string()
                }
            ));
        }
        if scalar(277, 1) != Ok(3) {
            return Err("TIFF stills with other than three samples per pixel".into());
        }
        if scalar(284, 1) != Ok(1) {
            return Err("planar TIFF stills".into());
        }
        let predictor = scalar(317, 1).map_err(|_| "TIFF predictor tag".to_string())?;
        if !matches!(predictor, 1 | 2) {
            return Err(format!("TIFF stills with predictor {predictor}"));
        }
        Ok(())
    })();
    // An orientation is reported first: it is the reason the legacy
    // comparison does not apply, whatever the structure.
    Ok(match class {
        Err(what) if header.transform.is_none() => Err(feature(what)),
        _ => Ok(header),
    })
}

/// HEIF: the item property boxes that transform the image (`irot`,
/// `imir`, `clap`), found anywhere in `meta/iprp/ipco`. Conservative: a
/// transform property of any item (the primary image or another) counts.
fn heif<R: Read + Seek>(r: &mut R) -> io::Result<Result<Header, Feature>> {
    let end = r.seek(SeekFrom::End(0))?;
    r.seek(SeekFrom::Start(0))?;
    let mut header = Header::default();
    boxes(r, end, &mut |r, ty, body_end| {
        if &ty != b"meta" {
            return Ok(());
        }
        array::<4, _>(r)?; // full box version and flags
        boxes(r, body_end, &mut |r, ty, body_end| {
            if &ty != b"iprp" {
                return Ok(());
            }
            boxes(r, body_end, &mut |r, ty, body_end| {
                if &ty != b"ipco" {
                    return Ok(());
                }
                boxes(r, body_end, &mut |r, ty, _| {
                    let t = match &ty {
                        b"irot" => {
                            let angle = (array::<1, _>(r)?[0] & 3) as u16 * 90;
                            (angle != 0).then(|| format!("HEIF irot {angle} degrees"))
                        }
                        b"imir" => Some("HEIF imir mirror".to_string()),
                        b"clap" => Some("HEIF clap clean aperture".to_string()),
                        _ => None,
                    };
                    if let Some(t) = t {
                        header.transform.get_or_insert(t);
                    }
                    Ok(())
                })
            })
        })
    })?;
    Ok(Ok(header))
}

/// A visitor of one ISO BMFF box: `(reader at the body, type, body end)`.
type BoxVisitor<'a, R> = dyn FnMut(&mut R, [u8; 4], u64) -> io::Result<()> + 'a;

/// Walks the ISO BMFF boxes from the current position to `end`, calling
/// `f(reader, type, body_end)` with the reader at each box's body.
fn boxes<R: Read + Seek>(r: &mut R, end: u64, f: &mut BoxVisitor<'_, R>) -> io::Result<()> {
    loop {
        let at = r.stream_position()?;
        if at + 8 > end {
            return Ok(());
        }
        let head: [u8; 8] = array(r)?;
        let ty: [u8; 4] = head[4..].try_into().unwrap();
        let size = match u32::from_be_bytes(head[..4].try_into().unwrap()) {
            0 => end - at,
            1 => u64::from_be_bytes(array(r)?),
            n => n as u64,
        };
        let body = r.stream_position()?;
        let body_end = at.checked_add(size).ok_or_else(|| malformed("box size"))?;
        if body_end < body || body_end > end {
            return Err(malformed("box size"));
        }
        f(r, ty, body_end)?;
        r.seek(SeekFrom::Start(body_end))?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(format: ImageFormat, bytes: &[u8]) -> Result<Header, Feature> {
        parse(&mut io::Cursor::new(bytes), format)
    }

    fn transform(format: ImageFormat, bytes: &[u8]) -> Option<String> {
        run(format, bytes).unwrap().transform
    }

    /// A TIFF structure with one IFD holding `entries` (tag, type, value).
    fn tiff_ifd(le: bool, entries: &[(u16, u16, u32)]) -> Vec<u8> {
        let u16b = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let u32b = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let mut b = if le {
            b"II*\0".to_vec()
        } else {
            b"MM\0*".to_vec()
        };
        b.extend_from_slice(&u32b(8));
        b.extend_from_slice(&u16b(entries.len() as u16));
        for &(tag, ty, v) in entries {
            b.extend_from_slice(&u16b(tag));
            b.extend_from_slice(&u16b(ty));
            b.extend_from_slice(&u32b(1));
            if ty == 3 {
                b.extend_from_slice(&u16b(v as u16));
                b.extend_from_slice(&[0, 0]);
            } else {
                b.extend_from_slice(&u32b(v));
            }
        }
        b.extend_from_slice(&[0; 4]);
        b
    }

    fn exif(orientation: u16) -> Vec<u8> {
        tiff_ifd(false, &[(271, 2, 0), (274, 3, orientation as u32)])
    }

    fn png_with_bodies(
        depth: u8,
        chunks: &[(&[u8; 4], &[u8])],
        after: &[(&[u8; 4], &[u8])],
    ) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut chunk = |ty: &[u8; 4], body: &[u8]| {
            b.extend_from_slice(&(body.len() as u32).to_be_bytes());
            b.extend_from_slice(ty);
            b.extend_from_slice(body);
            b.extend_from_slice(&[0; 4]);
        };
        chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, depth, 6, 0, 0, 0]);
        for (ty, body) in chunks {
            chunk(ty, body);
        }
        chunk(b"IDAT", &[1, 2, 3]);
        for (ty, body) in after {
            chunk(ty, body);
        }
        chunk(b"IEND", &[]);
        b
    }

    fn png_with(depth: u8, chunks: &[&[u8; 4]]) -> Vec<u8> {
        let with: Vec<(&[u8; 4], &[u8])> = chunks.iter().map(|c| (*c, &[0u8; 4][..])).collect();
        png_with_bodies(depth, &with, &[])
    }

    #[test]
    fn png_classes() {
        let png = |b: Vec<u8>| run(ImageFormat::Png, &b);
        assert!(png(png_with(8, &[])).is_ok());
        assert!(png(png_with(16, &[])).is_ok());
        assert!(png(png_with(4, &[])).is_err());
        assert!(png(png_with(8, &[b"gAMA"])).is_err());
        assert!(png(png_with(8, &[b"cHRM", b"gAMA"])).is_err());
        assert!(png(png_with(8, &[b"sRGB", b"gAMA", b"cHRM"])).is_ok());
        assert!(png(png_with(8, &[b"iCCP", b"gAMA"])).is_ok());
        assert!(png(png_with(8, &[b"cICP"])).is_err());
        assert!(png(png_with(16, &[b"mDCv"])).is_err());
        // An sRGB chunk after the image data overrides nothing.
        let late = png_with_bodies(8, &[(b"gAMA", &[0; 4])], &[(b"sRGB", &[0])]);
        assert!(png(late).is_err());
        assert!(png(b"\x89PNG\r\n\x1a".to_vec()).is_err());
    }

    #[test]
    fn png_orientation() {
        for (o, le) in [(1, true), (6, false), (8, true), (2, false)] {
            let e = tiff_ifd(le, &[(274, 3, o)]);
            for at_end in [false, true] {
                let chunk = [(b"eXIf", &e[..])];
                let b = if at_end {
                    png_with_bodies(8, &[], &chunk)
                } else {
                    png_with_bodies(8, &chunk, &[])
                };
                let t = transform(ImageFormat::Png, &b);
                assert_eq!(t.is_some(), o != 1, "{o} {at_end}: {t:?}");
            }
        }
        // EXIF without an orientation tag, and with the JPEG prefix.
        let none = tiff_ifd(true, &[(271, 2, 0)]);
        assert_eq!(
            transform(
                ImageFormat::Png,
                &png_with_bodies(8, &[(b"eXIf", &none)], &[])
            ),
            None
        );
        let prefixed = [b"Exif\0\0".to_vec(), exif(3)].concat();
        let t = transform(
            ImageFormat::Png,
            &png_with_bodies(8, &[(b"eXIf", &prefixed)], &[]),
        );
        assert_eq!(t.as_deref(), Some("EXIF orientation 3"));
        // Malformed EXIF is not trusted.
        assert!(
            run(
                ImageFormat::Png,
                &png_with_bodies(8, &[(b"eXIf", b"junk")], &[])
            )
            .is_err()
        );
    }

    fn jpeg_with(sof: u8, precision: u8, comps: &[(u8, u8)], adobe: Option<u8>) -> Vec<u8> {
        jpeg_with_exif(sof, precision, comps, adobe, None)
    }

    fn jpeg_with_exif(
        sof: u8,
        precision: u8,
        comps: &[(u8, u8)],
        adobe: Option<u8>,
        orientation: Option<u16>,
    ) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8];
        if let Some(o) = orientation {
            let body = [b"Exif\0\0".to_vec(), exif(o)].concat();
            b.extend_from_slice(&[0xFF, 0xE1]);
            b.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
            b.extend_from_slice(&body);
        }
        if let Some(t) = adobe {
            let body = [b'A', b'd', b'o', b'b', b'e', 0, 100, 0, 0, 0, 0, t];
            b.extend_from_slice(&[0xFF, 0xEE, 0, (body.len() + 2) as u8]);
            b.extend_from_slice(&body);
        }
        // A comment and fill bytes before the frame header.
        b.extend_from_slice(&[0xFF, 0xFE, 0, 4, b'h', b'i', 0xFF]);
        let mut body = vec![precision, 0, 8, 0, 8, comps.len() as u8];
        for (id, hv) in comps {
            body.extend_from_slice(&[*id, *hv, 0]);
        }
        b.extend_from_slice(&[0xFF, sof, 0, (body.len() + 2) as u8]);
        b.extend_from_slice(&body);
        b.extend_from_slice(&[0xFF, 0xDA, 0, 2]);
        b
    }

    #[test]
    fn jpeg_classes() {
        let jpeg = |b: Vec<u8>| run(ImageFormat::Jpeg, &b);
        let gray = [(1, 0x11)];
        assert!(jpeg(jpeg_with(0xC0, 8, &gray, None)).is_ok());
        assert!(jpeg(jpeg_with(0xC2, 8, &gray, None)).is_ok());
        let ycc = |y: u8| [(1, y), (2, 0x11), (3, 0x11)];
        for y in [0x11, 0x21, 0x22] {
            let e = jpeg(jpeg_with(0xC0, 8, &ycc(y), None)).unwrap_err();
            assert!(e.to_string().contains("Y'CbCr"), "{e}");
        }
        assert!(jpeg(jpeg_with(0xC0, 8, &ycc(0x11), Some(1))).is_err());
        // 12-bit, arithmetic, lossless, CMYK, RGB, malformed.
        assert!(jpeg(jpeg_with(0xC1, 12, &gray, None)).is_err());
        assert!(jpeg(jpeg_with(0xC0, 12, &gray, None)).is_err());
        assert!(jpeg(jpeg_with(0xC9, 8, &gray, None)).is_err());
        assert!(jpeg(jpeg_with(0xC3, 8, &gray, None)).is_err());
        let cmyk = [(1, 0x11), (2, 0x11), (3, 0x11), (4, 0x11)];
        assert!(jpeg(jpeg_with(0xC0, 8, &cmyk, Some(2))).is_err());
        let e = jpeg(jpeg_with(0xC0, 8, &ycc(0x11), Some(0))).unwrap_err();
        assert!(e.to_string().contains("RGB"), "{e}");
        let rgb = [(b'R', 0x11), (b'G', 0x11), (b'B', 0x11)];
        let e = jpeg(jpeg_with(0xC0, 8, &rgb, None)).unwrap_err();
        assert!(e.to_string().contains("RGB"), "{e}");
        assert!(jpeg(vec![0xFF, 0xD8, 0x00]).is_err());
        assert!(jpeg(vec![0xFF, 0xD8, 0xFF, 0xDA, 0, 2]).is_err());
    }

    #[test]
    fn jpeg_orientation() {
        let gray = [(1, 0x11)];
        for o in 1..=8 {
            let b = jpeg_with_exif(0xC0, 8, &gray, None, Some(o));
            let t = transform(ImageFormat::Jpeg, &b);
            assert_eq!(t.is_some(), o != 1, "{o}: {t:?}");
        }
    }

    fn riff(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut b = b"RIFF\0\0\0\0WEBP".to_vec();
        for (ty, body) in chunks {
            b.extend_from_slice(*ty);
            b.extend_from_slice(&(body.len() as u32).to_le_bytes());
            b.extend_from_slice(body);
            if body.len() & 1 == 1 {
                b.push(0);
            }
        }
        b
    }

    #[test]
    fn webp_classes() {
        let webp = |b: Vec<u8>| run(ImageFormat::WebP, &b);
        let z = |n: usize| vec![0u8; n];
        assert!(webp(riff(&[(b"VP8 ", z(11))])).is_ok());
        assert!(webp(riff(&[(b"VP8X", z(10)), (b"ALPH", z(5)), (b"VP8 ", z(8))])).is_ok());
        assert!(webp(riff(&[(b"VP8X", z(10)), (b"ICCP", z(7)), (b"VP8L", z(8))])).is_err());
        assert!(webp(riff(&[(b"VP8X", z(10)), (b"ANIM", z(6)), (b"ANMF", z(8))])).is_err());
        assert!(webp(b"RIFF\0\0\0\0WEBQ".to_vec()).is_err());
    }

    #[test]
    fn webp_orientation() {
        for o in [1, 5, 6] {
            for exif in [exif(o), [b"Exif\0\0".to_vec(), exif(o)].concat()] {
                let b = riff(&[
                    (b"VP8X", vec![0; 10]),
                    (b"VP8 ", vec![0; 9]),
                    (b"EXIF", exif),
                ]);
                assert_eq!(transform(ImageFormat::WebP, &b).is_some(), o != 1, "{o}");
            }
        }
    }

    /// The tags of a baseline RGB TIFF plus `extra`.
    fn rgb_tiff(le: bool, extra: &[(u16, u16, u32)]) -> Vec<u8> {
        let mut e = vec![(256, 3, 4), (262, 3, 2), (277, 3, 3)];
        e.extend_from_slice(extra);
        e.sort_by_key(|t| t.0);
        tiff_ifd(le, &e)
    }

    #[test]
    fn tiff_orientation() {
        for le in [true, false] {
            for o in [1, 6, 7] {
                let b = rgb_tiff(le, &[(274, 3, o)]);
                assert_eq!(
                    transform(ImageFormat::Tiff, &b).is_some(),
                    o != 1,
                    "{le} {o}"
                );
            }
            let long = rgb_tiff(le, &[(274, 4, 8)]);
            assert!(transform(ImageFormat::Tiff, &long).is_some());
            assert_eq!(transform(ImageFormat::Tiff, &rgb_tiff(le, &[])), None);
            // The orientation is reported even outside the RGB class.
            let gray = tiff_ifd(le, &[(262, 3, 1), (274, 3, 6)]);
            assert!(transform(ImageFormat::Tiff, &gray).is_some());
        }
        assert!(run(ImageFormat::Tiff, b"II+\0\x08\0\0\0").is_err());
    }

    #[test]
    fn tiff_classes() {
        let tiff = |extra: &[(u16, u16, u32)]| run(ImageFormat::Tiff, &rgb_tiff(true, extra));
        for c in [1, 5, 8, 32946, 32773] {
            assert!(tiff(&[(259, 3, c)]).is_ok(), "compression {c}");
        }
        for c in [2, 3, 4, 6, 7, 34712] {
            let e = tiff(&[(259, 3, c)]).unwrap_err();
            assert!(e.to_string().contains("compression"), "{c}: {e}");
        }
        assert!(tiff(&[(317, 3, 2)]).is_ok());
        assert!(tiff(&[(317, 3, 3)]).is_err(), "floating-point predictor");
        assert!(tiff(&[(284, 3, 1)]).is_ok());
        assert!(tiff(&[(284, 3, 2)]).is_err(), "planar");
        // Photometric: only RGB; a missing tag is outside the class.
        for (photometric, name) in [
            (0, "white"),
            (1, "black"),
            (3, "palette"),
            (5, "CMYK"),
            (6, "Y'CbCr"),
        ] {
            let b = tiff_ifd(true, &[(262, 3, photometric), (277, 3, 3)]);
            let e = run(ImageFormat::Tiff, &b).unwrap_err();
            assert!(e.to_string().contains("photometric"), "{name}: {e}");
        }
        let missing = tiff_ifd(true, &[(277, 3, 3)]);
        assert!(
            run(ImageFormat::Tiff, &missing)
                .unwrap_err()
                .to_string()
                .contains("missing")
        );
        // Samples per pixel: three only (an extra sample is not alpha the
        // decoders agree on).
        let four = tiff_ifd(true, &[(262, 3, 2), (277, 3, 4), (338, 3, 0)]);
        assert!(run(ImageFormat::Tiff, &four).is_err());
        let one = tiff_ifd(true, &[(262, 3, 2)]);
        assert!(
            run(ImageFormat::Tiff, &one).is_err(),
            "SamplesPerPixel defaults to 1"
        );
        // A structure tag with several values is outside the class.
        let mut multi = rgb_tiff(true, &[]);
        let at = 8 + 2 + 12; // the second entry, photometric
        multi[at + 4..at + 8].copy_from_slice(&2u32.to_le_bytes());
        assert!(run(ImageFormat::Tiff, &multi).is_err());
    }

    fn bmff(ty: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(ty);
        b.extend_from_slice(body);
        b
    }

    fn heif_with(properties: &[Vec<u8>]) -> Vec<u8> {
        let ipco = bmff(b"ipco", &properties.concat());
        let iprp = bmff(b"iprp", &[ipco, bmff(b"ipma", &[0; 8])].concat());
        let meta = bmff(
            b"meta",
            &[
                vec![0; 4],
                bmff(b"hdlr", &[0; 24]),
                bmff(b"pitm", &[0; 6]),
                iprp,
            ]
            .concat(),
        );
        [
            bmff(b"ftyp", b"heicmif1heic"),
            meta,
            bmff(b"mdat", &[0; 32]),
        ]
        .concat()
    }

    #[test]
    fn heif_transforms() {
        let ispe = bmff(b"ispe", &[0; 12]);
        assert_eq!(
            transform(ImageFormat::Heif, &heif_with(std::slice::from_ref(&ispe))),
            None
        );
        assert_eq!(
            transform(
                ImageFormat::Heif,
                &heif_with(&[ispe.clone(), bmff(b"irot", &[0])])
            ),
            None
        );
        for (p, want) in [
            (bmff(b"irot", &[1]), "HEIF irot 90 degrees"),
            (bmff(b"irot", &[3]), "HEIF irot 270 degrees"),
            (bmff(b"imir", &[0]), "HEIF imir mirror"),
            (bmff(b"clap", &[0; 32]), "HEIF clap clean aperture"),
        ] {
            let t = transform(ImageFormat::Heif, &heif_with(&[ispe.clone(), p]));
            assert_eq!(t.as_deref(), Some(want));
        }
        // A box that claims to run past its parent is malformed.
        let mut bad = heif_with(&[ispe]);
        let meta_at = bad.windows(4).position(|w| w == b"meta").unwrap() - 4;
        bad[meta_at..meta_at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(run(ImageFormat::Heif, &bad).is_err());
    }

    #[test]
    fn static_formats() {
        assert!(static_format("png", "").is_ok());
        assert!(static_format("", "photo.JPG").is_err());
        assert!(static_format("avif", "x.avif").is_err());
        assert!(static_format("HEIC", "x").is_ok());
        assert!(static_format("bmp", "x.bmp").is_err());
        assert!(static_format("gif", "x.gif").is_err());
        assert!(static_format("exr", "x.exr").is_err());
        assert!(static_format("", "noextension").is_err());
    }
}
