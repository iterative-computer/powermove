//! GPU regressions for source storage, chroma reconstruction and cache
//! isolation.

use super::*;
use pm_gpu::readback::{f32_to_half, half_to_f32, read_f32, read_raw};

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
    let color = SourceColor {
        transfer: Transfer::DisplayE,
        primaries: Primaries::Bt709,
        matrix: MatrixCoefficients::Identity,
        siting: ChromaSiting::Left,
        alpha: AlphaMode::Opaque,
        tone_map: None,
        chromaticities: None,
        power: None,
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
        &color,
        WorkingEncoding::Standard,
        precision,
        Factors { x: 1, y: 1 },
        None,
        format,
    );
    gpu.queue.submit([encoder.finish()]);
    target
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
    let eight = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../apps/desktop/e2e/fixtures/h264-aac.mp4");
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
