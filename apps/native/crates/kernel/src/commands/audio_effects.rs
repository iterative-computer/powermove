//! Audio effect stack edits use the same transaction planner and inverse-op
//! history as visual effects. Parameters are static in this first version.
use super::{Cx, values};
use crate::error::CommandError;
pub use pm_fx_defs::audio::{Validated, parse as validate_definition};
use pm_model::{Map, Value};

pub const MAX_EFFECTS: usize = 16;

#[derive(Clone, Debug, PartialEq)]
pub enum AudioEffectCommand {
    Add {
        layer_id: String,
        effect_id: String,
        definition: Validated,
    },
    Remove {
        layer_id: String,
        effect_id: String,
    },
    Reorder {
        layer_id: String,
        effect_id: String,
        index: usize,
    },
    Enable {
        layer_id: String,
        effect_id: String,
        enabled: bool,
    },
    SetParameter {
        layer_id: String,
        effect_id: String,
        definition: Validated,
        key: String,
        value: serde_json::Value,
    },
}
impl AudioEffectCommand {
    pub fn label(&self) -> String {
        match self {
            Self::Add { .. } => "Add Audio Effect",
            Self::Remove { .. } => "Remove Audio Effect",
            Self::Reorder { .. } => "Reorder Audio Effect",
            Self::Enable { .. } => "Toggle Audio Effect",
            Self::SetParameter { .. } => "Set Audio Effect Parameter",
        }
        .into()
    }
    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let layer_id = match self {
            Self::Add { layer_id, .. }
            | Self::Remove { layer_id, .. }
            | Self::Reorder { layer_id, .. }
            | Self::Enable { layer_id, .. }
            | Self::SetParameter { layer_id, .. } => layer_id,
        };
        let (loc, mut layer) = cx.layer_mut_checked(layer_id)?;
        let ty = values::text(&layer, "type");
        if ty != Some("audio")
            && !(ty == Some("video")
                && layer
                    .get("d")
                    .and_then(|d| d.get("embeddedAudio"))
                    .and_then(Value::as_bool)
                    == Some(true))
        {
            return Err(CommandError::Invalid("Layer has no audio".into()));
        }
        let mut stack = layer
            .get("audioFx")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Self::Add {
            effect_id,
            definition,
            ..
        } = self
        {
            if stack.len() >= MAX_EFFECTS {
                return Err(CommandError::Invalid(
                    "audioFx: effect count exceeds 16".into(),
                ));
            }
            if effect_id.is_empty()
                || stack
                    .iter()
                    .any(|v| values::text(v, "id") == Some(effect_id))
            {
                return Err(CommandError::Invalid(
                    "audioFx.id: empty or duplicate instance id".into(),
                ));
            }
            let def = definition.definition();
            let mut p = Map::new();
            for param in &def.params {
                p.insert(param.key(), model_value(&param.default_value())?);
            }
            let mut effect = Map::new();
            effect.insert("id", values::s(effect_id));
            effect.insert("type", values::s(&def.id));
            // Retained for a useful missing-extension diagnostic after reopen.
            effect.insert("name", values::s(&def.label));
            effect.insert("on", Value::Bool(true));
            effect.insert("p", Value::Object(p));
            stack.push(Value::Object(effect));
        } else {
            let effect_id = match self {
                Self::Remove { effect_id, .. }
                | Self::Reorder { effect_id, .. }
                | Self::Enable { effect_id, .. }
                | Self::SetParameter { effect_id, .. } => effect_id,
                _ => unreachable!(),
            };
            let index = stack
                .iter()
                .position(|v| values::text(v, "id") == Some(effect_id))
                .ok_or_else(|| {
                    CommandError::Invalid(format!("Audio effect not found: {effect_id}"))
                })?;
            match self {
                Self::Remove { .. } => {
                    stack.remove(index);
                }
                Self::Reorder { index: to, .. } => {
                    let value = stack.remove(index);
                    stack.insert((*to).min(stack.len()), value);
                }
                Self::Enable { enabled, .. } => {
                    values::obj(&mut stack[index]).insert("on", Value::Bool(*enabled));
                }
                Self::SetParameter {
                    definition,
                    key,
                    value,
                    ..
                } => {
                    let def = definition.definition();
                    if values::text(&stack[index], "type") != Some(def.id.as_str()) {
                        return Err(CommandError::Invalid(
                            "audioFx.type: definition mismatch".into(),
                        ));
                    }
                    let param = def.params.iter().find(|p| p.key() == key).ok_or_else(|| {
                        CommandError::Invalid(format!("params.{key}: unknown parameter"))
                    })?;
                    param
                        .validate_value(value)
                        .map_err(|e| CommandError::Invalid(e.to_string()))?;
                    let fx = values::obj(&mut stack[index]);
                    let p = fx.get_mut("p").ok_or_else(|| {
                        CommandError::Invalid("audioFx.p: missing parameters".into())
                    })?;
                    values::obj(p).insert(key.as_str(), model_value(value)?);
                }
                _ => unreachable!(),
            }
        }
        values::obj(&mut layer).insert("audioFx", Value::Array(stack));
        cx.put_layer(&loc, layer)
    }
}
fn model_value(v: &serde_json::Value) -> Result<Value, CommandError> {
    pm_model::json::parse(&v.to_string()).map_err(|e| CommandError::Invalid(e.to_string()))
}
