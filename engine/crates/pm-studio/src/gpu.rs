//! The GPU side: one path-tracing pipeline, resolve, denoise and pack.

use crate::math::Vec3;
use crate::scene::{Prepared, Scene};
use std::time::Instant;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    cam_pos: [f32; 4],
    cam_fwd: [f32; 4],
    cam_right: [f32; 4],
    cam_up: [f32; 4],
    backdrop: [f32; 4],
    env_zenith: [f32; 4],
    env_horizon: [f32; 4],
    dims: [u32; 4],
    counts: [u32; 4],
    opts: [u32; 4],
    ground: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PostParams {
    dims: [u32; 4],
    k: [f32; 4],
    backdrop: [f32; 4],
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub adapter: String,
    width: u32,
    height: u32,
    trace: wgpu::ComputePipeline,
    resolve: wgpu::ComputePipeline,
    compose: wgpu::ComputePipeline,
    atrous: wgpu::ComputePipeline,
    pack: wgpu::ComputePipeline,
    trace_layout: wgpu::BindGroupLayout,
    post_layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    post_params: Vec<wgpu::Buffer>,
    accum: wgpu::Buffer,
    bgsum: wgpu::Buffer,
    gbuf: wgpu::Buffer,
    ebuf: wgpu::Buffer,
    idbuf: wgpu::Buffer,
    display: [wgpu::Buffer; 2],
    packed: wgpu::Buffer,
    readback: wgpu::Buffer,
    screen: wgpu::Texture,
    sampler: wgpu::Sampler,
    trace_group: Option<wgpu::BindGroup>,
    scene_buffers: Vec<wgpu::Buffer>,
    light_count: u32,
    base: Params,
    exposure: f32,
    backdrop: [f32; 4],
    /// Samples accumulated since the last reset.
    pub samples: u32,
    frame: u32,
    /// Live tier: cheaper floor shadows and primary-only mirror coats.
    pub live: bool,
}

fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}
fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

impl Renderer {
    pub fn new(width: u32, height: u32) -> Result<Renderer, String> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::METAL;
        let instance = wgpu::Instance::new(desc);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        let limits = wgpu::Limits {
            max_storage_buffer_binding_size: adapter.limits().max_storage_buffer_binding_size,
            max_buffer_size: adapter.limits().max_buffer_size,
            max_storage_buffers_per_shader_stage: 12,
            max_texture_array_layers: adapter.limits().max_texture_array_layers,
            ..wgpu::Limits::default()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("pm-studio"),
            required_limits: limits,
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;

        let trace_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("studio trace"),
            entries: &[
                uniform(0),
                storage(1, true),
                storage(2, true),
                storage(3, true),
                storage(4, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 6, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                storage(7, false),
                storage(8, false),
                storage(9, false),
                storage(10, false),
                storage(11, true),
                storage(12, false),
            ],
        });
        let post_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("studio post"),
            entries: &[uniform(0), storage(1, true), storage(2, true), storage(3, true), storage(4, true), storage(5, false), storage(6, false), storage(7, true)],
        });
        let pipeline = |module: &wgpu::ShaderModule, layout: &wgpu::BindGroupLayout, entry: &str| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(entry), bind_group_layouts: &[Some(layout)], immediate_size: 0 });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let trace_src = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("trace.wgsl"), source: wgpu::ShaderSource::Wgsl(include_str!("shaders/trace.wgsl").into()) });
        let post_src = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("post.wgsl"), source: wgpu::ShaderSource::Wgsl(include_str!("shaders/post.wgsl").into()) });
        let trace = pipeline(&trace_src, &trace_layout, "main");
        let resolve = pipeline(&post_src, &post_layout, "resolve");
        let compose = pipeline(&post_src, &post_layout, "compose");
        let atrous = pipeline(&post_src, &post_layout, "atrous");
        let pack = pipeline(&post_src, &post_layout, "pack");

        let pixels = (width * height) as u64;
        let buf = |label: &str, size: u64, usage: wgpu::BufferUsages| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
        let rw = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        let screen = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("screen"),
            size: wgpu::Extent3d { width: 4, height: 4, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let r = Renderer {
            adapter: adapter.get_info().name,
            params: buf("params", std::mem::size_of::<Params>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST),
            post_params: (0..9).map(|_| buf("post params", std::mem::size_of::<PostParams>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST)).collect(),
            accum: buf("accum", pixels * 16, rw),
            bgsum: buf("bgsum", pixels * 16, rw),
            gbuf: buf("gbuf", pixels * 16, rw),
            ebuf: buf("ebuf", pixels * 16, rw),
            idbuf: buf("idbuf", pixels * 4, rw),
            display: [buf("display a", pixels * 16, rw), buf("display b", pixels * 16, rw)],
            packed: buf("packed", pixels * 4, rw),
            readback: buf("readback", pixels * 4, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST),
            device,
            queue,
            width,
            height,
            trace,
            resolve,
            compose,
            atrous,
            pack,
            trace_layout,
            post_layout,
            screen,
            sampler,
            trace_group: None,
            scene_buffers: Vec::new(),
            light_count: 0,
            base: bytemuck::Zeroable::zeroed(),
            exposure: 1.0,
            backdrop: [1.0; 4],
            samples: 0,
            frame: 0,
            live: false,
        };
        Ok(r)
    }

    /// The display composition, sRGB RGBA8.
    pub fn set_screen(&mut self, rgba: &[u8], w: u32, h: u32) {
        self.screen = self.device.create_texture_with_data(
            &self.queue,
            &wgpu::TextureDescriptor {
                label: Some("screen"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            rgba,
        );
        self.trace_group = None;
    }

    /// Upload a scene (geometry, materials, lights, camera). Resets accumulation.
    pub fn set_scene(&mut self, scene: &Scene, prepared: &Prepared) {
        let init = |label: &str, data: &[u8]| {
            let data: Vec<u8> = if data.is_empty() { vec![0u8; 64] } else { data.to_vec() };
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: &data, usage: wgpu::BufferUsages::STORAGE })
        };
        self.scene_buffers = vec![
            init("tris", bytemuck::cast_slice(&prepared.tris)),
            init("nodes", bytemuck::cast_slice(&prepared.nodes)),
            init("materials", bytemuck::cast_slice(&prepared.materials)),
            init("lights", bytemuck::cast_slice(&prepared.lights)),
            init("positions", bytemuck::cast_slice(&prepared.positions)),
        ];
        self.light_count = prepared.lights.len() as u32;
        self.trace_group = None;
        self.set_camera(scene);
    }

    /// Camera, backdrop and environment only (no geometry upload).
    pub fn set_camera(&mut self, scene: &Scene) {
        let c = &scene.camera;
        let fwd = (c.target - c.position).normalize();
        let right = fwd.cross(Vec3::Y).normalize();
        let up = right.cross(fwd).normalize();
        let tan_y = (c.fov.to_radians() * 0.5).tan();
        let aspect = self.width as f32 / self.height as f32;
        let lens_radius = if c.f_stop > 0.0 { c.focal_mm / (2.0 * c.f_stop) } else { 0.0 };
        let focus = c.focus.map(|f| (f - c.position).dot(fwd)).unwrap_or((c.target - c.position).length());
        self.base = Params {
            cam_pos: c.position.to4(lens_radius),
            cam_fwd: fwd.to4(focus),
            cam_right: right.to4(tan_y * aspect),
            cam_up: up.to4(tan_y),
            backdrop: scene.backdrop.to4(if scene.shadow_catcher { 1.0 } else { 0.0 }),
            env_zenith: scene.environment.0.to4(scene.exposure),
            env_horizon: scene.environment.1.to4(0.82),
            dims: [self.width, self.height, 0, 1],
            counts: [self.light_count, 6, 1, 0],
            opts: [0; 4],
            ground: (scene.environment.1 * 0.5).to4(0.0),
        };
        self.exposure = scene.exposure;
        // Product scenes show their backdrop; the app's layers leave the void transparent.
        self.backdrop = scene.backdrop.to4(1.0);
        self.samples = 0;
    }

    fn trace_group(&mut self) -> &wgpu::BindGroup {
        if self.trace_group.is_none() {
            let view = self.screen.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
            let b = &self.scene_buffers;
            self.trace_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("trace"),
                layout: &self.trace_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.params.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: b[0].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: b[1].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: b[2].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: b[3].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 7, resource: self.accum.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 8, resource: self.bgsum.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 9, resource: self.gbuf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 10, resource: self.ebuf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 11, resource: b[4].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 12, resource: self.idbuf.as_entire_binding() },
                ],
            }));
        }
        self.trace_group.as_ref().unwrap()
    }

    fn wait(&self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// Trace `spp` samples per pixel with up to `bounces` bounces; `reset`
    /// starts a new accumulation. Returns GPU wall time in ms.
    pub fn trace(&mut self, spp: u32, bounces: u32, reset: bool, clamp: u32) -> f64 {
        let (w, h) = (self.width, self.height);
        // Several dispatches of at most 4 samples keep each submission short.
        let start = Instant::now();
        let mut left = spp;
        let mut first = true;
        while left > 0 {
            let n = left.min(4);
            self.frame = self.frame.wrapping_add(1);
            let mut p = self.base;
            p.dims = [w, h, self.frame, n];
            p.counts = [self.light_count, bounces, if reset && first { 1 } else { 0 }, clamp];
            p.opts = [u32::from(self.live), 0, 0, 0];
            self.queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
            let group = self.trace_group().clone();
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.trace);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
            }
            self.queue.submit([enc.finish()]);
            self.wait();
            if reset && first {
                self.samples = 0;
            }
            self.samples += n;
            left -= n;
            first = false;
        }
        start.elapsed().as_secs_f64() * 1000.0
    }

    fn post_group(&self, slot: usize, src: usize, dst: usize) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post"),
            layout: &self.post_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.post_params[slot].as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.accum.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.bgsum.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.gbuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: self.display[src].as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: self.display[dst].as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: self.packed.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: self.ebuf.as_entire_binding() },
            ],
        })
    }

    /// Resolve the accumulation to display RGBA8, denoising with `passes`
    /// à-trous iterations (0 for Refine/Final). Returns (pixels, ms).
    pub fn finish(&mut self, passes: u32) -> (Vec<u8>, f64) {
        self.post(passes, true)
    }

    /// GPU time of resolve + denoise + compose + pack alone (what a viewer
    /// frame costs; it presents the result instead of reading it back).
    pub fn post_only(&mut self, passes: u32) -> f64 {
        self.post(passes, false).1
    }

    fn post(&mut self, passes: u32, readback: bool) -> (Vec<u8>, f64) {
        let start = Instant::now();
        let (w, h) = (self.width, self.height);
        let groups = (w.div_ceil(8), h.div_ceil(8));
        let mut enc = self.device.create_command_encoder(&Default::default());
        let mut slot = 0;
        let run = |enc: &mut wgpu::CommandEncoder, pipe: &wgpu::ComputePipeline, src: usize, dst: usize, step: u32, slot: &mut usize| {
            let params = PostParams { dims: [w, h, step, 0], k: [self.exposure, 1.6, 0.0, 0.0], backdrop: self.backdrop };
            self.queue.write_buffer(&self.post_params[*slot], 0, bytemuck::bytes_of(&params));
            let group = self.post_group(*slot, src, dst);
            *slot += 1;
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        };
        // resolve → display[0]; à-trous ping-pongs; compose; pack.
        run(&mut enc, &self.resolve, 1, 0, 1, &mut slot);
        let mut cur = 0;
        for i in 0..passes.min(5) {
            run(&mut enc, &self.atrous, cur, 1 - cur, 1 << i, &mut slot);
            cur = 1 - cur;
        }
        run(&mut enc, &self.compose, cur, 1 - cur, 1, &mut slot);
        cur = 1 - cur;
        run(&mut enc, &self.pack, cur, 1 - cur, 1, &mut slot);
        if !readback {
            self.queue.submit([enc.finish()]);
            self.wait();
            return (Vec::new(), start.elapsed().as_secs_f64() * 1000.0);
        }
        enc.copy_buffer_to_buffer(&self.packed, 0, &self.readback, 0, (w * h * 4) as u64);
        self.queue.submit([enc.finish()]);
        let slice = self.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.wait();
        let gpu_ms = start.elapsed().as_secs_f64() * 1000.0;
        let pixels = slice.get_mapped_range().expect("mapped readback").to_vec();
        self.readback.unmap();
        (pixels, gpu_ms)
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Write RGBA8 pixels as an sRGB PNG.
pub fn write_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> std::io::Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = enc.write_header().map_err(std::io::Error::other)?;
    writer.write_image_data(rgba).map_err(std::io::Error::other)?;
    Ok(())
}
