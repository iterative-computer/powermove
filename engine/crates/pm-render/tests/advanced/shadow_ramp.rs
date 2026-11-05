//! E07: independent saved shadow/ramp pixels, including historical programs.
use super::*;

fn storage(v: f64, depth: u32) -> f64 {
    if depth == 32 {
        v as f32 as f64
    } else {
        half(v as f32) as f64
    }
}
pub(super) fn sample(image: &[[f64; 4]], x: f64, y: f64, transparent: bool) -> [f64; 4] {
    if transparent && (!(0. ..=32.).contains(&x) || !(0. ..=32.).contains(&y)) {
        return [0.; 4];
    }
    let px = x - 0.5;
    let py = y - 0.5;
    let ix = px.floor();
    let iy = py.floor();
    let fx = px - ix;
    let fy = py - iy;
    let at = |dx: f64, dy: f64| {
        image[((iy + dy).clamp(0., 31.) as usize) * 32 + (ix + dx).clamp(0., 31.) as usize]
    };
    let a = at(0., 0.);
    let b = at(1., 0.);
    let c = at(0., 1.);
    let d = at(1., 1.);
    std::array::from_fn(|i| {
        (a[i] * (1. - fx) + b[i] * fx) * (1. - fy) + (c[i] * (1. - fx) + d[i] * fx) * fy
    })
}
fn shadow_reference(
    version: u32,
    angle: f64,
    distance: f64,
    softness: f64,
    only: bool,
    depth: u32,
) -> Vec<[f64; 4]> {
    let original: Vec<[f64; 4]> = (0..1024)
        .map(|i| {
            if i % 32 < 16 {
                [0.125, 0.25, 1., 0.5]
            } else {
                [0.; 4]
            }
        })
        .collect();
    let radians = angle.to_radians();
    let offset = if version == 1 {
        [radians.cos() * distance, radians.sin() * distance]
    } else {
        [radians.sin() * distance, -radians.cos() * distance]
    };
    let mut pixels: Vec<_> = (0..1024)
        .map(|i| {
            let alpha = sample(
                &original,
                (i % 32) as f64 + 0.5 - offset[0],
                (i / 32) as f64 + 0.5 - offset[1],
                version == 2,
            )[3];
            [0., 0., 0., storage(alpha, depth)]
        })
        .collect();
    for axis in [0, 1] {
        pixels = (0..1024)
            .map(|i| {
                let x = (i % 32) as f64 + 0.5;
                let y = (i / 32) as f64 + 0.5;
                let sigma = if version == 1 {
                    softness.max(0.5) * 0.5
                } else {
                    softness * 0.5
                };
                let mut sum = 0.;
                let mut total = 0.;
                if version == 2 && sigma < 0.01 {
                    sum = pixels[i][3];
                    total = 1.;
                } else {
                    let taps = if version == 1 {
                        18
                    } else {
                        (3. * sigma).ceil().clamp(1., 32.) as i32
                    };
                    for tap in -taps..=taps {
                        let d = if version == 1 {
                            tap as f64 / 18. * softness.max(0.5)
                        } else {
                            tap as f64
                        };
                        let weight = if version == 1 {
                            (-d * d / (2. * sigma * sigma + 1e-4)).exp()
                        } else {
                            (-0.5 * d * d / (sigma * sigma)).exp()
                        };
                        sum += sample(
                            &pixels,
                            x + if axis == 0 { d } else { 0. },
                            y + if axis == 1 { d } else { 0. },
                            version == 2,
                        )[3] * weight;
                        total += weight;
                    }
                }
                let a = storage(sum / total, depth);
                if axis == 0 {
                    return [0., 0., 0., a];
                }
                // All test shadows are red, with 50% opacity. Alpha remains
                // premultiplied and the original contains HDR blue.
                let alpha = (a * 0.5).clamp(0., 1.);
                if only {
                    return [storage(alpha, depth), 0., 0., storage(alpha, depth)];
                }
                let original = original[i];
                [
                    storage(original[0] + alpha * (1. - original[3]), depth),
                    storage(original[1], depth),
                    storage(original[2], depth),
                    storage(original[3] + alpha * (1. - original[3]), depth),
                ]
            })
            .collect();
    }
    pixels
}
#[test]
fn e07_saved_shadow_revisions_offset_softness_only_alpha_hdr_and_crop() {
    let mut renderer = Renderer::headless().unwrap();
    for version in [1, 2] {
        for source in [false, true] {
            for depth in [8, 16, 32] {
                for angle in [0., 90., 135.] {
                    for softness in [0., 4.] {
                        for only in [false, true] {
                            let mut value = fixture(
                                "shadow",
                                version,
                                json!({"angle":angle,"distance":3.,"softness":softness,"color":"#FF0000","opacity":50,"shadowOnly":only}),
                                json!({}),
                                Some(
                                    "void main(){fragColor=gl_FragCoord.x<16.?vec4(0.125,0.25,1.,0.5):vec4(0.);}",
                                ),
                            );
                            if source {
                                value["layers"][0]["effectSpace"] = json!("source");
                            }
                            if source && version == 1 {
                                let mut document = load(&value);
                                document.set_saving_app_version(Some("1.3.0".into()));
                                renderer.set_main_effects_for_document(&document).unwrap();
                                assert!(
                                    renderer
                                        .render(
                                            &document,
                                            "Ptest-root",
                                            Ticks::ZERO,
                                            &RenderRequest::sized(
                                                Size::new(32, 32),
                                                Precision::Float32
                                            ),
                                            &MapResolver::default()
                                        )
                                        .is_err(),
                                    "historical shadow has no sourceProcessing contract"
                                );
                            } else {
                                let actual =
                                    draw(&mut renderer, &value, &MapResolver::default(), depth);
                                let expected = shadow_reference(
                                    version,
                                    angle,
                                    3.,
                                    softness,
                                    version == 2 && only,
                                    depth,
                                );
                                for (index, (a, e)) in
                                    actual.chunks_exact(4).zip(expected).enumerate()
                                {
                                    for i in 0..4 {
                                        assert!(
                                            (a[i] as f64 - e[i]).abs() <= 1. / 1024.,
                                            "shadow v{version},depth {depth},angle {angle},softness {softness},only {only},pixel {index},channel {i}: {} != {}",
                                            a[i],
                                            e[i]
                                        );
                                    }
                                }
                                if depth == 32 {
                                    check_float_viewport(&mut renderer, &value, &actual);
                                } else {
                                    check_viewport(&mut renderer, &value, &actual);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn e07_saved_current_ramp_source_dimensions_linear_radial_blend_and_reset_points() {
    let mut renderer = Renderer::headless().unwrap();
    let definition = pm_fx_defs::catalog::main_catalog()
        .get("gradient", 2)
        .unwrap();
    for depth in [8, 16, 32] {
        for radial in [false, true] {
            for source in [false, true] {
                for blend in [0., 50., 100.] {
                    let mut params: serde_json::Map<String, Value> = definition
                        .params
                        .iter()
                        .map(|p| {
                            (
                                p.k.clone(),
                                pm_fx_defs::catalog::parameter_default(p, 32., 32.),
                            )
                        })
                        .collect();
                    assert_eq!(params["startX"], json!(16.));
                    assert_eq!(params["endX"], json!(16.));
                    assert_eq!(params["startY"], json!(0.));
                    assert_eq!(params["endY"], json!(32.));
                    params.insert("rampShape".into(), json!(u32::from(radial)));
                    params.insert("blendWithOriginal".into(), json!(blend));
                    let mut value = fixture(
                        "gradient",
                        2,
                        Value::Object(params),
                        json!({}),
                        Some("void main(){fragColor=vec4(0.125,0.25,1.,0.5);}"),
                    );
                    if source {
                        value["layers"][0]["effectSpace"] = json!("source");
                    }
                    let actual = draw(&mut renderer, &value, &MapResolver::default(), depth);
                    for (index, p) in actual.chunks_exact(4).enumerate() {
                        let x = (index % 32) as f64 + 0.5;
                        let y = (index / 32) as f64 + 0.5;
                        let t = if radial {
                            ((x - 16.).hypot(y)) / 32.
                        } else {
                            y / 32.
                        }
                        .clamp(0., 1.);
                        let expected = [0.25, 0.5, 2.].map(|original| {
                            storage(
                                (t * (1. - blend / 100.) + original * blend / 100.) * 0.5,
                                depth,
                            )
                        });
                        for i in 0..3 {
                            assert!(
                                (p[i] as f64 - expected[i]).abs() <= 1. / 1024.,
                                "ramp depth {depth},source {source},radial {radial},blend {blend},pixel {index}: {p:?} != {expected:?}"
                            );
                        }
                        assert_eq!(p[3], 0.5);
                    }
                    if depth == 32 {
                        check_float_viewport(&mut renderer, &value, &actual);
                    } else {
                        check_viewport(&mut renderer, &value, &actual);
                    }
                }
            }
        }
    }
}
