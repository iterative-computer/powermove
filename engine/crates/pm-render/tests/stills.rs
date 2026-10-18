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

/// Y'CbCr JPEG uses Chromium's libjpeg-turbo path; gray and Y'CbCr layouts
/// are compared in `tests/corpus.rs` and accepted by the media probe.
#[test]
fn ycbcr_jpeg_is_drawn() {
    // Decoded by libjpeg-turbo, as Chromium decodes it (corpus
    // `render/still-photo-*-jpg` within 1 CV).
    let d = dir().join("jpeg");
    std::fs::create_dir_all(&d).unwrap();
    for fmt in ["yuvj420p", "yuvj422p", "yuvj444p"] {
        let path = d.join(format!("{fmt}.jpg"));
        ffmpeg(&path, &["-pix_fmt", fmt, "-q:v", "3"]);
        assert_eq!(probe_image(&path), Ok(()), "{fmt}");
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

/// The compatibility bitmap consumes the decoder's applied orientation
/// where Chromium applies it (PNG eXIf before the image data, JPEG, TIFF,
/// HEIF; compared in `e2e/native-still-orientation-parity.spec.ts`). WebP
/// EXIF is ignored, as Blink ignores it. Late PNG eXIf is still gated when
/// the decoder did not apply it.
#[test]
fn decoded_orientation_is_drawn_and_unapplied_late_exif_stays_gated() {
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
    let exif6_webp = write("exif6.webp", &webp_with_exif(&webp, 6));
    cases.push(exif6_webp.clone());
    // Chromium shows a WebP as stored whatever its EXIF says.
    let plain_webp = write("plain.webp", &webp);
    assert!(
        compat_rgba8(&exif6_webp) == compat_rgba8(&plain_webp),
        "WebP EXIF orientation must not be applied"
    );
    {
        let pool = pm_media::pool::FramePool::new(0);
        let s = pm_media::image::read(&exif6_webp, &pool).unwrap();
        assert_eq!(s.orientation, 1);
        assert_eq!(
            (s.frame.format.width, s.frame.format.height),
            (160, 90),
            "the box today's import measured"
        );
    }

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
        if p.file_name().unwrap() == "exif6-late.png" {
            assert!(unsupported(p).contains("oriented stills"));
        } else {
            assert_eq!(probe_image(p), Ok(()), "{}", p.display());
        }
    }
    // Applied orientations are pixel permutations of the unoriented
    // bitmap (so is Chromium's oriented ImageBitmap; media.md).
    let plain_rgba = compat_rgba8(&plain);
    for (name, o) in [("exif6.png", 6u8), ("exif2.png", 2)] {
        let expect = orient_rgba8(&plain_rgba, 96, 54, o);
        assert!(compat_rgba8(&d.join(name)) == expect, "{name}");
    }
    std::fs::remove_dir_all(&d).ok();
}

/// EXIF orientation applied to packed RGBA8, placing each source pixel as
/// the EXIF definitions (and Chromium's `ImageOrientation`) do.
fn orient_rgba8(px: &[u8], w: usize, h: usize, o: u8) -> Vec<u8> {
    let (ow, oh) = if o >= 5 { (h, w) } else { (w, h) };
    let mut out = Vec::with_capacity(px.len());
    for oy in 0..oh {
        for ox in 0..ow {
            let (sx, sy) = match o {
                2 => (w - 1 - ox, oy),
                3 => (w - 1 - ox, h - 1 - oy),
                4 => (ox, h - 1 - oy),
                5 => (oy, ox),
                6 => (oy, h - 1 - ox),
                7 => (w - 1 - oy, h - 1 - ox),
                8 => (w - 1 - oy, ox),
                _ => (ox, oy),
            };
            let i = (sy * w + sx) * 4;
            out.extend_from_slice(&px[i..i + 4]);
        }
    }
    out
}

/// The level-1 AVIF bitmap follows Blink's `AVIFImageDecoder`: `irot` and
/// `imir` become the orientation (`kAxisAngleToOrientation`), a valid
/// `clap` at (0, 0) crops, any other `clap` shows the whole image; 8-bit
/// 4:2:0 at either range is the compared class (app comparison:
/// `e2e/native-still-orientation-parity.spec.ts`).
#[test]
fn avif_container_transforms_follow_blink() {
    let d = dir().join("avif");
    std::fs::create_dir_all(&d).unwrap();
    let src = d.join("src.png");
    ffmpeg(&src, &["-pix_fmt", "rgb24"]); // 96x54
    let odd_src = d.join("odd.png");
    // ImageIO codes 95x63 as 96x64 with a clean aperture.
    ffmpeg(
        &odd_src,
        &["-pix_fmt", "rgb24", "-vf", "scale=96:64,crop=95:63:0:0"],
    );
    let plain = d.join("plain.avif");
    imageio_write(&src, &plain, "public.avif", &ImageIo::default());
    let odd = d.join("odd.avif");
    imageio_write(&odd_src, &odd, "public.avif", &ImageIo::default());
    let bytes = std::fs::read(&plain).unwrap();
    // A property box by its size and type (`irot` 9 bytes, `clap` 40).
    let find = |b: &[u8], size: u8, ty: &[u8]| {
        let mut head = vec![0, 0, 0, size];
        head.extend_from_slice(ty);
        4 + b.windows(8).position(|w| w == &head[..]).expect("box")
    };
    let patch = |name: &str, f: &dyn Fn(&mut Vec<u8>)| {
        let mut b = bytes.clone();
        f(&mut b);
        let p = d.join(name);
        std::fs::write(&p, b).unwrap();
        p
    };
    let compat = |p: &Path| {
        let pool = pm_media::pool::FramePool::new(0);
        let original = pm_media::image::read(p, &pool).unwrap();
        assert!(
            pm_media::image::avif_compat::applies(&original),
            "{}",
            p.display()
        );
        let s = pm_media::image::avif_compat::read(p, &original, &pool).unwrap();
        let (w, h) = (s.frame.format.width, s.frame.format.height);
        let px = s
            .frame
            .with_planes(|planes| {
                let p = &planes[0];
                let mut out = Vec::new();
                for y in 0..h {
                    out.extend_from_slice(unsafe {
                        std::slice::from_raw_parts(p.ptr.add(y * p.stride), w * 4)
                    });
                }
                out
            })
            .unwrap();
        (px, w, h, s.orientation)
    };
    assert_eq!(probe_image(&plain), Ok(()));
    let (base, w, h, o) = compat(&plain);
    assert_eq!((w, h, o), (96, 54, 1));
    let irot = find(&bytes, 9, b"irot");
    for (angle, exif) in [(1u8, 8u8), (2, 3), (3, 6)] {
        let p = patch(&format!("irot{angle}.avif"), &|b| b[irot + 4] = angle);
        assert_eq!(probe_image(&p), Ok(()), "irot {angle}");
        let (px, ..) = compat(&p);
        assert!(px == orient_rgba8(&base, w, h, exif), "irot {angle}");
    }
    for (axis, exif) in [(0u8, 4u8), (1, 2)] {
        let p = patch(&format!("imir{axis}.avif"), &|b| {
            b[irot..irot + 4].copy_from_slice(b"imir");
            b[irot + 4] = axis;
        });
        assert_eq!(probe_image(&p), Ok(()), "imir {axis}");
        let (px, ..) = compat(&p);
        assert!(px == orient_rgba8(&base, w, h, exif), "imir {axis}");
    }
    // Blink accepts AV1 items without `pixi` (libheif 1.11 and older): an
    // unknown property in its place leaves the item without one.
    let pixi = find(&bytes, 16, b"pixi");
    let p = patch("no-pixi.avif", &|b| {
        b[pixi..pixi + 4].copy_from_slice(b"xixi")
    });
    assert_eq!(probe_image(&p), Ok(()), "no pixi");
    assert!(compat(&p).0 == base, "no pixi");
    // ImageIO writes a clap anchored at (0, 0) for odd sizes.
    assert_eq!(probe_image(&odd), Ok(()));
    let (_, cw, ch, _) = compat(&odd);
    assert_eq!((cw, ch), (95, 63), "clap crops");
    let mut shifted = std::fs::read(&odd).unwrap();
    let clap = find(&shifted, 40, b"clap") + 4;
    let n = i32::from_be_bytes(shifted[clap + 16..clap + 20].try_into().unwrap());
    let den = i32::from_be_bytes(shifted[clap + 20..clap + 24].try_into().unwrap());
    shifted[clap + 16..clap + 20].copy_from_slice(&(n + den).to_be_bytes());
    let shifted_path = d.join("clap-shifted.avif");
    std::fs::write(&shifted_path, shifted).unwrap();
    assert_eq!(probe_image(&shifted_path), Ok(()));
    let (_, sw, sh, _) = compat(&shifted_path);
    assert_eq!(
        (sw, sh),
        (96, 64),
        "an off-origin clap shows the whole image"
    );

    // Limited range (the app fixture) is in the class.
    let limited = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../apps/desktop/e2e/fixtures/still-limited-range.avif");
    assert_eq!(probe_image(&limited), Ok(()));
    assert_eq!(
        pm_media::image::avif_compat::unmeasured(&plain).unwrap(),
        None
    );
    std::fs::remove_dir_all(&d).ok();
}

/// The level-1 AVIF bitmap, tightly packed.
fn avif_compat_rgba8(path: &Path) -> Vec<u8> {
    let pool = pm_media::pool::FramePool::new(0);
    let original = pm_media::image::read(path, &pool).unwrap();
    assert!(
        pm_media::image::avif_compat::applies(&original),
        "{}",
        path.display()
    );
    let s = pm_media::image::avif_compat::read(path, &original, &pool)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    s.frame
        .with_planes(|planes| {
            let p = &planes[0];
            let mut out = Vec::new();
            for y in 0..p.desc.height {
                out.extend_from_slice(unsafe {
                    std::slice::from_raw_parts(p.ptr.add(y * p.stride), p.desc.width * 4)
                });
            }
            out
        })
        .unwrap()
}

/// Blink gives an AVIF without an ICC profile the sRGB profile when its
/// container's primaries are BT.709 (or unspecified) and its transfer is
/// sRGB, BT.709, SMPTE 170M, either BT.2020 transfer, or unspecified
/// (`gfx::ColorSpace` gives all of those the sRGB curve), so those draw the
/// same bitmap as sRGB-tagged files; FFmpeg and many tools write transfer
/// 1 for BT.709 sources. Chromium converts every other description, which
/// stays a named unsupported class, never a layer failure (app comparison:
/// `e2e/native-still-orientation-parity.spec.ts`).
#[test]
fn avif_color_descriptions_blink_reads_as_srgb_are_drawn() {
    let d = dir().join("avif-cicp");
    std::fs::create_dir_all(&d).unwrap();
    let encode = |p: u8, t: u8, m: u8| {
        let out = d.join(format!("cicp-{p}-{t}-{m}.avif"));
        // `setparams` reaches the muxer's `colr` box (encoder options do
        // not).
        let vf = format!(
            "setparams=color_primaries={p}:color_trc={t}:colorspace={m}:range=pc,format=yuv420p"
        );
        ffmpeg(
            &out,
            &[
                "-vf",
                &vf,
                "-c:v",
                "libsvtav1",
                "-preset",
                "8",
                "-crf",
                "30",
            ],
        );
        out
    };
    let srgb = avif_compat_rgba8(&encode(1, 13, 1));
    for t in [1, 6, 14, 15] {
        let p = encode(1, t, 1);
        assert_eq!(probe_image(&p), Ok(()), "{}", p.display());
        assert!(avif_compat_rgba8(&p) == srgb, "{}", p.display());
    }
    // Unspecified primaries read as BT.709 (matrix 2 reads as BT.601 in
    // both decoders, so compare with the unspecified file).
    let unspecified = avif_compat_rgba8(&encode(2, 2, 2));
    let p = encode(2, 1, 2);
    assert_eq!(probe_image(&p), Ok(()));
    assert!(avif_compat_rgba8(&p) == unspecified);
    // Matrices are CrabbyAvif's in both decoders.
    for (t, m) in [(6, 6), (14, 9)] {
        let p = encode(1, t, m);
        assert_eq!(probe_image(&p), Ok(()), "{}", p.display());
    }
    // Linear, gamma 2.2, SMPTE 240M, xvYCC, BT.1361, PQ and HLG transfers
    // and BT.601, BT.470 and BT.2020 primaries are converted by Chromium:
    // a named unsupported class whichever profile ImageIO reports.
    for (p, t, m) in [
        (1, 4, 1),
        (1, 7, 1),
        (1, 8, 1),
        (1, 11, 1),
        (1, 12, 1),
        (1, 16, 1),
        (1, 18, 1),
        (6, 6, 6),
        (5, 13, 5),
        (9, 1, 9),
    ] {
        let f = encode(p, t, m);
        let why = unsupported(&f);
        eprintln!("{p}/{t}/{m}: {why}");
        assert!(why.starts_with("AVIF stills "), "{}: {why}", f.display());
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

fn compat_rgba8(path: &Path) -> Vec<u8> {
    let pool = pm_media::pool::FramePool::new(0);
    let original = pm_media::image::read(path, &pool).unwrap();
    let bitmap = pm_media::image::compat::bitmap(&original, &pool).unwrap();
    bitmap
        .frame
        .with_planes(|planes| {
            let p = &planes[0];
            let mut out = Vec::new();
            for y in 0..p.desc.height {
                out.extend_from_slice(unsafe {
                    std::slice::from_raw_parts(p.ptr.add(y * p.stride), p.desc.width * 4)
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

/// RGB TIFF's shipped FFmpeg conversion forwards ICC to its PNG. The
/// compatibility bitmap must be byte-identical to that PNG's, while the
/// native-depth reader retains the source ICC for level 2/interpretation.
#[test]
fn tagged_tiff_matches_the_shipped_png_conversion_and_retains_native_icc() {
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
        assert_eq!(probe_image(&p), Ok(()), "{name}");
        let pool = pm_media::pool::FramePool::new(0);
        let original = pm_media::image::read(&p, &pool).unwrap();
        assert!(
            original.color.icc.as_ref().is_some_and(|p| !p.is_empty()),
            "native ICC retained: {name}"
        );
        let converted = d.join(format!("{name}.png"));
        let bin = PathBuf::from(std::env::var("PM_FFMPEG_DIR").unwrap()).join("bin/ffmpeg");
        assert!(
            Command::new(bin)
                .args(["-v", "error", "-y", "-i"])
                .arg(&p)
                .args(["-frames:v", "1", "-c:v", "png"])
                .arg(&converted)
                .status()
                .unwrap()
                .success()
        );
        let (a, b) = (compat_rgba8(&p), compat_rgba8(&converted));
        let max = a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
        assert!(max == 0, "{name}: shipped FFmpeg PNG bitmap max {max}");
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

#[test]
fn integer_and_alpha_tiff_bitmaps_match_the_shipped_png_conversion() {
    let d = dir().join("integer-tiff");
    std::fs::create_dir_all(&d).unwrap();
    let bin = PathBuf::from(std::env::var("PM_FFMPEG_DIR").unwrap()).join("bin/ffmpeg");
    for fmt in ["rgb48le", "rgba", "rgba64le"] {
        let tiff = d.join(format!("{fmt}.tiff"));
        let mut args = Vec::new();
        if fmt.starts_with("rgba") {
            args.extend([
                "-vf",
                "format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='X/W*255'",
            ]);
        }
        args.extend(["-pix_fmt", fmt]);
        ffmpeg(&tiff, &args);
        assert_eq!(probe_image(&tiff), Ok(()));
        let png = d.join(format!("{fmt}.png"));
        assert!(
            Command::new(&bin)
                .args(["-v", "error", "-y", "-i"])
                .arg(&tiff)
                .args(["-frames:v", "1", "-c:v", "png"])
                .arg(&png)
                .status()
                .unwrap()
                .success()
        );
        let (a, b) = (compat_rgba8(&tiff), compat_rgba8(&png));
        if fmt.starts_with("rgba") {
            assert!(a.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
        }
        let max = a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
        assert!(
            max == 0,
            "{fmt} TIFF bitmap differs from its shipped PNG by {max}"
        );
    }
    std::fs::remove_dir_all(d).unwrap();
}

/// The extra alpha-ramp app capture fails the unchanged 2D bar. Keep that
/// combination explicit without excluding opaque RGBA files from ICC support.
#[test]
fn wide_gamut_alpha_keeps_its_gate_but_opaque_rgba_is_supported() {
    let d = dir().join("icc-alpha");
    std::fs::create_dir_all(&d).unwrap();
    for (label, alpha) in [("opaque", 255u8), ("translucent", 128)] {
        let source = d.join(format!("{label}.png"));
        let mut encoder = png::Encoder::new(std::fs::File::create(&source).unwrap(), 4, 4);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[192, 96, 32, alpha].repeat(16))
            .unwrap();
        for space in ["p3", "adobe"] {
            for (ext, uti) in [("png", "public.png"), ("tiff", "public.tiff")] {
                let path = d.join(format!("{label}-{space}.{ext}"));
                imageio_write(
                    &source,
                    &path,
                    uti,
                    &ImageIo {
                        color_space: Some(space),
                        ..Default::default()
                    },
                );
                if alpha == 255 {
                    assert_eq!(probe_image(&path), Ok(()));
                } else {
                    assert!(unsupported(&path).contains("wide-gamut ICC stills with alpha"));
                }
            }
        }
    }
    std::fs::remove_dir_all(d).unwrap();
}
