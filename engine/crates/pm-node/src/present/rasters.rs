//! Layer rasters off the render thread (`render.raster()`'s base pixels:
//! layer snapshots, RFC 16; docs/engine/extensions-compat.md, round seven).
//!
//! A raster override's base is a layer snapshot, and during playback the
//! renderer asks for one on most frames (a typewriter's typed prefix, a
//! shape whose size animates, paths whose vertices move). Drawn on the
//! presentation thread, each one ran between two timed frames: shapes and
//! paths rasterize on the CPU, and text renders on the GPU and was read back
//! with a device-wide wait, which also waited for every view frame
//! submitted meanwhile. A slow one delayed the next frame's render.
//!
//! The worker owns a second renderer on the same device. It lays out and
//! paints the layer with its own evaluator, text context and fonts (the
//! session's font seeds, a health of its own), submits its own command
//! buffer, and waits only for that submission's readback. It answers main
//! itself; the render thread hears what it published (shared-memory
//! accounting) through [`super::render::Command::RasterDone`]. The pixels
//! are the same function's (`Renderer::layer_raster`) over the same document
//! revision, so a raster is identical to the one the render thread drew.
//!
//! The worker keeps a mirror of each session's document it drew for, kept
//! current by the same updates the render thread applies (as the audio
//! thread's mirror is): an edit never copies the document on the render
//! thread. The first raster of a session seeds the mirror with the render
//! thread's document (`Arc`, no copy there); the worker takes its own copy
//! and lets go, after which the render thread's updates apply in place
//! again. A job names the revision it is for; a mirror at another revision
//! (or none) sends the job back to be seeded afresh
//! ([`super::render::Command::RasterReseed`]). A renderer is made per GPU
//! device: after a device loss the next job builds one on the new device.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use pm_engine::document::DocumentUpdate;
use pm_engine::{Action, LogLevel};
use pm_protocol::codec;
use pm_protocol::common::Revision;
use pm_protocol::envelope::{Envelope, Message, Response};
use pm_protocol::error::{EngineError, EngineProcess, ErrorCode, ErrorScope};
use pm_protocol::frame::{FrameQuality, PreviewResolution, Representation};
use pm_protocol::snapshot::{
    PixelPayload, RasterGeometry, SnapshotOutcome, SnapshotResult, SnapshotTarget,
};
use pm_render::Renderer;

use super::render::{Command, Key, Sink, Stats, now_us};
use super::snapshot::SnapshotJob;

/// Layer rasters handed to the worker and not finished, at most. The
/// renderer keeps two preparations and one speculative one in flight per
/// window; this leaves room for several windows without letting a flood of
/// requests leave the snapshot queue's memory bound.
pub const MAX_IN_FLIGHT: usize = 8;

/// The document a [`RasterJob`] is drawn from on the worker.
pub enum RasterDocument {
    /// The session's mirror, which the worker keeps at the job's revision.
    Mirror,
    /// The render thread's document at the job's revision: the worker
    /// copies it into the session's mirror (its first raster, or after the
    /// mirror was dropped).
    Seed(Arc<pm_model::Document>),
}

/// One layer raster for the worker.
pub struct RasterJob {
    pub job: SnapshotJob,
    pub document: RasterDocument,
    pub revision: Revision,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// Which GPU device `device` is (a new one after a loss).
    pub device_epoch: u64,
}

/// What the worker tells the render thread about a finished job.
#[derive(Debug, Clone)]
pub struct RasterDone {
    pub key: Key,
    /// The job's result bytes (what the worker held for it).
    pub bytes: u64,
    /// A shared-memory segment it published for main, and its length.
    pub published: Option<(String, u64)>,
    /// Host µs from the job's start on the worker to its answer; `None`
    /// when it failed.
    pub render_us: Option<i64>,
    /// Host µs the job waited between dispatch and its start.
    pub queued_us: i64,
    /// The composition frame the raster is of (ticks), for the trace.
    pub time: i64,
}

enum Work {
    /// Builds the renderer on a new device before the first job needs it.
    Warm {
        device: wgpu::Device,
        queue: wgpu::Queue,
        device_epoch: u64,
    },
    Job(Box<RasterJob>, i64),
    /// An update the render thread applied to a session's document.
    Update(Key, DocumentUpdate),
    /// The session is gone, or its mirror can no longer follow.
    Forget(Key),
}

/// The worker thread and the jobs it holds.
pub struct RasterWorker {
    tx: Option<Sender<Work>>,
    thread: Option<JoinHandle<()>>,
    in_flight: usize,
    /// Result bytes of the jobs in flight ([`super::snapshot::SnapshotJob::bytes`]).
    in_flight_bytes: u64,
}

impl RasterWorker {
    /// Starts the worker. `fonts`: the render thread's fonts (the worker
    /// uses a sibling); `back` reaches the render thread.
    pub fn start(
        sink: Sink,
        back: Sender<Command>,
        stats: Arc<Mutex<Stats>>,
        fonts: Option<crate::fonts::render::RenderFonts>,
    ) -> Result<Self, String> {
        let (tx, rx) = channel::<Work>();
        let fonts = fonts.map(|f| f.sibling());
        let thread = pm_engine::qos::spawn_engine_thread(
            EngineProcess::Interactive,
            "pm-viewer-rasters",
            move || {
                pm_bridge::set_thread_user_interactive();
                run(rx, sink, back, stats, fonts)
            },
        )
        .map_err(|e| format!("could not start the layer raster worker: {e}"))?;
        Ok(Self {
            tx: Some(tx),
            thread: Some(thread),
            in_flight: 0,
            in_flight_bytes: 0,
        })
    }

    /// Whether another job may be handed over now.
    pub fn has_room(&self) -> bool {
        self.tx.is_some() && self.in_flight < MAX_IN_FLIGHT
    }

    /// Jobs handed over and not finished.
    pub fn in_flight(&self) -> usize {
        self.in_flight
    }

    /// Their result bytes, which the snapshot queue's memory bound counts.
    pub fn in_flight_bytes(&self) -> u64 {
        self.in_flight_bytes
    }

    /// Builds the worker's renderer on `device` ahead of its first job.
    pub fn warm(&self, device: &wgpu::Device, queue: &wgpu::Queue, device_epoch: u64) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Work::Warm {
                device: device.clone(),
                queue: queue.clone(),
                device_epoch,
            });
        }
    }

    /// Hands a job over; gives it back when the worker is gone.
    pub fn submit(&mut self, job: RasterJob) -> Result<(), Box<RasterJob>> {
        let Some(tx) = &self.tx else {
            return Err(Box::new(job));
        };
        let bytes = job.job.bytes();
        match tx.send(Work::Job(Box::new(job), now_us())) {
            Ok(()) => {
                self.in_flight += 1;
                self.in_flight_bytes += bytes;
                Ok(())
            }
            Err(error) => match error.0 {
                Work::Job(job, _) => Err(job),
                _ => unreachable!("sent a job"),
            },
        }
    }

    /// Hands the worker an update the render thread applied to the
    /// document of session `key` (whose mirror the worker keeps).
    pub fn update(&self, key: Key, update: DocumentUpdate) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Work::Update(key, update));
        }
    }

    /// Drops the worker's mirror of session `key`.
    pub fn forget(&self, key: Key) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Work::Forget(key));
        }
    }

    /// The worker finished (or sent back) a job of `bytes`.
    pub fn finished(&mut self, bytes: u64) {
        self.in_flight = self.in_flight.saturating_sub(1);
        self.in_flight_bytes = self.in_flight_bytes.saturating_sub(bytes);
    }
}

impl Drop for RasterWorker {
    fn drop(&mut self) {
        // Jobs already handed over finish; the thread ends with the queue.
        self.tx.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The worker's renderer, its device, and the font generation installed.
struct Painter {
    device_epoch: u64,
    renderer: Renderer,
    fonts_installed: Option<u64>,
}

fn run(
    rx: Receiver<Work>,
    sink: Sink,
    back: Sender<Command>,
    stats: Arc<Mutex<Stats>>,
    fonts: Option<crate::fonts::render::RenderFonts>,
) {
    let mut painter: Option<Painter> = None;
    let mut mirrors = Mirrors::default();
    let make = |device: wgpu::Device, queue: wgpu::Queue, device_epoch: u64| Painter {
        device_epoch,
        renderer: Renderer::new(device, queue),
        fonts_installed: None,
    };
    while let Ok(work) = rx.recv() {
        match work {
            Work::Warm {
                device,
                queue,
                device_epoch,
            } => {
                if painter
                    .as_ref()
                    .is_none_or(|p| p.device_epoch != device_epoch)
                {
                    painter = Some(make(device, queue, device_epoch));
                }
            }
            Work::Update(key, update) => mirrors.update(key, &update),
            Work::Forget(key) => mirrors.forget(key),
            Work::Job(mut job, dispatched) => {
                let key = job.job.key;
                let bytes = job.job.bytes();
                let source = std::mem::replace(&mut job.document, RasterDocument::Mirror);
                let Some(document) = mirrors.for_job(key, source, job.revision) else {
                    if back.send(Command::RasterReseed(job)).is_err() {
                        return;
                    }
                    continue;
                };
                let started = now_us();
                if painter
                    .as_ref()
                    .is_none_or(|p| p.device_epoch != job.device_epoch)
                {
                    painter = Some(make(
                        job.device.clone(),
                        job.queue.clone(),
                        job.device_epoch,
                    ));
                }
                let p = painter.as_mut().expect("made above");
                if let Some(fonts) = &fonts {
                    fonts.sync(&mut p.renderer, &mut p.fonts_installed);
                }
                let outcome =
                    layer_snapshot(&mut p.renderer, fonts.as_ref(), document, &job, started);
                let time = job.job.request.time.get();
                let done = match outcome {
                    Ok((envelope, published)) => {
                        send(&sink, job.job.host, envelope);
                        let render_us = (now_us() - started).max(0);
                        let mut stats = stats.lock().unwrap_or_else(|e| e.into_inner());
                        stats.snapshots += 1;
                        stats.snapshot_us_max = stats.snapshot_us_max.max(render_us);
                        stats.layer_rasters_off_thread += 1;
                        if published.is_some() {
                            stats.snapshot_shared += 1;
                        }
                        RasterDone {
                            key,
                            bytes,
                            published,
                            render_us: Some(render_us),
                            queued_us: (started - dispatched).max(0),
                            time,
                        }
                    }
                    Err((code, message)) => {
                        stats
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .snapshot_failures += 1;
                        let error = EngineError::new(
                            code,
                            ErrorScope::Request { id: job.job.id },
                            &message,
                        );
                        send(
                            &sink,
                            job.job.host,
                            Envelope::new(Message::Error {
                                id: job.job.id,
                                error,
                            }),
                        );
                        RasterDone {
                            key,
                            bytes,
                            published: None,
                            render_us: None,
                            queued_us: (started - dispatched).max(0),
                            time,
                        }
                    }
                };
                if back.send(Command::RasterDone(Box::new(done))).is_err() {
                    return;
                }
            }
        }
    }
}

/// The worker's mirrors of the documents it draws from, one per session.
#[derive(Default)]
struct Mirrors(HashMap<Key, pm_model::Document>);

impl Mirrors {
    /// An update the render thread applied. A mirror that cannot take it
    /// is dropped (the next job is seeded again).
    fn update(&mut self, key: Key, update: &DocumentUpdate) {
        if let Some(mirror) = self.0.get_mut(&key)
            && update.apply_to(mirror).is_err()
        {
            self.0.remove(&key);
        }
    }

    fn forget(&mut self, key: Key) {
        self.0.remove(&key);
    }

    /// The document a job at `revision` draws from: a seed becomes the
    /// session's mirror (copied here, off the render thread, which then
    /// holds its document alone again). `None`: no mirror at that revision;
    /// it is dropped and the job goes back for a seed.
    fn for_job(
        &mut self,
        key: Key,
        source: RasterDocument,
        revision: Revision,
    ) -> Option<&pm_model::Document> {
        if let RasterDocument::Seed(document) = source {
            self.0.insert(key, Arc::unwrap_or_clone(document));
        }
        if self
            .0
            .get(&key)
            .is_none_or(|m| m.revision() != revision.get())
        {
            self.0.remove(&key);
            return None;
        }
        self.0.get(&key)
    }
}

fn send(sink: &Sink, port: pm_protocol::host::PortId, envelope: Envelope) {
    match codec::encode(&envelope) {
        Ok(bytes) => sink(vec![Action::Send { port, bytes }]),
        Err(error) => sink(vec![Action::Log {
            level: LogLevel::Error,
            message: format!("layer rasters: could not encode a message: {error}"),
        }]),
    }
}

type Failure = (ErrorCode, String);

/// A layer's raster (the base pixels of today's `render.raster()`,
/// `Renderer::layer_raster`): text, captions, editable paths, and shape
/// content, with today's anchor, box, selection, and crop rules. The layer
/// is the document's, or the object the caller passed. Returns the answer's
/// envelope and the shared-memory segment it published, if any.
pub fn layer_snapshot(
    renderer: &mut Renderer,
    fonts: Option<&crate::fonts::render::RenderFonts>,
    document: &pm_model::Document,
    raster_job: &RasterJob,
    started: i64,
) -> Result<(Envelope, Option<(String, u64)>), Failure> {
    let job = &raster_job.job;
    let revision = raster_job.revision;
    let SnapshotTarget::Layer {
        layer,
        scale,
        crop,
        content,
    } = &job.request.target
    else {
        return Err((
            ErrorCode::InvalidMessage,
            "The layer raster worker was given a frame snapshot.".into(),
        ));
    };
    // The caller's layer object, when it is not the document's.
    let content = match content {
        Some(bytes) => match pm_model::json::parse_bytes(&bytes.0) {
            Ok(value) if value.as_object().is_some() => {
                Some(pm_model::Document::new(Some(value), document.revision()))
            }
            _ => {
                return Err((
                    ErrorCode::InvalidMessage,
                    "The layer object of a raster is not a JSON object.".into(),
                ));
            }
        },
        None => None,
    };
    let composition = job.request.composition.as_str();
    let seconds = match pm_eval::Comp::resolve(document, composition) {
        Ok(comp) => comp.seconds_at(job.request.time.get()),
        Err(_) => {
            return Err((
                ErrorCode::InvalidMessage,
                format!("The document has no composition {composition:?}."),
            ));
        }
    };
    // Layer rasters retain their own content, independent of frame captures.
    renderer.set_captions_hidden(false);
    renderer.set_text_seed_scope(pm_render::text::served::SeedScope {
        port: Some(job.key.0.0),
        session: Some(job.key.1.get()),
    });
    let painted = renderer.layer_raster(
        document,
        composition,
        layer.as_str(),
        content.as_ref().and_then(|c| c.root()),
        seconds,
        *scale,
        pm_protocol::color::Precision::Float16,
    );
    if let Some(why) = fonts.and_then(|f| f.after_render()) {
        return Err((
            ErrorCode::Timeout,
            format!("The engine's fonts did not answer for this layer ({why})."),
        ));
    }
    let reply = |outcome: SnapshotOutcome| {
        let mut envelope = Envelope::new(Message::Response {
            id: job.id,
            body: Response::Snapshot(Box::new(outcome)),
        });
        envelope.revision = Some(revision);
        envelope
    };
    let raster = match painted {
        Ok(Some(raster)) => raster,
        Ok(None) => return Ok((reply(SnapshotOutcome::Empty { revision }), None)),
        Err(pm_render::RenderError::Request(why)) => {
            return Err((
                ErrorCode::InvalidMessage,
                format!("The layer raster could not be made: {why}."),
            ));
        }
        Err(pm_render::RenderError::Unsupported(list)) => {
            let reasons: Vec<String> = list.iter().map(super::document::describe).collect();
            let mut message = format!(
                "The new engine cannot draw this layer: {}.",
                reasons.join("; ")
            );
            message.truncate(2_000);
            return Err((ErrorCode::Unsupported, message));
        }
        Err(error) => {
            return Err((
                ErrorCode::Internal,
                format!("The layer raster could not be painted: {error}"),
            ));
        }
    };
    let painted = super::snapshot::encode_rgba8_f32(&raster.pixels.pixels, job.request.pixels);
    let painted_crop = raster.pixels.crop;
    // Today's raster crops plain text and shapes only; the others come
    // back whole whatever the crop asked.
    let window = if raster.crops { *crop } else { None };
    let Some((pixels, width, height)) =
        super::snapshot::place_raster(&painted, painted_crop, raster.canvas, window)
    else {
        return Ok((reply(SnapshotOutcome::Empty { revision }), None));
    };
    if u64::from(width) * u64::from(height) > job.request.max_bytes() / 4 {
        return Err((
            ErrorCode::BudgetExceeded,
            format!(
                "The layer raster is {width}×{height}, over the {}×{} asked for.",
                job.request.output_size.width, job.request.output_size.height
            ),
        ));
    }
    let length = pixels.len();
    let (payload, published) = if length <= pm_protocol::snapshot::INLINE_PIXEL_BYTES {
        (
            PixelPayload::Inline {
                bytes: pm_protocol::common::Bytes(pixels),
            },
            None,
        )
    } else {
        match pm_bridge::shm::publish(&pixels) {
            Ok(name) => (
                PixelPayload::SharedMemory {
                    name: name.clone(),
                    length: pm_protocol::common::ByteSize::new(length as u64)
                        .unwrap_or(pm_protocol::common::ByteSize::MAX),
                },
                Some((name, length as u64)),
            ),
            Err(error) => {
                return Err((
                    ErrorCode::BudgetExceeded,
                    format!("The snapshot's pixels could not be shared: {error}"),
                ));
            }
        }
    };
    let render_us = (now_us() - started).max(0);
    let origin_in_canvas = window.map_or([0, 0], |w| {
        [w.x.min(raster.canvas.0), w.y.min(raster.canvas.1)]
    });
    let result = SnapshotResult {
        revision,
        view_seq: job.request.view_seq,
        width,
        height,
        pixels: job.request.pixels,
        payload,
        origin: [raster.anchor.0, raster.anchor.1],
        layer_box: Some([raster.box_size.0, raster.box_size.1]),
        raster: Some(RasterGeometry {
            canvas: [raster.canvas.0.max(1), raster.canvas.1.max(1)],
            window: origin_in_canvas,
            cropped: window.is_some(),
            selection: raster.selection,
            font_offset: raster.font_offset.map(|(x, y)| [x, y]),
        }),
        quality: FrameQuality {
            resolution: PreviewResolution::Full,
            representation: Representation::Original,
            draft_3d: false,
            cached: false,
            stand_in: false,
        },
        waited_us: (started - job.received).clamp(0, i64::from(u32::MAX)) as u32,
        render_us: render_us.clamp(0, i64::from(u32::MAX)) as u32,
    };
    Ok((reply(SnapshotOutcome::Rendered(result)), published))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::json::Value;
    use pm_protocol::common::SessionId;
    use pm_protocol::host::PortId;

    fn key(session: u64) -> Key {
        (PortId(1), SessionId::new(session).unwrap())
    }

    fn doc(revision: u64, title: &str) -> pm_model::Document {
        let value =
            pm_model::json::parse_bytes(format!(r#"{{"title":"{title}"}}"#).as_bytes()).unwrap();
        pm_model::Document::new(Some(value), revision)
    }

    fn replace(revision: u64, title: &str) -> DocumentUpdate {
        DocumentUpdate::Replace {
            root: doc(revision, title)
                .to_value()
                .map(|value| pm_model::Node::from_value(value, revision)),
            revision: Revision::new(revision).unwrap(),
        }
    }

    fn title(document: &pm_model::Document) -> Option<Value> {
        document
            .root()
            .and_then(|r| r.get("title"))
            .map(|n| n.to_value())
    }

    #[test]
    fn a_seed_becomes_the_mirror_and_updates_keep_it_at_the_render_threads_revision() {
        let mut mirrors = Mirrors::default();
        let seed = Arc::new(doc(3, "a"));
        let held = seed.clone();
        let rev = |r| Revision::new(r).unwrap();
        let mirror = mirrors
            .for_job(key(1), RasterDocument::Seed(seed), rev(3))
            .unwrap();
        assert_eq!(title(mirror), title(&held));
        // The worker let go of the seed: the render thread holds it alone.
        assert_eq!(Arc::strong_count(&held), 1);
        mirrors.update(key(1), &replace(4, "b"));
        let mirror = mirrors
            .for_job(key(1), RasterDocument::Mirror, rev(4))
            .unwrap();
        assert_eq!(mirror.revision(), 4);
        assert_eq!(title(mirror), title(&doc(4, "b")));
        // Another session has no mirror until seeded.
        assert!(
            mirrors
                .for_job(key(2), RasterDocument::Mirror, rev(4))
                .is_none()
        );
    }

    #[test]
    fn a_mirror_at_another_revision_sends_the_job_back_and_is_dropped() {
        let mut mirrors = Mirrors::default();
        let rev = |r| Revision::new(r).unwrap();
        assert!(
            mirrors
                .for_job(key(1), RasterDocument::Seed(Arc::new(doc(3, "a"))), rev(3))
                .is_some()
        );
        // An update the render thread applied but the worker never saw.
        assert!(
            mirrors
                .for_job(key(1), RasterDocument::Mirror, rev(5))
                .is_none()
        );
        // Dropped: even the old revision is no longer served from it.
        assert!(
            mirrors
                .for_job(key(1), RasterDocument::Mirror, rev(3))
                .is_none()
        );
        // A forgotten session is seeded again too.
        mirrors
            .for_job(key(1), RasterDocument::Seed(Arc::new(doc(6, "c"))), rev(6))
            .unwrap();
        mirrors.forget(key(1));
        assert!(
            mirrors
                .for_job(key(1), RasterDocument::Mirror, rev(6))
                .is_none()
        );
    }

    #[test]
    fn an_update_the_mirror_cannot_take_drops_it() {
        let mut mirrors = Mirrors::default();
        let rev = |r| Revision::new(r).unwrap();
        mirrors
            .for_job(key(1), RasterDocument::Seed(Arc::new(doc(3, "a"))), rev(3))
            .unwrap();
        let bad = DocumentUpdate::Patches {
            // A property on a string: JavaScript's strict mode rejects it.
            patches: Arc::from(vec![pm_model::patch::Patch::set(
                vec![
                    pm_model::patch::PathPart::Key("title".into()),
                    pm_model::patch::PathPart::Key("x".into()),
                ],
                Value::Number(1.0),
            )]),
            revision: rev(4),
        };
        mirrors.update(key(1), &bad);
        assert!(
            mirrors
                .for_job(key(1), RasterDocument::Mirror, rev(3))
                .is_none()
        );
    }
}
