//! Bounded, renderer-safe activity rows (port of `agent-tools/trace-format.ts`).
//!
//! Every provider turns its own events into the `CodexTraceEvent` shapes in
//! `shared/ipc.ts`; these helpers keep labels, details and excerpts within the
//! renderer's limits and strip control characters.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

/// `LIMITS` from `shared/ipc.ts`.
pub const PROGRESS_CHARS: usize = 320;
pub const TRACE_CHARS: usize = 2_000;
pub const TOOL_DETAIL_CHARS: usize = 200;
pub const TOOL_OUTPUT_CHARS: usize = 600;
pub const ITEM_ID_CHARS: usize = 120;

const DETAIL_KEYS: [&str; 6] = ["command", "file_path", "path", "query", "url", "description"];
const FILE_TOOLS: [&str; 5] = ["read", "edit", "write", "multiedit", "notebookedit"];

/// First `limit` UTF-16 units, like JS `slice` without splitting a surrogate pair.
pub fn truncate(value: &str, limit: usize) -> String {
    let mut units = 0;
    let mut out = String::new();
    for character in value.chars() {
        units += character.len_utf16();
        if units > limit {
            break;
        }
        out.push(character);
    }
    out
}

fn is_c0_or_c1(character: char, keep: &[char]) -> bool {
    !keep.contains(&character) && (character <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(&character))
}

fn tool_label(name: &str) -> Option<&'static str> {
    Some(match name {
        "read" => "Read",
        "edit" | "multiedit" => "Edit",
        "write" => "Write",
        "bash" => "Run",
        "grep" | "glob" => "Search",
        "ls" => "List",
        "websearch" => "Search web",
        "webfetch" => "Fetch",
        "agent" | "task" => "Agent",
        "todowrite" => "Plan",
        "notebookedit" => "Edit notebook",
        _ => return None,
    })
}

fn humanise(value: &str) -> String {
    static LOWER_UPPER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([a-z\d])([A-Z])").unwrap());
    static ACRONYM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([A-Z]+)([A-Z][a-z])").unwrap());
    static SEPARATORS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[_-]+").unwrap());
    static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
    let words = LOWER_UPPER.replace_all(value, "$1 $2");
    let words = ACRONYM.replace_all(&words, "$1 $2");
    let words = SEPARATORS.replace_all(&words, " ");
    let words = SPACES.replace_all(&words, " ").trim().to_lowercase();
    let mut characters = words.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => "Tool".into(),
    }
}

/// The short action label rendered before a tool's primary detail.
pub fn human_label(name: &str) -> String {
    let clean: String = name.chars().filter(|character| !is_c0_or_c1(*character, &[])).collect();
    let clean = clean.trim();
    if clean.is_empty() {
        return "Tool".into();
    }
    let label = match tool_label(&clean.to_lowercase()) {
        Some(label) => label.to_owned(),
        None => {
            let mcp = clean.strip_prefix("mcp__").or_else(|| clean.strip_prefix("MCP__")).and_then(|rest| rest.split_once("__")).map(|(_, tool)| tool).filter(|tool| !tool.is_empty());
            humanise(mcp.unwrap_or(clean))
        }
    };
    truncate(&label, 40)
}

fn project_path(value: &str, project_cwd: Option<&Path>) -> String {
    let (Some(cwd), path) = (project_cwd, Path::new(value)) else { return value.to_owned() };
    if !path.is_absolute() || !cwd.is_absolute() {
        return value.to_owned();
    }
    match path.strip_prefix(cwd) {
        Ok(relative) if relative.as_os_str().is_empty() => ".".into(),
        Ok(relative) => relative.display().to_string(),
        Err(_) => value.to_owned(),
    }
}

fn string_field<'a>(input: &'a Map<String, Value>, key: &str) -> &'a str {
    input.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn one_line(value: &str) -> String {
    let normalized = value.replace("\r\n", "\n").replace('\r', "\n");
    let cleaned: String = normalized.chars().filter(|character| !is_c0_or_c1(*character, &['\t', '\n', '\r'])).collect();
    truncate(&cleaned.replace('\n', " ⏎ "), TOOL_DETAIL_CHARS)
}

/// The most useful bounded argument for a tool activity row.
pub fn tool_detail(name: &str, input: &Value, project_cwd: Option<&Path>) -> String {
    let Some(input) = input.as_object() else { return String::new() };
    let key = name.to_lowercase();
    let mut detail = match key.as_str() {
        "bash" => string_field(input, "command").to_owned(),
        _ if FILE_TOOLS.contains(&key.as_str()) => project_path(string_field(input, "file_path"), project_cwd),
        "grep" => {
            let pattern = string_field(input, "pattern");
            let search = string_field(input, "path");
            if !pattern.is_empty() && !search.is_empty() { format!("{pattern} in {}", project_path(search, project_cwd)) } else { pattern.to_owned() }
        }
        "glob" => string_field(input, "pattern").to_owned(),
        "websearch" => string_field(input, "query").to_owned(),
        "webfetch" => string_field(input, "url").to_owned(),
        "agent" | "task" => string_field(input, "description").to_owned(),
        _ => String::new(),
    };
    if detail.is_empty() {
        detail = DETAIL_KEYS.iter().map(|key| string_field(input, key)).find(|value| !value.is_empty()).unwrap_or_default().to_owned();
    }
    if detail.is_empty() {
        detail = input.values().filter_map(Value::as_str).find(|value| !value.is_empty()).unwrap_or_default().to_owned();
    }
    one_line(&detail)
}

/// One append-only text delta, sanitized without altering meaningful whitespace.
pub fn fragment_text(value: &str) -> String {
    let cleaned: String = value.chars().filter(|character| !is_c0_or_c1(*character, &['\t', '\n', '\r'])).collect();
    truncate(&cleaned, TRACE_CHARS)
}

fn output_text(value: &Value, depth: usize) -> String {
    if depth > 32 {
        return String::new();
    }
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Array(items) => items.iter().map(|item| output_text(item, depth + 1)).filter(|text| !text.is_empty()).collect::<Vec<_>>().join("\n"),
        Value::Object(map) => {
            for key in ["error", "aggregated_output", "aggregatedOutput", "output", "content", "text", "message", "status"] {
                if let Some(item) = map.get(key) {
                    let text = output_text(item, depth + 1);
                    if !text.is_empty() {
                        return text;
                    }
                }
            }
            String::new()
        }
        Value::Null => String::new(),
    }
}

/// A compact tool-result excerpt: at most eight lines and 600 characters.
pub fn output_excerpt(value: &Value) -> String {
    let text = output_text(value, 0).replace("\r\n", "\n").replace('\r', "\n").replace('\t', " ");
    let text: String = text.chars().filter(|character| !is_c0_or_c1(*character, &['\n'])).collect();
    truncate(&text.split('\n').take(8).collect::<Vec<_>>().join("\n"), TOOL_OUTPUT_CHARS)
}

/// Collapse whitespace into one line for progress copy.
pub fn progress_line(value: &str) -> String {
    static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
    truncate(SPACES.replace_all(value, " ").trim(), PROGRESS_CHARS)
}

/// Strip control characters (except newline and tab) and trim, like the
/// question `clip` helpers in the runners.
pub fn clip(value: Option<&Value>, limit: usize) -> String {
    let Some(text) = value.and_then(Value::as_str) else { return String::new() };
    let cleaned: String = text.chars().filter(|character| !(*character <= '\u{1f}' && *character != '\t' && *character != '\n') && *character != '\u{7f}').collect();
    truncate(cleaned.trim(), limit)
}

pub fn item_id(value: &str) -> String {
    truncate(value, ITEM_ID_CHARS)
}

/* Constructors for the trace shapes in `CodexTraceEvent`. */

pub fn tool_start(item_id: &str, tool_name: &str, label: &str, detail: &str) -> Value {
    let mut step = json!({ "kind": "tool-start", "itemId": item_id, "toolName": truncate(tool_name, ITEM_ID_CHARS), "label": label });
    if !detail.is_empty() {
        step["detail"] = json!(detail);
    }
    step
}

pub fn tool_end(item_id: &str, is_error: bool, output: &str) -> Value {
    let mut step = json!({ "kind": "tool-end", "itemId": item_id, "isError": is_error });
    if !output.is_empty() {
        step["output"] = json!(output);
    }
    step
}

pub fn text_step(kind: &str, text: &str) -> Value {
    json!({ "kind": kind, "text": text })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_tools() {
        assert_eq!(human_label("Bash"), "Run");
        assert_eq!(human_label("mcp__powermove__get_project_state"), "Get project state");
        assert_eq!(human_label("renderFrames"), "Render frames");
        assert_eq!(human_label(""), "Tool");
    }

    #[test]
    fn details_and_excerpts_are_bounded() {
        let detail = tool_detail("Bash", &json!({ "command": "echo a\nb" }), None);
        assert_eq!(detail, "echo a ⏎ b");
        let long = "x".repeat(900);
        assert_eq!(tool_detail("Bash", &json!({ "command": long }), None).len(), TOOL_DETAIL_CHARS);
        let excerpt = output_excerpt(&json!({ "content": [{ "text": "1\n2\n3\n4\n5\n6\n7\n8\n9" }] }));
        assert_eq!(excerpt, "1\n2\n3\n4\n5\n6\n7\n8");
        assert_eq!(fragment_text("a\u{0}b\n"), "ab\n");
    }

    #[test]
    fn relative_project_paths() {
        let cwd = Path::new(if cfg!(windows) { r"C:\work" } else { "/work" });
        let file = if cfg!(windows) { r"C:\work\src\a.ts" } else { "/work/src/a.ts" };
        let detail = tool_detail("Read", &json!({ "file_path": file }), Some(cwd));
        assert!(detail.ends_with("a.ts") && !detail.contains("work"));
    }
}
