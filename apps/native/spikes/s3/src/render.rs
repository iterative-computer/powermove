//! Adapted from apps/mac/crates/engine-host: worker-only GPU waits, three leased
//! surfaces, exact Metal-device injection. Here the final consumer IS the viewer.
use crate::policy::Output;
use anyhow::{Result, anyhow, bail};
use foreign_types::ForeignTypeRef;
use pm_gpu::{Gpu, Target, bridge_out::surface::ExportedSurface};
use pm_protocol::{
    color::Precision,
    common::{Size, Ticks},
    interop::{SurfaceFormat, SurfaceRequest},
};
use pm_render::Renderer;
use std::sync::{Arc, atomic::AtomicBool};

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
    ring: [Option<Arc<ExportedSurface>>; 3],
    next: usize,
}
pub struct Submission {
    pub drawable: metal::MetalDrawable,
    pub command: metal::CommandBuffer,
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
            queue: metal.new_command_queue(),
            gpu,
            renderer,
            metal,
            pipeline,
            ring: Default::default(),
            next: 0,
        })
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
    pub fn encode(
        &mut self,
        scene: &Scene,
        time: Ticks,
        output: Output,
        layer: &metal::MetalLayer,
        rect: [f32; 4],
        overlay: u32,
        capture: bool,
    ) -> Result<Option<Submission>> {
        objc2::rc::autoreleasepool(|_| {
            self.encode_inner(scene, time, output, layer, rect, overlay, capture)
        })
    }
    fn encode_inner(
        &mut self,
        scene: &Scene,
        time: Ticks,
        output: Output,
        layer: &metal::MetalLayer,
        rect: [f32; 4],
        overlay: u32,
        capture: bool,
    ) -> Result<Option<Submission>> {
        let slot = self.next;
        self.next = (self.next + 1) % 3;
        if self.ring[slot]
            .as_ref()
            .is_some_and(|s| Arc::strong_count(s) > 1)
        {
            bail!("in-flight ring slot must never be overwritten");
        }
        let size = Size::new(output.width, output.height);
        if self.ring[slot].as_ref().is_none_or(|s| s.size != size) {
            self.ring[slot] = Some(Arc::new(ExportedSurface::create(
                &self.gpu.device,
                &SurfaceRequest {
                    size,
                    format: SurfaceFormat::Rgba16Float,
                },
            )?));
        }
        let surface = self.ring[slot].as_ref().unwrap().clone();
        let target = Target::wrap(surface.texture.clone());
        let request = pm_render::RenderRequest::sized(size, Precision::Float16);
        let rendered = self.renderer.render_into(
            &scene.doc,
            &scene.comp,
            time,
            &request,
            &scene.media,
            &target,
            None,
        )?;
        if !rendered.failures.is_empty() {
            bail!("viewer layer failures: {:?}", rendered.failures);
        }
        // Producer completion on render worker. Never wait for this on AppKit.
        self.gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
        let Some(drawable) = layer.next_drawable() else {
            return Ok(None);
        };
        let drawable = drawable.to_owned();
        if drawable.texture().width() != output.width as u64
            || drawable.texture().height() != output.height as u64
        {
            return Ok(None);
        }
        let source = unsafe {
            let hal = surface.texture.as_hal::<wgpu_hal::api::Metal>().unwrap();
            metal::TextureRef::from_ptr(
                (&*hal.raw_handle() as *const _ as *mut metal::MTLTexture).cast(),
            )
            .to_owned()
        };
        let desc = metal::RenderPassDescriptor::new();
        let color = desc.color_attachments().object_at(0).unwrap();
        color.set_texture(Some(drawable.texture()));
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
                drawable.texture(),
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
        Ok(Some(Submission {
            drawable,
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
        self.command.present_drawable(&self.drawable);
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
