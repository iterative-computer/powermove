//! Compatible-provider setup on a worker. Secrets stay in an injected native
//! key service; URLs bind each lookup and every HTTP request forbids redirects.
use crate::{error, trace};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
use url::Url;
pub const URL_ERROR: &str = "Use an HTTPS API address, or HTTP for a local model on this Mac.";
pub fn provider_url(value: &str) -> io::Result<String> {
    let text = value.trim();
    let scheme = regex::Regex::new(r"(?i)^[a-z][a-z0-9+.-]*://").unwrap();
    let local = regex::Regex::new(r"(?i)^(localhost|127\.0\.0\.1|\[::1\])(:|/|$)").unwrap();
    let input = if scheme.is_match(text) {
        text.into()
    } else {
        format!(
            "{}://{text}",
            if local.is_match(text) {
                "http"
            } else {
                "https"
            }
        )
    };
    let mut url = Url::parse(&input).map_err(|_| error("Invalid URL"))?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !(url.scheme() == "https" || url.scheme() == "http" && is_local(&url))
    {
        return Err(error(URL_ERROR));
    }
    let path = url
        .path()
        .trim_end_matches('/')
        .strip_suffix("/chat/completions")
        .unwrap_or_else(|| url.path().trim_end_matches('/'))
        .to_owned();
    url.set_path(if path.is_empty() { "/v1" } else { &path });
    Ok(url.to_string().trim_end_matches('/').into())
}
fn is_local(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    )
}
fn valid_id(value: &Value) -> bool {
    value.as_str().is_some_and(|v| {
        !v.trim().is_empty()
            && v.encode_utf16().count() <= 200
            && !v.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
    })
}
pub fn filter_models(
    payload: &Value,
    capabilities: &BTreeMap<String, Vec<String>>,
    local: bool,
    has_key: bool,
) -> Value {
    let entries = payload["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|v| v.is_object())
        .take(2000)
        .collect::<Vec<_>>();
    let mut seen = BTreeSet::new();
    let mut models = vec![];
    let mut hidden = 0;
    for entry in entries {
        if !valid_id(&entry["id"]) {
            continue;
        }
        let id = entry["id"].as_str().unwrap();
        if !seen.insert(id) {
            continue;
        }
        let caps = capabilities.get(id);
        let parameters = entry["supported_parameters"].as_array();
        let tools = caps
            .map(|v| v.iter().any(|v| v == "tools"))
            .or_else(|| parameters.map(|v| v.iter().any(|v| v == "tools")));
        if tools == Some(false) {
            hidden += 1;
            continue;
        }
        let vision = caps.map(|v| v.iter().any(|v| v == "vision")).or_else(|| {
            entry["architecture"]["input_modalities"]
                .as_array()
                .map(|v| v.iter().any(|v| v == "image"))
        });
        let mut model = json!({"id":id});
        if let Some(vision) = vision {
            model["vision"] = json!(vision);
        }
        models.push(model);
    }
    if local {
        models.sort_by_key(|v| v["id"].as_str().unwrap().to_lowercase());
    }
    json!({"models":models,"hidden":hidden,"hasKey":has_key})
}
pub struct Secret(String);
impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}
pub struct Request<'a> {
    pub method: &'a str,
    pub url: String,
    pub body: Option<Value>,
    pub key: Option<&'a Secret>,
    pub timeout_ms: u64,
}
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}
impl Response {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
    fn json(&self) -> io::Result<Value> {
        serde_json::from_slice(&self.body).map_err(|_|error("This address did not return a model list. Check that it is the API address, usually ending in /v1."))
    }
}
pub trait Host {
    /// Refuse redirects and apply streaming byte/time bounds before buffering.
    fn request(&mut self, request: Request<'_>, max_bytes: usize) -> io::Result<Response>;
    /// Address is canonicalized before lookup, including legacy-key migration.
    /// Implement via Keychain. Never log or serialize the returned secret.
    fn key(&mut self, address: &str) -> io::Result<Option<Secret>>;
    fn config(&mut self) -> io::Result<Value>;
    /// Compare expected config, archive replaced metadata/keys (no key GC), and
    /// durably commit public metadata plus this address's opaque secret handle.
    fn publish(
        &mut self,
        expected: &Value,
        public: &Value,
        address: &str,
        key: Option<&Secret>,
    ) -> io::Result<()>;
}
fn request(
    host: &mut impl Host,
    method: &str,
    url: String,
    body: Option<Value>,
    key: Option<&Secret>,
    timeout_ms: u64,
    max: usize,
) -> io::Result<Response> {
    let host_name = url::Url::parse(&url)
        .ok()
        .map(|u| u.host_str().unwrap_or_default().to_owned());
    let request_url = url.clone();
    let response = host.request(
        Request {
            method,
            url,
            body,
            key,
            timeout_ms,
        },
        max,
    ).map_err(|failure| {
        let parsed=url::Url::parse(&request_url).ok();
        let host=parsed.as_ref().map(|u|u.host().map(|h|h.to_string()).unwrap_or_default()).unwrap_or_else(||host_name.unwrap_or_default());
        let host=parsed.as_ref().and_then(|u|u.port().map(|p|format!("{host}:{p}"))).unwrap_or(host);
        let message=if failure.kind()==io::ErrorKind::TimedOut {format!("{host} took too long to answer. Check the address, then try again.")}
            else if parsed.as_ref().is_some_and(is_local) {
                match parsed.as_ref().and_then(|u|u.port()) {
                    Some(11434)=>"Ollama isn’t running. Open the Ollama app, then try again.".into(),
                    Some(1234)=>format!("LM Studio isn’t answering at {host}. Open it and start its local server, then try again."),
                    Some(8080)=>format!("llama.cpp server isn’t answering at {host}. Open it and start its local server, then try again."),
                    Some(1337)=>format!("Jan isn’t answering at {host}. Open it and start its local server, then try again."),
                    _=>format!("Nothing is answering at {host}. Start your local server, then try again."),
                }
            }else{format!("Couldn’t reach {host}. Check the address and your internet connection.")};
        io::Error::new(failure.kind(),message)
    })?;
    if response.body.len() > max {
        return Err(error("The provider response is too large."));
    }
    Ok(response)
}
fn detail(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(&body[..body.len().min(16000)]);
    let value = serde_json::from_str::<Value>(&text);
    let text = match value {
        Ok(v) => {
            let v = v
                .get("error")
                .or_else(|| v.get("message"))
                .or_else(|| v.get("detail"));
            v.and_then(|v| v.as_str().or_else(|| v["message"].as_str()))
                .unwrap_or_default()
                .to_owned()
        }
        Err(_) => text.into_owned(),
    };
    let text = text
        .chars()
        .map(|c| {
            if c <= '\u{1f}' || c == '\u{7f}' {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.to_lowercase().starts_with("<!doctype") || text.to_lowercase().starts_with("<html") {
        return String::new();
    }
    if text.encode_utf16().count() > 240 {
        format!("{}…", trace::truncate(&text, 239))
    } else {
        text
    }
}
pub fn http_error(status: u16, body: &[u8], model: &str) -> String {
    let detail = detail(body);
    if matches!(status, 401 | 403) {
        return "The provider rejected the API key. Check the key in Settings.".into();
    }
    let lower = detail.to_lowercase();
    if regex::Regex::new(r"\btools?\b").unwrap().is_match(&lower)
        && ["support", "enable", "allow"]
            .iter()
            .any(|v| lower.contains(v))
    {
        return format!(
            "{} can chat but can’t use tools, so it can’t edit your project. Choose a model with tool support.",
            if model.is_empty() {
                "This model"
            } else {
                model
            }
        );
    }
    match status {
        429 => format!(
            "The provider is rate limited or out of credit{}. Wait a moment, then try again.",
            if detail.is_empty() {
                String::new()
            } else {
                format!(" ({detail})")
            }
        ),
        404 => {
            if detail.is_empty() {
                "The API address or model was not found. Check the connection in Settings.".into()
            } else {
                format!("Not found: {detail}")
            }
        }
        _ => format!(
            "The model provider returned HTTP {status}{}. Your conversation is kept; try again.",
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        ),
    }
}
fn ollama(host: &mut impl Host, base: &Url) -> bool {
    request(
        host,
        "GET",
        format!("{}/api/version", base.origin().ascii_serialization()),
        None,
        None,
        1500,
        1024 * 1024,
    )
    .ok()
    .filter(Response::ok)
    .and_then(|v| v.json().ok())
    .is_some_and(|v| v["version"].is_string())
}
pub fn list_models(host: &mut impl Host, base: &str, key_input: Option<&str>) -> io::Result<Value> {
    if base.encode_utf16().count() > 2000
        || key_input.is_some_and(|key| key.encode_utf16().count() > 8192)
    {
        return Err(error("Enter the API address."));
    }
    let base = provider_url(base)?;
    let url = Url::parse(&base).unwrap();
    let saved = host.key(&base)?;
    let has_key = saved.is_some();
    let supplied = key_input
        .filter(|v| !v.trim().is_empty())
        .map(|v| Secret::new(v.trim().into()));
    let key = supplied.as_ref().or(saved.as_ref());
    let response = request(
        host,
        "GET",
        format!("{base}/models"),
        None,
        key,
        15000,
        20_000_000,
    )?;
    if matches!(response.status, 404 | 405) {
        return Ok(json!({"models":[],"hidden":0,"hasKey":has_key}));
    }
    if !response.ok() {
        return Err(error(http_error(response.status, &response.body, "")));
    }
    let payload = response.json()?;
    let mut capabilities = BTreeMap::new();
    if is_local(&url)
        && payload["data"].as_array().is_some_and(|v| !v.is_empty())
        && ollama(host, &url)
    {
        let mut seen = BTreeSet::new();
        for entry in payload["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| v.is_object())
            .take(2000)
        {
            if !valid_id(&entry["id"]) {
                continue;
            }
            let id = entry["id"].as_str().unwrap();
            if !seen.insert(id) || seen.len() > 60 {
                continue;
            }
            if let Ok(response) = request(
                host,
                "POST",
                format!("{}/api/show", url.origin().ascii_serialization()),
                Some(json!({"model":id})),
                None,
                5000,
                1024 * 1024,
            ) && response.ok()
                && let Ok(body) = response.json()
                && let Some(caps) = body["capabilities"].as_array()
            {
                capabilities.insert(
                    id.into(),
                    caps.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect(),
                );
            }
        }
    }
    Ok(filter_models(
        &payload,
        &capabilities,
        is_local(&url),
        has_key,
    ))
}
pub fn configure(host: &mut impl Host, input: &Value) -> io::Result<Value> {
    let base = input["baseUrl"]
        .as_str()
        .filter(|s| s.encode_utf16().count() <= 2000)
        .ok_or_else(|| error("Enter the API address and model name."))?;
    let model = input["model"]
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.encode_utf16().count() <= 200)
        .ok_or_else(|| error("Enter the API address and model name."))?
        .trim();
    if !input["vision"].is_boolean()
        || input
            .get("apiKey")
            .is_some_and(|v| v.as_str().is_none_or(|s| s.encode_utf16().count() > 8192))
    {
        return Err(error("Enter the API address and model name."));
    }
    let base = provider_url(base)?;
    let previous = host.config()?;
    let saved = host.key(&base)?;
    let supplied = input["apiKey"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| Secret::new(s.trim().into()));
    let key = supplied.as_ref().or(saved.as_ref());
    let response = request(
        host,
        "POST",
        format!("{base}/chat/completions"),
        Some(
            json!({"model":model,"messages":[{"role":"user","content":"Reply with OK."}],"tools":[{"type":"function","function":{"name":"powermove_check","description":"Not needed; reply with text.","parameters":{"type":"object","properties":{}}}}],"stream":false}),
        ),
        key,
        60000,
        2_000_000,
    )?;
    if !response.ok() {
        return Err(error(http_error(response.status, &response.body, model)));
    }
    let body = response.json()?;
    if !body["choices"][0]["message"].is_object() {
        return Err(error(
            "This address did not return a compatible chat response. Check the API address and model.",
        ));
    }
    let mut models = vec![model.to_owned()];
    if let Ok(response) = request(
        host,
        "GET",
        format!("{base}/models"),
        None,
        key,
        10000,
        20_000_000,
    ) && response.ok()
        && let Ok(body) = response.json()
    {
        for item in body["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| valid_id(&v["id"]))
            .take(1000)
        {
            let id = item["id"].as_str().unwrap().to_owned();
            if !models.contains(&id) {
                models.push(id);
            }
        }
    }
    let mut config = json!({"baseUrl":base,"model":model,"vision":input["vision"],"hasKey":key.is_some(),"models":models});
    let url = Url::parse(&base).unwrap();
    if ollama(host, &url)
        && let Ok(response) = request(
            host,
            "GET",
            format!("{}/api/ps", url.origin().ascii_serialization()),
            None,
            None,
            3000,
            1024 * 1024,
        )
        && response.ok()
        && let Ok(body) = response.json()
    {
        let matches = |v: &Value| {
            v.as_str().is_some_and(|name| {
                name == model
                    || name == format!("{model}:latest")
                    || format!("{name}:latest") == model
            })
        };
        if let Some(running) = body["models"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|v| matches(&v["name"]) || matches(&v["model"]))
        {
            let context = running["context_length"]
                .as_f64()
                .or_else(|| {
                    running["context_length"]
                        .as_str()
                        .and_then(|s| s.parse().ok())
                })
                .unwrap_or(0.);
            if context > 0. && context < 16384. {
                let integer = format!("{context:.0}");
                let formatted = integer
                    .chars()
                    .enumerate()
                    .map(|(i, c)| {
                        format!(
                            "{}{c}",
                            if i > 0 && (integer.len() - i).is_multiple_of(3) {
                                ","
                            } else {
                                ""
                            }
                        )
                    })
                    .collect::<String>();
                config["warning"] = json!(format!(
                    "Ollama is running {model} with only {formatted} tokens of context, so it will lose track of Powermove’s instructions. In Ollama’s settings, set Context length to 32k or more (or set OLLAMA_CONTEXT_LENGTH=32768), then test again."
                ));
            }
        }
    }
    host.publish(&previous, &config, &base, key)?;
    Ok(config)
}
pub fn detect_local(host: &mut impl Host) -> Vec<Value> {
    [
        ("ollama", "http://localhost:11434/v1"),
        ("lmstudio", "http://localhost:1234/v1"),
        ("llamacpp", "http://localhost:8080/v1"),
        ("jan", "http://localhost:1337/v1"),
    ]
    .into_iter()
    .filter_map(|(preset, base)| {
        let response = request(
            host,
            "GET",
            format!("{base}/models"),
            None,
            None,
            1200,
            20_000_000,
        )
        .ok()?;
        if !response.ok() {
            return None;
        }
        let body = response.json().ok()?;
        Some(json!({"preset":preset,"baseUrl":base,"models":body["data"].as_array()?.len()}))
    })
    .collect()
}

/// Public connection metadata executed from the owner-pinned main source.
/// Presentation uses the corresponding Opus spec; no layout is defined here.
pub fn presets() -> &'static [Value] {
    static PRESETS: std::sync::LazyLock<Vec<Value>> = std::sync::LazyLock::new(|| {
        serde_json::from_str(include_str!("../data/compatible-presets-main.json"))
            .expect("generated public provider metadata")
    });
    &PRESETS
}
pub fn preset_for(base: &str) -> &'static Value {
    let normalized = provider_url(base).unwrap_or_default();
    presets()
        .iter()
        .find(|p| {
            p["baseUrl"]
                .as_str()
                .is_some_and(|v| !v.is_empty() && v == normalized)
        })
        .unwrap_or_else(|| presets().last().unwrap())
}
pub fn default_config() -> Value {
    json!({"baseUrl":"http://localhost:11434/v1","model":"","vision":false,"hasKey":false})
}
