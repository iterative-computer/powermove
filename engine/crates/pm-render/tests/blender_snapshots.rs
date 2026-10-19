//! Section 25's "Blender snapshots" class on the parity corpus: for every
//! 3D corpus frame with Blender on (final and preview, the exporter's
//! opaque and alpha passes, every shutter sample, nested compositions), the
//! snapshot `pm_render::blender` builds equals today's `renderSnapshot`
//! output — geometry within 1e-6; materials, maps, blocks, the background
//! list, camera, lights, settings, and environment identical; asset bytes
//! equal (SHA-256) — and the frames `blenderFrameRequests` lists equal the
//! engine's (`blender::frames`): composition, time bit for bit, and size.
//!
//! The capture turned `render3d` on in memory for every composition with 3D
//! objects (`compositions`, with `settings`); this test does the same to
//! the loaded document.
//!
//! Allowed differences (appendix A), each counted and listed:
//! - Projects with imported glTF models: the glTF lane's loader builds
//!   their geometry; until it lands, their snapshots are not built (routed,
//!   `imported models`) and are listed, not compared. Their source
//!   materials follow amendment 14's fix (`sourceMaterialSnapshot`), which
//!   corpus v1 already records; v0 recorded the throw instead.
//!
//! `PM_ENGINE_CORPUS` picks the capture (default: the newest `v1-*`);
//! `PM_BLENDER_ONLY` limits to project names with a prefix.

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use pm_blender::compare::{Comparison, compare};
use pm_blender::snapshot::BlenderFrame;
use pm_eval::{Comp, Evaluator};
use pm_model::Value;
use pm_model::json::{JsString, Map};
use pm_render::blender::frames::{FrameOptions, frame_requests};
use pm_render::blender::snapshot::{SnapshotOptions, build};
use pm_render::expr::PmExprEngine;
use support::*;

const PASSES: [&str; 4] = ["final", "preview", "export", "exportAlpha"];

fn get<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.get(k)
}

fn s(v: Option<&Value>) -> String {
    v.and_then(Value::as_str).unwrap_or("").to_owned()
}

fn n(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64)
}

/// `loaded.json` with `render3d = settings` on `compositions`.
fn with_blender(loaded: &Value, compositions: &[String], settings: &Value) -> Value {
    let mut root = loaded.clone();
    let open = s(get(&root, "compId")).to_owned();
    let open = if open.is_empty() {
        s(get(&root, "id"))
    } else {
        open
    };
    let Value::Object(map) = &mut root else {
        panic!("root")
    };
    for c in compositions {
        if *c == open && map.get("layers").is_some() {
            map.insert("render3d", settings.clone());
        } else if let Some(Value::Object(comps)) = map.get_mut("comps")
            && let Some(Value::Object(comp)) = comps.get_mut(c)
        {
            comp.insert("render3d", settings.clone());
        }
    }
    root
}

#[derive(Default)]
struct Totals {
    snapshots: usize,
    equal: usize,
    geometry_values: usize,
    geometry_max: f64,
    exact_values: usize,
    frame_lists: usize,
    routed: BTreeMap<String, String>,
    failed: Vec<String>,
    /// Snapshots whose converted PNG textures equal today's by pixels.
    converted: usize,
}

/// A PNG's 8-bit RGBA pixels (`None` when it does not decode to that).
fn png_pixels(bytes: &[u8]) -> Option<Option<Vec<u8>>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    Some((info.bit_depth == png::BitDepth::Eight && info.color_type == png::ColorType::Rgba).then_some(buf))
}

#[test]
fn snapshots_equal_todays_render_snapshot() {
    let capture = match std::env::var("PM_ENGINE_CORPUS") {
        Ok(p) => Some(std::path::PathBuf::from(p.split(',').next().unwrap())),
        Err(_) => newest_capture("v1-"),
    };
    let Some(capture) = capture else {
        eprintln!("no parity corpus capture found; set PM_ENGINE_CORPUS to run this comparison");
        return;
    };
    let only = std::env::var("PM_BLENDER_ONLY").ok();
    let mut t = Totals::default();
    for dir in projects(&capture) {
        let blender = dir.join("blender");
        if !blender.is_dir() {
            continue;
        }
        let p = load_project(&capture, &dir);
        if only.as_deref().is_some_and(|o| !p.name.starts_with(o)) {
            continue;
        }
        let mut files: Vec<_> = std::fs::read_dir(&blender)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        let loaded = read_json(&dir.join("loaded.json"));
        for file in files {
            let record = read_json(&file);
            assert_eq!(s(get(&record, "format")), "powermove-corpus-blender");
            let compositions: Vec<String> = get(&record, "compositions")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(|c| s(Some(c))).collect())
                .unwrap_or_default();
            let settings = get(&record, "settings").cloned().unwrap();
            let doc =
                pm_model::Document::new(Some(with_blender(&loaded, &compositions, &settings)), 1);
            let time = n(get(&record, "time")).unwrap();
            let mut ev = Evaluator::new().with_expressions(Arc::new(PmExprEngine));
            let root = Comp::resolve(&doc, &p.composition).unwrap();
            for pass in PASSES {
                let Some(entry) = get(&record, pass) else {
                    continue;
                };
                if let Some(e) = get(entry, "error") {
                    t.failed.push(format!(
                        "{}/{pass}: the capture recorded an error: {}",
                        p.name,
                        s(Some(e))
                    ));
                    continue;
                }
                let snapshots = get(entry, "snapshots")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                // Frames: the engine's walk lists the same frames.
                if pass != "preview" {
                    let opts = get(entry, "options");
                    let options = FrameOptions {
                        width: n(opts.and_then(|o| get(o, "width"))).map(|v| v as u32),
                        height: n(opts.and_then(|o| get(o, "height"))).map(|v| v as u32),
                        motion_blur: !matches!(
                            opts.and_then(|o| get(o, "mblur")),
                            Some(Value::Bool(false))
                        ),
                        samples: n(opts.and_then(|o| get(o, "mbSamples"))).map_or(10, |v| v as u32),
                        shutter: n(opts.and_then(|o| get(o, "shutter"))),
                    };
                    let frames = frame_requests(&mut ev, &root, &root, time, options)
                        .unwrap()
                        .unwrap();
                    let want: Vec<(String, u64, u32, u32)> = snapshots
                        .iter()
                        .map(|x| {
                            let r = get(x, "request").unwrap();
                            (
                                s(get(r, "comp")),
                                n(get(r, "time")).unwrap().to_bits(),
                                n(get(r, "width")).unwrap() as u32,
                                n(get(r, "height")).unwrap() as u32,
                            )
                        })
                        .collect();
                    let got: Vec<(String, u64, u32, u32)> = frames
                        .iter()
                        .map(|f| {
                            (
                                f.composition().to_owned(),
                                f.time.to_bits(),
                                f.width,
                                f.height,
                            )
                        })
                        .collect();
                    if got == want {
                        t.frame_lists += 1;
                    } else {
                        t.failed.push(format!(
                            "{}/{pass} t={time}: frames differ: today {want:?}, engine {got:?}",
                            p.name
                        ));
                    }
                }
                for x in &snapshots {
                    let r = get(x, "request").unwrap();
                    let comp_id = s(get(r, "comp"));
                    let comp = Comp::resolve(&doc, &comp_id).unwrap();
                    let preview = matches!(get(r, "preview"), Some(Value::Bool(true)));
                    let w = n(get(r, "width"))
                        .unwrap_or_else(|| pm_eval::node::to_number(comp.member("w")));
                    let h = n(get(r, "height"))
                        .unwrap_or_else(|| pm_eval::node::to_number(comp.member("h")));
                    let frame = BlenderFrame {
                        composition_path: vec![comp_id.clone()],
                        time: n(get(r, "time")).unwrap(),
                        width: w as u32,
                        height: h as u32,
                        shutter: None,
                        preview,
                    };
                    let label = format!("{}/{pass} {comp_id} t={}", p.name, frame.time);
                    t.snapshots += 1;
                    let mut scope = pm_eval::Scope::new(&mut ev, &comp, &root);
                    let built = build(&mut scope, &frame, &p.resolver, &SnapshotOptions::default())
                        .unwrap();
                    let snapshot = match built {
                        Ok(s) => s,
                        Err(why) if why == "imported models" => {
                            t.routed.insert(p.name.clone(), why);
                            continue;
                        }
                        Err(why) => {
                            t.failed.push(format!("{label}: not built: {why}"));
                            continue;
                        }
                    };
                    let mut today = get(x, "snapshot").unwrap().clone();
                    // All captured fields remain compared. Add the new output
                    // policy from the original layer compatibility, independently
                    // of the snapshot builder's metadata.
                    let compatible: Vec<Value> = get(&today, "objects")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|object| {
                            let id = s(get(object, "id"));
                            let layer = scope.layers().iter().find(|l| {
                                pm_eval::node::string(pm_eval::node::member(Some(l), "id"))
                                    .is_some_and(|name| name.to_string_lossy() == id)
                            });
                            let level2 = layer.is_some_and(|l| {
                                pm_eval::node::to_number(pm_eval::node::member(Some(l), "compat"))
                                    >= 2.0
                            });
                            (!level2).then(|| Value::String(id.into()))
                        })
                        .collect();
                    if !compatible.is_empty() {
                        today
                            .as_object_mut()
                            .unwrap()
                            .insert("compatibilityLayers", Value::Array(compatible));
                    }
                    let c: Comparison = compare(&today, &snapshot.json);
                    // Assets: id, name, and bytes.
                    let want: Vec<(String, String, String)> = get(x, "assets")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(|a| (s(get(a, "id")), s(get(a, "name")), s(get(a, "sha256"))))
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    let got: Vec<(String, String, String)> = snapshot
                        .assets
                        .iter()
                        .map(|a| {
                            (
                                a.id.clone(),
                                a.name.clone(),
                                a.sha256.iter().map(|b| format!("{b:02x}")).collect(),
                            )
                        })
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    t.geometry_values += c.geometry_values;
                    t.exact_values += c.exact_values;
                    t.geometry_max = t.geometry_max.max(c.geometry_max);
                    // Converted images (`<id>.png` today made by a canvas's
                    // `toBlob`): the PNG container bytes are each encoder's
                    // own, so they are compared by decoded pixels, which is
                    // what Blender reads.
                    let converted_equal = want.len() == got.len()
                        && want.iter().zip(&got).all(|(w, g)| {
                            w == g
                                || (w.0 == g.0
                                    && w.1 == g.1
                                    && w.1.ends_with(".png")
                                    && snapshot.assets.iter().any(|a| {
                                        a.id == g.0
                                            && matches!(a.bytes, pm_blender::snapshot::AssetBytes::Bytes(_))
                                            && png_pixels(&a.bytes.read().unwrap())
                                                == std::fs::read(blender.join("assets").join(format!("{}.png", w.2)))
                                                    .ok()
                                                    .and_then(|b| png_pixels(&b))
                                    }))
                        });
                    if converted_equal && want != got {
                        t.converted += 1;
                    }
                    if c.equal() && (want == got || converted_equal) {
                        t.equal += 1;
                    } else {
                        let mut lines = vec![format!("{label}:")];
                        for d in c.differences.iter().take(12) {
                            lines.push(format!(
                                "  {}: today {} engine {}",
                                d.path, d.today, d.engine
                            ));
                        }
                        if want != got {
                            for (w, g) in want.iter().zip(&got).filter(|(w, g)| w != g) {
                                let ours = snapshot.assets.iter().find(|a| a.id == g.0).and_then(|a| png_pixels(&a.bytes.read().unwrap()));
                                let today = std::fs::read(blender.join("assets").join(format!("{}.png", w.2))).ok().and_then(|b| png_pixels(&b));
                                let diff = match (&ours, &today) {
                                    (Some(Some(a)), Some(Some(b))) if a.len() == b.len() => {
                                        let d = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
                                        let n = a.chunks(4).zip(b.chunks(4)).filter(|(x, y)| x != y).count();
                                        format!("max {d}, {n} pixels differ")
                                    }
                                    _ => format!("ours {:?} today {:?}", ours.as_ref().map(|o| o.as_ref().map(Vec::len)), today.as_ref().map(|o| o.as_ref().map(Vec::len))),
                                };
                                lines.push(format!("  asset {} {}: today {} engine {} ({diff})", w.0, w.1, w.2, g.2));
                            }
                        }
                        t.failed.push(lines.join("\n"));
                    }
                }
            }
        }
    }
    eprintln!(
        "Blender snapshots: {} compared, {} equal ({} with converted textures equal by pixels); {} frame lists equal; geometry {} values, max |Δ| {:e}; {} exact values",
        t.snapshots, t.equal, t.converted, t.frame_lists, t.geometry_values, t.geometry_max, t.exact_values
    );
    for (name, why) in &t.routed {
        eprintln!("not built (allowed until the glTF lane lands): {name}: {why}");
    }
    assert!(
        t.failed.is_empty(),
        "{} failures:\n{}",
        t.failed.len(),
        t.failed.join("\n")
    );
    if only.is_none() {
        assert!(t.equal > 0, "nothing compared");
    }
    let _ = (JsString::new(), Map::new());
}
