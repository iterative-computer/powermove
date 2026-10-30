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
    /// Caustics: cell size, photons emitted so far, on (1), photons this pass.
    caustic: [f32; 4],
    /// Scene centre and radius (where sun photons aim).
    bounds: [f32; 4],
}

/// Hashed photon grid slots (8 words each); must match trace.wgsl.
const GRID_CELLS: u64 = 1 << 19;
/// Photons traced per sample batch when caustics are on.
const PHOTONS_PER_PASS: u32 = 1 << 18;

/// Hardware ray tracing: the shared tracer with its `intersect` replaced by a
/// Metal ray query against an acceleration structure built from the same
/// triangles (one instance, so the primitive index is the triangle index).
const RT_WGSL: &str = r#"
@group(1) @binding(0) var tlas: acceleration_structure;

fn intersect(ro: vec3f, rd: vec3f, tmax: f32, any: bool) -> Hit {
  var h = Hit(tmax, -1, vec2f(0.0), 0u);
  if (arrayLength(&pos) < 3u || P.opts.z == 0u) { return h; }
  var rq: ray_query;
  var flags = RAY_FLAG_FORCE_OPAQUE;
  if (any) { flags |= RAY_FLAG_TERMINATE_ON_FIRST_HIT; }
  rayQueryInitialize(&rq, tlas, RayDesc(flags, 0xffu, EPS, tmax, ro, rd));
  rayQueryProceed(&rq);
  let x = rayQueryGetCommittedIntersection(&rq);
  if (x.kind != RAY_QUERY_INTERSECTION_TRIANGLE) { return h; }
  return Hit(x.t, i32(x.primitive_index + x.instance_custom_data), x.barycentrics, x.instance_index);
}
"#;

fn rt_source() -> String {
    let base = include_str!("shaders/trace.wgsl").replacen("fn intersect(ro: vec3f", "fn intersect_bvh(ro: vec3f", 1);
    format!("enable wgpu_ray_query;\n{base}\n{RT_WGSL}")
}

/// Where [`Renderer::present_to`] draws one layer: premultiplied display
/// color into `view` (a render attachment of `format`), only `layer`'s
/// pixels (`None`: every object), the backdrop where `include_void`, rows
/// bottom-up when `flip_y` (GL order, as the native engine's layers are).
pub struct PresentTarget<'a> {
    pub view: &'a wgpu::TextureView,
    pub format: wgpu::TextureFormat,
    pub layer: Option<u32>,
    pub include_void: bool,
    pub flip_y: bool,
}

/// present.wgsl (shared with the app): one layer's pixels into a texture.
struct Present {
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    pipelines: std::collections::HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
}

/// Pipelines and acceleration structures for hardware ray tracing.
struct Rt {
    layout: wgpu::BindGroupLayout,
    trace: wgpu::ComputePipeline,
    photons: wgpu::ComputePipeline,
    group: Option<wgpu::BindGroup>,
    tlas: Option<wgpu::Tlas>,
    blas: Option<wgpu::Blas>,
}

/// Guide images for the Final denoiser, traced with the same camera: first-hit
/// albedo and normal per pixel, averaged over 8 jittered rays. Appended to
/// trace.wgsl (the app never uses it) and bound as group 1.
const AOV_WGSL: &str = r#"
@group(1) @binding(0) var<storage, read_write> aov: array<vec4f>;

@compute @workgroup_size(8, 8)
fn aovs(@builtin(global_invocation_id) gid: vec3u) {
  let w = P.dims.x;
  let h = P.dims.y;
  if (gid.x >= w || gid.y >= h) { return; }
  let idx = gid.y * w + gid.x;
  seed = pcg(idx ^ 0x5bd1e995u);
  var albedo = vec3f(0.0);
  var normal = vec3f(0.0);
  for (var k = 0; k < 8; k++) {
    let ray = camera_ray(vec2f(f32(gid.x), f32(gid.y)) + rnd2());
    let hit = intersect(ray[0], ray[1], BIG, false);
    let tf = floor_t(ray[0], ray[1]);
    if (tf < hit.t) {
      albedo += vec3f(P.env_horizon.w);
      normal += vec3f(0.0, 1.0, 0.0);
    } else if (hit.tri >= 0) {
      let sf = surface(ray[0], ray[1], hit);
      // Glass and mirrors: white, as the denoiser expects for specular surfaces.
      let clear = sf.m.spec.w * (1.0 - sf.m.base.w) > 0.5 || (sf.m.base.w > 0.5 && sf.m.spec.x < 0.1);
      albedo += select(sf.m.base.xyz, vec3f(1.0), clear);
      normal += select(sf.n, -sf.n, dot(sf.n, ray[1]) > 0.0);
    }
  }
  aov[2u * idx] = vec4f(albedo / 8.0, 0.0);
  aov[2u * idx + 1u] = vec4f(normal / 8.0, 0.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PostParams {
    dims: [u32; 4],
    k: [f32; 4],
    backdrop: [f32; 4],
    /// Current and previous cameras for Live's reprojection (unused here).
    cameras: [[f32; 4]; 8],
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
    photons: wgpu::ComputePipeline,
    aovs: wgpu::ComputePipeline,
    aov_layout: wgpu::BindGroupLayout,
    aov: wgpu::Buffer,
    trace_layout: wgpu::BindGroupLayout,
    post_layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    post_params: Vec<wgpu::Buffer>,
    accum: wgpu::Buffer,
    bgsum: wgpu::Buffer,
    gbuf: wgpu::Buffer,
    ebuf: wgpu::Buffer,
    grid: wgpu::Buffer,
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
    bounds: [f32; 4],
    /// Samples accumulated since the last reset.
    pub samples: u32,
    frame: u32,
    /// Live tier: cheaper floor shadows and primary-only mirror coats.
    pub live: bool,
    /// Trace photons for caustics (glass or polished metal in the scene; not Live).
    pub caustics: bool,
    /// Use hardware ray tracing when the GPU has it (Apple M3 and later
    /// accelerate it; M1/M2 run Metal's ray queries in software).
    pub hardware_rt: bool,
    rt: Option<Rt>,
    photons_emitted: f32,
    present: Option<Present>,
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
        let rt_ok = adapter.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY) && std::env::var_os("STUDIO_NO_RT").is_none();
        let have = adapter.limits();
        let mut limits = wgpu::Limits {
            max_storage_buffer_binding_size: have.max_storage_buffer_binding_size,
            max_buffer_size: have.max_buffer_size,
            max_storage_buffers_per_shader_stage: 12,
            max_texture_array_layers: have.max_texture_array_layers,
            ..wgpu::Limits::default()
        };
        if rt_ok {
            limits.max_blas_primitive_count = have.max_blas_primitive_count;
            limits.max_blas_geometry_count = have.max_blas_geometry_count;
            limits.max_tlas_instance_count = have.max_tlas_instance_count;
            limits.max_acceleration_structures_per_shader_stage = have.max_acceleration_structures_per_shader_stage.max(1);
            limits.max_buffers_and_acceleration_structures_per_shader_stage = have.max_buffers_and_acceleration_structures_per_shader_stage;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("pm-studio"),
            required_limits: limits,
            required_features: if rt_ok { wgpu::Features::EXPERIMENTAL_RAY_QUERY } else { wgpu::Features::empty() },
            // SAFETY: ray queries are wgpu's experimental feature; Studio only
            // traces opaque triangles through one instance, the tested path.
            experimental_features: if rt_ok { unsafe { wgpu::ExperimentalFeatures::enabled() } } else { wgpu::ExperimentalFeatures::disabled() },
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        Renderer::with_device(device, queue, adapter.get_info().name, rt_ok, width, height)
    }

    /// The device and queue Studio needs (see [`Renderer::new`]); `limits`
    /// for a host that creates its own device: at least 10 storage buffers
    /// per stage and the adapter's largest bindings.
    pub fn required_limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
        let have = adapter.limits();
        wgpu::Limits {
            max_storage_buffer_binding_size: have.max_storage_buffer_binding_size,
            max_buffer_size: have.max_buffer_size,
            max_storage_buffers_per_shader_stage: have.max_storage_buffers_per_shader_stage.min(12),
            max_texture_array_layers: have.max_texture_array_layers,
            ..wgpu::Limits::default()
        }
    }

    /// Studio on a host's device (the native engine's): no hardware ray
    /// tracing unless the device was created with ray queries (`rt`).
    pub fn with_device(device: wgpu::Device, queue: wgpu::Queue, adapter: String, rt_ok: bool, width: u32, height: u32) -> Result<Renderer, String> {
        if device.limits().max_storage_buffers_per_shader_stage < 10 {
            return Err("Studio needs 10 storage buffers per shader stage".into());
        }
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
        let photons = pipeline(&trace_src, &trace_layout, "photons");
        let rt = rt_ok.then(|| {
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("studio rt"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::AccelerationStructure { vertex_return: false },
                    count: None,
                }],
            });
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("trace rt"), source: wgpu::ShaderSource::Wgsl(rt_source().into()) });
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("trace rt"), bind_group_layouts: &[Some(&trace_layout), Some(&layout)], immediate_size: 0 });
            let make = |entry: &str| {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pl),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            };
            Rt { trace: make("main"), photons: make("photons"), layout, group: None, tlas: None, blas: None }
        });
        let aov_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("studio aov"), entries: &[storage(0, false)] });
        let aov_src = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("aov"),
            source: wgpu::ShaderSource::Wgsl(format!("{}\n{}", include_str!("shaders/trace.wgsl"), AOV_WGSL).into()),
        });
        let aovs = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("aovs"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("aovs"),
                bind_group_layouts: &[Some(&trace_layout), Some(&aov_layout)],
                immediate_size: 0,
            })),
            module: &aov_src,
            entry_point: Some("aovs"),
            compilation_options: Default::default(),
            cache: None,
        });

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
            adapter,
            params: buf("params", std::mem::size_of::<Params>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST),
            post_params: (0..9).map(|_| buf("post params", std::mem::size_of::<PostParams>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST)).collect(),
            accum: buf("accum", pixels * 16, rw),
            bgsum: buf("bgsum", pixels * 16, rw),
            gbuf: buf("gbuf", pixels * 16, rw),
            ebuf: buf("ebuf", pixels * 16, rw),
            grid: buf("photon grid", GRID_CELLS * 32, rw),
            aov: buf("aov", pixels * 32, rw),
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
            photons,
            aovs,
            aov_layout,
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
            bounds: [0.0, 0.0, 0.0, 1.0],
            samples: 0,
            frame: 0,
            live: false,
            caustics: false,
            hardware_rt: true,
            rt,
            photons_emitted: 0.0,
            present: None,
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
        let extra = if self.rt.is_some() { wgpu::BufferUsages::BLAS_INPUT } else { wgpu::BufferUsages::empty() };
        let init = |label: &str, data: &[u8]| {
            let data: Vec<u8> = if data.is_empty() { vec![0u8; 64] } else { data.to_vec() };
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: &data, usage: wgpu::BufferUsages::STORAGE | extra })
        };
        self.scene_buffers = vec![
            init("tris", bytemuck::cast_slice(&prepared.tris)),
            init("nodes", bytemuck::cast_slice(&prepared.nodes)),
            init("materials", bytemuck::cast_slice(&prepared.materials)),
            init("scene data", bytemuck::cast_slice(&prepared.sdata)),
            init("positions", bytemuck::cast_slice(&prepared.positions)),
        ];
        self.light_count = prepared.lights.len() as u32;
        self.trace_group = None;
        self.build_rt(prepared.positions.len() as u32);
        // Caustics only where something can focus light: glass or polished metal.
        self.caustics = prepared.materials.iter().any(|m| m.spec[3] * (1.0 - m.base[3]) > 0.5 || (m.base[3] > 0.5 && m.spec[0] < 0.25));
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &prepared.positions {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let centre = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
        let radius = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt().max(1e-3) * 0.5;
        self.bounds = [centre[0], centre[1], centre[2], radius];
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
            caustic: [0.0; 4],
            bounds: self.bounds,
        };
        self.exposure = scene.exposure;
        // Product scenes show their backdrop; the app's layers leave the void transparent.
        self.backdrop = scene.backdrop.to4(1.0);
        self.samples = 0;
    }

    /// Whether the GPU and wgpu offer ray queries.
    pub fn has_hardware_rt(&self) -> bool {
        self.rt.is_some()
    }

    /// Whether traces use hardware ray tracing (the GPU has it, it is on and
    /// the scene has geometry).
    pub fn uses_hardware_rt(&self) -> bool {
        self.hardware_rt && self.rt.as_ref().is_some_and(|rt| rt.group.is_some())
    }

    /// The acceleration structures over the scene's triangles (corners in
    /// `scene_buffers[4]`, 16 bytes apart).
    fn build_rt(&mut self, vertices: u32) {
        let Some(rt) = self.rt.as_mut() else { return };
        rt.group = None;
        rt.tlas = None;
        rt.blas = None;
        if vertices < 3 {
            return;
        }
        let size = wgpu::BlasTriangleGeometrySizeDescriptor { vertex_format: wgpu::VertexFormat::Float32x3, vertex_count: vertices, index_format: None, index_count: None, flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE };
        let blas = self.device.create_blas(
            &wgpu::CreateBlasDescriptor { label: Some("studio blas"), flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build },
            wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: vec![size.clone()] },
        );
        let mut tlas = self.device.create_tlas(&wgpu::CreateTlasDescriptor { label: Some("studio tlas"), max_instances: 1, flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build });
        tlas[0] = Some(wgpu::TlasInstance::new(&blas, [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0], 0, 0xff));
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.build_acceleration_structures(
            std::iter::once(&wgpu::BlasBuildEntry {
                blas: &blas,
                geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                    size: &size,
                    vertex_buffer: &self.scene_buffers[4],
                    first_vertex: 0,
                    vertex_stride: 16,
                    index_buffer: None,
                    first_index: None,
                    transform_buffer: None,
                    transform_buffer_offset: None,
                }]),
            }),
            std::iter::once(&tlas),
        );
        self.queue.submit([enc.finish()]);
        rt.group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("studio rt"),
            layout: &rt.layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: tlas.as_binding() }],
        }));
        rt.tlas = Some(tlas);
        rt.blas = Some(blas);
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
                    wgpu::BindGroupEntry { binding: 12, resource: self.grid.as_entire_binding() },
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
        // Seeds follow the sample index, so a scene and view render the same every time.
        if reset {
            self.frame = 0;
            self.photons_emitted = 0.0;
        }
        let photons = self.caustics && !self.live && self.light_count > 0;
        let mut left = spp;
        let mut first = true;
        while left > 0 {
            let n = left.min(4);
            self.frame = self.frame.wrapping_add(1);
            let mut p = self.base;
            p.dims = [w, h, self.frame, n];
            p.counts = [self.light_count, bounces, if reset && first { 1 } else { 0 }, clamp];
            p.opts = [u32::from(self.live), 0, 1, 0];
            let group = self.trace_group().clone();
            let mut enc = self.device.create_command_encoder(&Default::default());
            if photons {
                // Photons first, so this batch's camera paths see them.
                if reset && first {
                    enc.clear_buffer(&self.grid, 0, None);
                }
                p.caustic = [self.bounds[3] / 160.0, self.photons_emitted, 1.0, PHOTONS_PER_PASS as f32];
                self.queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
                {
                    let mut pass = enc.begin_compute_pass(&Default::default());
                    match (self.uses_hardware_rt(), self.rt.as_ref()) {
                        (true, Some(rt)) => {
                            pass.set_pipeline(&rt.photons);
                            pass.set_bind_group(1, rt.group.as_ref(), &[]);
                        }
                        _ => pass.set_pipeline(&self.photons),
                    }
                    pass.set_bind_group(0, &group, &[]);
                    pass.dispatch_workgroups(PHOTONS_PER_PASS.div_ceil(64), 1, 1);
                }
                self.queue.submit([enc.finish()]);
                enc = self.device.create_command_encoder(&Default::default());
                self.photons_emitted += PHOTONS_PER_PASS as f32;
                p.caustic[1] = self.photons_emitted;
            }
            self.queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                match (self.uses_hardware_rt(), self.rt.as_ref()) {
                    (true, Some(rt)) => {
                        pass.set_pipeline(&rt.trace);
                        pass.set_bind_group(1, rt.group.as_ref(), &[]);
                    }
                    _ => pass.set_pipeline(&self.trace),
                }
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
        self.post(passes, true, None, None)
    }

    /// The void (where no object is hit): the backdrop color when `Some`
    /// (display-referred sRGB), transparent when `None`.
    pub fn set_void(&mut self, backdrop: Option<[f32; 3]>) {
        self.backdrop = match backdrop {
            Some(c) => [c[0], c[1], c[2], 1.0],
            None => [0.0, 0.0, 0.0, 0.0],
        };
    }

    /// Resolve and compose the accumulation (with `passes` à-trous
    /// iterations, 0 for Refine/Final), then draw one layer into a texture
    /// (see [`PresentTarget`]). Returns ms.
    pub fn present_to(&mut self, passes: u32, target: PresentTarget<'_>) -> f64 {
        self.ensure_present(target.format);
        self.post(passes, false, None, Some(target)).1
    }

    fn ensure_present(&mut self, format: wgpu::TextureFormat) {
        let device = &self.device;
        let present = self.present.get_or_insert_with(|| {
            let entry = |binding: u32, ty: wgpu::BufferBindingType| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            };
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("studio present"),
                entries: &[entry(0, wgpu::BufferBindingType::Uniform), entry(1, wgpu::BufferBindingType::Storage { read_only: true }), entry(2, wgpu::BufferBindingType::Storage { read_only: true })],
            });
            let params = device.create_buffer(&wgpu::BufferDescriptor { label: Some("present params"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            Present { layout, params, pipelines: Default::default() }
        });
        if present.pipelines.contains_key(&format) {
            return;
        }
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("present.wgsl"), source: wgpu::ShaderSource::Wgsl(include_str!("shaders/present.wgsl").into()) });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("present"), bind_group_layouts: &[Some(&present.layout)], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("present"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[Some(format.into())] }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        present.pipelines.insert(format, pipeline);
    }

    /// Final with a learned denoiser: the resolved image, guided by first-hit
    /// albedo and normals, goes through `denoise` (color, albedo, normal; RGBA
    /// f32, RGB read) before it is composed. The backdrop's shadow-catcher
    /// shading is denoised the same way. Returns (pixels, ms).
    pub fn finish_with(&mut self, denoise: &dyn Fn(&mut [f32], &[f32], &[f32]) -> Result<(), String>) -> (Vec<u8>, f64) {
        self.post(0, true, Some(denoise), None)
    }

    /// Linear scene radiance per pixel (RGBA f32; A is the backdrop's
    /// shading), before exposure and tone mapping: for comparing against
    /// reference renderers.
    pub fn radiance(&mut self) -> Vec<f32> {
        let (w, h) = (self.width, self.height);
        let params = PostParams { dims: [w, h, 1, 0], k: [self.exposure, 1.6, 0.0, 0.0], backdrop: self.backdrop, cameras: [[0.0; 4]; 8] };
        self.queue.write_buffer(&self.post_params[0], 0, bytemuck::bytes_of(&params));
        let group = self.post_group(0, 1, 0);
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.resolve);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        self.queue.submit([enc.finish()]);
        self.read_f32(&self.display[0], (w * h) as u64 * 16)
    }

    fn read_f32(&self, buffer: &wgpu::Buffer, bytes: u64) -> Vec<f32> {
        let rb = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("f32 readback"), size: bytes, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(buffer, 0, &rb, 0, bytes);
        self.queue.submit([enc.finish()]);
        let slice = rb.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.wait();
        let out = bytemuck::cast_slice(&slice.get_mapped_range().expect("mapped readback")).to_vec();
        rb.unmap();
        out
    }

    /// GPU time of resolve + denoise + compose + pack alone (what a viewer
    /// frame costs; it presents the result instead of reading it back).
    pub fn post_only(&mut self, passes: u32) -> f64 {
        self.post(passes, false, None, None).1
    }

    #[allow(clippy::type_complexity)]
    fn post(&mut self, passes: u32, readback: bool, denoise: Option<&dyn Fn(&mut [f32], &[f32], &[f32]) -> Result<(), String>>, present: Option<PresentTarget<'_>>) -> (Vec<u8>, f64) {
        let start = Instant::now();
        let (w, h) = (self.width, self.height);
        let groups = (w.div_ceil(8), h.div_ceil(8));
        let mut enc = self.device.create_command_encoder(&Default::default());
        let mut slot = 0;
        let run = |enc: &mut wgpu::CommandEncoder, pipe: &wgpu::ComputePipeline, src: usize, dst: usize, step: u32, slot: &mut usize| {
            let params = PostParams { dims: [w, h, step, 0], k: [self.exposure, 1.6, 0.0, 0.0], backdrop: self.backdrop, cameras: [[0.0; 4]; 8] };
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
        if let Some(denoise) = denoise {
            // Guides, then read back, denoise on the CPU and upload the result.
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("aov"),
                layout: &self.aov_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: self.aov.as_entire_binding() }],
            });
            let mut p = self.base;
            p.dims = [w, h, 0, 1];
            p.opts = [0, 0, 1, 0];
            p.counts = [self.light_count, 6, 1, 64];
            self.queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
            let trace_group = self.trace_group.clone().expect("traced before finishing");
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.aovs);
                pass.set_bind_group(0, &trace_group, &[]);
                pass.set_bind_group(1, &group, &[]);
                pass.dispatch_workgroups(groups.0, groups.1, 1);
            }
            self.queue.submit([std::mem::replace(&mut enc, self.device.create_command_encoder(&Default::default())).finish()]);
            let pixels = (w * h) as usize;
            let mut color = self.read_f32(&self.display[0], pixels as u64 * 16);
            let aov = self.read_f32(&self.aov, pixels as u64 * 32);
            let (mut albedo, mut normal) = (vec![0f32; pixels * 4], vec![0f32; pixels * 4]);
            let mut shade = vec![0f32; pixels * 4];
            for i in 0..pixels {
                albedo[i * 4..i * 4 + 4].copy_from_slice(&aov[i * 8..i * 8 + 4]);
                normal[i * 4..i * 4 + 4].copy_from_slice(&aov[i * 8 + 4..i * 8 + 8]);
                shade[i * 4..i * 4 + 3].fill(color[i * 4 + 3]);
            }
            if let Err(e) = denoise(&mut color, &albedo, &normal).and_then(|_| denoise(&mut shade, &albedo, &normal)) {
                eprintln!("denoise failed, keeping the noisy frame: {e}");
            } else {
                for i in 0..pixels {
                    color[i * 4 + 3] = shade[i * 4];
                }
                self.queue.write_buffer(&self.display[0], 0, bytemuck::cast_slice(&color));
            }
        }
        let mut cur = 0;
        for i in 0..passes.min(5) {
            run(&mut enc, &self.atrous, cur, 1 - cur, 1 << i, &mut slot);
            cur = 1 - cur;
        }
        run(&mut enc, &self.compose, cur, 1 - cur, 1, &mut slot);
        cur = 1 - cur;
        if let (Some(t), Some(p)) = (present, self.present.as_ref()) {
            let flags = u32::from(t.flip_y) | (u32::from(t.include_void) << 1);
            self.queue.write_buffer(&p.params, 0, bytemuck::cast_slice(&[w, h, t.layer.map_or(0, |l| l + 1), flags]));
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("present"),
                layout: &p.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: p.params.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: self.display[cur].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: self.gbuf.as_entire_binding() },
                ],
            });
            {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("studio present"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: t.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(&p.pipelines[&t.format]);
                pass.set_bind_group(0, &group, &[]);
                pass.draw(0..3, 0..1);
            }
            self.queue.submit([enc.finish()]);
            self.wait();
            return (Vec::new(), start.elapsed().as_secs_f64() * 1000.0);
        }
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

    /// The device Studio renders on (its own, or the host's).
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
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
