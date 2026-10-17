//! The job engine's Blender backend (contract 11 over the host port;
//! contract 14's Blender supervision runs in host services).
//!
//! For each Blender frame the stage needs, the backend writes the
//! snapshot's canonical JSON into the Blender Surfaces cache area
//! (`snapshots/<blake3>.json`) and any converted texture into
//! `assets/<sha256>.<ext>`, names every other asset by a media handle, and
//! sends `blenderRender` with one surface request per layer. Host services
//! run the pinned worker, write each surface as `<key>.png` in the same
//! area, send one `blenderSurface` event per surface (and progress), and
//! answer the request once the job ends: `ack`, or the error that ended it
//! (`blenderFailed`, `blenderMissing`, `cancelled`). Cancelling the export
//! sends `cancelJobs` for the Blender job, which stops Blender.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use pm_blender::BlenderError;
use pm_blender::job::{BlenderBackend, RenderJob, RenderedFrame, RenderedSurface, SurfacePng};
use pm_blender::key::hex;
use pm_blender::snapshot::AssetBytes;
use pm_protocol::blender::{
    BlenderAsset, BlenderJobRequest, BlenderSurface, ShutterSample, SnapshotEncoding, SnapshotRef,
    SurfaceRequest, limits,
};
use pm_protocol::common::{ByteSize, ContentHash, JobId, SessionId, Size, Ticks};
use pm_protocol::envelope::{Request, Response};
use pm_protocol::error::{EngineError, ErrorCode};
use pm_protocol::host::{
    CacheKind, CapabilityHandle, CapabilityKind, HandleAccess, MintHandle, ResolveHandle,
};
use pm_protocol::priority::{CancelJobs, PriorityClass};

use super::Shared;

/// How long one Blender frame may take (today's 10-minute job timeout,
/// plus the host's own bookkeeping).
const FRAME_TIMEOUT: Duration = Duration::from_millis(limits::JOB_TIMEOUT_MS as u64 + 30_000);

/// The Blender backend of one job lane.
pub(super) struct HostBackend {
    pub shared: Arc<Shared>,
    /// The export this backend works for now (set by the lane per job).
    pub job: Arc<std::sync::Mutex<JobParams>>,
    area: Option<(PathBuf, CapabilityHandle, SessionId)>,
    next: u64,
}

/// The export a lane's Blender backend works for.
#[derive(Debug, Clone)]
pub(super) struct JobParams {
    /// The export's session (handles are bound to it) and class.
    pub session: SessionId,
    pub priority: PriorityClass,
    /// The export job, to name Blender jobs after it.
    pub export: JobId,
}

impl HostBackend {
    pub fn new(shared: Arc<Shared>, job: Arc<std::sync::Mutex<JobParams>>) -> HostBackend {
        HostBackend {
            shared,
            job,
            area: None,
            next: 0,
        }
    }

    fn params(&self) -> JobParams {
        self.job.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The Blender Surfaces cache area and a handle to it for this session.
    pub fn area(&mut self) -> Result<(PathBuf, CapabilityHandle), BlenderError> {
        let session = self.params().session;
        if let Some((path, handle, s)) = &self.area
            && *s == session
        {
            return Ok((path.clone(), handle.clone()));
        }
        let failed = |e: EngineError| BlenderError::Failed(format!("Blender cache: {}", e.message));
        let grant = match self.shared.call(
            Request::MintHandle(MintHandle {
                kind: CapabilityKind::Cache {
                    cache: CacheKind::BlenderSurfaces,
                },
                session,
            }),
            Some(session),
        ) {
            Ok(Response::Handle(g)) => g,
            Ok(_) => {
                return Err(BlenderError::Failed(
                    "Blender cache: unexpected answer".into(),
                ));
            }
            Err(e) => return Err(failed(e)),
        };
        let resolved = match self.shared.call(
            Request::ResolveHandle(ResolveHandle {
                handle: grant.handle.clone(),
                session,
            }),
            Some(session),
        ) {
            Ok(Response::HandleResolved(r)) if r.access == HandleAccess::ReadWrite => r,
            Ok(_) => return Err(BlenderError::Failed("Blender cache: not writable".into())),
            Err(e) => return Err(failed(e)),
        };
        let path = PathBuf::from(resolved.path);
        for sub in ["snapshots", "assets", "surfaces"] {
            pm_blender::supervisor::create_private_dir(&path.join(sub))
                .map_err(|e| BlenderError::Failed(format!("Blender cache: {e}")))?;
        }
        prune(&path);
        self.area = Some((path.clone(), grant.handle.clone(), session));
        Ok((path, grant.handle))
    }

    fn media_handle(&self, asset: &str) -> Result<CapabilityHandle, BlenderError> {
        let session = self.params().session;
        match self.shared.call(
            Request::MintHandle(MintHandle {
                kind: CapabilityKind::MediaRead {
                    asset: asset.into(),
                },
                session,
            }),
            Some(session),
        ) {
            Ok(Response::Handle(g)) => Ok(g.handle),
            Ok(_) => Err(BlenderError::Failed(format!("Missing media for {asset}"))),
            Err(e) => Err(BlenderError::Failed(e.message)),
        }
    }
}

/// Snapshots and converted textures are inputs of running jobs only:
/// those older than the longest a job may run are removed (surfaces stay,
/// under the surface cache's own bound).
fn prune(area: &std::path::Path) {
    let horizon = FRAME_TIMEOUT * 2;
    for sub in ["snapshots", "assets", ""] {
        let Ok(entries) = std::fs::read_dir(area.join(sub)) else {
            continue;
        };
        // In the area itself, only surfaces host services wrote and no job
        // took (a failed or cancelled one).
        let only_png = sub.is_empty();
        for e in entries.flatten() {
            if only_png && e.path().extension().is_none_or(|x| x != "png") {
                continue;
            }
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > horizon);
            if old {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

fn of_engine(e: EngineError) -> BlenderError {
    match e.code {
        ErrorCode::BlenderMissing => BlenderError::Missing(e.message),
        ErrorCode::Cancelled => BlenderError::Cancelled,
        _ => BlenderError::Failed(e.message),
    }
}

impl BlenderBackend for HostBackend {
    fn version(&mut self) -> Result<String, BlenderError> {
        let session = self.params().session;
        match self.shared.call(Request::BlenderStatus, Some(session)) {
            Ok(Response::BlenderStatus(s)) if s.supported() => Ok(s.version.unwrap_or_default()),
            Ok(Response::BlenderStatus(s)) => Err(BlenderError::Missing(
                s.error
                    .filter(|e| !e.is_empty())
                    .unwrap_or_else(|| pm_blender::BLENDER_MISSING_MESSAGE.to_owned()),
            )),
            Ok(_) => Err(BlenderError::Failed(
                "unexpected Blender status answer".into(),
            )),
            Err(e) => Err(of_engine(e)),
        }
    }

    fn render(
        &mut self,
        job: &RenderJob<'_>,
        cancel: &AtomicBool,
    ) -> Result<RenderedFrame, BlenderError> {
        let (area, handle) = self.area()?;
        let params = self.params();
        let io = |e: std::io::Error| BlenderError::Failed(format!("Blender cache: {e}"));
        // The snapshot, addressed by its hash.
        let bytes = job.snapshot.encode();
        let hash = job.snapshot.hash();
        let file = area.join("snapshots").join(format!("{}.json", hex(&hash)));
        // Written again even when present: a fresh modification time keeps
        // it out of `prune` while the job runs.
        let tmp = file.with_extension(format!("{}.{}.part", std::process::id(), self.next));
        pm_blender::supervisor::write_private(&tmp, &bytes).map_err(io)?;
        std::fs::rename(&tmp, &file).map_err(io)?;
        // Assets: media handles, or converted bytes in the area.
        let mut assets = Vec::new();
        for a in &job.snapshot.assets {
            let handle = match &a.bytes {
                AssetBytes::File(_) => self.media_handle(&a.id)?,
                AssetBytes::Bytes(b) => {
                    let ext = std::path::Path::new(&a.name)
                        .extension()
                        .map(|e| e.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "png".into());
                    let path = area
                        .join("assets")
                        .join(format!("{}.{ext}", hex(&a.hash())));
                    pm_blender::supervisor::write_private(&path, b).map_err(io)?;
                    handle.clone()
                }
            };
            assets.push(BlenderAsset {
                asset: a.id.as_str().into(),
                hash: a.hash(),
                handle,
            });
        }
        self.next += 1;
        let id = JobId::new(format!("{}-blender-{}", params.export.0, self.next));
        let (w, h) = job.snapshot.size();
        let frame = &job.snapshot.frame;
        let surfaces: Vec<SurfaceRequest> = job
            .layers
            .iter()
            .zip(
                job.keys
                    .iter()
                    .chain(std::iter::repeat(&ContentHash([0; 32]))),
            )
            .map(|(layer, key)| SurfaceRequest {
                layer: layer.as_str().into(),
                composition_path: frame
                    .composition_path
                    .iter()
                    .map(|c| c.as_str().into())
                    .collect(),
                time: Ticks::new(
                    (frame.time * pm_protocol::common::TICKS_PER_SECOND as f64).round() as i64,
                )
                .unwrap_or(Ticks::ZERO),
                shutter: frame
                    .shutter
                    .map(|(index, count)| ShutterSample { index, count }),
                size: Size::new(w, h),
                key: *key,
            })
            .collect();
        let request = BlenderJobRequest {
            job: id.clone(),
            priority: params.priority,
            snapshot: SnapshotRef {
                hash,
                encoding: SnapshotEncoding::RenderSnapshotJsonV1,
                handle: handle.clone(),
                bytes: ByteSize::new(bytes.len() as u64)
                    .map_err(|_| BlenderError::Failed("snapshot size".into()))?,
            },
            worker_script_hash: pm_blender::worker::worker_script_hash(),
            assets,
            settings: job.settings.clone(),
            surfaces,
            batch_layers: job.batch,
            output: handle,
        };
        pm_protocol::validate::Validate::validate(&request)
            .map_err(|v| BlenderError::Failed(v.to_string()))?;
        let answer = self.shared.call_until(
            Request::BlenderRender(request),
            Some(params.session),
            Instant::now() + FRAME_TIMEOUT,
            cancel,
        );
        let delivered = self.shared.take_blender_surfaces(&id);
        match answer {
            Some(Ok(Response::Ack)) => {}
            Some(Ok(_)) => return Err(BlenderError::Failed("unexpected Blender answer".into())),
            Some(Err(e)) => return Err(of_engine(e)),
            None => {
                // Cancelled: stop Blender.
                let _ = self.shared.call(
                    Request::CancelJobs(CancelJobs::Job { job: id }),
                    Some(params.session),
                );
                return Err(BlenderError::Cancelled);
            }
        }
        let device = delivered
            .first()
            .map(|s| s.device.clone())
            .ok_or_else(|| BlenderError::Failed("Blender delivered no surfaces".into()))?;
        let mut out = Vec::new();
        for s in delivered {
            pm_protocol::validate::Validate::validate(&s)
                .map_err(|v| BlenderError::Failed(v.to_string()))?;
            let Some(layer) = job.layers.get(s.request_index as usize) else {
                return Err(BlenderError::Failed(
                    "a Blender surface for no request".into(),
                ));
            };
            out.push(RenderedSurface {
                layer: layer.clone(),
                png: SurfacePng::File(area.join(format!("{}.png", hex(&s.key)))),
            });
        }
        Ok(RenderedFrame {
            surfaces: out,
            device,
        })
    }
}

/// Surfaces received for each Blender job, until its answer arrives.
#[derive(Default)]
pub(super) struct Delivered {
    pub by_job: std::collections::HashMap<JobId, Vec<BlenderSurface>>,
}

impl Delivered {
    pub fn push(&mut self, s: BlenderSurface) {
        // Bounded: a job has at most the nested-frame limit of surfaces.
        let list = self.by_job.entry(s.job.clone()).or_default();
        if list.len() < limits::MAX_NESTED_FRAMES {
            list.push(s);
        }
    }
}
