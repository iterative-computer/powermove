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

/// Keyframes (members of a `kf` array) under `node` whose id (`i`) is
/// `wanted`, read on the tree without converting it.
pub fn count_keys(node: &Node, wanted: &dyn Fn(&str) -> bool) -> usize {
    match node.kind() {
        NodeKind::Object(members) => members
            .iter()
            .map(|(key, value)| match value.kind() {
                NodeKind::Array(keys) if key.as_str() == Some("kf") => keys
                    .iter()
                    .filter(|k| k.get("i").and_then(str_of).is_some_and(wanted))
                    .count(),
                _ => count_keys(value, wanted),
            })
            .sum(),
        NodeKind::Array(items) => items.iter().map(|item| count_keys(item, wanted)).sum(),
        _ => 0,
    }
}

/// JavaScript truthiness of a node (`Value::is_truthy`).
pub(crate) fn truthy(node: &Node) -> bool {
    match node.kind() {
        NodeKind::Null => false,
        NodeKind::Bool(b) => *b,
        NodeKind::Number(n) => *n != 0.0 && !n.is_nan(),
        NodeKind::String(s) => !s.as_bytes().is_empty(),
        NodeKind::Array(_) | NodeKind::Object(_) => true,
    }
}

/// Whether any `expr` member under `node` is truthy (keyframe lists are not
/// searched): a superset of the properties that carry an expression.
pub(crate) fn holds_expression(node: &Node) -> bool {
    match node.kind() {
        NodeKind::Object(members) => members.iter().any(|(key, value)| match key.as_str() {
            Some("expr") => truthy(value),
            Some("kf") => false,
            _ => holds_expression(value),
        }),
        NodeKind::Array(items) => items.iter().any(holds_expression),
        _ => false,
    }
}

/// [`count_keys`] over every composition's layers, the layers split across
/// the worker threads.
pub fn count_layer_keys(root: &Node, wanted: &(dyn Fn(&str) -> bool + Sync)) -> usize {
    let layers: Vec<&Node> = match root.get("comps").map(Node::kind) {
        Some(NodeKind::Object(comps)) => {
            comps.iter().flat_map(|(_, comp)| layers_of(comp)).collect()
        }
        _ => Vec::new(),
    };
    holding_keys(&layers, wanted).into_iter().sum()
}

/// How many wanted keys each of `layers` holds, in parallel.
pub(crate) fn holding_keys(layers: &[&Node], wanted: &(dyn Fn(&str) -> bool + Sync)) -> Vec<usize> {
    pm_model::parallel::map(layers.len(), |i| count_keys(layers[i], wanted))
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

/// The id the engine addresses the open composition by: `compId`, or the
/// project `id` when the layers sit at the root.
pub fn render_comp_id(root: &Node) -> Option<String> {
    open_comp_id(root).or_else(|| {
        root.get("id")
            .and_then(str_of)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    })
}

/// The project's Output (`color.mode` and `color.hdrTransfer`): SDR unless
/// the mode is `hdr`, then PQ unless the transfer is `hlg`. The engine reads
/// the same members (`pm_render::composite::project_color`).
pub fn project_output(root: &Node) -> pm_protocol::color::ProjectOutput {
    use pm_protocol::color::ProjectOutput;
    let text = |key: &str| {
        root.get("color")
            .and_then(|c| c.get(key))
            .and_then(|v| match v.kind() {
                NodeKind::String(s) => Some(s.to_string_lossy()),
                _ => None,
            })
    };
    match (text("mode").as_deref(), text("hdrTransfer").as_deref()) {
        (Some("hdr"), Some("hlg")) => ProjectOutput::Hlg,
        (Some("hdr"), _) => ProjectOutput::Pq,
        _ => ProjectOutput::Sdr,
    }
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
    if open.is_some()
        && let Some(NodeKind::Object(comps)) = root.get("comps").map(Node::kind)
    {
        for (key, _) in comps.iter() {
            if let Some(k) = key.as_str()
                && Some(k) != open.as_deref()
            {
                out.push(Some(k.to_owned()));
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
    let mut next = layer
        .get("group")
        .and_then(str_of)
        .filter(|s| !s.is_empty());
    while let Some(id) = next {
        if seen.contains(&id) {
            break;
        }
        seen.push(id);
        let Some(group) = layers_of(comp).find(|l| {
            l.get("id").and_then(str_of) == Some(id)
                && l.get("type").and_then(str_of) == Some("group")
        }) else {
            break;
        };
        out.push(group);
        next = group
            .get("group")
            .and_then(str_of)
            .filter(|s| !s.is_empty());
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

thread_local! {
    static LAYER_WALKS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
/// How many walks of a composition's layers ([`list_layers`],
/// [`layer_names`]) this thread has made: a hot path (a frame) can be shown
/// to make none.
pub fn layer_walks() -> u64 {
    LAYER_WALKS.with(std::cell::Cell::get)
}
fn walked() {
    LAYER_WALKS.with(|w| w.set(w.get() + 1));
}
/// Each layer's id and name in the composition `comp_id` (`None`: the open
/// one), top of the stack first, named as [`list_layers`] names them: what a
/// list of names needs (the command palette asks on every frame), nothing
/// else read.
pub fn layer_names(root: &Node, comp_id: Option<&str>) -> Vec<(String, String)> {
    walked();
    let open = open_comp_id(root);
    let Some(comp) = comp_node(root, comp_id.or(open.as_deref())) else {
        return Vec::new();
    };
    layers_of(comp)
        .filter_map(|layer| {
            let id = layer.get("id").and_then(str_of)?.to_owned();
            let name = layer
                .get("name")
                .and_then(string_of)
                .map(JsString::to_string_lossy)
                .unwrap_or_default();
            Some((id, name))
        })
        .collect()
}

/// The layers of the composition `comp_id` (`None`: the open one).
pub fn list_layers(root: &Node, comp_id: Option<&str>) -> Vec<LayerInfo> {
    walked();
    let open = open_comp_id(root);
    let comp_id = comp_id.map(str::to_owned).or(open);
    let Some(comp) = comp_node(root, comp_id.as_deref()) else {
        return Vec::new();
    };
    // Group layers by id, once: each layer's lock walks its group chain
    // through this map (`is_locked` searches the stack per group, which over
    // every layer of a large composition was quadratic: 41 ms for 867).
    // The first group layer with an id wins, as `group_ancestors` finds it.
    let mut groups: std::collections::HashMap<&str, &Node> = Default::default();
    for l in layers_of(comp).filter(|l| l.get("type").and_then(str_of) == Some("group")) {
        if let Some(id) = l.get("id").and_then(str_of) {
            groups.entry(id).or_insert(l);
        }
    }
    let locked = |layer: &Node| {
        if layer.get("lock").and_then(bool_of) == Some(true) {
            return true;
        }
        let mut seen: Vec<&str> = layer.get("id").and_then(str_of).into_iter().collect();
        let mut next = layer.get("group").and_then(str_of).filter(|s| !s.is_empty());
        while let Some(id) = next {
            if seen.contains(&id) {
                break;
            }
            seen.push(id);
            let Some(group) = groups.get(id) else {
                break;
            };
            if group.get("lock").and_then(bool_of) == Some(true) {
                return true;
            }
            next = group.get("group").and_then(str_of).filter(|s| !s.is_empty());
        }
        false
    };
    layers_of(comp)
        .enumerate()
        .filter_map(|(index, layer)| {
            let id = layer.get("id").and_then(str_of)?.to_owned();
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
                effectively_locked: locked(layer),
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
pub fn to_canonical(mut project: pm_model::Value) -> pm_model::Value {
    use pm_model::Value;
    // The stored active composition lives in the top-level fields. A legacy
    // null or invalid alternate-composition map must not strand those rows.
    if let Value::Object(map) = &mut project
        && map
            .get("layers")
            .is_some_and(|layers| layers.as_array().is_some())
        && map
            .get("comps")
            .is_some_and(|comps| comps.as_object().is_none())
    {
        eprintln!("Recovered the active composition from the stored project fields");
        map.insert("comps", Value::Object(pm_model::Map::new()));
    }
    let already = match &project {
        Value::Object(map) => {
            let open = map
                .get("compId")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());
            let comp = open.and_then(|id| map.get("comps")?.as_object()?.get(id)?.as_object());
            !map.contains_key("layers") && comp.is_some()
        }
        _ => false,
    };
    let mut project = if already {
        project
    } else {
        pm_model::canonical::canonical_project(project)
    };
    name_keys_without_ids(&mut project);
    project
}

// ── keys stored without ids ────────────────────────────────────────────────
//
// v1.2.0 stores a shape path's vertex keys without the `i` every other key
// has. When a project is read, such a key is given an id derived from its
// composition, layer, property path and place in its track, so the same key
// has the same id on every open of the same file. It lives on the key for
// the session (a moved key keeps it) and is taken off whenever the project
// is written ([`strip_derived_key_ids`], [`without_derived_key_ids`]): the
// file keeps v1's form. The timeline, selection, every keyframe command and
// undo use it as any id.

/// The id a key stored without one is given (see above).
pub fn derived_key_id(comp: &str, layer: &str, path: &str, index: usize) -> String {
    format!("~{comp}~{layer}~{path}~{index}")
}

/// The [`derived_key_id`] for place `index` of a track of `len` keys, or
/// the first one after it (`index + len`, `index + 2 len`, ...) no key of
/// the track (`taken`) has; taken from then on.
fn free_key_id(
    taken: &mut std::collections::HashSet<String>,
    comp: &str,
    layer: &str,
    path: &str,
    index: usize,
    len: usize,
) -> String {
    let mut n = index;
    loop {
        let id = derived_key_id(comp, layer, path, n);
        if taken.insert(id.clone()) {
            return id;
        }
        n += len.max(1);
    }
}

/// Operations naming the keys an Undo or Redo (`ops`) left without an id,
/// or with a derived id another key of their track already has (history
/// written before a save names them by older places): each gets a free
/// [`derived_key_id`]. Only the layers `ops` reach are walked.
pub(crate) fn naming_ops(root: &Node, ops: Option<&[pm_model::Op]>) -> Vec<pm_model::Op> {
    use pm_model::Value;
    // The layers the operations reach: (composition or "" for project-level
    // layers, layer index); every layer for a whole layer list or more, or
    // without operations.
    let mut every = ops.is_none();
    let ops = ops.unwrap_or_default();
    let mut touched: std::collections::HashSet<(String, usize)> = Default::default();
    let key_of = |p: &PathPart| p.property_key().to_string_lossy();
    for op in ops {
        let path = op.path();
        let (comp, rest) = match path.first().map(key_of).as_deref() {
            Some("comps") if path.len() >= 3 && key_of(&path[2]) == "layers" => {
                (key_of(&path[1]), &path[3..])
            }
            Some("layers") => (String::new(), &path[1..]),
            Some("comps") if path.len() < 3 => {
                every = true;
                continue;
            }
            _ => continue,
        };
        match rest.first() {
            Some(PathPart::Index(i)) => {
                touched.insert((comp, *i as usize));
            }
            _ => every = true,
        }
    }
    if !every && touched.is_empty() {
        return Vec::new();
    }
    fn walk(
        node: &Node,
        at: &mut Vec<PathPart>,
        name: &mut String,
        comp: &str,
        layer: &str,
        out: &mut Vec<pm_model::Op>,
    ) {
        match node.kind() {
            NodeKind::Object(map) => {
                if let Some(NodeKind::Array(keys)) = map.get("kf".as_bytes()).map(Node::kind) {
                    let mut taken = std::collections::HashSet::new();
                    let ids: Vec<Option<&str>> =
                        keys.iter().map(|k| k.get("i").and_then(str_of)).collect();
                    // A stored id is kept; a derived one only the first time.
                    for id in ids.iter().flatten().filter(|i| !is_derived_key_id(i)) {
                        taken.insert((*id).to_owned());
                    }
                    for (index, id) in ids.iter().enumerate() {
                        let keep = id.is_some_and(|i| !is_derived_key_id(i) || taken.insert(i.to_owned()));
                        if keep || !matches!(keys[index].kind(), NodeKind::Object(_)) {
                            continue;
                        }
                        let fresh = free_key_id(&mut taken, comp, layer, name, index, keys.len());
                        let mut path = at.clone();
                        path.extend([
                            PathPart::Key(JsString::from("kf")),
                            PathPart::Index(index as f64),
                            PathPart::Key(JsString::from("i")),
                        ]);
                        out.push(pm_model::Op::Leaf(pm_model::patch::Patch::set(
                            path,
                            Value::from(fresh.as_str()),
                        )));
                    }
                }
                for (k, child) in map.iter() {
                    if k.as_bytes() == b"kf"
                        || !matches!(child.kind(), NodeKind::Object(_) | NodeKind::Array(_))
                    {
                        continue;
                    }
                    let len = name.len();
                    if !name.is_empty() {
                        name.push('.');
                    }
                    name.push_str(&k.to_string_lossy());
                    at.push(PathPart::Key(k.clone()));
                    walk(child, at, name, comp, layer, out);
                    at.pop();
                    name.truncate(len);
                }
            }
            NodeKind::Array(items) => {
                for (i, child) in items.iter().enumerate() {
                    if !matches!(child.kind(), NodeKind::Object(_) | NodeKind::Array(_)) {
                        continue;
                    }
                    let len = name.len();
                    if !name.is_empty() {
                        name.push('.');
                    }
                    match child.get("id").and_then(str_of) {
                        Some(id) => name.push_str(id),
                        None => name.push_str(&i.to_string()),
                    }
                    at.push(PathPart::Index(i as f64));
                    walk(child, at, name, comp, layer, out);
                    at.pop();
                    name.truncate(len);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    let mut lists: Vec<(String, Vec<PathPart>, &Node)> = Vec::new();
    if let Some(layers) = root.get("layers") {
        lists.push((String::new(), vec![PathPart::Key(JsString::from("layers"))], layers));
    }
    if let Some(NodeKind::Object(comps)) = root.get("comps").map(Node::kind) {
        for (cid, comp) in comps.iter() {
            if let Some(layers) = comp.get("layers") {
                let at = vec![
                    PathPart::Key(JsString::from("comps")),
                    PathPart::Key(cid.clone()),
                    PathPart::Key(JsString::from("layers")),
                ];
                lists.push((cid.to_string_lossy(), at, layers));
            }
        }
    }
    for (comp, at, layers) in lists {
        let NodeKind::Array(items) = layers.kind() else {
            continue;
        };
        for (i, layer) in items.iter().enumerate() {
            if !every && !touched.contains(&(comp.clone(), i)) {
                continue;
            }
            let id = layer.get("id").and_then(str_of).unwrap_or_default().to_owned();
            let mut path = at.clone();
            path.push(PathPart::Index(i as f64));
            walk(layer, &mut path, &mut String::new(), &comp, &id, &mut out);
        }
    }
    out
}

/// The canonical hash of the document a reopen of `document` as written
/// would read (its derived ids taken off, then given again by place), or
/// `None` when it has no key stored without an id (its own hash is that).
/// A recovery journal's base is that document: a session whose keys moved
/// since they were named differs from it.
pub(crate) fn reopened_hash(document: &std::sync::Arc<pm_model::Document>) -> Option<String> {
    let stripped = without_derived_key_ids(document);
    let root_stripped = stripped.root()?;
    let naming = naming_ops(root_stripped, None);
    if naming.is_empty() {
        // Nothing stored without an id: the document is what a reopen reads.
        return None;
    }
    let revision = stripped.revision();
    let mut named = pm_model::Document::from_root(Some(root_stripped.clone()), revision);
    named.apply_ops_at(&naming, revision + 1).ok()?;
    named.canonical_hash()
}

/// Whether `id` is a [`derived_key_id`] (never stored).
pub fn is_derived_key_id(id: &str) -> bool {
    id.strip_prefix('~')
        .and_then(|rest| rest.rsplit_once('~'))
        .is_some_and(|(head, index)| {
            index.parse::<usize>().is_ok() && head.matches('~').count() >= 2
        })
}

/// Gives every key stored without an id in a canonical project its
/// [`derived_key_id`]. Keys with ids are left alone, so naming twice is
/// naming once.
pub fn name_keys_without_ids(project: &mut pm_model::Value) {
    use pm_model::Value;
    fn walk(v: &mut Value, comp: &str, layer: &str, path: &mut String) {
        match v {
            Value::Object(m) => {
                if let Some(Value::Array(keys)) = m.get_mut("kf") {
                    let mut taken: std::collections::HashSet<String> = keys
                        .iter()
                        .filter_map(|k| k.get("i").and_then(Value::as_str).map(str::to_owned))
                        .collect();
                    let len = keys.len();
                    for (index, key) in keys.iter_mut().enumerate() {
                        if let Value::Object(k) = key
                            && k.get("i").is_none()
                        {
                            let id = free_key_id(&mut taken, comp, layer, path, index, len);
                            k.insert("i", Value::from(id.as_str()));
                        }
                    }
                }
                for (name, child) in m.iter_mut() {
                    if name.as_bytes() == b"kf" {
                        continue;
                    }
                    let len = path.len();
                    if !path.is_empty() {
                        path.push('.');
                    }
                    path.push_str(&name.to_string_lossy());
                    walk(child, comp, layer, path);
                    path.truncate(len);
                }
            }
            Value::Array(items) => {
                for (i, child) in items.iter_mut().enumerate() {
                    let len = path.len();
                    // A record is named by its id: the path stays the same
                    // when a sibling is added or removed before it.
                    let name = child
                        .get("id")
                        .and_then(Value::as_str)
                        .map_or_else(|| i.to_string(), str::to_owned);
                    if !path.is_empty() {
                        path.push('.');
                    }
                    path.push_str(&name);
                    walk(child, comp, layer, path);
                    path.truncate(len);
                }
            }
            _ => {}
        }
    }
    fn layers(list: Option<&mut Value>, comp: &str) {
        for layer in list
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            let id = layer
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            walk(layer, comp, &id, &mut String::new());
        }
    }
    let Some(root) = project.as_object_mut() else {
        return;
    };
    layers(root.get_mut("layers"), "");
    if let Some(comps) = root.get_mut("comps").and_then(Value::as_object_mut) {
        for (id, comp) in comps.iter_mut() {
            let id = id.to_string_lossy();
            layers(comp.as_object_mut().and_then(|c| c.get_mut("layers")), &id);
        }
    }
}

/// Takes every [`derived_key_id`] off a project value (stored or
/// canonical) about to be written.
pub fn strip_derived_key_ids(project: &mut pm_model::Value) {
    use pm_model::Value;
    match project {
        Value::Object(m) => {
            if let Some(Value::Array(keys)) = m.get_mut("kf") {
                for key in keys.iter_mut() {
                    if let Value::Object(k) = key
                        && k.get("i").and_then(Value::as_str).is_some_and(is_derived_key_id)
                    {
                        k.remove("i");
                    }
                }
            }
            for (_, child) in m.iter_mut() {
                strip_derived_key_ids(child);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(strip_derived_key_ids),
        _ => {}
    }
}

/// The document without its [`derived_key_id`]s, for a save: the same
/// document when it has none.
pub fn without_derived_key_ids(
    document: &std::sync::Arc<pm_model::Document>,
) -> std::sync::Arc<pm_model::Document> {
    fn walk(node: &Node, path: &mut Vec<PathPart>, out: &mut Vec<Vec<PathPart>>) {
        match node.kind() {
            NodeKind::Object(map) => {
                if let Some(NodeKind::Array(keys)) = map.get("kf".as_bytes()).map(Node::kind) {
                    for (index, key) in keys.iter().enumerate() {
                        if key.get("i").and_then(str_of).is_some_and(is_derived_key_id) {
                            let mut at = path.clone();
                            at.extend([
                                PathPart::Key(JsString::from("kf")),
                                PathPart::Index(index as f64),
                                PathPart::Key(JsString::from("i")),
                            ]);
                            out.push(at);
                        }
                    }
                }
                for (name, child) in map.iter() {
                    if matches!(child.kind(), NodeKind::Object(_) | NodeKind::Array(_)) {
                        path.push(PathPart::Key(name.clone()));
                        walk(child, path, out);
                        path.pop();
                    }
                }
            }
            NodeKind::Array(items) => {
                for (i, child) in items.iter().enumerate() {
                    if matches!(child.kind(), NodeKind::Object(_) | NodeKind::Array(_)) {
                        path.push(PathPart::Index(i as f64));
                        walk(child, path, out);
                        path.pop();
                    }
                }
            }
            _ => {}
        }
    }
    let Some(root) = document.root() else {
        return document.clone();
    };
    let mut deletes = Vec::new();
    walk(root, &mut Vec::new(), &mut deletes);
    if deletes.is_empty() {
        return document.clone();
    }
    let ops: Vec<pm_model::Op> = deletes
        .into_iter()
        .map(|path| pm_model::Op::Leaf(pm_model::patch::Patch::delete(path)))
        .collect();
    let revision = document.revision();
    let mut stripped = pm_model::Document::from_root(Some(root.clone()), revision);
    if stripped.apply_ops_at(&ops, revision + 1).is_err() {
        // Not expected (the paths come from this tree): strip a copy.
        let mut value = root.to_value();
        strip_derived_key_ids(&mut value);
        stripped = pm_model::Document::new(Some(value), revision);
    }
    std::sync::Arc::new(stripped)
}

#[cfg(test)]
mod derived_key_tests {
    use super::*;
    use pm_model::json::parse;

    /// A recovery base names keys as a reopen does: the hash of a session
    /// whose keys stayed put equals a reopen's; once a key moved it differs.
    #[test]
    fn a_reopened_hash_matches_a_reopen() {
        let stored = parse(
            r##"{"id":"P","compId":"C","w":10,"h":10,"fps":30,"dur":4,"assets":{},"comps":{"D":{"id":"D","layers":[{"id":"M","p":{"x":{"v":0,"kf":[{"t":0,"v":1},{"t":1,"v":2,"i":"k"}]}}}]}},
            "layers":[{"id":"L","d":{"paths":[{"id":"o","vertices":[{"id":"v0","p":{"x":{"v":0,"kf":[{"t":0,"v":0},{"t":1,"v":1},{"t":2,"v":2}]}}},{"p":{"y":{"v":0,"kf":[{"t":0,"v":0,"i":"~C~L~d.paths.o.vertices.1.p.y~0"},{"t":1,"v":1}]}}}]}]}}]}"##,
        )
        .unwrap();
        let reopen = |v: &pm_model::Value| {
            std::sync::Arc::new(pm_model::Document::new(Some(to_canonical(v.clone())), 1))
        };
        let session = reopen(&stored);
        assert_eq!(
            reopened_hash(&session),
            session.canonical_hash(),
            "nothing moved: the session is what a reopen reads"
        );
        // Move the first vertex key to the end, as an edit would.
        let mut moved = session.root().unwrap().to_value();
        fn at<'a>(v: &'a mut pm_model::Value, path: &[&str]) -> &'a mut pm_model::Value {
            path.iter().fold(v, |v, step| match step.parse::<usize>() {
                Ok(i) => &mut v.as_array_mut().unwrap()[i],
                Err(_) => v.as_object_mut().unwrap().get_mut(*step).unwrap(),
            })
        }
        let kf = at(
            &mut moved,
            &["comps", "C", "layers", "0", "d", "paths", "0", "vertices", "0", "p", "x", "kf"],
        )
        .as_array_mut()
        .unwrap();
        let first = kf.remove(0);
        kf.push(first);
        let moved = std::sync::Arc::new(pm_model::Document::new(Some(moved), 2));
        let written = without_derived_key_ids(&moved);
        let read_back = reopen(&written.root().unwrap().to_value());
        assert_eq!(reopened_hash(&moved), read_back.canonical_hash());
        assert_ne!(reopened_hash(&moved), moved.canonical_hash());
    }
}
