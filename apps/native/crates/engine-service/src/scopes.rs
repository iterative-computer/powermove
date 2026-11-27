//! The viewer's scope tap. Analysis records on the render worker, at most ten
//! times a second; one other thread waits for the GPU, so neither the render
//! worker nor the main thread ever does. The latest frame always gets its turn:
//! one that arrives too soon waits, so a settled viewer's plot shows its picture.
use async_channel::Sender;
use pm_gpu::bridge_out::surface::ExportedSurface;
use pm_render::scopes::{ScopeConfig, ScopeFrame, ScopePass, ScopeReadback};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, sync_channel},
    },
    time::{Duration, Instant},
};

const INTERVAL: Duration = Duration::from_millis(100);
/// How often a waiting frame looks again while the last analysis is in flight.
const BUSY_RETRY: Duration = Duration::from_millis(10);

/// What a tap reports: counts, or why the frame can't be read.
pub type ScopeResult = Result<ScopeFrame, String>;

struct Job {
    readback: ScopeReadback,
    frames: Sender<ScopeResult>,
}

/// Holds the frame until its analysis is recorded, so the pool can't reuse it.
struct Waiting {
    surface: Arc<ExportedSurface>,
    config: ScopeConfig,
    frames: Sender<ScopeResult>,
}

/// Lives with the engine; the pass and its worker start on first use.
pub(crate) struct Tap {
    pass: ScopePass,
    jobs: SyncSender<Job>,
    busy: Arc<AtomicBool>,
    last: Option<Instant>,
    waiting: Option<Waiting>,
}

impl Tap {
    pub(crate) fn new(device: wgpu::Device) -> Self {
        let (jobs, inbox) = sync_channel(1);
        let busy = Arc::new(AtomicBool::new(false));
        let pass = ScopePass::new(&device);
        let finished = busy.clone();
        std::thread::Builder::new()
            .name("pm-scopes".into())
            .spawn(move || worker(device, inbox, finished))
            .expect("the scope worker starts");
        Self {
            pass,
            jobs,
            busy,
            last: None,
            waiting: None,
        }
    }

    /// Takes `surface` as the latest frame and analyzes it if a result is due.
    pub(crate) fn frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface: &Arc<ExportedSurface>,
        config: ScopeConfig,
        frames: &Sender<ScopeResult>,
    ) {
        self.waiting = Some(Waiting {
            surface: surface.clone(),
            config,
            frames: frames.clone(),
        });
        self.flush(device, queue);
    }

    /// Analyzes the waiting frame if it's due, or says when to ask again.
    pub(crate) fn flush(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Duration> {
        self.waiting.as_ref()?;
        let now = Instant::now();
        let wait = self
            .last
            .map_or(Duration::ZERO, |last| INTERVAL.saturating_sub(now - last));
        if self.busy.load(Ordering::Acquire) {
            return Some(wait.max(BUSY_RETRY));
        }
        if !wait.is_zero() {
            return Some(wait);
        }
        let waiting = self.waiting.take()?;
        self.last = Some(now);
        let readback = self
            .pass
            .analyze(device, queue, &waiting.surface.texture, waiting.config);
        self.busy.store(true, Ordering::Release);
        let job = Job {
            readback,
            frames: waiting.frames,
        };
        if self.jobs.try_send(job).is_err() {
            self.busy.store(false, Ordering::Release);
        }
        None
    }

    /// Lets go of a frame the current tap doesn't want.
    pub(crate) fn retain(&mut self, tap: Option<&(ScopeConfig, Sender<ScopeResult>)>) {
        self.waiting.take_if(|waiting| {
            tap.is_none_or(|(config, frames)| {
                waiting.config != *config || !waiting.frames.same_channel(frames)
            })
        });
    }

    /// The next frame analyzes at once (a new tap, a changed picture).
    pub(crate) fn reset(&mut self) {
        self.last = None;
    }
}

fn worker(device: wgpu::Device, inbox: Receiver<Job>, busy: Arc<AtomicBool>) {
    for job in inbox {
        let result = job.readback.wait(&device);
        busy.store(false, Ordering::Release);
        // The newest result replaces one the panel hasn't taken yet.
        let _ = job.frames.force_send(result);
    }
}
