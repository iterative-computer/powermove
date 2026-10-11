//! Priority classes and agent frame requests (contract 12).
//!
//! Interactive frames come first, then agent snapshots, then jobs (section 4,
//! principle 7). Agent exports run in their own lane ahead of user batch jobs
//! (section 17). Agent frame requests are batched, carry the agent's revision
//! for read-your-writes, queue instead of failing, and never pause playback.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::common::{AgentRunId, Bytes, CompositionId, JobId, Revision, Ticks};
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
#[serde(rename_all = "camelCase", tag = "kind")]
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
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ImagePayload {
    /// Small images inline (under the message limit).
    Inline { bytes: Bytes },
    /// Larger images in a cache file behind a capability handle.
    CacheFile { handle: CapabilityHandle, length: u32 },
}

/// The result for one requested frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", tag = "status")]
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
}
