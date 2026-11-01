//! Compositions and project-level settings.
//!
//! Ports `setComposition` and `setSceneParameter` of
//! `legacy/core/editing.ts`, the work-area commands of the timeline (B, N,
//! the ruler menu), `PM.Comps` (`legacy/core/compositions.ts`: create,
//! duplicate, delete, pre-compose) and the composition settings sheet
//! (`legacy/ui/project-settings.ts`).

use std::collections::HashSet;

use pm_model::{Map, Value};

use super::{Cx, layers, track, values};
use crate::error::CommandError;

/// The editable fields of `set_composition` (`None` leaves a field alone).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompPatch {
    /// Trimmed; blank keeps the old name.
    pub name: Option<String>,
    /// `w`, rounded, at least 16.
    pub width: Option<f64>,
    /// `h`, rounded, at least 16.
    pub height: Option<f64>,
    /// Rounded, 1–240.
    pub fps: Option<f64>,
    /// At least 0.1 s; resets the work area to the whole composition unless
    /// `work_area` is also given.
    pub duration: Option<f64>,
    /// `bg` (`#RRGGBB`); also replaces `backgroundFill` with a solid fill.
    pub background: Option<String>,
    /// `backgroundFill` (`PM.normalizeFill`); `bg` follows its first stop.
    pub background_fill: Option<Value>,
    /// 0–2.
    pub shutter: Option<f64>,
    /// `[start, end]`, start ≥ 0, end ≤ duration, end after start.
    pub work_area: Option<(f64, f64)>,
    /// 8, 16 or 32.
    pub color_depth: Option<u32>,
    /// Master gain, 0–4.
    pub audio_gain: Option<f64>,
}

/// How Pre-compose treats the selection (`PM.Comps.precompose`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrecomposeMode {
    /// Move all attributes into the new composition.
    MoveAll,
    /// Leave all attributes (one layer: its transform and effects stay on
    /// the pre-comp layer).
    LeaveAttributes,
}

/// A new composition's settings (defaults from the open composition).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NewComp {
    pub name: Option<String>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub fps: Option<f64>,
    pub duration: Option<f64>,
    pub background: Option<String>,
}

/// Composition commands. `comp: None` is the open composition.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CompCommand {
    /// `set_composition` (`setComposition`) on the open composition (`name`
    /// and `colorDepth` are the project's, as in the old command). Label
    /// "Composition Settings".
    Set { patch: Box<CompPatch> },
    /// The Composition Settings sheet and the Project panel's rename
    /// (`PM.Comps.setSettings`/`writeSettings`) on any composition: a
    /// unique name (pre-comp layers still showing the old name follow it),
    /// size (users' `d.w`/`d.h` follow), frame rate 1–240, duration (the
    /// work area is kept inside), background `#RRGGBB`. Label "Composition
    /// Settings".
    Settings { comp_id: String, settings: NewComp },
    /// B / N / the ruler menu: one or both work-area edges (composition
    /// seconds), clamped to the composition and kept at least one frame
    /// apart. Label "Set Work Area".
    SetWorkArea {
        comp: Option<String>,
        start: Option<f64>,
        end: Option<f64>,
    },
    /// Reset Work Area to the entire composition. Label "Reset Work Area".
    ResetWorkArea { comp: Option<String> },
    /// `set_scene_parameter` (`setSceneParameter`): a project parameter
    /// (`params[name]`) shown by generated controls. Label "Set Parameter".
    SetSceneParameter {
        name: String,
        label: Option<String>,
        control: Option<String>,
        value: Value,
        min: Option<Value>,
        max: Option<Value>,
        options: Option<Value>,
    },
    /// New Composition (Cmd+N, `blankComp`): an empty composition `comp_id`
    /// with the open one's settings as defaults and a unique name (`Comp
    /// n`). It does not open it (the shell switches compositions). Label
    /// "New Composition".
    Create { comp_id: String, settings: NewComp },
    /// Duplicate Composition (`PM.Comps.duplicate`): a copy under `new_id`
    /// named `<name> 2` (unique), layers cloned with fresh layer and key ids
    /// and their links remapped. Label "Duplicate Composition".
    Duplicate { comp_id: String, new_id: String },
    /// Delete Composition (`PM.Comps.remove`): pre-comp layers that use it go
    /// with it (children parented to them are unparented), as in After
    /// Effects; the last composition cannot be deleted; deleting the open
    /// one opens `fallback` (default: the first other composition). Label
    /// "Delete Composition".
    Delete {
        comp_id: String,
        fallback: Option<String>,
    },
    /// Pre-compose (Cmd+Shift+C): moves the layers into a new composition
    /// `new_comp_id` and replaces them with one pre-comp layer
    /// `precomp_layer_id`, keeping every rendered pixel. Not ported (needs
    /// `PM.Comps.precompose` and the `comp_bounds` kernel).
    Precompose {
        layer_ids: Vec<String>,
        new_comp_id: String,
        precomp_layer_id: String,
        name: Option<String>,
        mode: PrecomposeMode,
        time: f64,
    },
}

impl CompCommand {
    pub fn label(&self) -> String {
        match self {
            CompCommand::Set { .. } | CompCommand::Settings { .. } => "Composition Settings",
            CompCommand::SetWorkArea { .. } => "Set Work Area",
            CompCommand::ResetWorkArea { .. } => "Reset Work Area",
            CompCommand::SetSceneParameter { .. } => "Set Parameter",
            CompCommand::Create { .. } => "New Composition",
            CompCommand::Duplicate { .. } => "Duplicate Composition",
            CompCommand::Delete { .. } => "Delete Composition",
            CompCommand::Precompose { .. } => "Pre-compose",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        match self {
            CompCommand::Set { patch } => set_composition(cx, patch),
            CompCommand::Settings { comp_id, settings } => write_settings(cx, comp_id, settings),
            CompCommand::SetWorkArea { comp, start, end } => {
                set_work_area(cx, comp.as_deref(), *start, *end)
            }
            CompCommand::ResetWorkArea { comp } => {
                let dur = cx.duration(comp.as_deref());
                let mut path = cx.comp_path(comp.as_deref());
                path.push("work".into());
                cx.set(path, Value::Array(vec![n(0.0), n(dur)]))
            }
            CompCommand::SetSceneParameter {
                name,
                label,
                control,
                value,
                min,
                max,
                options,
            } => set_scene_parameter(cx, name, label, control, value, min, max, options),
            CompCommand::Create { comp_id, settings } => create(cx, comp_id, settings),
            CompCommand::Duplicate { comp_id, new_id } => duplicate(cx, comp_id, new_id),
            CompCommand::Delete { comp_id, fallback } => delete(cx, comp_id, fallback.as_deref()),
            CompCommand::Precompose { .. } => Err(CommandError::NotImplemented("precompose")),
        }
    }
}

fn n(x: f64) -> Value {
    Value::Number(x)
}

fn hex_color(s: &str) -> bool {
    let h = s.strip_prefix('#').unwrap_or("");
    (h.len() == 6 || h.len() == 8) && h.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `PM.normalizeFill(value, fallback)`.
pub(crate) fn normalize_fill(value: &Value, fallback: &str) -> Value {
    let color = |v: Option<&Value>| match v.and_then(Value::as_str) {
        Some(c) if hex_color(c) => values::s(&c.to_uppercase()),
        _ => values::s(fallback),
    };
    let raw = if value.as_object().is_some() {
        value.clone()
    } else {
        Value::Object(Map::new())
    };
    let kind = match values::text(&raw, "type") {
        Some(t @ ("solid" | "linear" | "radial" | "none")) => t.to_owned(),
        _ => "solid".to_owned(),
    };
    let mut stops: Vec<(f64, Value)> = raw
        .get("stops")
        .and_then(Value::as_array)
        .map(|a| a.iter().take(8).cloned().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(i, stop)| {
            let id = match values::text(&stop, "id") {
                Some(id) if !id.is_empty() => id.to_owned(),
                _ => format!("stop-{}", i + 1),
            };
            let pos = stop.get("position").map(track::js_number).unwrap_or(0.0);
            let pos = values::clamp(if pos.is_nan() { 0.0 } else { pos }, 0.0, 100.0);
            let mut m = Map::new();
            m.insert("id", values::s(&id));
            m.insert("color", color(stop.get("color")));
            m.insert("position", n(pos));
            (pos, Value::Object(m))
        })
        .collect();
    stops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut stops: Vec<Value> = stops.into_iter().map(|(_, v)| v).collect();
    if stops.is_empty() {
        let c = raw.get("color").filter(|c| c.is_truthy()).cloned();
        let mut m = Map::new();
        m.insert("id", values::s("stop-1"));
        m.insert(
            "color",
            match c {
                Some(c) => color(Some(&c)),
                None => color(Some(&values::s(fallback))),
            },
        );
        m.insert("position", n(0.0));
        stops.push(Value::Object(m));
    }
    let plain = kind == "solid" || kind == "none";
    if !plain && stops.len() < 2 {
        let mut m = Map::new();
        m.insert("id", values::s("stop-2"));
        m.insert("color", stops[0].get("color").cloned().unwrap_or_default());
        m.insert("position", n(100.0));
        stops.push(Value::Object(m));
    }
    if plain {
        let mut first = stops[0].clone();
        values::obj(&mut first).insert("position", n(0.0));
        stops = vec![first];
    }
    let angle = raw.get("angle").map(track::js_number).unwrap_or(0.0);
    let mut m = Map::new();
    m.insert("type", values::s(&kind));
    m.insert(
        "angle",
        n(values::clamp(
            if angle.is_nan() { 0.0 } else { angle },
            -180.0,
            180.0,
        )),
    );
    m.insert("stops", Value::Array(stops));
    Value::Object(m)
}

/// `setComposition(command)` on the open composition.
fn set_composition(cx: &mut Cx<'_>, p: &CompPatch) -> Result<(), CommandError> {
    let comp = cx.comp_path(None);
    let at = |k: &str| {
        let mut path = comp.clone();
        path.push(k.into());
        path
    };
    if let Some(depth) = p.color_depth {
        if ![8, 16, 32].contains(&depth) {
            return Err(CommandError::Invalid(
                "Color depth must be 8, 16, or 32".into(),
            ));
        }
        cx.set(vec!["colorDepth".into()], n(f64::from(depth)))?;
    }
    if let Some(name) = &p.name {
        let t = values::js_trim(name);
        if !t.is_empty() {
            cx.set(vec!["name".into()], values::s(t))?;
        }
    }
    if let Some(w) = p.width {
        cx.set(
            at("w"),
            n(16f64.max(values::js_round(values::finite(w, "width")?))),
        )?;
    }
    if let Some(h) = p.height {
        cx.set(
            at("h"),
            n(16f64.max(values::js_round(values::finite(h, "height")?))),
        )?;
    }
    if let Some(fps) = p.fps {
        cx.set(
            at("fps"),
            n(values::clamp(
                values::js_round(values::finite(fps, "fps")?),
                1.0,
                240.0,
            )),
        )?;
    }
    if let Some(d) = p.duration {
        let dur = 0.1f64.max(values::finite(d, "duration")?);
        cx.set(at("dur"), n(dur))?;
        if p.work_area.is_none() {
            cx.set(at("work"), Value::Array(vec![n(0.0), n(dur)]))?;
        }
    }
    if let Some(bg) = &p.background {
        cx.set(at("bg"), values::s(bg))?;
        let mut solid = Map::new();
        solid.insert("type", values::s("solid"));
        solid.insert("color", values::s(bg));
        cx.set(
            at("backgroundFill"),
            normalize_fill(&Value::Object(solid), bg),
        )?;
    }
    if let Some(fill) = &p.background_fill {
        let bg = cx
            .lookup(&at("bg"))
            .and_then(crate::model::str_of)
            .unwrap_or("#000000")
            .to_owned();
        let normalized = normalize_fill(fill, &bg);
        let first = normalized
            .get("stops")
            .and_then(Value::as_array)
            .and_then(|s| s.first())
            .and_then(|s| s.get("color"))
            .cloned()
            .unwrap_or_default();
        cx.set(at("backgroundFill"), normalized)?;
        cx.set(at("bg"), first)?;
    }
    if let Some(s) = p.shutter {
        cx.set(
            at("shutter"),
            n(values::clamp(values::finite(s, "shutter")?, 0.0, 2.0)),
        )?;
    }
    if let Some(g) = p.audio_gain {
        cx.set(
            at("audioGain"),
            n(values::clamp(values::finite(g, "audio gain")?, 0.0, 4.0)),
        )?;
    }
    if let Some((start, end)) = p.work_area {
        let dur = cx.duration(None);
        let start = values::finite(start, "work area start")?.max(0.0);
        let end = dur.min(values::finite(end, "work area end")?);
        if end <= start {
            return Err(CommandError::Invalid(
                "workArea end must be after its start".into(),
            ));
        }
        cx.set(at("work"), Value::Array(vec![n(start), n(end)]))?;
    }
    Ok(())
}

/// Every composition id.
fn comp_ids(cx: &Cx<'_>) -> Vec<String> {
    match cx.root().get("comps").map(pm_model::Node::kind) {
        Some(pm_model::NodeKind::Object(map)) => map
            .keys()
            .filter_map(|k| k.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

fn comp_name(cx: &Cx<'_>, id: &str) -> String {
    cx.comp_node(Some(id))
        .ok()
        .and_then(|c| c.get("name"))
        .and_then(crate::model::str_of)
        .unwrap_or_default()
        .to_owned()
}

/// `uniqueName(base, except)`.
fn unique_name(cx: &Cx<'_>, base: &str, except: Option<&str>) -> String {
    let taken: HashSet<String> = comp_ids(cx)
        .into_iter()
        .filter(|id| Some(id.as_str()) != except)
        .map(|id| comp_name(cx, &id))
        .collect();
    if !taken.contains(base) {
        return base.to_owned();
    }
    let stem = strip_number(base);
    (2..)
        .map(|n| format!("{stem} {n}"))
        .find(|c| !taken.contains(c))
        .expect("unbounded")
}

/// `/\s+\d+$/` removed.
fn strip_number(s: &str) -> &str {
    let t = s.trim_end_matches(|c: char| c.is_ascii_digit());
    if t.len() < s.len() && t.ends_with(char::is_whitespace) {
        t.trim_end()
    } else {
        s
    }
}

/// Pre-comp layers that show `comp_id`, as (composition, layer id).
fn users(cx: &Cx<'_>, comp_id: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for c in comp_ids(cx) {
        for l in cx.layers(Some(&c)) {
            if values::text(&l, "type") == Some("precomp")
                && l.get("d")
                    .and_then(|d| d.get("comp"))
                    .and_then(Value::as_str)
                    == Some(comp_id)
            {
                out.push((
                    c.clone(),
                    values::text(&l, "id").unwrap_or_default().to_owned(),
                ));
            }
        }
    }
    out
}

/// `writeSettings(id, patch)`.
fn write_settings(cx: &mut Cx<'_>, comp_id: &str, s: &NewComp) -> Result<(), CommandError> {
    let node = cx.comp_node(Some(comp_id))?.to_value();
    let mut path = cx.comp_path(Some(comp_id));
    let at = |path: &mut Vec<pm_model::PathPart>, k: &str| {
        let mut p = path.clone();
        p.push(k.into());
        p
    };
    if let Some(name) = s
        .name
        .as_deref()
        .map(values::js_trim)
        .filter(|n| !n.is_empty())
    {
        let previous = values::text(&node, "name").unwrap_or_default().to_owned();
        let unique = unique_name(cx, name, Some(comp_id));
        cx.set(at(&mut path, "name"), values::s(&unique))?;
        for (c, l) in users(cx, comp_id) {
            let loc = cx.find(&l)?;
            if loc.comp_id.as_deref() == Some(c.as_str()) {
                let layer = cx.layer_node(&loc)?.to_value();
                if values::text(&layer, "name") == Some(previous.as_str()) {
                    let mut p = loc.path();
                    p.push("name".into());
                    cx.set(p, values::s(&unique))?;
                }
            }
        }
    }
    if let Some(w) = s.width {
        cx.set(at(&mut path, "w"), n(16f64.max(values::js_round(w))))?;
    }
    if let Some(h) = s.height {
        cx.set(at(&mut path, "h"), n(16f64.max(values::js_round(h))))?;
    }
    if let Some(fps) = s.fps {
        cx.set(at(&mut path, "fps"), n(values::clamp(fps, 1.0, 240.0)))?;
    }
    if let Some(d) = s.duration {
        let fps = cx.fps(Some(comp_id));
        let dur = (1.0 / fps).max(d);
        cx.set(at(&mut path, "dur"), n(dur))?;
        let work = node.get("work").and_then(Value::as_array).cloned();
        let (w0, w1) = match work.as_deref() {
            Some([a, b, ..]) => (track::js_number(a), track::js_number(b)),
            _ => (0.0, dur),
        };
        let (mut a, mut b) = (w0.min(dur), w1.max(0.0).min(dur));
        if b <= a {
            (a, b) = (0.0, dur);
        }
        cx.set(at(&mut path, "work"), Value::Array(vec![n(a), n(b)]))?;
    }
    if let Some(bg) = s.background.as_deref().filter(|b| {
        let h = b.strip_prefix('#').unwrap_or("");
        h.len() == 6 && h.bytes().all(|c| c.is_ascii_hexdigit())
    }) {
        let bg = bg.to_uppercase();
        cx.set(at(&mut path, "bg"), values::s(&bg))?;
        let mut solid = Map::new();
        solid.insert("type", values::s("solid"));
        solid.insert("color", values::s(&bg));
        cx.set(
            at(&mut path, "backgroundFill"),
            normalize_fill(&Value::Object(solid), &bg),
        )?;
    }
    if s.width.is_some() || s.height.is_some() {
        let comp = cx.comp_node(Some(comp_id))?.to_value();
        for (_, l) in users(cx, comp_id) {
            let loc = cx.find(&l)?;
            for k in ["w", "h"] {
                let mut p = loc.path();
                p.extend(["d".into(), k.into()]);
                cx.set(p, comp.get(k).cloned().unwrap_or_default())?;
            }
        }
    }
    Ok(())
}

/// B / N / the ruler menu.
fn set_work_area(
    cx: &mut Cx<'_>,
    comp: Option<&str>,
    start: Option<f64>,
    end: Option<f64>,
) -> Result<(), CommandError> {
    let dur = cx.duration(comp);
    let frame = 1.0 / cx.fps(comp);
    let mut path = cx.comp_path(comp);
    path.push("work".into());
    let current = cx.value_at(&path);
    let (w0, w1) = match current
        .as_ref()
        .and_then(Value::as_array)
        .map(Vec::as_slice)
    {
        Some([a, b, ..]) => (track::js_number(a), track::js_number(b)),
        _ => (0.0, dur),
    };
    let clamp = |t: f64| values::clamp(t, 0.0, dur);
    let (a, b) = match (start, end) {
        (Some(s), Some(e)) => {
            let (s, e) = (
                clamp(values::finite(s, "work area start")?),
                clamp(values::finite(e, "work area end")?),
            );
            if e - s < frame - 1e-9 {
                return Err(CommandError::Invalid(
                    "workArea end must be after its start".into(),
                ));
            }
            (s, e)
        }
        (Some(s), None) => {
            let s = clamp(values::finite(s, "work area start")?).min((w1 - frame).max(0.0));
            (s, w1.max(s + frame).min(dur))
        }
        (None, Some(e)) => {
            let e = clamp(values::finite(e, "work area end")?).max((w0 + frame).min(dur));
            (w0.min(e - frame).max(0.0), e)
        }
        (None, None) => return Ok(()),
    };
    cx.set(path, Value::Array(vec![n(a), n(b)]))
}

#[allow(clippy::too_many_arguments)]
fn set_scene_parameter(
    cx: &mut Cx<'_>,
    name: &str,
    label: &Option<String>,
    control: &Option<String>,
    value: &Value,
    min: &Option<Value>,
    max: &Option<Value>,
    options: &Option<Value>,
) -> Result<(), CommandError> {
    let name = values::js_trim(name);
    if name.is_empty() {
        return Err(CommandError::Invalid(
            "Scene parameter name is required".into(),
        ));
    }
    let path = vec!["params".into(), name.into()];
    let mut current = cx
        .value_at(&path)
        .filter(|v| v.as_object().is_some())
        .unwrap_or_else(|| {
            let mut m = Map::new();
            m.insert("name", values::s(name));
            m.insert(
                "label",
                values::s(label.as_deref().filter(|l| !l.is_empty()).unwrap_or(name)),
            );
            m.insert(
                "control",
                values::s(
                    control
                        .as_deref()
                        .filter(|c| !c.is_empty())
                        .unwrap_or("num"),
                ),
            );
            Value::Object(m)
        });
    let pick = |given: &Option<String>, existing: Option<&str>, fallback: &str| -> Value {
        values::s(
            given
                .as_deref()
                .filter(|g| !g.is_empty())
                .or(existing.filter(|e| !e.is_empty()))
                .unwrap_or(fallback),
        )
    };
    let label_v = pick(label, values::text(&current, "label"), name);
    let control_v = pick(control, values::text(&current, "control"), "num");
    let m = values::obj(&mut current);
    m.insert("label", label_v);
    m.insert("control", control_v);
    m.insert("value", value.clone());
    if let Some(v) = min.as_ref().filter(|v| !v.is_null()) {
        m.insert("min", v.clone());
    }
    if let Some(v) = max.as_ref().filter(|v| !v.is_null()) {
        m.insert("max", v.clone());
    }
    match options.as_ref().filter(|v| !v.is_null()) {
        Some(o) => {
            m.insert("options", o.clone());
        }
        None => {
            if m.get("options").is_none() {
                m.insert("options", Value::Array(Vec::new()));
            }
        }
    }
    if cx.lookup(&["params".into()]).is_none() {
        cx.set(vec!["params".into()], Value::Object(Map::new()))?;
    }
    cx.replace(path, current)
}

/// `blankComp(settings)` + `create`.
fn create(cx: &mut Cx<'_>, comp_id: &str, s: &NewComp) -> Result<(), CommandError> {
    if comp_ids(cx).iter().any(|c| c == comp_id) {
        return Err(CommandError::Invalid(format!(
            "Composition id already exists: {comp_id}"
        )));
    }
    let base = cx.comp_node(None).map(|c| c.to_value()).unwrap_or_default();
    let w = values::js_round(s.width.or(values::num(&base, "w")).unwrap_or(1920.0));
    let h = values::js_round(s.height.or(values::num(&base, "h")).unwrap_or(1080.0));
    let fps = s.fps.or(values::num(&base, "fps")).unwrap_or(30.0);
    let dur = s.duration.or(values::num(&base, "dur")).unwrap_or(10.0);
    let bg = s.background.clone().unwrap_or_else(|| "#000000".into());
    let name = match s
        .name
        .as_deref()
        .map(values::js_trim)
        .filter(|n| !n.is_empty())
    {
        Some(n) => n.to_owned(),
        None => {
            let taken: HashSet<String> = comp_ids(cx).iter().map(|id| comp_name(cx, id)).collect();
            (1..)
                .map(|n| format!("Comp {n}"))
                .find(|c| !taken.contains(c))
                .expect("unbounded")
        }
    };
    let name = unique_name(cx, &name, None);
    let mut solid = Map::new();
    solid.insert("type", values::s("solid"));
    solid.insert("color", values::s(&bg));
    let mut m = Map::new();
    m.insert("id", values::s(comp_id));
    m.insert("name", values::s(&name));
    m.insert("w", n(w));
    m.insert("h", n(h));
    m.insert("fps", n(fps));
    m.insert("dur", n(dur));
    m.insert("bg", values::s(&bg));
    m.insert("backgroundFill", normalize_fill(&Value::Object(solid), &bg));
    m.insert("layers", Value::Array(Vec::new()));
    m.insert("comps", Value::Object(Map::new()));
    m.insert("markers", Value::Array(Vec::new()));
    m.insert("work", Value::Array(vec![n(0.0), n(dur)]));
    m.insert("shutter", base.get("shutter").cloned().unwrap_or(n(0.5)));
    cx.set(vec!["comps".into(), comp_id.into()], Value::Object(m))
}

/// `PM.Comps.duplicate(id)`.
fn duplicate(cx: &mut Cx<'_>, comp_id: &str, new_id: &str) -> Result<(), CommandError> {
    let mut copy = cx.comp_node(Some(comp_id))?.to_value();
    if comp_ids(cx).iter().any(|c| c == new_id) {
        return Err(CommandError::Invalid(format!(
            "Composition id already exists: {new_id}"
        )));
    }
    let name = values::text(&copy, "name").unwrap_or_default().to_owned();
    let name = unique_name(cx, &format!("{} 2", strip_number(&name)), None);
    let layers = cx.layers(Some(comp_id));
    let ids: Vec<(String, String)> = layers
        .iter()
        .map(|l| {
            (
                values::text(l, "id").unwrap_or_default().to_owned(),
                new_id_for("L"),
            )
        })
        .collect();
    let map = |id: Option<&str>| {
        id.and_then(|id| ids.iter().find(|(s, _)| s == id).map(|(_, n)| n.clone()))
    };
    let copies: Vec<Value> = layers
        .iter()
        .zip(&ids)
        .map(|(l, (_, nid))| {
            let mut c = layers::clone_layer(l, nid, &layers);
            let m = values::obj(&mut c);
            m.insert("name", l.get("name").cloned().unwrap_or_default());
            for f in ["parent", "group", "matteSource"] {
                if m.get(f).is_some_and(Value::is_truthy) {
                    let v = map(m.get(f).and_then(Value::as_str))
                        .map(|s| values::s(&s))
                        .unwrap_or(Value::Null);
                    m.insert(f, v);
                }
            }
            if let Some(fx) = m.get_mut("fx").and_then(Value::as_array_mut) {
                for e in fx {
                    if let Some(inputs) = e
                        .as_object_mut()
                        .and_then(|e| e.get_mut("inputs"))
                        .and_then(Value::as_object_mut)
                    {
                        for (_, r) in inputs.iter_mut() {
                            if let Some(t) = map(values::text(r, "layerId")) {
                                values::obj(r).insert("layerId", values::s(&t));
                            }
                        }
                    }
                }
            }
            c
        })
        .collect();
    let m = values::obj(&mut copy);
    m.insert("id", values::s(new_id));
    m.insert("name", values::s(&name));
    m.insert("layers", Value::Array(copies));
    m.insert("comps", Value::Object(Map::new()));
    cx.set(vec!["comps".into(), new_id.into()], copy)
}

fn new_id_for(prefix: &str) -> String {
    super::new_id(prefix)
}

/// `PM.Comps.remove(id)`.
fn delete(cx: &mut Cx<'_>, comp_id: &str, fallback: Option<&str>) -> Result<(), CommandError> {
    cx.comp_node(Some(comp_id))?;
    let all = comp_ids(cx);
    if all.len() <= 1 {
        return Err(CommandError::Invalid(
            "The last composition cannot be deleted".into(),
        ));
    }
    if cx.comp_id(None).as_deref() == Some(comp_id) {
        let next = fallback
            .filter(|f| *f != comp_id && all.iter().any(|c| c == f))
            .map(str::to_owned)
            .or_else(|| all.iter().find(|c| *c != comp_id).cloned())
            .expect("another composition");
        cx.set(vec!["compId".into()], values::s(&next))?;
    }
    for c in all.iter().filter(|c| *c != comp_id) {
        let layers = cx.layers(Some(c));
        let doomed: HashSet<String> = layers
            .iter()
            .filter(|l| {
                values::text(l, "type") == Some("precomp")
                    && l.get("d")
                        .and_then(|d| d.get("comp"))
                        .and_then(Value::as_str)
                        == Some(comp_id)
            })
            .filter_map(|l| values::text(l, "id").map(str::to_owned))
            .collect();
        if doomed.is_empty() {
            continue;
        }
        let next: Vec<Value> = layers
            .into_iter()
            .filter(|l| !values::text(l, "id").is_some_and(|i| doomed.contains(i)))
            .map(|mut l| {
                if values::text(&l, "parent").is_some_and(|p| doomed.contains(p)) {
                    values::obj(&mut l).insert("parent", Value::Null);
                }
                l
            })
            .collect();
        layers::write_layers(cx, Some(c), next)?;
    }
    cx.remove(vec!["comps".into(), comp_id.into()])
}
