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
    AssetId, ByteSize, Bytes, CompositionId, Count, DocumentId, LayerId, PixelRect, Revision, Seq,
    SessionId, Size, Ticks,
};
use crate::frame::{
    FrameQuality, MAX_OUTPUT_DIMENSION, Projection, Shading3d, ViewOverride, ViewerCanvas,
};
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
/// Longest a snapshot's Blender stage may run ([`SnapshotRequest::blender_wait_ms`]):
/// today's 10-minute Blender job timeout, plus the queue ahead of it.
pub const MAX_BLENDER_WAIT_MS: u32 = 900_000;
/// Longest shared-memory segment name (macOS `PSHMNAMLEN` is 31).
pub const MAX_SHARED_NAME: usize = 31;
/// Largest compatibility pixel band in one message.
pub const MAX_COMPAT_BAND_BYTES: usize = 8 * 1024 * 1024;
/// Largest compatibility raster edge.
pub const MAX_COMPAT_EDGE: u32 = 16384;
/// Today's continuousRasterScale reaches 32 for text/shape/captions.
/// This is raster density, independent of the output frame's scale limit.
/// It bounds the compositor's compatibility rasters (needs, answers, export
/// requests), which only ever ask for `continuousRasterScale`; a public
/// layer raster ([`SnapshotTarget::Layer`]) has no upper bound.
pub const MAX_RASTER_SCALE: f64 = 32.0;

/// A public layer raster's scale (`render.raster(layer, scale)` at
/// apiVersion 1 to 3): any positive finite number. Today's `PM.raster`
/// (legacy `gl/raster.ts`) takes any scale and caps only the density, at
/// `min(scale, 8192 / w, 8192 / h)`, which the engine's rasterizers apply
/// themselves, so a small layer at scale 50 or 0.005 is a valid surface.
pub fn valid_layer_raster_scale(scale: f64) -> bool {
    scale.is_finite() && scale > 0.0
}
/// Longest compatibility cache key (UTF-8 bytes).
pub const MAX_COMPAT_KEY: usize = 512;
/// Largest layer object a `Layer` snapshot may carry (canonical JSON).
pub const MAX_LAYER_CONTENT_BYTES: usize = 4 * 1024 * 1024;

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
    /// optionally cropped to `crop` (in the raster's pixels). Any positive
    /// finite `scale` ([`valid_layer_raster_scale`]): today's `PM.raster`
    /// has no upper bound, its density is `min(scale, 8192 / w, 8192 / h)`.
    Layer {
        layer: LayerId,
        scale: f64,
        crop: Option<PixelRect>,
        /// The layer object the caller passed, as canonical JSON, when it is
        /// not the document's own (a raster wrapper's modified copy): drawn
        /// in the composition as the document's layer `layer` would be.
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional = nullable))]
        content: Option<Bytes>,
    },
    /// A source frame at `SnapshotRequest::time`, independently of the composition.
    Source {
        asset: AssetId,
        fit: SourceFit,
        color: SourceColor,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum SourceFit {
    Stretch,
    Contain,
    Cover,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum SourceColor {
    /// Browser Canvas2D display pixels, used by posters and filmstrips.
    Display,
    /// Native decoded video planes and source colour tags for Chromium video
    /// presentation. Never an RGBA compatibility or composition conversion.
    Compositor,
    Interpreted,
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
    /// A frozen viewer's original canvas and preview view. When present,
    /// render that canvas before its CSS box is scaled into the stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    pub viewer: Option<SnapshotViewer>,
    /// Longest the engine waits for the envelope's revision and for its
    /// turn before it answers `timeout` (milliseconds, 1 to
    /// [`MAX_SNAPSHOT_WAIT_MS`]).
    pub wait_ms: u32,
    /// Blender frames (a `Frame` target of a composition with
    /// `render3d.enabled`, not `draft_3d`): 0, today's synchronous capture,
    /// draws the Blender surfaces an earlier capture prepared or fails with
    /// today's "The Blender frame is not prepared" sentence; above 0,
    /// today's `snapshotAsync` (which runs `prepareFrame` first), the
    /// engine has Blender render the frame's missing surfaces before it
    /// draws, for at most this long (milliseconds, up to
    /// [`MAX_BLENDER_WAIT_MS`]). The time the Blender stage runs does not
    /// count against `wait_ms`.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(as = "Option<u32>", optional))]
    pub blender_wait_ms: u32,
    /// `PM.captionsHidden`: omit caption layers from this frame capture.
    /// Per request; it never changes the viewer or a later capture.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub captions_hidden: bool,
    /// Keys of shader fallback requests (from an earlier `needsRenderer`)
    /// the caller drew and posted on the session's port before asking
    /// again: the engine waits for those pixels (they are already on their
    /// way) instead of naming them again.
    pub uploaded: Vec<String>,
    /// Compatibility needs the caller answered and posted after the last
    /// `needsRenderer` round. Only these answers are already on their way:
    /// a viewer ask may still need the caller's now-blocked event loop.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<CompatNeed>>", optional))]
    pub compat_uploaded: Vec<CompatNeed>,
}

/// Immutable view selected by a viewer snapshot. This never changes the
/// session's live viewer state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SnapshotViewer {
    pub view: Option<ViewOverride>,
    pub canvas: ViewerCanvas,
    pub view_seq: Seq,
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
            || {
                format!(
                    "snapshot size {:?} outside 1..{MAX_OUTPUT_DIMENSION}",
                    self.output_size
                )
            },
        )?;
        ensure(self.max_bytes() <= MAX_SNAPSHOT_BYTES, C, || {
            format!(
                "a {}×{} snapshot is over the {MAX_SNAPSHOT_BYTES}-byte cap",
                self.output_size.width, self.output_size.height
            )
        })?;
        if let Some(viewer) = &self.viewer {
            let crop = viewer.canvas.crop;
            let css = viewer.canvas.css;
            let positive_rect = |r: crate::common::Rect| {
                [r.x, r.y, r.width, r.height].iter().all(|v| v.is_finite())
                    && r.width > 0.0
                    && r.height > 0.0
            };
            ensure(
                matches!(self.target, SnapshotTarget::Frame { .. }),
                C,
                || "a viewer snapshot must target a frame".into(),
            )?;
            ensure(
                self.output_size == Size::new(viewer.canvas.width, viewer.canvas.height),
                C,
                || "viewer snapshot size must match its canvas backing size".into(),
            )?;
            ensure(
                self.view_seq.is_none_or(|seq| seq == viewer.view_seq),
                C,
                || "viewer snapshot viewSeq conflicts with its pinned viewer".into(),
            )?;
            ensure(positive_rect(crop) && positive_rect(css), C, || {
                "viewer snapshot canvas crop and CSS box must be finite and positive".into()
            })?;
            if let Some(view) = &viewer.view {
                ensure(
                    !view.xray || matches!(view.shading, Shading3d::Wireframe | Shading3d::Solid),
                    C,
                    || "X-ray applies in draft shading modes only".into(),
                )?;
                ensure(view.local_view.iter().all(|id| id.is_valid()), C, || {
                    "invalid Local View layer id".into()
                })?;
                if let Some(camera) = &view.camera {
                    ensure(camera.view.iter().all(|v| v.is_finite()), C, || {
                        "viewer snapshot camera matrix must be finite".into()
                    })?;
                    let valid_projection = match camera.projection {
                        Projection::Perspective {
                            fov_y_degrees,
                            near,
                            far,
                        } => {
                            fov_y_degrees.is_finite()
                                && fov_y_degrees > 0.0
                                && fov_y_degrees < 180.0
                                && near.is_finite()
                                && near > 0.0
                                && far.is_finite()
                                && far > near
                        }
                        Projection::Orthographic {
                            half_height,
                            near,
                            far,
                        } => {
                            half_height.is_finite()
                                && half_height > 0.0
                                && near.is_finite()
                                && far.is_finite()
                                && far > near
                        }
                    };
                    ensure(valid_projection, C, || {
                        "viewer snapshot camera projection is invalid".into()
                    })?;
                }
            }
        }
        ensure(
            (1..=MAX_SNAPSHOT_WAIT_MS).contains(&self.wait_ms),
            C,
            || {
                format!(
                    "snapshot wait {} ms outside 1..{MAX_SNAPSHOT_WAIT_MS}",
                    self.wait_ms
                )
            },
        )?;
        ensure(self.blender_wait_ms <= MAX_BLENDER_WAIT_MS, C, || {
            format!(
                "Blender wait {} ms over {MAX_BLENDER_WAIT_MS}",
                self.blender_wait_ms
            )
        })?;
        // A blocked thread never waits for Blender.
        ensure(
            self.blender_wait_ms == 0 || self.class != SnapshotClass::Sync,
            C,
            || "a sync snapshot cannot wait for Blender".into(),
        )?;
        ensure(
            self.uploaded.len() <= crate::shader_fallback::MAX_REQUESTS
                && self
                    .uploaded
                    .iter()
                    .all(|k| !k.is_empty() && k.len() <= MAX_COMPAT_KEY),
            C,
            || "too many uploaded keys".into(),
        )?;
        ensure(self.compat_uploaded.len() <= MAX_COMPAT_NEEDS, C, || {
            "too many uploaded compatibility needs".into()
        })?;
        for need in &self.compat_uploaded {
            ensure(
                need.composition.is_valid() && need.layer.is_valid(),
                C,
                || "invalid uploaded compatibility layer".into(),
            )?;
            valid_crop(need.crop)?;
            valid_scale(need.scale)?;
        }
        match &self.target {
            SnapshotTarget::Source { asset, color, .. } => {
                ensure(
                    *color != SourceColor::Compositor
                        || (self.class != SnapshotClass::Sync
                            && self.pixels == SnapshotPixels::Straight),
                    C,
                    || "native video presentation requires an async straight source request".into(),
                )?;
                ensure(asset.is_valid() && self.time.get() >= 0, C, || {
                    "source snapshots need an asset and nonnegative source time".into()
                })
            }
            SnapshotTarget::Frame {
                motion_blur_samples,
                ..
            } => ensure(*motion_blur_samples <= MAX_BLUR_SAMPLES, C, || {
                format!("{motion_blur_samples} motion blur samples")
            }),
            SnapshotTarget::Layer {
                layer,
                scale,
                crop,
                content,
            } => {
                ensure(layer.is_valid(), C, || "invalid layer id".into())?;
                ensure(
                    content
                        .as_ref()
                        .is_none_or(|c| !c.0.is_empty() && c.0.len() <= MAX_LAYER_CONTENT_BYTES),
                    C,
                    || "layer content outside 1 byte..4 MiB".into(),
                )?;
                ensure(valid_layer_raster_scale(*scale), C, || {
                    format!("layer raster scale {scale} is not a positive finite number")
                })?;
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
    /// Today's preview crop window (`legacy/gl/shape-raster-window.ts`, in
    /// the raster's pixels at `scale`) the producer is called with, as the
    /// compositor passes it to `PM.raster` for a viewer frame; absent: the
    /// whole raster (exports and snapshots never crop). Part of the need's
    /// identity: another window is another answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub crop: Option<crate::common::PixelRect>,
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
        /// Level 1 SVG rasters the frame needs (decision S1): the renderer
        /// draws them, posts them as `CompatMessage::SvgPixels`, and asks
        /// again naming each `SvgRasterKey::upload_key` in `uploaded`.
        #[serde(default)]
        svg: Vec<crate::svg_raster::SvgRasterRequest>,
    },
    /// A `Layer` target that draws nothing at that time (no ink, no cue
    /// showing): today's `render.raster()` returns `null`.
    Empty {
        revision: Revision,
    },
}

impl Validate for SnapshotOutcome {
    fn validate(&self) -> Result<(), Violation> {
        match self {
            SnapshotOutcome::Rendered(result) => result.validate(),
            SnapshotOutcome::Empty { .. } => Ok(()),
            SnapshotOutcome::NeedsRenderer {
                fallback,
                compat,
                svg,
                ..
            } => {
                crate::shader_fallback::ShaderFallbackMessage::Requests {
                    requests: fallback.clone(),
                }
                .validate()?;
                crate::svg_raster::validate_requests(svg)?;
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

/// Source video planes remain encoded in the stream's colour space. Chromium
/// samples and colour-manages them exactly as its CSS video compositor does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SourceVideoFrame {
    // WebCodecs buffer layout: nv12, planar i420a, i422 or i420p10, or rgba.
    pub format: String,
    // gfx::ColorSpace / WebCodecs identifiers describing the decoded frame.
    pub primaries: String,
    pub transfer: String,
    pub matrix: String,
    pub range: String,
    // Visible width when Chromium retains a wider coded row (VP9 stride).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub visible_width: Option<u32>,
}

impl SourceVideoFrame {
    /// Straight sRGB RGBA8 the engine converted itself (a reading Chromium
    /// cannot apply exactly).
    pub fn srgb_rgba() -> Self {
        Self {
            format: "rgba".into(),
            primaries: "bt709".into(),
            transfer: "srgb".into(),
            matrix: "rgb".into(),
            range: "full".into(),
            visible_width: None,
        }
    }

    /// Y then interleaved (nv12) or planar chroma, then i420a's alpha, or
    /// RGBA8; no padding. i420p10 samples are 16-bit little-endian.
    pub fn byte_len(&self, width: u32, height: u32) -> Option<u64> {
        if self.visible_width.is_some_and(|w| w == 0 || w > width) {
            return None;
        }
        let w = u64::from(width);
        let h = u64::from(height);
        if self.format == "rgba" {
            return (w * h).checked_mul(4);
        }
        let chroma_h = if self.format == "i422" {
            h
        } else {
            h.div_ceil(2)
        };
        let bytes = if self.format == "i420p10" { 2 } else { 1 };
        let alpha = if self.format == "i420a" { w * h } else { 0 };
        (w * h + w.div_ceil(2) * 2 * chroma_h)
            .checked_mul(bytes)?
            .checked_add(alpha)
    }
}

impl Validate for SourceVideoFrame {
    fn validate(&self) -> Result<(), Violation> {
        // Only tags Chromium applies exactly as the engine reads them
        // (`pm_render` chooses them); everything else arrives as RGBA.
        let rgba = self.format == "rgba";
        let ok = self
            .visible_width
            .is_none_or(|w| w > 0 && w <= MAX_OUTPUT_DIMENSION)
            && if rgba {
                *self == Self::srgb_rgba()
            } else {
                matches!(self.format.as_str(), "nv12" | "i420a" | "i422" | "i420p10")
                    && matches!(self.primaries.as_str(), "bt709" | "smpte170m" | "bt470bg")
                    && matches!(self.transfer.as_str(), "bt709" | "linear")
                    && matches!(self.matrix.as_str(), "bt709" | "smpte170m" | "bt2020-ncl")
                    && matches!(self.range.as_str(), "limited" | "full")
            };
        ensure(ok, ContractId::Frame, || {
            "invalid source video format or colour tags".into()
        })
    }
}

/// A rendered snapshot. `video` is present only for SourceColor::Compositor:
/// packed native YUV planes, or engine-converted RGBA, for a source-only
/// VideoFrame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SnapshotResult {
    /// The exact revision rendered (≥ the request's minimum).
    pub revision: Revision,
    /// The selected viewer's `view_seq`, or a legacy request's echoed `view_seq`.
    pub view_seq: Option<Seq>,
    pub width: u32,
    pub height: u32,
    pub pixels: SnapshotPixels,
    pub payload: PixelPayload,
    /// For a `Layer` target: the anchor in the layer's own pixels (today's
    /// `RasterSurface` `anchorX`, `anchorY`). Zero for frames.
    pub origin: [f64; 2],
    /// For a `Layer` target: the raster's box in layer pixels (today's
    /// `RasterSurface` `w`, `h`); the pixels are that box at the asked
    /// scale. `None` for frames.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    pub layer_box: Option<[f64; 2]>,
    /// For a `Layer` target: the rest of today's raster geometry.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    pub raster: Option<RasterGeometry>,
    pub quality: FrameQuality,
    /// Time the request waited in the engine (revision and its turn), and
    /// rendered, in microseconds.
    pub waited_us: u32,
    pub render_us: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    pub video: Option<SourceVideoFrame>,
}

/// A layer raster's geometry besides its anchor and box (today's
/// `RasterSurface` fields the pixels alone do not give).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RasterGeometry {
    /// The raster's whole canvas in pixels.
    pub canvas: [u32; 2],
    /// Where the result's pixels sit in the canvas (a crop window's origin;
    /// zero when the whole canvas came back).
    pub window: [u32; 2],
    /// Whether the asked crop was applied (today's raster crops plain text
    /// and shapes only).
    pub cropped: bool,
    /// Today's `selection` (`x0`, `y0`, `x1`, `y1` in layer pixels).
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    pub selection: Option<[f64; 4]>,
    /// Today's `fontOffset` (`x`, `y`): a text layer's typography anchoring
    /// shift, when it has one.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    pub font_offset: Option<[f64; 2]>,
}

impl Validate for SnapshotResult {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Frame;
        if let Some(video) = &self.video {
            video.validate()?;
            ensure(self.pixels == SnapshotPixels::Straight, C, || {
                "source planes are straight encoded samples".into()
            })?;
        }
        let expected = self
            .video
            .as_ref()
            .map_or(
                Some(u64::from(self.width) * u64::from(self.height) * 4),
                |v| v.byte_len(self.width, self.height),
            )
            .unwrap_or(u64::MAX);
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
                "{} pixel bytes for a {}×{} snapshot result",
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
            PixelPayload::SharedMemory { name, .. } => ensure(shared_name_ok(name), C, || {
                "invalid shared-memory name".into()
            }),
        }?;
        ensure(self.origin.iter().all(|v| v.is_finite()), C, || {
            "raster origin must be finite".into()
        })?;
        ensure(
            self.raster.as_ref().is_none_or(|r| {
                r.canvas
                    .iter()
                    .all(|v| (1..=MAX_OUTPUT_DIMENSION).contains(v))
                    && r.selection.is_none_or(|s| s.iter().all(|v| v.is_finite()))
            }),
            C,
            || "raster geometry must be finite and within the output limits".into(),
        )
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
    /// A complete Canvas2D `toBlob("image/png")` file for a Blender map.
    Png,
}

impl CompatFormat {
    pub const fn bytes_per_pixel(self) -> u64 {
        match self {
            CompatFormat::Rgba8Premultiplied | CompatFormat::Rgba8Straight => 4,
            CompatFormat::Rgba16fPremultiplied => 8,
            CompatFormat::Png => 0,
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
    /// A level 1 SVG image's raster at a size bucket, drawn by Chromium
    /// (decision S1; [`CompatFrameRequest::svg`] names it).
    SvgImage,
    /// Chromium's decoded image, drawn into Canvas2D and encoded as PNG.
    BlenderPng,
}

/// The image whose exact Chromium Canvas2D PNG Blender needs. `content` is
/// SHA-256 of the original file, so a window with a stale asset cannot answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BlenderPngRequest {
    pub asset: String,
    pub content: crate::common::ContentHash,
}

impl Validate for BlenderPngRequest {
    fn validate(&self) -> Result<(), Violation> {
        ensure(
            !self.asset.is_empty() && self.asset.len() <= 256,
            ContractId::HostServices,
            || "invalid Blender PNG asset id".into(),
        )
    }
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
    /// The raster's size in layer pixels (today's `RasterSurface` `w`,
    /// `h`, the quad the compositor draws it on). Absent: the pixel size
    /// over `scale`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub size: Option<[f64; 2]>,
    /// The part of the raster the quad samples (today's `uv`: offset and
    /// scale, 0..1). Absent: the whole raster.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub uv: Option<[f64; 4]>,
    /// The crop window of the need this raster answers ([`CompatNeed::crop`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub crop: Option<crate::common::PixelRect>,
}

/// A layer at one frame that a producer passed through: the engine draws it
/// natively (a `raster` override that returned the base raster unchanged).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CompatNative {
    pub composition: CompositionId,
    pub layer: LayerId,
    pub time: Ticks,
    pub source: CompatSource,
    /// Output pixels per layer pixel the producer was asked at.
    pub scale: f64,
    /// The document revision the producer decided at.
    pub revision: Revision,
    /// The crop window of the need it answers ([`CompatNeed::crop`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub crop: Option<crate::common::PixelRect>,
}

/// Renderer ↔ interactive engine (session-bound, the viewer's port):
/// `Pixels`, `Forget`, `Producers`, `Native`, `Reuse`, `SvgPixels`, and
/// `SvgFailed` go to the engine, `Needs` and `SvgRequests` to the renderer.
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
    /// Renderer → engine: the producers the session has now (a registered
    /// `raster` override, a drawing compatibility context), replacing the
    /// previous list. Without a producer for a source the engine never asks
    /// for it; a change drops every answer of the sources that went.
    Producers { sources: Vec<CompatSource> },
    /// Renderer → engine: the producer passed the layer through at this
    /// frame; the engine draws it itself.
    Native(CompatNative),
    /// Renderer → engine: the raster stored under `id` (already uploaded)
    /// is the producer's answer at `revision` and `scale` too (its cache
    /// key did not change), so no pixels are sent again.
    Reuse {
        id: CompatKey,
        scale: f64,
        revision: Revision,
        /// The crop window of the need it answers ([`CompatNeed::crop`]).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        crop: Option<crate::common::PixelRect>,
    },
    /// Engine → renderer: what a viewer frame at `revision` needs before it
    /// can be drawn. The renderer answers each need with `Pixels`,
    /// `Native`, or `Reuse`; the engine draws the frame once every answer
    /// arrived (the shader fallback runner's pattern, decision 19).
    Needs {
        revision: Revision,
        needs: Vec<CompatNeed>,
    },
    /// Interactive engine → renderer: level 1 SVG rasters a frame drew
    /// without (`crate::svg_raster`).
    SvgRequests {
        requests: Vec<crate::svg_raster::SvgRasterRequest>,
    },
    /// Renderer → interactive engine: one band of an SVG raster.
    SvgPixels(crate::svg_raster::SvgRasterPixels),
    /// Renderer → interactive engine: the renderer could not draw an SVG
    /// raster (`missing`: no such asset; `stale`: the window's bytes hash
    /// differently; `decode`: Chromium could not draw it; `size`).
    SvgFailed {
        key: crate::svg_raster::SvgRasterKey,
        reason: String,
        message: String,
    },
}

/// Most needs in one [`CompatMessage::Needs`].
pub const MAX_COMPAT_NEEDS: usize = 256;
/// Most producers a session names.
pub const MAX_COMPAT_SOURCES: usize = 4;

impl Validate for CompatMessage {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Frame;
        match self {
            CompatMessage::Pixels(p) => {
                ensure(p.format != CompatFormat::Png, C, || {
                    "PNG is not a compatibility pixel band".into()
                })?;
                p.id.validate()?;
                valid_crop(p.crop)?;
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
                let expected = u64::from(p.width) * u64::from(p.rows) * p.format.bytes_per_pixel();
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
                ensure(p.origin.iter().all(|v| v.is_finite()), C, || {
                    "compatibility origin must be finite".into()
                })?;
                valid_scale(p.scale)?;
                ensure(
                    p.size
                        .is_none_or(|s| s.iter().all(|v| v.is_finite() && *v > 0.0))
                        && p.uv.is_none_or(|u| u.iter().all(|v| v.is_finite())),
                    C,
                    || "compatibility size and uv must be finite".into(),
                )
            }
            CompatMessage::Forget { composition, layer } => {
                ensure(composition.is_valid() && layer.is_valid(), C, || {
                    "invalid compatibility layer".into()
                })
            }
            CompatMessage::Producers { sources } => {
                ensure(sources.len() <= MAX_COMPAT_SOURCES, C, || {
                    format!("{} compatibility producers", sources.len())
                })
            }
            CompatMessage::Native(n) => {
                ensure(n.composition.is_valid() && n.layer.is_valid(), C, || {
                    "invalid compatibility layer".into()
                })?;
                valid_crop(n.crop)?;
                valid_scale(n.scale)
            }
            CompatMessage::Reuse {
                id, scale, crop, ..
            } => {
                id.validate()?;
                valid_crop(*crop)?;
                valid_scale(*scale)
            }
            CompatMessage::Needs { needs, .. } => {
                ensure(
                    !needs.is_empty() && needs.len() <= MAX_COMPAT_NEEDS,
                    C,
                    || format!("{} compatibility needs", needs.len()),
                )?;
                for need in needs {
                    ensure(
                        need.composition.is_valid() && need.layer.is_valid(),
                        C,
                        || "invalid compatibility layer".into(),
                    )?;
                    valid_crop(need.crop)?;
                    valid_scale(need.scale)?;
                }
                Ok(())
            }
            CompatMessage::SvgRequests { requests } => {
                crate::svg_raster::validate_requests(requests)
            }
            CompatMessage::SvgPixels(p) => p.validate(),
            CompatMessage::SvgFailed {
                key,
                reason,
                message,
            } => crate::svg_raster::validate_failure(key, reason, message),
        }
    }
}

/// A crop window is a non-empty part of a raster no larger than today's
/// largest (8192 × 8192 device pixels, `textRasterGeometry`).
fn valid_crop(crop: Option<crate::common::PixelRect>) -> Result<(), Violation> {
    crop.map_or(Ok(()), |c| {
        ensure(
            !c.is_empty()
                && u64::from(c.x) + u64::from(c.width) <= MAX_COMPAT_CROP
                && u64::from(c.y) + u64::from(c.height) <= MAX_COMPAT_CROP,
            ContractId::Frame,
            || format!("compatibility crop {c:?}"),
        )
    })
}

/// The largest crop edge: the raster edge today allows (8192 device pixels).
pub const MAX_COMPAT_CROP: u64 = 8192;

fn valid_scale(scale: f64) -> Result<(), Violation> {
    ensure(
        scale.is_finite() && scale > 0.0 && scale <= MAX_RASTER_SCALE,
        ContractId::Frame,
        || format!("compatibility raster scale {scale} outside 0..=32 (exclusive zero)"),
    )
}

/// Job engine → host (contract 14): the compatibility pixels of one layer at
/// one frame of an export, which only the document's renderer can draw.
/// The host asks the window that owns `document` and answers with
/// [`CompatFrame`], or an `unavailable` error naming why (no window has the
/// document open, the producer is gone).
/// Output pixels per layer pixel the export needs.
/// The document revision the export snapshot was taken at.
/// For [`CompatSource::SvgImage`] (and only then): the raster (its
/// SVG's content hash and bucket) and the asset. The answer is that
/// bucket's premultiplied RGBA8 pixels, keyed
/// `<content>:<width>x<height>`.
/// Set only for `blenderPng`; the original file's content identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CompatFrameRequest {
    pub document: DocumentId,
    pub composition: CompositionId,
    pub layer: LayerId,
    pub time: Ticks,
    pub source: CompatSource,
    pub scale: f64,
    pub revision: Revision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub svg: Option<crate::svg_raster::SvgRasterRequest>,
    /// For [`CompatSource::CompatContext`] (and only then): the export
    /// frame as today's compositor canvas held it before `frame:rendered`
    /// (RGBA8, top-down, opaque, display-encoded). The window runs its
    /// `frame:rendered` handlers over it in the compatibility context and
    /// answers the frame they leave, or a pass-through when none drew
    /// (docs/engine/extensions-compat.md, "Exports"). `layer` names the
    /// composition then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub frame: Option<CompatFrameInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub blender: Option<BlenderPngRequest>,
}

/// An export frame a compatibility context draws over
/// ([`CompatFrameRequest::frame`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CompatFrameInput {
    pub width: u32,
    pub height: u32,
    /// `width × height × 4` bytes.
    pub payload: PixelPayload,
}

impl Validate for CompatFrameInput {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::HostServices;
        ensure(
            (1..=MAX_OUTPUT_DIMENSION).contains(&self.width)
                && (1..=MAX_OUTPUT_DIMENSION).contains(&self.height),
            C,
            || format!("compatibility frame {}×{}", self.width, self.height),
        )?;
        let expected = u64::from(self.width) * u64::from(self.height) * 4;
        ensure(self.payload.len() == expected, C, || {
            format!(
                "{} bytes for a {}×{} compatibility frame",
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
            PixelPayload::SharedMemory { name, .. } => ensure(shared_name_ok(name), C, || {
                "invalid shared-memory name".into()
            }),
        }
    }
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
            (self.source == CompatSource::SvgImage) == self.svg.is_some(),
            C,
            || "an svgImage compatibility frame names its SVG raster, and only it".into(),
        )?;
        if let Some(svg) = &self.svg {
            svg.validate()?;
        }
        ensure(
            (self.source == CompatSource::BlenderPng) == self.blender.is_some(),
            C,
            || "a blenderPng compatibility frame names its source image, and only it".into(),
        )?;
        if let Some(blender) = &self.blender {
            blender.validate()?;
        }
        ensure(
            (self.source == CompatSource::CompatContext) == self.frame.is_some(),
            C,
            || "a compatContext frame carries the export frame, and only it".into(),
        )?;
        if let Some(frame) = &self.frame {
            frame.validate()?;
        }
        ensure(
            self.scale.is_finite() && self.scale > 0.0 && self.scale <= MAX_RASTER_SCALE,
            C,
            || {
                format!(
                    "compatibility raster scale {} outside 0..=32 (exclusive zero)",
                    self.scale
                )
            },
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
    /// The producer passed the layer through: the engine draws it itself
    /// (the payload is empty).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub native: bool,
    /// The quad in layer pixels and the part of the raster it samples (as
    /// [`CompatPixels`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub size: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub uv: Option<[f64; 4]>,
}

impl Validate for CompatFrame {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::HostServices;
        ensure(
            self.scale.is_finite() && self.scale > 0.0 && self.scale <= MAX_RASTER_SCALE,
            C,
            || {
                format!(
                    "compatibility raster scale {} outside 0..=32 (exclusive zero)",
                    self.scale
                )
            },
        )?;
        if self.native {
            return ensure(self.payload.is_empty(), C, || {
                "a pass-through carries no pixels".into()
            });
        }
        ensure(
            (1..=MAX_COMPAT_EDGE).contains(&self.width)
                && (1..=MAX_COMPAT_EDGE).contains(&self.height),
            C,
            || format!("compatibility frame {}×{}", self.width, self.height),
        )?;
        ensure(
            self.size
                .is_none_or(|s| s.iter().all(|v| v.is_finite() && *v > 0.0))
                && self.uv.is_none_or(|u| u.iter().all(|v| v.is_finite())),
            C,
            || "compatibility size and uv must be finite".into(),
        )?;
        if self.format == CompatFormat::Png {
            ensure(
                (33..=256 * 1024 * 1024).contains(&self.payload.len()),
                C,
                || "Blender PNG size".into(),
            )?;
            if let PixelPayload::Inline { bytes } = &self.payload {
                let b = &bytes.0;
                ensure(
                    b.starts_with(b"\x89PNG\r\n\x1a\n")
                        && b.get(12..16) == Some(b"IHDR".as_slice())
                        && b.get(16..20) == Some(self.width.to_be_bytes().as_slice())
                        && b.get(20..24) == Some(self.height.to_be_bytes().as_slice()),
                    C,
                    || "Blender PNG header or dimensions".into(),
                )?;
            }
        } else {
            let expected =
                u64::from(self.width) * u64::from(self.height) * self.format.bytes_per_pixel();
            ensure(self.payload.len() == expected, C, || {
                "compatibility frame bytes".into()
            })?;
        }
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
    /// The session's audio playback health, once its audio started
    /// (absent for a session that has not opened audio).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub audio: Option<AudioDiagnostics>,
}

/// A session's audio playback health in [`RenderStats`], for the status bar
/// and the performance panel. Counters run since the session's audio
/// started; a report goes out when any of them changes, paused or not.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AudioDiagnostics {
    /// Why no output device opened (the run follows the host clock).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub error: Option<String>,
    /// Distinct playback problems, oldest first (at most 16): media that
    /// did not resolve, mix and transport errors, and voices dropped to
    /// silence after their source missed the bounded wait (issue A3).
    pub problems: Vec<String>,
    /// Segments a missing source block held (each counted once).
    pub starved_renders: Count,
    /// Device buffers the mixer did not fill in time (audible gaps).
    pub underruns: Count,
    /// Output device cycles that overran their real-time deadline.
    pub device_overloads: Count,
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
            viewer: None,
            wait_ms: 5_000,
            blender_wait_ms: 0,
            captions_hidden: false,
            uploaded: Vec::new(),
            compat_uploaded: Vec::new(),
        }
    }

    #[test]
    fn source_target_keeps_source_time_fit_and_color_on_the_wire() {
        let mut source = request(160, 90);
        source.time = Ticks::from_whole_seconds(7);
        source.target = SnapshotTarget::Source {
            asset: "source-a".into(),
            fit: SourceFit::Contain,
            color: SourceColor::Interpreted,
        };
        assert!(source.validate().is_ok());
        let bytes = rmp_serde::to_vec_named(&source).unwrap();
        let read: SnapshotRequest = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(read, source);
        assert_eq!(
            serde_json::to_value(&source.target).unwrap(),
            serde_json::json!({
                "kind": "source", "asset": "source-a", "fit": "contain", "color": "interpreted"
            })
        );
        source.time = Ticks::new(-1).unwrap();
        assert!(source.validate().is_err());
    }

    #[test]
    fn source_compositor_intent_is_distinct_from_canvas_display() {
        let mut source = request(160, 90);
        source.target = SnapshotTarget::Source {
            asset: "source-a".into(),
            fit: SourceFit::Contain,
            color: SourceColor::Compositor,
        };
        assert!(source.validate().is_err());
        source.class = SnapshotClass::Async;
        source.pixels = SnapshotPixels::Straight;
        assert!(source.validate().is_ok());
        let bytes = rmp_serde::to_vec_named(&source).unwrap();
        assert_eq!(
            rmp_serde::from_slice::<SnapshotRequest>(&bytes).unwrap(),
            source
        );
        assert_eq!(
            serde_json::to_value(&source.target).unwrap()["color"],
            "compositor"
        );
        assert_ne!(SourceColor::Compositor, SourceColor::Display);
    }

    #[test]
    fn native_video_planes_are_bounded_and_validate_colour_tags() {
        let mut video = SourceVideoFrame {
            format: "nv12".into(),
            primaries: "smpte170m".into(),
            transfer: "bt709".into(),
            matrix: "smpte170m".into(),
            range: "limited".into(),
            visible_width: None,
        };
        assert!(video.validate().is_ok());
        assert_eq!(video.byte_len(160, 90), Some(21600));
        assert_eq!(video.byte_len(3, 3), Some(17));
        let wire = rmp_serde::to_vec_named(&video).unwrap();
        assert_eq!(
            rmp_serde::from_slice::<SourceVideoFrame>(&wire).unwrap(),
            video
        );
        video.visible_width = Some(160);
        assert_eq!(video.byte_len(224, 90), Some(30240));
        assert_eq!(video.byte_len(159, 90), None);
        video.visible_width = None;
        video.primaries = "unspecified".into();
        assert!(video.validate().is_err(), "the reading is always complete");
        video.primaries = "bt470bg".into();
        video.transfer = "pq".into();
        assert!(video.validate().is_err(), "HDR converts in the engine");
        video.transfer = "linear".into();
        assert!(video.validate().is_ok());
        let rgba = SourceVideoFrame::srgb_rgba();
        assert!(rgba.validate().is_ok());
        assert_eq!(rgba.byte_len(160, 90), Some(57600));
        assert!(
            SourceVideoFrame {
                range: "limited".into(),
                ..rgba
            }
            .validate()
            .is_err()
        );
        video.format = "i420a".into();
        assert_eq!(video.byte_len(160, 90), Some(36000));
        assert_eq!(video.byte_len(3, 3), Some(26));
        video.format = "i420p10".into();
        assert_eq!(video.byte_len(160, 90), Some(43200));
        video.format = "i422".into();
        assert_eq!(video.byte_len(160, 90), Some(28800));
        video.transfer = "guessed-profile".into();
        assert!(video.validate().is_err());
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
            content: None,
        };
        assert!(layer.validate().is_err());
        // Blender waits: async and idle only, and bounded.
        let mut blender = request(8, 8);
        blender.blender_wait_ms = 60_000;
        assert!(
            blender.validate().is_err(),
            "a sync snapshot never waits for Blender"
        );
        blender.class = SnapshotClass::Async;
        assert!(blender.validate().is_ok());
        blender.blender_wait_ms = MAX_BLENDER_WAIT_MS + 1;
        assert!(blender.validate().is_err());
        // An older peer's request without the field reads as 0.
        let mut value = serde_json::to_value(request(8, 8)).unwrap();
        value.as_object_mut().unwrap().remove("blenderWaitMs");
        let read: SnapshotRequest = serde_json::from_value(value).unwrap();
        assert_eq!(read.blender_wait_ms, 0);
    }

    #[test]
    fn viewer_snapshot_pins_a_valid_canvas_and_sequence() {
        use crate::common::Rect;
        use crate::frame::ViewerCanvas;

        let mut capture = request(320, 180);
        capture.viewer = Some(SnapshotViewer {
            view: None,
            canvas: ViewerCanvas {
                width: 320,
                height: 180,
                crop: Rect {
                    x: 80.0,
                    y: 40.0,
                    width: 160.0,
                    height: 90.0,
                },
                css: Rect {
                    x: 8.0,
                    y: 12.0,
                    width: 640.0,
                    height: 360.0,
                },
            },
            view_seq: Seq::new(7).unwrap(),
        });
        assert!(capture.validate().is_ok());
        let wire = rmp_serde::to_vec_named(&capture).unwrap();
        let decoded: SnapshotRequest = rmp_serde::from_slice(&wire).unwrap();
        assert_eq!(decoded, capture);

        capture.view_seq = Some(Seq::new(6).unwrap());
        assert!(capture.validate().is_err());
        capture.view_seq = Some(Seq::new(7).unwrap());
        capture.output_size.width = 319;
        assert!(capture.validate().is_err());
        capture.output_size.width = 320;
        capture.viewer.as_mut().unwrap().canvas.crop.width = 0.0;
        assert!(capture.validate().is_err());
        capture.viewer.as_mut().unwrap().canvas.crop.width = f64::NAN;
        assert!(capture.validate().is_err());
        capture.viewer.as_mut().unwrap().canvas.crop.width = 160.0;
        capture.viewer.as_mut().unwrap().canvas.css.height = 0.0;
        assert!(capture.validate().is_err());
        capture.viewer.as_mut().unwrap().canvas.css.height = 360.0;
        capture.viewer.as_mut().unwrap().view = Some(ViewOverride {
            camera: Some(crate::frame::EditorCamera {
                view: crate::common::MAT4_IDENTITY,
                projection: Projection::Orthographic {
                    half_height: 3.0,
                    near: -10.0,
                    far: 10.0,
                },
            }),
            shading: Shading3d::Solid,
            xray: true,
            local_view: vec!["L".into()],
        });
        assert!(
            capture.validate().is_ok(),
            "negative orthographic near is valid"
        );
        capture
            .viewer
            .as_mut()
            .unwrap()
            .view
            .as_mut()
            .unwrap()
            .camera
            .as_mut()
            .unwrap()
            .view[0] = f64::NAN;
        assert!(capture.validate().is_err());
        capture
            .viewer
            .as_mut()
            .unwrap()
            .view
            .as_mut()
            .unwrap()
            .camera
            .as_mut()
            .unwrap()
            .view[0] = 1.0;
        capture
            .viewer
            .as_mut()
            .unwrap()
            .view
            .as_mut()
            .unwrap()
            .camera
            .as_mut()
            .unwrap()
            .projection = Projection::Perspective {
            fov_y_degrees: 45.0,
            near: 0.1,
            far: 100.0,
        };
        assert!(capture.validate().is_ok());
        capture
            .viewer
            .as_mut()
            .unwrap()
            .view
            .as_mut()
            .unwrap()
            .camera
            .as_mut()
            .unwrap()
            .projection = Projection::Perspective {
            fov_y_degrees: 45.0,
            near: 1.0,
            far: 0.5,
        };
        assert!(capture.validate().is_err());
        capture
            .viewer
            .as_mut()
            .unwrap()
            .view
            .as_mut()
            .unwrap()
            .camera
            .as_mut()
            .unwrap()
            .projection = Projection::Perspective {
            fov_y_degrees: 45.0,
            near: 0.1,
            far: 100.0,
        };
        capture
            .viewer
            .as_mut()
            .unwrap()
            .view
            .as_mut()
            .unwrap()
            .shading = Shading3d::Rendered;
        assert!(capture.validate().is_err(), "X-ray requires draft shading");
        capture.viewer.as_mut().unwrap().view.as_mut().unwrap().xray = false;
        assert!(capture.validate().is_ok());
        capture.target = SnapshotTarget::Layer {
            layer: "L".into(),
            scale: 1.0,
            crop: None,
            content: None,
        };
        assert!(capture.validate().is_err());

        let old = serde_json::to_value(request(8, 8)).unwrap();
        assert!(old.get("viewer").is_none());
        let decoded: SnapshotRequest = serde_json::from_value(old).unwrap();
        assert!(decoded.viewer.is_none());
    }

    #[test]
    fn caption_visibility_defaults_to_shown_and_round_trips_per_request() {
        let mut value = serde_json::to_value(request(8, 8)).unwrap();
        value.as_object_mut().unwrap().remove("captionsHidden");
        let mut capture: SnapshotRequest = serde_json::from_value(value).unwrap();
        assert!(!capture.captions_hidden);
        capture.captions_hidden = true;
        let bytes = rmp_serde::to_vec_named(&capture).unwrap();
        let read: SnapshotRequest = rmp_serde::from_slice(&bytes).unwrap();
        assert!(read.captions_hidden);
        assert!(read.validate().is_ok());
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
            video: None,
            origin: [0.0, 0.0],
            layer_box: None,
            raster: None,
            quality: quality(),
            waited_us: 0,
            render_us: 0,
        };
        assert!(
            result(
                2,
                2,
                PixelPayload::Inline {
                    bytes: Bytes(vec![0; 16])
                }
            )
            .validate()
            .is_ok()
        );
        assert!(
            result(
                2,
                2,
                PixelPayload::Inline {
                    bytes: Bytes(vec![0; 15])
                }
            )
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
        for bad in [
            "/pm.0123",
            "/etc/passwd",
            "/pm.0123456789ABCDEF01234567",
            "pm.0123456789abcdef012345678",
        ] {
            assert!(result(1024, 1024, shm(bad)).validate().is_err(), "{bad}");
        }
    }

    #[test]
    fn a_compat_context_frame_carries_the_export_frame_and_only_it() {
        let frame = |source, frame| CompatFrameRequest {
            document: "doc".into(),
            composition: "c".into(),
            layer: "c".into(),
            time: Ticks::ZERO,
            source,
            scale: 1.0,
            revision: Revision::ZERO,
            svg: None,
            frame,
            blender: None,
        };
        let input = |w: u32, h: u32, len: usize| CompatFrameInput {
            width: w,
            height: h,
            payload: PixelPayload::Inline {
                bytes: Bytes(vec![0; len]),
            },
        };
        assert!(
            frame(CompatSource::CompatContext, Some(input(4, 2, 32)))
                .validate()
                .is_ok()
        );
        assert!(frame(CompatSource::CompatContext, None).validate().is_err());
        assert!(
            frame(CompatSource::CompatContext, Some(input(4, 2, 31)))
                .validate()
                .is_err()
        );
        assert!(
            frame(CompatSource::CompatContext, Some(input(0, 2, 0)))
                .validate()
                .is_err()
        );
        assert!(
            frame(CompatSource::RasterOverride, Some(input(4, 2, 32)))
                .validate()
                .is_err()
        );
        assert!(frame(CompatSource::RasterOverride, None).validate().is_ok());
    }

    #[test]
    fn a_public_layer_raster_takes_any_positive_scale_as_today() {
        for scale in [
            1e-9,
            0.005,
            0.01,
            1.0,
            32.0,
            33.0,
            40.0,
            50.0,
            1_000.0,
            f64::MAX,
        ] {
            let mut layer = request(8, 8);
            layer.target = SnapshotTarget::Layer {
                layer: "L".into(),
                scale,
                crop: None,
                content: None,
            };
            assert!(layer.validate().is_ok(), "layer raster at scale {scale}");
        }
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut layer = request(8, 8);
            layer.target = SnapshotTarget::Layer {
                layer: "L".into(),
                scale,
                crop: None,
                content: None,
            };
            assert!(layer.validate().is_err(), "layer raster at scale {scale}");
        }
    }

    #[test]
    fn raster_density_matches_todays_continuous_scale_through_32() {
        let id = CompatKey {
            composition: "c".into(),
            layer: "L".into(),
            time: Ticks::ZERO,
            source: CompatSource::RasterOverride,
            key: "k".into(),
        };
        for scale in [16.01, 24.0, 32.0, 32.01, 0.0, f64::NAN, f64::INFINITY] {
            let good = scale.is_finite() && scale > 0.0 && scale <= 32.0;
            let mut layer = request(8, 8);
            layer.target = SnapshotTarget::Layer {
                layer: "L".into(),
                scale,
                crop: None,
                content: None,
            };
            // A public layer raster takes any positive finite scale.
            assert_eq!(
                layer.validate().is_ok(),
                scale.is_finite() && scale > 0.0,
                "layer {scale}"
            );
            let need = CompatNeed {
                composition: "c".into(),
                layer: "L".into(),
                time: Ticks::ZERO,
                source: CompatSource::RasterOverride,
                scale,
                crop: None,
            };
            let messages = [
                CompatMessage::Needs {
                    revision: Revision::ZERO,
                    needs: vec![need],
                },
                CompatMessage::Native(CompatNative {
                    composition: "c".into(),
                    layer: "L".into(),
                    time: Ticks::ZERO,
                    source: CompatSource::RasterOverride,
                    scale,
                    revision: Revision::ZERO,
                    crop: None,
                }),
                CompatMessage::Reuse {
                    id: id.clone(),
                    scale,
                    revision: Revision::ZERO,
                    crop: None,
                },
                CompatMessage::Pixels(CompatPixels {
                    id: id.clone(),
                    revision: Revision::ZERO,
                    width: 1,
                    height: 1,
                    format: CompatFormat::Rgba8Straight,
                    origin: [0.0, 0.0],
                    scale,
                    row: 0,
                    rows: 1,
                    data: Bytes(vec![0; 4]),
                    size: None,
                    uv: None,
                    crop: None,
                }),
            ];
            for message in messages {
                assert_eq!(message.validate().is_ok(), good, "{message:?}");
            }
            let frame_request = CompatFrameRequest {
                document: "doc".into(),
                composition: "c".into(),
                layer: "L".into(),
                time: Ticks::ZERO,
                source: CompatSource::RasterOverride,
                scale,
                revision: Revision::ZERO,
                svg: None,
                frame: None,
                blender: None,
            };
            assert_eq!(
                frame_request.validate().is_ok(),
                good,
                "export request {scale}"
            );
            for native in [false, true] {
                let frame = CompatFrame {
                    key: "k".into(),
                    width: 1,
                    height: 1,
                    format: CompatFormat::Rgba8Straight,
                    origin: [0.0, 0.0],
                    scale,
                    payload: PixelPayload::Inline {
                        bytes: Bytes(vec![0; if native { 0 } else { 4 }]),
                    },
                    native,
                    size: None,
                    uv: None,
                };
                assert_eq!(
                    frame.validate().is_ok(),
                    good,
                    "export answer {scale} {native}"
                );
            }
        }
    }

    #[test]
    fn compat_crops_are_windows_of_a_raster() {
        use crate::common::PixelRect;
        let need = |crop: Option<PixelRect>| CompatMessage::Needs {
            revision: Revision::ZERO,
            needs: vec![CompatNeed {
                composition: "c".into(),
                layer: "L".into(),
                time: Ticks::ZERO,
                source: CompatSource::RasterOverride,
                scale: 4.0,
                crop,
            }],
        };
        let rect = |x, y, width, height| {
            Some(PixelRect {
                x,
                y,
                width,
                height,
            })
        };
        assert!(need(None).validate().is_ok());
        assert!(need(rect(256, 512, 1024, 768)).validate().is_ok());
        assert!(need(rect(0, 0, 8192, 8192)).validate().is_ok());
        assert!(need(rect(0, 0, 0, 10)).validate().is_err());
        assert!(need(rect(8000, 0, 256, 10)).validate().is_err());
        let native = |crop| {
            CompatMessage::Native(CompatNative {
                composition: "c".into(),
                layer: "L".into(),
                time: Ticks::ZERO,
                source: CompatSource::RasterOverride,
                scale: 1.0,
                revision: Revision::ZERO,
                crop,
            })
        };
        assert!(native(rect(0, 0, 256, 256)).validate().is_ok());
        assert!(native(rect(0, 0, 256, 0)).validate().is_err());
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
                size: None,
                uv: None,
                crop: None,
            })
        };
        assert!(
            band(0, 4, 64, CompatFormat::Rgba8Premultiplied)
                .validate()
                .is_ok()
        );
        assert!(
            band(2, 2, 64, CompatFormat::Rgba16fPremultiplied)
                .validate()
                .is_ok()
        );
        assert!(
            band(3, 2, 32, CompatFormat::Rgba8Premultiplied)
                .validate()
                .is_err()
        );
        assert!(
            band(0, 1, 15, CompatFormat::Rgba8Straight)
                .validate()
                .is_err()
        );
        assert!(band(0, 1, 0, CompatFormat::Png).validate().is_err());
    }

    #[test]
    fn blender_png_is_an_encoded_answer_for_one_content_hash() {
        let mut request = CompatFrameRequest {
            document: "doc".into(),
            composition: "c".into(),
            layer: "L".into(),
            time: Ticks::ZERO,
            source: CompatSource::BlenderPng,
            scale: 1.0,
            revision: Revision::ZERO,
            svg: None,
            frame: None,
            blender: Some(BlenderPngRequest {
                asset: "a1".into(),
                content: crate::common::ContentHash([7; 32]),
            }),
        };
        assert!(request.validate().is_ok());
        request.blender = None;
        assert!(request.validate().is_err());
        let mut png = vec![0; 33];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[12..16].copy_from_slice(b"IHDR");
        png[16..20].copy_from_slice(&1u32.to_be_bytes());
        png[20..24].copy_from_slice(&1u32.to_be_bytes());
        let frame = CompatFrame {
            key: "a".repeat(64),
            width: 1,
            height: 1,
            format: CompatFormat::Png,
            origin: [0.0, 0.0],
            scale: 1.0,
            payload: PixelPayload::Inline { bytes: Bytes(png) },
            native: false,
            size: None,
            uv: None,
        };
        assert!(frame.validate().is_ok());
        let mut bad = frame.clone();
        bad.width = 2;
        assert!(bad.validate().is_err());
    }
}
