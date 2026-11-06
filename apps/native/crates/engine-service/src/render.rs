//! Adapted from engine-host: bounded cached/leased surfaces, nonblocking GPU
//! completion and exact Metal-device injection. The final consumer is the viewer.
use crate::policy::{Moment, Output};
use crate::trace::{Trace, now};
use anyhow::{Result, anyhow, bail};
use foreign_types::ForeignTypeRef;
use pm_gpu::{Gpu, Target, bridge_out::surface::ExportedSurface};
use pm_protocol::{
    color::Precision,
    common::{Size, Ticks},
    interop::{SurfaceFormat, SurfaceRequest},
};
use pm_render::{LayerFailure, RenderError, Renderer};
use std::{
    collections::VecDeque,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

/// One immutable render input. Content identity rotates on document, comp,
/// assets, definitions or fonts; it is independent of display output identity.
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
}
impl FrameSpec {
    pub fn exact(time: Ticks, output: Output) -> Self {
        Self {
            time,
            output,
            moment: Moment::Paused,
            scale: 1.,
            decode_deadline: None,
        }
    }
    fn render_size(&self) -> Size {
        let px = |v: u32| ((f64::from(v) * self.scale).round() as u32).max(1);
        Size::new(px(self.output.width), px(self.output.height))
    }
    fn request(&self, scene: &Scene) -> Result<pm_render::RenderRequest> {
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
        request.sampling = if playing {
            pm_render::sampling::Sampling::Playback
        } else {
            pm_render::sampling::Sampling::Final
        };
        Ok(request)
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
struct CompilerBusy;
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
    fonts: Option<pm_node::fonts::render::RenderFonts>,
    font_generation: Option<u64>,
    cache: VecDeque<CachedFrame>,
    offscreen: Option<metal::Texture>,
    pub cache_enabled: bool,
    studio_content: Option<(usize, u64)>,
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
    /// Video drew a nearest decoded frame; the exact one is on its way.
    pub stand_ins: bool,
}
impl Engine {
    pub fn new(gpu: Gpu) -> Result<Self> {
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
        renderer.set_definitions(&pm_render::all_builtin_definitions())?;
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
        })
    }
    pub fn set_fonts(&mut self, fonts: pm_node::fonts::render::RenderFonts) {
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
    pub fn prepare(&mut self, scene: &Scene, time: Ticks, spec: FrameSpec) {
        if self.sync_studio(scene).is_err() {
            return;
        }
        let Ok(request) = spec.request(scene) else {
            return;
        };
        // A plan that cannot be built reports through the frame itself.
        let _ = self.renderer.prepare_media(
            &scene.doc,
            &scene.comp,
            time,
            &request,
            scene.media.as_ref(),
        );
    }
    fn sync_studio(&mut self, scene: &Scene) -> Result<()> {
        let key = (Arc::as_ptr(&scene.doc) as usize, scene.content);
        if self.studio_content != Some(key) {
            self.renderer
                .set_main_effects_for_document(&scene.doc)
                .map_err(|e| {
                    if e.contains("compiler-busy") {
                        anyhow::Error::new(CompilerBusy)
                    } else {
                        anyhow::Error::msg(e)
                    }
                })?;
            #[cfg(feature = "kernel")]
            crate::studio::attach(&mut self.renderer, scene.doc.clone(), scene.media.as_ref());
            self.studio_content = Some(key);
        }
        Ok(())
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
        if !f.failures.is_empty() {
            bail!("export layer failures: {:?}", f.failures);
        }
        Ok(f.rgba)
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
            let target = Target::wrap(surface.texture.clone());
            let request = spec.request(scene)?;
            self.renderer.set_scrub(spec.moment == Moment::Scrub);
            self.renderer.set_playback(spec.decode_deadline);
            self.renderer.set_isolate_layers(true);
            trace.lock().unwrap().render_start = now();
            let rendered = self.renderer.render_into(
                &scene.doc,
                &scene.comp,
                time,
                &request,
                scene.media.as_ref(),
                &target,
                None,
            );
            trace.lock().unwrap().render_submitted = now();
            self.renderer.set_scrub(false);
            self.renderer.set_playback(None);
            self.renderer.set_isolate_layers(false);
            let rendered = rendered?;
            if let Some(error) = self.fonts.as_ref().and_then(|f| f.after_render()) {
                return Err(FontsPending(error.to_string()).into());
            }
            let failures: Arc<[LayerFailure]> = rendered.failures.into();
            stand_ins = !rendered.stand_ins.is_empty();
            // A frame with stand-ins is not the exact picture of its time.
            if self.cache_enabled && !stand_ins {
                self.cache.push_back(CachedFrame {
                    key,
                    surface: surface.clone(),
                    failures: failures.clone(),
                });
            }
            (surface, failures)
        };
        let source = unsafe {
            let hal = surface.texture.as_hal::<wgpu_hal::api::Metal>().unwrap();
            metal::TextureRef::from_ptr(
                (hal.raw_handle() as *const _ as *mut metal::MTLTexture).cast(),
            )
            .to_owned()
        };
        trace.lock().unwrap().drawable_asked = now();
        let drawable = if let Some(layer) = layer {
            let Some(d) = layer.next_drawable() else {
                return Ok(None);
            };
            Some(d.to_owned())
        } else {
            None
        };
        trace.lock().unwrap().drawable_acquired = now();
        if let Some(d) = &drawable {
            if d.texture().width() != output.width as u64
                || d.texture().height() != output.height as u64
            {
                return Ok(None);
            }
        } else if self
            .offscreen
            .as_ref()
            .is_none_or(|t| t.width() != output.width as u64 || t.height() != output.height as u64)
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
        params.output = self.mastering.params()?;
        params.reduced = u32::from(size != Size::new(output.width, output.height));
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
        }))
    }
}
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
            match engine.frame(&scene, spec, None, &Default::default(), false, Arc::default()) {
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
