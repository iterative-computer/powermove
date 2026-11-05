//! Real software source-monitor frames for headless cache/LUT/scope execution.
//! This is a source-media graph; composition graphs use the native renderer port.
use crate::{
    cache_frame::{EngineFrame, FrameRequest, FrameSource, GraphIdentity},
    media::Artifact,
    proxy::{Control, RenditionEncoder, SoftwareEncoder},
    storage::{invalid, regular},
    worker::{info, inspect},
};
use std::io;

pub struct SoftwareSourceFrames {
    encoder: SoftwareEncoder,
    source: Artifact,
    identity: GraphIdentity,
}
impl SoftwareSourceFrames {
    /// Freeze a verified source and exact decoder/conversion version. The host
    /// holds a stable media lease for this adapter's lifetime, including cache
    /// hits, and creates a new adapter on relink or interpretation changes.
    pub fn new(
        encoder: SoftwareEncoder,
        source: Artifact,
        decoder_version: String,
    ) -> io::Result<Self> {
        source.info.validate()?;
        if decoder_version.is_empty() {
            return Err(invalid("missing source decoder version"));
        }
        let graph = blake3::hash(&serde_json::to_vec(&(
            "source-code-readback-v1",
            &source.info,
            &decoder_version,
        ))?);
        let identity = GraphIdentity {
            graph: *graph.as_bytes(),
            media: source.fingerprint,
            renderer_version: decoder_version,
            encoding: source.info.colour.clone(),
        };
        Ok(Self {
            encoder,
            source,
            identity,
        })
    }
}
impl FrameSource for SoftwareSourceFrames {
    fn identity(&self) -> GraphIdentity {
        self.identity.clone()
    }
    fn render(&mut self, request: &FrameRequest) -> io::Result<EngineFrame> {
        request.validate()?;
        if request.identity != self.identity
            || request.width != self.source.info.width
            || request.height != self.source.info.height
        {
            return Err(invalid("source readback identity/dimensions mismatch"));
        }
        regular(&self.source.path)?;
        let probe = self.encoder.probe(&self.source.path)?;
        // Generic YUV readback needs the native colour pipeline: do not let
        // swscale guess an unspecified matrix/transfer/chroma interpretation.
        if !["bgra", "rgba", "argb", "abgr", "rgba64le", "bgra64le"]
            .contains(&probe.pixel_format.as_str())
            && !probe.pixel_format.starts_with("gbr")
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "software scope readback requires RGB source pixels",
            ));
        }
        if inspect(&self.source.path, info(&probe), &mut || Control::Continue)? != self.source {
            return Err(invalid("source changed before readback"));
        }
        let timebase = pm_time::Rational::new(
            i64::from(self.source.info.time_base.numerator),
            i64::from(self.source.info.time_base.denominator),
        )
        .map_err(|e| invalid(&e.to_string()))?;
        let pts = pm_time::timebase_from_ticks(request.ticks, timebase)
            .map_err(|e| invalid(&e.to_string()))?;
        if pm_time::ticks_from_timebase(pts, timebase).map_err(|e| invalid(&e.to_string()))?
            != request.ticks
        {
            return Err(invalid("requested time is not an exact source timestamp"));
        }
        let rgba =
            self.encoder
                .rgba_at_pts(&self.source.path, pts, request.width, request.height)?;
        if inspect(&self.source.path, self.source.info.clone(), &mut || {
            Control::Continue
        })? != self.source
        {
            return Err(invalid("source changed during readback"));
        }
        let frame = EngineFrame {
            request: request.clone(),
            rgba,
        };
        frame.validate()?;
        Ok(frame)
    }
}
