//! Native float SVG filter graphs against independent f64 evaluation.
//!
//! Each public synthetic export (`fixtures/svg-filters/exports`, the shapes
//! Figma, Illustrator and Sketch write for shadows, glows and layer blurs,
//! CSS-style `feDropShadow`, and every per-pixel primitive) is rendered by
//! the float painter and compared with the same filter graph evaluated in
//! f64 by `pm_reference::filter` from Filter Effects Level 1: exact source
//! coverage, the exact Gaussian, each primitive in its colour space. The
//! declared resvg fallback (RGBA8, what these documents drew before) is
//! measured against the same reference; the float path must be closer.

use pm_protocol::color::Precision;
use pm_reference::composite::{Blend, Operator, porter_duff};
use pm_reference::coverage::{FillRule, fill};
use pm_reference::filter::{
    self, CompositeOp, Function, Image, Matrix, Space, from_space, into_space,
};
use pm_reference::gaussian::kernel;
use pm_render::vector::svg_float::SvgPainter;
use pm_vector::kurbo::{Affine, BezPath, Circle, Rect, RoundedRect, Shape};
use pm_vector::render::scene::PixelRect;
use pm_vector::render::svg::SvgDocument;

/// The bound for float filter output against f64: the half-float
/// precision bound of the gate policy (2⁻¹¹ ≈ 0.125 CV), which also covers
/// the shared Gaussian's 2⁻¹² plan error once per blur in a chain.
const FLOAT_BOUND_CV: f64 = 0.125;

fn painter_render(
    doc: &SvgDocument,
    transform: Affine,
    w: u32,
    h: u32,
) -> Result<Vec<f32>, String> {
    let gpu = pm_gpu::Gpu::new("SVG filter graphs").unwrap();
    let mut painter = SvgPainter::new(&gpu.device).unwrap();
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let rendered = painter
        .render(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            doc,
            transform,
            PixelRect::new(0, 0, w, h),
            &[],
            Precision::Float32,
        )
        .unwrap();
    let Some((texture, _)) = rendered else {
        return Err(painter.fallback_reason().unwrap_or("no reason").to_owned());
    };
    gpu.queue.submit([encoder.finish()]);
    Ok(pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &texture).unwrap())
}

/// Max, p99.9 and mean of the largest channel error per pixel, in CV.
fn closeness(actual: &[f64], reference: &Image) -> [f64; 3] {
    let mut errors: Vec<f64> = reference
        .pixels
        .iter()
        .enumerate()
        .map(|(i, r)| {
            (0..4)
                .map(|c| (actual[i * 4 + c] - r[c]).abs() * 255.0)
                .fold(0.0, f64::max)
        })
        .collect();
    let mean = errors.iter().sum::<f64>() / errors.len() as f64;
    errors.sort_by(f64::total_cmp);
    let p999 = errors[((errors.len() as f64 * 0.999) as usize).min(errors.len() - 1)];
    [errors[errors.len() - 1], p999, mean]
}

/// A source paint: an outline in user units and its straight sRGB colour.
struct Shape2 {
    outline: BezPath,
    rgba: [f64; 4],
}

fn hex(v: u32, a: f64) -> [f64; 4] {
    [
        f64::from((v >> 16) & 255) / 255.0,
        f64::from((v >> 8) & 255) / 255.0,
        f64::from(v & 255) / 255.0,
        a,
    ]
}

fn card(rgba: [f64; 4], height: f64) -> Shape2 {
    Shape2 {
        outline: RoundedRect::new(20.0, 16.0, 100.0, 16.0 + height, 8.0).to_path(1e-7),
        rgba,
    }
}

/// The source graphic at `scale`: each shape's exact coverage, painted
/// source-over in sRGB encoding (SVG paint compositing).
fn source(shapes: &[Shape2], scale: f64, w: usize, h: usize) -> Image {
    let mut image = Image::new(w, h);
    for s in shapes {
        let cov = fill(
            &(Affine::scale(scale) * s.outline.clone()),
            FillRule::NonZero,
            16,
            w,
            h,
        );
        for y in 0..h {
            for x in 0..w {
                let a = s.rgba[3] * cov.at(x, y);
                let c = [s.rgba[0] * a, s.rgba[1] * a, s.rgba[2] * a, a];
                let p = &mut image.pixels[y * w + x];
                *p = porter_duff(Operator::SourceOver, c, *p);
            }
        }
    }
    image
}

/// An input of a reference primitive.
#[derive(Clone, Copy)]
enum In {
    Source,
    Alpha,
    R(usize),
}

/// A reference primitive, in device units (σ and offsets already scaled).
enum Op {
    Flood([f64; 3], f64),
    Matrix(Matrix, In),
    Transfer([Function; 4], In),
    Composite(CompositeOp, In, In),
    Blend(Blend, In, In),
    Offset(i64, i64, In),
    Blur(f64, In),
    Merge(Vec<In>),
    DropShadow {
        dx: i64,
        dy: i64,
        sigma: f64,
        rgb: [f64; 3],
        opacity: f64,
        input: In,
    },
}

/// The graph in f64: every primitive in `space`, clipped to `region`
/// (whole device pixels for these fixtures).
fn evaluate(src: &Image, ops: &[Op], space: Space, region: [i64; 4]) -> Image {
    let mut results: Vec<Image> = Vec::new();
    let get = |input: In, results: &[Image]| match input {
        In::Source => src.clone(),
        In::Alpha => src.map(filter::alpha),
        In::R(i) => results[i].clone(),
    };
    let pointwise =
        |a: &Image, f: &dyn Fn(pm_reference::composite::Rgba) -> pm_reference::composite::Rgba| {
            a.map(|p| from_space(f(into_space(p, space)), space))
        };
    let pairwise = |a: &Image,
                    b: &Image,
                    f: &dyn Fn(
        pm_reference::composite::Rgba,
        pm_reference::composite::Rgba,
    ) -> pm_reference::composite::Rgba| {
        a.zip(b, |p, q| {
            from_space(f(into_space(p, space), into_space(q, space)), space)
        })
    };
    let blur = |a: &Image, sigma: f64| {
        filter::gaussian(&a.map(|p| into_space(p, space)), (sigma, sigma))
            .map(|p| from_space(p, space))
    };
    for op in ops {
        let out = match op {
            Op::Flood(rgb, a) => Image::new(src.width, src.height).map(|_| filter::flood(*rgb, *a)),
            Op::Matrix(m, i) => pointwise(&get(*i, &results), &|p| filter::color_matrix(m, p)),
            Op::Transfer(f, i) => {
                pointwise(&get(*i, &results), &|p| filter::component_transfer(f, p))
            }
            Op::Composite(c, a, b) => pairwise(&get(*a, &results), &get(*b, &results), &|p, q| {
                filter::composite(*c, p, q)
            }),
            Op::Blend(m, a, b) => pairwise(&get(*a, &results), &get(*b, &results), &|p, q| {
                filter::blend(*m, p, q)
            }),
            Op::Offset(dx, dy, i) => filter::offset(&get(*i, &results), *dx, *dy),
            Op::Blur(sigma, i) => blur(&get(*i, &results), *sigma),
            Op::Merge(inputs) => {
                let images: Vec<Image> = inputs.iter().map(|i| get(*i, &results)).collect();
                let mut out = Image::new(src.width, src.height);
                for (k, p) in out.pixels.iter_mut().enumerate() {
                    let layers: Vec<_> = images
                        .iter()
                        .map(|im| into_space(im.pixels[k], space))
                        .collect();
                    *p = from_space(filter::merge(&layers), space);
                }
                out
            }
            Op::DropShadow {
                dx,
                dy,
                sigma,
                rgb,
                opacity,
                input,
            } => {
                let base = get(*input, &results);
                let shadow = filter::offset(&blur(&base.map(filter::alpha), *sigma), *dx, *dy);
                let color = filter::flood(*rgb, *opacity);
                let tinted = shadow.map(|p| color.map(|c| c * p[3]));
                pairwise(&base, &tinted, &|p, q| {
                    filter::composite(CompositeOp::Over, p, q)
                })
            }
        };
        results.push(filter::clip(&out, region));
    }
    results.pop().expect("a primitive")
}

struct Fixture {
    file: &'static str,
    shapes: Vec<Shape2>,
    space: Space,
    /// The filter region in user units.
    region: [f64; 4],
    graph: fn(f64) -> Vec<Op>,
}

const HARD_ALPHA: [f64; 20] = [
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
    127.0, 0.0,
];

fn alpha_matrix(a: f64) -> Matrix {
    let mut m = [0.0; 20];
    m[18] = a;
    Matrix::Values(m)
}

fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture {
            file: "figma-drop-shadow.svg",
            shapes: vec![card(hex(0x4C8DFF, 1.0), 64.0)],
            space: Space::Srgb,
            region: [8.0, 8.0, 112.0, 96.0],
            graph: |s| {
                vec![
                    Op::Flood([0.0; 3], 0.0),
                    Op::Matrix(Matrix::Values(HARD_ALPHA), In::Alpha),
                    Op::Offset(0, (4.0 * s) as i64, In::R(1)),
                    Op::Blur(6.0 * s, In::R(2)),
                    Op::Composite(CompositeOp::Out, In::R(3), In::R(1)),
                    Op::Matrix(alpha_matrix(0.25), In::R(4)),
                    Op::Blend(Blend::Normal, In::R(5), In::R(0)),
                    Op::Blend(Blend::Normal, In::Source, In::R(6)),
                ]
            },
        },
        Fixture {
            file: "figma-inner-shadow.svg",
            shapes: vec![card(hex(0xF2A541, 1.0), 64.0)],
            space: Space::Srgb,
            region: [20.0, 16.0, 100.0, 84.0],
            graph: |s| {
                vec![
                    Op::Flood([0.0; 3], 0.0),
                    Op::Blend(Blend::Normal, In::Source, In::R(0)),
                    Op::Matrix(Matrix::Values(HARD_ALPHA), In::Alpha),
                    Op::Offset(0, (4.0 * s) as i64, In::R(2)),
                    Op::Blur(2.0 * s, In::R(3)),
                    Op::Composite(
                        CompositeOp::Arithmetic([0.0, -1.0, 1.0, 0.0]),
                        In::R(4),
                        In::R(2),
                    ),
                    Op::Matrix(alpha_matrix(0.25), In::R(5)),
                    Op::Blend(Blend::Normal, In::R(6), In::R(1)),
                ]
            },
        },
        Fixture {
            file: "figma-layer-blur.svg",
            shapes: vec![card(hex(0x3FCF8E, 1.0), 64.0)],
            space: Space::Srgb,
            region: [-30.0, -34.0, 150.0, 130.0],
            graph: |s| {
                vec![
                    Op::Flood([0.0; 3], 0.0),
                    Op::Blend(Blend::Normal, In::Source, In::R(0)),
                    Op::Blur(25.0 * s, In::R(1)),
                ]
            },
        },
        Fixture {
            file: "illustrator-drop-shadow.svg",
            shapes: vec![card(hex(0xE83A5E, 1.0), 60.0)],
            space: Space::LinearRgb,
            region: [4.0, 4.0, 116.0, 88.0],
            graph: |s| {
                let d = (5.0 * s) as i64;
                vec![
                    Op::Blur(3.0 * s, In::Alpha),
                    Op::Offset(d, d, In::R(0)),
                    Op::Merge(vec![In::R(1), In::Source]),
                ]
            },
        },
        Fixture {
            file: "sketch-shadow.svg",
            shapes: vec![card(hex(0xFFFFFF, 1.0), 64.0)],
            space: Space::LinearRgb,
            region: [10.0, 8.0, 110.0, 88.0],
            graph: |s| {
                vec![
                    Op::Offset(0, (2.0 * s) as i64, In::Alpha),
                    Op::Blur(3.0 * s, In::R(0)),
                    Op::Matrix(alpha_matrix(0.5), In::R(1)),
                    Op::Merge(vec![In::R(2), In::Source]),
                ]
            },
        },
        Fixture {
            file: "css-drop-shadow.svg",
            shapes: vec![card(hex(0x4C8DFF, 1.0), 64.0)],
            space: Space::LinearRgb,
            region: [0.0, 0.0, 120.0, 96.0],
            graph: |s| {
                let c = hex(0x102040, 1.0);
                vec![Op::DropShadow {
                    dx: (6.0 * s) as i64,
                    dy: (8.0 * s) as i64,
                    sigma: 5.0 * s,
                    rgb: [c[0], c[1], c[2]],
                    opacity: 0.6,
                    input: In::Source,
                }]
            },
        },
        Fixture {
            file: "neon-glow.svg",
            shapes: vec![Shape2 {
                outline: Circle::new((60.0, 50.0), 24.0).to_path(1e-7),
                rgba: hex(0x39FF14, 1.0),
            }],
            space: Space::LinearRgb,
            region: [12.0, 2.0, 108.0, 98.0],
            graph: |s| {
                vec![
                    Op::Blur(8.0 * s, In::Source),
                    Op::Merge(vec![In::R(0), In::R(0), In::Source]),
                ]
            },
        },
        Fixture {
            file: "tinted-glow.svg",
            shapes: vec![card(hex(0x1B1F2A, 1.0), 64.0)],
            space: Space::Srgb,
            region: [-20.0, -16.0, 140.0, 112.0],
            graph: |s| {
                let c = hex(0xFF3D7F, 1.0);
                vec![
                    Op::Flood([c[0], c[1], c[2]], 0.9),
                    Op::Composite(CompositeOp::In, In::R(0), In::Alpha),
                    Op::Blur(6.0 * s, In::R(1)),
                    Op::Transfer(
                        [
                            Function::Identity,
                            Function::Identity,
                            Function::Identity,
                            Function::Linear {
                                slope: 1.6,
                                intercept: 0.0,
                            },
                        ],
                        In::R(2),
                    ),
                    Op::Merge(vec![In::R(3), In::Source]),
                ]
            },
        },
        Fixture {
            file: "color-adjust.svg",
            shapes: vec![
                Shape2 {
                    outline: RoundedRect::new(14.0, 12.0, 78.0, 64.0, 6.0).to_path(1e-7),
                    rgba: hex(0x4C8DFF, 1.0),
                },
                Shape2 {
                    outline: Circle::new((74.0, 56.0), 30.0).to_path(1e-7),
                    rgba: hex(0xE83A5E, 0.7),
                },
            ],
            space: Space::LinearRgb,
            region: [0.0, 0.0, 120.0, 100.0],
            graph: |_| {
                vec![
                    Op::Matrix(Matrix::Saturate(0.3), In::Source),
                    Op::Matrix(Matrix::HueRotate(90.0), In::R(0)),
                    Op::Transfer(
                        [
                            Function::Table(vec![0.0, 1.0, 0.5]),
                            Function::Discrete(vec![0.2, 0.8]),
                            Function::Gamma {
                                amplitude: 1.2,
                                exponent: 0.7,
                                offset: 0.05,
                            },
                            Function::Identity,
                        ],
                        In::R(1),
                    ),
                    Op::Matrix(Matrix::LuminanceToAlpha, In::Source),
                    Op::Blend(Blend::Multiply, In::R(2), In::R(3)),
                    Op::Blend(Blend::Screen, In::R(4), In::Source),
                    Op::Composite(CompositeOp::Atop, In::R(5), In::Source),
                    Op::Composite(CompositeOp::Xor, In::R(6), In::R(3)),
                    Op::Composite(
                        CompositeOp::Arithmetic([0.5, 0.25, 0.5, 0.0]),
                        In::R(7),
                        In::Source,
                    ),
                ]
            },
        },
    ]
}

fn fixture_doc(file: &str) -> SvgDocument {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/svg-filters/exports")
        .join(file);
    SvgDocument::parse(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn exports_meet_the_f64_reference_closer_than_resvg() {
    for fixture in fixtures() {
        let doc = fixture_doc(fixture.file);
        assert_eq!(doc.unsupported_feature(), None, "{}", fixture.file);
        for scale in [1.0, 2.0] {
            let (w, h) = ((120.0 * scale) as usize, (100.0 * scale) as usize);
            let src = source(&fixture.shapes, scale, w, h);
            let r = fixture.region.map(|v| (v * scale).round() as i64);
            let reference = evaluate(&src, &(fixture.graph)(scale), fixture.space, r);
            let engine: Vec<f64> = painter_render(&doc, Affine::scale(scale), w as u32, h as u32)
                .unwrap_or_else(|reason| panic!("{} fell back: {reason}", fixture.file))
                .iter()
                .map(|v| f64::from(*v))
                .collect();
            let resvg: Vec<f64> = doc
                .raster_device(
                    Affine::scale(scale),
                    PixelRect::new(0, 0, w as u32, h as u32),
                    None,
                )
                .rgba
                .iter()
                .map(|v| f64::from(*v) / 255.0)
                .collect();
            let ours = closeness(&engine, &reference);
            let before = closeness(&resvg, &reference);
            eprintln!(
                "{} @{scale}x: float max {:.3} p99.9 {:.3} mean {:.4} CV; resvg max {:.2} p99.9 {:.2} mean {:.3} CV",
                fixture.file, ours[0], ours[1], ours[2], before[0], before[1], before[2]
            );
            assert!(
                ours[0] <= FLOAT_BOUND_CV,
                "{} @{scale}x: {ours:?}",
                fixture.file
            );
            for k in 0..3 {
                assert!(
                    ours[k] <= before[k],
                    "{} @{scale}x: float {ours:?} vs resvg {before:?}",
                    fixture.file
                );
            }
        }
    }
}

/// The exact Gaussian at any σ: a pixel-aligned rectangle blurred by
/// 8 to 400 device pixels, against the analytic f64 Gaussian of its box
/// reconstruction over the whole frame, with area and centroid.
#[test]
fn large_gaussians_match_the_f64_gaussian_at_any_sigma() {
    let (w, h) = (360usize, 240usize);
    for sigma in [8.0, 25.0, 60.0, 150.0, 400.0] {
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}"><filter id="b" filterUnits="userSpaceOnUse" x="-4000" y="-4000" width="8000" height="8000"><feGaussianBlur stdDeviation="{sigma}"/></filter><rect x="140" y="90" width="80" height="60" fill="#fff" filter="url(#b)"/></svg>"##
        );
        let doc = SvgDocument::parse(svg.as_bytes()).unwrap();
        let frame = painter_render(&doc, Affine::IDENTITY, w as u32, h as u32).unwrap();
        let k = kernel(sigma);
        let r = (k.len() / 2) as i64;
        let interval = |p: i64, a: i64, b: i64| -> f64 {
            (a..b)
                .map(|s| {
                    let i = r + p - s;
                    if (0..k.len() as i64).contains(&i) {
                        k[i as usize]
                    } else {
                        0.0
                    }
                })
                .sum()
        };
        let mut reference = Image::new(w, h);
        for y in 0..h {
            let wy = interval(y as i64, 90, 150);
            for x in 0..w {
                let v = interval(x as i64, 140, 220) * wy;
                reference.pixels[y * w + x] = [v; 4];
            }
        }
        let engine: Vec<f64> = frame.iter().map(|v| f64::from(*v)).collect();
        let c = closeness(&engine, &reference);
        let (mut area, mut expected) = (0.0, 0.0);
        let (mut mx, mut emx) = (0.0, 0.0);
        for (i, p) in reference.pixels.iter().enumerate() {
            area += engine[i * 4 + 3];
            expected += p[3];
            mx += engine[i * 4 + 3] * (i % w) as f64;
            emx += p[3] * (i % w) as f64;
        }
        eprintln!(
            "Gaussian σ {sigma} px: max {:.4} p99.9 {:.4} mean {:.5} CV",
            c[0], c[1], c[2]
        );
        assert!(c[0] <= FLOAT_BOUND_CV, "σ {sigma}: {c:?}");
        assert!(
            (area - expected).abs() / expected < 1e-3,
            "σ {sigma}: area {area} vs {expected}"
        );
        assert!(
            (mx / area - emx / expected).abs() < 1e-3,
            "σ {sigma}: centroid"
        );
    }
}

/// Regions clip each primitive by exact pixel area, once: a flood over a
/// fractional subregion merged with itself covers each boundary pixel by
/// its area, and the merge does not clip that pixel a second time.
#[test]
fn fractional_regions_clip_each_pixel_once_by_area() {
    let doc = SvgDocument::parse(
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="24"><filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="32" height="24" color-interpolation-filters="sRGB"><feFlood x="4.25" y="3.5" width="20.5" height="10.75" flood-color="#3366CC" flood-opacity="0.5" result="a"/><feMerge><feMergeNode in="a"/><feMergeNode in="a"/></feMerge></filter><rect width="32" height="24" fill="#000" filter="url(#f)"/></svg>"##,
    )
    .unwrap();
    let frame = painter_render(&doc, Affine::IDENTITY, 32, 24).unwrap();
    let flood = filter::flood(
        [
            0x33 as f64 / 255.0,
            0x66 as f64 / 255.0,
            0xCC as f64 / 255.0,
        ],
        0.5,
    );
    let merged = filter::merge(&[flood, flood]);
    let sub = Rect::new(4.25, 3.5, 24.75, 14.25);
    for y in 0..24 {
        for x in 0..32 {
            let pixel = Rect::new(x as f64, y as f64, (x + 1) as f64, (y + 1) as f64);
            let i = pixel.intersect(sub);
            let c = if i.width() > 0.0 && i.height() > 0.0 {
                i.area()
            } else {
                0.0
            };
            for k in 0..4 {
                let actual = f64::from(frame[(y * 32 + x) * 4 + k]);
                assert!(
                    (actual - c * merged[k]).abs() * 255.0 < 0.01,
                    "({x},{y}) channel {k}: {actual} vs {}",
                    c * merged[k]
                );
            }
        }
    }
}

/// A shadow under rotation keeps its mass (a rotation preserves area) and
/// stays native: its isotropic blur is diagonal on the rotated grid.
#[test]
fn rotated_drop_shadow_stays_native_and_keeps_its_mass() {
    let doc = fixture_doc("css-drop-shadow.svg");
    let mass = |transform: Affine| -> f64 {
        let frame = painter_render(&doc, transform, 240, 240).unwrap();
        frame.chunks_exact(4).map(|p| f64::from(p[3])).sum()
    };
    let upright = mass(Affine::translate((60.0, 70.0)));
    let rotated = mass(
        Affine::translate((120.0, 120.0)) * Affine::rotate(0.6) * Affine::translate((-60.0, -50.0)),
    );
    assert!(
        (upright - rotated).abs() / upright < 2e-3,
        "{upright} vs {rotated}"
    );
}

#[test]
fn unsupported_primitives_fall_back_with_their_name() {
    let doc = SvgDocument::parse(
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><filter id="f"><feTurbulence baseFrequency="0.05"/><feComposite in2="SourceAlpha" operator="in"/></filter><rect width="32" height="32" filter="url(#f)"/></svg>"##,
    )
    .unwrap();
    let reason = painter_render(&doc, Affine::IDENTITY, 32, 32).unwrap_err();
    assert!(reason.contains("feTurbulence"), "{reason}");
}
