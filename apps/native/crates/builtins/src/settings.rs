//! Profile/project services used by the built-in Settings realm. Actor receipts
//! remain the only success boundary; unsupported native services refuse visibly.
use crate::{
    data::{Backend, wire},
    models::NativeCommand,
};
use pm_kernel::{
    Call, Request, WriteBody,
    commands::comps::{CompCommand, CompPatch},
};
use serde_json::{Value, json};

impl Backend {
    pub(crate) fn settings_rows(&mut self) -> Result<Vec<Value>, String> {
        let Some(p) = self.published() else {
            return Ok(vec![]);
        };
        let root = p.snapshot.root().ok_or("Project closed")?;
        let comp = pm_kernel::model::comp_node(root, p.snapshot.open_comp().as_deref())
            .ok_or("Composition closed")?;
        let fields = self.fields(&json!([
            "project.name",
            "project.fps",
            "project.duration",
            "project.background",
            "project.colorDepth"
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
        rows.push(json!({"id":"project.colorDepth","section":"project","group":"Composition",
            "label":"Color Depth","control":"choice","value":root.get("colorDepth")
                .map(|n| wire(&n.to_value())).unwrap_or(json!(8)),
            "options":[{"v":8,"label":"8-bit"},{"v":16,"label":"16-bit"},{"v":32,"label":"32-bit float"}],
            "detail":"Precision per color channel. Applies to every composition in this project.",
            "binding":fields["project.colorDepth"],"disabled":false}));
        let memory = self.fields(&json!(["memory.enabled"]))?;
        rows.push(json!({"id":"memory.enabled","section":"memory","group":"Project Memory",
            "label":"Use Project Memory","control":"switch", "disabled":false,
            "value":root.get("memoryEnabled").map(pm_model::Node::to_value) != Some(pm_model::Value::Bool(false)),
            "detail":"Agents read these notes before they edit, and add to them as they work. Turning it off keeps the notes but hides them from agents.",
            "binding":memory["memory.enabled"]}));
        Ok(rows)
    }
    pub(crate) fn settings_action(&mut self, args: &Value) -> Result<NativeCommand, String> {
        self.sync()?;
        if args["revision"].as_u64() != Some(self.native.settings_sequence) {
            return Err("Settings changed; retry from the latest publication".into());
        }
        let action = &args["action"];
        match action["type"].as_str() {
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
            // Enabling needs runtime activation as well as a registry flag.
            // Accounts, speech and archive actions require their actual services.
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
