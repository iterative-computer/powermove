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
use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use pm_protocol::clock::TransportState;
use pm_protocol::common::{PlaybackRate, TickRange};

use crate::clock::host_now_us;

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
    /// Media advanced per frame of the chunk, in project frames: 1, 2, −1,
    /// or −2 (J/K/L).
    pub rate: PlaybackRate,
}

impl Default for Tag {
    fn default() -> Self {
        Tag {
            media_ticks: 0,
            epoch: 0,
            state: TransportState::Stopped,
            frames: 0,
            loop_range: None,
            rate: PlaybackRate::NORMAL,
        }
    }
}

struct Slot {
    next: AtomicPtr<Slot>,
    tag: UnsafeCell<Tag>,
    data: UnsafeCell<[f32; CHUNK * MAX_CHANNELS]>,
    /// An upper bound on publication, stored after `head` publishes it.
    published_after_us: AtomicU64,
}

/// Shared state. `head` counts chunks written, `tail` chunks consumed.
pub struct Ring {
    // Stable addresses: the producer splices new empty nodes at its sentinel.
    // Only grow() locks/allocates; the callback follows published nodes only.
    #[allow(clippy::vec_box)]
    slots: Mutex<Vec<Box<Slot>>>,
    capacity: AtomicUsize,
    producer: AtomicPtr<Slot>,
    consumer: AtomicPtr<Slot>,
    channels: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
    /// Chunks of older epochs are skipped by the consumer.
    epoch: AtomicU32,
    underruns: AtomicU64,
    /// Frames the consumer has played (all epochs).
    played: AtomicU64,
    written: AtomicU64,
    retired: AtomicU64,
    epoch_base: AtomicU64,
    callback_frames: AtomicUsize,
}

// The single producer and single consumer touch disjoint nodes, ordered by
// acquire/release on head/tail. Growth splices at the unfilled producer node
// (the extra sentinel ensures it is empty even when the ring is full).
// Node addresses remain stable until Ring drops; only the producer accesses
// the allocations vector. The callback never locks or allocates.
unsafe impl Sync for Ring {}
unsafe impl Send for Ring {}

impl Ring {
    /// A ring of `chunks` chunks of `channels` interleaved channels.
    pub fn new(chunks: usize, channels: usize) -> Arc<Ring> {
        assert!(
            (1..=MAX_CHANNELS).contains(&channels),
            "1 to {MAX_CHANNELS} channels"
        );
        let count = chunks.max(2);
        // An extra, always-empty sentinel lets the producer grow even at full.
        let mut slots: Vec<_> = (0..=count)
            .map(|_| {
                Box::new(Slot {
                    next: AtomicPtr::new(std::ptr::null_mut()),
                    tag: UnsafeCell::new(Tag::default()),
                    data: UnsafeCell::new([0.0; CHUNK * MAX_CHANNELS]),
                    published_after_us: AtomicU64::new(0),
                })
            })
            .collect();
        for i in 0..slots.len() {
            let next = (i + 1) % slots.len();
            let ptr = &mut *slots[next] as *mut Slot;
            slots[i].next.store(ptr, Ordering::Relaxed);
        }
        let first = &mut *slots[0] as *mut Slot;
        Arc::new(Ring {
            slots: Mutex::new(slots),
            capacity: AtomicUsize::new(count),
            producer: AtomicPtr::new(first),
            consumer: AtomicPtr::new(first),
            channels,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            epoch: AtomicU32::new(0),
            underruns: AtomicU64::new(0),
            played: AtomicU64::new(0),
            written: AtomicU64::new(0),
            retired: AtomicU64::new(0),
            epoch_base: AtomicU64::new(0),
            callback_frames: AtomicUsize::new(CHUNK * 2),
        })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn capacity(&self) -> usize {
        self.capacity.load(Ordering::Acquire)
    }

    /// Producer only: grow to fit an observed callback plus its wait budget.
    /// Queued nodes never move, and no memory is freed until both sides drop.
    pub fn grow(&self, chunks: usize) {
        let old = self.capacity();
        if chunks <= old {
            return;
        }
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        let head = self.producer.load(Ordering::Relaxed);
        // SAFETY: head is the producer's empty sentinel. The consumer cannot
        // visit it until a push publishes it, after this entire splice.
        let after = unsafe { (*head).next.load(Ordering::Relaxed) };
        let base = slots.len();
        for _ in old..chunks {
            slots.push(Box::new(Slot {
                next: AtomicPtr::new(after),
                tag: UnsafeCell::new(Tag::default()),
                data: UnsafeCell::new([0.0; CHUNK * MAX_CHANNELS]),
                published_after_us: AtomicU64::new(0),
            }));
        }
        for i in base..slots.len() - 1 {
            let next = &mut *slots[i + 1] as *mut Slot;
            slots[i].next.store(next, Ordering::Relaxed);
        }
        let first = &mut *slots[base] as *mut Slot;
        unsafe {
            (*head).next.store(first, Ordering::Release);
        }
        self.capacity.store(chunks, Ordering::Release);
    }

    /// Chunks written and not yet consumed.
    pub fn queued(&self) -> usize {
        let tail = self.tail.load(Ordering::Acquire);
        self.head.load(Ordering::Acquire) - tail
    }

    pub fn epoch(&self) -> u32 {
        self.epoch.load(Ordering::Acquire)
    }

    /// Moves to a new epoch: queued chunks of older epochs will not play.
    pub fn set_epoch(&self, epoch: u32) {
        self.epoch_base
            .store(self.written.load(Ordering::Relaxed), Ordering::Relaxed);
        self.epoch.store(epoch, Ordering::Release);
    }

    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }

    pub fn played_frames(&self) -> u64 {
        self.played.load(Ordering::Relaxed)
    }

    /// Producer-side current-epoch reserve, without reading consumer-owned
    /// slots or its partial-chunk offset. Retired includes skipped old epochs.
    pub fn buffered_frames(&self) -> u64 {
        let retired = self
            .retired
            .load(Ordering::Acquire)
            .max(self.epoch_base.load(Ordering::Relaxed));
        self.written.load(Ordering::Relaxed).saturating_sub(retired)
    }

    /// Two callback buffers leave time to wake, mix, and prime downstream DSP.
    pub fn refill_reserve(&self) -> u64 {
        (self.callback_frames.load(Ordering::Relaxed).max(CHUNK) * 2) as u64
    }

    /// Producer: writes a chunk (`interleaved` holds `tag.frames` frames).
    /// Returns false when the ring is full.
    pub fn push(&self, tag: Tag, interleaved: &[f32]) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head - tail >= self.capacity() {
            return false;
        }
        let frames = (tag.frames as usize).min(CHUNK);
        // SAFETY: producer points at its exclusively owned empty node.
        let slot = unsafe { &*self.producer.load(Ordering::Relaxed) };
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
        slot.published_after_us.store(0, Ordering::Relaxed);
        self.written.fetch_add(frames as u64, Ordering::Relaxed);
        self.producer
            .store(slot.next.load(Ordering::Acquire), Ordering::Relaxed);
        // The zeroed timestamp and PCM become visible together here. If the
        // producer is descheduled before the following stamp, an observer can
        // still use its own clock as an upper bound on publication.
        self.head.store(head + 1, Ordering::Release);
        slot.published_after_us
            .store(host_now_us(), Ordering::Release);
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

/// Two upper bounds on a chunk's publication on the monotonic host clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Publication {
    /// Producer clock sampled after the release, if it stamped the slot yet.
    pub published_us: Option<u64>,
    /// Observer clock sampled after acquiring the published slot.
    pub observed_us: u64,
    /// Earlier of those upper bounds: the deadline must hold for this value.
    pub visible_by_us: u64,
}

impl Consumer {
    pub fn new(ring: Arc<Ring>) -> Consumer {
        Consumer { ring, offset: 0 }
    }

    pub fn ring(&self) -> &Arc<Ring> {
        &self.ring
    }

    /// Frames of the current epoch the next [`Self::fill`] can play (the
    /// rest of the chunk being played counts its unplayed frames only).
    /// Called from the callback's side, like `fill`.
    pub fn queued_frames(&self) -> usize {
        self.queued_snapshot(usize::MAX).0
    }

    /// Upper bounds on when at least `frames` of the current epoch were
    /// queued. The crossing chunk is read under the same acquired `head`
    /// snapshot as its tag and PCM. Either side may be descheduled after the
    /// release; the earlier clock still bounds publication from above.
    pub fn publication_for_queued_frames(&self, frames: usize) -> Option<Publication> {
        if frames == 0 {
            return None;
        }
        self.queued_snapshot(frames).1
    }

    fn queued_snapshot(&self, threshold: usize) -> (usize, Option<Publication>) {
        let ring = &*self.ring;
        let current = ring.epoch.load(Ordering::Acquire);
        let tail = ring.tail.load(Ordering::Relaxed);
        let head = ring.head.load(Ordering::Acquire);
        let mut frames = 0;
        let mut publication = None;
        let mut ptr = ring.consumer.load(Ordering::Relaxed);
        for i in 0..head - tail {
            // SAFETY: all nodes before the acquired head are published.
            let slot = unsafe { &*ptr };
            ptr = slot.next.load(Ordering::Acquire);
            // SAFETY: as in `fill`: `head` (acquired) is past this slot, and
            // only this side moves `tail`.
            let tag = unsafe { *slot.tag.get() };
            if tag.epoch < current {
                continue;
            }
            let played = if i == 0 { self.offset } else { 0 };
            frames += (tag.frames as usize).saturating_sub(played);
            if publication.is_none() && frames >= threshold {
                let stamped = slot.published_after_us.load(Ordering::Acquire);
                let observed_us = host_now_us();
                let published_us = (stamped != 0).then_some(stamped);
                publication = Some(Publication {
                    published_us,
                    observed_us,
                    visible_by_us: published_us.map_or(observed_us, |at| at.min(observed_us)),
                });
            }
        }
        (frames, publication)
    }

    /// Fills `out` (interleaved, `device_channels` wide) from the ring.
    /// Device channels beyond the ring's are silent. Returns what the first
    /// frame plays, when it plays media.
    pub fn fill(&mut self, out: &mut [f32], device_channels: usize) -> Option<Heard> {
        let ring = &*self.ring;
        let rc = ring.channels;
        let frames = out.len() / device_channels.max(1);
        ring.callback_frames.fetch_max(frames, Ordering::Relaxed);
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
            // SAFETY: tail is published and owned by this consumer.
            let slot = unsafe { &*ring.consumer.load(Ordering::Relaxed) };
            // SAFETY: `head` (acquired) is past this slot, so the producer
            // finished writing it and will not touch it until `tail` passes.
            let (tag, data) = unsafe { (*slot.tag.get(), &*slot.data.get()) };
            if tag.epoch < current || self.offset >= tag.frames as usize {
                ring.retired.fetch_add(
                    (tag.frames as usize).saturating_sub(self.offset) as u64,
                    Ordering::Release,
                );
                self.offset = 0;
                ring.consumer
                    .store(slot.next.load(Ordering::Acquire), Ordering::Relaxed);
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
            ring.retired.fetch_add(take as u64, Ordering::Release);
            done += take;
            self.offset += take;
            if self.offset >= tag.frames as usize {
                self.offset = 0;
                ring.consumer
                    .store(slot.next.load(Ordering::Acquire), Ordering::Relaxed);
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
            rate: PlaybackRate::NORMAL,
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
    fn queued_frames_count_what_the_next_fill_plays() {
        let ring = Ring::new(4, 1);
        let mut c = Consumer::new(ring.clone());
        let a = vec![0.5f32; CHUNK];
        assert_eq!(c.queued_frames(), 0);
        assert!(ring.push(tag(0, 0, CHUNK as u32), &a));
        assert!(ring.push(tag(0, 0, 10), &a));
        assert_eq!(c.queued_frames(), CHUNK + 10);
        assert_eq!(ring.buffered_frames(), (CHUNK + 10) as u64);
        let mut out = vec![0f32; 100];
        c.fill(&mut out, 1);
        assert_eq!(c.queued_frames(), CHUNK - 100 + 10);
        assert_eq!(ring.buffered_frames(), (CHUNK - 100 + 10) as u64);
        // A new epoch's chunks only: the old ones will be skipped.
        ring.set_epoch(1);
        assert_eq!(c.queued_frames(), 0);
        assert_eq!(ring.buffered_frames(), 0);
        assert!(ring.push(tag(0, 1, CHUNK as u32), &a));
        assert_eq!(c.queued_frames(), CHUNK);
        assert_eq!(ring.queued(), 3);
        assert_eq!(ring.buffered_frames(), CHUNK as u64);
        c.fill(&mut out, 1);
        assert_eq!(ring.buffered_frames(), (CHUNK - 100) as u64);
        assert_eq!(c.queued_frames(), CHUNK - 100);
    }

    #[test]
    fn publication_clock_follows_the_threshold_chunk_and_epoch() {
        let ring = Ring::new(4, 1);
        let mut consumer = Consumer::new(ring.clone());
        let first = vec![1.0; CHUNK];
        let second = vec![2.0; CHUNK];
        let third = vec![3.0; CHUNK];
        assert!(ring.push(tag(0, 0, CHUNK as u32), &first));
        assert!(ring.push(tag(CHUNK as i64, 0, CHUNK as u32), &second));
        let first_slot = ring.consumer.load(Ordering::Relaxed);
        let second_slot = unsafe { (*first_slot).next.load(Ordering::Relaxed) };
        let second_stamp = unsafe { (*second_slot).published_after_us.load(Ordering::Acquire) };
        std::thread::sleep(std::time::Duration::from_millis(1));
        let publication = consumer.publication_for_queued_frames(2 * CHUNK).unwrap();
        assert_eq!(publication.published_us, Some(second_stamp));
        assert_eq!(publication.visible_by_us, second_stamp);
        assert!(publication.observed_us >= second_stamp);
        assert!(
            consumer
                .publication_for_queued_frames(2 * CHUNK + 1)
                .is_none()
        );

        ring.set_epoch(1);
        assert!(ring.push(tag(10_000, 1, CHUNK as u32), &third));
        let third_slot = unsafe { (*second_slot).next.load(Ordering::Relaxed) };
        assert_eq!(consumer.queued_frames(), CHUNK);
        let third_stamp = unsafe { (*third_slot).published_after_us.load(Ordering::Acquire) };
        let publication = consumer.publication_for_queued_frames(CHUNK).unwrap();
        assert_eq!(publication.published_us, Some(third_stamp));
        assert_eq!(
            publication.visible_by_us,
            third_stamp.min(publication.observed_us)
        );

        // Model an observer arriving after the release but before the
        // producer's post-release stamp. Its own clock still bounds the PCM.
        unsafe { (*third_slot).published_after_us.store(0, Ordering::Release) };
        let publication = consumer.publication_for_queued_frames(CHUNK).unwrap();
        assert_eq!(publication.published_us, None);
        assert_eq!(publication.visible_by_us, publication.observed_us);
        let mut out = vec![0.0; CHUNK];
        assert_eq!(consumer.fill(&mut out, 1).unwrap().tag.epoch, 1);
        assert_eq!(out, third);
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
    fn growing_a_full_partially_played_ring_preserves_epochs_and_samples() {
        let ring = Ring::new(2, 1);
        let mut consumer = Consumer::new(ring.clone());
        let a: Vec<f32> = (0..CHUNK).map(|i| i as f32).collect();
        assert!(ring.push(tag(0, 0, CHUNK as u32), &a));
        assert!(ring.push(tag(256, 0, CHUNK as u32), &a));
        let mut out = vec![0.0; 17];
        consumer.fill(&mut out, 1);
        ring.grow(80);
        assert_eq!(ring.capacity(), 80);
        assert_eq!(consumer.queued_frames(), 2 * CHUNK - 17);
        assert_eq!(ring.buffered_frames(), (2 * CHUNK - 17) as u64);
        for i in 2..80 {
            assert!(ring.push(tag(i * 256, 0, CHUNK as u32), &a));
        }
        let mut out = vec![0.0; 80 * CHUNK - 17];
        consumer.fill(&mut out, 1);
        for (i, sample) in out.iter().enumerate() {
            assert_eq!(*sample, a[(i + 17) % CHUNK]);
        }
        assert_eq!(ring.underruns(), 0);
        assert!(ring.push(tag(0, 0, CHUNK as u32), &a));
        ring.set_epoch(1);
        ring.grow(160);
        assert!(ring.push(tag(1234, 1, CHUNK as u32), &a));
        let mut out = vec![0.0; CHUNK];
        assert_eq!(consumer.fill(&mut out, 1).unwrap().tag.media_ticks, 1234);
        assert_eq!(out, a);
        assert_eq!(ring.underruns(), 0);
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
                if n == 10 {
                    producer.grow(32);
                }
                if n == 150 {
                    producer.grow(128);
                }
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
