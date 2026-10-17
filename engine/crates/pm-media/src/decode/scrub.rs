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
//! nothing after a jump.

use super::service::{DecodeService, InstanceId, MediaSource};
use super::session::{DEFAULT_WINDOW, Progress};
use crate::cache::FrameKey;
use crate::error::{MediaError, Result};
use crate::frame::DecodedFrame;
use crate::vt::DecodePurpose;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// How the viewer wants a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Want {
    /// The exact frame; the caller waits for it.
    Exact,
    /// The exact frame if decoded, otherwise the nearest decoded frame now
    /// while the exact one decodes.
    Scrub,
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
    pub errors: u64,
    /// Worker threads started (one per source and instance; idle ones
    /// exit).
    pub workers: u64,
}

/// Frames decoded ahead of a target in the direction of motion.
const AHEAD: usize = 8;
/// While scrubbing backward, one seek from the GOP start serves the
/// reverse steps of up to this many presented frames before its target:
/// every reference picture on the way and the unreferenced ones at the
/// drag's stride are kept (within three quarters of the frame cache).
const KEEP_BEFORE: usize = 32;
/// Moves of more frames than this are jumps (a click on the ruler, a
/// random seek), not drags: nothing is decoded ahead of them.
const MAX_STRIDE: i64 = 16;
/// An idle worker exits after this long; the next request starts another.
const IDLE_EXIT: Duration = Duration::from_secs(30);

/// The decode workers of one renderer's display frames.
pub struct ViewerDecode {
    service: Arc<DecodeService>,
    workers: Mutex<HashMap<(u64, InstanceId), Arc<Worker>>>,
    landed: Arc<AtomicU64>,
    stats: Arc<Mutex<ScrubStats>>,
}

struct Worker {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

struct Shared {
    source: Arc<MediaSource>,
    instance: InstanceId,
    service: Arc<DecodeService>,
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
        ViewerDecode {
            service,
            workers: Mutex::new(HashMap::new()),
            landed: Arc::new(AtomicU64::new(0)),
            stats: Arc::new(Mutex::new(ScrubStats::default())),
        }
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
            purpose: DecodePurpose::Display,
        };
        self.stats.lock().unwrap().requests += 1;
        let worker = self.worker(source, instance);
        let shared = &worker.shared;
        if let Some(frame) = self.service.cache().get(&key) {
            self.stats.lock().unwrap().cached += 1;
            let mut st = shared.state.lock().unwrap();
            st.moved_to(n);
            // A drag that moves through cached frames keeps the look-ahead
            // ahead of it.
            st.ahead_from = Some(n);
            drop(st);
            shared.wake.notify_one();
            return Ok(Fetched {
                frame,
                shown: n,
                exact: true,
            });
        }
        let mut st = shared.state.lock().unwrap();
        st.moved_to(n);
        if let Some(e) = st.failed.remove(&n) {
            self.stats.lock().unwrap().errors += 1;
            return Err(MediaError::decode(e));
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
                return Ok(Fetched {
                    frame,
                    shown,
                    exact: false,
                });
            }
            // Nothing decoded yet to stand in: wait for the exact frame.
        }
        self.stats.lock().unwrap().waits += 1;
        st.exact.push_back(n);
        *st.waiting.entry(n).or_default() += 1;
        st.ahead_from = None;
        shared.wake.notify_one();
        loop {
            if let Some(frame) = st.delivered.get(&n).cloned() {
                let left = st.waiting.get_mut(&n).map(|w| {
                    *w -= 1;
                    *w
                });
                if left == Some(0) {
                    st.waiting.remove(&n);
                    st.delivered.remove(&n);
                }
                return Ok(Fetched {
                    frame,
                    shown: n,
                    exact: true,
                });
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
                self.stats.lock().unwrap().errors += 1;
                return Err(MediaError::decode(e));
            }
            if !shared.alive.load(Ordering::Acquire) {
                return Err(MediaError::decode("the decode worker stopped"));
            }
            st = shared.done.wait(st).unwrap();
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
            shown_at,
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            done: Condvar::new(),
            alive: AtomicBool::new(true),
            landed: self.landed.clone(),
            stats: self.stats.clone(),
        });
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
            purpose: DecodePurpose::Display,
        }
    }

    /// The decoded frame nearest presented frame `n`: the cached frame of
    /// this source closest in presentation order (the earlier one on a
    /// tie), else the newest frame the worker decoded.
    fn nearest(&self, n: usize, st: &State) -> Option<(usize, DecodedFrame)> {
        let cache = self.service.cache();
        let best = cache
            .samples_of(self.source.id, DecodePurpose::Display)
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

    fn run(self: Arc<Self>) {
        unsafe {
            libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0);
        }
        while let Some(job) = self.next_job() {
            let n = job.frame();
            let result = (|| -> Result<Option<DecodedFrame>> {
                let mut guard =
                    self.service
                        .session(&self.source, self.instance, DecodePurpose::Display)?;
                let (keep, stride) = {
                    let st = self.state.lock().unwrap();
                    if st.stride < 0 {
                        (self.reverse_reach(&st), st.stride.unsigned_abs() as usize)
                    } else {
                        (0, 1)
                    }
                };
                guard.set_window(DEFAULT_WINDOW.max(keep + 1));
                guard.set_keep_before(keep, stride);
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
                    drop(stats);
                    if !matches!(job, Job::Ahead(_)) {
                        st.ahead_from = Some(n);
                    }
                }
                Ok(None) => {
                    self.stats.lock().unwrap().stopped += 1;
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
            let (next, timeout) = self.wake.wait_timeout(st, IDLE_EXIT).unwrap();
            st = next;
            let idle = st.exact.is_empty() && st.scrub.is_none();
            if timeout.timed_out() && idle {
                // Exiting under the lock: `fetch` sees `alive` false only
                // after this, and starts a new worker.
                self.alive.store(false, Ordering::Release);
                return None;
            }
        }
    }

    /// The next frame to decode ahead of the last target, a stride at a
    /// time in the direction of motion: [`AHEAD`] steps forward within half
    /// the frame cache, or one reverse neighbourhood
    /// ([`ViewerDecode`]'s reverse reach). Nothing after a jump.
    fn ahead(&self, st: &mut State) -> Option<usize> {
        let from = st.ahead_from?;
        let stride = st.stride;
        if stride == 0 {
            st.ahead_from = None;
            return None;
        }
        // Reverse steps need a seek from the GOP start for every
        // neighbourhood, so they look as far ahead as one neighbourhood.
        let count = if stride < 0 {
            (self.reverse_reach(st) as i64 / -stride).max(1) as usize
        } else {
            AHEAD.min(self.cache_frames(st))
        };
        let frames = self.source.index.frames.len() as i64;
        let cache = self.service.cache();
        for i in 1..=count as i64 {
            let m = from as i64 + i * stride;
            if m < 0 || m >= frames {
                break;
            }
            let m = m as usize;
            if !cache.contains(&self.key(m)) {
                return Some(m);
            }
        }
        st.ahead_from = None;
        None
    }

    /// Frames that fit in half the frame cache (the rest keeps what the
    /// viewer showed recently).
    fn cache_frames(&self, st: &State) -> usize {
        let budget = self.service.cache().stats().budget;
        budget / 2 / st.frame_bytes.max(1)
    }

    /// How far before a target a reverse scrub keeps frames: [`KEEP_BEFORE`]
    /// within three quarters of the frame cache.
    fn reverse_reach(&self, st: &State) -> usize {
        let budget = self.service.cache().stats().budget;
        KEEP_BEFORE.min(budget / 4 * 3 / st.frame_bytes.max(1))
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
}
