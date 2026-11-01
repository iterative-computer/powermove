use crate::view::{array, number, text};
use crate::{Inspector, InspectorError, PropertyId};
use pm_kernel::commands::{
    comps::{CompCommand, CompPatch},
    keyframes::{EaseSpec, ReplaceKeyframes},
    layers::{LayerCommand, LayerPatch},
    properties::{KeyMode, SetProperty},
};
use pm_kernel::{
    Call, Caller, Command, ContentCommand, EditCall, KeyframeCommand, PropertyCommand, Request,
    WriteBody,
};
use pm_model::{Map, Value};
use std::collections::{HashMap, HashSet};

/// Fields are semantic addresses, shared by native inputs and agent adapters.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Binding {
    Property(PropertyId),
    Content {
        layer: String,
        field: String,
    },
    /// Layer fields with animated counterparts use Property(l.<field>).
    Layer {
        layer: String,
        field: String,
    },
    Composition(String),
    SceneParameter(String),
}
impl Binding {
    pub fn property_id(&self) -> Option<PropertyId> {
        match self {
            Self::Property(id) => Some(id.clone()),
            Self::Content { layer, field } => Some(PropertyId::new(layer, format!("c.{field}"))),
            Self::Layer { layer, field } => Some(PropertyId::new(
                layer,
                format!("l.{}", storage_field(field)),
            )),
            _ => None,
        }
    }
    pub fn layer_id(&self) -> Option<&str> {
        match self {
            Self::Property(id) => Some(&id.layer),
            Self::Content { layer, .. } | Self::Layer { layer, .. } => Some(layer),
            _ => None,
        }
    }
    fn retarget(&self, layer: &str, path: Option<String>) -> Self {
        match self {
            Self::Property(id) => Self::Property(PropertyId::new(
                layer,
                path.unwrap_or_else(|| id.path.clone()),
            )),
            Self::Content { field, .. } => Self::Content {
                layer: layer.into(),
                field: field.clone(),
            },
            Self::Layer { field, .. } => Self::Layer {
                layer: layer.into(),
                field: field.clone(),
            },
            _ => self.clone(),
        }
    }
}
fn storage_field(field: &str) -> &str {
    match field {
        "visible" => "on",
        "motionBlur" => "mblur",
        _ => field,
    }
}

/// No host services execute synchronously here. The visual host consumes ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceIntent {
    FocusGraph(PropertyId),
    Reveal(Vec<PropertyId>),
    OpenGraph(PropertyId),
    ActivePath {
        layer: String,
        path: String,
        pen: bool,
    },
    ImportSource {
        layer: String,
        kind: String,
    },
    EnsureFont {
        family: String,
    },
    RefreshFonts,
}
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyAction {
    Diamond,
    Stopwatch,
    AddKey,
    Ease(String),
    Expression(Option<String>),
    Reset(Option<Value>),
}

/// Fixed from-start values for one live gesture. Store this until the kernel
/// acknowledges commit/cancel. Selection/time changes cannot retarget a drag.
#[derive(Clone, Debug)]
pub struct EditSession {
    pub(crate) time: f64,
    document: u64,
    owner: Option<pm_kernel::Writer>,
    pub(crate) bindings: Vec<Binding>,
    pub(crate) initial: HashMap<Binding, Value>,
    pub(crate) targets: HashMap<Binding, Vec<Binding>>,
    pub(crate) keys: HashMap<PropertyId, Vec<Value>>,
    pairs: HashMap<Binding, (Binding, Option<f64>, bool)>,
}
impl Inspector {
    pub(crate) fn translate(
        &self,
        primary: &str,
        target: &str,
        path: &str,
    ) -> Result<Option<String>, InspectorError> {
        let source = self.layer(primary)?;
        let dest = self.layer(target)?;
        if let Some(k) = path.strip_prefix("c.") {
            return Ok(pm_eval::address::can_animate_content(
                self.layer_node(target)?,
                k.as_bytes(),
            )
            .then(|| path.into()));
        }
        if path.starts_with("l.") {
            return Ok((text(&dest, "type") != "audio" || path == "l.on").then(|| path.into()));
        }
        if dest.get("p").and_then(|p| p.get(path)).is_some() {
            return Ok(Some(path.into()));
        }
        if path.starts_with("u.") || path.starts_with("x.") {
            return Ok((text(&source, "type") == text(&dest, "type")
                && (text(&source, "type") != "extension"
                    || source.get("d").and_then(|d| d.get("definition"))
                        == dest.get("d").and_then(|d| d.get("definition"))))
            .then(|| path.into()));
        }
        if ["g.", "mp.", "ta.", "ts."]
            .iter()
            .any(|p| path.starts_with(p))
        {
            return Ok((primary == target).then(|| path.into()));
        }
        if let Some(rest) = path.strip_prefix("m.") {
            let Some((id, key)) = rest.split_once('.') else {
                return Ok(None);
            };
            return Ok(array(source.get("masks"))
                .iter()
                .position(|m| text(m, "id") == id)
                .and_then(|i| array(dest.get("masks")).get(i))
                .map(|m| format!("m.{}.{key}", text(m, "id"))));
        }
        let Some((id, key)) = path.split_once('.') else {
            return Ok(None);
        };
        Ok(array(source.get("fx"))
            .iter()
            .position(|f| text(f, "id") == id)
            .and_then(|i| {
                let a = &array(source.get("fx"))[i];
                let b = array(dest.get("fx")).get(i)?;
                (text(a, "type") == text(b, "type")).then(|| format!("{}.{key}", text(b, "id")))
            }))
    }
    pub(crate) fn binding_targets(
        &self,
        binding: &Binding,
    ) -> Result<Vec<Binding>, InspectorError> {
        let Some(primary) = binding.layer_id() else {
            return Ok(vec![binding.clone()]);
        };
        self.layer_node(primary)?;
        let selected: Vec<&str> = if self.selected.first().is_some_and(|id| id == primary)
            && self.published.selection.layers().len() > 1
        {
            self.selected.iter().map(String::as_str).collect()
        } else {
            vec![primary]
        };
        let mut out = Vec::new();
        for target in selected {
            if !self.editable(target) {
                continue;
            }
            match binding {
                Binding::Property(id) => {
                    if let Some(path) = if primary == target {
                        Some(id.path.clone())
                    } else {
                        self.translate(primary, target, &id.path)?
                    } {
                        out.push(binding.retarget(target, Some(path)));
                    }
                }
                Binding::Content { field, .. } => {
                    let src = self.layer(primary)?;
                    let dst = self.layer(target)?;
                    if text(&src, "type") == text(&dst, "type")
                        || (pm_eval::address::can_animate_content(
                            self.layer_node(primary)?,
                            field.as_bytes(),
                        ) && pm_eval::address::can_animate_content(
                            self.layer_node(target)?,
                            field.as_bytes(),
                        ))
                    {
                        out.push(binding.retarget(target, None));
                    }
                }
                _ => out.push(binding.retarget(target, None)),
            }
        }
        Ok(out)
    }
    pub(crate) fn read_binding(
        &mut self,
        binding: &Binding,
        time: f64,
    ) -> Result<Value, InspectorError> {
        match binding {
            Binding::Property(id) => self.evaluate(id, time),
            Binding::Content { layer, field } => {
                match self.evaluate(&PropertyId::new(layer, format!("c.{field}")), time) {
                    Err(InspectorError::MissingProperty(_)) => Ok(Value::Null),
                    result => result,
                }
            }
            Binding::Layer { layer, field } => {
                let raw = self.layer(layer)?;
                let value = raw
                    .get(storage_field(field))
                    .cloned()
                    .unwrap_or(Value::Null);
                if value.get("kf").is_some() {
                    self.evaluate(
                        &PropertyId::new(layer, format!("l.{}", storage_field(field))),
                        time,
                    )
                } else {
                    Ok(value)
                }
            }
            Binding::Composition(field) => Ok(self
                .composition()
                .get(match field.as_str() {
                    "width" => "w",
                    "height" => "h",
                    "duration" => "dur",
                    _ => field,
                })
                .cloned()
                .unwrap_or(Value::Null)),
            Binding::SceneParameter(name) => Ok(self
                .composition()
                .get("params")
                .and_then(|p| p.get(name))
                .and_then(|p| p.get("value"))
                .cloned()
                .unwrap_or(Value::Null)),
        }
    }
    /// Absolute typed/select/toggle edit. Numeric selected keys offset from
    /// their evaluated value, exactly as multi-edit.ts:117–140.
    pub fn set(
        &mut self,
        caller: Caller,
        binding: Binding,
        value: Value,
        time: f64,
    ) -> Result<Request, InspectorError> {
        let mut session = self.capture(&[binding], time)?;
        let commands = self.session_commands(&mut session, &[value], false)?;
        self.transaction(caller, commands)
    }
    /// Paired effect coordinates are one batch with independent channel ids.
    pub fn set_many(
        &mut self,
        caller: Caller,
        bindings: &[Binding],
        values: &[Value],
        time: f64,
    ) -> Result<Request, InspectorError> {
        let mut session = self.capture(bindings, time)?;
        let commands = self.session_commands(&mut session, values, false)?;
        self.transaction(caller, commands)
    }
    pub fn begin_edit(
        &mut self,
        caller: Caller,
        bindings: &[Binding],
        time: f64,
    ) -> Result<(EditSession, Request), InspectorError> {
        let mut session = self.capture(bindings, time)?;
        session.owner = Some(caller.writer);
        if session.initial.is_empty() {
            return Err(InspectorError::EmptyEdit);
        }
        Ok((
            session,
            Request::new(
                caller,
                Call::Edit(EditCall::Begin {
                    label: "Edit Content".into(),
                }),
            ),
        ))
    }
    pub fn edit_step(
        &mut self,
        caller: Caller,
        session: &mut EditSession,
        values: &[Value],
    ) -> Result<Request, InspectorError> {
        if session.document != self.published.snapshot.doc_id
            || session.owner != Some(caller.writer)
        {
            return Err(InspectorError::InvalidInput);
        }
        let commands = self.session_commands(session, values, true)?;
        if commands.is_empty() {
            return Err(InspectorError::EmptyEdit);
        }
        let mut commands = commands;
        if caller.origin.is_generated() {
            for command in &mut commands {
                protect_intent(command);
            }
        }
        Ok(Request::new(
            caller,
            Call::Edit(EditCall::Dispatch {
                body: WriteBody::Commands(commands),
            }),
        ))
    }
    pub fn finish_edit(&self, caller: Caller, cancel: bool) -> Request {
        Request::new(
            caller,
            Call::Edit(if cancel {
                EditCall::Cancel
            } else {
                EditCall::Commit { label: None }
            }),
        )
    }
    fn capture(&mut self, bindings: &[Binding], time: f64) -> Result<EditSession, InspectorError> {
        if !time.is_finite() || bindings.is_empty() {
            return Err(InspectorError::InvalidInput);
        }
        let mut initial = HashMap::new();
        let mut keys = HashMap::new();
        let mut targets = HashMap::new();
        let mut pairs = HashMap::new();
        let mut expanded = bindings.to_vec();
        for binding in bindings {
            if let Binding::Property(id) = binding
                && matches!(id.path.as_str(), "scale.x" | "scale.y")
            {
                let other = Binding::Property(PropertyId::new(
                    &id.layer,
                    if id.path == "scale.x" {
                        "scale.y"
                    } else {
                        "scale.x"
                    },
                ));
                let linked = self
                    .layer(&id.layer)?
                    .get("scaleLinked")
                    .and_then(Value::as_bool)
                    == Some(true);
                let own_prop = self.property(id)?;
                let other_prop = self.property(&other.property_id().expect("property"))?;
                let animated = [own_prop, other_prop]
                    .iter()
                    .any(|p| p.as_ref().is_some_and(|p| !array(p.get("kf")).is_empty()));
                if (linked || animated) && !bindings.contains(&other) {
                    let own = self
                        .read_binding(binding, time)?
                        .as_f64()
                        .ok_or(InspectorError::InvalidInput)?;
                    let paired = self
                        .read_binding(&other, time)?
                        .as_f64()
                        .ok_or(InspectorError::InvalidInput)?;
                    pairs.insert(
                        binding.clone(),
                        (
                            other.clone(),
                            linked.then_some(if own.abs() > 1e-8 { paired / own } else { 1.0 }),
                            animated,
                        ),
                    );
                    expanded.push(other);
                }
            }
        }
        let selected_keys: HashSet<String> =
            self.published.selection.keys().iter().cloned().collect();
        for binding in &expanded {
            // A locked primary may still define a shared edit for editable peers.
            initial.insert(binding.clone(), self.read_binding(binding, time)?);
            let captured = self.binding_targets(binding)?;
            targets.insert(binding.clone(), captured.clone());
            for target in captured {
                initial.insert(target.clone(), self.read_binding(&target, time)?);
                if let Binding::Property(id) = &target {
                    let prop = self.property(id)?;
                    let selected = array(prop.as_ref().and_then(|p| p.get("kf")))
                        .iter()
                        .filter(|k| {
                            selected_keys.contains(text(k, "i"))
                                && k.get("v").and_then(Value::as_f64).is_some()
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    if selected.len() >= 2 {
                        keys.insert(id.clone(), selected);
                    }
                }
            }
        }
        Ok(EditSession {
            time,
            document: self.published.snapshot.doc_id,
            owner: None,
            bindings: bindings.to_vec(),
            initial,
            keys,
            targets,
            pairs,
        })
    }
    fn session_commands(
        &mut self,
        session: &mut EditSession,
        values: &[Value],
        relative: bool,
    ) -> Result<Vec<Command>, InspectorError> {
        if values.len() != session.bindings.len() {
            return Err(InspectorError::InvalidInput);
        }
        let mut commands = Vec::new();
        let mut expanded = Vec::new();
        for (binding, value) in session.bindings.iter().zip(values) {
            let pair = session.pairs.get(binding);
            expanded.push((binding.clone(), value.clone(), pair.is_some_and(|p| p.2)));
            if let Some((other, ratio, animated)) = pair {
                let next = match ratio {
                    Some(ratio) => {
                        Value::Number(value.as_f64().ok_or(InspectorError::InvalidInput)? * ratio)
                    }
                    None => session.initial[other].clone(),
                };
                expanded.push((other.clone(), next, *animated));
            }
        }
        for (binding, value, force_keyframe) in &expanded {
            // Resolve ids from the gesture's fixed bindings, preserving the
            // original target set even if selection changes mid-gesture.
            let targets = session.targets.get(binding).cloned().unwrap_or_default();
            for target in targets {
                if target.layer_id().is_some_and(|id| !self.editable(id)) {
                    continue;
                }
                let next = if relative {
                    match (
                        value.as_f64(),
                        session.initial.get(binding).and_then(Value::as_f64),
                        session.initial.get(&target).and_then(Value::as_f64),
                    ) {
                        (Some(v), Some(base), Some(own)) => Value::Number(own + v - base),
                        _ => value.clone(),
                    }
                } else {
                    value.clone()
                };
                if let Binding::Property(id) = &target
                    && let Some(keys) = session.keys.get(id).filter(|_| next.as_f64().is_some())
                {
                    let base = session.initial[&target]
                        .as_f64()
                        .ok_or(InspectorError::InvalidInput)?;
                    for key in keys {
                        let mut c = SetProperty::new(
                            &id.layer,
                            &id.path,
                            number(key, "v", 0.0) + next.as_f64().expect("checked") - base,
                            number(&self.layer(&id.layer)?, "from", 0.0) + number(key, "t", 0.0),
                        );
                        c.mode = KeyMode::Keyframe;
                        c.preserve_hand_edits = false;
                        commands.push(PropertyCommand::Set(c).into());
                    }
                    continue;
                }
                let mut next_commands = self.value_commands(&target, next, session.time)?;
                if *force_keyframe {
                    for command in &mut next_commands {
                        if let Command::Property(PropertyCommand::Set(c)) = command {
                            c.mode = KeyMode::Keyframe;
                        }
                    }
                }
                commands.extend(next_commands);
            }
        }
        Ok(commands)
    }
    fn value_commands(
        &mut self,
        binding: &Binding,
        value: Value,
        time: f64,
    ) -> Result<Vec<Command>, InspectorError> {
        let command = match binding {
            Binding::Property(id) => {
                let mut c = SetProperty::new(&id.layer, &id.path, value, time);
                c.preserve_hand_edits = false;
                c.mark_intent = self
                    .layer_node(&id.layer)?
                    .get("p")
                    .and_then(|p| p.get(&id.path))
                    .is_some();
                PropertyCommand::Set(c).into()
            }
            Binding::Content { layer, field } => {
                let mut patch = Map::new();
                patch.insert(field.as_str(), value);
                ContentCommand::Set {
                    layer_id: layer.clone(),
                    patch,
                    time,
                }
                .into()
            }
            Binding::Layer { layer, field } => {
                let mut patch = LayerPatch {
                    time,
                    ..Default::default()
                };
                match field.as_str() {
                    "visible" => patch.visible = Some(boolean(&value)?),
                    "solo" => patch.solo = Some(boolean(&value)?),
                    "threeD" => patch.three_d = Some(boolean(&value)?),
                    "scaleLinked" => patch.scale_linked = Some(boolean(&value)?),
                    "motionBlur" => patch.motion_blur = Some(boolean(&value)?),
                    "continuousRaster" => patch.continuous_raster = Some(boolean(&value)?),
                    "color" => patch.color = Some(string(&value)?),
                    "name" => patch.name = Some(string(&value)?),
                    "blend" => patch.blend = Some(string(&value)?),
                    "parent" => {
                        patch.parent = Some(pm_kernel::commands::layers::ParentChange {
                            parent: nullable_string(&value)?,
                            time,
                        })
                    }
                    "matteSource" => patch.matte_source = Some(nullable_string(&value)?),
                    _ => return Err(InspectorError::InvalidInput),
                }
                LayerCommand::Set {
                    layer_id: layer.clone(),
                    patch,
                }
                .into()
            }
            Binding::Composition(field) => {
                let mut patch = CompPatch::default();
                match field.as_str() {
                    "width" => patch.width = Some(numeric(&value)?),
                    "height" => patch.height = Some(numeric(&value)?),
                    "duration" => patch.duration = Some(numeric(&value)?),
                    "fps" => patch.fps = Some(numeric(&value)?),
                    "backgroundFill" => patch.background_fill = Some(value),
                    _ => return Err(InspectorError::InvalidInput),
                }
                CompCommand::Set {
                    patch: Box::new(patch),
                }
                .into()
            }
            Binding::SceneParameter(name) => CompCommand::SetSceneParameter {
                name: name.clone(),
                value,
                label: None,
                control: None,
                min: None,
                max: None,
                options: None,
            }
            .into(),
        };
        Ok(vec![command])
    }
    pub(crate) fn transaction(
        &self,
        caller: Caller,
        mut commands: Vec<Command>,
    ) -> Result<Request, InspectorError> {
        if commands.is_empty() {
            return Err(InspectorError::EmptyEdit);
        }
        if caller.origin.is_generated() {
            for command in &mut commands {
                protect_intent(command);
            }
        }
        Ok(Request::new(
            caller,
            Call::Transact {
                label: None,
                body: WriteBody::Commands(commands),
                expected_revision: Some(self.revision()),
            },
        ))
    }
    /// Full typed command vocabulary for masks, path/text/caption structure,
    /// alignment, parenting and retiming. Kernel validates it atomically.
    pub fn commands(
        &self,
        caller: Caller,
        commands: Vec<Command>,
    ) -> Result<Request, InspectorError> {
        self.transaction(caller, commands)
    }
    pub fn property_action(
        &mut self,
        caller: Caller,
        ids: &[PropertyId],
        action: PropertyAction,
        time: f64,
    ) -> Result<Request, InspectorError> {
        if !time.is_finite() {
            return Err(InspectorError::InvalidInput);
        }
        let mut targets = Vec::new();
        let mut seen = HashSet::new();
        for id in ids {
            for b in self.binding_targets(&Binding::Property(id.clone()))? {
                if let Binding::Property(id) = b
                    && seen.insert(id.clone())
                {
                    targets.push(id);
                }
            }
        }
        let mut commands = Vec::new();
        let remove = !targets.is_empty()
            && targets
                .iter()
                .all(|id| self.key_at(id, time).ok().flatten().is_some());
        let disable = ids.iter().any(|id| {
            self.property(id)
                .ok()
                .flatten()
                .is_some_and(|p| !array(p.get("kf")).is_empty())
        });
        for id in targets {
            match &action {
                PropertyAction::Diamond if remove => {
                    if let Some(k) = self.key_at(&id, time)? {
                        commands.push(
                            KeyframeCommand::Delete {
                                key_ids: vec![text(&k, "i").into()],
                            }
                            .into(),
                        );
                    }
                }
                PropertyAction::Diamond | PropertyAction::AddKey => {
                    if self.key_at(&id, time)?.is_none() {
                        commands.push(
                            KeyframeCommand::Add {
                                layer_id: id.layer.clone(),
                                channel: id.path.clone(),
                                time,
                                key_id: pm_kernel::new_id("k"),
                                value: Some(self.evaluate(&id, time)?),
                            }
                            .into(),
                        );
                    }
                }
                PropertyAction::Stopwatch => {
                    let value = self.evaluate(&id, time)?;
                    if disable {
                        commands.push(replace(&id, time, None));
                    }
                    let mut c = SetProperty::new(&id.layer, &id.path, value, time);
                    c.mode = if disable {
                        KeyMode::Static
                    } else {
                        KeyMode::Keyframe
                    };
                    c.preserve_hand_edits = false;
                    commands.push(PropertyCommand::Set(c).into());
                }
                PropertyAction::Expression(expression) => commands.push(
                    PropertyCommand::SetExpression {
                        layer_id: id.layer.clone(),
                        channel: id.path.clone(),
                        expression: expression
                            .as_ref()
                            .map(|e| e.trim().to_owned())
                            .filter(|e| !e.is_empty()),
                    }
                    .into(),
                ),
                PropertyAction::Ease(preset) => {
                    if ![
                        "power",
                        "linear",
                        "easeInOut",
                        "expoOut",
                        "backOut",
                        "glide",
                        "snap",
                    ]
                    .contains(&preset.as_str())
                    {
                        return Err(InspectorError::InvalidInput);
                    }
                    let keys = self
                        .property(&id)?
                        .map(|p| {
                            array(p.get("kf"))
                                .iter()
                                .map(|k| text(k, "i").into())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    // The kernel caps SetEasing to 1000 ids per command.
                    for chunk in keys.chunks(1000) {
                        commands.push(
                            KeyframeCommand::SetEasing {
                                key_ids: chunk.to_vec(),
                                curve: EaseSpec::Preset(preset.clone()),
                            }
                            .into(),
                        );
                    }
                }
                PropertyAction::Reset(default) => {
                    let default = default
                        .clone()
                        .or_else(|| self.channel_default(&id).ok().flatten());
                    commands.push(replace(&id, time, Some(None)));
                    if let Some(value) = default {
                        let mut c = SetProperty::new(&id.layer, &id.path, value, time);
                        c.mode = KeyMode::Static;
                        c.preserve_hand_edits = false;
                        commands.push(PropertyCommand::Set(c).into());
                    }
                }
            }
        }
        self.transaction(caller, commands)
    }
    fn channel_default(&self, id: &PropertyId) -> Result<Option<Value>, InspectorError> {
        let layer = self.layer(&id.layer)?;
        let kind = text(&layer, "type");
        let value = match id.path.as_str() {
            "anchor.x" | "anchor.y" => Some(if kind == "null" { 50.0 } else { 0.0 }),
            "position.x" => Some(if ["group", "adjustment"].contains(&kind) {
                0.0
            } else {
                number(&self.composition(), "w", 0.0) / 2.0
            }),
            "position.y" => Some(if ["group", "adjustment"].contains(&kind) {
                0.0
            } else {
                number(&self.composition(), "h", 0.0) / 2.0
            }),
            "scale.x" | "scale.y" | "scale.z" => Some(100.0),
            "opacity" => Some(if kind == "null" { 0.0 } else { 100.0 }),
            "perspective" => Some(50.0),
            "anchor.z" | "position.z" | "rotation" | "rotation.x" | "rotation.y"
            | "orientation.x" | "orientation.y" | "orientation.z" | "skew" => Some(0.0),
            _ => None,
        };
        if let Some(v) = value {
            return Ok(Some(v.into()));
        }
        if let Some((effect, key)) = id.path.split_once('.') {
            let effect = array(layer.get("fx"))
                .iter()
                .find(|f| text(f, "id") == effect);
            if let Some(effect) = effect {
                return Ok(self
                    .definitions
                    .get(text(effect, "type"))
                    .and_then(|d| d.params.iter().find(|p| p.key == key))
                    .map(|p| p.default.clone()));
            }
        }
        Ok(None)
    }
}
fn replace(id: &PropertyId, time: f64, expression: Option<Option<String>>) -> Command {
    KeyframeCommand::Replace(Box::new(ReplaceKeyframes {
        layer_id: id.layer.clone(),
        channel: id.path.clone(),
        keyframes: vec![],
        replace: true,
        expression,
        preserve_hand_edits: false,
        time,
    }))
    .into()
}
fn numeric(v: &Value) -> Result<f64, InspectorError> {
    v.as_f64()
        .filter(|n| n.is_finite())
        .ok_or(InspectorError::InvalidInput)
}
fn boolean(v: &Value) -> Result<bool, InspectorError> {
    v.as_bool().ok_or(InspectorError::InvalidInput)
}
fn string(v: &Value) -> Result<String, InspectorError> {
    v.as_str()
        .map(str::to_owned)
        .ok_or(InspectorError::InvalidInput)
}
fn nullable_string(v: &Value) -> Result<Option<String>, InspectorError> {
    if v.is_null() {
        Ok(None)
    } else {
        string(v).map(Some)
    }
}
fn protect_intent(command: &mut Command) {
    match command {
        Command::Property(PropertyCommand::Set(c)) => {
            c.preserve_hand_edits = true;
            c.mark_intent = false;
        }
        Command::Keyframe(KeyframeCommand::Replace(c)) => c.preserve_hand_edits = true,
        _ => {}
    }
}
