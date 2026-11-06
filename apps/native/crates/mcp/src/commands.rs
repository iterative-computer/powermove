//! Legacy command JSON -> typed kernel commands. A scratch store resolves
//! names, $selection and 1-based references after each preceding command.
//! The real kernel receives the complete batch once, with a revision guard.
use crate::{Error, Result, json_value, model};
use pm_kernel::commands::{
    captions::{AddCaptions, CaptionCommand, CaptionEdit, CaptionFormat, CueSource},
    comps::CompPatch,
    effects::EffectPatch,
    keyframes::{EaseSpec, KeyframeSpec, ReplaceKeyframes, SimplifyScope},
    layers::{AddLayer, LayerPatch, ParentChange},
    properties::{KeyMode, SetProperty},
};
use pm_kernel::{
    Command, CompCommand, ContentCommand, DocumentStore, EffectCommand, GroupCommand,
    KeyframeCommand, LayerCommand, LibraryCommand, MarkerCommand, Origin, PropertyCommand,
};
use serde_json::{Value, json};

pub const OPERATIONS: &[&str] = &[
    "set_property",
    "replace_keyframes",
    "simplify_keyframes",
    "set_easing",
    "set_expression",
    "set_content",
    "set_layer",
    "set_composition",
    "add_layer",
    "delete_layers",
    "reorder_layer",
    "group_layers",
    "ungroup_layers",
    "move_to_group",
    "add_effect",
    "remove_effect",
    "set_effect",
    "set_transition",
    "set_scene_parameter",
    "add_marker",
    "remove_marker",
    "create_section",
    "update_section",
    "transform_layers",
    "add_captions",
    "edit_captions",
];
pub struct Batch {
    pub commands: Vec<Command>,
    pub results: Vec<Value>,
    pub descriptions: Vec<String>,
    pub selection: Vec<String>,
}
fn text(v: &Value, k: &str) -> Option<String> {
    v[k].as_str().map(str::to_owned)
}
fn required(v: &Value, k: &str) -> Result<String> {
    text(v, k)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::invalid(format!("{k} is required")))
}
fn number(v: &Value, k: &str, default: f64) -> Result<f64> {
    match v.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_f64()
            .ok_or_else(|| Error::invalid(format!("{k} must be a finite number"))),
    }
}
fn map(v: &Value) -> Result<pm_model::Map> {
    if v.is_null() {
        Ok(Default::default())
    } else {
        model(v)
            .as_object()
            .cloned()
            .ok_or_else(|| Error::invalid("Patch must be an object"))
    }
}
fn index(v: &Value, k: &str) -> Result<usize> {
    v[k].as_u64()
        .map(|v| v as usize)
        .ok_or_else(|| Error::invalid(format!("{k} must be a nonnegative integer")))
}
fn ease(v: &Value) -> Result<Option<EaseSpec>> {
    if v.is_null() {
        Ok(None)
    } else if let Some(s) = v.as_str() {
        Ok(Some(EaseSpec::Preset(s.into())))
    } else {
        let a = v
            .as_array()
            .filter(|a| a.len() == 4)
            .ok_or_else(|| Error::invalid("Easing curve must contain four handles"))?;
        let mut c = [0.; 4];
        for (i, n) in a.iter().enumerate() {
            c[i] = n
                .as_f64()
                .ok_or_else(|| Error::invalid("Invalid easing handle"))?;
        }
        Ok(Some(EaseSpec::Curve(c)))
    }
}
pub fn layer_patch(v: &Value, time: f64) -> Result<LayerPatch> {
    let allowed = [
        "name",
        "from",
        "duration",
        "visible",
        "locked",
        "shy",
        "solo",
        "blend",
        "motionBlur",
        "parent",
        "matteSource",
        "color",
        "collapsed",
        "threeD",
        "scaleLinked",
        "separateDimensions",
        "continuousRaster",
        "effectSpace",
    ];
    let o = v
        .as_object()
        .ok_or_else(|| Error::invalid("Layer patch must be an object"))?;
    for k in o.keys() {
        if !allowed.contains(&k.as_str()) {
            return Err(Error::invalid(format!("Layer field {k} is not editable")));
        }
    }
    let num = |k: &str| -> Result<Option<f64>> {
        v.get(k)
            .filter(|v| !v.is_null())
            .map(|_| number(v, k, 0.))
            .transpose()
    };
    Ok(LayerPatch {
        name: text(v, "name"),
        from: num("from")?,
        duration: num("duration")?,
        visible: v["visible"].as_bool(),
        locked: v["locked"].as_bool(),
        shy: v["shy"].as_bool(),
        solo: v["solo"].as_bool(),
        blend: text(v, "blend"),
        motion_blur: v["motionBlur"].as_bool(),
        parent: v.get("parent").map(|p| ParentChange {
            parent: p.as_str().map(str::to_owned),
            time,
        }),
        matte_source: v.get("matteSource").map(|p| p.as_str().map(str::to_owned)),
        color: text(v, "color"),
        collapsed: v["collapsed"].as_bool(),
        three_d: v["threeD"].as_bool(),
        scale_linked: v["scaleLinked"].as_bool(),
        separate_dimensions: v["separateDimensions"].as_bool(),
        continuous_raster: v["continuousRaster"].as_bool(),
        effect_space: match v["effectSpace"].as_str() {
            None => None,
            Some("source") => Some(pm_kernel::commands::layers::EffectSpace::Source),
            Some("composition") => Some(pm_kernel::commands::layers::EffectSpace::Composition),
            _ => return Err(Error::invalid("Invalid effectSpace")),
        },
        time,
    })
}
fn comp_patch(v: &Value) -> Result<CompPatch> {
    let allowed = [
        "name",
        "width",
        "height",
        "fps",
        "duration",
        "background",
        "backgroundFill",
        "shutter",
        "workArea",
        "colorDepth",
        "audioGain",
    ];
    let o = v
        .as_object()
        .ok_or_else(|| Error::invalid("Composition patch must be an object"))?;
    for k in o.keys() {
        if !allowed.contains(&k.as_str()) {
            return Err(Error::invalid(format!(
                "Composition field {k} is not editable"
            )));
        }
    }
    let num = |k: &str| -> Result<Option<f64>> {
        v.get(k)
            .filter(|v| !v.is_null())
            .map(|_| number(v, k, 0.))
            .transpose()
    };
    let work_area = if let Some(a) = v.get("workArea") {
        let a = a
            .as_array()
            .filter(|a| a.len() == 2)
            .ok_or_else(|| Error::invalid("workArea must contain start and end"))?;
        Some((
            a[0].as_f64()
                .ok_or_else(|| Error::invalid("Invalid workArea"))?,
            a[1].as_f64()
                .ok_or_else(|| Error::invalid("Invalid workArea"))?,
        ))
    } else {
        None
    };
    Ok(CompPatch {
        name: text(v, "name"),
        width: num("width")?,
        height: num("height")?,
        fps: num("fps")?,
        duration: num("duration")?,
        background: text(v, "background"),
        background_fill: v.get("backgroundFill").map(model),
        shutter: num("shutter")?,
        work_area,
        color_depth: v["colorDepth"].as_u64().map(|v| v as u32),
        hdr: None,
        mastering_peak: None,
        audio_gain: num("audioGain")?,
    })
}
fn realized(store: &DocumentStore) -> Value {
    json_value(&pm_model::canonical::realize_project(
        store.snapshot().document.to_value().unwrap(),
    ))
}
fn resolve(v: &Value, p: &Value, selection: &[String]) -> Result<String> {
    let layers = p["layers"]
        .as_array()
        .ok_or_else(|| Error::invalid("Missing layers"))?;
    if v == "$selection" {
        return selection
            .first()
            .cloned()
            .ok_or_else(|| Error::invalid("No selected layer"));
    }
    if let Some(n) = v.as_u64() {
        return layers
            .get(n.wrapping_sub(1) as usize)
            .and_then(|l| l["id"].as_str())
            .map(str::to_owned)
            .ok_or_else(|| Error::invalid("Layer index not found"));
    }
    let s = v
        .as_str()
        .ok_or_else(|| Error::invalid("Layer target is required"))?;
    let mut matches = layers.iter().filter(|l| l["id"] == s);
    if let Some(l) = matches.next() {
        return Ok(l["id"].as_str().unwrap().into());
    }
    let matches = layers
        .iter()
        .filter(|l| {
            l["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(s))
        })
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        Ok(matches[0]["id"].as_str().unwrap().into())
    } else {
        Err(Error::invalid(format!("Layer not found or ambiguous: {s}")))
    }
}
fn targets(c: &Value, p: &Value, selection: &[String]) -> Result<Vec<String>> {
    if let Some(a) = c["targets"].as_array() {
        a.iter().map(|v| resolve(v, p, selection)).collect()
    } else if let Some(t) = c.get("target") {
        if t == "$selection" {
            Ok(selection.to_vec())
        } else {
            Ok(vec![resolve(t, p, selection)?])
        }
    } else {
        Ok(selection.to_vec())
    }
}
/// The tagged harness cleans the entire batch before revision admission or
/// command semantics. In particular a malformed later command cannot expose a
/// stale-revision/earlier-target error or apply a prefix of a batch.
pub fn clean_commands(raw: &[Value]) -> Result<Vec<Value>> {
    const LIMIT: usize = 1_000_000;
    if raw.is_empty() {
        return Err(Error::invalid(
            "No valid Powermove edit commands were supplied.",
        ));
    }
    raw.iter().enumerate().map(|(index, raw)| {
        let invalid = || Error::invalid(format!("Invalid or oversized command at index {index}. No commands were applied. Each command must be valid JSON with a supported type and at most {LIMIT} characters."));
        let source = if let Some(text) = raw.as_str() {
            if text.encode_utf16().count() > LIMIT { return Err(invalid()); }
            serde_json::from_str::<Value>(text).map_err(|_| invalid())?
        } else { raw.clone() };
        let kind = source["type"].as_str().filter(|k| OPERATIONS.contains(k)).ok_or_else(invalid)?;
        let fields = crate::catalog::catalog()["commandFields"][kind].as_array().ok_or_else(invalid)?;
        let mut cleaned = serde_json::Map::new();
        for key in fields {
            let key = key.as_str().unwrap();
            if let Some(value) = source.get(key) {cleaned.insert(key.into(),value.clone());}
        }
        let mut command = Value::Object(cleaned);
        let serialized = pm_model::json::to_canonical_string(&model(&command));
        if serialized.encode_utf16().count() > LIMIT {return Err(invalid());}
        if ["replace_keyframes", "set_easing"].contains(&kind) && !command["keyframes"].is_array() {return Err(invalid());}
        if kind == "add_layer" && command.get("from").is_none() {command["from"] = json!(0);}
        if kind == "create_section" {
            if !command["section"]["layers"].is_array() {return Err(invalid());}
            if let Some(section) = command["section"].as_object_mut() {
                for key in ["__proto__", "prototype", "constructor"] {section.remove(key);}
            }
        }
        if kind == "update_section" && (!truthy(&command["sectionId"]) || command["layers"].as_array().is_none_or(|l| l.is_empty())) {return Err(invalid());}
        if ["set_property", "replace_keyframes"].contains(&kind) {command["preserveHandEdits"] = json!(command["preserveHandEdits"] != false);}
        Ok(command)
    }).collect()
}
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64() != Some(0.0),
        _ => true,
    }
}

pub fn plan(project: &Value, selection: &[String], raw: &[Value], time: f64) -> Result<Batch> {
    plan_with_definitions(project, selection, raw, time, None)
}
pub fn plan_with_definitions(
    project: &Value,
    selection: &[String],
    raw: &[Value],
    time: f64,
    state: Option<&pm_kernel::state::StateSnapshot>,
) -> Result<Batch> {
    let cleaned = clean_commands(raw)?;
    plan_cleaned_with_definitions(project, selection, &cleaned, time, state)
}

pub(crate) fn plan_cleaned_with_definitions(
    project: &Value,
    selection: &[String],
    cleaned: &[Value],
    time: f64,
    state: Option<&pm_kernel::state::StateSnapshot>,
) -> Result<Batch> {
    let mut scratch = DocumentStore::new_untitled(model(project));
    if let Some(state) = state {
        scratch.set_runtime_state(state.clone(), None);
    }
    let mut batch = Batch {
        commands: vec![],
        results: vec![],
        descriptions: vec![],
        selection: selection.to_vec(),
    };
    for mut c in cleaned.iter().cloned() {
        let kind = c["type"].as_str().unwrap().to_owned();
        let p = realized(&scratch);
        let target = || resolve(&c["target"], &p, &batch.selection);
        let mut data = Value::Null;
        let cmd: Command = match kind.as_str() {
            "set_property" => {
                let id = target()?;
                let path = required(&c, "path")?;
                let mut s =
                    SetProperty::new(&id, &path, model(&c["value"]), number(&c, "time", time)?);
                s.mode = match c["mode"].as_str() {
                    None | Some("auto") => KeyMode::Auto,
                    Some("static") => KeyMode::Static,
                    Some("keyframe") => KeyMode::Keyframe,
                    _ => return Err(Error::invalid("Unknown property mode")),
                };
                s.ease = ease(&c["ease"])?;
                s.hold = c["hold"].as_bool();
                s.preserve_hand_edits = c["preserveHandEdits"] != false;
                c["preserveHandEdits"] = json!(s.preserve_hand_edits);
                data = json!({"id":id,"channel":path,"value":c["value"]});
                PropertyCommand::Set(s).into()
            }
            "transform_layers" => LayerCommand::TransformLayers {
                transform: model(&c["transform"]),
                state: if c["state"].is_object() {
                    model(&c["state"])
                } else {
                    pm_model::Value::Object(Default::default())
                },
                selection: batch.selection.clone(),
                time,
            }
            .into(),
            "set_expression" => {
                let id = target()?;
                let channel = required(&c, "path")?;
                let expression = text(&c, "expression").filter(|s| !s.is_empty());
                data = json!({"id":id,"channel":channel,"expression":expression});
                PropertyCommand::SetExpression {
                    layer_id: id,
                    channel,
                    expression,
                }
                .into()
            }
            "replace_keyframes" => {
                let id = target()?;
                let channel = required(&c, "path")?;
                let keys = c["keyframes"]
                    .as_array()
                    .ok_or_else(|| Error::invalid("keyframes must be an array"))?
                    .iter()
                    .map(|k| {
                        let mut spec =
                            KeyframeSpec::new(number(k, "time", 0.)?, model(&k["value"]));
                        spec.ease = ease(&k["ease"])?;
                        spec.hold = k["hold"] == true;
                        spec.id = text(k, "id");
                        spec.temporal = k.get("temporal").map(model);
                        spec.spring = k
                            .get("spring")
                            .map(|v| if v.is_null() { None } else { Some(model(v)) });
                        if let Some(a) = k["handles"].as_array() {
                            if a.len() != 4 {
                                return Err(Error::invalid("Expected four keyframe handles"));
                            }
                            let mut h = [0.; 4];
                            for (i, v) in a.iter().enumerate() {
                                h[i] = v
                                    .as_f64()
                                    .ok_or_else(|| Error::invalid("Invalid keyframe handle"))?;
                            }
                            spec.handles = Some(h);
                        }
                        Ok(spec)
                    })
                    .collect::<Result<Vec<_>>>()?;
                c["preserveHandEdits"] = json!(c["preserveHandEdits"] != false);
                data = json!({"id":id,"channel":channel});
                KeyframeCommand::Replace(Box::new(ReplaceKeyframes {
                    layer_id: id,
                    channel,
                    keyframes: keys,
                    replace: c["replace"] != false,
                    expression: c.get("expression").map(|v| v.as_str().map(str::to_owned)),
                    preserve_hand_edits: c["preserveHandEdits"] != false,
                    time,
                }))
                .into()
            }
            "set_easing" => {
                let key_ids = c["keyframes"]
                    .as_array()
                    .ok_or_else(|| Error::invalid("keyframes must be an array"))?
                    .iter()
                    .map(|k| {
                        k.as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| Error::invalid("Keyframe ids must be strings"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                data = json!({"keyframes":key_ids.len()});
                KeyframeCommand::SetEasing {
                    key_ids,
                    curve: ease(&c["curve"])?.ok_or_else(|| Error::invalid("curve is required"))?,
                }
                .into()
            }
            "simplify_keyframes" => {
                let scope = match c["scope"].as_str() {
                    Some("project") => SimplifyScope::All,
                    Some("layers") => SimplifyScope::Layers(targets(&c, &p, &batch.selection)?),
                    Some("properties") => SimplifyScope::Properties(
                        c["properties"]
                            .as_array()
                            .ok_or_else(|| Error::invalid("properties are required"))?
                            .iter()
                            .map(|r| {
                                Ok((
                                    resolve(&r["layerId"], &p, &batch.selection)?,
                                    required(r, "path")?,
                                ))
                            })
                            .collect::<Result<_>>()?,
                    ),
                    _ => return Err(Error::invalid("Invalid simplification scope")),
                };
                KeyframeCommand::Simplify { scope }.into()
            }
            "set_layer" => {
                let id = target()?;
                let mut patch = layer_patch(&c["patch"], time)?;
                if let Some(ref mut parent) = patch.parent
                    && let Some(ref s) = parent.parent
                {
                    parent.parent = Some(resolve(&json!(s), &p, &batch.selection)?);
                }
                data = json!({"id":id,"keys":c["patch"].as_object().unwrap().keys().collect::<Vec<_>>()});
                LayerCommand::Set {
                    layer_id: id,
                    patch,
                }
                .into()
            }
            "add_captions" => {
                let id = text(&c, "id").unwrap_or_else(|| pm_kernel::new_id("L"));
                c["id"] = json!(id);
                let source = caption_source(&mut c, true)?;
                data = json!({"id":id});
                CaptionCommand::Add(Box::new(AddCaptions {
                    id,
                    name: text(&c, "name"),
                    source,
                    style: c.get("style").filter(|v| !v.is_null()).map(model),
                    language: text(&c, "language"),
                    from: c
                        .get("from")
                        .filter(|v| !v.is_null())
                        .map(|_| number(&c, "from", 0.))
                        .transpose()?,
                    duration: c
                        .get("duration")
                        .filter(|v| !v.is_null())
                        .map(|_| number(&c, "duration", 0.))
                        .transpose()?,
                    index: c
                        .get("index")
                        .filter(|v| !v.is_null())
                        .map(|_| index(&c, "index"))
                        .transpose()?,
                    comp: None,
                }))
                .into()
            }
            "edit_captions" => {
                let id = target()?;
                let edit = match c["op"].as_str() {
                    Some("replace") => CaptionEdit::Replace(caption_source(&mut c, false)?),
                    Some("insert") => {
                        reserve_insert_ids(&mut c, &p, &id);
                        caption_ids(&mut c);
                        CaptionEdit::Insert(model(&c["cues"]))
                    }
                    Some("update") => CaptionEdit::Update(model(&c["cues"])),
                    Some("delete") => CaptionEdit::Delete(caption_target_ids(&c)?),
                    Some("split") => {
                        let tail_id = pm_kernel::new_id("cue");
                        data = json!({"id":id,"tailId":tail_id});
                        CaptionEdit::SplitWithId {
                            id: required(&c, "id")?,
                            at: number(&c, "at", 0.)?,
                            tail_id,
                        }
                    }
                    Some("merge") => CaptionEdit::Merge(caption_target_ids(&c)?),
                    Some("move") => {
                        let by = number(&c, "by", 0.)?;
                        if c.get("ids").is_none_or(Value::is_null) {
                            CaptionEdit::MoveAll { by }
                        } else {
                            CaptionEdit::Move {
                                ids: caption_target_ids(&c)?,
                                by,
                            }
                        }
                    }
                    Some("import") => CaptionEdit::Import {
                        source: caption_source(&mut c, false)?,
                        replace: c["replace"] != false,
                    },
                    Some("style") => CaptionEdit::Style {
                        style: model(&c["style"]),
                        language: text(&c, "language"),
                    },
                    _ => {
                        return Err(Error::invalid(format!(
                            "Unknown captions operation: {}",
                            c["op"]
                        )));
                    }
                };
                if data.is_null() {
                    data = json!({"id":id});
                }
                if let Some(language) = c.get("language") {
                    CaptionCommand::EditWithLanguage {
                        layer_id: id,
                        edit,
                        language: (!language.is_null()).then(|| caption_text(language)),
                    }
                    .into()
                } else {
                    CaptionCommand::Edit { layer_id: id, edit }.into()
                }
            }
            "set_content" => {
                let id = target()?;
                data = json!({"id":id,"keys":c["patch"].as_object().map(|m|m.keys().collect::<Vec<_>>()).unwrap_or_default()});
                ContentCommand::Set {
                    layer_id: id,
                    patch: map(&c["patch"])?,
                    time,
                }
                .into()
            }
            "set_composition" => {
                data = json!({"keys":c["patch"].as_object().map(|m|m.keys().collect::<Vec<_>>()).unwrap_or_default()});
                CompCommand::Set {
                    patch: Box::new(comp_patch(&c["patch"])?),
                }
                .into()
            }
            "add_layer" => {
                let id = text(&c, "id").unwrap_or_else(|| pm_kernel::new_id("L"));
                let from = number(&c, "from", 0.)?;
                c["from"] = json!(from);
                if c["layerType"] == "captions"
                    && let Some(content) = c.get_mut("content")
                {
                    caption_ids(content);
                }
                data = json!({"id":id});
                LayerCommand::Add(Box::new(AddLayer {
                    id,
                    layer_type: required(&c, "layerType")?,
                    comp: None,
                    name: text(&c, "name"),
                    from: Some(from),
                    duration: c
                        .get("duration")
                        .map(|_| number(&c, "duration", 0.))
                        .transpose()?,
                    content: map(&c["content"])?,
                    properties: map(&c["properties"])?,
                    color: text(&c, "color"),
                    index: c.get("index").map(|_| index(&c, "index")).transpose()?,
                    parent: c
                        .get("parent")
                        .filter(|v| !v.is_null())
                        .map(|v| resolve(v, &p, &batch.selection))
                        .transpose()?,
                    blend: text(&c, "blend"),
                    motion_blur: c["motionBlur"].as_bool(),
                    visible: c["visible"].as_bool(),
                    shy: c["shy"].as_bool(),
                    collapsed: c["collapsed"].as_bool(),
                    time,
                }))
                .into()
            }
            "delete_layers" => {
                let ids = targets(&c, &p, &batch.selection)?;
                data = json!({"ids":ids});
                LayerCommand::Delete { layer_ids: ids }.into()
            }
            "reorder_layer" => {
                let id = target()?;
                let index = index(&c, "index")?;
                data = json!({"id":id,"index":index});
                LayerCommand::Reorder {
                    layer_id: id,
                    index,
                    time,
                }
                .into()
            }
            "group_layers" => {
                let group_id = pm_kernel::new_id("L");
                data = json!({"id":group_id});
                GroupCommand::Group {
                    layer_ids: targets(&c, &p, &batch.selection)?,
                    group_id,
                    name: text(&c, "name"),
                    time,
                }
                .into()
            }
            "ungroup_layers" => {
                let group_ids = targets(&c, &p, &batch.selection)?;
                let members = p["layers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|l| {
                        l["group"]
                            .as_str()
                            .is_some_and(|g| group_ids.iter().any(|id| id == g))
                    })
                    .map(|l| l["id"].clone())
                    .collect::<Vec<_>>();
                data = json!({"ids":members});
                GroupCommand::Ungroup { group_ids, time }.into()
            }
            "move_to_group" => {
                let layer_ids = targets(&c, &p, &batch.selection)?;
                let group = if c["group"].is_null() {
                    None
                } else {
                    Some(resolve(&c["group"], &p, &batch.selection)?)
                };
                data = json!({"ids":layer_ids,"group":group});
                GroupCommand::MoveToGroup {
                    layer_ids,
                    group,
                    time,
                }
                .into()
            }
            "add_effect" => {
                let id = target()?;
                let name = required(&c, "effect")?;
                let definition = definition(state, "effects", &name)?;
                let effect_id = pm_kernel::new_id("f");
                data = json!({"id":id,"effectId":effect_id});
                EffectCommand::Add(Box::new(pm_kernel::commands::effects::AddEffect {
                    layer_id: id,
                    effect_id,
                    definition: pm_kernel::commands::effects::EffectDefinition {
                        effect_type: name,
                        version: definition["version"].as_u64().unwrap_or(1) as u32,
                        label: definition["label"].as_str().unwrap_or("Effect").into(),
                        params: parameters(&definition)?,
                        source_processing: definition["sourceProcessing"] == true,
                        backdrop: definition["backdrop"] == true,
                    },
                    parameters: map(&c["parameters"])?,
                    inputs: c.get("inputs").map(model),
                    open: c["open"] != false,
                    enabled: true,
                    enabled_animation: None,
                    index: None,
                }))
                .into()
            }
            "remove_effect" => {
                let id = target()?;
                data = json!({"id":id,"effect":c["effect"]});
                EffectCommand::Remove {
                    layer_id: id,
                    effect: required(&c, "effect")?,
                }
                .into()
            }
            "set_effect" => {
                let id = target()?;
                let patch = &c["patch"];
                let object = patch
                    .as_object()
                    .ok_or_else(|| Error::invalid("Effect patch must be an object"))?;
                if object
                    .keys()
                    .any(|k| !["enabled", "open", "index"].contains(&k.as_str()))
                {
                    return Err(Error::invalid("Unsupported effect field"));
                }
                let instance = p["layers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|l| l["id"] == id)
                    .and_then(|l| l["fx"].as_array())
                    .and_then(|a| {
                        a.iter()
                            .find(|fx| fx["id"] == c["effect"] || fx["type"] == c["effect"])
                    })
                    .ok_or_else(|| Error::invalid("Effect not found"))?;
                data = json!({"id":id,"effectId":instance["id"]});
                EffectCommand::Set {
                    layer_id: id,
                    effect: required(&c, "effect")?,
                    patch: EffectPatch {
                        enabled: patch["enabled"].as_bool().or(patch["on"].as_bool()),
                        open: patch["open"].as_bool(),
                        index: patch
                            .get("index")
                            .map(|_| index(patch, "index"))
                            .transpose()?,
                        inputs: patch.get("inputs").map(model),
                        time,
                    },
                }
                .into()
            }
            "set_transition" => {
                let id = resolve(c.get("layer").unwrap_or(&c["target"]), &p, &batch.selection)?;
                let edge = match c["edge"].as_str() {
                    Some("in") => pm_kernel::commands::effects::Edge::In,
                    Some("out") => pm_kernel::commands::effects::Edge::Out,
                    _ => return Err(Error::invalid("Transition edge must be in or out")),
                };
                let transition = if c["transition"].is_null() {
                    None
                } else {
                    let name = required(&c["transition"], "type")?;
                    let definition = definition(state, "transitions", &name)?;
                    Some(Box::new(pm_kernel::commands::effects::TransitionSpec {
                        definition: pm_kernel::commands::effects::TransitionDefinition {
                            transition_type: name,
                            params: parameters(&definition)?,
                        },
                        duration: c["transition"]
                            .get("dur")
                            .map(|_| number(&c["transition"], "dur", 0.5))
                            .transpose()?,
                        params: map(&c["transition"]["p"])?,
                    }))
                };
                data = json!({"id":id,"edge":c["edge"],"transition":c["transition"]["type"]});
                EffectCommand::SetTransition {
                    layer_id: id,
                    edge,
                    transition,
                }
                .into()
            }
            "set_scene_parameter" => {
                let name = required(&c, "name")?;
                data = json!({"name":name,"value":c["value"]});
                CompCommand::SetSceneParameter {
                    name,
                    label: text(&c, "label"),
                    control: text(&c, "control"),
                    value: model(&c["value"]),
                    min: c.get("min").map(model),
                    max: c.get("max").map(model),
                    options: c.get("options").map(model),
                }
                .into()
            }
            "add_marker" => {
                let id = text(&c, "id").unwrap_or_else(|| pm_kernel::new_id("marker"));
                data = json!({"id":id});
                MarkerCommand::Add {
                    comp: None,
                    id,
                    time: number(&c, "time", time)?,
                    name: text(&c, "name"),
                }
                .into()
            }
            "remove_marker" => {
                let idx = index(&c, "index")?;
                data = p["markers"]
                    .get(idx)
                    .cloned()
                    .ok_or_else(|| Error::invalid("Marker not found"))?;
                MarkerCommand::Remove {
                    comp: None,
                    index: idx,
                }
                .into()
            }
            "create_section" => {
                data = json!({"id":c["section"]["id"]});
                LibraryCommand::CreateSection {
                    section: model(&c["section"]),
                }
                .into()
            }
            "update_section" => {
                let section_id = required(&c, "sectionId")?;
                data = json!({"id":section_id});
                LibraryCommand::UpdateSection {
                    section_id,
                    layers: c["layers"]
                        .as_array()
                        .ok_or_else(|| Error::invalid("Section layers are required"))?
                        .iter()
                        .map(model)
                        .collect(),
                    thumb: c.get("thumb").map(model),
                    at: number(&c, "at", 0.)?,
                    version: c.get("version").map(model),
                }
                .into()
            }
            _ => return Err(Error::unavailable(format!("kernel command: {kind}"))),
        };
        scratch
            .execute(cmd.clone(), Origin::Mcp)
            .map_err(Error::kernel)?;
        let after = realized(&scratch);
        if kind == "group_layers" {
            batch.selection = vec![data["id"].as_str().unwrap().into()];
        }
        if kind == "ungroup_layers" {
            batch.selection = data["ids"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
        }
        if kind == "remove_effect" {
            let id = data["id"].as_str().unwrap();
            let count = |value: &Value| {
                value["layers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|l| l["id"] == id)
                    .and_then(|l| l["fx"].as_array())
                    .map_or(0, Vec::len)
            };
            data = json!({"id":id,"removed":count(&p)-count(&after)});
        }
        if kind == "set_easing" {
            let ids = c["keyframes"].as_array().unwrap();
            let before = tracks(&p)?;
            let count = before
                .iter()
                .flat_map(|(_, _, p)| p["kf"].as_array().into_iter().flatten())
                .filter(|k| ids.contains(&k["i"]))
                .count();
            let curve = if c["curve"].is_string() {
                crate::catalog::catalog()["easePresets"][c["curve"].as_str().unwrap()].clone()
            } else {
                c["curve"].clone()
            };
            data = json!({"keyframes":count,"curve":curve});
        }
        if kind == "simplify_keyframes" {
            let before = tracks(&p)?;
            let after_tracks = tracks(&after)?;
            let wanted = |id: &str, path: &str| -> bool {
                match c["scope"].as_str() {
                    Some("layers") => targets(&c, &p, &batch.selection)
                        .unwrap_or_default()
                        .iter()
                        .any(|v| v == id),
                    Some("properties") => c["properties"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|r| r["layerId"] == id && r["path"] == path),
                    _ => true,
                }
            };
            let (mut keys_before, mut keys_after, mut changed, mut skipped) = (0, 0, 0, 0);
            for (id, path, prop) in before.iter().filter(|(id, path, _)| wanted(id, path)) {
                let n = prop["kf"].as_array().map_or(0, Vec::len);
                let next = after_tracks
                    .iter()
                    .find(|(a, b, _)| a == id && b == path)
                    .map(|(_, _, p)| p)
                    .unwrap_or(prop);
                let m = next["kf"].as_array().map_or(0, Vec::len);
                keys_before += n;
                keys_after += m;
                if prop["kf"] != next["kf"] {
                    changed += 1;
                } else if n > 2 {
                    skipped += 1;
                }
            }
            data = json!({"keysBefore":keys_before,"keysAfter":keys_after,"propertiesChanged":changed,"propertiesSkipped":skipped});
        }

        let id = data["id"].as_str().unwrap_or("").to_owned();
        let layer = after["layers"]
            .as_array()
            .and_then(|a| a.iter().find(|l| l["id"] == id));
        if kind == "add_layer" || kind == "add_captions" {
            let l = layer.unwrap();
            data = if kind == "add_captions" {
                json!({"id":l["id"],"name":l["name"],"cues":l["d"]["cues"].as_array().map_or(0,Vec::len)})
            } else {
                json!({"id":l["id"],"name":l["name"],"layer":l})
            };
            if c["select"] != false {
                batch.selection = vec![id.clone()];
            }
        }
        if kind == "edit_captions" {
            let cues = layer
                .and_then(|l| l["d"]["cues"].as_array())
                .ok_or_else(|| Error::kernel("Caption edit did not publish its cue list"))?;
            data["cues"] = json!(cues.len());
            if c["op"] == "merge" {
                let ids = caption_target_ids(&c)?;
                data["cue"] = cues
                    .iter()
                    .find(|cue| {
                        cue["id"]
                            .as_str()
                            .is_some_and(|id| ids.iter().any(|wanted| wanted == id))
                    })
                    .map(|cue| cue["id"].clone())
                    .unwrap_or(Value::Null);
            }
        }
        if kind == "add_marker" {
            data = after["markers"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["id"] == id)
                .unwrap()
                .clone();
        }
        if kind == "replace_keyframes" {
            let path = c["path"].as_str().unwrap();
            let props = crate::views::properties(layer.unwrap());
            let count = props
                .iter()
                .find(|(k, _)| k == path)
                .and_then(|(_, v)| v["kf"].as_array())
                .map_or(0, Vec::len);
            data["keyframes"] = json!(count);
        }
        if kind == "update_section" {
            data["versions"] = json!(
                after["library"]["sections"]
                    .as_array()
                    .and_then(|a| a.iter().find(|s| s["id"] == id))
                    .and_then(|s| s["versions"].as_array())
                    .map_or(0, Vec::len)
            );
        }
        let existing = after["layers"].as_array().unwrap();
        batch
            .selection
            .retain(|id| existing.iter().any(|l| l["id"] == *id));
        batch.descriptions.push(describe(&c));
        batch.results.push(json!({"command":c,"data":data}));
        batch.commands.push(cmd);
    }
    Ok(batch)
}
pub fn describe(c: &Value) -> String {
    let kind = c["type"].as_str().unwrap_or("");
    let label = match kind {
        "set_property" => "Set property",
        "replace_keyframes" => "Animate",
        "simplify_keyframes" => "Simplify Keyframes",
        "set_easing" => "Set easing",
        "set_expression" => "Set expression",
        "set_content" => "Edit content",
        "add_captions" => "Add Captions",
        "edit_captions" => "Edit Captions",
        "set_layer" => "Edit layer",
        "set_composition" => "Edit composition",
        "add_layer" => "Add layer",
        "delete_layers" => "Delete",
        "reorder_layer" => "Reorder layer",
        "add_effect" => "Add effect",
        "remove_effect" => "Remove effect",
        "set_effect" => "Edit effect",
        "set_scene_parameter" => "Set scene control",
        "add_marker" => "Add marker",
        "remove_marker" => "Remove marker",
        "create_section" => "Create section",
        "update_section" => "Update section",
        _ => kind,
    };
    let target = c
        .get("target")
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| v.to_string())
        })
        .or_else(|| {
            c["targets"]
                .as_array()
                .map(|a| format!("{} layers", a.len()))
        })
        .or_else(|| text(c, "name"))
        .unwrap_or_default();
    let detail = text(c, "path")
        .or_else(|| text(c, "layerType"))
        .or_else(|| text(c, "effect"))
        .unwrap_or_else(|| {
            c["patch"]
                .as_object()
                .map(|m| m.keys().cloned().collect::<Vec<_>>().join(", "))
                .unwrap_or_default()
        });
    [label.to_owned(), target, detail]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
        .chars()
        .take(150)
        .collect()
}

fn caption_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => pm_model::json::number_to_js_string(n.as_f64().unwrap_or(f64::NAN)),
        Value::Object(_) => "[object Object]".into(),
        Value::Array(items) => items
            .iter()
            .map(|v| {
                if v.is_null() {
                    String::new()
                } else {
                    caption_text(v)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
    }
}
fn reserve_insert_ids(command: &mut Value, project: &Value, layer: &str) {
    let existing = project["layers"]
        .as_array()
        .and_then(|layers| layers.iter().find(|l| l["id"] == layer))
        .and_then(|l| l["d"]["cues"].as_array())
        .map(|c| {
            c.iter()
                .filter_map(|c| c["id"].as_str().map(str::to_owned))
                .collect::<std::collections::HashSet<_>>()
        })
        .unwrap_or_default();
    for cue in command
        .get_mut("cues")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        if cue["id"].as_str().is_some_and(|id| existing.contains(id)) {
            cue["id"] = json!(pm_kernel::new_id("cue"));
        }
    }
}
fn caption_ids(command: &mut Value) {
    let mut seen = std::collections::HashSet::new();
    for cue in command
        .get_mut("cues")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        if !cue.is_object() {
            continue;
        }
        let id = cue["id"].as_str().unwrap_or("");
        if id.is_empty() || !seen.insert(id.to_owned()) {
            let id = pm_kernel::new_id("cue");
            seen.insert(id.clone());
            cue["id"] = json!(id);
        }
    }
}
fn caption_target_ids(command: &Value) -> Result<Vec<String>> {
    command
        .get("ids")
        .filter(|v| !v.is_null())
        .map(|v| {
            v.as_array()
                .ok_or_else(|| Error::invalid("Caption ids must be an array"))?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| Error::invalid("Caption ids must be strings"))
                })
                .collect()
        })
        .unwrap_or(Ok(vec![]))
}
fn caption_source(command: &mut Value, offset_cues: bool) -> Result<CueSource> {
    if let Some(text) = command.get("text").filter(|v| !v.is_null()) {
        let text = text
            .as_str()
            .ok_or_else(|| Error::invalid("Caption text must be a string"))?
            .to_owned();
        let format = match command["format"].as_str() {
            Some("srt") => CaptionFormat::Srt,
            Some("vtt") => CaptionFormat::Vtt,
            None | Some("auto") | Some("text") => CaptionFormat::Text,
            _ => return Err(Error::invalid("Unknown caption format")),
        };
        let input = CueSource::Text {
            text,
            format,
            offset: number(command, "offset", 0.)?,
        };
        let mut cues = json_value(
            &pm_kernel::commands::captions::materialize_source(&input).map_err(Error::kernel)?,
        );
        for cue in cues.as_array_mut().into_iter().flatten() {
            cue["id"] = json!(pm_kernel::new_id("cue"));
        }
        command["cues"] = cues.clone();
        return Ok(CueSource::Cues(model(&cues)));
    }
    caption_ids(command);
    let mut cues = command["cues"].clone();
    if offset_cues {
        let offset = number(command, "offset", 0.)?;
        for cue in cues.as_array_mut().into_iter().flatten() {
            for key in ["start", "end"] {
                if let Some(value) = cue.get_mut(key) {
                    *value = json!(
                        value.as_f64().ok_or_else(|| Error::invalid(
                            "Caption times must be finite numbers"
                        ))? + offset
                    );
                }
            }
            for word in cue
                .get_mut("words")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                for key in ["start", "end"] {
                    if let Some(value) = word.get_mut(key) {
                        *value = json!(
                            value.as_f64().ok_or_else(|| Error::invalid(
                                "Caption word times must be finite numbers"
                            ))? + offset
                        );
                    }
                }
            }
        }
    }
    Ok(CueSource::Cues(model(&cues)))
}

fn definition(
    state: Option<&pm_kernel::state::StateSnapshot>,
    registry: &str,
    id: &str,
) -> Result<Value> {
    state
        .and_then(|s| s.provider(registry, id))
        .map(|r| json_value(&r.definition))
        .ok_or_else(|| Error::unavailable(format!("{registry} runtime definition: {id}")))
}
fn parameters(definition: &Value) -> Result<Vec<pm_kernel::commands::effects::ParamDefinition>> {
    definition["params"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|p| {
            if p.get("defaultFrom").is_some() {
                return Err(Error::unavailable("runtime-resolved parameter defaults"));
            }
            let kind = p["type"].as_str().unwrap_or("number").to_owned();
            Ok(pm_kernel::commands::effects::ParamDefinition {
                key: required(p, "k")?,
                reference: kind == "layer" || kind == "texture",
                kind,
                default: model(&p["def"]),
                min: p["min"].as_f64(),
                max: p["max"].as_f64(),
            })
        })
        .collect()
}
fn tracks(project: &Value) -> Result<Vec<(String, String, Value)>> {
    let mut store = DocumentStore::new_untitled(model(project));
    let canonical = json_value(&store.snapshot().document.to_value().unwrap());
    let mut out = vec![];
    let comps = canonical["comps"]
        .as_object()
        .map(|c| c.iter().collect::<Vec<_>>())
        .unwrap_or_default();
    for (comp, value) in comps {
        for (index, layer) in value["layers"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .enumerate()
        {
            let entries = json_value(
                &store
                    .queries()
                    .query(&model(
                        &json!({"op":"allProps","comp":comp,"layer":index,"t":0}),
                    ))
                    .map_err(Error::kernel)?,
            );
            for entry in entries.as_array().unwrap_or(&vec![]) {
                let mut prop = layer;
                let mut valid = true;
                if let Some(value) = entry["path"].get("value") {
                    prop = value;
                } else {
                    for part in entry["path"].as_array().unwrap_or(&vec![]) {
                        let next = if let Some(k) = part.as_str() {
                            prop.get(k)
                        } else {
                            prop.get(part.as_u64().unwrap_or(0) as usize)
                        };
                        if let Some(next) = next {
                            prop = next
                        } else {
                            valid = false;
                            break;
                        }
                    }
                }
                if valid {
                    out.push((
                        layer["id"].as_str().unwrap_or("").into(),
                        entry["key"].as_str().unwrap_or("").into(),
                        prop.clone(),
                    ));
                }
            }
        }
    }
    Ok(out)
}
