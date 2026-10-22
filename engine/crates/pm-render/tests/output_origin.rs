//! The output origin (`RenderRequest::origin`, decision V1b): today's
//! viewer draws a cropped preview viewport through `scaledWorld(L, T, W,
//! H)`, which maps composition pixel `x` to canvas pixel `x · sx − origin`
//! with the crop's unrounded origin, and its background (`backgroundView`)
//! and effects (`effectViewportCoordinates`) follow the same origin. Every
//! drawing path must honour it alike:
//!
//! - an integer origin draws exactly the part of the whole frame it starts
//!   at (the background gradient, effects, masks, a 2.5D layer, a group,
//!   and a precomp included);
//! - a fractional origin is a composition translation: moving every layer
//!   by `d` and the origin by `d · s` draws the same pixels.
//!
//! Snapshots and exports never set an origin (`None`), so they render as
//! before.

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::{MapResolver, RenderRequest, Renderer, builtin_definitions};
use serde_json::{Value, json};
use support::doc::*;
use support::ticks;

const W: u32 = 320;
const H: u32 = 180;
const S: f64 = 2.0;

fn fx(id: &str, ty: &str, p: Value) -> Value {
    json!({ "id": id, "type": ty, "on": true, "p": p })
}

/// Every level 1 layer kind that draws without media or fonts: a rotated
/// solid, a blurred star, a rounded rectangle with a feathered parametric
/// mask, a 2.5D solid, a multiply group, and a precomp, over `background`.
fn document(background: Value, shift: (f64, f64)) -> pm_model::Document {
    let dur = 2.0;
    let at = |mut l: Value, x: f64, y: f64| {
        set(&mut l, "position.x", json!(x + shift.0));
        set(&mut l, "position.y", json!(y + shift.1));
        l
    };
    let mut chip = at(
        layer(
            "chip",
            "solid",
            json!({ "color": "#FF5500", "w": 41, "h": 17 }),
            0.0,
            0.0,
            dur,
        ),
        70.0,
        52.0,
    );
    set(&mut chip, "rotation", json!(9));
    let mut star = at(
        layer(
            "star",
            "shape",
            json!({ "shape": "star", "w": 60, "h": 60, "color": "#55CCFF" }),
            0.0,
            0.0,
            dur,
        ),
        150.0,
        70.0,
    );
    star["fx"] = json!([fx("f1", "blur", json!({ "amount": ch(json!(5)) }))]);
    let mut rect = at(
        layer(
            "rect",
            "shape",
            json!({ "shape": "rect", "w": 70, "h": 44, "radius": 9, "color": "#E8E83A" }),
            0.0,
            0.0,
            dur,
        ),
        240.0,
        60.0,
    );
    rect["masks"] = json!([{ "id": "k", "shape": "ellipse", "mode": "add", "on": true,
        "p": { "x": ch(json!(35)), "y": ch(json!(22)), "w": ch(json!(56)), "h": ch(json!(34)), "rotation": ch(json!(12)), "feather": ch(json!(3)) } }]);
    let mut plane = at(
        layer(
            "plane",
            "solid",
            json!({ "color": "#7CFFB2", "w": 50, "h": 30 }),
            0.0,
            0.0,
            dur,
        ),
        80.0,
        130.0,
    );
    plane["threeD"] = json!(true);
    set(&mut plane, "rotation.y", json!(35));
    set(&mut plane, "rotation.x", json!(-20));
    let mut a = layer(
        "a",
        "solid",
        json!({ "color": "#FF00AA", "w": 30, "h": 22 }),
        0.0,
        0.0,
        dur,
    );
    a["group"] = json!("g");
    let mut b = layer(
        "b",
        "solid",
        json!({ "color": "#4C8DFFC0", "w": 26, "h": 30 }),
        14.0,
        9.0,
        dur,
    );
    b["group"] = json!("g");
    let mut g = at(layer("g", "group", json!({}), 0.0, 0.0, dur), 170.0, 125.0);
    g["blend"] = json!("multiply");
    set(&mut g, "opacity", json!(80));
    let inner = layer(
        "in",
        "solid",
        json!({ "color": "#FFFFFF", "w": 33, "h": 21 }),
        30.0,
        20.0,
        dur,
    );
    let pre = at(
        layer(
            "pre",
            "precomp",
            json!({ "comp": "inner", "w": 60, "h": 40 }),
            0.0,
            0.0,
            dur,
        ),
        265.0,
        140.0,
    );
    let mut doc = project(
        W,
        H,
        dur,
        background,
        vec![pre, g, a, b, plane, rect, star, chip],
    );
    composition(&mut doc, "inner", 60, 40, dur, vec![inner]);
    load(&doc)
}

/// A horizontal gradient (angle 0): its value depends on x only.
fn gradient() -> Value {
    gradient_at(0.0, "#203040", "#C07020")
}

fn gradient_at(angle: f64, from: &str, to: &str) -> Value {
    json!({ "type": "linear", "angle": angle, "stops": [
        { "id": "s1", "color": from, "position": 0 },
        { "id": "s2", "color": to, "position": 100 }
    ] })
}

fn renderer() -> Renderer {
    let mut r = Renderer::headless().unwrap();
    assert!(
        r.set_definitions(&builtin_definitions())
            .unwrap()
            .is_empty()
    );
    r
}

fn render(r: &mut Renderer, doc: &pm_model::Document, req: &RenderRequest) -> Vec<f32> {
    let frame = r
        .render(doc, "Ptest-root", ticks(0.5), req, &MapResolver::default())
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    r.read_rgba(&frame).unwrap()
}

/// A canvas of `w` × `h` at scale `S` starting at `origin`.
fn canvas(w: u32, h: u32, origin: (f64, f64)) -> RenderRequest {
    RenderRequest {
        scale: Some((S, S)),
        origin: Some(origin),
        ..RenderRequest::sized(Size::new(w, h), Precision::Float32)
    }
}

/// The largest 8-bit difference between `a` (`aw` wide, read at `(ax, ay)`)
/// and `b` (`bw` × `bh`), over `b` less a `margin` on every side, and the
/// pixels it covers.
#[allow(clippy::too_many_arguments)]
fn diff(
    a: &[f32],
    aw: u32,
    (ax, ay): (u32, u32),
    b: &[f32],
    bw: u32,
    bh: u32,
    margin: u32,
) -> (u8, usize) {
    let (mut max, mut n) = (0u8, 0);
    for y in margin..bh - margin {
        for x in margin..bw - margin {
            let i = ((y + ay) * aw + x + ax) as usize * 4;
            let j = (y * bw + x) as usize * 4;
            for c in 0..4 {
                let d = (a[i + c] * 255.0).round() - (b[j + c] * 255.0).round();
                max = max.max(d.abs() as u8);
            }
            n += 1;
        }
    }
    (max, n)
}

/// A whole-pixel origin draws the part of the whole frame it starts at
/// (away from the canvas edges, where today's canvas also cuts effects).
#[test]
fn an_integer_origin_draws_the_part_of_the_whole_frame_it_starts_at() {
    let mut r = renderer();
    let doc = document(gradient(), (0.0, 0.0));
    let full = render(
        &mut r,
        &doc,
        &RenderRequest::at_scale(Size::new(W, H), S, Precision::Float32),
    );
    let fw = W * 2;
    for (ox, oy, w, h) in [
        (100u32, 60u32, 340u32, 220u32),
        (0, 0, 640, 360),
        (37, 121, 410, 201),
    ] {
        let part = render(&mut r, &doc, &canvas(w, h, (f64::from(ox), f64::from(oy))));
        let (max, n) = diff(&full, fw, (ox, oy), &part, w, h, 24);
        assert!(n > 10_000, "{n}");
        assert!(max <= 1, "origin ({ox}, {oy}): {max} CV over {n} pixels");
    }
    // `None` is the origin.
    let none = render(
        &mut r,
        &doc,
        &RenderRequest {
            scale: Some((S, S)),
            ..RenderRequest::sized(Size::new(640, 360), Precision::Float32)
        },
    );
    let zero = render(&mut r, &doc, &canvas(640, 360, (0.0, 0.0)));
    assert_eq!(none, zero, "an origin of (0, 0) is no origin");
}

/// A fractional origin moves every layer as a composition translation does:
/// the layers moved by `d` drawn from the origin moved by `d · s` are the
/// layers drawn from the origin.
#[test]
fn a_fractional_origin_moves_every_layer_like_a_composition_translation() {
    let mut r = renderer();
    let base = document(solid_fill("#203040"), (0.0, 0.0));
    let reference = render(&mut r, &base, &canvas(340, 220, (100.0, 60.0)));
    for d in [(0.15, 0.35), (0.25, 0.0), (0.4, 0.45)] {
        let moved = document(solid_fill("#203040"), d);
        let shifted = render(
            &mut r,
            &moved,
            &canvas(340, 220, (100.0 + d.0 * S, 60.0 + d.1 * S)),
        );
        let (max, n) = diff(&reference, 340, (0, 0), &shifted, 340, 220, 24);
        assert!(max <= 1, "{d:?}: {max} CV over {n} pixels");
        // Without the origin's fraction the same layers land elsewhere.
        let unshifted = render(&mut r, &moved, &canvas(340, 220, (100.0, 60.0)));
        let (off, _) = diff(&reference, 340, (0, 0), &unshifted, 340, 220, 24);
        assert!(off > 8, "{d:?}: the move must be visible ({off})");
    }
}

/// An origin is part of the draw-reuse key: a view that pans keeps no draw
/// placed for the previous origin.
#[test]
fn draw_reuse_follows_the_origin() {
    let mut r = renderer();
    r.set_draw_reuse(true);
    let doc = document(gradient(), (0.0, 0.0));
    let _ = render(&mut r, &doc, &canvas(340, 220, (100.0, 60.0)));
    let panned = render(&mut r, &doc, &canvas(340, 220, (100.6, 61.3)));
    r.set_draw_reuse(false);
    let fresh = render(&mut r, &doc, &canvas(340, 220, (100.6, 61.3)));
    assert_eq!(panned, fresh);
}

/// Today's background with a preview viewport (`backgroundView`): `p =
/// u_view.xy + v_st · u_view.zw − 0.5` with `u_view.y = viewport.y /
/// comp.h` from the top while GL's `v_st.y` runs up from the canvas's
/// bottom. A vertical gradient (angle 90, black to white) at a crop shows
/// `smoothstep(0, 1, t)`, `t = (oy + y from the canvas bottom) / (comp.h ·
/// sy)`, as today's does (not the crop of the whole frame's gradient).
#[test]
fn the_background_follows_todays_view_at_an_origin() {
    let mut r = renderer();
    let mut doc = project(W, H, 2.0, gradient_at(90.0, "#000000", "#FFFFFF"), vec![]);
    doc["layers"] = json!([]);
    let doc = load(&doc);
    let (w, h, oy) = (200u32, 120u32, 70.4);
    let px = render(&mut r, &doc, &canvas(w, h, (31.7, oy)));
    for y in [0u32, 37, 119] {
        let from_bottom = f64::from(h - y) - 0.5;
        let t = ((oy + from_bottom) / (f64::from(H) * S)).clamp(0.0, 1.0);
        let want = t * t * (3.0 - 2.0 * t);
        let got = f64::from(px[((y * w + 50) * 4) as usize]);
        assert!(
            (got - want).abs() < 1.0 / 255.0,
            "row {y}: {got} against {want}"
        );
    }
}
