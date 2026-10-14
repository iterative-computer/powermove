//! Window, shell, dialog and status channels that need no service of their own.

use std::path::Path;

use anyhow::{Result, bail};
use rfd::{MessageButtons, MessageDialogResult, MessageLevel};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::ids::{LOG_CHARS, is_project_id};
use crate::router::Router;

pub fn register(router: &mut Router) {
    router.json("web:hello", |call| async move {
        let config = &call.host.config;
        Ok(json!({
            "node": "",
            "version": config.version,
            "platform": "win32",
            "exportsDir": "",
            // Tells web-bridge.ts it runs inside the Windows app, not a browser tab.
            "desktop": true,
        }))
    });
    router.constant("app:ping", json!("pong"));

    /* windows: one WebView per editor window, each holding one project */
    router.json("window:initial-project", |call| async move {
        let own = call.client.project.lock().clone();
        let taken: Vec<String> = call.host.clients().iter().filter(|other| other.id != call.client.id).filter_map(|other| other.project.lock().clone()).collect();
        Ok(json!({ "projectId": own, "taken": taken }))
    });
    router.json("window:claim-project", |call| async move {
        let project = project_id(call.arg(0))?;
        *call.client.project.lock() = project;
        Ok(json!({ "claimed": true, "focused": false }))
    });
    router.json("window:release-project", |call| async move {
        let project = project_id(call.arg(0))?;
        let mut own = call.client.project.lock();
        if own.is_some() && *own == project {
            *own = None;
        }
        Ok(json!(true))
    });
    router.json("window:open-project", |call| async move {
        let Some(project) = project_id(call.arg(0))? else {
            return Ok(json!({ "opened": false, "focused": false, "error": "A project is required" }));
        };
        if call.host.clients().iter().any(|other| other.id != call.client.id && other.project.lock().as_deref() == Some(project.as_str())) {
            return Ok(json!({ "opened": false, "focused": true }));
        }
        call.client.emit("web:open-window", json!(project));
        Ok(json!({ "opened": true, "focused": false }))
    });
    router.json("window:new", |call| async move {
        call.client.emit("web:open-window", Value::Null);
        Ok(Value::Null)
    });
    router.json("window:close", |call| async move {
        call.client.emit("web:close-window", Value::Null);
        Ok(Value::Null)
    });

    router.json("log", |call| async move {
        #[derive(Deserialize)]
        struct Line {
            level: String,
            text: String,
        }
        let line: Line = call.parse()?;
        let text: String = line.text.chars().take(LOG_CHARS).map(|c| if c.is_control() { ' ' } else { c }).collect();
        match line.level.as_str() {
            "error" | "uncaught" => tracing::error!(target: "renderer", "{text}"),
            "warn" => tracing::warn!(target: "renderer", "{text}"),
            _ => tracing::info!(target: "renderer", "{text}"),
        }
        Ok(Value::Null)
    });

    /* shell */
    for channel in ["shell:open-external", "shell:open-extension-url"] {
        let https_only = channel == "shell:open-extension-url";
        router.json(channel, move |call| async move {
            let url = call.arg(0).as_str().unwrap_or_default().to_owned();
            let allowed = url.starts_with("https://") || (!https_only && url.starts_with("http://"));
            if !allowed || url.len() > 8_192 || url.chars().any(char::is_whitespace) {
                bail!("expected an http(s) URL");
            }
            open_external(&url)?;
            Ok(Value::Null)
        });
    }
    router.json("media:reveal-source", |call| async move {
        let path = call.arg(0).as_str().unwrap_or_default();
        if !Path::new(path).is_absolute() {
            bail!("expected an absolute file path");
        }
        reveal(Path::new(path))?;
        Ok(Value::Null)
    });

    /* native dialogs */
    router.json("dialog:confirm", |call| async move {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Confirm {
            message: String,
            detail: Option<String>,
            confirm_label: Option<String>,
            destructive: Option<bool>,
        }
        let request: Confirm = call.parse()?;
        let confirm = request.confirm_label.unwrap_or_else(|| "OK".into());
        let result = crate::dialog::message()
            .set_level(if request.destructive == Some(true) { MessageLevel::Warning } else { MessageLevel::Info })
            .set_title(&request.message)
            .set_description(request.detail.unwrap_or_default())
            .set_buttons(MessageButtons::OkCancelCustom(confirm.clone(), "Cancel".into()))
            .show()
            .await;
        Ok(json!(matches!(result, MessageDialogResult::Ok) || matches!(&result, MessageDialogResult::Custom(label) if *label == confirm)))
    });
    router.json("project:confirm-close", |call| async move {
        let name = call.arg(0).as_str().unwrap_or("this project").chars().take(200).collect::<String>();
        let result = crate::dialog::message()
            .set_level(MessageLevel::Warning)
            .set_title(format!("Do you want to save the changes to {name}?"))
            .set_description("Your changes will be lost if you don't save them.")
            .set_buttons(MessageButtons::YesNoCancelCustom("Save".into(), "Don't Save".into(), "Cancel".into()))
            .show()
            .await;
        let decision = match result {
            MessageDialogResult::Custom(label) if label == "Save" => "save",
            MessageDialogResult::Custom(label) if label == "Don't Save" => "discard",
            MessageDialogResult::Yes => "save",
            MessageDialogResult::No => "discard",
            _ => "cancel",
        };
        Ok(json!(decision))
    });

    /* features the Windows host does not offer yet (agent channels live in agents/) */
    router.constant("web:runs-list", json!([]));
    for channel in ["web:update-check", "web:update-install"] {
        router.constant(channel, Value::Null);
    }
    router.json("web:update-status", |call| async move {
        Ok(json!({ "status": "idle", "current": call.host.config.version, "version": null }))
    });
}

/// `{ projectId }` or null → a validated id or None.
fn project_id(payload: &Value) -> Result<Option<String>> {
    match payload.get("projectId") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(id)) if id.is_empty() => Ok(None),
        Some(Value::String(id)) if is_project_id(id) => Ok(Some(id.clone())),
        _ => bail!("invalid projectId"),
    }
}

/// Open a URL in the default browser.
pub fn open_external(url: &str) -> Result<()> {
    std::process::Command::new("explorer").arg(url).spawn()?;
    Ok(())
}

/// Show a file selected in File Explorer.
pub fn reveal(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn()?;
    }
    #[cfg(not(windows))]
    std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn()?;
    Ok(())
}
