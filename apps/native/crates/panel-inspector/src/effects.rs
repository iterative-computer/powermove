use crate::input::Modifiers;
use crate::view::{array, text};
use crate::{Inspector, InspectorError};
use pm_kernel::commands::effects::{AddEffect, EffectPatch};
use pm_kernel::{Caller, EffectCommand, Request};
use pm_model::{Map, Value};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// App-owned clipboard shared across panel instances; it outlives layer changes.
#[derive(Clone, Debug, Default)]
pub struct EffectClipboard(Arc<Mutex<Vec<Value>>>);
impl EffectClipboard {
    pub fn len(&self) -> usize {
        self.0.lock().expect("clipboard").len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn clear(&self) {
        self.0.lock().expect("clipboard").clear();
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EffectSelection {
    pub layer: Option<String>,
    pub ids: Vec<String>,
    pub anchor: Option<String>,
    pub focus: Option<String>,
}
impl EffectSelection {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub(crate) fn retain(&mut self, ordered: &[String]) {
        let available = ordered.iter().collect::<HashSet<_>>();
        self.ids.retain(|id| available.contains(id));
        if self.anchor.as_ref().is_some_and(|id| !ordered.contains(id)) {
            self.anchor = None;
        }
        if self.focus.as_ref().is_some_and(|id| !ordered.contains(id)) {
            self.focus = None;
        }
    }
    pub fn select(
        &mut self,
        layer: &str,
        ordered: &[String],
        id: &str,
        mods: Modifiers,
    ) -> Result<(), InspectorError> {
        let end = ordered
            .iter()
            .position(|i| i == id)
            .ok_or_else(|| InspectorError::MissingEffect(id.into()))?;
        if self.layer.as_deref() != Some(layer) {
            self.clear();
        }
        self.layer = Some(layer.into());
        if let Some(start) = self
            .anchor
            .as_ref()
            .and_then(|a| ordered.iter().position(|i| i == a))
            .filter(|_| mods.shift)
        {
            let range = &ordered[start.min(end)..=start.max(end)];
            if mods.command() {
                let mut selected = self.ids.iter().cloned().collect::<HashSet<_>>();
                for id in range {
                    if selected.insert(id.clone()) {
                        self.ids.push(id.clone());
                    }
                }
            } else {
                self.ids = range.to_vec();
            }
        } else if mods.command() {
            if let Some(at) = self.ids.iter().position(|i| i == id) {
                self.ids.remove(at);
            } else {
                self.ids.push(id.into());
            }
        } else {
            self.ids = vec![id.into()];
        }
        self.anchor = Some(id.into());
        self.focus = Some(id.into());
        Ok(())
    }
    /// Background/outside clears; nested controls and parameter surfaces retain.
    pub fn pointer_outside(&mut self, inside_effect_surface: bool) {
        if !inside_effect_surface {
            self.clear();
        }
    }
    pub fn navigate(
        &mut self,
        ordered: &[String],
        focused: &str,
        direction: i32,
        shift: bool,
    ) -> Result<(), InspectorError> {
        let at = ordered
            .iter()
            .position(|i| i == focused)
            .ok_or_else(|| InspectorError::MissingEffect(focused.into()))?;
        let to = (at as i64 + i64::from(direction)).clamp(0, ordered.len() as i64 - 1) as usize;
        if to == at {
            return Ok(());
        }
        if shift {
            let start = self
                .anchor
                .as_ref()
                .and_then(|a| ordered.iter().position(|id| id == a))
                .unwrap_or(at);
            self.ids = ordered[start.min(to)..=start.max(to)].to_vec();
        } else {
            self.ids = vec![ordered[to].clone()];
        }
        self.anchor.get_or_insert_with(|| focused.into());
        self.focus = Some(ordered[to].clone());
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectUiAction {
    Navigate(i32),
    Reorder(i32),
    Copy,
    Paste,
    Delete,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyDisposition {
    Decline,
    Consume(EffectUiAction),
}
impl EffectUiAction {
    /// Only the row route calls this: focused nested fields retain their keys.
    pub fn key(key: &str, mods: Modifiers, nested_control: bool) -> KeyDisposition {
        if nested_control {
            return KeyDisposition::Decline;
        }
        let action = match key.to_ascii_lowercase().as_str() {
            "arrowup" => {
                if mods.alt {
                    Self::Reorder(-1)
                } else {
                    Self::Navigate(-1)
                }
            }
            "arrowdown" => {
                if mods.alt {
                    Self::Reorder(1)
                } else {
                    Self::Navigate(1)
                }
            }
            "backspace" | "delete" => Self::Delete,
            "c" if mods.command() => Self::Copy,
            "v" if mods.command() => Self::Paste,
            _ => return KeyDisposition::Decline,
        };
        KeyDisposition::Consume(action)
    }
}
impl Inspector {
    pub fn effect_ids(&self, layer: &str) -> Result<Vec<String>, InspectorError> {
        Ok(array(self.layer(layer)?.get("fx"))
            .iter()
            .map(|f| text(f, "id").to_owned())
            .collect())
    }
    pub fn select_effect(
        &mut self,
        layer: &str,
        id: &str,
        mods: Modifiers,
    ) -> Result<(), InspectorError> {
        let ordered = self.effect_ids(layer)?;
        self.effects.select(layer, &ordered, id, mods)
    }
    /// Expansion changes UI state only; clicking the row never expands it.
    pub fn set_effect_disclosure(
        &mut self,
        layer: &str,
        id: &str,
        open: bool,
    ) -> Result<(), InspectorError> {
        self.effect(layer, id)?;
        self.disclosure.insert((layer.into(), id.into()), open);
        Ok(())
    }
    pub fn effect_disclosure(&self, layer: &str, id: &str) -> Result<bool, InspectorError> {
        let effect = self.effect(layer, id)?;
        Ok(self
            .disclosure
            .get(&(layer.into(), id.into()))
            .copied()
            .unwrap_or(effect.get("open").and_then(Value::as_bool) != Some(false)))
    }
    pub(crate) fn effect(&self, layer: &str, id: &str) -> Result<Value, InspectorError> {
        array(self.layer(layer)?.get("fx"))
            .iter()
            .find(|f| text(f, "id") == id)
            .cloned()
            .ok_or_else(|| InspectorError::MissingEffect(id.into()))
    }
    pub fn toggle_effect(
        &mut self,
        caller: Caller,
        layer: &str,
        id: &str,
        time: f64,
    ) -> Result<Request, InspectorError> {
        let enabled = self
            .evaluate(
                &crate::PropertyId::new(layer, format!("{id}.$enabled")),
                time,
            )?
            .as_bool()
            .unwrap_or(true);
        self.transaction(
            caller,
            vec![
                EffectCommand::Set {
                    layer_id: layer.into(),
                    effect: id.into(),
                    patch: EffectPatch {
                        enabled: Some(!enabled),
                        time,
                        ..Default::default()
                    },
                }
                .into(),
            ],
        )
    }
    pub fn reorder_effect(
        &self,
        caller: Caller,
        layer: &str,
        id: &str,
        index: usize,
    ) -> Result<Request, InspectorError> {
        let ordered = self.effect_ids(layer)?;
        let from = ordered
            .iter()
            .position(|i| i == id)
            .ok_or_else(|| InspectorError::MissingEffect(id.into()))?;
        let to = index.min(ordered.len() - 1);
        if from == to {
            return Err(InspectorError::EmptyEdit);
        }
        self.transaction(
            caller,
            vec![
                EffectCommand::Set {
                    layer_id: layer.into(),
                    effect: id.into(),
                    patch: EffectPatch {
                        index: Some(to),
                        ..Default::default()
                    },
                }
                .into(),
            ],
        )
    }
    /// The host supplies semantic insertion slot (before row / after last).
    pub fn drop_effect(
        &self,
        caller: Caller,
        layer: &str,
        id: &str,
        slot: usize,
    ) -> Result<Request, InspectorError> {
        let ordered = self.effect_ids(layer)?;
        let from = ordered
            .iter()
            .position(|i| i == id)
            .ok_or_else(|| InspectorError::MissingEffect(id.into()))?;
        let slot = slot.min(ordered.len());
        self.reorder_effect(caller, layer, id, if slot > from { slot - 1 } else { slot })
    }
    fn active_effects(&self, layer: &str, fallback: &str) -> Result<Vec<Value>, InspectorError> {
        let l = self.layer(layer)?;
        let fx = array(l.get("fx"));
        let selected = if self.effects.layer.as_deref() == Some(layer) {
            self.effects
                .ids
                .iter()
                .filter(|id| fx.iter().any(|f| text(f, "id") == id.as_str()))
                .cloned()
                .collect::<Vec<_>>()
        } else {
            vec![]
        };
        let selected = if selected.is_empty() {
            vec![fallback.into()]
        } else {
            selected
        };
        let out = fx
            .iter()
            .filter(|f| selected.iter().any(|id| id == text(f, "id")))
            .cloned()
            .collect::<Vec<_>>();
        if out.is_empty() {
            return Err(InspectorError::MissingEffect(fallback.into()));
        }
        Ok(out)
    }
    pub fn copy_effects(&mut self, layer: &str, fallback: &str) -> Result<usize, InspectorError> {
        let copied = self.active_effects(layer, fallback)?;
        self.effects.layer = Some(layer.into());
        self.effects.ids = copied.iter().map(|f| text(f, "id").into()).collect();
        self.effects.anchor = Some(fallback.into());
        let count = copied.len();
        *self.clipboard.0.lock().expect("clipboard") = copied;
        Ok(count)
    }
    /// Apply this selection ONLY after the kernel has acknowledged success.
    /// A refused delete must not discard the user's current effect selection.
    pub fn delete_effects(
        &self,
        caller: Caller,
        layer: &str,
        fallback: &str,
    ) -> Result<(Request, EffectSelection), InspectorError> {
        let removing = self.active_effects(layer, fallback)?;
        let ids = removing
            .iter()
            .map(|f| text(f, "id").to_owned())
            .collect::<Vec<_>>();
        let ordered = self.effect_ids(layer)?;
        let first = ordered
            .iter()
            .position(|id| ids.contains(id))
            .expect("available");
        let remaining = ordered
            .into_iter()
            .filter(|id| !ids.contains(id))
            .collect::<Vec<_>>();
        let next = remaining
            .get(first.min(remaining.len().saturating_sub(1)))
            .cloned();
        let request = self.transaction(
            caller,
            ids.into_iter()
                .map(|id| {
                    EffectCommand::Remove {
                        layer_id: layer.into(),
                        effect: id,
                    }
                    .into()
                })
                .collect(),
        )?;
        Ok((
            request,
            EffectSelection {
                layer: Some(layer.into()),
                ids: next.clone().into_iter().collect(),
                anchor: next.clone(),
                focus: next,
            },
        ))
    }
    pub fn paste_effects(
        &self,
        caller: Caller,
        layer: &str,
    ) -> Result<(Request, EffectSelection), InspectorError> {
        self.layer(layer)?;
        let saved = self.clipboard.0.lock().expect("clipboard").clone();
        let mut commands = Vec::new();
        let mut ids = Vec::new();
        for fx in saved {
            let Some(definition) = self.saved_effect_definition(&fx) else {
                continue;
            };
            let id = pm_kernel::new_id("f");
            ids.push(id.clone());
            let on = fx.get("on");
            commands.push(
                EffectCommand::Add(Box::new(AddEffect {
                    layer_id: layer.into(),
                    effect_id: id,
                    definition: definition.clone(),
                    parameters: fx
                        .get("p")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_else(Map::new),
                    inputs: None,
                    open: fx.get("open").and_then(Value::as_bool) != Some(false),
                    enabled: on
                        .and_then(|v| {
                            if v.get("kf").is_some() {
                                v.get("v")
                            } else {
                                Some(v)
                            }
                        })
                        .and_then(Value::as_bool)
                        != Some(false),
                    enabled_animation: on.filter(|v| v.get("kf").is_some()).cloned(),
                    index: None,
                }))
                .into(),
            );
        }
        let request = self.transaction(caller, commands)?;
        let last = ids.last().cloned();
        Ok((
            request,
            EffectSelection {
                layer: Some(layer.into()),
                ids,
                anchor: last.clone(),
                focus: last,
            },
        ))
    }
    pub fn add_effect_at(
        &mut self,
        caller: Caller,
        layer: &str,
        kind: &str,
        time: f64,
    ) -> Result<Request, InspectorError> {
        if !time.is_finite() {
            return Err(InspectorError::InvalidInput);
        }
        let definition = self
            .definitions
            .get(kind)
            .cloned()
            .ok_or_else(|| InspectorError::MissingDefinition(kind.into()))?;
        let definition = self.contextual_effect_definition(layer, definition, time)?;
        self.transaction(
            caller,
            vec![
                EffectCommand::Add(Box::new(AddEffect {
                    layer_id: layer.into(),
                    effect_id: pm_kernel::new_id("f"),
                    definition,
                    parameters: Map::new(),
                    inputs: None,
                    open: true,
                    enabled: true,
                    enabled_animation: None,
                    index: None,
                }))
                .into(),
            ],
        )
    }
}
impl Inspector {
    /// The inspector service's global paste targets the raw selected layers
    /// (index.ts:56–75), whereas a row-local paste targets its own layer.
    pub fn paste_selected_effects(&self, caller: Caller) -> Result<Request, InspectorError> {
        let mut commands = Vec::new();
        for layer in self.published.selection.layers() {
            if text(&self.layer(layer)?, "type") == "audio" {
                continue;
            }
            let (request, _) = self.paste_effects(caller, layer)?;
            if let pm_kernel::Call::Transact {
                body: pm_kernel::WriteBody::Commands(batch),
                ..
            } = request.call
            {
                commands.extend(batch);
            }
        }
        self.transaction(caller, commands)
    }
}
