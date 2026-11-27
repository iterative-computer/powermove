//! Picks on a posed glTF model: three.js semantics on a small skinned
//! model built here (exact hits against a brute-force posed raycast), and
//! the S7 timing targets on the 1,000,000-vertex skinned model
//! (`PM_GLTF_BIG=<big-skinned.glb>`, `tools/three-ref/gltf/models.ts`
//! `bigSkinned`; ignored, run in release).

use std::sync::Arc;

use pm_geom3d::gltf::math::IDENTITY;
use pm_geom3d::gltf::pick::{Picker, Ray, Side};
use pm_geom3d::gltf::{self, ObjectKind};

/// A GLB with one skinned quad strip on a two-joint chain and a clip.
fn skinned_glb() -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut push = |bytes: &[u8]| {
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let offset = bin.len();
        bin.extend_from_slice(bytes);
        views.push(format!(
            r#"{{"buffer":0,"byteOffset":{offset},"byteLength":{}}}"#,
            bytes.len()
        ));
        views.len() - 1
    };
    let f = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    // 2 × 10 vertices along +Y.
    let mut pos = Vec::new();
    let mut joints = Vec::new();
    let mut weights = Vec::new();
    for j in 0..10 {
        let y = j as f32 * 0.2;
        for x in [-0.2f32, 0.2] {
            pos.extend_from_slice(&[x, y, 0.0]);
            let w = (y / 1.8).clamp(0.0, 1.0);
            joints.extend_from_slice(&[0u8, 1, 0, 0]);
            weights.extend_from_slice(&[1.0 - w, w, 0.0, 0.0]);
        }
    }
    let mut idx: Vec<u16> = Vec::new();
    for j in 0..9u16 {
        let a = j * 2;
        idx.extend_from_slice(&[a, a + 1, a + 3, a, a + 3, a + 2]);
    }
    let vp = push(&f(&pos));
    let vj = push(&joints);
    let vw = push(&f(&weights));
    let vi = push(
        &idx.iter()
            .flat_map(|i| i.to_le_bytes())
            .collect::<Vec<u8>>(),
    );
    let ibm = f(&[
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, //
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -0.9, 0.0, 1.0,
    ]);
    let vb = push(&ibm);
    let vt = push(&f(&[0.0, 1.0]));
    let s = (0.35f32).sin();
    let c = (0.35f32).cos();
    let vr = push(&f(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, s, c]));
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0,2]}}],
"nodes":[{{"name":"root","children":[1]}},{{"name":"tip","translation":[0,0.9,0]}},{{"name":"strip","mesh":0,"skin":0}}],
"skins":[{{"joints":[0,1],"inverseBindMatrices":4}}],
"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"JOINTS_0":1,"WEIGHTS_0":2}},"indices":3}}]}}],
"accessors":[
{{"bufferView":{vp},"componentType":5126,"count":20,"type":"VEC3","min":[-0.2,0,0],"max":[0.2,1.8,0]}},
{{"bufferView":{vj},"componentType":5121,"count":20,"type":"VEC4"}},
{{"bufferView":{vw},"componentType":5126,"count":20,"type":"VEC4"}},
{{"bufferView":{vi},"componentType":5123,"count":{},"type":"SCALAR"}},
{{"bufferView":{vb},"componentType":5126,"count":2,"type":"MAT4"}},
{{"bufferView":{vt},"componentType":5126,"count":2,"type":"SCALAR","min":[0],"max":[1]}},
{{"bufferView":{vr},"componentType":5126,"count":2,"type":"VEC4"}}],
"animations":[{{"samplers":[{{"input":5,"output":6}}],"channels":[{{"sampler":0,"target":{{"node":1,"path":"rotation"}}}}]}}],
"bufferViews":[{}],"buffers":[{{"byteLength":{}}}]}}"#,
        idx.len(),
        views.join(","),
        bin.len()
    );
    let mut j = json.into_bytes();
    while !j.len().is_multiple_of(4) {
        j.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let total = 12 + 8 + j.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&0x4654_6c67u32.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(j.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4e4f_534au32.to_le_bytes());
    out.extend_from_slice(&j);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x004e_4942u32.to_le_bytes());
    out.extend_from_slice(&bin);
    out
}

#[test]
fn skinned_picks_follow_the_pose() {
    let model = Arc::new(gltf::load(&skinned_glb()).expect("loads"));
    let strip = model
        .objects
        .iter()
        .position(|o| matches!(o.kind, ObjectKind::Drawable(_)))
        .unwrap();
    assert!(matches!(&model.objects[strip].kind, ObjectKind::Drawable(d) if d.skin.is_some()));
    let picker = Picker::new(model.clone());
    let side = |_: usize| Side::Front;
    let skip = |_: usize| false;
    // At rest (t = 0) the strip is straight; at t = 0.6 the tip joint bends,
    // so a ray through the rest-pose tip misses and one through the posed
    // tip hits.
    let rest = picker.pose(&IDENTITY, 0.0);
    let bent = picker.pose(&IDENTITY, 0.6);
    let fit = model.fit;
    let tip_rest = [
        (0.0 - fit.center[0]) / fit.radius,
        (1.7 - fit.center[1]) / fit.radius,
        1.0,
    ];
    let ray = Ray {
        origin: tip_rest,
        direction: [0.0, 0.0, -1.0],
    };
    let hit = picker
        .pick(&rest, &ray, 0.0, f64::INFINITY, &side, &skip)
        .expect("rest tip");
    assert_eq!(hit.object, strip);
    assert!((hit.distance - 1.0).abs() < 1e-12);
    assert!(
        picker
            .pick(&bent, &ray, 0.0, f64::INFINITY, &side, &skip)
            .is_none()
    );
    // Every posed triangle centroid is hit at its depth (front faces).
    let posed = pm_geom3d::gltf::pose::Pose::at(&model, 0.6);
    let (parent, posed) = pm_geom3d::gltf::pose::placed(&model, posed, &IDENTITY);
    let worlds = pm_geom3d::gltf::pose::world(&model, &posed, Some(&parent));
    let ObjectKind::Drawable(d) = &model.objects[strip].kind else {
        unreachable!()
    };
    let poser =
        pm_geom3d::gltf::pose::Poser::new(&model, strip, d, &posed.influences[strip], &worlds);
    let g = &model.geometries[d.geometry];
    for t in g.index.as_ref().unwrap().chunks_exact(3) {
        let v: Vec<[f64; 3]> = t
            .iter()
            .map(|&i| gltf::math::apply(&worlds[strip], poser.vertex(i as usize)))
            .collect();
        let c = [
            (v[0][0] + v[1][0] + v[2][0]) / 3.0,
            (v[0][1] + v[1][1] + v[2][1]) / 3.0,
            (v[0][2] + v[1][2] + v[2][2]) / 3.0,
        ];
        let r = Ray {
            origin: [c[0], c[1], c[2] + 2.0],
            direction: [0.0, 0.0, -1.0],
        };
        let h = picker
            .pick(&bent, &r, 0.0, f64::INFINITY, &side, &skip)
            .expect("centroid hit");
        assert!((h.distance - 2.0).abs() < 1e-9, "{}", h.distance);
    }
}

/// S7's targets on the 1M-vertex skinned model: the first pick after a
/// time change ≤ 33 ms, picks at an unchanged frame ≤ 2 ms.
#[test]
#[ignore]
fn million_vertex_picks_meet_the_targets() {
    let Some(path) = std::env::var_os("PM_GLTF_BIG") else {
        eprintln!("PM_GLTF_BIG not set");
        return;
    };
    let bytes = std::fs::read(path).unwrap();
    let t0 = std::time::Instant::now();
    let model = Arc::new(gltf::load(&bytes).expect("loads"));
    let load = t0.elapsed();
    let t0 = std::time::Instant::now();
    let picker = Picker::new(model.clone());
    let build = t0.elapsed();
    let side = |_: usize| Side::Front;
    let skip = |_: usize| false;
    let mut first = Vec::new();
    let mut steady = Vec::new();
    for k in 0..40 {
        let time = 0.05 * f64::from(k);
        let t = std::time::Instant::now();
        let posed = picker.pose(&IDENTITY, time);
        let ray = Ray {
            origin: [0.0, 0.0, 3.0],
            direction: [0.0, 0.0, -1.0],
        };
        let _ = picker.pick(&posed, &ray, 0.0, f64::INFINITY, &side, &skip);
        first.push(t.elapsed().as_secs_f64() * 1e3);
        for j in 0..50 {
            let a = f64::from(j) / 50.0 * std::f64::consts::TAU;
            let ray = Ray {
                origin: [0.3 * a.cos(), 0.6 * a.sin(), 3.0],
                direction: [0.0, 0.0, -1.0],
            };
            let t = std::time::Instant::now();
            let _ = picker.pick(&posed, &ray, 0.0, f64::INFINITY, &side, &skip);
            steady.push(t.elapsed().as_secs_f64() * 1e3);
        }
    }
    let stats = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        (v[v.len() / 2], v[v.len() * 95 / 100], v[v.len() - 1])
    };
    let f = stats(&mut first);
    let s = stats(&mut steady);
    eprintln!(
        "load {load:?}, BVH {build:?}; first pick after a time change median {:.2} ms p95 {:.2} max {:.2}; unchanged frame median {:.3} ms p95 {:.3} max {:.3}",
        f.0, f.1, f.2, s.0, s.1, s.2
    );
    assert!(f.2 <= 33.0, "first pick {f:?}");
    assert!(s.2 <= 2.0, "steady pick {s:?}");
}
