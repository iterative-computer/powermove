//! E10: saved definitions, independent f64 color equations and real file decode.
//! Equations and matrices are pinned to c7f46a056, color-math.ts and
//! layer-compositing.ts. This oracle never executes the shader under test.
use super::*;
use pm_color::effect_transforms::{Profile, ProfileTransform};

pub(super) fn quantize(pixel: [f64; 4], depth: u32) -> [f64; 4] {
    pixel.map(|v| {
        if depth == 32 {
            v as f32 as f64
        } else {
            half(v as f32) as f64
        }
    })
}
pub(super) fn assert_pixels(pixels: &[f32], expected: [f64; 4], context: &str) {
    assert_eq!(pixels.len(), 32 * 32 * 4, "{context}: complete frame");
    for p in pixels.chunks_exact(4) {
        for i in 0..4 {
            assert!(
                (p[i] as f64 - expected[i]).abs() <= 1. / 1024.,
                "{context}, channel {i}: {} != {}",
                p[i],
                expected[i]
            );
        }
    }
}
pub(super) fn pixel_fixture(id: &str, version: u32, params: Value, pixel: [f64; 4]) -> Value {
    fixture(
        id,
        version,
        params,
        json!({}),
        Some(&format!(
            "void main(){{fragColor=vec4({},{},{},{});}}",
            pixel[0], pixel[1], pixel[2], pixel[3]
        )),
    )
}
#[test]
fn e10_saved_profile_all_d65_transfers_clip_amount_depths_and_alpha() {
    let oracle: Value =
        serde_json::from_str(include_str!("../../../pm-fx-defs/tests/main-values.json")).unwrap();
    let profiles = [Profile::Srgb, Profile::DisplayP3, Profile::Rec2020Gamma24];
    let mut renderer = Renderer::headless().unwrap();
    for row in oracle["profiles"].as_array().unwrap() {
        let a = &row["args"];
        for depth in [8, 16, 32] {
            let input = std::array::from_fn(|i| {
                if i == 3 {
                    0.5
                } else {
                    a[0][i].as_f64().unwrap() * 0.5
                }
            });
            let sampled = quantize(input, depth);
            for (clip, amount) in [(false, 100.), (true, 50.), (false, 0.)] {
                let transform = ProfileTransform {
                    input: profiles[a[1].as_u64().unwrap() as usize],
                    output: profiles[a[2].as_u64().unwrap() as usize],
                    linear_input: a[3].as_bool().unwrap(),
                    linear_output: a[4].as_bool().unwrap(),
                    clip,
                    amount,
                };
                // Validate the independent Rust equations against main's executed
                // f64 values before using them on quantized texture samples.
                if !clip && amount == 100. {
                    let unquantized = transform.pixel(input);
                    for i in 0..3 {
                        assert!(
                            (unquantized[i] - row["value"][i].as_f64().unwrap() * 0.5).abs()
                                < 1e-11
                        );
                    }
                }
                let value = pixel_fixture(
                    "colorProfileConverter",
                    1,
                    json!({"inputProfile":a[1],"outputProfile":a[2],"linearInput":a[3],"linearOutput":a[4],"clip":clip,"amount":amount}),
                    input,
                );
                let expected = quantize(transform.pixel(sampled), depth);
                assert_pixels(
                    &draw(&mut renderer, &value, &MapResolver::default(), depth),
                    expected,
                    &format!("profile {a},depth {depth},clip {clip},amount {amount}"),
                );
            }
        }
    }
    for depth in [8, 16, 32] {
        for alpha in [0., 1.] {
            let input = [0., 0.25, 0.5, alpha];
            let value = pixel_fixture(
                "colorProfileConverter",
                1,
                json!({"inputProfile":0,"outputProfile":2,"clip":false}),
                input,
            );
            let expected = ProfileTransform {
                input: Profile::Srgb,
                output: Profile::Rec2020Gamma24,
                linear_input: false,
                linear_output: false,
                clip: false,
                amount: 100.,
            }
            .pixel(quantize(input, depth));
            assert_pixels(
                &draw(&mut renderer, &value, &MapResolver::default(), depth),
                quantize(expected, depth),
                "profile alpha boundary",
            );
        }
    }
}
#[test]
fn e10_saved_compander_signed_hdr_zero_alpha_identity_and_all_depths() {
    let mut renderer = Renderer::headless().unwrap();
    for depth in [8, 16, 32] {
        for input in [
            [-0.125, 1., 4., 0.5],
            [0., 0.25, 0.5, 0.],
            [0., 0.25, 2., 1.],
        ] {
            for mode in [0, 1] {
                for (gain, gamma) in [(4f64, 2f64), (1., 1.), (2., 0.5)] {
                    let sampled = quantize(input, depth);
                    let mut expected = sampled;
                    if sampled[3] > 0. {
                        for i in 0..3 {
                            let v = sampled[i] / sampled[3];
                            expected[i] = v.signum()
                                * if mode == 0 {
                                    (v.abs() / gain).powf(1. / gamma)
                                } else {
                                    v.abs().powf(gamma) * gain
                                }
                                * sampled[3];
                        }
                    }
                    let value = pixel_fixture(
                        "hdrCompander",
                        1,
                        json!({"mode":mode,"gain":gain,"gamma":gamma}),
                        input,
                    );
                    assert_pixels(
                        &draw(&mut renderer, &value, &MapResolver::default(), depth),
                        quantize(expected, depth),
                        &format!(
                            "compander mode {mode},depth {depth},gain {gain},gamma {gamma},input {input:?}"
                        ),
                    );
                }
            }
        }
    }
}
pub(super) fn hls(c: [f64; 3]) -> [f64; 3] {
    let hi = c.into_iter().fold(f64::NEG_INFINITY, f64::max);
    let lo = c.into_iter().fold(f64::INFINITY, f64::min);
    let d = hi - lo;
    let l = (hi + lo) * 0.5;
    if d == 0. {
        return [0., l, 0.];
    }
    let h = if hi == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.)
    } else if hi == c[1] {
        (c[2] - c[0]) / d + 2.
    } else {
        (c[0] - c[1]) / d + 4.
    };
    [
        (h / 6. + 1.).fract(),
        l,
        (d / (2. * l.min(1. - l)).max(1e-30)).clamp(0., 1.),
    ]
}
pub(super) fn from_hls(c: [f64; 3]) -> [f64; 3] {
    [0., 4., 2.].map(|offset| {
        let hue = (((c[0] * 6. + offset).rem_euclid(6.) - 3.).abs() - 1.).clamp(0., 1.);
        c[1] + (hue - 0.5) * (2. * c[1].min(1. - c[1])) * c[2]
    })
}
fn inverse(m: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let [[a, b, c], [d, e, f], [g, h, i]] = m;
    let cof = [
        [e * i - f * h, c * h - b * i, b * f - c * e],
        [f * g - d * i, a * i - c * g, c * d - a * f],
        [d * h - e * g, b * g - a * h, a * e - b * d],
    ];
    let det = a * cof[0][0] + b * cof[1][0] + c * cof[2][0];
    cof.map(|row| row.map(|v| v / det))
}
fn mult(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    m.map(|row| row.iter().zip(v).map(|(a, b)| a * b).sum())
}
fn converted(c: [f64; 3], mode: u32) -> [f64; 3] {
    if mode == 11 {
        return hls(c.map(|v| v.clamp(0., 1.)));
    }
    if mode == 12 {
        return from_hls([c[0].rem_euclid(1.), c[1].clamp(0., 1.), c[2].clamp(0., 1.)]);
    }
    if mode == 13 {
        let c = c.map(|v| v.max(0.));
        let hi = c.into_iter().fold(0., f64::max);
        let lo = c.into_iter().fold(f64::INFINITY, f64::min);
        return [hls(c)[0], if hi > 0. { (hi - lo) / hi } else { 0. }, hi];
    }
    if mode == 14 {
        return [0., 4., 2.].map(|o| {
            c[2].max(0.)
                * (1. - c[1].clamp(0., 1.)
                    + c[1].clamp(0., 1.)
                        * (((c[0] * 6. + o).rem_euclid(6.) - 3.).abs() - 1.).clamp(0., 1.))
        });
    }
    let m = match mode {
        15 | 16 => [
            [0.299, 0.587, 0.114],
            [0.595716, -0.274453, -0.321263],
            [0.211456, -0.522591, 0.311135],
        ],
        17 | 18 => [
            [0.299, 0.587, 0.114],
            [-0.14713, -0.28886, 0.436],
            [0.615, -0.51499, -0.10001],
        ],
        19 | 20 => pm_color::primaries::rgb_to_xyz(
            &pm_color::primaries::chromaticities(pm_protocol::color::Primaries::Bt709).unwrap(),
        ),
        21 | 22 => [
            [0.2126, 0.7152, 0.0722],
            [-0.2126 / 1.8556, -0.7152 / 1.8556, 0.5],
            [0.5, -0.7152 / 1.5748, -0.0722 / 1.5748],
        ],
        _ => panic!("unknown conversion"),
    };
    let input = if mode == 22 {
        [c[0], c[1] - 0.5, c[2] - 0.5]
    } else {
        c
    };
    let mut out = mult(if mode.is_multiple_of(2) { inverse(m) } else { m }, input);
    if mode == 21 {
        out[1] += 0.5;
        out[2] += 0.5;
    }
    out
}
fn combiner(
    source: [f64; 4],
    owner_alpha: f64,
    from: u32,
    to: u32,
    invert: bool,
    solid: bool,
) -> [f64; 4] {
    let rgb = if source[3] > 0. {
        [
            source[0] / source[3],
            source[1] / source[3],
            source[2] / source[3],
        ]
    } else {
        [0.; 3]
    };
    let mut alpha = owner_alpha;
    let mut result = if from >= 11 {
        converted(rgb, from)
    } else {
        let hls = hls(rgb.map(|v| v.clamp(0., 1.)));
        let values = [
            rgb[0],
            rgb[1],
            rgb[2],
            source[3],
            mult([[0.2126, 0.7152, 0.0722]; 3], rgb)[0],
            hls[0],
            hls[1],
            hls[2],
            hls[2] * hls[1].min(1. - hls[1]),
            rgb.into_iter().fold(f64::INFINITY, f64::min),
            rgb.into_iter().fold(f64::NEG_INFINITY, f64::max),
        ];
        let value = values[from as usize];
        match to {
            1 => [value, 0., 0.],
            2 => [0., value, 0.],
            3 => [0., 0., value],
            4 => {
                alpha = value.clamp(0., 1.);
                [1.; 3]
            }
            5 => from_hls([value.rem_euclid(1.), 0.5, 1.]),
            7 => from_hls([0., 0.5, value.clamp(0., 1.)]),
            _ => [value; 3],
        }
    };
    if invert {
        if from < 11 && to == 4 {
            alpha = 1. - alpha;
        } else {
            result = result.map(|v| 1. - v);
        }
    }
    if solid {
        alpha = 1.;
    }
    [
        result[0] * alpha,
        result[1] * alpha,
        result[2] * alpha,
        alpha,
    ]
}
fn check_combiner(versions: &[u32], froms: std::ops::RangeInclusive<u32>) {
    let mut renderer = Renderer::headless().unwrap();
    for &version in versions {
        for depth in [8, 16, 32] {
            for from in froms.clone() {
                for to in 0..if from < 11 { 8 } else { 1 } {
                    for (input, invert, solid, map) in [
                        ([-0.125, 0.25, 0.5, 0.5], false, false, false),
                        ([0., 0.25, 0.5, 1.], true, true, true),
                        ([0., 0., 0., 0.], false, false, false),
                    ] {
                        let mut value = pixel_fixture(
                            "channelCombiner",
                            version,
                            json!({"from":from,"to":to,"invert":invert,"solidAlpha":solid}),
                            input,
                        );
                        let source = if map { [0.5, 0.125, 0.25, 0.5] } else { input };
                        if map {
                            value["layers"][0]["fx"][0]["inputs"] =
                                json!({"map":{"layerId":"hidden","stage":"source"}});
                            let mut hidden = layer(
                                "hidden",
                                "shader",
                                json!({"w":32,"h":32,"code":"void main(){fragColor=vec4(0.5,0.125,0.25,0.5);}","uniforms":{}}),
                                0.,
                                0.,
                                1.,
                            );
                            hidden["on"] = json!(false);
                            value["layers"].as_array_mut().unwrap().push(hidden);
                        }
                        let sampled = quantize(source, depth);
                        let expected =
                            quantize(combiner(sampled, input[3], from, to, invert, solid), depth);
                        assert_pixels(
                            &draw(&mut renderer, &value, &MapResolver::default(), depth),
                            expected,
                            &format!(
                                "combiner v{version},depth {depth},from {from},to {to},invert {invert},solid {solid},map {map}"
                            ),
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn e10_saved_channel_combiner_v1_v2_all_legacy_channels_targets_and_alpha() {
    check_combiner(&[1, 2], 0..=10);
}
#[test]
fn e10_saved_channel_combiner_v2_all_twelve_multichannel_conversions() {
    check_combiner(&[2], 11..=22);
}

pub(super) fn decoded_png(renderer: &mut Renderer, value: &Value, depth: u32, label: &str) {
    use pm_export::{
        AlphaOutput, Codec, ColorTags, Container, Dither, ExportSpec, Overwrite, WorkingEncoding,
        WorkingFrame,
    };
    let dir = tempfile::tempdir().unwrap();
    let preview = draw(renderer, value, &MapResolver::default(), depth);
    let mut saved = value.clone();
    saved["colorDepth"] = json!(depth);
    // Reload saved JSON rather than sharing a live evaluated document.
    let saved: Value = serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    let mut document = load(&saved);
    document.set_saving_app_version(Some("1.3.0".into()));
    renderer.set_main_effects_for_document(&document).unwrap();
    let mut request = RenderRequest::sized(Size::new(32, 32), Precision::Float32);
    request.output_kind = pm_render::OutputKind::Final;
    request.export = true;
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
    let pixels = renderer.read_rgba(&frame).unwrap();
    assert_eq!(pixels, preview, "viewer/export {label}, depth {depth}");
    for sixteen_bit in [false, true] {
        let path = dir.path().join(format!("output-{sixteen_bit}.png"));
        let spec = ExportSpec {
            width: 32,
            height: 32,
            rate: pm_time::Rate::FPS_30,
            frames: 1,
            codec: Codec::Png { sixteen_bit },
            container: Container::PngStill,
            tags: ColorTags::SrgbImage,
            alpha: AlphaOutput::Straight,
            working: WorkingEncoding::Standard,
            dither: Dither::Off,
            audio: None,
            timecode: None,
            destination: path.clone(),
            overwrite: Overwrite::Fail,
            mode: Some(0o600),
        };
        let mut export = pm_export::export(spec).unwrap();
        export
            .push_frame(&WorkingFrame {
                width: 32,
                height: 32,
                rgba: &pixels,
            })
            .unwrap();
        export.finish().unwrap();
        let mut reader =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()))
                .read_info()
                .unwrap();
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut bytes).unwrap();
        assert_eq!((info.width, info.height), (32, 32));
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(
            info.bit_depth,
            if sixteen_bit {
                png::BitDepth::Sixteen
            } else {
                png::BitDepth::Eight
            }
        );
        assert_eq!(
            info.buffer_size(),
            32 * 32 * 4 * if sixteen_bit { 2 } else { 1 }
        );
        let maximum = if sixteen_bit { 65535. } else { 255. };
        let decoded: Vec<u16> = if sixteen_bit {
            bytes[..info.buffer_size()]
                .chunks_exact(2)
                .map(|b| u16::from_be_bytes([b[0], b[1]]))
                .collect()
        } else {
            bytes[..info.buffer_size()]
                .iter()
                .map(|&v| u16::from(v))
                .collect()
        };
        for (actual, p) in decoded.chunks_exact(4).zip(pixels.chunks_exact(4)) {
            let alpha = f64::from(p[3]).clamp(0., 1.);
            let rgb = if alpha < 1. / 4096. {
                [0.; 3]
            } else {
                std::array::from_fn(|i| f64::from(p[i]) / alpha)
            };
            // The established SDR delivery contract maps extended sRGB through
            // CSS Color 4 for PNG8; PNG16 clips only at integer encoding.
            let rgb = if sixteen_bit {
                rgb
            } else {
                pm_color::css_srgb(rgb)
            };
            let expected: [u16; 4] = std::array::from_fn(|i| {
                let v = if i == 3 { alpha } else { rgb[i] };
                (v.clamp(0., 1.) * maximum).round_ties_even() as u16
            });
            assert_eq!(
                actual, expected,
                "decoded {label},depth {depth},16bit {sixteen_bit}"
            );
        }
    }
}
#[test]
fn e10_saved_effects_all_depths_encode_and_decode_png8_and_png16() {
    let mut renderer = Renderer::headless().unwrap();
    for depth in [8, 16, 32] {
        for (id, version, params) in [
            (
                "colorProfileConverter",
                1,
                json!({"inputProfile":0,"outputProfile":1}),
            ),
            ("hdrCompander", 1, json!({"mode":0,"gain":4,"gamma":2})),
            ("channelCombiner", 1, json!({"from":2,"to":0})),
            ("channelCombiner", 2, json!({"from":21})),
        ] {
            let value = pixel_fixture(id, version, params, [0.125, 0.25, 0.375, 0.5]);
            decoded_png(&mut renderer, &value, depth, &format!("{id} v{version}"));
        }
    }
}

#[test]
fn e10_absent_null_and_explicit_combiner_v1_keep_saved_pixels_and_missing_versions_fail() {
    let mut renderer = Renderer::headless().unwrap();
    for depth in [8, 16, 32] {
        let value = pixel_fixture(
            "channelCombiner",
            1,
            json!({"from":11}),
            [0.125, 0.25, 0.375, 0.5],
        );
        let expected = draw(&mut renderer, &value, &MapResolver::default(), depth);
        assert_pixels(
            &expected,
            [0.375, 0.375, 0.375, 0.5],
            "saved Channel Combiner v1 clamps unavailable conversion to Max RGB",
        );
        for version in [None, Some(Value::Null), Some(json!(1))] {
            let mut saved = value.clone();
            match version {
                None => {
                    saved["layers"][0]["fx"][0]
                        .as_object_mut()
                        .unwrap()
                        .remove("version");
                }
                Some(v) => saved["layers"][0]["fx"][0]["version"] = v,
            };
            assert_eq!(
                draw(&mut renderer, &saved, &MapResolver::default(), depth),
                expected,
                "absent/null/explicit saved v1 depth {depth}"
            );
        }
        let mut current = value.clone();
        current["layers"][0]["fx"][0]["version"] = json!(2);
        assert_ne!(
            draw(&mut renderer, &current, &MapResolver::default(), depth),
            expected,
            "v2 conversion must not replace saved v1"
        );
    }
    for id in ["colorProfileConverter", "hdrCompander", "channelCombiner"] {
        let value = pixel_fixture(id, 999, json!({}), [0.125, 0.25, 0.375, 0.5]);
        let mut doc = load(&value);
        doc.set_saving_app_version(Some("1.3.0".into()));
        renderer.set_main_effects_for_document(&doc).unwrap();
        let error = renderer
            .render(
                &doc,
                "Ptest-root",
                Ticks::ZERO,
                &RenderRequest::sized(Size::new(32, 32), Precision::Float32),
                &MapResolver::default(),
            )
            .err()
            .expect("unavailable saved revision must fail")
            .to_string();
        assert!(
            error.contains(id) && error.contains("999"),
            "missing saved revision must be explicit: {error}"
        );
    }
}
