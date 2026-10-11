//! The section 11 translation pipeline, both back ends:
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
    /// normalize, glslang, rewrite, naga-parse, naga-validate, spvc, metal, render
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
}

pub struct Translated {
    pub vertex: StageOutput,
    pub fragment: StageOutput,
    pub renamed: Vec<String>,
    pub timings_ms: HashMap<String, f64>,
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

fn stage_spirv(source: &str, stage: Stage, line_offset: u32, timings: &mut HashMap<String, f64>, prefix: &str) -> Result<(rspirv::dr::Module, compile::Spirv), Failure> {
    let start = Instant::now();
    let spv = compile::compile(source, stage).map_err(|e| Failure {
        stage: "glslang".into(),
        message: e.to_string(),
        diagnostics: errors::map(&e.log, line_offset),
    })?;
    timings.insert(format!("{prefix}glslang"), ms(start));
    let mut module = rewrite::load(&spv.words).map_err(|e| Failure::new("rewrite", e.0))?;
    rewrite::assign_bindings(&mut module, compile::DEFAULT_BLOCK_BINDING);
    if stage == Stage::Fragment {
        rewrite::remap_varyings(&mut module, VARYINGS);
    }
    Ok((module, spv))
}

fn naga_stage(mut module: rspirv::dr::Module, spv: &compile::Spirv, timings: &mut HashMap<String, f64>, prefix: &str) -> Result<StageOutput, Failure> {
    let reflection = reflect::reflect(&module, &reflect::bool_uniforms(&spv.preprocessed));
    let start = Instant::now();
    let splits = rewrite::split_combined_samplers(&mut module).map_err(|e| Failure::new("rewrite", e.0))?;
    timings.insert(format!("{prefix}split"), ms(start));
    let words = rewrite::assemble(&module);
    let start = Instant::now();
    let options = naga::front::spv::Options { adjust_coordinate_space: true, strict_capabilities: false, block_ctx_dump_prefix: None };
    let parsed = std::panic::catch_unwind(|| naga::front::spv::Frontend::new(words.into_iter(), &options).parse());
    let mut naga_module = match parsed {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => return Err(Failure::new("naga-parse", format!("{e:?}"))),
        Err(_) => return Err(Failure::new("naga-parse", "naga SPIR-V front end panicked")),
    };
    timings.insert(format!("{prefix}naga-parse"), ms(start));
    derivative_uniformity_off(&mut naga_module);
    let start = Instant::now();
    naga_validate(&naga_module).map_err(|e| Failure::new("naga-validate", e))?;
    timings.insert(format!("{prefix}naga-validate"), ms(start));
    Ok(StageOutput { reflection, splits, naga: Some(naga_module), msl: None, entry_point: "main".into() })
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
    Ok(StageOutput { reflection, splits: Vec::new(), naga: None, msl: Some(artifact.to_string()), entry_point: entry })
}

pub fn translate(gl_vertex: &str, fragment: &str, line_offset: u32, path: Path) -> Result<Translated, Failure> {
    let mut timings = HashMap::new();
    let start = Instant::now();
    let normalized = normalize::normalize(fragment).map_err(|e| Failure::new("normalize", e.0))?;
    let vertex = normalize::normalize(&wgpu_vertex_source(gl_vertex)).map_err(|e| Failure::new("normalize", format!("vertex: {}", e.0)))?;
    timings.insert("normalize".into(), ms(start));
    let (vmod, vspv) = stage_spirv(&vertex.source, Stage::Vertex, 0, &mut timings, "vertex-")?;
    let (fmod, fspv) = stage_spirv(&normalized.source, Stage::Fragment, line_offset, &mut timings, "")?;
    let (vertex_out, fragment_out) = match path {
        Path::Naga => (naga_stage(vmod, &vspv, &mut timings, "vertex-")?, naga_stage(fmod, &fspv, &mut timings, "")?),
        Path::SpirvCross => (
            spvc_stage(vmod, &vspv, Stage::Vertex, &mut timings, "vertex-")?,
            spvc_stage(fmod, &fspv, Stage::Fragment, &mut timings, "")?,
        ),
    };
    Ok(Translated { vertex: vertex_out, fragment: fragment_out, renamed: normalized.renamed, timings_ms: timings })
}
