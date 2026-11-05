//! E06: every saved spatial program, finite image boundaries and viewport invariance.
use super::shadow_ramp::sample;
use super::*;

fn cases() -> Vec<(&'static str, u32, Value)> {
    vec![
        ("blur", 1, json!({"amount":4})),
        ("blur", 2, json!({"amount":4,"repeatEdgePixels":false})),
        ("pixelate", 1, json!({"size":8})),
        (
            "pixelate",
            2,
            json!({"horizontalBlocks":4,"verticalBlocks":4,"sharpColors":false}),
        ),
        (
            "unsharpMask",
            1,
            json!({"amount":50,"radius":1,"threshold":0}),
        ),
        (
            "findEdges",
            1,
            json!({"invert":false,"blendWithOriginal":0}),
        ),
    ]
}
const CODE: &str = "void main(){float a=gl_FragCoord.x<16.?0.5:1.;fragColor=vec4((gl_FragCoord.x<16.?-0.25:0.75)*a,0.5*a,2.*a,a);}";
fn image() -> Vec<[f64; 4]> {
    (0..1024)
        .map(|i| {
            if i % 32 < 16 {
                [-0.125, 0.25, 1., 0.5]
            } else {
                [0.75, 0.5, 2., 1.]
            }
        })
        .collect()
}
fn blur(input: &[[f64; 4]], sigma: f64, transparent: bool) -> Vec<[f64; 4]> {
    let mut input = input.to_vec();
    for axis in [0, 1] {
        input = (0..1024)
            .map(|i| {
                let mut sum = [0.; 4];
                let mut weight = 0.;
                for tap in -(3. * sigma).ceil() as i32..=(3. * sigma).ceil() as i32 {
                    let d = f64::from(tap);
                    let w = (-0.5 * d * d / (sigma * sigma)).exp();
                    let c = sample(
                        &input,
                        (i % 32) as f64 + 0.5 + if axis == 0 { d } else { 0. },
                        (i / 32) as f64 + 0.5 + if axis == 1 { d } else { 0. },
                        transparent,
                    );
                    for k in 0..4 {
                        sum[k] += c[k] * w;
                    }
                    weight += w;
                }
                sum.map(|v| v / weight)
            })
            .collect();
    }
    input
}
#[test]
fn e06_all_saved_spatial_revisions_match_independent_hdr_alpha_edge_pixels() {
    use pm_reference::effects::{Frame, Image, Param};
    let mut renderer = Renderer::headless().unwrap();
    let original = image();
    for (id, version, params) in cases() {
        for depth in [8, 16, 32] {
            let value = fixture(id, version, params.clone(), json!({}), Some(CODE));
            let expected: Vec<[f64; 4]> = match (id, version) {
                ("blur" | "pixelate", 1) => {
                    let input = Image {
                        width: 32,
                        height: 32,
                        rgba: original.iter().flatten().copied().collect(),
                    };
                    let result = pm_reference::effects::run(
                        id,
                        &[Param::Float(if id == "blur" { 4. } else { 8. })],
                        &input,
                        &Frame::full(&input, 0.),
                        0.,
                    )
                    .unwrap();
                    result
                        .image
                        .rgba
                        .chunks_exact(4)
                        .map(|p| p.try_into().unwrap())
                        .collect()
                }
                ("blur", 2) => blur(&original, 2., true),
                ("pixelate", 2) => original.clone(), // Each 8x8 cell lies wholly within a constant patch.
                ("unsharpMask", 1) => {
                    let blurred = blur(&original, 1., true);
                    original
                        .iter()
                        .zip(blurred)
                        .map(|(o, b)| {
                            std::array::from_fn(|i| {
                                if i == 3 {
                                    o[3]
                                } else {
                                    let rgb = o[i] / o[3];
                                    let soft = if b[3] > 0. { b[i] / b[3] } else { 0. };
                                    (rgb + (rgb - soft) * 0.5) * o[3]
                                }
                            })
                        })
                        .collect()
                }
                ("findEdges", 1) => (0..1024)
                    .map(|index| {
                        let x = (index % 32) as f64 + 0.5;
                        let y = (index / 32) as f64 + 0.5;
                        let straight = |dx: f64, dy: f64| {
                            let p = sample(&original, x + dx, y + dy, true);
                            if p[3] > 0. {
                                [p[0] / p[3], p[1] / p[3], p[2] / p[3]]
                            } else {
                                [0.; 3]
                            }
                        };
                        let a = straight(-1., -1.);
                        let b = straight(0., -1.);
                        let d = straight(1., -1.);
                        let e = straight(-1., 0.);
                        let f = straight(1., 0.);
                        let g = straight(-1., 1.);
                        let h = straight(0., 1.);
                        let i = straight(1., 1.);
                        let alpha = original[index][3];
                        std::array::from_fn(|k| {
                            if k == 3 {
                                alpha
                            } else {
                                let gx = -a[k] + d[k] - 2. * e[k] + 2. * f[k] - g[k] + i[k];
                                let gy = -a[k] - 2. * b[k] - d[k] + g[k] + 2. * h[k] + i[k];
                                (1. - (gx.hypot(gy) * 0.25).clamp(0., 1.)) * alpha
                            }
                        })
                    })
                    .collect(),
                _ => panic!("unmodelled saved spatial program"),
            };
            let actual = draw(&mut renderer, &value, &MapResolver::default(), depth);
            for (index, (actual, expected)) in actual.chunks_exact(4).zip(expected).enumerate() {
                let expected = expected.map(|v| {
                    if depth == 32 {
                        v as f32 as f64
                    } else {
                        half(v as f32) as f64
                    }
                });
                for i in 0..4 {
                    assert!(
                        (f64::from(actual[i]) - expected[i]).abs() <= 1. / 1024.,
                        "{id} v{version},depth {depth},pixel {index},channel {i}: {} != {}",
                        actual[i],
                        expected[i]
                    );
                }
            }
        }
    }
}

#[test]
fn e06_all_saved_spatial_revisions_depth_crop_zoom_and_source_contracts() {
    let mut renderer = Renderer::headless().unwrap();
    for (id, version, params) in cases() {
        for depth in [8, 16, 32] {
            for source in [false, true] {
                let mut value = fixture(id, version, params.clone(), json!({}), Some(CODE));
                if source {
                    value["layers"][0]["effectSpace"] = json!("source");
                }
                if source && version == 1 && matches!(id, "blur" | "pixelate") {
                    // The pinned historical definitions lack sourceProcessing. Their
                    // refusal is part of the source boundary, never a substituted v2.
                    let mut doc = load(&value);
                    doc.set_saving_app_version(Some("1.3.0".into()));
                    renderer.set_main_effects_for_document(&doc).unwrap();
                    let result = renderer.render(
                        &doc,
                        "Ptest-root",
                        Ticks::ZERO,
                        &RenderRequest::sized(Size::new(32, 32), Precision::Float32),
                        &MapResolver::default(),
                    );
                    assert!(
                        result.is_err(),
                        "historical {id} must refuse undeclared source processing"
                    );
                } else {
                    let actual = draw(&mut renderer, &value, &MapResolver::default(), depth);
                    if depth == 32 {
                        check_float_viewport(&mut renderer, &value, &actual);
                    } else {
                        check_viewport(&mut renderer, &value, &actual);
                    }
                    let mut document = load(&value);
                    document.set_saving_app_version(Some("1.3.0".into()));
                    let full = renderer
                        .render(
                            &document,
                            "Ptest-root",
                            Ticks::ZERO,
                            &RenderRequest::sized(Size::new(64, 64), Precision::Float32),
                            &MapResolver::default(),
                        )
                        .unwrap();
                    assert!(full.failures.is_empty());
                    let full = renderer.read_rgba(&full).unwrap();
                    let mut request = RenderRequest::sized(Size::new(24, 16), Precision::Float32);
                    request.scale = Some((2., 2.));
                    request.origin = Some((16., 16.));
                    let cropped = renderer
                        .render(
                            &document,
                            "Ptest-root",
                            Ticks::ZERO,
                            &request,
                            &MapResolver::default(),
                        )
                        .unwrap();
                    assert!(cropped.failures.is_empty());
                    let actual = renderer.read_rgba(&cropped).unwrap();
                    for y in 0..16 {
                        for x in 0..24 {
                            assert_eq!(
                                &actual[(y * 24 + x) * 4..][..4],
                                &full[((y + 16) * 64 + x + 16) * 4..][..4],
                                "zoom crop {id} v{version},depth {depth},source {source},at {x},{y}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn e06_all_saved_spatial_revisions_viewer_final_and_decoded_exports() {
    let mut renderer = Renderer::headless().unwrap();
    for (id, version, params) in cases() {
        for depth in [8, 16, 32] {
            let value = fixture(id, version, params.clone(), json!({}), Some(CODE));
            super::color_contract::decoded_png(
                &mut renderer,
                &value,
                depth,
                &format!("{id} v{version}"),
            );
        }
    }
}
