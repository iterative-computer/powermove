//! Runs on an API or local model (A01): an OpenAI-compatible chat loop with
//! the same Powermove tools other runs get. The tools come from the running
//! app's own tool server (`--powermove-mcp --agent-run <grant>`), spoken to
//! over MCP stdio; the chat streams over HTTP the app supplies (`ChatHost`).
//! Everything here runs on the run's own worker threads.
use crate::{
    Access, Event, RunResult,
    transport::{self, Action, Handle},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fmt::Debug,
    io::{self, BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

/// The saved connection (v1's `agent-provider.json`), without its key.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub models: Vec<String>,
}
impl Saved {
    /// The models a run may choose: the configured one first.
    pub fn choices(&self) -> Vec<String> {
        let mut out = vec![self.model.clone()];
        for id in &self.models {
            if !out.contains(id) {
                out.push(id.clone());
            }
        }
        out
    }
}
/// What the app supplies: the connection, its key and a streamed POST.
pub trait ChatHost: Send + Sync + Debug {
    /// The saved connection; `None` when nothing is connected.
    fn saved(&self) -> Option<Saved>;
    /// The key saved for `endpoint`, read on the run's worker.
    fn key(&self, endpoint: &str) -> Option<String>;
    /// POSTs `body` as JSON to `url` (with `Bearer key`), never following a
    /// redirect off its origin. Response bytes reach `sink` as they arrive, and
    /// only for a 2xx status; an `Err` from `sink` or a set `stop` cancels the
    /// request. Returns the status.
    fn stream(
        &self,
        url: &str,
        key: Option<&str>,
        body: Vec<u8>,
        stop: &AtomicBool,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<u16, String>;
}

/// v1's reasoning efforts for a compatible model id (`modelEfforts`).
pub fn efforts(model: &str) -> Vec<String> {
    let family = |prefixes: &[&str]| {
        prefixes.iter().any(|p| {
            model
                .strip_prefix(p)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
        })
    };
    let list: &[&str] = if family(&["gpt-6-sol", "gpt-6-luna"]) {
        &["none", "low", "medium", "high", "xhigh", "max"]
    } else if family(&["gpt-6-astra", "gpt-5.6"]) {
        &["low", "medium", "high", "xhigh", "max"]
    } else if family(&[
        "gpt-5.2-chat",
        "gpt-5.3-chat",
        "gpt-5.4-chat",
        "gpt-5.5-chat",
    ]) {
        &[]
    } else if family(&["gpt-5.2-pro", "gpt-5.4-pro", "gpt-5.5-pro"]) {
        &["medium", "high", "xhigh"]
    } else if family(&["gpt-5.2", "gpt-5.3", "gpt-5.4", "gpt-5.5"]) {
        &["low", "medium", "high", "xhigh"]
    } else if family(&["o1", "o3", "o4", "gpt-oss"]) {
        &["low", "medium", "high"]
    } else {
        // Compatible services also host models without this parameter.
        &[]
    };
    list.iter().map(|s| (*s).to_owned()).collect()
}
/// The effort sent for `requested` (v1's `modelEffort`): the nearest supported
/// one at or below it, or none when the model takes none.
pub fn effort(model: &str, requested: Option<&str>) -> Option<String> {
    const ALL: [&str; 7] = ["none", "low", "medium", "high", "xhigh", "max", "ultra"];
    let requested = requested?;
    let supported = efforts(model);
    if supported.is_empty() {
        return None;
    }
    if supported.iter().any(|e| e == requested) {
        return Some(requested.into());
    }
    let rank = ALL.iter().position(|e| *e == requested).unwrap_or(0);
    ALL[..=rank]
        .iter()
        .rev()
        .find(|e| supported.iter().any(|s| s == *e))
        .map(|e| (*e).to_owned())
        .or_else(|| supported.first().cloned())
}

/// What one run needs.
pub struct Run {
    pub id: String,
    pub prompt: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub access: Access,
    pub instructions: String,
    /// Attached image files (shown only to a model that reads images).
    pub images: Vec<String>,
    /// The tool server: the app executable and its arguments.
    pub tools: (PathBuf, Vec<String>),
    pub tool_env: BTreeMap<String, String>,
    /// The tools this run may use, by name.
    pub allowed: Vec<String>,
}

const MAX_RESPONSE: usize = 2_000_000;
const TURNS: usize = 160;
const RUN_LIMIT: Duration = Duration::from_secs(60 * 60);

/// Starts the run on its own worker; the handle stops it.
pub fn start(host: Arc<dyn ChatHost>, run: Run) -> Handle {
    let (handle, actions, events) = transport::worker(crate::delivery::Surface::Compatible);
    let stop = Arc::new(AtomicBool::new(false));
    let id = run.id.clone();
    {
        let stop = stop.clone();
        thread::spawn(move || {
            for (action, reply) in actions {
                let accepted = match action {
                    Action::Cancel(run) => run == id,
                    Action::Shutdown => true,
                    _ => false,
                };
                if accepted {
                    stop.store(true, Ordering::Release);
                }
                if let Some(reply) = reply {
                    let _ = reply.try_send(accepted);
                }
            }
            // The handle dropped: nobody is listening any more.
            stop.store(true, Ordering::Release);
        });
    }
    thread::spawn(move || {
        let id = run.id.clone();
        let access = run.access;
        let started = Instant::now();
        let outcome = Loop {
            host,
            stop: stop.clone(),
            events: events.clone(),
            run,
            started,
        }
        .run();
        let result = match outcome {
            Ok(text) => RunResult::success(text, access),
            Err(_) if stop.load(Ordering::Acquire) => RunResult::failure("Stopped.", true),
            Err(_) if started.elapsed() >= RUN_LIMIT => RunResult::failure(
                "The model took too long. Your conversation is kept; try again.",
                false,
            ),
            Err(error) => RunResult::failure(error, false),
        };
        let _ = events.send(Event::Completed { run: id, result });
        let _ = events.send(Event::Stopped("The model run ended.".into()));
    });
    handle
}

struct Loop {
    host: Arc<dyn ChatHost>,
    stop: Arc<AtomicBool>,
    events: SyncSender<Event>,
    run: Run,
    started: Instant,
}
impl Loop {
    fn trace(&self, value: Value) -> Result<(), String> {
        self.events
            .send(Event::Trace {
                run: self.run.id.clone(),
                value,
            })
            .map_err(|_| "The run was closed.".to_owned())
    }
    fn check(&self) -> Result<(), String> {
        if self.stop.load(Ordering::Acquire) {
            return Err("Stopped.".into());
        }
        if self.started.elapsed() >= RUN_LIMIT {
            return Err("timeout".into());
        }
        Ok(())
    }
    fn run(&mut self) -> Result<String, String> {
        let saved = self
            .host
            .saved()
            .ok_or("Connect an API or local model in Settings to start chatting.".to_owned())?;
        let model = match self.run.model.as_deref() {
            None | Some("" | "configured") => saved.model.clone(),
            Some(model) => model.to_owned(),
        };
        if !saved.choices().contains(&model) {
            return Err("This model is no longer available in the saved connection. Test and connect again in Settings.".into());
        }
        let effort = effort(&model, self.run.effort.as_deref());
        let key = self.host.key(&saved.base_url).filter(|k| !k.is_empty());
        let mut tools = Tools::start(&self.run, self.stop.clone())?;
        let specs = tools.list(&self.run.allowed)?;
        let mut user = vec![json!({"type":"text","text":self.run.prompt})];
        if saved.vision {
            for path in &self.run.images {
                let bytes = std::fs::read(path)
                    .map_err(|e| format!("An attached image could not be read: {e}"))?;
                let kind = if bytes.first() == Some(&0xff) {
                    "jpeg"
                } else {
                    "png"
                };
                user.push(json!({"type":"image_url","image_url":{"url":format!("data:image/{kind};base64,{}", STANDARD.encode(bytes))}}));
            }
        }
        let user = if saved.vision {
            Value::Array(user)
        } else {
            json!(self.run.prompt)
        };
        let mut messages = vec![
            json!({"role":"system","content":self.run.instructions}),
            json!({"role":"user","content":user}),
        ];
        let url = format!("{}/chat/completions", saved.base_url.trim_end_matches('/'));
        for _ in 0..TURNS {
            self.check()?;
            let mut body = json!({"model":model,"messages":messages,"stream":true});
            if let Some(effort) = &effort {
                body["reasoning_effort"] = json!(effort);
            }
            if !specs.is_empty() {
                body["tools"] = json!(specs.iter().map(|t| json!({"type":"function","function":{
                    "name":t["name"],"description":t["description"],"parameters":t["inputSchema"]}})).collect::<Vec<_>>());
            }
            let message = self.complete(&url, key.as_deref(), &body)?;
            messages.push(message.assistant());
            if message.calls.is_empty() {
                if message.content.trim().is_empty() {
                    return Err(
                        "The model returned an empty response. Try again or choose another model."
                            .into(),
                    );
                }
                return Ok(message.content);
            }
            let mut images = vec![];
            for call in &message.calls {
                self.check()?;
                if !specs.iter().any(|t| t["name"] == call.name.as_str()) {
                    return Err("The model requested an unavailable tool. Choose a model that supports function calling.".into());
                }
                let args = serde_json::from_str::<Value>(if call.arguments.trim().is_empty() {
                    "{}"
                } else {
                    &call.arguments
                });
                let input = args.as_ref().ok().filter(|v| v.is_object());
                let mut start = json!({"kind":"tool-start","itemId":call.id,
                    "toolName":call.name.to_lowercase(),"label":crate::trace::label(&call.name)});
                if let Some(input) = input {
                    let detail = crate::trace::detail(&call.name, input);
                    if !detail.is_empty() {
                        start["detail"] = json!(detail);
                    }
                    if let Some(subject) = crate::trace::subject(&call.name, input) {
                        start["subject"] = subject;
                    }
                }
                self.trace(start)?;
                let result = match input {
                    Some(input) => tools.call(&call.name, input.clone()),
                    None => Err("Tool arguments must be a JSON object.".into()),
                };
                self.check()?;
                let (ok, content, error) = match result {
                    Ok(result) => (
                        result["isError"] != true,
                        result["content"].as_array().cloned().unwrap_or_default(),
                        None,
                    ),
                    Err(error) => (false, vec![], Some(error)),
                };
                let texts = content
                    .iter()
                    .filter(|c| c["type"] == "text")
                    .cloned()
                    .collect::<Vec<_>>();
                let mut shown = texts.clone();
                if let Some(error) = &error {
                    shown.insert(0, json!({"type":"text","text":error}));
                }
                let mut end = json!({"kind":"tool-end","itemId":call.id,"isError":!ok});
                let output = crate::trace::excerpt(&json!(shown));
                if !output.is_empty() {
                    end["output"] = json!(output);
                }
                self.trace(end)?;
                messages.push(json!({"role":"tool","tool_call_id":call.id,
                    "content":tool_result(ok, texts, error.as_deref())}));
                if saved.vision {
                    images.extend(content.iter().filter(|c| c["type"] == "image").map(|c| {
                        json!({"type":"image_url","image_url":{"url":format!("data:{};base64,{}",
                            c["mimeType"].as_str().unwrap_or("image/png"), c["data"].as_str().unwrap_or(""))}})
                    }));
                }
            }
            // Every tool result follows its assistant batch directly; images
            // between results break compatible chat protocols.
            if !images.is_empty() {
                messages.push(json!({"role":"user","content":images}));
            }
        }
        Err("The model reached the tool limit. Send a follow-up to continue.".into())
    }
    /// One streamed completion.
    fn complete(&self, url: &str, key: Option<&str>, body: &Value) -> Result<Message, String> {
        let mut stream = Stream::default();
        let mut failure = None;
        let status = {
            let events = &self.events;
            let run = &self.run.id;
            let mut sink = |bytes: &[u8]| -> Result<(), String> {
                let texts = stream
                    .push(bytes)
                    .inspect_err(|e| failure = Some(e.clone()))?;
                for text in texts {
                    let text = crate::trace::fragment(&json!(text));
                    if !text.is_empty() {
                        events
                            .send(Event::Trace {
                                run: run.clone(),
                                value: json!({"kind":"answer","text":text}),
                            })
                            .map_err(|_| "The run was closed.".to_owned())?;
                    }
                }
                Ok(())
            };
            self.host.stream(
                url,
                key,
                body.to_string().into_bytes(),
                &self.stop,
                &mut sink,
            )
        };
        if let Some(failure) = failure {
            return Err(failure);
        }
        self.check()?;
        let status = status.map_err(|_| {
            "The model connection failed. Check that the provider is running and try again."
                .to_owned()
        })?;
        if !(200..300).contains(&status) {
            return Err(http_error(status));
        }
        stream.finish()
    }
}
fn http_error(status: u16) -> String {
    match status {
        401 | 403 => "The provider rejected the API key. Reconnect in Settings.".into(),
        429 => {
            "The provider is rate limited or out of credit. Wait a moment, then try again.".into()
        }
        404 => "The API address or model was not found. Check the connection in Settings.".into(),
        _ => format!(
            "The model provider returned HTTP {status}. Your conversation is kept; try again."
        ),
    }
}
/// v1's `modelToolResult`: text content only, and an excerpt past 120,000
/// characters that keeps the success status (so a mutation isn't repeated).
fn tool_result(ok: bool, content: Vec<Value>, error: Option<&str>) -> String {
    let mut envelope = json!({"ok":ok,"content":content});
    if let Some(error) = error {
        envelope["error"] = json!(error);
    }
    let serialized = envelope.to_string();
    if serialized.chars().count() <= 120_000 {
        return serialized;
    }
    let text = content
        .iter()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let excerpt = text.chars().take(8000).collect::<String>();
    envelope["outputTruncated"] = json!(true);
    envelope["content"] = json!([{"type":"text","text":format!("The tool returned more data than fits in one response. Its success/change status above is unchanged; do not repeat a completed mutation. Output is an excerpt, not complete state. For get_project_state discover with indexOnly=true and follow nextLayerOffset, or layerSearch; then read a single layerId with propertyLimit:10/keyframeLimit:0 and page properties/keyframes. For thread_read use smaller limit/maxCharsPerMessage and textOffset. For files use bounded reads.\n\nUNTRUSTED OUTPUT EXCERPT\n{excerpt}")}]);
    envelope.to_string()
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Call {
    pub id: String,
    pub name: String,
    pub arguments: String,
}
#[derive(Debug, Default, PartialEq)]
pub struct Message {
    pub content: String,
    pub calls: Vec<Call>,
}
impl Message {
    fn assistant(&self) -> Value {
        let mut message = json!({"role":"assistant","content":self.content});
        if !self.calls.is_empty() {
            message["tool_calls"] = json!(
                self.calls
                    .iter()
                    .map(|c| json!({"id":c.id,"type":"function",
                "function":{"name":c.name,"arguments":c.arguments}}))
                    .collect::<Vec<_>>()
            );
        }
        message
    }
}
/// v1's `readCompletion`: SSE over split UTF-8 and CRLF, events a gateway
/// split across `data:` records, `[DONE]` or a finish reason, bounded size.
#[derive(Default)]
pub struct Stream {
    buffer: Vec<u8>,
    pending: String,
    bytes: usize,
    finished: bool,
    message: Message,
    calls: BTreeMap<u64, Call>,
}
const INVALID: &str =
    "The provider returned an invalid response stream. Your conversation is kept; try again.";
const ENDED: &str =
    "The connection ended before the model finished. Your conversation is kept; try again.";
impl Stream {
    /// Feeds bytes; returns the reply text that arrived in them.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, String> {
        self.bytes += bytes.len();
        if self.bytes > MAX_RESPONSE {
            return Err("The model response is too large. Try a smaller request.".into());
        }
        if self.finished {
            return Ok(vec![]);
        }
        self.buffer.extend_from_slice(bytes);
        let mut texts = vec![];
        while let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
            let line = self.buffer.drain(..=end).collect::<Vec<_>>();
            let line = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
            self.accept(line.strip_suffix('\r').unwrap_or(&line), &mut texts)?;
            if self.finished {
                break;
            }
        }
        Ok(texts)
    }
    pub fn finish(mut self) -> Result<Message, String> {
        if !self.finished && !self.buffer.is_empty() {
            let rest = String::from_utf8_lossy(&std::mem::take(&mut self.buffer)).into_owned();
            self.accept(rest.trim_end_matches(['\r', '\n']), &mut vec![])?;
        }
        if !self.pending.is_empty() || !self.finished {
            return Err(ENDED.into());
        }
        let calls = std::mem::take(&mut self.calls)
            .into_values()
            .collect::<Vec<_>>();
        if calls.iter().any(|c| c.id.is_empty() || c.name.is_empty()) {
            return Err("The model returned an incomplete tool call.".into());
        }
        self.message.calls = calls;
        Ok(self.message)
    }
    fn accept(&mut self, line: &str, texts: &mut Vec<String>) -> Result<(), String> {
        let Some(data) = line.strip_prefix("data:") else {
            return Ok(());
        };
        let data = data.strip_prefix(' ').unwrap_or(data);
        if data.is_empty() {
            return Ok(());
        }
        if data.trim() == "[DONE]" {
            if !self.pending.is_empty() {
                return Err(INVALID.into());
            }
            self.finished = true;
            return Ok(());
        }
        let candidates = if self.pending.is_empty() {
            vec![data.to_owned()]
        } else {
            vec![
                format!("{}{data}", self.pending),
                format!("{}\n{data}", self.pending),
            ]
        };
        let mut event = None;
        let mut incomplete = false;
        for candidate in &candidates {
            match serde_json::from_str::<Value>(candidate) {
                Ok(value) => {
                    event = Some(value);
                    break;
                }
                Err(error) => incomplete |= error.is_eof(),
            }
        }
        let Some(event) = event else {
            if !incomplete {
                return Err(INVALID.into());
            }
            // A gateway split one event across records: keep it for the next.
            self.pending = candidates[0].clone();
            return Ok(());
        };
        self.pending.clear();
        if !event.is_object() {
            return Err(INVALID.into());
        }
        if !event["error"].is_null() {
            return Err(
                "The provider interrupted the response. Your conversation is kept; try again."
                    .into(),
            );
        }
        let choice = &event["choices"][0];
        if choice["finish_reason"] == "length" {
            return Err("The model reached its output limit. Try a smaller request.".into());
        }
        if choice["finish_reason"].is_string() {
            self.finished = true;
        }
        let delta = &choice["delta"];
        if let Some(text) = delta["content"].as_str() {
            self.message.content.push_str(text);
            texts.push(text.to_owned());
        }
        for part in delta["tool_calls"].as_array().into_iter().flatten() {
            let index = part["index"]
                .as_u64()
                .filter(|i| *i < 80)
                .ok_or("Invalid tool response from model.")?;
            let call = self.calls.entry(index).or_default();
            if let Some(id) = part["id"].as_str().filter(|id| !id.is_empty()) {
                call.id = id.into();
            }
            if let Some(name) = part["function"]["name"].as_str() {
                call.name.push_str(name);
            }
            if let Some(arguments) = part["function"]["arguments"].as_str() {
                call.arguments.push_str(arguments);
            }
        }
        Ok(())
    }
}

/// The app's tool server for this run, over MCP stdio.
struct Tools {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<Value>,
    next: u64,
    stop: Arc<AtomicBool>,
}
const TOOL_TIMEOUT: Duration = Duration::from_secs(3600);
impl Tools {
    fn start(run: &Run, stop: Arc<AtomicBool>) -> Result<Self, String> {
        let failed = |e: io::Error| format!("Powermove's tools could not start: {e}");
        let mut command = Command::new(&run.tools.0);
        command
            .args(&run.tools.1)
            .env_clear()
            .envs(&run.tool_env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().map_err(failed)?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let (tx, lines) = mpsc::sync_channel(16);
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = Vec::new();
            loop {
                line.clear();
                match reader
                    .by_ref()
                    .take(64 * 1024 * 1024)
                    .read_until(b'\n', &mut line)
                {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if let Ok(value) = serde_json::from_slice::<Value>(&line)
                            && tx.send(value).is_err()
                        {
                            break;
                        }
                    }
                }
            }
        });
        let mut tools = Self {
            child,
            stdin,
            lines,
            next: 0,
            stop,
        };
        tools.request(
            "initialize",
            json!({"protocolVersion":"2025-06-18","capabilities":{},
                "clientInfo":{"name":"powermove-compatible","version":"1"}}),
            Duration::from_secs(30),
        )?;
        tools.write(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))?;
        Ok(tools)
    }
    fn write(&mut self, value: &Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        self.stdin
            .write_all(&bytes)
            .and_then(|()| self.stdin.flush())
            .map_err(|_| "Powermove's tools stopped responding.".to_owned())
    }
    fn request(&mut self, method: &str, params: Value, limit: Duration) -> Result<Value, String> {
        self.next += 1;
        let id = self.next;
        self.write(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        let deadline = Instant::now() + limit;
        loop {
            if self.stop.load(Ordering::Acquire) {
                return Err("Stopped.".into());
            }
            if Instant::now() >= deadline {
                return Err("A Powermove tool took too long to answer.".into());
            }
            match self.lines.recv_timeout(Duration::from_millis(50)) {
                Ok(message) if message["id"] == id => {
                    return match message.get("error") {
                        Some(error) => Err(error["message"]
                            .as_str()
                            .unwrap_or("The tool failed.")
                            .to_owned()),
                        None => Ok(message["result"].clone()),
                    };
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Powermove's tools stopped responding.".into());
                }
            }
        }
    }
    /// The tools this run may use, as the server describes them.
    fn list(&mut self, allowed: &[String]) -> Result<Vec<Value>, String> {
        let listed = self.request("tools/list", json!({}), Duration::from_secs(30))?;
        Ok(listed["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| {
                t["name"]
                    .as_str()
                    .is_some_and(|n| allowed.iter().any(|a| a == n))
            })
            .cloned()
            .collect())
    }
    fn call(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        self.request(
            "tools/call",
            json!({"name":name,"arguments":arguments}),
            TOOL_TIMEOUT,
        )
    }
}
impl Drop for Tools {
    fn drop(&mut self) {
        #[cfg(unix)]
        // SAFETY: signals this run's own tool server process group.
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGTERM);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sse(events: &[Value]) -> Vec<u8> {
        let mut out = String::new();
        for e in events {
            out.push_str(&format!("data: {e}\r\n\r\n"));
        }
        out.push_str("data: [DONE]\n\n");
        out.into_bytes()
    }
    #[test]
    fn stream_survives_split_bytes_crlf_and_split_records() {
        let mut bytes = sse(&[
            json!({"choices":[{"delta":{"content":"Hé"}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"get_project","arguments":"{\"a\""}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"_state","arguments":":1}"}}]},"finish_reason":"tool_calls"}]}),
        ]);
        // A gateway splits one event over two records.
        let split = b"data: {\"choices\":[{\"delta\":\ndata: {\"content\":\"llo\"}}]}\n\n".to_vec();
        bytes.splice(0..0, split);
        let mut stream = Stream::default();
        let mut texts = vec![];
        for chunk in bytes.chunks(3) {
            texts.extend(stream.push(chunk).unwrap());
        }
        let message = stream.finish().unwrap();
        assert_eq!(message.content, "lloHé");
        assert_eq!(texts.concat(), "lloHé");
        assert_eq!(
            message.calls,
            vec![Call {
                id: "c1".into(),
                name: "get_project_state".into(),
                arguments: "{\"a\":1}".into()
            }]
        );
    }
    #[test]
    fn stream_refuses_truncation_garbage_and_errors() {
        let mut stream = Stream::default();
        stream
            .push(b"data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n")
            .unwrap();
        assert_eq!(stream.finish().unwrap_err(), ENDED);
        assert_eq!(
            Stream::default().push(b"data: nope\n").unwrap_err(),
            INVALID
        );
        assert!(
            Stream::default()
                .push(b"data: {\"error\":{\"message\":\"x\"}}\n")
                .unwrap_err()
                .contains("interrupted")
        );
        assert!(
            Stream::default()
                .push(b"data: {\"choices\":[{\"finish_reason\":\"length\"}]}\n")
                .unwrap_err()
                .contains("output limit")
        );
        let mut stream = Stream::default();
        stream
            .push(b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"name\":\"x\"}}]},\"finish_reason\":\"tool_calls\"}]}\n")
            .unwrap();
        assert!(
            stream
                .finish()
                .unwrap_err()
                .contains("incomplete tool call")
        );
    }
    #[test]
    fn efforts_follow_v1() {
        assert_eq!(effort("llama3.1", Some("high")), None);
        assert_eq!(effort("o3-mini", Some("xhigh")).as_deref(), Some("high"));
        assert_eq!(
            effort("gpt-5.4-pro", Some("low")).as_deref(),
            Some("medium")
        );
        assert_eq!(effort("gpt-5.4", None), None);
        assert!(efforts("gpt-5.4-chat-latest").is_empty());
    }
    #[test]
    fn oversized_tool_output_keeps_its_status() {
        let big = "x".repeat(130_000);
        let out: Value = serde_json::from_str(&tool_result(
            true,
            vec![json!({"type":"text","text":big})],
            None,
        ))
        .unwrap();
        assert_eq!(out["ok"], true);
        assert_eq!(out["outputTruncated"], true);
        assert!(out["content"][0]["text"].as_str().unwrap().len() < 10_000);
    }
}
