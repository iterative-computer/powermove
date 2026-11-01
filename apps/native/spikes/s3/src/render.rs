//! Adapted from engine-host: bounded cached/leased surfaces, nonblocking GPU
//! completion and exact Metal-device injection. The final consumer is the viewer.
use crate::policy::Output;
use crate::trace::{Trace, now};
use anyhow::{Result, anyhow, bail};
use foreign_types::ForeignTypeRef;
use pm_gpu::{Gpu, Target, bridge_out::surface::ExportedSurface};
use pm_protocol::{
    color::Precision,
    common::{Size, Ticks},
    interop::{SurfaceFormat, SurfaceRequest},
};
use pm_render::Renderer;
use std::{
    collections::VecDeque,
    sync::{Arc, atomic::AtomicBool},
};

pub struct Scene {
    pub doc: pm_model::Document,
    pub comp: String,
    pub media: pm_export::render::MediaPaths,
}
impl Scene {
    pub fn load(path: &str) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        let container = pm_model::container::read_container_bytes(
            &bytes,
            pm_model::container::ReadOptions { verify_media: true },
        )?;
        let project = pm_model::canonical::canonical_project(pm_model::canonical::into_project(
            container.document,
        ));
        let json: serde_json::Value =
            serde_json::from_str(&pm_model::json::to_canonical_string(&project))?;
        let comp = json["compId"]
            .as_str()
            .ok_or_else(|| anyhow!("document needs compId"))?
            .to_string();
        Ok(Self {
            doc: pm_model::Document::new(Some(project), 1),
            comp,
            media: Default::default(),
        })
    }
    pub fn pq(path: &str) -> Result<Self> {
        let j = serde_json::json!({"id":"s3-pq","compId":"s3-pq-root","name":"PQ fixture","w":320,"h":180,"fps":25,"dur":4,"bg":"#000000","colorDepth":32,"requiresEngine":"native","backgroundFill":{"type":"solid","stops":[{"id":"s","position":0,"color":"#000000"}]},"layers":[{"id":"pq","type":"video","name":"PQ","from":0,"dur":4,"on":true,"blend":"normal","threeD":false,"p":{"position.x":{"v":160},"position.y":{"v":90},"scale.x":{"v":100},"scale.y":{"v":100},"opacity":{"v":100}},"d":{"asset":"pq-source","fit":"stretch","w":320,"h":180,"trim":0,"speed":1},"fx":[],"masks":[]}],"assets":{"pq-source":{"id":"pq-source","name":"PQ test source","kind":"video","format":"mov","w":320,"h":180,"dur":4,"fps":25,"persisted":true,"interpret":{"transfer":"pq","primaries":"bt2020","matrix":"bt2020Ncl"}}},"comps":{}});
        let project =
            pm_model::canonical::canonical_project(pm_model::json::parse(&j.to_string())?);
        let mut media = pm_export::render::MediaPaths::default();
        media.insert("pq-source", path.into());
        Ok(Self {
            doc: pm_model::Document::new(Some(project), 2),
            comp: "s3-pq-root".into(),
            media,
        })
    }
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
    gpu: Gpu,
    renderer: Renderer,
    metal: metal::Device,
    queue: metal::CommandQueue,
    pipeline: metal::RenderPipelineState,
    producer_probe: metal::Buffer,
    pool: Vec<Arc<ExportedSurface>>,
    cache: VecDeque<(crate::policy::CacheKey, Arc<ExportedSurface>)>,
    offscreen: Option<metal::Texture>,
    pub legacy_sync: bool,
    pub cache_enabled: bool,
}
pub struct Submission {
    pub drawable: Option<metal::MetalDrawable>,
    pub trace: Arc<std::sync::Mutex<Trace>>,
    pub command: metal::CommandBuffer,
    producer: metal::CommandBuffer,
    pub surface: Arc<ExportedSurface>,
    pub readback: Option<metal::Buffer>,
    pub width: u32,
    pub height: u32,
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
        renderer.set_definitions(&pm_render::all_builtin_definitions())?;
        Ok(Self {
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
            producer_probe: metal.new_buffer(256, metal::MTLResourceOptions::StorageModeShared),
            gpu,
            renderer,
            metal,
            pipeline,
            pool: Vec::with_capacity(11),
            cache: VecDeque::with_capacity(8),
            offscreen: None,
            legacy_sync: false,
            cache_enabled: true,
        })
    }
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
    pub fn export(&mut self, scene: &Scene, time: Ticks, size: Size) -> Result<Vec<f32>> {
        let request = pm_render::RenderRequest::sized(size, Precision::Float16);
        let f = pm_export::render::render_frame(
            &mut self.renderer,
            &scene.doc,
            &scene.comp,
            time,
            &request,
            &scene.media,
            &pm_export::render::Unrestricted,
            1,
            &AtomicBool::new(false),
        )?;
        if !f.failures.is_empty() {
            bail!("export layer failures: {:?}", f.failures);
        }
        Ok(f.rgba)
    }
    pub fn frame(
        &mut self,
        scene: &Scene,
        time: Ticks,
        output: Output,
        layer: Option<&metal::MetalLayer>,
        rect: [f32; 4],
        overlay: u32,
        capture: bool,
        trace: Arc<std::sync::Mutex<Trace>>,
    ) -> Result<Option<Submission>> {
        objc2::rc::autoreleasepool(|_| {
            self.encode_inner(scene, time, output, layer, rect, overlay, capture, trace)
        })
    }
    fn encode_inner(
        &mut self,
        scene: &Scene,
        time: Ticks,
        output: Output,
        layer: Option<&metal::MetalLayer>,
        rect: [f32; 4],
        overlay: u32,
        capture: bool,
        trace: Arc<std::sync::Mutex<Trace>>,
    ) -> Result<Option<Submission>> {
        self.gpu.device.poll(wgpu::PollType::Poll)?;
        let key = crate::policy::CacheKey {
            revision: scene.doc.revision(),
            time: time.get(),
            width: output.width,
            height: output.height,
        };
        // A revision or size change retires all old cached frames. Leases held
        // by submitted consumers still protect their storage until completion.
        self.cache.retain(|(k, _)| k.same_content(key));
        let hit = self
            .cache_enabled
            .then(|| self.cache.iter().position(|(k, _)| *k == key))
            .flatten();
        let size = Size::new(output.width, output.height);
        let surface = if let Some(index) = hit {
            let entry = self.cache.remove(index).unwrap();
            let surface = entry.1.clone();
            self.cache.push_back(entry);
            trace.lock().unwrap().cache_hit = true;
            surface
        } else {
            if self.cache.len() == 8 {
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
            let request = pm_render::RenderRequest::sized(size, Precision::Float16);
            trace.lock().unwrap().render_start = now();
            let rendered = self.renderer.render_into(
                &scene.doc,
                &scene.comp,
                time,
                &request,
                &scene.media,
                &target,
                None,
            )?;
            trace.lock().unwrap().render_submitted = now();
            if !rendered.failures.is_empty() {
                bail!("viewer layer failures: {:?}", rendered.failures);
            }
            if self.legacy_sync {
                self.gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
            }
            if self.cache_enabled {
                self.cache.push_back((key, surface.clone()));
            }
            surface
        };
        trace.lock().unwrap().producer_wait_end = now();
        let source = unsafe {
            let hal = surface.texture.as_hal::<wgpu_hal::api::Metal>().unwrap();
            metal::TextureRef::from_ptr(
                (&*hal.raw_handle() as *const _ as *mut metal::MTLTexture).cast(),
            )
            .to_owned()
        };
        // Nonblocking producer-completion marker on the SAME queue. Its end
        // timestamp is an upper bound on producer GPU readiness, not isolated
        // producer execution time. The consumer below has isolated GPU times.
        let marker = self.queue.new_command_buffer().to_owned();
        // Touch the producer's tracked texture: an EMPTY command buffer can
        // overlap unrelated work, so its completion would not be a valid bound.
        let blit = marker.new_blit_command_encoder();
        blit.copy_from_texture_to_buffer(
            &source,
            0,
            0,
            metal::MTLOrigin { x: 0, y: 0, z: 0 },
            metal::MTLSize {
                width: 1,
                height: 1,
                depth: 1,
            },
            &self.producer_probe,
            0,
            256,
            256,
            metal::MTLBlitOption::None,
        );
        blit.end_encoding();
        let t = trace.clone();
        marker.add_completed_handler(
            &block::ConcreteBlock::new(move |b: &metal::CommandBufferRef| {
                t.lock().unwrap().producer_ready = gpu_time(b, false);
            })
            .copy(),
        );
        marker.commit();
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
        let pass = command.new_render_command_encoder(&desc);
        pass.set_render_pipeline_state(&self.pipeline);
        pass.set_fragment_texture(0, Some(&source));
        #[repr(C)]
        struct Params {
            rect: [f32; 4],
            headroom: f32,
            overlay: u32,
            pad: [u32; 2],
        }
        let params = Params {
            rect,
            headroom: output.headroom.max(1.) as f32,
            overlay,
            pad: [0; 2],
        };
        pass.set_fragment_bytes(
            0,
            size_of::<Params>() as u64,
            (&params as *const Params).cast(),
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
            producer: marker,
            trace,
            command,
            surface,
            readback,
            width: output.width,
            height: output.height,
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
        objc2::rc::autoreleasepool(|_| self.submit_inner(due));
    }
    fn submit_inner(&self, due: Option<f64>) {
        if let Some(drawable) = &self.drawable {
            self.trace.lock().unwrap().present_requested = now();
            let t = self.trace.clone();
            drawable.add_presented_handler(
                &block::ConcreteBlock::new(move |d: &metal::DrawableRef| {
                    let mut t = t.lock().unwrap();
                    t.display = Some(d.presented_time());
                    t.display_callback = now();
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
            })
            .copy(),
        );
        self.trace.lock().unwrap().commit = now();
        self.command.commit();
    }
    pub fn finish_producer(&self) -> Result<()> {
        self.producer.wait_until_completed();
        anyhow::ensure!(
            self.producer.status() == metal::MTLCommandBufferStatus::Completed,
            "producer failed"
        );
        Ok(())
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
