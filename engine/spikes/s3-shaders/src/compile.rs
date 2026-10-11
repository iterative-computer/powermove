//! glslang front end: GLSL 450 (normalized from GLSL ES 3.00) to SPIR-V with
//! relaxed Vulkan rules and automatic bindings and locations (plan section 11,
//! pipeline step 3).
//!
//! The `glslang` crate parses inside `Shader::new`, before shader options can be
//! set, and relaxed Vulkan rules must be in place before parsing (they decide
//! whether loose `uniform float x;` declarations are legal). So this module
//! drives the glslang C interface from `glslang-sys` directly.

use std::ffi::{CStr, CString};
use std::ptr::NonNull;

use glslang_sys as sys;

/// Descriptor set and binding of the default uniform block (`gl_DefaultUniformBlock`)
/// per stage. Fragment resources live in set 0, vertex resources in set 1.
pub const FRAGMENT_SET: u32 = 0;
pub const VERTEX_SET: u32 = 1;
pub const DEFAULT_BLOCK_BINDING: u32 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Vertex,
    Fragment,
}

#[derive(Debug, Clone)]
pub struct CompileError {
    /// `parse`, `link`, or `map_io`.
    pub phase: &'static str,
    pub log: String,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "glslang {} failed: {}", self.phase, self.log.trim())
    }
}

pub struct Spirv {
    pub words: Vec<u32>,
    /// glslang's preprocessed source, used to recover declared GLSL types
    /// (relaxed rules lower `uniform bool` to a `uint` block member).
    pub preprocessed: String,
    pub warnings: String,
}

fn ensure_process() {
    // `Compiler::acquire` initializes the glslang process exactly once and keeps
    // it alive for the program's lifetime.
    let _ = glslang::Compiler::acquire().expect("glslang process initialization");
}

fn resource() -> &'static sys::glslang_resource_t {
    static LIMITS: glslang::limits::ResourceLimits = glslang::limits::DEFAULT_LIMITS;
    // SAFETY: `ResourceLimits` is `#[repr(transparent)]` over `glslang_resource_t`.
    unsafe { &*(&LIMITS as *const glslang::limits::ResourceLimits as *const sys::glslang_resource_t) }
}

struct ShaderHandle(NonNull<sys::glslang_shader_t>);
impl Drop for ShaderHandle {
    fn drop(&mut self) {
        unsafe { sys::glslang_shader_delete(self.0.as_ptr()) }
    }
}
struct ProgramHandle(NonNull<sys::glslang_program_t>);
impl Drop for ProgramHandle {
    fn drop(&mut self) {
        unsafe { sys::glslang_program_delete(self.0.as_ptr()) }
    }
}

fn cstr(ptr: *const std::os::raw::c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned()
}

/// Compile normalized `#version 450` GLSL to SPIR-V 1.0 for Vulkan 1.0.
pub fn compile(source: &str, stage: Stage) -> Result<Spirv, CompileError> {
    ensure_process();
    let code = CString::new(source).map_err(|_| CompileError { phase: "parse", log: "source contains a NUL byte".into() })?;
    let messages = sys::glslang_messages_t::DEFAULT
        | sys::glslang_messages_t::SPV_RULES
        | sys::glslang_messages_t::VULKAN_RULES;
    let input = sys::glslang_input_t {
        language: sys::glslang_source_t::GLSL,
        stage: match stage {
            Stage::Vertex => sys::glslang_stage_t::Vertex,
            Stage::Fragment => sys::glslang_stage_t::Fragment,
        },
        client: sys::glslang_client_t::Vulkan,
        client_version: sys::glslang_target_client_version_t::Vulkan1_0,
        target_language: sys::glslang_target_language_t::SPIRV,
        target_language_version: sys::glslang_target_language_version_t::SPIRV1_0,
        code: code.as_ptr(),
        default_version: 450,
        default_profile: sys::glslang_profile_t::None,
        force_default_version_and_profile: 0,
        forward_compatible: 0,
        messages,
        resource: resource(),
        callbacks: sys::glsl_include_callbacks_t { include_system: None, include_local: None, free_include_result: None },
        callbacks_ctx: std::ptr::null_mut(),
    };
    let shader = ShaderHandle(NonNull::new(unsafe { sys::glslang_shader_create(&input) }).expect("glslang_shader_create"));
    let set = match stage {
        Stage::Vertex => VERTEX_SET,
        Stage::Fragment => FRAGMENT_SET,
    };
    unsafe {
        sys::glslang_shader_set_options(
            shader.0.as_ptr(),
            (sys::glslang_shader_options_t::AUTO_MAP_BINDINGS
                | sys::glslang_shader_options_t::AUTO_MAP_LOCATIONS
                | sys::glslang_shader_options_t::VULKAN_RULES_RELAXED)
                .0,
        );
        sys::glslang_shader_set_default_uniform_block_set_and_binding(shader.0.as_ptr(), set, DEFAULT_BLOCK_BINDING);
        // Every other resource (the samplers) goes to the stage's set too.
        sys::glslang_shader_set_resource_set_binding(shader.0.as_ptr(), [CString::new(set.to_string()).unwrap().as_ptr()].as_ptr(), 1);
    }
    if unsafe { sys::glslang_shader_preprocess(shader.0.as_ptr(), &input) } == 0 {
        return Err(CompileError { phase: "parse", log: cstr(unsafe { sys::glslang_shader_get_info_log(shader.0.as_ptr()) }) });
    }
    let preprocessed = cstr(unsafe { sys::glslang_shader_get_preprocessed_code(shader.0.as_ptr()) });
    if unsafe { sys::glslang_shader_parse(shader.0.as_ptr(), &input) } == 0 {
        return Err(CompileError { phase: "parse", log: cstr(unsafe { sys::glslang_shader_get_info_log(shader.0.as_ptr()) }) });
    }
    let warnings = cstr(unsafe { sys::glslang_shader_get_info_log(shader.0.as_ptr()) });
    let program = ProgramHandle(NonNull::new(unsafe { sys::glslang_program_create() }).expect("glslang_program_create"));
    unsafe { sys::glslang_program_add_shader(program.0.as_ptr(), shader.0.as_ptr()) };
    if unsafe { sys::glslang_program_link(program.0.as_ptr(), messages.0) } == 0 {
        return Err(CompileError { phase: "link", log: cstr(unsafe { sys::glslang_program_get_info_log(program.0.as_ptr()) }) });
    }
    if unsafe { sys::glslang_program_map_io(program.0.as_ptr()) } == 0 {
        return Err(CompileError { phase: "map_io", log: cstr(unsafe { sys::glslang_program_get_info_log(program.0.as_ptr()) }) });
    }
    unsafe { sys::glslang_program_SPIRV_generate(program.0.as_ptr(), input.stage) };
    let size = unsafe { sys::glslang_program_SPIRV_get_size(program.0.as_ptr()) };
    let mut words = vec![0u32; size];
    unsafe { sys::glslang_program_SPIRV_get(program.0.as_ptr(), words.as_mut_ptr()) };
    let spv_messages = cstr(unsafe { sys::glslang_program_SPIRV_get_messages(program.0.as_ptr()) });
    if words.is_empty() {
        return Err(CompileError { phase: "link", log: format!("glslang produced no SPIR-V. {spv_messages}") });
    }
    Ok(Spirv { words, preprocessed, warnings: format!("{warnings}{spv_messages}") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loose_uniforms_compile_under_relaxed_rules() {
        let spv = compile(
            "#version 450\nuniform float u_amount; uniform sampler2D u_tex; layout(location=0) in vec2 v_st; layout(location=0) out vec4 o;\nvoid main(){ o = texture(u_tex, v_st) * u_amount; }\n",
            Stage::Fragment,
        )
        .expect("compiles");
        assert_eq!(spv.words[0], 0x0723_0203);
    }

    #[test]
    fn locations_are_assigned_automatically() {
        compile("#version 450\nin vec2 v_st; out vec4 o; void main(){ o = vec4(v_st, 0.0, 1.0); }\n", Stage::Fragment).expect("auto locations");
    }

    #[test]
    fn errors_carry_the_glslang_log() {
        let error = compile("#version 450\nout vec4 o; void main(){ o = vec4(undefinedThing); }\n", Stage::Fragment).err().expect("fails");
        assert_eq!(error.phase, "parse");
        assert!(error.log.contains("undefinedThing"), "{}", error.log);
    }
}
