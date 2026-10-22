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

use std::collections::HashMap;
use std::sync::{Arc, mpsc};
use std::time::Instant;

use pm_glsl::{Failure, Prepared};
use pm_protocol::common::DefinitionId;
use pm_protocol::definitions::DefinitionKind;

use super::program::GpuProgram;

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
    worker: Option<(mpsc::Sender<Job>, std::sync::Mutex<mpsc::Receiver<Built>>)>,
    in_flight: HashMap<Key, pm_glsl::Key>,
    /// Queued builds and whether the worker started them. A frame that
    /// needs a program still queued takes it back (builds it itself rather
    /// than wait behind other builds); one already started is waited for.
    started: Arc<std::sync::Mutex<HashMap<Key, bool>>>,
    /// Builds finished off the frame thread, and their time (µs).
    pub built: u64,
    pub built_us: u64,
    /// Frames that waited for a build in flight, and how long (µs).
    pub waited: u64,
    pub waited_us: u64,
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
    fn worker(&mut self) -> Option<&mpsc::Sender<Job>> {
        if self.worker.is_none() {
            let (jobs, rx) = mpsc::channel::<Job>();
            let (tx, done) = mpsc::channel::<Built>();
            let claims = self.started.clone();
            std::thread::Builder::new()
                .name("pm-program-prewarm".into())
                .spawn(move || {
                    // Ends when the renderer drops its sender.
                    while let Ok(job) = rx.recv() {
                        {
                            let mut claims = claims.lock().unwrap_or_else(|e| e.into_inner());
                            match claims.get_mut(&job.key) {
                                Some(started) => *started = true,
                                // A frame took it back.
                                None => continue,
                            }
                        }
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
        let sent = jobs
            .send(Job {
                key: key.clone(),
                source,
                prepared,
                device: device.clone(),
                format,
            })
            .is_ok();
        if sent {
            self.started
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(key.clone(), false);
            self.in_flight.insert(key, source);
        }
        sent
    }

    pub fn in_flight(&self, key: &Key) -> bool {
        self.in_flight.contains_key(key)
    }

    /// Takes `key`'s build back if the worker has not started it (true: the
    /// caller builds it); false when it is being built or was never queued.
    pub fn reclaim(&mut self, key: &Key) -> bool {
        let mut claims = self.started.lock().unwrap_or_else(|e| e.into_inner());
        if claims.get(key) == Some(&false) {
            claims.remove(key);
            drop(claims);
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
                        self.in_flight.remove(&built.key);
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
        let mut claims = self.started.lock().unwrap_or_else(|e| e.into_inner());
        for b in built {
            self.in_flight.remove(&b.key);
            claims.remove(&b.key);
        }
        drop(claims);
        self.built += built.len() as u64;
        self.built_us += built.iter().map(|b| b.us).sum::<u64>();
    }
}

/// The definitions a document's layers name: effects (`fx[].type`),
/// transitions (`transitionIn` and `transitionOut`) and extension layers
/// (`d.definition`), in the root composition and every `comps` entry, in
/// document order without repeats. A hint for what to build first; what a
/// frame draws is decided by the plan.
pub(crate) fn document_definitions(doc: &pm_model::Document) -> Vec<Key> {
    use pm_eval::node;
    let root = doc.root();
    let mut lists = vec![node::member(root, "layers")];
    if let Some(comps) = node::member(root, "comps") {
        lists.extend(
            node::entries(comps)
                .into_iter()
                .map(|(_, comp)| node::member(comp.node(), "layers")),
        );
    }
    let mut out: Vec<Key> = Vec::new();
    let mut push = |kind: DefinitionKind, id: Option<&pm_model::Node>| {
        let Some(id) = node::string(id).map(|s| s.to_string_lossy()) else {
            return;
        };
        if id.is_empty() {
            return;
        }
        let key = (kind, DefinitionId::from(id.as_str()));
        if !out.contains(&key) {
            out.push(key);
        }
    };
    for layer in lists.into_iter().filter_map(node::array).flatten() {
        for fx in node::array(node::member(Some(layer), "fx")).unwrap_or_default() {
            push(DefinitionKind::Effect, node::member(Some(fx), "type"));
        }
        for edge in ["transitionIn", "transitionOut"] {
            push(
                DefinitionKind::Transition,
                node::member(node::member(Some(layer), edge), "type"),
            );
        }
        if node::is_str(node::member(Some(layer), "type"), "extension") {
            push(
                DefinitionKind::Layer,
                node::member(node::member(Some(layer), "d"), "definition"),
            );
        }
    }
    out
}
