//! Frozen, bounded activity presentation from agent-tools/trace-format.ts.
use crate::text;
use serde_json::{Value, json};
pub fn truncate(s: &str, limit: usize) -> String {
    let mut n = 0;
    s.chars()
        .take_while(|c| {
            n += c.len_utf16();
            n <= limit
        })
        .collect()
}
fn clean(s: &str) -> String {
    s.chars()
        .filter(
            |c| !matches!(*c,'\0'..='\u{8}'|'\u{b}'|'\u{c}'|'\u{e}'..='\u{1f}'|'\u{7f}'..='\u{9f}'),
        )
        .collect()
}
pub fn fragment(v: &Value) -> String {
    truncate(&clean(v.as_str().unwrap_or("")), 2000)
}
pub fn label(name: &str) -> String {
    let clean = name.chars().filter(|c| !c.is_control()).collect::<String>();
    let clean = clean.trim();
    let key = clean.to_lowercase();
    let known = match key.as_str() {
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
        _ => "",
    };
    if !known.is_empty() {
        return known.into();
    }
    let name = if key.starts_with("mcp__") {
        clean.splitn(3, "__").nth(2).unwrap_or(clean)
    } else {
        clean
    };
    let chars = name.chars().collect::<Vec<_>>();
    let mut words = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' || c == '-' {
            words.push(' ');
            continue;
        }
        if i > 0
            && c.is_uppercase()
            && (chars[i - 1].is_lowercase()
                || chars[i - 1].is_ascii_digit()
                || chars[i - 1].is_uppercase()
                    && chars.get(i + 1).is_some_and(|c| c.is_lowercase()))
        {
            words.push(' ');
        }
        words.extend(c.to_lowercase());
    }
    let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = words.chars();
    let result = if let Some(c) = chars.next() {
        c.to_uppercase().collect::<String>() + chars.as_str()
    } else {
        "Tool".into()
    };
    truncate(&result, 40)
}
pub fn detail(name: &str, input: &Value) -> String {
    if !input.is_object() {
        return String::new();
    }
    let n = name.to_lowercase();
    let d = match n.as_str() {
        "bash" => text(input, "command"),
        "read" | "edit" | "write" | "multiedit" | "notebookedit" => text(input, "file_path"),
        "grep" => {
            let p = text(input, "pattern");
            let path = text(input, "path");
            if !p.is_empty() && !path.is_empty() {
                format!("{p} in {path}")
            } else {
                p
            }
        }
        "glob" => text(input, "pattern"),
        "websearch" => text(input, "query"),
        "webfetch" => text(input, "url"),
        "agent" | "task" => text(input, "description"),
        _ => String::new(),
    };
    let d = if !d.is_empty() {
        d
    } else {
        [
            "command",
            "file_path",
            "path",
            "query",
            "url",
            "description",
        ]
        .iter()
        .find_map(|k| input[k].as_str().filter(|s| !s.is_empty()))
        .or_else(|| {
            input
                .as_object()
                .unwrap()
                .values()
                .find_map(|v| v.as_str().filter(|s| !s.is_empty()))
        })
        .unwrap_or("")
        .into()
    };
    truncate(
        &clean(&d.replace("\r\n", "\n").replace('\r', "\n")).replace('\n', " ⏎ "),
        200,
    )
}
fn output_text(v: &Value) -> String {
    match v {
        Value::String(v) => v.clone(),
        Value::Bool(_) | Value::Number(_) => v.to_string(),
        Value::Array(a) => a
            .iter()
            .map(output_text)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(_) => [
            "error",
            "aggregated_output",
            "aggregatedOutput",
            "output",
            "content",
            "text",
            "message",
            "status",
        ]
        .iter()
        .map(|k| output_text(&v[k]))
        .find(|s| !s.is_empty())
        .unwrap_or_default(),
        _ => String::new(),
    }
}
pub fn excerpt(v: &Value) -> String {
    let raw = output_text(v)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', " ");
    truncate(
        &clean(&raw)
            .split('\n')
            .take(8)
            .collect::<Vec<_>>()
            .join("\n"),
        600,
    )
}
pub fn subject(tool: &str, input: &Value) -> Option<Value> {
    let tool = tool.trim().to_lowercase();
    let tool = tool.split("__").last().unwrap_or(&tool);
    if ![
        "probe_media",
        "sample_media_frames",
        "media_contact_sheet",
        "media_waveform",
        "transcribe_media",
        "generate_captions",
        "export_captions",
    ]
    .contains(&tool)
        || !input.is_object()
    {
        return None;
    }
    let id = |v: &Value| {
        v.as_str()
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 120
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
            })
            .map(str::to_string)
    };
    let mut result = json!({});
    if tool == "generate_captions" {
        let ids = input["layerIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(id)
            .collect::<Vec<_>>();
        if let Some(first) = ids.first() {
            result["layerId"] = json!(first);
            result["clips"] = json!(ids.len().min(999));
        }
        return Some(result);
    }
    if let Some(v) = id(&input["layerId"]) {
        result["layerId"] = json!(v);
    }
    if tool == "export_captions" {
        result["format"] = json!(if input["format"] == "vtt" {
            "vtt"
        } else {
            "srt"
        });
        return Some(result);
    }
    if let Some(v) = id(&input["assetId"]) {
        result["assetId"] = json!(v);
    }
    if let Some(times) = input["times"].as_array().filter(|a| !a.is_empty()) {
        result["frames"] = json!(times.len().min(999));
    } else if let Some(count) = input["count"].as_f64().filter(|c| *c > 0.0) {
        result["frames"] = json!(count.trunc().min(999.0) as u64);
    }
    if input["auto"] == true {
        result["auto"] = json!(true);
    }
    for k in ["start", "end"] {
        if input[k].is_number() {
            result[k] = input[k].clone();
        }
    }
    if matches!(input["target"].as_str(), Some("source" | "composition")) {
        result["target"] = input["target"].clone();
    }
    Some(result)
}
