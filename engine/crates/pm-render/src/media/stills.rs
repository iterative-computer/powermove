//! Stills decoded ahead of the playhead on a worker, so a still layer
//! entering playback does not decode on the frame that meets it.
//!
//! The worker runs the same [`pm_media::image::read`] the frame would (and
//! warms the per-stamp facts its probe reads); the frame takes the result
//! when the file's stamp still matches and decodes itself otherwise; a frame
//! that meets a still the worker is decoding at that moment waits for it
//! instead of decoding it a second time. Only the decode moves: conversion
//! and upload stay on the frame, so the pixels are the ones a cold read gives.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pm_media::image::StillImage;
use pm_media::pool::FramePool;

/// A file at one stamp (length, modification time), asked for as a draw
/// reads it ([`StillAsk`]).
type StillKey = (PathBuf, u64, u128, StillAsk);

/// How the frame that draws a still reads it: the viewer's preview edge (the
/// longest side it shows the still at, when it draws previews) and whether
/// the asset has Interpret Footage overrides. The worker decodes what
/// `MediaCache::image_scaled` would for the same ask.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct StillAsk {
    pub edge: Option<u32>,
    pub interpreted: bool,
}

/// Stills queued or waiting for their frame at once.
const MAX_READY: usize = 8;
/// The decoded bytes held at most, counted from the moment the worker
/// decodes a still, so stills on their way to the frame count too (a UHD
/// 16-bit RGBA still is 66 MB). One still is held whatever its size, and so
/// is the one a frame is waiting for: dropping either made the frame decode
/// the same file again on the render thread.
const MAX_READY_BYTES: usize = 24 << 20;
/// A decoded still no frame took, or asked for again, within this long is
/// dropped: the playhead went elsewhere.
const READY_TTL: Duration = Duration::from_secs(4);

/// A decoded still and the preview edge it was decoded at (none: the whole
/// still at its native depth).
type Decoded = (StillImage, Option<u32>);

/// The decode worker: requests in, decoded stills out.
struct Worker {
    ask: Sender<StillKey>,
    answers: Receiver<(StillKey, Option<Decoded>)>,
    thread: JoinHandle<()>,
}

/// What the worker is doing, shared with the frame.
#[derive(Default)]
struct Progress {
    /// The still being decoded, from its dequeue until its answer is sent.
    decoding: Option<StillKey>,
    /// The still a frame waits for, or gave up waiting for: kept whatever
    /// the hold, since a frame needs it.
    wanted: Option<StillKey>,
}

/// Whether a frame got the decode it waited for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Waited {
    /// Nothing of this still was being decoded, or its answer arrived.
    Done,
    /// The decode is still running past the frame's bound: the frame shows
    /// what it has and a later frame takes the answer.
    Pending,
}

pub(crate) struct StillPrefetch {
    worker: Option<Worker>,
    pending: HashSet<StillKey>,
    /// Decoded stills and when they were last asked for.
    ready: HashMap<StillKey, (Decoded, Instant)>,
    /// Bytes of decoded stills, ready or still in the channel.
    held: Arc<AtomicUsize>,
    progress: Arc<Mutex<Progress>>,
    /// The decode a frame gave up waiting for: later frames do not wait for
    /// it again (each would lose its bound to the same stalled read).
    abandoned: Option<StillKey>,
    /// Bumped when a decode a frame gave up on has answered.
    landed: Arc<AtomicU64>,
    limit: usize,
    ttl: Duration,
    /// Held by a test to keep the worker inside a decode.
    #[cfg(test)]
    stall: Arc<Mutex<()>>,
}

impl Default for StillPrefetch {
    fn default() -> Self {
        Self::with_limits(MAX_READY_BYTES, READY_TTL)
    }
}

fn bytes(still: &StillImage) -> usize {
    still.frame.format.visible_bytes()
}

impl StillPrefetch {
    fn with_limits(limit: usize, ttl: Duration) -> Self {
        Self {
            worker: None,
            pending: HashSet::new(),
            ready: HashMap::new(),
            held: Arc::new(AtomicUsize::new(0)),
            progress: Default::default(),
            abandoned: None,
            landed: Default::default(),
            limit,
            ttl,
            #[cfg(test)]
            stall: Default::default(),
        }
    }

    /// Queues `path` at `stamp` for decoding as `ask` reads it, unless it is
    /// queued or ready.
    pub(crate) fn request(
        &mut self,
        path: &Path,
        stamp: (u64, u128),
        ask: StillAsk,
        pool: &FramePool,
    ) {
        self.drain();
        let key = (path.to_path_buf(), stamp.0, stamp.1, ask);
        if let Some((_, asked)) = self.ready.get_mut(&key) {
            *asked = Instant::now();
            return;
        }
        if self.pending.contains(&key) || self.pending.len() + self.ready.len() >= MAX_READY {
            return;
        }
        if self.worker.is_none() {
            let (ask, asks) = channel::<StillKey>();
            let (done, answers) = channel();
            let pool = pool.clone();
            let (held, limit) = (self.held.clone(), self.limit);
            let progress = self.progress.clone();
            let landed = self.landed.clone();
            #[cfg(test)]
            let stall = self.stall.clone();
            let Ok(handle) = std::thread::Builder::new()
                .name("pm-still-prefetch".into())
                .spawn(move || {
                    for key in asks {
                        progress.lock().unwrap().decoding = Some(key.clone());
                        #[cfg(test)]
                        drop(stall.lock().unwrap());
                        // The frame's probe reads the file's facts first,
                        // once per stamp (a whole decode): warm that cache
                        // here too. They also say whether the frame reads a
                        // preview of this still or the whole of it.
                        let facts = super::still_facts(&key.0);
                        let edge = key
                            .3
                            .edge
                            .filter(|_| facts.is_ok_and(|f| f.previewed(key.3.interpreted)));
                        let decoded = match edge {
                            Some(edge) => {
                                pm_media::image::imageio::read_preview(&key.0, &pool, edge)
                                    .map(|still| (still, Some(edge)))
                                    .or_else(|_| {
                                        pm_media::image::read(&key.0, &pool).map(|s| (s, None))
                                    })
                            }
                            None => pm_media::image::read(&key.0, &pool).map(|s| (s, None)),
                        };
                        // Kept: a still that fits beside the held ones,
                        // the only one held, and the one a frame waits for.
                        // The frame decodes the others itself. The answer
                        // is sent before the frame can see the decode as
                        // over, so it never starts one for a still on its
                        // way.
                        let mut progress = progress.lock().unwrap();
                        let wanted = progress.wanted.as_ref() == Some(&key);
                        let still = decoded.ok().filter(|(still, _)| {
                            let size = bytes(still);
                            held.fetch_update(Ordering::AcqRel, Ordering::Acquire, |h| {
                                (wanted || h == 0 || h + size <= limit).then_some(h + size)
                            })
                            .is_ok()
                        });
                        progress.decoding = None;
                        if done.send((key, still)).is_err() {
                            break;
                        }
                        // A frame gave up on it: the viewer draws again.
                        if wanted {
                            landed.fetch_add(1, Ordering::AcqRel);
                        }
                    }
                })
            else {
                return;
            };
            self.worker = Some(Worker {
                ask,
                answers,
                thread: handle,
            });
        }
        if let Some(worker) = &self.worker
            && worker.ask.send(key.clone()).is_ok()
        {
            self.pending.insert(key);
        }
    }

    /// Waits while the worker is decoding `path` at `stamp` as `ask` reads
    /// it, so the frame that needs this still now gets that decode (and the
    /// file's facts with it) instead of starting the same one beside it; the
    /// worker runs at the waiting thread's class meanwhile. Never past
    /// `until`, nor once `cancelled` says the frame is superseded: then the
    /// decode is [`Waited::Pending`], and no later frame waits for it again
    /// until it answers. A still only queued is not waited for.
    pub(crate) fn wait_for_decoding(
        &mut self,
        path: &Path,
        stamp: (u64, u128),
        ask: StillAsk,
        until: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Waited {
        let Some(worker) = &self.worker else {
            return Waited::Done;
        };
        let is = |k: &StillKey| k.0 == path && (k.1, k.2) == stamp && k.3 == ask;
        {
            let mut progress = self.progress.lock().unwrap();
            let Some(decoding) = progress.decoding.clone().filter(is) else {
                return Waited::Done;
            };
            if self.abandoned.as_ref() == Some(&decoding) {
                return Waited::Pending;
            }
            progress.wanted = Some(decoding);
        }
        let boost = qos::Boost::of(&worker.thread);
        loop {
            let now = Instant::now();
            if now >= until || cancelled() {
                // The worker keeps the answer for a later frame.
                self.abandoned = Some((path.to_path_buf(), stamp.0, stamp.1, ask));
                return Waited::Pending;
            }
            // In slices, so a cancellation is seen while the read stalls.
            match worker
                .answers
                .recv_timeout((until - now).min(Duration::from_millis(4)))
            {
                Ok((key, still)) => {
                    let done = is(&key);
                    let asked =
                        self.pending.remove(&key) || done || self.abandoned.as_ref() == Some(&key);
                    if self.abandoned.as_ref() == Some(&key) {
                        // Its frame gave up on it; the wanted mark is this
                        // frame's now.
                        self.abandoned = None;
                    }
                    // Released while it decoded: not kept.
                    let still = still.filter(|(still, _)| {
                        asked || {
                            self.held.fetch_sub(bytes(still), Ordering::AcqRel);
                            false
                        }
                    });
                    if let Some(still) = still {
                        self.ready.insert(key, (still, now));
                    }
                    if done {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        drop(boost);
        self.progress.lock().unwrap().wanted = None;
        Waited::Done
    }

    /// Whether `path` at `stamp` is a decode a frame gave up waiting for and
    /// that has not answered yet.
    pub(crate) fn pending(&mut self, path: &Path, stamp: (u64, u128)) -> bool {
        self.drain();
        self.abandoned
            .as_ref()
            .is_some_and(|k| k.0 == path && (k.1, k.2) == stamp)
    }

    #[cfg(test)]
    pub(crate) fn stall(&self) -> Arc<Mutex<()>> {
        self.stall.clone()
    }
    #[cfg(test)]
    pub(crate) fn decoding(&self) -> bool {
        self.progress.lock().unwrap().decoding.is_some()
    }

    /// Changes whenever a decode a frame gave up on answers.
    pub(crate) fn landed(&self) -> u64 {
        self.landed.load(Ordering::Acquire)
    }

    /// The still decoded for `path` at `stamp` and `ask`, if the worker has
    /// it decoded at `edge` (the preview edge the frame is about to read at;
    /// none: the whole still). One decoded another way is left to expire.
    pub(crate) fn take(
        &mut self,
        path: &Path,
        stamp: (u64, u128),
        ask: StillAsk,
        edge: Option<u32>,
    ) -> Option<StillImage> {
        self.drain();
        let key = (path.to_path_buf(), stamp.0, stamp.1, ask);
        if self.ready.get(&key)?.0.1 != edge {
            return None;
        }
        let ((still, _), _) = self.ready.remove(&key)?;
        self.held.fetch_sub(bytes(&still), Ordering::AcqRel);
        Some(still)
    }

    /// Waits until the worker has answered every request (headless tests
    /// and performance runs; a frame never waits).
    pub(crate) fn settle(&mut self) {
        while !self.pending.is_empty() {
            std::thread::sleep(Duration::from_millis(2));
            self.drain();
        }
    }

    /// Decoded bytes held: ready, and on their way to the frame.
    pub(crate) fn held_bytes(&self) -> u64 {
        self.held.load(Ordering::Acquire) as u64
    }

    /// Lets go of every decoded still (a cache budget of zero, a viewer at
    /// rest, memory pressure), and of the answers of decodes already
    /// running when they arrive: nothing asked for them since.
    pub(crate) fn release(&mut self) {
        self.drain();
        self.pending.clear();
        for ((still, _), _) in std::mem::take(&mut self.ready).into_values() {
            self.held.fetch_sub(bytes(&still), Ordering::AcqRel);
        }
    }

    fn drain(&mut self) {
        let now = Instant::now();
        if let Some(worker) = &self.worker {
            while let Ok((key, still)) = worker.answers.try_recv() {
                let asked = self.pending.remove(&key) || self.abandoned.as_ref() == Some(&key);
                // Released while it decoded: not kept.
                let still = still.filter(|(still, _)| {
                    asked || {
                        self.held.fetch_sub(bytes(still), Ordering::AcqRel);
                        false
                    }
                });
                if self.abandoned.as_ref() == Some(&key) {
                    self.abandoned = None;
                    let mut progress = self.progress.lock().unwrap();
                    if progress.wanted.as_ref() == Some(&key) {
                        progress.wanted = None;
                    }
                }
                if let Some(still) = still {
                    self.ready.insert(key, (still, now));
                }
            }
        }
        let (held, ttl) = (&self.held, self.ttl);
        self.ready.retain(|_, ((still, _), asked)| {
            let keep = now.duration_since(*asked) < ttl;
            if !keep {
                held.fetch_sub(bytes(still), Ordering::AcqRel);
            }
            keep
        });
    }
}

/// The worker lent the class of the thread that waits for it: a render
/// thread at user-interactive would otherwise wait for a decode the
/// scheduler holds back at the default class.
#[cfg(target_os = "macos")]
mod qos {
    use std::ffi::c_void;
    use std::os::unix::thread::JoinHandleExt;

    unsafe extern "C" {
        fn qos_class_self() -> u32;
        fn pthread_override_qos_class_start_np(
            thread: std::os::unix::thread::RawPthread,
            qos: u32,
            relative: i32,
        ) -> *mut c_void;
        fn pthread_override_qos_class_end_np(token: *mut c_void) -> i32;
    }

    pub(super) struct Boost(*mut c_void);

    impl Boost {
        pub(super) fn of(thread: &std::thread::JoinHandle<()>) -> Self {
            // SAFETY: the handle keeps the thread's identifier valid; the
            // override ends when this value drops.
            Self(unsafe {
                pthread_override_qos_class_start_np(thread.as_pthread_t(), qos_class_self(), 0)
            })
        }
    }

    impl Drop for Boost {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: the token came from the matching start call.
                unsafe { pthread_override_qos_class_end_np(self.0) };
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod qos {
    pub(super) struct Boost;
    impl Boost {
        pub(super) fn of(_: &std::thread::JoinHandle<()>) -> Self {
            Self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plane_bytes(still: &StillImage) -> Vec<Vec<u8>> {
        let pm_media::frame::Pixels::Pool(frame) = &still.frame.pixels else {
            panic!("PNG stills decode into pool memory");
        };
        frame
            .planes
            .iter()
            .zip(&still.frame.format.planes)
            .map(|(plane, desc)| {
                let row = desc.width * desc.format.bytes();
                (0..desc.height)
                    .flat_map(|y| unsafe {
                        std::slice::from_raw_parts(plane.ptr().add(y * plane.stride), row).to_vec()
                    })
                    .collect()
            })
            .collect()
    }

    /// A still the worker decoded is the one a direct read gives; another
    /// stamp of the file is never answered.
    #[test]
    fn prefetched_stills_are_direct_reads() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/video/vp9-10bit-709-limited.paused.rgba.png");
        let pool = FramePool::new(0);
        let stamp = crate::media::stamp(&path).expect("fixture exists");
        let mut prefetch = StillPrefetch::default();
        prefetch.request(&path, stamp, StillAsk::default(), &pool);
        let started = std::time::Instant::now();
        let still = loop {
            if let Some(still) = prefetch.take(&path, stamp, StillAsk::default(), None) {
                break still;
            }
            assert!(started.elapsed().as_secs() < 30, "the worker answers");
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let direct = pm_media::image::read(&path, &pool).unwrap();
        assert_eq!(still.frame.format, direct.frame.format);
        assert_eq!(
            (
                still.bit_depth,
                still.float,
                &still.color,
                still.orientation
            ),
            (
                direct.bit_depth,
                direct.float,
                &direct.color,
                direct.orientation
            )
        );
        assert_eq!(plane_bytes(&still), plane_bytes(&direct));
        assert!(
            prefetch
                .take(&path, stamp, StillAsk::default(), None)
                .is_none(),
            "taken once"
        );
        prefetch.request(&path, stamp, StillAsk::default(), &pool);
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(
            prefetch
                .take(&path, (stamp.0 + 1, stamp.1), StillAsk::default(), None)
                .is_none()
        );
    }

    fn fixture() -> (PathBuf, (u64, u128)) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/video/vp9-10bit-709-limited.paused.rgba.png");
        let stamp = crate::media::stamp(&path).expect("fixture exists");
        (path, stamp)
    }

    fn settle(prefetch: &mut StillPrefetch) {
        prefetch.settle();
    }

    /// Asked for at the viewer's preview edge, an ordinary still is decoded
    /// no larger than that edge and only a frame reading that edge takes it;
    /// an interpreted asset's still is decoded whole, as its frame reads it.
    #[test]
    fn a_preview_ask_is_decoded_at_the_edge_the_frame_reads() {
        let (path, stamp) = fixture();
        let pool = FramePool::new(0);
        let whole = pm_media::image::read(&path, &pool).unwrap();
        let (w, h) = (whole.frame.format.width, whole.frame.format.height);
        let edge = (w.max(h) / 4).next_power_of_two() as u32;
        assert!(
            (edge as usize) < w.max(h),
            "the fixture is larger than the edge"
        );
        let ask = StillAsk {
            edge: Some(edge),
            interpreted: false,
        };
        let mut prefetch = StillPrefetch::default();
        prefetch.request(&path, stamp, ask, &pool);
        settle(&mut prefetch);
        assert!(
            prefetch.take(&path, stamp, ask, None).is_none(),
            "a frame reading the whole still does not get a preview"
        );
        let still = prefetch.take(&path, stamp, ask, Some(edge)).unwrap();
        let side = still.frame.format.width.max(still.frame.format.height);
        assert!(side <= edge as usize, "{side} > {edge}");
        let interpreted = StillAsk {
            edge: Some(edge),
            interpreted: true,
        };
        prefetch.request(&path, stamp, interpreted, &pool);
        settle(&mut prefetch);
        let still = prefetch.take(&path, stamp, interpreted, None).unwrap();
        assert_eq!(still.frame.format.width, w);
    }

    /// A still no frame takes leaves after the TTL, with its bytes, and
    /// stops holding a place, so later requests are queued again.
    #[test]
    fn unconsumed_stills_expire_and_free_their_places() {
        let (path, stamp) = fixture();
        let pool = FramePool::new(0);
        let mut prefetch = StillPrefetch::with_limits(MAX_READY_BYTES, Duration::from_millis(150));
        prefetch.request(&path, stamp, StillAsk::default(), &pool);
        settle(&mut prefetch);
        assert_eq!(prefetch.ready.len(), 1);
        assert!(prefetch.held.load(Ordering::Acquire) > 0);
        // Asking again keeps it.
        std::thread::sleep(Duration::from_millis(100));
        prefetch.request(&path, stamp, StillAsk::default(), &pool);
        std::thread::sleep(Duration::from_millis(100));
        prefetch.drain();
        assert_eq!(prefetch.ready.len(), 1, "asked for again within the TTL");
        // Left alone, it expires.
        std::thread::sleep(Duration::from_millis(200));
        prefetch.drain();
        assert!(prefetch.ready.is_empty());
        assert_eq!(prefetch.held.load(Ordering::Acquire), 0);
        // Fill every place with stamps no frame will take; they expire too.
        for n in 0..MAX_READY as u64 {
            prefetch.request(
                &path,
                (stamp.0, stamp.1 + u128::from(n) + 1),
                StillAsk::default(),
                &pool,
            );
        }
        settle(&mut prefetch);
        assert_eq!(prefetch.ready.len(), MAX_READY);
        std::thread::sleep(Duration::from_millis(200));
        prefetch.request(&path, stamp, StillAsk::default(), &pool);
        settle(&mut prefetch);
        assert!(
            prefetch
                .take(&path, stamp, StillAsk::default(), None)
                .is_some(),
            "a place was free again"
        );
        assert_eq!(prefetch.ready.len(), 0);
        assert_eq!(prefetch.held.load(Ordering::Acquire), 0);
    }

    /// A still larger than the whole hold is handed to its frame all the
    /// same (dropped, the frame decoded the file a second time); a second
    /// one beside it is not kept.
    #[test]
    fn one_still_is_held_whatever_its_size() {
        let (path, stamp) = fixture();
        let pool = FramePool::new(0);
        let mut prefetch = StillPrefetch::with_limits(1, READY_TTL);
        prefetch.request(&path, stamp, StillAsk::default(), &pool);
        prefetch.request(&path, (stamp.0, stamp.1 + 1), StillAsk::default(), &pool);
        settle(&mut prefetch);
        assert_eq!(prefetch.ready.len(), 1, "one over the limit, not two");
        assert!(
            prefetch
                .take(&path, stamp, StillAsk::default(), None)
                .is_some()
        );
        assert_eq!(prefetch.held.load(Ordering::Acquire), 0);
    }

    /// A frame that needs the still the worker is decoding at that moment
    /// waits for that decode and takes its result, even when it does not fit
    /// beside the stills already held; one only queued is not waited for.
    #[test]
    fn a_frame_waits_for_the_decode_in_progress() {
        let (path, stamp) = fixture();
        let pool = FramePool::new(0);
        let mut prefetch = StillPrefetch::with_limits(1, READY_TTL);
        let ask = StillAsk::default();
        // One still held, so the next does not fit.
        prefetch.request(&path, (stamp.0, stamp.1 + 1), ask, &pool);
        settle(&mut prefetch);
        assert_eq!(prefetch.ready.len(), 1);
        // A thread holds the worker's stall until told to let go.
        let (stall, (locked, held), (go, release)) = (prefetch.stall.clone(), channel(), channel());
        let holder = std::thread::spawn(move || {
            let _guard = stall.lock().unwrap();
            locked.send(()).unwrap();
            release.recv().unwrap();
            std::thread::sleep(Duration::from_millis(100));
        });
        held.recv().unwrap();
        prefetch.request(&path, stamp, ask, &pool);
        prefetch.request(&path, (stamp.0, stamp.1 + 2), ask, &pool);
        while prefetch.progress.lock().unwrap().decoding.is_none() {
            std::thread::sleep(Duration::from_millis(1));
        }
        // Queued behind the decode in progress: no wait, nothing to take.
        let never = Instant::now() + Duration::from_secs(60);
        assert_eq!(
            prefetch.wait_for_decoding(&path, (stamp.0, stamp.1 + 2), ask, never, &|| false),
            Waited::Done
        );
        assert!(
            prefetch
                .take(&path, (stamp.0, stamp.1 + 2), ask, None)
                .is_none()
        );
        assert!(prefetch.take(&path, stamp, ask, None).is_none());
        // The decode runs 100 ms after this; the frame waits for it.
        go.send(()).unwrap();
        assert_eq!(
            prefetch.wait_for_decoding(&path, stamp, ask, never, &|| false),
            Waited::Done
        );
        assert!(
            prefetch.take(&path, stamp, ask, None).is_some(),
            "the decode in progress reached the frame"
        );
        holder.join().unwrap();
        settle(&mut prefetch);
        assert!(prefetch.progress.lock().unwrap().wanted.is_none());
    }

    /// The byte bound counts a still from its decode: one that does not fit
    /// never reaches the ready set, and the held bytes stay within it.
    #[test]
    fn decoded_stills_are_bounded_before_they_queue() {
        let (path, stamp) = fixture();
        let pool = FramePool::new(0);
        let one = bytes(&pm_media::image::read(&path, &pool).unwrap());
        let mut prefetch = StillPrefetch::with_limits(one * 2, READY_TTL);
        for n in 0..4u128 {
            prefetch.request(&path, (stamp.0, stamp.1 + n), StillAsk::default(), &pool);
        }
        settle(&mut prefetch);
        assert_eq!(prefetch.ready.len(), 2, "two fit");
        assert_eq!(prefetch.held.load(Ordering::Acquire), one * 2);
        assert!(
            prefetch
                .take(&path, stamp, StillAsk::default(), None)
                .is_some()
        );
        assert_eq!(prefetch.held.load(Ordering::Acquire), one);
    }

    /// Holds the worker inside its next decode until the returned sender is
    /// dropped or sent to: a read that does not return.
    fn stall_worker(prefetch: &StillPrefetch) -> (Sender<()>, std::thread::JoinHandle<()>) {
        let (stall, (locked, held), (go, release)) = (prefetch.stall.clone(), channel(), channel());
        let holder = std::thread::spawn(move || {
            let _guard = stall.lock().unwrap();
            locked.send(()).unwrap();
            let _ = release.recv();
        });
        held.recv().unwrap();
        (go, holder)
    }

    fn decoding(prefetch: &StillPrefetch) {
        let started = Instant::now();
        while prefetch.progress.lock().unwrap().decoding.is_none() {
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "the worker starts"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// A decode that never returns holds a frame no longer than its bound:
    /// the frame is told it is pending, later frames do not wait for it
    /// again, and when it does answer the answer is kept for the next frame
    /// (whatever the hold) and `landed` changes.
    #[test]
    fn a_stalled_decode_holds_a_frame_no_longer_than_its_bound() {
        let (path, stamp) = fixture();
        let pool = FramePool::new(0);
        let mut prefetch = StillPrefetch::with_limits(1, READY_TTL);
        let ask = StillAsk::default();
        // One still held, so the next does not fit the hold.
        prefetch.request(&path, (stamp.0, stamp.1 + 1), ask, &pool);
        settle(&mut prefetch);
        let (go, holder) = stall_worker(&prefetch);
        prefetch.request(&path, stamp, ask, &pool);
        decoding(&prefetch);
        let landed = prefetch.landed();
        let started = Instant::now();
        let bound = started + Duration::from_millis(40);
        assert_eq!(
            prefetch.wait_for_decoding(&path, stamp, ask, bound, &|| false),
            Waited::Pending
        );
        let waited = started.elapsed();
        assert!(
            waited < Duration::from_millis(1000),
            "the frame waited {waited:?} for a decode that does not return"
        );
        assert!(prefetch.pending(&path, stamp));
        // A later frame does not spend its own bound on the same read.
        let started = Instant::now();
        let far = started + Duration::from_secs(60);
        assert_eq!(
            prefetch.wait_for_decoding(&path, stamp, ask, far, &|| false),
            Waited::Pending
        );
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(prefetch.take(&path, stamp, ask, None).is_none());
        // The read returns: the answer waits for the next frame.
        drop(go);
        holder.join().unwrap();
        let started = Instant::now();
        while prefetch.landed() == landed {
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "the decode lands"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!prefetch.pending(&path, stamp));
        assert!(
            prefetch.take(&path, stamp, ask, None).is_some(),
            "kept beside the held still for the frame that gave up on it"
        );
        assert!(prefetch.progress.lock().unwrap().wanted.is_none());
    }

    /// An exact frame may wait longer, but stops waiting as soon as it is
    /// superseded (a seek, a new revision, the viewer closing).
    #[test]
    fn a_superseded_frame_stops_waiting_for_a_stalled_decode() {
        let (path, stamp) = fixture();
        let pool = FramePool::new(0);
        let mut prefetch = StillPrefetch::default();
        let ask = StillAsk::default();
        let (go, holder) = stall_worker(&prefetch);
        prefetch.request(&path, stamp, ask, &pool);
        decoding(&prefetch);
        let superseded = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = superseded.clone();
        let seek = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            flag.store(true, Ordering::Release);
        });
        let started = Instant::now();
        let far = started + Duration::from_secs(60);
        assert_eq!(
            prefetch.wait_for_decoding(&path, stamp, ask, far, &|| superseded
                .load(Ordering::Acquire)),
            Waited::Pending
        );
        let waited = started.elapsed();
        assert!(
            waited < Duration::from_secs(5),
            "a superseded frame waited {waited:?}"
        );
        seek.join().unwrap();
        drop(go);
        holder.join().unwrap();
        settle(&mut prefetch);
        assert!(prefetch.take(&path, stamp, ask, None).is_some());
    }

    /// A release (budget zero, idle, pressure) lets go of every decoded
    /// still and of the answer of a decode already running when it lands;
    /// the held bytes are what a governor counts.
    #[test]
    fn a_release_lets_go_of_held_stills_and_late_answers() {
        let (path, stamp) = fixture();
        let pool = FramePool::new(0);
        let mut prefetch = StillPrefetch::default();
        let ask = StillAsk::default();
        prefetch.request(&path, stamp, ask, &pool);
        settle(&mut prefetch);
        assert!(prefetch.held_bytes() > 0, "a decoded still is counted");
        prefetch.release();
        assert_eq!(prefetch.held_bytes(), 0);
        assert!(prefetch.ready.is_empty());
        // Released while the worker decodes: the answer is not kept.
        let (go, holder) = stall_worker(&prefetch);
        prefetch.request(&path, (stamp.0, stamp.1 + 1), ask, &pool);
        decoding(&prefetch);
        prefetch.release();
        drop(go);
        holder.join().unwrap();
        let started = Instant::now();
        while prefetch.progress.lock().unwrap().decoding.is_some()
            || started.elapsed() < Duration::from_millis(50)
        {
            assert!(started.elapsed() < Duration::from_secs(30));
            std::thread::sleep(Duration::from_millis(2));
        }
        std::thread::sleep(Duration::from_millis(20));
        prefetch.drain();
        assert!(prefetch.ready.is_empty(), "a late answer was kept");
        assert_eq!(prefetch.held_bytes(), 0);
    }
}
