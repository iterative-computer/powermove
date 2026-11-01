//! Finding a property channel on a layer: `property(layer, path)` of
//! `legacy/core/editing.ts` over `PM.findProp` and `PM.allProps` of
//! `legacy/core/anim.ts`.
//!
//! A channel is addressed by the old key (`opacity`, `c.size`,
//! `<fxId>.<param>`, …). [`materialize`] first turns a plain value into a
//! channel where the old code did (`c.<key>` for animatable content,
//! `l.blend`/`l.mblur`/`l.matteMode`, `<fxId>.$enabled`, a mask's `shape`,
//! `mode` and `on`), then returns the path of the channel object inside the
//! layer.

use pm_model::Value;

use super::values::{is_property, property};

/// One step into a layer value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Step {
    Key(String),
    Index(usize),
}

pub(crate) fn key(k: &str) -> Step {
    Step::Key(k.to_owned())
}

/// `contentFields[layer.type]` (`content-properties.ts`).
pub(crate) fn content_fields(layer_type: &str) -> &'static [&'static str] {
    match layer_type {
        "text" => &[
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
        "solid" => &["color", "w", "h", "radius"],
        "shape" => &[
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
        "image" => &["fit", "w", "h"],
        "video" => &[
            "fit",
            "w",
            "h",
            "trim",
            "speed",
            "timeRemap",
            "sourceTime",
            "audioGain",
        ],
        "audio" => &["trim", "gain", "fadeIn", "fadeOut"],
        "shader" | "extension" => &["w", "h"],
        "precomp" => &["w", "h", "trim", "sourceTime", "timeRemap", "audioGain"],
        _ => &[],
    }
}

/// `canAnimateContent(layer, key)`.
pub(crate) fn can_animate_content(layer: &Value, key: &str) -> bool {
    let t = layer
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    content_fields(t).contains(&key)
        || (t == "text"
            && key.strip_prefix("fontAxis.").is_some_and(|tag| {
                tag.len() == 4 && tag.bytes().all(|b| (0x20..=0x7e).contains(&b))
            }))
}

pub(crate) fn get<'v>(layer: &'v Value, path: &[Step]) -> Option<&'v Value> {
    let mut at = layer;
    for step in path {
        at = match step {
            Step::Key(k) => at.get(k)?,
            Step::Index(i) => at.as_array()?.get(*i)?,
        };
    }
    Some(at)
}

pub(crate) fn get_mut<'v>(layer: &'v mut Value, path: &[Step]) -> Option<&'v mut Value> {
    let mut at = layer;
    for step in path {
        at = match step {
            Step::Key(k) => at.as_object_mut()?.get_mut(k)?,
            Step::Index(i) => at.as_array_mut()?.get_mut(*i)?,
        };
    }
    Some(at)
}

fn index_by_id(list: Option<&Value>, id: &str) -> Option<usize> {
    list?
        .as_array()?
        .iter()
        .position(|item| item.get("id").and_then(Value::as_str) == Some(id))
}

/// Wraps `layer[path]` into `PM.P(value)` unless it already is a channel.
fn wrap(layer: &mut Value, path: &[Step]) {
    if let Some(slot) = get_mut(layer, path)
        && !is_property(slot)
    {
        let v = std::mem::take(slot);
        *slot = property(v);
    }
}

/// `property(layer, path)`: makes the channel exist where the old code did
/// and returns its path, or `None` (`Property “x” was not found`).
pub(crate) fn materialize(layer: &mut Value, channel: &str) -> Option<Vec<Step>> {
    if let Some(k) = channel.strip_prefix("c.")
        && can_animate_content(layer, k)
    {
        let d = layer.get("d");
        let present = d.and_then(|d| d.get(k)).is_some_and(|v| !v.is_null());
        if !present
            && k == "audioGain"
            && let Some(m) = layer
                .as_object_mut()
                .and_then(|l| l.get_mut("d"))
                .and_then(Value::as_object_mut)
        {
            m.insert("audioGain", Value::Number(1.0));
        }
        let present = layer
            .get("d")
            .and_then(|d| d.get(k))
            .is_some_and(|v| !v.is_null());
        if present {
            wrap(layer, &[key("d"), key(k)]);
        }
    }
    if let Some(k @ ("blend" | "mblur" | "matteMode")) = channel.strip_prefix("l.") {
        if layer.get(k).is_none() {
            super::values::obj(layer).insert(k, Value::Null);
        }
        wrap(layer, &[key(k)]);
    }
    if let Some(fx_id) = channel.strip_suffix(".$enabled")
        && let Some(i) = index_by_id(layer.get("fx"), fx_id)
    {
        wrap(layer, &[key("fx"), Step::Index(i), key("on")]);
    }
    if let Some(rest) = channel.strip_prefix("m.")
        && let Some((mask_id, field @ ("shape" | "mode" | "on"))) = rest.split_once('.')
        && let Some(i) = index_by_id(layer.get("masks"), mask_id)
    {
        let path = [key("masks"), Step::Index(i), key(field)];
        if get(layer, &path).is_some() {
            wrap(layer, &path);
        }
    }
    find(layer, channel)
}

/// `PM.findProp(layer, key) || layer.p[key]`, read only.
pub(crate) fn find(layer: &Value, channel: &str) -> Option<Vec<Step>> {
    let found = |path: Vec<Step>| get(layer, &path).filter(|v| is_property(v)).map(|_| path);
    for edge in ["transitionIn", "transitionOut"] {
        if let Some(param) = channel
            .strip_prefix(edge)
            .and_then(|r| r.strip_prefix(".p."))
        {
            let ok = param
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
                && param.chars().all(|c| c.is_ascii_alphanumeric());
            return ok
                .then(|| vec![key(edge), key("p"), key(param)])
                .and_then(|p| {
                    get(layer, &p)
                        .filter(|v| v.as_object().is_some())
                        .map(|_| p)
                });
        }
    }
    if layer
        .get("p")
        .and_then(|p| p.get(channel))
        .is_some_and(Value::is_truthy)
    {
        return found(vec![key("p"), key(channel)]);
    }
    if let Some(k @ ("blend" | "mblur" | "matteMode")) = channel.strip_prefix("l.") {
        return found(vec![key(k)]);
    }
    if let Some(k) = channel.strip_prefix("c.") {
        return found(vec![key("d"), key(k)]);
    }
    if let Some(k) = channel.strip_prefix("u.") {
        return found(vec![key("d"), key("uniforms"), key(k)]);
    }
    if let Some(k) = channel.strip_prefix("x.") {
        return found(vec![key("d"), key("params"), key(k)]);
    }
    if let Some(rest) = channel.strip_prefix("m.") {
        let mut parts = rest.split('.');
        let (Some(mask_id), Some(field)) = (parts.next(), parts.next()) else {
            return None;
        };
        let i = index_by_id(layer.get("masks"), mask_id)?;
        return found(vec![key("masks"), Step::Index(i), key("p"), key(field)])
            .or_else(|| found(vec![key("masks"), Step::Index(i), key(field)]));
    }
    let mut parts = channel.split('.');
    let (fx_id, param) = (parts.next()?, parts.next()?);
    let i = index_by_id(layer.get("fx"), fx_id)?;
    if param == "$enabled" {
        found(vec![key("fx"), Step::Index(i), key("on")])
    } else {
        found(vec![key("fx"), Step::Index(i), key("p"), key(param)])
    }
}

/// Every channel of a layer with its key (`PM.allProps`, without the
/// structured and 3D scene channels).
pub(crate) fn all(layer: &Value) -> Vec<(String, Vec<Step>)> {
    let mut out = Vec::new();
    if let Some(p) = layer.get("p").and_then(Value::as_object) {
        for (k, v) in p.iter() {
            if is_property(v) {
                let k = k.to_string_lossy();
                out.push((k.clone(), vec![key("p"), Step::Key(k)]));
            }
        }
    }
    if let Some(d) = layer.get("d").and_then(Value::as_object) {
        for (k, v) in d.iter() {
            let k = k.to_string_lossy();
            if can_animate_content(layer, &k) && is_property(v) {
                out.push((format!("c.{k}"), vec![key("d"), Step::Key(k)]));
            }
        }
        for (group, prefix) in [("uniforms", "u."), ("params", "x.")] {
            if let Some(m) = d.get(group).and_then(Value::as_object) {
                for (k, v) in m.iter() {
                    if is_property(v) {
                        let k = k.to_string_lossy();
                        out.push((
                            format!("{prefix}{k}"),
                            vec![key("d"), key(group), Step::Key(k)],
                        ));
                    }
                }
            }
        }
    }
    for k in ["blend", "mblur", "matteMode", "on"] {
        if layer.get(k).is_some_and(is_property) {
            out.push((format!("l.{k}"), vec![key(k)]));
        }
    }
    if let Some(fx) = layer.get("fx").and_then(Value::as_array) {
        for (i, e) in fx.iter().enumerate() {
            let id = e.get("id").and_then(Value::as_str).unwrap_or_default();
            if e.get("on").is_some_and(is_property) {
                out.push((
                    format!("{id}.$enabled"),
                    vec![key("fx"), Step::Index(i), key("on")],
                ));
            }
            if let Some(p) = e.get("p").and_then(Value::as_object) {
                for (k, v) in p.iter() {
                    if is_property(v) {
                        let k = k.to_string_lossy();
                        out.push((
                            format!("{id}.{k}"),
                            vec![key("fx"), Step::Index(i), key("p"), Step::Key(k)],
                        ));
                    }
                }
            }
        }
    }
    if let Some(masks) = layer.get("masks").and_then(Value::as_array) {
        for (i, m) in masks.iter().enumerate() {
            let id = m.get("id").and_then(Value::as_str).unwrap_or_default();
            for f in ["shape", "mode", "on"] {
                if m.get(f).is_some_and(is_property) {
                    out.push((
                        format!("m.{id}.{f}"),
                        vec![key("masks"), Step::Index(i), key(f)],
                    ));
                }
            }
            if let Some(p) = m.get("p").and_then(Value::as_object) {
                for (k, v) in p.iter() {
                    if is_property(v) {
                        let k = k.to_string_lossy();
                        out.push((
                            format!("m.{id}.{k}"),
                            vec![key("masks"), Step::Index(i), key("p"), Step::Key(k)],
                        ));
                    }
                }
            }
        }
    }
    for edge in ["transitionIn", "transitionOut"] {
        if let Some(p) = layer
            .get(edge)
            .and_then(|t| t.get("p"))
            .and_then(Value::as_object)
        {
            for (k, v) in p.iter() {
                if is_property(v) {
                    let k = k.to_string_lossy();
                    out.push((
                        format!("{edge}.p.{k}"),
                        vec![key(edge), key("p"), Step::Key(k)],
                    ));
                }
            }
        }
    }
    out
}
