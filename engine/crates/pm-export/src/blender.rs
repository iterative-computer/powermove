//! The Blender stage of an export (plan section 17's pipeline:
//! `evaluate → [Blender stage] → render`; contract 11).
//!
//! Before each frame is composited, the renderer plans it without drawing
//! and lists the snapshot of every Blender frame it needs (per nested
//! composition at the plan's own times and sizes; per shutter sample once
//! pm-render draws motion blur); the stage
//! ([`pm_blender::job::BlenderStage`]) finds each layer's surface in the
//! cache or has Blender render it, and the frame then renders with
//! [`BlenderRender::Final`](pm_render::blender::BlenderRender::Final).
//! The job renders from its own frozen document, so edits never
//! invalidate its surfaces (appendix A item 31); a surface is rendered at
//! most once per key; a missing surface, a Blender failure, or Blender not
//! being installed fails the export (never draft shading in its place);
//! cancelling the export stops Blender.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pm_blender::BlenderError;
use pm_blender::job::{BlenderBackend, BlenderStage, Surfaces};
use pm_model::Document;
use pm_protocol::common::Ticks;
use pm_render::blender::SnapshotOptions;
use pm_render::{AssetResolver, RenderRequest, Renderer};

use crate::render::JobError;

/// Snapshot build failures the static check routes (the host renders them
/// with the current engine): imported models and recipe parts.
fn routed(why: &str) -> bool {
    matches!(why, "imported models" | "model-group recipe parts")
}

/// The Blender stage of one job lane: its stage (cache, device record) and
/// the backend that runs Blender.
pub struct ExportBlender {
    pub stage: BlenderStage,
    pub backend: Box<dyn BlenderBackend + Send>,
}

/// Progress of the stage for the job's status (surfaces done, total).
pub type BlenderProgress<'a> = &'a mut dyn FnMut(u32, u32);

impl ExportBlender {
    pub fn new(stage: BlenderStage, backend: Box<dyn BlenderBackend + Send>) -> ExportBlender {
        ExportBlender { stage, backend }
    }

    /// The surfaces of frame `time`, or `None` when it has no Blender frames.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        renderer: &mut Renderer,
        document: &Document,
        composition: &str,
        time: Ticks,
        request: &RenderRequest,
        media: &dyn AssetResolver,
        cancel: &AtomicBool,
        progress: BlenderProgress<'_>,
    ) -> Result<Option<Arc<Surfaces>>, JobError> {
        let snaps = renderer.blender_snapshots(
            document,
            composition,
            time,
            request,
            media,
            false,
            SnapshotOptions::default(),
        )?;
        if let Some((frame, why)) = snaps.errors.first() {
            return Err(if routed(why) {
                JobError::Unsupported(vec![format!("Blender-rendered 3D: {why}")])
            } else {
                JobError::Blender(BlenderError::Failed(format!(
                    "The Blender frame of {} at {} s could not be prepared: {why}",
                    frame.composition(),
                    frame.time
                )))
            });
        }
        if snaps.snapshots.is_empty() {
            return Ok(None);
        }
        let surfaces = self
            .stage
            .prepare(&mut *self.backend, &snaps.snapshots, cancel, progress)
            .map_err(|e| match e {
                BlenderError::Cancelled => JobError::Cancelled,
                other => JobError::Blender(other),
            })?;
        Ok(Some(Arc::new(surfaces)))
    }
}

/// `request` drawing `surfaces` (or native shading without Blender frames).
pub fn with_surfaces(request: &RenderRequest, surfaces: Option<Arc<Surfaces>>) -> RenderRequest {
    let mut r = request.clone();
    r.blender = match surfaces {
        Some(s) => pm_render::blender::BlenderRender::Final(s),
        None => pm_render::blender::BlenderRender::Final(Arc::new(Surfaces::new())),
    };
    r
}
