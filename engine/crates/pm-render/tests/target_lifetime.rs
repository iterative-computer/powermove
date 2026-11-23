//! Target lifetime within a frame (`Exec::recycle`, `TargetPool::recycle`):
//! node content, masks, mattes, effect passes, group passes, layer images,
//! and accumulators go back to the pool as soon as the last pass reading
//! them is recorded, and the frame's later passes draw into them. The
//! pixels must not change by a bit: every frame here is compared with the
//! reference that holds each target until the frame is submitted
//! (`Renderer::set_early_target_release(false)`), in a preview, a final
//! frame, a crop, and with the depth and id surface.

mod support;

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

/// Four passes, each reading the pass before it (shifted, so a stale or
/// wrong texture shows) and `u_orig` (the step's input, which must outlive
/// every pass of the step).
const ORIG4: &str = "#version 300 es
precision highp float;
in vec2 v_st;
uniform sampler2D u_tex;
uniform sampler2D u_orig;
uniform vec2 u_texel;
uniform int u_pass;
out vec4 o;
void main(){
  vec4 prev = texture(u_tex, v_st + vec2(float(u_pass + 1) * u_texel.x, 0.0));
  vec4 orig = texture(u_orig, v_st - vec2(0.0, float(u_pass) * u_texel.y));
  if (u_pass == 3) { o = prev * 0.5 + orig * 0.5; } else { o = prev * 0.75 + orig * 0.25; }
}
";

fn orig4() -> DefinitionSnapshot {
    serde_json::from_value(json!({
        "id": "acme.orig4", "kind": "effect", "version": 0, "origin": "local", "renderer": null,
        "glsl": { "source": ORIG4, "bodyLineOffset": 0, "bodyUtf16Len": ORIG4.encode_utf16().count(), "bodyUtf8Len": ORIG4.len() },
        "params": [],
        "flags": { "passes": 4, "keepOrig": true, "backdrop": false, "rawShader": true, "viewportSafe": false, "viewportPadding": [], "clamp": false }
    }))
    .unwrap()
}

fn renderer() -> Renderer {
    let mut set = builtin_definitions();
    set.definitions.push(orig4());
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

/// The shapes of the launch film's 0:11–0:14 in small: many effect leaves,
/// effect groups (group passes), masked and matted effect layers, a
/// non-normal blend, a chain of three effects, a multi-pass `u_orig`
/// effect, and an adjustment layer with a mask over all of it.
fn document() -> pm_model::Document {
    let colors = [
        "#FF6B1A", "#4C8DFF", "#3FCF8E", "#F0C020", "#C04080", "#20E0E0",
    ];
    // Top to bottom.
    let mut layers = Vec::new();
    layers.push(solid("top", "#FFFFFF", 20, 20, 290.0, 150.0));
    let mut adj = layer("adj", "adjustment", json!({}), 0.0, 0.0, DUR);
    adj["fx"] = json!([glow("a1"), fx("a2", "acme.orig4", json!({}))]);
    adj["masks"] = rect_mask(f64::from(W) * 0.4, f64::from(H) / 2.0, 200.0, 140.0, 6.0);
    set(&mut adj, "opacity", json!(85));
    layers.push(adj);
    // A non-normal effect node.
    let mut screen = solid("screen", "#8040FF", 90, 60, 200.0, 20.0);
    screen["blend"] = json!("screen");
    screen["fx"] = json!([glow("s1")]);
    layers.push(screen);
    // A chain of three effects, the last reading `u_orig` in every pass.
    let mut chained = solid("chained", "#F0F0F0", 60, 40, 130.0, 110.0);
    set(&mut chained, "rotation", json!(9));
    chained["fx"] = json!([
        glow("c1"),
        blur("c2", 4.0),
        fx("c3", "acme.orig4", json!({}))
    ]);
    layers.push(chained);
    // A matted effect layer, its matte source just below it.
    let mut matted = solid("matted", "#3FCF8E", 80, 50, 20.0, 120.0);
    matted["matteSource"] = json!("matte");
    matted["matteMode"] = ch(json!("alpha"));
    matted["fx"] = json!([blur("m1", 6.0)]);
    layers.push(matted);
    let mut matte = solid("matte", "#FFFFFF99", 50, 50, 35.0, 120.0);
    set(&mut matte, "rotation", json!(20));
    layers.push(matte);
    // A masked effect layer, and a masked layer without effects.
    let mut masked = solid("masked", "#FF4060", 100, 70, 210.0, 90.0);
    masked["masks"] = rect_mask(50.0, 35.0, 60.0, 50.0, 3.0);
    masked["fx"] = json!([glow("k1")]);
    layers.push(masked);
    let mut plain_mask = solid("plain-mask", "#40A0FF", 70, 50, 250.0, 10.0);
    plain_mask["masks"] = rect_mask(35.0, 25.0, 40.0, 30.0, 2.0);
    layers.push(plain_mask);
    // Two effect groups (compositing boundaries with a group pass each).
    for g in 0..2 {
        let group = format!("g{g}");
        for i in 0..3 {
            let mut child = solid(
                &format!("{group}-{i}"),
                colors[(g * 3 + i) % colors.len()],
                30,
                24,
                12.0 + 40.0 * i as f64 + 150.0 * g as f64,
                64.0 + 6.0 * i as f64,
            );
            set(&mut child, "rotation", json!(7 * i));
            child["group"] = json!(group.clone());
            layers.push(child);
        }
        let mut gl = layer(&group, "group", json!({}), 0.0, 0.0, DUR);
        set(&mut gl, "opacity", json!(80 + 10 * g));
        gl["fx"] = Value::Array(if g == 0 {
            vec![glow("g0f")]
        } else {
            vec![blur("g1f", 3.0), fx("g1o", "acme.orig4", json!({}))]
        });
        layers.push(gl);
    }
    // Many effect leaves.
    for i in 0..12 {
        let mut leaf = solid(
            &format!("leaf{i}"),
            colors[i % colors.len()],
            22,
            16,
            8.0 + 25.0 * i as f64,
            8.0 + 3.0 * (i % 4) as f64,
        );
        set(&mut leaf, "rotation", json!(-5 + i as i64));
        leaf["fx"] = Value::Array(if i % 3 == 0 {
            vec![blur(&format!("l{i}"), 2.0)]
        } else {
            vec![glow(&format!("l{i}"))]
        });
        layers.push(leaf);
    }
    layers.push(solid("back", "#2A2F45", W, H, 0.0, 0.0));
    load(&project(W, H, DUR, solid_fill("#14161C"), layers))
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
    ]
}

fn render(r: &mut Renderer, doc: &pm_model::Document, t: f64, req: &RenderRequest) -> Vec<u32> {
    let frame = r
        .render(doc, "Ptest-root", ticks(t), req, &MapResolver::default())
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    r.read_rgba(&frame)
        .unwrap()
        .into_iter()
        .map(f32::to_bits)
        .collect()
}

fn first_difference(a: &[u32], b: &[u32]) -> Option<(usize, f32, f32)> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .map(|i| (i, f32::from_bits(a[i]), f32::from_bits(b[i])))
}

#[test]
fn targets_given_back_mid_frame_keep_every_bit() {
    let _gpu = gpu();
    let doc = document();
    let mut r = renderer();
    for (what, req) in requests() {
        r.set_early_target_release(false);
        let reference = render(&mut r, &doc, 0.5, &req);
        assert!(
            reference
                .iter()
                .any(|v| f32::from_bits(*v) > 0.1 && f32::from_bits(*v) < 1.0),
            "{what}: the reference draws something"
        );
        r.set_early_target_release(true);
        // A cold frame, then one drawing into targets the last one left.
        for pass in ["first", "second"] {
            let reusing = render(&mut r, &doc, 0.5, &req);
            assert_eq!(reusing.len(), reference.len(), "{what}");
            assert!(
                reusing == reference,
                "{what}, {pass} reusing frame: first difference (index, reusing, reference) {:?}",
                first_difference(&reusing, &reference)
            );
        }
        // The reference again, after the reusing frames filled the pool.
        r.set_early_target_release(false);
        assert!(render(&mut r, &doc, 0.5, &req) == reference, "{what}");
    }
}

#[test]
fn a_frame_draws_into_the_targets_it_gave_back() {
    let _gpu = gpu();
    let doc = document();
    let mut r = renderer();
    // Room for one idle full-frame target, as 48 MiB holds one 2880×1620
    // half-float target: the budget no longer decides reuse within a frame.
    let target = u64::from(W) * u64::from(H) * 8;
    let budget = target + target / 2;
    r.set_target_pool_budget(budget);
    let req = preview();
    let mut created = Vec::new();
    for early in [false, true] {
        r.set_early_target_release(early);
        // Programs, pipelines, and caches settle first.
        render(&mut r, &doc, 0.5, &req);
        render(&mut r, &doc, 0.5, &req);
        let before = r.target_pool_created();
        render(&mut r, &doc, 0.5, &req);
        let made = r.target_pool_created() - before;
        // Idle, the pool is back within its free budget.
        assert!(
            r.target_pool_bytes() <= budget,
            "early {early}: {} idle bytes over {budget}",
            r.target_pool_bytes()
        );
        created.push(made);
    }
    let (held, reused) = (created[0], created[1]);
    eprintln!("targets created per frame: held to the end {held}, given back early {reused}");
    assert!(
        held >= 30,
        "the reference holds every node's targets ({held})"
    );
    assert!(
        reused * 4 <= held,
        "a frame giving targets back creates its working set only: {reused} vs {held}"
    );
}

#[test]
fn depth_and_id_frames_keep_their_images_to_the_end() {
    let _gpu = gpu();
    let doc = document();
    let mut r = renderer();
    let req = preview();
    let color = render(&mut r, &doc, 0.5, &req);
    let mut with_ids = req.clone();
    with_ids.depth_id = true;
    let mut surfaces = Vec::new();
    for early in [true, false] {
        r.set_early_target_release(early);
        let frame = r
            .render(
                &doc,
                "Ptest-root",
                ticks(0.5),
                &with_ids,
                &MapResolver::default(),
            )
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        let ids: Vec<u32> = r
            .read_depth_id(&frame)
            .unwrap()
            .expect("a depth and id surface")
            .into_iter()
            .map(f32::to_bits)
            .collect();
        // The surface reads the layer images after the frame's last pass:
        // it sees each layer's own coverage, never a reused target's.
        assert!(
            ids.iter().any(|v| f32::from_bits(*v) > 0.0),
            "early {early}: some id is drawn"
        );
        let pixels: Vec<u32> = r
            .read_rgba(&frame)
            .unwrap()
            .into_iter()
            .map(f32::to_bits)
            .collect();
        assert!(
            pixels == color,
            "early {early}: the depth and id frame draws the same color, first difference {:?}",
            first_difference(&pixels, &color)
        );
        surfaces.push(ids);
    }
    assert!(
        surfaces[0] == surfaces[1],
        "the surface does not depend on the switch"
    );
}
