//! Profile/project services used by the built-in Settings realm. Actor receipts
//! remain the only success boundary; unsupported native services refuse visibly.
use crate::{
    data::{Backend, wire},
    models::NativeCommand,
};
use pm_agents::{Provider, account::State};
use pm_kernel::{
    Call, Request, WriteBody,
    commands::comps::{CompCommand, CompPatch, EXPORT_RATE_CHOSEN},
};
use serde_json::{Value, json};

/// The shipped composition size presets (v1.2.0 `RESOLUTION_PRESETS`).
/// Settings ▸ Project ▸ Export defaults: (row id, field, label, detail).
const EXPORT_ROWS: [(&str, &str, &str, &str); 8] = [
    (
        "format",
        "format",
        "Format",
        "Container and codec of the exported file.",
    ),
    (
        "resolution",
        "scale",
        "Resolution",
        "Rendered size relative to the composition.",
    ),
    (
        "export-fps",
        "fps",
        "Frame Rate",
        "Frames written per second of exported video.",
    ),
    (
        "range",
        "range",
        "Range",
        "Export the work area or the whole composition.",
    ),
    (
        "quality",
        "quality",
        "Quality",
        "Video bitrate used for video exports.",
    ),
    (
        "motion-blur",
        "mblur",
        "Motion Blur",
        "Render motion blur for moving layers.",
    ),
    (
        "audio",
        "audio",
        "Include Audio",
        "Mix composition audio into the video.",
    ),
    (
        "alpha",
        "alpha",
        "Transparent Background",
        "Keep the background see-through, with straight alpha.",
    ),
];
/// A usable work area (`[start, end]`, end after start), as main's planner asks.
pub(crate) fn work_span(work: &Value) -> Option<(f64, f64)> {
    let (a, b) = (work.get(0)?.as_f64()?, work.get(1)?.as_f64()?);
    (b > a).then_some((a, b))
}
/// v1 `EXPORT_FRAME_RATES`.
const EXPORT_FRAME_RATES: [f64; 8] = [
    24000. / 1001.,
    24.,
    25.,
    30000. / 1001.,
    30.,
    50.,
    60000. / 1001.,
    60.,
];
/// The shipped composition sizes (v1.2.0 RESOLUTION_PRESETS) as (label,
/// width, height), in menu order. Every size picker offers these, then Custom.
pub const RESOLUTIONS: [(&str, u32, u32); 6] = [
    ("4K UHD · 3840×2160", 3840, 2160),
    ("HD 1080p · 1920×1080", 1920, 1080),
    ("HD 720p · 1280×720", 1280, 720),
    ("Vertical HD · 1080×1920", 1080, 1920),
    ("Portrait 4:5 · 1080×1350", 1080, 1350),
    ("Square · 1080×1080", 1080, 1080),
];

impl Backend {
    /// Live subscription rows. These replace the catalogue's disabled ones by id.
    pub(crate) fn account_rows(&mut self) -> Vec<Value> {
        let mut rows = cloud_rows(&self.native.facts["cloud"]);
        // A switch acts through its native binding lease.
        if let Some(remember) = rows.iter_mut().find(|r| r["id"] == "account.remember")
            && let Ok(fields) = self.fields(&json!(["account.remember"]))
        {
            remember["binding"] = fields["account.remember"].clone();
        }
        let powermove = self
            .native
            .agent_link
            .environment()
            .is_some_and(|env| env.aero.is_some());
        if powermove {
            // Settings › Providers replaces the agent account rows.
            rows.extend(self.providers_rows());
            rows.push(notification_row());
        } else {
            rows.extend(self.subscription_rows());
            rows.extend(provider_rows(&self.native.facts["provider"]));
        }
        rows.extend(agent_client_rows(&self.native.facts["agentClients"]));
        rows
    }
    /// Settings › Providers: three providers, each for its harness (the
    /// Powermove Account for Aero, OpenAI for Codex, Anthropic for Claude
    /// Code), each row the method, signed in or out with its name (keys
    /// masked), and the action.
    fn providers_rows(&self) -> Vec<Value> {
        let row = |id: &str, group: &str, label: &str| {
            json!({"id":format!("providers.{id}"),"section":"providers","group":group,"label":label,"control":"status"})
        };
        let action = |mut row: Value, action: &str, label: &str, detail: String| {
            row["control"] = json!("action");
            row["action"] = json!(action);
            row["actionLabel"] = json!(label);
            row["detail"] = json!(detail);
            row
        };
        let mut rows = self.powermove_provider_rows(&row, &action);
        let accounts = self.native.agent_link.accounts();
        // OpenAI: Sign in with ChatGPT (Codex's sign-in) or an API key, in
        // Codex's own Keychain item.
        const OPENAI: &str = "OpenAI (Codex)";
        let openai = accounts.as_ref().map(|a| a.status(Provider::Chatgpt));
        let both = "Used by Codex.";
        match openai.as_ref().map(|s| (s.state, s)) {
            // Both ways in, as Anthropic (Claude Code) has them: Codex holds
            // one sign-in, so the other row says it would replace it.
            Some((State::Connected, status)) => {
                let with_detail = |mut r: Value| {
                    if let Some(detail) = &status.detail {
                        r["detail"] = json!(format!(
                            "{} {detail}",
                            r["detail"].as_str().unwrap_or_default()
                        ));
                    }
                    r
                };
                match &status.key {
                    Some(masked) => {
                        rows.push(action(
                            row("openai", OPENAI, "Sign in with ChatGPT"),
                            "providers.openai.sign-in",
                            "Sign In…",
                            format!("Signed out. Signing in replaces the API key. {both}"),
                        ));
                        let mut r = action(
                            row("openai-key", OPENAI, "API key"),
                            "providers.openai.sign-out",
                            "Remove",
                            format!("{masked}. {both}"),
                        );
                        r["target"] = json!(masked);
                        rows.push(with_detail(r));
                    }
                    None => {
                        let name = [status.email.clone(), status.plan.clone()]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(" · ");
                        let detail = if name.is_empty() {
                            format!("Signed in. {both}")
                        } else {
                            format!("Signed in as {name}. {both}")
                        };
                        let mut r = action(
                            row("openai", OPENAI, "Sign in with ChatGPT"),
                            "providers.openai.sign-out",
                            "Sign Out",
                            detail,
                        );
                        r["target"] = json!(name);
                        rows.push(with_detail(r));
                        rows.push(action(
                            row("openai-key", OPENAI, "API key"),
                            "providers.openai.key",
                            "Add Key…",
                            format!("Not added. Adding one replaces the ChatGPT sign-in. {both}"),
                        ));
                    }
                }
            }
            Some((State::Connecting, status)) => rows.push(action(
                row("openai", OPENAI, "Sign in with ChatGPT"),
                "providers.openai.sign-out",
                "Cancel",
                status.detail.clone().unwrap_or_else(|| "Finish signing in in your browser.".into()),
            )),
            Some((State::Checking, _)) => {
                let mut r = row("openai", OPENAI, "OpenAI");
                r["value"] = json!("Checking…");
                rows.push(r);
            }
            _ => {
                let detail = openai.and_then(|s| s.detail).map_or_else(
                    || format!("Signed out. {both}"),
                    |d| format!("Signed out. {d}"),
                );
                rows.push(action(
                    row("openai", OPENAI, "Sign in with ChatGPT"),
                    "providers.openai.sign-in",
                    "Sign In…",
                    detail,
                ));
                rows.push(action(
                    row("openai-key", OPENAI, "API key"),
                    "providers.openai.key",
                    "Add Key…",
                    format!("Not added. {both}"),
                ));
            }
        }
        // Anthropic: Claude Code's own sign-in, or an API key it runs on
        // while signed out.
        const ANTHROPIC: &str = "Anthropic (Claude Code)";
        if let Some(accounts) = &accounts {
            let claude = accounts.status(Provider::Claude);
            let terms = "Used by Claude Code.";
            let who = [claude.email.clone(), claude.plan.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
            let r = match claude.state {
                State::Connected => {
                    let mut r = action(
                        row("claude", ANTHROPIC, "Sign in with Claude"),
                        "providers.claude.sign-out",
                        "Sign Out",
                        format!("Signed in as {who}. {terms}"),
                    );
                    r["target"] = json!(who);
                    r
                }
                State::Connecting => action(
                    row("claude", ANTHROPIC, "Sign in with Claude"),
                    "providers.claude.sign-out",
                    "Cancel",
                    claude.detail.clone().unwrap_or_default(),
                ),
                State::Disconnected => action(
                    row("claude", ANTHROPIC, "Sign in with Claude"),
                    "providers.claude.sign-in",
                    "Sign In…",
                    format!("Signed out. {terms}"),
                ),
                State::Checking => {
                    let mut r = row("claude", ANTHROPIC, "Sign in with Claude");
                    r["value"] = json!("Checking…");
                    r["detail"] = json!(terms);
                    r
                }
                // What went wrong reads under the title, and signing in
                // again is at hand.
                State::Unavailable => action(
                    row("claude", ANTHROPIC, "Sign in with Claude"),
                    "providers.claude.sign-in",
                    "Sign In…",
                    format!("{} {terms}", claude.detail.clone().unwrap_or_default()).trim().to_owned(),
                ),
            };
            rows.push(r);
            let key = accounts.anthropic();
            let used = "Used by Claude Code while signed out of Claude.";
            rows.push(match &key.masked {
                Some(masked) => {
                    let detail = match (&key.detail, key.checking) {
                        (_, true) => format!("{masked} · checking its models. {used}"),
                        (Some(problem), _) => format!("{masked} · {problem}"),
                        _ => format!(
                            "{masked} · {} {}. {used}",
                            key.models.len(),
                            if key.models.len() == 1 { "model" } else { "models" }
                        ),
                    };
                    let mut r = action(
                        row("anthropic-key", ANTHROPIC, "API key"),
                        "providers.anthropic.remove",
                        "Remove",
                        detail,
                    );
                    r["target"] = json!(masked);
                    r
                }
                None => action(
                    row("anthropic-key", ANTHROPIC, "API key"),
                    "providers.anthropic.key",
                    "Add Key…",
                    format!("Not added. {used}"),
                ),
            });
        }
        rows
    }
    /// The Powermove Account, which Aero runs on (the same account as
    /// Settings › Account): signed in or out, and where Aero stands on it.
    fn powermove_provider_rows(
        &self,
        row: &dyn Fn(&str, &str, &str) -> Value,
        action: &dyn Fn(Value, &str, &str, String) -> Value,
    ) -> Vec<Value> {
        use pm_agents::inference::Standing;
        const GROUP: &str = "Powermove Account (Aero)";
        let cloud = &self.native.facts["cloud"];
        if !cloud.is_object() {
            return vec![];
        }
        let standing = self.native.agent_link.accounts().map(|a| a.aero());
        let aero = match &standing {
            Some(Standing::Ready { billing, .. }) => {
                let plan = billing.plan.as_ref().map_or("", |p| p.name.as_str());
                format!("Aero runs on {plan}: {}.", billing.credits_left())
            }
            Some(Standing::Free(_)) => pm_agents::inference::NEEDS_PLAN.into(),
            Some(Standing::Unreachable(why)) => why.clone(),
            _ => "Aero runs on Powermove inference.".into(),
        };
        // Signed in here, but Powermove refused the session (it ended or
        // was revoked): the way back is Sign In, never only Sign Out.
        let rejected = cloud["signedIn"] == true && matches!(standing, Some(Standing::SignedOut));
        let mut r = if rejected {
            let identity = cloud["identity"].as_str().unwrap_or_default();
            let mut r = action(
                row("powermove", GROUP, "Powermove Account"),
                "account.sign-in",
                "Sign In…",
                format!("Your session as {identity} ended. Sign in again to use Aero."),
            );
            r["target"] = json!(identity);
            r
        } else if cloud["signedIn"] == true {
            let identity = cloud["identity"].as_str().unwrap_or_default();
            let mut r = action(
                row("powermove", GROUP, "Powermove Account"),
                "account.sign-out",
                "Sign Out",
                format!("Signed in as {identity}. {aero}"),
            );
            r["target"] = json!(identity);
            r
        } else {
            action(
                row("powermove", GROUP, "Powermove Account"),
                "account.sign-in",
                "Sign In…",
                "Signed out. Aero runs on Powermove inference.".into(),
            )
        };
        r["disabled"] = json!(cloud["busy"] == true);
        let mut rows = vec![r];
        // On Free: the plans are on Settings › Account.
        if cloud["signedIn"] == true && matches!(standing, Some(Standing::Free(_))) {
            rows.push(action(
                row("aero-plan", GROUP, "Aero"),
                "settings.show-account",
                "Choose a Plan…",
                "Aerolite or Aero, on your Powermove account.".into(),
            ));
        }
        rows
    }
    /// Settings › General › Timeline: hover-to-scrub (View ▸ Skimming), this
    /// Mac's choice, shown with or without a project. On until turned off.
    pub(crate) fn timeline_rows(&mut self) -> Vec<Value> {
        let on = self.native.facts["skimming"].as_bool().unwrap_or(true);
        let mut row = json!({"id":"timeline.skimming","section":"general","group":"Playback",
            "label":"Hover to Scrub","control":"switch","value":on,
            "detail":"Show the frame under the pointer as it moves over the timeline. The playhead stays put until you click."});
        if let Ok(fields) = self.fields(&json!(["timeline.skimming"])) {
            row["binding"] = fields["timeline.skimming"].clone();
        }
        vec![row]
    }
    fn subscription_rows(&self) -> Vec<Value> {
        let Some(accounts) = self.native.agent_link.accounts() else {
            return vec![notification_row()];
        };
        [(Provider::Chatgpt, "ChatGPT"), (Provider::Claude, "Claude")]
            .into_iter()
            .map(|(provider, name)| {
                let status = accounts.status(provider);
                let target = status.email.clone();
                let who = [status.email, status.plan]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · ");
                let id = format!("account.{}", name.to_lowercase());
                // The signed-in account (none while signing in) a press acts on.
                let mut row = json!({"id":id,"section":"agents","group":"Subscriptions",
                    "label":name,"target":target});
                let (action, label) = match status.state {
                    State::Connected => (Some("account.disconnect"), "Sign Out"),
                    State::Connecting => (Some("account.disconnect"), "Cancel"),
                    State::Disconnected => (Some("account.connect"), "Sign In…"),
                    State::Checking | State::Unavailable => (None, ""),
                };
                match action {
                    Some(action) => {
                        row["control"] = json!("action");
                        row["action"] = json!(action);
                        row["actionLabel"] = json!(label);
                        if let Some(detail) = status.detail {
                            row["detail"] = json!(detail);
                        } else if !who.is_empty() {
                            row["detail"] = json!(who);
                        }
                    }
                    None => {
                        row["control"] = json!("status");
                        let checking = status.state == State::Checking;
                        let value = status.detail.filter(|_| !checking);
                        row["value"] = json!(value.as_deref().unwrap_or("Checking…"));
                    }
                }
                row
            })
            .chain([notification_row()])
            .collect()
    }
    /// Settings › Media: proxy preferences and storage, from the host's
    /// `mediaSettings` fact (this Mac's, so shown without a project).
    pub(crate) fn media_rows(&mut self) -> Result<Vec<Value>, String> {
        let media = self.native.facts["mediaSettings"].clone();
        if media.is_null() {
            return Ok(vec![]);
        }
        let choices = media["rows"].as_array().cloned().unwrap_or_default();
        // A choice shows its control only with a lease; its change goes to
        // the panel's listener, which writes `media.*` through the host.
        let leases = self.fields(&Value::Array(
            choices.iter().map(|r| r["id"].clone()).collect(),
        ))?;
        // A row may name its own place and control (Settings › General ›
        // Playback's background rendering lives with these settings).
        let mut rows: Vec<Value> = choices
            .iter()
            .map(|r| {
                let or =
                    |key: &str, default: &str| r[key].as_str().map_or(json!(default), |v| json!(v));
                let group = match r["id"].as_str() {
                    Some("media.importToTimeline" | "media.stillDuration") => "Import",
                    Some("media.renderLimit") => "Storage",
                    Some("media.cacheLimit") => "Powermove",
                    _ => "Proxies",
                };
                json!({"id":r["id"],"section":or("section", if r["id"] == "media.cacheLimit" { "storage" } else { "media" }),"group":or("group",group),
                    "label":r["label"],"control":or("control", if r["value"].is_boolean() { "switch" } else { "choice" }),"value":r["value"],
                    "options":r["options"],"detail":r["detail"],
                    "binding":leases[r["id"].as_str().unwrap_or_default()],"disabled":false})
            })
            .collect();
        let used = &media["used"];
        let space = if used.is_null() {
            "Calculating storage…".to_owned()
        } else {
            format!(
                "Proxies: {} · Optimized Media: {} · Renders: {}",
                used["proxies"].as_str().unwrap_or(""),
                used["optimized"].as_str().unwrap_or(""),
                used["renders"].as_str().unwrap_or("")
            )
        };
        rows.extend([
            json!({"id":"media.location","section":"media","group":"Storage","label":"Location",
                "control":"action","action":"media.location","actionLabel":"Change…",
                "detail":media["location"]}),
            json!({"id":"media.used","section":"media","group":"Storage","label":"Space Used",
                "control":"status","detail":space}),
            json!({"id":"media.clear","section":"media","group":"Storage",
                "label":"Generated Media","control":"action","action":"media.clear",
                "actionLabel":"Clear…",
                "detail":"Removes proxies, optimized media and renders the open project doesn’t use. Originals, attached proxies and this project’s media stay."}),
            json!({"id":"media.reveal","section":"media","group":"Storage",
                "label":"Storage Folder","control":"action","action":"media.reveal",
                "actionLabel":"Show in Finder"}),
        ]);
        rows.extend(protected_folders_row(&media["folders"]));
        Ok(rows)
    }
    /// Settings › Storage, from the host's `storageSettings` fact (this Mac's
    /// folders, so shown with or without a project).
    pub(crate) fn storage_rows(&self) -> Vec<Value> {
        self.native.facts["storageSettings"]["rows"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
    pub(crate) fn settings_rows(&mut self) -> Result<Vec<Value>, String> {
        let media = self.media_rows()?;
        let Some(p) = self.published() else {
            return Ok(media);
        };
        let root = p.snapshot.root().ok_or("Project closed")?;
        let comp = pm_kernel::model::comp_node(root, p.snapshot.open_comp().as_deref())
            .ok_or("Composition closed")?;
        let fields = self.fields(&json!([
            "project.size",
            "project.width",
            "project.height",
            "project.name",
            "project.fps",
            "project.duration",
            "project.background",
            "project.colorDepth",
            "project.output",
            "project.masteringPeak"
        ]))?;
        let mut rows = vec![];
        for (id, group, label, control, value, detail) in [
            (
                "project.name",
                "Project",
                "Name",
                "text",
                root.get("name")
                    .map(|n| wire(&n.to_value()))
                    .unwrap_or(json!("")),
                "Shown in the title bar and used for exported file names.",
            ),
            (
                "project.fps",
                "Composition",
                "Frame Rate",
                "number",
                comp.get("fps")
                    .map(|n| wire(&n.to_value()))
                    .unwrap_or(json!(30)),
                "Frames rendered per second of the composition — any rate from 1 to 240.",
            ),
            (
                "project.duration",
                "Composition",
                "Duration",
                "number",
                comp.get("dur")
                    .map(|n| wire(&n.to_value()))
                    .unwrap_or(json!(0.1)),
                "",
            ),
            (
                "project.background",
                "Composition",
                "Background",
                "colour",
                // No `bg` renders black (the engine's `normalize_fill`).
                comp.get("bg")
                    .map(|n| wire(&n.to_value()))
                    .unwrap_or(json!("#000000")),
                "Colour shown behind every layer of the composition.",
            ),
        ] {
            let mut row = json!({"id":id,"section":"project","group":group,"label":label,"control":control,"value":value,"detail":detail,"binding":fields[id],"disabled":false});
            if id == "project.fps" {
                row["min"] = json!(1);
                row["max"] = json!(240);
                row["unit"] = json!("fps");
            }
            if id == "project.duration" {
                row["min"] = json!(0.1);
                row["unit"] = json!("s");
                // As every duration field shows it: two places, none trailing.
                row["precision"] = json!(2);
            }
            rows.push(row);
        }
        // The shipped presets (v1.2.0 RESOLUTION_PRESETS), then Custom with the size.
        let size = |k: &str| {
            comp.get(k)
                .and_then(|n| n.to_value().as_f64())
                .unwrap_or(0.)
                .round()
        };
        let (w, h) = (size("w"), size("h"));
        let preset = RESOLUTIONS
            .iter()
            .find(|(_, pw, ph)| f64::from(*pw) == w && f64::from(*ph) == h)
            .map(|(_, pw, ph)| format!("{pw}x{ph}"));
        let mut options: Vec<Value> = RESOLUTIONS
            .iter()
            .map(|(label, pw, ph)| json!({"v":format!("{pw}x{ph}"),"label":label}))
            .collect();
        options.push(json!({"v":"custom","label":format!("Custom · {w}×{h}")}));
        rows.insert(
            1,
            json!({"id":"project.size","section":"project","group":"Composition",
            "label":"Size","control":"choice","value":preset.unwrap_or_else(|| "custom".into()),
            "options":options,"detail":"Pick a common size, or set your own pixel dimensions.",
            "binding":fields["project.size"],"disabled":false}),
        );
        for (at, (id, label, value)) in [
            ("project.width", "Width", w),
            ("project.height", "Height", h),
        ]
        .into_iter()
        .enumerate()
        {
            rows.insert(
                2 + at,
                json!({"id":id,"section":"project","group":"Composition",
                "label":label,"control":"number","value":value,"min":16,"max":8192,"unit":"px",
                "binding":fields[id],"disabled":false}),
            );
        }
        rows.push(json!({"id":"project.colorDepth","section":"project","group":"Composition",
            "label":"Color Depth","control":"choice","value":root.get("colorDepth")
                .map(|n| wire(&n.to_value())).unwrap_or(json!(8)),
            "options":[{"v":8,"label":"8-bit"},{"v":16,"label":"16-bit"},{"v":32,"label":"32-bit float"}],
            "detail":"Precision per color channel. Applies to every composition in this project.",
            "binding":fields["project.colorDepth"],"disabled":false}));
        let color = root.get("color");
        let output = pm_kernel::model::project_output(root);
        let hdr = output.hdr();
        rows.push(json!({"id":"project.output","section":"project","group":"Color",
            "label":"Output","control":"choice","value":output.id(),
            "options":pm_protocol::color::ProjectOutput::ALL.iter()
                .map(|o| json!({"v":o.id(),"label":o.label()})).collect::<Vec<_>>(),
            "detail":"What this project delivers. HDR keeps PQ and HLG light in the project; SDR tone maps HDR footage to SDR white as it comes in.",
            "binding":fields["project.output"],"disabled":false}));
        rows.push(
            json!({"id":"project.masteringPeak","section":"project","group":"Color",
            "label":"Mastering Peak","control":"choice","value":color
                .and_then(|c| c.get("masteringPeak"))
                .map(|n| wire(&n.to_value())).unwrap_or(json!(1000)),
            "options":[{"v":600,"label":"600 nits"},{"v":1000,"label":"1000 nits"},
                {"v":2000,"label":"2000 nits"},{"v":4000,"label":"4000 nits"}],
            "detail":"The brightest light HDR delivery keeps. Brighter sources roll off to it.",
            "binding":fields["project.masteringPeak"],"disabled":!hdr}),
        );
        rows.extend(self.export_default_rows(root, comp)?);
        let memory = self.fields(&json!(["memory.enabled"]))?;
        rows.push(json!({"id":"memory.enabled","section":"project","group":"Project Memory",
            "label":"Use Project Memory","control":"switch", "disabled":false,
            "value":root.get("memoryEnabled").map(pm_model::Node::to_value) != Some(pm_model::Value::Bool(false)),
            "detail":"Agents read these notes before they edit, and add to them as they work. Turning it off keeps the notes but hides them from agents.",
            "binding":memory["memory.enabled"]}));
        rows.extend(media);
        Ok(rows)
    }
    /// The project's export defaults (main's `readExportDefaults`).
    pub(crate) fn export_defaults(&self, root: &pm_model::Node, fps: f64) -> Value {
        // As main: the project's own, normalized (a project without any reads
        // the defaults); the last export's options never stand in.
        let stored = root
            .get("exportDefaults")
            .map(|n| wire(&n.to_value()))
            .unwrap_or(Value::Null);
        let mut defaults = pm_promedia::export_panel::normalize_defaults(&stored, fps);
        // The frame rate is the composition's unless the person chose one:
        // a stored rate nobody chose is the one the project was created at
        // (v1 never moved it with the composition's).
        if stored[EXPORT_RATE_CHOSEN].as_bool() == Some(true) {
            defaults[EXPORT_RATE_CHOSEN] = json!(true);
        } else {
            defaults["fps"] =
                pm_promedia::export_panel::normalize_defaults(&Value::Null, fps)["fps"].clone();
        }
        defaults
    }
    fn export_default_rows(
        &mut self,
        root: &pm_model::Node,
        comp: &pm_model::Node,
    ) -> Result<Vec<Value>, String> {
        let fps = comp
            .get("fps")
            .and_then(|n| n.to_value().as_f64())
            .unwrap_or(30.);
        let d = self.export_defaults(root, fps);
        use pm_promedia::export_panel::Format;
        let format = d["format"].as_str().unwrap_or("mp4");
        let exported = serde_json::from_value::<Format>(json!(format)).ok();
        // A v1 format this app can't export (rec, web) reads its size and range.
        let reads = |field: &str| {
            exported.map_or(matches!(field, "scale" | "fps" | "range"), |f| {
                f.reads(field)
            })
        };
        let ids: Vec<String> = EXPORT_ROWS
            .iter()
            .map(|(id, ..)| format!("project.{id}"))
            .collect();
        let fields = self.fields(&json!(ids))?;
        let mut rates: Vec<f64> = EXPORT_FRAME_RATES.to_vec();
        let current = d["fps"].as_f64().unwrap_or(fps);
        if !rates.iter().any(|r| (r - current).abs() < 1e-6) {
            rates.push(current);
            rates.sort_by(f64::total_cmp);
        }
        let rate_label = |r: f64| {
            let shown = (r * 1000.).round() / 1000.;
            format!("{shown} fps")
        };
        Ok(EXPORT_ROWS
            .iter()
            .map(|(id, field, label, detail)| {
                let key = format!("project.{id}");
                let (control, options) = match *field {
                    "format" => {
                        let mut options: Vec<Value> = Format::ALL
                            .iter()
                            .map(|f| json!({"v":f,"label":f.label()}))
                            .collect();
                        // A v1 format this app can't export still shows as stored.
                        if exported.is_none() {
                            let label = match format {
                                "rec" => "WebM · Realtime Capture",
                                _ => "Web Animation · Code + Assets (.zip)",
                            };
                            options.push(json!({"v":format,"label":label}));
                        }
                        ("choice", json!(options))
                    }
                    "scale" => ("choice", json!([{"v":0.5,"label":"Half"},{"v":1,"label":"Full"},{"v":2,"label":"2×"}])),
                    "fps" => ("choice", json!(rates.iter().map(|r| json!({"v":r,"label":rate_label(*r)})).collect::<Vec<_>>())),
                    "range" => ("choice", json!([{"v":"work","label":"Work Area"},{"v":"all","label":"Full Composition"}])),
                    "quality" => ("choice", json!([{"v":"draft","label":"Draft · 4 Mbps"},{"v":"high","label":"High · 16 Mbps"},{"v":"max","label":"Max · 40 Mbps"}])),
                    _ => ("switch", Value::Null),
                };
                let mut row = json!({"id":key,"section":"project","group":"Export defaults",
                    "label":label,"control":control,"value":d[*field],"detail":detail,
                    "binding":fields[&key],"disabled":!reads(field)});
                if !options.is_null() {
                    row["options"] = options;
                }
                row
            })
            .collect())
    }
    /// A row button pressed on an older publication still lands when the
    /// latest one offers that row the same action, enabled, on the same
    /// target (the account or provider the person saw): an unrelated fact (an
    /// account check finishing) must not refuse a click, and a press must
    /// never reach a different account. A destructive press from a
    /// publication no longer remembered is refused. Writes keep the strict
    /// revision check.
    fn row_still_offers(&mut self, seen: Option<u64>, action: &Value) -> Result<bool, String> {
        // The preset is chosen from a fixed list: the choice stands whatever
        // the page showed when it was made (its value is checked after).
        if action["type"] == "write" && action["id"] == "shortcuts.preset" {
            return Ok(!self.native.facts["shortcuts"].is_null());
        }
        // A Shortcuts row acts on a command, not on what the page showed:
        // it still applies while the command is listed and changeable.
        if action["type"] == "shortcut" {
            let latest = self.settings_snapshot()?;
            return Ok(latest["shortcuts"]["rows"].as_array().is_some_and(|rows| {
                rows.iter()
                    .any(|row| row["id"] == action["id"] && row["fixed"] != true)
            }));
        }
        // On or off is the person's whole choice, not an edit of what the
        // page showed: it stands while the extension is listed. (Turning one
        // off moves the page on at once, as its host unloads it; the next
        // switch was refused as stale.)
        if action["type"] == "extension.enabled" {
            let latest = self.settings_snapshot()?;
            return Ok(latest["extensions"]
                .as_array()
                .is_some_and(|all| all.iter().any(|e| e["id"] == action["id"])));
        }
        if action["type"] != "row" {
            return Ok(false);
        }
        let latest = self.settings_snapshot()?;
        let offered = latest["rows"]
            .as_array()
            .and_then(|rows| rows.iter().find(|row| row["id"] == action["id"]))
            .is_some_and(|row| {
                row["action"] == action["action"] && row["disabled"] != true && row["busy"] != true
            });
        let id = action["id"].as_str().unwrap_or_default();
        let target = |revision: u64| {
            self.native
                .settings_targets
                .iter()
                .find(|(r, _)| *r == revision)
                .map(|(_, rows)| rows[id].clone())
        };
        let now = target(self.native.settings_sequence);
        Ok(offered
            && match seen.and_then(target) {
                Some(then) => then["action"] == action["action"] && Some(then) == now,
                None => !destructive(action["action"].as_str().unwrap_or_default()),
            })
    }
    /// Settings › Shortcuts' own rows: the preset, Reset All, Import… and
    /// Export… (the command list is the publication's `shortcuts`).
    pub(crate) fn shortcut_rows(&mut self) -> Result<Vec<Value>, String> {
        let page = self.native.facts["shortcuts"].clone();
        if page.is_null() {
            return Ok(vec![]);
        }
        let fields = self.fields(&json!(["shortcuts.preset"]))?;
        let action = |id: &str, label: &str, button: &str, detail: &str| {
            json!({"id":id,"section":"shortcuts","group":"Keys","label":label,"control":"action",
                "action":id,"actionLabel":button,"detail":detail})
        };
        let mut reset = action(
            "shortcuts.resetAll",
            "Your Changes",
            "Reset All",
            "Go back to the preset's keys for every command.",
        );
        reset["disabled"] = json!(page["changed"] != true);
        Ok(vec![
            json!({"id":"shortcuts.preset","section":"shortcuts","group":"Keys","label":"Keys From",
                "control":"choice","value":page["preset"],"options":page["presets"],
                "binding":fields["shortcuts.preset"],"disabled":false,
                "detail":"Start from the keys of the editor you know; your own changes stay on top."}),
            reset,
            action(
                "shortcuts.import",
                "Import Shortcuts",
                "Import…",
                "Use keys saved on another Mac, or written by an extension or your agent.",
            ),
            action(
                "shortcuts.export",
                "Export Shortcuts",
                "Export…",
                "Save the keys as a file to share or keep.",
            ),
        ])
    }
    pub(crate) fn settings_action(&mut self, args: &Value) -> Result<NativeCommand, String> {
        self.sync()?;
        let action = &args["action"];
        if args["revision"].as_u64() != Some(self.native.settings_sequence)
            && !self.row_still_offers(args["revision"].as_u64(), action)?
        {
            return Err("Settings changed; retry from the latest publication".into());
        }
        let shortcuts = |op: &str, value: &Value| json!({"name": "shortcuts", "args": [{"op": op, "value": value}]});
        match action["type"].as_str() {
            Some("write") if action["id"] == "shortcuts.preset" => {
                let options = &self.native.facts["shortcuts"]["presets"];
                if !options
                    .as_array()
                    .is_some_and(|o| o.iter().any(|o| o["v"] == action["value"]))
                {
                    return Err("Invalid setting value".into());
                }
                self.native
                    .platform
                    .push(("command".into(), shortcuts("preset", &action["value"])));
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            Some("row")
                if matches!(
                    action["action"].as_str(),
                    Some("shortcuts.resetAll" | "shortcuts.import" | "shortcuts.export")
                ) =>
            {
                let op = &action["action"].as_str().unwrap_or_default()["shortcuts.".len()..];
                self.native
                    .platform
                    .push(("command".into(), shortcuts(op, &Value::Null)));
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            Some("write")
                if action["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("media.")) =>
            {
                // The host validates and keeps it; the row shows it at once.
                let id = action["id"].as_str().unwrap_or_default();
                if let Some(rows) = self.native.facts["mediaSettings"]["rows"].as_array_mut()
                    && let Some(row) = rows.iter_mut().find(|r| r["id"] == id)
                {
                    if !row["options"]
                        .as_array()
                        .is_some_and(|o| o.iter().any(|o| o["v"] == action["value"]))
                    {
                        return Err("Invalid setting value".into());
                    }
                    row["value"] = action["value"].clone();
                }
                self.native.platform.push((
                    "media.settings".into(),
                    json!({"write":id,"value":action["value"]}),
                ));
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            Some("row")
                if action["action"]
                    .as_str()
                    .is_some_and(|a| a.starts_with("storage.")) =>
            {
                self.native
                    .platform
                    .push(("storage.settings".into(), json!({"action":action["action"]})));
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            Some("row")
                if matches!(
                    action["action"].as_str(),
                    Some("media.location" | "media.clear" | "media.reveal" | "media.folders")
                ) =>
            {
                self.native
                    .platform
                    .push(("media.settings".into(), json!({"action":action["action"]})));
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            Some("row")
                if matches!(
                    action["action"].as_str(),
                    Some("provider.setup" | "provider.disconnect")
                ) =>
            {
                let name = if action["action"] == "provider.setup" {
                    "providerSetUp"
                } else {
                    "providerDisconnect"
                };
                self.native
                    .platform
                    .push(("command".into(), json!({ "name": name })));
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            Some("row")
                if action["action"]
                    .as_str()
                    .is_some_and(|a| a.starts_with("agents.")) =>
            {
                let id = action["id"].as_str().unwrap_or_default();
                let client = id.strip_prefix("agents.").unwrap_or_default();
                let name = match action["action"].as_str() {
                    Some("agents.add") => format!("agentClientAdd.{client}"),
                    Some("agents.remove") => format!("agentClientRemove.{client}"),
                    Some("agents.copy") => "agentClientsCopy".into(),
                    _ => return Err("Unknown agent app action".into()),
                };
                if !client.bytes().all(|b| b.is_ascii_lowercase() || b == b'-') {
                    return Err("Unknown agent app".into());
                }
                self.native
                    .platform
                    .push(("command".into(), json!({ "name": name })));
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            Some("write") if action["id"] == "timeline.skimming" => {
                let on = action["value"]
                    .as_bool()
                    .ok_or("Hover to Scrub must be on or off")?;
                // The row shows the choice at once; the app saves it.
                self.native.facts["skimming"] = json!(on);
                let name = if on { "skimmingOn" } else { "skimmingOff" };
                self.native
                    .platform
                    .push(("command".into(), json!({ "name": name })));
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            Some("write") if action["id"] == "account.remember" => {
                let on = action["value"]
                    .as_bool()
                    .ok_or("Remember Installs must be on or off")?;
                let name = if on {
                    "cloudRememberOn"
                } else {
                    "cloudRememberOff"
                };
                self.native
                    .platform
                    .push(("command".into(), json!({ "name": name })));
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            Some("write") if action["id"] == "speech.language" => {
                let store = crate::speech::store().ok_or("Transcription isn’t available here")?;
                store.set_language(action["value"].as_str().ok_or("Choose a language.")?)?;
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            Some("write") => {
                let value = &action["value"];
                if action["id"] == "memory.enabled" {
                    let enabled = value
                        .as_bool()
                        .ok_or("Project memory access must be on or off")?;
                    let p = self.published().ok_or("No open project")?;
                    let root = p.snapshot.root().ok_or("Project closed")?;
                    return Ok(NativeCommand::Kernel(
                        Request::new(
                            self.caller,
                            pm_kernel::MemoryCall::SetEnabled {
                                expected: root.get("memoryEnabled").map(pm_model::Node::to_value),
                                enabled,
                            },
                        )
                        .with_document(p.snapshot.doc_id),
                    ));
                }
                let mut patch = CompPatch::default();
                match action["id"].as_str() {
                    Some("project.name") => {
                        patch.name = Some(value.as_str().ok_or("Project name must be text")?.into())
                    }
                    Some("project.size") => {
                        let Some((w, h)) =
                            value
                                .as_str()
                                .and_then(|v| v.split_once('x'))
                                .and_then(|(w, h)| {
                                    Some((w.parse::<f64>().ok()?, h.parse::<f64>().ok()?))
                                })
                        else {
                            // Custom: the Width and Height rows set it.
                            return Err("Choose a size, or set Width and Height".into());
                        };
                        patch.width = Some(number(&json!(w), 16., 8192.)?);
                        patch.height = Some(number(&json!(h), 16., 8192.)?);
                    }
                    Some("project.width") => patch.width = Some(number(value, 16., 8192.)?),
                    Some("project.height") => patch.height = Some(number(value, 16., 8192.)?),
                    Some("project.fps") => patch.fps = Some(number(value, 1., 240.)?),
                    Some("project.duration") => {
                        patch.duration = Some(number(value, 0.1, f64::MAX)?)
                    }
                    Some("project.colorDepth") => {
                        patch.color_depth = Some(
                            value
                                .as_u64()
                                .filter(|n| [8, 16, 32].contains(n))
                                .ok_or("Invalid color depth")? as u32,
                        );
                    }
                    Some("project.output") => {
                        patch.output = Some(
                            value
                                .as_str()
                                .and_then(pm_protocol::color::ProjectOutput::parse)
                                .ok_or("Output must be sdr, pq or hlg")?,
                        );
                    }
                    Some("project.masteringPeak") => {
                        patch.mastering_peak = Some(number(value, 400., 10000.)?);
                    }
                    Some(id)
                        if let Some((_, field, ..)) = EXPORT_ROWS
                            .iter()
                            .find(|(row, ..)| id.strip_prefix("project.") == Some(row)) =>
                    {
                        let p = self.published().ok_or("No open project")?;
                        let root = p.snapshot.root().ok_or("Project closed")?;
                        let fps =
                            pm_kernel::model::comp_node(root, p.snapshot.open_comp().as_deref())
                                .and_then(|c| c.get("fps"))
                                .and_then(|n| n.to_value().as_f64())
                                .unwrap_or(30.);
                        let mut defaults = self.export_defaults(root, fps);
                        defaults[*field] = value.clone();
                        // Only values a row offers, or a boolean for a switch.
                        let mut checked =
                            pm_promedia::export_panel::normalize_defaults(&defaults, fps);
                        if checked[*field] != *value {
                            return Err("Invalid export setting".into());
                        }
                        // Setting the frame rate here is choosing it; another
                        // row keeps whether it was chosen.
                        if *field == "fps" || defaults[EXPORT_RATE_CHOSEN] == json!(true) {
                            checked[EXPORT_RATE_CHOSEN] = json!(true);
                        }
                        patch.export_defaults = Some(
                            pm_model::json::parse(&checked.to_string())
                                .map_err(|e| format!("{e:?}"))?,
                        );
                    }
                    Some("project.background") => {
                        let s = value.as_str().ok_or("Background must be a colour")?;
                        if s.len() != 7
                            || !s.starts_with('#')
                            || !s.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
                        {
                            return Err("Background must be #RRGGBB".into());
                        }
                        patch.background = Some(s.into());
                    }
                    _ => return Err("Settings service is unavailable for this row".into()),
                }
                let p = self.published().ok_or("No open project")?;
                Ok(NativeCommand::Kernel(
                    Request::new(
                        self.caller,
                        Call::Transact {
                            label: Some("Composition Settings".into()),
                            expected_revision: Some(p.revision()),
                            body: WriteBody::Commands(vec![
                                CompCommand::Set {
                                    patch: Box::new(patch),
                                }
                                .into(),
                            ]),
                        },
                    )
                    .with_document(p.snapshot.doc_id),
                ))
            }
            Some("extensions.retry") => Ok(NativeCommand::Value(self.settings_snapshot()?)),
            Some("speech") => {
                let store = crate::speech::store().ok_or("Transcription isn’t available here")?;
                let id = action["id"].as_str().ok_or("Speech model")?;
                match action["action"].as_str() {
                    Some("download" | "retry") => store.download(id)?,
                    Some("cancel") => store.cancel(id),
                    Some("use") => store.set_active(id)?,
                    // Deleting asks first (S092): the app's alert, then the store.
                    Some("remove") => self
                        .native
                        .platform
                        .push(("command".into(), json!({"name":"speechDelete","args":[id]}))),
                    _ => return Err("Unknown speech model action".into()),
                }
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            Some("row") if action["action"] == "notifications.open-settings" => {
                self.native.platform.push((
                    "command".into(),
                    json!({"name": "openNotificationSettings"}),
                ));
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            // The Powermove account's sheets, Keychain, registry and billing
            // calls are the app's.
            Some("row")
                if matches!(
                    action["action"].as_str(),
                    Some(
                        "account.sign-in"
                            | "account.sign-out"
                            | "account.delete"
                            | "account.subscribe"
                            | "account.portal"
                            | "account.checkout-cancel"
                            | "account.billing-refresh"
                    )
                ) =>
            {
                let name = match action["action"].as_str() {
                    Some("account.sign-in") => "cloudSignIn".to_owned(),
                    Some("account.sign-out") => "cloudSignOut".to_owned(),
                    Some("account.portal") => "cloudPortal".to_owned(),
                    Some("account.checkout-cancel") => "cloudCheckoutCancel".to_owned(),
                    Some("account.billing-refresh") => "cloudBilling".to_owned(),
                    // Only a plan the account's billing offers is checked out.
                    Some("account.subscribe") => {
                        let plan = action["plan"].as_str().unwrap_or_default();
                        let offered = self.native.facts["cloud"]["billing"]["plans"]
                            .as_array()
                            .is_some_and(|p| p.iter().any(|o| o["id"] == plan));
                        if !offered {
                            return Err("That plan isn’t offered.".into());
                        }
                        format!("cloudCheckout.{plan}")
                    }
                    _ => "cloudDeleteAccount".to_owned(),
                };
                self.native
                    .platform
                    .push(("command".into(), json!({ "name": name })));
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            // Settings › Shortcuts: the app owns the keymap, its key recording
            // and its file panels.
            Some("shortcut") => {
                self.native.platform.push((
                    "command".into(),
                    json!({"name": "shortcuts", "args": [action]}),
                ));
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            Some("row") if action["action"] == "about.check-updates" => {
                self.native
                    .platform
                    .push(("command".into(), json!({"name": "checkForUpdates"})));
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            // Settings › Privacy › What We Collect: the public list.
            Some("row") if action["action"] == "privacy.what-we-collect" => {
                self.native
                    .platform
                    .push(("command".into(), json!({"name": "openTelemetryDocs"})));
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            // The After Effects workspace comes over with the agent, in its panel.
            Some("row") if action["action"] == "workspace.import" => {
                self.native
                    .platform
                    .push(("command".into(), json!({"name": "bringWorkspace"})));
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            // Keychain consent belongs to the app: it asks before anything is read.
            Some("row") if action["action"] == "account.bring-sign-ins" => {
                self.native
                    .platform
                    .push(("command".into(), json!({"name": "bringSignIns"})));
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            Some("row")
                if action["action"]
                    .as_str()
                    .is_some_and(|a| a.starts_with("providers.")) =>
            {
                let accounts = self
                    .native
                    .agent_link
                    .accounts()
                    .ok_or("Agent accounts are unavailable")?;
                match action["action"].as_str().unwrap_or_default() {
                    "providers.openai.sign-in" => accounts.connect(Provider::Chatgpt),
                    "providers.openai.sign-out" => accounts.disconnect(Provider::Chatgpt),
                    "providers.claude.sign-in" => accounts.connect(Provider::Claude),
                    "providers.claude.sign-out" => accounts.disconnect(Provider::Claude),
                    "providers.anthropic.remove" => accounts.remove_anthropic_key()?,
                    // The secure field is the app's native sheet.
                    kind @ ("providers.openai.key" | "providers.anthropic.key") => {
                        let which = if kind == "providers.openai.key" { "openai" } else { "anthropic" };
                        self.native.platform.push((
                            "command".into(),
                            json!({"name": format!("providerKey.{which}")}),
                        ));
                    }
                    _ => return Err("Unknown provider action".into()),
                }
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            Some("row") => {
                let id = action["id"].as_str().unwrap_or_default();
                let provider = match id {
                    "account.chatgpt" => Provider::Chatgpt,
                    "account.claude" => Provider::Claude,
                    _ => return Err("Settings service is unavailable for this action".into()),
                };
                let accounts = self
                    .native
                    .agent_link
                    .accounts()
                    .ok_or("Agent accounts are unavailable")?;
                match action["action"].as_str() {
                    Some("account.connect") => accounts.connect(provider),
                    Some("account.disconnect") => accounts.disconnect(provider),
                    _ => return Err("Unknown account action".into()),
                }
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            // The window's extension host follows this state and activates or unloads.
            Some("extension.enabled") => {
                let id = action["id"].as_str().ok_or("Extension id")?;
                let enabled = action["enabled"] == true;
                pm_kernel::state::switch_extension(&self.kernel.state(), id, enabled)
                    .map(NativeCommand::State)
                    .ok_or_else(|| "Unknown extension".into())
            }
            Some("extension.reload") => {
                let id = action["id"].as_str().ok_or("Extension id")?;
                let state = self.kernel.state();
                let entry = state.extensions.get(id).ok_or("Unknown extension")?;
                if !entry.installed || !entry.enabled || entry.stopped_this_run()
                    || crate::models::extension_attention(entry.source.get("hostRecord")) != Some("error")
                {
                    return Err("This extension cannot be reloaded here.".into());
                }
                self.native.platform.push((
                    "command".into(),
                    json!({"name": format!("extensionReload.{id}")}),
                ));
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            // Mods (S105): Show in Finder and Delete… are the app's (Finder,
            // a confirmation, then the archive), for what the row allows.
            Some(kind @ ("extension.finder" | "extension.archive")) => {
                use pm_services::surface_backing::projects::{self, ModAction};
                let id = action["id"].as_str().ok_or("Extension id")?;
                let (action, command) = if kind == "extension.finder" {
                    (ModAction::Reveal, "extensionReveal")
                } else {
                    (ModAction::Archive, "extensionArchive")
                };
                let records = crate::models::mod_records(&self.kernel.state());
                let mut host = PushMod {
                    platform: &mut self.native.platform,
                    command,
                };
                projects::act_on_mod(&mut host, &records, id, action).map_err(|e| e.to_string())?;
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            // Speech actions require their actual services.
            _ => Err("Settings service is unavailable for this action".into()),
        }
    }
}
/// Hands an allowed Mods action to the app as `<command>.<id>`.
struct PushMod<'a> {
    platform: &'a mut Vec<(String, Value)>,
    command: &'static str,
}
impl pm_services::surface_backing::projects::ModsHost for PushMod<'_> {
    fn act(
        &mut self,
        _: u64,
        id: &str,
        _: pm_services::surface_backing::projects::ModAction,
    ) -> pm_services::Result<()> {
        self.platform.push((
            "command".into(),
            json!({"name": format!("{}.{id}", self.command)}),
        ));
        Ok(())
    }
}
/// Settings › Account › Powermove Account, from the app's `cloud` fact
/// Row actions that sign out, disconnect or delete: from an older Settings
/// publication they land only on the target that publication showed.
#[cfg(test)]
pub(crate) fn destructive_for_test(action: &str) -> bool {
    destructive(action)
}
fn destructive(action: &str) -> bool {
    matches!(
        action,
        "account.disconnect"
            | "account.sign-out"
            | "account.delete"
            | "provider.disconnect"
            | "agents.remove"
            | "providers.openai.sign-out"
            | "providers.claude.sign-out"
            | "providers.anthropic.remove"
    ) || action.starts_with("storage.delete.")
}
/// (v1.2.0 SettingsScreen): Sign In… while signed out; the identity with Sign
/// Out, Remember Installs and Delete Account… while signed in.
pub(crate) fn cloud_rows(cloud: &Value) -> Vec<Value> {
    if !cloud.is_object() {
        return Vec::new();
    }
    let busy = cloud["busy"] == true;
    let row = |id: &str, label: &str, control: &str| {
        json!({"id":format!("account.{id}"),"section":"account","group":"Powermove Account",
            "label":label,"control":control,"disabled":busy})
    };
    if cloud["signedIn"] != true {
        let mut sign_in = row("sign-in", "Powermove Account", "action");
        sign_in["action"] = json!("account.sign-in");
        sign_in["actionLabel"] = json!("Sign In…");
        sign_in["detail"] = json!("Publish and manage your extensions on the Store.");
        let mut remember = row("remember", "Remember Installs on This Account", "switch");
        remember["value"] = json!(false);
        remember["disabled"] = json!(true);
        return vec![sign_in, remember];
    }
    let identity = cloud["identity"].as_str().unwrap_or_default();
    let email = cloud["email"].as_str().unwrap_or_default();
    let mut account = row("sign-in", identity, "action");
    account["target"] = json!(identity);
    account["action"] = json!("account.sign-out");
    account["actionLabel"] = json!("Sign Out");
    if identity != email {
        account["detail"] = json!(email);
    }
    let mut remember = row("remember", "Remember Installs on This Account", "switch");
    remember["value"] = json!(cloud["rememberInstalls"] == true);
    remember["detail"] = json!(
        "Keep a list of the extensions you install from the Store. Turning this off deletes the list."
    );
    let mut delete = row("delete", "Delete Account", "action");
    delete["target"] = json!(identity);
    delete["action"] = json!("account.delete");
    delete["actionLabel"] = json!("Delete Account…");
    delete["detail"] = json!("Your published extensions stay on the Store under your handle.");
    // Deleting the account is the page's last card, after the plan.
    delete["group"] = json!("Delete Account");
    let mut rows = vec![account, remember];
    rows.extend(billing_rows(&cloud["billing"], busy));
    rows.push(delete);
    rows
}
/// Settings › Account's plan card and this month's credits, from the
/// account's billing (`GET /v2/billing`, the app's `cloud.billing` fact).
/// Nothing before it is read, or when the backend has no billing.
pub(crate) fn billing_rows(billing: &Value, busy: bool) -> Vec<Value> {
    const PLAN: &str = "Plan";
    const CREDITS: &str = "Credits This Month";
    let row = |id: &str, group: &str, label: &str, control: &str| {
        json!({"id":format!("account.{id}"),"section":"account","group":group,
            "label":label,"control":control,"disabled":busy})
    };
    let action = |mut r: Value, action: &str, label: &str| {
        r["action"] = json!(action);
        r["actionLabel"] = json!(label);
        r
    };
    match billing["state"].as_str() {
        Some("ready") => {}
        Some("error") => {
            let mut r = action(
                row("plan", PLAN, "Plan", "action"),
                "account.billing-refresh",
                "Try Again",
            );
            r["value"] = json!("Couldn’t load your plan.");
            return vec![r];
        }
        _ => return vec![],
    }
    let dollars = |cents: u64| {
        if cents % 100 == 0 {
            format!("${}", cents / 100)
        } else {
            format!("${}.{:02}", cents / 100, cents % 100)
        }
    };
    let resets = billing["resets"].as_str().unwrap_or_default();
    let subscribed = billing["subscribed"] == true;
    let mut rows = vec![];
    if !subscribed {
        let mut free = row("plan", PLAN, "Free", "status");
        free["value"] = json!(pm_agents::inference::NEEDS_PLAN);
        rows.push(free);
        if billing["waiting"] == "checkout" {
            rows.push(action(
                row(
                    "waiting",
                    PLAN,
                    "Waiting for checkout in your browser…",
                    "action",
                ),
                "account.checkout-cancel",
                "Cancel",
            ));
        } else {
            let plans = billing["plans"].as_array().cloned().unwrap_or_default();
            let mut choose = action(
                row("choose", PLAN, "Choose a Plan", "plans"),
                "account.subscribe",
                "Continue…",
            );
            let name = |p: &Value| p["name"].as_str().unwrap_or_default().to_owned();
            choose["options"] = plans
                .iter()
                .map(|p| {
                    let price = dollars(p["priceCents"].as_u64().unwrap_or(0));
                    json!({"v": p["id"], "label": format!("{} · {price}", name(p))})
                })
                .collect();
            // What each plan holds, then the rules: monthly, no roll-over.
            let holds: Vec<String> = plans
                .iter()
                .map(|p| {
                    let credits = pm_agents::inference::grouped(p["credits"].as_u64().unwrap_or(0));
                    format!("{} has {credits} credits a month", name(p))
                })
                .collect();
            choose["detail"] = json!(format!(
                "{}. Credits reset each month and don’t roll over.",
                holds.join(", ")
            ));
            if let Some(error) = billing["error"].as_str() {
                choose["error"] = json!(error);
            }
            rows.push(choose);
        }
        return rows;
    }
    let plan = &billing["plan"];
    let name = plan["name"].as_str().unwrap_or_default();
    let mut card = action(
        row("plan", PLAN, name, "action"),
        "account.portal",
        "Manage Billing…",
    );
    card["value"] = json!(match () {
        _ if billing["status"] == "past_due" => "Payment failed".to_owned(),
        _ if billing["cancelAtPeriodEnd"] == true => format!("Cancels {resets}"),
        _ => format!(
            "{} a month",
            dollars(plan["priceCents"].as_u64().unwrap_or(0))
        ),
    });
    if billing["status"] == "past_due" {
        card["error"] = json!("Update your payment method to keep Aero running.");
    }
    if let Some(error) = billing["error"].as_str() {
        card["error"] = json!(error);
    }
    rows.push(card);
    let credits = &billing["credits"];
    let count = |k: &str| credits[k].as_u64().unwrap_or(0);
    let mut left = row(
        "credits",
        CREDITS,
        &format!(
            "{} of {} left",
            pm_agents::inference::grouped(count("remaining")),
            pm_agents::inference::grouped(count("included"))
        ),
        "status",
    );
    left["value"] = json!(format!("Resets {resets}"));
    left["meter"] = json!({"used": count("used"), "included": count("included")});
    if count("remaining") == 0 {
        left["error"] = json!(format!("You’re out of credits until {resets}."));
    }
    rows.push(left);
    let mut daily = row("daily", CREDITS, "Daily Use", "status");
    daily["value"] = json!(format!(
        "{} used",
        pm_agents::inference::grouped(count("used"))
    ));
    daily["days"] = billing["daily"].clone();
    daily["slots"] = billing["periodDays"].clone();
    rows.push(daily);
    if count("remaining") == 0
        && let Some(upgrade) = billing["upgrade"].as_object()
    {
        let label = format!(
            "{} · {} credits a month",
            upgrade["name"].as_str().unwrap_or_default(),
            pm_agents::inference::grouped(upgrade["credits"].as_u64().unwrap_or(0))
        );
        rows.push(action(
            row("upgrade", CREDITS, &label, "action"),
            "account.portal",
            "Upgrade…",
        ));
    }
    rows
}
/// Settings › Agents › API or Local Model, from the app's `provider` fact:
/// what is connected (never the key), then Set Up… / Change… and Disconnect.
/// The address, key and model are entered in the app's connection sheet.
pub(crate) fn provider_rows(provider: &Value) -> Vec<Value> {
    if !provider.is_object() {
        return Vec::new();
    }
    let status = |id: &str, label: &str, value: Value| {
        json!({"id":format!("account.{id}"),"section":"agents","group":"API or Local Model",
            "label":label,"control":"status","value":value})
    };
    let connected = provider["connected"] == true;
    let text = |key: &str| provider[key].as_str().unwrap_or("").to_owned();
    let dash = || "—".to_owned();
    let mut rows = vec![
        status(
            "provider",
            "Connection Type",
            json!(if connected {
                text("type")
            } else {
                "Not set up".into()
            }),
        ),
        status(
            "url",
            "API Base URL",
            json!(if connected { text("url") } else { dash() }),
        ),
        status(
            "key",
            "API Key",
            json!(match (connected, provider["hasKey"] == true) {
                (true, true) => "Saved in Keychain",
                (true, false) => "None",
                _ => "—",
            }),
        ),
        status(
            "model",
            "Model Name",
            json!(if connected { text("model") } else { dash() }),
        ),
        status(
            "vision",
            "This Model Supports Images",
            json!(match (connected, provider["vision"] == true) {
                (true, true) => "Yes",
                (true, false) => "No",
                _ => "—",
            }),
        ),
    ];
    let mut action = json!({"id":"account.status","section":"agents","group":"API or Local Model",
        "label":"Status","control":"action","action":"provider.setup",
        "actionLabel":if connected { "Change…" } else { "Set Up…" },"disabled":provider["busy"] == true});
    action["detail"] = json!(if connected {
        "Tested and saved."
    } else {
        "Use an API, Ollama, or LM Studio."
    });
    // The connection a press acts on: its type, address and model.
    let target = connected.then(|| json!([text("type"), text("url"), text("model")]));
    action["target"] = json!(target);
    rows.push(action);
    if connected {
        rows.push(json!({"id":"account.disconnect","section":"agents","group":"API or Local Model",
            "label":"Disconnect","control":"action","action":"provider.disconnect","actionLabel":"Disconnect",
            "target":target}));
    }
    rows
}
/// Settings › Agents › Other Agent Apps (main's AgentClientsSettings): one
/// row per agent app on this Mac with Add Powermove / Remove, then Copy Setup
/// for any other app.
pub(crate) fn agent_client_rows(agents: &Value) -> Vec<Value> {
    let Some(clients) = agents["clients"].as_array() else {
        return Vec::new();
    };
    let busy = agents["busy"].as_str();
    let mut rows = Vec::new();
    for client in clients.iter().filter(|c| c["state"] != "not-installed") {
        let id = client["id"].as_str().unwrap_or_default();
        let custom = client["custom"] == true;
        let note = agents["notes"][id].as_str();
        let detail = note
            .map(str::to_owned)
            .unwrap_or_else(|| match client["state"].as_str() {
                Some("unreadable") => {
                    "Its settings can’t be edited automatically. Use Copy Setup below.".into()
                }
                _ if custom => "Powermove is set up in this app.".into(),
                _ => client["detail"].as_str().unwrap_or_default().into(),
            });
        let mut row = json!({"id":format!("agents.{id}"),"section":"agents","group":"Other Agent Apps",
            "label":client["name"],"detail":detail,"disabled":busy.is_some()});
        let working = busy == Some(id);
        match (client["state"].as_str(), custom) {
            (Some("connected"), false) => {
                row["control"] = json!("action");
                row["action"] = json!("agents.remove");
                row["actionLabel"] = json!(if working { "Removing…" } else { "Remove" });
            }
            (Some("available"), _) => {
                row["control"] = json!("action");
                row["action"] = json!("agents.add");
                row["actionLabel"] = json!(if working {
                    "Adding…"
                } else {
                    "Add Powermove"
                });
            }
            (Some("connected"), true) => {
                row["control"] = json!("status");
                row["value"] = json!("Added");
            }
            _ => {
                row["control"] = json!("status");
                row["value"] = json!("");
            }
        }
        rows.push(row);
    }
    rows.push(json!({"id":"agents.other","section":"agents","group":"Other Agent Apps",
        "label":"Any Other Agent","control":"action","action":"agents.copy",
        "actionLabel":if agents["notes"]["other"] == "Copied." { "Copied" } else { "Copy Setup" },
        "detail":"Paste into any app that supports MCP servers, such as Cline, Roo Code or Kiro. It runs on that app’s own plan."}));
    rows
}
fn number(value: &Value, min: f64, max: f64) -> Result<f64, String> {
    value
        .as_f64()
        .filter(|n| n.is_finite() && *n >= min && *n <= max)
        .ok_or("Invalid setting value".into())
}

pub(crate) fn validate_profile(key: &str, value: &Value) -> Result<(), String> {
    let valid = match key {
        "themeMode" => matches!(value.as_str(), Some("system" | "light" | "dark")),
        "openProjectsIn" => matches!(value.as_str(), Some("tabs" | "windows")),
        "previewMemoryMiB" => value
            .as_f64()
            .is_some_and(|n| [0., 128., 256., 512., 1024., 2048.].contains(&n)),
        "autoDownloadCloudMedia" | "restoreWindows" | "shareUsageData" => value.is_boolean(),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err("Invalid profile setting".into())
    }
}

/// Settings › Media › Original Media: guarded folders whose originals the
/// migration couldn't open, with their files; Allow Access asks macOS for the
/// ones it still can. None when every original came across.
fn protected_folders_row(folders: &Value) -> Option<Value> {
    let folders = folders.as_array().filter(|f| !f.is_empty())?;
    let named = |refused: bool| {
        let names: Vec<String> = folders
            .iter()
            .filter(|f| f["refused"].as_bool() == Some(refused))
            .map(|f| {
                let files = f["files"].as_u64().unwrap_or(0);
                let unit = if files == 1 { "file" } else { "files" };
                format!("{} ({files} {unit})", f["name"].as_str().unwrap_or(""))
            })
            .collect();
        match names.as_slice() {
            [] => None,
            [one] => Some(one.clone()),
            [rest @ .., last] => Some(format!("{} and {last}", rest.join(", "))),
        }
    };
    let askable = named(false);
    let refused = named(true);
    let mut detail = Vec::new();
    if let Some(names) = &askable {
        detail.push(format!(
            "Originals in {names} weren’t opened, so they’re offline. Allow Access asks macOS once for each folder, then brings them in."
        ));
    }
    if let Some(names) = &refused {
        detail.push(format!(
            "macOS refused {names}: allow Powermove in System Settings › Privacy & Security › Files and Folders, then check again."
        ));
    }
    // macOS won't ask about a folder it refused again, but access granted in
    // System Settings since is checked again.
    let action = if askable.is_some() {
        "Allow Access…"
    } else {
        "Check Again…"
    };
    Some(
        json!({"id":"media.folders","section":"media","group":"Original Media",
        "label":"Media in Protected Folders","control":"action","action":"media.folders",
        "actionLabel":action,"disabled":false,"detail":detail.join(" ")}),
    )
}
/// macOS's answer about agent notifications (S114), as the app last read it.
static NOTIFICATIONS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
/// The app records the notification center's answer whenever it changes.
pub fn set_notification_status(status: pm_services::surface_backing::authorization::Authorization) {
    use pm_services::surface_backing::authorization::Authorization::*;
    let code = match status {
        NotDetermined => 0,
        Authorized | Provisional => 1,
        Denied => 2,
        _ => 3,
    };
    NOTIFICATIONS.store(code, std::sync::atomic::Ordering::Release);
}
/// Settings › General › Notifications: the outcome, and the way to change it.
fn notification_row() -> Value {
    use pm_services::surface_backing::{authorization::Authorization, copy::notification_detail};
    let code = NOTIFICATIONS.load(std::sync::atomic::Ordering::Acquire);
    let status = match code {
        1 => Authorization::Authorized,
        2 => Authorization::Denied,
        _ => Authorization::NotDetermined,
    };
    let mut row = json!({"id":"notifications.agent","section":"agents","group":"Permissions",
        "label":"Agent Notifications","detail":notification_detail(status)});
    if status == Authorization::Denied {
        row["control"] = json!("action");
        row["action"] = json!("notifications.open-settings");
        row["actionLabel"] = json!("Open System Settings…");
    } else {
        row["control"] = json!("status");
        row["value"] = json!(if code == 1 { "Allowed" } else { "Not Set Up" });
    }
    row
}

#[cfg(test)]
mod protected_folders_tests {
    use super::protected_folders_row;
    use serde_json::json;
    #[test]
    fn the_row_names_each_folder_with_its_files_and_what_macos_still_asks() {
        assert_eq!(protected_folders_row(&json!([])), None);
        let row = protected_folders_row(&json!([
            {"name":"Downloads","files":3,"refused":false},
            {"name":"Desktop","files":1,"refused":false},
            {"name":"Documents","files":2,"refused":true},
        ]))
        .unwrap();
        assert_eq!(row["disabled"], false);
        assert_eq!(row["actionLabel"], "Allow Access…");
        let detail = row["detail"].as_str().unwrap();
        assert!(
            detail.starts_with(
                "Originals in Downloads (3 files) and Desktop (1 file) weren’t opened"
            ),
            "{detail}"
        );
        assert!(
            detail.contains("macOS refused Documents (2 files)"),
            "{detail}"
        );
        // Only refused folders: Check Again, for access granted in System
        // Settings since.
        let row =
            protected_folders_row(&json!([{"name":"Desktop","files":1,"refused":true}])).unwrap();
        assert_eq!(row["disabled"], false);
        assert_eq!(row["actionLabel"], "Check Again…");
    }
}
#[cfg(test)]
mod cloud_tests {
    use super::cloud_rows;
    use serde_json::json;
    #[test]
    fn powermove_account_rows_follow_the_session() {
        assert!(cloud_rows(&serde_json::Value::Null).is_empty());
        let out = cloud_rows(&json!({"revision":2,"signedIn":false}));
        assert_eq!(out[0]["action"], "account.sign-in");
        assert_eq!(out[0]["actionLabel"], "Sign In…");
        assert_eq!(out[1]["disabled"], true);
        let signed = json!({"revision":3,"signedIn":true,"identity":"@maker",
            "email":"person@example.com","rememberInstalls":true});
        let out = cloud_rows(&signed);
        assert_eq!(out[0]["label"], "@maker");
        assert_eq!(out[0]["detail"], "person@example.com");
        assert_eq!(out[0]["action"], "account.sign-out");
        assert_eq!(out[1]["value"], true);
        assert_eq!(out[1]["disabled"], false);
        assert_eq!(out[2]["action"], "account.delete");
        assert_eq!(out[2]["group"], "Delete Account");
    }
    fn ready(extra: serde_json::Value) -> serde_json::Value {
        let mut billing = json!({"state":"ready","subscribed":true,
            "plan":{"id":"aerolite","name":"Aerolite","priceCents":999},
            "status":"active","cancelAtPeriodEnd":false,
            "credits":{"included":1000,"used":188,"remaining":812},"resets":"Dec 25",
            "periodDays":30,"daily":[["Nov 25",40],["Nov 26",0]],
            "plans":[{"id":"aerolite","name":"Aerolite","priceCents":999,"credits":1000},
                     {"id":"aero","name":"Aero","priceCents":1999,"credits":2500}],
            "upgrade":{"id":"aero","name":"Aero","credits":2500},"waiting":null,"error":null});
        for (k, v) in extra.as_object().unwrap() {
            billing[k] = v.clone();
        }
        billing
    }
    /// Settings › Account in each billing state: what a person reads and
    /// the one action each card offers.
    #[test]
    fn the_plan_card_and_credits_follow_the_accounts_billing() {
        use super::billing_rows;
        let ids = |rows: &[serde_json::Value]| {
            rows.iter()
                .map(|r| r["id"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        // Before it is read, or a backend without billing: nothing.
        assert!(billing_rows(&json!({"state":"loading"}), false).is_empty());
        assert!(billing_rows(&json!({"state":"missing"}), false).is_empty());
        let error = billing_rows(&json!({"state":"error"}), false);
        assert_eq!(error[0]["action"], "account.billing-refresh");
        // Free: says Aero needs a plan, then the plans and Continue…; no chart.
        let free = billing_rows(
            &ready(
                json!({"subscribed":false,"plan":null,"status":null,"resets":null,
                "credits":{"included":0,"used":0,"remaining":0},"daily":[]}),
            ),
            false,
        );
        assert_eq!(ids(&free), ["account.plan", "account.choose"]);
        assert_eq!(free[0]["value"], "Aero needs Aerolite or Aero.");
        assert_eq!(free[1]["control"], "plans");
        assert_eq!(free[1]["action"], "account.subscribe");
        assert_eq!(free[1]["actionLabel"], "Continue…");
        assert_eq!(free[1]["options"][0]["label"], "Aerolite · $9.99");
        assert_eq!(free[1]["options"][1]["label"], "Aero · $19.99");
        assert!(
            free[1]["detail"]
                .as_str()
                .unwrap()
                .contains("Aero has 2,500 credits a month")
        );
        // Waiting for Checkout: a quiet line with Cancel instead.
        let waiting = billing_rows(
            &ready(json!({"subscribed":false,"plan":null,"waiting":"checkout"})),
            false,
        );
        assert_eq!(waiting[1]["label"], "Waiting for checkout in your browser…");
        assert_eq!(waiting[1]["action"], "account.checkout-cancel");
        // Aerolite mid-period: the plan, Manage Billing…, the meter and the days.
        let lite = billing_rows(&ready(json!({})), false);
        assert_eq!(
            ids(&lite),
            ["account.plan", "account.credits", "account.daily"]
        );
        assert_eq!(lite[0]["label"], "Aerolite");
        assert_eq!(lite[0]["value"], "$9.99 a month");
        assert_eq!(lite[0]["action"], "account.portal");
        assert_eq!(lite[1]["label"], "812 of 1,000 left");
        assert_eq!(lite[1]["value"], "Resets Dec 25");
        assert_eq!(lite[1]["meter"], json!({"used":188,"included":1000}));
        assert_eq!(lite[2]["value"], "188 used");
        assert_eq!(lite[2]["slots"], 30);
        // Cancelling, past due.
        let cancelling = billing_rows(&ready(json!({"cancelAtPeriodEnd":true})), false);
        assert_eq!(cancelling[0]["value"], "Cancels Dec 25");
        let due = billing_rows(&ready(json!({"status":"past_due"})), false);
        assert_eq!(due[0]["value"], "Payment failed");
        assert!(due[0]["error"].as_str().unwrap().contains("payment method"));
        // Out of credits on Aerolite: until when, and the bigger plan.
        let out = billing_rows(
            &ready(json!({"credits":{"included":1000,"used":1000,"remaining":0}})),
            false,
        );
        assert_eq!(out[1]["error"], "You’re out of credits until Dec 25.");
        assert_eq!(out[3]["id"], "account.upgrade");
        assert_eq!(out[3]["action"], "account.portal");
        // On Aero, out of credits has no bigger plan.
        let top = billing_rows(
            &ready(json!({"upgrade":null,"credits":{"included":2500,"used":2500,"remaining":0}})),
            false,
        );
        assert_eq!(
            ids(&top),
            ["account.plan", "account.credits", "account.daily"]
        );
    }
}
#[cfg(test)]
mod provider_tests {
    use super::provider_rows;
    use serde_json::json;
    #[test]
    fn model_rows_show_the_connection_never_the_key() {
        let out = provider_rows(&json!({"revision":1,"connected":false}));
        assert_eq!(out[0]["value"], "Not set up");
        assert_eq!(out.last().unwrap()["actionLabel"], "Set Up…");
        let out = provider_rows(&json!({"revision":2,"connected":true,"type":"Ollama",
            "url":"http://localhost:11434/v1","model":"llama","vision":false,"hasKey":true}));
        assert_eq!(out[2]["value"], "Saved in Keychain");
        assert_eq!(out[5]["actionLabel"], "Change…");
        assert_eq!(out[6]["action"], "provider.disconnect");
        assert!(!out.iter().any(|r| r.to_string().contains("secret")));
    }
}
#[cfg(test)]
mod agent_client_tests {
    use super::agent_client_rows;
    use serde_json::json;
    #[test]
    fn agent_app_rows_offer_add_remove_and_copy_setup() {
        let rows = agent_client_rows(&json!({"revision":1,"clients":[
            {"id":"cursor","name":"Cursor","detail":"Your Cursor plan.","state":"available","custom":false},
            {"id":"codex","name":"Codex","detail":"","state":"not-installed","custom":false},
            {"id":"claude-desktop","name":"Claude","detail":"","state":"connected","custom":false},
            {"id":"vscode","name":"VS Code","detail":"","state":"connected","custom":true}],
            "notes":{"claude-desktop":"Quit and reopen Claude to use it."}}));
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0]["actionLabel"], "Add Powermove");
        assert_eq!(rows[1]["action"], "agents.remove");
        assert_eq!(rows[1]["detail"], "Quit and reopen Claude to use it.");
        assert_eq!(rows[2]["value"], "Added");
        assert_eq!(rows[3]["action"], "agents.copy");
    }
}
