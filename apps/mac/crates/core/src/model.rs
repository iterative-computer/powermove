//! Read-only helpers over the canonical project tree (`pm_model::Node`).
//!
//! Everything here addresses layers by **id**, never by index; an index is
//! resolved only at the moment a command builds its operations, inside the
//! transaction's scratch document (docs/gpui/engine-boundary.md, section 5.2:
//! "reorder invalidates indices").
//!
//! The canonical form (see `pm_model::canonical`) keeps the open composition
//! in `comps/<compId>`; a project with no `compId` keeps its layers at the
//! top level and is handled as a single anonymous composition.

use pm_model::patch::PathPart;
use pm_model::{JsString, Node, NodeKind};

/// Where a layer lives: which composition, and its index there *right now*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerLoc {
    /// `None` when the layers are at the project root (no `compId`).
    pub comp_id: Option<String>,
    /// Index in the composition's `layers` array at the document revision the
    /// location was resolved against.
    pub index: usize,
}

impl LayerLoc {
    /// The patch path of the layer node.
    pub fn path(&self) -> Vec<PathPart> {
        let mut path = comp_path(self.comp_id.as_deref());
        path.push("layers".into());
        path.push(self.index.into());
        path
    }
}

/// The patch path of a composition (`[]` for the anonymous root one).
pub fn comp_path(comp_id: Option<&str>) -> Vec<PathPart> {
    match comp_id {
        Some(id) => vec!["comps".into(), id.into()],
        None => Vec::new(),
    }
}

pub(crate) fn string_of(node: &Node) -> Option<&JsString> {
    match node.kind() {
        NodeKind::String(s) => Some(s),
        _ => None,
    }
}

pub(crate) fn str_of(node: &Node) -> Option<&str> {
    string_of(node).and_then(JsString::as_str)
}

pub(crate) fn bool_of(node: &Node) -> Option<bool> {
    match node.kind() {
        NodeKind::Bool(b) => Some(*b),
        _ => None,
    }
}

pub(crate) fn number_of(node: &Node) -> Option<f64> {
    match node.kind() {
        NodeKind::Number(n) => Some(*n),
        _ => None,
    }
}

/// The open composition id of a canonical project (a non-empty string
/// `compId`), or `None` for a project whose layers are at the root.
pub fn open_comp_id(root: &Node) -> Option<String> {
    root.get("compId")
        .and_then(str_of)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// The composition node: `comps/<id>`, or the root itself when `comp_id` is
/// `None`.
pub fn comp_node<'a>(root: &'a Node, comp_id: Option<&str>) -> Option<&'a Node> {
    match comp_id {
        Some(id) => root.get("comps")?.get(id),
        None => Some(root),
    }
}

/// The layer nodes of a composition, in stored (top-to-bottom) order.
pub fn layers_of<'a>(comp: &'a Node) -> impl Iterator<Item = &'a Node> + 'a {
    let items: &[Node] = match comp.get("layers").map(Node::kind) {
        Some(NodeKind::Array(items)) => items.as_slice(),
        _ => &[],
    };
    items.iter()
}

/// Ids of every composition of the project: the open one first.
fn comp_ids(root: &Node) -> Vec<Option<String>> {
    let open = open_comp_id(root);
    let mut out = vec![open.clone()];
    if open.is_some() {
        if let Some(NodeKind::Object(comps)) = root.get("comps").map(Node::kind) {
            for (key, _) in comps.iter() {
                if let Some(k) = key.as_str() {
                    if Some(k) != open.as_deref() {
                        out.push(Some(k.to_owned()));
                    }
                }
            }
        }
    }
    out
}

/// Finds a layer by id: the open composition first, then the others (a
/// precomp's layers are edited without opening it).
pub fn find_layer(root: &Node, layer_id: &str) -> Option<LayerLoc> {
    for comp_id in comp_ids(root) {
        let Some(comp) = comp_node(root, comp_id.as_deref()) else {
            continue;
        };
        for (index, layer) in layers_of(comp).enumerate() {
            if layer.get("id").and_then(str_of) == Some(layer_id) {
                return Some(LayerLoc { comp_id, index });
            }
        }
    }
    None
}

/// The layer node at `loc`.
pub fn layer_at<'a>(root: &'a Node, loc: &LayerLoc) -> Option<&'a Node> {
    comp_node(root, loc.comp_id.as_deref())?
        .get("layers")?
        .at(loc.index)
}

/// Whether the layer, or a group above it, is locked (`layer.lock ||
/// groupAncestors(layer).some(g => g.lock)` in the old app). Group membership
/// is the layer's `group` field naming a layer of `type: "group"` in the same
/// composition; a cycle or a dangling id ends the walk, as in
/// `legacy/core/layer-groups.ts`.
pub fn is_locked(root: &Node, loc: &LayerLoc) -> bool {
    let Some(comp) = comp_node(root, loc.comp_id.as_deref()) else {
        return false;
    };
    let Some(layer) = layer_at(root, loc) else {
        return false;
    };
    if layer.get("lock").and_then(bool_of) == Some(true) {
        return true;
    }
    group_ancestors(comp, layer)
        .iter()
        .any(|g| g.get("lock").and_then(bool_of) == Some(true))
}

/// `groupAncestors(layer, layers)`: the chain of containing group layers.
pub fn group_ancestors<'a>(comp: &'a Node, layer: &'a Node) -> Vec<&'a Node> {
    let mut out = Vec::new();
    let mut seen: Vec<&str> = layer.get("id").and_then(str_of).into_iter().collect();
    let mut next = layer.get("group").and_then(str_of).filter(|s| !s.is_empty());
    while let Some(id) = next {
        if seen.contains(&id) {
            break;
        }
        seen.push(id);
        let Some(group) = layers_of(comp).find(|l| {
            l.get("id").and_then(str_of) == Some(id) && l.get("type").and_then(str_of) == Some("group")
        }) else {
            break;
        };
        out.push(group);
        next = group.get("group").and_then(str_of).filter(|s| !s.is_empty());
    }
    out
}

/// Every keyframe id (`i`) under the layers of a composition: any object
/// with a `kf` array counts as a channel (layer, effect, mask and transition
/// channels alike).
pub fn keyframe_ids(comp: &Node, out: &mut std::collections::HashSet<String>) {
    fn walk(node: &Node, out: &mut std::collections::HashSet<String>) {
        match node.kind() {
            NodeKind::Object(map) => {
                if let Some(NodeKind::Array(keys)) = map.get("kf".as_bytes()).map(Node::kind) {
                    for key in keys.iter() {
                        if let Some(id) = key.get("i").and_then(str_of) {
                            out.insert(id.to_owned());
                        }
                    }
                }
                for (_, child) in map.iter() {
                    walk(child, out);
                }
            }
            NodeKind::Array(items) => items.iter().for_each(|c| walk(c, out)),
            _ => {}
        }
    }
    for layer in layers_of(comp) {
        walk(layer, out);
    }
}

/// One row of the layers panel: what a list needs, nothing evaluated.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerInfo {
    pub id: String,
    pub name: String,
    /// The layer `type` (`solid`, `text`, `shape`, `image`, `video`, `audio`,
    /// `group`, `precomp`, ...). Unknown types pass through unchanged.
    pub kind: String,
    /// `parent` (transform parenting), by layer id.
    pub parent: Option<String>,
    /// `group` (containing group layer), by layer id.
    pub group: Option<String>,
    /// Start on the composition timeline, seconds (`from`).
    pub in_time: f64,
    /// End on the composition timeline, seconds (`from + dur`).
    pub out_time: f64,
    /// The layer's own `lock` flag.
    pub locked: bool,
    /// Locked, or inside a locked group (what edits are refused for).
    pub effectively_locked: bool,
    /// The `on` switch (its static value when it is animated).
    pub visible: bool,
    pub solo: bool,
    pub shy: bool,
    /// Index in the composition's `layers` array (0 is the top of the stack).
    pub index: usize,
}

/// The layers of the composition `comp_id` (`None`: the open one).
pub fn list_layers(root: &Node, comp_id: Option<&str>) -> Vec<LayerInfo> {
    let open = open_comp_id(root);
    let comp_id = comp_id.map(str::to_owned).or(open);
    let Some(comp) = comp_node(root, comp_id.as_deref()) else {
        return Vec::new();
    };
    layers_of(comp)
        .enumerate()
        .filter_map(|(index, layer)| {
            let id = layer.get("id").and_then(str_of)?.to_owned();
            let loc = LayerLoc {
                comp_id: comp_id.clone(),
                index,
            };
            let from = layer.get("from").and_then(number_of).unwrap_or(0.0);
            let dur = layer.get("dur").and_then(number_of).unwrap_or(0.0);
            let on = layer.get("on");
            let visible = match on.map(Node::kind) {
                Some(NodeKind::Bool(b)) => *b,
                Some(NodeKind::Object(_)) => on
                    .and_then(|n| n.get("v"))
                    .and_then(bool_of)
                    .unwrap_or(true),
                _ => true,
            };
            Some(LayerInfo {
                name: layer
                    .get("name")
                    .and_then(string_of)
                    .map(JsString::to_string_lossy)
                    .unwrap_or_default(),
                kind: layer
                    .get("type")
                    .and_then(str_of)
                    .unwrap_or_default()
                    .to_owned(),
                parent: layer.get("parent").and_then(str_of).map(str::to_owned),
                group: layer.get("group").and_then(str_of).map(str::to_owned),
                in_time: from,
                out_time: from + dur,
                locked: layer.get("lock").and_then(bool_of) == Some(true),
                effectively_locked: is_locked(root, &loc),
                visible,
                solo: layer.get("solo").and_then(bool_of) == Some(true),
                shy: layer.get("shy").and_then(bool_of) == Some(true),
                index,
                id,
            })
        })
        .collect()
}

/// The layer ids of one composition (`None`: the open one).
pub fn layer_ids(root: &Node, comp_id: Option<&str>) -> std::collections::HashSet<String> {
    let open = open_comp_id(root);
    let comp_id = comp_id.map(str::to_owned).or(open);
    comp_node(root, comp_id.as_deref())
        .map(|comp| {
            layers_of(comp)
                .filter_map(|l| l.get("id").and_then(str_of).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// The canonical form of a project given in either form. A stored project
/// (the open composition's `layers` at the top level) is canonicalized; a
/// value that is already canonical is returned unchanged
/// (`canonical_project` is not idempotent: it would replace the open
/// composition with an empty one).
pub fn to_canonical(project: pm_model::Value) -> pm_model::Value {
    use pm_model::Value;
    let already = match &project {
        Value::Object(map) => {
            let open = map.get("compId").and_then(Value::as_str).filter(|s| !s.is_empty());
            let comp = open.and_then(|id| map.get("comps")?.as_object()?.get(id)?.as_object());
            !map.contains_key("layers") && comp.is_some()
        }
        _ => false,
    };
    if already {
        project
    } else {
        pm_model::canonical::canonical_project(project)
    }
}
