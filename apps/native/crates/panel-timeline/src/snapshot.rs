use pm_kernel::{Snapshot, model};
use pm_model::{JsString, Node, NodeKind, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock, Weak};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RowId {
    Layer(String),
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
    /// Complete stored key, including unknown fields, for lossless clipboard.
    pub stored: Value,
}
#[derive(Clone, Debug)]
pub struct Channel {
    pub path: String,
    pub value: Value,
    pub expression: Option<JsString>,
    pub keys: Vec<Key>,
}
#[derive(Clone, Debug)]
pub struct Layer {
    pub id: String,
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
    pub three_d: bool,
    pub audio_muted: bool,
    pub collapsed: bool,
    pub group_collapsed: bool,
    pub preferred_track: usize,
    pub scale_linked: bool,
    pub separate_dimensions: bool,
    pub effects: Vec<(String, String)>,
    /// The media asset an image, video or audio layer draws (`d.asset`).
    pub asset: Option<String>,
    /// Seconds of the media skipped before the layer starts (`d.trim`).
    pub trim: f64,
    /// Effects whose extension is missing (`fx[].missing`): the layer draws
    /// with them bypassed. Their display names, else types.
    pub missing_effects: Vec<String>,
    pub masks: Vec<String>,
    pub definition: Option<String>,
    pub channels: Vec<Channel>,
    /// Revision at which the layer's tree node last changed: an unchanged
    /// layer is reused, not read again, by the next index.
    rev: u64,
    channel_at: HashMap<String, usize>,
    key_at: HashMap<String, (usize, usize)>,
    /// The collapsed row's key marks, sorted, none selected: built once per
    /// read of the layer and shared by every scene until the layer changes.
    summary: std::sync::OnceLock<Arc<[crate::render::Key]>>,
}
impl Layer {
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
    by_id: HashMap<String, usize>,
    /// Key ids by layer position, [`KEY_BLOCK`] layers a block. A block whose
    /// layers are all reused is reused, so an edit re-indexes one block.
    key_blocks: Vec<Arc<KeyBlock>>,
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
            by_id: HashMap::new(),
            key_blocks: Vec::new(),
        }
    }
}
impl TimelineIndex {
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
        for layer in model::layers_of(comp) {
            let reused = same
                .then(|| string(layer.get("id")))
                .flatten()
                .and_then(|id| self.layer_arc(&id))
                .filter(|l| l.rev == layer.rev());
            match reused {
                Some(l) => result.layers.push(l.clone()),
                None => {
                    if let Some(read) = Self::read_layer(layer) {
                        result.layers.push(Arc::new(read));
                    }
                }
            }
        }
        result.by_id = result
            .layers
            .iter()
            .enumerate()
            .map(|(i, l)| (l.id.clone(), i))
            .collect();
        // Memoised group locks, bounded even on corrupt/cyclic input. Transform parents
        // do not imply group locks (kernel's lock policy).
        let mut locked = vec![None; result.layers.len()];
        for first in 0..result.layers.len() {
            let mut chain = Vec::new();
            let mut seen = HashSet::new();
            let mut at = Some(first);
            let mut inherited = false;
            while let Some(i) = at {
                if let Some(value) = locked[i] {
                    inherited |= value;
                    break;
                }
                if !seen.insert(i) {
                    inherited |= chain.iter().any(|i: &usize| result.layers[*i].locked);
                    break;
                }
                chain.push(i);
                at = result.layers[i]
                    .group
                    .as_ref()
                    .and_then(|id| result.by_id.get(id))
                    .copied();
            }
            for i in chain.into_iter().rev() {
                inherited |= result.layers[i].locked;
                locked[i] = Some(inherited);
            }
        }
        for (i, l) in result.layers.iter_mut().enumerate() {
            let effective = locked[i].unwrap_or(l.locked);
            if l.effectively_locked != effective {
                Arc::make_mut(l).effectively_locked = effective;
            }
        }
        let previous: HashMap<*const Layer, &Arc<KeyBlock>> = self
            .key_blocks
            .iter()
            .filter_map(|b| Some((Arc::as_ptr(b.layers.first()?), b)))
            .collect();
        result.key_blocks = result
            .layers
            .chunks(KEY_BLOCK)
            .map(|chunk| match previous.get(&Arc::as_ptr(&chunk[0])) {
                Some(block) if block.holds(chunk) => Arc::clone(block),
                _ => Arc::new(KeyBlock::new(chunk)),
            })
            .collect();
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
    /// One stored layer as the timeline reads it.
    fn read_layer(layer: &Node) -> Option<Layer> {
        let id = string(layer.get("id"))?;
        let mut channels = Vec::new();
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
                    collect_path(mask.get("path"), &format!("mp.{id}"), &mut channels);
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
        for path in array(content.and_then(|d| d.get("paths"))) {
            if let Some(id) = string(path.get("id")) {
                collect_path(Some(path), &format!("g.{id}"), &mut channels);
            }
        }
        for (field, prefix) in [("animators", "ta"), ("styles", "ts")] {
            for item in array(content.and_then(|d| d.get(field))) {
                if let Some(id) = string(item.get("id")) {
                    collect_direct(item.get("p"), &format!("{prefix}.{id}"), &mut channels);
                }
            }
        }
        let scene = content
            .and_then(|d| d.get("data"))
            .and_then(|d| d.get("scene"));
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
        let masks = array(layer.get("masks"))
            .iter()
            .filter_map(|m| string(m.get("id")))
            .collect();
        let mut read = Layer {
            trim: number(layer.get("d").and_then(|d| d.get("trim")))
                .filter(|t| t.is_finite())
                .unwrap_or(0.)
                .max(0.),
            asset,
            missing_effects,
            id,
            name: js_string(layer.get("name")).unwrap_or_default(),
            kind: string(layer.get("type")).unwrap_or_default(),
            group: string(layer.get("group")),
            parent: string(layer.get("parent")),
            from: number(layer.get("from")).unwrap_or(0.),
            duration: number(layer.get("dur")).unwrap_or(0.),
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
            three_d: boolean(layer.get("threeD")),
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
            separate_dimensions: !matches!(
                layer.get("separateDimensions").map(Node::kind),
                Some(NodeKind::Bool(false))
            ),
            effects,
            masks,
            definition: string(content.and_then(|d| d.get("definition"))),
            preferred_track: number(layer.get("track")).unwrap_or(0.).max(0.).floor() as usize,
            channels,
            rev: layer.rev(),
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
        Some(read)
    }
    fn layer_arc(&self, id: &str) -> Option<&Arc<Layer>> {
        self.by_id.get(id).map(|i| &self.layers[*i])
    }
    /// Original ordinal for the v4 index column; O(1), independent of viewport.
    pub fn layer_ordinal(&self, id: &str) -> Option<usize> {
        self.by_id.get(id).copied()
    }
    pub fn layer(&self, id: &str) -> Option<&Layer> {
        self.by_id.get(id).map(|i| &*self.layers[*i])
    }
    pub fn channel(&self, layer: &str, channel: &str) -> Option<&Channel> {
        let layer = self.layer(layer)?;
        layer.channel_at.get(channel).map(|c| &layer.channels[*c])
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
        selection
            .keys()
            .iter()
            .flat_map(|id| match selection.key_owners(id) {
                Some(owners) => owners
                    .into_iter()
                    .filter_map(|layer| self.key_on(layer, id))
                    .collect(),
                None => self.key_holders(id),
            })
            .collect()
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
    pub fn marker_at(&self, time: f64) -> Option<&Marker> {
        self.markers
            .iter()
            .find(|m| (m.time - time).abs() < 0.5 / self.fps)
    }
    pub fn minimum_time(&self) -> f64 {
        self.layers
            .iter()
            .map(|l| l.from)
            .fold(0., f64::min)
            .min(self.work[0])
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
    pub fn expanded_ids(&self, roots: &[String]) -> Vec<String> {
        let children = self.children();
        let mut seen = HashSet::new();
        let mut todo: Vec<_> = roots
            .iter()
            .filter_map(|id| self.by_id.get(id).copied())
            .collect();
        while let Some(i) = todo.pop() {
            if seen.insert(i)
                && let Some(kids) = children.get(&Some(self.layers[i].id.clone()))
            {
                todo.extend(kids);
            }
        }
        self.layers
            .iter()
            .enumerate()
            .filter(|(i, _)| seen.contains(i))
            .map(|(_, l)| l.id.clone())
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
                            stored: key.to_value(),
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
fn collect_path(path: Option<&Node>, prefix: &str, out: &mut Vec<Channel>) {
    collect_direct(path.and_then(|p| p.get("p")), prefix, out);
    for vertex in array(path.and_then(|p| p.get("vertices"))) {
        if let Some(id) = string(vertex.get("id")) {
            collect_direct(vertex.get("p"), &format!("{prefix}.v.{id}"), out);
        }
    }
}
