//! macOS 26's temporal denoised scaler. Wgpu owns all textures; only this
//! module borrows Metal handles, after a completed wgpu submission. The native
//! command buffer completes before wgpu uses its output. No handles escape.
use super::{Renderer, rt_source, storage, uniform};
use std::{ffi::c_void, time::Instant};
unsafe extern "C" {
    fn studio_metalfx_create(
        device: *mut c_void,
        iw: u32,
        ih: u32,
        ow: u32,
        oh: u32,
    ) -> *mut c_void;
    fn studio_metalfx_encode(
        context: *mut c_void,
        textures: *mut *mut c_void,
        view: *const f32,
        projection: *const f32,
        jitter_x: f32,
        jitter_y: f32,
        reset: bool,
    ) -> bool;
    fn studio_metalfx_send(context: *mut c_void, texture: *mut c_void, frame: u64) -> bool;
    fn studio_metalfx_release(context: *mut c_void, frame: u64);
    fn studio_metalfx_destroy(context: *mut c_void);
    fn studio_metal_allocated_bytes(device:*mut c_void)->u64;
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Guides {
    previous: [[f32; 4]; 4],
    misc: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Pack {
    dims: [u32; 4],
    mask: [u32; 4],
    backdrop: [f32; 4],
}

pub(super) struct MetalFX {
    context: *mut c_void,
    output_size: (u32, u32),
    input_size: (u32, u32),
    guides: wgpu::ComputePipeline,
    distance_pipeline: wgpu::ComputePipeline,
    distance_group: wgpu::BindGroup,
    guide_layout: wgpu::BindGroupLayout,
    guide_params: wgpu::Buffer,
    inputs: Vec<wgpu::Texture>,
    output: wgpu::Texture,
    frame_image: wgpu::Texture,
    pack: wgpu::ComputePipeline,
    pack_layout: wgpu::BindGroupLayout,
    pack_params: wgpu::Buffer,
    packed: wgpu::Buffer,
    readback: wgpu::Buffer,
    ray_queries: bool,
    empty_layout: wgpu::BindGroupLayout,
    valid: bool,
}
impl Drop for MetalFX {
    fn drop(&mut self) {
        unsafe { studio_metalfx_destroy(self.context) };
    }
}
fn raw_texture(t: &wgpu::Texture) -> Result<*mut c_void, String> {
    // SAFETY: every texture is created on this renderer's Metal device. We
    // retain the wgpu Texture and its HAL guard while borrowing its ObjC handle.
    let hal = unsafe { t.as_hal::<wgpu::hal::api::Metal>() }.ok_or("not a Metal texture")?;
    Ok((hal.raw_handle() as *const _ as *mut c_void).cast())
}
fn pcg(v: u32) -> u32 {
    let s = v.wrapping_mul(747796405).wrapping_add(2891336453);
    let w = ((s >> ((s >> 28) + 4)) ^ s).wrapping_mul(277803737);
    (w >> 22) ^ w
}
fn jitter(frame: u32) -> [f32; 2] {
    [
        pcg(frame ^ 0x68bc21eb) as f32 / 4294967296. - 0.5,
        pcg(frame ^ 0x02e5be93) as f32 / 4294967296. - 0.5,
    ]
}
impl MetalFX {
    fn new(r: &Renderer, ow: u32, oh: u32) -> Result<Self, String> {
        let device = &r.device;
        let (iw, ih) = (r.width, r.height);
        // SAFETY: the ObjC device is borrowed only for construction. The scaler
        // retains its device; Wgpu continues to own its original reference.
        let hal =
            unsafe { device.as_hal::<wgpu::hal::api::Metal>() }.ok_or("MetalFX needs Metal")?;
        let raw = (&**hal.raw_device() as *const _ as *mut c_void).cast();
        let context = unsafe { studio_metalfx_create(raw, iw, ih, ow, oh) };
        drop(hal);
        if context.is_null() {
            return Err("MetalFX temporal denoised scaling is unavailable on this Mac".into());
        }
        let input_formats = [
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::R32Float,
            wgpu::TextureFormat::Rg32Float,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::R32Float,
            wgpu::TextureFormat::Rgba32Float,
            wgpu::TextureFormat::R32Float,
        ];
        let texture = |format, w, h| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("MetalFX"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let inputs: Vec<_> = input_formats.iter().map(|&f| texture(f, iw, ih)).collect();
        let frame_image = texture(wgpu::TextureFormat::Rgba8Unorm, ow, oh);
        let output = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("MetalFX output"),
            size: wgpu::Extent3d {
                width: ow,
                height: oh,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        // Establish initialization through wgpu before the external writer.
        // Otherwise wgpu's first sampled read clears the MetalFX result.
        let view = output.create_view(&Default::default());
        let mut init = device.create_command_encoder(&Default::default());
        {
            let _pass = init.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("initialize MetalFX output"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        r.queue.submit([init.finish()]);
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let mut entries: Vec<_> = input_formats
            .iter().take(8)
            .enumerate()
            .map(|(i, &format)| wgpu::BindGroupLayoutEntry {
                binding: i as u32,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            })
            .collect();
        entries.push(uniform(8));
        let guide_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("MetalFX guides"),
            entries: &entries,
        });
        let ray_queries = r.uses_hardware_rt();
        let source = if ray_queries {
            rt_source()
        } else {
            include_str!("../shaders/trace.wgsl").into()
        };
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("MetalFX guides"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{source}\n{}",
                    include_str!("../shaders/metalfx-guides.wgsl")
                )
                .into(),
            ),
        });
        let empty_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[],
        });
        let ray_layout = if ray_queries {
            &r.rt.as_ref().unwrap().layout
        } else {
            &empty_layout
        };
        let guides = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("MetalFX guides"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[
                        Some(&r.trace_layout),
                        Some(ray_layout),
                        Some(&guide_layout),
                    ],
                    immediate_size: 0,
                }),
            ),
            module: &module,
            entry_point: Some("fx_guides"),
            compilation_options: Default::default(),
            cache: None,
        });
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        // Reuse normal.w for the ray length, then unpack it in a tiny pass.
        // This keeps the guide kernel within Metal's eight storage-texture limit.
        let distance_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("MetalFX specular distance"),
            entries: &[tex_entry(0), wgpu::BindGroupLayoutEntry {
                binding: 1, visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::R32Float, view_dimension: wgpu::TextureViewDimension::D2 }, count: None,
            }],
        });
        let distance_source = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("MetalFX specular distance"),
            source: wgpu::ShaderSource::Wgsl(r#"
@group(0) @binding(0) var normal:texture_2d<f32>;
@group(0) @binding(1) var distance_out:texture_storage_2d<r32float,write>;
@compute @workgroup_size(8,8)
fn unpack_distance(@builtin(global_invocation_id) g:vec3u){
  if(any(g.xy>=textureDimensions(normal))){return;}
  textureStore(distance_out,vec2i(g.xy),vec4f(textureLoad(normal,vec2i(g.xy),0).w));
}"#.into()),
        });
        let distance_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("MetalFX specular distance"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&distance_layout)], immediate_size: 0 })),
            module: &distance_source, entry_point: Some("unpack_distance"), compilation_options: Default::default(), cache: None,
        });
        let distance_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("MetalFX specular distance"), layout: &distance_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&inputs[5].create_view(&Default::default())) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&inputs[8].create_view(&Default::default())) },
            ],
        });
        let pack_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("MetalFX pack"),
            entries: &[
                uniform(0),
                tex_entry(1),
                tex_entry(2),
                storage(3, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("MetalFX pack"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/metalfx-pack.wgsl").into()),
        });
        let pack = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("MetalFX pack"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[Some(&pack_layout)],
                    immediate_size: 0,
                }),
            ),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let uniform_buf = |size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("MetalFX params"),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Ok(Self {
            context,
            output_size: (ow, oh),
            input_size: (iw, ih),
            guides,
            distance_pipeline,
            distance_group,
            guide_layout,
            guide_params: uniform_buf(std::mem::size_of::<Guides>() as u64),
            inputs,
            output,
            frame_image,
            pack,
            pack_layout,
            pack_params: uniform_buf(std::mem::size_of::<Pack>() as u64),
            packed: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("MetalFX frame"),
                size: u64::from(ow) * u64::from(oh) * 4,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("MetalFX readback"),
                size: u64::from(ow) * u64::from(oh) * 4,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            ray_queries,
            empty_layout,
            valid: false,
        })
    }
    /// Keep the guide/pack pipelines and output textures when only Live's
    /// tracing resolution changes. Recompiling the whole renderer stalls drags.
    fn resize_inputs(&mut self,device:&wgpu::Device,iw:u32,ih:u32)->Result<(),String> {
        // SAFETY: the guard keeps the device alive for construction; the
        // returned native scaler retains its own device reference.
        let hal=unsafe{device.as_hal::<wgpu::hal::api::Metal>()}.ok_or("MetalFX needs Metal")?;
        let raw=(&**hal.raw_device() as *const _ as *mut c_void).cast();
        let context=unsafe{studio_metalfx_create(raw,iw,ih,self.output_size.0,self.output_size.1)};
        drop(hal);
        if context.is_null(){return Err("MetalFX temporal denoised scaling is unavailable on this Mac".into());}
        let inputs=self.inputs.iter().map(|texture|device.create_texture(&wgpu::TextureDescriptor{
            label:Some("MetalFX"),size:wgpu::Extent3d{width:iw,height:ih,depth_or_array_layers:1},
            mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:texture.format(),usage:texture.usage(),view_formats:&[],
        })).collect();
        let previous=std::mem::replace(&mut self.context,context);
        unsafe{studio_metalfx_destroy(previous)};
        self.inputs=inputs;self.input_size=(iw,ih);self.valid=false;
        Ok(())
    }
}
impl Renderer {
    /// Metal's allocations for this process/device; unified allocations can
    /// overlap RSS and must not be added to it as an independent resident sum.
    pub fn allocated_gpu_bytes(&self)->Option<u64>{
        let hal=unsafe{self.device.as_hal::<wgpu::hal::api::Metal>()}?;
        let raw=(&**hal.raw_device() as *const _ as *mut c_void).cast();
        Some(unsafe{studio_metal_allocated_bytes(raw)})
    }
}
impl Renderer {
    pub fn release_surface(&mut self, frame: u64) {
        if let Some(fx) = &self.metalfx {
            unsafe { studio_metalfx_release(fx.context, frame) };
        }
    }
    /// Native-only denoise+upscale. On failure the host keeps the portable Live
    /// pipeline; Refine and Final continue through OIDN.
    pub fn finish_metalfx(
        &mut self,
        ow: u32,
        oh: u32,
        reuse: bool,
        surface_id: Option<u64>,
    ) -> Result<(Vec<u8>, f64), String> {
        let start = Instant::now();
        self.last_surface = None;
        if self
            .metalfx
            .as_ref()
            .is_some_and(|f| f.output_size != (ow, oh) || f.ray_queries!=self.uses_hardware_rt())
        {
            self.metalfx = None;
        }
        if self.metalfx.is_none() {
            self.metalfx = Some(MetalFX::new(self, ow, oh)?);
        }
        if self.metalfx.as_ref().unwrap().input_size!=(self.width,self.height) {
            self.metalfx.as_mut().unwrap().resize_inputs(&self.device,self.width,self.height)?;
        }
        let fx = self.metalfx.as_ref().unwrap();
        let j = jitter(self.frame);
        let near = 0.01f32;
        let far = (self.bounds[3] * 1000. + self.base.cam_fwd[3] * 10.).max(10000.);
        if !reuse || !fx.valid {
            let params = Guides {
                previous: self.previous_camera,
                misc: [near, far, j[0], j[1]],
            };
            self.queue
                .write_buffer(&fx.guide_params, 0, bytemuck::bytes_of(&params));
            let views: Vec<_> = fx
                .inputs
                .iter().take(8)
                .map(|t| t.create_view(&Default::default()))
                .collect();
            let mut entries: Vec<_> = views
                .iter()
                .enumerate()
                .map(|(i, v)| wgpu::BindGroupEntry {
                    binding: i as u32,
                    resource: wgpu::BindingResource::TextureView(v),
                })
                .collect();
            entries.push(wgpu::BindGroupEntry {
                binding: 8,
                resource: fx.guide_params.as_entire_binding(),
            });
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("MetalFX guides"),
                layout: &fx.guide_layout,
                entries: &entries,
            });
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                pass.set_pipeline(&fx.guides);
                pass.set_bind_group(0, self.trace_group.as_ref().unwrap(), &[]);
                if fx.ray_queries {
                    pass.set_bind_group(1, self.rt.as_ref().unwrap().group.as_ref(), &[]);
                } else {
                    let empty = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: None,
                        layout: &fx.empty_layout,
                        entries: &[],
                    });
                    pass.set_bind_group(1, &empty, &[]);
                }
                pass.set_bind_group(2, &group, &[]);
                pass.dispatch_workgroups(self.width.div_ceil(8), self.height.div_ceil(8), 1);
            }
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                pass.set_pipeline(&fx.distance_pipeline);
                pass.set_bind_group(0, &fx.distance_group, &[]);
                pass.dispatch_workgroups(self.width.div_ceil(8), self.height.div_ceil(8), 1);
            }
            self.queue.submit([enc.finish()]);
            self.wait();
            let mut textures = Vec::new();
            for input in fx.inputs.iter().take(7) {
                textures.push(raw_texture(input)?);
            }
            textures.push(raw_texture(&fx.output)?);
            textures.push(raw_texture(&fx.inputs[8])?);
            let p = self.base.cam_pos;
            let f = self.base.cam_fwd;
            let r = self.base.cam_right;
            let u = self.base.cam_up;
            let dot = |a: [f32; 4]| a[0] * p[0] + a[1] * p[1] + a[2] * p[2];
            let view = [
                r[0],
                u[0],
                f[0],
                0.,
                r[1],
                u[1],
                f[1],
                0.,
                r[2],
                u[2],
                f[2],
                0.,
                -dot(r),
                -dot(u),
                -dot(f),
                1.,
            ];
            let ortho = self.base.opts[3] & 1 != 0;
            let projection = [
                1. / r[3],
                0.,
                0.,
                0.,
                0.,
                1. / u[3],
                0.,
                0.,
                0.,
                0.,
                if ortho {
                    1. / (far - near)
                } else {
                    far / (far - near)
                },
                if ortho { 0. } else { 1. },
                0.,
                0.,
                if ortho {
                    -near / (far - near)
                } else {
                    -near * far / (far - near)
                },
                if ortho { 1. } else { 0. },
            ];
            // SAFETY: wgpu work has completed. Borrowed Metal textures remain
            // alive in `fx`; this call waits for its own command buffer.
            let ok = unsafe {
                studio_metalfx_encode(
                    fx.context,
                    textures.as_mut_ptr(),
                    view.as_ptr(),
                    projection.as_ptr(),
                    j[0],
                    j[1],
                    !self.history_valid || !fx.valid,
                )
            };
            if !ok {
                return Err("MetalFX command buffer failed".into());
            }
        }
        let params = Pack {
            dims: [ow, oh, self.width, self.height],
            mask: [
                self.output_layer.map_or(0, |l| l + 1),
                u32::from(self.output_include_void),
                0,
                0,
            ],
            backdrop: self.backdrop,
        };
        self.queue
            .write_buffer(&fx.pack_params, 0, bytemuck::bytes_of(&params));
        let color = fx.output.create_view(&Default::default());
        let mask = fx.inputs[7].create_view(&Default::default());
        let frame_image = fx.frame_image.create_view(&Default::default());
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("MetalFX pack"),
            layout: &fx.pack_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: fx.pack_params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&color),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&mask),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: fx.packed.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&frame_image),
                },
            ],
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&fx.pack);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(ow.div_ceil(8), oh.div_ceil(8), 1);
        }
        if let Some(frame) = surface_id {
            self.queue.submit([enc.finish()]);
            self.wait();
            // SAFETY: the completed image is borrowed for one blit. The sender
            // transfers a Mach right; no process-local pointer leaves Rust.
            if unsafe { studio_metalfx_send(fx.context, raw_texture(&fx.frame_image)?, frame) } {
                self.last_surface = Some(frame);
                self.metalfx.as_mut().unwrap().valid = true;
                self.advance_history();
                return Ok((Vec::new(), start.elapsed().as_secs_f64() * 1000.));
            }
            enc = self.device.create_command_encoder(&Default::default());
        }
        enc.copy_buffer_to_buffer(
            &fx.packed,
            0,
            &fx.readback,
            0,
            u64::from(ow) * u64::from(oh) * 4,
        );
        self.queue.submit([enc.finish()]);
        let slice = fx.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.wait();
        let bytes = slice
            .get_mapped_range()
            .map_err(|e| e.to_string())?
            .to_vec();
        fx.readback.unmap();
        self.metalfx.as_mut().unwrap().valid = true;
        self.advance_history();
        Ok((bytes, start.elapsed().as_secs_f64() * 1000.))
    }
}
