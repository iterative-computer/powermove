//! Immutable kernel → Studio geometry/resource adapter. Geometry is cached by
//! complete immutable source bytes, separately from animated world transforms.
use super::*;
use pm_studio::{
    math::{Vec3, srgb_hex, v3},
    mesh::Mesh,
    scene::{Camera, GpuLight, GpuMaterial, Material, Scene, Softbox},
};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub struct DocumentSource {
    snapshot: pm_kernel::Snapshot,
    media: pm_export::render::MediaPaths,
    meshes: Mutex<HashMap<String, Arc<Mesh>>>,
    resolved: Mutex<Option<(SceneRequest, Arc<pm_scene3d::service::ResolvedScene>)>>,
}
fn val(v: &Value) -> &Value {
    v.get("v").unwrap_or(v)
}
fn n(p: &Value, k: &str, d: f32) -> f32 {
    val(&p[k]).as_f64().map_or(d, |v| v as f32)
}
fn point(p: &Value, prefix: &str) -> Vec3 {
    v3(
        n(p, &format!("{prefix}X"), 0.),
        n(p, &format!("{prefix}Y"), 0.),
        n(p, &format!("{prefix}Z"), 0.),
    )
}
fn xyz(p: &Value) -> Vec3 {
    v3(n(p, "x", 0.), n(p, "y", 0.), n(p, "z", 0.))
}
fn color(p: &Value, key: &str, default: &str) -> Vec3 {
    srgb_hex(val(&p[key]).as_str().unwrap_or(default))
}
impl DocumentSource {
    pub fn new(document: Arc<pm_model::Document>, media: pm_export::render::MediaPaths) -> Self {
        Self {
            snapshot: pm_kernel::Snapshot {
                doc_id: 0,
                revision: document.revision(),
                document,
            },
            media,
            meshes: Mutex::default(),
            resolved: Mutex::default(),
        }
    }
    fn resolve(
        &self,
        request: &SceneRequest,
    ) -> Result<Arc<pm_scene3d::service::ResolvedScene>, String> {
        if request.revision != self.snapshot.revision {
            return Err("Studio snapshot revision changed".into());
        }
        let mut cached = self
            .resolved
            .lock()
            .map_err(|_| "Studio scene cache poisoned")?;
        if let Some((key, scene)) = cached.as_ref()
            && key == request
        {
            return Ok(scene.clone());
        }
        let scene = Arc::new(
            pm_kernel::commands::scene3d::resolve_scene(
                &self.snapshot,
                Some(&request.composition),
                request.ticks as f64 / pm_protocol::common::TICKS_PER_SECOND as f64,
            )
            .map_err(|e| e.to_string())?,
        );
        *cached = Some((request.clone(), scene.clone()));
        Ok(scene)
    }
    fn mesh(&self, source: &Value) -> Result<Arc<Mesh>, String> {
        let key = serde_json::to_string(source).map_err(|e| e.to_string())?;
        let mut meshes = self
            .meshes
            .lock()
            .map_err(|_| "Studio geometry cache poisoned")?;
        if let Some(mesh) = meshes.get(&key) {
            return Ok(mesh.clone());
        }
        // Share immutable derived geometry across snapshot/provider replacement.
        // Weak entries keep neither old documents nor mesh data alive.
        static SHARED: std::sync::OnceLock<Mutex<HashMap<String, std::sync::Weak<Mesh>>>> =
            std::sync::OnceLock::new();
        let shared = SHARED.get_or_init(Default::default);
        if let Some(mesh) = shared
            .lock()
            .map_err(|_| "Studio shared mesh cache poisoned")?
            .get(&key)
            .and_then(std::sync::Weak::upgrade)
        {
            meshes.insert(key, mesh.clone());
            return Ok(mesh);
        }
        if let Some(id) = source["assetId"].as_str() {
            let path = self
                .media
                .paths
                .get(id)
                .ok_or_else(|| format!("3D model geometry is not loaded: {id}"))?;
            // Imported models need their per-part material/pose conversion; never
            // replace unresolved geometry by the old orange placeholder.
            return Err(format!(
                "Studio imported model adapter unavailable for {}",
                path.extension().and_then(|x| x.to_str()).unwrap_or("model")
            ));
        }
        let value = pm_scene3d::to_model(source);
        let pm_model::Value::Object(map) = value else {
            return Err("Invalid Studio source".into());
        };
        let g = pm_render::scene3d::model::source_of(&map)?.geometry();
        let mesh = Arc::new(Mesh {
            positions: g
                .positions
                .chunks_exact(3)
                .map(|p| v3(p[0], p[1], p[2]))
                .collect(),
            normals: g
                .normals
                .chunks_exact(3)
                .map(|p| v3(p[0], p[1], p[2]))
                .collect(),
            uvs: g.uvs.chunks_exact(2).map(|p| [p[0], p[1]]).collect(),
            indices: if g.indices.is_empty() {
                (0..g.positions.len() as u32 / 3).collect::<Vec<_>>()
            } else {
                g.indices
            }
            .chunks_exact(3)
            .map(|t| [t[0], t[1], t[2]])
            .collect(),
        });
        let mut cache = shared
            .lock()
            .map_err(|_| "Studio shared mesh cache poisoned")?;
        cache.retain(|_, mesh| mesh.strong_count() > 0);
        if cache.len() >= 512 {
            cache.clear();
        }
        cache.insert(key.clone(), Arc::downgrade(&mesh));
        meshes.insert(key, mesh.clone());
        Ok(mesh)
    }
}
impl pm_render::scene3d::provider::Source for DocumentSource {
    fn final_samples(&self, request: &SceneRequest) -> Result<u32, String> {
        Ok(self.resolve(request)?.scene["environment"]["samples"]
            .as_u64()
            .unwrap_or(64) as u32)
    }
}
impl SceneSource for DocumentSource {
    fn scene(&self, request: &SceneRequest) -> Result<SceneInput, String> {
        let r = self.resolve(request)?;
        let c = &r.scene["camera"];
        if c["projection"] != "perspective" {
            return Err(
                "Orthographic cameras draw in Fast; Studio Fast adapter is not attached".into(),
            );
        }
        let cp = &c["p"];
        let eye = r
            .camera_position
            .map_or_else(|| xyz(cp), |p| v3(p[0] as f32, p[1] as f32, p[2] as f32));
        let target = r
            .camera_id
            .as_ref()
            .and_then(|id| r.targets.get(id))
            .map_or_else(
                || point(cp, "target"),
                |p| v3(p[0] as f32, p[1] as f32, p[2] as f32),
            );
        let fov = 2. * ((n(cp, "fov", 45.).to_radians() / 2.).tan() / n(cp, "zoom", 1.)).atan();
        let e = &r.scene["environment"];
        let ep = &e["p"];
        let env = color(ep, "ambientColor", "#FFFFFF") * n(ep, "ambient", 0.5);
        let mut scene = Scene {
            objects: vec![],
            materials: vec![],
            lights: vec![],
            camera: Camera {
                position: eye,
                target,
                fov: fov.to_degrees(),
                f_stop: n(cp, "fStop", 0.),
                focus: (n(cp, "focusDistance", 0.) > 0.)
                    .then(|| eye + (target - eye).normalize() * n(cp, "focusDistance", 0.)),
                focal_mm: 0.12 / (n(cp, "fov", 45.).to_radians() / 2.).tan(),
            },
            backdrop: Vec3::ZERO,
            environment: (env, env * 0.675),
            exposure: n(ep, "exposure", 1.),
            shadow_catcher: e["shadowCatcher"] == true,
            extra_lights: vec![],
            layers: None,
        };
        let mut instances = vec![];
        let mut materials: Vec<GpuMaterial> = vec![];
        for object in r.scene["objects"]
            .as_array()
            .ok_or("Studio scene has no objects")?
        {
            let id = object["id"].as_str().ok_or("Studio object has no ID")?;
            if val(&object["p"]["visible"]) == false {
                continue;
            }
            if object["material"]["maps"]
                .as_object()
                .is_some_and(|m| m.values().any(|v| !v.is_null()))
                || !object["slots"].as_array().is_none_or(Vec::is_empty)
                || object["material"]
                    .get("screen")
                    .is_some_and(|v| !v.is_null())
            {
                return Err("Studio material maps/slots/screens require resolved resources".into());
            }
            let p = &object["material"]["p"];
            let m = Material {
                metallic: n(p, "metalness", 0.1),
                coat: n(p, "clearcoat", 0.),
                coat_roughness: n(p, "clearcoatRoughness", 0.03),
                transmission: n(p, "transmission", 0.),
                ior: n(p, "ior", 1.5),
                specular: n(p, "specular", 1.),
                emission: color(p, "emissive", "#000000") * n(p, "emissiveIntensity", 1.),
                sheen: n(p, "sheen", 0.),
                film_nm: n(p, "iridescence", 0.),
                dispersion: n(p, "dispersion", 0.),
                droplets: n(p, "droplets", 0.),
                anisotropy: n(p, "anisotropy", 0.),
                subsurface: n(p, "subsurface", 0.),
                ..Material::dielectric(color(p, "color", "#C7C4FF"), n(p, "roughness", 0.4))
            };
            scene.materials.push(m);
            // Keep material opacity in transport; layer opacity belongs to the
            // outer compositor and must not be multiplied twice.
            let mut gpu = scene.gpu_materials().pop().unwrap();
            gpu.extra[2] = n(p, "opacity", 1.);
            materials.push(gpu);
            instances.push(MeshInstance {
                layer: id.into(),
                mesh: self.mesh(&object["source"])?,
                material: materials.len() - 1,
                world: Transform(
                    r.model_worlds
                        .get(id)
                        .ok_or("Studio object has no evaluated world transform")?
                        .map(|x| x as f32),
                ),
            });
        }
        let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for instance in &instances {
            let (a, z) = instance.mesh.bounds();
            for x in [a.x, z.x] {
                for y in [a.y, z.y] {
                    for z in [a.z, z.z] {
                        let p = instance.world.point(v3(x, y, z));
                        low = low.min(p);
                        high = high.max(p);
                    }
                }
            }
        }
        let size = if instances.is_empty() {
            1.
        } else {
            (high - low).length()
        };
        let radius = (size * 0.004).max(0.001);
        for light in r.scene["lights"]
            .as_array()
            .ok_or("Studio scene has no lights")?
        {
            let p = &light["p"];
            if val(&p["visible"]) == false {
                continue;
            }
            let id = light["id"].as_str().ok_or("Light has no ID")?;
            let at = r
                .light_worlds
                .get(id)
                .map_or_else(|| xyz(p), |m| v3(m[12] as f32, m[13] as f32, m[14] as f32));
            let aim = r.targets.get(id).map_or_else(
                || point(p, "target"),
                |p| v3(p[0] as f32, p[1] as f32, p[2] as f32),
            );
            let radiance = color(p, "color", "#FFFFFF") * n(p, "intensity", 3.);
            let direction = (aim - at).normalize();
            match light["type"].as_str().unwrap_or("sun") {
                "area" => {
                    let side = if direction.cross(Vec3::Y).length() < 0.001 {
                        v3(1., 0., 0.)
                    } else {
                        direction.cross(Vec3::Y).normalize()
                    };
                    scene.lights.push(Softbox {
                        center: at,
                        u: side * (n(p, "width", 2.) / 2.),
                        v: direction.cross(side) * (n(p, "height", 1.2) / 2.),
                        radiance,
                    });
                }
                "sun" => scene.extra_lights.push(GpuLight::sun(
                    -direction,
                    1.5_f32.to_radians(),
                    radiance
                        * (1. / (2. * std::f32::consts::PI * (1. - 1.5_f32.to_radians().cos()))),
                )),
                "point" => scene.extra_lights.push(GpuLight::sphere(
                    at,
                    radius,
                    radiance * (1. / (std::f32::consts::PI * radius * radius)),
                )),
                "spot" => scene.extra_lights.push(GpuLight::spot(
                    at,
                    radius,
                    direction,
                    n(p, "angle", 45.).to_radians().cos(),
                    (n(p, "angle", 45.) * (1. - n(p, "penumbra", 0.4)))
                        .to_radians()
                        .cos(),
                    radiance * (1. / (std::f32::consts::PI * radius * radius)),
                )),
                _ => return Err("Unsupported Studio light".into()),
            }
        }
        let basis = r.camera_up.map(|up| {
            let f = (target - eye).normalize();
            let right = f
                .cross(v3(up[0] as f32, up[1] as f32, up[2] as f32))
                .normalize();
            [f, right, right.cross(f)]
        });
        let floor_y = if e["shadowCatcher"] == true && !instances.is_empty() {
            low.y
        } else {
            0.
        };
        Ok(SceneInput {
            scene,
            instances,
            materials: Some(materials),
            maps: None,
            view: View {
                basis,
                ground: Some(env * 0.35),
                floor_y,
                shadows: e["shadows"] != false,
            },
        })
    }
}
