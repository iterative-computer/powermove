//! A composition inside a document, resolved for evaluation.
//!
//! The legacy evaluator reads the composition being evaluated from the global
//! scope (`PM.curComp()`) and the project frame rate from `PM.proj.fps`. Here
//! both are explicit: a [`Comp`] names one composition of one document
//! revision.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT_BINDING_TOKEN: AtomicU64 = AtomicU64::new(1);

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
///
/// # The legacy composition object
///
/// The legacy code reads the open composition as `PM.proj`, the stored
/// project, whose top level mixes the composition's fields (`w`, `h`, `fps`,
/// `dur`, `layers`, ...) with the project's (`id`, `name`, `params`,
/// `assets`, ...). In the canonical form the open composition lives in
/// `comps[compId]` with only its own fields. [`Comp::member`] answers member
/// reads the way the legacy object would: composition fields from the
/// composition, everything else from the project root. Nested compositions
/// are plain objects in both forms. The one difference: the legacy `comps`
/// member omits the open composition, the canonical one includes it.
#[derive(Clone, Debug)]
pub struct Comp<'d> {
    root: &'d Node,
    id: JsString,
    node: &'d Node,
    revision: u64,
    rate: std::cell::OnceCell<Option<IngestRate>>,
    expression_fps: std::cell::OnceCell<JsValue>,
    layers_node: Option<&'d Node>,
    binding_token: u64,
    /// The open composition of a canonical document (see above).
    open_canonical: bool,
    /// The open composition seen through `rootComp(PM.proj)` (see
    /// [`ROOT_VIEW`]) in a document still in the stored form.
    stored_root_view: bool,
}

/// The address prefix of the legacy `rootComp(PM.proj)` wrapper
/// (`legacy/core/compositions.ts`), which `PM.Comps.get(openId)` returns:
/// `{ id: compId, name: compName }` plus each composition field the project
/// has, and nothing else (no `params`, `comps`, `compId`, project `id` or
/// `name`). Expressions see that object as `comp`, and `param()` reads its
/// (absent) `params`, so a scope holding the wrapper answers differently
/// from the open composition itself. `@root:<openId>` resolves to the open
/// composition with exactly the wrapper's members; its own address keeps
/// every cache keyed by composition id apart from the open composition's.
pub const ROOT_VIEW: &str = "@root:";

impl<'d> Comp<'d> {
    /// Resolves `comp_id` in a document in canonical form
    /// (`pm_model::canonical`): `comps[comp_id]`. A document still in the
    /// stored form keeps its open composition at the top level; that one is
    /// found by its `compId` (or its `id` when it has no `compId`) and wins
    /// over a stale copy in `comps`.
    pub fn resolve(doc: &'d Document, comp_id: &str) -> Result<Comp<'d>, CompError> {
        let root = doc.root().ok_or(CompError::NoDocument)?;
        Comp::in_root(root, comp_id, doc.revision())
    }

    /// [`Comp::resolve`] on a bare root node at a given revision.
    pub fn in_root(root: &'d Node, comp_id: &str, revision: u64) -> Result<Comp<'d>, CompError> {
        if let Some(open) = comp_id.strip_prefix(ROOT_VIEW) {
            let mut comp = Comp::in_root(root, open, revision)?;
            // The canonical open composition is already exactly the wrapper:
            // `{ id: compId, name: compName, ...composition fields }`
            // (`pm_model::canonical`), so it reads as a plain object. The
            // stored form keeps it at the top level, among project members.
            comp.stored_root_view = comp.at_root();
            comp.open_canonical = false;
            comp.id = JsString::from(comp_id);
            return Ok(comp);
        }
        // The stored form keeps the open composition at the top level (and may
        // keep a stale copy in `comps`, which the legacy code never reads).
        let open = node::string(node::member(Some(root), "compId"))
            .or_else(|| node::string(node::member(Some(root), "id")));
        let stored_open = open.is_some_and(|s| s.as_bytes() == comp_id.as_bytes())
            && node::member(Some(root), "layers").is_some();
        // A nested composition is addressed by its canonical member path, so
        // duplicate ids in different composition maps remain distinct.
        let path_node = comp_id.strip_prefix("@path:").and_then(|json| {
            let path: Vec<String> = serde_json::from_str(json).ok()?;
            path.iter()
                .try_fold(root, |at, key| node::member(Some(at), key))
        });
        let node = if let Some(node) = path_node {
            node
        } else if stored_open {
            root
        } else {
            node::member(node::member(Some(root), "comps"), comp_id)
                .filter(|c| matches!(c.kind(), NodeKind::Object(_)))
                .ok_or_else(|| CompError::NotFound(comp_id.to_owned()))?
        };
        let open_canonical = !std::ptr::eq(node, root)
            && node::string(node::member(Some(root), "compId"))
                .is_some_and(|open| open.as_bytes() == comp_id.as_bytes());
        Ok(Comp {
            root,
            id: JsString::from(comp_id),
            node,
            revision,
            rate: std::cell::OnceCell::new(),
            expression_fps: std::cell::OnceCell::new(),
            layers_node: node::member(Some(node), "layers"),
            binding_token: NEXT_BINDING_TOKEN.fetch_add(1, Ordering::Relaxed),
            open_canonical,
            stored_root_view: false,
        })
    }

    /// An ad-hoc composition during project replacement. It keeps the open
    /// project's members/rate while evaluating the caller's exact scope.
    pub fn at_node(root: &'d Node, node: &'d Node, id: &str, revision: u64) -> Self {
        Self {
            root,
            node,
            id: JsString::from(id),
            revision,
            rate: std::cell::OnceCell::new(),
            expression_fps: std::cell::OnceCell::new(),
            layers_node: node::member(Some(node), "layers"),
            binding_token: NEXT_BINDING_TOKEN.fetch_add(1, Ordering::Relaxed),
            open_canonical: false,
            stored_root_view: false,
        }
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

    /// Whether the composition object is the document root (the stored
    /// form's open composition) rather than `comps[id]`. The same id can
    /// resolve to either over a document's history (opening another
    /// composition moves the stored form's open one back to its `comps`
    /// copy), so caches keyed by composition id also key on this.
    pub fn at_root(&self) -> bool {
        std::ptr::eq(self.node, self.root)
    }

    /// Identity of this immutable resolved view. Binding validation can share
    /// work inside one batch without retaining document-pointer assumptions.
    pub fn binding_token(&self) -> u64 {
        self.binding_token
    }

    /// The document revision this composition was resolved at.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The composition's exact rate (section 5), when its `fps` and
    /// `fpsRational` give a valid one.
    pub fn rate(&self) -> Option<IngestRate> {
        *self
            .rate
            .get_or_init(|| IngestRate::of_composition(&self.node.to_value_shallow_rate()).ok())
    }

    /// `comp.layers`, or no layers when it is not an array.
    pub fn layers(&self) -> &'d [Node] {
        node::array(self.layers_node()).unwrap_or(&[])
    }

    /// The `layers` array node.
    pub fn layers_node(&self) -> Option<&'d Node> {
        self.layers_node
    }

    /// A member of the legacy composition object (see the type
    /// documentation).
    pub fn member(&self, key: &str) -> Option<&'d Node> {
        if self.stored_root_view {
            return match key {
                "id" => node::member(Some(self.root), "compId"),
                "name" => node::member(Some(self.root), "compName"),
                _ if pm_model::canonical::COMP_FIELDS.contains(&key) => {
                    node::member(Some(self.node), key)
                }
                _ => None,
            };
        }
        if !self.open_canonical {
            return node::member(Some(self.node), key);
        }
        if pm_model::canonical::COMP_FIELDS.contains(&key) {
            return node::member(Some(self.node), key);
        }
        match key {
            "compName" => node::member(Some(self.node), "name"),
            _ => node::member(Some(self.root), key),
        }
    }

    /// `comp[key]` on the legacy composition object for any key.
    pub fn get(&self, key: &[u8]) -> Got<'d> {
        if self.stored_root_view {
            return match std::str::from_utf8(key) {
                Ok(k) => self.member(k).map_or(Got::Undefined, Got::Node),
                Err(_) => Got::Undefined,
            };
        }
        if !self.open_canonical {
            return node::get(self.node, key);
        }
        match std::str::from_utf8(key) {
            Ok(k) => self.member(k).map_or(Got::Undefined, Got::Node),
            Err(_) => node::get(self.root, key),
        }
    }

    /// Whether this is the open composition of a canonical document, whose
    /// legacy object merges project fields (see the type documentation).
    pub fn is_open_canonical(&self) -> bool {
        self.open_canonical
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
        self.expression_fps
            .get_or_init(|| {
                let fps = self.member("fps");
                if node::truthy(fps) {
                    node::js_value_opt(fps)
                } else {
                    self.project_fps()
                }
            })
            .clone()
    }

    /// `Number(comp.dur)`.
    pub fn duration(&self) -> f64 {
        node::to_number(self.member("dur"))
    }

    /// Seconds for a tick on this composition's grid: `n / fps` (the
    /// document's own float) for a frame start, `ticks / TICKS` otherwise
    /// (`IngestRate::seconds_of`). Without a valid rate, `ticks / TICKS`.
    pub fn seconds_at(&self, ticks: Tick) -> f64 {
        match self.rate() {
            Some(rate) => rate.seconds_of(ticks),
            None => pm_time::seconds_from_ticks(ticks),
        }
    }

    /// The composition frame containing a time in seconds (section 5, rule 9).
    pub fn frame_at(&self, seconds: f64) -> Option<i64> {
        let rate = self.rate()?;
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
                IdKey::Number(if n.is_nan() {
                    f64::NAN.to_bits()
                } else {
                    n.to_bits()
                })
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
    /// String ids: the first layer and whether the id is duplicated.
    pub strings: HashMap<JsString, (usize, bool)>,
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
                    if let IdKey::String(s) = &key {
                        index
                            .strings
                            .entry(s.clone())
                            .and_modify(|e| e.1 = true)
                            .or_insert((i, false));
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::json::parse;

    fn root(text: &str) -> Node {
        Node::from_value(parse(text).unwrap(), 1)
    }

    #[test]
    fn resolves_both_forms() {
        let stored = root(
            r#"{"id":"P","compId":"C","fps":30,"dur":5,"layers":[],"params":{"a":1},
                "comps":{"C":{"id":"C","fps":24,"layers":[1]},"N":{"id":"N","fps":25,"layers":[]}}}"#,
        );
        let open = Comp::in_root(&stored, "C", 1).unwrap();
        assert!(
            std::ptr::eq(open.node(), &stored),
            "the top level wins over a stale copy"
        );
        assert_eq!(open.expression_fps(), JsValue::Number(30.0));
        assert!(!open.is_open_canonical());
        let nested = Comp::in_root(&stored, "N", 1).unwrap();
        assert_eq!(nested.expression_fps(), JsValue::Number(25.0));
        assert_eq!(
            Comp::in_root(&stored, "X", 1).unwrap_err(),
            CompError::NotFound("X".into())
        );

        let canonical = root(
            r#"{"id":"P","name":"Project","compId":"C","params":{"a":1},
                "comps":{"C":{"id":"C","name":"Main","fps":30,"dur":5,"layers":[]}}}"#,
        );
        let open = Comp::in_root(&canonical, "C", 1).unwrap();
        assert!(open.is_open_canonical());
        assert_eq!(
            node::js_value_opt(open.member("params")),
            JsValue::from(parse(r#"{"a":1}"#).unwrap())
        );
        assert_eq!(
            node::js_value_opt(open.member("name")),
            JsValue::from("Project")
        );
        assert_eq!(
            node::js_value_opt(open.member("compName")),
            JsValue::from("Main")
        );
        assert_eq!(open.duration(), 5.0);
        assert_eq!(open.project_fps(), JsValue::Number(30.0));
    }

    /// `@root:<open>` reads exactly `rootComp(PM.proj)`'s members in both
    /// forms: the open composition's fields, `id: compId`, `name: compName`,
    /// and no project members.
    #[test]
    fn root_wrapper_view() {
        let canonical = root(
            r#"{"id":"P","name":"Project","compId":"C","params":{"amp":{"value":7}},
                "comps":{"C":{"id":"C","name":"Main","fps":30,"dur":5,"layers":[]}}}"#,
        );
        let open = Comp::in_root(&canonical, "C", 1).unwrap();
        let view = Comp::in_root(&canonical, "@root:C", 1).unwrap();
        assert!(std::ptr::eq(open.node(), view.node()));
        assert_eq!(view.id().to_string_lossy(), "@root:C");
        assert!(open.get(b"params").truthy());
        for (key, want) in [
            ("params", JsValue::Undefined),
            ("compId", JsValue::Undefined),
            ("compName", JsValue::Undefined),
            ("comps", JsValue::Undefined),
            ("id", JsValue::from("C")),
            ("name", JsValue::from("Main")),
            ("fps", JsValue::Number(30.0)),
        ] {
            assert_eq!(node::js_value_opt(view.member(key)), want, "{key}");
        }
        assert_eq!(view.expression_fps(), JsValue::Number(30.0));

        // The stored form (no `compId`: the project is its own open
        // composition, addressed by its `id`).
        let stored = root(
            r#"{"id":"P","name":"Project","compName":"Main","fps":24,"dur":3,"layers":[],
                "params":{"amp":{"value":7}}}"#,
        );
        let view = Comp::in_root(&stored, "@root:P", 1).unwrap();
        assert!(view.at_root());
        for (key, want) in [
            ("params", JsValue::Undefined),
            ("compName", JsValue::Undefined),
            ("id", JsValue::Undefined),
            ("name", JsValue::from("Main")),
            ("fps", JsValue::Number(24.0)),
            ("dur", JsValue::Number(3.0)),
        ] {
            assert_eq!(node::js_value_opt(view.member(key)), want, "{key}");
            assert_eq!(view.get(key.as_bytes()).truthy(), want.truthy(), "{key}");
        }
        assert!(Comp::in_root(&stored, "@root:X", 1).is_err());
    }

    #[test]
    fn group_ancestry() {
        let layers = root(
            r#"[{"id":"g1","type":"group","group":"g2"},{"id":"g2","type":"group","group":"g1"},
                {"id":"x","group":"g1"},{"id":"g1","type":"solid"}]"#,
        );
        let NodeKind::Array(items) = layers.kind() else {
            unreachable!()
        };
        let index = LayerIndex::build(items);
        // The last layer with id g1 is not a group: the chain stops at once.
        assert!(
            group_ancestors(&items[2], items, &index)
                .unwrap()
                .is_empty()
        );
        assert!(index.duplicated.contains(&IdKey::String("g1".into())));
        assert_eq!(index.strings.get(&JsString::from("g1")), Some(&(0, true)));
    }
}
