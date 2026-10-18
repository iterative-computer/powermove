//! Engine snapshots on the presenter's render thread (the Phase 3c render
//! transport; plan sections 18 to 20; `docs/engine/rfcs/16-render-transport.md`).
//!
//! Main asks for a snapshot of a renderer session's document
//! (`Request::Snapshot`); the engine routes it here, and the render thread
//! renders it between viewer frames with the same renderer, definitions,
//! media, and fonts as the session's viewer, reads it back, and answers on
//! main's host port: RGBA8 pixels inline or in a shared-memory segment.
//!
//! This module is the part that needs no GPU: the queue and its scheduling
//! classes, the revision wait, the memory caps, and the pixel conversion.
//! [`super::render`] renders the job the queue hands it.
//!
//! **Scheduling.** A `sync` job (a renderer thread is blocked on it) runs at
//! the first free point of the render thread. An `async` job runs once no
//! playing view has a frame due before the job would finish (its cost is
//! estimated from earlier snapshots). An `idle` job (thumbnails, library
//! cards, posters, the project thumbnail) runs only while nothing plays,
//! no gesture or scrub frame was drawn in the last [`GESTURE_IDLE_US`], and
//! no `sync` or `async` job waits; one idle job per turn of the thread, so
//! a gesture that starts is served at once.
//!
//! **No deadlock.** A job never waits on the renderer: one whose document
//! has not reached the asked revision waits (bounded by its own `wait_ms`)
//! for patches the renderer already posted; one that needs pixels only the
//! renderer draws (the shader fallback runner, compatibility pixels) is
//! answered `needsRenderer`, so the renderer draws them and asks again.

use std::collections::VecDeque;

use pm_protocol::common::{RequestId, Revision};
use pm_protocol::host::PortId;
use pm_protocol::snapshot::{SnapshotClass, SnapshotPixels, SnapshotRequest};

use super::render::Key;

/// A gesture or scrub frame this recent keeps idle snapshots waiting.
pub const GESTURE_IDLE_US: i64 = 200_000;
/// Most jobs queued at once (every class), and per session.
pub const MAX_QUEUED: usize = 64;
pub const MAX_QUEUED_PER_SESSION: usize = 16;
/// Most pixels one snapshot renders (an 8K UHD frame is 33.2 million). The
/// render target (binary16) and its readback take 12 bytes a pixel.
pub const MAX_PIXELS: u64 = 40_000_000;
/// Most bytes of results queued or published in shared memory and not yet
/// taken by main.
pub const MAX_PENDING_BYTES: u64 = 1 << 30;
/// The first estimate of a snapshot's cost before any was measured (µs).
pub const FIRST_COST_US: f64 = 30_000.0;

/// One snapshot waiting on the render thread.
#[derive(Debug, Clone)]
pub struct SnapshotJob {
    /// Main's host port and its request id: where the answer goes.
    pub host: PortId,
    pub id: RequestId,
    /// The renderer session whose document it shows.
    pub key: Key,
    pub request: SnapshotRequest,
    /// The envelope's minimum revision (read-your-writes).
    pub min_revision: Option<Revision>,
    /// Host µs it reached the render thread, and its deadline.
    pub received: i64,
    pub deadline: i64,
}

impl SnapshotJob {
    pub fn new(
        host: PortId,
        id: RequestId,
        key: Key,
        request: SnapshotRequest,
        min_revision: Option<Revision>,
        now: i64,
    ) -> SnapshotJob {
        let deadline = now + i64::from(request.wait_ms) * 1_000;
        SnapshotJob {
            host,
            id,
            key,
            request,
            min_revision,
            received: now,
            deadline,
        }
    }

    /// Bytes of its RGBA8 result.
    pub fn bytes(&self) -> u64 {
        self.request.max_bytes()
    }
}

/// What the thread's views are doing, for the scheduling decision.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Pressure {
    /// Some view plays.
    pub playing: bool,
    /// µs until the earliest playing view's next frame is due (`None`: no
    /// frame due, or nothing plays). Zero or negative: due now.
    pub due_in_us: Option<i64>,
    /// Host µs of the newest gesture, scrub, or paused frame drawn.
    pub last_interactive: Option<i64>,
}

/// Why a job cannot run yet, or that it can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ready {
    Run,
    /// The session's document has not reached the asked revision.
    Revision,
    /// Its class waits for the views (playback, a gesture).
    Pressure,
}

/// A job refused when it arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    Full,
    TooLarge(u64),
}

impl Refused {
    pub fn message(&self) -> String {
        match self {
            Refused::Full => "Too many snapshots are waiting; try again when they finish.".into(),
            Refused::TooLarge(pixels) => {
                format!("A {pixels}-pixel snapshot is over the engine's {MAX_PIXELS}-pixel limit.")
            }
        }
    }
}

/// The render thread's snapshot queue.
#[derive(Debug, Default)]
pub struct SnapshotQueue {
    jobs: VecDeque<SnapshotJob>,
    /// Exponential average of a snapshot's render time (µs).
    cost_us: Option<f64>,
    /// Bytes of results published to shared memory and not reaped yet.
    published: VecDeque<(i64, u64)>,
}

impl SnapshotQueue {
    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// The estimated cost of the next snapshot (µs).
    pub fn cost_us(&self) -> f64 {
        self.cost_us.unwrap_or(FIRST_COST_US)
    }

    /// Records a measured render time.
    pub fn observe(&mut self, us: f64) {
        self.cost_us = Some(match self.cost_us {
            Some(avg) => avg * 0.75 + us * 0.25,
            None => us,
        });
    }

    fn pending_bytes(&self) -> u64 {
        self.jobs.iter().map(SnapshotJob::bytes).sum::<u64>()
            + self.published.iter().map(|(_, b)| b).sum::<u64>()
    }

    /// Records a result published to shared memory at `now`, and forgets
    /// those older than `orphan_us` (taken or reaped by then).
    pub fn published(&mut self, now: i64, bytes: u64, orphan_us: i64) {
        self.published.retain(|(at, _)| now - at < orphan_us);
        self.published.push_back((now, bytes));
    }

    /// Adds a job, or refuses it (too large, or the queue or its memory is
    /// full).
    pub fn push(&mut self, job: SnapshotJob) -> Result<(), Box<(SnapshotJob, Refused)>> {
        let size = job.request.output_size;
        let pixels = u64::from(size.width) * u64::from(size.height);
        if pixels > MAX_PIXELS {
            return Err(Box::new((job, Refused::TooLarge(pixels))));
        }
        let per_session = self.jobs.iter().filter(|j| j.key == job.key).count();
        if self.jobs.len() >= MAX_QUEUED
            || per_session >= MAX_QUEUED_PER_SESSION
            || self.pending_bytes() + job.bytes() > MAX_PENDING_BYTES
        {
            return Err(Box::new((job, Refused::Full)));
        }
        self.jobs.push_back(job);
        Ok(())
    }

    /// Removes and returns every job of `key` (its session closed).
    pub fn take_session(&mut self, key: Key) -> Vec<SnapshotJob> {
        let (gone, kept): (Vec<_>, Vec<_>) = self.jobs.drain(..).partition(|j| j.key == key);
        self.jobs = kept.into();
        gone
    }

    /// Removes and returns every job whose deadline passed.
    pub fn take_expired(&mut self, now: i64) -> Vec<SnapshotJob> {
        let (gone, kept): (Vec<_>, Vec<_>) = self.jobs.drain(..).partition(|j| now >= j.deadline);
        self.jobs = kept.into();
        gone
    }

    /// Whether `job` can run now. `revision` is the session's document
    /// revision (`None`: the session has no document yet).
    pub fn ready(
        &self,
        job: &SnapshotJob,
        revision: Option<u64>,
        pressure: Pressure,
        now: i64,
    ) -> Ready {
        let Some(revision) = revision else {
            return Ready::Revision;
        };
        if job
            .min_revision
            .is_some_and(|wanted| revision < wanted.get())
        {
            return Ready::Revision;
        }
        match job.request.class {
            SnapshotClass::Sync => Ready::Run,
            SnapshotClass::Async => {
                let fits = !pressure.playing
                    || pressure
                        .due_in_us
                        .is_none_or(|due| due as f64 >= self.cost_us() * 1.25 + 2_000.0);
                if fits { Ready::Run } else { Ready::Pressure }
            }
            SnapshotClass::Idle => {
                let urgent = self
                    .jobs
                    .iter()
                    .any(|j| j.request.class != SnapshotClass::Idle);
                let gesture = pressure
                    .last_interactive
                    .is_some_and(|at| now - at < GESTURE_IDLE_US);
                if pressure.playing || gesture || urgent {
                    Ready::Pressure
                } else {
                    Ready::Run
                }
            }
        }
    }

    /// The next job to run: `sync` first, then `async`, then one `idle`, in
    /// arrival order within a class. `revision` gives each session's
    /// document revision.
    pub fn next(
        &mut self,
        revision: impl Fn(Key) -> Option<u64>,
        pressure: Pressure,
        now: i64,
    ) -> Option<SnapshotJob> {
        for class in [
            SnapshotClass::Sync,
            SnapshotClass::Async,
            SnapshotClass::Idle,
        ] {
            let found = self.jobs.iter().position(|j| {
                j.request.class == class
                    && self.ready(j, revision(j.key), pressure, now) == Ready::Run
            });
            if let Some(index) = found {
                return self.jobs.remove(index);
            }
        }
        None
    }

    /// Puts a job back at the front of the queue (it needs another turn).
    pub fn requeue(&mut self, job: SnapshotJob) {
        self.jobs.push_front(job);
    }

    /// Whether any job waits for a revision or for the views, so the thread
    /// polls (patches land through commands that wake it; the views'
    /// pressure ends without one).
    pub fn waiting(&self) -> bool {
        !self.jobs.is_empty()
    }

    /// The earliest deadline (host µs).
    pub fn earliest_deadline(&self) -> Option<i64> {
        self.jobs.iter().map(|j| j.deadline).min()
    }
}

/// Encodes a premultiplied binary16 RGBA readback (top-down, tightly
/// packed, 8 bytes a pixel, little-endian) as RGBA8 in `pixels`'s encoding.
/// Values are clamped to 0..1 and rounded to nearest, ties to even (a GPU's
/// float to unorm conversion, and today's capture through an RGBA8
/// renderbuffer).
pub fn encode_rgba8(half: &[u8], pixels: SnapshotPixels) -> Vec<u8> {
    let mut out = Vec::with_capacity(half.len() / 2);
    let unorm = |v: f32| -> u8 {
        let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
        (v * 255.0).round_ties_even() as u8
    };
    for px in half.chunks_exact(8) {
        let c = |i: usize| pm_gpu::readback::half_to_f32(u16::from_le_bytes([px[i], px[i + 1]]));
        let (r, g, b, a) = (c(0), c(2), c(4), c(6));
        match pixels {
            SnapshotPixels::Opaque => out.extend_from_slice(&[unorm(r), unorm(g), unorm(b), 255]),
            SnapshotPixels::Premultiplied => {
                out.extend_from_slice(&[unorm(r), unorm(g), unorm(b), unorm(a)])
            }
            SnapshotPixels::Straight => {
                // As a canvas's `getImageData` does: quantize the
                // premultiplied value first, then divide by alpha.
                let a8 = unorm(a);
                if a8 == 0 {
                    out.extend_from_slice(&[0, 0, 0, 0]);
                } else {
                    let un = |v: f32| -> u8 {
                        let p = u32::from(unorm(v));
                        ((p * 255 + u32::from(a8) / 2) / u32::from(a8)).min(255) as u8
                    };
                    out.extend_from_slice(&[un(r), un(g), un(b), a8]);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::common::{SessionId, Size, Ticks};
    use pm_protocol::snapshot::SnapshotTarget;

    fn key(port: u32, session: u64) -> Key {
        (PortId(port), SessionId::new(session).unwrap())
    }

    fn job(class: SnapshotClass, k: Key, revision: Option<u64>, now: i64) -> SnapshotJob {
        SnapshotJob::new(
            PortId(1),
            RequestId::new(7).unwrap(),
            k,
            SnapshotRequest {
                composition: "c".into(),
                time: Ticks::ZERO,
                output_size: Size::new(64, 36),
                target: SnapshotTarget::Frame {
                    motion_blur_samples: 0,
                    background: true,
                    draft_3d: false,
                },
                pixels: SnapshotPixels::Opaque,
                class,
                view_seq: None,
                wait_ms: 1_000,
            },
            revision.map(|r| Revision::new(r).unwrap()),
            now,
        )
    }

    fn idle() -> Pressure {
        Pressure::default()
    }

    #[test]
    fn a_job_waits_for_its_revision_and_then_runs() {
        let mut q = SnapshotQueue::default();
        let k = key(3, 1);
        q.push(job(SnapshotClass::Sync, k, Some(5), 0)).unwrap();
        assert!(q.next(|_| Some(4), idle(), 0).is_none(), "revision 4 < 5");
        assert!(q.next(|_| None, idle(), 0).is_none(), "no document yet");
        assert!(q.next(|_| Some(5), idle(), 0).is_some());
    }

    #[test]
    fn sync_runs_during_playback_async_waits_for_room_idle_waits_for_rest() {
        let mut q = SnapshotQueue::default();
        let k = key(3, 1);
        let playing = Pressure {
            playing: true,
            due_in_us: Some(4_000),
            last_interactive: None,
        };
        q.push(job(SnapshotClass::Idle, k, None, 0)).unwrap();
        q.push(job(SnapshotClass::Async, k, None, 0)).unwrap();
        q.push(job(SnapshotClass::Sync, k, None, 0)).unwrap();
        let first = q.next(|_| Some(0), playing, 0).unwrap();
        assert_eq!(first.request.class, SnapshotClass::Sync);
        // A frame due in 4 ms: no room for a 30 ms async snapshot.
        assert!(q.next(|_| Some(0), playing, 0).is_none());
        let room = Pressure {
            due_in_us: Some(100_000),
            ..playing
        };
        assert_eq!(
            q.next(|_| Some(0), room, 0).unwrap().request.class,
            SnapshotClass::Async
        );
        // Idle never runs during playback, however much room.
        assert!(q.next(|_| Some(0), room, 0).is_none());
        // Nor right after a gesture frame.
        let gesture = Pressure {
            last_interactive: Some(1_000_000 - 50_000),
            ..idle()
        };
        assert!(q.next(|_| Some(0), gesture, 1_000_000).is_none());
        assert_eq!(
            q.next(|_| Some(0), idle(), 1_000_000)
                .unwrap()
                .request
                .class,
            SnapshotClass::Idle
        );
    }

    #[test]
    fn idle_waits_while_urgent_jobs_wait_for_their_revision() {
        let mut q = SnapshotQueue::default();
        q.push(job(SnapshotClass::Sync, key(3, 1), Some(9), 0))
            .unwrap();
        q.push(job(SnapshotClass::Idle, key(3, 2), None, 0))
            .unwrap();
        assert!(q.next(|_| Some(0), idle(), 0).is_none());
    }

    #[test]
    fn the_cost_estimate_learns_from_measurements() {
        let mut q = SnapshotQueue::default();
        assert_eq!(q.cost_us(), FIRST_COST_US);
        q.observe(4_000.0);
        assert_eq!(q.cost_us(), 4_000.0);
        let playing = Pressure {
            playing: true,
            due_in_us: Some(9_000),
            last_interactive: None,
        };
        q.push(job(SnapshotClass::Async, key(3, 1), None, 0))
            .unwrap();
        assert!(
            q.next(|_| Some(0), playing, 0).is_some(),
            "4 ms fits before a frame due in 9 ms"
        );
    }

    #[test]
    fn caps_refuse_too_many_and_too_large() {
        let mut q = SnapshotQueue::default();
        for i in 0..MAX_QUEUED_PER_SESSION {
            q.push(job(SnapshotClass::Idle, key(3, 1), None, i as i64))
                .unwrap();
        }
        assert_eq!(
            q.push(job(SnapshotClass::Idle, key(3, 1), None, 0))
                .unwrap_err()
                .1,
            Refused::Full
        );
        q.push(job(SnapshotClass::Idle, key(3, 2), None, 0))
            .unwrap();
        let mut big = job(SnapshotClass::Sync, key(4, 1), None, 0);
        big.request.output_size = Size::new(16_000, 16_000);
        assert!(matches!(q.push(big).unwrap_err().1, Refused::TooLarge(_)));
        // Memory: published results count until they are reaped.
        let mut m = SnapshotQueue::default();
        m.published(0, MAX_PENDING_BYTES, 60_000_000);
        assert_eq!(
            m.push(job(SnapshotClass::Sync, key(3, 1), None, 0))
                .unwrap_err()
                .1,
            Refused::Full
        );
        m.published(60_000_001, 16, 60_000_000);
        assert!(m.push(job(SnapshotClass::Sync, key(3, 1), None, 0)).is_ok());
    }

    #[test]
    fn closed_sessions_and_deadlines_take_their_jobs() {
        let mut q = SnapshotQueue::default();
        q.push(job(SnapshotClass::Sync, key(3, 1), None, 0))
            .unwrap();
        q.push(job(SnapshotClass::Sync, key(3, 2), None, 500_000))
            .unwrap();
        assert_eq!(q.take_session(key(3, 1)).len(), 1);
        assert_eq!(q.take_expired(1_000_000).len(), 0);
        assert_eq!(q.take_expired(1_500_000).len(), 1);
        assert!(q.is_empty());
    }

    fn half(v: [f32; 4]) -> Vec<u8> {
        v.iter()
            .flat_map(|c| pm_gpu::readback::f32_to_half(*c).to_le_bytes())
            .collect()
    }

    #[test]
    fn pixels_encode_like_todays_capture() {
        // Half-covered red over black, premultiplied.
        let px = half([0.5, 0.0, 0.0, 0.5]);
        assert_eq!(
            encode_rgba8(&px, SnapshotPixels::Opaque),
            vec![128, 0, 0, 255]
        );
        assert_eq!(
            encode_rgba8(&px, SnapshotPixels::Premultiplied),
            vec![128, 0, 0, 128]
        );
        assert_eq!(
            encode_rgba8(&px, SnapshotPixels::Straight),
            vec![255, 0, 0, 128]
        );
        // Out of range and transparent.
        let px = half([1.5, -0.25, 0.0, 0.0]);
        assert_eq!(
            encode_rgba8(&px, SnapshotPixels::Opaque),
            vec![255, 0, 0, 255]
        );
        assert_eq!(
            encode_rgba8(&px, SnapshotPixels::Straight),
            vec![0, 0, 0, 0]
        );
        // Ties round to even: 0.5 × 255 = 127.5 → 128 (even), and an exact
        // binary16 value 1/510 × 255 = 0.5 → 0.
        let px = half([1.0 / 510.0, 0.0, 0.0, 1.0]);
        assert_eq!(encode_rgba8(&px, SnapshotPixels::Opaque)[0], 0);
    }
}
