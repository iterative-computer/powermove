//! Project-memory model from main c7f46a056 `core/memory.ts`. Operations return
//! a new project without touching authored `revision`, `edits` or Undo. The
//! kernel host schedules autosave with its independent recovery sequence.
//! Loading never sanitizes data: unknown/invalid notes remain in the project;
//! removed/evicted notes are archived, rather than discarded (N4).
use crate::{JsString, Map, Value};

pub const KINDS: [&str; 6] = [
    "goal",
    "constraint",
    "decision",
    "rejected",
    "feedback",
    "open",
];
pub const MAX_ENTRIES: usize = 100;
#[derive(Clone, Debug)]
pub enum Operation {
    Add {
        kind: String,
        text: JsString,
        why: Option<JsString>,
        resolved: bool,
    },
    Update {
        id: JsString,
        kind: Option<String>,
        text: Option<JsString>,
        why: Option<JsString>,
        resolved: Option<bool>,
    },
    Remove {
        id: JsString,
    },
}
pub struct Context {
    pub now: f64,
    pub id: JsString,
    pub author: &'static str,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Index {
    pub total: usize,
    pub goal: Option<Value>,
    pub constraints: Vec<Value>,
    pub open: Vec<Value>,
    pub recent: Vec<Value>,
}
pub fn enabled(project: &Value) -> bool {
    project.get("memoryEnabled") != Some(&Value::Bool(false))
}
fn kind(entry: &Value) -> Option<&str> {
    entry.get("kind")?.as_str()
}
fn standing(entry: &Value) -> bool {
    matches!(kind(entry), Some("goal" | "constraint"))
}
fn valid(entry: &Value) -> bool {
    entry
        .get("id")
        .and_then(Value::as_js_string)
        .is_some_and(|s| !s.as_bytes().is_empty())
        && kind(entry).is_some_and(|k| KINDS.contains(&k))
        && entry
            .get("text")
            .and_then(Value::as_js_string)
            .is_some_and(|s| !s.as_bytes().is_empty())
}
fn timestamp(entry: &Value) -> f64 {
    entry
        .get("updatedAt")
        .and_then(Value::as_f64)
        .filter(|t| t.is_finite())
        .unwrap_or(0.0)
}
fn entries(project: &Value) -> Vec<Value> {
    project
        .get("memory")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}
pub fn list(project: &Value, filter: Option<&str>, limit: usize) -> Vec<Value> {
    let mut all: Vec<_> = entries(project)
        .into_iter()
        .filter(valid)
        .filter(|e| filter.is_none_or(|k| kind(e) == Some(k)))
        .collect();
    all.sort_by(|a, b| timestamp(b).total_cmp(&timestamp(a)));
    all.truncate(limit.clamp(1, MAX_ENTRIES));
    all
}
pub fn index(project: &Value, recent: usize) -> Index {
    let all: Vec<_> = entries(project).into_iter().filter(valid).collect();
    let mut history: Vec<_> = all
        .iter()
        .filter(|e| !standing(e) && kind(e) != Some("open"))
        .cloned()
        .collect();
    history.sort_by(|a, b| timestamp(b).total_cmp(&timestamp(a)));
    history.truncate(recent);
    Index {
        total: all.len(),
        goal: all.iter().find(|e| kind(e) == Some("goal")).cloned(),
        constraints: all
            .iter()
            .filter(|e| kind(e) == Some("constraint"))
            .cloned()
            .collect(),
        open: all
            .iter()
            .filter(|e| kind(e) == Some("open") && e.get("resolved") != Some(&Value::Bool(true)))
            .cloned()
            .collect(),
        recent: history,
    }
}
fn clip(text: &JsString) -> JsString {
    // JS trim and slice count UTF-16 units; preserve lone surrogates.
    let mut units: Vec<_> = text.utf16_units().collect();
    let whitespace = |u: u16| {
        char::from_u32(u as u32).is_some_and(|c| crate::json::js_trim(&c.to_string()).is_empty())
    };
    let start = units
        .iter()
        .position(|u| !whitespace(*u))
        .unwrap_or(units.len());
    let end = units
        .iter()
        .rposition(|u| !whitespace(*u))
        .map(|i| i + 1)
        .unwrap_or(start);
    units = units[start..end].iter().take(600).copied().collect();
    JsString::from_utf16_units(&units)
}
fn archive(project: &mut Map, item: Value) {
    let key = "nativeMemoryArchive";
    let mut all = match project.remove(key) {
        Some(Value::Array(items)) => items,
        Some(value) => vec![value],
        None => Vec::new(),
    };
    all.push(item);
    project.insert(key, Value::Array(all));
}

/// Pure operation for the single writer. Callers publish its resulting leaf
/// patches through the kernel journal, without a history entry; no JS required.
pub fn apply(
    project: &Value,
    operation: &Operation,
    context: &Context,
) -> Result<(Value, Option<Value>), &'static str> {
    if !enabled(project) {
        return Err("project memory is disabled");
    }
    if !context.now.is_finite() || !["user", "agent"].contains(&context.author) {
        return Err("invalid memory context");
    }
    let mut out = project
        .as_object()
        .cloned()
        .ok_or("project is not an object")?;
    let mut all = entries(project);
    if let Some(raw) = project.get("memory").filter(|v| v.as_array().is_none()) {
        archive(&mut out, raw.clone());
    }
    let result = match operation {
        Operation::Remove { id } => {
            let at = all
                .iter()
                .position(|e| e.get("id").and_then(Value::as_js_string) == Some(id))
                .ok_or("memory entry is missing")?;
            archive(&mut out, all.remove(at));
            None
        }
        Operation::Add {
            kind: k,
            text,
            why,
            resolved,
        } => {
            if !KINDS.contains(&k.as_str()) {
                return Err("invalid memory kind");
            }
            let text = clip(text);
            if text.as_bytes().is_empty() {
                return Err("memory text cannot be empty");
            }
            if k == "goal"
                && let Some(at) = all.iter().position(|e| kind(e) == Some("goal"))
            {
                // A second goal edits the first, retaining identity/author and
                // createdAt. Main deliberately ignores add.resolved here.
                let mut current = all[at].as_object().cloned().ok_or("invalid memory entry")?;
                current.insert("text", Value::String(text));
                if let Some(why) = why {
                    set_why(&mut current, why);
                }
                current.insert("updatedAt", Value::Number(context.now));
                let value = Value::Object(current);
                all[at] = value.clone();
                Some(value)
            } else {
                if context.id.as_bytes().is_empty()
                    || all
                        .iter()
                        .any(|e| e.get("id").and_then(Value::as_js_string) == Some(&context.id))
                {
                    return Err("memory id is not unique");
                }
                let mut entry = Map::new();
                entry.insert("id", Value::String(context.id.clone()));
                entry.insert("kind", Value::from(k.as_str()));
                entry.insert("text", Value::String(text));
                entry.insert("author", Value::from(context.author));
                entry.insert("createdAt", Value::Number(context.now));
                entry.insert("updatedAt", Value::Number(context.now));
                if let Some(why) = why {
                    set_why(&mut entry, why);
                }
                if *resolved {
                    entry.insert("resolved", Value::Bool(true));
                }
                let value = Value::Object(entry);
                all.push(value.clone());
                Some(value)
            }
        }
        Operation::Update {
            id,
            kind: k,
            text,
            why,
            resolved,
        } => {
            if k.as_ref().is_some_and(|k| !KINDS.contains(&k.as_str())) {
                return Err("invalid memory kind");
            }
            let at = all
                .iter()
                .position(|e| e.get("id").and_then(Value::as_js_string) == Some(id))
                .ok_or("memory entry is missing")?;
            if k.as_deref() == Some("goal")
                && kind(&all[at]) != Some("goal")
                && all.iter().any(|e| kind(e) == Some("goal"))
            {
                return Err("project already has a goal");
            }
            let mut entry = all[at].as_object().cloned().ok_or("invalid memory entry")?;
            if let Some(k) = k {
                entry.insert("kind", Value::from(k.as_str()));
            }
            if let Some(text) = text {
                let text = clip(text);
                if text.as_bytes().is_empty() {
                    return Err("memory text cannot be empty");
                }
                entry.insert("text", Value::String(text));
            }
            if let Some(why) = why {
                set_why(&mut entry, why);
            }
            if let Some(resolved) = resolved {
                if *resolved {
                    entry.insert("resolved", Value::Bool(true));
                } else {
                    entry.remove("resolved");
                }
            }
            entry.insert("updatedAt", Value::Number(context.now));
            let value = Value::Object(entry);
            all[at] = value.clone();
            Some(value)
        }
    };
    while all.iter().filter(|e| valid(e)).count() > MAX_ENTRIES {
        let at = all
            .iter()
            .position(|e| valid(e) && !standing(e) && e.get("resolved") == Some(&Value::Bool(true)))
            .or_else(|| all.iter().position(|e| valid(e) && !standing(e)))
            .or_else(|| all.iter().position(valid))
            .unwrap();
        archive(&mut out, all.remove(at));
    }
    out.insert("memory", Value::Array(all));
    Ok((Value::Object(out), result))
}
fn set_why(entry: &mut Map, text: &JsString) {
    let text = clip(text);
    if text.as_bytes().is_empty() {
        entry.remove("why");
    } else {
        entry.insert("why", Value::String(text));
    }
}
