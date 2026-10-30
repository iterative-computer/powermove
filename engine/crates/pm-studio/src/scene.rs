//! Studio scenes: objects with layered materials, softbox lights, a seamless
//! backdrop with a shadow-catching floor, and a physical camera.

use crate::bvh;
use crate::math::{Vec3, srgb_hex, v3};
use crate::mesh::{self, Mesh};

/// The layered material (an OpenPBR subset): base metal/dielectric, specular
/// GGX, a clear coat, transmission, and emission (optionally from the screen
/// texture).
#[derive(Clone, Copy, Debug)]
pub struct Material {
    pub base: Vec3,
    pub metallic: f32,
    pub roughness: f32,
    pub coat: f32,
    pub coat_roughness: f32,
    pub transmission: f32,
    pub ior: f32,
    /// Scales the dielectric base reflection (0 for a matte panel under glass).
    pub specular: f32,
    pub emission: Vec3,
    /// Emission is multiplied by the screen composition texture.
    pub screen: bool,
    /// Fabric-like sheen (0–1).
    pub sheen: f32,
    /// Thin-film coating thickness in nanometres (0: none) — iridescence.
    pub film_nm: f32,
    /// How much more blue bends than red in transmission (0–1).
    pub dispersion: f32,
    /// Water droplets on the surface (0–1).
    pub droplets: f32,
    /// Brushed-metal stretch of the highlight along the tangent (-1–1).
    pub anisotropy: f32,
    /// Subsurface mean free path in scene units (0: opaque).
    pub subsurface: f32,
}

impl Material {
    pub fn dielectric(base: Vec3, roughness: f32) -> Self {
        Material { base, metallic: 0.0, roughness, coat: 0.0, coat_roughness: 0.03, transmission: 0.0, ior: 1.5, specular: 1.0, emission: Vec3::ZERO, screen: false, sheen: 0.0, film_nm: 0.0, dispersion: 0.0, droplets: 0.0, anisotropy: 0.0, subsurface: 0.0 }
    }
    pub fn metal(base: Vec3, roughness: f32) -> Self {
        Material { metallic: 1.0, ..Self::dielectric(base, roughness) }
    }
    /// Anodised aluminium: a slightly rough metal (bead-blasted finish).
    pub fn anodized(hex: &str) -> Self {
        Self::metal(srgb_hex(hex), 0.34)
    }
    /// Cover glass over a black display: a dark dielectric with a mirror coat.
    pub fn cover_glass() -> Self {
        Material { coat: 1.0, coat_roughness: 0.012, specular: 0.0, ..Self::dielectric(v3(0.003, 0.003, 0.004), 0.6) }
    }
    /// A lit display under its glass: emission from the screen composition.
    pub fn display(nits: f32) -> Self {
        Material { emission: Vec3::splat(nits), screen: true, ..Self::cover_glass() }
    }
    pub fn glass(tint: Vec3, roughness: f32) -> Self {
        Material { transmission: 1.0, ..Self::dielectric(tint, roughness) }
    }
}

/// A rectangular softbox: `center`, two half-extent vectors spanning its face,
/// emitting toward `u × v`.
#[derive(Clone, Copy, Debug)]
pub struct Softbox {
    pub center: Vec3,
    pub u: Vec3,
    pub v: Vec3,
    pub radiance: Vec3,
}

impl Softbox {
    /// A softbox of `width` × `height` mm at `distance` from `target`, seen
    /// from azimuth/elevation in degrees, facing the target.
    pub fn aimed(target: Vec3, azimuth: f32, elevation: f32, distance: f32, width: f32, height: f32, radiance: Vec3) -> Self {
        let (a, e) = (azimuth.to_radians(), elevation.to_radians());
        let dir = v3(a.sin() * e.cos(), e.sin(), a.cos() * e.cos());
        let center = target + dir * distance;
        let facing = -dir;
        let side = Vec3::Y.cross(facing).normalize();
        let side = if side.length() < 0.5 { v3(1.0, 0.0, 0.0) } else { side };
        let up = facing.cross(side).normalize();
        // u × v must point along `facing`.
        Softbox { center, u: side * (width / 2.0), v: up * (height / 2.0), radiance }
    }
}

impl Softbox {
    /// Light by pointing: a softbox placed so its reflection lands on
    /// `point` (surface normal `normal`) as seen from `eye`. The view ray is
    /// mirrored about the normal and the light sits `distance` along it,
    /// facing back. Bigger `width`/`height` give a softer highlight.
    pub fn for_highlight(point: Vec3, normal: Vec3, eye: Vec3, distance: f32, width: f32, height: f32, radiance: Vec3) -> Self {
        let n = normal.normalize();
        let v = (eye - point).normalize();
        let r = n * (2.0 * v.dot(n)) - v;
        let center = point + r * distance;
        let facing = -r;
        let side = Vec3::Y.cross(facing);
        let side = if side.length() < 1e-3 { v3(1.0, 0.0, 0.0) } else { side.normalize() };
        let up = facing.cross(side).normalize();
        Softbox { center, u: side * (width / 2.0), v: up * (height / 2.0), radiance }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub position: Vec3,
    pub target: Vec3,
    /// Vertical field of view, degrees (from a lens: 2·atan(12 / focal mm)).
    pub fov: f32,
    /// f-number; 0 disables depth of field.
    pub f_stop: f32,
    pub focus: Option<Vec3>,
    pub focal_mm: f32,
}

impl Camera {
    pub fn lens(position: Vec3, target: Vec3, focal_mm: f32, f_stop: f32) -> Self {
        Camera { position, target, fov: 2.0 * (12.0f32 / focal_mm).atan().to_degrees(), f_stop, focus: None, focal_mm }
    }
}

pub struct Object {
    pub mesh: Mesh,
    pub material: usize,
}

pub struct Scene {
    pub objects: Vec<Object>,
    pub materials: Vec<Material>,
    pub lights: Vec<Softbox>,
    pub camera: Camera,
    /// The seamless backdrop as it appears on screen (display-referred sRGB).
    pub backdrop: Vec3,
    /// Studio environment for reflections: (zenith, horizon) radiance.
    pub environment: (Vec3, Vec3),
    pub exposure: f32,
    /// Floor at y = 0 that only catches shadows (Apple-style void).
    pub shadow_catcher: bool,
}

/// What the GPU reads, flattened.
pub struct Prepared {
    /// Corner positions only (what intersection reads), 3 per triangle.
    pub positions: Vec<[f32; 4]>,
    pub tris: Vec<GpuTri>,
    pub nodes: Vec<bvh::WideNode>,
    pub materials: Vec<GpuMaterial>,
    pub lights: Vec<GpuLight>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuTri {
    pub n: [[f32; 4]; 3],
    /// uv0.xy, uv1.xy
    pub uv01: [f32; 4],
    /// uv2.xy, material, object
    pub uv2m: [f32; 4],
    /// Per-triangle tangent (xyz) and handedness (w), for normal maps.
    pub tangent: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuMaterial {
    pub base: [f32; 4],  // rgb, metallic
    pub spec: [f32; 4],  // roughness, coat, coat roughness, transmission
    pub emit: [f32; 4],  // rgb, screen flag
    pub extra: [f32; 4], // ior, specular, opacity
    pub maps: [f32; 4],  // texture layers: color, emissive, orm, normal (-1: none)
    pub layers: [f32; 4], // sheen, thin-film thickness (nm), dispersion, droplets
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuLight {
    pub pos: [f32; 4], // softbox centre, kind 0
    pub u: [f32; 4],   // half extent, area
    pub v: [f32; 4],
    pub radiance: [f32; 4],
}

impl Scene {
    pub fn prepare(&self) -> Prepared {
        let mut corners = Vec::new();
        let mut raw = Vec::new();
        let mut positions_raw = Vec::new();
        for (object, o) in self.objects.iter().enumerate() {
            let m = &o.mesh;
            for t in &m.indices {
                let [a, b, c] = t.map(|i| i as usize);
                corners.push([m.positions[a], m.positions[b], m.positions[c]]);
                positions_raw.push([m.positions[a].to4(0.0), m.positions[b].to4(0.0), m.positions[c].to4(0.0)]);
                raw.push(GpuTri {
                    n: [m.normals[a].to4(0.0), m.normals[b].to4(0.0), m.normals[c].to4(0.0)],
                    uv01: [m.uvs[a][0], m.uvs[a][1], m.uvs[b][0], m.uvs[b][1]],
                    uv2m: [m.uvs[c][0], m.uvs[c][1], o.material as f32, object as f32],
                    tangent: tangent([m.positions[a], m.positions[b], m.positions[c]], [m.uvs[a], m.uvs[b], m.uvs[c]]),
                });
            }
        }
        let (nodes, order) = bvh::build(&corners);
        let tris: Vec<GpuTri> = order.iter().map(|&i| raw[i as usize]).collect();
        let positions = order.iter().flat_map(|&i| positions_raw[i as usize]).collect();
        let nodes = bvh::widen(&nodes);
        let materials = self
            .materials
            .iter()
            .map(|m| GpuMaterial {
                base: m.base.to4(m.metallic),
                spec: [m.roughness.max(0.002), m.coat, m.coat_roughness.max(0.002), m.transmission],
                emit: m.emission.to4(m.subsurface),
                extra: [m.ior, m.specular, 1.0, m.anisotropy],
                // The display's composition is texture layer 0.
                maps: [-1.0, if m.screen { 0.0 } else { -1.0 }, -1.0, -1.0],
                layers: [m.sheen, m.film_nm, m.dispersion, m.droplets],
            })
            .collect();
        let lights = self
            .lights
            .iter()
            .map(|l| GpuLight { pos: l.center.to4(0.0), u: l.u.to4(4.0 * l.u.cross(l.v).length()), v: l.v.to4(0.0), radiance: l.radiance.to4(0.0) })
            .collect();
        Prepared { positions, tris, nodes, materials, lights }
    }

    pub fn triangle_count(&self) -> usize {
        self.objects.iter().map(|o| o.mesh.indices.len()).sum()
    }
}

/// A triangle's tangent (the direction of increasing u) and handedness, from its UVs.
fn tangent(p: [Vec3; 3], uv: [[f32; 2]; 3]) -> [f32; 4] {
    let (e1, e2) = (p[1] - p[0], p[2] - p[0]);
    let (d1, d2) = ([uv[1][0] - uv[0][0], uv[1][1] - uv[0][1]], [uv[2][0] - uv[0][0], uv[2][1] - uv[0][1]]);
    let r = d1[0] * d2[1] - d2[0] * d1[1];
    if r.abs() < 1e-12 {
        return [0.0; 4];
    }
    let t = ((e1 * d2[1]) - (e2 * d1[1])) / r;
    let b = ((e2 * d1[0]) - (e1 * d2[0])) / r;
    let n = e1.cross(e2);
    let handed = if n.cross(t).dot(b) < 0.0 { -1.0 } else { 1.0 };
    let t = t.normalize();
    [t.x, t.y, t.z, handed]
}

/// The finish of the tablet's aluminium.
pub const FINISHES: [(&str, &str); 4] = [("space-gray", "#3B3C40"), ("starlight", "#E3DDD3"), ("blue", "#9FB3CC"), ("purple", "#C6B9D8")];

/// The film's studio: a `#F6F5F8` void, three softboxes and a soft dome.
fn studio(target: Vec3) -> (Vec<Softbox>, (Vec3, Vec3)) {
    let lights = vec![
        Softbox::aimed(target, -38.0, 52.0, 700.0, 520.0, 320.0, Vec3::splat(5.2)),
        Softbox::aimed(target, 62.0, 24.0, 650.0, 260.0, 520.0, Vec3::splat(2.0)),
        Softbox::aimed(target, 140.0, 16.0, 720.0, 900.0, 140.0, Vec3::splat(6.5)),
        // Overhead silk: soft gradients on upward-facing metal, out of the screen's reflection.
        Softbox::aimed(target, 0.0, 89.0, 950.0, 1500.0, 1500.0, Vec3::splat(1.1)),
    ];
    // A dark studio: reflections come from the softboxes, so glass stays black.
    (lights, (Vec3::splat(0.012), Vec3::splat(0.03)))
}

/// Scenes for Phase 0. `t` animates in [0, 1).
pub fn preset(name: &str, finish: &str, t: f32) -> Option<Scene> {
    let hex = FINISHES.iter().find(|(n, _)| *n == finish).map(|f| f.1).unwrap_or(FINISHES[0].1);
    let mut materials = vec![
        Material::anodized(hex),                       // 0 body
        Material::cover_glass(),                       // 1 glass
        Material::display(1.6),                        // 2 display
        Material::metal(srgb_hex("#2A2B2E"), 0.12),    // 3 camera ring
        Material { coat: 1.0, coat_roughness: 0.01, ..Material::dielectric(v3(0.002, 0.002, 0.003), 0.2) }, // 4 lens
        Material { coat: 0.6, coat_roughness: 0.04, ..Material::dielectric(srgb_hex("#F4F4F2"), 0.32) },   // 5 stylus
    ];
    let tab = mesh::tablet();
    let parts = |tilt: f32, turn: f32, at: Vec3| -> Vec<Object> {
        vec![
            Object { mesh: tab.body.placed(tilt, turn, at), material: 0 },
            Object { mesh: tab.glass.placed(tilt, turn, at), material: 1 },
            Object { mesh: tab.display.placed(tilt, turn, at), material: 2 },
            Object { mesh: tab.camera_ring.placed(tilt, turn, at), material: 3 },
            Object { mesh: tab.camera_lens.placed(tilt, turn, at), material: 4 },
        ]
    };
    let turntable = t * std::f32::consts::TAU;
    let (objects, camera, target) = match name {
        // Flat lay, three-quarter view, stylus alongside.
        "hero" => {
            let mut objects = parts(0.0, 0.35 + turntable, Vec3::ZERO);
            objects.push(Object { mesh: mesh::stylus().placed(0.0, 1.92 + turntable, v3(92.0, 0.0, 70.0)), material: 5 });
            let target = v3(0.0, 4.0, 0.0);
            (objects, Camera::lens(v3(0.0, 430.0, 610.0), target, 70.0, 0.0), target)
        }
        // Standing on its long edge, turning: the thin silhouette reveal.
        // Upright on its long edge, display facing +z after a quarter turn: at
        // t = 0 the camera sees only the thin side, then it turns to face it.
        "edge" => {
            let at = v3(0.0, 195.4 / 2.0 + 2.0, 0.0);
            let objects = parts(std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2 - turntable, at);
            (objects, Camera::lens(v3(0.0, at.y + 30.0, 1150.0), at, 85.0, 0.0), at)
        }
        // Macro across the display corner with shallow focus.
        "macro" => {
            let objects = parts(0.0, 0.0, Vec3::ZERO);
            let focus = v3(52.0, 6.3, 84.0);
            let mut cam = Camera::lens(v3(118.0 + 40.0 * t, 46.0, 190.0), v3(40.0, 4.0, 70.0), 100.0, 2.8);
            cam.focus = Some(focus);
            (objects, cam, focus)
        }
        // Face down: the camera bump.
        "back" => {
            let (tilt, turn, at) = (std::f32::consts::PI, 0.4 + 0.3 * t, v3(0.0, 6.3, 0.0));
            let objects = parts(tilt, turn, at);
            // The camera bump's centre, through the same placement as the meshes.
            let bump = v3(134.8 / 2.0 - 13.0, -0.7, -(195.4 / 2.0 - 13.0)).rotate_x(tilt).rotate_y(turn) + at;
            let mut cam = Camera::lens(bump + v3(70.0, 95.0, 150.0), bump - v3(12.0, 0.0, 8.0), 90.0, 4.0);
            cam.focus = Some(bump);
            (objects, cam, bump)
        }
        // Reflections stress test: chrome, glass and a lacquered panel.
        "chrome" => {
            materials.push(Material::metal(v3(0.97, 0.97, 0.98), 0.02)); // 6
            materials.push(Material::glass(v3(1.0, 1.0, 1.0), 0.0)); // 7
            materials.push(Material { coat: 1.0, coat_roughness: 0.01, ..Material::dielectric(srgb_hex("#B3122E"), 0.45) }); // 8
            let objects = vec![
                Object { mesh: mesh::sphere(v3(-70.0, 45.0, 0.0), 45.0, 96, 192), material: 6 },
                Object { mesh: mesh::sphere(v3(40.0, 35.0, 30.0), 35.0, 96, 192), material: 7 },
                Object { mesh: tab.body.placed(-1.2, 0.5, v3(30.0, 0.0, -110.0)), material: 8 },
            ];
            let target = v3(-10.0, 35.0, 0.0);
            (objects, Camera::lens(v3(30.0, 140.0, 430.0), target, 60.0, 0.0), target)
        }
        // Phase 4 materials: sheen, thin film, dispersive glass, plain glass.
        "materials" => {
            materials.push(Material { sheen: 1.0, ..Material::dielectric(srgb_hex("#3E4A7A"), 0.75) }); // 6 fabric
            materials.push(Material { film_nm: 520.0, ..Material::metal(srgb_hex("#C9CCD2"), 0.08) }); // 7 thin film
            materials.push(Material { dispersion: 1.0, ..Material::glass(v3(1.0, 1.0, 1.0), 0.0) }); // 8 dispersive
            materials.push(Material::glass(v3(0.92, 0.97, 1.0), 0.0)); // 9 glass
            materials.push(Material { anisotropy: 0.85, ..Material::metal(srgb_hex("#C4C7CC"), 0.3) }); // 10 brushed
            materials.push(Material { subsurface: 6.0, ..Material::dielectric(srgb_hex("#F2D9B8"), 0.45) }); // 11 wax
            let objects = (0..6)
                .map(|i| Object { mesh: mesh::sphere(v3(-200.0 + 80.0 * i as f32, 34.0, 0.0), 34.0, 96, 192), material: 6 + i })
                .collect();
            let target = v3(0.0, 34.0, 0.0);
            (objects, Camera::lens(v3(0.0, 130.0, 560.0), target, 60.0, 0.0), target)
        }
        // The water shot: droplets beading on the display glass.
        "wet" => {
            materials[2].droplets = 0.55;
            materials[1].droplets = 0.55;
            let objects = parts(0.0, 0.0, Vec3::ZERO);
            let focus = v3(10.0, 6.3, 30.0);
            let mut cam = Camera::lens(v3(60.0, 140.0, 210.0), v3(0.0, 4.0, 10.0), 85.0, 4.0);
            cam.focus = Some(focus);
            (objects, cam, focus)
        }
        _ => return None,
    };
    let (mut lights, environment) = studio(target);
    // The back: a highlight painted onto the aluminium beside the camera bump.
    if name == "back" {
        let up = v3(0.0, 1.0, 0.0);
        lights.push(Softbox::for_highlight(target + v3(-30.0, 0.0, 20.0), up, camera.position, 600.0, 700.0, 380.0, Vec3::splat(3.0)));
    }
    Some(Scene { objects, materials, lights, camera, backdrop: v3(0xF6 as f32 / 255.0, 0xF5 as f32 / 255.0, 0xF8 as f32 / 255.0), environment, exposure: 1.0, shadow_catcher: true })
}

/// An original abstract wallpaper for the display: soft ribbons of blue and
/// violet over near-black. Linear RGBA8 (written as sRGB), `w` × `h`.
pub fn screen_wallpaper(w: u32, h: u32, t: f32) -> Vec<u8> {
    let mut out = vec![0u8; (w * h * 4) as usize];
    let enc = |v: f32| -> u8 {
        let v = v.clamp(0.0, 1.0);
        let s = if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        (s * 255.0 + 0.5) as u8
    };
    for y in 0..h {
        for x in 0..w {
            let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
            let mut c = v3(0.006, 0.006, 0.012);
            for k in 0..5 {
                let k = k as f32;
                let phase = t * std::f32::consts::TAU + k * 1.3;
                let centre = 0.15 + 0.17 * k + 0.06 * (v * 6.0 + phase).sin();
                let width = 0.035 + 0.012 * (v * 3.0 + k).cos();
                let d = (u - centre).abs() / width;
                let glow = (-d * d).exp();
                let hue = v3(0.10 + 0.08 * k, 0.18 + 0.06 * (k * 2.1).sin().abs(), 0.95);
                c = c + hue * glow * (0.65 + 0.35 * (v * 9.0 + phase).sin());
            }
            let i = ((y * w + x) * 4) as usize;
            out[i] = enc(c.x);
            out[i + 1] = enc(c.y);
            out[i + 2] = enc(c.z);
            out[i + 3] = 255;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_highlight_light_reflects_into_the_eye() {
        let point = v3(10.0, 0.0, -5.0);
        let eye = v3(0.0, 300.0, 400.0);
        let light = Softbox::for_highlight(point, Vec3::Y, eye, 500.0, 100.0, 100.0, Vec3::splat(1.0));
        // Mirror the light's direction about the normal: it must point at the eye.
        let to_light = (light.center - point).normalize();
        let mirrored = Vec3::Y * (2.0 * to_light.dot(Vec3::Y)) - to_light;
        assert!((mirrored - (eye - point).normalize()).length() < 1e-4);
        // And the softbox faces the point.
        let facing = light.u.cross(light.v).normalize();
        assert!(facing.dot((point - light.center).normalize()) > 0.999);
    }

    #[test]
    fn every_preset_builds_with_lights_and_a_bvh() {
        for name in ["hero", "edge", "macro", "back", "chrome"] {
            let s = preset(name, "starlight", 0.3).unwrap();
            let p = s.prepare();
            assert_eq!(p.tris.len(), s.triangle_count(), "{name}");
            assert_eq!(p.positions.len(), 3 * p.tris.len());
            assert!(!p.nodes.is_empty() && !p.lights.is_empty());
        }
        assert!(preset("nope", "starlight", 0.0).is_none());
    }

    #[test]
    fn studio_softboxes_face_the_subject() {
        let target = v3(0.0, 5.0, 0.0);
        let (lights, _) = studio(target);
        for l in lights {
            let facing = l.u.cross(l.v).normalize();
            assert!(facing.dot((target - l.center).normalize()) > 0.99);
        }
    }
}
