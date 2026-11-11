//! The GPU side: one path-tracing pipeline, resolve, denoise and pack.

use crate::scene::{Prepared, Scene};
use std::time::Instant;
use wgpu::util::DeviceExt;
mod resampling;
mod query;
pub use query::SurfacePick;
#[cfg(all(feature="metalfx",target_os="macos"))]
mod metalfx;

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
    iris: [f32; 4],
    ambient:[f32;4],
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
  rayQueryInitialize(&rq, tlas, RayDesc(flags, 0xffu, ray_epsilon(), tmax, ro, rd));
  rayQueryProceed(&rq);
  let x = rayQueryGetCommittedIntersection(&rq);
  if (x.kind != RAY_QUERY_INTERSECTION_TRIANGLE) { return h; }
  return Hit(x.t, i32(x.primitive_index + x.instance_custom_data), x.barycentrics, x.instance_index);
}
"#;

fn trace_source() -> String {
    // Native Live uses a uniform frame jitter, as temporal denoisers require.
    // Keep it out of the shared source: opts.w also carries camera clip bits.
    include_str!("shaders/trace.wgsl").replacen("    var jitter = rnd2();", "    var jitter = rnd2();\n    if (P.opts.x == 1u) {jitter=vec2f(f32(pcg(P.dims.z ^ 0x68bc21ebu)),f32(pcg(P.dims.z ^ 0x02e5be93u)))*(1.0/4294967296.0);}",1)
}
fn rt_source() -> String {
    let base = trace_source().replacen("fn intersect(ro: vec3f", "fn intersect_bvh(ro: vec3f", 1);
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
    aovs: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    trace: wgpu::ComputePipeline,
    resampled: wgpu::ComputePipeline,
    photons: wgpu::ComputePipeline,
    group: Option<wgpu::BindGroup>,
    tlas: Option<wgpu::Tlas>,
    blas: Vec<wgpu::Blas>,
}

/// Guide images for the Final denoiser (aov.wgsl), bound as group 1 here;
/// the app binds them in group 0 beside the scene.
const AOV_WGSL: &str = concat!("@group(1) @binding(0) var<storage, read_write> aov: array<vec4f>;\n", include_str!("shaders/aov.wgsl"));

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PostParams {
    dims: [u32; 4],
    k: [f32; 4],
    backdrop: [f32; 4],
    /// Current and previous cameras for Live's reprojection (unused here).
    cameras: [[f32; 4]; 8],
}

/// One resident native acceleration structure per mesh.
#[derive(Clone, Debug)]
pub struct RtGeometry { pub first_vertex: u32, pub vertex_count: u32 }
/// Metal/wgpu row-major 3×4 object-to-world transform and global triangle offset.
#[derive(Clone, Debug)]
pub struct RtPlacement { pub geometry: usize, pub transform: [f32;12], pub triangle_offset: u32 }

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
    temporal: wgpu::ComputePipeline,
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
    history: [wgpu::Buffer; 2],
    history_index: usize,
    history_valid: bool,
    previous_camera: [[f32;4];4],
    readback: wgpu::Buffer,
    screen: wgpu::Texture,
    sampler: wgpu::Sampler,
    trace_group: Option<wgpu::BindGroup>,
    scene_buffers: Vec<wgpu::Buffer>,
    light_count: u32,
    instance_count: u32,
    output_layer: Option<u32>,
    output_include_void: bool,
    straight_output: bool,
    last_display: usize,
    seed_salt: u32,
    base: Params,
    exposure: f32,
    backdrop: [f32; 4],
    bounds: [f32; 4],
    /// Samples accumulated since the last reset.
    pub samples: u32,
    frame: u32,
    /// Live tier: cheaper floor shadows and primary-only mirror coats.
    pub live: bool,
    /// Conservative post-OIDN history for sequential Final export frames.
    pub quality_temporal: bool,
    /// Trace photons for caustics (glass or polished metal in the scene; not Live).
    pub caustics: bool,
    /// Per-render switch; capability detection stays attached to the scene.
    pub caustics_enabled: bool,
    /// Build a fixed photon estimate before camera sampling, so error stopping
    /// cannot freeze pixels at different stages of a filling map.
    pub precompute_caustics: bool,
    /// OIDN auxiliary ray budget (prefiltering still runs once per frame).
    pub guide_samples: u32,
    adaptive_enabled: bool,
    pub budget_batch_samples: u32,
    shutter_guides: bool,
    /// Use hardware ray tracing when the GPU has it (Apple M3 and later
    /// accelerate it; M1/M2 run Metal's ray queries in software).
    pub hardware_rt: bool,
    rt: Option<Rt>,
    resampling:Option<resampling::Resampling>,
    query:Option<query::Query>,
    #[cfg(all(feature="metalfx",target_os="macos"))]
    metalfx:Option<metalfx::MetalFX>,
    pub metalfx_enabled:bool,
    pub last_surface:Option<u64>,
    pub resampled_lighting:bool,
    photons_emitted: f32,
    photon_bounces: u32,
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
    #[cfg(not(all(feature="metalfx",target_os="macos")))]
    pub fn allocated_gpu_bytes(&self)->Option<u64>{None}
    pub fn new(width: u32, height: u32) -> Result<Renderer, String> {
        crate::validation::target_bytes(width,height,u64::MAX)?;
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
            max_storage_textures_per_shader_stage: have.max_storage_textures_per_shader_stage.min(8),
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
            max_storage_textures_per_shader_stage: have.max_storage_textures_per_shader_stage.min(8),
            max_texture_array_layers: have.max_texture_array_layers,
            ..wgpu::Limits::default()
        }
    }

    /// Studio on a host's device (the native engine's): no hardware ray
    /// tracing unless the device was created with ray queries (`rt`).
    pub fn with_device(device: wgpu::Device, queue: wgpu::Queue, adapter: String, rt_ok: bool, width: u32, height: u32) -> Result<Renderer, String> {
        crate::validation::target_bytes(width,height,u64::from(device.limits().max_storage_buffer_binding_size))?;
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
            entries: &[uniform(0), storage(1, true), storage(2, true), storage(3, true), storage(4, true), storage(5, false), storage(6, false), storage(7, true), storage(8, true), storage(9, false), storage(10, true)],
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
        let trace_src = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("trace.wgsl"), source: wgpu::ShaderSource::Wgsl(trace_source().into()) });
        let post_src = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("post.wgsl"), source: wgpu::ShaderSource::Wgsl(include_str!("shaders/post.wgsl").into()) });
        let trace = pipeline(&trace_src, &trace_layout, "main");
        let resolve = pipeline(&post_src, &post_layout, "resolve");
        let compose = pipeline(&post_src, &post_layout, "compose");
        let atrous = pipeline(&post_src, &post_layout, "atrous");
        let pack = pipeline(&post_src, &post_layout, "pack");
        let temporal = pipeline(&post_src, &post_layout, "temporal");
        let photons = pipeline(&trace_src, &trace_layout, "photons");
        let resampling=rt_ok.then(||resampling::Resampling::new(&device,width,height));
        let aov_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("studio aov"), entries: &[storage(0, false)] });
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
            let ris_src=resampling::source(&rt_source());
            let ris_module=device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("trace reservoirs"),source:wgpu::ShaderSource::Wgsl(ris_src.into())});
            let ris_layout=device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{label:Some("trace reservoirs"),bind_group_layouts:&[Some(&trace_layout),Some(&layout),Some(&resampling.as_ref().unwrap().layout)],immediate_size:0});
            let resampled=device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor{label:Some("trace reservoirs"),layout:Some(&ris_layout),module:&ris_module,entry_point:Some("main"),compilation_options:Default::default(),cache:None});
            let guide_source = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("aovs rt"),
                source: wgpu::ShaderSource::Wgsl(format!("{}\n{}", rt_source(), AOV_WGSL.replacen("@group(1)", "@group(2)", 1)).into()),
            });
            let guide_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("aovs rt"), bind_group_layouts: &[Some(&trace_layout), Some(&layout), Some(&aov_layout)], immediate_size: 0,
            });
            let aovs = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("aovs rt"), layout: Some(&guide_layout), module: &guide_source,
                entry_point: Some("aovs"), compilation_options: Default::default(), cache: None,
            });
            Rt { aovs, trace: make("main"), resampled, photons: make("photons"), layout, group: None, tlas: None, blas: Vec::new() }
        });
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

        let metalfx_ok=device.limits().max_storage_textures_per_shader_stage>=8;
        let pixels = (width * height) as u64;
        let buf = |label: &str, size: u64, usage: wgpu::BufferUsages| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
        let rw = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        let screen = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("screen"),
            size: wgpu::Extent3d { width: 4, height: 4, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
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
            history: [buf("history a", 64, rw), buf("history b", 64, rw)],
            history_index: 0, history_valid: false, previous_camera: [[0.;4];4],
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
            temporal,
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
            instance_count: 1,
            output_layer: None, output_include_void: true, straight_output:false, last_display:0, seed_salt:0,
            base: bytemuck::Zeroable::zeroed(),
            exposure: 1.0,
            backdrop: [1.0; 4],
            bounds: [0.0, 0.0, 0.0, 1.0],
            samples: 0,
            frame: 0,
            live: false,
            quality_temporal: false,
            caustics: false,
            caustics_enabled: true,
            precompute_caustics: false,
            guide_samples: 64,
            adaptive_enabled: false,
            budget_batch_samples: 4,
            shutter_guides: false,
            hardware_rt: true,
            rt,
            #[cfg(all(feature="metalfx",target_os="macos"))] metalfx:None,
            last_surface:None,
            metalfx_enabled:metalfx_ok&&cfg!(all(feature="metalfx",target_os="macos"))&&std::env::var_os("STUDIO_NO_METALFX").is_none(),
            query:None,
            resampling, resampled_lighting: std::env::var_os("STUDIO_NO_RIS").is_none(),
            photons_emitted: 0.0,
            photon_bounces: 0,
            present: None,
        };
        Ok(r)
    }

    /// Resize image storage without rebuilding the device, shaders, scene or
    /// acceleration structures. Live's adaptive budget uses this between frames.
    pub fn resize(&mut self, width:u32, height:u32)->Result<bool,String> {
        crate::validation::target_bytes(width,height,u64::from(self.device.limits().max_storage_buffer_binding_size))?;
        if (width,height)==(self.width,self.height) {return Ok(false);}
        let pixels=u64::from(width)*u64::from(height);
        // Drop the bindings before replacing their buffers, so they cannot
        // keep a complete old target alive alongside the replacement.
        self.trace_group=None;
        let rw=wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::COPY_SRC;
        let buffer=|label,size,usage|self.device.create_buffer(&wgpu::BufferDescriptor{label:Some(label),size,usage,mapped_at_creation:false});
        self.accum=buffer("accum",pixels*16,rw);
        self.bgsum=buffer("bgsum",pixels*16,rw);
        self.gbuf=buffer("gbuf",pixels*16,rw);
        self.ebuf=buffer("ebuf",pixels*16,rw);
        self.aov=buffer("aov",pixels*32,rw);
        self.display=[buffer("display a",pixels*16,rw),buffer("display b",pixels*16,rw)];
        self.packed=buffer("packed",pixels*4,rw);
        self.readback=buffer("readback",pixels*4,wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST);
        self.history=[buffer("history a",64,rw),buffer("history b",64,rw)];
        self.width=width;self.height=height;self.samples=0;self.history_valid=false;self.history_index=0;
        Ok(true)
    }

    /// The display composition, sRGB RGBA8.
    pub fn set_screen(&mut self, rgba: &[u8], w: u32, h: u32) {
        self.photons_emitted = 0.;
        self.screen = self.device.create_texture_with_data(
            &self.queue,
            &wgpu::TextureDescriptor {
                label: Some("screen"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
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
        self.photons_emitted = 0.;
        let extra = if self.rt.is_some() { wgpu::BufferUsages::BLAS_INPUT } else { wgpu::BufferUsages::empty() };
        let init = |label: &str, data: &[u8]| {
            let data: Vec<u8> = if data.is_empty() { vec![0u8; std::mem::size_of::<crate::scene::GpuMaterial>()] } else { data.to_vec() };
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: &data, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | extra })
        };
        self.history_valid = false;
        self.instance_count = prepared.instancing.as_ref().map_or(1,|i|i.placements.len() as u32);
        self.scene_buffers = vec![
            init("tris", bytemuck::cast_slice(&prepared.tris)),
            init("nodes", bytemuck::cast_slice(&prepared.nodes)),
            init("materials", bytemuck::cast_slice(&prepared.materials)),
            init("scene data", bytemuck::cast_slice(&prepared.sdata)),
            init("positions", bytemuck::cast_slice(&prepared.positions)),
        ];
        self.light_count = prepared.lights.len() as u32;
        self.trace_group = None;
        if let Some(i)=&prepared.instancing {self.build_instanced_rt(&i.geometries,&i.placements,true);}else{self.build_rt(prepared.positions.len() as u32);}
        // Caustics only where something can focus light: glass or polished metal.
        self.caustics = prepared.materials.iter().any(|m| m.spec[3] * (1.0 - m.base[3]) > 0.5 || (m.base[3] > 0.5 && m.spec[0] < 0.25));
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &prepared.positions {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        if prepared.instancing.is_some() {
            lo=[f32::MAX;3];hi=[f32::MIN;3];let n=prepared.nodes[0];
            for (a,b,count) in [(n.lmin,n.lmax,n.lcount),(n.rmin,n.rmax,n.rcount)] {if count!=u32::MAX {for k in 0..3 {lo[k]=lo[k].min(a[k]);hi[k]=hi[k].max(b[k]);}}}
        }
        if prepared.positions.is_empty(){lo=[-1.;3];hi=[1.;3];}
        let centre = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
        let radius = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt().max(1e-3) * 0.5;
        self.bounds = [centre[0], centre[1], centre[2], radius];
        self.set_camera(scene);
    }

    /// Native host upload. Geometry stays resident across camera/transform edits.
    /// `top_nodes` is the mutable TLAS prefix; BLAS nodes and triangles are immutable.
    pub fn set_instanced_scene(&mut self, scene: &Scene, prepared: &Prepared,
        geometries: &[RtGeometry], placements: &[RtPlacement], top_nodes: usize,
        geometry_changed: bool) {
        self.photons_emitted = 0.;
        if geometry_changed || self.scene_buffers.is_empty() {
            self.set_scene(scene, prepared);
        } else {
            self.queue.write_buffer(&self.scene_buffers[1], 0, bytemuck::cast_slice(&prepared.nodes[..top_nodes]));
            self.queue.write_buffer(&self.scene_buffers[2], 0, bytemuck::cast_slice(&prepared.materials));
            self.queue.write_buffer(&self.scene_buffers[3], 0, bytemuck::cast_slice(&prepared.sdata));
            self.light_count = prepared.lights.len() as u32;
            self.set_camera(scene);
        }
        self.instance_count = placements.len() as u32;
        self.caustics=prepared.materials.iter().any(|m|m.spec[3]*(1.-m.base[3])>0.5 || (m.base[3]>0.5 && m.spec[0]<0.25));
        self.build_instanced_rt(geometries, placements, geometry_changed);
    }

    fn build_instanced_rt(&mut self, geometries: &[RtGeometry], placements: &[RtPlacement], rebuild: bool) {
        let Some(rt) = self.rt.as_mut() else { return };
        if geometries.is_empty() || placements.is_empty() { rt.group = None; return; }
        let mut enc = self.device.create_command_encoder(&Default::default());
        let sizes: Vec<_> = geometries.iter().map(|g| wgpu::BlasTriangleGeometrySizeDescriptor {
            vertex_format: wgpu::VertexFormat::Float32x3, vertex_count: g.vertex_count,
            index_format: None, index_count: None, flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
        }).collect();
        if rebuild || rt.blas.len() != geometries.len() {
            rt.blas = sizes.iter().map(|size| self.device.create_blas(
                &wgpu::CreateBlasDescriptor { label: Some("studio mesh blas"),
                    flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build },
                wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: vec![size.clone()] },
            )).collect();
            let entries: Vec<_> = rt.blas.iter().zip(sizes.iter()).zip(geometries.iter()).map(|((blas, size), g)| wgpu::BlasBuildEntry {
                blas, geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                    size, vertex_buffer: &self.scene_buffers[4], first_vertex: g.first_vertex,
                    vertex_stride: 16, index_buffer: None, first_index: None, transform_buffer: None, transform_buffer_offset: None,
                }]),
            }).collect();
            // Metal's acceleration builder can overlap work using the same
            // scratch allocation. Separate submissions prevent a later BLAS
            // build from overwriting an earlier one (chrome's glass sphere).
            // These builds happen on mesh changes; dragging reuses every BLAS.
            for entry in &entries {
                let mut build=self.device.create_command_encoder(&Default::default());
                build.build_acceleration_structures([entry],std::iter::empty::<&wgpu::Tlas>());
                self.queue.submit([build.finish()]);
                let _=self.device.poll(wgpu::PollType::wait_indefinitely());
            }
        }
        let mut tlas = self.device.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some("studio instances"), max_instances: placements.len() as u32,
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        });
        for (i, p) in placements.iter().enumerate() {
            tlas[i] = Some(wgpu::TlasInstance::new(&rt.blas[p.geometry], p.transform, p.triangle_offset, 0xff));
        }
        enc.build_acceleration_structures(std::iter::empty::<&wgpu::BlasBuildEntry<'_>>(), [&tlas]);
        self.queue.submit([enc.finish()]);
        rt.group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("studio rt"), layout: &rt.layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: tlas.as_binding() }],
        }));
        rt.tlas = Some(tlas);
    }

    /// Update the already serialized camera basis, keeping the editor's camera roll.
    pub fn set_view(&mut self, camera: [[f32; 4]; 4], ground: [f32;4], shadows: bool, bounds: [f32;4]) {
        self.base.cam_pos = camera[0]; self.base.cam_fwd = camera[1];
        self.base.cam_right = camera[2]; self.base.cam_up = camera[3];
        self.base.ground = ground; self.base.opts[1] = u32::from(!shadows);
        self.base.bounds = bounds; self.bounds = bounds; self.samples = 0;
    }

    pub fn set_ambient(&mut self,ambient:[f32;3]){self.base.ambient=[ambient[0],ambient[1],ambient[2],0.];self.samples=0;}
    pub fn set_iris(&mut self,iris:[f32;4]){self.base.iris=iris;self.samples=0;}

    /// Camera, backdrop and environment only (no geometry upload).
    pub fn set_camera(&mut self, scene: &Scene) {
        let c = &scene.camera;
        let (fwd,right,up)=c.basis();
        let tan_y = (c.fov.to_radians() * 0.5).tan();
        let aspect = self.width as f32 / self.height as f32;
        let lens_radius = c.aperture_radius(scene.meters_per_unit);
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
            iris: [0.,0.,0.,1.],
            ambient:[0.;4],
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
        rt.blas.clear();
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
        rt.blas = vec![blas];
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

    /// Override the packed world's adaptive policy for a quality request. This
    /// also covers the Electron binary upload path, which bypasses Scene.world.
    pub fn set_adaptive_sampling(&mut self, settings: crate::environment::AdaptiveSampling) {
        self.set_adaptive_sampling_with_floor(settings, false);
    }
    /// Saved quality owns its floor; legacy callers retain the caustic safety floor.
    pub fn set_adaptive_sampling_with_floor(&mut self, settings: crate::environment::AdaptiveSampling, explicit_floor: bool) {
        self.adaptive_enabled = settings.enabled;
        let row = 4 * self.light_count as u64 + 9 * self.instance_count as u64 + 5;
        if self.scene_buffers[3].size() >= (row + 1) * 16 {
            let values = [if settings.enabled {if explicit_floor {2.} else {1.}} else {0.}, settings.min_samples as f32, settings.relative_error, settings.absolute_error];
            self.queue.write_buffer(&self.scene_buffers[3], row * 16, bytemuck::cast_slice(&values));
        }
    }

    /// Trace `spp` samples per pixel with up to `bounces` bounces; `reset`
    /// starts a new accumulation. Returns GPU wall time in ms.
    pub fn trace(&mut self, spp: u32, bounces: u32, reset: bool, clamp: u32) -> f64 {
        self.trace_interruptible(spp,bounces,reset,clamp,&||false)
    }
    /// Refine may yield to new input between the existing four-sample GPU
    /// submissions. Uninterrupted sampling order and Final output are identical.
    pub fn trace_interruptible(&mut self,spp:u32,bounces:u32,reset:bool,clamp:u32,cancelled:&dyn Fn()->bool)->f64 {
        self.trace_budgeted_interruptible(spp, bounces, reset, clamp, 0, 0, cancelled)
    }
    /// The tracing deadline is checked between submissions. The sample minimum
    /// controls error stopping; the deadline can preempt it.
    /// No extra image buffers or GPU allocations are needed for adaptive work.
    pub fn trace_budgeted_interruptible(&mut self, spp:u32, bounces:u32, reset:bool, clamp:u32, min_samples:u32, budget_ms:u64, cancelled:&dyn Fn()->bool)->f64 {
        let (w, h) = (self.width, self.height);
        // Several dispatches of at most 4 samples keep each submission short.
        let start = Instant::now();
        // Seeds follow the sample index, so a scene and view render the same every time.
        if reset {
            if !self.live { self.frame = self.seed_salt.wrapping_mul(4099); }
            if !self.precompute_caustics { self.photons_emitted = 0.0; }
        }
        if self.photon_bounces != bounces {self.photons_emitted=0.;self.photon_bounces=bounces;}
        let photons = self.caustics && self.caustics_enabled && !self.live && self.light_count > 0;
        let mut left = spp;
        let mut first = true;
        while left > 0 {
            if cancelled(){break;}
            if !first && budget_ms > 0 && start.elapsed().as_millis() >= budget_ms as u128 { break; }
            let n = left.min(if budget_ms>0 {self.budget_batch_samples.clamp(1,4)} else {4});
            self.frame = self.frame.wrapping_add(1);
            let mut p = self.base;
            p.dims = [w, h, self.frame, n];
            p.counts = [self.light_count, bounces, if reset && first { 1 } else { 0 }, clamp];
            p.opts = [u32::from(self.live), self.base.opts[1], self.instance_count, self.base.opts[3]];
            let group = self.trace_group().clone();
            let mut enc = self.device.create_command_encoder(&Default::default());
            if photons && (!self.precompute_caustics || self.photons_emitted == 0.) {
                // Photons first, so this batch's camera paths see them.
                if reset && first || self.photons_emitted == 0. {
                    enc.clear_buffer(&self.grid, 0, None);
                }
                // Bootstrap the map before the first camera sample. Sparse
                // first maps otherwise survive in every accumulated estimate.
                let photon_count = if self.precompute_caustics { PHOTONS_PER_PASS * 32 } else if reset && first { PHOTONS_PER_PASS * 4 } else { PHOTONS_PER_PASS };
                p.caustic = [self.bounds[3] / 160.0, self.photons_emitted, 1.0, photon_count as f32];
                if self.precompute_caustics {p.dims[2]=1;}
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
                    pass.dispatch_workgroups(photon_count.div_ceil(64).min(4096), photon_count.div_ceil(64 * 4096), 1);
                }
                self.queue.submit([enc.finish()]);
                enc = self.device.create_command_encoder(&Default::default());
                self.photons_emitted += photon_count as f32;
                p.caustic[1] = self.photons_emitted;
            }
            p.dims[2]=self.frame;
            if photons && self.precompute_caustics { p.caustic = [self.bounds[3] / 160., self.photons_emitted, 2., 0.]; }
            self.queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
            let use_reservoirs=self.live && self.resampled_lighting && self.uses_hardware_rt();
            let reservoirs=use_reservoirs.then(||self.reservoir_group());
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                match (self.uses_hardware_rt(), self.rt.as_ref()) {
                    (true, Some(rt)) => {
                        if let Some(group)=&reservoirs {pass.set_pipeline(&rt.resampled);pass.set_bind_group(2,group,&[]);}else{pass.set_pipeline(&rt.trace);}
                        pass.set_bind_group(1, rt.group.as_ref(), &[]);
                    }
                    _ => pass.set_pipeline(&self.trace),
                }
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
            }
            self.queue.submit([enc.finish()]);
            self.wait();
            if use_reservoirs {let r=self.resampling.as_mut().unwrap();r.index=1-r.index;}
            if reset && first {
                self.samples = 0;
            }
            self.samples += n;
            left -= n;
            first = false;
            // Pixels keep their actual counts when the shared shader stops
            // them. A bounded occasional read avoids dispatching the entire
            // remaining ceiling after every pixel has converged.
            if self.adaptive_enabled && self.samples >= min_samples && self.samples % 32 == 0
                && self.sample_counts().iter().all(|&count|count <= (self.samples-4) as f32) { break; }
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
                wgpu::BindGroupEntry { binding: 8, resource: self.history[self.history_index].as_entire_binding() },
                wgpu::BindGroupEntry { binding: 9, resource: self.history[1-self.history_index].as_entire_binding() },
                wgpu::BindGroupEntry { binding: 10, resource: self.aov.as_entire_binding() },
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

    /// Final with a learned denoiser: the resolved image, guided by path-following
    /// albedo and normals, goes through `denoise` (color, albedo, normal; RGBA
    /// f32, RGB read) before it is composed. The backdrop's shadow-catcher
    /// shading is denoised the same way. Returns (pixels, ms).
    pub fn finish_with(&mut self, denoise: &dyn Fn(&mut [f32], &[f32], &[f32]) -> Result<(), String>) -> (Vec<u8>, f64) {
        self.finish_with_layers(&|color, shade, albedo, normal| {
            denoise(color, albedo, normal)?;
            denoise(shade, albedo, normal)
        })
    }

    /// Denoise both noisy layers together so auxiliary prefiltering is shared.
    pub fn finish_with_layers(&mut self, denoise: &dyn Fn(&mut [f32], &mut [f32], &[f32], &[f32]) -> Result<(), String>) -> (Vec<u8>, f64) {
        self.post(0, true, Some(denoise), None)
    }

    /// Actual per-pixel counts (adaptive rendering may stop before the ceiling).
    pub fn sample_counts(&self)->Vec<f32> {self.read_f32(&self.accum,u64::from(self.width)*u64::from(self.height)*16).chunks_exact(4).map(|p|p[3]).collect()}

    /// Diagnostic scene-linear beauty, including directly seen emission.
    /// RGB is premultiplied by coverage; alpha excludes the display-only void
    /// and shadow catcher. Unlike `radiance`, this is suitable for EXR export.
    pub fn linear_beauty(&self) -> Vec<f32> {
        let bytes = u64::from(self.width) * u64::from(self.height) * 16;
        let accum = self.read_f32(&self.accum, bytes);
        let emission = self.read_f32(&self.ebuf, bytes);
        let background = self.read_f32(&self.bgsum, bytes);
        accum.chunks_exact(4).zip(emission.chunks_exact(4)).zip(background.chunks_exact(4))
            .flat_map(|((a, e), b)| {
                let n = a[3].max(1.0);
                [(a[0]+e[0])/n, (a[1]+e[1])/n, (a[2]+e[2])/n, (1.0-b[3]/n).clamp(0.0,1.0)]
            }).collect()
    }

    /// Diagnostic first-hit object/layer IDs (zero is the void), not coverage.
    /// With depth of field these are the nearest sampled hits, not an ID pass
    /// that can be used to composite antialiased silhouettes.
    pub fn first_hit_ids(&self) -> Vec<u32> {
        self.read_f32(&self.gbuf, u64::from(self.width)*u64::from(self.height)*16)
            .chunks_exact(4).map(|p| p[2] as u32).collect()
    }

    /// Linear scene radiance before exposure and tone mapping.
    pub fn radiance(&mut self) -> Vec<f32> {
        let (w, h) = (self.width, self.height);
        let params = PostParams { dims: [w, h, 1, 0], k: [self.exposure, 1.6, self.output_layer.map_or(0.,|l|(l+1) as f32), (u32::from(self.output_include_void) | (u32::from(self.straight_output) << 1) | ((self.base.opts[3]&1)<<2) | (u32::from(self.quality_temporal)<<3) | (u32::from(self.base.backdrop[3]>0.)<<4)) as f32], backdrop: self.backdrop, cameras: [[0.0; 4]; 8] };
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

    fn encode_guides(&self, enc:&mut wgpu::CommandEncoder, shutter:Option<u32>) {
        let (w,h)=(self.width,self.height);let groups=(w.div_ceil(8),h.div_ceil(8));
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("aov"),
            layout: &self.aov_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: self.aov.as_entire_binding() }],
        });
        let mut p = self.base;
        p.dims = [w, h, 0, 1];
        p.caustic[3] = -(self.guide_samples.clamp(1, 64) as f32);
        p.opts = [0, self.base.opts[1], self.instance_count, self.base.opts[3]];
        p.counts = [self.light_count, 6, if shutter.is_some() {2} else {1}, 64];
        p.dims[2]=shutter.unwrap_or(0);
        self.queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
        let trace_group = self.trace_group.clone().expect("traced before finishing");
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_bind_group(0, &trace_group, &[]);
            if self.uses_hardware_rt() {
                let rt = self.rt.as_ref().unwrap();
                pass.set_pipeline(&rt.aovs);
                pass.set_bind_group(1, rt.group.as_ref(), &[]);
                pass.set_bind_group(2, &group, &[]);
            } else {
                pass.set_pipeline(&self.aovs);
                pass.set_bind_group(1, &group, &[]);
            }
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        }
    }
    pub fn begin_shutter(&mut self, first:bool, previous_samples:u32, salt:u32) {
        if first {self.shutter_guides=false;}
        self.samples=previous_samples;self.frame=salt.wrapping_mul(4099);self.seed_salt=salt;
    }
    pub fn end_shutter(&mut self) {self.shutter_guides=false;}
    pub fn accumulate_shutter_guides(&mut self,index:u32) {
        let mut enc=self.device.create_command_encoder(&Default::default());
        self.encode_guides(&mut enc,Some(index));self.queue.submit([enc.finish()]);self.wait();self.shutter_guides=true;
    }

    #[allow(clippy::type_complexity)]
    fn post(&mut self, passes: u32, readback: bool, denoise: Option<&dyn Fn(&mut [f32], &mut [f32], &[f32], &[f32]) -> Result<(), String>>, present: Option<PresentTarget<'_>>) -> (Vec<u8>, f64) {
        let start = Instant::now();
        let (w, h) = (self.width, self.height);
        if (self.live || self.quality_temporal) && self.history[0].size()<u64::from(w)*u64::from(h)*32 {
            self.history=std::array::from_fn(|_|self.device.create_buffer(&wgpu::BufferDescriptor{label:Some("live history"),size:u64::from(w)*u64::from(h)*32,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false}));self.history_valid=false;
        }
        let groups = (w.div_ceil(8), h.div_ceil(8));
        let mut enc = self.device.create_command_encoder(&Default::default());
        let mut slot = 0;
        let run = |enc: &mut wgpu::CommandEncoder, pipe: &wgpu::ComputePipeline, src: usize, dst: usize, step: u32, slot: &mut usize| {
            let params = PostParams { dims: [w, h, step, u32::from(self.history_valid)], k: [self.exposure, 1.6, self.output_layer.map_or(0.,|l|(l+1) as f32), (u32::from(self.output_include_void) | (u32::from(self.straight_output) << 1) | ((self.base.opts[3]&1)<<2) | (u32::from(self.quality_temporal)<<3) | (u32::from(self.base.backdrop[3]>0.)<<4)) as f32], backdrop: self.backdrop, cameras: [[self.base.cam_pos[0],self.base.cam_pos[1],self.base.cam_pos[2],self.base.ground[3]],self.base.cam_fwd,self.base.cam_right,self.base.cam_up,self.previous_camera[0],self.previous_camera[1],self.previous_camera[2],self.previous_camera[3]] };
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
            if !self.shutter_guides {self.encode_guides(&mut enc,None);}
            self.queue.submit([std::mem::replace(&mut enc, self.device.create_command_encoder(&Default::default())).finish()]);
            let pixels = (w * h) as usize;
            let mut color = self.read_f32(&self.display[0], pixels as u64 * 16);
            let aov = self.read_f32(&self.aov, pixels as u64 * 32);
            let (mut albedo, mut normal) = (vec![0f32; pixels * 4], vec![0f32; pixels * 4]);
            let mut shade = vec![0f32; pixels * 4];
            for i in 0..pixels {
                // The product's albedo × its share of the pixel (aov.wgsl).
                for c in 0..3 {
                    albedo[i * 4 + c] = aov[i * 8 + c] * aov[i * 8 + 3];
                }
                // The catcher is a diffuse receiver too. A black auxiliary
                // albedo falsely tells OIDN that its caustic/shadow shading is
                // detail on a black surface, leaving speckled light behind.
                if self.base.backdrop[3] > 0. && aov[i * 8 + 3] == 0. && aov[i * 8 + 5] > 0.9 {
                    albedo[i * 4..i * 4 + 3].fill(self.base.env_horizon[3]);
                }
                normal[i * 4..i * 4 + 4].copy_from_slice(&aov[i * 8 + 4..i * 8 + 8]);
                shade[i * 4..i * 4 + 3].fill(color[i * 4 + 3]);
            }
            if let Err(e) = denoise(&mut color, &mut shade, &albedo, &normal) {
                eprintln!("denoise failed, keeping the noisy frame: {e}");
            } else {
                for i in 0..pixels {
                    color[i * 4 + 3] = shade[i * 4];
                }
                self.queue.write_buffer(&self.display[0], 0, bytemuck::cast_slice(&color));
            }
        }
        let mut cur = 0;
        if self.live || self.quality_temporal {
            run(&mut enc, &self.temporal, cur, 1-cur, 1, &mut slot);
            cur = 1-cur;
        }
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
            self.last_display=cur;
            self.advance_history();
            return (Vec::new(), start.elapsed().as_secs_f64() * 1000.0);
        }
        run(&mut enc, &self.pack, cur, 1 - cur, 1, &mut slot);
        if !readback {
            self.queue.submit([enc.finish()]);
            self.wait();
            self.last_display=cur;
            self.advance_history();
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
        self.last_display=cur;
        self.advance_history();
        (pixels, gpu_ms)
    }

    /// Repack a different layer from the same finished trace (no tracing/denoising).
    pub fn frame_pixels(&mut self) -> (Vec<u8>,f64) {
        let start=Instant::now();let(w,h)=(self.width,self.height);
        let params=PostParams { dims:[w,h,1,0], k:[self.exposure,1.6,self.output_layer.map_or(0.,|l|(l+1) as f32),(u32::from(self.output_include_void)|(u32::from(self.straight_output)<<1)) as f32],backdrop:self.backdrop,cameras:[[0.;4];8] };
        self.queue.write_buffer(&self.post_params[0],0,bytemuck::bytes_of(&params));
        let group=self.post_group(0,self.last_display,1-self.last_display);
        let mut enc=self.device.create_command_encoder(&Default::default());
        {let mut pass=enc.begin_compute_pass(&Default::default());pass.set_pipeline(&self.pack);pass.set_bind_group(0,&group,&[]);pass.dispatch_workgroups(w.div_ceil(8),h.div_ceil(8),1);}
        enc.copy_buffer_to_buffer(&self.packed,0,&self.readback,0,(w*h*4) as u64);self.queue.submit([enc.finish()]);
        let slice=self.readback.slice(..);slice.map_async(wgpu::MapMode::Read,|_|{});self.wait();let bytes=slice.get_mapped_range().expect("mapped native frame").to_vec();self.readback.unmap();
        (bytes,start.elapsed().as_secs_f64()*1000.)
    }
    pub fn set_scene_bounds(&mut self,bounds:[f32;4]){if self.bounds!=bounds {self.photons_emitted=0.;}self.bounds=bounds;self.base.bounds=bounds;}
    pub fn set_precomputed_caustics(&mut self, enabled:bool) {if self.precompute_caustics!=enabled {self.photons_emitted=0.;}self.precompute_caustics=enabled;}
    pub fn set_projection(&mut self, orthographic:bool) {self.base.opts[3]=u32::from(orthographic);}
    pub fn set_clip_planes(&mut self,near:Option<f32>,far:Option<f32>) {
        if let (Some(near),Some(far))=(near,far) {
            if near.is_finite()&&far.is_finite()&&near>=0.&&far>near {
                self.base.opts[3]|=0x80000000|(near.to_bits()&0x7ffffffe);
                self.base.backdrop[3]=far*if self.base.backdrop[3]>0.{1.}else{-1.};
            }
        }
    }
    pub fn set_time_seed(&mut self, salt:u32) {self.seed_salt=salt;}
    pub fn set_straight_output(&mut self, on:bool) {self.straight_output=on;}

    fn advance_history(&mut self) {
        if self.live || self.quality_temporal { self.history_index=1-self.history_index; self.history_valid=true;
            self.previous_camera=[self.base.cam_pos,self.base.cam_fwd,self.base.cam_right,self.base.cam_up]; }
        else { self.history_valid=false; }
    }
    pub fn set_output_mask(&mut self, layer: Option<u32>, include_void: bool) {
        self.output_layer=layer;self.output_include_void=include_void;
    }
    pub fn invalidate_history(&mut self) { self.history_valid=false; }

    /// Texture-array data uses the same layer indices as Rust GpuMaterial.maps.
    pub fn set_maps(&mut self, rgba:&[u8], width:u32, height:u32, layers:u32) {
        self.photons_emitted = 0.;
        self.screen=self.device.create_texture_with_data(&self.queue,&wgpu::TextureDescriptor {
            label:Some("studio material maps"),size:wgpu::Extent3d{width,height,depth_or_array_layers:layers},
            mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,
            usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[],
        },wgpu::util::TextureDataOrder::LayerMajor,rgba);
        self.trace_group=None;self.samples=0;self.history_valid=false;
    }
    pub fn update_map(&mut self, rgba:&[u8], width:u32, height:u32, layer:u32) {
        self.photons_emitted=0.;
        self.queue.write_texture(wgpu::TexelCopyTextureInfo{texture:&self.screen,mip_level:0,origin:wgpu::Origin3d{x:0,y:0,z:layer},aspect:wgpu::TextureAspect::All},
            rgba,wgpu::TexelCopyBufferLayout{offset:0,bytes_per_row:Some(width*4),rows_per_image:Some(height)},wgpu::Extent3d{width,height,depth_or_array_layers:1});
        self.samples=0;self.history_valid=false;
    }

    /// Diagnostic: compare hardware and portable first-hit intersections.
    pub fn probe(&mut self, x:f32,y:f32)->Vec<f32> {
        let output=self.device.create_buffer(&wgpu::BufferDescriptor{label:Some("probe"),size:64,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
        let layout=self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{label:Some("probe"),entries:&[storage(0,false)]});
        let rt=self.uses_hardware_rt();
        let base=if rt {rt_source()}else {include_str!("shaders/trace.wgsl").to_string()};
        let kernel=format!(r#"
@group(2) @binding(0) var<storage,read_write> probe_out:array<vec4f>;
@compute @workgroup_size(1)
fn probe_main() {{
 seed=1u;let r=camera_ray(vec2f({x},{y}));let a=intersect(r[0],r[1],BIG,false);
 let b={portable}(r[0],r[1],BIG,false);
 probe_out[0]=vec4f(a.t,f32(a.tri),f32(a.inst),0.);
 probe_out[1]=vec4f(b.t,f32(b.tri),f32(b.inst),0.);
 if(a.tri>=0){{let sf=surface(r[0],r[1],a);probe_out[2]=vec4f(sf.m.base.xyz,sf.m.spec.w);}}
 if(b.tri>=0){{let sf=surface(r[0],r[1],b);probe_out[3]=vec4f(sf.m.base.xyz,sf.m.spec.w);}}
}}
"#,portable=if rt {"intersect_bvh"}else{"intersect"});
        let module=self.device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("probe"),source:wgpu::ShaderSource::Wgsl(format!("{base}\n{kernel}").into())});
        let empty=self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{label:None,entries:&[]});
        let ray_layout=if rt {&self.rt.as_ref().unwrap().layout}else{&empty};
        let pipeline=self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor{label:Some("probe"),layout:Some(&self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{label:None,bind_group_layouts:&[Some(&self.trace_layout),Some(ray_layout),Some(&layout)],immediate_size:0})),module:&module,entry_point:Some("probe_main"),compilation_options:Default::default(),cache:None});
        let group=self.device.create_bind_group(&wgpu::BindGroupDescriptor{label:None,layout:&layout,entries:&[wgpu::BindGroupEntry{binding:0,resource:output.as_entire_binding()}]});
        let mut p=self.base;p.dims=[self.width,self.height,1,1];p.opts[2]=self.instance_count;
        self.queue.write_buffer(&self.params,0,bytemuck::bytes_of(&p));
        let g=self.trace_group().clone();let mut enc=self.device.create_command_encoder(&Default::default());
        {let mut pass=enc.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&g,&[]);if rt {pass.set_bind_group(1,self.rt.as_ref().unwrap().group.as_ref(),&[]);}else{let g=self.device.create_bind_group(&wgpu::BindGroupDescriptor{label:None,layout:&empty,entries:&[]});pass.set_bind_group(1,&g,&[]);}pass.set_bind_group(2,&group,&[]);pass.dispatch_workgroups(1,1,1);}
        self.queue.submit([enc.finish()]);self.read_f32(&output,64)
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
