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
    crate::json_value(&agent_snapshot_model(&crate::model(project)))
}

/// Redact before a Unicode JSON projection. Authored UTF-16 is retained in
/// generated JSON files, while disabled and retired notes stay in the project.
pub fn agent_snapshot_model(project: &pm_model::Value) -> pm_model::Value {
    fn redact(comp: &mut pm_model::Value, hidden: bool) {
        let hidden = hidden || comp.get("memoryEnabled") == Some(&pm_model::Value::Bool(false));
        let Some(object) = comp.as_object_mut() else {
            return;
        };
        object.remove("nativeMemoryArchive");
        object.remove("nativeMediaVersions");
        if hidden {
            object.remove("memory");
        }
        if let Some(comps) = object
            .get_mut("comps")
            .and_then(pm_model::Value::as_object_mut)
        {
            for (_, child) in comps.iter_mut() {
                redact(child, hidden);
            }
        }
    }
    fn strip_retired_captions(value: &mut pm_model::Value) {
        if value.get("type").and_then(pm_model::Value::as_str) == Some("captions")
            && let Some(d) = value
                .as_object_mut()
                .and_then(|m| m.get_mut("d"))
                .and_then(pm_model::Value::as_object_mut)
        {
            d.remove("nativeCaptionArchive");
        }
        match value {
            pm_model::Value::Object(m) => {
                for (_, child) in m.iter_mut() {
                    strip_retired_captions(child);
                }
            }
            pm_model::Value::Array(items) => {
                for child in items {
                    strip_retired_captions(child);
                }
            }
            _ => {}
        }
    }
    let mut snapshot = project.clone();
    redact(&mut snapshot, false);
    strip_retired_captions(&mut snapshot);
    snapshot
}
