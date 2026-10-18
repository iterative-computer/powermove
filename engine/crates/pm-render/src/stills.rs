//! The still-image classes this renderer draws (docs/engine/pm-render.md,
//! "Image classes").
//!
//! Level 1 consumes Blink's 8-bit compatibility bitmap (pm-media/image/
//! compat.rs): PNG16 quantization, skcms ICC conversion/clipping, and WebP
//! premultiplication occur before filtering. AVIF uses Chromium 152's
//! pinned CrabbyAvif/dav1d/libyuv conversion, followed by the measured
//! Skia PQ/HLG/primaries transform. TIFF uses the shipped FFmpeg rendition;
//! JPEG supports CMYK and preserves deep JPEG's incomplete black sampler.
//! Native-depth source readers
//! remain available to level 2 and Interpret Footage.
//!
//! All 21 detailed still corpus cases pass the 2D/alpha classes. Header
//! checks keep unmeasured color descriptions explicit; measured decoder
//! failures preserve today's errors or incomplete texture as documented.
//!
//! Container transforms are drawn only as Chromium shows them, each
//! compared in the app (`e2e/native-still-orientation-parity.spec.ts`):
//! EXIF orientation the decoder applied from PNG `eXIf` before the image
//! data, JPEG APP1 and TIFF tag 274; HEIF `irot`/`imir`/`clap` (ImageIO, as
//! the `sips` rendition Chromium shows); AVIF `irot`/`imir`/`clap` as Blink
//! interprets them (`avif_compat`); WebP EXIF, which Blink ignores (so does
//! pm-media). Late PNG `eXIf` is ignored as Blink ignores it. A decoder
//! disagreeing with a pre-image-data transform stays explicit.

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

/// The samples a still layer draws, which decide its header classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StillReader {
    /// Level 1: Chromium's compatibility bitmap (`pm_media::image::compat`,
    /// `avif_compat`, and the FFmpeg TIFF rendition). Its header classes
    /// cover every color description and structure compared with the app.
    Chromium,
    /// Level 2 layers and 3D material maps: the native-depth readers
    /// (`pm_media::image::read`). These read neither PNG cICP/cHRM nor HDR
    /// transfers, and ImageIO converts CMYK and other models to sRGB, so the
    /// structures and color descriptions measured only for Chromium's
    /// bitmap stay explicit here.
    Native,
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
    /// The first EXIF orientation other than 1.
    exif: Option<u16>,
}

impl Header {
    fn exif(&mut self, orientation: Option<u16>) {
        if let Some(o) = orientation.filter(|&o| o != 1) {
            self.transform
                .get_or_insert(format!("EXIF orientation {o}"));
            self.exif.get_or_insert(o);
        }
    }
}

/// Whether the drawn still shows the container's transform as Chromium
/// shows it (module notes).
fn transform_matches(still: &StillImage, header: &Header, compat: bool) -> bool {
    compat
        && match still.format {
            ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Tiff => {
                header.exif == Some(u16::from(still.orientation))
            }
            ImageFormat::WebP => still.orientation == 1,
            ImageFormat::Heif => true,
            _ => false,
        }
}

/// Whether a decoded still is in a tested class for `reader`. An AVIF
/// Chromium's decoder cannot parse is not a class:
/// [`crate::media::probe_image`] reports it as an unreadable source before
/// calling this.
pub fn check(path: &Path, still: &StillImage, reader: StillReader) -> Result<(), Feature> {
    let chromium = reader == StillReader::Chromium;
    if still.float && !pm_media::image::compat::applies(still) {
        return unsupported("floating-point stills");
    }
    // Compatibility classes consume the decoder-applied orientation (the
    // container check below decides whether it is Chromium's).
    let compat =
        pm_media::image::compat::applies(still) || pm_media::image::avif_compat::applies(still);
    if still.orientation != 1 && !compat {
        return unsupported(orientation_message(&format!(
            "EXIF orientation {}",
            still.orientation
        )));
    }
    let icc = still.color.icc_class;
    if icc == Some(IccClass::DisplayP3) && !compat {
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
            if still.bit_depth != 8 && !compat {
                return unsupported("TIFF stills deeper than 8 bits");
            }
            if still.color.alpha != AlphaMode::Opaque && !compat {
                return unsupported("TIFF stills with alpha");
            }
            // ICC is preserved by the native reader and converted by skcms
            // in the level-1 bitmap, as in the shipped PNG rendition.
            if icc.is_some() && !compat {
                return unsupported(
                    "TIFF stills with an ICC profile (not compared with the legacy renderer)",
                );
            }
        }
        ImageFormat::Avif if pm_media::image::avif_compat::applies(still) => {
            // Container transforms follow Blink in `avif_compat`.
            let unmeasured = if chromium {
                pm_media::image::avif_compat::unmeasured(path)
            } else {
                pm_media::image::avif_compat::unmeasured_sdr(path)
            };
            return match unmeasured {
                Ok(None) => Ok(()),
                Ok(Some(what)) => {
                    unsupported(format!("{what} (not compared with the legacy renderer)"))
                }
                // Reported unreadable by `probe_image`.
                Err(_) => Ok(()),
            };
        }
        ImageFormat::Avif => {
            return unsupported("AVIF stills outside the tested SDR classes");
        }
        ImageFormat::Bmp | ImageFormat::Ico | ImageFormat::Gif => return Ok(()),
        ImageFormat::Exr => return unsupported("OpenEXR stills"),
        ImageFormat::Dpx => return unsupported("DPX stills"),
        ImageFormat::Svg => return unsupported("SVG stills"),
        ImageFormat::Other => return unsupported("stills of an unrecognized format"),
    }
    let file = File::open(path).map_err(|e| feature(format!("{}: {e}", path.display())))?;
    let header = if still.format == ImageFormat::Tiff && compat {
        tiff(&mut BufReader::new(file), true, reader)
            .map_err(|e| feature(format!("malformed TIFF headers ({e})")))??
    } else if still.format == ImageFormat::Png && compat {
        let cicp = match still.color.icc_class {
            Some(IccClass::DisplayP3) => Some([12, 13, 0, 1]),
            Some(IccClass::Srgb) => Some([1, 13, 0, 1]),
            _ => None,
        };
        png(&mut BufReader::new(file), cicp, reader)
            .map_err(|e| feature(format!("malformed PNG headers ({e})")))??
    } else {
        parse(&mut BufReader::new(file), still.format, reader)?
    };
    if let Some(t) = &header.transform
        && !transform_matches(still, &header, compat)
    {
        return unsupported(orientation_message(t));
    }
    match still.format {
        ImageFormat::Png if still.bit_depth != 8 && !compat => unsupported("16-bit PNG stills"),
        ImageFormat::WebP if still.color.alpha != AlphaMode::Opaque && !compat => {
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
        "png" | "webp" | "heic" | "heif" | "tif" | "tiff" | "bmp" | "ico" | "gif" => Ok(()),
        "jpg" | "jpeg" | "jpe" | "jfif" => unsupported("JPEG stills"),
        "" => unsupported("an image of unrecorded format"),
        other => unsupported(format!("{other} stills")),
    }
}

/// The geometric transform a still's container asks for (EXIF orientation
/// other than 1, HEIF `irot`/`imir`/`clap`) from the header parse alone,
/// whatever the decoder reports; [`check`] routes on either. `Err` when the
/// container is malformed, not one [`check`] parses, or (without a
/// transform) outside the drawn structure.
pub fn container_transform(path: &Path, format: ImageFormat) -> Result<Option<String>, Feature> {
    if !matches!(
        format,
        ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::WebP
            | ImageFormat::Tiff
            | ImageFormat::Heif
    ) {
        return Err(feature(format!("no header parse for {format:?} stills")));
    }
    let file = File::open(path).map_err(|e| feature(format!("{}: {e}", path.display())))?;
    Ok(parse(&mut BufReader::new(file), format, StillReader::Native)?.transform)
}

fn orientation_message(what: &str) -> String {
    format!("oriented stills ({what}; not compared with the legacy renderer)")
}

/// Parses the container header of a format [`check`] draws. Reads only
/// headers and metadata, seeking past image data.
fn parse<R: Read + Seek>(
    r: &mut R,
    format: ImageFormat,
    reader: StillReader,
) -> Result<Header, Feature> {
    let name = match format {
        ImageFormat::Png => "PNG",
        ImageFormat::Jpeg => "JPEG",
        ImageFormat::WebP => "WebP",
        ImageFormat::Tiff => "TIFF",
        _ => "HEIF",
    };
    let parsed = match format {
        ImageFormat::Png => png(r, None, reader),
        ImageFormat::Jpeg => jpeg(r, reader),
        ImageFormat::WebP => webp(r),
        ImageFormat::Tiff => tiff(r, false, reader),
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

/// SHORT or LONG entries with at most this many values have their values
/// read; longer arrays (strip offsets and the like) are listed without.
const IFD_VALUES: u32 = 16;

/// One entry of a TIFF structure's first IFD: its tag and, for SHORT or
/// LONG entries of at most [`IFD_VALUES`] values, the values (empty for
/// other types and longer arrays).
#[derive(Debug)]
struct IfdEntry {
    tag: u16,
    values: Vec<u32>,
}

/// The entries of a TIFF structure's first IFD (a TIFF file or an EXIF
/// payload; offsets are relative to the reader's start).
fn ifd0<R: Read + Seek>(r: &mut R) -> io::Result<Vec<IfdEntry>> {
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
    let raw = read_vec(r, count as u64 * 12)?;
    let mut entries = Vec::with_capacity(count as usize);
    for e in raw.chunks_exact(12) {
        let tag = u16_([e[0], e[1]]);
        let n = u32_(e[4..8].try_into().unwrap());
        let width = match u16_([e[2], e[3]]) {
            3 => 2,
            4 => 4,
            _ => 0,
        };
        let mut values = Vec::new();
        if width != 0 && (1..=IFD_VALUES).contains(&n) {
            let len = n as usize * width;
            let bytes = if len <= 4 {
                e[8..8 + len].to_vec()
            } else {
                r.seek(SeekFrom::Start(u32_(e[8..12].try_into().unwrap()) as u64))?;
                read_vec(r, len as u64)?
            };
            values = bytes
                .chunks_exact(width)
                .map(|v| match width {
                    2 => u16_([v[0], v[1]]) as u32,
                    _ => u32_(v.try_into().unwrap()),
                })
                .collect();
        }
        entries.push(IfdEntry { tag, values });
    }
    Ok(entries)
}

fn ifd_entry(entries: &[IfdEntry], tag: u16) -> Option<&IfdEntry> {
    entries.iter().find(|e| e.tag == tag)
}

/// A scalar tag: its value, `default` when absent, `Err` when it holds
/// several values or another type.
fn ifd_scalar(entries: &[IfdEntry], tag: u16, default: u32) -> Result<u32, ()> {
    match ifd_entry(entries, tag).map(|e| e.values.as_slice()) {
        None => Ok(default),
        Some(&[v]) => Ok(v),
        Some(_) => Err(()),
    }
}

/// A per-sample tag (BitsPerSample, SampleFormat): the value every sample
/// shares, `default` when absent, `Err` when the samples differ or the
/// entry is of another type.
fn ifd_per_sample(entries: &[IfdEntry], tag: u16, default: u32) -> Result<u32, ()> {
    match ifd_entry(entries, tag).map(|e| e.values.as_slice()) {
        None => Ok(default),
        Some([v, rest @ ..]) if rest.iter().all(|r| r == v) => Ok(*v),
        Some(_) => Err(()),
    }
}

/// The orientation tag (274) of a TIFF structure's first IFD; `None` when
/// absent.
fn ifd0_orientation(entries: &[IfdEntry]) -> io::Result<Option<u16>> {
    match ifd_entry(entries, 274).map(|e| e.values.as_slice()) {
        None => Ok(None),
        Some(&[v]) => Ok(Some(v.min(u16::MAX as u32) as u16)),
        Some(_) => Err(malformed("orientation tag of an unexpected type")),
    }
}

/// The orientation of an EXIF payload (with or without the `Exif\0\0`
/// prefix JPEG uses).
fn exif_orientation(payload: &[u8]) -> io::Result<Option<u16>> {
    let tiff = payload.strip_prefix(b"Exif\0\0").unwrap_or(payload);
    ifd0_orientation(&ifd0(&mut io::Cursor::new(tiff))?)
}

/// PNG at all legal integer depths.
///
/// For Chromium's bitmap ([`StillReader::Chromium`]): gAMA-only
/// conversion, cHRM/gAMA, and cICP that matches the ICC profile or is a
/// full-range RGB pair of primaries {1, 9, 12} and transfer {13, 16, 18},
/// every pair compared with the app at 8 and 16 bits, opaque and with alpha
/// (media.md, "Level 1 HDR and chromaticity stills"); mDCv/cLLi do not
/// change Chromium's bitmap.
///
/// For the native readers ([`StillReader::Native`]), which read neither
/// cICP nor cHRM nor HDR metadata: gAMA-only conversion and cICP equal to
/// the ICC profile's; other cICP, HDR metadata, and cHRM without sRGB or
/// iCCP stay explicit.
///
/// Like Blink, eXIf is read only before IDAT.
fn png<R: Read + Seek>(
    r: &mut R,
    equivalent_cicp: Option<[u8; 4]>,
    reader: StillReader,
) -> io::Result<Result<Header, Feature>> {
    if &array::<8, _>(r)? != b"\x89PNG\r\n\x1a\n" {
        return Err(malformed("no PNG signature"));
    }
    let chromium = reader == StillReader::Chromium;
    let mut header = Header::default();
    let mut depth = 0u8;
    let mut before_idat = true;
    let (mut srgb, mut iccp, mut chrm) = (false, false, false);
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
            b"cHRM" => chrm |= before_idat,
            b"cICP" if len == 4 && chromium && before_idat => {
                let tag = array::<4, _>(r)?;
                body_read = 4;
                if Some(tag) != equivalent_cicp && !chromium_png_cicp(tag) {
                    return Ok(unsupported("PNG CICP differs from its ICC profile").map(|_| header));
                }
            }
            b"cICP" if chromium && before_idat => {
                return Ok(unsupported("PNG stills with CICP or HDR metadata").map(|_| header));
            }
            b"cICP" if len == 4 && !chromium && equivalent_cicp.is_some() => {
                let tag = array::<4, _>(r)?;
                body_read = 4;
                if Some(tag) != equivalent_cicp {
                    return Ok(unsupported("PNG CICP differs from its ICC profile").map(|_| header));
                }
            }
            b"cICP" | b"mDCv" | b"cLLi" | b"mDCV" | b"cLLI" if !chromium => {
                return Ok(unsupported("PNG stills with CICP or HDR metadata").map(|_| header));
            }
            b"eXIf" if before_idat => {
                header.exif(exif_orientation(&read_vec(r, len)?)?);
                body_read = len;
            }
            b"IDAT" => before_idat = false,
            b"IEND" => break,
            _ => {}
        }
        r.seek_relative((len - body_read + 4) as i64)?;
    }
    if !matches!(depth, 1 | 2 | 4 | 8 | 16) {
        return Ok(unsupported(format!("{depth}-bit PNG stills")).map(|_| header));
    }
    if !chromium && chrm && !srgb && !iccp {
        return Ok(
            unsupported("PNG stills with independent chromaticity chunks (cHRM)").map(|_| header),
        );
    }
    Ok(Ok(header))
}

/// The PNG cICP tags Chromium's bitmap draws without an equivalent ICC
/// profile: full-range RGB with primaries BT.709, BT.2020 or P3-D65 and the
/// sRGB, PQ or HLG transfer, each pair compared with the app
/// (`pm_media::image::compat`, `PngColor::Cicp`).
fn chromium_png_cicp(tag: [u8; 4]) -> bool {
    matches!(tag[0], 1 | 9 | 12) && matches!(tag[1], 13 | 16 | 18) && tag[2..] == [0, 1]
}

/// JPEG: Huffman baseline/progressive with gray/RGB/Y'CbCr and APP1 EXIF.
/// For Chromium's bitmap also extended sequential, lossless and arithmetic
/// DCT (whose deep ImageBitmap uploads keep Chromium's opaque-black
/// incomplete sampler) and CMYK/YCCK. The native readers keep those
/// explicit: ImageIO converts CMYK to sRGB, and deep JPEG has no level 2
/// comparison.
fn jpeg<R: Read + Seek>(r: &mut R, reader: StillReader) -> io::Result<Result<Header, Feature>> {
    let chromium = reader == StillReader::Chromium;
    if array::<2, _>(r)? != [0xFF, 0xD8] {
        return Err(malformed("no start of image"));
    }
    let mut header = Header::default();
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
            // The native readers draw only baseline and progressive Huffman.
            0xC1 | 0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF if !chromium => {
                return Ok(
                    unsupported("JPEG stills other than baseline or progressive Huffman")
                        .map(|_| header),
                );
            }
            // The Adobe APP14 transform and component ids are decoder facts
            // libjpeg-turbo applies as Chromium does; only EXIF and the
            // frame header decide the class.
            0xE1 | 0xC0..=0xC3 | 0xC9 | 0xCA => {
                let body = read_vec(r, body_len)?;
                match marker {
                    0xE1 if body.starts_with(b"Exif\0\0") => {
                        header.exif(exif_orientation(&body)?);
                    }
                    0xC0..=0xC3 | 0xC9 | 0xCA => sof = Some(body),
                    _ => {}
                }
            }
            0xC5..=0xC7 | 0xCB | 0xCD..=0xCF => {
                return Ok(unsupported("hierarchical or differential JPEG stills").map(|_| header));
            }
            _ => r.seek_relative(body_len as i64)?,
        }
    }
    let sof = sof.ok_or_else(|| malformed("no frame header"))?;
    Ok(jpeg_frame(&sof, reader).map(|_| header))
}

/// The component/sample class of a JPEG frame header.
fn jpeg_frame(sof: &[u8], reader: StillReader) -> Result<(), Feature> {
    let chromium = reader == StillReader::Chromium;
    if sof.len() < 6 {
        return unsupported("malformed JPEG headers");
    }
    if !(chromium && (2..=16).contains(&sof[0]) || sof[0] == 8) {
        return unsupported(format!("{}-bit JPEG stills", sof[0]));
    }
    let n = sof[5] as usize;
    let comps: Vec<&[u8]> = sof[6..].chunks_exact(3).take(n).collect();
    if comps.len() != n {
        return unsupported("malformed JPEG headers");
    }
    match n {
        // libjpeg-turbo performs both RGB and Y'CbCr conversion as in
        // Chromium; component ids and Adobe transform are decoder facts.
        1 | 3 => Ok(()),
        // CMYK/YCCK: Chromium's integer conversion (`pm_media::image::jpeg`).
        4 if chromium => Ok(()),
        _ if chromium => unsupported("JPEG stills with neither 1, 3 nor 4 components"),
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
/// pm-media decodes).
///
/// Chromium's bitmap ([`StillReader::Chromium`] with `allow_icc`) reads the
/// shipped FFmpeg PNG rendition (`pm_media::image::compat_tiff`), which
/// decodes tiles, planar layouts, Y'CbCr/JPEG, CMYK and the float layouts
/// as the app does; files the app rejects are named media errors before
/// this check, so only the orientation tag (274) remains.
///
/// The native readers keep the structures compared with the legacy
/// renderer (`render/still-photo-tiff` and the app compositions in
/// `media.md`): strips (not tiles), fill order 1, no predictor other than
/// horizontal differencing, compressed without loss by a scheme whose
/// decode was shown identical to the uncompressed file (`tests/stills.rs`,
/// `tiff_compressions_decode_alike`): none, LZW, Deflate (8 and 32946),
/// PackBits. Without the compatibility bitmap's classes (`allow_icc`
/// false) only chunky 8-bit RGB without an ICC profile is in the class.
/// With them: RGB or RGBA in 8 or 16-bit unsigned integers or
/// uncompressed/Deflate float32 samples, gray or gray-alpha in 8 or 16
/// bits, 8-bit palettes, and an ICC profile (tag 34675). Colorimetry tags
/// (TransferFunction, WhitePoint, PrimaryChromaticities), CMYK, Y'CbCr and
/// planar layouts were not compared for the native readers. Depth and alpha
/// are also decided from the decoded still; the orientation tag other than
/// 1 is a transform.
fn tiff<R: Read + Seek>(
    r: &mut R,
    allow_icc: bool,
    reader: StillReader,
) -> io::Result<Result<Header, Feature>> {
    let entries = ifd0(r)?;
    let mut header = Header::default();
    header.exif(ifd0_orientation(&entries)?);
    if allow_icc && reader == StillReader::Chromium {
        return Ok(Ok(header));
    }
    // A tag of the image structure: present and scalar, or absent (the
    // TIFF default); several values (or another type) is outside the class.
    let scalar = |tag: u16, default: u32| ifd_scalar(&entries, tag, default);
    let class = (|| -> Result<(), String> {
        if ifd_entry(&entries, 34675).is_some() && !allow_icc {
            return Err(
                "TIFF stills with an ICC profile (not compared with the legacy renderer)".into(),
            );
        }
        if let Some(e) = [301, 318, 319]
            .into_iter()
            .find_map(|t| ifd_entry(&entries, t))
        {
            return Err(format!(
                "TIFF stills with colorimetry tags (tag {}; not compared with the legacy renderer)",
                e.tag
            ));
        }
        let compression = scalar(259, 1).map_err(|_| "TIFF compression tag".to_string())?;
        if !matches!(compression, 1 | 5 | 8 | 32946 | 32773) {
            return Err(format!("TIFF stills with compression {compression}"));
        }
        // RGB, gray and 8-bit palette indices were compared through the
        // app. CMYK and Y'CbCr remain separate unmeasured classes.
        let photometric = scalar(262, u32::MAX).map_err(|_| "TIFF photometric tag".to_string())?;
        if photometric != 2 && !(allow_icc && matches!(photometric, 0 | 1 | 3)) {
            return Err(format!(
                "TIFF stills with photometric interpretation {}",
                if photometric == u32::MAX {
                    "missing".to_string()
                } else {
                    photometric.to_string()
                }
            ));
        }
        let samples = scalar(277, 1);
        let bits = ifd_per_sample(&entries, 258, 1);
        if photometric == 3 {
            if samples != Ok(1) || bits != Ok(8) {
                return Err("TIFF palettes outside the tested 8-bit index class".into());
            }
        } else if matches!(photometric, 0 | 1) {
            if !matches!(samples, Ok(1 | 2)) || !matches!(bits, Ok(8 | 16)) {
                return Err("gray TIFF outside unsigned 8/16-bit gray/alpha samples".into());
            }
        } else {
            if !(samples == Ok(3) || (allow_icc && samples == Ok(4))) {
                return Err("TIFF stills with other than three samples per pixel".into());
            }
            if !(bits == Ok(8)
                || (allow_icc && bits == Ok(16))
                || (allow_icc
                    && bits == Ok(32)
                    && matches!(compression, 1 | 8)
                    && ifd_per_sample(&entries, 339, 1) == Ok(3)))
            {
                return Err("TIFF stills with other than 8 bits in every sample".into());
            }
        }
        if !(ifd_per_sample(&entries, 339, 1) == Ok(1)
            || (allow_icc
                && photometric == 2
                && bits == Ok(32)
                && matches!(compression, 1 | 8)
                && ifd_per_sample(&entries, 339, 1) == Ok(3)))
        {
            return Err(
                "TIFF stills outside unsigned integer or measured RGB float32 samples".into(),
            );
        }
        if scalar(284, 1) != Ok(1) {
            return Err("planar TIFF stills".into());
        }
        if scalar(266, 1) != Ok(1) {
            return Err("TIFF stills with fill order other than 1".into());
        }
        if [322, 323, 324, 325]
            .into_iter()
            .any(|t| ifd_entry(&entries, t).is_some())
        {
            return Err("tiled TIFF stills".into());
        }
        let predictor = scalar(317, 1).map_err(|_| "TIFF predictor tag".to_string())?;
        if !matches!(predictor, 1 | 2) {
            return Err(format!("TIFF stills with predictor {predictor}"));
        }
        Ok(())
    })();
    if allow_icc {
        return Ok(class.map(|()| header).map_err(feature));
    }
    // Preserve the old header-only orientation diagnostic.
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
        parse(&mut io::Cursor::new(bytes), format, StillReader::Chromium)
    }

    /// The classes of the native-depth readers (level 2 and 3D maps).
    fn native(format: ImageFormat, bytes: &[u8]) -> Result<Header, Feature> {
        parse(&mut io::Cursor::new(bytes), format, StillReader::Native)
    }

    fn transform(format: ImageFormat, bytes: &[u8]) -> Option<String> {
        run(format, bytes).unwrap().transform
    }

    /// A TIFF structure with one IFD holding `entries` (tag, type, value).
    fn tiff_ifd(le: bool, entries: &[(u16, u16, u32)]) -> Vec<u8> {
        let e: Vec<(u16, u16, Vec<u32>)> =
            entries.iter().map(|&(t, ty, v)| (t, ty, vec![v])).collect();
        tiff_ifd_values(le, &e)
    }

    /// A TIFF structure with one IFD holding `entries` (tag, type, values);
    /// SHORT and LONG values longer than four bytes are stored after the
    /// IFD, other types hold their first value as the raw offset field.
    fn tiff_ifd_values(le: bool, entries: &[(u16, u16, Vec<u32>)]) -> Vec<u8> {
        let u16b = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let u32b = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let mut b = if le {
            b"II*\0".to_vec()
        } else {
            b"MM\0*".to_vec()
        };
        b.extend_from_slice(&u32b(8));
        b.extend_from_slice(&u16b(entries.len() as u16));
        let mut out_of_line = Vec::new();
        let data_at = 8 + 2 + entries.len() * 12 + 4;
        for (tag, ty, v) in entries {
            b.extend_from_slice(&u16b(*tag));
            b.extend_from_slice(&u16b(*ty));
            b.extend_from_slice(&u32b(v.len() as u32));
            let mut bytes = Vec::new();
            for &x in v {
                match ty {
                    3 => bytes.extend_from_slice(&u16b(x as u16)),
                    _ => bytes.extend_from_slice(&u32b(x)),
                }
            }
            if !matches!(ty, 3 | 4) {
                bytes.truncate(4);
            }
            if bytes.len() <= 4 {
                bytes.resize(4, 0);
                b.extend_from_slice(&bytes);
            } else {
                b.extend_from_slice(&u32b((data_at + out_of_line.len()) as u32));
                out_of_line.extend_from_slice(&bytes);
            }
        }
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&out_of_line);
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
        for bits in [1, 2, 4] {
            assert!(png(png_with(bits, &[])).is_ok());
        }
        assert!(png(png_with(8, &[b"gAMA"])).is_ok());
        assert!(png(png_with(8, &[b"cHRM", b"gAMA"])).is_ok());
        assert!(png(png_with(8, &[b"sRGB", b"gAMA", b"cHRM"])).is_ok());
        assert!(png(png_with(8, &[b"iCCP", b"gAMA"])).is_ok());
        assert!(png(png_with(8, &[b"cICP"])).is_err());
        assert!(png(png_with(16, &[b"mDCv"])).is_ok());
        // An sRGB chunk after the image data overrides nothing.
        let late = png_with_bodies(8, &[(b"cHRM", &[0; 32])], &[(b"sRGB", &[0])]);
        assert!(png(late.clone()).is_ok());
        assert!(png(b"\x89PNG\r\n\x1a".to_vec()).is_err());
        // Chromium's bitmap: the 3 × 3 compared CICP pairs, full range RGB.
        let cicp = |tag: [u8; 4]| png_with_bodies(16, &[(b"cICP", &tag)], &[]);
        for p in [1, 9, 12] {
            for t in [13, 16, 18] {
                assert!(png(cicp([p, t, 0, 1])).is_ok(), "{p}/{t}");
                assert!(native(ImageFormat::Png, &cicp([p, t, 0, 1])).is_err());
            }
        }
        for tag in [[9, 16, 0, 0], [9, 16, 9, 1], [2, 16, 0, 1], [9, 1, 0, 1], [9, 14, 0, 1]] {
            assert!(png(cicp(tag)).is_err(), "{tag:?}");
        }
        // The native readers read neither cICP nor cHRM nor HDR metadata.
        let native_png = |b: Vec<u8>| native(ImageFormat::Png, &b);
        assert!(native_png(png_with(8, &[b"gAMA"])).is_ok());
        assert!(native_png(png_with(16, &[])).is_ok());
        assert!(native_png(png_with(8, &[b"sRGB", b"gAMA", b"cHRM"])).is_ok());
        assert!(native_png(png_with(8, &[b"iCCP", b"gAMA", b"cHRM"])).is_ok());
        for chunks in [
            &[b"cHRM", b"gAMA"][..],
            &[b"cHRM"],
            &[b"cICP"],
            &[b"mDCv"],
            &[b"cLLi"],
            &[b"mDCV"],
            &[b"cLLI"],
        ] {
            let e = native_png(png_with(16, chunks)).unwrap_err();
            assert!(
                e.to_string().contains("chromaticity") || e.to_string().contains("CICP"),
                "{e}"
            );
        }
        assert!(native_png(late).is_err());
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
                assert_eq!(t.is_some(), o != 1 && !at_end, "{o} {at_end}: {t:?}");
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
            assert!(jpeg(jpeg_with(0xC0, 8, &ycc(y), None)).is_ok());
            assert!(jpeg(jpeg_with(0xC2, 8, &ycc(y), None)).is_ok());
        }
        assert!(jpeg(jpeg_with(0xC0, 8, &ycc(0x11), Some(1))).is_ok());
        // Extended/deep/lossless, arithmetic DCT and CMYK are measured.
        assert!(jpeg(jpeg_with(0xC1, 12, &gray, None)).is_ok());
        assert!(jpeg(jpeg_with(0xC0, 12, &gray, None)).is_ok());
        assert!(jpeg(jpeg_with(0xC9, 8, &gray, None)).is_ok());
        assert!(jpeg(jpeg_with(0xC3, 8, &gray, None)).is_ok());
        let cmyk = [(1, 0x11), (2, 0x11), (3, 0x11), (4, 0x11)];
        assert!(jpeg(jpeg_with(0xC0, 8, &cmyk, Some(2))).is_ok());
        assert!(jpeg(jpeg_with(0xC0, 8, &ycc(0x11), Some(0))).is_ok());
        let rgb = [(b'R', 0x11), (b'G', 0x11), (b'B', 0x11)];
        assert!(jpeg(jpeg_with(0xC0, 8, &rgb, None)).is_ok());
        assert!(jpeg(jpeg_with(0xC2, 8, &rgb, Some(0))).is_ok());
        assert!(jpeg(vec![0xFF, 0xD8, 0x00]).is_err());
        assert!(jpeg(vec![0xFF, 0xD8, 0xFF, 0xDA, 0, 2]).is_err());
        // Hierarchical and differential frames stay explicit for both.
        for sof in [0xC5, 0xC6, 0xC7, 0xCB, 0xCD, 0xCE, 0xCF] {
            assert!(jpeg(jpeg_with(sof, 8, &gray, None)).is_err(), "{sof:x}");
        }
        // The native readers: baseline/progressive 8-bit gray/RGB/Y'CbCr.
        let native_jpeg = |b: Vec<u8>| native(ImageFormat::Jpeg, &b);
        assert!(native_jpeg(jpeg_with(0xC0, 8, &ycc(0x22), None)).is_ok());
        assert!(native_jpeg(jpeg_with(0xC2, 8, &gray, None)).is_ok());
        assert!(native_jpeg(jpeg_with(0xC0, 8, &rgb, None)).is_ok());
        for (sof, bits, comps, what) in [
            (0xC1, 12, &gray[..], "baseline"),
            (0xC0, 12, &gray[..], "12-bit"),
            (0xC9, 8, &gray[..], "baseline"),
            (0xCA, 8, &gray[..], "baseline"),
            (0xC3, 8, &gray[..], "baseline"),
            (0xC0, 8, &cmyk[..], "CMYK"),
        ] {
            let e = native_jpeg(jpeg_with(sof, bits, comps, Some(2))).unwrap_err();
            assert!(e.to_string().contains(what), "{sof:x}: {e}");
        }
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
        let mut e = vec![(256, 3, 4), (258, 3, 8), (262, 3, 2), (277, 3, 3)];
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
        // The structure FFmpeg and ImageIO write: per-sample arrays stored
        // after the IFD, in either byte order.
        for le in [true, false] {
            let b = tiff_ifd_values(
                le,
                &[
                    (256, 3, vec![4]),
                    (258, 3, vec![8, 8, 8]),
                    (259, 3, vec![5]),
                    (262, 3, vec![2]),
                    (266, 3, vec![1]),
                    (277, 3, vec![3]),
                    (284, 3, vec![1]),
                    (317, 3, vec![2]),
                    (339, 3, vec![1, 1, 1]),
                ],
            );
            assert!(run(ImageFormat::Tiff, &b).is_ok(), "{le}");
        }
        let arrays = |extra: (u16, u16, Vec<u32>)| {
            let mut e = vec![(256, 3, vec![4]), (262, 3, vec![2]), (277, 3, vec![3])];
            if extra.0 != 258 {
                e.push((258, 3, vec![8, 8, 8]));
            }
            e.push(extra);
            e.sort_by_key(|t| t.0);
            run(ImageFormat::Tiff, &tiff_ifd_values(true, &e))
        };
        for (entry, want) in [
            ((258, 3, vec![16, 16, 16]), "8 bits"),
            ((258, 3, vec![8, 8, 4]), "8 bits"),
            (
                (
                    258,
                    4,
                    vec![8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8],
                ),
                "8 bits",
            ),
            ((339, 3, vec![3, 3, 3]), "unsigned"),
            ((339, 3, vec![2, 2, 2]), "unsigned"),
            ((339, 3, vec![1, 1, 3]), "unsigned"),
            ((266, 3, vec![2]), "fill order"),
            ((322, 3, vec![16]), "tiled"),
            ((323, 4, vec![16]), "tiled"),
            ((324, 4, vec![8, 8]), "tiled"),
            ((34675, 7, vec![0]), "ICC profile"),
            ((301, 3, vec![0; 3]), "colorimetry"),
            ((318, 5, vec![0]), "colorimetry"),
            ((319, 5, vec![0]), "colorimetry"),
        ] {
            let tag = entry.0;
            let e = arrays(entry).unwrap_err().to_string();
            assert!(e.contains(want), "tag {tag}: {e}");
        }
        assert!(arrays((258, 3, vec![8, 8, 8])).is_ok());
        assert!(arrays((339, 3, vec![1, 1, 1])).is_ok());
        assert!(arrays((266, 3, vec![1])).is_ok());
        // An ICC profile is routed in either byte order, and the
        // orientation still takes precedence in the message.
        for le in [true, false] {
            let b = rgb_tiff(le, &[(34675, 7, 0)]);
            assert!(run(ImageFormat::Tiff, &b).is_err(), "{le}");
            let oriented = rgb_tiff(le, &[(274, 3, 6), (34675, 7, 0)]);
            assert!(transform(ImageFormat::Tiff, &oriented).is_some(), "{le}");
        }
        // A structure tag with several values is outside the class.
        let mut multi = rgb_tiff(true, &[]);
        let at = 8 + 2 + 2 * 12; // the third entry, photometric
        multi[at + 4..at + 8].copy_from_slice(&2u32.to_le_bytes());
        assert!(run(ImageFormat::Tiff, &multi).is_err());
    }

    /// Chromium's bitmap reads the shipped FFmpeg TIFF rendition, whose
    /// structures were compared with the app; the native readers keep the
    /// structures compared for them.
    #[test]
    fn tiff_classes_follow_the_reader() {
        let class = |entries: &[(u16, u16, u32)], reader| {
            let mut e = entries.to_vec();
            if !e.iter().any(|t| t.0 == 258) {
                e.push((258, 3, 8));
            }
            e.sort_by_key(|t| t.0);
            tiff(&mut io::Cursor::new(tiff_ifd(true, &e)), true, reader).unwrap()
        };
        for (entries, what) in [
            (&[(262, 3, 5), (277, 3, 4)][..], "photometric"),
            (&[(262, 3, 5), (277, 3, 5), (338, 3, 2)][..], "photometric"),
            (&[(262, 3, 6), (277, 3, 3), (259, 3, 7)][..], "compression"),
            (&[(262, 3, 2), (277, 3, 3), (322, 3, 16), (323, 3, 16)][..], "tiled"),
            (&[(262, 3, 2), (277, 3, 3), (284, 3, 2)][..], "planar"),
            (&[(262, 3, 2), (277, 3, 3), (258, 3, 16), (339, 3, 3)][..], "unsigned"),
            (&[(262, 3, 2), (277, 3, 3), (317, 3, 3)][..], "predictor"),
            (&[(262, 3, 8), (277, 3, 3)][..], "photometric"),
        ] {
            assert!(class(entries, StillReader::Chromium).is_ok(), "{entries:?}");
            let e = class(entries, StillReader::Native).unwrap_err().to_string();
            assert!(e.contains(what), "{entries:?}: {e}");
        }
        // Both draw the RGB classes compared for the native readers.
        let rgb = [(262, 3, 2), (277, 3, 3), (258, 3, 16)];
        assert!(class(&rgb, StillReader::Native).is_ok());
        assert!(class(&rgb, StillReader::Chromium).is_ok());
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
        assert!(static_format("bmp", "x.bmp").is_ok());
        assert!(static_format("gif", "x.gif").is_ok());
        assert!(static_format("exr", "x.exr").is_err());
        assert!(static_format("", "noextension").is_err());
    }
}
