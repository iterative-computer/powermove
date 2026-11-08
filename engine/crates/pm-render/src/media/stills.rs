//! Stills decoded ahead of the playhead on a worker, so a still layer
//! entering playback does not decode on the frame that meets it.
//!
//! The worker runs the same [`pm_media::image::read`] the frame would (and
//! warms the per-stamp facts its probe reads); the frame takes the result
//! when the file's stamp still matches and decodes itself otherwise. Only the decode moves: conversion and upload stay on
//! the frame, so the pixels are the ones a cold read gives.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use pm_media::image::StillImage;
use pm_media::pool::FramePool;

/// A file at one stamp (length, modification time).
type StillKey = (PathBuf, u64, u128);

/// Decoded stills kept waiting for their frame at once.
const MAX_READY: usize = 8;
/// Their bytes at most (a UHD 16-bit RGBA still is 66 MB).
const MAX_READY_BYTES: usize = 256 << 20;

/// The decode worker: requests in, decoded stills out.
struct Worker {
    ask: Sender<StillKey>,
    answers: Receiver<(StillKey, Option<StillImage>)>,
    _thread: JoinHandle<()>,
}

#[derive(Default)]
pub(crate) struct StillPrefetch {
    worker: Option<Worker>,
    pending: HashSet<StillKey>,
    ready: HashMap<StillKey, StillImage>,
}

fn bytes(still: &StillImage) -> usize {
    still.frame.format.visible_bytes()
}

impl StillPrefetch {
    /// Queues `path` at `stamp` for decoding, unless it is queued or ready.
    pub(crate) fn request(&mut self, path: &Path, stamp: (u64, u128), pool: &FramePool) {
        self.drain();
        let key = (path.to_path_buf(), stamp.0, stamp.1);
        if self.pending.contains(&key)
            || self.ready.contains_key(&key)
            || self.pending.len() + self.ready.len() >= MAX_READY
        {
            return;
        }
        if self.worker.is_none() {
            let (ask, asks) = channel::<StillKey>();
            let (done, answers) = channel();
            let pool = pool.clone();
            let Ok(handle) = std::thread::Builder::new()
                .name("pm-still-prefetch".into())
                .spawn(move || {
                    for key in asks {
                        // The frame's probe reads the file's facts first,
                        // once per stamp: warm that cache here too.
                        let _ = super::still_facts(&key.0);
                        let still = pm_media::image::read(&key.0, &pool).ok();
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
        self.ready.remove(&(path.to_path_buf(), stamp.0, stamp.1))
    }

    fn drain(&mut self) {
        let Some(worker) = &self.worker else {
            return;
        };
        while let Ok((key, still)) = worker.answers.try_recv() {
            self.pending.remove(&key);
            if let Some(still) = still {
                let held: usize = self.ready.values().map(bytes).sum();
                if held + bytes(&still) <= MAX_READY_BYTES {
                    self.ready.insert(key, still);
                }
            }
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
}
