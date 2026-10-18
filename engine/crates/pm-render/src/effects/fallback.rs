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
//! Only two failures say something about the program, and only they are
//! kept until its source changes ([`FallbackFailure::is_final`]): a program
//! WebGL refuses (`compile`) is skipped and reported, as `runEffects` skips
//! one today; one whose draw passed the runner's deadline (`watchdog`) is
//! turned off by the GPU watchdog like a runaway native program. Every
//! other failure (the runner's context was lost, its compile deadline
//! passed, no runner answered, a wrong size) is transient and the program
//! is asked for again ([`Retries`]), as today's compositor draws again once
//! its context is restored (`compositor.ts` clears `GL.errors` and
//! `GL.progs` on `webglcontextrestored`): the first retry at once, later
//! ones after a backoff, during which the viewer draws the frame without
//! the program and reports it once. A job retries in place and stops with
//! a named error if the runner still cannot draw it; it never delivers a
//! frame without it.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    /// Programs the runner refused or stopped (final failures), by
    /// `program_id`.
    failed: HashMap<String, FallbackFailure>,
    /// Programs whose last request failed for a transient reason, by
    /// `program_id` (effects, transitions, and shader layers).
    pub(crate) retries: Retries,
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
            retries: Retries::default(),
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

    /// The runner's pixels for a request: its program drew, so an earlier
    /// transient failure is over.
    pub fn put_pixels(&mut self, key: String, pixels: FallbackPixels) {
        if let Some((request, _)) = self.waiting.get(&key) {
            let program = crate::shader_layers::fallback::program_id(request);
            self.retries.succeeded(&program);
        }
        self.forget(&key);
        self.results.insert(key, pixels);
    }

    /// The runner could not draw a request. A final failure keeps its
    /// program from being asked for again until its source changes; a
    /// transient one asks again after the returned delay.
    pub fn put_failure(
        &mut self,
        key: &str,
        failure: FallbackFailure,
        now: Instant,
    ) -> Option<Duration> {
        let delay = if let Some((request, _)) = self.waiting.get(key) {
            let program = crate::shader_layers::fallback::program_id(request);
            self.program_failed(&program, failure, now)
        } else {
            None
        };
        self.forget(key);
        delay
    }

    /// A program's request failed (an effect's, a transition's, or a
    /// shader layer's): see [`EffectFallback::put_failure`].
    pub(crate) fn program_failed(
        &mut self,
        program: &str,
        failure: FallbackFailure,
        now: Instant,
    ) -> Option<Duration> {
        if failure.is_final() {
            self.retries.succeeded(program);
            self.failed.insert(program.to_owned(), failure);
            None
        } else {
            Some(self.retries.failed(program, failure, now))
        }
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

/// Backoff before the next attempt after a program's `attempts`-th
/// transient failure in a row: at once after the first (the runner made a
/// new context), then 250 ms, 1 s, 4 s, and 8 s from then on.
pub fn retry_delay(attempts: u32) -> Duration {
    match attempts {
        0 | 1 => Duration::ZERO,
        2 => Duration::from_millis(250),
        3 => Duration::from_secs(1),
        4 => Duration::from_secs(4),
        _ => Duration::from_secs(8),
    }
}

/// A program's transient failures in a row.
#[derive(Debug, Clone)]
pub(crate) struct Retry {
    pub failure: FallbackFailure,
    pub attempts: u32,
    pub at: Instant,
    /// Reported in this streak (once).
    pub reported: bool,
}

/// What to do with a program whose last request failed transiently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RetryState {
    /// Ask the runner again now.
    Due,
    /// Not yet: drawn without it (the viewer), reported when `report`.
    Wait {
        failure: FallbackFailure,
        report: bool,
    },
}

/// Programs whose last request failed for a transient reason.
#[derive(Debug, Default)]
pub(crate) struct Retries {
    map: HashMap<String, Retry>,
}

impl Retries {
    /// Records a transient failure; the delay before the next attempt.
    pub fn failed(&mut self, program: &str, failure: FallbackFailure, now: Instant) -> Duration {
        let entry = self.map.entry(program.to_owned()).or_insert(Retry {
            failure: failure.clone(),
            attempts: 0,
            at: now,
            reported: false,
        });
        entry.attempts += 1;
        entry.failure = failure;
        let delay = retry_delay(entry.attempts);
        entry.at = now + delay;
        delay
    }

    /// The program drew (or failed for good): its streak is over.
    pub fn succeeded(&mut self, program: &str) {
        self.map.remove(program);
    }

    /// Where a program with a transient streak stands at `now`.
    pub fn state(&mut self, program: &str, now: Instant) -> Option<RetryState> {
        let entry = self.map.get_mut(program)?;
        if now >= entry.at {
            return Some(RetryState::Due);
        }
        let report = !std::mem::replace(&mut entry.reported, true);
        Some(RetryState::Wait {
            failure: entry.failure.clone(),
            report,
        })
    }

    /// The program's streak, if it has one.
    pub fn last(&self, program: &str) -> Option<&Retry> {
        self.map.get(program)
    }

    /// Forgets every streak (a job's frame starts with fresh attempts).
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// The earliest pending retry (a host wakes then and draws again).
    pub fn next_at(&self) -> Option<Instant> {
        self.map.values().map(|r| r.at).min()
    }

    /// Programs with a transient streak.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lost() -> FallbackFailure {
        FallbackFailure {
            reason: "contextLost".into(),
            message: "lost".into(),
        }
    }

    #[test]
    fn transient_failures_back_off_and_end_when_the_program_draws() {
        let t0 = Instant::now();
        let mut r = Retries::default();
        assert_eq!(r.failed("p", lost(), t0), Duration::ZERO);
        assert_eq!(r.state("p", t0), Some(RetryState::Due), "the first retry is at once");
        assert_eq!(r.failed("p", lost(), t0), Duration::from_millis(250));
        assert_eq!(
            r.state("p", t0),
            Some(RetryState::Wait { failure: lost(), report: true })
        );
        assert_eq!(
            r.state("p", t0 + Duration::from_millis(100)),
            Some(RetryState::Wait { failure: lost(), report: false }),
            "reported once per streak"
        );
        assert_eq!(r.state("p", t0 + Duration::from_millis(250)), Some(RetryState::Due));
        assert_eq!(r.next_at(), Some(t0 + Duration::from_millis(250)));
        for expected in [1_000, 4_000, 8_000, 8_000] {
            assert_eq!(r.failed("p", lost(), t0), Duration::from_millis(expected));
        }
        r.succeeded("p");
        assert_eq!(r.state("p", t0), None);
        assert!(r.is_empty());
    }

    #[test]
    fn only_a_refusal_or_a_runaway_is_final() {
        for (reason, fin) in [
            ("compile", true),
            ("watchdog", true),
            ("contextLost", false),
            ("deadline", false),
            ("unavailable", false),
            ("size", false),
        ] {
            let f = FallbackFailure { reason: reason.into(), message: String::new() };
            assert_eq!(f.is_final(), fin, "{reason}");
        }
    }
}
