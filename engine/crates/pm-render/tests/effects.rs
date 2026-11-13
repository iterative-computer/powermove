//! Effects and transitions through `Renderer::render` (plan sections 8 and
//! 11, contract 10): staging, definitions and their lifecycle, missing and
//! unknown definitions, refused and fallback programs, animatable enables,
//! `clamp`, regions of interest, groups and precomps, and the GPU watchdog.
//! Pixel parity with today's compositor is `effects_webgl.rs` (shader
//! level) and `corpus.rs` (the `render/fx-*` and `render/tr-*` captures).

mod support;

use std::time::Duration;

use pm_protocol::color::Precision;
use pm_protocol::common::{PixelRect, Seq, Size};
use pm_protocol::definitions::{
    DefinitionKind, DefinitionRemoval, DefinitionSetSnapshot, DefinitionSnapshot,
    DefinitionsUpdate, UpdateError,
};
use pm_protocol::error::{ErrorCode, ErrorScope};
use pm_render::effects::watchdog::{FnTiming, Span, Subject, TimingSource};
use pm_render::{
    Feature, MapResolver, RenderError, RenderRequest, Renderer, builtin_definitions, unsupported,
    unsupported_with_definitions,
};
use serde_json::{Value, json};
use support::doc::*;
use support::{repo_root, ticks};

/// The tests take turns on the GPU: Metal left the watchdog's timestamp
/// samples unwritten while several test devices were busy at once (one
/// device per process in the engines).
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|e| e.into_inner())
}

const W: u32 = 320;
const H: u32 = 180;

fn fx(id: &str, ty: &str, p: Value) -> Value {
    json!({ "id": id, "type": ty, "on": true, "p": p })
}

/// A background, an image with `effects`, and a solid chip.
fn document(
    effects: Vec<Value>,
    chip_transition: Option<Value>,
) -> (pm_model::Document, MapResolver) {
    let dur = 2.0;
    let mut chip = layer(
        "chip",
        "solid",
        json!({ "color": "#3FCF8E", "w": 70, "h": 50 }),
        220.0,
        20.0,
        dur,
    );
    set(&mut chip, "rotation", json!(12));
    if let Some(t) = chip_transition {
        chip["transitionIn"] = t;
    }
    let mut image = layer(
        "img",
        "image",
        json!({ "asset": "still", "fit": "stretch", "w": 160, "h": 90 }),
        60.0,
        40.0,
        dur,
    );
    set(&mut image, "rotation", json!(-8));
    image["fx"] = Value::Array(effects);
    let back = layer(
        "back",
        "solid",
        json!({ "color": "#2A2F45", "w": 300, "h": 150 }),
        10.0,
        15.0,
        dur,
    );
    let mut doc = project(W, H, dur, solid_fill("#14161C"), vec![chip, image, back]);
    asset(&mut doc, "still", "image", "png", 160, 90, 0.0);
    let mut resolver = MapResolver::default();
    resolver.paths.insert(
        "still".into(),
        repo_root().join("engine/fixtures/still-alpha.png"),
    );
    (load(&doc), resolver)
}

fn request() -> RenderRequest {
    RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float16)
}

fn render(
    r: &mut Renderer,
    doc: &pm_model::Document,
    res: &MapResolver,
    t: f64,
    req: &RenderRequest,
) -> Result<Vec<f32>, RenderError> {
    let frame = r.render(doc, "Ptest-root", ticks(t), req, res)?;
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    Ok(r.read_rgba(&frame).unwrap())
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

fn builtins() -> Renderer {
    let mut r = Renderer::headless().unwrap();
    let failures = r.set_definitions(&builtin_definitions()).unwrap();
    assert!(failures.is_empty(), "{failures:?}");
    r
}

/// A raw effect snapshot (`rawShader`: the full program).
fn raw_effect(id: &str, source: &str, params: Value, clamp: bool) -> DefinitionSnapshot {
    serde_json::from_value(json!({
        "id": id, "kind": "effect", "version": 0, "origin": "local", "renderer": null,
        "glsl": { "source": source, "bodyLineOffset": 0, "bodyUtf16Len": source.encode_utf16().count(), "bodyUtf8Len": source.len() },
        "params": params,
        "flags": { "passes": 1, "keepOrig": false, "backdrop": false, "rawShader": true, "viewportSafe": false, "viewportPadding": [], "clamp": clamp }
    }))
    .unwrap()
}

const GAIN: &str = "#version 300 es\nprecision highp float;\nin vec2 v_st;\nuniform sampler2D u_tex;\nuniform float u_gain;\nout vec4 o;\nvoid main(){ o = texture(u_tex, v_st) * u_gain; }\n";

fn gain_params() -> Value {
    json!([{ "type": "number", "key": "gain", "default": 4, "min": 0, "max": 10, "step": null }])
}

fn update(
    generation: u64,
    full: bool,
    upserts: Vec<DefinitionSnapshot>,
    removals: Vec<DefinitionRemoval>,
) -> DefinitionsUpdate {
    DefinitionsUpdate {
        generation: Seq::new(generation).unwrap(),
        full,
        upserts,
        removals,
    }
}

/// Metal's render-target conversion truncates f32 to half; a legacy program
/// drawing into an RGBA16F target stores the nearest even half instead
/// (`pm_render::glsl::half_store`), so a chain of passes is not biased down
/// by a half step per pass. 0.3 lies at 204.8 half steps of its binade:
/// truncation stores 0x34CC, the nearest half is 0x34CD.
#[test]
fn rgba16f_stores_the_nearest_half_of_a_programs_output() {
    let _gpu = gpu();
    const CONSTANT: &str = "#version 300 es\nprecision highp float;\nin vec2 v_st;\nuniform sampler2D u_tex;\nout vec4 o;\nvoid main(){ o = vec4(0.3, -0.3, 1.0 + 3.0 * 0.0003, 1.0); }\n";
    let mut set = builtin_definitions();
    set.definitions
        .push(raw_effect("acme.constant", CONSTANT, json!([]), false));
    let mut r = Renderer::headless().unwrap();
    assert!(r.set_definitions(&set).unwrap().is_empty());
    let def = r
        .effects()
        .get(DefinitionKind::Effect, "acme.constant")
        .unwrap()
        .clone();
    let input = pm_gpu::Target::new(
        r.device(),
        "input",
        4,
        2,
        wgpu::TextureFormat::Rgba16Float,
        pm_gpu::target::working_usage(),
    );
    let step = pm_render::effects::plan::effect_step(&def, "fx", &[]);
    let out = r
        .run_effects(&input, None, &[step], 0.0, None, (4, 2), Precision::Float16)
        .unwrap();
    assert_eq!(out.format(), wgpu::TextureFormat::Rgba16Float);
    let raw = pm_gpu::readback::read_raw(r.device(), r.queue(), &out.texture).unwrap();
    let halves: Vec<u16> = raw
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let nearest = pm_gpu::readback::f32_to_half;
    let want = [
        nearest(0.3),
        nearest(-0.3),
        nearest(1.0 + 3.0 * 0.0003),
        0x3c00,
    ];
    assert_eq!(want[0], 0x34cd);
    for px in halves.chunks_exact(4) {
        assert_eq!(px, want, "{px:04x?}");
    }
}

/// GLSL built-ins whose Metal call the engine takes from ANGLE
/// (`pm_render::glsl`): `round` rounds ties away from zero as WebGL does
/// today (ANGLE's `metal::round`), `roundEven` to even (`metal::rint`).
#[test]
fn round_ties_away_from_zero_as_in_webgl() {
    let _gpu = gpu();
    const ROUND: &str = "#version 300 es\nprecision highp float;\nin vec2 v_st;\nuniform sampler2D u_tex;\nuniform float u_x;\nout vec4 o;\nvoid main(){ o = vec4(round(u_x) * 0.25, roundEven(u_x) * 0.25, round(-u_x - 1.0) * -0.125, 1.0); }\n";
    let params = json!([{ "type": "number", "key": "x", "default": 0.5, "min": 0, "max": 10, "step": null }]);
    let mut set = builtin_definitions();
    set.definitions
        .push(raw_effect("acme.round", ROUND, params, false));
    let mut r = Renderer::headless().unwrap();
    let failures = r.set_definitions(&set).unwrap();
    assert!(failures.is_empty(), "{failures:?}");
    for (x, want) in [
        (0.5, [0.25, 0.0, 0.25]),
        (2.5, [0.75, 0.5, 0.5]),
        (1.25, [0.25, 0.25, 0.25]),
    ] {
        let (doc, res) = document(
            vec![fx("f1", "acme.round", json!({ "x": ch(json!(x)) }))],
            None,
        );
        let px = render(&mut r, &doc, &res, 0.5, &request()).unwrap();
        // The effect writes every pixel of the layer's buffer; the image
        // layer is opaque where its alpha is, so read the composite where
        // nothing else is drawn over it: the frame's first pixel.
        let i = 0;
        let got = [px[i], px[i + 1], px[i + 2]];
        for c in 0..3 {
            assert!(
                (got[c] - want[c]).abs() < 1e-3,
                "x = {x}: {got:?}, want {want:?}"
            );
        }
    }
}

#[test]
fn effects_are_routed_without_definitions_and_drawn_with_them() {
    let _gpu = gpu();
    let (doc, res) = document(
        vec![fx("f1", "blur", json!({ "amount": ch(json!(12)) }))],
        None,
    );
    let mut bare = Renderer::headless().unwrap();
    match render(&mut bare, &doc, &res, 0.5, &request()) {
        Err(RenderError::Unsupported(u)) => {
            assert!(u.iter().any(|u| u.feature == Feature::Effects), "{u:?}")
        }
        other => panic!("expected routing, got {:?}", other.map(|_| ())),
    }
    assert!(
        unsupported(&doc, "Ptest-root")
            .iter()
            .any(|u| u.feature == Feature::Effects)
    );
    let mut r = builtins();
    assert!(unsupported_with_definitions(&doc, "Ptest-root", None, r.effects()).is_empty());
    let a = render(&mut r, &doc, &res, 0.5, &request()).unwrap();
    let b = render(&mut r, &doc, &res, 0.5, &request()).unwrap();
    assert_eq!(a, b, "deterministic");
    let (plain, _) = document(vec![], None);
    let p = render(&mut r, &plain, &res, 0.5, &request()).unwrap();
    assert!(max_diff(&a, &p) > 0.05, "the blur changes the frame");
}

#[test]
fn missing_definitions_are_skipped_and_unknown_ones_routed() {
    let _gpu = gpu();
    let gone = fx("f1", "com.example.gone", json!({ "amount": ch(json!(3)) }));
    let (doc, res) = document(vec![gone.clone()], None);
    let mut r = builtins();
    match render(&mut r, &doc, &res, 0.5, &request()) {
        Err(RenderError::Unsupported(u)) => {
            assert!(
                u.iter()
                    .any(|u| u.feature == Feature::Definition("effect com.example.gone".into())),
                "{u:?}"
            )
        }
        other => panic!("expected routing, got {:?}", other.map(|_| ())),
    }
    let mut set = builtin_definitions();
    set.missing.push("com.example.gone".into());
    r.set_definitions(&set).unwrap();
    assert!(unsupported_with_definitions(&doc, "Ptest-root", None, r.effects()).is_empty());
    let skipped = render(&mut r, &doc, &res, 0.5, &request()).unwrap();
    let (plain, _) = document(vec![], None);
    let p = render(&mut r, &plain, &res, 0.5, &request()).unwrap();
    // The layer buffer path (rgba16float) without an effect: the same picture.
    assert!(
        max_diff(&skipped, &p) <= 1.0 / 1024.0,
        "{}",
        max_diff(&skipped, &p)
    );
    // An instance the document marks missing takes the fast path: exact.
    let mut marked = gone;
    marked["missing"] = json!(true);
    let (doc, _) = document(vec![marked], None);
    assert_eq!(render(&mut r, &doc, &res, 0.5, &request()).unwrap(), p);
}

#[test]
fn effect_enables_are_animatable() {
    let _gpu = gpu();
    let mut blur = fx("f1", "blur", json!({ "amount": ch(json!(20)) }));
    blur["on"] = json!({ "v": true, "kf": [
        { "t": 0, "v": true, "i": "k0", "eo": [0.333, 0], "ei": [0.667, 1], "hold": true },
        { "t": 1, "v": false, "i": "k1", "eo": [0.333, 0], "ei": [0.667, 1], "hold": true }
    ], "expr": null });
    let (doc, res) = document(vec![blur], None);
    let (plain, _) = document(vec![], None);
    let mut r = builtins();
    let on = render(&mut r, &doc, &res, 0.4, &request()).unwrap();
    let off = render(&mut r, &doc, &res, 1.5, &request()).unwrap();
    assert!(max_diff(&on, &render(&mut r, &plain, &res, 0.4, &request()).unwrap()) > 0.05);
    assert_eq!(
        off,
        render(&mut r, &plain, &res, 1.5, &request()).unwrap(),
        "disabled: the fast path"
    );
}

#[test]
fn definition_updates_hot_reload_and_removal() {
    let _gpu = gpu();
    let (doc, res) = document(
        vec![fx("f1", "gain", json!({ "gain": ch(json!(4)) }))],
        None,
    );
    let (plain, _) = document(vec![], None);
    let mut r = Renderer::headless().unwrap();
    let mut defs = builtin_definitions().definitions;
    defs.push(raw_effect("gain", GAIN, gain_params(), false));
    assert!(
        r.apply_definitions(&update(1, true, defs, vec![]))
            .unwrap()
            .is_empty()
    );
    let base = render(&mut r, &plain, &res, 0.5, &request()).unwrap();
    let four = render(&mut r, &doc, &res, 0.5, &request()).unwrap();
    // Values above 1.0 pass through (no clamp).
    assert!(four.iter().any(|v| *v > 1.5), "E values above 1 are kept");
    // Hot reload: the same id with other GLSL.
    let half = GAIN.replace("* u_gain", "* u_gain * 0.125");
    assert!(
        r.apply_definitions(&update(
            2,
            false,
            vec![raw_effect("gain", &half, gain_params(), false)],
            vec![]
        ))
        .unwrap()
        .is_empty()
    );
    let reloaded = render(&mut r, &doc, &res, 0.5, &request()).unwrap();
    assert!(max_diff(&reloaded, &four) > 0.5);
    assert!(reloaded.iter().all(|v| *v <= 1.0 + 1e-3));
    // clamp: true limits the output to 0..1.
    assert!(
        r.apply_definitions(&update(
            3,
            false,
            vec![raw_effect("gain", GAIN, gain_params(), true)],
            vec![]
        ))
        .unwrap()
        .is_empty()
    );
    let clamped = render(&mut r, &doc, &res, 0.5, &request()).unwrap();
    assert!(clamped.iter().all(|v| *v <= 1.0), "clamp: true");
    // Removal: the effect is missing and skipped, its data kept.
    let removal = DefinitionRemoval {
        id: "gain".into(),
        kind: DefinitionKind::Effect,
    };
    r.apply_definitions(&update(4, false, vec![], vec![removal]))
        .unwrap();
    assert!(r.effects().is_missing(DefinitionKind::Effect, "gain"));
    let removed = render(&mut r, &doc, &res, 0.5, &request()).unwrap();
    assert!(max_diff(&removed, &base) <= 1.0 / 1024.0);
    // A gap is refused; a full update at a later generation is accepted.
    assert!(matches!(
        r.apply_definitions(&update(6, false, vec![], vec![])),
        Err(UpdateError::Gap { .. })
    ));
    r.apply_definitions(&update(9, true, builtin_definitions().definitions, vec![]))
        .unwrap();
    assert!(
        r.effects().is_missing(DefinitionKind::Effect, "gain"),
        "omitted from a full update"
    );
    assert_eq!(r.definitions().generation, Seq::new(9).unwrap());
}

#[test]
fn forks_register_under_built_in_ids() {
    let _gpu = gpu();
    let mut set = builtin_definitions();
    let blur = set
        .definitions
        .iter_mut()
        .find(|d| d.id.as_str() == "blur")
        .unwrap();
    // A fork's own GLSL under the built-in id: here, an invert.
    let source = blur.glsl.as_ref().unwrap().source.replace(
        "float r = u_p0;",
        "o = vec4(texture(u_tex, v_st).a - texture(u_tex, v_st).rgb, texture(u_tex, v_st).a); return; float r = u_p0;",
    );
    blur.glsl.as_mut().unwrap().source = source;
    blur.origin = pm_protocol::definitions::DefinitionOrigin::Fork;
    let (doc, res) = document(
        vec![fx("f1", "blur", json!({ "amount": ch(json!(20)) }))],
        None,
    );
    let mut builtin = builtins();
    let mut forked = Renderer::headless().unwrap();
    assert!(forked.set_definitions(&set).unwrap().is_empty());
    let a = render(&mut builtin, &doc, &res, 0.5, &request()).unwrap();
    let b = render(&mut forked, &doc, &res, 0.5, &request()).unwrap();
    assert!(max_diff(&a, &b) > 0.2, "the fork's definition is used");
}

#[test]
fn refused_programs_are_skipped_and_engine_only_failures_routed() {
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    let broken = raw_effect(
        "broken",
        &GAIN.replace("* u_gain", "* nope"),
        gain_params(),
        false,
    );
    let sampler3d = raw_effect(
        "volume",
        "#version 300 es\nprecision highp float;\nprecision highp sampler3D;\nin vec2 v_st;\nuniform sampler3D u_vol;\nout vec4 o;\nvoid main(){ o = texture(u_vol, vec3(v_st, 0.5)); }\n",
        json!([]),
        false,
    );
    let mut defs = builtin_definitions().definitions;
    defs.extend([broken, sampler3d]);
    let failures = r.apply_definitions(&update(1, true, defs, vec![])).unwrap();
    let code = |id: &str| {
        failures
            .iter()
            .find(|e| matches!(&e.scope, ErrorScope::Definition { definition } if definition.as_str() == id))
            .map(|e| e.code)
    };
    assert_eq!(code("broken"), Some(ErrorCode::ShaderCompile));
    assert_eq!(code("volume"), Some(ErrorCode::NotHeadlessRenderable));
    let detail = failures
        .iter()
        .find(|e| e.code == ErrorCode::ShaderCompile)
        .unwrap();
    assert!(
        format!("{:?}", detail.detail).contains("line: 7"),
        "diagnostics in the author's lines: {detail:?}"
    );
    // Refused (WebGL refuses it too): skipped, as today.
    let (doc, res) = document(vec![fx("f1", "broken", json!({}))], None);
    let (plain, _) = document(vec![], None);
    let skipped = render(&mut r, &doc, &res, 0.5, &request()).unwrap();
    assert!(
        max_diff(
            &skipped,
            &render(&mut r, &plain, &res, 0.5, &request()).unwrap()
        ) <= 1.0 / 1024.0
    );
    // Engine-only failure: routed to the fallback.
    let (doc, res) = document(vec![fx("f1", "volume", json!({}))], None);
    match render(&mut r, &doc, &res, 0.5, &request()) {
        Err(RenderError::Unsupported(u)) => assert!(
            u.iter()
                .any(|u| u.feature == Feature::ShaderFallback("volume".into())),
            "{u:?}"
        ),
        other => panic!("expected routing, got {:?}", other.map(|_| ())),
    }
    // A viewer keeps the layer, with the same pixels as the plain chain,
    // and reports the omitted effect through its ordinary failure notices.
    r.set_isolate_layers(true);
    let frame = r
        .render(&doc, "Ptest-root", ticks(0.5), &request(), &res)
        .unwrap();
    let pixels = r.read_rgba(&frame).unwrap();
    assert!(
        frame.failures.iter().any(|f| f.layer == "img"
            && f.message.contains("volume")
            && f.message.contains("bypassed")),
        "{:?}",
        frame.failures
    );
    let expected = render(&mut r, &plain, &res, 0.5, &request()).unwrap();
    assert!(
        max_diff(&pixels, &expected) <= 1.0 / 1024.0,
        "a refused effect must not erase its image"
    );
    r.set_isolate_layers(false);
    assert!(
        unsupported_with_definitions(&doc, "Ptest-root", None, r.effects())
            .iter()
            .any(|u| u.feature == Feature::ShaderFallback("volume".into()))
    );
}

fn crossfade(dur: f64) -> Value {
    json!({ "type": "crossfade", "dur": dur, "p": {} })
}

#[test]
fn transitions_are_drawn_routed_or_skipped_by_definition() {
    let _gpu = gpu();
    let (doc, res) = document(vec![], Some(crossfade(1.0)));
    let mut r = builtins();
    let mid = render(&mut r, &doc, &res, 0.25, &request()).unwrap();
    // A transition the document marks missing: the layer composites as is.
    let (marked, _) = document(
        vec![],
        Some(json!({ "type": "crossfade", "dur": 1.0, "p": {}, "missing": true })),
    );
    let end_state = render(&mut r, &marked, &res, 0.25, &request()).unwrap();
    let at = |v: &[f32], x: usize, y: usize| v[(y * W as usize + x) * 4 + 1];
    assert!(
        (at(&mid, 250, 45) - at(&end_state, 250, 45)).abs() > 0.05,
        "mid-transition differs from the end state"
    );
    // Unknown transition types are routed, missing ones composite without it.
    let (unknown, _) = document(
        vec![],
        Some(json!({ "type": "com.example.spin", "dur": 1.0, "p": {} })),
    );
    match render(&mut r, &unknown, &res, 0.25, &request()) {
        Err(RenderError::Unsupported(u)) => assert!(
            u.iter()
                .any(|u| u.feature == Feature::Definition("transition com.example.spin".into())),
            "{u:?}"
        ),
        other => panic!("expected routing, got {:?}", other.map(|_| ())),
    }
    let mut set = builtin_definitions();
    set.missing.push("com.example.spin".into());
    r.set_definitions(&set).unwrap();
    assert_eq!(
        render(&mut r, &unknown, &res, 0.25, &request()).unwrap(),
        end_state,
        "a missing transition is no transition"
    );
}

#[test]
fn crossfade_is_the_mix_of_before_and_after() {
    let _gpu = gpu();
    // A full-frame layer over a solid background: before = background,
    // with the layer = the layer, crossfade = mix at the progress.
    let dur = 2.0;
    let mut top = layer(
        "top",
        "solid",
        json!({ "color": "#FF0000", "w": W, "h": H }),
        0.0,
        0.0,
        dur,
    );
    top["transitionIn"] = crossfade(1.0);
    let doc = load(&project(W, H, dur, solid_fill("#0000FF"), vec![top]));
    let res = MapResolver::default();
    let mut r = builtins();
    for t in [0.0, 0.3, 0.7, 1.0] {
        let v = render(&mut r, &doc, &res, t, &request()).unwrap();
        let (red, blue) = (v[0], v[2]);
        assert!(
            (red - t as f32).abs() < 2e-3 && (blue - (1.0 - t as f32)).abs() < 2e-3,
            "t {t}: {red} {blue}"
        );
    }
}

#[test]
fn regions_equal_the_full_frame() {
    let _gpu = gpu();
    let mut r = builtins();
    let blur = fx("f1", "blur", json!({ "amount": ch(json!(9)) }));
    let custom_safe = (document(vec![blur], None), "viewport-safe blur");
    let pixelless = (
        document(
            vec![fx("f1", "invert", json!({ "amount": ch(json!(100)) }))],
            Some(crossfade(1.0)),
        ),
        "with a transition",
    );
    for ((doc, res), what) in [custom_safe, pixelless] {
        let full = render(&mut r, &doc, &res, 0.5, &request()).unwrap();
        for rect in [
            PixelRect {
                x: 101,
                y: 37,
                width: 150,
                height: 100,
            },
            PixelRect {
                x: 0,
                y: 0,
                width: 40,
                height: 30,
            },
        ] {
            let mut req = request();
            req.region = Some(rect);
            let part = render(&mut r, &doc, &res, 0.5, &req).unwrap();
            let mut worst = 0.0f32;
            for y in 0..rect.height as usize {
                for x in 0..rect.width as usize {
                    for c in 0..4 {
                        let a = part[(y * rect.width as usize + x) * 4 + c];
                        let b = full
                            [((y + rect.y as usize) * W as usize + x + rect.x as usize) * 4 + c];
                        worst = worst.max((a - b).abs());
                    }
                }
            }
            // Full-level rendering is exact; a padded crop is within 1 CV.
            assert!(worst <= 1.0 / 255.0, "{what} {rect:?}: {worst}");
        }
    }
}

/// A contributed glow in the shape of a multi-pass bloom: a 2×2
/// area-filtered half-resolution tile packed into the input texture, a wide
/// Gaussian blur read bilinearly from the tile, and a bilinear upsample.
/// It declares its spread (`viewportPadding: ["radius"]`), which does not
/// count the half-resolution taps' footprint, and its tiles follow the
/// input texture, so a band's crop moves its downsample grid.
const HALO: &str = "#version 300 es
precision highp float;
in vec2 v_st;
uniform sampler2D u_tex;
uniform sampler2D u_orig;
uniform vec2 u_texel;
uniform int u_pass;
uniform float u_radius;
out vec4 o;
void main() {
  vec2 halfTexel = 0.5 * u_texel;
  bool tile = v_st.x < 0.5 && v_st.y < 0.5;
  vec2 logicalUV = v_st / 0.5;
  if (u_pass == 0) {
    vec4 sum = vec4(0.0);
    if (tile) {
      for (int dy = 0; dy < 2; dy++) {
        for (int dx = 0; dx < 2; dx++) {
          sum += texture(u_orig, logicalUV + (vec2(float(dx), float(dy)) - 0.5) * u_texel) * 0.25;
        }
      }
    }
    o = sum;
  } else if (u_pass < 3) {
    vec4 sum = vec4(0.0);
    float weights = 0.0;
    if (tile) {
      vec2 axis = u_pass == 1 ? vec2(1.0, 0.0) : vec2(0.0, 1.0);
      for (int i = -16; i <= 16; i++) {
        float t = float(i) / 16.0;
        float w = exp(-4.5 * t * t);
        vec2 uv = logicalUV + axis * (u_radius * t) * u_texel;
        if (all(greaterThanEqual(uv, vec2(0.0))) && all(lessThanEqual(uv, vec2(1.0)))) {
          sum += texture(u_tex, clamp(uv * 0.5, halfTexel, vec2(0.5) - halfTexel)) * w;
        }
        weights += w;
      }
    }
    o = sum / max(weights, 0.00001);
  } else {
    vec4 bloom = texture(u_tex, clamp(v_st * 0.5, halfTexel, vec2(0.5) - halfTexel));
    o = texture(u_orig, v_st) + bloom * 2.0;
  }
}
";

/// Renders the halo whole and in export bands and rectangles, with its
/// declared spread `reach` beyond its kernel radius 40, and returns each
/// rectangle with its count of values whose bits differ from the whole
/// frame's, and their largest difference.
fn halo_bands(reach: f64) -> Vec<(Precision, PixelRect, usize, f32)> {
    let mut halo = raw_effect(
        "acme.halo",
        HALO,
        json!([
            { "type": "number", "key": "radius", "default": 40, "min": 0, "max": 500, "step": null },
            { "type": "number", "key": "reach", "default": 40, "min": 0, "max": 500, "step": null }
        ]),
        false,
    );
    halo.flags.passes = 4;
    halo.flags.keep_orig = true;
    halo.flags.viewport_safe = true;
    halo.flags.viewport_padding = vec!["reach".into()];
    let mut set = builtin_definitions();
    set.definitions.push(halo);
    let mut r = Renderer::headless().unwrap();
    assert!(r.set_definitions(&set).unwrap().is_empty());
    let (doc, res) = document(
        vec![fx(
            "f1",
            "acme.halo",
            json!({ "radius": ch(json!(40)), "reach": ch(json!(reach)) }),
        )],
        None,
    );
    bands_against_whole(&mut r, &doc, &res, (W, H), false)
}

/// Each export-like band and rectangle of the frame against the whole
/// frame, at half and full float precision: the rectangle, its count of
/// values whose bits differ, and their largest difference in CV.
fn bands_against_whole(
    r: &mut Renderer,
    doc: &pm_model::Document,
    res: &MapResolver,
    (w, h): (u32, u32),
    export: bool,
) -> Vec<(Precision, PixelRect, usize, f32)> {
    bands_against_whole_at(r, doc, res, (w, h), 1.0, export)
}

/// [`bands_against_whole`] of a `w` × `h` composition rendered at
/// `render_scale` output pixels a composition pixel.
fn bands_against_whole_at(
    r: &mut Renderer,
    doc: &pm_model::Document,
    res: &MapResolver,
    composition: (u32, u32),
    render_scale: f64,
    export: bool,
) -> Vec<(Precision, PixelRect, usize, f32)> {
    let mut out = Vec::new();
    for precision in [Precision::Float16, Precision::Float32] {
        let mut whole_req = RenderRequest::at_scale(
            Size::new(composition.0, composition.1),
            render_scale,
            precision,
        );
        whole_req.export = export;
        let (w, h) = (whole_req.output_size.width, whole_req.output_size.height);
        let px = |v: u32| ((f64::from(v) * f64::from(w) / f64::from(W)).round() as u32).max(1);
        let whole = render(r, doc, res, 0.5, &whole_req).unwrap();
        // Export-like bands (an export's 16-row units at full size), and
        // rectangles away from the frame's edges.
        let band = if w > W { 16 } else { 23 };
        let mut rects: Vec<PixelRect> = (0..h)
            .step_by(band as usize)
            .map(|y| PixelRect {
                x: 0,
                y,
                width: w,
                height: band.min(h - y),
            })
            .collect();
        rects.extend(
            [(101, 37, 150, 100), (60, 70, 41, 33)].map(|(x, y, rw, rh)| PixelRect {
                x: px(x),
                y: px(y),
                width: px(rw),
                height: px(rh),
            }),
        );
        for rect in rects {
            let mut req = whole_req.clone();
            req.region = Some(rect);
            let part = render(r, doc, res, 0.5, &req).unwrap();
            let mut differing = 0usize;
            let mut worst = 0.0f32;
            for y in 0..rect.height as usize {
                for x in 0..rect.width as usize {
                    for c in 0..4 {
                        let a = part[(y * rect.width as usize + x) * 4 + c];
                        let b = whole
                            [((y + rect.y as usize) * w as usize + x + rect.x as usize) * 4 + c];
                        if a.to_bits() != b.to_bits() {
                            differing += 1;
                            worst = worst.max((a - b).abs());
                        }
                    }
                }
            }
            out.push((precision, rect, differing, worst * 255.0));
        }
    }
    out
}

fn assert_bit_equal(results: Vec<(Precision, PixelRect, usize, f32)>) {
    let bad: Vec<_> = results.into_iter().filter(|r| r.2 > 0).collect();
    assert!(bad.is_empty(), "values differ (count, max CV): {bad:?}");
}

/// A band's padded crop lies on the frame's grid: the halo's
/// half-resolution tiles follow its input texture, so a crop starting on
/// an odd row or column moves the downsample grid, and an odd-sized crop
/// ending on a frame edge clamps its tiles half a texel off the whole
/// frame's. The declared spread here is generous, so only the grid counts.
#[test]
fn a_band_of_a_reduced_resolution_effect_keeps_the_frames_grid() {
    let _gpu = gpu();
    assert_bit_equal(halo_bands(60.0));
}

/// A dither hashed from the pixel coordinate and a ramp over the content
/// coordinate, as the built-in Gradient draws them.
const HASHED: &str = "#version 300 es
precision highp float;
in vec2 v_uv;
in vec2 v_st;
in vec2 v_px;
uniform sampler2D u_tex;
out vec4 o;
float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
void main() {
  vec4 c = texture(u_tex, v_st);
  float t = pow(clamp(dot(v_uv - 0.5, vec2(0.8, -0.6)) + 0.5, 0.0, 1.0), 1.7);
  o = vec4(vec3(t, hash(v_px), 1.0 - t) * c.a, c.a);
}
";

/// A final frame's bands read the pixel and content coordinates the
/// whole frame reads: a hash of an interpolated coordinate turns the
/// rounding of a differently sized draw into different pixels.
#[test]
fn final_bands_read_the_coordinates_of_the_whole_frame() {
    let _gpu = gpu();
    let mut hashed = raw_effect("acme.hashed", HASHED, json!([]), false);
    hashed.flags.viewport_safe = true;
    let mut set = builtin_definitions();
    set.definitions.push(hashed);
    let mut r = Renderer::headless().unwrap();
    assert!(r.set_definitions(&set).unwrap().is_empty());
    // A 1920 × 1080 composition, where interpolation rounding shows: a
    // wide shape in a group at a fractional offset, its gradient dithered.
    let (w, h) = (W * 6, H * 6);
    let mut group = layer("g", "group", json!({}), -910.0, -490.1958700034494, 2.0);
    group["p"]["position.x"] = ch(json!(-910.0));
    group["p"]["position.y"] = ch(json!(-490.1958700034494));
    let mut plate = layer(
        "plate",
        "shape",
        json!({ "shape": "rect", "w": 8000, "h": 1081.5, "radius": 0, "stroke": 0, "color": "#FFFFFF",
                "fill": { "type": "solid", "angle": 0, "stops": [{ "id": "s", "color": "#FFFFFF", "position": 0 }] } }),
        960.0 + 910.0,
        540.75 + 490.1958700034494,
        2.0,
    );
    plate["group"] = json!("g");
    plate["fx"] = json!([
        fx("f1", "acme.hashed", json!({})),
        fx(
            "f2",
            "gradient",
            json!({
                "amount": ch(json!(100)), "angle": ch(json!(90)), "dither": ch(json!(4)), "midpoint": ch(json!(75)),
                "spread": ch(json!(100)), "startColor": ch(json!("#0A0907")), "endColor": ch(json!("#6C381F"))
            })
        )
    ]);
    let doc = load(&project(
        w,
        h,
        2.0,
        solid_fill("#000000"),
        vec![plate, group],
    ));
    assert_bit_equal(bands_against_whole(
        &mut r,
        &doc,
        &MapResolver::default(),
        (w, h),
        true,
    ));
}

/// A contributed program whose arithmetic follows its input texture: a
/// ripple over the input's normalized row, as packed tiles and
/// `u_texel`-scaled taps round on a band-sized input.
const BAND_RELATIVE: &str = "#version 300 es
precision highp float;
in vec2 v_st;
uniform sampler2D u_tex;
uniform vec2 u_texel;
out vec4 o;
void main() {
  vec4 c = texture(u_tex, v_st);
  o = vec4(c.rgb * (0.75 + 0.25 * sin(v_st.y / u_texel.y * 0.7)), c.a);
}
";

/// A final frame's bands equal the whole frame whatever an effect
/// computes from its input's size: in exports and the render cache it
/// draws over the whole frame (a viewer crop keeps its declared spread).
#[test]
fn a_final_band_draws_an_effect_over_the_whole_frame() {
    let _gpu = gpu();
    let mut def = raw_effect("acme.band-relative", BAND_RELATIVE, json!([]), false);
    def.flags.viewport_safe = true;
    let mut set = builtin_definitions();
    set.definitions.push(def);
    let mut r = Renderer::headless().unwrap();
    assert!(r.set_definitions(&set).unwrap().is_empty());
    let (doc, res) = document(vec![fx("f1", "acme.band-relative", json!({}))], None);
    assert_bit_equal(bands_against_whole(&mut r, &doc, &res, (W, H), true));
}

/// Every built-in effect's bands of a final frame equal the whole frame.
#[test]
fn final_bands_of_every_built_in_effect_equal_the_whole_frame() {
    let _gpu = gpu();
    let mut r = builtins();
    let ids: Vec<String> = builtin_definitions()
        .definitions
        .iter()
        .filter(|d| d.kind == DefinitionKind::Effect)
        .map(|d| d.id.as_str().to_owned())
        .collect();
    assert!(ids.len() > 10, "{ids:?}");
    let mut bad = Vec::new();
    for id in &ids {
        let (doc, res) = document(vec![fx("f1", id, json!({}))], None);
        let differing: Vec<_> = bands_against_whole(&mut r, &doc, &res, (W, H), true)
            .into_iter()
            .filter(|b| b.2 > 0)
            .collect();
        if !differing.is_empty() {
            bad.push((id.clone(), differing));
        }
    }
    assert!(bad.is_empty(), "{bad:?}");
}

/// A final frame rendered above or below its composition's size (a 4K
/// export of a 1080p composition, a half-size one) bands like any other:
/// effects that read the pixel coordinate (Grain's cells, Gradient's
/// dither) and ones that reach across rows give a band the whole frame's
/// bits (a viewer's 2× crop differs from its frame in the dither).
#[test]
fn bands_of_a_scaled_frame_equal_the_whole_frame() {
    let _gpu = gpu();
    let mut r = builtins();
    let (doc, res) = document(
        vec![
            fx("f1", "blur", json!({ "amount": ch(json!(7)) })),
            fx("f2", "grain", json!({ "amount": ch(json!(40)) })),
            fx(
                "f3",
                "gradient",
                json!({ "amount": ch(json!(60)), "dither": ch(json!(4)) }),
            ),
        ],
        None,
    );
    for scale in [2.0, 1.5, 0.5] {
        let bad: Vec<_> = bands_against_whole_at(&mut r, &doc, &res, (W, H), scale, true)
            .into_iter()
            .filter(|b| b.2 > 0)
            .collect();
        assert!(bad.is_empty(), "at {scale}×: {bad:?}");
    }
}

/// A band's padding covers each pass's sampling footprint beyond the
/// declared spread: the halo declares exactly its kernel radius, and its
/// half-resolution bilinear taps, downsample and upsample read a few
/// pixels further.
#[test]
fn a_band_covers_the_sampling_beyond_an_effects_declared_spread() {
    let _gpu = gpu();
    assert_bit_equal(halo_bands(40.0));
}

#[test]
fn effects_on_groups_and_inside_precomps() {
    let _gpu = gpu();
    let dur = 2.0;
    let mut group = layer("g", "group", json!({}), 0.0, 0.0, dur);
    set(&mut group, "opacity", json!(80));
    group["fx"] = json!([fx(
        "f1",
        "glow",
        json!({ "radius": ch(json!(30)), "threshold": ch(json!(20)), "intensity": ch(json!(150)) })
    )]);
    let mut a = layer(
        "a",
        "solid",
        json!({ "color": "#FF6B1A", "w": 80, "h": 80 }),
        20.0,
        20.0,
        dur,
    );
    a["group"] = json!("g");
    let mut inner_layer = layer(
        "i",
        "solid",
        json!({ "color": "#4C8DFF", "w": 100, "h": 60 }),
        10.0,
        10.0,
        dur,
    );
    inner_layer["fx"] = json!([fx(
        "f2",
        "vignette",
        json!({ "amount": ch(json!(90)), "feather": ch(json!(50)) })
    )]);
    let mut pre = layer(
        "pre",
        "precomp",
        json!({ "comp": "inner", "w": 120, "h": 80 }),
        160.0,
        60.0,
        dur,
    );
    pre["fx"] = json!([fx("f3", "pixelate", json!({ "size": ch(json!(6)) }))]);
    let mut doc = project(W, H, dur, solid_fill("#14161C"), vec![pre, a, group]);
    composition(&mut doc, "inner", 120, 80, dur, vec![inner_layer]);
    let doc = load(&doc);
    let mut r = builtins();
    let v = render(&mut r, &doc, &MapResolver::default(), 0.5, &request()).unwrap();
    assert!(v.iter().all(|x| x.is_finite()));
    let plan = r
        .plan(
            &doc,
            "Ptest-root",
            ticks(0.5),
            &request(),
            &MapResolver::default(),
        )
        .unwrap()
        .0;
    let staged = plan
        .draws
        .iter()
        .filter(|d| match &d.content {
            pm_render::plan::Content::Composite(n) => matches!(
                n.as_ref(),
                pm_render::composite::Node::Layer(l) if l.effects.is_some()
            ),
            _ => false,
        })
        .count();
    assert_eq!(
        staged, 2,
        "the group and the precomp layer go through their own targets with effects"
    );
}

/// Adjustment layers with the built-in definitions (`compositeAdjustment`):
/// the stack beneath through the real GLSL chain (`runEffects(acc, acc)`),
/// masked, composited over the stack with the layer's opacity and blend;
/// layers above untouched; routed without definitions.
#[test]
fn adjustment_layers_run_real_definitions_on_the_stack_beneath() {
    let _gpu = gpu();
    let mut r = builtins();
    let dur = 2.0;
    let back = layer(
        "back",
        "solid",
        json!({ "color": "#336699", "w": W, "h": H }),
        0.0,
        0.0,
        dur,
    );
    let chip = layer(
        "chip",
        "solid",
        json!({ "color": "#F0C020", "w": 60, "h": 40 }),
        40.0,
        30.0,
        dur,
    );
    let above = layer(
        "above",
        "solid",
        json!({ "color": "#20E080", "w": 30, "h": 30 }),
        250.0,
        120.0,
        dur,
    );
    let doc = |adj: Option<Value>| {
        let mut layers = vec![above.clone()];
        layers.extend(adj);
        layers.extend([chip.clone(), back.clone()]);
        load(&project(W, H, dur, solid_fill("#000000"), layers))
    };
    let res = MapResolver::default();
    let px = |v: &[f32], x: usize, y: usize| -> [f32; 4] {
        let i = (y * W as usize + x) * 4;
        [v[i], v[i + 1], v[i + 2], v[i + 3]]
    };
    let close =
        |a: [f32; 4], b: [f32; 4]| a.iter().zip(&b).all(|(x, y)| (x - y).abs() <= 1.5 / 255.0);
    let stack = render(&mut r, &doc(None), &res, 0.5, &request()).unwrap();
    // Invert at 100%, opacity 100%: every pixel beneath is inverted, the
    // layer above is not.
    let mut adj = layer("adj", "adjustment", json!({}), 0.0, 0.0, dur);
    adj["fx"] = json!([fx("f1", "invert", json!({ "amount": ch(json!(100)) }))]);
    let inverted = render(&mut r, &doc(Some(adj.clone())), &res, 0.5, &request()).unwrap();
    for (x, y) in [(10, 10), (60, 50), (200, 100), (300, 170)] {
        let s = px(&stack, x, y);
        assert!(
            close(
                px(&inverted, x, y),
                [1.0 - s[0], 1.0 - s[1], 1.0 - s[2], 1.0]
            ),
            "({x}, {y}): {:?} from {s:?}",
            px(&inverted, x, y)
        );
    }
    assert!(
        close(px(&inverted, 260, 130), px(&stack, 260, 130)),
        "above"
    );
    // At 40% opacity, with a rect mask over the left half (a parametric
    // mask is placed by its center) and a screen blend: inside,
    // screen(inverted, stack) mixed at 40%; outside, the stack.
    adj["blend"] = json!("screen");
    set(&mut adj, "opacity", json!(40));
    adj["masks"] = json!([{ "id": "k", "shape": "rect", "mode": "add", "on": true,
        "p": { "x": ch(json!(W / 4)), "y": ch(json!(H / 2)), "w": ch(json!(W / 2)), "h": ch(json!(H)), "rotation": ch(json!(0)), "feather": ch(json!(0)) } }]);
    let masked = render(&mut r, &doc(Some(adj.clone())), &res, 0.5, &request()).unwrap();
    for (x, y) in [(10, 10), (60, 50), (100, 150)] {
        let s = px(&stack, x, y);
        let want: Vec<f32> = (0..3)
            .map(|c| {
                let (a, b) = (1.0 - s[c], s[c]);
                let screen = a + b - a * b;
                b + (screen - b) * 0.4
            })
            .collect();
        assert!(
            close(px(&masked, x, y), [want[0], want[1], want[2], 1.0]),
            "({x}, {y}): {:?}, want {want:?}",
            px(&masked, x, y)
        );
    }
    for (x, y) in [(200, 100), (300, 20)] {
        assert!(
            close(px(&masked, x, y), px(&stack, x, y)),
            "outside the mask ({x}, {y})"
        );
    }
    // A blur on an adjustment reads the stack beneath: the chip's edge is
    // soft, far from it the stack is unchanged.
    let mut soft = layer("adj", "adjustment", json!({}), 0.0, 0.0, dur);
    soft["fx"] = json!([fx("f1", "blur", json!({ "amount": ch(json!(10)) }))]);
    let blurred = render(&mut r, &doc(Some(soft)), &res, 0.5, &request()).unwrap();
    assert!(max_diff(&blurred, &stack) > 0.1);
    assert!(close(px(&blurred, 200, 100), px(&stack, 200, 100)));
    let edge = px(&blurred, 40, 50);
    let (inside, outside) = (px(&stack, 45, 50), px(&stack, 35, 50));
    assert!(
        (0..3).all(|c| edge[c] > inside[c].min(outside[c]) + 0.02
            && edge[c] < inside[c].max(outside[c]) - 0.02),
        "{edge:?} between {inside:?} and {outside:?}"
    );
    // An adjustment whose effect is missing is a no-op, its data kept; one
    // with no definitions at all is routed.
    let mut missing = layer("adj", "adjustment", json!({}), 0.0, 0.0, dur);
    missing["fx"] = json!([fx("f1", "com.example.gone", json!({}))]);
    let mut set_missing = builtin_definitions();
    set_missing.missing.push("com.example.gone".into());
    r.set_definitions(&set_missing).unwrap();
    let unchanged = render(&mut r, &doc(Some(missing)), &res, 0.5, &request()).unwrap();
    assert!(max_diff(&unchanged, &stack) <= 1.0 / 1024.0);
    let mut bare = Renderer::headless().unwrap();
    let adjusted = doc(Some(adj));
    assert!(
        unsupported(&adjusted, "Ptest-root")
            .iter()
            .any(|u| u.feature == Feature::Adjustment),
        "routed without definitions"
    );
    assert!(
        unsupported_with_definitions(&adjusted, "Ptest-root", None, r.effects()).is_empty(),
        "drawn with them"
    );
    assert!(matches!(
        render(&mut bare, &adjusted, &res, 0.5, &request()),
        Err(RenderError::Unsupported(_))
    ));
}

/// `Renderer::set_program_cache_dir` keeps translated GLSL on disk
/// (`dir/glsl`): a second renderer (the next engine launch) loads the
/// built-ins from it instead of translating them again.
#[test]
fn translated_programs_are_kept_in_the_program_cache_folder() {
    let _gpu = gpu();
    let dir = std::env::temp_dir().join(format!("pm-render-program-cache-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let mut first = Renderer::headless().unwrap();
    first.set_program_cache_dir(Some(dir.clone()));
    let started = std::time::Instant::now();
    assert!(
        first
            .set_definitions(&builtin_definitions())
            .unwrap()
            .is_empty()
    );
    let cold = started.elapsed();
    let entries = std::fs::read_dir(dir.join("glsl")).unwrap().count();
    assert_eq!(entries, 20, "one entry per built-in program");
    let mut second = Renderer::headless().unwrap();
    second.set_program_cache_dir(Some(dir.clone()));
    let started = std::time::Instant::now();
    assert!(
        second
            .set_definitions(&builtin_definitions())
            .unwrap()
            .is_empty()
    );
    let warm = started.elapsed();
    eprintln!("20 built-ins: cold {cold:?}, from the folder {warm:?}");
    assert!(warm < cold, "loaded from disk: {warm:?} vs {cold:?}");
    // The cached programs draw what fresh ones draw.
    let (doc, res) = document(vec![fx("f1", "glow", json!({}))], None);
    let a = render(&mut first, &doc, &res, 0.5, &request()).unwrap();
    let b = render(&mut second, &doc, &res, 0.5, &request()).unwrap();
    assert_eq!(a, b);
    std::fs::remove_dir_all(&dir).ok();
}

/// The watchdog's test hook (issue A2): the blur takes `blur_ms` of GPU
/// time per frame, everything else 1 ms; frames take 16 ms of wall time.
/// Every frame is judged at its completion, on any device, under any load.
fn blur_takes(
    ms: std::sync::Arc<std::sync::atomic::AtomicU64>,
) -> std::sync::Arc<dyn TimingSource> {
    std::sync::Arc::new(FnTiming(
        move |span: &Span| {
            Some(match &span.subject {
                Subject::Definition { id, .. } if id.as_str() == "blur" => {
                    Duration::from_millis(ms.load(std::sync::atomic::Ordering::Relaxed))
                }
                _ => Duration::from_millis(1),
            })
        },
        |_: &[Span]| Duration::from_millis(16),
    ))
}

#[test]
fn an_enforced_watchdog_budget_disables_an_overrunning_user_effect_from_the_next_frame() {
    // The budget records only unless a renderer opts in (today's
    // compositor never turns a program off, amendment 22), and an enforced
    // budget never applies to built-ins: Blur here is the user's override
    // of `blur` by id.
    let _gpu = gpu();
    let mut r = builtins();
    let user_set = || {
        let mut set: DefinitionSetSnapshot = builtin_definitions();
        for d in set
            .definitions
            .iter_mut()
            .filter(|d| d.id.as_str() == "blur")
        {
            d.origin = pm_protocol::definitions::DefinitionOrigin::Local;
        }
        set
    };
    assert!(r.set_definitions(&user_set()).unwrap().is_empty());
    r.effects_mut()
        .watchdog
        .set_budget(Duration::from_millis(250));
    let (doc, res) = document(
        vec![fx("f1", "blur", json!({ "amount": ch(json!(40)) }))],
        None,
    );
    let (plain, _) = document(vec![], None);
    let blur_ms = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(300));
    r.effects_mut()
        .watchdog
        .set_timing_source(Some(blur_takes(blur_ms.clone())));
    // Judged at the first frame's completion: 300 ms against 250.
    let first = render(&mut r, &doc, &res, 0.5, &request()).unwrap();
    let reports = r.watchdog_reports(true);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].code, ErrorCode::WatchdogTimeout);
    assert!(
        matches!(&reports[0].scope, ErrorScope::Effect { definition, .. } if definition.as_str() == "blur")
    );
    assert_eq!(
        r.effects()
            .watchdog
            .measured_ms(DefinitionKind::Effect, "blur"),
        Some(300.0)
    );
    let next = render(&mut r, &doc, &res, 0.5, &request()).unwrap();
    let base = render(&mut r, &plain, &res, 0.5, &request()).unwrap();
    assert!(max_diff(&first, &base) > 0.05);
    assert!(
        max_diff(&next, &base) <= 1.0 / 1024.0,
        "disabled, its data kept"
    );
    // The same definitions pushed again (any unrelated registration, a
    // second view's set) keep it off: its program and record are kept.
    let mut set: DefinitionSetSnapshot = user_set();
    assert!(r.set_definitions(&set).unwrap().is_empty());
    blur_ms.store(5, std::sync::atomic::Ordering::Relaxed);
    let kept = render(&mut r, &doc, &res, 0.5, &request()).unwrap();
    assert!(max_diff(&kept, &base) <= 1.0 / 1024.0, "still disabled");
    assert!(r.watchdog_reports(true).is_empty(), "not reported again");
    // Registered again with the same source (a kernel delta: a register or
    // a hot reload of that id) re-enables it.
    let same = set
        .definitions
        .iter()
        .find(|d| d.id.as_str() == "blur")
        .unwrap()
        .clone();
    let generation = r.effects().generation().unwrap().get() + 1;
    assert!(
        r.apply_definitions(&update(generation, false, vec![same], vec![]))
            .unwrap()
            .is_empty()
    );
    let reloaded = render(&mut r, &doc, &res, 0.5, &request()).unwrap();
    assert_eq!(reloaded, first, "a delta re-registration re-enables it");
    assert!(r.watchdog_reports(true).is_empty(), "5 ms is within budget");
    // Over budget again: off again from the next frame.
    blur_ms.store(300, std::sync::atomic::Ordering::Relaxed);
    render(&mut r, &doc, &res, 0.5, &request()).unwrap();
    assert_eq!(r.watchdog_reports(true).len(), 1, "overran again");
    let off_again = render(&mut r, &doc, &res, 0.5, &request()).unwrap();
    assert!(max_diff(&off_again, &base) <= 1.0 / 1024.0);
    // A changed source in a full set (a hot reload, a new registration)
    // re-enables it too.
    blur_ms.store(5, std::sync::atomic::Ordering::Relaxed);
    let blur = set
        .definitions
        .iter_mut()
        .find(|d| d.id.as_str() == "blur")
        .unwrap();
    blur.glsl
        .as_mut()
        .unwrap()
        .source
        .push_str("\n// reloaded\n");
    assert!(r.set_definitions(&set).unwrap().is_empty());
    let again = render(&mut r, &doc, &res, 0.5, &request()).unwrap();
    assert_eq!(again, first);
    assert!(r.watchdog_reports(true).is_empty());
}

#[test]
fn the_watchdog_judges_every_timed_frame_not_only_the_first() {
    // Resolved in the frame's own command buffer, Metal returned the
    // previous frame's end sample: only a renderer's first timed frame read
    // as a span, and every later one ended before it began and went
    // unjudged. Each frame here is timed (its timestamps are read before
    // the next one) and must be judged.
    let _gpu = gpu();
    let mut r = builtins();
    if !r
        .device()
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY)
    {
        eprintln!("no TIMESTAMP_QUERY on this device; the watchdog times nothing");
        return;
    }
    let (doc, res) = document(
        vec![fx("f1", "blur", json!({ "amount": ch(json!(40)) }))],
        None,
    );
    let mut judged = 0;
    for frame in 0..8 {
        let before = r.effects().watchdog.measurements();
        render(
            &mut r,
            &doc,
            &res,
            0.5 + f64::from(frame) * 0.01,
            &request(),
        )
        .unwrap();
        // Waits for the frame to be judged; the budget records only, so
        // nothing is reported or turned off.
        assert!(r.watchdog_reports(true).is_empty());
        if r.effects().watchdog.measurements() > before {
            judged += 1;
        }
    }
    assert!(
        r.effects()
            .watchdog
            .measured_ms(DefinitionKind::Effect, "blur")
            .is_some_and(|ms| ms > 0.0)
    );
    // Metal can leave a sample empty while other devices are busy in this
    // process; nearly every frame must still be judged.
    assert!(judged >= 6, "{judged} of 8 frames judged");
}

#[test]
fn a_viewport_safe_chain_renders_a_padded_crop_of_the_region() {
    let _gpu = gpu();
    // Large enough that the padded crop (128 px of overscan plus the
    // effects' padding) is smaller than the frame.
    let (w, h) = (1280u32, 720u32);
    let dur = 2.0;
    let mut image = layer(
        "img",
        "image",
        json!({ "asset": "still", "fit": "stretch", "w": 900, "h": 500 }),
        190.0,
        110.0,
        dur,
    );
    set(&mut image, "rotation", json!(-4));
    image["fx"] = json!([
        fx("f1", "blur", json!({ "amount": ch(json!(9)) })),
        fx("f2", "chroma", json!({ "amount": ch(json!(6)) })),
        fx(
            "f3",
            "vignette",
            json!({ "amount": ch(json!(60)), "feather": ch(json!(50)) })
        ),
    ]);
    let mut doc = project(w, h, dur, solid_fill("#14161C"), vec![image]);
    asset(&mut doc, "still", "image", "png", 160, 90, 0.0);
    let doc = load(&doc);
    let mut res = MapResolver::default();
    res.paths.insert(
        "still".into(),
        repo_root().join("engine/fixtures/still-alpha.png"),
    );
    let mut r = builtins();
    let full_req = RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float16);
    let full = render(&mut r, &doc, &res, 0.5, &full_req).unwrap();
    let rect = PixelRect {
        x: 500,
        y: 260,
        width: 240,
        height: 160,
    };
    let mut req = full_req;
    req.region = Some(rect);
    let part = render(&mut r, &doc, &res, 0.5, &req).unwrap();
    let mut worst = 0.0f32;
    let mut over = 0usize;
    for y in 0..rect.height as usize {
        for x in 0..rect.width as usize {
            for c in 0..4 {
                let a = part[(y * rect.width as usize + x) * 4 + c];
                let b = full[((y + rect.y as usize) * w as usize + x + rect.x as usize) * 4 + c];
                let d = (a - b).abs();
                worst = worst.max(d);
                if d > 1.0 / 255.0 {
                    over += 1;
                }
            }
        }
    }
    eprintln!(
        "padded crop against the full frame: max {:.3} CV, {over} values over 1 CV",
        worst * 255.0
    );
    assert!(worst <= 1.0 / 255.0, "max {} CV", worst * 255.0);
}

/// Measurement (run with `--ignored --nocapture`, release): translation of
/// every built-in cold and from the cache, and each built-in's GPU time at
/// UHD through the watchdog's timestamps.
#[test]
#[ignore]
fn builtin_costs_at_uhd() {
    let _gpu = gpu();
    let set = builtin_definitions();
    let t = std::time::Instant::now();
    let mut r = Renderer::headless().unwrap();
    r.set_definitions(&set).unwrap();
    eprintln!(
        "translate 20 built-ins (cold process): {:.1} ms",
        t.elapsed().as_secs_f64() * 1000.0
    );
    let t = std::time::Instant::now();
    r.set_definitions(&set).unwrap();
    eprintln!(
        "again (translation cache): {:.2} ms",
        t.elapsed().as_secs_f64() * 1000.0
    );
    let (w, h) = (3840u32, 2160u32);
    for d in set
        .definitions
        .iter()
        .filter(|d| d.kind == DefinitionKind::Effect)
    {
        let id = d.id.as_str();
        let mut image = layer(
            "img",
            "image",
            json!({ "asset": "still", "fit": "stretch", "w": 3000, "h": 1700 }),
            400.0,
            200.0,
            2.0,
        );
        image["fx"] = json!([fx("f1", id, json!({}))]);
        let mut doc = project(w, h, 2.0, solid_fill("#14161C"), vec![image]);
        asset(&mut doc, "still", "image", "png", 160, 90, 0.0);
        let doc = load(&doc);
        let mut res = MapResolver::default();
        res.paths.insert(
            "still".into(),
            repo_root().join("engine/fixtures/still-alpha.png"),
        );
        let req = RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float16);
        let first = std::time::Instant::now();
        render(&mut r, &doc, &res, 0.5, &req).unwrap();
        let first = first.elapsed().as_secs_f64() * 1000.0;
        for _ in 0..5 {
            render(&mut r, &doc, &res, 0.5, &req).unwrap();
            r.watchdog_reports(true);
        }
        eprintln!(
            "{id:<14} first frame {first:>7.1} ms (pipeline build included); GPU {:.2} ms (longest of 5, defaults)",
            r.effects()
                .watchdog
                .measured_ms(DefinitionKind::Effect, id)
                .unwrap_or(f64::NAN)
        );
    }
}

/// An effect's pixel-sized values are composition pixels at any render
/// scale: `v_px` counts them and `u_texel` is one of them, so a frame drawn
/// at half scale (a playing viewer, a reduced quality) shows the effect at
/// the size the 1:1 export does. A full-frame solid paints red from
/// `v_px.x = 40` and green from `40 × u_texel.x` in its own UV.
#[test]
fn effect_pixels_are_composition_pixels_at_any_render_scale() {
    let _gpu = gpu();
    const EDGE: &str = "#version 300 es\nprecision highp float;\nin vec2 v_st;\nin vec2 v_px;\nuniform sampler2D u_tex;\nuniform vec2 u_texel;\nuniform float u_edge;\nout vec4 o;\nvoid main(){ o = vec4(step(u_edge, v_px.x), step(u_edge * u_texel.x, v_st.x), 0.0, 1.0); }\n";
    let mut set = builtin_definitions();
    set.definitions.push(raw_effect(
        "acme.edge",
        EDGE,
        json!([{ "type": "number", "key": "edge", "default": 40, "min": 0, "max": 1000, "step": null }]),
        false,
    ));
    let mut r = Renderer::headless().unwrap();
    assert!(r.set_definitions(&set).unwrap().is_empty());
    let (w, h) = (128u32, 64u32);
    let mut plate = layer(
        "plate",
        "solid",
        json!({ "color": "#000000", "w": w, "h": h }),
        64.0,
        32.0,
        1.0,
    );
    plate["fx"] = json!([fx("e", "acme.edge", json!({}))]);
    let doc = load(&project(w, h, 1.0, solid_fill("#000000"), vec![plate]));
    let res = MapResolver::default();
    // The first column at or past half intensity in channel `c` of the middle row.
    let edge = |px: &[f32], width: u32, c: usize| {
        let y = (px.len() / 4 / width as usize) / 2;
        (0..width as usize)
            .find(|x| px[(y * width as usize + x) * 4 + c] >= 0.5)
            .map(|x| x as f64)
    };
    for scale in [1.0, 0.5] {
        let size = Size::new((f64::from(w) * scale) as u32, (f64::from(h) * scale) as u32);
        let req = RenderRequest::at_scale(size, scale, Precision::Float16);
        let px = render(&mut r, &doc, &res, 0.0, &req).unwrap();
        for (c, what) in [(0, "v_px"), (1, "u_texel")] {
            let at = edge(&px, size.width, c).map(|x| x / scale);
            assert!(
                at.is_some_and(|x| (x - 40.0).abs() <= 1.0 / scale),
                "{what} at scale {scale}: edge at composition x {at:?}, not 40"
            );
        }
    }
}

/// The effect pixel contract in final output: an effect without an owner
/// binding is drawn on the composition's own pixel grid at any output size,
/// so a resized export (half or twice the composition) shows the 1:1
/// export's picture resampled, whatever idiom its shader uses: a radius in
/// `u_texel`, a physical neighbour step, a half-texel tile clamp, or
/// `texelFetch` from `gl_FragCoord`. A viewer frame does the same for a
/// shader that addresses physical texels.
#[test]
fn resized_frames_draw_effects_on_the_composition_grid() {
    let _gpu = gpu();
    let program = |body: &str| {
        format!(
            "#version 300 es\nprecision highp float;\nin vec2 v_st;\nin vec2 v_px;\nuniform sampler2D u_tex;\nuniform vec2 u_texel;\nuniform vec2 u_res;\nout vec4 o;\nvoid main(){{ {body} }}\n"
        )
    };
    let shaders = [
        (
            "acme.radius",
            program("o = texture(u_tex, v_st - vec2(8.0 * u_texel.x, 0.0));"),
        ),
        (
            "acme.neighbours",
            program(
                "o = 0.5 * (texture(u_tex, v_st - vec2(u_texel.x, 0.0)) + texture(u_tex, v_st + vec2(u_texel.x, 0.0)));",
            ),
        ),
        (
            "acme.tile",
            program(
                "o = texture(u_tex, vec2(clamp(v_st.x * 0.5, 0.5 * u_texel.x, 0.5 - 0.5 * u_texel.x), v_st.y));",
            ),
        ),
        (
            "acme.fetch",
            program(
                "ivec2 at = clamp(ivec2(gl_FragCoord.xy) - ivec2(4, 0), ivec2(0), textureSize(u_tex, 0) - 1); o = texelFetch(u_tex, at, 0);",
            ),
        ),
    ];
    let mut set = builtin_definitions();
    for (id, source) in &shaders {
        set.definitions
            .push(raw_effect(id, source, json!([]), false));
    }
    let mut r = Renderer::headless().unwrap();
    assert!(r.set_definitions(&set).unwrap().is_empty());
    let (w, h) = (64u32, 32u32);
    let frame = |px: &[f32], width: usize, x: usize, y: usize| px[(y * width + x) * 4];
    for (id, _) in &shaders {
        // A white solid from column 20 to 60 over every row (the anchor is its
        // top-left corner), the effect on it, on black.
        let mut plate = layer(
            "plate",
            "solid",
            json!({ "color": "#FFFFFF", "w": 40, "h": h }),
            20.0,
            0.0,
            1.0,
        );
        plate["fx"] = json!([fx("e", id, json!({}))]);
        let doc = load(&project(w, h, 1.0, solid_fill("#000000"), vec![plate]));
        let res = MapResolver::default();
        let at = |r: &mut Renderer, scale: f64, final_output: bool| {
            let req = RenderRequest {
                export: final_output,
                output_kind: if final_output {
                    pm_render::OutputKind::Final
                } else {
                    pm_render::OutputKind::Preview
                },
                ..RenderRequest::at_scale(Size::new(w, h), scale, Precision::Float16)
            };
            let width = req.output_size.width as usize;
            (render(r, &doc, &res, 0.0, &req).unwrap(), width)
        };
        let (one, _) = at(&mut r, 1.0, true);
        // Compare along the middle row, in composition pixels: each output
        // sample against the 1:1 frame at the same composition position
        // (linear between its pixels).
        let compare = |px: &[f32], width: usize, scale: f64, label: &str| {
            let y = (f64::from(h) * scale / 2.0) as usize;
            let mut worst = (0.0f32, 0usize);
            let mut sum = 0.0f32;
            for x in 0..width {
                let cx = ((x as f64 + 0.5) / scale - 0.5).clamp(0.0, f64::from(w) - 1.0);
                let (x0, t) = (cx.floor() as usize, (cx - cx.floor()) as f32);
                let x1 = (x0 + 1).min(w as usize - 1);
                let want = frame(&one, w as usize, x0, h as usize / 2) * (1.0 - t)
                    + frame(&one, w as usize, x1, h as usize / 2) * t;
                let d = (frame(px, width, x, y) - want).abs();
                sum += d;
                if d > worst.0 {
                    worst = (d, x);
                }
            }
            let mean = sum / width as f32;
            assert!(
                mean < 0.03 && worst.0 < 0.55,
                "{id} {label}: off the 1:1 frame resampled by mean {mean}, worst {worst:?}"
            );
        };
        for scale in [0.5, 2.0] {
            let (px, width) = at(&mut r, scale, true);
            compare(&px, width, scale, &format!("export at {scale}x"));
        }
        if *id == "acme.fetch" {
            let (px, width) = at(&mut r, 2.0, false);
            compare(&px, width, 2.0, "viewer at 2x");
        }
    }
}
