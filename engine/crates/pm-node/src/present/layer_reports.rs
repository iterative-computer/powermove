//! The shader UI's layer reports while the engine presents (plan section
//! 11: `uiState.getShaderMeta` and the compile errors are "filled from
//! engine render events at the frame's revision"; contract 7
//! `FailureReport`).
//!
//! `pm_render::Renderer::layer_reports` says, for a document revision and
//! the viewed composition, which shader layers do not compile
//! (`ShaderCompile`) and which extension layers miss their definition
//! (`DefinitionMissing`), layer scope. A layer report is a statement about
//! that revision, so the presenter sends the whole list at every revision it
//! presents, before the frame, and only what changed when the definitions
//! or the composition change at the same revision. The app clears a layer
//! report once it shows a frame of a later revision that did not repeat it
//! (`kernel/engine-definitions.ts`), and the missing state of the layers of
//! a definition it pushes again.

use std::collections::BTreeMap;

use pm_protocol::common::{Revision, Seq};
use pm_protocol::error::{EngineError, ErrorScope};

/// What a view was last told.
#[derive(Debug, Clone, Default)]
pub(crate) struct LayerReports {
    told: Option<Told>,
}

#[derive(Debug, Clone)]
struct Told {
    revision: Revision,
    generation: Option<Seq>,
    composition: String,
    reports: BTreeMap<String, EngineError>,
}

fn layer(error: &EngineError) -> String {
    match &error.scope {
        ErrorScope::Layer { layer } => layer.as_str().to_owned(),
        _ => String::new(),
    }
}

impl LayerReports {
    /// The reports to send with a frame of `composition` at `revision` drawn
    /// with the definitions of `generation`. `compute` runs only when one
    /// of the three changed.
    pub(crate) fn due(
        &mut self,
        revision: Revision,
        generation: Option<Seq>,
        composition: &str,
        compute: impl FnOnce() -> Vec<EngineError>,
    ) -> Vec<EngineError> {
        if let Some(told) = &self.told
            && told.revision == revision
            && told.generation == generation
            && told.composition == composition
        {
            return Vec::new();
        }
        let reports: BTreeMap<String, EngineError> =
            compute().into_iter().map(|e| (layer(&e), e)).collect();
        let due = match &self.told {
            // The same revision: only what is new or different.
            Some(told) if told.revision == revision => reports
                .iter()
                .filter(|(id, e)| told.reports.get(*id) != Some(e))
                .map(|(_, e)| e.clone())
                .collect(),
            // A new revision (or the first frame): everything that holds.
            _ => reports.values().cloned().collect(),
        };
        self.told = Some(Told {
            revision,
            generation,
            composition: composition.to_owned(),
            reports,
        });
        due
    }

    /// Forgets what was said (the composition routed to the legacy viewer,
    /// whose compositor fills the shader meta itself): the next frame the
    /// engine presents repeats every report.
    pub(crate) fn reset(&mut self) {
        self.told = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::error::ErrorCode;

    fn report(layer: &str, message: &str) -> EngineError {
        EngineError::new(
            ErrorCode::ShaderCompile,
            ErrorScope::Layer {
                layer: layer.into(),
            },
            message,
        )
    }

    fn rev(n: u64) -> Revision {
        Revision::new(n).unwrap()
    }

    #[test]
    fn reports_repeat_per_revision_and_send_changes_within_one() {
        let mut r = LayerReports::default();
        let a = report("a", "a fails");
        assert_eq!(r.due(rev(1), None, "c", || vec![a.clone()]), vec![a.clone()]);
        // The same frame key: nothing is computed or sent.
        assert!(r.due(rev(1), None, "c", || unreachable!()).is_empty());
        // New definitions at the same revision: only what changed.
        let b = report("b", "b is missing");
        assert_eq!(
            r.due(rev(1), Some(Seq::new(2).unwrap()), "c", || vec![a.clone(), b.clone()]),
            vec![b.clone()]
        );
        // A new revision repeats every report that still holds.
        assert_eq!(
            r.due(rev(2), Some(Seq::new(2).unwrap()), "c", || vec![a.clone(), b.clone()]),
            vec![a.clone(), b.clone()]
        );
        // A changed message at the same revision is sent again.
        let a2 = report("a", "a fails differently");
        assert_eq!(
            r.due(rev(2), Some(Seq::new(2).unwrap()), "d", || vec![a2.clone()]),
            vec![a2.clone()]
        );
        r.reset();
        assert_eq!(
            r.due(rev(2), Some(Seq::new(2).unwrap()), "d", || vec![a2.clone()]),
            vec![a2]
        );
    }
}
