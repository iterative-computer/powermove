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
//! **Blender frames.** A `Frame` snapshot of a composition that renders
//! with Blender draws Blender's surfaces (today's capture never draws
//! native shading in their place). With `blender_wait_ms` above 0 (today's
//! `snapshotAsync`, which runs `prepareFrame` first) the job first waits
//! for the viewer's Blender worker to render the frame's missing surfaces
//! ([`BlenderPrep`]); that time does not count against `wait_ms`, only
//! against `blender_wait_ms`. With 0 (today's synchronous capture) it draws
//! surfaces an earlier capture prepared, or fails with today's sentence.
//!
//! **No deadlock.** A job never waits on the renderer: one whose document
//! has not reached the asked revision waits (bounded by its own `wait_ms`)
//! for patches the renderer already posted; one that needs pixels only the
//! renderer draws (the shader fallback runner, compatibility pixels) is
//! answered `needsRenderer`, so the renderer draws them and asks again.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use pm_blender::DecodedSurface;
use pm_blender::snapshot::FrameKey;

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

/// Times a job's Blender frame is prepared before it fails (each edit that
/// changes the frame's snapshots while Blender renders asks again).
pub const MAX_BLENDER_ATTEMPTS: u8 = 3;

/// One prepared Blender frame: its snapshot's hash, its frame, and the
/// surface of each layer.
pub type PreparedFrame = ([u8; 32], FrameKey, HashMap<String, Arc<DecodedSurface>>);

/// The Blender stage of a job (`SnapshotRequest::blender_wait_ms` above 0).
#[derive(Clone)]
pub enum BlenderPrep {
    /// The viewer's Blender worker renders the frame's surfaces; the job
    /// does not run, and fails with `timeout` at `until` (host µs).
    Running { token: u64, until: i64 },
    /// What the worker prepared. The job draws them when the frame's
    /// snapshots still hash the same (an edit while Blender rendered asks
    /// again).
    Ready(Arc<Vec<PreparedFrame>>),
}

impl std::fmt::Debug for BlenderPrep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlenderPrep::Running { token, until } => {
                write!(f, "Running {{ token: {token}, until: {until} }}")
            }
            BlenderPrep::Ready(frames) => write!(f, "Ready({} frames)", frames.len()),
        }
    }
}

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
    /// Shader fallback keys whose pixels the renderer posted before asking
    /// (`SnapshotRequest::uploaded`) and that had not landed when the job
    /// last rendered: it waits until none of them is outstanding.
    pub waiting_for: Vec<String>,
    /// Its Blender stage, once it needed one.
    pub blender: Option<BlenderPrep>,
    /// Times its Blender frame was sent to the worker.
    pub blender_attempts: u8,
    /// One deadline shared by every re-plan, starting at the first attempt.
    blender_until: Option<i64>,
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
            blender: None,
            blender_attempts: 0,
            blender_until: None,
        }
    }

    /// The Blender worker's token while it prepares this job's frame.
    pub fn preparing(&self) -> Option<u64> {
        match self.blender {
            Some(BlenderPrep::Running { token, .. }) => Some(token),
            _ => None,
        }
    }

    /// When the job fails with `timeout`: its Blender stage's bound while
    /// that runs, otherwise its own deadline.
    pub fn expires_at(&self) -> i64 {
        match self.blender {
            Some(BlenderPrep::Running { until, .. }) => until,
            _ => self.deadline,
        }
    }

    /// Hands the frame to the Blender worker (`token`) at `now`.
    pub fn start_blender(&mut self, token: u64, now: i64) {
        self.blender_attempts += 1;
        let bound = i64::from(self.request.blender_wait_ms) * 1_000;
        self.blender = Some(BlenderPrep::Running {
            token,
            until: *self.blender_until.get_or_insert(now + bound),
        });
    }

    /// The worker answered at `now`: the job runs again, with its own wait
    /// counted afresh from now (the Blender stage does not count against
    /// it).
    pub fn blender_done(&mut self, frames: Arc<Vec<PreparedFrame>>, now: i64) {
        // A late worker reply cannot revive an expired capture. Leave its
        // running deadline in place for the queue to expire and answer it.
        if self.blender_until.is_some_and(|until| now >= until) {
            return;
        }
        self.blender = Some(BlenderPrep::Ready(frames));
        self.deadline = self
            .deadline
            .max(now + i64::from(self.request.wait_ms) * 1_000);
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
    /// Blender renders its frame's surfaces.
    Blender,
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
        let (gone, kept): (Vec<_>, Vec<_>) =
            self.jobs.drain(..).partition(|j| now >= j.expires_at());
        self.jobs = kept.into();
        gone
    }

    /// The job whose Blender frame the worker prepares under `token`.
    pub fn preparing_mut(&mut self, token: u64) -> Option<&mut SnapshotJob> {
        self.jobs.iter_mut().find(|j| j.preparing() == Some(token))
    }

    /// Removes the job whose Blender frame the worker prepares under
    /// `token` (the worker failed).
    pub fn take_preparing(&mut self, token: u64) -> Option<SnapshotJob> {
        let index = self
            .jobs
            .iter()
            .position(|j| j.preparing() == Some(token))?;
        self.jobs.remove(index)
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
        if job.preparing().is_some() {
            return Ready::Blender;
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
                    .any(|j| j.request.class != SnapshotClass::Idle && j.preparing().is_none());
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
        // A job whose frame Blender renders is woken by the worker's answer.
        self.jobs.iter().any(|j| j.preparing().is_none())
    }

    /// The earliest deadline (host µs).
    pub fn earliest_deadline(&self) -> Option<i64> {
        self.jobs.iter().map(SnapshotJob::expires_at).min()
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

/// Read a frame's RGBA16F or RGBA32F pixels straight from mapped GPU rows into the
/// final RGBA8 allocation. The mapping belongs to this readback, so every
/// snapshot still waits for and converts the frame it just rendered.
pub fn read_rgba8(
    readback: &mut pm_gpu::readback::Readback,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    pixels: SnapshotPixels,
    rows: pm_render::SnapshotRows,
) -> Result<Vec<u8>, pm_gpu::readback::ReadbackError> {
    match texture.format() {
        wgpu::TextureFormat::Rgba16Float => {
            readback.with_mapped_rows(device, queue, texture, |data, row, stride, height| {
                encode_rgba8_rows(data, row, stride, height, pixels, rows)
            })
        }
        wgpu::TextureFormat::Rgba32Float => {
            readback.with_mapped_rows(device, queue, texture, |data, row, stride, height| {
                encode_rgba8_f32_rows(data, row, stride, height, pixels, rows)
            })
        }
        other => Err(pm_gpu::readback::ReadbackError::Format(other)),
    }
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
    match pixels {
        // Already in the encoding: no round trip through floats.
        SnapshotPixels::Premultiplied => rgba.to_vec(),
        _ => {
            let mut out = Vec::with_capacity(rgba.len());
            for px in rgba.chunks_exact(4) {
                let f = |v: u8| f32::from(v) / 255.0;
                push_rgba8(&mut out, [f(px[0]), f(px[1]), f(px[2]), f(px[3])], pixels);
            }
            out
        }
    }
}

fn push_rgba8(out: &mut Vec<u8>, [r, g, b, a]: [f32; 4], pixels: SnapshotPixels) {
    let unorm = |v: f32| -> u8 {
        let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
        (v * 255.0).round_ties_even() as u8
    };
    match pixels {
        SnapshotPixels::Opaque => out.extend_from_slice(&[unorm(r), unorm(g), unorm(b), 255]),
        SnapshotPixels::Premultiplied => {
            out.extend_from_slice(&[unorm(r), unorm(g), unorm(b), unorm(a)])
        }
        SnapshotPixels::Straight => {
            // As a canvas's `getImageData` does: quantize the premultiplied
            // value first, then divide by alpha.
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
                wait_ms: 1_000,
                blender_wait_ms: 0,
                captions_hidden: false,
                uploaded: Vec::new(),
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

    #[test]
    fn a_job_whose_frame_blender_renders_waits_on_its_own_bound_not_its_wait() {
        let mut q = SnapshotQueue::default();
        let mut j = job(SnapshotClass::Async, key(3, 1), None, 0);
        j.request.blender_wait_ms = 600_000;
        j.start_blender(17, 500_000);
        assert_eq!(j.blender_attempts, 1);
        assert_eq!(j.expires_at(), 500_000 + 600_000_000);
        q.push(j).unwrap();
        // It does not run, and the thread need not poll for it (the
        // worker's answer wakes it) ...
        assert!(q.next(|_| Some(0), |_| false, idle(), 600_000).is_none());
        assert!(!q.waiting());
        // ... and an idle job is not held behind it.
        q.push(job(SnapshotClass::Idle, key(3, 2), None, 0))
            .unwrap();
        assert_eq!(
            q.next(|_| Some(0), |_| false, idle(), 600_000)
                .unwrap()
                .request
                .class,
            SnapshotClass::Idle
        );
        // Its own 1 s wait passes while Blender renders: not expired.
        assert!(q.take_expired(5_000_000).is_empty());
        // The worker answers at 9 s: it runs, with its wait counted from then.
        q.preparing_mut(17)
            .unwrap()
            .blender_done(Arc::new(Vec::new()), 9_000_000);
        assert!(q.preparing_mut(17).is_none());
        assert!(q.take_expired(9_500_000).is_empty());
        let ready = q.next(|_| Some(0), |_| false, idle(), 9_500_000).unwrap();
        assert!(matches!(ready.blender, Some(BlenderPrep::Ready(_))));
        assert_eq!(ready.deadline, 10_000_000);
    }

    #[test]
    fn blender_replans_share_one_deadline() {
        let mut j = job(SnapshotClass::Async, key(3, 1), None, 0);
        j.request.blender_wait_ms = 2_000;
        j.start_blender(1, 100_000);
        let until = j.expires_at();
        j.blender_done(Arc::new(Vec::new()), 1_500_000);
        j.start_blender(2, 1_800_000);
        assert_eq!(j.expires_at(), until);
        j.blender_done(Arc::new(Vec::new()), 2_000_000);
        j.start_blender(3, 2_200_000);
        assert!(
            j.expires_at() < 2_200_000,
            "a third attempt has no fresh budget"
        );
        j.blender_done(Arc::new(Vec::new()), 2_300_000);
        assert_eq!(j.preparing(), Some(3), "a late reply stays expired");
        assert_eq!(j.expires_at(), until);
    }

    #[test]
    fn a_blender_stage_past_its_bound_expires_and_a_failed_one_is_taken() {
        let mut q = SnapshotQueue::default();
        let mut a = job(SnapshotClass::Async, key(3, 1), None, 0);
        a.request.blender_wait_ms = 2_000;
        a.start_blender(1, 0);
        q.push(a).unwrap();
        let mut b = job(SnapshotClass::Async, key(3, 1), None, 0);
        b.request.blender_wait_ms = 900_000;
        b.start_blender(2, 0);
        q.push(b).unwrap();
        assert_eq!(q.earliest_deadline(), Some(2_000_000));
        let gone = q.take_expired(2_000_000);
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].preparing(), Some(1));
        assert_eq!(q.take_preparing(2).unwrap().preparing(), Some(2));
        assert!(q.take_preparing(2).is_none());
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
}
