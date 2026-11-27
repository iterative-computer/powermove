//! Primitive geometry against three.js r180 (`tools/three-primitives.ts`
//! writes the reference hashes from three.js itself): every buffer
//! bit-identical.

use pm_geom3d::{Parameters, Primitive};
use sha2::{Digest, Sha256};

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn f32_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

#[test]
fn primitives_equal_three_js() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/three-primitives.json"
    ))
    .unwrap();
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(doc["three"], "0.180.0");
    let cases = doc["cases"].as_array().unwrap();
    assert!(cases.len() >= 18);
    for case in cases {
        let name = case["primitive"].as_str().unwrap();
        let p = &case["parameters"];
        let get = |k: &str| p.get(k).and_then(|v| v.as_f64());
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
        let g = Primitive::from_parameters(name, &params)
            .unwrap_or_else(|| panic!("{name} is a supported primitive"))
            .geometry();
        let what = format!("{name} {p}");
        assert_eq!(
            g.vertex_count() as u64,
            case["vertices"].as_u64().unwrap(),
            "{what}"
        );
        assert_eq!(
            g.indices.len() as u64,
            case["indices"].as_u64().unwrap(),
            "{what}"
        );
        assert_eq!(
            hex(&f32_bytes(&g.positions)),
            case["position"],
            "{what}: positions"
        );
        assert_eq!(
            hex(&f32_bytes(&g.normals)),
            case["normal"],
            "{what}: normals"
        );
        assert_eq!(hex(&f32_bytes(&g.uvs)), case["uv"], "{what}: uvs");
        let idx: Vec<u8> = g.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        assert_eq!(hex(&idx), case["index"], "{what}: indices");
    }
}
