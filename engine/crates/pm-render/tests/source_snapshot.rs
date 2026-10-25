//! Source frames through the same decoder and source pass as layers.
#![cfg(target_os = "macos")]

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::{Size, Ticks};
use pm_protocol::snapshot::SourceFit;
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::json;
use support::doc::*;

/// Add a centered clean aperture to our generated, non-faststart H.264
/// fixture. Its moov follows mdat, so growing the sample entry moves no media.
fn add_test_clean_aperture(path: &std::path::Path, width: u32, height: u32) {
    fn find(data: &[u8], start: usize, end: usize, names: &[&[u8; 4]]) -> Vec<usize> {
        let mut at = start;
        while at + 8 <= end {
            let size = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
            assert!(size >= 8 && at + size <= end);
            if &data[at + 4..at + 8] == names[0].as_slice() {
                let mut offsets = vec![at];
                if names.len() > 1 {
                    let skip = if names[0] == b"stsd" { 16 } else { 8 };
                    let child = find(data, at + skip, at + size, &names[1..]);
                    if child.is_empty() {
                        at += size;
                        continue;
                    }
                    offsets.extend(child);
                }
                return offsets;
            }
            at += size;
        }
        vec![]
    }
    let mut bytes = std::fs::read(path).unwrap();
    let boxes = find(
        &bytes,
        0,
        bytes.len(),
        &[
            b"moov", b"trak", b"mdia", b"minf", b"stbl", b"stsd", b"avc1",
        ],
    );
    assert_eq!(boxes.len(), 7);
    let mdat = find(&bytes, 0, bytes.len(), &[b"mdat"]);
    assert!(mdat[0] < boxes[0]);
    let entry = *boxes.last().unwrap();
    let end = entry + u32::from_be_bytes(bytes[entry..entry + 4].try_into().unwrap()) as usize;
    let mut clap = Vec::from(40u32.to_be_bytes());
    clap.extend(b"clap");
    for value in [width, 1, height, 1, 0, 1, 0, 1] {
        clap.extend(value.to_be_bytes());
    }
    bytes.splice(end..end, clap);
    for at in boxes {
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
        bytes[at..at + 4].copy_from_slice(&(size + 40).to_be_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn compositor_source_returns_coded_nv12_with_the_shared_sd_reading() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../apps/desktop/e2e/fixtures/h264-aac.mp4");
    let mut renderer = Renderer::headless().unwrap();
    let (size, bytes, tags) = renderer
        .source_video_planes(&fixture, Ticks::ZERO, false)
        .unwrap()
        .expect("coded planes");
    assert_eq!((size.width, size.height), (160, 90));
    assert_eq!(tags.format, "nv12");
    assert_eq!(
        Some(bytes.len() as u64),
        tags.byte_len(size.width, size.height)
    );
    // pm_media::probe::color::video_reading for untagged 160x90 video.
    assert_eq!(tags.range, "limited");
    assert_eq!(tags.primaries, "smpte170m");
    assert_eq!(tags.matrix, "smpte170m");
    assert_eq!(tags.transfer, "bt709");
    // A stream larger than the planes budget is resampled by the engine
    // instead; it is refused before the sink sees any bytes.
    let oversized = renderer
        .source_video_planes_with::<()>(&fixture, Ticks::ZERO, false, 160 * 90 - 1, |_, _| {
            panic!("an oversized stream must not be packed")
        })
        .unwrap();
    assert!(oversized.is_none());
    // A still cannot silently become a P3/Canvas RGBA compositor payload.
    let dir = tempfile::tempdir().unwrap();
    let still = dir.path().join("still.png");
    std::fs::write(
        &still,
        include_bytes!("../../../../apps/desktop/e2e/fixtures/still-red.png"),
    )
    .unwrap();
    assert!(
        renderer
            .source_video_planes(&still, Ticks::ZERO, false)
            .is_err()
    );
}

#[test]
fn original_prores_uses_native_conversion_when_the_presenter_cannot_import_it() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../apps/desktop/e2e/fixtures/prores422.mov");
    let mut renderer = Renderer::headless().unwrap();
    assert!(
        renderer
            .source_video_planes(&fixture, Ticks::ZERO, false)
            .unwrap()
            .is_none()
    );
    let size = Size::new(160, 90);
    let monitor = renderer
        .source_rgba8_composed(
            &fixture,
            Ticks::ZERO,
            size,
            SourceFit::Stretch,
            &Default::default(),
            false,
        )
        .unwrap();
    let snapshot = renderer
        .source_rgba8_export(&fixture, Ticks::ZERO, size, SourceFit::Stretch, None, false)
        .unwrap();
    assert_eq!(monitor, snapshot);
    assert!(monitor.chunks_exact(4).all(|pixel| pixel[3] == 255));
    assert!(monitor.chunks_exact(4).any(|pixel| pixel[0] > 200));
    // The monitor's endpoint (the original's 2 s) holds the final, green picture.
    let mut at = |seconds: f64| {
        renderer
            .source_rgba8_composed(
                &fixture,
                support::ticks(seconds),
                size,
                SourceFit::Stretch,
                &Default::default(),
                false,
            )
            .unwrap()
    };
    let (end, last) = (at(2.0), at(1.99));
    assert_eq!(end, last, "the endpoint holds the final frame");
    assert!(
        end.chunks_exact(4)
            .all(|pixel| pixel[3] == 255 && pixel[1] > 200 && pixel[2] < 40),
        "final frame is opaque green"
    );
}

#[test]
fn original_rotated_anamorphic_video_has_the_same_geometry_in_monitor_and_filmstrip() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("wide.mp4");
    let rotated = dir.path().join("rotated.mp4");
    let ffmpeg =
        std::path::PathBuf::from(std::env::var_os("PM_FFMPEG_DIR").unwrap()).join("bin/ffmpeg");
    assert!(std::process::Command::new(&ffmpeg).args([
        "-v", "error", "-nostdin", "-f", "lavfi", "-i",
        "color=red:size=64x32:rate=30:duration=1,drawbox=x=32:y=0:w=32:h=32:color=blue:t=fill,setsar=2/1",
        "-c:v", "h264_videotoolbox", "-pix_fmt", "yuv420p", "-threads", "2", "-y",
    ]).arg(&base).status().unwrap().success());
    assert!(
        std::process::Command::new(&ffmpeg)
            .args(["-v", "error", "-nostdin", "-display_rotation", "90", "-i"])
            .arg(&base)
            .args(["-c", "copy", "-y"])
            .arg(&rotated)
            .status()
            .unwrap()
            .success()
    );
    add_test_clean_aperture(&rotated, 32, 32);
    let source = pm_media::decode::service::MediaSource::open(&rotated).unwrap();
    let reading = pm_media::interpret::resolve_video(
        source.probe.video.as_ref().unwrap(),
        &Default::default(),
    );
    assert_eq!(reading.pixel_aspect.value, (2, 1));
    assert_eq!(reading.rotation, 90.0);
    assert_eq!(
        source
            .probe
            .video
            .as_ref()
            .unwrap()
            .clean_aperture
            .unwrap()
            .rect,
        (16, 0, 32, 32),
    );
    let size = Size::new(32, 128);
    let mut renderer = Renderer::headless().unwrap();
    assert!(
        renderer
            .source_video_planes(&rotated, Ticks::ZERO, false)
            .unwrap()
            .is_none()
    );
    let monitor = renderer
        .source_rgba8_composed(
            &rotated,
            Ticks::ZERO,
            size,
            SourceFit::Contain,
            &Default::default(),
            false,
        )
        .unwrap();
    let display = renderer
        .source_rgba8(&rotated, Ticks::ZERO, size, SourceFit::Contain, None, false)
        .unwrap();
    let strip = renderer
        .source_rgba8_batch(
            &rotated,
            &[Ticks::ZERO, Ticks::from_whole_seconds(1)],
            size,
            SourceFit::Contain,
            None,
            false,
        )
        .unwrap();
    assert_eq!(monitor, display);
    assert_eq!(strip[0], display);
    let last = renderer
        .source_rgba8(
            &rotated,
            Ticks::from_whole_seconds(1),
            size,
            SourceFit::Contain,
            None,
            false,
        )
        .unwrap();
    assert!(
        strip[1] == last,
        "batch endpoint matches the independently requested last frame"
    );
    let at = |y: usize| &display[(y * 32 + 16) * 4..(y * 32 + 16) * 4 + 4];
    assert!(
        at(24)[2] > 200 && at(24)[0] < 50,
        "rotated top is blue: {:?}",
        at(24)
    );
    assert!(
        at(104)[0] > 200 && at(104)[2] < 50,
        "rotated bottom is red: {:?}",
        at(104)
    );
    assert!(
        display.chunks_exact(4).all(|p| p[3] == 255),
        "pixel aspect fills the portrait without bars"
    );
    for (overrides, output) in [
        (json!({"pixelAspect":[1,1]}), Size::new(32, 64)),
        (json!({"cleanAperture":true}), Size::new(32, 64)),
        (
            json!({"cleanAperture":true,"pixelAspect":[1,1]}),
            Size::new(32, 32),
        ),
    ] {
        let (interpretation, issues) = pm_media::interpret::Interpretation::from_json(&overrides);
        assert!(issues.is_empty(), "{issues:?}");
        let monitor = renderer
            .source_rgba8_composed(
                &rotated,
                Ticks::ZERO,
                output,
                SourceFit::Contain,
                &interpretation,
                false,
            )
            .unwrap();
        let strip = renderer
            .source_rgba8_batch(
                &rotated,
                &[Ticks::ZERO],
                output,
                SourceFit::Contain,
                Some(&interpretation),
                false,
            )
            .unwrap();
        assert_eq!(monitor, strip[0], "{overrides}");
        assert!(
            monitor.chunks_exact(4).all(|p| p[3] == 255),
            "{overrides}: expected display size fills output"
        );
        let pixel = |y: u32| {
            let at = ((y * output.width + output.width / 2) * 4) as usize;
            &monitor[at..at + 4]
        };
        assert!(pixel(output.height / 4)[2] > 200, "{overrides}: blue top");
        assert!(
            pixel(3 * output.height / 4)[0] > 200,
            "{overrides}: red bottom"
        );
    }
}

#[test]
fn source_interlaced_field_times_follow_file_and_explicit_interpretation() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let raw = dir.path().join("fields.yuv");
    let video = dir.path().join("interlaced.mov");
    let mut file = std::fs::File::create(&raw).unwrap();
    // Sixteen 50 fps field pictures woven into eight 25 fps frames: field 2k
    // on the even (top) lines, field 2k+1 on the odd lines. Both field parity
    // and frame index are visible, including on a conformed grid.
    let field_luma = |field: u8| {
        if field.is_multiple_of(2) {
            16 + field / 2 * 8
        } else {
            235 - field / 2 * 8
        }
    };
    for frame in 0..8u8 {
        for line in 0..48u8 {
            file.write_all(&[field_luma(2 * frame + line % 2); 64])
                .unwrap();
        }
        file.write_all(&[128; 64 * 48]).unwrap(); // 4:2:2 Cb and Cr planes
    }
    drop(file);
    let ffmpeg =
        std::path::PathBuf::from(std::env::var_os("PM_FFMPEG_DIR").unwrap()).join("bin/ffmpeg");
    assert!(
        std::process::Command::new(ffmpeg)
            .args([
                "-v",
                "error",
                "-nostdin",
                "-f",
                "rawvideo",
                "-pixel_format",
                "yuv422p",
                "-video_size",
                "64x48",
                "-framerate",
                "25",
                "-i",
            ])
            .arg(&raw)
            .args([
                "-vf",
                "setfield=tff",
                "-an",
                "-c:v",
                "prores_videotoolbox",
                "-profile:v",
                "standard",
                "-pix_fmt",
                "p210le",
                "-field_order",
                "tt",
                "-flags",
                "+ilme+ildct",
                "-threads",
                "2",
                "-y",
            ])
            .arg(&video)
            .status()
            .unwrap()
            .success()
    );
    let source = pm_media::decode::service::MediaSource::open(&video).unwrap();
    assert_eq!(source.rate(), pm_time::Rate::new(25, 1).unwrap());
    assert_eq!(
        source.probe.video.as_ref().unwrap().fields.order,
        pm_media::probe::FieldOrder::TopFirst
    );
    let size = Size::new(64, 48);
    let mut renderer = Renderer::headless().unwrap();
    assert!(
        renderer
            .source_video_planes(&video, Ticks::ZERO, false)
            .unwrap()
            .is_none()
    );
    let mut doc = project(
        64,
        48,
        2.0,
        no_fill(),
        vec![layer(
            "v",
            "video",
            json!({"asset":"clip","w":64,"h":48,"fit":"stretch"}),
            32.0,
            24.0,
            2.0,
        )],
    );
    // Composition's field-rate output is already covered against bwdif in
    // footage.rs. A sufficiently fast composition supplies the same oracle
    // for source-time requests, without imposing its cadence on the source.
    doc["fps"] = json!(120);
    asset(&mut doc, "clip", "video", "mov", 64, 48, 0.32);
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), video.clone());
    let request = RenderRequest::at_scale(size, 1.0, Precision::Float16);
    for overrides in [
        json!({}),
        json!({"fieldOrder":"bottomFirst"}),
        json!({"fieldOrder":"progressive"}),
        json!({"frameRate":10}),
        json!({"fieldOrder":"bottomFirst","frameRate":10}),
        json!({"fieldOrder":"progressive","frameRate":10}),
    ] {
        let (interpretation, issues) = pm_media::interpret::Interpretation::from_json(&overrides);
        assert!(issues.is_empty(), "{issues:?}");
        let interpretation = (!interpretation.is_empty()).then_some(interpretation);
        let times: Vec<_> = if overrides.get("frameRate").is_some() {
            [0.1, 0.15, 0.2, 0.25, 0.8, 1.0]
        } else {
            [0.08, 0.1, 0.12, 0.14, 0.32, 1.0]
        }
        .into_iter()
        .map(support::ticks)
        .collect();
        let strip = renderer
            .source_rgba8_batch(
                &video,
                &times,
                size,
                SourceFit::Stretch,
                interpretation.as_ref(),
                false,
            )
            .unwrap();
        if overrides["fieldOrder"] == "progressive" {
            assert_eq!(
                strip[0], strip[1],
                "{overrides}: progressive override retains the whole frame"
            );
        } else {
            assert!(
                strip[0]
                    .iter()
                    .zip(&strip[1])
                    .any(|(a, b)| a.abs_diff(*b) > 50),
                "{overrides}: second field must be visible at its own source time"
            );
        }
        assert_eq!(strip[4], strip[5], "{overrides}: hold the final picture");
        // An explicit top-first reading lets composition draw this fixture
        // through its existing interpreted-media gate; the source's default
        // reading must produce exactly that same field sequence.
        let mut composed_overrides = overrides.clone();
        if composed_overrides.get("fieldOrder").is_none() {
            composed_overrides["fieldOrder"] = json!("topFirst");
        }
        doc["assets"]["clip"]["interpret"] = composed_overrides;
        for (i, &time) in times.iter().enumerate() {
            let monitor = renderer
                .source_rgba8(
                    &video,
                    time,
                    size,
                    SourceFit::Stretch,
                    interpretation.as_ref(),
                    false,
                )
                .unwrap();
            assert_eq!(
                monitor, strip[i],
                "{overrides} at {time:?}: monitor/filmstrip"
            );
            let export = renderer
                .source_rgba8_export(
                    &video,
                    time,
                    size,
                    SourceFit::Stretch,
                    interpretation.as_ref(),
                    false,
                )
                .unwrap();
            assert_eq!(
                monitor, export,
                "{overrides} at {time:?}: direct job decode"
            );
            let frame = renderer
                .render(&load(&doc), "Ptest-root", time, &request, &resolver)
                .unwrap();
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            let composed = renderer.read_rgba(&frame).unwrap();
            assert!(
                monitor
                    .iter()
                    .zip(composed)
                    .all(|(&a, b)| { a.abs_diff((b.clamp(0.0, 1.0) * 255.0).round() as u8) <= 1 }),
                "{overrides} at {time:?}: source/composition fields"
            );
            renderer.recycle(frame);
        }
    }
}

#[test]
fn native_source_gamma_png_preserves_file_curve_depth_and_partial_alpha() {
    let dir = tempfile::tempdir().unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let size = Size::new(3, 1);
    let stored = [(64u8, 255u8), (128, 128), (192, 64)];
    let display_encode = |linear: f64| {
        if linear <= 0.003_130_8 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        }
    };
    for sixteen in [false, true] {
        for gamma in [70_000u32, 100_000, 45_455] {
            let path = dir.path().join(format!("gamma-{gamma}-{sixteen}.png"));
            let mut png = png::Encoder::new(std::fs::File::create(&path).unwrap(), 3, 1);
            png.set_color(png::ColorType::Rgba);
            png.set_depth(if sixteen {
                png::BitDepth::Sixteen
            } else {
                png::BitDepth::Eight
            });
            png.set_source_gamma(png::ScaledFloat::from_scaled(gamma));
            let mut pixels = vec![];
            for (value, alpha) in stored {
                for channel in [value, value, value, alpha] {
                    if sixteen {
                        pixels.extend((u16::from(channel) * 257).to_be_bytes());
                    } else {
                        pixels.push(channel);
                    }
                }
            }
            png.write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
            let actual = renderer
                .source_rgba8(&path, Ticks::ZERO, size, SourceFit::Stretch, None, false)
                .unwrap();
            let composed = renderer
                .source_rgba8_composed(
                    &path,
                    Ticks::ZERO,
                    size,
                    SourceFit::Stretch,
                    &Default::default(),
                    false,
                )
                .unwrap();
            let poster = renderer
                .source_rgba8_export(&path, Ticks::ZERO, size, SourceFit::Stretch, None, false)
                .unwrap();
            let strip = renderer
                .source_rgba8_batch(
                    &path,
                    &[Ticks::ZERO, Ticks::from_whole_seconds(1)],
                    size,
                    SourceFit::Stretch,
                    None,
                    false,
                )
                .unwrap();
            assert_eq!(actual, composed);
            assert_eq!(actual, poster);
            assert!(strip.iter().all(|frame| frame == &actual));
            for (pixel, (value, alpha)) in actual.chunks_exact(4).zip(stored) {
                let encoded = if gamma == 45_455 {
                    // ImageIO's measured sRGB window uses the sRGB curve.
                    f64::from(value) / 255.0
                } else {
                    display_encode((f64::from(value) / 255.0).powf(100_000.0 / f64::from(gamma)))
                };
                let expected = (encoded * f64::from(alpha)).round() as u8;
                assert!(
                    pixel[..3].iter().all(|&v| v.abs_diff(expected) <= 1),
                    "gAMA {gamma}, sixteen={sixteen}: {pixel:?}, expected premultiplied {expected}"
                );
                assert!(pixel[3].abs_diff(alpha) <= 1);
            }
            if gamma != 45_455 {
                let mut interpretation = pm_media::interpret::Interpretation {
                    alpha: Some(pm_media::interpret::AlphaChoice::Ignore),
                    ..Default::default()
                };
                let error = renderer
                    .source_rgba8(
                        &path,
                        Ticks::ZERO,
                        size,
                        SourceFit::Stretch,
                        Some(&interpretation),
                        false,
                    )
                    .unwrap_err();
                assert!(error.contains("gAMA"), "{error}");
                interpretation.transfer = Some(pm_protocol::color::Transfer::Linear);
                assert!(
                    renderer
                        .source_rgba8(
                            &path,
                            Ticks::ZERO,
                            size,
                            SourceFit::Stretch,
                            Some(&interpretation),
                            false,
                        )
                        .is_ok(),
                    "an explicit transfer still replaces the PNG curve"
                );
            }
        }
    }
}

#[test]
fn native_source_heif_rejects_pq_and_hlg_container_transfers() {
    let dir = tempfile::tempdir().unwrap();
    // Mutate only the transfer code of a valid HEIF: item data, offsets,
    // property associations and dimensions remain intact. ImageIO's SDR
    // sample label must not erase the file's declared HDR transfer.
    let original = include_bytes!("../../../../apps/desktop/e2e/fixtures/still.heic");
    let nclx = original
        .windows(4)
        .position(|bytes| bytes == b"nclx")
        .unwrap();
    let size = Size::new(16, 16);
    let mut renderer = Renderer::headless().unwrap();
    for transfer in [16u16, 18] {
        let mut bytes = original.to_vec();
        bytes[nclx + 6..nclx + 8].copy_from_slice(&transfer.to_be_bytes());
        let path = dir.path().join(format!("hdr-{transfer}.heic"));
        std::fs::write(&path, bytes).unwrap();
        let default = pm_media::interpret::Interpretation::default();
        let error = pm_render::media::probe_interpreted_image(&path, &default).unwrap_err();
        assert!(
            matches!(error, pm_render::media::ProbeIssue::Unsupported(ref feature)
            if feature.to_string().contains("PQ/HLG")),
            "{error:?}"
        );
        let errors = [
            renderer
                .source_rgba8(&path, Ticks::ZERO, size, SourceFit::Stretch, None, false)
                .unwrap_err(),
            renderer
                .source_rgba8_composed(
                    &path,
                    Ticks::ZERO,
                    size,
                    SourceFit::Stretch,
                    &default,
                    false,
                )
                .unwrap_err(),
            renderer
                .source_rgba8_export(&path, Ticks::ZERO, size, SourceFit::Stretch, None, false)
                .unwrap_err(),
            renderer
                .source_rgba8_batch(&path, &[Ticks::ZERO], size, SourceFit::Stretch, None, false)
                .unwrap_err(),
        ];
        for error in errors {
            assert!(
                error.contains("PQ/HLG"),
                "nclx transfer {transfer}: {error}"
            );
        }
        let mut override_reading = default;
        override_reading.transfer = Some(pm_protocol::color::Transfer::DisplayE);
        assert!(
            renderer
                .source_rgba8(
                    &path,
                    Ticks::ZERO,
                    size,
                    SourceFit::Stretch,
                    Some(&override_reading),
                    false,
                )
                .unwrap_err()
                .contains("PQ/HLG"),
            "unsupported container HDR cannot be hidden by an SDR override"
        );
    }
}

#[test]
fn source_stretch_and_contain_preserve_rgba_and_transparent_bars() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.png");
    let pixels = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let file = std::fs::File::create(&path).unwrap();
    let mut png = png::Encoder::new(file, 2, 2);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let stretch = renderer
        .source_rgba8(
            &path,
            Ticks::ZERO,
            Size::new(2, 2),
            SourceFit::Stretch,
            None,
            false,
        )
        .unwrap();
    assert_eq!(stretch, pixels);
    let contain = renderer
        .source_rgba8(
            &path,
            Ticks::ZERO,
            Size::new(4, 2),
            SourceFit::Contain,
            None,
            false,
        )
        .unwrap();
    for y in 0..2 {
        assert_eq!(&contain[(y * 4) * 4..(y * 4 + 1) * 4], &[0, 0, 0, 0]);
        assert_eq!(
            &contain[(y * 4 + 1) * 4..(y * 4 + 3) * 4],
            &pixels[(y * 2) * 4..(y * 2 + 2) * 4]
        );
        assert_eq!(&contain[(y * 4 + 3) * 4..(y * 4 + 4) * 4], &[0, 0, 0, 0]);
    }
}

#[test]
fn source_snapshot_reads_premultiplied_alpha_for_snapshot_pixel_encoding() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("alpha.png");
    let file = std::fs::File::create(&path).unwrap();
    let mut png = png::Encoder::new(file, 1, 1);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()
        .unwrap()
        .write_image_data(&[255, 0, 0, 128])
        .unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let rgba = renderer
        .source_rgba8(
            &path,
            Ticks::ZERO,
            Size::new(1, 1),
            SourceFit::Stretch,
            None,
            false,
        )
        .unwrap();
    assert_eq!(rgba, [128, 0, 0, 128]);
}

#[test]
fn source_svg_renders_natively_with_fit_alpha_and_batch_reuse() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.svg");
    std::fs::write(&path, r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><rect width="4" height="2" fill="red" fill-opacity="0.25"/></svg>"#).unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let size = Size::new(4, 4);
    for fit in [SourceFit::Contain, SourceFit::Cover, SourceFit::Stretch] {
        let pixels = renderer
            .source_rgba8(&path, Ticks::ZERO, size, fit, None, false)
            .unwrap();
        for y in 0..4 {
            for x in 0..4 {
                let p = &pixels[(y * 4 + x) * 4..][..4];
                let wanted = if fit == SourceFit::Contain && (y == 0 || y == 3) {
                    [0, 0, 0, 0]
                } else {
                    [64, 0, 0, 64]
                };
                assert_eq!(p, wanted, "{fit:?} ({x},{y})");
            }
        }
        let frames = renderer
            .source_rgba8_batch(
                &path,
                &[Ticks::ZERO, Ticks::new(1).unwrap()],
                size,
                fit,
                None,
                false,
            )
            .unwrap();
        assert_eq!(frames, vec![pixels.clone(), pixels]);
    }
}

#[test]
fn source_svg_rejects_an_interpreted_color_override() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.svg");
    std::fs::write(&path, r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2"/></svg>"#).unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let interpretation = pm_media::interpret::Interpretation {
        transfer: Some(pm_protocol::color::Transfer::Linear),
        ..Default::default()
    };
    let error = renderer
        .source_rgba8(
            &path,
            Ticks::ZERO,
            Size::new(1, 1),
            SourceFit::Stretch,
            Some(&interpretation),
            false,
        )
        .unwrap_err();
    assert!(
        error.contains("interpretation cannot be applied"),
        "{error}"
    );
}

#[test]
fn untagged_vp9_rendition_uses_native_composition_colour() {
    // A browser rendition is still footage: its source snapshot must use
    // the native colour reading and conversion, without Canvas2D rounding.
    // Use the same converter flags as import.
    let dir = tempfile::tempdir().unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../apps/desktop/e2e/fixtures/prores422.mov");
    let proxy = dir.path().join("browser.webm");
    let ffmpeg = std::env::var_os("PM_FFMPEG_DIR")
        .map(std::path::PathBuf::from)
        .expect("source engine/env.sh")
        .join("bin/ffmpeg");
    let status = std::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(&source)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-c:v",
            "libvpx-vp9",
            "-pix_fmt",
            "yuva420p",
            "-b:v",
            "0",
            "-crf",
            "18",
            "-g",
            "15",
            "-deadline",
            "good",
            "-cpu-used",
            "4",
            "-row-mt",
            "1",
            "-threads",
            "4",
            "-c:a",
            "libopus",
            "-b:a",
            "192k",
            "-f",
            "webm",
            "-y",
        ])
        .arg(&proxy)
        .status()
        .unwrap();
    assert!(status.success());
    let pixels = Renderer::headless()
        .unwrap()
        .source_rgba8(
            &proxy,
            Ticks::ZERO,
            Size::new(160, 90),
            SourceFit::Stretch,
            None,
            false,
        )
        .unwrap();
    assert_eq!(pixels.len(), 160 * 90 * 4);
    let mut doc = project(
        160,
        90,
        2.0,
        no_fill(),
        vec![layer(
            "v",
            "video",
            json!({ "asset": "clip", "fit": "stretch", "w": 160, "h": 90 }),
            80.0,
            45.0,
            2.0,
        )],
    );
    asset(&mut doc, "clip", "video", "webm", 160, 90, 2.0);
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), proxy.clone());
    let mut composition = Renderer::headless().unwrap();
    let request = RenderRequest::at_scale(Size::new(160, 90), 1.0, Precision::Float16);
    let frame = composition
        .render(&load(&doc), "Ptest-root", Ticks::ZERO, &request, &resolver)
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let composed = composition.read_rgba(&frame).unwrap();
    let max = pixels
        .iter()
        .zip(composed)
        .map(|(&source, value)| source.abs_diff((value.clamp(0.0, 1.0) * 255.0).round() as u8))
        .max()
        .unwrap();
    assert!(max <= 1, "source vs native composition: {max} codes");
    composition.recycle(frame);

    // The monitor's coded planes for this rendition alternate across its
    // red (0–1 s) / green (1–2 s) cut, as resize-then-seek does in the app.
    let mut renderer = Renderer::headless().unwrap();
    for seconds in [1.5, 0.5, 1.5, 0.5, 0.5, 1.5, 0.5, 1.5] {
        let time =
            Ticks::new((seconds * pm_protocol::common::TICKS_PER_SECOND as f64) as i64).unwrap();
        let (_, planes, _) = renderer
            .source_video_planes(&proxy, time, false)
            .unwrap()
            .expect("coded planes");
        let luma = planes[0];
        let red = seconds < 1.0;
        assert!(
            if red { luma < 110 } else { luma > 120 },
            "{seconds} s returned luma {luma}"
        );
    }
}

#[test]
fn source_scrub_reuses_decoded_video_and_counts_work() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.webm");
    let ffmpeg = std::env::var_os("PM_FFMPEG_DIR")
        .map(std::path::PathBuf::from)
        .expect("source engine/env.sh")
        .join("bin/ffmpeg");
    let status = std::process::Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x48:rate=30:duration=1",
            "-an",
            "-c:v",
            "libvpx-vp9",
            "-b:v",
            "0",
            "-crf",
            "18",
            "-threads",
            "2",
            "-f",
            "webm",
            "-y",
        ])
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    let mut renderer = Renderer::headless().unwrap();
    let size = Size::new(64, 48);
    let times = [0, 7, 7, 7, 14, 14];
    let scope = pm_perf::Scope::begin();
    let began = std::time::Instant::now();
    let mut frames = Vec::new();
    for frame in times {
        let ticks = Ticks::new(frame * pm_protocol::common::TICKS_PER_SECOND / 30).unwrap();
        frames.push(
            renderer
                .source_rgba8(&path, ticks, size, SourceFit::Stretch, None, false)
                .unwrap(),
        );
    }
    let elapsed = began.elapsed();
    let counters = scope.finish();
    assert_eq!(frames[1], frames[2]);
    assert_eq!(frames[1], frames[3]);
    assert_eq!(frames[4], frames[5]);
    assert!(
        counters.get(pm_perf::Counter::MediaCacheHits) >= 3,
        "repeated seeks should reuse converted frames: {}",
        counters.to_json()
    );
    assert!(counters.get(pm_perf::Counter::DrawCalls) >= times.len() as u64);
    assert!(counters.get(pm_perf::Counter::Readbacks) >= times.len() as u64);
    let source_buffers = renderer.frame_reuse_bytes();
    assert!(
        source_buffers > 0,
        "source target and staging buffer are reusable"
    );
    renderer.trim(64 << 20);
    assert_eq!(
        renderer.frame_reuse_bytes(),
        source_buffers,
        "a prepared source keeps its bounded readback across job work"
    );
    let end = renderer
        .source_rgba8(
            &path,
            Ticks::from_whole_seconds(1),
            size,
            SourceFit::Stretch,
            None,
            false,
        )
        .unwrap();
    let last = renderer
        .source_rgba8(
            &path,
            Ticks::new(29 * pm_protocol::common::TICKS_PER_SECOND / 30).unwrap(),
            size,
            SourceFit::Stretch,
            None,
            false,
        )
        .unwrap();
    assert_eq!(end, last, "an End seek holds the last presented picture");
    renderer.trim(0);
    assert_eq!(renderer.frame_reuse_bytes(), 0);
    eprintln!(
        "source scrub: 6 frames in {:.3} ms; {}",
        elapsed.as_secs_f64() * 1000.0,
        counters.to_json()
    );
}

#[test]
fn source_batch_matches_individual_video_frames_in_order() {
    assert_eq!(Renderer::source_batch_limit(Size::new(160, 90)), 16);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("moving.mp4");
    let ffmpeg = std::env::var_os("PM_FFMPEG_DIR")
        .map(std::path::PathBuf::from)
        .expect("source engine/env.sh")
        .join("bin/ffmpeg");
    let status = std::process::Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x90:rate=30:duration=2",
            "-an",
            "-c:v",
            "h264_videotoolbox",
            "-pix_fmt",
            "yuv420p",
            "-g",
            "30",
            "-threads",
            "2",
            "-f",
            "mp4",
            "-y",
        ])
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    let size = Size::new(160, 90);
    let times = [0, 13, 27, 40, 13, 53, 0, 21, 2, 57, 30, 16, 35, 5, 59, 42]
        .map(|frame| Ticks::new(frame * pm_protocol::common::TICKS_PER_SECOND / 30).unwrap());
    let mut individual = Renderer::headless().unwrap();
    let expected: Vec<_> = times
        .iter()
        .map(|&time| {
            individual
                .source_rgba8(&path, time, size, SourceFit::Stretch, None, false)
                .unwrap()
        })
        .collect();
    assert_ne!(expected[0], expected[1], "the fixture must show motion");
    assert_ne!(expected[1], expected[2], "the fixture must show motion");
    let decode = individual.source_scrub_stats();
    assert_eq!(decode.stand_ins, 0);
    assert_eq!(decode.workers, 1, "adjacent seeks share one decoder worker");
    let mut batched = Renderer::headless().unwrap();
    let scope = pm_perf::Scope::begin();
    let actual = batched
        .source_rgba8_batch(&path, &times, size, SourceFit::Stretch, None, false)
        .unwrap();
    let counters = scope.finish();
    assert_eq!(actual, expected);
    assert_eq!(counters.get(pm_perf::Counter::Submits), 1);
    assert_eq!(counters.get(pm_perf::Counter::Readbacks), 1);
    assert_eq!(
        counters.get(pm_perf::Counter::DrawCalls),
        times.len() as u64
    );
}

/// The source monitor shows Interpret Footage's reading exactly as the
/// composition draws the same source: no coded planes (Chromium would apply
/// the probe's tags), and the engine's composed frame equals a full-frame
/// layer of that asset within one code.
#[test]
fn monitor_reads_interpret_footage_as_composition_does() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../apps/desktop/e2e/fixtures/h264-aac.mp4");
    let (w, h) = (160u32, 90u32);
    let interpret = json!({ "matrix": "bt709", "range": "full" });
    let (interpretation, issues) = pm_media::interpret::Interpretation::from_json(&interpret);
    assert!(
        issues.is_empty() && !interpretation.is_empty(),
        "{issues:?}"
    );
    let mut doc = project(
        w,
        h,
        2.0,
        no_fill(),
        vec![layer(
            "v",
            "video",
            json!({ "asset": "clip", "fit": "stretch", "w": w, "h": h, "trim": 0, "speed": 1 }),
            f64::from(w) / 2.0,
            f64::from(h) / 2.0,
            2.0,
        )],
    );
    asset(&mut doc, "clip", "video", "mp4", w, h, 2.0);
    doc["assets"]["clip"]["interpret"] = interpret;
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), fixture.clone());

    let mut renderer = Renderer::headless().unwrap();
    let time = support::ticks(0.5);
    let request = RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float16);
    let frame = renderer
        .render(&load(&doc), "Ptest-root", time, &request, &resolver)
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let composed: Vec<u8> = renderer
        .read_rgba(&frame)
        .unwrap()
        .iter()
        .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();
    let monitor = renderer
        .source_rgba8_composed(
            &fixture,
            time,
            Size::new(w, h),
            SourceFit::Stretch,
            &interpretation,
            false,
        )
        .unwrap();
    let probe = renderer
        .source_rgba8_composed(
            &fixture,
            time,
            Size::new(w, h),
            SourceFit::Stretch,
            &Default::default(),
            false,
        )
        .unwrap();
    let max = |a: &[u8], b: &[u8]| a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
    assert_eq!(monitor.len(), composed.len());
    assert!(
        max(&monitor, &composed) <= 1,
        "monitor vs composition: {}",
        max(&monitor, &composed)
    );
    // The override is visible: full-range BT.709 is not the file's reading.
    assert!(
        max(&probe, &monitor) > 8,
        "interpretation not applied: {}",
        max(&probe, &monitor)
    );
}

#[test]
fn sequence_sources_share_composition_slots_and_hd_colour_reading() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let raw = dir.path().join("frames.yuv");
    let video = dir.path().join("sequence.webm");
    let mut file = std::fs::File::create(&raw).unwrap();
    for codes in [[81u8, 90, 240], [145, 54, 34], [41, 240, 110]] {
        for (channel, value) in codes.into_iter().enumerate() {
            let samples = 1920 * 1080 / if channel == 0 { 1 } else { 4 };
            file.write_all(&vec![value; samples]).unwrap();
        }
    }
    drop(file);
    let ffmpeg =
        std::path::PathBuf::from(std::env::var_os("PM_FFMPEG_DIR").unwrap()).join("bin/ffmpeg");
    assert!(
        std::process::Command::new(ffmpeg)
            .args([
                "-v",
                "error",
                "-nostdin",
                "-f",
                "rawvideo",
                "-pixel_format",
                "yuv420p",
                "-video_size",
                "1920x1080",
                "-framerate",
                "10",
                "-i"
            ])
            .arg(&raw)
            .args([
                "-an",
                "-c:v",
                "libvpx-vp9",
                "-lossless",
                "1",
                "-deadline",
                "realtime",
                "-cpu-used",
                "8",
                "-threads",
                "2",
                "-y"
            ])
            .arg(&video)
            .status()
            .unwrap()
            .success()
    );
    let mut doc = project(
        16,
        16,
        1.0,
        no_fill(),
        vec![layer(
            "v",
            "video",
            json!({"asset":"clip","w":16,"h":16,"fit":"stretch"}),
            8.0,
            8.0,
            1.0,
        )],
    );
    asset(&mut doc, "clip", "video", "webm", 1920, 1080, 0.3);
    doc["assets"]["clip"]["imageSequence"] = json!({"fps":10,"frames":3});
    let record = pm_model::Node::from_value(
        pm_model::json::parse(&doc["assets"]["clip"].to_string()).unwrap(),
        0,
    );
    assert!(pm_render::footage::is_sequence(&record));
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), video.clone());
    let mut renderer = Renderer::headless().unwrap();
    let size = Size::new(16, 16);
    let request = RenderRequest::at_scale(size, 1.0, Precision::Float16);
    let times: Vec<_> = [0.0, 0.1 - 2e-7, 0.1 - 5e-8, 0.1, 0.2, 0.3, 0.9]
        .into_iter()
        .map(support::ticks)
        .collect();
    let mapped: Vec<_> = times
        .iter()
        .map(|&time| pm_render::footage::source_time(Some(&record), time, None))
        .collect();
    let strip = renderer
        .source_rgba8_batch(
            &video,
            &mapped,
            size,
            SourceFit::Stretch,
            None,
            pm_render::footage::is_sequence(&record),
        )
        .unwrap();
    for (i, (&time, &source_time)) in times.iter().zip(&mapped).enumerate() {
        let frame = renderer
            .render(&load(&doc), "Ptest-root", time, &request, &resolver)
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        let composed: Vec<u8> = renderer
            .read_rgba(&frame)
            .unwrap()
            .iter()
            .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect();
        let monitor = renderer
            .source_rgba8_composed(
                &video,
                source_time,
                size,
                SourceFit::Stretch,
                &Default::default(),
                true,
            )
            .unwrap();
        assert_eq!(monitor, strip[i]);
        let max = monitor
            .iter()
            .zip(&composed)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(max <= 1, "sequence source/composition at {time:?}: {max}");
    }
    assert_eq!(strip[0], strip[1]);
    assert_ne!(strip[1], strip[2]);
    assert_eq!(strip[2], strip[3]);
    assert_eq!(strip[4], strip[5]);
    assert_eq!(strip[5], strip[6]);
    let ordinary = renderer
        .source_rgba8(&video, mapped[0], size, SourceFit::Stretch, None, false)
        .unwrap();
    assert!(
        ordinary
            .iter()
            .zip(&strip[0])
            .any(|(a, b)| a.abs_diff(*b) > 8),
        "HD sequence keeps its BT.601 reading"
    );

    // Older animated imports can retain only their original format, with no
    // persisted slot metadata. Their colour reading is still a sequence's;
    // frame selection then uses the rendition's native timestamps.
    doc["assets"]["clip"]
        .as_object_mut()
        .unwrap()
        .remove("imageSequence");
    for format in ["gif", "webp", "png", "apng", "avif"] {
        doc["assets"]["clip"]["format"] = json!(format);
        let record = pm_model::Node::from_value(
            pm_model::json::parse(&doc["assets"]["clip"].to_string()).unwrap(),
            0,
        );
        let sequence = pm_render::footage::is_sequence(&record);
        assert!(sequence, "format-only {format}");
        let time = support::ticks(0.1);
        assert_eq!(
            pm_render::footage::source_time(Some(&record), time, None),
            time
        );
        let monitor = renderer
            .source_rgba8_composed(
                &video,
                time,
                size,
                SourceFit::Stretch,
                &Default::default(),
                sequence,
            )
            .unwrap();
        let filmstrip = renderer
            .source_rgba8_batch(&video, &[time], size, SourceFit::Stretch, None, sequence)
            .unwrap();
        assert_eq!(monitor, strip[3], "format-only {format}");
        assert_eq!(monitor, filmstrip[0]);
        let frame = renderer
            .render(&load(&doc), "Ptest-root", time, &request, &resolver)
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        let composed = renderer.read_rgba(&frame).unwrap();
        assert!(
            monitor
                .iter()
                .zip(composed)
                .all(|(&a, b)| { a.abs_diff((b.clamp(0.0, 1.0) * 255.0).round() as u8) <= 1 }),
            "format-only {format}: source/composition colour"
        );
        renderer.recycle(frame);
    }
    doc["assets"]["clip"]["format"] = json!("webm");
    let ordinary_record = pm_model::Node::from_value(
        pm_model::json::parse(&doc["assets"]["clip"].to_string()).unwrap(),
        0,
    );
    assert!(!pm_render::footage::is_sequence(&ordinary_record));
    let still_record = pm_model::Node::from_value(
        pm_model::json::parse(r#"{"kind":"image","format":"png"}"#).unwrap(),
        0,
    );
    assert!(!pm_render::footage::is_sequence(&still_record));
    assert_eq!(
        pm_render::footage::source_time(Some(&ordinary_record), times[1], None),
        times[1],
        "ordinary video keeps native source time",
    );

    // Conform overrides persisted slots: at 5 fps the second source frame
    // starts at 0.2 s, rather than the sequence metadata's 0.1 s.
    doc["assets"]["clip"]["imageSequence"] = json!({"fps":10,"frames":3});
    doc["assets"]["clip"]["interpret"] = json!({"frameRate":5});
    let record = pm_model::Node::from_value(
        pm_model::json::parse(&doc["assets"]["clip"].to_string()).unwrap(),
        0,
    );
    let interpretation = pm_render::footage::interpretation(&record).unwrap();
    let conformed_times = [
        support::ticks(0.1),
        support::ticks(0.2),
        support::ticks(0.9),
    ];
    for &time in &conformed_times {
        assert_eq!(
            pm_render::footage::source_time(Some(&record), time, Some(&interpretation)),
            time
        );
    }
    let conformed = renderer
        .source_rgba8_batch(
            &video,
            &conformed_times,
            size,
            SourceFit::Stretch,
            Some(&interpretation),
            pm_render::footage::is_sequence(&record),
        )
        .unwrap();
    assert_eq!(conformed[0], strip[0]);
    assert_eq!(conformed[1], strip[3]);
    assert_eq!(conformed[2], strip[4]);
    for (i, &time) in conformed_times.iter().enumerate() {
        let monitor = renderer
            .source_rgba8_composed(
                &video,
                time,
                size,
                SourceFit::Stretch,
                &interpretation,
                true,
            )
            .unwrap();
        assert_eq!(monitor, conformed[i]);
        let frame = renderer
            .render(&load(&doc), "Ptest-root", time, &request, &resolver)
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        let composed = renderer.read_rgba(&frame).unwrap();
        assert!(
            monitor
                .iter()
                .zip(composed)
                .all(|(&a, b)| { a.abs_diff((b.clamp(0.0, 1.0) * 255.0).round() as u8) <= 1 }),
            "conformed sequence source/composition at {time:?}"
        );
        renderer.recycle(frame);
    }
}
