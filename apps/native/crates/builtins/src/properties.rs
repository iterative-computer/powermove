//! Properties presentation using model-issued, selection-fenced leases.
use pm_kernel::commands::timing::{RetimeMode, TimingCommand};
use pm_kernel::{Caller, Request};
use pm_panel_inspector::{
    Binding, FieldType, Inspector, PropertyId,
    v4_properties::{V4Action, V4Adapter, V4Plan},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
#[derive(Default)]
pub struct Properties {
    adapter: V4Adapter,
    key: String,
    handles: BTreeMap<String, String>,
    /// What each font family offers for variation, read once (CoreText
    /// enumerates every installed face); Refresh Fonts forgets it.
    fonts: BTreeMap<String, pm_panel_inspector::content::FontFacts>,
    /// The axes the Type Settings rows were built from, by row id.
    axes: BTreeMap<String, pm_panel_inspector::content::FontAxis>,
    collapsed: BTreeMap<String, bool>,
    selection: Vec<String>,
    document: Option<u64>,
    sequence: u64,
    /// The viewer's layers left out of the frame, `{layer id: reason}`.
    pub failures: Value,
    /// Commands a row action asks the app to run (a sheet, another panel's
    /// view); the backend sends them on.
    pub(crate) commands: Vec<(String, Vec<Value>)>,
    /// Each listed row's label, for what a row action shows about it.
    labels: BTreeMap<String, String>,
    /// Imported files by asset ID, for a model's material slots (read once
    /// per asset into `slots`).
    pub originals: BTreeMap<String, std::path::PathBuf>,
    slots: BTreeMap<String, Vec<Value>>,
    /// The model part a viewer double-click focused, `(layer, slot)`, and
    /// the request number it came with (each request opens its section once).
    part_focus: Option<(String, u64)>,
    part_request: u64,
}
/// What Add Expression… starts from, as the shipped editor did.
pub const STARTING_EXPRESSION: &str = "value + wiggle(2, 20)";
impl Properties {
    /// A row's expression field, when one of its channels has an expression:
    /// the source, what the parser says about it, and a lease that writes
    /// every channel of the row.
    fn expression(
        &mut self,
        model: &Inspector,
        row: &str,
        bindings: Vec<Binding>,
        source: Option<String>,
    ) -> Result<Option<Value>, String> {
        let Some(source) = source else {
            return Ok(None);
        };
        let key = format!("expr.{row}");
        let handle = match self.handles.get(&key) {
            Some(handle) => handle.clone(),
            None => {
                let handle = self
                    .adapter
                    .register(model, bindings)
                    .map_err(|e| e.to_string())?;
                self.handles.insert(key.clone(), handle.clone());
                handle
            }
        };
        let error = pm_kernel::commands::properties::expression_diagnostic(&source);
        Ok(Some(
            json!({"binding":{"handle":handle,"channels":[key]},"source":source,"error":error}),
        ))
    }
    fn lease(&mut self, model: &Inspector, id: &str, binding: Binding) -> Result<Value, String> {
        let handle = if let Some(handle) = self.handles.get(id) {
            handle.clone()
        } else {
            let handle = self
                .adapter
                .register(model, vec![binding])
                .map_err(|e| e.to_string())?;
            self.handles.insert(id.into(), handle.clone());
            handle
        };
        Ok(json!({"handle":handle,"channels":[id]}))
    }
    /// The viewer's `focusPart` fact: a new request opens that slot's
    /// section and marks it; `null` leaves part focus.
    pub fn focus_part(&mut self, fact: &Value) {
        let (Some(layer), Some(slot), Some(request)) = (
            fact["layer"].as_str(),
            fact["slot"].as_u64(),
            fact["seq"].as_u64(),
        ) else {
            self.part_focus = None;
            return;
        };
        if request != self.part_request {
            self.part_request = request;
            self.part_focus = Some((layer.to_owned(), slot));
            self.collapsed.insert(format!("3d.part.{slot}"), false);
        }
    }
    pub fn snapshot(
        &mut self,
        model: &mut Inspector,
        time: f64,
        playing: bool,
    ) -> Result<Value, String> {
        // Leaving the model leaves its part (Electron's scene3d api.ts): a
        // selection that changes to anything but the model alone.
        if self.selection != model.selected_layers()
            && self
                .part_focus
                .as_ref()
                .is_some_and(|(layer, _)| model.selected_layers() != [layer.clone()])
        {
            self.part_focus = None;
        }
        if self.document != Some(model.document_id()) || self.selection != model.selected_layers() {
            self.adapter.retire();
            self.handles.clear();
            self.selection = model.selected_layers().to_vec();
            self.document = Some(model.document_id());
            self.sequence = self
                .sequence
                .checked_add(1)
                .ok_or("Properties identity exhausted")?;
            self.key = format!("properties:{}:{}", model.document_id(), self.sequence);
        }
        let target = if let Some(id) = model.primary() {
            let layer = model.layer(id).map_err(|e| e.to_string())?;
            let kind = layer
                .get("type")
                .and_then(pm_model::Value::as_str)
                .unwrap_or("");
            json!({"kind":"layers","key":self.key,"count":model.selected_layers().len(),"first":{"id":id,"name":layer.get("name").and_then(pm_model::Value::as_str).unwrap_or(""),"type":kind,"typeLabel":super::data::kind_label(kind),"locked":!model.editable(id),"audio":kind=="audio","acceptsEffects": !matches!(kind,"audio"|"camera")},"sharedType":if model.selected_layers().iter().all(|id|model.layer(id).ok().and_then(|l|l.get("type").and_then(pm_model::Value::as_str).map(str::to_owned)).as_deref()==Some(kind)) {Some(kind)} else {None}})
        } else {
            json!({"kind":"composition","key":self.key})
        };
        // A Studio 3D layer: its role, environment kind and the name it goes by.
        let studio = model
            .primary()
            .and_then(|id| model.layer(id).ok())
            .map(|l| super::data::wire(&l))
            .and_then(|r| {
                let (role, _) = pm_panel_studio3d::properties::fields(&r)?;
                let data = &r["d"]["data"];
                let env = pm_panel_studio3d::properties::environment(data);
                Some((role, env, studio_label(role, env, data)))
            });
        let mut target = target;
        if let Some((_, _, label)) = &studio
            && model.selected_layers().len() == 1
        {
            // `Terrain · Terrain` says nothing twice: a layer named for its
            // kind shows the generic 3D kind instead.
            let named = target["first"]["name"].as_str().unwrap_or("") == *label;
            target["first"]["typeLabel"] = json!(if named { "3D Layer" } else { label });
            target["first"]["acceptsEffects"] = json!(false);
            target["first"]["studio"] = json!(true);
        }
        if model.selected_layers().len() == 1
            && let Some(reason) = model
                .primary()
                .and_then(|id| self.failures.get(id))
                .and_then(Value::as_str)
        {
            // Failures are per layer: each layer reports only its own.
            target["first"]["error"] = json!(failure_copy(reason, studio.is_some()));
        }
        let mut sections = vec![];
        if let Some(layer) = model.primary().map(str::to_owned) {
            let audio = model
                .layer(&layer)
                .map_err(|e| e.to_string())?
                .get("type")
                .and_then(pm_model::Value::as_str)
                == Some("audio");
            let mut content = vec![];
            let mut layer_rows = vec![];
            let mut time_rows = vec![];
            // The model decides applicability; this adapter only maps authored
            // labels and supported control descriptors to frozen v4 rows.
            for field in model.fields().map_err(|e| e.to_string())? {
                let definition = model
                    .layer(&layer)
                    .ok()
                    .and_then(|l| l.get("d")?.get("definition")?.as_str().map(str::to_owned))
                    .unwrap_or_default();
                let contributed = match &field.binding {
                    Binding::Property(id) => id
                        .path
                        .strip_prefix("x.")
                        .and_then(|key| model.layer_param(&definition, key))
                        .cloned(),
                    _ => None,
                };
                let row_id = contributed.as_ref().map(|p| format!("content.x.{}", p.key));
                let (id, label, unit) = match &field.binding {
                    // A contributed layer type's parameter, labelled by its definition.
                    Binding::Property(_) if contributed.is_some() => (
                        row_id.as_deref().unwrap_or_default(),
                        contributed.as_ref().map_or("", |p| p.label.as_str()),
                        "",
                    ),
                    Binding::Content { field, .. } => match field.as_str() {
                        "text" => ("content.text", "Text", ""),
                        "font" => ("content.font", "Font", ""),
                        "weight" => ("content.weight", "Weight", ""),
                        "align" => ("content.align", "Align", ""),
                        "size" => ("content.size", "Size", "px"),
                        "tracking" => ("content.tracking", "Tracking", "px"),
                        "leading" => ("content.leading", "Leading", ""),
                        "color" => (
                            "content.color",
                            if audio || target["first"]["type"] == "text" {
                                "Color"
                            } else {
                                "Fill"
                            },
                            "",
                        ),
                        "w" => ("content.w", "Width", "px"),
                        "h" => ("content.h", "Height", "px"),
                        "shape" => ("content.shape", "Shape", ""),
                        "radius" => ("content.radius", "Corner Radius", "px"),
                        "radiusTL" => ("content.radiusTL", "Top Left", "px"),
                        "radiusTR" => ("content.radiusTR", "Top Right", "px"),
                        "radiusBL" => ("content.radiusBL", "Bottom Left", "px"),
                        "radiusBR" => ("content.radiusBR", "Bottom Right", "px"),
                        "smoothing" => ("content.smoothing", "Corner Smoothing", "%"),
                        "independentCorners" => {
                            ("content.independentCorners", "Independent Corners", "")
                        }
                        "stroke" => ("content.stroke", "Stroke", "px"),
                        "strokeColor" => ("content.strokeColor", "Stroke Color", ""),
                        "points" => ("content.points", "Points", ""),
                        "fit" => ("content.fit", "Fit", ""),
                        "trim" => ("content.trim", "Trim Start", "s"),
                        "speed" => ("content.speed", "Speed", ""),
                        "timeRemap" => ("content.timeRemap", "Time Remap", ""),
                        "sourceTime" => ("content.sourceTime", "Source Time", "s"),
                        "gain" => ("content.gain", "Gain", ""),
                        "fadeIn" => ("content.fadeIn", "Fade In", "s"),
                        "fadeOut" => ("content.fadeOut", "Fade Out", "s"),
                        "audioMuted" => ("content.audioMuted", "Mute Audio", ""),
                        "audioGain" => ("content.audioGain", "Gain", ""),
                        // DESIGN-TODO(opus): per-kind structured/contributed and
                        // variable-font-axis control details need design review.
                        // Retain authored data; no generic replacement control.
                        _ => continue,
                    },
                    Binding::Layer { field, .. } => match field.as_str() {
                        "parent" => ("layer.parent", "Parent", ""),
                        "matteSource" => ("layer.matteSource", "Track Matte", ""),
                        "threeD" => ("layer.threeD", "3D", ""),
                        "continuousRaster" => ("layer.continuousRaster", "Rasterize", ""),
                        "solo" => ("layer.solo", "Solo", ""),
                        "visible" => ("layer.visible", "Visible", ""),
                        "color" => ("layer.color", "Label Color", ""),
                        _ => continue,
                    },
                    Binding::Property(id) => match id.path.as_str() {
                        "l.blend" => ("layer.blend", "Blend Mode", ""),
                        "l.mblur" => ("layer.mblur", "Motion Blur", ""),
                        "l.matteMode" => ("layer.matteMode", "Matte Mode", ""),
                        _ => continue,
                    },
                    _ => continue,
                };
                let state = match model.binding_state(&field.binding, time) {
                    Ok(state) => state,
                    Err(pm_panel_inspector::InspectorError::MissingProperty(_)) => continue,
                    Err(error) => return Err(error.to_string()),
                };
                let lease = self.lease(model, id, field.binding.clone())?;
                let value = super::data::wire(&state.value);
                let control = match (&field.kind, &value) {
                    (FieldType::Number(spec), Value::Number(_)) => {
                        let mut axis = json!({"binding":lease,"value":value,"unit":unit,"step":spec.step,"precision":2,"mixed":state.mixed});
                        if let Some(min) = spec.min {
                            axis["min"] = json!(min);
                        }
                        if let Some(max) = spec.max {
                            axis["max"] = json!(max);
                        }
                        json!({"kind":"number","axes":[axis]})
                    }
                    (FieldType::Boolean, Value::Bool(_)) => {
                        json!({"kind":"toggle","binding":lease,"value":value,"mixed":state.mixed})
                    }
                    (FieldType::Text, Value::String(_)) => {
                        json!({"kind":"text","multiline":id=="content.text","binding":lease,"value":value,"mixed":state.mixed})
                    }
                    // The swatch and its hex beside it, both editable (v1).
                    (FieldType::Color, Value::String(_)) => {
                        let hex = self.lease(model, &format!("{id}#hex"), field.binding.clone())?;
                        json!({"kind":"colour","binding":lease,"value":value,"mixed":state.mixed,
                            "hexBinding":hex,"hex":value})
                    }
                    (FieldType::Font, Value::String(_)) => {
                        json!({"kind":"font","binding":lease,"value":value,"options":[],"mixed":state.mixed})
                    }
                    // Text alignment is three glyphs, not a menu of words (v1).
                    (FieldType::Choice, _) if id == "content.align" => json!({
                        "kind":"segmented","binding":lease,"value":value,"mixed":state.mixed,
                        "options":[
                            {"id":"left","label":"Align Left","symbol":"text.alignleft"},
                            {"id":"center","label":"Align Center","symbol":"text.aligncenter"},
                            {"id":"right","label":"Align Right","symbol":"text.alignright"},
                        ],
                    }),
                    (FieldType::Choice, _) => {
                        let options = match id {
                            "content.shape" => {
                                json!(["rect", "ellipse", "polygon", "star", "line"])
                            }
                            "content.fit" => json!(["cover", "contain", "stretch"]),
                            "content.weight" => {
                                let mut options = vec![
                                    json!({"v":100,"label":"Thin"}),
                                    json!({"v":200,"label":"Extra Light"}),
                                    json!({"v":300,"label":"Light"}),
                                    json!({"v":400,"label":"Regular"}),
                                    json!({"v":500,"label":"Medium"}),
                                    json!({"v":600,"label":"Semi Bold"}),
                                    json!({"v":700,"label":"Bold"}),
                                    json!({"v":800,"label":"Extra Bold"}),
                                    json!({"v":900,"label":"Black"}),
                                ];
                                if let Some(weight) = value.as_f64()
                                    && !options.iter().any(|o| o["v"] == value)
                                {
                                    options.push(json!({"v":weight,"label":weight.to_string()}));
                                    options.sort_by(|a, b| {
                                        a["v"]
                                            .as_f64()
                                            .unwrap()
                                            .total_cmp(&b["v"].as_f64().unwrap())
                                    });
                                }
                                json!(options)
                            }
                            _ if contributed.is_some() => json!(
                                contributed
                                    .iter()
                                    .flat_map(|p| p.options.iter().map(super::data::wire))
                                    .collect::<Vec<_>>()
                            ),
                            "layer.parent" | "layer.matteSource" => {
                                let ids = if id == "layer.parent" {
                                    model.parent_candidates(&layer)
                                } else {
                                    model.matte_candidates(&layer)
                                }
                                .map_err(|e| e.to_string())?;
                                let mut options = vec![json!({"v":null,"label":"None"})];
                                for id in ids {
                                    let candidate = model.layer(&id).map_err(|e| e.to_string())?;
                                    options.push(json!({"v":id,"label":candidate.get("name").and_then(pm_model::Value::as_str).unwrap_or(&id)}));
                                }
                                json!(options)
                            }
                            // The registry/spec owns choice labels and order.
                            // DESIGN-TODO(opus): supply blend/matte menu descriptors.
                            _ => continue,
                        };
                        json!({"kind":"choice","binding":lease,"value":value,"options":options,"mixed":state.mixed})
                    }
                    _ => continue,
                };
                let animation = if !field.animatable {
                    Value::Null
                } else {
                    json!(if state.key_at_playhead {
                        "key-at-playhead"
                    } else if state.animated {
                        "animated"
                    } else {
                        "static"
                    })
                };
                let mut row = json!({"id":id,"label":label,"locked":!model.editable(&layer),"animation":animation,"control":control});
                if field.animatable
                    && let Some(expression) = self.expression(
                        model,
                        id,
                        vec![field.binding.clone()],
                        state.expression.clone(),
                    )?
                {
                    row["expression"] = expression;
                }
                if id.starts_with("layer.") {
                    layer_rows.push(row);
                } else if matches!(id, "content.timeRemap" | "content.sourceTime") {
                    time_rows.push(row);
                } else if studio.is_none() {
                    // A 3D layer's canvas size is not something you edit.
                    content.push(row);
                }
            }
            if !content.is_empty() {
                sections.push(json!({"id":"content","title":"Content","collapsed":self.collapsed.get("content").copied().unwrap_or(false),"rows":content}));
            }
            // Time (`RetimingSection.svelte`): the three buttons, then the
            // remapping rows once a layer has a source time channel.
            if matches!(target["first"]["type"].as_str(), Some("video" | "precomp")) {
                let remapped = model
                    .layer(&layer)
                    .ok()
                    .is_some_and(|l| l.get("d").and_then(|d| d.get("sourceTime")).is_some());
                if !remapped {
                    time_rows.clear();
                }
                sections.push(json!({"id":"time","title":"Time","collapsed":self.collapsed.get("time").copied().unwrap_or(false),"rows":time_rows,"actions":["time-remap","freeze-frame","reverse"]}));
            }
            if let Some(section) = self.type_settings(model, &layer, time)? {
                sections.push(section);
            }
            if model.selected_layers().len() == 1 {
                sections.extend(self.caption_sections(model, &layer)?);
            }
            // One 3D section after content, single selection only.
            if model.selected_layers().len() == 1 {
                let record = super::data::wire(&model.layer(&layer).map_err(|e| e.to_string())?);
                if let Some((role, fields)) = pm_panel_studio3d::properties::fields(&record) {
                    sections
                        .extend(self.scene_sections(model, &layer, &record, role, fields, time)?);
                    sections.extend(self.part_sections(model, &layer, &record, time)?);
                } else if matches!(record["type"].as_str(), Some("text" | "shape")) {
                    sections.extend(self.extrude_sections(model, &layer, &record, time)?);
                }
            }
            if let Some((role, env, _)) = &studio {
                if *env != Some("world") {
                    let rows = self.transform_3d(model, &layer, *role, time)?;
                    if !rows.is_empty() {
                        sections.push(json!({"id":"transform","title":"Transform","collapsed":self.collapsed.get("transform").copied().unwrap_or(false),"rows":rows}));
                    }
                }
            } else if !audio {
                let mut rows = vec![];
                for (id, label, paths, unit, step) in [
                    (
                        "position",
                        "Position",
                        vec!["position.x", "position.y"],
                        "px",
                        1.,
                    ),
                    ("scale", "Scale", vec!["scale.x", "scale.y"], "%", 1.),
                    ("rotation", "Rotation", vec!["rotation"], "°", 1.),
                    ("skew", "Skew", vec!["skew"], "°", 1.),
                    (
                        "anchor",
                        "Anchor Point",
                        vec!["anchor.x", "anchor.y"],
                        "px",
                        1.,
                    ),
                    ("opacity", "Opacity", vec!["opacity"], "%", 1.),
                ] {
                    let mut axes = vec![];
                    let mut animation = "static";
                    let (mut bound, mut expression) = (vec![], None);
                    for (index, path) in paths.iter().enumerate() {
                        let binding = Binding::Property(PropertyId::new(&layer, *path));
                        let state = match model.binding_state(&binding, time) {
                            Ok(state) => state,
                            Err(pm_panel_inspector::InspectorError::MissingProperty(_)) => continue,
                            Err(error) => return Err(error.to_string()),
                        };
                        let value = state
                            .value
                            .as_f64()
                            .ok_or_else(|| format!("Non-numeric {path}"))?;
                        if state.key_at_playhead {
                            animation = "key-at-playhead";
                        } else if state.animated {
                            animation = "animated";
                        }
                        bound.push(binding.clone());
                        expression = expression.or_else(|| state.expression.clone());
                        let mut axis = json!({"binding":self.lease(model,path,binding)?,"value":value,"unit":unit,"step":step,"precision":2,"mixed":state.mixed,"animation": if state.key_at_playhead { "key-at-playhead" } else if state.animated { "animated" } else { "static" }});
                        if paths.len() > 1 {
                            axis["axis"] = json!(if index == 0 { "X" } else { "Y" });
                        }
                        if *path == "opacity" {
                            axis["min"] = json!(0);
                            axis["max"] = json!(100);
                        }
                        axes.push(axis);
                    }
                    if !axes.is_empty() {
                        let mut row = json!({"id":id,"label":label,"locked":!model.editable(&layer),"animation":animation,"control":{"kind":"number","axes":axes}});
                        if id == "scale" {
                            row["linked"] = json!(
                                model
                                    .layer(&layer)
                                    .map_err(|e| e.to_string())?
                                    .get("scaleLinked")
                                    .and_then(pm_model::Value::as_bool)
                                    .unwrap_or(true)
                            );
                        }
                        if let Some(expression) = self.expression(model, id, bound, expression)? {
                            row["expression"] = expression;
                        }
                        rows.push(row);
                    }
                }
                // A layer saved without transform channels has nothing to list.
                if !rows.is_empty() {
                    sections.push(json!({"id":"transform","title":"Transform","collapsed":self.collapsed.get("transform").copied().unwrap_or(false),"rows":rows}));
                }
            }
            if target["first"]["type"] == "text" {
                let animators = model
                    .text_animators(&layer)
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .map(|(id, name)| json!({"id":id,"name":name}))
                    .collect::<Vec<_>>();
                let presets = pm_panel_inspector::text_animators::preset_menu()
                    .into_iter()
                    .map(|(id, label)| json!({"id":id,"label":label,"disabled":id.is_none()}))
                    .collect::<Vec<_>>();
                sections.push(json!({"id":"animators","title":"Text Animators","collapsed":self.collapsed.get("animators").copied().unwrap_or(false),"rows":[],"action":"animator","animators":animators,"presets":presets}));
            }
            let kind = target["first"]["type"].as_str().unwrap_or("").to_owned();
            if !matches!(kind.as_str(), "audio" | "camera") && studio.is_none() {
                let masks = self.masks(model, &layer, time)?;
                sections.push(json!({"id":"masks","title":"Masks","collapsed":self.collapsed.get("masks").copied().unwrap_or(false),"rows":[],"action":"mask","masks":masks}));
            }
            if !layer_rows.is_empty() {
                let mut section = json!({"id":"layer","title":"Layer","collapsed":self.collapsed.get("layer").copied().unwrap_or(false),"rows":layer_rows});
                if target["first"]["type"] == "precomp" {
                    section["action"] = json!("release-precomp");
                }
                sections.push(section);
            }
        } else {
            let mut rows = vec![];
            for (id, label, unit, step) in [
                ("width", "Width", "px", 2.),
                ("height", "Height", "px", 2.),
                ("duration", "Duration", "s", 0.5),
                ("fps", "Frame Rate", "fps", 1.),
            ] {
                let binding = Binding::Composition(id.into());
                let state = model
                    .binding_state(&binding, time)
                    .map_err(|e| e.to_string())?;
                if let Some(value) = state.value.as_f64() {
                    rows.push(json!({"id":id,"label":label,"control":{"kind":"number","axes":[{"binding":self.lease(model,id,binding)?,"value":value,"unit":unit,"step":step}]}}));
                }
            }
            sections.push(
                json!({"id":"composition","title":"Composition","collapsed":false,"rows":rows}),
            );
            if let Some(section) = self.scene_section(model)? {
                sections.push(section);
            }
        }
        self.labels = sections
            .iter()
            .flat_map(|section| {
                let masks = section["masks"].as_array().into_iter().flatten();
                section["rows"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .chain(masks.flat_map(|m| m["rows"].as_array().into_iter().flatten()))
            })
            .filter_map(|row| {
                Some((
                    row["id"].as_str()?.to_owned(),
                    row["label"].as_str()?.to_owned(),
                ))
            })
            .collect();
        Ok(json!({"target":target,"sections":sections,"playing":playing,"time":time}))
    }
    /// Position, rotation and scale with depth, in scene units and degrees.
    /// Lights and cameras move but don't scale; a camera never needs X/Y tilt
    /// here because it aims at its target.
    fn transform_3d(
        &mut self,
        model: &mut Inspector,
        layer: &str,
        role: pm_panel_studio3d::properties::Role,
        time: f64,
    ) -> Result<Vec<Value>, String> {
        use pm_panel_studio3d::properties::Role;
        let mut rows = vec![];
        let mut groups: Vec<(&str, &str, Vec<(&str, &str)>, &str, f64)> = vec![(
            "position",
            "Position",
            vec![
                ("position.x", "X"),
                ("position.y", "Y"),
                ("position.z", "Z"),
            ],
            "",
            0.1,
        )];
        if role != Role::Camera {
            groups.push((
                "rotation3d",
                "Rotation",
                vec![("rotation.x", "X"), ("rotation.y", "Y"), ("rotation", "Z")],
                "°",
                1.,
            ));
        }
        if role == Role::Object {
            groups.push((
                "scale",
                "Scale %",
                vec![("scale.x", "X"), ("scale.y", "Y"), ("scale.z", "Z")],
                // Three axes leave no room for a unit; the label carries it.
                "",
                1.,
            ));
        }
        for (id, label, paths, unit, step) in groups {
            let mut axes = vec![];
            let mut animation = "static";
            for (path, axis) in paths {
                let binding = Binding::Property(PropertyId::new(layer, path));
                let state = match model.binding_state(&binding, time) {
                    Ok(state) => state,
                    Err(pm_panel_inspector::InspectorError::MissingProperty(_)) => continue,
                    Err(error) => return Err(error.to_string()),
                };
                let Some(value) = state.value.as_f64() else {
                    continue;
                };
                let a = if state.key_at_playhead {
                    "key-at-playhead"
                } else if state.animated {
                    "animated"
                } else {
                    "static"
                };
                if a != "static" && animation != "key-at-playhead" {
                    animation = a;
                }
                // Three narrow wells hold four characters: one place below
                // ten (-1.7), none above (-12). Typing still sets exact values.
                let shown = if value.abs() >= 10. {
                    value.round()
                } else {
                    (value * 10.).round() / 10.
                };
                axes.push(json!({"binding":self.lease(model,path,binding)?,"value":shown,"unit":unit,"step":step,"precision":2,"mixed":state.mixed,"animation":a,"axis":axis}));
            }
            if !axes.is_empty() {
                // Three axes fill the value column; no room for a link button.
                rows.push(json!({"id":id,"label":label,"locked":!model.editable(layer),"animation":animation,"control":{"kind":"number","axes":axes}}));
            }
        }
        Ok(rows)
    }
    /// The composition's 3D summary: its render quality and sky, so a scene
    /// looks right in a few clicks without selecting a camera or World.
    fn scene_section(&mut self, model: &mut Inspector) -> Result<Option<Value>, String> {
        use pm_panel_studio3d::properties::{QUALITY, SKIES, scene_summary};
        let Some(summary) = scene_summary(model.published()).map_err(|e| e.to_string())? else {
            // An empty composition says where 3D starts, with the one-click
            // lit set beside it.
            let empty = model
                .published()
                .snapshot
                .root()
                .and_then(|root| {
                    pm_kernel::model::comp_node(root, model.published().snapshot.open_comp().as_deref())
                        .and_then(|c| c.get("layers").map(|l| l.to_value()))
                })
                .is_some_and(|l| l.as_array().is_some_and(|a| a.is_empty()));
            if !empty {
                return Ok(None);
            }
            return Ok(Some(json!({"id":"3d.scene","title":"3D Scene","collapsed":false,"rows":[],
                "note":"Add a model, text or light from Layer ▸ Add 3D Layer, or start from a lit studio set.",
                "action":"set-up-studio"})));
        };
        let quality = summary["quality"]["preset"]
            .as_str()
            .unwrap_or("production");
        let mut options: Vec<Value> = QUALITY
            .iter()
            .map(|(v, label, detail)| json!({"v":v,"label":label,"detail":detail}))
            .collect();
        if quality == "custom" {
            options.push(json!({"v":"custom","label":"Custom","disabled":true}));
        }
        let lease = self.control_lease("3d.scene.quality");
        let mut rows = vec![
            json!({"id":"3d.scene.quality","label":"Render Quality","control":{"kind":"choice","binding":lease,"value":quality,"options":options}}),
        ];
        let q = &summary["quality"];
        let detail = format!(
            "{}–{} samples · {} bounces{}{}",
            q["minSamples"].as_f64().unwrap_or(0.),
            q["maxSamples"].as_f64().unwrap_or(0.),
            q["maxBounces"].as_f64().unwrap_or(0.),
            if q["caustics"] == true {
                " · caustics"
            } else {
                ""
            },
            if q["denoise"] == true {
                " · denoised"
            } else {
                ""
            },
        );

        let sky = summary["sky"].as_str().unwrap_or("none");
        let mut skies: Vec<Value> = vec![json!({"v":"none","label":"None"})];
        skies.extend(
            SKIES
                .iter()
                .map(|(v, label, detail)| json!({"v":v,"label":label,"detail":detail})),
        );
        if sky == "hdri" {
            skies.push(json!({"v":"hdri","label":"HDRI Image","disabled":true}));
        }
        let lease = self.control_lease("3d.scene.sky");
        rows.insert(0, json!({"id":"3d.scene.sky","label":"Sky","control":{"kind":"choice","binding":lease,"value":sky,"options":skies}}));
        // Add Shot (§8): a command pull-down, so it always shows its title.
        // Every row is explicitly unchecked: the title row is the control's
        // value, and a pop-up would otherwise mark it as a current choice.
        let mut shots = vec![json!({"v":"add","label":"Add Shot…","disabled":true,"checked":false})];
        shots.extend(
            pm_panel_studio3d::properties::shots()
                .as_array()
                .into_iter()
                .flatten()
                .map(|s| json!({"v":s["id"],"label":s["label"],"detail":s["detail"],"checked":false})),
        );
        let lease = self.control_lease("3d.scene.shot");
        rows.push(json!({"id":"3d.scene.shot","label":"Shot","control":{"kind":"choice","binding":lease,"value":"add","options":shots}}));
        Ok(Some(
            json!({"id":"3d.scene","title":"3D Scene","collapsed":self.collapsed.get("3d.scene").copied().unwrap_or(false),"rows":rows,"note":detail,"action":"set-up-studio"}),
        ))
    }
    /// A 3D control that isn't a channel (a pop-up or checkbox): its lease
    /// names the row, and `write` hands it to the Studio's typed edits.
    fn control_lease(&mut self, id: &str) -> Value {
        let handle = self
            .handles
            .entry(id.into())
            .or_insert_with(|| format!("{}#{id}", self.key))
            .clone();
        json!({"handle":handle,"channels":[id]})
    }
    /// An imported model's material slots (its parts): one section each,
    /// closed until opened, with the part's own values until it's edited.
    /// Edits are overrides in the project (`mi.<slot>.<key>`, keyframeable);
    /// the file is never touched.
    fn part_sections(
        &mut self,
        model: &mut Inspector,
        layer: &str,
        record: &Value,
        time: f64,
    ) -> Result<Vec<Value>, String> {
        let object = &record["d"]["data"]["object"];
        let Some(asset) = object["source"]["assetId"].as_str() else {
            return Ok(vec![]);
        };
        if object["useSourceMaterials"] != true {
            return Ok(vec![]);
        }
        if !self.slots.contains_key(asset) {
            let slots = self
                .originals
                .get(asset)
                .and_then(|path| Some((path.file_name()?.to_string_lossy().into_owned(), std::fs::read(path).ok()?)))
                .map(|(name, bytes)| pm_scene3d::import::material_slot_values_from_file(&name, &bytes))
                .unwrap_or_default();
            if self.originals.contains_key(asset) {
                self.slots.insert(asset.to_owned(), slots);
            }
        }
        let slots = self.slots.get(asset).cloned().unwrap_or_default();
        let locked = !model.editable(layer);
        let live = pm_panel_studio3d::extrude::sources(model.published(), layer)
            .map_err(|e| e.to_string())?;
        let mut sections = vec![];
        for slot in slots {
            let Some(index) = slot["index"].as_u64() else { continue };
            let title = slot["name"].as_str().unwrap_or("Part").to_owned();
            let stored = &object["sourceMaterialProperties"][index.to_string()];
            let mut rows = vec![];
            for (key, label, range, unit) in [
                ("color", "Color", None, ""),
                ("metallic", "Metalness", Some((0., 1., 0.01)), ""),
                ("roughness", "Roughness", Some((0., 1., 0.01)), ""),
                ("clearcoat", "Clearcoat", Some((0., 1., 0.01)), ""),
                ("transmission", "Transmission", Some((0., 1., 0.01)), ""),
                ("ior", "IOR", Some((1., 2.6, 0.01)), ""),
                ("emissionColor", "Emission", None, ""),
                ("emissionIntensity", "Emission Strength", Some((0., 100., 0.1)), ""),
                ("iridescence", "Iridescence", Some((0., 1200., 10.)), "nm"),
            ] {
                let path = format!("mi.{index}.{key}");
                if stored.get(key).is_some() {
                    let binding = Binding::Property(PropertyId::new(layer, &path));
                    if let Ok(state) = model.binding_state(&binding, time) {
                        let value = super::data::wire(&state.value);
                        let lease = self.lease(model, &path, binding)?;
                        let control = match (range, &value) {
                            (_, Value::String(_)) => json!({"kind":"colour","binding":lease,"value":value}),
                            (Some((min, max, step)), Value::Number(_)) => json!({"kind":"number","axes":[{"binding":lease,"value":value,"unit":unit,"step":step,"precision":2,"min":min,"max":max}]}),
                            _ => continue,
                        };
                        let animation = if state.key_at_playhead {
                            "key-at-playhead"
                        } else if state.animated {
                            "animated"
                        } else {
                            "static"
                        };
                        rows.push(json!({"id":path,"label":label,"locked":locked,"animation":animation,"control":control}));
                        continue;
                    }
                }
                // Not overridden yet: the part's own value; the first edit
                // creates the override.
                let id = format!("3d.part.{index}.{key}");
                let lease = self.control_lease(&id);
                let value = slot["values"][key].clone();
                let control = match range {
                    None => json!({"kind":"colour","binding":lease,"value":if value.is_string() { value } else { json!("#FFFFFF") }}),
                    Some((min, max, step)) => json!({"kind":"number","axes":[{"binding":lease,"value":if value.is_number() { value } else { json!(min) },"unit":unit,"step":step,"precision":2,"min":min,"max":max}]}),
                };
                rows.push(json!({"id":id,"label":label,"locked":locked,"animation":null,"control":control}));
            }
            // AE 27: a layer, group or composition as the part's material.
            let mut options = vec![json!({"v":null,"label":"None"})];
            options.extend(live.iter().cloned());
            let value = pm_panel_studio3d::extrude::source_value(
                &object["sourceMaterials"][index.to_string()]["color"],
            );
            let id = format!("3d.part.{index}.map.color");
            let lease = self.control_lease(&id);
            rows.push(json!({"id":id,"label":"Color Map","locked":locked,"animation":null,"control":{"kind":"choice","binding":lease,"value":value,"options":options}}));
            let id = format!("3d.part.{index}");
            let collapsed = self.collapsed.get(&id).copied().unwrap_or(true);
            let focused = self
                .part_focus
                .as_ref()
                .is_some_and(|(l, s)| l == layer && *s == index);
            sections.push(
                json!({"id":id,"title":title,"collapsed":collapsed,"focused":focused,"rows":rows}),
            );
        }
        Ok(sections)
    }
    /// Text and shape extrusion: Extrude (on, depth, bevel, detail), then
    /// Front, Bevel, Side and Back, each with its own look. A surface value
    /// that isn't stored yet shows what it starts from and is created on the
    /// first edit; once stored it is an ordinary keyframeable channel.
    fn extrude_sections(
        &mut self,
        model: &mut Inspector,
        layer: &str,
        record: &Value,
        time: f64,
    ) -> Result<Vec<Value>, String> {
        use pm_panel_studio3d::extrude;
        let locked = !model.editable(layer);
        let on = extrude::enabled(record);
        let Some(data) = extrude::data(record) else {
            return Ok(vec![]);
        };
        let row = |id: &str, label: &str, animation: Value, control: Value| json!({"id":id,"label":label,"locked":locked,"animation":animation,"control":control});
        // A stored channel: keyframeable, with its animation state.
        let channel = |this: &mut Self,
                       model: &mut Inspector,
                       path: &str,
                       label: &str,
                       unit: &str,
                       range: Option<(f64, f64, f64)>|
         -> Result<Option<Value>, String> {
            let binding = Binding::Property(PropertyId::new(layer, path));
            let state = match model.binding_state(&binding, time) {
                Ok(state) => state,
                Err(pm_panel_inspector::InspectorError::MissingProperty(_)) => return Ok(None),
                Err(error) => return Err(error.to_string()),
            };
            let value = super::data::wire(&state.value);
            let lease = this.lease(model, path, binding)?;
            let control = match (range, &value) {
                (_, Value::String(_)) => json!({"kind":"colour","binding":lease,"value":value}),
                (Some((min, max, step)), Value::Number(_)) => json!({"kind":"number","axes":[{"binding":lease,"value":value,"unit":unit,"step":step,"precision":2,"min":min,"max":max}]}),
                _ => return Ok(None),
            };
            let animation = if state.key_at_playhead {
                "key-at-playhead"
            } else if state.animated {
                "animated"
            } else {
                "static"
            };
            Ok(Some(row(path, label, json!(animation), control)))
        };
        let mut sections = vec![];
        let mut rows = vec![];
        let lease = self.control_lease("3d.extrude.enabled");
        rows.push(row(
            "3d.extrude.enabled",
            "Extrude",
            Value::Null,
            json!({"kind":"toggle","binding":lease,"value":on}),
        ));
        let bevel = data["p"]["bevelStyle"]["v"].as_str().unwrap_or("none").to_owned();
        if on {
            if let Some(r) = channel(self, model, "geo.depth", "Depth", "px", Some((0., 10000., 1.)))? {
                rows.push(r);
            }
            let lease = self.control_lease("3d.extrude.bevelStyle");
            let options: Vec<Value> = extrude::BEVELS
                .iter()
                .map(|(v, label)| json!({"v":v,"label":label}))
                .collect();
            rows.push(row(
                "3d.extrude.bevelStyle",
                "Bevel",
                Value::Null,
                json!({"kind":"choice","binding":lease,"value":bevel,"options":options}),
            ));
            if bevel != "none" {
                if let Some(r) = channel(self, model, "geo.bevelDepth", "Bevel Size", "px", Some((0., 1000., 0.5)))? {
                    rows.push(r);
                }
                if let Some(r) = channel(self, model, "geo.holeBevelDepth", "Inner Bevel", "%", Some((0., 100., 1.)))? {
                    rows.push(r);
                }
            }
            let lease = self.control_lease("3d.extrude.quality");
            let options: Vec<Value> = extrude::DETAIL
                .iter()
                .map(|(v, label, detail)| json!({"v":v,"label":label,"detail":detail}))
                .collect();
            rows.push(row(
                "3d.extrude.quality",
                "Detail",
                Value::Null,
                json!({"kind":"choice","binding":lease,"value":data["p"]["quality"]["v"],"options":options}),
            ));
        }
        let collapsed = self.collapsed.get("3d.extrude").copied().unwrap_or(false);
        let mut section = json!({"id":"3d.extrude","title":"Extrude","collapsed":collapsed,"rows":rows});
        if !on {
            section["note"] = json!("Give this layer depth, a bevel and its own surfaces. It lights like any 3D model.");
        }
        sections.push(section);
        if !on {
            return Ok(sections);
        }
        let sources = pm_panel_studio3d::extrude::sources(model.published(), layer)
            .map_err(|e| e.to_string())?;
        for (surface, title) in extrude::SURFACE_LABELS {
            if *surface == "bevel" && bevel == "none" {
                continue;
            }
            let mut rows = vec![];
            // Each surface reads like a model's material: Opacity in percent,
            // and Emission (colour) with its strength, as on 3D models.
            for (key, label, range, unit, precision) in [
                ("color", "Color", None, "", 2),
                ("roughness", "Roughness", Some((0., 1., 0.01)), "", 2),
                ("metallic", "Metalness", Some((0., 1., 0.01)), "", 2),
                ("opacity", "Opacity", Some((0., 100., 1.)), "%", 0),
                ("emissionColor", "Emission", None, "", 2),
                ("emissionIntensity", "Emission Strength", Some((0., 100., 0.1)), "", 1),
            ] {
                let path = format!("ms.{surface}.{key}");
                if extrude::has_override(record, surface, key)
                    && let Some(r) = channel(self, model, &path, label, unit, range)?
                {
                    rows.push(r);
                    continue;
                }
                // Not stored yet: what it starts from, created on first edit.
                let id = format!("3d.extrude.{surface}.{key}");
                let lease = self.control_lease(&id);
                let value = extrude::surface_value(record, surface, key);
                let control = match range {
                    None => json!({"kind":"colour","binding":lease,"value":value}),
                    Some((min, max, step)) => json!({"kind":"number","axes":[{"binding":lease,"value":value,"unit":unit,"step":step,"precision":precision,"min":min,"max":max}]}),
                };
                rows.push(row(&id, label, Value::Null, control));
            }
            // Color and Emission maps: a live layer, group or composition
            // can paint the surface or make it glow (AE 27 layers as materials).
            for (slot, label) in [("color", "Color Map"), ("emissive", "Emission Map")] {
                let map = &data["surfaces"][surface]["maps"][slot];
                let mut options = vec![json!({"v":null,"label":"None"})];
                options.extend(sources.iter().cloned());
                let value = extrude::source_value(map);
                let id = format!("3d.extrude.{surface}.map.{slot}");
                let lease = self.control_lease(&id);
                rows.push(row(
                    &id,
                    label,
                    Value::Null,
                    json!({"kind":"choice","binding":lease,"value":value,"options":options}),
                ));
            }
            let id = format!("3d.extrude.{surface}");
            let closed = *surface != "front";
            let collapsed = self.collapsed.get(&id).copied().unwrap_or(closed);
            sections.push(json!({"id":id,"title":title,"collapsed":collapsed,"rows":rows}));
        }
        Ok(sections)
    }
    /// 3d-studio.md §8: Model, Material and its groups, Light, Camera, Shot
    /// and Environment, in the inspector's row grammar.
    fn scene_sections(
        &mut self,
        model: &mut Inspector,
        layer: &str,
        record: &Value,
        role: pm_panel_studio3d::properties::Role,
        fields: Vec<pm_panel_studio3d::properties::Field>,
        time: f64,
    ) -> Result<Vec<Value>, String> {
        use pm_panel_studio3d::properties::Role;
        let data = &record["d"]["data"];
        let locked = !model.editable(layer);
        let mut groups: Vec<(&'static str, Vec<Value>)> = vec![];
        fn push(groups: &mut Vec<(&'static str, Vec<Value>)>, group: &'static str, row: Value) {
            match groups.iter_mut().find(|g| g.0 == group) {
                Some(g) => g.1.push(row),
                None => groups.push((group, vec![row])),
            }
        }
        let row = |id: &str, label: &str, control: Value| json!({"id":id,"label":label,"locked":locked,"animation":null,"control":control});
        let toggle = |this: &mut Self, id: &str, label: &str, value: bool| {
            let lease = this.control_lease(id);
            row(
                id,
                label,
                json!({"kind":"toggle","binding":lease,"value":value}),
            )
        };
        let env = pm_panel_studio3d::properties::environment(data);
        let mut shelf: Option<Value> = None;
        let choice = |this: &mut Self, id: &str, label: &str, value: Value, options: Value| {
            let lease = this.control_lease(id);
            row(
                id,
                label,
                json!({"kind":"choice","binding":lease,"value":value,"options":options}),
            )
        };
        let status = |id: &str, label: &str, value: String| json!({"id":id,"label":label,"control":{"kind":"status","value":value}});
        match (role, env) {
            (Role::Object, Some("world")) => {
                let world = &data["object"]["world"];
                let mode = world["mode"].as_str().unwrap_or("sky");
                push(
                    &mut groups,
                    "Sky",
                    choice(
                        self,
                        "3d.world.mode",
                        "Lighting",
                        json!(mode),
                        json!([{"v":"sky","label":"Physical Sky"},{"v":"hdri","label":"HDRI Image","disabled":world["source"].is_null()},{"v":"none","label":"None"}]),
                    ),
                );
                if mode == "sky" {
                    let skies = pm_panel_studio3d::properties::SKIES
                        .iter()
                        .map(|(v, label, detail)| json!({"v":v,"label":label,"detail":detail}))
                        .collect::<Vec<_>>();
                    push(
                        &mut groups,
                        "Sky",
                        choice(
                            self,
                            "3d.world.preset",
                            "Preset",
                            world["preset"].clone(),
                            json!(skies),
                        ),
                    );
                }
                let visible = world["p"]["visible"]["v"].as_bool().unwrap_or(true);
                push(
                    &mut groups,
                    "Sky",
                    toggle(self, "3d.world.visible", "Show in Camera", visible),
                );
            }
            (Role::Object, Some("fog")) => {
                let shape = data["object"]["fog"]["p"]["shape"]["v"].clone();
                push(
                    &mut groups,
                    "Fog",
                    choice(
                        self,
                        "3d.fog.shape",
                        "Shape",
                        if shape.is_null() { json!("box") } else { shape },
                        json!([{"v":"box","label":"Box"},{"v":"sphere","label":"Sphere"}]),
                    ),
                );
            }
            (Role::Object, Some("terrain")) => {
                let t = &data["object"]["terrain"];
                let flag = |k: &str| t["p"][k]["v"].as_bool().unwrap_or(false);
                push(
                    &mut groups,
                    "Terrain",
                    toggle(
                        self,
                        "3d.terrain.natural",
                        "Natural Ground",
                        flag("natural"),
                    ),
                );
                push(
                    &mut groups,
                    "Terrain",
                    toggle(self, "3d.terrain.ridged", "Ridged Peaks", flag("ridged")),
                );
                let regions = t["regions"].as_array().map_or(0, Vec::len);
                push(
                    &mut groups,
                    "Ground",
                    status(
                        "3d.terrain.regions",
                        "Regions",
                        match regions {
                            0 => "None, one ground material".into(),
                            1 => "1 region".into(),
                            n => format!("{n} regions"),
                        },
                    ),
                );
            }
            (Role::Object, Some(_)) => {
                let sc = &data["object"]["scatter"];
                // What is scattered and where, as pickers over the scene's
                // models (hidden scatter sources included) and surfaces.
                for (key, label) in [("model", "Object"), ("surface", "Surface")] {
                    let options = pm_panel_studio3d::properties::scatter_options(
                        model.published(),
                        layer,
                        key,
                    )
                    .map_err(|e| e.to_string())?;
                    push(
                        &mut groups,
                        "Scatter",
                        choice(
                            self,
                            &format!("3d.scatter.{key}"),
                            label,
                            sc[key].clone(),
                            json!(options),
                        ),
                    );
                }
            }
            (Role::Object, None) => {
                let object = &data["object"];
                let source = &object["source"];
                let name = if let Some(asset) = source["assetId"].as_str() {
                    model
                        .published()
                        .snapshot
                        .root()
                        .and_then(|r| {
                            r.get("assets")?
                                .get(asset)?
                                .get("name")?
                                .to_value()
                                .as_str()
                                .map(str::to_owned)
                        })
                        .unwrap_or_else(|| "Model".into())
                } else if let Some(primitive) = source["primitive"].as_str() {
                    match primitive {
                        "box" => "Cube",
                        "roundedBox" => "Rounded Cube",
                        "sphere" => "Sphere",
                        "plane" => "Plane",
                        "cylinder" => "Cylinder",
                        "cone" => "Cone",
                        "torus" => "Torus",
                        "capsule" => "Capsule",
                        "icosahedron" => "Icosphere",
                        _ => "Mesh",
                    }
                    .into()
                } else {
                    "Mesh".into()
                };
                let lease = self.control_lease("3d.source");
                push(
                    &mut groups,
                    "Model",
                    row(
                        "3d.source",
                        "Source",
                        json!({"kind":"text","binding":lease,"value":name,"disabled":true}),
                    ),
                );
                if source.get("assetId").is_some() {
                    let on = object["useSourceMaterials"] == true;
                    push(
                        &mut groups,
                        "Model",
                        toggle(self, "3d.useSourceMaterials", "Model Materials", on),
                    );
                }
                if object["useSourceMaterials"] != true {
                    let options = pm_panel_studio3d::properties::screens(model.published(), layer)
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .collect::<Vec<_>>();
                    // Studio finishes (3d-studio.md §8): a shelf of swatches
                    // under the Material heading, with no selected state.
                    let lease = self.control_lease("3d.finish");
                    push(
                        &mut groups,
                        "Material",
                        json!({"id":"3d.finish","label":"Finish","hidden":true,"locked":locked,"control":{"kind":"choice","binding":lease,"value":null,"options":[]}}),
                    );
                    shelf = Some(json!(
                        pm_panel_studio3d::properties::finishes()
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|f| {
                                let mut tile = finish_swatch(&f["p"]);
                                tile["id"] = f["id"].clone();
                                tile["label"] = f["label"].clone();
                                tile
                            })
                            .collect::<Vec<_>>()
                    ));
                    let lease = self.control_lease("3d.screen");
                    push(
                        &mut groups,
                        "Material",
                        row(
                            "3d.screen",
                            "Screen",
                            json!({"kind":"choice","binding":lease,"value":object["material"]["screen"],"options":options}),
                        ),
                    );
                    // Image maps: any imported image can drive a slot.
                    let mut images = vec![json!({"v":null,"label":"None"})];
                    if let Some(assets) = model
                        .published()
                        .snapshot
                        .root()
                        .and_then(|r| r.get("assets"))
                        .map(|a| super::data::wire(&a.to_value()))
                        && let Some(assets) = assets.as_object()
                    {
                        for (id, asset) in assets {
                            if asset["kind"] == "image" {
                                images.push(
                                    json!({"v":id,"label":asset["name"].as_str().unwrap_or(id)}),
                                );
                            }
                        }
                    }
                    let maps = &object["material"]["maps"];
                    // AE 27: a layer, group or composition can be the picture
                    // on a Color or Emissive map, live.
                    let live = pm_panel_studio3d::extrude::sources(model.published(), layer)
                        .map_err(|e| e.to_string())?;
                    for (slot, label) in [
                        ("color", "Color Map"),
                        ("normal", "Normal Map"),
                        ("roughness", "Roughness Map"),
                        ("metalness", "Metalness Map"),
                        ("emissive", "Emissive Map"),
                        ("ao", "Occlusion Map"),
                    ] {
                        let value = pm_panel_studio3d::extrude::source_value(&maps[slot]);
                        let mut options = images.clone();
                        if matches!(slot, "color" | "emissive") {
                            options.extend(live.iter().cloned());
                        }
                        if let Some(id) = value.as_str()
                            && !options.iter().any(|o| o["v"] == id)
                        {
                            options.push(json!({"v":id,"label":if id.contains(':') {"Missing source"} else {"Missing image"},"disabled":true}));
                        }
                        let mut r = choice(
                            self,
                            &format!("3d.texture.{slot}"),
                            label,
                            value,
                            json!(options),
                        );
                        if options.len() == 1 {
                            r["disabled"] = json!(true);
                        }
                        push(&mut groups, "Texture Maps", r);
                    }
                }
            }
            (Role::Light, _) => {
                let lease = self.control_lease("3d.type");
                let options = json!([{"v":"area","label":"Softbox"},{"v":"spot","label":"Spotlight"},{"v":"point","label":"Bulb"},{"v":"sun","label":"Sun"}]);
                push(
                    &mut groups,
                    "Light",
                    row(
                        "3d.type",
                        "Type",
                        json!({"kind":"choice","binding":lease,"value":data["light"]["type"],"options":options}),
                    ),
                );
            }
            (Role::Camera, _) => {
                let lease = self.control_lease("3d.projection");
                let options = json!([{"v":"perspective","label":"Perspective"},{"v":"orthographic","label":"Orthographic"}]);
                push(
                    &mut groups,
                    "Camera",
                    row(
                        "3d.projection",
                        "Projection",
                        json!({"kind":"choice","binding":lease,"value":data["camera"]["projection"],"options":options}),
                    ),
                );
            }
        }
        for field in fields {
            let binding = Binding::Property(PropertyId::new(layer, &field.id));
            let state = match model.binding_state(&binding, time) {
                Ok(state) => state,
                Err(pm_panel_inspector::InspectorError::MissingProperty(_)) => continue,
                Err(error) => return Err(error.to_string()),
            };
            let value = super::data::wire(&state.value);
            let lease = self.lease(model, &field.id, binding)?;
            let control = match (field.kind, &value) {
                ("color", Value::String(_)) => {
                    json!({"kind":"colour","binding":lease,"value":value})
                }
                ("number", Value::Number(_)) => {
                    let fine = field.max.is_some_and(|max| max <= 2.);
                    // A small value (fog density 0.002) shows its first two
                    // significant digits instead of reading "0".
                    let n = value.as_f64().unwrap_or(0.).abs();
                    let small = if n > 0. && n < 0.01 {
                        ((-n.log10()).ceil() as u32 + 1).min(6)
                    } else {
                        0
                    };
                    let floor = if field.min.is_some_and(|m| m > 0. && m < 0.01) { 3 } else { 2 };
                    let mut axis = json!({"binding":lease,"value":value,"unit":field.unit,"step":if fine {0.01} else {1.},"precision":floor.max(small)});
                    if let Some(min) = field.min {
                        axis["min"] = json!(min);
                    }
                    if let Some(max) = field.max {
                        axis["max"] = json!(max);
                    }
                    json!({"kind":"number","axes":[axis]})
                }
                _ => continue,
            };
            let animation = if state.key_at_playhead {
                "key-at-playhead"
            } else if state.animated {
                "animated"
            } else {
                "static"
            };
            push(
                &mut groups,
                field.group,
                json!({"id":field.id,"label":field.label,"locked":locked,"animation":animation,"control":control}),
            );
        }
        match role {
            Role::Object if env.is_some() => {}
            Role::Object => {
                let object = &data["object"];
                // AE 27 Material Options. The older per-model shadow switches
                // only appear while one is off, so it can be turned back on.
                for (key, label) in [
                    ("castShadow", "Cast Shadows"),
                    ("receiveShadow", "Receive Shadows"),
                ] {
                    if object[key] == false {
                        push(
                            &mut groups,
                            "Model",
                            toggle(self, &format!("3d.{key}"), label, false),
                        );
                    }
                }
                let o = &object["materialOptions"]["p"];
                let casts = o["castsShadows"]["v"].as_str().unwrap_or("on").to_owned();
                push(
                    &mut groups,
                    "Material Options",
                    choice(
                        self,
                        "3d.options.castsShadows",
                        "Casts Shadows",
                        json!(casts),
                        json!([{"v":"on","label":"On"},{"v":"off","label":"Off"},{"v":"only","label":"Only","detail":"Casts shadows but stays invisible."}]),
                    ),
                );
                for (key, label) in [
                    ("acceptsShadows", "Accepts Shadows"),
                    ("acceptsLights", "Accepts Lights"),
                    ("appearsInReflections", "Appears in Reflections"),
                ] {
                    let on = o[key]["v"].as_bool().unwrap_or(true);
                    push(
                        &mut groups,
                        "Material Options",
                        toggle(self, &format!("3d.options.{key}"), label, on),
                    );
                }
            }
            Role::Light => {
                let on = data["light"]["castShadow"] != false;
                push(
                    &mut groups,
                    "Light",
                    toggle(self, "3d.castShadow", "Cast Shadows", on),
                );
            }
            Role::Camera => {
                let environment = &data["environment"];
                // Samples per frame now come from the composition's Render
                // Quality; the camera shows that same control, not a second one.
                let preset = pm_panel_studio3d::properties::scene_summary(model.published())
                    .ok()
                    .flatten()
                    .and_then(|s| s["quality"]["preset"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| "production".into());
                let mut options: Vec<Value> = pm_panel_studio3d::properties::QUALITY
                    .iter()
                    .map(|(v, label, detail)| json!({"v":v,"label":label,"detail":detail}))
                    .collect();
                if preset == "custom" {
                    options.push(json!({"v":"custom","label":"Custom","disabled":true}));
                }
                let lease = self.control_lease("3d.scene.quality");
                push(
                    &mut groups,
                    "Shot",
                    row(
                        "3d.scene.quality",
                        "Render Quality",
                        json!({"kind":"choice","binding":lease,"value":preset,"options":options}),
                    ),
                );
                push(
                    &mut groups,
                    "Environment",
                    toggle(
                        self,
                        "3d.shadows",
                        "Shadows",
                        environment["shadows"] != false,
                    ),
                );
                let background = environment["background"].as_str();
                push(
                    &mut groups,
                    "Environment",
                    toggle(self, "3d.background", "Background", background.is_some()),
                );
                if let Some(colour) = background {
                    let lease = self.control_lease("3d.background.color");
                    push(
                        &mut groups,
                        "Environment",
                        row(
                            "3d.background.color",
                            "Background Color",
                            json!({"kind":"colour","binding":lease,"value":colour}),
                        ),
                    );
                }
            }
        }
        // Maps and options read after the material's own looks.
        groups.sort_by_key(|(title, _)| match *title {
            "Texture Maps" => 1,
            "Material Options" => 2,
            _ => 0,
        });
        Ok(groups
            .into_iter()
            .map(|(title, rows)| {
                let id = format!("3d.{}", title.to_ascii_lowercase().replace(' ', "-"));
                // Disclosure groups start closed unless a row inside is animated.
                let closed = matches!(
                    title,
                    "Coat and Glass"
                        | "Surface"
                        | "Emission"
                        | "Material Options"
                        | "Texture Maps"
                        | "Atmosphere"
                        | "Size"
                        | "Ground"
                        | "Variation"
                        | "Placement"
                ) && rows
                    .iter()
                    .all(|r| r["animation"] != "animated" && r["animation"] != "key-at-playhead");
                let collapsed = self.collapsed.get(&id).copied().unwrap_or(closed);
                let mut section = json!({"id":id,"title":title,"collapsed":collapsed,"rows":rows});
                if title == "Material"
                    && let Some(shelf) = &shelf
                {
                    section["finishes"] = shelf.clone();
                }
                section
            })
            .collect())
    }
    /// Each mask's rows (MasksSection.svelte): Enabled, Shape, Mode and its
    /// animatable box and feather.
    fn masks(
        &mut self,
        model: &mut Inspector,
        layer: &str,
        time: f64,
    ) -> Result<Vec<Value>, String> {
        let stored = model.layer(layer).map_err(|e| e.to_string())?;
        let masks = stored
            .get("masks")
            .and_then(pm_model::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let locked = !model.editable(layer);
        let mut out = vec![];
        for (index, mask) in masks.iter().enumerate() {
            let Some(mid) = mask.get("id").and_then(pm_model::Value::as_str) else {
                continue;
            };
            let mut rows = vec![];
            for (key, label, unit, step) in [
                ("on", "Enabled", "", 0.),
                ("shape", "Shape", "", 0.),
                ("mode", "Mode", "", 0.),
                ("x", "X", "px", 1.),
                ("y", "Y", "px", 1.),
                ("w", "Width", "px", 1.),
                ("h", "Height", "px", 1.),
                ("rotation", "Rotation", "°", 1.),
                ("feather", "Feather", "px", 0.5),
            ] {
                let path = format!("m.{mid}.{key}");
                let binding = Binding::Property(PropertyId::new(layer, &path));
                let state = match model.binding_state(&binding, time) {
                    Ok(state) => state,
                    Err(pm_panel_inspector::InspectorError::MissingProperty(_)) => continue,
                    Err(error) => return Err(error.to_string()),
                };
                let lease = self.lease(model, &path, binding)?;
                let value = super::data::wire(&state.value);
                let control = match (key, &value) {
                    ("shape", Value::String(_)) => {
                        json!({"kind":"choice","binding":lease,"value":value,"options":["rect","ellipse"],"mixed":state.mixed})
                    }
                    ("mode", Value::String(_)) => {
                        json!({"kind":"choice","binding":lease,"value":value,"options":["add","subtract"],"mixed":state.mixed})
                    }
                    (_, Value::Bool(_)) => {
                        json!({"kind":"toggle","binding":lease,"value":value,"mixed":state.mixed})
                    }
                    (_, Value::Number(_)) => {
                        let mut axis = json!({"binding":lease,"value":value,"unit":unit,"step":step,"precision":2,"mixed":state.mixed});
                        if key == "feather" {
                            axis["min"] = json!(0);
                        }
                        json!({"kind":"number","axes":[axis]})
                    }
                    _ => continue,
                };
                let animation = if state.key_at_playhead {
                    "key-at-playhead"
                } else if state.animated {
                    "animated"
                } else {
                    "static"
                };
                rows.push(json!({"id":path,"label":label,"locked":locked,"animation":animation,"control":control}));
            }
            let name = mask
                .get("name")
                .and_then(pm_model::Value::as_str)
                .filter(|n| !n.trim().is_empty())
                .map_or_else(|| format!("Mask {}", index + 1), str::to_owned);
            out.push(json!({"id":mid,"name":name,"rows":rows}));
        }
        Ok(out)
    }
    /// The layers a row's handle edits.
    pub fn layers_of(&self, handle: &str) -> Option<std::sync::Arc<[String]>> {
        self.adapter.selection(handle)
    }
    pub fn valid(&self, handle: &str, channels: &[String]) -> bool {
        self.handles
            .iter()
            .any(|(path, h)| h == handle && channels == [path.clone()])
    }
    /// Type Settings (v1): one row per variation axis of the text layer's
    /// font, plus axes the layer saved from a font this Mac doesn't have. A
    /// static font shows nothing; a missing one says so and offers Refresh Fonts.
    /// S088: a captions layer's style in the inspector's row grammar, each
    /// change one `Caption Style` step through `edit_captions` (op style).
    fn caption_sections(&mut self, model: &Inspector, layer: &str) -> Result<Vec<Value>, String> {
        let record = super::data::wire(&model.layer(layer).map_err(|e| e.to_string())?);
        if record["type"] != "captions" {
            return Ok(vec![]);
        }
        let style =
            serde_json::to_value(pm_captions::normalize_caption_style(&record["d"]["style"]))
                .unwrap_or_default();
        let locked = !model.editable(layer);
        let mut groups: Vec<(&str, &str, Vec<Value>)> = vec![
            ("captions", "Captions", vec![]),
            ("captions-background", "Background", vec![]),
            ("captions-placement", "Placement", vec![]),
            ("captions-words", "Words", vec![]),
        ];
        let row = |this: &mut Self, key: &str, label: &str, control: Value| {
            let mut control = control;
            control["binding"] = this.control_lease(&format!("captions.{key}"));
            if control.get("value").is_none() {
                control["value"] = style[key].clone();
            }
            control["mixed"] = json!(false);
            json!({"id":format!("captions.{key}"),"label":label,"locked":locked,"animation":null,"control":control})
        };
        let number = |this: &mut Self,
                      key: &str,
                      unit: &str,
                      min: f64,
                      max: Option<f64>,
                      step: f64| {
            let mut axis = json!({"binding":this.control_lease(&format!("captions.{key}")),"value":style[key],
                "unit":unit,"step":step,"precision":2,"mixed":false,"min":min});
            if let Some(max) = max {
                axis["max"] = json!(max);
            }
            json!({"kind":"number","axes":[axis]})
        };
        let choice = |pairs: &[(&str, &str)]| json!({"kind":"choice","options":pairs.iter().map(|(v, l)| json!({"v":v,"label":l})).collect::<Vec<_>>()});
        let mut presets: Vec<(String, String)> = pm_captions::caption_presets()
            .iter()
            .map(|p| (p.id.clone(), p.label.clone()))
            .collect();
        if style["preset"] == "custom" {
            presets.push(("custom".into(), "Custom".into()));
        }
        let presets: Vec<(&str, &str)> = presets
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        groups[0]
            .2
            .push(row(self, "preset", "Style", choice(&presets)));
        groups[0].2.push(row(
            self,
            "font",
            "Font",
            json!({"kind":"font","options":[]}),
        ));
        let mut weights: Vec<Value> = [
            (300, "Light"),
            (400, "Regular"),
            (500, "Medium"),
            (600, "Semibold"),
            (700, "Bold"),
            (800, "Heavy"),
            (900, "Black"),
        ]
        .iter()
        .map(|(v, l)| json!({"v":v,"label":l}))
        .collect();
        if !weights
            .iter()
            .any(|o| o["v"].as_f64() == style["weight"].as_f64())
        {
            weights.push(json!({"v":style["weight"],"label":style["weight"].to_string()}));
        }
        groups[0].2.push(row(
            self,
            "weight",
            "Weight",
            json!({"kind":"choice","options":weights}),
        ));
        let size = number(self, "size", "px", 4., None, 1.);
        groups[0].2.push(row(self, "size", "Size", size));
        groups[0]
            .2
            .push(row(self, "fill", "Color", json!({"kind":"colour"})));
        let stroke = number(self, "stroke", "px", 0., None, 0.5);
        groups[0].2.push(row(self, "stroke", "Outline", stroke));
        if style["stroke"].as_f64().is_some_and(|s| s > 0.) {
            groups[0].2.push(row(
                self,
                "strokeColor",
                "Outline Color",
                json!({"kind":"colour"}),
            ));
        }
        groups[0].2.push(row(
            self,
            "textCase",
            "Case",
            choice(&[
                ("original", "As Typed"),
                ("upper", "Uppercase"),
                ("lower", "Lowercase"),
            ]),
        ));
        groups[1]
            .2
            .push(row(self, "box", "Background", json!({"kind":"toggle"})));
        if style["box"] == true {
            groups[1]
                .2
                .push(row(self, "boxColor", "Color", json!({"kind":"colour"})));
            let opacity = number(self, "boxOpacity", "%", 0., Some(100.), 1.);
            groups[1]
                .2
                .push(row(self, "boxOpacity", "Opacity", opacity));
            let padding = number(self, "boxPadding", "px", 0., None, 1.);
            groups[1]
                .2
                .push(row(self, "boxPadding", "Padding", padding));
            let radius = number(self, "boxRadius", "px", 0., None, 1.);
            groups[1]
                .2
                .push(row(self, "boxRadius", "Corner Radius", radius));
        }
        groups[2].2.push(row(
            self,
            "placement",
            "Position",
            choice(&[("bottom", "Bottom"), ("middle", "Middle"), ("top", "Top")]),
        ));
        groups[2].2.push(row(
            self,
            "align",
            "Align",
            json!({"kind":"segmented","options":[
            {"id":"left","label":"Align Left","symbol":"text.alignleft"},
            {"id":"center","label":"Align Center","symbol":"text.aligncenter"},
            {"id":"right","label":"Align Right","symbol":"text.alignright"}]}),
        ));
        let x = self.control_lease("captions.offsetX");
        let y = self.control_lease("captions.offsetY");
        groups[2].2.push(json!({"id":"captions.offset","label":"Offset","locked":locked,"animation":null,
            "control":{"kind":"number","axes":[
                {"binding":x,"value":style["offsetX"],"unit":"px","step":1,"precision":2,"mixed":false,"label":"Horizontal offset"},
                {"binding":y,"value":style["offsetY"],"unit":"px","step":1,"precision":2,"mixed":false,"label":"Vertical offset"}]}}));
        let margin = number(self, "safeMargin", "%", 0., Some(45.), 0.5);
        groups[2]
            .2
            .push(row(self, "safeMargin", "Safe Margin", margin));
        let width = number(self, "maxWidth", "%", 10., Some(100.), 1.);
        groups[2].2.push(row(self, "maxWidth", "Max Width", width));
        let lines = number(self, "maxLines", "", 1., Some(6.), 1.);
        groups[2].2.push(row(self, "maxLines", "Max Lines", lines));
        groups[3].2.push(row(
            self,
            "highlight",
            "Highlight",
            json!({"kind":"toggle"}),
        ));
        if style["highlight"] == true {
            groups[3].2.push(row(
                self,
                "highlightColor",
                "Color",
                json!({"kind":"colour"}),
            ));
            let scale = number(self, "highlightScale", "%", 50., Some(300.), 1.);
            groups[3].2.push(row(self, "highlightScale", "Size", scale));
        }
        groups[3].2.push(row(
            self,
            "wordAnimation",
            "Animation",
            choice(&[("none", "None"), ("pop", "Pop In"), ("fade", "Fade In")]),
        ));
        Ok(groups
            .into_iter()
            .filter(|(_, _, rows)| !rows.is_empty())
            .map(|(id, title, rows)| {
                json!({"id":id,"title":title,
                "collapsed":self.collapsed.get(id).copied().unwrap_or(false),"rows":rows})
            })
            .collect())
    }
    fn type_settings(
        &mut self,
        model: &mut Inspector,
        layer: &str,
        time: f64,
    ) -> Result<Option<Value>, String> {
        let source = model.layer(layer).map_err(|e| e.to_string())?;
        if source.get("type").and_then(pm_model::Value::as_str) != Some("text") {
            return Ok(None);
        }
        let family = source
            .get("d")
            .and_then(|d| d.get("font"))
            .and_then(pm_model::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let facts = self
            .fonts
            .entry(family.clone())
            .or_insert_with(|| font_facts(&family))
            .clone();
        let axes = model
            .font_axes(layer, &facts, time)
            .map_err(|e| e.to_string())?;
        if axes.is_empty() && facts.readable {
            return Ok(None);
        }
        self.axes.clear();
        let mut rows = vec![];
        for state in axes {
            let id = format!("fontAxis.{}", state.axis.tag);
            let lease = self.lease(
                model,
                &id,
                Binding::Property(PropertyId::new(layer, format!("c.{id}"))),
            )?;
            let animation = match model.binding_state(
                &Binding::Property(PropertyId::new(layer, format!("c.{id}"))),
                time,
            ) {
                Ok(s) if s.key_at_playhead => "key-at-playhead",
                Ok(s) if s.animated => "animated",
                _ => "static",
            };
            let mut axis = json!({"binding":lease,"value":super::data::wire(&state.value),"step":1,"precision":2,"mixed":false});
            if let Some(min) = state.axis.min {
                axis["min"] = json!(min);
            }
            if let Some(max) = state.axis.max {
                axis["max"] = json!(max);
            }
            rows.push(json!({"id":id,"label":axis_label(&state.axis.tag),"locked":!model.editable(layer),"animation":animation,"control":{"kind":"number","axes":[axis]}}));
            self.axes.insert(id, state.axis);
        }
        if !facts.readable {
            rows.push(json!({"id":"fontAxis.status","label":"Font","locked":false,"disabled":true,"animation":null,
                "control":{"kind":"status","value":"Not installed"}}));
        }
        let mut section = json!({"id":"type","title":"Type Settings","collapsed":self.collapsed.get("type").copied().unwrap_or(false),"rows":rows});
        if !facts.readable {
            section["action"] = json!("refresh-fonts");
        }
        Ok(Some(section))
    }
    pub fn write(
        &self,
        model: &mut Inspector,
        caller: Caller,
        handle: &str,
        value: pm_model::Value,
        time: f64,
    ) -> Result<Request, String> {
        // A variation axis may not be stored yet: set_axis creates it first.
        if let Some((id, _)) = self
            .handles
            .iter()
            .find(|(id, h)| *h == handle && id.starts_with("fontAxis."))
            && let Some(axis) = self.axes.get(id)
        {
            let layer = model.primary().ok_or("No selected layer")?.to_owned();
            return model
                .set_axis(
                    caller,
                    &layer,
                    axis,
                    value.as_f64().ok_or("Expected a number")?,
                    time,
                    false,
                )
                .map_err(|e| e.to_string());
        }
        if self
            .handles
            .get("content.independentCorners")
            .is_some_and(|h| h == handle)
        {
            let layer = model.primary().ok_or("No selected layer")?.to_owned();
            // Verify the selection/document lease before the multi-field command.
            self.adapter
                .read(model, handle, time)
                .map_err(|e| e.to_string())?;
            return model
                .independent_corners(
                    caller,
                    &layer,
                    value.as_bool().ok_or("Expected toggle")?,
                    time,
                )
                .map_err(|e| e.to_string());
        }
        if let Some(key) = self
            .handles
            .iter()
            .find(|(_, h)| *h == handle)
            .and_then(|(path, _)| path.strip_prefix("captions."))
        {
            let layer_id = model.primary().ok_or("No selected layer")?.to_owned();
            let published = model.published().clone();
            if model.selected_layers().len() != 1
                || published.selection.layers() != model.selected_layers()
            {
                return Err("The selection changed".into());
            }
            let value = super::data::wire(&value);
            let patch = pm_model::json::parse(&json!({ key: value }).to_string())
                .map_err(|e| e.to_string())?;
            let edit = pm_kernel::commands::captions::CaptionEdit::Style {
                style: patch,
                language: None,
            };
            return Ok(Request::new(
                caller,
                pm_kernel::Call::Transact {
                    label: Some("Caption Style".into()),
                    expected_revision: Some(model.revision()),
                    body: pm_kernel::WriteBody::Commands(vec![
                        pm_kernel::CaptionCommand::Edit { layer_id, edit }.into(),
                    ]),
                },
            ));
        }
        if self
            .handles
            .iter()
            .any(|(path, h)| h == handle && path.starts_with("expr."))
        {
            // The field holds the whole source; cleared, the expression goes.
            let source = value.as_str().ok_or("Expected text")?.trim().to_owned();
            return match self
                .adapter
                .action(
                    model,
                    caller,
                    handle,
                    V4Action::Expression((!source.is_empty()).then_some(source)),
                    time,
                )
                .map_err(|e| e.to_string())?
            {
                V4Plan::Request(request) => Ok(*request),
                V4Plan::Service(_) => Err("Expected an edit".into()),
            };
        }
        if let Some(control) = self
            .handles
            .iter()
            .find(|(_, h)| *h == handle)
            .and_then(|(path, _)| path.strip_prefix("3d."))
        {
            let published = model.published().clone();
            if (!control.starts_with("scene.") && model.selected_layers().len() != 1)
                || published.selection.layers() != model.selected_layers()
            {
                return Err("The selection changed".into());
            }
            let (label, commands) = pm_panel_studio3d::properties::command(
                &published,
                time,
                control,
                super::data::wire(&value),
            )
            .map_err(|e| e.to_string())?;
            return Ok(Request::new(
                caller,
                pm_kernel::Call::Transact {
                    label: Some(label),
                    expected_revision: Some(model.revision()),
                    body: pm_kernel::WriteBody::Commands(commands),
                },
            ));
        }
        self.adapter
            .set(model, caller, handle, &[value], time)
            .map_err(|e| e.to_string())
    }
    pub fn intent(
        &mut self,
        model: &mut Inspector,
        caller: Caller,
        intent: &Value,
        time: f64,
    ) -> Result<Option<Request>, String> {
        if intent["type"] == "drop-effect" {
            let target = intent["target"].as_str().ok_or("Effect target")?;
            if model.primary() != Some(target) {
                return Err("Effect target selection changed".into());
            }
            let definition = intent["effect"].as_str().ok_or("Effect definition")?;
            return model
                .add_effect_at(caller, target, definition, time)
                .map(Some)
                .map_err(|e| e.to_string());
        }
        if intent["type"] == "add-animator" || intent["type"] == "remove-animator" {
            let layer = model.primary().ok_or("No selected layer")?.to_owned();
            let request = if intent["type"] == "add-animator" {
                let size = model
                    .binding_state(
                        &Binding::Content {
                            layer: layer.clone(),
                            field: "size".into(),
                        },
                        time,
                    )
                    .ok()
                    .and_then(|state| state.value.as_f64())
                    .unwrap_or(0.);
                let preset = intent["preset"].as_str().ok_or("Animator preset")?;
                model.add_text_animator(caller, &layer, preset, size)
            } else {
                let id = intent["animator"].as_str().ok_or("Text animator")?;
                model.remove_text_animator(caller, &layer, id)
            };
            return request.map(Some).map_err(|e| e.to_string());
        }
        if (intent["type"] == "section-action" && intent["action"] == "mask")
            || intent["type"] == "remove-mask"
        {
            use pm_kernel::commands::masks::{MaskCommand, MaskShape};
            let layer_id = model.primary().ok_or("No selected layer")?.to_owned();
            let (label, command) = if intent["type"] == "remove-mask" {
                let mask = intent["mask"].as_str().ok_or("Mask")?;
                (
                    "Remove Mask",
                    MaskCommand::Remove {
                        layer_id,
                        mask_ids: vec![mask.into()],
                    },
                )
            } else {
                (
                    "Add Mask",
                    MaskCommand::Add {
                        layer_id,
                        mask_id: pm_kernel::new_id("K"),
                        shape: MaskShape::Rect,
                    },
                )
            };
            return Ok(Some(Request::new(
                caller,
                pm_kernel::Call::Transact {
                    label: Some(label.into()),
                    expected_revision: Some(model.revision()),
                    body: pm_kernel::WriteBody::Commands(vec![command.into()]),
                },
            )));
        }
        if intent["type"] == "section-action" && intent["action"] == "set-up-studio" {
            let (label, commands) =
                pm_panel_studio3d::properties::set_up_studio(model.published(), time)
                    .map_err(|e| e.to_string())?;
            return Ok(Some(Request::new(
                caller,
                pm_kernel::Call::Transact {
                    label: Some(label),
                    expected_revision: Some(model.revision()),
                    body: pm_kernel::WriteBody::Commands(commands),
                },
            )));
        }
        if intent["type"] == "section-action"
            && let Some((mode, label)) = match intent["action"].as_str() {
                Some("time-remap") => Some((RetimeMode::Remap, "Enable Time Remapping")),
                Some("freeze-frame") => Some((RetimeMode::Freeze, "Freeze Frame")),
                Some("reverse") => Some((RetimeMode::Reverse, "Time-Reverse Layer")),
                _ => None,
            }
        {
            // The selected video and pre-comp layers, as the Layer menu's rows.
            let layer_ids: Vec<String> = model
                .selected_layers()
                .iter()
                .filter(|id| {
                    model.editable(id)
                        && model.layer(id).ok().is_some_and(|l| {
                            matches!(
                                l.get("type").and_then(pm_model::Value::as_str),
                                Some("video" | "precomp")
                            )
                        })
                })
                .cloned()
                .collect();
            if layer_ids.is_empty() {
                return Err("Select a video or pre-comp layer".into());
            }
            return Ok(Some(Request::new(
                caller,
                pm_kernel::Call::Transact {
                    label: Some(label.into()),
                    expected_revision: Some(model.revision()),
                    body: pm_kernel::WriteBody::Commands(vec![
                        TimingCommand::Retime {
                            layer_ids,
                            mode,
                            time,
                        }
                        .into(),
                    ]),
                },
            )));
        }
        if intent["type"] == "section-action" && intent["action"] == "refresh-fonts" {
            // Fonts installed since are read again on the next publication.
            self.fonts.clear();
            return Ok(None);
        }
        if intent["type"] == "section-action" && intent["action"] == "release-precomp" {
            let layer_id = model.primary().ok_or("No selected layer")?.to_owned();
            return Ok(Some(Request::new(
                caller,
                pm_kernel::Call::Transact {
                    label: Some("Release Pre-comp".into()),
                    expected_revision: Some(model.revision()),
                    body: pm_kernel::WriteBody::Commands(vec![pm_kernel::Command::Comp(
                        pm_kernel::commands::comps::CompCommand::ReleasePrecomp { layer_id },
                    )]),
                },
            )));
        }
        if intent["type"] == "align" {
            use pm_kernel::commands::transform::{AlignMode, TransformCommand};
            let mode = match intent["alignment"].as_str() {
                Some("left") => AlignMode::Left,
                Some("hcenter") => AlignMode::HorizontalCenter,
                Some("right") => AlignMode::Right,
                Some("top") => AlignMode::Top,
                Some("vcenter") => AlignMode::VerticalCenter,
                Some("bottom") => AlignMode::Bottom,
                _ => return Err("Unknown alignment".into()),
            };
            return Ok(Some(Request::new(
                caller,
                pm_kernel::Call::Transact {
                    label: Some("Align Layers".into()),
                    expected_revision: Some(model.revision()),
                    body: pm_kernel::WriteBody::Commands(vec![pm_kernel::Command::Transform(
                        TransformCommand::Align {
                            layer_ids: model.selected_layers().to_vec(),
                            mode,
                            time,
                        },
                    )]),
                },
            )));
        }
        if intent["type"] == "distribute" {
            use pm_kernel::commands::transform::{Spread, TransformCommand};
            let spread = match intent["axis"].as_str() {
                Some("horizontal") => Spread::HorizontalSpace,
                Some("vertical") => Spread::VerticalSpace,
                _ => return Err("Unknown distribution".into()),
            };
            if model.selected_layers().len() < 3 {
                return Err("Select three or more layers to distribute".into());
            }
            return Ok(Some(Request::new(
                caller,
                pm_kernel::Call::Transact {
                    label: Some("Distribute Layers".into()),
                    expected_revision: Some(model.revision()),
                    body: pm_kernel::WriteBody::Commands(vec![pm_kernel::Command::Transform(
                        TransformCommand::Distribute {
                            layer_ids: model.selected_layers().to_vec(),
                            spread,
                            time,
                        },
                    )]),
                },
            )));
        }
        let row = intent["row"].as_str().unwrap_or("");
        let path = match row {
            "position" => "position.x",
            "scale" => "scale.x",
            "anchor" => "anchor.x",
            _ => row,
        };
        if intent["type"] == "collapse" {
            self.collapsed.insert(
                intent["section"].as_str().ok_or("section")?.into(),
                intent["collapsed"].as_bool().ok_or("collapsed")?,
            );
            return Ok(None);
        }
        let handle = self.handles.get(path).ok_or("Unknown property row")?;
        if intent["type"] == "set-value" {
            let value =
                pm_model::json::parse(&intent["value"].to_string()).map_err(|e| e.to_string())?;
            return self.write(model, caller, handle, value, time).map(Some);
        }
        let group;
        let handle = if matches!(row, "position" | "scale" | "anchor") {
            let layer = model.primary().ok_or("No selected layer")?.to_owned();
            group = self
                .adapter
                .register(
                    model,
                    ["x", "y"]
                        .iter()
                        .map(|axis| {
                            Binding::Property(PropertyId::new(&layer, format!("{row}.{axis}")))
                        })
                        .collect(),
                )
                .map_err(|e| e.to_string())?;
            &group
        } else {
            handle
        };
        if intent["type"] == "row-action" && intent["action"] == "edit-expression" {
            // The sheet is the app's: it gets what it edits and where it
            // applies (every channel of the row, on each selected layer).
            let targets = self.adapter.targets(model, handle);
            let source = self
                .adapter
                .read(model, handle, time)
                .map(|states| states.into_iter().find_map(|s| s.expression));
            if matches!(row, "position" | "scale" | "anchor") {
                self.adapter.dispose(handle);
            }
            let targets = targets.map_err(|e| e.to_string())?;
            if targets.is_empty() {
                return Err("Locked layers can’t be edited.".into());
            }
            let layer = model.primary().ok_or("No selected layer")?.to_owned();
            let name = model
                .layer(&layer)
                .ok()
                .and_then(|l| {
                    l.get("name")
                        .and_then(pm_model::Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_default();
            let label = self.labels.get(row).map_or(row, String::as_str);
            self.commands.push((
                "properties.expression".into(),
                vec![json!({
                    "document": model.document_id(),
                    "layer": name,
                    "layers": model.selected_layers().len(),
                    "property": label,
                    "source": source.map_err(|e| e.to_string())?,
                    "targets": targets.iter().map(|id| json!([id.layer, id.path])).collect::<Vec<_>>(),
                })],
            ));
            return Ok(None);
        }
        let action = if intent["type"] == "key" {
            if intent["removeAnimation"] == true {
                V4Action::RemoveAnimation
            } else {
                V4Action::Diamond
            }
        } else {
            match intent["action"].as_str() {
                Some("add-key") => V4Action::AddKey,
                Some("remove-animation") => V4Action::RemoveAnimation,
                Some("reset") => V4Action::Reset(None),
                Some("remove-expression") => V4Action::Expression(None),
                Some("link") => V4Action::LinkScale,
                Some("show-graph") => V4Action::ShowGraph,
                _ => return Err("Unknown property action".into()),
            }
        };
        let result = self.adapter.action(model, caller, handle, action, time);
        if matches!(row, "position" | "scale" | "anchor") {
            self.adapter.dispose(handle);
        }
        match result.map_err(|e| e.to_string())? {
            V4Plan::Request(request) => Ok(Some(*request)),
            V4Plan::Service(intents) => {
                // Show in Graph Editor: the Timeline's graph, on this property.
                use pm_panel_inspector::ServiceIntent;
                let mut shown = None;
                let mut keys = vec![];
                for intent in intents {
                    match intent {
                        ServiceIntent::OpenGraph(id) | ServiceIntent::FocusGraph(id) => {
                            shown = Some(id)
                        }
                        ServiceIntent::Reveal(ids) => {
                            keys = ids.into_iter().map(|id| id.path).collect()
                        }
                        _ => {}
                    }
                }
                let id = shown.ok_or("No property to show")?;
                self.commands.push((
                    "timeline.showGraph".into(),
                    vec![json!(id.layer), json!(id.path), json!(keys)],
                ));
                Ok(None)
            }
        }
    }
}

/// A family's variation facts from the installed fonts (`inspectFont`).
fn font_facts(family: &str) -> pm_panel_inspector::content::FontFacts {
    let installed = (!family.is_empty())
        .then(|| pm_text::registry::installed_family_axes(family))
        .flatten();
    pm_panel_inspector::content::FontFacts {
        readable: installed.is_some(),
        axes: installed
            .unwrap_or_default()
            .into_iter()
            .map(|a| pm_panel_inspector::content::FontAxis {
                tag: String::from_utf8_lossy(&a.tag.to_be_bytes()).into_owned(),
                default: a.default as f64,
                min: Some(a.min as f64),
                max: Some(a.max as f64),
            })
            .collect(),
        faces: vec![],
    }
}
/// The registered axes by name (OpenType); custom axes keep their tag.
fn axis_label(tag: &str) -> String {
    match tag {
        "wght" => "Weight",
        "wdth" => "Width",
        "slnt" => "Slant",
        "ital" => "Italic",
        "opsz" => "Optical Size",
        _ => tag,
    }
    .to_owned()
}

/// What a Studio layer is, in the header (`Key · Softbox`, `World · Sky`).
fn studio_label(
    role: pm_panel_studio3d::properties::Role,
    env: Option<&str>,
    data: &Value,
) -> &'static str {
    use pm_panel_studio3d::properties::Role;
    match (role, env) {
        (_, Some("world")) => "Sky",
        (_, Some("fog")) => "Fog Volume",
        (_, Some("terrain")) => "Terrain",
        (_, Some("scatter")) => "Scatter",
        (Role::Object, _) => {
            if data["object"]["source"].get("nature").is_some() {
                "Nature"
            } else if data["object"]["source"].get("assetId").is_some() {
                "3D Model"
            } else {
                "3D Shape"
            }
        }
        (Role::Light, _) => match data["light"]["type"].as_str() {
            Some("area") => "Softbox",
            Some("spot") => "Spotlight",
            Some("point") => "Bulb",
            _ => "Sun",
        },
        (Role::Camera, _) => "Camera",
    }
}

/// A layer the viewer couldn't draw, in the inline Error grammar: what
/// happened and the fix in one sentence. Raw engine text stays in the Error Log.
fn failure_copy(reason: &str, studio: bool) -> String {
    let r = reason.to_ascii_lowercase();
    if r.contains("offline") || r.contains("missing") && r.contains("media") {
        "This layer’s file is missing, so it isn’t drawn. Relink it in Project.".into()
    } else if r.contains("font") {
        "This layer’s font isn’t installed, so it isn’t drawn. Choose an installed font.".into()
    } else if r.contains("texture") || r.contains("image") {
        "A texture on this layer couldn’t be read. Choose another image in Texture Maps.".into()
    } else if r.contains("primitive") {
        "This shape isn’t supported yet, so it isn’t drawn. Choose another 3D shape.".into()
    } else if studio {
        "This 3D layer couldn’t be loaded, so it isn’t drawn. The rest of the scene still renders; details are in Window ▸ Error Log.".into()
    } else {
        "This layer couldn’t be drawn. Details are in Window ▸ Error Log.".into()
    }
}

/// A finish as a tiny shaded sphere (3d-studio.md §8 shelf), drawn from its
/// material: a rim, a lit body, a highlight sized by roughness and tinted by
/// metal, and a sharp glint for a clear coat. Glass reads as a cool,
/// see-through body with a bright edge. Content colours, not UI chrome.
fn finish_swatch(p: &Value) -> Value {
    let hex = |c: [f64; 3]| {
        format!(
            "#{:02X}{:02X}{:02X}",
            (c[0].clamp(0., 1.) * 255.).round() as u8,
            (c[1].clamp(0., 1.) * 255.).round() as u8,
            (c[2].clamp(0., 1.) * 255.).round() as u8
        )
    };
    let base = p["color"]
        .as_str()
        .and_then(|c| u32::from_str_radix(c.trim_start_matches('#'), 16).ok())
        .unwrap_or(0x808080);
    let base = [(base >> 16) & 255, (base >> 8) & 255, base & 255].map(|c| c as f64 / 255.);
    let num = |k: &str, d: f64| p[k].as_f64().unwrap_or(d);
    let (metal, rough, coat, glass, spec) = (
        num("metalness", 0.),
        num("roughness", 0.5),
        num("clearcoat", 0.),
        num("transmission", 0.),
        num("specular", 1.),
    );
    let mix = |a: [f64; 3], b: [f64; 3], t: f64| -> [f64; 3] {
        std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
    };
    let scale = |a: [f64; 3], k: f64| a.map(|v| v * k);
    let white = [1.; 3];
    let (rim, body, highlight) = if glass > 0.5 {
        // Clear glass: cool, dark-ish centre and a bright edge; frosted is milky.
        let core = mix([0.62, 0.70, 0.78], base, rough * 1.4);
        (mix(core, white, 0.65), core, mix(core, white, 0.95 - rough))
    } else if metal > 0.5 {
        // Metal: strong contrast, the highlight takes the metal's tint.
        (
            scale(base, 0.32),
            base,
            mix(base, white, 0.85 - rough * 0.6),
        )
    } else {
        (
            scale(base, 0.62),
            base,
            mix(base, white, (spec * (1. - rough)).clamp(0.08, 0.9)),
        )
    };
    json!({
        "rim": hex(rim),
        "body": hex(body),
        "highlight": hex(highlight),
        // Sharp finishes: a small bright spot; rough ones: a broad soft one.
        "size": "78%",
        "spot": format!("{}%", (16. + rough * 44.).round()),
        "glintSize": "12%",
        "glint": (coat > 0.5 || (glass > 0.5 && rough < 0.1)).then_some("#FFFFFF"),
    })
}
