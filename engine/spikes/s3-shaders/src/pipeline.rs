//! The section 11 translation pipeline, both back ends:
//!
//! Both paths first validate the unmodified GLSL ES 3.00 program with the
//! compositor's vertex shader, as WebGL's compile and link would.
//!
//! * `Path::Naga`: normalize, glslang (relaxed Vulkan rules), varying remap,
//!   combined-sampler split, naga SPIR-V front end, naga validation with the
//!   derivative-uniformity diagnostic off, then wgpu (`ShaderSource::Naga`,
//!   naga's MSL back end).
//! * `Path::SpirvCross`: the same SPIR-V up to the varying remap, then
//!   SPIRV-Cross to MSL (combined samplers are native in MSL), then a wgpu
//!   passthrough shader.

use std::collections::HashMap;
use std::time::Instant;

use crate::compile::{self, Stage};
use crate::errors::{self, Diagnostic};
use crate::limits;
use crate::normalize;
use crate::reflect::{self, Reflection};
use crate::rewrite::{self, SplitBinding};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Path {
    #[serde(rename = "naga")]
    Naga,
    #[serde(rename = "spvc")]
    SpirvCross,
}

impl Path {
    pub fn name(self) -> &'static str {
        match self {
            Path::Naga => "naga",
            Path::SpirvCross => "spvc",
        }
    }
}

pub const VARYINGS: &[(&str, u32)] = &[("v_uv", 0), ("v_st", 1), ("v_px", 2)];

#[derive(Debug, Clone, serde::Serialize)]
pub struct Failure {
    /// es-validate, normalize, glslang, rewrite, naga-parse, naga-validate, spvc, metal, render
    pub stage: String,
    pub message: String,
    pub diagnostics: Vec<Diagnostic>,
}

impl Failure {
    pub fn new(stage: &str, message: impl Into<String>) -> Self {
        Failure { stage: stage.into(), message: message.into(), diagnostics: Vec::new() }
    }
}

pub struct StageOutput {
    pub reflection: Reflection,
    pub splits: Vec<SplitBinding>,
    pub naga: Option<naga::Module>,
    pub msl: Option<String>,
    pub entry_point: String,
    /// Non-finite constants rebuilt at run time for naga (see `rewrite`).
    pub materialized: usize,
    /// naga-only SPIR-V rewrites that changed this stage (`rewrite::naga_compat`).
    pub notes: Vec<String>,
}

pub struct Translated {
    pub vertex: StageOutput,
    pub fragment: StageOutput,
    pub renamed: Vec<String>,
    pub polyfilled: Vec<String>,
    /// glslang warnings for the fragment stage.
    pub warnings: String,
    pub timings_ms: HashMap<String, f64>,
}

/// `S3_TRACE=1` prints each stage as it starts (triage for hangs).
fn trace(stage: &str) {
    if std::env::var_os("S3_TRACE").is_some() {
        eprintln!("[s3] {stage}");
    }
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

/// GL `PM.VERT` with the quad from `gl_VertexIndex` instead of a vertex buffer
/// (same values as the compositor's `[0,0, 1,0, 0,1, 1,1]` triangle strip).
pub fn wgpu_vertex_source(gl_vertex: &str) -> String {
    gl_vertex.replacen(
        "in vec2 a_pos;",
        "vec2 pm_quad(){ return vec2(float(gl_VertexIndex & 1), float(gl_VertexIndex >> 1)); }\n#define a_pos pm_quad()",
        1,
    )
}

fn derivative_uniformity_off(module: &mut naga::Module) {
    use naga::diagnostic_filter::{DiagnosticFilter, DiagnosticFilterNode, FilterableTriggeringRule, Severity, StandardFilterableTriggeringRule};
    let node = module.diagnostic_filters.append(
        DiagnosticFilterNode {
            inner: DiagnosticFilter {
                new_severity: Severity::Off,
                triggering_rule: FilterableTriggeringRule::Standard(StandardFilterableTriggeringRule::DerivativeUniformity),
            },
            parent: module.diagnostic_filter_leaf,
        },
        naga::Span::UNDEFINED,
    );
    module.diagnostic_filter_leaf = Some(node);
}

pub fn naga_validate(module: &naga::Module) -> Result<naga::valid::ModuleInfo, String> {
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(module)
        .map_err(|e| format!("{:?}", e.into_inner()))
}

/// glslang, then the rewrites both paths share. Returns notes on rewrites
/// that changed the program (for the report).
fn stage_spirv(source: &str, stage: Stage, line_offset: u32, timings: &mut HashMap<String, f64>, prefix: &str) -> Result<(rspirv::dr::Module, compile::Spirv, Vec<String>), Failure> {
    trace(&format!("{prefix}glslang"));
    let start = Instant::now();
    let spv = compile::compile(source, stage).map_err(|e| Failure {
        stage: "glslang".into(),
        message: e.to_string(),
        diagnostics: errors::map(&e.log, line_offset),
    })?;
    timings.insert(format!("{prefix}glslang"), ms(start));
    let mut module = rewrite::load(&spv.words).map_err(|e| Failure::new("rewrite", e.0))?;
    let mut notes = Vec::new();
    if stage == Stage::Fragment {
        let arrays = rewrite::scalarize_sampler_arrays(&mut module).map_err(|e| Failure::new("rewrite", e.0))?;
        if arrays > 0 {
            notes.push(format!("sampler2D arrays ({arrays})"));
        }
    }
    rewrite::assign_bindings(&mut module, compile::DEFAULT_BLOCK_BINDING);
    if stage == Stage::Fragment {
        rewrite::remap_varyings(&mut module, VARYINGS);
        let names: Vec<&str> = VARYINGS.iter().map(|(n, _)| *n).collect();
        notes.extend(rewrite::fragment_interface(&mut module, &names));
    }
    let remainders = rewrite::signed_remainder(&mut module);
    if remainders > 0 {
        notes.push(format!("signed % as remainder ({remainders})"));
    }
    Ok((module, spv, notes))
}

fn naga_stage(mut module: rspirv::dr::Module, spv: &compile::Spirv, timings: &mut HashMap<String, f64>, prefix: &str) -> Result<StageOutput, Failure> {
    let compat = rewrite::naga_compat(&mut module);
    let reflection = reflect::reflect(&module, &reflect::bool_uniforms(&spv.preprocessed));
    let materialized = rewrite::materialize_non_finite_constants(&mut module);
    let start = Instant::now();
    let splits = rewrite::split_combined_samplers(&mut module).map_err(|e| Failure::new("rewrite", e.0))?;
    timings.insert(format!("{prefix}split"), ms(start));
    let words = rewrite::assemble(&module);
    trace(&format!("{prefix}naga-parse"));
    let start = Instant::now();
    let options = naga::front::spv::Options { adjust_coordinate_space: true, strict_capabilities: false, block_ctx_dump_prefix: None };
    let parsed = std::panic::catch_unwind(|| naga::front::spv::Frontend::new(words.into_iter(), &options).parse());
    let mut naga_module = match parsed {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => return Err(Failure::new("naga-parse", format!("{e:?}"))),
        Err(_) => return Err(Failure::new("naga-parse", "naga SPIR-V front end panicked")),
    };
    timings.insert(format!("{prefix}naga-parse"), ms(start));
    // `S3_UNIFORMITY=on` keeps naga's derivative-uniformity analysis, to
    // measure how many WebGL-valid shaders step 5's filter is needed for.
    if std::env::var("S3_UNIFORMITY").as_deref() != Ok("on") {
        derivative_uniformity_off(&mut naga_module);
    }
    let start = Instant::now();
    naga_validate(&naga_module).map_err(|e| Failure::new("naga-validate", e))?;
    timings.insert(format!("{prefix}naga-validate"), ms(start));
    // `S3_NONCE` makes the generated MSL unique so the Metal compiler cache
    // misses (cold pipeline timings); it renames the entry point only.
    let mut entry_point = "main".to_string();
    if let Ok(nonce) = std::env::var("S3_NONCE") {
        entry_point = format!("main_{nonce}");
        naga_module.entry_points[0].name = entry_point.clone();
    }
    Ok(StageOutput { reflection, splits, naga: Some(naga_module), msl: None, entry_point, materialized, notes: compat })
}

/// Metal resource indices as wgpu-hal assigns them for our layouts (see
/// `render::layout_for`): per stage, buffers, textures, and samplers are
/// numbered in bind-group then binding order.
fn spvc_stage(module: rspirv::dr::Module, spv: &compile::Spirv, stage: Stage, timings: &mut HashMap<String, f64>, prefix: &str) -> Result<StageOutput, Failure> {
    use spirv_cross2::compile::msl::{BindTarget, CompilerOptions, MslVersion, ResourceBinding};
    use spirv_cross2::spirv::ExecutionModel;
    use spirv_cross2::{Compiler, Module, targets};

    let reflection = reflect::reflect(&module, &reflect::bool_uniforms(&spv.preprocessed));
    let words = rewrite::assemble(&module);
    let start = Instant::now();
    let fail = |e: spirv_cross2::SpirvCrossError| Failure::new("spvc", e.to_string());
    let mut compiler = Compiler::<targets::Msl>::new(Module::from_words(&words)).map_err(fail)?;
    let model = match stage {
        Stage::Vertex => ExecutionModel::Vertex,
        Stage::Fragment => ExecutionModel::Fragment,
    };
    if let Some(block) = &reflection.block {
        compiler
            .add_resource_binding(model, ResourceBinding::from_qualified(block.set, block.binding), &BindTarget { buffer: 0, texture: 0, sampler: 0, count: None })
            .map_err(fail)?;
    }
    let mut samplers = reflection.samplers.clone();
    samplers.sort_by_key(|s| s.binding);
    for (index, sampler) in samplers.iter().enumerate() {
        compiler
            .add_resource_binding(
                model,
                ResourceBinding::from_qualified(sampler.set, sampler.binding),
                &BindTarget { buffer: 0, texture: index as u32, sampler: index as u32, count: None },
            )
            .map_err(fail)?;
    }
    let mut options = CompilerOptions::default();
    options.version = MslVersion::new(3, 0, 0);
    // WebGL zero-initializes every variable (ANGLE's initialization of
    // uninitialized locals and globals); naga's MSL does too. Without this,
    // SPIRV-Cross reads garbage from unwritten private array elements
    // (coverage sweep, `types/global-arrays-private`).
    options.common.force_zero_initialized_variables = true;
    // The GL vertex shader writes GL clip space; glslang reads it with Vulkan's
    // y-down convention, so flip like naga's `adjust_coordinate_space` does.
    options.common.flip_vertex_y = true;
    let artifact = compiler.compile(&options).map_err(fail)?;
    // SPIRV-Cross renames `main` (reserved in MSL) while compiling, so the
    // cleansed name is only final on the compiled artifact.
    let entry = artifact
        .cleansed_entry_point_name("main", model)
        .map_err(fail)?
        .map(|s| s.to_string())
        .ok_or_else(|| Failure::new("spvc", "SPIRV-Cross reported no entry point"))?;
    timings.insert(format!("{prefix}spvc"), ms(start));
    let mut msl = artifact.to_string();
    if let Ok(pragma) = std::env::var("S3_MSL_PRAGMA") {
        msl = format!("{}\n{msl}", pragma.replace(';', "\n"));
    }
    if let Ok(nonce) = std::env::var("S3_NONCE") {
        msl = format!("// {nonce}\n{msl}");
    }
    Ok(StageOutput { reflection, splits: Vec::new(), naga: None, msl: Some(msl), entry_point: entry, materialized: 0, notes: Vec::new() })
}

fn limit_failure(error: &limits::LimitError, line_offset: u32) -> Failure {
    Failure { stage: "limits".into(), message: error.to_string(), diagnostics: errors::map(&error.to_string(), line_offset) }
}

pub fn translate(gl_vertex: &str, fragment: &str, line_offset: u32, path: Path) -> Result<Translated, Failure> {
    let mut timings = HashMap::new();
    trace("limits");
    let start = Instant::now();
    // `S3_LIMITS=off` skips the source budgets, only to measure what they prevent.
    if std::env::var("S3_LIMITS").as_deref() != Ok("off") {
        limits::check_source(fragment).map_err(|e| limit_failure(&e, line_offset))?;
    }
    timings.insert("limits".into(), ms(start));
    trace("es-validate");
    let start = Instant::now();
    compile::validate_es(gl_vertex, &normalize::rename_reserved(fragment)).map_err(|e| Failure {
        stage: "es-validate".into(),
        message: e.to_string(),
        diagnostics: errors::map(&e.log, line_offset),
    })?;
    timings.insert("es-validate".into(), ms(start));
    let start = Instant::now();
    let target = match path {
        Path::Naga => normalize::Target::Naga,
        Path::SpirvCross => normalize::Target::SpirvCross,
    };
    let normalized = normalize::normalize(fragment, target).map_err(|e| Failure::new("normalize", e.0))?;
    let vertex = normalize::normalize(&wgpu_vertex_source(gl_vertex), target).map_err(|e| Failure::new("normalize", format!("vertex: {}", e.0)))?;
    timings.insert("normalize".into(), ms(start));
    let (vmod, vspv, _) = stage_spirv(&vertex.source, Stage::Vertex, 0, &mut timings, "vertex-")?;
    let (fmod, fspv, notes) = stage_spirv(&normalized.source, Stage::Fragment, line_offset, &mut timings, "")?;
    let unbound = reflect::unbound_resources(&fmod);
    if !unbound.is_empty() {
        return Err(Failure::new(
            "bind",
            format!("the compositor binds only float sampler2D textures and the default uniform block; WebGL cannot draw this program as Powermove binds it: {}", unbound.join(", ")),
        ));
    }
    let start = Instant::now();
    let active: std::collections::HashSet<u32> = reflect::reflect(&fmod, &reflect::bool_uniforms(&fspv.preprocessed))
        .block
        .map(|b| b.members.iter().enumerate().filter(|(_, m)| m.active).map(|(k, _)| k as u32).collect())
        .unwrap_or_default();
    limits::check_spirv(&fmod, &active).map_err(|e| limit_failure(&e, line_offset))?;
    timings.insert("limits-spirv".into(), ms(start));
    let (vertex_out, fragment_out) = match path {
        Path::Naga => (naga_stage(vmod, &vspv, &mut timings, "vertex-")?, naga_stage(fmod, &fspv, &mut timings, "")?),
        Path::SpirvCross => (
            spvc_stage(vmod, &vspv, Stage::Vertex, &mut timings, "vertex-")?,
            spvc_stage(fmod, &fspv, Stage::Fragment, &mut timings, "")?,
        ),
    };
    let mut polyfilled = normalized.polyfilled;
    polyfilled.extend(notes);
    polyfilled.extend(fragment_out.notes.iter().cloned());
    if fragment_out.materialized > 0 {
        polyfilled.push(format!("non-finite constants ({})", fragment_out.materialized));
    }
    Ok(Translated { vertex: vertex_out, fragment: fragment_out, renamed: normalized.renamed, polyfilled, warnings: fspv.warnings.trim().to_string(), timings_ms: timings })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The compositor's vertex shader (`PM.VERT`).
    const VERT: &str = "#version 300 es\nprecision highp float;\nin vec2 a_pos;\nuniform mat3 u_m;\nuniform vec2 u_res;\nuniform vec2 u_pxOrigin;\nuniform vec4 u_uv;\nout vec2 v_uv;\nout vec2 v_st;\nout vec2 v_px;\nvoid main(){\n  v_uv = u_uv.xy + a_pos * u_uv.zw;\n  v_st = vec2(a_pos.x, 1.0 - a_pos.y);\n  vec3 p = u_m * vec3(a_pos, 1.0);\n  v_px = p.xy / p.z + u_pxOrigin;\n  gl_Position = vec4((p.x / u_res.x) * 2.0 - p.z, p.z - (p.y / u_res.y) * 2.0, 0.0, p.z);\n}\n";
    const H: &str = "#version 300 es\nprecision highp float;\nin vec2 v_uv;\nuniform sampler2D u_tex;\nuniform vec2 u_res;\nout vec4 o;\n";

    /// Translate on both paths and, for naga, emit MSL as wgpu would.
    fn both(body: &str) -> Vec<Translated> {
        let source = format!("{H}{body}");
        [Path::Naga, Path::SpirvCross]
            .into_iter()
            .map(|path| {
                let t = translate(VERT, &source, 6, path).unwrap_or_else(|f| panic!("{path:?}: {}: {}\n{source}", f.stage, f.message));
                if path == Path::Naga {
                    let module = t.fragment.naga.as_ref().unwrap();
                    let info = naga_validate(module).unwrap();
                    naga::back::msl::write_string(module, &info, &naga::back::msl::Options { lang_version: (3, 0), ..Default::default() }, &Default::default()).expect("msl");
                }
                t
            })
            .collect()
    }

    #[test]
    fn gentype_builtins_naga_lacks_are_polyfilled() {
        let t = both("void main(){ float w; vec3 w3; float f = modf(v_uv.x * 3.0, w); vec3 f3 = modf(vec3(v_uv, 1.5), w3);\n  float n = normalize(v_uv.x - 0.5) + faceforward(1.0, v_uv.y, -1.0) + reflect(v_uv.x, 0.6) + refract(0.5, -0.8, 0.66);\n  o = vec4(f + w, f3.x + w3.y, n, normalize(v_uv).x); }\n");
        for name in ["modf", "normalize", "faceforward", "reflect", "refract"] {
            assert!(t[0].polyfilled.iter().any(|p| p == name), "{name}: {:?}", t[0].polyfilled);
        }
        assert!(t[1].polyfilled.is_empty(), "SPIRV-Cross keeps the built-ins: {:?}", t[1].polyfilled);
    }

    #[test]
    fn integer_remainder_and_unsigned_negation() {
        let t = both("void main(){ int i = int(v_uv.x * 100.0) - 50; uint u = uint(i + 60); int r = i % 7; r %= 5; uint n = -u; o = vec4(float(r), float(n & 255u), float(i % 3), 1.0); }\n");
        assert!(t[0].polyfilled.iter().any(|p| p.starts_with("signed % as remainder")), "{:?}", t[0].polyfilled);
        assert!(t[0].polyfilled.iter().any(|p| p.starts_with("uint negation")), "{:?}", t[0].polyfilled);
    }

    #[test]
    fn sampler_arrays_become_one_sampler_per_element() {
        let t = both("uniform sampler2D u_extra[3];\nvec4 tap(sampler2D s, vec2 uv){ return texture(s, uv); }\nvoid main(){ const int K = 2; o = texture(u_extra[0], v_uv) + tap(u_extra[K], v_uv) + texture(u_tex, v_uv); }\n");
        for translated in &t {
            let mut names: Vec<&str> = translated.fragment.reflection.samplers.iter().map(|s| s.name.as_str()).collect();
            names.sort();
            assert_eq!(names, ["u_extra[0]", "u_extra[1]", "u_extra[2]", "u_tex"]);
            let mut bindings: Vec<u32> = translated.fragment.reflection.samplers.iter().map(|s| s.binding).collect();
            bindings.sort();
            bindings.dedup();
            assert_eq!(bindings.len(), 4, "distinct bindings");
        }
        assert_eq!(t[0].fragment.splits.len(), 4);
    }

    #[test]
    fn fragment_interface_follows_webgl() {
        // Narrow and array outputs, gl_FragDepth, and an unused input the
        // vertex shader does not write.
        for body in [
            "void main(){ o3 = texture(u_tex, v_uv).rgb; }\n",
            "void main(){ of = v_uv.x; }\n",
            "void main(){ oa[0] = vec4(v_uv, 0.0, 1.0); }\n",
        ] {
            let decl = if body.contains("o3") { "out vec3 o3;" } else if body.contains("of") { "out float of;" } else { "out vec4 oa[1];" };
            let source = format!("#version 300 es\nprecision highp float;\nin vec2 v_uv;\nuniform sampler2D u_tex;\n{decl}\n{body}");
            for path in [Path::Naga, Path::SpirvCross] {
                let t = translate(VERT, &source, 5, path).unwrap_or_else(|f| panic!("{path:?}: {}: {}", f.stage, f.message));
                assert!(t.polyfilled.iter().any(|p| p.contains("widened to vec4")), "{:?}", t.polyfilled);
            }
        }
        let t = both("centroid in vec2 nope;\nvoid main(){ gl_FragDepth = 0.25; o = texture(u_tex, v_uv); }\n");
        assert!(t[0].polyfilled.iter().any(|p| p.contains("gl_FragDepth")), "{:?}", t[0].polyfilled);
        assert!(t[0].polyfilled.iter().any(|p| p.contains("unused input `nope`")), "{:?}", t[0].polyfilled);
    }

    #[test]
    fn two_row_matrix_uniforms_and_struct_uniforms_reach_naga() {
        let t = both("struct P { float k; vec4 c; mat2 r; };\nuniform P st;\nuniform mat2 um2; uniform mat3x2 um32; uniform mat2 ums[2];\nvoid main(){ o = vec4(um2 * v_uv + um32 * vec3(v_uv, 1.0) + ums[1] * v_uv, st.k, 1.0) + st.c + vec4(st.r[0], 0.0, 0.0); }\n");
        let block = t[0].fragment.reflection.block.as_ref().unwrap();
        let st = block.members.iter().find(|m| m.name == "st").unwrap();
        assert_eq!(crate::reflect::size_of(&st.ty), 48, "std140 struct size from its members");
    }

    #[test]
    fn unbound_resources_are_refused_with_a_reason() {
        for decl in ["uniform highp sampler3D u_s;\nvoid main(){ o = texture(u_s, vec3(v_uv, 0.5)); }\n", "uniform highp isampler2D u_s;\nvoid main(){ o = vec4(texelFetch(u_s, ivec2(0), 0)); }\n", "uniform Params { vec4 c; };\nvoid main(){ o = c; }\n"] {
            for path in [Path::Naga, Path::SpirvCross] {
                let f = translate(VERT, &format!("{H}{decl}"), 6, path).err().expect("refused");
                assert_eq!(f.stage, "bind", "{}", f.message);
            }
        }
        // Declared but unused: inactive in GL, accepted.
        both("uniform highp sampler3D u_s;\nuniform Params { vec4 c; };\nvoid main(){ o = texture(u_tex, v_uv); }\n");
    }

    #[test]
    fn texel_fetch_offset_and_vulkan_keywords() {
        let t = both("#define texture2D texture\nfloat sampler(float x){ return x; }\nvoid main(){ vec4 textureCube = texelFetchOffset(u_tex, ivec2(gl_FragCoord.xy), 0, ivec2(2, -3)); o = textureCube + texture2D(u_tex, v_uv) * sampler(0.5); }\n");
        assert!(t[0].polyfilled.iter().any(|p| p.starts_with("texelFetchOffset")), "{:?}", t[0].polyfilled);
    }
}
