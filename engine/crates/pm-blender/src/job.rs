//! The Blender stage of a render (section 17, contract 11): the surfaces a
//! frame needs, from the cache or from Blender, before compositing.
//!
//! [`BlenderStage::prepare`] takes the snapshots of every Blender frame a
//! render needs (per frame, shutter sample, and nested composition; built
//! by `pm_render::blender`), keys every layer's surface (section 14.7),
//! looks each up in the [`SurfaceCache`], asks a [`BlenderBackend`] for the
//! missing ones — one request per frame — decodes them, and returns them
//! by frame for the renderer. A surface is rendered once per key: within a
//! job, a later lookup finds it in memory or on disk. A requested surface
//! that does not come back fails the stage (contract 11 invariant 7: an
//! export never draws native shading in place of a Blender surface).
//!
//! **Batching.** A frame's layers render in one worker invocation
//! ([`BATCH_LAYERS`]): S7 measured batched and per-layer surfaces equal
//! pixel for pixel for EEVEE and Cycles on the CPU (and today's 8-bit
//! worker); Cycles on a GPU differs by ±1 of 65,535 run to run with or
//! without batching, so batching changes nothing there either
//! (amendment 11).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pm_protocol::blender::{BlenderEngine, BlenderSettings, RequestedDevice, ResolvedDevice};
use pm_protocol::common::ContentHash;

use crate::BlenderError;
use crate::cache::SurfaceCache;
use crate::key::{key_hash, surface_key};
use crate::snapshot::{FrameKey, Snapshot};
use crate::surface::{DecodedSurface, check_size, decode_surface};

/// A frame's layers render in one worker invocation (S7, amendment 11).
pub const BATCH_LAYERS: bool = true;

/// One Blender request: the layers of one frame.
#[derive(Debug, Clone)]
pub struct RenderJob<'a> {
    /// A safe id (`render_<n>`).
    pub id: String,
    pub snapshot: &'a Snapshot,
    /// The layers wanted, in snapshot order.
    pub layers: Vec<String>,
    pub settings: BlenderSettings,
    /// Render every layer in one invocation.
    pub batch: bool,
    /// The cache keys of `layers` (host services name files by them).
    pub keys: Vec<ContentHash>,
}

/// A finished surface's PNG.
#[derive(Debug, Clone, PartialEq)]
pub enum SurfacePng {
    File(PathBuf),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderedSurface {
    pub layer: String,
    pub png: SurfacePng,
}

/// What a backend rendered for one job.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedFrame {
    pub surfaces: Vec<RenderedSurface>,
    /// The device Blender actually used (recorded per job).
    pub device: ResolvedDevice,
}

/// Runs Blender jobs: the CLI host's local worker ([`LocalBackend`]) or the
/// job engine's host services (contract 11 over the protocol, `pm-node`).
pub trait BlenderBackend {
    /// The Blender version (`major.minor.patch`), or why Blender is missing.
    fn version(&mut self) -> Result<String, BlenderError>;
    /// Renders `job`; setting `cancel` stops Blender.
    fn render(
        &mut self,
        job: &RenderJob<'_>,
        cancel: &AtomicBool,
    ) -> Result<RenderedFrame, BlenderError>;
}

/// The settings of a snapshot (its `settings` object and frame).
pub fn snapshot_settings(snapshot: &Snapshot) -> Result<BlenderSettings, BlenderError> {
    let bad = || BlenderError::Failed("Invalid Blender render settings".into());
    let s = snapshot.settings().ok_or_else(bad)?;
    let num = |k: &str| s.get(k).and_then(pm_model::Value::as_f64);
    Ok(BlenderSettings {
        engine: match s.get("engine").and_then(pm_model::Value::as_str) {
            Some("cycles") => BlenderEngine::Cycles,
            Some("eevee") => BlenderEngine::Eevee,
            _ => return Err(bad()),
        },
        samples: num("samples")
            .filter(|n| (1.0..=4096.0).contains(n))
            .ok_or_else(bad)? as u16,
        preview: snapshot.frame.preview,
        preview_scale: num("previewScale").unwrap_or(0.5),
        denoise: s
            .get("denoise")
            .and_then(pm_model::Value::as_bool)
            .unwrap_or(true),
        device: match s.get("device").and_then(pm_model::Value::as_str) {
            Some("cpu") => RequestedDevice::Cpu,
            Some("gpu") => RequestedDevice::Gpu,
            _ => RequestedDevice::Auto,
        },
    })
}

/// Decoded surfaces by frame and layer: what the renderer draws.
pub type Surfaces = HashMap<FrameKey, HashMap<String, Arc<DecodedSurface>>>;

/// Counters of one stage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StageStats {
    /// Surfaces found in the cache.
    pub cached: u64,
    /// Surfaces Blender rendered.
    pub rendered: u64,
    /// Blender requests made.
    pub jobs: u64,
}

/// The Blender stage of one job engine (or CLI) process.
pub struct BlenderStage {
    pub cache: SurfaceCache,
    worker_hash: ContentHash,
    version: Option<String>,
    pub batch: bool,
    pub stats: StageStats,
    /// The device each job resolved, in job order (recorded per job).
    pub devices: Vec<ResolvedDevice>,
    next_id: u64,
}

impl BlenderStage {
    pub fn new(cache: SurfaceCache) -> BlenderStage {
        BlenderStage {
            cache,
            worker_hash: crate::worker::worker_script_hash(),
            version: None,
            batch: BATCH_LAYERS,
            stats: StageStats::default(),
            devices: Vec::new(),
            next_id: 0,
        }
    }

    /// The surfaces of every frame in `snapshots`. `progress` is told
    /// (surfaces done, surfaces total) as Blender finishes requests.
    pub fn prepare(
        &mut self,
        backend: &mut dyn BlenderBackend,
        snapshots: &[Snapshot],
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u32, u32),
    ) -> Result<Surfaces, BlenderError> {
        let mut out: Surfaces = HashMap::new();
        if snapshots.is_empty() {
            return Ok(out);
        }
        let version = match &self.version {
            Some(v) => v.clone(),
            None => {
                let v = backend.version()?;
                self.version = Some(v.clone());
                v
            }
        };
        let total: u32 = snapshots.iter().map(|s| s.layers().len() as u32).sum();
        let mut done = 0u32;
        for snapshot in snapshots {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(BlenderError::Cancelled);
            }
            let settings = snapshot_settings(snapshot)?;
            let frame = snapshot.frame.lookup();
            let layers = snapshot.layers();
            let entry = out.entry(frame.clone()).or_default();
            let (w, h) = snapshot.size();
            let device = self.cache.resolved_device(&version, &settings);
            let mut missing = Vec::new();
            for layer in &layers {
                let found = device.as_ref().and_then(|d| {
                    let key = key_hash(&surface_key(
                        snapshot,
                        layer,
                        self.worker_hash,
                        &version,
                        &settings,
                        d,
                    ));
                    self.cache.get(&key)
                });
                match found {
                    Some(s) => {
                        self.stats.cached += 1;
                        entry.insert(layer.clone(), s);
                        done += 1;
                    }
                    None => missing.push(layer.clone()),
                }
            }
            if missing.is_empty() {
                progress(done, total);
                continue;
            }
            self.next_id += 1;
            let keys = missing
                .iter()
                .map(|l| {
                    key_hash(&surface_key(
                        snapshot,
                        l,
                        self.worker_hash,
                        &version,
                        &settings,
                        device.as_ref().unwrap_or(&ResolvedDevice::Cpu),
                    ))
                })
                .collect();
            let job = RenderJob {
                id: format!("render_{}", self.next_id),
                snapshot,
                layers: missing.clone(),
                settings: settings.clone(),
                batch: self.batch,
                keys,
            };
            let rendered = backend.render(&job, cancel)?;
            self.stats.jobs += 1;
            self.devices.push(rendered.device.clone());
            self.cache
                .record_device(&version, &settings, &rendered.device);
            for layer in &missing {
                let Some(found) = rendered.surfaces.iter().find(|s| &s.layer == layer) else {
                    return Err(BlenderError::Failed(format!(
                        "Blender did not render the surface of layer {layer}"
                    )));
                };
                let key = key_hash(&surface_key(
                    snapshot,
                    layer,
                    self.worker_hash,
                    &version,
                    &settings,
                    &rendered.device,
                ));
                let decoded = match &found.png {
                    SurfacePng::File(p) => self.cache.insert_file(&key, p),
                    SurfacePng::Bytes(b) => self.cache.insert_bytes(&key, b),
                }
                .map_err(|e| BlenderError::Failed(e.to_string()))?;
                check_size(&decoded, w, h).map_err(|e| BlenderError::Failed(e.to_string()))?;
                self.stats.rendered += 1;
                entry.insert(layer.clone(), decoded);
                done += 1;
            }
            progress(done, total);
        }
        Ok(out)
    }
}

/// Runs jobs in a local supervised worker (the CLI host, tests).
pub struct LocalBackend {
    worker: crate::supervisor::Worker,
}

impl LocalBackend {
    pub fn new(worker: crate::supervisor::Worker) -> LocalBackend {
        LocalBackend { worker }
    }

    /// A backend on the first supported Blender, scratch under `scratch`.
    pub fn discover(scratch: &std::path::Path) -> Result<LocalBackend, BlenderError> {
        let install = crate::supervisor::discover(None)?;
        Ok(LocalBackend {
            worker: crate::supervisor::Worker::new(
                install,
                scratch,
                std::time::Duration::from_millis(u64::from(
                    pm_protocol::blender::limits::JOB_TIMEOUT_MS,
                )),
            )?,
        })
    }

    pub fn worker(&mut self) -> &mut crate::supervisor::Worker {
        &mut self.worker
    }
}

/// Writes converted asset bytes beside the worker's inputs.
fn asset_files(
    job: &RenderJob<'_>,
    scratch: &std::path::Path,
) -> Result<Vec<(String, PathBuf)>, BlenderError> {
    let mut out = Vec::new();
    for a in &job.snapshot.assets {
        let path = match &a.bytes {
            crate::snapshot::AssetBytes::File(p) => {
                // The worker loads by extension: the file keeps the name's.
                let ext = std::path::Path::new(&a.name)
                    .extension()
                    .map(|e| e.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if p.extension()
                    .map(|e| e.to_string_lossy().to_ascii_lowercase())
                    == Some(ext.to_ascii_lowercase())
                {
                    p.clone()
                } else {
                    let target = scratch.join(&a.name);
                    std::fs::copy(p, &target).map_err(|e| BlenderError::Failed(e.to_string()))?;
                    target
                }
            }
            crate::snapshot::AssetBytes::Bytes(b) => {
                let target = scratch.join(&a.name);
                crate::supervisor::write_private(&target, b)
                    .map_err(|e| BlenderError::Failed(e.to_string()))?;
                target
            }
        };
        out.push((a.id.clone(), path));
    }
    Ok(out)
}

impl BlenderBackend for LocalBackend {
    fn version(&mut self) -> Result<String, BlenderError> {
        Ok(self.worker.install().version.clone())
    }

    fn render(
        &mut self,
        job: &RenderJob<'_>,
        cancel: &AtomicBool,
    ) -> Result<RenderedFrame, BlenderError> {
        let bytes = job.snapshot.encode();
        let scratch = std::env::temp_dir().join(format!(
            "pm-blender-assets-{}-{}",
            std::process::id(),
            job.id
        ));
        crate::supervisor::create_private_dir(&scratch)
            .map_err(|e| BlenderError::Failed(e.to_string()))?;
        let result = (|| {
            let assets = asset_files(job, &scratch)?;
            let groups: Vec<Vec<String>> = if job.batch {
                vec![job.layers.clone()]
            } else {
                job.layers.iter().map(|l| vec![l.clone()]).collect()
            };
            let mut surfaces = Vec::new();
            let mut device = None;
            for (i, outputs) in groups.iter().enumerate() {
                let id = format!("{}_{i}", job.id);
                let out = self.worker.render(
                    &crate::supervisor::WorkerRequest {
                        id: &id,
                        snapshot: &bytes,
                        assets: &assets,
                        outputs,
                    },
                    cancel,
                )?;
                device = crate::worker::resolved_device(&out.manifest).or(device);
                for (layer, path) in out.files {
                    surfaces.push(RenderedSurface {
                        layer,
                        png: SurfacePng::Bytes(
                            std::fs::read(&path)
                                .map_err(|e| BlenderError::Failed(e.to_string()))?,
                        ),
                    });
                }
            }
            Ok(RenderedFrame {
                surfaces,
                device: device.ok_or_else(|| {
                    BlenderError::Failed("Blender did not report its device".into())
                })?,
            })
        })();
        let _ = std::fs::remove_dir_all(&scratch);
        result
    }
}

/// Decodes PNG bytes of a rendered surface (helpers for hosts).
pub fn decode_png(png: &SurfacePng) -> Result<DecodedSurface, BlenderError> {
    let bytes = match png {
        SurfacePng::File(p) => std::fs::read(p).map_err(|e| BlenderError::Failed(e.to_string()))?,
        SurfacePng::Bytes(b) => b.clone(),
    };
    decode_surface(&bytes).map_err(|e| BlenderError::Failed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::BlenderFrame;
    use crate::surface::encode_rgba16;

    struct Fake {
        calls: Vec<Vec<String>>,
        drop_layer: Option<String>,
    }

    impl BlenderBackend for Fake {
        fn version(&mut self) -> Result<String, BlenderError> {
            Ok("4.5.14".into())
        }
        fn render(
            &mut self,
            job: &RenderJob<'_>,
            _: &AtomicBool,
        ) -> Result<RenderedFrame, BlenderError> {
            self.calls.push(job.layers.clone());
            let (w, h) = job.snapshot.size();
            Ok(RenderedFrame {
                surfaces: job
                    .layers
                    .iter()
                    .filter(|l| Some(*l) != self.drop_layer.as_ref())
                    .map(|l| RenderedSurface {
                        layer: l.clone(),
                        png: SurfacePng::Bytes(encode_rgba16(
                            w,
                            h,
                            &vec![65535; (w * h * 4) as usize],
                            &[],
                        )),
                    })
                    .collect(),
                device: ResolvedDevice::Metal {
                    name: "test".into(),
                },
            })
        }
    }

    fn snapshot(time: f64) -> Snapshot {
        Snapshot {
            frame: BlenderFrame {
                composition_path: vec!["root".into()],
                time,
                width: 4,
                height: 2,
                shutter: None,
                preview: false,
            },
            json: pm_model::json::parse(&format!(
                r#"{{"width":4,"height":2,"time":{time},"objects":[{{"id":"a"}},{{"id":"b"}}],"camera":{{}},"settings":{{"enabled":true,"engine":"eevee","samples":16,"previewSamples":8,"previewScale":0.5,"denoise":true,"device":"auto"}}}}"#
            ))
            .unwrap(),
            assets: vec![],
        }
    }

    #[test]
    fn surfaces_render_once_per_key_and_batch_by_frame() {
        let mut stage = BlenderStage::new(SurfaceCache::new(1 << 20, None));
        let mut fake = Fake {
            calls: vec![],
            drop_layer: None,
        };
        let cancel = AtomicBool::new(false);
        let mut seen = vec![];
        let frames = [snapshot(0.0), snapshot(1.0)];
        let s = stage
            .prepare(&mut fake, &frames, &cancel, &mut |d, t| seen.push((d, t)))
            .unwrap();
        assert_eq!(fake.calls, [vec!["a", "b"], vec!["a", "b"]]);
        assert_eq!(s.len(), 2);
        assert_eq!(s[&frames[0].frame.lookup()].len(), 2);
        assert_eq!(seen.last(), Some(&(4, 4)));
        // Again: everything from the cache, the device learned from the
        // first job.
        let again = stage
            .prepare(&mut fake, &frames, &cancel, &mut |_, _| {})
            .unwrap();
        assert_eq!(fake.calls.len(), 2);
        assert_eq!(stage.stats.cached, 4);
        assert_eq!(again.len(), 2);
    }

    #[test]
    fn a_missing_surface_fails_the_stage() {
        let mut stage = BlenderStage::new(SurfaceCache::new(1 << 20, None));
        let mut fake = Fake {
            calls: vec![],
            drop_layer: Some("b".into()),
        };
        let e = stage
            .prepare(
                &mut fake,
                &[snapshot(0.0)],
                &AtomicBool::new(false),
                &mut |_, _| {},
            )
            .unwrap_err();
        assert_eq!(
            e,
            BlenderError::Failed("Blender did not render the surface of layer b".into())
        );
        let cancelled = stage
            .prepare(
                &mut fake,
                &[snapshot(2.0)],
                &AtomicBool::new(true),
                &mut |_, _| {},
            )
            .unwrap_err();
        assert_eq!(cancelled, BlenderError::Cancelled);
    }
}
