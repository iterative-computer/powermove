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

/// Re-encodes `src` with ImageIO (`uti`, e.g. `public.heic`) and the given
/// orientation property, as a camera or Photos export does. ImageIO writes
/// HEIF orientation as an `irot`/`imir` property and TIFF orientation as
/// tag 274.
fn imageio_with_orientation(src: &Path, out: &Path, uti: &str, orientation: u16) {
    let script = out.with_extension("swift");
    std::fs::write(
        &script,
        r#"import Foundation
import ImageIO
let a = CommandLine.arguments
let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: a[1]) as CFURL, nil)!
let img = CGImageSourceCreateImageAtIndex(src, 0, nil)!
let dst = CGImageDestinationCreateWithURL(URL(fileURLWithPath: a[2]) as CFURL, a[3] as CFString, 1, nil)!
CGImageDestinationAddImage(dst, img, [kCGImagePropertyOrientation: Int(a[4])!] as CFDictionary)
exit(CGImageDestinationFinalize(dst) ? 0 : 1)
"#,
    )
    .unwrap();
    let ok = Command::new("/usr/bin/swift")
        .arg(&script)
        .arg(src)
        .arg(out)
        .arg(uti)
        .arg(orientation.to_string())
        .status()
        .expect("/usr/bin/swift (Xcode command line tools) writes the ImageIO cases")
        .success();
    assert!(ok, "ImageIO {uti} orientation {orientation}");
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

    // 8-bit opaque TIFF and HEIC written by ImageIO.
    for (ext, uti) in [("tiff", "public.tiff"), ("heic", "public.heic")] {
        let control = d.join(format!("o1.{ext}"));
        imageio_with_orientation(&plain, &control, uti, 1);
        assert_eq!(probe_image(&control), Ok(()), "{ext} orientation 1");
        for o in [3, 6, 7] {
            let p = d.join(format!("o{o}.{ext}"));
            imageio_with_orientation(&plain, &p, uti, o);
            cases.push(p);
        }
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
/// compressions it also accepts decode, in pm-media, to the same pixels as
/// the uncompressed file (and FFmpeg, the legacy converter, decodes them
/// losslessly by definition), so they are the same class. Gray files
/// written by FFmpeg are routed (palette, Y'CbCr, and planar TIFF are
/// covered by `stills::tests::tiff_classes`; pm-media cannot read FFmpeg's
/// palette or Y'CbCr TIFF at all).
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
    // ImageIO's LZW with horizontal differencing (predictor 2), as Preview
    // and Photos write it.
    let imageio = d.join("imageio-lzw.tiff");
    let script = d.join("lzw.swift");
    std::fs::write(
        &script,
        r#"import Foundation
import ImageIO
let a = CommandLine.arguments
let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: a[1]) as CFURL, nil)!
let img = CGImageSourceCreateImageAtIndex(src, 0, nil)!
let dst = CGImageDestinationCreateWithURL(URL(fileURLWithPath: a[2]) as CFURL, "public.tiff" as CFString, 1, nil)!
let tiff: [CFString: Any] = [kCGImagePropertyTIFFCompression: 5]
CGImageDestinationAddImage(dst, img, [kCGImagePropertyTIFFDictionary: tiff] as CFDictionary)
exit(CGImageDestinationFinalize(dst) ? 0 : 1)
"#,
    )
    .unwrap();
    let ok = Command::new("/usr/bin/swift")
        .arg(&script)
        .arg(&raw)
        .arg(&imageio)
        .status()
        .expect("/usr/bin/swift (Xcode command line tools)")
        .success();
    assert!(ok, "ImageIO LZW TIFF");
    match probe_image(&imageio) {
        // ImageIO may attach a profile or an alpha channel; then the file
        // is routed and there is nothing to compare.
        Ok(()) => {
            assert!(
                decoded_rgba8(&imageio) == reference,
                "ImageIO LZW decodes differently"
            );
            eprintln!("ImageIO LZW TIFF drawn, decodes like the uncompressed file");
        }
        Err(e) => eprintln!("ImageIO LZW TIFF routed: {e:?}"),
    }
    let gray = d.join("gray.tiff");
    ffmpeg(&gray, &["-pix_fmt", "gray"]);
    let got = unsupported(&gray);
    assert!(got.contains("photometric interpretation 1"), "{got}");
    eprintln!("gray.tiff    unsupported: {got}");
    std::fs::remove_dir_all(&d).ok();
}
