//! Level 1 SVG rasters drawn by Chromium (decision S1; RFC 16 "SVG
//! rasters"; docs/engine/vector.md, "Level 1 SVG images").
//!
//! A `compat` level 1 SVG image is today's `<img>` of the SVG drawn into a
//! canvas at a size bucket (`svgRasterDimensions`) and uploaded as a
//! texture. The engine composites that texture natively, but the raster
//! itself is Chromium's: the renderer that owns the document draws it with
//! today's code and uploads its pixels, once per content and bucket.
//!
//! - Interactive engine → renderer: [`SvgRasterRequest`]s
//!   (`CompatMessage::SvgRequests`), for the buckets a frame drew without.
//! - Renderer → interactive engine: the pixels in row bands
//!   ([`SvgRasterPixels`], `CompatMessage::SvgPixels`), or why it could not
//!   (`CompatMessage::SvgFailed`).
//! - Job engine → host: `CompatFrameRequest` with source `svgImage` and
//!   [`CompatFrameRequest::svg`](crate::snapshot::CompatFrameRequest::svg);
//!   the host asks the document's window, which answers with a
//!   `CompatFrame` of the raster.
//! - Snapshots name missing buckets in `SnapshotOutcome::NeedsRenderer`'s
//!   `svg`; the renderer draws them, posts them, and asks again naming
//!   [`SvgRasterKey::upload_key`] in `uploaded`.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::common::Bytes;
use crate::validate::{Violation, ensure};
use crate::version::ContractId;

/// Largest bucket edge (`svgRasterDimensions`' `maxDimension`).
pub const MAX_SVG_EDGE: u32 = 8192;
/// Largest pixel band in one message.
pub const MAX_SVG_BAND_BYTES: usize = 8 * 1024 * 1024;
/// Most requests in one message.
pub const MAX_SVG_REQUESTS: usize = 64;
/// Longest asset id.
pub const MAX_SVG_ASSET: usize = 512;
/// Longest failure message.
pub const MAX_SVG_MESSAGE: usize = 2048;

/// One raster: the SVG's bytes (SHA-256, lowercase hex) drawn at one
/// bucket. The bucket and the bytes decide the pixels; nothing else does
/// (`drawImage(img, 0, 0, width, height)` into a cleared canvas of that
/// size).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SvgRasterKey {
    pub content: String,
    pub width: u32,
    pub height: u32,
}

impl SvgRasterKey {
    /// The key a snapshot's `uploaded` names once the renderer posted the
    /// raster's bands (`svg:<content>:<width>x<height>`).
    pub fn upload_key(&self) -> String {
        format!("svg:{}:{}x{}", self.content, self.width, self.height)
    }

    pub fn validate(&self) -> Result<(), Violation> {
        let c = ContractId::Frame;
        ensure(
            self.content.len() == 64
                && self
                    .content
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            c,
            || "an SVG raster's content is a lowercase SHA-256".into(),
        )?;
        ensure(
            (1..=MAX_SVG_EDGE).contains(&self.width) && (1..=MAX_SVG_EDGE).contains(&self.height),
            c,
            || format!("SVG raster {}×{}", self.width, self.height),
        )
    }

    /// Bytes of the raster (premultiplied RGBA8).
    pub fn bytes(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * 4
    }
}

/// A raster the engine needs, and the asset whose SVG it is (the renderer
/// draws that asset's decoded `<img>`, after checking its bytes hash to
/// `key.content`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SvgRasterRequest {
    pub key: SvgRasterKey,
    pub asset: String,
}

impl SvgRasterRequest {
    pub fn validate(&self) -> Result<(), Violation> {
        self.key.validate()?;
        ensure(
            !self.asset.is_empty() && self.asset.len() <= MAX_SVG_ASSET,
            ContractId::Frame,
            || "SVG raster asset id length".into(),
        )
    }
}

/// Rows `row .. row + rows` of a raster: premultiplied RGBA8, top-down
/// (the bytes today's `texImage2D` of the canvas stores, read back).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SvgRasterPixels {
    pub key: SvgRasterKey,
    pub row: u32,
    pub rows: u32,
    pub data: Bytes,
}

impl SvgRasterPixels {
    pub fn validate(&self) -> Result<(), Violation> {
        self.key.validate()?;
        let c = ContractId::Frame;
        ensure(
            self.rows >= 1
                && u64::from(self.row) + u64::from(self.rows) <= u64::from(self.key.height),
            c,
            || format!("rows {} + {} of {}", self.row, self.rows, self.key.height),
        )?;
        let expected = u64::from(self.key.width) * u64::from(self.rows) * 4;
        ensure(
            self.data.0.len() as u64 == expected && self.data.0.len() <= MAX_SVG_BAND_BYTES,
            c,
            || {
                format!(
                    "{} SVG raster bytes for {} rows of {}",
                    self.data.0.len(),
                    self.rows,
                    self.key.width
                )
            },
        )
    }
}

/// Validates a request list.
pub fn validate_requests(requests: &[SvgRasterRequest]) -> Result<(), Violation> {
    ensure(
        requests.len() <= MAX_SVG_REQUESTS,
        ContractId::Frame,
        || format!("{} SVG raster requests in one message", requests.len()),
    )?;
    requests.iter().try_for_each(SvgRasterRequest::validate)
}

/// Validates a failure.
pub fn validate_failure(key: &SvgRasterKey, reason: &str, message: &str) -> Result<(), Violation> {
    key.validate()?;
    ensure(
        !reason.is_empty() && reason.len() <= 64 && message.len() <= MAX_SVG_MESSAGE,
        ContractId::Frame,
        || "SVG raster failure length".into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> SvgRasterKey {
        SvgRasterKey {
            content: "0123456789abcdef".repeat(4),
            width: 3,
            height: 2,
        }
    }

    #[test]
    fn keys_and_bands_validate() {
        assert!(key().validate().is_ok());
        assert_eq!(
            key().upload_key(),
            format!("svg:{}:3x2", "0123456789abcdef".repeat(4))
        );
        let upper = SvgRasterKey {
            content: "0123456789ABCDEF".repeat(4),
            ..key()
        };
        assert!(upper.validate().is_err());
        assert!(SvgRasterKey { width: 0, ..key() }.validate().is_err());
        assert!(
            SvgRasterKey {
                width: MAX_SVG_EDGE + 1,
                ..key()
            }
            .validate()
            .is_err()
        );
        let band = |row, rows, len| SvgRasterPixels {
            key: key(),
            row,
            rows,
            data: Bytes(vec![0; len]),
        };
        assert!(band(0, 2, 24).validate().is_ok());
        assert!(band(1, 1, 12).validate().is_ok());
        assert!(band(1, 2, 24).validate().is_err(), "past the last row");
        assert!(band(0, 1, 11).validate().is_err(), "short band");
        assert!(band(0, 0, 0).validate().is_err(), "empty band");
        let request = SvgRasterRequest {
            key: key(),
            asset: String::new(),
        };
        assert!(request.validate().is_err());
        assert!(validate_failure(&key(), "missing", "no such asset").is_ok());
        assert!(validate_failure(&key(), "", "").is_err());
    }

    #[test]
    fn job_engine_frames_name_their_raster_only_for_svg_images() {
        use crate::snapshot::{CompatFrameRequest, CompatSource};
        use crate::validate::Validate;
        let frame = |source, svg| CompatFrameRequest {
            document: "doc-1".into(),
            composition: "comp-1".into(),
            layer: "L1".into(),
            time: crate::common::Ticks::new(0).unwrap(),
            source,
            scale: 1.0,
            revision: crate::common::Revision::ZERO,
            svg,
        };
        let request = SvgRasterRequest {
            key: key(),
            asset: "a1".into(),
        };
        let svg = frame(CompatSource::SvgImage, Some(request.clone()));
        assert!(svg.validate().is_ok());
        assert!(frame(CompatSource::SvgImage, None).validate().is_err());
        assert!(
            frame(CompatSource::RasterOverride, Some(request))
                .validate()
                .is_err()
        );
        assert!(frame(CompatSource::RasterOverride, None).validate().is_ok());
        // Round trip, and an older request without the member still decodes.
        let bytes = rmp_serde::to_vec_named(&svg).unwrap();
        assert_eq!(
            rmp_serde::from_slice::<CompatFrameRequest>(&bytes).unwrap(),
            svg
        );
        let plain = frame(CompatSource::CompatContext, None);
        let json = serde_json::to_value(&plain).unwrap();
        assert!(json.get("svg").is_none());
        assert_eq!(
            serde_json::from_value::<CompatFrameRequest>(json).unwrap(),
            plain
        );
    }
}
