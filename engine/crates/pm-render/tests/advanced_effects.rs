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
    let mut document = load(value);
    document.set_saving_app_version(Some("1.3.0".into()));
    let mut request = RenderRequest::sized(Size::new(16, 12), Precision::Float16);
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
