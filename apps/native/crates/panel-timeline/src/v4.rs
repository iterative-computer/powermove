//! Thin v4 control adapter. Rust retains selection, timing, history and UI state.
//! Platform hit testing, native fields and rendering are explicit host work.
use crate::{
    ClientError, GraphDisplay, HostEffect, KeyInput, LayerSwitch, ModelError, Modifiers, Plan,
    PropertyFacts, RowId, TimelineClient, TimelineMode, resolve_key,
};
use serde_json::{Value, json};

#[derive(Debug)]
pub enum Error {
    InvalidIntent(&'static str),
    MissingHost(&'static str),
    Model(ModelError),
    Client(ClientError),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
impl From<ModelError> for Error {
    fn from(value: ModelError) -> Self {
        Self::Model(value)
    }
}
impl From<ClientError> for Error {
    fn from(value: ClientError) -> Self {
        Self::Client(value)
    }
}

/// Native transport time and monotonic reveal time, supplied by the application.
/// Shell must arbitrate extension shortcuts and text ownership before admitting
/// key events. Coordinates never become authored times in this adapter.
pub struct Context<'a> {
    pub time: f64,
    pub expected_document: u64,
    pub expected_revision: u64,
    pub millis: u64,
    pub in_field: bool,
    pub keys_admitted: bool,
    pub property_facts: &'a PropertyFacts,
}

/// Timecode typed in the timeline header: `HH:MM:SS:FF` with any leading fields
/// omitted, or a bare frame count. The inverse of the header, which counts
/// whole seconds of the nominal rate (30 at 29.97), so typed text lands on the
/// frame it names.
pub fn parse_timecode(text: &str, fps: f64) -> Option<f64> {
    let fields: Vec<f64> = text
        .trim()
        .split([':', ';', '.'])
        .map(|field| field.parse::<u32>().ok().map(f64::from))
        .collect::<Option<_>>()?;
    let (frames, seconds) = fields.split_last()?;
    let seconds = seconds.iter().fold(0., |sum, field| sum * 60. + field);
    let nominal = fps.round().max(1.);
    (fps > 0.).then(|| (seconds * nominal + frames) / fps)
}
/// Collision-free stable IDs, even when imported layer IDs contain punctuation.
pub fn row_id(row: &RowId) -> String {
    match row {
        RowId::Effect { layer_id, effect } => json!(["effect", layer_id, effect]).to_string(),
        RowId::Path { layer_id, path } => json!(["path", layer_id, path]).to_string(),
        RowId::Layer(id) => json!(["layer", id]).to_string(),
        RowId::Property { layer_id, channel } => json!(["property", layer_id, channel]).to_string(),
    }
}
pub fn parse_row(id: &str) -> Result<RowId, Error> {
    let parts: Vec<String> =
        serde_json::from_str(id).map_err(|_| Error::InvalidIntent("row id"))?;
    match parts.as_slice() {
        [kind, id] if kind == "layer" && !id.is_empty() => Ok(RowId::Layer(id.clone())),
        [kind, layer_id, channel]
            if kind == "property" && !layer_id.is_empty() && !channel.is_empty() =>
        {
            Ok(RowId::Property {
                layer_id: layer_id.clone(),
                channel: channel.clone(),
            })
        }
        [kind, layer_id, effect]
            if kind == "effect" && !layer_id.is_empty() && !effect.is_empty() =>
        {
            Ok(RowId::Effect {
                layer_id: layer_id.clone(),
                effect: effect.clone(),
            })
        }
        [kind, layer_id, path] if kind == "path" && !layer_id.is_empty() && !path.is_empty() => {
            Ok(RowId::Path {
                layer_id: layer_id.clone(),
                path: path.clone(),
            })
        }
        _ => Err(Error::InvalidIntent("row id")),
    }
}
/// The app commands the timeline's empty state offers (v1's buttons).
pub const RUN_COMMANDS: [&str; 5] = ["newText", "newShape", "newSolid", "newNull", "import"];

fn string<'a>(v: &'a Value, key: &'static str) -> Result<&'a str, Error> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or(Error::InvalidIntent(key))
}
fn boolean(v: &Value, key: &'static str) -> Result<bool, Error> {
    v.get(key)
        .and_then(Value::as_bool)
        .ok_or(Error::InvalidIntent(key))
}
fn modifiers(v: &Value) -> Result<Modifiers, Error> {
    let flag = |key| match v.get(key) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        _ => Err(Error::InvalidIntent("modifiers")),
    };
    Ok(Modifiers {
        shift: flag("shiftKey")?,
        alt: flag("altKey")?,
        meta: flag("metaKey")?,
        control: flag("ctrlKey")?,
    })
}
/// `["track", area, lane]` (see `track_window`).
fn track_row(intent: &Value) -> Option<(crate::tracks::Area, usize)> {
    use crate::tracks::Area;
    let parts: Vec<Value> = serde_json::from_str(intent.get("id")?.as_str()?).ok()?;
    match parts.as_slice() {
        [kind, area, lane] if kind.as_str() == Some("track") => {
            let area = match area.as_str()? {
                "Video" => Area::Video,
                "Audio" => Area::Audio,
                "Captions" => Area::Captions,
                _ => return None,
            };
            Some((area, usize::try_from(lane.as_u64()?).ok()?))
        }
        _ => None,
    }
}
fn layer_row(intent: &Value) -> Result<String, Error> {
    match parse_row(string(intent, "id")?)? {
        RowId::Layer(id) => Ok(id),
        _ => Err(Error::InvalidIntent("layer row")),
    }
}
fn key_name(key: &str) -> String {
    match key {
        " " => "space".into(),
        "ArrowLeft" => "left".into(),
        "ArrowRight" => "right".into(),
        "ArrowUp" => "up".into(),
        "ArrowDown" => "down".into(),
        other => other.to_lowercase(),
    }
}

/// Native control actions, invoked only on the worker. Returned HostEffects
/// require real host execution/acknowledgement before resolving the JS act call.
/// Missing host work is an error, never a success-shaped placeholder.
pub async fn dispatch(
    client: &mut TimelineClient,
    intent: &Value,
    context: &Context<'_>,
) -> Result<Vec<HostEffect>, Error> {
    if !context.time.is_finite() {
        return Err(Error::InvalidIntent("time"));
    }
    client.sync()?;
    if client.model.index.doc_id != context.expected_document
        || client.model.index.revision != context.expected_revision
    {
        return Err(Error::InvalidIntent("stale native surface"));
    }
    if let Some(effects) = crate::v4_actions::dispatch(client, intent, context).await? {
        return Ok(effects);
    }
    let action = string(intent, "type")?;
    let mut persist = false;
    let plan = match action {
        "play" => client.model.command(
            "play",
            context.time,
            None,
            false,
            context.millis,
            context.property_facts,
        )?,
        // The shell's word on whether the timeline holds the keyboard.
        "panelFocus" => {
            client.model.panel_blurred = !boolean(intent, "focused")?;
            Plan::default()
        }
        "mode" => {
            client.model.mode = match string(intent, "mode")? {
                "Layers" => TimelineMode::Layers,
                "Tracks" => TimelineMode::Tracks,
                _ => return Err(Error::InvalidIntent("mode")),
            };
            persist = true;
            Plan::default()
        }
        "view" => {
            client.model.graph_display = match string(intent, "view")? {
                "DopeSheet" => GraphDisplay::DopeSheet,
                "Combined" => GraphDisplay::Combined,
                "Graph" => GraphDisplay::Graph,
                _ => return Err(Error::InvalidIntent("view")),
            };
            persist = true;
            Plan::default()
        }
        // v1–3's `timeline.graph = shown`: the graph shown or not, keeping a
        // combined view, whatever it was.
        "graph" => {
            if !boolean(intent, "shown")? {
                client.model.graph_display = GraphDisplay::DopeSheet;
            } else if client.model.graph_display == GraphDisplay::DopeSheet {
                client.model.graph_display = GraphDisplay::Graph;
            }
            persist = true;
            Plan::default()
        }
        "snap" => {
            client.model.track_snap_disabled = !client.model.track_snap_disabled;
            persist = true;
            Plan::default()
        }
        "hideShy" => {
            client.model.hide_shy = !client.model.hide_shy;
            client.model.rebuild_rows();
            persist = true;
            Plan::default()
        }
        "selectRow" => {
            let row = parse_row(string(intent, "id")?)?;
            if client.model.row_position(&row).is_none() {
                return Err(ModelError::HiddenRow.into());
            }
            let m = modifiers(
                intent
                    .get("modifiers")
                    .ok_or(Error::InvalidIntent("modifiers"))?,
            )?;
            let calls = match row {
                RowId::Effect { .. } | RowId::Path { .. } => {
                    return Err(Error::InvalidIntent("section row"));
                }
                RowId::Layer(id) => {
                    if client.model.mode == TimelineMode::Tracks {
                        client.model.select_track_layer(&id, m)?.0
                    } else {
                        client.model.select_layer(&id, m)?
                    }
                }
                RowId::Property { layer_id, channel } => {
                    client.model.key_selection_active = false;
                    client.model.graph_focus = Some((layer_id.clone(), channel.clone()));
                    vec![
                        pm_kernel::SelectionCall::Batch(vec![
                            pm_kernel::SelectionCall::Layers {
                                ids: vec![layer_id],
                                add: m.command() || m.shift,
                            },
                            pm_kernel::SelectionCall::Channel(Some(channel)),
                        ])
                        .user()
                        .into(),
                    ]
                }
            };
            Plan {
                calls,
                ..Plan::default()
            }
        }

        // A track header's chevron is the layer rows' disclosure on a lane.
        "disclosure" if track_row(intent).is_some() => {
            let (area, lane) = track_row(intent).expect("checked");
            let open = boolean(intent, "open")?;
            if intent.get("all").and_then(Value::as_bool) == Some(true) {
                client.model.disclose_all_tracks(open);
            } else {
                client.model.disclose_track(area, lane, open);
            }
            persist = true;
            Plan::default()
        }
        "disclosure" => {
            let id = layer_row(intent)?;
            let open = boolean(intent, "open")?;
            if intent.get("all").and_then(Value::as_bool) == Some(true) {
                let ids: Vec<_> = client
                    .model
                    .index
                    .layers
                    .iter()
                    .filter(|layer| client.model.layer_disclosable(&layer.id))
                    .map(|layer| layer.id.clone())
                    .collect();
                for id in ids {
                    client.model.set_disclosure(&id, open)?;
                }
                client.model.rebuild_rows();
            } else if open != client.model.layer_expanded(&id)
                && client.model.layer_disclosable(&id)
            {
                client.model.toggle_disclosure(&id)?;
            }
            persist = true;
            Plan::default()
        }
        // A track row's switches (Tracks mode) act on every clip on its lane.
        "switch" if track_row(intent).is_some() => {
            let (area, lane) = track_row(intent).expect("checked");
            client
                .model
                .track_switch(area, lane, string(intent, "name")?, context.time)?
        }
        "switch" => {
            let id = layer_row(intent)?;
            match string(intent, "name")? {
                "visibility" => client.model.switch(&id, LayerSwitch::Visible)?,
                "lock" => client.model.switch(&id, LayerSwitch::Locked)?,
                "solo" => client.model.switch(&id, LayerSwitch::Solo)?,
                "shy" => client.model.switch(&id, LayerSwitch::Shy)?,
                "threeD" => client.model.switch(&id, LayerSwitch::ThreeD)?,
                "mute" => client.model.mute(&id, context.time)?,
                _ => return Err(Error::InvalidIntent("switch")),
            }
        }
        "setName" => {
            let draft = crate::Draft::Name {
                layer_id: layer_row(intent)?,
                text: string(intent, "name")?.to_owned(),
            };
            draft.finish(true, client.model.index.revision)
        }
        "seek" => {
            let time = intent
                .get("time")
                .and_then(Value::as_f64)
                .filter(|t| t.is_finite())
                .ok_or(Error::InvalidIntent("time"))?;
            Plan::seek(time.clamp(0., client.model.index.duration))
        }
        "graphMenu" => {
            persist = true;
            // The curves shown decide what the menu offers (Merge wants two).
            let series = client.model.graph_series().0.len();
            client.model.menu_action(
                &crate::Context::Graph { series },
                string(intent, "action")?,
                context.time,
            )?
        }
        "tab" if !boolean(intent, "close")? => Plan {
            effects: vec![HostEffect::OpenComposition(
                string(intent, "id")?.to_owned(),
            )],
            ..Plan::default()
        },
        "propertyValue" => {
            let layer = string(intent, "layer")?;
            let path = string(intent, "path")?;
            let value = intent["value"]
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or(Error::InvalidIntent("value"))?;
            let targets = client.model.property_targets(layer, &[path.to_owned()])?;
            let published = client
                .kernel
                .published(&client.project)
                .ok_or(Error::MissingHost("project"))?;
            let mut mirror = pm_kernel::QueryMirror::new(&published);
            let mut commands = Vec::new();
            for (layer, path) in targets {
                let evaluated =
                    |mirror: &mut pm_kernel::QueryMirror, path: &str| -> Result<f64, Error> {
                        mirror
                            .queries()
                            .evaluate(
                                client.model.index.comp_id.as_deref(),
                                &layer,
                                path,
                                context.time,
                            )
                            .map_err(|_| Error::MissingHost("evaluated property"))?
                            .as_f64()
                            .ok_or(Error::InvalidIntent("numeric property"))
                    };
                let baseline = evaluated(&mut mirror, &path)?;
                if matches!(path.as_str(), "scale.x" | "scale.y") {
                    let paired = if path == "scale.x" {
                        "scale.y"
                    } else {
                        "scale.x"
                    };
                    let other = evaluated(&mut mirror, paired)?;
                    for call in client
                        .model
                        .scale_value(&layer, &path, value, context.time, baseline, other)?
                        .calls
                    {
                        if let pm_kernel::Call::Transact {
                            body: pm_kernel::WriteBody::Commands(c),
                            ..
                        } = call
                        {
                            commands.extend(c);
                        }
                    }
                } else {
                    let mut set = pm_kernel::commands::properties::SetProperty::new(
                        &layer,
                        &path,
                        pm_model::Value::Number(value),
                        context.time,
                    );
                    set.preserve_hand_edits = false;
                    commands.push(pm_kernel::PropertyCommand::Set(set).into());
                }
            }
            Plan::edit(commands, client.model.index.revision)
        }
        "keyToggle" => {
            let layer = string(intent, "layer")?;
            let path = string(intent, "path")?;
            let paths = client
                .model
                .track_channels(layer, path)
                .iter()
                .map(|c| c.path.clone())
                .collect::<Vec<_>>();
            if paths.is_empty() {
                return Err(ModelError::UnknownId(path.into()).into());
            }
            let targets = client.model.property_targets(layer, &paths)?;
            if targets.is_empty() {
                return Err(ModelError::Locked(layer.into()).into());
            }
            let mut plan = client.model.property_diamond(&targets, context.time)?;
            let focus = focus_property(client, layer, path);
            plan.calls.splice(0..0, focus);
            plan
        }
        "adjacentKey" => {
            let layer = string(intent, "layer")?;
            let path = string(intent, "path")?;
            if client.model.track_channels(layer, path).is_empty() {
                return Err(ModelError::UnknownId(path.into()).into());
            }
            let direction = match string(intent, "direction")? {
                "previous" => -1,
                "next" => 1,
                _ => return Err(Error::InvalidIntent("direction")),
            };
            let effects = client
                .model
                .adjacent_key(direction, context.time, Some((layer, path)))
                .map(HostEffect::Seek)
                .into_iter()
                .collect();
            Plan {
                calls: focus_property(client, layer, path),
                effects,
                ..Plan::default()
            }
        }
        "scaleLink" => {
            let id = string(intent, "layer")?;
            let layer = client
                .model
                .index
                .layer(id)
                .ok_or_else(|| ModelError::UnknownId(id.into()))?;
            client.model.scale_link(id, !layer.scale_linked)
        }
        "keyDown" => {
            if !context.keys_admitted {
                return Err(Error::MissingHost("shell shortcut arbitration"));
            }
            let event = intent.get("event").ok_or(Error::InvalidIntent("event"))?;
            let key = key_name(string(event, "key")?);
            let m = modifiers(event)?;
            let repeat = match event.get("repeat") {
                None => false,
                Some(Value::Bool(v)) => *v,
                _ => return Err(Error::InvalidIntent("repeat")),
            };
            if key == "escape" && client.active_drag().is_some() {
                client.finish_drag(false).await?;
                return Ok(Vec::new());
            }
            let shortcut = resolve_key(&KeyInput {
                key: &key,
                modifiers: m,
                repeat,
                in_field: context.in_field,
            })
            .ok_or(Error::MissingHost(
                "local focus/field or unclaimed key routing",
            ))?;
            // View/reveal commands are UI state; authored edits stay in the kernel.
            persist =
                shortcut.command.starts_with("timeline.reveal") || shortcut.command == "graph";
            client.model.command(
                shortcut.command,
                context.time,
                shortcut.frames,
                shortcut.shift_reveal,
                context.millis,
                context.property_facts,
            )?
        }
        "import" => Plan {
            effects: vec![client.model.import_drop(context.time, None, false)?],
            ..Plan::default()
        },
        // The empty composition's buttons run the app's own commands, as
        // v1's did; only these.
        "run" => {
            let command = string(intent, "command")?;
            if !RUN_COMMANDS.contains(&command) {
                return Err(Error::InvalidIntent("command"));
            }
            Plan {
                effects: vec![HostEffect::Run(command.into())],
                ..Plan::default()
            }
        }
        "tab" => return Err(Error::MissingHost("composition tab closing")),
        // Pointer gestures on tracks run in the native surface, never through V8.
        _ => return Err(Error::InvalidIntent("action")),
    };
    let effects = client.dispatch(plan).await?;
    // Whatever changed the view (a key toggling the mode too) publishes it.
    let published = if persist {
        client.publish_view().await
    } else {
        client.publish_view_if_changed().await
    };
    if let Err(error) = published {
        if let Some(published) = client.kernel.published(&client.project) {
            client.model.restore_ui(&published.ui_state);
        }
        return Err(error.into());
    }
    Ok(effects)
}

/// Rows and authored labels for a native presentation/kit adapter.
/// A native presenter supplies units, readouts, timecode and media readiness.
/// Ordinal windows use exactly one viewport of overscan on each side.
pub fn window(client: &mut TimelineClient, first: usize, visible: usize) -> Result<Value, Error> {
    if visible == 0 {
        return Err(Error::InvalidIntent("visible rows"));
    }
    client.sync()?;
    project_window(&client.model, first, visible)
}

/// Project a model already synchronized to one immutable publication. Native
/// presenters use this with evaluator/readout facts from that same publication;
/// another actor publication cannot mix two document revisions in one frame.
pub fn project_window(
    model: &crate::TimelineModel,
    first: usize,
    visible: usize,
) -> Result<Value, Error> {
    if visible == 0 {
        return Err(Error::InvalidIntent("visible rows"));
    }
    if model.mode == TimelineMode::Tracks {
        return track_window(model, first, visible);
    }
    let window = model.window(first, visible, visible);
    let selected = model
        .selection
        .layers()
        .iter()
        .collect::<std::collections::HashSet<_>>();
    let rows = window
        .rows
        .iter()
        .map(|row| {
            let mut value = json!({ "id": row_id(&row.id), "depth": row.depth });
            match &row.id {
                RowId::Path { layer_id, path } => return path_heading(model, row, layer_id, path),
                RowId::Effect { layer_id, effect } => {
                    let layer = model
                        .index
                        .layer(layer_id)
                        .ok_or_else(|| ModelError::UnknownId(layer_id.clone()))?;
                    value["kind"] = json!("heading");
                    value["locked"] = json!(layer.effectively_locked);
                    value["layer"] = json!(layer_id);
                    value["label"] = json!(
                        layer
                            .effects
                            .iter()
                            .find(|(id, _)| id == effect)
                            .map(|(_, label)| label.as_str())
                            .unwrap_or(effect)
                    );
                    value["selected"] = json!(false);
                }
                RowId::Layer(id) => {
                    let layer = model
                        .index
                        .layer(id)
                        .ok_or_else(|| ModelError::UnknownId(id.clone()))?;
                    value["kind"] = json!("layer");
                    value["canOpen"] = json!(model.layer_disclosable(id));
                    value["layer"] = json!(id);
                    let definition = layer.definition.as_deref();
                    value["layerKind"] =
                        json!(crate::render::display_kind(&layer.kind, definition));
                    value["labelColor"] =
                        json!(crate::render::Label::for_layer(&layer.kind, definition).name());
                    value["index"] = json!(model.index.layer_ordinal(id).map(|n| n + 1));
                    value["label"] = json!(layer.name.to_string_lossy());
                    value["selected"] = json!(selected.contains(id));
                    value["locked"] = json!(layer.effectively_locked);
                    value["visible"] = json!(layer.visible);
                    value["solo"] = json!(layer.solo);
                    value["shy"] = json!(layer.shy);
                    value["threeD"] = json!(layer.three_d);
                    value["canBe3D"] = json!(layer.can_be_3d());
                    value["expanded"] = json!(model.layer_expanded(id));
                    if layer.effectively_locked {
                        value["lockRow"] = json!(
                            model
                                .index
                                .locking_layer(id)
                                .map(|owner| row_id(&RowId::Layer(owner.id.clone())))
                        );
                    } else if layer.kind == "group"
                        && value["expanded"] == false
                        && model.index.has_locked_descendant(id)
                    {
                        value["containsLockedLayers"] = json!(true);
                    }
                    // Keep authored timing exact. Never convert these to frames on read.
                    value["from"] = json!(layer.from);
                    value["duration"] = json!(layer.duration);
                }
                RowId::Property { layer_id, channel } => {
                    value["kind"] = json!("property");
                    value["layer"] = json!(layer_id);
                    value["path"] = json!(channel);
                    value["label"] = json!(model.channel_label(layer_id, channel));
                    value["locked"] = json!(
                        model
                            .index
                            .layer(layer_id)
                            .ok_or_else(|| ModelError::UnknownId(layer_id.clone()))?
                            .effectively_locked
                    );
                    value["selected"] = json!(false);
                    if channel == "scale" {
                        value["scaleLinked"] =
                            json!(model.index.layer(layer_id).unwrap().scale_linked);
                    }
                    value["channels"] = json!(
                        model
                            .track_channels(layer_id, channel)
                            .iter()
                            .map(|c| c.path.as_str())
                            .collect::<Vec<_>>()
                    );
                }
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(frame_facts(model, window.first, window.total, rows))
}

fn path_heading(
    model: &crate::TimelineModel,
    row: &crate::Row,
    layer_id: &str,
    path: &str,
) -> Result<Value, Error> {
    let layer = model
        .index
        .layer(layer_id)
        .ok_or_else(|| ModelError::UnknownId(layer_id.to_owned()))?;
    Ok(json!({
        "id": row_id(&row.id), "depth": row.depth, "kind": "heading",
        "layer": layer_id, "locked": layer.effectively_locked, "selected": false,
        "label": layer.path_name(path),
    }))
}
/// Lanes in the renderer's order, so outline rows and painted rows share ordinals.
fn track_window(
    model: &crate::TimelineModel,
    first: usize,
    visible: usize,
) -> Result<Value, Error> {
    use crate::tracks::Area;
    let layout = model.track_layout();
    let mut clips_by_lane = std::collections::HashMap::<_, Vec<_>>::new();
    for item in &layout.items {
        if let Some(clip) = model.index.layer(&item.id) {
            clips_by_lane.entry((item.area, item.lane)).or_default().push(clip);
        }
    }
    let mut properties_by_lane = std::collections::HashMap::<_, Vec<_>>::new();
    for row in model.rows() {
        if let RowId::Path { layer_id, .. } | RowId::Property { layer_id, .. } = &row.id
            && let Some(item) = layout.item(layer_id)
        {
            properties_by_lane.entry((item.area, item.lane)).or_default().push((row, layer_id));
        }
    }
    let mut rows = vec![];
    for (area, lane) in layout.display_order() {
        let (short, audio) = match area {
            Area::Video => ("V", false),
            Area::Audio => ("A", true),
            Area::Captions => ("C", false),
        };
        let clips = clips_by_lane.remove(&(area, lane)).unwrap_or_default();
        let label = format!("{short}{}", lane + 1);
        let enabled = clips
            .iter()
            .all(|l| if audio { !l.audio_muted } else { l.visible });
        // What the track holds, quietly after its name (v1): the clip's
        // name, or how many clips.
        let summary = match clips.as_slice() {
            [] => String::new(),
            [one] => one.name.to_string_lossy(),
            many => format!("{} clips", many.len()),
        };
        let collapsed = model.track_collapsed(area, lane);
        let mut lane_rows = vec![];
        let properties = properties_by_lane.remove(&(area, lane)).unwrap_or_default();
        let can_open = !properties.is_empty();
        for (row, layer) in properties.into_iter().filter(|_| !collapsed) {
            match &row.id {
                RowId::Path { path, .. } => lane_rows.push(path_heading(model, row, layer, path)?),
                RowId::Property { channel, .. } => lane_rows.push(json!({
                    "id": row_id(&row.id), "depth": row.depth, "kind": "property",
                    "layer": layer, "path": channel, "selected": false,
                    "label": model.channel_label(layer, channel),
                    "locked": model.index.layer(layer).is_none_or(|l| l.effectively_locked),
                    "channels": model.track_channels(layer, channel).iter()
                        .map(|c| c.path.as_str()).collect::<Vec<_>>(),
                })),
                _ => unreachable!(),
            }
        }
        rows.push(json!({
            "id": json!(["track", format!("{area:?}"), lane]).to_string(),
            "kind": "track", "depth": 0, "track": label, "label": label, "audio": audio,
            "summary": summary,
            "enabled": enabled, "selected": false,
            // A lane with nothing to show gets no chevron, only the column's
            // blank space, so every track's label stays on the same inset.
            "canOpen": can_open, "expanded": !collapsed,
            "locked": clips.iter().all(|l| l.effectively_locked) && !clips.is_empty(),
        }));
        rows.extend(lane_rows);
    }
    let total = rows.len();
    let start = first.min(total).saturating_sub(visible);
    let end = first.saturating_add(visible.saturating_mul(2)).min(total);
    Ok(frame_facts(model, start, total, rows[start..end].to_vec()))
}
fn frame_facts(
    model: &crate::TimelineModel,
    first: usize,
    total: usize,
    rows: Vec<Value>,
) -> Value {
    json!({
        "document": model.index.doc_id.to_string(), "revision": model.index.revision.to_string(),
        "hasComposition": model.index.has_composition(),
        "focused": !model.panel_blurred,
        "mode": if model.mode == TimelineMode::Layers { "Layers" } else { "Tracks" },
        "view": match model.graph_display { GraphDisplay::DopeSheet => "DopeSheet", GraphDisplay::Combined => "Combined", GraphDisplay::Graph => "Graph" },
        "snapping": !model.track_snap_disabled,
        "hideShy": model.hide_shy,
        "shyLayers": model.index.layers.iter().any(|l| l.shy),
        "window": { "first": first, "total": total, "rows": rows },
        "indexDigits": model.index.layers.len().max(1).to_string().len(),
        "selection": if !model.selection.keys().is_empty() { json!({"kind":"keys","count":model.selection.keys().len()}) }
            else if !model.selection.layers().is_empty() { json!({"kind":"layers","count":model.selection.layers().len()}) } else { Value::Null },
    })
}
/// A press on a property row's diamond or key arrows works on that property
/// as a press on the row does (v1's gutterDown): the graph plots its track,
/// it becomes the selected channel and its layer is selected. The buttons
/// keep the click from the row, so they do it themselves.
fn focus_property(client: &mut TimelineClient, layer: &str, path: &str) -> Vec<pm_kernel::Call> {
    client.model.graph_focus = Some((layer.into(), path.into()));
    let mut calls: Vec<pm_kernel::Call> = vec![];
    if !client.model.selection.contains(layer) {
        calls.push(
            pm_kernel::SelectionCall::Layers {
                ids: vec![layer.into()],
                add: false,
            }
            .into(),
        );
    }
    calls.push(pm_kernel::SelectionCall::Channel(Some(path.into())).into());
    calls
}
