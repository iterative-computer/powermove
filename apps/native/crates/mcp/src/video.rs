//! Source-aware v1.2.0 clip gestures, planned before the single kernel write.
use crate::{Error, Result, commands, json_value, media, model};
use pm_kernel::{DocumentStore, LayerCommand, Origin, WriteBody};
use serde_json::{Value, json};
use std::collections::HashSet;

pub struct Plan {
    pub body: WriteBody,
    pub clip: Value,
}
fn n(v: &Value, key: &str, min: f64) -> Result<f64> {
    v[key]
        .as_f64()
        .filter(|v| *v >= min)
        .ok_or_else(|| Error::invalid(format!("{key} must be a finite number >= {min}.")))
}
fn value(v: &Value) -> &Value {
    if v.get("kf").is_some() { &v["v"] } else { v }
}
fn scalar(v: &Value, fallback: f64) -> f64 {
    value(v).as_f64().unwrap_or(fallback)
}
fn animated(v: &Value) -> bool {
    v["kf"].as_array().is_some_and(|a| !a.is_empty())
        || v["expr"].as_str().is_some_and(|s| !s.is_empty())
}
fn prop(v: f64) -> Value {
    json!({"v":v,"kf":[],"expr":null})
}
fn props_map(v: &mut Value, f: &mut impl FnMut(&mut Value)) {
    if let Some(o) = v.as_object_mut() {
        for p in o.values_mut() {
            f(p)
        }
    }
}
/// The allProps closure for video/audio: transforms, content, effects, masks,
/// transitions and layer channels. Arbitrary unknown metadata is untouched.
fn properties(l: &mut Value, mut f: impl FnMut(&mut Value)) {
    if let Some(p) = l.get_mut("p") {
        props_map(p, &mut f);
    }
    let kind = l["type"].as_str().unwrap_or("").to_owned();
    if let Some(d) = l.get_mut("d").and_then(Value::as_object_mut) {
        for (key, p) in d {
            if (kind == "video"
                && [
                    "fit",
                    "w",
                    "h",
                    "trim",
                    "speed",
                    "timeRemap",
                    "sourceTime",
                    "audioGain",
                ]
                .contains(&key.as_str()))
                || (kind == "audio"
                    && ["trim", "gain", "fadeIn", "fadeOut"].contains(&key.as_str()))
            {
                f(p);
            }
        }
    }
    for key in ["blend", "mblur", "matteMode"] {
        if let Some(p) = l.get_mut(key) {
            f(p);
        }
    }
    if let Some(fx) = l.get_mut("fx").and_then(Value::as_array_mut) {
        for fx in fx {
            if let Some(on) = fx.get_mut("on") {
                f(on);
            }
            if let Some(p) = fx.get_mut("p") {
                props_map(p, &mut f);
            }
        }
    }
    if let Some(masks) = l.get_mut("masks").and_then(Value::as_array_mut) {
        for m in masks {
            for key in ["shape", "mode", "on"] {
                if let Some(p) = m.get_mut(key) {
                    f(p);
                }
            }
            if let Some(p) = m.get_mut("p") {
                props_map(p, &mut f);
            }
        }
    }
    for key in ["transitionIn", "transitionOut"] {
        if let Some(p) = l.get_mut(key).and_then(|v| v.get_mut("p")) {
            props_map(p, &mut f);
        }
    }
}

fn keys(l: &mut Value, mut f: impl FnMut(&mut Value)) {
    properties(l, |p| {
        if let Some(keys) = p.get_mut("kf").and_then(Value::as_array_mut) {
            for k in keys {
                f(k)
            }
        }
    });
}
fn new_layer(
    p: &Value,
    kind: &str,
    name: &Value,
    from: f64,
    duration: f64,
    content: Value,
) -> Result<Value> {
    let id = pm_kernel::new_id("L");
    let batch = commands::plan(
        p,
        &[],
        &[
            json!({"type":"add_layer","id":id,"layerType":kind,"name":name,"from":from,"duration":duration,"content":content}),
        ],
        0.,
    )?;
    let mut scratch = DocumentStore::new_untitled(model(p));
    for c in batch.commands {
        scratch.execute(c, Origin::Mcp).map_err(Error::kernel)?;
    }
    let realized = json_value(&pm_model::canonical::realize_project(
        scratch.snapshot().document.to_value().unwrap(),
    ));
    Ok(realized["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == id)
        .unwrap()
        .clone())
}
fn protected(l: &Value, layers: &[Value], seen: &mut HashSet<String>) -> bool {
    if l["lock"] == true {
        return true;
    }
    let Some(group) = l["group"].as_str() else {
        return false;
    };
    if !seen.insert(group.into()) {
        return true;
    }
    layers
        .iter()
        .find(|l| l["id"] == group)
        .is_some_and(|g| protected(g, layers, seen))
}
pub fn plan(p: &Value, a: &Value, registry: Option<&dyn media::MediaRegistry>) -> Result<Plan> {
    let op = a["operation"].as_str().unwrap_or("");
    let layers = p["layers"]
        .as_array()
        .ok_or_else(|| Error::invalid("Missing layers"))?;
    let fps = p["fps"].as_f64().unwrap_or(30.);
    let comp_dur = p["dur"].as_f64().unwrap_or(0.);
    let runtime = |asset: &str| {
        registry
            .map(|r| r.state(p["id"].as_str().unwrap_or(""), asset))
            .unwrap_or_default()
    };
    let source_dur = |asset: &str| {
        runtime(asset)
            .duration
            .or_else(|| p["assets"][asset]["dur"].as_f64())
    };
    let mut next = p.clone();
    let mut clip = json!({});
    if op == "insert" {
        let asset = a["assetId"].as_str().ok_or_else(|| {
            Error::invalid("Choose an imported video/audio asset from get_project_state.")
        })?;
        let meta = &p["assets"][asset];
        let kind = meta["kind"]
            .as_str()
            .filter(|k| ["video", "audio"].contains(k))
            .ok_or_else(|| {
                Error::invalid("Choose an imported video/audio asset from get_project_state.")
            })?;
        let from = n(a, "from", 0.)?;
        let trim = if a.get("sourceIn").is_some() {
            n(a, "sourceIn", 0.)?
        } else {
            0.
        };
        let dur = n(a, "duration", 1. / fps)?;
        if source_dur(asset).is_none_or(|d| trim + dur > d + 1e-6) {
            return Err(Error::invalid(
                "The requested clip exceeds the available source duration.",
            ));
        }
        if from + dur > comp_dur + 1e-6 {
            return Err(Error::invalid(
                "Extend the composition before inserting beyond its end.",
            ));
        }
        let mut d = json!({"asset":asset,"trim":prop(trim)});
        if kind == "video" {
            d["speed"] = prop(1.);
            d["embeddedAudio"] = json!(runtime(asset).has_audio);
        }
        let l = new_layer(p, kind, &meta["name"], from, dur, d)?;
        clip["layerId"] = l["id"].clone();
        next["layers"].as_array_mut().unwrap().insert(0, l);
    } else {
        let index = layers
            .iter()
            .position(|l| l["id"] == a["layerId"] && a["layerId"].is_string())
            .ok_or_else(|| {
                Error::invalid("Choose a video or audio layer from get_project_state.")
            })?;
        let mut l = layers[index].clone();
        let kind = l["type"].as_str().unwrap_or("").to_owned();
        if !["video", "audio"].contains(&kind.as_str()) {
            return Err(Error::invalid(
                "Choose a video or audio layer from get_project_state.",
            ));
        }
        if protected(&l, layers, &mut HashSet::new()) {
            return Err(Error::invalid("The clip is locked."));
        }
        if l["locked_intent"]
            .as_object()
            .is_some_and(|o| o.values().any(|v| v != &json!(false) && !v.is_null()))
        {
            return Err(Error::invalid(
                "The clip contains protected hand edits. Edit unprotected channels explicitly with apply_commands.",
            ));
        }
        let from = l["from"].as_f64().unwrap_or(0.);
        let dur = l["dur"].as_f64().unwrap_or(0.);
        let end = from + dur;
        let d = l["d"].clone();
        let rate = if kind == "video" {
            scalar(&d["speed"], 1.)
        } else {
            1.
        };
        let trim = scalar(&d["trim"], 0.);
        if ["split", "trim", "speed", "separate_audio", "slip"].contains(&op) {
            for key in ["trim", "speed", "sourceTime", "timeRemap"] {
                if animated(&d[key])
                    || key == "timeRemap"
                        && value(&d[key]) != &Value::Null
                        && value(&d[key]) != &json!(false)
                        && value(&d[key]) != &json!(0)
                {
                    return Err(Error::invalid(
                        "This clip has animated timing or time remapping. Use property/keyframe commands to edit its timing explicitly.",
                    ));
                }
            }
            if rate <= 0. {
                return Err(Error::invalid(
                    "This clip needs explicit time-remapping edits.",
                ));
            }
        }
        clip["layerId"] = l["id"].clone();
        match op {
            "move" => {
                let at = n(a, "from", 0.)?;
                if at + dur > comp_dur + 1e-6 {
                    return Err(Error::invalid(
                        "Extend the composition before moving beyond its end.",
                    ));
                }
                l["from"] = json!(at);
            }
            "trim" => {
                let start = n(a, "start", 0.)?;
                let stop = n(a, "end", 0.)?;
                if stop > comp_dur || stop - start < 1. / fps - 1e-6 {
                    return Err(Error::invalid(
                        "Trim must retain at least one frame inside the composition.",
                    ));
                }
                let offset = start - from;
                let source_in = trim + offset * rate;
                let asset = d["asset"].as_str().unwrap_or("");
                if source_in < -1e-6
                    || (start < from || stop > end)
                        && source_dur(asset)
                            .is_none_or(|d| source_in + (stop - start) * rate > d + 1e-6)
                {
                    return Err(Error::invalid("Trim exceeds available source handles."));
                }
                keys(&mut l, |k| {
                    k["t"] = json!(k["t"].as_f64().unwrap_or(0.) - offset)
                });
                l["d"]["trim"] = prop(source_in);
                l["from"] = json!(start);
                l["dur"] = json!(stop - start);
            }
            "slip" => {
                let at = n(a, "sourceIn", 0.)?;
                if source_dur(d["asset"].as_str().unwrap_or(""))
                    .is_none_or(|d| at + dur * rate > d + 1e-6)
                {
                    return Err(Error::invalid("Slip exceeds available source handles."));
                }
                l["d"]["trim"] = prop(at);
            }
            "split" => {
                let at = n(a, "at", 0.)?;
                if at - from < 1. / fps - 1e-6 || end - at < 1. / fps - 1e-6 {
                    return Err(Error::invalid(
                        "Split must leave at least one frame on each side.",
                    ));
                }
                let offset = at - from;
                let mut tail = l.clone();
                tail["id"] = json!(pm_kernel::new_id("L"));
                let old = l["name"].as_str().unwrap_or("");
                let base = regex::Regex::new(r" \d+$")
                    .unwrap()
                    .replace(old, "")
                    .into_owned();
                let count = layers
                    .iter()
                    .filter(|l| l["name"].as_str().unwrap_or("").starts_with(&base))
                    .count();
                tail["name"] = json!(format!("{base} {}", count + 1));
                keys(&mut tail, |k| {
                    k["i"] = json!(pm_kernel::new_id("k"));
                    k["t"] = json!(k["t"].as_f64().unwrap_or(0.) - offset);
                });
                tail["from"] = json!(at);
                tail["dur"] = json!(end - at);
                tail["d"]["trim"] = prop(trim + offset * rate);
                l["dur"] = json!(offset);
                l["transitionOut"] = Value::Null;
                tail["transitionIn"] = Value::Null;
                if kind == "audio" {
                    l["d"]["fadeOut"] = prop(0.);
                    tail["d"]["fadeIn"] = prop(0.);
                }
                clip["tailId"] = tail["id"].clone();
                next["layers"][index] = l;
                next["layers"].as_array_mut().unwrap().insert(index, tail);
                return Ok(Plan {
                    body: media::body(p, &next)?,
                    clip,
                });
            }
            "speed" => {
                if kind != "video" {
                    return Err(Error::invalid(
                        "Playback speed is supported for video clips.",
                    ));
                }
                if value(&d["embeddedAudio"]) == &json!(true) {
                    return Err(Error::invalid(
                        "Separate embedded audio first; retimed embedded soundtracks are not supported.",
                    ));
                }
                let speed = n(a, "speed", 0.01)?;
                let duration = dur * rate / speed;
                if duration < 1. / fps || from + duration > comp_dur + 1e-6 {
                    return Err(Error::invalid(
                        "The retimed clip must fit the composition and retain at least one frame.",
                    ));
                }
                keys(&mut l, |k| {
                    k["t"] = json!(k["t"].as_f64().unwrap_or(0.) * rate / speed)
                });
                l["d"]["speed"] = prop(speed);
                l["dur"] = json!(duration);
            }
            "separate_audio" => {
                if kind != "video" || value(&d["embeddedAudio"]) != &json!(true) {
                    return Err(Error::invalid("This video has no enabled embedded audio."));
                }
                if rate != 1. {
                    return Err(Error::invalid(
                        "Separate audio before retiming; audio layers do not support playback speed.",
                    ));
                }
                let audio = new_layer(
                    p,
                    "audio",
                    &json!(format!("{} Audio", l["name"].as_str().unwrap_or(""))),
                    from,
                    dur,
                    json!({"asset":d["asset"],"trim":prop(trim)}),
                )?;
                l["d"]["embeddedAudio"] = json!(false);
                clip["audioId"] = audio["id"].clone();
                next["layers"][index] = l;
                next["layers"].as_array_mut().unwrap().insert(index, audio);
                return Ok(Plan {
                    body: media::body(p, &next)?,
                    clip,
                });
            }
            "audio" => {
                if kind != "audio" {
                    return Err(Error::invalid(
                        "Separate embedded audio before adjusting gain or fades.",
                    ));
                }
                let mut count = 0;
                for key in ["gain", "fadeIn", "fadeOut"] {
                    if a.get(key).is_some() {
                        let v = n(a, key, 0.)?;
                        if key == "gain" && v > 4. {
                            return Err(Error::invalid("Audio gain must be between 0 and 4."));
                        }
                        if key != "gain" && v > dur {
                            return Err(Error::invalid("Audio fades cannot exceed clip duration."));
                        }
                        if animated(&d[key]) {
                            return Err(Error::invalid(
                                "Use property/keyframe commands to edit animated audio controls.",
                            ));
                        }
                        l["d"][key] = prop(v);
                        count += 1;
                    }
                }
                if count == 0 {
                    return Err(Error::invalid("Supply gain, fadeIn, or fadeOut."));
                }
            }
            "remove" => {
                let mut scratch = DocumentStore::new_untitled(model(p));
                scratch
                    .execute(
                        LayerCommand::Delete {
                            layer_ids: vec![l["id"].as_str().unwrap().into()],
                        }
                        .into(),
                        Origin::Mcp,
                    )
                    .map_err(Error::kernel)?;
                next = json_value(&pm_model::canonical::realize_project(
                    scratch.snapshot().document.to_value().unwrap(),
                ));
                return Ok(Plan {
                    body: media::body(p, &next)?,
                    clip,
                });
            }
            _ => return Err(Error::invalid("Unknown video editing operation.")),
        }
        next["layers"][index] = l;
    }
    Ok(Plan {
        body: media::body(p, &next)?,
        clip,
    })
}
