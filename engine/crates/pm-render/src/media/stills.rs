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

/// A file at one stamp (length, modification time).
type StillKey = (PathBuf, u64, u128);

/// Stills queued or waiting for their frame at once.
const MAX_READY: usize = 8;
/// The decoded bytes held at most, counted from the moment the worker
/// decodes a still, so stills on their way to the frame count too (a UHD
/// 16-bit RGBA still is 66 MB).
const MAX_READY_BYTES: usize = 256 << 20;
/// A decoded still no frame took, or asked for again, within this long is
/// dropped: the playhead went elsewhere.
const READY_TTL: Duration = Duration::from_secs(4);

/// The decode worker: requests in, decoded stills out.
struct Worker {
    ask: Sender<StillKey>,
    answers: Receiver<(StillKey, Option<StillImage>)>,
    _thread: JoinHandle<()>,
}

pub(crate) struct StillPrefetch {
    worker: Option<Worker>,
    pending: HashSet<StillKey>,
    /// Decoded stills and when they were last asked for.
    ready: HashMap<StillKey, (StillImage, Instant)>,
    /// Bytes of decoded stills, ready or still in the channel.
    held: Arc<AtomicUsize>,
    limit: usize,
    ttl: Duration,
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
            limit,
            ttl,
        }
    }

    /// Queues `path` at `stamp` for decoding, unless it is queued or ready.
    pub(crate) fn request(&mut self, path: &Path, stamp: (u64, u128), pool: &FramePool) {
        self.drain();
        let key = (path.to_path_buf(), stamp.0, stamp.1);
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
            let Ok(handle) = std::thread::Builder::new()
                .name("pm-still-prefetch".into())
                .spawn(move || {
                    for key in asks {
                        // The frame's probe reads the file's facts first,
                        // once per stamp: warm that cache here too.
                        let _ = super::still_facts(&key.0);
                        // Only a still that fits beside the held ones is
                        // kept; the frame decodes the others itself.
                        let still = pm_media::image::read(&key.0, &pool).ok().filter(|still| {
                            let size = bytes(still);
                            held.fetch_update(Ordering::AcqRel, Ordering::Acquire, |h| {
                                (h + size <= limit).then_some(h + size)
                            })
                            .is_ok()
                        });
                        if done.send((key, still)).is_err() {
                            break;
                        }
                    }
                })
            else {
                return;
            };
            self.worker = Some(Worker {
                ask,
                answers,
                _thread: handle,
            });
        }
        if let Some(worker) = &self.worker
            && worker.ask.send(key.clone()).is_ok()
        {
            self.pending.insert(key);
        }
    }

    /// The still decoded for `path` at `stamp`, if the worker has it.
    pub(crate) fn take(&mut self, path: &Path, stamp: (u64, u128)) -> Option<StillImage> {
        self.drain();
        let (still, _) = self.ready.remove(&(path.to_path_buf(), stamp.0, stamp.1))?;
        self.held.fetch_sub(bytes(&still), Ordering::AcqRel);
        Some(still)
    }

    fn drain(&mut self) {
        let now = Instant::now();
        if let Some(worker) = &self.worker {
            while let Ok((key, still)) = worker.answers.try_recv() {
                self.pending.remove(&key);
                if let Some(still) = still {
                    self.ready.insert(key, (still, now));
                }
            }
        }
        let (held, ttl) = (&self.held, self.ttl);
        self.ready.retain(|_, (still, asked)| {
            let keep = now.duration_since(*asked) < ttl;
            if !keep {
                held.fetch_sub(bytes(still), Ordering::AcqRel);
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
        prefetch.request(&path, stamp, &pool);
        let started = std::time::Instant::now();
        let still = loop {
            if let Some(still) = prefetch.take(&path, stamp) {
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
        assert!(prefetch.take(&path, stamp).is_none(), "taken once");
        prefetch.request(&path, stamp, &pool);
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(prefetch.take(&path, (stamp.0 + 1, stamp.1)).is_none());
    }

    fn fixture() -> (PathBuf, (u64, u128)) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/video/vp9-10bit-709-limited.paused.rgba.png");
        let stamp = crate::media::stamp(&path).expect("fixture exists");
        (path, stamp)
    }

    fn settle(prefetch: &mut StillPrefetch) {
        let started = std::time::Instant::now();
        while !prefetch.pending.is_empty() {
            assert!(started.elapsed().as_secs() < 30, "the worker answers");
            std::thread::sleep(Duration::from_millis(5));
            prefetch.drain();
        }
    }

    /// A still no frame takes leaves after the TTL, with its bytes, and
    /// stops holding a place, so later requests are queued again.
    #[test]
    fn unconsumed_stills_expire_and_free_their_places() {
        let (path, stamp) = fixture();
        let pool = FramePool::new(0);
        let mut prefetch = StillPrefetch::with_limits(MAX_READY_BYTES, Duration::from_millis(150));
        prefetch.request(&path, stamp, &pool);
        settle(&mut prefetch);
        assert_eq!(prefetch.ready.len(), 1);
        assert!(prefetch.held.load(Ordering::Acquire) > 0);
        // Asking again keeps it.
        std::thread::sleep(Duration::from_millis(100));
        prefetch.request(&path, stamp, &pool);
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
            prefetch.request(&path, (stamp.0, stamp.1 + u128::from(n) + 1), &pool);
        }
        settle(&mut prefetch);
        assert_eq!(prefetch.ready.len(), MAX_READY);
        std::thread::sleep(Duration::from_millis(200));
        prefetch.request(&path, stamp, &pool);
        settle(&mut prefetch);
        assert!(
            prefetch.take(&path, stamp).is_some(),
            "a place was free again"
        );
        assert_eq!(prefetch.ready.len(), 0);
        assert_eq!(prefetch.held.load(Ordering::Acquire), 0);
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
            prefetch.request(&path, (stamp.0, stamp.1 + n), &pool);
        }
        settle(&mut prefetch);
        assert_eq!(prefetch.ready.len(), 2, "two fit");
        assert_eq!(prefetch.held.load(Ordering::Acquire), one * 2);
        assert!(prefetch.take(&path, stamp).is_some());
        assert_eq!(prefetch.held.load(Ordering::Acquire), one);
    }
}
