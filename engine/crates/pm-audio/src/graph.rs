//! The mix graph of a composition: which clips sound, through which strips
//! and precompositions, and how each one's level and source time follow
//! the document.
//!
//! The walk is `audioLayers` of `legacy/core/audio.ts`, so the same layers
//! sound: audio layers with media; video layers with `embeddedAudio`; the
//! audio of precompositions at any depth up to 8, one voice per path, so a
//! composition used twice plays twice; per-composition solo (a layer is
//! skipped when its composition has a soloed layer and neither it nor an
//! enclosing group is soloed); layers and groups switched off; `audioMuted`
//! on videos and precompositions. Each voice feeds the strip of the
//! top-level layer it belongs to.
//!
//! Levels follow `gainAt`: the clip's `gain` (audio layers, 0 to 4) or
//! `audioGain` (embedded audio), times every enclosing precomposition's
//! `audioGain` and its composition's output level; linear fades over the
//! clip's audible length (the layer, cut short where the source ends); zero
//! while the layer, an enclosing group, or (an addition) an enclosing
//! precomposition is switched off. The open composition's own `audioGain`
//! is the master fader.
//!
//! Differences from the legacy engine, each a fix:
//!
//! * Speed and time remapping are followed (appendix A item 10: today
//!   speed is ignored and remapped clips are silent). Source time is
//!   `pm_eval`'s `sourceTime` at every level, so audio stays with the
//!   picture. The pitch is preserved by default (Signalsmith Stretch) or
//!   follows the speed per clip (`pitch`, [`crate::conform::Pitch`]); a
//!   conformed frame rate (Interpret Footage) scales the soundtrack's time
//!   with the picture's.
//! * A negative layer start plays from the right source time (the legacy
//!   walk clamps `from` to 0 and shifts the clip).
//! * Animated `audioMuted`, precomposition `on`, and groups enclosing a
//!   precomposition are evaluated over time, not only when static.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use pm_eval::retime::RetimeCache;
use pm_eval::{Comp, Evaluator, JsValue, Scope, node};
use pm_model::{Document, JsString, Node, NodeKind};

use crate::conform::{self, Pitch};
use crate::error::{AudioError, Result};
use crate::source::{Source, SourceCache};

/// Nesting limit (`MAX_PRECOMP_DEPTH`).
pub const MAX_PRECOMP_DEPTH: usize = 8;
/// The legacy walk drops windows shorter than this (seconds).
const MIN_WINDOW: f64 = 1e-4;

/// Resolves an asset id to a file on disk (the media store).
pub trait AssetResolver: Send + Sync {
    fn resolve(&self, asset: &str) -> Option<PathBuf>;
}

impl<F> AssetResolver for F
where
    F: Fn(&str) -> Option<PathBuf> + Send + Sync,
{
    fn resolve(&self, asset: &str) -> Option<PathBuf> {
        self(asset)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceKind {
    /// An `audio` layer.
    Audio,
    /// The soundtrack of a `video` layer with `embeddedAudio`.
    Embedded,
}

/// One layer on a voice's path: a precomposition, or the clip itself last.
#[derive(Clone, Debug)]
pub struct Level {
    /// Slot of the composition holding the layer ([`MixGraph::comps`]).
    pub comp: usize,
    /// Index in that composition's `layers`.
    pub layer: usize,
    /// Indexes of the enclosing groups in the same composition.
    pub groups: Vec<usize>,
    /// For precompositions: the static output level of the nested
    /// composition (`comp.audioGain`).
    pub nested_level: f64,
}

/// What a voice plays and how its time maps to the source.
#[derive(Clone, Debug)]
pub struct VoicePlan {
    /// The legacy voice id: layer ids joined by `/` along the path, with
    /// `:embedded-audio` for a video's soundtrack.
    pub id: String,
    /// The strip (top-level layer id).
    pub strip: String,
    pub kind: VoiceKind,
    pub name: String,
    pub asset: String,
    pub source: Arc<Source>,
    /// Outermost precomposition first; the clip last.
    pub levels: Vec<Level>,
    /// Master-time span outside which the voice is silent. Exact when the
    /// time map is linear ([`Self::offset`]); otherwise a bound from the
    /// linear prefix of the path, refined per control point.
    pub window: (f64, f64),
    /// `source time = master time − offset` (seconds) when every level maps
    /// time with slope 1 and no animation (no remap, no speed, static trim).
    pub offset: Option<f64>,
    /// Whether any level or gain input is animated (evaluated per control
    /// point); otherwise [`Self::static_point`] holds everywhere.
    pub animated: bool,
    /// Gain, fades, and audible length when nothing is animated.
    pub static_point: Option<Point>,
    /// The master time [`Self::static_point`] was evaluated at.
    pub static_at: f64,
    /// How the pitch follows a speed change (`d.pitch`).
    pub pitch: Pitch,
    /// Source seconds per clip second from a conformed frame rate
    /// (Interpret Footage on a video asset); 1 otherwise. Applied in
    /// [`Point::source_time`].
    pub conform: f64,
    /// The read rate (source seconds per master second) when it is the
    /// same everywhere: a static speed, no remap, static trims along the
    /// path. `None` when the rate varies over time.
    pub constant_rate: Option<f64>,
    /// A digest of every input of the voice's time map (the source, the
    /// path's spans, trims, speeds, remaps, rates, the conform factor, and
    /// the pitch choice): two revisions with the same key read the source
    /// identically, so a time stretch in progress carries over a live edit.
    pub map_key: u64,
}

/// One evaluation of a voice at a master time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    /// Every level is switched on and not muted.
    pub active: bool,
    /// Every level's time is inside its layer's span. Linear voices take the
    /// span from their exact window instead (control points would round it
    /// to the control interval).
    pub in_span: bool,
    /// Level before fades (clip, precompositions, compositions).
    pub gain: f64,
    /// Source program time, seconds (a conformed clip's clip time scaled by
    /// [`VoicePlan::conform`]).
    pub source_time: f64,
    /// The clip's local time (`t − from` in its composition), seconds.
    pub local: f64,
    pub fade_in: f64,
    pub fade_out: f64,
    /// Local time at which the clip stops sounding (its duration, or where
    /// its source ends), for the fade out.
    pub audible: f64,
}

impl Point {
    /// `min(entering, leaving)` at a local time (`gainAt`).
    #[inline]
    pub fn fade(&self, local: f64) -> f64 {
        let local = local.clamp(0.0, self.audible.max(0.0));
        let entering = if self.fade_in > 0.0 {
            (local / self.fade_in).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let leaving = if self.fade_out > 0.0 {
            ((self.audible - local) / self.fade_out).clamp(0.0, 1.0)
        } else {
            1.0
        };
        entering.min(leaving)
    }
}

/// The mix of one composition at one document revision.
#[derive(Clone, Debug)]
pub struct MixGraph {
    /// The open composition.
    pub comp_id: String,
    /// Composition ids by slot (slot 0 is the open composition).
    pub comps: Vec<String>,
    pub voices: Vec<VoicePlan>,
    /// Strip ids in timeline order.
    pub strips: Vec<String>,
    /// The master fader (`audioGain` of the open composition).
    pub master: f64,
    /// Media that could not be opened: (layer name, asset, reason).
    pub problems: Vec<(String, String, String)>,
}

/// `finite(value, fallback)`: `Number(value)` when finite.
fn finite(v: &JsValue, fallback: f64) -> f64 {
    let n = v.to_number();
    if n.is_finite() { n } else { fallback }
}

/// `levelOf`: absent or null is unity, else clamped to 0..4.
fn level_of(v: Option<&JsValue>) -> f64 {
    match v {
        None | Some(JsValue::Undefined) | Some(JsValue::Null) => 1.0,
        Some(v) => finite(v, 1.0).clamp(0.0, 4.0),
    }
}

fn level_of_node(n: Option<&Node>) -> f64 {
    level_of(Some(&node::js_value_opt(n)))
}

/// A channel with keyframes or an expression.
fn animated(n: Option<&Node>) -> bool {
    node::is_property(n)
        && (node::array(node::member(n, "kf")).is_some_and(|k| !k.is_empty())
            || node::truthy(node::member(n, "expr")))
}

fn is_false(n: Option<&Node>) -> bool {
    matches!(n.map(Node::kind), Some(NodeKind::Bool(false)))
}

fn is_true(n: Option<&Node>) -> bool {
    matches!(n.map(Node::kind), Some(NodeKind::Bool(true)))
}

fn number(n: Option<&Node>, fallback: f64) -> f64 {
    let v = node::to_number(n);
    if v.is_finite() { v } else { fallback }
}

/// A static number, or `None` when the value is a channel (animated or not,
/// a channel's static value is read through the evaluator).
fn static_number(n: Option<&Node>) -> Option<f64> {
    if node::is_property(n) {
        if animated(n) {
            return None;
        }
        return Some(node::to_number(node::member(n, "v")));
    }
    Some(node::to_number(n))
}

/// `speed` absent, null, or a static 1 (`sourceTime` treats absent and
/// null as 1).
fn unit_speed(n: Option<&Node>) -> bool {
    if node::is_nullish(n) {
        return true;
    }
    static_number(n) == Some(1.0)
}

struct Walk<'a, 'd> {
    doc: &'d Document,
    root: &'d Node,
    ev: &'a mut Evaluator,
    /// The source cache and the media resolver; `None` for [`audible`]'s
    /// walk, which counts the clips the mix would play without opening
    /// their media.
    media: Option<(&'a SourceCache, &'a dyn AssetResolver)>,
    /// Clips found by a walk without media.
    candidates: usize,
    comps: Vec<String>,
    comp_slots: HashMap<String, usize>,
    voices: Vec<VoicePlan>,
    problems: Vec<(String, String, String)>,
}

#[derive(Clone)]
struct PathState {
    levels: Vec<Level>,
    /// Master-time offset of the composition when the path so far is
    /// linear (legacy `offset`).
    offset: Option<f64>,
    window: (f64, f64),
    animated: bool,
    id: String,
    strip: Option<String>,
}

impl<'d> Walk<'_, 'd> {
    fn slot(&mut self, id: &str) -> usize {
        if let Some(&s) = self.comp_slots.get(id) {
            return s;
        }
        self.comps.push(id.to_owned());
        self.comp_slots.insert(id.to_owned(), self.comps.len() - 1);
        self.comps.len() - 1
    }

    fn visit(&mut self, comp_id: &str, depth: usize, path: &PathState) -> Result<()> {
        if depth > MAX_PRECOMP_DEPTH {
            return Ok(());
        }
        let comp = match Comp::in_root(self.root, comp_id, self.doc.revision()) {
            Ok(c) => c,
            Err(_) => return Ok(()),
        };
        let slot = self.slot(comp_id);
        let layers = comp.layers();
        let soloing = layers
            .iter()
            .any(|l| node::truthy(node::member(Some(l), "solo")));
        for (index, layer) in layers.iter().enumerate() {
            if node::is_nullish(Some(layer)) || is_false(node::member(Some(layer), "on")) {
                continue;
            }
            let groups = self.ev.group_ancestors(&comp, layer)?;
            if soloing
                && !node::truthy(node::member(Some(layer), "solo"))
                && !groups
                    .iter()
                    .any(|g| node::truthy(node::member(Some(g), "solo")))
            {
                continue;
            }
            if groups.iter().any(|g| is_false(node::member(Some(g), "on"))) {
                continue;
            }
            let group_indexes: Vec<usize> = groups
                .iter()
                .filter_map(|g| layers.iter().position(|l| std::ptr::eq(l, *g)))
                .collect();
            let groups_animated = groups.iter().any(|g| animated(node::member(Some(g), "on")));
            let from = number(node::member(Some(layer), "from"), 0.0);
            let dur = number(node::member(Some(layer), "dur"), 0.0).max(0.0);
            let mut window = path.window;
            if let Some(offset) = path.offset {
                window = (
                    window.0.max(offset + from),
                    window.1.min(offset + from + dur),
                );
                if window.1 - window.0 <= MIN_WINDOW {
                    continue;
                }
            }
            let id = node::to_js_string(node::member(Some(layer), "id")).to_string_lossy();
            let item = if path.id.is_empty() {
                id.clone()
            } else {
                format!("{}/{}", path.id, id)
            };
            let strip = path.strip.clone().unwrap_or_else(|| id.clone());
            let d = node::member(Some(layer), "d");
            let ty = node::to_js_string(node::member(Some(layer), "type")).to_string_lossy();
            let level = Level {
                comp: slot,
                layer: index,
                groups: group_indexes,
                nested_level: 1.0,
            };
            let on_animated = animated(node::member(Some(layer), "on")) || groups_animated;
            match ty.as_str() {
                "audio" => {
                    let Some(asset) =
                        node::string(node::member(d, "asset")).map(|s| s.to_string_lossy())
                    else {
                        continue;
                    };
                    if asset.is_empty() {
                        continue;
                    }
                    let dynamic = on_animated
                        || ["gain", "fadeIn", "fadeOut"]
                            .iter()
                            .any(|k| animated(node::member(d, k)));
                    let linear = static_number(node::member(d, "trim")).is_some()
                        && unit_speed(node::member(d, "speed"));
                    self.voice(
                        &comp,
                        layer,
                        VoiceKind::Audio,
                        item,
                        strip,
                        asset,
                        path,
                        level,
                        window,
                        from,
                        linear,
                        dynamic,
                    )?;
                }
                "video" => {
                    if !is_true(node::member(d, "embeddedAudio"))
                        || is_true(node::member(d, "audioMuted"))
                    {
                        continue;
                    }
                    let Some(asset) =
                        node::string(node::member(d, "asset")).map(|s| s.to_string_lossy())
                    else {
                        continue;
                    };
                    if asset.is_empty() {
                        continue;
                    }
                    let remap = node::member(d, "timeRemap");
                    let dynamic = on_animated
                        || animated(node::member(d, "audioGain"))
                        || animated(node::member(d, "audioMuted"));
                    let linear = !(node::truthy(remap) || node::is_property(remap))
                        && static_number(node::member(d, "trim")).is_some()
                        && unit_speed(node::member(d, "speed"));
                    self.voice(
                        &comp,
                        layer,
                        VoiceKind::Embedded,
                        format!("{item}:embedded-audio"),
                        strip,
                        asset,
                        path,
                        level,
                        window,
                        from,
                        linear,
                        dynamic,
                    )?;
                }
                "precomp" => {
                    let Some(nested) =
                        node::string(node::member(d, "comp")).map(|s| s.to_string_lossy())
                    else {
                        continue;
                    };
                    if nested.is_empty() || is_true(node::member(d, "audioMuted")) {
                        continue;
                    }
                    let nested_node = node::member(node::member(Some(self.root), "comps"), &nested)
                        .or_else(|| {
                            // The stored form keeps the open composition at the root.
                            Comp::in_root(self.root, &nested, 0).ok().map(|c| c.node())
                        });
                    let Some(nested_node) = nested_node else {
                        continue;
                    };
                    let remap = node::member(d, "timeRemap");
                    let trim = static_number(node::member(d, "trim"));
                    let linear =
                        !(node::truthy(remap) || node::is_property(remap)) && trim.is_some();
                    let mut next = path.clone();
                    let mut level = level;
                    level.nested_level =
                        level_of_node(node::member(Some(nested_node), "audioGain"));
                    next.levels.push(level);
                    next.window = window;
                    next.id = item;
                    next.strip = Some(strip);
                    next.animated |= on_animated
                        || animated(node::member(d, "audioGain"))
                        || animated(node::member(d, "audioMuted"));
                    next.offset = match (path.offset, linear, trim) {
                        (Some(o), true, Some(trim)) => Some(
                            o + from
                                - if trim.is_finite() && trim != 0.0 {
                                    trim
                                } else {
                                    0.0
                                },
                        ),
                        _ => None,
                    };
                    self.visit(&nested, depth + 1, &next)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn voice(
        &mut self,
        comp: &Comp<'d>,
        layer: &'d Node,
        kind: VoiceKind,
        id: String,
        strip: String,
        asset: String,
        path: &PathState,
        level: Level,
        window: (f64, f64),
        from: f64,
        linear: bool,
        dynamic: bool,
    ) -> Result<()> {
        let name = node::string(node::member(Some(layer), "name"))
            .map(|s| s.to_string_lossy())
            .unwrap_or_else(|| match kind {
                VoiceKind::Audio => "Audio".into(),
                VoiceKind::Embedded => "Video".into(),
            });
        let Some((cache, resolver)) = self.media else {
            self.candidates += 1;
            return Ok(());
        };
        let Some(file) = resolver.resolve(&asset) else {
            self.problems
                .push((name, asset, "the media is missing".into()));
            return Ok(());
        };
        let source = match cache.open(&file, None) {
            Ok(s) => s,
            // A video without a soundtrack has nothing to play.
            Err(AudioError::Media(pm_media::MediaError::Invalid(m)))
                if kind == VoiceKind::Embedded && m == "no audio stream" =>
            {
                return Ok(());
            }
            Err(e) => {
                self.problems.push((name, asset, e.to_string()));
                return Ok(());
            }
        };
        let mut levels = path.levels.clone();
        levels.push(level);
        let d = node::member(Some(layer), "d");
        let trim = static_number(node::member(d, "trim"))
            .filter(|t| t.is_finite())
            .unwrap_or(0.0);
        let pitch = Pitch::of(node::member(d, "pitch"));
        // A conformed video plays its soundtrack with its frames (also
        // through an audio layer of the same asset).
        let record = node::member(node::member(Some(self.root), "assets"), asset.as_str());
        let conform = match conform::conformed_rate(record) {
            Some(rate) => conform::conform_factor(Some(rate), conform::native_rate(&file)),
            None => 1.0,
        };
        let offset = match (path.offset, linear) {
            (Some(o), true) if conform == 1.0 => Some(o + from - trim),
            _ => None,
        };
        // A static speed along a linear path reads at one rate everywhere.
        let remap = node::member(d, "timeRemap");
        let constant_rate = match path.offset {
            Some(_)
                if static_number(node::member(d, "trim")).is_some()
                    && !(node::truthy(remap) || node::is_property(remap)) =>
            {
                let speed = if node::is_nullish(node::member(d, "speed")) {
                    Some(1.0)
                } else {
                    static_number(node::member(d, "speed"))
                };
                speed.filter(|s| s.is_finite()).map(|s| s * conform)
            }
            _ => None,
        };
        let map_key = self.map_key(&levels, kind, &asset, &file, &source, conform, pitch);
        let mut plan = VoicePlan {
            id,
            strip,
            kind,
            name,
            asset,
            source,
            levels,
            window,
            offset,
            animated: path.animated || dynamic,
            static_point: None,
            static_at: 0.0,
            pitch,
            conform,
            constant_rate,
            map_key,
        };
        if !plan.animated {
            // Every input is static: evaluate once, inside the window.
            let _ = comp;
            let (w0, w1) = plan.window;
            let t = match (w0.is_finite(), w1.is_finite()) {
                (true, true) => 0.5 * (w0 + w1),
                (true, false) => w0 + 1.0,
                _ => 0.0,
            };
            plan.static_at = t;
            let comps: Vec<Comp<'d>> = self
                .comps
                .iter()
                .map(|c| Comp::in_root(self.root, c, self.doc.revision()))
                .collect::<std::result::Result<_, _>>()?;
            let mut retime = RetimeCache::new();
            let open = comps[0].clone();
            plan.static_point = Some(plan.evaluate(self.ev, &comps, &open, t, &mut retime)?);
        }
        self.voices.push(plan);
        Ok(())
    }
}

impl Walk<'_, '_> {
    /// [`VoicePlan::map_key`]: every field the voice's time map reads.
    #[allow(clippy::too_many_arguments)]
    fn map_key(
        &self,
        levels: &[Level],
        kind: VoiceKind,
        asset: &str,
        file: &std::path::Path,
        source: &Source,
        conform: f64,
        pitch: Pitch,
    ) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (kind == VoiceKind::Embedded).hash(&mut h);
        asset.hash(&mut h);
        file.hash(&mut h);
        source.id().hash(&mut h);
        conform.to_bits().hash(&mut h);
        pitch.hash(&mut h);
        let digest = |h: &mut std::collections::hash_map::DefaultHasher, n: Option<&Node>| match n {
            Some(n) => pm_model::hash::canonical_digest(n).hash(h),
            None => 0u8.hash(h),
        };
        for level in levels {
            let comp_id = &self.comps[level.comp];
            comp_id.hash(&mut h);
            let Ok(comp) = Comp::in_root(self.root, comp_id, self.doc.revision()) else {
                continue;
            };
            digest(&mut h, comp.member("fps"));
            digest(&mut h, comp.member("fpsRational"));
            let Some(layer) = comp.layers().get(level.layer) else {
                continue;
            };
            digest(&mut h, node::member(Some(layer), "id"));
            digest(&mut h, node::member(Some(layer), "from"));
            let d = node::member(Some(layer), "d");
            for key in ["trim", "speed", "timeRemap", "sourceTime"] {
                digest(&mut h, node::member(d, key));
            }
        }
        h.finish()
    }
}

/// Whether the composition `comp_id` of `doc` has anything to play: a clip
/// the mix would include (audio layers with media, videos with embedded
/// audio that is not muted, through precomps, solo, and on states, as
/// [`MixGraph::build`] walks them), without opening any media. Today's
/// `hasAudibleLayers` asks the same before it warms the audio device.
pub fn audible(doc: &Document, comp_id: &str, ev: &mut Evaluator) -> Result<bool> {
    let root = doc
        .root()
        .ok_or_else(|| AudioError::invalid("the document is empty"))?;
    Comp::resolve(doc, comp_id)?;
    let mut walk = Walk {
        doc,
        root,
        ev,
        media: None,
        candidates: 0,
        comps: vec![],
        comp_slots: HashMap::new(),
        voices: vec![],
        problems: vec![],
    };
    walk.slot(comp_id);
    let start = PathState {
        levels: vec![],
        offset: Some(0.0),
        window: (0.0, f64::INFINITY),
        animated: false,
        id: String::new(),
        strip: None,
    };
    walk.visit(comp_id, 0, &start)?;
    Ok(walk.candidates > 0)
}

impl MixGraph {
    /// Walks the composition `comp_id` of `doc`. Media that cannot be
    /// resolved or opened is listed in [`MixGraph::problems`] and left out.
    pub fn build(
        doc: &Document,
        comp_id: &str,
        ev: &mut Evaluator,
        cache: &SourceCache,
        resolver: &dyn AssetResolver,
    ) -> Result<MixGraph> {
        let root = doc
            .root()
            .ok_or_else(|| AudioError::invalid("the document is empty"))?;
        let open = Comp::resolve(doc, comp_id)?;
        let master = level_of_node(open.member("audioGain"));
        let mut walk = Walk {
            doc,
            root,
            ev,
            media: Some((cache, resolver)),
            candidates: 0,
            comps: vec![],
            comp_slots: HashMap::new(),
            voices: vec![],
            problems: vec![],
        };
        walk.slot(comp_id);
        let start = PathState {
            levels: vec![],
            offset: Some(0.0),
            window: (0.0, f64::INFINITY),
            animated: false,
            id: String::new(),
            strip: None,
        };
        walk.visit(comp_id, 0, &start)?;
        let mut strips: Vec<String> = Vec::new();
        for v in &walk.voices {
            if !strips.contains(&v.strip) {
                strips.push(v.strip.clone());
            }
        }
        Ok(MixGraph {
            comp_id: comp_id.to_owned(),
            comps: walk.comps,
            voices: walk.voices,
            strips,
            master,
            problems: walk.problems,
        })
    }

    /// The compositions by slot, resolved in `doc`.
    pub fn resolve<'d>(&self, doc: &'d Document) -> Result<Vec<Comp<'d>>> {
        let root = doc
            .root()
            .ok_or_else(|| AudioError::invalid("the document is empty"))?;
        self.comps
            .iter()
            .map(|c| Comp::in_root(root, c, doc.revision()).map_err(AudioError::from))
            .collect()
    }
}

impl VoicePlan {
    /// Evaluates the voice at master time `t`.
    pub fn evaluate<'d>(
        &self,
        ev: &mut Evaluator,
        comps: &[Comp<'d>],
        open: &Comp<'d>,
        t: f64,
        retime: &mut RetimeCache,
    ) -> Result<Point> {
        let mut out = Point {
            active: true,
            in_span: true,
            gain: 1.0,
            ..Point::default()
        };
        let mut tau = t;
        let last = self.levels.len() - 1;
        for (k, level) in self.levels.iter().enumerate() {
            let comp = &comps[level.comp];
            let Some(layer) = comp.layers().get(level.layer) else {
                return Ok(Point::default());
            };
            let from = number(node::member(Some(layer), "from"), 0.0);
            let dur = number(node::member(Some(layer), "dur"), 0.0).max(0.0);
            if !(tau >= from && tau < from + dur) {
                out.in_span = false;
            }
            let on = node::member(Some(layer), "on");
            if node::is_property(on)
                && ev.ev_p(comp, Some(layer), on, tau, &JsString::from("l.on"))?
                    == JsValue::Bool(false)
            {
                out.active = false;
            }
            for &g in &level.groups {
                let group = &comp.layers()[g];
                let on = node::member(Some(group), "on");
                if node::is_property(on)
                    && !ev
                        .ev_p(comp, Some(group), on, tau, &JsString::from("l.on"))?
                        .truthy()
                {
                    out.active = false;
                }
            }
            let d = node::member(Some(layer), "d");
            // Only the content fields the level needs, each as
            // `resolveContent` evaluates it (a channel through `evP`, any
            // other value as it is).
            let field = |ev: &mut Evaluator, key: &str| -> Result<JsValue> {
                let v = node::member(d, key);
                if node::is_property(v) {
                    Ok(ev.ev_p(
                        comp,
                        Some(layer),
                        v,
                        tau,
                        &JsString::from(format!("c.{key}").as_str()),
                    )?)
                } else {
                    Ok(node::js_value_opt(v))
                }
            };
            if k < last {
                out.gain *= level_of(Some(&field(ev, "audioGain")?)) * level.nested_level;
                if field(ev, "audioMuted")? == JsValue::Bool(true) {
                    out.active = false;
                }
                tau = if self.offset.is_some() {
                    // A linear path: `local × 1 + trim` (sourceTime).
                    let trim = node::js_value_opt(node::member(d, "trim"));
                    (tau - from) * 1.0 + if trim.truthy() { trim.to_number() } else { 0.0 }
                } else {
                    Scope::new(ev, comp, open).source_time(layer, tau, Some(retime))?
                };
                continue;
            }
            out.local = tau - from;
            out.audible = dur;
            match self.kind {
                VoiceKind::Audio => {
                    out.gain *= finite(&field(ev, "gain")?, 1.0).clamp(0.0, 4.0);
                    out.fade_in = finite(&field(ev, "fadeIn")?, 0.0).max(0.0);
                    out.fade_out = finite(&field(ev, "fadeOut")?, 0.0).max(0.0);
                }
                VoiceKind::Embedded => {
                    out.gain *= level_of(Some(&field(ev, "audioGain")?));
                    if field(ev, "audioMuted")? == JsValue::Bool(true) {
                        out.active = false;
                    }
                }
            }
            out.source_time = match self.offset {
                Some(offset) => t - offset,
                None => {
                    Scope::new(ev, comp, open).source_time(layer, tau, Some(retime))? * self.conform
                }
            };
            // Where the source runs out, for linear maps: the clip stops there
            // and the fade out ends there (`audibleDuration`).
            if let Some(len) = self.source.info.duration() {
                let speed = match self.offset {
                    Some(_) => Some(1.0),
                    None => static_number(node::member(node::member(Some(layer), "d"), "speed"))
                        .filter(|s| s.is_finite() && *s > 0.0)
                        .filter(|_| {
                            !node::truthy(node::member(node::member(Some(layer), "d"), "timeRemap"))
                        }),
                };
                if let Some(speed) = speed.map(|s| s * self.conform) {
                    let start = out.source_time - out.local * speed;
                    out.audible = out.audible.min((len - start) / speed);
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fades_follow_gain_at() {
        let p = Point {
            active: true,
            gain: 1.0,
            fade_in: 0.2,
            fade_out: 0.3,
            audible: 2.0,
            ..Point::default()
        };
        assert_eq!(p.fade(0.0), 0.0);
        assert!((p.fade(0.1) - 0.5).abs() < 1e-12);
        assert_eq!(p.fade(1.0), 1.0);
        assert!((p.fade(1.85) - 0.5).abs() < 1e-12);
        assert_eq!(p.fade(2.5), 0.0);
        // Overlapping fades meet.
        let short = Point { audible: 0.3, ..p };
        assert!((short.fade(0.12) - 0.6).abs() < 1e-12);
    }

    #[test]
    fn level_parsing_matches_the_legacy_helpers() {
        assert_eq!(level_of(None), 1.0);
        assert_eq!(level_of(Some(&JsValue::Null)), 1.0);
        assert_eq!(level_of(Some(&JsValue::Number(9.0))), 4.0);
        assert_eq!(finite(&JsValue::Null, 1.0), 0.0);
        assert_eq!(finite(&JsValue::Undefined, 1.0), 1.0);
    }

    fn document(layers: &str, comps: &str) -> Document {
        let text = format!(
            r#"{{"id":"P","compId":"C","w":64,"h":64,"fps":30,"dur":4,"layers":[{layers}],"comps":{{{comps}}},"params":{{}}}}"#
        );
        Document::new(Some(pm_model::json::parse(&text).unwrap()), 1)
    }

    fn layer(id: &str, ty: &str, extra: &str, d: &str) -> String {
        format!(
            r#"{{"id":"{id}","type":"{ty}","from":0,"dur":4,"on":true,"p":{{}},"d":{{{d}}}{extra}}}"#
        )
    }

    fn audible_in(doc: &Document) -> bool {
        audible(doc, "C", &mut Evaluator::new()).unwrap()
    }

    #[test]
    fn audible_finds_the_clips_the_mix_would_play_without_media() {
        let solid = layer("s", "solid", "", r##""color":"#ff0000""##);
        let audio = layer("a", "audio", "", r#""asset":"missing""#);
        // Nothing to play: solids, an audio layer without media or switched
        // off, a video without embedded audio or with it muted.
        assert!(!audible_in(&document(&solid, "")));
        assert!(!audible_in(&document(&layer("a", "audio", "", ""), "")));
        assert!(!audible_in(&document(
            &layer("a", "audio", r#","on":false"#, r#""asset":"x""#),
            ""
        )));
        assert!(!audible_in(&document(
            &layer("v", "video", "", r#""asset":"x""#),
            ""
        )));
        assert!(!audible_in(&document(
            &layer(
                "v",
                "video",
                "",
                r#""asset":"x","embeddedAudio":true,"audioMuted":true"#
            ),
            ""
        )));
        // An audio clip (resolvable or not: no media is opened) and a video's
        // own sound are audible.
        assert!(audible_in(&document(&audio, "")));
        assert!(audible_in(&document(
            &layer("v", "video", "", r#""asset":"x","embeddedAudio":true"#),
            ""
        )));
        // Solo leaves the audio out.
        let soloed = layer("s", "solid", r#","solo":true"#, r##""color":"#ff0000""##);
        assert!(!audible_in(&document(&format!("{soloed},{audio}"), "")));
        // Through a precomp, unless the precomp is muted.
        let nested = r#""N":{"id":"N","w":64,"h":64,"fps":30,"dur":4,"layers":[AUDIO]}"#
            .replace("AUDIO", &audio);
        assert!(audible_in(&document(
            &layer("p", "precomp", "", r#""comp":"N""#),
            &nested
        )));
        assert!(!audible_in(&document(
            &layer("p", "precomp", "", r#""comp":"N","audioMuted":true"#),
            &nested
        )));
    }
}
