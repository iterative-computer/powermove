//! S3-style GLSL pipeline for three.js programs (plan section 11, "The pipeline"):
//!
//! 1. Normalize `#version 300 es` to `#version 450`, drop default-precision statements.
//! 2. Preprocess with glslang, then expand sampler arrays (three.js unrolls its loops, so
//!    indices are literals) into one sampler per element.
//! 3. Wrap the vertex `main` with the clip-space conventions of the replay target
//!    (see [`VERTEX_EPILOGUE`]).
//! 4. Compile both stages as one linked glslang program with relaxed Vulkan rules
//!    (loose uniforms go to `gl_DefaultUniformBlock`) and automatic bindings/locations.
//! 5. Inline functions with opaque parameters (SPIRV-Tools) and split combined
//!    image-samplers into separate image and sampler variables at the SPIR-V level.
//! 6. Parse with naga's SPIR-V frontend and validate with derivative uniformity off.

use anyhow::{Context, Result, anyhow, bail};
use std::collections::{BTreeMap, HashMap};
use std::ffi::{CStr, CString};

pub use naga;

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
        Ok(Self { _compiler: glslang::Compiler::acquire().ok_or_else(|| anyhow!("glslang initialization failed"))? })
    }

    fn input(stage: Stage, code: &CStr) -> glslang_sys::glslang_input_t {
        let limits: &glslang::limits::ResourceLimits = &glslang::limits::DEFAULT_LIMITS;
        // SAFETY: ResourceLimits is #[repr(transparent)] over glslang_resource_t.
        let resource = limits as *const glslang::limits::ResourceLimits as *const glslang_sys::glslang_resource_t;
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
            callbacks: glslang_sys::glsl_include_callbacks_s { include_system: None, include_local: None, free_include_result: None },
            callbacks_ctx: std::ptr::null_mut(),
        }
    }

    fn log(shader: *mut glslang_sys::glslang_shader_t) -> String {
        unsafe { CStr::from_ptr(glslang_sys::glslang_shader_get_info_log(shader)).to_string_lossy().into_owned() }
    }

    /// Runs only the preprocessor and returns the expanded source.
    fn preprocess(&self, stage: Stage, source: &str) -> Result<String> {
        let code = CString::new(source)?;
        let input = Self::input(stage, &code);
        unsafe {
            let shader = glslang_sys::glslang_shader_create(&input);
            let ok = glslang_sys::glslang_shader_preprocess(shader, &input);
            let result = if ok == 0 {
                Err(anyhow!("preprocess failed: {}", Self::log(shader)))
            } else {
                Ok(CStr::from_ptr(glslang_sys::glslang_shader_get_preprocessed_code(shader)).to_string_lossy().into_owned())
            };
            glslang_sys::glslang_shader_delete(shader);
            result
        }
    }

    /// Compiles and links both stages, returning SPIR-V words per stage.
    fn compile(&self, vertex: &str, fragment: &str) -> Result<(Vec<u32>, Vec<u32>)> {
        let codes = [CString::new(vertex)?, CString::new(fragment)?];
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
                glslang_sys::glslang_shader_set_default_uniform_block_set_and_binding(shader, 0, DEFAULT_BLOCK_BINDING);
                let name = CString::new("gl_DefaultUniformBlock").unwrap();
                glslang_sys::glslang_shader_set_default_uniform_block_name(shader, name.as_ptr());
                shaders.push(shader);
                if glslang_sys::glslang_shader_preprocess(shader, input) == 0 || glslang_sys::glslang_shader_parse(shader, input) == 0 {
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
                    bail!("glslang link failed: {}", CStr::from_ptr(glslang_sys::glslang_program_get_info_log(program)).to_string_lossy());
                }
                if glslang_sys::glslang_program_map_io(program) == 0 {
                    bail!("glslang map_io failed: {}", CStr::from_ptr(glslang_sys::glslang_program_get_info_log(program)).to_string_lossy());
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
                if real.contains("sampler") {
                    if let Some((name, n)) = parse_array_decl(d2) {
                        arrays.push((real.to_string(), name, n));
                        continue;
                    }
                }
                ty
            } else {
                ty
            };
            if ty.contains("sampler") {
                if let Some((name, n)) = parse_array_decl(decl) {
                    arrays.push((ty.to_string(), name, n));
                    continue;
                }
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
                let close = after.find(']').ok_or_else(|| anyhow!("unterminated index for {name}"))?;
                let index_text = after[..close].trim();
                let k: u32 = index_text.parse().map_err(|_| anyhow!("non-literal index `{index_text}` into sampler array {name}"))?;
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
        let ctor = format!("mat{cols}({})", (0..*cols).map(|k| format!("{name}_pmc{k}")).collect::<Vec<_>>().join(", "));
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
        bail!("vertex shader has no main()");
    }
    out.push_str(VERTEX_EPILOGUE);
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
                sampled_image_types.insert(inst.result_id.unwrap(), inst.operands[0].unwrap_id_ref());
            }
            Op::TypeSampler => sampler_type = inst.result_id,
            Op::TypePointer => {
                let storage = match inst.operands[0] {
                    Operand::StorageClass(s) => s as u32,
                    _ => continue,
                };
                ptr_types.insert((storage, inst.operands[1].unwrap_id_ref()), inst.result_id.unwrap());
            }
            Op::TypeArray | Op::TypeRuntimeArray => {
                if sampled_image_types.contains_key(&inst.operands[0].unwrap_id_ref()) {
                    bail!("arrays of combined samplers are not supported (expand them in GLSL)");
                }
            }
            _ => {}
        }
    }
    for ((storage, pointee), ptr) in &ptr_types {
        if *storage == StorageClass::UniformConstant as u32 && sampled_image_types.contains_key(pointee) {
            ptr_to_si.insert(*ptr, *pointee);
        }
    }
    if ptr_to_si.is_empty() {
        return Ok(words.to_vec());
    }
    // Combined variables.
    let mut combined: BTreeMap<u32, (u32, u32)> = BTreeMap::new(); // var -> (si, image)
    for inst in &module.types_global_values {
        if inst.class.opcode == Op::Variable {
            if let Some(si) = ptr_to_si.get(&inst.result_type.unwrap()) {
                combined.insert(inst.result_id.unwrap(), (*si, sampled_image_types[si]));
            }
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
    let sampler_ptr = ptr_types.get(&(uc, sampler_ty)).copied().unwrap_or_else(&mut fresh);
    let mut image_ptrs: HashMap<u32, u32> = HashMap::new();
    for (_, image) in combined.values() {
        if !image_ptrs.contains_key(image) {
            let id = ptr_types.get(&(uc, *image)).copied().unwrap_or_else(&mut fresh);
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
        if opcode == Op::Variable {
            if let Some((_, image)) = combined.get(&id.unwrap()) {
                if !emitted_sampler {
                    if sampler_type.is_none() {
                        types.push(Instruction::new(Op::TypeSampler, None, Some(sampler_ty), vec![]));
                    }
                    types.push(Instruction::new(Op::TypePointer, None, Some(sampler_ptr), vec![Operand::StorageClass(StorageClass::UniformConstant), Operand::IdRef(sampler_ty)]));
                    emitted_sampler = true;
                }
                inst.result_type = Some(image_ptrs[image]);
                let var = id.unwrap();
                types.push(inst);
                types.push(Instruction::new(Op::Variable, Some(sampler_ptr), Some(sampler_vars[&var]), vec![Operand::StorageClass(StorageClass::UniformConstant)]));
                continue;
            }
        }
        types.push(inst);
        if let Some(id) = id {
            if let Some(ptr) = image_ptrs.get(&id) {
                if !existing_ptrs.contains(ptr) {
                    types.push(Instruction::new(Op::TypePointer, None, Some(*ptr), vec![Operand::StorageClass(StorageClass::UniformConstant), Operand::IdRef(id)]));
                }
            }
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
                Operand::Decoration(Decoration::DescriptorSet) => extra_annotations.push(Instruction::new(Op::Decorate, None, None, vec![Operand::IdRef(*svar), Operand::Decoration(Decoration::DescriptorSet), inst.operands[2].clone()])),
                Operand::Decoration(Decoration::Binding) => {
                    let binding = match inst.operands[2] {
                        Operand::LiteralBit32(b) => b,
                        _ => bail!("unexpected binding operand"),
                    };
                    extra_annotations.push(Instruction::new(Op::Decorate, None, None, vec![Operand::IdRef(*svar), Operand::Decoration(Decoration::Binding), Operand::LiteralBit32(binding + SAMPLER_BINDING_OFFSET)]));
                }
                _ => {}
            }
        }
    }
    module.annotations.extend(extra_annotations);
    let mut names = Vec::new();
    for inst in &module.debug_names {
        if inst.class.opcode == Op::Name {
            if let Some(svar) = sampler_vars.get(&inst.operands[0].unwrap_id_ref()) {
                if let Operand::LiteralString(s) = &inst.operands[1] {
                    names.push(Instruction::new(Op::Name, None, None, vec![Operand::IdRef(*svar), Operand::LiteralString(format!("{s}_pmsampler"))]));
                }
            }
        }
    }
    module.debug_names.extend(names);
    // Entry point interfaces (SPIR-V 1.4+ lists every global; harmless otherwise).
    for ep in &mut module.entry_points {
        let mut add = Vec::new();
        for op in ep.operands.iter().skip(3) {
            if let Operand::IdRef(id) = op {
                if let Some(s) = sampler_vars.get(id) {
                    add.push(Operand::IdRef(*s));
                }
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
                        rewritten.push(Instruction::new(Op::Load, Some(*image), Some(image_value), vec![Operand::IdRef(pointer)]));
                        rewritten.push(Instruction::new(Op::Load, Some(sampler_ty), Some(sampler_value), vec![Operand::IdRef(sampler_vars[&pointer])]));
                        rewritten.push(Instruction::new(Op::SampledImage, Some(*si), inst.result_id, vec![Operand::IdRef(image_value), Operand::IdRef(sampler_value)]));
                        continue;
                    }
                }
                if inst.operands.iter().any(|o| matches!(o, Operand::IdRef(id) if combined.contains_key(id))) && inst.class.opcode != Op::Load {
                    bail!("combined sampler variable used by {:?} (only loads are supported)", inst.class.opcode);
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

fn inline_opaque(words: &[u32]) -> Result<Vec<u32>> {
    use spirv_tools::opt::Optimizer;
    let mut opt = spirv_tools::opt::create(Some(spirv_tools::TargetEnv::Vulkan_1_1));
    opt.register_pass(spirv_tools::opt::Passes::InlineOpaque);
    opt.register_pass(spirv_tools::opt::Passes::EliminateDeadFunctions);
    let mut messages = Vec::new();
    let out = opt
        .optimize(words, &mut |m: spirv_tools::error::Message| messages.push(format!("{m:?}")), None)
        .map_err(|e| anyhow!("SPIRV-Tools inline failed: {e:?} {messages:?}"))?;
    Ok(out.as_words().to_vec())
}

fn to_naga(words: &[u32]) -> Result<(naga::Module, naga::valid::ModuleInfo)> {
    let options = naga::front::spv::Options { adjust_coordinate_space: false, strict_capabilities: false, block_ctx_dump_prefix: None };
    let module = naga::front::spv::Frontend::new(words.iter().cloned(), &options).parse().map_err(|e| anyhow!("naga spv-in: {e:?}"))?;
    let flags = naga::valid::ValidationFlags::all() ^ naga::valid::ValidationFlags::CONTROL_FLOW_UNIFORMITY;
    let info = naga::valid::Validator::new(flags, naga::valid::Capabilities::all())
        .validate(&module)
        .map_err(|e| anyhow!("naga validation: {}", e.emit_to_string("")))?;
    Ok((module, info))
}

/// Compiles a three.js program (vertex and fragment sources exactly as WebGL received them).
pub fn compile(vertex_src: &str, fragment_src: &str) -> Result<Program> {
    let glslang = Glslang::get()?;
    let vs = glslang.preprocess(Stage::Vertex, &normalize(vertex_src)).context("vertex preprocess")?;
    let fs = glslang.preprocess(Stage::Fragment, &normalize(fragment_src)).context("fragment preprocess")?;
    let (vs, vs_arrays) = expand_sampler_arrays(&vs)?;
    let (fs, fs_arrays) = expand_sampler_arrays(&fs)?;
    let vs = split_matrix_attributes(&vs)?;
    let vs = wrap_vertex_main(&vs)?;
    let (vs_spv, fs_spv) = glslang.compile(&vs, &fs)?;
    let vs_spv = split_combined_samplers(&inline_opaque(&vs_spv).context("vertex inline")?).context("vertex split")?;
    let fs_spv = split_combined_samplers(&inline_opaque(&fs_spv).context("fragment inline")?).context("fragment split")?;
    let (vertex, vertex_info) = to_naga(&vs_spv).context("vertex")?;
    let (fragment, fragment_info) = to_naga(&fs_spv).context("fragment")?;
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
    Ok(program)
}

fn reflect(program: &mut Program, stage: Stage, array_names: &HashMap<String, String>) -> Result<()> {
    let module = match stage {
        Stage::Vertex => &program.vertex,
        Stage::Fragment => &program.fragment,
    };
    let mut images = Vec::new();
    for (_, var) in module.global_variables.iter() {
        let Some(binding) = &var.binding else { continue };
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
                    flatten(module, &name, member.ty, member.offset, &mut program.uniforms);
                }
            }
            (naga::TypeInner::Image { dim, arrayed, class }, _) => {
                let raw = var.name.clone().unwrap_or_default();
                let gl = array_names.get(&raw).cloned().unwrap_or(raw);
                images.push((gl, binding.binding, *dim, *arrayed, *class));
            }
            _ => {}
        }
    }
    for (gl, binding, dim, arrayed, class) in images {
        let slot = program.textures.entry(gl).or_insert(TextureSlot { binding, sampler_binding: binding + SAMPLER_BINDING_OFFSET, dim, arrayed, class, stages: Vec::new() });
        if slot.binding != binding {
            bail!("texture binding differs between stages");
        }
        slot.stages.push(stage);
    }
    let entry = module.entry_points.first().ok_or_else(|| anyhow!("no entry point"))?;
    for arg in &entry.function.arguments {
        if let Some(naga::Binding::Location { location, .. }) = &arg.binding {
            if stage == Stage::Vertex {
                let name = arg.name.clone().unwrap_or_default();
                program.attributes.insert(name.clone(), attribute(&name, *location, module.types[arg.ty].inner.clone()));
            }
        }
        if arg.binding.is_none() {
            if let naga::TypeInner::Struct { members, .. } = &module.types[arg.ty].inner {
                for m in members {
                    if let Some(naga::Binding::Location { location, .. }) = &m.binding {
                        if stage == Stage::Vertex {
                            let name = m.name.clone().unwrap_or_default();
                            program.attributes.insert(name.clone(), attribute(&name, *location, module.types[m.ty].inner.clone()));
                        }
                    }
                }
            }
        }
    }
    if stage == Stage::Fragment {
        if let Some(result) = &entry.function.result {
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
    }
    Ok(())
}

fn attribute(name: &str, location: u32, ty: naga::TypeInner) -> Attribute {
    if let Some(pos) = name.rfind("_pmc") {
        if let Ok(column) = name[pos + 4..].parse::<u32>() {
            return Attribute { location, ty, gl_name: name[..pos].to_string(), column };
        }
    }
    Attribute { location, ty, gl_name: name.to_string(), column: 0 }
}

fn flatten(module: &naga::Module, name: &str, ty: naga::Handle<naga::Type>, offset: u32, out: &mut HashMap<String, UniformSlot>) {
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
            out.insert(name.to_string(), UniformSlot { offset, kind: scalar_kind(*s), columns: 1, rows: 1, column_stride: 0, count: 1, array_stride: 0 });
        }
        T::Vector { size, scalar } => {
            out.insert(name.to_string(), UniformSlot { offset, kind: scalar_kind(*scalar), columns: 1, rows: *size as u32, column_stride: 0, count: 1, array_stride: 0 });
        }
        T::Matrix { columns, rows, scalar } => {
            let stride = if *rows as u32 == 2 { 8 } else { 16 };
            out.insert(name.to_string(), UniformSlot { offset, kind: scalar_kind(*scalar), columns: *columns as u32, rows: *rows as u32, column_stride: stride, count: 1, array_stride: 0 });
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
            let base_is_leaf = !matches!(module.types[*base].inner, T::Struct { .. } | T::Array { .. });
            for i in 0..n {
                flatten(module, &format!("{name}[{i}]"), *base, offset + i * stride, out);
                if base_is_leaf {
                    if let Some(slot) = out.get_mut(&format!("{name}[{i}]")) {
                        slot.count = n - i;
                        slot.array_stride = *stride;
                    }
                }
            }
            if base_is_leaf {
                if let Some(first) = out.get(&format!("{name}[0]")).cloned() {
                    out.insert(name.to_string(), first);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VS: &str = "#version 300 es\nprecision highp float;\nin vec3 position;\nin vec2 uv;\nuniform mat4 projectionMatrix;\nuniform mat4 modelViewMatrix;\nout vec2 vUv;\nvoid main() { vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }\n";
    const FS: &str = "#version 300 es\nprecision highp float;\nprecision highp sampler2D;\n#define N 2\nuniform sampler2D maps[ N ];\nuniform sampler2D single;\nuniform vec3 tint;\nuniform bool flip;\nstruct L { vec3 dir; float k; };\nuniform L lights[ 3 ];\nin vec2 vUv;\nlayout(location = 0) out highp vec4 pc_fragColor;\nvec4 get( sampler2D s, vec2 uv ) { return texture( s, uv ); }\nvoid main() { vec4 c = get( maps[ 0 ], vUv ) + get( maps[ 1 ], vUv ) + texture( single, vUv ) * float( textureSize( single, 0 ).x ); if ( flip ) c.rgb = 1.0 - c.rgb; pc_fragColor = vec4( c.rgb * tint * lights[ 2 ].dir * lights[ 1 ].k, c.a ); }\n";

    #[test]
    fn compiles_and_reflects() {
        let p = compile(VS, FS).expect("compile");
        assert!(p.uniforms.contains_key("projectionMatrix"));
        assert!(p.uniforms.contains_key("tint"));
        assert!(p.uniforms.contains_key("lights[2].dir"));
        assert_eq!(p.uniforms["flip"].kind, ScalarKind::Uint);
        assert!(p.textures.contains_key("maps[0]") && p.textures.contains_key("maps[1]") && p.textures.contains_key("single"));
        assert_eq!(p.attributes.len(), 2);
        assert_eq!(p.outputs, vec![0]);
        // Uniform block shared by both stages.
        assert_eq!(p.block_stages.len(), 2);
    }

    #[test]
    fn splits_matrix_attributes() {
        let vs = "#version 300 es\nin vec3 position;\nin mat4 instanceMatrix;\nuniform mat4 projectionMatrix;\nvoid main() { gl_Position = projectionMatrix * instanceMatrix * vec4(position, 1.0); }\n";
        let fs = "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(1.0); }\n";
        let p = compile(vs, fs).expect("compile");
        let cols: Vec<_> = p.attributes.values().filter(|a| a.gl_name == "instanceMatrix").map(|a| a.column).collect();
        assert_eq!(cols.len(), 4);
    }

    #[test]
    fn expands_sampler_arrays_with_literal_indices_only() {
        let (s, map) = expand_sampler_arrays("uniform sampler2D a[ 2 ];\nvoid f(){ texture(a[ 1 ], vec2(0)); }\n").unwrap();
        assert!(s.contains("uniform sampler2D a_pm0;") && s.contains("texture(a_pm1, vec2(0))"));
        assert_eq!(map.len(), 2);
        assert!(expand_sampler_arrays("uniform sampler2D a[ 2 ];\nvoid f(int i){ texture(a[ i ], vec2(0)); }\n").is_err());
    }
}
