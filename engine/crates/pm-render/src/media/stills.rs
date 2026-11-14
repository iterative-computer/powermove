//! Stills decoded ahead of the playhead on a worker, so a still layer
//! entering playback does not decode on the frame that meets it.
//!
//! The worker runs the same [`pm_media::image::read`] the frame would (and
//! warms the per-stamp facts its probe reads); the frame takes the result
//! when the file's stamp still matches and decodes itself otherwise. A frame
//! that meets a still a worker is decoding at that moment waits for it
//! instead of decoding it a second time, but never past the frame's own
//! bound ([`StillPrefetch::wait_for_decoding`]): a read that stalls (a
//! network volume, a huge file) is given up on, the frame shows what it has,
//! and the answer is taken by a later frame when it lands
//! ([`StillPrefetch::landed`]). Only the decode moves: conversion and upload
//! stay on the frame, so the pixels are the ones a cold read gives.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pm_media::image::StillImage;
use pm_media::pool::FramePool;

/// A file at one stamp (length, modification time), asked for as a draw
/// reads it ([`StillAsk`]).
type StillKey = (PathBuf, u64, u128, StillAsk);

/// How the frame that draws a still reads it: the viewer's preview edge (the
/// longest side it shows the still at, when it draws previews), whether
/// the asset has Interpret Footage overrides, and which picture of a still
/// with a gain map it reads. The worker decodes what
/// `MediaCache::image_scaled` would for the same ask.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct StillAsk {
    pub edge: Option<u32>,
    pub interpreted: bool,
    pub rendition: pm_media::image::Rendition,
}

impl StillAsk {
    pub fn of(edge: Option<u32>, interpret: Option<&pm_media::interpret::Interpretation>) -> Self {
        StillAsk {
            edge,
            interpreted: interpret.is_some(),
            rendition: interpret.and_then(|i| i.gain_map).unwrap_or_default(),
        }
    }
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

/// The decode worker: requests (with the epoch they were asked in) in,
/// decoded stills out.
struct Worker {
    ask: Sender<(StillKey, u64)>,
    answers: Receiver<(StillKey, Option<Decoded>)>,
    _threads: Vec<JoinHandle<()>>,
}

/// Requests a worker is decoding now, and queued ones the frame decoded
/// itself (a worker skips those).
#[derive(Default)]
struct Claims {
    started: HashSet<StillKey>,
    dropped: HashSet<StillKey>,
    /// The still a frame waits for, or gave up waiting for: charged whatever
    /// the allowance once decoded (the frame needs it), and its answer moves
    /// [`Shared::landed`].
    wanted: Option<StillKey>,
}

/// Whether a frame got the decode it waited for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Waited {
    /// Nothing of this still was being decoded, or its answer arrived; a
    /// still only queued is the frame's to decode now (a worker skips it).
    Done,
    /// The decode is still running past the frame's bound: the frame shows
    /// what it has and a later frame takes the answer.
    Pending,
}

/// What the frame and the workers share.
#[derive(Default)]
struct Shared {
    /// Bytes reserved for stills being decoded, and of decoded stills ready
    /// or on their way to the frame.
    held: AtomicUsize,
    /// The allowance, read live at every reservation.
    limit: AtomicUsize,
    /// Rises when the allowance shrinks or play stops: work asked for in an
    /// earlier epoch is abandoned, its reservation given back.
    epoch: AtomicU64,
    /// A decode whose size its header does not give is admitted alone,
    /// beside the allowance (the one decode not bounded by it).
    unknown: AtomicBool,
    /// Held while a worker checks the epoch and sends, and while the frame
    /// moves the epoch on: no answer of an earlier epoch is sent after.
    fence: Mutex<()>,
    /// Bumped when a decode a frame gave up waiting for has answered, or was
    /// abandoned by a fence: the viewer draws the held frame again.
    landed: AtomicU64,
    #[cfg(test)]
    gate: (Mutex<bool>, std::sync::Condvar),
}

pub(crate) struct StillPrefetch {
    worker: Option<Worker>,
    claims: Arc<Mutex<Claims>>,
    /// Queued and decoding stills.
    pending: HashSet<StillKey>,
    /// Decoded stills and when they were last asked for.
    ready: HashMap<StillKey, (Decoded, Instant)>,
    shared: Arc<Shared>,
    ttl: Duration,
    /// The decode a frame gave up waiting for: later frames do not wait for
    /// it again (each would lose its bound to the same stalled read).
    abandoned: Option<StillKey>,
}

impl Default for StillPrefetch {
    fn default() -> Self {
        Self::with_limits(MAX_READY_BYTES, READY_TTL)
    }
}

fn bytes_of(still: &StillImage) -> usize {
    still.frame.format.visible_bytes()
}

/// A worker's answer for one request.
enum Answer {
    /// Decoded, and charged to the allowance.
    Decoded(Box<Decoded>),
    /// It did not fit (the frame decodes it itself).
    Refused,
    /// Asked for in an earlier epoch: nothing is sent.
    Abandoned,
}

/// Decodes `key` as its frame will read it, admitted before the decode at
/// the size its header gives for a whole decode (beside the held and
/// reserved stills, within the live allowance), or alone when the header
/// gives none; then charged its real size.
fn decode(
    key: &StillKey,
    pool: &FramePool,
    shared: &Shared,
    claims: &Mutex<Claims>,
    epoch: u64,
) -> Answer {
    let path = &key.0;
    let current = || shared.epoch.load(Ordering::Acquire) == epoch;
    let estimate = pm_media::image::imageio::decoded_bytes_estimate(path).ok();
    let reserved = match estimate {
        Some(size) => {
            let admitted = shared
                .held
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |h| {
                    (current() && h + size <= shared.limit.load(Ordering::Acquire))
                        .then_some(h + size)
                })
                .is_ok();
            if !admitted {
                return if current() {
                    Answer::Refused
                } else {
                    Answer::Abandoned
                };
            }
            size
        }
        None => {
            if shared
                .unknown
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return Answer::Refused;
            }
            0
        }
    };
    #[cfg(test)]
    {
        let (blocked, wake) = &shared.gate;
        let mut blocked = blocked.lock().unwrap();
        while *blocked {
            blocked = wake.wait(blocked).unwrap();
        }
    }
    // The frame's probe reads the file's facts first, once per stamp. Read
    // whole here, the still gives them, and is not decoded twice for them.
    // The file's facts are those of its default reading (a gain-map still
    // as HDR); a still read for its SDR base is decoded as such.
    let rendition = key.3.rendition;
    let default = rendition == pm_media::image::Rendition::default();
    let (facts, whole) = match super::known_still_facts(path) {
        Some(facts) => (facts, None),
        None => {
            let whole = pm_media::image::read(path, pool).ok();
            (super::still_facts_from(path, whole.as_ref()), whole)
        }
    };
    let whole = whole.filter(|_| default);
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
                None => pm_media::image::read_with(path, pool, rendition).map(|s| (s, None)),
            }),
        (None, Some(whole)) => Ok((whole, None)),
        (None, None) => pm_media::image::read_with(path, pool, rendition).map(|s| (s, None)),
    };
    if estimate.is_none() {
        shared.unknown.store(false, Ordering::Release);
    }
    let give_back = |bytes: usize| {
        shared.held.fetch_sub(bytes, Ordering::AcqRel);
    };
    let Ok(decoded) = decoded else {
        give_back(reserved);
        return if current() {
            Answer::Refused
        } else {
            Answer::Abandoned
        };
    };
    if !current() {
        give_back(reserved);
        return Answer::Abandoned;
    }
    // Charged its real size, within the allowance as it is now; the still a
    // frame waits for whatever the allowance (dropping it made that frame
    // decode the same file again).
    let size = bytes_of(&decoded.0);
    let wanted = claims.lock().unwrap().wanted.as_ref() == Some(key);
    let charged = shared
        .held
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |h| {
            let h = h - reserved;
            (wanted || h + size <= shared.limit.load(Ordering::Acquire)).then_some(h + size)
        })
        .is_ok();
    if !charged {
        give_back(reserved);
        return Answer::Refused;
    }
    Answer::Decoded(Box::new(decoded))
}

impl StillPrefetch {
    fn with_limits(limit: usize, ttl: Duration) -> Self {
        let shared = Shared::default();
        shared.limit.store(limit, Ordering::Release);
        Self {
            worker: None,
            claims: Default::default(),
            pending: HashSet::new(),
            ready: HashMap::new(),
            shared: Arc::new(shared),
            ttl,
            abandoned: None,
        }
    }

    /// Moves the epoch on: work asked for before is abandoned (queued work
    /// is skipped, decodes in flight give their reservation back and send
    /// nothing), and what the frame was waiting for is forgotten.
    fn fence(&mut self) {
        {
            let _fence = self.shared.fence.lock().unwrap();
            self.shared.epoch.fetch_add(1, Ordering::AcqRel);
        }
        // Answers sent before the fence moved.
        self.drain();
        let mut claims = self.claims.lock().unwrap();
        for key in self.pending.drain() {
            if !claims.started.contains(&key) {
                claims.dropped.insert(key);
            }
        }
        // A frame held for a decode now abandoned draws again (and decodes
        // the still itself).
        claims.wanted = None;
        if self.abandoned.take().is_some() {
            self.shared.landed.fetch_add(1, Ordering::AcqRel);
        }
    }

    pub(crate) fn limit(&self) -> usize {
        self.shared.limit.load(Ordering::Acquire)
    }

    /// Holds up to `bytes` of decoded stills: a lower allowance cancels the
    /// queued and in-flight reservations and gives the stills held beyond it
    /// back now, those asked for longest ago first.
    pub(crate) fn set_limit(&mut self, bytes: usize) {
        let shrinks = bytes < self.shared.limit.swap(bytes, Ordering::AcqRel);
        if shrinks {
            self.fence();
        }
        self.drain();
        let mut by_age: Vec<(StillKey, Instant)> = self
            .ready
            .iter()
            .map(|(k, (_, asked))| (k.clone(), *asked))
            .collect();
        by_age.sort_by_key(|(_, asked)| *asked);
        for (key, _) in by_age {
            if self.held() <= bytes {
                break;
            }
            if let Some(((still, _), _)) = self.ready.remove(&key) {
                self.shared
                    .held
                    .fetch_sub(bytes_of(&still), Ordering::AcqRel);
            }
        }
    }

    /// Bytes reserved for or held by stills decoded ahead.
    pub(crate) fn held(&self) -> usize {
        self.shared.held.load(Ordering::Acquire)
    }

    /// Play stopped: every still decoded ahead is given back, queued ones
    /// are not decoded, and ones decoding now give their reservation back
    /// on the worker and are never sent.
    pub(crate) fn clear(&mut self) {
        self.fence();
        for ((still, _), _) in std::mem::take(&mut self.ready).into_values() {
            self.shared
                .held
                .fetch_sub(bytes_of(&still), Ordering::AcqRel);
        }
    }

    /// Queues `path` at `stamp` for decoding as `ask` reads it, unless it is
    /// queued or ready. A worker admits it against the allowance before
    /// decoding it.
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
            let (ask, asks) = channel::<(StillKey, u64)>();
            let asks = Arc::new(Mutex::new(asks));
            let (done, answers) = channel();
            let mut threads = Vec::new();
            for _ in 0..workers() {
                let (asks, done) = (asks.clone(), done.clone());
                let pool = pool.clone();
                let shared = self.shared.clone();
                let claims = self.claims.clone();
                let Ok(handle) = std::thread::Builder::new()
                    .name("pm-still-prefetch".into())
                    .spawn(move || {
                        // The queue's lock is held only while waiting for
                        // the next request, never while decoding.
                        let next = || asks.lock().ok().and_then(|a| a.recv().ok());
                        while let Some((key, epoch)) = next() {
                            let skip = {
                                let mut c = claims.lock().unwrap();
                                let skip = c.dropped.remove(&key);
                                if !skip {
                                    c.started.insert(key.clone());
                                }
                                skip
                            };
                            let answer = if skip {
                                Answer::Refused
                            } else {
                                decode(&key, &pool, &shared, &claims, epoch)
                            };
                            claims.lock().unwrap().started.remove(&key);
                            let fence = shared.fence.lock().unwrap();
                            let still = match answer {
                                _ if shared.epoch.load(Ordering::Acquire) != epoch => {
                                    if let Answer::Decoded(decoded) = answer {
                                        let (still, _) = *decoded;
                                        shared.held.fetch_sub(bytes_of(&still), Ordering::AcqRel);
                                    }
                                    continue;
                                }
                                Answer::Abandoned => continue,
                                Answer::Refused => None,
                                Answer::Decoded(decoded) => Some(*decoded),
                            };
                            let wanted = claims.lock().unwrap().wanted.as_ref() == Some(&key);
                            let sent = done.send((key, still));
                            // A frame gave up on it: the viewer draws again.
                            if wanted {
                                shared.landed.fetch_add(1, Ordering::AcqRel);
                            }
                            drop(fence);
                            if sent.is_err() {
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
        let epoch = self.shared.epoch.load(Ordering::Acquire);
        if let Some(worker) = &self.worker
            && worker.ask.send((key.clone(), epoch)).is_ok()
        {
            self.pending.insert(key);
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
        self.shared
            .held
            .fetch_sub(bytes_of(&still), Ordering::AcqRel);
        Some(still)
    }

    /// Waits while a worker is decoding `path` at `stamp` as `ask` reads it,
    /// so the frame that needs this still now gets that decode (and the
    /// file's facts with it) instead of starting the same one beside it.
    /// Never past `until`, nor once `cancelled` says the frame is superseded:
    /// then the decode is [`Waited::Pending`], and no later frame waits for
    /// it again until it answers. A still only queued behind others (the
    /// playhead jumped) is the frame's to decode now: the worker skips it.
    pub(crate) fn wait_for_decoding(
        &mut self,
        path: &Path,
        stamp: (u64, u128),
        ask: StillAsk,
        until: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Waited {
        self.drain();
        let key = (path.to_path_buf(), stamp.0, stamp.1, ask);
        if !self.pending.contains(&key) {
            return Waited::Done;
        }
        {
            let mut claims = self.claims.lock().unwrap();
            if !claims.started.contains(&key) {
                claims.dropped.insert(key);
                return Waited::Done;
            }
            if self.abandoned.as_ref() == Some(&key) {
                return Waited::Pending;
            }
            claims.wanted = Some(key.clone());
        }
        while self.pending.contains(&key) {
            if Instant::now() >= until || cancelled() {
                // The worker keeps the answer for a later frame.
                self.abandoned = Some(key);
                return Waited::Pending;
            }
            std::thread::sleep(Duration::from_millis(1));
            self.drain();
        }
        let mut claims = self.claims.lock().unwrap();
        if claims.wanted.as_ref() == Some(&key) {
            claims.wanted = None;
        }
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

    /// Changes whenever a decode a frame gave up on answers (or is
    /// abandoned).
    pub(crate) fn landed(&self) -> u64 {
        self.shared.landed.load(Ordering::Acquire)
    }

    /// Tests: holds every decode just before it starts reading, until
    /// released.
    #[cfg(test)]
    pub(crate) fn hold_decodes(&self, on: bool) {
        (self.decode_gate())(on);
    }

    /// Tests: [`Self::hold_decodes`] from another thread.
    #[cfg(test)]
    pub(crate) fn decode_gate(&self) -> impl Fn(bool) + Send + 'static {
        let shared = self.shared.clone();
        move |on| {
            *shared.gate.0.lock().unwrap() = on;
            shared.gate.1.notify_all();
        }
    }

    /// Tests: a worker is inside a decode.
    #[cfg(test)]
    pub(crate) fn decoding(&self) -> bool {
        !self.claims.lock().unwrap().started.is_empty()
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
                let pending = self.pending.remove(&key);
                if self.abandoned.as_ref() == Some(&key) {
                    self.abandoned = None;
                    let mut claims = self.claims.lock().unwrap();
                    if claims.wanted.as_ref() == Some(&key) {
                        claims.wanted = None;
                    }
                }
                match still {
                    Some(still) if pending => {
                        self.ready.insert(key, (still, now));
                    }
                    // Forgotten while it was on its way.
                    Some(still) => {
                        self.shared
                            .held
                            .fetch_sub(bytes_of(&still.0), Ordering::AcqRel);
                    }
                    None => {}
                }
            }
        }
        let (held, ttl) = (&self.shared.held, self.ttl);
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
            ..Default::default()
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
            ..Default::default()
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
        assert!(prefetch.held() > 0);
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
        assert_eq!(prefetch.held(), 0);
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
        assert_eq!(prefetch.held(), 0);
    }

    /// Admission comes before the decode, at the size the file's header
    /// gives: only the stills that fit beside the held and reserved ones
    /// are decoded, however many workers take requests at once.
    #[test]
    fn only_stills_that_fit_are_decoded() {
        let (path, stamp) = fixture();
        let pool = FramePool::new(0);
        let one = bytes_of(&pm_media::image::read(&path, &pool).unwrap());
        assert_eq!(
            pm_media::image::imageio::decoded_bytes_estimate(&path).unwrap(),
            one,
            "the header gives the decoded size"
        );
        let mut prefetch = StillPrefetch::with_limits(one * 2, READY_TTL);
        for n in 0..5u128 {
            prefetch.request(&path, (stamp.0, stamp.1 + n), StillAsk::default(), &pool);
        }
        settle(&mut prefetch);
        assert_eq!(prefetch.ready.len(), 2);
        assert_eq!(prefetch.held(), one * 2);
    }

    /// A lower allowance gives held stills back at once, the oldest asked
    /// for first, down to nothing.
    #[test]
    fn a_lower_allowance_gives_stills_back_at_once() {
        let (path, stamp) = fixture();
        let pool = FramePool::new(0);
        let one = bytes_of(&pm_media::image::read(&path, &pool).unwrap());
        let mut prefetch = StillPrefetch::with_limits(one * 4, READY_TTL);
        for n in 0..4u128 {
            prefetch.request(&path, (stamp.0, stamp.1 + n), StillAsk::default(), &pool);
            settle(&mut prefetch);
        }
        assert_eq!(prefetch.held(), one * 4);
        prefetch.set_limit(one * 2);
        assert_eq!(prefetch.held(), one * 2);
        assert_eq!(prefetch.ready.len(), 2);
        assert!(
            prefetch
                .take(&path, (stamp.0, stamp.1 + 3), StillAsk::default(), None)
                .is_some(),
            "the latest asked for is kept"
        );
        prefetch.set_limit(0);
        assert_eq!(prefetch.held(), 0);
        assert!(prefetch.ready.is_empty());
    }

    /// A decode in flight when the allowance shrinks to nothing, or when
    /// play stops, gives its reservation back on the worker and is never
    /// sent: nothing is held and nothing waits in the channel, with no
    /// later call from the frame.
    #[test]
    fn a_decode_in_flight_is_abandoned_when_the_allowance_shrinks_or_play_stops() {
        let (path, stamp) = fixture();
        let pool = FramePool::new(0);
        let one = bytes_of(&pm_media::image::read(&path, &pool).unwrap());
        for stop in [false, true] {
            let mut prefetch = StillPrefetch::with_limits(one * 4, READY_TTL);
            *prefetch.shared.gate.0.lock().unwrap() = true;
            prefetch.request(&path, stamp, StillAsk::default(), &pool);
            let deadline = Instant::now() + Duration::from_secs(10);
            while prefetch.held() == 0 {
                assert!(Instant::now() < deadline, "the decode never started");
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(prefetch.held(), one, "reserved before decoding");
            if stop {
                prefetch.clear();
            } else {
                prefetch.set_limit(0);
            }
            assert!(prefetch.pending.is_empty());
            *prefetch.shared.gate.0.lock().unwrap() = false;
            prefetch.shared.gate.1.notify_all();
            while prefetch.held() != 0 {
                assert!(
                    Instant::now() < deadline,
                    "the reservation was kept (stop {stop})"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            while !prefetch.claims.lock().unwrap().started.is_empty() {
                assert!(Instant::now() < deadline, "the decode never ended");
                std::thread::sleep(Duration::from_millis(1));
            }
            // Past its decision whether to send.
            std::thread::sleep(Duration::from_millis(100));
            assert_eq!(prefetch.held(), 0, "stop {stop}");
            assert!(
                prefetch
                    .worker
                    .as_ref()
                    .unwrap()
                    .answers
                    .try_recv()
                    .is_err(),
                "an abandoned still was sent (stop {stop})"
            );
        }
    }

    /// Holds every decode until the returned guard drops.
    struct Held(Arc<Shared>);
    impl Drop for Held {
        fn drop(&mut self) {
            *self.0.gate.0.lock().unwrap() = false;
            self.0.gate.1.notify_all();
        }
    }
    fn hold(prefetch: &StillPrefetch) -> Held {
        prefetch.hold_decodes(true);
        Held(prefetch.shared.clone())
    }

    fn started(prefetch: &StillPrefetch, n: usize) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while prefetch.claims.lock().unwrap().started.len() < n {
            assert!(Instant::now() < deadline, "the workers start");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// A frame that needs the still a worker is decoding at that moment
    /// waits for that decode and takes its result; one only queued (every
    /// worker busy) is not waited for, and the worker skips it.
    #[test]
    fn a_frame_waits_for_the_decode_in_progress() {
        let (path, stamp) = fixture();
        let pool = FramePool::new(0);
        let one = bytes_of(&pm_media::image::read(&path, &pool).unwrap());
        let n = workers();
        let mut prefetch = StillPrefetch::with_limits(one * (n + 2), READY_TTL);
        let ask = StillAsk::default();
        let held = hold(&prefetch);
        for i in 0..=n as u128 {
            prefetch.request(&path, (stamp.0, stamp.1 + i), ask, &pool);
        }
        started(&prefetch, n);
        let queued = (0..=n as u128)
            .map(|i| (stamp.0, stamp.1 + i))
            .find(|s| {
                !prefetch
                    .claims
                    .lock()
                    .unwrap()
                    .started
                    .contains(&(path.clone(), s.0, s.1, ask))
            })
            .unwrap();
        let never = Instant::now() + Duration::from_secs(60);
        let at = Instant::now();
        assert_eq!(
            prefetch.wait_for_decoding(&path, queued, ask, never, &|| false),
            Waited::Done,
            "only queued: not waited for"
        );
        assert!(at.elapsed() < Duration::from_secs(1));
        let decoding = (stamp.0, stamp.1 + if queued.1 == stamp.1 { 1 } else { 0 });
        std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(100));
                drop(held);
            });
            assert_eq!(
                prefetch.wait_for_decoding(&path, decoding, ask, never, &|| false),
                Waited::Done
            );
        });
        assert!(
            prefetch.take(&path, decoding, ask, None).is_some(),
            "the decode in progress reached the frame"
        );
        settle(&mut prefetch);
        assert!(
            prefetch.take(&path, queued, ask, None).is_none(),
            "the worker skipped the still the frame took over"
        );
        assert!(prefetch.claims.lock().unwrap().wanted.is_none());
    }

    /// A decode that never returns holds a frame no longer than its bound:
    /// the frame is told it is pending, later frames do not wait for it
    /// again, and when it does answer the answer is kept for the next frame
    /// and `landed` changes.
    #[test]
    fn a_stalled_decode_holds_a_frame_no_longer_than_its_bound() {
        let (path, stamp) = fixture();
        let pool = FramePool::new(0);
        let mut prefetch = StillPrefetch::default();
        let ask = StillAsk::default();
        let held = hold(&prefetch);
        prefetch.request(&path, stamp, ask, &pool);
        started(&prefetch, 1);
        let landed = prefetch.landed();
        let at = Instant::now();
        assert_eq!(
            prefetch
                .wait_for_decoding(&path, stamp, ask, at + Duration::from_millis(40), &|| false),
            Waited::Pending
        );
        let waited = at.elapsed();
        assert!(
            waited < Duration::from_millis(1000),
            "the frame waited {waited:?} for a decode that does not return"
        );
        assert!(prefetch.pending(&path, stamp));
        // A later frame does not spend its own bound on the same read.
        let at = Instant::now();
        let far = at + Duration::from_secs(60);
        assert_eq!(
            prefetch.wait_for_decoding(&path, stamp, ask, far, &|| false),
            Waited::Pending
        );
        assert!(at.elapsed() < Duration::from_millis(500));
        assert!(prefetch.take(&path, stamp, ask, None).is_none());
        // The read returns: the answer waits for the next frame.
        drop(held);
        let deadline = Instant::now() + Duration::from_secs(30);
        while prefetch.landed() == landed {
            assert!(Instant::now() < deadline, "the decode lands");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!prefetch.pending(&path, stamp));
        assert!(
            prefetch.take(&path, stamp, ask, None).is_some(),
            "kept for the frame that gave up on it"
        );
        assert!(prefetch.claims.lock().unwrap().wanted.is_none());
    }

    /// An exact frame may wait longer, but stops waiting as soon as it is
    /// superseded (a seek, a new revision, the viewer closing).
    #[test]
    fn a_superseded_frame_stops_waiting_for_a_stalled_decode() {
        let (path, stamp) = fixture();
        let pool = FramePool::new(0);
        let mut prefetch = StillPrefetch::default();
        let ask = StillAsk::default();
        let held = hold(&prefetch);
        prefetch.request(&path, stamp, ask, &pool);
        started(&prefetch, 1);
        let superseded = Arc::new(AtomicBool::new(false));
        let flag = superseded.clone();
        let seek = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            flag.store(true, Ordering::Release);
        });
        let at = Instant::now();
        let far = at + Duration::from_secs(60);
        assert_eq!(
            prefetch.wait_for_decoding(&path, stamp, ask, far, &|| superseded
                .load(Ordering::Acquire)),
            Waited::Pending
        );
        let waited = at.elapsed();
        assert!(
            waited < Duration::from_secs(5),
            "a superseded frame waited {waited:?}"
        );
        seek.join().unwrap();
        drop(held);
        settle(&mut prefetch);
        assert!(prefetch.take(&path, stamp, ask, None).is_some());
    }

    /// A release (play stopped, budget zero, idle) lets go of every decoded
    /// still and of the answer of a decode running during it; a frame held
    /// for that decode is told to draw again.
    #[test]
    fn a_release_lets_go_of_held_stills_and_late_answers() {
        let (path, stamp) = fixture();
        let pool = FramePool::new(0);
        let mut prefetch = StillPrefetch::default();
        let ask = StillAsk::default();
        prefetch.request(&path, stamp, ask, &pool);
        settle(&mut prefetch);
        assert!(prefetch.held() > 0, "a decoded still is counted");
        prefetch.clear();
        assert_eq!(prefetch.held(), 0);
        assert!(prefetch.ready.is_empty());
        // Released while a worker decodes, with a frame held for it.
        let held = hold(&prefetch);
        let late = (stamp.0, stamp.1 + 1);
        prefetch.request(&path, late, ask, &pool);
        started(&prefetch, 1);
        let at = Instant::now();
        assert_eq!(
            prefetch.wait_for_decoding(&path, late, ask, at, &|| false),
            Waited::Pending
        );
        let landed = prefetch.landed();
        prefetch.clear();
        assert_ne!(prefetch.landed(), landed, "the held frame draws again");
        assert!(!prefetch.pending(&path, late));
        drop(held);
        let deadline = Instant::now() + Duration::from_secs(30);
        while prefetch.decoding() {
            assert!(Instant::now() < deadline, "the decode ends");
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(Duration::from_millis(50));
        prefetch.drain();
        assert!(prefetch.ready.is_empty(), "a late answer was kept");
        assert_eq!(prefetch.held(), 0);
    }
}
