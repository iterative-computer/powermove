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
/// The formats this app exports, with v1's labels.
const EXPORT_FORMATS: [(&str, &str); 6] = [
    ("prores", "QuickTime · ProRes 4444"),
    ("mp4", "MP4 · H.264"),
    ("webm", "WebM · VP9"),
    ("png", "PNG Sequence"),
    ("still", "Still Frame (PNG)"),
    ("json", "Project File (.pmv)"),
];
/// JavaScript `Number(value)` for a JSON value.
fn js_number(value: &Value) -> f64 {
    match value {
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Null => 0.,
        Value::String(text) => {
            let text = text.trim();
            if text.is_empty() {
                0.
            } else {
                text.parse().unwrap_or(f64::NAN)
            }
        }
        _ => f64::NAN,
    }
}
/// main's `normalizeExportDefaults(raw, fallbackFps)`, field for field.
pub(crate) fn normalize_export_defaults(stored: &Value, fallback_fps: f64) -> Value {
    let pick = |key: &str, allowed: &[&str], fallback: &str| {
        let value = stored[key].as_str().filter(|v| allowed.contains(v));
        json!(value.unwrap_or(fallback))
    };
    let flag = |key: &str, fallback: bool| stored[key].as_bool().unwrap_or(fallback);
    let base_fps = if fallback_fps.is_finite() && fallback_fps > 0. {
        fallback_fps.min(240.)
    } else {
        30.
    };
    let fps = js_number(&stored["fps"]);
    let scale = js_number(&stored["scale"]);
    let number = |n: f64| serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number);
    json!({
        "format": pick("format", &V1_FORMATS, "mp4"),
        "scale": number(if !scale.is_finite() || scale <= 0. { 1. } else { scale.clamp(0.01, 16.) }),
        "fps": number(if fps.is_finite() && fps > 0. { fps.min(240.) } else { base_fps }),
        "range": pick("range", &["work", "all"], "work"),
        "quality": pick("quality", &["draft", "high", "max"], "high"),
        "mblur": flag("mblur", true),
        "audio": flag("audio", true),
        "alpha": flag("alpha", false),
    })
}
/// A usable work area (`[start, end]`, end after start), as main's planner asks.
pub(crate) fn work_span(work: &Value) -> Option<(f64, f64)> {
    let (a, b) = (work.get(0)?.as_f64()?, work.get(1)?.as_f64()?);
    (b > a).then_some((a, b))
}
/// main's `EXPORT_FORMAT_OPTIONS` values: a stored format is kept even where
/// this app has no encoder for it.
const V1_FORMATS: [&str; 8] = [
    "prores", "mp4", "webm", "rec", "png", "still", "web", "json",
];
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
const SIZE_PRESETS: [(f64, f64, &str); 6] = [
    (3840., 2160., "4K UHD · 3840×2160"),
    (1920., 1080., "HD 1080p · 1920×1080"),
    (1280., 720., "HD 720p · 1280×720"),
    (1080., 1920., "Vertical HD · 1080×1920"),
    (1080., 1350., "Portrait 4:5 · 1080×1350"),
    (1080., 1080., "Square · 1080×1080"),
];

impl Backend {
    /// Live subscription rows. These replace the catalogue's disabled ones by id.
    pub(crate) fn account_rows(&self) -> Vec<Value> {
        let Some(accounts) = self.native.agent_link.accounts() else {
            return vec![];
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
        let preset = SIZE_PRESETS
            .iter()
            .find(|(pw, ph, _)| *pw == w && *ph == h)
            .map(|(pw, ph, _)| format!("{pw}x{ph}"));
        let mut options: Vec<Value> = SIZE_PRESETS
            .iter()
            .map(|(pw, ph, label)| json!({"v":format!("{pw}x{ph}"),"label":label}))
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
        normalize_export_defaults(&stored, fps)
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
        let format = d["format"].as_str().unwrap_or("mp4");
        // Which settings a format reads (v1 `exportFieldSupport`); the rest stay
        // visible but dimmed.
        let video = matches!(format, "prores" | "mp4" | "webm");
        let reads = |field: &str| match field {
            "scale" | "fps" | "range" => format != "json",
            "quality" => matches!(format, "mp4" | "webm"),
            "mblur" | "audio" => video,
            "alpha" => format == "prores" || format == "png" || format == "still",
            _ => true,
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
                        let mut options: Vec<Value> = EXPORT_FORMATS
                            .iter()
                            .map(|(v, l)| json!({"v":v,"label":l}))
                            .collect();
                        // A v1 format this app can't export still shows as stored.
                        if !EXPORT_FORMATS.iter().any(|(v, _)| *v == format) {
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
                    Some("media.location" | "media.clear" | "media.reveal")
                ) =>
            {
                self.native
                    .platform
                    .push(("media.settings".into(), json!({"action":action["action"]})));
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
                        let checked = normalize_export_defaults(&defaults, fps);
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
                Ok(NativeCommand::State(pm_kernel::state::StateTransaction {
                    expected_revision: None,
                    ops: vec![pm_kernel::state::StateOp::Extension(entry)],
                }))
            }
            // Speech and archive actions require their actual services.
            _ => Err("Settings service is unavailable for this action".into()),
        }
    }
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
