//! The eval runner owns this stdio connection. Reads stay responsive while an
//! Aero turn is in flight so its cancel notification can reach the same Handle
//! as the Agent panel's Stop action.
use crate::{
    Automation, Error, McpSession, Result, failure,
    session::aero_tools::Tools,
    transport::{Rpc, ToolConnection, line, write_json},
};
use pm_agents::{
    Access, Provider,
    aero::{self, Model, Options},
    launch::Request,
    transport::{Action, Handle},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{BufRead, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

struct Connection {
    session: McpSession,
    host: Arc<Mutex<Automation>>,
    replay: Option<Model>,
    opened: Option<String>,
    profile: PathBuf,
}

impl ToolConnection for Connection {
    fn list(&mut self) -> Result<Value> {
        let mut tools = self.session.tools().as_array().cloned().unwrap_or_default();
        // A stock headless host answers these with "unavailable", so omit
        // them from the eval discovery answer.
        let host = self
            .host
            .lock()
            .map_err(|_| Error::unavailable("eval host"))?;
        tools.retain(|tool| match tool["name"].as_str() {
            Some("render_frames") => host.engine.is_some(),
            Some("open_panel" | "get_panel_state" | "interact_panel" | "capture_panel") => {
                host.surfaces.is_some()
            }
            _ => true,
        });
        if host.engine.is_some() {
            tools.push(json!({"name":"measure_frame_time","description":"Time paired renders with one target enabled and disabled.",
                "inputSchema":{"type":"object","required":["target","times","width","samples"]}}));
        }
        if host.coedit_host().is_some() {
            tools.push(json!({"name":"set_playhead","description":"Seek the eval project.",
                "inputSchema":{"type":"object","required":["time"],"properties":{"time":{"type":"number"}}}}));
        }
        tools.push(json!({"name":"evaluate_rubric","description":"Score supplied evidence in a fresh replay context.",
            "inputSchema":{"type":"object","required":["rubric","prompt","sees"],"properties":{"rubric":{},"prompt":{"type":"string"},"sees":{}}}}));
        Ok(json!(tools))
    }

    fn invoke(&mut self, name: &str, args: Value) -> Result<Value> {
        if name == "measure_frame_time" {
            let result = (|| {
                let host = self
                    .host
                    .lock()
                    .map_err(|_| Error::unavailable("eval host"))?;
                let project = self
                    .opened
                    .as_deref()
                    .ok_or_else(|| Error::invalid("Open an eval project first."))?;
                let view = host.view(project)?;
                let engine = host
                    .engine
                    .clone()
                    .ok_or_else(|| Error::unavailable("paired frame timing"))?;
                drop(host);
                engine.measure_frame_time(project, view.document, view.revision, &args)
            })();
            return Ok(result.map_or_else(failure, crate::success));
        }
        if name == "set_playhead" {
            let result = (|| {
                let time = args["time"]
                    .as_f64()
                    .filter(|time| time.is_finite() && *time >= 0.)
                    .ok_or_else(|| Error::invalid("Choose a finite nonnegative playhead time."))?;
                let host = self
                    .host
                    .lock()
                    .map_err(|_| Error::unavailable("eval host"))?;
                let project = host
                    .active_project()
                    .ok_or_else(|| Error::invalid("Open an eval project first."))?;
                let app = host
                    .coedit_host()
                    .ok_or_else(|| Error::unavailable("native eval playhead"))?;
                let mut ui = host.state().ui.get(&project).cloned().unwrap_or_default();
                let mut transport = ui.transport.as_object().cloned().unwrap_or_default();
                transport.insert("time", pm_model::Value::Number(time));
                transport.insert("playing", pm_model::Value::Bool(false));
                ui.transport = pm_model::Value::Object(transport);
                app.kernel
                    .state_transaction(pm_kernel::state::StateTransaction {
                        ops: vec![pm_kernel::state::StateOp::Ui { project, next: ui }],
                        ..Default::default()
                    })
                    .wait()
                    .ok_or_else(|| Error::unavailable("kernel actor"))?
                    .map_err(Error::kernel)?;
                self.session.note_eval_playhead(time);
                Ok(json!({"time":time}))
            })();
            return Ok(result.map_or_else(failure, crate::success));
        }
        if name == "get_workspace_state" {
            let base = self.session.call(name, args);
            let enriched = (|| {
                let host = self
                    .host
                    .lock()
                    .map_err(|_| Error::unavailable("eval host"))?;
                let Some(extensions) = host.extensions.clone() else {
                    return Ok(None);
                };
                let project = host
                    .active_project()
                    .ok_or_else(|| Error::invalid("Open an eval project first."))?;
                let view = host.view(&project)?;
                let engine = host.engine.clone();
                let mut state = host.workspace_state(Some(&project))?;
                state["openProjects"] = json!([{"id":project,"active":true}]);
                drop(host);
                if let Some(engine) = engine {
                    engine.prepare_definitions(&project, view.document, view.revision)?;
                    if let Some(fields) = engine
                        .eval_registrations(&project, view.document, view.revision)?
                        .as_object()
                    {
                        for (key, value) in fields {
                            state[key] = value.clone();
                        }
                    }
                }
                state["extensions"] = json!(extensions.health()?);
                Ok(Some(state))
            })();
            return Ok(match enriched {
                Ok(Some(state)) => crate::success(state),
                Ok(None) => base,
                Err(error) => failure(error),
            });
        }
        if name != "evaluate_rubric" {
            let opened = (name == "open_project")
                .then(|| args["projectId"].as_str().map(str::to_owned))
                .flatten();
            let result = self.session.call(name, args);
            if let Some(opened) = opened
                && result["isError"] == false
            {
                let opened_value = result["content"][0]["text"]
                    .as_str()
                    .and_then(|text| serde_json::from_str::<Value>(text).ok());
                if !opened_value.is_some_and(|value| value["recoveryRequired"] == true) {
                    self.opened = Some(opened);
                }
            }
            return Ok(result);
        }
        let result = (|| {
            let model = self.replay.as_ref().ok_or_else(|| {
                Error::unavailable("fresh evaluator with a configured replay stream")
            })?;
            let text = aero::evaluate(model, &self.profile, &args.to_string(), &[])
                .map_err(Error::invalid)?;
            serde_json::from_str(&text)
                .map_err(|e| Error::invalid(format!("Evaluator returned invalid JSON: {e}")))
        })();
        Ok(match result {
            Ok(value) => crate::success(value),
            Err(error) => failure(error),
        })
    }

    fn disconnect(&mut self) -> Result<()> {
        self.session.close()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Run {
    run_id: String,
    project_id: String,
    revision: Option<u64>,
    prompt: String,
    model: String,
    effort: String,
    wall_secs: u64,
    max_steps: u32,
    max_gate_continuations: u32,
    provider: Replay,
    #[serde(default)]
    ablate: Vec<String>,
}

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase", deny_unknown_fields)]
enum Replay {
    Replay {
        #[serde(rename = "baseUrl")]
        base_url: String,
    },
    None,
    Live,
}

struct Pending {
    id: Value,
    run: String,
    handle: Handle,
    notices: mpsc::Receiver<Value>,
    sequence: u64,
}

impl Pending {
    fn cancel(&self) {
        let _ = self.handle.send(Action::Cancel(self.run.clone()));
    }

    fn drain(&mut self, writer: &mut impl Write) -> Result<bool> {
        // Drain harness delivery so its UI event mailbox cannot back up.
        let mut disconnected = false;
        loop {
            match self.handle.try_event() {
                Ok(_) => {}
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        for event in self.notices.try_iter() {
            if let Some(result) = event.get("result") {
                write_json(
                    writer,
                    &json!({"jsonrpc":"2.0","id":self.id,"result":result}),
                )?;
                return Ok(true);
            }
            write_json(
                writer,
                &json!({"jsonrpc":"2.0","method":"aero/event",
                "params":{"runId":self.run,"seq":self.sequence,"event":event}}),
            )?;
            self.sequence += 1;
        }
        if disconnected {
            write_json(
                writer,
                &rpc_error(&self.id, -32603, "Aero's worker exited without a result."),
            )?;
        }
        Ok(disconnected)
    }
}

fn start(
    request: Run,
    id: Value,
    host: &Arc<Mutex<Automation>>,
    profile: &Path,
) -> Result<(Pending, Model)> {
    if request.run_id.is_empty()
        || request.run_id.len() > 120
        || !request
            .run_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        || request.wall_secs == 0
        || request.wall_secs > 3600
        || request.max_steps == 0
        || request.max_steps > 4096
        || request.max_gate_continuations > 32
    {
        return Err(Error::invalid(
            "Supply a plain run id and bounded, positive wall-time and step limits.",
        ));
    }
    let model = match request.provider {
        Replay::Replay { base_url } => Model::replay(&base_url).map_err(Error::invalid)?,
        _ => {
            return Err(Error::invalid(
                "Aero eval requires an explicit recorded replay provider.",
            ));
        }
    };
    let skip = |key: &str| request.ablate.iter().any(|value| value == key);
    if request.ablate.iter().any(|value| {
        !["completion-gate", "topics", "evaluator", "self-check"].contains(&value.as_str())
    }) {
        return Err(Error::invalid("Unknown Aero ablation."));
    }
    let revision = request
        .revision
        .ok_or_else(|| Error::invalid("Supply the revision of the open project."))?;
    {
        let host = host.lock().map_err(|_| Error::unavailable("eval host"))?;
        if host.active_project().as_deref() != Some(&request.project_id)
            || host.revision(&request.project_id)? != revision
        {
            return Err(Error::conflict(
                "Aero eval must name the project opened on this host and its current revision.",
            ));
        }
    }
    let (notes, notices) = mpsc::channel();
    let options = Options {
        model: model.clone(),
        wall: Duration::from_secs(request.wall_secs),
        steps: request.max_steps,
        continuations: request.max_gate_continuations,
        skip_gate: skip("completion-gate"),
        skip_topics: skip("topics"),
        skip_review: skip("evaluator"),
        report: Some(Arc::new(move |value| {
            let _ = notes.send(value);
        })),
    };
    let skip_self_check = skip("self-check");
    let run = request.run_id;
    let scope = profile.join("aero").join("sessions").join(&run);
    let project = request.project_id;
    let host = host.clone();
    let handle = aero::start(
        Request {
            run: run.clone(),
            prompt: request.prompt,
            provider: Provider::Aero,
            model: Some(request.model),
            effort: Some(request.effort),
            access: Access::Project,
            approval: Default::default(),
            computer_consent: false,
            workspace: scope.clone(),
            images: Vec::new(),
            role: None,
            files: Vec::new(),
            project: Some(project.clone()),
        },
        run.clone(),
        scope,
        options,
        move |scope| {
            let mut tools =
                Tools::open(host, &project, Some(revision), scope, Access::Project, None)
                    .map_err(|e| e.to_string())?;
            tools.check_edits = !skip_self_check;
            Ok(Box::new(tools))
        },
    );
    Ok((
        Pending {
            id,
            run,
            handle,
            notices,
            sequence: 0,
        },
        model,
    ))
}

pub fn serve(
    reader: impl BufRead + Send + 'static,
    mut writer: impl Write,
    host: Arc<Mutex<Automation>>,
    profile: PathBuf,
) -> Result<()> {
    let (input, messages) = mpsc::sync_channel(16);
    std::thread::spawn(move || {
        let mut reader = reader;
        loop {
            let next = line(&mut reader);
            let finished = !matches!(next, Ok(Some(_)));
            if input.send(next).is_err() || finished {
                break;
            }
        }
    });
    let mut rpc = Rpc::new(Connection {
        session: McpSession::new(host.clone()),
        host: host.clone(),
        replay: None,
        opened: None,
        profile: profile.clone(),
    });
    let mut pending: Option<Pending> = None;
    loop {
        if let Some(turn) = pending.as_mut()
            && turn.drain(&mut writer)?
        {
            pending = None;
        }
        let bytes = match messages.recv_timeout(Duration::from_millis(10)) {
            Ok(Ok(Some(bytes))) => bytes,
            Ok(Ok(None)) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(Err(error)) => return Err(error.into()),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
        };
        if bytes.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let message = match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => value,
            Err(_) => {
                write_json(&mut writer, &rpc_error(&Value::Null, -32700, "Parse error"))?;
                continue;
            }
        };
        let id = &message["id"];
        let valid = message["jsonrpc"] == "2.0";
        match (valid, message["method"].as_str()) {
            (true, Some("aero/cancel")) if message.get("id").is_none() => {
                if let Some(turn) = &pending
                    && message["params"]["runId"] == turn.run
                {
                    turn.cancel();
                }
            }
            (true, Some("aero/run")) if id.is_number() || id.is_string() => {
                let started = if !rpc.initialized() || pending.is_some() {
                    Err(Error::invalid(
                        "Initialize the connection and finish its current run first.",
                    ))
                } else {
                    serde_json::from_value(message["params"].clone())
                        .map_err(|e| Error::invalid(format!("Invalid Aero run request: {e}")))
                        .and_then(|request: Run| {
                            if rpc.session_mut().opened.as_deref() != Some(&request.project_id) {
                                return Err(Error::invalid(
                                    "Open the project on this connection before starting Aero.",
                                ));
                            }
                            start(request, id.clone(), &host, &profile)
                        })
                };
                match started {
                    Ok((turn, model)) => {
                        rpc.session_mut().replay = Some(model);
                        pending = Some(turn);
                    }
                    Err(error) => {
                        write_json(&mut writer, &rpc_error(id, -32602, &error.to_string()))?
                    }
                }
            }
            _ if pending.is_some() && message.get("id").is_some() => write_json(
                &mut writer,
                &rpc_error(id, -32000, "An Aero run is in flight."),
            )?,
            _ => {
                let init = message["method"] == "initialize";
                if let Some(mut response) = rpc.handle(message) {
                    if init && response.get("result").is_some() {
                        response["result"]["capabilities"]["experimental"] = json!({
                            // The evaluator needs a model stream, which only an
                            // aero/run supplies, so it is not advertised up front.
                            "powermoveEval": {"evaluator":false,
                                "extensionHealth":host.lock().map(|host| host.extensions.is_some()).unwrap_or(false),
                                "transitions":host.lock().map(|host| host.engine.is_some()).unwrap_or(false)}
                        });
                    }
                    write_json(&mut writer, &response)?;
                }
            }
        }
    }
    if let Some(turn) = pending {
        turn.cancel();
    }
    rpc.close()
}

fn rpc_error(id: &Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
