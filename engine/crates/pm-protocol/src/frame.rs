//! `FrameRequest` and `FrameDescriptor` (contract 2, sections 8, 14, 18).
//!
//! A frame request says what to render (composition, time, region, scale,
//! color, precision, quality inputs, view override). A descriptor is the one
//! small message per presented frame: `{poolIndex, seq, pts, revision,
//! colorSpace, overlayGeometry}` plus what is on screen. The viewer releases
//! each frame by `{poolIndex, seq}`.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::color::{OutputColor, Precision, ViewerColorSpace};
use crate::common::{
    CompositionId, HostMicros, LayerId, Mat3, Mat4, PixelRect, Rect, Revision, Seq, Size, Ticks,
};
use crate::interop::SurfaceFormat;
use crate::priority::PriorityClass;
use crate::validate::{Validate, Violation, ensure};
use crate::version::ContractId;

/// The viewer's Resolution control (appendix D).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum PreviewResolution {
    Automatic,
    Full,
    Half,
    Quarter,
}

/// The viewer's Media control (appendix D).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum MediaPreference {
    Automatic,
    /// Originals only (and Always Play Original clips).
    Originals,
    /// Proxies only, like Final Cut Pro's Proxy Only.
    Proxies,
}

/// Which motion-blur sample count applies (composition `motionBlurSamples`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum MotionBlurKind {
    Export,
    AlphaExport,
    PreviewPlaying,
    PreviewPaused,
    /// Motion blur off (agent frames by default).
    Off,
}

/// What the frame is for. Decides which inputs may apply: view overrides and
/// reduced quality never reach exports or agent captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum FramePurpose {
    /// Viewer presentation through the surface pool.
    Viewer,
    /// Scrub or gesture frame: bypasses the jitter buffer.
    Scrub,
    /// Thumbnails, posters, library cards (async engine snapshots).
    Thumbnail,
    /// Agent capture (contract 12 batches these).
    AgentCapture,
    /// Export and the synchronous compatibility path for `render.renderFrameTo()`.
    Export,
}

impl FramePurpose {
    /// Exports and agent captures ignore view overrides and draft quality.
    pub const fn is_final(self) -> bool {
        matches!(self, FramePurpose::Export | FramePurpose::AgentCapture)
    }
}

/// 3D shading for the editor view (the 3D viewport's shading modes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum Shading3d {
    Wireframe,
    Solid,
    Material,
    Rendered,
}

/// Camera projection.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum Projection {
    /// Vertical field of view in degrees.
    Perspective {
        fov_y_degrees: f64,
        near: f64,
        far: f64,
    },
    /// Orthographic with a fixed half-height (3 divided by zoom, section 8).
    Orthographic {
        half_height: f64,
        near: f64,
        far: f64,
    },
}

/// A camera that is not in the document (the editor camera).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EditorCamera {
    /// World-to-camera matrix.
    pub view: Mat4,
    pub projection: Projection,
}

/// Preview-only overrides (glossary: view override). Never applied to
/// exports or agent captures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ViewOverride {
    pub camera: Option<EditorCamera>,
    pub shading: Shading3d,
    /// Multiplies opacity by 0.45 and writes no depth, in draft shading modes only.
    pub xray: bool,
    /// Layers shown in Local View; empty when Local View is off.
    pub local_view: Vec<LayerId>,
}

/// The X-ray opacity multiplier (section 8).
pub const XRAY_OPACITY: f64 = 0.45;

/// Ask for one frame. The envelope's revision is the minimum document
/// revision to render (read-your-writes).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FrameRequest {
    pub composition: CompositionId,
    pub time: Ticks,
    pub purpose: FramePurpose,
    pub priority: PriorityClass,
    /// Output size in device pixels (viewer) or output pixels (export).
    pub output_size: Size,
    /// Region of interest inside `output_size`; `None` renders all of it.
    /// Region of interest applies to the final output only (section 8).
    pub region: Option<PixelRect>,
    /// Output pixels per composition pixel, 0.01 to 16.
    pub scale: f64,
    pub resolution: PreviewResolution,
    pub media: MediaPreference,
    pub color: OutputColor,
    pub precision: Precision,
    pub motion_blur: MotionBlurKind,
    pub view: Option<ViewOverride>,
    /// Native shading stands in for Blender-rendered compositions: preview,
    /// thumbnails, and agent captures reported as `blender: "pending"`.
    /// Never for exports (a missing Blender surface fails an export).
    pub draft_3d: bool,
    /// The user chose "Render using: Proxies (draft)" for this export
    /// (section 14.6). The only way proxy pixels reach an export.
    pub draft_render: bool,
    /// Also render the depth and object-id surface (frames with 3D layers).
    pub depth_id: bool,
    /// Also return overlay geometry at the frame's revision.
    pub overlay: bool,
}

/// Smallest and largest export scale (section 17).
pub const MIN_SCALE: f64 = 0.01;
pub const MAX_SCALE: f64 = 16.0;
/// Largest output edge.
pub const MAX_OUTPUT_DIMENSION: u32 = 16384;

impl Validate for FrameRequest {
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
                    "output size {:?} outside 1..{MAX_OUTPUT_DIMENSION}",
                    self.output_size
                )
            },
        )?;
        if let Some(region) = &self.region {
            ensure(
                !region.is_empty() && self.output_size.contains(region),
                C,
                || {
                    format!(
                        "region {region:?} is empty or outside {:?}",
                        self.output_size
                    )
                },
            )?;
        }
        ensure(
            self.scale.is_finite() && (MIN_SCALE..=MAX_SCALE).contains(&self.scale),
            C,
            || format!("scale {} outside {MIN_SCALE}..{MAX_SCALE}", self.scale),
        )?;
        if self.purpose.is_final() {
            ensure(self.view.is_none(), C, || {
                "exports and agent captures ignore view overrides; send none".into()
            })?;
            ensure(self.resolution == PreviewResolution::Full, C, || {
                "exports and agent captures render at full resolution".into()
            })?;
            ensure(
                !matches!(
                    self.motion_blur,
                    MotionBlurKind::PreviewPlaying | MotionBlurKind::PreviewPaused
                ),
                C,
                || "exports and agent captures never use preview motion-blur counts".into(),
            )?;
            ensure(!self.overlay && !self.depth_id, C, || {
                "overlay geometry and depth surfaces are viewer outputs".into()
            })?;
        }
        ensure(
            !self.draft_render || self.purpose == FramePurpose::Export,
            C,
            || "only exports take an explicit draft render".into(),
        )?;
        ensure(
            self.media != MediaPreference::Proxies || !self.purpose.is_final() || self.draft_render,
            C,
            || {
                "proxy pixels reach an export only through an explicit draft render, and never an agent capture".into()
            },
        )?;
        ensure(
            !self.draft_render || self.media == MediaPreference::Proxies,
            C,
            || "a draft render reads proxies".into(),
        )?;
        if self.purpose == FramePurpose::Export {
            ensure(!self.draft_3d, C, || {
                "exports never use draft 3D; a missing Blender surface fails the export".into()
            })?;
            ensure(
                !matches!(
                    self.priority,
                    PriorityClass::Interactive | PriorityClass::AgentSnapshot
                ),
                C,
                || "exports run in a job class".into(),
            )?;
            ensure(
                matches!(
                    self.motion_blur,
                    MotionBlurKind::Export | MotionBlurKind::AlphaExport | MotionBlurKind::Off
                ),
                C,
                || "exports use export motion-blur counts".into(),
            )?;
        }
        if self.purpose == FramePurpose::AgentCapture {
            ensure(self.priority == PriorityClass::AgentSnapshot, C, || {
                "agent captures run in the agent snapshot class".into()
            })?;
        }
        if matches!(self.purpose, FramePurpose::Viewer | FramePurpose::Scrub) {
            ensure(self.priority == PriorityClass::Interactive, C, || {
                "viewer frames are interactive".into()
            })?;
            ensure(matches!(self.color, OutputColor::Viewer { .. }), C, || {
                "viewer frames use the viewer output color".into()
            })?;
        }
        if let Some(view) = &self.view {
            ensure(
                !view.xray || matches!(view.shading, Shading3d::Wireframe | Shading3d::Solid),
                C,
                || "X-ray applies in draft shading modes only".into(),
            )?;
        }
        Ok(())
    }
}

/// Which representation produced a frame's source pixels (section 14.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum Representation {
    Original,
    ReducedResolution,
    Proxy,
    OptimizedMedia,
    KeptRender,
}

/// What is on screen, so the viewer can say so ("Half · Proxy").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FrameQuality {
    /// The resolution actually rendered (Automatic resolves to one of the others).
    pub resolution: PreviewResolution,
    /// The lowest representation any visible source used.
    pub representation: Representation,
    /// Native shading stood in for Blender.
    pub draft_3d: bool,
    /// Served from the engine memory cache (cached preview).
    pub cached: bool,
    /// A scrub frame in which a video layer shows the nearest decoded frame
    /// of its source while its exact frame decodes (`SeekMode::Scrub`, as
    /// today's viewer draws a video element's last decoded picture). The
    /// exact frame follows when it decodes. Never set on timed frames,
    /// exports, or captures. Added after `protocol-a6aa78d4b8a57c0d`
    /// (RFC 02 version notes); a peer that does not send it decodes as
    /// `false`.
    #[serde(default)]
    pub stand_in: bool,
}

/// A quad in view pixels, clockwise from top-left.
pub type Quad = [[f64; 2]; 4];

/// One path command in view pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "op")]
pub enum PathCommand {
    MoveTo {
        x: f64,
        y: f64,
    },
    LineTo {
        x: f64,
        y: f64,
    },
    CubicTo {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        x: f64,
        y: f64,
    },
    Close,
}

/// Geometry the overlay draws for one layer, at the frame's revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OverlayLayer {
    pub layer: LayerId,
    /// Layer-to-view transform (2D layers).
    pub world_2d: Option<Mat3>,
    /// Layer-to-world transform (3D layers; the chrome projects with the view camera).
    pub world_3d: Option<Mat4>,
    pub bounds: Quad,
    /// Mask and shape paths when the layer is selected.
    pub paths: Vec<Vec<PathCommand>>,
}

/// The text caret at the frame's revision.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Caret {
    pub top: [f64; 2],
    pub bottom: [f64; 2],
}

/// Overlay geometry that travels with a frame, so handles never lead pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OverlayGeometry {
    /// Composition rectangle in view pixels.
    pub viewport: Rect,
    pub layers: Vec<OverlayLayer>,
    pub caret: Option<Caret>,
}

/// The depth and object-id surface (`rgbaf16`: r depth, g and b the object
/// id minus one, a coverage) of a viewer frame, in the depth pool
/// (`SurfacePool::role` `depthId`) of `pool_generation`, at the frame's own
/// size and placement. It holds this frame's surface while the frame is on
/// screen: the engine reuses a depth slot only after `pool size − 2` newer
/// frames were rendered, more than a viewer holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DepthIdSurface {
    pub pool_index: u32,
    pub seq: Seq,
    pub pool_generation: Seq,
}

/// One presented frame. Sent after the command buffer completes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FrameDescriptor {
    pub pool_index: u32,
    /// The surface pool generation this index belongs to.
    pub pool_generation: Seq,
    /// Increases by one per frame produced on this session.
    pub seq: Seq,
    /// Presentation time: the composition time the frame shows.
    pub pts: Ticks,
    /// The exact revision rendered (≥ the request's minimum).
    pub revision: Revision,
    pub color_space: ViewerColorSpace,
    pub format: SurfaceFormat,
    pub quality: FrameQuality,
    pub overlay: Option<OverlayGeometry>,
    pub depth_id: Option<DepthIdSurface>,
    /// The viewer state the frame shows ([`ViewerState::view_seq`]): with
    /// `revision`, it tags the frame so overlays drawn from its geometry match
    /// its pixels during view gestures (zoom, pan, orbit, walk).
    pub view_seq: Seq,
    /// `Some`: a timed frame, shown from `pts` for this long while the clock
    /// runs (playback); the viewer's jitter buffer presents it by predicted
    /// display time. `None`: shown at the next refresh (paused frames, scrub,
    /// gestures).
    pub duration: Option<Ticks>,
}

impl Validate for FrameDescriptor {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Frame;
        ensure(self.duration.is_none_or(|d| d.get() > 0), C, || {
            "a timed frame lasts a positive time".into()
        })?;
        ensure(
            matches!(
                self.format,
                SurfaceFormat::Bgra8Unorm | SurfaceFormat::Rgba16Float
            ),
            C,
            || "viewer frames are bgra8 or rgba16float".into(),
        )?;
        if let Some(overlay) = &self.overlay {
            let finite = |v: f64| v.is_finite();
            ensure(
                [
                    overlay.viewport.x,
                    overlay.viewport.y,
                    overlay.viewport.width,
                    overlay.viewport.height,
                ]
                .into_iter()
                .all(finite)
                    && overlay
                        .layers
                        .iter()
                        .all(|l| l.bounds.iter().flatten().copied().all(finite)),
                C,
                || "overlay geometry must be finite".into(),
            )?;
        }
        Ok(())
    }
}

/// The viewer is done with a frame: it called `VideoFrame.close()` after
/// submitting the GPU work that read it, and that work completed
/// (`queue.onSubmittedWorkDone()`). The per-use release callback is not a
/// completion signal (S1: it runs before the GPU finished reading).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FrameRelease {
    pub pool_index: u32,
    pub seq: Seq,
}

/// The viewer's state, sent whenever it changes (section 18, view override).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ViewerState {
    pub composition: CompositionId,
    /// Canvas size in device pixels.
    pub size: Size,
    pub device_pixel_ratio: f64,
    pub zoom: f64,
    pub pan: [f64; 2],
    pub resolution: PreviewResolution,
    pub media: MediaPreference,
    pub color_space: ViewerColorSpace,
    pub view: Option<ViewOverride>,
    /// Play Every Frame (viewer menu check item).
    pub play_every_frame: bool,
    /// The canvas's display can show values above 1.0 (CSS
    /// `dynamic-range: high`): the engine then presents through an `rgbaf16`
    /// pool (amendment 4).
    pub edr: bool,
    /// Increases whenever the viewer state changes; descriptors echo the
    /// state they show.
    pub view_seq: Seq,
    /// The refresh grid of the display showing the viewer (amendment 18),
    /// so the engine can start timed frames' GPU work just after the
    /// compositor's. Absent until the viewer has fitted its animation
    /// frames. Added after `protocol-16c8e6e125019103` as an optional field
    /// (RFC 02 version notes): a peer that does not send it is decoded as
    /// `None`, and a change of the hint alone is not a new view (it keeps
    /// `view_seq`).
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    pub vsync: Option<VsyncHint>,
    /// The 3D viewport chrome wants each frame's depth and object-id
    /// surface (plan section 8; the Phase 3c render transport). Absent or
    /// false: no depth pool. A change of it is a new view.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    pub depth_id: Option<bool>,
}

/// A display's refresh grid on the host clock: refreshes (BeginFrame times)
/// fall at `host_time + n × period_us`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct VsyncHint {
    /// One refresh instant (host µs).
    pub host_time: HostMicros,
    /// The refresh period in µs (8,333.3 at 120 Hz).
    pub period_us: f64,
}

impl VsyncHint {
    /// The shortest and longest refresh periods accepted (500 Hz to 10 Hz).
    pub const PERIOD_RANGE_US: (f64, f64) = (2_000.0, 100_000.0);

    /// The first refresh at or after host µs `t`, moved by `offset_us`.
    pub fn next_at_or_after(&self, t: f64, offset_us: f64) -> f64 {
        let origin = self.host_time.get() as f64 + offset_us;
        let n = ((t - origin) / self.period_us - 1e-9).ceil();
        origin + n * self.period_us
    }
}

impl Validate for ViewerState {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Frame;
        ensure(self.composition.is_valid(), C, || {
            "invalid composition id".into()
        })?;
        ensure(
            !self.size.is_empty()
                && self.size.width <= MAX_OUTPUT_DIMENSION
                && self.size.height <= MAX_OUTPUT_DIMENSION,
            C,
            || {
                format!(
                    "viewer size {:?} outside 1..{MAX_OUTPUT_DIMENSION}",
                    self.size
                )
            },
        )?;
        ensure(
            self.device_pixel_ratio.is_finite() && self.device_pixel_ratio > 0.0,
            C,
            || "device pixel ratio must be positive".into(),
        )?;
        ensure(self.zoom.is_finite() && self.zoom > 0.0, C, || {
            "zoom must be positive".into()
        })?;
        ensure(self.pan.iter().all(|v| v.is_finite()), C, || {
            "pan must be finite".into()
        })?;
        if let Some(vsync) = &self.vsync {
            let (min, max) = VsyncHint::PERIOD_RANGE_US;
            ensure(
                vsync.period_us.is_finite() && vsync.period_us >= min && vsync.period_us <= max,
                C,
                || format!("vsync period {} µs outside {min}..{max}", vsync.period_us),
            )?;
        }
        Ok(())
    }
}

/// Engine-side bookkeeping of frames the viewer holds. A surface is never
/// written while held; a release must name the frame's exact `seq`.
#[derive(Debug, Clone)]
pub struct PoolLedger {
    held: Vec<Option<Seq>>,
}

/// A release that does not match a held frame.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReleaseError {
    #[error("pool index {0} is out of range")]
    OutOfRange(u32),
    #[error("pool index {index} is not held")]
    NotHeld { index: u32 },
    #[error("pool index {index} holds seq {held}, release named {released}")]
    WrongSeq {
        index: u32,
        held: Seq,
        released: Seq,
    },
}

impl PoolLedger {
    pub fn new(surfaces: usize) -> Self {
        Self {
            held: vec![None; surfaces],
        }
    }

    /// A free surface to render into, if any.
    pub fn free(&self) -> Option<u32> {
        self.held.iter().position(Option::is_none).map(|i| i as u32)
    }

    /// Marks a surface as sent to the viewer with `seq`. Returns false if it
    /// is already held (a bug: the engine would overwrite a presented frame).
    pub fn present(&mut self, index: u32, seq: Seq) -> bool {
        match self.held.get_mut(index as usize) {
            Some(slot @ None) => {
                *slot = Some(seq);
                true
            }
            _ => false,
        }
    }

    pub fn release(&mut self, release: FrameRelease) -> Result<(), ReleaseError> {
        let slot = self
            .held
            .get_mut(release.pool_index as usize)
            .ok_or(ReleaseError::OutOfRange(release.pool_index))?;
        match *slot {
            None => Err(ReleaseError::NotHeld {
                index: release.pool_index,
            }),
            Some(held) if held != release.seq => Err(ReleaseError::WrongSeq {
                index: release.pool_index,
                held,
                released: release.seq,
            }),
            Some(_) => {
                *slot = None;
                Ok(())
            }
        }
    }

    pub fn held_count(&self) -> usize {
        self.held.iter().filter(|s| s.is_some()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn viewer_request() -> FrameRequest {
        FrameRequest {
            composition: "comp".into(),
            time: Ticks::from_whole_seconds(2),
            purpose: FramePurpose::Viewer,
            priority: PriorityClass::Interactive,
            output_size: Size::new(3840, 2160),
            region: None,
            scale: 1.0,
            resolution: PreviewResolution::Automatic,
            media: MediaPreference::Automatic,
            color: OutputColor::Viewer {
                space: ViewerColorSpace::DisplayP3,
            },
            precision: Precision::Float16,
            motion_blur: MotionBlurKind::PreviewPlaying,
            view: None,
            draft_3d: true,
            draft_render: false,
            depth_id: true,
            overlay: true,
        }
    }

    fn viewer_state() -> ViewerState {
        ViewerState {
            composition: "main".into(),
            size: Size::new(2400, 1300),
            device_pixel_ratio: 2.0,
            zoom: 0.5,
            pan: [0.0, 0.0],
            resolution: PreviewResolution::Automatic,
            media: MediaPreference::Automatic,
            color_space: ViewerColorSpace::DisplayP3,
            view: None,
            play_every_frame: false,
            edr: true,
            view_seq: Seq::new(3).unwrap(),
            vsync: Some(VsyncHint {
                host_time: HostMicros::new(1_000_000).unwrap(),
                period_us: 8_333.333,
            }),
            depth_id: Some(true),
        }
    }

    #[test]
    fn the_vsync_hint_is_optional_on_the_wire_and_validated() {
        // A viewer built before the hint existed sends no `vsync` key.
        let mut map = serde_json::to_value(viewer_state()).unwrap();
        map.as_object_mut().unwrap().remove("vsync");
        let bytes = rmp_serde::to_vec_named(&map).unwrap();
        let decoded: ViewerState = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(decoded.vsync, None);
        assert!(decoded.validate().is_ok());
        // With it, it round-trips and its period is checked.
        let state = viewer_state();
        let bytes = rmp_serde::to_vec_named(&state).unwrap();
        assert_eq!(rmp_serde::from_slice::<ViewerState>(&bytes).unwrap(), state);
        assert!(state.validate().is_ok());
        let mut bad = viewer_state();
        bad.vsync = Some(VsyncHint {
            host_time: HostMicros::new(1).unwrap(),
            period_us: 0.0,
        });
        assert!(bad.validate().is_err());
    }

    #[test]
    fn vsync_hint_finds_the_next_refresh_with_an_offset() {
        let hint = VsyncHint {
            host_time: HostMicros::new(1_000_000).unwrap(),
            period_us: 8_000.0,
        };
        assert_eq!(hint.next_at_or_after(1_000_000.0, 0.0), 1_000_000.0);
        assert_eq!(hint.next_at_or_after(1_000_001.0, 0.0), 1_008_000.0);
        assert_eq!(hint.next_at_or_after(1_000_001.0, 2_000.0), 1_002_000.0);
        // Before the anchor, the grid extends backwards.
        assert_eq!(hint.next_at_or_after(990_000.0, 2_000.0), 994_000.0);
    }

    #[test]
    fn viewer_requests_validate() {
        assert!(viewer_request().validate().is_ok());
        let mut bad = viewer_request();
        bad.region = Some(PixelRect {
            x: 3800,
            y: 0,
            width: 100,
            height: 10,
        });
        assert!(bad.validate().is_err());
        let mut bad = viewer_request();
        bad.scale = 0.0;
        assert!(bad.validate().is_err());
        let mut bad = viewer_request();
        bad.priority = PriorityClass::UserJob;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn exports_never_take_preview_inputs() {
        let export = FrameRequest {
            purpose: FramePurpose::Export,
            priority: PriorityClass::UserJob,
            resolution: PreviewResolution::Full,
            media: MediaPreference::Originals,
            color: OutputColor::Rec709Broadcast,
            motion_blur: MotionBlurKind::Export,
            draft_3d: false,
            depth_id: false,
            overlay: false,
            ..viewer_request()
        };
        assert!(export.validate().is_ok());
        let with_view = FrameRequest {
            view: Some(ViewOverride {
                camera: None,
                shading: Shading3d::Solid,
                xray: false,
                local_view: vec![],
            }),
            ..export.clone()
        };
        assert!(with_view.validate().is_err());
        assert!(
            FrameRequest {
                media: MediaPreference::Proxies,
                ..export.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            FrameRequest {
                media: MediaPreference::Proxies,
                draft_render: true,
                ..export.clone()
            }
            .validate()
            .is_ok(),
            "explicit draft render"
        );
        assert!(
            FrameRequest {
                draft_render: true,
                ..export.clone()
            }
            .validate()
            .is_err(),
            "a draft render reads proxies"
        );
        assert!(
            FrameRequest {
                draft_3d: true,
                ..export.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            FrameRequest {
                resolution: PreviewResolution::Half,
                ..export.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            FrameRequest {
                overlay: true,
                ..export.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            FrameRequest {
                motion_blur: MotionBlurKind::PreviewPaused,
                ..export.clone()
            }
            .validate()
            .is_err()
        );
        let agent = FrameRequest {
            purpose: FramePurpose::AgentCapture,
            priority: PriorityClass::AgentSnapshot,
            color: OutputColor::SrgbImage,
            motion_blur: MotionBlurKind::Off,
            draft_3d: true,
            ..export
        };
        assert!(
            agent.validate().is_ok(),
            "agent captures may stand in native shading (blender: pending)"
        );
        assert!(
            FrameRequest {
                media: MediaPreference::Proxies,
                ..agent.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            FrameRequest {
                media: MediaPreference::Proxies,
                draft_render: true,
                ..agent.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            FrameRequest {
                priority: PriorityClass::UserJob,
                ..agent
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn xray_only_in_draft_shading() {
        let mut request = viewer_request();
        request.view = Some(ViewOverride {
            camera: None,
            shading: Shading3d::Rendered,
            xray: true,
            local_view: vec![],
        });
        assert!(request.validate().is_err());
        request.view = Some(ViewOverride {
            camera: None,
            shading: Shading3d::Solid,
            xray: true,
            local_view: vec![],
        });
        assert!(request.validate().is_ok());
    }

    #[test]
    fn pool_ledger_never_overwrites_a_held_frame() {
        let seq = |n| Seq::new(n).unwrap();
        let mut pool = PoolLedger::new(2);
        assert!(pool.present(0, seq(1)));
        assert!(!pool.present(0, seq(2)));
        assert_eq!(pool.free(), Some(1));
        assert!(pool.present(1, seq(2)));
        assert_eq!(pool.free(), None);
        assert_eq!(
            pool.release(FrameRelease {
                pool_index: 0,
                seq: seq(9)
            }),
            Err(ReleaseError::WrongSeq {
                index: 0,
                held: seq(1),
                released: seq(9)
            })
        );
        assert_eq!(
            pool.release(FrameRelease {
                pool_index: 5,
                seq: seq(1)
            }),
            Err(ReleaseError::OutOfRange(5))
        );
        pool.release(FrameRelease {
            pool_index: 0,
            seq: seq(1),
        })
        .unwrap();
        assert_eq!(
            pool.release(FrameRelease {
                pool_index: 0,
                seq: seq(1)
            }),
            Err(ReleaseError::NotHeld { index: 0 })
        );
        assert_eq!(pool.held_count(), 1);
    }
}
