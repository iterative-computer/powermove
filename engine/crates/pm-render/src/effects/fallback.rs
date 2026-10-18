//! The decision 19 fallback runner for effects and transitions (plan
//! section 11; amendment 21: a program WebGL draws that the native
//! pipeline cannot translate is drawn, not routed).
//!
//! A step whose program only the runner can draw runs in three moves
//! inside the frame (`exec.rs`, `Gpu::fallback_step`):
//!
//! 1. The frame's commands so far are submitted and the step's textures
//!    (an effect's input and backdrop, a transition's two frames, E-encoded
//!    as every legacy program reads them) are read back as binary16 RGBA in
//!    GL row order.
//! 2. The request (`shader_layers::fallback::request_with_textures`: the
//!    wrapped program, the compositor's calls for every pass, which unit
//!    samples which texture) is keyed by everything that decides its
//!    pixels, the textures' pixels included.
//! 3. Pixels the runner returned for that key are written into a working
//!    target that continues the chain (`clamp` after it, like a native
//!    step). Without them the step is left out and the frame is not
//!    presented: the request and its textures wait here for the runner
//!    (`RenderError::ShaderFallbackPending`; a job renderer with a
//!    [`FallbackHost`](crate::shader_layers::FallbackHost) runs it at once
//!    and renders the frame again).
//!
//! A program the runner could not draw is not asked for again until its
//! source changes: a program WebGL refuses (`compile`) is skipped and
//! reported, as `runEffects` skips one today; one whose draw passed the
//! runner's deadline (`watchdog`) is turned off by the GPU watchdog like a
//! runaway native program.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::shader_layers::{FallbackFailure, FallbackPixels, FallbackRequest, FallbackStore};

/// `shader_layers::fallback::program_id` of a program not yet requested.
pub(crate) fn program_id_of(vertex: &str, fragment: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(vertex.as_bytes());
    h.update([0]);
    h.update(fragment.as_bytes());
    h.finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}

/// Requests whose textures are kept for the runner at once (the oldest
/// unanswered one is dropped past it and asked for again by a later frame).
const WAITING: usize = 32;

/// Runner results kept (bytes): a few full-HD steps of an animation.
const RESULTS: u64 = 512 << 20;

/// The renderer's effect and transition fallback state.
#[derive(Debug)]
pub struct EffectFallback {
    /// A runner draws for this renderer (the viewer's, or a host's).
    pub(crate) runner: bool,
    results: FallbackStore,
    /// Requests asked for and not answered, with their textures.
    waiting: HashMap<String, (Arc<FallbackRequest>, Vec<Arc<FallbackPixels>>)>,
    order: VecDeque<String>,
    /// Programs the runner could not draw, by `program_id`.
    failed: HashMap<String, FallbackFailure>,
    /// Failures already reported (once per program).
    reported: HashSet<String>,
    /// Requests the frame being drawn waits for.
    pub(crate) pending: Vec<Arc<FallbackRequest>>,
    /// A request waits in the frame being drawn (a step's or a shader
    /// layer's): later steps are not asked for in this frame.
    pub(crate) blocked: bool,
    /// Job policy: why the frame cannot be delivered (the runner could not
    /// draw a program WebGL draws today).
    pub(crate) fatal: Option<String>,
}

impl Default for EffectFallback {
    fn default() -> Self {
        EffectFallback {
            runner: false,
            results: FallbackStore::with_budget(RESULTS),
            waiting: HashMap::new(),
            order: VecDeque::new(),
            failed: HashMap::new(),
            reported: HashSet::new(),
            pending: Vec::new(),
            fatal: None,
            blocked: false,
        }
    }
}

impl EffectFallback {
    /// Whether a request with this key waits for the runner here.
    pub fn expects(&self, key: &str) -> bool {
        self.waiting.contains_key(key)
    }

    /// The textures of a request that waits (their pixels go to the runner
    /// with it).
    pub fn textures(&self, key: &str) -> Option<Vec<Arc<FallbackPixels>>> {
        self.waiting.get(key).map(|(_, t)| t.clone())
    }

    /// The runner's pixels for a request.
    pub fn put_pixels(&mut self, key: String, pixels: FallbackPixels) {
        self.forget(&key);
        self.results.insert(key, pixels);
    }

    /// The runner could not draw a request: its program is not asked for
    /// again until its source changes.
    pub fn put_failure(&mut self, key: &str, failure: FallbackFailure) {
        if let Some((request, _)) = self.waiting.get(key) {
            let program = crate::shader_layers::fallback::program_id(request);
            self.failed.insert(program, failure);
        }
        self.forget(key);
    }

    /// Why the runner could not draw `program`, if it could not.
    pub(crate) fn failure(&self, program: &str) -> Option<&FallbackFailure> {
        self.failed.get(program)
    }

    /// Whether `program`'s failure still has to be reported (once).
    pub(crate) fn report_once(&mut self, program: &str) -> bool {
        self.reported.insert(program.to_owned())
    }

    pub(crate) fn result(&mut self, key: &str) -> Option<Arc<FallbackPixels>> {
        self.results.get(key)
    }

    /// A request the frame waits for, with its textures.
    pub(crate) fn wait(
        &mut self,
        request: Arc<FallbackRequest>,
        textures: Vec<Arc<FallbackPixels>>,
    ) {
        if !self.waiting.contains_key(&request.key) {
            while self.order.len() >= WAITING {
                if let Some(old) = self.order.pop_front() {
                    self.waiting.remove(&old);
                }
            }
            self.order.push_back(request.key.clone());
            self.waiting
                .insert(request.key.clone(), (request.clone(), textures));
        }
        if !self.pending.iter().any(|p| p.key == request.key) {
            self.pending.push(request);
        }
        self.blocked = true;
    }

    fn forget(&mut self, key: &str) {
        self.waiting.remove(key);
        self.order.retain(|k| k != key);
    }

    /// Results and requests held (tests, stats).
    pub fn results(&self) -> usize {
        self.results.len()
    }

    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }
}
