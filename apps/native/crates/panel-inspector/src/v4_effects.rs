//! Thin worker adapter for the built-in v4 Effect Controls panel.
//!
//! The application registers `panel.effect-controls` and the evaluated channel
//! `panel:layer-effects`. It supplies the real Caller, time, registry display
//! metadata and published Inspector. `plan` never submits or waits. For a request,
//! the host must await the actual kernel result, sync, and call `acknowledge` only
//! after success. No document copy, synchronous JS, or platform UI is owned here.
use crate::view::{array, number, object, text};
use crate::{Binding, EffectSelection, Inspector, InspectorError, PropertyAction, PropertyId};
use pm_kernel::{Call, Caller, Command, EffectCommand, Reply, Request, WriteBody};
use pm_model::Value;
use std::collections::HashMap;
use std::ops::Range;

/// Runtime registry display metadata, carried unchanged (not guessed from ids).
#[derive(Clone, Debug, Default)]
pub struct Presentation {
    pub layer_type_labels: HashMap<String, String>,
    pub parameters: HashMap<(String, String), ParameterPresentation>,
}
#[derive(Clone, Debug)]
pub struct ParameterPresentation {
    pub label: String,
    pub control: String,
    pub step: Option<f64>,
    pub unit: Option<String>,
    pub precision: Option<usize>,
    pub options: Vec<Value>,
}
#[derive(Clone)]
struct FlatRow {
    id: String,
    effect: usize,
    parameter: Option<(PropertyId, String, Option<crate::ParamDefinition>)>,
    action: Option<crate::effect_definitions::ColorAction>,
}
#[derive(Default)]
pub struct EffectsPanel {
    pub presentation: Presentation,
    range: Range<usize>,
    cache_key: Option<(u64, u64, String, Vec<bool>)>,
    cached_definitions: HashMap<String, crate::EffectDefinition>,
    cached_definition_generation: u64,
    flat: Vec<FlatRow>,
}
/// A document plan's selection must not be installed before kernel success.
#[derive(Debug)]
pub struct Plan {
    pub request: Option<Request>,
    pub selection: Option<EffectSelection>,
    layer: String,
    document: u64,
}
impl Plan {
    fn pin_document(mut self) -> Self {
        if let Some(request) = &mut self.request {
            request.expected_document = Some(self.document);
        }
        self
    }
}
impl EffectsPanel {
    pub fn new(presentation: Presentation, range: Range<usize>) -> Self {
        Self {
            presentation,
            range,
            ..Self::default()
        }
    }
    fn rebuild(
        &mut self,
        model: &Inspector,
        layer: &str,
        fx: &[Value],
    ) -> Result<(), InspectorError> {
        let open = fx
            .iter()
            .map(|f| model.effect_disclosure(layer, text(f, "id")))
            .collect::<Result<Vec<_>, _>>()?;
        let key = (
            model.published.snapshot.doc_id,
            model.revision(),
            layer.to_owned(),
            open.clone(),
        );
        if self.cache_key.as_ref() == Some(&key)
            && self.cached_definitions == model.definitions
            && self.cached_definition_generation == model.definition_generation
        {
            return Ok(());
        }
        let anchor = self
            .cache_key
            .as_ref()
            .filter(|old| old.0 == key.0 && old.2 == key.2)
            .and_then(|_| self.flat.get(self.range.start))
            .map(|row| row.id.clone());
        let length = self.range.end.saturating_sub(self.range.start);
        self.flat.clear();
        for (index, effect) in fx.iter().enumerate() {
            self.flat.push(FlatRow {
                id: format!("effect:{}", text(effect, "id")),
                effect: index,
                parameter: None,
                action: None,
            });
            if !open[index] {
                continue;
            }
            let id = text(effect, "id");
            for action in model.color_actions(effect)? {
                let mut action_id = format!("action:{id}:{}", action.index);
                while fx.iter().any(|f| text(f, "id") == action_id) {
                    action_id.push('#');
                }
                self.flat.push(FlatRow {
                    id: action_id,
                    effect: index,
                    parameter: None,
                    action: Some(action),
                });
            }
            match model.effect_parameters(layer, id) {
                Ok(params) => self
                    .flat
                    .extend(params.into_iter().map(|(id, param)| FlatRow {
                        id: pm_model::json::to_canonical_string(&id.value()),
                        effect: index,
                        parameter: Some((id, param.key.clone(), Some(param))),
                        action: None,
                    })),
                Err(InspectorError::MissingDefinition(_)) => {
                    if let Some(params) = effect.get("p").and_then(Value::as_object) {
                        self.flat.extend(params.iter().map(|(key, _)| {
                            FlatRow {
                                id: pm_model::json::to_canonical_string(
                                    &PropertyId::new(
                                        layer,
                                        format!("{id}.{}", key.to_string_lossy()),
                                    )
                                    .value(),
                                ),
                                effect: index,
                                action: None,
                                parameter: Some((
                                    PropertyId::new(
                                        layer,
                                        format!("{id}.{}", key.to_string_lossy()),
                                    ),
                                    key.to_string_lossy(),
                                    None,
                                )),
                            }
                        }));
                    }
                }
                Err(error) => return Err(error),
            }
        }
        if let Some(index) = anchor.and_then(|id| self.flat.iter().position(|row| row.id == id)) {
            self.range = index..index.saturating_add(length);
        }
        self.cached_definitions = model.definitions.clone();
        self.cached_definition_generation = model.definition_generation;
        self.cache_key = Some(key);
        Ok(())
    }
    /// Values are evaluated only for the requested flattened header/parameter window.
    pub fn project(&mut self, model: &mut Inspector, time: f64) -> Result<Value, InspectorError> {
        if !time.is_finite() {
            return Err(InspectorError::InvalidInput);
        }
        let layer = model.primary().map(str::to_owned);
        let mut out = object([
            ("version", 1.into()),
            ("target", "NoSelection".into()),
            ("layer", Value::Null),
            ("name", "".into()),
            ("typeLabel", "".into()),
            ("locked", false.into()),
            ("revision", (model.revision() as f64).into()),
            (
                "documentId",
                (model.published.snapshot.doc_id as f64).into(),
            ),
            ("count", 0.into()),
            ("effectCount", 0.into()),
            ("selectedCount", 0.into()),
            ("range", Value::Array(vec![0.into(), 0.into()])),
            ("rows", Value::Array(vec![])),
            ("focus", Value::Null),
            ("focusLabel", Value::Null),
            ("clipboardCount", (model.clipboard.len() as f64).into()),
        ]);
        let Some(layer) = layer else {
            return Ok(out);
        };
        let value = model.layer(&layer)?;
        let kind = text(&value, "type");
        let supported = !["audio", "camera"].contains(&kind);
        let map = out.as_object_mut().expect("object");
        map.insert(
            "target",
            if supported { "Layer" } else { "Unsupported" }.into(),
        );
        map.insert("layer", layer.clone().into());
        map.insert("name", text(&value, "name").into());
        map.insert(
            "typeLabel",
            self.presentation
                .layer_type_labels
                .get(kind)
                .cloned()
                .unwrap_or_default()
                .into(),
        );
        map.insert("locked", (!model.editable(&layer)).into());
        if !supported {
            return Ok(out);
        }
        let fx = array(value.get("fx"));
        self.rebuild(model, &layer, fx)?;
        let start = self.range.start.min(self.flat.len());
        let end = self.range.end.max(start).min(self.flat.len());
        let selected = model.effects.layer.as_deref() == Some(&layer);
        map.insert("count", (self.flat.len() as f64).into());
        map.insert("effectCount", (fx.len() as f64).into());
        map.insert(
            "selectedCount",
            (if selected { model.effects.ids.len() } else { 0 } as f64).into(),
        );
        map.insert(
            "range",
            Value::Array(vec![(start as f64).into(), (end as f64).into()]),
        );
        map.insert(
            "focus",
            if selected {
                model.effects.focus.clone().map_or(Value::Null, Value::from)
            } else {
                Value::Null
            },
        );
        let focus_label = if selected {
            model
                .effects
                .focus
                .as_ref()
                .and_then(|id| fx.iter().find(|f| text(f, "id") == id))
                .map(|f| {
                    model
                        .definitions
                        .get(text(f, "type"))
                        .map(|d| d.label.clone())
                        .unwrap_or_else(|| text(f, "type").into())
                })
        } else {
            None
        };
        map.insert("focusLabel", focus_label.map_or(Value::Null, Value::from));
        let mut rows = Vec::with_capacity(end - start);
        for entry in &self.flat[start..end] {
            let effect = &fx[entry.effect];
            let id = text(effect, "id");
            let definition = model.saved_effect_definition(effect);
            let label = definition
                .as_ref()
                .map(|d| d.label.clone())
                .unwrap_or_else(|| text(effect, "type").into());
            let enabled = enabled(model, &layer, effect, time)?;
            if let Some(action) = &entry.action {
                rows.push(object([
                    ("kind", "action".into()),
                    ("id", entry.id.clone().into()),
                    ("effect", id.into()),
                    ("index", (action.index as f64).into()),
                    ("label", action.label.clone().into()),
                    ("readonly", (!model.editable(&layer)).into()),
                ]));
                continue;
            }
            if let Some((property, key, param)) = &entry.parameter {
                let saved_display = model.saved_parameter_presentation(effect, key);
                let display = saved_display.as_ref().or_else(|| {
                    self.presentation
                        .parameters
                        .get(&(text(effect, "type").into(), key.clone()))
                });
                let raw = effect
                    .get("p")
                    .and_then(|p| p.get(key))
                    .cloned()
                    .unwrap_or(Value::Null);
                let state = if param.is_some() {
                    Some(model.binding_state(&Binding::Property(property.clone()), time)?)
                } else {
                    None
                };
                let mut p = object([
                    (
                        "id",
                        pm_model::json::to_canonical_string(&property.value()).into(),
                    ),
                    ("effect", id.into()),
                    ("key", key.clone().into()),
                    (
                        "label",
                        display
                            .map(|d| d.label.clone())
                            .unwrap_or_else(|| pm_surface::humanize(key))
                            .into(),
                    ),
                    (
                        "binding",
                        format!(
                            "effect-parameter:{}:{}",
                            model.document_id(),
                            pm_model::json::to_canonical_string(&property.value())
                        )
                        .into(),
                    ),
                    (
                        "value",
                        state
                            .as_ref()
                            .map(|s| s.value.clone())
                            .unwrap_or(raw.clone()),
                    ),
                    (
                        "control",
                        if param.is_none() {
                            "opaque"
                        } else {
                            display.map(|d| d.control.as_str()).unwrap_or("opaque")
                        }
                        .into(),
                    ),
                    (
                        "animated",
                        state.as_ref().is_some_and(|s| s.animated).into(),
                    ),
                    (
                        "keyAtPlayhead",
                        state.as_ref().is_some_and(|s| s.key_at_playhead).into(),
                    ),
                    ("mixed", state.as_ref().is_some_and(|s| s.mixed).into()),
                    (
                        "readonly",
                        (param.is_none()
                            || !model.editable(&layer)
                            || display.is_none_or(|d| d.control == "opaque"))
                        .into(),
                    ),
                ]);
                let fields = p.as_object_mut().expect("parameter");
                // Without a definition the stored data is kept whole in `value`;
                // `display` is what the row shows: the value at rest, or
                // Animated when keyed.
                if state.is_none() {
                    let keyed = raw
                        .get("kf")
                        .and_then(Value::as_array)
                        .is_some_and(|k| !k.is_empty());
                    let display = match raw.get("v") {
                        Some(_) if keyed => Some(Value::String("Animated".into())),
                        Some(v) => Some(v.clone()),
                        None => None,
                    };
                    if let Some(display) = display {
                        fields.insert("display", display);
                    }
                }
                if let Some(d) = display {
                    if let Some(step) = d.step {
                        fields.insert("step", step.into());
                    }
                    if let Some(unit) = &d.unit {
                        fields.insert("unit", unit.clone().into());
                    }
                    if let Some(precision) = d.precision {
                        fields.insert("precision", (precision as f64).into());
                    }
                    if !d.options.is_empty() {
                        fields.insert("options", Value::Array(d.options.clone()));
                    }
                }
                if let Some(d) = param {
                    if let Some(min) = d.min {
                        fields.insert("min", min.into());
                    }
                    if let Some(max) = d.max {
                        fields.insert("max", max.into());
                    }
                }
                rows.push(object([
                    ("kind", "parameter".into()),
                    (
                        "id",
                        format!(
                            "parameter:{}",
                            pm_model::json::to_canonical_string(&property.value())
                        )
                        .into(),
                    ),
                    ("parameter", p),
                    ("effectLabel", label.into()),
                    ("enabled", enabled.into()),
                ]));
            } else {
                rows.push(object([
                    ("kind", "effect".into()),
                    ("id", id.into()),
                    ("definition", text(effect, "type").into()),
                    ("grade", model.is_grade(effect).into()),
                    ("label", label.into()),
                    ("enabled", enabled.into()),
                    ("expanded", model.effect_disclosure(&layer, id)?.into()),
                    ("missing", definition.is_none().into()),
                    (
                        "selected",
                        (selected && model.effects.ids.iter().any(|s| s == id)).into(),
                    ),
                    ("ordinal", (entry.effect as f64).into()),
                    ("effectCount", (fx.len() as f64).into()),
                ]));
            }
        }
        map.insert("rows", Value::Array(rows));
        Ok(out)
    }
    /// Resolve one semantic intent. The host validates project/realm/generation
    /// and request-id dedupe before entering this adapter.
    pub fn plan(
        &mut self,
        model: &mut Inspector,
        caller: Caller,
        args: &Value,
        time: f64,
    ) -> Result<Plan, InspectorError> {
        if !time.is_finite()
            || number(args, "revision", -1.0) != model.revision() as f64
            || number(args, "documentId", -1.0) != model.published.snapshot.doc_id as f64
        {
            return Err(InspectorError::InvalidInput);
        }
        let layer = model.primary().unwrap_or_default().to_owned();
        if text(args, "layer") != layer {
            return Err(InspectorError::InvalidInput);
        }
        let intent = args.get("intent").ok_or(InspectorError::InvalidInput)?;
        let action = text(intent, "type");
        let mut plan = Plan {
            request: None,
            selection: None,
            layer: layer.clone(),
            document: model.published.snapshot.doc_id,
        };
        if action == "range" {
            let range = array(intent.get("range"));
            if range.len() != 2 {
                return Err(InspectorError::InvalidInput);
            }
            let start = ordinal(&range[0])?;
            let end = ordinal(&range[1])?;
            if end < start {
                return Err(InspectorError::InvalidInput);
            }
            self.range = start..end;
            return Ok(plan.pin_document());
        }
        if action == "clear" {
            model.effects.clear();
            return Ok(plan.pin_document());
        }
        if !model.editable(&layer)
            || ["audio", "camera"].contains(&text(&model.layer(&layer)?, "type"))
        {
            return Err(InspectorError::InvalidInput);
        }
        let id = text(intent, "id");
        let ids = model.effect_ids(&layer)?;
        let direction = || -> Result<i32, InspectorError> {
            match number(intent, "direction", 0.0) {
                -1.0 => Ok(-1),
                1.0 => Ok(1),
                _ => Err(InspectorError::InvalidInput),
            }
        };
        let label;
        match action {
            "select" => {
                model.select_effect(
                    &layer,
                    id,
                    crate::input::Modifiers {
                        meta: intent.get("meta").and_then(Value::as_bool) == Some(true),
                        ctrl: intent.get("ctrl").and_then(Value::as_bool) == Some(true),
                        shift: intent.get("shift").and_then(Value::as_bool) == Some(true),
                        ..Default::default()
                    },
                )?;
                return Ok(plan.pin_document());
            }
            "navigate" => {
                let focused = model
                    .effects
                    .focus
                    .clone()
                    .or_else(|| ids.first().cloned())
                    .ok_or(InspectorError::EmptyEdit)?;
                model.effects.layer = Some(layer.clone());
                model.effects.navigate(
                    &ids,
                    &focused,
                    direction()?,
                    intent.get("shift").and_then(Value::as_bool) == Some(true),
                )?;
                return Ok(plan.pin_document());
            }
            "disclose" => {
                model.set_effect_disclosure(
                    &layer,
                    id,
                    intent
                        .get("open")
                        .and_then(Value::as_bool)
                        .ok_or(InspectorError::InvalidInput)?,
                )?;
                return Ok(plan.pin_document());
            }
            "copy" => {
                model.copy_effects(&layer, id)?;
                return Ok(plan.pin_document());
            }
            "toggle" => {
                let effect = model.effect(&layer, id)?;
                let enabled = enabled(model, &layer, &effect, time)?;
                plan.request = Some(model.toggle_effect(caller, &layer, id, time)?);
                label = if enabled {
                    "Disable Effect"
                } else {
                    "Enable Effect"
                }
                .to_owned();
            }
            "remove" => {
                let (r, s) = model.delete_effects(caller, &layer, id)?;
                let count = if let Call::Transact {
                    body: WriteBody::Commands(commands),
                    ..
                } = &r.call
                {
                    commands.len()
                } else {
                    0
                };
                label = if count > 1 {
                    "Remove Effects"
                } else {
                    "Remove Effect"
                }
                .to_owned();
                plan.request = Some(r);
                plan.selection = Some(s);
            }
            "paste" => {
                let (r, s) = model.paste_effects(caller, &layer)?;
                label = if s.ids.len() > 1 {
                    "Paste Effects"
                } else {
                    "Paste Effect"
                }
                .to_owned();
                plan.request = Some(r);
                plan.selection = Some(s);
            }
            "reorder" => {
                let index = ids
                    .iter()
                    .position(|s| s == id)
                    .ok_or_else(|| InspectorError::MissingEffect(id.into()))?;
                let next = (index as i64 + i64::from(direction()?)).clamp(0, ids.len() as i64 - 1)
                    as usize;
                plan.request = Some(model.reorder_effect(caller, &layer, id, next)?);
                label = "Reorder Effect".into();
            }
            "drop" => {
                plan.request = Some(model.drop_effect(
                    caller,
                    &layer,
                    id,
                    ordinal(intent.get("slot").ok_or(InspectorError::InvalidInput)?)?,
                )?);
                label = "Reorder Effect".into();
            }
            "action" => {
                let effect = model.effect(&layer, id)?;
                let index = ordinal(intent.get("index").ok_or(InspectorError::InvalidInput)?)?;
                let action = model
                    .color_actions(&effect)?
                    .into_iter()
                    .find(|a| a.index == index)
                    .ok_or(InspectorError::InvalidInput)?;
                let bindings = action
                    .keys
                    .iter()
                    .map(|key| PropertyId::new(&layer, format!("{id}.{key}")))
                    .collect::<Vec<_>>();
                let first = model
                    .binding_state(&Binding::Property(bindings[0].clone()), time)?
                    .value;
                let second = model
                    .binding_state(&Binding::Property(bindings[1].clone()), time)?
                    .value;
                let commands = bindings
                    .into_iter()
                    .zip([second, first])
                    .map(|(id, value)| {
                        let mut c = pm_kernel::commands::properties::SetProperty::new(
                            id.layer, id.path, value, time,
                        );
                        c.preserve_hand_edits = false;
                        pm_kernel::Command::Property(
                            pm_kernel::commands::properties::PropertyCommand::Set(c),
                        )
                    })
                    .collect();
                plan.request = Some(model.commands(caller, commands)?);
                label = action.label;
            }
            "reset" => {
                let effect = model.effect(&layer, id)?;
                let definition = model.saved_effect_definition(&effect).ok_or_else(|| {
                    InspectorError::MissingDefinition(text(&effect, "type").into())
                })?;
                let definition = model.contextual_effect_definition(&layer, definition, time)?;
                let commands: Vec<Command> = definition
                    .params
                    .iter()
                    .filter(|p| effect.get("p").and_then(|v| v.get(&p.key)).is_some())
                    .map(|p| {
                        // AE-style effect Reset: the default keyed at the playhead,
                        // existing animation kept. The parameter row's Reset clears it.
                        let mut c = pm_kernel::commands::properties::SetProperty::new(
                            &layer,
                            format!("{id}.{}", p.key),
                            p.default.clone(),
                            time,
                        );
                        c.preserve_hand_edits = false;
                        pm_kernel::Command::Property(
                            pm_kernel::commands::properties::PropertyCommand::Set(c),
                        )
                    })
                    .collect();
                if !commands.is_empty() {
                    plan.request = Some(model.commands(caller, commands)?);
                }
                label = format!("Reset {}", definition.label);
            }
            "add" => {
                let kind = text(intent, "definition");
                let definition = model
                    .definitions
                    .get(kind)
                    .ok_or_else(|| InspectorError::MissingDefinition(kind.into()))?;
                label = format!("Add {}", definition.label);
                let mut request = model.add_effect_at(caller, &layer, kind, time)?;
                if let Call::Transact {
                    body: WriteBody::Commands(commands),
                    ..
                } = &mut request.call
                {
                    for command in commands {
                        if let Command::Effect(EffectCommand::Add(add)) = command {
                            if let Some(slot) = intent.get("slot") {
                                add.index = Some(ordinal(slot)?.min(ids.len()));
                            }
                            plan.selection = Some(EffectSelection {
                                layer: Some(layer.clone()),
                                ids: vec![add.effect_id.clone()],
                                anchor: Some(add.effect_id.clone()),
                                focus: Some(add.effect_id.clone()),
                            });
                        }
                    }
                }
                plan.request = Some(request);
            }
            "parameter" => {
                let address =
                    pm_model::json::parse(id).map_err(|_| InspectorError::InvalidInput)?;
                let property = PropertyId::from_value(&address)?;
                // Only an installed effect parameter in this target is addressable.
                let allowed = ids.iter().any(|fx| {
                    model
                        .effect_parameters(&layer, fx)
                        .is_ok_and(|p| p.iter().any(|(p, _)| p == &property))
                });
                if property.layer != layer || !allowed {
                    return Err(InspectorError::InvalidInput);
                }
                let removing_animation = text(intent, "action") == "stopwatch";
                if removing_animation
                    && !model
                        .binding_state(&Binding::Property(property.clone()), time)?
                        .animated
                {
                    return Ok(plan.pin_document());
                }
                let action = match text(intent, "action") {
                    "diamond" => PropertyAction::Diamond,
                    "stopwatch" => PropertyAction::Stopwatch,
                    // v1 B367: keys and expression removed, then the static default.
                    "reset" => {
                        // The same saved-version, layer-sized default as the effect Reset.
                        let (fx, key) = property
                            .path
                            .split_once('.')
                            .ok_or(InspectorError::InvalidInput)?;
                        let effect = model.effect(&layer, fx)?;
                        let definition =
                            model.saved_effect_definition(&effect).ok_or_else(|| {
                                InspectorError::MissingDefinition(text(&effect, "type").into())
                            })?;
                        let definition =
                            model.contextual_effect_definition(&layer, definition, time)?;
                        let default = definition
                            .params
                            .into_iter()
                            .find(|p| p.key == key)
                            .map(|p| p.default);
                        PropertyAction::Reset(default)
                    }
                    _ => return Err(InspectorError::InvalidInput),
                };
                plan.request = Some(model.property_action(caller, &[property], action, time)?);
                label = match text(intent, "action") {
                    // Keep the kernel's exact action-specific undo name for diamond
                    // add/remove, rather than labelling a removal as an addition.
                    "diamond" => return Ok(plan.pin_document()),
                    "reset" => "Reset".into(),
                    _ => "Remove Animation".into(),
                };
            }
            _ => return Err(InspectorError::InvalidInput),
        }
        if let Some(Request {
            call:
                Call::Transact {
                    label: request_label,
                    ..
                },
            ..
        }) = &mut plan.request
        {
            *request_label = Some(label);
        }
        Ok(plan.pin_document())
    }
    pub fn acknowledge(
        &mut self,
        model: &mut Inspector,
        plan: Plan,
        ack: &pm_kernel::Ack,
    ) -> Result<(), InspectorError> {
        if matches!(ack.reply, Reply::Throw(_))
            || model.revision() != ack.revision
            || model.published.snapshot.doc_id != plan.document
        {
            return Err(InspectorError::InvalidInput);
        }
        if model.primary() == Some(&plan.layer)
            && let Some(selection) = plan.selection
        {
            model.effects = selection;
        }
        Ok(())
    }
}
fn ordinal(value: &Value) -> Result<usize, InspectorError> {
    let number = value.as_f64().ok_or(InspectorError::InvalidInput)?;
    if !number.is_finite()
        || number < 0.0
        || number.fract() != 0.0
        || number > (1u64 << 53) as f64 - 1.0
    {
        return Err(InspectorError::InvalidInput);
    }
    usize::try_from(number as u64).map_err(|_| InspectorError::InvalidInput)
}

fn enabled(
    model: &mut Inspector,
    layer: &str,
    effect: &Value,
    time: f64,
) -> Result<bool, InspectorError> {
    if effect.get("on").is_none() {
        return Ok(true);
    }
    Ok(model
        .binding_state(
            &Binding::Property(PropertyId::new(
                layer,
                format!("{}.$enabled", text(effect, "id")),
            )),
            time,
        )?
        .value
        .as_bool()
        != Some(false))
}
