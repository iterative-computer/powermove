//! Caption layers.
//!
//! Ports `addCaptions`, `editCaptions` and `setCaptionsContent` of
//! `captions/commands.ts` (cue lists are `{ id, start, end, text, words? }`
//! stored in layer-local seconds; command cue times are composition seconds.
//! Styles fill in from their existing preset).

use pm_model::Value;

use super::Cx;
use crate::error::CommandError;

/// A cue list or text to import (`cues`, or `text` in `format`).
#[derive(Clone, Debug, PartialEq)]
pub enum CueSource {
    /// Cue objects `{ id?, start, end, text, words? }`.
    Cues(Value),
    /// SubRip, WebVTT or plain text, shifted by `offset` seconds.
    Text {
        text: String,
        format: CaptionFormat,
        offset: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionFormat {
    Srt,
    Vtt,
    Text,
}

/// `add_captions`.
#[derive(Clone, Debug, PartialEq)]
pub struct AddCaptions {
    pub id: String,
    pub name: Option<String>,
    pub source: CueSource,
    /// Partial style over the preset (`style.preset`, default `classic`).
    pub style: Option<Value>,
    pub language: Option<String>,
    pub from: Option<f64>,
    pub duration: Option<f64>,
    pub index: Option<usize>,
    pub comp: Option<String>,
}

/// `edit_captions` operations (`op`).
#[derive(Clone, Debug, PartialEq)]
pub enum CaptionEdit {
    /// Replace every cue.
    Replace(CueSource),
    /// Insert cues (sorted by start).
    Insert(Value),
    /// Patch cues by id (text, start, end, words): the cue drag and text edit.
    Update(Value),
    /// Delete cues by id.
    Delete(Vec<String>),
    /// Split a cue at composition-time `at` seconds.
    Split { id: String, at: f64 },
    /// Same split with a host-reserved identity, shared by planning and apply.
    SplitWithId {
        id: String,
        at: f64,
        tail_id: String,
    },
    /// Merge cues into the first.
    Merge(Vec<String>),
    /// Shift cues by `by` seconds.
    Move { ids: Vec<String>, by: f64 },
    /// No explicit target list: shift every normalized cue, as main does.
    MoveAll { by: f64 },
    /// Import text, replacing (default) or adding.
    Import { source: CueSource, replace: bool },
    /// Patch the style (`preset` resets to that preset first) and language.
    Style {
        style: Value,
        language: Option<String>,
    },
}

/// Caption commands.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CaptionCommand {
    /// `add_captions` (`addCaptions`). Label "Add Captions".
    Add(Box<AddCaptions>),
    /// `edit_captions` (`editCaptions`); the layer's duration grows to fit
    /// its cues. Label "Edit Captions".
    Edit { layer_id: String, edit: CaptionEdit },
    EditWithLanguage {
        layer_id: String,
        edit: CaptionEdit,
        language: Option<String>,
    },
}

impl CaptionCommand {
    pub fn label(&self) -> String {
        match self {
            CaptionCommand::Add(_) => "Add Captions",
            CaptionCommand::Edit { .. } | CaptionCommand::EditWithLanguage { .. } => {
                "Edit Captions"
            }
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        match self {
            Self::Add(command) => add(cx, command),
            Self::Edit { layer_id, edit } => edit_layer(cx, layer_id, edit),
            Self::EditWithLanguage {
                layer_id,
                edit,
                language,
            } => {
                edit_layer(cx, layer_id, edit)?;
                let (loc, mut layer) = cx.layer_mut_checked(layer_id)?;
                let mut content = content(&layer)?;
                content.language = language
                    .as_ref()
                    .filter(|s| pm_captions::valid_language(s))
                    .cloned();
                write_content(&mut layer, &content)?;
                cx.put_layer(&loc, layer)
            }
        }
    }
}

use super::{layers, values};
use pm_model::Map;
use serde_json::{Value as Json, json};

fn json(value: &Value) -> Result<Json, CommandError> {
    serde_json::from_str(&pm_model::json::to_canonical_string(value)).map_err(|_| {
        CommandError::Invalid("Caption input cannot be represented by the caption service".into())
    })
}
fn model(value: &impl serde::Serialize) -> Result<Value, CommandError> {
    let text = serde_json::to_string(value).map_err(|e| CommandError::Invalid(e.to_string()))?;
    pm_model::json::parse(&text).map_err(|e| CommandError::Invalid(e.to_string()))
}
fn cue_id() -> String {
    super::new_id("cue")
}
fn height(cx: &Cx<'_>, comp: Option<&str>) -> f64 {
    cx.comp_node(comp)
        .ok()
        .and_then(|c| c.get("h"))
        .and_then(crate::model::number_of)
        .filter(|h| *h > 0.)
        .unwrap_or(1080.)
}
fn shifted(raw: &Value, shift: f64) -> Result<Json, CommandError> {
    let mut raw = json(raw)?;
    for cue in raw.as_array_mut().into_iter().flatten() {
        for key in ["start", "end"] {
            if let Some(value) = cue.get_mut(key) {
                *value = Json::from(
                    values::finite(super::track::js_number(&model(value)?), key)? - shift,
                );
            }
        }
        for word in cue
            .get_mut("words")
            .and_then(Json::as_array_mut)
            .into_iter()
            .flatten()
        {
            for key in ["start", "end"] {
                if let Some(value) = word.get_mut(key) {
                    *value = Json::from(
                        values::finite(super::track::js_number(&model(value)?), key)? - shift,
                    );
                }
            }
        }
    }
    Ok(raw)
}
fn source(source: &CueSource, shift: f64) -> Result<Json, CommandError> {
    match source {
        CueSource::Cues(cues) => shifted(cues, shift),
        CueSource::Text {
            text,
            format,
            offset,
        } => {
            values::finite(*offset, "Caption offset")?;
            let format = match format {
                CaptionFormat::Srt => Some(pm_captions::CaptionFormat::Srt),
                CaptionFormat::Vtt => Some(pm_captions::CaptionFormat::Vtt),
                CaptionFormat::Text => None,
            };
            let parsed = pm_captions::parse_captions(text, format, "");
            if parsed.cues.is_empty() {
                return Err(CommandError::Invalid(
                    "No captions could be read from that text. Use SRT or WebVTT.".into(),
                ));
            }
            shifted(&model(&parsed.cues)?, shift - offset)
        }
    }
}
/// Resolve an imported text source once on the owning service worker, before
/// a host plans and applies the same command. Returned cue times are composition
/// seconds; callers reserve identities before admitting the typed command.
pub fn materialize_source(input: &CueSource) -> Result<Value, CommandError> {
    model(&source(input, 0.)?)
}

fn content(layer: &Value) -> Result<pm_captions::CaptionsContent, CommandError> {
    // Unknown content remains in the authored model; only service-owned fields
    // are projected through the caption service's Unicode JSON representation.
    let d = layer.get("d");
    let mut known = Map::new();
    for key in ["cues", "style", "language"] {
        if let Some(value) = d.and_then(|d| d.get(key)) {
            known.insert(key, value.clone());
        }
    }
    Ok(pm_captions::normalize_captions_content(
        &json(&Value::Object(known))?,
        &mut cue_id,
    ))
}
fn write_content(
    layer: &mut Value,
    content: &pm_captions::CaptionsContent,
) -> Result<(), CommandError> {
    let mut next = layer
        .get("d")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let normalized = model(content)?;
    for key in ["cues", "style", "language"] {
        if let Some(value) = normalized.get(key) {
            next.insert(key, value.clone());
        } else {
            next.remove(key);
        }
    }
    values::obj(layer).insert("d", Value::Object(next));
    Ok(())
}
fn stack_placement(
    cx: &Cx<'_>,
    comp: Option<&str>,
    layer: &Value,
    content: &mut pm_captions::CaptionsContent,
) {
    let from = values::num(layer, "from").unwrap_or(0.);
    let end = from + values::num(layer, "dur").unwrap_or(0.);
    let start = content
        .cues
        .first()
        .map_or(from, |c| from.max(from + c.start));
    let end = content.cues.last().map_or(end, |c| end.min(from + c.end));
    let mut taken = std::collections::HashSet::new();
    for other in cx.layers(comp) {
        if values::text(&other, "id") == values::text(layer, "id")
            || values::text(&other, "type") != Some("captions")
            || other.get("on").and_then(Value::as_bool) == Some(false)
        {
            continue;
        }
        let d = other.get("d");
        let Some(cues) = d
            .and_then(|d| d.get("cues"))
            .and_then(Value::as_array)
            .filter(|c| !c.is_empty())
        else {
            continue;
        };
        let a = values::num(&other, "from").unwrap_or(0.);
        let b = a + values::num(&other, "dur").unwrap_or(0.);
        let a0 = a.max(a + values::num(&cues[0], "start").unwrap_or(0.));
        let b0 = b.min(a + values::num(cues.last().unwrap(), "end").unwrap_or(0.));
        if a0 < end && start < b0 {
            taken.insert(
                d.and_then(|d| d.get("style"))
                    .and_then(|s| values::text(s, "placement"))
                    .unwrap_or("bottom")
                    .to_owned(),
            );
        }
    }
    if taken.contains(&content.style.placement) {
        if let Some(free) = ["bottom", "top", "middle"]
            .into_iter()
            .find(|p| !taken.contains(*p))
        {
            content.style.placement = free.into();
            content.style.offset_y = 0.;
        }
    }
}
fn explicit_placement(style: &Json) -> bool {
    style.get("placement").is_some() || style.get("offsetY").is_some()
}
fn add(cx: &mut Cx<'_>, c: &AddCaptions) -> Result<(), CommandError> {
    if cx.find(&c.id).is_ok() {
        return Err(CommandError::Invalid(format!(
            "Layer id already exists: {}",
            c.id
        )));
    }
    let from = values::finite(c.from.unwrap_or(0.), "Caption start")?.max(0.);
    let comp = c.comp.as_deref();
    let h = height(cx, comp);
    let w = cx
        .comp_node(comp)?
        .get("w")
        .and_then(crate::model::number_of)
        .unwrap_or(1920.);
    let dur = cx.duration(comp);
    let style = c.style.as_ref().map(json).transpose()?.unwrap_or(json!({}));
    let preset = style["preset"].as_str().unwrap_or("classic");
    let mut content = pm_captions::normalize_captions_content(
        &json!({"cues":source(&c.source, from)?,"style":pm_captions::patch_style(&pm_captions::preset_style(preset,h),&style,h),"language":c.language}),
        &mut cue_id,
    );
    let mut layer = layers::make_caption_layer(
        &c.id,
        c.name.as_deref().filter(|name| !name.is_empty()),
        (w, h, dur),
    )?;
    values::obj(&mut layer).insert("from", Value::Number(from));
    let duration = match c.duration {
        Some(v) => values::finite(v, "Caption duration")?.max(1. / cx.fps(comp)),
        None => (dur - from)
            .max(0.1)
            .max(pm_captions::captions_end(&content.cues)),
    };
    values::obj(&mut layer).insert("dur", Value::Number(duration));
    if !explicit_placement(&style) {
        stack_placement(cx, comp, &layer, &mut content);
    }
    write_content(&mut layer, &content)?;
    cx.insert_layer(
        comp,
        c.index.unwrap_or(0).min(cx.layer_ids(comp).len()),
        layer,
    )
}
fn edit_layer(cx: &mut Cx<'_>, id: &str, edit: &CaptionEdit) -> Result<(), CommandError> {
    let (loc, mut layer) = cx.layer_mut_checked(id)?;
    if values::text(&layer, "type") != Some("captions") {
        return Err(CommandError::Invalid(format!(
            "“{}” is not a captions layer",
            values::layer_name(&layer, id)
        )));
    }
    let mut content = content(&layer)?;
    let before = content.cues.clone();
    let shift = values::num(&layer, "from").unwrap_or(0.);
    let mut replace = false;
    match edit {
        CaptionEdit::Replace(input) => {
            content.cues = pm_captions::normalize_cues(&source(input, shift)?, &mut cue_id);
            replace = true;
        }
        CaptionEdit::Insert(cues) => {
            content.cues = pm_captions::insert_cues(&before, &shifted(cues, shift)?, &mut cue_id)
        }
        CaptionEdit::Update(patches) => {
            let patches = shifted(patches, shift)?;
            for patch in patches.as_array().into_iter().flatten() {
                if !before
                    .iter()
                    .any(|c| Some(c.id.as_str()) == patch["id"].as_str())
                {
                    return Err(CommandError::Invalid(format!(
                        "Cue not found: {}",
                        patch["id"]
                    )));
                }
            }
            content.cues = pm_captions::update_cues(&before, &patches);
        }
        CaptionEdit::Delete(ids) => content.cues = pm_captions::delete_cues(&before, ids),
        CaptionEdit::Split { id, at } | CaptionEdit::SplitWithId { id, at, .. } => {
            let tail_id = match edit {
                CaptionEdit::SplitWithId { tail_id, .. } => tail_id.clone(),
                _ => cue_id(),
            };
            let (cues, tail) = pm_captions::split_cue(
                &before,
                id,
                values::finite(*at, "Caption split")? - shift,
                &tail_id,
            );
            if tail.is_none() {
                return Err(CommandError::Invalid(
                    "Split a cue at a time inside it, at least one frame from its edges".into(),
                ));
            }
            content.cues = cues;
        }
        CaptionEdit::Merge(ids) => {
            let (cues, merged) = pm_captions::merge_cues(&before, ids);
            if merged.is_none() {
                return Err(CommandError::Invalid(
                    "Merge needs at least two cues".into(),
                ));
            }
            content.cues = cues;
        }
        CaptionEdit::Move { ids, by } => {
            content.cues =
                pm_captions::move_cues(&before, ids, values::finite(*by, "Caption move")?);
        }
        CaptionEdit::MoveAll { by } => {
            let ids = before.iter().map(|c| c.id.clone()).collect::<Vec<_>>();
            content.cues =
                pm_captions::move_cues(&before, &ids, values::finite(*by, "Caption move")?);
        }
        CaptionEdit::Import {
            source: input,
            replace: replacing,
        } => {
            let cues = source(input, shift)?;
            content.cues = if *replacing {
                pm_captions::normalize_cues(&cues, &mut cue_id)
            } else {
                pm_captions::insert_cues(&before, &cues, &mut cue_id)
            };
            replace = *replacing;
        }
        CaptionEdit::Style { style, language } => {
            let patch = json(style)?;
            let old = content.style.placement.clone();
            content.style = pm_captions::patch_style(
                &content.style,
                &patch,
                height(cx, loc.comp_id.as_deref()),
            );
            if let Some(language) = language {
                content.language = (!language.is_empty() && pm_captions::valid_language(language))
                    .then(|| language.clone());
            }
            if old != content.style.placement && !explicit_placement(&patch) {
                stack_placement(cx, loc.comp_id.as_deref(), &layer, &mut content);
            }
        }
    }
    let end = content
        .cues
        .iter()
        .filter(|c| {
            replace
                || before
                    .iter()
                    .find(|old| old.id == c.id)
                    .is_none_or(|old| old.start != c.start || old.end != c.end)
        })
        .map(|c| c.end)
        .fold(0f64, f64::max);
    if end > values::num(&layer, "dur").unwrap_or(0.) {
        values::obj(&mut layer).insert("dur", Value::Number((end * 1e6).round() / 1e6));
    }
    write_content(&mut layer, &content)?;
    cx.put_layer(&loc, layer)
}

pub(super) fn set_content(layer: &mut Value, patch: &Map) -> Result<(), CommandError> {
    for key in patch.keys() {
        if !["cues", "style", "language"].contains(&key.to_string_lossy().as_str()) {
            return Err(CommandError::Invalid(format!(
                "Captions content field “{}” is not editable; use edit_captions",
                key.to_string_lossy()
            )));
        }
    }
    let mut next = layer.clone();
    let mut d = next
        .get("d")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (key, value) in patch.iter() {
        let value = if key.as_str() == Some("style") && value.as_object().is_some() {
            let mut style = d
                .get("style")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            for (key, value) in value.as_object().unwrap().iter() {
                style.insert(key.clone(), value.clone());
            }
            Value::Object(style)
        } else {
            value.clone()
        };
        d.insert(key.clone(), value);
    }
    values::obj(&mut next).insert("d", Value::Object(d));
    let normalized = content(&next)?;
    write_content(&mut next, &normalized)?;
    *layer = next;
    Ok(())
}
