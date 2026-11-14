//! The viewer's decode worker: scrubbing and paused frames without waiting
//! on the render thread (docs/engine/scrub.md; plan sections 13 and 15).
//!
//! One worker thread per source and viewer instance owns the decoding of
//! display frames. The render thread asks it for presented frame `n`:
//!
//! - [`Want::Exact`] (paused frames after an exact seek, playback) waits for
//!   the exact frame. Exact frames are never approximated.
//! - [`Want::Scrub`] returns at once: the exact frame when it is decoded,
//!   otherwise the nearest decoded frame of the source (a stand-in, as
//!   today's viewer draws its video element's last decoded picture) while
//!   the worker decodes the exact one. [`ViewerDecode::landed`] changes when
//!   it lands, and the presenter draws again.
//! - [`Want::Playback`] (frames while playing) waits at most the frame's
//!   decode allowance for the exact frame, then returns a stand-in, so a
//!   slow frame costs picture accuracy and never the render thread's
//!   deadline. Look-ahead normally has it cached already. Playing forward,
//!   a stand-in is never earlier than a frame already shown (the picture
//!   holds, never goes back), and a frame the decoder's run has already
//!   passed is held over rather than decoded again from its sync sample
//!   when that costs more than waiting for the run to reach the playhead.
//!
//! The worker keeps one decoder session warm per source and instance (the
//! decode service's pool), always serves the newest scrub target (older
//! targets that never started are dropped), and stops a target in flight
//! when a newer one is cheaper to reach than finishing it: a target earlier
//! on the same run, or a seek elsewhere costing fewer samples than the
//! samples left. A stopped decode keeps its place, so a later target
//! further along the run continues from there. After a target lands it
//! decodes ahead in the direction of motion into the frame cache (the GOP
//! neighbourhood, within the cache's lease share), a stride at a time;
//! the display worker does nothing after a jump. The source-only Export
//! worker may warm the rest of a bounded GOP after a jump.

use super::service::{DecodeService, InstanceId, MediaSource};
use super::session::{DEFAULT_WINDOW, Progress};
use crate::cache::FrameKey;
use crate::error::{MediaError, Result};
use crate::frame::DecodedFrame;
use crate::vt::DecodePurpose;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How the viewer wants a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Want {
    /// The exact frame; the caller waits for it.
    Exact,
    /// The exact frame if decoded, otherwise the nearest decoded frame now
    /// while the exact one decodes.
    Scrub,
    /// The exact frame if it lands within this allowance, otherwise the
    /// nearest decoded frame while the exact one keeps decoding. With nothing
    /// decoded to stand in (the first frame of a source) it waits, as `Scrub`
    /// does.
    Playback(Duration),
}

/// A frame for the viewer.
#[derive(Clone)]
pub struct Fetched {
    pub frame: DecodedFrame,
    /// The presented frame it is (the request's when `exact`).
    pub shown: usize,
    pub exact: bool,
}

/// Counters for the presenter's report and the measurements.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrubStats {
    pub requests: u64,
    /// Requests answered from the frame cache.
    pub cached: u64,
    /// Scrub requests answered with a stand-in.
    pub stand_ins: u64,
    /// Requests that waited for their exact frame.
    pub waits: u64,
    /// Targets decoded (scrub and exact).
    pub decoded: u64,
    /// Scrub targets replaced by a newer one before they started.
    pub superseded: u64,
    /// Targets stopped in flight for a newer one.
    pub stopped: u64,
    /// Frames decoded ahead in the direction of motion.
    pub ahead: u64,
    /// Targets a playing viewer asked for after the decoder's run had
    /// passed them: held over instead of seeking back.
    pub passed_over: u64,
    pub errors: u64,
    /// Worker threads started (one per source and instance; idle ones
    /// exit).
    pub workers: u64,
}

/// Frames decoded ahead of a target in the direction of motion.
const AHEAD: usize = 8;
/// Source monitor seeks can be scattered within a short GOP. Warm only
/// contiguous presented frames of the same GOP, bounded by the decoder
/// lease and this count; a new exact request always preempts look-ahead.
const SOURCE_GOP_AHEAD: usize = 64;
/// While scrubbing backward, one seek from the GOP start serves the
/// reverse steps of up to this many presented frames before its target:
/// every reference picture on the way and the unreferenced ones at the
/// drag's stride are kept (within three quarters of the worker's part of
/// the frame cache).
const KEEP_BEFORE: usize = 32;
/// Moves of more frames than this are jumps (a click on the ruler, a
/// random seek), not drags. Only the source Export worker warms the bounded
/// remainder of the current GOP after them.
const MAX_STRIDE: i64 = 16;
/// An idle worker exits after this long; the next request starts another.
pub const IDLE_EXIT: Duration = Duration::from_secs(30);

/// The decode workers of one renderer's display frames.
pub struct ViewerDecode {
    service: Arc<DecodeService>,
    purpose: DecodePurpose,
    workers: Mutex<HashMap<(u64, InstanceId), Arc<Worker>>>,
    landed: Arc<AtomicU64>,
    stats: Arc<Mutex<ScrubStats>>,
    idle_exit: Duration,
    /// Workers running: they share the frame cache's look-ahead room.
    live: Arc<AtomicUsize>,
}

struct Worker {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

struct Shared {
    source: Arc<MediaSource>,
    instance: InstanceId,
    service: Arc<DecodeService>,
    purpose: DecodePurpose,
    /// Presented position of each sample.
    shown_at: Vec<usize>,
    state: Mutex<State>,
    /// Wakes the worker.
    wake: Condvar,
    /// Wakes callers waiting for an exact frame.
    done: Condvar,
    alive: AtomicBool,
    landed: Arc<AtomicU64>,
    stats: Arc<Mutex<ScrubStats>>,
    idle_exit: Duration,
    live: Arc<AtomicUsize>,
}

#[derive(Default)]
struct State {
    /// Exact frames callers wait for, oldest first.
    exact: VecDeque<usize>,
    /// Waiters per exact frame, and the frames delivered to them.
    waiting: HashMap<usize, usize>,
    delivered: HashMap<usize, DecodedFrame>,
    /// The newest scrub target not yet started.
    scrub: Option<usize>,
    /// The scrub target in flight.
    busy: Option<usize>,
    /// Errors of targets since asked.
    failed: HashMap<usize, String>,
    /// The last frame requested, and the step of the last move when it was
    /// a drag (at most [`MAX_STRIDE`] frames; 0 after a jump or before any
    /// move): its sign is the direction of motion.
    last: Option<usize>,
    stride: i64,
    /// Decode ahead of this frame by `stride`.
    ahead_from: Option<usize>,
    /// The newest frame the worker decoded: the stand-in of last resort.
    latest: Option<(usize, DecodedFrame)>,
    /// The newest frame handed to a playing viewer and its presented
    /// position. A playing viewer is never handed an earlier frame than
    /// this until it moves back (a loop or a seek): holding a picture
    /// is a stutter, going back in time is a glitch.
    presented: Option<(usize, DecodedFrame)>,
    /// The last request came from a playing viewer ([`Want::Playback`]).
    playing: bool,
    /// Frames a caller waits for without a deadline ([`Want::Exact`]):
    /// never passed over.
    firm: HashMap<usize, usize>,
    /// Bytes of one decoded frame (bounds the look-ahead by the cache).
    frame_bytes: usize,
    shutdown: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Job {
    Exact(usize),
    Scrub(usize),
    Ahead(usize),
}

impl Job {
    fn frame(self) -> usize {
        match self {
            Job::Exact(n) | Job::Scrub(n) | Job::Ahead(n) => n,
        }
    }
}

impl ViewerDecode {
    pub fn new(service: Arc<DecodeService>) -> ViewerDecode {
        Self::with_purpose(service, DecodePurpose::Display)
    }

    /// An exact source viewer can keep Export-purpose frames and its decoder
    /// warm without changing the composition viewer's Display-purpose cache.
    pub fn with_purpose(service: Arc<DecodeService>, purpose: DecodePurpose) -> ViewerDecode {
        ViewerDecode {
            service,
            purpose,
            workers: Mutex::new(HashMap::new()),
            landed: Arc::new(AtomicU64::new(0)),
            stats: Arc::new(Mutex::new(ScrubStats::default())),
            idle_exit: IDLE_EXIT,
            live: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Idle workers exit after `idle` instead of [`IDLE_EXIT`] (workers
    /// started from now on).
    pub fn with_idle_exit(mut self, idle: Duration) -> ViewerDecode {
        self.idle_exit = idle;
        self
    }

    pub fn service(&self) -> &Arc<DecodeService> {
        &self.service
    }

    /// Changes whenever a worker finished a target (decoded or failed): a
    /// frame drawn with stand-ins can be drawn again.
    pub fn landed(&self) -> u64 {
        self.landed.load(Ordering::Acquire)
    }

    pub fn stats(&self) -> ScrubStats {
        *self.stats.lock().unwrap()
    }

    /// Workers running; each looks ahead within an equal part of the
    /// frame cache.
    pub fn live_workers(&self) -> usize {
        self.live.load(Ordering::Acquire)
    }

    /// Whether any scrub target is still waiting or decoding.
    pub fn pending(&self) -> bool {
        self.workers.lock().unwrap().values().any(|w| {
            let st = w.shared.state.lock().unwrap();
            st.scrub.is_some() || st.busy.is_some() || !st.exact.is_empty()
        })
    }

    /// Presented frame `n` of `source` for viewer instance `instance`.
    pub fn fetch(
        &self,
        source: &Arc<MediaSource>,
        instance: InstanceId,
        n: usize,
        want: Want,
    ) -> Result<Fetched> {
        let sample = source
            .index
            .frames
            .get(n)
            .ok_or_else(|| MediaError::invalid(format!("frame {n} out of range")))?
            .sample;
        let key = FrameKey {
            source: source.id,
            sample,
            purpose: self.purpose,
        };
        self.stats.lock().unwrap().requests += 1;
        loop {
            let worker = self.worker(source, instance);
            if let Some(fetched) = self.fetch_on(&worker.shared, &key, n, want) {
                return fetched;
            }
            // The worker exited idle before it saw the request: `worker`
            // starts another.
        }
    }

    /// [`Self::fetch`] on one worker; `None` when the worker exited idle
    /// before the request reached it. A worker exits only under its state
    /// lock: one seen alive while that lock is held decodes what is queued
    /// before it may exit.
    fn fetch_on(
        &self,
        shared: &Shared,
        key: &FrameKey,
        n: usize,
        want: Want,
    ) -> Option<Result<Fetched>> {
        if let Some(frame) = self.service.cache().get(key) {
            let mut st = shared.state.lock().unwrap();
            if !shared.alive.load(Ordering::Acquire) {
                return None;
            }
            self.stats.lock().unwrap().cached += 1;
            super::diag::bump(&super::diag::FETCH_CACHED, 1);
            st.moved_to(n);
            st.present(n, want, n, &frame);
            // A drag that moves through cached frames keeps the look-ahead
            // ahead of it.
            st.ahead_from = Some(n);
            drop(st);
            shared.wake.notify_one();
            return Some(Ok(Fetched {
                frame,
                shown: n,
                exact: true,
            }));
        }
        let mut st = shared.state.lock().unwrap();
        if !shared.alive.load(Ordering::Acquire) {
            return None;
        }
        st.moved_to(n);
        st.playing = matches!(want, Want::Playback(_));
        st.forget_presented_unless_forward(n, want);
        if let Some(e) = st.failed.remove(&n) {
            self.stats.lock().unwrap().errors += 1;
            return Some(Err(MediaError::decode(e)));
        }
        if want == Want::Scrub {
            if st.scrub.is_some_and(|old| old != n) {
                self.stats.lock().unwrap().superseded += 1;
            }
            if st.busy != Some(n) {
                st.scrub = Some(n);
            }
            st.ahead_from = None;
            shared.wake.notify_one();
            if let Some((shown, frame)) = shared.nearest(n, &st) {
                self.stats.lock().unwrap().stand_ins += 1;
                    super::diag::bump(&super::diag::FETCH_STAND_INS, 1);
                return Some(Ok(Fetched {
                    frame,
                    shown,
                    exact: false,
                }));
            }
            // Nothing decoded yet to stand in: wait for the exact frame.
        }
        self.stats.lock().unwrap().waits += 1;
        super::diag::bump(&super::diag::FETCH_WAITS, 1);
        let waited = Instant::now();
        st.exact.push_back(n);
        *st.waiting.entry(n).or_default() += 1;
        if want == Want::Exact {
            *st.firm.entry(n).or_default() += 1;
        }
        st.ahead_from = None;
        shared.wake.notify_one();
        let deadline = match want {
            Want::Playback(allowance) => Instant::now().checked_add(allowance),
            _ => None,
        };
        loop {
            if let Some(frame) = st.delivered.get(&n).cloned() {
                super::diag::bump(&super::diag::FETCH_WAIT_US, waited.elapsed().as_micros() as u64);
                let left = st.waiting.get_mut(&n).map(|w| {
                    *w -= 1;
                    *w
                });
                if left == Some(0) {
                    st.waiting.remove(&n);
                    st.delivered.remove(&n);
                }
                st.unfirm(n, want);
                st.present(n, want, n, &frame);
                return Some(Ok(Fetched {
                    frame,
                    shown: n,
                    exact: true,
                }));
            }
            if let Some(e) = st.failed.get(&n).cloned() {
                let left = st.waiting.get_mut(&n).map(|w| {
                    *w -= 1;
                    *w
                });
                if left == Some(0) {
                    st.waiting.remove(&n);
                    st.failed.remove(&n);
                }
                st.unfirm(n, want);
                self.stats.lock().unwrap().errors += 1;
                return Some(Err(MediaError::decode(e)));
            }
            if !shared.alive.load(Ordering::Acquire) {
                // Shut down (its source was released) after it took the
                // request: not an idle exit.
                return Some(Err(MediaError::decode("the decode worker stopped")));
            }
            let Some(deadline) = deadline else {
                st = shared.done.wait(st).unwrap();
                continue;
            };
            let now = Instant::now();
            if now >= deadline {
                if let Some((shown, frame)) = shared.playing_stand_in(n, &st) {
                    // Keep decoding it as the newest target: a queue of late
                    // exact frames would only ever fall further behind.
                    st.leave(n);
                    st.present(n, want, shown, &frame);
                    self.stats.lock().unwrap().stand_ins += 1;
                    super::diag::bump(&super::diag::FETCH_STAND_INS, 1);
                    return Some(Ok(Fetched {
                        frame,
                        shown,
                        exact: false,
                    }));
                }
                st = shared.done.wait(st).unwrap();
                continue;
            }
            st = shared.done.wait_timeout(st, deadline - now).unwrap().0;
        }
    }

    /// Stops and forgets the workers of a source (it was replaced on disk
    /// or closed), or of every source with `None`.
    pub fn release(&self, source: Option<u64>) {
        let gone: Vec<Arc<Worker>> = {
            let mut workers = self.workers.lock().unwrap();
            let keys: Vec<_> = workers
                .keys()
                .filter(|k| source.is_none_or(|s| k.0 == s))
                .copied()
                .collect();
            keys.iter().filter_map(|k| workers.remove(k)).collect()
        };
        for w in gone {
            w.stop();
        }
    }

    fn worker(&self, source: &Arc<MediaSource>, instance: InstanceId) -> Arc<Worker> {
        let mut workers = self.workers.lock().unwrap();
        let key = (source.id, instance);
        if let Some(w) = workers.get(&key)
            && w.shared.alive.load(Ordering::Acquire)
        {
            return w.clone();
        }
        if let Some(old) = workers.remove(&key) {
            old.stop();
        }
        let mut shown_at = vec![usize::MAX; source.index.samples.len()];
        for (n, f) in source.index.frames.iter().enumerate() {
            shown_at[f.sample] = shown_at[f.sample].min(n);
        }
        let shared = Arc::new(Shared {
            source: source.clone(),
            instance,
            service: self.service.clone(),
            purpose: self.purpose,
            shown_at,
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            done: Condvar::new(),
            alive: AtomicBool::new(true),
            landed: self.landed.clone(),
            stats: self.stats.clone(),
            idle_exit: self.idle_exit,
            live: self.live.clone(),
        });
        self.live.fetch_add(1, Ordering::AcqRel);
        let run = shared.clone();
        let thread = std::thread::Builder::new()
            .name("pm-media scrub".into())
            .spawn(move || run.run())
            .expect("spawn the decode worker");
        self.stats.lock().unwrap().workers += 1;
        let w = Arc::new(Worker {
            shared,
            thread: Mutex::new(Some(thread)),
        });
        workers.insert(key, w.clone());
        w
    }
}

impl Drop for ViewerDecode {
    fn drop(&mut self) {
        self.release(None);
    }
}

impl Worker {
    fn stop(&self) {
        self.shared.state.lock().unwrap().shutdown = true;
        self.shared.wake.notify_all();
        if let Some(t) = self.thread.lock().unwrap().take() {
            let _ = t.join();
        }
    }
}

impl State {
    /// A caller stops waiting for exact frame `n`; the frame remains the
    /// worker's scrub target unless it is already being decoded.
    fn leave(&mut self, n: usize) {
        let left = self.waiting.get_mut(&n).map(|w| {
            *w -= 1;
            *w
        });
        if left == Some(0) {
            self.waiting.remove(&n);
            self.exact.retain(|&e| e != n);
            if self.busy != Some(n) {
                self.scrub = Some(n);
            }
        }
    }

    fn unfirm(&mut self, n: usize, want: Want) {
        if want == Want::Exact
            && let Some(w) = self.firm.get_mut(&n)
        {
            *w -= 1;
            if *w == 0 {
                self.firm.remove(&n);
            }
        }
    }

    /// Records a frame handed to a playing viewer.
    fn present(&mut self, n: usize, want: Want, shown: usize, frame: &DecodedFrame) {
        self.forget_presented_unless_forward(n, want);
        if matches!(want, Want::Playback(_))
            && self.presented.as_ref().is_none_or(|(p, _)| shown >= *p)
        {
            self.presented = Some((shown, frame.clone()));
        }
    }

    /// A paused or scrubbing viewer, or a playing one that moved back
    /// (a loop or a seek), may be handed any frame again.
    fn forget_presented_unless_forward(&mut self, n: usize, want: Want) {
        if !matches!(want, Want::Playback(_))
            || self.presented.as_ref().is_some_and(|(p, _)| n < *p)
        {
            self.presented = None;
        }
    }

    fn moved_to(&mut self, n: usize) {
        if let Some(last) = self.last
            && last != n
        {
            let d = n as i64 - last as i64;
            self.stride = if d.abs() <= MAX_STRIDE { d } else { 0 };
        }
        self.last = Some(n);
    }
}

impl Shared {
    fn key(&self, n: usize) -> FrameKey {
        FrameKey {
            source: self.source.id,
            sample: self.source.index.frames[n].sample,
            purpose: self.purpose,
        }
    }

    /// The decoded frame nearest presented frame `n`: the cached frame of
    /// this source closest in presentation order (the earlier one on a
    /// tie), else the newest frame the worker decoded.
    fn nearest(&self, n: usize, st: &State) -> Option<(usize, DecodedFrame)> {
        let cache = self.service.cache();
        let best = cache
            .samples_of(self.source.id, self.purpose)
            .into_iter()
            .filter_map(|s| self.shown_at.get(s).copied().filter(|&m| m != usize::MAX))
            .min_by_key(|&m| (m.abs_diff(n), m > n));
        if let Some(m) = best
            && let Some(f) = cache.get(&self.key(m))
        {
            return Some((m, f));
        }
        st.latest.clone()
    }

    /// The stand-in for a playing viewer at presented frame `n`: the newest
    /// decoded frame at or before `n` that is not earlier than the frame it
    /// was last handed, else that frame again. Playing forward, the picture
    /// holds or advances and never goes back in time (a cached frame from
    /// earlier in the run, after the newer ones were evicted, did).
    fn playing_stand_in(&self, n: usize, st: &State) -> Option<(usize, DecodedFrame)> {
        let Some((floor, held)) = st.presented.as_ref() else {
            return self.nearest(n, st);
        };
        let cache = self.service.cache();
        let cached = cache
            .samples_of(self.source.id, self.purpose)
            .into_iter()
            .filter_map(|s| self.shown_at.get(s).copied())
            .filter(|&m| (*floor..=n).contains(&m))
            .max()
            .and_then(|m| cache.get(&self.key(m)).map(|f| (m, f)));
        let latest = st
            .latest
            .clone()
            .filter(|(m, _)| (*floor..=n).contains(m));
        [cached, latest]
            .into_iter()
            .flatten()
            .max_by_key(|(m, _)| *m)
            .or_else(|| Some((*floor, held.clone())))
    }

    fn run(self: Arc<Self>) {
        unsafe {
            libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0);
        }
        while let Some(job) = self.next_job() {
            let n = job.frame();
            let mut passed_over = false;
            let result = (|| -> Result<Option<DecodedFrame>> {
                let mut guard = self
                    .service
                    .session(&self.source, self.instance, self.purpose)?;
                let (keep, stride, forward) = {
                    let st = self.state.lock().unwrap();
                    // Holding over needs a frame to hold: a worker that
                    // has handed a playing viewer nothing yet (a fresh one
                    // on a warm session) decodes the frame.
                    let forward = st.playing
                        && st.stride > 0
                        && st.presented.is_some()
                        && !st.firm.contains_key(&n);
                    if st.stride < 0 {
                        (self.reverse_reach(&st), st.stride.unsigned_abs() as usize, forward)
                    } else {
                        (0, 1, forward)
                    }
                };
                guard.set_window(DEFAULT_WINDOW.max(keep + 1));
                guard.set_keep_before(keep, stride);
                // A frame the decoder's run has passed (decoded ahead, then
                // evicted) is not decoded again by seeking back while the
                // viewer plays forward: look-ahead steps past it, and a
                // playing viewer holds its picture for the few frames until
                // the run reaches its playhead, when that costs less than
                // decoding again from the sync sample. Seeking back sent a
                // clip with one sync sample to its start and held the
                // picture ~350 ms (Fable 5.5, "23c Solar farm", 46.2 s).
                let passed = guard.passed(n);
                let pass_over = match job {
                    Job::Ahead(_) => passed.is_some(),
                    _ => forward && passed.is_some_and(|(back, again)| back <= again),
                };
                if pass_over {
                    passed_over = true;
                    return Ok(None);
                }
                match job {
                    Job::Exact(n) => guard.frame(n).map(Some),
                    Job::Scrub(n) | Job::Ahead(n) => {
                        let index = self.source.index.clone();
                        let target = index.frames[n].sample;
                        guard.frame_with(n, &mut |p| self.supersedes(job, target, p))
                    }
                }
            })();
            let mut st = self.state.lock().unwrap();
            if matches!(job, Job::Scrub(_)) {
                st.busy = None;
            }
            match result {
                Ok(Some(frame)) => {
                    st.frame_bytes = st.frame_bytes.max(frame.bytes());
                    if st.waiting.contains_key(&n) {
                        st.delivered.insert(n, frame.clone());
                    }
                    st.exact.retain(|&e| e != n);
                    if st.scrub == Some(n) {
                        st.scrub = None;
                    }
                    st.latest = Some((n, frame));
                    let mut stats = self.stats.lock().unwrap();
                    match job {
                        Job::Ahead(_) => stats.ahead += 1,
                        _ => stats.decoded += 1,
                    }
                    super::diag::bump(
                        if matches!(job, Job::Ahead(_)) {
                            &super::diag::WORKER_AHEAD
                        } else {
                            &super::diag::WORKER_DECODED
                        },
                        1,
                    );
                    drop(stats);
                    if !matches!(job, Job::Ahead(_)) {
                        st.ahead_from = Some(n);
                    }
                }
                Ok(None) if passed_over => {
                    self.stats.lock().unwrap().passed_over += 1;
                    super::diag::bump(&super::diag::WORKER_PASSED_OVER, 1);
                    // Waiters take a stand-in at their deadline; decoding
                    // continues ahead from here.
                    st.exact.retain(|&e| e != n);
                    st.ahead_from = Some(n);
                }
                Ok(None) => {
                    self.stats.lock().unwrap().stopped += 1;
                    super::diag::bump(&super::diag::WORKER_STOPPED, 1);
                    if matches!(job, Job::Ahead(_)) {
                        st.ahead_from = None;
                    }
                }
                Err(e) => {
                    if !matches!(job, Job::Ahead(_)) {
                        st.failed.insert(n, e.to_string());
                    }
                    st.exact.retain(|&e| e != n);
                    st.ahead_from = None;
                }
            }
            drop(st);
            if !matches!(job, Job::Ahead(_)) {
                self.landed.fetch_add(1, Ordering::AcqRel);
            }
            self.done.notify_all();
        }
        self.alive.store(false, Ordering::Release);
        self.live.fetch_sub(1, Ordering::AcqRel);
        self.done.notify_all();
    }

    /// The next job, waiting for one; `None` to exit (shut down, or idle).
    fn next_job(&self) -> Option<Job> {
        let mut st = self.state.lock().unwrap();
        loop {
            if st.shutdown {
                return None;
            }
            if let Some(&n) = st.exact.front() {
                return Some(Job::Exact(n));
            }
            if let Some(n) = st.scrub.take() {
                st.busy = Some(n);
                return Some(Job::Scrub(n));
            }
            if let Some(m) = self.ahead(&mut st) {
                return Some(Job::Ahead(m));
            }
            let (next, timeout) = self.wake.wait_timeout(st, self.idle_exit).unwrap();
            st = next;
            let idle = st.exact.is_empty() && st.scrub.is_none();
            if timeout.timed_out() && idle {
                // Exiting under the lock: a request queued while it was
                // held is decoded first (the loop sees it); one that takes
                // the lock later sees `alive` false and `fetch` starts a
                // new worker.
                self.alive.store(false, Ordering::Release);
                return None;
            }
        }
    }

    /// The next frame to decode ahead of the last target, a stride at a
    /// time in the direction of motion: [`AHEAD`] steps forward within half
    /// its part of the frame cache, or one reverse neighbourhood
    /// ([`ViewerDecode`]'s reverse reach). The source Export worker may
    /// warm a bounded same-GOP run after a jump; composition does not.
    fn ahead(&self, st: &mut State) -> Option<usize> {
        let from = st.ahead_from?;
        let stride = st.stride;
        if stride == 0 && self.purpose != DecodePurpose::Export {
            st.ahead_from = None;
            return None;
        }
        // Reverse steps need a seek from the GOP start for every
        // neighbourhood, so they look as far ahead as one neighbourhood.
        let count = if stride == 0 {
            SOURCE_GOP_AHEAD.min(self.cache_frames(st))
        } else if stride < 0 {
            (self.reverse_reach(st) as i64 / -stride).max(1) as usize
        } else {
            AHEAD.min(self.cache_frames(st))
        };
        let frames = self.source.index.frames.len() as i64;
        let cache = self.service.cache();
        let source_gop = (stride == 0).then(|| {
            self.source
                .index
                .sync_for(self.source.index.frames[from].sample)
        });
        for i in 1..=count as i64 {
            let m = from as i64 + i * if stride == 0 { 1 } else { stride };
            if m < 0 || m >= frames {
                break;
            }
            let m = m as usize;
            if source_gop.is_some_and(|gop| {
                self.source
                    .index
                    .sync_for(self.source.index.frames[m].sample)
                    != gop
            }) {
                break;
            }
            if !cache.contains(&self.key(m)) {
                return Some(m);
            }
        }
        st.ahead_from = None;
        None
    }

    /// This worker's part of the frame cache: an equal share among the
    /// running workers (sources scrubbed or played together share one
    /// cache).
    fn cache_share(&self) -> usize {
        let budget = self.service.cache().stats().budget;
        budget / self.live.load(Ordering::Acquire).max(1)
    }

    /// Frames that fit in half this worker's part of the frame cache (the
    /// rest keeps what the viewer showed recently).
    fn cache_frames(&self, st: &State) -> usize {
        self.cache_share() / 2 / st.frame_bytes.max(1)
    }

    /// How far before a target a reverse scrub keeps frames: [`KEEP_BEFORE`]
    /// within three quarters of this worker's part of the frame cache.
    fn reverse_reach(&self, st: &State) -> usize {
        KEEP_BEFORE.min(self.cache_share() / 4 * 3 / st.frame_bytes.max(1))
    }

    /// Whether a newer request should stop the job in flight. A request
    /// earlier on the same run stops it (continuing reaches the request
    /// sooner); a request further along the same run does not (finishing
    /// first costs it nothing). A request elsewhere stops look-ahead, and
    /// stops a scrub target when its own seek costs fewer samples than
    /// finishing the target; otherwise finishing first costs the newer
    /// request less than starting over, and shows a frame meanwhile.
    fn supersedes(&self, job: Job, target: usize, p: Progress) -> bool {
        let st = self.state.lock().unwrap();
        if st.shutdown {
            return true;
        }
        let newer = st.exact.front().copied().or(st.scrub);
        drop(st);
        let Some(r) = newer else { return false };
        let index = &self.source.index;
        let rs = index.frames[r].sample;
        if rs == target {
            return false;
        }
        let sync = index.sync_for(rs);
        let on_path = sync >= p.run_start && sync < p.next.max(p.run_start + 1) && rs >= p.next;
        if on_path {
            return rs < target;
        }
        if matches!(job, Job::Ahead(_)) {
            // Look-ahead is a guess; a request elsewhere is not.
            return true;
        }
        let remaining = (target + 1).saturating_sub(p.next);
        let fresh = rs + 1 - sync;
        remaining > fresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stride_follows_drags_and_resets_on_jumps() {
        let mut st = State::default();
        st.moved_to(10);
        assert_eq!(st.stride, 0);
        st.moved_to(12);
        assert_eq!(st.stride, 2);
        st.moved_to(12);
        assert_eq!(st.stride, 2);
        st.moved_to(3);
        assert_eq!(st.stride, -9);
        st.moved_to(300);
        assert_eq!(st.stride, 0, "a jump is not a drag");
    }

    /// A request that reaches a worker after it exited idle is not queued
    /// there (nobody would decode it): `fetch` starts another worker.
    #[test]
    fn a_worker_that_exited_idle_takes_no_requests() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/h264-aac.mp4");
        let src = MediaSource::open(&path).unwrap();
        let service = Arc::new(DecodeService::new(Default::default()));
        let viewer = ViewerDecode::new(service).with_idle_exit(Duration::ZERO);
        // The worker `fetch` took, exiting idle before it locked the state.
        let worker = viewer.worker(&src, 1);
        let t = std::time::Instant::now();
        while worker.shared.alive.load(Ordering::Acquire) {
            assert!(t.elapsed().as_secs() < 20, "the worker never exited");
            std::thread::sleep(Duration::from_millis(1));
        }
        let key = |n: usize| FrameKey {
            source: src.id,
            sample: src.index.frames[n].sample,
            purpose: DecodePurpose::Display,
        };
        for want in [Want::Exact, Want::Scrub] {
            assert!(viewer.fetch_on(&worker.shared, &key(5), 5, want).is_none());
        }
        let st = worker.shared.state.lock().unwrap();
        assert!(st.exact.is_empty() && st.scrub.is_none());
        drop(st);
        let f = viewer.fetch(&src, 1, 5, Want::Exact).unwrap();
        assert!(f.exact && f.shown == 5);
        assert_eq!(viewer.stats().workers, 2);
    }

    /// Playing forward, a stand-in is never earlier than a frame already
    /// shown: Fable 5.5 at 46 s showed source frame 41, then 38 (the newer
    /// frames were evicted while an older one stayed cached).
    #[test]
    fn playback_never_goes_back_to_an_earlier_frame() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/h264-aac.mp4");
        let src = MediaSource::open(&path).unwrap();
        assert!(src.index.frames.len() > 12);
        let service = Arc::new(DecodeService::new(Default::default()));
        let viewer = ViewerDecode::new(service.clone());
        let key = |n: usize| FrameKey {
            source: src.id,
            sample: src.index.frames[n].sample,
            purpose: DecodePurpose::Display,
        };
        let early = viewer.fetch(&src, 1, 3, Want::Exact).unwrap();
        let shown = viewer.fetch(&src, 1, 8, Want::Exact).unwrap();
        assert!(shown.exact && shown.shown == 8);
        // Playing reaches frame 8 from the cache.
        let play = |n| viewer.fetch(&src, 1, n, Want::Playback(Duration::ZERO)).unwrap();
        assert!(play(8).exact);
        // Frame 8 and everything after it is evicted; frame 3 stays. (The
        // look-ahead from 8 settles first and is not restarted.)
        std::thread::sleep(Duration::from_millis(300));
        let worker = viewer.worker(&src, 1);
        {
            let mut st = worker.shared.state.lock().unwrap();
            st.ahead_from = None;
            service.cache().remove_source(src.id);
            service.cache().insert(key(3), early.frame.clone());
            st.latest = Some((3, early.frame.clone()));
        }
        let late = play(12);
        assert!(
            late.exact || late.shown >= 8,
            "playing forward went back to frame {}",
            late.shown
        );
        // Moving back (a loop) may show earlier frames again.
        {
            let mut st = worker.shared.state.lock().unwrap();
            service.cache().remove_source(src.id);
            service.cache().insert(key(3), early.frame.clone());
            st.latest = Some((3, early.frame.clone()));
        }
        let looped = play(2);
        assert!(looped.exact || looped.shown < 8, "a loop held frame {}", looped.shown);
    }

    /// Playing forward, a frame the decoder's run has passed (decoded ahead,
    /// then evicted) is held over, not decoded again from the sync sample.
    #[test]
    fn playback_does_not_seek_back_for_a_frame_the_run_passed() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/vp9-opus.webm");
        let src = MediaSource::open(&path).unwrap();
        let service = Arc::new(DecodeService::new(Default::default()));
        let viewer = ViewerDecode::new(service.clone());
        let play = |n, wait| {
            viewer
                .fetch(&src, 1, n, Want::Playback(Duration::from_millis(wait)))
                .unwrap()
        };
        for n in 0..=6 {
            assert!(play(n, 5_000).exact, "frame {n} did not land");
        }
        // The next render finds frame 6 cached and the look-ahead runs past
        // frame 7 on the same run (frame 8 lands).
        assert!(play(6, 0).exact);
        let t = std::time::Instant::now();
        let key8 = FrameKey {
            source: src.id,
            sample: src.index.frames[8].sample,
            purpose: DecodePurpose::Display,
        };
        while !service.cache().contains(&key8) {
            assert!(
                t.elapsed().as_secs() < 5,
                "the look-ahead never ran: {:?} cache {:?} samples {:?}",
                viewer.stats(),
                service.cache().stats(),
                service.cache().samples_of(src.id, DecodePurpose::Display)
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(Duration::from_millis(100));
        let worker = viewer.worker(&src, 1);
        {
            let mut st = worker.shared.state.lock().unwrap();
            st.ahead_from = None;
            service.cache().remove_source(src.id);
            service.clear_idle_windows();
        }
        let before = viewer.stats();
        let held = play(7, 0);
        assert!(!held.exact && held.shown == 6, "held {} ({})", held.shown, held.exact);
        let t = std::time::Instant::now();
        while viewer.stats().passed_over == before.passed_over {
            assert!(t.elapsed().as_secs() < 20, "frame 7 was decoded again from its sync sample");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(viewer.stats().decoded, before.decoded);
        // A paused viewer still gets the exact frame.
        let exact = viewer.fetch(&src, 1, 7, Want::Exact).unwrap();
        assert!(exact.exact && exact.shown == 7);
    }

    /// A playing viewer never waits out a decode: past the allowance it gets
    /// a stand-in at once and the exact frame lands for the next render.
    #[test]
    fn playback_past_its_allowance_shows_a_stand_in_and_the_exact_frame_lands() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/h264-aac.mp4");
        let src = MediaSource::open(&path).unwrap();
        let service = Arc::new(DecodeService::new(Default::default()));
        let viewer = ViewerDecode::new(service);
        let first = viewer.fetch(&src, 1, 0, Want::Exact).unwrap();
        assert!(first.exact && first.shown == 0);
        let last = src.index.frames.len() - 1;
        let late = viewer
            .fetch(&src, 1, last, Want::Playback(Duration::ZERO))
            .unwrap();
        assert!(!late.exact, "a zero allowance cannot wait for a decode");
        assert!(late.shown < last);
        let t = std::time::Instant::now();
        loop {
            let f = viewer
                .fetch(&src, 1, last, Want::Playback(Duration::ZERO))
                .unwrap();
            if f.exact {
                assert_eq!(f.shown, last);
                break;
            }
            assert!(t.elapsed().as_secs() < 20, "the exact frame never landed");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
