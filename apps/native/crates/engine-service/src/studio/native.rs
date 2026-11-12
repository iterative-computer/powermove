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

#[path = "environment.rs"]
mod environment;
#[path = "extrusion.rs"]
mod extrusion;
#[path = "geometry.rs"]
mod geometry;
#[path = "imported.rs"]
mod imported;
#[path = "intrinsic.rs"]
mod intrinsic;
#[path = "maps.rs"]
mod maps;
#[path = "material_capture.rs"]
mod material_capture;
#[path = "material_policy.rs"]
mod material_policy;
#[path = "scatter.rs"]
mod scatter;

pub struct DocumentSource {
    refine_observer: Option<super::RefineObserver>,
    text: Mutex<pm_render::text::TextContext>,
    capture_id: u64,
    capture_fonts: Option<pm_render::text::host::RenderFonts>,
    capture_font_factory:
        Option<Arc<dyn Fn() -> Box<dyn pm_render::text::TextFonts> + Send + Sync>>,
    capture_definitions: pm_protocol::definitions::DefinitionSetSnapshot,
    capture_bindings: Mutex<Option<material_capture::Bindings>>,
    capture_plans: Mutex<HashMap<(String, String), Arc<material_capture::Plan>>>,
    material_references: Mutex<HashMap<String, Arc<Vec<(String, Value)>>>>,
    extruded: Mutex<Vec<(String, Arc<pm_scene3d::extrusion::Extrusion>)>>,
    snapshot: pm_kernel::Snapshot,
    media: pm_export::render::MediaPaths,
    meshes: Mutex<HashMap<String, Arc<Mesh>>>,
    resources: Mutex<HashMap<String, Arc<pm_scene3d::render::Resource>>>,
    resolved: Mutex<Option<(SceneRequest, Arc<pm_scene3d::service::ResolvedScene>)>>,
    quality_override: Option<(String, Value)>,
    export_render_scale:f32,
    camera: super::ViewCamera,
    view_composition: Option<String>,
    environment_maps: Mutex<Vec<(String, Arc<pm_studio3d_engine::environment::EnvironmentMap>)>>,
    height_maps: Mutex<Vec<((String, u32), Arc<Vec<f32>>)>>,
    material_images: Mutex<Vec<(String, Arc<Vec<u8>>)>>,
    vertex_attributes:
        Mutex<HashMap<(usize, usize, bool), Arc<pm_studio3d_engine::extract::VertexAttributes>>>,
    scatter_placements: Mutex<Vec<(String, Arc<Vec<pm_studio3d_engine::scatter::Instance>>)>>,
    failures: Mutex<Option<(SceneRequest, Vec<pm_render::LayerFailure>)>>,
    backdrop_owner: Mutex<Option<(SceneRequest, Option<String>)>>,
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
    pub(super) fn revision(&self) -> u64 {
        self.snapshot.revision
    }
    pub fn new(document: Arc<pm_model::Document>, media: pm_export::render::MediaPaths) -> Self {
        static NEXT_CAPTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            refine_observer: None,
            capture_id: NEXT_CAPTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            capture_fonts: None,
            capture_font_factory: None,
            capture_definitions: Default::default(),
            capture_bindings: Mutex::default(),
            capture_plans: Mutex::default(),
            material_references: Mutex::default(),
            text: Mutex::default(),
            extruded: Mutex::default(),
            snapshot: pm_kernel::Snapshot {
                doc_id: 0,
                revision: document.revision(),
                document,
            },
            media,
            meshes: Mutex::default(),
            resources: Mutex::default(),
            resolved: Mutex::default(),
            quality_override: None,
            export_render_scale:1.,
            camera: super::ViewCamera::Active,
            view_composition: None,
            environment_maps: Mutex::default(),
            height_maps: Mutex::default(),
            material_images: Mutex::default(),
            vertex_attributes: Mutex::default(),
            scatter_placements: Mutex::default(),
            failures: Mutex::default(),
            backdrop_owner: Mutex::default(),
        }
    }
    pub fn with_export_render_scale(mut self, scale:f32)->Result<Self,String> {
        if !scale.is_finite() || scale<=0. || scale>1. {return Err("Studio render scale must be greater than zero and at most one".into());}
        self.export_render_scale=scale;Ok(self)
    }
    pub fn with_refine_observer(mut self, observer: Option<super::RefineObserver>) -> Self {
        self.refine_observer = observer; self
    }
    pub fn with_text_fonts(self, fonts: Box<dyn pm_render::text::TextFonts>) -> Self {
        self.text
            .lock()
            .expect("new text context")
            .set_fonts(Some(fonts));
        self
    }
    pub fn with_native_fonts(mut self, fonts: Option<&pm_render::text::host::RenderFonts>) -> Self {
        self.capture_fonts = fonts.cloned();
        if let Some(fonts) = fonts {
            self.with_text_fonts(Box::new(pm_render::text::host::EngineFonts::new(
                &fonts.broker,
                &fonts.health,
            )))
        } else {
            self
        }
    }
    pub fn with_capture_font_factory(
        mut self,
        factory: Arc<dyn Fn() -> Box<dyn pm_render::text::TextFonts> + Send + Sync>,
    ) -> Self {
        self.capture_font_factory = Some(factory);
        self
    }
    pub fn with_capture_definitions(
        mut self,
        definitions: pm_protocol::definitions::DefinitionSetSnapshot,
    ) -> Self {
        self.capture_definitions = definitions;
        self
    }
    pub fn with_view(mut self, view: Option<super::FreeView>) -> Self {
        self.camera = view.into();
        self
    }
    pub fn with_view_for(self, composition: &str, view: Option<super::FreeView>) -> Self {
        self.with_camera_for(composition, view.into())
    }
    pub fn with_camera_for(mut self, composition: &str, camera: super::ViewCamera) -> Self {
        self.camera = camera;
        self.view_composition = Some(composition.into());
        self
    }
    fn camera_for(&self, request: &SceneRequest) -> &super::ViewCamera {
        if self
            .view_composition
            .as_ref()
            .is_none_or(|id| id == &request.composition)
        {
            &self.camera
        } else {
            &super::ViewCamera::Active
        }
    }
    /// Resolve one export's override without editing the saved composition.
    pub fn with_quality_override(
        mut self,
        composition: &str,
        patch: Value,
    ) -> Result<Self, String> {
        let request = SceneRequest {
            composition: composition.into(),
            revision: self.snapshot.revision,
            ticks: 0,
            width: 1,
            height: 1,
        };
        let scene = self.resolve(&request)?;
        let current = pm_scene3d::quality::get(&scene.scene).map_err(|e| e.to_string())?;
        let quality = pm_scene3d::quality::resolve(&current, &patch).map_err(|e| e.to_string())?;
        self.quality_override = Some((composition.into(), quality));
        Ok(self)
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
            pm_kernel::commands::scene3d::resolve_scene_with_camera(
                &self.snapshot,
                Some(&request.composition),
                request.ticks as f64 / pm_protocol::common::TICKS_PER_SECOND as f64,
                match self.camera_for(request) {
                    super::ViewCamera::Layer { id } => Some(id.as_str()),
                    _ => None,
                },
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
    fn environment_image(
        &self,
        source: &Value,
    ) -> Result<pm_studio3d_engine::environment::EnvironmentMap, String> {
        let id = source
            .as_str()
            .ok_or("Live layer/composition environment capture is not connected yet")?;
        let path = self
            .media
            .paths
            .get(id)
            .ok_or_else(|| format!("World image is not loaded: {id}"))?;
        let mut cache = self
            .environment_maps
            .lock()
            .map_err(|_| "World image cache poisoned")?;
        if let Some(index) = cache.iter().position(|(key, _)| key == id) {
            let entry = cache.remove(index);
            let image = (*entry.1).clone();
            cache.push(entry);
            return Ok(image);
        }
        let image = Arc::new(pm_studio3d_engine::environment::EnvironmentMap::load(path)?);
        cache.push((id.into(), image.clone()));
        while cache.len() > 4
            || cache
                .iter()
                .map(|(_, m)| m.pixels.len() * std::mem::size_of::<Vec3>())
                .sum::<usize>()
                > 96 * 1024 * 1024
        {
            cache.remove(0);
        }
        Ok((*image).clone())
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
        let mesh = build()?;
        pm_studio3d_engine::extract::validate_mesh(&mesh, None)?;
        let mesh = Arc::new(mesh);
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
    fn final_render_scale(&self,_request:&SceneRequest)->Result<f32,String>{Ok(self.export_render_scale)}
    fn needs_materials(&self, request: &SceneRequest) -> Result<bool, String> {
        Ok(!self.material_references(request)?.is_empty())
    }
    fn prepare_materials(
        &self,
        renderer: &mut pm_render::Renderer,
        request: &SceneRequest,
        tier: Tier,
    ) -> Result<bool, String> {
        self.capture_materials(renderer, request, tier)
    }
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
            let Ok(resource) = self.resource(asset) else {
                continue;
            };
            let pm_scene3d::import::Model::Splats(data) = &resource.model else {
                continue;
            };
            let id = object["id"].as_str().ok_or("Splat layer has no ID")?;
            let Ok((center, radius)) = data.normalization() else {
                continue;
            };
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
        if let Some(quality) = self.final_quality(request)? {
            return Ok(quality.max_samples);
        }
        Ok(self.resolve(request)?.scene["environment"]["samples"]
            .as_u64()
            .unwrap_or(64) as u32)
    }
}
impl SceneSource for DocumentSource {
    fn refine_progress(&self, request: &SceneRequest, progress: super::RefineProgress) -> bool {
        self.refine_observer.as_ref().is_none_or(|f| f(request,progress))
    }
    fn layer_failures(
        &self,
        request: &SceneRequest,
    ) -> Result<Vec<pm_render::LayerFailure>, String> {
        Ok(self
            .failures
            .lock()
            .map_err(|_| "Studio failure cache poisoned")?
            .as_ref()
            .filter(|(key, _)| key == request)
            .map_or_else(Vec::new, |(_, f)| f.clone()))
    }
    fn auxiliary_layers(
        &self,
        request: &SceneRequest,
    ) -> Result<Vec<pm_studio3d_engine::extract::AuxiliaryLayer>, String> {
        let failures = self.layer_failures(request)?;
        let resolved = self.resolve(request)?;
        let mut layers: Vec<_> = resolved.scene["objects"]
            .as_array()
            .ok_or("Missing Studio objects")?
            .iter()
            .filter(|o| {
                !failures.iter().any(|f| o["id"] == f.layer)
                    && (o.get("world").is_some()
                        || o.get("fog").is_some()
                        || o.get("scatter").is_some())
            })
            .map(|o| pm_studio3d_engine::extract::AuxiliaryLayer {
                id: o["id"].as_str().unwrap().into(),
                include_void: o.get("world").is_some()
                    && val(&o["p"]["visible"]) != false
                    && val(&o["world"]["p"]["visible"]) != false
                    && o["world"]["mode"] != "none",
            })
            .collect();
        for layer in resolved.scene["extrusions"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if layer["threeD"] == true
                && layer["extrusion"]["enabled"] == true
                && !failures.iter().any(|f| layer["id"] == f.layer)
            {
                layers.push(pm_studio3d_engine::extract::AuxiliaryLayer {
                    id: layer["id"].as_str().ok_or("Extrusion has no ID")?.into(),
                    include_void: false,
                });
            }
        }
        if !layers.iter().any(|l| l.include_void) {
            let owner = self
                .backdrop_owner
                .lock()
                .map_err(|_| "Studio backdrop state poisoned")?;
            if let Some((_, Some(id))) = owner.as_ref().filter(|(key, _)| key == request) {
                if let Some(layer) = layers.iter_mut().find(|l| l.id == *id) {
                    layer.include_void = true;
                } else {
                    layers.push(pm_studio3d_engine::extract::AuxiliaryLayer {
                        id: id.clone(),
                        include_void: true,
                    });
                }
            }
        }
        // Uncovered volumetric radiance still needs a compositor output when
        // the sky and studio backdrop are transparent, including fog-only sets.
        if !layers.iter().any(|layer| layer.include_void) {
            for layer in layers.iter_mut().rev() {
                let owner = resolved.scene["objects"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|o| o["id"] == layer.id);
                if owner.is_some_and(|o| {
                    val(&o["p"]["visible"]) != false
                        && (n(&o["fog"]["p"], "density", 0.) > 0.
                            || n(&o["world"]["p"], "fogDensity", 0.) > 0.)
                }) {
                    layer.include_void = true;
                    break;
                }
            }
        }
        Ok(layers)
    }
    fn final_quality(
        &self,
        request: &SceneRequest,
    ) -> Result<Option<pm_studio3d_engine::extract::FinalQuality>, String> {
        let resolved = self.resolve(request)?;
        let value = self
            .quality_override
            .as_ref()
            .filter(|(id, _)| id == &request.composition)
            .map(|(_, q)| q)
            .or_else(|| resolved.scene.get("studioQuality"));
        let q = match value {
            Some(value) => pm_scene3d::quality::validate(value.clone()),
            None => pm_scene3d::quality::preset("production"),
        }
        .map_err(|e| e.to_string())?;
        Ok(Some(
            pm_studio3d_engine::extract::FinalQuality {
                sampling: pm_studio3d_engine::sampling::SamplingPolicy {
                    perceptual_noise:q["perceptualNoise"].as_bool().unwrap_or(false),
                    edge_multiplier:q["edgeSamplesMultiplier"].as_u64().unwrap_or(4) as u32,
                    diffuse_bounces:q["diffuseBounces"].as_u64().unwrap_or(0) as u32,
                    glossy_bounces:q["glossyBounces"].as_u64().unwrap_or(0) as u32,
                    transmission_bounces:q["transmissionBounces"].as_u64().unwrap_or(0) as u32,
                },
                min_samples: q["minSamples"].as_u64().unwrap() as u32,
                max_samples: q["maxSamples"].as_u64().unwrap() as u32,
                noise_threshold: q["noiseThreshold"].as_f64().unwrap() as f32,
                max_bounces: q["maxBounces"].as_u64().unwrap() as u32,
                caustics: q["caustics"].as_bool().unwrap(),
                denoise: q["denoise"].as_bool().unwrap(),
                fast_denoise: q["preset"] == "draft",
                half_resolution_guides: q["preset"] != "final",
            }
            .validate()?,
        ))
    }
    fn scene(&self, request: &SceneRequest) -> Result<SceneInput, String> {
        let r = self.resolve(request)?;
        let c = &r.scene["camera"];
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
        let choice = self.camera_for(request);
        choice.validate()?;
        let view = choice.pose();
        let (eye, target, zoom) = if let Some(view) = view {
            let f = |p: [f64; 3]| v3(p[0] as f32, p[1] as f32, p[2] as f32);
            (f(view.eye), f(view.target), view.zoom as f32)
        } else {
            (eye, target, n(cp, "zoom", 1.))
        };
        let fov = 2. * ((n(cp, "fov", 45.).to_radians() / 2.).tan() / zoom).atan();
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
                f_stop: if matches!(choice, super::ViewCamera::Orthographic { .. }) {
                    0.
                } else {
                    n(cp, "fStop", 0.)
                },
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
        let mut failures = vec![];
        let mut instances = vec![];
        environment::apply(
            &r,
            &mut scene,
            &request.composition,
            &mut failures,
            &|source| self.environment_image(source),
        )?;
        let mut materials: Vec<GpuMaterial> = vec![];
        let mut map_layers = vec![];
        let mut map_indices = HashMap::new();
        for object in r.scene["objects"]
            .as_array()
            .ok_or("Studio scene has no objects")?
        {
            let id = object["id"].as_str().ok_or("Studio object has no ID")?;
            let checkpoint = (
                instances.len(),
                materials.len(),
                scene.materials.len(),
                map_layers.len(),
            );
            let contribution = (|| -> Result<(), String> {
                if object.get("world").is_some() || object.get("fog").is_some() {
                    return Ok(());
                }
                if object.get("scatter").is_some() {
                    return Ok(());
                }
                if object
                    .get("parts")
                    .and_then(Value::as_object)
                    .is_some_and(|p| !p.is_empty())
                {
                    return Err("Studio part-offset document adapter is not connected yet".into());
                }

                if !object["slots"].as_array().is_none_or(Vec::is_empty) {
                    return Err("Studio material slots require their native slot adapter".into());
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
                if object.get("terrain").is_some() || object["source"].get("nature").is_some() {
                    let (mesh, palette) = if object.get("terrain").is_some() {
                        self.terrain_mesh(object, m, &scene.camera)?
                    } else {
                        self.nature_mesh(&object["source"], 0)?
                    };
                    let first = materials.len();
                    let indices = Arc::new((first..first + palette.len()).collect());
                    for (slot, mut material) in palette.into_iter().enumerate() {
                        let mut gpu = material.gpu();
                        gpu.extra[2] = n(p, "opacity", 1.);
                        self.bind_material_maps(
                            request,
                            id,
                            &maps::overrides(object, Some(0), false),
                            &mut gpu,
                            &mut map_layers,
                            &mut map_indices,
                        )?;
                        if object.get("terrain").is_none() || slot == 0 {
                            let emission = material.emission;
                            intrinsic::apply(
                                object,
                                None,
                                &mut material,
                                &mut gpu,
                                emission,
                                1.,
                                false,
                            );
                        }
                        maps::screen(object, Some(0), &mut material, &mut gpu, false);
                        materials.push(gpu);
                        scene.materials.push(material);
                    }
                    instances.push(MeshInstance {
                        layer: id.into(),
                        mesh,
                        material: first,
                        material_palette: Some(indices),
                        attributes: None,
                        primary_visible: true,
                        world: Transform(world.map(|v| v as f32)),
                    });
                } else if let Some(asset) = object["source"]["assetId"].as_str() {
                    let resource = self.resource(asset)?;
                    if let pm_scene3d::import::Model::Splats(data) = &resource.model {
                        data.normalization().map_err(|e| e.to_string())?;
                        return Ok(());
                    }
                    for part in resource
                        .parts(request.ticks as f64 / pm_protocol::common::TICKS_PER_SECOND as f64)
                        .map_err(|e| e.to_string())?
                    {
                        let source_material = (object["useSourceMaterials"] == true)
                            .then_some(part.material.as_ref())
                            .flatten();
                        let mut selected = source_material.map_or(m, imported_material);
                        let mut gpu = selected.gpu();
                        gpu.extra[2] =
                            source_material.map_or(n(p, "opacity", 1.), |m| m.opacity as f32);
                        if let Some(material) = source_material {
                            imported::pack(material, &mut gpu, |texture_index| {
                                let pm_scene3d::import::Model::Gltf(model) = &resource.model else {
                                    return Err("Imported maps require a glTF resource".into());
                                };
                                let texture = model
                                    .textures
                                    .get(texture_index)
                                    .ok_or("Imported map texture is missing")?;
                                // One image layer; each physical slot keeps its own sampler.
                                let key = (resource.key.clone(), texture.image);
                                let index = if let Some(index) = map_indices.get(&key) {
                                    *index
                                } else {
                                    if map_layers.len() >= 256 {
                                        return Err("Studio map array exceeds 256 layers".into());
                                    }
                                    let pixels = imported::image(&resource, texture_index)?;
                                    let index = map_layers.len();
                                    map_layers.push((*pixels).clone());
                                    map_indices.insert(key, index);
                                    index
                                };
                                Ok((index, texture.sampler.wrap_s, texture.sampler.wrap_t))
                            })?;
                        } else {
                            // Three/OBJ geometry uses bottom-origin V, while
                            // native image and live captures have top-down rows.
                            // Keep authored UVs/tangents intact and flip only
                            // the legacy map lookup; imported physical slots
                            // retain their own glTF coordinates and transforms.
                            gpu.policy[1] = (gpu.policy[1] as u32 | 128) as f32;
                            if object["material"]["doubleSided"] == false {
                                gpu.policy[1] = (gpu.policy[1] as u32 & !4) as f32;
                            }
                        }
                        let basic = source_material.is_some_and(|m| {
                            m.kind == pm_scene3d::render::gltf::MaterialType::Basic
                        });
                        let index = if source_material.is_some() {
                            part.source_material
                        } else {
                            Some(0)
                        };
                        let overrides = maps::overrides(object, index, basic);
                        self.bind_material_maps(
                            request,
                            id,
                            &overrides,
                            &mut gpu,
                            &mut map_layers,
                            &mut map_indices,
                        )?;
                        let mut emission_color = source_material.map_or_else(
                            || color(p, "emissive", "#000000"),
                            |m| {
                                v3(
                                    m.emissive[0] as f32,
                                    m.emissive[1] as f32,
                                    m.emissive[2] as f32,
                                )
                            },
                        );
                        let mut emission_intensity = source_material
                            .map_or(n(p, "emissiveIntensity", 1.), |m| {
                                m.emissive_intensity as f32
                            });
                        if source_material.is_some() {
                            if overrides["maps"].get("color").is_some() {
                                selected.base = Vec3::splat(1.);
                                gpu.base[..3].fill(1.);
                            }
                            if overrides["maps"].get("emissive").is_some() {
                                emission_color = Vec3::splat(1.);
                                emission_intensity = emission_intensity.max(1.);
                                selected.emission = emission_color * emission_intensity;
                                gpu.emit[..3].fill(emission_intensity);
                            }
                        }
                        intrinsic::apply(
                            object,
                            index,
                            &mut selected,
                            &mut gpu,
                            emission_color,
                            emission_intensity,
                            basic,
                        );
                        maps::screen(object, index, &mut selected, &mut gpu, basic);
                        scene.materials.push(selected);
                        materials.push(gpu);
                        let mesh = self.cached_mesh(
                            format!("asset:{}", pm_scene3d::render::geometry_key(&part.geometry)),
                            || Ok(convert_mesh(&part.geometry)),
                        )?;
                        instances.push(MeshInstance {
                            layer: id.into(),
                            mesh,
                            material: materials.len() - 1,
                            material_palette: None,
                            attributes: {
                                let flat_shading = source_material.is_some_and(|m| m.flat_shading);
                                let attrs = &part.attributes;
                                if attrs.colors.is_empty()
                                    && attrs.secondary_uvs.is_empty()
                                    && !flat_shading
                                {
                                    None
                                } else {
                                    let key = (
                                        Arc::as_ptr(&attrs.colors) as usize,
                                        Arc::as_ptr(&attrs.secondary_uvs) as usize,
                                        flat_shading,
                                    );
                                    let mut cache = self
                                        .vertex_attributes
                                        .lock()
                                        .map_err(|_| "Studio vertex cache poisoned")?;
                                    Some(if let Some(attrs) = cache.get(&key) {
                                        attrs.clone()
                                    } else {
                                        let attrs = pm_studio3d_engine::extract::VertexAttributes {
                                            colors: attrs.colors.clone(),
                                            secondary_uvs: attrs.secondary_uvs.clone(),
                                            flat_shading,
                                        }
                                        .shared()?;
                                        cache.insert(key, attrs.clone());
                                        attrs
                                    })
                                }
                            },
                            primary_visible: true,
                            world: Transform(
                                pm_scene3d::render::gltf::math::multiply(&world, &part.world)
                                    .map(|v| v as f32),
                            ),
                        });
                    }
                } else {
                    let mut material = m;
                    let mut gpu = material.gpu();
                    gpu.policy[1] = (gpu.policy[1] as u32 | 128) as f32;
                    gpu.extra[2] = n(p, "opacity", 1.);
                    if object["material"]["doubleSided"] == false {
                        gpu.policy[1] = (gpu.policy[1] as u32 & !4) as f32;
                    }
                    self.bind_material_maps(
                        request,
                        id,
                        &maps::overrides(object, Some(0), false),
                        &mut gpu,
                        &mut map_layers,
                        &mut map_indices,
                    )?;
                    intrinsic::apply(
                        object,
                        Some(0),
                        &mut material,
                        &mut gpu,
                        color(p, "emissive", "#000000"),
                        n(p, "emissiveIntensity", 1.),
                        false,
                    );
                    maps::screen(object, Some(0), &mut material, &mut gpu, false);
                    scene.materials.push(material);
                    materials.push(gpu);
                    instances.push(MeshInstance {
                        layer: id.into(),
                        mesh: self.mesh(&object["source"])?,
                        material: materials.len() - 1,
                        material_palette: None,
                        attributes: None,
                        primary_visible: true,
                        world: Transform(world.map(|v| v as f32)),
                    });
                }

                for material in &materials[checkpoint.1..] {
                    material.validate()?;
                }
                for instance in &instances[checkpoint.0..] {
                    instance.world.validate()?;
                    if instance.attributes.as_ref().is_some_and(|a| {
                        (!a.colors.is_empty() && a.colors.len() != instance.mesh.positions.len())
                            || (!a.secondary_uvs.is_empty()
                                && a.secondary_uvs.len() != instance.mesh.positions.len())
                    }) {
                        return Err("Invalid Studio imported vertex attributes".into());
                    }
                }
                Ok(())
            })();
            if let Err(message) = contribution {
                instances.truncate(checkpoint.0);
                materials.truncate(checkpoint.1);
                scene.materials.truncate(checkpoint.2);
                map_layers.truncate(checkpoint.3);
                map_indices.retain(|_, index| *index < checkpoint.3);
                failures.push(pm_render::LayerFailure {
                    composition: request.composition.clone(),
                    layer: id.into(),
                    message,
                });
            }
        }
        scatter::apply(
            self,
            &r,
            &scene,
            &mut instances,
            &request.composition,
            &mut failures,
        )?;
        material_policy::apply(&r, &mut instances, &mut scene, &mut materials)?;
        self.add_extrusions(
            request,
            &mut scene,
            &mut instances,
            &mut materials,
            &mut map_layers,
            &mut map_indices,
            &mut failures,
        )?;
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
        let mut ambient = Vec3::ZERO;
        for light in r.scene["lights"]
            .as_array()
            .ok_or("Studio scene has no lights")?
        {
            let p = &light["p"];
            if val(&p["visible"]) == false {
                continue;
            }
            let id = light["id"].as_str().ok_or("Light has no ID")?;
            let checkpoint = (scene.lights.len(), scene.extra_lights.len(), ambient);
            let result = (|| -> Result<(), String> {
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
                    "sun" | "parallel" => scene.extra_lights.push(GpuLight::sun(
                        -direction,
                        1.5_f32.to_radians(),
                        radiance
                            * (1.
                                / (2. * std::f32::consts::PI * (1. - 1.5_f32.to_radians().cos()))),
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
                    "ambient" => {
                        ambient = ambient + radiance;
                    }
                    "environment" => {}
                    _ => return Err("Unsupported Studio light".into()),
                }
                for light in &scene.lights[checkpoint.0..] {
                    light.validate()?;
                }
                for light in &scene.extra_lights[checkpoint.1..] {
                    light.validate()?;
                }
                if ambient.to_array().iter().any(|v| !v.is_finite() || *v < 0.) {
                    return Err("Invalid ambient light".into());
                }
                Ok(())
            })();
            if let Err(message) = result {
                scene.lights.truncate(checkpoint.0);
                scene.extra_lights.truncate(checkpoint.1);
                ambient = checkpoint.2;
                failures.push(pm_render::LayerFailure {
                    composition: request.composition.clone(),
                    layer: id.into(),
                    message,
                });
            }
        }
        let basis = if view.is_some() {
            let (forward, right, up) = scene.camera.basis();
            Some([forward, right, up])
        } else {
            r.camera_up.map(|up| {
                let f = (target - eye).normalize();
                let side = f.cross(v3(up[0] as f32, up[1] as f32, up[2] as f32));
                let right = if side.length() < 1e-6 {
                    scene.camera.basis().1
                } else {
                    side.normalize()
                };
                [f, right, right.cross(f)]
            })
        };
        let floor_y = if e["shadowCatcher"] == true && !instances.is_empty() {
            low.y
        } else {
            0.
        };
        // Port the source runtime's display-referred studio backdrop. It is
        // independent of the lighting environment and is composited only once.
        scene.shadow_catcher &= !instances.is_empty();
        let background = e["background"]
            .as_str()
            .or_else(|| scene.shadow_catcher.then_some("#F6F5F8"));
        scene.backdrop = background.map_or(Vec3::ZERO, |hex| {
            let rgb = u32::from_str_radix(hex.strip_prefix('#').unwrap_or(hex), 16).unwrap_or(0);
            v3(
                ((rgb >> 16) & 255) as f32 / 255.,
                ((rgb >> 8) & 255) as f32 / 255.,
                (rgb & 255) as f32 / 255.,
            )
        });
        let backdrop_owner = if background.is_some()
            || scene.world.atmosphere.density > 0.
            || !scene.world.volumes.is_empty()
        {
            let visible: std::collections::HashSet<_> = instances
                .iter()
                .filter(|i| i.primary_visible)
                .map(|i| i.layer.as_str())
                .collect();
            let comp = pm_eval::Comp::resolve(&self.snapshot.document, &request.composition)
                .map_err(|e| e.to_string())?;
            comp.layers()
                .iter()
                .rev()
                .filter_map(|l| pm_eval::node::string(l.get("id")).map(|id| id.to_string_lossy()))
                .find(|id| visible.contains(id.as_str()))
        } else {
            None
        };
        *self
            .backdrop_owner
            .lock()
            .map_err(|_| "Studio backdrop state poisoned")? =
            Some((request.clone(), backdrop_owner));
        *self
            .failures
            .lock()
            .map_err(|_| "Studio failure cache poisoned")? = Some((request.clone(), failures));
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
                backdrop_opaque: Some(background.is_some()),
                shadows: e["shadows"] != false,
                ambient,
                orthographic_span: match choice {
                    super::ViewCamera::Orthographic { span, .. } => Some(*span as f32 * 0.5),
                    _ => (c["projection"] == "orthographic").then(|| 3. / zoom),
                },
                clip_planes: Some(match choice {
                    super::ViewCamera::Orthographic { span, .. } => {
                        [0.01, (*span as f32 * 4. + 100.).max(1000.)]
                    }
                    _ => [n(cp, "near", 0.01), n(cp, "far", 1000.)],
                }),
                iris: [
                    n(cp, "irisShape", 0.),
                    n(cp, "irisRotation", 0.).to_radians(),
                    n(cp, "irisRoundness", 0.),
                    n(cp, "irisAspect", 1.),
                ],
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
        dispersion: m.dispersion as f32,
        anisotropy: m.anisotropy as f32,
        film_nm: if m.iridescence > 0. {
            m.iridescence_thickness_range[1] as f32
        } else {
            0.
        },
        ..Material::dielectric(color, if basic { 1. } else { m.roughness as f32 })
    }
}
