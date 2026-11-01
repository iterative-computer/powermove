//! Renders Studio's reference scene and writes what a reference renderer
//! needs to render the same thing (`reference/compare.py` runs Mitsuba 3):
//!
//!   studio-reference --out DIR [--size 480x270] [--spp 1024] [--no-rt] [--no-caustics] [--clamp N]
//!
//! Writes `studio.pfm` (linear radiance), `scene.json` and `studio.png`.

use pm_studio::gpu::{Renderer, write_png};
use pm_studio::math::{Vec3, v3};
use pm_studio::mesh::{self, Mesh};
use pm_studio::scene::{Camera, Material, Object, Scene, Softbox};
use std::io::Write;
use std::path::PathBuf;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

/// The scene both renderers draw: four spheres (rough plastic, rough gold,
/// clear glass, polished chrome) on a grey floor under one softbox, in the
/// dark. Plain materials only, so both renderers define them the same way.
fn reference(matte: bool) -> (Scene, serde_json::Value) {
    let mut first = (v3(-150.0, 34.0, 0.0), Material::dielectric(v3(0.80, 0.30, 0.10), 0.5), "plastic");
    if matte {
        // `--matte`: the first sphere without its specular layer (isolates diffuse).
        first.1.specular = 0.0;
        first.2 = "matte";
    }
    let spheres = [
        first,
        (v3(-50.0, 34.0, 0.0), Material::metal(v3(1.0, 0.78, 0.34), 0.3), "gold"),
        (v3(50.0, 34.0, 0.0), Material::glass(v3(1.0, 1.0, 1.0), 0.0), "glass"),
        (v3(150.0, 34.0, 0.0), Material::metal(v3(0.95, 0.95, 0.95), 0.05), "chrome"),
    ];
    let floor_albedo = v3(0.5, 0.5, 0.5);
    let mut floor = Mesh::default();
    let s = 1200.0;
    floor.positions = vec![v3(-s, 0.0, -s), v3(s, 0.0, -s), v3(s, 0.0, s), v3(-s, 0.0, s)];
    floor.normals = vec![Vec3::Y; 4];
    floor.uvs = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    floor.indices = vec![[0, 2, 1], [0, 3, 2]];
    let mut materials = vec![Material::dielectric(floor_albedo, 1.0)];
    // The floor is matte: no specular layer.
    materials[0].specular = 0.0;
    let mut objects = vec![Object { mesh: floor, material: 0 }];
    for (i, (c, m, _)) in spheres.iter().enumerate() {
        materials.push(*m);
        objects.push(Object { mesh: mesh::sphere(*c, 34.0, 128, 256), material: i + 1 });
    }
    let light = Softbox::aimed(v3(0.0, 30.0, 0.0), -30.0, 55.0, 450.0, 260.0, 160.0, Vec3::splat(6.0));
    let camera = Camera::lens(v3(0.0, 150.0, 520.0), v3(0.0, 25.0, 0.0), 50.0, 0.0);
    let spec = serde_json::json!({
        "camera": { "position": camera.position.to_array(), "target": camera.target.to_array(), "fov_y": camera.fov },
        "light": { "center": light.center.to_array(), "u": light.u.to_array(), "v": light.v.to_array(), "radiance": light.radiance.to_array(), "rim": [1.0, 0.82] },
        "floor": { "size": s, "albedo": floor_albedo.to_array() },
        "spheres": spheres.iter().map(|(c, m, kind)| serde_json::json!({
            "kind": kind, "center": c.to_array(), "radius": 34.0, "color": m.base.to_array(), "roughness": m.roughness, "ior": m.ior,
        })).collect::<Vec<_>>(),
    });
    let scene = Scene { objects, materials, lights: vec![light], camera, backdrop: Vec3::ZERO, environment: (Vec3::ZERO, Vec3::ZERO), exposure: 1.0, shadow_catcher: false, extra_lights: Vec::new(), layers: None };
    (scene, spec)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = PathBuf::from(arg(&args, "--out").unwrap_or_else(|| "studio-reference".into()));
    std::fs::create_dir_all(&out).expect("output directory");
    let (w, h) = arg(&args, "--size")
        .and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap_or(480), b.parse().unwrap_or(270))))
        .unwrap_or((480u32, 270u32));
    let spp: u32 = arg(&args, "--spp").and_then(|s| s.parse().ok()).unwrap_or(1024);
    let (scene, mut spec) = reference(args.iter().any(|a| a == "--matte"));
    let mut r = Renderer::new(w, h).expect("GPU");
    if args.iter().any(|a| a == "--no-rt") {
        r.hardware_rt = false;
    }
    r.set_scene(&scene, &scene.prepare());
    if args.iter().any(|a| a == "--no-caustics") {
        r.caustics = false;
    }
    let clamp: u32 = arg(&args, "--clamp").and_then(|s| s.parse().ok()).unwrap_or(1 << 30);
    let t = std::time::Instant::now();
    // No firefly clamp: a reference comparison wants the unbiased estimate.
    r.trace(spp, 10, true, clamp);
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    let px = r.radiance();
    spec["size"] = serde_json::json!([w, h]);
    spec["spp"] = serde_json::json!(spp);
    spec["studio_ms"] = serde_json::json!(ms);
    spec["adapter"] = serde_json::json!(r.adapter);
    std::fs::write(out.join("scene.json"), serde_json::to_string_pretty(&spec).unwrap()).unwrap();
    // PFM: little-endian RGB floats, bottom row first.
    let mut f = std::io::BufWriter::new(std::fs::File::create(out.join("studio.pfm")).unwrap());
    write!(f, "PF\n{w} {h}\n-1.0\n").unwrap();
    for y in (0..h).rev() {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            for c in 0..3 {
                f.write_all(&px[i + c].to_le_bytes()).unwrap();
            }
        }
    }
    let (png, _) = r.finish(0);
    write_png(&out.join("studio.png"), w, h, &png).unwrap();
    eprintln!("studio: {w}×{h}, {spp} spp in {ms:.0} ms on {}", r.adapter);
}
