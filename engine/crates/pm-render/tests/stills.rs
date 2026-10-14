//! Still-image routing (docs/engine/pm-render.md, "Image classes"): stills
//! outside the classes compared against the legacy renderer are reported
//! unsupported by the media probe that `unsupported_with_media` and the
//! renderer use, so capability routing sends them to the legacy engine.
//! The tested classes themselves are compared in `tests/corpus.rs`
//! (`render/still-*` and the fixture stills).

use std::path::{Path, PathBuf};
use std::process::Command;

use pm_render::Feature;
use pm_render::media::{ProbeIssue, probe_image};

fn dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("pm-render-stills-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ffmpeg(out: &Path, args: &[&str]) {
    let bin = std::env::var_os("PM_FFMPEG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap())
                .join("Library/Caches/powermove-engine/ffmpeg/out")
        })
        .join("bin/ffmpeg");
    let ok = Command::new(bin)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=96x54:rate=1",
        ])
        .args(["-frames:v", "1"])
        .args(args)
        .arg(out)
        .status()
        .expect("ffmpeg (PM_FFMPEG_DIR)")
        .success();
    assert!(ok, "ffmpeg {args:?}");
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                0xEDB8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Inserts an ancillary chunk after IHDR.
fn with_chunk(png: &[u8], ty: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let ihdr_end = 8 + 12 + 13;
    let mut out = png[..ihdr_end].to_vec();
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let mut typed = ty.to_vec();
    typed.extend_from_slice(body);
    out.extend_from_slice(&typed);
    out.extend_from_slice(&crc32(&typed).to_be_bytes());
    out.extend_from_slice(&png[ihdr_end..]);
    out
}

fn unsupported(path: &Path) -> String {
    match probe_image(path) {
        Err(ProbeIssue::Unsupported(Feature::Media(m))) => m,
        other => panic!(
            "{}: expected an unsupported still, got {other:?}",
            path.display()
        ),
    }
}

#[test]
fn untested_still_classes_are_unsupported() {
    let d = dir().join("other");
    std::fs::create_dir_all(&d).unwrap();
    let cases: &[(&str, &[&str], &str)] = &[
        ("a.bmp", &[], "BMP stills"),
        ("a.gif", &[], "GIF stills"),
        ("a.ico", &["-c:v", "bmp", "-f", "ico"], "ICO stills"),
        (
            "a.exr",
            &["-c:v", "exr", "-pix_fmt", "gbrpf32le"],
            "floating-point stills",
        ),
        (
            "a16.tiff",
            &["-pix_fmt", "rgb48le"],
            "TIFF stills deeper than 8 bits",
        ),
        (
            "alpha.tiff",
            &["-pix_fmt", "rgba"],
            "TIFF stills with alpha",
        ),
        ("mono.png", &["-pix_fmt", "monob"], "1-bit PNG stills"),
        ("a.dpx", &["-pix_fmt", "gbrp10le"], ""),
    ];
    for (name, args, message) in cases {
        let path = d.join(name);
        ffmpeg(&path, args);
        let got = unsupported(&path);
        assert!(got.contains(message), "{name}: {got}");
        eprintln!("{name:<12} unsupported: {got}");
    }
    // PNG gamma and chromaticity chunks Chromium honors and pm-media does
    // not: unsupported unless an sRGB chunk overrides them.
    let plain = d.join("plain.png");
    ffmpeg(&plain, &["-pix_fmt", "rgb24"]);
    assert_eq!(probe_image(&plain), Ok(()));
    let bytes = std::fs::read(&plain).unwrap();
    let gama = d.join("gama.png");
    std::fs::write(&gama, with_chunk(&bytes, b"gAMA", &55555u32.to_be_bytes())).unwrap();
    assert!(unsupported(&gama).contains("gAMA"));
    let overridden = d.join("gama-srgb.png");
    let srgb = with_chunk(&bytes, b"sRGB", &[0]);
    std::fs::write(
        &overridden,
        with_chunk(&srgb, b"gAMA", &45455u32.to_be_bytes()),
    )
    .unwrap();
    assert_eq!(probe_image(&overridden), Ok(()));
    let cicp = d.join("cicp.png");
    std::fs::write(&cicp, with_chunk(&bytes, b"cICP", &[9, 16, 0, 1])).unwrap();
    assert!(unsupported(&cicp).contains("CICP"));
    std::fs::remove_dir_all(&d).ok();
}

/// Y'CbCr JPEG (every layout) is unsupported: pm-media's decoder differs
/// from Chromium's libjpeg-turbo (docs/engine/pm-render.md). Gray JPEG is
/// drawn and compared in `tests/corpus.rs` (`render/still-photo-gray-jpg`);
/// header classes are covered by `stills::tests::jpeg_classes`.
#[test]
fn ycbcr_jpeg_is_unsupported() {
    let d = dir().join("jpeg");
    std::fs::create_dir_all(&d).unwrap();
    for fmt in ["yuvj420p", "yuvj422p", "yuvj444p"] {
        let path = d.join(format!("{fmt}.jpg"));
        ffmpeg(&path, &["-pix_fmt", fmt, "-q:v", "3"]);
        assert!(unsupported(&path).contains("Y'CbCr JPEG"), "{fmt}");
    }
    std::fs::remove_dir_all(&d).ok();
}

/// A big-endian EXIF payload (no `Exif\0\0` prefix) with one orientation
/// tag.
fn exif(orientation: u16) -> Vec<u8> {
    let mut b = b"MM\0*\0\0\0\x08\0\x01".to_vec();
    b.extend_from_slice(&[0x01, 0x12, 0, 3, 0, 0, 0, 1]);
    b.extend_from_slice(&orientation.to_be_bytes());
    b.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    b
}

/// Inserts an ancillary chunk before IEND.
fn with_chunk_at_end(png: &[u8], ty: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let iend = png.len() - 12;
    assert_eq!(&png[iend + 4..iend + 8], b"IEND");
    let mut out = png[..iend].to_vec();
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let mut typed = ty.to_vec();
    typed.extend_from_slice(body);
    out.extend_from_slice(&typed);
    out.extend_from_slice(&crc32(&typed).to_be_bytes());
    out.extend_from_slice(&png[iend..]);
    out
}

/// Inserts an `Exif` APP1 segment after SOI.
fn jpeg_with_exif(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let body = [b"Exif\0\0".to_vec(), exif(orientation)].concat();
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&body);
    out.extend_from_slice(&jpeg[2..]);
    out
}

/// Wraps a simple-format (`VP8 `) WebP in the extended format with an
/// `EXIF` chunk after the image.
fn webp_with_exif(simple: &[u8], orientation: u16) -> Vec<u8> {
    assert_eq!(&simple[12..16], b"VP8 ", "a simple lossy WebP");
    let frame = &simple[12..];
    let w = u16::from_le_bytes([simple[26], simple[27]]) as u32 & 0x3FFF;
    let h = u16::from_le_bytes([simple[28], simple[29]]) as u32 & 0x3FFF;
    let mut vp8x = vec![0x08, 0, 0, 0];
    vp8x.extend_from_slice(&(w - 1).to_le_bytes()[..3]);
    vp8x.extend_from_slice(&(h - 1).to_le_bytes()[..3]);
    let mut body = b"WEBP".to_vec();
    body.extend_from_slice(b"VP8X\x0a\0\0\0");
    body.extend_from_slice(&vp8x);
    body.extend_from_slice(frame);
    let e = exif(orientation);
    body.extend_from_slice(b"EXIF");
    body.extend_from_slice(&(e.len() as u32).to_le_bytes());
    body.extend_from_slice(&e);
    if e.len() & 1 == 1 {
        body.push(0);
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// How [`imageio_write`] encodes.
#[derive(Default)]
struct ImageIo<'a> {
    /// EXIF orientation property (1 when `None`).
    orientation: Option<u16>,
    /// Color space the image is tagged with before encoding: `device`
    /// (ImageIO embeds no profile), `srgb`, `p3`, `adobe`; the source's
    /// when `None` (ImageIO embeds sRGB for an untagged PNG).
    color_space: Option<&'a str>,
    /// TIFF compression tag value.
    tiff_compression: Option<u16>,
    /// TIFF tile width and height.
    tiff_tile: Option<u16>,
    /// Progressive JPEG.
    progressive: bool,
}

/// Re-encodes `src` with ImageIO (`uti`, e.g. `public.heic`), as Preview,
/// Photos, or a camera export does. ImageIO writes HEIF orientation as an
/// `irot`/`imir` property and TIFF orientation as tag 274.
fn imageio_write(src: &Path, out: &Path, uti: &str, o: &ImageIo) {
    let script = out.with_extension("swift");
    std::fs::write(
        &script,
        r#"import Foundation
import ImageIO
import CoreGraphics
let a = CommandLine.arguments
let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: a[1]) as CFURL, nil)!
var img = CGImageSourceCreateImageAtIndex(src, 0, nil)!
let spaces: [String: CGColorSpace] = [
  "device": CGColorSpaceCreateDeviceRGB(),
  "srgb": CGColorSpace(name: CGColorSpace.sRGB)!,
  "p3": CGColorSpace(name: CGColorSpace.displayP3)!,
  "adobe": CGColorSpace(name: CGColorSpace.adobeRGB1998)!,
]
if a[4] != "-" { img = img.copy(colorSpace: spaces[a[4]]!)! }
var props: [CFString: Any] = [kCGImagePropertyOrientation: Int(a[5])!]
var tiff: [CFString: Any] = [:]
if a[6] != "-" { tiff[kCGImagePropertyTIFFCompression] = Int(a[6])! }
if a[7] != "-" {
  tiff[kCGImagePropertyTIFFTileWidth] = Int(a[7])!
  tiff[kCGImagePropertyTIFFTileLength] = Int(a[7])!
}
if !tiff.isEmpty { props[kCGImagePropertyTIFFDictionary] = tiff }
if a[8] == "1" { props[kCGImagePropertyJFIFDictionary] = [kCGImagePropertyJFIFIsProgressive: true] }
let dst = CGImageDestinationCreateWithURL(URL(fileURLWithPath: a[2]) as CFURL, a[3] as CFString, 1, nil)!
CGImageDestinationAddImage(dst, img, props as CFDictionary)
exit(CGImageDestinationFinalize(dst) ? 0 : 1)
"#,
    )
    .unwrap();
    let opt = |v: Option<String>| v.unwrap_or_else(|| "-".into());
    let ok = Command::new("/usr/bin/swift")
        .arg(&script)
        .arg(src)
        .arg(out)
        .arg(uti)
        .arg(o.color_space.unwrap_or("-"))
        .arg(o.orientation.unwrap_or(1).to_string())
        .arg(opt(o.tiff_compression.map(|c| c.to_string())))
        .arg(opt(o.tiff_tile.map(|t| t.to_string())))
        .arg(if o.progressive { "1" } else { "0" })
        .status()
        .expect("/usr/bin/swift (Xcode command line tools) writes the ImageIO cases")
        .success();
    assert!(ok, "ImageIO {uti} -> {}", out.display());
}

fn imageio_with_orientation(src: &Path, out: &Path, uti: &str, orientation: u16) {
    imageio_write(
        src,
        out,
        uti,
        &ImageIo {
            orientation: Some(orientation),
            ..Default::default()
        },
    );
}

/// The first IFD's entries of a TIFF file, as (byte offset of the entry,
/// tag), for patching test files.
fn tiff_entries(b: &[u8]) -> Vec<(usize, u16)> {
    let le = &b[..2] == b"II";
    let u16_ = |at: usize| {
        let v = [b[at], b[at + 1]];
        if le {
            u16::from_le_bytes(v)
        } else {
            u16::from_be_bytes(v)
        }
    };
    let v = [b[4], b[5], b[6], b[7]];
    let ifd = if le {
        u32::from_le_bytes(v)
    } else {
        u32::from_be_bytes(v)
    } as usize;
    (0..u16_(ifd) as usize)
        .map(|i| (ifd + 2 + 12 * i, u16_(ifd + 2 + 12 * i)))
        .collect()
}

/// Sets a SHORT scalar entry of the first IFD in place.
fn tiff_set_short(b: &mut [u8], tag: u16, value: u16) {
    let le = &b[..2] == b"II";
    let (at, _) = tiff_entries(b)
        .into_iter()
        .find(|e| e.1 == tag)
        .unwrap_or_else(|| panic!("tag {tag}"));
    let v = if le {
        value.to_le_bytes()
    } else {
        value.to_be_bytes()
    };
    b[at + 8..at + 10].copy_from_slice(&v);
}

fn decoded_orientation(path: &Path) -> u8 {
    let pool = pm_media::pool::FramePool::new(0);
    pm_media::image::read(path, &pool)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .orientation
}

/// No oriented still of a drawn class has been compared with the legacy
/// renderer, so EXIF orientation (PNG `eXIf`, JPEG APP1, WebP `EXIF`, TIFF
/// tag 274) and HEIF transform properties other than the identity are
/// unsupported in every container, whether or not pm-media's decoder
/// reports them. The unrotated controls are drawn.
#[test]
fn oriented_stills_are_unsupported() {
    let d = dir().join("oriented");
    std::fs::create_dir_all(&d).unwrap();
    let mut cases: Vec<PathBuf> = Vec::new();
    let write = |name: &str, bytes: &[u8]| {
        let p = d.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    };

    // PNG: eXIf before the image data (pm-media applies it) and after it.
    let plain = d.join("plain.png");
    ffmpeg(&plain, &["-pix_fmt", "rgb24"]);
    assert_eq!(probe_image(&plain), Ok(()));
    let png = std::fs::read(&plain).unwrap();
    assert_eq!(
        probe_image(&write("exif1.png", &with_chunk(&png, b"eXIf", &exif(1)))),
        Ok(())
    );
    cases.push(write("exif6.png", &with_chunk(&png, b"eXIf", &exif(6))));
    cases.push(write(
        "exif6-late.png",
        &with_chunk_at_end(&png, b"eXIf", &exif(6)),
    ));
    cases.push(write("exif2.png", &with_chunk(&png, b"eXIf", &exif(2))));

    // Gray JPEG (the drawn JPEG class; FFmpeg's encoder writes only
    // Y'CbCr, ImageIO keeps a gray source gray).
    let gray_png = d.join("gray.png");
    ffmpeg(&gray_png, &["-pix_fmt", "gray"]);
    let gray = d.join("gray.jpg");
    imageio_with_orientation(&gray_png, &gray, "public.jpeg", 1);
    assert_eq!(probe_image(&gray), Ok(()), "gray JPEG orientation 1");
    let jpeg = std::fs::read(&gray).unwrap();
    cases.push(write("gray-exif6.jpg", &jpeg_with_exif(&jpeg, 6)));
    for o in [6, 8] {
        let p = d.join(format!("gray-o{o}.jpg"));
        imageio_with_orientation(&gray_png, &p, "public.jpeg", o);
        cases.push(p);
    }

    // Opaque lossy WebP.
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../apps/desktop/e2e/fixtures");
    let webp = std::fs::read(fixtures.join("still.webp")).expect("e2e fixture still.webp");
    assert_eq!(
        probe_image(&write("exif1.webp", &webp_with_exif(&webp, 1))),
        Ok(())
    );
    cases.push(write("exif6.webp", &webp_with_exif(&webp, 6)));

    // 8-bit opaque TIFF and HEIC written by ImageIO. The TIFF is an
    // untagged device RGB image (ImageIO embeds a profile otherwise, which
    // routes the file for another reason; see `tagged_tiff_is_unsupported`).
    for (ext, uti, color_space) in [
        ("tiff", "public.tiff", Some("device")),
        ("heic", "public.heic", None),
    ] {
        for o in [1, 3, 6, 7] {
            let p = d.join(format!("o{o}.{ext}"));
            let opts = ImageIo {
                orientation: Some(o),
                color_space,
                ..Default::default()
            };
            imageio_write(&plain, &p, uti, &opts);
            if o == 1 {
                assert_eq!(probe_image(&p), Ok(()), "{ext} orientation 1");
            } else {
                cases.push(p);
            }
        }
    }

    // The container header parse finds every transform on its own, also
    // where the decoder reports the orientation first (ImageIO's HEIC and
    // TIFF, PNG eXIf before the image data).
    for p in &cases {
        let format = match p.extension().unwrap().to_str().unwrap() {
            "png" => pm_media::image::ImageFormat::Png,
            "jpg" => pm_media::image::ImageFormat::Jpeg,
            "webp" => pm_media::image::ImageFormat::WebP,
            "tiff" => pm_media::image::ImageFormat::Tiff,
            _ => pm_media::image::ImageFormat::Heif,
        };
        let t = pm_render::stills::container_transform(p, format);
        assert!(
            matches!(t, Ok(Some(_))),
            "{}: header parse found {t:?}",
            p.display()
        );
    }
    for o in [1, 3, 6, 7] {
        let heic = d.join(format!("o{o}.heic"));
        let t = pm_render::stills::container_transform(&heic, pm_media::image::ImageFormat::Heif)
            .unwrap();
        eprintln!("o{o}.heic header transform: {t:?}");
        assert_eq!(t.is_some(), o != 1, "o{o}.heic: {t:?}");
    }

    for p in &cases {
        let got = unsupported(p);
        assert!(got.contains("oriented stills"), "{}: {got}", p.display());
        eprintln!(
            "{:<16} decoder orientation {}: {got}",
            p.file_name().unwrap().to_string_lossy(),
            decoded_orientation(p)
        );
    }
    std::fs::remove_dir_all(&d).ok();
}

/// The RGBA8 pixels pm-media decodes, tightly packed.
fn decoded_rgba8(path: &Path) -> Vec<u8> {
    let pool = pm_media::pool::FramePool::new(0);
    let still =
        pm_media::image::read(path, &pool).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(still.bit_depth, 8, "{}", path.display());
    still
        .frame
        .with_planes(|planes| {
            let p = &planes[0];
            let row = p.desc.width * 4;
            let mut out = Vec::with_capacity(row * p.desc.height);
            for y in 0..p.desc.height {
                // SAFETY: the plane view covers `height` rows of `stride` bytes.
                out.extend_from_slice(unsafe {
                    std::slice::from_raw_parts(p.ptr.add(y * p.stride), row)
                });
            }
            out
        })
        .unwrap()
}

/// The TIFF class pm-render draws was compared with the legacy renderer on
/// an uncompressed file (`render/still-photo-tiff`). The lossless
/// compressions it also accepts (LZW with and without predictor 2, both
/// Deflate tags, PackBits) decode, in pm-media, to the same pixels as the
/// uncompressed file (and FFmpeg, the legacy converter, decodes them
/// losslessly by definition), so they are the same class. Tiled and gray
/// files are routed (palette, Y'CbCr, planar, fill order, sample format,
/// and colorimetry tags are covered by `stills::tests::tiff_classes`;
/// pm-media cannot read FFmpeg's palette or Y'CbCr TIFF at all).
#[test]
fn tiff_compressions_decode_alike() {
    let d = dir().join("tiff");
    std::fs::create_dir_all(&d).unwrap();
    let raw = d.join("raw.tiff");
    ffmpeg(&raw, &["-pix_fmt", "rgb24", "-compression_algo", "raw"]);
    assert_eq!(probe_image(&raw), Ok(()));
    let reference = decoded_rgba8(&raw);
    for algo in ["lzw", "deflate", "packbits"] {
        let p = d.join(format!("{algo}.tiff"));
        ffmpeg(&p, &["-pix_fmt", "rgb24", "-compression_algo", algo]);
        assert_eq!(probe_image(&p), Ok(()), "{algo}");
        assert!(decoded_rgba8(&p) == reference, "{algo} decodes differently");
    }
    // Adobe Deflate (compression 8) carries the same zlib stream as the
    // Deflate FFmpeg writes (32946); only the tag differs.
    let mut adobe = std::fs::read(d.join("deflate.tiff")).unwrap();
    tiff_set_short(&mut adobe, 259, 8);
    let adobe_deflate = d.join("adobe-deflate.tiff");
    std::fs::write(&adobe_deflate, adobe).unwrap();
    assert_eq!(probe_image(&adobe_deflate), Ok(()));
    assert!(
        decoded_rgba8(&adobe_deflate) == reference,
        "Adobe Deflate decodes differently"
    );
    // ImageIO's LZW with horizontal differencing (predictor 2) and
    // per-sample arrays, as Preview and Photos write it; untagged (an
    // uncalibrated device RGB image), so in the class.
    let imageio = d.join("imageio-lzw.tiff");
    let lzw = ImageIo {
        color_space: Some("device"),
        tiff_compression: Some(5),
        ..Default::default()
    };
    imageio_write(&raw, &imageio, "public.tiff", &lzw);
    let tags: Vec<u16> = tiff_entries(&std::fs::read(&imageio).unwrap())
        .into_iter()
        .map(|e| e.1)
        .collect();
    assert!(tags.contains(&317) && !tags.contains(&34675), "{tags:?}");
    assert_eq!(probe_image(&imageio), Ok(()));
    assert!(
        decoded_rgba8(&imageio) == reference,
        "ImageIO LZW decodes differently"
    );
    eprintln!(
        "ImageIO LZW TIFF (predictor 2, no profile) drawn, decodes like the uncompressed file"
    );
    // Tiled TIFF was not compared with the legacy renderer.
    let tiled = d.join("imageio-tiled.tiff");
    imageio_write(
        &raw,
        &tiled,
        "public.tiff",
        &ImageIo {
            color_space: Some("device"),
            tiff_compression: Some(1),
            tiff_tile: Some(16),
            ..Default::default()
        },
    );
    let got = unsupported(&tiled);
    assert!(got.contains("tiled"), "{got}");
    eprintln!("imageio-tiled.tiff unsupported: {got}");
    let gray = d.join("gray.tiff");
    ffmpeg(&gray, &["-pix_fmt", "gray"]);
    let got = unsupported(&gray);
    assert!(got.contains("photometric interpretation 1"), "{got}");
    eprintln!("gray.tiff    unsupported: {got}");
    std::fs::remove_dir_all(&d).ok();
}

/// A TIFF with an ICC profile (tag 34675) is unsupported whatever the
/// profile: pm-media's TIFF decoder does not report it (so a wide-gamut
/// TIFF would otherwise be drawn as untagged sRGB), and no tagged TIFF was
/// compared with the legacy renderer. ImageIO embeds a profile in every
/// TIFF of a tagged image, including sRGB for an untagged source.
#[test]
fn tagged_tiff_is_unsupported() {
    let d = dir().join("tagged-tiff");
    std::fs::create_dir_all(&d).unwrap();
    let plain = d.join("plain.png");
    ffmpeg(&plain, &["-pix_fmt", "rgb24"]);
    for (space, compression) in [
        (Some("adobe"), 5),
        (Some("p3"), 5),
        (Some("srgb"), 5),
        (Some("p3"), 1),
        (None, 1),
    ] {
        let name = format!("{}-{compression}.tiff", space.unwrap_or("source"));
        let p = d.join(&name);
        imageio_write(
            &plain,
            &p,
            "public.tiff",
            &ImageIo {
                color_space: space,
                tiff_compression: Some(compression),
                ..Default::default()
            },
        );
        let tags: Vec<u16> = tiff_entries(&std::fs::read(&p).unwrap())
            .into_iter()
            .map(|e| e.1)
            .collect();
        assert!(
            tags.contains(&34675),
            "{name}: ImageIO wrote no profile: {tags:?}"
        );
        let got = unsupported(&p);
        assert!(got.contains("ICC profile"), "{name}: {got}");
        eprintln!("{name:<16} unsupported: {got}");
    }
    std::fs::remove_dir_all(&d).ok();
}

/// Gray JPEG is drawn only as baseline, the structure compared with the
/// legacy renderer (`render/still-photo-gray-jpg`); progressive gray is
/// routed.
#[test]
fn progressive_gray_jpeg_is_unsupported() {
    let d = dir().join("gray-jpeg");
    std::fs::create_dir_all(&d).unwrap();
    let gray_png = d.join("gray.png");
    ffmpeg(&gray_png, &["-pix_fmt", "gray"]);
    let baseline = d.join("baseline.jpg");
    imageio_write(&gray_png, &baseline, "public.jpeg", &ImageIo::default());
    assert_eq!(probe_image(&baseline), Ok(()));
    let progressive = d.join("progressive.jpg");
    imageio_write(
        &gray_png,
        &progressive,
        "public.jpeg",
        &ImageIo {
            progressive: true,
            ..Default::default()
        },
    );
    let bytes = std::fs::read(&progressive).unwrap();
    assert!(
        bytes.windows(2).any(|w| w == [0xFF, 0xC2]),
        "ImageIO wrote no progressive frame header"
    );
    let got = unsupported(&progressive);
    assert!(got.contains("progressive gray"), "{got}");
    eprintln!("progressive gray JPEG unsupported: {got}");
    std::fs::remove_dir_all(&d).ok();
}
