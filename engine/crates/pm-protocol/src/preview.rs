//! Session-scoped control and completion of the paused Blender viewer preview.
//! A generation identifies one explicit renderer request; revision and frame
//! sequence identify the document and presented frame that satisfied it.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::common::{CompositionId, Revision, Seq, Size, Ticks};
use crate::frame::{EditorCamera, Projection, Shading3d};
use crate::validate::{Validate, Violation, ensure};
use crate::version::ContractId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum PreviewAction {
    Render,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PreviewControl {
    pub generation: Seq,
    pub revision: Revision,
    pub composition: CompositionId,
    pub time: Ticks,
    pub shading: Shading3d,
    pub action: PreviewAction,
    /// Prepare preview Blender surfaces without changing or presenting a
    /// viewer frame (Solid, playback, and viewerless renderFrame calls).
    #[serde(default)]
    pub cache_only: bool,
    /// The composition's full size; the Blender snapshot applies its own
    /// preview scale. Required for cache-only work.
    #[serde(default)]
    pub size: Option<Size>,
    /// Editor camera when a cache-only preview is requested in editor view.
    #[serde(default)]
    pub camera: Option<EditorCamera>,
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
    /// A cache-only Ready has no presented frame sequence.
    #[serde(default)]
    pub cache_only: bool,
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
        )?;
        if self.cache_only {
            ensure(
                self.action == PreviewAction::Render
                    && self.size.is_some_and(|size| {
                        !size.is_empty() && size.width <= 8192 && size.height <= 8192
                    }),
                ContractId::Blender,
                || "cache-only preview needs a size within 1..8192".into(),
            )?;
        } else {
            ensure(
                self.size.is_none() && self.camera.is_none(),
                ContractId::Blender,
                || "viewer preview cannot carry cache-only size or camera".into(),
            )?;
        }
        if let Some(camera) = self.camera {
            let projection = match camera.projection {
                Projection::Perspective {
                    fov_y_degrees,
                    near,
                    far,
                } => {
                    fov_y_degrees.is_finite()
                        && fov_y_degrees > 0.0
                        && fov_y_degrees < 180.0
                        && near.is_finite()
                        && near > 0.0
                        && far.is_finite()
                        && far > near
                }
                Projection::Orthographic {
                    half_height,
                    near,
                    far,
                } => {
                    half_height.is_finite()
                        && half_height > 0.0
                        && near.is_finite()
                        && far.is_finite()
                        && far > near
                }
            };
            ensure(
                camera.view.iter().all(|n| n.is_finite()) && projection,
                ContractId::Blender,
                || "invalid preview editor camera".into(),
            )?;
        }
        Ok(())
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
            self.phase != PreviewPhase::Ready || self.cache_only || self.frame_seq.is_some(),
            ContractId::Blender,
            || "ready preview needs a frame sequence".into(),
        )?;
        ensure(
            self.phase != PreviewPhase::Failed
                || self.error.as_deref().is_some_and(|e| !e.is_empty()),
            ContractId::Blender,
            || "failed preview needs an error".into(),
        )?;
        ensure(
            !self.cache_only || (!self.automatic && self.frame_seq.is_none()),
            ContractId::Blender,
            || "cache-only preview is explicit and has no frame sequence".into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_ready_has_no_frame_and_cache_request_needs_bounded_size() {
        let control = PreviewControl {
            generation: Seq::new(1).unwrap(),
            revision: Revision::ZERO,
            composition: CompositionId::new("comp"),
            time: Ticks::ZERO,
            shading: Shading3d::Rendered,
            action: PreviewAction::Render,
            cache_only: true,
            size: Some(Size::new(640, 360)),
            camera: None,
        };
        assert!(control.validate().is_ok());
        assert!(
            PreviewControl {
                size: Some(Size::new(0, 360)),
                ..control.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            PreviewControl {
                size: Some(Size::new(8193, 360)),
                ..control.clone()
            }
            .validate()
            .is_err()
        );
        let status = PreviewStatus {
            generation: control.generation,
            automatic: false,
            revision: control.revision,
            composition: control.composition,
            time: control.time,
            phase: PreviewPhase::Ready,
            frame_seq: None,
            error: None,
            cache_only: true,
        };
        assert!(status.validate().is_ok());
        assert!(
            PreviewStatus {
                cache_only: false,
                ..status
            }
            .validate()
            .is_err()
        );
    }
}
