//! Engine snapshots, compatibility pixels, and presentation stats: the
//! Phase 3c render transport (contract 2's extension; plan sections 18, 19,
//! and 20; `docs/engine/rfcs/16-render-transport.md`).
//!
//! **Snapshots.** Main asks the interactive engine for one frame of a
//! renderer session's document ([`HostSnapshot`], on main's host port only)
//! and the engine answers with its pixels ([`SnapshotResult`]): inline when
//! small, otherwise in a POSIX shared-memory segment main maps once and
//! unlinks ([`PixelPayload`]). Every snapshot is pinned to a revision (the
//! envelope's revision is the minimum, read-your-writes) and tagged with
//! the revision it rendered and the viewer state the caller saw. They serve
//! the synchronous compatibility path (`render.snapshot`, `renderFrameTo`,
//! and `raster` at apiVersion 1 to 3), their async siblings, and the UI's
//! engine snapshots (thumbnails, library cards, posters, the project
//! thumbnail), told apart by [`SnapshotClass`].
//!
//! **Compatibility pixels.** A renderer uploads pixels it produced for one
//! layer at one frame (an extension's offscreen WebGL2 compatibility
//! context, a `raster` override's result) with [`CompatMessage`], keyed so
//! the engine's caches can use them; the job engine asks the host for the
//! same pixels at export time ([`CompatFrameRequest`]).
//!
//! **Stats.** The interactive engine reports what its presentation costs
//! ([`RenderStats`]) for the status bar and the performance panel.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::common::{
    ByteSize, Bytes, CompositionId, Count, DocumentId, LayerId, PixelRect, Revision, Seq, SessionId,
    Size, Ticks,
};
use crate::frame::{FrameQuality, MAX_OUTPUT_DIMENSION};
use crate::host::PortId;
use crate::validate::{Validate, Violation, ensure};
use crate::version::ContractId;

/// Results at or below this size travel inline; larger ones in shared memory.
pub const INLINE_PIXEL_BYTES: usize = 1024 * 1024;
/// Largest snapshot result (a 16,384 × 16,384 RGBA8 frame is 1 GiB; the
/// engine refuses more than this before it renders).
pub const MAX_SNAPSHOT_BYTES: u64 = 512 * 1024 * 1024;
/// Most motion blur samples a snapshot asks for (`renderFrameTo`'s
/// `mbSamples`).
pub const MAX_BLUR_SAMPLES: u32 = 256;
/// Longest a snapshot may wait in the engine (for its revision and its
/// turn) before it fails with `timeout`.
pub const MAX_SNAPSHOT_WAIT_MS: u32 = 120_000;
/// Longest shared-memory segment name (macOS `PSHMNAMLEN` is 31).
pub const MAX_SHARED_NAME: usize = 31;
/// Largest compatibility pixel band in one message.
pub const MAX_COMPAT_BAND_BYTES: usize = 8 * 1024 * 1024;
/// Largest compatibility raster edge.
pub const MAX_COMPAT_EDGE: u32 = 16384;
/// Longest compatibility cache key (UTF-8 bytes).
pub const MAX_COMPAT_KEY: usize = 512;

/// How a snapshot is scheduled against playback and gestures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum SnapshotClass {
    /// A renderer thread is blocked on it (`render.snapshot`,
    /// `renderFrameTo`, `raster` at apiVersion 1 to 3): rendered at the
    /// first free point of the render thread, between viewer frames.
    Sync,
    /// An async sibling (`snapshotAsync`, `renderFrameToAsync`,
    /// `rasterAsync`): rendered once no viewer frame is due before it would
    /// finish.
    Async,
    /// A UI snapshot (thumbnails, library cards, posters, the project
    /// thumbnail): rendered only while no view plays or gestures, in idle
    /// slices that never hold a viewer frame.
    Idle,
}

/// How the result's pixels are encoded: always RGBA8, rows top-down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum SnapshotPixels {
    /// Premultiplied color over black with alpha 255 (today's
    /// `renderOpaqueFrame` and `toDataURL('image/jpeg')` input).
    Opaque,
    /// Premultiplied color and alpha (today's `renderToPixels` without
    /// `opaque`).
    Premultiplied,
    /// Straight (unpremultiplied) color and alpha (canvas `ImageData`).
    Straight,
}

/// What a snapshot renders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum SnapshotTarget {
    /// The whole composition at `output_size`.
    Frame {
        /// Motion blur samples for layers with their switch on (0: off).
        /// Today's `renderFrameTo` defaults to 16 with the project's
        /// shutter; `snapshot` asks for 0.
        motion_blur_samples: u32,
        /// Draw the composition background (off: transparent captures).
        background: bool,
        /// Native shading may stand in for Blender-rendered compositions
        /// (thumbnails and posters); never for the compatibility path.
        draft_3d: bool,
    },
    /// One layer's raster (the base pixels `render.raster()` returns at
    /// apiVersion 1 to 3), at `scale` output pixels per layer pixel,
    /// optionally cropped to `crop` (in the raster's pixels).
    Layer {
        layer: LayerId,
        scale: f64,
        crop: Option<PixelRect>,
    },
}

/// One snapshot of a composition (or one layer of it) at one time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRequest {
    pub composition: CompositionId,
    pub time: Ticks,
    /// Output size in pixels (for `Layer`, the size cap: the raster is
    /// returned at its own size, never larger than this).
    pub output_size: Size,
    pub target: SnapshotTarget,
    pub pixels: SnapshotPixels,
    pub class: SnapshotClass,
    /// The viewer state the caller saw (`ViewerState::view_seq`), echoed in
    /// the result so a consumer can tie it to what was on screen.
    pub view_seq: Option<Seq>,
    /// Longest the engine waits for the envelope's revision and for its
    /// turn before it answers `timeout` (milliseconds, 1 to
    /// [`MAX_SNAPSHOT_WAIT_MS`]).
    pub wait_ms: u32,
}

impl SnapshotRequest {
    /// Bytes of the RGBA8 result at the output size (the most a `Layer`
    /// target returns).
    pub fn max_bytes(&self) -> u64 {
        u64::from(self.output_size.width) * u64::from(self.output_size.height) * 4
    }
}

impl Validate for SnapshotRequest {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Frame;
        ensure(self.composition.is_valid(), C, || {
            "invalid composition id".into()
        })?;
        ensure(
            !self.output_size.is_empty()
                && self.output_size.width <= MAX_OUTPUT_DIMENSION
                && self.output_size.height <= MAX_OUTPUT_DIMENSION,
            C,
            || format!("snapshot size {:?} outside 1..{MAX_OUTPUT_DIMENSION}", self.output_size),
        )?;
        ensure(self.max_bytes() <= MAX_SNAPSHOT_BYTES, C, || {
            format!(
                "a {}×{} snapshot is over the {MAX_SNAPSHOT_BYTES}-byte cap",
                self.output_size.width, self.output_size.height
            )
        })?;
        ensure(
            (1..=MAX_SNAPSHOT_WAIT_MS).contains(&self.wait_ms),
            C,
            || format!("snapshot wait {} ms outside 1..{MAX_SNAPSHOT_WAIT_MS}", self.wait_ms),
        )?;
        match &self.target {
            SnapshotTarget::Frame {
                motion_blur_samples,
                ..
            } => ensure(*motion_blur_samples <= MAX_BLUR_SAMPLES, C, || {
                format!("{motion_blur_samples} motion blur samples")
            }),
            SnapshotTarget::Layer { layer, scale, crop } => {
                ensure(layer.is_valid(), C, || "invalid layer id".into())?;
                ensure(
                    scale.is_finite()
                        && (crate::frame::MIN_SCALE..=crate::frame::MAX_SCALE).contains(scale),
                    C,
                    || format!("raster scale {scale} outside 0.01..16"),
                )?;
                ensure(crop.is_none_or(|c| !c.is_empty()), C, || {
                    "an empty raster crop".into()
                })
            }
        }
    }
}

/// Main → interactive engine, on main's host port only: a snapshot of the
/// document of `session`, which `renderer` (the window's port) owns. Main
/// names the port from the IPC sender, so a window can only snapshot its
/// own sessions; the engine refuses a session the port does not own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct HostSnapshot {
    pub renderer: PortId,
    pub session: SessionId,
    pub snapshot: SnapshotRequest,
}

impl Validate for HostSnapshot {
    fn validate(&self) -> Result<(), Violation> {
        self.snapshot.validate()
    }
}

/// Where a result's pixels are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum PixelPayload {
    /// At most [`INLINE_PIXEL_BYTES`] bytes in the message.
    Inline { bytes: Bytes },
    /// A POSIX shared-memory segment (`shm_open` name, mode 0600, created
    /// exclusively by the engine). The reader maps `length` bytes once and
    /// unlinks it; the engine unlinks segments nobody read after a minute
    /// and at shutdown.
    SharedMemory { name: String, length: ByteSize },
}

/// A shared-memory segment name the engine makes: `/pm.` and 24 lower-case
/// hex digits (96 random bits).
pub fn shared_name_ok(name: &str) -> bool {
    name.len() <= MAX_SHARED_NAME
        && name.len() == 28
        && name.starts_with("/pm.")
        && name[4..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl PixelPayload {
    pub fn len(&self) -> u64 {
        match self {
            PixelPayload::Inline { bytes } => bytes.0.len() as u64,
            PixelPayload::SharedMemory { length, .. } => length.get(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Pixels of one layer at one frame that only the session's renderer can
/// draw (a `raster` override's result, an extension's compatibility WebGL2
/// context), named so the renderer can draw and upload them
/// ([`CompatMessage::Pixels`]) before it asks again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CompatNeed {
    pub composition: CompositionId,
    pub layer: LayerId,
    pub time: Ticks,
    pub source: CompatSource,
    /// Output pixels per layer pixel the frame draws the layer at.
    pub scale: f64,
}

/// The answer to a [`HostSnapshot`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "outcome"
)]
pub enum SnapshotOutcome {
    Rendered(SnapshotResult),
    /// The frame needs pixels only the session's renderer draws. The engine
    /// never waits on the renderer for a snapshot (the renderer may be the
    /// thread blocked on it): it names what it needs, the renderer draws
    /// and uploads them on the session's port, and asks again. The
    /// fallback requests are the same the viewer's frames send
    /// (`ShaderFallbackMessage::Requests`).
    NeedsRenderer {
        revision: Revision,
        fallback: Vec<crate::shader_fallback::ShaderFallbackRequest>,
        compat: Vec<CompatNeed>,
    },
}

impl Validate for SnapshotOutcome {
    fn validate(&self) -> Result<(), Violation> {
        match self {
            SnapshotOutcome::Rendered(result) => result.validate(),
            SnapshotOutcome::NeedsRenderer {
                fallback, compat, ..
            } => {
                crate::shader_fallback::ShaderFallbackMessage::Requests {
                    requests: fallback.clone(),
                }
                .validate()?;
                ensure(
                    compat.len() <= crate::shader_fallback::MAX_REQUESTS
                        && compat.iter().all(|c| {
                            c.composition.is_valid()
                                && c.layer.is_valid()
                                && c.scale.is_finite()
                                && c.scale > 0.0
                        }),
                    ContractId::Frame,
                    || "invalid compatibility needs".into(),
                )
            }
        }
    }
}

/// A rendered snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SnapshotResult {
    /// The exact revision rendered (≥ the request's minimum).
    pub revision: Revision,
    /// The request's `view_seq`, echoed.
    pub view_seq: Option<Seq>,
    pub width: u32,
    pub height: u32,
    pub pixels: SnapshotPixels,
    pub payload: PixelPayload,
    /// For a `Layer` target: where the raster's top-left sits in the
    /// layer's own pixels (today's `RasterSurface` `x`/`y`, the anchor
    /// offset), at scale 1. Zero for frames.
    pub origin: [f64; 2],
    pub quality: FrameQuality,
    /// Time the request waited in the engine (revision and its turn), and
    /// rendered, in microseconds.
    pub waited_us: u32,
    pub render_us: u32,
}

impl Validate for SnapshotResult {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Frame;
        let expected = u64::from(self.width) * u64::from(self.height) * 4;
        ensure(
            self.width >= 1
                && self.height >= 1
                && self.width <= MAX_OUTPUT_DIMENSION
                && self.height <= MAX_OUTPUT_DIMENSION,
            C,
            || format!("snapshot result {}×{}", self.width, self.height),
        )?;
        ensure(self.payload.len() == expected, C, || {
            format!(
                "{} pixel bytes for a {}×{} RGBA8 result",
                self.payload.len(),
                self.width,
                self.height
            )
        })?;
        match &self.payload {
            PixelPayload::Inline { bytes } => {
                ensure(bytes.0.len() <= INLINE_PIXEL_BYTES, C, || {
                    "inline pixels over the inline limit".into()
                })
            }
            PixelPayload::SharedMemory { name, .. } => {
                ensure(shared_name_ok(name), C, || {
                    "invalid shared-memory name".into()
                })
            }
        }?;
        ensure(self.origin.iter().all(|v| v.is_finite()), C, || {
            "raster origin must be finite".into()
        })
    }
}

/// How compatibility pixels are encoded: rows top-down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum CompatFormat {
    /// 8-bit premultiplied RGBA (a canvas or `readPixels` RGBA8 result).
    Rgba8Premultiplied,
    /// 8-bit straight RGBA (canvas `getImageData`).
    Rgba8Straight,
    /// Binary16 premultiplied RGBA, little-endian (a float target).
    Rgba16fPremultiplied,
}

impl CompatFormat {
    pub const fn bytes_per_pixel(self) -> u64 {
        match self {
            CompatFormat::Rgba8Premultiplied | CompatFormat::Rgba8Straight => 4,
            CompatFormat::Rgba16fPremultiplied => 8,
        }
    }
}

/// What produced compatibility pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum CompatSource {
    /// A `services` `raster` override's result for the layer.
    RasterOverride,
    /// An extension's offscreen WebGL2 compatibility context
    /// (`render.gl.context` at apiVersion 1 to 3).
    CompatContext,
}

/// The identity of one compatibility raster: one layer at one frame, as one
/// producer drew it. `key` is the producer's cache key (today's raster
/// cache key: it changes whenever anything that decides the pixels does).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CompatKey {
    pub composition: CompositionId,
    pub layer: LayerId,
    pub time: Ticks,
    pub source: CompatSource,
    pub key: String,
}

impl CompatKey {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Frame;
        ensure(
            self.composition.is_valid() && self.layer.is_valid(),
            C,
            || "invalid compatibility layer".into(),
        )?;
        ensure(
            !self.key.is_empty() && self.key.len() <= MAX_COMPAT_KEY,
            C,
            || "compatibility key length".into(),
        )
    }
}

/// Rows `row .. row + rows` of a compatibility raster.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CompatPixels {
    pub id: CompatKey,
    /// The document revision the producer drew at.
    pub revision: Revision,
    pub width: u32,
    pub height: u32,
    pub format: CompatFormat,
    /// Where the raster's top-left sits in the layer's own pixels, and its
    /// pixels per layer pixel (today's `RasterSurface` `x`, `y`, and scale).
    pub origin: [f64; 2],
    pub scale: f64,
    pub row: u32,
    pub rows: u32,
    pub data: Bytes,
}

/// Renderer → interactive engine (session-bound, the viewer's port).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "compat"
)]
pub enum CompatMessage {
    /// One band of a raster; the raster is usable once every row arrived.
    Pixels(CompatPixels),
    /// The layer's compatibility rasters are gone (the override or the
    /// extension was disposed, the layer was deleted): drop them all.
    Forget {
        composition: CompositionId,
        layer: LayerId,
    },
}

impl Validate for CompatMessage {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Frame;
        match self {
            CompatMessage::Pixels(p) => {
                p.id.validate()?;
                ensure(
                    (1..=MAX_COMPAT_EDGE).contains(&p.width)
                        && (1..=MAX_COMPAT_EDGE).contains(&p.height),
                    C,
                    || format!("compatibility raster {}×{}", p.width, p.height),
                )?;
                ensure(
                    p.rows >= 1 && u64::from(p.row) + u64::from(p.rows) <= u64::from(p.height),
                    C,
                    || format!("rows {} + {} of {}", p.row, p.rows, p.height),
                )?;
                let expected =
                    u64::from(p.width) * u64::from(p.rows) * p.format.bytes_per_pixel();
                ensure(
                    p.data.0.len() as u64 == expected && p.data.0.len() <= MAX_COMPAT_BAND_BYTES,
                    C,
                    || {
                        format!(
                            "{} compatibility bytes for {} rows of {}",
                            p.data.0.len(),
                            p.rows,
                            p.width
                        )
                    },
                )?;
                ensure(
                    p.origin.iter().all(|v| v.is_finite())
                        && p.scale.is_finite()
                        && p.scale > 0.0
                        && p.scale <= crate::frame::MAX_SCALE,
                    C,
                    || "compatibility origin and scale must be finite".into(),
                )
            }
            CompatMessage::Forget { composition, layer } => ensure(
                composition.is_valid() && layer.is_valid(),
                C,
                || "invalid compatibility layer".into(),
            ),
        }
    }
}

/// Job engine → host (contract 14): the compatibility pixels of one layer at
/// one frame of an export, which only the document's renderer can draw.
/// The host asks the window that owns `document` and answers with
/// [`CompatFrame`], or an `unavailable` error naming why (no window has the
/// document open, the producer is gone).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CompatFrameRequest {
    pub document: DocumentId,
    pub composition: CompositionId,
    pub layer: LayerId,
    pub time: Ticks,
    pub source: CompatSource,
    /// Output pixels per layer pixel the export needs.
    pub scale: f64,
    /// The document revision the export snapshot was taken at.
    pub revision: Revision,
}

impl Validate for CompatFrameRequest {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::HostServices;
        ensure(
            self.document.is_valid() && self.composition.is_valid() && self.layer.is_valid(),
            C,
            || "invalid compatibility frame ids".into(),
        )?;
        ensure(
            self.scale.is_finite() && self.scale > 0.0 && self.scale <= crate::frame::MAX_SCALE,
            C,
            || format!("compatibility scale {}", self.scale),
        )
    }
}

/// The answer to a [`CompatFrameRequest`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CompatFrame {
    pub key: String,
    pub width: u32,
    pub height: u32,
    pub format: CompatFormat,
    pub origin: [f64; 2],
    pub scale: f64,
    pub payload: PixelPayload,
}

impl Validate for CompatFrame {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::HostServices;
        ensure(
            (1..=MAX_COMPAT_EDGE).contains(&self.width)
                && (1..=MAX_COMPAT_EDGE).contains(&self.height),
            C,
            || format!("compatibility frame {}×{}", self.width, self.height),
        )?;
        let expected =
            u64::from(self.width) * u64::from(self.height) * self.format.bytes_per_pixel();
        ensure(self.payload.len() == expected, C, || {
            "compatibility frame bytes".into()
        })?;
        if let PixelPayload::SharedMemory { name, .. } = &self.payload {
            ensure(shared_name_ok(name), C, || {
                "invalid shared-memory name".into()
            })?;
        }
        Ok(())
    }
}

/// Interactive engine → renderer (session-bound): what presenting the
/// session costs, at most twice a second while it presents, and once
/// after it goes idle. Replaces today's `GL.stats` and GPU timing for the
/// status bar and the performance panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RenderStats {
    /// Frames presented to the session since it opened.
    pub frames: Count,
    /// Timed (playback) frames among them, and those skipped as late.
    pub timed_frames: Count,
    pub late_skipped: Count,
    /// The last frame: CPU time to build and submit it, GPU time to finish
    /// it (microseconds), and its layers drawn.
    pub cpu_us: u32,
    pub gpu_us: u32,
    pub layers: u32,
    /// Exponential averages (factor 0.15, today's `E.ms`) of CPU + GPU time.
    pub frame_us_avg: u32,
    /// Translated shader programs resident (today's `GL.progs.size`).
    pub programs: u32,
    /// Cached vector and text rasters (today's raster cache size).
    pub rasters: u32,
    /// Layer render failures since the session opened.
    pub layer_failures: Count,
    /// Snapshots served for the session, and the slowest (microseconds).
    pub snapshots: Count,
    pub snapshot_us_max: u32,
    /// The revision of the newest frame presented.
    pub revision: Revision,
}

/// A viewer frame's tag: the frame a consumer asks about (the depth and id
/// surface, the overlay geometry) is the one presented with these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FrameTag {
    pub revision: Revision,
    pub view_seq: Seq,
    pub pool_generation: Seq,
    pub seq: Seq,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{PreviewResolution, Representation};

    fn request(w: u32, h: u32) -> SnapshotRequest {
        SnapshotRequest {
            composition: "c".into(),
            time: Ticks::ZERO,
            output_size: Size::new(w, h),
            target: SnapshotTarget::Frame {
                motion_blur_samples: 0,
                background: true,
                draft_3d: false,
            },
            pixels: SnapshotPixels::Opaque,
            class: SnapshotClass::Sync,
            view_seq: None,
            wait_ms: 5_000,
        }
    }

    fn quality() -> FrameQuality {
        FrameQuality {
            resolution: PreviewResolution::Full,
            representation: Representation::Original,
            draft_3d: false,
            cached: false,
            stand_in: false,
        }
    }

    #[test]
    fn snapshot_requests_are_bounded() {
        assert!(request(1920, 1080).validate().is_ok());
        assert!(request(0, 1080).validate().is_err());
        assert!(request(16_385, 2).validate().is_err());
        // 16,384 × 16,384 × 4 is over the byte cap.
        assert!(request(16_384, 16_384).validate().is_err());
        let mut slow = request(8, 8);
        slow.wait_ms = 0;
        assert!(slow.validate().is_err());
        slow.wait_ms = MAX_SNAPSHOT_WAIT_MS + 1;
        assert!(slow.validate().is_err());
        let mut blur = request(8, 8);
        blur.target = SnapshotTarget::Frame {
            motion_blur_samples: MAX_BLUR_SAMPLES + 1,
            background: true,
            draft_3d: false,
        };
        assert!(blur.validate().is_err());
        let mut layer = request(8, 8);
        layer.target = SnapshotTarget::Layer {
            layer: "L".into(),
            scale: 0.0,
            crop: None,
        };
        assert!(layer.validate().is_err());
    }

    #[test]
    fn results_match_their_payload() {
        let result = |w: u32, h: u32, payload: PixelPayload| SnapshotResult {
            revision: Revision::ZERO,
            view_seq: None,
            width: w,
            height: h,
            pixels: SnapshotPixels::Opaque,
            payload,
            origin: [0.0, 0.0],
            quality: quality(),
            waited_us: 0,
            render_us: 0,
        };
        assert!(
            result(2, 2, PixelPayload::Inline { bytes: Bytes(vec![0; 16]) })
                .validate()
                .is_ok()
        );
        assert!(
            result(2, 2, PixelPayload::Inline { bytes: Bytes(vec![0; 15]) })
                .validate()
                .is_err()
        );
        let shm = |name: &str| PixelPayload::SharedMemory {
            name: name.into(),
            length: ByteSize::new(4 * 1024 * 1024).unwrap(),
        };
        assert!(
            result(1024, 1024, shm("/pm.0123456789abcdef01234567"))
                .validate()
                .is_ok()
        );
        for bad in ["/pm.0123", "/etc/passwd", "/pm.0123456789ABCDEF01234567", "pm.0123456789abcdef012345678"] {
            assert!(result(1024, 1024, shm(bad)).validate().is_err(), "{bad}");
        }
    }

    #[test]
    fn compat_bands_must_fit_their_rows() {
        let band = |row: u32, rows: u32, len: usize, format: CompatFormat| {
            CompatMessage::Pixels(CompatPixels {
                id: CompatKey {
                    composition: "c".into(),
                    layer: "L".into(),
                    time: Ticks::ZERO,
                    source: CompatSource::RasterOverride,
                    key: "k".into(),
                },
                revision: Revision::ZERO,
                width: 4,
                height: 4,
                format,
                origin: [0.0, 0.0],
                scale: 1.0,
                row,
                rows,
                data: Bytes(vec![0; len]),
            })
        };
        assert!(band(0, 4, 64, CompatFormat::Rgba8Premultiplied).validate().is_ok());
        assert!(band(2, 2, 64, CompatFormat::Rgba16fPremultiplied).validate().is_ok());
        assert!(band(3, 2, 32, CompatFormat::Rgba8Premultiplied).validate().is_err());
        assert!(band(0, 1, 15, CompatFormat::Rgba8Straight).validate().is_err());
    }
}
