//! E06: every saved spatial program, finite image boundaries and viewport invariance.
use super::shadow_ramp::sample;
use super::*;

fn cases() -> Vec<(&'static str, u32, Value)> {
    vec![
        ("blur", 1, json!({"amount":4})),
        ("blur", 2, json!({"amount":4,"repeatEdgePixels":false})),
        (
            "blur",
            2,
            json!({"amount":4,"dimensions":1,"repeatEdgePixels":true}),
        ),
        (
            "blur",
            2,
            json!({"amount":4,"dimensions":2,"repeatEdgePixels":true}),
        ),
        ("blur", 2, json!({"amount":0,"repeatEdgePixels":false})),
        ("pixelate", 1, json!({"size":8})),
        (
            "pixelate",
            2,
            json!({"horizontalBlocks":4,"verticalBlocks":4,"sharpColors":false}),
        ),
        (
            "pixelate",
            2,
            json!({"horizontalBlocks":4,"verticalBlocks":4,"sharpColors":true}),
        ),
        (
            "unsharpMask",
            1,
            json!({"amount":50,"radius":1,"threshold":0}),
        ),
        (
            "unsharpMask",
            1,
            json!({"amount":50,"radius":1,"threshold":255}),
        ),
        (
            "unsharpMask",
            1,
            json!({"amount":0,"radius":1,"threshold":0}),
        ),
        (
            "unsharpMask",
            1,
            json!({"amount":50,"radius":0,"threshold":0}),
        ),
        (
            "findEdges",
            1,
            json!({"invert":false,"blendWithOriginal":0}),
        ),
        (
            "findEdges",
            1,
            json!({"invert":true,"blendWithOriginal":50}),
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
fn blur(input: &[[f64; 4]], sigma: f64, transparent: bool, dimensions: u64) -> Vec<[f64; 4]> {
    let mut input = input.to_vec();
    if sigma < 0.01 {
        return input;
    }
    for axis in [0, 1]
        .into_iter()
        .filter(|&axis| dimensions == 0 || dimensions == axis + 1)
    {
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
                ("blur", 2) => blur(
                    &original,
                    params["amount"].as_f64().unwrap() / 2.,
                    params["repeatEdgePixels"] != true,
                    params["dimensions"].as_u64().unwrap_or(0),
                ),
                ("pixelate", 2) => original.clone(), // Each 8x8 cell lies wholly within a constant patch.
                ("unsharpMask", 1) => {
                    let radius = params["radius"].as_f64().unwrap();
                    let amount = params["amount"].as_f64().unwrap() / 100.;
                    let threshold = params["threshold"].as_f64().unwrap() / 255.;
                    let blurred = blur(&original, radius, true, 0);
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
                                    let delta = rgb - soft;
                                    (rgb + if delta.abs() >= threshold {
                                        delta * amount
                                    } else {
                                        0.
                                    }) * o[3]
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
                                let edge = (gx.hypot(gy) * 0.25).clamp(0., 1.);
                                let mapped = if params["invert"] == true {
                                    edge
                                } else {
                                    1. - edge
                                };
                                let blend = params["blendWithOriginal"].as_f64().unwrap() / 100.;
                                (mapped + (original[index][k] / alpha - mapped) * blend) * alpha
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
                value["colorDepth"] = json!(depth);
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
                    eprintln!(
                        "spatial crop {id} v{version},source {source},depth {depth},params {params}"
                    );
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

#[test]
fn e06_wide_saved_gaussian_and_mosaic_kernels_keep_hdr_alpha_and_crop_zoom() {
    let mut renderer = Renderer::headless().unwrap();
    let input = [-0.125, 0.25, 1., 0.5];
    let mut variants = vec![
        ("blur", 1, json!({"amount":96})),
        ("pixelate", 1, json!({"size":96})),
    ];
    for dimensions in 0..3 {
        variants.push((
            "blur",
            2,
            json!({"amount":96,"dimensions":dimensions,"repeatEdgePixels":true}),
        ));
    }
    for sharp in [false, true] {
        variants.push((
            "pixelate",
            2,
            json!({"horizontalBlocks":3,"verticalBlocks":5,"sharpColors":sharp}),
        ));
    }
    for (id, version, params) in variants {
        for depth in [8, 16, 32] {
            let value = super::color_contract::pixel_fixture(id, version, params.clone(), input);
            let actual = draw(&mut renderer, &value, &MapResolver::default(), depth);
            super::color_contract::assert_pixels(
                &actual,
                input,
                &format!("wide {id} v{version},depth {depth},params {params}"),
            );
            if depth == 32 {
                check_float_viewport(&mut renderer, &value, &actual);
            } else {
                check_viewport(&mut renderer, &value, &actual);
            }
            let mut value = value;
            value["colorDepth"] = json!(depth);
            let mut document = load(&value);
            document.set_saving_app_version(Some("1.3.0".into()));
            let mut request = RenderRequest::sized(Size::new(24, 16), Precision::Float32);
            request.scale = Some((2., 2.));
            request.origin = Some((16., 16.));
            let frame = renderer
                .render(
                    &document,
                    "Ptest-root",
                    Ticks::ZERO,
                    &request,
                    &MapResolver::default(),
                )
                .unwrap();
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            for p in renderer.read_rgba(&frame).unwrap().chunks_exact(4) {
                for channel in 0..4 {
                    assert!(
                        (f64::from(p[channel]) - input[channel]).abs() <= 1. / 1024.,
                        "wide cropped zoom {id} v{version},depth {depth},channel {channel}"
                    );
                }
            }
        }
    }
}

// Independent bilinear taps of the pinned Mosaic program. Composition-space
// input retains transparent pixels beyond the owner; a local source target
// clamps its own edge. Uniform taps, rather than an idealized area integral,
// therefore give different edge-cell averages in these two spaces.
fn mosaic_cell_reference(
    source: bool,
    rotated: bool,
    scale: f64,
    sharp: bool,
    world: [f64; 2],
    depth: u32,
) -> [f64; 4] {
    let owner = |x: f64, y: f64| {
        if rotated {
            [(y - 32.) / scale, (96. - x) / scale]
        } else {
            [(x - 32.) / scale, (y - 32.) / scale]
        }
    };
    let size = if source { 32 } else { 128 };
    let image: Vec<[f64; 4]> = (0..size * size)
        .map(|i| {
            let p = if source {
                [(i % size) as f64 + 0.5, (i / size) as f64 + 0.5]
            } else {
                owner((i % size) as f64 + 0.5, (i / size) as f64 + 0.5)
            };
            if p.iter().any(|v| *v < 0. || *v >= 32.) {
                [0.; 4]
            } else if p[0] < 16. {
                [-0.125, 0.25, 1., 0.5]
            } else {
                [0.75, 0.5, 2., 1.]
            }
        })
        .collect();
    let q = owner(world[0], world[1]);
    let cell = [(q[0] / 32. * 3.).floor(), (q[1] / 32. * 5.).floor()];
    let density = if source { 1. } else { scale };
    let samples = if sharp {
        [1, 1]
    } else {
        [
            (32. * density / 3.).ceil() as usize,
            (32. * density / 5.).ceil() as usize,
        ]
    };
    let mut sum = [0.; 4];
    for y in 0..samples[1] {
        for x in 0..samples[0] {
            let local = [
                (cell[0] + (x as f64 + 0.5) / samples[0] as f64) / 3. * 32.,
                (cell[1] + (y as f64 + 0.5) / samples[1] as f64) / 5. * 32.,
            ];
            let p = if source {
                local
            } else if rotated {
                [96. - local[1] * scale, 32. + local[0] * scale]
            } else {
                [32. + local[0] * scale, 32. + local[1] * scale]
            };
            let tap = super::shadow_ramp::sample_sized(&image, size, p[0], p[1], false);
            for k in 0..4 {
                sum[k] += tap[k];
            }
        }
    }
    super::color_contract::quantize(sum.map(|v| v / (samples[0] * samples[1]) as f64), depth)
}

#[test]
fn e06_mosaic_cells_follow_translated_rotated_scaled_owner_and_partial_alpha_hdr() {
    let mut renderer = Renderer::headless().unwrap();
    let code = "void main(){float a=gl_FragCoord.x<iResolution.x*.5?.5:1.;fragColor=vec4((gl_FragCoord.x<iResolution.x*.5?-.25:.75)*a,.5*a,2.*a,a);}";
    for depth in [8, 16, 32] {
        for source in [false, true] {
            for rotated in [false, true] {
                for scale in [1f64, 2.] {
                    for sharp in [false, true] {
                        let mut value = fixture(
                            "pixelate",
                            2,
                            json!({"horizontalBlocks":3,"verticalBlocks":5,"sharpColors":sharp}),
                            json!({}),
                            Some(code),
                        );
                        value["w"] = json!(128);
                        value["h"] = json!(128);
                        value["colorDepth"] = json!(depth);
                        let owner = &mut value["layers"][0];
                        owner["parent"] = json!("rig");
                        if source {
                            owner["effectSpace"] = json!("source");
                        }
                        set(owner, "position.x", json!(32));
                        set(owner, "position.y", json!(32));
                        set(owner, "scale.x", json!(scale * 100.));
                        set(owner, "scale.y", json!(scale * 100.));
                        let mut rig = layer(
                            "rig",
                            "null",
                            json!({}),
                            if rotated { 128. } else { 0. },
                            0.,
                            1.,
                        );
                        set(&mut rig, "rotation", json!(if rotated { 90 } else { 0 }));
                        value["layers"].as_array_mut().unwrap().push(rig);
                        let mut document = load(&value);
                        document.set_saving_app_version(Some("1.3.0".into()));
                        renderer.set_main_effects_for_document(&document).unwrap();
                        let request = RenderRequest::sized(Size::new(128, 128), Precision::Float32);
                        let frame = renderer
                            .render(
                                &document,
                                "Ptest-root",
                                Ticks::ZERO,
                                &request,
                                &MapResolver::default(),
                            )
                            .unwrap();
                        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
                        let full = renderer.read_rgba(&frame).unwrap();
                        for x in [5., 16., 26.] {
                            let (px, py) = if rotated {
                                (96. - 16. * scale, 32. + x * scale)
                            } else {
                                (32. + x * scale, 32. + 16. * scale)
                            };
                            let expected = mosaic_cell_reference(
                                source,
                                rotated,
                                scale,
                                sharp,
                                [px.floor() + 0.5, py.floor() + 0.5],
                                depth,
                            );
                            let p = &full[((py as usize) * 128 + px as usize) * 4..][..4];
                            for k in 0..4 {
                                assert!(
                                    (f64::from(p[k]) - expected[k]).abs() <= 1. / 1024.,
                                    "owner-local Mosaic cell at {x},depth {depth},source {source},rotated {rotated},scale {scale},sharp {sharp},channel {k}: {} != {}",
                                    p[k],
                                    expected[k]
                                );
                            }
                        }
                        let mut crop = RenderRequest::sized(Size::new(16, 16), Precision::Float32);
                        crop.scale = Some((1., 1.));
                        crop.origin = Some((48., 40.));
                        let frame = renderer
                            .render(
                                &document,
                                "Ptest-root",
                                Ticks::ZERO,
                                &crop,
                                &MapResolver::default(),
                            )
                            .unwrap();
                        assert!(frame.failures.is_empty());
                        for (index, p) in renderer
                            .read_rgba(&frame)
                            .unwrap()
                            .chunks_exact(4)
                            .enumerate()
                        {
                            assert_eq!(
                                p,
                                &full[((40 + index / 16) * 128 + 48 + index % 16) * 4..][..4],
                                "transformed Mosaic crop"
                            );
                        }
                    }
                }
            }
        }
    }
}
