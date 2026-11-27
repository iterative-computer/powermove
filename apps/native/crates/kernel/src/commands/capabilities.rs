//! Generated-tool transforms (`legacy/core/capabilities.ts`). A transform
//! compiles to primitive commands before the edit runs, so history records the
//! real layer and property changes as one Undo step.
use std::collections::HashMap;

use pm_model::{Map, Value};

use super::content::ContentCommand;
use super::layers::{LAYER_TYPES, LayerCommand, LayerPatch, ParentChange};
use super::properties::{PropertyCommand, SetProperty};
use super::{Command, Cx, values};
use crate::error::CommandError;

const MAX_EDITS: usize = 32;
const MAX_EXPR_DEPTH: usize = 8;
const MAX_EXPR_NODES: usize = 160;
const MAX_SOURCE: usize = 80_000;
const LAYER_PATHS: [&str; 11] = [
    "layer.name",
    "layer.from",
    "layer.duration",
    "layer.visible",
    "layer.locked",
    "layer.shy",
    "layer.blend",
    "layer.motionBlur",
    "layer.parent",
    "layer.color",
    "layer.collapsed",
];
const ORDERS: [&str; 8] = [
    "stack",
    "reverseStack",
    "selection",
    "reverseSelection",
    "start",
    "reverseStart",
    "name",
    "random",
];
const REFS: [&str; 6] = [
    "current",
    "index",
    "count",
    "fps",
    "playhead",
    "composition.duration",
];
const AGGREGATES: [&str; 6] = ["min", "max", "first", "last", "sum", "average"];

fn arity(op: &str) -> Option<(usize, usize)> {
    Some(match op {
        "add" | "multiply" => (2, 8),
        "subtract" | "divide" | "equal" => (2, 2),
        "min" | "max" => (1, 8),
        "clamp" | "if" => (3, 3),
        "round" | "floor" | "ceil" | "abs" | "negate" | "frames" => (1, 1),
        _ => return None,
    })
}

/// `/^[a-z][a-z0-9_.-]{0,79}$/i`.
fn state_key(value: &Value) -> Option<&str> {
    let key = value.as_str()?;
    let mut chars = key.chars();
    (chars.next()?.is_ascii_alphabetic()
        && key.len() <= 80
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')))
    .then_some(key)
}
/// `/^<prefix>[a-z0-9_.-]{1,120}$/i` for the two editable prefixes.
fn path_allowed(path: &str) -> bool {
    LAYER_PATHS.contains(&path)
        || ["properties.", "content."].iter().any(|prefix| {
            path.strip_prefix(prefix).is_some_and(|rest| {
                !rest.is_empty()
                    && rest.len() <= 120
                    && rest
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
            })
        })
}
fn plain(value: &Value) -> bool {
    value.is_null()
        || value.as_str().is_some()
        || value.as_bool().is_some()
        || value.as_f64().is_some_and(f64::is_finite)
}

/// `sanitizeExpression`: `None` refuses, keeping `null` a valid literal.
fn sanitize_expression(value: &Value, nodes: &mut usize, depth: usize) -> Option<Value> {
    *nodes += 1;
    if *nodes > MAX_EXPR_NODES || depth > MAX_EXPR_DEPTH {
        return None;
    }
    if plain(value) {
        return Some(value.clone());
    }
    value.as_object()?;
    let mut out = Map::new();
    if let Some(state) = value.get("state").filter(|s| state_key(s).is_some()) {
        out.insert("state", state.clone());
        if let Some(fallback) = value.get("fallback").filter(|f| plain(f)) {
            out.insert("fallback", fallback.clone());
        }
        return Some(Value::Object(out));
    }
    if let Some(reference) = value
        .get("ref")
        .filter(|r| r.as_str().is_some_and(|r| REFS.contains(&r)))
    {
        out.insert("ref", reference.clone());
        return Some(Value::Object(out));
    }
    if let (Some(kind), Some(path)) = (value.get("aggregate"), values::text(value, "path"))
        && kind.as_str().is_some_and(|k| AGGREGATES.contains(&k))
        && path_allowed(path)
    {
        out.insert("aggregate", kind.clone());
        out.insert("path", values::s(path));
        return Some(Value::Object(out));
    }
    let op = values::text(value, "op")?;
    let (min, max) = arity(op)?;
    let mut args = Vec::new();
    let source = value.get("args").and_then(Value::as_array);
    for arg in source.into_iter().flatten().take(8) {
        args.push(sanitize_expression(arg, nodes, depth + 1)?);
    }
    if args.len() < min || args.len() > max {
        return None;
    }
    out.insert("op", values::s(op));
    out.insert("args", Value::Array(args));
    Some(Value::Object(out))
}

struct Edit {
    path: String,
    value: Value,
}
struct Transform {
    scope: String,
    types: Vec<String>,
    order: Value,
    seed: String,
    edits: Vec<Edit>,
}

fn clean_text(value: Option<&Value>, fallback: &str, max: usize) -> String {
    match value.and_then(Value::as_str).map(values::js_trim) {
        Some(text) if !text.is_empty() => text.chars().take(max).collect(),
        _ => fallback.to_owned(),
    }
}
fn sanitize_order(value: Option<&Value>) -> Value {
    if value
        .and_then(Value::as_str)
        .is_some_and(|o| ORDERS.contains(&o))
    {
        return value.cloned().unwrap_or_default();
    }
    if let Some(value) = value
        && let Some(state) = value.get("state").filter(|s| state_key(s).is_some())
    {
        let mut out = Map::new();
        out.insert("state", state.clone());
        out.insert(
            "fallback",
            value
                .get("fallback")
                .filter(|f| f.as_str().is_some_and(|f| ORDERS.contains(&f)))
                .cloned()
                .unwrap_or_else(|| values::s("stack")),
        );
        return Value::Object(out);
    }
    values::s("stack")
}
/// `sanitizeTransform`. Generated tools never include locked layers.
fn sanitize(raw: &Value) -> Option<Transform> {
    let parsed;
    let source = match raw.as_str() {
        Some(text) => {
            if text.len() > MAX_SOURCE {
                return None;
            }
            parsed = pm_model::json::parse(text).ok()?;
            &parsed
        }
        None => raw,
    };
    source.as_object()?;
    let selector = source.get("selector").filter(|s| s.as_object().is_some());
    let scope = selector
        .and_then(|s| values::text(s, "scope"))
        .filter(|s| ["selection", "all", "visible"].contains(s))
        .unwrap_or("selection")
        .to_owned();
    let types = selector
        .and_then(|s| s.get("types"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|t| LAYER_TYPES.contains(t))
        .take(16)
        .map(str::to_owned)
        .collect();
    let mut edits = Vec::new();
    let list = source.get("edits").and_then(Value::as_array);
    for edit in list.into_iter().flatten().take(MAX_EDITS) {
        let path = clean_text(edit.get("path"), "", 140);
        if !path_allowed(&path) {
            continue;
        }
        let mut nodes = 0;
        if let Some(value) = edit
            .get("value")
            .and_then(|raw| sanitize_expression(raw, &mut nodes, 0))
        {
            edits.push(Edit { path, value });
        }
    }
    if edits.is_empty() {
        return None;
    }
    Some(Transform {
        scope,
        types,
        order: sanitize_order(source.get("order")),
        seed: clean_text(source.get("seed"), "powermove", 80),
        edits,
    })
}

/// `Number(value)` for the values transforms carry.
fn number(value: &Value) -> f64 {
    match value {
        Value::Null => 0.,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Number(n) => *n,
        Value::String(s) => pm_model::json::string_to_number(&s.to_string_lossy()),
        _ => f64::NAN,
    }
}
fn truthy(value: &Value) -> bool {
    match value {
        Value::Number(n) => *n != 0. && !n.is_nan(),
        other => other.is_truthy(),
    }
}
fn stable_hash(text: &str) -> u32 {
    text.chars().fold(2_166_136_261u32, |hash, c| {
        let unit = u32::from(c.encode_utf16(&mut [0; 2])[0]);
        (hash ^ unit).wrapping_mul(16_777_619)
    })
}

struct Context<'a> {
    current: Value,
    index: usize,
    targets: &'a [Target],
    state: &'a Value,
    fps: f64,
    time: f64,
    duration: f64,
}
struct Target {
    layer: Value,
    before: HashMap<String, Value>,
}

fn aggregate(kind: &str, path: &str, targets: &[Target]) -> f64 {
    let numbers: Vec<f64> = targets
        .iter()
        .filter_map(|t| t.before.get(path))
        .filter_map(Value::as_f64)
        .filter(|n| n.is_finite())
        .collect();
    if numbers.is_empty() {
        return 0.;
    }
    match kind {
        "min" => numbers.iter().copied().fold(f64::INFINITY, f64::min),
        "max" => numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        "first" => numbers[0],
        "last" => numbers[numbers.len() - 1],
        "sum" => numbers.iter().sum(),
        _ => numbers.iter().sum::<f64>() / numbers.len() as f64,
    }
}

fn evaluate(expression: &Value, c: &Context<'_>) -> Value {
    if expression.as_object().is_none() {
        return expression.clone();
    }
    if let Some(key) = values::text(expression, "state") {
        return c
            .state
            .get(key)
            .filter(|v| !v.is_null())
            .or_else(|| expression.get("fallback").filter(|v| !v.is_null()))
            .cloned()
            .unwrap_or(Value::Number(0.));
    }
    if let Some(reference) = values::text(expression, "ref") {
        return match reference {
            "current" => c.current.clone(),
            "index" => Value::Number(c.index as f64),
            "count" => Value::Number(c.targets.len() as f64),
            "fps" => Value::Number(c.fps),
            "playhead" => Value::Number(c.time),
            _ => Value::Number(c.duration),
        };
    }
    if let Some(kind) = values::text(expression, "aggregate") {
        let path = values::text(expression, "path").unwrap_or_default();
        return Value::Number(aggregate(kind, path, c.targets));
    }
    let args: Vec<Value> = expression
        .get("args")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|arg| evaluate(arg, c))
        .collect();
    let at = |i: usize| args.get(i).map_or(f64::NAN, number);
    let numbers = || args.iter().map(number);
    let result = match values::text(expression, "op").unwrap_or_default() {
        "add" => numbers().sum(),
        "subtract" => at(0) - args.get(1).filter(|v| truthy(v)).map_or(0., number),
        "multiply" => numbers().product(),
        "divide" => {
            if at(1) == 0. {
                at(0)
            } else {
                at(0) / at(1)
            }
        }
        "min" => numbers().fold(f64::INFINITY, f64::min),
        "max" => numbers().fold(f64::NEG_INFINITY, f64::max),
        "clamp" => values::clamp(at(0), at(1), at(2)),
        "round" => values::js_round(at(0)),
        "floor" => at(0).floor(),
        "ceil" => at(0).ceil(),
        "abs" => at(0).abs(),
        "negate" => -at(0),
        "frames" => at(0) / c.fps.max(1.),
        "equal" => {
            return Value::Bool(match (args.first(), args.get(1)) {
                (Some(Value::Number(a)), Some(Value::Number(b))) => a == b,
                (Some(a), Some(b)) => a.identical(b),
                _ => false,
            });
        }
        "if" => {
            return args
                .get(if args.first().is_some_and(truthy) {
                    1
                } else {
                    2
                })
                .cloned()
                .unwrap_or_default();
        }
        _ => 0.,
    };
    Value::Number(result)
}

/// `orderedTargets`.
fn ordered(targets: &mut [Target], name: &str, selection: &[String], seed: &str) {
    let id = |t: &Target| values::text(&t.layer, "id").unwrap_or_default().to_owned();
    let picked = |t: &Target, missing: usize| {
        selection
            .iter()
            .position(|s| *s == id(t))
            .unwrap_or(missing)
    };
    let from = |t: &Target| values::num(&t.layer, "from").unwrap_or(0.);
    let label = |t: &Target| {
        values::text(&t.layer, "name")
            .unwrap_or_default()
            .to_owned()
    };
    match name {
        "reverseStack" => targets.reverse(),
        "selection" => targets.sort_by_key(|t| picked(t, usize::MAX)),
        "reverseSelection" => {
            targets.sort_by_key(|t| std::cmp::Reverse(picked(t, 0)));
        }
        "start" => targets.sort_by(|a, b| from(a).total_cmp(&from(b))),
        "reverseStart" => targets.sort_by(|a, b| from(b).total_cmp(&from(a))),
        "name" => targets.sort_by(|a, b| {
            let (a, b) = (label(a), label(b));
            a.to_lowercase()
                .cmp(&b.to_lowercase())
                .then_with(|| b.cmp(&a))
        }),
        "random" => targets.sort_by_key(|t| stable_hash(&format!("{seed}:{}", id(t)))),
        _ => {}
    }
}

/// `getPath`: `None` when the layer has no such source.
fn read_path(
    cx: &mut Cx<'_>,
    layer: &Value,
    path: &str,
    time: f64,
) -> Result<Option<Value>, CommandError> {
    if let Some(key) = path.strip_prefix("layer.") {
        let key = match key {
            "duration" => "dur",
            "visible" => "on",
            "locked" => "lock",
            "motionBlur" => "mblur",
            other => other,
        };
        return Ok(layer.get(key).cloned());
    }
    if let Some(key) = path.strip_prefix("content.") {
        return Ok(layer.get("d").and_then(|d| d.get(key)).cloned());
    }
    let key = path.strip_prefix("properties.").unwrap_or(path);
    let id = values::text(layer, "id").unwrap_or_default();
    let loc = cx.find(id)?;
    if super::channels::find(layer, key).is_none() {
        return Ok(None);
    }
    let index = loc.index;
    let value = cx.kernel(loc.comp_id.as_deref(), |edit| {
        edit.read(|s| s.channel(&s.layers()[index], key, time))
    })?;
    Ok(value.to_json())
}

/// `normalizedValue`.
fn normalized(path: &str, value: Value, fps: f64) -> Value {
    let frame = |v: &Value| values::snap(if number(v).is_nan() { 0. } else { number(v) }, fps);
    match path {
        "layer.from" => Value::Number(frame(&value).max(0.)),
        "layer.duration" => Value::Number(frame(&value).max(1. / fps.max(1.))),
        _ => value,
    }
}

fn set_layer_field(patch: &mut LayerPatch, key: &str, value: &Value, time: f64) {
    let text = || value.as_str().map(str::to_owned);
    let flag = value.as_bool();
    match key {
        "name" => patch.name = text(),
        "from" => patch.from = value.as_f64(),
        "duration" => patch.duration = value.as_f64(),
        "visible" => patch.visible = flag,
        "locked" => patch.locked = flag,
        "shy" => patch.shy = flag,
        "blend" => patch.blend = text(),
        "motionBlur" => patch.motion_blur = flag,
        "parent" => {
            patch.parent = Some(ParentChange {
                parent: text(),
                time,
            })
        }
        "color" => patch.color = text(),
        "collapsed" => patch.collapsed = flag,
        _ => {}
    }
}

/// `compile`, then every primitive on this scratch tree.
pub(super) fn run(
    cx: &mut Cx<'_>,
    raw: &Value,
    state: &Value,
    selection: &[String],
    time: f64,
) -> Result<(), CommandError> {
    let invalid = |message: &str| CommandError::Invalid(message.into());
    let time = values::finite(time, "time")?;
    let transform =
        sanitize(raw).ok_or_else(|| invalid("The generated transformation is invalid"))?;
    let visible = |l: &Value| {
        l.get("on").is_some_and(|on| match on.get("v") {
            Some(v) => v.is_truthy(),
            None => on.is_truthy(),
        })
    };
    let mut targets = Vec::new();
    for layer in cx.layers(None) {
        let id = values::text(&layer, "id").unwrap_or_default();
        let included = match transform.scope.as_str() {
            "selection" => selection.iter().any(|s| s == id),
            "visible" => visible(&layer),
            _ => true,
        } && (transform.types.is_empty()
            || values::text(&layer, "type")
                .is_some_and(|t| transform.types.iter().any(|x| x == t)))
            && !values::flag(&layer, "lock");
        if !included {
            continue;
        }
        let mut before = HashMap::new();
        for path in transform.edits.iter().map(|e| e.path.as_str()).chain(
            transform
                .edits
                .iter()
                .flat_map(|e| aggregate_paths(&e.value)),
        ) {
            if !before.contains_key(path)
                && let Some(value) = read_path(cx, &layer, path, time)?
            {
                before.insert(path.to_owned(), value);
            }
        }
        targets.push(Target { layer, before });
    }
    if targets.is_empty() {
        return Err(invalid(if transform.scope == "selection" {
            "Select at least one unlocked layer"
        } else {
            "No unlocked layers match this tool"
        }));
    }
    let order = match &transform.order {
        Value::String(name) => name.to_string_lossy(),
        object => {
            let key = values::text(object, "state").unwrap_or_default();
            state
                .get(key)
                .and_then(Value::as_str)
                .filter(|o| ORDERS.contains(o))
                .or_else(|| values::text(object, "fallback"))
                .unwrap_or("stack")
                .to_owned()
        }
    };
    ordered(&mut targets, &order, selection, &transform.seed);
    let comp = cx.comp_node(None)?.to_value();
    let fps = values::num(&comp, "fps").unwrap_or(30.);
    let duration = cx.duration(None);
    let mut commands: Vec<Command> = Vec::new();
    for (index, target) in targets.iter().enumerate() {
        let id = values::text(&target.layer, "id")
            .unwrap_or_default()
            .to_owned();
        let mut layer_patch = LayerPatch {
            time,
            ..Default::default()
        };
        let mut touched_layer = false;
        let mut content = Map::new();
        let mut properties = Vec::new();
        for edit in &transform.edits {
            let Some(before) = target.before.get(&edit.path) else {
                continue;
            };
            let context = Context {
                current: before.clone(),
                index,
                targets: &targets,
                state,
                fps,
                time,
                duration,
            };
            let value = normalized(&edit.path, evaluate(&edit.value, &context), fps);
            if value.identical(before) {
                continue;
            }
            if let Some(key) = edit.path.strip_prefix("layer.") {
                set_layer_field(&mut layer_patch, key, &value, time);
                touched_layer = true;
            } else if let Some(key) = edit.path.strip_prefix("content.") {
                content.insert(key, value);
            } else {
                let key = edit.path.strip_prefix("properties.").unwrap_or(&edit.path);
                properties.push(PropertyCommand::Set(SetProperty::new(
                    id.clone(),
                    key,
                    value,
                    time,
                )));
            }
        }
        if touched_layer {
            commands.push(
                LayerCommand::Set {
                    layer_id: id.clone(),
                    patch: layer_patch,
                }
                .into(),
            );
        }
        if !content.is_empty() {
            commands.push(
                ContentCommand::Set {
                    layer_id: id.clone(),
                    patch: content,
                    time,
                }
                .into(),
            );
        }
        commands.extend(properties.into_iter().map(Command::Property));
    }
    if commands.is_empty() {
        return Err(invalid(
            "This transformation would not change the selected source",
        ));
    }
    for command in commands {
        match command {
            Command::Layer(c) => c.plan(cx)?,
            Command::Content(c) => c.plan(cx)?,
            Command::Property(c) => c.plan(cx)?,
            _ => unreachable!("transforms compile to layer, content and property edits"),
        }
    }
    Ok(())
}

/// Paths an expression aggregates over, read for every target before editing.
fn aggregate_paths(expression: &Value) -> Vec<&str> {
    let mut out: Vec<&str> = values::text(expression, "aggregate")
        .and(values::text(expression, "path"))
        .into_iter()
        .collect();
    for arg in expression
        .get("args")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        out.extend(aggregate_paths(arg));
    }
    out
}
