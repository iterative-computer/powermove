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

/// FFmpeg's JPEG layouts are among the tested ones (untested layouts, frame
/// types, and component counts are covered by `stills::tests::jpeg_classes`).
#[test]
fn tested_jpeg_samplings_are_supported() {
    let d = dir().join("jpeg");
    std::fs::create_dir_all(&d).unwrap();
    for fmt in ["yuvj420p", "yuvj422p", "yuvj444p", "gray"] {
        let path = d.join(format!("{fmt}.jpg"));
        ffmpeg(&path, &["-pix_fmt", fmt, "-q:v", "3"]);
        assert_eq!(probe_image(&path), Ok(()), "{fmt}");
    }
    std::fs::remove_dir_all(&d).ok();
}
