//! Representation isolation: preview may use fewer pixels, while original
//! and export reads remain byte-exact after the reduced path is exercised.

mod common;

use pm_media::decode::scrub::{ViewerDecode, Want};
use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
use pm_media::image::{self, mip_cache::TiledMipCache};
use pm_media::pool::FramePool;
use pm_media::vt::DecodePurpose;
use std::sync::Arc;

#[test]
fn prores_partial_decode_is_isolated_from_export() {
    let original = common::fixtures_dir().join("prores422.mov");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clip.mov");
    std::fs::copy(&original, &path).unwrap();
    let source = MediaSource::open(&path).unwrap();
    let svc = Arc::new(DecodeService::new(ServiceConfig::default()));
    let before = svc
        .session(&source, 8, DecodePurpose::Export)
        .unwrap()
        .frame(0)
        .unwrap();
    let digest = before.md5_hex().unwrap();
    let reduced = DecodePurpose::DisplayReduced {
        width: (source.info.width / 2) as u32,
        height: (source.info.height / 2) as u32,
    };
    let viewer = ViewerDecode::new(svc.clone());
    let small = viewer
        .fetch_with_purpose(&source, 8, 0, Want::Exact, reduced)
        .unwrap()
        .frame;
    assert!(small.format.width < before.format.width);
    assert!(small.format.height < before.format.height);
    assert!(svc.cache().samples_of(source.id, reduced).len() > 0);
    assert!(
        svc.cache()
            .samples_of(source.id, DecodePurpose::Export)
            .len()
            > 0
    );
    let after = svc
        .session(&source, 8, DecodePurpose::Export)
        .unwrap()
        .frame(0)
        .unwrap();
    assert_eq!(after.md5_hex().unwrap(), digest);
    assert_eq!(
        (after.format.width, after.format.height),
        (before.format.width, before.format.height)
    );
}

#[test]
fn reduced_request_never_falls_back_to_full_software_decode() {
    let source = MediaSource::open(&common::fixtures_dir().join("h264-aac.mp4")).unwrap();
    let svc = DecodeService::new(ServiceConfig::default());
    let result = svc.session(
        &source,
        1,
        DecodePurpose::DisplayReduced {
            width: (source.info.width / 2) as u32,
            height: (source.info.height / 2) as u32,
        },
    );
    assert!(result.is_err());
    assert_eq!(svc.cache().stats().frames, 0);
}

#[test]
fn imageio_thumbnail_and_tiled_cache_leave_original_exact() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pattern.png");
    let mut data = ::image::RgbaImage::new(1024, 768);
    for (x, y, px) in data.enumerate_pixels_mut() {
        *px = ::image::Rgba([x as u8, y as u8, (x ^ y) as u8, 255]);
    }
    data.save(&path).unwrap();
    let file_before = std::fs::read(&path).unwrap();
    let pool = FramePool::new(0);
    let original = image::read(&path, &pool).unwrap();
    let digest = original.frame.md5_hex().unwrap();
    let small = image::read_reduced(&path, &pool, 512).unwrap();
    assert!(small.frame.format.width <= 512);
    assert!(small.frame.format.height <= 512);
    assert!(small.frame.format.width < original.frame.format.width);
    let cache = TiledMipCache::new(2 << 20);
    let tile = cache.tile(&path, &pool, 512, 1, 0).unwrap();
    assert_eq!(tile.frame.format.width, small.frame.format.width - 256);
    assert!(tile.frame.format.height <= 256);
    let bpp = tile.frame.format.planes[0].format.bytes();
    let from_small = small
        .frame
        .with_planes(|p| unsafe {
            std::slice::from_raw_parts(p[0].ptr.add(256 * bpp), bpp).to_vec()
        })
        .unwrap();
    let from_tile = tile
        .frame
        .with_planes(|p| unsafe { std::slice::from_raw_parts(p[0].ptr, bpp).to_vec() })
        .unwrap();
    assert_eq!(from_tile, from_small);
    assert!(cache.bytes() <= 2 << 20);
    let after = image::read(&path, &pool).unwrap();
    assert_eq!(after.frame.md5_hex().unwrap(), digest);
    assert_eq!(std::fs::read(&path).unwrap(), file_before);
}

#[test]
fn openexr_reads_stored_mip_without_touching_level_zero() {
    use pm_media::sys::openexr::*;
    use std::ffi::{CStr, CString};
    struct Writer(exr_context_t);
    impl Drop for Writer {
        fn drop(&mut self) {
            unsafe {
                exr_finish(&mut self.0);
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mipped.exr");
    let cpath = CString::new(path.to_str().unwrap()).unwrap();
    let init = exr_context_initializer_t {
        size: std::mem::size_of::<exr_context_initializer_t>(),
        ..Default::default()
    };
    let mut raw: exr_context_t = std::ptr::null_mut();
    unsafe {
        assert_eq!(
            exr_start_write(&mut raw, cpath.as_ptr(), EXR_WRITE_FILE_DIRECTLY, &init),
            0
        );
        let writer = Writer(raw);
        let mut part = 0;
        let name = CString::new("main").unwrap();
        assert_eq!(
            exr_add_part(writer.0, name.as_ptr(), EXR_STORAGE_TILED, &mut part),
            0
        );
        assert_eq!(
            exr_initialize_required_attr_simple(writer.0, part, 32, 32, EXR_COMPRESSION_ZIP),
            0
        );
        assert_eq!(
            exr_set_tile_descriptor(
                writer.0,
                part,
                16,
                16,
                EXR_TILE_MIPMAP_LEVELS,
                EXR_TILE_ROUND_DOWN
            ),
            0
        );
        for ch in ["A", "B", "G", "R"] {
            let channel = CString::new(ch).unwrap();
            assert_eq!(
                exr_add_channel(
                    writer.0,
                    part,
                    channel.as_ptr(),
                    EXR_PIXEL_FLOAT,
                    EXR_PERCEPTUALLY_LINEAR,
                    1,
                    1
                ),
                0
            );
        }
        assert_eq!(exr_write_header(writer.0), 0);
        for level in 0..=5 {
            let side = 32usize >> level;
            let pixels = vec![level as f32; side * side * 4];
            let tiles = side.div_ceil(16);
            for j in 0..tiles {
                for i in 0..tiles {
                    let mut ci = exr_chunk_info_t::default();
                    assert_eq!(
                        exr_write_tile_chunk_info(
                            writer.0, part, i as i32, j as i32, level, level, &mut ci
                        ),
                        0
                    );
                    let mut enc: exr_encode_pipeline_t = std::mem::zeroed();
                    assert_eq!(
                        exr_encoding_initialize(writer.0 as _, part, &ci, &mut enc),
                        0
                    );
                    for k in 0..enc.channel_count as usize {
                        let ch = &mut *enc.channels.add(k);
                        let channel = CStr::from_ptr(ch.channel_name).to_str().unwrap();
                        let component = match channel {
                            "R" => 0,
                            "G" => 1,
                            "B" => 2,
                            _ => 3,
                        };
                        let at = ((j * 16 * side + i * 16) * 4 + component) * 4;
                        ch.__bindgen_anon_1.encode_from_ptr =
                            (pixels.as_ptr() as *const u8).add(at);
                        ch.user_pixel_stride = 16;
                        ch.user_line_stride = (side * 16) as i32;
                        ch.user_data_type = EXR_PIXEL_FLOAT as u16;
                        ch.user_bytes_per_element = 4;
                    }
                    assert_eq!(
                        exr_encoding_choose_default_routines(writer.0 as _, part, &mut enc),
                        0
                    );
                    assert_eq!(exr_encoding_run(writer.0 as _, part, &mut enc), 0);
                    exr_encoding_destroy(writer.0 as _, &mut enc);
                }
            }
        }
        drop(writer);
    }
    let pool = FramePool::new(0);
    let full = image::read(&path, &pool).unwrap();
    let exact = full.frame.md5_hex().unwrap();
    let half = image::read_reduced(&path, &pool, 16).unwrap();
    assert_eq!(
        (half.frame.format.width, half.frame.format.height),
        (16, 16)
    );
    let value = half
        .frame
        .with_planes(|p| unsafe {
            f32::from_ne_bytes(std::slice::from_raw_parts(p[0].ptr, 4).try_into().unwrap())
        })
        .unwrap();
    assert_eq!(value, 1.0);
    assert_eq!(
        image::read(&path, &pool).unwrap().frame.md5_hex().unwrap(),
        exact
    );
}

#[test]
fn scanline_exr_builds_memory_mips_without_changing_original() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scanline.exr");
    let status = std::process::Command::new(common::ffmpeg_bin())
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x32:rate=1",
            "-frames:v",
            "1",
            "-c:v",
            "exr",
            "-pix_fmt",
            "gbrpf32le",
            "-format",
            "float",
            path.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let bytes = std::fs::read(&path).unwrap();
    let pool = FramePool::new(0);
    let full = image::read(&path, &pool).unwrap();
    let exact = full.frame.md5_hex().unwrap();
    let small = image::read_reduced(&path, &pool, 16).unwrap();
    assert_eq!(
        (small.frame.format.width, small.frame.format.height),
        (16, 8)
    );
    assert!(small.float);
    assert_eq!(
        small.frame.format.planes[0].format,
        pm_media::frame::PlaneFormat::Rgba32Float
    );
    assert_eq!(
        image::read(&path, &pool).unwrap().frame.md5_hex().unwrap(),
        exact
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}
