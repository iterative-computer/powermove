//! Compatibility rasters in exports (plan section 20; owner amendments 23
//! and 24): a `raster` override's layers in an export frame.
//!
//! The exporting window names its producers in the job's settings
//! (`DeliveryOptions::compat`). For every layer today's compositor would
//! rasterize through `PM.raster` (`pm_render::compat`), the job asks the
//! host (`Request::CompatFrame`, contract 14); main asks the editor windows,
//! and the one with the document open runs its override at that frame and
//! answers with the raster or a pass-through. The job waits for the answer
//! (an export has no deadline per frame), so the frame draws exactly what
//! the preview draws. A frame whose answer cannot be had fails the export
//! with the host's reason (no window has the document open, the override
//! went away); it never draws the layer without its override.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pm_protocol::common::{Revision, SessionId};
use pm_protocol::envelope::{Request, Response};
use pm_protocol::snapshot::{CompatFrameRequest, CompatSource, PixelPayload};
use pm_render::compat::{CompatAnswer, CompatRaster, CompatRasters, ticks_of};

/// Answers a job's compatibility layers through the host.
pub(crate) trait CompatHost: Send + Sync {
    fn call(&self, request: Request, session: SessionId) -> Result<Response, String>;
    /// Several requests at once, answered in order (the host serves them
    /// back to back instead of one round trip each).
    fn call_many(&self, requests: Vec<Request>, session: SessionId) -> Vec<Result<Response, String>> {
        requests
            .into_iter()
            .map(|request| self.call(request, session))
            .collect()
    }
}

/// Frames past the asked one whose answers an export fetches with it: one
/// host round trip serves this many frames of a layer.
const FETCH_AHEAD: i64 = 4;
/// Most requests in one batch.
const MAX_BATCH: usize = 512;
/// Most requests sent to the host at once: main draws at most 32 of a
/// window's compatibility frames at a time (`compat-frames.ts` `MAX_ASKS`)
/// and refuses more; a batch goes in turns of this many.
const HOST_ASKS: usize = 24;

type AnswerKey = (String, String, i64, CompatSource, u64);

/// One export job's producers and the answers it got.
pub(crate) struct JobCompat {
    host: Arc<dyn CompatHost>,
    session: SessionId,
    document: String,
    revision: Revision,
    sources: Vec<CompatSource>,
    answers: Mutex<HashMap<AnswerKey, CompatAnswer>>,
    /// Why an answer could not be had (the export fails with it).
    failure: Mutex<Option<String>>,
    /// The earliest time of the last batch (ticks): the frame step is the
    /// distance to the next one.
    last_batch: Mutex<Option<i64>>,
}

impl JobCompat {
    pub(crate) fn new(
        host: Arc<dyn CompatHost>,
        session: SessionId,
        document: String,
        revision: Revision,
        sources: Vec<CompatSource>,
    ) -> JobCompat {
        JobCompat {
            host,
            session,
            document,
            revision,
            sources,
            answers: Mutex::new(HashMap::new()),
            failure: Mutex::new(None),
            last_batch: Mutex::new(None),
        }
    }

    /// The first reason an answer could not be had.
    pub(crate) fn failure(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Records `message` as the failure (the first one wins) and returns it.
    fn fail_message(&self, message: String) -> String {
        let mut failure = self.failure.lock().unwrap_or_else(|e| e.into_inner());
        if failure.is_none() {
            *failure = Some(message.clone());
        }
        message
    }
}

/// RGBA8 of a premultiplied working pixel as today's compositor canvas
/// held it (`GL.render`'s present pass copies the accumulator into an
/// `alpha: false` canvas: the premultiplied display-encoded color over
/// black, opaque).
fn canvas_rgba8(rgba: &[f32], working: pm_export::WorkingEncoding) -> Vec<u8> {
    let mut out = vec![0u8; rgba.len()];
    for (px, o) in rgba.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
        let (c, _) = pm_export::convert::output_rgba(
            [px[0], px[1], px[2], px[3]],
            working,
            pm_export::AlphaOutput::Premultiplied,
        );
        for k in 0..3 {
            let v = if c[k].is_nan() {
                0.0
            } else {
                c[k].clamp(0.0, 1.0)
            };
            o[k] = (v * 255.0).round() as u8;
        }
        o[3] = 255;
    }
    out
}

/// Working values of an opaque canvas frame (the inverse of [`canvas_rgba8`]).
fn working_of(rgba8: &[u8], straight: bool, working: pm_export::WorkingEncoding) -> Vec<f32> {
    let mut out = vec![0f32; rgba8.len()];
    for (px, o) in rgba8.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
        // An `alpha: false` canvas reads back opaque; a straight answer is
        // composited over black as that canvas would show it.
        let a = if straight {
            f64::from(px[3]) / 255.0
        } else {
            1.0
        };
        for k in 0..3 {
            let e = f64::from(px[k]) / 255.0 * a;
            o[k] = match working {
                pm_export::WorkingEncoding::Standard => e,
                pm_export::WorkingEncoding::Linear => pm_export::convert::e_decode(e),
            } as f32;
        }
        o[3] = 1.0;
    }
    out
}

impl JobCompat {
    /// The composition whose export frames a window's compatibility
    /// context draws over (`CompatSource::CompatContext` among the
    /// producers): each frame goes to the window before it is encoded.
    pub(crate) fn context_frames(
        self: &Arc<Self>,
        composition: String,
    ) -> Option<Arc<dyn pm_export::render::FramePost>> {
        if !self.sources.contains(&CompatSource::CompatContext) {
            return None;
        }
        Some(Arc::new(ContextFrames {
            compat: self.clone(),
            composition,
        }))
    }
}

/// Export frames through the window's compatibility context.
struct ContextFrames {
    compat: Arc<JobCompat>,
    composition: String,
}

impl pm_export::render::FramePost for ContextFrames {
    fn frame(
        &self,
        _index: u64,
        time: pm_protocol::common::Ticks,
        width: u32,
        height: u32,
        working: pm_export::WorkingEncoding,
        rgba: Vec<f32>,
    ) -> Result<Vec<f32>, pm_export::render::JobError> {
        let fail = |message: String| {
            let message = self.compat.fail_message(message);
            pm_export::render::JobError::Render(format!(
                "An extension in the editor draws over this export, and its frame could not be drawn: {message}"
            ))
        };
        let bytes = canvas_rgba8(&rgba, working);
        let payload = if bytes.len() <= pm_protocol::snapshot::INLINE_PIXEL_BYTES {
            PixelPayload::Inline {
                bytes: pm_protocol::common::Bytes(bytes),
            }
        } else {
            let length = bytes.len() as u64;
            let name = pm_bridge::shm::publish(&bytes)
                .map_err(|e| fail(format!("the frame could not be shared: {e}")))?;
            PixelPayload::SharedMemory {
                name,
                length: pm_protocol::common::ByteSize::new(length)
                    .map_err(|_| fail("the frame is too large to share".into()))?,
            }
        };
        let request = CompatFrameRequest {
            document: self.compat.document.as_str().into(),
            composition: self.composition.as_str().into(),
            layer: self.composition.as_str().into(),
            time,
            source: CompatSource::CompatContext,
            scale: 1.0,
            revision: self.compat.revision,
            svg: None,
            frame: Some(pm_protocol::snapshot::CompatFrameInput {
                width,
                height,
                payload,
            }),
        };
        match self
            .compat
            .host
            .call(Request::CompatFrame(request), self.compat.session)
        {
            // No handler drew: the frame as the engine rendered it.
            Ok(Response::CompatFrame(frame)) if frame.native => Ok(rgba),
            Ok(Response::CompatFrame(frame)) => {
                if frame.width != width || frame.height != height {
                    return Err(fail(format!(
                        "the window answered a {}×{} frame for a {width}×{height} export",
                        frame.width, frame.height
                    )));
                }
                let straight = match frame.format {
                    pm_protocol::snapshot::CompatFormat::Rgba8Premultiplied => false,
                    pm_protocol::snapshot::CompatFormat::Rgba8Straight => true,
                    other => return Err(fail(format!("the window answered {other:?} pixels"))),
                };
                let data = match frame.payload {
                    PixelPayload::Inline { bytes } => bytes.0,
                    PixelPayload::SharedMemory { name, length } => {
                        pm_bridge::shm::take(&name, length.get() as usize)
                            .map_err(|e| fail(format!("the frame could not be read: {e}")))?
                    }
                };
                if data.len() != rgba.len() {
                    return Err(fail("the window's frame does not match its size".into()));
                }
                Ok(working_of(&data, straight, working))
            }
            Ok(_) => Err(fail(
                "the host answered a compatibility frame with something else".into(),
            )),
            Err(message) => Err(fail(message)),
        }
    }
}

impl CompatRasters for JobCompat {
    fn producing(&self, source: CompatSource) -> bool {
        self.sources.contains(&source)
    }

    fn answer(
        &self,
        composition: &str,
        layer: &str,
        time: f64,
        scale: f64,
        source: CompatSource,
        crop: Option<pm_protocol::common::PixelRect>,
    ) -> CompatAnswer {
        // Export frames never crop (today's `opt.exporting` turns preview
        // source clipping off; `pm_render::compat_crop`).
        debug_assert!(crop.is_none(), "an export frame asked for a crop");
        let ticks = ticks_of(time);
        let key = (
            composition.to_owned(),
            layer.to_owned(),
            ticks.get(),
            source,
            scale.to_bits(),
        );
        if let Some(answer) = self
            .answers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
        {
            return answer.clone();
        }
        // Not answered yet: the frame lists it, and the renderer fetches
        // the frame's answers in one batch ([`Self::fetch`]).
        CompatAnswer::Missing
    }

    fn fetch(&self, needs: &[pm_protocol::snapshot::CompatNeed]) -> bool {
        if self.failure().is_some() || needs.is_empty() {
            return false;
        }
        let key_of = |n: &pm_protocol::snapshot::CompatNeed, time: i64| -> AnswerKey {
            (
                n.composition.as_str().to_owned(),
                n.layer.as_str().to_owned(),
                time,
                n.source,
                n.scale.to_bits(),
            )
        };
        // The frame step: from the last batch's earliest time to this one's.
        let earliest = needs.iter().map(|n| n.time.get()).min().unwrap_or(0);
        let step = {
            let mut last = self.last_batch.lock().unwrap_or_else(|e| e.into_inner());
            let step = last.map_or(0, |l| earliest - l);
            *last = Some(earliest);
            step
        };
        // The frame's needs first, then the same layers at the next frames,
        // in time order (the order today's export ran the chain in).
        let mut asks: Vec<(AnswerKey, bool)> = Vec::new();
        {
            let answers = self.answers.lock().unwrap_or_else(|e| e.into_inner());
            let mut seen = std::collections::HashSet::new();
            let ahead = if step > 0 && step <= pm_protocol::common::TICKS_PER_SECOND {
                FETCH_AHEAD
            } else {
                0
            };
            for k in 0..=ahead {
                for n in needs {
                    let key = key_of(n, n.time.get() + k * step);
                    if answers.contains_key(&key) || !seen.insert(key.clone()) {
                        continue;
                    }
                    if k > 0 && asks.len() >= MAX_BATCH {
                        break;
                    }
                    asks.push((key, k == 0));
                }
            }
            // An export moves forward: answers before this frame are done.
            drop(answers);
            let floor = earliest - step.max(0);
            self.answers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|k, _| k.2 >= floor);
        }
        let requests: Vec<Request> = asks
            .iter()
            .map(|((composition, layer, time, source, scale), _)| {
                Request::CompatFrame(CompatFrameRequest {
                    document: self.document.as_str().into(),
                    composition: composition.as_str().into(),
                    layer: layer.as_str().into(),
                    time: pm_protocol::common::Ticks::new(*time).unwrap_or(pm_protocol::common::Ticks::ZERO),
                    source: *source,
                    scale: f64::from_bits(*scale),
                    revision: self.revision,
                    // Layer producers only: SVG rasters have their own source
                    // (`pm_render::vector::svg_rasters::SvgRasterSource`).
                    svg: None,
                    frame: None,
                })
            })
            .collect();
        let mut results = Vec::with_capacity(asks.len());
        let mut requests = requests.into_iter().peekable();
        while requests.peek().is_some() {
            let turn: Vec<Request> = requests.by_ref().take(HOST_ASKS).collect();
            results.extend(self.host.call_many(turn, self.session));
        }
        let mut answers = self.answers.lock().unwrap_or_else(|e| e.into_inner());
        for ((key, needed), result) in asks.into_iter().zip(results) {
            match answer_of(result) {
                Ok(answer) => {
                    answers.insert(key, answer);
                }
                // A frame ahead that cannot be had is asked for again when
                // its frame comes (and fails then, with its own reason).
                Err(message) if needed => {
                    drop(answers);
                    self.fail_message(message);
                    return false;
                }
                Err(_) => {}
            }
        }
        true
    }
}

/// A host answer to a layer's compatibility request.
fn answer_of(result: Result<Response, String>) -> Result<CompatAnswer, String> {
    match result {
        Ok(Response::CompatFrame(frame)) if frame.native => Ok(CompatAnswer::Native),
        Ok(Response::CompatFrame(frame)) => {
            let data = match frame.payload {
                PixelPayload::Inline { bytes } => bytes.0,
                PixelPayload::SharedMemory { name, length } => {
                    pm_bridge::shm::take(&name, length.get() as usize)
                        .map_err(|e| format!("the layer's pixels could not be read: {e}"))?
                }
            };
            Ok(CompatAnswer::Raster(Arc::new(CompatRaster {
                width: frame.width,
                height: frame.height,
                format: frame.format,
                anchor: frame.origin,
                size: frame.size.unwrap_or([
                    f64::from(frame.width) / frame.scale,
                    f64::from(frame.height) / frame.scale,
                ]),
                uv: frame.uv.unwrap_or([0.0, 0.0, 1.0, 1.0]),
                data: Arc::new(data),
            })))
        }
        Ok(_) => Err("the host answered a compatibility frame with something else".into()),
        Err(message) => Err(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::common::Bytes;
    use pm_protocol::snapshot::{CompatFormat, CompatFrame};

    struct Host {
        calls: Mutex<Vec<CompatFrameRequest>>,
        answer: Box<dyn Fn(&CompatFrameRequest) -> Result<Response, String> + Send + Sync>,
    }

    impl CompatHost for Host {
        fn call(&self, request: Request, _session: SessionId) -> Result<Response, String> {
            let Request::CompatFrame(r) = request else {
                panic!("unexpected request");
            };
            let answer = (self.answer)(&r);
            self.calls.lock().unwrap().push(r);
            answer
        }
    }

    fn job(
        answer: impl Fn(&CompatFrameRequest) -> Result<Response, String> + Send + Sync + 'static,
    ) -> (Arc<Host>, JobCompat) {
        let host = Arc::new(Host {
            calls: Mutex::new(Vec::new()),
            answer: Box::new(answer),
        });
        let compat = JobCompat::new(
            host.clone(),
            SessionId::new(4).unwrap(),
            "P".into(),
            Revision::new(9).unwrap(),
            vec![CompatSource::RasterOverride],
        );
        (host, compat)
    }

    fn frame(native: bool) -> Response {
        Response::CompatFrame(Box::new(CompatFrame {
            key: "k".into(),
            width: if native { 1 } else { 2 },
            height: 1,
            format: CompatFormat::Rgba8Premultiplied,
            origin: [3.0, 4.0],
            scale: 2.0,
            payload: PixelPayload::Inline {
                bytes: Bytes(if native { Vec::new() } else { vec![9; 8] }),
            },
            native,
            size: Some([5.0, 6.0]),
            uv: None,
        }))
    }

    fn need(layer: &str, seconds: f64, scale: f64) -> pm_protocol::snapshot::CompatNeed {
        pm_protocol::snapshot::CompatNeed {
            composition: "root".into(),
            layer: layer.into(),
            time: ticks_of(seconds),
            source: CompatSource::RasterOverride,
            scale,
            crop: None,
        }
    }

    #[test]
    fn asks_the_host_once_per_layer_frame_and_density_at_the_job_s_revision() {
        let (host, compat) = job(|r| Ok(frame(r.layer.as_str() == "plain")));
        assert!(compat.producing(CompatSource::RasterOverride));
        assert!(!compat.producing(CompatSource::CompatContext));
        // A frame lists what it lacks; the renderer fetches it in one batch.
        assert!(matches!(
            compat.answer("root", "typed", 0.5, 2.0, CompatSource::RasterOverride, None),
            CompatAnswer::Missing
        ));
        assert!(host.calls.lock().unwrap().is_empty());
        assert!(compat.fetch(&[need("typed", 0.5, 2.0), need("plain", 0.5, 2.0)]));
        let CompatAnswer::Raster(raster) = compat.answer(
            "root",
            "typed",
            0.5,
            2.0,
            CompatSource::RasterOverride,
            None,
        ) else {
            panic!("expected a raster");
        };
        assert_eq!(
            (
                raster.width,
                raster.height,
                raster.anchor,
                raster.size,
                &raster.data[..]
            ),
            (2, 1, [3.0, 4.0], [5.0, 6.0], &[9u8; 8][..])
        );
        assert!(matches!(
            compat.answer(
                "root",
                "typed",
                0.5,
                2.0,
                CompatSource::RasterOverride,
                None
            ),
            CompatAnswer::Raster(_)
        ));
        assert!(matches!(
            compat.answer(
                "root",
                "plain",
                0.5,
                2.0,
                CompatSource::RasterOverride,
                None
            ),
            CompatAnswer::Native
        ));
        let calls = host.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            (
                calls[0].document.as_str(),
                calls[0].time.get(),
                calls[0].revision.get(),
                calls[0].scale
            ),
            ("P", 352_800_000, 9, 2.0)
        );
    }

    #[test]
    fn a_frame_the_host_cannot_answer_fails_with_its_reason() {
        let (_, compat) = job(|_| Err("open the project in a window to export it".into()));
        assert!(!compat.fetch(&[need("typed", 0.0, 1.0)]));
        assert!(matches!(
            compat.answer(
                "root",
                "typed",
                0.0,
                1.0,
                CompatSource::RasterOverride,
                None
            ),
            CompatAnswer::Missing
        ));
        assert_eq!(
            compat.failure().as_deref(),
            Some("open the project in a window to export it")
        );
    }

    #[test]
    fn later_frames_of_the_same_layers_come_with_one_batch_in_time_order() {
        let (host, compat) = job(|r| Ok(frame(r.layer.as_str() == "plain")));
        let step = 1.0 / 30.0;
        // The first frame: its own needs only (no step known yet).
        assert!(compat.fetch(&[need("typed", 0.0, 2.0), need("plain", 0.0, 2.0)]));
        assert_eq!(host.calls.lock().unwrap().len(), 2);
        // The next frame gives the step: its needs and the same layers at
        // the next frames come in one batch, frame by frame.
        assert!(compat.fetch(&[need("typed", step, 2.0), need("plain", step, 2.0)]));
        let times: Vec<(String, i64)> = host.calls.lock().unwrap()[2..]
            .iter()
            .map(|c| (c.layer.as_str().to_owned(), c.time.get()))
            .collect();
        let at = |k: f64| ticks_of(step).get() + (k * ticks_of(step).get() as f64) as i64;
        assert_eq!(times.len(), 2 * (1 + FETCH_AHEAD as usize));
        assert_eq!(&times[..4], &[("typed".into(), at(0.0)), ("plain".into(), at(0.0)), ("typed".into(), at(1.0)), ("plain".into(), at(1.0))]);
        // Those frames are answered without asking again.
        let asked = host.calls.lock().unwrap().len();
        for k in 1..=FETCH_AHEAD {
            let t = step * (1 + k) as f64;
            assert!(!matches!(
                compat.answer("root", "typed", t, 2.0, CompatSource::RasterOverride, None),
                CompatAnswer::Missing
            ), "frame {k} ahead");
        }
        assert_eq!(host.calls.lock().unwrap().len(), asked);
    }

    #[test]
    fn a_large_frame_is_fetched_in_turns_the_host_accepts() {
        struct Turns {
            sizes: Mutex<Vec<usize>>,
        }
        impl CompatHost for Turns {
            fn call(&self, _request: Request, _session: SessionId) -> Result<Response, String> {
                unreachable!("batched")
            }
            fn call_many(&self, requests: Vec<Request>, _session: SessionId) -> Vec<Result<Response, String>> {
                self.sizes.lock().unwrap().push(requests.len());
                assert!(requests.len() <= HOST_ASKS, "main refuses more than its in-flight bound");
                requests.iter().map(|_| Ok(frame(true))).collect()
            }
        }
        let host = Arc::new(Turns { sizes: Mutex::new(Vec::new()) });
        let compat = JobCompat::new(host.clone(), SessionId::new(4).unwrap(), "P".into(), Revision::new(9).unwrap(), vec![CompatSource::RasterOverride]);
        let needs: Vec<_> = (0..60).map(|i| need(&format!("L{i}"), 0.0, 1.0)).collect();
        assert!(compat.fetch(&needs));
        assert_eq!(host.sizes.lock().unwrap().iter().sum::<usize>(), 60);
        assert!(needs.iter().all(|n| !matches!(compat.answer("root", n.layer.as_str(), 0.0, 1.0, CompatSource::RasterOverride, None), CompatAnswer::Missing)));
    }

    #[test]
    fn a_compat_context_draws_over_export_frames_as_today_s_canvas() {
        use pm_export::WorkingEncoding;
        let host = Arc::new(Host {
            calls: Mutex::new(Vec::new()),
            answer: Box::new(|r: &CompatFrameRequest| {
                let input = r.frame.as_ref().expect("the export frame goes along");
                let PixelPayload::Inline { bytes } = &input.payload else {
                    panic!("a small frame is inline");
                };
                // Pixel 0 is left as it was; pixel 1 is drawn over in red.
                if r.time.get() == 0 {
                    return Ok(frame(true));
                }
                let mut out = bytes.0.clone();
                out[4..8].copy_from_slice(&[255, 0, 0, 255]);
                Ok(Response::CompatFrame(Box::new(CompatFrame {
                    key: "context".into(),
                    width: input.width,
                    height: input.height,
                    format: CompatFormat::Rgba8Premultiplied,
                    origin: [0.0, 0.0],
                    scale: 1.0,
                    payload: PixelPayload::Inline { bytes: Bytes(out) },
                    native: false,
                    size: None,
                    uv: None,
                })))
            }),
        });
        let compat = Arc::new(JobCompat::new(
            host.clone(),
            SessionId::new(4).unwrap(),
            "P".into(),
            Revision::new(9).unwrap(),
            vec![CompatSource::CompatContext],
        ));
        let post = compat
            .context_frames("P-root".into())
            .expect("a context producer");
        // Premultiplied working values: half-transparent white, opaque blue.
        let rgba = vec![0.5, 0.5, 0.5, 0.5, 0.0, 0.0, 1.0, 1.0];
        let t0 = pm_protocol::common::Ticks::ZERO;
        let same = post
            .frame(0, t0, 2, 1, WorkingEncoding::Standard, rgba.clone())
            .unwrap();
        assert_eq!(same, rgba, "no handler drew: the frame as rendered");
        {
            let calls = host.calls.lock().unwrap();
            let r = &calls[0];
            assert_eq!(
                (r.source, r.layer.as_str(), r.composition.as_str()),
                (CompatSource::CompatContext, "P-root", "P-root")
            );
            let PixelPayload::Inline { bytes } = &r.frame.as_ref().unwrap().payload else {
                panic!()
            };
            // Today's canvas: premultiplied color over black, opaque.
            assert_eq!(bytes.0, vec![128, 128, 128, 255, 0, 0, 255, 255]);
        }
        let t1 = pm_protocol::common::Ticks::new(23_520_000).unwrap();
        let drawn = post
            .frame(1, t1, 2, 1, WorkingEncoding::Standard, rgba.clone())
            .unwrap();
        assert_eq!(drawn[4..8], [1.0, 0.0, 0.0, 1.0]);
        assert!((drawn[0] - 128.0 / 255.0).abs() < 1e-6 && drawn[3] == 1.0);
        // Linear blending: E-encoded on the way out, decoded on the way back.
        let linear = post
            .frame(
                1,
                t1,
                2,
                1,
                WorkingEncoding::Linear,
                vec![0.2, 0.2, 0.2, 1.0, 0.0, 0.0, 0.0, 1.0],
            )
            .unwrap();
        assert!((f64::from(linear[0]) - 0.2).abs() < 0.004, "{linear:?}");
        // Without a context producer no frame is sent.
        let (_, raster_only) = job(|_| Ok(frame(true)));
        assert!(
            Arc::new(raster_only)
                .context_frames("P-root".into())
                .is_none()
        );
    }
}
