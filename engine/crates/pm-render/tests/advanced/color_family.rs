//! E05 independent scalar equations from the pinned color-correction,
//! core-image and tonal definitions. Shader translation is not the oracle.
use super::color_contract::{assert_pixels, from_hls, hls, pixel_fixture, quantize};
use super::*;
fn key(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.into()
    } else {
        format!("{prefix}{}{}", name[..1].to_uppercase(), &name[1..])
    }
}
fn reference(id: &str, version: u32, p: &Value, c: [f64; 4]) -> [f64; 4] {
    let n = |k: &str| {
        p[k].as_f64()
            .or_else(|| p[k].as_bool().map(|v| u32::from(v) as f64))
            .unwrap()
    };
    let color = |k: &str| {
        let s = p[k].as_str().unwrap().trim_start_matches('#');
        std::array::from_fn::<_, 3, _>(|i| {
            u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap() as f64 / 255.
        })
    };
    let luma = |rgb: [f64; 3]| rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
    if version == 1 && matches!(id, "invert" | "levels") && c[3] < 0.0005 {
        return c;
    }
    if c[3] <= 0.
        && !matches!(
            id,
            "invert" | "levels" | "levelsIndividual" | "shiftChannels"
        )
    {
        return [0.; 4];
    }
    let rgb = if c[3] > 0. {
        [c[0] / c[3], c[1] / c[3], c[2] / c[3]]
    } else {
        [0.; 3]
    };
    let mut alpha = c[3];
    let result = match (id, version) {
        ("invert", 1) => rgb.map(|v| v + (1. - 2. * v) * n("amount") / 100.),
        ("levels", 1) => rgb.map(|v| {
            ((v - n("inBlack") / 100.) / (n("inWhite") / 100. - n("inBlack") / 100.).max(1e-3))
                .clamp(0., 1.)
                .powf(1. / n("gamma").max(0.01))
        }),
        ("tint", 1) => {
            let lo = color("mapBlackTo");
            let hi = color("mapWhiteTo");
            let y = luma(rgb);
            std::array::from_fn(|i| {
                rgb[i] + (lo[i] + (hi[i] - lo[i]) * y - rgb[i]) * n("amountToTint") / 100.
            })
        }
        ("channelMixer", 1) => {
            let mut mixed = ["red", "green", "blue"].map(|o| {
                let values = ["Red", "Green", "Blue"].map(|i| n(&format!("{o}{i}")));
                (values.iter().zip(rgb).map(|(a, b)| a * b).sum::<f64>() + n(&format!("{o}Const")))
                    / 100.
            });
            if n("monochrome") > 0.5 {
                mixed = [mixed[0]; 3];
            }
            mixed.map(|v| v.clamp(0., 1.))
        }
        ("invert", 2) => {
            let channel = n("channel").clamp(0., 12.).round() as usize;
            let mut out = rgb;
            match channel {
                0 => out = rgb.map(|v| 1. - v),
                1..=3 => out[channel - 1] = 1. - out[channel - 1],
                4..=7 => {
                    let mut v = hls(rgb.map(|v| v.clamp(0., 1.)));
                    if channel == 4 || channel == 5 {
                        v[0] = (1. - v[0]).rem_euclid(1.);
                    }
                    if channel == 4 || channel == 6 {
                        v[1] = 1. - v[1];
                    }
                    if channel == 4 || channel == 7 {
                        v[2] = 1. - v[2];
                    }
                    out = from_hls(v);
                }
                8..=11 => {
                    let mut v = [
                        rgb[0] * 0.299 + rgb[1] * 0.587 + rgb[2] * 0.114,
                        rgb[0] * 0.596 - rgb[1] * 0.274 - rgb[2] * 0.322,
                        rgb[0] * 0.211 - rgb[1] * 0.523 + rgb[2] * 0.312,
                    ];
                    if channel == 8 || channel == 9 {
                        v[0] = 1. - v[0];
                    }
                    if channel == 8 || channel == 10 {
                        v[1] = -v[1];
                    }
                    if channel == 8 || channel == 11 {
                        v[2] = -v[2];
                    }
                    out = [
                        v[0] + 0.956 * v[1] + 0.621 * v[2],
                        v[0] - 0.272 * v[1] - 0.647 * v[2],
                        v[0] - 1.106 * v[1] + 1.703 * v[2],
                    ];
                }
                _ => alpha = 1. - alpha,
            }
            let amount = n("blendWithOriginal") / 100.;
            return std::array::from_fn(|i| {
                let value = if i == 3 { alpha } else { out[i] * alpha };
                value + (c[i] - value) * amount
            });
        }
        ("levels", 2) | ("levelsIndividual", 1) => {
            let level = |x: f64, prefix: &str| {
                let black = n(&key(prefix, "inBlack"));
                let span = n(&key(prefix, "inWhite")) - black;
                let mut t = if span.abs() < 1e-7 {
                    if x >= black { 1. } else { 0. }
                } else {
                    (x - black) / span
                };
                if n("clipBlack") > 0.5 {
                    t = t.max(0.);
                }
                if n("clipWhite") > 0.5 {
                    t = t.min(1.);
                }
                let black = n(&key(prefix, "outBlack"));
                black
                    + (n(&key(prefix, "outWhite")) - black)
                        * t.signum()
                        * t.abs().powf(1. / n(&key(prefix, "gamma")).max(0.01))
            };
            alpha = level(alpha, "alpha").clamp(0., 1.);
            std::array::from_fn(|i| level(level(rgb[i], ""), ["red", "green", "blue"][i]))
        }
        ("threshold", 1) => {
            [if luma(rgb) >= n("level") / 255. {
                1.
            } else {
                0.
            }; 3]
        }
        ("shiftChannels", 1) => {
            let v = hls(rgb.map(|v| v.clamp(0., 1.)));
            let choices = [
                c[3],
                rgb[0],
                rgb[1],
                rgb[2],
                luma(rgb),
                v[0],
                v[1],
                v[2],
                1.,
                0.,
            ];
            let choose = |k: &str| choices[n(k).clamp(0., 9.).round() as usize];
            alpha = choose("takeAlphaFrom").clamp(0., 1.);
            ["takeRedFrom", "takeGreenFrom", "takeBlueFrom"].map(choose)
        }
        ("exposure", 1) => {
            use pm_color::effect_transforms::{Profile, decode_transfer, encode_transfer};
            let expose = |x: f64, prefix: &str| {
                let v = x * 2f64.powf(n(&key(prefix, "exposure"))) + n(&key(prefix, "offset"));
                v.signum() * v.abs().powf(1. / n(&key(prefix, "gamma")).max(0.01))
            };
            std::array::from_fn(|i| {
                let v = if n("bypassLinearLight") > 0.5 {
                    rgb[i]
                } else {
                    decode_transfer(rgb[i], Profile::Srgb)
                };
                let v = expose(expose(v, ""), ["red", "green", "blue"][i]);
                if n("bypassLinearLight") > 0.5 {
                    v
                } else {
                    encode_transfer(v, Profile::Srgb)
                }
            })
        }
        ("tritone", 1) => {
            let l = luma(rgb);
            let (a, b, t) = if l < 0.5 {
                (color("shadows"), color("midtones"), l * 2.)
            } else {
                (color("midtones"), color("highlights"), l * 2. - 1.)
            };
            std::array::from_fn(|i| {
                let mapped = a[i] + (b[i] - a[i]) * t;
                mapped + (rgb[i] - mapped) * n("blendWithOriginal") / 100.
            })
        }
        ("colorBalanceHLS", 1) => {
            if n("hue") == 0. && n("lightness") == 0. && n("saturation") == 0. {
                return c;
            }
            let mut v = hls(rgb.map(|v| v.clamp(0., 1.)));
            v[0] = (v[0] + n("hue") / 360.).rem_euclid(1.);
            let l = n("lightness") / 100.;
            v[1] = if l >= 0. {
                v[1] + (1. - v[1]) * l
            } else {
                v[1] * (1. + l)
            };
            v[2] = (v[2] * (1. + n("saturation") / 100.)).clamp(0., 1.);
            from_hls(v)
        }
        ("brightnessContrast", 1) => {
            if n("brightness") == 0. && n("contrast") == 0. {
                return c;
            }
            rgb.map(|v| {
                if n("useLegacy") > 0.5 {
                    (v - 0.5) * (1. + n("contrast") / 100.) + 0.5 + n("brightness") / 100.
                } else {
                    let b = n("brightness") / 150.;
                    let v = if b >= 0. {
                        v + (1. - v) * b
                    } else {
                        v * (1. + b)
                    };
                    let contrast = n("contrast") / 100.;
                    let slope = if contrast >= 0. {
                        1. / (1. - contrast).max(0.01)
                    } else {
                        1. + contrast
                    };
                    (v - 0.5) * slope + 0.5
                }
            })
        }
        ("blackWhite", 1) => {
            let bounded = rgb.map(|v| v.clamp(0., 1.));
            let hue = hls(bounded)[0].rem_euclid(1.) * 6.;
            let weights = ["reds", "yellows", "greens", "cyans", "blues", "magentas"].map(n);
            let sector = hue.floor() as usize;
            let weight =
                weights[sector] + (weights[(sector + 1) % 6] - weights[sector]) * hue.fract();
            let low = bounded.into_iter().fold(f64::INFINITY, f64::min);
            let high = bounded.into_iter().fold(f64::NEG_INFINITY, f64::max);
            let gray = low + (high - low) * weight / 100.;
            if n("tint") > 0.5 {
                let v = hls(color("tintColor"));
                from_hls([v[0], gray.clamp(0., 1.), v[2]])
            } else {
                [gray; 3]
            }
        }
        _ => panic!("unmodelled color revision {id} v{version}"),
    };
    [
        result[0] * alpha,
        result[1] * alpha,
        result[2] * alpha,
        alpha,
    ]
}
fn cases() -> Vec<(&'static str, u32, Value)> {
    let mut cases = vec![];
    for (id, version, custom) in [
        (
            "tint",
            1,
            json!({"mapBlackTo":"#FF0000","mapWhiteTo":"#0000FF","amountToTint":50}),
        ),
        (
            "channelMixer",
            1,
            json!({"redRed":50,"redGreen":-50,"redBlue":100,"redConst":25,"greenRed":100,"greenGreen":0,"blueConst":-50,"monochrome":false}),
        ),
        ("invert", 1, json!({"amount":50})),
        ("levels", 1, json!({"inBlack":20,"inWhite":80,"gamma":2})),
        (
            "tritone",
            1,
            json!({"shadows":"#FF0000","midtones":"#00FF00","highlights":"#0000FF","blendWithOriginal":50}),
        ),
        (
            "colorBalanceHLS",
            1,
            json!({"hue":-90,"lightness":-25,"saturation":50}),
        ),
        (
            "brightnessContrast",
            1,
            json!({"brightness":-75,"contrast":50,"useLegacy":false}),
        ),
        (
            "blackWhite",
            1,
            json!({"reds":-50,"yellows":200,"greens":30,"cyans":-20,"blues":80,"magentas":250,"tint":true,"tintColor":"#D7B68C"}),
        ),
        ("threshold", 1, json!({"level":64})),
        (
            "exposure",
            1,
            json!({"exposure":1,"offset":-0.125,"gamma":2,"redExposure":-1,"greenOffset":0.25,"blueGamma":0.5,"bypassLinearLight":false}),
        ),
        (
            "levels",
            2,
            json!({"inBlack":0.25,"inWhite":0.75,"gamma":2,"outBlack":-0.25,"outWhite":1.5,"redGamma":0.5,"blueOutWhite":2,"alphaOutBlack":0.25,"alphaOutWhite":0.75,"clipBlack":false,"clipWhite":false}),
        ),
        (
            "levelsIndividual",
            1,
            json!({"inBlack":0.25,"inWhite":0.75,"gamma":2,"outBlack":-0.25,"outWhite":1.5,"redGamma":0.5,"blueOutWhite":2,"alphaOutBlack":0.25,"alphaOutWhite":0.75,"clipBlack":false,"clipWhite":false}),
        ),
    ] {
        cases.extend([(id, version, json!({})), (id, version, custom)]);
    }
    for channel in 0..13 {
        for blend in [0., 50.] {
            cases.push((
                "invert",
                2,
                json!({"channel":channel,"blendWithOriginal":blend}),
            ));
        }
    }
    for output in [
        "takeAlphaFrom",
        "takeRedFrom",
        "takeGreenFrom",
        "takeBlueFrom",
    ] {
        for input in 0..10 {
            cases.push(("shiftChannels", 1, json!({(output):input})));
        }
    }
    cases.extend([
        (
            "channelMixer",
            1,
            json!({"redRed":-100,"redBlue":200,"monochrome":true}),
        ),
        (
            "colorBalanceHLS",
            1,
            json!({"hue":90,"lightness":50,"saturation":-50}),
        ),
        (
            "brightnessContrast",
            1,
            json!({"brightness":25,"contrast":-25,"useLegacy":true}),
        ),
        (
            "exposure",
            1,
            json!({"exposure":1,"offset":-0.125,"gamma":2,"bypassLinearLight":true}),
        ),
    ]);
    for id in ["levels", "levelsIndividual"] {
        for clip_black in [false, true] {
            for clip_white in [false, true] {
                cases.push((id,if id=="levels"{2}else{1},json!({"inBlack":0.5,"inWhite":0.5,"clipBlack":clip_black,"clipWhite":clip_white})));
            }
        }
    }
    cases
}
#[test]
fn e05_every_saved_color_family_all_depths_signed_hdr_alpha_and_controls() {
    let mut renderer = Renderer::headless().unwrap();
    for (id, version, custom) in cases() {
        let definition = pm_fx_defs::catalog::main_catalog()
            .get(id, u64::from(version))
            .unwrap();
        let mut params: serde_json::Map<String, Value> = definition
            .params
            .iter()
            .map(|p| (p.k.clone(), p.def.clone()))
            .collect();
        params.extend(custom.as_object().unwrap().clone());
        let params = Value::Object(params);
        for depth in [8, 16, 32] {
            for pixel in [
                [-0.125, 0.25, 1., 0.5],
                [0., 0.25, 0.5, 0.],
                [0., 0., 0., 0.],
                [0.75, 0.125, 0.5, 1.],
            ] {
                let value = pixel_fixture(id, version, params.clone(), pixel);
                let expected = quantize(
                    reference(id, version, &params, quantize(pixel, depth)),
                    depth,
                );
                assert_pixels(
                    &draw(&mut renderer, &value, &MapResolver::default(), depth),
                    expected,
                    &format!("{id} v{version},depth {depth},pixel {pixel:?},params {params}"),
                );
            }
        }
    }
}
#[test]
fn e05_black_white_all_six_hue_sectors_and_tint_keep_premultiplied_alpha() {
    let mut renderer = Renderer::headless().unwrap();
    for (id, version, custom) in cases().into_iter().filter(|(id, _, _)| *id == "blackWhite") {
        let def = pm_fx_defs::catalog::main_catalog()
            .get(id, u64::from(version))
            .unwrap();
        let mut params: serde_json::Map<String, Value> = def
            .params
            .iter()
            .map(|p| (p.k.clone(), p.def.clone()))
            .collect();
        params.extend(custom.as_object().unwrap().clone());
        let params = Value::Object(params);
        for depth in [8, 16, 32] {
            for rgb in [
                [1., 0., 0.],
                [1., 1., 0.],
                [0., 1., 0.],
                [0., 1., 1.],
                [0., 0., 1.],
                [1., 0., 1.],
            ] {
                let input = [rgb[0] * 0.5, rgb[1] * 0.5, rgb[2] * 0.5, 0.5];
                let value = pixel_fixture(id, version, params.clone(), input);
                let expected = quantize(reference(id, version, &params, input), depth);
                assert_pixels(
                    &draw(&mut renderer, &value, &MapResolver::default(), depth),
                    expected,
                    "Black & White hue sector",
                );
            }
        }
    }
}
#[test]
fn e05_every_saved_color_family_viewer_final_and_decoded_output() {
    let mut renderer = Renderer::headless().unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for (id, version, params) in cases() {
        if seen.insert((id, version)) {
            for depth in [8, 16, 32] {
                let value = pixel_fixture(id, version, params.clone(), [-0.125, 0.25, 1., 0.5]);
                super::color_contract::decoded_png(
                    &mut renderer,
                    &value,
                    depth,
                    &format!("{id} v{version}"),
                );
            }
        }
    }
    assert_eq!(seen.len(), 14, "the complete saved color-family inventory");
}

#[test]
fn e05_saved_color_channels_survive_document_serialization_and_reverse_seek() {
    let mut renderer = Renderer::headless().unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for (id, version, _) in cases() {
        if !seen.insert((id, version)) {
            continue;
        }
        let definition = pm_fx_defs::catalog::main_catalog()
            .get(id, u64::from(version))
            .unwrap();
        let params: serde_json::Map<String, Value> = definition
            .params
            .iter()
            .map(|p| (p.k.clone(), p.def.clone()))
            .collect();
        let animated = definition
            .params
            .iter()
            .find(|p| p.def.as_f64().is_some())
            .unwrap();
        let a = animated.def.as_f64().unwrap();
        let b = a + 0.25;
        for depth in [8, 16, 32] {
            let input = [-0.125, 0.25, 1., 0.5];
            let mut value = pixel_fixture(id, version, Value::Object(params.clone()), input);
            value["dur"] = json!(3);
            value["layers"][0]["dur"] = json!(3);
            value["colorDepth"] = json!(depth);
            value["layers"][0]["fx"][0]["p"][&animated.k] =
                keys(json!(a), &[(0., json!(a)), (2., json!(b))]);
            let document = load(&value);
            let saved = pm_model::json::to_canonical_string(&document.to_value().unwrap());
            let mut reopened = load(&serde_json::from_str(&saved).unwrap());
            reopened.set_saving_app_version(Some("1.3.0".into()));
            renderer.set_main_effects_for_document(&reopened).unwrap();
            assert_eq!(
                reopened.to_value(),
                document.to_value(),
                "{id} v{version}: authored channels"
            );
            for time in [0., 2., 0., 2.] {
                let mut evaluated = params.clone();
                evaluated.insert(animated.k.clone(), json!(if time == 0. { a } else { b }));
                let frame = renderer
                    .render(
                        &reopened,
                        "Ptest-root",
                        Ticks::new(pm_time::ticks_from_seconds(time).unwrap()).unwrap(),
                        &RenderRequest::sized(Size::new(32, 32), Precision::Float32),
                        &MapResolver::default(),
                    )
                    .unwrap();
                assert!(frame.failures.is_empty(), "{:?}", frame.failures);
                let actual = renderer.read_rgba(&frame).unwrap();
                assert_pixels(
                    &actual,
                    quantize(
                        reference(
                            id,
                            version,
                            &Value::Object(evaluated),
                            quantize(input, depth),
                        ),
                        depth,
                    ),
                    &format!("{id} v{version}, depth {depth}, saved key at {time}"),
                );
            }
        }
    }
    assert_eq!(seen.len(), 14);
}
