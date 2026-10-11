//! Priority classes and agent frame requests (contract 12).
//!
//! Interactive frames come first, then agent snapshots, then jobs (section 4,
//! principle 7). Agent exports run in their own lane ahead of user batch jobs
//! (section 17). Agent frame requests are batched, carry the agent's revision
//! for read-your-writes, queue instead of failing, and never pause playback.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::color::{OutputColor, Precision};
use crate::common::{AgentRunId, AssetId, ByteSize, Bytes, CompositionId, FrameRate, JobId, Revision, SessionId, Size, TickRange, Ticks};
use crate::host::OutputChoice;
use crate::error::EngineError;
use crate::host::CapabilityHandle;
use crate::validate::{ensure, Validate, Violation};
use crate::version::ContractId;

/// Scheduling class, highest first. The derived order puts the most urgent
/// class last, so `a > b` means `a` preempts `b`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum PriorityClass {
    /// Background renders (4b), lowest.
    Background,
    /// Proxy and generated-media jobs; paused during playback and export.
    Generation,
    /// User exports and render-queue jobs.
    UserJob,
    /// Agent `export_video` jobs: their own lane ahead of user batch jobs.
    AgentExport,
    /// Agent snapshots (`render_frames`, contact sheets, `observeVisual`).
    AgentSnapshot,
    /// Viewer frames, scrubbing, playback. Never preempted.
    Interactive,
}

impl PriorityClass {
    /// Which engine process serves the class. Agent snapshots run in the job
    /// engine so they never pause playback.
    pub const fn process(self) -> crate::error::EngineProcess {
        match self {
            PriorityClass::Interactive => crate::error::EngineProcess::Interactive,
            _ => crate::error::EngineProcess::Job,
        }
    }

    /// Whether work of this class pauses while the interactive engine plays.
    pub const fn pauses_during_playback(self) -> bool {
        matches!(self, PriorityClass::Generation | PriorityClass::Background)
    }

    /// Whether work of this class is sliced into short command buffers during
    /// playback (contract 13). Everything outside the interactive engine is.
    pub const fn sliced_during_playback(self) -> bool {
        !matches!(self, PriorityClass::Interactive)
    }

    /// Whether a job of this class persists and resumes after a relaunch.
    /// User queue and generation jobs do; agent jobs never resume (section 17).
    pub const fn persists_across_relaunch(self) -> bool {
        matches!(self, PriorityClass::UserJob | PriorityClass::Generation | PriorityClass::Background)
    }

    /// Classes that are jobs (submitted with [`SubmitJob`]) rather than frames.
    pub const fn is_job(self) -> bool {
        !matches!(self, PriorityClass::Interactive | PriorityClass::AgentSnapshot)
    }
}

/// `render_frames` default longest edge (section 20).
pub const RENDER_FRAMES_DEFAULT_PX: u32 = 720;
/// `observeVisual` default longest edge.
pub const OBSERVE_VISUAL_DEFAULT_PX: u32 = 480;
/// Agent JPEG quality default.
pub const AGENT_JPEG_QUALITY: f32 = 0.74;

/// How agent frames treat Blender-rendered compositions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum BlenderPolicy {
    /// Use cached surfaces, else native shading reported as `pending` (default).
    CachedOrPending,
    /// Render with Blender and wait.
    Render,
}

/// Image encoding for agent frames.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum AgentImageFormat {
    Jpeg { quality: f32 },
    Png,
}

/// One frame an agent wants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AgentFrame {
    pub composition: CompositionId,
    pub time: Ticks,
}

/// A batch of agent frames. The envelope's revision is the agent's last write;
/// the engine answers at that revision or later (read-your-writes).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AgentFrameRequest {
    pub run: AgentRunId,
    pub frames: Vec<AgentFrame>,
    /// Longest edge in pixels; the frame keeps its aspect.
    pub max_dimension: u32,
    pub format: AgentImageFormat,
    /// Off by default for agents.
    pub motion_blur: bool,
    pub blender: BlenderPolicy,
}

/// Most frames in one agent batch.
pub const AGENT_FRAMES_MAX_BATCH: usize = 64;

impl AgentFrameRequest {
    pub fn render_frames_default(run: AgentRunId, frames: Vec<AgentFrame>) -> Self {
        Self {
            run,
            frames,
            max_dimension: RENDER_FRAMES_DEFAULT_PX,
            format: AgentImageFormat::Jpeg { quality: AGENT_JPEG_QUALITY },
            motion_blur: false,
            blender: BlenderPolicy::CachedOrPending,
        }
    }

}

impl Validate for AgentFrameRequest {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Priority;
        ensure(self.run.is_valid(), C, || "agent run id is empty or too long".into())?;
        ensure(!self.frames.is_empty() && self.frames.len() <= AGENT_FRAMES_MAX_BATCH, C, || {
            format!("an agent batch has 1 to {AGENT_FRAMES_MAX_BATCH} frames, got {}", self.frames.len())
        })?;
        ensure(self.frames.iter().all(|f| f.composition.is_valid()), C, || "invalid composition id".into())?;
        ensure(self.max_dimension > 0 && self.max_dimension <= 8192, C, || {
            format!("max dimension must be 1 to 8192 px, got {}", self.max_dimension)
        })?;
        if let AgentImageFormat::Jpeg { quality } = self.format {
            ensure(quality > 0.0 && quality <= 1.0, C, || format!("JPEG quality must be in (0, 1], got {quality}"))?;
        }
        Ok(())
    }
}

impl Validate for JobStatus {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Priority;
        ensure(self.job.is_valid(), C, || "job id is empty or too long".into())?;
        ensure((0.0..=1.0).contains(&self.progress), C, || format!("progress {} outside 0..1", self.progress))?;
        ensure(self.path.is_none() || self.state == JobState::Done, C, || "only a finished job has a path".into())?;
        ensure(self.error.is_none() || self.state == JobState::Failed, C, || "only a failed job has an error".into())
    }
}

/// How the Blender surface for a frame was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum BlenderUse {
    /// The composition does not render with Blender.
    NotUsed,
    /// Cached Blender surfaces were used.
    Cached,
    /// Native shading stood in; reported to the agent as `blender: "pending"`.
    Pending,
    /// Blender rendered for this request.
    Rendered,
}

/// Where the encoded image is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum ImagePayload {
    /// Small images inline (under the message limit).
    Inline { bytes: Bytes },
    /// Larger images in a cache file behind a capability handle.
    CacheFile { handle: CapabilityHandle, length: ByteSize },
}

/// The result for one requested frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "status")]
pub enum AgentFrameOutcome {
    Rendered {
        index: u32,
        revision: Revision,
        width: u32,
        height: u32,
        image: ImagePayload,
        blender: BlenderUse,
    },
    Failed {
        index: u32,
        error: EngineError,
    },
}

/// Response to [`AgentFrameRequest`], one outcome per frame in request order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AgentFrameResult {
    pub outcomes: Vec<AgentFrameOutcome>,
    /// Time spent queued behind higher classes, for the latency bound.
    pub queued_ms: u32,
}

/// Job states as `get_render_status` reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum JobState {
    Queued,
    Running,
    /// Waiting for offline media (queued jobs wait, section 14.8).
    WaitingForMedia,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    pub const fn is_terminal(self) -> bool {
        matches!(self, JobState::Done | JobState::Failed | JobState::Cancelled)
    }
}

/// Job status event; maps one-to-one onto `get_render_status`
/// `{jobId, state, progress, path?, error?}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    pub job: JobId,
    pub class: PriorityClass,
    pub state: JobState,
    /// 0 to 1.
    pub progress: f64,
    /// The real output path, only after encoding, verification, and the move
    /// into place (host services fill it in; the engine never sees paths).
    pub path: Option<String>,
    pub error: Option<EngineError>,
}

// ---------------------------------------------------------------------------
// Jobs: exports, agent exports, generated media
// ---------------------------------------------------------------------------

/// ProRes profiles (VideoToolbox on macOS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ProResProfile {
    Proxy,
    Lt,
    Standard,
    Hq,
    P4444,
    P4444Xq,
}

impl ProResProfile {
    pub const fn supports_alpha(self) -> bool {
        matches!(self, ProResProfile::P4444 | ProResProfile::P4444Xq)
    }
}

/// MP4 or QuickTime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum Container {
    Mp4,
    Mov,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum WebmCodec {
    Vp9,
    Vp8,
}

/// The engine-rendered formats at cutover (section 17). Code export and
/// project export are not render jobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum ExportFormat {
    ProRes { profile: ProResProfile },
    H264,
    /// 8 or 10-bit SDR; alpha only at 8-bit (confirmed in S2).
    Hevc { ten_bit: bool, container: Container },
    /// Frame-exact; today's "WebM · realtime capture" migrates here.
    Webm { codec: WebmCodec },
    PngSequence { sixteen_bit: bool },
    PngStill { sixteen_bit: bool },
}

impl ExportFormat {
    pub const fn is_image(self) -> bool {
        matches!(self, ExportFormat::PngSequence { .. } | ExportFormat::PngStill { .. })
    }

    pub const fn supports_alpha(self) -> bool {
        match self {
            ExportFormat::ProRes { profile } => profile.supports_alpha(),
            ExportFormat::H264 => false,
            ExportFormat::Hevc { ten_bit, .. } => !ten_bit,
            ExportFormat::Webm { .. } | ExportFormat::PngSequence { .. } | ExportFormat::PngStill { .. } => true,
        }
    }

    /// Whether the quality tier (bitrate) applies.
    pub const fn uses_bitrate(self) -> bool {
        matches!(self, ExportFormat::H264 | ExportFormat::Hevc { .. } | ExportFormat::Webm { .. })
    }
}

/// How alpha is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum AlphaOutput {
    None,
    Straight,
    Premultiplied,
}

/// Today's quality tiers (`export-defaults.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum QualityTier {
    Draft,
    High,
    Max,
}

impl QualityTier {
    pub const fn bitrate_mbps(self) -> u32 {
        match self {
            QualityTier::Draft => 4,
            QualityTier::High => 16,
            QualityTier::Max => 40,
        }
    }
}

/// Export settings as the job engine receives them. Today's settings are
/// preserved (section 17); pm-export owns their meaning, this contract owns
/// their shape and limits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExportSettings {
    pub format: ExportFormat,
    pub alpha: AlphaOutput,
    pub quality: QualityTier,
    /// Output size after scale (0.01 to 16) or a custom size.
    pub size: Size,
    /// May differ from the composition rate (section 5 rule 6).
    pub frame_rate: FrameRate,
    pub range: TickRange,
    pub audio: bool,
    pub motion_blur: bool,
    pub color: OutputColor,
    /// Defaults to 32-bit when the output or any source exceeds 10 bits.
    pub precision: Precision,
    /// Blue-noise dither, on by default for 8-bit outputs (section 17).
    pub dither: bool,
    /// "Render using: Proxies (draft)", chosen explicitly (section 14.6).
    pub draft_from_proxies: bool,
}

/// Longest export edge.
pub const MAX_EXPORT_DIMENSION: u32 = 16384;

impl Validate for ExportSettings {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Priority;
        ensure(!self.size.is_empty() && self.size.width <= MAX_EXPORT_DIMENSION && self.size.height <= MAX_EXPORT_DIMENSION, C, || {
            format!("export size {:?} outside 1..{MAX_EXPORT_DIMENSION}", self.size)
        })?;
        ensure(self.frame_rate.is_valid(), C, || "export frame rate must be positive".into())?;
        ensure(self.range.is_valid(), C, || "export range must be non-empty".into())?;
        ensure(self.alpha == AlphaOutput::None || self.format.supports_alpha(), C, || {
            format!("{:?} cannot carry alpha", self.format)
        })?;
        ensure(!(self.format.is_image() && self.audio), C, || "image outputs carry no audio".into())?;
        let image_tag = matches!(self.color, OutputColor::SrgbImage | OutputColor::Rec709Image);
        ensure(!matches!(self.color, OutputColor::Viewer { .. }), C, || "exports use an output tag, not the viewer's".into())?;
        ensure(image_tag == self.format.is_image(), C, || "images use image tags and video uses video tags".into())
    }
}

/// Proxy formats (appendix D, Settings › Project › Media).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ProxyFormat {
    ProResProxy,
    ProResLt,
    /// "HEVC (smaller files)", 10-bit.
    Hevc10,
}

/// Proxy sizes (appendix D). Automatic never goes below 540p.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ProxySize {
    Automatic,
    Half,
    Quarter,
    P1080,
    P720,
}

/// What a job does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum JobSpec {
    /// Render a composition from a frozen job snapshot session (section 17:
    /// users keep editing; edits never invalidate a running export).
    Export { session: SessionId, composition: CompositionId, settings: ExportSettings },
    /// Generate a whole-clip proxy (Phase 4a). Sources with alpha always get
    /// ProRes 4444 at proxy size, whatever `format` says.
    GenerateProxy { asset: AssetId, format: ProxyFormat, size: ProxySize },
}

/// Submit a job to the job engine (host → job engine).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SubmitJob {
    pub job: JobId,
    pub class: PriorityClass,
    /// The agent run that owns the job. Cancelling or rolling back the run
    /// cancels its jobs.
    pub run: Option<AgentRunId>,
    pub spec: JobSpec,
    /// Where the output goes; the job engine asks host services for a
    /// destination handle with it (contract 14). Agents never choose paths.
    pub output: Option<OutputChoice>,
}

impl Validate for SubmitJob {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Priority;
        ensure(self.job.is_valid(), C, || "invalid job id".into())?;
        ensure(self.class.is_job(), C, || format!("{:?} is not a job class", self.class))?;
        ensure((self.class == PriorityClass::AgentExport) == self.run.is_some(), C, || {
            "agent exports, and only agent exports, belong to an agent run".into()
        })?;
        match &self.spec {
            JobSpec::Export { composition, settings, .. } => {
                ensure(composition.is_valid(), C, || "invalid composition id".into())?;
                settings.validate()?;
                ensure(matches!(self.class, PriorityClass::UserJob | PriorityClass::AgentExport | PriorityClass::Background), C, || {
                    "exports run as user, agent, or background jobs".into()
                })?;
                let output = self.output.as_ref().ok_or_else(|| Violation::new(C, "an export needs an output choice"))?;
                ensure(matches!(output, OutputChoice::AgentAutomatic { .. }) == (self.class == PriorityClass::AgentExport), C, || {
                    "agent exports use the automatic destination; user exports use a chosen one".into()
                })?;
                ensure(!(settings.draft_from_proxies && self.class == PriorityClass::AgentExport), C, || {
                    "agent exports never render from proxies".into()
                })
            }
            JobSpec::GenerateProxy { .. } => {
                ensure(self.class == PriorityClass::Generation, C, || "proxy generation runs in the generation class".into())?;
                ensure(self.output.is_none(), C, || "generated media goes to the generated-media store, not an output".into())
            }
        }
    }
}

/// The job engine accepted a job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct JobAccepted {
    pub job: JobId,
    pub state: JobState,
    /// Jobs ahead of it in its class and above.
    pub position: u32,
}

/// What to cancel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum CancelJobs {
    Job { job: JobId },
    /// Every job of an agent run (the run was cancelled or rolled back).
    Run { run: AgentRunId },
}

/// The jobs a cancel stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct JobsCancelled {
    pub jobs: Vec<JobId>,
}

/// `export_video` `mode` (section 17): `wait` (default) or `async`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum AgentExportMode {
    #[default]
    Wait,
    Async,
}

/// What `export_video` returns to the agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "status")]
pub enum AgentExportReply {
    /// The real path after encoding, verification, and the move into place.
    Done { job: JobId, path: String },
    /// `{status: "queued", jobId}`: read progress with `get_render_status`.
    Queued { job: JobId },
    Failed { job: JobId, error: EngineError },
    Cancelled { job: JobId },
}

/// Decides whether `export_video` answers now. `None` means keep waiting.
/// In `wait` mode a job that has waited in the queue longer than
/// `queue_bound_ms` answers `queued`; tool timeouts count from job start.
pub fn agent_export_reply(mode: AgentExportMode, status: &JobStatus, queued_ms: u32, queue_bound_ms: u32) -> Option<AgentExportReply> {
    let job = status.job.clone();
    match status.state {
        JobState::Done => Some(match &status.path {
            Some(path) => AgentExportReply::Done { job, path: path.clone() },
            None => AgentExportReply::Failed {
                job: job.clone(),
                error: EngineError::new(crate::error::ErrorCode::Internal, crate::error::ErrorScope::Job { job }, "The export finished without a path."),
            },
        }),
        JobState::Failed => Some(AgentExportReply::Failed {
            job: job.clone(),
            error: status.error.clone().unwrap_or_else(|| {
                EngineError::new(crate::error::ErrorCode::ExportFailed, crate::error::ErrorScope::Job { job }, "The export failed.")
            }),
        }),
        JobState::Cancelled => Some(AgentExportReply::Cancelled { job }),
        _ if mode == AgentExportMode::Async => Some(AgentExportReply::Queued { job }),
        JobState::Queued | JobState::WaitingForMedia if queued_ms > queue_bound_ms => Some(AgentExportReply::Queued { job }),
        JobState::Queued | JobState::WaitingForMedia | JobState::Running => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_order_matches_section_4_and_17() {
        use PriorityClass::*;
        let mut classes = [UserJob, Interactive, Background, AgentSnapshot, Generation, AgentExport];
        classes.sort();
        assert_eq!(classes, [Background, Generation, UserJob, AgentExport, AgentSnapshot, Interactive]);
        assert!(Interactive > AgentSnapshot && AgentSnapshot > AgentExport && AgentExport > UserJob);
        assert_eq!(AgentSnapshot.process(), crate::error::EngineProcess::Job);
        assert!(Generation.pauses_during_playback() && !AgentSnapshot.pauses_during_playback());
    }

    #[test]
    fn render_frames_defaults() {
        let request = AgentFrameRequest::render_frames_default(
            "run".into(),
            vec![AgentFrame { composition: "c".into(), time: Ticks::ZERO }],
        );
        assert_eq!(request.max_dimension, 720);
        assert_eq!(request.format, AgentImageFormat::Jpeg { quality: 0.74 });
        assert!(!request.motion_blur);
        assert!(request.validate().is_ok());
        let empty = AgentFrameRequest { frames: vec![], ..request };
        assert!(empty.validate().is_err());
    }

    pub(crate) fn export_settings() -> ExportSettings {
        ExportSettings {
            format: ExportFormat::ProRes { profile: ProResProfile::P4444 },
            alpha: AlphaOutput::Straight,
            quality: QualityTier::High,
            size: Size::new(3840, 2160),
            frame_rate: FrameRate::new(24000, 1001),
            range: TickRange { start: Ticks::ZERO, end: Ticks::from_whole_seconds(10) },
            audio: true,
            motion_blur: true,
            color: OutputColor::Rec709Broadcast,
            precision: Precision::Float32,
            dither: false,
            draft_from_proxies: false,
        }
    }

    pub(crate) fn agent_export() -> SubmitJob {
        SubmitJob {
            job: "job-1".into(),
            class: PriorityClass::AgentExport,
            run: Some("run-1".into()),
            spec: JobSpec::Export { session: crate::common::SessionId::new(4).unwrap(), composition: "main".into(), settings: export_settings() },
            output: Some(OutputChoice::AgentAutomatic { suggested_name: "Launch cut".into(), extension: "mov".into() }),
        }
    }

    #[test]
    fn export_settings_follow_section_17() {
        assert!(export_settings().validate().is_ok());
        let hevc10_alpha = ExportSettings { format: ExportFormat::Hevc { ten_bit: true, container: Container::Mov }, ..export_settings() };
        assert!(hevc10_alpha.validate().is_err(), "HEVC alpha is 8-bit only");
        let hq_alpha = ExportSettings { format: ExportFormat::ProRes { profile: ProResProfile::Hq }, ..export_settings() };
        assert!(hq_alpha.validate().is_err(), "422 HQ has no alpha");
        let png_audio = ExportSettings { format: ExportFormat::PngSequence { sixteen_bit: true }, color: OutputColor::SrgbImage, ..export_settings() };
        assert!(png_audio.validate().is_err());
        assert!(ExportSettings { audio: false, ..png_audio.clone() }.validate().is_ok());
        assert!(ExportSettings { audio: false, color: OutputColor::Rec709Broadcast, ..png_audio }.validate().is_err(), "images use image tags");
        assert_eq!(QualityTier::Max.bitrate_mbps(), 40);
    }

    #[test]
    fn job_submission_rules() {
        assert!(agent_export().validate().is_ok());
        let user_choosing_agent_output = SubmitJob { class: PriorityClass::UserJob, run: None, ..agent_export() };
        assert!(user_choosing_agent_output.validate().is_err());
        let orphan = SubmitJob { run: None, ..agent_export() };
        assert!(orphan.validate().is_err(), "agent exports belong to a run");
        let mut drafty = agent_export();
        if let JobSpec::Export { settings, .. } = &mut drafty.spec {
            settings.draft_from_proxies = true;
        }
        assert!(drafty.validate().is_err(), "agents never export proxies");
        let snapshot_class = SubmitJob { class: PriorityClass::AgentSnapshot, ..agent_export() };
        assert!(snapshot_class.validate().is_err());
        let proxy = SubmitJob {
            job: "gen-1".into(),
            class: PriorityClass::Generation,
            run: None,
            spec: JobSpec::GenerateProxy { asset: "a1".into(), format: ProxyFormat::ProResProxy, size: ProxySize::Automatic },
            output: None,
        };
        assert!(proxy.validate().is_ok());
        assert!(SubmitJob { class: PriorityClass::UserJob, ..proxy }.validate().is_err());
        assert!(PriorityClass::UserJob.persists_across_relaunch() && !PriorityClass::AgentExport.persists_across_relaunch());
    }

    #[test]
    fn agent_export_replies() {
        let status = |state, path: Option<&str>| JobStatus {
            job: "job-1".into(),
            class: PriorityClass::AgentExport,
            state,
            progress: 0.0,
            path: path.map(str::to_owned),
            error: None,
        };
        let wait = AgentExportMode::Wait;
        assert_eq!(agent_export_reply(wait, &status(JobState::Queued, None), 100, 5_000), None);
        assert_eq!(agent_export_reply(wait, &status(JobState::Queued, None), 5_001, 5_000), Some(AgentExportReply::Queued { job: "job-1".into() }));
        assert_eq!(agent_export_reply(wait, &status(JobState::Running, None), 60_000, 5_000), None, "timeouts count from job start");
        assert_eq!(agent_export_reply(AgentExportMode::Async, &status(JobState::Running, None), 0, 5_000), Some(AgentExportReply::Queued { job: "job-1".into() }));
        assert_eq!(
            agent_export_reply(wait, &status(JobState::Done, Some("/Users/x/Powermove/Launch cut.mov")), 0, 5_000),
            Some(AgentExportReply::Done { job: "job-1".into(), path: "/Users/x/Powermove/Launch cut.mov".into() })
        );
        assert!(matches!(agent_export_reply(wait, &status(JobState::Done, None), 0, 5_000), Some(AgentExportReply::Failed { .. })));
        assert!(matches!(agent_export_reply(wait, &status(JobState::Failed, None), 0, 5_000), Some(AgentExportReply::Failed { .. })));
    }
}
