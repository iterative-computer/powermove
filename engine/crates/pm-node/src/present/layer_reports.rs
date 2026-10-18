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
    runtime: BTreeMap<(String, String), (Revision, String)>,
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

    /// Runtime media failures can change within a revision (a linked file
    /// goes offline or a requested decode fails). Report them before the
    /// frame, without flooding the port on every playback frame.
    pub(crate) fn runtime_due(
        &mut self,
        revision: Revision,
        failures: &[pm_render::LayerFailure],
    ) -> Vec<EngineError> {
        use pm_protocol::error::{ErrorCode, MAX_ERROR_MESSAGE_BYTES, Retry};
        let current: BTreeMap<_, _> = failures
            .iter()
            .map(|f| {
                (
                    (f.composition.clone(), f.layer.clone()),
                    (revision, f.message.clone()),
                )
            })
            .collect();
        let reports = current
            .iter()
            .filter(|(key, value)| self.runtime.get(*key) != Some(*value))
            .map(|((composition, layer), (_, detail))| {
                let mut message = format!("Layer {layer} in {composition}: {detail}");
                if message.len() > MAX_ERROR_MESSAGE_BYTES {
                    let mut end = MAX_ERROR_MESSAGE_BYTES;
                    while !message.is_char_boundary(end) {
                        end -= 1;
                    }
                    message.truncate(end);
                }
                EngineError::new(
                    ErrorCode::DecodeFailed,
                    ErrorScope::Layer {
                        layer: layer.as_str().into(),
                    },
                    message,
                )
                .with_retry(Retry::AfterUserAction)
            })
            .collect();
        self.runtime = current;
        reports
    }

    /// Forgets what was said (the composition routed to the legacy viewer,
    /// whose compositor fills the shader meta itself): the next frame the
    /// engine presents repeats every report.
    pub(crate) fn reset(&mut self) {
        self.told = None;
        self.runtime.clear();
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
        assert_eq!(
            r.due(rev(1), None, "c", || vec![a.clone()]),
            vec![a.clone()]
        );
        // The same frame key: nothing is computed or sent.
        assert!(r.due(rev(1), None, "c", || unreachable!()).is_empty());
        // New definitions at the same revision: only what changed.
        let b = report("b", "b is missing");
        assert_eq!(
            r.due(rev(1), Some(Seq::new(2).unwrap()), "c", || vec![
                a.clone(),
                b.clone()
            ]),
            vec![b.clone()]
        );
        // A new revision repeats every report that still holds.
        assert_eq!(
            r.due(rev(2), Some(Seq::new(2).unwrap()), "c", || vec![
                a.clone(),
                b.clone()
            ]),
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
    #[test]
    fn runtime_failures_report_changes_recovery_and_revisions() {
        let mut reports = LayerReports::default();
        let failure = pm_render::LayerFailure {
            composition: "c".into(),
            layer: "image".into(),
            message: "photo.png is offline".into(),
        };
        let first = reports.runtime_due(rev(1), std::slice::from_ref(&failure));
        assert_eq!(first.len(), 1);
        assert_eq!(
            first[0].scope,
            ErrorScope::Layer {
                layer: "image".into()
            }
        );
        assert!(first[0].message.contains("photo.png"));
        assert!(
            reports
                .runtime_due(rev(1), std::slice::from_ref(&failure))
                .is_empty()
        );
        assert_eq!(
            reports
                .runtime_due(rev(2), std::slice::from_ref(&failure))
                .len(),
            1
        );
        assert!(reports.runtime_due(rev(2), &[]).is_empty());
        assert_eq!(reports.runtime_due(rev(2), &[failure]).len(), 1);
        let long = pm_render::LayerFailure {
            composition: "c".into(),
            layer: "image".into(),
            message: "é".repeat(4096),
        };
        let error = reports.runtime_due(rev(2), &[long]).pop().unwrap();
        use pm_protocol::validate::Validate;
        assert!(error.validate().is_ok());
    }
}
