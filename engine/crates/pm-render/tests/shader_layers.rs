//! Shader layers, extension fragment layers, and extension mesh layers
//! (plan sections 10 and 11): the regression probes for the compositor's
//! binding rules and the plan, with values that need no WebGL reference
//! (each program writes the uniforms it reads; the expected values are
//! today's GL semantics, `compositor.ts:1015-1127`). The WebGL comparison of
//! every corpus shader is `tests/shader_gate.rs`; the app captures are
//! `tests/corpus.rs` (`layers/shader-*`, `layers/studio-cube`,
//! `layers/obj-model`).

use pm_protocol::color::Precision;
use pm_protocol::common::{Seq, Size, Ticks};
use pm_protocol::definitions::{DefinitionSetSnapshot, DefinitionSnapshot};
use pm_render::{Feature, MapResolver, RenderError, RenderRequest, Renderer};
use serde_json::{Value, json};

fn ch(v: Value) -> Value {
    json!({ "v": v, "kf": [], "expr": null })
}

fn layer(id: &str, ty: &str, d: Value, from: f64, dur: f64) -> Value {
    json!({
        "id": id, "type": ty, "name": id, "from": from, "dur": dur, "on": true, "lock": false,
        "threeD": false, "solo": false, "blend": "normal", "mblur": false, "parent": null,
        "group": null, "matteSource": null,
        "p": {
            "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(0)),
            "position.y": ch(json!(0)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
            "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
        },
        "fx": [], "transitionIn": null, "transitionOut": null, "masks": [], "d": d
    })
}

fn document(w: u32, h: u32, fps: f64, layers: Vec<Value>) -> pm_model::Document {
    let doc = json!({
        "id": "P", "name": "p", "w": w, "h": h, "fps": fps, "dur": 10, "bg": "#000000",
        "backgroundFill": { "type": "none", "angle": 0, "stops": [{ "id": "s1", "color": "#000000", "position": 0 }] },
        "compId": "P-root", "compName": "p", "markers": [], "work": [0, 10], "params": {},
        "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "layers": layers, "assets": {}, "comps": {}
    });
    pm_model::Document::new(Some(pm_model::json::parse(&doc.to_string()).unwrap()), 1)
}

fn ticks(seconds: f64) -> Ticks {
    Ticks::new((seconds * pm_time::TICKS_PER_SECOND as f64).round() as i64).unwrap()
}

/// The frame's top-left pixel, premultiplied f32 (`fragColor` straight
/// through, over transparency).
fn render(
    r: &mut Renderer,
    doc: &pm_model::Document,
    size: (u32, u32),
    t: f64,
) -> Result<(Vec<f32>, Vec<pm_render::LayerFailure>), RenderError> {
    let mut request = RenderRequest::sized(Size::new(size.0, size.1), Precision::Float16);
    request.background = false;
    let frame = r.render(doc, "P-root", ticks(t), &request, &MapResolver::default())?;
    let px = r.read_rgba(&frame).unwrap();
    Ok((px, frame.failures))
}

fn close(a: &[f32], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1.0 / 1024.0)
}

fn shader(code: &str, uniforms: Value) -> Value {
    json!({ "code": code, "uniforms": uniforms.as_object().unwrap().iter().map(|(k, v)| (k.clone(), ch(v.clone()))).collect::<serde_json::Map<_, _>>() })
}

#[test]
fn uniform_binding_follows_the_compositor_quirks() {
    let mut r = Renderer::headless().unwrap();
    let decls = "uniform float uF;   // @param 0.25 0 1\nuniform vec2 uV2;   // @param 0.5\nuniform vec4 uV4;   // @param 0.5\nuniform int uI;     // @param 3 0 9\nuniform bool uB;    // @param true\nuniform vec3 uC;    // @param #FF8000\nuniform vec3 uDir;  // @param 1 0 0\nuniform float uNoAnn;\n";
    let values = json!({ "uF": 0.25, "uV2": 0.5, "uV4": 0.5, "uI": 3, "uB": true, "uC": "#FF8000", "uDir": 1, "uNoAnn": 0.75 });
    let cases: [(&str, [f32; 4]); 3] = [
        // A float binds; one float never sets a vec2 or a vec4 (they stay 0).
        (
            "fragColor = vec4(uF, uV2.x + uV2.y, uV4.x + uV4.w, 1.0);",
            [0.25, 0.0, 0.0, 1.0],
        ),
        // `uniform1f` never sets an int; a toggle is `uniform1i` on the bool;
        // a `#` default makes a vec3 a color (`uniform3f(hex2rgb)`).
        (
            "fragColor = vec4(float(uI) / 9.0, uB ? 1.0 : 0.0, uC.g, 1.0);",
            [0.0, 1.0, 128.0 / 255.0, 1.0],
        ),
        // A vec3 without a color default gets one float (stays 0); an
        // unannotated uniform is still a number control.
        (
            "fragColor = vec4(uDir.x, uNoAnn, 0.0, 1.0);",
            [0.0, 0.75, 0.0, 1.0],
        ),
    ];
    for (body, expected) in cases {
        let code = format!("{decls}void main(){{ {body} }}");
        let doc = document(
            16,
            8,
            30.0,
            vec![layer(
                "L",
                "shader",
                shader(&code, values.clone()),
                0.0,
                4.0,
            )],
        );
        let (px, failures) = render(&mut r, &doc, (16, 8), 1.0).unwrap();
        assert!(failures.is_empty(), "{failures:?}");
        assert!(
            close(&px[..4], expected),
            "{body}: {:?} vs {expected:?}",
            &px[..4]
        );
    }
    // `d.uniforms` binds in its own order after the header's uniforms: a
    // stored `iTime` overrides the layer-local time (the loop sets it last).
    let doc = document(
        16,
        8,
        30.0,
        vec![layer(
            "L",
            "shader",
            shader(
                "void main(){ fragColor = vec4(iTime / 10.0, 0.0, 0.0, 1.0); }",
                json!({ "iTime": 5 }),
            ),
            0.0,
            4.0,
        )],
    );
    let (px, _) = render(&mut r, &doc, (16, 8), 1.0).unwrap();
    assert!(close(&px[..4], [0.5, 0.0, 0.0, 1.0]), "{:?}", &px[..4]);
}

#[test]
fn time_frame_mouse_and_resolution_uniforms() {
    let mut r = Renderer::headless().unwrap();
    let mut d = shader(
        "void main(){ fragColor = vec4(iTime, iGlobalTime / 2.0, iProgress, float(iFrame) / 60.0); }",
        json!({}),
    );
    d["w"] = json!(80);
    d["h"] = json!(45);
    // At 1.25 s in a 24 fps composition, a layer from 0.5 s for 2 s:
    // iTime 0.75, iGlobalTime 1.25, iProgress 0.375, iFrame round(1.25 · 24) = 30.
    let doc = document(
        160,
        90,
        24.0,
        vec![layer("L", "shader", d.clone(), 0.5, 2.0)],
    );
    let (px, _) = render(&mut r, &doc, (160, 90), 1.25).unwrap();
    assert!(close(&px[..4], [0.75, 0.625, 0.375, 0.5]), "{:?}", &px[..4]);
    // iMouse is (0, 0); iResolution is the target: the layer's size at the
    // output scale (80 × 45 at 2×).
    d["code"] = json!("void main(){ fragColor = vec4(iMouse, iResolution / 1000.0); }");
    let doc = document(160, 90, 24.0, vec![layer("L", "shader", d, 0.5, 2.0)]);
    let (px, _) = render(&mut r, &doc, (320, 180), 1.25).unwrap();
    assert!(close(&px[..4], [0.0, 0.0, 0.16, 0.09]), "{:?}", &px[..4]);
}

#[test]
fn targets_are_capped_at_4096_pixels() {
    let mut r = Renderer::headless().unwrap();
    let mut d = shader(
        "void main(){ fragColor = vec4(iResolution.x / 8192.0, iResolution.y / 8192.0, 0.0, 1.0); }",
        json!({}),
    );
    d["w"] = json!(20000);
    d["h"] = json!(10);
    let doc = document(64, 32, 30.0, vec![layer("L", "shader", d, 0.0, 4.0)]);
    let (px, _) = render(&mut r, &doc, (64, 32), 1.0).unwrap();
    assert!(
        close(&px[..4], [0.5, 10.0 / 8192.0, 0.0, 1.0]),
        "{:?}",
        &px[..4]
    );
}

#[test]
fn code_webgl_refuses_draws_nothing_and_is_reported() {
    let mut r = Renderer::headless().unwrap();
    let doc = document(
        16,
        8,
        30.0,
        vec![layer(
            "L",
            "shader",
            shader("void main(){ fragColor = nope; }", json!({})),
            0.0,
            4.0,
        )],
    );
    assert!(pm_render::unsupported(&doc, "P-root").is_empty());
    let (px, failures) = render(&mut r, &doc, (16, 8), 1.0).unwrap();
    assert!(px.iter().all(|v| *v == 0.0));
    assert_eq!(failures.len(), 1);
    assert!(
        failures[0].message.contains("does not compile"),
        "{}",
        failures[0].message
    );
    // `d.code` missing is the text "undefined": it does not compile either.
    let doc = document(
        16,
        8,
        30.0,
        vec![layer("L", "shader", json!({ "uniforms": {} }), 0.0, 4.0)],
    );
    let (px, failures) = render(&mut r, &doc, (16, 8), 1.0).unwrap();
    assert!(px.iter().all(|v| *v == 0.0) && failures.len() == 1);
}

fn fragment_definition(id: &str, fragment: &str) -> DefinitionSnapshot {
    let header = pm_render::shader_layers::header();
    let uniforms = "uniform float u_amount;\nuniform vec3 u_tint;\nuniform bool u_on;";
    let source = format!("{header}\n{uniforms}\n{fragment}");
    let offset = source[..source.len() - fragment.len()]
        .matches('\n')
        .count();
    serde_json::from_value(json!({
        "id": id, "kind": "layer", "version": 1, "origin": "local",
        "renderer": { "kind": "fragment", "fragment": fragment },
        "glsl": { "source": source, "bodyLineOffset": offset, "bodyUtf16Len": fragment.encode_utf16().count(), "bodyUtf8Len": fragment.len() },
        "params": [
            { "type": "number", "key": "amount", "default": 0.5, "min": 0, "max": 1, "step": null },
            { "type": "color", "key": "tint", "default": "#000080" },
            { "type": "toggle", "key": "on", "default": false }
        ],
        "flags": { "passes": 1, "keepOrig": false, "backdrop": false, "rawShader": false, "viewportSafe": false, "viewportPadding": [], "clamp": false }
    }))
    .unwrap()
}

fn set(definitions: Vec<DefinitionSnapshot>, missing: Vec<&str>) -> DefinitionSetSnapshot {
    DefinitionSetSnapshot {
        generation: Seq::new(1).unwrap(),
        definitions,
        missing: missing.into_iter().map(Into::into).collect(),
    }
}

#[test]
fn extension_fragment_layers_bind_their_parameters() {
    let mut r = Renderer::headless().unwrap();
    let def = fragment_definition(
        "example.orb",
        "void main(){ fragColor = vec4(u_amount, u_tint.b, u_on ? 1.0 : 0.25, 1.0); }",
    );
    let ext = |params: Value| {
        layer(
            "L",
            "extension",
            json!({ "definition": "example.orb", "version": 1, "params": params, "data": {} }),
            0.0,
            4.0,
        )
    };
    // Without definitions the layer routes, as before.
    let doc = document(16, 8, 30.0, vec![ext(json!({}))]);
    assert!(
        pm_render::unsupported(&doc, "P-root")
            .iter()
            .any(|u| matches!(u.feature, Feature::ExtensionDefinition(_)))
    );
    assert!(matches!(
        render(&mut r, &doc, (16, 8), 1.0),
        Err(RenderError::Unsupported(_))
    ));
    r.set_definitions(&set(vec![def], vec![])).unwrap();
    assert!(pm_render::unsupported_with_definitions(&doc, "P-root", None, r.effects()).is_empty());
    // No properties: the defaults (`param.def`), a toggle off → `uniform1i(0)`.
    let (px, _) = render(&mut r, &doc, (16, 8), 1.0).unwrap();
    assert!(
        close(&px[..4], [0.5, 128.0 / 255.0, 0.25, 1.0]),
        "{:?}",
        &px[..4]
    );
    // Properties evaluated: a number, a short color, a toggle on (`uniform bool`).
    let doc = document(
        16,
        8,
        30.0,
        vec![ext(
            json!({ "amount": ch(json!(0.125)), "tint": ch(json!("#00F")), "on": ch(json!(true)) }),
        )],
    );
    let (px, _) = render(&mut r, &doc, (16, 8), 1.0).unwrap();
    assert!(close(&px[..4], [0.125, 1.0, 1.0, 1.0]), "{:?}", &px[..4]);
}

#[test]
fn missing_extension_definitions_draw_the_checkerboard_and_unknown_ones_route() {
    let mut r = Renderer::headless().unwrap();
    let doc = document(
        48,
        48,
        30.0,
        vec![layer(
            "L",
            "extension",
            json!({ "definition": "com.example.absent", "params": {}, "data": {} }),
            0.0,
            4.0,
        )],
    );
    r.set_definitions(&pm_render::all_builtin_definitions())
        .unwrap();
    let issues = pm_render::unsupported_with_definitions(&doc, "P-root", None, r.effects());
    assert!(
        issues
            .iter()
            .any(|u| u.feature == Feature::Definition("layer com.example.absent".into())),
        "{issues:?}"
    );
    let mut builtins = pm_render::all_builtin_definitions();
    builtins.missing.push("com.example.absent".into());
    r.set_definitions(&builtins).unwrap();
    assert!(pm_render::unsupported_with_definitions(&doc, "P-root", None, r.effects()).is_empty());
    let (px, _) = render(&mut r, &doc, (48, 48), 1.0).unwrap();
    // `floor(uv · 24)`: 2 px squares alternating (.055, .05, .07) and (.16, .09, .15).
    let at = |x: usize, y: usize| &px[(y * 48 + x) * 4..][..4];
    let a = [0.055, 0.05, 0.07, 1.0];
    let b = [0.16, 0.09, 0.15, 1.0];
    let near = |p: &[f32], c: [f32; 4]| p.iter().zip(c).all(|(x, y)| (x - y).abs() < 2e-3);
    assert!(near(at(0, 0), a) || near(at(0, 0), b), "{:?}", at(0, 0));
    assert!(
        near(at(2, 0), if near(at(0, 0), a) { b } else { a }),
        "{:?}",
        at(2, 0)
    );
}

#[test]
fn third_party_mesh_layers_render_from_their_snapshot() {
    let mut r = Renderer::headless().unwrap();
    let mesh: DefinitionSnapshot = serde_json::from_value(json!({
        "id": "example.obj", "kind": "layer", "version": 1, "origin": "local",
        "renderer": { "kind": "mesh", "assetField": "model" }, "glsl": null,
        "params": [{ "type": "color", "key": "background", "default": "#102030" }],
        "flags": { "passes": 1, "keepOrig": false, "backdrop": false, "rawShader": false, "viewportSafe": false, "viewportPadding": [], "clamp": false }
    }))
    .unwrap();
    let doc = document(
        32,
        18,
        30.0,
        vec![layer(
            "L",
            "extension",
            json!({ "definition": "example.obj", "params": {}, "data": {} }),
            0.0,
            4.0,
        )],
    );
    // Routed until its definition snapshot arrives.
    assert!(
        pm_render::unsupported(&doc, "P-root")
            .iter()
            .any(|u| u.feature == Feature::ExtensionDefinition("example.obj".into()))
    );
    r.set_definitions(&set(vec![mesh], vec![])).unwrap();
    assert!(pm_render::unsupported_with_definitions(&doc, "P-root", None, r.effects()).is_empty());
    // Without an OBJ asset: the compositor's missing-mesh checkerboard.
    let (px, failures) = render(&mut r, &doc, (32, 18), 1.0).unwrap();
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(px[3], 1.0, "drawn");
}

/// Never asked: these documents have no text layers.
struct NoText;

impl pm_eval::text_geom::TextMeasure for NoText {
    fn measure_lines(
        &mut self,
        _: &pm_text::compat::TextContent,
    ) -> Vec<pm_eval::text_geom::MeasuredLine> {
        unreachable!()
    }
    fn plain_width(&mut self, _: &pm_text::compat::TextContent, _: &str) -> f64 {
        unreachable!()
    }
    fn glyph_layout(&mut self, _: &pm_text::compat::TextContent) -> pm_text::compat::GlyphLayout {
        unreachable!()
    }
    fn spaced_width(&mut self, _: &pm_text::compat::TextContent, _: &str) -> f64 {
        unreachable!()
    }
}

#[test]
fn shader_extension_and_obj_layers_pick_by_their_rectangle() {
    // `GL.pick` (pm-eval's port): no 3D raycast for these kinds; the legacy
    // OBJ layer with its default (truthy) background picks by rectangle too.
    let mut d = shader("void main(){ fragColor = vec4(1.0); }", json!({}));
    d["w"] = json!(100);
    d["h"] = json!(50);
    let mut obj = layer(
        "obj",
        "extension",
        json!({ "definition": "powermove.3d.obj-model", "w": 100, "h": 50, "params": {}, "data": { "assetId": "a1" } }),
        0.0,
        4.0,
    );
    obj["p"]["position.x"] = ch(json!(300));
    let mut ext = layer(
        "ext",
        "extension",
        json!({ "definition": "example.orb", "w": 100, "h": 50, "params": {}, "data": {} }),
        0.0,
        4.0,
    );
    ext["p"]["position.x"] = ch(json!(150));
    let doc = document(
        640,
        360,
        30.0,
        vec![layer("sh", "shader", d, 0.0, 4.0), ext, obj],
    );
    let comp = pm_eval::Comp::resolve(&doc, "P-root").unwrap();
    let mut ev = pm_eval::Evaluator::new();
    let mut scope = pm_eval::Scope::open(&mut ev, &comp);
    let opts = pm_eval::bounds::PickOptions::default();
    let mut pick = |x: f64, y: f64| {
        scope
            .pick(x, y, 1.0, &opts, &mut NoText, &mut pm_eval::bounds::NoHost)
            .unwrap()
            .map(|n| pm_eval::node::to_js_string(n.get("id")).to_string_lossy())
    };
    assert_eq!(pick(50.0, 25.0).as_deref(), Some("sh"));
    assert_eq!(pick(200.0, 25.0).as_deref(), Some("ext"));
    assert_eq!(pick(350.0, 25.0).as_deref(), Some("obj"));
    assert_eq!(pick(50.0, 60.0), None);
}

/// `PM.parseUniforms` against the app's own function on the presets, the
/// generated agent shader layers, and edge cases of its regular expression
/// and `parseFloat` (`tools/shaders/parse-uniforms-fixture.ts`).
#[test]
fn parse_uniforms_matches_the_app() {
    use pm_render::shader_layers::{Control, parse_uniforms};
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/parse-uniforms.json")).unwrap();
    let js_number = |s: &str| match s {
        "NaN" => f64::NAN,
        "Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        other => other.parse().unwrap(),
    };
    let same = |a: f64, b: f64| (a.is_nan() && b.is_nan()) || a == b;
    let mut cases = 0;
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let ours = parse_uniforms(case["code"].as_str().unwrap());
        let theirs = case["defs"].as_array().unwrap();
        assert_eq!(ours.len(), theirs.len(), "{name}: {ours:?}");
        for (o, t) in ours.iter().zip(theirs) {
            assert_eq!(o.name, t["name"].as_str().unwrap(), "{name}");
            assert_eq!(o.ty.name(), t["type"].as_str().unwrap(), "{name}");
            assert_eq!(o.label, t["label"].as_str().unwrap(), "{name}");
            match (&o.control, t["control"].as_str().unwrap()) {
                (Control::Color { default }, "color") => {
                    assert_eq!(default, t["def"].as_str().unwrap(), "{name}")
                }
                (Control::Toggle { default }, "toggle") => {
                    assert_eq!(*default, t["def"].as_bool().unwrap(), "{name}")
                }
                (Control::Number { default, min, max }, "num") => {
                    assert!(
                        same(*default, js_number(t["def"].as_str().unwrap())),
                        "{name} {}",
                        o.name
                    );
                    assert!(
                        same(*min, js_number(t["min"].as_str().unwrap())),
                        "{name} {} min {min}",
                        o.name
                    );
                    assert!(
                        same(*max, js_number(t["max"].as_str().unwrap())),
                        "{name} {} max {max}",
                        o.name
                    );
                }
                (c, t) => panic!("{name} {}: {c:?} vs {t}", o.name),
            }
        }
        cases += 1;
    }
    assert!(cases >= 40, "{cases} cases");
}
