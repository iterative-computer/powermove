//! Capability routing (plan section 21; section 26, Phase 2: "behind 'Use
//! New Engine', with capability routing").
//!
//! [`unsupported`] lists every feature of a composition (and every
//! composition it nests) that this renderer cannot draw yet. It is
//! conservative: anything not implemented *and tested* is listed, whether
//! or not it is visible at a particular time, so a routing decision made
//! once per document revision holds at every frame. The answer is static
//! (the document only); [`unsupported_with_media`] also opens media headers
//! for facts the document does not record (animated images, unreadable
//! color profiles). A render that still meets something unsupported fails
//! with [`crate::RenderError::Unsupported`] rather than drawing it wrong.

use std::collections::BTreeSet;
use std::fmt;

use pm_eval::node;
use pm_model::{Document, JsString, Node, NodeKind};

use crate::assets::AssetResolver;
use crate::js;

/// One thing the renderer cannot draw.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Unsupported {
    /// The composition holding it (`None` for request-level features).
    pub composition: Option<String>,
    /// The layer, when the feature belongs to one.
    pub layer: Option<String>,
    pub feature: Feature,
}

/// What is unsupported.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Feature {
    /// A layer type this renderer does not draw (text, captions, ...).
    LayerType(String),
    /// An extension layer whose definition is not the 3D primitive path.
    ExtensionDefinition(String),
    /// Effects (enabled or animatable).
    Effects,
    /// Layer masks.
    Masks,
    /// Transitions.
    Transition,
    /// Layer or group motion blur.
    MotionBlur,
    /// 2.5D layers (the 3D switch or a 3D group).
    TwoAndAHalfD,
    /// Adjustment layers with effects.
    Adjustment,
    /// Content created at `compat` level 2.
    CompatLevel2,
    /// Non-default project color (linear blending, 32-bit project precision).
    ProjectColor,
    /// A project that requires native-engine features.
    RequiresEngine,
    /// Blender-rendered 3D (`render3d.enabled`).
    Blender,
    /// `compat` level 2 SVG images with `<text>` (level 1 SVG images are
    /// Chromium's rasters, decision S1).
    SvgImage,
    /// Animated images and image sequences.
    ImageSequence,
    /// A media file this renderer cannot read (format, color profile).
    Media(String),
    /// A request option (view override, editor looks).
    Request(String),
    /// An effect or transition whose definition the renderer was not given
    /// and that is not marked missing (contract 10: `pm-cli` fails on it).
    Definition(String),
    /// A program WebGL draws but the native pipeline cannot translate (a
    /// shader layer's code or a definition): it needs the decision 19
    /// fallback runner (`crate::shader_layers::fallback`), so it is not
    /// headless-renderable.
    ShaderFallback(String),
    /// Text content this renderer does not draw (color vector glyphs, no
    /// configured fonts).
    Text(String),
}

impl Unsupported {
    pub fn request(what: &str) -> Unsupported {
        Unsupported {
            composition: None,
            layer: None,
            feature: Feature::Request(what.to_owned()),
        }
    }

    pub(crate) fn layer(comp: &str, layer: &Node, feature: Feature) -> Unsupported {
        Unsupported {
            composition: Some(comp.to_owned()),
            layer: Some(js::layer_id(layer)),
            feature,
        }
    }
}

impl fmt::Display for Feature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Feature::LayerType(t) => write!(f, "{t} layers"),
            Feature::ExtensionDefinition(d) => write!(f, "extension layer {d}"),
            Feature::Effects => write!(f, "effects"),
            Feature::Masks => write!(f, "masks"),
            Feature::Transition => write!(f, "transitions"),
            Feature::MotionBlur => write!(f, "motion blur"),
            Feature::TwoAndAHalfD => write!(f, "2.5D layers"),
            Feature::Adjustment => write!(f, "adjustment layers with effects"),
            Feature::CompatLevel2 => write!(f, "compat level 2 content"),
            Feature::ProjectColor => write!(f, "non-default project color settings"),
            Feature::RequiresEngine => write!(f, "native-engine features"),
            Feature::Blender => write!(f, "Blender-rendered 3D"),
            Feature::SvgImage => write!(f, "level 2 SVG images with text"),
            Feature::ImageSequence => write!(f, "animated images and image sequences"),
            Feature::Media(m) => write!(f, "media: {m}"),
            Feature::Request(r) => write!(f, "request: {r}"),
            Feature::Definition(d) => write!(f, "no definition for {d}"),
            Feature::ShaderFallback(d) => write!(f, "{d} needs the GLSL fallback runner"),
            Feature::Text(t) => write!(f, "text: {t}"),
        }
    }
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.feature)?;
        match (&self.composition, &self.layer) {
            (Some(c), Some(l)) => write!(f, " (layer {l} in {c})"),
            (Some(c), None) => write!(f, " (composition {c})"),
            _ => Ok(()),
        }
    }
}

/// Layer types this renderer draws (or knows draw nothing). Text and
/// captions need fonts (`Renderer::set_text_fonts`); without them a render
/// fails with `RenderError::Unsupported`.
const DRAWN: [&str; 8] = [
    "solid", "image", "video", "precomp", "group", "shape", "text", "captions",
];
const NOT_VISUAL: [&str; 2] = ["null", "audio"];

/// The open composition's id: `compId`, else `id`.
pub(crate) fn open_composition_id(root: &Node) -> Option<String> {
    node::string(node::member(Some(root), "compId"))
        .or_else(|| node::string(node::member(Some(root), "id")))
        .map(JsString::to_string_lossy)
}

/// `PM.compOf(L)` against the document: `comps[d.comp]` with a layers
/// array. The legacy `comps` omits the open composition, so a precomp of
/// the open composition resolves to nothing, as today.
pub(crate) fn precomp_target<'d>(root: &'d Node, layer: &Node) -> Option<(String, &'d Node)> {
    if js::layer_type(layer) != "precomp" {
        return None;
    }
    let d = node::member(Some(layer), "d")?;
    let id = node::member(Some(d), "comp").filter(|c| node::truthy(Some(c)))?;
    let id = node::js_value_opt(Some(id))
        .to_js_string()
        .to_string_lossy();
    if open_composition_id(root).as_deref() == Some(id.as_str())
        && node::member(Some(root), "layers").is_some()
    {
        return None;
    }
    let comp = node::member(node::member(Some(root), "comps"), &id)?;
    node::array(node::member(Some(comp), "layers"))?;
    Some((id, comp))
}

/// The layers array of a composition by id (stored or canonical form).
pub(crate) fn composition_node<'d>(root: &'d Node, id: &str) -> Option<&'d Node> {
    let open = open_composition_id(root);
    if open.as_deref() == Some(id) && node::member(Some(root), "layers").is_some() {
        return Some(root);
    }
    node::member(node::member(Some(root), "comps"), id)
        .filter(|c| matches!(c.kind(), NodeKind::Object(_)))
}

fn is_property(v: Option<&Node>) -> bool {
    node::is_property(v)
}

/// Static check of `composition` and everything it nests, for a renderer
/// without fonts (text and captions layers are listed: they need
/// `Renderer::set_text_fonts`).
pub fn unsupported(document: &Document, composition: &str) -> Vec<Unsupported> {
    check(document, composition, None, None, false)
}

/// [`unsupported`] (with media facts when `assets` is given) for a renderer
/// holding these definitions: effects and transitions are drawn, except
/// those with no definition that is not marked missing
/// ([`Feature::Definition`]) and definitions that need the fallback runner
/// ([`Feature::ShaderFallback`]), on layers and adjustment layers alike.
/// Text and captions are listed as for a renderer without fonts.
pub fn unsupported_with_definitions(
    document: &Document,
    composition: &str,
    assets: Option<&dyn AssetResolver>,
    effects: &crate::effects::Effects,
) -> Vec<Unsupported> {
    check(document, composition, assets, Some(effects), false)
}

/// [`unsupported`] plus facts read from media headers through `assets`
/// (animated images, color profiles this renderer cannot convert, files
/// that do not open).
pub fn unsupported_with_media(
    document: &Document,
    composition: &str,
    assets: &dyn AssetResolver,
) -> Vec<Unsupported> {
    check(document, composition, Some(assets), None, false)
}

/// [`unsupported_with_definitions`] for a renderer with fonts configured
/// (`Renderer::set_text_fonts`): text and captions layers are drawn.
pub fn unsupported_with_definitions_and_fonts(
    document: &Document,
    composition: &str,
    assets: Option<&dyn AssetResolver>,
    effects: &crate::effects::Effects,
) -> Vec<Unsupported> {
    check(document, composition, assets, Some(effects), true)
}

/// [`unsupported_with_media`] for a renderer with fonts configured
/// (`Renderer::set_text_fonts`): text and captions layers are drawn
/// (`Renderer::unsupported_with_media` picks the right check).
pub fn unsupported_with_fonts(
    document: &Document,
    composition: &str,
    assets: Option<&dyn AssetResolver>,
) -> Vec<Unsupported> {
    check(document, composition, assets, None, true)
}

/// The check for a renderer as configured: media facts when `assets` is
/// given, effects, transitions, and extension layers against `effects`
/// when the renderer holds definitions (`Effects::configured`), and text
/// drawn when it has fonts. What the job engine, stills, and `pm-cli` route
/// with (`Renderer::unsupported_with_media`).
pub fn unsupported_for(
    document: &Document,
    composition: &str,
    assets: Option<&dyn AssetResolver>,
    effects: Option<&crate::effects::Effects>,
    fonts: bool,
) -> Vec<Unsupported> {
    check(
        document,
        composition,
        assets,
        effects.filter(|e| e.configured()),
        fonts,
    )
}

fn check(
    document: &Document,
    composition: &str,
    assets: Option<&dyn AssetResolver>,
    effects: Option<&crate::effects::Effects>,
    fonts: bool,
) -> Vec<Unsupported> {
    let mut out = BTreeSet::new();
    let Some(root) = document.root() else {
        return vec![];
    };
    // Project-level settings.
    if crate::composite::project_color(root).is_err() {
        out.insert(Unsupported {
            composition: None,
            layer: None,
            feature: Feature::ProjectColor,
        });
    }
    // The stamp (section 6) only tells the legacy engine to refuse the
    // project: every native-only feature it stands for is drawn here or
    // judged by its own check ([`STAMP_FIELDS`]), so it routes nothing.
    // `Feature::RequiresEngine` remains for a stamping field no lane draws;
    // the classification test keeps that list empty.
    if let Some(r) = node::member(Some(root), "requiresEngine")
        && node::truthy(Some(r))
        && !native_features_drawn(root)
    {
        out.insert(Unsupported {
            composition: None,
            layer: None,
            feature: Feature::RequiresEngine,
        });
    }
    // Each composition is checked once per context: `sampled` is true when
    // it is reached through a blurred precomp or a precomp inside a blurred
    // group, so its layers are drawn at every shutter sample time.
    let mut seen = BTreeSet::new();
    let mut stack = vec![(composition.to_owned(), false)];
    while let Some((id, sampled)) = stack.pop() {
        if !seen.insert((id.clone(), sampled)) {
            continue;
        }
        let Some(comp) = composition_node(root, &id) else {
            continue;
        };
        check_composition(
            root, &id, comp, sampled, assets, effects, fonts, &mut out, &mut stack,
        );
    }
    out.into_iter().collect()
}

#[allow(clippy::too_many_arguments)]
fn check_composition(
    root: &Node,
    comp_id: &str,
    comp: &Node,
    sampled: bool,
    assets: Option<&dyn AssetResolver>,
    effects: Option<&crate::effects::Effects>,
    fonts: bool,
    out: &mut BTreeSet<Unsupported>,
    stack: &mut Vec<(String, bool)>,
) {
    let layers = node::array(node::member(Some(comp), "layers")).unwrap_or(&[]);
    let mut add = |layer: &Node, feature: Feature| {
        out.insert(Unsupported::layer(comp_id, layer, feature));
    };
    let mut has_3d = false;
    let groups = GroupIndex::new(layers);
    for layer in layers {
        let ty = js::layer_type(layer);
        if !DRAWN.contains(&ty.as_str()) && !NOT_VISUAL.contains(&ty.as_str()) {
            match ty.as_str() {
                "extension" => {
                    let def = node::member(node::member(Some(layer), "d"), "definition")
                        .map(|d| node::js_value_opt(Some(d)).to_js_string().to_string_lossy())
                        .unwrap_or_default();
                    match def.as_str() {
                        pm_eval::parenting::Role3d::OBJECT => {
                            has_3d = true;
                        }
                        pm_eval::parenting::Role3d::LIGHT | pm_eval::parenting::Role3d::CAMERA => {
                            has_3d = true;
                        }
                        // Legacy scene layers: the 3D pass renders them whole,
                        // so their embedded scene's maps meet the image checks.
                        crate::scene3d::legacy::SCENE3D_DEFINITION => {
                            for feature in crate::scene3d::check_legacy_scene(root, layer, assets) {
                                add(layer, feature);
                            }
                        }
                        // The built-in OBJ model layer, with its fixed shading.
                        crate::scene3d::legacy::OBJ_MODEL_DEFINITION => {}
                        // Fragment layers, mesh layers, and missing
                        // definitions need the definition set
                        // (`crate::shader_layers`).
                        _ => match effects {
                            Some(e) => {
                                if let Some(f) =
                                    crate::shader_layers::plan::extension_feature(e, &def)
                                {
                                    add(layer, f);
                                }
                            }
                            None => add(layer, Feature::ExtensionDefinition(def)),
                        },
                    }
                }
                // Drawn through the GLSL pipeline; code only the fallback
                // runner can draw is listed (`Feature::ShaderFallback`).
                "shader" => {
                    if let Some(f) = crate::shader_layers::plan::shader_layer_feature(layer) {
                        add(layer, f);
                    }
                }
                "adjustment" => {
                    if has_effects(layer) {
                        match effects {
                            None => add(layer, Feature::Adjustment),
                            Some(e) => {
                                for f in crate::effects::layer_features(e, layer) {
                                    add(layer, f);
                                }
                            }
                        }
                    }
                }
                other => add(layer, Feature::LayerType(other.to_owned())),
            }
        }
        if has_effects(layer) && ty != "adjustment" {
            match effects {
                None => add(layer, Feature::Effects),
                Some(e) => {
                    for f in crate::effects::layer_features(e, layer) {
                        add(layer, f);
                    }
                }
            }
        }
        // Parametric and bezier path masks (also on 3D object layers),
        // track mattes, and blend modes are drawn (`crate::composite`).
        // Masks that are not an array fail the layer as today's
        // `(L.masks || []).some(…)` does (a named layer failure, not a
        // route).
        if js::flag(layer, "transitionIn") || js::flag(layer, "transitionOut") {
            match effects {
                None => add(layer, Feature::Transition),
                Some(e) => {
                    for f in crate::effects::transition_features(e, layer) {
                        add(layer, f);
                    }
                }
            }
        }
        // Motion blur and 2.5D planes are drawn for every visual layer type
        // (`crate::motion::DRAWN_TYPES`, `crate::space25d::DRAWN_TYPES`,
        // each compared with today's in the corpus; docs/engine/motion.md).
        // A type missing from those lists is a new layer type and routes.
        let mblur = node::member(Some(layer), "mblur");
        let own_blur = is_property(mblur) || node::truthy(mblur);
        if own_blur && !crate::motion::DRAWN_TYPES.contains(&ty.as_str()) {
            add(layer, Feature::MotionBlur);
        }
        // 3D object, light, and camera layers draw in screen space; their
        // 3D switch does not make them 2.5D planes. Nulls and audio draw
        // nothing (a 3D null only places its children). A 3D group's
        // members are planes of its space; each member is checked itself.
        if js::flag(layer, "threeD")
            && pm_eval::parenting::Role3d::of(layer).is_none()
            && !NOT_VISUAL.contains(&ty.as_str())
            && !crate::space25d::DRAWN_TYPES.contains(&ty.as_str())
        {
            add(layer, Feature::TwoAndAHalfD);
        }
        // Members of a 3D group are placed in its space, members of a
        // blurred group are sampled with it, and the layers of a composition
        // reached through a blurred precomp are drawn at every sample time:
        // types outside the compared lists keep routing.
        let (three_d, blurred) = enclosing_groups(&groups, layer);
        if !NOT_VISUAL.contains(&ty.as_str()) {
            if three_d && !crate::space25d::DRAWN_TYPES.contains(&ty.as_str()) {
                add(layer, Feature::TwoAndAHalfD);
            }
            if (blurred || sampled) && !crate::motion::DRAWN_TYPES.contains(&ty.as_str()) {
                add(layer, Feature::MotionBlur);
            }
        }
        // Level 2 content of every layer type is drawn: shapes, solids, and
        // SVG images (`crate::vector`), image, video, precomp, shader, and
        // extension raster content resampled (`crate::sampling`), text and
        // captions at output resolution (`crate::text`, which names the
        // level 2 text it does not lay out yet), and layers that draw
        // nothing of their own.
        if node::member(Some(layer), "compat").is_some_and(|c| node::to_number(Some(c)) >= 2.0)
            && ty == "text"
            && let Some(why) = level2_text_gap(layer)
        {
            add(layer, Feature::Text(why.into()));
        }
        if !fonts && matches!(ty.as_str(), "text" | "captions") {
            add(
                layer,
                Feature::Text(format!("{ty} layers without the engine's fonts")),
            );
        }
        match ty.as_str() {
            "image" | "video" => check_media(root, layer, &ty, assets, &mut add),
            "precomp" => {
                if let Some((id, _)) = precomp_target(root, layer) {
                    stack.push((id, sampled || own_blur || blurred));
                }
            }
            _ => {}
        }
    }
    if has_3d {
        for (layer, feature) in crate::scene3d::check(root, comp, layers, assets) {
            out.insert(Unsupported {
                composition: Some(comp_id.to_owned()),
                layer,
                feature,
            });
        }
        if node::member(Some(comp), "render3d")
            .and_then(|r| node::member(Some(r), "enabled"))
            .is_some_and(|e| node::truthy(Some(e)))
        {
            out.insert(Unsupported {
                composition: Some(comp_id.to_owned()),
                layer: None,
                feature: Feature::Blender,
            });
        }
    }
}

/// Whether every native-only feature the document uses (the
/// `requiresEngine` stamp's reasons, `shared/engine-stamp.ts`; the section 6
/// field table, [`pm_protocol::patch::FIELD_TABLE`]) is one this engine
/// draws. The stamp carries no reasons, so each field that stamps is
/// classified ([`STAMP_FIELDS`]):
///
/// - drawn here: linked media (`storage: "linked"` or `link`), Interpret
///   Footage overrides (`interpret`), and precomp `keepFrameRate`
///   (`crate::retime::keep_frame_rate`);
/// - judged elsewhere: the project `color` ([`Feature::ProjectColor`] lists
///   only settings this renderer does not know) and clip `pitch` (audio,
///   `pm-audio`).
///
/// A stamp without any of them (a project whose only native-only setting is
/// its color, or one stamped before its feature was removed) is accepted:
/// the stamp is the legacy engine's refusal, not a capability.
fn native_features_drawn(root: &Node) -> bool {
    // Every stamping field not drawn here is set (`true`, or any value
    // other than absent, false, or null) nowhere in the document.
    let set = |v: &Node| !matches!(v.kind(), NodeKind::Null | NodeKind::Bool(false));
    !STAMP_FIELDS
        .iter()
        .any(|&(_, field, class)| class == StampField::NotDrawn && any_member(root, field, &set))
}

/// How [`native_features_drawn`] treats each field of the section 6 table
/// that stamps the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StampField {
    Drawn,
    JudgedElsewhere,
    /// A native-only field no lane draws: a document that sets it routes
    /// ([`Feature::RequiresEngine`]). None today.
    #[allow(dead_code)]
    NotDrawn,
}

/// Every stamping field of [`pm_protocol::patch::FIELD_TABLE`], classified
/// (a test keeps the two lists equal, so a new native-only field must be
/// classified here before a stamp carrying it is accepted).
const STAMP_FIELDS: [(&str, &str, StampField); 6] = [
    ("asset", "link", StampField::Drawn),
    ("asset", "interpret", StampField::Drawn),
    ("asset", "storage", StampField::Drawn),
    ("precomp", "keepFrameRate", StampField::Drawn),
    ("clip", "pitch", StampField::JudgedElsewhere),
    ("project", "color", StampField::JudgedElsewhere),
];

/// Whether any object member named `name` anywhere under `n` satisfies
/// `test`.
fn any_member(n: &Node, name: &str, test: &dyn Fn(&Node) -> bool) -> bool {
    match n.kind() {
        NodeKind::Object(map) => map
            .iter()
            .any(|(k, v)| (k.to_string_lossy() == name && test(v)) || any_member(v, name, test)),
        NodeKind::Array(list) => list.iter().any(|v| any_member(v, name, test)),
        _ => false,
    }
}

/// What `crate::text` does not lay out at `compat` level 2 yet (its
/// `text_layer_level2` checks): animated or range-styled text and
/// typography anchoring.
fn level2_text_gap(layer: &Node) -> Option<&'static str> {
    let d = node::member(Some(layer), "d");
    let has = |key: &str| {
        node::member(d, key)
            .map(|n| node::get(n, b"length").truthy())
            .unwrap_or(false)
    };
    if has("animators") || has("styles") {
        return Some("animated or range-styled text at compat level 2");
    }
    if node::member(d, "fontAnchorBounds").is_some_and(|b| node::truthy(Some(b))) {
        return Some("typography anchoring at compat level 2");
    }
    None
}

/// Whether any group enclosing `layer` (its `group` chain within `layers`)
/// has the 3D switch, and whether any has motion blur (on or animated).
/// The group layers of one composition by id (the first group layer with
/// each id, as a scan in layer order finds it): what [`enclosing_groups`]
/// walks, built once per composition rather than scanned per layer (a
/// 500-layer composition's check was 7.8 ms of the presenter's per-edit
/// work; docs/engine/perf.md).
struct GroupIndex<'a> {
    groups: std::collections::HashMap<String, &'a Node>,
    layers: usize,
}

impl<'a> GroupIndex<'a> {
    fn new(layers: &'a [Node]) -> Self {
        let mut groups = std::collections::HashMap::new();
        for l in layers {
            if js::layer_type(l) == "group" {
                groups.entry(js::layer_id(l)).or_insert(l);
            }
        }
        GroupIndex {
            groups,
            layers: layers.len(),
        }
    }
}

/// Whether a layer sits in a 3D group or a motion-blurred group, through
/// any depth of group membership.
fn enclosing_groups(index: &GroupIndex<'_>, layer: &Node) -> (bool, bool) {
    let id_of = |n: Option<&Node>| {
        n.filter(|g| node::truthy(Some(g)))
            .map(|g| node::js_value_opt(Some(g)).to_js_string().to_string_lossy())
    };
    let (mut three_d, mut blurred) = (false, false);
    let mut next = id_of(node::member(Some(layer), "group"));
    // A cycle in group membership ends at the layer count.
    for _ in 0..index.layers {
        let Some(id) = next else { break };
        let Some(group) = index.groups.get(&id).copied() else {
            break;
        };
        three_d |= js::flag(group, "threeD");
        let mblur = node::member(Some(group), "mblur");
        blurred |= is_property(mblur) || node::truthy(mblur);
        next = id_of(node::member(Some(group), "group"));
    }
    (three_d, blurred)
}

/// `L.fx` holds an effect that is not marked missing (enabled or not: the
/// enable is animatable).
fn has_effects(layer: &Node) -> bool {
    node::array(node::member(Some(layer), "fx")).is_some_and(|fx| {
        fx.iter().any(|e| {
            !matches!(
                node::member(Some(e), "missing").map(Node::kind),
                Some(NodeKind::Bool(true))
            )
        })
    })
}

fn check_media(
    root: &Node,
    layer: &Node,
    ty: &str,
    assets: Option<&dyn AssetResolver>,
    add: &mut impl FnMut(&Node, Feature),
) {
    let d = node::member(Some(layer), "d");
    let Some(asset_id) = node::member(d, "asset")
        .map(|a| node::js_value_opt(Some(a)).to_js_string().to_string_lossy())
    else {
        return;
    };
    let record = node::member(node::member(Some(root), "assets"), &asset_id);
    let Some(record) = record else {
        // A missing asset draws nothing, as today.
        return;
    };
    let format = js::member_string(record, "format")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if ty == "image" && format == "svg" {
        // Level 1 SVG images are drawn: Chromium's raster at today's
        // bucket, from the document's renderer (decision S1; broken and
        // unreadable documents are handled at execution).
        if !crate::vector::level2(layer) {
            return;
        }
        // Level 2: unreadable SVGs are named layer failures at execution.
        let has_text = match assets {
            Some(assets) => {
                let Some(path) = assets.path(&asset_id, Some(record)) else {
                    return;
                };
                match crate::vector::svg::document(&path) {
                    Ok((doc, _)) => doc.has_text(),
                    Err(_) => return,
                }
            }
            None => false,
        };
        if has_text {
            add(layer, Feature::SvgImage);
        }
        return;
    }
    if ty == "video"
        && (node::member(Some(record), "imageSequence").is_some_and(|s| node::truthy(Some(s)))
            || matches!(format.as_str(), "gif" | "webp" | "png" | "apng" | "avif"))
    {
        // Existing imports persist a WebM rendition: that rendition is
        // the shipped appearance and uses the tested video path. Original
        // animation containers still need an explicit import policy.
        match assets.and_then(|a| a.path(&asset_id, Some(record))) {
            Some(path) => match crate::media::sequence_rendition(&path) {
                Ok(true) => {}
                Ok(false) => {
                    add(layer, Feature::ImageSequence);
                    return;
                }
                Err(_) => return, // Execution reports the unreadable media.
            },
            None if assets.is_some() => return, // Offline: named layer failure.
            None => {
                add(layer, Feature::ImageSequence);
                return;
            }
        }
    }
    let Some(assets) = assets else {
        // Without media, the recorded container decides (the tested still
        // classes need the file; see `unsupported_with_media`).
        if ty == "image" && format != "svg" {
            let name = js::member_string(record, "name").unwrap_or_default();
            if let Err(f) = crate::stills::static_format(&format, &name) {
                add(layer, f);
            }
        }
        return;
    };
    let Some(path) = assets.path(&asset_id, Some(record)) else {
        return;
    };
    match ty {
        "image" => match crate::footage::interpretation(record).map_or_else(
            || {
                crate::media::probe_still(
                    &path,
                    crate::media::still_reader(!crate::sampling::is_level2(layer)),
                )
            },
            |i| crate::media::probe_interpreted_image(&path, &i),
        ) {
            Ok(()) => {}
            Err(crate::media::ProbeIssue::Unsupported(f)) => add(layer, f),
            // Execution reports an unreadable source as a named layer
            // failure. It is not an unsupported rendering capability.
            Err(crate::media::ProbeIssue::Unreadable(_)) => {}
        },
        _ => {
            let probed = match crate::footage::interpretation(record) {
                Some(i) => crate::media::probe_interpreted_video_issue(&path, &i),
                None => crate::media::probe_video_issue(&path),
            };
            if let Err(crate::media::ProbeIssue::Unsupported(feature)) = probed {
                add(layer, feature);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::patch::{FIELD_TABLE, LegacyBehavior};

    #[test]
    fn every_stamping_field_is_classified() {
        let mut table: Vec<(&str, &str)> = FIELD_TABLE
            .iter()
            .filter(|e| {
                matches!(
                    e.legacy,
                    LegacyBehavior::RequiresNative | LegacyBehavior::RequiresNativeWhen
                )
            })
            .map(|e| (e.owner, e.field))
            .collect();
        let mut ours: Vec<(&str, &str)> = STAMP_FIELDS.iter().map(|&(o, f, _)| (o, f)).collect();
        table.sort();
        ours.sort();
        assert_eq!(ours, table);
    }

    fn stamped(extra: serde_json::Value) -> Document {
        let mut doc = serde_json::json!({
            "id": "root", "w": 64, "h": 64, "fps": 25, "dur": 1, "layers": [],
            "requiresEngine": "native",
            "assets": { "a": { "type": "video", "interpret": { "fieldOrder": "topFirst" } } },
        });
        if let (Some(d), serde_json::Value::Object(e)) = (doc.as_object_mut(), extra) {
            d.extend(e);
        }
        Document::new(Some(pm_model::json::parse(&doc.to_string()).unwrap()), 1)
    }

    #[test]
    fn every_stamp_reason_is_drawn() {
        let drawn = |doc: &Document| native_features_drawn(doc.root().unwrap());
        assert!(drawn(&stamped(serde_json::json!({}))));
        // keepFrameRate is drawn (`crate::retime::keep_frame_rate`).
        let precomp = serde_json::json!({ "comps": { "c": { "layers": [
            { "id": "p", "type": "precomp", "d": { "comp": "x", "keepFrameRate": true } }
        ] } } });
        assert!(drawn(&stamped(precomp)));
        // A stamp whose only reason is the project color (linear blending)
        // or clip pitch is drawn: those are judged by their own checks.
        let color =
            stamped(serde_json::json!({ "assets": {}, "color": { "linearBlending": true } }));
        assert!(drawn(&color));
        let doc = color;
        assert!(
            super::unsupported(&doc, "root").is_empty(),
            "{:?}",
            super::unsupported(&doc, "root")
        );
        let pitch = serde_json::json!({ "assets": {}, "layers": [
            { "id": "a", "type": "audio", "d": { "pitch": "follow" } }
        ] });
        assert!(drawn(&stamped(pitch)));
        assert!(drawn(&stamped(serde_json::json!({ "assets": {} }))));
        // Invalid project color is still its own named feature.
        let bad = stamped(serde_json::json!({ "color": { "mode": "nonsense" } }));
        assert_eq!(
            super::unsupported(&bad, "root")
                .iter()
                .map(|u| u.feature.clone())
                .collect::<Vec<_>>(),
            vec![Feature::ProjectColor]
        );
    }

    /// The scan [`GroupIndex`] replaced: the first group layer with the id,
    /// found again for every step of every layer.
    fn enclosing_by_scan(layers: &[Node], layer: &Node) -> (bool, bool) {
        let id_of = |n: Option<&Node>| {
            n.filter(|g| node::truthy(Some(g)))
                .map(|g| node::js_value_opt(Some(g)).to_js_string().to_string_lossy())
        };
        let (mut three_d, mut blurred) = (false, false);
        let mut next = id_of(node::member(Some(layer), "group"));
        for _ in 0..layers.len() {
            let Some(id) = next else { break };
            let Some(group) = layers
                .iter()
                .find(|l| js::layer_id(l) == id && js::layer_type(l) == "group")
            else {
                break;
            };
            three_d |= js::flag(group, "threeD");
            let mblur = node::member(Some(group), "mblur");
            blurred |= is_property(mblur) || node::truthy(mblur);
            next = id_of(node::member(Some(group), "group"));
        }
        (three_d, blurred)
    }

    #[test]
    fn the_group_index_walks_membership_as_the_scan_did() {
        let layers = serde_json::json!([
            // A non-group layer sharing a group's id comes first: skipped.
            { "id": "g1", "type": "solid" },
            { "id": "a", "type": "solid", "group": "g1" },
            { "id": "g1", "type": "group", "group": "g2" },
            { "id": "g2", "type": "group", "threeD": true },
            // A second group with the same id: the first one wins.
            { "id": "g2", "type": "group", "mblur": true },
            { "id": "b", "type": "solid", "group": "g3" },
            { "id": "g3", "type": "group", "mblur": { "v": 1, "kf": [] }, "group": "g4" },
            // A membership cycle ends at the layer count.
            { "id": "g4", "type": "group", "group": "g3" },
            { "id": "c", "type": "solid", "group": "missing" },
            { "id": "d", "type": "solid", "group": 7 },
            { "id": 7, "type": "group", "threeD": true },
            { "id": "e", "type": "solid", "group": "" },
            { "id": "f", "type": "solid" }
        ]);
        let doc = Document::new(
            Some(
                pm_model::json::parse(&serde_json::json!({ "layers": layers }).to_string())
                    .unwrap(),
            ),
            1,
        );
        let layers = node::array(node::member(doc.root(), "layers")).unwrap();
        let index = GroupIndex::new(layers);
        for layer in layers {
            assert_eq!(
                enclosing_groups(&index, layer),
                enclosing_by_scan(layers, layer),
                "{}",
                js::layer_id(layer)
            );
        }
        let at = |i: usize| enclosing_groups(&index, &layers[i]);
        assert_eq!(at(1), (true, false));
        assert_eq!(at(5), (false, true));
        assert_eq!(at(8), (false, false));
        assert_eq!(at(9), (true, false));
    }
}
