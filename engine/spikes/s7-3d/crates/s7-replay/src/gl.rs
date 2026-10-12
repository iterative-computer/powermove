//! WebGL2 emulation on wgpu for the subset three.js r180 uses.
//!
//! Conventions (see `s7_glsl::VERTEX_EPILOGUE`): every texture and render target keeps GL
//! row order (row 0 = GL bottom row); vertex shaders negate clip y, so viewport and scissor
//! rectangles, `gl_FragCoord`, and derivatives keep their GL values, and GL's CCW front
//! faces become CW. Clip z is remapped from [-w, w] to [0, w].
//!
//! GL is immediate; wgpu records passes. Draws and clears are deferred into an open pass
//! for the bound draw framebuffer; anything that changes attachments, uploads texels, or
//! reads back closes and submits the pass first. Buffers are versioned: every
//! `bufferData`/`bufferSubData` creates a new GPU buffer, so deferred draws keep the data
//! they were issued with.
//!
//! Anything outside the supported subset is an error, never silently ignored. The only
//! deliberate no-ops are GL hints with no effect on pixels (`invalidateFramebuffer`,
//! `hint`, `lineWidth(1)`, deletes) and writes to uniforms the compiler eliminated.

use crate::consts::*;
use crate::trace::{Call, Trace};
use anyhow::{Context, Result, anyhow, bail};
use s7_glsl::ScalarKind;
use std::borrow::Cow;
use std::collections::HashMap;

/// One captured default-framebuffer image (top-down RGBA8 rows, premultiplied, as
/// `readPixels` returns them after flipping to image order).
pub struct Frame {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Default, Debug, Clone, serde::Serialize)]
pub struct ReplayStats {
    pub calls: usize,
    pub draws: usize,
    pub clears: usize,
    pub passes: usize,
    pub programs: usize,
    pub programs_overridden: usize,
    pub program_failures: Vec<String>,
    pub skipped_draws: usize,
    pub pipelines: usize,
    pub converted_attributes: usize,
    pub eliminated_uniform_writes: usize,
    pub blits: usize,
    pub mipmaps: usize,
}

/// A replacement for a three.js program (hand-written WGSL). Returning `Some` swaps the
/// compiled GLSL program for this one; uniform, texture, and attribute names must match.
pub type ProgramOverride = dyn Fn(&str, &str) -> Option<Result<s7_glsl::Program>>;

// ---------------------------------------------------------------------------------------
// Objects

struct BufferObj {
    data: Vec<u8>,
    version: u64,
    gpu: Option<(u64, wgpu::Buffer)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum TexKind {
    D2,
    Cube,
    D3,
    D2Array,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SamplerParams {
    min: u32,
    mag: u32,
    wrap_s: u32,
    wrap_t: u32,
    wrap_r: u32,
    compare: Option<u32>,
}

impl Default for SamplerParams {
    fn default() -> Self {
        Self { min: NEAREST_MIPMAP_LINEAR, mag: LINEAR, wrap_s: REPEAT, wrap_t: REPEAT, wrap_r: REPEAT, compare: None }
    }
}

struct TextureObj {
    kind: Option<TexKind>,
    format: wgpu::TextureFormat,
    internal: u32,
    width: u32,
    height: u32,
    depth: u32,
    levels: u32,
    immutable: bool,
    texture: Option<wgpu::Texture>,
    params: SamplerParams,
    compare_func: u32,
}

struct RenderbufferObj {
    texture: Option<wgpu::Texture>,
    format: wgpu::TextureFormat,
    samples: u32,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Attachment {
    Texture { id: u32, level: u32, layer: u32 },
    Renderbuffer { id: u32 },
}

#[derive(Default)]
struct FramebufferObj {
    colors: [Option<Attachment>; 8],
    depth: Option<Attachment>,
    draw_buffers: Vec<u32>,
}

#[derive(Clone, Copy, Debug, Default)]
struct AttribState {
    enabled: bool,
    buffer: u32,
    size: u32,
    ty: u32,
    normalized: bool,
    stride: u32,
    offset: u64,
    divisor: u32,
}

#[derive(Default, Clone)]
struct VaoObj {
    attribs: Vec<AttribState>,
    elements: u32,
}

struct ShaderObj {
    stage: u32,
    source: String,
}

struct ProgramObj {
    shaders: Vec<u32>,
    compiled: Option<std::rc::Rc<ProgramGpu>>,
    error: Option<String>,
    attrib_bindings: HashMap<String, u32>,
    attrib_locations: HashMap<String, u32>,
    block: Vec<u8>,
    sampler_units: HashMap<String, u32>,
}

struct ProgramGpu {
    program: s7_glsl::Program,
    vertex: wgpu::ShaderModule,
    fragment: wgpu::ShaderModule,
    overridden: bool,
}

// ---------------------------------------------------------------------------------------
// Passes

#[derive(Clone)]
struct TargetView {
    view: wgpu::TextureView,
    format: wgpu::TextureFormat,
}

#[derive(Clone)]
struct PassTarget {
    colors: Vec<Option<TargetView>>,
    depth: Option<TargetView>,
    width: u32,
    height: u32,
    samples: u32,
}

enum PassItem {
    Draw(DrawCmd),
    Quad(QuadCmd),
}

struct DrawCmd {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: Option<(u32, u32)>,
    textures: Vec<(u32, wgpu::TextureView)>,
    samplers: Vec<(u32, wgpu::Sampler)>,
    vertex_buffers: Vec<(wgpu::Buffer, u64)>,
    index: Option<(wgpu::Buffer, wgpu::IndexFormat)>,
    first: u32,
    count: u32,
    instances: u32,
    viewport: [f32; 4],
    scissor: [u32; 4],
}

struct QuadCmd {
    pipeline: wgpu::RenderPipeline,
    uniform: u32,
    scissor: [u32; 4],
}

struct OpenPass {
    target: PassTarget,
    color_clear: Vec<Option<wgpu::Color>>,
    depth_clear: Option<f32>,
    items: Vec<PassItem>,
    uniforms: Vec<u8>,
}

struct DefaultFramebuffer {
    width: u32,
    height: u32,
    samples: u32,
    color: wgpu::Texture,
    depth: Option<wgpu::Texture>,
    resolve: Option<wgpu::Texture>,
}

#[derive(Clone, Copy, Debug)]
struct BlendState {
    enabled: bool,
    eq_rgb: u32,
    eq_alpha: u32,
    src_rgb: u32,
    dst_rgb: u32,
    src_alpha: u32,
    dst_alpha: u32,
}

#[derive(Clone, Copy, Debug)]
struct State {
    blend: BlendState,
    depth_test: bool,
    depth_func: u32,
    depth_mask: bool,
    cull: bool,
    cull_face: u32,
    front_face: u32,
    color_mask: [bool; 4],
    scissor_test: bool,
    scissor: [i32; 4],
    viewport: [i32; 4],
    clear_color: [f64; 4],
    clear_depth: f64,
}

pub struct Replayer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub stats: ReplayStats,
    over: Option<Box<ProgramOverride>>,
    buffers: HashMap<u32, BufferObj>,
    textures: HashMap<u32, TextureObj>,
    renderbuffers: HashMap<u32, RenderbufferObj>,
    framebuffers: HashMap<u32, FramebufferObj>,
    vaos: HashMap<u32, VaoObj>,
    default_vao: VaoObj,
    shaders: HashMap<u32, ShaderObj>,
    programs: HashMap<u32, ProgramObj>,
    locations: HashMap<u32, (u32, String)>,
    // Bindings.
    program: u32,
    vao: u32,
    array_buffer: u32,
    active_unit: u32,
    units: Vec<[u32; 4]>,
    draw_fb: u32,
    read_fb: u32,
    renderbuffer: u32,
    state: State,
    unpack_alignment: u32,
    unpack_flip_y: bool,
    unpack_premultiply: bool,
    default_fb: Option<DefaultFramebuffer>,
    pass: Option<OpenPass>,
    // Caches.
    pipelines: HashMap<String, wgpu::RenderPipeline>,
    layouts: HashMap<String, (wgpu::BindGroupLayout, wgpu::PipelineLayout)>,
    samplers: HashMap<(SamplerParams, bool, bool), wgpu::Sampler>,
    converted: HashMap<String, wgpu::Buffer>,
    index_cache: HashMap<String, wgpu::Buffer>,
    dummies: HashMap<String, wgpu::TextureView>,
    constant_attrib: wgpu::Buffer,
    util: Option<Util>,
}

struct Util {
    mip_module: wgpu::ShaderModule,
    mip_layout: wgpu::BindGroupLayout,
    mip_pipeline_layout: wgpu::PipelineLayout,
    mip_pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    quad_module: wgpu::ShaderModule,
    quad_layout: wgpu::BindGroupLayout,
    quad_pipeline_layout: wgpu::PipelineLayout,
    quad_pipelines: HashMap<String, wgpu::RenderPipeline>,
}

const UTIL_WGSL: &str = r#"
struct VO { @builtin(position) p: vec4f, @location(0) uv: vec2f }
@vertex fn vs(@builtin(vertex_index) i: u32) -> VO {
  let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
  var o: VO;
  o.p = vec4f(uv * 2.0 - 1.0, 0.0, 1.0);
  o.uv = vec2f(uv.x, 1.0 - uv.y);
  return o;
}
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
@fragment fn fs(v: VO) -> @location(0) vec4f { return textureSampleLevel(t, s, v.uv, 0.0); }
"#;

const QUAD_WGSL: &str = r#"
struct C { color: vec4f, depth: vec4f }
@group(0) @binding(0) var<uniform> c: C;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
  let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
  return vec4f(uv * 2.0 - 1.0, c.depth.x, 1.0);
}
@fragment fn fs() -> @location(0) vec4f { return c.color; }
@fragment fn fs_depth() {}
"#;

/// Bytes appended to every GPU copy of a GL buffer (see `gpu_buffer`).
const VERTEX_PULL_PAD: usize = 256;

fn align(v: usize, a: usize) -> usize {
    v.div_ceil(a) * a
}

fn mip_count(w: u32, h: u32) -> u32 {
    32 - w.max(h).max(1).leading_zeros()
}

fn depth_format(f: wgpu::TextureFormat) -> bool {
    f.is_depth_stencil_format()
}

impl Replayer {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let constant_attrib = device.create_buffer(&wgpu::BufferDescriptor { label: Some("constant attrib"), size: 64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        queue.write_buffer(&constant_attrib, 0, bytemuck::cast_slice(&[0f32, 0.0, 0.0, 1.0]));
        let mut r = Self {
            device,
            queue,
            stats: ReplayStats::default(),
            over: None,
            buffers: HashMap::new(),
            textures: HashMap::new(),
            renderbuffers: HashMap::new(),
            framebuffers: HashMap::new(),
            vaos: HashMap::new(),
            default_vao: VaoObj::default(),
            shaders: HashMap::new(),
            programs: HashMap::new(),
            locations: HashMap::new(),
            program: 0,
            vao: 0,
            array_buffer: 0,
            active_unit: 0,
            units: vec![[0; 4]; 32],
            draw_fb: 0,
            read_fb: 0,
            renderbuffer: 0,
            state: State {
                blend: BlendState { enabled: false, eq_rgb: FUNC_ADD, eq_alpha: FUNC_ADD, src_rgb: ONE, dst_rgb: ZERO, src_alpha: ONE, dst_alpha: ZERO },
                depth_test: false,
                depth_func: LESS,
                depth_mask: true,
                cull: false,
                cull_face: BACK,
                front_face: CCW,
                color_mask: [true; 4],
                scissor_test: false,
                scissor: [0, 0, 0, 0],
                viewport: [0, 0, 0, 0],
                clear_color: [0.0; 4],
                clear_depth: 1.0,
            },
            unpack_alignment: 4,
            unpack_flip_y: false,
            unpack_premultiply: false,
            default_fb: None,
            pass: None,
            pipelines: HashMap::new(),
            layouts: HashMap::new(),
            samplers: HashMap::new(),
            converted: HashMap::new(),
            index_cache: HashMap::new(),
            dummies: HashMap::new(),
            constant_attrib,
            util: None,
        };
        r.util = Some(r.make_util());
        r
    }

    pub fn set_override(&mut self, over: Box<ProgramOverride>) {
        self.over = Some(over);
    }

    fn make_util(&self) -> Util {
        let d = &self.device;
        let mip_module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("mip"), source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(UTIL_WGSL)) });
        let mip_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mip"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let mip_pipeline_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("mip"), bind_group_layouts: &[Some(&mip_layout)], immediate_size: 0 });
        let quad_module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("clear quad"), source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(QUAD_WGSL)) });
        let quad_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("clear quad"),
            entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: None }, count: None }],
        });
        let quad_pipeline_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("clear quad"), bind_group_layouts: &[Some(&quad_layout)], immediate_size: 0 });
        Util { mip_module, mip_layout, mip_pipeline_layout, mip_pipelines: HashMap::new(), quad_module, quad_layout, quad_pipeline_layout, quad_pipelines: HashMap::new() }
    }

    /// Replays a whole trace. Returns one frame per `__mark` (the default framebuffer at
    /// the next mark or at the end).
    pub fn run(&mut self, trace: &Trace) -> Result<Vec<Frame>> {
        let mut frames = Vec::new();
        let mut current: Option<String> = None;
        for (index, entry) in trace.calls.iter().enumerate() {
            let call = Call::new(entry, &trace.blob)?;
            if call.name == "__mark" {
                if let Some(name) = current.take() {
                    frames.push(self.capture(&name)?);
                }
                current = Some(call.s(0)?.to_string());
                continue;
            }
            self.stats.calls += 1;
            self.call(&call).with_context(|| format!("call #{index} {}", serde_json::to_string(entry).unwrap_or_default().chars().take(300).collect::<String>()))?;
        }
        if let Some(name) = current.take() {
            frames.push(self.capture(&name)?);
        }
        self.flush()?;
        Ok(frames)
    }

    fn call(&mut self, c: &Call) -> Result<()> {
        match c.name {
            "__context" => {
                let attrs = &c.args[0];
                let w = c.u(1)?;
                let h = c.u(2)?;
                let antialias = attrs.get("antialias").and_then(|v| v.as_bool()).unwrap_or(true);
                let depth = attrs.get("depth").and_then(|v| v.as_bool()).unwrap_or(true);
                let stencil = attrs.get("stencil").and_then(|v| v.as_bool()).unwrap_or(false);
                if stencil {
                    bail!("stencil default framebuffer not supported");
                }
                if attrs.get("premultipliedAlpha").and_then(|v| v.as_bool()) == Some(false) {
                    bail!("premultipliedAlpha: false not supported");
                }
                let samples = if antialias { 4 } else { 1 };
                // S7_DEFAULT_SAMPLES overrides the canvas sample count (diagnostics only).
                let samples = std::env::var("S7_DEFAULT_SAMPLES").ok().and_then(|v| v.parse().ok()).unwrap_or(samples);
                self.make_default_fb(w, h, samples, depth);
                self.state.viewport = [0, 0, w as i32, h as i32];
                self.state.scissor = [0, 0, w as i32, h as i32];
            }
            "__canvas" => {
                let (w, h) = (c.u(0)?, c.u(1)?);
                let fb = self.default_fb.as_ref().ok_or_else(|| anyhow!("canvas before context"))?;
                let (fw, fh, samples, depth) = (fb.width, fb.height, fb.samples, fb.depth.is_some());
                if fw != w || fh != h {
                    self.flush()?;
                    self.make_default_fb(w, h, samples, depth);
                }
            }
            // Objects.
            "createBuffer" => {
                self.buffers.insert(c.ret_id(), BufferObj { data: Vec::new(), version: 0, gpu: None });
            }
            "createTexture" => {
                self.textures.insert(c.ret_id(), TextureObj { kind: None, format: wgpu::TextureFormat::Rgba8Unorm, internal: 0, width: 0, height: 0, depth: 0, levels: 0, immutable: false, texture: None, params: SamplerParams::default(), compare_func: LEQUAL });
            }
            "createFramebuffer" => {
                self.framebuffers.insert(c.ret_id(), FramebufferObj { draw_buffers: vec![COLOR_ATTACHMENT0], ..Default::default() });
            }
            "createRenderbuffer" => {
                self.renderbuffers.insert(c.ret_id(), RenderbufferObj { texture: None, format: wgpu::TextureFormat::Rgba8Unorm, samples: 1, width: 0, height: 0 });
            }
            "createVertexArray" => {
                self.vaos.insert(c.ret_id(), VaoObj::default());
            }
            "createShader" => {
                self.shaders.insert(c.ret_id(), ShaderObj { stage: c.u(0)?, source: String::new() });
            }
            "createProgram" => {
                self.programs.insert(c.ret_id(), ProgramObj { shaders: Vec::new(), compiled: None, error: None, attrib_bindings: HashMap::new(), attrib_locations: HashMap::new(), block: Vec::new(), sampler_units: HashMap::new() });
            }
            "deleteBuffer" | "deleteTexture" | "deleteFramebuffer" | "deleteRenderbuffer" | "deleteVertexArray" | "deleteShader" | "deleteProgram" => {
                // Objects are dropped with the replayer; ids are never reused by the recorder.
            }
            "shaderSource" => {
                let id = c.o(0)?;
                self.shaders.get_mut(&id).ok_or_else(|| anyhow!("no shader {id}"))?.source = c.s(1)?.to_string();
            }
            "compileShader" => {}
            "attachShader" => {
                let (p, s) = (c.o(0)?, c.o(1)?);
                self.programs.get_mut(&p).ok_or_else(|| anyhow!("no program"))?.shaders.push(s);
            }
            "bindAttribLocation" => {
                let p = c.o(0)?;
                let (index, name) = (c.u(1)?, c.s(2)?.to_string());
                self.programs.get_mut(&p).ok_or_else(|| anyhow!("no program"))?.attrib_bindings.insert(name, index);
            }
            "linkProgram" => self.link(c.o(0)?)?,
            "getAttribLocation" => {
                let p = c.o(0)?;
                let name = c.s(1)?.to_string();
                let loc = c.ret.and_then(|v| v.as_i64()).unwrap_or(-1);
                if loc >= 0 {
                    self.programs.get_mut(&p).ok_or_else(|| anyhow!("no program"))?.attrib_locations.insert(name, loc as u32);
                }
            }
            "getUniformLocation" => {
                let id = c.ret_id();
                if id != 0 {
                    self.locations.insert(id, (c.o(0)?, c.s(1)?.to_string()));
                }
            }
            "getActiveAttrib" | "getActiveUniform" | "getUniformBlockIndex" => {}
            "useProgram" => self.program = c.o(0)?,
            // Buffers.
            "bindBuffer" => {
                let (target, id) = (c.u(0)?, c.o(1)?);
                match target {
                    ARRAY_BUFFER => self.array_buffer = id,
                    ELEMENT_ARRAY_BUFFER => self.vao_mut().elements = id,
                    _ => bail!("bindBuffer target {target:#x}"),
                }
            }
            "bufferData" => {
                let target = c.u(0)?;
                let id = self.bound_buffer(target)?;
                let data = if let Ok((bytes, _)) = c.bytes(1) { bytes.to_vec() } else { vec![0u8; c.u(1)? as usize] };
                let b = self.buffers.get_mut(&id).ok_or_else(|| anyhow!("no buffer {id}"))?;
                b.data = data;
                b.version += 1;
            }
            "bufferSubData" => {
                let target = c.u(0)?;
                let id = self.bound_buffer(target)?;
                let offset = c.u(1)? as usize;
                let (bytes, _) = c.bytes(2)?;
                let b = self.buffers.get_mut(&id).ok_or_else(|| anyhow!("no buffer {id}"))?;
                if offset + bytes.len() > b.data.len() {
                    bail!("bufferSubData out of range");
                }
                b.data[offset..offset + bytes.len()].copy_from_slice(bytes);
                b.version += 1;
            }
            // Vertex arrays.
            "bindVertexArray" => self.vao = c.o(0)?,
            "enableVertexAttribArray" | "disableVertexAttribArray" => {
                let loc = c.u(0)? as usize;
                let on = c.name == "enableVertexAttribArray";
                let vao = self.vao_mut();
                if vao.attribs.len() <= loc {
                    vao.attribs.resize(loc + 1, AttribState::default());
                }
                vao.attribs[loc].enabled = on;
            }
            "vertexAttribPointer" => {
                let loc = c.u(0)? as usize;
                let buffer = self.array_buffer;
                if buffer == 0 {
                    bail!("vertexAttribPointer without an array buffer");
                }
                let st = AttribState { enabled: false, buffer, size: c.u(1)?, ty: c.u(2)?, normalized: c.b(3)?, stride: c.u(4)?, offset: c.u(5)? as u64, divisor: 0 };
                let vao = self.vao_mut();
                if vao.attribs.len() <= loc {
                    vao.attribs.resize(loc + 1, AttribState::default());
                }
                let a = &mut vao.attribs[loc];
                *a = AttribState { enabled: a.enabled, divisor: a.divisor, ..st };
            }
            "vertexAttribIPointer" => bail!("integer vertex attributes are not supported"),
            "vertexAttribDivisor" => {
                let loc = c.u(0)? as usize;
                let divisor = c.u(1)?;
                let vao = self.vao_mut();
                if vao.attribs.len() <= loc {
                    vao.attribs.resize(loc + 1, AttribState::default());
                }
                vao.attribs[loc].divisor = divisor;
            }
            // Textures.
            "activeTexture" => self.active_unit = c.u(0)? - TEXTURE0,
            "bindTexture" => {
                let (target, id) = (c.u(0)?, c.o(1)?);
                let slot = Self::target_slot(target)?;
                self.units[self.active_unit as usize][slot] = id;
                if id != 0 {
                    let kind = Self::target_kind(target)?;
                    let t = self.textures.get_mut(&id).ok_or_else(|| anyhow!("no texture {id}"))?;
                    if t.kind.is_none() {
                        t.kind = Some(kind);
                    } else if t.kind != Some(kind) {
                        bail!("texture {id} rebound to another target");
                    }
                }
            }
            "texParameteri" | "texParameterf" => {
                let id = self.bound_texture(c.u(0)?)?;
                let (pname, value) = (c.u(1)?, c.f(2)?);
                let t = self.textures.get_mut(&id).ok_or_else(|| anyhow!("no texture"))?;
                match pname {
                    TEXTURE_MIN_FILTER => t.params.min = value as u32,
                    TEXTURE_MAG_FILTER => t.params.mag = value as u32,
                    TEXTURE_WRAP_S => t.params.wrap_s = value as u32,
                    TEXTURE_WRAP_T => t.params.wrap_t = value as u32,
                    TEXTURE_WRAP_R => t.params.wrap_r = value as u32,
                    TEXTURE_COMPARE_MODE => {
                        t.params.compare = if value as u32 == COMPARE_REF_TO_TEXTURE { Some(t.compare_func) } else { None };
                    }
                    TEXTURE_COMPARE_FUNC => {
                        t.compare_func = value as u32;
                        if t.params.compare.is_some() {
                            t.params.compare = Some(value as u32);
                        }
                    }
                    TEXTURE_MAX_ANISOTROPY_EXT if value <= 1.0 => {}
                    _ => bail!("texParameter {pname:#x} = {value}"),
                }
            }
            "pixelStorei" => {
                let (pname, value) = (c.u(0)?, c.f(1)?);
                match pname {
                    UNPACK_ALIGNMENT => self.unpack_alignment = value as u32,
                    PACK_ALIGNMENT => {}
                    UNPACK_FLIP_Y_WEBGL => self.unpack_flip_y = value != 0.0,
                    UNPACK_PREMULTIPLY_ALPHA_WEBGL => self.unpack_premultiply = value != 0.0,
                    UNPACK_COLORSPACE_CONVERSION_WEBGL => {}
                    UNPACK_ROW_LENGTH | UNPACK_SKIP_ROWS | UNPACK_SKIP_PIXELS | UNPACK_IMAGE_HEIGHT | UNPACK_SKIP_IMAGES => {
                        if value != 0.0 {
                            bail!("pixelStorei {pname:#x} = {value} not supported");
                        }
                    }
                    _ => bail!("pixelStorei {pname:#x}"),
                }
            }
            "texImage2D" => self.tex_image(c, false)?,
            "texImage3D" => self.tex_image(c, true)?,
            "texSubImage2D" => self.tex_sub_image(c, false)?,
            "texSubImage3D" => self.tex_sub_image(c, true)?,
            "texStorage2D" | "texStorage3D" => {
                let target = c.u(0)?;
                let id = self.bound_texture(target)?;
                let levels = c.u(1)?;
                let internal = c.u(2)?;
                let (w, h) = (c.u(3)?, c.u(4)?);
                let d = if c.name == "texStorage3D" { c.u(5)? } else if Self::target_kind(target)? == TexKind::Cube { 6 } else { 1 };
                let format = Self::sized_format(internal)?;
                self.flush()?;
                self.alloc_texture(id, format, internal, w, h, d, levels, true)?;
            }
            "generateMipmap" => {
                let id = self.bound_texture(c.u(0)?)?;
                self.generate_mipmap(id)?;
            }
            "compressedTexImage2D" | "copyTexImage2D" | "copyTexSubImage2D" => bail!("{} not supported", c.name),
            // Framebuffers.
            "bindFramebuffer" => {
                let (target, id) = (c.u(0)?, c.o(1)?);
                match target {
                    FRAMEBUFFER => {
                        self.set_draw_fb(id)?;
                        self.read_fb = id;
                    }
                    DRAW_FRAMEBUFFER => self.set_draw_fb(id)?,
                    READ_FRAMEBUFFER => self.read_fb = id,
                    _ => bail!("bindFramebuffer target {target:#x}"),
                }
            }
            "framebufferTexture2D" | "framebufferTextureLayer" | "framebufferRenderbuffer" => {
                let target = c.u(0)?;
                let fb = if target == READ_FRAMEBUFFER { self.read_fb } else { self.draw_fb };
                if fb == 0 {
                    bail!("attachment on the default framebuffer");
                }
                let point = c.u(1)?;
                let attachment = match c.name {
                    "framebufferTexture2D" => {
                        let tex_target = c.u(2)?;
                        let id = c.o(3)?;
                        let level = c.u(4)?;
                        let layer = if (TEXTURE_CUBE_MAP_POSITIVE_X..TEXTURE_CUBE_MAP_POSITIVE_X + 6).contains(&tex_target) { tex_target - TEXTURE_CUBE_MAP_POSITIVE_X } else { 0 };
                        (id != 0).then_some(Attachment::Texture { id, level, layer })
                    }
                    "framebufferTextureLayer" => {
                        let id = c.o(2)?;
                        (id != 0).then_some(Attachment::Texture { id, level: c.u(3)?, layer: c.u(4)? })
                    }
                    _ => {
                        let id = c.o(3)?;
                        (id != 0).then_some(Attachment::Renderbuffer { id })
                    }
                };
                if fb == self.draw_fb {
                    self.flush()?;
                }
                let f = self.framebuffers.get_mut(&fb).ok_or_else(|| anyhow!("no framebuffer {fb}"))?;
                match point {
                    DEPTH_ATTACHMENT | DEPTH_STENCIL_ATTACHMENT => f.depth = attachment,
                    STENCIL_ATTACHMENT => bail!("stencil attachments not supported"),
                    p if (COLOR_ATTACHMENT0..COLOR_ATTACHMENT0 + 8).contains(&p) => f.colors[(p - COLOR_ATTACHMENT0) as usize] = attachment,
                    p => bail!("attachment point {p:#x}"),
                }
            }
            "drawBuffers" => {
                let list = c.list(0)?;
                self.flush()?;
                if self.draw_fb == 0 {
                    if list != [BACK] {
                        bail!("drawBuffers {list:?} on the default framebuffer");
                    }
                } else {
                    self.framebuffers.get_mut(&self.draw_fb).ok_or_else(|| anyhow!("no framebuffer"))?.draw_buffers = list;
                }
            }
            "bindRenderbuffer" => self.renderbuffer = c.o(1)?,
            "renderbufferStorage" | "renderbufferStorageMultisample" => {
                let multi = c.name == "renderbufferStorageMultisample";
                let samples = if multi { c.u(1)? } else { 0 };
                let i = if multi { 2 } else { 1 };
                let internal = c.u(i)?;
                let (w, h) = (c.u(i + 1)?, c.u(i + 2)?);
                let format = Self::sized_format(internal)?;
                let samples = match samples {
                    0 | 1 => 1,
                    2..=4 => 4,
                    s => bail!("{s} samples"),
                };
                let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("renderbuffer"),
                    size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let id = self.renderbuffer;
                let rb = self.renderbuffers.get_mut(&id).ok_or_else(|| anyhow!("no renderbuffer {id}"))?;
                *rb = RenderbufferObj { texture: Some(texture), format, samples, width: w, height: h };
            }
            "blitFramebuffer" => self.blit(c)?,
            "invalidateFramebuffer" => {}
            // State.
            "enable" | "disable" => {
                let on = c.name == "enable";
                match c.u(0)? {
                    BLEND => self.state.blend.enabled = on,
                    DEPTH_TEST => self.state.depth_test = on,
                    CULL_FACE => self.state.cull = on,
                    SCISSOR_TEST => self.state.scissor_test = on,
                    DITHER => {}
                    STENCIL_TEST | POLYGON_OFFSET_FILL | SAMPLE_ALPHA_TO_COVERAGE | SAMPLE_COVERAGE | RASTERIZER_DISCARD if !on => {}
                    cap => bail!("{} {cap:#x} not supported", c.name),
                }
            }
            "blendEquation" => {
                let e = c.u(0)?;
                self.state.blend.eq_rgb = e;
                self.state.blend.eq_alpha = e;
            }
            "blendEquationSeparate" => {
                self.state.blend.eq_rgb = c.u(0)?;
                self.state.blend.eq_alpha = c.u(1)?;
            }
            "blendFunc" => {
                let (s, d) = (c.u(0)?, c.u(1)?);
                let b = &mut self.state.blend;
                (b.src_rgb, b.dst_rgb, b.src_alpha, b.dst_alpha) = (s, d, s, d);
            }
            "blendFuncSeparate" => {
                let b = &mut self.state.blend;
                (b.src_rgb, b.dst_rgb, b.src_alpha, b.dst_alpha) = (c.u(0)?, c.u(1)?, c.u(2)?, c.u(3)?);
            }
            "blendColor" => bail!("blendColor not supported"),
            "depthFunc" => self.state.depth_func = c.u(0)?,
            "depthMask" => self.state.depth_mask = c.b(0)?,
            "depthRange" => {
                if c.f(0)? != 0.0 || c.f(1)? != 1.0 {
                    bail!("depthRange not supported");
                }
            }
            "cullFace" => self.state.cull_face = c.u(0)?,
            "frontFace" => self.state.front_face = c.u(0)?,
            "colorMask" => self.state.color_mask = [c.b(0)?, c.b(1)?, c.b(2)?, c.b(3)?],
            "stencilMask" | "stencilFunc" | "stencilOp" | "stencilMaskSeparate" | "stencilFuncSeparate" | "stencilOpSeparate" | "clearStencil" => {}
            "scissor" => self.state.scissor = [c.i(0)?, c.i(1)?, c.i(2)?, c.i(3)?],
            "viewport" => self.state.viewport = [c.i(0)?, c.i(1)?, c.i(2)?, c.i(3)?],
            "clearColor" => self.state.clear_color = [c.f(0)?, c.f(1)?, c.f(2)?, c.f(3)?],
            "clearDepth" => self.state.clear_depth = c.f(0)?,
            "lineWidth" => {
                if c.f(0)? != 1.0 {
                    bail!("lineWidth != 1");
                }
            }
            "polygonOffset" => bail!("polygonOffset not supported"),
            "hint" | "pixelStoref" => {}
            "clear" => self.clear(c.u(0)?)?,
            // Uniforms.
            n if n.starts_with("uniform") => self.uniform(c)?,
            "drawArrays" => self.draw(c.u(0)?, c.u(1)?, c.u(2)?, None, 1)?,
            "drawArraysInstanced" => self.draw(c.u(0)?, c.u(1)?, c.u(2)?, None, c.u(3)?)?,
            "drawElements" => self.draw(c.u(0)?, 0, c.u(1)?, Some((c.u(2)?, c.u(3)? as u64)), 1)?,
            "drawElementsInstanced" => self.draw(c.u(0)?, 0, c.u(1)?, Some((c.u(2)?, c.u(3)? as u64)), c.u(4)?)?,
            "drawRangeElements" => self.draw(c.u(0)?, 0, c.u(3)?, Some((c.u(4)?, c.u(5)? as u64)), 1)?,
            other => bail!("unsupported call {other}"),
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------
    // Helpers

    fn vao_mut(&mut self) -> &mut VaoObj {
        if self.vao == 0 { &mut self.default_vao } else { self.vaos.get_mut(&self.vao).expect("vao") }
    }

    fn vao_ref(&self) -> &VaoObj {
        if self.vao == 0 { &self.default_vao } else { &self.vaos[&self.vao] }
    }

    fn bound_buffer(&self, target: u32) -> Result<u32> {
        let id = match target {
            ARRAY_BUFFER => self.array_buffer,
            ELEMENT_ARRAY_BUFFER => self.vao_ref().elements,
            _ => bail!("buffer target {target:#x}"),
        };
        if id == 0 {
            bail!("no buffer bound to {target:#x}");
        }
        Ok(id)
    }

    fn target_slot(target: u32) -> Result<usize> {
        Ok(match target {
            TEXTURE_2D => 0,
            TEXTURE_CUBE_MAP => 1,
            TEXTURE_3D => 2,
            TEXTURE_2D_ARRAY => 3,
            t if (TEXTURE_CUBE_MAP_POSITIVE_X..TEXTURE_CUBE_MAP_POSITIVE_X + 6).contains(&t) => 1,
            t => bail!("texture target {t:#x}"),
        })
    }

    fn target_kind(target: u32) -> Result<TexKind> {
        Ok([TexKind::D2, TexKind::Cube, TexKind::D3, TexKind::D2Array][Self::target_slot(target)?])
    }

    fn bound_texture(&self, target: u32) -> Result<u32> {
        let id = self.units[self.active_unit as usize][Self::target_slot(target)?];
        if id == 0 {
            bail!("no texture bound to {target:#x} on unit {}", self.active_unit);
        }
        Ok(id)
    }

    fn sized_format(internal: u32) -> Result<wgpu::TextureFormat> {
        use wgpu::TextureFormat as F;
        Ok(match internal {
            RGBA8 | RGB8 => F::Rgba8Unorm,
            SRGB8_ALPHA8 | SRGB8 => F::Rgba8UnormSrgb,
            RGBA16F | RGB16F => F::Rgba16Float,
            RGBA32F | RGB32F => F::Rgba32Float,
            R8 => F::R8Unorm,
            RG8 => F::Rg8Unorm,
            R16F => F::R16Float,
            RG16F => F::Rg16Float,
            R32F => F::R32Float,
            RG32F => F::Rg32Float,
            DEPTH_COMPONENT16 => F::Depth16Unorm,
            DEPTH_COMPONENT24 | DEPTH_COMPONENT32F => F::Depth32Float,
            DEPTH24_STENCIL8 | DEPTH32F_STENCIL8 => F::Depth24PlusStencil8,
            f => bail!("internal format {f:#x}"),
        })
    }

    /// Destination format and internal format for a (possibly unsized) upload.
    fn upload_format(internal: u32, format: u32, ty: u32) -> Result<(wgpu::TextureFormat, u32)> {
        let sized = match (internal, format, ty) {
            (RGBA, RGBA, UNSIGNED_BYTE) => RGBA8,
            (RGB, RGB, UNSIGNED_BYTE) => RGB8,
            (RGBA, RGBA, FLOAT) => RGBA32F,
            (RGBA, RGBA, HALF_FLOAT) => RGBA16F,
            (DEPTH_COMPONENT, DEPTH_COMPONENT, UNSIGNED_SHORT) => DEPTH_COMPONENT16,
            (DEPTH_COMPONENT, DEPTH_COMPONENT, UNSIGNED_INT) => DEPTH_COMPONENT24,
            (DEPTH_STENCIL, DEPTH_STENCIL, UNSIGNED_INT_24_8) => DEPTH24_STENCIL8,
            (LUMINANCE | ALPHA | LUMINANCE_ALPHA, _, _) => bail!("luminance/alpha textures not supported"),
            (i, _, _) => i,
        };
        Ok((Self::sized_format(sized)?, sized))
    }

    /// Converts client pixels (`format`, `ty`, rows of `w` with the unpack alignment) into
    /// tightly packed texels of `dst`.
    fn convert_pixels(&self, src: &[u8], w: u32, h: u32, d: u32, format: u32, ty: u32, internal: u32, dst: wgpu::TextureFormat, flip: bool) -> Result<Vec<u8>> {
        let comps = match format {
            RGBA | RGBA_INTEGER => 4,
            RGB | RGB_INTEGER => 3,
            RG | RG_INTEGER => 2,
            RED | RED_INTEGER | DEPTH_COMPONENT => 1,
            f => bail!("pixel format {f:#x}"),
        };
        let size = match ty {
            UNSIGNED_BYTE | BYTE => 1,
            UNSIGNED_SHORT | SHORT | HALF_FLOAT => 2,
            UNSIGNED_INT | INT | FLOAT => 4,
            t => bail!("pixel type {t:#x}"),
        };
        let pixel = comps * size;
        let row = align((w as usize) * pixel, self.unpack_alignment.max(1) as usize);
        let dst_bpp = dst.block_copy_size(None).ok_or_else(|| anyhow!("format size"))? as usize;
        let mut out = vec![0u8; (w * h * d) as usize * dst_bpp];
        let rows = (h * d) as usize;
        if src.len() < row * (rows - 1) + (w as usize) * pixel {
            bail!("pixel data too short: {} for {w}x{h}x{d}", src.len());
        }
        use wgpu::TextureFormat as F;
        for r in 0..rows {
            let (z, y) = (r / h as usize, r % h as usize);
            let sy = if flip { h as usize - 1 - y } else { y };
            let srow = &src[(z * h as usize + sy) * row..];
            let drow = &mut out[r * w as usize * dst_bpp..(r + 1) * w as usize * dst_bpp];
            for x in 0..w as usize {
                let s = &srow[x * pixel..(x + 1) * pixel];
                let dpx = &mut drow[x * dst_bpp..(x + 1) * dst_bpp];
                match (dst, ty) {
                    (F::Rgba8Unorm | F::Rgba8UnormSrgb, UNSIGNED_BYTE) => {
                        dpx[..comps].copy_from_slice(s);
                        if comps == 3 {
                            dpx[3] = 255;
                        }
                    }
                    (F::R8Unorm | F::Rg8Unorm, UNSIGNED_BYTE) => dpx.copy_from_slice(s),
                    (F::Rgba32Float | F::R32Float | F::Rg32Float, FLOAT) => {
                        dpx[..comps * 4].copy_from_slice(s);
                        if comps == 3 {
                            dpx[12..16].copy_from_slice(&1f32.to_le_bytes());
                        }
                    }
                    (F::Rgba16Float | F::R16Float | F::Rg16Float, HALF_FLOAT) => {
                        dpx[..comps * 2].copy_from_slice(s);
                        if comps == 3 {
                            dpx[6..8].copy_from_slice(&half::f16::ONE.to_le_bytes());
                        }
                    }
                    (F::Rgba16Float | F::R16Float | F::Rg16Float, FLOAT) => {
                        for k in 0..comps {
                            let v = f32::from_le_bytes(s[k * 4..k * 4 + 4].try_into().unwrap());
                            dpx[k * 2..k * 2 + 2].copy_from_slice(&half::f16::from_f32(v).to_le_bytes());
                        }
                        if comps == 3 {
                            dpx[6..8].copy_from_slice(&half::f16::ONE.to_le_bytes());
                        }
                    }
                    _ => bail!("upload {dst:?} from type {ty:#x} (internal {internal:#x})"),
                }
            }
        }
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn alloc_texture(&mut self, id: u32, format: wgpu::TextureFormat, internal: u32, w: u32, h: u32, d: u32, levels: u32, immutable: bool) -> Result<()> {
        let t = self.textures.get_mut(&id).ok_or_else(|| anyhow!("no texture {id}"))?;
        let kind = t.kind.ok_or_else(|| anyhow!("texture {id} never bound"))?;
        let dimension = if kind == TexKind::D3 { wgpu::TextureDimension::D3 } else { wgpu::TextureDimension::D2 };
        let mut usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC;
        if kind != TexKind::D3 {
            usage |= wgpu::TextureUsages::RENDER_ATTACHMENT;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gl texture"),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: d.max(1) },
            mip_level_count: levels.max(1),
            sample_count: 1,
            dimension,
            format,
            usage,
            view_formats: &[],
        });
        *t = TextureObj { kind: Some(kind), format, internal, width: w, height: h, depth: d, levels: levels.max(1), immutable, texture: Some(texture), params: t.params, compare_func: t.compare_func };
        Ok(())
    }

    fn tex_image(&mut self, c: &Call, three_d: bool) -> Result<()> {
        let target = c.u(0)?;
        let id = self.bound_texture(target)?;
        let level = c.u(1)?;
        let internal = c.u(2)?;
        let kind = Self::target_kind(target)?;
        let layer = if (TEXTURE_CUBE_MAP_POSITIVE_X..TEXTURE_CUBE_MAP_POSITIVE_X + 6).contains(&target) { target - TEXTURE_CUBE_MAP_POSITIVE_X } else { 0 };
        // (w, h, d, format, type, data index)
        let (w, h, d, format, ty, data_index) = if three_d {
            (c.u(3)?, c.u(4)?, c.u(5)?, c.u(7)?, c.u(8)?, 9)
        } else if c.len() == 6 {
            let (w, h, _) = c.image(5).ok_or_else(|| anyhow!("texImage2D source is not an image"))?;
            (w, h, 1, c.u(3)?, c.u(4)?, 5)
        } else {
            (c.u(3)?, c.u(4)?, 1, c.u(6)?, c.u(7)?, 8)
        };
        let (dst, sized) = Self::upload_format(internal, format, ty)?;
        let layers = if kind == TexKind::Cube { 6 } else { d };
        self.flush()?;
        let needs_alloc = {
            let t = &self.textures[&id];
            if t.immutable {
                bail!("texImage on an immutable texture");
            }
            t.texture.is_none() || (level == 0 && (t.width != w || t.height != h || t.depth != layers || t.format != dst))
        };
        if needs_alloc {
            if level != 0 {
                bail!("texImage level {level} before level 0");
            }
            let levels = if kind == TexKind::D2 || kind == TexKind::Cube { mip_count(w, h) } else { 1 };
            self.alloc_texture(id, dst, sized, w, h, layers, levels, false)?;
        }
        let bytes: Option<Vec<u8>> = if let Some((iw, ih, px)) = c.image(data_index) {
            if iw != w || ih != h {
                bail!("image size mismatch");
            }
            // Snapshots already hold what GL received (flip and premultiply applied).
            let mut v = px.to_vec();
            if matches!(format, RGB) {
                for p in v.chunks_mut(4) {
                    p[3] = 255;
                }
            }
            if dst != wgpu::TextureFormat::Rgba8Unorm && dst != wgpu::TextureFormat::Rgba8UnormSrgb {
                bail!("image upload into {dst:?}");
            }
            Some(v)
        } else if c.is_null(data_index) {
            None
        } else {
            let (src, _) = c.bytes(data_index)?;
            if self.unpack_premultiply {
                bail!("premultiply on typed array upload");
            }
            Some(self.convert_pixels(src, w, h, d, format, ty, sized, dst, self.unpack_flip_y && !three_d)?)
        };
        if let Some(bytes) = bytes {
            self.write_texels(id, level, [0, 0, if kind == TexKind::Cube { layer } else { 0 }], [w, h, d], &bytes)?;
        }
        Ok(())
    }

    fn tex_sub_image(&mut self, c: &Call, three_d: bool) -> Result<()> {
        let target = c.u(0)?;
        let id = self.bound_texture(target)?;
        let level = c.u(1)?;
        let layer = if (TEXTURE_CUBE_MAP_POSITIVE_X..TEXTURE_CUBE_MAP_POSITIVE_X + 6).contains(&target) { target - TEXTURE_CUBE_MAP_POSITIVE_X } else { 0 };
        let (x, y, z, w, h, d, format, ty, data_index) = if three_d {
            (c.u(2)?, c.u(3)?, c.u(4)?, c.u(5)?, c.u(6)?, c.u(7)?, c.u(8)?, c.u(9)?, 10)
        } else if c.len() == 7 {
            let (w, h, _) = c.image(6).ok_or_else(|| anyhow!("texSubImage2D source is not an image"))?;
            (c.u(2)?, c.u(3)?, layer, w, h, 1, c.u(4)?, c.u(5)?, 6)
        } else {
            (c.u(2)?, c.u(3)?, layer, c.u(4)?, c.u(5)?, 1, c.u(6)?, c.u(7)?, 8)
        };
        let (dst, internal) = {
            let t = self.textures.get(&id).ok_or_else(|| anyhow!("no texture"))?;
            if t.texture.is_none() {
                bail!("texSubImage before storage");
            }
            (t.format, t.internal)
        };
        self.flush()?;
        let bytes = if let Some((_, _, px)) = c.image(data_index) {
            if dst != wgpu::TextureFormat::Rgba8Unorm && dst != wgpu::TextureFormat::Rgba8UnormSrgb {
                bail!("image sub-upload into {dst:?}");
            }
            px.to_vec()
        } else {
            let (src, _) = c.bytes(data_index)?;
            if self.unpack_premultiply {
                bail!("premultiply on typed array upload");
            }
            self.convert_pixels(src, w, h, d, format, ty, internal, dst, self.unpack_flip_y && !three_d)?
        };
        self.write_texels(id, level, [x, y, z], [w, h, d], &bytes)
    }

    fn write_texels(&mut self, id: u32, level: u32, origin: [u32; 3], size: [u32; 3], bytes: &[u8]) -> Result<()> {
        let t = &self.textures[&id];
        let texture = t.texture.as_ref().unwrap();
        let bpp = t.format.block_copy_size(None).unwrap();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture, mip_level: level, origin: wgpu::Origin3d { x: origin[0], y: origin[1], z: origin[2] }, aspect: wgpu::TextureAspect::All },
            bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size[0] * bpp), rows_per_image: Some(size[1]) },
            wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: size[2] },
        );
        Ok(())
    }

    fn generate_mipmap(&mut self, id: u32) -> Result<()> {
        self.flush()?;
        self.stats.mipmaps += 1;
        let (texture, format, levels, kind) = {
            let t = &self.textures[&id];
            (t.texture.clone().ok_or_else(|| anyhow!("mipmap of empty texture"))?, t.format, t.levels, t.kind)
        };
        if kind != Some(TexKind::D2) {
            bail!("generateMipmap on {kind:?}");
        }
        let util = self.util.as_mut().unwrap();
        let pipeline = util
            .mip_pipelines
            .entry(format)
            .or_insert_with(|| {
                self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("mip"),
                    layout: Some(&util.mip_pipeline_layout),
                    vertex: wgpu::VertexState { module: &util.mip_module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState { module: &util.mip_module, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[Some(format.into())] }),
                    multiview_mask: None,
                    cache: None,
                })
            })
            .clone();
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for level in 1..levels {
            let src = texture.create_view(&wgpu::TextureViewDescriptor { base_mip_level: level - 1, mip_level_count: Some(1), dimension: Some(wgpu::TextureViewDimension::D2), ..Default::default() });
            let dst = texture.create_view(&wgpu::TextureViewDescriptor { base_mip_level: level, mip_level_count: Some(1), dimension: Some(wgpu::TextureViewDimension::D2), ..Default::default() });
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.util.as_ref().unwrap().mip_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&src) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mip"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &dst, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        Ok(())
    }

    fn make_default_fb(&mut self, w: u32, h: u32, samples: u32, depth: bool) {
        let make = |format, samples, label| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let color = make(wgpu::TextureFormat::Rgba8Unorm, samples, "default color");
        let depth = depth.then(|| make(wgpu::TextureFormat::Depth32Float, samples, "default depth"));
        let resolve = (samples > 1).then(|| make(wgpu::TextureFormat::Rgba8Unorm, 1, "default resolve"));
        self.default_fb = Some(DefaultFramebuffer { width: w, height: h, samples, color, depth, resolve });
        // A new drawing buffer starts cleared (transparent black, depth 1).
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let fb = self.default_fb.as_ref().unwrap();
        let cv = fb.color.create_view(&Default::default());
        let dv = fb.depth.as_ref().map(|t| t.create_view(&Default::default()));
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("init"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &cv, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: dv.as_ref().map(|v| wgpu::RenderPassDepthStencilAttachment { view: v, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        self.queue.submit([encoder.finish()]);
    }

    fn set_draw_fb(&mut self, id: u32) -> Result<()> {
        if id != self.draw_fb {
            self.flush()?;
            self.draw_fb = id;
        }
        Ok(())
    }

    fn attachment_view(&self, a: Attachment) -> Result<(TargetView, u32, u32, u32)> {
        Ok(match a {
            Attachment::Texture { id, level, layer } => {
                let t = self.textures.get(&id).ok_or_else(|| anyhow!("no texture {id}"))?;
                let tex = t.texture.as_ref().ok_or_else(|| anyhow!("attachment texture {id} has no storage"))?;
                let view = tex.create_view(&wgpu::TextureViewDescriptor { base_mip_level: level, mip_level_count: Some(1), base_array_layer: layer, array_layer_count: Some(1), dimension: Some(wgpu::TextureViewDimension::D2), ..Default::default() });
                (TargetView { view, format: t.format }, (t.width >> level).max(1), (t.height >> level).max(1), 1)
            }
            Attachment::Renderbuffer { id } => {
                let r = self.renderbuffers.get(&id).ok_or_else(|| anyhow!("no renderbuffer {id}"))?;
                let tex = r.texture.as_ref().ok_or_else(|| anyhow!("renderbuffer {id} has no storage"))?;
                (TargetView { view: tex.create_view(&Default::default()), format: r.format }, r.width, r.height, r.samples)
            }
        })
    }

    fn current_target(&self) -> Result<PassTarget> {
        if self.draw_fb == 0 {
            let fb = self.default_fb.as_ref().ok_or_else(|| anyhow!("no default framebuffer"))?;
            return Ok(PassTarget {
                colors: vec![Some(TargetView { view: fb.color.create_view(&Default::default()), format: wgpu::TextureFormat::Rgba8Unorm })],
                depth: fb.depth.as_ref().map(|t| TargetView { view: t.create_view(&Default::default()), format: wgpu::TextureFormat::Depth32Float }),
                width: fb.width,
                height: fb.height,
                samples: fb.samples,
            });
        }
        let f = self.framebuffers.get(&self.draw_fb).ok_or_else(|| anyhow!("no framebuffer {}", self.draw_fb))?;
        let mut colors = Vec::new();
        let mut size = None;
        let mut check = |w: u32, h: u32, s: u32| -> Result<()> {
            match size {
                None => size = Some((w, h, s)),
                Some(prev) if prev != (w, h, s) => bail!("framebuffer attachments differ in size or samples"),
                _ => {}
            }
            Ok(())
        };
        for &db in &f.draw_buffers {
            if db == NONE {
                colors.push(None);
                continue;
            }
            let i = (db - COLOR_ATTACHMENT0) as usize;
            match f.colors.get(i).copied().flatten() {
                Some(a) => {
                    let (v, w, h, s) = self.attachment_view(a)?;
                    check(w, h, s)?;
                    colors.push(Some(v));
                }
                None => colors.push(None),
            }
        }
        while colors.last().is_some_and(|c| c.is_none()) {
            colors.pop();
        }
        let depth = match f.depth {
            Some(a) => {
                let (v, w, h, s) = self.attachment_view(a)?;
                check(w, h, s)?;
                Some(v)
            }
            None => None,
        };
        let (width, height, samples) = size.ok_or_else(|| anyhow!("framebuffer {} has no attachments", self.draw_fb))?;
        Ok(PassTarget { colors, depth, width, height, samples })
    }

    fn open_pass(&mut self) -> Result<&mut OpenPass> {
        if self.pass.is_none() {
            let target = self.current_target()?;
            let n = target.colors.len();
            self.pass = Some(OpenPass { target, color_clear: vec![None; n], depth_clear: None, items: Vec::new(), uniforms: Vec::new() });
        }
        Ok(self.pass.as_mut().unwrap())
    }

    fn scissor_rect(&self, target: &PassTarget) -> [u32; 4] {
        if !self.state.scissor_test {
            return [0, 0, target.width, target.height];
        }
        let [x, y, w, h] = self.state.scissor;
        let x0 = x.clamp(0, target.width as i32) as u32;
        let y0 = y.clamp(0, target.height as i32) as u32;
        let x1 = (x + w).clamp(0, target.width as i32) as u32;
        let y1 = (y + h).clamp(0, target.height as i32) as u32;
        [x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0)]
    }

    fn clear(&mut self, bits: u32) -> Result<()> {
        self.stats.clears += 1;
        let color = bits & COLOR_BUFFER_BIT != 0 && self.state.color_mask.iter().any(|m| *m);
        let depth = bits & DEPTH_BUFFER_BIT != 0 && self.state.depth_mask;
        if !color && !depth {
            return Ok(());
        }
        let state = self.state;
        let pass_target = self.open_pass()?.target.clone();
        let scissor = self.scissor_rect(&pass_target);
        let full_scissor = scissor == [0, 0, pass_target.width, pass_target.height];
        let full_mask = state.color_mask.iter().all(|m| *m);
        let [r, g, b, a] = state.clear_color;
        let pass = self.pass.as_mut().unwrap();
        if pass.items.is_empty() && full_scissor && (!color || full_mask) {
            if color {
                for (i, c) in pass.target.colors.iter().enumerate() {
                    if c.is_some() {
                        pass.color_clear[i] = Some(wgpu::Color { r, g, b, a });
                    }
                }
            }
            if depth && pass.target.depth.is_some() {
                pass.depth_clear = Some(state.clear_depth as f32);
            }
            return Ok(());
        }
        if scissor[2] == 0 || scissor[3] == 0 {
            return Ok(());
        }
        // Masked or scissored clear, or a clear after draws: a full-target quad.
        if pass.target.colors.len() > 1 {
            bail!("masked clear with multiple color targets");
        }
        let color_format = pass.target.colors.first().cloned().flatten().map(|v| v.format);
        let depth_format_ = pass.target.depth.as_ref().map(|v| v.format);
        let samples = pass.target.samples;
        let mask = if color { state.color_mask } else { [false; 4] };
        let key = format!("{color_format:?}|{depth_format_:?}|{samples}|{mask:?}|{depth}");
        let util = self.util.as_mut().unwrap();
        let pipeline = util
            .quad_pipelines
            .entry(key)
            .or_insert_with(|| {
                let mut write = wgpu::ColorWrites::empty();
                for (i, w) in [wgpu::ColorWrites::RED, wgpu::ColorWrites::GREEN, wgpu::ColorWrites::BLUE, wgpu::ColorWrites::ALPHA].iter().enumerate() {
                    if mask[i] {
                        write |= *w;
                    }
                }
                let targets = [color_format.map(|f| wgpu::ColorTargetState { format: f, blend: None, write_mask: write })];
                self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("clear quad"),
                    layout: Some(&util.quad_pipeline_layout),
                    vertex: wgpu::VertexState { module: &util.quad_module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: depth_format_.map(|f| wgpu::DepthStencilState { format: f, depth_write_enabled: Some(depth), depth_compare: Some(wgpu::CompareFunction::Always), stencil: Default::default(), bias: Default::default() }),
                    multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
                    fragment: Some(wgpu::FragmentState { module: &util.quad_module, entry_point: Some(if color_format.is_some() { "fs" } else { "fs_depth" }), compilation_options: Default::default(), targets: if color_format.is_some() { &targets } else { &[] } }),
                    multiview_mask: None,
                    cache: None,
                })
            })
            .clone();
        let pass = self.pass.as_mut().unwrap();
        let offset = align(pass.uniforms.len(), 256);
        pass.uniforms.resize(offset, 0);
        for v in [r as f32, g as f32, b as f32, a as f32, state.clear_depth as f32, 0.0, 0.0, 0.0] {
            pass.uniforms.extend_from_slice(&v.to_le_bytes());
        }
        pass.items.push(PassItem::Quad(QuadCmd { pipeline, uniform: offset as u32, scissor }));
        Ok(())
    }

    fn blit(&mut self, c: &Call) -> Result<()> {
        self.flush()?;
        self.stats.blits += 1;
        let src_rect = [c.i(0)?, c.i(1)?, c.i(2)?, c.i(3)?];
        let dst_rect = [c.i(4)?, c.i(5)?, c.i(6)?, c.i(7)?];
        let mask = c.u(8)?;
        if src_rect != dst_rect {
            bail!("scaled blit not supported");
        }
        if mask != COLOR_BUFFER_BIT {
            bail!("blit mask {mask:#x} not supported");
        }
        let src = self.framebuffers.get(&self.read_fb).ok_or_else(|| anyhow!("blit from default framebuffer"))?;
        let src_att = src.colors[0].ok_or_else(|| anyhow!("blit source has no color"))?;
        let dst = self.framebuffers.get(&self.draw_fb).ok_or_else(|| anyhow!("blit to default framebuffer"))?;
        let dst_att = dst.colors[0].ok_or_else(|| anyhow!("blit destination has no color"))?;
        let (sv, sw, sh, ss) = self.attachment_view(src_att)?;
        let (dv, dw, dh, ds) = self.attachment_view(dst_att)?;
        if (sw, sh) != (dw, dh) || src_rect != [0, 0, sw as i32, sh as i32] || sv.format != dv.format || ds != 1 {
            bail!("blit must be a full-size resolve or copy with matching formats");
        }
        let mut encoder = self.device.create_command_encoder(&Default::default());
        if ss > 1 {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit resolve"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &sv.view, depth_slice: None, resolve_target: Some(&dv.view), ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        } else {
            bail!("non-multisampled blit not supported");
        }
        self.queue.submit([encoder.finish()]);
        Ok(())
    }

    // -----------------------------------------------------------------------------------
    // Programs and uniforms

    fn link(&mut self, id: u32) -> Result<()> {
        let (vs, fs) = {
            let p = self.programs.get(&id).ok_or_else(|| anyhow!("no program {id}"))?;
            let mut vs = None;
            let mut fs = None;
            for s in &p.shaders {
                let sh = self.shaders.get(s).ok_or_else(|| anyhow!("no shader {s}"))?;
                match sh.stage {
                    VERTEX_SHADER => vs = Some(sh.source.clone()),
                    FRAGMENT_SHADER => fs = Some(sh.source.clone()),
                    _ => bail!("shader stage"),
                }
            }
            (vs.ok_or_else(|| anyhow!("no vertex shader"))?, fs.ok_or_else(|| anyhow!("no fragment shader"))?)
        };
        self.stats.programs += 1;
        // Debug aid: S7_FS_TAIL appends statements to the end of every fragment main.
        let fs = match std::env::var("S7_FS_TAIL") {
            Ok(tail) if fs.contains("SHADER_TYPE MeshStandardMaterial") => {
                let end = fs.rfind('}').unwrap();
                format!("{}{tail}\n}}\n", &fs[..end])
            }
            _ => fs,
        };
        let mut overridden = false;
        let compiled = match self.over.as_ref().and_then(|o| o(&vs, &fs)) {
            Some(r) => {
                overridden = true;
                r
            }
            None => s7_glsl::compile(&vs, &fs),
        };
        let p = self.programs.get_mut(&id).unwrap();
        match compiled {
            Ok(program) => {
                if overridden {
                    self.stats.programs_overridden += 1;
                }
                let module = |m: &naga::Module, label| self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Naga(Cow::Owned(m.clone())) });
                let vertex = module(&program.vertex, "three vs");
                let fragment = module(&program.fragment, "three fs");
                p.block = vec![0u8; program.block_size as usize];
                p.compiled = Some(std::rc::Rc::new(ProgramGpu { program, vertex, fragment, overridden }));
            }
            Err(e) => {
                let name = fs.lines().find(|l| l.starts_with("#define SHADER_TYPE") || l.starts_with("#define SHADER_NAME")).unwrap_or("").to_string();
                let msg = format!("{name}: {e:#}");
                self.stats.program_failures.push(msg.chars().take(600).collect());
                p.error = Some(msg);
            }
        }
        Ok(())
    }

    fn uniform(&mut self, c: &Call) -> Result<()> {
        let loc = c.o(0)?;
        if loc == 0 {
            return Ok(());
        }
        let (pid, name) = self.locations.get(&loc).cloned().ok_or_else(|| anyhow!("unknown uniform location {loc}"))?;
        let p = self.programs.get_mut(&pid).ok_or_else(|| anyhow!("no program"))?;
        let Some(gpu) = p.compiled.clone() else { return Ok(()) };
        let n = c.name;
        // Values as f64 plus element arity.
        let values: Vec<f64> = if n.starts_with("uniformMatrix") && c.args.get(2).is_some_and(|v| v.is_array()) {
            if c.b(1)? {
                bail!("transposed matrix uniform");
            }
            c.args[2].as_array().unwrap().iter().map(|v| crate::trace::number(v).ok_or_else(|| anyhow!("matrix element"))).collect::<Result<Vec<_>>>()?
        } else if n.starts_with("uniformMatrix") {
            if c.b(1)? {
                bail!("transposed matrix uniform");
            }
            let (bytes, t) = c.bytes(2)?;
            if t != "f32" {
                bail!("matrix uniform of {t}");
            }
            bytemuck::cast_slice::<u8, f32>(bytes).iter().map(|v| *v as f64).collect()
        } else if n.ends_with('v') && c.args.get(1).is_some_and(|v| v.is_array()) {
            c.args[1].as_array().unwrap().iter().map(|v| crate::trace::number(v).ok_or_else(|| anyhow!("uniform array element"))).collect::<Result<Vec<_>>>()?
        } else if n.ends_with('v') {
            let (bytes, t) = c.bytes(1)?;
            let v: Vec<f64> = match t {
                "f32" => bytemuck::cast_slice::<u8, f32>(bytes).iter().map(|v| *v as f64).collect(),
                "i32" => bytemuck::cast_slice::<u8, i32>(bytes).iter().map(|v| *v as f64).collect(),
                "u32" => bytemuck::cast_slice::<u8, u32>(bytes).iter().map(|v| *v as f64).collect(),
                t => bail!("uniform data {t}"),
            };
            v
        } else {
            (1..c.len()).map(|i| c.f(i)).collect::<Result<Vec<_>>>()?
        };
        // Sampler uniforms set texture units.
        let base = name.strip_suffix("[0]").unwrap_or(&name).to_string();
        if gpu.program.textures.contains_key(&name) || gpu.program.textures.contains_key(&format!("{base}[0]")) || gpu.program.textures.contains_key(&base) {
            if values.len() == 1 && !name.ends_with("[0]") {
                p.sampler_units.insert(name.clone(), values[0] as u32);
            } else {
                for (k, v) in values.iter().enumerate() {
                    p.sampler_units.insert(format!("{base}[{k}]"), *v as u32);
                }
                if values.len() == 1 {
                    p.sampler_units.insert(base.clone(), values[0] as u32);
                }
            }
            return Ok(());
        }
        let Some(slot) = gpu.program.uniforms.get(&name).or_else(|| gpu.program.uniforms.get(&base)) else {
            self.stats.eliminated_uniform_writes += 1;
            if std::env::var_os("S7_DEBUG").is_some() {
                let mut known: Vec<_> = gpu.program.uniforms.keys().cloned().collect();
                known.sort();
                eprintln!("eliminated uniform {name} (program {pid}); known: {}", known.join(","));
            }
            return Ok(());
        };
        let per = (slot.columns * slot.rows) as usize;
        let elements = values.len() / per.max(1);
        if elements == 0 || elements > slot.count as usize {
            bail!("uniform {name}: {} values for {} x {per}", values.len(), slot.count);
        }
        for e in 0..elements {
            for col in 0..slot.columns as usize {
                for row in 0..slot.rows as usize {
                    let v = values[e * per + col * slot.rows as usize + row];
                    let off = slot.offset as usize + e * slot.array_stride as usize + col * slot.column_stride as usize + row * 4;
                    let bytes = match slot.kind {
                        ScalarKind::Float => (v as f32).to_le_bytes(),
                        ScalarKind::Sint => (v as i32).to_le_bytes(),
                        ScalarKind::Uint => (v as u32).to_le_bytes(),
                        ScalarKind::Bool => ((v != 0.0) as u32).to_le_bytes(),
                    };
                    p.block[off..off + 4].copy_from_slice(&bytes);
                }
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------
    // Draws

    fn gpu_buffer(&mut self, id: u32) -> Result<wgpu::Buffer> {
        let b = self.buffers.get_mut(&id).ok_or_else(|| anyhow!("no buffer {id}"))?;
        if b.gpu.as_ref().map(|(v, _)| *v) != Some(b.version) {
            let mut data = b.data.clone();
            // naga's MSL vertex pulling (always on in wgpu-hal Metal) admits element i only
            // when i < (bound size / stride). For an attribute at offset o > 0 into an
            // interleaved buffer (instanceMatrix columns at 16, 32, 48 with stride 64), the
            // bound slice holds n·stride − o bytes, so the last element reads as zero. GL
            // only needs o + size bytes for it. Padding every buffer by the WebGL maximum
            // stride (255) restores GL's rule without changing any element that GL reads.
            data.resize(align(data.len().max(4), 4) + VERTEX_PULL_PAD, 0);
            let buffer = wgpu::util::DeviceExt::create_buffer_init(&self.device, &wgpu::util::BufferInitDescriptor { label: Some("gl buffer"), contents: &data, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::INDEX });
            b.gpu = Some((b.version, buffer));
        }
        Ok(b.gpu.as_ref().unwrap().1.clone())
    }

    /// Vertex buffer and format for one GL attribute. Float32 and 4-byte-aligned normalized
    /// formats pass through; everything else is converted to tightly packed f32 once.
    fn attribute_buffer(&mut self, a: &AttribState) -> Result<(wgpu::Buffer, u64, wgpu::VertexFormat, u64)> {
        use wgpu::VertexFormat as V;
        let elem = match a.ty {
            BYTE | UNSIGNED_BYTE => 1,
            SHORT | UNSIGNED_SHORT | HALF_FLOAT => 2,
            INT | UNSIGNED_INT | FLOAT => 4,
            t => bail!("attribute type {t:#x}"),
        };
        let stride = if a.stride == 0 { (elem * a.size) as u64 } else { a.stride as u64 };
        let direct = match (a.ty, a.size, a.normalized) {
            (FLOAT, 1, _) => Some(V::Float32),
            (FLOAT, 2, _) => Some(V::Float32x2),
            (FLOAT, 3, _) => Some(V::Float32x3),
            (FLOAT, 4, _) => Some(V::Float32x4),
            (UNSIGNED_BYTE, 4, true) => Some(V::Unorm8x4),
            (BYTE, 4, true) => Some(V::Snorm8x4),
            (UNSIGNED_SHORT, 2, true) => Some(V::Unorm16x2),
            (UNSIGNED_SHORT, 4, true) => Some(V::Unorm16x4),
            (SHORT, 2, true) => Some(V::Snorm16x2),
            (SHORT, 4, true) => Some(V::Snorm16x4),
            (HALF_FLOAT, 2, _) => Some(V::Float16x2),
            (HALF_FLOAT, 4, _) => Some(V::Float16x4),
            _ => None,
        };
        if let Some(format) = direct {
            if a.offset % 4 == 0 && stride % 4 == 0 {
                return Ok((self.gpu_buffer(a.buffer)?, a.offset, format, stride));
            }
        }
        let b = self.buffers.get(&a.buffer).ok_or_else(|| anyhow!("no buffer"))?;
        let key = format!("{}:{}:{:?}", a.buffer, b.version, (a.ty, a.size, a.normalized, stride, a.offset));
        if !self.converted.contains_key(&key) {
            self.stats.converted_attributes += 1;
            let data = &b.data;
            let size = (elem * a.size) as u64;
            let count = if (data.len() as u64) < a.offset + size { 0 } else { (data.len() as u64 - a.offset - size) / stride + 1 };
            let mut out: Vec<f32> = Vec::with_capacity((count * a.size as u64) as usize);
            for i in 0..count {
                let base = (a.offset + i * stride) as usize;
                for k in 0..a.size as usize {
                    let p = base + k * elem as usize;
                    let v: f32 = match a.ty {
                        UNSIGNED_BYTE => {
                            let x = data[p] as f32;
                            if a.normalized { x / 255.0 } else { x }
                        }
                        BYTE => {
                            let x = data[p] as i8 as f32;
                            if a.normalized { (x / 127.0).max(-1.0) } else { x }
                        }
                        UNSIGNED_SHORT => {
                            let x = u16::from_le_bytes([data[p], data[p + 1]]) as f32;
                            if a.normalized { x / 65535.0 } else { x }
                        }
                        SHORT => {
                            let x = i16::from_le_bytes([data[p], data[p + 1]]) as f32;
                            if a.normalized { (x / 32767.0).max(-1.0) } else { x }
                        }
                        HALF_FLOAT => half::f16::from_le_bytes([data[p], data[p + 1]]).to_f32(),
                        FLOAT => f32::from_le_bytes(data[p..p + 4].try_into().unwrap()),
                        UNSIGNED_INT => {
                            let x = u32::from_le_bytes(data[p..p + 4].try_into().unwrap()) as f64;
                            (if a.normalized { x / 4294967295.0 } else { x }) as f32
                        }
                        INT => {
                            let x = i32::from_le_bytes(data[p..p + 4].try_into().unwrap()) as f64;
                            (if a.normalized { (x / 2147483647.0).max(-1.0) } else { x }) as f32
                        }
                        _ => unreachable!(),
                    };
                    out.push(v);
                }
            }
            if out.is_empty() {
                out.push(0.0);
            }
            let buffer = wgpu::util::DeviceExt::create_buffer_init(&self.device, &wgpu::util::BufferInitDescriptor { label: Some("converted attribute"), contents: bytemuck::cast_slice(&out), usage: wgpu::BufferUsages::VERTEX });
            self.converted.insert(key.clone(), buffer);
        }
        let format = [V::Float32, V::Float32x2, V::Float32x3, V::Float32x4][a.size as usize - 1];
        Ok((self.converted[&key].clone(), 0, format, a.size as u64 * 4))
    }

    fn index_buffer(&mut self, mode: u32, first: u32, count: u32, elements: Option<(u32, u64)>) -> Result<Option<(wgpu::Buffer, wgpu::IndexFormat, u32)>> {
        let Some((ty, offset)) = elements else {
            if mode == LINE_LOOP {
                let mut idx: Vec<u32> = (first..first + count).collect();
                idx.push(first);
                let buffer = wgpu::util::DeviceExt::create_buffer_init(&self.device, &wgpu::util::BufferInitDescriptor { label: Some("line loop"), contents: bytemuck::cast_slice(&idx), usage: wgpu::BufferUsages::INDEX });
                return Ok(Some((buffer, wgpu::IndexFormat::Uint32, 0)));
            }
            return Ok(None);
        };
        let id = self.vao_ref().elements;
        if id == 0 {
            bail!("drawElements without an element buffer");
        }
        let size = match ty {
            UNSIGNED_BYTE => 1,
            UNSIGNED_SHORT => 2,
            UNSIGNED_INT => 4,
            t => bail!("index type {t:#x}"),
        };
        if offset % size != 0 {
            bail!("misaligned index offset");
        }
        let first_index = (offset / size) as u32;
        if ty == UNSIGNED_BYTE || mode == LINE_LOOP {
            let b = self.buffers.get(&id).ok_or_else(|| anyhow!("no buffer"))?;
            let key = format!("{id}:{}:{ty}:{mode}:{first_index}:{count}", b.version);
            if !self.index_cache.contains_key(&key) {
                let read = |i: usize| -> u32 {
                    match ty {
                        UNSIGNED_BYTE => b.data[i] as u32,
                        UNSIGNED_SHORT => u16::from_le_bytes([b.data[2 * i], b.data[2 * i + 1]]) as u32,
                        _ => u32::from_le_bytes(b.data[4 * i..4 * i + 4].try_into().unwrap()),
                    }
                };
                let mut idx: Vec<u32> = (0..count as usize).map(|k| read(first_index as usize + k)).collect();
                if mode == LINE_LOOP && !idx.is_empty() {
                    idx.push(idx[0]);
                }
                let buffer = wgpu::util::DeviceExt::create_buffer_init(&self.device, &wgpu::util::BufferInitDescriptor { label: Some("converted index"), contents: bytemuck::cast_slice(&idx), usage: wgpu::BufferUsages::INDEX });
                self.index_cache.insert(key.clone(), buffer);
            }
            return Ok(Some((self.index_cache[&key].clone(), wgpu::IndexFormat::Uint32, 0)));
        }
        let format = if ty == UNSIGNED_SHORT { wgpu::IndexFormat::Uint16 } else { wgpu::IndexFormat::Uint32 };
        Ok(Some((self.gpu_buffer(id)?, format, first_index)))
    }

    fn dummy_view(&mut self, dim: wgpu::TextureViewDimension, sample: wgpu::TextureSampleType) -> wgpu::TextureView {
        let key = format!("{dim:?}{sample:?}");
        if let Some(v) = self.dummies.get(&key) {
            return v.clone();
        }
        let (format, bytes): (wgpu::TextureFormat, Vec<u8>) = match sample {
            wgpu::TextureSampleType::Depth => (wgpu::TextureFormat::Depth32Float, Vec::new()),
            wgpu::TextureSampleType::Uint => (wgpu::TextureFormat::Rgba8Uint, vec![0, 0, 0, 1]),
            wgpu::TextureSampleType::Sint => (wgpu::TextureFormat::Rgba8Sint, vec![0, 0, 0, 1]),
            _ => (wgpu::TextureFormat::Rgba8Unorm, vec![0, 0, 0, 255]),
        };
        let layers = if dim == wgpu::TextureViewDimension::Cube { 6 } else { 1 };
        let dimension = if dim == wgpu::TextureViewDimension::D3 { wgpu::TextureDimension::D3 } else { wgpu::TextureDimension::D2 };
        let mut usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        if format.is_depth_stencil_format() {
            usage |= wgpu::TextureUsages::RENDER_ATTACHMENT;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor { label: Some("dummy"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: layers }, mip_level_count: 1, sample_count: 1, dimension, format, usage, view_formats: &[] });
        if !bytes.is_empty() {
            for z in 0..layers {
                self.queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d { x: 0, y: 0, z }, aspect: wgpu::TextureAspect::All }, &bytes, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) }, wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 });
            }
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(dim), ..Default::default() });
        self.dummies.insert(key, view.clone());
        view
    }

    fn sampler(&mut self, p: SamplerParams, filterable: bool, levels_used: bool, levels: u32) -> wgpu::Sampler {
        let key = (p, filterable, levels_used);
        if let Some(s) = self.samplers.get(&key) {
            return s.clone();
        }
        let wrap = |w: u32| match w {
            CLAMP_TO_EDGE => wgpu::AddressMode::ClampToEdge,
            MIRRORED_REPEAT => wgpu::AddressMode::MirrorRepeat,
            _ => wgpu::AddressMode::Repeat,
        };
        let lin = |f: u32| filterable && matches!(f, LINEAR | LINEAR_MIPMAP_NEAREST | LINEAR_MIPMAP_LINEAR);
        let filter = |l: bool| if l { wgpu::FilterMode::Linear } else { wgpu::FilterMode::Nearest };
        let mip = filterable && matches!(p.min, NEAREST_MIPMAP_LINEAR | LINEAR_MIPMAP_LINEAR);
        let compare = p.compare.map(|f| match f {
            NEVER => wgpu::CompareFunction::Never,
            LESS => wgpu::CompareFunction::Less,
            EQUAL => wgpu::CompareFunction::Equal,
            LEQUAL => wgpu::CompareFunction::LessEqual,
            GREATER => wgpu::CompareFunction::Greater,
            NOTEQUAL => wgpu::CompareFunction::NotEqual,
            GEQUAL => wgpu::CompareFunction::GreaterEqual,
            _ => wgpu::CompareFunction::Always,
        });
        let s = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: None,
            address_mode_u: wrap(p.wrap_s),
            address_mode_v: wrap(p.wrap_t),
            address_mode_w: wrap(p.wrap_r),
            mag_filter: filter(lin(p.mag)),
            min_filter: filter(lin(p.min)),
            mipmap_filter: if mip { wgpu::MipmapFilterMode::Linear } else { wgpu::MipmapFilterMode::Nearest },
            lod_min_clamp: 0.0,
            lod_max_clamp: if levels_used { levels as f32 } else { 0.0 },
            compare,
            anisotropy_clamp: 1,
            border_color: None,
        });
        self.samplers.insert(key, s.clone());
        s
    }

    fn draw(&mut self, mode: u32, first: u32, count: u32, elements: Option<(u32, u64)>, instances: u32) -> Result<()> {
        if count == 0 || instances == 0 {
            return Ok(());
        }
        let pid = self.program;
        let Some(gpu) = self.programs.get(&pid).and_then(|p| p.compiled.clone()) else {
            self.stats.skipped_draws += 1;
            return Ok(());
        };
        let state = self.state;
        if state.cull && state.cull_face == FRONT_AND_BACK && mode >= TRIANGLES {
            return Ok(());
        }
        self.stats.draws += 1;
        let topology = match mode {
            POINTS => wgpu::PrimitiveTopology::PointList,
            LINES => wgpu::PrimitiveTopology::LineList,
            LINE_STRIP | LINE_LOOP => wgpu::PrimitiveTopology::LineStrip,
            TRIANGLES => wgpu::PrimitiveTopology::TriangleList,
            TRIANGLE_STRIP => wgpu::PrimitiveTopology::TriangleStrip,
            m => bail!("draw mode {m}"),
        };
        // Vertex inputs.
        let program = &gpu.program;
        let gl_locations = self.programs[&pid].attrib_locations.clone();
        let vao = self.vao_ref().clone();
        let mut inputs: Vec<(u32, String, u32, naga::TypeInner)> = program.attributes.values().map(|a| (a.location, a.gl_name.clone(), a.column, a.ty.clone())).collect();
        inputs.sort_by_key(|i| i.0);
        let mut vertex_buffers = Vec::new();
        let mut layout_desc: Vec<(u64, wgpu::VertexStepMode, wgpu::VertexFormat, u32)> = Vec::new();
        for (shader_loc, gl_name, column, ty) in &inputs {
            if let naga::TypeInner::Vector { scalar, .. } | naga::TypeInner::Scalar(scalar) = ty {
                if scalar.kind != naga::ScalarKind::Float {
                    bail!("non-float vertex input {gl_name}");
                }
            }
            let gl_loc = gl_locations.get(gl_name).or_else(|| self.programs[&pid].attrib_bindings.get(gl_name)).copied();
            let state = gl_loc.and_then(|l| vao.attribs.get((l + column) as usize).copied()).filter(|a| a.enabled);
            match state {
                Some(a) => {
                    let (buffer, offset, format, stride) = self.attribute_buffer(&a)?;
                    let step = match a.divisor {
                        0 => wgpu::VertexStepMode::Vertex,
                        1 => wgpu::VertexStepMode::Instance,
                        d => bail!("divisor {d}"),
                    };
                    vertex_buffers.push((buffer, offset));
                    layout_desc.push((stride, step, format, *shader_loc));
                }
                None => {
                    vertex_buffers.push((self.constant_attrib.clone(), 0));
                    layout_desc.push((0, wgpu::VertexStepMode::Instance, wgpu::VertexFormat::Float32x4, *shader_loc));
                }
            }
        }
        let index = self.index_buffer(mode, first, count, elements)?;
        let (count, first) = match (&index, mode) {
            (Some((_, _, fi)), LINE_LOOP) => (count + 1, *fi),
            (Some((_, _, fi)), _) => (count, *fi),
            (None, _) => (count, first),
        };
        // Textures.
        let mut slots: Vec<(String, s7_glsl::TextureSlot)> = program.textures.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        slots.sort_by_key(|s| s.1.binding);
        let mut textures = Vec::new();
        let mut samplers = Vec::new();
        let mut layout_key = format!("p{pid}|u{}", program.block_size);
        let mut entries = Vec::new();
        if program.block_size > 0 {
            entries.push(wgpu::BindGroupLayoutEntry { binding: s7_glsl::DEFAULT_BLOCK_BINDING, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(program.block_size as u64) }, count: None });
        }
        for (name, slot) in &slots {
            let unit = self.programs[&pid].sampler_units.get(name).copied().unwrap_or(0) as usize;
            let (dim, target_slot) = match (slot.dim, slot.arrayed) {
                (naga::ImageDimension::D2, false) => (wgpu::TextureViewDimension::D2, 0),
                (naga::ImageDimension::D2, true) => (wgpu::TextureViewDimension::D2Array, 3),
                (naga::ImageDimension::Cube, false) => (wgpu::TextureViewDimension::Cube, 1),
                (naga::ImageDimension::D3, false) => (wgpu::TextureViewDimension::D3, 2),
                other => bail!("texture {name}: {other:?}"),
            };
            let shadow = matches!(slot.class, naga::ImageClass::Depth { .. });
            let shader_kind = match slot.class {
                naga::ImageClass::Sampled { kind, .. } => Some(kind),
                _ => None,
            };
            let tex_id = self.units.get(unit).map(|u| u[target_slot]).unwrap_or(0);
            let bound = self.textures.get(&tex_id).filter(|t| t.texture.is_some());
            let (view, sample_type, params, levels) = match bound {
                Some(t) => {
                    let format = t.format;
                    let st = if depth_format(format) {
                        if shadow { wgpu::TextureSampleType::Depth } else { wgpu::TextureSampleType::Float { filterable: false } }
                    } else {
                        format.sample_type(None, Some(self.device.features())).ok_or_else(|| anyhow!("sample type of {format:?}"))?
                    };
                    let view = t.texture.as_ref().unwrap().create_view(&wgpu::TextureViewDescriptor { dimension: Some(dim), aspect: if depth_format(format) { wgpu::TextureAspect::DepthOnly } else { wgpu::TextureAspect::All }, ..Default::default() });
                    (view, st, t.params, t.levels)
                }
                None => {
                    let st = if shadow {
                        wgpu::TextureSampleType::Depth
                    } else {
                        match shader_kind {
                            Some(naga::ScalarKind::Uint) => wgpu::TextureSampleType::Uint,
                            Some(naga::ScalarKind::Sint) => wgpu::TextureSampleType::Sint,
                            _ => wgpu::TextureSampleType::Float { filterable: true },
                        }
                    };
                    (self.dummy_view(dim, st), st, SamplerParams { min: NEAREST, mag: NEAREST, ..Default::default() }, 1)
                }
            };
            let filterable = matches!(sample_type, wgpu::TextureSampleType::Float { filterable: true });
            let mipmapped = !matches!(params.min, NEAREST | LINEAR);
            let comparison = shadow;
            let mut params = params;
            if comparison && params.compare.is_none() {
                bail!("shadow sampler {name} bound to a texture without compare mode");
            }
            if !comparison {
                params.compare = None;
            }
            let sampler = self.sampler(params, filterable || comparison, mipmapped, levels);
            let sampler_ty = if comparison {
                wgpu::SamplerBindingType::Comparison
            } else if filterable {
                wgpu::SamplerBindingType::Filtering
            } else {
                wgpu::SamplerBindingType::NonFiltering
            };
            layout_key.push_str(&format!("|t{}:{dim:?}:{sample_type:?}:{sampler_ty:?}", slot.binding));
            entries.push(wgpu::BindGroupLayoutEntry { binding: slot.binding, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Texture { sample_type, view_dimension: dim, multisampled: false }, count: None });
            entries.push(wgpu::BindGroupLayoutEntry { binding: slot.sampler_binding, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Sampler(sampler_ty), count: None });
            textures.push((slot.binding, view));
            samplers.push((slot.sampler_binding, sampler));
        }
        if !self.layouts.contains_key(&layout_key) {
            let bgl = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("three program"), entries: &entries });
            let pl = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("three program"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
            self.layouts.insert(layout_key.clone(), (bgl, pl));
        }
        let (bgl, pipeline_layout) = self.layouts[&layout_key].clone();
        // Pipeline.
        let target = self.open_pass()?.target.clone();
        let strip_index_format = match (&index, topology) {
            (Some((_, f, _)), wgpu::PrimitiveTopology::LineStrip | wgpu::PrimitiveTopology::TriangleStrip) => Some(*f),
            _ => None,
        };
        let triangles = mode >= TRIANGLES;
        let cull_mode = if state.cull && triangles {
            match state.cull_face {
                FRONT => Some(wgpu::Face::Front),
                BACK => Some(wgpu::Face::Back),
                f => bail!("cullFace {f:#x}"),
            }
        } else {
            None
        };
        // Clip y is negated, so GL's counter-clockwise front faces are clockwise here.
        let front_face = if state.front_face == CCW { wgpu::FrontFace::Cw } else { wgpu::FrontFace::Ccw };
        let compare = |f: u32| -> Result<wgpu::CompareFunction> {
            Ok(match f {
                NEVER => wgpu::CompareFunction::Never,
                LESS => wgpu::CompareFunction::Less,
                EQUAL => wgpu::CompareFunction::Equal,
                LEQUAL => wgpu::CompareFunction::LessEqual,
                GREATER => wgpu::CompareFunction::Greater,
                NOTEQUAL => wgpu::CompareFunction::NotEqual,
                GEQUAL => wgpu::CompareFunction::GreaterEqual,
                ALWAYS => wgpu::CompareFunction::Always,
                f => bail!("compare {f:#x}"),
            })
        };
        let depth_stencil = match &target.depth {
            Some(d) => Some(wgpu::DepthStencilState {
                format: d.format,
                depth_write_enabled: Some(state.depth_test && state.depth_mask),
                depth_compare: Some(if state.depth_test { compare(state.depth_func)? } else { wgpu::CompareFunction::Always }),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            None => None,
        };
        let factor = |f: u32| -> Result<wgpu::BlendFactor> {
            use wgpu::BlendFactor as B;
            Ok(match f {
                ZERO => B::Zero,
                ONE => B::One,
                SRC_COLOR => B::Src,
                ONE_MINUS_SRC_COLOR => B::OneMinusSrc,
                SRC_ALPHA => B::SrcAlpha,
                ONE_MINUS_SRC_ALPHA => B::OneMinusSrcAlpha,
                DST_ALPHA => B::DstAlpha,
                ONE_MINUS_DST_ALPHA => B::OneMinusDstAlpha,
                DST_COLOR => B::Dst,
                ONE_MINUS_DST_COLOR => B::OneMinusDst,
                SRC_ALPHA_SATURATE => B::SrcAlphaSaturated,
                f => bail!("blend factor {f:#x}"),
            })
        };
        let op = |e: u32| -> Result<wgpu::BlendOperation> {
            Ok(match e {
                FUNC_ADD => wgpu::BlendOperation::Add,
                FUNC_SUBTRACT => wgpu::BlendOperation::Subtract,
                FUNC_REVERSE_SUBTRACT => wgpu::BlendOperation::ReverseSubtract,
                MIN => wgpu::BlendOperation::Min,
                MAX => wgpu::BlendOperation::Max,
                e => bail!("blend equation {e:#x}"),
            })
        };
        let b = state.blend;
        let blend = if b.enabled {
            let comp = |src, dst, eq| -> Result<wgpu::BlendComponent> {
                let operation = op(eq)?;
                // GL ignores factors for MIN and MAX; WebGPU requires One.
                if matches!(operation, wgpu::BlendOperation::Min | wgpu::BlendOperation::Max) {
                    return Ok(wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation });
                }
                Ok(wgpu::BlendComponent { src_factor: factor(src)?, dst_factor: factor(dst)?, operation })
            };
            Some(wgpu::BlendState { color: comp(b.src_rgb, b.dst_rgb, b.eq_rgb)?, alpha: comp(b.src_alpha, b.dst_alpha, b.eq_alpha)? })
        } else {
            None
        };
        let mut write = wgpu::ColorWrites::empty();
        for (i, w) in [wgpu::ColorWrites::RED, wgpu::ColorWrites::GREEN, wgpu::ColorWrites::BLUE, wgpu::ColorWrites::ALPHA].iter().enumerate() {
            if state.color_mask[i] {
                write |= *w;
            }
        }
        let targets: Vec<Option<wgpu::ColorTargetState>> = target
            .colors
            .iter()
            .enumerate()
            .map(|(i, c)| {
                c.as_ref().and_then(|c| {
                    program.outputs.contains(&(i as u32)).then(|| wgpu::ColorTargetState { format: c.format, blend, write_mask: write })
                })
            })
            .collect();
        let key = format!("{layout_key}|{topology:?}|{strip_index_format:?}|{cull_mode:?}|{front_face:?}|{depth_stencil:?}|{}|{targets:?}|{layout_desc:?}", target.samples);
        if !self.pipelines.contains_key(&key) {
            let buffers: Vec<Vec<wgpu::VertexAttribute>> = layout_desc.iter().map(|(_, _, format, loc)| vec![wgpu::VertexAttribute { format: *format, offset: 0, shader_location: *loc }]).collect();
            let layouts: Vec<Option<wgpu::VertexBufferLayout>> = layout_desc.iter().zip(buffers.iter()).map(|((stride, step, _, _), attrs)| Some(wgpu::VertexBufferLayout { array_stride: *stride, step_mode: *step, attributes: attrs })).collect();
            let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("three"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState { module: &gpu.vertex, entry_point: None, compilation_options: Default::default(), buffers: &layouts },
                primitive: wgpu::PrimitiveState { topology, strip_index_format, front_face, cull_mode, unclipped_depth: false, polygon_mode: wgpu::PolygonMode::Fill, conservative: false },
                depth_stencil,
                multisample: wgpu::MultisampleState { count: target.samples, mask: !0, alpha_to_coverage_enabled: false },
                fragment: Some(wgpu::FragmentState { module: &gpu.fragment, entry_point: None, compilation_options: Default::default(), targets: &targets }),
                multiview_mask: None,
                cache: None,
            });
            self.stats.pipelines += 1;
            self.pipelines.insert(key.clone(), pipeline);
        }
        let pipeline = self.pipelines[&key].clone();
        let [vx, vy, vw, vh] = state.viewport;
        let scissor = self.scissor_rect(&target);
        if scissor[2] == 0 || scissor[3] == 0 {
            return Ok(());
        }
        let block = self.programs[&pid].block.clone();
        if std::env::var_os("S7_DEBUG").is_some() && self.stats.draws < 8 {
            let mut names: Vec<_> = program.uniforms.iter().filter(|(k, _)| !k.ends_with("]")).collect();
            names.sort_by_key(|(_, s)| s.offset);
            for (k, slot) in names.iter().take(60) {
                let o = slot.offset as usize;
                let n = (slot.rows * slot.columns).min(4) as usize;
                let vals: Vec<f32> = (0..n).map(|i| f32::from_le_bytes(block[o + 4 * i..o + 4 * i + 4].try_into().unwrap())).collect();
                eprintln!("  {k} @{o} {:?} {vals:?}", slot.kind);
            }
            eprintln!("  textures: {:?}", program.textures.keys().collect::<Vec<_>>());
            eprintln!("  units: {:?}", self.programs[&pid].sampler_units);
        }
        let pass = self.pass.as_mut().unwrap();
        let uniform = if program.block_size > 0 {
            let offset = align(pass.uniforms.len(), 256);
            pass.uniforms.resize(offset, 0);
            pass.uniforms.extend_from_slice(&block);
            Some((offset as u32, program.block_size))
        } else {
            None
        };
        pass.items.push(PassItem::Draw(DrawCmd {
            pipeline,
            layout: bgl,
            uniform,
            textures,
            samplers,
            vertex_buffers,
            index: index.map(|(b, f, _)| (b, f)),
            first,
            count,
            instances,
            viewport: [vx as f32, vy as f32, vw as f32, vh as f32],
            scissor,
        }));
        Ok(())
    }

    /// Ends and submits the open pass.
    pub fn flush(&mut self) -> Result<()> {
        let Some(pass) = self.pass.take() else { return Ok(()) };
        self.stats.passes += 1;
        let mut uniforms = pass.uniforms;
        uniforms.resize(align(uniforms.len().max(256), 256) + 256, 0);
        let ubuf = wgpu::util::DeviceExt::create_buffer_init(&self.device, &wgpu::util::BufferInitDescriptor { label: Some("pass uniforms"), contents: &uniforms, usage: wgpu::BufferUsages::UNIFORM });
        let mut groups = Vec::with_capacity(pass.items.len());
        for item in &pass.items {
            match item {
                PassItem::Draw(d) => {
                    let mut entries = Vec::new();
                    if let Some((_, size)) = d.uniform {
                        entries.push(wgpu::BindGroupEntry { binding: s7_glsl::DEFAULT_BLOCK_BINDING, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &ubuf, offset: 0, size: wgpu::BufferSize::new(size as u64) }) });
                    }
                    for (b, v) in &d.textures {
                        entries.push(wgpu::BindGroupEntry { binding: *b, resource: wgpu::BindingResource::TextureView(v) });
                    }
                    for (b, s) in &d.samplers {
                        entries.push(wgpu::BindGroupEntry { binding: *b, resource: wgpu::BindingResource::Sampler(s) });
                    }
                    groups.push(self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &d.layout, entries: &entries }));
                }
                PassItem::Quad(_) => {
                    let layout = &self.util.as_ref().unwrap().quad_layout;
                    groups.push(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: None,
                        layout,
                        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &ubuf, offset: 0, size: wgpu::BufferSize::new(32) }) }],
                    }));
                }
            }
        }
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let colors: Vec<Option<wgpu::RenderPassColorAttachment>> = pass
                .target
                .colors
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    c.as_ref().map(|c| wgpu::RenderPassColorAttachment {
                        view: &c.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: match pass.color_clear[i] { Some(col) => wgpu::LoadOp::Clear(col), None => wgpu::LoadOp::Load }, store: wgpu::StoreOp::Store },
                    })
                })
                .collect();
            let depth = pass.target.depth.as_ref().map(|d| wgpu::RenderPassDepthStencilAttachment {
                view: &d.view,
                depth_ops: Some(wgpu::Operations { load: match pass.depth_clear { Some(v) => wgpu::LoadOp::Clear(v), None => wgpu::LoadOp::Load }, store: wgpu::StoreOp::Store }),
                stencil_ops: if d.format.has_stencil_aspect() { Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }) } else { None },
            });
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("gl pass"), color_attachments: &colors, depth_stencil_attachment: depth, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
            for (item, group) in pass.items.iter().zip(groups.iter()) {
                match item {
                    PassItem::Draw(d) => {
                        rp.set_pipeline(&d.pipeline);
                        let offsets: Vec<u32> = d.uniform.map(|(o, _)| vec![o]).unwrap_or_default();
                        rp.set_bind_group(0, group, &offsets);
                        for (i, (b, off)) in d.vertex_buffers.iter().enumerate() {
                            rp.set_vertex_buffer(i as u32, b.slice(*off..));
                        }
                        let [x, y, w, h] = d.viewport;
                        rp.set_viewport(x, y, w, h, 0.0, 1.0);
                        rp.set_scissor_rect(d.scissor[0], d.scissor[1], d.scissor[2], d.scissor[3]);
                        match &d.index {
                            Some((b, f)) => {
                                rp.set_index_buffer(b.slice(..), *f);
                                rp.draw_indexed(d.first..d.first + d.count, 0, 0..d.instances);
                            }
                            None => rp.draw(d.first..d.first + d.count, 0..d.instances),
                        }
                    }
                    PassItem::Quad(q) => {
                        rp.set_pipeline(&q.pipeline);
                        rp.set_bind_group(0, group, &[q.uniform]);
                        rp.set_viewport(0.0, 0.0, pass.target.width as f32, pass.target.height as f32, 0.0, 1.0);
                        rp.set_scissor_rect(q.scissor[0], q.scissor[1], q.scissor[2], q.scissor[3]);
                        rp.draw(0..3, 0..1);
                    }
                }
            }
        }
        self.queue.submit([encoder.finish()]);
        Ok(())
    }

    /// Resolves and reads the default framebuffer (as `readPixels` after a render).
    pub fn capture(&mut self, name: &str) -> Result<Frame> {
        self.flush()?;
        let fb = self.default_fb.as_ref().ok_or_else(|| anyhow!("no default framebuffer"))?;
        let (w, h) = (fb.width, fb.height);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let source = if let Some(resolve) = &fb.resolve {
            let cv = fb.color.create_view(&Default::default());
            let rv = resolve.create_view(&Default::default());
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("resolve"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &cv, depth_slice: None, resolve_target: Some(&rv), ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            resolve
        } else {
            &fb.color
        };
        let rgba = read_texture(&self.device, &self.queue, encoder, source, w, h)?;
        // wgpu row 0 is GL row 0 (the bottom); images are top-down.
        let row = (w * 4) as usize;
        let mut out = vec![0u8; rgba.len()];
        for y in 0..h as usize {
            out[y * row..(y + 1) * row].copy_from_slice(&rgba[(h as usize - 1 - y) * row..(h as usize - y) * row]);
        }
        Ok(Frame { name: name.to_string(), width: w, height: h, rgba: out })
    }
}

/// Copies an RGBA8 texture into memory (rows in texture order).
pub fn read_texture(device: &wgpu::Device, queue: &wgpu::Queue, mut encoder: wgpu::CommandEncoder, texture: &wgpu::Texture, w: u32, h: u32) -> Result<Vec<u8>> {
    let padded = align((w * 4) as usize, 256) as u32;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some("readback"), size: (padded * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("poll: {e:?}"))?;
    let data = slice.get_mapped_range().map_err(|e| anyhow!("map: {e:?}"))?;
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        out.extend_from_slice(&data[y * padded as usize..y * padded as usize + (w * 4) as usize]);
    }
    drop(data);
    buffer.unmap();
    Ok(out)
}

