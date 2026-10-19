//! The definitions a job lane's renderer draws with (contract 10; plan
//! section 17: "Runs in the job engine from a snapshot with definition
//! snapshots").
//!
//! Each job and agent frame batch carries the definition set its session
//! held when it was submitted (`JobDocument::definitions`): the winning
//! effect, transition, and layer registrations of the window that asked,
//! forks and overrides by id included. The lane's renderer loads that set
//! before the work is planned, so the capability check and the render use
//! the same definitions, and keeps translated programs for definitions
//! whose source did not change from one job to the next.
//!
//! Every work item starts with a fresh watchdog (`Policy::Job`): nothing an
//! earlier job's runaway turned off carries over, and a job never skips a
//! program (it fails with a named error instead).

use std::sync::Arc;

use pm_engine::jobs::JobDocument;
use pm_protocol::definitions::DefinitionSetSnapshot;
use pm_render::Renderer;
use pm_render::effects::watchdog::Policy;

/// The set a lane's renderer holds.
#[derive(Default)]
pub(crate) struct LaneDefinitions {
    applied: Option<Arc<DefinitionSetSnapshot>>,
}

impl LaneDefinitions {
    /// Loads `document`'s definitions into `renderer` (nothing to do when it
    /// already holds that set) and resets its watchdog for the work item.
    /// A document without definitions leaves the renderer with none: its
    /// effects, transitions, and extension layers are reported unsupported
    /// by the capability check, never drawn from a stale set. `Err` when the
    /// renderer refuses the set (the work fails with that message).
    pub(crate) fn use_for(
        &mut self,
        renderer: &mut Renderer,
        document: &JobDocument,
    ) -> Result<(), String> {
        match &document.definitions {
            Some(set) => {
                if !self.applied.as_ref().is_some_and(|a| Arc::ptr_eq(a, set)) {
                    // Per-definition failures are statuses the plan reads
                    // (programs WebGL refuses are skipped as it skips them).
                    if let Err(e) = renderer.set_definitions(set) {
                        renderer.clear_definitions();
                        self.applied = None;
                        return Err(format!("The job's definitions were refused: {e}"));
                    }
                    self.applied = Some(set.clone());
                }
            }
            None => {
                if self.applied.take().is_some() || renderer.effects().configured() {
                    renderer.clear_definitions();
                }
            }
        }
        let watchdog = &mut renderer.effects_mut().watchdog;
        watchdog.set_policy(Policy::Job);
        watchdog.reset();
        Ok(())
    }
}
