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
    /// A layer type this renderer does not draw (shape, text, captions,
    /// shader, extension fragment or mesh, legacy `scene3d`, ...).
    LayerType(String),
    /// An extension layer whose definition is not the 3D primitive path.
    ExtensionDefinition(String),
    /// A 3D object, light, camera, or world setting outside the Phase 2
    /// primitive (one primitive, a sun, no shadows).
    Scene3d(String),
    /// Effects (enabled or animatable).
    Effects,
    /// Layer masks.
    Masks,
    /// Bezier path masks (today's Canvas2D coverage route, which also takes
    /// every other mask of the layer).
    PathMasks,
    /// A track matte, or a layer used as one.
    TrackMatte,
    /// Transitions.
    Transition,
    /// A blend mode other than normal (or an animated blend mode).
    BlendMode(String),
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
    /// SVG images.
    SvgImage,
    /// Animated images and image sequences.
    ImageSequence,
    /// A media file this renderer cannot read (format, color profile).
    Media(String),
    /// A request option (view override, editor looks).
    Request(String),
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
            Feature::Scene3d(s) => write!(f, "3D: {s}"),
            Feature::Effects => write!(f, "effects"),
            Feature::Masks => write!(f, "masks"),
            Feature::PathMasks => write!(f, "path masks"),
            Feature::TrackMatte => write!(f, "track mattes"),
            Feature::Transition => write!(f, "transitions"),
            Feature::BlendMode(m) => write!(f, "blend mode {m}"),
            Feature::MotionBlur => write!(f, "motion blur"),
            Feature::TwoAndAHalfD => write!(f, "2.5D layers"),
            Feature::Adjustment => write!(f, "adjustment layers with effects"),
            Feature::CompatLevel2 => write!(f, "compat level 2 content"),
            Feature::ProjectColor => write!(f, "non-default project color settings"),
            Feature::RequiresEngine => write!(f, "native-engine features"),
            Feature::Blender => write!(f, "Blender-rendered 3D"),
            Feature::SvgImage => write!(f, "SVG images"),
            Feature::ImageSequence => write!(f, "animated images and image sequences"),
            Feature::Media(m) => write!(f, "media: {m}"),
            Feature::Request(r) => write!(f, "request: {r}"),
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

/// Layer types this renderer draws (or knows draw nothing).
const DRAWN: [&str; 5] = ["solid", "image", "video", "precomp", "group"];
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

/// Static check of `composition` and everything it nests.
pub fn unsupported(document: &Document, composition: &str) -> Vec<Unsupported> {
    check(document, composition, None)
}

/// [`unsupported`] plus facts read from media headers through `assets`
/// (animated images, color profiles this renderer cannot convert, files
/// that do not open).
pub fn unsupported_with_media(
    document: &Document,
    composition: &str,
    assets: &dyn AssetResolver,
) -> Vec<Unsupported> {
    check(document, composition, Some(assets))
}

fn check(
    document: &Document,
    composition: &str,
    assets: Option<&dyn AssetResolver>,
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
    // The stamp (section 6) marks native-only features. The ones this
    // renderer draws are linked media and Interpret Footage overrides; a
    // stamp with neither stands for a feature it does not know.
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
    let mut seen = BTreeSet::new();
    let mut stack = vec![composition.to_owned()];
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(comp) = composition_node(root, &id) else {
            continue;
        };
        check_composition(root, &id, comp, assets, &mut out, &mut stack);
    }
    out.into_iter().collect()
}

fn check_composition(
    root: &Node,
    comp_id: &str,
    comp: &Node,
    assets: Option<&dyn AssetResolver>,
    out: &mut BTreeSet<Unsupported>,
    stack: &mut Vec<String>,
) {
    let layers = node::array(node::member(Some(comp), "layers")).unwrap_or(&[]);
    let mut add = |layer: &Node, feature: Feature| {
        out.insert(Unsupported::layer(comp_id, layer, feature));
    };
    let mut objects = 0usize;
    let mut has_3d = false;
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
                            objects += 1;
                            has_3d = true;
                        }
                        pm_eval::parenting::Role3d::LIGHT | pm_eval::parenting::Role3d::CAMERA => {
                            has_3d = true;
                        }
                        _ => add(layer, Feature::ExtensionDefinition(def)),
                    }
                }
                "adjustment" => {
                    if has_effects(layer) {
                        add(layer, Feature::Adjustment);
                    }
                }
                other => add(layer, Feature::LayerType(other.to_owned())),
            }
        }
        if has_effects(layer) && ty != "adjustment" {
            add(layer, Feature::Effects);
        }
        // Parametric masks, track mattes, and blend modes are drawn
        // (`crate::composite`); bezier path masks and masks that are not an
        // array are not.
        let masks = node::member(Some(layer), "masks");
        if is_property(masks) || (node::truthy(masks) && node::array(masks).is_none()) {
            add(layer, Feature::Masks);
        }
        // Masks on 3D object layers (today's slow path turns the scene
        // pass; not matched yet).
        if pm_eval::parenting::Role3d::of(layer).is_some()
            && node::array(masks).is_some_and(|m| !m.is_empty())
        {
            add(layer, Feature::Masks);
        }
        if node::array(masks).is_some_and(|m| {
            m.iter()
                .any(|mask| node::truthy(node::member(Some(mask), "path")))
        }) {
            add(layer, Feature::PathMasks);
        }
        if js::flag(layer, "transitionIn") || js::flag(layer, "transitionOut") {
            add(layer, Feature::Transition);
        }
        let mblur = node::member(Some(layer), "mblur");
        if is_property(mblur) || node::truthy(mblur) {
            add(layer, Feature::MotionBlur);
        }
        // 3D object, light, and camera layers draw in screen space; their
        // 3D switch does not make them 2.5D planes. Nulls and audio draw
        // nothing (a 3D null only places its children). A 3D group is
        // listed itself, which covers its members.
        if js::flag(layer, "threeD")
            && pm_eval::parenting::Role3d::of(layer).is_none()
            && !NOT_VISUAL.contains(&ty.as_str())
        {
            add(layer, Feature::TwoAndAHalfD);
        }
        if node::member(Some(layer), "compat").is_some_and(|c| node::to_number(Some(c)) >= 2.0) {
            add(layer, Feature::CompatLevel2);
        }
        if ty == "group" && js::flag(node::member(Some(layer), "d").unwrap_or(layer), "modeling") {
            add(layer, Feature::Scene3d("model groups".into()));
        }
        match ty.as_str() {
            "image" | "video" => check_media(root, layer, &ty, assets, &mut add),
            "precomp" => {
                if let Some((id, _)) = precomp_target(root, layer) {
                    stack.push(id);
                }
            }
            _ => {}
        }
    }
    if has_3d {
        for issue in crate::scene3d::check(root, comp, layers, objects) {
            out.insert(Unsupported {
                composition: Some(comp_id.to_owned()),
                layer: issue.0,
                feature: Feature::Scene3d(issue.1),
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

/// Whether the document's native-only features (the `requiresEngine`
/// stamp's reasons, `shared/engine-stamp.ts`) are ones this renderer
/// draws: linked media (`storage: "linked"` or `link`) and Interpret
/// Footage overrides (`interpret`). False when the stamp has neither reason.
fn native_features_drawn(root: &Node) -> bool {
    let Some(assets) = node::member(Some(root), "assets") else {
        return false;
    };
    let mut any = false;
    let mut visit = |asset: &Node| {
        let linked = js::member_string(asset, "storage").as_deref() == Some("linked")
            || node::member(Some(asset), "link").is_some_and(|l| node::truthy(Some(l)));
        let interpreted =
            node::member(Some(asset), "interpret").is_some_and(|i| node::truthy(Some(i)));
        any |= linked || interpreted;
    };
    match assets.kind() {
        NodeKind::Object(map) => map.values().for_each(&mut visit),
        NodeKind::Array(list) => list.iter().for_each(&mut visit),
        _ => {}
    }
    any
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
        add(layer, Feature::SvgImage);
    }
    if ty == "video"
        && (node::member(Some(record), "imageSequence").is_some_and(|s| node::truthy(Some(s)))
            || matches!(format.as_str(), "gif" | "webp" | "png" | "apng" | "avif"))
    {
        add(layer, Feature::ImageSequence);
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
            || crate::media::probe_image(&path),
            |i| crate::media::probe_interpreted_image(&path, &i),
        ) {
            Ok(()) => {}
            Err(crate::media::ProbeIssue::Unsupported(f)) => add(layer, f),
            Err(crate::media::ProbeIssue::Unreadable(m)) => add(layer, Feature::Media(m)),
        },
        _ => {
            let probed = match crate::footage::interpretation(record) {
                Some(i) => crate::media::probe_interpreted_video(&path, &i),
                None => crate::media::probe_video(&path),
            };
            if let Err(m) = probed {
                add(layer, Feature::Media(m));
            }
        }
    }
}
