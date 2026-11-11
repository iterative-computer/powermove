//! Immutable kernel → Studio geometry/resource adapter. Geometry is cached by
//! complete immutable source bytes, separately from animated world transforms.
use super::*;
use pm_studio3d_engine::MaterialExt;
use pm_studio3d_engine::{
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
    resources: Mutex<HashMap<String, Arc<pm_scene3d::render::Resource>>>,
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
            resources: Mutex::default(),
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
    fn resource(&self, id: &str) -> Result<Arc<pm_scene3d::render::Resource>, String> {
        let mut resources = self
            .resources
            .lock()
            .map_err(|_| "Studio resource cache poisoned")?;
        if let Some(resource) = resources.get(id) {
            return Ok(resource.clone());
        }
        let path = self
            .media
            .paths
            .get(id)
            .ok_or_else(|| format!("3D model geometry is not loaded: {id}"))?;
        let mut file = std::fs::File::open(path).map_err(|e| format!("3D model resource: {e}"))?;
        use std::io::Read;
        let mut bytes = vec![];
        file.by_ref()
            .take(pm_scene3d::import::MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        // Owned imports are copied to content-addressed or `.bin` caches.
        // Their saved format/name describes the decoder; it never grants a
        // file read. The approved media path remains the only byte source.
        let asset = self
            .snapshot
            .document
            .root()
            .and_then(|root| root.get("assets"))
            .and_then(|assets| assets.get(id));
        let name = asset
            .and_then(|asset| asset.get("format"))
            .and_then(|format| {
                format
                    .to_value()
                    .as_str()
                    .map(|format| format!("resource.{format}"))
            })
            .or_else(|| {
                asset
                    .and_then(|asset| asset.get("name"))
                    .and_then(|name| name.to_value().as_str().map(str::to_owned))
            });
        let name = name
            .as_deref()
            .or_else(|| path.to_str())
            .ok_or("Invalid model resource path")?;
        let resource =
            pm_scene3d::render::Resource::load(name, &bytes).map_err(|e| e.to_string())?;
        resources.insert(id.into(), resource.clone());
        Ok(resource)
    }
    fn mesh(&self, source: &Value) -> Result<Arc<Mesh>, String> {
        let key = serde_json::to_string(source).map_err(|e| e.to_string())?;
        self.cached_mesh(key, || {
            let value = pm_scene3d::to_model(source);
            let pm_model::Value::Object(map) = value else {
                return Err("Invalid Studio source".into());
            };
            Ok(convert_mesh(
                &pm_render::scene3d::model::source_of(&map)?.geometry(),
            ))
        })
    }
    fn cached_mesh(
        &self,
        key: String,
        build: impl FnOnce() -> Result<Mesh, String>,
    ) -> Result<Arc<Mesh>, String> {
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
        let mesh = Arc::new(build()?);
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
impl pm_studio3d_engine::compositor::Source for DocumentSource {
    fn splat_layers(
        &self,
        request: &SceneRequest,
    ) -> Result<Vec<pm_studio3d_engine::splats::Layer>, String> {
        let resolved = self.resolve(request)?;
        let mut layers = vec![];
        for object in resolved.scene["objects"]
            .as_array()
            .ok_or("Studio scene has no objects")?
        {
            if val(&object["p"]["visible"]) == false {
                continue;
            }
            let Some(asset) = object["source"]["assetId"].as_str() else {
                continue;
            };
            let resource = self.resource(asset)?;
            let pm_scene3d::import::Model::Splats(data) = &resource.model else {
                continue;
            };
            let id = object["id"].as_str().ok_or("Splat layer has no ID")?;
            let (center, radius) = data.normalization().map_err(|e| e.to_string())?;
            let math = pm_scene3d::render::gltf::math::compose(
                center.map(|v| -v),
                [0., 0., 0., 1.],
                [1.; 3],
            );
            let scale = pm_scene3d::render::gltf::math::compose(
                [0.; 3],
                [0., 0., 0., 1.],
                [1. / radius, -1. / radius, -1. / radius],
            );
            let world = pm_scene3d::render::gltf::math::multiply(
                resolved
                    .model_worlds
                    .get(id)
                    .ok_or("Splat transform unavailable")?,
                &pm_scene3d::render::gltf::math::multiply(&scale, &math),
            );
            let scene = self.scene(request)?;
            layers.push(pm_studio3d_engine::splats::Layer {
                id: id.into(),
                data: Arc::new(
                    data.splats
                        .iter()
                        .map(|g| pm_studio3d_engine::splats::Gaussian {
                            center: g.center,
                            covariance: g.covariance,
                            rgba: g.rgba,
                        })
                        .collect(),
                ),
                world: Transform(world.map(|v| v as f32)),
                camera: scene.scene.camera,
                basis: scene.view.basis,
                near: n(&resolved.scene["camera"]["p"], "near", 0.01),
                far: n(&resolved.scene["camera"]["p"], "far", 10000.),
            });
        }
        Ok(layers)
    }
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
            meters_per_unit: 0.1,
            world: Default::default(),
            instances: vec![],
            scatters: vec![],
            terrains: vec![],
            displacements: vec![],
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
                focal_mm: 12. / (n(cp, "fov", 45.).to_radians() / 2.).tan(),
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
        let mut map_layers = vec![];
        let mut map_indices = HashMap::new();
        for object in r.scene["objects"]
            .as_array()
            .ok_or("Studio scene has no objects")?
        {
            let id = object["id"].as_str().ok_or("Studio object has no ID")?;
            if ["world", "fog", "terrain", "scatter"]
                .iter()
                .any(|key| object.get(*key).is_some())
                || object["source"].get("nature").is_some()
            {
                return Err("Studio environment document adapter is not connected yet".into());
            }
            if [
                "materialProperties",
                "sourceMaterialProperties",
                "sourceMaterials",
                "parts",
            ]
            .iter()
            .any(|key| {
                object
                    .get(*key)
                    .and_then(Value::as_object)
                    .is_some_and(|m| !m.is_empty())
            }) {
                return Err(
                    "Studio sparse material/part document adapter is not connected yet".into(),
                );
            }
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
            let world = *r
                .model_worlds
                .get(id)
                .ok_or("Studio object has no evaluated world transform")?;
            if let Some(asset) = object["source"]["assetId"].as_str() {
                let resource = self.resource(asset)?;
                if matches!(resource.model, pm_scene3d::import::Model::Splats(_)) {
                    continue;
                }
                for part in resource
                    .parts(request.ticks as f64 / pm_protocol::common::TICKS_PER_SECOND as f64)
                    .map_err(|e| e.to_string())?
                {
                    let source_material = (object["useSourceMaterials"] == true)
                        .then_some(part.material.as_ref())
                        .flatten();
                    let selected = source_material.map_or(m, imported_material);
                    scene.materials.push(selected);
                    let mut gpu = selected.gpu();
                    gpu.extra[2] =
                        source_material.map_or(n(p, "opacity", 1.), |m| m.opacity as f32);
                    if let Some(material) = source_material {
                        for (slot, map) in [
                            material.map.as_ref(),
                            material.emissive_map.as_ref(),
                            material
                                .roughness_map
                                .as_ref()
                                .or(material.metalness_map.as_ref()),
                            material.normal_map.as_ref(),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            if let Some(map) = map {
                                let key = (resource.key.clone(), map.texture);
                                let index = if let Some(index) = map_indices.get(&key) {
                                    *index
                                } else {
                                    if map_layers.len() >= 256 {
                                        return Err("Studio map array exceeds 256 layers".into());
                                    }
                                    let index = map_layers.len();
                                    map_layers.push(
                                        resource.texture(map.texture).map_err(|e| e.to_string())?,
                                    );
                                    map_indices.insert(key, index);
                                    index
                                };
                                // Basic materials glow with their colour map.
                                let slot = if material.kind
                                    == pm_scene3d::render::gltf::MaterialType::Basic
                                    && slot == 0
                                {
                                    1
                                } else {
                                    slot
                                };
                                gpu.maps[slot] = index as f32;
                            }
                        }
                    }
                    materials.push(gpu);
                    let mesh = self.cached_mesh(
                        format!("asset:{}", pm_scene3d::render::geometry_key(&part.geometry)),
                        || Ok(convert_mesh(&part.geometry)),
                    )?;
                    instances.push(MeshInstance {
                        layer: id.into(),
                        mesh,
                        material: materials.len() - 1,
                        world: Transform(
                            pm_scene3d::render::gltf::math::multiply(&world, &part.world)
                                .map(|v| v as f32),
                        ),
                    });
                }
            } else {
                scene.materials.push(m);
                let mut gpu = m.gpu();
                gpu.extra[2] = n(p, "opacity", 1.);
                materials.push(gpu);
                instances.push(MeshInstance {
                    layer: id.into(),
                    mesh: self.mesh(&object["source"])?,
                    material: materials.len() - 1,
                    world: Transform(world.map(|v| v as f32)),
                });
            }
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
            maps: (!map_layers.is_empty()).then_some(TextureMaps {
                width: 1024,
                height: 1024,
                layers: map_layers,
            }),
            view: View {
                basis,
                ground: Some(env * 0.35),
                floor_y,
                shadows: e["shadows"] != false,
            },
        })
    }
}

fn convert_mesh(g: &pm_scene3d::render::Geometry) -> Mesh {
    Mesh {
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
            g.indices.clone()
        }
        .chunks_exact(3)
        .map(|t| [t[0], t[1], t[2]])
        .collect(),
        ..Default::default()
    }
}
fn imported_material(m: &pm_scene3d::render::gltf::Material) -> Material {
    let color = v3(m.color[0] as f32, m.color[1] as f32, m.color[2] as f32);
    let basic = m.kind == pm_scene3d::render::gltf::MaterialType::Basic;
    Material {
        metallic: if basic { 0. } else { m.metalness as f32 },
        coat: m.clearcoat as f32,
        coat_roughness: m.clearcoat_roughness as f32,
        transmission: m.transmission as f32,
        ior: m.ior as f32,
        specular: m.specular_intensity as f32,
        emission: if basic {
            color
        } else {
            v3(
                m.emissive[0] as f32,
                m.emissive[1] as f32,
                m.emissive[2] as f32,
            ) * m.emissive_intensity as f32
        },
        sheen: m.sheen as f32,
        film_nm: if m.iridescence > 0. {
            m.iridescence_thickness_range[1] as f32
        } else {
            0.
        },
        ..Material::dielectric(color, if basic { 1. } else { m.roughness as f32 })
    }
}
