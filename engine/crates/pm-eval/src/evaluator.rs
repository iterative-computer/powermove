//! The evaluator: `PM.ev`, `PM.evP`, `PM.active`, and `PM.resolveContent`
//! as pure functions of (document, composition, time), plus the cached,
//! numerically addressed fast path the JavaScript bridge uses.
//!
//! # Two ways in
//!
//! * **By node** ([`Evaluator::ev`], [`Evaluator::ev_p`],
//!   [`Evaluator::active`], [`Evaluator::resolve_content`]): the legacy
//!   signatures over [`Node`] references. Nodes may belong to the document or
//!   be detached (`api.model.mkLayer(type)` evaluated ad hoc). These calls keep
//!   no per-node state: each prepares the track it reads (microseconds for
//!   ordinary tracks). Content-keyed caches apply (spring tables, compiled
//!   expressions), and `active` reads group ancestry through the
//!   composition's layer index, which is cached per `layers` node the way the
//!   legacy code caches it per `layers` array object (see "Invalidation").
//! * **By id** ([`Evaluator::intern`], [`Evaluator::ev_id`],
//!   [`Evaluator::ev_batch`]): a [`PropAddress`] (composition id, layer id,
//!   key, and how the key is read) interned to a [`PropId`]. Each id keeps its
//!   prepared track and its last answer, keyed by the document revision and
//!   the time, and survives edits that do not touch it (see "Invalidation").
//!   This is the JS fast path (section 7: numeric ids, results in a shared
//!   `Float64Array`) and what renderers use per frame.
//!
//! # Invalidation
//!
//! Caches are keyed by node identity (the layer id), input revision (the
//! `pm_model` node revisions), and time; a patch moves the revisions of what
//! it touched, so applying patches invalidates exactly the affected ids
//! without any notification. An id's prepared work is kept across a revision
//! when its layer is the same logical layer (the composition resolved to the
//! same node path, see [`Comp::at_root`]; the layer's `id` is unique in the
//! composition; and its `id` member did not change) and either the whole
//! layer is unchanged or, for object-only paths ([`Via::Channel`],
//! [`Via::Member`]), the addressed property node is unchanged. A node's
//! revision only moves forward, object members never move between keys, and
//! array elements only move within their array, so an unchanged revision at
//! the same path proves unchanged content. The path matters: the stored
//! form's open composition lives at the document root, and opening another
//! composition makes the same id resolve to its older copy in `comps`, whose
//! nodes are unchanged but are other nodes. Ids whose answer runs an
//! expression keep their last answer only while the document is unchanged,
//! since an expression may read anything in its composition.
//!
//! A composition's layer index (layer positions by id) is reused only for the
//! same `layers` node: the same document root and path, the same node
//! address, and no newer revision. The interned path also checks that the
//! layer found has the addressed id before using it
//! ([`CacheStats::stale_indexes`] counts repairs; zero unless the contract
//! below is broken).
//!
//! An evaluator follows one document's history: a revision names one state
//! of that document. The by-id calls reset the per-document caches when they
//! see a lower revision or another root node (another document, or the same
//! one moved in memory). Use [`Evaluator::reset`] after replacing a document
//! in place with an unrelated one at the same revision.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use pm_model::{Document, JsString, Node};

use crate::address::{self, DefinitionLabels, NoDefinitions, PropEntry};
use crate::comp::{Comp, LayerIndex, group_ancestors};
use crate::easing::SpringTables;
use crate::expr::{CompiledExpression, ExprContext, ExpressionEngine, messages};
use crate::js_sem::{JsResult, JsValue};
use crate::keyframes::{PreparedTrack, TemporalTrack, interpolates_color};
use crate::node::{self, Got};

/// Extension points for later lanes. Every method has the legacy behavior as
/// its default.
pub trait EvalHooks: Send + Sync {
    /// Takes over evaluation of a channel (`ev`, `evP`, and interned ids) when
    /// it returns `Some`. Consulted only when [`EvalHooks::overrides_channels`]
    /// is true.
    fn channel(
        &self,
        _cx: &Comp<'_>,
        _layer: Option<&Node>,
        _key: &JsString,
        _time: f64,
    ) -> Option<JsResult<JsValue>> {
        None
    }

    /// Whether [`EvalHooks::channel`] can return `Some`.
    fn overrides_channels(&self) -> bool {
        false
    }

    /// Refines `PM.active` (precomp time, transitions, ...). `base` is the
    /// legacy answer.
    fn active(
        &self,
        _cx: &Comp<'_>,
        _layer: Option<&Node>,
        _time: f64,
        base: bool,
        _eval: &mut dyn PropEval,
    ) -> JsResult<bool> {
        Ok(base)
    }

    /// Adds to `PM.resolveContent`'s result after the declared channels.
    fn resolve_content(
        &self,
        _cx: &Comp<'_>,
        _layer: &Node,
        _time: f64,
        _content: &mut Content,
        _eval: &mut dyn PropEval,
    ) -> JsResult<()> {
        Ok(())
    }

    /// Labels for extension-layer parameters in `allProps`.
    fn extension_labels(
        &self,
        definition: &JsValue,
        param: &JsString,
    ) -> (Option<JsValue>, Option<JsValue>) {
        NoDefinitions.extension_labels(definition, param)
    }
}

/// The legacy behavior.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoHooks;

impl EvalHooks for NoHooks {}

struct HookLabels<'a>(&'a dyn EvalHooks);

impl DefinitionLabels for HookLabels<'_> {
    fn extension_labels(
        &self,
        definition: &JsValue,
        param: &JsString,
    ) -> (Option<JsValue>, Option<JsValue>) {
        self.0.extension_labels(definition, param)
    }
}

/// `evP` for hooks.
pub trait PropEval {
    fn ev_p(
        &mut self,
        cx: &Comp<'_>,
        layer: Option<&Node>,
        prop: Option<&Node>,
        time: f64,
        key: &JsString,
    ) -> JsResult<JsValue>;
}

/// `resolveContent`'s result: `{ ...layer.d }` with channels evaluated.
pub type Content = std::collections::BTreeMap<JsString, JsValue>;

/// A numeric property id (the JS fast path).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PropId(u32);

impl PropId {
    pub fn index(self) -> u32 {
        self.0
    }

    pub fn from_index(index: u32) -> PropId {
        PropId(index)
    }
}

/// How an interned key is read from its layer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Via {
    /// `PM.ev(L, key, T)`: `L.p[key]`; a missing channel answers 0.
    Channel,
    /// `PM.evP(L, PM.findProp(L, key), T, key)`.
    Find,
    /// `PM.evP(L, L[m0][m1]…, T, key)` (`l.on` is `["on"]`, content channels
    /// are `["d", field]`); a missing member is `undefined`.
    Member(Vec<JsString>),
}

/// What an interned id addresses.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PropAddress {
    pub comp: JsString,
    /// The layer's `id` (first layer with that id).
    pub layer: JsString,
    pub key: JsString,
    pub via: Via,
}

impl PropAddress {
    /// A transform channel read with `PM.ev`.
    pub fn channel(comp: &str, layer: &str, key: &str) -> PropAddress {
        PropAddress {
            comp: JsString::from(comp),
            layer: JsString::from(layer),
            key: JsString::from(key),
            via: Via::Channel,
        }
    }

    /// Any key read through `findProp`.
    pub fn find(comp: &str, layer: &str, key: &str) -> PropAddress {
        PropAddress {
            comp: JsString::from(comp),
            layer: JsString::from(layer),
            key: JsString::from(key),
            via: Via::Find,
        }
    }
}

/// The kind of a value written by [`Evaluator::ev_batch`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ValueKind {
    /// The value is the number written.
    Number = 0,
    /// The value is not a number; NaN was written. Ask [`Evaluator::ev_id`].
    Other = 1,
    /// Evaluation threw; NaN was written.
    Threw = 2,
}

/// Counters of the interned-id caches (for tests and diagnostics).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// Plans built (an id resolved and its track prepared).
    pub plans: u64,
    /// Plans kept across a document revision.
    pub reused: u64,
    /// Answers served from an id's last answer at the same time (at the same
    /// revision, or kept across a revision that provably did not change it).
    pub memo_hits: u64,
    /// Layer indexes built.
    pub indexes: u64,
    /// Index lookups that found a layer without the addressed id and rebuilt
    /// the index. Zero unless the caller broke the one-document contract
    /// (see the module documentation).
    pub stale_indexes: u64,
}

/// Where expression errors are kept (`PM.expressionErrors`, keyed by the
/// channel object in JavaScript).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ErrorKey {
    pub comp: JsString,
    /// The layer's id, as a string (`""` when the layer has none).
    pub layer: JsString,
    pub key: JsString,
}

const EXPR_CACHE_MAX: usize = 256;

#[derive(Default)]
struct CompiledCache {
    map: HashMap<JsString, Option<Arc<dyn CompiledExpression>>>,
    order: VecDeque<JsString>,
}

impl CompiledCache {
    fn get(
        &mut self,
        engine: Option<&dyn ExpressionEngine>,
        source: &JsString,
    ) -> Option<Arc<dyn CompiledExpression>> {
        if let Some(hit) = self.map.get(source) {
            return hit.clone();
        }
        let compiled = engine.and_then(|e| e.compile(source));
        if self.map.len() >= EXPR_CACHE_MAX
            && let Some(old) = self.order.pop_front()
        {
            self.map.remove(&old);
        }
        self.order.push_back(source.clone());
        self.map.insert(source.clone(), compiled.clone());
        compiled
    }
}

#[derive(Clone, Debug)]
enum Plan {
    /// The answer does not depend on time.
    Const(JsResult<JsValue>),
    /// A temporal track read at `T - from`; `first` answers non-finite times.
    Temporal {
        track: Arc<TemporalTrack>,
        first: JsValue,
    },
    /// Read the nodes at every evaluation (mixed tracks, expressions, reads
    /// that throw).
    Live { expression: bool },
}

#[derive(Clone, Debug)]
struct SlotState {
    validated: u64,
    /// Whether the layer's id was unique in the composition when validated.
    unique: bool,
    /// [`Comp::at_root`] when validated: a layer under another path is
    /// another node, whatever its revision.
    at_root: bool,
    plan: Plan,
    from: f64,
    memo: Option<(u64, JsResult<JsValue>)>,
}

#[derive(Clone, Debug)]
struct Slot {
    address: Arc<PropAddress>,
    state: Option<SlotState>,
    live: bool,
}

/// A composition's layer index and what it was built from. It is reused only
/// for the same `layers` node: the same document root (by address), the same
/// resolution path ([`Comp::at_root`]), the same `layers` node address, and
/// unchanged indexed inputs (ids and nullish positions).
struct IndexEntry {
    validated: u64,
    len: usize,
    /// The address of the document root it was built in.
    root: usize,
    /// Whether the composition was the document root.
    at_root: bool,
    /// The address of the `layers` node (0 when there was none).
    layers: usize,
    index: Arc<LayerIndex>,
    // The index reads only each position's nullish state and id. A drag
    // changes the layers' subtree revision, but neither input. Addresses
    // detect reordered nodes even when reconciliation preserves revisions.
    ids: Vec<(bool, usize)>,
}

fn index_input(layer: &Node) -> (bool, usize) {
    (
        node::is_nullish(Some(layer)),
        address_of(node::member(Some(layer), "id")),
    )
}

fn address_of(node: Option<&Node>) -> usize {
    node.map_or(0, |n| n as *const Node as usize)
}

/// Compositions resolved during one call (a batch shares them).
#[derive(Default)]
struct Resolved<'d>(Vec<(JsString, Option<ResolvedComp<'d>>)>);

type ResolvedComp<'d> = (Comp<'d>, Arc<LayerIndex>);

/// The evaluator. See the module documentation.
pub struct Evaluator {
    expressions: Option<Arc<dyn ExpressionEngine>>,
    hooks: Arc<dyn EvalHooks>,
    springs: SpringTables,
    compiled: CompiledCache,
    errors: HashMap<ErrorKey, String>,
    ids: HashMap<PropAddress, PropId>,
    slots: Vec<Slot>,
    free: Vec<u32>,
    indexes: HashMap<JsString, IndexEntry>,
    last_revision: u64,
    /// The address of the root of the document last seen by id.
    last_root: usize,
    stats: CacheStats,
}

impl Default for Evaluator {
    fn default() -> Evaluator {
        Evaluator::new()
    }
}

impl core::fmt::Debug for Evaluator {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Evaluator")
            .field("expressions", &self.expressions.is_some())
            .field("ids", &self.ids.len())
            .field("last_revision", &self.last_revision)
            .finish_non_exhaustive()
    }
}

fn layer_id_string(layer: Option<&Node>) -> JsString {
    node::string(node::member(layer, "id"))
        .cloned()
        .unwrap_or_default()
}

/// `T - L.from` (throws when `L` is `null` or `undefined`).
fn local_time(layer: Option<&Node>, time: f64) -> JsResult<f64> {
    Ok(time - node::get_checked(layer, b"from")?.to_js().to_number())
}

/// `prop.kf?.length` is truthy.
fn kf_length_truthy(prop: &Got<'_>) -> bool {
    match prop {
        Got::Node(p) => {
            let kf = node::get(p, b"kf");
            match kf {
                Got::Node(k) if !node::is_nullish(Some(k)) => node::get(k, b"length").truthy(),
                Got::Value(JsValue::String(s)) => s.utf16_len() > 0,
                _ => false,
            }
        }
        _ => false,
    }
}

impl Evaluator {
    /// An evaluator without an expression engine (every expression reports
    /// invalid syntax) and with the legacy hooks.
    pub fn new() -> Evaluator {
        Evaluator {
            expressions: None,
            hooks: Arc::new(NoHooks),
            springs: SpringTables::new(),
            compiled: CompiledCache::default(),
            errors: HashMap::new(),
            ids: HashMap::new(),
            slots: Vec::new(),
            free: Vec::new(),
            indexes: HashMap::new(),
            last_revision: 0,
            last_root: 0,
            stats: CacheStats::default(),
        }
    }

    /// Uses `engine` for expressions.
    pub fn with_expressions(mut self, engine: Arc<dyn ExpressionEngine>) -> Evaluator {
        self.expressions = Some(engine);
        self.compiled = CompiledCache::default();
        self
    }

    /// Uses `hooks`.
    pub fn with_hooks(mut self, hooks: Arc<dyn EvalHooks>) -> Evaluator {
        self.hooks = hooks;
        self
    }

    /// Drops every per-document cache (interned ids stay valid and re-resolve).
    pub fn reset(&mut self) {
        for slot in &mut self.slots {
            slot.state = None;
        }
        self.indexes.clear();
        self.errors.clear();
        self.last_revision = 0;
        self.last_root = 0;
    }

    /// Follows the document the by-id API is called with: a lower revision or
    /// another root node (a different document, or the same one moved in
    /// memory) drops the per-document caches.
    fn observe(&mut self, doc: &Document) {
        let revision = doc.revision();
        let root = address_of(doc.root());
        if revision < self.last_revision || (self.last_root != 0 && root != self.last_root) {
            self.reset();
        }
        self.last_revision = revision;
        self.last_root = root;
    }

    /// The interned-id cache counters.
    pub fn cache_stats(&self) -> CacheStats {
        self.stats
    }

    /// The spring tables built so far.
    pub fn springs(&mut self) -> &mut SpringTables {
        &mut self.springs
    }

    /// `PM.expressionErrors.get(prop)` for a channel.
    pub fn expression_error(&self, comp: &str, layer: &str, key: &str) -> Option<&str> {
        self.errors
            .get(&ErrorKey {
                comp: JsString::from(comp),
                layer: JsString::from(layer),
                key: JsString::from(key),
            })
            .map(String::as_str)
    }

    /// All recorded expression errors.
    pub fn expression_errors(&self) -> impl Iterator<Item = (&ErrorKey, &str)> {
        self.errors.iter().map(|(k, v)| (k, v.as_str()))
    }

    fn error_key(cx: &Comp<'_>, layer: Option<&Node>, key: &JsString) -> ErrorKey {
        ErrorKey {
            comp: cx.id().clone(),
            layer: layer_id_string(layer),
            key: key.clone(),
        }
    }

    /// Forgets the expression error of a channel that evaluated without one.
    /// Most documents record none, and building the key (three strings, a
    /// SipHash) on every channel read cost 12% of a viewer frame
    /// (docs/engine/perf.md), so an empty map answers at once.
    fn clear_error(&mut self, cx: &Comp<'_>, layer: Option<&Node>, key: &JsString) {
        if !self.errors.is_empty() {
            self.errors.remove(&Self::error_key(cx, layer, key));
        }
    }

    // ── by node ──────────────────────────────────────────────────────────

    /// `evalKfs(kf, t, interpolateColor)`.
    pub fn eval_keyframes(
        &mut self,
        kf: Option<&Node>,
        t: f64,
        interpolate_color: bool,
    ) -> JsResult<JsValue> {
        crate::keyframes::evaluate(kf, t, interpolate_color, &mut self.springs)
    }

    /// `PM.ev(L, key, T)`: one channel of `L.p` at composition time `T`.
    pub fn ev(
        &mut self,
        cx: &Comp<'_>,
        layer: Option<&Node>,
        key: &JsString,
        time: f64,
    ) -> JsResult<JsValue> {
        if self.hooks.overrides_channels()
            && let Some(r) = self.hooks.clone().channel(cx, layer, key, time)
        {
            return r;
        }
        let p = node::get_checked(layer, b"p")?;
        let prop = match p {
            Got::Node(p) => node::get_checked(Some(p), key.as_bytes())?,
            _ => node::get_checked(None, key.as_bytes())?,
        };
        if !prop.truthy() {
            return Ok(JsValue::Number(0.0));
        }
        let expr = match &prop {
            Got::Node(n) => node::get(n, b"expr"),
            _ => Got::Undefined,
        };
        let color = interpolates_color(key.as_bytes());
        if !expr.truthy() {
            self.clear_error(cx, layer, key);
            return self.static_or_track(&prop, layer, time, color);
        }
        let tl = local_time(layer, time)?;
        let prop_node = prop.node().expect("an expression lives on an object");
        self.eval_prop(cx, layer, prop_node, &expr, tl, time, key)
    }

    /// `prop.kf?.length ? evalKfs(prop.kf, tl, color) : prop.v`, with `tl`
    /// (`T - L.from`) read only for a track. `L` is an object here (its `p`
    /// was read), so reading `L.from` cannot throw and the order of reads
    /// does not show.
    fn static_or_track(
        &mut self,
        prop: &Got<'_>,
        layer: Option<&Node>,
        time: f64,
        color: bool,
    ) -> JsResult<JsValue> {
        if kf_length_truthy(prop) {
            let tl = local_time(layer, time)?;
            let kf = prop.node().and_then(|p| node::member(Some(p), "kf"));
            return self.eval_keyframes(kf, tl, color);
        }
        Ok(match prop {
            Got::Node(p) => node::get(p, b"v").to_js(),
            _ => JsValue::Undefined,
        })
    }

    /// `PM.evP(L, prop, T, key)`: any channel object at composition time `T`.
    pub fn ev_p(
        &mut self,
        cx: &Comp<'_>,
        layer: Option<&Node>,
        prop: Option<&Node>,
        time: f64,
        key: &JsString,
    ) -> JsResult<JsValue> {
        if self.hooks.overrides_channels()
            && let Some(r) = self.hooks.clone().channel(cx, layer, key, time)
        {
            return r;
        }
        let Some(p) = prop.filter(|p| {
            node::truthy(Some(p))
                && matches!(
                    p.kind(),
                    pm_model::NodeKind::Object(_) | pm_model::NodeKind::Array(_)
                )
        }) else {
            return Ok(JsValue::Null);
        };
        let expr = node::get(p, b"expr");
        let color = interpolates_color(key.as_bytes());
        if !expr.truthy() {
            self.clear_error(cx, layer, key);
            let got = Got::Node(p);
            if kf_length_truthy(&got) {
                let tl = local_time(layer, time)?;
                return self.eval_keyframes(node::member(Some(p), "kf"), tl, color);
            }
            return Ok(node::get(p, b"v").to_js());
        }
        let tl = local_time(layer, time)?;
        self.eval_prop(cx, layer, p, &expr, tl, time, key)
    }

    /// `evalProp(prop, tLocal, ctx)` for a channel with a truthy `expr`.
    #[allow(clippy::too_many_arguments)]
    fn eval_prop(
        &mut self,
        cx: &Comp<'_>,
        layer: Option<&Node>,
        prop: &Node,
        expr: &Got<'_>,
        tl: f64,
        time: f64,
        key: &JsString,
    ) -> JsResult<JsValue> {
        let keys = node::member(Some(prop), "kf")
            .filter(|k| node::array(Some(k)).is_some_and(|a| !a.is_empty()));
        let value = match keys {
            Some(kf) => self.eval_keyframes(Some(kf), tl, interpolates_color(key.as_bytes()))?,
            None => node::get(prop, b"v").to_js(),
        };
        let compiled = match expr {
            Got::Node(n) => node::string(Some(n)).cloned(),
            _ => None,
        }
        .and_then(|src| self.compiled.get(self.expressions.as_deref(), &src));
        let error_key = Self::error_key(cx, layer, key);
        let Some(compiled) = compiled else {
            self.errors.insert(error_key, messages::INVALID.to_owned());
            return Ok(value);
        };
        let fps = cx.expression_fps();
        let idx = if compiled.needs_idx() {
            cx.index_of_layer(layer)
        } else {
            0.0
        };
        let context = ExprContext {
            t: tl,
            time,
            fps: &fps,
            frame: cx.frame_at(time),
            value: &value,
            layer,
            comp: cx,
            channel: key,
            idx,
        };
        match compiled.evaluate(&context) {
            Ok(r) if r.is_finite_number() || matches!(r, JsValue::String(_) | JsValue::Bool(_)) => {
                self.errors.remove(&error_key);
                Ok(r)
            }
            Ok(_) => {
                self.errors
                    .insert(error_key, messages::NOT_FINITE.to_owned());
                Ok(value)
            }
            Err(e) => {
                self.errors.insert(
                    error_key,
                    e.message.unwrap_or_else(|| messages::FAILED.to_owned()),
                );
                Ok(value)
            }
        }
    }

    /// The composition's layer index, reused while its `layers` node provably
    /// did not change (see [`IndexEntry`]).
    pub(crate) fn layer_index(&mut self, cx: &Comp<'_>) -> Arc<LayerIndex> {
        let layers_node = cx.layers_node();
        let layers_rev = layers_node.map_or(0, Node::rev);
        let len = cx.layers().len();
        let root = address_of(Some(cx.root()));
        let layers = address_of(layers_node);
        if let Some(entry) = self.indexes.get_mut(cx.id())
            && entry.root == root
            && entry.at_root == cx.at_root()
            && entry.layers == layers
            && entry.validated <= cx.revision()
            && entry.len == len
            && (layers_rev <= entry.validated
                || cx.layers().iter().zip(&entry.ids).all(|(layer, input)| {
                    index_input(layer) == *input
                        && node::member(Some(layer), "id")
                            .is_none_or(|id| id.rev() <= entry.validated)
                }))
        {
            entry.validated = cx.revision();
            return entry.index.clone();
        }
        self.build_layer_index(cx)
    }

    fn build_layer_index(&mut self, cx: &Comp<'_>) -> Arc<LayerIndex> {
        let index = Arc::new(LayerIndex::build(cx.layers()));
        self.stats.indexes += 1;
        self.indexes.insert(
            cx.id().clone(),
            IndexEntry {
                validated: cx.revision(),
                len: cx.layers().len(),
                root: address_of(Some(cx.root())),
                at_root: cx.at_root(),
                layers: address_of(cx.layers_node()),
                index: index.clone(),
                ids: cx.layers().iter().map(index_input).collect(),
            },
        );
        index
    }

    /// `PM.groupAncestors(L)` in the composition.
    pub fn group_ancestors<'d>(
        &mut self,
        cx: &Comp<'d>,
        layer: &'d Node,
    ) -> JsResult<Vec<&'d Node>> {
        let index = self.layer_index(cx);
        group_ancestors(layer, cx.layers(), &index)
    }

    /// `isProperty(x.on) ? evP(x, x.on, T, 'l.on') : x.on`, truthy.
    fn on(&mut self, cx: &Comp<'_>, layer: Option<&Node>, time: f64) -> JsResult<bool> {
        let on = node::member(layer, "on");
        if node::is_property(on) {
            Ok(self
                .ev_p(cx, layer, on, time, &JsString::from("l.on"))?
                .truthy())
        } else {
            Ok(node::truthy(on))
        }
    }

    /// `PM.active(L, T)`: whether the layer is on and its time span holds `T`.
    pub fn active(&mut self, cx: &Comp<'_>, layer: Option<&Node>, time: f64) -> JsResult<bool> {
        let base = self.active_base(cx, layer, time, false)?;
        let hooks = self.hooks.clone();
        hooks.active(cx, layer, time, base, self)
    }

    /// `PM.active({ ...L, on: true }, T)`: the track matte pass's reading of
    /// a matte source, which renders even when the source is switched off
    /// (its enclosing groups and time span still apply).
    pub fn active_forced_on(
        &mut self,
        cx: &Comp<'_>,
        layer: Option<&Node>,
        time: f64,
    ) -> JsResult<bool> {
        let base = self.active_base(cx, layer, time, true)?;
        let hooks = self.hooks.clone();
        hooks.active(cx, layer, time, base, self)
    }

    fn active_base(
        &mut self,
        cx: &Comp<'_>,
        layer: Option<&Node>,
        time: f64,
        forced_on: bool,
    ) -> JsResult<bool> {
        if let Some(l) = layer
            && node::truthy(node::member(Some(l), "group"))
        {
            for group in self.group_ancestors(cx, l)? {
                if !self.on(cx, Some(group), time)? {
                    return Ok(false);
                }
            }
        }
        let start = node::to_number(node::member(layer, "from"));
        let duration = node::to_number(node::member(layer, "dur"));
        if !(forced_on || self.on(cx, layer, time)?)
            || !start.is_finite()
            || !duration.is_finite()
            || duration <= 0.0
            || !time.is_finite()
        {
            return Ok(false);
        }
        if node::is_str(node::member(layer, "type"), "group") {
            return Ok(true);
        }
        let end = start + duration;
        if time < start - 1e-6 {
            return Ok(false);
        }
        let comp_end = cx.duration();
        Ok(time < end - 1e-6
            || (comp_end.is_finite()
                && (time - comp_end).abs() <= 1e-6
                && (end - comp_end).abs() <= 1e-6))
    }

    /// `PM.resolveContent(L, T)`: `{ ...L.d }` with every declared content
    /// channel (and `fontAxis.*` on text) evaluated.
    pub fn resolve_content(&mut self, cx: &Comp<'_>, layer: &Node, time: f64) -> JsResult<Content> {
        let d = node::get_checked(Some(layer), b"d")?;
        let mut content = Content::new();
        let mut sources: Vec<(JsString, Got<'_>)> = Vec::new();
        if let Got::Node(d) = &d
            && !node::is_nullish(Some(d))
        {
            sources = node::entries(d);
        }
        let ty = node::to_js_string(node::member(Some(layer), "type"));
        let fields = address::content_fields(ty.as_bytes());
        let is_text = ty.as_bytes() == b"text";
        for (key, value) in &sources {
            let declared = fields.iter().any(|f| f.as_bytes() == key.as_bytes())
                || (is_text && address::is_font_axis_key(key.as_bytes()));
            let resolved = if declared && node::is_property(value.node()) {
                let path = concat_key("c.", key);
                self.ev_p(cx, Some(layer), value.node(), time, &path)?
            } else {
                value.to_js()
            };
            content.insert(key.clone(), resolved);
        }
        let hooks = self.hooks.clone();
        hooks.resolve_content(cx, layer, time, &mut content, self)?;
        Ok(content)
    }

    /// `PM.findProp(L, key)`.
    pub fn find_prop<'d>(&self, layer: &'d Node, key: &JsString) -> JsResult<Got<'d>> {
        address::find_prop(layer, key)
    }

    /// `PM.allProps(L)`, with extension labels from the hooks.
    pub fn all_props<'d>(&self, layer: &'d Node) -> JsResult<Vec<PropEntry<'d>>> {
        address::all_props(layer, &HookLabels(self.hooks.as_ref()))
    }

    // ── by id ────────────────────────────────────────────────────────────

    /// The id of an address, interning it on first use.
    pub fn intern(&mut self, address: PropAddress) -> PropId {
        if let Some(&id) = self.ids.get(&address) {
            return id;
        }
        let slot = Slot {
            address: Arc::new(address.clone()),
            state: None,
            live: true,
        };
        let id = match self.free.pop() {
            Some(i) => {
                self.slots[i as usize] = slot;
                PropId(i)
            }
            None => {
                self.slots.push(slot);
                PropId((self.slots.len() - 1) as u32)
            }
        };
        self.ids.insert(address, id);
        id
    }

    /// The address of an id.
    pub fn address(&self, id: PropId) -> Option<&PropAddress> {
        self.slots
            .get(id.0 as usize)
            .filter(|s| s.live)
            .map(|s| s.address.as_ref())
    }

    /// Releases an id; it may be reused by a later [`Evaluator::intern`].
    pub fn release(&mut self, id: PropId) {
        if let Some(slot) = self.slots.get_mut(id.0 as usize)
            && slot.live
        {
            slot.live = false;
            slot.state = None;
            self.ids.remove(slot.address.as_ref());
            self.free.push(id.0);
        }
    }

    /// The number of live ids.
    pub fn interned(&self) -> usize {
        self.ids.len()
    }

    /// Evaluates an interned id at composition time `T` (seconds).
    pub fn ev_id(&mut self, doc: &Document, id: PropId, time: f64) -> JsResult<JsValue> {
        self.ev_id_in(doc, id, time, &mut Resolved::default())
    }

    fn ev_id_in<'d>(
        &mut self,
        doc: &'d Document,
        id: PropId,
        time: f64,
        resolved: &mut Resolved<'d>,
    ) -> JsResult<JsValue> {
        self.observe(doc);
        let revision = doc.revision();
        let index = id.0 as usize;
        let Some(slot) = self.slots.get(index).filter(|s| s.live) else {
            return Ok(JsValue::Undefined);
        };
        let time_bits = time.to_bits();
        if let Some(state) = &slot.state
            && state.validated == revision
        {
            if let Some((t, value)) = &state.memo
                && *t == time_bits
            {
                self.stats.memo_hits += 1;
                return value.clone();
            }
            match &state.plan {
                Plan::Const(value) => return value.clone(),
                Plan::Temporal { track, first } => {
                    let tl = time - state.from;
                    let value = if tl.is_finite() {
                        Ok(track.value(tl))
                    } else {
                        Ok(first.clone())
                    };
                    let state = self.slots[index].state.as_mut().expect("state");
                    state.memo = Some((time_bits, value.clone()));
                    return value;
                }
                Plan::Live { .. } => {}
            }
        }
        let value = self.ev_slot_slow(doc, index, time, resolved);
        if let Some(state) = self.slots[index].state.as_mut() {
            state.memo = Some((time_bits, value.clone()));
        }
        value
    }

    /// Evaluates an interned id at a tick of its composition's grid. The tick
    /// becomes the document's own float seconds ([`Comp::seconds_at`]), so a
    /// frame-aligned tick evaluates exactly where the legacy evaluator does.
    pub fn ev_id_ticks(
        &mut self,
        doc: &Document,
        id: PropId,
        ticks: pm_time::Tick,
    ) -> JsResult<JsValue> {
        let Some(address) = self.address(id) else {
            return Ok(JsValue::Undefined);
        };
        let seconds = match Comp::resolve(doc, &address.comp.to_string_lossy()) {
            Ok(cx) => cx.seconds_at(ticks),
            Err(_) => pm_time::seconds_from_ticks(ticks),
        };
        self.ev_id(doc, id, seconds)
    }

    /// Evaluates many ids at one time into `out` (numbers) and `kinds`.
    ///
    /// # Panics
    ///
    /// Panics if `out` or `kinds` is shorter than `ids`.
    pub fn ev_batch(
        &mut self,
        doc: &Document,
        ids: &[PropId],
        time: f64,
        out: &mut [f64],
        kinds: &mut [ValueKind],
    ) {
        assert!(out.len() >= ids.len() && kinds.len() >= ids.len());
        let mut resolved = Resolved::default();
        for (i, &id) in ids.iter().enumerate() {
            match self.ev_id_in(doc, id, time, &mut resolved) {
                Ok(JsValue::Number(n)) => {
                    out[i] = n;
                    kinds[i] = ValueKind::Number;
                }
                Ok(_) => {
                    out[i] = f64::NAN;
                    kinds[i] = ValueKind::Other;
                }
                Err(_) => {
                    out[i] = f64::NAN;
                    kinds[i] = ValueKind::Threw;
                }
            }
        }
    }

    /// Revalidates (or plans) a slot at the current revision and evaluates.
    fn ev_slot_slow<'d>(
        &mut self,
        doc: &'d Document,
        index: usize,
        time: f64,
        resolved: &mut Resolved<'d>,
    ) -> JsResult<JsValue> {
        let revision = doc.revision();
        let address = self.slots[index].address.clone();
        let entry = match resolved.0.iter().position(|(id, _)| *id == address.comp) {
            Some(i) => i,
            None => {
                let comp = Comp::resolve(doc, &address.comp.to_string_lossy())
                    .ok()
                    .map(|cx| {
                        let index = self.layer_index(&cx);
                        (cx, index)
                    });
                resolved.0.push((address.comp.clone(), comp));
                resolved.0.len() - 1
            }
        };
        let Some((cx, layer_index)) = &resolved.0[entry].1 else {
            self.slots[index].state = None;
            return Ok(JsValue::Undefined);
        };
        let mut found = layer_index.strings.get(&address.layer).copied();
        let mut layers = cx.layers();
        // The layer's `id` node, read once: checked here and reused below.
        let mut id_node = found.and_then(|(pos, _)| node::member(layers.get(pos), "id"));
        // The index belongs to this `layers` node (see `layer_index`), so the
        // layer at `pos` has the addressed id. Check anyway: a caller that
        // breaks the one-document contract gets a rebuilt index, not another
        // layer's answer.
        if found.is_some() && node::string(id_node).is_none_or(|s| *s != address.layer) {
            self.stats.stale_indexes += 1;
            let rebuilt = self.build_layer_index(cx);
            found = rebuilt.strings.get(&address.layer).copied();
            if let Some((_, slot_index)) = resolved.0[entry].1.as_mut() {
                *slot_index = rebuilt;
            }
            let Some((cx, _)) = &resolved.0[entry].1 else {
                unreachable!("resolved above");
            };
            layers = cx.layers();
            id_node = found.and_then(|(pos, _)| node::member(layers.get(pos), "id"));
        }
        let Some((cx, _)) = &resolved.0[entry].1 else {
            unreachable!("resolved above");
        };
        let Some((pos, duplicated)) = found else {
            self.slots[index].state = None;
            return Ok(JsValue::Undefined);
        };
        let layer = &layers[pos];
        let unique = !duplicated;
        let at_root = cx.at_root();

        // Keep the previous plan when the inputs provably did not change.
        let keep = match &self.slots[index].state {
            Some(state) if state.validated <= revision => {
                // Unique now and then: the same logical layer (see the module
                // documentation).
                let same_layer = unique
                    && state.unique
                    && state.at_root == at_root
                    && id_node.is_some_and(|n| n.rev() <= state.validated);
                let unchanged = same_layer
                    && (layer.rev() <= state.validated
                        || match &address.via {
                            Via::Channel => {
                                node::member(node::member(Some(layer), "p"), &key_str(&address.key))
                                    .is_some_and(|n| n.rev() <= state.validated)
                            }
                            Via::Member(path) => {
                                member_path(layer, path).is_some_and(|n| n.rev() <= state.validated)
                            }
                            Via::Find => false,
                        });
                let expression = matches!(state.plan, Plan::Live { expression: true });
                if unchanged {
                    let from = node::to_number(node::member(Some(layer), "from"));
                    let memo_ok = if expression {
                        cx.root().rev() <= state.validated
                    } else {
                        from.to_bits() == state.from.to_bits()
                    };
                    Some((memo_ok, from))
                } else {
                    None
                }
            }
            _ => None,
        };
        match keep {
            Some((memo_ok, from)) => {
                self.stats.reused += 1;
                let state = self.slots[index].state.as_mut().expect("state");
                state.validated = revision;
                state.unique = unique;
                state.from = from;
                if !memo_ok {
                    state.memo = None;
                }
            }
            None => {
                self.stats.plans += 1;
                let (plan, from) = self.plan(cx, layer, &address);
                self.slots[index].state = Some(SlotState {
                    validated: revision,
                    unique,
                    at_root,
                    plan,
                    from,
                    memo: None,
                });
            }
        }
        let state = self.slots[index].state.as_ref().expect("planned");
        if let Some((t, value)) = &state.memo
            && *t == time.to_bits()
        {
            self.stats.memo_hits += 1;
            return value.clone();
        }
        match &state.plan {
            Plan::Const(value) => value.clone(),
            Plan::Temporal { track, first } => {
                let tl = time - state.from;
                Ok(if tl.is_finite() {
                    track.value(tl)
                } else {
                    first.clone()
                })
            }
            Plan::Live { .. } => self.ev_address(cx, layer, &address, time),
        }
    }

    /// Evaluates an address on its nodes, uncached.
    fn ev_address(
        &mut self,
        cx: &Comp<'_>,
        layer: &Node,
        address: &PropAddress,
        time: f64,
    ) -> JsResult<JsValue> {
        match &address.via {
            Via::Channel => self.ev(cx, Some(layer), &address.key, time),
            Via::Find => {
                let prop = address::find_prop(layer, &address.key)?;
                self.ev_p(cx, Some(layer), prop.node(), time, &address.key)
            }
            Via::Member(path) => {
                let prop = member_path(layer, path);
                self.ev_p(cx, Some(layer), prop, time, &address.key)
            }
        }
    }

    /// Decides how an address evaluates at this revision.
    fn plan(&mut self, cx: &Comp<'_>, layer: &Node, address: &PropAddress) -> (Plan, f64) {
        let from = node::to_number(node::member(Some(layer), "from"));
        if self.hooks.overrides_channels() {
            return (Plan::Live { expression: true }, from);
        }
        // The property, with ev's and evP's own answers for missing ones.
        let prop: Got<'_> = match &address.via {
            Via::Channel => {
                let p = node::member(Some(layer), "p");
                match p {
                    Some(p) if !node::is_nullish(Some(p)) => node::get(p, address.key.as_bytes()),
                    _ => return (Plan::Live { expression: false }, from),
                }
            }
            Via::Find => match address::find_prop(layer, &address.key) {
                Ok(p) => p,
                Err(e) => return (Plan::Const(Err(e)), from),
            },
            Via::Member(path) => member_path(layer, path).map_or(Got::Undefined, Got::Node),
        };
        let missing = match &address.via {
            Via::Channel => !prop.truthy(),
            _ => {
                !matches!(
                    prop.node().map(Node::kind),
                    Some(pm_model::NodeKind::Object(_) | pm_model::NodeKind::Array(_))
                ) || !prop.truthy()
            }
        };
        if missing {
            let value = if address.via == Via::Channel {
                JsValue::Number(0.0)
            } else {
                JsValue::Null
            };
            return (Plan::Const(Ok(value)), from);
        }
        // A computed value (a string's character) has no members.
        let Some(p) = prop.node() else {
            return (Plan::Const(Ok(JsValue::Undefined)), from);
        };
        if node::get(p, b"expr").truthy() {
            return (Plan::Live { expression: true }, from);
        }
        self.clear_error(cx, Some(layer), &address.key);
        if !kf_length_truthy(&prop) {
            return (Plan::Const(Ok(node::get(p, b"v").to_js())), from);
        }
        let kf = node::member(Some(p), "kf");
        match PreparedTrack::prepare(kf, &mut self.springs) {
            PreparedTrack::Null => (Plan::Const(Ok(JsValue::Null)), from),
            PreparedTrack::Temporal(track) => {
                let first = node::array(kf)
                    .and_then(|k| k.first())
                    .map_or(JsValue::Null, |k| {
                        node::get(k, b"v").to_js().or_nullish(JsValue::Null)
                    });
                (Plan::Temporal { track, first }, from)
            }
            _ => (Plan::Live { expression: false }, from),
        }
    }
}

fn key_str(key: &JsString) -> String {
    key.to_string_lossy()
}

/// `prefix + key` as a JavaScript string.
fn concat_key(prefix: &str, key: &JsString) -> JsString {
    match key.as_str() {
        Some(k) => JsString::from(format!("{prefix}{k}")),
        None => {
            let units: Vec<u16> = prefix.encode_utf16().chain(key.utf16_units()).collect();
            JsString::from_utf16_units(&units)
        }
    }
}

/// `L[m0][m1]…` through object members, `None` where one is missing.
fn member_path<'d>(layer: &'d Node, path: &[JsString]) -> Option<&'d Node> {
    let mut node = layer;
    for part in path {
        node = match node::get(node, part.as_bytes()) {
            Got::Node(n) => n,
            _ => return None,
        };
    }
    Some(node)
}

impl PropEval for Evaluator {
    fn ev_p(
        &mut self,
        cx: &Comp<'_>,
        layer: Option<&Node>,
        prop: Option<&Node>,
        time: f64,
        key: &JsString,
    ) -> JsResult<JsValue> {
        Evaluator::ev_p(self, cx, layer, prop, time, key)
    }
}
