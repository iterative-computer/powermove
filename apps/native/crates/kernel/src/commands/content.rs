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
            return Err(CommandError::NotImplemented(
                "set_content (extension layers)",
            ));
        }
        "captions" => {
            return Err(CommandError::NotImplemented(
                "set_content (captions: use edit_captions)",
            ));
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
