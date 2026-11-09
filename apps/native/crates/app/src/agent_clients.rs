//! Settings › Accounts › Other Agent Apps (A02/S070): add Powermove to an
//! agent app on this Mac so it can drive Powermove's tools on its own plan,
//! or take it out again; Copy Setup for any other MCP app. Registration is
//! `pm-mcp`'s (`registration.rs`): reviewed, never at launch, never replacing
//! an entry set up by hand. Adding to Claude asks first with a native alert,
//! and the answer is recorded once in the profile (`client_consent`).
//! Development builds may name a scratch home (`--dev-agent-home`) so a
//! review never edits the owner's real agent apps.
use gpui::{App, AsyncApp, Global};
use objc2::rc::Retained;
use objc2_app_kit::NSWindow;
use pm_mcp::registration::{Action, Consent, Registration, RegistrationConsent, Review};
use pm_services::surfaces::client_consent::{Answer, ConsentStore, RecordWork, Recorded};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

/// The home whose agent apps are registered: the person's, or a development
/// build's scratch folder.
fn home() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(dev) = std::env::var_os("PM_DEV_AGENT_HOME") {
        return Some(PathBuf::from(dev));
    }
    std::env::var_os("HOME").map(PathBuf::from)
}
/// `pm-mcp` registration for this app and profile.
pub fn registration(
    profile: &Path,
    home: &Path,
    applications: &Path,
    executable: PathBuf,
) -> Result<Registration, String> {
    let mut registration = Registration::new(
        home.to_owned(),
        profile.to_owned(),
        executable,
        applications.to_owned(),
    )
    .map_err(|e| e.to_string())?;
    // Claude Code edits its own settings through its CLI, run with this home.
    let environment = pm_agents::launch::Environment::current(profile.to_owned(), vec![], vec![]);
    if let Some(claude) = environment.binary(pm_agents::runtime::Runtime::Claude) {
        let vars = BTreeMap::from([("HOME".to_owned(), home.display().to_string())]);
        registration.claude_cli = Some(Arc::new(pm_mcp::registration_cli::NativeClaudeCli::new(
            claude,
            home.to_owned(),
            vars,
        )));
    }
    Ok(registration)
}
fn system(profile: &Path) -> Result<Registration, String> {
    let home = home().ok_or("Powermove’s home folder is unknown.")?;
    let applications = if std::env::var_os("PM_DEV_AGENT_HOME").is_some() && cfg!(debug_assertions)
    {
        home.join("Applications")
    } else {
        PathBuf::from("/Applications")
    };
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    registration(profile, &home, &applications, executable)
}
/// The person approved this exact review in the native alert, on main.
struct Approved;
impl RegistrationConsent for Approved {
    fn ask(&self, _: &Review) -> pm_mcp::Result<Consent> {
        Ok(Consent::Approved)
    }
}
/// The once-only Claude answer, in `<profile>/agent-clients.json`.
pub struct ProfileConsent(pub PathBuf);
impl ConsentStore for ProfileConsent {
    fn record_once(&mut self, key: &str, answer: Answer) -> pm_services::Result<Recorded> {
        let path = self.0.join("agent-clients.json");
        let mut stored: Value = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| json!({}));
        if let Ok(previous) = serde_json::from_value::<Answer>(stored[key].clone()) {
            return Ok(Recorded {
                answer: previous,
                fresh: false,
            });
        }
        stored[key] = serde_json::to_value(answer).unwrap_or_default();
        std::fs::write(&path, stored.to_string())?;
        Ok(Recorded {
            answer,
            fresh: true,
        })
    }
}
/// Adds or removes Powermove in one app; the sentence for its row.
pub fn change(registration: &Registration, id: &str, add: bool) -> Result<String, String> {
    let action = if add {
        Action::Connect
    } else {
        Action::Disconnect
    };
    let plan = registration
        .prepare(id, action)
        .map_err(|e| e.to_string())?;
    let applied = registration
        .apply(plan, Some(&Approved))
        .map_err(|e| e.to_string())?;
    Ok(if add { applied.next } else { "Removed.".into() })
}

struct Clients {
    profile: PathBuf,
    info: Value,
    notes: BTreeMap<String, String>,
    busy: Option<String>,
    revision: u64,
}
impl Global for Clients {}
pub fn init(profile: PathBuf, cx: &mut App) {
    if cx.has_global::<Clients>() {
        return;
    }
    cx.set_global(Clients {
        profile,
        info: Value::Null,
        notes: BTreeMap::new(),
        busy: None,
        revision: 1,
    });
    refresh(cx);
}
/// Which agent apps are on this Mac and whether Powermove is in each (off main).
pub fn refresh(cx: &mut App) {
    let profile = cx.global::<Clients>().profile.clone();
    let task = cx
        .background_executor()
        .spawn(async move { system(&profile).map(|r| r.inspect()) });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let info = task.await;
        let _ = cx.update(|cx| {
            let clients = cx.global_mut::<Clients>();
            match info {
                Ok(info) => clients.info = info,
                Err(error) => {
                    clients.info = Value::Null;
                    clients.notes.insert("other".into(), error);
                }
            }
            changed(cx);
        });
    })
    .detach();
}
fn changed(cx: &mut App) {
    cx.global_mut::<Clients>().revision += 1;
    // Deferred: a command arrives inside its editor's update, and every
    // editor (that one too) republishes Settings.
    cx.defer(|cx| {
        for editor in crate::entry::editors(cx) {
            editor.update(cx, |c, cx| c.update_builtins(cx));
        }
    });
}
/// Settings › Accounts reads this through the built-in facts.
pub fn fact(cx: &App) -> Value {
    let Some(c) = cx.try_global::<Clients>() else {
        return Value::Null;
    };
    json!({"revision":c.revision,"clients":c.info["clients"],"notes":c.notes,"busy":c.busy})
}
/// A row's Add Powermove / Remove. Adding to Claude asks first.
pub fn request(id: String, add: bool, window: Retained<NSWindow>, cx: &mut App) {
    if !cx.has_global::<Clients>() || cx.global::<Clients>().busy.is_some() {
        return;
    }
    if add && id == "claude-desktop" {
        use crate::alert::{Alert, Button};
        let rx = Alert::new(
            "Add Powermove to Claude?",
            "Claude can then use Powermove’s tools on your Claude plan. Powermove changes only its own entry in Claude’s settings.",
            vec![Button::new("Add Powermove", true).default(), Button::new("Cancel", false).cancel()],
        )
        .sheet(&window);
        cx.spawn(async move |cx: &mut AsyncApp| {
            let accepted = rx.recv().await == Ok(Some(true));
            let _ = cx.update(|cx| {
                // The answer is recorded once (S070), before any write.
                let profile = cx.global::<Clients>().profile.clone();
                let answer = if accepted {
                    Answer::Added
                } else {
                    Answer::Declined
                };
                let _ = RecordWork {
                    ticket: pm_services::surfaces::Flight::default().start(),
                    answer,
                }
                .run(&mut ProfileConsent(profile));
                if accepted {
                    run(id, true, cx);
                }
            });
        })
        .detach();
        return;
    }
    run(id, add, cx);
}
fn run(id: String, add: bool, cx: &mut App) {
    let clients = cx.global_mut::<Clients>();
    clients.busy = Some(id.clone());
    let profile = clients.profile.clone();
    changed(cx);
    let task = cx.background_executor().spawn({
        let id = id.clone();
        async move { system(&profile).and_then(|r| change(&r, &id, add)) }
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let note = task.await.unwrap_or_else(|e| e);
        let _ = cx.update(|cx| {
            let clients = cx.global_mut::<Clients>();
            clients.busy = None;
            clients.notes.insert(id, note);
            refresh(cx);
        });
    })
    .detach();
}
/// Any Other Agent › Copy Setup: the standard MCP entry most apps accept.
pub fn copy_setup(cx: &mut App) {
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let text = serde_json::to_string_pretty(&json!({"mcpServers":{"powermove":{
        "command": executable, "args": [pm_services::surfaces::agent_clients::MCP_FLAG]}}}))
    .unwrap_or_default();
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    use objc2_foundation::NSString;
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    board.setString_forType(&NSString::from_str(&text), unsafe {
        NSPasteboardTypeString
    });
    if cx.has_global::<Clients>() {
        cx.global_mut::<Clients>()
            .notes
            .insert("other".into(), "Copied.".into());
        changed(cx);
    }
}
