//! OBJ against the app (`tools/three-models.ts --corpus`, reference in
//! `tests/fixtures/three-obj.json`, files in the model corpus outside the
//! repository):
//!
//! - `obj::parse_obj` against the kernel's `parseObj`: counts, center and
//!   radius, the
//!   Float32 output buffers by SHA-256, and the error messages of malformed
//!   sources.
//! - `obj::prepare_import` against `prepareModelImport` (OBJLoader,
//!   MTLLoader, the material conversion, and GLTFExporter): both GLBs are
//!   imported and must build the same objects, geometry, and materials.

use pm_geom3d::obj;
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
    if dir.join("obj-shapes.three.glb").is_file() {
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
    serde_json::from_str(include_str!("fixtures/three-obj.json")).unwrap()
}

fn sha(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn floats(v: &[f32]) -> String {
    sha(&v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<_>>())
}

fn close(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= 1e-15 * b.abs()
}

/// Compares one parse result with the reference entry; returns failures.
fn check_parse(label: &str, source: &str, want: &Value) -> Vec<String> {
    let mut out = vec![];
    match (obj::parse_obj(source), want["error"].as_str()) {
        (Err(e), Some(w)) if e == w => {}
        (Err(e), w) => out.push(format!("{label}: error {e:?} vs {w:?}")),
        (Ok(_), Some(w)) => out.push(format!("{label}: parsed, parseObj threw {w:?}")),
        (Ok(m), None) => {
            for (k, ours) in [
                ("vertexCount", m.vertex_count),
                ("triangleCount", m.triangle_count),
                ("sourceVertexCount", m.source_vertex_count),
            ] {
                if Some(ours as u64) != want[k].as_u64() {
                    out.push(format!("{label}: {k} {ours} vs {}", want[k]));
                }
            }
            let b = &want["bounds"];
            let vec3 = |v: &Value| -> Vec<f64> { v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect() };
            if m.center.iter().zip(vec3(&b["center"])).any(|(a, w)| !close(*a, w)) {
                out.push(format!("{label}: center {:?} vs {}", m.center, b["center"]));
            }
            if !close(m.radius, b["radius"].as_f64().unwrap()) {
                out.push(format!("{label}: radius {} vs {}", m.radius, b["radius"]));
            }
            for (k, ours) in [("positions", &m.positions), ("normals", &m.normals), ("texcoords", &m.texcoords)] {
                if Some(floats(ours).as_str()) != want[k].as_str() {
                    out.push(format!("{label}: {k} differ"));
                }
            }
        }
    }
    out
}

#[test]
fn parses_like_parse_obj() {
    let reference = reference();
    let mut failures = vec![];
    for entry in reference["sources"].as_array().unwrap() {
        let source = entry["source"].as_str().unwrap();
        failures.extend(check_parse(&format!("{source:?}"), source, entry));
    }
    if let Some(dir) = corpus() {
        for (file, want) in reference["parseObj"].as_object().unwrap() {
            let bytes = std::fs::read(dir.join(file)).unwrap();
            assert_eq!(Some(sha(&bytes).as_str()), want["sha256"].as_str(), "{file} changed; rerun the tool");
            failures.extend(check_parse(file, std::str::from_utf8(&bytes).unwrap(), want));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[cfg(feature = "import")]
#[test]
fn packs_like_prepare_model_import() {
    use pm_geom3d::gltf;
    let Some(dir) = corpus() else { return };
    let reference = reference();
    let mut failures = vec![];
    for (name, want) in reference["bundles"].as_object().unwrap() {
        let read = |ext: &str| std::fs::read(dir.join(format!("{name}{ext}"))).unwrap();
        let (obj_bytes, mtl_bytes, three) = (read(".obj"), read(".mtl"), read(".three.glb"));
        assert_eq!(Some(sha(&obj_bytes).as_str()), want["obj"].as_str(), "{name}.obj changed");
        assert_eq!(Some(sha(&mtl_bytes).as_str()), want["mtl"].as_str(), "{name}.mtl changed");
        assert_eq!(Some(sha(&three).as_str()), want["glb"].as_str(), "{name}.three.glb changed");
        let files = [
            obj::ImportFile {
                name: format!("{name}.obj"),
                mime: String::new(),
                bytes: obj_bytes,
            },
            obj::ImportFile {
                name: format!("{name}.mtl"),
                mime: String::new(),
                bytes: mtl_bytes,
            },
        ];
        let obj::Prepared::Glb { name: packed, bytes } = obj::prepare_import(&files).unwrap() else {
            panic!("{name}: not packed");
        };
        assert_eq!(Some(packed.as_str()), want["name"].as_str());
        let (om, os, of) = gltf::import(&bytes).unwrap_or_else(|e| panic!("{name}: ours: {e}"));
        let (tm, ts, tf) = gltf::import(&three).unwrap_or_else(|e| panic!("{name}: three.js's: {e}"));
        let mut fail = |w: String| failures.push(format!("{name}: {w}"));
        if (of.vertices, of.triangles, of.radius, of.center) != (tf.vertices, tf.triangles, tf.radius, tf.center) {
            fail(format!("facts {of:?} vs {tf:?}"));
        }
        if os.objects.len() != ts.objects.len() {
            fail(format!("{} objects vs {}", os.objects.len(), ts.objects.len()));
            continue;
        }
        for (o, t) in os.objects.iter().zip(&ts.objects) {
            let label = format!("{} {:?}", t.name, t.kind);
            if (o.kind, &o.name, o.matrix) != (t.kind, &t.name, t.matrix) {
                fail(format!("{label}: object {:?} {:?}", o.kind, o.name));
            }
            match (o.primitive, t.primitive) {
                (None, None) => {}
                (Some((om_, op)), Some((tm_, tp))) => {
                    let (a, b) = (&om.meshes[om_].primitives[op], &tm.meshes[tm_].primitives[tp]);
                    let same = |x: &dyn std::fmt::Debug, y: &dyn std::fmt::Debug| format!("{x:?}") == format!("{y:?}");
                    for (k, x, y) in [
                        ("topology", &a.topology as &dyn std::fmt::Debug, &b.topology as &dyn std::fmt::Debug),
                        ("positions", &a.positions, &b.positions),
                        ("normals", &a.normals, &b.normals),
                        ("uv", &a.uv, &b.uv),
                        ("colors", &a.colors, &b.colors),
                        ("indices", &a.indices, &b.indices),
                        ("bounds", &a.accessor_bounds, &b.accessor_bounds),
                        ("undrawable", &a.undrawable, &b.undrawable),
                    ] {
                        if !same(x, y) {
                            fail(format!("{label}: {k} differ"));
                        }
                    }
                    let (ma, mb) = (gltf::material_facts(&om, o), gltf::material_facts(&tm, t));
                    if ma != mb {
                        fail(format!("{label}: material {ma:?} vs {mb:?}"));
                    }
                    let (da, db) = (
                        a.material.map(|i| &om.materials[i]),
                        b.material.map(|i| &tm.materials[i]),
                    );
                    if da != db {
                        fail(format!("{label}: material definition {da:?} vs {db:?}"));
                    }
                }
                _ => fail(format!("{label}: geometry on one side only")),
            }
        }
        eprintln!("{name}: {} objects, {} vertices, {} triangles", os.objects.len(), of.vertices, of.triangles);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
