//! The still-image classes this renderer draws (docs/engine/media.md,
//! "Stills").
//!
//! Every still at every compat level is read once, at its native depth, by
//! `pm_media::image::read` and converted in float by the source pass. The
//! classes are those today's app draws: decoder structures, color
//! descriptions and container transforms each compared with the app, so a
//! project that imported a still keeps drawing it. Files today's importer
//! rejects stay rejected ([`rejected`]); other readings outside the classes
//! stay explicit.
//!
//! Container transforms are drawn as Chromium shows them, each compared in
//! the app (`e2e/native-still-orientation-parity.spec.ts`): EXIF orientation
//! the decoder applied from PNG `eXIf` before the image data, JPEG APP1 and
//! TIFF tag 274; HEIF `irot`/`imir`/`clap` (ImageIO); AVIF `irot`/`imir` and
//! an origin-anchored `clap` as Blink interprets them (`pm_media::image::
//! avif`); WebP EXIF, which Blink ignores (so does pm-media). Late PNG
//! `eXIf` is ignored as Blink ignores it. A decoder disagreeing with a
//! pre-image-data transform stays explicit.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use pm_media::image::{ImageFormat, StillImage};

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
fn transform_matches(still: &StillImage, header: &Header) -> bool {
    match still.format {
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Tiff => {
            header.exif == Some(u16::from(still.orientation))
        }
        ImageFormat::WebP => still.orientation == 1,
        ImageFormat::Heif => true,
        _ => false,
    }
}

/// Whether a decoded still is in a drawn class.
pub fn check(path: &Path, still: &StillImage) -> Result<(), Feature> {
    match still.format {
        ImageFormat::Png
        | ImageFormat::Jpeg
        | ImageFormat::WebP
        | ImageFormat::Tiff
        | ImageFormat::Bmp
        | ImageFormat::Ico
        | ImageFormat::Gif => {}
        ImageFormat::Heif => {
            // ImageIO supplies oriented integer RGBA with its ICC profile
            // and alpha reading intact.
            if !matches!(still.bit_depth, 8 | 16) {
                return unsupported("HEIF stills outside the integer 8/16-bit decoder layouts");
            }
        }
        // Container transforms are applied in the reader, as Blink does.
        ImageFormat::Avif => return avif(still),
        ImageFormat::Exr => return unsupported("OpenEXR stills"),
        ImageFormat::Dpx => return unsupported("DPX stills"),
        ImageFormat::Svg => return unsupported("SVG stills"),
        ImageFormat::Other => return unsupported("stills of an unrecognized format"),
    }
    let file = File::open(path).map_err(|e| feature(format!("{}: {e}", path.display())))?;
    let header = parse(&mut BufReader::new(file), still.format)?;
    if let Some(t) = &header.transform
        && !transform_matches(still, &header)
    {
        return unsupported(orientation_message(t));
    }
    Ok(())
}

/// AVIF: 8, 10 and 12-bit Y′CbCr, monochrome or GBR, with an RGB (or, for
/// monochrome, gray) ICC profile, or the H.273 descriptions compared with
/// the app (`pm_media::interpret::still_cicp`: primaries BT.709,
/// unspecified, BT.2020, P3; the sRGB/BT.709 transfers, PQ, HLG).
fn avif(still: &StillImage) -> Result<(), Feature> {
    if !matches!(still.bit_depth, 8 | 10 | 12) {
        return unsupported(format!("{}-bit AVIF stills", still.bit_depth));
    }
    let cicp = still.color.cicp.unwrap_or([1, 13, 6, 1]);
    if let Some(icc) = &still.color.icc {
        let gray = still.frame.format.label.contains("yuv400");
        match icc.get(16..20) {
            Some(b"RGB ") => {}
            Some(b"GRAY") if gray => {}
            _ => {
                return unsupported("AVIF stills whose ICC color model does not match their planes");
            }
        }
        // The profile replaces primaries and transfer; the matrix stays.
        return match pm_media::interpret::still_cicp([1, 13, cicp[2], cicp[3]]) {
            Some(_) => Ok(()),
            None => unsupported(format!("AVIF stills with matrix coefficients {}", cicp[2])),
        };
    }
    match pm_media::interpret::still_cicp(cicp) {
        Some(_) => Ok(()),
        None => unsupported(format!(
            "AVIF stills with color primaries {}, transfer {} and matrix {}",
            cicp[0], cicp[1], cicp[2]
        )),
    }
}

/// Files today's importer rejects stay rejected (as unreadable media): the
/// TIFF layouts its FFmpeg conversion does not decode, whatever the native
/// readers could read (docs/engine/media.md, "Rejected imports").
pub fn rejected(path: &Path) -> Result<(), String> {
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut head = [0; 16];
    let n = file
        .read(&mut head)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if pm_media::image::sniff(&head[..n]) != ImageFormat::Tiff {
        return Ok(());
    }
    file.rewind().map_err(|e| e.to_string())?;
    // A malformed first IFD is left to the decoder to report.
    let Ok(entries) = ifd0(&mut BufReader::new(file)) else {
        return Ok(());
    };
    let scalar = |tag, default| ifd_scalar(&entries, tag, default).ok();
    let per_sample = |tag, default| ifd_per_sample(&entries, tag, default).ok();
    let photometric = scalar(262, 2);
    let float = per_sample(339, 1) == Some(3);
    let bits = per_sample(258, 1);
    let samples = scalar(277, 1).unwrap_or(1);
    let why = if matches!(photometric, Some(8..=10)) {
        Some("CIELab")
    } else if float && bits == Some(64) {
        Some("64-bit floating-point")
    } else if float && matches!(photometric, Some(0 | 1)) {
        Some("gray floating-point")
    } else if float && bits == Some(32) && samples == 4 && scalar(317, 1) == Some(3) {
        Some("RGBA float32 floating-point-predictor")
    } else if scalar(259, 1) == Some(7) && bits == Some(12) {
        Some("12-bit JPEG-compressed")
    } else {
        None
    };
    match why {
        Some(w) => Err(format!(
            "{}: {w} TIFF is not an importable still",
            path.display()
        )),
        None => Ok(()),
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
    Ok(parse(&mut BufReader::new(file), format)?.transform)
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
        ImageFormat::Heif => heif(r),
        _ => return Ok(Header::default()),
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

/// PNG at all legal integer depths: `gAMA`, `cHRM`, `iCCP`, `sRGB`, HDR
/// metadata, and a `cICP` of full-range RGB with primaries BT.709, BT.2020
/// or P3-D65 and the sRGB, PQ or HLG transfer, every pair compared with the
/// app at 8 and 16 bits, opaque and with alpha (media.md). Like Blink, eXIf
/// is read only before IDAT.
fn png<R: Read + Seek>(r: &mut R) -> io::Result<Result<Header, Feature>> {
    if &array::<8, _>(r)? != b"\x89PNG\r\n\x1a\n" {
        return Err(malformed("no PNG signature"));
    }
    let mut header = Header::default();
    let mut depth = 0u8;
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
            b"cICP" if len == 4 && before_idat => {
                let tag = array::<4, _>(r)?;
                body_read = 4;
                if !(matches!(tag[0], 1 | 9 | 12)
                    && matches!(tag[1], 13 | 16 | 18)
                    && tag[2..] == [0, 1])
                {
                    return Ok(unsupported(format!(
                        "PNG stills with CICP {}/{}/{}/{}",
                        tag[0], tag[1], tag[2], tag[3]
                    ))
                    .map(|_| header));
                }
            }
            b"cICP" if before_idat => {
                return Ok(unsupported("PNG stills with a malformed cICP chunk").map(|_| header));
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
    Ok(Ok(header))
}

/// JPEG: baseline, extended sequential, progressive and lossless, Huffman
/// or arithmetic, 2 to 16 bits, gray/RGB/Y'CbCr/CMYK/YCCK, with APP1 EXIF.
/// All decode at their sample precision.
fn jpeg<R: Read + Seek>(r: &mut R) -> io::Result<Result<Header, Feature>> {
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
    Ok(jpeg_frame(&sof).map(|_| header))
}

/// The component/sample class of a JPEG frame header.
fn jpeg_frame(sof: &[u8]) -> Result<(), Feature> {
    if sof.len() < 6 {
        return unsupported("malformed JPEG headers");
    }
    if !(2..=16).contains(&sof[0]) {
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
        // CMYK/YCCK: through a CMYK profile, else device CMYK
        // (`pm_media::image::jpeg`).
        1 | 3 | 4 => Ok(()),
        _ => unsupported("JPEG stills with neither 1, 3 nor 4 components"),
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

/// TIFF, first IFD: its orientation (tag 274). Every structure today's app
/// imports is drawn by the native readers; [`rejected`] keeps the others
/// rejected.
fn tiff<R: Read + Seek>(r: &mut R) -> io::Result<Result<Header, Feature>> {
    let entries = ifd0(r)?;
    let mut header = Header::default();
    header.exif(ifd0_orientation(&entries)?);
    Ok(Ok(header))
}

/// HEIF: the item property boxes that transform the image (`irot`,
/// `imir`, `clap`), found anywhere in `meta/iprp/ipco`. Conservative: a
/// transform property of any item (the primary image or another) counts.
fn heif<R: Read + Seek>(r: &mut R) -> io::Result<Result<Header, Feature>> {
    let end = r.seek(SeekFrom::End(0))?;
    r.seek(SeekFrom::Start(0))?;
    let mut header = Header::default();
    let mut hdr = false;
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
                boxes(r, body_end, &mut |r, ty, property_end| {
                    let t = match &ty {
                        b"irot" => {
                            let angle = (array::<1, _>(r)?[0] & 3) as u16 * 90;
                            (angle != 0).then(|| format!("HEIF irot {angle} degrees"))
                        }
                        b"imir" => Some("HEIF imir mirror".to_string()),
                        b"clap" => Some("HEIF clap clean aperture".to_string()),
                        b"colr" => {
                            if property_end - r.stream_position()? < 4 {
                                return Err(malformed("HEIF colr type"));
                            }
                            if &array::<4, _>(r)? == b"nclx" {
                                if property_end - r.stream_position()? < 7 {
                                    return Err(malformed("HEIF nclx color description"));
                                }
                                let color = array::<7, _>(r)?;
                                let transfer = u16::from_be_bytes([color[2], color[3]]);
                                // ImageIO's reader labels integer samples as
                                // DisplayE; it does not propagate PQ/HLG to
                                // the still tone-mapping path.
                                hdr |= matches!(transfer, 16 | 18);
                            }
                            None
                        }
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
    Ok(if hdr {
        unsupported("HEIF PQ/HLG stills (native HDR transfer reading is unavailable)")
            .map(|_| header)
    } else {
        Ok(header)
    })
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
        // The 3 × 3 compared CICP pairs, full range RGB.
        let cicp = |tag: [u8; 4]| png_with_bodies(16, &[(b"cICP", &tag)], &[]);
        for p in [1, 9, 12] {
            for t in [13, 16, 18] {
                assert!(png(cicp([p, t, 0, 1])).is_ok(), "{p}/{t}");
            }
        }
        for tag in [
            [9, 16, 0, 0],
            [9, 16, 9, 1],
            [2, 16, 0, 1],
            [9, 1, 0, 1],
            [9, 14, 0, 1],
        ] {
            assert!(png(cicp(tag)).is_err(), "{tag:?}");
        }
        for chunks in [&[b"cHRM", b"gAMA"][..], &[b"cHRM"], &[b"mDCV"], &[b"cLLI"]] {
            assert!(png(png_with(16, chunks)).is_ok());
        }
        assert!(png(late).is_ok());
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

    /// The TIFF layouts today's importer rejects, and nothing else.
    #[test]
    fn tiff_imports_today_rejects() {
        let dir = tempfile::Builder::new()
            .prefix("pm-tiff-rejected-")
            .tempdir()
            .unwrap();
        let verdict = |entries: &[(u16, u16, u32)]| {
            let mut e = entries.to_vec();
            e.sort_by_key(|t| t.0);
            let path = dir.path().join("t.tiff");
            std::fs::write(&path, tiff_ifd(true, &e)).unwrap();
            rejected(&path)
        };
        for entries in [
            &[(262, 3, 8), (277, 3, 3)][..],
            &[(262, 3, 2), (277, 3, 3), (258, 3, 64), (339, 3, 3)],
            &[(262, 3, 1), (277, 3, 1), (258, 3, 32), (339, 3, 3)],
            &[
                (262, 3, 2),
                (277, 3, 4),
                (258, 3, 32),
                (339, 3, 3),
                (317, 3, 3),
            ],
            &[(262, 3, 6), (277, 3, 3), (258, 3, 12), (259, 3, 7)],
        ] {
            assert!(verdict(entries).is_err(), "{entries:?}");
        }
        for entries in [
            &[(262, 3, 2), (277, 3, 3), (258, 3, 8)][..],
            &[
                (262, 3, 2),
                (277, 3, 3),
                (258, 3, 32),
                (339, 3, 3),
                (317, 3, 3),
            ],
            &[(262, 3, 2), (277, 3, 3), (258, 3, 16), (339, 3, 3)],
            &[(262, 3, 5), (277, 3, 5), (338, 3, 2)],
            &[(262, 3, 6), (277, 3, 3), (259, 3, 7)],
            &[(262, 3, 2), (277, 3, 3), (322, 3, 16), (323, 3, 16)],
        ] {
            assert!(verdict(entries).is_ok(), "{entries:?}");
        }
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
    fn heif_hdr_transfers_stay_explicit_and_truncated_color_is_malformed() {
        for transfer in [1u16, 13, 16, 18] {
            let color = [
                b"nclx".as_slice(),
                &1u16.to_be_bytes(),
                &transfer.to_be_bytes(),
                &1u16.to_be_bytes(),
                &[0x80],
            ]
            .concat();
            let result = run(ImageFormat::Heif, &heif_with(&[bmff(b"colr", &color)]));
            if matches!(transfer, 16 | 18) {
                assert!(result.unwrap_err().to_string().contains("PQ/HLG"));
            } else {
                assert!(result.is_ok(), "SDR transfer {transfer}");
            }
        }
        let error = run(
            ImageFormat::Heif,
            &heif_with(&[bmff(b"colr", b"nclx\0\x01\0")]),
        )
        .unwrap_err();
        assert!(error.to_string().contains("malformed HEIF headers"));
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
