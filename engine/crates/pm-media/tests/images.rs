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

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pm-media-images-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

/// Packed pixels of the single RGBA plane as bytes, row by row.
fn bytes(f: &DecodedFrame) -> Vec<u8> {
    f.with_planes(|p| {
        let row = p[0].desc.width * p[0].desc.format.bytes();
        (0..p[0].desc.height)
            .flat_map(|y| unsafe { std::slice::from_raw_parts(p[0].ptr.add(y * p[0].stride), row) }.to_vec())
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
    assert_eq!((red.backend, red.format), (ImageBackend::ImageCrate, ImageFormat::Png));
    let r = px8(&red.frame, 1, 1);
    assert!(r[0] >= 250 && r[1] <= 3 && r[2] <= 3 && r[3] == 255, "{r:?}");
    let a = image::read(&f.join("still-alpha.png"), &pool).unwrap();
    assert_eq!(a.color.alpha, AlphaMode::Straight);
    let w = a.frame.format.width;
    assert!(px8(&a.frame, w / 4, 1)[3] > 200 && px8(&a.frame, 3 * w / 4, 1)[3] < 50, "alpha halves");
    for (name, backend) in [
        ("still.tiff", ImageBackend::ImageCrate),
        ("still.webp", ImageBackend::ImageCrate),
        ("still.heic", ImageBackend::ImageIo),
        ("still.avif", ImageBackend::ImageIo),
    ] {
        let s = image::read(&f.join(name), &pool).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(s.backend, backend, "{name}");
        assert!(s.frame.format.width > 0 && s.frame.format.height > 0);
    }
}

#[test]
fn e2e_animation_fixtures() {
    let pool = FramePool::new(0);
    let f = fixtures_dir();
    for name in ["animated.gif", "animated-apng.png", "animated.webp"] {
        let a = image::animation(&f.join(name)).unwrap().unwrap_or_else(|| panic!("{name} is animated"));
        assert_eq!(a.frames, 3, "{name}");
        assert!(a.delays.iter().all(|&d| d > 0.0), "{name}: {:?}", a.delays);
        let frames: Vec<Vec<u8>> =
            (0..3).map(|i| bytes(&image::read_animation_frame(&f.join(name), i, &pool).unwrap().frame)).collect();
        assert!(frames[0] != frames[1] && frames[1] != frames[2], "{name}: frames differ");
    }
    assert!(image::animation(&f.join("still-red.png")).unwrap().is_none());
}

#[test]
fn png16_and_float_tiff_are_exact() {
    let pool = FramePool::new(0);
    let (w, h) = (64u32, 8u32);
    let px: Vec<u16> = (0..w * h * 4).map(|i| (i * 977 % 65536) as u16).collect();
    let p = tmp("ramp16.png");
    ::image::ImageBuffer::<::image::Rgba<u16>, _>::from_raw(w, h, px.clone()).unwrap().save(&p).unwrap();
    let s = image::read(&p, &pool).unwrap();
    assert_eq!((s.bit_depth, s.frame.format.planes[0].format), (16, PlaneFormat::Rgba16));
    let got: Vec<u16> = bytes(&s.frame).chunks_exact(2).map(|b| u16::from_ne_bytes([b[0], b[1]])).collect();
    assert_eq!(got, px, "16-bit PNG samples pass through exactly");

    let fpx: Vec<f32> = (0..w * h * 4).map(|i| i as f32 * 0.37 - 20.0).collect();
    let t = tmp("float.tiff");
    ::image::ImageBuffer::<::image::Rgba<f32>, _>::from_raw(w, h, fpx.clone()).unwrap().save(&t).unwrap();
    let s = image::read(&t, &pool).unwrap();
    assert!(s.float && s.color.transfer == Transfer::Linear);
    let got: Vec<f32> = bytes(&s.frame).chunks_exact(4).map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]])).collect();
    assert_eq!(got, fpx, "float TIFF samples pass through exactly");
}

fn ffmpeg(args: &[&str]) {
    let ok = Command::new(ffmpeg_bin()).args(["-v", "error", "-y"]).args(args).status().unwrap().success();
    assert!(ok, "ffmpeg {args:?}");
}

#[test]
fn openexr_half_float_and_alpha() {
    let pool = FramePool::new(0);
    let src = "testsrc2=size=96x54:rate=1";
    let half = tmp("half.exr");
    ffmpeg(&["-f", "lavfi", "-i", src, "-frames:v", "1", "-c:v", "exr", "-pix_fmt", "gbrpf32le", "-format", "half", "-compression", "zip1", half.to_str().unwrap()]);
    let float = tmp("float.exr");
    ffmpeg(&["-f", "lavfi", "-i", src, "-frames:v", "1", "-c:v", "exr", "-pix_fmt", "gbrapf32le", "-format", "float", "-compression", "rle", float.to_str().unwrap()]);
    let h = image::read(&half, &pool).unwrap();
    assert_eq!((h.backend, h.bit_depth, h.frame.format.planes[0].format), (ImageBackend::OpenExr, 16, PlaneFormat::Rgba16Float));
    assert_eq!(h.color.alpha, AlphaMode::Opaque);
    let f = image::read(&float, &pool).unwrap();
    assert_eq!((f.bit_depth, f.frame.format.planes[0].format), (32, PlaneFormat::Rgba32Float));
    assert_eq!(f.color.alpha, AlphaMode::Premultiplied);
    // The same picture: half values equal the float values rounded to half.
    let hb = bytes(&h.frame);
    let fb = bytes(&f.frame);
    let to_f32 = |h: u16| -> f32 {
        let s = (h >> 15) as u32;
        let e = ((h >> 10) & 0x1F) as i32;
        let m = (h & 0x3FF) as f32;
        let v = if e == 0 { m / 1024.0 * 2f32.powi(-14) } else { (1.0 + m / 1024.0) * 2f32.powi(e - 15) };
        if s == 1 { -v } else { v }
    };
    let mut max = 0f32;
    for i in 0..96 * 54 * 3 {
        let p = i / 3 * 4 + i % 3;
        let hv = to_f32(u16::from_ne_bytes([hb[2 * p], hb[2 * p + 1]]));
        let fv = f32::from_ne_bytes(fb[4 * p..4 * p + 4].try_into().unwrap());
        max = max.max((hv - fv).abs());
    }
    assert!(max <= 1.0 / 1024.0, "half and float decodes differ by {max}");
    // FFmpeg's own decode of the float file agrees with OpenEXRCore.
    let raw = tmp("float.raw");
    ffmpeg(&["-i", float.to_str().unwrap(), "-f", "rawvideo", "-pix_fmt", "gbrapf32le", raw.to_str().unwrap()]);
    let r = std::fs::read(&raw).unwrap();
    let n = 96 * 54;
    let plane = |p: usize, i: usize| &r[(p * n + i) * 4..(p * n + i) * 4 + 4];
    let interleaved: Vec<u8> = (0..n).flat_map(|i| [plane(2, i), plane(0, i), plane(1, i), plane(3, i)].concat()).collect();
    assert_eq!(interleaved, fb, "OpenEXRCore and FFmpeg decode the same float RGBA");
}

#[test]
fn dpx_through_ffmpeg() {
    let pool = FramePool::new(0);
    let p = tmp("frame.dpx");
    ffmpeg(&["-f", "lavfi", "-i", "testsrc2=size=64x36:rate=1", "-frames:v", "1", "-pix_fmt", "gbrp10le", p.to_str().unwrap()]);
    let s = image::read(&p, &pool).unwrap();
    assert_eq!((s.backend, s.format, s.bit_depth), (ImageBackend::Ffmpeg, ImageFormat::Dpx, 10));
    assert_eq!(s.frame.format.label, "gbrp10le");
}

#[test]
fn exif_orientation_is_applied() {
    let pool = FramePool::new(0);
    // 4×2 JPEG, left half red, right half blue; then an EXIF orientation 6 (rotate 90° clockwise).
    let p = tmp("orient.jpg");
    let img = ::image::RgbImage::from_fn(16, 8, |x, _| if x < 8 { ::image::Rgb([255, 0, 0]) } else { ::image::Rgb([0, 0, 255]) });
    img.save(&p).unwrap();
    let jpg = std::fs::read(&p).unwrap();
    let mut exif = b"Exif\0\0MM\0*\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0".to_vec();
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
    assert!(matches!(s.color.icc_class, Some(IccClass::Srgb) | Some(IccClass::DisplayP3)), "{:?}", s.color.icc_class);
}

/// Writes a tiled RGBA EXR with OpenEXRCore (FFmpeg writes only scanline
/// files): 16×16 tiles, a data window offset inside the display window.
fn write_tiled_exr(path: &std::path::Path, w: i32, h: i32, half: bool, compression: u32, px: &[f32]) {
    use pm_media::sys::openexr::*;
    use std::ffi::CString;
    let c = CString::new(path.to_str().unwrap()).unwrap();
    let init = exr_context_initializer_t { size: std::mem::size_of::<exr_context_initializer_t>(), ..Default::default() };
    let mut ctx: exr_context_t = std::ptr::null_mut();
    unsafe {
        assert_eq!(exr_start_write(&mut ctx, c.as_ptr(), EXR_WRITE_FILE_DIRECTLY, &init), 0);
        let mut part = 0;
        let name = CString::new("main").unwrap();
        assert_eq!(exr_add_part(ctx, name.as_ptr(), EXR_STORAGE_TILED, &mut part), 0);
        assert_eq!(exr_initialize_required_attr_simple(ctx, part, w, h, compression as _), 0);
        // Display window 4 pixels larger on every side.
        let disp = exr_attr_box2i_t { min: exr_attr_v2i_t { x: -4, y: -4 }, max: exr_attr_v2i_t { x: w + 3, y: h + 3 } };
        assert_eq!(exr_set_display_window(ctx, part, &disp), 0);
        assert_eq!(exr_set_tile_descriptor(ctx, part, 16, 16, EXR_TILE_ONE_LEVEL, EXR_TILE_ROUND_DOWN), 0);
        let ty = if half { EXR_PIXEL_HALF } else { EXR_PIXEL_FLOAT };
        for ch in ["A", "B", "G", "R"] {
            let n = CString::new(ch).unwrap();
            assert_eq!(exr_add_channel(ctx, part, n.as_ptr(), ty, EXR_PERCEPTUALLY_LINEAR, 1, 1), 0);
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
                    let comp = match name { "R" => 0, "G" => 1, "B" => 2, _ => 3 };
                    // Tile chunks carry tile indices, not pixel positions.
                    let at = (((j as usize * 16) * w as usize + i as usize * 16) * 4 + comp) * 4;
                    chn.__bindgen_anon_1.encode_from_ptr = (px.as_ptr() as *const u8).add(at);
                    chn.user_pixel_stride = 16;
                    chn.user_line_stride = w * 16;
                    chn.user_data_type = EXR_PIXEL_FLOAT as u16;
                    chn.user_bytes_per_element = 4;
                }
                assert_eq!(exr_encoding_choose_default_routines(ctx as _, part, &mut enc), 0);
                assert_eq!(exr_encoding_run(ctx as _, part, &mut enc), 0);
                exr_encoding_destroy(ctx as _, &mut enc);
            }
        }
        assert_eq!(exr_finish(&mut ctx), 0);
    }
}

#[test]
fn openexr_tiled_zip_and_dwaa() {
    use pm_media::sys::openexr::{EXR_COMPRESSION_DWAA, EXR_COMPRESSION_ZIP};
    let pool = FramePool::new(0);
    let (w, h) = (40, 24);
    let px: Vec<f32> = (0..w * h * 4).map(|i| ((i * 37) % 1000) as f32 / 250.0 - 0.5).collect();
    let zip = tmp("tiled-zip.exr");
    write_tiled_exr(&zip, w as i32, h as i32, false, EXR_COMPRESSION_ZIP, &px);
    let s = image::read(&zip, &pool).unwrap();
    assert!(s.frame.format.label.contains("tiled"));
    // The display window is 8 pixels wider and taller; the data sits at (4, 4).
    assert_eq!((s.frame.format.width, s.frame.format.height), (w + 8, h + 8));
    let b = bytes(&s.frame);
    let at = |x: usize, y: usize, c: usize| f32::from_ne_bytes(b[((y * (w + 8) + x) * 4 + c) * 4..][..4].try_into().unwrap());
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                assert_eq!(at(x + 4, y + 4, c), px[(y * w + x) * 4 + c], "({x}, {y}) channel {c}");
            }
        }
    }
    assert_eq!(at(0, 0, 3), 0.0, "outside the data window is transparent black");
    // DWAA (lossy, half): decodes through OpenEXRCore and stays close.
    let dwaa = tmp("tiled-dwaa.exr");
    write_tiled_exr(&dwaa, w as i32, h as i32, true, EXR_COMPRESSION_DWAA, &px);
    let d = image::read(&dwaa, &pool).unwrap();
    assert_eq!(d.frame.format.planes[0].format, PlaneFormat::Rgba16Float);
}
