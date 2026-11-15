//! Electron schemaVersion 1 animation presets. Times on disk are relative to capture.
use crate::{Inspector, InspectorError, PropertyId};
use pm_kernel::commands::{
    effects::{AddEffect, EffectCommand},
    keyframes::{KeyframeCommand, KeyframeSpec, ReplaceKeyframes},
    properties::{KeyMode, PropertyCommand, SetProperty},
};
use pm_kernel::{Caller, Command, Request, new_id};
use pm_model::{Map, Value};
fn portable(path: &str) -> bool {
    !["m.", "g.", "mp.", "ta.", "ts.", "u.", "transition"]
        .iter()
        .any(|p| path.starts_with(p))
        && !path
            .split('.')
            .any(|p| ["__proto__", "prototype", "constructor"].contains(&p))
}
fn shift(channel: &Value, offset: f64) -> Value {
    let mut result = channel.clone();
    if let Some(keys) = result
        .as_object_mut()
        .and_then(|m| m.get_mut("kf"))
        .and_then(Value::as_array_mut)
    {
        for key in keys {
            if let Some(o) = key.as_object_mut() {
                o.remove("i");
                let t = o.get("t").and_then(Value::as_f64).unwrap_or(0.);
                o.insert("t", (t + offset).into());
            }
        }
    }
    result
}
impl Inspector {
    fn capture_preset_channel(
        &mut self,
        layer: &str,
        animated: bool,
        time: f64,
        from: f64,
        path: &str,
        channel: &Value,
    ) -> Result<Value, InspectorError> {
        let value = self.evaluate(&PropertyId::new(layer, path), time)?;
        let mut saved = shift(channel, from - time);
        let object = saved.as_object_mut().ok_or(InspectorError::InvalidInput)?;
        let remove: Vec<_> = object
            .iter()
            .filter(|(k, _)| *k != "v" && *k != "kf")
            .map(|(k, _)| k.clone())
            .collect();
        for key in remove {
            object.remove_key(&key);
        }
        object.insert("v", value);
        if !animated {
            object.insert("kf", Value::Array(vec![]));
        }
        Ok(saved)
    }

    pub fn preset_choices(&mut self, layer: &str) -> Result<Value, InspectorError> {
        let source = self.layer(layer)?;
        let properties = self
            .property_window(layer, 0..4096, 0.)?
            .rows
            .into_iter()
            .filter(|r| {
                portable(&r.id.path)
                    && !source
                        .get("fx")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .any(|f| {
                            f.get("id")
                                .and_then(Value::as_str)
                                .is_some_and(|id| r.id.path.starts_with(&format!("{id}.")))
                        })
            })
            .map(|r| Value::from(r.id.path))
            .collect();
        let mut out = Map::new();
        out.insert("properties", Value::Array(properties));
        out.insert(
            "effects",
            source.get("fx").cloned().unwrap_or(Value::Array(vec![])),
        );
        Ok(Value::Object(out))
    }
    pub fn capture_preset(
        &mut self,
        layer: &str,
        name: &str,
        effects: &[String],
        paths: &[String],
        animated: bool,
        time: f64,
    ) -> Result<Value, InspectorError> {
        if name.trim().is_empty() || name.trim().chars().count() > 80 || !time.is_finite() {
            return Err(InspectorError::InvalidInput);
        }
        let source = self.layer(layer)?;
        let from = source.get("from").and_then(Value::as_f64).unwrap_or(0.);
        let mut saved_effects = Vec::new();
        for fx in source
            .get("fx")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let id = fx.get("id").and_then(Value::as_str).unwrap_or("");
            if !effects.iter().any(|s| s == id) {
                continue;
            }
            let mut parameters = Map::new();
            for (key, channel) in fx.get("p").and_then(Value::as_object).into_iter().flatten() {
                parameters.insert(
                    key.clone(),
                    self.capture_preset_channel(
                        layer,
                        animated,
                        time,
                        from,
                        &format!("{id}.{key}"),
                        channel,
                    )?,
                );
            }
            let on = fx.get("on").cloned().unwrap_or(Value::Bool(true));
            let on = if on.get("v").is_some() {
                self.capture_preset_channel(
                    layer,
                    animated,
                    time,
                    from,
                    &format!("{id}.$enabled"),
                    &on,
                )?
            } else {
                on
            };
            let mut object = Map::new();
            object.insert("type", fx.get("type").cloned().unwrap_or(Value::Null));
            object.insert("parameters", Value::Object(parameters));
            object.insert("enabled", on);
            saved_effects.push(Value::Object(object));
        }
        let mut properties = Vec::new();
        for path in paths {
            if !portable(path) {
                return Err(InspectorError::InvalidInput);
            }
            let channel = self
                .property(&PropertyId::new(layer, path))?
                .ok_or(InspectorError::InvalidInput)?;
            let mut entry = Map::new();
            entry.insert("path", path.clone().into());
            entry.insert(
                "channel",
                self.capture_preset_channel(layer, animated, time, from, path, &channel)?,
            );
            properties.push(Value::Object(entry));
        }
        if saved_effects.is_empty() && properties.is_empty() {
            return Err(InspectorError::EmptyEdit);
        }
        let mut out = Map::new();
        out.insert("schemaVersion", 1.into());
        out.insert("id", new_id("preset").into());
        out.insert("name", name.trim().into());
        out.insert("effects", Value::Array(saved_effects));
        out.insert("properties", Value::Array(properties));
        Ok(Value::Object(out))
    }
    pub fn apply_preset(
        &mut self,
        caller: Caller,
        preset: &Value,
        targets: &[String],
        time: f64,
    ) -> Result<Request, InspectorError> {
        if preset.get("schemaVersion").and_then(Value::as_f64) != Some(1.) || !time.is_finite() {
            return Err(InspectorError::InvalidInput);
        }
        let mut commands: Vec<Command> = Vec::new();
        for layer in targets {
            let source = self.layer(layer)?;
            let offset = time - source.get("from").and_then(Value::as_f64).unwrap_or(0.);
            for fx in preset
                .get("effects")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(kind) = fx.get("type").and_then(Value::as_str) else {
                    continue;
                };
                let Some(definition) = self.definitions.get(kind).cloned() else {
                    continue;
                };
                let parameters = fx
                    .get("parameters")
                    .and_then(Value::as_object)
                    .map(|m| {
                        m.iter()
                            .map(|(k, v)| (k.clone(), shift(v, offset)))
                            .collect()
                    })
                    .unwrap_or_default();
                let on = fx.get("enabled").cloned().unwrap_or(Value::Bool(true));
                commands.push(
                    EffectCommand::Add(Box::new(AddEffect {
                        layer_id: layer.clone(),
                        effect_id: new_id("f"),
                        definition,
                        parameters,
                        inputs: None,
                        open: true,
                        enabled: on.as_bool().unwrap_or(true),
                        enabled_animation: on.get("v").is_some().then(|| shift(&on, offset)),
                        index: None,
                    }))
                    .into(),
                );
            }
            for entry in preset
                .get("properties")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(path) = entry
                    .get("path")
                    .and_then(Value::as_str)
                    .filter(|p| portable(p))
                else {
                    continue;
                };
                let id = PropertyId {
                    layer: layer.clone(),
                    path: path.into(),
                };
                if self.property(&id)?.is_none() {
                    continue;
                }
                let Some(saved) = entry.get("channel") else {
                    continue;
                };
                let value = saved.get("v").cloned().unwrap_or(Value::Null);
                let mut set = SetProperty::new(layer.as_str(), path, value.clone(), time);
                set.mode = KeyMode::Auto;
                commands.push(PropertyCommand::Set(set).into());
                let keys: Vec<_> = saved
                    .get("kf")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|k| {
                        let mut spec = KeyframeSpec::new(
                            k.get("t").and_then(Value::as_f64).unwrap_or(0.) + offset,
                            k.get("v").cloned().unwrap_or(value.clone()),
                        );
                        spec.temporal = Some(k.clone());
                        spec.hold = k.get("hold").and_then(Value::as_bool).unwrap_or(false);
                        spec
                    })
                    .collect();
                if !keys.is_empty() {
                    commands.push(
                        KeyframeCommand::Replace(Box::new(ReplaceKeyframes {
                            layer_id: layer.clone(),
                            channel: path.into(),
                            keyframes: keys,
                            replace: false,
                            expression: Some(None),
                            preserve_hand_edits: true,
                            time,
                        }))
                        .into(),
                    );
                }
            }
        }
        self.transaction(caller, commands)
    }
}
