//! The section 11 translation pipeline (amendment 9: the naga path of
//! spike S3, `docs/engine/spikes/S3.md`, "Pipeline as built").
//!
//! 0. **Limits** on the author's program ([`crate::limits::check_source`]).
//! 1. **ES validation**: glslang's GLSL ES 3.00 front end compiles and links
//!    the unmodified program with the compositor's vertex shader, as WebGL's
//!    compile and link would.
//! 2. **Normalize** ([`crate::normalize`]): `#version 450`, precision removed,
//!    GLSL 4.50 keywords renamed, polyfills for built-ins naga lacks.
//! 3. **glslang** with relaxed Vulkan rules, automatic bindings and locations.
//! 4. **SPIR-V rewrites** ([`crate::rewrite`]): sampler arrays, bindings,
//!    varyings by name, the fragment interface, signed remainder, naga
//!    compatibility, non-finite constants, the combined-sampler split; then
//!    the SPIR-V limits and the bind check.
//! 5. **naga**: the SPIR-V front end with `adjust_coordinate_space` (legacy
//!    GLSL runs in GL row order, S3.md "Origin"), derivative uniformity off,
//!    validation. wgpu takes the module as `ShaderSource::Naga`.
//!
//! [`prepare`] runs steps 0 to 4 and returns serializable SPIR-V plus
//! reflection ([`Prepared`], what the cache keeps); [`Prepared::finish`] runs
//! step 5. [`translate`] does both.

use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::compile::{self, Stage};
use crate::errors::{self, Diagnostic};
use crate::limits;
use crate::normalize;
use crate::reflect::{self, Reflection};
use crate::rewrite::{self, SplitBinding};

/// The compositor's vertex shader (`PM.VERT`, `legacy/gl/shaders.ts`),
/// byte for byte. Every effect, transition, and fragment layer links with it.
pub const VERTEX: &str = "#version 300 es
precision highp float;
in vec2 a_pos;
uniform mat3 u_m;
uniform vec2 u_res;
uniform vec2 u_pxOrigin;
uniform vec4 u_uv;           // xy = offset, zw = scale
out vec2 v_uv;               // content space (top-left origin)
out vec2 v_st;               // fbo space
out vec2 v_px;               // comp pixels
void main(){
  v_uv = u_uv.xy + a_pos * u_uv.zw;
  v_st = vec2(a_pos.x, 1.0 - a_pos.y);
  vec3 p = u_m * vec3(a_pos, 1.0);
  v_px = p.xy / p.z + u_pxOrigin;
  gl_Position = vec4((p.x / u_res.x) * 2.0 - p.z, p.z - (p.y / u_res.y) * 2.0, 0.0, p.z);
}";

/// The vertex outputs and the locations fragment inputs are pinned to.
pub const VARYINGS: &[(&str, u32)] = &[("v_uv", 0), ("v_st", 1), ("v_px", 2)];

/// Bumped whenever a stage changes what it emits; part of every cache key.
pub const PIPELINE_REVISION: u32 = 1;

/// The stage a program failed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureStage {
    /// Source limits: WebGL's own rules, or the engine's budgets.
    Limits,
    /// WebGL's compile and link (glslang's ES front end).
    EsValidate,
    Normalize,
    Glslang,
    Rewrite,
    /// An active resource the compositor never binds (a named uniform
    /// block, a sampler other than a float `sampler2D`).
    Bind,
    NagaParse,
    NagaValidate,
    /// Translation did not finish within its deadline.
    Deadline,
    /// A stage panicked.
    Panic,
    /// The program translates, but its translation is proved to draw
    /// differently from WebGL's at discontinuities (`pm_render`'s
    /// `effects::ANGLE_ONLY`): where a runner is attached it draws instead.
    AngleParity,
}

impl FailureStage {
    pub fn name(self) -> &'static str {
        match self {
            FailureStage::Limits => "limits",
            FailureStage::EsValidate => "es-validate",
            FailureStage::Normalize => "normalize",
            FailureStage::Glslang => "glslang",
            FailureStage::Rewrite => "rewrite",
            FailureStage::Bind => "bind",
            FailureStage::NagaParse => "naga-parse",
            FailureStage::NagaValidate => "naga-validate",
            FailureStage::Deadline => "deadline",
            FailureStage::Panic => "panic",
            FailureStage::AngleParity => "angle-parity",
        }
    }

    /// WebGL refuses the program too (or the engine's budget refuses a
    /// program that stalls WebGL): today the effect renders nothing and is
    /// skipped, so the engine skips it the same way. Every other stage is an
    /// engine-only failure: the program is valid WebGL and needs the
    /// decision 19 fallback runner.
    pub fn webgl_refuses(self) -> bool {
        matches!(self, FailureStage::Limits | FailureStage::EsValidate)
    }
}

/// Why a program cannot run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub stage: FailureStage,
    pub message: String,
    /// Diagnostics in the author's lines (section 11, step 7).
    pub diagnostics: Vec<Diagnostic>,
}

impl Failure {
    pub fn new(stage: FailureStage, message: impl Into<String>) -> Self {
        Failure {
            stage,
            message: message.into(),
            diagnostics: Vec::new(),
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.stage.name(), self.message)
    }
}

impl std::error::Error for Failure {}

/// One stage after the rewrites: SPIR-V naga reads, and what binding needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreparedStage {
    pub words: Vec<u32>,
    /// The default uniform block (GL "active" members) and the samplers.
    pub reflection: Reflection,
    /// Image and sampler bindings of every former `sampler2D`.
    pub splits: Vec<SplitBinding>,
}

/// Steps 0 to 4 of one program: what the translation cache keeps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prepared {
    pub vertex: PreparedStage,
    pub fragment: PreparedStage,
    /// Identifiers renamed away from GLSL 4.50 keywords and built-ins.
    pub renamed: Vec<String>,
    /// Polyfills and SPIR-V rewrites that changed the program.
    pub notes: Vec<String>,
    /// glslang warnings for the fragment stage.
    pub warnings: String,
}

/// A stage ready for wgpu.
#[derive(Debug, Clone)]
pub struct StageModule {
    pub module: naga::Module,
    pub reflection: Reflection,
    pub splits: Vec<SplitBinding>,
}

/// A translated program.
#[derive(Debug, Clone)]
pub struct Program {
    pub vertex: StageModule,
    pub fragment: StageModule,
    pub renamed: Vec<String>,
    pub notes: Vec<String>,
    pub warnings: String,
    /// Milliseconds per stage, for diagnostics.
    pub timings_ms: BTreeMap<String, f64>,
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

/// `PM.VERT` with the quad from `gl_VertexIndex` instead of a vertex buffer:
/// the compositor's `[0,0, 1,0, 0,1, 1,1]` triangle strip.
pub fn wgpu_vertex_source(gl_vertex: &str) -> String {
    gl_vertex.replacen(
        "in vec2 a_pos;",
        "vec2 pm_quad(){ return vec2(float(gl_VertexIndex & 1), float(gl_VertexIndex >> 1)); }\n#define a_pos pm_quad()",
        1,
    )
}

fn derivative_uniformity_off(module: &mut naga::Module) {
    use naga::diagnostic_filter::{
        DiagnosticFilter, DiagnosticFilterNode, FilterableTriggeringRule, Severity,
        StandardFilterableTriggeringRule,
    };
    let node = module.diagnostic_filters.append(
        DiagnosticFilterNode {
            inner: DiagnosticFilter {
                new_severity: Severity::Off,
                triggering_rule: FilterableTriggeringRule::Standard(
                    StandardFilterableTriggeringRule::DerivativeUniformity,
                ),
            },
            parent: module.diagnostic_filter_leaf,
        },
        naga::Span::UNDEFINED,
    );
    module.diagnostic_filter_leaf = Some(node);
}

/// naga's validator with every check and capability.
pub fn naga_validate(module: &naga::Module) -> Result<naga::valid::ModuleInfo, String> {
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(module)
    .map_err(|e| format!("{:?}", e.into_inner()))
}

/// glslang, then the rewrites. Returns the module, glslang's output, and
/// notes on rewrites that changed the program.
fn stage_spirv(
    source: &str,
    stage: Stage,
    line_offset: u32,
) -> Result<(rspirv::dr::Module, compile::Spirv, Vec<String>), Failure> {
    let spv = compile::compile(source, stage).map_err(|e| Failure {
        stage: FailureStage::Glslang,
        message: e.to_string(),
        diagnostics: errors::map(&e.log, line_offset),
    })?;
    let rewrite_failure = |e: rewrite::RewriteError| Failure::new(FailureStage::Rewrite, e.0);
    let mut module = rewrite::load(&spv.words).map_err(rewrite_failure)?;
    let mut notes = Vec::new();
    if stage == Stage::Fragment {
        let arrays = rewrite::scalarize_sampler_arrays(&mut module).map_err(rewrite_failure)?;
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

/// The naga-only rewrites and the sampler split; the result is what naga
/// parses.
fn finish_spirv(
    mut module: rspirv::dr::Module,
    spv: &compile::Spirv,
    notes: &mut Vec<String>,
) -> Result<PreparedStage, Failure> {
    notes.extend(rewrite::naga_compat(&mut module));
    let reflection = reflect::reflect(&module, &reflect::bool_uniforms(&spv.preprocessed));
    let materialized = rewrite::materialize_non_finite_constants(&mut module);
    if materialized > 0 {
        notes.push(format!("non-finite constants ({materialized})"));
    }
    let splits = rewrite::split_combined_samplers(&mut module)
        .map_err(|e| Failure::new(FailureStage::Rewrite, e.0))?;
    Ok(PreparedStage {
        words: rewrite::assemble(&module),
        reflection,
        splits,
    })
}

fn limit_failure(error: &limits::LimitError, line_offset: u32) -> Failure {
    Failure {
        stage: FailureStage::Limits,
        message: error.to_string(),
        diagnostics: errors::map(&error.to_string(), line_offset),
    }
}

/// The vertex stage, prepared once per process (it never changes).
fn vertex_stage() -> Result<PreparedStage, Failure> {
    static VERTEX_STAGE: std::sync::OnceLock<Result<PreparedStage, Failure>> =
        std::sync::OnceLock::new();
    VERTEX_STAGE
        .get_or_init(|| {
            let normalized = normalize::normalize(&wgpu_vertex_source(VERTEX))
                .map_err(|e| Failure::new(FailureStage::Normalize, format!("vertex: {}", e.0)))?;
            let (module, spv, mut notes) = stage_spirv(&normalized.source, Stage::Vertex, 0)?;
            finish_spirv(module, &spv, &mut notes)
        })
        .clone()
}

/// Steps 0 to 4 for a fragment program linked with [`VERTEX`].
/// `line_offset` is the number of program lines before the author's first
/// line (`WrappedGlsl::body_line_offset`).
pub fn prepare(fragment: &str, line_offset: u32) -> Result<Prepared, Failure> {
    prepare_timed(fragment, line_offset, &mut BTreeMap::new())
}

fn prepare_timed(
    fragment: &str,
    line_offset: u32,
    timings: &mut BTreeMap<String, f64>,
) -> Result<Prepared, Failure> {
    let start = Instant::now();
    limits::check_source(fragment).map_err(|e| limit_failure(&e, line_offset))?;
    timings.insert("limits".into(), ms(start));
    let start = Instant::now();
    compile::validate_es(VERTEX, &normalize::rename_reserved(fragment)).map_err(|e| Failure {
        stage: FailureStage::EsValidate,
        message: e.to_string(),
        diagnostics: errors::map(&e.log, line_offset),
    })?;
    timings.insert("es-validate".into(), ms(start));
    let start = Instant::now();
    let normalized =
        normalize::normalize(fragment).map_err(|e| Failure::new(FailureStage::Normalize, e.0))?;
    timings.insert("normalize".into(), ms(start));
    let start = Instant::now();
    let (module, spv, mut notes) = stage_spirv(&normalized.source, Stage::Fragment, line_offset)?;
    timings.insert("glslang".into(), ms(start));
    let unbound = reflect::unbound_resources(&module);
    if !unbound.is_empty() {
        return Err(Failure::new(
            FailureStage::Bind,
            format!(
                "the compositor binds only float sampler2D textures and the default uniform block; WebGL cannot draw this program as Powermove binds it: {}",
                unbound.join(", ")
            ),
        ));
    }
    let start = Instant::now();
    let active: HashSet<u32> =
        reflect::reflect(&module, &reflect::bool_uniforms(&spv.preprocessed))
            .block
            .map(|b| {
                b.members
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| m.active)
                    .map(|(k, _)| k as u32)
                    .collect()
            })
            .unwrap_or_default();
    limits::check_spirv(&module, &active).map_err(|e| limit_failure(&e, line_offset))?;
    timings.insert("limits-spirv".into(), ms(start));
    let start = Instant::now();
    let fragment_stage = finish_spirv(module, &spv, &mut notes)?;
    timings.insert("rewrite".into(), ms(start));
    let mut all_notes = normalized.polyfilled;
    all_notes.extend(notes);
    Ok(Prepared {
        vertex: vertex_stage()?,
        fragment: fragment_stage,
        renamed: normalized.renamed,
        notes: all_notes,
        warnings: spv.warnings.trim().to_string(),
    })
}

fn naga_module(words: &[u32]) -> Result<naga::Module, Failure> {
    let options = naga::front::spv::Options {
        adjust_coordinate_space: true,
        strict_capabilities: false,
        block_ctx_dump_prefix: None,
    };
    let words = words.to_vec();
    let parsed = std::panic::catch_unwind(|| {
        naga::front::spv::Frontend::new(words.into_iter(), &options).parse()
    });
    let mut module = match parsed {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => return Err(Failure::new(FailureStage::NagaParse, format!("{e:?}"))),
        Err(_) => {
            return Err(Failure::new(
                FailureStage::NagaParse,
                "naga's SPIR-V front end panicked",
            ));
        }
    };
    derivative_uniformity_off(&mut module);
    naga_validate(&module).map_err(|e| Failure::new(FailureStage::NagaValidate, e))?;
    Ok(module)
}

impl Prepared {
    /// Step 5: naga modules for wgpu.
    pub fn finish(&self) -> Result<Program, Failure> {
        let start = Instant::now();
        let vertex = naga_module(&self.vertex.words)?;
        let fragment = naga_module(&self.fragment.words)?;
        let mut timings_ms = BTreeMap::new();
        timings_ms.insert("naga".into(), ms(start));
        Ok(Program {
            vertex: StageModule {
                module: vertex,
                reflection: self.vertex.reflection.clone(),
                splits: self.vertex.splits.clone(),
            },
            fragment: StageModule {
                module: fragment,
                reflection: self.fragment.reflection.clone(),
                splits: self.fragment.splits.clone(),
            },
            renamed: self.renamed.clone(),
            notes: self.notes.clone(),
            warnings: self.warnings.clone(),
            timings_ms,
        })
    }
}

/// Steps 0 to 5.
pub fn translate(fragment: &str, line_offset: u32) -> Result<Program, Failure> {
    let mut timings = BTreeMap::new();
    let prepared = prepare_timed(fragment, line_offset, &mut timings)?;
    let mut program = prepared.finish()?;
    timings.append(&mut program.timings_ms);
    program.timings_ms = timings;
    Ok(program)
}

/// naga's MSL for a module, as wgpu's Metal back end writes it (fuzzing and
/// diagnostics; wgpu runs the same writer at pipeline creation).
pub fn msl(module: &naga::Module) -> Result<String, String> {
    let info = naga_validate(module)?;
    let options = naga::back::msl::Options {
        lang_version: (3, 0),
        ..Default::default()
    };
    naga::back::msl::write_string(module, &info, &options, &Default::default())
        .map(|(source, _)| source)
        .map_err(|e| format!("{e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: &str = "#version 300 es\nprecision highp float;\nin vec2 v_uv;\nuniform sampler2D u_tex;\nuniform vec2 u_res;\nout vec4 o;\n";

    fn ok(body: &str) -> Program {
        let source = format!("{H}{body}");
        let p = translate(&source, 6)
            .unwrap_or_else(|f| panic!("{}: {}\n{source}", f.stage.name(), f.message));
        msl(&p.fragment.module).expect("msl");
        msl(&p.vertex.module).expect("vertex msl");
        p
    }

    #[test]
    fn gentype_builtins_naga_lacks_are_polyfilled() {
        let t = ok(
            "void main(){ float w; vec3 w3; float f = modf(v_uv.x * 3.0, w); vec3 f3 = modf(vec3(v_uv, 1.5), w3);\n  float n = normalize(v_uv.x - 0.5) + faceforward(1.0, v_uv.y, -1.0) + reflect(v_uv.x, 0.6) + refract(0.5, -0.8, 0.66);\n  o = vec4(f + w, f3.x + w3.y, n, normalize(v_uv).x); }\n",
        );
        for name in ["modf", "normalize", "faceforward", "reflect", "refract"] {
            assert!(t.notes.iter().any(|p| p == name), "{name}: {:?}", t.notes);
        }
    }

    #[test]
    fn integer_remainder_and_unsigned_negation() {
        let t = ok(
            "void main(){ int i = int(v_uv.x * 100.0) - 50; uint u = uint(i + 60); int r = i % 7; r %= 5; uint n = -u; o = vec4(float(r), float(n & 255u), float(i % 3), 1.0); }\n",
        );
        assert!(
            t.notes
                .iter()
                .any(|p| p.starts_with("signed % as remainder")),
            "{:?}",
            t.notes
        );
        assert!(
            t.notes.iter().any(|p| p.starts_with("uint negation")),
            "{:?}",
            t.notes
        );
    }

    #[test]
    fn sampler_arrays_become_one_sampler_per_element() {
        let t = ok(
            "uniform sampler2D u_extra[3];\nvec4 tap(sampler2D s, vec2 uv){ return texture(s, uv); }\nvoid main(){ const int K = 2; o = texture(u_extra[0], v_uv) + tap(u_extra[K], v_uv) + texture(u_tex, v_uv); }\n",
        );
        let mut names: Vec<&str> = t
            .fragment
            .reflection
            .samplers
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        names.sort();
        assert_eq!(names, ["u_extra[0]", "u_extra[1]", "u_extra[2]", "u_tex"]);
        assert_eq!(t.fragment.splits.len(), 4);
    }

    #[test]
    fn fragment_interface_follows_webgl() {
        for (decl, body) in [
            (
                "out vec3 o3;",
                "void main(){ o3 = texture(u_tex, v_uv).rgb; }\n",
            ),
            ("out float of;", "void main(){ of = v_uv.x; }\n"),
            (
                "out vec4 oa[1];",
                "void main(){ oa[0] = vec4(v_uv, 0.0, 1.0); }\n",
            ),
        ] {
            let source = format!(
                "#version 300 es\nprecision highp float;\nin vec2 v_uv;\nuniform sampler2D u_tex;\n{decl}\n{body}"
            );
            let t = translate(&source, 5).unwrap_or_else(|f| panic!("{f}"));
            assert!(
                t.notes.iter().any(|p| p.contains("widened to vec4")),
                "{:?}",
                t.notes
            );
        }
        let t = ok(
            "centroid in vec2 nope;\nvoid main(){ gl_FragDepth = 0.25; o = texture(u_tex, v_uv); }\n",
        );
        assert!(
            t.notes.iter().any(|p| p.contains("gl_FragDepth")),
            "{:?}",
            t.notes
        );
    }

    #[test]
    fn unbound_resources_are_refused_with_a_reason() {
        for decl in [
            "uniform highp sampler3D u_s;\nvoid main(){ o = texture(u_s, vec3(v_uv, 0.5)); }\n",
            "uniform highp isampler2D u_s;\nvoid main(){ o = vec4(texelFetch(u_s, ivec2(0), 0)); }\n",
            "uniform Params { vec4 c; };\nvoid main(){ o = c; }\n",
        ] {
            let f = translate(&format!("{H}{decl}"), 6).expect_err("refused");
            assert_eq!(f.stage, FailureStage::Bind, "{}", f.message);
            assert!(!f.stage.webgl_refuses());
        }
        ok(
            "uniform highp sampler3D u_s;\nuniform Params { vec4 c; };\nvoid main(){ o = texture(u_tex, v_uv); }\n",
        );
    }

    #[test]
    fn errors_map_to_the_authors_lines() {
        let f = translate(&format!("{H}void main(){{\n  o = vec4(nope);\n}}\n"), 6)
            .err()
            .unwrap();
        assert_eq!(f.stage, FailureStage::EsValidate);
        assert!(f.stage.webgl_refuses());
        assert_eq!(f.diagnostics[0].line, Some(2), "{:?}", f.diagnostics);
    }

    #[test]
    fn prepared_programs_round_trip_through_json() {
        let prepared = prepare(&format!("{H}uniform float u_amount;\nvoid main(){{ o = texture(u_tex, v_uv) * u_amount; }}\n"), 6).unwrap();
        let json = serde_json::to_vec(&prepared).unwrap();
        let back: Prepared = serde_json::from_slice(&json).unwrap();
        assert_eq!(back, prepared);
        back.finish().unwrap();
    }
}
