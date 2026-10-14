//! `codex exec --json` events → progress copy and trace rows (port of
//! `codex/events.ts`). Structured agent output is restricted to known display
//! fields so edit or shell commands never leak into the progress stream.

use serde_json::Value;

use super::trace::{self, fragment_text, human_label, item_id, output_excerpt, progress_line, tool_detail};

const DISPLAY_FIELDS: [&str; 4] = ["message", "summary", "critique", "status"];
const TOOL_ITEM_TYPES: [&str; 8] = ["command_execution", "file_change", "web_search", "image_generation", "computer_use", "mcp_tool_call", "collab_tool_call", "context_compaction"];

fn started_activity(item_type: &str) -> Option<&'static str> {
    Some(match item_type {
        "command_execution" => "Working with project files and shell tools…",
        "web_search" => "Researching on the web…",
        "computer_use" => "Operating an application on this PC…",
        "image_generation" => "Generating a visual deliverable…",
        "file_change" => "Preparing project files…",
        _ => return None,
    })
}

fn completed_display_text(item: &Value, minimum_structured: usize) -> Option<String> {
    let text = item["text"].as_str()?;
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(structured)) => Some(
            DISPLAY_FIELDS
                .iter()
                .filter_map(|field| structured.get(*field).and_then(Value::as_str))
                .find(|value| value.chars().count() > minimum_structured)
                .unwrap_or_default()
                .to_owned(),
        ),
        Ok(_) => None,
        Err(_) => Some(text.to_owned()),
    }
}

fn basename(value: &Value) -> String {
    let Some(text) = value.as_str() else { return String::new() };
    let cleaned: String = text.chars().filter(|character| !character.is_control()).collect();
    cleaned.trim().split(['/', '\\']).filter(|part| !part.is_empty()).last().unwrap_or_default().to_owned()
}

pub fn changed_file_names(item: &Value) -> Vec<String> {
    let mut values: Vec<&Value> = Vec::new();
    for change in item["changes"].as_array().into_iter().flatten() {
        values.push(if change.is_string() { change } else { &change["path"] });
    }
    if values.is_empty() {
        values.extend(item["files"].as_array().into_iter().flatten());
    }
    if values.is_empty() {
        values.push(&item["path"]);
    }
    values.into_iter().map(basename).filter(|name| !name.is_empty()).collect()
}

fn tool_start(item: &Value) -> Option<Value> {
    let id = item["id"].as_str().filter(|id| !id.is_empty())?;
    let id = item_id(id);
    Some(match item["type"].as_str()? {
        "command_execution" => trace::tool_start(&id, "bash", "Run", &tool_detail("Bash", &serde_json::json!({ "command": item["command"] }), None)),
        "file_change" => trace::tool_start(&id, "edit", "Edit", &tool_detail("Edit", &serde_json::json!({ "file_path": changed_file_names(item).join(", ") }), None)),
        "web_search" => trace::tool_start(&id, "search", "Search", &tool_detail("WebSearch", &serde_json::json!({ "query": item["query"] }), None)),
        "image_generation" => trace::tool_start(&id, "image", "Image", ""),
        "computer_use" => trace::tool_start(&id, "computer", "Computer", ""),
        "context_compaction" => trace::tool_start(&id, "compact", "Compacting context", ""),
        "collab_tool_call" => trace::tool_start(&id, "agent", "Subagent", &tool_detail("Task", &serde_json::json!({ "description": item["prompt"] }), None)),
        "mcp_tool_call" => {
            let tool = item["tool"].as_str().filter(|tool| !tool.is_empty()).unwrap_or("mcp");
            let name = item["name"].as_str().filter(|name| !name.is_empty()).unwrap_or(tool);
            trace::tool_start(&id, tool, &human_label(name), &tool_detail(name, &serde_json::json!({ "tool": name }), None))
        }
        _ => return None,
    })
}

fn error_text(item: &Value) -> Value {
    match &item["error"] {
        Value::String(text) => Value::String(text.clone()),
        Value::Object(error) if error.get("message").is_some_and(Value::is_string) => error["message"].clone(),
        _ => item["status"].clone(),
    }
}

fn tool_output(item: &Value, is_error: bool) -> String {
    if is_error {
        return output_excerpt(&error_text(item));
    }
    match item["type"].as_str().unwrap_or_default() {
        "command_execution" => output_excerpt(if item["aggregated_output"].is_null() { &item["output"] } else { &item["aggregated_output"] }),
        "file_change" => {
            let files = changed_file_names(item);
            let count = item["changes"].as_array().map_or(files.len(), Vec::len);
            output_excerpt(&Value::String(if count > 3 { format!("{count} files changed") } else { files.join(", ") }))
        }
        "web_search" => output_excerpt(&item["query"]),
        "mcp_tool_call" => output_excerpt(if item["result"].is_null() { &item["output"] } else { &item["result"] }),
        "context_compaction" => "Earlier context summarized".into(),
        "collab_tool_call" => String::new(),
        _ => output_excerpt(&item["output"]),
    }
}

pub fn thread_id(event: &Value) -> Option<String> {
    (event["type"] == "thread.started").then(|| event["thread_id"].as_str().filter(|id| !id.is_empty()).map(str::to_owned)).flatten()
}

/// Renderer-safe progress copy for one event.
pub fn progress(event: &Value) -> Option<String> {
    // Stream retries ("Reconnecting... 2/5") arrive as top-level errors; they do not end the run.
    if event["type"] == "error" {
        return event["message"].as_str().map(progress_line).filter(|text| !text.is_empty());
    }
    let item = event.get("item").filter(|item| item.is_object())?;
    if event["type"] == "item.completed" && item["type"] == "error" {
        return item["message"].as_str().map(progress_line).filter(|text| !text.is_empty());
    }
    let item_type = item["type"].as_str()?;
    if event["type"] == "item.started" {
        if item_type == "mcp_tool_call" {
            let tool = item["tool"].as_str().or_else(|| item["name"].as_str()).unwrap_or("integration");
            return Some(format!("Using the installed {} integration…", trace::truncate(tool, 80)));
        }
        return started_activity(item_type).map(str::to_owned);
    }
    if event["type"] != "item.completed" || (item_type != "reasoning" && item_type != "agent_message") {
        return None;
    }
    let summary = progress_line(&completed_display_text(item, 8)?);
    (!summary.is_empty()).then_some(summary)
}

/// The bounded trace row for one event.
pub fn trace_step(event: &Value) -> Option<Value> {
    let item = event.get("item").filter(|item| item.is_object())?;
    let item_type = item["type"].as_str()?;
    if event["type"] == "item.started" {
        return tool_start(item);
    }
    if event["type"] != "item.completed" {
        return None;
    }
    if item_type == "reasoning" || item_type == "agent_message" {
        let text = fragment_text(&completed_display_text(item, 0)?);
        return (!text.is_empty()).then(|| trace::text_step(if item_type == "reasoning" { "thought" } else { "answer" }, &text));
    }
    let id = item["id"].as_str().filter(|id| !id.is_empty())?;
    if !TOOL_ITEM_TYPES.contains(&item_type) {
        return None;
    }
    let status = item["status"].as_str().unwrap_or_default().to_lowercase();
    let is_error = matches!(status.as_str(), "failed" | "error" | "declined");
    Some(trace::tool_end(&item_id(id), is_error, &tool_output(item, is_error)))
}

/// The actual failure from `codex exec --json` stdout, newest first.
pub fn error_from_stdout(stdout: &str) -> Option<String> {
    fn message(value: &Value, depth: usize) -> Option<String> {
        if depth > 8 {
            return None;
        }
        match value {
            Value::String(text) => {
                let text = text.trim();
                if text.is_empty() {
                    return None;
                }
                if text.starts_with('{') {
                    if let Ok(nested) = serde_json::from_str::<Value>(text) {
                        return message(&nested, depth + 1).or_else(|| Some(text.to_owned()));
                    }
                }
                Some(text.to_owned())
            }
            Value::Object(_) => message(&value["error"], depth + 1).or_else(|| message(&value["message"], depth + 1)).or_else(|| message(&value["detail"], depth + 1)),
            _ => None,
        }
    }
    let failure = regex::Regex::new(r"(?i)(?:^|[._-])(?:error|failed|failure)(?:$|[._-])").expect("static pattern");
    for line in stdout.lines().rev() {
        let Ok(event) = serde_json::from_str::<Value>(line) else { continue };
        let Some(kind) = event["type"].as_str() else { continue };
        if kind == "item.completed" && event["item"]["type"] == "error" {
            if let Some(text) = message(&event["item"], 0) {
                return Some(trace::truncate(&text, 4_000));
            }
        }
        if failure.is_match(kind) {
            if let Some(text) = message(&event, 0) {
                return Some(trace::truncate(&text, 4_000));
            }
        }
    }
    None
}

/// Whether Codex began the turn (a fresh retry is no longer safe).
pub fn turn_started(stdout: &str) -> bool {
    stdout.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()).any(|event| {
        let kind = event["type"].as_str().unwrap_or_default();
        kind == "turn.started" || (kind.starts_with("item.") && event["item"]["type"] != "error")
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn maps_items_to_progress_and_trace() {
        let started = json!({ "type": "item.started", "item": { "id": "i1", "type": "command_execution", "command": "ls -la" } });
        assert_eq!(progress(&started).unwrap(), "Working with project files and shell tools…");
        assert_eq!(trace_step(&started).unwrap(), json!({ "kind": "tool-start", "itemId": "i1", "toolName": "bash", "label": "Run", "detail": "ls -la" }));
        let done = json!({ "type": "item.completed", "item": { "id": "i1", "type": "command_execution", "aggregated_output": "a\nb", "status": "completed" } });
        assert_eq!(trace_step(&done).unwrap(), json!({ "kind": "tool-end", "itemId": "i1", "isError": false, "output": "a\nb" }));
        let mcp = json!({ "type": "item.started", "item": { "id": "m", "type": "mcp_tool_call", "server": "powermove", "tool": "get_project_state" } });
        assert_eq!(trace_step(&mcp).unwrap()["label"], "Get project state");
    }

    #[test]
    fn structured_answers_only_expose_display_fields() {
        let event = json!({ "type": "item.completed", "item": { "type": "agent_message", "text": "{\"summary\":\"Moved the title up\",\"commands\":[\"rm -rf\"]}" } });
        assert_eq!(trace_step(&event).unwrap(), json!({ "kind": "answer", "text": "Moved the title up" }));
        assert_eq!(progress(&event).unwrap(), "Moved the title up");
    }

    #[test]
    fn finds_errors_and_turn_starts() {
        let stdout = "{\"type\":\"thread.started\",\"thread_id\":\"t\"}\n{\"type\":\"turn.failed\",\"error\":{\"message\":\"{\\\"error\\\":{\\\"message\\\":\\\"Quota exceeded\\\"}}\"}}\n";
        assert_eq!(error_from_stdout(stdout).unwrap(), "Quota exceeded");
        assert!(!turn_started("{\"type\":\"thread.started\",\"thread_id\":\"t\"}"));
        assert!(turn_started("{\"type\":\"turn.started\"}"));
        assert_eq!(thread_id(&json!({ "type": "thread.started", "thread_id": "abc" })).unwrap(), "abc");
    }
}
