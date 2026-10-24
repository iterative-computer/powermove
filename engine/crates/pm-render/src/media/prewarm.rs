//! Bounded, nonblocking source and decoder preparation for planned views.

use super::*;
use std::collections::{HashMap, HashSet};
use std::sync::Weak;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::Duration;

const MAX_PENDING: usize = 16;
const MAX_PREPARED_SESSIONS: usize = 8;
const MAX_ACTIVE_WORKERS: usize = 2;
static ACTIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);

struct WorkerSlot;

impl WorkerSlot {
    fn acquire() -> Option<Self> {
        ACTIVE_WORKERS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_ACTIVE_WORKERS).then_some(active + 1)
            })
            .ok()
            .map(|_| Self)
    }
}

impl Drop for WorkerSlot {
    fn drop(&mut self) {
        ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Hash, Eq, PartialEq)]
struct Key {
    path: PathBuf,
    instance: u64,
    export: bool,
    stamp: (u64, u128),
    settings: String,
}

struct Job {
    key: Key,
    stamp: (u64, u128),
    existing: Option<Arc<MediaSource>>,
    seconds: f64,
    interpret: Option<Interpretation>,
    composition: pm_time::Rate,
}

struct Completed {
    key: Key,
    stamp: (u64, u128),
    source: Option<Arc<MediaSource>>,
    prepared: Result<bool, MediaFailure>,
}

pub(super) struct BackgroundPrepare {
    sender: Option<SyncSender<Job>>,
    results: Receiver<Completed>,
    pending: HashSet<Key>,
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl BackgroundPrepare {
    pub(super) fn new(service: Arc<DecodeService>) -> Option<Self> {
        let (sender, jobs) = mpsc::sync_channel::<Job>(MAX_PENDING);
        let (finished, results) = mpsc::sync_channel::<Completed>(MAX_PENDING);
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let worker = std::thread::Builder::new()
            .name("pm-media-prepare".into())
            .spawn(move || {
                let mut sources: HashMap<(PathBuf, (u64, u128)), Weak<MediaSource>> =
                    HashMap::new();
                'work: loop {
                    let job = match jobs.recv_timeout(Duration::from_millis(500)) {
                        Ok(job) => job,
                        Err(
                            mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected,
                        ) => break,
                    };
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    // Idle renderers hold no global startup capacity.
                    let _slot = loop {
                        if stop.load(Ordering::Relaxed) {
                            break 'work;
                        }
                        if let Some(slot) = WorkerSlot::acquire() {
                            break slot;
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    };
                    let route = (job.key.path.clone(), job.stamp);
                    let existing = sources.get(&route).and_then(Weak::upgrade);
                    let completed = prepare_one(job, existing, &service, &stop, true);
                    if let Some(source) = &completed.source {
                        if sources.len() >= MAX_PENDING && !sources.contains_key(&route) {
                            sources.clear();
                        }
                        sources.insert(route, Arc::downgrade(source));
                    }
                    sources.retain(|_, source| source.strong_count() != 0);
                    // The owner may be rendering or closing. Never wait for it.
                    if finished.try_send(completed).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Self {
            sender: Some(sender),
            results,
            pending: HashSet::new(),
            cancel,
            worker: Some(worker),
        })
    }
}

impl Drop for BackgroundPrepare {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.sender.take();
        // Index building cannot be interrupted. A busy worker owns only its
        // service and channels and may finish after the renderer is gone.
        if let Some(worker) = self.worker.take()
            && worker.is_finished()
        {
            let _ = worker.join();
        }
    }
}

/// Opens a job's source and decodes its selected frame into the service's
/// cache. Background preparation is `bounded` by the prepared-session cap;
/// a frame about to draw the source is not.
fn prepare_one(
    job: Job,
    shared: Option<Arc<MediaSource>>,
    service: &DecodeService,
    cancel: &AtomicBool,
    bounded: bool,
) -> Completed {
    let mut source = job.existing.or(shared);
    let prepared = (|| {
        if cancel.load(Ordering::Relaxed) {
            return Ok(false);
        }
        if source.is_none() {
            MediaSource::prewarm_cancelled(&job.key.path, cancel)
                .map_err(|e| MediaFailure::Failed(e.to_string()))?;
            if cancel.load(Ordering::Relaxed) || stamp(&job.key.path) != Some(job.stamp) {
                return Ok(false);
            }
            source =
                Some(MediaSource::open(&job.key.path).map_err(|e| {
                    MediaFailure::Failed(format!("{}: {e}", job.key.path.display()))
                })?);
        }
        let src = source.as_ref().expect("source opened");
        match &job.interpret {
            Some(i) => video_drawable(src, i),
            None => video_supported(src),
        }
        .map_err(|m| MediaFailure::Unsupported(Feature::Media(m)))?;
        let none = Interpretation::default();
        let interp = job.interpret.as_ref().unwrap_or(&none);
        let v = src
            .probe
            .video
            .as_ref()
            .ok_or_else(|| MediaFailure::Failed("no video stream".into()))?;
        let reading = resolve_video(v, interp);
        let timing = Timing::new(src, &reading, job.composition);
        let ticks = (job.seconds * pm_time::TICKS_PER_SECOND as f64).round() as i64;
        let Some(pick) = timing.pick(src, ticks) else {
            return Ok(false);
        };
        if cancel.load(Ordering::Relaxed) {
            return Ok(false);
        }
        if bounded && service.usage().sessions >= MAX_PREPARED_SESSIONS {
            return Ok(false);
        }
        let purpose = if job.key.export {
            DecodePurpose::Export
        } else {
            DecodePurpose::Display
        };
        // Warm the exact selected decoded frame as well as its decoder. The
        // service's bounded frame cache is shared with both the viewer and
        // export path, so the first planned render can reuse it without
        // retaining a converted GPU texture. A changed source or cancelled
        // preparation is discarded by the usual completion checks.
        let mut session = service
            .session(src, job.key.instance, purpose)
            .map_err(|e| MediaFailure::Failed(e.to_string()))?;
        let frame = session
            .frame_with(pick.frame, &mut |_| cancel.load(Ordering::Relaxed))
            .map_err(|e| MediaFailure::Failed(e.to_string()))?;
        if frame.is_none() || cancel.load(Ordering::Relaxed) {
            return Ok(false);
        }
        Ok(true)
    })();
    Completed {
        key: job.key,
        stamp: job.stamp,
        source,
        prepared,
    }
}

/// A video draw of a planned frame: what the selected frame depends on.
pub(crate) struct PlannedVideoDraw {
    pub layer: String,
    pub path: PathBuf,
    pub instance: u64,
    pub seconds: f64,
    pub interpret: Option<Interpretation>,
    pub composition: pm_time::Rate,
}

impl MediaCache {
    /// Before a frame draws: opens the frame's video sources and decodes
    /// their selected frames into the shared decoded-frame cache, on a
    /// bounded pool of workers, when two or more of them need it. A source
    /// needs it when it is not open, or when this renderer has never drawn
    /// it and its selected frame is not decoded (background preparation
    /// opened it but stopped at its session cap). Drawing then finds each
    /// frame ready instead of decoding them one layer at a time.
    pub(crate) fn open_cold(&mut self, videos: &[PlannedVideoDraw], export: bool) {
        self.drain_prepared();
        let purpose = if export {
            DecodePurpose::Export
        } else {
            DecodePurpose::Display
        };
        let mut jobs: Vec<Job> = Vec::new();
        for v in videos {
            let Some(current) = stamp(&v.path) else {
                continue;
            };
            // One job per file: layers of a file share its decoded frames.
            if jobs.iter().any(|j| j.key.path == v.path) {
                continue;
            }
            let existing = self
                .sources
                .get(&v.path)
                .filter(|(seen, _)| *seen == current)
                .map(|(_, src)| src.clone());
            if let Some(src) = &existing
                && (self.frames.keys().any(|k| k.path == v.path)
                    || self.selected_frame_decoded(src, v, purpose))
            {
                continue;
            }
            jobs.push(Job {
                key: Key {
                    path: v.path.clone(),
                    instance: v.instance,
                    export,
                    stamp: current,
                    settings: String::new(),
                },
                stamp: current,
                existing,
                seconds: v.seconds,
                interpret: v.interpret.clone(),
                composition: v.composition,
            });
        }
        if jobs.len() < 2 {
            return;
        }
        // One worker per core at most: probes, indexes and decoder startup
        // are CPU work, and 50-clip projects must not start 50 at once.
        let workers = std::thread::available_parallelism()
            .map_or(2, |n| n.get())
            .min(jobs.len());
        let service = self.service.clone();
        let cancel = AtomicBool::new(false);
        let next = AtomicUsize::new(0);
        let jobs: Vec<Mutex<Option<Job>>> = jobs.into_iter().map(|j| Mutex::new(Some(j))).collect();
        let completed: Vec<Completed> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..workers)
                .map(|_| {
                    let (service, cancel, next, jobs) = (&service, &cancel, &next, &jobs);
                    scope.spawn(move || {
                        let mut done = Vec::new();
                        loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            let Some(job) = jobs.get(i).and_then(|j| j.lock().unwrap().take())
                            else {
                                break done;
                            };
                            done.push(prepare_one(job, None, service, cancel, false));
                        }
                    })
                })
                .collect();
            handles
                .into_iter()
                .filter_map(|h| h.join().ok())
                .flatten()
                .collect()
        });
        for result in completed {
            // Failures surface again, named, when the layer draws.
            let _ = self.adopt_prepared(result);
        }
    }

    /// Whether `v`'s selected frame of an open source is in the decoded
    /// cache.
    fn selected_frame_decoded(
        &self,
        src: &MediaSource,
        v: &PlannedVideoDraw,
        purpose: DecodePurpose,
    ) -> bool {
        let Some(video) = src.probe.video.as_ref() else {
            return false;
        };
        let none = Interpretation::default();
        let reading = resolve_video(video, v.interpret.as_ref().unwrap_or(&none));
        let timing = Timing::new(src, &reading, v.composition);
        let ticks = (v.seconds * pm_time::TICKS_PER_SECOND as f64).round() as i64;
        timing.pick(src, ticks).is_some_and(|pick| {
            self.service.cache().contains(&pm_media::cache::FrameKey {
                source: src.id,
                sample: src.index.frames[pick.frame].sample,
                purpose,
            })
        })
    }

    fn adopt_prepared(&mut self, result: Completed) -> Result<bool, MediaFailure> {
        if let Some(background) = &mut self.background {
            background.pending.remove(&result.key);
        }
        let valid = stamp(&result.key.path) == Some(result.stamp);
        if let Some(src) = result.source {
            let current_id = self.sources.get(&result.key.path).map(|(_, s)| s.id);
            if valid
                && self.sources.get(&result.key.path).map(|(seen, _)| *seen) != Some(result.stamp)
            {
                if let Some((_, old)) = self.sources.remove(&result.key.path) {
                    self.viewer.release(Some(old.id));
                    self.service.release(None, Some(old.id));
                    self.frames.retain(|key, _| key.path != result.key.path);
                    self.recount();
                }
                self.sources
                    .insert(result.key.path.clone(), (result.stamp, src));
            } else if current_id != Some(src.id) {
                self.service.release(None, Some(src.id));
            }
        }
        if valid { result.prepared } else { Ok(false) }
    }

    pub(super) fn drain_prepared(&mut self) {
        while let Some(result) = self
            .background
            .as_ref()
            .and_then(|background| background.results.try_recv().ok())
        {
            let _ = self.adopt_prepared(result);
        }
    }

    /// Explicit synchronization for headless benchmarks and tests. The UI
    /// presenter never waits here; an unready source takes the normal path.
    pub(crate) fn wait_for_preparation(&mut self) -> Vec<MediaFailure> {
        let mut failures = Vec::new();
        while self
            .background
            .as_ref()
            .is_some_and(|background| !background.pending.is_empty())
        {
            match self.background.as_ref().expect("worker").results.recv() {
                Ok(result) => {
                    if let Err(error) = self.adopt_prepared(result) {
                        failures.push(error);
                    }
                }
                Err(_) => break,
            }
        }
        failures
    }

    /// Queue source probe/index, decoder startup and the selected decoded
    /// frame without blocking the presenter. Nothing reads the whole file. Returns true for a queued job;
    /// full queues defer to normal source opening at render time.
    pub(crate) fn prepare_video(
        &mut self,
        path: &Path,
        instance: u64,
        seconds: f64,
        export: bool,
        interpret: Option<&Interpretation>,
        composition: pm_time::Rate,
    ) -> Result<bool, MediaFailure> {
        self.drain_prepared();
        let current = stamp(path)
            .ok_or_else(|| MediaFailure::Failed(format!("{} is offline", path.display())))?;
        if self
            .background
            .as_ref()
            .and_then(|background| background.worker.as_ref())
            .is_some_and(JoinHandle::is_finished)
        {
            self.background = None;
        }
        if self.background.is_none() {
            self.background = BackgroundPrepare::new(self.service.clone());
        }
        let Some(background) = &mut self.background else {
            // Thread creation failed; rendering still has its normal path.
            return Ok(false);
        };
        let key = Key {
            path: path.to_path_buf(),
            instance,
            export,
            stamp: current,
            settings: format!("{interpret:?}|{composition:?}|{seconds:?}"),
        };
        if background.pending.contains(&key) || background.pending.len() >= MAX_PENDING {
            return Ok(false);
        }
        let existing = self.sources.get(path).and_then(|(seen, source)| {
            if *seen == current {
                Some(source.clone())
            } else {
                None
            }
        });
        let job = Job {
            key: key.clone(),
            stamp: current,
            existing,
            seconds,
            interpret: interpret.cloned(),
            composition,
        };
        match background
            .sender
            .as_ref()
            .expect("worker sender")
            .try_send(job)
        {
            Ok(()) => {
                background.pending.insert(key);
                Ok(true)
            }
            Err(TrySendError::Full(_)) => Ok(false),
            Err(TrySendError::Disconnected(_)) => Err(MediaFailure::Failed(
                "media preparation worker stopped".into(),
            )),
        }
    }
}
