//! Legacy GLSL with linear blending on (plan section 11, "Color domain":
//! legacy GLSL receives E-encoded values independent of the display; section
//! 9's rule: E⁻¹ on input, linear blending, E on output).
//!
//! Today's compositor has no linear blending, so there is no WebGL capture to
//! compare with. What the rule fixes is testable instead: every effect chain,
//! its backdrop, and both transition inputs are encoded with E before the
//! legacy program and decoded after, so wherever no linear-light blend
//! touches the result (an opaque full-frame layer over an opaque stack, an
//! adjustment layer at 100%), a project with linear blending on renders what
//! the same project renders in Standard, which `effects_webgl.rs` and
//! `corpus.rs` hold to WebGL. A control frame (a 50% composite) proves the
//! renderer is blending in linear light at the same time.

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_protocol::definitions::DefinitionSnapshot;
use pm_render::{MapResolver, RenderRequest, Renderer, builtin_definitions};
use serde_json::{Value, json};
use support::doc::*;
use support::ticks;

const W: u32 = 160;
const H: u32 = 96;

/// The tests take turns on the GPU (as `effects.rs`).
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|e| e.into_inner())
}

/// An opaque `w` × `h` RGB PNG whose centered W × H window holds ramps,
/// hard edges, and saturated stripes (the rest repeats it).
fn picture(path: &std::path::Path, w: u32, h: u32) {
    let mut data = Vec::with_capacity((w * h * 3) as usize);
    for py in 0..h {
        for px in 0..w {
            let x = (px + W - (w - W) / 2 % W) % W;
            let y = (py + H - (h - H) / 2 % H) % H;
            let px = if x < W / 4 {
                // Black and white bars with hard edges.
                if (x / 6 + y / 12).is_multiple_of(2) {
                    [255, 255, 255]
                } else {
                    [0, 0, 0]
                }
            } else if y < H / 3 {
                [(x * 255 / (W - 1)) as u8, (y * 255 / (H / 3)) as u8, 40]
            } else if y < 2 * H / 3 {
                match (x / 10) % 4 {
                    0 => [255, 32, 0],
                    1 => [0, 200, 90],
                    2 => [40, 80, 255],
                    _ => [250, 230, 30],
                }
            } else {
                let v = ((x + y) * 255 / (W + H)) as u8;
                [v, 255 - v, (x * 7 % 256) as u8]
            };
            data.extend_from_slice(&px);
        }
    }
    let file = std::fs::File::create(path).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(&data).unwrap();
}

struct Fixture {
    dir: std::path::PathBuf,
    resolver: MapResolver,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("pm-render-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut resolver = MapResolver::default();
        let path = dir.join("picture.png");
        picture(&path, W, H);
        resolver.paths.insert("pic".into(), path);
        let path = dir.join("overscanned.png");
        picture(&path, W + 2 * OVERSCAN, H + 2 * OVERSCAN);
        resolver.paths.insert("big".into(), path);
        Fixture { dir, resolver }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

/// The full-frame picture as an image layer (an image layer's position is
/// its center).
fn picture_layer(id: &str) -> Value {
    layer(
        id,
        "image",
        json!({ "asset": "pic", "fit": "stretch", "w": W, "h": H }),
        f64::from(W) / 2.0,
        f64::from(H) / 2.0,
        2.0,
    )
}

/// How far an effect layer reaches past the frame: the built-ins' largest
/// reach at the parameters used (glow's radius, shadow's distance plus
/// softness) stays inside it, so no transparent edge is blurred into view
/// (a partly transparent pixel composites in linear light, which is the
/// point of linear blending, not of this test).
const OVERSCAN: u32 = 128;

/// The picture past the frame by [`OVERSCAN`] on every side, one texel per
/// pixel (resampling would blend in linear light).
fn overscanned(id: &str) -> Value {
    layer(
        id,
        "image",
        json!({ "asset": "big", "fit": "stretch", "w": W + 2 * OVERSCAN, "h": H + 2 * OVERSCAN }),
        f64::from(W) / 2.0,
        f64::from(H) / 2.0,
        2.0,
    )
}

fn document(layers: Vec<Value>, background: &str, linear: bool) -> pm_model::Document {
    let mut doc = project(W, H, 2.0, solid_fill(background), layers);
    asset(&mut doc, "pic", "image", "png", W, H, 0.0);
    asset(
        &mut doc,
        "big",
        "image",
        "png",
        W + 2 * OVERSCAN,
        H + 2 * OVERSCAN,
        0.0,
    );
    doc["color"] = json!({ "mode": "standard", "linearBlending": linear });
    load(&doc)
}

fn renderer() -> Renderer {
    let mut r = Renderer::headless().unwrap();
    let failures = r.set_definitions(&builtin_definitions()).unwrap();
    assert!(failures.is_empty(), "{failures:?}");
    r
}

fn render(r: &mut Renderer, doc: &pm_model::Document, res: &MapResolver, t: f64) -> Vec<f32> {
    let req = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32);
    let frame = r.render(doc, "Ptest-root", ticks(t), &req, res).unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    r.read_rgba(&frame).unwrap()
}

/// 8-bit code differences: (fraction of channel values within 1 CV, max CV).
fn diff(a: &[f32], b: &[f32]) -> (f64, u32) {
    let code = |v: f32| (f64::from(v.clamp(0.0, 1.0)) * 255.0).round() as i64;
    let mut within = 0usize;
    let mut max = 0u32;
    for (x, y) in a.iter().zip(b) {
        let d = (code(*x) - code(*y)).unsigned_abs() as u32;
        if d <= 1 {
            within += 1;
        }
        max = max.max(d);
    }
    (within as f64 / a.len() as f64, max)
}

/// The definition's defaults as effect parameters, with `overrides`.
fn params(definition: &DefinitionSnapshot, overrides: &Value) -> Value {
    let snapshot = serde_json::to_value(definition).unwrap();
    let mut p = serde_json::Map::new();
    for param in snapshot["params"].as_array().unwrap() {
        let key = param["key"].as_str().unwrap();
        let value = overrides
            .get(key)
            .cloned()
            .unwrap_or(param["default"].clone());
        p.insert(key.to_owned(), ch(value));
    }
    Value::Object(p)
}

/// Parameter sets that change the picture (several defaults are no-ops).
fn overrides(id: &str) -> Value {
    match id {
        "color" => {
            json!({ "exposure": 0.4, "contrast": 25, "saturation": -35, "hue": 30, "temperature": 20 })
        }
        "levels" => json!({ "inBlack": 12, "inWhite": 85, "gamma": 1.6 }),
        "gradient" => json!({ "amount": 60, "shade": true }),
        _ => json!({}),
    }
}

#[test]
fn every_builtin_effect_reads_e_with_linear_blending_on() {
    let _gpu = gpu();
    let fixture = Fixture::new("fx-linear");
    let mut r = renderer();
    let builtins = builtin_definitions();
    let effects: Vec<_> = builtins
        .definitions
        .iter()
        .filter(|d| d.kind == pm_protocol::definitions::DefinitionKind::Effect)
        .collect();
    assert_eq!(effects.len(), 16);
    for def in effects {
        let id = def.id.as_str();
        let effect =
            json!([{ "id": "f1", "type": id, "on": true, "p": params(def, &overrides(id)) }]);
        // The overscanned picture over black, and a chip over transparency
        // (what an effect spills past a layer: shadows, glow, blur edges;
        // transparent output is premultiplied E in either mode).
        let mut picture = overscanned("top");
        picture["fx"] = effect.clone();
        let mut chip = layer(
            "top",
            "solid",
            json!({ "color": "#FF6B1A", "w": 60, "h": 40 }),
            50.0,
            28.0,
            2.0,
        );
        chip["fx"] = effect;
        let mut changed = false;
        for (what, top, background) in [("picture", picture, Some("#000000")), ("chip", chip, None)]
        {
            let build = |linear: bool, top: Value| {
                let mut raw = project(
                    W,
                    H,
                    2.0,
                    background.map_or_else(no_fill, solid_fill),
                    vec![top],
                );
                asset(&mut raw, "pic", "image", "png", W, H, 0.0);
                asset(
                    &mut raw,
                    "big",
                    "image",
                    "png",
                    W + 2 * OVERSCAN,
                    H + 2 * OVERSCAN,
                    0.0,
                );
                raw["color"] = json!({ "mode": "standard", "linearBlending": linear });
                load(&raw)
            };
            let standard = render(&mut r, &build(false, top.clone()), &fixture.resolver, 0.5);
            let linear = render(&mut r, &build(true, top.clone()), &fixture.resolver, 0.5);
            let mut plain = top.clone();
            plain["fx"] = json!([]);
            let untouched = render(&mut r, &build(false, plain), &fixture.resolver, 0.5);
            changed |= diff(&standard, &untouched).0 < 0.98;
            let (within, max) = diff(&standard, &linear);
            if max > 4 {
                let code = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round();
                let mut shown = 0;
                for (i, (a, b)) in standard.chunks(4).zip(linear.chunks(4)).enumerate() {
                    if (0..4).any(|c| (code(a[c]) - code(b[c])).abs() > 4.0) && shown < 6 {
                        eprintln!(
                            "  px {} ({}, {}): standard {:?} linear {:?}",
                            i,
                            i % W as usize,
                            i / W as usize,
                            a,
                            b
                        );
                        shown += 1;
                    }
                }
            }
            eprintln!(
                "{id:<14} {what:<8} linear vs Standard: ≤1 CV {:.4}%  max {max}",
                100.0 * within
            );
            assert!(
                within >= 0.999 && max <= 4,
                "{id} {what}: {within} within 1 CV, max {max}"
            );
        }
        assert!(changed, "{id}: the parameters change the picture");
    }
    // The control: a 50% composite blends in linear light (section 2: 188,
    // not 128, for white over black).
    let mut half = layer(
        "half",
        "solid",
        json!({ "color": "#FFFFFF", "w": W, "h": H }),
        0.0,
        0.0,
        2.0,
    );
    set(&mut half, "opacity", json!(50));
    let on = render(
        &mut r,
        &document(vec![half.clone()], "#000000", true),
        &fixture.resolver,
        0.5,
    );
    let off = render(
        &mut r,
        &document(vec![half], "#000000", false),
        &fixture.resolver,
        0.5,
    );
    assert_eq!(
        ((on[0] * 255.0).round(), (off[0] * 255.0).round()),
        (188.0, 128.0)
    );
}

/// A blur across a hard black and white edge averages E values: the
/// midpoint is near 0.5 with linear blending on (a blur in linear light would
/// put it near E(0.5) = 0.735).
#[test]
fn a_blur_across_an_edge_averages_e_values() {
    let _gpu = gpu();
    let fixture = Fixture::new("fx-linear-edge");
    let mut r = renderer();
    let mut white = layer(
        "white",
        "solid",
        json!({ "color": "#FFFFFF", "w": W / 2, "h": H }),
        0.0,
        0.0,
        2.0,
    );
    white["fx"] = json!([]);
    let black = layer(
        "black",
        "solid",
        json!({ "color": "#000000", "w": W, "h": H }),
        0.0,
        0.0,
        2.0,
    );
    // A precomp-free way to blur the stack: an adjustment layer over both.
    let mut adj = layer("adj", "adjustment", json!({}), 0.0, 0.0, 2.0);
    adj["fx"] =
        json!([{ "id": "f1", "type": "blur", "on": true, "p": { "amount": ch(json!(12)) } }]);
    for linear in [false, true] {
        let doc = document(
            vec![adj.clone(), white.clone(), black.clone()],
            "#000000",
            linear,
        );
        let v = render(&mut r, &doc, &fixture.resolver, 0.5);
        let at = |x: u32| v[((H / 2 * W + x) * 4) as usize];
        let edge = at(W / 2 - 1).max(at(W / 2));
        let mid = (at(W / 2 - 1) + at(W / 2)) / 2.0;
        assert!(
            (mid - 0.5).abs() < 0.03,
            "linear {linear}: the edge midpoint is {mid}"
        );
        assert!(edge < 0.7, "linear {linear}: {edge}");
        assert!(
            at(4) > 0.99 && at(W - 4) < 0.01,
            "linear {linear}: far from the edge"
        );
    }
}

/// `u_backdrop` (the stack beneath) is in E too: a raw effect mixing the
/// layer with its backdrop renders the same with linear blending on.
#[test]
fn the_backdrop_is_e_encoded_with_linear_blending_on() {
    let _gpu = gpu();
    let fixture = Fixture::new("fx-linear-backdrop");
    let mut r = renderer();
    const MIX: &str = "#version 300 es\nprecision highp float;\nin vec2 v_st;\nuniform sampler2D u_tex;\nuniform sampler2D u_backdrop;\nout vec4 o;\nvoid main(){ vec4 a = texture(u_tex, v_st); vec4 b = texture(u_backdrop, v_st); o = vec4(mix(a.rgb, b.rgb, 0.5), 1.0); }\n";
    let mix: DefinitionSnapshot = serde_json::from_value(json!({
        "id": "com.example.mix", "kind": "effect", "version": 0, "origin": "local", "renderer": null,
        "glsl": { "source": MIX, "bodyLineOffset": 0, "bodyUtf16Len": MIX.encode_utf16().count(), "bodyUtf8Len": MIX.len() },
        "params": [],
        "flags": { "passes": 1, "keepOrig": false, "backdrop": true, "rawShader": true, "viewportSafe": false, "viewportPadding": [], "clamp": false }
    }))
    .unwrap();
    let mut set = builtin_definitions();
    set.definitions.push(mix);
    assert!(r.set_definitions(&set).unwrap().is_empty());
    let mut top = layer(
        "top",
        "solid",
        json!({ "color": "#FFFFFF", "w": W, "h": H }),
        0.0,
        0.0,
        2.0,
    );
    top["fx"] = json!([{ "id": "f1", "type": "com.example.mix", "on": true, "p": {} }]);
    let back = picture_layer("back");
    let mut out = Vec::new();
    for linear in [false, true] {
        let doc = document(vec![top.clone(), back.clone()], "#000000", linear);
        out.push(render(&mut r, &doc, &fixture.resolver, 0.5));
    }
    // White over a black bar: the mix is 0.5 in E (not E(0.5) = 0.735).
    let black_bar = ((H / 2 * W + 8) * 4) as usize;
    for (i, v) in out.iter().enumerate() {
        let white_bar = (0..W / 4)
            .map(|x| ((H / 2 * W + x) * 4) as usize)
            .find(|&p| v[p] > 0.9);
        assert!(white_bar.is_some(), "a white bar under the layer");
        assert!(
            (v[black_bar] - 0.5).abs() < 2.0 / 255.0 || (v[black_bar] - 1.0).abs() < 2.0 / 255.0,
            "{i}: {}",
            v[black_bar]
        );
    }
    let (within, max) = diff(&out[0], &out[1]);
    assert!(within >= 0.999 && max <= 2, "{within} {max}");
}

/// Transitions mix their two inputs in E: each built-in transition with
/// linear blending on renders what it renders in Standard, at several
/// progress values.
#[test]
fn transitions_mix_e_values_with_linear_blending_on() {
    let _gpu = gpu();
    let fixture = Fixture::new("tr-linear");
    let mut r = renderer();
    for (ty, p) in [
        ("crossfade", json!({})),
        (
            "wipe",
            json!({ "angle": ch(json!(30)), "softness": ch(json!(40)) }),
        ),
        ("slide", json!({ "direction": ch(json!(90)) })),
        ("zoom", json!({ "amount": ch(json!(40)) })),
    ] {
        let mut top = picture_layer("top");
        top["transitionIn"] = json!({ "type": ty, "dur": 1.0, "p": p });
        let back = layer(
            "back",
            "solid",
            json!({ "color": "#3355AA", "w": W, "h": H }),
            0.0,
            0.0,
            2.0,
        );
        for t in [0.2, 0.5, 0.8] {
            let standard = render(
                &mut r,
                &document(vec![top.clone(), back.clone()], "#000000", false),
                &fixture.resolver,
                t,
            );
            let linear = render(
                &mut r,
                &document(vec![top.clone(), back.clone()], "#000000", true),
                &fixture.resolver,
                t,
            );
            let (within, max) = diff(&standard, &linear);
            eprintln!("{ty:<10} t={t}: ≤1 CV {:.4}%  max {max}", 100.0 * within);
            assert!(within >= 0.999 && max <= 4, "{ty} t={t}: {within} {max}");
        }
    }
    // A crossfade at 50% between white and black is 0.5 in E either way.
    let mut white = layer(
        "white",
        "solid",
        json!({ "color": "#FFFFFF", "w": W, "h": H }),
        0.0,
        0.0,
        2.0,
    );
    white["transitionIn"] = json!({ "type": "crossfade", "dur": 1.0, "p": {} });
    for linear in [false, true] {
        let v = render(
            &mut r,
            &document(vec![white.clone()], "#000000", linear),
            &fixture.resolver,
            0.5,
        );
        assert!((v[0] - 0.5).abs() < 1e-3, "linear {linear}: {}", v[0]);
    }
}

/// An adjustment layer at 100% with linear blending on: its effects read
/// the stack beneath in E.
#[test]
fn adjustment_layers_read_e_with_linear_blending_on() {
    let _gpu = gpu();
    let fixture = Fixture::new("adj-linear");
    let mut r = renderer();
    for (id, p) in [
        ("invert", json!({ "amount": ch(json!(100)) })),
        ("blur", json!({ "amount": ch(json!(10)) })),
        (
            "duotone",
            json!({ "shadow": ch(json!("#1b2a4a")), "highlight": ch(json!("#ffd66b")), "amount": ch(json!(100)) }),
        ),
    ] {
        let mut adj = layer("adj", "adjustment", json!({}), 0.0, 0.0, 2.0);
        adj["fx"] = json!([{ "id": "f1", "type": id, "on": true, "p": p }]);
        let stack = vec![adj, picture_layer("pic")];
        let standard = render(
            &mut r,
            &document(stack.clone(), "#000000", false),
            &fixture.resolver,
            0.5,
        );
        let linear = render(
            &mut r,
            &document(stack, "#000000", true),
            &fixture.resolver,
            0.5,
        );
        let (within, max) = diff(&standard, &linear);
        eprintln!(
            "adjustment {id:<8}: ≤1 CV {:.4}%  max {max}",
            100.0 * within
        );
        assert!(within >= 0.999 && max <= 4, "{id}: {within} {max}");
    }
}
