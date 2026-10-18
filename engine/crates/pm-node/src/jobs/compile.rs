//! Compiling pushed definitions (contract 10; plan section 11 and the
//! `validate_effect` agent tool).
//!
//! A `pushDefinitions` to a job session is acknowledged by this worker, not
//! by the engine at once: every upserted effect, transition, and fragment
//! layer definition is translated through the production GLSL pipeline
//! (`pm_glsl`: limits, ES validation, glslang, the SPIR-V rewrites and bind
//! check, naga), and every user definition (any origin but built-in) also
//! has its GPU pipeline built for the working format, as its first frame
//! would, so a program the device refuses is found here. The answer's
//! `failures` are the definitions' errors: `shaderCompile` for a program
//! WebGL refuses too (nothing draws it; the diagnostics are in the author's
//! lines), `notHeadlessRenderable` for one only the app's WebGL fallback
//! runner draws (valid in the app). Acceptance is never narrower than
//! WebGL's: an engine-only failure is not reported as a compile error.
//!
//! Translations are cached by source (a re-push of an unchanged set costs
//! a hash per definition), and one worker answers pushes in the order they
//! arrived, so a caller that waits for the acknowledgment before
//! submitting a job or a frame batch knows the job engine holds the set.

use std::collections::VecDeque;
use std::sync::Arc;

use pm_protocol::common::{RequestId, SessionId};
use pm_protocol::definitions::{
    DefinitionOrigin, DefinitionSetSnapshot, DefinitionsApplied, DefinitionsUpdate,
};
use pm_protocol::envelope::{Envelope, Message, Response};
use pm_protocol::error::EngineError;
use pm_protocol::host::PortId;
use pm_render::Effects;

use super::Shared;

/// A push waiting for its answer.
pub(crate) struct Pending {
    pub port: PortId,
    pub id: RequestId,
    pub session: SessionId,
    pub update: DefinitionsUpdate,
}

/// Pushes in arrival order.
#[derive(Default)]
pub(crate) struct Queue(pub VecDeque<Pending>);

/// The worker: answers every push in order until the service shuts down.
pub(crate) fn run(shared: &Arc<Shared>) {
    let base = Effects::new(format!("{}; wgpu 30", std::env::consts::OS));
    loop {
        let pending = {
            let mut st = shared.lock();
            loop {
                if st.shutdown {
                    return;
                }
                if let Some(p) = st.compiles.0.pop_front() {
                    break p;
                }
                st = shared.work.wait(st).unwrap_or_else(|e| e.into_inner());
            }
        };
        let failures = compile(shared, &base, &pending.update);
        let mut envelope = Envelope::new(Message::Response {
            id: pending.id,
            body: Response::DefinitionsApplied(DefinitionsApplied {
                generation: pending.update.generation,
                failures,
            }),
        });
        envelope.session = Some(pending.session);
        shared.send(pending.port, envelope);
    }
}

/// The failures of `update`'s upserts.
pub(crate) fn compile(
    shared: &Shared,
    base: &Effects,
    update: &DefinitionsUpdate,
) -> Vec<EngineError> {
    if update.upserts.is_empty() {
        return Vec::new();
    }
    let mut effects = base.sibling();
    let set = DefinitionSetSnapshot {
        generation: update.generation,
        definitions: update.upserts.clone(),
        missing: Vec::new(),
    };
    // The engine applied this update already, so it is valid.
    let mut failures = effects.set_snapshot(&set).unwrap_or_default();
    let device = shared.device().ok();
    for snapshot in &update.upserts {
        if snapshot.origin == DefinitionOrigin::BuiltIn || snapshot.glsl.is_none() {
            continue;
        }
        if failures.iter().any(|f| {
            matches!(&f.scope, pm_protocol::error::ErrorScope::Definition { definition } if *definition == snapshot.id)
        }) {
            continue;
        }
        if let Some(device) = &device
            && let Some(error) = effects.build(device, snapshot.kind, snapshot.id.as_str())
        {
            failures.push(error);
        }
    }
    failures
}
