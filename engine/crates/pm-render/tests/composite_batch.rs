//! Normal-blend layer nodes draw into the accumulator in batches
//! (`composite/exec.rs`, `draws_over_the_stack`): a node that reads nothing
//! of the stack beneath it records its own passes first, and its image
//! joins the next accumulator pass with the plain draws around it, so
//! several nodes share one full-frame load and store. Nodes that read the
//! stack (an adjustment, another blend mode, a transition, an effect
//! reading the backdrop) still get the stack drawn before them.
//!
//! The pixels must not change by a bit: every frame is compared with the
//! reference that records one accumulator pass per node
//! (`Renderer::set_early_target_release(false)`, which also holds every
//! target to the end of the frame), and the batched frame records fewer
//! render passes.

mod support;

use pm_perf::{Counter, Scope};
use pm_protocol::color::Precision;
use pm_protocol::common::{PixelRect, Size};
use pm_protocol::definitions::DefinitionSnapshot;
use pm_render::{MapResolver, OutputKind, RenderRequest, Renderer, builtin_definitions};
use serde_json::{Value, json};
use support::doc::*;
use support::ticks;

/// The tests take turns on the GPU (as `effects.rs`).
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|e| e.into_inner())
}

const W: u32 = 320;
const H: u32 = 180;
const DUR: f64 = 2.0;

fn fx(id: &str, ty: &str, p: Value) -> Value {
    json!({ "id": id, "type": ty, "on": true, "p": p })
}

fn glow(id: &str) -> Value {
    fx(
        id,
        "glow",
        json!({ "radius": ch(json!(12)), "threshold": ch(json!(20)), "intensity": ch(json!(150)) }),
    )
}

fn blur(id: &str, amount: f64) -> Value {
    fx(id, "blur", json!({ "amount": ch(json!(amount)) }))
}

/// Mixes the layer with the stack beneath it (`u_backdrop`): wrong unless
/// that stack is in the accumulator when the effect runs.
const BEHIND: &str = "#version 300 es
precision highp float;
in vec2 v_st;
uniform sampler2D u_tex;
uniform sampler2D u_backdrop;
out vec4 o;
void main(){
  vec4 a = texture(u_tex, v_st);
  vec4 b = texture(u_backdrop, v_st);
  o = vec4(mix(a.rgb, b.bgr, 0.5) * a.a, a.a);
}
";

fn behind() -> DefinitionSnapshot {
    serde_json::from_value(json!({
        "id": "acme.behind", "kind": "effect", "version": 0, "origin": "local", "renderer": null,
        "glsl": { "source": BEHIND, "bodyLineOffset": 0, "bodyUtf16Len": BEHIND.encode_utf16().count(), "bodyUtf8Len": BEHIND.len() },
        "params": [],
        "flags": { "passes": 1, "keepOrig": false, "backdrop": true, "rawShader": true, "viewportSafe": false, "viewportPadding": [], "clamp": false }
    }))
    .unwrap()
}

fn renderer() -> Renderer {
    let mut set = builtin_definitions();
    set.definitions.push(behind());
    let mut r = Renderer::headless().unwrap();
    let failures = r.set_definitions(&set).unwrap();
    assert!(failures.is_empty(), "{failures:?}");
    r
}

fn solid(id: &str, color: &str, w: u32, h: u32, x: f64, y: f64) -> Value {
    layer(
        id,
        "solid",
        json!({ "color": color, "w": w, "h": h }),
        x,
        y,
        DUR,
    )
}

fn rect_mask(x: f64, y: f64, w: f64, h: f64, feather: f64) -> Value {
    json!([{ "id": "k", "shape": "rect", "mode": "add", "on": true,
        "p": { "x": ch(json!(x)), "y": ch(json!(y)), "w": ch(json!(w)), "h": ch(json!(h)),
               "rotation": ch(json!(0)), "feather": ch(json!(feather)) } }])
}

const COLORS: [&str; 6] = [
    "#FF6B1A", "#4C8DFF", "#3FCF8E", "#F0C020", "#C04080", "#20E0E0",
];

/// `n` overlapping effect leaves (layer nodes, normal blend) with a plain
/// solid (a draw, not a node) between every two, so the deferred images
/// and the plain draws interleave in the accumulator pass.
fn leaves(prefix: &str, n: usize, y: f64) -> Vec<Value> {
    let mut layers = Vec::new();
    for i in 0..n {
        let mut leaf = solid(
            &format!("{prefix}{i}"),
            COLORS[i % COLORS.len()],
            30,
            22,
            6.0 + 21.0 * i as f64,
            y + 4.0 * (i % 3) as f64,
        );
        set(&mut leaf, "rotation", json!(-6 + i as i64));
        set(&mut leaf, "opacity", json!(70 + (i % 4) * 10));
        leaf["fx"] = Value::Array(if i % 3 == 0 {
            vec![blur(&format!("{prefix}b{i}"), 2.0)]
        } else {
            vec![glow(&format!("{prefix}g{i}"))]
        });
        layers.push(leaf);
        if i % 2 == 1 {
            layers.push(solid(
                &format!("{prefix}plain{i}"),
                COLORS[(i + 3) % COLORS.len()],
                14,
                40,
                14.0 + 21.0 * i as f64,
                y - 6.0,
            ));
        }
    }
    layers
}

/// Batched nodes between every kind of node that reads the stack beneath
/// it, and inside a precomp (a nested node loop, deferring at the same
/// time as the loop around it).
fn document() -> pm_model::Document {
    // Top to bottom.
    let mut layers = Vec::new();
    layers.extend(leaves("top", 5, 150.0));
    // An adjustment over everything beneath it.
    let mut adj = layer("adj", "adjustment", json!({}), 0.0, 0.0, DUR);
    adj["fx"] = json!([glow("a1")]);
    adj["masks"] = rect_mask(f64::from(W) * 0.45, f64::from(H) / 2.0, 220.0, 150.0, 6.0);
    set(&mut adj, "opacity", json!(80));
    layers.push(adj);
    layers.extend(leaves("mid", 4, 120.0));
    // A transition halfway (crossfade at 0.5 s of 1 s): it reads the stack.
    let mut fading = solid("fading", "#E0E0FF", 80, 60, 60.0, 90.0);
    fading["fx"] = json!([blur("fb", 3.0)]);
    fading["transitionIn"] = json!({ "type": "crossfade", "dur": 1.0, "p": {} });
    layers.push(fading);
    layers.extend(leaves("low", 4, 96.0));
    // An effect reading the backdrop.
    let mut reader = solid("reader", "#FFFFFF", 120, 70, 150.0, 60.0);
    reader["fx"] = json!([fx("r1", "acme.behind", json!({}))]);
    layers.push(reader);
    layers.extend(leaves("deep", 9, 64.0));
    // A non-normal effect node, and a non-normal full-frame solid (the
    // accumulator's own blend path).
    let mut screen = solid("screen", "#8040FF", 90, 60, 200.0, 20.0);
    screen["blend"] = json!("screen");
    screen["fx"] = json!([glow("s1")]);
    layers.push(screen);
    let mut multiply = solid("multiply", "#C0E0FF", W, H, 0.0, 0.0);
    multiply["blend"] = json!("multiply");
    set(&mut multiply, "opacity", json!(50));
    layers.push(multiply);
    // A matted effect layer (its matte source just below it), a masked
    // effect layer, and a masked layer without effects: all normal blend,
    // all batched.
    let mut matted = solid("matted", "#3FCF8E", 80, 50, 20.0, 120.0);
    matted["matteSource"] = json!("matte");
    matted["matteMode"] = ch(json!("alpha"));
    matted["fx"] = json!([blur("m1", 6.0)]);
    layers.push(matted);
    let mut matte = solid("matte", "#FFFFFF99", 50, 50, 35.0, 120.0);
    set(&mut matte, "rotation", json!(20));
    layers.push(matte);
    let mut masked = solid("masked", "#FF4060", 100, 70, 210.0, 90.0);
    masked["masks"] = rect_mask(50.0, 35.0, 60.0, 50.0, 3.0);
    masked["fx"] = json!([glow("k1")]);
    layers.push(masked);
    let mut plain_mask = solid("plain-mask", "#40A0FF", 70, 50, 250.0, 10.0);
    plain_mask["masks"] = rect_mask(35.0, 25.0, 40.0, 30.0, 2.0);
    layers.push(plain_mask);
    // A precomp of effect leaves, itself with an effect (a node holding a
    // nested node loop).
    let mut pc = layer(
        "pc",
        "precomp",
        json!({ "comp": "N", "w": 200, "h": 120 }),
        40.0,
        30.0,
        DUR,
    );
    pc["fx"] = json!([blur("pcb", 1.5)]);
    layers.push(pc);
    layers.extend(leaves("bottom", 6, 20.0));
    layers.push(solid("back", "#2A2F45", W, H, 0.0, 0.0));
    let mut doc = project(W, H, DUR, solid_fill("#14161C"), layers);
    let mut nested = leaves("n", 10, 50.0);
    nested.push(solid("nback", "#405070", 200, 120, 0.0, 0.0));
    composition(&mut doc, "N", 200, 120, DUR, nested);
    load(&doc)
}

fn preview() -> RenderRequest {
    RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float16)
}

fn requests() -> Vec<(&'static str, RenderRequest)> {
    let mut final_frame = preview();
    final_frame.output_kind = OutputKind::Final;
    let mut crop = preview();
    crop.region = Some(PixelRect {
        x: 101,
        y: 37,
        width: 150,
        height: 100,
    });
    let mut scaled = RenderRequest::at_scale(Size::new(W, H), 0.75, Precision::Float16);
    scaled.output_kind = OutputKind::Final;
    vec![
        ("preview", preview()),
        ("final", final_frame),
        ("crop", crop),
        ("final at 75 %", scaled),
        (
            "float32",
            RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32),
        ),
    ]
}

/// The frame's bits and the render passes it recorded.
fn render(
    r: &mut Renderer,
    doc: &pm_model::Document,
    t: f64,
    req: &RenderRequest,
) -> (Vec<u32>, u64) {
    let scope = Scope::begin();
    let frame = r
        .render(doc, "Ptest-root", ticks(t), req, &MapResolver::default())
        .unwrap();
    let passes = scope.finish().get(Counter::RenderPasses);
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let bits = r
        .read_rgba(&frame)
        .unwrap()
        .into_iter()
        .map(f32::to_bits)
        .collect();
    (bits, passes)
}

fn first_difference(a: &[u32], b: &[u32]) -> Option<(usize, f32, f32)> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .map(|i| (i, f32::from_bits(a[i]), f32::from_bits(b[i])))
}

/// Renders `doc` as the reference, then batched twice (a cold and a warm
/// pool), then the reference again: all bit-equal. The render passes of
/// the reference and the batched frame.
fn compare(
    r: &mut Renderer,
    doc: &pm_model::Document,
    t: f64,
    what: &str,
    req: &RenderRequest,
) -> (u64, u64) {
    r.set_early_target_release(false);
    let (reference, reference_passes) = render(r, doc, t, req);
    assert!(
        reference
            .iter()
            .any(|v| f32::from_bits(*v) > 0.1 && f32::from_bits(*v) < 1.0),
        "{what}: the reference draws something"
    );
    r.set_early_target_release(true);
    let mut batched_passes = 0;
    for pass in ["first", "second"] {
        let (batched, passes) = render(r, doc, t, req);
        assert_eq!(batched.len(), reference.len(), "{what}");
        assert!(
            batched == reference,
            "{what}, {pass} batched frame: first difference (index, batched, reference) {:?}",
            first_difference(&batched, &reference)
        );
        batched_passes = passes;
    }
    r.set_early_target_release(false);
    assert!(render(r, doc, t, req).0 == reference, "{what}");
    (reference_passes, batched_passes)
}

#[test]
fn batched_node_composites_keep_every_bit_and_record_fewer_passes() {
    let _gpu = gpu();
    let doc = document();
    let mut r = renderer();
    // 0.5 s: the crossfade is halfway.
    for (what, req) in requests() {
        let (reference, batched) = compare(&mut r, &doc, 0.5, what, &req);
        eprintln!("{what}: render passes {reference} one per node, {batched} batched");
        // Top level: 5 + 4 + 4 + 9 + 3 + 6 batched nodes (the precomp
        // included) in runs between the 4 nodes reading the stack; the
        // nine in a row split once at `DEFERRED_IMAGES`. The precomp's own
        // loop batches its 10 leaves too.
        assert!(
            reference >= batched + 20,
            "{what}: {reference} passes one per node, {batched} batched"
        );
    }
    // After the transition: a plain normal-blend node like the others.
    let (reference, batched) = compare(&mut r, &doc, 1.5, "after the transition", &preview());
    assert!(reference > batched, "{reference} vs {batched}");
}

/// A frame whose batched images reach the byte bound: three 2880 × 1620
/// half-float images (37.3 MB each) fit in 128 MiB, a fourth does not, so
/// eight leaves in a row draw in three accumulator passes, not one. The
/// bits are the reference's.
#[test]
fn a_frame_past_the_byte_bound_keeps_every_bit() {
    let _gpu = gpu();
    let (w, h) = (2880u32, 1620u32);
    let mut layers = Vec::new();
    for i in 0..8 {
        let mut leaf = solid(
            &format!("big{i}"),
            COLORS[i % COLORS.len()],
            400,
            300,
            100.0 + 300.0 * i as f64,
            200.0 + 120.0 * (i % 3) as f64,
        );
        set(&mut leaf, "rotation", json!(4 * i as i64 - 10));
        leaf["fx"] = json!([blur(&format!("bb{i}"), 6.0)]);
        layers.push(leaf);
    }
    layers.push(solid("back", "#2A2F45", w, h, 0.0, 0.0));
    let doc = load(&project(w, h, DUR, solid_fill("#14161C"), layers));
    let mut r = renderer();
    let req = RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float16);
    let (reference, batched) = compare(&mut r, &doc, 0.5, "2880 × 1620", &req);
    eprintln!("2880 × 1620: render passes {reference} one per node, {batched} batched");
    // One per node and the last (9) against at most three (bound) and the
    // last: at least five fewer.
    assert!(
        reference >= batched + 5,
        "{reference} passes one per node, {batched} batched"
    );
}
