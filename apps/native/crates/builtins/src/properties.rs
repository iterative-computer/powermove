//! Properties presentation using model-issued, selection-fenced leases.
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
    collapsed: BTreeMap<String, bool>,
    selection: Vec<String>,
    document: Option<u64>,
    sequence: u64,
}
impl Properties {
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
    pub fn snapshot(
        &mut self,
        model: &mut Inspector,
        time: f64,
        playing: bool,
    ) -> Result<Value, String> {
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
            // The model decides applicability; this adapter only maps authored
            // labels and supported control descriptors to frozen v4 rows.
            for field in model.fields().map_err(|e| e.to_string())? {
                let (id, label, unit) = match &field.binding {
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
                    (FieldType::Color, Value::String(_)) => {
                        json!({"kind":"colour","binding":lease,"value":value,"mixed":state.mixed})
                    }
                    (FieldType::Font, Value::String(_)) => {
                        json!({"kind":"font","binding":lease,"value":value,"options":[],"mixed":state.mixed})
                    }
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
                            "content.align" => json!(["left", "center", "right"]),
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
                let row = json!({"id":id,"label":label,"locked":!model.editable(&layer),"animation":animation,"control":control});
                if id.starts_with("layer.") {
                    layer_rows.push(row);
                } else {
                    content.push(row);
                }
            }
            if !content.is_empty() {
                sections.push(json!({"id":"content","title":"Content","collapsed":self.collapsed.get("content").copied().unwrap_or(false),"rows":content}));
            }
            if !audio {
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
                        rows.push(row);
                    }
                }
                sections.push(json!({"id":"transform","title":"Transform","collapsed":self.collapsed.get("transform").copied().unwrap_or(false),"rows":rows}));
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
        }
        Ok(json!({"target":target,"sections":sections,"playing":playing,"time":time}))
    }
    pub fn valid(&self, handle: &str, channels: &[String]) -> bool {
        self.handles
            .iter()
            .any(|(path, h)| h == handle && channels == [path.clone()])
    }
    pub fn write(
        &self,
        model: &mut Inspector,
        caller: Caller,
        handle: &str,
        value: pm_model::Value,
        time: f64,
    ) -> Result<Request, String> {
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
                _ => return Err("Property action requires native service".into()),
            }
        };
        let result = self.adapter.action(model, caller, handle, action, time);
        if matches!(row, "position" | "scale" | "anchor") {
            self.adapter.dispose(handle);
        }
        match result.map_err(|e| e.to_string())? {
            V4Plan::Request(request) => Ok(Some(request)),
            V4Plan::Service(_) => Err("Graph service unavailable".into()),
        }
    }
}
