//! Production hosts behind MCP's service traits. Everything here runs on MCP or
//! export workers. Exports and frames use the same child job as the Export panel.
use anyhow::{Context, Result as AnyResult};
use pm_engine_service::jobs::{ExportHandle, ExportRequest};
use pm_kernel::{KernelHandle, Published};
use pm_mcp::{
    CaptureRequest, EngineHook, Error, Result,
    export_jobs::Progress,
    services::{ExportChild, FrozenProject, VideoExportRequest},
};
use pm_protocol::{
    color::{OutputColor, Precision},
    common::{FrameRate, SessionId, Size, TickRange, Ticks},
    priority::{AlphaOutput, ExportFormat, ExportSettings, JobSpec, JobState, QualityTier},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::{Duration, Instant},
};

/// Starts engine child jobs from the running executable. `activity` lets the
/// viewer keep precedence over export bands, as it does for panel exports.
pub struct EngineJobs {
    kernel: KernelHandle,
    executable: PathBuf,
    activity: Arc<AtomicBool>,
    cache: PathBuf,
    progress: Mutex<HashMap<u64, Progress>>,
}
impl EngineJobs {
    pub fn new(
        kernel: KernelHandle,
        executable: PathBuf,
        activity: Arc<AtomicBool>,
        cache: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            kernel,
            executable,
            activity,
            cache,
            progress: Mutex::default(),
        })
    }
    fn published(&self, frozen: &FrozenProject) -> Result<Arc<Published>> {
        self.kernel
            .published(&frozen.project_id)
            .filter(|p| p.snapshot.doc_id == frozen.document_id && p.revision() == frozen.revision)
            .ok_or_else(|| Error::conflict("The composition changed during engine work"))
    }
    fn start(
        &self,
        published: &Published,
        settings: ExportSettings,
        destination: PathBuf,
    ) -> AnyResult<ExportHandle> {
        let media = crate::media::for_session(
            &published.snapshot,
            &published.media,
            published.path.as_deref(),
            &self.cache.join(&published.project),
        )?;
        let request = ExportRequest::new(
            pm_kernel::new_id("export").as_str().into(),
            &published.snapshot,
            JobSpec::Export {
                session: SessionId::new(published.snapshot.doc_id)?,
                composition: published
                    .snapshot
                    .open_comp()
                    .unwrap_or_default()
                    .as_str()
                    .into(),
                settings,
            },
            media,
            destination,
        )?;
        ExportHandle::start_scheduled(&self.executable, request, self.activity.clone())
    }
}
fn failed(error: impl std::fmt::Display) -> Error {
    Error {
        code: "export_failed".into(),
        message: error.to_string(),
        backing: Some("pm-engine-service".into()),
    }
}
fn finish(handle: ExportHandle) -> Result<PathBuf> {
    let status = handle.wait().map_err(failed)?;
    match status.state {
        JobState::Done => status
            .path
            .map(PathBuf::from)
            .ok_or_else(|| failed("Export completed without an output path")),
        _ => Err(failed(status.error.map(|e| e.message).unwrap_or_else(
            || "Powermove lost contact with the export engine. Try again.".into(),
        ))),
    }
}
/// The published name made safe for a file, with a unique suffix.
fn file_name(name: &str, extension: &str) -> String {
    let stem = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect::<String>();
    format!("{stem}-{}.{extension}", pm_kernel::new_id("export"))
}
impl ExportChild for EngineJobs {
    fn video(&self, request: &VideoExportRequest) -> Result<PathBuf> {
        let frozen = &request.snapshot;
        let published = self.published(frozen)?;
        let project = &frozen.project;
        let options = pm_builtins::delivery::options(&json!({
            "format": request.options.format,
            "scale": 1,
            "width": project["w"],
            "height": project["h"],
            "fps": project["fps"],
            "range": request.options.range,
            "quality": "high",
            "mblur": request.options.motion_blur,
            "audio": request.options.audio,
            "alpha": request.options.alpha,
            "captionsBurn": false,
            "captionsSidecar": "none",
        }))
        .map_err(Error::invalid)?;
        let time = project["time"].as_f64().unwrap_or(0.);
        let plan = pm_builtins::delivery::plan(&published.snapshot, &options, time)
            .map_err(Error::invalid)?;
        let settings = plan
            .settings
            .clone()
            .ok_or_else(|| Error::invalid("Missing render settings"))?;
        std::fs::create_dir_all(&request.directory)?;
        let name = request
            .options
            .name
            .clone()
            .or_else(|| project["name"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "Untitled".into());
        let destination = request
            .directory
            .join(file_name(&name, options.format.extension()));
        let handle = self
            .start(&published, settings, destination)
            .map_err(failed)?;
        let started = Instant::now();
        while !matches!(
            handle.status().state,
            JobState::Done | JobState::Failed | JobState::Cancelled
        ) {
            self.progress.lock().unwrap().insert(
                frozen.document_id,
                Progress {
                    frames: (handle.status().progress * plan.frames as f64).round() as u64,
                    total: Some(plan.frames),
                    encoder: None,
                    seconds: started.elapsed().as_secs_f64(),
                },
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        self.progress.lock().unwrap().remove(&frozen.document_id);
        finish(handle)
    }
    fn video_progress(&self, frozen: &FrozenProject) -> Option<Progress> {
        self.progress
            .lock()
            .unwrap()
            .get(&frozen.document_id)
            .cloned()
    }
}
/// One composition frame as PNG bytes, through the production export renderer.
fn frame(
    jobs: &EngineJobs,
    published: &Published,
    fps: f64,
    time: f64,
    size: Size,
) -> AnyResult<Vec<u8>> {
    let rate = pm_time::rate_from_document(None, fps)?.rate;
    let start = pm_time::ticks_from_seconds(time)?;
    let span = rate.den() as f64 / rate.num() as f64;
    let end = pm_time::ticks_from_seconds(time + span)?;
    let output = tempfile::tempdir_in(jobs.cache.as_path())?;
    let destination = output.path().join("frame");
    let settings = ExportSettings {
        format: ExportFormat::PngSequence { sixteen_bit: false },
        alpha: AlphaOutput::Straight,
        quality: QualityTier::High,
        size,
        frame_rate: FrameRate::new(u32::try_from(rate.num())?, u32::try_from(rate.den())?),
        range: TickRange {
            start: Ticks::new(start)?,
            end: Ticks::new(end)?,
        },
        audio: false,
        audio_index_playhead: None,
        motion_blur: false,
        color: OutputColor::SrgbImage,
        precision: Precision::Float16,
        dither: false,
        draft_from_proxies: false,
        delivery: Default::default(),
    };
    let directory = finish(jobs.start(published, settings, destination)?)?;
    first_png(&directory)
}
fn first_png(directory: &Path) -> AnyResult<Vec<u8>> {
    let mut frames = std::fs::read_dir(directory)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|e| e == "png"))
        .collect::<Vec<_>>();
    frames.sort();
    Ok(std::fs::read(
        frames.first().context("Render produced no frame")?,
    )?)
}
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                text.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                text.push('=');
            }
        }
    }
    text
}
impl EngineHook for EngineJobs {
    fn capture(&self, _: &CaptureRequest) -> Result<Value> {
        Err(Error::invalid("Frame capture needs a frozen project"))
    }
    fn capture_snapshot(&self, request: &CaptureRequest, frozen: &FrozenProject) -> Result<Value> {
        let published = self.published(frozen)?;
        let project = &frozen.project;
        let (w, h) = (
            project["w"].as_f64().unwrap_or(1920.),
            project["h"].as_f64().unwrap_or(1080.),
        );
        let fps = project["fps"].as_f64().unwrap_or(30.);
        let width = request.width.clamp(16, 8192);
        let height = ((width as f64 * h / w).round() as u32).max(16);
        std::fs::create_dir_all(&self.cache)?;
        let mut images = vec![];
        for time in &request.times {
            let size = Size::new(width, height);
            let bytes = frame(self, &published, fps, *time, size).map_err(failed)?;
            images.push(format!("data:image/png;base64,{}", base64(&bytes)));
        }
        Ok(json!({"times":request.times,"images":images}))
    }
}
