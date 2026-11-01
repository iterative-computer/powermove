//! The synchronous query context.
//!
//! [`QueryContext`] owns a [`pm_wasm::Engine`] used as a plain Rust library
//! (docs/gpui/engine-boundary.md, sections 2.3 and 5.2). The
//! [`DocumentStore`](crate::DocumentStore) feeds it every committed
//! transaction **before** it notifies subscribers, so a UI handler that runs
//! for revision *R* and then asks a question always gets an answer at *R*
//! (never at *R−1*, never at a later revision).
//!
//! The context never mutates the document by itself; it is a read mirror whose
//! tree is shared (copy-on-write) with the authoritative document.
//!
//! # What exists in `pm-wasm` and is wrapped here
//!
//! | Method | Engine query | Notes |
//! |---|---|---|
//! | [`QueryContext::layers`] | none (reads the tree) | the layers panel rows of a composition |
//! | [`QueryContext::evaluate`] | `ev` | one channel of `layer.p` (`opacity`, `position.x`, ...) at composition seconds; honours keyframes and expressions |
//! | [`QueryContext::world_opacity`] | `worldOpacity` | opacity multiplied through parents |
//! | [`QueryContext::local_matrix`], [`world_matrix`](QueryContext::world_matrix) | `localMatrix`, `worldMatrix` | 2D affine `[a, b, c, d, e, f]` |
//! | [`QueryContext::bounds`], [`group_bounds`](QueryContext::group_bounds) | `bounds`, `groupBounds` | `Bounds`; text, 3D models and fonts may answer *needs-X* instead |
//! | [`QueryContext::pick`] | `pick` | hit test in composition pixels, returns a layer **id** |
//! | [`QueryContext::query`] | any | raw JSON request/reply for everything else (`allProps`, `textLayout`, `hasKeyAt`, `edit.*` kernels, ...) |
//!
//! Not wrapped yet (TODO, tracked in `docs/gpui/core-api.md`): typed wrappers
//! for text layout/caret geometry (`textLayout`, `textRaster`), 3D object
//! boxes and cursor hits, the `edit.*` kernels (they return patches that
//! must become commands, never be applied directly), the interned
//! `intern`/`ev_id`/`ev_batch` fast path for drags, and font/model preloading
//! (`add_font_answers`, `set_font_outlines`, `set_model`; reachable through
//! [`QueryContext::engine_mut`] today). Answers are not cached per
//! revision/time yet (section 5.2).
//!
//! Answers can be *conditional*: the engine replies `fonts`, `ink`, `models`,
//! `host`, or `unsupported` instead of a value when it lacks an input. These
//! are surfaced as [`QueryError::NeedsFonts`] etc. and must **not** be read as
//! "no layer".

use pm_model::consistency::ConsistencyHash;
use pm_model::json::{self, to_canonical_string};
use pm_model::{Document, Map, Node, Op, Value};
use pm_wasm::Engine;

use crate::model::{self, LayerInfo, LayerLoc};

/// Why a query had no answer.
#[derive(Clone, Debug, PartialEq)]
pub enum QueryError {
    /// No layer has this id in the composition.
    LayerNotFound(String),
    /// No such composition.
    CompNotFound(String),
    /// The evaluator threw (`TypeError`-style); the message is the legacy
    /// JavaScript message.
    Thrown { kind: String, message: String },
    /// Text layout met fonts the mirror does not have yet; load them (see
    /// [`QueryContext::engine_mut`]) and ask again.
    NeedsFonts,
    /// Glyph outlines are needed for ink bounds; call
    /// `engine_mut().set_font_outlines(true)`.
    NeedsInk,
    /// 3D models must be loaded first (`Engine::set_model`); ids attached.
    NeedsModels(Vec<String>),
    /// Only today's browser 3D runtime can answer (Phase 1 leftovers).
    NeedsHost,
    /// The engine cannot answer for this content (unloadable model, ...).
    Unsupported(String),
    /// A malformed request or reply.
    Error(String),
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryError::LayerNotFound(id) => write!(f, "No layer {id}"),
            QueryError::CompNotFound(id) => write!(f, "No composition {id}"),
            QueryError::Thrown { message, .. } => f.write_str(message),
            QueryError::NeedsFonts => f.write_str("Text layout needs fonts that are not loaded"),
            QueryError::NeedsInk => f.write_str("Ink bounds need glyph outlines"),
            QueryError::NeedsModels(ids) => write!(f, "3D models not loaded: {}", ids.join(", ")),
            QueryError::NeedsHost => f.write_str("Only the legacy 3D runtime can answer this"),
            QueryError::Unsupported(m) => write!(f, "Unsupported: {m}"),
            QueryError::Error(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for QueryError {}

/// Layer bounds (pm-eval `Bounds`) in the layer's **own plane**, before its
/// transform: map corners with [`QueryContext::world_matrix`] to get
/// composition pixels (the old viewer did the same with `GL.bounds`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub w: f64,
    pub h: f64,
    /// Anchor point, in the same plane.
    pub ax: f64,
    pub ay: f64,
}

/// Options of [`QueryContext::pick`].
#[derive(Clone, Debug, Default)]
pub struct PickOptions {
    /// Hit locked layers too (the old selection tool skipped them).
    pub include_locked: bool,
}

/// A frozen answer to "what is under the pointer".
#[derive(Clone, Debug, PartialEq)]
pub struct PickHit {
    /// Stable id of the hit layer; `None` when nothing was hit.
    pub layer_id: Option<String>,
    /// The revision the answer was computed at (declare it with the overlay
    /// that uses it, section 5.2: never mix revisions).
    pub revision: u64,
}

/// The synchronous query facade. See the module documentation.
pub struct QueryContext {
    engine: Engine,
}

impl QueryContext {
    /// A context at `document`'s revision, sharing its tree.
    pub(crate) fn new(document: &Document) -> QueryContext {
        let mut engine = Engine::new();
        engine.adopt_document(document.clone(), ConsistencyHash::default());
        QueryContext { engine }
    }

    /// Follows a committed transaction. `base` must be the revision this
    /// context is at; on any mismatch the mirror is replaced from `current`
    /// (a resync, never a silent divergence).
    pub(crate) fn apply(&mut self, ops: &[Op], base: u64, revision: u64, current: &Document) {
        let ok = self.engine.revision() == base
            && pm_model::binary::encode(ops)
                .ok()
                .is_some_and(|body| self.engine.apply(&body, base, revision).is_ok());
        if !ok {
            self.resync(current);
        }
    }

    /// Replaces the mirror with `document` (open, replace, resync).
    pub(crate) fn resync(&mut self, document: &Document) {
        self.engine
            .adopt_document(document.clone(), ConsistencyHash::default());
    }

    /// The revision of the mirror (equals the store's current revision).
    pub fn revision(&self) -> u64 {
        self.engine.revision()
    }

    /// The mirror's document (read-only).
    pub fn document(&self) -> &Document {
        self.engine.document()
    }

    /// The engine itself, for preloading fonts and models and for queries not
    /// wrapped here. Do **not** call `apply`/`resync` on it: the store keeps
    /// it at the committed revision.
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    fn root(&self) -> Option<&Node> {
        self.engine.document().root()
    }

    /// The layers panel rows of composition `comp_id` (`None`: the open one),
    /// top of the stack first, at the mirror's revision.
    pub fn layers(&self, comp_id: Option<&str>) -> Vec<LayerInfo> {
        self.root()
            .map(|root| model::list_layers(root, comp_id))
            .unwrap_or_default()
    }

    /// Several channels of one layer at composition time `t` (seconds).
    pub fn values(
        &mut self,
        comp_id: Option<&str>,
        layer_id: &str,
        channels: &[&str],
        t: f64,
    ) -> Vec<Result<Value, QueryError>> {
        channels
            .iter()
            .map(|c| self.evaluate(comp_id, layer_id, c, t))
            .collect()
    }

    fn resolve(
        &self,
        comp_id: Option<&str>,
        layer_id: &str,
    ) -> Result<(String, usize), QueryError> {
        let root = self
            .root()
            .ok_or_else(|| QueryError::Error("no document".into()))?;
        let comp = comp_id
            .map(str::to_owned)
            .or_else(|| model::open_comp_id(root));
        let node = model::comp_node(root, comp.as_deref())
            .ok_or_else(|| QueryError::CompNotFound(comp.clone().unwrap_or_default()))?;
        let index = model::layers_of(node)
            .position(|l| l.get("id").and_then(model::str_of) == Some(layer_id))
            .ok_or_else(|| QueryError::LayerNotFound(layer_id.to_owned()))?;
        Ok((comp.unwrap_or_default(), index))
    }

    fn open_comp(&self) -> String {
        self.root().and_then(model::open_comp_id).unwrap_or_default()
    }

    // ── raw ────────────────────────────────────────────────────────────────

    /// One raw `pm-wasm` query (`{"op": ..., "comp": ..., "t": ...}`).
    /// Returns the `ok` value with special numbers (`NaN`, `±Infinity`, `-0`)
    /// revived, or the typed reason there was no answer.
    pub fn query(&mut self, request: &Value) -> Result<Value, QueryError> {
        let bytes = to_canonical_string(request).into_bytes();
        let reply = self.engine.query(&bytes);
        let parsed = json::parse(&reply.text)
            .map_err(|e| QueryError::Error(format!("unreadable reply: {e}")))?;
        let Value::Object(map) = parsed else {
            return Err(QueryError::Error("unreadable reply".into()));
        };
        if let Some(ok) = map.get("ok") {
            return Ok(revive(ok.clone()));
        }
        if let Some(t) = map.get("throw") {
            return Err(QueryError::Thrown {
                kind: t.get("kind").and_then(Value::as_str).unwrap_or("").into(),
                message: t.get("message").and_then(Value::as_str).unwrap_or("").into(),
            });
        }
        if map.get("fonts").is_some() {
            return Err(QueryError::NeedsFonts);
        }
        if map.get("ink").is_some() {
            return Err(QueryError::NeedsInk);
        }
        if let Some(models) = map.get("models").and_then(Value::as_array) {
            return Err(QueryError::NeedsModels(
                models
                    .iter()
                    .filter_map(|m| m.as_str().map(str::to_owned))
                    .collect(),
            ));
        }
        if map.get("host").is_some() {
            return Err(QueryError::NeedsHost);
        }
        if let Some(u) = map.get("unsupported") {
            return Err(QueryError::Unsupported(u.as_str().unwrap_or("").into()));
        }
        Err(QueryError::Error(
            map.get("error")
                .and_then(Value::as_str)
                .unwrap_or("unreadable reply")
                .to_owned(),
        ))
    }

    fn layer_query(
        &mut self,
        op: &str,
        comp_id: Option<&str>,
        layer_id: &str,
        t: f64,
        extra: &[(&str, Value)],
    ) -> Result<Value, QueryError> {
        let (comp, index) = self.resolve(comp_id, layer_id)?;
        let open = self.open_comp();
        let mut m = Map::new();
        m.insert("op", Value::from(op));
        m.insert("open", Value::from(open.as_str()));
        m.insert("comp", Value::from(comp));
        m.insert("layer", Value::Number(index as f64));
        m.insert("t", Value::Number(t));
        for (k, v) in extra {
            m.insert(*k, v.clone());
        }
        self.query(&Value::Object(m))
    }

    // ── values ─────────────────────────────────────────────────────────────

    /// One channel of the layer's `p` table at composition time `t` seconds
    /// (`PM.ev`). `comp_id` `None` is the open composition.
    pub fn evaluate(
        &mut self,
        comp_id: Option<&str>,
        layer_id: &str,
        channel: &str,
        t: f64,
    ) -> Result<Value, QueryError> {
        self.layer_query("ev", comp_id, layer_id, t, &[("key", Value::from(channel))])
    }

    /// Opacity (percent) multiplied through the parent chain.
    pub fn world_opacity(
        &mut self,
        comp_id: Option<&str>,
        layer_id: &str,
        t: f64,
    ) -> Result<f64, QueryError> {
        number(self.layer_query("worldOpacity", comp_id, layer_id, t, &[])?)
    }

    /// The 2D affine matrix `[a, b, c, d, e, f]` of the layer alone.
    pub fn local_matrix(
        &mut self,
        comp_id: Option<&str>,
        layer_id: &str,
        t: f64,
    ) -> Result<[f64; 6], QueryError> {
        matrix6(self.layer_query("localMatrix", comp_id, layer_id, t, &[])?)
    }

    /// The layer's matrix including its parents.
    pub fn world_matrix(
        &mut self,
        comp_id: Option<&str>,
        layer_id: &str,
        t: f64,
    ) -> Result<[f64; 6], QueryError> {
        matrix6(self.layer_query("worldMatrix", comp_id, layer_id, t, &[])?)
    }

    // ── geometry ───────────────────────────────────────────────────────────

    /// The layer's bounds, or `None` when it has none at `t` (inactive or
    /// empty). May answer *needs fonts / ink / models*.
    pub fn bounds(
        &mut self,
        comp_id: Option<&str>,
        layer_id: &str,
        t: f64,
    ) -> Result<Option<Bounds>, QueryError> {
        bounds_of(self.layer_query("bounds", comp_id, layer_id, t, &[])?)
    }

    /// Bounds of a group layer including its members.
    pub fn group_bounds(
        &mut self,
        comp_id: Option<&str>,
        layer_id: &str,
        t: f64,
    ) -> Result<Option<Bounds>, QueryError> {
        bounds_of(self.layer_query("groupBounds", comp_id, layer_id, t, &[])?)
    }

    /// Hit test at `(x, y)` composition pixels, time `t` seconds.
    ///
    /// The engine answers with a *layer index*; this binds it back to the
    /// stable id **inside the same call**, at the same revision, so a later
    /// reorder cannot make the answer point at the wrong layer.
    pub fn pick(
        &mut self,
        comp_id: Option<&str>,
        x: f64,
        y: f64,
        t: f64,
        options: &PickOptions,
    ) -> Result<PickHit, QueryError> {
        let root = self
            .root()
            .ok_or_else(|| QueryError::Error("no document".into()))?;
        let comp = comp_id
            .map(str::to_owned)
            .or_else(|| model::open_comp_id(root))
            .unwrap_or_default();
        let open = self.open_comp();
        let mut m = Map::new();
        m.insert("op", Value::from("pick"));
        m.insert("open", Value::from(open.as_str()));
        m.insert("comp", Value::from(comp.as_str()));
        m.insert("t", Value::Number(t));
        m.insert("x", Value::Number(x));
        m.insert("y", Value::Number(y));
        m.insert("includeLocked", Value::Bool(options.include_locked));
        let reply = self.query(&Value::Object(m))?;
        let revision = self.revision();
        let index = reply
            .as_array()
            .and_then(|a| a.first())
            .and_then(Value::as_f64)
            .filter(|i| *i >= 0.0)
            .map(|i| i as usize);
        let layer_id = index.and_then(|i| {
            let root = self.root()?;
            let comp_ref = (!comp.is_empty()).then_some(comp.as_str());
            let loc = LayerLoc {
                comp_id: comp_ref.map(str::to_owned),
                index: i,
            };
            model::layer_at(root, &loc)?
                .get("id")
                .and_then(model::str_of)
                .map(str::to_owned)
        });
        Ok(PickHit { layer_id, revision })
    }
}

/// Revives the marker objects `{"\u0000": "NaN" | "Infinity" | "-Infinity" |
/// "-0" | "undefined"}` that JSON cannot spell.
fn revive(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            if map.len() == 1 {
                if let Some(Value::String(s)) = map.get("\u{0}") {
                    return match s.as_str() {
                        Some("NaN") => Value::Number(f64::NAN),
                        Some("Infinity") => Value::Number(f64::INFINITY),
                        Some("-Infinity") => Value::Number(f64::NEG_INFINITY),
                        Some("-0") => Value::Number(-0.0),
                        _ => Value::Null,
                    };
                }
            }
            Value::Object(map.into_iter().map(|(k, v)| (k, revive(v))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(revive).collect()),
        other => other,
    }
}

fn number(v: Value) -> Result<f64, QueryError> {
    v.as_f64()
        .ok_or_else(|| QueryError::Error("expected a number".into()))
}

fn matrix6(v: Value) -> Result<[f64; 6], QueryError> {
    let items = v
        .as_array()
        .filter(|a| a.len() == 6)
        .ok_or_else(|| QueryError::Error("expected a 6-number matrix".into()))?;
    let mut out = [0.0; 6];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = item
            .as_f64()
            .ok_or_else(|| QueryError::Error("expected a 6-number matrix".into()))?;
    }
    Ok(out)
}

fn bounds_of(v: Value) -> Result<Option<Bounds>, QueryError> {
    if v.is_null() {
        return Ok(None);
    }
    let get = |k: &str| {
        v.get(k)
            .and_then(Value::as_f64)
            .ok_or_else(|| QueryError::Error(format!("bounds reply lacks {k}")))
    };
    Ok(Some(Bounds {
        x0: get("x0")?,
        y0: get("y0")?,
        x1: get("x1")?,
        y1: get("y1")?,
        w: get("w")?,
        h: get("h")?,
        ax: get("ax")?,
        ay: get("ay")?,
    }))
}
