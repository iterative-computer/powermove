//! Powermove Studio's isolated, headless native process. No Electron/Node dependency.
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

#[derive(Deserialize)]
struct Request {
    id: u64,
    #[serde(flatten)]
    command: Command,
}
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
enum Command {
    Hello,
    Prepared {
        #[serde(flatten)]
        upload: packed::Upload,
    },
    PreparedPatch {
        #[serde(flatten)]
        patch: packed::Patch,
    },
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
        reset: bool,
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
    Bounds,
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
    live: bool,
}
struct Host {
    packed: Option<packed::PackedScene>,
    resident: Option<ResidentScene>,
    live: Option<Target>,
    full: Option<Target>,
    revision: u64,
    geometry: u64,
    screen: Option<(u32, u32, u32, Vec<u8>)>,
    start: Instant,
    #[cfg(feature = "oidn")]
    oidn: Option<pm_studio::oidn::Denoiser>,
}
impl Host {
    fn new() -> Self {
        Self {
            packed: None,
            resident: None,
            live: None,
            full: None,
            revision: 0,
            geometry: 0,
            screen: None,
            start: Instant::now(),
            #[cfg(feature = "oidn")]
            oidn: None,
        }
    }
    fn install(&mut self, r: ResidentScene) {
        self.packed = None;
        self.resident = Some(r);
        self.revision += 1;
        self.geometry += 1;
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
        let camera = r.scene.camera;
        let forward = (camera.target - camera.position).normalize();
        let right = forward.cross(pm_studio::math::Vec3::Y).normalize();
        let up = right.cross(forward);
        let tan_y = (camera.fov.to_radians() * 0.5).tan();
        let (lo, hi) = r.bounds();
        let center = (lo + hi) * 0.5;
        let view = json!({"camera":{"position":camera.position.to_array(),"forward":forward.to_array(),"right":right.to_array(),"up":up.to_array(),"tanY":tan_y,"aspect":16.0/9.0,"lensRadius":if camera.f_stop>0.{camera.focal_mm/(2.*camera.f_stop)}else{0.},"focus":camera.focus.map(|f|(f-camera.position).dot(forward)).unwrap_or((camera.target-camera.position).length())},"exposure":r.scene.exposure,"backdrop":r.scene.backdrop.to_array(),"backdropOpaque":true,"shadowCatcher":r.scene.shadow_catcher,"shadows":true,"floorY":0.,"environment":{"zenith":r.scene.environment.0.to_array(),"horizon":r.scene.environment.1.to_array(),"ground":(r.scene.environment.1*0.5).to_array()},"bounds":[center.x,center.y,center.z,(hi-lo).length()*0.5]});
        Ok((
            json!({"positions":positions,"tris":tris,"nodes":nodes,"materials":materials,"sdata":sdata,"topNodes":r.top_nodes,"instanceCount":r.placements.len(),"lightCount":p.lights.len(),"objects":r.identities,"view":view}),
            binary,
        ))
    }
    fn handle(&mut self, command: Command, binary: &[u8]) -> Result<(Value, Vec<u8>), String> {
        let begin = Instant::now();
        let value = match command {
            Command::Prepared { upload } => {
                self.packed = Some(packed::PackedScene::new(upload, binary)?);
                self.resident = None;
                self.revision += 1;
                self.geometry += 1;
                json!({"revision":self.revision})
            }
            Command::PreparedPatch { patch } => {
                self.packed
                    .as_mut()
                    .ok_or("no prepared scene")?
                    .patch(patch, binary)?;
                self.revision += 1;
                json!({"revision":self.revision})
            }
            Command::Snapshot => {
                return self.snapshot();
            }
            Command::Hello => {
                json!({"protocol":VERSION,"renderer":"Powermove Studio","transport":"rgba-pipe","pid":std::process::id(),"oidn":cfg!(feature="oidn"),"capabilities":["scene","binaryMeshes","transformPatches","camera","time","pick","bounds","hardwareRayQueries"],"uptimeMs":self.start.elapsed().as_secs_f64()*1000.})
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
                r.patch_transforms(&transforms)?;
                if let Some(c) = camera {
                    r.scene.camera = c;
                }
                if let Some(m) = materials {
                    r.scene.materials = m;
                }
                if let Some(l) = lights {
                    r.scene.lights = l;
                }
                // Small material/light arrays can be rebuilt without touching meshes or BLAS.
                let metadata = Scene {
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
                self.revision += 1;
                json!({"revision":self.revision,"time":time})
            }
            Command::Textures {
                width,
                height,
                layers,
                layer,
            } => {
                dimensions(width, height)?;
                if layers == 0 || layers > 64 {
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
                self.screen = Some((width, height, 1, binary.to_vec()));
                for target in [&mut self.live, &mut self.full].into_iter().flatten() {
                    target.renderer.set_screen(binary, width, height);
                }
                self.revision += 1;
                json!({"revision":self.revision})
            }
            Command::Bounds => {
                let r = self.resident.as_ref().ok_or("no scene")?;
                let (lo, hi) = r.bounds();
                json!({"min":lo.to_array(),"max":hi.to_array(),"objects":(0..r.transforms.len()).map(|i|{let(a,b)=r.object_bounds(i);json!({"layer":r.identities[i].layer,"part":r.identities[i].part,"min":a.to_array(),"max":b.to_array()})}).collect::<Vec<_>>()})
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
            Command::Render {
                width,
                height,
                tier,
                samples,
                reset,
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
                let (w, h) = if is_live {
                    (width.div_ceil(2), height.div_ceil(2))
                } else {
                    (width, height)
                };
                if self.resident.is_none() && self.packed.is_none() {
                    return Err("no scene".into());
                }
                let slot = if is_live {
                    &mut self.live
                } else {
                    &mut self.full
                };
                if slot.as_ref().is_some_and(|t| t.renderer.size() != (w, h)) {
                    *slot = None;
                }
                let init = Instant::now();
                if slot.is_none() {
                    let mut renderer = Renderer::new(w, h)?;
                    if let Some((sw, sh, layers, p)) = &self.screen {
                        renderer.set_maps(p, *sw, *sh, *layers);
                    }
                    *slot = Some(Target {
                        renderer,
                        revision: u64::MAX,
                        geometry: u64::MAX,
                        live: is_live,
                    });
                }
                let init_ms = init.elapsed().as_secs_f64() * 1000.;
                let target = slot.as_mut().unwrap();
                let changed = target.revision != self.revision;
                if changed {
                    if let Some(r) = &self.resident {
                        target.renderer.set_instanced_scene(
                            &r.scene,
                            &r.prepared,
                            &r.geometries,
                            &r.placements,
                            r.top_nodes,
                            target.geometry != self.geometry,
                        );
                    } else if let Some(p) = &self.packed {
                        target.renderer.set_instanced_scene(
                            &p.view.scene(),
                            &p.prepared,
                            &p.geometries,
                            &p.placements,
                            p.top_nodes,
                            target.geometry != self.geometry,
                        );
                        target.renderer.set_view(
                            p.view.basis(),
                            p.view.ground(),
                            p.view.shadows,
                            p.view.bounds,
                        );
                        target.renderer.set_void(if p.view.backdrop_opaque {
                            Some(p.view.backdrop)
                        } else {
                            None
                        });
                    }
                    target.revision = self.revision;
                    target.geometry = self.geometry;
                }
                let renderer = &mut target.renderer;
                renderer.live = target.live;
                renderer.set_output_mask(layer, background);
                let spp = if is_live { 1 } else { samples };
                let trace_ms = renderer.trace(
                    spp,
                    if is_live { 3 } else { 8 },
                    reset || changed || is_live,
                    if is_live { 12 } else { 64 },
                );
                let mut denoiser = "atrous";
                #[cfg(feature = "oidn")]
                let (pixels, post_ms) = if !is_live && renderer.samples >= 16 {
                    if self.oidn.is_none() {
                        self.oidn = Some(pm_studio::oidn::Denoiser::new()?);
                    }
                    denoiser = "oidn";
                    let d = self.oidn.as_ref().unwrap();
                    renderer.finish_with(&|c, a, n| d.run(c, a, n, 4, w as usize, h as usize))
                } else {
                    renderer.finish(if is_live { 3 } else { 0 })
                };
                #[cfg(not(feature = "oidn"))]
                let (pixels, post_ms) = {
                    denoiser = if is_live { "atrous" } else { "none" };
                    renderer.finish(if is_live { 3 } else { 0 })
                };
                return Ok((
                    json!({"width":w,"height":h,"outputWidth":width,"outputHeight":height,"format":"rgba8","premultiplied":true,"tier":tier,"samples":renderer.samples,"revision":self.revision,"time":time,"hardwareRt":renderer.uses_hardware_rt(),"adapter":renderer.adapter,"denoiser":denoiser,"timings":{"initMs":init_ms,"traceMs":trace_ms,"postMs":post_ms,"hostMs":begin.elapsed().as_secs_f64()*1000.}}),
                    pixels,
                ));
            }
            Command::Shutdown => json!({"shutdown":true}),
        };
        Ok((value, vec![]))
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
    let mut input = io::stdin().lock();
    let mut output = io::BufWriter::new(io::stdout().lock());
    let mut host = Host::new();
    loop {
        let Packet { json, binary } = match protocol::read(&mut input) {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(e) => {
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
        assert!(h.handle(Command::Bounds, &[]).is_ok());
    }
    #[test]
    fn rejects_bad_render_sizes() {
        assert!(dimensions(0, 10).is_err());
        assert!(dimensions(99999, 10).is_err());
        assert!(dimensions(1920, 1080).is_ok());
    }
}
