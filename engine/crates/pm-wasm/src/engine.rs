//! The renderer's mirror of the document and its synchronous evaluator.
//!
//! [`Engine`] is plain Rust: the WebAssembly exports in [`crate::abi`] are a
//! thin layer over it, and native tests drive the same code.
//!
//! * **The document** is the canonical project form (`PM.Comps.canonicalize`,
//!   `pm_model::canonical`): every composition in `comps`, `compId` the open
//!   one. It is loaded with [`Engine::resync`] and kept current with canonical
//!   binary patch bodies (contract 5, `canonicalV1`) through
//!   [`Engine::apply`], one renderer flush per batch.
//! * **The fast path** (section 7): [`Engine::intern`] turns a property
//!   address into a numeric id once; [`Engine::ev_id`] and
//!   [`Engine::ev_batch`] answer with no strings crossing the boundary.
//! * **Queries** ([`Engine::query`]) take a small JSON request naming the
//!   composition, the layer (by index, or a detached layer's JSON), and the
//!   time, and answer in JSON ([`crate::reply`]).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use pm_eval::bounds::{Bounds, HOST_ONLY, LayerHost, NoHost, PickOptions};
use pm_eval::node::{self, Got};
use pm_eval::space3d::Rect;
use pm_eval::text_anim::UnitKind;
use pm_eval::text_geom::text_content;
use pm_eval::text_measure::CompatMeasure;
use pm_eval::{
    Comp, EvalHooks, Evaluator, JsThrow, JsValue, PropAddress, PropId, Scope, ValueKind, Via,
};
use pm_model::json::parse_bytes;
use pm_model::{Document, JsString, Node, Value};
use pm_text::answers::{Answers, ReplayResolver};
use pm_text::font::FontBytes;

use crate::expr::PmExpressions;
use crate::reply::Reply;

/// Why a patch batch was not applied.
#[derive(Clone, Debug, PartialEq)]
pub enum ApplyError {
    /// The batch's base is not the mirror's revision (contract 5,
    /// invariant 2): the renderer must resync.
    Gap { have: u64, base: u64 },
    /// The batch's revision does not exceed its base.
    Revision,
    /// The body is not a valid `canonicalV1` body.
    Decode(String),
    /// An operation did not match the mirror (the mirror has diverged). The
    /// operations before it are applied; the renderer must resync.
    Mismatch(String),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyError::Gap { have, base } => {
                write!(
                    f,
                    "revision gap: the mirror is at {have}, the batch is based on {base}"
                )
            }
            ApplyError::Revision => f.write_str("a batch must move the revision forward"),
            ApplyError::Decode(m) => write!(f, "malformed patch body: {m}"),
            ApplyError::Mismatch(m) => write!(f, "the mirror diverged: {m}"),
        }
    }
}

/// Labels for extension-layer parameters (`PM.layerDefinition`), set by the
/// renderer before an `allProps` query on an extension layer.
/// A definition's label and its parameters' labels.
type DefinitionLabelSet = (Option<String>, HashMap<String, String>);

#[derive(Default)]
struct Labels {
    /// Definition id → labels.
    defs: RwLock<HashMap<String, DefinitionLabelSet>>,
}

impl EvalHooks for Labels {
    fn extension_labels(
        &self,
        definition: &JsValue,
        param: &JsString,
    ) -> (Option<JsValue>, Option<JsValue>) {
        let JsValue::String(id) = definition else {
            return (None, None);
        };
        let Ok(defs) = self.defs.read() else {
            return (None, None);
        };
        let Some((label, params)) = defs.get(&id.to_string_lossy()) else {
            return (None, None);
        };
        (
            label
                .as_ref()
                .map(|l| JsValue::String(JsString::from(l.as_str()))),
            params
                .get(&param.to_string_lossy())
                .map(|l| JsValue::String(JsString::from(l.as_str()))),
        )
    }
}

struct LabelsRef(Arc<Labels>);

impl EvalHooks for LabelsRef {
    fn extension_labels(
        &self,
        definition: &JsValue,
        param: &JsString,
    ) -> (Option<JsValue>, Option<JsValue>) {
        self.0.extension_labels(definition, param)
    }
}

/// Text measurement over the registry answers and font subsets main served.
pub struct Fonts {
    /// Whether the added faces carry outlines (whole fonts rather than
    /// metrics-and-shaping subsets), so ink can be answered.
    pub(crate) outlines: bool,
    answers: Answers,
    fonts: Vec<(String, u32, FontBytes)>,
    /// The editor renderer's font cache seeds (amendment 19), from main.
    seeds: Vec<pm_text::compat::FontCacheSeed>,
    pub(crate) measure: CompatMeasure<ReplayResolver>,
}

impl Fonts {
    fn new() -> Fonts {
        let answers = Answers::default();
        Fonts {
            outlines: false,
            measure: CompatMeasure::new(ReplayResolver::new(&answers)),
            answers,
            fonts: Vec::new(),
            seeds: Vec::new(),
        }
    }

    /// Rebuilds the measurer so no cached fallback outlives new answers or
    /// faces (the re-layout event of docs/engine/text.md).
    fn rebuild(&mut self) {
        let mut resolver = ReplayResolver::new(&self.answers);
        for (blob, index, data) in &self.fonts {
            resolver.add_font(blob, *index, data.clone(), 0);
        }
        self.measure = CompatMeasure::new(resolver);
        self.measure
            .compat()
            .set_font_cache_seeds(self.seeds.clone());
    }

    /// Whether the last layout met a query the answers lack, or a face whose
    /// bytes are not loaded.
    pub(crate) fn incomplete(&mut self) -> bool {
        let r = self.measure.compat().resolver();
        !r.misses.is_empty() || !r.pending_blobs.is_empty()
    }

    pub(crate) fn clear_reports(&mut self) {
        self.measure.compat().resolver_mut().clear_reports();
    }
}

/// A query that cannot be answered natively; the renderer answers it with
/// the legacy code.
pub(crate) enum Unanswered {
    /// The question needs today's 3D runtime (three.js) in Phase 1.
    Host,
    /// Text layout met fonts the mirror does not have yet.
    Fonts,
    /// The answer depends on glyph ink, and the faces are metrics-and-shaping
    /// subsets without outlines (docs/engine/pm-eval-layers.md, allowed
    /// difference 6).
    Ink,
}

/// Measures through [`CompatMeasure`] and notes whether anything asked for
/// glyph ink (`measure_lines`); widths, carets, and glyph positions need no
/// outlines.
pub(crate) struct InkProbe<'m> {
    pub(crate) inner: &'m mut CompatMeasure<ReplayResolver>,
    pub(crate) ink: bool,
}

impl pm_eval::text_geom::TextMeasure for InkProbe<'_> {
    fn require_ink(&mut self) {
        self.ink = true;
    }
    fn level2(
        &mut self,
        d: &pm_text::compat::TextContent,
        spans: &[pm_text::level2::StyleSpan],
    ) -> Result<pm_eval::text_level2::Shaped, String> {
        self.inner.level2(d, spans)
    }
    fn level2_ink(
        &mut self,
        data: &pm_text::font::FontBytes,
        run: &pm_text::level2::Run,
        glyph: u32,
    ) -> Option<[f64; 4]> {
        self.inner.level2_ink(data, run, glyph)
    }
    fn measure_lines(
        &mut self,
        d: &pm_text::compat::TextContent,
    ) -> Vec<pm_eval::text_geom::MeasuredLine> {
        self.ink = true;
        self.inner.measure_lines(d)
    }
    fn plain_width(&mut self, d: &pm_text::compat::TextContent, text: &str) -> f64 {
        self.inner.plain_width(d, text)
    }
    fn glyph_layout(&mut self, d: &pm_text::compat::TextContent) -> pm_text::compat::GlyphLayout {
        self.inner.glyph_layout(d)
    }
    fn spaced_width(&mut self, d: &pm_text::compat::TextContent, text: &str) -> f64 {
        self.inner.spaced_width(d, text)
    }
}

pub(crate) type QueryResult = Result<Result<(), JsThrow>, Unanswered>;

/// Engine statistics for the renderer's diagnostics.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub batches: u64,
    pub ops: u64,
    pub resyncs: u64,
    pub queries: u64,
    pub unanswered: u64,
    pub detached_hits: u64,
    pub detached_misses: u64,
}

pub struct Engine {
    doc: Document,
    ev: Evaluator,
    // A bounded resident scope cache. Each entry has its own evaluator so an
    // ad-hoc scope never resets the open document's interned property plans.
    detached: HashMap<String, DetachedComp>,
    detached_tick: u64,
    labels: Arc<Labels>,
    fonts: Fonts,
    /// The last `ev_id` answer when it was not a number (for `value_json`).
    last: JsValue,
    last_kind: ValueKind,
    pub stats: Stats,
}

struct DetachedComp {
    node: Box<Node>,
    reads: Option<pm_model::doc::reads::Index>,
    ev: Evaluator,
    revision: u64,
    used: u64,
}

fn evaluator(labels: &Arc<Labels>) -> Evaluator {
    Evaluator::new()
        .with_expressions(Arc::new(PmExpressions))
        .with_hooks(Arc::new(LabelsRef(labels.clone())))
}

impl Default for Engine {
    fn default() -> Self {
        Engine::new()
    }
}

fn number(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64)
}

fn get<'v>(request: &'v Value, key: &str) -> Option<&'v Value> {
    request.as_object().and_then(|m| m.get(key))
}

/// A JSON value from the request as the JavaScript value it stands for.
fn js_of(v: Option<&Value>) -> JsValue {
    match v {
        None => JsValue::Undefined,
        Some(Value::Null) => JsValue::Null,
        Some(Value::Bool(b)) => JsValue::Bool(*b),
        Some(Value::Number(n)) => JsValue::Number(*n),
        Some(Value::String(s)) => JsValue::String(s.clone()),
        Some(v) => JsValue::Object(Box::new(v.clone())),
    }
}

/// `{"$u":1}` in a request stands for `undefined` (JSON has no form for it).
fn arg(v: Option<&Value>) -> JsValue {
    match v {
        Some(Value::Object(m)) if m.len() == 1 && m.contains_key("$u") => JsValue::Undefined,
        other => js_of(other),
    }
}

fn js_values(v: Option<&Value>) -> Vec<JsValue> {
    v.and_then(Value::as_array)
        .map(|items| items.iter().map(|i| js_of(Some(i))).collect())
        .unwrap_or_default()
}

/// The path of `target` inside `root` (pointer identity), as keys and indexes.
fn path_of(root: &Node, target: &Node, path: &mut Vec<Value>) -> bool {
    if std::ptr::eq(root, target) {
        return true;
    }
    match root.kind() {
        pm_model::NodeKind::Object(map) => {
            for (k, child) in map.iter() {
                path.push(Value::String(k.clone()));
                if path_of(child, target, path) {
                    return true;
                }
                path.pop();
            }
        }
        pm_model::NodeKind::Array(items) => {
            for (i, child) in items.iter().enumerate() {
                path.push(Value::Number(i as f64));
                if path_of(child, target, path) {
                    return true;
                }
                path.pop();
            }
        }
        _ => {}
    }
    false
}

/// The node at a path of own members and array indexes inside a layer.
fn at_path<'a>(root: &'a Node, path: &[Value]) -> Option<&'a Node> {
    let mut node = root;
    for part in path {
        node = match (node.kind(), part) {
            (pm_model::NodeKind::Object(map), Value::String(k)) => map.get(k)?,
            (pm_model::NodeKind::Array(items), Value::Number(i)) => items.get(*i as usize)?,
            _ => return None,
        };
    }
    Some(node)
}

fn bounds_reply(out: &mut Reply, b: &Bounds) {
    let mut first = true;
    for (k, v) in [
        ("x0", b.x0),
        ("y0", b.y0),
        ("x1", b.x1),
        ("y1", b.y1),
        ("w", b.w),
        ("h", b.h),
        ("ax", b.ax),
        ("ay", b.ay),
    ] {
        out.key(&mut first, k);
        out.num(v);
    }
}

/// Layer indexes of `nodes` within `layers` (pointer identity), for replies
/// the renderer maps back to its live layers.
fn indexes(out: &mut Reply, layers: &[Node], nodes: &[&Node]) {
    let all: Vec<f64> = nodes
        .iter()
        .map(|n| {
            layers
                .iter()
                .position(|l| std::ptr::eq(l, *n))
                .map_or(-1.0, |i| i as f64)
        })
        .collect();
    out.nums(&all);
}

/// A [`LayerHost`] that refuses every 3D question, so the renderer falls back
/// to today's three.js answer (section 26, Phase 1).
pub(crate) struct RefuseHost;
impl LayerHost for RefuseHost {}

pub(crate) fn host_only(e: &JsThrow) -> bool {
    e.kind == HOST_ONLY
}

impl Engine {
    pub fn new() -> Engine {
        let labels = Arc::new(Labels::default());
        Engine {
            doc: Document::new(None, 0),
            ev: evaluator(&labels),
            detached: HashMap::new(),
            detached_tick: 0,
            labels,
            fonts: Fonts::new(),
            last: JsValue::Undefined,
            last_kind: ValueKind::Number,
            stats: Stats::default(),
        }
    }

    // ── the document ──────────────────────────────────────────────────────

    pub fn revision(&self) -> u64 {
        self.doc.revision()
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// Replaces the mirror with the canonical project JSON at `revision`
    /// (contract 5, `Resync` with `canonicalJsonV1`). The revision may be
    /// any value: a resync starts a new history (the evaluator notices a
    /// lower revision and drops its caches).
    pub fn resync(&mut self, json: &[u8], revision: u64) -> Result<(), String> {
        let root = if json.is_empty() {
            None
        } else {
            Some(parse_bytes(json).map_err(|e| e.to_string())?)
        };
        if revision > self.doc.revision() {
            self.doc.resync(root, revision);
        } else {
            self.doc = Document::new(root, revision);
            self.ev.reset();
        }
        self.detached.clear();
        self.stats.resyncs += 1;
        Ok(())
    }

    /// Applies a `canonicalV1` body as `revision`, on top of `base`.
    pub fn apply(&mut self, body: &[u8], base: u64, revision: u64) -> Result<usize, ApplyError> {
        if base != self.doc.revision() {
            return Err(ApplyError::Gap {
                have: self.doc.revision(),
                base,
            });
        }
        if revision <= base {
            return Err(ApplyError::Revision);
        }
        let ops = pm_model::binary::decode(body).map_err(|e| ApplyError::Decode(e.to_string()))?;
        self.doc
            .apply_ops_at(&ops, revision)
            .map_err(|e| ApplyError::Mismatch(e.to_string()))?;
        self.ev.document_patched(&self.doc, base, &ops);
        self.stats.batches += 1;
        self.stats.ops += ops.len() as u64;
        Ok(ops.len())
    }

    /// The canonical hash (contract 5, "Canonical hash"), or `None` for an
    /// undefined document.
    pub fn hash(&self) -> Option<String> {
        self.doc.canonical_hash()
    }

    /// The canonical serialization of the mirror.
    pub fn to_json(&self) -> String {
        self.doc
            .root()
            .map(pm_model::json::write::to_canonical_string)
            .unwrap_or_default()
    }

    // ── the fast path ─────────────────────────────────────────────────────

    /// Interns `{comp, layer, key, via}` (`via`: `"channel"`, `"find"`, or a
    /// member path of strings inside the layer).
    pub fn intern(&mut self, request: &Value) -> Option<PropId> {
        let s = |k: &str| get(request, k).and_then(Value::as_js_string).cloned();
        let comp = s("comp")?;
        let layer = s("layer")?;
        let key = s("key")?;
        let via = match get(request, "via")? {
            Value::String(v) if v.as_str() == Some("channel") => Via::Channel,
            Value::String(v) if v.as_str() == Some("find") => Via::Find,
            Value::Array(parts) => Via::Member(
                parts
                    .iter()
                    .map(|p| match p {
                        Value::String(s) => Some(s.clone()),
                        // Array indexes are member keys too (`fx.0.p.amount`).
                        Value::Number(n) => Some(JsString::from(ryu_js::Buffer::new().format(*n))),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()?,
            ),
            _ => return None,
        };
        Some(self.ev.intern(PropAddress {
            comp,
            layer,
            key,
            via,
        }))
    }

    pub fn release(&mut self, id: PropId) {
        self.ev.release(id);
    }

    /// One id at time `t`; a non-number answer is kept for [`Engine::last_json`].
    pub fn ev_id(&mut self, id: PropId, t: f64) -> (ValueKind, f64) {
        match self.ev.ev_id(&self.doc, id, t) {
            Ok(JsValue::Number(n)) => {
                self.last_kind = ValueKind::Number;
                (ValueKind::Number, n)
            }
            Ok(v) => {
                self.last = v;
                self.last_kind = ValueKind::Other;
                (ValueKind::Other, f64::NAN)
            }
            Err(e) => {
                self.last = JsValue::String(JsString::from(e.message.as_str()));
                self.last_kind = ValueKind::Threw;
                (ValueKind::Threw, f64::NAN)
            }
        }
    }

    /// The last non-number answer of [`Engine::ev_id`] (or the thrown message).
    pub fn last_json(&self) -> String {
        let mut out = Reply::new();
        out.js(&self.last);
        out.text
    }

    pub fn last_special(&self) -> bool {
        matches!(&self.last, JsValue::Undefined)
            || matches!(&self.last, JsValue::Number(n) if !n.is_finite())
    }

    pub fn ev_batch(&mut self, ids: &[u32], t: f64, out: &mut [f64], kinds: &mut [u8]) {
        let ids: Vec<PropId> = ids.iter().map(|&i| PropId::from_index(i)).collect();
        let mut k = vec![ValueKind::Number; ids.len()];
        self.ev.ev_batch(&self.doc, &ids, t, out, &mut k);
        for ((dst, src), &id) in kinds.iter_mut().zip(k).zip(&ids) {
            *dst = src as u8
                | if self.expression_error(id).is_some() {
                    0x80
                } else {
                    0
                };
        }
    }

    /// Diagnostics for incremental plan reuse (not on the evaluation path).
    pub fn evaluation_cache_stats(&self) -> pm_eval::CacheStats {
        self.ev.cache_stats()
    }

    /// The expression error recorded for an id's channel, if any.
    pub fn expression_error(&self, id: PropId) -> Option<&str> {
        let a = self.ev.address(id)?;
        self.ev.expression_error_at(a)
    }

    // ── fonts ─────────────────────────────────────────────────────────────

    /// Merges registry answers (`{ faces, answers }` JSON from main).
    pub fn add_font_answers(&mut self, json: &[u8]) -> Result<(), String> {
        let answers: Answers = serde_json::from_slice(json).map_err(|e| e.to_string())?;
        self.fonts.answers.merge(&answers);
        self.fonts.rebuild();
        Ok(())
    }

    /// Adds a metrics-and-shaping subset for face `index` of `blob`.
    pub fn add_font(&mut self, blob: &str, index: u32, bytes: Vec<u8>) {
        let data = FontBytes::new(bytes);
        self.fonts
            .fonts
            .retain(|(b, i, _)| !(b == blob && *i == index));
        self.fonts.fonts.push((blob.to_owned(), index, data));
        self.fonts.rebuild();
    }

    /// `{ misses, pending }`: what the last text layout lacked.
    pub fn font_report(&mut self) -> String {
        let r = self.fonts.measure.compat().resolver();
        let misses = serde_json::to_string(&r.misses).unwrap_or_else(|_| "[]".into());
        let pending: Vec<(&String, &u32)> = r.pending_blobs.iter().map(|(b, i)| (b, i)).collect();
        let pending = serde_json::to_string(&pending).unwrap_or_else(|_| "[]".into());
        format!("{{\"misses\":{misses},\"pending\":{pending}}}")
    }

    /// Drops every answer and face (after a registry generation change).
    /// The seeds stay: they describe the renderer, not the registry.
    pub fn clear_fonts(&mut self) {
        let outlines = self.fonts.outlines;
        let seeds = std::mem::take(&mut self.fonts.seeds);
        self.fonts = Fonts::new();
        self.fonts.outlines = outlines;
        self.fonts.seeds = seeds;
        self.fonts.rebuild();
    }

    /// The editor renderer's font cache seeds (`{ seeds: FontCacheSeed[] }`
    /// or a bare array, main's `engine-fonts:seeds`): level 1 measures as
    /// that renderer does (amendment 19, docs/engine/text.md). Returns
    /// whether they changed; a change lays text out again.
    pub fn set_font_seeds(&mut self, json: &[u8]) -> Result<bool, String> {
        let value: serde_json::Value = serde_json::from_slice(json).map_err(|e| e.to_string())?;
        let list = match value {
            serde_json::Value::Object(mut o) => o.remove("seeds").unwrap_or_default(),
            other => other,
        };
        let seeds: Vec<pm_text::compat::FontCacheSeed> =
            serde_json::from_value(list).map_err(|e| e.to_string())?;
        if seeds == self.fonts.seeds {
            return Ok(false);
        }
        self.fonts.seeds = seeds;
        self.fonts.rebuild();
        Ok(true)
    }

    /// Declares that the faces added are whole fonts with outlines, so
    /// ink-dependent answers (text bounds, raster placement) are native.
    pub fn set_font_outlines(&mut self, outlines: bool) {
        self.fonts.outlines = outlines;
    }

    // ── queries ───────────────────────────────────────────────────────────

    /// Sets the labels `allProps` reports for an extension definition.
    pub fn set_definition(&mut self, request: &Value) {
        let Some(id) = get(request, "id").and_then(Value::as_str) else {
            return;
        };
        let label = get(request, "label")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let params = get(request, "params")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| Some((k.to_string_lossy(), v.as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        if let Ok(mut defs) = self.labels.defs.write() {
            defs.insert(id.to_owned(), (label, params));
        }
    }

    /// Answers one JSON request. The reply is `{"ok":value}`, `{"throw":
    /// {"kind","message"}}` where the legacy code throws, `{"host":true}` for
    /// questions only today's 3D runtime answers in Phase 1, `{"fonts":true}`
    /// when text layout met fonts the mirror does not have yet, or
    /// `{"error":message}` for a malformed request.
    pub fn query(&mut self, bytes: &[u8]) -> Reply {
        self.stats.queries += 1;
        let mut out = Reply::new();
        let request = match parse_bytes(bytes) {
            Ok(r) => r,
            Err(e) => {
                out.raw("{\"error\":");
                out.str(&format!("malformed request: {e}"));
                out.raw("}");
                return out;
            }
        };
        let tracked = get(&request, "scopeId").and_then(Value::as_str);
        let mut body = Reply::new();
        let result = self.dispatch(&request, &mut body);
        match result {
            Ok(Ok(())) => {
                out.raw("{\"ok\":");
                out.raw(&body.text);
                out.raw("}");
                out.special = body.special;
            }
            Ok(Err(e)) => {
                out.raw("{\"throw\":{\"kind\":");
                out.str(e.kind);
                out.raw(",\"message\":");
                out.str(&e.message);
                out.raw("}}");
            }
            Err(Unanswered::Host) => {
                self.stats.unanswered += 1;
                out.raw("{\"host\":true}");
            }
            Err(Unanswered::Fonts) => {
                self.stats.unanswered += 1;
                out.raw("{\"fonts\":true}");
            }
            Err(Unanswered::Ink) => {
                self.stats.unanswered += 1;
                out.raw("{\"ink\":true}");
            }
        }
        if let Some(id) = tracked {
            let reads =
                pm_model::doc::reads::finish(self.detached.get(id).and_then(|c| c.reads.as_ref()));
            out.text.pop();
            out.raw(",\"reads\":[");
            for (i, path) in reads.iter().enumerate() {
                if i > 0 {
                    out.raw(",");
                }
                out.raw("[");
                for (j, part) in path.iter().enumerate() {
                    if j > 0 {
                        out.raw(",");
                    }
                    match part {
                        pm_model::PathPart::Key(k) => out.js_str(k),
                        pm_model::PathPart::Index(n) => out.num(*n),
                    }
                }
                out.raw("]");
            }
            out.raw("]}");
        }
        out
    }

    fn dispatch(&mut self, request: &Value, out: &mut Reply) -> QueryResult {
        let op = get(request, "op").and_then(Value::as_str).unwrap_or("");
        // Questions that need no document.
        match op {
            "mul" => {
                let m = mat::<6>(get(request, "a"));
                let n = mat::<6>(get(request, "b"));
                out.nums(&pm_eval::transform::mul(&m, &n));
                return Ok(Ok(()));
            }
            "projectPoint" => {
                let m = mat::<9>(get(request, "m"));
                let (x, y) = pm_eval::space3d::project_point(
                    &m,
                    number(get(request, "x")).unwrap_or(f64::NAN),
                    number(get(request, "y")).unwrap_or(f64::NAN),
                );
                out.raw("{\"x\":");
                out.num(x);
                out.raw(",\"y\":");
                out.num(y);
                out.raw("}");
                return Ok(Ok(()));
            }
            "inversePlane" => {
                let m = mat::<9>(get(request, "m"));
                match pm_eval::space3d::inverse_plane(&m) {
                    Some(inv) => out.nums(&inv),
                    None => out.raw("null"),
                }
                return Ok(Ok(()));
            }
            "countTextUnits" => {
                let lines: Vec<JsString> = get(request, "lines")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().map(Value::to_js_string).collect())
                    .unwrap_or_default();
                let unit = match get(request, "unit").and_then(Value::as_str) {
                    Some("words") => UnitKind::Words,
                    Some("lines") => UnitKind::Lines,
                    _ => UnitKind::Characters,
                };
                out.num(pm_eval::text_anim::count_text_units(&lines, unit) as f64);
                return Ok(Ok(()));
            }
            "expressionDiagnostic" => {
                let source = get(request, "source")
                    .map(Value::to_js_string)
                    .unwrap_or_default();
                let units: Vec<u16> = source.utf16_units().collect();
                match pm_expr::expression_diagnostic_utf16(&units, pm_expr::Dialect::Legacy) {
                    Some(m) => out.str(&m),
                    None => out.raw("null"),
                }
                return Ok(Ok(()));
            }
            "frameRate" => {
                frame_rate(request, out);
                return Ok(Ok(()));
            }
            _ => {}
        }

        // The edit kernels (pm-edit): patches the renderer applies.
        if let Some(name) = op.strip_prefix("edit.") {
            return crate::edit::dispatch(&self.doc, &mut self.fonts, name, request, out);
        }
        let Some(root) = self.doc.root() else {
            out.raw("null");
            return Ok(Err(JsThrow::type_error("The mirror holds no document")));
        };
        let comp_id = get(request, "comp").and_then(Value::as_str).unwrap_or("");
        let open_id = get(request, "open")
            .and_then(Value::as_str)
            .unwrap_or(comp_id);
        let revision = self.doc.revision();
        let detached = get(request, "compJson").filter(|v| matches!(v, Value::Object(_)));
        // The renderer carries detached JSON as a string so repeated queries
        // need no object parse or canonical number serialization before hashing.
        let text = get(request, "compJsonText").and_then(Value::as_str);
        let scope_id = get(request, "scopeId").and_then(Value::as_str);
        let hash = scope_id.map(str::to_owned).or_else(|| {
            text.map(|s| pm_model::hash::sha256_hex(s.as_bytes()))
                .or_else(|| detached.map(pm_model::hash::canonical_hash))
        });
        let resident = if let Some(hash) = hash {
            self.detached_tick += 1;
            if scope_id.is_some() && (text.is_some() || detached.is_some()) {
                self.detached.remove(&hash);
            }
            if self.detached.contains_key(&hash) {
                self.stats.detached_hits += 1;
            } else {
                let value = match (text, detached) {
                    (Some(json), _) => match parse_bytes(json.as_bytes()) {
                        Ok(v) if matches!(v, Value::Object(_)) => v,
                        _ => {
                            return Ok(Err(JsThrow::type_error(
                                "Invalid detached composition JSON",
                            )));
                        }
                    },
                    (_, Some(v)) => v.clone(),
                    _ => return Ok(Err(JsThrow::type_error("detached-scope-missing"))),
                };
                self.stats.detached_misses += 1;
                if self.detached.len() >= 4
                    && let Some(oldest) = self
                        .detached
                        .iter()
                        .min_by_key(|(_, c)| c.used)
                        .map(|(h, _)| h.clone())
                {
                    self.detached.remove(&oldest);
                }
                let node = Box::new(Node::from_value(value, revision));
                let reads = scope_id.map(|_| pm_model::doc::reads::Index::new(&node));
                self.detached.insert(
                    hash.clone(),
                    DetachedComp {
                        node,
                        reads,
                        ev: evaluator(&self.labels),
                        revision,
                        used: self.detached_tick,
                    },
                );
            }
            let c = self.detached.get_mut(&hash).unwrap();
            // Expressions can read project members as well as their own scope.
            if c.revision != revision {
                c.ev.reset();
                c.revision = revision;
            }
            c.used = self.detached_tick;
            if let Some(patches) = get(request, "scopePatches") {
                let patches = match pm_model::patch::patches_from_value(patches) {
                    Ok(patches) => patches,
                    Err(e) => {
                        return Ok(Err(JsThrow::type_error(format!(
                            "Invalid scope patches: {e}"
                        ))));
                    }
                };
                if let Err(e) = c.node.apply_patches(&patches, revision) {
                    return Ok(Err(JsThrow::type_error(format!(
                        "Invalid scope update: {e}"
                    ))));
                }
            }
            if scope_id.is_some() && get(request, "scopePatches").is_some() {
                c.reads = Some(pm_model::doc::reads::Index::new(&c.node));
            }
            // Dependencies must include reads otherwise hidden by prepared
            // plans or expression memos from an earlier request.
            if scope_id.is_some() {
                c.ev.reset();
                // Trace the answer, not reconciliation's bookkeeping reads.
                pm_model::doc::reads::begin();
            }
            Some(c)
        } else {
            None
        };
        let (resolved, ev) = match resident {
            Some(c) => (
                Ok(Comp::at_node(root, &c.node, comp_id, revision)),
                &mut c.ev,
            ),
            None => (Comp::in_root(root, comp_id, revision), &mut self.ev),
        };
        let Ok(comp) = resolved else {
            return Ok(Err(JsThrow::type_error(format!(
                "No composition {comp_id}"
            ))));
        };
        let resolved_open = if open_id.is_empty() {
            Ok(Comp::at_node(root, root, open_id, revision))
        } else {
            Comp::in_root(root, open_id, revision)
        };
        let Ok(open) = resolved_open else {
            return Ok(Err(JsThrow::type_error(format!(
                "No composition {open_id}"
            ))));
        };
        let t = number(get(request, "t")).unwrap_or(f64::NAN);
        let layers = comp.layers();
        // The layer: an index into the composition, or a detached layer's JSON
        // evaluated ad hoc (section 6, item 3).
        let detached: Option<Node> = match get(request, "layer") {
            Some(v @ Value::Object(_)) => Some(Node::from_value(v.clone(), revision)),
            _ => None,
        };
        let layer: Option<&Node> = match (&detached, number(get(request, "layer"))) {
            (Some(d), _) => Some(d),
            (None, Some(i)) if i >= 0.0 => layers.get(i as usize),
            _ => None,
        };
        // A parent layer outside the composition (`transformParentMatrix`,
        // `parent3D`), a channel outside the layer (`hasKeyAt`), and a
        // vector path outside `d.paths` (`pathValues`, `groupMatrix`) are
        // evaluated ad hoc from their JSON, as the layer itself is.
        let detached_parent: Option<Node> = match get(request, "parent") {
            Some(v @ Value::Object(_)) => Some(Node::from_value(v.clone(), revision)),
            _ => None,
        };
        let detached_prop: Option<Node> = match get(request, "prop") {
            Some(v @ Value::Object(_)) => Some(Node::from_value(v.clone(), revision)),
            _ => None,
        };
        let detached_path: Option<Node> = match get(request, "pathJson") {
            Some(v @ Value::Object(_)) => Some(Node::from_value(v.clone(), revision)),
            _ => None,
        };
        let layer_at = |key: &str| -> Option<Option<&Node>> {
            match get(request, key) {
                None => None,
                Some(Value::Number(i)) if *i >= 0.0 => Some(layers.get(*i as usize)),
                Some(Value::Object(_)) if key == "parent" => Some(detached_parent.as_ref()),
                Some(_) => Some(None),
            }
        };
        let fonts = &mut self.fonts;
        let mut scope = Scope::new(ev, &comp, &open);
        macro_rules! layer {
            () => {
                match layer {
                    Some(l) => l,
                    None => {
                        return Ok(Err(JsThrow::type_error(
                            "Cannot read properties of undefined (reading 'p')",
                        )))
                    }
                }
            };
        }
        macro_rules! try_js {
            ($e:expr) => {
                match $e {
                    Ok(v) => v,
                    Err(e) if host_only(&e) => return Err(Unanswered::Host),
                    Err(e) => return Ok(Err(e)),
                }
            };
        }
        match op {
            "ev" => {
                let l = layer!();
                let key = get(request, "key")
                    .map(Value::to_js_string)
                    .unwrap_or_default();
                let v = try_js!(scope.ev.ev(&comp, Some(l), &key, t));
                out.js(&v);
            }
            "evP" => {
                let l = layer!();
                let key = get(request, "key")
                    .map(Value::to_js_string)
                    .unwrap_or_default();
                let path: Vec<Value> = get(request, "path")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                // A channel object outside the layer (evaluated ad hoc from
                // its JSON, with the layer as context).
                let prop = if detached_prop.is_some() {
                    detached_prop.as_ref()
                } else if get(request, "path").is_some_and(Value::is_null) {
                    None
                } else {
                    at_path(l, &path)
                };
                let v = try_js!(scope.ev.ev_p(&comp, Some(l), prop, t, &key));
                out.js(&v);
            }
            "active" => {
                let l = layer!();
                out.bool(try_js!(scope.active(l, t)));
            }
            "resolveContent" => {
                let l = layer!();
                let content = try_js!(scope.ev.resolve_content(&comp, l, t));
                out.raw("{");
                let mut first = true;
                for (k, v) in &content {
                    if !first {
                        out.raw(",");
                    }
                    first = false;
                    out.js_str(k);
                    out.raw(":");
                    out.js(v);
                }
                out.raw("}");
            }
            "findProp" => {
                let l = layer!();
                let key = get(request, "key")
                    .map(Value::to_js_string)
                    .unwrap_or_default();
                match try_js!(scope.ev.find_prop(l, &key)) {
                    Got::Node(n) => {
                        let mut path = Vec::new();
                        if path_of(l, n, &mut path) {
                            out.value(&Value::Array(path));
                        } else {
                            out.raw("null");
                        }
                    }
                    _ => out.raw("null"),
                }
            }
            "allProps" => {
                let l = layer!();
                let entries = try_js!(scope.ev.all_props(l));
                out.raw("[");
                for (i, e) in entries.iter().enumerate() {
                    if i > 0 {
                        out.raw(",");
                    }
                    out.raw("{\"key\":");
                    out.js_str(&e.key);
                    out.raw(",\"path\":");
                    match &e.prop {
                        Got::Node(n) => {
                            let mut path = Vec::new();
                            if path_of(l, n, &mut path) {
                                out.value(&Value::Array(path));
                            } else {
                                out.raw("null");
                            }
                        }
                        Got::Value(v) => {
                            out.raw("{\"value\":");
                            out.js(v);
                            out.raw("}");
                        }
                        Got::Undefined => out.raw("null"),
                    }
                    out.raw(",\"label\":");
                    out.js(&e.label);
                    out.raw(",\"group\":");
                    out.js(&e.group);
                    out.raw("}");
                }
                out.raw("]");
            }
            "hasKeyAt" => {
                // `p.kf.find(k => Math.abs(k.t - (T - L.from)) < .5 / PM.proj.fps)`.
                let l = layer!();
                let path: Vec<Value> = get(request, "path")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let Some(prop) = detached_prop.as_ref().or_else(|| at_path(l, &path)) else {
                    return Ok(Err(JsThrow::type_error(
                        "Cannot read properties of undefined (reading 'kf')",
                    )));
                };
                let from = node::get(l, b"from").to_js();
                let tl = pm_eval::js_sem::sub(&JsValue::Number(t), &from);
                let fps = open.project_fps().to_number();
                let limit = 0.5 / fps;
                let kf = node::array(node::member(Some(prop), "kf"));
                let Some(kf) = kf else {
                    return Ok(Err(JsThrow::type_error("p.kf.find is not a function")));
                };
                let hit = kf.iter().position(|k| {
                    let kt = node::get(k, b"t").to_js().to_number();
                    (kt - tl).abs() < limit
                });
                match hit {
                    Some(i) => out.num(i as f64),
                    None => out.raw("null"),
                }
            }
            "wouldCycle" => {
                let l = layer!();
                let parent = arg(get(request, "parent"));
                let got = match &parent {
                    JsValue::Undefined => Got::Undefined,
                    other => Got::Value(other.clone()),
                };
                out.bool(try_js!(scope.would_cycle(l, &got)));
            }
            "localMatrix" => {
                let l = layer!();
                out.nums(&try_js!(scope.local_matrix(l, t)));
            }
            "worldMatrix" => {
                let l = layer!();
                out.nums(&try_js!(scope.world_matrix(l, t)));
            }
            "transformParentMatrix" => {
                let l = layer!();
                let m = try_js!(scope.transform_parent_matrix(l, t, layer_at("parent")));
                out.nums(&m);
            }
            "worldOpacity" => {
                let l = layer!();
                out.num(try_js!(scope.world_opacity(l, t)));
            }
            "local3D" => {
                let l = layer!();
                out.nums(&try_js!(scope.local3d(l, t)));
            }
            "parent3D" => {
                let l = layer!();
                out.nums(&try_js!(scope.parent3d(l, t, layer_at("parent"))));
            }
            "world3D" => {
                let l = layer!();
                let positioning = arg(get(request, "positioning")).truthy();
                out.nums(&try_js!(scope.world3d(l, t, positioning)));
            }
            "is3DLayer" => {
                let l = layer!();
                out.bool(try_js!(scope.is_3d_layer(l)));
            }
            "perspectiveAmount" => {
                let l = layer!();
                out.num(try_js!(scope.perspective_amount(l, t)));
            }
            "planeMatrix" => {
                let l = layer!();
                let positioning = arg(get(request, "positioning")).truthy();
                out.nums(&try_js!(scope.plane_matrix(l, t, positioning)));
            }
            "planeContains" => {
                let l = layer!();
                let b = get(request, "bounds");
                let r = Rect {
                    x0: number(b.and_then(|b| get(b, "x0"))).unwrap_or(f64::NAN),
                    y0: number(b.and_then(|b| get(b, "y0"))).unwrap_or(f64::NAN),
                    x1: number(b.and_then(|b| get(b, "x1"))).unwrap_or(f64::NAN),
                    y1: number(b.and_then(|b| get(b, "y1"))).unwrap_or(f64::NAN),
                };
                let x = number(get(request, "x")).unwrap_or(f64::NAN);
                let y = number(get(request, "y")).unwrap_or(f64::NAN);
                out.bool(try_js!(scope.plane_contains(l, t, x, y, &r)));
            }
            "groupAncestors" => {
                let l = layer!();
                let groups = try_js!(scope.group_ancestors(l));
                indexes(out, layers, &groups);
            }
            "groupSpan" => {
                let l = layer!();
                let s = try_js!(scope.group_span(l));
                out.raw("{\"from\":");
                out.num(s.from);
                out.raw(",\"dur\":");
                out.num(s.dur);
                out.raw("}");
            }
            "expandGroups" => {
                let ids = js_values(get(request, "ids"));
                let found = try_js!(scope.expand_groups(&ids));
                out.raw("[");
                for (i, l) in found.iter().enumerate() {
                    if i > 0 {
                        out.raw(",");
                    }
                    out.js(&node::get(l, b"id").to_js());
                }
                out.raw("]");
            }
            "transformRoots" => {
                let ids = js_values(get(request, "ids"));
                let roots = try_js!(scope.transform_roots(&ids));
                indexes(out, layers, &roots);
            }
            "pathValues" => {
                // `pathValues(PM, layer, path, time, prefix)` for `d.paths[index]`.
                let l = layer!();
                let index = number(get(request, "index")).unwrap_or(-1.0);
                let paths =
                    node::array(node::member(node::member(Some(l), "d"), "paths")).unwrap_or(&[]);
                let stored = (index >= 0.0).then(|| paths.get(index as usize)).flatten();
                let Some(path) = detached_path.as_ref().or(stored) else {
                    return Ok(Err(JsThrow::type_error(
                        "Cannot read properties of undefined (reading 'p')",
                    )));
                };
                let prefix = match get(request, "prefix").and_then(Value::as_str) {
                    Some(p) => p.to_owned(),
                    None => format!(
                        "g.{}",
                        node::to_js_string(node::member(Some(path), "id")).to_string_lossy()
                    ),
                };
                let e = try_js!(scope.path_values(l, path, t, &prefix));
                out.raw("{");
                let mut first = true;
                for (k, v) in &e.values {
                    if !first {
                        out.raw(",");
                    }
                    first = false;
                    out.js_str(k);
                    out.raw(":");
                    out.js(v);
                }
                if !first {
                    out.raw(",");
                }
                out.raw("\"vertices\":[");
                for (i, v) in e.vertices.iter().enumerate() {
                    if i > 0 {
                        out.raw(",");
                    }
                    out.raw("{");
                    for (j, (k, x)) in v.iter().enumerate() {
                        if j > 0 {
                            out.raw(",");
                        }
                        out.js_str(k);
                        out.raw(":");
                        out.num(*x);
                    }
                    out.raw("}");
                }
                out.raw("]}");
            }
            "groupMatrix" => {
                // `groupMatrix(PM, layer, d.paths[index], time)`.
                let l = layer!();
                let index = number(get(request, "index")).unwrap_or(-1.0);
                if let Some(path) = detached_path.as_ref() {
                    out.nums(&try_js!(scope.detached_group_matrix(l, path, t)));
                    return Ok(Ok(()));
                }
                let (_, matrices, _) = try_js!(scope.path_geometry(l, t));
                match (index >= 0.0)
                    .then(|| matrices.get(index as usize))
                    .flatten()
                {
                    Some(m) => out.nums(m),
                    None => out.raw("null"),
                }
            }
            "validMatteSource" => {
                let l = layer!();
                let id = arg(get(request, "id"));
                out.bool(scope.valid_matte_source(l, &id));
            }
            "bounds" | "groupBounds" | "pick" | "textLayout" | "textRaster" => {
                fonts.clear_reports();
                let mut host = RefuseHost;
                let outlines = fonts.outlines;
                let mut probe = InkProbe {
                    inner: &mut fonts.measure,
                    ink: false,
                };
                match op {
                    "bounds" => {
                        let l = layer!();
                        let b = scope.bounds(l, t, &mut probe, &mut host);
                        let ink = probe.ink;
                        if fonts.incomplete() {
                            return Err(Unanswered::Fonts);
                        }
                        if ink && !outlines {
                            return Err(Unanswered::Ink);
                        }
                        match try_js!(b) {
                            Some(b) => {
                                out.raw("{");
                                bounds_reply(out, &b);
                                out.raw("}");
                            }
                            None => out.raw("null"),
                        }
                    }
                    "groupBounds" => {
                        let l = layer!();
                        let b = scope.group_bounds(l, t, &mut probe, &mut host);
                        let ink = probe.ink;
                        if fonts.incomplete() {
                            return Err(Unanswered::Fonts);
                        }
                        if ink && !outlines {
                            return Err(Unanswered::Ink);
                        }
                        match try_js!(b) {
                            Some(b) => {
                                out.raw("{");
                                bounds_reply(out, &b);
                                out.raw("}");
                            }
                            None => out.raw("null"),
                        }
                    }
                    "pick" => {
                        let local: Option<Vec<JsValue>> = get(request, "localView")
                            .filter(|v| !v.is_null())
                            .map(|v| js_values(Some(v)));
                        let options = PickOptions {
                            include_locked: arg(get(request, "includeLocked")).truthy(),
                            local_view: local.as_deref(),
                        };
                        let x = number(get(request, "x")).unwrap_or(f64::NAN);
                        let y = number(get(request, "y")).unwrap_or(f64::NAN);
                        let hit = scope.pick(x, y, t, &options, &mut probe, &mut host);
                        let ink = probe.ink;
                        if fonts.incomplete() {
                            return Err(Unanswered::Fonts);
                        }
                        if ink && !outlines {
                            return Err(Unanswered::Ink);
                        }
                        match try_js!(hit) {
                            Some(l) => indexes(out, layers, &[l]),
                            None => out.raw("null"),
                        }
                    }
                    "textLayout" => {
                        let l = layer!();
                        let d = try_js!(scope.resolved_text_content(l, t));
                        let content = text_content(&d);
                        let caret = if pm_eval::text_level2::is_level2(l) {
                            scope
                                .level2_text(l, t, &mut probe, false, false)
                                .map(|frame| frame.caret_layout(&content))
                        } else {
                            Ok(probe.inner.compat().caret_layout(&content))
                        };
                        if fonts.incomplete() {
                            return Err(Unanswered::Fonts);
                        }
                        let caret = try_js!(caret);
                        match serde_json::to_string(&caret) {
                            Ok(json) => out.raw(&json),
                            Err(e) => return Ok(Err(JsThrow::type_error(e.to_string()))),
                        }
                    }
                    _ => {
                        let l = layer!();
                        let r = scope.text_raster(l, t, &mut probe);
                        let ink = probe.ink;
                        if fonts.incomplete() {
                            return Err(Unanswered::Fonts);
                        }
                        if ink && !outlines {
                            return Err(Unanswered::Ink);
                        }
                        let r = try_js!(r);
                        let mut first = true;
                        out.raw("{");
                        for (k, v) in [
                            ("w", r.w),
                            ("h", r.h),
                            ("anchorX", r.anchor_x),
                            ("anchorY", r.anchor_y),
                        ] {
                            out.key(&mut first, k);
                            out.num(v);
                        }
                        out.raw(",\"selection\":{");
                        let s = r.selection;
                        let mut first = true;
                        for (k, v) in [
                            ("x0", s.x0),
                            ("y0", s.y0),
                            ("x1", s.x1),
                            ("y1", s.y1),
                            ("w", s.w),
                            ("h", s.h),
                        ] {
                            out.key(&mut first, k);
                            out.num(v);
                        }
                        out.raw("}}");
                    }
                }
            }
            other => {
                if let Some(result) =
                    crate::queries::dispatch(other, &mut scope, fonts, layer, t, request, out)
                {
                    return result;
                }
                let _ = NoHost;
                return Ok(Err(JsThrow::type_error(format!("unknown query '{other}'"))));
            }
        }
        Ok(Ok(()))
    }
}

/// A fixed-size matrix from a JSON array (missing entries are NaN).
fn mat<const N: usize>(v: Option<&Value>) -> [f64; N] {
    let mut m = [f64::NAN; N];
    if let Some(items) = v.and_then(Value::as_array) {
        for (dst, src) in m.iter_mut().zip(items) {
            *dst = src.as_f64().unwrap_or(f64::NAN);
        }
    }
    m
}

/// The frame-rate validator's rate resolution (plan section 5, rule 1):
/// `{fps, fpsRational?}` from a float `fps` and an optional `[num, den]`.
fn frame_rate(request: &Value, out: &mut Reply) {
    let fps = number(get(request, "fps")).unwrap_or(f64::NAN);
    let rational = pm_model::ingest::fps_rational_of(get(request, "fpsRational"));
    match pm_time::rate_from_document(rational, fps) {
        Ok(r) => {
            let source = match r.source {
                pm_time::RateSource::Exact => "exact",
                pm_time::RateSource::Ntsc => "ntsc",
                pm_time::RateSource::Integer => "integer",
                pm_time::RateSource::Approximated => "approximated",
            };
            out.raw("{\"num\":");
            out.num(r.rate.num() as f64);
            out.raw(",\"den\":");
            out.num(r.rate.den() as f64);
            out.raw(",\"supported\":");
            out.bool(r.rate.is_supported());
            out.raw(",\"dividesFlick\":");
            out.bool(r.rate.divides_flick());
            out.raw(",\"source\":");
            out.str(source);
            out.raw("}");
        }
        Err(_) => out.raw("null"),
    }
}
