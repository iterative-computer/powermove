//! Profile/project services used by the built-in Settings realm. Actor receipts
//! remain the only success boundary; unsupported native services refuse visibly.
use crate::{
    data::{Backend, wire},
    models::NativeCommand,
};
use pm_agents::{Provider, account::State};
use pm_kernel::{
    Call, Request, WriteBody,
    commands::comps::{CompCommand, CompPatch},
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
        rows.extend(self.subscription_rows());
        rows.extend(provider_rows(&self.native.facts["provider"]));
        rows.extend(agent_client_rows(&self.native.facts["agentClients"]));
        rows
    }
    fn subscription_rows(&self) -> Vec<Value> {
        let Some(accounts) = self.native.agent_link.accounts() else {
            return vec![notification_row()];
        };
        [(Provider::Chatgpt, "ChatGPT"), (Provider::Claude, "Claude")]
            .into_iter()
            .map(|(provider, name)| {
                let status = accounts.status(provider);
                let who = [status.email, status.plan]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · ");
                let id = format!("account.{}", name.to_lowercase());
                let mut row =
                    json!({"id":id,"section":"accounts","group":"Subscriptions","label":name});
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
    fn media_rows(&self) -> Vec<Value> {
        let media = &self.native.facts["mediaSettings"];
        if media.is_null() {
            return vec![];
        }
        let mut rows: Vec<Value> = media["rows"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| {
                json!({"id":r["id"],"section":"media","group":"Proxies","label":r["label"],
                    "control":"choice","value":r["value"],"options":r["options"],
                    "detail":r["detail"],"disabled":false})
            })
            .collect();
        let used = &media["used"];
        let space = if used.is_null() {
            "Calculating storage…".to_owned()
        } else {
            format!(
                "Proxies {} · Optimized Media {} · Renders {}",
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
                "control":"status","value":space}),
            json!({"id":"media.clear","section":"media","group":"Storage",
                "label":"Generated Media","control":"action","action":"media.clear",
                "actionLabel":"Clear…",
                "detail":"Removes proxies, optimized media and renders the open project doesn’t use. Originals, attached proxies and this project’s media stay."}),
            json!({"id":"media.reveal","section":"media","group":"Storage",
                "label":"Storage Folder","control":"action","action":"media.reveal",
                "actionLabel":"Show in Finder"}),
        ]);
        rows.extend(protected_folders_row(&media["folders"]));
        rows
    }
    pub(crate) fn settings_rows(&mut self) -> Result<Vec<Value>, String> {
        let media = self.media_rows();
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
            "project.hdr",
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
                comp.get("bg")
                    .map(|n| wire(&n.to_value()))
                    .unwrap_or(Value::Null),
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
        let hdr = color
            .and_then(|c| c.get("mode"))
            .is_some_and(|m| m.to_value() == pm_model::Value::from("hdr"));
        rows.push(json!({"id":"project.hdr","section":"project","group":"Color",
            "label":"HDR","control":"switch","value":hdr,
            "detail":"Keeps PQ and HLG light in the project for HDR delivery. Off, HDR footage is tone mapped to SDR white as it comes in.",
            "binding":fields["project.hdr"],"disabled":false}));
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
        rows.push(json!({"id":"memory.enabled","section":"memory","group":"Project Memory",
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
        pm_promedia::export_panel::normalize_defaults(&stored, fps)
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
    pub(crate) fn settings_action(&mut self, args: &Value) -> Result<NativeCommand, String> {
        self.sync()?;
        if args["revision"].as_u64() != Some(self.native.settings_sequence) {
            return Err("Settings changed; retry from the latest publication".into());
        }
        let action = &args["action"];
        match action["type"].as_str() {
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
                    Some("project.hdr") => {
                        patch.hdr = Some(value.as_bool().ok_or("HDR must be on or off")?);
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
                        let checked = pm_promedia::export_panel::normalize_defaults(&defaults, fps);
                        if checked[*field] != *value {
                            return Err("Invalid export setting".into());
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
            Some("row") if action["action"] == "notifications.open-settings" => {
                self.native.platform.push((
                    "command".into(),
                    json!({"name": "openNotificationSettings"}),
                ));
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            // The Powermove account's sheets, Keychain and registry calls are the app's.
            Some("row")
                if matches!(
                    action["action"].as_str(),
                    Some("account.sign-in" | "account.sign-out" | "account.delete")
                ) =>
            {
                let name = match action["action"].as_str() {
                    Some("account.sign-in") => "cloudSignIn",
                    Some("account.sign-out") => "cloudSignOut",
                    _ => "cloudDeleteAccount",
                };
                self.native
                    .platform
                    .push(("command".into(), json!({ "name": name })));
                Ok(NativeCommand::Value(self.settings_snapshot()?))
            }
            // Keychain consent belongs to the app: it asks before anything is read.
            Some("row") if action["action"] == "account.bring-sign-ins" => {
                self.native
                    .platform
                    .push(("command".into(), json!({"name": "bringSignIns"})));
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
                let mut entry = self
                    .kernel
                    .state()
                    .extensions
                    .get(id)
                    .cloned()
                    .ok_or("Unknown extension")?;
                entry.enabled = action["enabled"] == true;
                // Turning on an extension D11 stopped this run runs it again.
                if entry.enabled
                    && let Some(record) = entry
                        .source
                        .as_object_mut()
                        .and_then(|source| source.get_mut("hostRecord"))
                        .and_then(pm_model::Value::as_object_mut)
                {
                    record.remove("stopped");
                }
                Ok(NativeCommand::State(pm_kernel::state::StateTransaction {
                    expected_revision: None,
                    ops: vec![pm_kernel::state::StateOp::Extension(entry)],
                }))
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
/// Settings › Accounts › Powermove Account, from the app's `cloud` fact
/// (v1.2.0 SettingsScreen): Sign In… while signed out; the identity with Sign
/// Out, Remember Installs and Delete Account… while signed in.
pub(crate) fn cloud_rows(cloud: &Value) -> Vec<Value> {
    if !cloud.is_object() {
        return Vec::new();
    }
    let busy = cloud["busy"] == true;
    let row = |id: &str, label: &str, control: &str| {
        json!({"id":format!("account.{id}"),"section":"accounts","group":"Powermove Account",
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
    delete["action"] = json!("account.delete");
    delete["actionLabel"] = json!("Delete Account…");
    delete["detail"] = json!("Your published extensions stay on the Store under your handle.");
    vec![account, remember, delete]
}
/// Settings › Accounts › API or Local Model, from the app's `provider` fact:
/// what is connected (never the key), then Set Up… / Change… and Disconnect.
/// The address, key and model are entered in the app's connection sheet.
pub(crate) fn provider_rows(provider: &Value) -> Vec<Value> {
    if !provider.is_object() {
        return Vec::new();
    }
    let status = |id: &str, label: &str, value: Value| {
        json!({"id":format!("account.{id}"),"section":"accounts","group":"API or Local Model",
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
    let mut action = json!({"id":"account.status","section":"accounts","group":"API or Local Model",
        "label":"Status","control":"action","action":"provider.setup",
        "actionLabel":if connected { "Change…" } else { "Set Up…" },"disabled":provider["busy"] == true});
    action["detail"] = json!(if connected {
        "Tested and saved."
    } else {
        "Use an API, Ollama, or LM Studio."
    });
    rows.push(action);
    if connected {
        rows.push(json!({"id":"account.disconnect","section":"accounts","group":"API or Local Model",
            "label":"Disconnect","control":"action","action":"provider.disconnect","actionLabel":"Disconnect"}));
    }
    rows
}
/// Settings › Accounts › Other Agent Apps (main's AgentClientsSettings): one
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
        let mut row = json!({"id":format!("agents.{id}"),"section":"accounts","group":"Other Agent Apps",
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
    rows.push(json!({"id":"agents.other","section":"accounts","group":"Other Agent Apps",
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
        "previewMemoryMiB" => value
            .as_f64()
            .is_some_and(|n| [0., 128., 256., 512., 1024., 2048.].contains(&n)),
        "autoDownloadCloudMedia" | "restoreWindows" => value.is_boolean(),
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
    let mut row = json!({"id":"notifications.agent","section":"general","group":"Notifications",
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
