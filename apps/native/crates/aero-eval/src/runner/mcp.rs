//! A small MCP client (newline-delimited JSON-RPC 2.0) for the runner's own
//! connection to the headless `powermove-mcp` host: seeding, collection and
//! panel probes. Lines are read on one thread and written on another, and
//! the caller only ever waits on a channel with its own deadline: a host that
//! stops answering, or stops reading its input, fails the call at its
//! deadline instead of holding the runner in a blocked read or write. Once a
//! write has missed its deadline the connection is spent: every later call
//! fails at once, and the runner stops the host.
use super::sandbox::Confinement;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

/// Why a request failed.
#[derive(Clone, Debug, PartialEq)]
pub enum RpcError {
    /// No answer before the deadline.
    Timeout(String),
    /// The host closed its output or input.
    Closed(String),
    /// A JSON-RPC error answer: code and message.
    Remote(i64, String),
    /// The host wrote something that is not JSON-RPC.
    Protocol(String),
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpcError::Timeout(m) | RpcError::Closed(m) | RpcError::Protocol(m) => f.write_str(m),
            RpcError::Remote(code, m) => write!(f, "{m} ({code})"),
        }
    }
}

impl RpcError {
    fn context(self, method: &str) -> Self {
        match self {
            RpcError::Timeout(m) => RpcError::Timeout(format!("{method}: {m}")),
            RpcError::Closed(m) => RpcError::Closed(format!("{method}: {m}")),
            RpcError::Protocol(m) => RpcError::Protocol(format!("{method}: {m}")),
            e @ RpcError::Remote(..) => e,
        }
    }
}

impl From<RpcError> for String {
    fn from(e: RpcError) -> String {
        e.to_string()
    }
}

/// What to do with a notification that arrives while a request waits: keep
/// waiting, optionally sending a notification back (e.g. a cancel).
pub type OnNote<'a> = &'a mut dyn FnMut(&Value) -> Option<(String, Value)>;

/// One line for the writer thread, and where to report that it was written.
type Outgoing = (Vec<u8>, Sender<std::io::Result<()>>);

pub struct McpClient {
    lines: Receiver<Result<String, String>>,
    writer: Sender<Outgoing>,
    /// A write missed its deadline: the host is not reading its input.
    stuck: bool,
    next_id: u64,
    /// Per-request deadline.
    pub call_deadline: Duration,
    /// No request may wait past this (a collection budget).
    pub budget_end: Option<Instant>,
    /// How long a notification (a cancel) may take to write.
    pub write_deadline: Duration,
    /// Notifications received while waiting for responses, in order.
    pub notifications: Vec<Value>,
    /// The request whose deadline passed, for [`McpClient::resume`].
    pending: Option<(u64, String)>,
}

/// A tool result: its content blocks and whether the host flagged an error.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: Vec<Value>,
    pub is_error: bool,
}

impl ToolResult {
    /// The first text block parsed as JSON (most Powermove tools answer so).
    pub fn json(&self) -> Option<Value> {
        self.content
            .iter()
            .filter(|c| c["type"] == "text")
            .find_map(|c| serde_json::from_str(c["text"].as_str()?).ok())
    }
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|c| c["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
    /// Decoded image blocks with their MIME types.
    pub fn images(&self) -> Vec<(String, Vec<u8>)> {
        use base64::Engine;
        self.content
            .iter()
            .filter(|c| c["type"] == "image")
            .filter_map(|c| {
                let data = base64::engine::general_purpose::STANDARD
                    .decode(c["data"].as_str()?)
                    .ok()?;
                Some((c["mimeType"].as_str().unwrap_or("").to_owned(), data))
            })
            .collect()
    }
}

/// Lines longer than this are a broken host, not a message.
const MAX_LINE: usize = 256 << 20;

impl McpClient {
    pub fn new<R: Read + Send + 'static, W: Write + Send + 'static>(reader: R, writer: W) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut r = BufReader::new(reader);
            loop {
                let mut buf = Vec::new();
                let sent = match r.read_until(b'\n', &mut buf) {
                    Ok(0) => tx.send(Err("closed".into())),
                    Ok(_) if buf.len() > MAX_LINE => tx.send(Err("line too long".into())),
                    Ok(_) => tx.send(
                        String::from_utf8(buf).map_err(|_| "a line that is not UTF-8".into()),
                    ),
                    Err(e) => tx.send(Err(e.to_string())),
                };
                if sent.is_err() {
                    return;
                }
            }
        });
        let (wtx, wrx) = std::sync::mpsc::channel::<Outgoing>();
        std::thread::spawn(move || {
            let mut writer = writer;
            for (bytes, done) in wrx {
                let r = writer.write_all(&bytes).and_then(|()| writer.flush());
                let failed = r.is_err();
                let _ = done.send(r);
                if failed {
                    return;
                }
            }
        });
        Self {
            lines: rx,
            writer: wtx,
            stuck: false,
            next_id: 1,
            call_deadline: Duration::from_secs(60),
            write_deadline: Duration::from_secs(5),
            budget_end: None,
            notifications: Vec::new(),
            pending: None,
        }
    }

    fn end_for(&self, deadline: Duration) -> Instant {
        let end = Instant::now() + deadline;
        self.budget_end.map_or(end, |b| b.min(end))
    }

    /// Send a request and wait for its response until the call deadline.
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, RpcError> {
        let end = self.end_for(self.call_deadline);
        self.request_until(method, params, end, &mut |_| None)
    }

    /// Send a request and wait until `end`, handing each notification to
    /// `on_note` as it arrives (it may answer with a notification of its own).
    pub fn request_until(
        &mut self,
        method: &str,
        params: Value,
        end: Instant,
        on_note: OnNote,
    ) -> Result<Value, RpcError> {
        let id = self.next_id;
        self.next_id += 1;
        self.pending = None;
        self.send(
            &json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
            end,
        )
        .map_err(|e| e.context(method))?;
        let r = self.wait(id, method, end, on_note);
        if let Err(RpcError::Timeout(_)) = r {
            self.pending = Some((id, method.to_owned()));
        }
        r
    }

    /// Keep waiting for the request whose deadline passed (e.g. after a
    /// cancel), until `end`.
    pub fn resume(&mut self, end: Instant, on_note: OnNote) -> Result<Value, RpcError> {
        if self.stuck {
            return Err(RpcError::Timeout(
                "the host stopped reading its input at an earlier deadline".into(),
            ));
        }
        let (id, method) = self
            .pending
            .take()
            .ok_or_else(|| RpcError::Protocol("no request is pending".into()))?;
        let r = self.wait(id, &method, end, on_note);
        if let Err(RpcError::Timeout(_)) = r {
            self.pending = Some((id, method));
        }
        r
    }

    fn wait(
        &mut self,
        id: u64,
        method: &str,
        end: Instant,
        on_note: OnNote,
    ) -> Result<Value, RpcError> {
        loop {
            let left = end.saturating_duration_since(Instant::now());
            let line = match self.lines.recv_timeout(left) {
                Ok(Ok(l)) => l,
                Ok(Err(e)) => {
                    return Err(RpcError::Closed(format!(
                        "{method}: host output {e} before answering"
                    )));
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(RpcError::Timeout(format!(
                        "{method}: no answer within the deadline"
                    )));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(RpcError::Closed(format!("{method}: host output closed")));
                }
            };
            let Ok(m) = serde_json::from_str::<Value>(&line) else {
                return Err(RpcError::Protocol(format!(
                    "{method}: host wrote a line that is not JSON"
                )));
            };
            if m.get("id") == Some(&json!(id)) {
                if let Some(e) = m.get("error") {
                    return Err(RpcError::Remote(
                        e["code"].as_i64().unwrap_or(0),
                        format!("{method}: {}", e["message"].as_str().unwrap_or("error")),
                    ));
                }
                return Ok(m.get("result").cloned().unwrap_or(Value::Null));
            }
            if m.get("method").is_some() && m.get("id").is_none() {
                if let Some((note, params)) = on_note(&m) {
                    let _ = self.notify(&note, params);
                }
                self.notifications.push(m);
            }
        }
    }

    /// Hand one line to the writer thread and wait until it is written, at
    /// most until `end`.
    fn send(&mut self, v: &Value, end: Instant) -> Result<(), RpcError> {
        if self.stuck {
            return Err(RpcError::Timeout(
                "the host stopped reading its input at an earlier deadline".into(),
            ));
        }
        let mut line = v.to_string().into_bytes();
        line.push(b'\n');
        let (tx, rx) = std::sync::mpsc::channel();
        if self.writer.send((line, tx)).is_err() {
            return Err(RpcError::Closed("host closed its input".into()));
        }
        match rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(RpcError::Closed(format!("host closed its input ({e})"))),
            Err(RecvTimeoutError::Timeout) => {
                self.stuck = true;
                Err(RpcError::Timeout(
                    "the host did not read its input within the deadline".into(),
                ))
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err(RpcError::Closed("host closed its input".into()))
            }
        }
    }

    /// Send a notification; the write is bounded by [`Self::write_deadline`]
    /// (and the collection budget).
    pub fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        let end = self.end_for(self.write_deadline);
        self.send(
            &json!({"jsonrpc":"2.0","method":method,"params":params}),
            end,
        )
        .map_err(|e| e.context(method).to_string())
    }

    pub fn initialize(&mut self, deadline: Duration) -> Result<Value, RpcError> {
        let end = self.end_for(deadline);
        let r = self.request_until(
            "initialize",
            json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"aero-eval","version":"1"}}),
            end,
            &mut |_| None,
        )?;
        self.notify("notifications/initialized", json!({}))
            .map_err(RpcError::Closed)?;
        Ok(r)
    }

    /// Call a tool. A host-flagged error comes back as `Err` with its text.
    pub fn call(&mut self, name: &str, args: Value) -> Result<ToolResult, String> {
        let r = self.request("tools/call", json!({"name":name,"arguments":args}))?;
        let result = ToolResult {
            content: r["content"].as_array().cloned().unwrap_or_default(),
            is_error: r["isError"].as_bool().unwrap_or(false),
        };
        if result.is_error {
            return Err(format!("{name}: {}", result.text()));
        }
        Ok(result)
    }

    pub fn tool_names(&mut self) -> Result<Vec<String>, String> {
        let r = self.request("tools/list", json!({}))?;
        Ok(r["tools"]
            .as_array()
            .map(|t| {
                t.iter()
                    .filter_map(|t| t["name"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default())
    }
}

/// How long the runner waits on the host, per phase.
#[derive(Clone, Copy, Debug)]
pub struct Deadlines {
    /// Start-up and `initialize`.
    pub init: Duration,
    /// One collection or probe call.
    pub call: Duration,
    /// All of the baseline, and all of the collection after the run.
    pub collect: Duration,
    /// After the harness deadline: time for a cancelled run to answer, and
    /// for a stopped process group to disappear.
    pub grace: Duration,
}

impl Default for Deadlines {
    fn default() -> Self {
        Self {
            init: Duration::from_secs(30),
            call: Duration::from_secs(60),
            collect: Duration::from_secs(600),
            grace: Duration::from_secs(5),
        }
    }
}

/// The headless host as a child process (leader of its own process group),
/// with the runner's stdio client.
pub struct Host {
    pub child: Child,
    pub client: McpClient,
    /// The host's `initialize` answer (its advertised capabilities).
    pub init: Value,
}

impl Host {
    /// `powermove-mcp --profile P --exports E --listen`: the runner talks over
    /// stdio; harnesses attach with `--powermove-mcp --profile P`. The
    /// environment is exactly `env` (nothing inherited), and `confine` puts
    /// the host under the run's OS boundary.
    pub fn spawn(
        bin: &Path,
        profile: &Path,
        exports: &Path,
        log: &Path,
        env: &[(String, String)],
        confine: Option<&Confinement>,
        deadlines: &Deadlines,
    ) -> Result<Self, String> {
        let stderr = super::paths::create_new(log)?;
        let mut cmd = super::sandbox::command(bin, confine);
        cmd.arg("--profile")
            .arg(profile)
            .arg("--exports")
            .arg(exports)
            .arg("--listen")
            .current_dir(profile)
            .env_clear()
            .envs(env.iter().cloned())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr);
        let mut child =
            super::proc::spawn_group(&mut cmd).map_err(|e| format!("{}: {e}", bin.display()))?;
        let mut client = McpClient::new(child.stdout.take().unwrap(), child.stdin.take().unwrap());
        client.call_deadline = deadlines.call;
        let mut host = Host {
            child,
            client,
            init: Value::Null,
        };
        match host.client.initialize(deadlines.init) {
            Ok(init) => host.init = init,
            Err(e) => {
                host.kill(deadlines.grace);
                return Err(format!("the host did not initialise: {e}"));
            }
        }
        Ok(host)
    }

    /// Close stdin (the host's clean shutdown), wait `grace`, then stop the
    /// whole process group whether or not the host exited.
    pub fn close(self, grace: Duration) -> super::proc::Stopped {
        let Host {
            mut child, client, ..
        } = self;
        drop(client);
        let _ = super::proc::wait_until(&mut child, Instant::now() + grace);
        super::proc::stop_group(&mut child, grace)
    }

    /// Stop the host and everything it started, now.
    pub fn kill(&mut self, grace: Duration) -> super::proc::Stopped {
        super::proc::stop_group(&mut self.child, grace)
    }
}
