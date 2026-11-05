//! Layer content (`d`): text, shape geometry, media fit and trim, audio
//! levels, extension parameters.
//!
//! Ports `setContent` (with `setAudioContent`, `setExtensionContent`,
//! `normalizeAudioLevelPatch`, `resizeVideoForSpeed`) of
//! `legacy/core/editing.ts`, used by the inspector's content section and the
//! mixer.

use pm_model::{Map, Value};

use super::properties::{self, SetProperty};
use super::{Cx, channels, layers, track, values};
use crate::error::CommandError;

/// Content commands.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ContentCommand {
    /// `set_content` (`setContent`): merges `patch` into `d`. A field whose
    /// channel is animated is keyed at `time` instead (as `set_property`
    /// `c.<key>`). Audio layers accept only `asset`, `trim`, `gain`,
    /// `fadeIn`, `fadeOut` (clamped); extension layers only `w`, `h`,
    /// `data`, `parameters`; `audioGain` on video/pre-comp is clamped to 0–4
    /// (`null` clears it); a constant video `speed` change rescales the
    /// layer's duration. Keys `__proto__`, `prototype`, `constructor` are
    /// refused. Label "Edit Content".
    Set {
        layer_id: String,
        patch: Map,
        time: f64,
    },
}

impl ContentCommand {
    pub fn label(&self) -> String {
        "Edit Content".to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let ContentCommand::Set {
            layer_id,
            patch,
            time,
        } = self;
        set_content(cx, layer_id, patch, *time)
    }
}

const AUDIO_FIELDS: [&str; 5] = ["asset", "trim", "gain", "fadeIn", "fadeOut"];

/// `setContent(command)`.
fn set_content(
    cx: &mut Cx<'_>,
    layer_id: &str,
    patch: &Map,
    time: f64,
) -> Result<(), CommandError> {
    let (loc, layer) = cx.layer_mut_checked(layer_id)?;
    layers::safe_patch(patch, "content patch")?;
    let layer_type = values::text(&layer, "type").unwrap_or_default().to_owned();
    let mut patch = patch.clone();
    // `normalizeAudioLevelPatch`.
    let mut clears_level = false;
    if matches!(layer_type.as_str(), "video" | "precomp") {
        if let Some(m) = patch.get("audioMuted").cloned() {
            match m {
                Value::Bool(_) => {}
                Value::Null => {
                    patch.insert("audioMuted", Value::Bool(false));
                }
                _ => return Err(CommandError::Invalid("Audio mute must be on or off".into())),
            }
        }
        match patch.get("audioGain").cloned() {
            Some(Value::Null) => {
                patch.remove("audioGain");
                clears_level = true;
            }
            Some(v) if values::is_property(&v) => {}
            Some(Value::Number(g)) => {
                patch.insert(
                    "audioGain",
                    Value::Number(values::clamp(values::finite(g, "Audio gain")?, 0.0, 4.0)),
                );
            }
            Some(_) => {
                return Err(CommandError::Invalid(
                    "Audio gain must be a finite number".into(),
                ));
            }
            None => {}
        }
    }
    if clears_level {
        let mut p = loc.path();
        p.extend(["d".into(), "audioGain".into()]);
        cx.remove(p)?;
    }
    // Animated content fields are keyed at the playhead instead.
    let keys: Vec<String> = patch.keys().map(|k| k.to_string_lossy()).collect();
    for key in keys {
        let current = layer.get("d").and_then(|d| d.get(&key));
        let value = patch.get(&key).cloned().unwrap_or_default();
        if current.is_some_and(values::is_property)
            && !values::is_property(&value)
            && channels::can_animate_content(&layer, &key)
        {
            let mut c = SetProperty::new(layer_id, format!("c.{key}"), value, time);
            c.preserve_hand_edits = false;
            properties::set_property(cx, &c)?;
            patch.remove(&key);
        }
    }
    if patch.is_empty() {
        return Ok(());
    }
    let (_, mut layer) = cx.layer(layer_id)?;
    let fps = cx.fps(loc.comp_id.as_deref());
    let speed_before = patch
        .contains_key("speed")
        .then(|| properties::previous_speed(&layer))
        .flatten();
    match layer_type.as_str() {
        "audio" => audio_content(&mut layer, &patch)?,
        "extension" => {
            if super::scene3d::is_known_layer(&layer) {
                return super::scene3d::set_known_content(cx, layer_id, &patch);
            }
            let definition = layer
                .get("d")
                .and_then(|d| values::text(d, "definition"))
                .and_then(|id| cx.state.and_then(|state| state.provider("layerTypes", id)))
                .map(|registration| &registration.definition);
            extension_content(&mut layer, &patch, definition)?;
        }
        "captions" => {
            super::captions::set_content(&mut layer, &patch)?;
        }
        _ => {
            let m = values::obj(&mut layer);
            if m.get("d").and_then(Value::as_object).is_none() {
                m.insert("d", Value::Object(Map::new()));
            }
            let d = m.get_mut("d").and_then(Value::as_object_mut).expect("made");
            for (k, v) in patch.iter() {
                d.insert(k.clone(), v.clone());
            }
        }
    }
    if patch.contains_key("speed") {
        properties::resize_video_for_speed(&mut layer, speed_before, fps);
    }
    if layer_type == "shader" && patch.contains_key("code") {
        return Err(CommandError::NotImplemented(
            "set_content (shader code: uniforms are synced by the shader host)",
        ));
    }
    cx.put_layer(&loc, layer)
}

/// `setAudioContent(layer, patch)`.
fn audio_content(layer: &mut Value, patch: &Map) -> Result<(), CommandError> {
    for k in patch.keys() {
        let k = k.to_string_lossy();
        if !AUDIO_FIELDS.contains(&k.as_str()) {
            return Err(CommandError::Invalid(format!(
                "Audio content field “{k}” is not editable"
            )));
        }
    }
    let mut next = layer
        .get("d")
        .filter(|d| d.as_object().is_some())
        .cloned()
        .unwrap_or_default();
    for (k, v) in patch.iter() {
        values::obj(&mut next).insert(k.clone(), v.clone());
    }
    let number = |k: &str, fallback: f64, min: f64, max: f64| -> Result<Value, CommandError> {
        match next.get(k) {
            Some(v) if values::is_property(v) => Ok(v.clone()),
            Some(Value::Null) | None => Ok(Value::Number(values::clamp(fallback, min, max))),
            Some(v) => Ok(Value::Number(values::clamp(
                values::finite(track::js_number(v), k)?,
                min,
                max,
            ))),
        }
    };
    let asset = match next.get("asset") {
        Some(Value::String(s)) if !s.as_bytes().is_empty() => Value::String(s.clone()),
        Some(Value::Null) | None => Value::Null,
        _ => {
            return Err(CommandError::Invalid(
                "Audio source must be a media asset ID or null".into(),
            ));
        }
    };
    let mut d = Map::new();
    d.insert("asset", asset);
    d.insert("trim", number("trim", 0.0, 0.0, f64::INFINITY)?);
    d.insert("gain", number("gain", 1.0, 0.0, 4.0)?);
    d.insert("fadeIn", number("fadeIn", 0.0, 0.0, f64::INFINITY)?);
    d.insert("fadeOut", number("fadeOut", 0.0, 0.0, f64::INFINITY)?);
    let m = values::obj(layer);
    m.insert("d", Value::Object(d));
    m.insert("p", Value::Object(Map::new()));
    m.insert("fx", Value::Array(Vec::new()));
    m.insert("masks", Value::Array(Vec::new()));
    m.insert("parent", Value::Null);
    m.insert("blend", values::s("normal"));
    m.insert("mblur", Value::Bool(false));
    Ok(())
}

/// Main's extension content contract uses the winning registered definition,
/// while an unavailable definition leaves existing opaque parameters editable.
fn extension_content(
    layer: &mut Value,
    patch: &Map,
    definition: Option<&Value>,
) -> Result<(), CommandError> {
    const ALLOWED: &[&str] = &["definition", "version", "w", "h", "data", "parameters"];
    for key in patch.keys() {
        if !ALLOWED.contains(&key.to_string_lossy().as_str()) {
            return Err(CommandError::Invalid(format!(
                "Extension layer content field “{}” is not editable",
                key.to_string_lossy()
            )));
        }
    }
    let mut d = layer
        .get("d")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(id) = patch
        .get("definition")
        .filter(|v| !matches!(v, Value::Null))
        && d.get("definition") != Some(id)
    {
        return Err(CommandError::Invalid(
            "An extension layer definition cannot be changed in place".into(),
        ));
    }
    if let Some(version) = patch.get("version").filter(|v| !matches!(v, Value::Null))
        && definition.and_then(|v| v.get("version")) != Some(version)
    {
        return Err(CommandError::Invalid(
            "Extension layer version is owned by its definition".into(),
        ));
    }
    for (key, label) in [("w", "width"), ("h", "height")] {
        if let Some(value) = patch.get(key).filter(|v| !matches!(v, Value::Null)) {
            d.insert(
                key,
                Value::Number(values::finite(track::js_number(value), label)?.clamp(1., 16384.)),
            );
        }
    }
    if let Some(data) = patch.get("data").filter(|v| !matches!(v, Value::Null)) {
        let data = data
            .as_object()
            .ok_or_else(|| CommandError::Invalid("extension data must be an object".into()))?;
        layers::safe_patch(data, "extension data")?;
        let mut current = d
            .get("data")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (key, value) in data.iter() {
            current.insert(key.clone(), value.clone());
        }
        d.insert("data", Value::Object(current));
    }
    if let Some(parameters) = patch
        .get("parameters")
        .filter(|v| !matches!(v, Value::Null))
    {
        let parameters = parameters.as_object().ok_or_else(|| {
            CommandError::Invalid("extension parameters must be an object".into())
        })?;
        layers::safe_patch(parameters, "extension parameters")?;
        let mut channels = d
            .get("params")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (key, value) in parameters.iter() {
            let channel = channels
                .get_key_mut(key)
                .and_then(Value::as_object_mut)
                .ok_or_else(|| {
                    CommandError::Invalid(format!(
                        "Unknown extension parameter: {}",
                        key.to_string_lossy()
                    ))
                })?;
            let parameter = definition
                .and_then(|v| v.get("params"))
                .and_then(Value::as_array)
                .and_then(|items| {
                    items
                        .iter()
                        .find(|v| v.get("k").and_then(Value::as_js_string) == Some(key))
                });
            let value = match parameter {
                None => value.clone(),
                Some(parameter) => {
                    let label = values::text(parameter, "label")
                        .unwrap_or_else(|| key.as_str().unwrap_or("Parameter"));
                    match values::text(parameter, "type") {
                        Some("color") => {
                            let text = value
                                .as_str()
                                .filter(|s| {
                                    (s.len() == 7 || s.len() == 9)
                                        && s.starts_with('#')
                                        && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
                                })
                                .ok_or_else(|| {
                                    CommandError::Invalid(format!("{label} must be a hex color"))
                                })?;
                            Value::from(text.to_ascii_uppercase())
                        }
                        Some("toggle") => Value::Bool(value.as_bool().ok_or_else(|| {
                            CommandError::Invalid(format!("{label} must be on or off"))
                        })?),
                        _ => {
                            let n = values::finite(track::js_number(value), label)?;
                            let min = parameter
                                .get("min")
                                .and_then(Value::as_f64)
                                .unwrap_or(f64::NEG_INFINITY);
                            let max = parameter
                                .get("max")
                                .and_then(Value::as_f64)
                                .unwrap_or(f64::INFINITY);
                            Value::Number(n.max(min).min(max))
                        }
                    }
                }
            };
            channel.insert("v", value);
        }
        d.insert("params", Value::Object(channels));
    }
    values::obj(layer).insert("d", Value::Object(d));
    Ok(())
}
