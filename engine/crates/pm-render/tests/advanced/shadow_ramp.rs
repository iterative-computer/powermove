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
    sample_sized(image, 32, x, y, transparent)
}
fn sample_sized(image: &[[f64; 4]], size: usize, x: f64, y: f64, transparent: bool) -> [f64; 4] {
    if transparent && (!(0. ..=size as f64).contains(&x) || !(0. ..=size as f64).contains(&y)) {
        return [0.; 4];
    }
    let px = x - 0.5;
    let py = y - 0.5;
    let ix = px.floor();
    let iy = py.floor();
    let fx = px - ix;
    let fy = py - iy;
    let at = |dx: f64, dy: f64| {
        image[((iy + dy).clamp(0., (size - 1) as f64) as usize) * size
            + (ix + dx).clamp(0., (size - 1) as f64) as usize]
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
    source: bool,
) -> Vec<[f64; 4]> {
    // Main sourceEffectPadding grows this complete source domain before the
    // offset and both softness passes, then places it through the owner.
    let pad = if source {
        ((distance.abs() + softness.abs()) * 2.).ceil() as usize
    } else {
        0
    };
    let size = 32 + 2 * pad;
    let original: Vec<[f64; 4]> = (0..size * size)
        .map(|i| {
            if (pad..pad + 16).contains(&(i % size)) && (pad..pad + 32).contains(&(i / size)) {
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
    let mut pixels: Vec<_> = (0..size * size)
        .map(|i| {
            let alpha = sample_sized(
                &original,
                size,
                (i % size) as f64 + 0.5 - offset[0],
                (i / size) as f64 + 0.5 - offset[1],
                version == 2,
            )[3];
            [0., 0., 0., storage(alpha, depth)]
        })
        .collect();
    for axis in [0, 1] {
        pixels = (0..size * size)
            .map(|i| {
                let x = (i % size) as f64 + 0.5;
                let y = (i / size) as f64 + 0.5;
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
                        sum += sample_sized(
                            &pixels,
                            size,
                            x + if axis == 0 { d } else { 0. },
                            y + if axis == 1 { d } else { 0. },
                            version == 2,
                        )[3] * weight;
                        total += weight;
                    }
                }
                let a = sum / total;
                if axis == 0 {
                    return [0., 0., 0., storage(a, depth)];
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
    (0..32)
        .flat_map(|y| (0..32).map(move |x| (y + pad) * size + x + pad))
        .map(|i| pixels[i])
        .collect()
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
                                    source,
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

#[test]
fn e07_saved_main_ramp_v1_owner_coordinates_radial_midpoint_shade_scatter_and_depths() {
    let mut renderer = Renderer::headless().unwrap();
    for depth in [8, 16, 32] {
        for rotated in [false, true] {
            for radial in [false, true] {
                for angle in [0f64, 90., 135.] {
                    for midpoint in [25f64, 50., 75.] {
                        for dither in [0., 1.] {
                            for shade in [false, true] {
                                for amount in [0., 50., 100.] {
                                    let mut value = fixture(
                                        "gradient",
                                        1,
                                        json!({"startColor":"#FF0000","endColor":"#0000FF","centerX":25,"centerY":-25,"angle":angle,"radial":radial,"spread":75,"midpoint":midpoint,"dither":dither,"shade":shade,"amount":amount}),
                                        json!({}),
                                        Some("void main(){fragColor=vec4(0.125,0.25,1.,0.5);}"),
                                    );
                                    if rotated {
                                        set(&mut value["layers"][0], "rotation", json!(90));
                                        set(&mut value["layers"][0], "position.x", json!(32));
                                    }
                                    let actual =
                                        draw(&mut renderer, &value, &MapResolver::default(), depth);
                                    for (index, pixel) in actual.chunks_exact(4).enumerate() {
                                        let x = (index % 32) as f64 + 0.5;
                                        let y = (index / 32) as f64 + 0.5;
                                        let (x, y) = if rotated { (y, 32. - x) } else { (x, y) };
                                        let local = [x / 32. - 0.5 - 0.125, 0.5 - y / 32. - 0.125];
                                        let base = if radial {
                                            local[0].hypot(local[1]) * 2. / 0.75
                                        } else {
                                            (local[0] * angle.to_radians().cos()
                                                - local[1] * angle.to_radians().sin())
                                                / 0.75
                                                + 0.5
                                        };
                                        // Main's sin hash is unspecified at the last bits. Bound exactly
                                        // its declared [-.5,.5] jitter before clamping and midpoint power.
                                        let gamma = 0.5f64.ln() / (midpoint / 100.).ln();
                                        let lo = (base - dither * 0.002).clamp(0., 1.).powf(gamma);
                                        let hi = (base + dither * 0.002).clamp(0., 1.).powf(gamma);
                                        let mapped = |t: f64| {
                                            let mut rgb = [1. - t, 0., t];
                                            if shade {
                                                let peak = (1. - t).max(t);
                                                rgb = rgb.map(|v| v * 2. / peak);
                                            }
                                            std::array::from_fn::<_, 3, _>(|i| {
                                                storage(
                                                    ([0.25, 0.5, 2.][i]
                                                        + (rgb[i] - [0.25, 0.5, 2.][i]) * amount
                                                            / 100.)
                                                        * 0.5,
                                                    depth,
                                                )
                                            })
                                        };
                                        let a = mapped(lo);
                                        let b = mapped(hi);
                                        let middle = mapped(0.5f64.clamp(lo, hi));
                                        for channel in 0..3 {
                                            let lower =
                                                a[channel].min(b[channel]).min(middle[channel]);
                                            let upper =
                                                a[channel].max(b[channel]).max(middle[channel]);
                                            let actual = f64::from(pixel[channel]);
                                            let residual = if actual < lower {
                                                lower - actual
                                            } else if actual > upper {
                                                actual - upper
                                            } else {
                                                0.
                                            };
                                            assert!(
                                                residual <= 1. / 1024.,
                                                "main Ramp v1,depth {depth},rotated {rotated},radial {radial},angle {angle},midpoint {midpoint},dither {dither},shade {shade},amount {amount},pixel {index},channel {channel}: {actual} outside {lower}..{upper}"
                                            );
                                        }
                                        assert_eq!(pixel[3], 0.5);
                                    }
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
fn e07_shadow_layer_local_offsets_inherit_parent_rotation_scale_and_source_space() {
    let mut renderer = Renderer::headless().unwrap();
    for depth in [8, 16, 32] {
        for source in [false, true] {
            for rotated in [false, true] {
                for scale in [1f64, 2.] {
                    for only in [false, true] {
                        let mut value = fixture(
                            "shadow",
                            2,
                            json!({"angle":90,"distance":9,"softness":0,"opacity":50,"color":"#FF0000","shadowOnly":only}),
                            json!({}),
                            None,
                        );
                        value["w"] = json!(128);
                        value["h"] = json!(128);
                        value["colorDepth"] = json!(depth);
                        let owner = &mut value["layers"][0];
                        owner["d"] = json!({"w":16,"h":8,"color":"#FFFFFF"});
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
                            if rotated { 96. } else { 0. },
                            0.,
                            1.,
                        );
                        set(&mut rig, "rotation", json!(if rotated { 90 } else { 0 }));
                        value["layers"].as_array_mut().unwrap().push(rig);
                        let mut document = load(&value);
                        document.set_saving_app_version(Some("1.3.0".into()));
                        renderer.set_main_effects_for_document(&document).unwrap();
                        let frame = renderer
                            .render(
                                &document,
                                "Ptest-root",
                                Ticks::ZERO,
                                &RenderRequest::sized(Size::new(128, 128), Precision::Float32),
                                &MapResolver::default(),
                            )
                            .unwrap();
                        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
                        let pixels = renderer.read_rgba(&frame).unwrap();
                        let probe = |x: f64, y: f64| {
                            let (x, y) = if rotated {
                                (64. - y * scale, 32. + x * scale)
                            } else {
                                (32. + x * scale, 32. + y * scale)
                            };
                            pixels[((y as usize) * 128 + x as usize) * 4..][..4].to_vec()
                        };
                        assert_eq!(
                            probe(20., 4.),
                            vec![0.5, 0., 0., 0.5],
                            "local +X shadow: source {source},rotation {rotated},scale {scale},depth {depth}"
                        );
                        assert_eq!(
                            probe(4., 4.),
                            if only { vec![0.; 4] } else { vec![1.; 4] },
                            "unshadowed original"
                        );
                        assert_eq!(
                            probe(12., 4.),
                            if only {
                                vec![0.5, 0., 0., 0.5]
                            } else {
                                vec![1.; 4]
                            },
                            "overlap preserves the original or Shadow Only"
                        );
                        assert_eq!(&pixels[(5 * 128 + 5) * 4..][..4], &[0.; 4]);
                        let area = pixels.chunks_exact(4).map(|p| f64::from(p[3])).sum::<f64>();
                        let expected = if only { 64. } else { 164. } * scale * scale;
                        assert!(
                            (area - expected).abs() <= 1. / 1024.,
                            "complete shadow area source {source},rotation {rotated},scale {scale},depth {depth}: {area} != {expected}"
                        );
                    }
                }
            }
        }
    }
}
