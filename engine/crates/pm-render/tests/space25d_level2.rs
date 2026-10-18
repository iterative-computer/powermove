//! Level 2 content on 2.5D planes (docs/engine/motion.md, "2.5D"):
//! solids, shapes, and editable paths at `compat` level 2 with the 3D
//! switch rasterize in layer space at the plane's density and draw through
//! the plane's homography with the level 2 resampler.
//!
//! Level 2 has no legacy reference (new content). The checks: a flat plane
//! (the 3D switch with no 3D transform) draws what the same layer draws in
//! 2D at output resolution; a turned plane covers what its homography
//! covers (interior opaque, exterior empty, partial coverage only at the
//! projected edges); and the content is never drawn flat, ignoring the
//! plane.

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::{RenderRequest, Renderer};
use serde_json::{Value, json};
use support::doc::*;
use support::*;

const W: u32 = 240;
const H: u32 = 160;

struct NoMedia;

impl pm_render::AssetResolver for NoMedia {
    fn path(&self, _: &str, _: Option<&pm_model::Node>) -> Option<std::path::PathBuf> {
        None
    }
}

fn render(renderer: &mut Renderer, layers: Vec<Value>) -> Vec<f32> {
    let doc = load(&project(W, H, 2.0, no_fill(), layers));
    assert!(
        pm_render::unsupported(&doc, "Ptest-root").is_empty(),
        "{:?}",
        pm_render::unsupported(&doc, "Ptest-root")
    );
    let mut request = RenderRequest::sized(Size::new(W, H), Precision::Float32);
    request.background = false;
    let f = renderer
        .render(&doc, "Ptest-root", ticks(0.5), &request, &NoMedia)
        .unwrap();
    assert!(f.failures.is_empty(), "{:?}", f.failures);
    renderer.read_rgba(&f).unwrap()
}

fn level2(mut l: Value, three_d: bool) -> Value {
    l["compat"] = json!(2);
    l["threeD"] = json!(three_d);
    l
}

fn max_alpha_diff(a: &[f32], b: &[f32]) -> f32 {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .map(|(x, y)| (x[3] - y[3]).abs())
        .fold(0.0, f32::max)
}

fn solid() -> Value {
    let mut l = layer(
        "s",
        "solid",
        json!({ "color": "#FFFFFF", "w": 90.0, "h": 60.0, "radius": 14.0 }),
        75.0,
        50.0,
        2.0,
    );
    set(
        &mut l,
        "rotation",
        json!({ "v": 8.0, "kf": [], "expr": null }),
    );
    l
}

fn shape() -> Value {
    layer(
        "sh",
        "shape",
        json!({ "shape": "star", "w": 80.0, "h": 80.0, "points": 6, "color": "#FFFFFF" }),
        120.0,
        80.0,
        2.0,
    )
}

#[test]
fn a_flat_plane_draws_what_2d_draws() {
    let mut renderer = Renderer::headless().unwrap();
    for l in [solid(), shape()] {
        let flat = render(&mut renderer, vec![level2(l.clone(), false)]);
        let plane = render(&mut renderer, vec![level2(l, true)]);
        // The projected outline on a flat plane is the 2D outline: the same
        // analytic coverage, to the flattening tolerance.
        let d = max_alpha_diff(&flat, &plane);
        assert!(d < 0.005, "flat plane differs from 2D by {d}");
        let covered = |f: &[f32]| f.chunks_exact(4).map(|p| f64::from(p[3])).sum::<f64>();
        let (a, b) = (covered(&flat), covered(&plane));
        assert!((a - b).abs() / a < 1e-4, "coverage {a} against {b}");
    }
}

/// The inverse of a column-major plane at an output pixel center, in unit
/// quad coordinates.
fn unproject(p: &[f64; 9], x: f64, y: f64) -> Option<(f64, f64)> {
    // Solve [p0 p3 p6; p1 p4 p7; p2 p5 p8] (u, v, 1) ~ (x, y, 1).
    let a = [
        [p[0] - x * p[2], p[3] - x * p[5]],
        [p[1] - y * p[2], p[4] - y * p[5]],
    ];
    let b = [x * p[8] - p[6], y * p[8] - p[7]];
    let det = a[0][0] * a[1][1] - a[0][1] * a[1][0];
    if det.abs() < 1e-12 {
        return None;
    }
    let u = (b[0] * a[1][1] - a[0][1] * b[1]) / det;
    let v = (a[0][0] * b[1] - b[0] * a[1][0]) / det;
    Some((u, v))
}

#[test]
fn a_turned_plane_covers_its_homography() {
    let mut renderer = Renderer::headless().unwrap();
    // A level 2 solid without radius turned about Y: its quad's
    // homography is exactly the level 1 solid's (a single-sample quad).
    let mut l = layer(
        "s",
        "solid",
        json!({ "color": "#FFFFFF", "w": 100.0, "h": 70.0 }),
        70.0,
        45.0,
        2.0,
    );
    set(
        &mut l,
        "rotation.y",
        json!({ "v": 50.0, "kf": [], "expr": null }),
    );
    set(
        &mut l,
        "anchor.x",
        json!({ "v": 50.0, "kf": [], "expr": null }),
    );
    set(
        &mut l,
        "anchor.y",
        json!({ "v": 35.0, "kf": [], "expr": null }),
    );
    let ours = render(&mut renderer, vec![level2(l.clone(), true)]);
    let mut level1 = l.clone();
    level1["threeD"] = json!(true);
    let reference = render(&mut renderer, vec![level1]);
    // The level 1 quad: fully covered pixels (whose 4 corners are inside)
    // must be opaque at level 2, and pixels the quad does not touch empty.
    let mut interior = 0;
    let mut outside = 0;
    for y in 1..H as usize - 1 {
        for x in 1..W as usize - 1 {
            let at = |dx: isize, dy: isize| {
                reference
                    [((y as isize + dy) as usize * W as usize + (x as isize + dx) as usize) * 4 + 3]
            };
            let block: Vec<f32> = (-1..=1)
                .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
                .map(|(dx, dy)| at(dx, dy))
                .collect();
            let a = ours[(y * W as usize + x) * 4 + 3];
            if block.iter().all(|&v| v == 1.0) {
                interior += 1;
                assert!((a - 1.0).abs() < 1e-3, "interior ({x},{y}) {a}");
            } else if block.iter().all(|&v| v == 0.0) {
                outside += 1;
                assert!(a.abs() < 1e-3, "outside ({x},{y}) {a}");
            }
        }
    }
    assert!(interior > 2000 && outside > 10000, "{interior} {outside}");
    // Not drawn flat: the flat (2D) solid covers pixels the plane does not.
    let mut flat = l;
    flat["compat"] = json!(2);
    let flat = render(&mut renderer, vec![flat]);
    assert!(max_alpha_diff(&flat, &ours) > 0.9);
    let _ = unproject;
}

#[test]
fn level2_paths_and_text_free_layers_on_planes_are_supported() {
    // Editable paths at level 2 with the 3D switch.
    let mut renderer = Renderer::headless().unwrap();
    let mut l = layer(
        "p",
        "shape",
        json!({ "shape": "rect", "w": 200.0, "h": 200.0, "color": "#FFFFFF", "paths": [{
            "id": "path1", "name": "Tri", "parent": null,
            "p": {
                "x": ch(json!(0)), "y": ch(json!(0)), "rotation": ch(json!(0)),
                "scaleX": ch(json!(100)), "scaleY": ch(json!(100)), "closed": ch(json!(true)),
                "fill": ch(json!("#FFFFFF")), "fillEnabled": ch(json!(true)), "fillOpacity": ch(json!(100)),
                "stroke": ch(json!("#FFFFFF")), "strokeWidth": ch(json!(0)), "strokeOpacity": ch(json!(100)),
                "trimStart": ch(json!(0)), "trimEnd": ch(json!(100)), "trimOffset": ch(json!(0)),
                "copies": ch(json!(1)), "repeatX": ch(json!(30)), "repeatY": ch(json!(0)), "repeatRotation": ch(json!(0))
            },
            "vertices": [
                { "id": "v1", "p": { "x": ch(json!(-40)), "y": ch(json!(-30)), "inX": ch(json!(0)), "inY": ch(json!(0)), "outX": ch(json!(0)), "outY": ch(json!(0)) } },
                { "id": "v2", "p": { "x": ch(json!(40)), "y": ch(json!(-30)), "inX": ch(json!(0)), "inY": ch(json!(0)), "outX": ch(json!(0)), "outY": ch(json!(0)) } },
                { "id": "v3", "p": { "x": ch(json!(0)), "y": ch(json!(40)), "inX": ch(json!(0)), "inY": ch(json!(0)), "outX": ch(json!(0)), "outY": ch(json!(0)) } }
            ]
        }] }),
        120.0,
        80.0,
        2.0,
    );
    set(
        &mut l,
        "rotation.x",
        json!({ "v": 35.0, "kf": [], "expr": null }),
    );
    let plane = render(&mut renderer, vec![level2(l.clone(), true)]);
    let flat = render(&mut renderer, vec![level2(l, false)]);
    let covered = |f: &[f32]| f.chunks_exact(4).map(|p| f64::from(p[3])).sum::<f64>();
    // Turned about X the triangle is foreshortened: it covers less than flat.
    let (p, f) = (covered(&plane), covered(&flat));
    assert!(p > 0.5 * f && p < 0.95 * f, "{p} {f}");
}
