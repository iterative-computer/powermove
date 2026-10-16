//! The single-producer, single-consumer ring between the render thread and
//! the device callback.
//!
//! The ring holds chunks of up to [`CHUNK`] device frames. Each chunk is
//! linear in media time and carries a [`Tag`]: where in the media its first
//! frame is, in which transport epoch, and in what state. The callback never
//! allocates, locks, or waits: it copies frames out, skips chunks of an
//! epoch the transport has left, and outputs silence when the ring is empty
//! (an underrun, counted).

use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

use pm_protocol::clock::TransportState;
use pm_protocol::common::TickRange;

/// Frames per chunk.
pub const CHUNK: usize = 256;
/// The widest device the ring carries.
pub const MAX_CHANNELS: usize = 8;

/// Where a chunk sits in the media.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tag {
    /// Media time of the chunk's first frame, ticks.
    pub media_ticks: i64,
    pub epoch: u32,
    pub state: TransportState,
    /// Valid frames in the chunk.
    pub frames: u32,
    /// The loop the clock line wraps by from this chunk on (`None` outside
    /// a loop: before it, past it, in a grain).
    pub loop_range: Option<TickRange>,
}

impl Default for Tag {
    fn default() -> Self {
        Tag {
            media_ticks: 0,
            epoch: 0,
            state: TransportState::Stopped,
            frames: 0,
            loop_range: None,
        }
    }
}

struct Slot {
    tag: UnsafeCell<Tag>,
    data: UnsafeCell<[f32; CHUNK * MAX_CHANNELS]>,
}

/// Shared state. `head` counts chunks written, `tail` chunks consumed.
pub struct Ring {
    slots: Box<[Slot]>,
    channels: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
    /// Chunks of older epochs are skipped by the consumer.
    epoch: AtomicU32,
    underruns: AtomicU64,
    /// Frames the consumer has played (all epochs).
    played: AtomicU64,
}

// The producer and the consumer touch disjoint slots, ordered by the
// acquire/release pairs on `head` and `tail`.
unsafe impl Sync for Ring {}
unsafe impl Send for Ring {}

impl Ring {
    /// A ring of `chunks` chunks of `channels` interleaved channels.
    pub fn new(chunks: usize, channels: usize) -> Arc<Ring> {
        assert!(
            (1..=MAX_CHANNELS).contains(&channels),
            "1 to {MAX_CHANNELS} channels"
        );
        let slots = (0..chunks.max(2))
            .map(|_| Slot {
                tag: UnsafeCell::new(Tag::default()),
                data: UnsafeCell::new([0.0; CHUNK * MAX_CHANNELS]),
            })
            .collect();
        Arc::new(Ring {
            slots,
            channels,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            epoch: AtomicU32::new(0),
            underruns: AtomicU64::new(0),
            played: AtomicU64::new(0),
        })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Chunks written and not yet consumed.
    pub fn queued(&self) -> usize {
        self.head.load(Ordering::Acquire) - self.tail.load(Ordering::Acquire)
    }

    pub fn epoch(&self) -> u32 {
        self.epoch.load(Ordering::Acquire)
    }

    /// Moves to a new epoch: queued chunks of older epochs will not play.
    pub fn set_epoch(&self, epoch: u32) {
        self.epoch.store(epoch, Ordering::Release);
    }

    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }

    pub fn played_frames(&self) -> u64 {
        self.played.load(Ordering::Relaxed)
    }

    /// Producer: writes a chunk (`interleaved` holds `tag.frames` frames).
    /// Returns false when the ring is full.
    pub fn push(&self, tag: Tag, interleaved: &[f32]) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head - tail >= self.slots.len() {
            return false;
        }
        let frames = (tag.frames as usize).min(CHUNK);
        let slot = &self.slots[head % self.slots.len()];
        // SAFETY: the consumer reads this slot only after `head` passes it,
        // and the producer is the only writer.
        unsafe {
            let data = &mut *slot.data.get();
            data[..frames * self.channels].copy_from_slice(&interleaved[..frames * self.channels]);
            *slot.tag.get() = Tag {
                frames: frames as u32,
                ..tag
            };
        }
        self.head.store(head + 1, Ordering::Release);
        true
    }
}

/// The device callback's side: a read position inside the current chunk.
pub struct Consumer {
    ring: Arc<Ring>,
    offset: usize,
}

/// What the start of a device buffer plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Heard {
    pub tag: Tag,
    /// Frames into the tag's chunk.
    pub offset: usize,
}

impl Consumer {
    pub fn new(ring: Arc<Ring>) -> Consumer {
        Consumer { ring, offset: 0 }
    }

    pub fn ring(&self) -> &Arc<Ring> {
        &self.ring
    }

    /// Fills `out` (interleaved, `device_channels` wide) from the ring.
    /// Device channels beyond the ring's are silent. Returns what the first
    /// frame plays, when it plays media.
    pub fn fill(&mut self, out: &mut [f32], device_channels: usize) -> Option<Heard> {
        let ring = &*self.ring;
        let rc = ring.channels;
        let frames = out.len() / device_channels.max(1);
        out.fill(0.0);
        let current = ring.epoch.load(Ordering::Acquire);
        let mut first: Option<Heard> = None;
        let mut done = 0;
        while done < frames {
            let tail = ring.tail.load(Ordering::Relaxed);
            let head = ring.head.load(Ordering::Acquire);
            if tail == head {
                ring.underruns.fetch_add(1, Ordering::Relaxed);
                break;
            }
            let slot = &ring.slots[tail % ring.slots.len()];
            // SAFETY: `head` (acquired) is past this slot, so the producer
            // finished writing it and will not touch it until `tail` passes.
            let (tag, data) = unsafe { (*slot.tag.get(), &*slot.data.get()) };
            if tag.epoch < current || self.offset >= tag.frames as usize {
                self.offset = 0;
                ring.tail.store(tail + 1, Ordering::Release);
                continue;
            }
            if first.is_none() {
                first = Some(Heard {
                    tag,
                    offset: self.offset,
                });
            }
            let take = (tag.frames as usize - self.offset).min(frames - done);
            for i in 0..take {
                let src = &data[(self.offset + i) * rc..(self.offset + i + 1) * rc];
                let dst = &mut out[(done + i) * device_channels..(done + i + 1) * device_channels];
                for (d, s) in dst.iter_mut().zip(src) {
                    *d = *s;
                }
            }
            done += take;
            self.offset += take;
            if self.offset >= tag.frames as usize {
                self.offset = 0;
                ring.tail.store(tail + 1, Ordering::Release);
            }
        }
        ring.played.fetch_add(frames as u64, Ordering::Relaxed);
        first
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(media: i64, epoch: u32, frames: u32) -> Tag {
        Tag {
            media_ticks: media,
            epoch,
            state: TransportState::Playing,
            frames,
            loop_range: None,
        }
    }

    #[test]
    fn frames_flow_in_order_and_old_epochs_are_skipped() {
        let ring = Ring::new(4, 2);
        let mut c = Consumer::new(ring.clone());
        let a: Vec<f32> = (0..CHUNK * 2).map(|i| i as f32).collect();
        assert!(ring.push(tag(0, 0, CHUNK as u32), &a));
        assert!(ring.push(tag(1000, 0, 10), &a));
        let mut out = vec![0f32; 100 * 2];
        let h = c.fill(&mut out, 2).unwrap();
        assert_eq!(h.tag.media_ticks, 0);
        assert_eq!(out[2], 2.0);
        let h = c.fill(&mut out, 2).unwrap();
        assert_eq!(h.offset, 100);
        // An epoch change drops what is queued.
        ring.set_epoch(1);
        assert!(ring.push(tag(5000, 1, CHUNK as u32), &a));
        let h = c.fill(&mut out, 2).unwrap();
        assert_eq!(h.tag.epoch, 1);
        assert_eq!(h.tag.media_ticks, 5000);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[3], 3.0);
    }

    #[test]
    fn full_and_empty() {
        let ring = Ring::new(2, 1);
        let a = vec![0.5f32; CHUNK];
        assert!(ring.push(tag(0, 0, CHUNK as u32), &a));
        assert!(ring.push(tag(0, 0, CHUNK as u32), &a));
        assert!(!ring.push(tag(0, 0, CHUNK as u32), &a));
        let mut c = Consumer::new(ring.clone());
        let mut out = vec![0f32; CHUNK * 3];
        c.fill(&mut out, 1);
        assert_eq!(ring.underruns(), 1);
        assert_eq!(out[CHUNK * 2 + 5], 0.0);
        assert_eq!(out[CHUNK + 5], 0.5);
    }

    #[test]
    fn threads() {
        let ring = Ring::new(8, 1);
        let producer = ring.clone();
        let total = 2000usize;
        let h = std::thread::spawn(move || {
            let mut n = 0;
            while n < total {
                let chunk: Vec<f32> = (0..CHUNK).map(|i| (n * CHUNK + i) as f32).collect();
                if producer.push(tag((n * CHUNK) as i64, 0, CHUNK as u32), &chunk) {
                    n += 1;
                } else {
                    std::thread::yield_now();
                }
            }
        });
        let mut c = Consumer::new(ring.clone());
        let mut expected = 0f32;
        let mut out = vec![0f32; 100];
        while (expected as usize) < total * CHUNK {
            let written = ring.head.load(Ordering::Acquire) * CHUNK;
            let n = out.len().min(written - expected as usize);
            if n == 0 {
                std::thread::yield_now();
                continue;
            }
            c.fill(&mut out[..n], 1);
            for &v in &out[..n] {
                assert_eq!(v, expected);
                expected += 1.0;
            }
        }
        h.join().unwrap();
    }
}
