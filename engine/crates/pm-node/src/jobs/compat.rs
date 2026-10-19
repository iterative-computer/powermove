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
}

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
        }
    }

    /// The first reason an answer could not be had.
    pub(crate) fn failure(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn fail(&self, message: String) -> CompatAnswer {
        let mut failure = self.failure.lock().unwrap_or_else(|e| e.into_inner());
        if failure.is_none() {
            *failure = Some(message);
        }
        CompatAnswer::Missing
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
        if self.failure().is_some() {
            return CompatAnswer::Missing;
        }
        let request = CompatFrameRequest {
            document: self.document.as_str().into(),
            composition: composition.into(),
            layer: layer.into(),
            time: ticks,
            source,
            scale,
            revision: self.revision,
            // Layer producers only: SVG rasters have their own source
            // (`pm_render::vector::svg_rasters::SvgRasterSource`).
            svg: None,
        };
        let answer = match self.host.call(Request::CompatFrame(request), self.session) {
            Ok(Response::CompatFrame(frame)) if frame.native => CompatAnswer::Native,
            Ok(Response::CompatFrame(frame)) => {
                let bytes = match frame.payload {
                    PixelPayload::Inline { bytes } => Ok(bytes.0),
                    PixelPayload::SharedMemory { name, length } => {
                        pm_bridge::shm::take(&name, length.get() as usize)
                            .map_err(|e| format!("the layer's pixels could not be read: {e}"))
                    }
                };
                match bytes {
                    Ok(data) => CompatAnswer::Raster(Arc::new(CompatRaster {
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
                    })),
                    Err(message) => return self.fail(message),
                }
            }
            Ok(_) => {
                return self
                    .fail("the host answered a compatibility frame with something else".into());
            }
            Err(message) => return self.fail(message),
        };
        self.answers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, answer.clone());
        answer
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

    #[test]
    fn asks_the_host_once_per_layer_frame_and_density_at_the_job_s_revision() {
        let (host, compat) = job(|r| Ok(frame(r.layer.as_str() == "plain")));
        assert!(compat.producing(CompatSource::RasterOverride));
        assert!(!compat.producing(CompatSource::CompatContext));
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
}
