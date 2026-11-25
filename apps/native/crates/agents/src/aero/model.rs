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
}

type Fake = Arc<dyn Fn(&[u8]) -> Reply + Send + Sync>;

#[derive(Clone)]
enum Transport {
    Fake(Fake),
    Replay(Url),
}

pub enum Reply {
    Text(String),
    Tool { name: String, args: Value },
    Hold,
}

impl Model {
    pub fn fake(f: impl Fn(&[u8]) -> Reply + Send + Sync + 'static) -> Self {
        Self {
            transport: Transport::Fake(Arc::new(f)),
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
        })
    }

    pub fn endpoint(&self) -> &str {
        match &self.transport {
            Transport::Fake(_) => "http://127.0.0.1:1/aero-fake",
            Transport::Replay(url) => url.as_str(),
        }
    }
}

/// The fake and replay edges answer the catalog GET here, never on the
/// network: the session's model with tool use and vision, so tool images
/// reach it (the core refuses images to a model without vision), and the
/// session's effort as its one reasoning option (the core refuses an effort
/// the model does not list).
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

pub(super) fn deliver(model: &Model, remote: Remote, fetch: u64, url: &str, body: &[u8]) {
    let result = match &model.transport {
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
    let reason = match reply {
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
    let mut runtime = Runtime::create(&Config {
        model: "fake/model", endpoint_url: model.endpoint(), home: dir, workspace_root: dir,
        instructions: "You are an independent evaluator. Use only the provided result, captures and rubric. Score each criterion with specific evidence. Return the evaluation as JSON.",
        ..Config::default()
    }, Arc::new(NoTools)).map_err(err)?;
    let opening = runtime.open_session("").map_err(err)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    wait_open(&mut runtime, opening, ("fake/model", ""), deadline)?;
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
                answer_catalog(remote, fetch.fetch, "fake/model", "");
            } else {
                let (id, url, body) = (fetch.fetch, fetch.url.to_owned(), fetch.body.to_vec());
                let model = model.clone();
                thread::spawn(move || deliver(&model, remote, id, &url, &body));
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
