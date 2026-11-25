//! Settings › Providers, the app's part: the Keychain and the system HTTP
//! stack for `pm-agents` (Codex's shared OpenAI item, the Anthropic key,
//! Powermove's model requests), and the native sheet that takes an OpenAI
//! or Anthropic API key in a secure field. The key goes from the field to
//! the Keychain (Anthropic) or to Codex, which saves it in the shared item
//! (OpenAI); it is never published, logged or written to the profile.
use crate::cloud::Items as _;
use crate::sheets::{OpenSheet, from_surface};
use gpui::{App, AsyncApp, Global};
use objc2::rc::Retained;
use objc2_app_kit::NSWindow;
use pm_agents::edge::{Chunk, Net, NetRequest, Secrets};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Generic passwords in the login Keychain, by exact service and account.
#[derive(Debug)]
pub struct Keychain;
impl Secrets for Keychain {
    fn read(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
        crate::cloud::keychain::Mac.read(service, account)
    }
    fn write(&self, service: &str, account: &str, bytes: &[u8]) -> Result<(), String> {
        crate::cloud::keychain::Mac.write(service, account, bytes)
    }
    fn delete(&self, service: &str, account: &str) -> Result<(), String> {
        crate::cloud::keychain::Mac.delete(service, account)
    }
}

/// The system HTTP stack, each request held to its own origin (a redirect
/// elsewhere is never followed with its headers).
#[derive(Debug)]
pub struct SystemNet;
impl Net for SystemNet {
    fn stream(
        &self,
        request: &NetRequest<'_>,
        stop: &AtomicBool,
        on: &mut dyn FnMut(Chunk<'_>) -> Result<(), String>,
    ) -> Result<(), String> {
        use pm_compat_runner::net::{self, Reach, Streamed};
        use std::sync::mpsc;
        let origin = url::Url::parse(request.url)
            .map(|u| u.origin().ascii_serialization())
            .map_err(|e| e.to_string())?;
        let (tx, chunks) = mpsc::channel();
        let mut pending = net::stream(
            pm_runtime_v8::FetchRequest {
                url: request.url.into(),
                method: request.method.into(),
                headers: request.headers.to_vec(),
                body: request.body.to_vec(),
                max_response_bytes: 64 << 20,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(3600),
            },
            Reach::Origin(crate::providers::origin_static(origin)),
            tx,
        )
        .map_err(|e| e.to_string())?;
        let mut deliver = |chunk: Streamed| match chunk {
            Streamed::Status(code) => on(Chunk::Status(code)),
            Streamed::Data(bytes) => on(Chunk::Data(&bytes)),
        };
        loop {
            if stop.load(Ordering::Acquire) {
                pending.cancel();
                return Err("Stopped.".into());
            }
            match chunks.recv_timeout(std::time::Duration::from_millis(20)) {
                Ok(chunk) => {
                    if let Err(error) = deliver(chunk) {
                        pending.cancel();
                        return Err(error);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Some(done) = pending.poll() {
                        while let Ok(chunk) = chunks.try_recv() {
                            deliver(chunk)?;
                        }
                        return done.map(|_| ()).map_err(|e| e.to_string());
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return pending
                        .poll()
                        .unwrap_or_else(|| Err(pm_runtime_v8::Error::Host("ended".into())))
                        .map(|_| ())
                        .map_err(|e| e.to_string());
                }
            }
        }
    }
}

/// The hosts `pm-agents` gets: the Keychain, the network (provider origins
/// answered by local fakes in a development build that names them), this
/// build's Keychain prefix, and the browser hop.
pub fn attach(environment: &mut pm_agents::launch::Environment) -> Arc<pm_agents::account::Opener> {
    environment.secrets = Some(Arc::new(Keychain));
    environment.net = Some(pm_agents::edge::Reroute::from_env(&environment.vars, Arc::new(SystemNet)));
    environment.keychain_prefix =
        crate::platform::bundle_id().unwrap_or_else(|| "com.powermove.native.dev".into());
    let fake_browser = cfg!(debug_assertions)
        && environment.vars.get("PM_AGENT_FAKE_BROWSER").is_some_and(|v| v == "1");
    if fake_browser {
        // A development build against local fakes: the sign-in page is a
        // loopback fake that approves at once, so follow it here rather than
        // open the person's browser.
        Arc::new(|url: &str| {
            let url = url.to_owned();
            std::thread::spawn(move || follow_loopback(&url));
            Ok(())
        })
    } else {
        Arc::new(pm_agents::account::open_in_browser)
    }
}
fn follow_loopback(url: &str) {
    use pm_agents::edge::{Loopback, fetch};
    let mut next = url.to_owned();
    for _ in 0..4 {
        let mut location = None;
        let request = NetRequest { method: "GET", url: &next, headers: &[], body: b"" };
        let (status, body) = match fetch(&Loopback, &request) {
            Ok(found) => found,
            Err(_) => return,
        };
        // The fake answers a redirect's target in its body's first line.
        if (300..400).contains(&status) {
            location = String::from_utf8(body).ok().and_then(|b| b.lines().next().map(str::to_owned));
        }
        match location {
            Some(target) if target.starts_with("http://127.0.0.1:") => next = target,
            _ => return,
        }
    }
}

// ---- The API key sheet ----

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    OpenAi,
    Anthropic,
}
#[derive(Default)]
struct KeySheet(Option<OpenSheet>);
impl Global for KeySheet {}

/// Opens "Add an API Key" over Settings; Save hands the key to `accounts`.
pub fn open(kind: Kind, window: Retained<NSWindow>, accounts: Arc<pm_agents::account::Accounts>, cx: &mut App) {
    use crate::dialogs::{Button, DialogId, Field, NativePiece, Surface, WindowId};
    if cx.try_global::<KeySheet>().is_some_and(|s| s.0.is_some()) {
        return;
    }
    let (title, line) = match kind {
        Kind::OpenAi => ("Add an OpenAI API Key", "Codex saves it in your Keychain. Used by Codex and Powermove."),
        Kind::Anthropic => ("Add an Anthropic API Key", "Saved in your Keychain. Used by Claude Code and Powermove."),
    };
    let mut key = Field::new("key", "API Key", NativePiece::TextField, vec![String::new()]);
    key.secure = true;
    key.placeholder = Some(if kind == Kind::OpenAi { "sk-…" } else { "sk-ant-…" });
    let mut surface = Surface::sheet(DialogId("provider-key".into()), WindowId("settings".into()), title, vec![key], "Save");
    surface.line = Some(line.into());
    surface.buttons = vec![Button::cancel(false), Button::action("Save")];
    let tokens = pm_shell::tokens();
    let open = OpenSheet::present(from_surface(&surface, &tokens), &window, Some("key"));
    let actions = open.actions.clone();
    cx.set_global(KeySheet(Some(open)));
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Ok(action) = actions.recv().await {
            let done = cx.update(|cx| {
                let sheet = cx.global_mut::<KeySheet>();
                match action.as_str() {
                    "apply" => {
                        let Some(open) = sheet.0.take() else { return true };
                        let key = open.text("key").trim().to_owned();
                        open.close();
                        let accounts = accounts.clone();
                        // The Keychain and Codex are worker work, never the main thread's.
                        std::thread::spawn(move || match kind {
                            Kind::OpenAi => accounts.connect_openai_key(key),
                            Kind::Anthropic => {
                                let _ = accounts.set_anthropic_key(key);
                            }
                        });
                        true
                    }
                    "cancel" => {
                        if let Some(open) = sheet.0.take() {
                            open.close();
                        }
                        true
                    }
                    _ => false,
                }
            });
            if done {
                break;
            }
        }
    })
    .detach();
}
