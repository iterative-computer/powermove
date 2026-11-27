//! Depth sets and the ordering of 3D runs against today's TypeScript (plan
//! section 8): `tests/fixtures/scene3d-layering.json` holds random
//! compositions of 3D object layers, 2D solids, 3D nulls, lights, cameras,
//! plain groups, inactive and invisible layers, and solo, with the pass
//! order (`compositionOrderedLayers`), every active object's depth set
//! (`compositionDepthIds`), and whether it is the bottom of its set (it
//! draws the scene background) as `core/scene3d/service.ts` computes them.
//! Regenerate with `bun engine/crates/pm-render/tools/layering/generate.ts`.
//!
//! Lights and cameras are left out of the order comparison: they draw
//! nothing and never end a depth set, and the engine sorts them at depth 0
//! where today sorts them by their world position (docs/engine/scene3d.md).

mod support;

use std::collections::{BTreeMap, BTreeSet};

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::plan::{Content, Source};
use pm_render::scene3d::Render3d;
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::Value;
use support::doc::load;
use support::ticks;

/// A layer as its template plus `[path, value]` overrides.
fn expand(templates: &Value, spec: &Value) -> Value {
    let mut layer = templates[spec["kind"].as_str().unwrap()].clone();
    for entry in spec["set"].as_array().unwrap() {
        let path: Vec<&str> = entry[0]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap())
            .collect();
        let mut cur = &mut layer;
        for k in &path[..path.len() - 1] {
            cur = &mut cur[*k];
        }
        cur[*path.last().unwrap()] = entry[1].clone();
    }
    layer
}

#[test]
fn depth_sets_and_ordering_equal_today() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/scene3d-layering.json")).unwrap();
    let templates = &fixture["templates"];
    let mut r = Renderer::headless().unwrap();
    let request = RenderRequest::at_scale(Size::new(320, 180), 1.0, Precision::Float16);
    let (mut objects, mut backgrounds, mut ordered_pairs) = (0, 0, 0);
    let mut problems = Vec::new();
    for (n, case) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let mut doc = fixture["project"].clone();
        doc["layers"] = Value::Array(
            case["layers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| expand(templates, s))
                .collect(),
        );
        let roles: BTreeMap<String, String> = doc["layers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| {
                (
                    l["id"].as_str().unwrap().to_owned(),
                    l["d"]["definition"].as_str().unwrap_or("").to_owned(),
                )
            })
            .collect();
        let time = case["time"].as_f64().unwrap();
        let (plan, failures) = r
            .plan(
                &load(&doc),
                "Ptest-root",
                ticks(time),
                &request,
                &MapResolver::default(),
            )
            .unwrap();
        assert!(failures.is_empty(), "case {n}: {failures:?}");
        // The engine's draws, top first, and each 3D object's set.
        let mut drawn: Vec<String> = Vec::new();
        for d in plan.draws.iter().rev() {
            if !drawn.contains(&d.layer) {
                drawn.push(d.layer.clone());
            }
            let Content::Texture {
                source: Source::Object3d(o),
                ..
            } = &d.content
            else {
                continue;
            };
            let Render3d::Layer {
                depth_set,
                background,
                ..
            } = &o.kind
            else {
                problems.push(format!(
                    "case {n}: {} is not a composition 3D layer",
                    o.layer
                ));
                continue;
            };
            let set: BTreeSet<&str> = depth_set
                .iter()
                .map(|&i| o.scene.objects[i].id.as_str())
                .collect();
            let want = &case["sets"][o.layer.as_str()];
            if want.is_null() {
                problems.push(format!(
                    "case {n}: {} draws but is not active today",
                    o.layer
                ));
                continue;
            }
            let want_set: BTreeSet<&str> = want["set"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            if set != want_set {
                problems.push(format!(
                    "case {n}: {} depth set {set:?}, today {want_set:?}",
                    o.layer
                ));
            }
            if *background != want["background"].as_bool().unwrap() {
                problems.push(format!(
                    "case {n}: {} draws the background: {background}, today {}",
                    o.layer, want["background"]
                ));
            }
            objects += 1;
            backgrounds += usize::from(*background);
        }
        // The order of drawn layers that are not lights or cameras.
        let compared = |id: &String| {
            !matches!(
                roles[id].as_str(),
                "powermove.3d.light" | "powermove.3d.camera"
            )
        };
        let ours: Vec<&String> = drawn.iter().filter(|id| compared(id)).collect();
        let today: Vec<String> = case["ordered"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .filter(|id| ours.contains(&id))
            .collect();
        let ours: Vec<String> = ours.into_iter().cloned().collect();
        if ours != today {
            problems.push(format!("case {n}: order {ours:?}, today {today:?}"));
        }
        ordered_pairs += ours.len().saturating_sub(1);
    }
    eprintln!(
        "{objects} 3D object draws compared ({backgrounds} set bottoms), {ordered_pairs} adjacent draw pairs ordered"
    );
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    assert!(objects > 100 && backgrounds > 20);
}
