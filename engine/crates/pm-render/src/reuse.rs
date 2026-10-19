//! Layer draws reused across the frames of one paused time
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
//! Only layers whose draws provably depend on nothing else are kept:
//!
//! - solid layers and raster image layers (not SVG: its plan reads the
//!   file), in 2D, through today's fast path: no effects, masks,
//!   transitions, track matte, blend mode, motion blur, or 2.5D plane, and
//!   no expression anywhere in the layer or in the layers it depends on;
//! - nothing the build reported for the layer (no failure, no unsupported
//!   feature);
//! - built outside a track matte's pass, outside motion-blur samples, and
//!   in a pass without solo layers.
//!
//! What such a layer's draws read: its own subtree; the layers its
//! transform and activity go through (its parent chain and enclosing
//! groups, and theirs, transitively: the *closure*); the composition's own
//! fields, the open composition's, and the document root's (assets,
//! project settings; `Comp::member` falls back to the root); the
//! composition time, the pass's size and scale, the build's options; and,
//! for an image, the media path the asset resolver answers.
//!
//! # Validity
//!
//! Every node remembers the revision its subtree last changed at
//! (`pm_model::doc`). Entries are kept for one revision (the last frame's,
//! the *stamp*) and given back only when:
//!
//! - the closure, found again in this revision's document, is the same
//!   nodes (address and id) and every one of them is unchanged since the
//!   stamp (`rev <= stamp`), with every id unique in the composition (a
//!   unique id found again unchanged is the same logical layer, the
//!   argument `pm-eval`'s interned properties make);
//! - the composition's, the open composition's, and the root's fields
//!   other than `layers` and `comps` are unchanged since the stamp;
//! - the pass (composition, time, size, scale, precomp depth) and the
//!   build's options are the same, and so is the image's media path.
//!
//! Anything else plans the layer as before. A build at another time than
//! the last one does not look up or keep anything (playback pays one
//! comparison per frame), and a different document (root address, project
//! id, or a revision going back) or other build options drop every entry.
//! Entries a frame did not use are dropped after it.

use std::collections::HashMap;
use std::path::PathBuf;

use pm_eval::{JsResult, Scope, node};
use pm_model::{JsString, Node, NodeKind};

use crate::plan::{Build, Content, Draw, Source};

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
    /// Builds that did not use the reuse (another time, another
    /// document, other options).
    pub cold_builds: u64,
    /// Frames drawn from the kept bottom of an earlier frame
    /// (`crate::frame_prefix`), and the top-level draws they did not draw
    /// again.
    pub prefix_frames: u64,
    pub prefix_draws: u64,
}

/// The build options a layer's draws depend on.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BuildKey {
    pub sampling: crate::sampling::Sampling,
    pub blur: Option<crate::motion::Blur>,
    pub video_padding: u64,
    pub view_override: bool,
    /// Today's output scale for level 1 SVG buckets (bits), when the
    /// request has one: it changes the buckets without changing the pass.
    pub svg_scale: Option<(u64, u64)>,
}

/// Which document the entries belong to.
#[derive(Debug, Clone, PartialEq)]
struct DocKey {
    root: usize,
    /// The first member of the root: a replaced document is built while
    /// the old one is alive, so its members are elsewhere.
    first: usize,
    project: Option<JsString>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PassKey {
    time: u64,
    width: u32,
    height: u32,
    scale: (u64, u64),
    precomp_depth: u32,
    at_root: bool,
}

struct Entry {
    draws: Option<Vec<Draw>>,
    /// The closure: node address and id, the layer first.
    closure: Vec<(usize, JsString)>,
    /// An image's asset and the path its resolver answered.
    media: Option<(String, PathBuf)>,
    /// The build that last used the entry.
    used: u64,
}

/// The reuse state a renderer keeps across frames.
#[derive(Default)]
pub struct DrawReuse {
    doc: Option<DocKey>,
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
    fields_unchanged: Vec<(usize, bool)>,
    stats: ReuseStats,
    facts: crate::pass_facts::FactsCache,
}

impl DrawReuse {
    pub fn new() -> DrawReuse {
        DrawReuse::default()
    }

    pub fn stats(&self) -> ReuseStats {
        self.stats
    }

    /// Drops every entry (the next build at the same time stores again).
    pub fn clear(&mut self) {
        self.passes.clear();
        self.facts.clear();
        self.doc = None;
        self.build = None;
        self.time = None;
    }

    /// Starts a build of `root` at revision `revision`, time `time`.
    pub(crate) fn begin(&mut self, root: &Node, revision: u64, time: f64, build: BuildKey) {
        self.builds += 1;
        self.fields_unchanged.clear();
        let first = match root.kind() {
            NodeKind::Object(map) => map
                .values()
                .next()
                .map_or(0, |n| std::ptr::from_ref(n) as usize),
            _ => 0,
        };
        let doc = DocKey {
            root: std::ptr::from_ref(root) as usize,
            first,
            project: node::string(node::member(Some(root), "id")).cloned(),
        };
        if self.doc.as_ref() != Some(&doc) || revision < self.stamp {
            self.facts.clear();
        }
        let time = time.to_bits();
        let same = self.doc.as_ref() == Some(&doc)
            && self.build.as_ref() == Some(&build)
            && self.time == Some(time)
            && revision >= self.stamp;
        if !same {
            self.passes.clear();
        }
        // The first build at a time stores nothing: playback never pays
        // for entries it would not use.
        self.active = same || (self.doc.as_ref() == Some(&doc) && self.time == Some(time));
        if !self.active {
            self.stats.cold_builds += 1;
        }
        self.doc = Some(doc);
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
        self.stamp = self.revision;
        self.active = false;
    }

    pub(crate) fn pass_facts(
        &mut self,
        layers: &[Node],
        all: &[&Node],
    ) -> std::sync::Arc<crate::pass_facts::PassFacts> {
        self.facts.get(layers, all, self.revision)
    }

    /// Whether `comp`'s own fields (and the root's, and the open
    /// composition's) are unchanged since the stamp.
    fn fields_unchanged(&mut self, scope: &Scope<'_, '_>) -> bool {
        let stamp = self.stamp;
        let mut check = |node: &Node, skip: &[&[u8]]| -> bool {
            let address = std::ptr::from_ref(node) as usize;
            if let Some(&(_, ok)) = self.fields_unchanged.iter().find(|(a, _)| *a == address) {
                return ok;
            }
            let ok = match node.kind() {
                NodeKind::Object(map) => map
                    .iter()
                    .all(|(k, v)| skip.contains(&k.as_bytes()) || v.rev() <= stamp),
                _ => node.rev() <= stamp,
            };
            self.fields_unchanged.push((address, ok));
            ok
        };
        check(scope.comp.node(), &[b"layers", b"comps"])
            && check(scope.project.node(), &[b"layers", b"comps"])
            && check(scope.comp.root(), &[b"layers", b"comps"])
    }
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

/// Draws a reused layer may hold: today's fast path in 2D.
fn keepable_draws(draws: &Option<Vec<Draw>>) -> bool {
    let simple = |content: &Content| match content {
        Content::Solid(_) => true,
        Content::Texture { source, .. } => matches!(source, Source::Image { .. }),
        _ => false,
    };
    draws.as_ref().is_none_or(|ds| {
        ds.iter().all(|d| match &d.content {
            Content::Placed(p) => p.plane.is_none() && simple(&p.content),
            // A level 2 solid's scene at output resolution.
            Content::Texture {
                source: Source::Vector(v),
                ..
            } => matches!(**v, crate::vector::VectorSource::Scene { .. }),
            other => simple(other),
        })
    })
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
    time: f64,
    width: u32,
    height: u32,
    scale: (f64, f64),
    solo: bool,
    plan: impl FnOnce(&mut Scope<'_, 'd>, &mut Build<'_>) -> JsResult<Option<Vec<Draw>>>,
) -> JsResult<Option<Vec<Draw>>> {
    let eligible = !solo
        && b.forced_on.is_none()
        && b.matte_depth == 0
        && b.video_center.is_none()
        && b.reuse.as_ref().is_some_and(|r| r.active);
    if !eligible {
        return plan(scope, b);
    }
    let ty = crate::js::layer_type(layer);
    let Some(id) = node::string(node::member(Some(layer), "id")).cloned() else {
        return plan(scope, b);
    };
    if !matches!(ty.as_str(), "solid" | "image") {
        return plan(scope, b);
    }
    let Some(nodes) = closure(scope, layer) else {
        return plan(scope, b);
    };
    let index = scope.layer_index();
    let mut ids: Vec<(usize, JsString)> = Vec::with_capacity(nodes.len());
    for n in &nodes {
        let Some(nid) = node::string(node::member(Some(n), "id")) else {
            return plan(scope, b);
        };
        if !matches!(index.strings.get(nid), Some((_, false))) {
            return plan(scope, b);
        }
        ids.push((std::ptr::from_ref(*n) as usize, nid.clone()));
    }
    let pass = (
        scope.comp.id().clone(),
        PassKey {
            time: time.to_bits(),
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
    let reuse = b.reuse.as_mut().expect("eligible");
    let fields = reuse.fields_unchanged(scope);
    let stamp = reuse.stamp;
    let build = reuse.builds;
    if fields
        && let Some(entry) = reuse.passes.get_mut(&pass).and_then(|p| p.get_mut(&id))
        && entry.closure == ids
        && entry.media == media
        && nodes.iter().all(|n| n.rev() <= stamp)
    {
        entry.used = build;
        reuse.stats.reused += 1;
        return Ok(entry.draws.clone());
    }
    reuse.stats.planned += 1;
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
        && keepable_draws(&draws)
        && empty_list(layer, "fx")
        && empty_list(layer, "masks")
        && !node::truthy(node::member(Some(layer), "transitionIn"))
        && !node::truthy(node::member(Some(layer), "transitionOut"))
        && !node::truthy(node::member(Some(layer), "matteSource"))
        && !node::truthy(node::member(Some(layer), "threeD"))
        && !nodes.iter().any(|n| has_expression(n))
        && scope.is_3d_layer(layer).is_ok_and(|three| !three);
    if keep {
        let reuse = b.reuse.as_mut().expect("eligible");
        reuse.stats.stored += 1;
        reuse.passes.entry(pass).or_default().insert(
            id,
            Entry {
                draws: draws.clone(),
                closure: ids,
                media,
                used: build,
            },
        );
    }
    Ok(draws)
}
