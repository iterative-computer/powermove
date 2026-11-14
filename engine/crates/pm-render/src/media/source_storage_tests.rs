//! GPU regressions for source storage, chroma reconstruction and cache
//! isolation.

use super::*;
use pm_gpu::readback::{f32_to_half, half_to_f32, read_f32, read_raw};

#[test]
fn preview_scaling_preserves_aspect_and_never_upscales_exact_sources() {
    assert_eq!(preview_size((3840, 2160), Some(1920)), (1920, 1080));
    assert_eq!(preview_size((3024, 4032), Some(1024)), (768, 1024));
    assert_eq!(preview_size((640, 360), Some(2048)), (640, 360));
    assert_eq!(preview_size((3840, 2160), None), (3840, 2160));
}

#[test]
fn subsampled_still_then_exact_read_matches_an_independent_full_read() {
    let gpu = pm_gpu::Gpu::new("subsampled still isolation").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/still-red.png");
    let mut media = MediaCache::new(&gpu.device);
    let read = |cache: &mut MediaCache, edge| {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let target = cache
            .image_scaled(
                &gpu.device,
                &gpu.queue,
                &pipelines,
                &mut encoder,
                &path,
                WorkingEncoding::Standard,
                Precision::Float16,
                None,
                edge,
            )
            .unwrap();
        gpu.queue.submit([encoder.finish()]);
        let pixels = read_f32(&gpu.device, &gpu.queue, &target.texture).unwrap();
        (target, pixels)
    };
    let (small, _) = read(&mut media, Some(16));
    assert!(small.width() <= 16 && small.height() <= 16);
    let (full, actual) = read(&mut media, None);
    let mut independent = MediaCache::new(&gpu.device);
    let (reference, expected) = read(&mut independent, None);
    assert_eq!(
        (full.width(), full.height(), full.format()),
        (reference.width(), reference.height(), reference.format())
    );
    assert_eq!(actual, expected);
    media.trim_images(0);
    assert_eq!(media.image_bytes(), 0);
}

/// One still read at two sizes keeps both: a frame that draws it at each
/// (two layers, or a preview and an exact read) decodes it once per size,
/// not once per draw per frame.
#[test]
fn a_still_keeps_each_size_it_was_read_at() {
    let gpu = pm_gpu::Gpu::new("still sizes kept").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/still-red.png");
    let mut media = MediaCache::new(&gpu.device);
    let mut read = |edge| {
        let before = pm_perf::snapshot();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let target = media
            .image_scaled(
                &gpu.device,
                &gpu.queue,
                &pipelines,
                &mut encoder,
                &path,
                WorkingEncoding::Standard,
                Precision::Float16,
                None,
                edge,
            )
            .unwrap();
        gpu.queue.submit([encoder.finish()]);
        let work = pm_perf::snapshot().since(&before);
        (
            read_f32(&gpu.device, &gpu.queue, &target.texture).unwrap(),
            work.get(pm_perf::Counter::StillDecodes),
        )
    };
    let (small, decoded) = read(Some(16));
    assert!(decoded > 0);
    let (full, decoded) = read(None);
    assert!(decoded > 0);
    for _ in 0..3 {
        assert_eq!(read(Some(16)), (small.clone(), 0), "the preview is kept");
        assert_eq!(read(None), (full.clone(), 0), "the exact read is kept");
    }
}

/// A still larger than the whole image budget stays while frames keep
/// drawing it (it was decoded again for every frame), goes once frames
/// passed without it, and a budget of zero releases it at once.
#[test]
fn a_still_larger_than_the_budget_stays_while_frames_draw_it() {
    let gpu = pm_gpu::Gpu::new("still over budget").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/still-red.png");
    let mut media = MediaCache::new(&gpu.device);
    media.trim_images(1);
    let frame = |media: &mut MediaCache, draw: bool| {
        media.begin_frame(false);
        let before = pm_perf::snapshot();
        if draw {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            media
                .image_scaled(
                    &gpu.device,
                    &gpu.queue,
                    &pipelines,
                    &mut encoder,
                    &path,
                    WorkingEncoding::Standard,
                    Precision::Float16,
                    None,
                    None,
                )
                .unwrap();
            gpu.queue.submit([encoder.finish()]);
        }
        pm_perf::snapshot()
            .since(&before)
            .get(pm_perf::Counter::StillDecodes)
    };
    assert!(frame(&mut media, true) > 0);
    assert!(media.image_bytes() > 1, "kept beyond the budget");
    for _ in 0..4 {
        assert_eq!(frame(&mut media, true), 0, "drawn again without a decode");
    }
    // Two frames without it: the budget applies again.
    frame(&mut media, false);
    frame(&mut media, false);
    media.trim_images(1);
    assert_eq!(media.image_bytes(), 0);
    assert!(
        frame(&mut media, true) > 0,
        "decoded again once it had gone"
    );
    media.trim_images(0);
    assert_eq!(media.image_bytes(), 0, "a budget of zero releases at once");
}

#[test]
fn preview_video_conversion_does_not_replace_the_exact_source() {
    let gpu = pm_gpu::Gpu::new("preview source isolation").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/h264-aac.mp4");
    let mut media = MediaCache::new(&gpu.device);
    let convert = |media: &mut MediaCache, preview: bool| {
        media.preview_video = preview;
        media.draw_edge = Some(128);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let target = media
            .video(
                &gpu.device,
                &gpu.queue,
                &pipelines,
                &mut encoder,
                &path,
                1,
                0.2,
                false,
                WorkingEncoding::Standard,
                Precision::Float16,
                None,
                pm_time::Rate::FPS_30,
                false,
            )
            .unwrap()
            .unwrap();
        gpu.queue.submit([encoder.finish()]);
        let pixels = read_f32(&gpu.device, &gpu.queue, &target.texture).unwrap();
        media.flush();
        (target, pixels)
    };
    let (exact, pixels) = convert(&mut media, false);
    let (preview, _) = convert(&mut media, true);
    assert!(preview.width() <= 128 && preview.height() <= 128);
    let (again, after) = convert(&mut media, false);
    assert_eq!(
        (again.width(), again.height(), again.format()),
        (exact.width(), exact.height(), exact.format())
    );
    assert_eq!(pixels, after);
}

/// A preview-sized video frame averages detail finer than its pixels: 480
/// px of 3-px stripes (two white, one black) reduced 16× show a flat grey
/// (each pixel 10 or 11 sixteenths white), not moire from four source taps.
#[test]
fn preview_video_downscale_averages_detail_finer_than_its_pixels() {
    let gpu = pm_gpu::Gpu::new("preview downscale filter").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video/stripes-709-limited.mp4");
    let mut media = MediaCache::new(&gpu.device);
    let convert = |media: &mut MediaCache, preview: bool| {
        media.preview_video = preview;
        media.draw_edge = Some(30);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let target = media
            .video(
                &gpu.device,
                &gpu.queue,
                &pipelines,
                &mut encoder,
                &path,
                1,
                0.0,
                false,
                WorkingEncoding::Standard,
                Precision::Float16,
                None,
                pm_time::Rate::FPS_30,
                false,
            )
            .unwrap()
            .unwrap();
        gpu.queue.submit([encoder.finish()]);
        let pixels = read_f32(&gpu.device, &gpu.queue, &target.texture).unwrap();
        media.flush();
        (target.width(), target.height(), pixels)
    };
    let (w, _, exact) = convert(&mut media, false);
    assert_eq!(w, 480);
    let reds = |p: &[f32]| p.chunks(4).map(|c| c[0]).collect::<Vec<_>>();
    let exact = reds(&exact);
    let (black, white) = exact
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    assert!(white - black > 0.9, "stripes {black}..{white}");
    let (pw, ph, preview) = convert(&mut media, true);
    assert_eq!((pw, ph), (30, 30));
    let preview = reds(&preview);
    let (lo, hi) = preview
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    // 10/16 to 11/16 white through the transfer: well inside 0.1 of each other.
    assert!(
        hi - lo < 0.1 && lo > black + 0.3 && hi < white - 0.1,
        "preview {lo}..{hi} of stripes {black}..{white}"
    );
}

/// A preview-sized video frame is the area average of the exact frame's
/// working colour, whatever the reading: averaging encoded values and
/// converting afterwards darkens or shifts footage whose transfer is not the
/// identity (a linear-light project, PQ, premultiplied alpha with a transfer),
/// which then changes when playback pauses.
#[test]
fn preview_video_reduction_averages_working_colour() {
    let gpu = pm_gpu::Gpu::new("preview reduction space").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cases = [
        (
            root.join("tests/fixtures/video/stripes-709-limited.mp4"),
            WorkingEncoding::LinearLight,
            37u32,
        ),
        (
            root.join("tests/fixtures/video/hevc10-2020-pq.mp4"),
            WorkingEncoding::Standard,
            37,
        ),
        (
            root.join("../../fixtures/hevc-alpha.mov"),
            WorkingEncoding::LinearLight,
            23,
        ),
        (
            root.join("../../fixtures/prores4444-alpha.mov"),
            WorkingEncoding::LinearLight,
            23,
        ),
    ];
    for (path, working, edge) in cases {
        let mut media = MediaCache::new(&gpu.device);
        let mut convert = |preview: bool| {
            media.preview_video = preview;
            media.draw_edge = Some(edge);
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            let target = media
                .video(
                    &gpu.device,
                    &gpu.queue,
                    &pipelines,
                    &mut encoder,
                    &path,
                    1,
                    0.0,
                    false,
                    working,
                    Precision::Float32,
                    None,
                    pm_time::Rate::FPS_30,
                    false,
                )
                .unwrap()
                .unwrap();
            gpu.queue.submit([encoder.finish()]);
            let pixels = read_f32(&gpu.device, &gpu.queue, &target.texture).unwrap();
            media.flush();
            (target.width() as usize, target.height() as usize, pixels)
        };
        let (w, h, exact) = convert(false);
        let (pw, ph, preview) = convert(true);
        assert!(pw < w && ph < h, "{path:?}: {pw}x{ph} of {w}x{h}");
        let (sx, sy) = (w as f64 / pw as f64, h as f64 / ph as f64);
        let mut worst = (0.0f64, 0usize, 0usize);
        for py in 0..ph {
            for px in 0..pw {
                let (x0, x1) = (px as f64 * sx, (px + 1) as f64 * sx);
                let (y0, y1) = (py as f64 * sy, (py + 1) as f64 * sy);
                let mut sum = [0.0f64; 4];
                for y in y0.floor() as usize..(y1.ceil() as usize).min(h) {
                    let wy = (y as f64 + 1.).min(y1) - (y as f64).max(y0);
                    for x in x0.floor() as usize..(x1.ceil() as usize).min(w) {
                        let wt = ((x as f64 + 1.).min(x1) - (x as f64).max(x0)) * wy;
                        for c in 0..4 {
                            sum[c] += f64::from(exact[(y * w + x) * 4 + c]) * wt;
                        }
                    }
                }
                for c in 0..4 {
                    let want = sum[c] / (sx * sy);
                    let got = f64::from(preview[(py * pw + px) * 4 + c]);
                    let d = (got - want).abs() / want.abs().max(1.);
                    if d > worst.0 {
                        worst = (d, px, py);
                    }
                }
            }
        }
        assert!(
            worst.0 < 0.01,
            "{path:?}: preview off the exact average by {worst:?}"
        );
    }
}

/// Exercise the production source compute pass with normalized f32 RGB.
/// Keeping the input at f32 makes an accidental half-float conversion
/// before RGBA8 storage observable independently of a video decoder.
fn source(
    gpu: &pm_gpu::Gpu,
    pipelines: &Pipelines,
    pixels: &[[f32; 4]],
    precision: Precision,
    format: wgpu::TextureFormat,
) -> Target {
    let color = SourceColor {
        unorm8: false,
        transfer: Transfer::DisplayE,
        primaries: Primaries::Bt709,
        matrix: MatrixCoefficients::Identity,
        siting: ChromaSiting::Left,
        alpha: AlphaMode::Opaque,
        tone_map: None,
        chromaticities: None,
        power: None,
    };
    source_read(
        gpu,
        pipelines,
        pixels,
        &color,
        WorkingEncoding::Standard,
        precision,
        format,
    )
}

fn source_read(
    gpu: &pm_gpu::Gpu,
    pipelines: &Pipelines,
    pixels: &[[f32; 4]],
    color: &SourceColor,
    working: WorkingEncoding,
    precision: Precision,
    format: wgpu::TextureFormat,
) -> Target {
    let width = u32::try_from(pixels.len()).unwrap();
    let input = Target::new(
        &gpu.device,
        "source storage test input",
        width,
        1,
        wgpu::TextureFormat::Rgba32Float,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    let bytes: Vec<u8> = pixels
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    gpu.queue.write_texture(
        input.texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 16),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let normalized = Normalized::Rgb {
        rgba: input.texture,
    };
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("source storage test"),
        });
    let target = convert_format(
        &gpu.device,
        &gpu.queue,
        pipelines,
        &mut encoder,
        &normalized,
        color,
        working,
        precision,
        Factors { x: 1, y: 1 },
        None,
        format,
        false,
        None,
    );
    gpu.queue.submit([encoder.finish()]);
    target
}

/// BT.709 linear from linear light in `gamut`, through the reference's
/// matrices (pm_reference shares no code with pm-color).
fn to_rec709(gamut: &pm_reference::color::Primaries, light: [f64; 3]) -> [f64; 3] {
    use pm_reference::color::{Primaries as P, apply, invert, mul};
    apply(&mul(&invert(&P::BT709.to_xyz()), &gamut.to_xyz()), light)
}

#[test]
fn camera_log_reaches_the_working_space_as_the_vendor_transform_says() {
    use pm_reference::camera_log::Curve;
    use pm_reference::color::Primaries as P;
    let gpu = pm_gpu::Gpu::new("camera log test").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let pixels: Vec<[f32; 4]> = (0..24)
        .map(|i| {
            let v = i as f32 / 20.0;
            [v, (0.2 + 0.7 * v).min(1.1), 0.9 - 0.5 * v, 1.0]
        })
        .collect();
    for (transfer, curve, primaries, gamut) in [
        (
            Transfer::AppleLog,
            Curve::AppleLog,
            Primaries::Bt2020,
            P::BT2020,
        ),
        (
            Transfer::SLog3,
            Curve::SLog3,
            Primaries::SGamut3Cine,
            P::S_GAMUT3_CINE,
        ),
        (
            Transfer::SLog3,
            Curve::SLog3,
            Primaries::SGamut3,
            P::S_GAMUT3,
        ),
        (Transfer::VLog, Curve::VLog, Primaries::VGamut, P::V_GAMUT),
        (Transfer::LogC3, Curve::LogC3, Primaries::AwgGen3, P::AWG3),
        (Transfer::LogC4, Curve::LogC4, Primaries::AwgGen4, P::AWG4),
    ] {
        let color = SourceColor {
            unorm8: false,
            transfer,
            primaries,
            matrix: MatrixCoefficients::Identity,
            siting: ChromaSiting::Left,
            alpha: AlphaMode::Opaque,
            tone_map: None,
            chromaticities: None,
            power: None,
        };
        let target = source_read(
            &gpu,
            &pipelines,
            &pixels,
            &color,
            WorkingEncoding::LinearLight,
            Precision::Float32,
            wgpu::TextureFormat::Rgba32Float,
        );
        let actual = read_f32(&gpu.device, &gpu.queue, &target.texture).unwrap();
        for (i, p) in pixels.iter().enumerate() {
            let expected = to_rec709(
                &gamut,
                [p[0], p[1], p[2]].map(|v| curve.to_linear(f64::from(v))),
            );
            for c in 0..3 {
                let a = f64::from(actual[i * 4 + c]);
                assert!(
                    (a - expected[c]).abs() <= 4e-6 + 4e-6 * expected[c].abs(),
                    "{transfer:?} {primaries:?} pixel {i}: {a} {}",
                    expected[c]
                );
            }
        }
    }
}

#[test]
fn hdr_projects_keep_bt2100_light_and_sdr_projects_tone_map_it() {
    use pm_reference::color::{Primaries as P, Transfer as T};
    let gpu = pm_gpu::Gpu::new("hdr light test").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let pixels: Vec<[f32; 4]> = (0..21)
        .map(|i| {
            let v = i as f32 / 20.0;
            [v, v * 0.8, v * 0.6, 1.0]
        })
        .collect();
    let mut color = SourceColor {
        unorm8: false,
        transfer: Transfer::Pq,
        primaries: Primaries::Bt2020,
        matrix: MatrixCoefficients::Identity,
        siting: ChromaSiting::Left,
        alpha: AlphaMode::Opaque,
        tone_map: None,
        chromaticities: None,
        power: None,
    };
    let read = |color: &SourceColor, pixels: &[[f32; 4]]| {
        let target = source_read(
            &gpu,
            &pipelines,
            pixels,
            color,
            WorkingEncoding::LinearLight,
            Precision::Float32,
            wgpu::TextureFormat::Rgba32Float,
        );
        read_f32(&gpu.device, &gpu.queue, &target.texture).unwrap()
    };
    let kept = read(&color, &pixels);
    for (i, p) in pixels.iter().enumerate() {
        let nits = [p[0], p[1], p[2]].map(|v| T::Pq.to_linear(f64::from(v)) * 10000.0 / 203.0);
        let expected = to_rec709(&P::BT2020, nits);
        for c in 0..3 {
            let a = f64::from(kept[i * 4 + c]);
            assert!(
                (a - expected[c]).abs() <= 1e-5 + 1e-4 * expected[c].abs(),
                "pixel {i}: {a} {}",
                expected[c]
            );
        }
    }
    assert!(kept.iter().step_by(4).copied().fold(0.0, f32::max) > 40.0);
    color.tone_map = Some(ToneMap {
        transfer: Transfer::Pq,
        peak_nits: 1000.0,
        method: 1,
    });
    let neutral: Vec<[f32; 4]> = (0..21)
        .map(|i| {
            let v = i as f32 / 20.0;
            [v, v, v, 1.0]
        })
        .collect();
    let mapped = read(&color, &neutral);
    assert!(mapped.iter().step_by(4).copied().fold(0.0, f32::max) <= 1.0001);
}

#[test]
fn native_source_storage_preserves_negative_and_superwhite_values() {
    let gpu = pm_gpu::Gpu::new("native source storage test").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let pixels = [[-0.25, 1.25, 0.3333, 1.0], [1.5, -0.125, 0.7, 1.0]];
    for precision in [Precision::Float16, Precision::Float32] {
        let target = source(
            &gpu,
            &pipelines,
            &pixels,
            precision,
            working_format(precision),
        );
        let actual = read_f32(&gpu.device, &gpu.queue, &target.texture).unwrap();
        let expected: Vec<f32> = pixels
            .iter()
            .flatten()
            .map(|&v| match precision {
                Precision::Unorm8 => unreachable!("floating source test"),
                Precision::Float16 => half_to_f32(f32_to_half(v)),
                Precision::Float32 => v,
            })
            .collect();
        assert_eq!(actual, expected, "{precision:?}");
        assert!(actual[0] < 0.0 && actual[1] > 1.0);
    }
}

#[test]
fn video_storage_follows_project_and_source_depth_with_one_reading() {
    let gpu = pm_gpu::Gpu::new("video source storage test").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let mut media = MediaCache::new(&gpu.device);
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video");
    let eight = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/h264-aac.mp4");
    let ten = fixtures.join("hevc10-709-limited.mp4");
    let hdr = fixtures.join("hevc10-2020-pq.mp4");
    use wgpu::TextureFormat::{Rgba8Unorm, Rgba16Float, Rgba32Float};
    let convert = |media: &mut MediaCache,
                   path: &Path,
                   precision,
                   interpretation: Option<&Interpretation>| {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let target = media
            .video(
                &gpu.device,
                &gpu.queue,
                &pipelines,
                &mut encoder,
                path,
                1,
                0.2,
                true,
                WorkingEncoding::Standard,
                precision,
                interpretation,
                pm_time::Rate::FPS_30,
                false,
            )
            .unwrap()
            .unwrap();
        gpu.queue.submit([encoder.finish()]);
        let pixels = read_f32(&gpu.device, &gpu.queue, &target.texture).unwrap();
        media.flush();
        (target.format(), pixels)
    };
    // Never below 8 bits; half float or better for 10-bit and HDR sources.
    for (path, precision, format) in [
        (&eight, Precision::Unorm8, Rgba8Unorm),
        (&eight, Precision::Float16, Rgba16Float),
        (&eight, Precision::Float32, Rgba32Float),
        (&ten, Precision::Unorm8, Rgba16Float),
        (&ten, Precision::Float32, Rgba32Float),
        (&hdr, Precision::Unorm8, Rgba16Float),
    ] {
        assert_eq!(
            convert(&mut media, path, precision, None).0,
            format,
            "{path:?} {precision:?}"
        );
    }
    // Frame-placing overrides and an override restating the file's reading
    // leave an SDR frame's colour as it is.
    let (_, base) = convert(&mut media, &eight, Precision::Float32, None);
    let placed = Interpretation {
        pixel_aspect: Some((1, 1)),
        clean_aperture: Some(false),
        field_order: pm_media::interlace::FieldOverride::Progressive,
        ..Default::default()
    };
    let same = Interpretation {
        transfer: Some(Transfer::DisplayE),
        ..Default::default()
    };
    let colour = |media: &mut MediaCache, i| convert(media, &eight, Precision::Float32, i).1;
    assert_eq!(colour(&mut media, Some(&placed)), base);
    assert_eq!(colour(&mut media, Some(&same)), base);
    // The host's per-file override restating the reading too.
    let reading = media.video_color(&eight).unwrap();
    media.interpret(&eight, Some(reading));
    assert_eq!(colour(&mut media, None), base);
    media.interpret(&eight, None);
}

#[test]
fn rgba8_source_storage_rounds_the_f32_conversion_once() {
    let gpu = pm_gpu::Gpu::new("rgba8 source storage test").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    // Probe both sides of every half-byte boundary, far enough away
    // to avoid a tie but close enough that f16 can cross the boundary.
    let pixels: Vec<[f32; 4]> = (0..255)
        .map(|code| {
            let boundary = (code as f32 + 0.5) / 255.0;
            [
                boundary - 1.0 / 65536.0,
                boundary + 1.0 / 65536.0,
                -0.25,
                1.0,
            ]
        })
        .chain([[1.25, -0.25, 0.25, 1.0]])
        .collect();
    let unorm = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let expected: Vec<u8> = pixels.iter().flatten().map(|&v| unorm(v)).collect();
    let rounded_via_half: Vec<u8> = pixels
        .iter()
        .flatten()
        .map(|&v| unorm(half_to_f32(f32_to_half(v))))
        .collect();
    assert_ne!(
        expected, rounded_via_half,
        "fixture must detect double rounding"
    );
    let target = source(
        &gpu,
        &pipelines,
        &pixels,
        Precision::Unorm8,
        wgpu::TextureFormat::Rgba8Unorm,
    );
    let actual = read_raw(&gpu.device, &gpu.queue, &target.texture).unwrap();
    assert_eq!(actual, expected, "source byte quantization");
    assert_eq!(&actual[actual.len() - 4..], &[255, 0, 64, 255]);
}

#[test]
fn chroma_follows_its_siting_with_section_9_filters_at_every_storage_depth() {
    let gpu = pm_gpu::Gpu::new("source chroma reconstruction test").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let (width, height) = (8u32, 6u32);
    let chroma = [
        [-0.375f32, 0.25],
        [-0.125, -0.375],
        [0.375, 0.125],
        [0.125, 0.375],
        [0.25, -0.375],
        [-0.25, 0.25],
        [0.125, 0.375],
        [-0.375, -0.125],
        [-0.125, 0.375],
        [0.375, 0.125],
        [-0.25, -0.375],
        [0.25, -0.25],
    ];
    // These already-normalized planes isolate upsampling from the decoder
    // and limited-range normalization. The irregular ramps and sharp sign
    // changes distinguish the filter and siting independently.
    let luma: Vec<f32> = (0..width * height)
        .flat_map(|_| [0.421875, 0.0, 0.0, 1.0])
        .collect();
    let uv: Vec<f32> = chroma.iter().flatten().copied().collect();
    let upload = |label, w, h, format, channels, values: &[f32]| {
        let target = Target::new(
            &gpu.device,
            label,
            w,
            h,
            format,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        gpu.queue.write_texture(
            target.texture.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * channels * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        target.texture
    };
    let normalized = Normalized::YCbCr {
        luma: upload(
            "test luma",
            width,
            height,
            wgpu::TextureFormat::Rgba32Float,
            4,
            &luma,
        ),
        chroma: upload(
            "test chroma",
            width / 2,
            height / 2,
            wgpu::TextureFormat::Rg32Float,
            2,
            &uv,
        ),
    };
    let color = SourceColor {
        unorm8: false,
        transfer: Transfer::DisplayE,
        primaries: Primaries::Bt709,
        matrix: MatrixCoefficients::Bt601,
        siting: ChromaSiting::Left,
        alpha: AlphaMode::Opaque,
        tone_map: None,
        chromaticities: None,
        power: None,
    };
    let converted = |format| {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let target = convert_format(
            &gpu.device,
            &gpu.queue,
            &pipelines,
            &mut encoder,
            &normalized,
            &color,
            WorkingEncoding::Standard,
            if format == wgpu::TextureFormat::Rgba32Float {
                Precision::Float32
            } else {
                Precision::Unorm8
            },
            Factors { x: 2, y: 2 },
            None,
            format,
            false,
            None,
        );
        gpu.queue.submit([encoder.finish()]);
        target
    };
    // Independent CPU reference, in f64. Pixel indices address texel centers:
    // centered 4:2:0 places chroma at (0.5, 0.5) on the luma index grid;
    // native left siting places it at (0, 0.5). Clamp each tapped texel.
    let at = |x: i32, y: i32, channel: usize| {
        f64::from(chroma[(y.clamp(0, 2) * 4 + x.clamp(0, 3)) as usize][channel])
    };
    let interpolate = |x: u32, y: u32, centered: bool, channel: usize| {
        let cx = (f64::from(x) - if centered { 0.5 } else { 0.0 }) / 2.0;
        let cy = (f64::from(y) - 0.5) / 2.0;
        let (ix, iy) = (cx.floor() as i32, cy.floor() as i32);
        let cubic = |distance: f64| {
            let d = distance.abs();
            if d < 1.0 {
                1.5 * d * d * d - 2.5 * d * d + 1.0
            } else if d < 2.0 {
                -0.5 * d * d * d + 2.5 * d * d - 4.0 * d + 2.0
            } else {
                0.0
            }
        };
        let mut value = 0.0;
        for j in iy..=iy + 1 {
            let wy = (1.0 - (cy - f64::from(j)).abs()).max(0.0);
            for i in ix - 1..=ix + 2 {
                let wx = if centered {
                    (1.0 - (cx - f64::from(i)).abs()).max(0.0)
                } else {
                    cubic(cx - f64::from(i))
                };
                value += at(i, j, channel) * wx * wy;
            }
        }
        value
    };
    let reference = |centered| {
        let mut pixels = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let cb = interpolate(x, y, centered, 0);
                let cr = interpolate(x, y, centered, 1);
                let y = 0.421875;
                // BT.601's Kr/Kb definition, independently of Matrix's
                // packed f32 columns used by the production shader.
                let r = y + 2.0 * (1.0 - 0.299) * cr;
                let b = y + 2.0 * (1.0 - 0.114) * cb;
                let g = (y - 0.299 * r - 0.114 * b) / (1.0 - 0.299 - 0.114);
                pixels.extend([r, g, b, 1.0]);
            }
        }
        pixels
    };
    let unorm = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let native = reference(false);
    // Today's viewer reconstructs chroma centered and bilinear: a half
    // chroma sample off for left-sited 4:2:0, far from the reference here.
    let centered: Vec<u8> = reference(true).iter().map(|&v| unorm(v)).collect();
    let expected: Vec<u8> = native.iter().map(|&v| unorm(v)).collect();
    assert!(
        expected
            .iter()
            .zip(&centered)
            .any(|(&a, &b)| a.abs_diff(b) > 20)
    );
    let working8 = converted(wgpu::TextureFormat::Rgba8Unorm);
    assert_eq!(
        read_raw(&gpu.device, &gpu.queue, &working8.texture).unwrap(),
        expected,
        "RGBA8 storage keeps the siting and filter"
    );
    let float = converted(wgpu::TextureFormat::Rgba32Float);
    let actual = read_f32(&gpu.device, &gpu.queue, &float.texture).unwrap();
    for (i, (&actual, &expected)) in actual.iter().zip(&native).enumerate() {
        assert!(
            (f64::from(actual) - expected).abs() < 2e-6,
            "channel {i}: {actual} != {expected}"
        );
    }
    assert!(actual.iter().any(|&v| v < 0.0));
    assert!(actual.iter().any(|&v| v > 1.0));
}

#[test]
fn packed_still_fusion_matches_separate_normalization_byte_for_byte() {
    use pm_media::frame::{PixelFormat, PlaneDesc, PlaneFormat};
    use pm_protocol::color::{Range, SampleAlignment};
    let gpu = pm_gpu::Gpu::new("packed source fusion").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let importer = DecodeImporter::new(&gpu.device).unwrap();
    let color = SourceColor {
        unorm8: false,
        transfer: Transfer::DisplayE,
        primaries: Primaries::Bt709,
        matrix: MatrixCoefficients::Identity,
        siting: ChromaSiting::Left,
        alpha: AlphaMode::Straight,
        tone_map: None,
        chromaticities: None,
        power: None,
    };
    for (plane, bits, format, bpp) in [
        (PlaneFormat::Rgba8, 8, wgpu::TextureFormat::Rgba8Unorm, 4),
        (PlaneFormat::Rgba16, 16, wgpu::TextureFormat::Rgba16Unorm, 8),
        (
            PlaneFormat::Rgba16Float,
            16,
            wgpu::TextureFormat::Rgba16Float,
            8,
        ),
        (
            PlaneFormat::Rgba32Float,
            32,
            wgpu::TextureFormat::Rgba32Float,
            16,
        ),
    ] {
        let mut bytes = Vec::new();
        for i in 0..256u32 {
            for code in [i, 255 - i, i * 73 % 256, i * 37 % 256] {
                match plane {
                    PlaneFormat::Rgba8 => bytes.push(code as u8),
                    PlaneFormat::Rgba16 => {
                        bytes.extend_from_slice(&((code * 257) as u16).to_le_bytes())
                    }
                    PlaneFormat::Rgba16Float => {
                        bytes.extend_from_slice(&f32_to_half(code as f32 / 255.0).to_le_bytes())
                    }
                    PlaneFormat::Rgba32Float => {
                        bytes.extend_from_slice(&(code as f32 / 255.0).to_le_bytes())
                    }
                    _ => unreachable!(),
                }
            }
        }
        let raw = Target::new(
            &gpu.device,
            "packed test source",
            256,
            1,
            format,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        gpu.queue.write_texture(
            raw.texture.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256 * bpp),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 256,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let pixel_format = Arc::new(PixelFormat {
            layout: pm_media::frame::Layout::Rgba,
            bits,
            alignment: SampleAlignment::Lsb,
            range: Range::Full,
            log2_chroma_w: 0,
            log2_chroma_h: 0,
            alpha_bits: bits,
            planes: vec![PlaneDesc {
                format: plane,
                width: 256,
                height: 1,
            }],
            width: 256,
            height: 1,
            label: "synthetic packed source".into(),
            rgba_premultiplied: false,
        });
        let imported = pm_gpu::decode_in::ImportedFrame {
            textures: vec![raw.texture.clone()],
            format: pixel_format.clone(),
            aliased: false,
            desc: None,
        };
        for working in [WorkingEncoding::Standard, WorkingEncoding::LinearLight] {
            for precision in [Precision::Unorm8, Precision::Float16, Precision::Float32] {
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                let normalized = importer
                    .normalize(&gpu.queue, &mut encoder, &imported)
                    .unwrap();
                let direct = Normalized::Rgb {
                    rgba: raw.texture.clone(),
                };
                let mut convert = |input: &Normalized, packed| {
                    convert_format(
                        &gpu.device,
                        &gpu.queue,
                        &pipelines,
                        &mut encoder,
                        input,
                        &color,
                        working,
                        precision,
                        Factors { x: 1, y: 1 },
                        None,
                        working_format(precision),
                        false,
                        packed,
                    )
                };
                let expected = convert(&normalized, None);
                let actual = convert(&direct, Some(pixel_format.as_ref()));
                gpu.queue.submit([encoder.finish()]);
                assert_eq!(
                    read_raw(&gpu.device, &gpu.queue, &actual.texture).unwrap(),
                    read_raw(&gpu.device, &gpu.queue, &expected.texture).unwrap(),
                    "{plane:?} {working:?} {precision:?}"
                );
            }
        }
    }
}

/// `n` distinct 256x256 PNG stills in a directory of their own.
fn distinct_stills(n: u8) -> (tempfile::TempDir, Vec<PathBuf>) {
    let dir = tempfile::tempdir().unwrap();
    let paths = (0..n)
        .map(|i| {
            let path = dir.path().join(format!("still-{i}.png"));
            let rgba: Vec<u8> = (0..256u32 * 256)
                .flat_map(|p| [i.wrapping_mul(37), (p % 256) as u8, (p / 256) as u8, 255])
                .collect();
            let mut encoder = png::Encoder::new(std::fs::File::create(&path).unwrap(), 256, 256);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&rgba)
                .unwrap();
            path
        })
        .collect();
    (dir, paths)
}

/// One frame of the media cache reading `paths`: the stills it decoded.
fn image_frame(
    gpu: &pm_gpu::Gpu,
    pipelines: &Pipelines,
    media: &mut MediaCache,
    paths: &[&PathBuf],
) -> u64 {
    media.begin_frame(false);
    let before = pm_perf::snapshot();
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    for path in paths {
        media
            .image(
                &gpu.device,
                &gpu.queue,
                pipelines,
                &mut encoder,
                path,
                WorkingEncoding::Standard,
                Precision::Float16,
                None,
            )
            .unwrap();
    }
    gpu.queue.submit([encoder.finish()]);
    media.flush();
    pm_perf::snapshot()
        .since(&before)
        .get(pm_perf::Counter::StillDecodes)
}

/// Over a small budget, the stills kept are the frame being drawn's own:
/// the frame before gives way (two adjacent frames that read different
/// stills do not keep both sets), a frame that reads the same stills again
/// decodes nothing, and a frame whose own stills exceed the budget says so.
#[test]
fn over_budget_only_the_frame_being_drawn_keeps_its_stills() {
    let gpu = pm_gpu::Gpu::new("image floor").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let (_dir, s) = distinct_stills(4);
    let mut media = MediaCache::new(&gpu.device);
    media.trim_images(1);
    assert!(image_frame(&gpu, &pipelines, &mut media, &[&s[0], &s[1]]) > 0);
    let one = media.image_bytes() / 2;
    assert!(one > 1);
    assert_eq!(media.image_over_budget(), Some((2 * one, 1)));
    assert!(image_frame(&gpu, &pipelines, &mut media, &[&s[2], &s[3]]) > 0);
    assert_eq!(
        media.image_bytes(),
        2 * one,
        "the frame before kept its stills beside this frame's"
    );
    assert_eq!(
        image_frame(&gpu, &pipelines, &mut media, &[&s[2], &s[3]]),
        0,
        "the next frame reads them again without a decode"
    );
    // Within the budget nothing is said, and the frame before stays.
    media.trim_images(64 * one);
    assert!(image_frame(&gpu, &pipelines, &mut media, &[&s[0]]) > 0);
    assert_eq!(media.image_over_budget(), None);
    assert!(image_frame(&gpu, &pipelines, &mut media, &[&s[1]]) > 0);
    assert_eq!(
        image_frame(&gpu, &pipelines, &mut media, &[&s[0], &s[1]]),
        0
    );
}

/// Under critical memory pressure a frame's stills, its own too, go down to
/// the budget once its commands are submitted.
#[test]
fn critical_pressure_lets_go_of_stills_after_the_frame() {
    let gpu = pm_gpu::Gpu::new("image pressure").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let (_dir, s) = distinct_stills(2);
    let mut media = MediaCache::new(&gpu.device);
    media.trim_images(1);
    media.set_image_pressure(true);
    assert!(image_frame(&gpu, &pipelines, &mut media, &[&s[0], &s[1]]) > 0);
    assert_eq!(media.image_bytes(), 0);
    assert!(
        media.image_over_budget().is_some(),
        "still an over-budget frame"
    );
    media.set_image_pressure(false);
    assert!(image_frame(&gpu, &pipelines, &mut media, &[&s[0]]) > 0);
    assert!(media.image_bytes() > 0, "kept while frames draw it");
}

/// What the still prefetch holds is counted and goes with a budget of zero
/// and with an idle release, like the converted stills.
#[test]
fn the_still_prefetch_hold_goes_at_budget_zero_and_at_rest() {
    let gpu = pm_gpu::Gpu::new("prefetch hold").unwrap();
    let (_dir, s) = distinct_stills(2);
    let mut media = MediaCache::new(&gpu.device);
    let hold = |media: &mut MediaCache| {
        for path in &s {
            media.prepare_image(path, None, false);
        }
        media.wait_for_stills();
        assert!(media.still_prefetch_bytes() > 0, "the hold is counted");
    };
    hold(&mut media);
    media.trim_images(0);
    assert_eq!(media.still_prefetch_bytes(), 0, "budget zero");
    media.trim_images(48 << 20);
    hold(&mut media);
    media.release_idle_frames();
    assert_eq!(media.still_prefetch_bytes(), 0, "at rest");
}

/// A frame draws its newest still first (the top layer): caching it must
/// not push out the frame before's stills this frame reads next. Over the
/// budget they stay until the frame is submitted, and only those it did not
/// read again go then.
#[test]
fn a_new_still_never_pushes_out_stills_the_frame_reads_next() {
    let gpu = pm_gpu::Gpu::new("image floor order").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let (_dir, s) = distinct_stills(4);
    let mut media = MediaCache::new(&gpu.device);
    media.trim_images(1);
    assert!(image_frame(&gpu, &pipelines, &mut media, &[&s[0], &s[1]]) > 0);
    let one = media.image_bytes() / 2;
    let before = pm_perf::snapshot();
    image_frame(&gpu, &pipelines, &mut media, &[&s[2], &s[1], &s[0]]);
    let decoded = pm_perf::snapshot()
        .since(&before)
        .get(pm_perf::Counter::MediaCacheMisses);
    assert_eq!(decoded, 1, "only the new still is decoded");
    assert_eq!(media.image_bytes(), 3 * one);
    // The next frame reads one new still: the others go once it is submitted.
    image_frame(&gpu, &pipelines, &mut media, &[&s[3]]);
    assert_eq!(media.image_bytes(), one);
}

#[test]
fn legacy_video_uploads_clip_and_round_to_rgba8_before_float_effects() {
    let gpu = pm_gpu::Gpu::new("legacy RGBA8 upload").unwrap();
    let pipelines = Pipelines::new(&gpu.device);
    let mut color = SourceColor {
        unorm8: true,
        transfer: Transfer::DisplayE,
        primaries: Primaries::Bt709,
        matrix: MatrixCoefficients::Identity,
        siting: ChromaSiting::Left,
        alpha: AlphaMode::Opaque,
        tone_map: None,
        chromaticities: None,
        power: None,
    };
    let pixels = [[0.1234, 0.503, 1.1, 1.], [-0.1, 0.25, 0.75, 1.]];
    let legacy = source_read(
        &gpu,
        &pipelines,
        &pixels,
        &color,
        WorkingEncoding::Standard,
        Precision::Float32,
        wgpu::TextureFormat::Rgba32Float,
    );
    let values = read_f32(&gpu.device, &gpu.queue, &legacy.texture).unwrap();
    for (pixel, expected) in values.chunks_exact(4).zip(pixels) {
        for i in 0..3 {
            assert!(
                (pixel[i] - (expected[i].clamp(0., 1.) * 255. + 0.5).floor() / 255.).abs() < 1e-6
            );
        }
    }
    color.unorm8 = false;
    let native = source_read(
        &gpu,
        &pipelines,
        &pixels,
        &color,
        WorkingEncoding::Standard,
        Precision::Float32,
        wgpu::TextureFormat::Rgba32Float,
    );
    let values = read_f32(&gpu.device, &gpu.queue, &native.texture).unwrap();
    for (pixel, expected) in values.chunks_exact(4).zip(pixels) {
        for i in 0..3 {
            assert!((pixel[i] - expected[i]).abs() < 1e-6);
        }
    }
}
