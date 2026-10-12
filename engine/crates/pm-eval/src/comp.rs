//! A composition inside a document, resolved for evaluation.
//!
//! The legacy evaluator reads the composition being evaluated from the global
//! scope (`PM.curComp()`) and the project frame rate from `PM.proj.fps`. Here
//! both are explicit: a [`Comp`] names one composition of one document
//! revision.

use std::collections::HashMap;

use pm_model::ingest::IngestRate;
use pm_model::{Document, JsString, Node, NodeKind};
use pm_time::Tick;

use crate::js_sem::JsValue;
use crate::node::{self, Got};

/// Why a composition could not be resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompError {
    /// The document is `undefined`.
    NoDocument,
    /// No composition with this id.
    NotFound(String),
}

impl core::fmt::Display for CompError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CompError::NoDocument => write!(f, "the document is empty"),
            CompError::NotFound(id) => write!(f, "no composition {id:?}"),
        }
    }
}

impl std::error::Error for CompError {}

/// One composition of a document revision.
#[derive(Clone, Debug)]
pub struct Comp<'d> {
    root: &'d Node,
    id: JsString,
    node: &'d Node,
    revision: u64,
    rate: Option<IngestRate>,
}

impl<'d> Comp<'d> {
    /// Resolves `comp_id` in a document in canonical form
    /// (`pm_model::canonical`): `comps[comp_id]`. A document still in the
    /// stored form keeps its open composition at the top level; that one is
    /// found by its `compId` (or its `id` when it has no `compId`).
    pub fn resolve(doc: &'d Document, comp_id: &str) -> Result<Comp<'d>, CompError> {
        let root = doc.root().ok_or(CompError::NoDocument)?;
        Comp::in_root(root, comp_id, doc.revision())
    }

    /// [`Comp::resolve`] on a bare root node at a given revision.
    pub fn in_root(root: &'d Node, comp_id: &str, revision: u64) -> Result<Comp<'d>, CompError> {
        let in_comps = node::member(node::member(Some(root), "comps"), comp_id)
            .filter(|c| matches!(c.kind(), NodeKind::Object(_)));
        let node = match in_comps {
            Some(c) => c,
            None => {
                let open = node::string(node::member(Some(root), "compId"))
                    .or_else(|| node::string(node::member(Some(root), "id")));
                if open.is_some_and(|s| s.as_bytes() == comp_id.as_bytes())
                    && node::member(Some(root), "layers").is_some()
                {
                    root
                } else {
                    return Err(CompError::NotFound(comp_id.to_owned()));
                }
            }
        };
        let rate = IngestRate::of_composition(&node.to_value_shallow_rate()).ok();
        Ok(Comp {
            root,
            id: JsString::from(comp_id),
            node,
            revision,
            rate,
        })
    }

    pub fn id(&self) -> &JsString {
        &self.id
    }

    /// The composition object (`PM.curComp()`).
    pub fn node(&self) -> &'d Node {
        self.node
    }

    /// The document root.
    pub fn root(&self) -> &'d Node {
        self.root
    }

    /// The document revision this composition was resolved at.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The composition's exact rate (section 5), when its `fps` and
    /// `fpsRational` give a valid one.
    pub fn rate(&self) -> Option<IngestRate> {
        self.rate
    }

    /// `comp.layers`, or no layers when it is not an array.
    pub fn layers(&self) -> &'d [Node] {
        node::array(node::member(Some(self.node), "layers")).unwrap_or(&[])
    }

    /// The `layers` array node.
    pub fn layers_node(&self) -> Option<&'d Node> {
        node::member(Some(self.node), "layers")
    }

    /// The open composition's `fps` (`PM.proj.fps`): the stored form keeps it
    /// at the top level, the canonical form in `comps[compId]`.
    pub fn project_fps(&self) -> JsValue {
        let root = Some(self.root);
        if let Some(fps) = node::member(root, "fps") {
            return node::js_value(fps);
        }
        let open = node::string(node::member(root, "compId"));
        let comp = open.and_then(|id| node::member(node::member(root, "comps"), id.as_str()?));
        node::js_value_opt(node::member(comp, "fps"))
    }

    /// `comp.fps || PM.proj.fps`, the `fps` expressions see.
    pub fn expression_fps(&self) -> JsValue {
        let fps = node::member(Some(self.node), "fps");
        if node::truthy(fps) {
            node::js_value_opt(fps)
        } else {
            self.project_fps()
        }
    }

    /// `Number(comp.dur)`.
    pub fn duration(&self) -> f64 {
        node::to_number(node::member(Some(self.node), "dur"))
    }

    /// Seconds for a tick on this composition's grid: `n / fps` (the
    /// document's own float) for a frame start, `ticks / TICKS` otherwise
    /// (`IngestRate::seconds_of`). Without a valid rate, `ticks / TICKS`.
    pub fn seconds_at(&self, ticks: Tick) -> f64 {
        match self.rate {
            Some(rate) => rate.seconds_of(ticks),
            None => pm_time::seconds_from_ticks(ticks),
        }
    }

    /// The composition frame containing a time in seconds (section 5, rule 9).
    pub fn frame_at(&self, seconds: f64) -> Option<i64> {
        let rate = self.rate?;
        let t = rate.time(seconds).ok()?;
        Some(t.frame.unwrap_or_else(|| rate.rate.frame_of(t.ticks)))
    }

    /// `indexOfLayer(comp, layer)`: the layer's position in `comp.layers`, by
    /// identity, or 0.
    pub fn index_of_layer(&self, layer: Option<&Node>) -> f64 {
        let Some(layer) = layer else { return 0.0 };
        self.layers()
            .iter()
            .position(|l| std::ptr::eq(l, layer))
            .map_or(0.0, |i| i as f64)
    }
}

trait RateSource {
    fn to_value_shallow_rate(&self) -> pm_model::Value;
}

impl RateSource for Node {
    /// Only the members `IngestRate::of_composition` reads.
    fn to_value_shallow_rate(&self) -> pm_model::Value {
        let mut map = pm_model::Map::new();
        for key in ["fps", "fpsRational"] {
            if let Some(v) = self.get(key) {
                map.insert(key, v.to_value());
            }
        }
        pm_model::Value::Object(map)
    }
}

/// A layer id as a `Map` key (SameValueZero): strings, numbers, booleans.
/// Objects are never equal to another value's identity here, so they are
/// not indexed.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum IdKey {
    String(JsString),
    Number(u64),
    Bool(bool),
    Null,
}

impl IdKey {
    /// The key of a value, or `None` for `undefined` and objects.
    pub fn of(value: &Got<'_>) -> Option<IdKey> {
        IdKey::of_js(&value.to_js())
    }

    pub fn of_js(value: &JsValue) -> Option<IdKey> {
        Some(match value {
            JsValue::String(s) => IdKey::String(s.clone()),
            JsValue::Number(n) => {
                let n = if *n == 0.0 { 0.0 } else { *n };
                IdKey::Number(if n.is_nan() { f64::NAN.to_bits() } else { n.to_bits() })
            }
            JsValue::Bool(b) => IdKey::Bool(*b),
            JsValue::Null => IdKey::Null,
            JsValue::Undefined | JsValue::Object(_) => return None,
        })
    }
}

/// Layer lookups for one composition revision.
#[derive(Clone, Debug, Default)]
pub struct LayerIndex {
    /// First layer per id (`parentOf`, interned properties).
    pub first: HashMap<IdKey, usize>,
    /// Last layer per id (`new Map(layers.map(l => [l.id, l]))`,
    /// `groupAncestors`).
    pub last: HashMap<IdKey, usize>,
    /// Ids held by more than one layer.
    pub duplicated: std::collections::HashSet<IdKey>,
    /// Layers whose id is `undefined` or an object, by position.
    pub unkeyed: Vec<usize>,
    /// Whether some element of `layers` is `null` (building the legacy
    /// group index throws then).
    pub has_nullish: bool,
}

impl LayerIndex {
    pub fn build(layers: &[Node]) -> LayerIndex {
        let mut index = LayerIndex::default();
        for (i, layer) in layers.iter().enumerate() {
            let id = if node::is_nullish(Some(layer)) {
                index.has_nullish = true;
                None
            } else {
                IdKey::of(&node::get(layer, b"id"))
            };
            match id {
                Some(key) => {
                    if index.first.contains_key(&key) {
                        index.duplicated.insert(key.clone());
                    } else {
                        index.first.insert(key.clone(), i);
                    }
                    index.last.insert(key, i);
                }
                None => index.unkeyed.push(i),
            }
        }
        index
    }
}

/// `groupAncestors(layer, layers, byId)` with the last-wins index: the chain
/// of enclosing groups, innermost first. Stops at a missing or non-group
/// layer and at a cycle.
///
/// Throws when the layer has a group and some element of `layers` is `null`
/// (the legacy index reads every layer's `id`).
pub fn group_ancestors<'d>(
    layer: &'d Node,
    layers: &'d [Node],
    index: &LayerIndex,
) -> crate::js_sem::JsResult<Vec<&'d Node>> {
    let mut result = Vec::new();
    let mut id = node::get(layer, b"group");
    if !id.truthy() {
        return Ok(result);
    }
    if index.has_nullish {
        return Err(crate::js_sem::JsThrow::type_error(
            "Cannot read properties of null (reading 'id')",
        ));
    }
    let mut seen: Vec<Option<IdKey>> = vec![IdKey::of(&node::get(layer, b"id"))];
    while id.truthy() {
        let key = IdKey::of(&id);
        if key.is_some() && seen.contains(&key) {
            break;
        }
        seen.push(key.clone());
        let Some(group) = key.and_then(|k| index.last.get(&k)).map(|&i| &layers[i]) else {
            break;
        };
        if !node::is_str(node::member(Some(group), "type"), "group") {
            break;
        }
        result.push(group);
        id = node::get(group, b"group");
    }
    Ok(result)
}
