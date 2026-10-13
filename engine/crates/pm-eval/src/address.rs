//! Property addressing, ported from `legacy/core/anim.ts` (`PM.findProp`,
//! `PM.allProps`) with the collectors it calls:
//! `structuredProperties` (`legacy/core/vector-paths.ts`), `sceneProperties`
//! (`core/scene3d/schema.ts`), `layer3DProperties` (`core/scene3d/layers.ts`),
//! and `canAnimateContent` (`legacy/core/content-properties.ts`).
//!
//! # Key namespaces
//!
//! | Key | Property |
//! |---|---|
//! | `position.x`, `opacity`, ... (any `L.p` member) | transform channels |
//! | `c.<field>` | content channel `L.d[field]` |
//! | `l.blend`, `l.mblur`, `l.matteMode` | layer switches |
//! | `<fxId>.<param>`, `<fxId>.$enabled` | effect parameters and the effect's enable |
//! | `m.<maskId>.<param>` | mask parameters (`m.p`) and `shape`, `mode`, `on` |
//! | `u.<name>` | shader uniforms |
//! | `x.<name>` | extension-layer parameters |
//! | `transitionIn.p.<name>`, `transitionOut.p.<name>` | transition parameters |
//! | `g.<pathId>.<param>`, `g.<pathId>.v.<vertexId>.<param>` | vector path groups and vertices |
//! | `mp.<maskId>.<param>`, `mp.<maskId>.v.<vertexId>.<param>` | mask paths |
//! | `ta.<animatorId>.<param>` | text animators |
//! | `ts.<rangeId>.<param>` | text style ranges |
//! | `m.<param>` | 3D material (object layers) |
//! | `shader.<param>`, `slots.<slotId>.<param>`, `slots.<slotId>.shader.<param>` | 3D shader and material slots |
//! | `light.<param>`, `camera.<param>`, `environment.<param>`, `model.<param>` | 3D light, camera, environment, generated model |
//! | `o.<objectId>.<param>`, `o.<objectId>.m.<param>`, `light.<lightId>.<param>` | legacy `scene3d` layers |
//!
//! [`find_prop`] resolves a key to the stored value (normally a channel
//! object `{ v, kf, expr }`); [`all_props`] lists every animatable channel of
//! a layer with its label and group.
//!
//! Throws where the legacy code throws (a `masks` member that is not an
//! array, a `null` mask, ...). The listing order of [`all_props`] follows
//! the legacy order except for non-integer member names inside one object
//! (see [`crate::node`]).

use pm_model::{JsString, Node};

use crate::js_sem::{self, JsResult, JsValue};
use crate::node::{self, Got, Item};

/// One entry of [`all_props`].
#[derive(Clone, Debug, PartialEq)]
pub struct PropEntry<'a> {
    /// The address (`position.x`, `c.color`, `fx1.amount`, ...).
    pub key: JsString,
    /// The stored value at the address (not necessarily a channel object:
    /// `L.p` members are listed as they are).
    pub prop: Got<'a>,
    pub label: JsValue,
    pub group: JsValue,
}

/// Labels and groups for extension-layer parameters (`PM.layerDefinition`),
/// which live in the extension registry, not the document.
pub trait DefinitionLabels {
    /// `(definition label, parameter label)` for an extension layer's
    /// definition value and parameter key.
    fn extension_labels(
        &self,
        definition: &JsValue,
        param: &JsString,
    ) -> (Option<JsValue>, Option<JsValue>);
}

/// No registry: groups are `Extension`, labels the keys.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoDefinitions;

impl DefinitionLabels for NoDefinitions {
    fn extension_labels(&self, _: &JsValue, _: &JsString) -> (Option<JsValue>, Option<JsValue>) {
        (None, None)
    }
}

/// `contentFields` (`legacy/core/content-properties.ts`).
pub fn content_fields(layer_type: &[u8]) -> &'static [&'static str] {
    match layer_type {
        b"text" => &[
            "text",
            "font",
            "weight",
            "size",
            "tracking",
            "leading",
            "align",
            "color",
            "italic",
            "boxWidth",
            "boxHeight",
        ],
        b"solid" => &["color", "w", "h", "radius"],
        b"shape" => &[
            "color",
            "shape",
            "w",
            "h",
            "radius",
            "radiusTL",
            "radiusTR",
            "radiusBR",
            "radiusBL",
            "smoothing",
            "stroke",
            "strokeColor",
            "points",
        ],
        b"image" => &["fit", "w", "h"],
        b"video" => &[
            "fit",
            "w",
            "h",
            "trim",
            "speed",
            "timeRemap",
            "sourceTime",
            "audioGain",
        ],
        b"audio" => &["trim", "gain", "fadeIn", "fadeOut"],
        b"shader" | b"extension" => &["w", "h"],
        b"precomp" => &["w", "h", "trim", "sourceTime", "timeRemap", "audioGain"],
        _ => &[],
    }
}

/// `/^fontAxis\.[\x20-\x7e]{4}$/`.
pub fn is_font_axis_key(key: &[u8]) -> bool {
    key.len() == 13
        && key.starts_with(b"fontAxis.")
        && key[9..].iter().all(|c| (0x20..=0x7e).contains(c))
}

fn layer_type(layer: &Node) -> JsString {
    node::to_js_string(node::member(Some(layer), "type"))
}

/// `canAnimateContent(layer, key)`.
pub fn can_animate_content(layer: &Node, key: &[u8]) -> bool {
    let ty = layer_type(layer);
    content_fields(ty.as_bytes())
        .iter()
        .any(|f| f.as_bytes() == key)
        || (ty.as_bytes() == b"text" && is_font_axis_key(key))
}

/// `contentLabel(key)`.
pub fn content_label(key: &JsString) -> JsValue {
    let name = match key.as_bytes() {
        b"w" => "Width",
        b"h" => "Height",
        b"text" => "Text",
        b"font" => "Font",
        b"weight" => "Weight",
        b"size" => "Size",
        b"tracking" => "Tracking",
        b"leading" => "Leading",
        b"align" => "Align",
        b"color" => "Color",
        b"italic" => "Italic",
        b"boxWidth" => "Text Box Width",
        b"boxHeight" => "Text Box Height",
        b"shape" => "Shape",
        b"radius" => "Corner radius",
        b"radiusTL" => "Top left radius",
        b"radiusTR" => "Top right radius",
        b"radiusBR" => "Bottom right radius",
        b"radiusBL" => "Bottom left radius",
        b"smoothing" => "Corner smoothing",
        b"stroke" => "Stroke",
        b"strokeColor" => "Stroke color",
        b"points" => "Points",
        b"fit" => "Fit",
        b"trim" => "Trim start",
        b"speed" => "Speed",
        b"gain" => "Gain",
        b"audioGain" => "Audio gain",
        b"fadeIn" => "Fade in",
        b"fadeOut" => "Fade out",
        _ => return JsValue::String(key.clone()),
    };
    JsValue::from(name)
}

/// The label of a transform channel (`PM.CH[k].label`), or `None`.
pub fn channel_label(key: &[u8]) -> Option<&'static str> {
    Some(match key {
        b"perspective" => "Perspective",
        b"position.z" => "Position Z",
        b"anchor.z" => "Anchor Z",
        b"scale.z" => "Scale Z",
        b"rotation.x" => "Rotation X",
        b"rotation.y" => "Rotation Y",
        b"orientation.x" => "Orientation X",
        b"orientation.y" => "Orientation Y",
        b"orientation.z" => "Orientation Z",
        b"anchor.x" => "Anchor X",
        b"anchor.y" => "Anchor Y",
        b"position.x" => "Position X",
        b"position.y" => "Position Y",
        b"scale.x" => "Scale X",
        b"scale.y" => "Scale Y",
        b"rotation" => "Rotation",
        b"opacity" => "Opacity",
        b"skew" => "Skew",
        _ => return None,
    })
}

/// `textAnimatorLabel(key)`.
pub fn text_animator_label(key: &JsString) -> JsValue {
    let name = match key.as_bytes() {
        b"opacity" => "Opacity",
        b"x" => "Position X",
        b"y" => "Position Y",
        b"scale" => "Scale",
        b"rotation" => "Rotation",
        b"skew" => "Skew",
        b"tracking" => "Tracking",
        b"blur" => "Blur",
        b"color" => "Color",
        b"amount" => "Amount",
        b"delay" => "Start",
        b"duration" => "Duration",
        b"stagger" => "Stagger",
        b"start" => "Range start",
        b"end" => "Range end",
        b"offset" => "Range offset",
        b"smoothness" => "Softness",
        b"speed" => "Speed",
        b"spread" => "Spread",
        b"unit" => "Based on",
        _ => return JsValue::String(key.clone()),
    };
    JsValue::from(name)
}

fn js(s: &str) -> JsValue {
    JsValue::from(s)
}

/// String concatenation of template-literal parts.
fn cat(parts: &[&JsString]) -> JsString {
    let mut units: Vec<u16> = Vec::new();
    let mut plain = String::new();
    let mut all_plain = true;
    for part in parts {
        match part.as_str() {
            Some(s) if all_plain => plain.push_str(s),
            _ => {
                if all_plain {
                    units.extend(plain.encode_utf16());
                    all_plain = false;
                }
                units.extend(part.utf16_units());
            }
        }
    }
    if all_plain {
        JsString::from(plain)
    } else {
        JsString::from_utf16_units(&units)
    }
}

fn s(text: &str) -> JsString {
    JsString::from(text)
}

/// `value || fallback` for a label or group.
fn or_str(value: Option<&Node>, fallback: &str) -> JsValue {
    if node::truthy(value) {
        node::js_value_opt(value)
    } else {
        js(fallback)
    }
}

/// `${x}` of a member of an iterated item (a string item has no members).
fn item_member_string(item: &Item<'_>, key: &str) -> JsString {
    node::to_js_string(item.node().and_then(|n| node::member(Some(n), key)))
}

fn item_member<'a>(item: &Item<'a>, key: &str) -> JsResult<Option<&'a Node>> {
    match item {
        Item::Node(n) => node::member_checked(Some(n), key),
        Item::Char(_) => Ok(None),
    }
}

/// `Object.entries(x || {})`.
fn entries_or_empty(value: Option<&Node>) -> Vec<(JsString, Got<'_>)> {
    if node::truthy(value) {
        node::entries(value.expect("truthy"))
    } else {
        Vec::new()
    }
}

/// `collect(path, prefix)` of `structuredProperties`.
fn collect_path<'a>(
    out: &mut Vec<PropEntry<'a>>,
    path: &Item<'a>,
    prefix: &JsString,
) -> JsResult<()> {
    let group = match path {
        Item::Node(n) => or_str(node::member(Some(n), "name"), "Path"),
        Item::Char(_) => js("Path"),
    };
    // path.p (throws on null)
    let p = item_member(path, "p")?;
    for (k, prop) in entries_or_empty(p) {
        if node::is_property(prop.node()) {
            out.push(PropEntry {
                key: cat(&[prefix, &s("."), &k]),
                prop,
                label: JsValue::String(k),
                group: group.clone(),
            });
        }
    }
    let vertices = item_member(path, "vertices")?;
    for (index, vertex) in node::for_of_or_empty(vertices)?.iter().enumerate() {
        let vp = item_member(vertex, "p")?;
        let vid = item_member_string(vertex, "id");
        for (k, prop) in entries_or_empty(vp) {
            if node::is_property(prop.node()) {
                let label = format!("Vertex {} {}", index + 1, k.to_string_lossy());
                out.push(PropEntry {
                    key: cat(&[prefix, &s(".v."), &vid, &s("."), &k]),
                    prop,
                    label: js(&label),
                    group: group.clone(),
                });
            }
        }
    }
    Ok(())
}

/// `structuredProperties(layer)`.
pub fn structured_properties(layer: &Node) -> JsResult<Vec<PropEntry<'_>>> {
    let mut out = Vec::new();
    let d = node::member(Some(layer), "d");
    for path in node::for_of_or_empty(node::member(d, "paths"))? {
        let prefix = cat(&[&s("g."), &item_member_string(&path, "id")]);
        if let Item::Node(n) = path {
            node::member_checked(Some(n), "id")?;
        }
        collect_path(&mut out, &path, &prefix)?;
    }
    let masks = node::member(Some(layer), "masks");
    for mask in node::for_of_or_empty(masks)? {
        let path = item_member(&mask, "path")?;
        if node::truthy(path) {
            let prefix = cat(&[&s("mp."), &item_member_string(&mask, "id")]);
            collect_path(&mut out, &Item::Node(path.expect("truthy")), &prefix)?;
        }
    }
    for animator in node::for_of_or_empty(node::member(d, "animators"))? {
        let p = item_member(&animator, "p")?;
        let aid = item_member_string(&animator, "id");
        let group = match &animator {
            Item::Node(n) => or_str(node::member(Some(n), "name"), "Text animator"),
            Item::Char(_) => js("Text animator"),
        };
        for (key, prop) in entries_or_empty(p) {
            if node::is_property(prop.node()) {
                out.push(PropEntry {
                    key: cat(&[&s("ta."), &aid, &s("."), &key]),
                    label: text_animator_label(&key),
                    prop,
                    group: group.clone(),
                });
            }
        }
    }
    for range in node::for_of_or_empty(node::member(d, "styles"))? {
        let p = item_member(&range, "p")?;
        let rid = item_member_string(&range, "id");
        for (key, prop) in entries_or_empty(p) {
            if node::is_property(prop.node()) {
                out.push(PropEntry {
                    key: cat(&[&s("ts."), &rid, &s("."), &key]),
                    label: JsValue::String(key),
                    prop,
                    group: js("Text range"),
                });
            }
        }
    }
    Ok(out)
}

/// The 3D layer role of a layer (`layer3DRole`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer3dRole {
    Object,
    Light,
    Camera,
}

/// `layer3DRole(layer)`: from `d.definition`.
pub fn layer3d_role(layer: &Node) -> Option<Layer3dRole> {
    let definition = node::member(node::member(Some(layer), "d"), "definition");
    if node::is_str(definition, "powermove.3d.object") {
        Some(Layer3dRole::Object)
    } else if node::is_str(definition, "powermove.3d.light") {
        Some(Layer3dRole::Light)
    } else if node::is_str(definition, "powermove.3d.camera") {
        Some(Layer3dRole::Camera)
    } else {
        None
    }
}

/// `collect(p, prefix, group)` of `layer3DProperties`: members with an array
/// `kf` (no `v` requirement).
fn collect_kf<'a>(
    out: &mut Vec<PropEntry<'a>>,
    p: Option<&'a Node>,
    prefix: &JsString,
    group: &JsValue,
) {
    for (key, prop) in entries_or_empty(p) {
        if prop.truthy() && node::has_kf_array(prop.node()) {
            out.push(PropEntry {
                key: cat(&[prefix, &s("."), &key]),
                label: JsValue::String(key),
                prop,
                group: group.clone(),
            });
        }
    }
}

/// `layer3DProperties(layer)`.
pub fn layer3d_properties(layer: &Node) -> JsResult<Vec<PropEntry<'_>>> {
    let role = layer3d_role(layer);
    let d = node::member(Some(layer), "d");
    let data = node::member(d, "data");
    let modeling = node::member(d, "modeling");
    let mut out = Vec::new();
    if role.is_none() && !node::truthy(modeling) {
        return Ok(out);
    }
    if node::truthy(modeling) {
        collect_kf(
            &mut out,
            node::member(modeling, "p"),
            &s("model"),
            &js("Model"),
        );
    }
    match role {
        Some(Layer3dRole::Object) => {
            let object = node::member_checked(data, "object")?;
            let material = node::member(object, "material");
            collect_kf(
                &mut out,
                node::member(material, "p"),
                &s("m"),
                &js("Material"),
            );
            let shader = node::member(material, "shader");
            collect_kf(
                &mut out,
                node::member(shader, "p"),
                &s("shader"),
                &js("Shader"),
            );
            for slot in node::for_of_or_empty(node::member(object, "slots"))? {
                let name = match &slot {
                    Item::Node(n) => node::js_value_opt(node::member(Some(n), "name")),
                    Item::Char(_) => JsValue::Undefined,
                };
                let sid = item_member_string(&slot, "id");
                let material = item_member(&slot, "material")?;
                let p = node::member_checked(material, "p")?;
                collect_kf(&mut out, p, &cat(&[&s("slots."), &sid]), &name);
                let shader = node::member(material, "shader");
                collect_kf(
                    &mut out,
                    node::member(shader, "p"),
                    &cat(&[&s("slots."), &sid, &s(".shader")]),
                    &name,
                );
            }
        }
        Some(Layer3dRole::Light) => {
            let light = node::member_checked(data, "light")?;
            collect_kf(
                &mut out,
                node::member(light, "p"),
                &s("light"),
                &js("Light"),
            );
        }
        Some(Layer3dRole::Camera) => {
            let camera = node::member_checked(data, "camera")?;
            collect_kf(
                &mut out,
                node::member(camera, "p"),
                &s("camera"),
                &js("Camera"),
            );
            let environment = node::member(data, "environment");
            collect_kf(
                &mut out,
                node::member(environment, "p"),
                &s("environment"),
                &js("Environment"),
            );
        }
        None => {}
    }
    Ok(out)
}

/// `collect(node, prefix, name)` of `sceneProperties`: `node?.p`.
fn collect_node<'a>(
    out: &mut Vec<PropEntry<'a>>,
    item: Option<&'a Node>,
    prefix: &JsString,
    name: &JsValue,
) {
    collect_kf(out, node::member(item, "p"), prefix, name);
}

/// `sceneProperties(layer)`: legacy `scene3d` layers (`d.data.scene`), else
/// [`layer3d_properties`].
pub fn scene_properties(layer: &Node) -> JsResult<Vec<PropEntry<'_>>> {
    let scene = node::member(
        node::member(node::member(Some(layer), "d"), "data"),
        "scene",
    );
    if !node::truthy(scene) {
        return layer3d_properties(layer);
    }
    let mut out = Vec::new();
    for object in node::for_of_or_empty(node::member(scene, "objects"))? {
        let oid = match &object {
            Item::Node(n) => node::to_js_string(node::member_checked(Some(n), "id")?),
            Item::Char(_) => s("undefined"),
        };
        let name = match &object {
            Item::Node(n) => node::js_value_opt(node::member(Some(n), "name")),
            Item::Char(_) => JsValue::Undefined,
        };
        let prefix = cat(&[&s("o."), &oid]);
        collect_node(&mut out, object.node(), &prefix, &name);
        let material_name = js_sem::add(&name, &js(" material"));
        let material = item_member(&object, "material")?;
        collect_node(
            &mut out,
            material,
            &cat(&[&prefix, &s(".m")]),
            &material_name,
        );
    }
    for light in node::for_of_or_empty(node::member(scene, "lights"))? {
        let lid = match &light {
            Item::Node(n) => node::to_js_string(node::member_checked(Some(n), "id")?),
            Item::Char(_) => s("undefined"),
        };
        let name = match &light {
            Item::Node(n) => node::js_value_opt(node::member(Some(n), "name")),
            Item::Char(_) => JsValue::Undefined,
        };
        collect_node(&mut out, light.node(), &cat(&[&s("light."), &lid]), &name);
    }
    collect_node(
        &mut out,
        node::member(scene, "camera"),
        &s("camera"),
        &js("Camera"),
    );
    collect_node(
        &mut out,
        node::member(scene, "environment"),
        &s("environment"),
        &js("Scene"),
    );
    Ok(out)
}

/// `/^(?:m\.[^.]+$|(?:o|light|camera|environment|shader|slots|model)\.)/`.
pub fn is_scene_key(key: &[u8]) -> bool {
    if let Some(rest) = key.strip_prefix(b"m.")
        && !rest.is_empty()
        && !rest.contains(&b'.')
    {
        return true;
    }
    [
        &b"o."[..],
        b"light.",
        b"camera.",
        b"environment.",
        b"shader.",
        b"slots.",
        b"model.",
    ]
    .iter()
    .any(|p| key.starts_with(p))
}

/// `/^(g|mp|ta|ts)\./`.
pub fn is_structured_key(key: &[u8]) -> bool {
    [&b"g."[..], b"mp.", b"ta.", b"ts."]
        .iter()
        .any(|p| key.starts_with(p))
}

/// `/^(transitionIn|transitionOut)\.p\.([a-zA-Z][a-zA-Z0-9]*)$/`.
fn transition_key(key: &[u8]) -> Option<(&'static str, &[u8])> {
    for field in ["transitionIn", "transitionOut"] {
        if let Some(rest) = key.strip_prefix(field.as_bytes())
            && let Some(param) = rest.strip_prefix(b".p.")
            && !param.is_empty()
            && param[0].is_ascii_alphabetic()
            && param.iter().all(u8::is_ascii_alphanumeric)
        {
            return Some((field, param));
        }
    }
    None
}

/// The JavaScript split parts of a key (`key.split('.')`); a missing part is
/// `undefined`, which becomes the property key `"undefined"`.
fn split_part(key: &[u8], index: usize) -> Option<&[u8]> {
    key.split(|&c| c == b'.').nth(index)
}

fn part_key(part: Option<&[u8]>) -> &[u8] {
    part.unwrap_or(b"undefined")
}

/// `PM.findProp(L, key)`: the stored value a key addresses, or
/// [`Got::Undefined`] for `null` and `undefined` (the legacy function returns
/// either; callers treat both as "no property"). Throws where the legacy
/// function throws.
pub fn find_prop<'a>(layer: &'a Node, key: &JsString) -> JsResult<Got<'a>> {
    let k = key.as_bytes();
    let found = |list: Vec<PropEntry<'a>>| -> Got<'a> {
        list.into_iter()
            .find(|e| e.key == *key)
            .map_or(Got::Undefined, |e| {
                if e.prop.is_nullish() {
                    Got::Undefined
                } else {
                    e.prop
                }
            })
    };
    // Every branch reads a member of the layer first.
    if matches!(layer.kind(), pm_model::NodeKind::Null) {
        return Err(crate::js_sem::JsThrow::type_error(
            "Cannot read properties of null (reading 'd')",
        ));
    }
    if is_scene_key(k) {
        return Ok(found(scene_properties(layer)?));
    }
    if is_structured_key(k) {
        return Ok(found(structured_properties(layer)?));
    }
    let l = Some(layer);
    if let Some(p) = node::member(l, "p")
        && !node::is_nullish(Some(p))
    {
        let got = node::get(p, k);
        if got.truthy() {
            return Ok(got);
        }
    }
    if k == b"l.blend" || k == b"l.mblur" || k == b"l.matteMode" {
        let v = node::member(l, std::str::from_utf8(&k[2..]).expect("ascii"));
        return Ok(if node::is_property(v) {
            Got::Node(v.expect("property"))
        } else {
            Got::Undefined
        });
    }
    if let Some(field) = k.strip_prefix(b"c.") {
        let d = node::member(l, "d");
        let v = if node::is_nullish(d) {
            Got::Undefined
        } else {
            node::get(d.expect("d"), field)
        };
        return Ok(if node::is_property(v.node()) {
            v
        } else {
            Got::Undefined
        });
    }
    if let Some((field, param)) = transition_key(k) {
        let p = node::member(node::member(l, field), "p");
        let v = if node::is_nullish(p) {
            Got::Undefined
        } else {
            node::get(p.expect("p"), param)
        };
        return Ok(if v.truthy() { v } else { Got::Undefined });
    }
    for (prefix, member) in [(&b"u."[..], "uniforms"), (b"x.", "params")] {
        if let Some(name) = k.strip_prefix(prefix) {
            let container = node::member(node::member(l, "d"), member);
            if !node::truthy(container) {
                return Ok(container.map_or(Got::Undefined, Got::Node));
            }
            return Ok(node::get(container.expect("truthy"), name));
        }
    }
    if k.starts_with(b"m.") {
        let mid = split_part(k, 1);
        let mk = part_key(split_part(k, 2));
        let masks = node::array_or_empty(node::member(l, "masks"))?;
        let mut found_mask = None;
        for m in masks {
            let id = node::get_checked(Some(m), b"id")?;
            if let (Some(mid), Got::Node(idn)) = (mid, &id)
                && node::string(Some(idn)).is_some_and(|s| s.as_bytes() == mid)
            {
                found_mask = Some(m);
                break;
            }
        }
        let Some(m) = found_mask else {
            return Ok(Got::Undefined);
        };
        let mp = node::get_checked(Some(m), b"p")?;
        let direct = match mp {
            Got::Node(p) => node::get_checked(Some(p), mk)?,
            _ => node::get_checked(None, mk)?,
        };
        if !direct.is_nullish() {
            return Ok(direct);
        }
        let own = node::get(m, mk);
        return Ok(if node::is_property(own.node()) {
            own
        } else {
            Got::Undefined
        });
    }
    let fid = split_part(k, 0).unwrap_or(b"");
    let pk = split_part(k, 1);
    let effects = node::array_or_empty(node::member(l, "fx"))?;
    let mut found_fx = None;
    for f in effects {
        let id = node::get_checked(Some(f), b"id")?;
        if let Got::Node(idn) = &id
            && node::string(Some(idn)).is_some_and(|s| s.as_bytes() == fid)
        {
            found_fx = Some(f);
            break;
        }
    }
    let Some(fx) = found_fx else {
        return Ok(Got::Undefined);
    };
    if pk == Some(b"$enabled") {
        let on = node::member(Some(fx), "on");
        return Ok(if node::is_property(on) {
            Got::Node(on.expect("on"))
        } else {
            Got::Undefined
        });
    }
    let p = node::get_checked(Some(fx), b"p")?;
    match p {
        Got::Node(p) => node::get_checked(Some(p), part_key(pk)),
        _ => node::get_checked(None, part_key(pk)),
    }
    .map(|g| if g.is_nullish() { Got::Undefined } else { g })
}

/// `PM.allProps(L)`.
pub fn all_props<'a>(
    layer: &'a Node,
    definitions: &dyn DefinitionLabels,
) -> JsResult<Vec<PropEntry<'a>>> {
    let l = Some(layer);
    if matches!(layer.kind(), pm_model::NodeKind::Null) {
        return Err(crate::js_sem::JsThrow::type_error(
            "Cannot read properties of null (reading 'p')",
        ));
    }
    let mut out = Vec::new();
    let p = node::member(l, "p");
    if node::truthy(p) {
        for k in node::for_in_keys(p) {
            let label = channel_label(k.as_bytes()).map_or(JsValue::String(k.clone()), js);
            out.push(PropEntry {
                prop: node::get(p.expect("truthy"), k.as_bytes()),
                key: k,
                label,
                group: js("Transform"),
            });
        }
    }
    for (key, prop) in entries_or_empty(node::member(l, "d")) {
        if can_animate_content(layer, key.as_bytes()) && node::is_property(prop.node()) {
            out.push(PropEntry {
                key: cat(&[&s("c."), &key]),
                label: content_label(&key),
                prop,
                group: js("Content"),
            });
        }
    }
    for key in ["blend", "mblur", "matteMode"] {
        let v = node::member(l, key);
        if node::is_property(v) {
            out.push(PropEntry {
                key: s(&format!("l.{key}")),
                prop: Got::Node(v.expect("property")),
                label: js(if key == "blend" {
                    "Blend mode"
                } else {
                    "Motion blur"
                }),
                group: js("Layer"),
            });
        }
    }
    for fx in node::array_or_empty(node::member(l, "fx"))? {
        let fx_id = node::get_checked(Some(fx), b"id")?.to_js();
        let group = node::get(fx, b"type").to_js();
        let on = node::member(Some(fx), "on");
        if node::is_property(on) {
            out.push(PropEntry {
                key: js_sem::add(&fx_id, &js(".$enabled")).to_js_string(),
                prop: Got::Node(on.expect("property")),
                label: js("Enabled"),
                group: group.clone(),
            });
        }
        let fp = node::member(Some(fx), "p");
        if node::truthy(fp) {
            for k in node::for_in_keys(fp) {
                let key = js_sem::add(&js_sem::add(&fx_id, &js(".")), &JsValue::String(k.clone()));
                out.push(PropEntry {
                    key: key.to_js_string(),
                    prop: node::get(fp.expect("truthy"), k.as_bytes()),
                    label: JsValue::String(k),
                    group: group.clone(),
                });
            }
        }
    }
    for (index, m) in node::array_or_empty(node::member(l, "masks"))?
        .iter()
        .enumerate()
    {
        let group = JsValue::from(format!("Mask {}", index + 1).as_str());
        let mid_got = node::get_checked(Some(m), b"id")?;
        let mid = mid_got.to_js();
        for key in ["shape", "mode", "on"] {
            let v = node::get(m, key.as_bytes());
            if node::is_property(v.node()) {
                out.push(PropEntry {
                    key: cat(&[&s("m."), &mid.to_js_string(), &s("."), &s(key)]),
                    prop: v,
                    label: js(key),
                    group: group.clone(),
                });
            }
        }
        let mp = node::get(m, b"p");
        if let Got::Node(mp) = mp {
            for k in node::for_in_keys(Some(mp)) {
                let key = js_sem::add(
                    &js_sem::add(&js_sem::add(&js("m."), &mid), &js(".")),
                    &JsValue::String(k.clone()),
                );
                out.push(PropEntry {
                    key: key.to_js_string(),
                    prop: node::get(mp, k.as_bytes()),
                    label: JsValue::String(k),
                    group: group.clone(),
                });
            }
        }
    }
    let d = node::member(l, "d");
    let ty = node::member(l, "type");
    if node::is_str(ty, "shader") {
        let uniforms = node::member(d, "uniforms");
        if node::truthy(uniforms) {
            for k in node::for_in_keys(uniforms) {
                out.push(PropEntry {
                    key: cat(&[&s("u."), &k]),
                    prop: node::get(uniforms.expect("truthy"), k.as_bytes()),
                    label: JsValue::String(k),
                    group: js("Shader"),
                });
            }
        }
    }
    if node::is_str(ty, "extension") {
        let definition = node::js_value_opt(node::member(d, "definition"));
        let params = node::member(d, "params");
        if node::truthy(params) {
            for k in node::for_in_keys(params) {
                let (group, label) = definitions.extension_labels(&definition, &k);
                out.push(PropEntry {
                    key: cat(&[&s("x."), &k]),
                    prop: node::get(params.expect("truthy"), k.as_bytes()),
                    label: label.filter(JsValue::truthy).unwrap_or(JsValue::String(k)),
                    group: group
                        .filter(JsValue::truthy)
                        .unwrap_or_else(|| js("Extension")),
                });
            }
        }
    }
    for (field, label) in [
        ("transitionIn", "Transition in"),
        ("transitionOut", "Transition out"),
    ] {
        let tp = node::member(node::member(l, field), "p");
        if node::truthy(tp) {
            for k in node::for_in_keys(tp) {
                out.push(PropEntry {
                    key: cat(&[&s(field), &s(".p."), &k]),
                    prop: node::get(tp.expect("truthy"), k.as_bytes()),
                    label: JsValue::String(k),
                    group: js(label),
                });
            }
        }
    }
    out.extend(structured_properties(layer)?);
    out.extend(scene_properties(layer)?);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::json::parse;

    fn n(text: &str) -> Node {
        Node::from_value(parse(text).unwrap(), 0)
    }

    #[test]
    fn key_patterns() {
        assert!(is_scene_key(b"m.color"));
        assert!(!is_scene_key(b"m.mask1.feather"));
        assert!(!is_scene_key(b"m."));
        assert!(is_scene_key(b"slots.a.shader.x"));
        assert!(is_structured_key(b"g.p1.x"));
        assert!(transition_key(b"transitionIn.p.amount").is_some());
        assert!(transition_key(b"transitionIn.p.1x").is_none());
        assert!(is_font_axis_key(b"fontAxis.wght"));
        assert!(!is_font_axis_key(b"fontAxis.wg"));
    }

    #[test]
    fn finds_effect_and_mask_params() {
        let layer = n(r#"{"type":"solid","p":{"opacity":{"v":100,"kf":[]}},
            "fx":[{"id":"f1","type":"blur","on":{"v":true,"kf":[]},"p":{"amount":{"v":3,"kf":[]}}}],
            "masks":[{"id":"k","p":{"feather":{"v":2,"kf":[]}},"on":{"v":true,"kf":[]}}]}"#);
        let get = |k: &str| find_prop(&layer, &JsString::from(k)).unwrap();
        assert!(get("opacity").node().is_some());
        assert!(get("f1.amount").node().is_some());
        assert!(get("f1.$enabled").node().is_some());
        assert!(get("m.k.feather").node().is_some());
        assert!(get("m.k.on").node().is_some());
        assert_eq!(get("f2.amount"), Got::Undefined);
        let all = all_props(&layer, &NoDefinitions).unwrap();
        let keys: Vec<String> = all.iter().map(|e| e.key.to_string_lossy()).collect();
        assert_eq!(
            keys,
            [
                "opacity",
                "f1.$enabled",
                "f1.amount",
                "m.k.on",
                "m.k.feather"
            ]
        );
    }
}
