//! Shared app/stdio admission for durable project notes. The JSON transport
//! projects only the note fields; unrelated authored UTF-16 remains opaque.
use crate::{Error, Result, memory};
use pm_kernel::{Ack, MemoryCall, Notice, Reply};
use pm_model::{JsString, Map, Node};
use serde_json::{Value, json};

pub enum Prepared {
    Read(pm_model::Value),
    Write(MemoryCall),
}

pub fn prepare(root: &Node, args: &Value) -> Result<Prepared> {
    if root.get("memoryEnabled").map(Node::to_value) == Some(pm_model::Value::Bool(false)) {
        return Err(Error::invalid(memory::DISABLED));
    }
    let raw = root.get("memory").map(Node::to_value);
    if args["operation"] == "list" {
        return Ok(Prepared::Read(list(raw.as_ref(), args)));
    }
    // The pinned validation body consumes only these fields. Its temporary
    // Unicode view must never replace authored notes or become a tool response.
    let validation: Vec<Value> = raw
        .as_ref()
        .and_then(pm_model::Value::as_array)
        .into_iter()
        .flatten()
        .map(|entry| {
            let mut item = serde_json::Map::new();
            for key in ["id", "kind", "text", "why", "resolved"] {
                if let Some(value) = entry.get(key) {
                    let value = match value {
                        pm_model::Value::String(text) => json!(text.to_string_lossy()),
                        _ => serde_json::from_str(&pm_model::json::to_canonical_string(value))
                            .unwrap_or(Value::Null),
                    };
                    item.insert(key.into(), value);
                }
            }
            Value::Object(item)
        })
        .collect();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(Error::kernel)?
        .as_millis() as f64;
    // Keep the pinned main validation/error precedence. The actor repeats model
    // validation against the exact observed notes and current access setting.
    memory::apply(&validation, args, now, "validation", "agent")?;
    let text = |key: &str| args.get(key).and_then(Value::as_str).map(JsString::from);
    let operation = match args["operation"].as_str() {
        Some("add") => pm_model::memory::Operation::Add {
            kind: args["kind"].as_str().unwrap_or("").into(),
            text: text("text").unwrap_or_default(),
            why: text("why"),
            resolved: args["resolved"] == true,
        },
        Some("update") => pm_model::memory::Operation::Update {
            id: text("id").unwrap_or_default(),
            kind: args.get("kind").and_then(Value::as_str).map(str::to_owned),
            text: text("text"),
            why: text("why"),
            resolved: args.get("resolved").and_then(Value::as_bool),
        },
        Some("remove") => pm_model::memory::Operation::Remove {
            id: text("id").unwrap_or_default(),
        },
        _ => {
            return Err(Error::invalid(
                "operation must be list, add, update or remove.",
            ));
        }
    };
    Ok(Prepared::Write(MemoryCall::Mutate {
        expected: root.get("memory").map(Node::to_value),
        operation,
        now,
        id: pm_kernel::new_id("mem").as_str().into(),
    }))
}

/// Main's `list` over the authored notes, newest first, without sanitizing them.
pub fn list(memory: Option<&pm_model::Value>, args: &Value) -> pm_model::Value {
    let all = memory
        .and_then(pm_model::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut selected: Vec<_> = all
        .iter()
        .filter(|e| {
            args.get("kind")
                .is_none_or(|k| k.is_null() || k == "" || e.get("kind") == Some(&crate::model(k)))
        })
        .cloned()
        .collect();
    let time = |e: &pm_model::Value| {
        e.get("updatedAt")
            .and_then(pm_model::Value::as_f64)
            .unwrap_or(0.0)
    };
    selected.sort_by(|a, b| time(b).total_cmp(&time(a)));
    let limit = args["limit"]
        .as_f64()
        .unwrap_or(100.0)
        .trunc()
        .clamp(1.0, 100.0) as usize;
    selected.truncate(limit);
    let mut result = Map::new();
    result.insert("memory", pm_model::Value::Array(selected));
    result.insert("total", pm_model::Value::Number(all.len() as f64));
    pm_model::Value::Object(result)
}

pub fn result(ack: Ack) -> Result<pm_model::Value> {
    if let Some(Notice::NotDurable { message }) = ack.notice {
        return Err(Error {
            code: "io_error".into(),
            message: format!(
                "Project notes remain applied but durability failed: {message}. Read the notes before retrying."
            ),
            backing: Some("pm-kernel".into()),
        });
    }
    match ack.reply {
        Reply::Memory { entry, total, .. } => {
            let mut result = Map::new();
            result.insert("ok", pm_model::Value::Bool(true));
            result.insert("entry", entry.unwrap_or(pm_model::Value::Null));
            result.insert("total", pm_model::Value::Number(total as f64));
            Ok(pm_model::Value::Object(result))
        }
        other => Err(Error::kernel(other)),
    }
}

/// Checked convenience projection for structured callers. Wire callers encode
/// the model directly into MCP text so JavaScript UTF-16 remains exact.
pub fn unicode(value: pm_model::Value) -> Result<Value> {
    serde_json::from_str(&pm_model::json::to_canonical_string(&value)).map_err(|_| {
        Error::unavailable(
            "Structured Unicode projection of project notes; use the MCP text response",
        )
    })
}
