//! Legacy OBJ mesh layers against the compositor's own WebGL rendering
//! (`meshExtensionQuad`, its shader sources taken from `compositor.ts` by
//! `tools/three-ref/run-scenes.ts`; the kernel's `parseObj`): the fixed
//! shading into a half-float target with a 16-bit depth buffer, and the
//! missing-definition checkerboard. Judged by section 25's 3D bar on the
//! 8-bit values, with the silhouette (pixels whose neighborhood holds both
//! the background and the model) as the edge class; there is no MSAA.

mod support;

use std::sync::Arc;

use pm_geom3d::obj::parse_obj;
use pm_model::json::{Map, Value};
use pm_render::parity::bar_3d;
use pm_render::scene3d::legacy::ObjMeshPlan;
use pm_render::scene3d::{Object3dPlan, Render3d, Scene3d, Scene3dRenderer};

fn meshes() -> Vec<Map> {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/scene3d-cases.json"
    ))
    .unwrap();
    let Value::Object(doc) = pm_model::json::parse(&text).unwrap() else {
        panic!()
    };
    match doc.get("meshes") {
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| match v {
                Value::Object(m) => m.clone(),
                _ => panic!(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn num(v: &Value) -> f64 {
    match v {
        Value::Number(n) => *n,
        _ => panic!("number"),
    }
}

fn text(m: &Map, k: &str) -> Option<String> {
    match m.get(k) {
        Some(Value::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}

fn rgb(m: &Map, k: &str) -> [f64; 3] {
    pm_color::standard::parse_hex(&text(m, k).unwrap()).rgb
}

#[test]
fn mesh_layers_equal_the_compositor() {
    let Some(dir) = std::env::var_os("PM_SCENE3D_REF")
        .map(std::path::PathBuf::from)
        .or_else(|| support::newest_capture("scene3d-"))
    else {
        eprintln!("no scene3d reference capture; run tools/three-ref/run-scenes.ts");
        return;
    };
    let cases = meshes();
    assert!(cases.len() >= 4);
    let gpu = pm_gpu::Gpu::new("pm-render mesh layers").unwrap();
    let mut renderer = Scene3dRenderer::default();
    let mut pool = pm_gpu::TargetPool::new();
    let mut failed = Vec::new();
    for c in &cases {
        let id = text(c, "id").unwrap();
        let reference = dir.join(&id).join("mesh.f32");
        let Ok(bytes) = std::fs::read(&reference) else {
            failed.push(format!("{id}: no reference (recapture)"));
            continue;
        };
        let (w, h) = (
            num(c.get("width").unwrap()) as u32,
            num(c.get("height").unwrap()) as u32,
        );
        let values: Vec<f64> = match c.get("values") {
            Some(Value::Array(a)) => a.iter().map(num).collect(),
            _ => panic!(),
        };
        let plan = ObjMeshPlan {
            width: w,
            height: h,
            mesh: text(c, "obj").map(|o| Arc::new(parse_obj(&o).unwrap())),
            values: values.try_into().unwrap(),
            object_color: rgb(c, "objectColor"),
            light_color: rgb(c, "lightColor"),
            background: rgb(c, "background"),
        };
        let plan = Object3dPlan {
            layer: id.clone(),
            scene: Arc::new(Scene3d::empty(w, h)),
            kind: Render3d::ObjMesh(plan),
        };
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let out = renderer
            .render(
                &gpu.device,
                &gpu.queue,
                &mut encoder,
                &mut pool,
                &plan,
                false,
            )
            .unwrap();
        gpu.queue.submit([encoder.finish()]);
        let ours = pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &out.color.texture).unwrap();
        let theirs: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        assert_eq!(ours.len(), theirs.len(), "{id}");
        let (w, h) = (w as usize, h as usize);
        let eight = |v: &[f32]| v.iter().map(|x| support::unorm8(*x)).collect::<Vec<u8>>();
        let (a, b) = (eight(&ours), eight(&theirs));
        // The silhouette: a pixel whose 3×3 neighborhood (in the reference)
        // holds both the background and something else.
        let bg = theirs[..4].to_vec();
        let is_bg = |i: usize| theirs[i * 4..i * 4 + 4] == bg[..];
        let mut edges = vec![false; w * h];
        let mut object = vec![false; w * h];
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                object[i] = !is_bg(i);
                let (mut some_bg, mut some_fg) = (false, false);
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let (xx, yy) = (x as i64 + dx, y as i64 + dy);
                        if xx < 0 || yy < 0 || xx >= w as i64 || yy >= h as i64 {
                            continue;
                        }
                        let j = yy as usize * w + xx as usize;
                        if is_bg(j) {
                            some_bg = true;
                        } else {
                            some_fg = true;
                        }
                    }
                }
                edges[i] = some_bg && some_fg;
            }
        }
        let s = bar_3d(&b, &a, &edges, &object, w, h);
        let line = format!(
            "{id:<24} ≤2 CV {:>8.4}%  max {:>3}  max outside edges {:>3}  SSIM {:.5}",
            100.0 * s.fraction_within_2,
            s.max_cv,
            s.max_cv_outside_edges,
            s.object_ssim.unwrap_or(f64::NAN)
        );
        eprintln!("{line}");
        if !s.pass() {
            failed.push(line);
        }
        pool.give(out.color);
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
