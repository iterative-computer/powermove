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
        _ => Err(Error::InvalidIntent("row id")),
    }
}
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
        "snap" => {
            client.model.track_snap_disabled = !client.model.track_snap_disabled;
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
                RowId::Layer(id) => {
                    if client.model.mode == TimelineMode::Tracks {
                        client.model.select_track_layer(&id, m)?.0
                    } else {
                        client.model.select_layer(&id, m)?
                    }
                }
                RowId::Property { layer_id, channel } => {
                    client.model.graph_focus = Some((layer_id.clone(), channel.clone()));
                    vec![
                        pm_kernel::SelectionCall::Layers {
                            ids: vec![layer_id],
                            add: m.command() || m.shift,
                        }
                        .into(),
                        pm_kernel::SelectionCall::Channel(Some(channel)).into(),
                    ]
                }
            };
            Plan {
                calls,
                ..Plan::default()
            }
        }

        "disclosure" => {
            let id = layer_row(intent)?;
            let open = boolean(intent, "open")?;
            let layer = client
                .model
                .index
                .layer(&id)
                .ok_or_else(|| ModelError::UnknownId(id.clone()))?;
            let current = client.model.reveal.layers.get(&id);
            let closed = if layer.kind == "group" {
                current
                    .map(|d| d.group_collapsed)
                    .unwrap_or(layer.group_collapsed)
            } else {
                current.map(|d| d.collapsed).unwrap_or(layer.collapsed)
            };
            if open == closed {
                client.model.toggle_disclosure(&id)?;
            }
            persist = true;
            Plan::default()
        }
        "switch" => {
            let id = layer_row(intent)?;
            match string(intent, "name")? {
                "visibility" => client.model.switch(&id, LayerSwitch::Visible)?,
                "lock" => client.model.switch(&id, LayerSwitch::Locked)?,
                "solo" => client.model.switch(&id, LayerSwitch::Solo)?,
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
            client.model.menu_action(
                &crate::Context::Graph { series: 0 },
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
            client.model.property_diamond(&targets, context.time)?
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
            Plan {
                effects: client
                    .model
                    .adjacent_key(direction, context.time, Some((layer, path)))
                    .map(HostEffect::Seek)
                    .into_iter()
                    .collect(),
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
        "tab" => return Err(Error::MissingHost("composition tab closing")),
        // Pointer gestures on tracks run in the native surface, never through V8.
        _ => return Err(Error::InvalidIntent("action")),
    };
    let effects = client.dispatch(plan).await?;
    if persist && let Err(error) = client.publish_view().await {
        if let Some(published) = client.kernel.published(&client.project) {
            client.model.restore_ui(&published.ui_state);
        }
        return Err(error.into());
    }
    Ok(effects)
}

/// Model-owned window facts for a native presentation/kit adapter. This does not
/// invent property labels, units, timecode formatting, media readiness or copy.
/// A native presenter supplies those via its schema/evaluation/media services.
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
                RowId::Layer(id) => {
                    let layer = model
                        .index
                        .layer(id)
                        .ok_or_else(|| ModelError::UnknownId(id.clone()))?;
                    let disclosure = model.reveal.layers.get(id);
                    value["kind"] = json!("layer");
                    value["layer"] = json!(id);
                    value["layerKind"] = json!(layer.kind);
                    value["index"] = if layer.group.is_some() {
                        Value::Null
                    } else {
                        json!(model.index.layer_ordinal(id).map(|n| n + 1))
                    };
                    value["label"] = json!(layer.name.to_string_lossy());
                    value["selected"] = json!(selected.contains(id));
                    value["locked"] = json!(layer.effectively_locked);
                    value["visible"] = json!(layer.visible);
                    value["solo"] = json!(layer.solo);
                    value["expanded"] = json!(!if layer.kind == "group" {
                        disclosure
                            .map(|d| d.group_collapsed)
                            .unwrap_or(layer.group_collapsed)
                    } else {
                        disclosure.map(|d| d.collapsed).unwrap_or(layer.collapsed)
                    });
                    // Keep authored timing exact. Never convert these to frames on read.
                    value["from"] = json!(layer.from);
                    value["duration"] = json!(layer.duration);
                }
                RowId::Property { layer_id, channel } => {
                    value["kind"] = json!("property");
                    value["layer"] = json!(layer_id);
                    value["path"] = json!(channel);
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
/// Lanes in the renderer's order, so outline rows and painted rows share ordinals.
fn track_window(
    model: &crate::TimelineModel,
    first: usize,
    visible: usize,
) -> Result<Value, Error> {
    use crate::tracks::Area;
    let layout = crate::tracks::layout(&model.index, &model.reveal);
    let mut rows = vec![];
    for (area, count, short, audio) in [
        (Area::Video, layout.video, "V", false),
        (Area::Audio, layout.audio, "A", true),
        (Area::Captions, layout.captions, "C", false),
    ] {
        for lane in 0..count {
            let clips: Vec<_> = layout
                .items
                .iter()
                .filter(|i| i.area == area && i.lane == lane)
                .filter_map(|i| model.index.layer(&i.id))
                .collect();
            let label = format!("{short}{}", lane + 1);
            let enabled = clips
                .iter()
                .all(|l| if audio { !l.audio_muted } else { l.visible });
            rows.push(json!({
                "id": json!(["track", format!("{area:?}"), lane]).to_string(),
                "kind": "track", "depth": 0, "track": label, "label": label, "audio": audio,
                "enabled": enabled, "expanded": false, "selected": false,
                "locked": clips.iter().all(|l| l.effectively_locked) && !clips.is_empty(),
            }));
            for row in model.rows() {
                let RowId::Property { layer_id, channel } = &row.id else {
                    continue;
                };
                if clips.iter().any(|l| l.id == *layer_id) {
                    rows.push(json!({
                        "id": row_id(&row.id), "depth": row.depth, "kind": "property",
                        "layer": layer_id, "path": channel, "selected": false,
                        "locked": model.index.layer(layer_id).is_none_or(|l| l.effectively_locked),
                        "channels": model.track_channels(layer_id, channel).iter()
                            .map(|c| c.path.as_str()).collect::<Vec<_>>(),
                    }));
                }
            }
        }
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
        "hasComposition": model.index.comp_id.is_some(),
        "mode": if model.mode == TimelineMode::Layers { "Layers" } else { "Tracks" },
        "view": match model.graph_display { GraphDisplay::DopeSheet => "DopeSheet", GraphDisplay::Combined => "Combined", GraphDisplay::Graph => "Graph" },
        "snapping": !model.track_snap_disabled,
        "window": { "first": first, "total": total, "rows": rows },
        "indexDigits": model.index.layers.len().max(1).to_string().len(),
        "selection": if !model.selection.keys().is_empty() { json!({"kind":"keys","count":model.selection.keys().len()}) }
            else if !model.selection.layers().is_empty() { json!({"kind":"layers","count":model.selection.layers().len()}) } else { Value::Null },
    })
}
