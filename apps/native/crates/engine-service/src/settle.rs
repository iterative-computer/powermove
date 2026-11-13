//! A paused viewer's settled picture, drawn off the interactive worker.
//!
//! The settled picture (`FrameSpec::final_picture`) is one whole final
//! frame: originals decoded, the whole composition planned, every effect
//! over the whole frame. Drawn on the viewer's worker it held every seek,
//! Play and close behind it. It is drawn here instead, by a renderer of its
//! own on its own thread, on the viewer's device and queue:
//!
//! - decoding, planning and encoding never hold the worker;
//! - its GPU work is submitted in time slices (`pm_render::slices`), one on
//!   the GPU at a time, each admitted by a gate: a picture someone waits
//!   for goes first, and a playing viewer's next frame is left its room
//!   ([`Traffic`]): a slice that would still run when that frame is
//!   submitted waits for it. Each wait lasts [`policy::SETTLE_HOLD_SECONDS`]
//!   at most, so another viewer playing never keeps a paused one from
//!   settling. What this does NOT give is a bound on the delay a playing
//!   viewer can meet: a slice point is a chance to give way, a unit of
//!   work (one layer, one effect pass) is never split, and one that takes
//!   longer than the room between two playing frames, or that its wait
//!   bound lets go, delays the next playing frame by what it takes;
//! - it gives way as soon as its viewer asks for anything newer, at the
//!   next layer;
//! - its picture is handed to the worker, which shows it only for the
//!   stamp it was asked for. The preview stays up until then.
//!
//! Nothing waits for this thread: dropping the [`Settler`] ends it after
//! the frame it is in. A panic while it draws is caught: the picture it was
//! drawing fails with the panic's message, the thread ends (its renderer is
//! in no known state), and the engine makes a settler again for the next
//! picture ([`Settler::dead`]).

use crate::render::{FrameSpec, Scene};
use anyhow::Result;
use pm_gpu::bridge_out::surface::ExportedSurface;
use pm_render::slices::{Gate, Slices};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What the interactive worker has to draw, as a slice's gate sees it.
#[derive(Default)]
pub(crate) struct Traffic {
    /// A picture is wanted and not submitted yet (a viewer to draw, a
    /// frame being encoded).
    wanted: AtomicBool,
    /// When a playing viewer's next frame is encoded (host seconds as
    /// bits; 0: none plays).
    next_frame: AtomicU64,
}
impl Traffic {
    pub(crate) fn set(&self, wanted: bool, next_frame: Option<f64>) {
        self.wanted.store(wanted, Ordering::Release);
        self.next_frame
            .store(next_frame.unwrap_or(0.).to_bits(), Ordering::Release);
    }
    /// Whether a submission taking `expected` should wait: a picture is
    /// wanted now, or a playing frame is encoded before it would end.
    fn holds(&self, expected: Duration) -> bool {
        if self.wanted.load(Ordering::Acquire) {
            return true;
        }
        let next = f64::from_bits(self.next_frame.load(Ordering::Acquire));
        let now = crate::trace::now();
        // A time long past is a worker that has not looked since: no claim.
        next > 0. && now + expected.as_secs_f64() > next && now < next + 0.05
    }
}

/// The gate of one settled picture's slices: `stop` ends it (its viewer
/// asked for something newer); otherwise a submission waits while the
/// worker's pictures hold it, `hold` at most.
pub(crate) fn gate(
    traffic: Arc<Traffic>,
    hold: Duration,
    stop: impl Fn() -> bool + Send + Sync + 'static,
    pressing: impl Fn() -> bool + Send + Sync + 'static,
) -> Gate {
    Arc::new(move |expected: Duration| {
        if stop() {
            return true;
        }
        if expected.is_zero() {
            return false;
        }
        let asked = Instant::now();
        while (traffic.holds(expected) || pressing()) && asked.elapsed() < hold {
            std::thread::sleep(Duration::from_micros(500));
            if stop() {
                return true;
            }
        }
        false
    })
}

/// A scene of this content makes the settler panic (tests).
#[cfg(test)]
pub(crate) const PANICS: u64 = u64::MAX - 7;

pub(crate) struct Job {
    pub key: crate::policy::CacheKey,
    pub scene: Scene,
    pub spec: FrameSpec,
    pub request: pm_render::RenderRequest,
    /// Where it is drawn: the viewer engine's surface, on the shared device.
    pub surface: Arc<ExportedSurface>,
    pub resources: Arc<crate::resources::ResourceSnapshot>,
    pub compat: Option<Arc<crate::compat::CompatStore>>,
    pub fonts: Option<pm_render::text::host::RenderFonts>,
    pub gate: Gate,
}

pub(crate) enum Outcome {
    /// The surface holds the picture. `exact`: no stand-in, no font failure.
    Drawn {
        failures: Vec<pm_render::LayerFailure>,
        exact: bool,
    },
    /// Its viewer asked for something newer first.
    GaveWay,
    /// Raster override answers it needs are not there yet.
    Pending(Vec<pm_render::compat::CompatNeed>),
    Failed(anyhow::Error),
}

/// When a settled picture's phases happened (host seconds), for the trace.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Timing {
    pub sent: f64,
    pub began: f64,
    /// Its plan was built: decoding and encoding go on from here.
    pub planned: Option<f64>,
    pub first_submission: Option<f64>,
    pub ended: f64,
    pub slices: u32,
    pub worst_slice_ms: f64,
    pub worst_unit_ms: f64,
    pub held_ms: f64,
}

pub(crate) struct Settled {
    pub key: crate::policy::CacheKey,
    pub scene: Scene,
    pub surface: Arc<ExportedSurface>,
    pub outcome: Outcome,
    /// The font registry generation it was drawn with.
    pub fonts: Option<u64>,
    /// Every media file version the frame looked at.
    pub reads: Vec<(
        std::path::PathBuf,
        Option<pm_text::file_version::FileVersion>,
    )>,
    pub timing: Timing,
}

/// The bytes the settling renderer holds between frames.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Held {
    pub target_pool: u64,
    pub images: u64,
    pub converted: u64,
    pub text: u64,
    pub vector: u64,
    pub pyramids: u64,
    /// Its own decode service's frames and decoders.
    pub frames: u64,
    pub decoders: u64,
    /// 3D meshes and maps resident for it.
    pub geometry: u64,
    /// The share of the engine's grant its kept caches are held to
    /// ([`Settler::budget`]).
    pub budget: u64,
    /// The most it held at the end of a picture's drawing, before that
    /// picture's originals and targets are let go: its peak working set as
    /// the caches count it (the surface it draws into is the engine's).
    pub peak: u64,
}
impl Held {
    pub(crate) fn total(&self) -> u64 {
        self.target_pool
            + self.images
            + self.converted
            + self.text
            + self.vector
            + self.pyramids
            + self.frames
            + self.decoders
            + self.geometry
    }
}

enum Message {
    Draw(Box<Job>, f64),
    /// A document closed: nothing of it is kept.
    Forget,
    /// The engine's grant moved: the kept caches are held to this many
    /// bytes from now on.
    Budget(u64),
    /// Memory is wanted back: whatever is kept between pictures goes.
    Release,
}

struct Shared {
    done: Mutex<Option<Settled>>,
    held: Mutex<Held>,
    busy: AtomicBool,
    /// The thread ended (a panic): nothing more is drawn here.
    dead: AtomicBool,
    /// Memory is wanted back: the picture being drawn gives way.
    abort: Arc<AtomicBool>,
    /// The picture sent and not answered yet: what fails if the thread
    /// ends before it answers.
    inflight: Mutex<Option<(crate::policy::CacheKey, Scene, Arc<ExportedSurface>, f64)>>,
}
impl Shared {
    /// The thread is ending: the picture it owes fails with `why`.
    fn end(&self, why: &str, waker: &Waker) {
        if let Some((key, scene, surface, sent)) = self.inflight.lock().unwrap().take() {
            let at = crate::trace::now();
            *self.done.lock().unwrap() = Some(Settled {
                key,
                scene,
                surface,
                outcome: Outcome::Failed(anyhow::anyhow!(
                    "Drawing a settled picture panicked: {why}"
                )),
                fonts: None,
                reads: Vec::new(),
                timing: Timing {
                    sent,
                    began: at,
                    ended: at,
                    ..Default::default()
                },
            });
        }
        *self.held.lock().unwrap() = Held::default();
        self.dead.store(true, Ordering::Release);
        if let Some(wake) = &*waker.lock().unwrap() {
            wake.try_send(()).ok();
        }
    }
}
fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_default()
}

pub(crate) struct Settler {
    jobs: std::sync::mpsc::Sender<Message>,
    shared: Arc<Shared>,
}

type Waker = Arc<Mutex<Option<std::sync::mpsc::SyncSender<()>>>>;

impl Settler {
    /// `effects`: a definition set sharing the viewer renderer's translated
    /// programs (`pm_render::Effects::sibling`), so nothing the viewer
    /// translated is translated again here.
    pub(crate) fn spawn(
        gpu: &pm_gpu::Gpu,
        effects: pm_render::Effects,
        waker: Waker,
        budget: u64,
    ) -> Result<Self> {
        let (jobs, inbox) = std::sync::mpsc::channel();
        let shared = Arc::new(Shared {
            done: Mutex::new(None),
            held: Mutex::default(),
            busy: AtomicBool::new(false),
            dead: AtomicBool::new(false),
            abort: Arc::default(),
            inflight: Mutex::default(),
        });
        let (device, queue, worker) = (gpu.device.clone(), gpu.queue.clone(), shared.clone());
        std::thread::Builder::new()
            .name("pm-engine-settle".into())
            .spawn(move || {
                let (ended, waking) = (worker.clone(), waker.clone());
                // A panic anywhere on this thread (a picture, making the
                // renderer, letting it go): the picture owed fails, and
                // the thread is over.
                if let Err(panic) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(device, queue, effects, inbox, worker, waker, budget)
                })) {
                    let why = panic_message(&*panic);
                    eprintln!("a settled picture panicked: {why}");
                    ended.end(&why, &waking);
                }
            })?;
        Ok(Self { jobs, shared })
    }
    /// The thread ended after a panic: a settler is made again.
    pub(crate) fn dead(&self) -> bool {
        self.shared.dead.load(Ordering::Acquire)
    }
    /// Whether a picture is being drawn or waits to be taken.
    pub(crate) fn busy(&self) -> bool {
        self.shared.busy.load(Ordering::Acquire)
    }
    pub(crate) fn draw(&self, job: Job) {
        self.shared.busy.store(true, Ordering::Release);
        *self.shared.inflight.lock().unwrap() = Some((
            job.key,
            job.scene.clone(),
            job.surface.clone(),
            crate::trace::now(),
        ));
        if self
            .jobs
            .send(Message::Draw(Box::new(job), crate::trace::now()))
            .is_err()
        {
            self.shared.busy.store(false, Ordering::Release);
        }
    }
    pub(crate) fn take(&self) -> Option<Settled> {
        let done = self.shared.done.lock().unwrap().take();
        if done.is_some() {
            self.shared.busy.store(false, Ordering::Release);
        }
        done
    }
    pub(crate) fn forget(&self) {
        let _ = self.jobs.send(Message::Forget);
    }
    /// Its kept caches are held to `bytes` of the engine's grant from its
    /// next turn on.
    pub(crate) fn budget(&self, bytes: u64) {
        let _ = self.jobs.send(Message::Budget(bytes));
    }
    /// Memory is wanted back: the picture being drawn gives way at its
    /// next slice, and what is kept between pictures goes.
    pub(crate) fn release(&self) {
        self.shared.abort.store(true, Ordering::Release);
        let _ = self.jobs.send(Message::Release);
    }
    pub(crate) fn held(&self) -> Held {
        *self.shared.held.lock().unwrap()
    }
}

/// What the renderer keeps of a frame's work between settled pictures:
/// its programs and text, never the stills or targets of one frame; its
/// kept caches within `budget` bytes, its share of the engine's grant.
fn configure(renderer: &mut pm_render::Renderer, budget: u64) {
    let part = |percent: u64| budget * percent / 100;
    renderer.apply_media_lease(pm_media::decode::service::MediaLease {
        frame_cache: part(25) as usize,
        decoders: part(25) as usize,
    });
    renderer.set_image_cache_budget(part(20));
    renderer.set_converted_frame_budget(part(10));
    renderer.set_target_pool_budget(part(10));
    renderer.set_vector_cache_budget(part(5), 512);
    renderer.trim(part(5));
    // Each settled picture is of another time: nothing to reuse between
    // them, and no kept draw to answer for when a frame gives way.
    renderer.set_draw_reuse(false);
    renderer.set_preview_media(false, false);
    renderer.set_isolate_layers(true);
}

fn run(
    device: wgpu::Device,
    queue: wgpu::Queue,
    mut effects: pm_render::Effects,
    inbox: std::sync::mpsc::Receiver<Message>,
    shared: Arc<Shared>,
    waker: Waker,
    mut budget: u64,
) {
    // Below the worker's frames, which a person watches land; above jobs.
    // SAFETY: affects only the calling thread.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INITIATED, 0);
    }
    let mut renderer = pm_render::Renderer::new(device, queue);
    // A document's effects (the built-in ones with them) are installed
    // with its first settled picture.
    renderer.swap_effects(&mut effects);
    drop(effects);
    configure(&mut renderer, budget);
    renderer.set_slices(Some(Slices::new(
        Duration::from_secs_f64(crate::policy::SETTLE_SLICE_SECONDS),
        Arc::new(|_| false),
    )));
    let mut document = None;
    let mut fonts = None;
    let mut idle = false;
    loop {
        let message = match inbox.recv_timeout(Duration::from_secs(2)) {
            Ok(message) => message,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if !std::mem::replace(&mut idle, true) {
                    objc2::rc::autoreleasepool(|_| {
                        renderer.release_idle_targets();
                        renderer.release_idle_media();
                        renderer.trim(0);
                    });
                    let now = held(&renderer, budget, &shared);
                    *shared.held.lock().unwrap() = now;
                }
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        };
        idle = false;
        match message {
            Message::Forget => {
                objc2::rc::autoreleasepool(|_| renderer.release_project_memory());
                configure(&mut renderer, budget);
                (document, fonts) = (None, None);
            }
            Message::Budget(bytes) => {
                budget = bytes;
                configure(&mut renderer, budget);
            }
            Message::Release => {
                objc2::rc::autoreleasepool(|_| {
                    renderer.release_idle_targets();
                    renderer.release_idle_media();
                    renderer.trim(0);
                });
            }
            Message::Draw(job, sent) => {
                // A release asked before this picture is not about it.
                shared.abort.store(false, Ordering::Release);
                let drawn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    objc2::rc::autoreleasepool(|_| {
                        draw(
                            &mut renderer,
                            &mut document,
                            &mut fonts,
                            *job,
                            sent,
                            &shared,
                        )
                    })
                }));
                let settled = match drawn {
                    Ok(settled) => settled,
                    Err(panic) => {
                        let why = panic_message(&*panic);
                        eprintln!("a settled picture panicked: {why}");
                        shared.end(&why, &waker);
                        // Its renderer is let go, in no known state.
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            drop(renderer)
                        }));
                        return;
                    }
                };
                // What it holds is known before its picture is taken.
                let now = held(&renderer, budget, &shared);
                *shared.held.lock().unwrap() = now;
                *shared.inflight.lock().unwrap() = None;
                *shared.done.lock().unwrap() = Some(settled);
                if let Some(wake) = &*waker.lock().unwrap() {
                    wake.try_send(()).ok();
                }
            }
        }
        let now = held(&renderer, budget, &shared);
        *shared.held.lock().unwrap() = now;
    }
}

fn held(renderer: &pm_render::Renderer, budget: u64, shared: &Shared) -> Held {
    let media = renderer.media_usage();
    let resident = renderer.scene3d_resident();
    let peak = shared.held.lock().unwrap().peak;
    Held {
        target_pool: renderer.target_pool_bytes(),
        images: renderer.image_cache_bytes(),
        converted: renderer.converted_frame_bytes(),
        text: renderer.text_cache_bytes(),
        vector: renderer.vector_cache_bytes(),
        pyramids: renderer.pyramid_bytes(),
        frames: media.frame_cache as u64,
        decoders: media.decoders as u64,
        geometry: resident.mesh_bytes + resident.map_bytes,
        budget,
        peak,
    }
}

/// The document state whose effects the renderer has installed.
type Installed = (usize, u64, usize);

fn draw(
    renderer: &mut pm_render::Renderer,
    document: &mut Option<Installed>,
    fonts: &mut Option<u64>,
    job: Job,
    sent: f64,
    shared: &Shared,
) -> Settled {
    let mut timing = Timing {
        sent,
        began: crate::trace::now(),
        ..Default::default()
    };
    let mut reads = Vec::new();
    #[cfg(test)]
    if job.scene.content == PANICS {
        panic!("a test's settled picture");
    }
    let outcome = (|| -> Result<Outcome> {
        if (job.gate)(Duration::ZERO) {
            return Ok(Outcome::GaveWay);
        }
        let scene = &job.scene;
        let state = (
            Arc::as_ptr(&scene.doc) as usize,
            scene.content,
            Arc::as_ptr(&job.resources) as usize,
        );
        if *document != Some(state) {
            let busy = |e: &str| e.contains("compiler-busy");
            let failures = renderer
                .set_document_effects(&scene.doc, &job.resources.definitions)
                .map_err(|e| {
                    if busy(&e) {
                        anyhow::Error::new(crate::render::CompilerBusy)
                    } else {
                        anyhow::Error::msg(e)
                    }
                })?;
            if failures.iter().any(|failure| busy(&failure.message)) {
                return Err(anyhow::Error::new(crate::render::CompilerBusy));
            }
            #[cfg(feature = "kernel")]
            crate::studio::attach(renderer, scene.doc.clone(), scene.media.as_ref());
            renderer.prewarm(&scene.doc);
            renderer.prewarm_programs(&scene.doc);
            *document = Some(state);
        }
        if let Some(host) = &job.fonts {
            host.sync(renderer, fonts);
        }
        renderer.set_compat_rasters(job.compat.clone().map(|store| {
            Arc::new(crate::compat::ExactAnswers(store))
                as Arc<dyn pm_render::compat::CompatRasters>
        }));
        let slices = renderer
            .slices_mut()
            .expect("set when the renderer was made");
        // Memory wanted back stops it as its viewer would.
        let (gate, abort) = (job.gate.clone(), shared.abort.clone());
        slices.set_gate(Arc::new(move |expected| {
            abort.load(Ordering::Acquire) || gate(expected)
        }));
        slices.reset_report();
        pm_render::media::note_reads();
        let rendered = renderer.render_into(
            &scene.doc,
            &scene.comp,
            job.spec.time,
            &job.request,
            scene.media.as_ref(),
            &pm_gpu::Target::wrap(job.surface.texture.clone()),
            None,
        );
        reads = pm_render::media::take_reads();
        // Its working set at its fullest: every original and target of the
        // picture still held.
        let working = held(renderer, 0, shared).total();
        {
            let mut kept = shared.held.lock().unwrap();
            kept.peak = kept.peak.max(working);
        }
        let report = renderer.slices_mut().expect("set above").report();
        let host = |at: Instant| crate::trace::now() - at.elapsed().as_secs_f64();
        timing.planned = report.planned.map(host);
        timing.first_submission = report.first_submission.map(host);
        timing.slices = report.slices;
        timing.worst_slice_ms = report.worst.as_secs_f64() * 1e3;
        timing.worst_unit_ms = report.worst_unit.as_secs_f64() * 1e3;
        timing.held_ms = report.held.as_secs_f64() * 1e3;
        // Nothing of one frame is kept for the next: the originals a final
        // picture reads cost gigabytes on large images.
        renderer.release_whole_stills();
        renderer.trim_submitted_frame_caches();
        renderer.release_idle_targets();
        let font_failure = job.fonts.as_ref().and_then(|host| host.after_render());
        Ok(match rendered {
            Ok(rendered) => {
                // Its slices have all run: the hang guard judges them.
                renderer.watchdog_reports(false);
                Outcome::Drawn {
                    exact: rendered.stand_ins.is_empty() && font_failure.is_none(),
                    failures: rendered.failures,
                }
            }
            Err(pm_render::RenderError::Abandoned) => Outcome::GaveWay,
            Err(pm_render::RenderError::CompatPending(needs)) => Outcome::Pending(needs),
            Err(error) => Outcome::Failed(error.into()),
        })
    })()
    .unwrap_or_else(Outcome::Failed);
    timing.ended = crate::trace::now();
    Settled {
        key: job.key,
        scene: job.scene,
        surface: job.surface,
        outcome,
        fonts: *fonts,
        reads,
        timing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate_of(traffic: &Arc<Traffic>, stop: bool, pressing: bool) -> Gate {
        gate(
            traffic.clone(),
            Duration::from_millis(40),
            move || stop,
            move || pressing,
        )
    }

    /// A slice goes at once on a quiet GPU, waits for a picture someone
    /// wants (or a command not taken yet) but never past its bound, and a
    /// stopped picture never waits.
    #[test]
    fn a_slice_waits_for_wanted_pictures_within_its_bound() {
        let slice = Duration::from_millis(6);
        let traffic = Arc::new(Traffic::default());
        let took = |gate: &Gate| {
            let asked = Instant::now();
            (gate(slice), asked.elapsed())
        };
        let (stop, waited) = took(&gate_of(&traffic, false, false));
        assert!(!stop && waited < Duration::from_millis(20), "{waited:?}");
        // A picture is wanted: held for the bound, then admitted.
        traffic.set(true, None);
        let (stop, waited) = took(&gate_of(&traffic, false, false));
        assert!(!stop && waited >= Duration::from_millis(40), "{waited:?}");
        // It is submitted meanwhile: the slice goes right after.
        let worker = traffic.clone();
        let submit = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(5));
            worker.set(false, None);
        });
        let (stop, waited) = took(&gate_of(&traffic, false, false));
        submit.join().unwrap();
        assert!(!stop && waited < Duration::from_millis(40), "{waited:?}");
        // A command the worker has not taken yet holds it the same way.
        let (stop, waited) = took(&gate_of(&traffic, false, true));
        assert!(!stop && waited >= Duration::from_millis(40), "{waited:?}");
        // Its own viewer asked for something newer: it stops, unheld.
        traffic.set(true, None);
        let (stop, waited) = took(&gate_of(&traffic, true, true));
        assert!(stop && waited < Duration::from_millis(20), "{waited:?}");
    }

    /// Beside a playing viewer a slice goes only where it ends before the
    /// next frame is encoded; a slice that would run into it waits for the
    /// frame, within its bound.
    #[test]
    fn a_slice_leaves_a_playing_viewers_next_frame_its_room() {
        let traffic = Arc::new(Traffic::default());
        let gate = gate_of(&traffic, false, false);
        let took = |slice: u64| {
            let asked = Instant::now();
            gate(Duration::from_millis(slice));
            asked.elapsed()
        };
        // The next frame is 30 ms away: a 6 ms slice fits before it.
        traffic.set(false, Some(crate::trace::now() + 0.030));
        assert!(took(6) < Duration::from_millis(15));
        // 3 ms away: the slice would still run when the frame is encoded.
        traffic.set(false, Some(crate::trace::now() + 0.003));
        assert!(took(6) >= Duration::from_millis(40));
        // A worker that has not looked for a while claims nothing.
        traffic.set(false, Some(crate::trace::now() - 1.));
        assert!(took(6) < Duration::from_millis(15));
    }
}
