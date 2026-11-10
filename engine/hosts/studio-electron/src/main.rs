//! Powermove Studio's isolated, headless native process. No Electron/Node dependency.
mod budget;
mod fbx;
mod packed;
mod protocol;
mod scene;
use pm_studio::{
    gpu::Renderer,
    scene::{Camera, Material, Scene, Softbox},
};
use protocol::{Packet, VERSION};
use scene::{Identity, MeshBuffers, ResidentScene, Transform};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{io, time::Instant};
use std::sync::{Arc,atomic::{AtomicU64,AtomicUsize,Ordering}};

#[derive(Deserialize)]
struct Request {
    id: u64,
    #[serde(flatten)]
    command: Command,
}
#[derive(Clone, Deserialize)]
struct Shutter { key:String, index:u32, count:u32 }

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
enum Command {
    Hello,
    CancelRefine,
    ImportFbx,
    ReleaseFrame {
        frame: u64,
    },

    Prepared {
        #[serde(flatten)]
        upload: packed::Upload,
    },
    PreparedBegin { upload:packed::Upload, bytes:usize },
    PreparedChunk { offset:usize },
    PreparedCommit,
    PreparedPatch {
        #[serde(flatten)]
        patch: packed::Patch,
    },
    PreparedView { key:String, view:packed::View },
    Snapshot,
    Scene {
        scene: Scene,
        #[serde(default)]
        meshes: Vec<MeshBuffers>,
        #[serde(default)]
        identities: Vec<Identity>,
        #[serde(default)]
        transforms: Vec<Transform>,
    },
    Preset {
        name: String,
        #[serde(default = "blue")]
        finish: String,
        #[serde(default)]
        time: f32,
    },
    Patch {
        #[serde(default)]
        transforms: Vec<Transform>,
        camera: Option<Camera>,
        materials: Option<Vec<Material>>,
        lights: Option<Vec<Softbox>>,
        #[serde(default)]
        time: f64,
    },
    Textures {
        width: u32,
        height: u32,
        layers: u32,
        layer: Option<u32>,
    },
    Screen {
        width: u32,
        height: u32,
    },
    Render {
        width: u32,
        height: u32,
        #[serde(default = "live")]
        tier: String,
        #[serde(default = "one")]
        samples: u32,
        #[serde(default)]
        adaptive: bool,
        #[serde(rename="timeBudgetMs", default)]
        time_budget_ms: u64,
        #[serde(rename="minSamples", default)]
        min_samples: Option<u32>,
        #[serde(rename="noiseThreshold", default)]
        noise_threshold: Option<f32>,
        #[serde(rename="maxBounces", default)]
        max_bounces: Option<u32>,
        #[serde(default)]
        caustics: Option<bool>,
        #[serde(default)]
        shutter: Option<Shutter>,
        #[serde(default)]
        temporal: bool,
        #[serde(default)]
        reset: bool,
        #[serde(default)]
        goal: bool,
        #[serde(default = "yes")]
        denoise: bool,
        #[serde(rename = "sharedTexture", default)]
        shared_texture: bool,
        layer: Option<u32>,
        #[serde(default = "yes")]
        background: bool,
        #[serde(default)]
        time: f64,
    },
    Pick {
        x: f32,
        y: f32,
        width: u32,
        height: u32,
    },
    Bounds {
        layer: Option<String>,
    },
    Shutdown,
}
fn blue() -> String {
    "blue".into()
}
fn live() -> String {
    "live".into()
}
fn yes() -> bool {
    true
}
fn one() -> u32 {
    1
}
struct Target {
    renderer: Renderer,
    revision: u64,
    geometry: u64,
    history_revision: u64,
    scene_revision: u64,
    live: bool,
    denoiser: &'static str,
    quality_denoise: bool,
    quality_samples: u32,
    quality_adaptive: bool,
    quality_temporal: bool,
    quality_budget_ms: u64,
    quality_policy: String,
    shutter: Option<(String,u32,u32,u32)>,
    shutter_policy:String,
    tier: String,
    time: f64,
}
struct Host {
    cancel_revision:Arc<AtomicU64>,
    request_generation:u64,
    peak_gpu_bytes:u64,
    upload:Option<(packed::Upload,Vec<u8>,usize)>,
    packed: Option<packed::PackedScene>,
    resident: Option<ResidentScene>,
    live: Option<Target>,
    full: Option<Target>,
    revision: u64,
    geometry: u64,
    history_revision: u64,
    screen: Option<(u32, u32, u32, Vec<u8>)>,
    start: Instant,
    frame_counter: u64,
    live_budget: budget::LiveBudget,
    #[cfg(feature = "oidn")]
    oidn: Option<pm_studio::oidn::Denoiser>,
}
impl Host {
    fn new() -> Self {
        Self {
            cancel_revision:Arc::new(AtomicU64::new(0)),request_generation:0,peak_gpu_bytes:0,
            upload:None,
            packed: None,
            resident: None,
            live: None,
            full: None,
            revision: 0,
            geometry: 0,
            history_revision: 0,
            screen: None,
            start: Instant::now(),
            frame_counter: u64::from(std::process::id()) << 32,
            live_budget: budget::LiveBudget::new(),
            #[cfg(feature = "oidn")]
            oidn: None,
        }
    }
    fn install(&mut self, r: ResidentScene) {
        self.live_budget.reset();
        self.packed = None;
        self.resident = Some(r);
        self.revision += 1;
        self.geometry += 1;
        self.history_revision += 1;
    }
    fn snapshot(&self) -> Result<(Value, Vec<u8>), String> {
        let r = self
            .resident
            .as_ref()
            .ok_or("snapshot needs a canonical scene")?;
        let p = &r.prepared;
        let mut binary = Vec::new();
        let mut append = |data: &[u8]| {
            let offset = binary.len();
            binary.extend_from_slice(data);
            json!({"offset":offset,"length":data.len()})
        };
        let positions = append(bytemuck::cast_slice(&p.positions));
        let tris = append(bytemuck::cast_slice(&p.tris));
        let nodes = append(bytemuck::cast_slice(&p.nodes));
        let materials = append(bytemuck::cast_slice(&p.materials));
        let sdata = append(bytemuck::cast_slice(&p.sdata));
        let texture=self.screen.as_ref().map(|(w,h,layers,data)|json!({"width":w,"height":h,"layers":layers,"bytes":append(data)}));
        let camera = r.scene.camera;
        let (forward,right,up)=camera.basis();
        let tan_y = (camera.fov.to_radians() * 0.5).tan();
        let (lo, hi) = r.bounds();
        let center = (lo + hi) * 0.5;
        let view = json!({"camera":{"position":camera.position.to_array(),"forward":forward.to_array(),"right":right.to_array(),"up":up.to_array(),"tanY":tan_y,"aspect":16.0/9.0,"lensRadius":camera.aperture_radius(r.scene.meters_per_unit),"focus":camera.focus.map(|f|(f-camera.position).dot(forward)).unwrap_or((camera.target-camera.position).length())},"exposure":r.scene.exposure,"backdrop":r.scene.backdrop.to_array(),"backdropOpaque":true,"shadowCatcher":r.scene.shadow_catcher,"shadows":true,"floorY":0.,"environment":{"zenith":r.scene.environment.0.to_array(),"horizon":r.scene.environment.1.to_array(),"ground":(r.scene.environment.1*0.5).to_array()},"bounds":[center.x,center.y,center.z,(hi-lo).length()*0.5]});
        Ok((
            json!({"positions":positions,"tris":tris,"nodes":nodes,"materials":materials,"sdata":sdata,"topNodes":r.top_nodes,"instanceCount":r.placements.len(),"lightCount":p.lights.len(),"objects":r.identities,"view":view,"bounds":{"min":lo.to_array(),"max":hi.to_array()},"texture":texture}),
            binary,
        ))
    }
    fn handle(&mut self, command: Command, binary: &[u8]) -> Result<(Value, Vec<u8>), String> {
        let begin = Instant::now();
        let value = match command {
            Command::CancelRefine=>json!({"cancelledRefine":true}),
            Command::PreparedBegin{upload,bytes}=>{
                self.upload=None;
                if bytes==0 || bytes>protocol::MAX_BINARY {return Err("invalid prepared upload size".into());}
                let mut data=Vec::new();data.try_reserve_exact(bytes).map_err(|_|"prepared upload exceeds available memory")?;
                self.upload=Some((upload,data,bytes));
                json!({"accepted":bytes})
            }
            Command::PreparedChunk{offset}=>{
                let Some((_,data,total))=&mut self.upload else{return Err("no prepared upload".into())};
                if binary.is_empty() || binary.len()>1024*1024 || offset!=data.len() || binary.len()>total.saturating_sub(offset){
                    self.upload=None;return Err("invalid prepared upload chunk".into());
                }
                data.extend_from_slice(binary);
                json!({"received":data.len()})
            }
            Command::PreparedCommit=>{
                let (upload,data,total)=self.upload.take().ok_or("no prepared upload")?;
                if data.len()!=total{return Err("incomplete prepared upload".into());}
                return self.handle(Command::Prepared{upload},&data);
            }
            Command::ImportFbx => {
                let bytes=binary.to_vec();
                let imported=std::thread::Builder::new().name("fbx-import".into()).stack_size(8*1024*1024)
                    .spawn(move||fbx::convert(&bytes)).map_err(|e|e.to_string())?
                    .join().map_err(|_|"FBX importer failed".to_string())??;
                return Ok((imported.stats,imported.glb));
            }
            Command::Prepared { upload } => {
                self.packed = Some(packed::PackedScene::new(upload, binary)?);
                self.live_budget.reset();
                self.resident = None;
                self.revision += 1;
                self.geometry += 1;
                self.history_revision += 1;
                json!({"revision":self.revision})
            }
            Command::PreparedPatch { patch } => {
                if self
                    .packed
                    .as_mut()
                    .ok_or("no prepared scene")?
                    .patch(patch, binary)?
                {
                    self.history_revision += 1;
                }
                self.revision += 1;
                json!({"revision":self.revision})
            }
            Command::PreparedView{key,view}=>{
                if self.packed.as_mut().ok_or("no prepared scene")?.patch_view(&key,view)?{self.history_revision+=1;}
                self.revision+=1;json!({"revision":self.revision})
            }
            Command::Snapshot => {
                return self.snapshot();
            }
            Command::ReleaseFrame { frame } => {
                #[cfg(all(feature = "metalfx", target_os = "macos"))]
                if let Some(target) = &mut self.live {
                    target.renderer.release_surface(frame);
                }
                json!({"released":frame})
            }
            Command::Hello => {
                json!({"protocol":VERSION,"layout":{"materialBytes":std::mem::size_of::<pm_studio::scene::GpuMaterial>(),"triangleBytes":std::mem::size_of::<pm_studio::scene::GpuTri>(),"nodeBytes":64,"instanceFloats":36},"renderer":"Powermove Studio","transport":"rgba-pipe","pid":std::process::id(),"oidn":cfg!(feature="oidn"),"capabilities":["scene","binaryMeshes","transformPatches","camera","time","pick","bounds","hardwareRayQueries","preparedView","preparedUpload","instanceParts","cancelRefine","environments","renderQuality","shutterAccumulation"],"peakRssMiB":peak_memory_mib(),"uptimeMs":self.start.elapsed().as_secs_f64()*1000.})
            }
            Command::Scene {
                scene,
                meshes,
                identities,
                transforms,
            } => {
                let mut r = ResidentScene::new(scene, meshes, binary, identities)?;
                r.patch_transforms(&transforms)?;
                self.install(r);
                json!({"revision":self.revision})
            }
            Command::Preset { name, finish, time } => {
                let scene = pm_studio::scene::preset(&name, &finish, time)
                    .ok_or("unknown corpus preset")?;
                self.install(ResidentScene::new(scene, vec![], &[], vec![])?);
                let (w, h) = (512, 512);
                self.screen = Some((w, h, 1, pm_studio::scene::screen_wallpaper(w, h, time)));
                json!({"revision":self.revision})
            }
            Command::Patch {
                transforms,
                camera,
                materials,
                lights,
                time,
            } => {
                if !time.is_finite() {
                    return Err("time must be finite".into());
                }
                let content_changed =
                    !transforms.is_empty() || materials.is_some() || lights.is_some();
                let r = self.resident.as_mut().ok_or("no scene")?;
                if materials
                    .as_ref()
                    .is_some_and(|m| m.len() != r.scene.materials.len())
                    || lights
                        .as_ref()
                        .is_some_and(|l| l.len() != r.scene.lights.len())
                {
                    return Err("material/light count changes require scene replacement".into());
                }
                if let Some(c) = camera {
                    scene::validate_camera(&c)?;
                }
                if let Some(m) = &materials { for m in m { m.validate()?; } }
                if let Some(l) = &lights { for l in l { l.validate()?; } }
                r.patch_transforms(&transforms)?;
                if content_changed {
                    self.history_revision += 1;
                }
                if let Some(c) = camera {
                    r.scene.camera = c;
                }
                let lighting_or_materials_changed=materials.is_some()||lights.is_some();
                if let Some(m) = materials {
                    r.scene.materials = m;
                }
                if let Some(l) = lights {
                    r.scene.lights = l;
                }
                if lighting_or_materials_changed {
                // Small material/light arrays can be rebuilt without touching meshes or BLAS.
                let metadata = Scene {
                    meters_per_unit:r.scene.meters_per_unit,
                    instances:vec![], scatters:vec![], terrains:vec![], displacements:vec![],
                    world: r.scene.world.clone(),
                    objects: vec![],
                    materials: r.scene.materials.clone(),
                    lights: r.scene.lights.clone(),
                    camera: r.scene.camera,
                    backdrop: r.scene.backdrop,
                    environment: r.scene.environment,
                    exposure: r.scene.exposure,
                    shadow_catcher: r.scene.shadow_catcher,
                    extra_lights: r.scene.extra_lights.clone(),
                    layers: None,
                }
                .prepare();
                r.prepared.materials = metadata.materials;
                r.prepared.lights = metadata.lights;
                let light_rows = r.prepared.lights.len() * 4;
                r.prepared.sdata[..light_rows].copy_from_slice(&metadata.sdata[..light_rows]);
                r.refresh_world();
                }
                self.revision += 1;
                json!({"revision":self.revision,"time":time})
            }
            Command::Textures {
                width,
                height,
                layers,
                layer,
            } => {
                if width==0 || height==0 || width>4096 || height>4096 {return Err("texture dimensions exceed native memory budget".into());}
                self.history_revision += 1;
                if layers == 0 || layers > 256 || width as u64*height as u64*layers as u64*4>256*1024*1024 {
                    return Err("texture layer budget exceeded".into());
                }
                let stride = width as usize * height as usize * 4;
                if let Some(layer) = layer {
                    let (w, h, n, data) =
                        self.screen.as_mut().ok_or("no material texture array")?;
                    if (*w, *h, *n) != (width, height, layers)
                        || layer >= layers
                        || binary.len() != stride
                    {
                        return Err("invalid texture layer patch".into());
                    }
                    data[layer as usize * stride..(layer as usize + 1) * stride]
                        .copy_from_slice(binary);
                    for t in [&mut self.live, &mut self.full].into_iter().flatten() {
                        t.renderer.update_map(binary, width, height, layer);
                    }
                } else {
                    if binary.len() != stride * layers as usize {
                        return Err("texture array byte count mismatch".into());
                    }
                    self.screen = Some((width, height, layers, binary.to_vec()));
                    for t in [&mut self.live, &mut self.full].into_iter().flatten() {
                        t.renderer.set_maps(binary, width, height, layers);
                    }
                }
                self.revision += 1;
                json!({"revision":self.revision})
            }
            Command::Screen { width, height } => {
                dimensions(width, height)?;
                if binary.len() != width as usize * height as usize * 4 {
                    return Err("screen RGBA byte count mismatch".into());
                }
                self.history_revision += 1;
                self.screen = Some((width, height, 1, binary.to_vec()));
                for target in [&mut self.live, &mut self.full].into_iter().flatten() {
                    target.renderer.set_screen(binary, width, height);
                }
                self.revision += 1;
                json!({"revision":self.revision})
            }
            Command::Bounds { layer } => {
                if let Some(r) = &self.resident {
                    let bounds = (0..r.transforms.len())
                        .filter(|&i| layer.as_ref().is_none_or(|l| &r.identities[r.object_indices[i]].layer == l))
                        .map(|i| r.object_bounds(i))
                        .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)));
                    bounds
                        .map(|(a, b)| json!({"min":a.to_array(),"max":b.to_array()}))
                        .unwrap_or(Value::Null)
                } else {
                    self.packed
                        .as_ref()
                        .ok_or("no scene")?
                        .bounds(layer.as_deref())
                        .map(|(a, b)| json!({"min":a.to_array(),"max":b.to_array()}))
                        .unwrap_or(Value::Null)
                }
            }
            Command::Pick {
                x,
                y,
                width,
                height,
            } => {
                dimensions(width, height)?;
                if !x.is_finite()
                    || !y.is_finite()
                    || x < 0.
                    || y < 0.
                    || x >= width as f32
                    || y >= height as f32
                {
                    return Err("pick outside frame".into());
                }
                if let Some(target) = self.live.as_mut().or(self.full.as_mut()) {
                    let edited = target.revision != self.revision;
                    sync_target(
                        target,
                        self.resident.as_ref(),
                        self.packed.as_ref(),
                        self.revision,
                        self.geometry,
                        self.history_revision,
                    );
                    if edited {
                        target.time = f64::NAN;
                    }
                    if let Some(hit) = target
                        .renderer
                        .pick_surface((x + 0.5) / width as f32, (y + 0.5) / height as f32)
                    {
                        let (layer, part) = if let Some(p) = &self.packed {
                            let id = &p.objects[hit.layer as usize];
                            (
                                id.layer.clone(),
                                p.parts
                                    .get(hit.instance as usize)
                                    .and_then(|p| p.clone())
                                    .unwrap_or_else(|| id.part.clone()),
                            )
                        } else {
                            let r = self.resident.as_ref().unwrap();
                            let object = r.placements[hit.instance as usize].geometry;
                            let id = &r.identities[object];
                            (id.layer.clone(), id.part.clone())
                        };
                        json!({"layer":layer,"part":part,"point":hit.point,"normal":hit.normal})
                    } else {
                        Value::Null
                    }
                } else {
                    if let Some(r) = &self.resident {
                        r.pick(x, y, width, height)
                    } else {
                        self.packed
                            .as_ref()
                            .ok_or("no scene")?
                            .pick(x, y, width, height)
                    }
                    .unwrap_or(Value::Null)
                }
            }

            Command::Render {
                width,
                height,
                tier,
                samples,
                adaptive,
                time_budget_ms, min_samples, noise_threshold, max_bounces, caustics, shutter,
                temporal,
                reset,
                goal,
                denoise,
                shared_texture,
                layer,
                background,
                time,
            } => {
                dimensions(width, height)?;
                if !time.is_finite() {
                    return Err("time must be finite".into());
                }
                if !["live", "refine", "final"].contains(&tier.as_str()) {
                    return Err("unknown quality tier".into());
                }
                if samples == 0 || samples > 4096 {
                    return Err("samples must be 1–4096".into());
                }
                let is_live = tier == "live";
                if min_samples.is_some_and(|n| n == 0 || n > 4096) || max_bounces.is_some_and(|n| n == 0 || n > 32)
                    || noise_threshold.is_some_and(|n| !n.is_finite() || !(0.0..=1.0).contains(&n)) || time_budget_ms > 3_600_000 {
                    return Err("invalid render quality settings".into());
                }
                if shutter.as_ref().is_some_and(|s|s.key.len()>256 || s.count<2 || s.count>64 || s.index>=s.count || layer.is_some() || is_live || temporal) {return Err("invalid shutter accumulation".into());}
                let policy = format!("{min_samples:?}:{noise_threshold:?}:{max_bounces:?}:{caustics:?}");
                let cancellation=self.cancel_revision.clone();let generation=self.request_generation;
                let cancelled=||tier=="refine"&&cancellation.load(Ordering::Relaxed)!=generation;
                if cancelled(){return Ok((json!({"cancelled":true}),vec![]));}
                let (w, h) = if is_live {
                    // Keep Live within a predictable pixel budget for small worlds.
                    // Final/Refine always render at the requested output size.
                    let (instances, lights, media) = if let Some(p) = &self.packed {
                        (p.placements.len(), p.prepared.lights.len(),pm_studio::environment::packed_has_media(&p.prepared.sdata,p.prepared.lights.len(),p.placements.len()))
                    } else {
                        let r = self.resident.as_ref().ok_or("no scene")?;
                        (r.placements.len(), r.prepared.lights.len(),pm_studio::environment::packed_has_media(&r.prepared.sdata,r.prepared.lights.len(),r.placements.len()))
                    };
                    self.live_budget.atmosphere(media);
                    self.live_budget
                        .size(width, height, instances > 128 || lights > 32)
                } else {
                    (width, height)
                };
                if self.resident.is_none() && self.packed.is_none() {
                    return Err("no scene".into());
                }
                self.frame_counter = self.frame_counter.wrapping_add(1);
                let frame_id = self.frame_counter;
                let prepared = self.packed.as_ref().map(|p| &p.prepared)
                    .or_else(|| self.resident.as_ref().map(|r| &r.prepared)).ok_or("no scene")?;
                let instances = self.packed.as_ref().map(|p| p.placements.len())
                    .or_else(|| self.resident.as_ref().map(|r| r.placements.len())).unwrap_or(0);
                let scene_bytes = pm_studio::validation::validate_scene_budget(prepared.tris.len(),instances,prepared.materials.len(),prepared.sdata.len()*16)? as u64;
                let maps_bytes = self.screen.as_ref().map_or(0,|(_,_,_,bytes)|bytes.len() as u64);
                let needed = pm_studio::validation::target_bytes(w,h,u64::MAX)? + scene_bytes + maps_bytes
                    + if is_live {u64::from(width)*u64::from(height)*32} else {0};
                if needed > pm_studio::validation::SESSION_BUDGET_BYTES {
                    return Err("Studio exceeds the 2 GiB session budget; reduce mesh detail, scatter count or output size".into());
                }
                let other = if is_live { &mut self.full } else { &mut self.live };
                let other_bytes = other.as_ref().map_or(0,|t|{
                    let (w,h)=t.renderer.size();
                    pm_studio::validation::target_bytes(w,h,u64::MAX).unwrap_or(u64::MAX/2)+scene_bytes+maps_bytes
                });
                if other_bytes + needed > pm_studio::validation::SESSION_BUDGET_BYTES { *other = None; }
                let slot = if is_live {
                    &mut self.live
                } else {
                    &mut self.full
                };
                let init = Instant::now();
                if let Some(target)=slot.as_mut() {
                    if target.renderer.resize(w,h)? {target.time=f64::NAN;}
                }
                if slot.is_none() {
                    let mut renderer = Renderer::new(w, h)?;
                    if let Some((sw, sh, layers, p)) = &self.screen {
                        renderer.set_maps(p, *sw, *sh, *layers);
                    }
                    *slot = Some(Target {
                        renderer,
                        revision: u64::MAX,
                        geometry: u64::MAX,
                        history_revision: u64::MAX,
                        scene_revision: u64::MAX,
                        live: is_live,
                        denoiser: "none",
                        quality_denoise: false,
                        quality_samples: 0,
                        quality_adaptive: false,
                        quality_temporal: false,
                        quality_budget_ms: 0,
                        quality_policy: String::new(),
                        shutter: None,
                        shutter_policy:String::new(),
                        tier: String::new(),
                        time: f64::NAN,
                    });
                }
                let init_ms = init.elapsed().as_secs_f64() * 1000.;
                let target = slot.as_mut().unwrap();
                let changed = target.revision != self.revision
                    || target.time != time
                    || target.tier != tier
                    || target.quality_denoise != denoise
                    || target.quality_adaptive != adaptive
                    || target.quality_temporal != temporal
                    || target.quality_budget_ms != time_budget_ms
                    || target.quality_policy != policy
                    || tier == "final" && target.quality_samples != samples;
                let post_policy=if let Some(p)=&self.packed {format!("{}:{:?}:{}:{}",p.view.exposure,p.view.backdrop,p.view.backdrop_opaque,p.view.shadow_catcher)}
                    else {let r=self.resident.as_ref().unwrap();format!("{}:{:?}:{}",r.scene.exposure,r.scene.backdrop,r.scene.shadow_catcher)};
                let frame_policy=format!("{policy}:{tier}:{samples}:{width}:{height}:{denoise}:{time_budget_ms}:{post_policy}");
                let shutter_samples=if let Some(s)=&shutter {
                    if s.index==0 {0} else {
                        match &target.shutter {Some((key,index,count,samples)) if key==&s.key && *index+1==s.index && *count==s.count && target.shutter_policy==frame_policy=>*samples,_=>return Err("shutter samples must arrive in order".into())}
                    }
                } else {0};
                sync_target(
                    target,
                    self.resident.as_ref(),
                    self.packed.as_ref(),
                    self.revision,
                    self.geometry,
                    self.history_revision,
                );
                let renderer = &mut target.renderer;
                renderer.live = target.live;
                renderer.budget_batch_samples=if min_samples.is_some() || pm_studio::environment::packed_has_media(&prepared.sdata,prepared.lights.len(),instances) {1} else {4};
                if let Some(s)=&shutter {renderer.begin_shutter(s.index==0,shutter_samples,(time*9600.).round() as u32);}else{renderer.end_shutter();target.shutter=None;}
                renderer.caustics_enabled = caustics.unwrap_or(true);
                renderer.set_precomputed_caustics(min_samples.is_some() && noise_threshold.is_some_and(|n|n>=0.025));
                renderer.guide_samples = match noise_threshold { Some(n) if n >= 0.06 => 8, Some(n) if n >= 0.025 => 32, _ => 64 };
                if let Some(s)=&shutter {renderer.guide_samples=(renderer.guide_samples*2).div_ceil(s.count).clamp(1,64);}
                renderer.quality_temporal = temporal && tier == "final" && denoise;
                // Cuts, seeks and switching quality modes must not reuse history.
                if !target.time.is_finite() || time < target.time || time - target.time > 0.1 || target.quality_temporal != temporal {
                    renderer.invalidate_history();
                }
                if target.history_revision != self.history_revision {
                    renderer.invalidate_history();
                    target.history_revision = self.history_revision;
                }
                renderer.set_output_mask(layer, background);
                renderer.set_straight_output(true);
                renderer.set_time_seed((time * 9600.).round() as u32);
                let adaptive = adaptive && !is_live && shutter.is_none();
                renderer.set_adaptive_sampling_with_floor(pm_studio::environment::AdaptiveSampling {
                    enabled: adaptive, min_samples: min_samples.unwrap_or(64), relative_error: noise_threshold.unwrap_or(0.015), absolute_error: 0.0005,
                }, min_samples.is_some());
                let spp = if let Some(s)=shutter.as_ref().filter(|s|s.index>0) {shutter_samples/s.index} else if goal && !changed && !reset && (adaptive || time_budget_ms>0) && tier == "final" {
                    // The first request already completed its bounded quality
                    // render. Layer masks must reuse that same finished trace.
                    0
                } else if goal && !changed && !reset {
                    samples.saturating_sub(renderer.samples)
                } else if is_live {
                    1
                } else {
                    samples
                };
                let trace_ms = if spp == 0 {
                    0.
                } else {
                    renderer.trace_budgeted_interruptible(
                        spp,
                        if is_live { 3 } else { max_bounces.unwrap_or(8) },
                        shutter.as_ref().map_or(reset || changed || is_live,|s|s.index==0),
                        if is_live { 12 } else { 64 },
                        min_samples.unwrap_or(if adaptive { 256 } else { 0 }).min(samples),
                        if is_live || shutter.as_ref().is_some_and(|s|s.index>0) { 0 } else { time_budget_ms },
                        &cancelled,
                    )
                };
                if cancelled(){return Ok((json!({"cancelled":true}),vec![]));}
                if let Some(s)=&shutter {
                    if denoise {renderer.accumulate_shutter_guides(s.index);}
                    target.shutter=Some((s.key.clone(),s.index,s.count,renderer.samples));target.shutter_policy=frame_policy;
                    if s.index+1<s.count {return Ok((json!({"deferred":true,"samples":renderer.samples,"width":w,"height":h,"hardwareRt":renderer.uses_hardware_rt(),"timings":{"traceMs":trace_ms,"postMs":0.,"hostMs":begin.elapsed().as_secs_f64()*1000.}}),vec![]));}
                }
                renderer.last_surface = None;
                let mut denoiser = target.denoiser;
                let mut frame_size = (w, h);
                #[cfg(all(feature = "metalfx", target_os = "macos"))]
                let metal_result = if is_live && denoise && renderer.metalfx_enabled {
                    match renderer.finish_metalfx(
                        width,
                        height,
                        spp == 0,
                        if shared_texture { Some(frame_id) } else { None },
                    ) {
                        Ok(frame) => {
                            denoiser = "metalfx";
                            frame_size = (width, height);
                            Some(frame)
                        }
                        Err(error) => {
                            eprintln!("Studio MetalFX fallback: {error}");
                            renderer.metalfx_enabled = false;
                            None
                        }
                    }
                } else {
                    None
                };
                #[cfg(not(all(feature = "metalfx", target_os = "macos")))]
                let metal_result: Option<(Vec<u8>, f64)> = None;
                #[cfg(feature = "oidn")]
                let (pixels, post_ms) = if let Some(frame) = metal_result {
                    frame
                } else if spp == 0 && denoiser != "metalfx" {
                    renderer.frame_pixels()
                // Final divides its total budget across shutter times. Keep
                // requested denoising on those smaller batches as well.
                } else if denoise && !is_live && noise_threshold.is_some_and(|n| n >= 0.1) {
                    denoiser = "atrous";
                    renderer.finish(3)
                } else if denoise && !is_live {
                    if self.oidn.is_none() {
                        self.oidn = Some(pm_studio::oidn::Denoiser::new()?);
                    }
                    denoiser = "oidn";
                    let d = self.oidn.as_ref().unwrap();
                    d.set_quality(match noise_threshold { Some(n) if n>=0.06 => pm_studio::oidn::Quality::Fast, Some(n) if n>=0.025 => pm_studio::oidn::Quality::Balanced, _ => pm_studio::oidn::Quality::High });
                    renderer.finish_with_layers(&|c, shade, a, n| d.run_layers_interruptible(&mut [c,shade],a,n,4,w as usize,h as usize,if tier=="refine"{Some((&cancellation,generation))}else{None}))
                } else {
                    denoiser = if is_live && denoise { "atrous" } else { "none" };
                    renderer.finish(if is_live && denoise { 3 } else { 0 })
                };
                if cancelled(){return Ok((json!({"cancelled":true}),vec![]));}
                #[cfg(not(feature = "oidn"))]
                let (pixels, post_ms) = if let Some(frame) = metal_result {
                    frame
                } else if spp == 0 && denoiser != "metalfx" {
                    renderer.frame_pixels()
                } else {
                    denoiser = if is_live && denoise { "atrous" } else { "none" };
                    renderer.finish(if is_live && denoise { 3 } else { 0 })
                };
                if is_live && spp > 0 && init_ms < 1. {
                    self.live_budget.observe(trace_ms + post_ms);
                }
                target.denoiser = denoiser;
                target.quality_denoise = denoise;
                target.quality_samples = samples;
                target.quality_adaptive = adaptive;
                target.quality_temporal = temporal;
                target.quality_budget_ms = time_budget_ms;
                target.quality_policy = policy;
                target.time = time;
                target.tier = tier.clone();
                let gpu_bytes=renderer.allocated_gpu_bytes();self.peak_gpu_bytes=self.peak_gpu_bytes.max(gpu_bytes.unwrap_or(0));
                return Ok((
                    json!({"pid":std::process::id(),"width":frame_size.0,"height":frame_size.1,"outputWidth":width,"outputHeight":height,"inputWidth":w,"inputHeight":h,"format":"rgba8","premultiplied":false,"surface":renderer.last_surface,"tier":tier,"samples":renderer.samples,"revision":self.revision,"time":time,"hardwareRt":renderer.uses_hardware_rt(),"adapter":renderer.adapter,"denoiser":denoiser,"gpuAllocatedMiB":gpu_bytes.map(|b|b as f64/1048576.),"peakGpuMiB":self.peak_gpu_bytes as f64/1048576.,"peakRssMiB":peak_memory_mib(),"timings":{"initMs":init_ms,"traceMs":trace_ms,"postMs":post_ms,"hostMs":begin.elapsed().as_secs_f64()*1000.}}),
                    pixels,
                ));
            }
            Command::Shutdown => json!({"shutdown":true}),
        };
        Ok((value, vec![]))
    }
}
fn sync_target(
    target: &mut Target,
    resident: Option<&ResidentScene>,
    packed: Option<&packed::PackedScene>,
    revision: u64,
    geometry: u64,
    scene_revision: u64,
) {
    if target.revision == revision {
        return;
    }
    let content_changed = target.scene_revision != scene_revision || target.geometry != geometry;
    if let Some(r) = resident {
        if content_changed {
            target.renderer.set_instanced_scene(
                &r.scene,
                &r.prepared,
                &r.geometries,
                &r.placements,
                r.top_nodes,
                target.geometry != geometry,
            );
        } else {
            target.renderer.set_camera(&r.scene);
        }
        let (lo, hi) = r.bounds();
        let center = (lo + hi) * 0.5;
        target.renderer.set_scene_bounds([
            center.x,
            center.y,
            center.z,
            (hi - lo).length().max(1e-3) * 0.5,
        ]);
    } else if let Some(p) = packed {
        if content_changed {
            target.renderer.set_instanced_scene(
                &p.view.scene(),
                &p.prepared,
                &p.geometries,
                &p.placements,
                p.top_nodes,
                target.geometry != geometry,
            );
        } else {
            target.renderer.set_camera(&p.view.scene());
        }
        target.renderer.set_view(
            p.view.basis(),
            p.view.ground(),
            p.view.shadows,
            p.view.bounds,
        );
        target.renderer.set_iris(p.view.camera.iris);
        target.renderer.set_ambient(p.view.ambient);
        target.renderer.set_projection(p.view.camera.orthographic);
        target
            .renderer
            .set_clip_planes(p.view.camera.near, p.view.camera.far);
        target.renderer.set_void(if p.view.backdrop_opaque {
            Some(p.view.backdrop)
        } else {
            None
        });
    }
    target.revision = revision;
    target.geometry = geometry;
    target.scene_revision = scene_revision;
}
fn peak_memory_mib() -> Option<f64> {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        // SAFETY: getrusage initializes the caller-owned rusage on success.
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
            return None;
        }
        let usage = unsafe { usage.assume_init() };
        let unit = if cfg!(target_os = "macos") { 1. } else { 1024. };
        Some(usage.ru_maxrss as f64 * unit / (1024. * 1024.))
    }
    #[cfg(not(unix))]
    {
        None
    }
}

fn dimensions(w: u32, h: u32) -> Result<(), String> {
    if w == 0 || h == 0 || w > 4096 || h > 4096 || w as u64 * h as u64 > 8_847_360 {
        Err("frame dimensions exceed native memory budget".into())
    } else {
        Ok(())
    }
}
fn main() {
    let mut output = io::BufWriter::new(io::stdout().lock());
    let mut host = Host::new();
    let cancellation=host.cancel_revision.clone();
    // Read ahead by four bounded packets so input can interrupt a running
    // Refine. GPU ownership and all scene mutation remain on this thread.
    let (sender,receiver)=std::sync::mpsc::sync_channel(4);
    let queued_bytes=Arc::new(AtomicUsize::new(0));let queued=queued_bytes.clone();
    std::thread::spawn(move||{
        let mut input=io::stdin().lock();
        loop{
            match protocol::read(&mut input){
                Ok(Some(packet))=>{
                    if matches!(packet.json.get("op").and_then(Value::as_str),Some("cancelRefine"|"shutdown")){cancellation.fetch_add(1,Ordering::Relaxed);}
                    let generation=cancellation.load(Ordering::Relaxed);
                    let bytes=packet.binary.len()+packet.json.to_string().len();
                    if queued.fetch_add(bytes,Ordering::Relaxed)+bytes>protocol::MAX_BINARY+4*protocol::MAX_JSON{
                        queued.fetch_sub(bytes,Ordering::Relaxed);cancellation.fetch_add(1,Ordering::Relaxed);
                        let _=sender.send(Err("queued Studio input exceeds memory budget".into()));break;
                    }
                    if sender.send(Ok((packet,generation,bytes))).is_err(){break;}
                }
                Ok(None)=>break,
                Err(error)=>{let _=sender.send(Err(error.to_string()));break;}
            }
        }
    });
    loop {
        let Packet { json, binary } = match receiver.recv() {
            Ok(Ok((packet,generation,bytes)))=>{queued_bytes.fetch_sub(bytes,Ordering::Relaxed);host.request_generation=generation;packet},
            Err(_)=>break,
            Ok(Err(e)) => {
                eprintln!("Studio protocol: {e}");
                std::process::exit(2)
            }
        };
        let id = json.get("id").and_then(Value::as_u64).unwrap_or(0);
        let result = serde_json::from_value::<Request>(json)
            .map_err(|e| e.to_string())
            .and_then(|r| {
                let _ = r.id;
                host.handle(r.command, &binary)
            });
        let (response, bytes, shutdown) = match result {
            Ok((value, bytes)) => {
                let shutdown = value.get("shutdown") == Some(&Value::Bool(true));
                (json!({"id":id,"ok":true,"result":value}), bytes, shutdown)
            }
            Err(e) => (json!({"id":id,"ok":false,"error":e}), vec![], false),
        };
        if let Err(e) = protocol::write(&mut output, &response, &bytes) {
            eprintln!("Studio output: {e}");
            break;
        }
        if shutdown {
            break;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handles_invalid_edits_without_losing_scene() {
        let mut h = Host::new();
        h.handle(
            Command::Preset {
                name: "hero".into(),
                finish: blue(),
                time: 0.,
            },
            &[],
        )
        .unwrap();
        assert!(
            h.handle(
                Command::Patch {
                    transforms: vec![Transform {
                        object: 999,
                        matrix: scene::IDENTITY
                    }],
                    camera: None,
                    materials: None,
                    lights: None,
                    time: 0.
                },
                &[]
            )
            .is_err()
        );
        assert!(h.handle(Command::Bounds { layer: None }, &[]).is_ok());
    }
    #[test]
    fn oversized_scene_is_rejected_before_bvh_build_and_preserves_previous_scene() {
        let mut h=Host::new();
        h.handle(Command::Preset{name:"hero".into(),finish:blue(),time:0.},&[]).unwrap();
        let revision=h.revision;
        let old_positions=h.resident.as_ref().unwrap().prepared.positions.clone();
        let mut scene=pm_studio::scene::preset("hero","blue",0.).unwrap();
        scene.objects.truncate(1);
        scene.objects[0].mesh.indices.resize(1_100_000,[0,1,2]);
        let error=h.handle(Command::Scene{scene,meshes:vec![],identities:vec![],transforms:vec![]},&[]).unwrap_err();
        assert!(error.contains("budget"));
        assert_eq!(h.revision,revision);
        assert_eq!(h.resident.as_ref().unwrap().prepared.positions,old_positions);
        assert!(h.live.is_none() && h.full.is_none());
        assert!(h.handle(Command::Bounds{layer:None},&[]).is_ok());
    }
    #[test]
    fn metal_instances_keep_the_glass_sphere() {
        let scene = pm_studio::scene::preset("chrome", "blue", 0.).unwrap();
        let r = ResidentScene::new(scene, vec![], &[], vec![]).unwrap();
        let mut gpu = Renderer::new(960, 540).expect("native Studio test needs a GPU");
        gpu.set_instanced_scene(
            &r.scene,
            &r.prepared,
            &r.geometries,
            &r.placements,
            r.top_nodes,
            true,
        );
        let hit = gpu.probe(635., 320.);
        assert_eq!(
            hit[1], hit[5],
            "hardware and BVH must see the same glass triangle"
        );
        assert!((hit[0] - hit[4]).abs() < 0.001);
        assert_eq!(hit[11], 1.0, "the frontmost material is glass");
        let pick = gpu
            .pick_surface(635. / 960., 320. / 540.)
            .expect("glass is pickable");
        assert!(pick.point.iter().all(|v| v.is_finite()));
        assert!((glam::Vec3::from(pick.normal).length() - 1.).abs() < 1e-4);
        assert_eq!(pick.instance, hit[2] as u32);
    }

    #[test]
    fn chunked_prepared_upload_is_atomic_and_preserves_part_ids() {
        let mut h=Host::new();h.handle(Command::Preset{name:"hero".into(),finish:blue(),time:0.},&[]).unwrap();
        let (mut value,mut binary)=h.snapshot().unwrap();value["key"]=json!("chunks");
        let count=value["instanceCount"].as_u64().unwrap() as usize;
        let offset=binary.len();for _ in 0..count{binary.extend_from_slice(&0u32.to_le_bytes());}
        value["instanceParts"]=json!({"names":["body"],"indices":{"offset":offset,"length":count*4}});
        let upload=serde_json::from_value(value).unwrap();let revision=h.revision;
        h.handle(Command::PreparedBegin{upload,bytes:binary.len()},&[]).unwrap();
        for (i,chunk) in binary.chunks(65536).enumerate(){h.handle(Command::PreparedChunk{offset:i*65536},chunk).unwrap();}
        assert_eq!(h.revision,revision);assert!(h.resident.is_some());
        h.handle(Command::PreparedCommit,&[]).unwrap();
        assert_eq!(h.revision,revision+1);assert!(h.resident.is_none());
        assert_eq!(h.packed.as_ref().unwrap().parts,vec![Some("body".into());count]);assert!(h.upload.is_none());
    }
    #[test]
    fn bad_or_incomplete_upload_preserves_the_installed_scene() {
        let mut h=Host::new();h.handle(Command::Preset{name:"hero".into(),finish:blue(),time:0.},&[]).unwrap();
        let (mut value,binary)=h.snapshot().unwrap();value["key"]=json!("bad-chunks");let revision=h.revision;
        let upload=serde_json::from_value(value.clone()).unwrap();h.handle(Command::PreparedBegin{upload,bytes:binary.len()},&[]).unwrap();
        assert!(h.handle(Command::PreparedChunk{offset:1},&[1,2]).is_err());assert!(h.upload.is_none());
        let upload=serde_json::from_value(value.clone()).unwrap();h.handle(Command::PreparedBegin{upload,bytes:binary.len()},&[]).unwrap();
        assert!(h.handle(Command::PreparedCommit,&[]).is_err());assert!(h.upload.is_none());
        let upload=serde_json::from_value(value).unwrap();assert!(h.handle(Command::PreparedBegin{upload,bytes:protocol::MAX_BINARY+1},&[]).is_err());
        assert_eq!(h.revision,revision);assert!(h.resident.is_some());
    }
    #[test]
    fn prepared_bounds_and_identity_survive_binary_upload() {
        let mut h = Host::new();
        h.handle(
            Command::Preset {
                name: "chrome".into(),
                finish: blue(),
                time: 0.,
            },
            &[],
        )
        .unwrap();
        let (mut value, binary) = h.snapshot().unwrap();
        value["key"] = json!("test");
        let count = value["instanceCount"].as_u64().unwrap() as usize;
        value["instancePolicies"] = json!(
            (0..count)
                .map(|i| json!({"part":format!("part-{i}")}))
                .collect::<Vec<_>>()
        );
        let u: packed::Upload = serde_json::from_value(value).unwrap();
        let mut packed = packed::PackedScene::new(u, &binary).unwrap();
        let positions=packed.prepared.positions.as_ptr();let instances=packed.prepared.sdata.as_ptr();
        let mut view=packed.view.clone();view.camera.position[0]+=1.;
        assert!(!packed.patch_view("test",view.clone()).unwrap());
        assert_eq!(positions,packed.prepared.positions.as_ptr());assert_eq!(instances,packed.prepared.sdata.as_ptr());
        assert!(packed.patch_view("stale",view.clone()).is_err());
        view.shadows=!view.shadows;assert!(packed.patch_view("test",view).unwrap());
        // Restore the original view for the picking assertions below.
        let original:packed::View=serde_json::from_value(h.snapshot().unwrap().0["view"].clone()).unwrap();packed.patch_view("test",original).unwrap();
        let expected = h.resident.as_ref().unwrap().bounds();
        let actual = packed.bounds(None).unwrap();
        assert!((glam::Vec3::from(expected.0.to_array()) - actual.0).length() < 1e-3);
        assert!((glam::Vec3::from(expected.1.to_array()) - actual.1).length() < 1e-3);
        assert!(packed.bounds(Some("missing-layer")).is_none());
        let hit = packed.pick(635., 320., 960, 540).unwrap();
        assert!(hit["part"].as_str().unwrap().starts_with("part-"));
        let layer = hit["layer"].as_str().unwrap();
        let subset = packed.bounds(Some(layer)).unwrap();
        assert!(subset.0.cmpge(actual.0).all() && subset.1.cmple(actual.1).all());
    }

    #[test]
    fn accepts_empty_prepared_scenes_with_gpu_padding() {
        let mut h = Host::new();
        h.handle(
            Command::Preset {
                name: "hero".into(),
                finish: blue(),
                time: 0.,
            },
            &[],
        )
        .unwrap();
        let (mut value, binary) = h.snapshot().unwrap();
        value["key"] = json!("empty");
        value["instanceCount"] = json!(0);
        value["lightCount"] = json!(0);
        value["objects"] = json!([]);
        value["topNodes"] = json!(1);
        value["positions"]["length"] = json!(48);
        value["tris"]["length"] = json!(std::mem::size_of::<pm_studio::scene::GpuTri>());
        value["nodes"]["length"] = json!(64);
        value["sdata"]["length"] = json!(16);
        let empty =
            packed::PackedScene::new(serde_json::from_value(value).unwrap(), &binary).unwrap();
        assert!(empty.bounds(None).is_none());
        assert!(empty.pick(10., 10., 20, 20).is_none());
    }
    #[test]
    fn accepts_4k_material_images_and_rejects_arrays_above_the_memory_budget() {
        let mut h=Host::new();
        let pixels=vec![0u8;4096*4096*4];
        assert!(h.handle(Command::Textures{width:4096,height:4096,layers:1,layer:None},&pixels).is_ok());
        assert!(h.handle(Command::Textures{width:4096,height:4096,layers:5,layer:None},&[]).is_err());
        assert!(h.handle(Command::Textures{width:1,height:1,layers:257,layer:None},&[]).is_err());
    }

    #[cfg(feature="oidn")]
    #[test]
    fn shutter_strata_share_one_accumulation_and_one_denoise() {
        let mut h=Host::new();h.handle(Command::Preset{name:"hero".into(),finish:blue(),time:0.},&[]).unwrap();
        let mut command=json!({"op":"render","width":64,"height":48,"tier":"final","samples":8,"minSamples":4,"noiseThreshold":0.03,
            "adaptive":true,"maxBounces":4,"caustics":false,"denoise":true,"goal":true,"reset":true,
            "shutter":{"key":"frame-1","index":0,"count":2}});
        let (first,pixels)=h.handle(serde_json::from_value(command.clone()).unwrap(),&[]).unwrap();
        assert_eq!(first["deferred"],true);assert!(pixels.is_empty());assert_eq!(first["samples"],8);
        command["shutter"]["index"]=json!(1);command["time"]=json!(1./30.);
        let (last,pixels)=h.handle(serde_json::from_value(command.clone()).unwrap(),&[]).unwrap();
        assert_eq!(last["samples"],16);assert_eq!(last["denoiser"],"oidn");assert_eq!(pixels.len(),64*48*4);
        command["shutter"]["key"]=json!("wrong-frame");
        assert!(h.handle(serde_json::from_value(command).unwrap(),&[]).is_err());
    }

    #[test]
    fn explicit_quality_floor_and_policy_changes_are_honored() {
        let mut h=Host::new();
        h.handle(Command::Preset{name:"hero".into(),finish:blue(),time:0.},&[]).unwrap();
        let command=json!({"op":"render","width":48,"height":32,"tier":"final","samples":128,
            "minSamples":8,"noiseThreshold":0.03,"timeBudgetMs":1,"maxBounces":4,"caustics":false,"adaptive":true,"denoise":false,"goal":true});
        let (first,_)=h.handle(serde_json::from_value(command.clone()).unwrap(),&[]).unwrap();
        assert!(first["samples"].as_u64().unwrap()>=1 && first["samples"].as_u64().unwrap()<128);
        let (same,_)=h.handle(serde_json::from_value(command.clone()).unwrap(),&[]).unwrap();
        assert_eq!(same["timings"]["traceMs"],0.);
        let mut changed=command.clone();changed["maxBounces"]=json!(6);
        let (changed,_)=h.handle(serde_json::from_value(changed).unwrap(),&[]).unwrap();
        assert!(changed["timings"]["traceMs"].as_f64().unwrap()>0.);
        let mut timed=command.clone();timed["adaptive"]=json!(false);timed["noiseThreshold"]=json!(0.);
        let (timed_first,_)=h.handle(serde_json::from_value(timed.clone()).unwrap(),&[]).unwrap();
        timed["layer"]=json!(0);
        let (timed_mask,_)=h.handle(serde_json::from_value(timed).unwrap(),&[]).unwrap();
        assert_eq!(timed_mask["samples"],timed_first["samples"]);assert_eq!(timed_mask["timings"]["traceMs"],0.);
        for (key,value) in [("minSamples",json!(0)),("maxBounces",json!(33)),("noiseThreshold",json!(-1.))] {
            let mut invalid=command.clone();invalid[key]=value;
            assert!(h.handle(serde_json::from_value(invalid).unwrap(),&[]).is_err());
        }
    }

    #[test]
    fn rejects_bad_render_sizes() {
        assert!(dimensions(0, 10).is_err());
        assert!(dimensions(99999, 10).is_err());
        assert!(dimensions(1920, 1080).is_ok());
    }

    #[cfg(feature = "oidn")]
    #[test]
    fn bounded_final_honors_the_deadline_and_reuses_it_for_layer_masks() {
        let mut host = Host::new();
        host.handle(Command::Preset { name: "chrome".into(), finish: blue(), time: 0. }, &[]).unwrap();
        let request = json!({"op":"render","width":48,"height":32,"tier":"final","samples":1024,"adaptive":true,"timeBudgetMs":1,"goal":true,"denoise":true});
        let (first, _) = host.handle(serde_json::from_value(request.clone()).unwrap(), &[]).unwrap();
        let samples = first["samples"].as_u64().unwrap();
        assert!(samples >= 1 && samples < 1024, "time limit must win over a noise-stopping floor: {first}");
        let mut masked = request; masked["layer"] = json!(0);
        let (second, _) = host.handle(serde_json::from_value(masked).unwrap(), &[]).unwrap();
        assert_eq!(second["samples"], first["samples"], "a layer mask cannot multiply the Final budget");
        assert_eq!(second["timings"]["traceMs"], 0.);
        // A raw reference after an adaptive image must restart without stopping.
        let raw = json!({"op":"render","width":48,"height":32,"tier":"final","samples":256,"adaptive":false,"denoise":false,"goal":true});
        let (reference, _) = host.handle(serde_json::from_value(raw).unwrap(), &[]).unwrap();
        assert_eq!(reference["samples"], 256);
        assert_eq!(reference["denoiser"], "none");
        assert!(host.full.as_ref().unwrap().renderer.sample_counts().iter().all(|n| *n == 256.));
    }

    #[cfg(feature = "oidn")]
    #[test]
    fn final_shutter_subsamples_keep_requested_denoising() {
        let mut host = Host::new();
        host.handle(Command::Preset { name: "chrome".into(), finish: blue(), time: 0. }, &[]).unwrap();
        for samples in [4, 8] {
            let command = serde_json::from_value(json!({"op":"render","width":64,"height":64,"tier":"final","samples":samples,"reset":true,"denoise":true})).unwrap();
            let (result, pixels) = host.handle(command, &[]).unwrap();
            assert_eq!(result["denoiser"], "oidn");
            assert_eq!(pixels.len(), 64 * 64 * 4);
        }
    }
    #[cfg(feature="oidn")]
    #[test]
    fn cancelled_refine_yields_to_input_and_final_keeps_its_sample_budget() {
        let mut host=Host::new();
        host.handle(Command::Preset{name:"hero".into(),finish:blue(),time:0.},&[]).unwrap();
        host.cancel_revision.store(1,Ordering::Relaxed);
        let refine=serde_json::from_value(json!({"op":"render","width":64,"height":64,"tier":"refine","samples":32,"reset":true,"denoise":true})).unwrap();
        let (result,pixels)=host.handle(refine,&[]).unwrap();assert_eq!(result["cancelled"],true);assert!(pixels.is_empty());
        let final_frame=serde_json::from_value(json!({"op":"render","width":64,"height":64,"tier":"final","samples":64,"reset":true,"denoise":true})).unwrap();
        let (result,pixels)=host.handle(final_frame,&[]).unwrap();assert_eq!(result["samples"],64);assert_eq!(result["denoiser"],"oidn");assert_eq!(pixels.len(),64*64*4);
    }
}
