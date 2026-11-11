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
        ratio.clamp(1., 4.)
    }
    fn render_size(&self) -> Size {
        let px = |v: u32| ((f64::from(v) * self.scale).round() as u32).max(1);
        Size::new(px(self.output.width), px(self.output.height))
    }
    pub(crate) fn request(&self, scene: &Scene) -> Result<pm_render::RenderRequest> {
        use pm_protocol::frame::MotionBlurKind;
        let playing = self.moment == Moment::Playing;
        let mut request = pm_render::RenderRequest::sized(self.render_size(), Precision::Float16);
        if let Some(crop) = self.output.crop {
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
        // Today's viewer: 6 motion samples while playing, 12 stopped.
        request.motion_blur = pm_render::motion::MotionBlur::Kind(if playing {
            MotionBlurKind::PreviewPlaying
        } else {
            MotionBlurKind::PreviewPaused
        });
        request.sampling = self.sampling();
        #[cfg(test)]
        SAMPLED
            .lock()
            .unwrap()
            .push((self.exact, self.moment, self.scale, request.sampling));
        Ok(request)
    }

    /// Frames while playing, and a drag's reduced frames (a preview the
    /// final resampler redraws once the playhead settles), take the
    /// playback resampler (pm-render's rule for scrub frames); paused
    /// frames, drags at full resolution (which nothing redraws), and exact
    /// reads such as captures, the final one.
    fn sampling(&self) -> pm_render::sampling::Sampling {
        if !self.exact
            && (self.moment == Moment::Playing || (self.moment == Moment::Scrub && self.scale < 1.))
        {
            pm_render::sampling::Sampling::Playback
        } else {
            pm_render::sampling::Sampling::Final
        }
    }
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
pub(crate) fn transient(error: &anyhow::Error) -> bool {
    error.is::<FontsPending>()
        || error.is::<CompilerBusy>()
        || matches!(
            error.downcast_ref::<RenderError>(),
            Some(
                RenderError::WatchdogPending
                    | RenderError::CompatPending(_)
                    | RenderError::ShaderFallbackPending(_)
            )
        )
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
    pub(crate) fonts: Option<pm_render::text::host::RenderFonts>,
    font_generation: Option<u64>,
    cache: VecDeque<CachedFrame>,
    offscreen: Option<metal::Texture>,
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
    /// The peaks of the content on screen, and the output tone of its frames.
    hdr: Option<((u64, u64, String), Option<crate::color::HdrView>)>,
    output_tone: Option<pm_render::OutputTone>,
    /// Where the viewer's analysis goes, while a Scopes panel is showing.
    pub scope_frames: Option<(
        pm_render::scopes::ScopeConfig,
        async_channel::Sender<crate::scopes::ScopeResult>,
    )>,
    scopes: Option<crate::scopes::Tap>,
    /// The disk render cache playback reads (`crate::render_cache`).
    pub(crate) disk: Option<crate::render_cache::DiskFrames>,
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
        let _ = self.gpu.device.poll(wgpu::PollType::Poll);
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
            .new_library_with_source(include_str!("viewer.metal"), &metal::CompileOptions::new())
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
        let mut renderer = Renderer::new(gpu.device.clone(), gpu.queue.clone());
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
            pool: Vec::with_capacity(11),
            cache: VecDeque::with_capacity(8),
            offscreen: None,
            cache_enabled: true,
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
    /// Changes when a frame a scrub or playback render missed has decoded.
    pub fn media_landed(&self) -> u64 {
        self.renderer.media_landed()
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
    /// The HDR view of this content, and the frame cache kept to its tone.
    fn sync_hdr(&mut self, scene: &Scene) -> Option<crate::color::HdrView> {
        let key = (scene.content, scene.doc.revision(), scene.comp.clone());
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
        self.sync_studio(scene)?;
        self.sync_fonts();
        self.gpu.device.poll(wgpu::PollType::Poll)?;
        let FrameSpec { time, output, .. } = spec;
        let size = spec.render_size();
        let hdr = self.sync_hdr(scene);
        let key = crate::policy::CacheKey {
            content: scene.content,
            revision: scene.doc.revision(),
            time: time.get(),
            width: size.width,
            height: size.height,
            crop: output.crop,
            moment: spec.moment,
        };
        // A revision or size change retires all old cached frames. Leases held
        // by submitted consumers still protect their storage until completion.
        self.cache
            .retain(|c| c.key.content != key.content || c.key.same_content(key));
        let hit = self
            .cache_enabled
            .then(|| self.cache.iter().position(|c| c.key == key))
            .flatten();
        let mut stand_ins = false;
        let mut compat_late = Vec::new();
        // A frame Render Selection put on disk: shown while playing or
        // scrubbing an uncropped viewer instead of drawn again (the present
        // pass scales it to the drawable).
        // Never upscaled: a canvas larger than the composition renders.
        let canvas = output.crop.map_or((output.width, output.height), |c| {
            (c.canvas_width, c.canvas_height)
        });
        let looked = hit.is_none() && spec.moment != Moment::Paused;
        let disk = looked
            .then(|| self.disk.as_ref()?.read(scene, time))
            .flatten()
            .filter(|(size, _)| canvas.0 <= size.width && canvas.1 <= size.height);
        let whole = disk.is_some();
        trace.lock().unwrap().disk = match (looked, whole) {
            (false, _) => 0,
            (true, false) => 1,
            (true, true) => 2,
        };
        let size = disk.as_ref().map_or(size, |(size, _)| *size);
        let (surface, failures) = if let Some(index) = hit {
            let entry = self.cache.remove(index).unwrap();
            let found = (entry.surface.clone(), entry.failures.clone());
            self.cache.push_back(entry);
            trace.lock().unwrap().cache_hit = true;
            found
        } else {
            let bytes = u64::from(size.width) * u64::from(size.height) * 8;
            let capacity = ((128 * 1024 * 1024 / bytes).clamp(1, 8)) as usize;
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
            let surface = match free {
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
            };
            if let Some((_, pixels)) = disk {
                self.gpu.queue.write_texture(
                    surface.texture.as_image_copy(),
                    &pixels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(size.width * 8),
                        rows_per_image: Some(size.height),
                    },
                    surface.texture.size(),
                );
                self.gpu.queue.submit([]);
                if let Some(error) = self.failed.lock().unwrap().take() {
                    bail!("The GPU refused this frame: {error}");
                }
                trace.lock().unwrap().cache_hit = true;
                (surface, Arc::from(Vec::new()))
            } else {
                let target = Target::wrap(surface.texture.clone());
                let mut request = spec.request(scene)?;
                request.output_tone = self.output_tone;
                // An exact read never takes a stand-in, whatever the moment.
                self.renderer
                    .set_scrub(spec.moment == Moment::Scrub && !spec.exact);
                self.renderer.set_playback(spec.decode_deadline);
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
                let rendered = self.renderer.render_into(
                    &scene.doc,
                    &scene.comp,
                    time,
                    &request,
                    scene.media.as_ref(),
                    &target,
                    None,
                );
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
                if let Some(error) = self.failed.lock().unwrap().take() {
                    bail!("The GPU refused this frame: {error}");
                }
                if let Some(error) = self.fonts.as_ref().and_then(|f| f.after_render()) {
                    return Err(FontsPending(error.to_string()).into());
                }
                let failures: Arc<[LayerFailure]> = rendered.failures.into();
                stand_ins = !rendered.stand_ins.is_empty() || !compat_late.is_empty();
                // A frame with stand-ins is not the exact picture of its time.
                if self.cache_enabled && !stand_ins {
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
        let desc = metal::RenderPassDescriptor::new();
        let color = desc.color_attachments().object_at(0).unwrap();
        color.set_texture(Some(destination));
        color.set_load_action(metal::MTLLoadAction::Clear);
        color.set_store_action(metal::MTLStoreAction::Store);
        let command = self.queue.new_command_buffer().to_owned();
        let pass = command.new_render_command_encoder(desc);
        pass.set_render_pipeline_state(&self.pipeline);
        pass.set_fragment_texture(0, Some(&source));
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
        pass.set_fragment_bytes(
            0,
            size_of_val(&params) as u64,
            (&params as *const crate::OverlayParams).cast(),
        );
        pass.draw_primitives(metal::MTLPrimitiveType::Triangle, 0, 3);
        pass.end_encoding();
        let readback = if capture {
            let b = self.metal.new_buffer(
                output.width as u64 * output.height as u64 * 8,
                metal::MTLResourceOptions::StorageModeShared,
            );
            let blit = command.new_blit_command_encoder();
            blit.copy_from_texture_to_buffer(
                destination,
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
            Some(b)
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
            width: output.width,
            height: output.height,
            failures,
            stand_ins,
            compat_late,
        }))
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
    fn only_frames_a_final_one_replaces_take_the_playback_resampler() {
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
        assert_eq!(spec(Moment::Playing, 1.).sampling(), Sampling::Playback);
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
