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
/// Today's layer bitmap cap, independent of a full-frame capture's 40 MP.
pub const MAX_LAYER_PIXELS: u64 = 8192 * 8192;
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
    /// Dependencies the caller explicitly posted before asking
    /// (`SnapshotRequest::uploaded` and `compat_uploaded`) and that had not
    /// landed when the job last rendered: wait until none is outstanding.
    pub waiting_for: Vec<String>,
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
            waiting_for: Vec::new(),
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
    TooLarge { pixels: u64, limit: u64 },
}

impl Refused {
    pub fn message(&self) -> String {
        match self {
            Refused::Full => "Too many snapshots are waiting; try again when they finish.".into(),
            Refused::TooLarge { pixels, limit } => {
                format!("A {pixels}-pixel snapshot is over the engine's {limit}-pixel limit.")
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
    /// Names and bytes of results main has not taken (or the reaper unlinked).
    published: VecDeque<(String, u64)>,
    /// Result bytes of the layer rasters handed to the raster worker and not
    /// finished (`super::rasters`): they left the queue but not memory.
    in_worker: u64,
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
            + self.in_worker
    }

    /// The raster worker's jobs' result bytes, counted against
    /// [`MAX_PENDING_BYTES`] while they are drawn.
    pub fn set_in_worker(&mut self, bytes: u64) {
        self.in_worker = bytes;
    }

    /// Records a result until main takes it or orphan cleanup unlinks it.
    pub fn published(&mut self, name: String, bytes: u64) {
        self.published.push_back((name, bytes));
    }

    /// Main unlinks a result before reading it. Count only actual outstanding
    /// results, rather than keeping already-consumed pixels charged for 60 s.
    /// An OS error retains the charge conservatively.
    fn reclaim_published(&mut self) {
        self.published
            .retain(|(name, _)| pm_bridge::shm::exists(name).unwrap_or(true));
    }

    /// Adds a job, or refuses it (too large, or the queue or its memory is
    /// full).
    pub fn push(&mut self, job: SnapshotJob) -> Result<(), Box<(SnapshotJob, Refused)>> {
        let size = job.request.output_size;
        let pixels = u64::from(size.width) * u64::from(size.height);
        let limit = if matches!(
            job.request.target,
            pm_protocol::snapshot::SnapshotTarget::Layer { .. }
        ) {
            MAX_LAYER_PIXELS
        } else {
            MAX_PIXELS
        };
        if pixels > limit {
            return Err(Box::new((job, Refused::TooLarge { pixels, limit })));
        }
        self.reclaim_published();
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

    pub fn take_request(&mut self, host: PortId, id: RequestId) -> Option<SnapshotJob> {
        let index = self
            .jobs
            .iter()
            .position(|job| job.host == host && job.id == id)?;
        self.jobs.remove(index)
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
        revision: impl Fn(&SnapshotJob) -> Option<u64>,
        outstanding: impl Fn(&str) -> bool,
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
                    && !j.waiting_for.iter().any(|k| outstanding(k))
                    && self.ready(j, revision(j), pressure, now) == Ready::Run
            });
            if let Some(index) = found {
                return self.jobs.remove(index);
            }
        }
        None
    }

    /// The next layer raster for the worker ([`super::rasters`]): `sync`
    /// first, then `async`, in arrival order. They run off the render
    /// thread, so playback does not hold them back; `idle` ones still wait
    /// for the views like every idle job (thumbnails yield to playback and
    /// gestures on the GPU too).
    pub fn next_layer(
        &mut self,
        revision: impl Fn(Key) -> Option<u64>,
        outstanding: impl Fn(&str) -> bool,
        pressure: Pressure,
        now: i64,
    ) -> Option<SnapshotJob> {
        let unpressed = Pressure {
            playing: false,
            due_in_us: None,
            last_interactive: None,
        };
        for class in [
            SnapshotClass::Sync,
            SnapshotClass::Async,
            SnapshotClass::Idle,
        ] {
            let found = self.jobs.iter().position(|j| {
                let p = if class == SnapshotClass::Idle {
                    pressure
                } else {
                    unpressed
                };
                j.request.class == class
                    && matches!(
                        j.request.target,
                        pm_protocol::snapshot::SnapshotTarget::Layer { .. }
                    )
                    && !j.waiting_for.iter().any(|k| outstanding(k))
                    && self.ready(j, revision(j.key), p, now) == Ready::Run
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
    append_rgba8(half, pixels, &mut out);
    out
}

/// The mapped RGBA8/half/float32 rows of one completed snapshot ticket. This is
/// the single final allocation, preserving alpha, row-order, and rounding.
pub fn encode_mapped_rgba8(
    format: wgpu::TextureFormat,
    data: &[u8],
    row: usize,
    stride: usize,
    height: usize,
    pixels: SnapshotPixels,
    rows: pm_render::SnapshotRows,
) -> Result<Vec<u8>, pm_gpu::readback::ReadbackError> {
    match format {
        wgpu::TextureFormat::Rgba8Unorm => Ok(encode_rgba8_unorm_rows(
            data, row, stride, height, pixels, rows,
        )),
        wgpu::TextureFormat::Rgba16Float => {
            Ok(encode_rgba8_rows(data, row, stride, height, pixels, rows))
        }
        wgpu::TextureFormat::Rgba32Float => Ok(encode_rgba8_f32_rows(
            data, row, stride, height, pixels, rows,
        )),
        other => Err(pm_gpu::readback::ReadbackError::Format(other)),
    }
}

fn encode_rgba8_unorm_rows(
    raw: &[u8],
    row: usize,
    stride: usize,
    height: usize,
    pixels: SnapshotPixels,
    rows: pm_render::SnapshotRows,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(row * height);
    for y in 0..height {
        let source_y = match rows {
            pm_render::SnapshotRows::TopDown => y,
            pm_render::SnapshotRows::Gl => height - 1 - y,
        };
        let source = &raw[source_y * stride..source_y * stride + row];
        for px in source.chunks_exact(4) {
            push_rgba8_unorm(&mut out, [px[0], px[1], px[2], px[3]], pixels);
        }
    }
    out
}

fn encode_rgba8_f32_rows(
    raw: &[u8],
    row: usize,
    stride: usize,
    height: usize,
    pixels: SnapshotPixels,
    rows: pm_render::SnapshotRows,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(row / 4 * height);
    for y in 0..height {
        let source_y = match rows {
            pm_render::SnapshotRows::TopDown => y,
            pm_render::SnapshotRows::Gl => height - 1 - y,
        };
        for px in raw[source_y * stride..source_y * stride + row].chunks_exact(16) {
            let c = |i: usize| f32::from_le_bytes(px[i..i + 4].try_into().expect("four bytes"));
            push_rgba8(&mut out, [c(0), c(4), c(8), c(12)], pixels);
        }
    }
    out
}

fn encode_rgba8_rows(
    half: &[u8],
    row: usize,
    stride: usize,
    height: usize,
    pixels: SnapshotPixels,
    rows: pm_render::SnapshotRows,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(row / 2 * height);
    for y in 0..height {
        let source_y = match rows {
            pm_render::SnapshotRows::TopDown => y,
            pm_render::SnapshotRows::Gl => height - 1 - y,
        };
        append_rgba8(
            &half[source_y * stride..source_y * stride + row],
            pixels,
            &mut out,
        );
    }
    out
}

fn append_rgba8(half: &[u8], pixels: SnapshotPixels, out: &mut Vec<u8>) {
    for px in half.chunks_exact(8) {
        let c = |i: usize| pm_gpu::readback::half_to_f32(u16::from_le_bytes([px[i], px[i + 1]]));
        push_rgba8(out, [c(0), c(2), c(4), c(6)], pixels);
    }
}

/// [`encode_rgba8`] from premultiplied f32 RGBA (a text raster's readback).
pub fn encode_rgba8_f32(rgba: &[f32], pixels: SnapshotPixels) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        push_rgba8(&mut out, [px[0], px[1], px[2], px[3]], pixels);
    }
    out
}

/// [`encode_rgba8`] from premultiplied RGBA8 (a Canvas2D raster's pixels).
pub fn encode_rgba8_unorm(rgba: &[u8], pixels: SnapshotPixels) -> Vec<u8> {
    if pixels == SnapshotPixels::Premultiplied {
        return rgba.to_vec();
    }
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        push_rgba8_unorm(&mut out, [px[0], px[1], px[2], px[3]], pixels);
    }
    out
}

fn push_rgba8_unorm(out: &mut Vec<u8>, [r, g, b, a]: [u8; 4], pixels: SnapshotPixels) {
    match pixels {
        SnapshotPixels::Opaque => out.extend_from_slice(&[r, g, b, 255]),
        SnapshotPixels::Premultiplied => out.extend_from_slice(&[r, g, b, a]),
        SnapshotPixels::Straight if a == 0 => out.extend_from_slice(&[0, 0, 0, 0]),
        SnapshotPixels::Straight => {
            let un =
                |v: u8| ((u32::from(v) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8;
            out.extend_from_slice(&[un(r), un(g), un(b), a]);
        }
    }
}

/// A final Canvas2D `getImageData` raster is straight already. Keep those
/// exact bytes for source snapshots; only premultiply when that encoding was
/// requested, without a lossy premultiply/unpremultiply round trip.
pub fn encode_straight_rgba8_unorm(rgba: &[u8], pixels: SnapshotPixels) -> Vec<u8> {
    if pixels == SnapshotPixels::Straight {
        return rgba.to_vec();
    }
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        let a = u32::from(px[3]);
        for &channel in &px[..3] {
            out.push(((u32::from(channel) * a + 127) / 255) as u8);
        }
        out.push(if pixels == SnapshotPixels::Opaque {
            255
        } else {
            px[3]
        });
    }
    out
}

fn push_rgba8(out: &mut Vec<u8>, [r, g, b, a]: [f32; 4], pixels: SnapshotPixels) {
    // Final renderer frames are premultiplied E, including LinearLight
    // projects (pm-render's final color pass encodes those before readback).
    let alpha = match pixels {
        SnapshotPixels::Opaque => pm_export::AlphaOutput::None,
        SnapshotPixels::Premultiplied => pm_export::AlphaOutput::Premultiplied,
        SnapshotPixels::Straight => pm_export::AlphaOutput::Straight,
    };
    out.extend_from_slice(&pm_export::convert::rgba8_pixel(
        [r, g, b, a],
        pm_export::WorkingEncoding::Standard,
        alpha,
        pm_export::ColorTags::SrgbImage,
        [0.0; 3],
    ));
}

/// A layer raster's pixels placed in its whole canvas (`crop` = the painted
/// part, `[x, y, width, height]` in canvas pixels), then cut to `window`
/// (today's `RasterWindow`, clamped to the canvas). RGBA8 rows top-down.
/// Returns the pixels and their size, or `None` when the window is empty.
pub fn place_raster(
    painted: &[u8],
    crop: [u32; 4],
    canvas: (u32, u32),
    window: Option<pm_protocol::common::PixelRect>,
) -> Option<(Vec<u8>, u32, u32)> {
    let (cw, ch) = (canvas.0.max(1), canvas.1.max(1));
    let (wx, wy, ww, wh) = match window {
        Some(w) => {
            let x = w.x.min(cw);
            let y = w.y.min(ch);
            (x, y, w.width.min(cw - x), w.height.min(ch - y))
        }
        None => (0, 0, cw, ch),
    };
    if ww == 0 || wh == 0 {
        return None;
    }
    let mut out = vec![0u8; (ww as usize) * (wh as usize) * 4];
    let [px, py, pw, ph] = crop;
    for row in 0..ph {
        let y = py + row;
        if y < wy || y >= wy + wh {
            continue;
        }
        for col in 0..pw {
            let x = px + col;
            if x < wx || x >= wx + ww {
                continue;
            }
            let src = ((row * pw + col) * 4) as usize;
            let dst = (((y - wy) * ww + (x - wx)) * 4) as usize;
            if src + 4 <= painted.len() {
                out[dst..dst + 4].copy_from_slice(&painted[src..src + 4]);
            }
        }
    }
    Some((out, ww, wh))
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
                viewer: None,
                wait_ms: 1_000,
                blender_wait_ms: 0,
                captions_hidden: false,
                uploaded: Vec::new(),
                compat_uploaded: Vec::new(),
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
        assert!(
            q.next(|_| Some(4), |_| false, idle(), 0).is_none(),
            "revision 4 < 5"
        );
        assert!(
            q.next(|_| None, |_| false, idle(), 0).is_none(),
            "no document yet"
        );
        assert!(q.next(|_| Some(5), |_| false, idle(), 0).is_some());
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
        let first = q.next(|_| Some(0), |_| false, playing, 0).unwrap();
        assert_eq!(first.request.class, SnapshotClass::Sync);
        // A frame due in 4 ms: no room for a 30 ms async snapshot.
        assert!(q.next(|_| Some(0), |_| false, playing, 0).is_none());
        let room = Pressure {
            due_in_us: Some(100_000),
            ..playing
        };
        assert_eq!(
            q.next(|_| Some(0), |_| false, room, 0)
                .unwrap()
                .request
                .class,
            SnapshotClass::Async
        );
        // Idle never runs during playback, however much room.
        assert!(q.next(|_| Some(0), |_| false, room, 0).is_none());
        // Nor right after a gesture frame.
        let gesture = Pressure {
            last_interactive: Some(1_000_000 - 50_000),
            ..idle()
        };
        assert!(q.next(|_| Some(0), |_| false, gesture, 1_000_000).is_none());
        assert_eq!(
            q.next(|_| Some(0), |_| false, idle(), 1_000_000)
                .unwrap()
                .request
                .class,
            SnapshotClass::Idle
        );
    }

    fn layer_job(class: SnapshotClass, k: Key, revision: Option<u64>) -> SnapshotJob {
        let mut j = job(class, k, revision, 0);
        j.request.target = SnapshotTarget::Layer {
            layer: "l".into(),
            scale: 1.0,
            crop: None,
            content: None,
        };
        j
    }

    /// Layer rasters run on the worker, off the render thread: sync and
    /// async ones do not wait for a playing view's next frame; idle ones
    /// still wait; frame snapshots are never handed to the worker.
    #[test]
    fn layer_rasters_leave_for_the_worker_during_playback() {
        let mut q = SnapshotQueue::default();
        let k = key(3, 1);
        let due_now = Pressure {
            playing: true,
            due_in_us: Some(0),
            last_interactive: None,
        };
        q.push(job(SnapshotClass::Async, k, None, 0)).unwrap();
        q.push(layer_job(SnapshotClass::Idle, k, None)).unwrap();
        q.push(layer_job(SnapshotClass::Async, k, Some(4))).unwrap();
        q.push(layer_job(SnapshotClass::Sync, k, None)).unwrap();
        let first = q.next_layer(|_| Some(3), |_| false, due_now, 0).unwrap();
        assert_eq!(first.request.class, SnapshotClass::Sync);
        // The async raster waits for its revision, never for the frame due.
        assert!(q.next_layer(|_| Some(3), |_| false, due_now, 0).is_none());
        let second = q.next_layer(|_| Some(4), |_| false, due_now, 0).unwrap();
        assert_eq!(second.request.class, SnapshotClass::Async);
        // The idle raster waits for playback to stop (and the async frame
        // snapshot is not the worker's).
        assert!(q.next_layer(|_| Some(4), |_| false, due_now, 0).is_none());
        assert!(
            q.next_layer(|_| Some(4), |_| false, idle(), 0).is_none(),
            "an urgent frame snapshot waits"
        );
        let frame = q.next(|_| Some(4), |_| false, idle(), 0).unwrap();
        assert!(matches!(frame.request.target, SnapshotTarget::Frame { .. }));
        let last = q.next_layer(|_| Some(4), |_| false, idle(), 0).unwrap();
        assert_eq!(last.request.class, SnapshotClass::Idle);
        assert!(q.is_empty());
    }

    #[test]
    fn idle_waits_while_urgent_jobs_wait_for_their_revision() {
        let mut q = SnapshotQueue::default();
        q.push(job(SnapshotClass::Sync, key(3, 1), Some(9), 0))
            .unwrap();
        q.push(job(SnapshotClass::Idle, key(3, 2), None, 0))
            .unwrap();
        assert!(q.next(|_| Some(0), |_| false, idle(), 0).is_none());
    }

    #[test]
    fn a_job_waits_for_fallback_pixels_already_on_their_way() {
        let mut q = SnapshotQueue::default();
        let mut j = job(SnapshotClass::Sync, key(3, 1), None, 0);
        j.waiting_for = vec!["k1".into()];
        q.push(j).unwrap();
        assert!(q.next(|_| Some(0), |k| k == "k1", idle(), 0).is_none());
        assert!(q.next(|_| Some(0), |_| false, idle(), 0).is_some());
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
            q.next(|_| Some(0), |_| false, playing, 0).is_some(),
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
        assert!(matches!(
            q.push(big).unwrap_err().1,
            Refused::TooLarge { .. }
        ));
        // Memory: outstanding results retain the same hard cap. Main taking
        // a result releases its charge immediately, without waiting 60 s.
        let mut m = SnapshotQueue::default();
        let name = pm_bridge::shm::publish(&[7; 16]).unwrap();
        let mut segments = pm_bridge::shm::Segments::default();
        segments.add(name.clone());
        m.published(name.clone(), MAX_PENDING_BYTES);
        assert_eq!(
            m.push(job(SnapshotClass::Sync, key(3, 1), None, 0))
                .unwrap_err()
                .1,
            Refused::Full
        );
        assert_eq!(pm_bridge::shm::take(&name, 16).unwrap(), vec![7; 16]);
        assert!(m.push(job(SnapshotClass::Sync, key(3, 1), None, 0)).is_ok());
        // Layer rasters the worker is drawing left the queue, not memory.
        let mut w = SnapshotQueue::default();
        w.set_in_worker(MAX_PENDING_BYTES);
        assert_eq!(
            w.push(job(SnapshotClass::Sync, key(3, 1), None, 0))
                .unwrap_err()
                .1,
            Refused::Full
        );
        w.set_in_worker(0);
        assert!(w.push(job(SnapshotClass::Sync, key(3, 1), None, 0)).is_ok());
    }

    #[test]
    fn layer_bitmaps_retain_todays_8192_square_cap_without_raising_frame_limits() {
        let mut q = SnapshotQueue::default();
        let mut frame = job(SnapshotClass::Sync, key(9, 1), None, 0);
        frame.request.output_size = Size::new(8192, 8192);
        assert!(matches!(
            q.push(frame).unwrap_err().1,
            Refused::TooLarge {
                limit: MAX_PIXELS,
                ..
            }
        ));
        let mut layer = job(SnapshotClass::Sync, key(9, 1), None, 0);
        layer.request.output_size = Size::new(8192, 8192);
        layer.request.target = pm_protocol::snapshot::SnapshotTarget::Layer {
            layer: "label".into(),
            scale: 32.0,
            crop: None,
            content: None,
        };
        q.push(layer.clone()).unwrap();
        layer.request.output_size = Size::new(8193, 8192);
        assert!(matches!(
            q.push(layer).unwrap_err().1,
            Refused::TooLarge {
                limit: MAX_LAYER_PIXELS,
                ..
            }
        ));
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

    #[test]
    fn cancellation_removes_only_its_host_and_request_id() {
        let mut q = SnapshotQueue::default();
        for (host, id) in [(10, 1), (10, 2), (11, 1)] {
            let mut pending = job(SnapshotClass::Sync, key(3, 1), None, 0);
            pending.host = PortId(host);
            pending.id = RequestId::new(id).unwrap();
            q.push(pending).unwrap();
        }
        assert!(
            q.take_request(PortId(11), RequestId::new(2).unwrap())
                .is_none()
        );
        let cancelled = q
            .take_request(PortId(10), RequestId::new(1).unwrap())
            .unwrap();
        assert_eq!(
            (cancelled.host, cancelled.id),
            (PortId(10), RequestId::new(1).unwrap())
        );
        assert!(
            q.take_request(PortId(10), RequestId::new(1).unwrap())
                .is_none()
        );
        let remaining = [
            q.next(|_| Some(0), |_| false, idle(), 0).unwrap(),
            q.next(|_| Some(0), |_| false, idle(), 0).unwrap(),
        ];
        assert_eq!(
            (remaining[0].host, remaining[0].id),
            (PortId(10), RequestId::new(2).unwrap())
        );
        assert_eq!(
            (remaining[1].host, remaining[1].id),
            (PortId(11), RequestId::new(1).unwrap())
        );
        assert!(q.is_empty());
    }

    fn half(v: [f32; 4]) -> Vec<u8> {
        v.iter()
            .flat_map(|c| pm_gpu::readback::f32_to_half(*c).to_le_bytes())
            .collect()
    }

    #[test]
    fn a_layer_raster_is_placed_in_its_canvas_and_cut_to_the_window() {
        // A 2×1 painted part at (1, 1) of a 4×3 canvas.
        let painted = [9u8; 8];
        let (px, w, h) = place_raster(&painted, [1, 1, 2, 1], (4, 3), None).unwrap();
        assert_eq!((w, h), (4, 3));
        assert_eq!(&px[(4 + 1) * 4..(4 + 3) * 4], &[9; 8]);
        assert_eq!(px.iter().filter(|&&b| b == 9).count(), 8);
        let window = pm_protocol::common::PixelRect {
            x: 2,
            y: 1,
            width: 5,
            height: 5,
        };
        let (px, w, h) = place_raster(&painted, [1, 1, 2, 1], (4, 3), Some(window)).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(&px[..4], &[9; 4]);
        assert_eq!(&px[4..8], &[0; 4]);
        let outside = pm_protocol::common::PixelRect {
            x: 9,
            y: 9,
            width: 2,
            height: 2,
        };
        assert!(place_raster(&painted, [1, 1, 2, 1], (4, 3), Some(outside)).is_none());
    }

    #[test]
    fn float_snapshot_maps_at_its_final_srgb_byte_boundary() {
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
        let mapped = pm_reference::color::css_gamut_map_srgb([1.5, -0.25, 0.0]);
        let expected = mapped.map(|v| (v * 255.0).round_ties_even() as u8);
        assert_eq!(
            encode_rgba8(&px, SnapshotPixels::Opaque),
            vec![expected[0], expected[1], expected[2], 255]
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

    #[test]
    fn source_rgba8_alpha_is_straight_for_canvas_and_filmstrip() {
        let premultiplied = [128, 0, 0, 128, 0, 0, 0, 0];
        assert_eq!(
            encode_rgba8_unorm(&premultiplied, SnapshotPixels::Premultiplied),
            premultiplied,
        );
        assert_eq!(
            encode_rgba8_unorm(&premultiplied, SnapshotPixels::Straight),
            [255, 0, 0, 128, 0, 0, 0, 0],
        );
        let chromium = [255, 129, 64, 128, 0, 0, 0, 0];
        assert_eq!(
            encode_straight_rgba8_unorm(&chromium, SnapshotPixels::Straight),
            chromium
        );
        assert_eq!(
            encode_straight_rgba8_unorm(&chromium, SnapshotPixels::Premultiplied),
            [128, 65, 32, 128, 0, 0, 0, 0],
        );
    }

    #[test]
    fn mapped_rows_match_tightly_packed_encoding_with_padding() {
        let top = half([0.5, 0.25, 0.0, 0.5]);
        let bottom = half([1.0, 0.0, 0.5, 1.0]);
        let mut padded = top.clone();
        padded.extend_from_slice(&[0xa5; 8]);
        padded.extend_from_slice(&bottom);
        let packed = [top.clone(), bottom.clone()].concat();
        for pixels in [
            SnapshotPixels::Opaque,
            SnapshotPixels::Premultiplied,
            SnapshotPixels::Straight,
        ] {
            assert_eq!(
                encode_rgba8_rows(&padded, 8, 16, 2, pixels, pm_render::SnapshotRows::TopDown),
                encode_rgba8(&packed, pixels)
            );
            assert_eq!(
                encode_rgba8_rows(&padded, 8, 16, 2, pixels, pm_render::SnapshotRows::Gl),
                encode_rgba8(&[bottom.clone(), top.clone()].concat(), pixels)
            );
        }
    }

    #[test]
    fn mapped_float32_rows_match_float32_encoding_with_padding() {
        let top = [0.5f32, 0.25, 0.0, 0.5];
        let bottom = [1.0f32, 0.0, 0.5, 1.0];
        let mut padded = Vec::new();
        for value in top {
            padded.extend_from_slice(&value.to_le_bytes());
        }
        padded.extend_from_slice(&[0xa5; 16]);
        for value in bottom {
            padded.extend_from_slice(&value.to_le_bytes());
        }
        for pixels in [
            SnapshotPixels::Opaque,
            SnapshotPixels::Premultiplied,
            SnapshotPixels::Straight,
        ] {
            assert_eq!(
                encode_rgba8_f32_rows(&padded, 16, 32, 2, pixels, pm_render::SnapshotRows::TopDown,),
                encode_rgba8_f32(&[top, bottom].concat(), pixels)
            );
            assert_eq!(
                encode_rgba8_f32_rows(&padded, 16, 32, 2, pixels, pm_render::SnapshotRows::Gl),
                encode_rgba8_f32(&[bottom, top].concat(), pixels)
            );
        }
    }

    #[test]
    fn mapped_rgba8_rows_keep_exact_codes_flip_and_straight_alpha() {
        let top = [64, 32, 16, 128, 9, 7, 5, 0];
        let bottom = [12, 24, 48, 255, 128, 64, 32, 128];
        let mut mapped = top.to_vec();
        mapped.extend([0xa5; 8]);
        mapped.extend(bottom);
        for pixels in [
            SnapshotPixels::Opaque,
            SnapshotPixels::Premultiplied,
            SnapshotPixels::Straight,
        ] {
            for rows in [
                pm_render::SnapshotRows::TopDown,
                pm_render::SnapshotRows::Gl,
            ] {
                let expected = if rows == pm_render::SnapshotRows::TopDown {
                    [top.as_slice(), bottom.as_slice()].concat()
                } else {
                    [bottom.as_slice(), top.as_slice()].concat()
                };
                assert_eq!(
                    encode_mapped_rgba8(
                        wgpu::TextureFormat::Rgba8Unorm,
                        &mapped,
                        8,
                        16,
                        2,
                        pixels,
                        rows,
                    )
                    .unwrap(),
                    encode_rgba8_unorm(&expected, pixels),
                    "{pixels:?} {rows:?}"
                );
            }
        }
        assert_eq!(
            encode_mapped_rgba8(
                wgpu::TextureFormat::Rgba8Unorm,
                &mapped,
                8,
                16,
                2,
                SnapshotPixels::Straight,
                pm_render::SnapshotRows::TopDown,
            )
            .unwrap(),
            vec![
                128, 64, 32, 128, 0, 0, 0, 0, 12, 24, 48, 255, 255, 128, 64, 128
            ]
        );
    }

    #[test]
    fn byte_snapshot_keeps_exact_existing_codes_for_opaque_and_premultiplied() {
        for alpha in u8::MIN..=u8::MAX {
            for value in u8::MIN..=u8::MAX {
                let byte = [value, value, value, alpha];
                assert_eq!(
                    encode_rgba8_unorm(&byte, SnapshotPixels::Opaque),
                    vec![value, value, value, 255],
                    "opaque {byte:?}"
                );
                // An existing RGBA8 source has reached its byte boundary.
                // Preserve every code, including hidden RGB at zero alpha.
                assert_eq!(
                    encode_rgba8_unorm(&byte, SnapshotPixels::Premultiplied),
                    byte.to_vec(),
                    "premultiplied {byte:?}"
                );
            }
        }
    }

    #[test]
    fn straight_float_snapshot_unpremultiplies_before_quantizing() {
        let float = [0.25f32, 0.01, 0.0, 0.5];
        assert_eq!(
            encode_rgba8_f32(&float, SnapshotPixels::Straight),
            vec![128, 5, 0, 128],
        );
        // A genuine byte raster has already crossed its quantization
        // boundary and retains the existing byte unpremultiply policy.
        assert_eq!(
            encode_rgba8_unorm(&[64, 3, 0, 128], SnapshotPixels::Straight),
            vec![128, 6, 0, 128],
        );
    }
}
