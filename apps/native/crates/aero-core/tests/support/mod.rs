//! Test support: the vendored fake inference server
//! (`vendor/fx/test-support/fake-inference-server.zig`, built by build.rs),
//! a minimal HTTP/1.1 client the tests use as the host's fetch edge, and a
//! Rust fake model that answers model requests inside the fetch edge (Gateway
//! SSE), so a test can script tool calls. No real model, no credentials.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pm_aero_core::{
    Config, Disposition, Error, Event, Outcome, Purpose, Remote, Runtime, State, Steered,
    StopReason,
};

/// Runs `body` on a fresh thread: the core is never called on the main
/// thread, and a test harness may run tests on it.
pub fn on_worker(body: impl FnOnce() + Send + 'static) {
    std::thread::spawn(body).join().expect("test thread");
}

pub fn config<'a>(endpoint: &'a str, tools: &'a [pm_aero_core::Tool<'a>]) -> Config<'a> {
    Config {
        endpoint_url: endpoint,
        model: "fake/model",
        instructions: "Reply with five x characters.",
        home: "/nonexistent/aero-home",
        workspace_root: "/nonexistent/aero-workspace",
        tools,
        ..Config::default()
    }
}

/// The fake server, killed (by the pid this test started) when dropped.
pub struct FakeServer {
    child: Child,
    pub port: u16,
}

impl FakeServer {
    pub fn start() -> FakeServer {
        let path = env!("PM_AERO_FAKE_SERVER");
        assert!(
            !path.is_empty(),
            "built without the libfx core: no fake server"
        );
        let mut child = Command::new(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the fake inference server");
        let mut line = String::new();
        BufReader::new(child.stdout.take().expect("stdout"))
            .read_line(&mut line)
            .expect("read the server's port");
        // {"host":"127.0.0.1","port":N}
        let port = line
            .split("\"port\":")
            .nth(1)
            .and_then(|rest| rest.trim().trim_end_matches('}').parse().ok())
            .unwrap_or_else(|| panic!("unexpected server banner: {line}"));
        FakeServer { child, port }
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Sends one request and streams the response body to `on_body` as it
/// arrives (chunked encoding decoded).
pub fn http(
    url: &str,
    method: &str,
    headers: &[(String, String)],
    body: &[u8],
    mut on_status: impl FnMut(u16),
    mut on_body: impl FnMut(&[u8]),
) -> std::io::Result<()> {
    let rest = url.strip_prefix("http://").expect("loopback http only");
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let path = if path.is_empty() { "/" } else { path };
    let mut stream = TcpStream::connect(authority)?;
    stream.set_nodelay(true)?;
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nhost: {authority}\r\nconnection: close\r\ncontent-length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        if lower == "host" || lower == "content-length" || lower == "connection" {
            continue;
        }
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let status: u16 = line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    on_status(status);
    let mut chunked = false;
    let mut length: Option<usize> = None;
    loop {
        line.clear();
        reader.read_line(&mut line)?;
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        let lower = header.to_ascii_lowercase();
        if lower.starts_with("transfer-encoding:") && lower.contains("chunked") {
            chunked = true;
        } else if let Some(value) = lower.strip_prefix("content-length:") {
            length = value.trim().parse().ok();
        }
    }
    if chunked {
        loop {
            line.clear();
            reader.read_line(&mut line)?;
            let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or("0"), 16)
                .unwrap_or(0);
            if size == 0 {
                break;
            }
            let mut chunk = vec![0u8; size];
            reader.read_exact(&mut chunk)?;
            on_body(&chunk);
            line.clear();
            reader.read_line(&mut line)?;
        }
    } else {
        let mut buf = [0u8; 16 * 1024];
        let mut remaining = length.unwrap_or(usize::MAX);
        while remaining > 0 {
            let want = remaining.min(buf.len());
            let read = reader.read(&mut buf[..want])?;
            if read == 0 {
                break;
            }
            on_body(&buf[..read]);
            remaining -= read;
        }
    }
    Ok(())
}

/// Waits until `fd` is readable or `timeout_ms` passes, then empties it.
pub fn wait_ready(runtime: &Runtime, timeout_ms: i32) {
    use std::os::fd::AsRawFd;
    let Some(fd) = runtime.ready_fd() else {
        std::thread::sleep(Duration::from_millis(timeout_ms as u64));
        return;
    };
    let fd = fd.as_raw_fd();
    let mut poll = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd.
    unsafe { libc::poll(&mut poll, 1, timeout_ms) };
    let mut sink = [0u8; 64];
    // SAFETY: non-blocking read into a local buffer; wake bytes carry no state.
    while unsafe { libc::read(fd, sink.as_mut_ptr().cast(), sink.len()) } > 0 {}
}

// ---------------------------------------------------------------------------
// The Rust fake model
// ---------------------------------------------------------------------------

/// One model request, as the fake model sees it.
#[derive(Clone, Debug)]
pub struct ModelCall {
    pub index: usize,
    pub url: String,
    pub body: String,
    pub headers: Vec<(String, String)>,
}

/// How the fake model answers.
pub enum Reply {
    /// A 200 with this Gateway SSE body.
    Sse(String),
    /// Never send the status line: the request hangs until the core retires it.
    Hold,
    /// The loopback fake server answers (the smoke test's path).
    Server,
    /// A 200 whose body arrives in these pieces, `gap` apart; it stops when
    /// the core retires the fetch.
    Chunks(Vec<String>, Duration),
}

pub type Model = Arc<dyn Fn(&ModelCall) -> Reply + Send + Sync>;

/// Gateway SSE: a text reply.
pub fn sse_text(text: &str) -> String {
    format!(
        "data: {{\"type\":\"text-delta\",\"id\":\"0\",\"delta\":{}}}\n\n\
         data: {{\"type\":\"finish\",\"finishReason\":{{\"unified\":\"stop\",\"raw\":\"stop\"}},\"usage\":{{\"inputTokens\":{{\"total\":1}},\"outputTokens\":{{\"total\":1}}}}}}\n\n\
         data: [DONE]\n\n",
        json_string(text)
    )
}

/// Gateway SSE pieces: one text delta per piece, then the finish.
pub fn sse_pieces(pieces: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = pieces
        .iter()
        .map(|piece| {
            format!(
                "data: {{\"type\":\"text-delta\",\"id\":\"0\",\"delta\":{}}}\n\n",
                json_string(piece)
            )
        })
        .collect();
    out.push(
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\",\"raw\":\"stop\"},\"usage\":{\"inputTokens\":{\"total\":1},\"outputTokens\":{\"total\":1}}}\n\n\
         data: [DONE]\n\n"
            .to_string(),
    );
    out
}

/// Gateway SSE: reasoning, then a text reply.
pub fn sse_reasoning(reasoning: &str, text: &str) -> String {
    format!(
        "data: {{\"type\":\"reasoning-start\",\"id\":\"r1\"}}\n\n\
         data: {{\"type\":\"reasoning-delta\",\"id\":\"r1\",\"delta\":{}}}\n\n\
         data: {{\"type\":\"reasoning-end\",\"id\":\"r1\"}}\n\n{}",
        json_string(reasoning),
        sse_text(text)
    )
}

/// Gateway SSE: calls to `tool` with each input JSON, in parallel.
pub fn sse_tool_calls(tool: &str, inputs: &[&str]) -> String {
    let mut out = String::new();
    for (index, input) in inputs.iter().enumerate() {
        let id = format!("call_{index}");
        out.push_str(&format!(
            "data: {{\"type\":\"tool-input-start\",\"id\":\"{id}\",\"toolName\":\"{tool}\"}}\n\n\
             data: {{\"type\":\"tool-input-delta\",\"id\":\"{id}\",\"delta\":{}}}\n\n\
             data: {{\"type\":\"tool-call\",\"toolCallId\":\"{id}\",\"toolName\":\"{tool}\",\"input\":{}}}\n\n",
            json_string(input),
            json_string(input)
        ));
    }
    out.push_str(
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\",\"raw\":\"tool_calls\"},\"usage\":{\"inputTokens\":{\"total\":1},\"outputTokens\":{\"total\":1}}}\n\n\
         data: [DONE]\n\n",
    );
    out
}

pub fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A model that replies `text` to everything.
pub fn text_model(text: &'static str) -> Model {
    Arc::new(move |_| Reply::Sse(sse_text(text)))
}

/// An owned copy of an event, for assertions after the drain.
#[derive(Clone, Debug, PartialEq)]
pub enum Seen {
    Opened {
        request: u64,
        session: String,
        resumable: bool,
        turns: u64,
    },
    RequestDone {
        request: u64,
        outcome: Result<Outcome, Error>,
        message: String,
    },
    TurnStart {
        turn: u64,
        message_id: String,
        repeated: bool,
        resumed: bool,
    },
    Text {
        turn: u64,
        text: String,
    },
    Thought {
        turn: u64,
        text: String,
    },
    Usage {
        turn: u64,
        used: u64,
        size: u64,
    },
    Recovery {
        request: u64,
        kind: i32,
        text: String,
        text2: String,
        bytes: Vec<u8>,
    },
    Checkpoint {
        request: u64,
        at_seq: u64,
        bytes: Vec<u8>,
    },
    ImagesNeeded {
        request: u64,
        names: Vec<String>,
    },
    ToolStart {
        turn: u64,
        name: String,
    },
    ToolEnd {
        turn: u64,
        name: String,
        failed: bool,
    },
    SteerJoined {
        turn: u64,
        request: u64,
        message_id: String,
    },
    SteerReturned {
        turn: u64,
        request: u64,
        message_id: String,
    },
    TurnEnd {
        turn: u64,
        reason: StopReason,
    },
    Error {
        turn: u64,
        message: String,
    },
    Exited {
        state: State,
        message: String,
        events_lost: bool,
    },
    Other(String),
}

impl Seen {
    fn of(event: &Event<'_>) -> Seen {
        match *event {
            Event::SessionOpened {
                request,
                session_id,
                resumable,
                turns,
                ..
            } => Seen::Opened {
                request,
                session: session_id.to_string(),
                resumable,
                turns,
            },
            Event::ThoughtDelta { turn, text } => Seen::Thought {
                turn,
                text: text.to_string(),
            },
            Event::Usage { turn, usage } => Seen::Usage {
                turn,
                used: usage.context_used,
                size: usage.context_size,
            },
            Event::Recovery {
                request,
                kind,
                text,
                text2,
                bytes,
            } => Seen::Recovery {
                request,
                kind,
                text: text.to_string(),
                text2: text2.to_string(),
                bytes: bytes.to_vec(),
            },
            Event::Checkpoint {
                request,
                at_seq,
                bytes,
            } => Seen::Checkpoint {
                request,
                at_seq,
                bytes: bytes.to_vec(),
            },
            Event::ImagesNeeded { request, names, .. } => Seen::ImagesNeeded {
                request,
                names: names.iter().map(str::to_string).collect(),
            },
            Event::RequestDone {
                request,
                outcome,
                message,
            } => Seen::RequestDone {
                request,
                outcome,
                message: message.to_string(),
            },
            Event::TurnStart {
                turn,
                message_id,
                repeated,
                resumed,
            } => Seen::TurnStart {
                turn,
                message_id: message_id.to_string(),
                repeated,
                resumed,
            },
            Event::TextDelta { turn, text } => Seen::Text {
                turn,
                text: text.to_string(),
            },
            Event::ToolStart { turn, name, .. } => Seen::ToolStart {
                turn,
                name: name.to_string(),
            },
            Event::ToolEnd {
                turn, name, failed, ..
            } => Seen::ToolEnd {
                turn,
                name: name.to_string(),
                failed,
            },
            Event::SteerJoined {
                turn,
                request,
                message_id,
                ..
            } => Seen::SteerJoined {
                turn,
                request,
                message_id: message_id.to_string(),
            },
            Event::SteerReturned {
                turn,
                request,
                message_id,
            } => Seen::SteerReturned {
                turn,
                request,
                message_id: message_id.to_string(),
            },
            Event::TurnEnd { turn, reason, .. } => Seen::TurnEnd { turn, reason },
            Event::Error { turn, message, .. } => Seen::Error {
                turn,
                message: message.to_string(),
            },
            Event::Exited {
                state,
                message,
                events_lost,
            } => Seen::Exited {
                state,
                message: message.to_string(),
                events_lost,
            },
            other => Seen::Other(format!("{other:?}")),
        }
    }
}

/// Fetches the driver has seen, by id.
#[derive(Clone, Debug)]
pub struct FetchSeen {
    pub fetch: u64,
    pub turn: u64,
    pub purpose: Purpose,
    pub provider: String,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// What the edge sends with SYNTHETIC_CREDENTIALS (FetchRequest::outgoing_headers).
    pub outgoing: Vec<(String, String)>,
}

/// Synthetic values only: the lane never uses a real credential.
pub const GATEWAY_KEY: &str = "Bearer synthetic-gateway-key";
pub const CODEX_KEY: &str = "Bearer synthetic-codex-key";
pub const CODEX_ACCOUNT: &str = "synthetic-account";
/// The trusted allowlist: two headers for Codex's origin.
pub const SYNTHETIC_CREDENTIALS: [pm_aero_core::Credential<'static>; 3] = [
    pm_aero_core::Credential {
        provider: "gateway",
        origin: "https://ai-gateway.vercel.sh",
        header: "authorization",
        value: GATEWAY_KEY,
    },
    pm_aero_core::Credential {
        provider: "codex",
        origin: "https://chatgpt.com",
        header: "authorization",
        value: CODEX_KEY,
    },
    pm_aero_core::Credential {
        provider: "codex",
        origin: "https://chatgpt.com",
        header: "chatgpt-account-id",
        value: CODEX_ACCOUNT,
    },
];

/// Threads in this process.
pub fn thread_count() -> i32 {
    // SAFETY: an all-zero proc_taskinfo is valid; the call writes at most
    // its size.
    unsafe {
        let mut info: libc::proc_taskinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_taskinfo>() as i32;
        let got = libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDTASKINFO,
            0,
            (&mut info as *mut libc::proc_taskinfo).cast(),
            size,
        );
        assert_eq!(got, size, "proc_pidinfo");
        info.pti_threadnum
    }
}

/// Drives one runtime: waits on its ready fd, drains its events and serves
/// its fetches with the fake model (one thread per fetch, as the app's HTTP
/// client would).
pub struct Driver {
    pub runtime: Runtime,
    pub model: Model,
    pub endpoint: String,
    calls: Arc<AtomicUsize>,
    pub seen: Vec<Seen>,
    pub fetches: Arc<Mutex<Vec<FetchSeen>>>,
    /// The model catalog the Gateway's catalog GET reads; None answers 404
    /// (the core falls back to its built-in capabilities).
    pub catalog: Option<String>,
}

/// A Gateway catalog listing `fake/model` with vision and a context window.
pub const VISION_CATALOG: &str = r#"{"object":"list","data":[{"id":"fake/model","type":"language","tags":["tool-use","vision"],"context_window":200000,"max_tokens":8000}]}"#;

impl Driver {
    pub fn new(runtime: Runtime, model: Model, endpoint: &str) -> Driver {
        Driver {
            runtime,
            model,
            endpoint: endpoint.to_string(),
            calls: Arc::new(AtomicUsize::new(0)),
            seen: Vec::new(),
            fetches: Arc::new(Mutex::new(Vec::new())),
            catalog: None,
        }
    }

    pub fn remote(&self) -> Remote {
        self.runtime.remote()
    }

    /// One round: wait up to `wait_ms`, drain, serve. Returns what was new.
    pub fn pump(&mut self, wait_ms: i32) -> Vec<Seen> {
        wait_ready(&self.runtime, wait_ms);
        let mut new = Vec::new();
        match self.runtime.events.drain() {
            Ok(events) => new.extend(events.map(|event| Seen::of(&event))),
            Err(Error::Busy) => {}
            Err(error) => panic!("drain: {error}"),
        }
        let remote = self.runtime.remote();
        loop {
            let request = match self.runtime.fetches.take() {
                Ok(Some(request)) => request,
                Ok(None) | Err(Error::Failed) | Err(Error::Busy) => break,
                Err(error) => panic!("take fetch: {error}"),
            };
            let seen = FetchSeen {
                fetch: request.fetch,
                turn: request.turn,
                purpose: request.purpose,
                provider: request.provider.to_string(),
                method: request.method.to_string(),
                url: request.url.to_string(),
                headers: request
                    .headers()
                    .map(|(n, v)| (n.to_string(), v.to_string()))
                    .collect(),
                outgoing: request.outgoing_headers(&SYNTHETIC_CREDENTIALS),
            };
            let body = String::from_utf8_lossy(request.body).into_owned();
            self.fetches.lock().unwrap().push(seen.clone());
            serve(
                remote,
                seen,
                body,
                self.model.clone(),
                self.calls.clone(),
                &self.endpoint,
                self.catalog.clone(),
            );
        }
        self.seen.extend(new.iter().cloned());
        new
    }

    /// Serves the fetches the core published, without draining events.
    pub fn serve_only(&mut self) {
        let remote = self.runtime.remote();
        while let Ok(Some(request)) = self.runtime.fetches.take() {
            let seen = FetchSeen {
                fetch: request.fetch,
                turn: request.turn,
                purpose: request.purpose,
                provider: request.provider.to_string(),
                method: request.method.to_string(),
                url: request.url.to_string(),
                headers: request
                    .headers()
                    .map(|(n, v)| (n.to_string(), v.to_string()))
                    .collect(),
                outgoing: request.outgoing_headers(&SYNTHETIC_CREDENTIALS),
            };
            let body = String::from_utf8_lossy(request.body).into_owned();
            self.fetches.lock().unwrap().push(seen.clone());
            serve(
                remote,
                seen,
                body,
                self.model.clone(),
                self.calls.clone(),
                &self.endpoint,
                self.catalog.clone(),
            );
        }
    }

    /// Pumps until `done` holds for an event, or panics after `secs`.
    pub fn until(&mut self, secs: u64, mut done: impl FnMut(&Seen) -> bool) -> Seen {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            for event in self.pump(20) {
                if done(&event) {
                    return event;
                }
            }
            assert!(Instant::now() < deadline, "timed out; saw {:#?}", self.seen);
        }
    }

    /// Opens the session and waits for it.
    pub fn open(&mut self) -> String {
        let request = self.runtime.open_session("").expect("open the session");
        match self.until(20, |e| {
            matches!(
                e,
                Seen::Opened { .. } | Seen::RequestDone { .. } | Seen::Exited { .. }
            )
        }) {
            Seen::Opened {
                request: r,
                session,
                ..
            } => {
                assert_eq!(r, request);
                session
            }
            other => panic!("open failed: {other:?}"),
        }
    }

    /// Prompts and pumps to the turn's end; returns the turn id.
    pub fn turn(&mut self, prompt: &str, message_id: &str) -> (u64, StopReason) {
        let turn = self.runtime.prompt(prompt, message_id).expect("prompt");
        match self.until(30, |e| {
            matches!(e, Seen::TurnEnd { turn: t, .. } if *t == turn)
                || matches!(e, Seen::Exited { .. })
        }) {
            Seen::TurnEnd { reason, .. } => (turn, reason),
            other => panic!("turn ended with {other:?}"),
        }
    }

    pub fn text_of(&self, turn: u64) -> String {
        self.seen
            .iter()
            .filter_map(|event| match event {
                Seen::Text { turn: t, text } if *t == turn => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// Serves one fetch on its own thread.
fn serve(
    remote: Remote,
    seen: FetchSeen,
    body: String,
    model: Model,
    calls: Arc<AtomicUsize>,
    endpoint: &str,
    catalog: Option<String>,
) {
    let endpoint = endpoint.to_string();
    std::thread::spawn(move || {
        if let (Purpose::Catalog, Some(listing)) = (seen.purpose, &catalog)
            && seen.provider == "gateway"
            && seen.url.ends_with("/models")
        {
            if remote.fetch_status(seen.fetch, 200, None) == Ok(Outcome::Applied) {
                push_all(remote, seen.fetch, listing.as_bytes());
            }
            let _ = remote.fetch_finish(seen.fetch, true);
            return;
        }
        // The model catalog (a GET to the provider's canonical URL) and
        // anything but the model route: answered here, Not Found. Tests
        // never reach the network.
        if seen.purpose == Purpose::Catalog || seen.url != endpoint {
            let _ = remote.fetch_status(seen.fetch, 404, None);
            let _ = remote.fetch_finish(seen.fetch, true);
            return;
        }
        let call = ModelCall {
            index: calls.fetch_add(1, Ordering::SeqCst),
            url: seen.url.clone(),
            body,
            headers: seen.headers.clone(),
        };
        match model(&call) {
            Reply::Hold => {}
            Reply::Sse(text) => {
                if remote.fetch_status(seen.fetch, 200, None) != Ok(Outcome::Applied) {
                    return;
                }
                push_all(remote, seen.fetch, text.as_bytes());
                let _ = remote.fetch_finish(seen.fetch, true);
            }
            Reply::Chunks(pieces, gap) => {
                if remote.fetch_status(seen.fetch, 200, None) != Ok(Outcome::Applied) {
                    return;
                }
                for piece in pieces {
                    if remote.fetch_disposition(seen.fetch) == Disposition::Retired {
                        return;
                    }
                    push_all(remote, seen.fetch, piece.as_bytes());
                    std::thread::sleep(gap);
                }
                let _ = remote.fetch_finish(seen.fetch, true);
            }
            Reply::Server => {
                let live = std::cell::Cell::new(true);
                let result = http(
                    &seen.url,
                    &seen.method,
                    &seen.outgoing,
                    call.body.as_bytes(),
                    |status| {
                        live.set(
                            remote.fetch_status(seen.fetch, status, None) == Ok(Outcome::Applied),
                        )
                    },
                    |chunk| {
                        if live.get()
                            && matches!(
                                remote.fetch_disposition(seen.fetch),
                                Disposition::Active | Disposition::Busy
                            )
                        {
                            push_all(remote, seen.fetch, chunk);
                        } else {
                            live.set(false);
                        }
                    },
                );
                let _ = remote.fetch_finish(seen.fetch, result.is_ok());
            }
        }
    });
}

pub fn push_all(remote: Remote, fetch: u64, bytes: &[u8]) {
    loop {
        match remote.fetch_push(fetch, bytes) {
            Ok(Outcome::Backpressure) => std::thread::sleep(Duration::from_millis(1)),
            _ => return,
        }
    }
}

/// Steers, asserting the outcome kind.
pub fn steer(runtime: &mut Runtime, turn: u64, text: &str, id: &str) -> Steered {
    runtime.steer(turn, text, id).expect("steer")
}
