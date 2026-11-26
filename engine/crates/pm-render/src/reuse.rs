//! Layer draws reused across viewer frames
//! (docs/engine/perf.md, "Draw reuse").
//!
//! A gesture (a layer drag, an inspector scrub while paused) makes a new
//! revision every pointer move, and each one is planned again: on a
//! 500-layer composition the plan was about 4 ms of the frame, nearly all
//! of it evaluating layers the gesture did not touch. The interactive
//! renderer keeps the draws a layer produced at the last frame and gives
//! them back while nothing they were computed from changed. A reused draw
//! is the value the planner returned for the same inputs, so the plan, and
//! every pixel drawn from it, is the one a fresh build makes.
//!
//! # What a layer's draws depend on
//!
//! Cached layers must produce self-contained native draws. Their transforms
//! and activity can depend on parents and enclosing groups, and a 2.5D
//! pass also depends on its camera. Groups additionally depend on their
//! members in stack order, including members that produced no draw.
//!
//! Snapshot edits can copy the layer list. Retain the layer's object
//! identity, which survives that copy, and check its revision as well.
//! Shared identity prevents an unrelated document or a same-revision
//! replacement from matching an old layer. Non-layer composition fields
//! and build options are checked independently.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use pm_eval::{JsResult, Scope, node};
use pm_model::{JsString, Node, NodeKind};

use crate::plan::{Build, Draw, Quad};

/// Counters of the reuse (tests and `compose_bench`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReuseStats {
    /// Layers whose draws were given back.
    pub reused: u64,
    /// Layers planned with the reuse active (a miss or a layer it does
    /// not keep).
    pub planned: u64,
    /// Entries stored.
    pub stored: u64,
    /// Builds that did not use the reuse (a new document or exact time).
    pub cold_builds: u64,
    /// Frames drawn from the kept bottom of an earlier frame
    /// (`crate::frame_prefix`), and the top-level draws they did not draw
    /// again.
    pub prefix_frames: u64,
    pub prefix_draws: u64,
    /// Offscreen group passes loaded from a preceding viewer frame.
    pub group_hits: u64,
}

/// The build options a layer's draws depend on.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BuildKey {
    pub sampling: crate::sampling::Sampling,
    pub blur: Option<crate::motion::Blur>,
    pub view: Option<crate::scene3d::View3d>,
    pub text: Option<u64>,
    pub preview: Option<crate::compat_crop::PreviewWindow>,
    pub keep_empty_groups: bool,
    pub offline_cards: bool,
    pub studio: bool,
    /// The output origin (bits): every draw of the top level and its
    /// groups moves with it.
    pub origin: Option<(u64, u64)>,
    /// Final output plans levels at the composition's density where their
    /// effects need it (`crate::plan::comp_pass`).
    pub final_output: bool,
    /// Playing frames name the files a sequence reads next.
    pub read_ahead: bool,
    /// Interactive viewer group focus (`RenderRequest::focus_group`).
    pub focus_group: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PassKey {
    width: u32,
    height: u32,
    scale: (u64, u64),
    precomp_depth: u32,
    at_root: bool,
}

struct Entry {
    draws: Option<Vec<Draw>>,
    /// This pass's local time can differ from the root playhead in a precomp.
    time: f64,
    /// All inputs are independent of time except each layer's activity span.
    static_time: bool,
    /// The closure: layer id and its shared object.
    closure: Vec<(JsString, Node)>,
    /// Group membership and layer dependencies, including inactive children.
    group: Option<Arc<crate::pass_facts::PassFacts>>,
    camera: Option<pm_eval::space3d::Camera3d>,
    shaders: Vec<pm_glsl::Key>,
    /// An image's asset and the path its resolver answered.
    media: Option<(String, PathBuf)>,
    /// The build that last used the entry.
    used: u64,
}

/// The reuse state a renderer keeps across frames.
#[derive(Default)]
pub struct DrawReuse {
    project: Option<JsString>,
    build: Option<BuildKey>,
    /// The revision the entries were validated at.
    stamp: u64,
    /// The time of the last build (bits).
    time: Option<u64>,
    /// Entries by composition id and pass, then by layer id.
    passes: HashMap<(JsString, PassKey), HashMap<JsString, Entry>>,
    /// Builds so far (an entry's `used`).
    builds: u64,
    /// Whether this build looks up and stores.
    active: bool,
    /// The revision of this build.
    revision: u64,
    /// Compositions whose fields were found unchanged in this build, by
    /// composition node address.
    fields_unchanged: HashMap<usize, bool>,
    fields: HashMap<(JsString, bool), Fields>,
    next_fields: HashMap<(JsString, bool), Fields>,
    stats: ReuseStats,
    layer_traits: HashMap<(JsString, bool, JsString), LayerTraits>,
    facts: crate::pass_facts::FactsCache,
    pub(crate) geometry: FrameGeometry,
    pub(crate) geometry_active: bool,
}

struct LayerTraits {
    node: Node,
    has_expression: bool,
    static_time: bool,
}

/// Only contains camera-independent work for one immutable document and time.
#[derive(Default)]
pub(crate) struct FrameGeometry {
    root: Option<Node>,
    facts: HashMap<usize, Arc<crate::pass_facts::PassFacts>>,
    memberships: HashMap<(ScopeKey, usize, bool), Box<[usize]>>,
    orders: HashMap<(ScopeKey, bool, Vec<usize>), pm_eval::space3d::PlaneOrder>,
    pub(crate) quads: HashMap<(ScopeKey, usize, [u64; 6]), LayerGeometry>,
}

pub(crate) type ScopeKey = (usize, usize, u64);

pub(crate) fn scope_key(scope: &Scope<'_, '_>, time: f64) -> ScopeKey {
    (
        address(scope.comp.node()),
        address(scope.project.node()),
        time.to_bits(),
    )
}

pub(crate) fn address(node: &Node) -> usize {
    std::ptr::from_ref(node) as usize
}

pub(crate) struct LayerGeometry {
    pub quad: Quad,
    pub affine: pm_eval::transform::Mat2d,
    pub world: pm_eval::space3d::Mat4,
}

impl FrameGeometry {
    pub(crate) fn membership<'d>(
        &mut self,
        scope: &mut Scope<'_, 'd>,
        parent: Option<&Node>,
        time: f64,
        blur: bool,
        compute: impl FnOnce(&mut Scope<'_, 'd>) -> JsResult<Vec<&'d Node>>,
    ) -> JsResult<Vec<&'d Node>> {
        let key = (scope_key(scope, time), parent.map_or(0, address), blur);
        if let Some(indices) = self.memberships.get(&key) {
            return Ok(indices.iter().map(|&i| &scope.layers()[i]).collect());
        }
        let pass = compute(scope)?;
        if let Some(members) = pass
            .iter()
            .map(|layer| scope.comp.layer_position(layer))
            .collect::<Option<Vec<_>>>()
        {
            self.memberships.insert(key, members.into());
        }
        Ok(pass)
    }

    pub(crate) fn order<'d>(
        &mut self,
        scope: &mut Scope<'_, 'd>,
        layers: &[&'d Node],
        time: f64,
        skip_inactive: bool,
    ) -> JsResult<Vec<&'d Node>> {
        let key = (
            scope_key(scope, time),
            skip_inactive,
            layers.iter().map(|l| address(l)).collect(),
        );
        let order = match self.orders.entry(key) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(scope.prepare_plane_order(layers, time, skip_inactive)?)
            }
        };
        order.project(scope, layers, time)
    }
}

impl DrawReuse {
    pub fn new() -> DrawReuse {
        DrawReuse::default()
    }

    pub(crate) fn active(&self) -> bool {
        self.active
    }

    pub fn stats(&self) -> ReuseStats {
        self.stats
    }

    /// Drops every entry (the next build at the same time stores again).
    pub fn clear(&mut self) {
        self.passes.clear();
        self.geometry = FrameGeometry::default();
        self.geometry_active = false;
        self.facts.clear();
        self.layer_traits.clear();
        self.project = None;
        self.fields.clear();
        self.next_fields.clear();
        self.build = None;
        self.time = None;
    }

    /// Starts a build of `root` at revision `revision`, time `time`.
    pub(crate) fn begin(
        &mut self,
        root: &Node,
        revision: u64,
        time: f64,
        build: BuildKey,
        camera_preview: bool,
    ) {
        self.builds += 1;
        self.fields_unchanged.clear();
        self.next_fields.clear();
        let project = node::string(node::member(Some(root), "id")).cloned();
        let same_doc = project.is_some() && project == self.project && revision >= self.stamp;
        if !same_doc {
            self.facts.clear();
            self.fields.clear();
            self.layer_traits.clear();
        }
        let time = time.to_bits();
        let same_time = same_doc && self.time == Some(time);
        // Frame geometry holds the immutable root so the stored layer
        // addresses stay valid. Retain it only for an unchanged snapshot.
        let same_snapshot = same_time
            && revision == self.stamp
            && self
                .geometry
                .root
                .as_ref()
                .is_some_and(|old| same_field(old, root));
        let same_build = self.build.as_ref() == Some(&build);
        let reusable_time = same_time
            || (!build.final_output
                && (build.read_ahead || build.sampling == crate::sampling::Sampling::Playback));
        if !same_doc || !same_build || !reusable_time {
            self.passes.clear();
        }
        if !same_time || !same_build {
            self.geometry = FrameGeometry::default();
        }
        if !same_snapshot || self.geometry.root.is_none() {
            self.geometry = FrameGeometry {
                root: Some(root.clone()),
                ..Default::default()
            };
        }
        // Moving the viewer can use the first pose's geometry immediately
        // on the next frame. Other cold builds can move to another time.
        self.geometry_active = same_snapshot || camera_preview;
        self.active = same_doc && reusable_time;
        if !self.active {
            self.stats.cold_builds += 1;
        }
        self.project = project;
        self.build = Some(build);
        self.time = Some(time);
        self.revision = revision;
    }

    /// Ends a build: entries it did not use go, the rest are valid at its
    /// revision.
    pub(crate) fn end(&mut self) {
        if self.active {
            let build = self.builds;
            for entries in self.passes.values_mut() {
                entries.retain(|_, e| e.used == build);
            }
            self.passes.retain(|_, entries| !entries.is_empty());
        } else {
            self.passes.clear();
        }
        std::mem::swap(&mut self.fields, &mut self.next_fields);
        self.next_fields.clear();
        self.stamp = self.revision;
        self.active = false;
    }

    pub(crate) fn pass_facts(
        &mut self,
        comp: &pm_eval::Comp<'_>,
        all: &[&Node],
    ) -> std::sync::Arc<crate::pass_facts::PassFacts> {
        if self.geometry_active {
            return self.geometry.facts.entry(address(comp.node()))
                .or_insert_with(|| self.facts.get(comp, all, self.revision))
                .clone();
        }
        self.facts.get(comp, all, self.revision)
    }

    fn layer_traits(&mut self, comp: &pm_eval::Comp<'_>, id: &JsString, node: &Node) -> &LayerTraits {
        const MAX_LAYERS: usize = 4096;
        let key = (comp.id().clone(), comp.at_root(), id.clone());
        if self.layer_traits.len() >= MAX_LAYERS && !self.layer_traits.contains_key(&key) {
            self.layer_traits.clear();
        }
        self.layer_traits
            .entry(key)
            .and_modify(|traits| {
                if !same_field(node, &traits.node) || traits.node.rev() != node.rev() {
                    *traits = LayerTraits::of(node);
                }
            })
            .or_insert_with(|| LayerTraits::of(node))
    }

    /// Whether `comp`'s own fields (and the root's, and the open
    /// composition's) are unchanged since the stamp, the layer lists and
    /// the project's provenance aside.
    fn fields_unchanged(&mut self, scope: &Scope<'_, '_>) -> bool {
        let mut check = |n: &Node, id: &JsString, root: bool| {
            let address = std::ptr::from_ref(n) as usize;
            if let Some(&ok) = self.fields_unchanged.get(&address) {
                return ok;
            }
            let key = (id.clone(), root);
            let fields = Fields::of(n);
            let ok = self.fields.get(&key).is_some_and(|old| {
                fields.0.len() == old.0.len()
                    && fields.0.iter().zip(&old.0).all(|((k, v), (old_k, old_v))| {
                        k == old_k && v.rev() <= self.stamp && same_field(v, old_v)
                    })
            });
            self.next_fields.insert(key, fields);
            self.fields_unchanged.insert(address, ok);
            ok
        };
        let comp = check(scope.comp.node(), scope.comp.id(), scope.comp.at_root());
        let project = check(
            scope.project.node(),
            scope.project.id(),
            scope.project.at_root(),
        );
        let root = check(scope.comp.root(), scope.project.id(), true);
        comp && project && root
    }
}

struct Fields(Vec<(JsString, Node)>);

impl Fields {
    fn of(n: &Node) -> Self {
        // Layer lists are validated by the entry. The editing provenance
        // is updated on every edit and is not a rendering input.
        const SKIP: &[&[u8]] = &[b"layers", b"comps", b"revision", b"edits"];
        Self(match n.kind() {
            NodeKind::Object(map) => map
                .iter()
                .filter(|(k, _)| !SKIP.contains(&k.as_bytes()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            _ => Vec::new(),
        })
    }
}

fn same_field(current: &Node, old: &Node) -> bool {
    match (current.kind(), old.kind()) {
        (NodeKind::Object(a), NodeKind::Object(b)) => Arc::ptr_eq(a, b),
        (NodeKind::Array(a), NodeKind::Array(b)) => Arc::ptr_eq(a, b),
        (a, b) => a == b,
    }
}

fn unchanged(current: &Node, old: &Node, stamp: u64) -> bool {
    current.rev() <= stamp
        && matches!((current.kind(), old.kind()),
            (NodeKind::Object(a), NodeKind::Object(b)) if Arc::ptr_eq(a, b))
}

/// The layers `layer`'s draws read besides itself: parents and enclosing
/// groups, transitively; the layer first. `None` when a lookup throws or
/// the closure is unusually large.
fn closure<'d>(scope: &mut Scope<'_, 'd>, layer: &'d Node) -> Option<Vec<&'d Node>> {
    let mut out: Vec<&'d Node> = vec![layer];
    let mut i = 0;
    while i < out.len() {
        if out.len() > 64 {
            return None;
        }
        let x = out[i];
        if let Some(parent) = scope.parent_of(x).ok()?
            && !out.iter().any(|y| std::ptr::eq(*y, parent))
        {
            out.push(parent);
        }
        for group in scope.group_ancestors(x).ok()? {
            if !out.iter().any(|y| std::ptr::eq(*y, group)) {
                out.push(group);
            }
        }
        i += 1;
    }
    Some(out)
}

/// Whether a subtree holds an expression (a truthy `expr` member anywhere).
fn has_expression(n: &Node) -> bool {
    match n.kind() {
        NodeKind::Object(map) => map
            .iter()
            .any(|(k, v)| (k.as_bytes() == b"expr" && node::truthy(Some(v))) || has_expression(v)),
        NodeKind::Array(items) => items.iter().any(has_expression),
        _ => false,
    }
}

/// Check the full dependency, including path properties, text animators,
/// masks and grouping. This is only needed when reusing at a different time.
fn time_independent(layer: &Node) -> bool {
    if !matches!(
        crate::js::layer_type(layer).as_str(),
        "shape" | "text" | "solid" | "group" | "null" | "audio"
    ) || ["timeRemap", "transitionIn", "transitionOut", "mblur"]
        .into_iter()
        .any(|key| node::truthy(layer.get(key)))
    {
        return false;
    }
    fn has_keys(node: &Node) -> bool {
        match node.kind() {
            NodeKind::Object(map) => map.iter().any(|(key, value)| {
                (key.as_bytes() == b"kf" && node::get(value, b"length").truthy()) || has_keys(value)
            }),
            NodeKind::Array(items) => items.iter().any(has_keys),
            _ => false,
        }
    }
    (!node::is_str(layer.get("type"), "text") || crate::text::plain_level1(layer))
        && !has_keys(layer)
}

fn stable_span(layer: &Node, before: f64, after: f64, comp_end: f64) -> bool {
    if node::is_str(layer.get("type"), "group") {
        return true;
    }
    let (Some(from), Some(dur)) = (
        node::number(layer.get("from")),
        node::number(layer.get("dur")),
    ) else {
        return false;
    };
    let end = from + dur;
    let within = |time: f64| {
        time >= from - 1e-6
            && (time < end - 1e-6
                || ((time - comp_end).abs() <= 1e-6 && (end - comp_end).abs() <= 1e-6))
    };
    before.is_finite() && after.is_finite() && within(before) == within(after)
}

impl LayerTraits {
    fn of(node: &Node) -> Self {
        let has_expression = has_expression(node);
        Self {
            node: node.clone(),
            has_expression,
            static_time: !has_expression && time_independent(node),
        }
    }
}

/// An empty or missing list member (`fx`, `masks`).
fn empty_list(layer: &Node, key: &str) -> bool {
    match node::member(Some(layer), key) {
        None => true,
        Some(n) => match n.kind() {
            NodeKind::Null => true,
            NodeKind::Array(items) => items.is_empty(),
            _ => false,
        },
    }
}

/// Draws which contain only native geometry, text and stills. A kept
/// group deliberately excludes media: its paths are not resolved again
/// when we skip planning the entire pass.
fn keepable_draws(draws: &Option<Vec<Draw>>, group: bool) -> bool {
    draws.as_ref().is_none_or(|ds| {
        ds.iter().all(|d| {
            if group {
                crate::frame_prefix::stable(&d.content, false)
            } else {
                crate::frame_prefix::stable(&d.content, true)
            }
        })
    })
}

fn simple_layer(layer: &Node) -> bool {
    empty_list(layer, "fx")
        && empty_list(layer, "masks")
        && ["transitionIn", "transitionOut", "matteSource"]
            .into_iter()
            .all(|key| !node::truthy(node::member(Some(layer), key)))
}

/// The image asset of a layer and the path its resolver answers now.
fn media_of(scope: &Scope<'_, '_>, b: &Build<'_>, layer: &Node) -> Option<(String, PathBuf)> {
    let d = node::member(Some(layer), "d");
    let asset = node::string(node::member(d, "asset"))?.to_string_lossy();
    let record = node::member(node::member(Some(scope.comp.root()), "assets"), &asset);
    let path = b.assets.path(&asset, record)?;
    Some((asset, path))
}

/// `draw_layer` through the reuse: the kept draws when they are still
/// valid, else `plan` (which plans the layer) and, when the layer can be
/// kept, an entry for the next frame.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw<'d>(
    scope: &mut Scope<'_, 'd>,
    b: &mut Build<'_>,
    layer: &'d Node,
    ty: &str,
    time: f64,
    width: u32,
    height: u32,
    scale: (f64, f64),
    facts: &Arc<crate::pass_facts::PassFacts>,
    plan: impl FnOnce(&mut Scope<'_, 'd>, &mut Build<'_>) -> JsResult<Option<Vec<Draw>>>,
) -> JsResult<Option<Vec<Draw>>> {
    let eligible = !facts.solo
        && b.can_reuse_draws()
        && b.forced_on.is_none()
        && b.matte_depth == 0
        && b.video_center.is_none()
        && b.reuse.as_ref().is_some_and(|r| r.active);
    if !eligible {
        return plan(scope, b);
    }
    let Some(id) = node::string(node::member(Some(layer), "id")).cloned() else {
        return plan(scope, b);
    };
    let group = ty == "group";
    if b.has_studio()
        && (group
            || node::truthy(
                layer
                    .get("d")
                    .and_then(|d| d.get("extrusion"))
                    .and_then(|e| e.get("enabled")),
            ))
    {
        return plan(scope, b);
    }
    if !matches!(
        ty,
        "solid" | "image" | "shape" | "text" | "shader" | "group"
    ) {
        return plan(scope, b);
    }
    let Ok(plane) = scope.is_3d_layer(layer) else {
        return plan(scope, b);
    };
    if plane && !group && b.camera_preview() {
        return plan(scope, b);
    }
    let Some(nodes) = closure(scope, layer) else {
        return plan(scope, b);
    };
    let camera = if group || plane {
        let Ok(camera) = scope.projection_camera_3d(time) else {
            return plan(scope, b);
        };
        camera
    } else {
        None
    };
    let pass = (
        scope.comp.id().clone(),
        PassKey {
            width,
            height,
            scale: (scale.0.to_bits(), scale.1.to_bits()),
            precomp_depth: b.precomp_depth,
            at_root: scope.comp.at_root(),
        },
    );
    let media = if ty == "image" {
        media_of(scope, b, layer)
    } else {
        None
    };
    let reuse_across_time = b.interactive_rasters();
    let reuse = b.reuse.as_mut().expect("eligible");
    let fields = reuse.fields_unchanged(scope);
    let stamp = reuse.stamp;
    let build = reuse.builds;
    let index = scope.layer_index();
    let current = |id: &JsString| match index.strings.get(id) {
        Some(&(i, false)) => scope.layers().get(i),
        _ => None,
    };
    let comp_end = scope.comp.duration();
    if fields
        && let Some(entry) = reuse.passes.get_mut(&pass).and_then(|p| p.get_mut(&id))
        && entry.media == media
        && entry.camera == camera
        && (entry.time.to_bits() == time.to_bits()
            || (reuse_across_time
                && entry.static_time
                && entry
                    .closure
                    .iter()
                    .all(|(_, layer)| stable_span(layer, entry.time, time, comp_end))))
        && entry
            .shaders
            .iter()
            .all(|key| !crate::shader_layers::translate::native_failed(key))
        && match &entry.group {
            Some(old) => Arc::ptr_eq(old, facts),
            None => {
                entry.closure.len() == nodes.len()
                    && entry
                        .closure
                        .iter()
                        .zip(&nodes)
                        .all(|((id, _), node)| current(id).is_some_and(|n| std::ptr::eq(n, *node)))
            }
        }
        && entry
            .closure
            .iter()
            .all(|(id, old)| current(id).is_some_and(|n| unchanged(n, old, stamp)))
    {
        entry.used = build;
        entry.time = time;
        reuse.stats.reused += 1;
        pm_perf::bump(pm_perf::Counter::DrawReuseHits);
        return Ok(entry.draws.clone());
    }
    reuse.stats.planned += 1;
    pm_perf::bump(pm_perf::Counter::DrawReuseMisses);
    // Plan the layer and keep it when it qualifies.
    let failures = b.failures.len();
    let unsupported = b.unsupported.len();
    let draws = plan(scope, b)?;
    let reported = b.failures.len() != failures || b.unsupported.len() != unsupported;
    let svg = ty == "image"
        && media.as_ref().is_some_and(|(asset, _)| {
            let record = node::member(node::member(Some(scope.comp.root()), "assets"), asset);
            node::string(node::member(record, "format"))
                .is_none_or(|f| f.to_string_lossy().eq_ignore_ascii_case("svg"))
        });
    let keep = !reported
        && !svg
        && (ty != "image" || media.is_some())
        && keepable_draws(&draws, group)
        && simple_layer(layer);
    let reuse = b.reuse.as_mut().expect("eligible");
    if keep
        && let Some((closure, static_time)) =
            dependencies(scope, nodes, group.then_some(facts), &id, reuse)
    {
        reuse.stats.stored += 1;
        reuse.passes.entry(pass).or_default().insert(
            id,
            Entry {
                time,
                static_time: reuse_across_time && static_time,
                shaders: crate::frame_prefix::shader_keys(draws.as_deref().unwrap_or_default()),
                draws: draws.clone(),
                closure,
                group: group.then(|| facts.clone()),
                camera,
                media,
                used: build,
            },
        );
    }
    Ok(draws)
}

/// Validate all inputs of a boundary group, including the parents of its
/// children outside the group. The pass facts identify moves and reorders
/// even when the layers themselves retain their revisions.
fn dependencies<'d>(
    scope: &mut Scope<'_, 'd>,
    mut nodes: Vec<&'d Node>,
    group: Option<&Arc<crate::pass_facts::PassFacts>>,
    id: &JsString,
    reuse: &mut DrawReuse,
) -> Option<(Vec<(JsString, Node)>, bool)> {
    if let Some(facts) = group {
        for layer in facts.owners.descendants(scope.layers(), id)? {
            if !matches!(
                crate::js::layer_type(layer).as_str(),
                "group" | "solid" | "shape" | "text" | "shader" | "null" | "audio"
            ) || !simple_layer(layer)
            {
                return None;
            }
            nodes.extend(closure(scope, layer)?);
        }
    }
    let index = scope.layer_index();
    let mut checked = HashSet::new();
    let mut result = Vec::new();
    let mut static_time = true;
    for n in nodes {
        if !checked.insert(std::ptr::from_ref(n)) {
            continue;
        }
        let id = node::string(node::member(Some(n), "id"))?;
        if !matches!(index.strings.get(id), Some((_, false))) {
            return None;
        }
        let traits = reuse.layer_traits(scope.comp, id, n);
        if traits.has_expression {
            return None;
        }
        static_time &= traits.static_time;
        result.push((id.clone(), n.clone()));
    }
    Some((result, static_time))
}
