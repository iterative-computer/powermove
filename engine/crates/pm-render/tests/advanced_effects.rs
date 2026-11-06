//! Offscreen Metal pixels through saved main definitions and real resources.
mod support;
use pm_protocol::{
    color::Precision,
    common::{Size, Ticks},
};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Value, json};
use support::doc::*;
fn fixture(id: &str, version: u32, params: Value, inputs: Value, code: Option<&str>) -> Value {
    let mut owner = layer(
        "owner",
        if code.is_some() { "shader" } else { "solid" },
        code.map_or_else(
            || json!({"w":32,"h":32,"color":"#4080C0"}),
            |code| json!({"w":32,"h":32,"code":code,"uniforms":{}}),
        ),
        0.,
        0.,
        1.,
    );
    let params = params
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), ch(v.clone())))
        .collect::<serde_json::Map<_, _>>();
    owner["fx"] =
        json!([{"id":"effect","type":id,"version":version,"on":true,"p":params,"inputs":inputs}]);
    let mut project = project(32, 32, 1., no_fill(), vec![owner]);
    if id == "applyColorLut" {
        asset(&mut project, "grade", "lut", "cube", 0, 0, 0.);
    }
    project["colorDepth"] = json!(16);
    project
}
fn draw(renderer: &mut Renderer, value: &Value, resolver: &MapResolver, depth: u32) -> Vec<f32> {
    let mut value = value.clone();
    value["colorDepth"] = json!(depth);
    let mut doc = load(&value);
    doc.set_saving_app_version(Some("1.3.0".into()));
    renderer.set_main_effects_for_document(&doc).unwrap();
    let frame = renderer
        .render(
            &doc,
            "Ptest-root",
            Ticks::ZERO,
            &RenderRequest::sized(Size::new(32, 32), Precision::Float32),
            resolver,
        )
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    renderer.read_rgba(&frame).unwrap()
}
fn check_viewport(renderer: &mut Renderer, value: &Value, full: &[f32]) {
    check_viewport_precision(renderer, value, full, Precision::Float16);
}
fn check_float_viewport(renderer: &mut Renderer, value: &Value, full: &[f32]) {
    let mut value = value.clone();
    value["colorDepth"] = json!(32);
    check_viewport_precision(renderer, &value, full, Precision::Float32);
}
fn check_viewport_precision(
    renderer: &mut Renderer,
    value: &Value,
    full: &[f32],
    precision: Precision,
) {
    let mut document = load(value);
    document.set_saving_app_version(Some("1.3.0".into()));
    let mut request = RenderRequest::sized(Size::new(16, 12), precision);
    request.scale = Some((1., 1.));
    request.origin = Some((8., 8.));
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
    let actual = renderer.read_rgba(&frame).unwrap();
    for y in 0..12 {
        for x in 0..16 {
            assert_eq!(
                &actual[(y * 16 + x) * 4..][..4],
                &full[((y + 8) * 32 + x + 8) * 4..][..4],
                "viewport at {x},{y}"
            );
        }
    }
}
fn half(v: f32) -> f32 {
    pm_gpu::readback::half_to_f32(pm_gpu::readback::f32_to_half(v))
}
#[test]
fn metal_typed_lut_1d_3d_domain_alpha_replacement_and_missing_lease() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("grade.cube");
    let mut resolver = MapResolver::default();
    resolver.paths.insert("grade".into(), path.clone());
    let value = fixture(
        "applyColorLut",
        1,
        json!({"amount":100}),
        json!({"lut":{"kind":"asset","assetId":"grade"}}),
        None,
    );
    let mut renderer = Renderer::headless().unwrap();
    for cube in [false, true] {
        let text = if cube {
            format!(
                "LUT_3D_SIZE 2\n{}",
                (0..8)
                    .map(|i| format!(
                        "{} {} {}\n",
                        1 - (i & 1),
                        1 - ((i >> 1) & 1),
                        1 - ((i >> 2) & 1)
                    ))
                    .collect::<String>()
            )
        } else {
            "LUT_1D_SIZE 2\n1 1 1\n0 0 0\n".into()
        };
        std::fs::write(&path, text).unwrap();
        for depth in [8, 16, 32] {
            let pixels = draw(&mut renderer, &value, &resolver, depth);
            let input = [64f32 / 255., 128. / 255., 192. / 255.];
            let expected = if depth == 32 {
                input.map(|v| 1. - v)
            } else {
                input.map(|v| half(1. - half(v)))
            };
            assert!(
                pixels
                    .chunks_exact(4)
                    .all(|p| p == [expected[0], expected[1], expected[2], 1.]),
                "{}D depth {depth}: {:?} != {:?}",
                if cube { 3 } else { 1 },
                &pixels[..4],
                expected
            );
        }
    }
    let alpha = fixture(
        "applyColorLut",
        1,
        json!({"amount":100}),
        json!({"lut":{"kind":"asset","assetId":"grade"}}),
        Some("void main(){fragColor=vec4(0.125,0.25,0.375,0.5);}"),
    );
    for depth in [8, 16, 32] {
        let pixels = draw(&mut renderer, &alpha, &resolver, depth);
        assert!(
            pixels
                .chunks_exact(4)
                .all(|p| p == [0.375, 0.25, 0.125, 0.5])
        );
    }
    std::fs::write(
        &path,
        "DOMAIN_MIN 0 0 0\nDOMAIN_MAX 2 2 2\nLUT_1D_SIZE 2\n0 0 0\n2 2 2\n",
    )
    .unwrap();
    let pixels = draw(&mut renderer, &value, &resolver, 16);
    assert_eq!(
        &pixels[..4],
        &[half(64. / 255.), half(128. / 255.), half(192. / 255.), 1.]
    );
    std::fs::remove_file(path).unwrap();
    let doc = load(&value);
    renderer.set_main_effects_for_document(&doc).unwrap();
    assert!(
        renderer
            .render(
                &doc,
                "Ptest-root",
                Ticks::ZERO,
                &RenderRequest::sized(Size::new(32, 32), Precision::Float16),
                &resolver
            )
            .is_err()
    );
}
#[test]
fn metal_mosaic_cell_atlas_and_wide_gaussian_mips_preserve_reference_values() {
    let mut renderer = Renderer::headless().unwrap();
    let stripes = "void main(){ fragColor=vec4(mod(floor(gl_FragCoord.x),2.0),0.,0.,1.); }";
    let mosaic = fixture(
        "pixelate",
        2,
        json!({"horizontalBlocks":2,"verticalBlocks":2,"sharpColors":false}),
        json!({}),
        Some(stripes),
    );
    let pixels = draw(&mut renderer, &mosaic, &MapResolver::default(), 32);
    assert!(
        pixels.chunks_exact(4).all(|p| p == [0.5, 0., 0., 1.]),
        "source cell integral {:?}",
        &pixels[..4]
    );
    let blur = fixture(
        "blur",
        2,
        json!({"amount":128,"repeatEdgePixels":true}),
        json!({}),
        Some("void main(){fragColor=vec4(0.25,0.5,2.,0.5);}"),
    );
    let pixels = draw(&mut renderer, &blur, &MapResolver::default(), 16);
    assert!(
        pixels.chunks_exact(4).all(|p| p == [0.25, 0.5, 2., 0.5]),
        "wide Gaussian must retain premultiplied HDR constant: {:?}",
        &pixels[..4]
    );
    let document = load(&blur);
    let comp = pm_eval::Comp::resolve(&document, "Ptest-root").unwrap();
    let mut evaluator = pm_eval::Evaluator::new();
    let mut scope = pm_eval::Scope::new(&mut evaluator, &comp, &comp);
    let owner = &scope.layers()[0];
    let steps = pm_render::effects::plan::effect_steps_scaled(
        &mut scope,
        renderer.effects(),
        owner,
        0.,
        (1., 1.),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        steps[0].spatial.unwrap().lod,
        3,
        "128*1.5/32 chooses source LOD 3"
    );
}
#[test]
fn metal_hdr_compander_keeps_signed_color_and_alpha_at_saved_depths() {
    let mut renderer = Renderer::headless().unwrap();
    for (mode, code, expected) in [
        (
            0,
            "void main(){fragColor=vec4(-0.125,1.,4.,0.5);}",
            [
                -0.125,
                std::f64::consts::FRAC_1_SQRT_2 / 2.,
                std::f64::consts::FRAC_1_SQRT_2,
                0.5,
            ],
        ),
        (
            1,
            "void main(){fragColor=vec4(-0.125,1.,4.,0.5);}",
            [-0.125, 8., 128., 0.5],
        ),
    ] {
        // Reference GLSL: sign(rgb)*pow(abs(rgb)/gain,1/gamma)
        // or sign(rgb)*pow(abs(rgb),gamma)*gain, then premultiply.
        let value = fixture(
            "hdrCompander",
            1,
            json!({"mode":mode,"gain":4,"gamma":2}),
            json!({}),
            Some(code),
        );
        for depth in [8, 16] {
            let pixels = draw(&mut renderer, &value, &MapResolver::default(), depth);
            let expected = expected.map(|v| half(v as f32));
            assert!(
                pixels.chunks_exact(4).all(|p| p == expected),
                "mode {mode}, depth {depth}: {:?} != {:?}",
                &pixels[..4],
                expected
            );
        }
    }
}

#[test]
fn metal_profile_converter_pixels_match_the_executed_main_value_oracle() {
    let oracle: Value =
        serde_json::from_str(include_str!("../../pm-fx-defs/tests/main-values.json")).unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let mut worst = 0f64;
    for row in oracle["profiles"].as_array().unwrap() {
        let args = &row["args"];
        let rgb: [f64; 3] = std::array::from_fn(|i| args[0][i].as_f64().unwrap());
        let code = format!(
            "void main(){{fragColor=vec4({},{},{},0.5);}}",
            rgb[0] * 0.5,
            rgb[1] * 0.5,
            rgb[2] * 0.5
        );
        for (clip, amount) in [(false, 100.), (true, 50.)] {
            let value = fixture(
                "colorProfileConverter",
                1,
                json!({"inputProfile":args[1],"outputProfile":args[2],"linearInput":args[3],"linearOutput":args[4],"clip":clip,"amount":amount}),
                json!({}),
                Some(&code),
            );
            let pixels = draw(&mut renderer, &value, &MapResolver::default(), 32);
            let expected: [f64; 3] = std::array::from_fn(|i| {
                let converted = row["value"][i].as_f64().unwrap();
                let converted = if clip {
                    converted.clamp(0., 1.)
                } else {
                    converted
                };
                (rgb[i] + (converted - rgb[i]) * amount / 100.) * 0.5
            });
            for p in pixels.chunks_exact(4) {
                assert_eq!(p[3], 0.5);
                for i in 0..3 {
                    let error = (p[i] as f64 - expected[i]).abs();
                    worst = worst.max(error);
                    assert!(
                        error <= 1. / 1024.,
                        "main oracle {args},clip={clip},amount={amount},channel={i}: {} != {}",
                        p[i],
                        expected[i]
                    );
                }
            }
        }
    }
    eprintln!(
        "216 source-executed profile cases × 2 clip/amount variants; worst absolute component error {worst:e}; existing shader pixel bound 1/1024"
    );
}

#[test]
fn metal_hidden_reference_stages_produce_before_the_consumer_and_cycles_fail() {
    let mut renderer = Renderer::headless().unwrap();
    for (stage, expected) in [
        ("source", [1., 1.]),
        ("masks", [1., 0.]),
        ("effects", [0., 0.]),
    ] {
        let mut value = fixture(
            "setMatte",
            1,
            json!({"channel":0,"composite":false}),
            json!({"map":{"layerId":"hidden","stage":stage}}),
            None,
        );
        let mut source = layer(
            "hidden",
            "solid",
            json!({"w":32,"h":32,"color":"#FFFFFF"}),
            0.,
            0.,
            1.,
        );
        source["on"] = json!(false);
        set(&mut source, "opacity", json!(25));
        source["masks"] = json!([{"id":"cut","shape":"rect","mode":"add","on":true,"p":{"x":ch(json!(8)),"y":ch(json!(16)),"w":ch(json!(16)),"h":ch(json!(32)),"rotation":ch(json!(0)),"feather":ch(json!(0))}}]);
        source["fx"] = json!([{"id":"invert","type":"invert","version":2,"on":true,"p":{}}]);
        value["layers"].as_array_mut().unwrap().push(source);
        let pixels = draw(&mut renderer, &value, &MapResolver::default(), 16);
        assert_eq!(
            pixels[(16 * 32 + 8) * 4 + 3],
            expected[0],
            "stage {stage} left"
        );
        assert_eq!(
            pixels[(16 * 32 + 24) * 4 + 3],
            expected[1],
            "stage {stage} right"
        );
        check_viewport(&mut renderer, &value, &pixels);
    }
    let cycle = fixture(
        "setMatte",
        1,
        json!({}),
        json!({"map":{"layerId":"owner","stage":"effects"}}),
        None,
    );
    let doc = load(&cycle);
    renderer.set_main_effects_for_document(&doc).unwrap();
    let error = renderer
        .render(
            &doc,
            "Ptest-root",
            Ticks::ZERO,
            &RenderRequest::sized(Size::new(32, 32), Precision::Float16),
            &MapResolver::default(),
        )
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("cycle"), "{error}");
}

fn echo_reference(samples: &[[f32; 4]], operator: u32, intensity: f32, decay: f32) -> [f32; 4] {
    let mut out = [0.; 4];
    for (index, sample) in samples.iter().enumerate() {
        let weight = intensity * decay.powi(index as i32)
            / if operator == 6 {
                samples.len() as f32
            } else {
                1.
            };
        let color = sample.map(|v| v * weight);
        if index == 0 {
            out = color.map(half);
            continue;
        }
        out = std::array::from_fn(|i| match operator {
            1 => out[i].max(color[i]),
            2 => out[i].min(color[i]),
            3 => out[i] + color[i] - out[i] * color[i],
            4 => color[i] + out[i] * (1. - color[3]),
            5 => out[i] + color[i] * (1. - out[3]),
            _ => out[i] + color[i],
        });
        if out[3] <= 0. {
            out = [0.; 4];
        } else {
            out[3] = out[3].clamp(0., 1.);
        }
        out = out.map(half);
    }
    out
}
#[test]
fn metal_echo_exact_prefix_subframes_seven_operators_and_reverse_seek() {
    let mut renderer = Renderer::headless().unwrap();
    for operator in 0..7 {
        let mut value = fixture(
            "echo",
            1,
            json!({"echoTime":-0.25,"echoes":2,"intensity":0.5,"decay":0.5,"operator":operator}),
            json!({}),
            Some("void main(){fragColor=vec4(iTime*.5,.25,.125,.5);}"),
        );
        let echo = value["layers"][0]["fx"][0].clone();
        value["layers"][0]["fx"] = json!([{"id":"before","type":"invert","version":2,"on":true,"p":{}},echo,{"id":"after","type":"hdrCompander","version":1,"on":true,"p":{"mode":ch(json!(1)),"gain":ch(json!(2)),"gamma":ch(json!(1))}}]);
        let mut doc = load(&value);
        doc.set_saving_app_version(Some("1.3.0".into()));
        renderer.set_main_effects_for_document(&doc).unwrap();
        let mut seen = vec![];
        for time in [0.75, 0.5, 0.75] {
            let frame = renderer
                .render(
                    &doc,
                    "Ptest-root",
                    support::ticks(time),
                    &RenderRequest::sized(Size::new(32, 32), Precision::Float16),
                    &MapResolver::default(),
                )
                .unwrap();
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            let actual = renderer.read_rgba(&frame).unwrap();
            let samples =
                [time, time - 0.25, time - 0.5].map(|t| [(1. - t) as f32 * 0.5, 0.25, 0.375, 0.5]);
            let expected = echo_reference(&samples, operator, 0.5, 0.5).map(half);
            let expected = [
                half(expected[0] * 2.),
                half(expected[1] * 2.),
                half(expected[2] * 2.),
                expected[3],
            ];
            assert!(
                actual.chunks_exact(4).all(|p| p == expected),
                "operator {operator},time {time}: {:?} != {:?}",
                &actual[..4],
                expected
            );
            seen.push(actual);
        }
        assert_eq!(seen[0], seen[2], "operator {operator}: reverse seek");
    }
}

#[test]
fn metal_typed_mask_stroke_matches_reference_soft_brush_coverage() {
    let mut value = fixture(
        "stroke",
        2,
        json!({"brushSize":4,"hardness":50,"spacing":0,"start":0,"end":100,"color":"#FF0000","opacity":100,"paintStyle":1}),
        json!({"path":{"kind":"path","layerId":"geometry","pathId":"outline"}}),
        None,
    );
    let mut provider = layer(
        "geometry",
        "solid",
        json!({"w":32,"h":32,"color":"#FFFFFF"}),
        0.,
        0.,
        1.,
    );
    provider["on"] = json!(false);
    provider["masks"] = json!([{"id":"outline","shape":"rect","on":true,"mode":"add","p":{"x":ch(json!(16)),"y":ch(json!(16)),"w":ch(json!(16)),"h":ch(json!(8)),"rotation":ch(json!(0)),"feather":ch(json!(0))}}]);
    value["layers"].as_array_mut().unwrap().push(provider);
    let mut renderer = Renderer::headless().unwrap();
    let pixels = draw(&mut renderer, &value, &MapResolver::default(), 16);
    // effect-path.ts: radius=2, antialias=.5, inner=1, d=1.5;
    // round((radius+AA-d)/max(2AA,radius-inner+AA)*255)=170.
    let document = load(&value);
    let comp = pm_eval::Comp::resolve(&document, "Ptest-root").unwrap();
    let mut evaluator = pm_eval::Evaluator::new();
    let mut scope = pm_eval::Scope::new(&mut evaluator, &comp, &comp);
    let owner = &scope.layers()[0];
    let steps = pm_render::effects::plan::effect_steps_scaled(
        &mut scope,
        renderer.effects(),
        owner,
        0.,
        (1., 1.),
    )
    .unwrap()
    .unwrap();
    let asset = &steps[0].assets[0].1;
    assert_eq!(
        asset.rgba[((31 - 10) * 32 + 16) * 4 + 3],
        170. / 255.,
        "typed brush CPU alpha; nonzero={}",
        asset.rgba.chunks_exact(4).filter(|p| p[3] > 0.).count()
    );
    check_viewport(&mut renderer, &value, &pixels);
    let coverage = half(170. / 255.);
    assert_eq!(
        &pixels[(10 * 32 + 16) * 4..][..4],
        &[coverage, 0., 0., coverage]
    );
    assert_eq!(&pixels[(12 * 32 + 16) * 4..][..4], &[1., 0., 0., 1.]);
    assert_eq!(&pixels[(16 * 32 + 16) * 4..][..4], &[0., 0., 0., 0.]);
}

#[test]
fn metal_typed_image_samples_the_reference_texturize_slope() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("texture.png");
    let mut png = png::Encoder::new(std::fs::File::create(&path).unwrap(), 32, 32);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    let rgba = (0..32)
        .flat_map(|_| (0..32).flat_map(|x| [x * 8, 0, 0, 255]))
        .collect::<Vec<u8>>();
    png.write_header().unwrap().write_image_data(&rgba).unwrap();
    let mut value = fixture(
        "texturize",
        2,
        json!({"direction":0,"contrast":100,"placement":2}),
        json!({"map":{"layerId":null,"stage":"source"},"texture":{"kind":"asset","assetId":"image"}}),
        None,
    );
    asset(&mut value, "image", "image", "png", 32, 32, 0.);
    let mut resolver = MapResolver::default();
    resolver.paths.insert("image".into(), path);
    let mut renderer = Renderer::headless().unwrap();
    let pixels = draw(&mut renderer, &value, &resolver, 16);
    // Reference texturize.ts: central red-luminance difference, normalise(-slope,0,1), dot with (1,0,1).
    let slope = (16f64 / 255.) * 0.2126 * 100. * 0.04;
    let gain = ((1. - slope) / (1. + slope * slope).sqrt()) as f32;
    let expected = [
        half(half(64. / 255.) * gain),
        half(half(128. / 255.) * gain),
        half(half(192. / 255.) * gain),
        1.,
    ];
    assert_eq!(&pixels[(16 * 32 + 16) * 4..][..4], &expected);
}

#[test]
fn metal_saved_stroke_v1_retains_hard_brush_pixels_and_path_identity() {
    let mut value = fixture(
        "stroke",
        2,
        json!({"brushSize":4,"hardness":100,"spacing":0,"start":0,"end":100,"color":"#FF0000","opacity":100,"paintStyle":1}),
        json!({"path":{"kind":"path","layerId":"geometry","pathId":"outline"}}),
        None,
    );
    let mut provider = layer(
        "geometry",
        "solid",
        json!({"w":32,"h":32,"color":"#FFFFFF"}),
        0.,
        0.,
        1.,
    );
    provider["on"] = json!(false);
    provider["masks"] = json!([{"id":"outline","shape":"rect","on":true,"mode":"add","p":{"x":ch(json!(16)),"y":ch(json!(16)),"w":ch(json!(16)),"h":ch(json!(8)),"rotation":ch(json!(0)),"feather":ch(json!(0))}}]);
    value["layers"].as_array_mut().unwrap().push(provider);
    let mut renderer = Renderer::headless().unwrap();
    let current = draw(&mut renderer, &value, &MapResolver::default(), 16);
    value["layers"][0]["fx"][0]["version"] = json!(1);
    let historical = draw(&mut renderer, &value, &MapResolver::default(), 16);
    assert_eq!(current, historical, "saved hard brush v1 and v2 defaults");
    assert_eq!(&historical[(11 * 32 + 16) * 4..][..4], &[1., 0., 0., 1.]);
    assert_eq!(&historical[(16 * 32 + 16) * 4..][..4], &[0., 0., 0., 0.]);
}

#[test]
fn metal_source_owner_effects_precede_opacity_and_transform() {
    let mut renderer = Renderer::headless().unwrap();
    let mut value = fixture(
        "shiftChannels",
        1,
        json!({"takeAlphaFrom":8}),
        json!({}),
        None,
    );
    value["layers"][0]["effectSpace"] = json!("source");
    set(&mut value["layers"][0], "opacity", json!(50));
    let pixels = draw(&mut renderer, &value, &MapResolver::default(), 32);
    let expected = [0.5 * 64. / 255., 0.5 * 128. / 255., 0.5 * 192. / 255., 0.5];
    for p in pixels.chunks_exact(4) {
        for i in 0..4 {
            assert!(
                (p[i] - expected[i]).abs() <= 1. / 1024.,
                "{p:?} != {expected:?}"
            );
        }
    }
}

#[test]
fn metal_source_reference_retains_hidden_offscreen_singular_content_and_stages() {
    let mut renderer = Renderer::headless().unwrap();
    for (stage, expected) in [
        ("source", [1., 1.]),
        ("masks", [1., 0.]),
        ("effects", [0., 0.]),
    ] {
        let mut value = fixture(
            "setMatte",
            1,
            json!({"channel":0,"composite":false}),
            json!({"map":{"layerId":"hidden","stage":stage,"space":"source"}}),
            None,
        );
        let mut source = layer(
            "hidden",
            "solid",
            json!({"w":16,"h":8,"color":"#FFFFFF"}),
            10000.,
            -10000.,
            1.,
        );
        source["on"] = json!(false);
        set(&mut source, "scale.x", json!(0));
        set(&mut source, "opacity", json!(0));
        source["masks"] = json!([{"id":"cut","shape":"rect","mode":"add","on":true,"p":{"x":ch(json!(4)),"y":ch(json!(4)),"w":ch(json!(8)),"h":ch(json!(8)),"rotation":ch(json!(0)),"feather":ch(json!(0))}}]);
        source["fx"] = json!([{"id":"invert","type":"invert","version":2,"on":true,"p":{}}]);
        value["layers"].as_array_mut().unwrap().push(source);
        let pixels = draw(&mut renderer, &value, &MapResolver::default(), 32);
        assert_eq!(pixels[(16 * 32 + 8) * 4 + 3], expected[0], "{stage} left");
        assert_eq!(pixels[(16 * 32 + 24) * 4 + 3], expected[1], "{stage} right");
        check_float_viewport(&mut renderer, &value, &pixels);
    }
}

#[test]
fn metal_source_padding_clips_each_prefix_before_later_spatial_passes() {
    let mut renderer = Renderer::headless().unwrap();
    let mut value = fixture(
        "blur",
        2,
        json!({"amount":2,"repeatEdgePixels":false}),
        json!({}),
        None,
    );
    value["layers"][0]["effectSpace"] = json!("source");
    value["layers"][0]["d"] = json!({"w":8,"h":8,"color":"#FFFFFF"});
    set(&mut value["layers"][0], "position.x", json!(12));
    set(&mut value["layers"][0], "position.y", json!(12));
    let plain = draw(&mut renderer, &value, &MapResolver::default(), 32);
    let blur = value["layers"][0]["fx"][0].clone();
    value["layers"][0]["fx"] = json!([
        {"id":"opaque","type":"shiftChannels","version":1,"on":true,"p":{"takeAlphaFrom":ch(json!(8))}}, blur]);
    let prefixed = draw(&mut renderer, &value, &MapResolver::default(), 32);
    assert_eq!(
        plain, prefixed,
        "an opaque-making pass must not fill future padding"
    );
    assert!(
        plain[(16 * 32 + 10) * 4 + 3] > 0.,
        "source blur must grow outside original bounds"
    );
    assert_eq!(plain[3], 0.);
    check_float_viewport(&mut renderer, &value, &prefixed);
}

#[test]
fn metal_source_owner_local_ramp_survives_parent_transform_and_vector_bounds() {
    let mut renderer = Renderer::headless().unwrap();
    for shape in [false, true] {
        let mut value = fixture(
            "gradient",
            2,
            json!({"startX":0,"startY":0,"endX":16,"endY":0,"rampScatter":0}),
            json!({}),
            None,
        );
        let l = &mut value["layers"][0];
        l["effectSpace"] = json!("source");
        l["d"] = if shape {
            json!({"shape":"rect","w":16,"h":16,"color":"#FFFFFF","stroke":0})
        } else {
            json!({"w":16,"h":16,"color":"#FFFFFF"})
        };
        if shape {
            l["type"] = json!("shape");
        }
        set(l, "position.x", json!(if shape { 16 } else { 8 }));
        set(l, "position.y", json!(if shape { 16 } else { 8 }));
        set(l, "opacity", json!(50));
        let pixels = draw(&mut renderer, &value, &MapResolver::default(), 32);
        for x in 9..23 {
            let p = &pixels[(16 * 32 + x) * 4..][..4];
            // Main's shape content raster includes four transparent pixels
            // per side; source owners use that full box for layer coordinates.
            let expected =
                ((x as f32 + 0.5 - 8. + if shape { 4. } else { 0. }) / 16.).clamp(0., 1.) * 0.5;
            assert!(
                (p[0] - expected).abs() <= 1. / 1024.,
                "shape {shape} at {x}: {p:?} != {expected}"
            );
            assert_eq!(p[3], 0.5);
        }
        check_float_viewport(&mut renderer, &value, &pixels);
    }
}

#[test]
fn metal_source_raster_budget_and_dimensions_fail_before_allocation() {
    let mut renderer = Renderer::headless().unwrap();
    for (size, diagnostic) in [(8192, "memory budget"), (20000, "GPU dimensions")] {
        let mut value = fixture("invert", 2, json!({}), json!({}), None);
        value["layers"][0]["effectSpace"] = json!("source");
        value["layers"][0]["d"]["w"] = json!(size);
        value["layers"][0]["d"]["h"] = json!(size);
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
            .unwrap()
            .to_string();
        assert!(error.contains(diagnostic), "{error}");
    }
    let value = fixture("invert", 2, json!({}), json!({}), None);
    let recovered = draw(&mut renderer, &value, &MapResolver::default(), 32);
    assert!(recovered.chunks_exact(4).all(|p| p[3] == 1.));
}

#[test]
fn metal_source_precomp_subframes_seek_and_preview_export_are_exact() {
    let mut value = fixture(
        "setMatte",
        1,
        json!({"channel":0,"composite":false}),
        json!({"map":{"layerId":"nested","stage":"source","space":"source"}}),
        None,
    );
    let mut nested = layer(
        "nested",
        "precomp",
        json!({"comp":"child","w":16,"h":8,"trim":0,"speed":1}),
        1000.,
        1000.,
        1.,
    );
    nested["on"] = json!(false);
    nested["continuousRaster"] = json!(true);
    set(&mut nested, "scale.x", json!(500));
    set(&mut nested, "opacity", json!(0));
    value["layers"].as_array_mut().unwrap().push(nested);
    composition(
        &mut value,
        "child",
        16,
        8,
        1.,
        vec![layer(
            "clock",
            "shader",
            json!({"w":16,"h":8,"code":"void main(){fragColor=vec4(iTime,0.,0.,1.);}","uniforms":{}}),
            0.,
            0.,
            1.,
        )],
    );
    value["colorDepth"] = json!(32);
    let mut doc = load(&value);
    doc.set_saving_app_version(Some("1.3.0".into()));
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_main_effects_for_document(&doc).unwrap();
    let mut seen = vec![];
    for time in [0.125, 0.375, 0.125] {
        let mut outputs = vec![];
        for output in [pm_render::OutputKind::Preview, pm_render::OutputKind::Final] {
            let mut request = RenderRequest::sized(Size::new(32, 32), Precision::Float32);
            request.output_kind = output;
            let frame = renderer
                .render(
                    &doc,
                    "Ptest-root",
                    support::ticks(time),
                    &request,
                    &MapResolver::default(),
                )
                .unwrap();
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            let pixels = renderer.read_rgba(&frame).unwrap();
            assert!(
                pixels.chunks_exact(4).all(|p| p[3] == time as f32),
                "{time}: {:?}",
                &pixels[..4]
            );
            outputs.push(pixels);
        }
        assert_eq!(outputs[0], outputs[1]);
        seen.push(outputs.remove(0));
    }
    assert_eq!(seen[0], seen[2]);
}

#[test]
fn metal_source_owner_inherits_parent_rotation_after_local_processing() {
    let mut value = fixture(
        "gradient",
        2,
        json!({"startX":0,"startY":0,"endX":16,"endY":0,"rampScatter":0}),
        json!({}),
        None,
    );
    let owner = &mut value["layers"][0];
    owner["effectSpace"] = json!("source");
    owner["d"] = json!({"w":16,"h":8,"color":"#FFFFFF"});
    owner["parent"] = json!("rig");
    set(owner, "opacity", json!(50));
    let mut rig = layer("rig", "null", json!({}), 24., 8., 1.);
    set(&mut rig, "rotation", json!(90));
    value["layers"].as_array_mut().unwrap().push(rig);
    let mut renderer = Renderer::headless().unwrap();
    let pixels = draw(&mut renderer, &value, &MapResolver::default(), 32);
    for y in 9..23 {
        let p = &pixels[(y * 32 + 20) * 4..][..4];
        let expected = (y as f32 + 0.5 - 8.) / 16. * 0.5;
        assert!(
            (p[0] - expected).abs() <= 1. / 1024.,
            "at {y}: {p:?} != {expected}"
        );
        assert_eq!(p[3], 0.5);
    }
    assert_eq!(pixels[(4 * 32 + 20) * 4 + 3], 0.);
    check_float_viewport(&mut renderer, &value, &pixels);
}

#[test]
fn metal_source_video_subframes_keep_native_frame_selection_and_seek_identity() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/video/h264-709-limited.mp4");
    assert!(path.is_file(), "required owned video fixture is missing");
    let video = layer(
        "video",
        "video",
        json!({"asset":"clip","w":32,"h":32,"fit":"fill","trim":0,"speed":1}),
        16.,
        16.,
        1.,
    );
    let mut direct = project(32, 32, 1., no_fill(), vec![video.clone()]);
    direct["colorDepth"] = json!(32);
    asset(&mut direct, "clip", "video", "mp4", 256, 144, 1.);
    let mut value = fixture(
        "setMatte",
        1,
        json!({"channel":0,"composite":false}),
        json!({"map":{"layerId":"video","stage":"source","space":"source"}}),
        None,
    );
    value["colorDepth"] = json!(32);
    let mut hidden = video;
    hidden["on"] = json!(false);
    set(&mut hidden, "opacity", json!(0));
    set(&mut hidden, "position.x", json!(10000));
    value["layers"].as_array_mut().unwrap().push(hidden);
    value["assets"] = direct["assets"].clone();
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), path);
    let mut renderer = Renderer::headless().unwrap();
    let mut seen = vec![];
    for time in [0.125, 0.375, 0.125] {
        let mut outputs = vec![];
        for v in [&direct, &value] {
            let mut doc = load(v);
            doc.set_saving_app_version(Some("1.3.0".into()));
            renderer.set_main_effects_for_document(&doc).unwrap();
            let frame = renderer
                .render(
                    &doc,
                    "Ptest-root",
                    support::ticks(time),
                    &RenderRequest::sized(Size::new(32, 32), Precision::Float32),
                    &resolver,
                )
                .unwrap();
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            outputs.push(renderer.read_rgba(&frame).unwrap());
        }
        for (native, reference) in outputs[0].chunks_exact(4).zip(outputs[1].chunks_exact(4)) {
            assert_eq!(
                reference[3],
                native[0].clamp(0., 1.),
                "at {time}: {reference:?} != {native:?}"
            );
        }
        seen.push(outputs.remove(1));
    }
    assert_eq!(seen[0], seen[2]);
}

#[test]
fn metal_source_owner_shutter_samples_accumulate_after_effects_and_opacity() {
    let mut value = fixture(
        "shiftChannels",
        1,
        json!({"takeAlphaFrom":8}),
        json!({}),
        None,
    );
    value["layers"][0]["effectSpace"] = json!("source");
    value["layers"][0]["mblur"] = json!(true);
    set(&mut value["layers"][0], "opacity", json!(50));
    let mut doc = load(&value);
    doc.set_saving_app_version(Some("1.3.0".into()));
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_main_effects_for_document(&doc).unwrap();
    let mut request = RenderRequest::sized(Size::new(32, 32), Precision::Float32);
    request.motion_blur = pm_render::motion::MotionBlur::Samples(4);
    let frame = renderer
        .render(
            &doc,
            "Ptest-root",
            support::ticks(0.5),
            &request,
            &MapResolver::default(),
        )
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let pixels = renderer.read_rgba(&frame).unwrap();
    assert!(
        pixels.chunks_exact(4).all(|p| p[3] == 0.5),
        "sample weights must add exactly: {:?}",
        &pixels[..4]
    );
}

#[test]
fn metal_typed_stroke_reads_provider_geometry_in_the_consuming_owner_space() {
    let mut value = fixture(
        "stroke",
        2,
        json!({"brushSize":4,"hardness":100,"spacing":0,"start":0,"end":100,"color":"#FF0000","opacity":100,"paintStyle":1}),
        json!({"path":{"kind":"path","layerId":"geometry","pathId":"outline"}}),
        None,
    );
    value["layers"][0]["d"] = json!({"w":16,"h":16,"color":"#FFFFFF"});
    set(&mut value["layers"][0], "position.x", json!(8));
    set(&mut value["layers"][0], "position.y", json!(8));
    let mut provider = layer(
        "geometry",
        "solid",
        json!({"w":16,"h":16,"color":"#FFFFFF"}),
        1000.,
        -1000.,
        1.,
    );
    provider["on"] = json!(false);
    provider["masks"] = json!([{"id":"outline","shape":"rect","on":true,"mode":"add","p":{"x":ch(json!(8)),"y":ch(json!(8)),"w":ch(json!(8)),"h":ch(json!(8)),"rotation":ch(json!(0)),"feather":ch(json!(0))}}]);
    value["layers"].as_array_mut().unwrap().push(provider);
    let mut renderer = Renderer::headless().unwrap();
    let pixels = draw(&mut renderer, &value, &MapResolver::default(), 32);
    assert_eq!(&pixels[(12 * 32 + 16) * 4..][..4], &[1., 0., 0., 1.]);
    assert_eq!(pixels[(4 * 32 + 16) * 4 + 3], 0.);
    check_float_viewport(&mut renderer, &value, &pixels);
}

#[test]
fn metal_source_shape_full_raster_preserves_stroke_coverage() {
    let mut renderer = Renderer::headless().unwrap();
    let mut value = fixture("invert", 2, json!({}), json!({}), None);
    value["layers"][0]["type"] = json!("shape");
    value["layers"][0]["d"] =
        json!({"shape":"rect","w":16,"h":16,"color":"#FFFFFF","stroke":4,"strokeColor":"#FFFFFF"});
    value["layers"][0]["effectSpace"] = json!("source");
    set(&mut value["layers"][0], "position.x", json!(16));
    set(&mut value["layers"][0], "position.y", json!(16));
    let with_effect = draw(&mut renderer, &value, &MapResolver::default(), 32);
    value["layers"][0]["fx"] = json!([]);
    let base = draw(&mut renderer, &value, &MapResolver::default(), 32);
    assert!(
        base[(16 * 32 + 7) * 4 + 3] > 0.,
        "stroke must extend outside the nominal box"
    );
    for (a, b) in with_effect.chunks_exact(4).zip(base.chunks_exact(4)) {
        assert_eq!(a[3], b[3]);
    }
}

#[test]
fn metal_source_editable_path_bounds_preserve_authored_coverage() {
    let vertices = [(0,0),(16,0),(0,16)].into_iter().enumerate().map(|(i,(x,y))|
        json!({"id":format!("v{i}"),"p":{"x":ch(json!(x)),"y":ch(json!(y)),"inX":ch(json!(0)),"inY":ch(json!(0)),"outX":ch(json!(0)),"outY":ch(json!(0))}})).collect::<Vec<_>>();
    let mut value = fixture("invert", 2, json!({}), json!({}), None);
    value["layers"][0]["type"] = json!("shape");
    value["layers"][0]["effectSpace"] = json!("source");
    value["layers"][0]["d"] = json!({"paths":[{"id":"triangle","name":"Triangle","parent":null,
        "p":{"x":ch(json!(0)),"y":ch(json!(0)),"rotation":ch(json!(0)),"scaleX":ch(json!(100)),"scaleY":ch(json!(100)),
            "closed":ch(json!(true)),"fill":ch(json!("#FFFFFF")),"fillEnabled":ch(json!(true)),"fillOpacity":ch(json!(100)),
            "stroke":ch(json!("#FFFFFF")),"strokeWidth":ch(json!(0)),"strokeOpacity":ch(json!(100)),"trimStart":ch(json!(0)),
            "trimEnd":ch(json!(100)),"trimOffset":ch(json!(0)),"copies":ch(json!(1)),"repeatX":ch(json!(0)),"repeatY":ch(json!(0)),"repeatRotation":ch(json!(0))},"vertices":vertices}]});
    set(&mut value["layers"][0], "position.x", json!(8));
    set(&mut value["layers"][0], "position.y", json!(8));
    let mut renderer = Renderer::headless().unwrap();
    let with_effect = draw(&mut renderer, &value, &MapResolver::default(), 32);
    value["layers"][0]["fx"] = json!([]);
    let base = draw(&mut renderer, &value, &MapResolver::default(), 32);
    assert_eq!(base[(12 * 32 + 12) * 4 + 3], 1.);
    assert_eq!(base[(24 * 32 + 24) * 4 + 3], 0.);
    for (a, b) in with_effect.chunks_exact(4).zip(base.chunks_exact(4)) {
        assert_eq!(a[3], b[3]);
    }
}

#[test]
fn metal_inactive_source_video_needs_no_decode_or_full_size_target() {
    let mut value = fixture(
        "setMatte",
        1,
        json!({"channel":3,"composite":false}),
        json!({"map":{"layerId":"inactive","stage":"source","space":"source"}}),
        None,
    );
    let mut inactive = layer(
        "inactive",
        "video",
        json!({"asset":"clip","w":8192,"h":8192,"fit":"fill","trim":0,"speed":1}),
        10000.,
        10000.,
        0.25,
    );
    inactive["from"] = json!(0.5);
    inactive["on"] = json!(false);
    value["layers"].as_array_mut().unwrap().push(inactive);
    asset(&mut value, "clip", "video", "mp4", 8192, 8192, 1.);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/video/h264-709-limited.mp4");
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), path);
    let mut renderer = Renderer::headless().unwrap();
    let before = pm_perf::snapshot();
    let pixels = draw(&mut renderer, &value, &resolver, 32);
    assert!(pixels.iter().all(|v| *v == 0.));
    let counters = pm_perf::snapshot().since(&before);
    assert_eq!(counters.get(pm_perf::Counter::MediaCacheMisses), 0);
}

#[test]
fn metal_reference_cache_keeps_each_precomp_density_independent() {
    let mut consumer = layer(
        "consumer",
        "solid",
        json!({"w":8,"h":8,"color":"#FFFFFF"}),
        0.,
        0.,
        1.,
    );
    consumer["fx"] = json!([{"id":"matte","type":"setMatte","version":1,"on":true,"p":{"channel":ch(json!(0)),"composite":ch(json!(false))},"inputs":{"map":{"layerId":"provider","stage":"source"}}}]);
    let mut provider = layer(
        "provider",
        "shader",
        json!({"w":8,"h":8,"code":"void main(){fragColor=vec4(mod(floor(gl_FragCoord.x),2.),0.,0.,1.);}","uniforms":{}}),
        0.,
        0.,
        1.,
    );
    provider["on"] = json!(false);
    let mut first = layer(
        "small",
        "precomp",
        json!({"comp":"child","w":8,"h":8}),
        0.,
        0.,
        1.,
    );
    first["continuousRaster"] = json!(true);
    let mut second = layer(
        "large",
        "precomp",
        json!({"comp":"child","w":8,"h":8}),
        16.,
        0.,
        1.,
    );
    second["continuousRaster"] = json!(true);
    set(&mut second, "scale.x", json!(200));
    set(&mut second, "scale.y", json!(200));
    let mut value = project(32, 32, 1., no_fill(), vec![first, second]);
    composition(&mut value, "child", 8, 8, 1., vec![consumer, provider]);
    value["comps"]["child"]["backgroundFill"] = no_fill();
    let mut renderer = Renderer::headless().unwrap();
    let together = draw(&mut renderer, &value, &MapResolver::default(), 32);
    let layers = value["layers"].as_array().unwrap().clone();
    for (index, x0, size) in [(0, 0, 8), (1, 16, 16)] {
        value["layers"] = json!([layers[index].clone()]);
        let alone = draw(&mut renderer, &value, &MapResolver::default(), 32);
        for y in 0..size {
            for x in x0..x0 + size {
                let i = (y * 32 + x) * 4;
                assert_eq!(
                    &together[i..i + 4],
                    &alone[i..i + 4],
                    "density {index} at {x},{y}"
                );
            }
        }
        assert!(alone.chunks_exact(4).any(|p| p[3] > 0.));
    }
}

#[test]
fn metal_source_atlas_passes_are_charged_before_allocating_them() {
    let mut value = fixture(
        "pixelate",
        2,
        json!({"horizontalBlocks":2048,"verticalBlocks":2048,"sharpColors":false}),
        json!({}),
        None,
    );
    value["layers"][0]["effectSpace"] = json!("source");
    let fx = value["layers"][0]["fx"][0].clone();
    value["layers"][0]["fx"] = json!(
        (0..7)
            .map(|i| {
                let mut fx = fx.clone();
                fx["id"] = json!(format!("atlas{i}"));
                fx
            })
            .collect::<Vec<_>>()
    );
    let mut doc = load(&value);
    doc.set_saving_app_version(Some("1.3.0".into()));
    let mut renderer = Renderer::headless().unwrap();
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
        .unwrap()
        .to_string();
    assert!(error.contains("memory budget"), "{error}");
}

#[test]
fn metal_source_shared_plan_replays_each_count_toward_admission() {
    let mut value = fixture(
        "setMatte",
        1,
        json!({}),
        json!({"map":{"layerId":"shared","stage":"source","space":"source"}}),
        None,
    );
    let owner = value["layers"][0].clone();
    value["layers"] = json!(
        (0..3)
            .map(|i| {
                let mut l = owner.clone();
                l["id"] = json!(format!("consumer{i}"));
                l
            })
            .collect::<Vec<_>>()
    );
    let mut source = layer(
        "shared",
        "solid",
        json!({"w":1280,"h":1280,"color":"#FFFFFF"}),
        0.,
        0.,
        1.,
    );
    source["on"] = json!(false);
    value["layers"].as_array_mut().unwrap().push(source);
    let mut doc = load(&value);
    doc.set_saving_app_version(Some("1.3.0".into()));
    let mut renderer = Renderer::headless().unwrap();
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
        .unwrap()
        .to_string();
    assert!(error.contains("memory budget"), "{error}");
}

/// A cube whose colour axes are not separable, so tetrahedral and trilinear
/// interpolation disagree.
fn twisted_cube(size: usize) -> String {
    let mut text = format!("LUT_3D_SIZE {size}\n");
    let at = |i: usize| i as f64 / (size - 1) as f64;
    for b in 0..size {
        for g in 0..size {
            for r in 0..size {
                let (r, g, b) = (at(r), at(g), at(b));
                text += &format!(
                    "{} {} {}\n",
                    (r * g).sqrt(),
                    0.5 * g + 0.5 * b * b,
                    (b + 0.3 * r * (1.0 - g)).min(1.0)
                );
            }
        }
    }
    text
}

fn lut_layer(input: f64, output: f64) -> Value {
    json!([{
        "id":"effect","type":"applyColorLut","version":2,"on":true,
        "p":{"amount":ch(json!(100)),"inputSpace":ch(json!(input)),"outputSpace":ch(json!(output))},
        "inputs":{"lut":{"kind":"asset","assetId":"grade"}}
    }])
}

fn lut_project(code: &str, fx: Value, adjustment: bool) -> Value {
    let source = layer(
        "owner",
        "shader",
        json!({"w":32,"h":32,"code":code,"uniforms":{}}),
        0.,
        0.,
        1.,
    );
    let mut layers = vec![source];
    if adjustment {
        let mut adj = layer("adj", "adjustment", json!({}), 0., 0., 1.);
        adj["fx"] = fx;
        layers.insert(0, adj);
    } else {
        layers[0]["fx"] = fx;
    }
    let mut value = project(32, 32, 1., no_fill(), layers);
    asset(&mut value, "grade", "lut", "cube", 0, 0, 0.);
    value
}

fn lut_resolver(dir: &tempfile::TempDir, text: &str) -> MapResolver {
    let path = dir.path().join("grade.cube");
    std::fs::write(&path, text).unwrap();
    let mut resolver = MapResolver::default();
    resolver.paths.insert("grade".into(), path);
    resolver
}

/// E-encoded Rec.709 working values for the reference.
fn e_decode(v: f64) -> f64 {
    pm_reference::color::Transfer::Srgb.to_linear(v)
}
fn e_encode(v: f64) -> f64 {
    pm_reference::color::Transfer::Srgb.from_linear(v)
}

#[test]
fn metal_lut_revision_two_is_tetrahedral_and_matches_f64_per_clip_and_adjustment() {
    let dir = tempfile::tempdir().unwrap();
    let text = twisted_cube(5);
    let resolver = lut_resolver(&dir, &text);
    let lut = pm_color::lut::CubeLut::parse(&text).unwrap();
    let mut renderer = Renderer::headless().unwrap();
    for input in [
        [0.125, 0.25, 0.375],
        [0.61, 0.33, 0.87],
        [0.93, 0.07, 0.41],
        [0.2, 0.9, 0.55],
    ] {
        let code = format!(
            "void main(){{fragColor=vec4({:?},{:?},{:?},1.0);}}",
            input[0], input[1], input[2]
        );
        let expected = lut.apply(input).unwrap();
        for adjustment in [false, true] {
            let value = lut_project(&code, lut_layer(0., 0.), adjustment);
            let pixels = draw(&mut renderer, &value, &resolver, 32);
            for c in 0..3 {
                assert!(
                    (f64::from(pixels[c]) - expected[c]).abs() < 1e-5,
                    "adjustment {adjustment}, {input:?}: {:?} {expected:?}",
                    &pixels[..3]
                );
            }
        }
    }
}

#[test]
fn metal_lut_revision_two_reads_large_cubes_shapers_and_keeps_revision_one_exact() {
    let dir = tempfile::tempdir().unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let code = "void main(){fragColor=vec4(0.31,0.62,0.77,1.0);}";
    let input = [0.31f32, 0.62, 0.77].map(f64::from);
    let rows = "0 0 0\n0.1 0.2 0.3\n0.5 0.5 0.5\n0.7 0.8 0.6\n1 1 1\n";
    let cube = twisted_cube(9);
    let (cube_size, cube_rows) = cube.split_once('\n').unwrap();
    for text in [
        twisted_cube(65),
        format!("LUT_1D_SIZE 5\n{rows}"),
        format!("LUT_1D_SIZE 5\n{cube_size}\n{rows}{cube_rows}"),
    ] {
        let resolver = lut_resolver(&dir, &text);
        let lut = pm_color::lut::CubeLut::parse(&text).unwrap();
        let expected = lut.apply(input).unwrap();
        let value = lut_project(code, lut_layer(0., 0.), false);
        let pixels = draw(&mut renderer, &value, &resolver, 32);
        for c in 0..3 {
            assert!(
                (f64::from(pixels[c]) - expected[c]).abs() < 1e-5,
                "{:?} {expected:?}",
                &pixels[..3]
            );
        }
    }
    // Saved revision 1 keeps main's trilinear result, which differs here.
    let text = twisted_cube(3);
    let resolver = lut_resolver(&dir, &text);
    let mut one = lut_project(code, lut_layer(0., 0.), false);
    one["layers"][0]["fx"][0]["version"] = json!(1);
    one["layers"][0]["fx"][0]["p"] = json!({"amount":ch(json!(100))});
    let trilinear = draw(&mut renderer, &one, &resolver, 32);
    let lut = pm_color::lut::CubeLut::parse(&text).unwrap();
    let tetrahedral = lut.apply(input).unwrap();
    assert!(
        (0..3).any(|c| (f64::from(trilinear[c]) - tetrahedral[c]).abs() > 1e-3),
        "{:?} {tetrahedral:?}",
        &trilinear[..3]
    );
}

#[test]
fn metal_lut_spaces_convert_through_the_stated_camera_encoding() {
    use pm_reference::camera_log::Curve;
    use pm_reference::color::{Primaries as P, Transfer as T, apply, invert, mul};
    let dir = tempfile::tempdir().unwrap();
    // Identity table over 0..2 in every encoding: what leaves the table is the
    // encoded value, so the path out through the same space restores the pixel.
    let identity =
        "DOMAIN_MIN -0.5 -0.5 -0.5\nDOMAIN_MAX 2 2 2\nLUT_1D_SIZE 2\n-0.5 -0.5 -0.5\n2 2 2\n";
    // Half the encoded value: output must decode through the space.
    let half =
        "DOMAIN_MIN -0.5 -0.5 -0.5\nDOMAIN_MAX 2 2 2\nLUT_1D_SIZE 2\n-0.25 -0.25 -0.25\n1 1 1\n";
    let mut renderer = Renderer::headless().unwrap();
    let code = "void main(){fragColor=vec4(0.40,0.55,0.70,1.0);}";
    let input = [0.40f64, 0.55, 0.70];
    let rec709 = input.map(e_decode);
    let log = |curve: Curve,
               gamut: P|
     -> (
        Box<dyn Fn([f64; 3]) -> [f64; 3]>,
        Box<dyn Fn([f64; 3]) -> [f64; 3]>,
    ) {
        let to = mul(&invert(&gamut.to_xyz()), &P::BT709.to_xyz());
        let from = invert(&to);
        (
            Box::new(move |l| apply(&to, l).map(|v| curve.from_linear(v))),
            Box::new(move |e| apply(&from, e.map(|v| curve.to_linear(v)))),
        )
    };
    type Pair = (
        Box<dyn Fn([f64; 3]) -> [f64; 3]>,
        Box<dyn Fn([f64; 3]) -> [f64; 3]>,
    );
    let pq = || -> Pair {
        let to = mul(&invert(&P::BT2020.to_xyz()), &P::BT709.to_xyz());
        let from = invert(&to);
        (
            Box::new(move |l| {
                apply(&to, l).map(|v| T::Pq.from_linear((v * 203.0 / 10000.0).max(0.0)))
            }),
            Box::new(move |e| apply(&from, e.map(|v| T::Pq.to_linear(v) * 10000.0 / 203.0))),
        )
    };
    let spaces: Vec<(f64, Pair)> = vec![
        (2., log(Curve::AppleLog, P::BT2020)),
        (3., log(Curve::SLog3, P::S_GAMUT3_CINE)),
        (4., log(Curve::SLog3, P::S_GAMUT3)),
        (5., log(Curve::VLog, P::V_GAMUT)),
        (6., log(Curve::LogC3, P::AWG3)),
        (7., log(Curve::LogC4, P::AWG4)),
        (8., pq()),
    ];
    for (space, (encode, decode)) in &spaces {
        let value = lut_project(code, lut_layer(*space, *space), false);
        let resolver = lut_resolver(&dir, identity);
        let pixels = draw(&mut renderer, &value, &resolver, 32);
        for c in 0..3 {
            assert!(
                (f64::from(pixels[c]) - input[c]).abs() < 2e-3,
                "space {space} round trip: {:?}",
                &pixels[..3]
            );
        }
        let resolver = lut_resolver(&dir, half);
        let pixels = draw(&mut renderer, &value, &resolver, 32);
        let encoded = encode(rec709);
        let halved = decode(encoded.map(|v| v / 2.0));
        for c in 0..3 {
            let expected = e_encode(halved[c]);
            assert!(
                (f64::from(pixels[c]) - expected).abs() < 2e-3,
                "space {space}: {:?} {expected}",
                &pixels[..3]
            );
        }
    }
}
