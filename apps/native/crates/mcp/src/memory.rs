//! Owner-pinned main project-memory semantics. Plans are pure; publishing notes
//! requires kernel metadata admission without edit revision/history changes.
use crate::{Error, Result};
use serde_json::{Value, json};

pub const KINDS: &[&str] = &[
    "goal",
    "constraint",
    "decision",
    "rejected",
    "feedback",
    "open",
];
pub const DISABLED: &str =
    "Project memory is turned off for this project. The user can turn it on in Settings > Memory.";
fn standing(entry: &Value) -> bool {
    entry["kind"] == "goal" || entry["kind"] == "constraint"
}
pub fn enabled(project: &Value) -> bool {
    project["memoryEnabled"] != false
}
fn clip(value: Option<&Value>, max: usize) -> String {
    let Some(text) = value.and_then(Value::as_str) else {
        return String::new();
    };
    cut(pm_model::json::js_trim(text), max)
}
fn cut(text: &str, max: usize) -> String {
    let mut units = 0;
    text.chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= max
        })
        .collect()
}
fn timestamp(value: Option<&Value>, fallback: f64) -> Value {
    json!(value.and_then(Value::as_f64).unwrap_or(fallback))
}
pub fn sanitize(raw: &Value, now: f64) -> Vec<Value> {
    let mut seen = std::collections::HashSet::new();
    let mut entries = Vec::new();
    for item in raw.as_array().into_iter().flatten() {
        let text = clip(item.get("text"), 600);
        let id = cut(item["id"].as_str().unwrap_or(""), 80);
        if text.is_empty()
            || id.is_empty()
            || !item["kind"].as_str().is_some_and(|s| KINDS.contains(&s))
            || !seen.insert(id.clone())
        {
            continue;
        }
        let created = timestamp(item.get("createdAt"), now);
        let why = clip(item.get("why"), 600);
        let mut entry = json!({"id":id,"kind":item["kind"],"text":text,"author":if item["author"]=="user"{"user"}else{"agent"},"createdAt":created,"updatedAt":timestamp(item.get("updatedAt"),created.as_f64().unwrap())});
        if !why.is_empty() {
            entry["why"] = json!(why)
        }
        if item["resolved"] == true {
            entry["resolved"] = json!(true)
        }
        entries.push(entry);
    }
    if entries.len() > 100 {
        entries.drain(..entries.len() - 100);
    }
    entries
}
pub fn index(raw: &Value) -> Value {
    let entries = raw.as_array().cloned().unwrap_or_default();
    let mut recent = entries
        .iter()
        .filter(|e| !standing(e) && e["kind"] != "open")
        .cloned()
        .collect::<Vec<_>>();
    sort_recent(&mut recent);
    recent.truncate(8);
    json!({"total":entries.len(),"goal":entries.iter().find(|e|e["kind"]=="goal"),"constraints":entries.iter().filter(|e|e["kind"]=="constraint").collect::<Vec<_>>(),"open":entries.iter().filter(|e|e["kind"]=="open"&&e["resolved"]!=true).collect::<Vec<_>>(),"recent":recent})
}
fn sort_recent(entries: &mut [Value]) {
    entries.sort_by(|a, b| {
        b["updatedAt"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&a["updatedAt"].as_f64().unwrap_or(0.0))
    });
}
pub fn list(project: &Value, args: &Value) -> Result<Value> {
    if !enabled(project) {
        return Err(Error::invalid(DISABLED));
    }
    let entries = project["memory"].as_array().cloned().unwrap_or_default();
    let mut selected = entries
        .iter()
        .filter(|e| {
            args.get("kind")
                .is_none_or(|k| k.is_null() || k == "" || e["kind"] == *k)
        })
        .cloned()
        .collect::<Vec<_>>();
    sort_recent(&mut selected);
    let limit = args["limit"]
        .as_f64()
        .unwrap_or(100.0)
        .trunc()
        .clamp(1.0, 100.0) as usize;
    selected.truncate(limit);
    Ok(json!({"memory":selected,"total":entries.len()}))
}
pub struct Plan {
    pub entries: Vec<Value>,
    pub entry: Value,
    pub retired: Vec<Value>,
}
impl Plan {
    pub fn result(&self) -> Value {
        json!({"ok":true,"entry":self.entry,"total":self.entries.len()})
    }
}
pub fn apply(entries: &[Value], args: &Value, now: f64, id: &str, author: &str) -> Result<Plan> {
    let operation = args["operation"].as_str().unwrap_or("");
    if operation == "remove" {
        let Some(position) = entries.iter().position(|e| e["id"] == args["id"]) else {
            return Err(missing(args));
        };
        let mut next = entries.to_vec();
        let retired = vec![next.remove(position)];
        return Ok(Plan {
            entries: next,
            entry: Value::Null,
            retired,
        });
    }
    let patch = |current: &Value, change: &Value| -> Result<Value> {
        let text = if change.get("text").is_none() {
            current["text"].as_str().unwrap_or("").to_owned()
        } else {
            clip(change.get("text"), 600)
        };
        if text.is_empty() {
            return Err(Error::invalid("Memory text cannot be empty."));
        }
        let why = if change.get("why").is_none() {
            current["why"].as_str().unwrap_or("").to_owned()
        } else {
            clip(change.get("why"), 600)
        };
        let mut next = current.clone();
        let object = next
            .as_object_mut()
            .ok_or_else(|| Error::invalid("Invalid stored memory entry"))?;
        object.remove("resolved");
        object.remove("why");
        next["text"] = json!(text);
        next["kind"] = change
            .get("kind")
            .cloned()
            .unwrap_or_else(|| current["kind"].clone());
        next["updatedAt"] = json!(now);
        if !why.is_empty() {
            next["why"] = json!(why)
        }
        if change
            .get("resolved")
            .filter(|v| !v.is_null())
            .unwrap_or(&current["resolved"])
            == &json!(true)
        {
            next["resolved"] = json!(true)
        }
        Ok(next)
    };
    if operation == "update" {
        let Some(position) = entries.iter().position(|e| e["id"] == args["id"]) else {
            return Err(missing(args));
        };
        if args["kind"] == "goal"
            && entries[position]["kind"] != "goal"
            && entries.iter().any(|e| e["kind"] == "goal")
        {
            return Err(Error::invalid(
                "The project already has a goal. Update that entry instead.",
            ));
        }
        let entry = patch(&entries[position], args)?;
        let mut next = entries.to_vec();
        next[position] = entry.clone();
        return Ok(Plan {
            entries: next,
            entry,
            retired: vec![entries[position].clone()],
        });
    }
    if operation != "add" {
        return Err(Error::invalid(
            "operation must be list, add, update or remove.",
        ));
    }
    if !args["kind"].as_str().is_some_and(|s| KINDS.contains(&s)) {
        return Err(Error::invalid(format!(
            "kind must be one of {}.",
            KINDS.join(", ")
        )));
    }
    let text = clip(args.get("text"), 600);
    if text.is_empty() {
        return Err(Error::invalid("Memory text cannot be empty."));
    }
    if args["kind"] == "goal"
        && let Some(position) = entries.iter().position(|e| e["kind"] == "goal")
    {
        let mut change = json!({"text":text});
        if let Some(why) = args.get("why") {
            change["why"] = why.clone()
        }
        let entry = patch(&entries[position], &change)?;
        let mut next = entries.to_vec();
        next[position] = entry.clone();
        return Ok(Plan {
            entries: next,
            entry,
            retired: vec![entries[position].clone()],
        });
    }
    let why = clip(args.get("why"), 600);
    let mut entry = json!({"id":id,"kind":args["kind"],"text":text,"author":author,"createdAt":now,"updatedAt":now});
    if !why.is_empty() {
        entry["why"] = json!(why)
    }
    if args["resolved"] == true {
        entry["resolved"] = json!(true)
    }
    let mut next = entries.to_vec();
    next.push(entry.clone());
    let mut retired = Vec::new();
    while next.len() > 100 {
        let position = next
            .iter()
            .position(|e| !standing(e) && e["resolved"] == true)
            .or_else(|| next.iter().position(|e| !standing(e)))
            .unwrap_or(0);
        retired.push(next.remove(position));
    }
    Ok(Plan {
        entries: next,
        entry,
        retired,
    })
}
fn missing(args: &Value) -> Error {
    let id = args.get("id").map_or("undefined".into(), |v| {
        v.as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| v.to_string())
    });
    Error::invalid(format!(
        "No memory entry {id}. Use project_memory list to see the ids."
    ))
}

/// Generated agent inputs obey the same disabled-note policy as tool reads.
/// This never mutates the kernel document, PMV, or an authored source file.
pub fn agent_snapshot(project: &Value) -> Value {
    fn redact(comp: &mut Value, hidden: bool) {
        let hidden = hidden || comp["memoryEnabled"] == false;
        if hidden && let Some(object) = comp.as_object_mut() {
            object.remove("memory");
        }
        if let Some(comps) = comp.get_mut("comps").and_then(Value::as_object_mut) {
            for child in comps.values_mut() {
                redact(child, hidden);
            }
        }
    }
    let mut snapshot = project.clone();
    redact(&mut snapshot, false);
    snapshot
}
