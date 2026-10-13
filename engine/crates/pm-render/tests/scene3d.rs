//! The 3D primitive against three.js r180 (section 25's 3D class; amendment
//! 11): every case of `tests/fixtures/three-scenes.json` is rendered by the
//! 3D scene pass and compared with three.js's own render of the same scene
//! through the production `SceneRuntime` (`tools/three-ref`, captured outside
//! the repository). Also checks that the uniform values the pass computes
//! equal the values three.js uploaded for the same draw.
//!
//! `PM_THREE_REF=<dir>` picks a capture (default: the newest `three-*`);
//! `PM_RENDER_DIFFS=<dir>` writes ours and theirs for failing cases.

mod support;

use pm_geom3d::{Parameters, Primitive};
use pm_render::parity::{bar_3d, msaa_edges};
use pm_render::scene3d::{Camera3d, Material3d, Object3dPlan, Scene3dRenderer, Sun, uniforms};
use pm_render::three::math;
use serde_json::Value;

fn cases() -> Vec<Value> {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/three-scenes.json"
    ))
    .unwrap();
    let doc: Value = serde_json::from_str(&text).unwrap();
    doc["cases"].as_array().unwrap().clone()
}

fn capture() -> Option<std::path::PathBuf> {
    std::env::var_os("PM_THREE_REF")
        .map(std::path::PathBuf::from)
        .or_else(|| support::newest_capture("three-"))
}

fn v3(v: &Value) -> [f64; 3] {
    let a = v.as_array().unwrap();
    [a[0].as_f64().unwrap(), a[1].as_f64().unwrap(), a[2].as_f64().unwrap()]
}

/// The plan `plan_object` would build for the case's scene.
fn plan_of(c: &Value) -> Object3dPlan {
    let o = &c["object"];
    let p = &o["parameters"];
    let get = |k: &str| p.get(k).and_then(Value::as_f64);
    let primitive = Primitive::from_parameters(
        o["primitive"].as_str().unwrap(),
        &Parameters {
            width: get("width"),
            height: get("height"),
            depth: get("depth"),
            radius: get("radius"),
            radius_top: get("radiusTop"),
            radius_bottom: get("radiusBottom"),
            tube: get("tube"),
            segments: get("segments"),
            detail: get("detail"),
        },
    )
    .unwrap();
    let mut model = [0.0; 16];
    for (i, v) in o["matrix"].as_array().unwrap().iter().enumerate() {
        model[i] = v.as_f64().unwrap();
    }
    let m = &o["material"];
    let hex = |v: &Value| math::color_hex(v.as_str().unwrap()).unwrap();
    let cam = &c["camera"];
    let near = cam["near"].as_f64().unwrap().max(0.0001);
    let env = &c["environment"];
    let sky = hex(&env["ambientColor"]);
    Object3dPlan {
        layer: c["id"].as_str().unwrap().to_owned(),
        width: c["width"].as_u64().unwrap() as u32,
        height: c["height"].as_u64().unwrap() as u32,
        primitive,
        model,
        material: Material3d {
            name: "Surface".into(),
            color: hex(&m["color"]),
            roughness: m["roughness"].as_f64().unwrap(),
            metalness: m["metalness"].as_f64().unwrap(),
            emissive: hex(&m["emissive"]),
            emissive_intensity: m["emissiveIntensity"].as_f64().unwrap(),
            double_sided: m["doubleSided"].as_bool().unwrap(),
        },
        depth_write: c["layerOpacity"].as_f64().unwrap() >= 0.999,
        camera: Camera3d {
            world: math::camera_world(v3(&cam["position"]), v3(&cam["target"]), v3(&cam["up"])),
            orthographic: cam["projection"] == "orthographic",
            fov: cam["fov"].as_f64().unwrap().clamp(1.0, 150.0),
            zoom: cam["zoom"].as_f64().unwrap().max(0.001),
            near,
            far: cam["far"].as_f64().unwrap().max(near + 0.01),
        },
        exposure: env["exposure"].as_f64().unwrap(),
        hemisphere: (
            sky,
            [sky[0] * 0.35, sky[1] * 0.35, sky[2] * 0.35],
            env["ambient"].as_f64().unwrap(),
        ),
        suns: c["lights"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|l| l["visible"].as_bool() != Some(false))
            .map(|l| Sun {
                position: v3(&l["position"]),
                target: v3(&l["target"]),
                color: hex(&l["color"]),
                intensity: l["intensity"].as_f64().unwrap(),
            })
            .collect(),
        background: if c["bottom"].as_bool() == Some(true) {
            env["background"].as_str().map(|s| math::color_hex(s).unwrap())
        } else {
            None
        },
    }
}

#[test]
fn uniforms_equal_three_js() {
    let Some(dir) = capture() else {
        eprintln!("no three.js reference capture; run tools/three-ref/run.ts");
        return;
    };
    let mut worst = (0.0f64, String::new());
    let mut compared = 0;
    for c in cases() {
        let id = c["id"].as_str().unwrap();
        let record: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(id).join("record.json")).unwrap())
                .unwrap();
        // Draw 0 is the depth prepass, draw 1 the color pass (MSAA canvas).
        let draw = &record["draws"][1];
        assert_eq!(draw["depthMask"], false, "{id}: draw 1 is the color pass");
        let ours = uniforms(&plan_of(&c));
        for (name, recorded) in draw["uniforms"].as_object().unwrap() {
            let recorded: Vec<f64> = recorded
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect();
            let mine = ours
                .iter()
                .find(|(k, _)| k == name)
                .unwrap_or_else(|| panic!("{id}: three.js sets {name}, the pass does not"))
                .1
                .clone();
            assert_eq!(mine.len(), recorded.len(), "{id}: {name} length");
            for (a, b) in mine.iter().zip(&recorded) {
                // The GPU receives f32.
                let (a, b) = (*a as f32, *b as f32);
                let d = f64::from((a - b).abs()) / f64::from(b.abs().max(1.0));
                if d > worst.0 {
                    worst = (d, format!("{id} {name}: ours {a} three.js {b}"));
                }
                compared += 1;
            }
        }
    }
    eprintln!("{compared} uniform values; worst relative difference {:e} ({})", worst.0, worst.1);
    assert!(worst.0 <= 1e-6, "uniforms differ: {}", worst.1);
}

#[test]
fn primitives_within_the_3d_bar() {
    let Some(dir) = capture() else {
        eprintln!("no three.js reference capture; run tools/three-ref/run.ts");
        return;
    };
    let diffs = std::env::var_os("PM_RENDER_DIFFS").map(std::path::PathBuf::from);
    let gpu = pm_gpu::Gpu::new("pm-render 3d test").expect("a Metal device");
    let mut scene = Scene3dRenderer::default();
    let mut pool = pm_gpu::TargetPool::new();
    let mut failed = Vec::new();
    for c in cases() {
        let id = c["id"].as_str().unwrap().to_owned();
        let plan = plan_of(&c);
        let (w, h) = (plan.width as usize, plan.height as usize);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let out = scene
            .render(&gpu.device, &gpu.queue, &mut encoder, &mut pool, &plan, true)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        gpu.queue.submit([encoder.finish()]);
        let gl = pm_gpu::readback::read_raw(&gpu.device, &gpu.queue, &out.color.texture).unwrap();
        let depth = pm_gpu::readback::read_f32(
            &gpu.device,
            &gpu.queue,
            &out.depth.as_ref().unwrap().texture,
        )
        .unwrap();
        // GL row order to top-down.
        let row = w * 4;
        let mut ours = vec![0u8; gl.len()];
        let mut object = vec![false; w * h];
        for y in 0..h {
            ours[y * row..(y + 1) * row].copy_from_slice(&gl[(h - 1 - y) * row..(h - y) * row]);
            for x in 0..w {
                object[y * w + x] = depth[(h - 1 - y) * w + x].is_finite();
            }
        }
        let (rw, rh, reference) = support::read_png(&dir.join(&id).join("msaa.png"));
        let (_, _, noaa) = support::read_png(&dir.join(&id).join("noaa.png"));
        assert_eq!((rw as usize, rh as usize), (w, h), "{id}");
        let edges = msaa_edges(&reference, &noaa, w, h);
        let s = bar_3d(&reference, &ours, &edges, &object, w, h);
        let line = format!(
            "{id:<40} ≤2 CV {:>8.4}%  max {:>3}  max outside edges {:>3}  SSIM {:.5}  ({} edge px)",
            100.0 * s.fraction_within_2,
            s.max_cv,
            s.max_cv_outside_edges,
            s.object_ssim.unwrap_or(f64::NAN),
            s.excluded
        );
        eprintln!("{line}");
        if !s.pass() {
            failed.push(line);
            if let Some(d) = &diffs {
                std::fs::create_dir_all(d).unwrap();
                support::write_png(&d.join(format!("{id}-engine.png")), w as u32, h as u32, &ours);
                support::write_png(&d.join(format!("{id}-three.png")), w as u32, h as u32, &reference);
            }
        }
        pool.give(out.color);
        if let Some(d) = out.depth {
            pool.give(d);
        }
    }
    assert!(failed.is_empty(), "outside the 3D bar:\n{}", failed.join("\n"));
}
