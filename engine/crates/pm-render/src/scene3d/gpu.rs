//! Executes the 3D pass on the GPU: three.js r180's programs (generated
//! by [`crate::three::program`], translated through `pm_gpu::glsl` and, for
//! Metal's safe math, `pm_gpu::msl`), the passes [`super::describe`] lays
//! out, 4× MSAA into RGBA8 with a resolve (the WebGL canvas), the shared
//! depth prepass kept between the layers of one depth set, shadow maps,
//! material maps, and the linear view depth of the depth and id surface.

use std::collections::HashMap;
use std::sync::Arc;

use pm_gpu::glsl::{self, ScalarKind};
pub use pm_gpu::msl::MathMode;
use pm_gpu::{Target, TargetPool};

use crate::three::cache;
use crate::three::math::Mat4;
use crate::three::program::{Parameters, sources};

use super::describe::{self, DrawDesc, DrawState, Layer3d, PassDesc, TextureRef};
use super::model::Scene3d;
use super::textures::{MapError, Textures};

#[path = "chrome_gpu.rs"]
mod chrome;

/// The MSAA sample count of the output canvas (`antialias: true`; ANGLE on
/// Metal allocates 4 samples, recorded by `tools/three-ref`).
pub const SAMPLES: u32 = 4;

struct Compiled {
    block_size: u32,
    uniforms: HashMap<String, glsl::UniformSlot>,
    /// Sampler name, texture binding, sampler binding; by texture binding.
    textures: Vec<(String, u32, u32)>,
    attributes: HashMap<String, u32>,
    vertex: wgpu::ShaderModule,
    fragment: wgpu::ShaderModule,
    vertex_entry: Option<String>,
    fragment_entry: Option<String>,
    math: MathMode,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
}

struct GpuMesh {
    position: wgpu::Buffer,
    normal: wgpu::Buffer,
    uv: wgpu::Buffer,
    index: wgpu::Buffer,
    count: u32,
    wireframe: Option<(wgpu::Buffer, u32)>,
}

impl GpuMesh {
    fn bytes(&self) -> u64 {
        [&self.position, &self.normal, &self.uv, &self.index]
            .into_iter()
            .chain(self.wireframe.as_ref().map(|(b, _)| b))
            .map(wgpu::Buffer::size)
            .sum()
    }
}

/// Why a 3D render drew nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum Error3d {
    /// Something this pass does not draw, met at render time (the frame is
    /// routed to the legacy engine, `MediaFailure::Unsupported`).
    Unsupported(String),
    /// The layer failed and drew nothing.
    Failed(String),
}

impl From<String> for Error3d {
    fn from(message: String) -> Error3d {
        Error3d::Failed(message)
    }
}

impl From<&str> for Error3d {
    fn from(message: &str) -> Error3d {
        Error3d::Failed(message.to_owned())
    }
}

impl std::fmt::Display for Error3d {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error3d::Unsupported(m) => write!(f, "unsupported: {m}"),
            Error3d::Failed(m) => f.write_str(m),
        }
    }
}

/// GPU resources the 3D pass keeps between frames
/// ([`Scene3dRenderer::resident`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Resident3d {
    /// Geometry buffers (one per distinct geometry, and per OBJ mesh of a
    /// legacy mesh layer).
    pub meshes: usize,
    pub mesh_bytes: u64,
    /// Material map textures (one per file version and color space).
    pub maps: usize,
    pub map_bytes: u64,
}

/// 3D frames a mesh or map stays resident without being drawn: the
/// previous 3D frame's resources survive, so a viewer alternating two
/// frames (a scrub across a cut, a thumbnail between viewer frames) does
/// not re-upload them every frame.
const KEEP_FRAMES: u64 = 2;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PipelineKey {
    program: u64,
    math: MathMode,
    samples: u32,
    state: DrawState,
    color4: bool,
}

/// The vertex inputs a program reads, by location.
fn vertex_inputs(compiled: &Compiled) -> Vec<(u32, &'static str)> {
    const NAMES: [&str; 15] = [
        "position",
        "normal",
        "uv",
        "uv1",
        "uv2",
        "uv3",
        "tangent",
        "color",
        "skinIndex",
        "skinWeight",
        "instanceMatrix_pmc0",
        "instanceMatrix_pmc1",
        "instanceMatrix_pmc2",
        "instanceMatrix_pmc3",
        "instanceColor",
    ];
    let mut inputs: Vec<(u32, &'static str)> = NAMES
        .iter()
        .filter_map(|n| compiled.attributes.get(*n).map(|&l| (l, *n)))
        .collect();
    inputs.sort();
    inputs
}

/// The matrix column an `instanceMatrix` input reads (0 otherwise).
fn instance_column(name: &str) -> u64 {
    match name {
        "instanceMatrix_pmc1" => 1,
        "instanceMatrix_pmc2" => 2,
        "instanceMatrix_pmc3" => 3,
        _ => 0,
    }
}

struct Attachments {
    width: u32,
    height: u32,
    samples: u32,
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
}

struct ShadowTarget {
    size: (u32, u32),
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
}

/// The last prepass recorded into the MSAA depth attachment.
struct PrepassKey {
    scene: Arc<Scene3d>,
    depth_set: Vec<usize>,
    samples: u32,
}

const DEPTH_SURFACE_WGSL: &str = r#"
struct U {
    model_view: mat4x4<f32>,
    projection: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> u: U;
struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) depth: f32,
};
@vertex
fn vs(@location(0) position: vec3<f32>) -> Out {
    let mv = u.model_view * vec4<f32>(position, 1.0);
    var p = u.projection * mv;
    // GL row order and depth range, as the color passes (pm_gpu::glsl::VERTEX_EPILOGUE).
    p.y = -p.y;
    p.z = (p.z + p.w) * 0.5;
    var o: Out;
    o.pos = p;
    o.depth = -mv.z;
    return o;
}
@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    return vec4<f32>(in.depth, 0.0, 0.0, 0.0);
}
"#;

struct DepthSurface {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

/// The pass output: premultiplied 8-bit color (E codes) and, when asked,
/// linear view depth (`r32float`, +inf where nothing drew), both in GL row
/// order.
pub struct Rendered3d {
    pub color: Target,
    pub depth: Option<Target>,
}

/// GPU state of the 3D pass, kept across frames.
pub struct Scene3dRenderer {
    samples: u32,
    math: MathMode,
    programs: HashMap<u64, Arc<Compiled>>,
    /// Program parameters (by a hash of their debug form) to their program,
    /// so a draw does not regenerate its sources.
    by_parameters: HashMap<u64, (u64, Arc<Compiled>)>,
    pipelines: HashMap<PipelineKey, wgpu::RenderPipeline>,
    /// GPU meshes by geometry key, with the 3D frame generation of their
    /// last use.
    meshes: HashMap<u64, (Arc<GpuMesh>, u64)>,
    /// The current 3D frame generation: advanced by the first 3D render
    /// after [`Scene3dRenderer::begin_frame`].
    generation: u64,
    frame_started: bool,
    /// The last scene whose maps were loaded, and that scene with the maps
    /// that failed to load dropped (shared by the layers of one pass).
    resolved: Option<(Arc<Scene3d>, Arc<Scene3d>)>,
    attachments: Option<Attachments>,
    shadows: HashMap<usize, ShadowTarget>,
    prepass: Option<PrepassKey>,
    depth_surface: Option<DepthSurface>,
    chrome: Option<chrome::Targets>,
    textures: Textures,
    empty: Option<wgpu::TextureView>,
    cache: Option<cache::ProgramCache>,
    cache_hits: u64,
    cache_write_failures: u64,
    pub(crate) legacy: super::legacy::MeshRenderer,
    gltf: super::gltf::gpu::GltfGpu,
}

impl Default for Scene3dRenderer {
    fn default() -> Self {
        Scene3dRenderer {
            samples: SAMPLES,
            math: MathMode::Safe,
            programs: HashMap::new(),
            by_parameters: HashMap::new(),
            pipelines: HashMap::new(),
            meshes: HashMap::new(),
            generation: 0,
            frame_started: false,
            resolved: None,
            attachments: None,
            shadows: HashMap::new(),
            prepass: None,
            depth_surface: None,
            chrome: None,
            textures: Textures::default(),
            empty: None,
            cache: None,
            cache_hits: 0,
            cache_write_failures: 0,
            legacy: Default::default(),
            gltf: Default::default(),
        }
    }
}

fn hash_of(parts: &[&[u8]]) -> u64 {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p);
    }
    let d = h.finalize();
    u64::from_le_bytes(d[..8].try_into().unwrap())
}

/// Writes one uniform into a default-block image (std140, from the
/// program's reflection). Unknown names are uniforms the compiler
/// eliminated.
pub(crate) fn write_uniform(
    uniforms: &HashMap<String, glsl::UniformSlot>,
    block: &mut [u8],
    name: &str,
    values: &[f64],
) {
    let base = name.strip_suffix("[0]").unwrap_or(name);
    let Some(slot) = uniforms.get(name).or_else(|| uniforms.get(base)) else {
        return;
    };
    let per = (slot.columns * slot.rows) as usize;
    let elements = (values.len() / per.max(1)).min(slot.count as usize);
    for e in 0..elements {
        for col in 0..slot.columns as usize {
            for row in 0..slot.rows as usize {
                let v = values[e * per + col * slot.rows as usize + row];
                let off = slot.offset as usize
                    + e * slot.array_stride as usize
                    + col * slot.column_stride as usize
                    + row * 4;
                let bytes = match slot.kind {
                    ScalarKind::Float => (v as f32).to_le_bytes(),
                    ScalarKind::Sint => (v as i32).to_le_bytes(),
                    ScalarKind::Uint => (v as u32).to_le_bytes(),
                    ScalarKind::Bool => u32::from(v != 0.0).to_le_bytes(),
                };
                if off + 4 <= block.len() {
                    block[off..off + 4].copy_from_slice(&bytes);
                }
            }
        }
    }
}

/// Renumbers texture and sampler bindings compactly (the block keeps 0;
/// textures 1..=n and samplers n+1..=2n in texture binding order): wgpu
/// allows bindings up to 1000, and `pm_gpu::glsl` places samplers at their
/// texture's binding plus 1000.
fn compact_bindings(program: &mut glsl::Program) {
    let mut sorted: Vec<(u32, u32)> = program
        .textures
        .values()
        .map(|t| (t.binding, t.sampler_binding))
        .collect();
    sorted.sort();
    let n = sorted.len() as u32;
    let mut map: HashMap<u32, u32> = HashMap::new();
    for (k, (t, smp)) in sorted.iter().enumerate() {
        map.insert(*t, 1 + k as u32);
        map.insert(*smp, 1 + n + k as u32);
    }
    for module in [&mut program.vertex, &mut program.fragment] {
        for (_, var) in module.global_variables.iter_mut() {
            if let Some(b) = var.binding.as_mut()
                && b.group == 0
                && let Some(&to) = map.get(&b.binding)
            {
                b.binding = to;
            }
        }
    }
    for t in program.textures.values_mut() {
        t.binding = map[&t.binding];
        t.sampler_binding = map[&t.sampler_binding];
    }
}

impl Scene3dRenderer {
    /// Starts a frame. Its first 3D render releases the meshes and maps no
    /// 3D frame drew in the last [`KEEP_FRAMES`] 3D frames, so the
    /// resident set follows the scenes being drawn.
    pub fn begin_frame(&mut self) {
        self.frame_started = true;
    }

    fn start_generation(&mut self) {
        if !self.frame_started {
            return;
        }
        self.frame_started = false;
        self.generation += 1;
        let keep_from = self.generation.saturating_sub(KEEP_FRAMES - 1);
        self.meshes.retain(|_, (_, used)| *used >= keep_from);
        self.textures.evict(self.generation, keep_from);
        self.legacy.evict(self.generation, keep_from);
        self.gltf.evict(self.generation, keep_from);
    }

    /// The meshes and maps resident on the GPU.
    pub fn resident(&self) -> Resident3d {
        let (maps, map_bytes) = self.textures.resident();
        let (legacy, legacy_bytes) = self.legacy.resident();
        let (gltf, gltf_bytes) = self.gltf.resident();
        Resident3d {
            meshes: self.meshes.len() + legacy + gltf,
            mesh_bytes: self.meshes.values().map(|(m, _)| m.bytes()).sum::<u64>()
                + legacy_bytes
                + gltf_bytes,
            maps,
            map_bytes,
        }
    }

    /// Loads every map of `scene` before anything draws. A map that no
    /// longer loads (deleted or unreadable since the plan) is dropped with
    /// a `Missing texture` warning, as today's runtime draws a model whose
    /// texture getter returns nothing; one outside the classes this pass
    /// draws (changed since the capability check) makes the render
    /// unsupported.
    fn resolve_maps(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene: &Arc<Scene3d>,
    ) -> Result<Arc<Scene3d>, Error3d> {
        if let Some((from, to)) = &self.resolved
            && Arc::ptr_eq(from, scene)
        {
            return Ok(to.clone());
        }
        let mut failed: Vec<(std::path::PathBuf, bool)> = Vec::new();
        for o in &scene.objects {
            for m in &o.meshes {
                for mat in m.materials.iter() {
                    for (_, t) in mat.maps.all() {
                        let Some(t) = t else { continue };
                        match self.textures.map(device, queue, encoder, &t.path, t.srgb) {
                            Ok(_) => {}
                            Err(MapError::Unsupported(why)) => {
                                return Err(Error3d::Unsupported(why));
                            }
                            Err(MapError::Failed(_)) => failed.push((t.path.clone(), t.srgb)),
                        }
                    }
                }
            }
        }
        let out = if failed.is_empty() {
            scene.clone()
        } else {
            let mut s = Scene3d::clone(scene);
            let mut warnings = Vec::new();
            for o in &mut s.objects {
                for m in &mut o.meshes {
                    for mat in m.materials.iter_mut() {
                        for slot in mat.maps.all_mut() {
                            if let Some(t) = slot
                                && failed.contains(&(t.path.clone(), t.srgb))
                            {
                                warnings.push(format!("Missing texture: {}", t.asset));
                                *slot = None;
                            }
                        }
                    }
                }
            }
            s.warnings.extend(warnings);
            Arc::new(s)
        };
        self.resolved = Some((scene.clone(), out.clone()));
        Ok(out)
    }

    /// Keeps translated programs in `dir` across processes (the engine
    /// passes its cache directory; `None` translates in memory only).
    pub fn set_program_cache(&mut self, dir: Option<std::path::PathBuf>) {
        self.cache = dir.map(cache::ProgramCache::new);
    }

    /// Programs read from the disk cache, and cache writes that failed.
    pub fn program_cache_stats(&self) -> (u64, u64) {
        (self.cache_hits, self.cache_write_failures)
    }

    /// Sets the Metal math mode of the 3D programs (safe by default,
    /// amendment 11; fast only for comparisons).
    pub fn set_math_mode(&mut self, math: MathMode) {
        self.math = math;
        self.by_parameters.clear();
    }

    /// The math mode the programs compile with on `device` (fast when the
    /// device has no passthrough shaders).
    pub fn math_mode(&self, device: &wgpu::Device) -> MathMode {
        if self.math != MathMode::Fast && pm_gpu::msl::supported(device) {
            self.math
        } else {
            MathMode::Fast
        }
    }

    /// Sets the MSAA sample count (4 as today's canvas; 1 only to classify
    /// MSAA edges for parity).
    pub fn set_samples(&mut self, samples: u32) {
        assert!(
            samples == 1 || samples == SAMPLES,
            "MSAA samples must be 1 or {SAMPLES}"
        );
        self.samples = samples;
        self.prepass = None;
    }

    fn program(
        &mut self,
        device: &wgpu::Device,
        params: &Parameters,
    ) -> Result<(u64, Arc<Compiled>), String> {
        let pkey = hash_of(&[
            format!("{params:?}").as_bytes(),
            format!("{:?}", self.math).as_bytes(),
        ]);
        if let Some(found) = self.by_parameters.get(&pkey) {
            return Ok(found.clone());
        }
        let found = self.program_of(device, params)?;
        self.by_parameters.insert(pkey, found.clone());
        Ok(found)
    }

    fn program_of(
        &mut self,
        device: &wgpu::Device,
        params: &Parameters,
    ) -> Result<(u64, Arc<Compiled>), String> {
        let (vs, fs) = sources(params)?;
        // The sampler specialization and binding layout are part of the
        // translation.
        let mode = format!("{:?} preprocess 1 specialize 1 compact 1", self.math);
        let key = hash_of(&[vs.as_bytes(), fs.as_bytes(), mode.as_bytes()]);
        if let Some(c) = self.programs.get(&key) {
            return Ok((key, c.clone()));
        }
        let translate = || -> Result<glsl::Program, String> {
            // Sampler-parameter functions are specialized on the live code
            // (after the preprocessor), then compiled.
            let live = |stage, src: &str| -> Result<String, String> {
                let pre = glsl::preprocess(stage, src).map_err(|e| format!("3D program: {e}"))?;
                crate::three::specialize::specialize(&pre)
            };
            let mut program = glsl::compile(
                &live(glsl::Stage::Vertex, &vs)?,
                &live(glsl::Stage::Fragment, &fs)?,
            )
            .map_err(|e| format!("3D program: {e}"))?;
            for (name, t) in &program.textures {
                // `morphTargetsTexture` is r180's one array sampler.
                let array_ok = name == "morphTargetsTexture" && t.arrayed;
                if t.dim != glsl::naga::ImageDimension::D2 || (t.arrayed && !array_ok) {
                    return Err(format!("3D program samples {name} as a non-2D texture"));
                }
            }
            compact_bindings(&mut program);
            Ok(program)
        };
        let reflection = |p: &glsl::Program| {
            (
                p.block_size,
                p.uniforms.clone(),
                p.textures
                    .iter()
                    .map(|(n, t)| (n.clone(), (t.binding, t.sampler_binding)))
                    .collect::<HashMap<_, _>>(),
                p.attributes
                    .iter()
                    .map(|(n, a)| (n.clone(), a.location))
                    .collect::<HashMap<_, _>>(),
            )
        };
        // Metal slots: the block in buffer 0; the k-th texture by binding
        // in texture and sampler slot k (wgpu-hal assigns layout entries in
        // order, every entry visible to both stages).
        let slots = |textures: &HashMap<String, (u32, u32)>| {
            let mut sorted: Vec<(u32, u32)> = textures.values().copied().collect();
            sorted.sort();
            let mut s = vec![pm_gpu::msl::Slot {
                group: 0,
                binding: glsl::DEFAULT_BLOCK_BINDING,
                buffer: Some(0),
                texture: None,
                sampler: None,
            }];
            for (k, (t, smp)) in sorted.iter().enumerate() {
                s.push(pm_gpu::msl::Slot {
                    group: 0,
                    binding: *t,
                    buffer: None,
                    texture: Some(k as u8),
                    sampler: None,
                });
                s.push(pm_gpu::msl::Slot {
                    group: 0,
                    binding: *smp,
                    buffer: None,
                    texture: None,
                    sampler: Some(k as u8),
                });
            }
            s
        };
        let safe = self.math != MathMode::Fast && pm_gpu::msl::supported(device);
        let points = params.shader_type == "PointsMaterial";
        let (
            vertex,
            fragment,
            vertex_entry,
            fragment_entry,
            math,
            (block_size, uniforms, textures, attributes),
        ) = if safe {
            let disk_key = cache::key(&vs, &fs, &mode);
            let entry = match self.cache.as_ref().and_then(|c| c.load(&disk_key)) {
                Some(e) => {
                    self.cache_hits += 1;
                    e
                }
                None => {
                    let program = translate()?;
                    let (block_size, uniforms, textures, attributes) = reflection(&program);
                    let s = slots(&textures);
                    let (vertex_msl, vertex_entry) = pm_gpu::msl::write_with(
                        &program.vertex,
                        &program.vertex_info,
                        glsl::naga::ShaderStage::Vertex,
                        &s,
                        self.math,
                        points,
                    )?;
                    let (fragment_msl, fragment_entry) = pm_gpu::msl::write(
                        &program.fragment,
                        &program.fragment_info,
                        glsl::naga::ShaderStage::Fragment,
                        &s,
                        self.math,
                    )?;
                    let e = cache::CachedProgram {
                        block_size,
                        uniforms,
                        textures,
                        attributes,
                        vertex_msl,
                        vertex_entry,
                        fragment_msl,
                        fragment_entry,
                    };
                    if let Some(c) = &self.cache
                        && c.store(&disk_key, &e).is_err()
                    {
                        self.cache_write_failures += 1;
                    }
                    e
                }
            };
            let vs = pm_gpu::msl::from_source(
                device,
                "three vs",
                &entry.vertex_msl,
                &entry.vertex_entry,
            )?;
            let fs = pm_gpu::msl::from_source(
                device,
                "three fs",
                &entry.fragment_msl,
                &entry.fragment_entry,
            )?;
            (
                vs.module,
                fs.module,
                Some(vs.entry_point),
                Some(fs.entry_point),
                self.math,
                (
                    entry.block_size,
                    entry.uniforms,
                    entry.textures,
                    entry.attributes,
                ),
            )
        } else {
            let program = translate()?;
            let module = |m: &glsl::naga::Module, label: &str| {
                device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(label),
                    source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(m.clone())),
                })
            };
            (
                module(&program.vertex, "three vs"),
                module(&program.fragment, "three fs"),
                None,
                None,
                MathMode::Fast,
                reflection(&program),
            )
        };
        let mut sorted: Vec<(String, u32, u32)> =
            textures.into_iter().map(|(n, (t, s))| (n, t, s)).collect();
        sorted.sort_by_key(|(_, t, _)| *t);
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: glsl::DEFAULT_BLOCK_BINDING,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(u64::from(block_size.max(16))),
            },
            count: None,
        }];
        for (name, t, _) in &sorted {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: *t,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: if name == "morphTargetsTexture" {
                        wgpu::TextureViewDimension::D2Array
                    } else {
                        wgpu::TextureViewDimension::D2
                    },
                    multisampled: false,
                },
                count: None,
            });
        }
        for (_, _, s) in &sorted {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: *s,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("three program"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("three program"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let c = Arc::new(Compiled {
            block_size,
            uniforms,
            textures: sorted,
            attributes,
            vertex,
            fragment,
            vertex_entry,
            fragment_entry,
            math,
            layout,
            pipeline_layout,
        });
        self.programs.insert(key, c.clone());
        Ok((key, c))
    }

    fn mesh(
        &mut self,
        device: &wgpu::Device,
        g: &super::model::Geometry3d,
        wireframe: bool,
    ) -> Arc<GpuMesh> {
        let generation = self.generation;
        if let Some((m, used)) = self.meshes.get_mut(&g.key)
            && (!wireframe || m.wireframe.is_some())
        {
            *used = generation;
            return m.clone();
        }
        let geometry = &g.geometry;
        let buffer = |label: &str, mut bytes: Vec<u8>, usage: wgpu::BufferUsages| {
            // naga's MSL vertex pulling admits element i only when
            // i < bound size / stride (S7 finding 6); padding keeps GL's rule.
            bytes.resize(bytes.len() + 256, 0);
            wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: &bytes,
                    usage,
                },
            )
        };
        let f = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let count = geometry.vertex_count();
        let uvs = if geometry.uvs.is_empty() {
            // No `uv` attribute: WebGL's constant attribute value (0, 0).
            vec![0f32; count * 2]
        } else {
            geometry.uvs.clone()
        };
        // No `normal` attribute (flat-shaded glTF): WebGL's constant zero.
        let normals = if geometry.normals.len() < count * 3 {
            vec![0f32; count * 3]
        } else {
            geometry.normals.clone()
        };
        let index = |v: &[u32]| v.iter().flat_map(|i| i.to_le_bytes()).collect::<Vec<u8>>();
        let wire = wireframe.then(|| {
            let w = geometry.wireframe_indices();
            (
                buffer("3d wireframe", index(&w), wgpu::BufferUsages::INDEX),
                w.len() as u32,
            )
        });
        let m = Arc::new(GpuMesh {
            position: buffer(
                "3d position",
                f(&geometry.positions),
                wgpu::BufferUsages::VERTEX,
            ),
            normal: buffer("3d normal", f(&normals), wgpu::BufferUsages::VERTEX),
            uv: buffer("3d uv", f(&uvs), wgpu::BufferUsages::VERTEX),
            index: buffer(
                "3d index",
                index(&geometry.indices),
                wgpu::BufferUsages::INDEX,
            ),
            count: geometry.indices.len() as u32,
            wireframe: wire,
        });
        self.meshes.insert(g.key, (m.clone(), generation));
        m
    }

    fn attachments(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let samples = self.samples;
        if matches!(&self.attachments, Some(a) if a.width == width && a.height == height && a.samples == samples)
        {
            return;
        }
        let make = |format: wgpu::TextureFormat, label: &str| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        self.attachments = Some(Attachments {
            width,
            height,
            samples,
            color: make(wgpu::TextureFormat::Rgba8Unorm, "3d msaa color"),
            depth: make(wgpu::TextureFormat::Depth32Float, "3d msaa depth"),
        });
        self.prepass = None;
    }

    fn shadow_target(&mut self, device: &wgpu::Device, light: usize, size: (u32, u32)) {
        if matches!(self.shadows.get(&light), Some(t) if t.size == size) {
            return;
        }
        let make = |format: wgpu::TextureFormat, usage: wgpu::TextureUsages, label: &str| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        self.shadows.insert(
            light,
            ShadowTarget {
                size,
                color: make(
                    wgpu::TextureFormat::Rgba8Unorm,
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    "3d shadow map",
                ),
                depth: make(
                    wgpu::TextureFormat::Depth32Float,
                    wgpu::TextureUsages::RENDER_ATTACHMENT,
                    "3d shadow depth",
                ),
            },
        );
    }

    fn pipeline(
        &mut self,
        device: &wgpu::Device,
        program: u64,
        compiled: &Compiled,
        samples: u32,
        state: DrawState,
        extra: Option<Arc<super::gltf::VertexExtra>>,
    ) -> wgpu::RenderPipeline {
        // The color attribute's item size is part of the vertex layout.
        let color4 = extra
            .as_ref()
            .and_then(|x| x.color.as_ref())
            .is_some_and(|c| c.1 == 4);
        let key = PipelineKey {
            program,
            math: compiled.math,
            samples,
            state,
            color4,
        };
        if let Some(p) = self.pipelines.get(&key) {
            return p.clone();
        }
        let inputs = vertex_inputs(compiled);
        let attrs: Vec<[wgpu::VertexAttribute; 1]> = inputs
            .iter()
            .map(|(loc, name)| {
                let (format, _, _) = super::gltf::gpu::attribute_format(name, extra.as_deref())
                    .unwrap_or((wgpu::VertexFormat::Float32x3, 12, false));
                [wgpu::VertexAttribute {
                    format,
                    offset: instance_column(name) * 16,
                    shader_location: *loc,
                }]
            })
            .collect();
        let layouts: Vec<Option<wgpu::VertexBufferLayout>> = inputs
            .iter()
            .zip(attrs.iter())
            .map(|((_, name), a)| {
                let (_, stride, instance) = super::gltf::gpu::attribute_format(
                    name,
                    extra.as_deref(),
                )
                .unwrap_or((wgpu::VertexFormat::Float32x3, 12, false));
                Some(wgpu::VertexBufferLayout {
                    array_stride: stride,
                    step_mode: if instance {
                        wgpu::VertexStepMode::Instance
                    } else {
                        wgpu::VertexStepMode::Vertex
                    },
                    attributes: a,
                })
            })
            .collect();
        // Clip y is negated (GL row order), so GL's counter-clockwise front
        // faces are clockwise here.
        let front_face = if state.front_face_cw {
            wgpu::FrontFace::Ccw
        } else {
            wgpu::FrontFace::Cw
        };
        let blend = state.blend.then_some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("three draw"),
            layout: Some(&compiled.pipeline_layout),
            vertex: wgpu::VertexState {
                module: &compiled.vertex,
                entry_point: compiled.vertex_entry.as_deref(),
                compilation_options: Default::default(),
                buffers: &layouts,
            },
            primitive: wgpu::PrimitiveState {
                topology: if state.lines {
                    wgpu::PrimitiveTopology::LineList
                } else {
                    match state.topology {
                        describe::Topology::Triangles => wgpu::PrimitiveTopology::TriangleList,
                        describe::Topology::Lines => wgpu::PrimitiveTopology::LineList,
                        describe::Topology::LineStrip => wgpu::PrimitiveTopology::LineStrip,
                        describe::Topology::Points => wgpu::PrimitiveTopology::PointList,
                    }
                },
                strip_index_format: (!state.lines
                    && state.topology == describe::Topology::LineStrip)
                    .then_some(wgpu::IndexFormat::Uint32),
                front_face,
                cull_mode: (state.cull
                    && !state.lines
                    && state.topology == describe::Topology::Triangles)
                    .then_some(wgpu::Face::Back),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(state.depth_write),
                depth_compare: Some(if state.depth_test {
                    wgpu::CompareFunction::LessEqual
                } else {
                    wgpu::CompareFunction::Always
                }),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: &compiled.fragment,
                entry_point: compiled.fragment_entry.as_deref(),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: if state.hdr {
                        wgpu::TextureFormat::Rgba16Float
                    } else {
                        wgpu::TextureFormat::Rgba8Unorm
                    },
                    blend,
                    write_mask: if state.color_write {
                        state.color_mask.map_or(
                            wgpu::ColorWrites::ALL,
                            wgpu::ColorWrites::from_bits_truncate,
                        )
                    } else {
                        wgpu::ColorWrites::empty()
                    },
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.insert(key, pipeline.clone());
        pipeline
    }

    fn empty_texture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
        if let Some(v) = &self.empty {
            return v.clone();
        }
        let t = wgpu::util::DeviceExt::create_texture_with_data(
            device,
            queue,
            &wgpu::TextureDescriptor {
                label: Some("3d empty texture"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &[0, 0, 0, 0],
        );
        let v = t.create_view(&wgpu::TextureViewDescriptor::default());
        self.empty = Some(v.clone());
        v
    }

    /// Prepares one draw: program, pipeline, uniform block, bind group.
    #[allow(clippy::too_many_arguments)]
    fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene: &Scene3d,
        draw: &DrawDesc,
        samples: u32,
    ) -> Result<Prepared, Error3d> {
        let (key, compiled) = self.program(device, &draw.params)?;
        let mesh_desc = &scene.objects[draw.object].meshes[draw.mesh];
        let pipeline = self.pipeline(
            device,
            key,
            &compiled,
            samples,
            draw.state,
            mesh_desc.geometry.extra.clone(),
        );
        let mut block = vec![0u8; compiled.block_size.max(16) as usize];
        for (name, v) in &draw.uniforms {
            write_uniform(&compiled.uniforms, &mut block, name, v);
        }
        let ubuf = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("three uniforms"),
                contents: &block,
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        self.textures.samplers(device);
        let mut views: Vec<(u32, u32, wgpu::TextureView, wgpu::Sampler)> = Vec::new();
        for (name, t, s) in &compiled.textures {
            let bound = draw
                .textures
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, r)| r);
            let (view, sampler) = match bound {
                Some(TextureRef::Map(map)) => {
                    let tex = self
                        .textures
                        .map(device, queue, encoder, &map.path, map.srgb)
                        .map_err(|e| match e {
                            MapError::Unsupported(m) => Error3d::Unsupported(m),
                            MapError::Failed(m) => Error3d::Failed(m),
                        })?;
                    (
                        tex.view.clone(),
                        self.textures.repeat_trilinear.clone().expect("samplers"),
                    )
                }
                Some(TextureRef::Shadow(light)) => (
                    self.shadows
                        .get(light)
                        .ok_or("a shadow map that was not rendered")?
                        .color
                        .clone(),
                    self.textures.clamp_nearest.clone().expect("samplers"),
                ),
                Some(TextureRef::Gltf(m)) => {
                    self.gltf.map(device, queue, encoder, &mut self.textures, m)
                }
                Some(TextureRef::Data {
                    key,
                    width,
                    height,
                    layers,
                    array,
                    data,
                }) => self
                    .gltf
                    .data(device, queue, *key, *width, *height, *layers, *array, data),
                // Before the layer's transmission pass (the depth prepass of
                // a transmissive object, which writes no color) any texture.
                Some(TextureRef::Transmission) => match self.gltf.transmission_binding(device) {
                    Some(b) => b,
                    None => (
                        self.empty_texture(device, queue),
                        self.textures.clamp_nearest.clone().expect("samplers"),
                    ),
                },
                Some(TextureRef::Ltc1) | Some(TextureRef::Ltc2) => {
                    let (a, b) = self
                        .textures
                        .ltc(device, queue)
                        .map_err(Error3d::Unsupported)?;
                    let v = if matches!(bound, Some(TextureRef::Ltc1)) {
                        a
                    } else {
                        b
                    };
                    (v, self.textures.ltc_sampler.clone().expect("samplers"))
                }
                None => (
                    self.empty_texture(device, queue),
                    self.textures.clamp_nearest.clone().expect("samplers"),
                ),
            };
            views.push((*t, *s, view, sampler));
        }
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: glsl::DEFAULT_BLOCK_BINDING,
            resource: ubuf.as_entire_binding(),
        }];
        for (t, _, view, _) in &views {
            entries.push(wgpu::BindGroupEntry {
                binding: *t,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        for (_, s, _, sampler) in &views {
            entries.push(wgpu::BindGroupEntry {
                binding: *s,
                resource: wgpu::BindingResource::Sampler(sampler),
            });
        }
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("three draw"),
            layout: &compiled.layout,
            entries: &entries,
        });
        let mesh = self.mesh(device, &mesh_desc.geometry, draw.state.lines);
        let mut buffers = Vec::new();
        for (_, n) in vertex_inputs(&compiled) {
            let b = match n {
                "position" => mesh.position.clone(),
                "normal" => mesh.normal.clone(),
                "uv" => mesh.uv.clone(),
                n if n.starts_with("instance") => {
                    let ext = mesh_desc
                        .gltf
                        .as_ref()
                        .ok_or("instance attributes on a scene mesh")?;
                    self.gltf
                        .instance(device, ext, n)
                        .ok_or("an instanced program without instances")?
                }
                n => self
                    .gltf
                    .vertex(device, &mesh_desc.geometry, n)
                    .ok_or_else(|| format!("3D program attribute {n}"))?,
            };
            buffers.push(b);
        }
        let instances = mesh_desc
            .gltf
            .as_ref()
            .and_then(|g| g.instances.as_ref())
            .map_or(1, |(_, i)| i.matrices.len() as u32);
        let factor = if draw.state.lines { 2 } else { 1 };
        let (index, total) = if draw.state.lines {
            mesh.wireframe.clone().expect("wireframe index")
        } else {
            (mesh.index.clone(), mesh.count)
        };
        let range = match draw.range {
            Some((start, count)) => {
                let s = (start * factor).min(total);
                s..(s + count * factor).min(total)
            }
            None => 0..total,
        };
        Ok(Prepared {
            pipeline,
            bind,
            buffers,
            index,
            range,
            instances,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene: &Scene3d,
        pass: &PassDesc,
        resolve: Option<&wgpu::TextureView>,
    ) -> Result<(), Error3d> {
        match pass {
            PassDesc::Shadow { light, size, views } => {
                self.shadow_target(device, *light, *size);
                let mut prepared = Vec::new();
                for (view, draws) in views {
                    let mut list = Vec::new();
                    for d in draws {
                        list.push(self.prepare(device, queue, encoder, scene, d, 1)?);
                    }
                    prepared.push((view.viewport, list));
                }
                let t = &self.shadows[light];
                let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("3d shadow map"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &t.color,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &t.depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                for (vp, list) in &prepared {
                    rp.set_viewport(
                        vp[0] as f32,
                        vp[1] as f32,
                        vp[2] as f32,
                        vp[3] as f32,
                        0.0,
                        1.0,
                    );
                    for p in list {
                        p.draw(&mut rp);
                    }
                }
            }
            PassDesc::Transmission {
                clear,
                opaque,
                back,
            } => {
                let (w, h) = scene.canvas();
                self.gltf.transmission(device, w, h);
                let samples = super::gltf::gpu::TRANSMISSION_SAMPLES;
                for (k, draws) in [opaque, back].into_iter().enumerate() {
                    if k == 1 && draws.is_empty() {
                        break;
                    }
                    let mut list = Vec::new();
                    for d in draws {
                        list.push(self.prepare(device, queue, encoder, scene, d, samples)?);
                    }
                    let resolve = self
                        .gltf
                        .transmission_level0()
                        .expect("transmission target");
                    let t = self
                        .gltf
                        .transmission
                        .as_ref()
                        .expect("transmission target");
                    // The back faces sample the resolved texture while they
                    // draw into the multisampled target, so their pass resolves
                    // separately (`updateMultisampleRenderTarget` after them).
                    for step in 0..(1 + usize::from(k == 1)) {
                        let resolve_now = k == 0 || step == 1;
                        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("3d transmission"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &t.msaa,
                                depth_slice: None,
                                resolve_target: resolve_now.then_some(&resolve),
                                ops: wgpu::Operations {
                                    load: if k == 0 {
                                        wgpu::LoadOp::Clear(wgpu::Color {
                                            r: clear[0],
                                            g: clear[1],
                                            b: clear[2],
                                            a: clear[3],
                                        })
                                    } else {
                                        wgpu::LoadOp::Load
                                    },
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            depth_stencil_attachment: Some(
                                wgpu::RenderPassDepthStencilAttachment {
                                    view: &t.depth,
                                    depth_ops: Some(wgpu::Operations {
                                        load: if k == 0 {
                                            wgpu::LoadOp::Clear(1.0)
                                        } else {
                                            wgpu::LoadOp::Load
                                        },
                                        store: wgpu::StoreOp::Store,
                                    }),
                                    stencil_ops: None,
                                },
                            ),
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });
                        if step == 0 {
                            for p in &list {
                                p.draw(&mut rp);
                            }
                        }
                    }
                    let texture = t.texture.clone();
                    self.gltf.generate_mips(device, encoder, &texture);
                }
            }
            PassDesc::Prepass(draws) => {
                let samples = self.samples;
                let mut list = Vec::new();
                for d in draws {
                    list.push(self.prepare(device, queue, encoder, scene, d, samples)?);
                }
                let a = self.attachments.as_ref().expect("attachments");
                let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("3d depth prepass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &a.color,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Discard,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &a.depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                for p in &list {
                    p.draw(&mut rp);
                }
            }
            PassDesc::Color { clear, draws } => {
                let samples = self.samples;
                let mut list = Vec::new();
                for d in draws {
                    list.push(self.prepare(device, queue, encoder, scene, d, samples)?);
                }
                let a = self.attachments.as_ref().expect("attachments");
                let resolve = resolve.expect("a resolve target");
                let (view, resolve_target, store) = if samples > 1 {
                    (&a.color, Some(resolve), wgpu::StoreOp::Discard)
                } else {
                    (resolve, None, wgpu::StoreOp::Store)
                };
                let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("3d color"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: clear[0],
                                g: clear[1],
                                b: clear[2],
                                a: clear[3],
                            }),
                            store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &a.depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                for p in &list {
                    p.draw(&mut rp);
                }
            }
        }
        Ok(())
    }

    /// Fetches the SVG map uploads this scene needs. No scene geometry is
    /// drawn by Chromium: it only decodes the SVG image into today's texels.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_svg_maps(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        plan: &super::Object3dPlan,
        rasters: &mut crate::vector::svg_rasters::SvgRasters,
        composition: &str,
    ) -> bool {
        self.start_generation();
        let mut ready = true;
        for object in &plan.scene.objects {
            for mesh in &object.meshes {
                for material in mesh.materials.iter() {
                    for (_, map) in material.maps.all() {
                        let Some(map) = map else { continue };
                        let Some(request) = &map.svg else { continue };
                        let key = &request.key;
                        if !rasters.contains(key)
                            && rasters.rasterizer
                                == crate::vector::svg_rasters::SvgRasterizer::Resvg
                        {
                            // pm-cli's explicitly reported SVG approximation.
                            if let Ok((doc, _)) = crate::vector::svg::document(&map.path) {
                                let mut pixels = doc.raster(key.width, key.height).rgba;
                                crate::blender::snapshot::unpremultiply(&mut pixels);
                                let row = key.width as usize * 4;
                                for y in 0..key.height as usize / 2 {
                                    let other = key.height as usize - 1 - y;
                                    for x in 0..row {
                                        pixels.swap(y * row + x, other * row + x);
                                    }
                                }
                                let _ = rasters.put(key.clone(), pixels);
                            }
                        }
                        if rasters.rasterizer == crate::vector::svg_rasters::SvgRasterizer::Resvg {
                            rasters.approximated += 1;
                        }
                        if let Some(pixels) = rasters.get(key) {
                            // The file was probed when the scene was planned;
                            // if it disappeared, resolve_maps reports Missing texture.
                            let _ = self.textures.put_svg(
                                device, queue, encoder, &map.path, map.srgb, key.width, key.height,
                                &pixels,
                            );
                        } else {
                            rasters.miss(crate::vector::svg_rasters::SvgRasterNeed {
                                request: request.clone(),
                                path: map.path.clone(),
                                composition: composition.into(),
                                layer: plan.layer.clone(),
                            });
                            ready = false;
                        }
                    }
                }
            }
        }
        ready
    }

    /// Records a 3D render into `encoder`: a layer of the 3D scene pass, a
    /// legacy scene layer, or a legacy mesh layer. Targets come from `pool`;
    /// the caller returns them after the frame is submitted.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &mut TargetPool,
        plan: &super::Object3dPlan,
        depth_surface: bool,
    ) -> Result<Rendered3d, Error3d> {
        self.start_generation();
        match &plan.kind {
            super::Render3d::Layer {
                object,
                depth_set,
                background,
            } => self.render_layer(
                device,
                queue,
                encoder,
                pool,
                &plan.scene,
                *object,
                depth_set,
                *background,
                depth_surface,
            ),
            super::Render3d::Whole => {
                self.render_whole(device, queue, encoder, pool, &plan.scene, depth_surface)
            }
            super::Render3d::ObjMesh(mesh) => {
                let color = self
                    .legacy
                    .render(device, queue, encoder, pool, mesh, self.samples)
                    .map_err(Error3d::Failed)?;
                Ok(Rendered3d { color, depth: None })
            }
        }
    }

    /// Keeps only the shadow maps `scene`'s casting lights use (at most 4).
    fn keep_shadows(&mut self, scene: &Scene3d) {
        self.shadows.retain(|&i, _| {
            scene
                .lights
                .get(i)
                .is_some_and(|l| l.cast_shadow && l.visible)
        });
    }

    fn output(&self, device: &wgpu::Device, pool: &mut TargetPool, w: u32, h: u32) -> Target {
        pool.take(
            device,
            w,
            h,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &mut TargetPool,
        scene: &Arc<Scene3d>,
        object: usize,
        depth_set: &[usize],
        background: bool,
        depth_surface: bool,
    ) -> Result<Rendered3d, Error3d> {
        let scene = &self.resolve_maps(device, queue, encoder, scene)?;
        let (w, h) = scene.canvas();
        self.attachments(device, w, h);
        let layer = Layer3d {
            scene,
            object,
            depth_set,
            background,
        };
        let cached = matches!(&self.prepass, Some(k) if Arc::ptr_eq(&k.scene, scene)
            && k.depth_set == depth_set && k.samples == self.samples);
        if !cached {
            self.prepass = None;
            self.keep_shadows(scene);
            for pass in describe::prepass(&layer)? {
                self.record(device, queue, encoder, scene, &pass, None)?;
            }
            self.prepass = Some(PrepassKey {
                scene: scene.clone(),
                depth_set: depth_set.to_vec(),
                samples: self.samples,
            });
        }
        let out = self.output(device, pool, w, h);
        for pass in describe::color_passes(&layer)? {
            self.record(device, queue, encoder, scene, &pass, Some(&out.view))?;
        }
        let depth = if depth_surface {
            Some(self.render_depth_surface(device, queue, encoder, pool, scene, &[object])?)
        } else {
            None
        };
        Ok(Rendered3d { color: out, depth })
    }

    fn render_whole(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &mut TargetPool,
        scene: &Arc<Scene3d>,
        depth_surface: bool,
    ) -> Result<Rendered3d, Error3d> {
        let scene = &self.resolve_maps(device, queue, encoder, scene)?;
        let (w, h) = scene.canvas();
        self.attachments(device, w, h);
        self.prepass = None;
        let out = self.output(device, pool, w, h);
        let passes = describe::whole_scene(scene)?;
        self.keep_shadows(scene);
        // The color pass clears depth too (`autoClear`).
        let a = self.attachments.as_ref().expect("attachments");
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3d clear depth"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &a.color,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &a.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        for pass in &passes {
            self.record(device, queue, encoder, scene, pass, Some(&out.view))?;
        }
        let depth = if depth_surface {
            let all: Vec<usize> = (0..scene.objects.len()).collect();
            Some(self.render_depth_surface(device, queue, encoder, pool, scene, &all)?)
        } else {
            None
        };
        Ok(Rendered3d { color: out, depth })
    }

    fn depth_surface_pipeline(&mut self, device: &wgpu::Device) {
        if self.depth_surface.is_some() {
            return;
        }
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("3d depth surface"),
            source: wgpu::ShaderSource::Wgsl(DEPTH_SURFACE_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3d depth surface"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(128),
                },
                count: None,
            }],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3d depth surface"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("3d depth surface"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    }],
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R32Float,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.depth_surface = Some(DepthSurface { pipeline, layout });
    }

    /// Linear view depth of `objects`' visible meshes (`r32float`, +inf
    /// where none drew), single-sampled, in GL row order.
    fn render_depth_surface(
        &mut self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &mut TargetPool,
        scene: &Scene3d,
        objects: &[usize],
    ) -> Result<Target, String> {
        let (w, h) = scene.canvas();
        self.depth_surface_pipeline(device);
        let view = scene.camera.view();
        let projection = scene.camera.projection_matrix(w, h);
        let mut draws = Vec::new();
        for &o in objects {
            let object = &scene.objects[o];
            if !object.visible {
                continue;
            }
            for mesh in &object.meshes {
                let gpu = self.mesh(device, &mesh.geometry, false);
                let mv: Mat4 = crate::three::math::multiply(&view, &mesh.world);
                let data: Vec<u8> = mv
                    .iter()
                    .chain(projection.iter())
                    .flat_map(|v| (*v as f32).to_le_bytes())
                    .collect();
                let ubuf = wgpu::util::DeviceExt::create_buffer_init(
                    device,
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("3d depth surface"),
                        contents: &data,
                        usage: wgpu::BufferUsages::UNIFORM,
                    },
                );
                let surface = self.depth_surface.as_ref().expect("pipeline");
                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("3d depth surface"),
                    layout: &surface.layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: ubuf.as_entire_binding(),
                    }],
                });
                draws.push((gpu, bind));
            }
        }
        let target = pool.take(
            device,
            w,
            h,
            wgpu::TextureFormat::R32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = pool.take(
            device,
            w,
            h,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3d depth surface"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::INFINITY,
                            g: 0.0,
                            b: 0.0,
                            a: 0.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let surface = self.depth_surface.as_ref().expect("pipeline");
            pass.set_pipeline(&surface.pipeline);
            for (mesh, bind) in &draws {
                pass.set_bind_group(0, bind, &[]);
                pass.set_vertex_buffer(0, mesh.position.slice(..));
                pass.set_index_buffer(mesh.index.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.count, 0, 0..1);
            }
        }
        pool.give(depth);
        Ok(target)
    }
}

/// A draw ready to record.
struct Prepared {
    pipeline: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
    buffers: Vec<wgpu::Buffer>,
    index: wgpu::Buffer,
    range: std::ops::Range<u32>,
    instances: u32,
}

impl Prepared {
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.range.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind, &[]);
        for (i, b) in self.buffers.iter().enumerate() {
            pass.set_vertex_buffer(i as u32, b.slice(..));
        }
        pass.set_index_buffer(self.index.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(self.range.clone(), 0, 0..self.instances);
    }
}
