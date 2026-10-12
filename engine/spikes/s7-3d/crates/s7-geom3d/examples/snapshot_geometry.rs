//! Section 25 "Blender snapshots" check for geometry: the world-space triangle positions
//! a snapshot carries (renderSnapshot: `getVertexPosition` × `matrixWorld`) against
//! pm-geom3d's posed geometry at the same time.
//!
//!   cargo run --example snapshot_geometry -- <model.glb> <snapshot.json> <layer> <picks.json> <time>
use glam::{DMat4, DVec4};
use serde_json::Value;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let prepared = std::sync::Arc::new(s7_geom3d::Prepared::new(s7_geom3d::load(&std::fs::read(&a[0]).unwrap(), Default::default()).unwrap()));
    let snapshot: Value = serde_json::from_slice(&std::fs::read(&a[1]).unwrap()).unwrap();
    let picks: Value = serde_json::from_slice(&std::fs::read(&a[3]).unwrap()).unwrap();
    let time: f64 = a[4].parse().unwrap();
    let frame = picks["frames"].as_array().unwrap().iter().find(|f| (f["time"].as_f64().unwrap() - time).abs() < 1e-9).expect("frame");
    let li = picks["ids"].as_array().unwrap().iter().position(|v| v == a[2].as_str()).unwrap();
    let m: Vec<f64> = frame["layers"][li]["gltfToWorld"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    let mut layer = s7_geom3d::Layer::new(&a[2], prepared, DMat4::from_cols_slice(&m));
    layer.set_time(time);
    let ours = layer.posed_triangles(s7_geom3d::wasm::now_ms);
    let object = snapshot["objects"].as_array().unwrap().iter().find(|o| o["id"] == a[2].as_str()).unwrap();
    let meshes = object["meshes"].as_array().unwrap();
    // renderSnapshot pushes meshes asynchronously; match by vertex count, then by distance.
    let mut used = vec![false; ours.len()];
    let mut worst = 0f64;
    for mesh in meshes {
        let mat: Vec<f64> = mesh["matrix"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        let mat = DMat4::from_cols_slice(&mat);
        let pos: Vec<f64> = mesh["positions"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        let world: Vec<[f64; 3]> = pos.chunks_exact(3).map(|p| (mat * DVec4::new(p[0], p[1], p[2], 1.0)).truncate().to_array()).collect();
        let mut best: Option<(usize, f64)> = None;
        for (k, (_, list)) in ours.iter().enumerate() {
            if used[k] || list.len() != world.len() {
                continue;
            }
            let d = list.iter().zip(&world).map(|(x, y)| (0..3).map(|c| (x[c] - y[c]).abs()).fold(0.0, f64::max)).fold(0.0, f64::max);
            if best.is_none_or(|b| d < b.1) {
                best = Some((k, d));
            }
        }
        let (k, d) = best.expect("no matching drawable");
        used[k] = true;
        worst = worst.max(d);
        println!("{} ({} vertices): max |Δ| {d:.3e}", ours[k].0, world.len());
    }
    println!("layer {}: {} meshes, max |Δ| {worst:.3e} (section 25 bound 1e-6)", a[2], meshes.len());
}
