//! SVG filter contracts against independently evaluated f64 Gaussian weights.
//! The source is authored as pixel-aligned rectangles, so pm-reference's
//! Gaussian integrates each constant source square exactly. No RGBA8 image
//! participates in the expected result.

use pm_protocol::color::Precision;
use pm_protocol::common::{Size, Ticks};
use pm_reference::compare::{Bounds, Class, compare, geometry, same_geometry};
use pm_reference::coverage::{FillRule, fill};
use pm_reference::gaussian::{Edge, blur, kernel};
use pm_reference::stroke::outline as reference_stroke;
use pm_render::vector::svg_float::SvgPainter;
use pm_render::{MapResolver, OutputKind, RenderRequest, Renderer};
use pm_vector::kurbo::{Affine, BezPath, Cap, Circle, Point, RoundedRect, Shape, Stroke};
use pm_vector::render::scene::PixelRect;
use pm_vector::render::svg::SvgDocument;
use serde::Deserialize;
use sha2::{Digest, Sha256};

fn render(svg: &str, width: u32, height: u32, transform: Affine) -> Vec<f32> {
    let gpu = pm_gpu::Gpu::new("SVG filter reference").unwrap();
    let doc = SvgDocument::parse(svg.as_bytes()).unwrap();
    assert!(
        doc.float_scene(transform, &[]).is_some(),
        "filtered SVG must take the float path"
    );
    let mut painter = SvgPainter::new(&gpu.device).unwrap();
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let (texture, _) = painter
        .render(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &doc,
            transform,
            PixelRect::new(0, 0, width, height),
            &[],
            Precision::Float32,
        )
        .unwrap()
        .expect("filter must render");
    gpu.queue.submit([encoder.finish()]);
    pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &texture).unwrap()
}

fn pixel(frame: &[f32], width: usize, x: usize, y: usize) -> [f64; 4] {
    std::array::from_fn(|c| f64::from(frame[(y * width + x) * 4 + c]))
}

fn weight(sigma: f64, destination: i32, source: i32) -> f64 {
    let weights = kernel(sigma);
    let center = weights.len() / 2;
    let index = center as i32 + destination - source;
    usize::try_from(index)
        .ok()
        .and_then(|i| weights.get(i))
        .copied()
        .unwrap_or(0.0)
}

fn box_blur(x: i32, y: i32, rect: (i32, i32, i32, i32), sigma: (f64, f64)) -> f64 {
    let wx: f64 = (rect.0..rect.2).map(|s| weight(sigma.0, x, s)).sum();
    let wy: f64 = (rect.1..rect.3).map(|s| weight(sigma.1, y, s)).sum();
    wx * wy
}

fn close(actual: f64, expected: f64, cv: f64, label: &str) {
    assert!(
        (actual - expected).abs() * 255.0 <= cv,
        "{label}: {actual} vs f64 {expected} ({} CV)",
        (actual - expected).abs() * 255.0
    );
}

const ANISOTROPIC_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><defs><filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="64" height="64"><feGaussianBlur stdDeviation="2 4"/></filter></defs><rect x="31" y="31" width="1" height="1" fill="#fff" filter="url(#f)"/></svg>"##;
const COLOR_SPACES_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="24"><defs><filter id="linear" filterUnits="userSpaceOnUse" x="0" y="0" width="64" height="12"><feGaussianBlur stdDeviation="2"/></filter><filter id="srgb" filterUnits="userSpaceOnUse" x="0" y="12" width="64" height="12" color-interpolation-filters="sRGB"><feGaussianBlur stdDeviation="2"/></filter></defs><g filter="url(#linear)"><rect x="8" y="2" width="16" height="8" fill="#000"/><rect x="24" y="2" width="16" height="8" fill="#fff"/></g><g filter="url(#srgb)"><rect x="8" y="14" width="16" height="8" fill="#000"/><rect x="24" y="14" width="16" height="8" fill="#fff"/></g></svg>"##;
const REGION_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="24"><defs><filter id="f" filterUnits="userSpaceOnUse" x="14" y="4" width="8" height="16"><feGaussianBlur stdDeviation="2"/></filter></defs><rect x="10" y="10" width="1" height="1" fill="#fff" filter="url(#f)"/></svg>"##;
const ZERO_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><defs><filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="16" height="16"><feGaussianBlur stdDeviation="0"/></filter></defs><rect x="3" y="4" width="8" height="6" fill="#38a7e5" filter="url(#f)"/></svg>"##;

#[test]
fn anisotropic_gaussian_has_independent_f64_axis_weights() {
    let svg = ANISOTROPIC_SVG;
    let frame = render(svg, 64, 64, Affine::IDENTITY);
    for (x, y) in [(31, 31), (32, 31), (31, 34), (37, 31), (31, 42)] {
        let expected = box_blur(x, y, (31, 31, 32, 32), (2.0, 4.0));
        let p = pixel(&frame, 64, x as usize, y as usize);
        for (channel, actual) in p.iter().enumerate() {
            close(
                *actual,
                expected,
                0.45,
                &format!("{x},{y} channel {channel}"),
            );
        }
    }
}

#[test]
fn rotated_nonuniform_and_skewed_gaussian_follow_f64_covariance() {
    // A one-unit square blurred in SVG user space has covariance
    // diag(sigma_x² + 1/12, sigma_y² + 1/12). An affine image placement
    // transforms that by A Σ Aᵀ. The 1/12 is the square's own variance.
    for matrix in [
        Affine::rotate(27.0f64.to_radians()) * Affine::scale_non_uniform(1.4, 0.75),
        Affine::new([1.2, 0.25, 0.45, 0.9, 0.0, 0.0]),
    ] {
        let transform =
            Affine::translate((48.0, 48.0)) * matrix * Affine::translate((-31.5, -31.5));
        let frame = render(ANISOTROPIC_SVG, 96, 96, transform);
        let mut area = 0.0;
        let mut mx = 0.0;
        let mut my = 0.0;
        for y in 0..96 {
            for x in 0..96 {
                let a = pixel(&frame, 96, x, y)[3];
                area += a;
                mx += a * (x as f64 + 0.5);
                my += a * (y as f64 + 0.5);
            }
        }
        mx /= area;
        my /= area;
        let (mut xx, mut xy, mut yy) = (0.0, 0.0, 0.0);
        for y in 0..96 {
            for x in 0..96 {
                let a = pixel(&frame, 96, x, y)[3];
                let (dx, dy) = (x as f64 + 0.5 - mx, y as f64 + 0.5 - my);
                xx += a * dx * dx;
                xy += a * dx * dy;
                yy += a * dy * dy;
            }
        }
        let [a, b, c, d, _, _] = matrix.as_coeffs();
        let (sx, sy) = (4.0 + 1.0 / 12.0, 16.0 + 1.0 / 12.0);
        let expected = [
            a * a * sx + c * c * sy,
            a * b * sx + c * d * sy,
            b * b * sx + d * d * sy,
        ];
        let actual = [xx / area, xy / area, yy / area];
        for k in 0..3 {
            assert!(
                (actual[k] - expected[k]).abs() < 0.18,
                "matrix {matrix:?}, covariance {actual:?} vs f64 {expected:?}"
            );
        }
        let determinant = (a * d - b * c).abs();
        assert!(
            (area - determinant).abs() < 0.02,
            "matrix {matrix:?}: area {area} vs {determinant}"
        );
        assert!(
            (mx - 48.0).abs() < 0.02 && (my - 48.0).abs() < 0.02,
            "matrix {matrix:?}: centroid {mx},{my}"
        );
    }
}

#[test]
fn default_linear_rgb_and_explicit_srgb_blur_have_distinct_f64_colors() {
    let svg = COLOR_SPACES_SVG;
    let frame = render(svg, 64, 24, Affine::IDENTITY);
    // A white source pixel contributes in linear light. At the interface,
    // sRGB is the linear-light result encoded through IEC 61966-2-1.
    let x = 23;
    let white: f64 = (24..40).map(|s| weight(2.0, x, s)).sum();
    let alpha_x: f64 = (8..40).map(|s| weight(2.0, x, s)).sum();
    let alpha_y: f64 = (2..10).map(|s| weight(2.0, 5, s)).sum();
    let alpha = alpha_x * alpha_y;
    let premul_linear = white * alpha_y;
    let straight_linear = premul_linear / alpha;
    let encoded = if straight_linear <= 0.0031308 {
        12.92 * straight_linear
    } else {
        1.055 * straight_linear.powf(1.0 / 2.4) - 0.055
    };
    let linear = pixel(&frame, 64, x as usize, 5);
    let srgb = pixel(&frame, 64, x as usize, 17);
    close(linear[0], encoded * alpha, 0.7, "linearRGB default");
    close(srgb[0], premul_linear, 0.7, "sRGB override");
    close(linear[3], alpha, 0.7, "linear alpha");
    close(srgb[3], alpha, 0.7, "sRGB alpha");
    assert!(linear[0] > srgb[0] + 0.05, "{linear:?} vs {srgb:?}");
}

#[test]
fn output_region_clips_but_source_halo_contributes() {
    let svg = REGION_SVG;
    let frame = render(svg, 32, 24, Affine::IDENTITY);
    close(
        pixel(&frame, 32, 13, 10)[3],
        0.0,
        0.02,
        "outside hard filter region",
    );
    let expected = box_blur(14, 10, (10, 10, 11, 11), (2.0, 2.0));
    close(
        pixel(&frame, 32, 14, 10)[3],
        expected,
        0.45,
        "source outside region supplies halo",
    );
    assert!(expected > 0.0);
}

#[test]
fn filter_clip_and_mask_integrate_on_the_same_subpixel_grid() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><defs><filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="32" height="32"><feGaussianBlur stdDeviation="1.5"/></filter><clipPath id="c"><rect x="0" y="0" width="12.5" height="32"/></clipPath><mask id="m" maskUnits="userSpaceOnUse" x="0" y="0" width="32" height="32" mask-type="alpha"><rect x="0" y="10.5" width="32" height="21.5" fill="#fff"/></mask></defs><g filter="url(#f)" clip-path="url(#c)" mask="url(#m)"><rect x="8" y="8" width="8" height="8" fill="#fff"/></g></svg>"##;
    let frame = render(svg, 32, 32, Affine::IDENTITY);
    let actual = pixel(&frame, 32, 12, 10);
    // The exact SVG Gaussian at a point is the product of two normal-CDF
    // differences. Only the lower-left quarter of this pixel survives both
    // half-pixel clip and mask boundaries. Integrate in f64 at 64×64 here.
    let mut expected = 0.0;
    const N: usize = 64;
    for y in 0..N / 2 {
        for x in 0..N / 2 {
            let px = 12.0 + (x as f64 + 0.5) / N as f64;
            let py = 10.5 + (y as f64 + 0.5) / N as f64;
            expected += gaussian_interval(px, 8.0, 16.0, 1.5)
                * gaussian_interval(py, 8.0, 16.0, 1.5)
                / (N * N) as f64;
        }
    }
    // x is restricted to [12,12.5), y to [10.5,11).
    for (channel, observed) in actual.iter().enumerate() {
        close(
            *observed,
            expected,
            0.8,
            &format!("joint filter/clip/mask channel {channel}"),
        );
    }
    close(pixel(&frame, 32, 13, 10)[3], 0.0, 0.02, "outside clip");
    close(pixel(&frame, 32, 12, 9)[3], 0.0, 0.02, "outside mask");
}

#[test]
fn filter_res_uses_an_authored_grid_and_zero_grid_draws_nothing() {
    let document = |resolution: &str, sigma: f64| {
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><defs><filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="64" height="64" {resolution}><feGaussianBlur stdDeviation="{sigma}"/></filter></defs><rect x="24" y="24" width="16" height="16" fill="#fff" filter="url(#f)"/></svg>"##
        )
    };
    let ordinary = render(&document("", 0.0), 64, 64, Affine::IDENTITY);
    let same_grid = render(
        &document("filterRes=\"64 64\"", 0.0),
        64,
        64,
        Affine::IDENTITY,
    );
    let coarse = render(
        &document("filterRes=\"8 8\"", 0.0),
        64,
        64,
        Affine::IDENTITY,
    );
    let zero = render(
        &document("filterRes=\"0 8\"", 0.0),
        64,
        64,
        Affine::IDENTITY,
    );
    for (x, y) in [(32, 32), (21, 32), (24, 24), (40, 32)] {
        let a = pixel(&ordinary, 64, x, y);
        let b = pixel(&same_grid, 64, x, y);
        for c in 0..4 {
            close(
                b[c],
                a[c],
                0.05,
                &format!("matching explicit identity filterRes grid at {x},{y}, channel {c}"),
            );
        }
    }
    // The 8×8 grid's cell 2 samples black (source x16..24), cell 3
    // samples white (x24..32). Restoring to 64 pixels bilinearly samples
    // q=(x+1/2)/8; at x=20 and x=23 its f64 weights are 1/16 and 7/16.
    for (x, expected) in [(20, 1.0 / 16.0), (23, 7.0 / 16.0)] {
        let p = pixel(&coarse, 64, x, 32);
        for (channel, value) in p.iter().enumerate() {
            close(
                *value,
                expected,
                0.5,
                &format!("filterRes 8×8 at x={x} channel={channel}"),
            );
        }
    }
    // σ=8 source units becomes σ=1 on the 8×8 authored filter grid.
    // The only four source grid cells are (3,3)..(4,4); convolve those
    // constant cells exactly in f64, then bilinearly restore to output.
    let blurred = render(
        &document("filterRes=\"8 8\"", 8.0),
        64,
        64,
        Affine::IDENTITY,
    );
    let mut source = vec![0.0; 8 * 8];
    for y in 3..5 {
        for x in 3..5 {
            source[y * 8 + x] = 1.0;
        }
    }
    let grid = blur(&source, 8, 8, 1, (1.0, 1.0), Edge::Transparent);
    let grid_at = |x: i32, y: i32| -> f64 {
        if (0..8).contains(&x) && (0..8).contains(&y) {
            grid[y as usize * 8 + x as usize]
        } else {
            0.0
        }
    };
    for (x, y) in [(20, 32), (32, 32), (40, 32)] {
        let qx = (x as f64 + 0.5) / 8.0 - 0.5;
        let qy = (y as f64 + 0.5) / 8.0 - 0.5;
        let (bx, by) = (qx.floor() as i32, qy.floor() as i32);
        let (fx, fy) = (qx.fract(), qy.fract());
        let expected = (1.0 - fy) * ((1.0 - fx) * grid_at(bx, by) + fx * grid_at(bx + 1, by))
            + fy * ((1.0 - fx) * grid_at(bx, by + 1) + fx * grid_at(bx + 1, by + 1));
        let p = pixel(&blurred, 64, x, y);
        for (channel, value) in p.iter().enumerate() {
            close(
                *value,
                expected,
                1.0,
                &format!("blurred filterRes grid at {x},{y}, channel {channel}"),
            );
        }
    }
    assert!(
        zero.iter().all(|v| v.abs() < 1e-6),
        "zero filterRes must produce transparent output"
    );
}

#[test]
fn zero_std_deviation_is_identity_even_when_normalized_to_offset() {
    let svg = ZERO_SVG;
    let frame = render(svg, 16, 16, Affine::IDENTITY);
    let c = pixel(&frame, 16, 4, 5);
    for (i, v) in [0x38, 0xa7, 0xe5].iter().enumerate() {
        close(c[i], f64::from(*v) / 255.0, 0.02, "zero blur color");
    }
    close(c[3], 1.0, 0.02, "zero blur alpha");
    close(pixel(&frame, 16, 2, 5)[3], 0.0, 0.02, "zero blur outside");
}

#[derive(Deserialize)]
struct WitnessManifest {
    version: u32,
    width: usize,
    height: usize,
    encoding: String,
    capture: String,
    records: Vec<WitnessRow>,
}

#[derive(Deserialize)]
struct WitnessRow {
    case: String,
    fit: String,
    bw: f64,
    bh: f64,
    scale: f64,
    rotation: f64,
    source_sha256: String,
    file: String,
    sha256: String,
}

fn filter_reference(name: &str, width: usize, height: usize) -> Vec<f64> {
    let mut out = vec![0.0; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            let (x, y) = (x as i32, y as i32);
            let rgba = match name {
                "anisotropic_gaussian_has_independent_f64_axis_weights" => {
                    let a = box_blur(x, y, (31, 31, 32, 32), (2.0, 4.0));
                    [a; 4]
                }
                "default_linear_rgb_and_explicit_srgb_blur_have_distinct_f64_colors" => {
                    let (top, bottom, linear) = if y < 12 {
                        (2, 10, true)
                    } else {
                        (14, 22, false)
                    };
                    let ay: f64 = (top..bottom).map(|s| weight(2.0, y, s)).sum();
                    let ax: f64 = (8..40).map(|s| weight(2.0, x, s)).sum();
                    let white: f64 = (24..40).map(|s| weight(2.0, x, s)).sum();
                    let a = ax * ay;
                    let p = white * ay;
                    let c = if linear && a > 0.0 {
                        let value = p / a;
                        a * if value <= 0.0031308 {
                            12.92 * value
                        } else {
                            1.055 * value.powf(1.0 / 2.4) - 0.055
                        }
                    } else {
                        p
                    };
                    [c, c, c, a]
                }
                "output_region_clips_but_source_halo_contributes" => {
                    let a = if (14..22).contains(&x) && (4..20).contains(&y) {
                        box_blur(x, y, (10, 10, 11, 11), (2.0, 2.0))
                    } else {
                        0.0
                    };
                    [a; 4]
                }
                "zero_std_deviation_is_identity_even_when_normalized_to_offset" => {
                    if (3..11).contains(&x) && (4..10).contains(&y) {
                        [
                            0x38 as f64 / 255.0,
                            0xa7 as f64 / 255.0,
                            0xe5 as f64 / 255.0,
                            1.0,
                        ]
                    } else {
                        [0.0; 4]
                    }
                }
                _ => unreachable!("every witness has an f64 reference"),
            };
            out[(y as usize * width + x as usize) * 4..][..4].copy_from_slice(&rgba);
        }
    }
    out
}

#[test]
fn filter_witness_quality() {
    let location = std::env::var_os("PM_SVG_FILTER_FRAME_REF").expect(
        "SVG filter quality gate requires PM_SVG_FILTER_FRAME_REF with actual legacy-app captures",
    );
    let path = std::path::PathBuf::from(location);
    let manifest_path = if path.is_dir() {
        path.join("manifest.json")
    } else {
        path
    };
    let root = manifest_path.parent().unwrap();
    let manifest: WitnessManifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest.version, 1);
    assert_eq!((manifest.width, manifest.height), (64, 64));
    assert_eq!(manifest.encoding, "premultiplied-rgba8-topdown");
    assert!(manifest.capture.starts_with("legacy PM.GL.renderToPixels"));
    assert_eq!(manifest.records.len(), 4);
    let cases = [
        (
            "anisotropic_gaussian_has_independent_f64_axis_weights",
            ANISOTROPIC_SVG,
            64,
            64,
        ),
        (
            "default_linear_rgb_and_explicit_srgb_blur_have_distinct_f64_colors",
            COLOR_SPACES_SVG,
            64,
            24,
        ),
        (
            "output_region_clips_but_source_halo_contributes",
            REGION_SVG,
            32,
            24,
        ),
        (
            "zero_std_deviation_is_identity_even_when_normalized_to_offset",
            ZERO_SVG,
            16,
            16,
        ),
    ];
    let mut evidence = Vec::new();
    let mut failures = Vec::new();
    for (name, svg, width, height) in cases {
        let rows: Vec<_> = manifest.records.iter().filter(|r| r.case == name).collect();
        assert_eq!(
            rows.len(),
            1,
            "{name}: actual witness missing or duplicated"
        );
        let row = rows[0];
        assert_eq!(
            row.source_sha256,
            format!("{:x}", Sha256::digest(svg.as_bytes()))
        );
        assert_eq!(row.fit, "contain");
        assert_eq!(
            (row.bw, row.bh, row.scale, row.rotation),
            (width as f64, height as f64, 100.0, 0.0)
        );
        assert_eq!(
            std::path::Path::new(&row.file)
                .file_name()
                .and_then(|f| f.to_str()),
            Some(row.file.as_str())
        );
        let bytes = std::fs::read(root.join(&row.file)).unwrap();
        assert_eq!(bytes.len(), 64 * 64 * 4);
        assert_eq!(row.sha256, format!("{:x}", Sha256::digest(&bytes)));
        let actual = render(svg, width as u32, height as u32, Affine::IDENTITY);
        let engine: Vec<f64> = actual.into_iter().map(f64::from).collect();
        let reference = filter_reference(name, width, height);
        let mut today = Vec::with_capacity(width * height * 4);
        let (ox, oy) = ((64 - width) / 2, (64 - height) / 2);
        for y in 0..height {
            for x in 0..width {
                let index = ((y + oy) * 64 + x + ox) * 4;
                today.extend(
                    bytes[index..index + 4]
                        .iter()
                        .map(|v| f64::from(*v) / 255.0),
                );
            }
        }
        let report = compare(
            &engine,
            &today,
            &reference,
            &vec![Class::Resampled; width * height],
        );
        let edge = report.class(Class::Resampled);
        let alpha = |f: &[f64]| f.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>();
        let (eg, rg) = (
            geometry(&alpha(&engine), width),
            geometry(&alpha(&reference), width),
        );
        let area_relative = (eg.area - rg.area).abs() / rg.area.max(1.0);
        let centroid_px = (eg.centroid[0] - rg.centroid[0])
            .abs()
            .max((eg.centroid[1] - rg.centroid[1]).abs());
        if let Err(error) = same_geometry(&eg, &rg, 1e-3) {
            failures.push(format!("{name}: {error}"));
        }
        for error in report.failures(&Bounds::EIGHT_BIT) {
            failures.push(format!("{name}: {error}"));
        }
        evidence.push(serde_json::json!({
            "case": name, "actual_witness_sha256": row.sha256,
            "engine": { "max_cv": edge.engine_vs_reference.max, "p999_cv": edge.engine_vs_reference.p999, "mean_cv": edge.engine_vs_reference.mean },
            "today": { "max_cv": edge.today_vs_reference.max, "p999_cv": edge.today_vs_reference.p999, "mean_cv": edge.today_vs_reference.mean },
            "excess": { "max_cv": edge.excess.max, "p999_cv": edge.excess.p999 },
            "geometry": { "area_relative": area_relative, "centroid_px": centroid_px },
        }));
    }
    if let Some(output) = std::env::var_os("PM_SVG_FILTER_EVIDENCE") {
        std::fs::write(output, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// The desktop gate's exact ART fixture. World matrices are read live from
// PM.worldMatrix; this reference owns only the authored SVG math. Pixel
// evidence is binary little-endian f64: RGBA frame, then alpha for each
// layer in input order. It never accepts engine or legacy pixels as input.
const ART_WIDTH: usize = 640;
const ART_HEIGHT: usize = 360;
const ART_GRID: usize = 8;

#[derive(Clone, Deserialize)]
struct ArtLayer {
    name: String,
    fit: String,
    bw: f64,
    bh: f64,
    m: [f64; 6],
    alpha: f64,
}

#[derive(Deserialize)]
struct ArtInput {
    width: usize,
    height: usize,
    svg_sha256: String,
    layers: Vec<ArtLayer>,
    project: serde_json::Value,
    asset_id: String,
    time: f64,
}

struct ArtGeometry {
    source_from_fine: Affine,
    layer_from_fine: Affine,
    round: pm_reference::coverage::Coverage,
    circle: pm_reference::coverage::Coverage,
    line: pm_reference::coverage::Coverage,
    dash: pm_reference::coverage::Coverage,
}

fn art_geometry(layer: &ArtLayer) -> ArtGeometry {
    art_geometry_window(
        layer,
        ART_GRID,
        0,
        0,
        ART_WIDTH * ART_GRID,
        ART_HEIGHT * ART_GRID,
        false,
    )
}

fn art_geometry_window(
    layer: &ArtLayer,
    grid: usize,
    pixel_x: usize,
    pixel_y: usize,
    fw: usize,
    fh: usize,
    spot: bool,
) -> ArtGeometry {
    assert!(layer.bw > 0.0 && layer.bh > 0.0 && layer.alpha >= 0.0 && layer.alpha <= 1.0);
    let world = Affine::new(layer.m);
    let aspect = 160.0 / 90.0;
    let box_aspect = layer.bw / layer.bh;
    let (image_w, image_h) = match layer.fit.as_str() {
        "contain" => {
            if aspect > box_aspect {
                (layer.bw, layer.bw / aspect)
            } else {
                (layer.bh * aspect, layer.bh)
            }
        }
        "cover" => {
            if aspect > box_aspect {
                (layer.bh * aspect, layer.bh)
            } else {
                (layer.bw, layer.bw / aspect)
            }
        }
        "stretch" => (layer.bw, layer.bh),
        other => panic!("unexpected fit {other}"),
    };
    let image = Affine::translate((-image_w / 2.0, -image_h / 2.0))
        * Affine::scale_non_uniform(image_w / 160.0, image_h / 90.0);
    let fine =
        Affine::scale(grid as f64) * Affine::translate((-(pixel_x as f64), -(pixel_y as f64)));
    let source_to_fine = fine * world * image;
    let coverage = |path: BezPath, cells: usize| {
        fill(
            &(source_to_fine * path),
            FillRule::NonZero,
            if spot { 1 } else { cells },
            fw,
            fh,
        )
    };
    let round = coverage(
        RoundedRect::new(4.0, 4.0, 156.0, 86.0, 14.0).to_path(1e-7),
        2,
    );
    let circle = coverage(Circle::new((120.0, 45.0), 34.0).to_path(1e-7), 2);
    let mut line_path = BezPath::new();
    line_path.move_to((80.0, 90.0));
    line_path.line_to((160.0, 0.0));
    let line = coverage(
        reference_stroke(&line_path, &Stroke::new(6.0).with_caps(Cap::Butt), 1e-6)
            .unwrap()
            .path,
        2,
    );
    let mut curve = BezPath::new();
    curve.move_to((10.0, 80.0));
    curve.quad_to((60.0, 40.0), (110.0, 80.0));
    let dash_style = Stroke::new(3.5)
        .with_caps(Cap::Round)
        .with_dashes(0.0, [7.0, 4.0]);
    let dash = coverage(reference_stroke(&curve, &dash_style, 1e-6).unwrap().path, 4);
    ArtGeometry {
        source_from_fine: source_to_fine.inverse(),
        layer_from_fine: (fine * world).inverse(),
        round,
        circle,
        line,
        dash,
    }
}

#[derive(Deserialize)]
struct ArtSpotInput {
    svg_sha256: String,
    layers: Vec<ArtLayer>,
    pixels: Vec<[usize; 2]>,
}

fn art_compose_resolved(layers: &[[f64; 4]]) -> [f64; 4] {
    let mut color = [
        0x20 as f64 / 255.0,
        0x30 as f64 / 255.0,
        0x40 as f64 / 255.0,
        1.0,
    ];
    for layer in layers {
        for c in 0..3 {
            color[c] = layer[c] + color[c] * (1.0 - layer[3]);
        }
    }
    color
}

fn art_max_color_delta_cv(a: [f64; 4], b: [f64; 4]) -> f64 {
    (0..3)
        .map(|c| (a[c] - b[c]).abs() * 255.0)
        .fold(0.0, f64::max)
}

fn art_spot_pixel(
    input: &ArtSpotInput,
    x: usize,
    y: usize,
    grid: usize,
) -> ([f64; 4], Vec<[f64; 4]>, [f64; 4]) {
    assert!(x < ART_WIDTH && y < ART_HEIGHT && grid > 0);
    let geometry: Vec<_> = input
        .layers
        .iter()
        .map(|layer| art_geometry_window(layer, grid, x, y, grid, grid, true))
        .collect();
    let mut color = [0.0; 4];
    let mut layers = vec![[0.0; 4]; input.layers.len()];
    for sy in 0..grid {
        for sx in 0..grid {
            let mut sample = [
                0x20 as f64 / 255.0,
                0x30 as f64 / 255.0,
                0x40 as f64 / 255.0,
                1.0,
            ];
            for (j, (layer, g)) in input.layers.iter().zip(&geometry).enumerate() {
                let source = art_sample(layer, g, sx, sy);
                for c in 0..3 {
                    sample[c] = source[c] + sample[c] * (1.0 - source[3]);
                }
                for c in 0..4 {
                    layers[j][c] += source[c];
                }
            }
            for c in 0..4 {
                color[c] += sample[c];
            }
        }
    }
    let samples = (grid * grid) as f64;
    let joint = color.map(|v| v / samples);
    for layer in &mut layers {
        for value in layer {
            *value /= samples;
        }
    }
    let separately_resolved = art_compose_resolved(&layers);
    (joint, layers, separately_resolved)
}

#[test]
fn art_high_resolution_spot() {
    // The public ART fixture's t=1.5 authored placement. At this pixel the
    // stretch and cover SVG images overlap with partial coverage. Resolving
    // each image's own premultiplied samples before compositing is observably
    // different from compositing both at the same artificial fine grid.
    let authored = ArtSpotInput {
        svg_sha256: "bdac5780bec879d66ecde5f9306be7136f38da06e1f385fea30f2b36684af7a7".into(),
        layers: vec![
            ArtLayer {
                name: "Contain".into(),
                fit: "contain".into(),
                bw: 200.0,
                bh: 140.0,
                m: [1.0, 0.0, 0.0, 1.0, 130.0, 100.0],
                alpha: 1.0,
            },
            ArtLayer {
                name: "Stretch".into(),
                fit: "stretch".into(),
                bw: 160.0,
                bh: 60.0,
                m: [2.475, 0.0, 0.0, 1.0, 300.0, 290.0],
                alpha: 0.7,
            },
            ArtLayer {
                name: "Cover".into(),
                fit: "cover".into(),
                bw: 120.0,
                bh: 120.0,
                m: [
                    2.117161162940613,
                    0.8986815955253294,
                    -0.8986815955253294,
                    2.117161162940613,
                    420.0,
                    120.0,
                ],
                alpha: 1.0,
            },
        ],
        pixels: vec![[468, 277]],
    };
    let n8 = art_spot_pixel(&authored, 468, 277, 8);
    let n16 = art_spot_pixel(&authored, 468, 277, 16);
    let n512 = art_spot_pixel(&authored, 468, 277, 512);
    assert!(art_max_color_delta_cv(n512.0, n512.2) > 5.0);
    assert!(art_max_color_delta_cv(n8.0, n512.0) < 0.41);
    assert!(art_max_color_delta_cv(n16.0, n512.0) < 0.06);
    assert!(art_max_color_delta_cv(n8.2, n512.2) < 0.01);
    assert!(art_max_color_delta_cv(n16.2, n512.2) < 0.002);
    assert!(art_ring_alpha(20.0, 45.0) > art_ring_alpha(45.0, 45.0));
    match (
        std::env::var_os("PM_SVG_SPOT_INPUT"),
        std::env::var_os("PM_SVG_SPOT_OUTPUT"),
    ) {
        (None, None) => (),
        (Some(input), Some(output)) => {
            let input: ArtSpotInput =
                serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
            assert_eq!(
                input.svg_sha256,
                "bdac5780bec879d66ecde5f9306be7136f38da06e1f385fea30f2b36684af7a7"
            );
            assert_eq!(
                input
                    .layers
                    .iter()
                    .map(|layer| layer.name.as_str())
                    .collect::<Vec<_>>(),
                ["Contain", "Stretch", "Cover"]
            );
            assert!(!input.pixels.is_empty() && input.pixels.len() <= 3);
            let results: Vec<_> = input
                .pixels
                .iter()
                .map(|&[x, y]| {
                    let n4 = art_spot_pixel(&input, x, y, 4);
                    let n8 = art_spot_pixel(&input, x, y, 8);
                    let n16 = art_spot_pixel(&input, x, y, 16);
                    let n512 = art_spot_pixel(&input, x, y, 512);
                    serde_json::json!({"x":x,"y":y,
                    "grid4":{"joint":n4.0,"layers":n4.1,"separate":n4.2},
                    "grid8":{"joint":n8.0,"layers":n8.1,"separate":n8.2},
                    "grid16":{"joint":n16.0,"layers":n16.1,"separate":n16.2},
                    "grid512":{"joint":n512.0,"layers":n512.1,"separate":n512.2},
                    "jointDeltaCv":art_max_color_delta_cv(n16.0,n512.0),
                    "separateDeltaCv":art_max_color_delta_cv(n16.2,n512.2)})
                })
                .collect();
            std::fs::write(output, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
        }
        _ => panic!("both SVG spot input and output paths are required"),
    }
}

fn normal_cdf(z: f64) -> f64 {
    0.5 * libm::erfc(-z / std::f64::consts::SQRT_2)
}

fn gaussian_interval(x: f64, a: f64, b: f64, sigma: f64) -> f64 {
    normal_cdf((x - a) / sigma) - normal_cdf((x - b) / sigma)
}

fn art_ring_alpha(x: f64, y: f64) -> f64 {
    // SVG's default filter region expands the path's 20..70 bbox by 10%.
    if !(15.0..75.0).contains(&x) || !(15.0..75.0).contains(&y) {
        return 0.0;
    }
    let outer = gaussian_interval(x, 20.0, 70.0, 2.5) * gaussian_interval(y, 20.0, 70.0, 2.5);
    let inner = gaussian_interval(x, 32.0, 58.0, 2.5) * gaussian_interval(y, 32.0, 58.0, 2.5);
    (outer - inner).max(0.0)
}

fn over(dst: &mut [f64; 4], rgb: [f64; 3], alpha: f64) {
    let remaining = 1.0 - alpha;
    for c in 0..3 {
        dst[c] = rgb[c] * alpha + dst[c] * remaining;
    }
    dst[3] = alpha + dst[3] * remaining;
}

fn art_sample(layer: &ArtLayer, g: &ArtGeometry, fine_x: usize, fine_y: usize) -> [f64; 4] {
    let sample = Point::new(fine_x as f64 + 0.5, fine_y as f64 + 0.5);
    let local = g.layer_from_fine * sample;
    if local.x < -layer.bw / 2.0
        || local.x >= layer.bw / 2.0
        || local.y < -layer.bh / 2.0
        || local.y >= layer.bh / 2.0
    {
        return [0.0; 4];
    }
    let p = g.source_from_fine * sample;
    if p.x < 0.0 || p.x >= 160.0 || p.y < 0.0 || p.y >= 90.0 {
        return [0.0; 4];
    }
    let mut out = [0.0; 4];
    let t = (((p.x - 4.0) / 152.0 + (p.y - 4.0) / 82.0) * 0.5).clamp(0.0, 1.0);
    let blue = [0x4c as f64 / 255.0, 0x8d as f64 / 255.0, 1.0];
    let red = [
        0xe8 as f64 / 255.0,
        0x3a as f64 / 255.0,
        0x5e as f64 / 255.0,
    ];
    let rgb = std::array::from_fn(|c| blue[c] * (1.0 - t) + red[c] * t);
    over(&mut out, rgb, g.round.at(fine_x, fine_y) * (1.0 - 0.4 * t));
    let mut group = [0.0; 4];
    if p.x >= 80.0 && p.x < 160.0 {
        over(
            &mut group,
            [1.0, 0xd2 as f64 / 255.0, 0x3f as f64 / 255.0],
            1.0,
        );
    }
    over(
        &mut group,
        [
            0x14 as f64 / 255.0,
            0x16 as f64 / 255.0,
            0x1c as f64 / 255.0,
        ],
        g.line.at(fine_x, fine_y),
    );
    let clip = g.circle.at(fine_x, fine_y);
    for value in &mut group {
        *value *= clip;
    }
    for c in 0..3 {
        out[c] = group[c] + out[c] * (1.0 - group[3]);
    }
    out[3] = group[3] + out[3] * (1.0 - group[3]);
    over(&mut out, [1.0; 3], art_ring_alpha(p.x, p.y));
    over(
        &mut out,
        [0x7c as f64 / 255.0, 1.0, 0xb2 as f64 / 255.0],
        g.dash.at(fine_x, fine_y),
    );
    for v in &mut out {
        *v *= layer.alpha;
    }
    out
}

fn art_reference(input: &ArtInput) -> Vec<f64> {
    assert_eq!((input.width, input.height), (ART_WIDTH, ART_HEIGHT));
    assert_eq!(
        input.svg_sha256, "bdac5780bec879d66ecde5f9306be7136f38da06e1f385fea30f2b36684af7a7",
        "the ART fixture changed; review its authored f64 reference"
    );
    assert_eq!(input.layers.len(), 3);
    assert_eq!(
        input
            .layers
            .iter()
            .map(|l| l.name.as_str())
            .collect::<Vec<_>>(),
        ["Contain", "Stretch", "Cover"]
    );
    let geometry: Vec<_> = input.layers.iter().map(art_geometry).collect();
    let pixels = ART_WIDTH * ART_HEIGHT;
    let mut result = vec![0.0; (4 + input.layers.len()) * pixels];
    for y in 0..ART_HEIGHT {
        for x in 0..ART_WIDTH {
            let i = y * ART_WIDTH + x;
            let mut layer_rgba = vec![[0.0; 4]; input.layers.len()];
            for sy in 0..ART_GRID {
                for sx in 0..ART_GRID {
                    let fx = x * ART_GRID + sx;
                    let fy = y * ART_GRID + sy;
                    for (j, (layer, g)) in input.layers.iter().zip(&geometry).enumerate() {
                        let source = art_sample(layer, g, fx, fy);
                        for c in 0..4 {
                            layer_rgba[j][c] += source[c];
                        }
                    }
                }
            }
            // Each SVG image is rasterized as its own layer. Resolve its
            // premultiplied RGBA coverage first, then composite the resolved
            // image pixels. Compositing fine samples across different images
            // would invent subpixel correlation between their independent
            // raster surfaces.
            for (j, layer) in layer_rgba.iter_mut().enumerate() {
                for value in layer.iter_mut() {
                    *value /= (ART_GRID * ART_GRID) as f64;
                }
                result[4 * pixels + j * pixels + i] = layer[3];
            }
            let color = art_compose_resolved(&layer_rgba);
            for c in 0..4 {
                result[i * 4 + c] = color[c];
            }
        }
    }
    result
}

/// Render the exact live project document at its authored time and depth.
/// The first frame has the project background; each later frame contains one
/// original layer with the background disabled, as the window snapshot does.
fn art_native_frames(input: &ArtInput, svg_path: &std::path::Path) -> Vec<Vec<f32>> {
    assert_eq!(input.project["colorDepth"].as_u64(), Some(32));
    assert!(input.project["assets"].get(&input.asset_id).is_some());
    let composition = input.project["compId"].as_str().expect("live composition");
    let project_layers = input.project["layers"]
        .as_array()
        .expect("live project layers");
    assert_eq!(project_layers.len(), input.layers.len());
    for layer in project_layers {
        assert_eq!(layer["d"]["asset"].as_str(), Some(input.asset_id.as_str()));
    }
    let mut assets = MapResolver::default();
    assets
        .paths
        .insert(input.asset_id.clone(), svg_path.to_path_buf());
    let mut renderer = Renderer::headless().expect("native SVG renderer");
    let time = Ticks::new((input.time * pm_time::TICKS_PER_SECOND as f64).round() as i64)
        .expect("live time");
    let mut request = RenderRequest::sized(
        Size::new(ART_WIDTH as u32, ART_HEIGHT as u32),
        Precision::Float32,
    );
    request.output_kind = OutputKind::Final;
    let mut frame_for = |project: &serde_json::Value, background: bool| {
        let document = pm_model::Document::new(
            Some(pm_model::json::parse(&project.to_string()).expect("live project document")),
            1,
        );
        request.background = background;
        let frame = renderer
            .render(&document, composition, time, &request, &assets)
            .expect("native SVG frame");
        assert!(
            frame.failures.is_empty(),
            "native SVG frame failures: {:?}",
            frame.failures
        );
        assert_eq!(frame.color.format(), wgpu::TextureFormat::Rgba32Float);
        let pixels = renderer.read_rgba(&frame).expect("native RGBA32F readback");
        renderer.recycle(frame);
        assert_eq!(pixels.len(), ART_WIDTH * ART_HEIGHT * 4);
        pixels
    };
    let mut frames = vec![frame_for(&input.project, true)];
    for layer in &input.layers {
        let matching: Vec<_> = project_layers
            .iter()
            .filter(|entry| entry["name"].as_str() == Some(layer.name.as_str()))
            .cloned()
            .collect();
        assert_eq!(matching.len(), 1, "one live layer for {}", layer.name);
        let mut isolated = input.project.clone();
        isolated["layers"] = serde_json::Value::Array(matching);
        frames.push(frame_for(&isolated, false));
    }
    frames
}

fn snapshot_byte(value: f32) -> u8 {
    let clamped = if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    };
    (clamped * 255.0).round_ties_even() as u8
}

fn write_art_native(
    input: &ArtInput,
    svg_path: &std::path::Path,
    float_path: &std::path::Path,
    byte_path: &std::path::Path,
) {
    let source = std::fs::read(svg_path).expect("live ART source bytes");
    assert_eq!(format!("{:x}", Sha256::digest(&source)), input.svg_sha256);
    let frames = art_native_frames(input, svg_path);
    let mut floats = Vec::with_capacity(frames.len() * ART_WIDTH * ART_HEIGHT * 16);
    let mut bytes = Vec::with_capacity(frames.len() * ART_WIDTH * ART_HEIGHT * 4);
    for (index, frame) in frames.iter().enumerate() {
        for pixel in frame.chunks_exact(4) {
            for value in pixel {
                assert!(value.is_finite(), "finite native SVG pixel");
                floats.extend_from_slice(&value.to_le_bytes());
            }
            bytes.extend(pixel.iter().enumerate().map(|(channel, value)| {
                if index == 0 && channel == 3 {
                    255
                } else {
                    snapshot_byte(*value)
                }
            }));
        }
    }
    std::fs::write(float_path, floats).expect("native RGBA32F evidence");
    std::fs::write(byte_path, bytes).expect("quantized native endpoint evidence");
}

#[test]
fn art_reference_export() {
    // This is also a normal test: a white even-odd ring's analytically
    // blurred center must be nearly empty, while its rim and halo survive.
    assert!(art_ring_alpha(45.0, 45.0) < 1e-5);
    assert!(art_ring_alpha(20.0, 45.0) > 0.45);
    assert!(art_ring_alpha(17.0, 45.0) > 0.0);
    assert_eq!(art_ring_alpha(14.0, 45.0), 0.0);
    let input = std::env::var_os("PM_SVG_REFERENCE_INPUT");
    let output = std::env::var_os("PM_SVG_REFERENCE_OUTPUT");
    let native = std::env::var_os("PM_SVG_NATIVE_OUTPUT");
    let endpoint = std::env::var_os("PM_SVG_ENDPOINT_OUTPUT");
    let svg = std::env::var_os("PM_SVG_ART_PATH");
    match (input, output, native, endpoint, svg) {
        (None, None, None, None, None) => (),
        (Some(input), Some(output), Some(native), Some(endpoint), Some(svg)) => {
            let input: ArtInput = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
            let frame = art_reference(&input);
            let mut bytes = Vec::with_capacity(frame.len() * 8);
            for value in frame {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            std::fs::write(output, bytes).unwrap();
            write_art_native(
                &input,
                std::path::Path::new(&svg),
                std::path::Path::new(&native),
                std::path::Path::new(&endpoint),
            );
        }
        _ => panic!("all SVG reference input and output paths must be supplied together"),
    }
}
