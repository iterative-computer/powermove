//! Level 1 SVG rasters for the job engine (decision S1; RFC 16, "SVG
//! rasters"): an export or agent frame that needs a raster asks host
//! services (`compatFrame` with source `svgImage`), which ask the window
//! that owns the document to draw it with today's code. With no such
//! window the host's answer names why, and the frame fails with it.

use std::sync::Arc;

use pm_engine::jobs::JobDocument;
use pm_protocol::common::{DocumentId, Revision, SessionId, Ticks};
use pm_protocol::envelope::{Request, Response};
use pm_protocol::snapshot::{CompatFormat, CompatFrameRequest, CompatSource, PixelPayload};
use pm_render::vector::svg_rasters::{SvgRasterNeed, SvgRasterSource};

use super::Shared;

/// Today's `documentId(PM.proj.id)` (engine-connection/session.ts): the
/// project id, or `untitled`, at most 256 bytes.
pub(super) fn document_id(document: &pm_model::Document) -> Option<DocumentId> {
    let id = document
        .root()
        .and_then(|r| r.get("id"))
        .and_then(|n| match n.kind() {
            pm_model::NodeKind::String(s) => Some(s.to_string_lossy()),
            _ => None,
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "untitled".into());
    let id = if id.len() <= 256 {
        id
    } else {
        id.chars().take(64).collect()
    };
    let id = DocumentId::new(id);
    id.is_valid().then_some(id)
}

/// The host services source of one work item's document.
pub(super) struct HostSvgRasters {
    shared: Arc<Shared>,
    session: SessionId,
    document: Option<DocumentId>,
    revision: Revision,
}

impl HostSvgRasters {
    pub(super) fn new(shared: Arc<Shared>, document: &JobDocument) -> HostSvgRasters {
        HostSvgRasters {
            shared,
            session: document.session,
            document: document_id(&document.document),
            revision: document.revision,
        }
    }
}

impl SvgRasterSource for HostSvgRasters {
    fn fetch(&self, need: &SvgRasterNeed) -> Result<Vec<u8>, String> {
        let key = &need.request.key;
        let document = self
            .document
            .clone()
            .ok_or_else(|| "The project has no document id to find its window by.".to_owned())?;
        let request = CompatFrameRequest {
            document,
            composition: need.composition.as_str().into(),
            layer: need.layer.as_str().into(),
            // The raster does not depend on the time.
            time: Ticks::ZERO,
            source: CompatSource::SvgImage,
            scale: 1.0,
            revision: self.revision,
            svg: Some(need.request.clone()),
            frame: None,
        };
        let frame = match self
            .shared
            .call(Request::CompatFrame(request), Some(self.session))
        {
            Ok(Response::CompatFrame(frame)) => *frame,
            Ok(other) => {
                return Err(format!(
                    "host services answered {other:?} for an SVG raster"
                ));
            }
            Err(e) => return Err(e.message),
        };
        let expected = format!("{}:{}x{}", key.content, key.width, key.height);
        if frame.key != expected
            || frame.width != key.width
            || frame.height != key.height
            || frame.format != CompatFormat::Rgba8Premultiplied
        {
            return Err(format!(
                "The window drew another SVG raster ({} {}x{}) than the {}x{} one asked for.",
                frame.key, frame.width, frame.height, key.width, key.height
            ));
        }
        let data = match frame.payload {
            PixelPayload::Inline { bytes } => bytes.0,
            PixelPayload::SharedMemory { name, length } => {
                pm_bridge::shm::take(&name, length.get() as usize)
                    .map_err(|e| format!("The SVG raster's shared memory could not be read: {e}"))?
            }
        };
        if data.len() as u64 != key.bytes() {
            return Err(format!(
                "{} bytes for a {}x{} SVG raster",
                data.len(),
                key.width,
                key.height
            ));
        }
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_document_id_is_the_windows() {
        let doc = |v: serde_json::Value| {
            pm_model::Document::new(Some(pm_model::json::parse(&v.to_string()).unwrap()), 1)
        };
        let id = |d| document_id(&d).map(|d: DocumentId| d.as_str().to_owned());
        assert_eq!(
            id(doc(serde_json::json!({ "id": "P1" }))).as_deref(),
            Some("P1")
        );
        assert_eq!(
            id(doc(serde_json::json!({ "id": "" }))).as_deref(),
            Some("untitled")
        );
        assert_eq!(
            id(doc(serde_json::json!({ "id": 7 }))).as_deref(),
            Some("untitled")
        );
        let long = "x".repeat(300);
        assert_eq!(
            id(doc(serde_json::json!({ "id": long }))).map(|s| s.len()),
            Some(64)
        );
    }
}
