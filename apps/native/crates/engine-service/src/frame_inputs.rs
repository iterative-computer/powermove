//! What each frame of a composition is drawn from, as one digest per frame
//! (`crate::render_cache`'s keys): an edit changes only the digests of the
//! frames whose inputs it touched, so the disk render cache keeps every
//! other frame.
//!
//! A frame's inputs are, conservatively:
//!
//! - the composition's own fields and the root's (`Comp::member` falls back
//!   to the root), except bookkeeping no frame reads ([`BOOKKEEPING`]);
//! - in stacking order, every layer whose span meets the frame (widened by
//!   half the composition's shutter and a frame each side, for motion-blur
//!   samples), every layer those name anywhere in their subtree (parent,
//!   group, track matte, an effect's layer parameter), and every member of
//!   a group among them (its transitions time from all its members),
//!   transitively;
//! - every composition and asset those layers name, whole (a precomp's own
//!   layers, transitively).
//!
//! Anything that can couple a frame to inputs it does not name makes every
//! frame depend on the whole document ([`Inputs::Whole`]): a solo switch,
//! an expression whose code reads `comp` or calls `sample` or `motionPath`
//! (another layer's channels, whatever its span), one that does not compile
//! ([`Reads`], from the renderer's own expression engine), or (the caller's
//! `whole`) extension raster overrides, whose reads of the document are not
//! tracked. An expression reading `idx` adds its layer's index to the
//! frames it draws in. A key that missed an input would play a stale frame;
//! re-rendering is the lesser cost.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use pm_model::{Node, NodeKind};
use pm_time::Rate;

/// Root and composition fields that never change a frame: the edit log and
/// its counter, notes, the agent's memory, export defaults and the work area.
const BOOKKEEPING: [&str; 9] = [
    "revision",
    "edits",
    "created",
    "notes",
    "library",
    "memory",
    "memoryEnabled",
    "exportDefaults",
    "work",
];
/// Fields handled structurally (per layer, per named composition or asset).
const STRUCTURAL: [&str; 3] = ["layers", "comps", "assets"];

/// One digest per frame `0..count` of `comp` (`None`: no such composition).
pub(crate) fn frame_digests(
    doc: &pm_model::Document,
    comp: &str,
    rate: Rate,
    count: i64,
    whole: bool,
) -> Option<Vec<[u8; 32]>> {
    let resolved = pm_eval::Comp::resolve(doc, comp).ok()?;
    let (root, node) = (resolved.root(), resolved.node());
    let mut base = blake3::Hasher::new_derive_key("powermove frame inputs v1");
    base.update(comp.as_bytes());
    let skip: Vec<&str> = BOOKKEEPING.iter().chain(&STRUCTURAL).copied().collect();
    base.update(&fields_digest(node, &skip));
    if !std::ptr::eq(node, root) {
        base.update(&fields_digest(root, &skip));
    }
    let layers = resolved.layers();
    let seconds = |n: i64| {
        rate.frame_start(n).map_or(0., |t| {
            t as f64 / pm_protocol::common::TICKS_PER_SECOND as f64
        })
    };
    let frame = seconds(1) - seconds(0);
    // Motion-blur samples reach half the shutter each side; one frame more
    // for rounding.
    let margin = (pm_render::motion::shutter_frames(node) / 2. + 1.) * frame;
    let mut reads = Reads::default();
    if whole || matches!(inputs(layers, root, &mut reads), Inputs::Whole) {
        // Every frame reads every layer, composition and asset.
        base.update(b"whole");
        base.update(&fields_digest(root, &BOOKKEEPING));
        if !std::ptr::eq(node, root) {
            base.update(&fields_digest(node, &BOOKKEEPING));
        }
        let all = *base.finalize().as_bytes();
        return Some(vec![all; count.max(0) as usize]);
    }
    let index = Index::new(layers, root);
    let mut named = Named::new(root);
    let digests: Vec<[u8; 32]> = layers
        .iter()
        .map(pm_model::hash::canonical_digest)
        .collect();
    // `idx` is the layer's position among all the composition's layers:
    // a layer added, removed or moved before it changes what it draws.
    let indexed: Vec<bool> = layers.iter().map(|l| reads.of(l).index).collect();
    let mut out = Vec::with_capacity(count.max(0) as usize);
    let mut last: Option<(Vec<usize>, [u8; 32])> = None;
    for n in 0..count {
        let (from, to) = (seconds(n) - margin, seconds(n) + frame + margin);
        let drawn = index.closure(from, to);
        // Neighbouring frames usually draw the same layers.
        if let Some((same, digest)) = &last
            && *same == drawn
        {
            out.push(*digest);
            continue;
        }
        let mut hash = base.clone();
        let (mut comps, mut assets) = (BTreeSet::new(), BTreeSet::new());
        for &i in &drawn {
            hash.update(&digests[i]);
            if indexed[i] {
                hash.update(b"idx");
                hash.update(&(i as u64).to_le_bytes());
            }
            comps.extend(index.comps[i].iter().cloned());
            assets.extend(index.assets[i].iter().cloned());
        }
        for (id, digest) in named.comps(comps) {
            hash.update(id.as_bytes());
            hash.update(&digest);
        }
        assets.extend(named.assets_seen.iter().cloned());
        for id in &assets {
            if let Some(asset) = named.asset(id) {
                hash.update(id.as_bytes());
                hash.update(&asset);
            }
        }
        let digest = *hash.finalize().as_bytes();
        out.push(digest);
        last = Some((drawn, digest));
    }
    Some(out)
}

enum Inputs {
    /// Each frame reads the layers it names (see the module).
    Named,
    /// Every frame reads the whole document.
    Whole,
}

/// Whether the layers' frames can read layers they don't name.
fn inputs(layers: &[Node], root: &Node, reads: &mut Reads) -> Inputs {
    let solo = layers
        .iter()
        .any(|l| matches!(l.get("solo").map(Node::kind), Some(NodeKind::Bool(true))));
    let comps = root.get("comps").into_iter();
    if solo || layers.iter().chain(comps).any(|n| reads.of(n).others) {
        Inputs::Whole
    } else {
        Inputs::Named
    }
}

/// What the expressions of a subtree read beyond their own layer.
#[derive(Clone, Copy, Default)]
struct Reach {
    /// Other layers or compositions, or nothing that can be proved.
    others: bool,
    /// `idx`.
    index: bool,
}

/// Expression sources compiled once each, as the renderer compiles them
/// (`pm_render::expr::PmExprEngine`): what a frame reads is what the
/// program's code can reach, never a guess from its text.
#[derive(Default)]
struct Reads {
    sources: HashMap<pm_model::JsString, Reach>,
}
impl Reads {
    /// Every `expr` in `node`'s subtree. A string that does not compile, or
    /// any other value the evaluator takes for an expression (truthy), reads
    /// nothing that can be proved.
    fn of(&mut self, node: &Node) -> Reach {
        let mut reach = Reach::default();
        self.walk(node, &mut reach);
        reach
    }
    fn walk(&mut self, node: &Node, reach: &mut Reach) {
        match node.kind() {
            NodeKind::Object(map) => {
                for (key, value) in map.iter() {
                    if key.as_str() == Some("expr") {
                        let own = self.expression(value);
                        reach.others |= own.others;
                        reach.index |= own.index;
                    }
                    self.walk(value, reach);
                }
            }
            NodeKind::Array(items) => items.iter().for_each(|n| self.walk(n, reach)),
            _ => {}
        }
    }
    fn expression(&mut self, value: &Node) -> Reach {
        use pm_eval::ExpressionEngine as _;
        const UNKNOWN: Reach = Reach {
            others: true,
            index: true,
        };
        let NodeKind::String(source) = value.kind() else {
            return if pm_eval::node::truthy(Some(value)) {
                UNKNOWN
            } else {
                Reach::default()
            };
        };
        if let Some(known) = self.sources.get(source) {
            return *known;
        }
        let reach = if source.as_bytes().is_empty() {
            Reach::default()
        } else {
            pm_render::expr::PmExprEngine
                .compile(source)
                .map_or(UNKNOWN, |compiled| {
                    let deps = compiled.dependencies();
                    Reach {
                        others: deps.comp || deps.document,
                        index: deps.index || compiled.needs_idx(),
                    }
                })
        };
        self.sources.insert(source.clone(), reach);
        reach
    }
}

/// The digest of `node`'s members other than `skip`.
fn fields_digest(node: &Node, skip: &[&str]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    if let NodeKind::Object(map) = node.kind() {
        let mut members: Vec<(String, &Node)> = map
            .iter()
            .filter_map(|(k, v)| {
                let k = k.to_string_lossy();
                (!skip.contains(&k.as_str())).then_some((k, v))
            })
            .collect();
        members.sort_by(|a, b| a.0.cmp(&b.0));
        for (key, value) in members {
            hash.update(key.as_bytes());
            hash.update(&pm_model::hash::canonical_digest(value));
        }
    } else {
        hash.update(&pm_model::hash::canonical_digest(node));
    }
    *hash.finalize().as_bytes()
}

/// Every string in `node`'s subtree.
fn strings<'a>(node: &'a Node, out: &mut Vec<&'a pm_model::JsString>) {
    match node.kind() {
        NodeKind::String(s) => out.push(s),
        NodeKind::Array(items) => items.iter().for_each(|n| strings(n, out)),
        NodeKind::Object(map) => map.iter().for_each(|(_, n)| strings(n, out)),
        _ => {}
    }
}

/// The composition's layers: spans and what each names.
struct Index {
    /// `[from, to)` seconds a layer draws in (`None`: never directly, a
    /// group or an audio layer, drawn through what names it).
    spans: Vec<Option<(f64, f64)>>,
    layers: Vec<Vec<usize>>,
    /// A group's members: its transitions time from all of them.
    members: Vec<Vec<usize>>,
    comps: Vec<Vec<String>>,
    assets: Vec<Vec<String>>,
}
impl Index {
    fn new(layers: &[Node], root: &Node) -> Self {
        let by_id: HashMap<String, usize> = layers
            .iter()
            .enumerate()
            .filter_map(|(i, l)| Some((string(l.get("id")?)?, i)))
            .collect();
        let keys = |member: &str| -> BTreeSet<String> {
            match root.get(member).map(Node::kind) {
                Some(NodeKind::Object(map)) => map.keys().map(|k| k.to_string_lossy()).collect(),
                _ => BTreeSet::new(),
            }
        };
        let (comp_ids, asset_ids) = (keys("comps"), keys("assets"));
        let mut index = Index {
            spans: Vec::new(),
            layers: Vec::new(),
            members: vec![Vec::new(); layers.len()],
            comps: Vec::new(),
            assets: Vec::new(),
        };
        for (i, layer) in layers.iter().enumerate() {
            if let Some(&g) = layer
                .get("group")
                .and_then(string)
                .and_then(|g| by_id.get(&g))
                && g != i
            {
                index.members[g].push(i);
            }
        }
        for (i, layer) in layers.iter().enumerate() {
            let ty = layer.get("type").and_then(string);
            let number = |key: &str| match layer.get(key).map(Node::kind) {
                Some(NodeKind::Number(v)) if v.is_finite() => Some(*v),
                _ => None,
            };
            let span = match ty.as_deref() {
                Some("group" | "audio") => None,
                _ => {
                    let from = number("from").unwrap_or(f64::NEG_INFINITY);
                    let to = number("dur").map_or(f64::INFINITY, |d| from + d.max(0.));
                    Some((from, to))
                }
            };
            let mut found = Vec::new();
            strings(layer, &mut found);
            let (mut l, mut c, mut a) = (Vec::new(), Vec::new(), Vec::new());
            for s in found {
                let s = s.to_string_lossy();
                if let Some(&j) = by_id.get(&s)
                    && j != i
                {
                    l.push(j);
                }
                if comp_ids.contains(&s) {
                    c.push(s.clone());
                }
                if asset_ids.contains(&s) {
                    a.push(s);
                }
            }
            index.spans.push(span);
            index.layers.push(l);
            index.comps.push(c);
            index.assets.push(a);
        }
        index
    }

    /// The layers drawn in `[from, to)` and everything they name, in
    /// stacking order.
    fn closure(&self, from: f64, to: f64) -> Vec<usize> {
        let mut seen = vec![false; self.spans.len()];
        let mut stack: Vec<usize> = self
            .spans
            .iter()
            .enumerate()
            .filter(|(_, s)| s.is_some_and(|(a, b)| a < to && b > from))
            .map(|(i, _)| i)
            .collect();
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut seen[i], true) {
                continue;
            }
            stack.extend(
                self.layers[i]
                    .iter()
                    .chain(&self.members[i])
                    .filter(|&&j| !seen[j]),
            );
        }
        seen.iter()
            .enumerate()
            .filter(|(_, s)| **s)
            .map(|(i, _)| i)
            .collect()
    }
}

/// A named composition's digest, and the compositions and assets it names.
type NamedComp = ([u8; 32], Vec<String>, Vec<String>);

/// Compositions and assets by id, digested once, with what a named
/// composition names in turn.
struct Named<'a> {
    root: &'a Node,
    comps: HashMap<String, Option<NamedComp>>,
    assets: HashMap<String, Option<[u8; 32]>>,
    /// Assets named from inside named compositions.
    assets_seen: BTreeSet<String>,
}
impl<'a> Named<'a> {
    fn new(root: &'a Node) -> Self {
        Self {
            root,
            comps: HashMap::new(),
            assets: HashMap::new(),
            assets_seen: BTreeSet::new(),
        }
    }
    /// The named compositions and those they name, transitively, with
    /// their digests, ordered by id.
    fn comps(&mut self, named: BTreeSet<String>) -> BTreeMap<String, [u8; 32]> {
        self.assets_seen.clear();
        let mut out = BTreeMap::new();
        let mut stack: Vec<String> = named.into_iter().collect();
        while let Some(id) = stack.pop() {
            if out.contains_key(&id) {
                continue;
            }
            let Some((digest, comps, assets)) = self.comp(&id) else {
                continue;
            };
            out.insert(id, digest);
            stack.extend(comps.into_iter().filter(|c| !out.contains_key(c)));
            self.assets_seen.extend(assets);
        }
        out
    }
    fn comp(&mut self, id: &str) -> Option<NamedComp> {
        let root = self.root;
        self.comps
            .entry(id.to_owned())
            .or_insert_with(|| {
                let comps = root.get("comps")?;
                let node = comps.get(id)?;
                let mut found = Vec::new();
                strings(node, &mut found);
                let has = |member: &str, s: &str| root.get(member).and_then(|m| m.get(s)).is_some();
                let (mut c, mut a) = (Vec::new(), Vec::new());
                for s in found {
                    let s = s.to_string_lossy();
                    if s != id && has("comps", &s) {
                        c.push(s.clone());
                    }
                    if has("assets", &s) {
                        a.push(s);
                    }
                }
                Some((fields_digest(node, &BOOKKEEPING), c, a))
            })
            .clone()
    }
    fn asset(&mut self, id: &str) -> Option<[u8; 32]> {
        let root = self.root;
        *self.assets.entry(id.to_owned()).or_insert_with(|| {
            Some(pm_model::hash::canonical_digest(
                root.get("assets")?.get(id)?,
            ))
        })
    }
}

fn string(node: &Node) -> Option<String> {
    match node.kind() {
        NodeKind::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "Proot";
    fn project(layers: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "id": ROOT, "name": "p", "w": 640, "h": 360, "fps": 30, "dur": 4,
            "bg": "#000000", "revision": 1, "work": [0, 4], "markers": [],
            "assets": {"Aimg": {"name": "a.png", "type": "image", "w": 10, "h": 10}},
            "comps": {"Cpre": {"id": "Cpre", "name": "pre", "w": 100, "h": 100, "fps": 30, "dur": 4,
                "layers": [{"id": "Lin", "type": "image", "from": 0, "dur": 4, "d": {"asset": "Aimg"}}]}},
            "layers": layers,
        })
    }
    fn layer(id: &str, from: f64, dur: f64) -> serde_json::Value {
        serde_json::json!({"id": id, "type": "shape", "name": id, "from": from, "dur": dur,
            "p": {"opacity": {"v": 100}}})
    }
    fn digests(project: &serde_json::Value, whole: bool) -> Vec<[u8; 32]> {
        let parsed = pm_model::json::parse(&project.to_string()).unwrap();
        let doc = pm_model::Document::new(Some(parsed), 1);
        frame_digests(&doc, ROOT, Rate::FPS_30, 120, whole).unwrap()
    }
    /// Frames whose digest differs between two documents.
    fn changed(a: &serde_json::Value, b: &serde_json::Value) -> Vec<i64> {
        changed_with(a, b, false)
    }
    fn changed_with(a: &serde_json::Value, b: &serde_json::Value, whole: bool) -> Vec<i64> {
        let (a, b) = (digests(a, whole), digests(b, whole));
        (0..a.len() as i64)
            .filter(|&n| a[n as usize] != b[n as usize])
            .collect()
    }
    fn frames(range: std::ops::Range<i64>) -> Vec<i64> {
        range.collect()
    }

    /// Editing a layer changes the frames it draws in (and the motion-blur
    /// margin each side), nothing else; bookkeeping changes nothing at all.
    #[test]
    fn an_edit_changes_only_the_frames_its_layer_draws() {
        let a = project(serde_json::json!([
            layer("L1", 0., 1.),
            layer("L2", 2., 1.)
        ]));
        let mut b = a.clone();
        b["layers"][1]["p"]["opacity"]["v"] = 50.into();
        assert_eq!(changed(&a, &b), frames(58..92));
        let mut c = a.clone();
        c["revision"] = 2.into();
        c["work"] = serde_json::json!([1, 2]);
        c["edits"] = serde_json::json!([{"by": "agent"}]);
        assert!(changed(&a, &c).is_empty());
        // Moving a layer changes where it was and where it is now.
        let mut d = a.clone();
        d["layers"][0]["from"] = 3.into();
        let moved = changed(&a, &d);
        assert!(moved.contains(&0) && moved.contains(&100) && !moved.contains(&70));
    }

    /// What a drawn layer names is part of its frames: its parent (drawn
    /// or not), a precomp and its assets.
    #[test]
    fn named_layers_compositions_and_assets_are_inputs() {
        let mut child = layer("L1", 0., 1.);
        child["parent"] = "Lnull".into();
        let mut parent = layer("Lnull", 3., 1.);
        parent["type"] = "null".into();
        let mut pre = layer("Lpre", 2., 1.);
        pre["type"] = "comp".into();
        pre["d"] = serde_json::json!({"comp": "Cpre"});
        let a = project(serde_json::json!([child, parent, pre]));
        let mut b = a.clone();
        b["layers"][1]["p"]["opacity"]["v"] = 20.into();
        // The parent's own span (3-4 s) and its child's (0-1 s).
        let parent = changed(&a, &b);
        assert!(parent.contains(&0) && parent.contains(&100) && !parent.contains(&70));
        let mut c = a.clone();
        c["comps"]["Cpre"]["layers"][0]["from"] = 1.into();
        assert_eq!(changed(&a, &c), frames(58..92));
        let mut d = a.clone();
        d["assets"]["Aimg"]["name"] = "b.png".into();
        assert_eq!(changed(&a, &d), frames(58..92));
    }

    /// Composition fields change every frame; a solo switch or an
    /// expression addressing other layers makes every edit change every
    /// frame.
    #[test]
    fn couplings_beyond_names_change_every_frame() {
        let a = project(serde_json::json!([
            layer("L1", 0., 1.),
            layer("L2", 2., 1.)
        ]));
        let mut b = a.clone();
        b["bg"] = "#ffffff".into();
        assert_eq!(changed(&a, &b).len(), 120);
        let mut expr = a.clone();
        expr["layers"][0]["p"]["opacity"]["expr"] = "comp.layers[1].p.opacity.v".into();
        let mut edited = expr.clone();
        edited["layers"][1]["p"]["opacity"]["v"] = 10.into();
        assert_eq!(changed(&expr, &edited).len(), 120);
        // An expression that does not compile proves nothing.
        let mut broken = a.clone();
        broken["layers"][0]["p"]["opacity"]["expr"] = "value +".into();
        let mut edited = broken.clone();
        edited["layers"][1]["p"]["opacity"]["v"] = 10.into();
        assert_eq!(changed(&broken, &edited).len(), 120);
        // One reading its own layer and the composition's parameters keeps
        // edits local (the layer and the composition's fields are inputs).
        let mut own = a.clone();
        own["layers"][0]["p"]["opacity"]["expr"] = "layer.from + param('speed') * t".into();
        let mut edited = own.clone();
        edited["layers"][1]["p"]["opacity"]["v"] = 10.into();
        assert_eq!(changed(&own, &edited), frames(58..92));
        // A time-only expression keeps edits local.
        let mut wiggle = a.clone();
        wiggle["layers"][0]["p"]["opacity"]["expr"] = "wiggle(2, 10)".into();
        let mut edited = wiggle.clone();
        edited["layers"][1]["p"]["opacity"]["v"] = 10.into();
        assert_eq!(changed(&wiggle, &edited), frames(58..92));
        let mut solo = a.clone();
        solo["layers"][0]["solo"] = true.into();
        let mut edited = solo.clone();
        edited["layers"][1]["p"]["opacity"]["v"] = 10.into();
        assert_eq!(changed(&solo, &edited).len(), 120);
    }

    /// `sample` and `motionPath` read another layer's channels whatever its
    /// span, and name it inside the expression's text: an edit to a layer
    /// that is not drawn at a frame changes the frame that samples it.
    #[test]
    fn a_sampled_layer_changes_the_frames_that_sample_it() {
        for source in [
            "sample('L2', 'opacity', 0)",
            "t > 9 ? 0 : motionPath('L2', 'position.x', 0.5, 0, 1)",
        ] {
            let mut a = project(serde_json::json!([
                layer("L1", 0., 1.),
                layer("L2", 3., 1.)
            ]));
            a["layers"][0]["p"]["opacity"]["expr"] = source.into();
            let mut b = a.clone();
            b["layers"][1]["p"]["opacity"]["v"] = 10.into();
            assert!(changed(&a, &b).contains(&0), "{source}");
        }
    }

    /// `idx` is the layer's position among all layers: one added before it,
    /// drawn at the frame or not, changes the frames it draws in, and only
    /// those of layers that read it.
    #[test]
    fn a_layer_index_is_an_input_where_idx_is_read() {
        let mut a = project(serde_json::json!([
            layer("L1", 0., 1.),
            layer("L2", 2., 1.)
        ]));
        a["layers"][0]["p"]["opacity"]["expr"] = "idx * 100".into();
        let mut b = a.clone();
        b["layers"]
            .as_array_mut()
            .unwrap()
            .insert(0, layer("L0", 3., 1.));
        let moved = changed(&a, &b);
        assert!(moved.contains(&0), "L1's index changed: {moved:?}");
        assert!(!moved.contains(&70), "L2 reads no index: {moved:?}");
    }

    /// A group's transitions time from all its members: moving a member
    /// that isn't drawn at a frame still changes the frames its siblings
    /// draw in.
    #[test]
    fn a_group_member_changes_its_siblings_frames() {
        let mut group = layer("Lgroup", 0., 4.);
        group["type"] = "group".into();
        group["transitionIn"] = serde_json::json!({"type": "fade", "duration": 0.5});
        let mut first = layer("L1", 0., 1.);
        first["group"] = "Lgroup".into();
        let mut second = layer("L2", 2., 1.);
        second["group"] = "Lgroup".into();
        let a = project(serde_json::json!([group, first, second]));
        let mut b = a.clone();
        b["layers"][2]["from"] = 2.5.into();
        assert!(changed(&a, &b).contains(&10), "frame 10 draws L1 only");
    }

    /// With raster overrides, whose reads of the document aren't tracked,
    /// any edit changes every frame.
    #[test]
    fn overrides_key_every_frame_on_the_whole_document() {
        let a = project(serde_json::json!([
            layer("L1", 0., 1.),
            layer("L2", 2., 1.)
        ]));
        let mut b = a.clone();
        b["layers"][1]["p"]["opacity"]["v"] = 50.into();
        assert_eq!(changed_with(&a, &b, true).len(), 120);
        assert_eq!(changed_with(&a, &b, false), frames(58..92));
    }

    /// The margin follows the shutter: a two-frame shutter half reaches
    /// further than one frame each side.
    #[test]
    fn the_motion_margin_follows_the_shutter() {
        let mut a = project(serde_json::json!([
            layer("L1", 0., 1.),
            layer("L2", 2., 1.)
        ]));
        a["shutter"] = 4.into();
        let mut b = a.clone();
        b["layers"][1]["p"]["opacity"]["v"] = 50.into();
        assert_eq!(changed(&a, &b), frames(57..93));
    }
}
