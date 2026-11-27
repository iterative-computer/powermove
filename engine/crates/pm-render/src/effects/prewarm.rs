//! Programs built ahead of their first frame (decision PROBE-1,
//! docs/engine/effects.md "Background probes and program builds").
//!
//! A definition's GPU program is built on first use: naga writes its MSL and
//! the Metal compiler runs on the thread that draws the frame. On a large
//! project's first play that put several hundred milliseconds of compiles
//! inside single frames. Today's compositor compiles its GL programs on
//! first use too, so its first play pays them as well; the engine moves the
//! work out of the frame instead. When a project opens or the definition
//! set changes (an extension installed, updated or reloaded), the renderer
//! queues the programs the document uses, then every other user program, to
//! one worker thread that builds them exactly as the frame would (the same
//! `Prepared::finish`, `GpuProgram::new` and pipeline for the working
//! format). A frame that reaches a program still being built waits for that
//! build, which started earlier than the frame would have; a frame that
//! reaches one never queued builds it in place, as before.
//!
//! Compiled programs persist across sessions where Metal keeps them: the
//! translation (steps 0 to 4) is in `pm_glsl::Cache` on disk, and Metal's
//! own shader cache keeps compiled functions per app bundle (wgpu exposes no
//! binary archive on Metal).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

use pm_glsl::{Failure, Prepared};
use pm_protocol::common::DefinitionId;
use pm_protocol::definitions::DefinitionKind;

use super::program::GpuProgram;

/// A worker queue whose pending jobs can move to the front without running
/// duplicate jobs. The channel wakes the worker once per submitted job.
pub(crate) struct JobQueue<T> {
    ready: mpsc::Sender<()>,
    jobs: Arc<Mutex<VecDeque<T>>>,
}

impl<T> JobQueue<T> {
    pub fn new() -> (Self, mpsc::Receiver<()>, Arc<Mutex<VecDeque<T>>>) {
        let (ready, wake) = mpsc::channel();
        let jobs = Arc::new(Mutex::new(VecDeque::new()));
        (
            Self {
                ready,
                jobs: jobs.clone(),
            },
            wake,
            jobs,
        )
    }

    pub fn send(&self, job: T) -> bool {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if self.ready.send(()).is_err() {
            return false;
        }
        jobs.push_back(job);
        true
    }

    pub fn remove(&self, matches: impl Fn(&T) -> bool) -> Option<T> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        let index = jobs.iter().position(matches)?;
        jobs.remove(index)
    }

    pub fn clear(&self) {
        self.jobs.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    pub fn promote(&self, matches: impl Fn(&T) -> bool) {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(index) = jobs.iter().position(matches) {
            let job = jobs.remove(index).expect("pending job");
            jobs.push_front(job);
        }
    }
}

pub(crate) type Key = (DefinitionKind, DefinitionId);

struct Job {
    key: Key,
    source: pm_glsl::Key,
    prepared: Arc<Prepared>,
    device: wgpu::Device,
    format: wgpu::TextureFormat,
}

/// A program the worker built (or failed to build).
pub(crate) struct Built {
    pub key: Key,
    /// The source the build belongs to: a definition changed meanwhile
    /// drops it.
    pub source: pm_glsl::Key,
    pub result: Result<GpuProgram, Failure>,
    /// How long the build took on the worker (µs).
    pub us: u64,
}

/// The worker and the builds in flight.
#[derive(Default)]
pub(crate) struct Prewarm {
    worker: Option<(JobQueue<Job>, std::sync::Mutex<mpsc::Receiver<Built>>)>,
    in_flight: HashMap<Key, pm_glsl::Key>,
    /// Builds finished off the frame thread, and their time (µs).
    pub built: u64,
    pub built_us: u64,
    /// Frames that waited for a build in flight, and how long (µs).
    pub waited: u64,
    pub waited_us: u64,
}

impl Drop for Prewarm {
    fn drop(&mut self) {
        // The worker can finish its current build, but queued work must not
        // keep a retiring device alive.
        if let Some((jobs, _)) = &self.worker {
            jobs.clear();
        }
    }
}

impl std::fmt::Debug for Prewarm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Prewarm")
            .field("in_flight", &self.in_flight.len())
            .field("built", &self.built)
            .finish()
    }
}

/// Builds a program as its first frame would (the Metal compiler runs in
/// the pipeline).
pub(crate) fn build(
    device: &wgpu::Device,
    prepared: &Prepared,
    label: &str,
    format: wgpu::TextureFormat,
) -> Result<GpuProgram, Failure> {
    let mut program = prepared
        .finish()
        .and_then(|p| GpuProgram::new(device, p, label))?;
    program.pipeline(device, format)?;
    Ok(program)
}

impl Prewarm {
    fn worker(&mut self) -> Option<&JobQueue<Job>> {
        if self.worker.is_none() {
            let (jobs, rx, pending) = JobQueue::<Job>::new();
            let (tx, done) = mpsc::channel::<Built>();
            std::thread::Builder::new()
                .name("pm-program-prewarm".into())
                .spawn(move || {
                    pm_glsl::compiler::background_worker();
                    // Ends when the renderer drops its sender.
                    while rx.recv().is_ok() {
                        let job = pending
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .pop_front();
                        let Some(job) = job else { continue };
                        let started = Instant::now();
                        let result =
                            build(&job.device, &job.prepared, job.key.1.as_str(), job.format);
                        let built = Built {
                            key: job.key,
                            source: job.source,
                            result,
                            us: started.elapsed().as_micros() as u64,
                        };
                        if tx.send(built).is_err() {
                            break;
                        }
                    }
                })
                .ok()?;
            self.worker = Some((jobs, std::sync::Mutex::new(done)));
        }
        self.worker.as_ref().map(|(jobs, _)| jobs)
    }

    /// Queues a build. False when it is already in flight or no worker runs.
    pub fn queue(
        &mut self,
        key: Key,
        source: pm_glsl::Key,
        prepared: Arc<Prepared>,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> bool {
        if self.in_flight.get(&key) == Some(&source) {
            return false;
        }
        let Some(jobs) = self.worker() else {
            return false;
        };
        let sent = jobs.send(Job {
            key: key.clone(),
            source,
            prepared,
            device: device.clone(),
            format,
        });
        if sent {
            self.in_flight.insert(key, source);
        }
        sent
    }

    pub fn promote(&self, key: &Key, source: pm_glsl::Key) {
        if let Some((jobs, _)) = &self.worker {
            jobs.promote(|job| &job.key == key && job.source == source);
        }
    }

    pub fn in_flight(&self, key: &Key) -> bool {
        self.in_flight.contains_key(key)
    }

    /// Takes `key`'s build back if the worker has not started it (true: the
    /// caller builds it); false when it is being built or was never queued.
    pub fn reclaim(&mut self, key: &Key) -> bool {
        let Some(source) = self.in_flight.get(key).copied() else {
            return false;
        };
        if self.worker.as_ref().is_some_and(|(jobs, _)| {
            jobs.remove(|job| &job.key == key && job.source == source)
                .is_some()
        }) {
            self.in_flight.remove(key);
            return true;
        }
        false
    }

    pub fn pending(&self) -> usize {
        self.in_flight.len()
    }

    /// The builds that finished, without waiting.
    pub fn take(&mut self) -> Vec<Built> {
        let mut out = Vec::new();
        if let Some((_, done)) = &self.worker {
            let done = done.lock().unwrap_or_else(|e| e.into_inner());
            while let Ok(built) = done.try_recv() {
                out.push(built);
            }
        }
        self.settle(&out);
        out
    }

    /// Waits for `key`'s build (and returns it with every other build that
    /// finished meanwhile). Returns what finished when the worker is gone.
    pub fn wait_for(&mut self, key: &Key) -> Vec<Built> {
        let started = Instant::now();
        let mut out = Vec::new();
        if let Some((_, done)) = &self.worker {
            let done = done.lock().unwrap_or_else(|e| e.into_inner());
            while self.in_flight.contains_key(key) {
                match done.recv() {
                    Ok(built) => {
                        if self.in_flight.get(&built.key) == Some(&built.source) {
                            self.in_flight.remove(&built.key);
                        }
                        out.push(built);
                    }
                    Err(_) => {
                        self.in_flight.clear();
                        break;
                    }
                }
            }
        }
        self.in_flight.remove(key);
        self.waited += 1;
        self.waited_us += started.elapsed().as_micros() as u64;
        self.settle(&out);
        out
    }

    fn settle(&mut self, built: &[Built]) {
        for b in built {
            if self.in_flight.get(&b.key) == Some(&b.source) {
                self.in_flight.remove(&b.key);
            }
        }
        self.built += built.len() as u64;
        self.built_us += built.iter().map(|b| b.us).sum::<u64>();
    }
}

/// The definitions a document's layers name: effects (`fx[].type`),
/// transitions (`transitionIn` and `transitionOut`) and extension layers
/// (`d.definition`), in the root composition and every `comps` entry, in
/// approximate timeline order without repeats. A hint for what to build first; what a
/// frame draws is decided by the plan.
pub(crate) fn document_definitions<'a>(
    layers: impl IntoIterator<Item = &'a pm_model::Node>,
    effects: &super::Effects,
) -> Vec<Key> {
    use pm_eval::node;
    let mut out = Vec::new();
    let mut saved = SavedEffects::default();
    for layer in layers {
        for fx in node::array(layer.get("fx")).unwrap_or_default() {
            if !matches!(
                fx.get("on").map(pm_model::Node::kind),
                Some(pm_model::NodeKind::Bool(false))
            ) {
                saved.push(effects, fx, &mut out);
            }
        }
        push_other_programs(layer, &mut out);
    }
    out
}

/// Resolve each authored revision once per scan. Program ids are also the
/// ids the foreground plan uses for legacy revisions and extension overrides.
#[derive(Default)]
pub(crate) struct SavedEffects<'a>(HashSet<(&'a str, Option<u64>)>);

impl<'a> SavedEffects<'a> {
    pub(crate) fn push(
        &mut self,
        effects: &super::Effects,
        fx: &'a pm_model::Node,
        out: &mut Vec<Key>,
    ) {
        use pm_eval::node;
        let Some(id) = node::string(fx.get("type")).and_then(|s| s.as_str()) else {
            return;
        };
        let version = fx.get("version");
        let number = if node::is_nullish(version) {
            None
        } else if let Some(n) = node::number(version) {
            Some(n.to_bits())
        } else {
            return;
        };
        if self.0.insert((id, number))
            && let Ok(key) = effects.saved_key(id, version)
        {
            push_program(out, DefinitionKind::Effect, &key);
        }
    }
}

pub(crate) fn push_other_programs(layer: &pm_model::Node, out: &mut Vec<Key>) {
    use pm_eval::node;
    let mut push = |kind, node| {
        if let Some(id) = node::string(node).and_then(|id| id.as_str()) {
            push_program(out, kind, id);
        }
    };
    for edge in ["transitionIn", "transitionOut"] {
        push(
            DefinitionKind::Transition,
            node::member(layer.get(edge), "type"),
        );
    }
    if node::is_str(layer.get("type"), "extension") {
        push(
            DefinitionKind::Layer,
            node::member(layer.get("d"), "definition"),
        );
    }
}

fn push_program(out: &mut Vec<Key>, kind: DefinitionKind, id: &str) {
    let key = (kind, DefinitionId::from(id));
    if !id.is_empty() && !out.contains(&key) {
        out.push(key);
    }
}


#[cfg(test)]
mod tests {
    use super::JobQueue;

    #[test]
    fn a_pending_cut_can_promote_and_reclaim_its_job() {
        let (queue, ready, pending) = JobQueue::new();
        for job in 0..4 {
            assert!(queue.send(job));
        }
        queue.promote(|job| *job == 3);
        queue.promote(|job| *job == 3);
        assert_eq!(queue.remove(|job| *job == 2), Some(2));
        for _ in 0..4 {
            ready.recv().unwrap();
        }
        assert_eq!(pending.lock().unwrap().drain(..).collect::<Vec<_>>(), [3, 0, 1]);
    }
}
