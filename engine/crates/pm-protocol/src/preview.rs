//! Session-scoped control and completion of the paused Blender viewer preview.
//! A generation identifies one explicit renderer request; revision and frame
//! sequence identify the document and presented frame that satisfied it.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::common::{CompositionId, Revision, Seq, Ticks};
use crate::frame::Shading3d;
use crate::validate::{Validate, Violation, ensure};
use crate::version::ContractId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum PreviewAction {
    Render,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PreviewControl {
    pub generation: Seq,
    pub revision: Revision,
    pub composition: CompositionId,
    pub time: Ticks,
    pub shading: Shading3d,
    pub action: PreviewAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum PreviewPhase {
    Queued,
    Running,
    Ready,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PreviewStatus {
    // Per-session monotonically increasing generation for this kind of work.
    pub generation: Seq,
    // Automatic Material/Rendered shading work has its own generation line.
    pub automatic: bool,
    pub revision: Revision,
    pub composition: CompositionId,
    pub time: Ticks,
    pub phase: PreviewPhase,
    pub frame_seq: Option<Seq>,
    pub error: Option<String>,
}

impl Validate for PreviewControl {
    fn validate(&self) -> Result<(), Violation> {
        ensure(self.generation != Seq::ZERO, ContractId::Blender, || {
            "preview generation must be positive".into()
        })?;
        ensure(self.composition.is_valid(), ContractId::Blender, || {
            "invalid preview composition".into()
        })?;
        ensure(self.time.get() >= 0, ContractId::Blender, || {
            "preview time must be nonnegative".into()
        })?;
        ensure(
            matches!(self.shading, Shading3d::Material | Shading3d::Rendered),
            ContractId::Blender,
            || "preview control needs a Blender shading".into(),
        )
    }
}

impl Validate for PreviewStatus {
    fn validate(&self) -> Result<(), Violation> {
        ensure(self.generation != Seq::ZERO, ContractId::Blender, || {
            "preview generation must be positive".into()
        })?;
        ensure(self.composition.is_valid(), ContractId::Blender, || {
            "invalid preview composition".into()
        })?;
        ensure(self.time.get() >= 0, ContractId::Blender, || {
            "preview time must be nonnegative".into()
        })?;
        ensure(
            self.phase != PreviewPhase::Ready || self.frame_seq.is_some(),
            ContractId::Blender,
            || "ready preview needs a frame sequence".into(),
        )?;
        ensure(
            self.phase != PreviewPhase::Failed
                || self.error.as_deref().is_some_and(|e| !e.is_empty()),
            ContractId::Blender,
            || "failed preview needs an error".into(),
        )
    }
}
