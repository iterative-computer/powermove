//! Effect passes scissored to the pixels a zero-preserving program can
//! change (`effects::bounds`) render exactly the bits of passes that shade
//! their whole target: every comparison here is of the frames' bits, not a
//! tolerance.
mod support;
use pm_protocol::color::Precision;
use pm_protocol::common::{PixelRect, Size};
use pm_render::{MapResolver, OutputKind, RenderRequest, Renderer};
use serde_json::{Value, json};
use support::doc::*;
use support::ticks;

const W: u32 = 256;
const H: u32 = 144;

/// The tests take turns on the GPU (as `effects.rs`).
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|e| e.into_inner())
}

fn blur2(amount: f64, repeat_edges: bool) -> Value {
    json!({"id":"b2","type":"blur","version":2,"on":true,"p":{
        "amount":ch(json!(amount)),"dimensions":ch(json!(0)),"repeatEdgePixels":ch(json!(repeat_edges))}})
}
fn blur1(amount: f64) -> Value {
    json!({"id":"b1","type":"blur","version":1,"on":true,"p":{"amount":ch(json!(amount))}})
}
fn shadow1(distance: f64, angle: f64, softness: f64, opacity: f64) -> Value {
    json!({"id":"s1","type":"shadow","version":1,"on":true,"p":{
        "distance":ch(json!(distance)),"angle":ch(json!(angle)),
        "softness":ch(json!(softness)),"opacity":ch(json!(opacity)),
        "color":ch(json!("#182040"))}})
}
fn shadow2(distance: f64, angle: f64, softness: f64, opacity: f64, shadow_only: bool) -> Value {
    json!({"id":"s2","type":"shadow","version":2,"on":true,"p":{
        "color":ch(json!("#201030")),"opacity":ch(json!(opacity)),
        "angle":ch(json!(angle)),"distance":ch(json!(distance)),
        "softness":ch(json!(softness)),"shadowOnly":ch(json!(shadow_only))}})
}
fn glow(threshold: f64, radius: f64, intensity: f64) -> Value {
    json!({"id":"g","type":"glow","version":1,"on":true,"p":{
        "threshold":ch(json!(threshold)),"radius":ch(json!(radius)),"intensity":ch(json!(intensity))}})
}
fn color1() -> Value {
    json!({"id":"c","type":"color","version":1,"on":true,"p":{
        "exposure":ch(json!(0)),"contrast":ch(json!(20)),"saturation":ch(json!(30)),
        "hue":ch(json!(0)),"temperature":ch(json!(0))}})
}
fn levels1() -> Value {
    json!({"id":"l1","type":"levels","version":1,"on":true,"p":{
        "inBlack":ch(json!(10)),"inWhite":ch(json!(90)),"gamma":ch(json!(1.2))}})
}
/// Levels v2 as the bench's effects scene sets it; `alpha_out_black` lifts
/// transparent pixels (not zero-preserving).
fn levels2(alpha_out_black: f64) -> Value {
    let mut p = serde_json::Map::new();
    for (k, v) in [
        ("inBlack", 0.1),
        ("inWhite", 0.9),
        ("gamma", 1.2),
        ("outBlack", 0.),
        ("outWhite", 1.),
    ] {
        p.insert(k.into(), ch(json!(v)));
    }
    for channel in ["red", "green", "blue", "alpha"] {
        for (k, v) in [
            ("InBlack", 0.),
            ("InWhite", 1.),
            ("Gamma", 1.),
            ("OutBlack", 0.),
            ("OutWhite", 1.),
        ] {
            p.insert(format!("{channel}{k}"), ch(json!(v)));
        }
    }
    p.insert("alphaOutBlack".into(), ch(json!(alpha_out_black)));
    p.insert("clipBlack".into(), ch(json!(true)));
    p.insert("clipWhite".into(), ch(json!(true)));
    json!({"id":"l2","type":"levels","version":2,"on":true,"p":p})
}

fn solid(id: &str, color: &str, w: u32, h: u32, x: f64, y: f64) -> Value {
    layer(id, "solid", json!({"w":w,"h":h,"color":color}), x, y, 2.)
}

/// Small layers among a mostly empty frame: rotated and animated, one
/// partly off the frame, one masked (feathered), one under a track matte.
fn scene(extra: Vec<Value>) -> Value {
    let mut a = solid("a", "#F0C040", 40, 30, 60., 50.);
    a["p"]["rotation"] = keys(json!(0), &[(0., json!(0)), (1., json!(90))]);
    a["fx"] = json!([blur2(8., false), glow(40., 20., 150.), levels2(0.)]);
    let mut b = solid("b", "#40A0F0", 30, 30, 250., 20.);
    b["fx"] = json!([blur1(6.), color1()]);
    let mut c = solid("c", "#FFFFFF", 36, 36, 150., 100.);
    c["masks"] = json!([{"id":"m","shape":"rect","mode":"add","on":true,"p":{
        "x":ch(json!(4)),"y":ch(json!(4)),"w":ch(json!(20)),"h":ch(json!(28)),
        "rotation":ch(json!(15)),"feather":ch(json!(4))}}]);
    c["fx"] = json!([glow(20., 12., 90.), levels1()]);
    let mut d = solid("d", "#E04080", 30, 20, 200., 110.);
    d["matteSource"] = json!("s");
    d["matteMode"] = ch(json!("alpha"));
    d["fx"] = json!([blur2(3., false), levels2(0.)]);
    let s = solid("s", "#FFFFFF99", 20, 20, 205., 112.);
    let mut layers = vec![a, b, c, d, s];
    layers.extend(extra);
    project(W, H, 2., no_fill(), layers)
}

struct Variant {
    precision: Precision,
    linear: bool,
    kind: OutputKind,
    region: Option<PixelRect>,
    scale: f64,
}

fn render(r: &mut Renderer, v: &Value, t: f64, variant: &Variant) -> Vec<u32> {
    let mut v = v.clone();
    v["colorDepth"] = json!(match variant.precision {
        Precision::Unorm8 => 8,
        Precision::Float16 => 16,
        Precision::Float32 => 32,
    });
    v["color"] = json!({"mode":"standard","linearBlending":variant.linear});
    let mut doc = load(&v);
    doc.set_saving_app_version(Some("1.3.0".into()));
    r.set_main_effects_for_document(&doc).unwrap();
    let mut req = RenderRequest::at_scale(Size::new(W, H), variant.scale, variant.precision);
    req.viewer = variant.kind == OutputKind::Preview;
    req.output_kind = variant.kind;
    req.region = variant.region;
    req.background = false;
    let frame = r
        .render(&doc, "Ptest-root", ticks(t), &req, &MapResolver::default())
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    r.read_rgba(&frame)
        .unwrap()
        .into_iter()
        .map(f32::to_bits)
        .collect()
}

/// Renders with the scissor on and off; asserts the same bits and returns
/// how many passes were scissored.
fn compare(r: &mut Renderer, v: &Value, t: f64, variant: &Variant, what: &str) -> u64 {
    r.set_effect_scissor(false);
    let scope = pm_perf::Scope::begin();
    let whole = render(r, v, t, variant);
    assert_eq!(
        scope.finish().get(pm_perf::Counter::EffectScissorHits),
        0,
        "{what}: scissored with the scissor off"
    );
    r.set_effect_scissor(true);
    let scope = pm_perf::Scope::begin();
    let kept = render(r, v, t, variant);
    let hits = scope.finish().get(pm_perf::Counter::EffectScissorHits);
    if let Some(i) = whole.iter().zip(&kept).position(|(a, b)| a != b) {
        panic!(
            "{what}: pixel {} channel {} differs: whole {:?} scissored {:?}",
            i / 4,
            i % 4,
            f32::from_bits(whole[i]),
            f32::from_bits(kept[i])
        );
    }
    assert_eq!(whole.len(), kept.len(), "{what}");
    if !what.contains("detail 0 resolve 0 ") {
        assert!(
            whole.iter().any(|b| *b != 0),
            "{what}: the frame drew nothing"
        );
    }
    hits
}

#[test]
fn scissored_effect_passes_are_bit_identical() {
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    let v = scene(vec![]);
    let region = PixelRect {
        x: 40,
        y: 24,
        width: 160,
        height: 96,
    };
    let mut hits = 0;
    for precision in [Precision::Float16, Precision::Float32] {
        for linear in [false, true] {
            for kind in [OutputKind::Final, OutputKind::Preview] {
                for region in [None, Some(region)] {
                    for t in [0., 0.5, 1.] {
                        let variant = Variant {
                            precision,
                            linear,
                            kind,
                            region,
                            scale: 1.,
                        };
                        let what = format!(
                            "{precision:?} linear {linear} {kind:?} region {region:?} t {t}"
                        );
                        hits += compare(&mut r, &v, t, &variant, &what);
                    }
                }
            }
        }
    }
    // A fractional density (effects reach more output pixels).
    for (precision, linear, kind, region, t) in [
        (Precision::Float16, false, OutputKind::Final, None, 0.5),
        (
            Precision::Float32,
            true,
            OutputKind::Preview,
            Some(PixelRect {
                x: 60,
                y: 36,
                width: 240,
                height: 144,
            }),
            1.,
        ),
    ] {
        let variant = Variant {
            precision,
            linear,
            kind,
            region,
            scale: 1.5,
        };
        let what = format!("×1.5 {precision:?} linear {linear} {kind:?} region {region:?} t {t}");
        hits += compare(&mut r, &v, t, &variant, &what);
    }
    assert!(hits > 0, "no pass was scissored");
}

/// Large reaches: the scissor grows past the frame's edges and still holds
/// every lit pixel.
#[test]
fn wide_reaches_stay_bit_identical() {
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    for amount in [0.5, 40., 200.] {
        let mut l = solid("w", "#FFFFFF", 12, 12, 128., 72.);
        l["fx"] = json!([blur2(amount, false), glow(10., amount.min(120.), 200.), blur1(amount)]);
        let v = project(W, H, 2., no_fill(), vec![l]);
        let variant = Variant {
            precision: Precision::Float32,
            linear: false,
            kind: OutputKind::Final,
            region: None,
            scale: 1.,
        };
        compare(&mut r, &v, 0., &variant, &format!("amount {amount}"));
    }
}

/// Drop Shadow v1 and v2 are zero-preserving and scissor bit-identically
/// across angles, distances, softness, shadow-only mode, rotation, and scale.
#[test]
fn shadow_v1_and_v2_scissored_passes_are_bit_identical() {
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    let mut a = solid("a", "#F0C040", 32, 24, 90., 60.);
    a["p"]["rotation"] = ch(json!(28.));
    a["fx"] = json!([shadow2(18., 135., 16., 75., false)]);
    let mut b = solid("b", "#40A0F0", 28, 20, 175., 95.);
    b["fx"] = json!([shadow2(12., -45., 24., 85., true)]);
    let mut c = solid("c", "#E04080", 26, 22, 60., 110.);
    c["fx"] = json!([shadow1(14., 210., 18., 70.)]);
    let v = project(W, H, 2., no_fill(), vec![a, b, c]);
    let mut hits = 0;
    for (precision, linear, kind, scale) in [
        (Precision::Float16, false, OutputKind::Final, 1.0),
        (Precision::Float32, true, OutputKind::Final, 1.0),
        (Precision::Float16, false, OutputKind::Preview, 1.5),
    ] {
        let variant = Variant {
            precision,
            linear,
            kind,
            region: None,
            scale,
        };
        hits += compare(
            &mut r,
            &v,
            0.,
            &variant,
            &format!("shadow {precision:?} linear {linear} {kind:?} scale {scale}"),
        );
    }
    assert!(hits > 0, "shadow passes were not scissored");
}

/// Steps that can lift a transparent pixel are never scissored, nor is any
/// step after them.
#[test]
fn steps_that_do_not_keep_zero_are_not_scissored() {
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    let variant = Variant {
        precision: Precision::Float16,
        linear: false,
        kind: OutputKind::Final,
        region: None,
        scale: 1.,
    };
    for (what, fx) in [
        ("blur v2 repeating edge pixels", json!([blur2(6., true), blur1(4.)])),
        ("levels v2 lifting alpha", json!([levels2(0.2), blur1(4.)])),
    ] {
        let mut l = solid("n", "#80FF80", 30, 30, 128., 72.);
        l["fx"] = fx;
        let v = project(W, H, 2., no_fill(), vec![l]);
        let hits = compare(&mut r, &v, 0., &variant, what);
        assert_eq!(hits, 0, "{what} was scissored");
    }
}

/// The reviewed `dot-matrix-resolve` extension (pinned in
/// `effects::bounds` by the SHA-256 of its wrapped source) scissored to
/// its reach renders exactly the bits of whole passes: on its own layers
/// and on group boundaries (looked through to their members), across its
/// parameter range, in final output (raster space) and viewer frames at
/// several densities.
///
/// Its source is a user's extension and is not committed: point
/// `PM_DOT_MATRIX_DEFINITIONS` at a `DefinitionSetSnapshot` JSON holding it
/// (the scene bench's `<rundir>/definitions.json`). Without it the test
/// passes without rendering.
#[test]
fn reviewed_extension_scissor_is_bit_identical() {
    use sha2::Digest;
    let Ok(path) = std::env::var("PM_DOT_MATRIX_DEFINITIONS") else {
        eprintln!("PM_DOT_MATRIX_DEFINITIONS is not set: skipped");
        return;
    };
    let snapshot: pm_protocol::definitions::DefinitionSetSnapshot =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let definition = snapshot
        .definitions
        .into_iter()
        .find(|d| d.id.as_str() == "dot-matrix-resolve")
        .expect("dot-matrix-resolve in the definitions");
    let source = &definition.glsl.as_ref().unwrap().source;
    let digest: String = sha2::Sha256::digest(source.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    eprintln!(
        "dot-matrix-resolve: {} bytes, sha256 {digest}",
        source.len()
    );
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    let dots = |pitch: f64, detail: f64, resolve: f64, heat: f64| {
        json!({"id":"dm","type":"dot-matrix-resolve","on":true,"p":{
            "pitch":ch(json!(pitch)),"detail":ch(json!(detail)),"dot":ch(json!(1.35)),
            "heat":ch(json!(heat)),"resolve":ch(json!(resolve)),"color":ch(json!("#FF690F"))}})
    };
    // A group boundary with the effect over a nested boundary (half
    // opacity), a rotated member, and one partly off the frame; and a
    // layer with the effect of its own.
    let document = |fx: Value| {
        let mut g = layer("g", "group", json!({}), 0., 0., 2.);
        g["fx"] = json!([fx.clone()]);
        let mut h = layer("h", "group", json!({}), 0., 0., 2.);
        h["group"] = json!("g");
        set(&mut h, "opacity", json!(50));
        let member = |id: &str, color: &str, w: u32, h: u32, x: f64, y: f64, group: &str| {
            let mut l = solid(id, color, w, h, x, y);
            l["group"] = json!(group);
            l
        };
        let mut rotated = member("r", "#40A0F0", 20, 12, 150., 90., "g");
        set(&mut rotated, "rotation", json!(30));
        let mut own = solid("o", "#FFFFFF", 28, 18, 210., 60.);
        own["fx"] = json!([fx]);
        project(
            W,
            H,
            2.,
            no_fill(),
            vec![
                g,
                member("a", "#F0C040", 24, 16, 60., 40., "g"),
                h,
                member("b", "#E04080", 16, 16, 110., 30., "h"),
                member("c", "#FFFFFF", 30, 10, -12., 120., "g"),
                rotated,
                own,
            ],
        )
    };
    let mut hits = 0;
    for (pitch, detail, resolve, heat) in [
        (8., 3., 0., 100.),
        (10., 1.6, 40., 55.),
        (10., 0., 0., 100.),
        (2., 5., 20., 0.),
        (1., 2., 10., 50.),
        (16., 2.5, 70., 100.),
        (64., 5., 0., 100.),
        (9., 3., 100., 0.),
    ] {
        let v = document(dots(pitch, detail, resolve, heat));
        for (kind, scale, region) in [
            (OutputKind::Final, 1., None),
            (OutputKind::Final, 0.75, None),
            (OutputKind::Final, 1.5, None),
            (OutputKind::Preview, 1., None),
            (OutputKind::Preview, 0.75, None),
            (OutputKind::Preview, 1.5, None),
            (
                OutputKind::Preview,
                1.,
                Some(PixelRect {
                    x: 40,
                    y: 24,
                    width: 160,
                    height: 96,
                }),
            ),
        ] {
            for precision in [Precision::Float16, Precision::Float32] {
                let variant = Variant {
                    precision,
                    linear: false,
                    kind,
                    region,
                    scale,
                };
                let what = format!(
                    "pitch {pitch} detail {detail} resolve {resolve} heat {heat} \
                     {kind:?} ×{scale} {region:?} {precision:?}"
                );
                hits += compare_with(&mut r, &v, 0., &variant, &what, &[definition.clone()]);
            }
        }
    }
    assert!(
        hits > 0,
        "no pass was scissored: is sha256 {digest} the reviewed pin?"
    );
}

/// [`compare`] with contributed definitions.
fn compare_with(
    r: &mut Renderer,
    v: &Value,
    t: f64,
    variant: &Variant,
    what: &str,
    contributed: &[pm_protocol::definitions::DefinitionSnapshot],
) -> u64 {
    let run = |r: &mut Renderer, scissor: bool| {
        r.set_effect_scissor(scissor);
        let mut v = v.clone();
        v["colorDepth"] = json!(match variant.precision {
            Precision::Unorm8 => 8,
            Precision::Float16 => 16,
            Precision::Float32 => 32,
        });
        v["color"] = json!({"mode":"standard","linearBlending":variant.linear});
        let mut doc = load(&v);
        doc.set_saving_app_version(Some("1.3.0".into()));
        let refused = r.set_document_effects(&doc, contributed).unwrap();
        assert!(refused.is_empty(), "{what}: {refused:?}");
        let mut req = RenderRequest::at_scale(Size::new(W, H), variant.scale, variant.precision);
        req.viewer = variant.kind == OutputKind::Preview;
        req.output_kind = variant.kind;
        req.region = variant.region;
        req.background = false;
        let scope = pm_perf::Scope::begin();
        let frame = r
            .render(&doc, "Ptest-root", ticks(t), &req, &MapResolver::default())
            .unwrap();
        assert!(frame.failures.is_empty(), "{what}: {:?}", frame.failures);
        let bits: Vec<u32> = r
            .read_rgba(&frame)
            .unwrap()
            .into_iter()
            .map(f32::to_bits)
            .collect();
        (
            bits,
            scope.finish().get(pm_perf::Counter::EffectScissorHits),
        )
    };
    let (whole, off) = run(r, false);
    assert_eq!(off, 0, "{what}: scissored with the scissor off");
    let (kept, hits) = run(r, true);
    assert_eq!(whole.len(), kept.len(), "{what}");
    if let Some(i) = whole.iter().zip(&kept).position(|(a, b)| a != b) {
        panic!(
            "{what}: pixel {} channel {} differs: whole {:?} scissored {:?}",
            i / 4,
            i % 4,
            f32::from_bits(whole[i]),
            f32::from_bits(kept[i])
        );
    }
    if !what.contains("detail 0 resolve 0 ") {
        assert!(
            whole.iter().any(|b| *b != 0),
            "{what}: the frame drew nothing"
        );
    }
    hits
}
