//! Adapted from engine-host: bounded cached/leased surfaces, nonblocking GPU
//! completion and exact Metal-device injection. The final consumer is the viewer.
use crate::policy::{Moment, Output};
use crate::trace::{Trace, now};
use anyhow::{Result, anyhow, bail};
use foreign_types::ForeignTypeRef;
use pm_gpu::{Gpu, Target, bridge_out::surface::ExportedSurface};
use pm_protocol::snapshot::CompatNeed;
use pm_protocol::{
    color::Precision,
    common::{Size, Ticks},
    interop::{SurfaceFormat, SurfaceRequest},
};
use pm_render::{LayerFailure, RenderError, Renderer};
use std::{
    collections::VecDeque,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

/// One immutable render input. Content identity rotates on document, comp,
/// assets, definitions or fonts; it is independent of display output identity.
#[derive(Clone)]
pub struct Scene {
    pub doc: Arc<pm_model::Document>,
    pub comp: String,
    pub media: Arc<pm_export::render::MediaPaths>,
    pub content: u64,
}
/// The viewer's rendered-frame cache (RGBA16F surfaces).
const FRAME_CACHE_BYTES: u64 = 128 * 1024 * 1024;
/// The most a paused composition-size surface may take: half the frame
/// cache, so it always holds two.
pub const PAUSED_FULL_BYTES: u64 = FRAME_CACHE_BYTES / 2;
/// One viewer frame's request. The renderer's playback path, never an export.
#[derive(Clone, Copy, Debug)]
pub struct FrameSpec {
    pub time: Ticks,
    pub output: Output,
    pub moment: Moment,
    /// Render resolution as a share of `output`; the final pass scales once.
    pub scale: f64,
    /// Playing frames: video waits for exact frames until here, then stands in.
    pub decode_deadline: Option<Instant>,
    /// The exact picture is read (a capture): the final resampler, whatever
    /// the moment and scale.
    pub exact: bool,
    /// The settled picture of a paused frame whose drawable is the
    /// composition's own size: the final picture, drawn once the viewer
    /// has rested, after the preview of the same time and size
    /// ([`Self::final_picture`]).
    pub settled: bool,
}
impl FrameSpec {
    pub fn exact(time: Ticks, output: Output) -> Self {
        Self {
            time,
            output,
            moment: Moment::Paused,
            scale: 1.,
            decode_deadline: None,
            exact: true,
            settled: false,
        }
    }
    /// A paused frame at Full shows the composition as an export draws it:
    /// rendered at the composition's size (After Effects' Full) and scaled
    /// down to the drawable once, by the final pass's area filter, so
    /// pixel-sized effects (an ASCII grid, grain, dither) and text weight
    /// look as they do in a delivered file at any zoom. Zoomed-in views (a
    /// crop) and frames of other moments keep their scale. Bounded at 4×
    /// the drawable.
    pub fn paused_full_scale(scene: &Scene, output: &Output) -> f64 {
        // A crop that is the whole canvas is no crop: at Fit the shell's
        // window can come out a pixel narrower than its canvas by rounding
        // (a 1265 × 712 drawable on a 1266 × 712 canvas).
        let zoomed = output.crop.is_some_and(|c| {
            c.x != 0
                || c.y != 0
                || c.canvas_width.abs_diff(output.width) > 1
                || c.canvas_height.abs_diff(output.height) > 1
        });
        if zoomed || output.width == 0 || output.height == 0 {
            return 1.;
        }
        let Ok(comp) = pm_eval::Comp::resolve(&scene.doc, &scene.comp) else {
            return 1.;
        };
        let side = |key: &str| {
            let v = pm_eval::node::to_number(comp.member(key));
            if v.is_finite() && v > 0. { v } else { 0. }
        };
        let ratio = (side("w") / f64::from(output.width)).max(side("h") / f64::from(output.height));
        let scale = ratio.clamp(1., 4.);
        // Admitted whole within [`PAUSED_FULL_BYTES`] (RGBA16F), or not at
        // all: a share of the way gives neither the delivered pixel grid nor
        // the drawable's (an 8K composition in a 4K drawable draws at the
        // drawable's size). A side one pixel longer bounds the rounding.
        let side = |v: u32| (f64::from(v) * scale).round() + 1.;
        if side(output.width) * side(output.height) * 8. <= PAUSED_FULL_BYTES as f64 {
            scale
        } else {
            1.
        }
    }
    /// Whether this frame is the final picture of its time: a capture, or
    /// the picture a paused viewer settles on, the whole composition at its
    /// own size (the refinement above the drawable's size, or the settled
    /// frame of a drawable of exactly that size). It is drawn from the
    /// sources and with the effect passes an export draws with, so it is
    /// the picture a rendered frame on disk holds: a paused picture is the
    /// same before and after its range is rendered. Every other viewer
    /// frame is a preview (the picture a paused viewer shows at once, kept
    /// until the settled one is ready), and a paused one is never replaced
    /// by the frame on disk.
    pub(crate) fn final_picture(&self, scene: &Scene) -> bool {
        self.exact
            || (self.moment == Moment::Paused
                && (self.whole_composition(scene).is_some()
                    || (self.settled && self.scale == 1. && self.fills_composition(scene))))
    }
    /// Whether a paused frame of this output is followed by a settled one
    /// ([`Self::final_picture`]): its refinement at the composition's size,
    /// or the final picture on a drawable of that size.
    pub(crate) fn settles(&self, scene: &Scene) -> bool {
        Self::paused_full_scale(scene, &self.output) > 1. || self.fills_composition(scene)
    }
    /// Whether the drawable is the whole composition at its own size.
    pub(crate) fn fills_composition(&self, scene: &Scene) -> bool {
        self.output.crop.is_none()
            && composition_size(scene) == Some(Size::new(self.output.width, self.output.height))
    }
    /// Whether a paused frame of this output settles on the final picture
    /// ([`Self::final_picture`]), at once or by its refinement: where the
    /// frame on disk may be shown for it. A reduced quality never does.
    pub(crate) fn settles_final(&self, scene: &Scene) -> bool {
        let refined = FrameSpec {
            scale: Self::paused_full_scale(scene, &self.output),
            ..*self
        };
        self.scale >= 1.
            && (refined.whole_composition(scene).is_some() || self.fills_composition(scene))
    }
    /// Whether stills are read at the view's resolution (ImageIO's
    /// filtered thumbnail of the original): every preview frame. A final
    /// picture reads the originals, which the engine lets go of once it is
    /// drawn (held, they cost gigabytes on large images).
    pub(crate) fn preview_stills(&self, scene: &Scene) -> bool {
        !self.final_picture(scene)
    }
    /// Whether video frames are converted at the view's resolution: only
    /// for frames a final picture replaces, playing and dragged ones. A
    /// paused frame converts video as the export does, so paused Full shows
    /// the export's pixels.
    pub(crate) fn preview_video(&self) -> bool {
        !self.exact && self.moment != Moment::Paused
    }
    pub(crate) fn render_size(&self) -> Size {
        let px = |v: u32| ((f64::from(v) * self.scale).round() as u32).max(1);
        Size::new(px(self.output.width), px(self.output.height))
    }
    /// The composition's own size, when this frame is the whole composition
    /// at its own size: a frame at the paused Full scale above 1
    /// ([`Self::paused_full_scale`]: paused or captured, not zoomed, within
    /// its bounds). It is drawn on the composition's pixel grid exactly, as
    /// an export and a rendered frame on disk are, and presented as they
    /// are (the drawable's window of the canvas, one footprint filter):
    /// `output × scale` rounds to another size whenever the drawable's
    /// aspect is a pixel off the composition's (1081 rows for 1080).
    pub(crate) fn whole_composition(&self, scene: &Scene) -> Option<Size> {
        if self.scale <= 1. || self.scale != Self::paused_full_scale(scene, &self.output) {
            return None;
        }
        composition_size(scene)
    }
    /// The size the frame is rendered at.
    pub(crate) fn size_in(&self, scene: &Scene) -> Size {
        self.whole_composition(scene)
            .unwrap_or_else(|| self.render_size())
    }
    pub(crate) fn request(&self, scene: &Scene) -> Result<pm_render::RenderRequest> {
        use pm_protocol::frame::MotionBlurKind;
        let playing = self.moment == Moment::Playing;
        let whole = self.whole_composition(scene);
        let mut request = pm_render::RenderRequest::sized(
            whole.unwrap_or_else(|| self.render_size()),
            Precision::Float16,
        );
        if let Some(crop) = self.output.crop.filter(|_| whole.is_none()) {
            let comp = pm_eval::Comp::resolve(&scene.doc, &scene.comp)
                .map_err(|_| RenderError::Composition(scene.comp.clone()))?;
            let side = |key: &str| {
                let v = pm_eval::node::to_number(comp.member(key));
                if v.is_finite() && v > 0. { v } else { 1. }
            };
            request.scale = Some((
                f64::from(crop.canvas_width) * self.scale / side("w"),
                f64::from(crop.canvas_height) * self.scale / side("h"),
            ));
            request.origin = Some((
                f64::from(crop.x) * self.scale,
                f64::from(crop.y) * self.scale,
            ));
        }
        // Playing and dragged frames are previews: 6 motion samples playing,
        // 12 dragged. A paused frame and a capture are the picture a
        // delivery draws, and the one a rendered frame on disk holds: the
        // export's count (16, or the composition's own), so a paused
        // picture is the same drawn or read from disk.
        request.motion_blur = pm_render::motion::MotionBlur::Kind(if playing && !self.exact {
            MotionBlurKind::PreviewPlaying
        } else if self.moment == Moment::Scrub && !self.exact {
            MotionBlurKind::PreviewPaused
        } else {
            MotionBlurKind::Export
        });
        request.sampling = self.sampling();
        // A capture and a settled paused frame are the final picture:
        // final output, as an export (effects at the composition's density
        // and over the whole frame, `pm_render::plan`).
        if self.final_picture(scene) {
            request.output_kind = pm_render::OutputKind::Final;
        }
        #[cfg(test)]
        SAMPLED
            .lock()
            .unwrap()
            .push((self.exact, self.moment, self.scale, request.sampling));
        #[cfg(test)]
        MEDIA_SAMPLED
            .lock()
            .unwrap()
            .push((self.exact, self.moment, scene.media.clone()));
        Ok(request)
    }

    /// A drag's reduced frames (a preview the final resampler redraws once
    /// the playhead settles) take the playback resampler; every frame a
    /// person watches, playing or paused, and exact reads such as captures
    /// take the final one (Lanczos-3), so footage and precomps don't sharpen
    /// when playback stops. Its cost on a real project's playing frames was
    /// within run-to-run noise.
    pub(crate) fn sampling(&self) -> pm_render::sampling::Sampling {
        if !self.exact && self.moment == Moment::Scrub && self.scale < 1. {
            pm_render::sampling::Sampling::Playback
        } else {
            pm_render::sampling::Sampling::Final
        }
    }
}
/// The composition's size in whole pixels.
fn composition_size(scene: &Scene) -> Option<Size> {
    let comp = pm_eval::Comp::resolve(&scene.doc, &scene.comp).ok()?;
    let side = |key: &str| {
        let v = pm_eval::node::to_number(comp.member(key)).round();
        (v.is_finite() && v >= 1. && v <= f64::from(u32::MAX)).then_some(v as u32)
    };
    Some(Size::new(side("w")?, side("h")?))
}
/// A frame that could not be drawn yet: the picture stays and it is drawn again.
#[derive(Debug)]
struct FontsPending(String);
impl std::fmt::Display for FontsPending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a frame waited for fonts ({})", self.0)
    }
}
impl std::error::Error for FontsPending {}
/// Every compiler slot was taken while definitions were installing. Nothing is
/// wrong with the shader; a sticky error here would stick to a healthy project.
#[derive(Debug)]
pub(crate) struct CompilerBusy;
impl std::fmt::Display for CompilerBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("every shader compiler slot is busy")
    }
}
impl std::error::Error for CompilerBusy {}
/// A playing frame is on disk and was still being read when its turn came.
/// It gives way for a tick (the clock goes on and the reader catches up)
/// instead of being drawn again: it is on disk because drawing it takes
/// longer than its interval, and drawing it would leave the reader reading
/// frames the playhead has passed.
#[derive(Debug)]
pub(crate) struct DiskFramePending;
impl std::fmt::Display for DiskFramePending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the rendered frame is still being read")
    }
}
impl std::error::Error for DiskFramePending {}
pub(crate) fn transient(error: &anyhow::Error) -> bool {
    error.is::<FontsPending>()
        || error.is::<CompilerBusy>()
        || error.is::<DiskFramePending>()
        || matches!(
            error.downcast_ref::<RenderError>(),
            Some(
                RenderError::WatchdogPending
                    | RenderError::CompatPending(_)
                    | RenderError::ShaderFallbackPending(_)
            )
        )
}
/// A rendered frame uploaded ahead of its turn: the pixels of one frame key
/// under one cache generation ([`crate::render_cache::Held`]).
struct DiskUpload {
    held: crate::render_cache::Held,
    time: Ticks,
    size: Size,
    surface: Arc<ExportedSurface>,
}
impl DiskUpload {
    /// Whether it is still `scene`'s rendered frame at `time`: the cache
    /// has that frame under the key and generation it was read under. A
    /// document, media, font, tone or override change, a storage change or
    /// a deletion since the upload each fail this.
    fn is(&self, disk: &crate::render_cache::DiskFrames, scene: &Scene, time: Ticks) -> bool {
        self.time == time && disk.held(scene, time) == Some(self.held)
    }
}
/// A set of media files as the engine last looked at them.
struct MediaSeen {
    media: Arc<pm_export::render::MediaPaths>,
    /// Their versions as keys take them ([`crate::render_cache::Stamp`]):
    /// a frame is kept under them only when it read these very versions.
    stamps: Vec<crate::render_cache::Stamp>,
    looked: Instant,
}
impl MediaSeen {
    fn identity(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        self.stamps.hash(&mut hash);
        hash.finish()
    }
}
struct CachedFrame {
    key: crate::policy::CacheKey,
    surface: Arc<ExportedSurface>,
    failures: Arc<[LayerFailure]>,
}
pub fn metal_device(gpu: &Gpu) -> metal::Device {
    // HAL owns +1; to_owned retains a separate +1 for metal 0.33 and GPUI.
    unsafe {
        let hal = gpu.device.as_hal::<wgpu_hal::api::Metal>().unwrap();
        metal::DeviceRef::from_ptr(
            (&**hal.raw_device() as *const _ as *mut metal::MTLDevice).cast(),
        )
        .to_owned()
    }
}
pub struct Engine {
    pub mastering: crate::color::Mastering,
    gpu: Gpu,
    renderer: Renderer,
    metal: metal::Device,
    queue: metal::CommandQueue,
    pipeline: metal::RenderPipelineState,
    pool: Vec<Arc<ExportedSurface>>,
    /// A refinement band's target ([`Engine::refine`]).
    strip: Option<Arc<ExportedSurface>>,
    /// The last refinement's band count (0: drawn whole by its frame), for
    /// the diagnostic trace.
    pub(crate) refine_bands: u32,
    pub(crate) fonts: Option<pm_render::text::host::RenderFonts>,
    font_generation: Option<u64>,
    cache: VecDeque<CachedFrame>,
    /// The media of the scenes drawn lately ([`Engine::observed_inputs`]).
    media_seen: Vec<MediaSeen>,
    offscreen: Option<metal::Texture>,
    /// A capture of an SDR view: the frame presented without the delivery clip.
    capture_target: Option<metal::Texture>,
    pub cache_enabled: bool,
    studio_content: Option<(usize, u64)>,
    /// Set by the device's lost callback or an internal GPU error: the
    /// worker rebuilds the engine on a new device.
    lost: Arc<AtomicBool>,
    /// A validation or out-of-memory error raised since the last frame. The
    /// device is fine; the frame that raised it fails and is never shown.
    failed: Arc<std::sync::Mutex<Option<String>>>,
    /// Wakes an idle worker when the device is lost ([`Engine::set_waker`]).
    waker: Waker,
    resources: Arc<crate::resources::ResourceSnapshot>,
    /// Why a contributed definition cannot draw, for the viewer's status.
    pub definition_failure: Option<String>,
    /// Built-in definitions still to load on the service thread.
    pub(crate) builtins_pending: bool,
    /// The translated-shader folder, kept for a rebuilt engine.
    pub(crate) shader_cache: Option<std::path::PathBuf>,
    /// Effects the document uses that no installed provider defines; the
    /// viewer draws their layers without them.
    pub missing_effects: Vec<pm_render::effects::versions::MissingEffect>,
    pub(crate) compat: Option<Arc<crate::compat::CompatStores>>,
    /// The store of the document whose frame is rendering.
    compat_document: Option<Arc<crate::compat::CompatStore>>,
    /// The peaks of the content on screen as its media were observed, and
    /// the output tone of its frames.
    hdr: Option<((u64, u64, String, u64), Option<crate::color::HdrView>)>,
    output_tone: Option<pm_render::OutputTone>,
    /// Where the viewer's analysis goes, while a Scopes panel is showing.
    pub scope_frames: Option<(
        pm_render::scopes::ScopeConfig,
        async_channel::Sender<crate::scopes::ScopeResult>,
    )>,
    scopes: Option<crate::scopes::Tap>,
    /// The disk render cache playback reads (`crate::render_cache`).
    pub(crate) disk: Option<crate::render_cache::DiskFrames>,
    last_frame: Instant,
    idle_released: bool,
    /// Playing frames in a row that gave way to the disk reader.
    disk_late: u32,
    /// The next playing frame's rendered frame, uploaded already
    /// ([`Engine::upload_ahead`]).
    disk_next: Option<DiskUpload>,
    memory_client: crate::memory::Registration,
    memory_request: Arc<std::sync::atomic::AtomicU64>,
    budgeted_surfaces: u64,
}
type Waker = Arc<std::sync::Mutex<Option<std::sync::mpsc::SyncSender<()>>>>;
fn lose(lost: &AtomicBool, waker: &Waker) {
    lost.store(true, std::sync::atomic::Ordering::Release);
    if let Some(wake) = &*waker.lock().unwrap() {
        wake.try_send(()).ok();
    }
}
pub struct Submission {
    pub drawable: Option<metal::MetalDrawable>,
    pub trace: Arc<std::sync::Mutex<Trace>>,
    pub command: metal::CommandBuffer,
    pub surface: Arc<ExportedSurface>,
    pub readback: Option<metal::Buffer>,
    /// Tests: the presented picture of a capture whose readback skipped the
    /// presentation's SDR clip.
    #[cfg(test)]
    pub presented: Option<metal::Buffer>,
    pub width: u32,
    pub height: u32,
    /// Layers left out of this frame, with why.
    pub failures: Arc<[LayerFailure]>,
    /// Video drew a nearest decoded frame, or a raster override layer its
    /// nearest answer; the exact one is on its way.
    pub stand_ins: bool,
    /// The override layers drawn from a nearest answer (or not at all).
    pub compat_late: Vec<CompatNeed>,
}
impl Engine {
    pub fn memory_usage(&self) -> crate::memory::CacheUsage {
        let media = self.renderer.media_usage();
        crate::memory::CacheUsage {
            target_pool_bytes: self.renderer.target_pool_bytes(),
            target_pool_idle: self.renderer.target_pool_idle(),
            targets_created: self.renderer.target_pool_created(),
            image_bytes: self.renderer.image_cache_bytes(),
            converted_frame_bytes: self.renderer.converted_frame_bytes(),
            decoded_frame_bytes: media.frame_cache as u64,
            decoder_bytes: media.decoders as u64,
            decoder_sessions: media.sessions,
            decoder_pool_free_bytes: media.pool.free_bytes as u64,
            disk_read_ahead_bytes: self.disk.as_ref().map_or(0, |disk| disk.ready_bytes()),
            surface_bytes: self
                .pool
                .iter()
                .map(|s| u64::from(s.size.width) * u64::from(s.size.height) * 8)
                .sum(),
            text_bytes: self.renderer.text_cache_bytes(),
            vector_bytes: self.renderer.vector_cache_bytes(),
            pyramid_bytes: self.renderer.pyramid_bytes(),
            geometry_bytes: self.renderer.scene3d_resident().mesh_bytes
                + self.renderer.scene3d_resident().map_bytes,
            binding_bytes: self.renderer.binding_cache_bytes(),
            gpu_allocated_bytes: self.metal.current_allocated_size(),
            // Counted by the service across engine rebuilds.
            project_releases: 0,
        }
    }

    pub fn release_idle_memory(&mut self) {
        objc2::rc::autoreleasepool(|_| self.release_idle_memory_inner());
        crate::memory::pressure_relief();
    }

    fn release_idle_memory_inner(&mut self) {
        self.renderer.clear_draw_reuse();
        self.renderer.release_idle_targets();
        self.renderer.release_idle_media();
        self.renderer.trim(0);
        self.cache.clear();
        self.disk_next = None;
        self.pool.retain(|surface| Arc::strong_count(surface) > 1);
        self.offscreen = None;
        self.idle_released = true;
        let _ = self.gpu.device.poll(wgpu::PollType::Poll);
    }

    pub fn maintain_memory(&mut self) -> bool {
        self.govern_memory();
        if !self.idle_released && self.last_frame.elapsed() >= Duration::from_secs(2) {
            self.release_idle_memory();
            return true;
        }
        false
    }

    pub fn release_project_memory(&mut self) {
        objc2::rc::autoreleasepool(|_| self.release_project_memory_inner());
        crate::memory::pressure_relief();
    }

    fn release_project_memory_inner(&mut self) {
        self.release_idle_memory_inner();
        self.renderer.release_project_memory();
        self.media_seen.clear();
        self.compat_document = None;
        self.resources = Arc::default();
        self.studio_content = None;
        self.font_generation = None;
        self.hdr = None;
        self.missing_effects.clear();
        self.definition_failure = None;
        self.scopes = None;
        self.scope_frames = None;
        self.memory_request.store(
            self.memory_client.grant(),
            std::sync::atomic::Ordering::Release,
        );
        self.govern_memory();
        // Process deferred wgpu/Metal destruction after the last cache
        // handles go, rather than waiting for the next project's submission.
        self.gpu.queue.submit([]);
        self.gpu.wait_idle();
    }

    pub fn govern_memory(&mut self) {
        objc2::rc::autoreleasepool(|_| self.govern_memory_inner());
    }

    fn govern_memory_inner(&mut self) {
        use std::sync::atomic::Ordering;
        if let Some(disk) = &self.disk {
            disk.memory_pressure(crate::memory::pressure());
        }
        let request = self.memory_request.swap(u64::MAX, Ordering::AcqRel);
        let surfaces = self.memory_usage().surface_bytes;
        let share = self.memory_client.grant();
        if request != u64::MAX || surfaces != self.budgeted_surfaces {
            self.budgeted_surfaces = surfaces;
            // In-flight surfaces are pinned. Divide only the remainder,
            // so every independently reusable cache shares the same allowance.
            let cache = share.saturating_sub(surfaces);
            self.renderer
                .apply_media_lease(pm_media::decode::service::MediaLease {
                    frame_cache: (cache * 15 / 100) as usize,
                    decoders: (cache / 4) as usize,
                });
            self.renderer.set_image_cache_budget(cache / 5);
            self.renderer.set_converted_frame_budget(cache / 10);
            self.renderer.set_target_pool_budget(cache / 10);
            self.renderer.set_vector_cache_budget(cache / 20, 512);
            self.renderer.trim(cache / 20);
            if request != u64::MAX {
                self.cache.clear();
                self.pool.retain(|s| Arc::strong_count(s) > 1);
            }
            if crate::memory::pressure() == crate::memory::Pressure::Critical {
                self.release_idle_memory();
                self.renderer.trim(0);
                self.renderer.set_image_cache_budget(share / 5);
                crate::memory::pressure_relief();
            }
        }
        let u = self.memory_usage();
        self.memory_client
            .report(pm_protocol::lease::LeaseCategories {
                frame_cache: crate::memory::bytes(u.decoded_frame_bytes + u.disk_read_ahead_bytes),
                render_cache: crate::memory::bytes(
                    u.pyramid_bytes
                        + u.target_pool_bytes
                        + u.image_bytes
                        + u.converted_frame_bytes
                        + u.text_bytes
                        + u.vector_bytes,
                ),
                surface_pool: crate::memory::bytes(u.surface_bytes),
                decoders: crate::memory::bytes(u.decoder_bytes),
                geometry_3d: crate::memory::bytes(u.geometry_bytes),
                ..Default::default()
            });
    }
    /// A fresh engine on a new device after `lost` reported a loss. Fonts
    /// carry over; every surface, cache and program is made again.
    pub fn rebuild(&self) -> Result<Self> {
        let gpu = Gpu::new("pm-engine-service").map_err(|e| anyhow!("{e}"))?;
        let mut next = Self::with_shader_cache(gpu, self.shader_cache.clone())?;
        next.set_waker(self.waker.lock().unwrap().clone());
        next.resources = self.resources.clone();
        next.mastering = self.mastering;
        next.cache_enabled = self.cache_enabled;
        if let Some(fonts) = &self.fonts {
            next.set_fonts(fonts.clone());
        }
        next.set_compat(self.compat.clone());
        next.disk = self.disk.clone();
        Ok(next)
    }
    pub fn set_waker(&self, wake: Option<std::sync::mpsc::SyncSender<()>>) {
        *self.waker.lock().unwrap() = wake;
    }
    /// Non-blocking: runs wgpu's completed callbacks, a device loss among them.
    pub fn poll(&self) {
        objc2::rc::autoreleasepool(|_| {
            let _ = self.gpu.device.poll(wgpu::PollType::Poll);
        });
    }
    pub fn lost(&self) -> bool {
        self.lost.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn metal(&self) -> &metal::DeviceRef {
        &self.metal
    }
    /// Test hook: lose the device as a removed or reset GPU does.
    #[cfg(test)]
    pub fn destroy_device(&self) {
        self.gpu.device.destroy();
    }
    /// Test hook: a real validation error, as a bad pipeline or texture raises.
    #[cfg(test)]
    pub fn raise_validation_error(&self) {
        let _invalid = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("invalid"),
            size: 4,
            usage: wgpu::BufferUsages::empty(),
            mapped_at_creation: false,
        });
    }
    pub fn new(gpu: Gpu) -> Result<Self> {
        Self::with_shader_cache(gpu, None)
    }
    /// `shader_cache` keeps translated shaders across launches, so a launch
    /// after the first doesn't translate every built-in effect again.
    pub fn with_shader_cache(gpu: Gpu, shader_cache: Option<std::path::PathBuf>) -> Result<Self> {
        let mut engine = Self::without_definitions(gpu, shader_cache)?;
        engine.load_builtin_definitions()?;
        Ok(engine)
    }
    /// Every built-in effect's definition. On a profile whose shader cache is
    /// cold this translates each of them (about a second), so the service
    /// does it on its own thread, never while the app waits to open a window.
    pub fn load_builtin_definitions(&mut self) -> Result<()> {
        self.renderer
            .set_definitions(&pm_render::all_builtin_definitions())?;
        Ok(())
    }
    /// An engine whose built-in definitions are loaded later
    /// ([`Self::load_builtin_definitions`]).
    pub(crate) fn without_definitions(
        gpu: Gpu,
        shader_cache: Option<std::path::PathBuf>,
    ) -> Result<Self> {
        let lost = Arc::new(AtomicBool::new(false));
        let waker = Waker::default();
        let (flag, wake) = (lost.clone(), waker.clone());
        gpu.device.set_device_lost_callback(move |reason, message| {
            eprintln!("Viewer GPU device lost ({reason:?}): {message}");
            lose(&flag, &wake);
        });
        // wgpu's default handler panics, which would stop every viewer. A
        // validation or out-of-memory error fails the frame; an internal
        // error rebuilds.
        let failed = Arc::<std::sync::Mutex<Option<String>>>::default();
        let (flag, wake, frame) = (lost.clone(), waker.clone(), failed.clone());
        gpu.device
            .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
                eprintln!("Viewer GPU error: {error}");
                match error {
                    wgpu::Error::Validation { .. } | wgpu::Error::OutOfMemory { .. } => {
                        *frame.lock().unwrap() = Some(error.to_string());
                    }
                    wgpu::Error::Internal { .. } => lose(&flag, &wake),
                }
            }));
        let metal = metal_device(&gpu);
        let library = metal
            .new_library_with_source(
                &format!(
                    "#define SDR_MIN {:?}\n#define SDR_MAX {:?}\n{}",
                    pm_color::delivery::SDR_MIN,
                    pm_color::delivery::SDR_MAX,
                    include_str!("viewer.metal")
                ),
                &metal::CompileOptions::new(),
            )
            .map_err(|s| anyhow!(s))?;
        let desc = metal::RenderPipelineDescriptor::new();
        let vertex = library
            .get_function("vs", None)
            .map_err(anyhow::Error::msg)?;
        let fragment = library
            .get_function("fs", None)
            .map_err(anyhow::Error::msg)?;
        desc.set_vertex_function(Some(&vertex));
        desc.set_fragment_function(Some(&fragment));
        desc.color_attachments()
            .object_at(0)
            .unwrap()
            .set_pixel_format(metal::MTLPixelFormat::RGBA16Float);
        let pipeline = metal
            .new_render_pipeline_state(&desc)
            .map_err(|s| anyhow!(s))?;
        let memory_request = Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX));
        let pending = memory_request.clone();
        let memory_client = crate::memory::Registration::new(12, move |share, _| {
            pending.store(share, std::sync::atomic::Ordering::Release);
        });
        let mut renderer = Renderer::new(gpu.device.clone(), gpu.queue.clone());
        renderer.apply_media_lease(pm_media::decode::service::MediaLease {
            frame_cache: 24 << 20,
            decoders: 32 << 20,
        });
        renderer.set_draw_reuse(true);
        renderer.set_program_cache_dir(shader_cache.clone());
        Ok(Self {
            mastering: Default::default(),
            // Use the producer's actual queue. Metal tracks the shared source
            // texture between producer and consumer command buffers. No CPU
            // completion wait or cross-queue race is needed (§3.3 / N8).
            queue: unsafe {
                let hal = gpu.queue.as_hal::<wgpu_hal::api::Metal>().unwrap();
                metal::CommandQueueRef::from_ptr(
                    (hal.as_raw() as *const _ as *mut metal::MTLCommandQueue).cast(),
                )
                .to_owned()
            },
            fonts: None,
            font_generation: None,
            gpu,
            renderer,
            metal,
            pipeline,
            pool: Vec::with_capacity(4),
            cache: VecDeque::with_capacity(4),
            media_seen: Vec::new(),
            offscreen: None,
            capture_target: None,
            cache_enabled: true,
            strip: None,
            refine_bands: 0,
            studio_content: None,
            lost,
            failed,
            waker,
            resources: Arc::default(),
            definition_failure: None,
            builtins_pending: false,
            shader_cache: shader_cache.clone(),
            missing_effects: Vec::new(),
            compat: None,
            compat_document: None,
            hdr: None,
            output_tone: None,
            scope_frames: None,
            scopes: None,
            disk: None,
            last_frame: Instant::now(),
            idle_released: false,
            disk_late: 0,
            disk_next: None,
            memory_client,
            memory_request,
            budgeted_surfaces: u64::MAX,
        })
    }
    /// Raster overrides the viewer's frames ask for (`crate::compat`).
    pub fn set_compat(&mut self, stores: Option<Arc<crate::compat::CompatStores>>) {
        self.compat = stores;
        self.compat_document = None;
        self.renderer.set_compat_rasters(None);
    }
    /// The next frames are `doc_id`'s: they ask that document's store.
    pub(crate) fn use_compat_document(&mut self, doc_id: u64) {
        let store = self.compat.as_ref().map(|stores| stores.document(doc_id));
        if store.as_ref().map(Arc::as_ptr) != self.compat_document.as_ref().map(Arc::as_ptr) {
            self.renderer.set_compat_rasters(
                store
                    .clone()
                    .map(|s| s as Arc<dyn pm_render::compat::CompatRasters>),
            );
            self.compat_document = store;
        }
    }
    pub(crate) fn compat(&self) -> Option<&Arc<crate::compat::CompatStore>> {
        self.compat_document.as_ref()
    }
    pub fn set_fonts(&mut self, fonts: pm_render::text::host::RenderFonts) {
        self.fonts = Some(fonts);
        self.font_generation = None;
        self.clear_cache();
    }
    /// The font registry generation the next frame draws with: fonts sync
    /// from the broker first, so the disk render cache observes a font
    /// change before the frame it would affect looks for a disk frame.
    pub(crate) fn font_generation(&mut self) -> Option<u64> {
        self.sync_fonts();
        self.font_generation
    }
    fn sync_fonts(&mut self) {
        if let Some(fonts) = &self.fonts
            && fonts.sync(&mut self.renderer, &mut self.font_generation)
        {
            self.cache.clear();
        }
    }
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
    /// A settled paused frame read its stills whole, as an export does. Its
    /// picture is kept; the originals are not (a capture's stay within the
    /// image budget, for the capture after it).
    fn release_final_stills(&mut self, scene: &Scene, spec: &FrameSpec) {
        if !spec.exact && spec.final_picture(scene) {
            self.renderer.release_whole_stills();
        }
    }
    /// The key a frame of `spec` is kept in memory under.
    fn cache_key(
        &mut self,
        scene: &Scene,
        spec: &FrameSpec,
        size: Size,
    ) -> crate::policy::CacheKey {
        crate::policy::CacheKey {
            content: scene.content,
            revision: scene.doc.revision(),
            time: spec.time.get(),
            width: size.width,
            height: size.height,
            crop: spec.output.crop,
            moment: spec.moment,
            scale: spec.scale.to_bits(),
            sampling: spec.sampling(),
            finished: spec.final_picture(scene),
            inputs: self.observed_inputs(scene),
        }
    }
    /// What a frame of `scene` reads besides its document, as observed: its
    /// media files (looked at every 2 s, as the disk render cache looks)
    /// and the override wrappers answering. A frame kept in memory is the
    /// picture of one such state, as a frame on disk is: a file saved again
    /// by another app is another state, whose frames are drawn. (Fonts and
    /// the output tone retire every kept frame when they change.)
    fn observed_inputs(&mut self, scene: &Scene) -> u64 {
        use std::hash::{Hash, Hasher};
        let at = self.media_seen_at(scene);
        let fresh = at.is_some_and(|at| {
            self.media_seen[at].looked.elapsed() < crate::render_cache::MEDIA_RECHECK
        });
        let media = if let (Some(at), true) = (at, fresh) {
            self.media_seen[at].identity()
        } else {
            // With the media, the font files read: one rewritten in place
            // counts as a change of the installed fonts.
            pm_text::system_fonts::refresh();
            let seen = MediaSeen {
                media: scene.media.clone(),
                stamps: match &self.disk {
                    Some(disk) => disk.media_stamps(&scene.media),
                    None => crate::render_cache::stamps(&scene.media, None),
                },
                looked: Instant::now(),
            };
            let identity = seen.identity();
            let before = at.map(|at| self.media_seen.remove(at).identity());
            if before.is_some_and(|before| before != identity) {
                // Kept draws name their files, not what was in them.
                self.renderer.clear_draw_reuse();
            }
            // A main viewer and a popout, each playing and paused.
            if self.media_seen.len() >= 4 {
                self.media_seen.remove(0);
            }
            self.media_seen.push(seen);
            identity
        };
        let overrides = self
            .compat_document
            .as_ref()
            .filter(|store| store.producing_now())
            .map(|store| store.cache_identity());
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (media, overrides).hash(&mut hash);
        hash.finish()
    }
    fn media_seen_at(&self, scene: &Scene) -> Option<usize> {
        self.media_seen
            .iter()
            .position(|seen| Arc::ptr_eq(&seen.media, &scene.media) || seen.media == scene.media)
    }
    /// Whether a frame of `scene` just rendered read its media as they
    /// were last looked at (`reads`: `pm_render::media::take_reads`): only
    /// then is it kept under the key taken from that look. A file saved
    /// again within the two seconds between looks is drawn (the frame is
    /// shown) but not kept as a frame of the file as it was, and the media
    /// are looked at again for the next frame.
    fn reads_observed(
        &mut self,
        scene: &Scene,
        reads: &[(
            std::path::PathBuf,
            Option<pm_text::file_version::FileVersion>,
        )],
    ) -> bool {
        let Some(at) = self.media_seen_at(scene) else {
            return false;
        };
        let content_named = self.disk.as_ref().and_then(|disk| disk.content_named());
        let observed = crate::render_cache::reads_are(
            reads,
            &self.media_seen[at].stamps,
            content_named.as_deref(),
        )
        .is_ok();
        if !observed {
            self.media_seen.remove(at);
            self.renderer.clear_draw_reuse();
        }
        observed
    }
    /// Changes when a frame a scrub or playback render missed has decoded.
    pub fn media_landed(&self) -> u64 {
        self.renderer.media_landed()
    }
    /// A frame read from disk into `surface`.
    fn upload(&self, surface: &ExportedSurface, size: Size, pixels: &[u8]) {
        self.gpu.queue.write_texture(
            surface.texture.as_image_copy(),
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.width * 8),
                rows_per_image: Some(size.height),
            },
            surface.texture.size(),
        );
        self.gpu.queue.submit([]);
    }

    /// After a playing frame is submitted: the frame after it, if it is
    /// rendered on disk and read already, is uploaded now, in the slack
    /// before its turn. Uploading a composition-size frame when its turn
    /// came took 5 ms on a quiet machine and 12 to 100 ms on a busy one,
    /// of the 15 ms a frame has before its refresh.
    pub fn upload_ahead(&mut self, scene: &Scene, time: Ticks) {
        let Some(disk) = self.disk.clone() else {
            self.disk_next = None;
            return;
        };
        if self
            .disk_next
            .as_ref()
            .is_some_and(|n| n.is(&disk, scene, time))
        {
            return;
        }
        self.disk_next = None;
        let crate::render_cache::Read::Frame(size, pixels, held) =
            disk.read_within(scene, time, std::time::Duration::ZERO)
        else {
            return;
        };
        let Ok(surface) = self.cache_surface(size) else {
            return;
        };
        self.upload(&surface, size, &pixels);
        self.disk_next = Some(DiskUpload {
            held,
            time,
            size,
            surface,
        });
    }
    /// The rendered frames' state changed (a new observation): a frame
    /// uploaded ahead is of the state before.
    pub(crate) fn drop_upload_ahead(&mut self) {
        self.disk_next = None;
    }

    /// Opens and warms the videos of a coming frame on background workers, so
    /// a clip entering the playhead does not stall the frame that meets it.
    /// The video draws planned at `time`: (layer, instance).
    pub fn prepare(&mut self, scene: &Scene, time: Ticks, spec: FrameSpec) -> Vec<(String, u64)> {
        if self.sync_studio(scene).is_err() {
            return Vec::new();
        }
        let Ok(request) = spec.request(scene) else {
            return Vec::new();
        };
        // Media is prepared as the frame will read it (a viewer frame's
        // stills at the size it draws them, paused or playing; video at the
        // view's size only while playing or dragging).
        self.renderer
            .set_preview_media(spec.preview_stills(scene), spec.preview_video());
        // A plan that cannot be built reports through the frame itself.
        self.renderer
            .prepare_media(
                &scene.doc,
                &scene.comp,
                time,
                &request,
                scene.media.as_ref(),
            )
            .map(|prepared| prepared.videos)
            .unwrap_or_default()
    }
    /// The extension definitions the next frames render with (the viewer's
    /// pinned resource snapshot). A new snapshot installs them again.
    /// The definitions this engine draws with (the render cache's fingerprint).
    pub(crate) fn definitions(&self) -> &[pm_protocol::definitions::DefinitionSnapshot] {
        &self.resources.definitions
    }
    pub fn set_resources(&mut self, resources: Arc<crate::resources::ResourceSnapshot>) {
        if !crate::resources::ResourceSnapshot::same_pins(&self.resources, &resources) {
            self.resources = resources;
            self.studio_content = None;
            self.cache.clear();
        }
    }
    fn sync_studio(&mut self, scene: &Scene) -> Result<()> {
        let key = (Arc::as_ptr(&scene.doc) as usize, scene.content);
        if self.studio_content != Some(key) {
            // A bad contribution draws nothing and is reported; frames carry on.
            // A compiler slot that was busy is retried, never reported as broken.
            let busy = |e: &str| e.contains("compiler-busy");
            let failures = self
                .renderer
                .set_document_effects(&scene.doc, &self.resources.definitions)
                .map_err(|e| {
                    if busy(&e) {
                        anyhow::Error::new(CompilerBusy)
                    } else {
                        anyhow::Error::msg(e)
                    }
                })?;
            if failures.iter().any(|failure| busy(&failure.message)) {
                return Err(anyhow::Error::new(CompilerBusy));
            }
            self.definition_failure = failures.first().map(|failure| failure.message.clone());
            self.missing_effects = self.renderer.missing_effects().to_vec();
            #[cfg(feature = "kernel")]
            crate::studio::attach(&mut self.renderer, scene.doc.clone(), scene.media.as_ref());
            // A document's first frame compiles the frame and text pipelines
            // it draws with, so no frame met while playing does (built ones
            // are kept: later documents pay nothing).
            self.renderer.prewarm(&scene.doc);
            // Effect programs (built-in and extension GLSL) build on the
            // renderer's worker now, not inside the first frame that draws
            // each one while playing.
            self.renderer.prewarm_programs(&scene.doc);
            self.studio_content = Some(key);
        }
        Ok(())
    }
    /// The output tone of `scene`'s frames as its media are now observed:
    /// what the disk render cache's identity takes
    /// ([`Self::tone_identity`]), before the frame that draws with it.
    pub(crate) fn sync_tone(&mut self, scene: &Scene) {
        self.sync_hdr(scene);
    }
    /// The HDR view of this content, and the frame cache kept to its tone.
    /// The view reads the media's own peaks: it is of the media as observed
    /// ([`Self::observed_inputs`]), so a clip replaced on disk by one with
    /// another peak is tone-mapped with its own.
    fn sync_hdr(&mut self, scene: &Scene) -> Option<crate::color::HdrView> {
        let key = (
            scene.content,
            scene.doc.revision(),
            scene.comp.clone(),
            self.observed_inputs(scene),
        );
        if self.hdr.as_ref().is_none_or(|(k, _)| *k != key) {
            let view = crate::color::HdrView::of(&scene.doc, &scene.comp, &scene.media);
            self.hdr = Some((key, view));
        }
        let view = self.hdr.as_ref().and_then(|(_, view)| *view);
        let tone = view.and_then(|v| v.tone(self.mastering));
        if tone != self.output_tone {
            self.output_tone = tone;
            self.cache.clear();
        }
        view
    }
    /// A new tap analyzes the very next frame.
    pub fn scopes_changed(&mut self) {
        if let Some(tap) = &mut self.scopes {
            tap.reset();
        }
    }
    /// Gives the frame waiting for the tap its turn, on a viewer that has gone
    /// quiet. Returns when to ask again, if it still waits.
    pub fn flush_scopes(
        &mut self,
        tap: Option<&(
            pm_render::scopes::ScopeConfig,
            async_channel::Sender<crate::scopes::ScopeResult>,
        )>,
    ) -> Option<Duration> {
        let scopes = self.scopes.as_mut()?;
        scopes.retain(tap);
        scopes.flush(&self.gpu.device, &self.gpu.queue)
    }
    pub fn export(&mut self, scene: &Scene, time: Ticks, size: Size) -> Result<Vec<f32>> {
        self.renderer.set_preview_media(false, false);
        self.last_frame = Instant::now();
        self.idle_released = false;
        self.sync_studio(scene)?;
        self.sync_fonts();
        let mut request = pm_render::RenderRequest::sized(size, Precision::Float16);
        request.export = true;
        request.output_kind = pm_render::OutputKind::Final;
        self.renderer.set_draw_reuse(false);
        let f = pm_export::render::render_frame(
            &mut self.renderer,
            &scene.doc,
            &scene.comp,
            time,
            &request,
            scene.media.as_ref(),
            &pm_export::render::Unrestricted,
            1,
            &AtomicBool::new(false),
        );
        self.renderer.set_draw_reuse(true);
        let f = f?;
        if let Some(error) = self.failed.lock().unwrap().take() {
            bail!("The GPU refused this frame: {error}");
        }
        if !f.failures.is_empty() {
            bail!("export layer failures: {:?}", f.failures);
        }
        Ok(f.rgba)
    }
    /// The viewer's output tone, as the render cache's identity takes it.
    pub(crate) fn tone_identity(&self) -> Option<String> {
        self.output_tone.map(|tone| format!("{tone:?}"))
    }
    pub fn frame(
        &mut self,
        scene: &Scene,
        spec: FrameSpec,
        layer: Option<&metal::MetalLayer>,
        overlays: &crate::OverlayGeometry,
        capture: bool,
        trace: Arc<std::sync::Mutex<Trace>>,
    ) -> Result<Option<Submission>> {
        self.govern_memory();
        self.last_frame = Instant::now();
        self.idle_released = false;
        objc2::rc::autoreleasepool(|_| {
            self.encode_inner(scene, spec, layer, overlays, capture, trace)
        })
    }
    fn encode_inner(
        &mut self,
        scene: &Scene,
        spec: FrameSpec,
        layer: Option<&metal::MetalLayer>,
        overlays: &crate::OverlayGeometry,
        capture: bool,
        trace: Arc<std::sync::Mutex<Trace>>,
    ) -> Result<Option<Submission>> {
        {
            let mut t = trace.lock().unwrap();
            t.enter_ms = (now() - t.worker_start) * 1e3;
        }
        self.sync_studio(scene)?;
        self.sync_fonts();
        self.gpu.device.poll(wgpu::PollType::Poll)?;
        let FrameSpec { time, output, .. } = spec;
        let size = spec.size_in(scene);
        let drawn_whole = spec.whole_composition(scene).is_some();
        let hdr = self.sync_hdr(scene);
        let key = self.cache_key(scene, &spec, size);
        // A revision, size or observed-input change retires all old cached
        // frames. Leases held by submitted consumers still protect their
        // storage until completion.
        self.cache
            .retain(|c| c.key.content != key.content || c.key.same_content(key));
        let hit = (self.cache_enabled && !spec.exact)
            .then(|| self.cache.iter().position(|c| c.key == key))
            .flatten();
        let mut stand_ins = false;
        let mut compat_late = Vec::new();
        // A frame rendered on disk: shown in an uncropped viewer instead of
        // drawn again (the present pass scales it to the drawable), playing,
        // scrubbing and paused alike: it is the composition-size frame an
        // export draws, so a paused playhead in a rendered range shows its
        // picture at once, whatever else the GPU is doing. A capture always
        // draws its own exact frame.
        // Never upscaled: a canvas larger than the composition renders.
        let canvas = output.crop.map_or((output.width, output.height), |c| {
            (c.canvas_width, c.canvas_height)
        });
        let looked = hit.is_none() && !spec.exact;
        // A playing or paused frame waits a moment for its frame on disk
        // rather than drawing what was rendered because it can't be drawn
        // in time (a dragged one never waits).
        let patience = match spec.moment {
            Moment::Playing => crate::render_cache::PATIENCE,
            Moment::Paused => crate::render_cache::PAUSED_PATIENCE,
            Moment::Scrub => std::time::Duration::ZERO,
        };
        // A paused frame shows the frame on disk only where it would draw
        // that very picture itself: the final picture of the whole
        // composition at its own size (its refinement at the paused Full
        // scale, or a drawable that is the composition's size). Where a
        // paused frame is drawn otherwise (zoomed in, at a reduced quality,
        // or so far out that Full is bounded to the drawable), it is drawn
        // as ever: whether a frame is rendered on disk never changes a
        // paused picture.
        let as_drawn = spec.moment != Moment::Paused || spec.settles_final(scene);
        let fits = |size: &Size| as_drawn && canvas.0 <= size.width && canvas.1 <= size.height;
        let asked = std::time::Instant::now();
        // Uploaded in the slack after the frame before: nothing to read.
        // Shown only while it still is this frame's picture.
        let uploaded = self.disk_next.take().filter(|next| {
            looked
                && fits(&next.size)
                && self
                    .disk
                    .as_ref()
                    .is_some_and(|disk| next.is(disk, scene, time))
        });
        let disk = match (looked && uploaded.is_none())
            .then(|| Some(self.disk.as_ref()?.read_within(scene, time, patience)))
            .flatten()
        {
            Some(crate::render_cache::Read::Frame(size, pixels, _)) if fits(&size) => {
                self.disk_late = 0;
                Some((size, pixels))
            }
            Some(crate::render_cache::Read::Late(size))
                if spec.moment == Moment::Playing
                    && fits(&size)
                    && self.disk_late < crate::render_cache::LATE_AT_MOST =>
            {
                self.disk_late += 1;
                return Err(DiskFramePending.into());
            }
            other => {
                // Drawn after all: the frames after it give way again.
                if matches!(other, Some(crate::render_cache::Read::Late(_))) {
                    self.disk_late = 0;
                }
                None
            }
        };
        // A paused playhead keeps the frames a play starts from read.
        if spec.moment == Moment::Paused
            && let Some(disk) = &self.disk
        {
            disk.warm(scene, time);
        }
        if uploaded.is_some() {
            self.disk_late = 0;
        }
        let from_disk = disk.is_some() || uploaded.is_some();
        // The source is the whole composition: presented as the drawable's
        // window of the canvas, a rendered frame and a drawn one alike.
        let whole = from_disk || drawn_whole;
        trace.lock().unwrap().disk_wait_ms = asked.elapsed().as_secs_f64() * 1e3;
        trace.lock().unwrap().disk = match (looked, from_disk) {
            (false, _) => 0,
            (true, false) => 1,
            (true, true) => 2,
        };
        let size = disk.as_ref().map_or(size, |(size, _)| *size);
        let size = uploaded.as_ref().map_or(size, |next| next.size);
        let (surface, failures) = if let Some(next) = uploaded {
            trace.lock().unwrap().cache_hit = true;
            (next.surface, Arc::from(Vec::new()))
        } else if let Some(index) = hit {
            let entry = self.cache.remove(index).unwrap();
            let found = (entry.surface.clone(), entry.failures.clone());
            self.cache.push_back(entry);
            trace.lock().unwrap().cache_hit = true;
            found
        } else {
            let uploading = std::time::Instant::now();
            let surface = self.cache_surface(size)?;
            if let Some((_, pixels)) = disk {
                self.upload(&surface, size, &pixels);
                if let Some(error) = self.failed.lock().unwrap().take() {
                    bail!("The GPU refused this frame: {error}");
                }
                let mut t = trace.lock().unwrap();
                t.cache_hit = true;
                t.disk_upload_ms = uploading.elapsed().as_secs_f64() * 1e3;
                drop(t);
                (surface, Arc::from(Vec::new()))
            } else {
                let target = Target::wrap(surface.texture.clone());
                let mut request = spec.request(scene)?;
                request.output_tone = self.output_tone;
                // An exact read never takes a stand-in, whatever the moment,
                // and draws from original media without reused draws.
                self.renderer
                    .set_preview_media(spec.preview_stills(scene), spec.preview_video());
                self.renderer.set_draw_reuse(!spec.exact);
                self.renderer
                    .set_scrub(spec.moment == Moment::Scrub && !spec.exact);
                self.renderer.set_playback(if spec.exact {
                    None
                } else {
                    spec.decode_deadline
                });
                self.renderer.set_isolate_layers(true);
                if let Some(store) = &self.compat_document {
                    store.begin_frame(if spec.exact {
                        Moment::Paused
                    } else {
                        spec.moment
                    });
                }
                trace.lock().unwrap().render_start = now();
                let cpu_start = crate::trace::thread_cpu_ms();
                let work = trace.lock().unwrap().count_work.then(pm_perf::snapshot);
                pm_render::media::note_reads();
                let rendered = self.renderer.render_into(
                    &scene.doc,
                    &scene.comp,
                    time,
                    &request,
                    scene.media.as_ref(),
                    &target,
                    None,
                );
                let reads = pm_render::media::take_reads();
                {
                    let mut t = trace.lock().unwrap();
                    t.render_submitted = now();
                    t.render_cpu_ms = crate::trace::thread_cpu_ms() - cpu_start;
                    if let Some(work) = work {
                        t.work = pm_perf::snapshot().since(&work);
                    }
                }
                self.renderer.set_scrub(false);
                self.renderer.set_playback(None);
                self.renderer.set_isolate_layers(false);
                compat_late = self
                    .compat_document
                    .as_ref()
                    .map(|store| store.end_frame())
                    .unwrap_or_default();
                let rendered = rendered?;
                self.renderer.trim_submitted_frame_caches();
                self.release_final_stills(scene, &spec);
                if let Some(error) = self.failed.lock().unwrap().take() {
                    bail!("The GPU refused this frame: {error}");
                }
                if let Some(error) = self.fonts.as_ref().and_then(|f| f.after_render()) {
                    return Err(FontsPending(error.to_string()).into());
                }
                let failures: Arc<[LayerFailure]> = rendered.failures.into();
                stand_ins = !rendered.stand_ins.is_empty() || !compat_late.is_empty();
                // A frame with stand-ins is not the exact picture of its
                // time, and one that read another version of a file than
                // the one its key was taken from is not that key's.
                if self.cache_enabled
                    && !spec.exact
                    && !stand_ins
                    && self.reads_observed(scene, &reads)
                {
                    self.cache.push_back(CachedFrame {
                        key,
                        surface: surface.clone(),
                        failures: failures.clone(),
                    });
                }
                (surface, failures)
            }
        };
        if let Some((config, frames)) = &self.scope_frames {
            self.scopes
                .get_or_insert_with(|| crate::scopes::Tap::new(self.gpu.device.clone()))
                .frame(&self.gpu.device, &self.gpu.queue, &surface, *config, frames);
        }
        let source = unsafe {
            let hal = surface.texture.as_hal::<wgpu_hal::api::Metal>().unwrap();
            metal::TextureRef::from_ptr(
                (hal.raw_handle() as *const _ as *mut metal::MTLTexture).cast(),
            )
            .to_owned()
        };
        trace.lock().unwrap().drawable_asked = now();
        let drawable = if let Some(layer) = layer {
            // A rebuilt engine has a new device; the layer follows it.
            if layer.device().as_ptr() != self.metal.as_ptr() {
                layer.set_device(&self.metal);
            }
            let Some(d) = layer.next_drawable() else {
                return Ok(None);
            };
            Some(d.to_owned())
        } else {
            None
        };
        trace.lock().unwrap().drawable_acquired = now();
        // The drawable is what is shown. A layer resized after the last
        // configure (its size publication can lag or be refused) is drawn at
        // its real size: dropping the drawable would retry this frame forever
        // and never show another.
        let output = match &drawable {
            Some(d) => output.resized(d.texture().width() as u32, d.texture().height() as u32),
            None => output,
        };
        if drawable.is_none()
            && self.offscreen.as_ref().is_none_or(|t| {
                t.width() != output.width as u64 || t.height() != output.height as u64
            })
        {
            let desc = metal::TextureDescriptor::new();
            desc.set_texture_type(metal::MTLTextureType::D2);
            desc.set_pixel_format(metal::MTLPixelFormat::RGBA16Float);
            desc.set_width(output.width as u64);
            desc.set_height(output.height as u64);
            desc.set_storage_mode(metal::MTLStorageMode::Private);
            desc.set_usage(
                metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead,
            );
            self.offscreen = Some(self.metal.new_texture(&desc));
        }
        let destination = drawable.as_ref().map_or_else(
            || self.offscreen.as_ref().unwrap().as_ref(),
            |d| d.texture(),
        );
        let command = self.queue.new_command_buffer().to_owned();
        let mut params = overlays.params();
        params.output = hdr
            .map_or(self.mastering, |v| {
                self.mastering.with_peaks(v.source_peak, v.mastering_peak)
            })
            .params()?;
        params.reduced = u32::from(size != Size::new(output.width, output.height));
        params.whole = u32::from(whole);
        params.drawable = [output.width as f32, output.height as f32];
        params.canvas = match output.crop {
            Some(c) => [
                c.x as f32,
                c.y as f32,
                c.canvas_width as f32,
                c.canvas_height as f32,
            ],
            None => [0., 0., output.width as f32, output.height as f32],
        };
        // A capture reads the engine's frame, not the display's picture: the
        // SDR delivery clip (mode 3) is presentation only, so a capture of an
        // SDR view is drawn again without it.
        let unclipped_capture = capture && params.output[0] == 3.;
        if unclipped_capture
            && self.capture_target.as_ref().is_none_or(|t| {
                t.width() != output.width as u64 || t.height() != output.height as u64
            })
        {
            let desc = metal::TextureDescriptor::new();
            desc.set_texture_type(metal::MTLTextureType::D2);
            desc.set_pixel_format(metal::MTLPixelFormat::RGBA16Float);
            desc.set_width(output.width as u64);
            desc.set_height(output.height as u64);
            desc.set_storage_mode(metal::MTLStorageMode::Private);
            desc.set_usage(
                metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead,
            );
            self.capture_target = Some(self.metal.new_texture(&desc));
        }
        let present = |target: &metal::TextureRef, params: &crate::OverlayParams| {
            let desc = metal::RenderPassDescriptor::new();
            let color = desc.color_attachments().object_at(0).unwrap();
            color.set_texture(Some(target));
            color.set_load_action(metal::MTLLoadAction::Clear);
            color.set_store_action(metal::MTLStoreAction::Store);
            let pass = command.new_render_command_encoder(desc);
            pass.set_render_pipeline_state(&self.pipeline);
            pass.set_fragment_texture(0, Some(&source));
            pass.set_fragment_bytes(
                0,
                size_of_val(params) as u64,
                (params as *const crate::OverlayParams).cast(),
            );
            pass.draw_primitives(metal::MTLPrimitiveType::Triangle, 0, 3);
            pass.end_encoding();
        };
        present(destination, &params);
        let read = |texture: &metal::TextureRef| {
            let b = self.metal.new_buffer(
                output.width as u64 * output.height as u64 * 8,
                metal::MTLResourceOptions::StorageModeShared,
            );
            let blit = command.new_blit_command_encoder();
            blit.copy_from_texture_to_buffer(
                texture,
                0,
                0,
                metal::MTLOrigin { x: 0, y: 0, z: 0 },
                metal::MTLSize {
                    width: output.width as u64,
                    height: output.height as u64,
                    depth: 1,
                },
                &b,
                0,
                output.width as u64 * 8,
                output.width as u64 * output.height as u64 * 8,
                metal::MTLBlitOption::None,
            );
            blit.end_encoding();
            b
        };
        #[cfg(test)]
        let mut presented = None;
        let readback = if unclipped_capture {
            let target = self.capture_target.as_ref().unwrap();
            present(
                target,
                &crate::OverlayParams {
                    output: [0.; 4],
                    ..params
                },
            );
            #[cfg(test)]
            {
                presented = Some(read(destination));
            }
            Some(read(target))
        } else if capture {
            Some(read(destination))
        } else {
            None
        };
        trace.lock().unwrap().encode_end = now();
        Ok(Some(Submission {
            drawable,
            trace,
            command,
            surface,
            readback,
            #[cfg(test)]
            presented,
            width: output.width,
            height: output.height,
            failures,
            stand_ins,
            compat_late,
        }))
    }
    /// A pooled surface of `size` for a frame the cache will hold, making
    /// room in the cache first.
    fn cache_surface(&mut self, size: Size) -> Result<Arc<ExportedSurface>> {
        let bytes = u64::from(size.width) * u64::from(size.height) * 8;
        let capacity = ((FRAME_CACHE_BYTES / bytes).clamp(1, 8)) as usize;
        while self.cache.len() >= capacity {
            self.cache.pop_front();
        }
        self.pool
            .retain(|s| s.size == size || Arc::strong_count(s) > 1);
        let free = self
            .pool
            .iter()
            .find(|s| s.size == size && Arc::strong_count(s) == 1)
            .cloned();
        Ok(match free {
            Some(s) => s,
            None => {
                anyhow::ensure!(self.pool.len() < 11, "bounded source pool exhausted");
                let s = Arc::new(ExportedSurface::create(
                    &self.gpu.device,
                    &SurfaceRequest {
                        size,
                        format: SurfaceFormat::Rgba16Float,
                    },
                )?);
                self.pool.push(s.clone());
                s
            }
        })
    }
    /// Draws a paused picture's settled frame (`spec`: its refinement at
    /// the composition's size) into the frame cache in bands, as
    /// background fills submit them ([`crate::jobs::Costs`]): `stop` is
    /// asked before each band and every few milliseconds while a band is
    /// on the GPU, so the worker never waits behind a submission once
    /// something newer asked (Metal cannot preempt the submission itself:
    /// what follows on the GPU waits behind at most that one). Where bands
    /// cost about a whole frame (each band builds the frame's plan; a
    /// precomp or an effect chain renders whole for any region), the frame
    /// draws whole, and so do later ones with the same `costs`. The next
    /// [`Engine::frame`] of `spec` presents it from the cache.
    /// `Ok(false)`: stopped; `Ok(true)`: the frame will find it, or draws
    /// it itself (a cache that's off, a stand-in that isn't exact).
    pub fn refine(
        &mut self,
        scene: &Scene,
        spec: FrameSpec,
        costs: &mut crate::jobs::Costs,
        stop: &dyn Fn() -> bool,
    ) -> Result<bool> {
        use pm_export::render::BAND_ROWS;
        use pm_protocol::common::PixelRect;
        self.refine_bands = 0;
        if !self.cache_enabled {
            return Ok(true);
        }
        self.sync_studio(scene)?;
        self.sync_fonts();
        let size = spec.size_in(scene);
        let key = self.cache_key(scene, &spec, size);
        if self.cache.iter().any(|c| c.key == key) {
            return Ok(true);
        }
        let mut request = spec.request(scene)?;
        request.output_tone = self.output_tone;
        let area = request.region.unwrap_or(PixelRect {
            x: 0,
            y: 0,
            width: request.output_size.width,
            height: request.output_size.height,
        });
        let total = area.height.div_ceil(BAND_ROWS).max(1);
        costs.frame_units = costs.frame_units.max(total);
        let whole_at_once = costs.indivisible
            || costs
                .whole_us
                .is_some_and(|us| us <= crate::jobs::BAND_BUDGET_US);
        if whole_at_once {
            return Ok(true);
        }
        let surface = self.cache_surface(size)?;
        // Drawn as `frame` draws this spec: the cached picture stands for it.
        self.renderer
            .set_preview_media(spec.preview_stills(scene), spec.preview_video());
        self.renderer.set_draw_reuse(!spec.exact);
        self.renderer.set_scrub(false);
        self.renderer.set_playback(None);
        self.renderer.set_isolate_layers(true);
        if let Some(store) = &self.compat_document {
            store.begin_frame(Moment::Paused);
        }
        pm_render::media::note_reads();
        let drawn = self.refine_bands(
            scene, spec.time, &request, area, &surface, costs, total, stop,
        );
        let reads = pm_render::media::take_reads();
        self.renderer.set_isolate_layers(false);
        self.release_final_stills(scene, &spec);
        let late = self
            .compat_document
            .as_ref()
            .map(|store| store.end_frame())
            .unwrap_or_default();
        let Some(failures) = drawn? else {
            return Ok(false);
        };
        // As for any frame: a render that met a font failure is not kept,
        // and fonts are installed afresh before the next.
        if let Some(error) = self.fonts.as_ref().and_then(|f| f.after_render()) {
            return Err(FontsPending(error).into());
        }
        // A stand-in is not the exact picture, and bands that read another
        // version of a file than the key's are not the key's: the frame
        // draws it itself.
        if late.is_empty() && self.reads_observed(scene, &reads) {
            self.cache.push_back(CachedFrame {
                key,
                surface,
                failures: failures.into(),
            });
        }
        Ok(true)
    }
    /// A band's own target, kept for the next band of its size.
    fn band_surface(&mut self, width: u32, height: u32) -> Result<Arc<ExportedSurface>> {
        let size = Size::new(width, height);
        if let Some(strip) = self.strip.as_ref().filter(|s| s.size == size) {
            return Ok(strip.clone());
        }
        let strip = Arc::new(ExportedSurface::create(
            &self.gpu.device,
            &SurfaceRequest {
                size,
                format: SurfaceFormat::Rgba16Float,
            },
        )?);
        self.strip = Some(strip.clone());
        Ok(strip)
    }
    /// [`Engine::refine`]'s bands: `None` when `stop` said so.
    #[allow(clippy::too_many_arguments)]
    fn refine_bands(
        &mut self,
        scene: &Scene,
        spec_time: Ticks,
        request: &pm_render::RenderRequest,
        area: pm_protocol::common::PixelRect,
        surface: &ExportedSurface,
        costs: &mut crate::jobs::Costs,
        total: u32,
        stop: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<LayerFailure>>> {
        use pm_export::render::BAND_ROWS;
        let mut failures = Vec::new();
        let mut y = 0;
        while y < area.height {
            if stop() {
                return Ok(None);
            }
            let remaining = (area.height - y).div_ceil(BAND_ROWS);
            let units = costs.bounded(remaining).clamp(1, remaining);
            let rows = (units * BAND_ROWS).min(area.height - y);
            let whole = y == 0 && rows == area.height;
            let mut band = request.clone();
            band.region = Some(pm_protocol::common::PixelRect {
                y: area.y + y,
                height: rows,
                ..area
            });
            let started = std::time::Instant::now();
            let rendered = if whole {
                band.region = request.region;
                self.renderer.render_into(
                    &scene.doc,
                    &scene.comp,
                    spec_time,
                    &band,
                    scene.media.as_ref(),
                    &Target::wrap(surface.texture.clone()),
                    None,
                )?
            } else {
                let strip = self.band_surface(area.width, rows)?;
                let rendered = self.renderer.render_into(
                    &scene.doc,
                    &scene.comp,
                    spec_time,
                    &band,
                    scene.media.as_ref(),
                    &Target::wrap(strip.texture.clone()),
                    None,
                )?;
                let mut encoder =
                    self.gpu
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("refine band"),
                        });
                encoder.copy_texture_to_texture(
                    strip.texture.as_image_copy(),
                    wgpu::TexelCopyTextureInfo {
                        texture: &surface.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d { x: 0, y, z: 0 },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: area.width,
                        height: rows,
                        depth_or_array_layers: 1,
                    },
                );
                self.gpu.queue.submit([encoder.finish()]);
                rendered
            };
            if !self.wait_submitted(stop) {
                return Ok(None);
            }
            if let Some(error) = self.failed.lock().unwrap().take() {
                bail!("The GPU refused this frame: {error}");
            }
            costs.record(
                if whole {
                    total
                } else {
                    rows.div_ceil(BAND_ROWS)
                },
                started.elapsed().as_secs_f64() * 1e6,
            );
            failures.extend(rendered.failures);
            self.refine_bands += 1;
            y += rows;
        }
        Ok(Some(failures))
    }
    /// Waits until the GPU has finished what was submitted, asking `stop`
    /// every few milliseconds: false when it said so first (the submission
    /// runs on; nobody waits for it).
    fn wait_submitted(&mut self, stop: &dyn Fn() -> bool) -> bool {
        let mut waited = 0;
        loop {
            let done = self.gpu.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_millis(4)),
            });
            if !matches!(done, Err(wgpu::PollError::Timeout)) {
                break;
            }
            if stop() {
                return false;
            }
            // As `Renderer::wait_idle` looks: a frame that runs away is
            // seen running.
            waited += 1;
            if waited % 25 == 0 {
                self.renderer.watchdog_watch();
            }
        }
        // Nothing left to wait for: the hang guard judges what ended.
        self.renderer.wait_idle();
        true
    }
    /// The raster override answers a frame at `time` would need and that
    /// are not answered yet, from its plan alone (playback's look-ahead).
    pub fn compat_needs(&mut self, scene: &Scene, time: Ticks, spec: FrameSpec) -> Vec<CompatNeed> {
        if self.compat_document.is_none() || self.sync_studio(scene).is_err() {
            return Vec::new();
        }
        let Ok(request) = spec.request(scene) else {
            return Vec::new();
        };
        self.renderer
            .compat_missing(
                &scene.doc,
                &scene.comp,
                time,
                &request,
                scene.media.as_ref(),
            )
            .unwrap_or_default()
    }
}
/// Delays every completion handler (tests: Metal's completion queue running
/// late under load).
#[cfg(test)]
/// Every viewer request's (exact, moment, scale, resampler), for tests.
#[cfg(test)]
pub(crate) static SAMPLED: std::sync::Mutex<
    Vec<(bool, Moment, f64, pm_render::sampling::Sampling)>,
> = std::sync::Mutex::new(Vec::new());
/// Every viewer request's (exact, moment, media files), for tests.
#[cfg(test)]
pub(crate) static MEDIA_SAMPLED: std::sync::Mutex<
    Vec<(bool, Moment, Arc<pm_export::render::MediaPaths>)>,
> = std::sync::Mutex::new(Vec::new());
#[cfg(test)]
pub(crate) static LATE_COMPLETION_MS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

impl Submission {
    pub fn pixels(&self) -> Vec<[f64; 3]> {
        let b = self.readback.as_ref().unwrap();
        let values = unsafe {
            std::slice::from_raw_parts(
                b.contents().cast::<u16>(),
                self.width as usize * self.height as usize * 4,
            )
        };
        values
            .chunks_exact(4)
            .map(|p| [p[0], p[1], p[2]].map(|v| half::f16::from_bits(v).to_f64()))
            .collect()
    }
    pub fn commit(&self) {
        self.submit(None);
    }
    pub fn submit(&self, due: Option<f64>) {
        // Metal's present convenience methods may create autoreleased helpers.
        // Drain these every frame as well as the encoding pool above.
        self.submit_notifying(due, None);
    }
    pub fn submit_notifying(&self, due: Option<f64>, wake: Option<Arc<dyn Fn() + Send + Sync>>) {
        objc2::rc::autoreleasepool(|_| self.submit_inner(due, wake));
    }
    fn submit_inner(&self, due: Option<f64>, wake: Option<Arc<dyn Fn() + Send + Sync>>) {
        if let Some(drawable) = &self.drawable {
            self.trace.lock().unwrap().present_requested = now();
            let t = self.trace.clone();
            let displayed = wake.clone();
            drawable.add_presented_handler(
                &block::ConcreteBlock::new(move |d: &metal::DrawableRef| {
                    let mut t = t.lock().unwrap();
                    t.display = Some(d.presented_time());
                    t.display_callback = now();
                    drop(t);
                    if let Some(wake) = &displayed {
                        wake();
                    }
                })
                .copy(),
            );
            if let Some(due) = due {
                unsafe {
                    use objc2::{msg_send, runtime::AnyObject};
                    let _: () = msg_send![self.command.as_ptr() as *mut AnyObject,
                        presentDrawable:drawable.as_ptr() as *mut AnyObject,atTime:due];
                }
            } else {
                self.command.present_drawable(drawable);
            }
        }
        let t = self.trace.clone();
        self.command.add_scheduled_handler(
            &block::ConcreteBlock::new(move |_: &metal::CommandBufferRef| {
                t.lock().unwrap().scheduled = now();
            })
            .copy(),
        );
        let t = self.trace.clone();
        let lease = self.surface.clone();
        self.command.add_completed_handler(
            &block::ConcreteBlock::new(move |b: &metal::CommandBufferRef| {
                let _keep_until_complete = &lease;
                #[cfg(test)]
                std::thread::sleep(std::time::Duration::from_millis(
                    LATE_COMPLETION_MS.load(std::sync::atomic::Ordering::Relaxed),
                ));
                let mut t = t.lock().unwrap();
                t.gpu_start = gpu_time(b, true);
                t.gpu_done = gpu_time(b, false);
                t.completed = now();
                t.gpu_ok = b.status() == metal::MTLCommandBufferStatus::Completed;
                drop(t);
                if let Some(wake) = &wake {
                    wake();
                }
            })
            .copy(),
        );
        self.trace.lock().unwrap().commit = now();
        self.command.commit();
    }
    pub fn complete(&self) -> Result<()> {
        self.command.wait_until_completed();
        if self.command.status() != metal::MTLCommandBufferStatus::Completed {
            bail!("Metal consumer failed: {:?}", self.command.status());
        }
        Ok(())
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        self.gpu.wait_idle();
    }
}

fn gpu_time(b: &metal::CommandBufferRef, start: bool) -> f64 {
    unsafe {
        use objc2::{msg_send, runtime::AnyObject};
        let p = b.as_ptr() as *mut AnyObject;
        if start {
            msg_send![p, GPUStartTime]
        } else {
            msg_send![p, GPUEndTime]
        }
    }
}

#[cfg(test)]
mod sampling_tests {
    use super::*;
    use pm_render::sampling::Sampling;

    #[test]
    fn only_reduced_drag_frames_take_the_playback_resampler() {
        let output = Output {
            width: 64,
            height: 36,
            display: 0,
            headroom: 1.,
            crop: None,
        };
        let spec = |moment, scale| FrameSpec {
            moment,
            scale,
            exact: false,
            ..FrameSpec::exact(Ticks::new(0).unwrap(), output)
        };
        assert_eq!(spec(Moment::Playing, 1.).sampling(), Sampling::Final);
        assert_eq!(spec(Moment::Playing, 0.5).sampling(), Sampling::Final);
        assert_eq!(spec(Moment::Scrub, 0.5).sampling(), Sampling::Playback);
        assert_eq!(spec(Moment::Scrub, 1.).sampling(), Sampling::Final);
        assert_eq!(spec(Moment::Paused, 0.5).sampling(), Sampling::Final);
        assert_eq!(spec(Moment::Paused, 1.).sampling(), Sampling::Final);
        // A capture during a reduced drag reads the final picture.
        let capture = FrameSpec {
            exact: true,
            ..spec(Moment::Scrub, 0.5)
        };
        assert_eq!(capture.sampling(), Sampling::Final);
    }
}

#[cfg(test)]
mod lifetime_tests {
    use super::*;
    /// A font installed or removed is the generation the disk render
    /// cache observes at once, before any frame renders with it.
    #[test]
    fn the_font_generation_follows_the_broker_before_a_frame() {
        let mut engine = Engine::new(Gpu::new("font-generation-test").unwrap()).unwrap();
        let broker = pm_text::host::FontBroker::new();
        broker.set_generation(1);
        engine.set_fonts(pm_render::text::host::RenderFonts::new(broker.clone()));
        assert_eq!(engine.font_generation(), Some(1));
        broker.set_generation(2);
        assert_eq!(engine.font_generation(), Some(2));
    }
    /// A refinement whose render met a font failure is not cached: it
    /// fails as a frame does (transient, fonts installed afresh) and the
    /// next one is kept.
    #[test]
    fn a_refinement_that_met_a_font_failure_is_not_cached() {
        let mut engine = Engine::new(Gpu::new("refine-fonts-test").unwrap()).unwrap();
        let broker = pm_text::host::FontBroker::new();
        engine.set_fonts(pm_render::text::host::RenderFonts::new(broker));
        let scene = Scene {
            doc: Arc::new(pm_model::Document::new(
                Some(pm_model::canonical::canonical_project(
                    pm_model::json::parse(include_str!("../tests/fixtures/shapes.json")).unwrap(),
                )),
                1,
            )),
            comp: "Pshapes-primitives-root".into(),
            media: Default::default(),
            content: 1,
        };
        let spec = FrameSpec {
            scale: 4.,
            ..FrameSpec::exact(
                Ticks::from_whole_seconds(1),
                Output {
                    width: 160,
                    height: 90,
                    display: 0,
                    headroom: 1.,
                    crop: None,
                },
            )
        };
        engine
            .fonts
            .as_ref()
            .unwrap()
            .health
            .fail("the font host went away".into());
        let mut costs = crate::jobs::Costs::default();
        let error = engine
            .refine(&scene, spec, &mut costs, &|| false)
            .unwrap_err();
        assert!(transient(&error), "{error}");
        assert!(engine.cache.is_empty());
        let mut costs = crate::jobs::Costs::default();
        assert!(engine.refine(&scene, spec, &mut costs, &|| false).unwrap());
        assert_eq!(engine.cache.len(), 1);
    }
    #[test]
    fn delayed_metal_completion_retains_the_actual_source_texture() {
        let mut engine = Engine::new(Gpu::new("completion-lease-test").unwrap()).unwrap();
        let doc = pm_model::Document::new(
            Some(pm_model::canonical::canonical_project(
                pm_model::json::parse(include_str!("../tests/fixtures/shapes.json")).unwrap(),
            )),
            1,
        );
        let scene = Scene {
            doc: Arc::new(doc),
            comp: "C001".into(),
            media: Default::default(),
            content: 1,
        };
        // Use the fixture's real composition id, independent of its display name.
        let value = scene.doc.to_value().unwrap();
        let scene = Scene {
            comp: value
                .as_object()
                .unwrap()
                .get("compId")
                .unwrap()
                .as_str()
                .unwrap()
                .into(),
            ..scene
        };
        let spec = FrameSpec::exact(
            Ticks::ZERO,
            Output {
                width: 160,
                height: 90,
                display: 0,
                headroom: 1.,
                crop: None,
            },
        );
        // Another test may hold both compiler slots: the worker retries that.
        let s = loop {
            match engine.frame(
                &scene,
                spec,
                None,
                &Default::default(),
                false,
                Arc::default(),
            ) {
                Err(e) if transient(&e) => std::thread::sleep(std::time::Duration::from_millis(5)),
                other => break other.unwrap().unwrap(),
            }
        };
        let event = engine.metal.new_shared_event();
        s.command.encode_wait_for_event(&event, 1);
        let weak = Arc::downgrade(&s.surface);
        s.commit();
        let command = s.command.clone();
        drop(s);
        engine.clear_cache();
        engine.pool.clear();
        assert!(
            weak.upgrade().is_some(),
            "GPU source released before completion"
        );
        assert_ne!(command.status(), metal::MTLCommandBufferStatus::Completed);
        event.set_signaled_value(1);
        command.wait_until_completed();
        assert_eq!(command.status(), metal::MTLCommandBufferStatus::Completed);
    }
    /// A refinement drawn in bands is the frame drawn whole, pixel for
    /// pixel; a stop between bands leaves nothing in the cache.
    #[test]
    fn a_banded_refinement_is_the_whole_frame_and_gives_way_between_bands() {
        let mut engine = Engine::new(Gpu::new("refine-bands-test").unwrap()).unwrap();
        let doc = pm_model::Document::new(
            Some(pm_model::canonical::canonical_project(
                pm_model::json::parse(include_str!("../tests/fixtures/shapes.json")).unwrap(),
            )),
            1,
        );
        let scene = Scene {
            doc: Arc::new(doc),
            comp: "Pshapes-primitives-root".into(),
            media: Default::default(),
            content: 1,
        };
        let output = Output {
            width: 160,
            height: 90,
            display: 0,
            headroom: 1.,
            crop: None,
        };
        // Refinement refines the paused viewer's picture; exact reads
        // (captures) never use the frame cache.
        let spec = FrameSpec {
            scale: 4.,
            exact: false,
            ..FrameSpec::exact(Ticks::from_whole_seconds(1), output)
        };
        let cached = |engine: &Engine| {
            let surface = engine.cache.back().unwrap().surface.clone();
            pm_gpu::readback::read_f32(&engine.gpu.device, &engine.gpu.queue, &surface.texture)
                .unwrap()
        };
        // Whole, as a frame draws it.
        let trace = Arc::new(std::sync::Mutex::new(Trace::default()));
        while engine
            .frame(
                &scene,
                spec,
                None,
                &Default::default(),
                false,
                trace.clone(),
            )
            .unwrap()
            .is_none()
        {}
        let whole = cached(&engine);
        engine.cache.clear();
        // Stopped after its first band: nothing is kept.
        let mut costs = crate::jobs::Costs::default();
        assert!(!engine.refine(&scene, spec, &mut costs, &|| true).unwrap());
        assert!(engine.cache.is_empty());
        // In bands: the same pixels.
        let mut costs = crate::jobs::Costs::default();
        assert!(engine.refine(&scene, spec, &mut costs, &|| false).unwrap());
        assert_eq!(engine.cache.len(), 1);
        let banded = cached(&engine);
        assert_eq!(banded.len(), 640 * 360 * 4);
        let worst = whole
            .iter()
            .zip(&banded)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(worst < 1e-3, "banded differs by {worst}");
    }

    /// The frame cache holds viewer pictures only. A paused viewer frame at
    /// a scale is kept and served again at that scale; an exact read of the
    /// same time and scale (a capture, an export frame) is drawn from the
    /// originals every time, is never kept, and never takes or displaces
    /// the viewer picture that is.
    #[test]
    fn an_exact_frame_at_a_scale_is_drawn_each_time_beside_the_cached_viewer_frame() {
        let mut engine = Engine::new(Gpu::new("exact-scale-cache-test").unwrap()).unwrap();
        let doc = pm_model::Document::new(
            Some(pm_model::canonical::canonical_project(
                pm_model::json::parse(include_str!("../tests/fixtures/shapes.json")).unwrap(),
            )),
            1,
        );
        let scene = Scene {
            doc: Arc::new(doc),
            comp: "Pshapes-primitives-root".into(),
            media: Default::default(),
            content: 1,
        };
        let output = Output {
            width: 160,
            height: 90,
            display: 0,
            headroom: 1.,
            crop: None,
        };
        let exact = FrameSpec {
            scale: 4.,
            ..FrameSpec::exact(Ticks::from_whole_seconds(1), output)
        };
        let viewer = FrameSpec {
            exact: false,
            ..exact
        };
        // Draws `spec` and says whether the frame cache served it.
        let draw = |engine: &mut Engine, spec: FrameSpec| {
            let trace = Arc::new(std::sync::Mutex::new(Trace::default()));
            while engine
                .frame(
                    &scene,
                    spec,
                    None,
                    &Default::default(),
                    false,
                    trace.clone(),
                )
                .unwrap()
                .is_none()
            {}
            trace.lock().unwrap().cache_hit
        };
        assert!(
            !draw(&mut engine, viewer),
            "the first viewer frame is drawn"
        );
        assert_eq!(engine.cache.len(), 1, "and kept at its scale");
        assert!(draw(&mut engine, viewer), "the same viewer frame is served");
        for read in 1..=2 {
            assert!(
                !draw(&mut engine, exact),
                "exact read {read} took a cached picture"
            );
            assert_eq!(engine.cache.len(), 1, "exact read {read} was kept");
        }
        assert!(
            draw(&mut engine, viewer),
            "the viewer frame is still served after the exact reads"
        );
    }

    #[test]
    fn only_a_traced_frame_counts_its_work() {
        let mut engine = Engine::new(Gpu::new("work-count-test").unwrap()).unwrap();
        let doc = pm_model::Document::new(
            Some(pm_model::canonical::canonical_project(
                pm_model::json::parse(include_str!("../tests/fixtures/shapes.json")).unwrap(),
            )),
            1,
        );
        let comp = doc
            .to_value()
            .unwrap()
            .as_object()
            .unwrap()
            .get("compId")
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned();
        let scene = Scene {
            doc: Arc::new(doc),
            comp,
            media: Default::default(),
            content: 1,
        };
        let output = Output {
            width: 160,
            height: 90,
            display: 0,
            headroom: 1.,
            crop: None,
        };
        let mut draw = |at: i32, count_work: bool| {
            let trace = Arc::new(std::sync::Mutex::new(Trace {
                count_work,
                ..Default::default()
            }));
            let spec = FrameSpec::exact(Ticks::from_whole_seconds(at), output);
            loop {
                match engine.frame(
                    &scene,
                    spec,
                    None,
                    &Default::default(),
                    false,
                    trace.clone(),
                ) {
                    Err(e) if transient(&e) => {
                        std::thread::sleep(std::time::Duration::from_millis(5))
                    }
                    other => {
                        other.unwrap().unwrap().commit();
                        break;
                    }
                }
            }
            trace.lock().unwrap().work
        };
        assert_eq!(draw(0, false), pm_perf::Counters::default());
        assert!(draw(1, true).get(pm_perf::Counter::LayersPlanned) > 0);
    }
    #[test]
    fn a_contributed_effect_changes_rendered_pixels_through_its_snapshot() {
        use pm_protocol::definitions::{
            DefinitionFlags, DefinitionKind, DefinitionOrigin, DefinitionSnapshot, ParamSpec,
            WrappedGlsl,
        };
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/shapes.json")).unwrap();
        value["layers"][0]["fx"] = serde_json::json!([{
            "id":"fx1","type":"synthetic.black","version":1,"on":true,
            "p":{"amount":{"v":1,"kf":[],"expr":null}}
        }]);
        let body = "o = vec4(0.0, 0.0, 0.0, src(v_uv).a * u_amount);";
        let source = format!(
            "#version 300 es\nprecision highp float;\nin vec2 v_uv;\nuniform sampler2D u_tex;\nout vec4 o;\nvec4 src(vec2 uv){{return texture(u_tex,uv);}}\nuniform float u_amount;\nvoid main(){{\n{body}\n}}\n"
        );
        let definition = DefinitionSnapshot {
            id: "synthetic.black".into(),
            kind: DefinitionKind::Effect,
            version: 0,
            origin: DefinitionOrigin::Local,
            renderer: None,
            glsl: Some(WrappedGlsl {
                body_line_offset: 8,
                body_utf16_len: body.len() as u32,
                body_utf8_len: body.len() as u32,
                source,
            }),
            params: vec![ParamSpec::Number {
                key: "amount".into(),
                default: 1.,
                min: 0.,
                max: 1.,
                step: None,
            }],
            flags: DefinitionFlags::default(),
        };
        let size = Size::new(160, 90);
        // Contributions reach the engine as the viewer's pinned resource snapshot.
        let draw = |gpu: &str, value: &serde_json::Value, definitions: Vec<DefinitionSnapshot>| {
            let parsed = pm_model::json::parse(&value.to_string()).unwrap();
            let comp = parsed.get("compId").unwrap().as_str().unwrap().to_owned();
            let scene = Scene {
                doc: Arc::new(pm_model::Document::new(
                    Some(pm_model::canonical::canonical_project(parsed)),
                    1,
                )),
                comp,
                media: Default::default(),
                content: 1,
            };
            let mut engine = Engine::new(Gpu::new(gpu).unwrap()).unwrap();
            engine.set_resources(Arc::new(crate::resources::ResourceSnapshot {
                definitions,
                ..Default::default()
            }));
            let pixels = engine.export(&scene, Ticks::ZERO, size).unwrap();
            (pixels, engine.definition_failure.take())
        };
        let mut plain: serde_json::Value = value.clone();
        plain["layers"][0]["fx"] = serde_json::json!([]);
        let (before, _) = draw("contributed-effect", &plain, vec![]);
        let (after, _) = draw("contributed-effect-after", &value, vec![definition.clone()]);
        assert_ne!(before, after, "the contributed program drew");
        // A broken contribution fails alone: unused it costs nothing, and the
        // valid one beside it still draws.
        let mut broken = definition.clone();
        broken.id = "synthetic.broken".into();
        broken.glsl.as_mut().unwrap().source = "not glsl".into();
        let mut invalid = definition.clone();
        invalid.id = "synthetic.invalid".into();
        invalid.params[0] = ParamSpec::Number {
            key: "Not A Key".into(),
            default: 1.,
            min: 2.,
            max: 1.,
            step: None,
        };
        let (unused, failure) = draw(
            "contributed-effect-broken",
            &plain,
            vec![broken.clone(), invalid.clone()],
        );
        assert_eq!(before, unused, "unused bad definitions change nothing");
        assert!(failure.is_some());
        let (beside, _) = draw(
            "contributed-effect-beside",
            &value,
            vec![broken, invalid, definition],
        );
        assert_eq!(
            after, beside,
            "a good definition is untouched beside a bad one"
        );
    }
    #[test]
    fn encoded_hdr_outputs_match_independent_f64_reference() {
        use crate::color::{self, Mastering};
        use pm_color::tonemap::{hlg_oetf, pq_eotf, pq_inverse_eotf};
        let mut engine = Engine::new(Gpu::new("mastering-reference").unwrap()).unwrap();
        let value = pm_model::json::parse(include_str!("../tests/fixtures/shapes.json")).unwrap();
        let comp = value.get("compId").unwrap().as_str().unwrap().to_owned();
        let scene = Scene {
            doc: Arc::new(pm_model::Document::new(
                Some(pm_model::canonical::canonical_project(value)),
                1,
            )),
            comp,
            media: Default::default(),
            content: 1,
        };
        let size = Size::new(160, 90);
        let reference = loop {
            match engine.export(&scene, Ticks::ZERO, size) {
                Err(e) if transient(&e) => std::thread::sleep(std::time::Duration::from_millis(5)),
                other => break other.unwrap(),
            }
        };
        for mode in [
            Mastering::Pq {
                source_peak: 1000.,
                target_peak: 203.,
            },
            Mastering::Pq {
                source_peak: 1000.,
                target_peak: 1000.,
            },
            Mastering::Hlg {
                source_peak: 1000.,
                target_peak: 1000.,
            },
        ] {
            engine.mastering = mode;
            let sub = engine
                .frame(
                    &scene,
                    FrameSpec::exact(
                        Ticks::ZERO,
                        Output {
                            width: 160,
                            height: 90,
                            display: 0,
                            headroom: 1.,
                            crop: None,
                        },
                    ),
                    None,
                    &Default::default(),
                    true,
                    Arc::default(),
                )
                .unwrap()
                .unwrap();
            sub.commit();
            sub.complete().unwrap();
            let u = mode.params().unwrap();
            let mut worst = 0_f64;
            for (actual, encoded) in sub.pixels().iter().zip(reference.chunks_exact(4)) {
                let a = encoded[3] as f64;
                if a < 0.999 {
                    continue;
                }
                let p = color::p3(encoded);
                let mut c = [
                    0.753833 * p[0] + 0.198597 * p[1] + 0.047570 * p[2],
                    0.045744 * p[0] + 0.941777 * p[1] + 0.012479 * p[2],
                    -0.001210 * p[0] + 0.017602 * p[1] + 0.983609 * p[2],
                ]
                .map(|v| v * 203.);
                if u[2] < u[1] {
                    c = c
                        .map(|n| pq_eotf(pm_color::tonemap::eetf(pq_inverse_eotf(n), u[1] as f64)));
                }
                let expected = if u[0] == 1. {
                    c.map(pq_inverse_eotf)
                } else {
                    let c = c.map(|v| (v / 1000.).max(0.));
                    let y = 0.2627 * c[0] + 0.6780 * c[1] + 0.0593 * c[2];
                    c.map(|v| {
                        hlg_oetf(if y > 0. {
                            v * y.powf(1. / 1.2 - 1.)
                        } else {
                            0.
                        })
                    })
                };
                for (a, b) in actual.iter().zip(expected) {
                    worst = worst.max((a - b).abs());
                }
            }
            assert!(worst < 0.002, "{mode:?}: max signal error {worst}");
        }
    }
}

#[cfg(test)]
mod paused_media_tests {
    use super::*;
    /// A 64 × 64 composition showing 480 px of 3-px stripes at 6.25 %.
    fn stripes() -> Scene {
        let clip = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../../../engine/crates/pm-render/tests/fixtures/video/stripes-709-limited.mp4",
        );
        let p = |k: &str, v: f64| format!(r#""{k}":{{"v":{v},"kf":[],"expr":null}}"#);
        let transform = [
            p("position.x", 32.),
            p("position.y", 32.),
            p("anchor.x", 0.),
            p("anchor.y", 0.),
            p("scale.x", 6.25),
            p("scale.y", 6.25),
            p("rotation", 0.),
            p("opacity", 100.),
        ]
        .join(",");
        let project = format!(
            r##"{{"id":"P","name":"P","compId":"C","compName":"C","w":64,"h":64,"fps":30,"dur":1,"bg":"#000000",
            "layers":[{{"id":"v","type":"video","name":"clip","from":0,"dur":1,"on":true,"solo":false,"mblur":false,
              "p":{{{transform}}},"fx":[],"masks":[],"d":{{"asset":"clip","trim":0,"speed":1,"w":480,"h":480}}}}],
            "comps":{{}},"assets":{{"clip":{{"id":"clip","type":"video","name":"clip","w":480,"h":480,"dur":1}}}},
            "markers":[],"work":[0,1],"params":{{}}}}"##
        );
        Scene {
            doc: Arc::new(pm_model::Document::new(
                Some(pm_model::canonical::canonical_project(
                    pm_model::json::parse(&project).unwrap(),
                )),
                1,
            )),
            comp: "C".into(),
            media: Arc::new(pm_export::render::MediaPaths {
                paths: [("clip".to_string(), clip)].into(),
                ..Default::default()
            }),
            content: 1,
        }
    }
    fn drawn(engine: &mut Engine, scene: &Scene, spec: FrameSpec) -> Vec<[f64; 3]> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let sub = engine
                .frame(scene, spec, None, &Default::default(), true, Arc::default())
                .unwrap()
                .unwrap();
            sub.commit();
            sub.complete().unwrap();
            assert!(sub.failures.is_empty(), "{:?}", sub.failures);
            if !sub.stand_ins {
                return sub.pixels();
            }
            assert!(Instant::now() < deadline, "the clip's frame never landed");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    /// A paused frame shows a large clip drawn small exactly as an export
    /// (a capture) draws it: the source read through the export's path, not
    /// a preview-sized copy.
    #[test]
    fn a_paused_frame_reads_video_as_the_export_does() {
        let mut engine = Engine::new(Gpu::new("paused-media-test").unwrap()).unwrap();
        let scene = stripes();
        let output = Output {
            width: 64,
            height: 64,
            display: 0,
            headroom: 1.,
            crop: None,
        };
        let exact = drawn(&mut engine, &scene, FrameSpec::exact(Ticks::ZERO, output));
        let paused = FrameSpec {
            exact: false,
            ..FrameSpec::exact(Ticks::ZERO, output)
        };
        assert!(!paused.preview_video());
        let shown = drawn(&mut engine, &scene, paused);
        // The clip is drawn (grey stripes averaged, not the black ground).
        assert!(shown[32 * 64 + 32][0] > 0.2, "{:?}", shown[32 * 64 + 32]);
        assert_eq!(shown, exact);
        // Playing frames may read the smaller copy.
        assert!(
            FrameSpec {
                moment: Moment::Playing,
                ..paused
            }
            .preview_video()
        );
    }
}

#[cfg(test)]
mod viewer_output_tests {
    use super::*;
    use crate::color::Mastering;
    /// `n` white solids `w` px wide, every other `w` px from x = 0, one stop
    /// over white (2.0 encoded), on black, in a 64 × 36 composition.
    fn columns(n: usize, w: f64) -> Scene {
        let ch = |v: f64| format!(r#"{{"v":{v},"kf":[],"expr":null}}"#);
        let layers = (0..n)
            .map(|i| {
                // The anchor (0, 0) is the solid's top-left corner.
                let x = 2. * w * i as f64;
                let transform = [
                    ("position.x", x),
                    ("position.y", 0.),
                    ("anchor.x", 0.),
                    ("anchor.y", 0.),
                    ("scale.x", 100.),
                    ("scale.y", 100.),
                    ("rotation", 0.),
                    ("opacity", 100.),
                ]
                .map(|(k, v)| format!(r#""{k}":{}"#, ch(v)))
                .join(",");
                format!(
                    r##"{{"id":"s{i}","type":"solid","name":"white","from":0,"dur":1,"on":true,"solo":false,"mblur":false,
                    "p":{{{transform}}},"masks":[],"d":{{"color":"#FFFFFF","w":{w},"h":36}},
                    "fx":[{{"id":"e","type":"color","on":true,"p":{{"exposure":{}}}}}]}}"##,
                    ch(1.)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let project = format!(
            r##"{{"id":"P","name":"P","compId":"C","compName":"C","w":64,"h":36,"fps":30,"dur":1,"bg":"#000000",
            "layers":[{layers}],"comps":{{}},"assets":{{}},"markers":[],"work":[0,1],"params":{{}}}}"##
        );
        Scene {
            doc: Arc::new(pm_model::Document::new(
                Some(pm_model::canonical::canonical_project(
                    pm_model::json::parse(&project).unwrap(),
                )),
                1,
            )),
            comp: "C".into(),
            media: Default::default(),
            content: 1,
        }
    }
    fn halves(b: &metal::Buffer, len: usize) -> Vec<[f64; 3]> {
        let values = unsafe { std::slice::from_raw_parts(b.contents().cast::<u16>(), len * 4) };
        values
            .chunks_exact(4)
            .map(|p| [p[0], p[1], p[2]].map(|v| half::f16::from_bits(v).to_f64()))
            .collect()
    }
    /// (capture readback, presented picture) at the middle of the drawable.
    fn draw(
        engine: &mut Engine,
        scene: &Scene,
        spec: FrameSpec,
        mastering: Mastering,
    ) -> ([f64; 3], Option<[f64; 3]>) {
        engine.mastering = mastering;
        let sub = engine
            .frame(scene, spec, None, &Default::default(), true, Arc::default())
            .unwrap()
            .unwrap();
        sub.commit();
        sub.complete().unwrap();
        assert!(sub.failures.is_empty(), "{:?}", sub.failures);
        let (w, h) = (sub.width as usize, sub.height as usize);
        let at = (h / 2) * w + w / 2;
        (
            sub.pixels()[at],
            sub.presented.as_ref().map(|b| halves(b, w * h)[at]),
        )
    }
    fn output(width: u32, height: u32) -> Output {
        Output {
            width,
            height,
            display: 0,
            headroom: 1.,
            crop: None,
        }
    }
    /// An SDR project's view shows its delivery: a white solid pushed to
    /// twice white (one stop of exposure) shows as white, as the SDR file
    /// holds it; the EDR view keeps the light above white. A capture is the
    /// engine's frame, not the display's picture: it keeps the light.
    #[test]
    fn an_sdr_view_shows_light_above_white_as_the_sdr_delivery_does() {
        let mut engine = Engine::new(Gpu::new("sdr-view-test").unwrap()).unwrap();
        let scene = columns(1, 64.);
        let spec = FrameSpec::exact(Ticks::ZERO, output(64, 36));
        let (edr, _) = draw(&mut engine, &scene, spec, Mastering::Edr);
        assert!(
            edr.iter().all(|c| *c > 1.5),
            "EDR keeps light above white: {edr:?}"
        );
        let (captured, shown) = draw(&mut engine, &scene, spec, Mastering::LinearP3);
        let shown = shown.expect("the presented picture");
        assert!(
            shown.iter().all(|c| (*c - 1.).abs() < 0.002),
            "the SDR view shows white: {shown:?}"
        );
        assert!(
            captured.iter().zip(edr).all(|(c, e)| (*c - e).abs() < 0.01),
            "a capture is the engine's frame: {captured:?} vs {edr:?}"
        );
    }
    /// The SDR view shows a colour past white as every SDR file delivers it
    /// (`pm_color::delivery`, channel by channel in Rec.709): two (0.8, 0.25,
    /// 0.1) solids added make (1.6, 0.5, 0.2), shown as (1, 0.5, 0.2).
    #[test]
    fn an_sdr_view_shows_out_of_range_colour_by_the_delivery_rule() {
        let ch = |v: f64| format!(r#"{{"v":{v},"kf":[],"expr":null}}"#);
        let transform = [
            ("position.x", 0.),
            ("position.y", 0.),
            ("anchor.x", 0.),
            ("anchor.y", 0.),
            ("scale.x", 100.),
            ("scale.y", 100.),
            ("rotation", 0.),
            ("opacity", 100.),
        ]
        .map(|(k, v)| format!(r#""{k}":{}"#, ch(v)))
        .join(",");
        let solid = |id: &str, blend: &str| {
            format!(
                r##"{{"id":"{id}","type":"solid","name":"{id}","from":0,"dur":1,"on":true,"solo":false,"mblur":false,"blend":"{blend}",
                "p":{{{transform}}},"fx":[],"masks":[],"d":{{"color":"#CC401A","w":64,"h":36}}}}"##
            )
        };
        let project = format!(
            r##"{{"id":"P","name":"P","compId":"C","compName":"C","w":64,"h":36,"fps":30,"dur":1,"bg":"#000000",
            "layers":[{},{}],"comps":{{}},"assets":{{}},"markers":[],"work":[0,1],"params":{{}}}}"##,
            solid("a", "add"),
            solid("b", "normal")
        );
        let scene = Scene {
            doc: Arc::new(pm_model::Document::new(
                Some(pm_model::canonical::canonical_project(
                    pm_model::json::parse(&project).unwrap(),
                )),
                1,
            )),
            comp: "C".into(),
            media: Default::default(),
            content: 1,
        };
        let mut engine = Engine::new(Gpu::new("sdr-colour-test").unwrap()).unwrap();
        let spec = FrameSpec::exact(Ticks::ZERO, output(64, 36));
        let (_, shown) = draw(&mut engine, &scene, spec, Mastering::LinearP3);
        let shown = shown.expect("the presented picture");
        let encoded = [0xCC, 0x40, 0x1A].map(|v: u8| f64::from(v) / 255.0 * 2.0);
        let dec = |v: f64| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let l = pm_color::delivery::sdr(encoded).map(dec);
        let want = [
            0.8224621 * l[0] + 0.1775380 * l[1],
            0.0331941 * l[0] + 0.9668058 * l[1],
            0.0170827 * l[0] + 0.0723974 * l[1] + 0.9105199 * l[2],
        ];
        assert!(
            shown.iter().zip(want).all(|(s, w)| (s - w).abs() < 0.004),
            "{shown:?} vs the delivered {want:?}"
        );
    }
    /// The SDR view clips each engine sample before it reduces the frame to
    /// the drawable: 1-px columns of twice white and black, drawn at the
    /// composition's size and shown at half size, average to the delivered
    /// grey (encoded 0.5), not to clipped white.
    #[test]
    fn an_sdr_view_clips_before_it_reduces() {
        let mut engine = Engine::new(Gpu::new("sdr-reduce-test").unwrap()).unwrap();
        let scene = columns(32, 1.);
        let spec = FrameSpec {
            scale: 2.,
            ..FrameSpec::exact(Ticks::ZERO, output(32, 18))
        };
        let (_, shown) = draw(&mut engine, &scene, spec, Mastering::LinearP3);
        let shown = shown.expect("the presented picture");
        // Encoded 0.5 is 0.214 linear (sRGB); white would be 1.0.
        assert!(
            shown.iter().all(|c| (*c - 0.214).abs() < 0.03),
            "half white, half black: {shown:?}"
        );
    }
}

#[cfg(test)]
mod frame_cache_tests {
    use super::*;
    /// A 256 × 256 still of 1-px black and white stripes (a 24-bit BMP).
    fn stripes(dir: &std::path::Path) -> std::path::PathBuf {
        let (w, h) = (256u32, 256u32);
        let row = w * 3;
        let size = 54 + row * h;
        let mut b = Vec::with_capacity(size as usize);
        b.extend_from_slice(b"BM");
        b.extend_from_slice(&size.to_le_bytes());
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&54u32.to_le_bytes());
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(&(w as i32).to_le_bytes());
        b.extend_from_slice(&(h as i32).to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&24u16.to_le_bytes());
        b.extend_from_slice(&[0; 24]);
        for _ in 0..h {
            for x in 0..w {
                let v = if x % 2 == 0 { 255 } else { 0 };
                b.extend_from_slice(&[v, v, v]);
            }
        }
        let path = dir.join("stripes.bmp");
        std::fs::write(&path, b).unwrap();
        path
    }
    fn scene(path: &std::path::Path) -> Scene {
        let ch = |v: f64| format!(r#"{{"v":{v},"kf":[],"expr":null}}"#);
        let transform = [
            ("position.x", 64.),
            ("position.y", 36.),
            ("anchor.x", 0.),
            ("anchor.y", 0.),
            ("scale.x", 23.),
            ("scale.y", 23.),
            ("rotation", 0.),
            ("opacity", 100.),
        ]
        .map(|(k, v)| format!(r#""{k}":{}"#, ch(v)))
        .join(",");
        let project = format!(
            r##"{{"id":"P","name":"P","compId":"C","compName":"C","w":128,"h":72,"fps":30,"dur":1,"bg":"#000000",
            "layers":[{{"id":"i","type":"image","name":"stripes","from":0,"dur":1,"on":true,"solo":false,"mblur":false,
              "p":{{{transform}}},"fx":[],"masks":[],"d":{{"asset":"pic","fit":"stretch","w":256,"h":256}}}}],
            "comps":{{}},"assets":{{"pic":{{"id":"pic","type":"image","name":"stripes","w":256,"h":256,"dur":0}}}},
            "markers":[],"work":[0,1],"params":{{}}}}"##
        );
        Scene {
            doc: Arc::new(pm_model::Document::new(
                Some(pm_model::canonical::canonical_project(
                    pm_model::json::parse(&project).unwrap(),
                )),
                1,
            )),
            comp: "C".into(),
            media: Arc::new(pm_export::render::MediaPaths {
                paths: [("pic".to_string(), path.to_path_buf())].into(),
                ..Default::default()
            }),
            content: 1,
        }
    }
    fn drawn(engine: &mut Engine, scene: &Scene, spec: FrameSpec) -> Vec<[f64; 3]> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let sub = engine
                .frame(scene, spec, None, &Default::default(), true, Arc::default())
                .unwrap()
                .unwrap();
            sub.commit();
            sub.complete().unwrap();
            assert!(sub.failures.is_empty(), "{:?}", sub.failures);
            if !sub.stand_ins {
                return sub.pixels();
            }
            assert!(Instant::now() < deadline, "the still never landed");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    /// Stopping playback never changes the picture at the moment it stops:
    /// a playing frame draws a reduced still with the final resampler and
    /// from the same source as the paused frame shown at once, at the
    /// composition's own drawable size and at half of it (footage and
    /// precomps don't sharpen when playback stops). What the paused viewer
    /// then settles on, once it has rested, is another picture by design:
    /// the final one, drawn from the original still at the composition's
    /// size, the picture a capture reads (and an export delivers).
    #[test]
    fn playing_and_paused_frames_resample_alike() {
        let dir = tempfile::tempdir().unwrap();
        let scene = scene(&stripes(dir.path()));
        for (width, height) in [(128, 72), (64, 36)] {
            let output = Output {
                width,
                height,
                display: 0,
                headroom: 1.,
                crop: None,
            };
            let spec = |moment| FrameSpec {
                moment,
                exact: false,
                ..FrameSpec::exact(Ticks::ZERO, output)
            };
            let mut engine = Engine::new(Gpu::new("playing-resampler").unwrap()).unwrap();
            let paused = drawn(&mut engine, &scene, spec(Moment::Paused));
            let playing = drawn(&mut engine, &scene, spec(Moment::Playing));
            assert_eq!(
                playing, paused,
                "{width}×{height}: the picture changes when playback stops"
            );
            // Drawn as the worker draws it: above the drawable's size, in
            // bands into the frame cache first.
            let settled = FrameSpec {
                scale: FrameSpec::paused_full_scale(&scene, &output),
                settled: true,
                ..spec(Moment::Paused)
            };
            assert!(spec(Moment::Paused).settles(&scene) && settled.final_picture(&scene));
            if settled.scale > 1. {
                let mut costs = crate::jobs::Costs::default();
                assert!(
                    engine
                        .refine(&scene, settled, &mut costs, &|| false)
                        .unwrap()
                );
            }
            let settled_picture = drawn(&mut engine, &scene, settled);
            let capture = drawn(
                &mut engine,
                &scene,
                FrameSpec {
                    exact: true,
                    ..settled
                },
            );
            assert_eq!(
                settled_picture, capture,
                "{width}×{height}: the settled picture is not the one a capture reads"
            );
        }
    }
    /// A rendered frame uploaded ahead of its turn is of one cache state.
    /// When a media file its composition draws from changes on disk, the
    /// document untouched, the cache's next observation retires that state:
    /// the frame already uploaded is not shown either.
    #[test]
    fn an_uploaded_frame_is_dropped_when_its_media_changes() {
        use crate::render_cache::DiskFrames;
        let dir = tempfile::tempdir().unwrap();
        let still = stripes(dir.path());
        let scene = scene(&still);
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), u64::MAX)))
            .unwrap();
        assert!(disk.observe(&scene, &[], None, None, None, 1, Ticks::ZERO));
        disk.test_ready();
        let (first, next) = (disk.test_put(3, 0.25), disk.test_put(4, 0.25));
        let mut engine = Engine::new(Gpu::new("upload-pin").unwrap()).unwrap();
        engine.disk = Some(disk.clone());
        // A playing frame: whether it came from disk (2) or was drawn (1),
        // and its picture.
        let shown = |engine: &mut Engine, time: Ticks| {
            let spec = FrameSpec {
                moment: Moment::Playing,
                exact: false,
                ..FrameSpec::exact(
                    time,
                    Output {
                        width: 128,
                        height: 72,
                        display: 0,
                        headroom: 1.,
                        crop: None,
                    },
                )
            };
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                let trace = Arc::new(std::sync::Mutex::new(Trace::default()));
                match engine.frame(&scene, spec, None, &Default::default(), true, trace.clone()) {
                    Ok(sub) => {
                        let sub = sub.unwrap();
                        sub.commit();
                        sub.complete().unwrap();
                        if !sub.stand_ins {
                            return (trace.lock().unwrap().disk, sub.pixels());
                        }
                    }
                    Err(e) => assert!(e.is::<DiskFramePending>(), "{e:#}"),
                }
                assert!(Instant::now() < deadline, "the frame never settled");
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let (from, rendered) = shown(&mut engine, first);
        assert_eq!(from, 2, "the frame on disk plays");
        let deadline = Instant::now() + Duration::from_secs(20);
        while engine.disk_next.is_none() {
            engine.upload_ahead(&scene, next);
            assert!(Instant::now() < deadline, "the next frame was never read");
            std::thread::sleep(Duration::from_millis(5));
        }
        // The still is saved again by another app.
        let mut bytes = std::fs::read(&still).unwrap();
        for b in &mut bytes[54..] {
            *b = 255 - *b;
        }
        bytes.push(0);
        std::fs::write(&still, bytes).unwrap();
        disk.test_recheck_media();
        assert!(disk.observe(&scene, &[], None, None, None, 1, next));
        disk.test_ready();
        let (from, drawn) = shown(&mut engine, next);
        assert_eq!(from, 1, "the upload is of the still as it was");
        assert_ne!(drawn, rendered);
    }
    /// A paused frame at a reduced quality is drawn as ever although its
    /// frame is rendered: the frame on disk is the final picture, which a
    /// reduced frame is not, and whether a frame is rendered never changes
    /// a paused picture.
    #[test]
    fn a_reduced_paused_frame_is_never_replaced_by_the_frame_on_disk() {
        use crate::render_cache::DiskFrames;
        let dir = tempfile::tempdir().unwrap();
        let scene = scene(&stripes(dir.path()));
        let disk = DiskFrames::default();
        disk.set_storage(Some((dir.path().join("frames"), u64::MAX)))
            .unwrap();
        assert!(disk.observe(&scene, &[], None, None, None, 1, Ticks::ZERO));
        disk.test_ready();
        let at = disk.test_put(3, 0.25);
        let mut engine = Engine::new(Gpu::new("reduced-paused").unwrap()).unwrap();
        engine.disk = Some(disk.clone());
        // Whether a paused frame at `scale` came from disk (2) or was drawn (1).
        let mut from = |scale: f64| {
            let spec = FrameSpec {
                exact: false,
                scale,
                ..FrameSpec::exact(
                    at,
                    Output {
                        width: 128,
                        height: 72,
                        display: 0,
                        headroom: 1.,
                        crop: None,
                    },
                )
            };
            let trace = Arc::new(std::sync::Mutex::new(Trace::default()));
            let sub = engine
                .frame(&scene, spec, None, &Default::default(), true, trace.clone())
                .unwrap()
                .unwrap();
            sub.commit();
            sub.complete().unwrap();
            sub.pixels();
            trace.lock().unwrap().disk
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        while from(1.) != 2 {
            assert!(
                Instant::now() < deadline,
                "the frame on disk was never read"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(from(0.5), 1, "a half-resolution paused frame is drawn");
        assert_eq!(from(1.), 2, "at Full the frame on disk is the picture");
    }
    /// Two dragged frames of one render size at one time but different
    /// densities (output 256 at half resolution, output 128 at full) take
    /// different resamplers: the second is drawn for itself, as a cold
    /// engine draws it, never answered from the first's cached frame.
    #[test]
    fn frames_of_one_size_but_another_density_are_not_shared() {
        let dir = tempfile::tempdir().unwrap();
        let scene = scene(&stripes(dir.path()));
        let output = |w, h| Output {
            width: w,
            height: h,
            display: 0,
            headroom: 1.,
            crop: None,
        };
        let reduced = FrameSpec {
            moment: Moment::Scrub,
            scale: 0.5,
            exact: false,
            ..FrameSpec::exact(Ticks::ZERO, output(256, 144))
        };
        let full = FrameSpec {
            moment: Moment::Scrub,
            scale: 1.,
            exact: false,
            ..FrameSpec::exact(Ticks::ZERO, output(128, 72))
        };
        assert_eq!(reduced.render_size(), full.render_size());
        assert_ne!(reduced.sampling(), full.sampling());
        let mut cold = Engine::new(Gpu::new("cache-identity-cold").unwrap()).unwrap();
        let want = drawn(&mut cold, &scene, full);
        let mut engine = Engine::new(Gpu::new("cache-identity").unwrap()).unwrap();
        drawn(&mut engine, &scene, reduced);
        assert_eq!(drawn(&mut engine, &scene, full), want);
    }
}
