//! Settings › Accounts › API or Local Model (A01/S071): connect an
//! OpenAI-compatible API, Ollama or LM Studio. A native sheet takes the
//! address, the key (an `NSSecureTextField`; the key goes to the Keychain per
//! address and never into Settings' tree or a file), the model with
//! discovery (`/models`, Ollama capabilities) and whether it reads images;
//! Test and Connect runs a real chat completion before anything is saved
//! (`pm-services` providers). The connection is `<profile>/agent-provider.json`
//! in v1's shape, without its sealed secret.
use crate::cloud::{Keychain, Vault};
use crate::sheets::{OpenSheet, from_surface};
use gpui::{App, AsyncApp, Global};
use objc2::rc::Retained;
use objc2_app_kit::NSWindow;
use pm_services::{
    ServiceError, ServiceErrorCode,
    surfaces::{
        Flight,
        agent_auth::{ProviderInput, Secret, provider_url},
        providers::{
            ConfigureHost, ConfigureWork, Configured, DiscoveryHost, DiscoveryWork, ModelInfo,
            Response, preset_for,
        },
    },
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};

/// Keys by address: `<bundle id> Model Provider` / the normalized endpoint.
pub type Keys = Box<dyn Fn(&str) -> Box<dyn Vault> + Send>;
pub fn keychain() -> Keys {
    Box::new(|endpoint| Box::new(Keychain::named("Model Provider", endpoint)))
}
/// The provider's HTTP side and the saved connection, on a worker.
pub struct Host {
    pub profile: PathBuf,
    pub keys: Keys,
}
fn send(
    method: &str,
    url: &str,
    key: Option<&Secret>,
    body: Option<&Value>,
    seconds: u64,
) -> pm_services::Result<Response> {
    let mut headers = vec![];
    if let Some(key) = key {
        headers.push((
            "Authorization".to_owned(),
            format!("Bearer {}", key.expose()),
        ));
    }
    if body.is_some() {
        headers.push(("Content-Type".into(), "application/json".into()));
    }
    // The key goes only to this address: a redirect may not leave its origin.
    let origin = url::Url::parse(url)
        .map(|u| u.origin().ascii_serialization())
        .map_err(|_| ServiceError::message(ServiceErrorCode::InvalidInput, "address"))?;
    let response = pm_compat_runner::net::send(
        pm_runtime_v8::FetchRequest {
            url: url.into(),
            method: method.into(),
            headers,
            body: body.map(|b| b.to_string().into_bytes()).unwrap_or_default(),
            max_response_bytes: 80 * 1024 * 1024,
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(seconds),
        },
        pm_compat_runner::net::Reach::Origin(origin_static(origin)),
    )
    .map_err(|_| ServiceError::message(ServiceErrorCode::Offline, "network"))?;
    Ok(Response {
        status: response.status,
        body: response.body,
    })
}
/// One leaked string per origin ever reached (a handful per person).
fn origin_static(origin: String) -> &'static str {
    static SEEN: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap();
    if let Some(found) = seen.iter().find(|o| **o == origin) {
        return found;
    }
    let leaked: &'static str = Box::leak(origin.into_boxed_str());
    seen.push(leaked);
    leaked
}
/// `http://host:11434` of `http://host:11434/v1`.
fn server_root(endpoint: &str) -> String {
    endpoint
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .to_owned()
}
impl DiscoveryHost for Host {
    fn list(&mut self, endpoint: &str, key: Option<&Secret>) -> pm_services::Result<Response> {
        send("GET", &format!("{endpoint}/models"), key, None, 15)
    }
    fn ollama_capabilities(
        &mut self,
        endpoint: &str,
        ids: &[String],
    ) -> pm_services::Result<BTreeMap<String, Vec<String>>> {
        let root = server_root(endpoint);
        let version = send("GET", &format!("{root}/api/version"), None, None, 2)?;
        if version.status != 200 {
            return Ok(BTreeMap::new());
        }
        let mut caps = BTreeMap::new();
        for id in ids.iter().take(60) {
            let Ok(show) = send(
                "POST",
                &format!("{root}/api/show"),
                None,
                Some(&json!({ "model": id })),
                5,
            ) else {
                continue;
            };
            let value: Value = serde_json::from_slice(&show.body).unwrap_or_default();
            if let Some(list) = value["capabilities"].as_array() {
                caps.insert(
                    id.clone(),
                    list.iter()
                        .filter_map(|c| c.as_str().map(str::to_owned))
                        .collect(),
                );
            }
        }
        Ok(caps)
    }
    fn sort_local(&self, models: &mut [ModelInfo]) {
        models.sort_by_key(|m| m.id.to_lowercase());
    }
}
impl ConfigureHost for Host {
    fn test(
        &mut self,
        endpoint: &str,
        key: Option<&Secret>,
        body: Value,
    ) -> pm_services::Result<Response> {
        send(
            "POST",
            &format!("{endpoint}/chat/completions"),
            key,
            Some(&body),
            60,
        )
    }
    fn models(&mut self, endpoint: &str, key: Option<&Secret>) -> pm_services::Result<Value> {
        let response = send("GET", &format!("{endpoint}/models"), key, None, 10)?;
        Ok(serde_json::from_slice(&response.body).unwrap_or_default())
    }
    fn running_ollama(&mut self, endpoint: &str) -> pm_services::Result<Option<Value>> {
        let response = send(
            "GET",
            &format!("{}/api/ps", server_root(endpoint)),
            None,
            None,
            2,
        )?;
        Ok((response.status == 200)
            .then(|| serde_json::from_slice(&response.body).ok())
            .flatten())
    }
    fn publish(&mut self, config: &Configured, key: Option<&Secret>) -> pm_services::Result<()> {
        let unavailable = |e: String| ServiceError::message(ServiceErrorCode::Unavailable, e);
        if let Some(key) = key {
            (self.keys)(&config.endpoint)
                .write(key.expose().as_bytes())
                .map_err(unavailable)?;
        }
        let saved = json!({"baseUrl":config.endpoint,"model":config.model,"vision":config.vision,
            "hasKey":config.has_key,"models":config.models});
        let path = self.profile.join("agent-provider.json");
        let temporary = self.profile.join(".agent-provider.json.tmp");
        std::fs::write(
            &temporary,
            serde_json::to_vec_pretty(&saved).unwrap_or_default(),
        )
        .and_then(|()| std::fs::rename(&temporary, &path))
        .map_err(|e| unavailable(e.to_string()))
    }
}
impl Host {
    /// The saved connection (v1's `agent-provider.json`).
    pub fn saved(&self) -> Option<Value> {
        let value: Value =
            serde_json::from_slice(&std::fs::read(self.profile.join("agent-provider.json")).ok()?)
                .ok()?;
        value["model"]
            .as_str()
            .is_some_and(|m| !m.is_empty())
            .then_some(value)
    }
    pub fn saved_key(&self, endpoint: &str) -> Option<String> {
        let bytes = (self.keys)(endpoint).read().ok()??;
        String::from_utf8(bytes).ok()
    }
    /// Disconnect: the connection goes; keys stay for a reconnect (S071).
    pub fn disconnect(&self) -> std::io::Result<()> {
        match std::fs::remove_file(self.profile.join("agent-provider.json")) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// The saved connection for agent runs (`pm_agents::compatible`): the file,
/// the Keychain key per address and a streamed chat POST held to its origin.
#[derive(Debug)]
pub struct Chat {
    pub profile: PathBuf,
}
impl pm_agents::compatible::ChatHost for Chat {
    fn saved(&self) -> Option<pm_agents::compatible::Saved> {
        let bytes = std::fs::read(self.profile.join("agent-provider.json")).ok()?;
        serde_json::from_slice::<pm_agents::compatible::Saved>(&bytes)
            .ok()
            .filter(|saved| !saved.model.is_empty() && !saved.base_url.is_empty())
    }
    fn key(&self, endpoint: &str) -> Option<String> {
        let bytes = Keychain::named("Model Provider", endpoint).read().ok()??;
        String::from_utf8(bytes).ok()
    }
    fn stream(
        &self,
        url: &str,
        key: Option<&str>,
        body: Vec<u8>,
        stop: &std::sync::atomic::AtomicBool,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<u16, String> {
        use pm_compat_runner::net::{self, Reach, Streamed};
        use std::sync::{atomic::Ordering, mpsc};
        let origin = url::Url::parse(url)
            .map(|u| u.origin().ascii_serialization())
            .map_err(|e| e.to_string())?;
        let mut headers = vec![("Content-Type".to_owned(), "application/json".to_owned())];
        if let Some(key) = key {
            headers.push(("Authorization".into(), format!("Bearer {key}")));
        }
        let (tx, chunks) = mpsc::channel();
        let mut request = net::stream(
            pm_runtime_v8::FetchRequest {
                url: url.into(),
                method: "POST".into(),
                headers,
                body,
                max_response_bytes: 4 * 1024 * 1024,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(3600),
            },
            Reach::Origin(origin_static(origin)),
            tx,
        )
        .map_err(|e| e.to_string())?;
        let mut status = None;
        loop {
            if stop.load(Ordering::Acquire) {
                request.cancel();
                return Err("Stopped.".into());
            }
            match chunks.recv_timeout(std::time::Duration::from_millis(20)) {
                Ok(Streamed::Status(code)) => status = Some(code),
                Ok(Streamed::Data(bytes)) => {
                    if status.is_some_and(|s| (200..300).contains(&s))
                        && let Err(error) = sink(&bytes)
                    {
                        request.cancel();
                        return Err(error);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Some(done) = request.poll() {
                        // Bytes sent before completion are still queued.
                        while let Ok(chunk) = chunks.try_recv() {
                            match chunk {
                                Streamed::Status(code) => status = Some(code),
                                Streamed::Data(bytes)
                                    if status.is_some_and(|s| (200..300).contains(&s)) =>
                                {
                                    sink(&bytes)?
                                }
                                Streamed::Data(_) => {}
                            }
                        }
                        let response = done.map_err(|e| e.to_string())?;
                        return Ok(status.unwrap_or(response.status));
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let response = request
                        .poll()
                        .unwrap_or_else(|| Err(pm_runtime_v8::Error::Host("ended".into())))
                        .map_err(|e| e.to_string())?;
                    return Ok(status.unwrap_or(response.status));
                }
            }
        }
    }
}

/// The connection types the sheet offers: (label, endpoint).
pub const TYPES: [(&str, &str); 4] = [
    ("OpenAI API", "https://api.openai.com/v1"),
    ("Ollama", "http://localhost:11434/v1"),
    ("LM Studio", "http://localhost:1234/v1"),
    ("Other compatible provider", ""),
];
fn type_of(endpoint: &str) -> usize {
    match preset_for(endpoint).id {
        "openai" => 0,
        "ollama" => 1,
        "lmstudio" => 2,
        _ => 3,
    }
}

struct Sheet {
    open: Option<OpenSheet>,
    window: Retained<NSWindow>,
    kind: usize,
    url: String,
    model: String,
    vision: bool,
    models: Vec<String>,
    note: Option<String>,
    flight: Flight,
}
struct Provider {
    profile: PathBuf,
    saved: Option<Value>,
    revision: u64,
    sheet: Option<Sheet>,
}
impl Global for Provider {}
fn host(profile: PathBuf) -> Host {
    Host {
        profile,
        keys: keychain(),
    }
}
/// Reads the saved connection (off the main thread) for Settings.
pub fn init(profile: PathBuf, cx: &mut App) {
    if cx.has_global::<Provider>() {
        return;
    }
    cx.set_global(Provider {
        profile: profile.clone(),
        saved: None,
        revision: 1,
        sheet: None,
    });
    let read = cx
        .background_executor()
        .spawn(async move { host(profile).saved() });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let saved = read.await;
        cx.update(|cx| {
            cx.global_mut::<Provider>().saved = saved;
            changed(cx);
        });
    })
    .detach();
}
fn changed(cx: &mut App) {
    cx.global_mut::<Provider>().revision += 1;
    // Deferred: a command arrives inside its editor's update, and every
    // editor (that one too) republishes Settings.
    cx.defer(|cx| {
        for editor in crate::entry::editors(cx) {
            editor.update(cx, |c, cx| c.update_builtins(cx));
        }
    });
}
/// Settings › Accounts reads this through the built-in facts. No key.
pub fn fact(cx: &App) -> Value {
    let Some(p) = cx.try_global::<Provider>() else {
        return Value::Null;
    };
    let busy = p.sheet.is_some();
    match &p.saved {
        Some(saved) => {
            let endpoint = saved["baseUrl"].as_str().unwrap_or_default();
            json!({"revision":p.revision,"connected":true,"type":TYPES[type_of(endpoint)].0,
                "url":endpoint,"model":saved["model"],"vision":saved["vision"],"hasKey":saved["hasKey"],"busy":busy})
        }
        None => json!({"revision":p.revision,"connected":false,"busy":busy}),
    }
}
/// Settings › Accounts › Set Up… / Change…: the connection sheet on `window`.
pub fn open(window: Retained<NSWindow>, cx: &mut App) {
    if !cx.has_global::<Provider>() || cx.global::<Provider>().sheet.is_some() {
        return;
    }
    let p = cx.global_mut::<Provider>();
    let saved = p.saved.clone().unwrap_or_default();
    let url = saved["baseUrl"].as_str().unwrap_or(TYPES[1].1).to_owned();
    p.sheet = Some(Sheet {
        open: None,
        window,
        kind: type_of(&url),
        url,
        model: saved["model"].as_str().unwrap_or_default().into(),
        vision: saved["vision"] == true,
        models: saved["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m.as_str().map(str::to_owned))
            .collect(),
        note: None,
        flight: Flight::default(),
    });
    present(cx);
}
fn present(cx: &mut App) {
    use crate::dialogs::{Button, DialogId, Field, NativePiece, Surface, WindowId};
    let has_key = {
        let p = cx.global::<Provider>();
        p.saved.as_ref().is_some_and(|s| s["hasKey"] == true)
    };
    let Some(sheet) = cx.global_mut::<Provider>().sheet.as_mut() else {
        return;
    };
    if let Some(open) = sheet.open.take() {
        open.close();
    }
    let popup = |id: &str, label: &str, values: Vec<String>, selected: usize| Field {
        selected,
        ..Field::new(id, label, NativePiece::PopUpButton, values)
    };
    let text = |id: &str, label: &str, value: &str| {
        Field::new(id, label, NativePiece::TextField, vec![value.into()])
    };
    let mut key = text("key", "API Key", "");
    key.secure = true;
    key.placeholder = Some(if has_key {
        "Saved. Leave blank to keep it."
    } else {
        "Optional for local models"
    });
    let mut model = text("model", "Model Name", &sheet.model);
    model.placeholder = Some("The exact model name your provider expects");
    let fields = vec![
        popup(
            "type",
            "Connection Type",
            TYPES.iter().map(|t| t.0.to_owned()).collect(),
            sheet.kind,
        ),
        text("url", "API Base URL", &sheet.url),
        key,
        model,
        popup(
            "vision",
            "Reads Images",
            vec!["No".into(), "Yes".into()],
            usize::from(sheet.vision),
        ),
    ];
    let mut surface = Surface::sheet(
        DialogId("model-provider".into()),
        WindowId("settings".into()),
        "Connect a Model",
        fields,
        "Test and Connect",
    );
    surface.line = Some(match sheet.models.is_empty() {
        true => {
            "Use an API, Ollama, or LM Studio. Find Models lists what the address offers.".into()
        }
        false => format!(
            "Models here: {}",
            sheet
                .models
                .iter()
                .take(12)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ),
    });
    surface.warning = sheet.note.clone();
    surface.buttons = vec![
        Button::cancel(false),
        Button {
            id: "find",
            label: "Find Models",
            default: false,
            destructive: false,
            enabled: true,
        },
        Button::action("Test and Connect"),
    ];
    let tokens = pm_shell::tokens();
    let open = OpenSheet::present(from_surface(&surface, &tokens), &sheet.window, Some("url"));
    let actions = open.actions.clone();
    sheet.open = Some(open);
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Ok(action) = actions.recv().await {
            if ["cancel", "find", "apply", "type"].contains(&action.as_str()) {
                cx.update(|cx| act(&action, cx));
            }
        }
    })
    .detach();
}
/// The sheet's current entries, read from its controls.
fn read(sheet: &mut Sheet) -> String {
    let Some(open) = &sheet.open else {
        return String::new();
    };
    sheet.kind = open
        .popup("type")
        .unwrap_or(sheet.kind)
        .min(TYPES.len() - 1);
    sheet.url = open.text("url").trim().to_owned();
    // An address of its own is an Other provider, whatever was chosen first.
    if TYPES[sheet.kind].1 != sheet.url {
        sheet.kind = type_of(&sheet.url);
    }
    sheet.model = open.text("model").trim().to_owned();
    sheet.vision = open.popup("vision") == Some(1);
    open.text("key").trim().to_owned()
}
fn act(action: &str, cx: &mut App) {
    let profile = cx.global::<Provider>().profile.clone();
    let saved = cx.global::<Provider>().saved.clone();
    let Some(sheet) = cx.global_mut::<Provider>().sheet.as_mut() else {
        return;
    };
    let entered = read(sheet);
    match action {
        "cancel" => {
            if let Some(open) = sheet.open.take() {
                open.close();
            }
            cx.global_mut::<Provider>().sheet = None;
            changed(cx);
        }
        // A connection type fills in its usual address.
        "type" => {
            let endpoint = TYPES[sheet.kind].1;
            if let Some(open) = &sheet.open
                && !endpoint.is_empty()
            {
                open.set_text("url", endpoint);
                sheet.url = endpoint.into();
            }
        }
        "find" | "apply" => {
            let Some(endpoint) = provider_url(&sheet.url) else {
                sheet.note = Some(
                    "Enter an address that starts with https://, or http:// for this Mac.".into(),
                );
                present(cx);
                return;
            };
            let ticket = sheet.flight.start();
            let (model, vision) = (sheet.model.clone(), sheet.vision);
            let saved_url = saved
                .as_ref()
                .and_then(|s| s["baseUrl"].as_str())
                .map(str::to_owned);
            let find = action == "find";
            let task = cx.background_executor().spawn(async move {
                let mut host = host(profile);
                let saved_key = saved_url.as_deref().and_then(|u| host.saved_key(u));
                let saved_pair = saved_url.as_deref().zip(saved_key.as_deref());
                if find {
                    let key = (!entered.is_empty()).then(|| Secret::new(entered.clone()));
                    let key = key.or_else(|| {
                        saved_pair
                            .filter(|(u, _)| provider_url(u).as_deref() == Some(endpoint.as_str()))
                            .map(|(_, k)| Secret::new(k.to_owned()))
                    });
                    let work = DiscoveryWork {
                        ticket,
                        endpoint,
                        has_saved_key: key.is_some(),
                        key,
                    };
                    return Outcome::Found(work.run(&mut host).result);
                }
                let Some(input) =
                    ProviderInput::prepare(&endpoint, &model, vision, &entered, saved_pair)
                else {
                    return Outcome::Problem("Enter the API address and model name.".into());
                };
                Outcome::Connected(ConfigureWork { ticket, input }.run(&mut host).result)
            });
            cx.spawn(async move |cx: &mut AsyncApp| {
                let outcome = task.await;
                cx.update(|cx| finish(ticket, outcome, cx));
            })
            .detach();
        }
        _ => {}
    }
}
enum Outcome {
    Found(pm_services::Result<pm_services::surfaces::providers::ModelList>),
    Connected(pm_services::Result<Configured>),
    Problem(String),
}
fn finish(ticket: pm_services::surfaces::Ticket, outcome: Outcome, cx: &mut App) {
    let Some(sheet) = cx.global_mut::<Provider>().sheet.as_mut() else {
        return;
    };
    if !sheet.flight.finish(ticket) {
        return;
    }
    match outcome {
        Outcome::Found(Ok(list)) => {
            sheet.models = list.models.iter().map(|m| m.id.clone()).collect();
            if sheet.model.is_empty()
                && let Some(first) = list.models.first()
            {
                sheet.model = first.id.clone();
                sheet.vision = first.vision == Some(true);
            }
            sheet.note = sheet
                .models
                .is_empty()
                .then(|| "This address lists no models. Enter the model name yourself.".into());
            present(cx);
        }
        Outcome::Found(Err(error)) | Outcome::Connected(Err(error)) => {
            sheet.note = Some(failure(&error));
            present(cx);
        }
        Outcome::Problem(text) => {
            sheet.note = Some(text);
            present(cx);
        }
        Outcome::Connected(Ok(config)) => {
            if let Some(open) = sheet.open.take() {
                open.close();
            }
            let p = cx.global_mut::<Provider>();
            p.sheet = None;
            p.saved = Some(
                json!({"baseUrl":config.endpoint,"model":config.model,"vision":config.vision,
                "hasKey":config.has_key,"models":config.models}),
            );
            changed(cx);
        }
    }
}
/// A failed test or lookup, as one sentence (v1 compatible-provider copy).
fn failure(error: &ServiceError) -> String {
    match error.code {
        ServiceErrorCode::Offline => "Can’t reach this address. Check that the server is running and the address is right.".into(),
        ServiceErrorCode::PermissionDenied => "The provider refused the API key. Check it and try again.".into(),
        ServiceErrorCode::InvalidData => "This address did not return a compatible chat response. Check the API address and model.".into(),
        _ => "The API address or model was not found. Check the connection.".into(),
    }
}
/// Settings › Accounts › Disconnect.
pub fn disconnect(cx: &mut App) {
    if !cx.has_global::<Provider>() {
        return;
    }
    let profile = cx.global::<Provider>().profile.clone();
    let task = cx
        .background_executor()
        .spawn(async move { host(profile).disconnect() });
    cx.spawn(async move |cx: &mut AsyncApp| {
        if task.await.is_ok() {
            cx.update(|cx| {
                cx.global_mut::<Provider>().saved = None;
                changed(cx);
            });
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::Chat;
    use pm_agents::compatible::{ChatHost, Stream};
    use std::{
        io::BufRead,
        sync::atomic::{AtomicBool, Ordering},
        time::{Duration, Instant},
    };
    /// The fake OpenAI-compatible server; killed on drop.
    struct Fake(std::process::Child, u16);
    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn fake() -> Fake {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../builtins/fixtures/fake-openai.py");
        let mut child = std::process::Command::new(script)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let port = line.trim().parse().unwrap();
        Fake(child, port)
    }
    fn body(prompt: &str) -> Vec<u8> {
        serde_json::json!({"model":"fixture-model","stream":true,
            "messages":[{"role":"user","content":prompt}]})
        .to_string()
        .into_bytes()
    }
    #[test]
    fn the_saved_connection_streams_its_reply_as_it_arrives_and_stops() {
        let profile = tempfile::tempdir().unwrap();
        let server = fake();
        let base = format!("http://127.0.0.1:{}/v1", server.1);
        std::fs::write(
            profile.path().join("agent-provider.json"),
            serde_json::json!({"baseUrl":base,"model":"fixture-model","vision":false,
                "hasKey":false,"models":["fixture-model"]})
            .to_string(),
        )
        .unwrap();
        let chat = Chat {
            profile: profile.path().into(),
        };
        let saved = chat.saved().unwrap();
        assert_eq!(saved.model, "fixture-model");
        let url = format!("{base}/chat/completions");
        // A whole reply, through the same SSE reader runs use.
        let mut stream = Stream::default();
        let stop = AtomicBool::new(false);
        let status = chat
            .stream(&url, None, body("Hello"), &stop, &mut |bytes| {
                stream.push(bytes).map(|_| ())
            })
            .unwrap();
        assert_eq!(status, 200);
        assert_eq!(stream.finish().unwrap().content, "Saw: Hello");
        // A slow reply arrives a fragment at a time; Stop ends it at once.
        let started = Instant::now();
        let mut first = None;
        let stop = AtomicBool::new(false);
        let result = chat.stream(&url, None, body("slow"), &stop, &mut |bytes| {
            if String::from_utf8_lossy(bytes).contains("token0") {
                first.get_or_insert(started.elapsed());
                stop.store(true, Ordering::Release);
            }
            Ok(())
        });
        assert_eq!(result.unwrap_err(), "Stopped.");
        assert!(first.unwrap() < Duration::from_secs(5), "{first:?}");
        assert!(started.elapsed() < Duration::from_secs(10));
        // A refused address reports its status and feeds nothing to the reader.
        let mut fed = false;
        let status = chat
            .stream(
                &format!("{base}/missing"),
                None,
                body("x"),
                &AtomicBool::new(false),
                &mut |_| {
                    fed = true;
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(status, 404);
        assert!(!fed);
    }
}
