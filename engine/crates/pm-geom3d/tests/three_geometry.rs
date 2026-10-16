//! Every scene source kind against three.js r180 and the kernel's OBJ
//! parser (`tools/three-geometry.ts` writes the reference hashes from
//! three.js itself): buffers, index, and groups bit-identical.

use pm_geom3d::{Group, MeshSource, Parameters, Source, obj::parse_obj};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn f32_hex(v: &[f32]) -> String {
    hex(&v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>())
}

fn points(v: &Value) -> Vec<[f64; 2]> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

fn numbers(v: Option<&Value>) -> Option<Vec<f64>> {
    v.map(|a| a.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect())
}

/// The source as `parseScene` stores it.
fn source(s: &Value) -> Source {
    if let Some(name) = s.get("primitive").and_then(Value::as_str) {
        let p = &s["parameters"];
        let get = |k: &str| p.get(k).and_then(Value::as_f64);
        let params = Parameters {
            width: get("width"),
            height: get("height"),
            depth: get("depth"),
            radius: get("radius"),
            radius_top: get("radiusTop"),
            radius_bottom: get("radiusBottom"),
            tube: get("tube"),
            segments: get("segments"),
            detail: get("detail"),
        };
        return Source::primitive(name, &params).unwrap();
    }
    if let Some(l) = s.get("lathe") {
        return Source::Lathe {
            points: points(l),
            segments: s.get("segments").and_then(Value::as_f64).unwrap_or(0.0),
        };
    }
    if let Some(e) = s.get("extrude") {
        return Source::Extrude {
            points: points(e),
            depth: s["depth"].as_f64().unwrap(),
            bevel: s.get("bevel").and_then(Value::as_f64).unwrap_or(0.0),
        };
    }
    let m = &s["mesh"];
    Source::Mesh(MeshSource {
        positions: numbers(m.get("positions")).unwrap(),
        indices: numbers(m.get("indices")).map(|v| v.iter().map(|i| *i as u32).collect()),
        normals: numbers(m.get("normals")),
        uvs: numbers(m.get("uvs")),
        groups: m
            .get("groups")
            .and_then(Value::as_array)
            .map(|g| {
                g.iter()
                    .map(|g| Group {
                        start: g["start"].as_u64().unwrap() as u32,
                        count: g["count"].as_u64().unwrap() as u32,
                        material_index: g["materialIndex"].as_u64().unwrap() as u32,
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// serde_json's default float parsing can be one ulp off; the fixture's
/// numbers are read with the model's correctly rounded JSON parser.
fn exact(v: pm_model::json::Value) -> Value {
    use pm_model::json::Value as M;
    match v {
        M::Null => Value::Null,
        M::Bool(b) => Value::Bool(b),
        M::Number(n) if n >= 0.0 && n.fract() == 0.0 && n < 9e15 => Value::from(n as u64),
        M::Number(n) => serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number),
        M::String(s) => Value::String(s.to_string_lossy()),
        M::Array(a) => Value::Array(a.into_iter().map(exact).collect()),
        M::Object(m) => Value::Object(
            m.into_iter()
                .map(|(k, v)| (k.to_string_lossy(), exact(v)))
                .collect(),
        ),
    }
}

fn fixture() -> Value {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/three-geometry.json"
    ))
    .unwrap();
    exact(pm_model::json::parse(&text).unwrap())
}

#[test]
fn sources_equal_three_js() {
    let doc = fixture();
    assert_eq!(doc["three"], "0.180.0");
    let cases = doc["geometry"].as_array().unwrap();
    assert!(cases.len() >= 30);
    let mut failures = Vec::new();
    for case in cases {
        let g = source(&case["source"]).geometry();
        let what = case["source"].to_string();
        let what = format!("{} ({} chars)", &what[..what.len().min(60)], what.len());
        let mut check = |label: &str, ok: bool| {
            if !ok {
                failures.push(format!("{what}: {label}"));
            }
        };
        check(
            "vertices",
            g.vertex_count() as u64 == case["vertices"].as_u64().unwrap(),
        );
        check("indexed", g.indexed == case["indexed"].as_bool().unwrap());
        check(
            "draw count",
            g.indices.len() as u64 == case["indices"].as_u64().unwrap(),
        );
        check("positions", f32_hex(&g.positions) == case["position"]);
        check("normals", f32_hex(&g.normals) == case["normal"]);
        match case["uv"].as_str() {
            Some(h) => check("uvs", f32_hex(&g.uvs) == h),
            None => check("no uvs", g.uvs.is_empty()),
        }
        if let Some(h) = case["index"].as_str() {
            let idx: Vec<u8> = g.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
            check("indices", hex(&idx) == h);
        }
        let groups: Vec<[u64; 3]> = g
            .groups
            .iter()
            .map(|x| {
                [
                    u64::from(x.start),
                    u64::from(x.count),
                    u64::from(x.material_index),
                ]
            })
            .collect();
        let expected: Vec<[u64; 3]> = case["groups"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| {
                [
                    g[0].as_u64().unwrap(),
                    g[1].as_u64().unwrap(),
                    g[2].as_u64().unwrap(),
                ]
            })
            .collect();
        check("groups", groups == expected);
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn obj_meshes_equal_the_kernel_parser() {
    let doc = fixture();
    let cases = doc["obj"].as_array().unwrap();
    assert!(cases.len() >= 5);
    for case in cases {
        let m = parse_obj(case["text"].as_str().unwrap()).unwrap();
        assert_eq!(m.vertex_count as u64, case["vertices"].as_u64().unwrap());
        assert_eq!(f32_hex(&m.positions), case["position"]);
        assert_eq!(f32_hex(&m.normals), case["normal"]);
        assert_eq!(f32_hex(&m.texcoords), case["texcoord"]);
        assert_eq!(m.radius, case["radius"].as_f64().unwrap());
    }
}
