//! Stills decoded ahead of the playhead on a worker, so a still layer
//! entering playback does not decode on the frame that meets it.
//!
//! The worker runs the same [`pm_media::image::read`] the frame would (and
//! warms the per-stamp facts its probe reads); the frame takes the result
//! when the file's stamp still matches and decodes itself otherwise. Only the decode moves: conversion and upload stay on
//! the frame, so the pixels are the ones a cold read gives.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
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

/// Stills queued or waiting for their frame at once (eight UHD 16-bit
/// frames are 530 MB; more than its workers decode in a second).
const MAX_READY: usize = 8;
/// The decoded bytes held at most until the engine leases a share of its
/// decoded frame allowance ([`StillPrefetch::set_limit`]), counting stills
/// queued for decoding (at the size of the last one decoded), decoding and
/// on their way to the frame (a UHD 16-bit RGBA still is 66 MB).
pub(crate) const MAX_READY_BYTES: usize = 24 << 20;
/// Decode workers: a playing image sequence needs several stills decoded
/// at once to keep up (one UHD 16-bit PNG takes longer than a frame).
fn workers() -> usize {
    std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(1, 6))
}
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
    _threads: Vec<JoinHandle<()>>,
}

/// Requests a worker is decoding now, and queued ones the frame decoded
/// itself (a worker skips those).
#[derive(Default)]
struct Claims {
    started: HashSet<StillKey>,
    dropped: HashSet<StillKey>,
}

pub(crate) struct StillPrefetch {
    worker: Option<Worker>,
    claims: Arc<std::sync::Mutex<Claims>>,
    /// Queued and decoding stills, each with the bytes it was admitted at.
    pending: HashMap<StillKey, usize>,
    /// The size of the last still decoded: what the next is admitted at.
    last_bytes: Option<usize>,
    /// Decoded stills and when they were last asked for.
    ready: HashMap<StillKey, (Decoded, Instant)>,
    /// Bytes of decoded stills, ready or still in the channel.
    held: Arc<AtomicUsize>,
    limit: Arc<AtomicUsize>,
    ttl: Duration,
}

impl Default for StillPrefetch {
    fn default() -> Self {
        Self::with_limits(MAX_READY_BYTES, READY_TTL)
    }
}

fn bytes_of(still: &StillImage) -> usize {
    still.frame.format.visible_bytes()
}

/// What a worker answers for `key`: the still decoded as its frame will
/// read it, when it fits beside the held ones (the frame decodes the others
/// itself).
fn decode(key: &StillKey, pool: &FramePool, held: &AtomicUsize, limit: usize) -> Option<Decoded> {
    let path = &key.0;
    // The frame's probe reads the file's facts first, once per stamp. Read
    // whole here, the still gives them, and is not decoded twice for them.
    let (facts, whole) = match super::known_still_facts(path) {
        Some(facts) => (facts, None),
        None => {
            let whole = pm_media::image::read(path, pool).ok();
            (super::still_facts_from(path, whole.as_ref()), whole)
        }
    };
    // The facts say whether the frame reads a preview of this still or the
    // whole of it.
    let edge = key
        .3
        .edge
        .filter(|_| facts.is_ok_and(|f| f.previewed(key.3.interpreted)));
    let decoded = match (edge, whole) {
        (Some(edge), whole) => pm_media::image::imageio::read_preview(path, pool, edge)
            .map(|still| (still, Some(edge)))
            .or_else(|_| match whole {
                Some(whole) => Ok((whole, None)),
                None => pm_media::image::read(path, pool).map(|s| (s, None)),
            }),
        (None, Some(whole)) => Ok((whole, None)),
        (None, None) => pm_media::image::read(path, pool).map(|s| (s, None)),
    };
    decoded.ok().filter(|(still, _)| {
        let size = bytes_of(still);
        held.fetch_update(Ordering::AcqRel, Ordering::Acquire, |h| {
            (h + size <= limit).then_some(h + size)
        })
        .is_ok()
    })
}

impl StillPrefetch {
    fn with_limits(limit: usize, ttl: Duration) -> Self {
        Self {
            worker: None,
            claims: Default::default(),
            pending: HashMap::new(),
            last_bytes: None,
            ready: HashMap::new(),
            held: Arc::new(AtomicUsize::new(0)),
            limit: Arc::new(AtomicUsize::new(limit)),
            ttl,
        }
    }

    /// Holds up to `bytes` of decoded stills: a lower allowance gives the
    /// stills held beyond it back now, those asked for longest ago first.
    pub(crate) fn set_limit(&mut self, bytes: usize) {
        self.limit.store(bytes, Ordering::Release);
        self.drain();
        let mut by_age: Vec<(StillKey, Instant)> = self
            .ready
            .iter()
            .map(|(k, (_, asked))| (k.clone(), *asked))
            .collect();
        by_age.sort_by_key(|(_, asked)| *asked);
        for (key, _) in by_age {
            if self.held.load(Ordering::Acquire) <= bytes {
                break;
            }
            if let Some(((still, _), _)) = self.ready.remove(&key) {
                self.held.fetch_sub(bytes_of(&still), Ordering::AcqRel);
            }
        }
    }

    /// Bytes of decoded stills held (ready, or decoded and on their way).
    pub(crate) fn held(&self) -> usize {
        self.held.load(Ordering::Acquire)
    }

    /// Play stopped: every still decoded ahead is given back, queued ones
    /// are not decoded, and ones decoding now are dropped as they land.
    pub(crate) fn clear(&mut self) {
        self.drain();
        {
            let mut claims = self.claims.lock().unwrap();
            for key in self.pending.keys() {
                if !claims.started.contains(key) {
                    claims.dropped.insert(key.clone());
                }
            }
        }
        self.pending.clear();
        for ((still, _), _) in std::mem::take(&mut self.ready).into_values() {
            self.held.fetch_sub(bytes_of(&still), Ordering::AcqRel);
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
        if self.pending.contains_key(&key) || self.pending.len() + self.ready.len() >= MAX_READY {
            return;
        }
        // Admitted before it is decoded: at the last still's size, beside
        // the held and queued ones; with no size known yet, alone.
        let queued: usize = self.pending.values().sum();
        let admitted = match self.last_bytes {
            None if !self.pending.is_empty() => return,
            None => 0,
            Some(size) => size,
        };
        if self.held.load(Ordering::Acquire) + queued + admitted
            > self.limit.load(Ordering::Acquire)
        {
            return;
        }
        if self.worker.is_none() {
            let (ask, asks) = channel::<StillKey>();
            let asks = Arc::new(std::sync::Mutex::new(asks));
            let (done, answers) = channel();
            let mut threads = Vec::new();
            for _ in 0..workers() {
                let (asks, done) = (asks.clone(), done.clone());
                let pool = pool.clone();
                let (held, limit) = (self.held.clone(), self.limit.clone());
                let claims = self.claims.clone();
                let Ok(handle) = std::thread::Builder::new()
                    .name("pm-still-prefetch".into())
                    .spawn(move || {
                        // The queue's lock is held only while waiting for
                        // the next request, never while decoding.
                        let next = || asks.lock().ok().and_then(|a| a.recv().ok());
                        while let Some(key) = next() {
                            let skip = {
                                let mut c = claims.lock().unwrap();
                                let skip = c.dropped.remove(&key);
                                if !skip {
                                    c.started.insert(key.clone());
                                }
                                skip
                            };
                            let still = (!skip)
                                .then(|| decode(&key, &pool, &held, limit.load(Ordering::Acquire)))
                                .flatten();
                            claims.lock().unwrap().started.remove(&key);
                            if done.send((key, still)).is_err() {
                                break;
                            }
                        }
                    })
                else {
                    continue;
                };
                threads.push(handle);
            }
            if threads.is_empty() {
                return;
            }
            self.worker = Some(Worker {
                ask,
                answers,
                _threads: threads,
            });
        }
        if let Some(worker) = &self.worker
            && worker.ask.send(key.clone()).is_ok()
        {
            self.pending.insert(key, admitted);
        }
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
        self.held.fetch_sub(bytes_of(&still), Ordering::AcqRel);
        Some(still)
    }

    /// Waits for the still a worker has started decoding for `path` at
    /// `stamp` and `ask` (up to two seconds), so the frame about to draw it
    /// does not decode it a second time beside the worker. One still queued
    /// behind others (the playhead jumped) is the frame's to decode now: the
    /// worker skips it.
    pub(crate) fn finish_pending(&mut self, path: &Path, stamp: (u64, u128), ask: StillAsk) {
        self.drain();
        let key = (path.to_path_buf(), stamp.0, stamp.1, ask);
        if !self.pending.contains_key(&key) {
            return;
        }
        {
            let mut claims = self.claims.lock().unwrap();
            if !claims.started.contains(&key) {
                claims.dropped.insert(key);
                return;
            }
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.pending.contains_key(&key) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
            self.drain();
        }
    }

    /// Waits until the worker has answered every request (headless tests
    /// and performance runs; a frame never waits).
    pub(crate) fn settle(&mut self) {
        while !self.pending.is_empty() {
            std::thread::sleep(Duration::from_millis(2));
            self.drain();
        }
    }

    fn drain(&mut self) {
        let now = Instant::now();
        if let Some(worker) = &self.worker {
            while let Ok((key, still)) = worker.answers.try_recv() {
                let Some(still) = still else {
                    self.pending.remove(&key);
                    continue;
                };
                self.last_bytes = Some(bytes_of(&still.0));
                if self.pending.remove(&key).is_some() {
                    self.ready.insert(key, (still, now));
                } else {
                    // Dropped while it decoded (play stopped).
                    self.held.fetch_sub(bytes_of(&still.0), Ordering::AcqRel);
                }
            }
        }
        let (held, ttl) = (&self.held, self.ttl);
        self.ready.retain(|_, ((still, _), asked)| {
            let keep = now.duration_since(*asked) < ttl;
            if !keep {
                held.fetch_sub(bytes_of(still), Ordering::AcqRel);
            }
            keep
        });
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

    /// Admission comes before the decode: once a still's size is known,
    /// only the stills that fit beside the held and queued ones are queued
    /// (a worker's decode allocates the whole still before it is counted).
    /// While no size is known, one still at a time.
    #[test]
    fn only_stills_that_fit_are_decoded() {
        let (path, stamp) = fixture();
        let pool = FramePool::new(0);
        let one = bytes_of(&pm_media::image::read(&path, &pool).unwrap());
        let mut prefetch = StillPrefetch::with_limits(one * 2, READY_TTL);
        for n in 0..3u128 {
            prefetch.request(&path, (stamp.0, stamp.1 + n), StillAsk::default(), &pool);
        }
        assert_eq!(prefetch.pending.len(), 1, "no size known yet: one decode");
        settle(&mut prefetch);
        for n in 1..6u128 {
            prefetch.request(&path, (stamp.0, stamp.1 + n), StillAsk::default(), &pool);
        }
        assert_eq!(
            prefetch.pending.len(),
            1,
            "one more fits beside the held one"
        );
        settle(&mut prefetch);
        assert_eq!(prefetch.ready.len(), 2);
        assert_eq!(prefetch.held.load(Ordering::Acquire), one * 2);
    }

    /// A lower allowance gives held stills back at once (the oldest asked
    /// for first), down to nothing; play stopping drops them all, and
    /// stills still decoding when it stopped are dropped as they arrive.
    #[test]
    fn a_lower_allowance_or_a_stop_gives_stills_back_at_once() {
        let (path, stamp) = fixture();
        let pool = FramePool::new(0);
        let one = bytes_of(&pm_media::image::read(&path, &pool).unwrap());
        let mut prefetch = StillPrefetch::with_limits(one * 4, READY_TTL);
        for n in 0..4u128 {
            prefetch.request(&path, (stamp.0, stamp.1 + n), StillAsk::default(), &pool);
            settle(&mut prefetch);
        }
        assert_eq!(prefetch.held.load(Ordering::Acquire), one * 4);
        prefetch.set_limit(one * 2);
        assert_eq!(prefetch.held.load(Ordering::Acquire), one * 2);
        assert_eq!(prefetch.ready.len(), 2);
        assert!(
            prefetch
                .take(&path, (stamp.0, stamp.1 + 3), StillAsk::default(), None)
                .is_some(),
            "the latest asked for is kept"
        );
        prefetch.set_limit(0);
        assert_eq!(prefetch.held.load(Ordering::Acquire), 0);
        assert!(prefetch.ready.is_empty());

        prefetch.set_limit(one * 4);
        prefetch.request(&path, (stamp.0, stamp.1 + 9), StillAsk::default(), &pool);
        prefetch.clear();
        assert!(prefetch.pending.is_empty() && prefetch.ready.is_empty());
        // The decode that was running lands later and is not kept.
        std::thread::sleep(Duration::from_millis(500));
        prefetch.drain();
        assert!(prefetch.ready.is_empty());
        assert_eq!(prefetch.held.load(Ordering::Acquire), 0);
    }
}
