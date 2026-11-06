//! The viewer's scope tap. Analysis records on the render worker, at most ten
//! times a second; one other thread waits for the GPU, so neither the render
//! worker nor the main thread ever does.
use async_channel::Sender;
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

/// What a tap reports: counts, or why the frame can't be read.
pub type ScopeResult = Result<ScopeFrame, String>;

struct Job {
    readback: ScopeReadback,
    frames: Sender<ScopeResult>,
}

/// Lives with the engine; the pass and its worker start on first use.
pub(crate) struct Tap {
    pass: ScopePass,
    jobs: SyncSender<Job>,
    busy: Arc<AtomicBool>,
    last: Option<Instant>,
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
        }
    }

    /// Analyzes `texture` when a result is due and none is in flight.
    pub(crate) fn frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        config: ScopeConfig,
        frames: &Sender<ScopeResult>,
    ) {
        let now = Instant::now();
        if self.busy.load(Ordering::Acquire) || self.last.is_some_and(|last| now - last < INTERVAL)
        {
            return;
        }
        self.last = Some(now);
        let readback = self.pass.analyze(device, queue, texture, config);
        self.busy.store(true, Ordering::Release);
        let job = Job {
            readback,
            frames: frames.clone(),
        };
        if self.jobs.try_send(job).is_err() {
            self.busy.store(false, Ordering::Release);
        }
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
        let _ = job.frames.try_send(result);
    }
}
