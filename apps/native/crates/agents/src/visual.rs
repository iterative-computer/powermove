//! Exact visual-reply content with an opaque-origin display contract. These
//! policies must be enforced by the macOS WebKit host before loading content.
use crate::{error, trace};
use serde_json::{Value, json};
use std::io;
pub const MAX_BYTES: usize = 384 * 1024;
pub const MAX_PER_RUN: usize = 4;
pub const CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; font-src 'none'; connect-src 'none'; media-src 'none'; worker-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
pub fn input(value: &Value) -> io::Result<Value> {
    let object = value
        .as_object()
        .ok_or_else(|| error("Visual reply expects title, html and optional height."))?;
    if object
        .keys()
        .any(|k| !matches!(k.as_str(), "title" | "html" | "height"))
        || !value["title"]
            .as_str()
            .is_some_and(|v| !v.trim().is_empty() && v.encode_utf16().count() <= 120)
        || !value["html"]
            .as_str()
            .is_some_and(|v| !v.trim().is_empty() && v.len() <= MAX_BYTES)
        || value.get("height").is_some_and(|v| {
            !v.as_f64()
                .is_some_and(|v| v.fract() == 0. && (160. ..=900.).contains(&v))
        })
    {
        return Err(error(
            "Visual reply requires a title (1–120 characters), HTML up to 384 KiB and height from 160 to 900.",
        ));
    }
    Ok(
        json!({"title":value["title"].as_str().unwrap().trim(),"html":value["html"],"height":value.get("height").cloned().unwrap_or_else(||json!(360))}),
    )
}
pub fn read(value: &Value) -> Option<Value> {
    let id = value["id"].as_str()?;
    let suffix = id.strip_prefix("visual-")?;
    if value["version"] != 1
        || !crate::valid_id(suffix, 120)
        || value["runId"].as_str()?.encode_utf16().count() > 160
    {
        return None;
    }
    input(&json!({"title":value["title"],"html":value["html"],"height":value["height"]})).ok()?;
    Some(value.clone())
}
pub fn event(
    source: u64,
    expected_source: Option<u64>,
    origin: &str,
    token: &str,
    data: &Value,
) -> Option<Value> {
    if expected_source != Some(source)
        || origin != "null"
        || data["channel"] != "powermove-visual"
        || data["token"] != token
    {
        return None;
    }
    let object = data.as_object()?;
    let kind = data["type"].as_str()?;
    if object.keys().any(|k| {
        !(matches!(k.as_str(), "channel" | "token" | "type")
            || kind == "size" && k == "height"
            || kind == "error" && k == "message")
    }) {
        return None;
    }
    match kind {
        "ready" => Some(json!({"type":"ready"})),
        "size" => {
            let height = data["height"].as_f64()?;
            if !(0. ..=100000.).contains(&height) {
                return None;
            }
            Some(json!({"type":"size","height":height.ceil().clamp(160.,900.) as u64}))
        }
        "error" => {
            let message = data["message"].as_str()?;
            if message.encode_utf16().count() > 300 {
                return None;
            }
            Some(json!({"type":"error","message":message}))
        }
        _ => None,
    }
}
/// Publication stays in the durable ordinary thread, including raw HTML. The
/// native sandbox may execute that HTML but receives no PM/tool/file bridge.
pub fn publish(thread: &mut Value, run: &str, id: &str, value: &Value) -> io::Result<Value> {
    let normalized = input(value)?;
    if !id.starts_with("visual-")
        || !crate::valid_id(&id[7..], 120)
        || run.encode_utf16().count() > 160
    {
        return Err(error("Invalid visual reply identity"));
    }
    if thread.get("visualReplies").is_some_and(|v| !v.is_array()) {
        return Err(error("Unknown visual reply archive; preserved unchanged"));
    }
    let mut replies = thread["visualReplies"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut reply = normalized;
    reply["version"] = json!(1);
    reply["id"] = json!(id);
    reply["runId"] = json!(run);
    if let Some(existing) = replies.iter().find(|v| v["id"] == id) {
        if *existing != reply {
            return Err(error("Visual reply identity conflict"));
        }
        return Ok(reply);
    }
    if replies.iter().filter(|v| v["runId"] == run).count() >= MAX_PER_RUN {
        return Err(error("A run can publish up to four visual replies."));
    }
    replies.push(reply.clone());
    thread["visualReplies"] = json!(replies);
    Ok(reply)
}
pub fn inert_error(message: &str) -> String {
    trace::truncate(message, 300)
}
