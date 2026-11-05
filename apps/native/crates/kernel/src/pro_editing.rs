//! Native pro-edit adapter for UI/agent/MCP hosts. Decode only arguments here;
//! the host supplies the authenticated caller and observed document identity.
//! All planning runs on the kernel actor, against its current state.
use crate::{
    Ack, Call, Caller, Command, CommandError, Kernel, KernelFailure, Request, SelectionCall,
    commands::{
        pro_edit::{Edge, ProEditCommand as Edit, SplitTail, TrackTarget},
        pro_links::{self, ProLinkCommand as Link},
        pro_source::{
            EditPoints, ProSourceCommand, SourceEditMode, SourceMedia, SourceSplit, SourceTarget,
        },
        pro_transport::{ShuttleInput, ShuttleOutcome, ShuttleState},
    },
    state::{StateAck, StateOp, StateTransaction},
};
use pm_model::{Map, Value};

/// A native namespace, separate from the frozen v1–3/MCP vocabulary.
pub const OPERATIONS: &[&str] = &[
    "move",
    "trim",
    "slip",
    "delete",
    "split",
    "rippleDelete",
    "rippleTrim",
    "roll",
    "slide",
    "link",
    "unlink",
    "select",
    "inspect",
    "undo",
    "redo",
    "sourceMarks",
    "sourceEdit",
    "shuttle",
];
#[derive(Clone, Debug)]
pub struct ProRequest {
    pub caller: Caller,
    pub document: u64,
    pub revision: u64,
    /// Required for state-dependent actions; prevents lost marks/clock updates.
    pub state_revision: Option<u64>,
    /// Document/selection requests use the kernel's ordinary lost-ack dedupe.
    /// State-dependent actions reject txids: callers must re-inspect on retry.
    pub txid: Option<u64>,
    pub action: ProAction,
}
#[derive(Clone, Debug)]
pub enum ProAction {
    Command(Command),
    Undo,
    Redo,
    Select {
        ids: Vec<String>,
        bypass: bool,
        add: bool,
    },
    Inspect {
        ids: Vec<String>,
    },
    SourceMarks(SourceMonitor),
    SourceEdit {
        mode: SourceEditMode,
        targets: Vec<SourceTarget>,
        link_id: Option<String>,
        splits: Vec<SourceSplit>,
    },
    Shuttle(ShuttleInput),
    /// Native clock host only; intentionally absent from argument decoding.
    ObserveClock {
        binding: u64,
        time: f64,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct SourceMonitor {
    pub comp_id: Option<String>,
    pub asset_id: String,
    pub points: EditPoints,
}
#[derive(Clone, Debug)]
pub enum ProReply {
    Document(Ack),
    SourceEdit {
        ack: Ack,
        state_revision: u64,
    },
    State(StateAck),
    Inspection(Value),
    /// Key resolver may invoke the next binding exactly once.
    Declined,
}
#[derive(Clone, Debug)]
pub(crate) struct SourceFinish {
    state: StateTransaction,
}
pub(crate) enum Prepared {
    Call(Request),
    SourceEdit {
        request: Request,
        finish: SourceFinish,
    },
    State(StateTransaction),
    Inspection(Value),
    Declined,
}
fn error(message: impl Into<String>) -> KernelFailure {
    KernelFailure::Storage(message.into())
}
fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::Invalid(message.into())
}
fn object(v: &Value) -> Result<&Map, CommandError> {
    v.as_object()
        .ok_or_else(|| invalid("Expected an argument object"))
}
fn text(v: &Value, key: &str) -> Result<String, CommandError> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| invalid(format!("Expected nonempty {key}")))
}
fn optional_text(v: &Value, key: &str) -> Result<Option<String>, CommandError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        _ => text(v, key).map(Some),
    }
}
fn num(v: &Value, key: &str) -> Result<f64, CommandError> {
    v.get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .ok_or_else(|| invalid(format!("Expected finite {key}")))
}
fn optional_num(v: &Value, key: &str) -> Result<Option<f64>, CommandError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        _ => num(v, key).map(Some),
    }
}
fn flag(v: &Value, key: &str) -> Result<bool, CommandError> {
    v.get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid(format!("Expected boolean {key}")))
}
fn array<'a>(v: &'a Value, key: &str) -> Result<&'a [Value], CommandError> {
    v.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| invalid(format!("Expected array {key}")))
}
fn ids(v: &Value) -> Result<Vec<String>, CommandError> {
    array(v, "layerIds")?
        .iter()
        .map(|v| {
            v.as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| invalid("Expected nonempty layer id"))
        })
        .collect()
}
fn fields(v: &Value, keys: &[&str]) -> Result<(), CommandError> {
    for (key, _) in object(v)? {
        if !keys.contains(&key.as_str().unwrap_or("")) {
            return Err(invalid(format!("Unknown argument {key}")));
        }
    }
    Ok(())
}
fn track(v: &Value) -> Result<TrackTarget, CommandError> {
    fields(v, &["groupId", "track"])?;
    let t = num(v, "track")?;
    if t.fract() != 0. || t < i32::MIN as f64 || t > i32::MAX as f64 {
        return Err(invalid("Track must be an i32"));
    }
    Ok(TrackTarget {
        group_id: optional_text(v, "groupId")?,
        track: t as i32,
    })
}
fn tracks(v: &Value) -> Result<Vec<TrackTarget>, CommandError> {
    array(v, "tracks")?.iter().map(track).collect()
}
fn edge(v: &Value) -> Result<Edge, CommandError> {
    match text(v, "edge")?.as_str() {
        "in" => Ok(Edge::In),
        "out" => Ok(Edge::Out),
        _ => Err(invalid("Edge must be in or out")),
    }
}
impl ProAction {
    /// Strict native argument decoding. No caller, revision, capability or source
    /// provenance is accepted from this object. Never invoke a legacy fallback
    /// on an error (only `ProReply::Declined` requests fallback).
    pub fn decode(v: &Value) -> Result<Self, CommandError> {
        let op = text(v, "op")?;
        let keys: &[&str] = match op.as_str() {
            "move" | "slip" => &["op", "layerIds", "delta", "linked"],
            "trim" => &["op", "layerIds", "edge", "delta", "linked"],
            "delete" | "unlink" => &["op", "layerIds", "linked"],
            "split" => &["op", "layerId", "time", "tails", "linked", "tailLinkId"],
            "rippleDelete" => &["op", "layerIds", "tracks", "linked"],
            "rippleTrim" => &["op", "layerId", "edge", "delta", "tracks", "linked"],
            "roll" => &["op", "leftId", "rightId", "delta", "linked"],
            "slide" => &["op", "previousId", "layerId", "nextId", "delta", "linked"],
            "link" => &["op", "videoId", "audioId", "linkId"],
            "select" => &["op", "layerIds", "bypass", "add"],
            "inspect" => &["op", "layerIds"],
            "undo" | "redo" => &["op"],
            "sourceMarks" => &[
                "op",
                "compId",
                "assetId",
                "sourceIn",
                "sourceOut",
                "timelineIn",
                "timelineOut",
            ],
            "sourceEdit" => &["op", "mode", "targets", "linkId", "splits"],
            "shuttle" => &["op", "key", "before", "after"],
            _ => return Err(invalid("Unknown native pro-edit operation")),
        };
        fields(v, keys)?;
        let c = match op.as_str() {
            "move" => Edit::Move {
                layer_ids: ids(v)?,
                delta: num(v, "delta")?,
                linked: flag(v, "linked")?,
            }
            .into(),
            "trim" => Edit::Trim {
                layer_ids: ids(v)?,
                edge: edge(v)?,
                delta: num(v, "delta")?,
                linked: flag(v, "linked")?,
            }
            .into(),
            "slip" => Edit::Slip {
                layer_ids: ids(v)?,
                delta: num(v, "delta")?,
                linked: flag(v, "linked")?,
            }
            .into(),
            "delete" => Edit::Delete {
                layer_ids: ids(v)?,
                linked: flag(v, "linked")?,
            }
            .into(),
            "split" => Edit::Split {
                layer_id: text(v, "layerId")?,
                time: num(v, "time")?,
                linked: flag(v, "linked")?,
                tail_link_id: optional_text(v, "tailLinkId")?,
                tails: array(v, "tails")?
                    .iter()
                    .map(|t| {
                        fields(t, &["layerId", "tailId"])?;
                        Ok(SplitTail {
                            layer_id: text(t, "layerId")?,
                            tail_id: text(t, "tailId")?,
                        })
                    })
                    .collect::<Result<_, CommandError>>()?,
            }
            .into(),
            "rippleDelete" => Edit::RippleDelete {
                layer_ids: ids(v)?,
                tracks: tracks(v)?,
                linked: flag(v, "linked")?,
            }
            .into(),
            "rippleTrim" => Edit::RippleTrim {
                layer_id: text(v, "layerId")?,
                edge: edge(v)?,
                delta: num(v, "delta")?,
                tracks: tracks(v)?,
                linked: flag(v, "linked")?,
            }
            .into(),
            "roll" => Edit::Roll {
                left_id: text(v, "leftId")?,
                right_id: text(v, "rightId")?,
                delta: num(v, "delta")?,
                linked: flag(v, "linked")?,
            }
            .into(),
            "slide" => Edit::Slide {
                previous_id: text(v, "previousId")?,
                layer_id: text(v, "layerId")?,
                next_id: text(v, "nextId")?,
                delta: num(v, "delta")?,
                linked: flag(v, "linked")?,
            }
            .into(),
            "link" => Link::Link {
                video_id: text(v, "videoId")?,
                audio_id: text(v, "audioId")?,
                link_id: text(v, "linkId")?,
            }
            .into(),
            "unlink" => {
                fields(v, &["op", "layerIds"])?;
                Link::Unlink { layer_ids: ids(v)? }.into()
            }
            "select" => {
                return Ok(Self::Select {
                    ids: ids(v)?,
                    bypass: flag(v, "bypass")?,
                    add: flag(v, "add")?,
                });
            }
            "inspect" => return Ok(Self::Inspect { ids: ids(v)? }),
            "undo" => return Ok(Self::Undo),
            "redo" => return Ok(Self::Redo),
            "sourceMarks" => return Ok(Self::SourceMarks(SourceMonitor::decode(v)?)),
            "sourceEdit" => {
                let mode = match text(v, "mode")?.as_str() {
                    "insert" => SourceEditMode::Insert,
                    "overwrite" => SourceEditMode::Overwrite,
                    _ => return Err(invalid("Unknown source edit mode")),
                };
                let targets = array(v, "targets")?
                    .iter()
                    .map(|t| {
                        fields(t, &["media", "track", "layerId"])?;
                        let media = match text(t, "media")?.as_str() {
                            "video" => SourceMedia::Video,
                            "audio" => SourceMedia::Audio,
                            _ => return Err(invalid("Unknown source stream")),
                        };
                        Ok(SourceTarget {
                            media,
                            track: track(t.get("track").ok_or_else(|| invalid("Missing track"))?)?,
                            layer_id: text(t, "layerId")?,
                        })
                    })
                    .collect::<Result<_, CommandError>>()?;
                let splits = array(v, "splits")?
                    .iter()
                    .map(|t| {
                        fields(t, &["layerId", "tailId", "tailLinkId"])?;
                        Ok(SourceSplit {
                            layer_id: text(t, "layerId")?,
                            tail_id: text(t, "tailId")?,
                            tail_link_id: optional_text(t, "tailLinkId")?,
                        })
                    })
                    .collect::<Result<_, CommandError>>()?;
                return Ok(Self::SourceEdit {
                    mode,
                    targets,
                    link_id: optional_text(v, "linkId")?,
                    splits,
                });
            }
            "shuttle" => {
                let input = match text(v, "key")?.as_str() {
                    "j" => ShuttleInput::J,
                    "k" => ShuttleInput::K,
                    "l" => ShuttleInput::L,
                    "reverse" => ShuttleInput::Reverse,
                    // Duration comes from the actor's current composition.
                    "around" => ShuttleInput::PlayAround {
                        before: num(v, "before")?,
                        after: num(v, "after")?,
                        duration: 0.,
                    },
                    _ => return Err(invalid("Unknown shuttle key")),
                };
                if !matches!(input, ShuttleInput::PlayAround { .. }) {
                    fields(v, &["op", "key"])?;
                }
                return Ok(Self::Shuttle(input));
            }
            _ => unreachable!(),
        };
        Ok(Self::Command(c))
    }
}
impl SourceMonitor {
    fn decode(v: &Value) -> Result<Self, CommandError> {
        Ok(Self {
            comp_id: optional_text(v, "compId")?,
            asset_id: text(v, "assetId")?,
            points: EditPoints {
                source_in: optional_num(v, "sourceIn")?,
                source_out: optional_num(v, "sourceOut")?,
                timeline_in: optional_num(v, "timelineIn")?,
                timeline_out: optional_num(v, "timelineOut")?,
                playhead: 0.,
            },
        })
    }
    pub fn value(&self) -> Value {
        let mut m = Map::new();
        m.insert(
            "compId",
            self.comp_id
                .as_deref()
                .map(Value::from)
                .unwrap_or(Value::Null),
        );
        m.insert("assetId", Value::from(self.asset_id.as_str()));
        for (key, n) in [
            ("sourceIn", self.points.source_in),
            ("sourceOut", self.points.source_out),
            ("timelineIn", self.points.timeline_in),
            ("timelineOut", self.points.timeline_out),
        ] {
            m.insert(key, n.map(Value::Number).unwrap_or(Value::Null));
        }
        Value::Object(m)
    }
}
fn comp_facts(root: &pm_model::Node, comp: Option<&str>) -> Result<(f64, f64), KernelFailure> {
    let c =
        crate::model::comp_node(root, comp).ok_or_else(|| error("Composition is unavailable"))?;
    let n = |k| {
        c.get(k)
            .and_then(crate::model::number_of)
            .filter(|n| n.is_finite() && *n > 0.)
            .ok_or_else(|| error("Invalid composition timing"))
    };
    Ok((n("fps")?, n("dur")?))
}
/// Called only on the actor; converting to existing messages preserves the
/// existing journal → publication → acknowledgement → events boundary.
pub(crate) fn prepare(
    kernel: &Kernel,
    project: &str,
    r: ProRequest,
    clock_binding: Option<u64>,
) -> Result<Prepared, KernelFailure> {
    let clock_available = clock_binding.is_some();
    let p = kernel
        .project(project)
        .ok_or_else(|| KernelFailure::NoProject(project.into()))?;
    let store = p.store();
    if store.doc_id() != r.document {
        return Err(error("Pro edit belongs to a different document lifetime"));
    }
    // Native adapters only. Legacy realm behavior remains on its existing API.
    if r.caller.writer != Caller::native().writer {
        return Err(error("Pro adapter requires a native caller"));
    }
    let root = store.root().ok_or_else(|| error("No project tree"))?;
    let state = kernel.state();
    let mut ui = state.ui.get(project).cloned().unwrap_or_default();
    let make_call = |call| {
        let mut request = Request::new(r.caller, call);
        request.txid = r.txid;
        Prepared::Call(request)
    };
    let transact = |c: Command| {
        make_call(Call::Transact {
            label: None,
            body: c.into(),
            expected_revision: Some(r.revision),
        })
    };
    match r.action {
        ProAction::Command(c) => return Ok(transact(c)),
        ProAction::Undo | ProAction::Redo => {
            if r.revision != store.revision() {
                return Err(error("Pro history revision changed"));
            }
            return Ok(make_call(if matches!(r.action, ProAction::Undo) {
                crate::HistoryCall::Undo.into()
            } else {
                crate::HistoryCall::Redo.into()
            }));
        }
        ProAction::Select { ids, bypass, add } => {
            if r.revision != store.revision() {
                return Err(error("Pro selection revision changed"));
            }
            let ids = pro_links::linked_selection(root, &ids, bypass)
                .map_err(|e| error(e.to_string()))?;
            return Ok(make_call(SelectionCall::Layers { ids, add }.into()));
        }
        _ => (),
    }
    if r.txid.is_some() {
        return Err(error(
            "State-dependent pro actions require re-inspection on retry",
        ));
    }
    if let ProAction::Inspect { ids } = r.action {
        let mut result = Map::new();
        result.insert("documentId", Value::from(r.document.to_string()));
        result.insert("revision", Value::from(store.revision().to_string()));
        result.insert("stateRevision", Value::from(state.revision.to_string()));
        result.insert(
            "sourceMonitor",
            ui.session.get("proSource").cloned().unwrap_or(Value::Null),
        );
        result.insert("transport", ui.transport.clone());
        result.insert("clockAvailable", Value::Bool(clock_available));
        let ids = if ids.is_empty() {
            let comp = crate::model::open_comp_id(root);
            crate::model::comp_node(root, comp.as_deref())
                .map(|c| {
                    crate::model::layers_of(c)
                        .filter_map(|l| {
                            l.get("id")
                                .and_then(crate::model::str_of)
                                .map(str::to_owned)
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            ids
        };
        let mut layers = Vec::new();
        for id in &ids {
            let loc = crate::model::find_layer(root, id)
                .ok_or_else(|| error(format!("Unknown layer {id}")))?;
            layers.push(
                crate::model::layer_at(root, &loc)
                    .ok_or_else(|| error("Layer disappeared"))?
                    .to_value(),
            );
        }
        result.insert("layers", Value::Array(layers));
        result.insert(
            "assetMetadata",
            root.get("assets")
                .map(pm_model::Node::to_value)
                .unwrap_or(Value::Null),
        );
        result.insert(
            "selection",
            Value::Array(
                store
                    .selection()
                    .layers()
                    .iter()
                    .cloned()
                    .map(Value::from)
                    .collect(),
            ),
        );
        let mut badges = Vec::new();
        let mut diagnostics = Vec::new();
        for id in ids {
            let found = match pro_links::sync_badges(root, &id) {
                Ok(b) => b,
                Err(e) => {
                    let mut m = Map::new();
                    m.insert("layerId", Value::from(id.as_str()));
                    m.insert("error", Value::from(e.to_string()));
                    diagnostics.push(Value::Object(m));
                    continue;
                }
            };
            for b in found {
                if badges
                    .iter()
                    .any(|v: &Value| v.get("layerId").and_then(Value::as_str) == Some(&b.layer_id))
                {
                    continue;
                }
                let mut m = Map::new();
                m.insert("layerId", Value::from(b.layer_id.as_str()));
                m.insert("peerId", Value::from(b.peer_id.as_str()));
                m.insert("offsetSeconds", Value::Number(b.offset_seconds));
                badges.push(Value::Object(m));
            }
        }
        result.insert("syncBadges", Value::Array(badges));
        result.insert("syncDiagnostics", Value::Array(diagnostics));
        return Ok(Prepared::Inspection(Value::Object(result)));
    }
    if r.revision != store.revision() {
        return Err(error("Pro edit revision changed"));
    }
    if r.state_revision != Some(state.revision) {
        return Err(error("Pro state revision changed; inspect before retrying"));
    }
    let bad = |e: CommandError| error(e.to_string());
    match r.action {
        ProAction::SourceMarks(mut monitor) => {
            if monitor.comp_id.is_none() {
                monitor.comp_id = crate::model::open_comp_id(root);
            }
            let (_, duration) = comp_facts(root, monitor.comp_id.as_deref())?;
            let asset = root
                .get("assets")
                .and_then(|a| a.get(&monitor.asset_id))
                .ok_or_else(|| error("Source asset is unavailable"))?;
            let source_duration = asset
                .get("dur")
                .and_then(crate::model::number_of)
                .filter(|d| d.is_finite() && *d > 0.)
                .ok_or_else(|| error("Source duration is unavailable"))?;
            for (a, b, limit) in [
                (
                    monitor.points.source_in,
                    monitor.points.source_out,
                    source_duration,
                ),
                (
                    monitor.points.timeline_in,
                    monitor.points.timeline_out,
                    duration,
                ),
            ] {
                if [a, b]
                    .into_iter()
                    .flatten()
                    .any(|n| !n.is_finite() || n < 0. || n > limit)
                    || a.zip(b).is_some_and(|(a, b)| b <= a)
                {
                    return Err(error("Invalid source-monitor marks"));
                }
            }
            if ui
                .session
                .get("proSource")
                .is_some_and(|v| !matches!(v, Value::Null) && v.as_object().is_none())
            {
                return Err(error(
                    "Malformed source-monitor facts; preserve and repair explicitly",
                ));
            }
            let mut stored = ui
                .session
                .get("proSource")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            for (k, v) in monitor.value().as_object().expect("monitor object") {
                stored.insert(k.clone(), v.clone());
            }
            ui.session
                .as_object_mut()
                .ok_or_else(|| error("Malformed session facts"))?
                .insert("proSource", Value::Object(stored));
        }
        ProAction::SourceEdit {
            mode,
            targets,
            link_id,
            splits,
        } => {
            let mut monitor = SourceMonitor::decode(
                ui.session
                    .get("proSource")
                    .ok_or_else(|| error("Set source-monitor marks first"))?,
            )
            .map_err(bad)?;
            if monitor.comp_id != crate::model::open_comp_id(root) {
                return Err(error("Source-monitor composition is no longer active"));
            }
            monitor.points.playhead = num(&ui.transport, "time").map_err(bad)?;
            let end = monitor.points.resolve().map_err(bad)?.timeline_out;
            // Opus N006: successful source edits leave the playhead at the end
            // and clear source marks. Preserve timeline marks and opaque facts.
            ui.transport
                .as_object_mut()
                .ok_or_else(|| error("Malformed transport facts"))?
                .insert("time", Value::Number(end));
            let marks = ui
                .session
                .as_object_mut()
                .ok_or_else(|| error("Malformed session facts"))?
                .get_mut("proSource")
                .and_then(Value::as_object_mut)
                .ok_or_else(|| error("Malformed source-monitor facts"))?;
            marks.insert("sourceIn", Value::Null);
            marks.insert("sourceOut", Value::Null);
            let Prepared::Call(request) = transact(
                ProSourceCommand {
                    comp_id: monitor.comp_id,
                    asset_id: monitor.asset_id,
                    points: monitor.points,
                    mode,
                    targets,
                    link_id,
                    splits,
                }
                .into(),
            ) else {
                unreachable!()
            };
            return Ok(Prepared::SourceEdit {
                request,
                finish: SourceFinish {
                    state: StateTransaction {
                        expected_revision: r.state_revision,
                        ops: vec![StateOp::Ui {
                            project: project.into(),
                            next: ui,
                        }],
                    },
                },
            });
        }
        ProAction::Shuttle(input) => {
            let comp = crate::model::open_comp_id(root);
            let (fps, duration) = comp_facts(root, comp.as_deref())?;
            let mut s =
                ShuttleState::from_transport(&ui.transport, clock_available, fps).map_err(bad)?;
            let input = match input {
                ShuttleInput::PlayAround { before, after, .. } => ShuttleInput::PlayAround {
                    before,
                    after,
                    duration,
                },
                other => other,
            };
            s = match s.dispatch(input).map_err(bad)? {
                ShuttleOutcome::Declined => return Ok(Prepared::Declined),
                ShuttleOutcome::Handled(s) => s,
            };
            ui.transport = s.transport_value(&ui.transport).map_err(bad)?;
        }
        ProAction::ObserveClock { binding, time } => {
            if clock_binding != Some(binding) {
                return Err(error("Native clock is unavailable"));
            }
            let comp = crate::model::open_comp_id(root);
            let (fps, duration) = comp_facts(root, comp.as_deref())?;
            let s = ShuttleState::from_transport(&ui.transport, true, fps)
                .map_err(bad)?
                .observe_clock(time, duration)
                .map_err(bad)?;
            ui.transport = s.transport_value(&ui.transport).map_err(bad)?;
        }
        _ => unreachable!(),
    }
    Ok(Prepared::State(StateTransaction {
        expected_revision: r.state_revision,
        ops: vec![StateOp::Ui {
            project: project.into(),
            next: ui,
        }],
    }))
}

/// Only after a successful document apply, and before actor publication/reply.
/// Failure is an applied-with-notice outcome, never a false command refusal.
pub(crate) fn finish_source(
    kernel: &mut Kernel,
    project: &str,
    finish: SourceFinish,
    handled: &mut crate::tx::Handled,
) {
    let Some(ack) = handled.reply.as_mut() else {
        return;
    };
    if !matches!(ack.reply, crate::Reply::Applied { changed: true, .. }) {
        return;
    }
    if let Err(e) = kernel.state_transaction(finish.state) {
        let message = format!("Source edit applied, but session facts could not be updated: {e}");
        let notice = crate::Notice::NotDurable { message };
        let diagnostic = match ack.notice.take() {
            Some(crate::Notice::NotDurable { message: earlier }) => format!("{earlier}; {e}"),
            Some(earlier) => format!("{earlier:?}; {e}"),
            None => e.to_string(),
        };
        ack.notice = Some(crate::Notice::NotDurable {
            message: format!(
                "Source edit applied, but session facts could not be updated: {diagnostic}"
            ),
        });
        if let Some(p) = kernel.project_mut(project) {
            p.push_notice(notice);
        }
    }
}
