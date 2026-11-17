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

mod brush;
pub(crate) mod exec;
pub mod fallback;
pub mod plan;
pub(crate) mod prewarm;
pub mod program;
pub mod resources;
pub(crate) mod retry;
pub mod versions;
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
    /// The shader compiler kept failing to build it (crashed, past its
    /// deadline, out of resources: [`retry::LIMIT`] of one kind), or cannot
    /// start: nothing about the source. Its layers are refused, never drawn
    /// without it and never kept, until a retry, a reload of its definitions
    /// or a build of the same source by another renderer.
    GaveUp(Failure),
}

struct Entry {
    /// The source hash the status belongs to.
    key: pm_glsl::Key,
    status: Status,
    prepared: Option<Arc<Prepared>>,
    /// Built on first use (the Metal compiler runs there).
    program: Option<GpuProgram>,
    /// The last build failed because of the compiler: built again after
    /// its backoff.
    retry: Option<retry::Retry>,
    /// Its shader addresses physical texels ([`physical_idiom`]).
    physical: bool,
}

/// Whether GLSL `source` addresses the physical raster: `texelFetch`,
/// `texelFetchOffset`, `textureSize` or `gl_FragCoord` outside comments.
/// Such a shader cannot be given composition pixels in place of the texels
/// it is drawn on, so its chain draws at the composition's density
/// (`crate::plan::comp_pass`).
pub(crate) fn physical_idiom(source: &str) -> bool {
    let mut code = String::with_capacity(source.len());
    let mut rest = source;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("//") {
            rest = r.find('\n').map_or("", |i| &r[i..]);
        } else if let Some(r) = rest.strip_prefix("/*") {
            rest = r.find("*/").map_or("", |i| &r[i + 2..]);
            code.push(' ');
        } else {
            let c = rest.chars().next().unwrap();
            code.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    // A fetch of the fragment's own texel (`texelFetch(t, ivec2(gl_FragCoord.xy),
    // 0)`) reads the same pixel at any density: it is not physical addressing.
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let mut own = 0;
    let mut at = 0;
    while let Some(i) = compact[at..].find("texelFetch(") {
        let start = at + i + "texelFetch(".len();
        let rest = &compact[start..];
        let name = rest.find(',').filter(|&n| {
            n > 0
                && rest[..n]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
        if let Some(n) = name
            && rest[n..].starts_with(",ivec2(gl_FragCoord.xy),0)")
        {
            own += 1;
        }
        at = start;
    }
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let count = |word: &str| {
        code.split(|c: char| !ident(c))
            .filter(|w| *w == word)
            .count()
    };
    count("texelFetch") > own
        || count("gl_FragCoord") > own
        || count("texelFetchOffset") > 0
        || count("textureSize") > 0
}

#[cfg(test)]
mod physical_tests {
    use super::physical_idiom;
    #[test]
    fn physical_addressing_is_found_and_own_pixel_reads_are_not() {
        assert!(physical_idiom(
            "o = texelFetch(u_tex, ivec2(v_px) + ivec2(1, 0), 0);"
        ));
        assert!(physical_idiom("ivec2 s = textureSize(u_tex, 0);"));
        assert!(physical_idiom("vec2 uv = gl_FragCoord.xy / u_res;"));
        assert!(!physical_idiom(
            "vec4 c = texelFetch( u_tex , ivec2(gl_FragCoord.xy), 0 );"
        ));
        assert!(!physical_idiom(
            "// texelFetch(u_tex, p, 0)\n/* textureSize */ o = texture(u_tex, v_st);"
        ));
        assert!(physical_idiom(
            "vec4 c = texelFetch(u_tex, ivec2(gl_FragCoord.xy), 0); vec4 d = texelFetch(u_tex, ivec2(gl_FragCoord.xy) + 1, 0);"
        ));
    }
}

/// The renderer's definitions and their programs.
pub struct Effects {
    /// A job's cancel flag: registration stops before the next definition
    /// (and so before another compiler call) once it is set.
    pub(crate) cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// The interactive viewer: a compile that cannot start now is done in
    /// the background, in its turn, and the frame stays pending meanwhile
    /// (no frame thread waits for a compiler slot).
    pub(crate) interactive: bool,
    pub(crate) file_app_version: Option<String>,
    pub(crate) versioned_builtins: bool,
    /// Viewer frames: an effect whose provider isn't installed is bypassed
    /// (its layer draws without it) instead of refusing the layer.
    pub(crate) bypass_missing: bool,
    bypassed: std::sync::Mutex<Vec<crate::LayerFailure>>,
    /// Neutral of a layer map: byte projects have a representable 128 of 255.
    pub(crate) map_midpoint: f64,
    set: DefinitionSet,
    /// Whether any definitions were given. Without them nothing is known
    /// missing either, and effects and transitions are routed.
    configured: bool,
    /// Ids an embedded set lists as missing (`DefinitionSetSnapshot`
    /// carries ids without kinds; contract 10, open question 3).
    missing_ids: std::collections::BTreeSet<DefinitionId>,
    entries: BTreeMap<(DefinitionKind, DefinitionId), Entry>,
    /// Consecutive passing compile failures (busy, slow, restarted compiler)
    /// of each definition's current source: asked again by a later
    /// registration once its backoff ran, given up as a visible refusal
    /// after [`limit`] of them.
    passing: BTreeMap<(DefinitionKind, DefinitionId), (pm_glsl::Key, retry::Retry)>,
    /// Shared by the renderers' definition sets ([`Effects::sibling`]).
    cache: Arc<pm_glsl::Cache>,
    context: String,
    pub watchdog: Watchdog,
    /// The frame's uniforms (draw quads and legacy programs' blocks), its
    /// ranges of shared chunks (a buffer per pass per frame before); here
    /// so the effect runner's mid-frame submit can flush them.
    pub(crate) uniforms: crate::exec::UniformPool,
    /// The frame runs in time slices (`crate::slices`).
    pub(crate) slices: Option<crate::slices::Slices>,
    /// Effects and transitions only the fallback runner draws
    /// ([`fallback`]).
    pub fallback: fallback::EffectFallback,
    /// What an empty texture unit samples: (0, 0, 0, 1), per format.
    dummies: std::collections::HashMap<wgpu::TextureFormat, pm_gpu::Target>,
    pub(crate) assets: resources::Assets,
    /// The effect types each subtree of the document uses
    /// ([`versions::for_document`]).
    pub(crate) effect_types: crate::doc_walk::SubtreeMemo<String>,
    mips: Option<crate::sampling::gpu::Resampler>,
    simple: std::collections::HashMap<(SimpleOp, wgpu::TextureFormat), wgpu::RenderPipeline>,
    /// Programs built ahead of their first frame (decision PROBE-1).
    prewarm: prewarm::Prewarm,
    /// Time frames spent building programs or waiting for builds in
    /// flight (µs; renderer telemetry).
    pub(crate) frame_build_us: u64,
    /// Tests: every program build fails with this, and builds are counted.
    #[cfg(test)]
    fail_builds: Option<Failure>,
    #[cfg(test)]
    builds: u32,
}

/// Per-pixel passes around legacy programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SimpleOp {
    /// `clamp: true`: the effect's output limited to 0..1.
    Clamp,
    /// Premultiplied linear light to premultiplied E (a legacy program's
    /// input with linear blending on).
    EncodeE,
    /// Premultiplied E back to premultiplied linear light (its output).
    ///
    /// Legacy programs write alpha above 1 (glow adds its light over
    /// `u_orig`, so alpha reaches 1 + intensity) and color with no alpha
    /// (emitted light). Both passes divide by alpha only within (floor, 1];
    /// elsewhere the premultiplied color is light and is converted per
    /// channel, as if its alpha were 1 (the frame's output encode does the
    /// same, `composite/ops.wgsl`). Where no linear-light blend touches the
    /// pixel (over black or transparency) it then displays the E value it
    /// displays in Standard.
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
  return store(clamp(texel(p), vec4<f32>(0.0), vec4<f32>(1.0)));
}
fn coverage(a: f32) -> bool {
  return pm_may_unpremultiply(a) && a <= 1.0;
}
@fragment fn encode(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
  let c = texel(p);
  if (coverage(c.a)) {
    return store(pm_premultiplied_linear_to_e(c));
  }
  return store(vec4<f32>(pm_e_encode3(c.rgb), c.a));
}
@fragment fn decode(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
  let c = texel(p);
  if (coverage(c.a)) {
    return store(pm_premultiplied_e_to_linear(c));
  }
  return store(vec4<f32>(pm_e_decode3(c.rgb), c.a));
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

pub(crate) fn diagnostics(failure: &Failure) -> Vec<ShaderDiagnostic> {
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

pub(crate) fn truncate(mut message: String) -> String {
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

/// Failures of one kind (crash, deadline, resources) of one unchanged
/// source before the compiler is given up on for it. A busy compiler is no
/// failure: the request waits its turn ([`pm_glsl::Admission`]).
pub const PASSING_LIMIT: u32 = retry::LIMIT;

/// The definition-scoped error of a program still being built after a
/// passing compiler failure: retried (never a bypass), its own diagnostic
/// kept.
pub fn pending_error(id: &DefinitionId, failure: &Failure) -> Option<EngineError> {
    let message = truncate(format!(
        "The shader of {} {} ({}: {}).",
        id.as_str(),
        crate::capability::PROGRAM_PENDING,
        failure.stage.name(),
        failure.message.lines().next().unwrap_or_default()
    ));
    Some(
        EngineError::new(
            ErrorCode::NotHeadlessRenderable,
            ErrorScope::Definition {
                definition: id.clone(),
            },
            message,
        )
        .with_retry(Retry::After { ms: 500 }),
    )
}

/// The definition-scoped error a failure is reported as (contract 7).
pub fn definition_error(id: &DefinitionId, status: &Status) -> Option<EngineError> {
    let (code, failure, what) = match status {
        Status::Ready => return None,
        Status::Refused(f) => (ErrorCode::ShaderCompile, f, "does not compile"),
        Status::GaveUp(f) => (
            ErrorCode::NotHeadlessRenderable,
            f,
            crate::capability::COMPILER_GAVE_UP,
        ),
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
    pub(crate) fn take_bypassed(&self) -> Vec<crate::LayerFailure> {
        std::mem::take(&mut *self.bypassed.lock().unwrap())
    }

    fn report_bypassed(
        &self,
        composition: String,
        layer: &pm_model::Node,
        effect: &str,
        feature: &crate::Feature,
    ) {
        self.bypassed.lock().unwrap().push(crate::LayerFailure {
            composition,
            layer: crate::js::layer_id(layer),
            message: format!("effect {effect} bypassed: {feature}"),
        });
    }

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
            assets: Default::default(),
            effect_types: Default::default(),
            mips: None,
            file_app_version: None,
            versioned_builtins: false,
            bypass_missing: false,
            bypassed: Default::default(),
            map_midpoint: 0.5,
            set: DefinitionSet::default(),
            configured: false,
            missing_ids: Default::default(),
            entries: BTreeMap::new(),
            passing: BTreeMap::new(),
            cancel: None,
            interactive: false,
            cache,
            context,
            watchdog: Watchdog::default(),
            uniforms: crate::exec::UniformPool::default(),
            slices: None,
            fallback: Default::default(),
            dummies: Default::default(),
            simple: Default::default(),
            prewarm: Default::default(),
            frame_build_us: 0,
            #[cfg(test)]
            fail_builds: None,
            #[cfg(test)]
            builds: 0,
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
    ///
    /// A delta's upserts are registrations (a register or a hot reload of
    /// that id): each starts with a fresh watchdog record, so an effect the
    /// watchdog turned off runs again even when its source is unchanged. A
    /// full update (session start, after a refused push) re-sends the set
    /// and keeps the records of unchanged definitions.
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
            if self.cancelled() {
                break;
            }
            if !update.full {
                self.watchdog.forget(snapshot.kind, &snapshot.id);
                // Sent again on its own (reloaded, retried): a compiler that
                // gave up on it is asked afresh.
                let key = (snapshot.kind, snapshot.id.clone());
                self.passing.remove(&key);
                if matches!(
                    self.entries.get(&key).map(|e| &e.status),
                    Some(Status::GaveUp(_))
                ) {
                    self.entries.remove(&key);
                }
            }
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
        self.versioned_builtins = false;
        self.file_app_version = None;
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
        // Definitions sent again (the app's resources, an extension
        // reloaded): one the compiler gave up on is asked afresh.
        self.retry_given_up();
        let mut failures = Vec::new();
        for snapshot in &snapshot.definitions {
            if self.cancelled() {
                break;
            }
            if let Some(e) = self.translate(snapshot) {
                failures.push(e);
            }
        }
        Ok(failures)
    }

    /// Asks the compiler afresh for every definition it gave up on (an
    /// explicit retry; definitions sent again do the same). Returns how many.
    pub fn retry_given_up(&mut self) -> usize {
        let given_up: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, e)| matches!(e.status, Status::GaveUp(_)))
            .map(|(k, _)| k.clone())
            .collect();
        for key in &given_up {
            self.entries.remove(key);
            self.passing.remove(key);
        }
        given_up.len()
    }

    /// How a compiler request of this renderer is admitted: a job waits its
    /// turn until cancelled; the interactive viewer does not wait on its
    /// frame thread; anything else waits its turn.
    fn admission(&self) -> pm_glsl::Admission {
        match &self.cancel {
            Some(cancel) => pm_glsl::Admission::WaitUnless(cancel.clone()),
            None if self.interactive => pm_glsl::Admission::Now,
            None => pm_glsl::Admission::Wait,
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|c| c.load(std::sync::atomic::Ordering::Acquire))
    }

    /// Drops the entries `gone` selects, with their watchdog records.
    fn drop_entries(&mut self, gone: impl Fn(&(DefinitionKind, DefinitionId)) -> bool) {
        let removed: Vec<_> = self.entries.keys().filter(|k| gone(k)).cloned().collect();
        for key in removed {
            self.entries.remove(&key);
            self.watchdog.forget(key.0, &key.1);
        }
        self.passing.retain(|k, _| !gone(k));
    }

    /// Forgets every definition: effects and transitions are routed again.
    pub fn clear(&mut self) {
        self.set = DefinitionSet::default();
        self.configured = false;
        self.missing_ids.clear();
        self.entries.clear();
        self.passing.clear();
        self.watchdog.reset();
        self.prewarm = Default::default();
        self.uniforms = Default::default();
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
            // Given up on, but translated since (another renderer, in its
            // turn): asked again.
            let recovered = matches!(entry.status, Status::GaveUp(_))
                && self.cache.peek(source_key).is_some_and(|e| e.is_ok());
            if !recovered {
                return definition_error(&snapshot.id, &entry.status);
            }
            self.entries.remove(&key);
            self.passing.remove(&key);
        }
        // Still in its backoff: pending without starting the compiler again.
        if let Some((k, retry)) = self.passing.get(&key)
            && *k == source_key
            && let Some(f) = retry.waiting()
        {
            return pending_error(&snapshot.id, f);
        }
        // Being translated in the background, in its turn.
        if self.cache.in_background(source_key) {
            return pending_error(
                &snapshot.id,
                &Failure::new(pm_glsl::FailureStage::CompilerBusy, pm_glsl::QUEUED),
            );
        }
        #[cfg(any(test, feature = "test-hooks"))]
        let injected = crate::test_hooks::next_compile(snapshot.id.as_str());
        #[cfg(not(any(test, feature = "test-hooks")))]
        let injected: Option<Failure> = None;
        // A background translation that failed for a passing reason counts
        // once, like one made here.
        let landed = self.cache.take_landed(source_key);
        let prepared = match injected.or(landed) {
            Some(f) => Arc::new(Err(f)),
            None => pm_glsl::with_admission(self.admission(), || {
                self.cache.prepare(&glsl.source, glsl.body_line_offset)
            }),
        };
        // Every compiler slot taken: no failure. The definition is pending
        // and builds in its turn (the viewer's in the background; a job
        // waited and was cancelled). Never counted, never remembered.
        if let Err(f) = prepared.as_ref()
            && f.admission()
        {
            if f.message == pm_glsl::QUEUED {
                self.cache
                    .prepare_in_background(&glsl.source, glsl.body_line_offset);
            }
            self.entries.remove(&key);
            return pending_error(&snapshot.id, f);
        }
        // A crashed, slow or starved compiler says nothing about the source:
        // pending, asked again after its backoff, until one kind of failure
        // used up its budget or the compiler cannot start at all: then given
        // up (a terminal refusal with the compiler's own message).
        if let Err(f) = prepared.as_ref()
            && f.stage.retryable()
            && !f.compiler_unavailable()
        {
            let previous = self
                .passing
                .get(&key)
                .filter(|(k, _)| *k == source_key)
                .map(|(_, r)| r);
            let retry = retry::Retry::after(previous, f.clone());
            if !retry.exhausted() {
                self.entries.remove(&key);
                self.passing.insert(key, (source_key, retry));
                return pending_error(&snapshot.id, f);
            }
        }
        self.passing.remove(&key);
        let (status, prepared) = match prepared.as_ref() {
            Ok(p) => (Status::Ready, Some(Arc::new(p.clone()))),
            Err(f) if f.stage.webgl_refuses() => (Status::Refused(f.clone()), None),
            Err(f) if f.stage.retryable() => (Status::GaveUp(f.clone()), None),
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
                retry: None,
                physical: physical_idiom(&glsl.source),
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

    /// Keeps translated sources and per-definition policy while releasing
    /// resources bound to a retiring GPU device. A successful CPU translation
    /// whose GPU pipeline failed may be tried again on the next device.
    pub fn release_device(&mut self) -> bool {
        if !self.watchdog.release_device() {
            return false;
        }
        for entry in self.entries.values_mut() {
            entry.program = None;
            if entry.prepared.is_some() && matches!(entry.status, Status::NeedsFallback(_)) {
                entry.status = Status::Ready;
            }
        }
        self.prewarm = Default::default();
        self.dummies.clear();
        self.simple.clear();
        true
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
                    // `store`: the nearest half on RGBA16F (Metal's
                    // conversion truncates), the value itself otherwise.
                    format!(
                        "{}\n{}\nfn store(c: vec4<f32>) -> vec4<f32> {{ return {}; }}\n{SIMPLE_WGSL}",
                        pm_color::wgsl::COLOR,
                        crate::nearest_half::NEAREST_HALF_WGSL,
                        if format == wgpu::TextureFormat::Rgba16Float {
                            "pm_scene_half_color(c)"
                        } else {
                            "c"
                        }
                    )
                    .into(),
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

    /// Puts programs the worker built into their definitions (decision
    /// PROBE-1). A build of a source that changed meanwhile is dropped; a
    /// build the device refused turns the definition into a fallback one,
    /// as a failed build on first use does.
    fn absorb(&mut self, built: Vec<prewarm::Built>) {
        for b in built {
            let Some(entry) = self.entries.get_mut(&b.key) else {
                continue;
            };
            if entry.key != b.source
                || entry.program.is_some()
                || !matches!(entry.status, Status::Ready)
            {
                continue;
            }
            match b.result {
                Ok(program) => {
                    self.cache.note_built(entry.key);
                    entry.program = Some(program);
                    entry.retry = None;
                }
                // Cancelled while it waited its turn: built later.
                Err(f) if f.admission() => {}
                Err(f) => settle_build_failure(entry, f),
            }
        }
    }

    /// Queues the programs of `first` (the definitions a document uses),
    /// then every other user definition, to be built off the frame thread
    /// for `format` (decision PROBE-1). Programs already built or in flight
    /// are skipped. Returns how many were queued.
    pub fn prewarm(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        first: &[(DefinitionKind, DefinitionId)],
    ) -> usize {
        let done = self.prewarm.take();
        self.absorb(done);
        let mut order: Vec<(DefinitionKind, DefinitionId)> = first
            .iter()
            .filter(|k| self.entries.contains_key(*k))
            .cloned()
            .collect();
        for key in self.entries.keys() {
            let user = self
                .get(key.0, key.1.as_str())
                .is_some_and(|d| d.origin != pm_protocol::definitions::DefinitionOrigin::BuiltIn);
            if user && !order.contains(key) {
                order.push(key.clone());
            }
        }
        let mut queued = 0;
        for key in order {
            let Some(entry) = self.entries.get(&key) else {
                continue;
            };
            let (Status::Ready, None, Some(prepared)) =
                (&entry.status, &entry.program, &entry.prepared)
            else {
                continue;
            };
            // Its backoff runs: not built again before it ends.
            if entry.retry.as_ref().is_some_and(|r| r.waiting().is_some()) {
                continue;
            }
            if self
                .prewarm
                .queue(key.clone(), entry.key, prepared.clone(), device, format)
            {
                queued += 1;
            }
        }
        queued
    }

    /// Programs built ahead: (built, their µs, frames that waited, their µs,
    /// still in flight).
    pub fn prewarm_stats(&self) -> (u64, u64, u64, u64, usize) {
        let p = &self.prewarm;
        (p.built, p.built_us, p.waited, p.waited_us, p.pending())
    }

    /// Time spent building or awaiting native programs since the previous
    /// sample, in microseconds. Call from the renderer's worker.
    pub fn take_program_build_us(&mut self) -> u64 {
        std::mem::take(&mut self.frame_build_us)
    }

    /// Builds a ready definition's GPU pipeline for the working format, as
    /// its first frame would (the Metal compiler runs here), and returns its
    /// definition-scoped failure, if any: `ShaderCompile` for a program WebGL
    /// refuses too, `NotHeadlessRenderable` for one only the fallback runner
    /// draws (a device refusal turns a ready definition into that). What
    /// `validate_effect` and the job engine's acknowledgment report.
    pub fn build(
        &mut self,
        device: &wgpu::Device,
        kind: DefinitionKind,
        id: &str,
    ) -> Option<EngineError> {
        let _ = self.program(device, kind, id, wgpu::TextureFormat::Rgba16Float);
        self.status(kind, id)
            .and_then(|status| definition_error(&DefinitionId::from(id), status))
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
        let key = (kind, DefinitionId::from(id));
        let cache = self.cache.clone();
        let entry = self.entries.get_mut(&key).ok_or_else(|| {
            Failure::new(
                pm_glsl::FailureStage::Rewrite,
                format!("{id} has no program"),
            )
        })?;
        match &entry.status {
            Status::Ready => {}
            // Given up on, but built since by another renderer: built again.
            Status::GaveUp(_) if entry.prepared.is_some() && cache.was_built(entry.key) => {
                entry.status = Status::Ready;
                entry.retry = None;
            }
            Status::Refused(f) | Status::NeedsFallback(f) | Status::GaveUp(f) => {
                return Err(f.clone());
            }
        }
        pm_perf::cache(
            pm_perf::Counter::ProgramCacheHits,
            pm_perf::Counter::ProgramCacheMisses,
            entry.program.is_some(),
        );
        let building = entry.program.is_none().then(std::time::Instant::now);
        let _span = entry
            .program
            .is_none()
            .then(|| pm_perf::Span::new(pm_perf::Counter::ProgramMicros));
        if entry.program.is_none() {
            // Built ahead (decision PROBE-1): taken if it finished; waited
            // for if it is being built now, except on the interactive
            // viewer's frame thread, whose frame stays pending meanwhile.
            let done = if self.prewarm.in_flight(&key) && !self.prewarm.reclaim(&key) {
                if self.interactive {
                    let done = self.prewarm.take();
                    self.absorb(done);
                    if self.prewarm.in_flight(&key) {
                        return Err(Failure::new(
                            pm_glsl::FailureStage::CompilerBusy,
                            pm_glsl::QUEUED,
                        ));
                    }
                    Vec::new()
                } else {
                    self.prewarm.wait_for(&key)
                }
            } else {
                self.prewarm.take()
            };
            self.absorb(done);
        }
        let admission = self.admission();
        let interactive = self.interactive;
        let entry = self.entries.get_mut(&key).expect("looked up above");
        if let Status::Refused(f) | Status::NeedsFallback(f) | Status::GaveUp(f) = &entry.status {
            return Err(f.clone());
        }
        if entry.program.is_none() {
            if let Some(f) = entry.retry.as_ref().and_then(retry::Retry::waiting) {
                return Err(f.clone());
            }
            #[cfg(test)]
            let forced = {
                self.builds += 1;
                self.fail_builds
                    .clone()
                    .or_else(|| crate::test_hooks::next_build(id))
            };
            #[cfg(all(not(test), feature = "test-hooks"))]
            let forced = crate::test_hooks::next_build(id);
            #[cfg(not(any(test, feature = "test-hooks")))]
            let forced: Option<Failure> = None;
            let entry = self.entries.get_mut(&key).expect("looked up above");
            let built = match forced {
                Some(f) => Err(f),
                None => {
                    let prepared = entry.prepared.clone().ok_or_else(|| {
                        Failure::new(pm_glsl::FailureStage::Rewrite, "no prepared program")
                    })?;
                    pm_glsl::with_admission(admission, || {
                        prepared
                            .finish()
                            .and_then(|p| GpuProgram::new(device, p, id))
                    })
                }
            };
            match built {
                Ok(p) => {
                    cache.note_built(entry.key);
                    entry.program = Some(p);
                    entry.retry = None;
                }
                // Every compiler slot taken: no failure. The viewer's build
                // goes to the background, in its turn; a job was cancelled.
                Err(f) if f.admission() => {
                    if interactive
                        && f.message == pm_glsl::QUEUED
                        && let Some(prepared) = entry.prepared.clone()
                    {
                        let source = entry.key;
                        self.prewarm
                            .queue(key.clone(), source, prepared, device, format);
                    }
                    return Err(f);
                }
                Err(f) => {
                    settle_build_failure(entry, f.clone());
                    return Err(f);
                }
            }
        }
        let program = entry_program(&mut self.entries, &key);
        let piped = program.pipeline(device, format).map(|_| ());
        if let Some(at) = building {
            self.frame_build_us += at.elapsed().as_micros() as u64;
        }
        let entry = self.entries.get_mut(&key).expect("looked up above");
        if let Err(f) = piped {
            entry.status = Status::NeedsFallback(f.clone());
            entry.program = None;
            return Err(f);
        }
        Ok(entry.program.as_mut().expect("still built"))
    }
}

fn entry_program<'a>(
    entries: &'a mut BTreeMap<(DefinitionKind, DefinitionId), Entry>,
    key: &(DefinitionKind, DefinitionId),
) -> &'a mut GpuProgram {
    entries
        .get_mut(key)
        .and_then(|e| e.program.as_mut())
        .expect("built above")
}

/// A program build that failed: a failure of its source is remembered (a
/// fallback); a crashed, slow or starved compiler is retried after its
/// backoff, and given up on once one kind used up its budget or when it
/// cannot start.
fn settle_build_failure(entry: &mut Entry, f: Failure) {
    if remembered(&f) {
        entry.status = Status::NeedsFallback(f);
        entry.retry = None;
        return;
    }
    let retry = retry::Retry::after(entry.retry.as_ref(), f.clone());
    if f.compiler_unavailable() || retry.exhausted() {
        entry.status = Status::GaveUp(f);
        entry.retry = None;
    } else {
        entry.retry = Some(retry);
    }
}

/// Whether a program build's failure belongs to the definition (remembered
/// until it changes) or only to this attempt: a busy, crashed or timed-out
/// compiler says nothing about the source, and remembering it kept a
/// healthy project "waiting for a shader compiler slot" for good.
fn remembered(f: &Failure) -> bool {
    !f.stage.retryable()
}

impl crate::composite::EffectHost for Effects {
    fn prepare<'d>(
        &self,
        scope: &mut pm_eval::Scope<'_, 'd>,
        layer: &'d pm_model::Node,
        frame: crate::composite::effects::EffectFrame,
    ) -> pm_eval::JsResult<Result<Option<crate::composite::PreparedEffects>, crate::Feature>> {
        self.prepare_prefix(scope, layer, frame, None)
    }
    fn prepare_source<'d>(
        &self,
        scope: &mut pm_eval::Scope<'_, 'd>,
        layer: &'d pm_model::Node,
        frame: crate::composite::effects::EffectFrame,
        prefix: Option<usize>,
        mut raster: crate::composite::effects::SourceRaster,
        grow: bool,
    ) -> pm_eval::JsResult<
        Result<
            (
                Option<crate::composite::PreparedEffects>,
                crate::composite::effects::SourceRaster,
            ),
            crate::Feature,
        >,
    > {
        let mut steps = match plan::effect_steps_raster(
            scope,
            self,
            layer,
            frame.time,
            (1., 1.),
            prefix,
            Some(raster),
            None,
        )? {
            Ok(s) => s,
            Err(f) => return Ok(Err(f)),
        };
        let failure = |s: &str| crate::Feature::Definition(s.into());
        let mut domains = vec![];
        for step in &steps {
            let Some(def) = versions::main_definition(&step.definition) else {
                return Ok(Err(failure("Effect has no source processing contract")));
            };
            let Some(contract) = def.metadata.get("sourceProcessing") else {
                return Ok(Err(failure(
                    "Effect does not declare source processing support",
                )));
            };
            if step.backdrop || step.temporal || contract.get("geometry").is_some() {
                return Ok(Err(failure(
                    "Source raster backdrop/temporal/brush domain adapter unavailable",
                )));
            }
            let value = |key: &str| {
                step.params
                    .iter()
                    .find(|p| p.key == key)
                    .map_or(0., |p| match p.value {
                        plan::Binding::Float(v) => v,
                        _ => 0.,
                    })
            };
            let enabled = contract
                .get("when")
                .and_then(|v| v.as_str())
                .is_none_or(|k| value(k) != 0.);
            let pad = if grow && enabled {
                (contract
                    .get("pixels")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.)
                    + contract
                        .get("padding")
                        .and_then(|v| v.as_array())
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.as_str())
                        .map(|k| value(k).abs())
                        .sum::<f64>()
                        * contract
                            .get("factor")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(1.))
                .ceil()
            } else {
                0.
            };
            raster.extent = match raster.extent.pad(pad) {
                Ok(e) => e,
                Err(e) => return Ok(Err(failure(&e.to_string()))),
            };
            domains.push(raster.extent);
        }
        let (w, h) = match raster.size() {
            Ok(s) => s,
            Err(e) => return Ok(Err(e)),
        };
        let owner = std::sync::Arc::new(raster.owner());
        for (step, domain) in steps.iter_mut().zip(domains) {
            step.owner = Some(owner.clone());
            step.density = owner.pixel_scale;
            step.source_domain = Some((domain, raster.extent));
            if let Some(spatial) = &mut step.spatial {
                let def = versions::main_definition(&step.definition).unwrap();
                if let Some(sampling) = def.metadata.get("sampling") {
                    let radius = step
                        .params
                        .iter()
                        .find(|p| p.key == sampling["radius"].as_str().unwrap())
                        .map_or(0., |p| match p.value {
                            plan::Binding::Float(v) => v,
                            _ => 0.,
                        });
                    spatial.lod = pm_fx_defs::spatial::sample_lod(
                        radius,
                        sampling["support"].as_f64().unwrap(),
                        sampling["maxRadius"].as_f64().unwrap(),
                        owner.pixel_scale,
                        w,
                        h,
                    )
                    .unwrap();
                } else {
                    spatial.lod = w.max(h).ilog2();
                }
            }
        }
        let prepared = if steps.is_empty() {
            None
        } else {
            let sources = steps
                .iter()
                .map(|s| self.source_key(DefinitionKind::Effect, &s.definition))
                .collect();
            Some(crate::composite::PreparedEffects {
                padding: crate::composite::Padding::Full,
                backdrop: false,
                passes: exec::chain_passes(exec::Chain {
                    inputs: vec![],
                    layer: crate::js::layer_id(layer),
                    steps,
                    time: frame.time,
                    sources,
                }),
            })
        };
        Ok(Ok((prepared, raster)))
    }
    fn prepare_prefix<'d>(
        &self,
        scope: &mut pm_eval::Scope<'_, 'd>,
        layer: &'d pm_model::Node,
        frame: crate::composite::effects::EffectFrame,
        prefix: Option<usize>,
    ) -> pm_eval::JsResult<Result<Option<crate::composite::PreparedEffects>, crate::Feature>> {
        self.prepare_measured(scope, layer, frame, prefix, None)
    }

    fn take_bypassed(&self) -> Vec<crate::LayerFailure> {
        Effects::take_bypassed(self)
    }

    fn bypasses_refusals(&self) -> bool {
        self.bypass_missing
    }

    fn prepare_measured<'d>(
        &self,
        scope: &mut pm_eval::Scope<'_, 'd>,
        layer: &'d pm_model::Node,
        frame: crate::composite::effects::EffectFrame,
        prefix: Option<usize>,
        measure: Option<&mut dyn pm_eval::text_geom::TextMeasure>,
    ) -> pm_eval::JsResult<Result<Option<crate::composite::PreparedEffects>, crate::Feature>> {
        let mut steps = match plan::effect_steps_frame(
            scope,
            self,
            layer,
            frame.time,
            frame.scale,
            prefix,
            None,
            measure,
            frame.owner,
        )? {
            Ok(steps) => steps,
            Err(f) => return Ok(Err(f)),
        };
        // A viewer frame at a reduced (or raised) resolution is the full
        // picture resampled: an extension's cells, radii and offsets in
        // pixels keep their composition size. Only final output keeps
        // Electron's output raster.
        if !frame.final_output {
            for step in &mut steps {
                step.raster_space = false;
            }
        }
        if steps.is_empty() {
            return Ok(Ok(None));
        }
        let sources = steps
            .iter()
            .map(|s| self.source_key(DefinitionKind::Effect, &s.definition))
            .collect();
        Ok(Ok(Some(crate::composite::PreparedEffects {
            padding: plan::region_padding(&steps),
            backdrop: steps.iter().any(|s| s.backdrop),
            passes: exec::chain_passes(exec::Chain {
                inputs: vec![],
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

    fn needs_composition_density<'d>(
        &self,
        scope: &mut pm_eval::Scope<'_, 'd>,
        layer: &'d pm_model::Node,
        time: f64,
        final_output: bool,
    ) -> pm_eval::JsResult<bool> {
        use pm_eval::node;
        let needs = |kind: DefinitionKind, ty: &str| {
            let Some(def) = self.get(kind, ty) else {
                return false;
            };
            // Electron extensions run on the requested output grid in final
            // output, including physical texels. In a viewer frame one that
            // addresses physical texels cannot follow the frame's density,
            // so its level draws at the composition's (Full) and resamples.
            if kind == DefinitionKind::Effect
                && def.origin != pm_protocol::definitions::DefinitionOrigin::BuiltIn
            {
                return !final_output
                    && self
                        .entries
                        .get(&(kind, DefinitionId::from(ty)))
                        .is_some_and(|e| e.physical);
            }
            // Owner-bound main definitions scale their own kernels.
            let owned = kind == DefinitionKind::Effect
                && self.versioned_builtins
                && def.glsl.as_ref().is_some_and(|g| {
                    g.source.contains("u_layerUV") || g.source.contains("u_temporalOwnerUV")
                });
            if owned {
                return false;
            }
            final_output
                || self
                    .entries
                    .get(&(kind, DefinitionId::from(ty)))
                    .is_some_and(|e| e.physical)
        };
        if scope.has_renderable_effects(layer, time)? {
            for fx in node::array_or_empty(node::member(Some(layer), "fx"))? {
                if node::member(Some(fx), "on").is_some_and(|on| !node::truthy(Some(on))) {
                    continue;
                }
                if let Some(ty) = node::string(node::member(Some(fx), "type"))
                    && needs(DefinitionKind::Effect, &ty.to_string_lossy())
                {
                    return Ok(true);
                }
            }
        }
        for key in ["transitionIn", "transitionOut"] {
            if let Some(ty) = node::string(node::member(node::member(Some(layer), key), "type"))
                && needs(DefinitionKind::Transition, &ty.to_string_lossy())
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn layer_definitions(&self) -> Option<&Effects> {
        Some(self)
    }

    fn prepare_transition<'d>(
        &self,
        scope: &mut pm_eval::Scope<'_, 'd>,
        layer: &'d pm_model::Node,
        frame: crate::composite::effects::EffectFrame,
    ) -> pm_eval::JsResult<Result<Option<crate::composite::PreparedTransition>, crate::Feature>>
    {
        let step = match plan::transition_at(scope, self, layer, frame.time)? {
            Ok(Some(step)) => plan::TransitionStep {
                density: [frame.scale.0.abs(), frame.scale.1.abs()],
                ..step
            },
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
        // Switched off for good (not animated): it never draws.
        let on = node::member(Some(fx), "on");
        let off = match on.map(pm_model::Node::kind) {
            Some(pm_model::NodeKind::Bool(false)) => true,
            _ if node::is_property(on) => {
                !node::has_kf_array(on)
                    && matches!(
                        node::member(on, "v").map(pm_model::Node::kind),
                        Some(pm_model::NodeKind::Bool(false))
                    )
            }
            _ => false,
        };
        if off {
            continue;
        }
        let ty = node::to_js_string(node::member(Some(fx), "type")).to_string_lossy();
        // The planner's bypass ([`Effects::bypass_missing`]): no provider, so
        // nothing to route; the layer draws without it.
        if effects.bypass_missing && effects.get(DefinitionKind::Effect, &ty).is_none() {
            continue;
        }
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

/// The feature a definition the compiler gave up on refuses its layers with,
/// carrying the compiler's own message.
pub(crate) fn gave_up(id: &str, f: &Failure) -> crate::Feature {
    crate::Feature::CompilerGaveUp(
        id.to_owned(),
        format!(
            "{}: {}",
            f.stage.name(),
            f.message.lines().next().unwrap_or_default()
        ),
    )
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
    match effects.status(kind, id) {
        Some(Status::NeedsFallback(_)) => Some(crate::Feature::ShaderFallback(id.to_owned())),
        Some(Status::GaveUp(f)) => Some(gave_up(id, f)),
        _ => None,
    }
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

    /// A build refused because every compiler slot was busy (prewarm at
    /// launch, many extensions) leaves the definition ready, to be built by
    /// the first frame that draws it; a failure of the source is kept.
    #[test]
    fn a_busy_compiler_is_never_remembered_for_a_definition() {
        let mut effects = Effects::new("test");
        effects.set_snapshot(&builtin_definitions()).unwrap();
        let def = builtin_definitions()
            .definitions
            .into_iter()
            .find(|d| d.kind == DefinitionKind::Effect)
            .unwrap();
        let key = (def.kind, def.id.clone());
        let source = effects.entries[&key].key;
        let built = |result| prewarm::Built {
            key: key.clone(),
            source,
            result,
            us: 0,
        };
        let busy = built(Err(Failure::new(
            pm_glsl::FailureStage::CompilerBusy,
            "all GLSL compiler process slots are occupied; retry later",
        )));
        effects.absorb(vec![busy]);
        assert!(matches!(
            effects.status(def.kind, def.id.as_str()),
            Some(Status::Ready)
        ));
        let broken = built(Err(Failure::new(
            pm_glsl::FailureStage::NagaValidate,
            "the source does not validate",
        )));
        effects.absorb(vec![broken]);
        assert!(matches!(
            effects.status(def.kind, def.id.as_str()),
            Some(Status::NeedsFallback(_))
        ));
    }

    /// Draws during a busy compiler build once per backoff window (100 ms,
    /// then 200 ms), not once per draw, and a build that works ends it.
    #[test]
    fn draws_during_a_busy_compiler_build_once_per_backoff_window() {
        use std::time::Duration;
        let gpu = pm_gpu::Gpu::new("build backoff test").unwrap();
        let format = wgpu::TextureFormat::Rgba16Float;
        let mut effects = Effects::new("test");
        effects.set_snapshot(&builtin_definitions()).unwrap();
        let def = builtin_definitions()
            .definitions
            .into_iter()
            .find(|d| d.kind == DefinitionKind::Effect)
            .unwrap();
        effects.fail_builds = Some(Failure::new(
            pm_glsl::FailureStage::CompilerBusy,
            "all GLSL compiler process slots are occupied; retry later",
        ));
        let draw = |effects: &mut Effects, n: usize| {
            for _ in 0..n {
                assert!(
                    effects
                        .program(&gpu.device, def.kind, def.id.as_str(), format)
                        .is_err()
                );
            }
        };
        draw(&mut effects, 30);
        assert_eq!(effects.builds, 1, "one build in the first window");
        std::thread::sleep(Duration::from_millis(120));
        draw(&mut effects, 30);
        assert_eq!(effects.builds, 2);
        std::thread::sleep(Duration::from_millis(120));
        draw(&mut effects, 30);
        assert_eq!(effects.builds, 2, "the second window is 200 ms");
        std::thread::sleep(Duration::from_millis(100));
        draw(&mut effects, 30);
        assert_eq!(effects.builds, 3);
        assert!(matches!(
            effects.status(def.kind, def.id.as_str()),
            Some(Status::Ready)
        ));
        // The compiler is back: the next build after the window works.
        effects.fail_builds = None;
        std::thread::sleep(Duration::from_millis(420));
        effects
            .program(&gpu.device, def.kind, def.id.as_str(), format)
            .unwrap();
        assert_eq!(effects.builds, 4);
    }

    /// One contributed effect named `id` (a built-in's program), alone in
    /// its set: injected failures (`test_hooks`) are keyed by it.
    fn one(id: &str) -> DefinitionSetSnapshot {
        let mut set = builtin_definitions();
        let mut def = set
            .definitions
            .into_iter()
            .find(|d| d.kind == DefinitionKind::Effect)
            .unwrap();
        def.id = id.into();
        set.definitions = vec![def];
        set
    }
    fn expire(effects: &mut Effects) {
        for (_, retry) in effects.passing.values_mut() {
            retry.expire();
        }
        for entry in effects.entries.values_mut() {
            if let Some(retry) = &mut entry.retry {
                retry.expire();
            }
        }
    }
    fn failure(stage: pm_glsl::FailureStage, message: &str) -> Failure {
        Failure::new(stage, message)
    }

    /// A compiler that keeps crashing, timing out or running out of
    /// resources on one unchanged source is asked a bounded number of
    /// times, each a pending registration, then gives up in a visible
    /// refusal with its own diagnostic; nothing asks it again by itself.
    #[test]
    fn a_compiler_that_keeps_failing_one_source_ends_in_a_refusal() {
        use pm_glsl::FailureStage::*;
        for stage in [CompilerProcess, Deadline, CompilerResources] {
            let id = format!("test.keeps-failing.{}", stage.name());
            let set = one(&id);
            let said = format!("the compiler said {}", stage.name());
            crate::test_hooks::fail_compiles(&id, vec![failure(stage, &said); 8]);
            let mut effects = Effects::new("test");
            for attempt in 1..PASSING_LIMIT {
                let failures = effects.set_snapshot(&set).unwrap();
                assert!(
                    failures[0].message.contains(crate::PROGRAM_PENDING)
                        && failures[0].message.contains(&said),
                    "attempt {attempt}: {failures:?}"
                );
                assert!(effects.status(DefinitionKind::Effect, &id).is_none());
                // Asked again within the backoff: no compiler call.
                effects.set_snapshot(&set).unwrap();
                assert_eq!(crate::test_hooks::left(&id).0, 8 - attempt as usize);
                expire(&mut effects);
            }
            let failures = effects.set_snapshot(&set).unwrap();
            assert!(
                !failures[0].message.contains(crate::PROGRAM_PENDING)
                    && failures[0].message.contains(&said)
                    && failures[0].message.contains(stage.name()),
                "{failures:?}"
            );
            assert!(matches!(
                effects.status(DefinitionKind::Effect, &id),
                Some(Status::NeedsFallback(f)) if f.stage == stage
            ));
            let left = crate::test_hooks::left(&id).0;
            for _ in 0..5 {
                expire(&mut effects);
                effects.set_snapshot(&set).unwrap();
            }
            assert_eq!(
                crate::test_hooks::left(&id).0,
                left,
                "{stage:?} asked again"
            );
            // Sent again on its own (an explicit retry): asked afresh.
            effects
                .apply(&DefinitionsUpdate {
                    generation: Seq::new(2).unwrap(),
                    full: false,
                    upserts: set.definitions.clone(),
                    removals: vec![],
                })
                .unwrap();
            assert_eq!(crate::test_hooks::left(&id).0, left - 1);
        }
    }

    /// A compiler that cannot be started is final at once, for a
    /// registration and for a program build.
    #[test]
    fn an_unstartable_compiler_is_refused_at_once() {
        let gpu = pm_gpu::Gpu::new("unstartable compiler test").unwrap();
        let gone = failure(
            pm_glsl::FailureStage::CompilerProcess,
            &format!("{}: No such file or directory", pm_glsl::CANNOT_START),
        );
        let id = "test.unstartable.compile";
        crate::test_hooks::fail_compiles(id, vec![gone.clone()]);
        let mut effects = Effects::new("test");
        let failures = effects.set_snapshot(&one(id)).unwrap();
        assert!(
            failures[0].message.contains("No such file")
                && !failures[0].message.contains(crate::PROGRAM_PENDING),
            "{failures:?}"
        );
        assert!(matches!(
            effects.status(DefinitionKind::Effect, id),
            Some(Status::NeedsFallback(_))
        ));
        let id = "test.unstartable.build";
        let mut effects = Effects::new("test");
        assert!(effects.set_snapshot(&one(id)).unwrap().is_empty());
        crate::test_hooks::fail_builds(id, vec![gone]);
        let format = wgpu::TextureFormat::Rgba16Float;
        assert!(
            effects
                .program(&gpu.device, DefinitionKind::Effect, id, format)
                .is_err()
        );
        assert!(matches!(
            effects.status(DefinitionKind::Effect, id),
            Some(Status::NeedsFallback(_))
        ));
        let failure = effects.failures().pop().unwrap();
        assert!(failure.message.contains("No such file"), "{failure:?}");
    }

    /// A program build that keeps failing for the compiler is given up
    /// after the bound: a refusal, and no build after it.
    #[test]
    fn a_program_build_that_keeps_failing_ends_in_a_refusal() {
        let gpu = pm_gpu::Gpu::new("bounded build test").unwrap();
        let format = wgpu::TextureFormat::Rgba16Float;
        let id = "test.bounded.build";
        let mut effects = Effects::new("test");
        assert!(effects.set_snapshot(&one(id)).unwrap().is_empty());
        let crashed = failure(
            pm_glsl::FailureStage::CompilerProcess,
            "the compiler crashed",
        );
        crate::test_hooks::fail_builds(id, vec![crashed; PASSING_LIMIT as usize + 3]);
        for _ in 1..PASSING_LIMIT {
            assert!(
                effects
                    .program(&gpu.device, DefinitionKind::Effect, id, format)
                    .is_err()
            );
            assert!(matches!(
                effects.status(DefinitionKind::Effect, id),
                Some(Status::Ready)
            ));
            expire(&mut effects);
        }
        assert!(
            effects
                .program(&gpu.device, DefinitionKind::Effect, id, format)
                .is_err()
        );
        assert!(matches!(
            effects.status(DefinitionKind::Effect, id),
            Some(Status::NeedsFallback(_))
        ));
        for _ in 0..5 {
            expire(&mut effects);
            let _ = effects.program(&gpu.device, DefinitionKind::Effect, id, format);
        }
        assert_eq!(
            crate::test_hooks::left(id).1,
            3,
            "no build after the refusal"
        );
    }

    /// A busy compiler (its slots taken by other builds) is a pending
    /// registration that becomes ready once it answers, with no trace.
    #[test]
    fn a_busy_compiler_recovers_without_a_trace() {
        let id = "test.busy.recovers";
        let set = one(id);
        let busy = failure(
            pm_glsl::FailureStage::CompilerBusy,
            "all slots are occupied",
        );
        crate::test_hooks::fail_compiles(id, vec![busy; 2]);
        let mut effects = Effects::new("test");
        for _ in 0..2 {
            let failures = effects.set_snapshot(&set).unwrap();
            assert!(
                failures[0].message.contains(crate::PROGRAM_PENDING),
                "{failures:?}"
            );
            expire(&mut effects);
        }
        assert!(effects.set_snapshot(&set).unwrap().is_empty());
        assert!(matches!(
            effects.status(DefinitionKind::Effect, id),
            Some(Status::Ready)
        ));
        assert!(effects.passing.is_empty());
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
