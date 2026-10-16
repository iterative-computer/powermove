//! Effects and transitions through the production GLSL pipeline (plan
//! section 11; section 8, "effect pass" and "transition" nodes, the
//! watchdog; contract 10). Design notes: `docs/engine/effects.md`.
//!
//! - [`Effects`]: the definition set of a renderer (the winning
//!   registrations the kernel pushes, contract 10), each definition's
//!   translation through `pm_glsl` (cached by source hash), compiled
//!   programs, and the per-effect GPU watchdog.
//! - `plan`: what a layer's effects and transition need at a frame (the
//!   effect pass and transition nodes of the render graph).
//! - `exec`: the compositor's `runEffects`, `runTransition`, and the layer
//!   buffer around them, in GL row order.
//! - `program`: a translated program as wgpu pipelines, and GL's uniform
//!   binding rules.
//!
//! Built-ins keep their GLSL (section 11): the 16 effects and 4
//! transitions run through the same pipeline as every Store or agent
//! shader; [`builtin_definitions`] embeds their snapshots as the kernel
//! produces them (`tools/effects/builtins.ts`).

pub(crate) mod exec;
pub mod plan;
pub mod program;
pub mod watchdog;

use std::collections::BTreeMap;
use std::sync::Arc;

use pm_glsl::{Failure, Prepared};
use pm_protocol::common::{DefinitionId, Seq};
use pm_protocol::definitions::{
    DefinitionKind, DefinitionSet, DefinitionSetSnapshot, DefinitionSnapshot, DefinitionsUpdate,
    UpdateError,
};
use pm_protocol::error::{
    EngineError, ErrorCode, ErrorDetail, ErrorScope, Retry, ShaderDiagnostic,
};

use program::GpuProgram;
pub use watchdog::Watchdog;

/// The compositor's vertex shader every legacy program links with.
pub use pm_glsl::VERTEX;

/// Built-in effects in amendment 9's WebGL-unstable class: today's WebGL
/// output itself moves past the Standard tolerance when its input moves by
/// one binary16 step (`tests/effects_webgl.rs`,
/// `webgl_instability_under_a_one_step_input_perturbation`: Posterize and
/// Levels are step functions at some parameters, Color's hue rotation at
/// saturated extremes). Where the engine's input to them differs from
/// today's at rounding level, their output differs at isolated pixels;
/// such frames are judged by review, not by the Standard class alone.
pub const WEBGL_UNSTABLE: &[&str] = &["posterize", "levels", "color"];

/// Where a definition stands.
#[derive(Debug, Clone)]
pub enum Status {
    /// Translated; draws.
    Ready,
    /// WebGL refuses the program too (or the engine's budget stops a program
    /// that stalls WebGL): today it renders nothing and is skipped, so the
    /// engine skips it the same way and reports the diagnostics.
    Refused(Failure),
    /// A WebGL-valid program the engine cannot run: it needs the decision 19
    /// fallback runner, so layers using it are routed (not drawn wrong).
    NeedsFallback(Failure),
}

struct Entry {
    /// The source hash the status belongs to.
    key: pm_glsl::Key,
    status: Status,
    prepared: Option<Arc<Prepared>>,
    /// Built on first use (the Metal compiler runs there).
    program: Option<GpuProgram>,
}

/// The renderer's definitions and their programs.
pub struct Effects {
    set: DefinitionSet,
    /// Whether any definitions were given. Without them nothing is known
    /// missing either, and effects and transitions are routed.
    configured: bool,
    /// Ids an embedded set lists as missing (`DefinitionSetSnapshot`
    /// carries ids without kinds; contract 10, open question 3).
    missing_ids: std::collections::BTreeSet<DefinitionId>,
    entries: BTreeMap<(DefinitionKind, DefinitionId), Entry>,
    /// Shared by the renderers' definition sets ([`Effects::sibling`]).
    cache: Arc<pm_glsl::Cache>,
    context: String,
    pub watchdog: Watchdog,
    /// What an empty texture unit samples: (0, 0, 0, 1), per format.
    dummies: std::collections::HashMap<wgpu::TextureFormat, pm_gpu::Target>,
    simple: std::collections::HashMap<(SimpleOp, wgpu::TextureFormat), wgpu::RenderPipeline>,
}

/// Per-pixel passes around legacy programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SimpleOp {
    /// `clamp: true`: the effect's output limited to 0..1.
    Clamp,
    /// Premultiplied linear light to premultiplied E (a legacy program's
    /// input with linear blending on).
    EncodeE,
    /// Premultiplied E back to premultiplied linear light.
    DecodeE,
}

const SIMPLE_WGSL: &str = "
@group(0) @binding(0) var src: texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
  let a = vec2<f32>(f32(i & 1u), f32(i >> 1u));
  return vec4<f32>(a * 2.0 - 1.0, 0.0, 1.0);
}
fn texel(p: vec4<f32>) -> vec4<f32> {
  return textureLoad(src, vec2<i32>(p.xy), 0);
}
@fragment fn clamp01(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
  return clamp(texel(p), vec4<f32>(0.0), vec4<f32>(1.0));
}
@fragment fn encode(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
  return pm_premultiplied_linear_to_e(texel(p));
}
@fragment fn decode(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
  return pm_premultiplied_e_to_linear(texel(p));
}
";

/// The built-in definitions as the kernel snapshots them, and `PM.VERT`
/// (`src/effects/builtins.json`, written by `tools/effects/builtins.ts`).
pub fn builtin_definitions() -> DefinitionSetSnapshot {
    #[derive(serde::Deserialize)]
    struct Builtins {
        vertex: String,
        definitions: Vec<DefinitionSnapshot>,
    }
    let builtins: Builtins = serde_json::from_str(include_str!("builtins.json"))
        .expect("builtins.json is a definition list");
    debug_assert_eq!(builtins.vertex, VERTEX);
    DefinitionSetSnapshot {
        generation: Seq::new(1).expect("1 is safe"),
        definitions: builtins.definitions,
        missing: Vec::new(),
    }
}

fn diagnostics(failure: &Failure) -> Vec<ShaderDiagnostic> {
    failure
        .diagnostics
        .iter()
        .map(|d| ShaderDiagnostic {
            // 0: the problem is in Powermove's preamble or generated
            // declarations, not in a line the author wrote.
            line: d.line.unwrap_or(0),
            column: None,
            message: d.message.clone(),
        })
        .collect()
}

fn truncate(mut message: String) -> String {
    const MAX: usize = pm_protocol::error::MAX_ERROR_MESSAGE_BYTES;
    if message.len() > MAX {
        let mut end = MAX - 3;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
        message.push_str("...");
    }
    message
}

/// The definition-scoped error a failure is reported as (contract 7).
pub fn definition_error(id: &DefinitionId, status: &Status) -> Option<EngineError> {
    let (code, failure, what) = match status {
        Status::Ready => return None,
        Status::Refused(f) => (ErrorCode::ShaderCompile, f, "does not compile"),
        Status::NeedsFallback(f) => (
            ErrorCode::NotHeadlessRenderable,
            f,
            "compiles in WebGL but not in the native engine's pipeline",
        ),
    };
    let message = truncate(format!(
        "The shader of {} {what} ({}: {}).",
        id.as_str(),
        failure.stage.name(),
        failure.message.lines().next().unwrap_or_default()
    ));
    Some(
        EngineError::new(
            code,
            ErrorScope::Definition {
                definition: id.clone(),
            },
            message,
        )
        .with_retry(Retry::AfterUserAction)
        .with_detail(ErrorDetail::ShaderCompile {
            diagnostics: diagnostics(failure),
        }),
    )
}

impl Effects {
    /// `context` names the device (back end and driver) for the
    /// translation cache.
    pub fn new(context: impl Into<String>) -> Effects {
        let context = context.into();
        Effects::with_cache(
            Arc::new(pm_glsl::Cache::new(context.clone(), None)),
            context,
        )
    }

    fn with_cache(cache: Arc<pm_glsl::Cache>, context: String) -> Effects {
        Effects {
            set: DefinitionSet::default(),
            configured: false,
            missing_ids: Default::default(),
            entries: BTreeMap::new(),
            cache,
            context,
            watchdog: Watchdog::default(),
            dummies: Default::default(),
            simple: Default::default(),
        }
    }

    /// An empty definition set sharing this one's translation cache (one
    /// per view of a device: each keeps its own programs and watchdog).
    pub fn sibling(&self) -> Effects {
        Effects::with_cache(self.cache.clone(), self.context.clone())
    }

    /// Keeps prepared programs on disk across processes. Siblings made
    /// before the call keep the previous cache.
    pub fn set_cache_dir(&mut self, dir: Option<std::path::PathBuf>) {
        self.cache = Arc::new(pm_glsl::Cache::new(self.context.clone(), dir));
    }

    pub fn configured(&self) -> bool {
        self.configured
    }

    pub fn generation(&self) -> Option<Seq> {
        self.set.generation()
    }

    /// Applies a kernel update (contract 10) and translates what changed.
    /// Returns the failures of the definitions it upserted, for
    /// `DefinitionsApplied`.
    pub fn apply(&mut self, update: &DefinitionsUpdate) -> Result<Vec<EngineError>, UpdateError> {
        self.set.apply(update)?;
        self.configured = true;
        if update.full {
            self.missing_ids.clear();
        }
        let mut failures = Vec::new();
        if update.full {
            let live: std::collections::BTreeSet<_> = update
                .upserts
                .iter()
                .map(|d| (d.kind, d.id.clone()))
                .collect();
            self.drop_entries(|k| !live.contains(k));
        }
        for removal in &update.removals {
            let key = (removal.kind, removal.id.clone());
            self.drop_entries(|k| *k == key);
        }
        for snapshot in &update.upserts {
            if let Some(e) = self.translate(snapshot) {
                failures.push(e);
            }
        }
        Ok(failures)
    }

    /// Replaces every definition with an embedded set (an export job, a
    /// collected project, `pm-cli`, tests, the interactive engine's view).
    /// Definitions whose source did not change keep their program and their
    /// watchdog record (an effect the watchdog turned off stays off); a
    /// changed or removed one is translated again or dropped.
    pub fn set_snapshot(
        &mut self,
        snapshot: &DefinitionSetSnapshot,
    ) -> Result<Vec<EngineError>, UpdateError> {
        let mut set = DefinitionSet::default();
        let generation = snapshot.generation.max(Seq::new(1).expect("safe"));
        let update = DefinitionsUpdate {
            generation,
            full: true,
            upserts: snapshot.definitions.clone(),
            removals: Vec::new(),
        };
        set.apply(&update)?;
        self.missing_ids = snapshot.missing.iter().cloned().collect();
        self.set = set;
        self.configured = true;
        let live: std::collections::BTreeSet<_> = snapshot
            .definitions
            .iter()
            .map(|d| (d.kind, d.id.clone()))
            .collect();
        self.drop_entries(|k| !live.contains(k));
        let mut failures = Vec::new();
        for snapshot in &snapshot.definitions {
            if let Some(e) = self.translate(snapshot) {
                failures.push(e);
            }
        }
        Ok(failures)
    }

    /// Drops the entries `gone` selects, with their watchdog records.
    fn drop_entries(&mut self, gone: impl Fn(&(DefinitionKind, DefinitionId)) -> bool) {
        let removed: Vec<_> = self.entries.keys().filter(|k| gone(k)).cloned().collect();
        for key in removed {
            self.entries.remove(&key);
            self.watchdog.forget(key.0, &key.1);
        }
    }

    /// Forgets every definition: effects and transitions are routed again.
    pub fn clear(&mut self) {
        self.set = DefinitionSet::default();
        self.configured = false;
        self.missing_ids.clear();
        self.entries.clear();
        self.watchdog.reset();
    }

    /// The definition set an export job or Collect Project embeds.
    pub fn snapshot(&self) -> DefinitionSetSnapshot {
        let mut snapshot = self.set.snapshot();
        for id in &self.missing_ids {
            if !snapshot.missing.contains(id) && !snapshot.definitions.iter().any(|d| &d.id == id) {
                snapshot.missing.push(id.clone());
            }
        }
        snapshot
    }

    /// Translates one snapshot (cached by source); reuses the program when
    /// the source did not change.
    fn translate(&mut self, snapshot: &DefinitionSnapshot) -> Option<EngineError> {
        let key = (snapshot.kind, snapshot.id.clone());
        let Some(glsl) = &snapshot.glsl else {
            self.entries.remove(&key);
            return None;
        };
        let source_key = self.cache.key(&glsl.source, glsl.body_line_offset);
        if let Some(entry) = self.entries.get(&key)
            && entry.key == source_key
        {
            return definition_error(&snapshot.id, &entry.status);
        }
        let prepared = self.cache.prepare(&glsl.source, glsl.body_line_offset);
        let (status, prepared) = match prepared.as_ref() {
            Ok(p) => (Status::Ready, Some(Arc::new(p.clone()))),
            Err(f) if f.stage.webgl_refuses() => (Status::Refused(f.clone()), None),
            Err(f) => (Status::NeedsFallback(f.clone()), None),
        };
        let error = definition_error(&snapshot.id, &status);
        // A changed definition gets a fresh watchdog record.
        self.watchdog.forget(snapshot.kind, &snapshot.id);
        self.entries.insert(
            key,
            Entry {
                key: source_key,
                status,
                prepared,
                program: None,
            },
        );
        error
    }

    pub fn get(&self, kind: DefinitionKind, id: &str) -> Option<&DefinitionSnapshot> {
        self.set.get(kind, &DefinitionId::from(id))
    }

    pub fn is_missing(&self, kind: DefinitionKind, id: &str) -> bool {
        let id = DefinitionId::from(id);
        self.set.is_missing(kind, &id)
            || (self.set.get(kind, &id).is_none() && self.missing_ids.contains(&id))
    }

    pub fn status(&self, kind: DefinitionKind, id: &str) -> Option<&Status> {
        self.entries
            .get(&(kind, DefinitionId::from(id)))
            .map(|e| &e.status)
    }

    /// Every definition-scoped failure (refused and fallback programs).
    pub fn failures(&self) -> Vec<EngineError> {
        self.entries
            .iter()
            .filter_map(|((_, id), e)| definition_error(id, &e.status))
            .collect()
    }

    /// Whether a definition's GPU program is built (it is on first use, and
    /// kept while its source does not change).
    pub fn has_program(&self, kind: DefinitionKind, id: &str) -> bool {
        self.entries
            .get(&(kind, DefinitionId::from(id)))
            .is_some_and(|e| e.program.is_some())
    }

    /// The source hash of a definition's program (content keys).
    pub fn source_key(&self, kind: DefinitionKind, id: &str) -> Option<pm_glsl::Key> {
        self.entries
            .get(&(kind, DefinitionId::from(id)))
            .map(|e| e.key)
    }

    /// A 1×1 (0, 0, 0, 1) texture: WebGL2 samples an empty unit so.
    pub(crate) fn dummy(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> pm_gpu::Target {
        self.dummies
            .entry(format)
            .or_insert_with(|| {
                let t = pm_gpu::Target::new(
                    device,
                    "pm empty texture unit",
                    1,
                    1,
                    format,
                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                );
                let bytes: Vec<u8> = match format {
                    wgpu::TextureFormat::Rgba32Float => [0.0f32, 0.0, 0.0, 1.0]
                        .iter()
                        .flat_map(|v| v.to_le_bytes())
                        .collect(),
                    // rgba16float: 0, 0, 0, 1.0 (0x3c00).
                    _ => vec![0, 0, 0, 0, 0, 0, 0x00, 0x3c],
                };
                queue.write_texture(
                    t.texture.as_image_copy(),
                    &bytes,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(bytes.len() as u32),
                        rows_per_image: Some(1),
                    },
                    wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                );
                t
            })
            .clone()
    }

    /// The pipeline of a per-pixel pass.
    pub(crate) fn simple_pipeline(
        &mut self,
        device: &wgpu::Device,
        op: SimpleOp,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        self.simple.entry((op, format)).or_insert_with(|| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("pm effect pixel pass"),
                source: wgpu::ShaderSource::Wgsl(
                    format!("{}\n{SIMPLE_WGSL}", pm_color::wgsl::COLOR).into(),
                ),
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("pm effect pixel pass"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(match op {
                        SimpleOp::Clamp => "clamp01",
                        SimpleOp::EncodeE => "encode",
                        SimpleOp::DecodeE => "decode",
                    }),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
    }

    /// The GPU program of a ready definition, built on first use. A build
    /// failure turns the definition into a fallback one.
    pub(crate) fn program(
        &mut self,
        device: &wgpu::Device,
        kind: DefinitionKind,
        id: &str,
        format: wgpu::TextureFormat,
    ) -> Result<&mut GpuProgram, Failure> {
        let entry = self
            .entries
            .get_mut(&(kind, DefinitionId::from(id)))
            .ok_or_else(|| {
                Failure::new(
                    pm_glsl::FailureStage::Rewrite,
                    format!("{id} has no program"),
                )
            })?;
        match &entry.status {
            Status::Ready => {}
            Status::Refused(f) | Status::NeedsFallback(f) => return Err(f.clone()),
        }
        if entry.program.is_none() {
            let built = entry
                .prepared
                .as_ref()
                .ok_or_else(|| Failure::new(pm_glsl::FailureStage::Rewrite, "no prepared program"))?
                .finish()
                .and_then(|p| GpuProgram::new(device, p, id));
            match built {
                Ok(p) => entry.program = Some(p),
                Err(f) => {
                    entry.status = Status::NeedsFallback(f.clone());
                    return Err(f);
                }
            }
        }
        let program = entry.program.as_mut().expect("built above");
        if let Err(f) = program.pipeline(device, format) {
            entry.status = Status::NeedsFallback(f.clone());
            entry.program = None;
            return Err(f);
        }
        Ok(entry.program.as_mut().expect("still built"))
    }
}

impl crate::composite::EffectHost for Effects {
    fn prepare<'d>(
        &self,
        scope: &mut pm_eval::Scope<'_, 'd>,
        layer: &'d pm_model::Node,
        frame: crate::composite::effects::EffectFrame,
    ) -> pm_eval::JsResult<Result<Option<crate::composite::PreparedEffects>, crate::Feature>> {
        let steps = match plan::effect_steps(scope, self, layer, frame.time)? {
            Ok(steps) => steps,
            Err(f) => return Ok(Err(f)),
        };
        if steps.is_empty() {
            return Ok(Ok(None));
        }
        let sources = steps
            .iter()
            .map(|s| self.source_key(DefinitionKind::Effect, &s.definition))
            .collect();
        Ok(Ok(Some(crate::composite::PreparedEffects {
            padding: plan::chain_padding(&steps),
            backdrop: steps.iter().any(|s| s.backdrop),
            passes: exec::chain_passes(exec::Chain {
                layer: crate::js::layer_id(layer),
                steps,
                time: frame.time,
                sources,
            }),
        })))
    }

    fn transition_known(&self, ty: &pm_model::JsString) -> bool {
        plan::transition_known(self, ty)
    }

    fn prepare_transition<'d>(
        &self,
        scope: &mut pm_eval::Scope<'_, 'd>,
        layer: &'d pm_model::Node,
        frame: crate::composite::effects::EffectFrame,
    ) -> pm_eval::JsResult<Result<Option<crate::composite::PreparedTransition>, crate::Feature>>
    {
        let step = match plan::transition_at(scope, self, layer, frame.time)? {
            Ok(Some(step)) => step,
            Ok(None) => return Ok(Ok(None)),
            Err(f) => return Ok(Err(f)),
        };
        let source = self.source_key(DefinitionKind::Transition, &step.definition);
        Ok(Ok(Some(crate::composite::PreparedTransition {
            passes: exec::transition_passes(exec::Transition {
                layer: crate::js::layer_id(layer),
                step,
                time: frame.time,
                source,
            }),
        })))
    }
}

/// Static routing of a layer's effects (`capability`): every effect not
/// marked missing, enabled or not (the enable is animatable).
pub(crate) fn layer_features(effects: &Effects, layer: &pm_model::Node) -> Vec<crate::Feature> {
    use pm_eval::node;
    let mut out = Vec::new();
    for fx in node::array(node::member(Some(layer), "fx")).unwrap_or(&[]) {
        if matches!(
            node::member(Some(fx), "missing").map(pm_model::Node::kind),
            Some(pm_model::NodeKind::Bool(true))
        ) {
            continue;
        }
        let ty = node::to_js_string(node::member(Some(fx), "type")).to_string_lossy();
        out.extend(definition_feature(effects, DefinitionKind::Effect, &ty));
    }
    out
}

/// Static routing of a layer's transitions (`capability`).
pub(crate) fn transition_features(
    effects: &Effects,
    layer: &pm_model::Node,
) -> Vec<crate::Feature> {
    use pm_eval::node;
    let mut out = Vec::new();
    for key in ["transitionIn", "transitionOut"] {
        let Some(t) = node::member(Some(layer), key).filter(|t| node::truthy(Some(t))) else {
            continue;
        };
        if node::truthy(node::member(Some(t), "missing")) {
            continue;
        }
        let Some(ty) = node::string(node::member(Some(t), "type")) else {
            continue;
        };
        out.extend(definition_feature(
            effects,
            DefinitionKind::Transition,
            &ty.to_string_lossy(),
        ));
    }
    out
}

fn definition_feature(effects: &Effects, kind: DefinitionKind, id: &str) -> Option<crate::Feature> {
    let what = match kind {
        DefinitionKind::Effect => "effect",
        DefinitionKind::Transition => "transition",
        DefinitionKind::Layer => "layer",
    };
    if effects.get(kind, id).is_none() {
        return (!effects.is_missing(kind, id))
            .then(|| crate::Feature::Definition(format!("{what} {id}")));
    }
    matches!(effects.status(kind, id), Some(Status::NeedsFallback(_)))
        .then(|| crate::Feature::ShaderFallback(id.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_the_16_effects_and_4_transitions() {
        let set = builtin_definitions();
        let effects = set
            .definitions
            .iter()
            .filter(|d| d.kind == DefinitionKind::Effect)
            .count();
        let transitions = set
            .definitions
            .iter()
            .filter(|d| d.kind == DefinitionKind::Transition)
            .count();
        assert_eq!((effects, transitions), (16, 4));
        for d in &set.definitions {
            pm_protocol::validate::Validate::validate(d)
                .unwrap_or_else(|e| panic!("{}: {e:?}", d.id.as_str()));
        }
    }

    #[test]
    fn every_builtin_translates() {
        let mut effects = Effects::new("test");
        let failures = effects.set_snapshot(&builtin_definitions()).unwrap();
        assert!(failures.is_empty(), "{failures:#?}");
        for d in builtin_definitions().definitions {
            assert!(
                matches!(effects.status(d.kind, d.id.as_str()), Some(Status::Ready)),
                "{}",
                d.id.as_str()
            );
        }
    }
}
