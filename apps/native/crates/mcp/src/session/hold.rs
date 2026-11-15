//! A suspended MCP write owns its session on a worker until release. The
//! provider gets a held acknowledgement after four seconds, never a retry.
use super::*;
use pm_model::patch::PathPart;
use std::{
    sync::{atomic::AtomicBool, mpsc},
    time::{Duration, Instant},
};
pub(super) struct PendingHold {
    receive: mpsc::Receiver<(Option<Active>, Result<Value>)>,
    cancel: Arc<AtomicBool>,
}
fn overlaps(a: &[PathPart], b: &[PathPart]) -> bool {
    a.starts_with(b) || b.starts_with(a)
}
fn changed(before: &pm_kernel::Snapshot, after: &pm_kernel::Snapshot) -> Vec<Vec<PathPart>> {
    pm_model::ops::diff_node_ops(before.root(), after.root())
        .iter()
        .map(|op| op.path().to_vec())
        .collect()
}
fn order() -> &'static Mutex<std::collections::VecDeque<(String, u64)>> {
    static ORDER: std::sync::OnceLock<Mutex<std::collections::VecDeque<(String, u64)>>> =
        std::sync::OnceLock::new();
    ORDER.get_or_init(Default::default)
}
struct Ticket {
    project: String,
    id: u64,
}
impl Ticket {
    fn new(project: &str, document: u64) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let project = format!("{project}/{document}");
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        order().lock().unwrap().push_back((project.clone(), id));
        Self { project, id }
    }
    fn queued(project: &str, document: u64) -> bool {
        let key = format!("{project}/{document}");
        order().lock().unwrap().iter().any(|(p, _)| p == &key)
    }
    fn first(&self) -> bool {
        order()
            .lock()
            .unwrap()
            .iter()
            .find(|(project, _)| project == &self.project)
            .is_some_and(|(_, id)| *id == self.id)
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        order().lock().unwrap().retain(|(_, id)| *id != self.id);
    }
}
impl McpSession {
    pub(super) fn take_hold(&mut self, wait: bool) -> Result<bool> {
        let Some(pending) = self.pending.take() else {
            return Ok(true);
        };
        let result = if wait {
            pending
                .receive
                .recv_timeout(Duration::from_millis(crate::coedit::TOOL_WAIT_MS))
                .map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => mpsc::TryRecvError::Empty,
                    mpsc::RecvTimeoutError::Disconnected => mpsc::TryRecvError::Disconnected,
                })
        } else {
            pending.receive.try_recv()
        };
        match result {
            Ok((active, result)) => {
                self.active = active;
                self.hold_note = Some(match result {
                    Ok(value) => value,
                    Err(error) => {
                        json!({"status":"dropped","reason":error.message,"error":error.code})
                    }
                });
                Ok(true)
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.pending = Some(pending);
                Ok(false)
            }
            Err(_) => Err(Error::unavailable("held MCP write worker stopped")),
        }
    }
    pub(super) fn cancel_hold(&mut self) {
        if let Some(pending) = &self.pending {
            pending.cancel.store(true, Ordering::Release);
        }
        let _ = self.take_hold(true);
    }
    pub(super) fn defer_if_touched(&mut self, name: &str, args: &Value) -> Result<Option<Value>> {
        // Legacy external MCP connections retain their synchronous folding
        // contract. Co-edit admission belongs to an explicitly bound run.
        if self.agent_token.is_none()
            || self.bypass_hold
            || ![
                "apply_commands",
                "edit_layer_strips",
                "edit_video",
                "edit_3d",
            ]
            .contains(&name)
        {
            return Ok(None);
        }
        let Some(active) = self.active.as_mut() else {
            return Ok(None);
        };
        let Some(project) = active.project.clone() else {
            return Ok(None);
        };
        let (host, baseline, scopes) = {
            let automation = self
                .host
                .lock()
                .map_err(|_| Error::unavailable("MCP host stopped"))?;
            let view = automation.view(&project)?;
            observe_foreign(active, &automation, &project);
            let touching = automation.coedit_host().is_some_and(|h| {
                h.kernel
                    .published(&project)
                    .is_some_and(|p| p.entry.is_some())
            }) || crate::coedit::typing(&project)
                || Ticket::queued(&project, view.snapshot.doc_id);
            if !touching && active.owned_scopes.is_empty() {
                return Ok(None);
            }

            for command in args["commands"].as_array().into_iter().flatten() {
                let target = command["target"]
                    .as_str()
                    .filter(|t| !t.starts_with('$'))
                    .or_else(|| active.focus.first().map(String::as_str));
                if let Some(target) = target {
                    if active.known_layers.contains(target)
                        && view
                            .snapshot
                            .root()
                            .is_none_or(|r| pm_kernel::model::find_layer(r, target).is_none())
                    {
                        return Err(Error {
                            code: "target_removed".into(),
                            message: format!(
                                "The user deleted {target}. Skip it, do not recreate it, and mention it in your reply."
                            ),
                            backing: None,
                        });
                    }
                }
            }
            let scopes = if name == "apply_commands" {
                let cleaned = commands::clean_commands(
                    args["commands"]
                        .as_array()
                        .map(Vec::as_slice)
                        .unwrap_or_default(),
                )?;
                let batch = commands::plan_snapshot(
                    &view.snapshot,
                    &active.focus,
                    &cleaned,
                    active.time,
                    Some(&automation.state()),
                )?;
                let mut scratch = pm_kernel::DocumentStore::scratch(&view.snapshot);
                scratch
                    .transact("Held write", Origin::Mcp, &batch.commands)
                    .map_err(Error::kernel)?;
                scratch
                    .history()
                    .peek_undo()
                    .map(|r| r.forward.iter().map(|op| op.path().to_vec()).collect())
                    .unwrap_or_default()
            } else {
                vec![Vec::new()]
            };
            let epoch = crate::coedit::steer_epoch(&project);
            let explicit_batch = args["commands"].as_array().is_some_and(|commands| {
                !commands.is_empty() && commands.iter().all(|c| c["preserveHandEdits"] == false)
            });
            if epoch > active.steer_epoch && explicit_batch {
                active
                    .owned_scopes
                    .retain(|owned| !scopes.iter().any(|path| overlaps(path, owned)));
                active.steer_epoch = epoch;
            }
            if scopes.iter().any(|path| {
                active
                    .owned_scopes
                    .iter()
                    .any(|owned| overlaps(path, owned))
            }) {
                return Ok(Some(
                    json!({"error":"user_owned","status":"dropped","reason":"The user owns this value for this turn. A fresh human steer must explicitly ask to change it; then send only those requested values with preserveHandEdits:false."}),
                ));
            }
            let Some(host) = automation.coedit_host() else {
                return Ok(None);
            };
            let touching = host
                .kernel
                .published(&project)
                .is_some_and(|p| p.entry.is_some())
                || crate::coedit::typing(&project)
                || Ticket::queued(&project, view.snapshot.doc_id);
            if !touching {
                return Ok(None);
            }
            (host, view.snapshot, scopes)
        };
        let mut worker = McpSession {
            host: self.host.clone(),
            active: self.active.take(),
            agent_token: self.agent_token.clone(),
            writer: self.writer,
            closed: false,
            pending: None,
            bypass_hold: true,
            hold_note: None,
        };
        let (send, receive) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let name = name.to_owned();
        let args = args.clone();
        host.publish_presence(crate::presence::Presence {
            project: project.clone(),
            run: worker.active.as_ref().unwrap().run.clone(),
            kind: crate::presence::Kind::Held,
            layer: worker.active.as_ref().unwrap().focus.last().cloned(),
            property: None,
            times: Vec::new(),
        });
        let ticket = Ticket::new(&project, baseline.doc_id);
        std::thread::spawn(move || {
            let ticket = ticket;
            let events = host.kernel.subscribe();
            let result = (|| {
                let waiting_since = Instant::now();
                let mut released: Option<Instant> = None;
                loop {
                    if waiting_since.elapsed() >= Duration::from_millis(crate::coedit::MAX_HOLD_MS)
                    {
                        return Ok(
                            json!({"status":"dropped","reason":"The user kept editing beyond the hold limit. This write expired; do not retry it."}),
                        );
                    }
                    if stop.load(Ordering::Acquire) || worker.run_revoked() {
                        return Ok(
                            json!({"status":"dropped","reason":"The run stopped before this write landed."}),
                        );
                    }
                    let publication = host.kernel.published(&project).ok_or_else(|| {
                        Error::conflict("The project closed while the write waited.")
                    })?;
                    if publication.snapshot.doc_id != baseline.doc_id {
                        return Err(Error::conflict(
                            "The project reopened while the write waited.",
                        ));
                    }
                    if !ticket.first()
                        || publication.entry.is_some()
                        || crate::coedit::typing(&project)
                    {
                        released = None;
                    } else {
                        let start = released.get_or_insert_with(Instant::now);
                        if start.elapsed() >= Duration::from_millis(crate::coedit::RELEASE_DELAY_MS)
                        {
                            break;
                        }
                    }
                    let _ = events.recv_timeout(Duration::from_millis(50));
                }
                if !host.wait_if_paused(&project, || {
                    !stop.load(Ordering::Acquire) && !worker.run_revoked()
                }) {
                    return Ok(
                        json!({"status":"dropped","reason":"The run stopped before this write landed."}),
                    );
                }
                let mut automation = worker
                    .host
                    .lock()
                    .map_err(|_| Error::unavailable("MCP host stopped"))?;
                let current = automation.view(&project)?.snapshot;
                let user = changed(&baseline, &current);
                let contested: Vec<_> = scopes
                    .iter()
                    .filter(|path| user.iter().any(|u| overlaps(path, u)))
                    .cloned()
                    .collect();
                let active = worker.active.as_mut().unwrap();
                for target in active.focus.iter().map(String::as_str).chain(
                    args["commands"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|c| c["target"].as_str()),
                ) {
                    if baseline
                        .root()
                        .is_some_and(|r| pm_kernel::model::find_layer(r, target).is_some())
                        && current
                            .root()
                            .is_none_or(|r| pm_kernel::model::find_layer(r, target).is_none())
                    {
                        return Err(Error {
                            code: "target_removed".into(),
                            message: format!(
                                "The user deleted {target}. Skip it and mention the skipped target in your reply; do not recreate it unless asked."
                            ),
                            backing: None,
                        });
                    }
                }
                active.owned_scopes.extend(user);
                refresh(active, &mut automation, &project, worker.writer)?;
                let mut landing_args = args.clone();
                let dropped = !contested.is_empty();
                if dropped {
                    if name != "apply_commands" {
                        return Ok(
                            json!({"status":"dropped","reason":"The user changed a value while this transaction waited. Their value is kept."}),
                        );
                    }
                    let mut remaining = Vec::new();
                    for command in args["commands"].as_array().into_iter().flatten() {
                        let cleaned = commands::clean_commands(std::slice::from_ref(command))?;
                        let planned = commands::plan_snapshot(
                            &baseline,
                            &active.focus,
                            &cleaned,
                            active.time,
                            Some(&automation.state()),
                        );
                        let Ok(planned) = planned else {
                            return Ok(
                                json!({"status":"dropped","reason":"A dependent write cannot safely land by itself after the user's change."}),
                            );
                        };
                        let mut scratch = pm_kernel::DocumentStore::scratch(&baseline);
                        scratch
                            .transact("Held command", Origin::Mcp, &planned.commands)
                            .map_err(Error::kernel)?;
                        let paths: Vec<_> = scratch
                            .history()
                            .peek_undo()
                            .map(|r| r.forward.iter().map(|op| op.path().to_vec()).collect())
                            .unwrap_or_default();
                        if !paths
                            .iter()
                            .any(|path| contested.iter().any(|owned| overlaps(path, owned)))
                        {
                            remaining.push(command.clone());
                        }
                    }
                    if remaining.is_empty() {
                        return Ok(
                            json!({"status":"dropped","reason":"The user changed this value while it waited. Their value is kept and user-owned for this turn."}),
                        );
                    }
                    landing_args["commands"] = json!(remaining);
                }
                drop(automation);
                let mut result = worker.dispatch_inner(&name, &landing_args)?;
                result["status"] = json!(if dropped { "dropped" } else { "applied" });
                if dropped {
                    result["appliedRemainder"] = json!(true);
                    result["reason"] = json!("Kept the user's values; independent writes landed.");
                }
                Ok(result)
            })();
            if let Some(active) = &worker.active {
                host.publish_presence(crate::presence::Presence {
                    project: project.clone(),
                    run: active.run.clone(),
                    kind: if matches!(&result,Ok(v) if v["status"]=="applied") {
                        crate::presence::Kind::Landed
                    } else {
                        crate::presence::Kind::Dropped
                    },
                    layer: active.focus.last().cloned(),
                    property: None,
                    times: Vec::new(),
                });
            }
            let active = worker.active.take();
            if let Err(mpsc::SendError((active, _))) = send.send((active, result)) {
                worker.active = active;
                let _ = worker.finish(true);
            }
            worker.closed = true;
        });
        self.pending = Some(PendingHold { receive, cancel });
        // The first result waits up to four seconds. Later calls report held
        // until the worker's single queued write resolves.
        let pending = self.pending.take().unwrap();
        match pending
            .receive
            .recv_timeout(Duration::from_millis(crate::coedit::TOOL_WAIT_MS))
        {
            Ok((active, result)) => {
                self.active = active;
                result.map(Some)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.pending = Some(pending);
                Ok(Some(
                    json!({"status":"held","reason":"The user is editing this value. Your change will resolve when they finish; do not retry it."}),
                ))
            }
            Err(_) => Err(Error::unavailable("held write worker stopped")),
        }
    }
}
