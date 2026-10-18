//! Model import against three.js r180 (the app's own `loadGltf`), on the
//! model corpus (`tools/three-models.ts --corpus`: the parity corpus's GLBs
//! and synthetic edge cases, written outside the repository; the reference,
//! `tests/fixtures/three-models.json`, records each file's SHA-256).
//!
//! For every model: the import facts (vertices, triangles, the
//! normalization's center and radius), the object graph in traversal order
//! (type, local matrix, geometry counts and bounds), each object's material
//! class and parameters, and the animation clips.

use pm_geom3d::model_import::{self as gltf, ObjectKind};
use serde_json::Value;
use sha2::Digest;
use std::path::PathBuf;

fn corpus() -> Option<PathBuf> {
    let dir = std::env::var_os("PM_MODEL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap())
                .join("Library/Caches/powermove-engine/model-corpus")
        });
    if dir.join("corpus-model.glb").is_file() {
        Some(dir)
    } else {
        eprintln!(
            "the model corpus is missing at {}; run `bun engine/crates/pm-geom3d/tools/three-models.ts --corpus` (skipped)",
            dir.display()
        );
        None
    }
}

fn reference() -> Value {
    serde_json::from_str(include_str!("fixtures/three-models.json")).unwrap()
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol * (1.0 + b.abs())
}

/// The reference rounds every float to f32 (`Math.fround`) and serde_json
/// reads it back within an ulp; compare the f32 rounding of ours.
fn same(ours: f64, theirs: f64) -> bool {
    close(f64::from(ours as f32), theirs, 1e-12)
}

fn nums(v: &Value) -> Vec<f64> {
    v.as_array()
        .map(|a| a.iter().map(|x| x.as_f64().unwrap()).collect())
        .unwrap_or_default()
}

/// Differences three.js has by defect, not by design (recorded for
/// docs/engine/formats.md): with a sparse accessor over an interleaved
/// buffer view, GLTFLoader copies the whole interleaved array into a
/// non-interleaved attribute (`array.slice()`), so the position count is the
/// view's length over the item size (16 / 3 here) and the geometry is
/// garbage. The engine reads the accessor.
const THREE_DEFECTS: &[&str] = &["sparse-quantized.gltf"];

#[test]
fn imports_like_three() {
    let Some(dir) = corpus() else { return };
    let reference = reference();
    let mut failures = vec![];
    for (file, want) in reference.as_object().unwrap() {
        let path = dir.join(file);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let sha: String = sha2::Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            Some(sha.as_str()),
            want["sha256"].as_str(),
            "{file} differs from the file the reference was made from; rerun the tool"
        );
        let (model, scene, facts) = match gltf::import(&bytes) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("{file}: {e}"));
                continue;
            }
        };
        let mut fail = |what: String| failures.push(format!("{file}: {what}"));
        if THREE_DEFECTS.contains(&file.as_str()) {
            assert_eq!(facts.vertices, 4, "{file}: the engine reads the sparse accessor");
            assert!(want["vertices"].as_f64().unwrap().fract() != 0.0);
            continue;
        }
        if facts.vertices as f64 != want["vertices"].as_f64().unwrap() {
            fail(format!("vertices {} vs {}", facts.vertices, want["vertices"]));
        }
        if facts.triangles as f64 != want["triangles"].as_f64().unwrap() {
            fail(format!("triangles {} vs {}", facts.triangles, want["triangles"]));
        }
        if !close(facts.radius, want["radius"].as_f64().unwrap(), 1e-9) {
            fail(format!("radius {} vs {}", facts.radius, want["radius"]));
        }
        for (c, w) in facts.center.iter().zip(nums(&want["center"])) {
            if !close(*c, w, 1e-9) {
                fail(format!("center {:?} vs {}", facts.center, want["center"]));
                break;
            }
        }
        let objects = want["objects"].as_array().unwrap();
        if objects.len() != scene.objects.len() {
            fail(format!(
                "{} objects vs three.js's {}: {:?}",
                scene.objects.len(),
                objects.len(),
                scene
                    .objects
                    .iter()
                    .map(|o| o.kind.type_name())
                    .collect::<Vec<_>>()
            ));
            continue;
        }
        for (i, (ours, theirs)) in scene.objects.iter().zip(objects).enumerate() {
            let label = format!("object {i} {}", theirs["name"]);
            if ours.kind.type_name() != theirs["type"].as_str().unwrap() {
                fail(format!("{label}: {} vs {}", ours.kind.type_name(), theirs["type"]));
                continue;
            }
            if ours.name != theirs["name"].as_str().unwrap() {
                fail(format!("{label}: named {:?}", ours.name));
            }
            let m = nums(&theirs["matrix"]);
            if ours.matrix.iter().zip(&m).any(|(a, b)| !close(*a, *b, 1e-6)) {
                fail(format!("{label}: matrix {:?} vs {m:?}", ours.matrix));
            }
            if ours.kind == ObjectKind::InstancedMesh {
                let n = model.nodes[ours.node.unwrap()].instances.as_ref().unwrap().len();
                if Some(n as u64) != theirs["count"].as_u64() {
                    fail(format!("{label}: {n} instances"));
                }
            }
            let geometry = &theirs["geometry"];
            match ours.primitive {
                None => {
                    if !geometry.is_null() {
                        fail(format!("{label}: no geometry"));
                    }
                }
                Some((mi, pi)) => {
                    let p = &model.meshes[mi].primitives[pi];
                    let attrs = &geometry["attributes"];
                    let count = |name: &str| attrs[name]["count"].as_u64().map(|c| c as usize);
                    let checks = [
                        ("position", Some(p.positions.len())),
                        ("normal", p.normals.as_ref().map(Vec::len)),
                        ("uv", p.uv.as_ref().map(Vec::len)),
                        ("uv1", p.uv1.as_ref().map(Vec::len)),
                        ("color", p.colors.as_ref().map(Vec::len)),
                        ("tangent", p.tangents.as_ref().map(Vec::len)),
                        ("skinIndex", p.joints.as_ref().map(Vec::len)),
                        ("skinWeight", p.weights.as_ref().map(Vec::len)),
                    ];
                    for (name, ours) in checks {
                        if ours != count(name) {
                            fail(format!("{label}: {name} {ours:?} vs {:?}", count(name)));
                        }
                    }
                    // Attribute sums catch decoding errors (normalized and
                    // quantized components, strides).
                    let sum3 = |v: &[[f32; 3]]| -> f64 {
                        v.iter().flatten().map(|x| f64::from(*x)).sum()
                    };
                    let sum2 = |v: &[[f32; 2]]| -> f64 {
                        v.iter().flatten().map(|x| f64::from(*x)).sum()
                    };
                    let mut sums = vec![("position", Some(sum3(&p.positions)))];
                    sums.push(("normal", p.normals.as_deref().map(sum3)));
                    sums.push(("uv", p.uv.as_deref().map(sum2)));
                    sums.push(("uv1", p.uv1.as_deref().map(sum2)));
                    for (name, s) in sums {
                        let (Some(s), Some(w)) = (s, attrs[name]["sum"].as_f64()) else {
                            continue;
                        };
                        if !close(s, w, 1e-5) {
                            fail(format!("{label}: {name} sums to {s} vs {w}"));
                        }
                    }
                    let index = geometry["index"]["count"].as_u64().map(|c| c as usize);
                    if p.indices.as_ref().map(Vec::len) != index {
                        fail(format!("{label}: index {:?} vs {index:?}", p.indices.as_ref().map(Vec::len)));
                    }
                    if let Some(isum) = geometry["index"]["sum"].as_f64() {
                        let s: f64 = p.indices.iter().flatten().map(|&i| f64::from(i)).sum();
                        if s != isum {
                            fail(format!("{label}: indices sum to {s} vs {isum}"));
                        }
                    }
                    let b = gltf::geometry_box(p);
                    let wb: Vec<Vec<f64>> = geometry["bounds"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(nums)
                        .collect();
                    let ours_b = [b.min, b.max];
                    if ours_b
                        .iter()
                        .zip(&wb)
                        .any(|(o, w)| o.iter().zip(w).any(|(a, b)| !same(*a, *b)))
                    {
                        fail(format!("{label}: bounds {ours_b:?} vs {wb:?}"));
                    }
                    let targets = geometry["morphTargets"]["position"].as_u64().unwrap_or(0) as usize;
                    if p.targets.iter().filter(|t| t.position.is_some()).count() != targets {
                        fail(format!("{label}: morph targets"));
                    }
                    let mf = gltf::material_facts(&model, ours).unwrap();
                    let wm = &theirs["material"];
                    let mut mfail = |w: String| fail(format!("{label}: material {w}"));
                    if mf.class != wm["type"].as_str().unwrap() {
                        mfail(format!("{} vs {}", mf.class, wm["type"]));
                    }
                    for (k, ours, theirs) in [
                        ("side", mf.side as f64, wm["side"].as_f64()),
                        ("opacity", f64::from(mf.opacity), wm["opacity"].as_f64()),
                        ("alphaTest", f64::from(mf.alpha_test), wm["alphaTest"].as_f64()),
                    ] {
                        if !theirs.is_some_and(|t| same(ours, t)) {
                            mfail(format!("{k} {ours} vs {theirs:?}"));
                        }
                    }
                    for (k, ours) in [
                        ("transparent", mf.transparent),
                        ("vertexColors", mf.vertex_colors),
                        ("flatShading", mf.flat_shading),
                    ] {
                        if wm[k].as_bool() != Some(ours) {
                            mfail(format!("{k} {ours} vs {}", wm[k]));
                        }
                    }
                    let c = nums(&wm["color"]);
                    if mf.color.iter().zip(&c).any(|(a, b)| !same(f64::from(*a), *b)) {
                        mfail(format!("color {:?} vs {c:?}", mf.color));
                    }
                    for (k, ours) in [
                        ("roughness", mf.roughness),
                        ("metalness", mf.metalness),
                        ("emissiveIntensity", mf.emissive_intensity),
                    ] {
                        let ok = match (ours, wm[k].as_f64()) {
                            (Some(o), Some(t)) => same(f64::from(o), t),
                            (None, None) => true,
                            _ => false,
                        };
                        if !ok {
                            mfail(format!("{k} {ours:?} vs {}", wm[k]));
                        }
                    }
                    if let Some(e) = mf.emissive
                        && e.iter().zip(nums(&wm["emissive"])).any(|(a, b)| !same(f64::from(*a), b))
                    {
                        mfail(format!("emissive {e:?} vs {}", wm["emissive"]));
                    }
                    if let Some(ns) = mf.normal_scale {
                        let w = nums(&wm["normalScale"]);
                        if ns.iter().zip(&w).any(|(a, b)| !same(f64::from(*a), *b)) {
                            mfail(format!("normalScale {ns:?} vs {w:?}"));
                        }
                    }
                    let mut maps: Vec<(String, u64)> = wm["maps"]
                        .as_object()
                        .unwrap()
                        .iter()
                        .map(|(k, v)| (k.clone(), v["channel"].as_u64().unwrap()))
                        .collect();
                    maps.sort();
                    let mut ours_maps: Vec<(String, u64)> = mf
                        .maps
                        .iter()
                        .map(|(n, c)| ((*n).to_string(), u64::from(*c)))
                        .collect();
                    ours_maps.sort();
                    if ours_maps != maps {
                        mfail(format!("maps {ours_maps:?} vs {maps:?}"));
                    }
                }
            }
        }
        let clips = want["animations"].as_array().unwrap();
        if clips.len() != model.clips.len() {
            fail(format!("{} clips vs {}", model.clips.len(), clips.len()));
        }
        for (c, w) in model.clips.iter().zip(clips) {
            if c.name != w["name"].as_str().unwrap() || !same(f64::from(c.duration), w["duration"].as_f64().unwrap()) {
                fail(format!("clip {:?} {} vs {} {}", c.name, c.duration, w["name"], w["duration"]));
            }
            let tracks = w["tracks"].as_array().unwrap();
            if tracks.len() != c.channels.len() {
                fail(format!("clip {:?}: {} channels vs {} tracks", c.name, c.channels.len(), tracks.len()));
            }
            for (ch, t) in c.channels.iter().zip(tracks) {
                if ch.times.len() as u64 != t["times"].as_u64().unwrap() {
                    fail(format!("clip {:?}: track {} keys", c.name, t["name"]));
                }
            }
        }
        eprintln!(
            "{file}: {} objects, {} vertices, {} triangles, radius {:.6}",
            scene.objects.len(),
            facts.vertices,
            facts.triangles,
            facts.radius
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
