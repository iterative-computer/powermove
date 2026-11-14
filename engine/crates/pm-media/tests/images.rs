//! Image readers (section 13 "Images"): the e2e still and animation
//! fixtures, exact 16-bit PNG and float TIFF round trips, OpenEXR (half and
//! float, scanline and tiled, with and without alpha), DPX, EXIF
//! orientation, and ICC classification.

mod common;

use common::*;
use pm_media::frame::{DecodedFrame, PlaneFormat};
use pm_media::image::{self, ImageBackend, ImageFormat, icc::IccClass};
use pm_media::pool::FramePool;
use pm_protocol::color::{AlphaMode, Transfer};
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn tmp(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

/// Packed pixels of the single RGBA plane as bytes, row by row.
fn bytes(f: &DecodedFrame) -> Vec<u8> {
    f.with_planes(|p| {
        let row = p[0].desc.width * p[0].desc.format.bytes();
        (0..p[0].desc.height)
            .flat_map(|y| {
                unsafe { std::slice::from_raw_parts(p[0].ptr.add(y * p[0].stride), row) }.to_vec()
            })
            .collect()
    })
    .unwrap()
}

fn px8(f: &DecodedFrame, x: usize, y: usize) -> [u8; 4] {
    let b = bytes(f);
    let i = (y * f.format.width + x) * 4;
    [b[i], b[i + 1], b[i + 2], b[i + 3]]
}

#[test]
fn e2e_still_fixtures() {
    let pool = FramePool::new(0);
    let f = fixtures_dir();
    let red = image::read(&f.join("still-red.png"), &pool).unwrap();
    assert_eq!(
        (red.backend, red.format),
        (ImageBackend::ImageCrate, ImageFormat::Png)
    );
    let r = px8(&red.frame, 1, 1);
    assert!(
        r[0] >= 250 && r[1] <= 3 && r[2] <= 3 && r[3] == 255,
        "{r:?}"
    );
    let a = image::read(&f.join("still-alpha.png"), &pool).unwrap();
    assert_eq!(a.color.alpha, AlphaMode::Straight);
    let w = a.frame.format.width;
    assert!(
        px8(&a.frame, w / 4, 1)[3] > 200 && px8(&a.frame, 3 * w / 4, 1)[3] < 50,
        "alpha halves"
    );
    for (name, backend) in [
        ("still.tiff", ImageBackend::ImageCrate),
        ("still.webp", ImageBackend::ImageCrate),
        ("still.heic", ImageBackend::ImageIo),
        ("still.avif", ImageBackend::Dav1d),
    ] {
        let s = image::read(&f.join(name), &pool).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(s.backend, backend, "{name}");
        assert!(s.frame.format.width > 0 && s.frame.format.height > 0);
    }
}

/// Every AVIF fixture decodes through the native reader at its depth, as
/// Y′CbCr planes with the container's color description, including those
/// ImageIO rejects (deep alpha, no `pixi`).
#[test]
fn avif_fixtures_decode_natively_at_depth() {
    let pool = FramePool::new(0);
    let f = fixtures_dir();
    for (name, bits, alpha, planes) in [
        ("still.avif", 8, false, 3),
        ("still-no-pixi.avif", 8, false, 3),
        ("still-limited-range.avif", 8, false, 3),
        ("still-avif-400-full.avif", 8, false, 3),
        ("still-avif-444-full.avif", 8, false, 3),
        ("still-red-limited-range-422-12bpc.avif", 12, false, 3),
        ("still-red-with-alpha-12bpc.avif", 12, true, 4),
        ("still-red-with-profile-12bpc.avif", 12, false, 3),
        ("still-avif-icc-adobe-alpha.avif", 8, true, 4),
    ] {
        let s = image::read(&f.join(name), &pool).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(s.format, ImageFormat::Avif, "{name}");
        assert_eq!(s.bit_depth, bits, "{name}");
        assert_eq!(s.frame.format.planes.len(), planes, "{name}");
        assert_eq!(s.color.alpha != AlphaMode::Opaque, alpha, "{name}");
        assert!(s.color.cicp.is_some(), "{name}");
    }
    let icc = image::read(&f.join("still-avif-icc-adobe-alpha.avif"), &pool).unwrap();
    assert_eq!(icc.color.icc_class, Some(IccClass::Other));
}

#[test]
fn e2e_animation_fixtures() {
    let pool = FramePool::new(0);
    let f = fixtures_dir();
    for name in ["animated.gif", "animated-apng.png", "animated.webp"] {
        let a = image::animation(&f.join(name))
            .unwrap()
            .unwrap_or_else(|| panic!("{name} is animated"));
        assert_eq!(a.frames, 3, "{name}");
        assert!(a.delays.iter().all(|&d| d > 0.0), "{name}: {:?}", a.delays);
        let frames: Vec<Vec<u8>> = (0..3)
            .map(|i| {
                bytes(
                    &image::read_animation_frame(&f.join(name), i, &pool)
                        .unwrap()
                        .frame,
                )
            })
            .collect();
        assert!(
            frames[0] != frames[1] && frames[1] != frames[2],
            "{name}: frames differ"
        );
    }
    assert!(
        image::animation(&f.join("still-red.png"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn png16_and_float_tiff_are_exact() {
    let dir = tempfile::Builder::new()
        .prefix("pm-media-images-")
        .tempdir()
        .unwrap();
    let pool = FramePool::new(0);
    let (w, h) = (64u32, 8u32);
    let px: Vec<u16> = (0..w * h * 4).map(|i| (i * 977 % 65536) as u16).collect();
    let p = tmp(&dir, "ramp16.png");
    ::image::ImageBuffer::<::image::Rgba<u16>, _>::from_raw(w, h, px.clone())
        .unwrap()
        .save(&p)
        .unwrap();
    let s = image::read(&p, &pool).unwrap();
    assert_eq!(
        (s.bit_depth, s.frame.format.planes[0].format),
        (16, PlaneFormat::Rgba16)
    );
    let got: Vec<u16> = bytes(&s.frame)
        .chunks_exact(2)
        .map(|b| u16::from_ne_bytes([b[0], b[1]]))
        .collect();
    assert_eq!(got, px, "16-bit PNG samples pass through exactly");

    let fpx: Vec<f32> = (0..w * h * 4).map(|i| i as f32 * 0.37 - 20.0).collect();
    let t = tmp(&dir, "float.tiff");
    ::image::ImageBuffer::<::image::Rgba<f32>, _>::from_raw(w, h, fpx.clone())
        .unwrap()
        .save(&t)
        .unwrap();
    let s = image::read(&t, &pool).unwrap();
    assert!(s.float && s.color.transfer == Transfer::Linear);
    let got: Vec<f32> = bytes(&s.frame)
        .chunks_exact(4)
        .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    assert_eq!(got, fpx, "float TIFF samples pass through exactly");
}

fn ffmpeg(args: &[&str]) {
    let ok = Command::new(ffmpeg_bin())
        .args(["-v", "error", "-y"])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "ffmpeg {args:?}");
}

#[test]
fn openexr_half_float_and_alpha() {
    let dir = tempfile::Builder::new()
        .prefix("pm-media-images-")
        .tempdir()
        .unwrap();
    let pool = FramePool::new(0);
    let src = "testsrc2=size=96x54:rate=1";
    let half = tmp(&dir, "half.exr");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        src,
        "-frames:v",
        "1",
        "-c:v",
        "exr",
        "-pix_fmt",
        "gbrpf32le",
        "-format",
        "half",
        "-compression",
        "zip1",
        half.to_str().unwrap(),
    ]);
    let float = tmp(&dir, "float.exr");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        src,
        "-frames:v",
        "1",
        "-c:v",
        "exr",
        "-pix_fmt",
        "gbrapf32le",
        "-format",
        "float",
        "-compression",
        "rle",
        float.to_str().unwrap(),
    ]);
    let h = image::read(&half, &pool).unwrap();
    assert_eq!(
        (h.backend, h.bit_depth, h.frame.format.planes[0].format),
        (ImageBackend::OpenExr, 16, PlaneFormat::Rgba16Float)
    );
    assert_eq!(h.color.alpha, AlphaMode::Opaque);
    let f = image::read(&float, &pool).unwrap();
    assert_eq!(
        (f.bit_depth, f.frame.format.planes[0].format),
        (32, PlaneFormat::Rgba32Float)
    );
    assert_eq!(f.color.alpha, AlphaMode::Premultiplied);
    // The same picture: half values equal the float values rounded to half.
    let hb = bytes(&h.frame);
    let fb = bytes(&f.frame);
    let to_f32 = |h: u16| -> f32 {
        let s = (h >> 15) as u32;
        let e = ((h >> 10) & 0x1F) as i32;
        let m = (h & 0x3FF) as f32;
        let v = if e == 0 {
            m / 1024.0 * 2f32.powi(-14)
        } else {
            (1.0 + m / 1024.0) * 2f32.powi(e - 15)
        };
        if s == 1 { -v } else { v }
    };
    let mut max = 0f32;
    for i in 0..96 * 54 * 3 {
        let p = i / 3 * 4 + i % 3;
        let hv = to_f32(u16::from_ne_bytes([hb[2 * p], hb[2 * p + 1]]));
        let fv = f32::from_ne_bytes(fb[4 * p..4 * p + 4].try_into().unwrap());
        max = max.max((hv - fv).abs());
    }
    assert!(
        max <= 1.0 / 1024.0,
        "half and float decodes differ by {max}"
    );
    // FFmpeg's own decode of the float file agrees with OpenEXRCore.
    let raw = tmp(&dir, "float.raw");
    ffmpeg(&[
        "-i",
        float.to_str().unwrap(),
        "-f",
        "rawvideo",
        "-pix_fmt",
        "gbrapf32le",
        raw.to_str().unwrap(),
    ]);
    let r = std::fs::read(&raw).unwrap();
    let n = 96 * 54;
    let plane = |p: usize, i: usize| &r[(p * n + i) * 4..(p * n + i) * 4 + 4];
    let interleaved: Vec<u8> = (0..n)
        .flat_map(|i| [plane(2, i), plane(0, i), plane(1, i), plane(3, i)].concat())
        .collect();
    assert_eq!(
        interleaved, fb,
        "OpenEXRCore and FFmpeg decode the same float RGBA"
    );
}

#[test]
fn dpx_through_ffmpeg() {
    let dir = tempfile::Builder::new()
        .prefix("pm-media-images-")
        .tempdir()
        .unwrap();
    let pool = FramePool::new(0);
    let p = tmp(&dir, "frame.dpx");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=64x36:rate=1",
        "-frames:v",
        "1",
        "-pix_fmt",
        "gbrp10le",
        p.to_str().unwrap(),
    ]);
    let s = image::read(&p, &pool).unwrap();
    assert_eq!(
        (s.backend, s.format, s.bit_depth),
        (ImageBackend::Ffmpeg, ImageFormat::Dpx, 10)
    );
    assert_eq!(s.frame.format.label, "gbrp10le");
}

#[test]
fn exif_orientation_is_applied() {
    let dir = tempfile::Builder::new()
        .prefix("pm-media-images-")
        .tempdir()
        .unwrap();
    let pool = FramePool::new(0);
    // 4×2 JPEG, left half red, right half blue; then an EXIF orientation 6 (rotate 90° clockwise).
    let p = tmp(&dir, "orient.jpg");
    let img = ::image::RgbImage::from_fn(16, 8, |x, _| {
        if x < 8 {
            ::image::Rgb([255, 0, 0])
        } else {
            ::image::Rgb([0, 0, 255])
        }
    });
    img.save(&p).unwrap();
    let jpg = std::fs::read(&p).unwrap();
    let mut exif =
        b"Exif\0\0MM\0*\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0".to_vec();
    let mut app1 = vec![0xFF, 0xE1];
    app1.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    app1.append(&mut exif);
    let mut out = jpg[..2].to_vec();
    out.extend(app1);
    out.extend_from_slice(&jpg[2..]);
    std::fs::write(&p, &out).unwrap();
    let s = image::read(&p, &pool).unwrap();
    assert_eq!(s.orientation, 6);
    assert_eq!((s.frame.format.width, s.frame.format.height), (8, 16));
    // After rotating clockwise, red (the left half) is on top.
    let top = px8(&s.frame, 4, 2);
    let bottom = px8(&s.frame, 4, 13);
    assert!(top[0] > 200 && top[2] < 60, "{top:?}");
    assert!(bottom[2] > 200 && bottom[0] < 60, "{bottom:?}");
}

#[test]
fn heic_profiles_are_classified() {
    let pool = FramePool::new(0);
    let s = image::read(&fixtures_dir().join("still.heic"), &pool).unwrap();
    // `sips` writes an sRGB or Display P3 profile; either way it is recognized.
    assert!(
        matches!(
            s.color.icc_class,
            Some(IccClass::Srgb) | Some(IccClass::DisplayP3)
        ),
        "{:?}",
        s.color.icc_class
    );
}

/// Writes a tiled RGBA EXR with OpenEXRCore (FFmpeg writes only scanline
/// files): 16×16 tiles, a data window offset inside the display window.
fn write_tiled_exr(
    path: &std::path::Path,
    w: i32,
    h: i32,
    half: bool,
    compression: u32,
    px: &[f32],
) {
    use pm_media::sys::openexr::*;
    use std::ffi::CString;
    let c = CString::new(path.to_str().unwrap()).unwrap();
    let init = exr_context_initializer_t {
        size: std::mem::size_of::<exr_context_initializer_t>(),
        ..Default::default()
    };
    let mut ctx: exr_context_t = std::ptr::null_mut();
    unsafe {
        assert_eq!(
            exr_start_write(&mut ctx, c.as_ptr(), EXR_WRITE_FILE_DIRECTLY, &init),
            0
        );
        let mut part = 0;
        let name = CString::new("main").unwrap();
        assert_eq!(
            exr_add_part(ctx, name.as_ptr(), EXR_STORAGE_TILED, &mut part),
            0
        );
        assert_eq!(
            exr_initialize_required_attr_simple(ctx, part, w, h, compression as _),
            0
        );
        // Display window 4 pixels larger on every side.
        let disp = exr_attr_box2i_t {
            min: exr_attr_v2i_t { x: -4, y: -4 },
            max: exr_attr_v2i_t { x: w + 3, y: h + 3 },
        };
        assert_eq!(exr_set_display_window(ctx, part, &disp), 0);
        assert_eq!(
            exr_set_tile_descriptor(ctx, part, 16, 16, EXR_TILE_ONE_LEVEL, EXR_TILE_ROUND_DOWN),
            0
        );
        let ty = if half {
            EXR_PIXEL_HALF
        } else {
            EXR_PIXEL_FLOAT
        };
        for ch in ["A", "B", "G", "R"] {
            let n = CString::new(ch).unwrap();
            assert_eq!(
                exr_add_channel(ctx, part, n.as_ptr(), ty, EXR_PERCEPTUALLY_LINEAR, 1, 1),
                0
            );
        }
        assert_eq!(exr_write_header(ctx), 0);
        let (tx, ty_count) = ((w + 15) / 16, (h + 15) / 16);
        for j in 0..ty_count {
            for i in 0..tx {
                let mut ci = exr_chunk_info_t::default();
                assert_eq!(exr_write_tile_chunk_info(ctx, part, i, j, 0, 0, &mut ci), 0);
                let mut enc: exr_encode_pipeline_t = std::mem::zeroed();
                assert_eq!(exr_encoding_initialize(ctx as _, part, &ci, &mut enc), 0);
                for k in 0..enc.channel_count as usize {
                    let chn = &mut *enc.channels.add(k);
                    let name = std::ffi::CStr::from_ptr(chn.channel_name).to_str().unwrap();
                    let comp = match name {
                        "R" => 0,
                        "G" => 1,
                        "B" => 2,
                        _ => 3,
                    };
                    // Tile chunks carry tile indices, not pixel positions.
                    let at = (((j as usize * 16) * w as usize + i as usize * 16) * 4 + comp) * 4;
                    chn.__bindgen_anon_1.encode_from_ptr = (px.as_ptr() as *const u8).add(at);
                    chn.user_pixel_stride = 16;
                    chn.user_line_stride = w * 16;
                    chn.user_data_type = EXR_PIXEL_FLOAT as u16;
                    chn.user_bytes_per_element = 4;
                }
                assert_eq!(
                    exr_encoding_choose_default_routines(ctx as _, part, &mut enc),
                    0
                );
                assert_eq!(exr_encoding_run(ctx as _, part, &mut enc), 0);
                exr_encoding_destroy(ctx as _, &mut enc);
            }
        }
        assert_eq!(exr_finish(&mut ctx), 0);
    }
}

#[test]
fn openexr_tiled_zip_and_dwaa() {
    let dir = tempfile::Builder::new()
        .prefix("pm-media-images-")
        .tempdir()
        .unwrap();
    use pm_media::sys::openexr::{EXR_COMPRESSION_DWAA, EXR_COMPRESSION_ZIP};
    let pool = FramePool::new(0);
    let (w, h) = (40, 24);
    let px: Vec<f32> = (0..w * h * 4)
        .map(|i| ((i * 37) % 1000) as f32 / 250.0 - 0.5)
        .collect();
    let zip = tmp(&dir, "tiled-zip.exr");
    write_tiled_exr(&zip, w as i32, h as i32, false, EXR_COMPRESSION_ZIP, &px);
    let s = image::read(&zip, &pool).unwrap();
    assert!(s.frame.format.label.contains("tiled"));
    // The display window is 8 pixels wider and taller; the data sits at (4, 4).
    assert_eq!(
        (s.frame.format.width, s.frame.format.height),
        (w + 8, h + 8)
    );
    let b = bytes(&s.frame);
    let at = |x: usize, y: usize, c: usize| {
        f32::from_ne_bytes(
            b[((y * (w + 8) + x) * 4 + c) * 4..][..4]
                .try_into()
                .unwrap(),
        )
    };
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                assert_eq!(
                    at(x + 4, y + 4, c),
                    px[(y * w + x) * 4 + c],
                    "({x}, {y}) channel {c}"
                );
            }
        }
    }
    assert_eq!(
        at(0, 0, 3),
        0.0,
        "outside the data window is transparent black"
    );
    // DWAA (lossy, half): decodes through OpenEXRCore and stays close.
    let dwaa = tmp(&dir, "tiled-dwaa.exr");
    write_tiled_exr(&dwaa, w as i32, h as i32, true, EXR_COMPRESSION_DWAA, &px);
    let d = image::read(&dwaa, &pool).unwrap();
    assert_eq!(d.frame.format.planes[0].format, PlaneFormat::Rgba16Float);
}

/// A 4×2 RGBA test picture: every sample distinct, alpha varied.
fn picture(x: usize, y: usize) -> [u8; 4] {
    let i = (y * 4 + x) as u8;
    [10 + i * 29, 200 - i * 23, 3 + i * 31, 255 - i * 17]
}

/// A Truevision TGA of [`picture`], 32-bit BGRA: uncompressed with the
/// default bottom-left origin, or run-length encoded with a top-left origin.
fn tga(rle: bool) -> Vec<u8> {
    let (w, h) = (4usize, 2usize);
    let mut out = vec![0u8; 18];
    out[2] = if rle { 10 } else { 2 };
    out[12..14].copy_from_slice(&(w as u16).to_le_bytes());
    out[14..16].copy_from_slice(&(h as u16).to_le_bytes());
    out[16] = 32;
    out[17] = 8 | if rle { 0x20 } else { 0 };
    let rows: Vec<usize> = if rle {
        (0..h).collect()
    } else {
        (0..h).rev().collect()
    };
    for y in rows {
        if rle {
            // A raw packet of the first two pixels, then each of the rest
            // as a repeat packet of one.
            out.push(1);
            for x in 0..2 {
                let [r, g, b, a] = picture(x, y);
                out.extend_from_slice(&[b, g, r, a]);
            }
            for x in 2..w {
                let [r, g, b, a] = picture(x, y);
                out.push(0x80);
                out.extend_from_slice(&[b, g, r, a]);
            }
        } else {
            for x in 0..w {
                let [r, g, b, a] = picture(x, y);
                out.extend_from_slice(&[b, g, r, a]);
            }
        }
    }
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(b"TRUEVISION-XFILE.\0");
    out
}

/// A flattened Photoshop document of [`picture`]'s RGB at 8 or 16 bits
/// (16-bit samples carry the 8-bit code in the high byte and a distinct low
/// byte), uncompressed.
fn psd(bits: u16) -> Vec<u8> {
    let (w, h) = (4usize, 2usize);
    let mut out = b"8BPS".to_vec();
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&[0; 6]);
    out.extend_from_slice(&3u16.to_be_bytes());
    out.extend_from_slice(&(h as u32).to_be_bytes());
    out.extend_from_slice(&(w as u32).to_be_bytes());
    out.extend_from_slice(&bits.to_be_bytes());
    out.extend_from_slice(&3u16.to_be_bytes()); // RGB
    out.extend_from_slice(&[0; 12]); // no color mode data, resources or layers
    out.extend_from_slice(&0u16.to_be_bytes()); // raw image data
    for c in 0..3 {
        for y in 0..h {
            for x in 0..w {
                let v = picture(x, y)[c];
                if bits == 16 {
                    out.extend_from_slice(&psd16(v, x, y).to_be_bytes());
                } else {
                    out.push(v);
                }
            }
        }
    }
    out
}

fn psd16(v: u8, x: usize, y: usize) -> u16 {
    u16::from(v) << 8 | (37 + 53 * (y * 4 + x)) as u16
}

#[test]
fn tga_and_psd_are_recognized_and_read_exactly() {
    let dir = tempfile::Builder::new()
        .prefix("pm-media-images-")
        .tempdir()
        .unwrap();
    let pool = FramePool::new(0);
    for (name, bytes) in [("flat.tga", tga(false)), ("rle.tga", tga(true))] {
        assert_eq!(image::sniff(&bytes), ImageFormat::Tga, "{name}");
        let p = tmp(&dir, name);
        std::fs::write(&p, &bytes).unwrap();
        let s = image::read(&p, &pool).unwrap();
        assert_eq!(
            (s.format, s.bit_depth, s.float),
            (ImageFormat::Tga, 8, false)
        );
        assert_eq!(s.color.alpha, AlphaMode::Straight, "{name}");
        for y in 0..2 {
            for x in 0..4 {
                assert_eq!(px8(&s.frame, x, y), picture(x, y), "{name} ({x}, {y})");
            }
        }
    }
    for bits in [8u16, 16] {
        let name = format!("rgb{bits}.psd");
        let bytes = psd(bits);
        assert_eq!(image::sniff(&bytes), ImageFormat::Psd, "{name}");
        let p = tmp(&dir, &name);
        std::fs::write(&p, &bytes).unwrap();
        let s = image::read(&p, &pool).unwrap();
        assert_eq!(
            (s.format, s.bit_depth, s.float),
            (ImageFormat::Psd, bits as u8, false),
            "{name}"
        );
        let samples = bytes_of(&s.frame, bits);
        for y in 0..2 {
            for x in 0..4 {
                let i = (y * 4 + x) * 4;
                let want: Vec<u16> = (0..3)
                    .map(|c| {
                        let v = picture(x, y)[c];
                        if bits == 16 {
                            psd16(v, x, y)
                        } else {
                            u16::from(v)
                        }
                    })
                    .collect();
                assert_eq!(&samples[i..i + 3], &want[..], "{name} ({x}, {y})");
            }
        }
    }
    // Other files with small first bytes are not taken for TGA: MPEG-TS,
    // a QuickTime `mdat` atom, an MP3 frame, a truncated header.
    for head in [
        &b"\x47\x40\x00\x10\x00\x00\xb0\x0d\x00\x01\xc1\x00\x00\x00\x01\xf0\x00\x2a"[..],
        &b"\x00\x01\x0a\x00mdat\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00"[..],
        &b"\xff\xfb\x90\x64\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00"[..],
        &tga(false)[..17],
    ] {
        assert_eq!(image::sniff(head), ImageFormat::Other, "{head:?}");
    }
}

/// The single RGBA plane's samples, widened to u16.
fn bytes_of(f: &DecodedFrame, bits: u16) -> Vec<u16> {
    let b = bytes(f);
    if bits == 16 {
        b.chunks_exact(2)
            .map(|s| u16::from_ne_bytes([s[0], s[1]]))
            .collect()
    } else {
        b.into_iter().map(u16::from).collect()
    }
}

/// The 16-bit RGBA sample at (x, row 18) of a still's single plane.
fn px16(f: &DecodedFrame, x: usize) -> [u16; 4] {
    let b = bytes(f);
    let i = (18 * f.format.width + x) * 8;
    let at = |k: usize| u16::from_ne_bytes([b[i + 2 * k], b[i + 2 * k + 1]]);
    [at(0), at(1), at(2), at(3)]
}

/// HDR HEIC stills (`engine/fixtures/hdr-stills`, a picture whose left half
/// is 18 % grey and whose right half is three times SDR white) are read
/// with their BT.2100 transfer: the PQ or HLG codes as stored, at 16 bits,
/// with their H.273 description, not flattened to SDR light.
#[test]
fn hdr_heic_stills_are_read_with_their_transfer() {
    let pool = FramePool::new(0);
    let dir = fixtures_dir().join("hdr-stills");
    // PQ codes of 36.8 and 609 cd/m² (0.18 and 3.0 times 203 cd/m²).
    for (name, cicp, codes) in [
        ("still-hdr-pq.heic", [9, 16, 0, 1], [26906u16, 45740]),
        ("still-hdr-hlg.heic", [9, 18, 0, 1], [28571, 60538]),
    ] {
        let s = image::read(&dir.join(name), &pool).unwrap();
        assert_eq!(
            (
                s.bit_depth,
                s.float,
                s.color.cicp,
                s.color.icc_class,
                s.color.alpha
            ),
            (16, false, Some(cicp), None, AlphaMode::Opaque),
            "{name}"
        );
        for (x, code) in [(8, codes[0]), (56, codes[1])] {
            let got = px16(&s.frame, x);
            assert!(
                got[..3].iter().all(|v| v.abs_diff(code) <= 96) && got[3] == u16::MAX,
                "{name} at {x}: {got:?}, {code} expected"
            );
        }
        assert_eq!(s.color.gain_map, false, "{name}");
    }
}

/// Stills with a gain map (ISO 21496-1, written by Core Image into HEIC,
/// JPEG and AVIF) are read as HDR: ImageIO applies the map at the headroom
/// it describes and gives BT.709 PQ codes, SDR white at 203 cd/m², the
/// content peak the headroom. Read for their SDR base only, they are the
/// base image their own decoder gives.
#[test]
fn gain_map_stills_read_as_hdr_or_as_their_sdr_base() {
    let pool = FramePool::new(0);
    let dir = fixtures_dir().join("hdr-stills");
    for (name, format, base_backend) in [
        (
            "still-gainmap.heic",
            ImageFormat::Heif,
            ImageBackend::ImageIo,
        ),
        (
            "still-gainmap.jpg",
            ImageFormat::Jpeg,
            ImageBackend::LibjpegTurbo,
        ),
        ("still-gainmap.avif", ImageFormat::Avif, ImageBackend::Dav1d),
    ] {
        let path = dir.join(name);
        let hdr = image::read(&path, &pool).unwrap();
        assert_eq!(
            (hdr.format, hdr.backend, hdr.bit_depth, hdr.color.cicp),
            (format, ImageBackend::ImageIo, 16, Some([1, 16, 0, 1])),
            "{name}"
        );
        assert!(hdr.color.gain_map, "{name}");
        let peak = hdr.color.mastering_peak.unwrap_or_default();
        assert!((peak - 609.0).abs() < 3.0, "{name}: peak {peak}");
        for (x, code) in [(8, 26970u16), (56, 45740)] {
            let got = px16(&hdr.frame, x);
            assert!(
                got[..3].iter().all(|v| v.abs_diff(code) <= 96),
                "{name} at {x}: {got:?}, {code} expected"
            );
        }
        let base = image::read_with(&path, &pool, image::Rendition::SdrBase).unwrap();
        assert_eq!(
            (base.format, base.backend, base.bit_depth),
            (format, base_backend, 8),
            "{name}"
        );
        // The base's own description (AVIF's unspecified nclx), never PQ.
        assert!(base.color.cicp.is_none_or(|c| c[1] != 16), "{name}");
        assert!(base.color.gain_map, "{name}: the base knows its map");
        // 0.18 and 0.8 linear, sRGB encoded (AVIF's Y′CbCr base is drawn
        // in pm-render's tests).
        let w = base.frame.format.width;
        let packed = base.frame.format.layout == pm_media::frame::Layout::Rgba;
        for (x, code) in [(8, 118u8), (w - 8, 231)].into_iter().filter(|_| packed) {
            let got = px8(&base.frame, x, 18);
            assert!(
                got[..3].iter().all(|v| v.abs_diff(code) <= 2),
                "{name} base at {x}: {got:?}, {code} expected"
            );
        }
    }
    // A JPEG or HEIC without a map reads as before.
    let plain = image::read(&fixtures_dir().join("still.heic"), &pool).unwrap();
    assert!(!plain.color.gain_map && plain.color.cicp.is_none());
}
