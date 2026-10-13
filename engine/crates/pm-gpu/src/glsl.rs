//! The GLSL ES pipeline for three.js programs (plan section 11, "The
//! pipeline"; amendments 9 and 11): WebGL2 GLSL ES 3.00 to naga IR for wgpu.
//!
//! 1. Normalize `#version 300 es` to `#version 450`, drop default-precision
//!    statements.
//! 2. Preprocess with glslang, then expand sampler arrays (three.js unrolls
//!    its loops, so indices are literals) into one sampler per element; split
//!    matrix attributes, varying arrays, `modf`, Vulkan keywords, std140
//!    scalar arrays; program-wide sampler bindings and varying locations
//!    (varyings link by name, as GL does).
//! 3. Wrap the vertex `main` with the clip-space conventions of the target
//!    ([`VERTEX_EPILOGUE`]: GL row order, depth to `[0, w]`).
//! 4. Compile both stages as one linked glslang program with relaxed Vulkan
//!    rules (loose uniforms go to `gl_DefaultUniformBlock`) and automatic
//!    bindings and locations.
//! 5. Split combined image-samplers into separate image and sampler variables
//!    at the SPIR-V level.
//! 6. Parse with naga's SPIR-V front end, pad std140 struct spans, write an
//!    all-ones sample mask from every fragment shader (amendment 11), and
//!    validate with derivative uniformity off.
//!
//! Ported from the S7 spike's `s7-glsl` (docs/engine/spikes/S7.md, "Pipeline
//! findings"), which compiled all 49 three.js r180 programs of the 3D corpus
//! and replayed them within the 3D bar. Functions taking sampler parameters
//! need SPIRV-Tools' opaque inliner, which this module does not link yet:
//! such programs (shadow maps, transmission, CubeUV environment maps) are
//! refused with an error naming the function, never compiled wrong.

pub use naga;

use std::collections::{BTreeMap, HashMap};
use std::ffi::{CStr, CString};

/// A pipeline failure (glslang, a source rewrite, naga).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct GlslError(pub String);

pub type Result<T, E = GlslError> = std::result::Result<T, E>;

macro_rules! anyhow {
    ($($t:tt)*) => { GlslError(format!($($t)*)) };
}

macro_rules! bail {
    ($($t:tt)*) => { return Err(GlslError(format!($($t)*))) };
}

trait Context<T> {
    fn context(self, what: &str) -> Result<T>;
}

impl<T, E: std::fmt::Display> Context<T> for std::result::Result<T, E> {
    fn context(self, what: &str) -> Result<T> {
        self.map_err(|e| GlslError(format!("{what}: {e}")))
    }
}

/// Vertex epilogue. Two conventions are applied after the user's `main`:
///
/// * GL clip-space depth is `[-w, w]`; Metal and wgpu use `[0, w]`, so z' = (z + w) / 2
///   (exactly what ANGLE's Metal backend emits).
/// * Y is negated. The replay keeps every texture in GL row order (row 0 = GL bottom);
///   negating clip y makes GL's bottom row land in wgpu's row 0, so render targets and
///   sampled textures stay in the same orientation as in GL, viewport and scissor
///   rectangles pass through unchanged, `gl_FragCoord` keeps GL's values, and front-face
///   winding flips (handled by the pipeline's `front_face`).
pub const VERTEX_EPILOGUE: &str = "\nvoid main() {\n  pm_main();\n  gl_Position.y = -gl_Position.y;\n  gl_Position.z = (gl_Position.z + gl_Position.w) * 0.5;\n}\n";

/// Default uniform block binding (shared by both stages after linking).
pub const DEFAULT_BLOCK_BINDING: u32 = 0;
/// Sampler bindings are their texture's binding plus this offset.
pub const SAMPLER_BINDING_OFFSET: u32 = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    Vertex,
    Fragment,
}

/// A uniform's location in the default block.
#[derive(Clone, Debug)]
pub struct UniformSlot {
    pub offset: u32,
    pub kind: ScalarKind,
    /// Components per element (1-4 for vectors; columns × rows for matrices).
    pub columns: u32,
    pub rows: u32,
    /// Bytes between matrix columns (std140: 16).
    pub column_stride: u32,
    /// Number of array elements starting at this slot, and their stride.
    pub count: u32,
    pub array_stride: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarKind {
    Float,
    Sint,
    Uint,
    /// GLSL `bool` in a block is stored as a 32-bit integer.
    Bool,
}

/// A sampled texture after splitting: one image binding plus one sampler binding.
#[derive(Clone, Debug)]
pub struct TextureSlot {
    pub binding: u32,
    pub sampler_binding: u32,
    pub dim: naga::ImageDimension,
    pub arrayed: bool,
    pub class: naga::ImageClass,
    pub stages: Vec<Stage>,
}

#[derive(Clone, Debug)]
pub struct Attribute {
    pub location: u32,
    pub ty: naga::TypeInner,
    /// GL attribute name and matrix column (matrix inputs are split into columns,
    /// which GL places at consecutive locations).
    pub gl_name: String,
    pub column: u32,
}

pub struct Program {
    pub vertex: naga::Module,
    pub fragment: naga::Module,
    pub vertex_info: naga::valid::ModuleInfo,
    pub fragment_info: naga::valid::ModuleInfo,
    /// Size of the default uniform block in bytes (0 when absent).
    pub block_size: u32,
    pub block_stages: Vec<Stage>,
    /// GL uniform name (`a.b[2].c`, also `a[0]` and `a` for arrays) to block slot.
    pub uniforms: HashMap<String, UniformSlot>,
    /// GL sampler uniform name (`map`, `directionalShadowMap[0]`) to texture slot.
    pub textures: HashMap<String, TextureSlot>,
    pub attributes: HashMap<String, Attribute>,
    /// Fragment outputs by location.
    pub outputs: Vec<u32>,
    pub preprocessed: [String; 2],
}

fn stage_sys(stage: Stage) -> glslang_sys::glslang_stage_t {
    match stage {
        Stage::Vertex => glslang_sys::glslang_stage_t::Vertex,
        Stage::Fragment => glslang_sys::glslang_stage_t::Fragment,
    }
}

/// Step 1: textual normalization of WebGL2 GLSL ES 3.00.
pub fn normalize(source: &str) -> String {
    let mut out = String::with_capacity(source.len() + 64);
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("#version") {
            out.push_str("#version 450\n");
            continue;
        }
        if trimmed.starts_with("precision ") && trimmed.ends_with(';') {
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

struct Glslang {
    _compiler: &'static glslang::Compiler,
}

impl Glslang {
    fn get() -> Result<Self> {
        Ok(Self {
            _compiler: glslang::Compiler::acquire()
                .ok_or_else(|| anyhow!("glslang initialization failed"))?,
        })
    }

    fn input(stage: Stage, code: &CStr) -> glslang_sys::glslang_input_t {
        let limits: &glslang::limits::ResourceLimits = &glslang::limits::DEFAULT_LIMITS;
        // SAFETY: ResourceLimits is #[repr(transparent)] over glslang_resource_t.
        let resource = limits as *const glslang::limits::ResourceLimits
            as *const glslang_sys::glslang_resource_t;
        glslang_sys::glslang_input_t {
            language: glslang_sys::glslang_source_t::GLSL,
            stage: stage_sys(stage),
            client: glslang_sys::glslang_client_t::Vulkan,
            client_version: glslang_sys::glslang_target_client_version_t::Vulkan1_1,
            target_language: glslang_sys::glslang_target_language_t::SPIRV,
            target_language_version: glslang_sys::glslang_target_language_version_t::SPIRV1_3,
            code: code.as_ptr(),
            default_version: 450,
            default_profile: glslang_sys::glslang_profile_t::None,
            force_default_version_and_profile: 0,
            forward_compatible: 0,
            messages: glslang_sys::glslang_messages_t::DEFAULT
                | glslang_sys::glslang_messages_t::SPV_RULES
                | glslang_sys::glslang_messages_t::VULKAN_RULES,
            resource,
            callbacks: glslang_sys::glsl_include_callbacks_s {
                include_system: None,
                include_local: None,
                free_include_result: None,
            },
            callbacks_ctx: std::ptr::null_mut(),
        }
    }

    fn log(shader: *mut glslang_sys::glslang_shader_t) -> String {
        unsafe {
            CStr::from_ptr(glslang_sys::glslang_shader_get_info_log(shader))
                .to_string_lossy()
                .into_owned()
        }
    }

    /// Runs only the preprocessor and returns the expanded source.
    fn preprocess(&self, stage: Stage, source: &str) -> Result<String> {
        let code = CString::new(source).map_err(|e| anyhow!("{e}"))?;
        let input = Self::input(stage, &code);
        unsafe {
            let shader = glslang_sys::glslang_shader_create(&input);
            let ok = glslang_sys::glslang_shader_preprocess(shader, &input);
            let result = if ok == 0 {
                Err(anyhow!("preprocess failed: {}", Self::log(shader)))
            } else {
                Ok(
                    CStr::from_ptr(glslang_sys::glslang_shader_get_preprocessed_code(shader))
                        .to_string_lossy()
                        .into_owned(),
                )
            };
            glslang_sys::glslang_shader_delete(shader);
            result
        }
    }

    /// Compiles and links both stages, returning SPIR-V words per stage.
    fn compile(&self, vertex: &str, fragment: &str) -> Result<(Vec<u32>, Vec<u32>)> {
        let codes = [
            CString::new(vertex).map_err(|e| anyhow!("{e}"))?,
            CString::new(fragment).map_err(|e| anyhow!("{e}"))?,
        ];
        let stages = [Stage::Vertex, Stage::Fragment];
        unsafe {
            let program = glslang_sys::glslang_program_create();
            let mut shaders = Vec::new();
            let mut inputs = Vec::new();
            for (stage, code) in stages.iter().zip(codes.iter()) {
                inputs.push(Self::input(*stage, code));
            }
            let mut failure = None;
            for (stage, input) in stages.iter().zip(inputs.iter()) {
                let shader = glslang_sys::glslang_shader_create(input);
                glslang_sys::glslang_shader_set_options(
                    shader,
                    (glslang_sys::glslang_shader_options_t::AUTO_MAP_BINDINGS
                        | glslang_sys::glslang_shader_options_t::AUTO_MAP_LOCATIONS
                        | glslang_sys::glslang_shader_options_t::VULKAN_RULES_RELAXED)
                        .0,
                );
                glslang_sys::glslang_shader_set_default_uniform_block_set_and_binding(
                    shader,
                    0,
                    DEFAULT_BLOCK_BINDING,
                );
                let name = CString::new("gl_DefaultUniformBlock").unwrap();
                glslang_sys::glslang_shader_set_default_uniform_block_name(shader, name.as_ptr());
                shaders.push(shader);
                if glslang_sys::glslang_shader_preprocess(shader, input) == 0
                    || glslang_sys::glslang_shader_parse(shader, input) == 0
                {
                    failure = Some(format!("{stage:?}: {}", Self::log(shader)));
                    break;
                }
                glslang_sys::glslang_program_add_shader(program, shader);
            }
            let result = (|| {
                if let Some(f) = failure {
                    bail!("glslang parse failed: {f}");
                }
                let messages = (glslang_sys::glslang_messages_t::DEFAULT
                    | glslang_sys::glslang_messages_t::SPV_RULES
                    | glslang_sys::glslang_messages_t::VULKAN_RULES)
                    .0;
                if glslang_sys::glslang_program_link(program, messages) == 0 {
                    bail!(
                        "glslang link failed: {}",
                        CStr::from_ptr(glslang_sys::glslang_program_get_info_log(program))
                            .to_string_lossy()
                    );
                }
                if glslang_sys::glslang_program_map_io(program) == 0 {
                    bail!(
                        "glslang map_io failed: {}",
                        CStr::from_ptr(glslang_sys::glslang_program_get_info_log(program))
                            .to_string_lossy()
                    );
                }
                let mut words = Vec::new();
                for stage in stages {
                    glslang_sys::glslang_program_SPIRV_generate(program, stage_sys(stage));
                    let size = glslang_sys::glslang_program_SPIRV_get_size(program);
                    let mut buffer = vec![0u32; size];
                    glslang_sys::glslang_program_SPIRV_get(program, buffer.as_mut_ptr());
                    words.push(buffer);
                }
                let fragment = words.pop().unwrap();
                let vertex = words.pop().unwrap();
                Ok((vertex, fragment))
            })();
            glslang_sys::glslang_program_delete(program);
            for shader in shaders {
                glslang_sys::glslang_shader_delete(shader);
            }
            result
        }
    }
}

/// Step 2b: `uniform samplerX name[ N ];` becomes N uniforms `name_pmK`, and every
/// `name[ K ]` (literal K, guaranteed by three.js's loop unrolling) is renamed.
pub fn expand_sampler_arrays(source: &str) -> Result<(String, Vec<(String, u32)>)> {
    let mut arrays: Vec<(String, String, u32)> = Vec::new();
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("uniform ") {
            let rest = rest.trim();
            let mut parts = rest.splitn(2, char::is_whitespace);
            let ty = parts.next().unwrap_or("");
            let decl = parts.next().unwrap_or("").trim();
            let ty = ty.trim();
            let ty = if ["highp", "mediump", "lowp"].contains(&ty) {
                // `uniform highp sampler2D x[2];`
                let mut p2 = decl.splitn(2, char::is_whitespace);
                let real = p2.next().unwrap_or("");
                let d2 = p2.next().unwrap_or("").trim();
                if real.contains("sampler")
                    && let Some((name, n)) = parse_array_decl(d2)
                {
                    arrays.push((real.to_string(), name, n));
                    continue;
                }
                ty
            } else {
                ty
            };
            if ty.contains("sampler")
                && let Some((name, n)) = parse_array_decl(decl)
            {
                arrays.push((ty.to_string(), name, n));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if arrays.is_empty() {
        return Ok((out, Vec::new()));
    }
    // Declarations go where the first array was declared: before any use in practice,
    // since three.js declares uniforms ahead of functions. Insert after `#version` and
    // any extension lines to be safe.
    let mut decls = String::new();
    for (ty, name, n) in &arrays {
        for k in 0..*n {
            decls.push_str(&format!("uniform {ty} {name}_pm{k};\n"));
        }
    }
    let insert_at = out.find('\n').map(|i| i + 1).unwrap_or(0);
    out.insert_str(insert_at, &decls);
    let mut renamed = out;
    let mut mapping = Vec::new();
    for (_, name, n) in &arrays {
        renamed = replace_indexed(&renamed, name, *n)?;
        for k in 0..*n {
            mapping.push((format!("{name}[{k}]"), k));
        }
    }
    Ok((renamed, mapping))
}

fn parse_array_decl(decl: &str) -> Option<(String, u32)> {
    let decl = decl.trim_end_matches(';').trim();
    let open = decl.find('[')?;
    let close = decl.find(']')?;
    let name = decl[..open].trim();
    let n: u32 = decl[open + 1..close].trim().parse().ok()?;
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    Some((name.to_string(), n))
}

fn replace_indexed(source: &str, name: &str, n: u32) -> Result<String> {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < bytes.len() {
        let is_start = source[i..].starts_with(name)
            && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
            && !source[i + name.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_');
        if is_start {
            let rest = &source[i + name.len()..];
            let trimmed = rest.trim_start();
            if let Some(after) = trimmed.strip_prefix('[') {
                let close = after
                    .find(']')
                    .ok_or_else(|| anyhow!("unterminated index for {name}"))?;
                let index_text = after[..close].trim();
                let k: u32 = index_text.parse().map_err(|_| {
                    anyhow!("non-literal index `{index_text}` into sampler array {name}")
                })?;
                if k >= n {
                    bail!("index {k} out of range for sampler array {name}[{n}]");
                }
                out.push_str(&format!("{name}_pm{k}"));
                let consumed = name.len() + (rest.len() - trimmed.len()) + 1 + close + 1;
                i += consumed;
                continue;
            }
        }
        let ch = source[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    Ok(out)
}

/// Step 2d: arrays of varyings (`out vec4 vDirectionalShadowCoord[ 1 ];`) become one
/// varying per element, `name_pmK`; naga only accepts scalars and vectors as entry point
/// inputs and outputs. three.js unrolls its light loops, so every index is a literal.
pub fn expand_io_arrays(source: &str) -> Result<String> {
    let mut arrays: Vec<(String, u32)> = Vec::new();
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        let t = line.trim();
        let tokens: Vec<&str> = t.split_whitespace().collect();
        let io = tokens.iter().any(|w| *w == "in" || *w == "out")
            && !t.contains('(')
            && t.ends_with(';');
        if io {
            let decl = t.trim_end_matches(';');
            if let (Some(open), Some(close)) = (decl.rfind('['), decl.rfind(']')) {
                let n: u32 = decl[open + 1..close]
                    .trim()
                    .parse()
                    .map_err(|_| anyhow!("non-literal varying array size in `{t}`"))?;
                let head = decl[..open].trim_end();
                let name_start = head.rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0);
                let name = &head[name_start..];
                let qualifiers = &head[..name_start];
                for k in 0..n {
                    out.push_str(&format!("{qualifiers}{name}_pm{k};\n"));
                }
                arrays.push((name.to_string(), n));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    for (name, n) in &arrays {
        out = replace_indexed(&out, name, *n)?;
    }
    Ok(out)
}

/// Step 2e: `modf` through a helper. naga's SPIR-V front end produces an unregistered
/// result type for `Modf`; `x - trunc(x)` is exact, so the helper is bit-identical.
pub fn rewrite_modf(source: &str) -> String {
    if !source.contains("modf") {
        return source.to_string();
    }
    let renamed = replace_identifier(source, "modf", "pm_modf");
    let helpers = "float pm_modf(float x, out float i) { i = trunc(x); return x - i; }\nvec2 pm_modf(vec2 x, out vec2 i) { i = trunc(x); return x - i; }\nvec3 pm_modf(vec3 x, out vec3 i) { i = trunc(x); return x - i; }\nvec4 pm_modf(vec4 x, out vec4 i) { i = trunc(x); return x - i; }\n";
    // After `#version` and any `#extension` lines.
    let mut insert_at = 0;
    for line in renamed.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with("#version") || t.starts_with("#extension") || t.is_empty() {
            insert_at += line.len();
        } else {
            break;
        }
    }
    let mut out = renamed;
    out.insert_str(insert_at, helpers);
    out
}

/// Step 2f: explicit, program-wide sampler bindings. glslang's automatic mapping numbers
/// each stage from 1, so a vertex-stage texture (the bone texture) and a fragment-stage
/// texture would collide in one bind group. A name used in both stages gets one binding.
pub fn assign_sampler_bindings(stages: [&str; 2]) -> [String; 2] {
    let mut bindings: Vec<String> = Vec::new();
    let mut out: [String; 2] = [String::new(), String::new()];
    for (k, source) in stages.iter().enumerate() {
        for line in source.lines() {
            let t = line.trim();
            let tokens: Vec<&str> = t.trim_end_matches(';').split_whitespace().collect();
            let is_sampler = tokens.first() == Some(&"uniform")
                && t.ends_with(';')
                && tokens.len() >= 3
                && tokens[tokens.len() - 2].contains("sampler")
                && !t.contains('[');
            if is_sampler {
                let name = tokens[tokens.len() - 1].to_string();
                let index = match bindings.iter().position(|b| *b == name) {
                    Some(i) => i,
                    None => {
                        bindings.push(name);
                        bindings.len() - 1
                    }
                };
                out[k].push_str(&format!("layout(binding = {}) {t}\n", index as u32 + 1));
                continue;
            }
            out[k].push_str(line);
            out[k].push('\n');
        }
    }
    out
}

/// Step 2g: identifiers that are keywords in Vulkan GLSL (`sampler` names a parameter in
/// three.js's transmission code) are renamed.
pub fn rename_vulkan_keywords(source: &str) -> String {
    let mut s = source.to_string();
    for word in ["sampler", "samplerShadow"] {
        if s.contains(word) {
            s = replace_identifier(&s, word, &format!("pm_{word}"));
        }
    }
    s
}

/// Step 2h: fragment inputs the vertex stage never declares. GLSL ES links them when the
/// fragment shader does not statically use them (three.js declares `vNormal` in
/// MeshBasicMaterial unconditionally), but wgpu requires every fragment input to be a
/// vertex output, so the vertex stage gets a matching (never written) output.
pub fn add_missing_varyings(vs: &str, fs: &str) -> String {
    let decls = |src: &str, kw: &str| -> Vec<(String, String)> {
        src.lines()
            .filter_map(|l| {
                let t = l.trim();
                let tokens: Vec<&str> = t.trim_end_matches(';').split_whitespace().collect();
                if !t.ends_with(';')
                    || t.contains('(')
                    || t.starts_with("layout")
                    || !tokens.contains(&kw)
                    || tokens.len() < 3
                {
                    return None;
                }
                Some((tokens[tokens.len() - 1].to_string(), t.to_string()))
            })
            .collect()
    };
    let outs: Vec<String> = decls(vs, "out").into_iter().map(|(n, _)| n).collect();
    let mut extra = String::new();
    for (name, decl) in decls(fs, "in") {
        if !outs.contains(&name) {
            let tokens: Vec<String> = decl
                .split_whitespace()
                .map(|w| {
                    if w == "in" {
                        "out".to_string()
                    } else {
                        w.to_string()
                    }
                })
                .collect();
            extra.push_str(&tokens.join(" "));
            extra.push('\n');
        }
    }
    format!("{vs}{extra}")
}

/// Step 2j: explicit, program-wide varying locations. glslang's automatic mapping
/// numbers each stage's inputs and outputs in declaration order, and three.js declares
/// varyings in different orders per stage (the transmission program declares
/// `vWorldPosition` before `vNormal` in the vertex stage and after it in the fragment
/// stage), so the stages would disagree. GL links varyings by name; this gives every
/// fragment input the location of its position in the fragment stage and the vertex
/// output of the same name the same location. Vertex outputs the fragment stage does not
/// read follow. Runs after [`add_missing_varyings`] and [`expand_io_arrays`], so every
/// varying is a single non-array declaration.
pub fn assign_varying_locations(vs: &str, fs: &str) -> Result<[String; 2]> {
    fn varying(line: &str, kw: &str) -> Option<String> {
        let t = line.trim();
        let tokens: Vec<&str> = t.trim_end_matches(';').split_whitespace().collect();
        if !t.ends_with(';')
            || t.contains('(')
            || t.contains('[')
            || t.starts_with("layout")
            || t.starts_with('#')
            || !tokens.contains(&kw)
            || tokens.len() < 3
        {
            return None;
        }
        Some(tokens[tokens.len() - 1].to_string())
    }
    let mut locations: Vec<String> = fs.lines().filter_map(|l| varying(l, "in")).collect();
    for name in vs.lines().filter_map(|l| varying(l, "out")) {
        if !locations.contains(&name) {
            locations.push(name);
        }
    }
    let rewrite = |src: &str, kw: &str| -> Result<String> {
        let mut out = String::with_capacity(src.len() + locations.len() * 24);
        for line in src.lines() {
            if let Some(name) = varying(line, kw) {
                let index = locations
                    .iter()
                    .position(|n| *n == name)
                    .ok_or_else(|| anyhow!("varying {name} has no location"))?;
                out.push_str(&format!("layout(location = {index}) {}\n", line.trim()));
            } else {
                out.push_str(line);
                out.push('\n');
            }
        }
        Ok(out)
    };
    Ok([rewrite(vs, "out")?, rewrite(fs, "in")?])
}

/// Step 2i: std140 gives `uniform float a[N]` (and `vec2`, `int`, `ivec2`, `uint`,
/// `bool` arrays) a 16-byte stride; naga's MSL back end lays such arrays out at the
/// element size, so later block members move. The array is declared as the matching
/// 4-component vector (same stride, same offsets) and every access takes the original
/// components. Returns the widened names with their original component counts.
pub fn widen_uniform_arrays(source: &str) -> Result<(String, Vec<(String, u32)>)> {
    let mut widened: Vec<(String, u32)> = Vec::new();
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        let t = line.trim();
        let tokens: Vec<&str> = t.trim_end_matches(';').split_whitespace().collect();
        if tokens.first() == Some(&"uniform")
            && t.ends_with(';')
            && t.contains('[')
            && !t.contains('(')
        {
            let ty_index =
                if ["highp", "mediump", "lowp"].contains(&tokens.get(1).copied().unwrap_or("")) {
                    2
                } else {
                    1
                };
            let ty = tokens.get(ty_index).copied().unwrap_or("");
            let wide = match ty {
                "float" => Some(("vec4", 1)),
                "vec2" => Some(("vec4", 2)),
                "int" => Some(("ivec4", 1)),
                "ivec2" => Some(("ivec4", 2)),
                "uint" => Some(("uvec4", 1)),
                "uvec2" => Some(("uvec4", 2)),
                "bool" => Some(("bvec4", 1)),
                "bvec2" => Some(("bvec4", 2)),
                _ => None,
            };
            if let Some((wide_ty, comps)) = wide {
                let rest = tokens[ty_index + 1..].join(" ");
                let name = rest.split('[').next().unwrap_or("").trim().to_string();
                out.push_str(&format!("uniform {wide_ty} {rest};\n"));
                widened.push((name, comps));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    for (name, comps) in &widened {
        let swizzle = if *comps == 1 { ".x" } else { ".xy" };
        out = suffix_indexed(&out, name, swizzle)?;
    }
    Ok((out, widened))
}

/// Appends `suffix` after every `name[ ... ]` (balanced brackets) outside declarations.
fn suffix_indexed(source: &str, name: &str, suffix: &str) -> Result<String> {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len() + 64);
    let mut i = 0;
    while i < bytes.len() {
        let at_name = source[i..].starts_with(name)
            && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
            && !source[i + name.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_');
        if at_name {
            let line_start = source[..i].rfind('\n').map(|p| p + 1).unwrap_or(0);
            let is_decl = source[line_start..i].trim_start().starts_with("uniform");
            let rest = &source[i + name.len()..];
            let trimmed = rest.trim_start();
            if !is_decl && trimmed.starts_with('[') {
                let open = i + name.len() + (rest.len() - trimmed.len());
                let mut depth = 0;
                let mut close = None;
                for (k, ch) in source[open..].char_indices() {
                    match ch {
                        '[' => depth += 1,
                        ']' => {
                            depth -= 1;
                            if depth == 0 {
                                close = Some(open + k);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                let close = close.ok_or_else(|| anyhow!("unbalanced index into {name}"))?;
                out.push_str(&source[i..=close]);
                out.push_str(suffix);
                i = close + 1;
                continue;
            }
        }
        let ch = source[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    Ok(out)
}

/// Step 2c: vertex inputs of matrix type (`in mat4 instanceMatrix;`) become one vec
/// input per column, `name_pmcK`, and uses are rebuilt with a matrix constructor.
/// naga does not accept matrix-typed entry point inputs.
pub fn split_matrix_attributes(source: &str) -> Result<String> {
    let mut out = String::with_capacity(source.len());
    let mut matrices: Vec<(String, u32)> = Vec::new();
    for line in source.lines() {
        let t = line.trim();
        let tokens: Vec<&str> = t.trim_end_matches(';').split_whitespace().collect();
        if tokens.len() >= 3 && tokens[0] == "in" && t.ends_with(';') {
            let (ty, name) = (tokens[tokens.len() - 2], tokens[tokens.len() - 1]);
            let cols = match ty {
                "mat2" => 2,
                "mat3" => 3,
                "mat4" => 4,
                _ => 0,
            };
            if cols > 0 {
                for k in 0..cols {
                    out.push_str(&format!("in vec{cols} {name}_pmc{k};\n"));
                }
                matrices.push((name.to_string(), cols));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    for (name, cols) in &matrices {
        let ctor = format!(
            "mat{cols}({})",
            (0..*cols)
                .map(|k| format!("{name}_pmc{k}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        out = replace_identifier(&out, name, &ctor);
    }
    Ok(out)
}

fn replace_identifier(source: &str, name: &str, with: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < bytes.len() {
        if source[i..].starts_with(name)
            && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
            && !source[i + name.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
        {
            out.push_str(with);
            i += name.len();
            continue;
        }
        let ch = source[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Step 3: rename the vertex `main` and append [`VERTEX_EPILOGUE`].
pub fn wrap_vertex_main(source: &str) -> Result<String> {
    Ok(format!(
        "{}{VERTEX_EPILOGUE}",
        rename_main(source).context("vertex shader")?
    ))
}

fn rename_main(source: &str) -> Result<String> {
    let mut found = false;
    let mut out = String::with_capacity(source.len() + 128);
    let mut rest = source;
    while let Some(pos) = rest.find("void main") {
        let after = rest[pos + 9..].trim_start();
        if after.starts_with('(') {
            out.push_str(&rest[..pos]);
            out.push_str("void pm_main");
            rest = &rest[pos + 9..];
            found = true;
        } else {
            out.push_str(&rest[..pos + 9]);
            rest = &rest[pos + 9..];
        }
    }
    out.push_str(rest);
    if !found {
        bail!("no main()");
    }
    Ok(out)
}

/// Step 5b: split every combined image-sampler variable into an image variable (same
/// binding) and a sampler variable (binding + [`SAMPLER_BINDING_OFFSET`]).
pub fn split_combined_samplers(words: &[u32]) -> Result<Vec<u32>> {
    use rspirv::dr::{Instruction, Operand};
    use rspirv::spirv::{Decoration, Op, StorageClass};
    let mut module = rspirv::dr::load_words(words).map_err(|e| anyhow!("rspirv load: {e:?}"))?;
    let mut bound = module.header.as_ref().map(|h| h.bound).unwrap_or(0);
    let mut fresh = || {
        let id = bound;
        bound += 1;
        id
    };
    let mut sampled_image_types: HashMap<u32, u32> = HashMap::new(); // si -> image
    let mut ptr_to_si: HashMap<u32, u32> = HashMap::new(); // ptr -> si
    let mut sampler_type = None;
    let mut ptr_types: HashMap<(u32, u32), u32> = HashMap::new(); // (storage, pointee) -> ptr
    for inst in &module.types_global_values {
        match inst.class.opcode {
            Op::TypeSampledImage => {
                sampled_image_types
                    .insert(inst.result_id.unwrap(), inst.operands[0].unwrap_id_ref());
            }
            Op::TypeSampler => sampler_type = inst.result_id,
            Op::TypePointer => {
                let storage = match inst.operands[0] {
                    Operand::StorageClass(s) => s as u32,
                    _ => continue,
                };
                ptr_types.insert(
                    (storage, inst.operands[1].unwrap_id_ref()),
                    inst.result_id.unwrap(),
                );
            }
            Op::TypeArray | Op::TypeRuntimeArray
                if sampled_image_types.contains_key(&inst.operands[0].unwrap_id_ref()) =>
            {
                bail!("arrays of combined samplers are not supported (expand them in GLSL)");
            }
            _ => {}
        }
    }
    for ((storage, pointee), ptr) in &ptr_types {
        if *storage == StorageClass::UniformConstant as u32
            && sampled_image_types.contains_key(pointee)
        {
            ptr_to_si.insert(*ptr, *pointee);
        }
    }
    if ptr_to_si.is_empty() {
        return Ok(words.to_vec());
    }
    // Combined variables.
    let mut combined: BTreeMap<u32, (u32, u32)> = BTreeMap::new(); // var -> (si, image)
    for inst in &module.types_global_values {
        if inst.class.opcode == Op::Variable
            && let Some(si) = ptr_to_si.get(&inst.result_type.unwrap())
        {
            combined.insert(inst.result_id.unwrap(), (*si, sampled_image_types[si]));
        }
    }
    if combined.is_empty() {
        return Ok(words.to_vec());
    }
    let uc = StorageClass::UniformConstant as u32;
    // New types appended at the end of the type section would follow the variables that
    // use them, so build a new type list: every new pointer type goes right after the
    // instruction defining its pointee.
    let sampler_ty = sampler_type.unwrap_or_else(&mut fresh);
    let sampler_ptr = ptr_types
        .get(&(uc, sampler_ty))
        .copied()
        .unwrap_or_else(&mut fresh);
    let mut image_ptrs: HashMap<u32, u32> = HashMap::new();
    for (_, image) in combined.values() {
        if !image_ptrs.contains_key(image) {
            let id = ptr_types
                .get(&(uc, *image))
                .copied()
                .unwrap_or_else(&mut fresh);
            image_ptrs.insert(*image, id);
        }
    }
    let mut sampler_vars: HashMap<u32, u32> = HashMap::new();
    for var in combined.keys() {
        sampler_vars.insert(*var, fresh());
    }
    let existing_ptrs: std::collections::HashSet<u32> = ptr_types.values().copied().collect();
    let mut types = Vec::with_capacity(module.types_global_values.len() + 8);
    let mut emitted_sampler = sampler_type.is_some() && existing_ptrs.contains(&sampler_ptr);
    for inst in module.types_global_values.drain(..) {
        let id = inst.result_id;
        let opcode = inst.class.opcode;
        let mut inst = inst;
        if opcode == Op::Variable
            && let Some((_, image)) = combined.get(&id.unwrap())
        {
            if !emitted_sampler {
                if sampler_type.is_none() {
                    types.push(Instruction::new(
                        Op::TypeSampler,
                        None,
                        Some(sampler_ty),
                        vec![],
                    ));
                }
                types.push(Instruction::new(
                    Op::TypePointer,
                    None,
                    Some(sampler_ptr),
                    vec![
                        Operand::StorageClass(StorageClass::UniformConstant),
                        Operand::IdRef(sampler_ty),
                    ],
                ));
                emitted_sampler = true;
            }
            inst.result_type = Some(image_ptrs[image]);
            let var = id.unwrap();
            types.push(inst);
            types.push(Instruction::new(
                Op::Variable,
                Some(sampler_ptr),
                Some(sampler_vars[&var]),
                vec![Operand::StorageClass(StorageClass::UniformConstant)],
            ));
            continue;
        }
        types.push(inst);
        if let Some(id) = id
            && let Some(ptr) = image_ptrs.get(&id)
            && !existing_ptrs.contains(ptr)
        {
            types.push(Instruction::new(
                Op::TypePointer,
                None,
                Some(*ptr),
                vec![
                    Operand::StorageClass(StorageClass::UniformConstant),
                    Operand::IdRef(id),
                ],
            ));
        }
    }
    module.types_global_values = types;
    // Decorations and names for the sampler variables.
    let mut extra_annotations = Vec::new();
    for inst in &module.annotations {
        if inst.class.opcode != Op::Decorate {
            continue;
        }
        let target = inst.operands[0].unwrap_id_ref();
        if let Some(svar) = sampler_vars.get(&target) {
            match inst.operands[1] {
                Operand::Decoration(Decoration::DescriptorSet) => {
                    extra_annotations.push(Instruction::new(
                        Op::Decorate,
                        None,
                        None,
                        vec![
                            Operand::IdRef(*svar),
                            Operand::Decoration(Decoration::DescriptorSet),
                            inst.operands[2].clone(),
                        ],
                    ))
                }
                Operand::Decoration(Decoration::Binding) => {
                    let binding = match inst.operands[2] {
                        Operand::LiteralBit32(b) => b,
                        _ => bail!("unexpected binding operand"),
                    };
                    extra_annotations.push(Instruction::new(
                        Op::Decorate,
                        None,
                        None,
                        vec![
                            Operand::IdRef(*svar),
                            Operand::Decoration(Decoration::Binding),
                            Operand::LiteralBit32(binding + SAMPLER_BINDING_OFFSET),
                        ],
                    ));
                }
                _ => {}
            }
        }
    }
    module.annotations.extend(extra_annotations);
    let mut names = Vec::new();
    for inst in &module.debug_names {
        if inst.class.opcode == Op::Name
            && let Some(svar) = sampler_vars.get(&inst.operands[0].unwrap_id_ref())
            && let Operand::LiteralString(s) = &inst.operands[1]
        {
            names.push(Instruction::new(
                Op::Name,
                None,
                None,
                vec![
                    Operand::IdRef(*svar),
                    Operand::LiteralString(format!("{s}_pmsampler")),
                ],
            ));
        }
    }
    module.debug_names.extend(names);
    // Entry point interfaces (SPIR-V 1.4+ lists every global; harmless otherwise).
    for ep in &mut module.entry_points {
        let mut add = Vec::new();
        for op in ep.operands.iter().skip(3) {
            if let Operand::IdRef(id) = op
                && let Some(s) = sampler_vars.get(id)
            {
                add.push(Operand::IdRef(*s));
            }
        }
        ep.operands.extend(add);
    }
    // Loads.
    for function in &mut module.functions {
        for block in &mut function.blocks {
            let mut rewritten = Vec::with_capacity(block.instructions.len());
            for inst in block.instructions.drain(..) {
                if inst.class.opcode == Op::Load {
                    let pointer = inst.operands[0].unwrap_id_ref();
                    if let Some((si, image)) = combined.get(&pointer) {
                        let image_value = fresh();
                        let sampler_value = fresh();
                        rewritten.push(Instruction::new(
                            Op::Load,
                            Some(*image),
                            Some(image_value),
                            vec![Operand::IdRef(pointer)],
                        ));
                        rewritten.push(Instruction::new(
                            Op::Load,
                            Some(sampler_ty),
                            Some(sampler_value),
                            vec![Operand::IdRef(sampler_vars[&pointer])],
                        ));
                        rewritten.push(Instruction::new(
                            Op::SampledImage,
                            Some(*si),
                            inst.result_id,
                            vec![Operand::IdRef(image_value), Operand::IdRef(sampler_value)],
                        ));
                        continue;
                    }
                }
                if inst
                    .operands
                    .iter()
                    .any(|o| matches!(o, Operand::IdRef(id) if combined.contains_key(id)))
                    && inst.class.opcode != Op::Load
                {
                    bail!(
                        "combined sampler variable used by {:?} (only loads are supported)",
                        inst.class.opcode
                    );
                }
                rewritten.push(inst);
            }
            block.instructions = rewritten;
        }
    }
    if let Some(h) = module.header.as_mut() {
        h.bound = bound;
    }
    use rspirv::binary::Assemble;
    Ok(module.assemble())
}

/// Refuses programs with functions that take opaque (image or sampler)
/// parameters: naga's SPIR-V front end needs them inlined first, which
/// SPIRV-Tools' `InlineOpaque` pass does; this crate does not link it yet.
fn refuse_opaque_parameters(words: &[u32]) -> Result<()> {
    use rspirv::spirv::Op;
    let module = rspirv::dr::load_words(words).map_err(|e| anyhow!("rspirv load: {e:?}"))?;
    let mut opaque = std::collections::HashSet::new();
    for inst in &module.types_global_values {
        match inst.class.opcode {
            Op::TypeImage | Op::TypeSampler | Op::TypeSampledImage => {
                opaque.insert(inst.result_id.unwrap_or(0));
            }
            Op::TypePointer => {
                if let Some(pointee) = inst.operands.get(1).map(|o| o.unwrap_id_ref())
                    && opaque.contains(&pointee)
                {
                    opaque.insert(inst.result_id.unwrap_or(0));
                }
            }
            _ => {}
        }
    }
    let names: HashMap<u32, String> = module
        .debug_names
        .iter()
        .filter_map(|n| match (&n.operands.first(), &n.operands.get(1)) {
            (Some(rspirv::dr::Operand::IdRef(id)), Some(rspirv::dr::Operand::LiteralString(s))) => {
                Some((*id, s.clone()))
            }
            _ => None,
        })
        .collect();
    for function in &module.functions {
        if function
            .parameters
            .iter()
            .any(|p| p.result_type.is_some_and(|t| opaque.contains(&t)))
        {
            let id = function.def.as_ref().and_then(|d| d.result_id).unwrap_or(0);
            bail!(
                "function {} takes a sampler parameter; programs like this need SPIRV-Tools' opaque inliner, not linked yet",
                names.get(&id).cloned().unwrap_or_else(|| format!("%{id}"))
            );
        }
    }
    Ok(())
}

fn to_naga(words: &[u32]) -> Result<(naga::Module, naga::valid::ModuleInfo)> {
    let options = naga::front::spv::Options {
        adjust_coordinate_space: false,
        strict_capabilities: false,
        block_ctx_dump_prefix: None,
    };
    let mut module = naga::front::spv::Frontend::new(words.iter().cloned(), &options)
        .parse()
        .map_err(|e| anyhow!("naga spv-in: {e:?}"))?;
    pad_struct_spans(&mut module)?;
    let info = validate(&module)?;
    Ok((module, info))
}

fn validate(module: &naga::Module) -> Result<naga::valid::ModuleInfo> {
    let flags =
        naga::valid::ValidationFlags::all() ^ naga::valid::ValidationFlags::CONTROL_FLOW_UNIFORMITY;
    naga::valid::Validator::new(flags, naga::valid::Capabilities::all())
        .validate(module)
        .map_err(|e| anyhow!("naga validation: {}", e.emit_to_string("")))
}

/// Step 6b: every fragment entry point also returns an all-ones `sample_mask`, as ANGLE's
/// Metal back end does for every WebGL fragment shader on Apple GPUs (its
/// `writeHelperSampleMask` workaround, and the coverage mask when multisampled). Without
/// a sample-mask write, Apple GPUs shade 2×2 quads that span two triangles of one draw,
/// so `dFdx`/`dFdy` at triangle edges mix both primitives: flat-shading normals and
/// `geometryRoughness` go wrong along every edge. The mask is ANDed with raster
/// coverage, so nothing else changes. (GLSL `gl_SampleMask` is an `int[]` that naga's
/// SPIR-V front end rejects, so this is done on the IR.)
pub fn add_sample_mask_output(module: &mut naga::Module) -> Result<()> {
    use naga::{
        Binding, BuiltIn, Expression, Literal, Scalar, Span, Statement, StructMember, Type,
        TypeInner,
    };
    let u32_ty = module.types.insert(
        Type {
            name: None,
            inner: TypeInner::Scalar(Scalar::U32),
        },
        Span::UNDEFINED,
    );
    for ep_index in 0..module.entry_points.len() {
        if module.entry_points[ep_index].stage != naga::ShaderStage::Fragment {
            continue;
        }
        let Some(result) = module.entry_points[ep_index].function.result.clone() else {
            bail!("fragment entry point without outputs");
        };
        // The new result struct: the old members (or the single bound output) plus the mask.
        let (mut members, single) = match &module.types[result.ty].inner {
            TypeInner::Struct { members, .. } => (members.clone(), false),
            _ => (
                vec![StructMember {
                    name: Some("pm_color".into()),
                    ty: result.ty,
                    binding: result.binding.clone(),
                    offset: 0,
                }],
                true,
            ),
        };
        if members
            .iter()
            .any(|m| matches!(m.binding, Some(Binding::BuiltIn(BuiltIn::SampleMask))))
        {
            continue;
        }
        let mut offset = 0u32;
        for m in members.iter_mut() {
            let inner = &module.types[m.ty].inner;
            let align = inner
                .size(module.to_ctx())
                .min(16)
                .next_power_of_two()
                .max(4);
            offset = offset.div_ceil(align) * align;
            m.offset = offset;
            offset += inner.size(module.to_ctx());
        }
        members.push(StructMember {
            name: Some("pm_sample_mask".into()),
            ty: u32_ty,
            binding: Some(Binding::BuiltIn(BuiltIn::SampleMask)),
            offset,
        });
        let span = (offset + 4).div_ceil(16) * 16;
        let count = members.len();
        let new_ty = module.types.insert(
            Type {
                name: Some("PmFragmentOutput".into()),
                inner: TypeInner::Struct { members, span },
            },
            Span::UNDEFINED,
        );
        let ep = &mut module.entry_points[ep_index];
        ep.function.result = Some(naga::FunctionResult {
            ty: new_ty,
            binding: None,
        });
        // Rewrite every `return value;` in the entry point's top-level body.
        let body = std::mem::take(&mut ep.function.body);
        let mut rewritten = naga::Block::with_capacity(body.len() + 2);
        let mut returns = 0;
        for (statement, span) in body.span_into_iter() {
            match statement {
                Statement::Return { value: Some(value) } => {
                    returns += 1;
                    let exprs = &mut ep.function.expressions;
                    let mask = exprs.append(Expression::Literal(Literal::U32(u32::MAX)), span);
                    let mut components = Vec::with_capacity(count);
                    let first = if single {
                        components.push(value);
                        None
                    } else {
                        let mut first = None;
                        for index in 0..count as u32 - 1 {
                            let h =
                                exprs.append(Expression::AccessIndex { base: value, index }, span);
                            first.get_or_insert(h);
                            components.push(h);
                        }
                        first
                    };
                    components.push(mask);
                    let compose = exprs.append(
                        Expression::Compose {
                            ty: new_ty,
                            components,
                        },
                        span,
                    );
                    rewritten.push(
                        Statement::Emit(naga::Range::new_from_bounds(
                            first.unwrap_or(compose),
                            compose,
                        )),
                        span,
                    );
                    rewritten.push(
                        Statement::Return {
                            value: Some(compose),
                        },
                        span,
                    );
                }
                Statement::Return { value: None } => bail!("fragment entry point returns no value"),
                other => rewritten.push(other, span),
            }
        }
        if returns == 0 {
            bail!("fragment entry point has no top-level return");
        }
        ep.function.body = rewritten;
    }
    Ok(())
}

/// SPIR-V does not record struct sizes, so naga gives a std140 struct its natural span
/// (`{float x4, vec2}` = 24) while the array holding it has stride 32. The MSL back end
/// then lays the array out at the struct's size and every later block member moves.
/// Widen such structs to their array stride.
pub fn pad_struct_spans(module: &mut naga::Module) -> Result<()> {
    let mut needed: BTreeMap<naga::Handle<naga::Type>, u32> = BTreeMap::new();
    for (_, ty) in module.types.iter() {
        if let naga::TypeInner::Array { base, stride, .. } = ty.inner
            && let naga::TypeInner::Struct { span, .. } = module.types[base].inner
            && span < stride
        {
            let e = needed.entry(base).or_insert(stride);
            *e = (*e).max(stride);
        }
    }
    for (handle, stride) in needed {
        let old = module.types[handle].clone();
        let naga::TypeInner::Struct { members, .. } = old.inner else {
            unreachable!()
        };
        let new = naga::Type {
            name: old.name.clone(),
            inner: naga::TypeInner::Struct {
                members,
                span: stride,
            },
        };
        if module.types.get(&new).is_some() {
            bail!("padded struct {:?} already exists", old.name);
        }
        module.types.replace(handle, new);
    }
    Ok(())
}

/// Compiles a three.js program (vertex and fragment sources exactly as WebGL received them).
pub fn compile(vertex_src: &str, fragment_src: &str) -> Result<Program> {
    let glslang = Glslang::get()?;
    let vs = glslang
        .preprocess(Stage::Vertex, &normalize(vertex_src))
        .context("vertex preprocess")?;
    let fs = glslang
        .preprocess(Stage::Fragment, &normalize(fragment_src))
        .context("fragment preprocess")?;
    let (vs, vs_arrays) = expand_sampler_arrays(&vs)?;
    let (fs, fs_arrays) = expand_sampler_arrays(&fs)?;
    let vs = split_matrix_attributes(&vs)?;
    let vs = rename_vulkan_keywords(&rewrite_modf(&expand_io_arrays(&vs)?));
    let fs = rename_vulkan_keywords(&rewrite_modf(&expand_io_arrays(&fs)?));
    let (vs, vs_wide) = widen_uniform_arrays(&vs)?;
    let (fs, fs_wide) = widen_uniform_arrays(&fs)?;
    let [vs, fs] = assign_sampler_bindings([&vs, &fs]);
    let vs = add_missing_varyings(&vs, &fs);
    let [vs, fs] = assign_varying_locations(&vs, &fs)?;
    let vs = wrap_vertex_main(&vs)?;
    let (vs_spv, fs_spv) = glslang.compile(&vs, &fs)?;
    refuse_opaque_parameters(&vs_spv).context("vertex")?;
    refuse_opaque_parameters(&fs_spv).context("fragment")?;
    let vs_spv = split_combined_samplers(&vs_spv).context("vertex split")?;
    let fs_spv = split_combined_samplers(&fs_spv).context("fragment split")?;
    let (vertex, vertex_info) = to_naga(&vs_spv).context("vertex")?;
    let (mut fragment, _) = to_naga(&fs_spv).context("fragment")?;
    add_sample_mask_output(&mut fragment)?;
    let fragment_info = validate(&fragment).context("fragment with sample mask")?;
    let mut program = Program {
        vertex,
        fragment,
        vertex_info,
        fragment_info,
        block_size: 0,
        block_stages: Vec::new(),
        uniforms: HashMap::new(),
        textures: HashMap::new(),
        attributes: HashMap::new(),
        outputs: Vec::new(),
        preprocessed: [vs, fs],
    };
    let array_names: HashMap<String, String> = vs_arrays
        .into_iter()
        .chain(fs_arrays)
        .map(|(gl, _)| {
            let open = gl.find('[').unwrap();
            let k = &gl[open + 1..gl.len() - 1];
            (format!("{}_pm{}", &gl[..open], k), gl)
        })
        .collect();
    reflect(&mut program, Stage::Vertex, &array_names)?;
    reflect(&mut program, Stage::Fragment, &array_names)?;
    // Widened arrays keep their GL element size for uniform writes.
    for (name, comps) in vs_wide.iter().chain(fs_wide.iter()) {
        for (key, slot) in program.uniforms.iter_mut() {
            if key == name || (key.starts_with(name.as_str()) && key[name.len()..].starts_with('['))
            {
                slot.rows = *comps;
            }
        }
    }
    Ok(program)
}

/// Builds a [`Program`] from hand-written WGSL (the comparison path for S7). The modules
/// follow the same conventions as compiled GLSL: one default uniform block at group 0,
/// binding [`DEFAULT_BLOCK_BINDING`], whose member names are the GL uniform names; each
/// texture at its binding with its sampler at binding + [`SAMPLER_BINDING_OFFSET`]; GL
/// sampler arrays as `name_pmK`; vertex inputs named as the GL attributes; and the
/// replay's clip-space conventions applied by the vertex stage itself.
pub fn from_wgsl(vertex_wgsl: &str, fragment_wgsl: &str) -> Result<Program> {
    let parse = |src: &str, stage: &str| -> Result<(naga::Module, naga::valid::ModuleInfo)> {
        let module = naga::front::wgsl::parse_str(src)
            .map_err(|e| anyhow!("{stage} WGSL: {}", e.emit_to_string(src)))?;
        let info = validate(&module).map_err(|e| anyhow!("{stage} WGSL: {e}"))?;
        Ok((module, info))
    };
    let (vertex, vertex_info) = parse(vertex_wgsl, "vertex")?;
    let (fragment, fragment_info) = parse(fragment_wgsl, "fragment")?;
    let mut program = Program {
        vertex,
        fragment,
        vertex_info,
        fragment_info,
        block_size: 0,
        block_stages: Vec::new(),
        uniforms: HashMap::new(),
        textures: HashMap::new(),
        attributes: HashMap::new(),
        outputs: Vec::new(),
        preprocessed: [vertex_wgsl.to_string(), fragment_wgsl.to_string()],
    };
    let mut array_names = HashMap::new();
    for module in [&program.vertex, &program.fragment] {
        for (_, var) in module.global_variables.iter() {
            if let Some(name) = &var.name
                && let Some(pos) = name.rfind("_pm")
                && let Ok(k) = name[pos + 3..].parse::<u32>()
            {
                array_names.insert(name.clone(), format!("{}[{k}]", &name[..pos]));
            }
        }
    }
    reflect(&mut program, Stage::Vertex, &array_names)?;
    reflect(&mut program, Stage::Fragment, &array_names)?;
    Ok(program)
}

fn reflect(
    program: &mut Program,
    stage: Stage,
    array_names: &HashMap<String, String>,
) -> Result<()> {
    let module = match stage {
        Stage::Vertex => &program.vertex,
        Stage::Fragment => &program.fragment,
    };
    let mut images = Vec::new();
    for (_, var) in module.global_variables.iter() {
        let Some(binding) = &var.binding else {
            continue;
        };
        let ty = &module.types[var.ty];
        match (&ty.inner, var.space) {
            (naga::TypeInner::Struct { members, span }, naga::AddressSpace::Uniform) => {
                program.block_size = program.block_size.max(*span);
                program.block_stages.push(stage);
                if binding.binding != DEFAULT_BLOCK_BINDING {
                    bail!("unexpected uniform block binding {}", binding.binding);
                }
                for member in members {
                    let name = member.name.clone().unwrap_or_default();
                    flatten(
                        module,
                        &name,
                        member.ty,
                        member.offset,
                        &mut program.uniforms,
                    );
                }
            }
            (
                naga::TypeInner::Image {
                    dim,
                    arrayed,
                    class,
                },
                _,
            ) => {
                let raw = var.name.clone().unwrap_or_default();
                let gl = array_names.get(&raw).cloned().unwrap_or(raw);
                images.push((gl, binding.binding, *dim, *arrayed, *class));
            }
            _ => {}
        }
    }
    for (gl, binding, dim, arrayed, class) in images {
        let slot = program.textures.entry(gl).or_insert(TextureSlot {
            binding,
            sampler_binding: binding + SAMPLER_BINDING_OFFSET,
            dim,
            arrayed,
            class,
            stages: Vec::new(),
        });
        if slot.binding != binding {
            bail!("texture binding differs between stages");
        }
        slot.stages.push(stage);
    }
    let entry = module
        .entry_points
        .first()
        .ok_or_else(|| anyhow!("no entry point"))?;
    for arg in &entry.function.arguments {
        if let Some(naga::Binding::Location { location, .. }) = &arg.binding
            && stage == Stage::Vertex
        {
            let name = arg.name.clone().unwrap_or_default();
            program.attributes.insert(
                name.clone(),
                attribute(&name, *location, module.types[arg.ty].inner.clone()),
            );
        }
        if arg.binding.is_none()
            && let naga::TypeInner::Struct { members, .. } = &module.types[arg.ty].inner
        {
            for m in members {
                if let Some(naga::Binding::Location { location, .. }) = &m.binding
                    && stage == Stage::Vertex
                {
                    let name = m.name.clone().unwrap_or_default();
                    program.attributes.insert(
                        name.clone(),
                        attribute(&name, *location, module.types[m.ty].inner.clone()),
                    );
                }
            }
        }
    }
    if stage == Stage::Fragment
        && let Some(result) = &entry.function.result
    {
        match &result.binding {
            Some(naga::Binding::Location { location, .. }) => program.outputs.push(*location),
            None => {
                if let naga::TypeInner::Struct { members, .. } = &module.types[result.ty].inner {
                    for m in members {
                        if let Some(naga::Binding::Location { location, .. }) = &m.binding {
                            program.outputs.push(*location);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn attribute(name: &str, location: u32, ty: naga::TypeInner) -> Attribute {
    if let Some(pos) = name.rfind("_pmc")
        && let Ok(column) = name[pos + 4..].parse::<u32>()
    {
        return Attribute {
            location,
            ty,
            gl_name: name[..pos].to_string(),
            column,
        };
    }
    Attribute {
        location,
        ty,
        gl_name: name.to_string(),
        column: 0,
    }
}

fn flatten(
    module: &naga::Module,
    name: &str,
    ty: naga::Handle<naga::Type>,
    offset: u32,
    out: &mut HashMap<String, UniformSlot>,
) {
    use naga::{ScalarKind as K, TypeInner as T};
    let scalar_kind = |s: naga::Scalar| match s.kind {
        K::Float => ScalarKind::Float,
        K::Sint => ScalarKind::Sint,
        K::Uint => ScalarKind::Uint,
        K::Bool => ScalarKind::Bool,
        _ => ScalarKind::Float,
    };
    match &module.types[ty].inner {
        T::Scalar(s) => {
            out.insert(
                name.to_string(),
                UniformSlot {
                    offset,
                    kind: scalar_kind(*s),
                    columns: 1,
                    rows: 1,
                    column_stride: 0,
                    count: 1,
                    array_stride: 0,
                },
            );
        }
        T::Vector { size, scalar } => {
            out.insert(
                name.to_string(),
                UniformSlot {
                    offset,
                    kind: scalar_kind(*scalar),
                    columns: 1,
                    rows: *size as u32,
                    column_stride: 0,
                    count: 1,
                    array_stride: 0,
                },
            );
        }
        T::Matrix {
            columns,
            rows,
            scalar,
        } => {
            let stride = if *rows as u32 == 2 { 8 } else { 16 };
            out.insert(
                name.to_string(),
                UniformSlot {
                    offset,
                    kind: scalar_kind(*scalar),
                    columns: *columns as u32,
                    rows: *rows as u32,
                    column_stride: stride,
                    count: 1,
                    array_stride: 0,
                },
            );
        }
        T::Struct { members, .. } => {
            for m in members {
                let child = format!("{name}.{}", m.name.clone().unwrap_or_default());
                flatten(module, &child, m.ty, offset + m.offset, out);
            }
        }
        T::Array { base, size, stride } => {
            let n = match size {
                naga::ArraySize::Constant(n) => n.get(),
                _ => 0,
            };
            let base_is_leaf = !matches!(
                module.types[*base].inner,
                T::Struct { .. } | T::Array { .. }
            );
            for i in 0..n {
                flatten(
                    module,
                    &format!("{name}[{i}]"),
                    *base,
                    offset + i * stride,
                    out,
                );
                if base_is_leaf && let Some(slot) = out.get_mut(&format!("{name}[{i}]")) {
                    slot.count = n - i;
                    slot.array_stride = *stride;
                }
            }
            if base_is_leaf && let Some(first) = out.get(&format!("{name}[0]")).cloned() {
                out.insert(name.to_string(), first);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VS: &str = "#version 300 es\nprecision highp float;\nin vec3 position;\nin vec2 uv;\nuniform mat4 projectionMatrix;\nuniform mat4 modelViewMatrix;\nout vec2 vUv;\nvoid main() { vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }\n";
    const FS: &str = "#version 300 es\nprecision highp float;\nprecision highp sampler2D;\n#define N 2\nuniform sampler2D maps[ N ];\nuniform sampler2D single;\nuniform vec3 tint;\nuniform bool flip;\nstruct L { vec3 dir; float k; };\nuniform L lights[ 3 ];\nin vec2 vUv;\nlayout(location = 0) out highp vec4 pc_fragColor;\nvoid main() { vec4 c = texture( maps[ 0 ], vUv ) + texture( maps[ 1 ], vUv ) + texture( single, vUv ) * float( textureSize( single, 0 ).x ); if ( flip ) c.rgb = 1.0 - c.rgb; pc_fragColor = vec4( c.rgb * tint * lights[ 2 ].dir * lights[ 1 ].k, c.a ); }\n";

    #[test]
    fn compiles_and_reflects() {
        let p = compile(VS, FS).expect("compile");
        assert!(p.uniforms.contains_key("projectionMatrix"));
        assert!(p.uniforms.contains_key("tint"));
        assert!(p.uniforms.contains_key("lights[2].dir"));
        assert_eq!(p.uniforms["flip"].kind, ScalarKind::Uint);
        assert!(
            p.textures.contains_key("maps[0]")
                && p.textures.contains_key("maps[1]")
                && p.textures.contains_key("single")
        );
        assert_eq!(p.attributes.len(), 2);
        assert_eq!(p.outputs, vec![0]);
        // Uniform block shared by both stages.
        assert_eq!(p.block_stages.len(), 2);
    }

    #[test]
    fn refuses_sampler_parameters_instead_of_compiling_them_wrong() {
        let fs = "#version 300 es\nprecision highp float;\nuniform sampler2D m;\nin vec2 vUv;\nlayout(location = 0) out vec4 o;\nvec4 get( sampler2D s, vec2 uv ) { return texture( s, uv ); }\nvoid main() { o = get( m, vUv ); }\n";
        let error = compile(VS, fs).err().expect("refused");
        assert!(error.0.contains("sampler parameter"), "{error}");
    }

    #[test]
    fn splits_matrix_attributes() {
        let vs = "#version 300 es\nin vec3 position;\nin mat4 instanceMatrix;\nuniform mat4 projectionMatrix;\nvoid main() { gl_Position = projectionMatrix * instanceMatrix * vec4(position, 1.0); }\n";
        let fs = "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(1.0); }\n";
        let p = compile(vs, fs).expect("compile");
        let cols: Vec<_> = p
            .attributes
            .values()
            .filter(|a| a.gl_name == "instanceMatrix")
            .map(|a| a.column)
            .collect();
        assert_eq!(cols.len(), 4);
    }

    #[test]
    fn expands_varying_arrays_and_modf() {
        let vs = "#version 300 es\nin vec3 position;\nout vec4 vCoord[ 2 ];\nvoid main() { vCoord[ 0 ] = vec4(position, 1.0); vCoord[ 1 ] = vec4(1.0); gl_Position = vCoord[ 0 ]; }\n";
        let fs = "#version 300 es\nprecision highp float;\nin vec4 vCoord[ 2 ];\nlayout(location = 0) out vec4 o;\nvoid main() { float i; float f = modf(vCoord[ 1 ].x * 3.5, i); o = vCoord[ 0 ] + vec4(f, i, 0.0, 0.0); }\n";
        let p = compile(vs, fs).expect("compile");
        assert!(p.preprocessed[0].contains("vCoord_pm1") && p.preprocessed[1].contains("pm_modf"));
    }

    #[test]
    fn widens_scalar_uniform_arrays() {
        let vs = "#version 300 es\nin vec3 position;\nuniform float w[ 3 ];\nuniform float after;\nvoid main() { float s = 0.0; for (int i = 0; i < 3; i++) s += w[ i ]; gl_Position = vec4(position * s * after, 1.0); }\n";
        let fs = "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(1.0); }\n";
        let p = compile(vs, fs).expect("compile");
        assert_eq!(p.uniforms["w[1]"].offset, 16);
        assert_eq!(p.uniforms["w[1]"].rows, 1);
        assert_eq!(p.uniforms["w"].count, 3);
        assert_eq!(p.uniforms["after"].offset, 48);
    }

    #[test]
    fn varyings_link_by_name_not_declaration_order() {
        // The vertex stage declares a, extra, b; the fragment stage reads b, then a.
        let vs = "#version 300 es\nin vec3 position;\nout vec3 a;\nout float extra;\nout vec3 b;\nvoid main() { a = position; extra = 1.0; b = -position; gl_Position = vec4(position, 1.0); }\n";
        let fs = "#version 300 es\nprecision highp float;\nin vec3 b;\nin vec3 a;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(a - b, 1.0); }\n";
        let p = compile(vs, fs).expect("compile");
        let [v, f] = &p.preprocessed;
        assert!(
            f.contains("layout(location = 0) in vec3 b;")
                && f.contains("layout(location = 1) in vec3 a;"),
            "{f}"
        );
        assert!(
            v.contains("layout(location = 0) out vec3 b;")
                && v.contains("layout(location = 1) out vec3 a;")
                && v.contains("layout(location = 2) out float extra;"),
            "{v}"
        );
        let inputs: HashMap<String, u32> = p.fragment.entry_points[0]
            .function
            .arguments
            .iter()
            .filter_map(|a| match (&a.name, &a.binding) {
                (Some(n), Some(naga::Binding::Location { location, .. })) => {
                    Some((n.clone(), *location))
                }
                _ => None,
            })
            .collect();
        assert_eq!(inputs.get("b"), Some(&0));
        assert_eq!(inputs.get("a"), Some(&1));
    }

    #[test]
    fn expands_sampler_arrays_with_literal_indices_only() {
        let (s, map) = expand_sampler_arrays(
            "uniform sampler2D a[ 2 ];\nvoid f(){ texture(a[ 1 ], vec2(0)); }\n",
        )
        .unwrap();
        assert!(s.contains("uniform sampler2D a_pm0;") && s.contains("texture(a_pm1, vec2(0))"));
        assert_eq!(map.len(), 2);
        assert!(
            expand_sampler_arrays(
                "uniform sampler2D a[ 2 ];\nvoid f(int i){ texture(a[ i ], vec2(0)); }\n"
            )
            .is_err()
        );
    }
}
