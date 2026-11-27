//! Test-only deterministic geometry for native export benchmark projects.
use pm_studio::{
    math::{Vec3, v3},
    mesh::{self, Mesh},
    scene::{self, Material, Object},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::path::PathBuf::from(std::env::args().nth(1).ok_or("output directory required")?);
    std::fs::create_dir_all(&out)?;
    for name in ["hero", "chrome", "product-shot"] {
        let mut s = scene::preset(
            if name == "product-shot" {
                "chrome"
            } else {
                name
            },
            "starlight",
            0.,
        )
        .unwrap();
        if name == "product-shot" {
            let profile = [
                [0., 0.],
                [25., 0.],
                [32., 6.],
                [34., 80.],
                [32., 100.],
                [18., 114.],
                [17., 140.],
                [0., 140.],
            ];
            let mut m = Mesh::default();
            for (row, p) in profile.iter().enumerate() {
                let a = profile[row.saturating_sub(1)];
                let b = profile[(row + 1).min(profile.len() - 1)];
                for i in 0..=128 {
                    let angle = std::f32::consts::TAU * i as f32 / 128.;
                    let (c, s) = (angle.cos(), angle.sin());
                    m.positions.push(v3(p[0] * c, p[1], p[0] * s));
                    m.normals
                        .push(v3((b[1] - a[1]) * c, a[0] - b[0], (b[1] - a[1]) * s).normalize());
                    m.uvs.push([i as f32 / 128., row as f32 / 8.]);
                }
            }
            for row in 0..profile.len() - 1 {
                for i in 0..128 {
                    let a = (row * 129 + i) as u32;
                    let b = a + 129;
                    m.indices.extend([[a, b, a + 1], [a + 1, b, b + 1]]);
                }
            }
            s.objects = vec![
                Object {
                    mesh: m,
                    material: 0,
                },
                Object {
                    mesh: mesh::sphere(v3(0., 143., 0.), 18., 24, 48),
                    material: 1,
                },
            ];
            s.materials = vec![
                Material {
                    coat: 1.,
                    ..Material::dielectric(v3(0.45, 0.012, 0.026), 0.45)
                },
                Material::metal(Vec3::splat(0.12), 0.3),
            ];
            s.camera = scene::Camera::lens(v3(170., 140., 420.), v3(0., 70., 0.), 60., 0.);
            for light in &mut s.lights {
                light.center = light.center + v3(10., 35., 0.);
            }
        }
        let dir = out.join(name);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("scene.json"), serde_json::to_vec(&s)?)?;
        for (i, obj) in s.objects.iter().enumerate() {
            use std::fmt::Write;
            let mut text = String::new();
            for p in &obj.mesh.positions {
                writeln!(text, "v {} {} {}", p.x, p.y, p.z)?;
            }
            for n in &obj.mesh.normals {
                writeln!(text, "vn {} {} {}", n.x, n.y, n.z)?;
            }
            for t in &obj.mesh.uvs {
                writeln!(text, "vt {} {}", t[0], t[1])?;
            }
            for f in &obj.mesh.indices {
                let [a, b, c] = f.map(|x| x + 1);
                writeln!(text, "f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}")?;
            }
            std::fs::write(dir.join(format!("part-{i}.obj")), text)?;
        }
        println!("{} {} objects", name, s.objects.len());
    }
    Ok(())
}
