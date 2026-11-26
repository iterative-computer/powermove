use pm_kernel::{Snapshot, model};
use pm_model::{JsString, Node, NodeKind, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock, Weak};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RowId {
    Layer(String),
    Effect { layer_id: String, effect: String },
    Path { layer_id: String, path: String },
    Property { layer_id: String, channel: String },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: RowId,
    pub depth: usize,
}
#[derive(Debug)]
pub struct Window<'a> {
    pub total: usize,
    pub first: usize,
    pub rows: &'a [Row],
}
#[derive(Clone, Debug)]
pub struct Key {
    pub id: String,
    /// Authored layer-local time; never frame-normalised on read.
    pub time: f64,
    pub value: Value,
    /// Complete stored key, including unknown fields, shared with the
    /// immutable document. Materialize a value only for clipboard/edits.
    pub stored: Node,
}
impl Key {
    /// A hold key, as the file stores it: v1's `hold: true`, or the AE
    /// model's `outInterp: "hold"`.
    pub fn held(&self) -> bool {
        boolean(self.stored.get("hold")) || self.stored_string("outInterp") == Some("hold")
    }
    pub fn stored_string(&self, field: &str) -> Option<&str> {
        match self.stored.get(field)?.kind() {
            NodeKind::String(value) => value.as_str(),
            _ => None,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Channel {
    pub path: String,
    pub value: Value,
    pub expression: Option<JsString>,
    pub keys: Vec<Key>,
    pub(crate) path_group: Option<String>,
    pub(crate) vertex_label: Option<String>,
}
/// A layer's transition at one edge, as the timeline draws and edits it.
#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    /// The registry id (`type`); `fade` for an audio clip's fade.
    pub kind: String,
    pub duration: f64,
    /// Its extension is not installed: it shows, but is not edited.
    pub missing: bool,
    /// The parameter keys it stores (`p`), kept by a duration edit.
    pub params: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct Layer {
    pub id: String,
    pub(crate) row_id: String,
    pub name: JsString,
    pub kind: String,
    pub group: Option<String>,
    pub parent: Option<String>,
    pub from: f64,
    pub duration: f64,
    pub locked: bool,
    pub effectively_locked: bool,
    pub visible: bool,
    pub solo: bool,
    /// Shy: left out of the timeline's rows while Hide Shy Layers is on.
    pub shy: bool,
    pub three_d: bool,
    pub audio_muted: bool,
    pub collapsed: bool,
    pub group_collapsed: bool,
    pub preferred_track: usize,
    pub scale_linked: bool,
    /// True only when `scaleLinked` is explicitly `false` (unlinked).
    pub scale_unlinked: bool,
    pub separate_dimensions: bool,
    /// True only when `separateDimensions` is explicitly `true` (Position X/Y
    /// separated in the Graph Editor).
    pub position_separated: bool,
    pub effects: Vec<(String, String)>,
    path_groups: HashMap<String, String>,
    /// The media asset an image, video or audio layer draws (`d.asset`).
    pub asset: Option<String>,
    /// Seconds of the media skipped before the layer starts (`d.trim`).
    pub trim: f64,
    /// An audio layer's level (`d.gain`, 1 = 0 dB; its base value when
    /// animated), and its fades in seconds (`d.fadeIn`, `d.fadeOut`).
    pub gain: f64,
    /// `d.gain` has keys or an expression: the level line shows the base.
    pub gain_animated: bool,
    pub fade_in: f64,
    pub fade_out: f64,
    /// Effects whose extension is missing (`fx[].missing`): the layer draws
    /// with them bypassed. Their display names, else types.
    pub missing_effects: Vec<String>,
    /// The In and Out transitions (`transitionIn`, `transitionOut`); an
    /// audio clip's fades (`d.fadeIn`, `d.fadeOut`) stand in for them.
    pub transitions: [Option<Transition>; 2],
    pub(crate) has_authored_transition: bool,
    pub masks: Vec<String>,
    /// The composition id referenced by a `precomp` layer (`d.comp`).
    pub comp: Option<String>,
    pub definition: Option<String>,
    pub channels: Vec<Channel>,
    /// Revision at which the layer's tree node last changed: an unchanged
    /// layer is reused, not read again, by the next index.
    channel_at: HashMap<String, usize>,
    key_at: HashMap<String, (usize, usize)>,
    /// The collapsed row's key marks, sorted, none selected: built once per
    /// read of the layer and shared by every scene until the layer changes.
    summary: std::sync::OnceLock<Arc<[crate::render::Key]>>,
}

impl Layer {
    pub(crate) fn channel(&self, path: &str) -> Option<&Channel> {
        self.channel_at.get(path).map(|i| &self.channels[*i])
    }

    pub fn can_be_3d(&self) -> bool {
        !matches!(self.kind.as_str(), "audio" | "adjustment")
    }

    pub(crate) fn has_keyframed_channels(&self) -> bool {
        self.channels
            .iter()
            .any(|channel| !channel.keys.is_empty() && self.channel_available(&channel.path))
    }

    pub(crate) fn channel_available(&self, path: &str) -> bool {
        self.three_d
            || !matches!(
                path,
                "perspective"
                    | "position.z"
                    | "anchor.z"
                    | "scale.z"
                    | "rotation.x"
                    | "rotation.y"
                    | "orientation.x"
                    | "orientation.y"
                    | "orientation.z"
                    | "dofBlur"
                    | "dofFocus"
            )
    }
}

impl Layer {
    pub(crate) fn path_name(&self, path: &str) -> &str {
        self.path_groups.get(path).map_or("Path", String::as_str)
    }

    /// The collapsed row's key marks (see `summary`), made by `fill` once.
    pub(crate) fn summary(
        &self,
        fill: impl FnOnce(&Layer) -> Vec<crate::render::Key>,
    ) -> Arc<[crate::render::Key]> {
        self.summary
            .get_or_init(|| {
                let mut keys = fill(self);
                keys.sort_by(|a, b| a.seconds.total_cmp(&b.seconds));
                keys.into()
            })
            .clone()
    }
}
#[derive(Clone, Debug)]
pub struct Marker {
    pub id: Option<String>,
    pub time: f64,
    pub name: JsString,
}
#[derive(Clone, Debug)]
pub struct TimelineIndex {
    pub doc_id: u64,
    pub revision: u64,
    pub comp_id: Option<String>,
    pub(crate) source: Option<Snapshot>,
    pub fps: f64,
    pub duration: f64,
    pub work: [f64; 2],
    pub layers: Vec<Arc<Layer>>,
    pub markers: Vec<Marker>,
    by_id: Arc<HashMap<String, usize>>,
    source_ordinals: Arc<Vec<usize>>,
    hierarchy: Arc<Hierarchy>,
    locked_descendants: Arc<Vec<bool>>,
    pub(crate) shy_layers: bool,
    /// Legacy groups use the union of their leaf layers' timing.
    group_extents: Arc<Vec<Option<(f64, f64)>>>,
    layer_time_bounds: (f64, f64),
    /// Key ids by layer position, [`KEY_BLOCK`] layers a block. A block whose
    /// layers are all reused is reused, so an edit re-indexes one block.
    key_blocks: Vec<Arc<KeyBlock>>,
    split_targets: Arc<OnceLock<Vec<(f64, usize)>>>,
}

#[derive(Debug, Default)]
struct Hierarchy {
    children: Vec<Vec<usize>>,
    roots: Vec<usize>,
    parent: Vec<Option<usize>>,
}

fn same_node(old: &Node, new: &Node) -> bool {
    old.rev() == new.rev()
        && match (old.kind(), new.kind()) {
            (NodeKind::Object(a), NodeKind::Object(b)) => Arc::ptr_eq(a, b),
            (NodeKind::Array(a), NodeKind::Array(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
}
const KEY_BLOCK: usize = 64;
#[derive(Debug, Default)]
struct KeyBlock {
    layers: Vec<Arc<Layer>>,
    /// Each key id's last layer in the block.
    keys: HashMap<String, usize>,
    /// Every layer of an id more than one layer of the block holds (the
    /// kernel lets key ids repeat across layers).
    repeated: HashMap<String, Vec<usize>>,
}
impl KeyBlock {
    fn new(layers: &[Arc<Layer>]) -> Self {
        let mut keys = HashMap::new();
        let mut repeated: HashMap<String, Vec<usize>> = HashMap::new();
        for (offset, layer) in layers.iter().enumerate() {
            for id in layer.key_at.keys() {
                if let Some(previous) = keys.insert(id.clone(), offset) {
                    repeated
                        .entry(id.clone())
                        .or_insert_with(|| vec![previous])
                        .push(offset);
                }
            }
        }
        Self {
            layers: layers.to_vec(),
            keys,
            repeated,
        }
    }
    fn holders(&self, id: &str) -> impl Iterator<Item = &Arc<Layer>> {
        let offsets = match self.repeated.get(id) {
            Some(all) => all.as_slice(),
            None => self.keys.get(id).map_or(&[][..], std::slice::from_ref),
        };
        offsets.iter().map(|offset| &self.layers[*offset])
    }
    fn holds(&self, layers: &[Arc<Layer>]) -> bool {
        self.layers.len() == layers.len()
            && self
                .layers
                .iter()
                .zip(layers)
                .all(|(a, b)| Arc::ptr_eq(a, b))
    }
}
impl Default for TimelineIndex {
    fn default() -> Self {
        Self {
            doc_id: 0,
            revision: 0,
            comp_id: None,
            source: None,
            fps: 30.,
            duration: 0.,
            work: [0., 0.],
            layers: Vec::new(),
            markers: Vec::new(),
            by_id: Default::default(),
            source_ordinals: Default::default(),
            hierarchy: Default::default(),
            locked_descendants: Default::default(),
            shy_layers: false,
            group_extents: Default::default(),
            layer_time_bounds: (0., 0.),
            key_blocks: Vec::new(),
            split_targets: Default::default(),
        }
    }
}
impl TimelineIndex {
    /// Read-only source for edits that need media-handle metadata.
    pub fn root(&self) -> Option<&Node> {
        self.source.as_ref()?.root()
    }

    /// The native painter and panel outline read the same immutable index.
    /// Weak entries never keep a closed document or an old edit alive.
    pub(crate) fn shared(snapshot: &Snapshot, previous: &Self) -> Arc<Self> {
        type Key = (u64, u64, u64);
        static INDEXES: OnceLock<Mutex<HashMap<Key, Weak<TimelineIndex>>>> = OnceLock::new();
        let mut indexes = INDEXES.get_or_init(Default::default).lock().unwrap();
        indexes.retain(|_, index| index.strong_count() > 0);
        // The edition distinguishes independent snapshots with equal ids and
        // revisions (including synthetic/test documents). It is never reused.
        let key = (
            snapshot.doc_id,
            snapshot.revision,
            snapshot.document.edition(),
        );
        if let Some(index) = indexes.get(&key).and_then(Weak::upgrade) {
            return index;
        }
        let index = Arc::new(previous.updated(snapshot));
        indexes.insert(key, Arc::downgrade(&index));
        index
    }

    /// An open composition: `comps/<compId>`, or the root of a project whose
    /// layers sit at the top level.
    pub fn has_composition(&self) -> bool {
        self.source
            .as_ref()
            .and_then(Snapshot::root)
            .and_then(|root| model::comp_node(root, self.comp_id.as_deref()))
            .is_some()
    }
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        Self::default().updated(snapshot)
    }
    /// The index of `snapshot`, reusing every layer whose tree node has not
    /// changed since this index was read (same document and composition):
    /// an edit reads the layers it touched, not the composition.
    pub fn updated(&self, snapshot: &Snapshot) -> Self {
        let mut result = Self {
            doc_id: snapshot.doc_id,
            revision: snapshot.revision,
            comp_id: snapshot.open_comp(),
            source: Some(snapshot.clone()),
            ..Self::default()
        };
        let same = self.doc_id == snapshot.doc_id
            && self.source.is_some()
            && self.comp_id == result.comp_id;
        let Some(root) = snapshot.root() else {
            return result;
        };
        let Some(comp) = model::comp_node(root, result.comp_id.as_deref()) else {
            return result;
        };
        result.fps = number(comp.get("fps")).filter(|n| *n > 0.).unwrap_or(30.);
        result.duration = number(comp.get("dur")).unwrap_or(0.).max(0.);
        result.work = [
            number(comp.get("work").and_then(|v| v.at(0))).unwrap_or(0.),
            number(comp.get("work").and_then(|v| v.at(1))).unwrap_or(result.duration),
        ];
        let layers = array(comp.get("layers"));
        let old_comp = same
            .then(|| {
                self.root()
                    .and_then(|root| model::comp_node(root, self.comp_id.as_deref()))
            })
            .flatten();
        let old_layers = array(old_comp.and_then(|comp| comp.get("layers")));
        if let Some((layers, timing_changed, locks_changed)) =
            self.update_in_place(old_layers, layers)
        {
            result.layers = layers;
            result.by_id = self.by_id.clone();
            result.source_ordinals = self.source_ordinals.clone();
            result.hierarchy = self.hierarchy.clone();
            if locks_changed {
                result.index_locks();
            } else {
                result.locked_descendants = self.locked_descendants.clone();
            }
            if timing_changed {
                result.index_timing();
            } else {
                result.layer_time_bounds = self.layer_time_bounds;
                result.group_extents = self.group_extents.clone();
            }
        } else {
            let mut by_id = HashMap::with_capacity(layers.len());
            let mut ordinals = Vec::with_capacity(layers.len());
            result.layers.reserve(layers.len());
            for (ordinal, layer) in layers.iter().enumerate() {
                let Some(id) = string(layer.get("id")).filter(|id| !id.is_empty()) else {
                    eprintln!("Timeline skipped layer {} with no usable id", ordinal + 1);
                    continue;
                };
                if by_id.contains_key(&id) {
                    eprintln!("Timeline skipped duplicate layer id at row {}", ordinal + 1);
                    continue;
                }
                let Some((from, duration)) = Self::timing(layer) else {
                    eprintln!("Timeline skipped layer {} with invalid timing", ordinal + 1);
                    continue;
                };
                let previous = same
                    .then(|| self.layer_ordinal(&id))
                    .flatten()
                    .map(|i| (&self.layers[i], &old_layers[self.source_ordinals[i]]));
                let indexed = match previous {
                    Some((indexed, old)) if same_node(old, layer) => indexed.clone(),
                    _ => Arc::new(Self::read_layer(
                        layer,
                        id.clone(),
                        from,
                        duration,
                        previous.map(|(index, node)| (index.as_ref(), node)),
                    )),
                };
                by_id.insert(id, result.layers.len());
                ordinals.push(ordinal);
                result.layers.push(indexed);
            }
            result.by_id = Arc::new(by_id);
            result.source_ordinals = Arc::new(ordinals);
            result.index_hierarchy();
            result.index_locks();
            result.index_timing();
        }
        result.shy_layers = result.layers.iter().any(|layer| layer.shy);
        result.index_keys(self);
        if let Some(NodeKind::Array(markers)) = comp.get("markers").map(Node::kind) {
            result.markers = markers
                .iter()
                .filter_map(|m| {
                    Some(Marker {
                        id: string(m.get("id")),
                        time: number(m.get("t"))?,
                        name: js_string(m.get("name")).unwrap_or_default(),
                    })
                })
                .collect();
        }
        result
    }

    fn timing(node: &Node) -> Option<(f64, f64)> {
        let from = number(node.get("from")).unwrap_or(0.);
        let duration = number(node.get("dur")).unwrap_or(0.);
        (duration >= 0. && (from + duration).is_finite()).then_some((from, duration))
    }

    /// Compare persistent layer nodes in stack order. Scalar edits leave every
    /// other object and the layer topology intact, even across missed revisions.
    fn update_in_place(
        &self,
        old_nodes: &[Node],
        nodes: &[Node],
    ) -> Option<(Vec<Arc<Layer>>, bool, bool)> {
        if nodes.len() != old_nodes.len() || old_nodes.len() != self.layers.len() {
            return None;
        }
        let mut layers = Vec::with_capacity(nodes.len());
        let mut timing_changed = false;
        let mut locks_changed = false;
        for ((old, node), indexed) in old_nodes.iter().zip(nodes).zip(&self.layers) {
            if same_node(old, node) {
                layers.push(indexed.clone());
                continue;
            }
            if node.get("id").and_then(as_str) != Some(indexed.id.as_str())
                || node.get("group").and_then(as_str) != indexed.group.as_deref()
                || node.get("type").and_then(as_str).unwrap_or_default() != indexed.kind
            {
                return None;
            }
            let (from, duration) = Self::timing(node)?;
            timing_changed |= (from, duration) != (indexed.from, indexed.duration);
            let mut layer = Self::read_layer(
                node,
                indexed.id.clone(),
                from,
                duration,
                Some((indexed, old)),
            );
            locks_changed |= layer.locked != indexed.locked;
            layer.effectively_locked = indexed.effectively_locked;
            layers.push(Arc::new(layer));
        }
        Some((layers, timing_changed, locks_changed))
    }

    fn index_timing(&mut self) {
        for layer in &self.layers {
            self.layer_time_bounds.0 = self.layer_time_bounds.0.min(layer.from);
            self.layer_time_bounds.1 = self.layer_time_bounds.1.max(layer.from + layer.duration);
        }
        if self
            .layers
            .iter()
            .any(|layer| layer.kind == "group" && layer.duration == 0.)
        {
            self.group_extents = Arc::new(self.group_extents());
        }
    }

    pub(crate) fn shares_hierarchy(&self, previous: &Self) -> bool {
        Arc::ptr_eq(&self.hierarchy, &previous.hierarchy)
    }

    fn index_hierarchy(&mut self) {
        let len = self.layers.len();
        let parent: Vec<_> = self
            .layers
            .iter()
            .map(|layer| layer.group.as_deref().and_then(|id| self.layer_ordinal(id)))
            .collect();
        let mut children = vec![Vec::new(); len];
        let mut roots = Vec::new();
        for (i, p) in parent.iter().copied().enumerate() {
            if let Some(p) = p.filter(|p| *p != i) {
                children[p].push(i);
            } else {
                roots.push(i);
            }
        }
        // Keep cyclic layers reachable in stored order, as with regular roots.
        let mut seen = vec![false; len];
        let mut stack = Vec::new();
        for candidate in roots.clone().into_iter().chain(0..len) {
            if seen[candidate] {
                continue;
            }
            if parent[candidate].is_some_and(|p| p != candidate) {
                roots.push(candidate);
            }
            stack.push(candidate);
            while let Some(i) = stack.pop() {
                if !std::mem::replace(&mut seen[i], true) {
                    stack.extend(&children[i]);
                }
            }
        }
        self.hierarchy = Arc::new(Hierarchy {
            children,
            roots,
            parent,
        });
    }

    fn index_locks(&mut self) {
        let len = self.layers.len();
        let parent = &self.hierarchy.parent;
        // Memoised group locks, bounded even on corrupt/cyclic input. Transform parents
        // do not imply group locks (kernel's lock policy).
        let mut locked = vec![None; len];
        let mut chain = Vec::new();
        let mut visited = vec![None; len];
        for first in 0..len {
            let mut at = Some(first);
            let mut inherited = false;
            while let Some(i) = at {
                if let Some(value) = locked[i] {
                    inherited = value;
                    break;
                }
                if visited[i] == Some(first) {
                    inherited = chain.iter().any(|i: &usize| self.layers[*i].locked);
                    break;
                }
                visited[i] = Some(first);
                chain.push(i);
                at = parent[i];
            }
            for i in chain.drain(..).rev() {
                inherited |= self.layers[i].locked;
                locked[i] = Some(inherited);
            }
        }
        let mut locked_descendants = vec![false; len];
        for (i, layer) in self.layers.iter_mut().enumerate() {
            let effective = locked[i].unwrap_or(layer.locked);
            if layer.effectively_locked != effective {
                Arc::make_mut(layer).effectively_locked = effective;
            }
        }
        for (i, _) in self
            .layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| layer.locked)
        {
            let mut at = parent[i];
            while let Some(p) = at {
                if p == i || self.layers[p].kind != "group" || locked_descendants[p] {
                    break;
                }
                locked_descendants[p] = true;
                at = parent[p];
            }
        }
        self.locked_descendants = Arc::new(locked_descendants);
    }

    fn index_keys(&mut self, old: &Self) {
        let previous: HashMap<*const Layer, &Arc<KeyBlock>> = old
            .key_blocks
            .iter()
            .filter_map(|b| Some((Arc::as_ptr(b.layers.first()?), b)))
            .collect();
        self.key_blocks = self
            .layers
            .chunks(KEY_BLOCK)
            .map(|chunk| match previous.get(&Arc::as_ptr(&chunk[0])) {
                Some(block) if block.holds(chunk) => Arc::clone(block),
                _ => Arc::new(KeyBlock::new(chunk)),
            })
            .collect();
    }
    /// One stored layer as the timeline reads it.
    fn read_layer(
        layer: &Node,
        id: String,
        from: f64,
        duration: f64,
        previous: Option<(&Layer, &Node)>,
    ) -> Layer {
        let mut channels = Vec::new();
        let mut path_groups = HashMap::new();
        collect_channels(layer.get("p"), "", &mut channels);
        collect_direct(layer.get("d"), "c", &mut channels);
        if let Some(NodeKind::Array(effects)) = layer.get("fx").map(Node::kind) {
            for effect in effects.iter() {
                if let Some(id) = string(effect.get("id")) {
                    collect_channels(effect.get("p"), &id, &mut channels);
                    collect_channels(effect.get("on"), &format!("{id}.$enabled"), &mut channels);
                }
            }
        }
        if let Some(NodeKind::Array(masks)) = layer.get("masks").map(Node::kind) {
            for mask in masks.iter() {
                if let Some(id) = string(mask.get("id")) {
                    collect_direct(mask.get("p"), &format!("m.{id}"), &mut channels);
                    for field in ["shape", "mode", "on"] {
                        collect_channels(
                            mask.get(field),
                            &format!("m.{id}.{field}"),
                            &mut channels,
                        );
                    }
                    if let Some(path) = mask.get("path") {
                        collect_path(path, &format!("mp.{id}"), &mut channels, &mut path_groups);
                    }
                }
            }
        }
        // Layer switch channels participate in key navigation too.
        for (field, path) in [
            ("matteMode", "l.matteMode"),
            ("blend", "l.blend"),
            ("mblur", "l.mblur"),
        ] {
            collect_channels(layer.get(field), path, &mut channels);
        }
        let content = layer.get("d");
        if string(layer.get("type")).as_deref() == Some("shader") {
            collect_direct(content.and_then(|d| d.get("uniforms")), "u", &mut channels);
        }
        if string(layer.get("type")).as_deref() == Some("extension") {
            collect_direct(content.and_then(|d| d.get("params")), "x", &mut channels);
        }
        for field in ["transitionIn", "transitionOut"] {
            collect_direct(
                layer.get(field).and_then(|t| t.get("p")),
                &format!("{field}.p"),
                &mut channels,
            );
        }
        if let Some(paths) = content.and_then(|d| d.get("paths")) {
            if let Some((indexed, _)) = previous.filter(|(_, old)| {
                old.get("d")
                    .and_then(|d| d.get("paths"))
                    .is_some_and(|old| same_node(old, paths))
            }) {
                channels.extend(
                    indexed
                        .channels
                        .iter()
                        .filter(|c| {
                            c.path_group
                                .as_deref()
                                .is_some_and(|group| group.starts_with("g."))
                        })
                        .cloned(),
                );
                path_groups.extend(
                    indexed
                        .path_groups
                        .iter()
                        .filter(|(id, _)| id.starts_with("g."))
                        .map(|(id, name)| (id.clone(), name.clone())),
                );
            } else {
                for path in array(Some(paths)) {
                    if let Some(id) = string(path.get("id")) {
                        collect_path(path, &format!("g.{id}"), &mut channels, &mut path_groups);
                    }
                }
            }
        }
        for (field, prefix) in [("animators", "ta"), ("styles", "ts")] {
            for item in array(content.and_then(|d| d.get(field))) {
                if let Some(id) = string(item.get("id")) {
                    collect_direct(item.get("p"), &format!("{prefix}.{id}"), &mut channels);
                }
            }
        }
        let data = content.and_then(|d| d.get("data"));
        let scene = data.and_then(|d| d.get("scene"));
        for (field, prefix) in [("objects", "o"), ("lights", "light")] {
            for item in array(scene.and_then(|s| s.get(field))) {
                if let Some(id) = string(item.get("id")) {
                    collect_direct(item.get("p"), &format!("{prefix}.{id}"), &mut channels);
                    if field == "objects" {
                        collect_direct(
                            item.get("material").and_then(|m| m.get("p")),
                            &format!("o.{id}.m"),
                            &mut channels,
                        );
                    }
                }
            }
        }
        for field in ["camera", "environment"] {
            collect_direct(
                scene.and_then(|s| s.get(field)).and_then(|n| n.get("p")),
                field,
                &mut channels,
            );
        }
        // Standalone 3D layers (`powermove.3d.camera`, `powermove.3d.light`,
        // `powermove.3d.object`) store animated channels directly under `d.data`
        // (e.g. `camera.targetX/Y/Z`, `camera.fov`, `light.targetX/Y/Z`, `m.*`).
        if scene.is_none()
            && let Some(data) = data
        {
            for field in ["camera", "environment", "light"] {
                collect_direct(
                    data.get(field).and_then(|n| n.get("p")),
                    field,
                    &mut channels,
                );
            }
            collect_direct(
                data.get("object")
                    .and_then(|o| o.get("material"))
                    .and_then(|m| m.get("p")),
                "m",
                &mut channels,
            );
        }
        let effects = array(layer.get("fx"))
            .iter()
            .filter_map(|fx| Some((string(fx.get("id"))?, string(fx.get("type"))?)))
            .collect();
        let missing_effects = array(layer.get("fx"))
            .iter()
            .filter(|fx| boolean(fx.get("missing")))
            .filter_map(|fx| string(fx.get("name")).or_else(|| string(fx.get("type"))))
            .collect();
        let asset = layer.get("d").and_then(|d| string(d.get("asset")));
        let audio = string(layer.get("type")).as_deref() == Some("audio");
        let transitions =
            [("transitionIn", "fadeIn"), ("transitionOut", "fadeOut")].map(|(field, fade)| {
                if audio {
                    return number(layer.get("d").and_then(|d| d.get(fade)))
                        .filter(|d| d.is_finite() && *d > 0.)
                        .map(|duration| Transition {
                            kind: "fade".into(),
                            duration,
                            missing: false,
                            params: vec![],
                        });
                }
                let t = layer.get(field)?;
                let kind = string(t.get("type"))?;
                let params = match t.get("p").map(Node::kind) {
                    Some(NodeKind::Object(fields)) => {
                        fields.iter().map(|(k, _)| k.to_string_lossy()).collect()
                    }
                    _ => vec![],
                };
                Some(Transition {
                    kind,
                    duration: number(t.get("dur")).filter(|d| d.is_finite())?.max(0.),
                    missing: boolean(t.get("missing")),
                    params,
                })
            });
        let masks = array(layer.get("masks"))
            .iter()
            .filter_map(|m| string(m.get("id")))
            .collect();
        let field = |key: &str| layer.get("d").and_then(|d| d.get(key));
        let channel_value = |n: Option<&Node>| {
            number(n).or_else(|| number(n.and_then(|n| n.get("v"))))
        };
        let gain_node = field("gain");
        let gain_animated = gain_node.is_some_and(|g| {
            matches!(g.get("kf").map(Node::kind), Some(NodeKind::Array(k)) if !k.is_empty())
                || string(g.get("expr")).is_some_and(|e| !e.is_empty())
        });
        let fade = |key: &str| channel_value(field(key)).filter(|f| *f > 0.).unwrap_or(0.);
        let mut read = Layer {
            gain: channel_value(gain_node).filter(|g| *g >= 0.).unwrap_or(1.),
            gain_animated,
            fade_in: fade("fadeIn"),
            fade_out: fade("fadeOut"),
            trim: number(layer.get("d").and_then(|d| d.get("trim")))
                .filter(|t| t.is_finite())
                .unwrap_or(0.)
                .max(0.),
            asset,
            missing_effects,
            transitions,
            has_authored_transition: ["transitionIn", "transitionOut"]
                .iter()
                .any(|field| matches!(layer.get(field).map(Node::kind), Some(NodeKind::Object(_)))),
            row_id: serde_json::to_string(&("layer", &id))
                .expect("row identifiers contain only strings"),
            id,
            name: js_string(layer.get("name")).unwrap_or_default(),
            kind: string(layer.get("type")).unwrap_or_default(),
            group: string(layer.get("group")),
            parent: string(layer.get("parent")),
            from,
            duration,
            locked: boolean(layer.get("lock")),
            effectively_locked: false,
            visible: match layer.get("on").map(Node::kind) {
                Some(NodeKind::Bool(b)) => *b,
                _ => !matches!(
                    layer.get("on").and_then(|n| n.get("v")).map(Node::kind),
                    Some(NodeKind::Bool(false))
                ),
            },
            solo: boolean(layer.get("solo")),
            shy: boolean(layer.get("shy")),
            three_d: boolean(layer.get("threeD"))
                || string(layer.get("type")).as_deref() == Some("camera")
                || string(content.and_then(|d| d.get("definition")))
                    .is_some_and(|def| def.starts_with("scene3d.")),
            audio_muted: boolean(layer.get("d").and_then(|n| n.get("audioMuted"))),
            collapsed: string(layer.get("type")).as_deref() == Some("group")
                || !matches!(
                    layer.get("collapsed").map(Node::kind),
                    Some(NodeKind::Bool(false))
                ),
            group_collapsed: !matches!(
                layer.get("collapsed").map(Node::kind),
                Some(NodeKind::Bool(false))
            ),
            scale_linked: boolean(layer.get("scaleLinked")),
            scale_unlinked: matches!(
                layer.get("scaleLinked").map(Node::kind),
                Some(NodeKind::Bool(false))
            ),
            separate_dimensions: !matches!(
                layer.get("separateDimensions").map(Node::kind),
                Some(NodeKind::Bool(false))
            ),
            position_separated: matches!(
                layer.get("separateDimensions").map(Node::kind),
                Some(NodeKind::Bool(true))
            ),
            effects,
            masks,
            comp: string(content.and_then(|d| d.get("comp"))),
            definition: string(content.and_then(|d| d.get("definition"))),
            preferred_track: number(layer.get("track")).unwrap_or(0.).max(0.).floor() as usize,
            channels,
            path_groups,
            channel_at: HashMap::new(),
            key_at: HashMap::new(),
            summary: Default::default(),
        };
        for (c, channel) in read.channels.iter().enumerate() {
            read.channel_at.insert(channel.path.clone(), c);
            for (k, key) in channel.keys.iter().enumerate() {
                read.key_at.insert(key.id.clone(), (c, k));
            }
        }
        read
    }
    /// Original ordinal for the v4 index column; O(1), independent of viewport.
    pub fn layer_ordinal(&self, id: &str) -> Option<usize> {
        self.by_id.get(id).copied()
    }
    pub fn layer(&self, id: &str) -> Option<&Layer> {
        self.by_id.get(id).map(|i| &*self.layers[*i])
    }
    pub fn has_locked_descendant(&self, group: &str) -> bool {
        self.layer_ordinal(group)
            .is_some_and(|i| self.locked_descendants[i])
    }

    /// The nearest explicit lock, including a lock inherited from a group.
    pub fn locking_layer(&self, id: &str) -> Option<&Layer> {
        let mut layer = self.layer(id)?;
        for _ in 0..self.layers.len() {
            if layer.locked {
                return Some(layer);
            }
            layer = self.layer(layer.group.as_deref()?)?;
        }
        None
    }
    pub fn channel(&self, layer: &str, channel: &str) -> Option<&Channel> {
        self.layer(layer)?.channel(channel)
    }
    /// The key `id` on layer `layer` (key ids may repeat across layers).
    pub fn key_on(&self, layer: &str, id: &str) -> Option<(&Layer, &Channel, &Key)> {
        let layer = self.layer(layer)?;
        let (c, k) = layer.key_at.get(id)?;
        Some((layer, &layer.channels[*c], &layer.channels[*c].keys[*k]))
    }
    /// Every layer's key `id`, in layer order.
    pub fn key_holders(&self, id: &str) -> Vec<(&Layer, &Channel, &Key)> {
        self.key_blocks
            .iter()
            .flat_map(|block| block.holders(id))
            .filter_map(|layer| self.key_on(&layer.id, id))
            .collect()
    }
    /// The selected keys, each on the layer it was picked on; a key selected
    /// by id alone is selected on every layer holding it.
    pub fn selected_keys(&self, selection: &pm_kernel::Selection) -> Vec<(&Layer, &Channel, &Key)> {
        let mut owners: HashMap<&str, Vec<&str>> = HashMap::new();
        for (layer, key) in selection.key_layers() {
            owners.entry(key).or_default().push(layer);
        }
        let mut selected = Vec::new();
        for id in selection.keys() {
            if let Some(layers) = owners.get(id.as_str()) {
                selected.extend(layers.iter().filter_map(|layer| self.key_on(layer, id)));
            } else {
                selected.extend(self.key_holders(id));
            }
        }
        selected
    }
    /// A key by id alone: its last layer. Keys picked on a row resolve with
    /// [`key_on`](Self::key_on); the selection with
    /// [`selected_keys`](Self::selected_keys).
    pub fn key(&self, id: &str) -> Option<(&Layer, &Channel, &Key)> {
        let layer = self
            .key_blocks
            .iter()
            // A key id two layers share resolves to the later layer.
            .rev()
            .find_map(|block| block.keys.get(id).map(|offset| &block.layers[*offset]))?;
        let (c, k) = layer.key_at.get(id)?;
        Some((layer, &layer.channels[*c], &layer.channels[*c].keys[*k]))
    }
    /// The nearest razor target can be found with a binary search. Equal
    /// distances prefer the playhead, then markers, then the original stack.
    pub(crate) fn split_targets(&self) -> &[(f64, usize)] {
        self.split_targets.get_or_init(|| {
            let markers = self.markers.iter().map(|marker| marker.time);
            let layers = self.layers.iter().flat_map(|layer| {
                let keys = layer
                    .channels
                    .iter()
                    .flat_map(|c| c.keys.iter().map(|k| layer.from + k.time));
                [layer.from, layer.from + layer.duration]
                    .into_iter()
                    .chain(keys)
            });
            let mut targets: Vec<_> = markers
                .chain(layers)
                .enumerate()
                .filter(|(_, time)| time.is_finite())
                .map(|(i, time)| (time, i + 1))
                .collect();
            targets.sort_by(|(a, _), (b, _)| a.partial_cmp(b).expect("finite time"));
            targets.dedup_by(|(a, _), (b, _)| *a == *b);
            targets
        })
    }

    pub fn marker_at(&self, time: f64) -> Option<&Marker> {
        self.markers
            .iter()
            .find(|m| (m.time - time).abs() < 0.5 / self.fps)
    }
    /// The latest time the timeline shows: the composition's end, or a
    /// layer or the work area running past it.
    pub fn maximum_time(&self) -> f64 {
        self.layer_time_bounds.1.max(self.duration).max(self.work[1])
    }
    pub fn minimum_time(&self) -> f64 {
        self.layer_time_bounds.0.min(self.work[0])
    }
    pub(crate) fn group_has_children(&self, id: &str) -> bool {
        self.layer_ordinal(id)
            .is_some_and(|i| !self.child_ordinals(i).is_empty())
    }

    pub(crate) fn child_ordinals(&self, ordinal: usize) -> &[usize] {
        &self.hierarchy.children[ordinal]
    }

    pub(crate) fn root_ordinals(&self) -> &[usize] {
        &self.hierarchy.roots
    }

    pub(crate) fn source_layers(&self) -> &[Node] {
        let comp = self
            .root()
            .and_then(|root| model::comp_node(root, self.comp_id.as_deref()));
        array(comp.and_then(|comp| comp.get("layers")))
    }

    pub(crate) fn source_ordinal(&self, id: &str) -> Option<usize> {
        self.layer_ordinal(id).map(|i| self.source_ordinals[i])
    }

    pub fn children(&self) -> HashMap<Option<String>, Vec<usize>> {
        let mut children: HashMap<Option<String>, Vec<usize>> = HashMap::new();
        for (i, l) in self.layers.iter().enumerate() {
            let group = l
                .group
                .as_ref()
                .filter(|id| id.as_str() != l.id && self.by_id.contains_key(*id))
                .cloned();
            children.entry(group).or_default().push(i);
        }
        children
    }
    pub fn roots(&self, children: &HashMap<Option<String>, Vec<usize>>) -> Vec<usize> {
        let mut roots = children.get(&None).cloned().unwrap_or_default();
        let mut seen = HashSet::new();
        let mut root_set: HashSet<_> = roots.iter().copied().collect();
        for candidate in roots.clone().into_iter().chain(0..self.layers.len()) {
            if seen.contains(&candidate) {
                continue;
            }
            if root_set.insert(candidate) {
                roots.push(candidate);
            }
            let mut todo = vec![candidate];
            while let Some(i) = todo.pop() {
                if seen.insert(i)
                    && let Some(kids) = children.get(&Some(self.layers[i].id.clone()))
                {
                    todo.extend(kids);
                }
            }
        }
        roots
    }
    /// Accumulate descendants once, from leaves to parents. On malformed
    /// cycles, each participant can reach the same remaining subtree.
    fn group_extents(&self) -> Vec<Option<(f64, f64)>> {
        let parent = |i: usize| self.hierarchy.parent[i].filter(|p| *p != i);
        let mut children = vec![0; self.layers.len()];
        for i in 0..self.layers.len() {
            if let Some(p) = parent(i) {
                children[p] += 1;
            }
        }
        let mut bounds: Vec<_> = self.layers.iter().map(|layer| {
            (layer.kind != "group").then_some((layer.from, layer.from + layer.duration))
        }).collect();
        let combine = |a: Option<(f64, f64)>, b: Option<(f64, f64)>| match (a, b) {
            (Some(a), Some(b)) => Some((a.0.min(b.0), a.1.max(b.1))),
            _ => a.or(b),
        };
        let mut leaves: Vec<_> = children.iter().enumerate().filter_map(|(i, &n)| (n == 0).then_some(i)).collect();
        while let Some(i) = leaves.pop() {
            if let Some(p) = parent(i) {
                bounds[p] = combine(bounds[p], bounds[i]);
                children[p] -= 1;
                if children[p] == 0 {
                    leaves.push(p);
                }
            }
        }
        for start in 0..children.len() {
            let mut cycle = Vec::new();
            let mut extent = None;
            let mut at = start;
            while children[at] != 0 {
                children[at] = 0;
                cycle.push(at);
                extent = combine(extent, bounds[at]);
                at = parent(at).expect("remaining layers have a parent in their cycle");
            }
            for i in cycle {
                bounds[i] = extent;
            }
        }
        bounds
    }

    /// Imported groups without saved timing display the extent of their members.
    pub fn strip_span(&self, layer: &Layer) -> (f64, f64) {
        if layer.kind != "group" || layer.duration > 0. {
            return (layer.from, layer.duration);
        }
        self.by_id.get(&layer.id)
            .and_then(|&i| self.group_extents.get(i).copied().flatten())
            .map_or((0., self.duration), |(start, end)| (start, end - start))
    }
    pub fn expanded_ids(&self, roots: &[String]) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut todo: Vec<_> = roots
            .iter()
            .filter_map(|id| self.layer_ordinal(id))
            .collect();
        while let Some(i) = todo.pop() {
            if seen.insert(i) {
                todo.extend(self.child_ordinals(i));
            }
        }
        let mut ordinals: Vec<_> = seen.into_iter().collect();
        ordinals.sort_unstable();
        ordinals
            .into_iter()
            .map(|i| self.layers[i].id.clone())
            .collect()
    }
}
fn collect_channels(node: Option<&Node>, prefix: &str, out: &mut Vec<Channel>) {
    let Some(node) = node else {
        return;
    };
    let mut todo = vec![(node, prefix.to_owned())];
    while let Some((node, path)) = todo.pop() {
        if node.get("kf").is_some() && node.get("v").is_some() {
            let keys = match node.get("kf").map(Node::kind) {
                Some(NodeKind::Array(keys)) => keys
                    .iter()
                    .filter_map(|key| {
                        Some(Key {
                            id: string(key.get("i"))?,
                            time: number(key.get("t"))?,
                            value: key.get("v").map(Node::to_value).unwrap_or(Value::Null),
                            stored: key.clone(),
                        })
                    })
                    .collect(),
                _ => Vec::new(),
            };
            out.push(Channel {
                path,
                value: node.get("v").unwrap().to_value(),
                expression: js_string(node.get("expr")),
                keys,
                path_group: None,
                vertex_label: None,
            });
        } else if let NodeKind::Object(fields) = node.kind() {
            for (key, child) in fields.iter().rev() {
                let key = key.to_string_lossy();
                let next = if path.is_empty() {
                    key
                } else {
                    format!("{path}.{key}")
                };
                todo.push((child, next));
            }
        }
    }
}
pub(crate) fn number(n: Option<&Node>) -> Option<f64> {
    match n?.kind() {
        NodeKind::Number(n) if n.is_finite() => Some(*n),
        _ => None,
    }
}
pub(crate) fn boolean(n: Option<&Node>) -> bool {
    matches!(n.map(Node::kind), Some(NodeKind::Bool(true)))
}
pub(crate) fn js_string(n: Option<&Node>) -> Option<JsString> {
    match n?.kind() {
        NodeKind::String(s) => Some(s.clone()),
        _ => None,
    }
}
pub(crate) fn as_str(n: &Node) -> Option<&str> {
    match n.kind() {
        NodeKind::String(s) => s.as_str(),
        _ => None,
    }
}
pub(crate) fn string(n: Option<&Node>) -> Option<String> {
    js_string(n).map(|s| s.to_string_lossy())
}

fn array(node: Option<&Node>) -> &[Node] {
    match node.map(Node::kind) {
        Some(NodeKind::Array(a)) => a,
        _ => &[],
    }
}
fn collect_direct(node: Option<&Node>, prefix: &str, out: &mut Vec<Channel>) {
    if let Some(NodeKind::Object(fields)) = node.map(Node::kind) {
        for (key, child) in fields.iter() {
            if child.get("kf").is_some() && child.get("v").is_some() {
                let path = if prefix.is_empty() {
                    key.to_string_lossy()
                } else {
                    format!("{prefix}.{}", key.to_string_lossy())
                };
                collect_channels(Some(child), &path, out);
            }
        }
    }
}
fn collect_path(
    path: &Node,
    prefix: &str,
    out: &mut Vec<Channel>,
    groups: &mut HashMap<String, String>,
) {
    let first = out.len();
    collect_direct(path.get("p"), prefix, out);
    for (index, vertex) in array(path.get("vertices")).iter().enumerate() {
        if let Some(id) = string(vertex.get("id")) {
            let vertex_prefix = format!("{prefix}.v.{id}");
            let first_vertex_channel = out.len();
            collect_direct(vertex.get("p"), &vertex_prefix, out);
            for channel in &mut out[first_vertex_channel..] {
                let key = &channel.path[vertex_prefix.len() + 1..];
                channel.vertex_label = Some(pm_eval::address::vertex_label(index + 1, key));
            }
        }
    }
    if first < out.len() {
        let name = string(path.get("name"))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Path".into());
        groups.insert(prefix.to_owned(), name);
        for channel in &mut out[first..] {
            channel.path_group = Some(prefix.to_owned());
        }
    }
}
