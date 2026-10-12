//! Native pick benchmark and parity check against the three.js references
//! (`bun ref/picks.ts` → out/picks/<scene>.json).
//!
//!   s7-picks <spike-root> [scene ...]
use anyhow::{Context, Result, bail};
use glam::DMat4;
use s7_geom3d::scene::{Camera, Layer, Prepared, Scene};
use s7_geom3d::wasm::now_ms;
use s7_geom3d::{LoadOptions, load};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

fn mat(v: &Value) -> Result<DMat4> {
    let a: Vec<f64> = v.as_array().context("matrix")?.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect();
    if a.len() != 16 {
        bail!("matrix length");
    }
    Ok(DMat4::from_cols_slice(&a))
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(args.first().context("spike root")?);
    let picks_dir = root.join("out/picks");
    let mut scenes: Vec<String> = args[1..].to_vec();
    if scenes.is_empty() {
        let mut all: Vec<String> = std::fs::read_dir(&picks_dir)?.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".json") && !n.starts_with("native") && !n.starts_with("wasm")).map(|n| n.trim_end_matches(".json").to_string()).collect();
        all.sort();
        scenes = all;
    }
    let mut report = Vec::new();
    for id in &scenes {
        let reference: Value = serde_json::from_slice(&std::fs::read(picks_dir.join(format!("{id}.json")))?)?;
        let def: Value = serde_json::from_slice(&std::fs::read(root.join(format!("out/corpus/scenes/{id}.json")))?)?;
        let ids: Vec<String> = reference["ids"].as_array().context("ids")?.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect();
        let mut scene = Scene::new(Camera { projection: DMat4::IDENTITY, world: DMat4::IDENTITY, orthographic: false, near: 0.1, far: 100.0 });
        scene.clock = now_ms;
        let mut load_ms = Vec::new();
        for layer_id in &ids {
            let object = def["scene"]["objects"].as_array().context("objects")?.iter().find(|o| o["id"] == layer_id.as_str()).context("layer in scene")?;
            let asset = object["source"]["assetId"].as_str().context("asset id")?;
            let file = def["assets"][asset]["file"].as_str().context("asset file")?;
            let bytes = std::fs::read(root.join("out/corpus").join(file))?;
            let t0 = now_ms();
            let model = load(&bytes, LoadOptions::default()).map_err(|e| anyhow::anyhow!("{file}: {e}"))?;
            let t1 = now_ms();
            let prepared = Arc::new(Prepared::new(model));
            let t2 = now_ms();
            load_ms.push(json!({ "layer": layer_id, "decode_ms": t1 - t0, "bvh_ms": t2 - t1 }));
            let mut layer = Layer::new(layer_id, prepared, DMat4::IDENTITY);
            layer.lazy = std::env::var_os("S7_FULL_SKIN").is_none();
            scene.layers.push(layer);
        }
        let mut frames_out = Vec::new();
        let (mut total, mut agree, mut fresh_vs_prod_diff, mut ours_vs_prod_diff) = (0usize, 0usize, 0usize, 0usize);
        let mut max_rel = 0f64;
        let mut mismatches = Vec::new();
        for frame in reference["frames"].as_array().context("frames")? {
            let cam = &frame["camera"];
            scene.camera = Camera { projection: mat(&cam["projection"])?, world: mat(&cam["world"])?, orthographic: cam["isOrthographic"].as_bool().unwrap_or(false), near: cam["near"].as_f64().unwrap_or(0.1), far: cam["far"].as_f64().unwrap_or(2000.0) };
            let time = frame["time"].as_f64().unwrap_or(0.0);
            let mut bone_err = 0f64;
            let mut morph_err = 0f64;
            for (li, l) in frame["layers"].as_array().context("layers")?.iter().enumerate() {
                let layer = &mut scene.layers[li];
                layer.gltf_to_world = mat(&l["gltfToWorld"])?;
                layer.set_time(time);
                // Pose check: bone world matrices and morph influences against three.js.
                for skin in l["skins"].as_array().into_iter().flatten() {
                    for bone in skin["bones"].as_array().into_iter().flatten() {
                        let name = bone[0].as_str().unwrap_or("");
                        let Some(ni) = layer.model().nodes.iter().position(|n| n.name == name) else { continue };
                        let ours = layer.node_world(ni).unwrap().to_cols_array();
                        let theirs = mat(&bone[1])?.to_cols_array();
                        for k in 0..16 {
                            bone_err = bone_err.max((ours[k] - theirs[k]).abs());
                        }
                    }
                    if let Some(m) = skin["morph"].as_array() {
                        let name = skin["name"].as_str().unwrap_or("");
                        if let Some(ni) = layer.model().nodes.iter().position(|n| n.name == name) {
                            let ours = layer.morph_weights(ni);
                            for (k, v) in m.iter().enumerate() {
                                morph_err = morph_err.max((ours.get(k).copied().unwrap_or(0.0) - v.as_f64().unwrap_or(0.0)).abs());
                            }
                        }
                    }
                }
            }
            let rays = frame["rays"].as_array().context("rays")?;
            let hits = frame["hits"].as_array().context("hits")?;
            let prod = frame["prod"].as_array().context("prod")?;
            let mut times = Vec::new();
            let mut first_ms = 0.0;
            for (k, (ray, theirs)) in rays.iter().zip(hits).enumerate() {
                let (x, y) = (ray[0].as_f64().unwrap(), ray[1].as_f64().unwrap());
                let t0 = now_ms();
                let ours = scene.pick(x, y, None);
                let dt = now_ms() - t0;
                if k == 0 { first_ms = dt } else { times.push(dt) }
                total += 1;
                let their_layer = theirs.get("layer").and_then(|v| v.as_str());
                let our_layer = ours.as_ref().map(|h| ids[h.layer].as_str());
                let same = match (&ours, theirs.is_null()) {
                    (None, true) => true,
                    (Some(h), false) => {
                        let rel = (h.distance - theirs["distance"].as_f64().unwrap_or(0.0)).abs() / theirs["distance"].as_f64().unwrap_or(1.0).max(1e-9);
                        let kind_ok = theirs["kind"].as_str() == Some(match h.kind { s7_geom3d::model::Topology::Triangles => "mesh", s7_geom3d::model::Topology::Points => "points", _ => "line" });
                        if our_layer == their_layer && kind_ok {
                            max_rel = max_rel.max(rel);
                        }
                        our_layer == their_layer && kind_ok && rel < 1e-4
                    }
                    _ => false,
                };
                if same { agree += 1 } else if mismatches.len() < 12 {
                    mismatches.push(json!({ "time": time, "ray": [x, y], "ours": ours.as_ref().map(|h| json!({ "layer": our_layer, "node": layer_node_name(&scene, h), "kind": format!("{:?}", h.kind), "distance": h.distance, "index": h.index })), "three": theirs }));
                }
                if their_layer != prod[k].as_str() {
                    fresh_vs_prod_diff += 1;
                }
                if our_layer != prod[k].as_str() {
                    ours_vs_prod_diff += 1;
                }
            }
            let hit_count = hits.iter().filter(|h| !h.is_null()).count();
            frames_out.push(json!({
                "time": time, "rays": rays.len(), "hits": hit_count,
                "first_pick_ms": first_ms, "update_ms": scene.layers.iter().map(|l| l.last_update_ms).fold(0.0, f64::max),
                "median_ms": percentile(&mut times.clone(), 0.5), "p95_ms": percentile(&mut times.clone(), 0.95), "max_ms": percentile(&mut times, 1.0),
                "bone_max_abs_err": bone_err, "morph_max_abs_err": morph_err,
                "three_prod_first_ms": frame["prodFirstMs"], "three_prod_hit_median_ms": frame["prodHitMedianMs"],
            }));
        }
        // First pick after a time change: 20 distinct times, a ray that hits.
        let mut changes = Vec::new();
        if let Some(frame) = reference["frames"].as_array().and_then(|f| f.first()) {
            let rays = frame["rays"].as_array().unwrap();
            let hits = frame["hits"].as_array().unwrap();
            let k = hits.iter().position(|h| !h.is_null()).unwrap_or(0);
            let (x, y) = (rays[k][0].as_f64().unwrap(), rays[k][1].as_f64().unwrap());
            for c in 0..20 {
                for l in scene.layers.iter_mut() {
                    l.set_time(0.05 + c as f64 * 0.097);
                }
                let t0 = now_ms();
                scene.pick(x, y, None);
                changes.push(now_ms() - t0);
            }
        }
        let vertices: usize = scene.layers.iter().map(|l| l.vertex_count()).sum();
        let entry = json!({ "scene": id, "vertices": vertices, "load": load_ms, "rays": total, "agree": agree, "max_rel_distance": max_rel, "three_fresh_vs_production_differs": fresh_vs_prod_diff, "ours_vs_production_differs": ours_vs_prod_diff, "frames": frames_out, "mismatches": mismatches, "time_change_first_pick_ms": { "median": percentile(&mut changes.clone(), 0.5), "p95": percentile(&mut changes.clone(), 0.95), "max": percentile(&mut changes, 1.0) } });
        println!("{id}: {vertices} vertices, {agree}/{total} picks agree with three.js (max rel distance {max_rel:.2e}); production (stale caches) differs from fresh three.js on {fresh_vs_prod_diff}");
        for f in entry["frames"].as_array().unwrap() {
            println!("  t={}: first pick {:.2} ms (update {:.2} ms), median {:.4} ms, p95 {:.4} ms, max {:.4} ms; bones err {:.1e}, morph err {:.1e}; three.js production first {} ms, median on hits {} ms", f["time"], f["first_pick_ms"].as_f64().unwrap(), f["update_ms"].as_f64().unwrap(), f["median_ms"].as_f64().unwrap(), f["p95_ms"].as_f64().unwrap(), f["max_ms"].as_f64().unwrap(), f["bone_max_abs_err"].as_f64().unwrap(), f["morph_max_abs_err"].as_f64().unwrap(), f["three_prod_first_ms"], f["three_prod_hit_median_ms"]);
        }
        let tc = &entry["time_change_first_pick_ms"];
        println!("  first pick after a time change (20 times): median {:.3} ms, p95 {:.3} ms, max {:.3} ms", tc["median"].as_f64().unwrap(), tc["p95"].as_f64().unwrap(), tc["max"].as_f64().unwrap());
        report.push(entry);
    }
    let name = if std::env::var_os("S7_FULL_SKIN").is_some() { "native-full.json" } else { "native.json" };
    std::fs::write(picks_dir.join(name), serde_json::to_string_pretty(&report)?)?;
    Ok(())
}

fn layer_node_name(scene: &Scene, h: &s7_geom3d::Hit) -> String {
    scene.layers[h.layer].model().nodes[h.node].name.clone()
}
