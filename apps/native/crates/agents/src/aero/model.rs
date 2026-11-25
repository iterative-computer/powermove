//! The fake and replay model edges. Replay can reach exactly one numeric
//! loopback origin. No default headers, proxies, redirects or auth discovery.
use super::*;
use pm_aero_core::{Disposition, Image, Outcome, Remote};
use serde::Deserialize;
use std::io::Read;
use url::{Host as UrlHost, Url};

#[derive(Clone)]
pub struct Model {
    transport: Transport,
    /// Why the latest provider request failed, in plain words: the core only
    /// hears that it failed, and would show its own error code instead.
    failure: Arc<std::sync::Mutex<Option<String>>>,
}

type Fake = Arc<dyn Fn(&[u8]) -> Reply + Send + Sync>;

#[derive(Clone)]
enum Transport {
    Fake(Fake),
    Replay(Url),
    Live(Arc<Live>),
}

/// Powermove inference (`crate::inference::live`): the core's provider and
/// its description (no credential), and the credential the edge adds for
/// `origin` only. `credentials` gets the bearer a backend just refused.
pub struct Live {
    pub provider: String,
    pub configured: String,
    pub model: String,
    pub net: Arc<dyn crate::edge::Net>,
    #[allow(clippy::type_complexity)]
    pub credentials:
        Arc<dyn Fn(Option<&str>) -> Result<(String, Vec<(String, String)>), String> + Send + Sync>,
}

pub enum Reply {
    Text(String),
    Tool {
        name: String,
        args: Value,
    },
    /// Text the model streams before its reply in the same step (a tool
    /// call's narration).
    Said(String, Box<Reply>),
    Hold,
}

impl Model {
    pub fn fake(f: impl Fn(&[u8]) -> Reply + Send + Sync + 'static) -> Self {
        Self {
            transport: Transport::Fake(Arc::new(f)),
            failure: Default::default(),
        }
    }

    pub fn replay(base: &str) -> Result<Self, String> {
        let url = Url::parse(base).map_err(|e| format!("Invalid replay URL: {e}"))?;
        let loopback = match url.host() {
            Some(UrlHost::Ipv4(addr)) => addr.is_loopback(),
            Some(UrlHost::Ipv6(addr)) => addr.is_loopback(),
            _ => false,
        };
        if !loopback
            || url.scheme() != "http"
            || url.port().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "Aero replay requires an explicit HTTP URL on a numeric loopback address.".into(),
            );
        }
        Ok(Self {
            transport: Transport::Replay(url),
            failure: Default::default(),
        })
    }

    pub fn live(live: Live) -> Self {
        Self {
            transport: Transport::Live(Arc::new(live)),
            failure: Default::default(),
        }
    }

    /// Why the latest provider request failed, once.
    pub fn take_failure(&self) -> Option<String> {
        self.failure.lock().unwrap().take()
    }
    /// Whether the latest provider request failed in words the edge wrote:
    /// the core's own account of it then repeats nothing a person needs.
    pub fn failure_pending(&self) -> bool {
        self.failure.lock().unwrap().is_some()
    }

    pub fn endpoint(&self) -> &str {
        match &self.transport {
            Transport::Fake(_) => "http://127.0.0.1:1/aero-fake",
            Transport::Replay(url) => url.as_str(),
            Transport::Live(_) => "",
        }
    }
    /// The core's provider id ("" is the Gateway the fakes stand behind).
    pub fn provider(&self) -> &str {
        match &self.transport {
            Transport::Live(live) => &live.provider,
            _ => "",
        }
    }
    pub fn configured(&self) -> &str {
        match &self.transport {
            Transport::Live(live) => &live.configured,
            _ => "",
        }
    }
    /// The model the core is given: a live source's own id, else `requested`.
    pub fn name<'a>(&'a self, requested: &'a str) -> &'a str {
        match &self.transport {
            Transport::Live(live) => &live.model,
            _ => requested,
        }
    }
}

/// The fake and replay edges answer the catalog GET here, never on the
/// network: the session's model with tool use and vision, so tool images
/// reach it (the core refuses images to a model without vision), and the
/// session's effort as its one reasoning option (the core refuses an effort
/// the model does not list).
/// A catalog GET: the fakes' answer; Aero's own connection lists none.
pub(super) fn catalog(
    edge: &Model,
    remote: Remote,
    fetch: u64,
    _url: &str,
    model: &str,
    effort: &str,
) {
    if !matches!(edge.transport, Transport::Live(_)) {
        return answer_catalog(remote, fetch, model, effort);
    }
    let ok = status(remote, fetch, 404);
    let _ = remote.fetch_finish(fetch, ok.is_ok());
}

pub(super) fn answer_catalog(remote: Remote, fetch: u64, model: &str, effort: &str) {
    let mut entry = json!({"id":model,"type":"language","tags":["tool-use","vision"],
        "context_window":200000,"max_tokens":8000});
    if !effort.is_empty() {
        entry["tags"] = json!(["reasoning", "tool-use", "vision"]);
        entry["reasoning_options"] = json!([{"type":"effort","values":[effort]}]);
    }
    let catalog = json!({"object":"list","data":[entry]});
    let ok = status(remote, fetch, 200)
        .and_then(|_| push(remote, fetch, catalog.to_string().as_bytes()));
    let _ = remote.fetch_finish(fetch, ok.is_ok());
}

pub(super) fn deliver(
    model: &Model,
    remote: Remote,
    fetch: u64,
    url: &str,
    headers: &[(String, String)],
    body: &[u8],
) {
    let result = match &model.transport {
        Transport::Live(live) => {
            *model.failure.lock().unwrap() = None;
            let result = deliver_live(live, remote, fetch, url, headers, body, &model.failure);
            let mut failure = model.failure.lock().unwrap();
            match &result {
                Ok(()) => {}
                Err(_) if remote.fetch_disposition(fetch) == Disposition::Retired => {}
                // Words the stream's own error was given stay.
                Err(error) => {
                    failure.get_or_insert_with(|| error.clone());
                }
            }
            drop(failure);
            result
        }
        Transport::Fake(fake) => {
            let content = response(fake(body), body);
            if let Some(content) = content {
                status(remote, fetch, 200).and_then(|_| push(remote, fetch, content.as_bytes()))
            } else {
                // A held fake hangs until cancellation retires the fetch.
                while remote.fetch_disposition(fetch) != Disposition::Retired {
                    thread::sleep(Duration::from_millis(4));
                }
                return;
            }
        }
        Transport::Replay(base) => deliver_replay(base, remote, fetch, url, body),
    };
    let _ = remote.fetch_finish(fetch, result.is_ok());
}

fn deliver_replay(
    base: &Url,
    remote: Remote,
    fetch: u64,
    url: &str,
    body: &[u8],
) -> Result<(), String> {
    let url = Url::parse(url).map_err(|e| e.to_string())?;
    if &url != base {
        return Err("Aero refused a model request outside the configured replay endpoint.".into());
    }
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .body(body.to_vec())
        .send()
        .map_err(|e| e.to_string())?;
    status(remote, fetch, response.status().as_u16())?;
    let mut buffer = [0u8; 16 * 1024];
    while remote.fetch_disposition(fetch) != Disposition::Retired {
        let len = response.read(&mut buffer).map_err(|e| e.to_string())?;
        if len == 0 {
            break;
        }
        push(remote, fetch, &buffer[..len])?;
    }
    Ok(())
}

/// Sends the core's request with the source's credential when `url` is on
/// the credential's origin (header rule 13), streaming the answer back. A
/// 401 before any byte reached the core asks for a fresh credential once.
fn deliver_live(
    live: &Live,
    remote: Remote,
    fetch: u64,
    url: &str,
    headers: &[(String, String)],
    body: &[u8],
    failed: &std::sync::Mutex<Option<String>>,
) -> Result<(), String> {
    use crate::edge::{Chunk, NetRequest};
    let origin = pm_aero_core::origin_of(url);
    let mut rejected: Option<String> = None;
    for attempt in 0..2 {
        let (trusted, credentials) = match (live.credentials)(rejected.as_deref()) {
            Ok(found) => found,
            Err(reason) => {
                // The core reads why from the body of a refusal.
                let body =
                    json!({"error":{"message":reason,"code":"powermove_account"}}).to_string();
                return status(remote, fetch, 401)
                    .and_then(|_| push(remote, fetch, body.as_bytes()));
            }
        };
        let mut all: Vec<(String, String)> = headers
            .iter()
            .filter(|(name, _)| {
                !credentials
                    .iter()
                    .any(|(c, _)| c.eq_ignore_ascii_case(name))
            })
            .cloned()
            .collect();
        let bearer = credentials
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .map(|(_, value)| value.clone());
        if origin.as_deref() == Some(trusted.as_str()) {
            all.extend(credentials);
        }
        let mut retry = false;
        let request = NetRequest {
            method: "POST",
            url,
            headers: &all,
            body,
        };
        // Stops the request when the core retires it.
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let (stop, done) = (stop.clone(), done.clone());
            thread::spawn(move || {
                use std::sync::atomic::Ordering::{Acquire, Release};
                while !done.load(Acquire) {
                    if remote.fetch_disposition(fetch) == Disposition::Retired {
                        stop.store(true, Release);
                        return;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            });
        }
        let mut code = 0;
        let mut failure = Vec::new();
        let mut guard = SseGuard::default();
        let streamed = live.net.stream(&request, &stop, &mut |chunk| match chunk {
            Chunk::Status(401) if attempt == 0 && bearer.is_some() => {
                retry = true;
                Ok(())
            }
            Chunk::Status(got) => {
                code = got;
                // A failure's status goes with its safe body, below.
                if (200..300).contains(&got) {
                    status(remote, fetch, got)
                } else {
                    Ok(())
                }
            }
            Chunk::Data(_) if retry => Ok(()),
            Chunk::Data(bytes) if !(200..300).contains(&code) => {
                if failure.len() < 64 * 1024 {
                    failure.extend_from_slice(bytes);
                }
                Ok(())
            }
            Chunk::Data(bytes) => {
                let safe = guard.feed(bytes);
                // Recorded before the core reads the event: it stops at an
                // error and retires the request, ending this delivery early.
                if let Some(words) = guard.failure.take() {
                    *failed.lock().unwrap() = Some(words);
                }
                push(remote, fetch, &safe)
            }
        });
        done.store(true, std::sync::atomic::Ordering::Release);
        if let Err(error) = streamed {
            return Err(if error.starts_with("The model stopped responding") {
                error
            } else if (200..300).contains(&code) {
                "The connection to the model dropped before its reply finished. Try again.".into()
            } else {
                format!("Aero couldn't reach Powermove inference ({error}). Try again.")
            });
        }
        if retry {
            rejected = bearer.map(|b| b.trim_start_matches("Bearer ").to_owned());
            continue;
        }
        if !(200..300).contains(&code) {
            // Never the provider's own bytes: a careless server may repeat the
            // credential it was sent (P1-6, ABI rule 13).
            let shown: Vec<&str> = all.iter().map(|(_, v)| v.as_str()).collect();
            let safe = safe_failure(code, &failure, &shown);
            // Aero's backend: the plan's credits are spent (the contract's 402).
            if code == 402 {
                *failed.lock().unwrap() = Some(
                    "You’re out of Aero credits. They come back when your plan resets.".into(),
                );
            }
            return status(remote, fetch, code).and_then(|_| push(remote, fetch, &safe));
        }
        let rest = guard.finish();
        if let Some(words) = guard.failure.take() {
            *failed.lock().unwrap() = Some(words);
        }
        push(remote, fetch, &rest)?;
        return Ok(());
    }
    Err("The provider refused the refreshed credential.".into())
}

/// What the core hears of a failed provider response: the status and, when
/// it is a plain identifier that repeats none of the request's header
/// values, the provider's error code. Nothing else of its body.
pub(crate) fn safe_failure(code: u16, body: &[u8], sent: &[&str]) -> Vec<u8> {
    let value: Value = serde_json::from_slice(body).unwrap_or_default();
    let kind = [
        &value["error"]["code"],
        &value["error"]["type"],
        &value["error"],
    ]
    .into_iter()
    .find_map(Value::as_str)
    .filter(|kind| {
        !kind.is_empty()
            && kind.len() <= 48
            && kind
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
            && !sent.iter().any(|value| {
                let value = value.trim_start_matches("Bearer ");
                !value.is_empty() && (value.contains(kind) || kind.contains(value))
            })
    });
    let message = match kind {
        Some(kind) => format!("The provider refused the request (HTTP {code}, {kind})."),
        None => format!("The provider refused the request (HTTP {code})."),
    };
    json!({"error": {"message": message, "type": kind}})
        .to_string()
        .into_bytes()
}

/// A provider error's category, kept only when it is one of these, and the
/// words Powermove shows for it. Nothing else of the provider's error is kept.
fn error_words(kind: &str) -> (&'static str, &'static str) {
    let kind = kind.to_ascii_lowercase();
    if kind.contains("overload") {
        (
            "overloaded_error",
            "The model is overloaded right now. Try again in a moment.",
        )
    } else if kind.contains("rate_limit") || kind.contains("quota") {
        (
            "rate_limit_error",
            "The provider's rate limit was reached. Wait a moment and try again.",
        )
    } else if kind.contains("auth")
        || kind.contains("permission")
        || kind.contains("api_key")
        || kind.contains("unauthorized")
    {
        (
            "authentication_error",
            "Powermove refused the session. Sign in to Powermove again in Settings › Providers.",
        )
    } else if kind.contains("invalid_request") {
        (
            "invalid_request_error",
            "The provider refused the request as invalid.",
        )
    } else {
        (
            "api_error",
            "The provider failed while answering. Try again.",
        )
    }
}

/// Error events inside a successful (2xx) server-sent stream, rewritten
/// before the core reads them: the provider's own message can quote the
/// prompt, a header or a key, and the core would show it as the reply
/// (P1-6). The event keeps its place and an allowlisted category; its words
/// are Powermove's. Other events pass through whole, each as soon as it ends.
#[derive(Default)]
pub(crate) struct SseGuard {
    pending: Vec<u8>,
    /// Powermove's words for the error the stream carried.
    pub(crate) failure: Option<String>,
}
impl SseGuard {
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.pending.extend_from_slice(bytes);
        let mut out = Vec::new();
        let end_of = |bytes: &[u8]| {
            let lf = bytes.windows(2).position(|w| w == b"\n\n").map(|at| at + 2);
            let crlf = bytes
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|at| at + 4);
            match (lf, crlf) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            }
        };
        while let Some(end) = end_of(&self.pending) {
            let event: Vec<u8> = self.pending.drain(..end).collect();
            out.extend(self.event(&event));
        }
        out
    }
    /// What is left when the stream ends.
    pub(crate) fn finish(&mut self) -> Vec<u8> {
        let rest = std::mem::take(&mut self.pending);
        if rest.is_empty() {
            rest
        } else {
            self.event(&rest)
        }
    }
    fn event(&mut self, raw: &[u8]) -> Vec<u8> {
        let text = String::from_utf8_lossy(raw);
        let data: String = text
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim_start)
            .collect::<Vec<_>>()
            .join("\n");
        if !data.contains("error") && !data.contains("response.failed") {
            return raw.to_vec();
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            return raw.to_vec();
        };
        let kind_of = |error: &Value| {
            [&error["type"], &error["code"]]
                .into_iter()
                .find_map(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let safe = match value["type"].as_str() {
            Some("error") => {
                let kind = if value["error"].is_object() {
                    kind_of(&value["error"])
                } else {
                    kind_of(&value)
                };
                let (kind, words) = error_words(&kind);
                self.failure = Some(words.into());
                json!({"type":"error","code":kind,"message":words,
                    "error":{"type":kind,"message":words}})
            }
            Some("response.failed") => {
                let (kind, words) = error_words(&kind_of(&value["response"]["error"]));
                self.failure = Some(words.into());
                let mut safe = value.clone();
                safe["response"]["error"] = json!({"code":kind,"message":words});
                safe
            }
            None if value["error"].is_object() => {
                let (kind, words) = error_words(&kind_of(&value["error"]));
                self.failure = Some(words.into());
                json!({"error":{"type":kind,"code":kind,"message":words}})
            }
            _ => return raw.to_vec(),
        };
        let name = text
            .lines()
            .find_map(|line| line.strip_prefix("event:"))
            .map(|name| format!("event: {}\n", name.trim()))
            .unwrap_or_default();
        format!("{name}data: {safe}\n\n").into_bytes()
    }
}

fn status(remote: Remote, fetch: u64, code: u16) -> Result<(), String> {
    match remote
        .fetch_status(fetch, code, None)
        .map_err(|e| e.to_string())?
    {
        Outcome::Applied => Ok(()),
        other => Err(format!("Aero's fetch status was rejected: {other:?}")),
    }
}

fn push(remote: Remote, fetch: u64, bytes: &[u8]) -> Result<(), String> {
    for chunk in bytes.chunks(16 * 1024) {
        loop {
            match remote.fetch_push(fetch, chunk).map_err(|e| e.to_string())? {
                Outcome::Applied => break,
                Outcome::Stale => return Ok(()),
                Outcome::Backpressure | Outcome::Busy => thread::sleep(Duration::from_millis(2)),
                other => return Err(format!("Aero's fetch bytes were rejected: {other:?}")),
            }
        }
    }
    Ok(())
}

fn response(reply: Reply, request: &[u8]) -> Option<String> {
    let mut out = String::new();
    let mut sse = |value: Value| {
        out.push_str("data: ");
        out.push_str(&value.to_string());
        out.push_str("\n\n");
    };
    let mut reply = reply;
    while let Reply::Said(text, then) = reply {
        sse(json!({"type":"text-delta","id":"0","delta":text}));
        reply = *then;
    }
    let reason = match reply {
        Reply::Said(..) => unreachable!("narration was streamed above"),
        Reply::Hold => return None,
        Reply::Text(text) => {
            sse(json!({"type":"text-delta","id":"0","delta":text}));
            "stop"
        }
        Reply::Tool { name, args } => {
            use sha2::{Digest, Sha256};
            let hash = Sha256::digest(request);
            let id = format!("call-{hash:x}");
            let input = args.to_string();
            sse(json!({"type":"tool-input-start","id":id,"toolName":name}));
            sse(json!({"type":"tool-input-delta","id":id,"delta":input}));
            sse(json!({"type":"tool-call","toolCallId":id,"toolName":name,"input":input}));
            "tool-calls"
        }
    };
    sse(
        json!({"type":"finish","finishReason":{"unified":reason,"raw":reason},
               "usage":{"inputTokens":{"total":1},"outputTokens":{"total":1}}}),
    );
    out.push_str("data: [DONE]\n\n");
    Some(out)
}

pub(super) fn demo(bytes: &[u8]) -> Reply {
    let text = String::from_utf8_lossy(bytes);
    if text.contains("Completion checks failed") {
        return Reply::Text("The synthetic run reached the completion checks.".into());
    }
    if text.contains("Aero demo edit") {
        return Reply::Text("Renamed the first layer to Aero demo edit.".into());
    }
    let request: Value = serde_json::from_slice(bytes).unwrap_or_default();
    if let Some(header) = find_text(&request, "Project at session start\n")
        .or_else(|| find_text(&request, "Project updates\n"))
        && let Some(line) = header.lines().next()
        && let Ok(context) = serde_json::from_str::<Value>(line)
        && let Some(revision) = context["composition"]["revision"].as_u64()
        && let Some(id) = context["layers"][0]["id"].as_str()
    {
        return Reply::Tool {
            name: "edit".into(),
            args: json!({"op":"commands","revision":revision,
            "args":{"label":"Aero demo edit","commands":[{"type":"set_layer","target":id,
            "patch":{"name":"Aero demo edit"}}]}}),
        };
    }
    Reply::Text("Aero is running on a fake model. Open a project with a layer and choose Project access to try the demo edit.".into())
}

fn find_text<'a>(value: &'a Value, marker: &str) -> Option<&'a str> {
    match value {
        Value::String(s) => s.split_once(marker).map(|(_, text)| text),
        Value::Array(items) => items.iter().find_map(|item| find_text(item, marker)),
        Value::Object(items) => items.values().find_map(|value| find_text(value, marker)),
        _ => None,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewInput {
    op: String,
    args: Review,
    /// Every tool shares one input schema; review is not revision-bound.
    #[serde(rename = "revision")]
    _revision: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Review {
    result: Value,
    rubric: Value,
    #[serde(default)]
    captures: Vec<String>,
}

pub(super) fn review(
    model: &Model,
    scope: &Path,
    input: &str,
    captures: &BTreeMap<String, (String, Vec<u8>)>,
) -> Result<ToolResult, String> {
    let input: ReviewInput = serde_json::from_str(input)
        .map_err(|e| format!("Review needs a result, a rubric, and optional capture ids: {e}"))?;
    if input.op != "evaluate" {
        return Err("Use review with op evaluate.".into());
    }
    let request = input.args;
    let images: Vec<_> = request
        .captures
        .iter()
        .map(|id| {
            let (mime, data) = captures.get(id).ok_or_else(|| {
                format!("Capture {id} is unavailable. Use look to capture it first.")
            })?;
            Ok(Image { mime, data })
        })
        .collect::<Result<_, String>>()?;
    let evidence = json!({"result":request.result,"rubric":request.rubric});
    evaluate(model, scope, &evidence.to_string(), &images).map(ToolResult::text)
}

pub fn evaluate(
    model: &Model,
    scope: &Path,
    evidence: &str,
    images: &[Image<'_>],
) -> Result<String, String> {
    struct NoTools;
    impl Host for NoTools {}
    let err = |e: pm_aero_core::Error| e.to_string();
    let dir = scope.to_str().ok_or("Invalid reviewer session path")?;
    let name = model.name("fake/model");
    let mut runtime = Runtime::create(&Config {
        provider: model.provider(), configured_providers_json: model.configured(),
        model: name, endpoint_url: model.endpoint(), home: dir, workspace_root: dir,
        instructions: "You are an independent evaluator. Use only the provided result, captures and rubric. Score each criterion with specific evidence. Return the evaluation as JSON.",
        ..Config::default()
    }, Arc::new(NoTools)).map_err(err)?;
    let opening = runtime.open_session("").map_err(err)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    wait_open(&mut runtime, model, opening, (name, ""), deadline)?;
    runtime
        .prompt_input(
            &pm_aero_core::Input {
                text: evidence,
                images,
            },
            "evaluation",
        )
        .map_err(err)?;
    let remote = runtime.remote();
    let mut text = String::new();
    while Instant::now() < deadline {
        if let Some(fetch) = runtime.fetches.take().map_err(err)? {
            if fetch.purpose == pm_aero_core::Purpose::Catalog {
                catalog(model, remote, fetch.fetch, fetch.url, name, "");
            } else {
                let (id, url, body) = (fetch.fetch, fetch.url.to_owned(), fetch.body.to_vec());
                let headers: Vec<_> = fetch
                    .headers()
                    .map(|(k, v)| (k.to_owned(), v.to_owned()))
                    .collect();
                let model = model.clone();
                thread::spawn(move || deliver(&model, remote, id, &url, &headers, &body));
            }
        }
        for event in runtime.events.drain().map_err(err)? {
            match event {
                pm_aero_core::Event::TextDelta { text: delta, .. } => text.push_str(delta),
                pm_aero_core::Event::TurnEnd {
                    reason: StopReason::EndTurn,
                    ..
                } => return Ok(text),
                pm_aero_core::Event::TurnEnd { reason, .. } => {
                    return Err(format!("Evaluator stopped: {reason:?}"));
                }
                pm_aero_core::Event::Error { message, .. } => return Err(message.into()),
                _ => {}
            }
        }
        wait_ready(&runtime);
    }
    Err("Evaluator exceeded its time limit.".into())
}
