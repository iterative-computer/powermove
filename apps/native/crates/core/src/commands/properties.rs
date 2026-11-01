//! Property channels (`{ v, kf, expr }`): values, expressions, the
//! stopwatch, and reset.
//!
//! Ports `setProperty` and `setExpression` of `legacy/core/editing.ts`,
//! `PM.toggleStopwatch` of `legacy/core/anim.ts`, and the inspector's
//! channel-row actions (`extensions/inspector/ChannelRow.svelte`).

use pm_model::{Map, Value};

use super::channels::{self, Step};
use super::keyframes::EaseSpec;
use super::{Cx, new_id, track, values};
use crate::error::CommandError;
use crate::model::LayerLoc;

/// How `set_property` writes (`mode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyMode {
    /// Key at `time` when the channel already has keys, else the static
    /// value (the inspector and canvas).
    #[default]
    Auto,
    /// Always key at `time` (starts animation on a static channel).
    Keyframe,
    /// Always the static value `v`, even on an animated channel (its keys
    /// keep winning; the old `mode: 'static'` behaved the same).
    Static,
}

/// `set_property`.
#[derive(Clone, Debug, PartialEq)]
pub struct SetProperty {
    pub layer_id: String,
    /// The channel key (see the module docs of [`super`]).
    pub channel: String,
    /// The new value. A numeric channel needs a finite number
    /// (`<channel> must be a finite number`); colors are `#RRGGBB[AA]`
    /// strings, switches booleans, menus strings.
    pub value: Value,
    /// The playhead (composition seconds): where a key is written.
    pub time: f64,
    pub mode: KeyMode,
    /// Ease of a key this command creates (default linear); with an
    /// explicit ease an existing key's ease is replaced too.
    pub ease: Option<EaseSpec>,
    /// Hold flag of the written key.
    pub hold: Option<bool>,
    /// `preserveHandEdits` (default true): refuse a channel with a
    /// hand-edit marker. Generated origins always preserve.
    pub preserve_hand_edits: bool,
    /// `markIntent`: record a hand-edit marker (`locked_intent[channel]`)
    /// so agents leave the value alone.
    pub mark_intent: bool,
}

impl SetProperty {
    /// An interface edit with the defaults (`mode: auto`, linear, preserve).
    pub fn new(
        layer_id: impl Into<String>,
        channel: impl Into<String>,
        value: impl Into<Value>,
        time: f64,
    ) -> SetProperty {
        SetProperty {
            layer_id: layer_id.into(),
            channel: channel.into(),
            value: value.into(),
            time,
            mode: KeyMode::Auto,
            ease: None,
            hold: None,
            preserve_hand_edits: true,
            mark_intent: false,
        }
    }
}

/// Property commands.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PropertyCommand {
    /// `set_property` (`setProperty`): the static value, or a key at `time`
    /// (layer-local, snapped to a frame; an existing key within half a frame
    /// is updated). Changing a video's constant `c.speed` rescales its
    /// duration. Label "Change <Channel label>" (e.g. "Change Opacity").
    Set(SetProperty),
    /// `set_expression` (`setExpression`): `None` or empty removes it.
    /// Refuses an expression the expression engine rejects. Generated
    /// origins may not overwrite a hand-edited 3D channel. Label "Set
    /// Expression" / "Remove Expression".
    SetExpression {
        layer_id: String,
        channel: String,
        expression: Option<String>,
    },
    /// The stopwatch (`PM.toggleStopwatch`): animated → static at the value
    /// shown at `time` (keys removed); static → one key at `time` holding
    /// the static value. Label "Enable Animation" / "Remove Animation".
    ToggleStopwatch {
        layer_id: String,
        channel: String,
        time: f64,
    },
    /// Reset (channel-row menu): back to the channel's default value (keys
    /// and expression removed). Needs the channel schema defaults
    /// (`PM.CH`, effect definitions); not ported.
    Reset {
        layer_id: String,
        channel: String,
        time: f64,
    },
}

impl PropertyCommand {
    pub fn label(&self) -> String {
        match self {
            PropertyCommand::Set(c) => format!("Change {}", channel_label(&c.channel)),
            PropertyCommand::SetExpression {
                expression: Some(e),
                ..
            } if !e.is_empty() => "Set Expression".into(),
            PropertyCommand::SetExpression { .. } => "Remove Expression".into(),
            PropertyCommand::ToggleStopwatch { .. } => "Toggle Animation".into(),
            PropertyCommand::Reset { .. } => "Reset Property".into(),
        }
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        match self {
            PropertyCommand::Set(c) => set_property(cx, c),
            PropertyCommand::SetExpression {
                layer_id,
                channel,
                expression,
            } => set_expression(cx, layer_id, channel, expression.as_deref()),
            PropertyCommand::ToggleStopwatch {
                layer_id,
                channel,
                time,
            } => toggle_stopwatch(cx, layer_id, channel, *time),
            PropertyCommand::Reset { .. } => Err(CommandError::NotImplemented("reset_property")),
        }
    }
}

/// The layer and its channel path, after the lock check and the
/// `property()` materialization (`Property “x” was not found on “name”`).
pub(crate) fn resolve(
    cx: &Cx<'_>,
    layer_id: &str,
    channel: &str,
) -> Result<(LayerLoc, Value, String, Vec<Step>), CommandError> {
    let (loc, mut layer) = cx.layer_mut_checked(layer_id)?;
    let key = values::channel_key(channel).to_owned();
    let path =
        channels::materialize(&mut layer, &key).ok_or_else(|| CommandError::PropertyNotFound {
            layer_id: layer_id.to_owned(),
            name: values::layer_name(&layer, layer_id),
            channel: key.clone(),
        })?;
    Ok((loc, layer, key, path))
}

/// `lockedIntent(layer, channel)`: a marker on the channel or its root.
pub(crate) fn locked_intent(layer: &Value, channel: &str) -> bool {
    let Some(intent) = layer.get("locked_intent") else {
        return false;
    };
    let root = channel.split('.').next().unwrap_or(channel);
    intent.get(channel).is_some_and(Value::is_truthy)
        || intent.get(root).is_some_and(Value::is_truthy)
}

/// `policyCommand`: extensions always preserve hand edits; the interface and
/// agents may ask to overwrite them.
pub(crate) fn preserves(cx: &Cx<'_>, requested: bool) -> bool {
    requested || cx.origin() == super::Origin::Extension
}

fn hand_edit(layer_id: &str, channel: &str) -> CommandError {
    CommandError::PreservedHandEdit {
        layer_id: layer_id.to_owned(),
        channel: channel.to_owned(),
    }
}

/// Whether the channel holds numbers (`typeof current === 'number'`).
fn numeric_channel(prop: &Value) -> bool {
    let keys = track::keys_of(prop);
    match keys.first() {
        Some(k) => k.get("v").and_then(Value::as_f64).is_some(),
        None => prop.get("v").and_then(Value::as_f64).is_some(),
    }
}

/// `constantVideoSpeed(layer)`.
fn constant_video_speed(layer: &Value) -> Option<f64> {
    if values::text(layer, "type") != Some("video") {
        return None;
    }
    let d = layer.get("d")?;
    if let Some(remap) = d.get("timeRemap") {
        let animated = if values::is_property(remap) {
            remap.get("v").is_some_and(Value::is_truthy)
                || !track::keys_of(remap).is_empty()
                || remap.get("expr").is_some_and(Value::is_truthy)
        } else {
            remap.is_truthy()
        };
        if animated {
            return None;
        }
    }
    let speed = d.get("speed");
    if let Some(s) = speed.filter(|s| values::is_property(s))
        && (!track::keys_of(s).is_empty() || s.get("expr").is_some_and(Value::is_truthy))
    {
        return None;
    }
    let value = match speed {
        Some(s) if values::is_property(s) => s.get("v").map(track::js_number).unwrap_or(f64::NAN),
        Some(Value::Null) | None => 1.0,
        Some(s) => track::js_number(s),
    };
    (value.is_finite() && value > 0.0).then_some(value)
}

/// `resizeVideoForSpeed(layer, previous)`.
pub(crate) fn resize_video_for_speed(layer: &mut Value, previous: Option<f64>, fps: f64) {
    let next = constant_video_speed(layer);
    let (Some(previous), Some(next)) = (previous, next) else {
        return;
    };
    if previous == next {
        return;
    }
    let dur = values::num(layer, "dur").unwrap_or(f64::NAN);
    let duration = dur * previous / next;
    if !duration.is_finite() {
        return;
    }
    values::obj(layer).insert("dur", Value::Number((1.0 / fps).max(duration)));
}

pub(crate) fn previous_speed(layer: &Value) -> Option<f64> {
    constant_video_speed(layer)
}

fn ease_name(ease: Option<&EaseSpec>) -> Result<Option<String>, CommandError> {
    match ease {
        None => Ok(None),
        Some(EaseSpec::Preset(name)) => Ok(Some(name.clone())),
        Some(EaseSpec::Curve(_)) => Err(CommandError::Invalid(
            "set_property takes a named ease; use set_easing for a curve".into(),
        )),
    }
}

/// `setProperty(command)`.
pub(crate) fn set_property(cx: &mut Cx<'_>, c: &SetProperty) -> Result<(), CommandError> {
    let (loc, mut layer, channel, path) = resolve(cx, &c.layer_id, &c.channel)?;
    if preserves(cx, c.preserve_hand_edits) && locked_intent(&layer, &channel) {
        return Err(hand_edit(&c.layer_id, &channel));
    }
    let time = values::finite(c.time, "time")?;
    let fps = cx.fps(loc.comp_id.as_deref());
    let ease = ease_name(c.ease.as_ref())?;
    let prop = channels::get(&layer, &path).cloned().unwrap_or_default();
    let mut value = c.value.clone();
    if numeric_channel(&prop) {
        value = Value::Number(values::finite(track::js_number(&value), &channel)?);
    }
    let animated = !track::keys_of(&prop).is_empty();
    let keyed = match c.mode {
        KeyMode::Keyframe => true,
        KeyMode::Auto => animated,
        KeyMode::Static => false,
    };
    let local = track::local_time(&layer, time);
    if keyed {
        let prop = channels::get_mut(&mut layer, &path).expect("resolved");
        let before = track::keys_of(prop).len();
        let (id, _) = track::set_key_on(
            prop,
            local,
            value,
            ease.as_deref().unwrap_or("linear"),
            fps,
            true,
            || new_id("k"),
        );
        let created = track::keys_of(prop).len() != before;
        let keys = track::keys_mut(prop);
        if let Some(hold) = c.hold
            && let Some(i) = track::index_of(keys, &id)
        {
            track::set_hold_flag(keys, i, hold);
        }
        // Plain updates keep native velocity; new keys and explicit presets
        // get their preset against the final neighbours.
        if created || ease.is_some() {
            let name = ease.clone().unwrap_or_else(|| "linear".into());
            track::apply_presets(keys, &[(id, name, None)]);
        }
    } else {
        let speed_before = (channel == "c.speed")
            .then(|| previous_speed(&layer))
            .flatten();
        let prop = channels::get_mut(&mut layer, &path).expect("resolved");
        values::obj(prop).insert("v", value);
        if channel == "c.speed" {
            resize_video_for_speed(&mut layer, speed_before, fps);
        }
    }
    if c.mark_intent {
        mark_intent(&mut layer, &channel, local);
    }
    cx.put_layer(&loc, layer)
}

/// `layer.locked_intent[channel] = { by: 'human', at, t }`.
pub(crate) fn mark_intent(layer: &mut Value, channel: &str, local: f64) {
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or_default();
    let mut m = Map::new();
    m.insert("by", values::s("human"));
    m.insert("at", Value::Number(at));
    m.insert("t", Value::Number(track::round3(local)));
    let l = values::obj(layer);
    if l.get("locked_intent").and_then(Value::as_object).is_none() {
        l.insert("locked_intent", Value::Object(Map::new()));
    }
    if let Some(intent) = l.get_mut("locked_intent").and_then(Value::as_object_mut) {
        intent.insert(channel, Value::Object(m));
    }
}

/// `MAX_EXPRESSION_CHARS` of `validate/commands.ts`.
pub const MAX_EXPRESSION_CHARS: usize = 2_000;

/// `expressionDiagnostic(source)`: the engine's parser decides.
pub(crate) fn check_expression(source: &str) -> Result<(), CommandError> {
    if source.encode_utf16().count() > MAX_EXPRESSION_CHARS {
        return Err(CommandError::Invalid(format!(
            "Use an expression of at most {MAX_EXPRESSION_CHARS} characters."
        )));
    }
    pm_expr::Expression::parse(source, pm_expr::Dialect::Native)
        .map(|_| ())
        .map_err(|e| CommandError::Invalid(e.to_string()))
}

/// `setExpression(command)`.
fn set_expression(
    cx: &mut Cx<'_>,
    layer_id: &str,
    channel: &str,
    expression: Option<&str>,
) -> Result<(), CommandError> {
    let (loc, mut layer, _, path) = resolve(cx, layer_id, channel)?;
    let expression = expression.filter(|e| !e.is_empty());
    if let Some(e) = expression {
        check_expression(e)?;
    }
    let prop = channels::get_mut(&mut layer, &path).expect("resolved");
    values::obj(prop).insert("expr", expression.map(values::s).unwrap_or(Value::Null));
    cx.put_layer(&loc, layer)
}

/// `PM.toggleStopwatch(L, key, T)`: animated → static at the value shown at
/// `time`; static → one key at `time` with the static value.
fn toggle_stopwatch(
    cx: &mut Cx<'_>,
    layer_id: &str,
    channel: &str,
    time: f64,
) -> Result<(), CommandError> {
    let (loc, mut layer, _, path) = resolve(cx, layer_id, channel)?;
    let fps = cx.fps(loc.comp_id.as_deref());
    let local = track::local_time(&layer, values::finite(time, "time")?);
    let prop = channels::get_mut(&mut layer, &path).expect("resolved");
    if track::keys_of(prop).is_empty() {
        let v = prop.get("v").cloned().unwrap_or(Value::Null);
        track::set_key_on(prop, local, v, "linear", fps, true, || new_id("k"));
    } else {
        let v = super::keyframes::value_at(prop, local, channel);
        let m = values::obj(prop);
        m.insert("v", v);
        m.insert("kf", Value::Array(Vec::new()));
    }
    cx.put_layer(&loc, layer)
}

/// `PM.CH[key].label` for transform channels, else the key's last part.
pub fn channel_label(channel: &str) -> String {
    let key = super::values::channel_key(channel);
    let known = match key {
        "anchor.x" => "Anchor X",
        "anchor.y" => "Anchor Y",
        "anchor.z" => "Anchor Z",
        "position.x" => "Position X",
        "position.y" => "Position Y",
        "position.z" => "Position Z",
        "scale.x" => "Scale X",
        "scale.y" => "Scale Y",
        "scale.z" => "Scale Z",
        "rotation" => "Rotation",
        "rotation.x" => "Rotation X",
        "rotation.y" => "Rotation Y",
        "opacity" => "Opacity",
        "skew" => "Skew",
        "perspective" => "Perspective",
        "l.blend" => "Blend mode",
        "l.mblur" => "Motion blur",
        "l.on" => "Visibility",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_owned();
    }
    let last = key.rsplit('.').next().unwrap_or(key);
    let last = if last == "$enabled" { "Enabled" } else { last };
    let mut chars = last.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => "Property".into(),
    }
}
