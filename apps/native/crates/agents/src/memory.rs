//! Main-reference project memory. Plans are non-Undo autosave mutations on the
//! kernel actor; no independent project/library writer is opened here.
use crate::{error, trace};
use serde_json::{Value, json};
use std::{collections::BTreeSet, io};
pub const KINDS: [&str; 6] = [
    "goal",
    "constraint",
    "decision",
    "rejected",
    "feedback",
    "open",
];
pub const DISABLED: &str =
    "Project memory is turned off for this project. The user can turn it on in Settings > Memory.";
fn clipped(value: &Value, limit: usize) -> String {
    trace::truncate(value.as_str().unwrap_or_default().trim(), limit)
}
fn kind(value: &Value) -> bool {
    value.as_str().is_some_and(|s| KINDS.contains(&s))
}
fn number(value: &Value, fallback: &Value) -> Value {
    if value.as_f64().is_some_and(f64::is_finite) {
        value.clone()
    } else {
        fallback.clone()
    }
}
pub fn enabled(project: &Value) -> bool {
    project["memoryEnabled"] != false
}
/// Bounded read projection. The original archive is never sanitized in place.
pub fn sanitize(raw: &Value, now: u64) -> Vec<Value> {
    let mut seen = BTreeSet::new();
    let mut out = vec![];
    for item in raw.as_array().into_iter().flatten() {
        let id = trace::truncate(item["id"].as_str().unwrap_or_default(), 80);
        let text = clipped(&item["text"], 600);
        if id.is_empty() || text.is_empty() || !kind(&item["kind"]) || !seen.insert(id.clone()) {
            continue;
        }
        let created = number(&item["createdAt"], &json!(now));
        let mut entry = json!({"id":id,"kind":item["kind"],"text":text,"author":if item["author"]=="user"{"user"}else{"agent"},"createdAt":created,"updatedAt":number(&item["updatedAt"],&created)});
        let why = clipped(&item["why"], 600);
        if !why.is_empty() {
            entry["why"] = json!(why);
        }
        if item["resolved"] == true {
            entry["resolved"] = json!(true);
        }
        out.push(entry);
    }
    out.into_iter()
        .rev()
        .take(100)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}
fn recent(mut entries: Vec<Value>, limit: usize) -> Vec<Value> {
    entries.sort_by(|a, b| {
        b["updatedAt"]
            .as_f64()
            .unwrap_or(0.)
            .total_cmp(&a["updatedAt"].as_f64().unwrap_or(0.))
    });
    entries.truncate(limit);
    entries
}
pub fn list(entries: &[Value], filter: Option<&str>, limit: usize) -> Vec<Value> {
    recent(
        entries
            .iter()
            .filter(|v| filter.is_none_or(|k| v["kind"] == k))
            .cloned()
            .collect(),
        limit.clamp(1, 100),
    )
}
pub fn index(entries: &[Value]) -> Value {
    json!({"total":entries.len(),"goal":entries.iter().find(|v|v["kind"]=="goal"),
        "constraints":entries.iter().filter(|v|v["kind"]=="constraint").collect::<Vec<_>>(),
        "open":entries.iter().filter(|v|v["kind"]=="open"&&v["resolved"]!=true).collect::<Vec<_>>(),
        "recent":recent(entries.iter().filter(|v|!matches!(v["kind"].as_str(),Some("goal"|"constraint"|"open"))).cloned().collect(),8)})
}
fn patch(current: &Value, op: &Value, now: u64) -> io::Result<Value> {
    let mut next = current.clone();
    if let Some(text) = op.get("text") {
        let text = clipped(text, 600);
        if text.is_empty() {
            return Err(error("Memory text cannot be empty."));
        }
        next["text"] = json!(text);
    }
    if let Some(why) = op.get("why") {
        let why = clipped(why, 600);
        if why.is_empty() {
            next.as_object_mut().unwrap().remove("why");
        } else {
            next["why"] = json!(why);
        }
    }
    if let Some(kind) = op.get("kind") {
        next["kind"] = kind.clone();
    }
    if let Some(resolved) = op.get("resolved") {
        if *resolved == true {
            next["resolved"] = json!(true);
        } else {
            next.as_object_mut().unwrap().remove("resolved");
        }
    }
    next["updatedAt"] = json!(now);
    Ok(next)
}
/// Main's active-list algorithm; the actor plan below archives every displaced
/// raw record, including explicit removals, capacity eviction and malformed data.
pub fn apply(
    entries: &[Value],
    op: &Value,
    now: u64,
    uid: &str,
    author: &str,
) -> io::Result<(Vec<Value>, Value)> {
    let mut next = entries.to_vec();
    match op["operation"].as_str() {
        Some("remove" | "update") => {
            let id = op["id"].as_str().unwrap_or("undefined");
            let position = next.iter().position(|v| v["id"] == id).ok_or_else(|| {
                error(format!(
                    "No memory entry {id}. Use project_memory list to see the ids."
                ))
            })?;
            if op["operation"] == "remove" {
                next.remove(position);
                return Ok((next, Value::Null));
            }
            if op["kind"] == "goal"
                && next[position]["kind"] != "goal"
                && next.iter().any(|v| v["kind"] == "goal")
            {
                return Err(error(
                    "The project already has a goal. Update that entry instead.",
                ));
            }
            let entry = patch(&next[position], op, now)?;
            next[position] = entry.clone();
            Ok((next, entry))
        }
        Some("add") => {
            if !kind(&op["kind"]) {
                return Err(error(format!("kind must be one of {}.", KINDS.join(", "))));
            }
            let text = clipped(&op["text"], 600);
            if text.is_empty() {
                return Err(error("Memory text cannot be empty."));
            }
            if op["kind"] == "goal"
                && let Some(position) = next.iter().position(|v| v["kind"] == "goal")
            {
                let mut change = json!({"text":text});
                if let Some(why) = op.get("why") {
                    change["why"] = why.clone();
                }
                let entry = patch(&next[position], &change, now)?;
                next[position] = entry.clone();
                return Ok((next, entry));
            }
            if uid.is_empty() || entries.iter().any(|v| v["id"] == uid) {
                return Err(error("Memory identity conflict"));
            }
            let mut entry = json!({"id":uid,"kind":op["kind"],"text":text,"author":author,"createdAt":now,"updatedAt":now});
            let why = clipped(&op["why"], 600);
            if !why.is_empty() {
                entry["why"] = json!(why);
            }
            if op["resolved"] == true {
                entry["resolved"] = json!(true);
            }
            next.push(entry.clone());
            while next.len() > 100 {
                let droppable =
                    |v: &Value| !matches!(v["kind"].as_str(), Some("goal" | "constraint"));
                let position = next
                    .iter()
                    .position(|v| droppable(v) && v["resolved"] == true)
                    .or_else(|| next.iter().position(droppable))
                    .unwrap_or(0);
                next.remove(position);
            }
            Ok((next, entry))
        }
        _ => Err(error("operation must be list, add, update or remove.")),
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub document: String,
    pub generation: u64,
    pub revision: u64,
    pub project: Value,
}
pub trait Host {
    fn snapshot(&mut self) -> io::Result<Snapshot>;
    /// Compare document identity, generation, revision and memory bytes on the
    /// actor, persist/autosave without Undo or a revision bump, then emit memory.
    fn publish(&mut self, expected: &Snapshot, project: &Value) -> io::Result<()>;
}
pub fn project_index(project: &Value, now: u64) -> Value {
    if enabled(project) {
        index(&sanitize(&project["memory"], now))
    } else {
        json!({"enabled":false})
    }
}
fn validate_args(args: &Value) -> io::Result<()> {
    let fields = args
        .as_object()
        .ok_or_else(|| error("Memory arguments must be an object"))?;
    if fields.keys().any(|k| {
        ![
            "operation",
            "id",
            "kind",
            "text",
            "why",
            "resolved",
            "limit",
        ]
        .contains(&k.as_str())
    }) {
        return Err(error("Unknown project_memory argument"));
    }
    for (key, limit) in [("id", 80), ("text", 600), ("why", 600)] {
        if fields
            .get(key)
            .is_some_and(|v| v.as_str().is_none_or(|s| s.encode_utf16().count() > limit))
        {
            return Err(error(format!("Invalid memory {key}")));
        }
    }
    if fields.get("kind").is_some_and(|v| !kind(v))
        || fields.get("resolved").is_some_and(|v| !v.is_boolean())
        || fields
            .get("limit")
            .is_some_and(|v| !v.as_u64().is_some_and(|v| (1..=100).contains(&v)))
    {
        return Err(error("Invalid memory arguments"));
    }
    Ok(())
}
pub fn call(
    host: &mut impl Host,
    pinned: (&str, u64),
    args: &Value,
    now: u64,
    uid: &str,
) -> io::Result<Value> {
    validate_args(args)?;
    let snapshot = host.snapshot()?;
    if snapshot.document != pinned.0 || snapshot.generation != pinned.1 {
        return Err(error("The project changed before the memory operation."));
    }
    if !enabled(&snapshot.project) {
        return Err(error(DISABLED));
    }
    let entries = sanitize(&snapshot.project["memory"], now);
    if args["operation"] == "list" {
        return Ok(
            json!({"memory":list(&entries,args["kind"].as_str(),args["limit"].as_u64().unwrap_or(100) as usize),"total":entries.len()}),
        );
    }
    let (next, entry) = apply(&entries, args, now, uid, "agent")?;
    let mut project = snapshot.project.clone();
    if project.get("memoryArchive").is_some_and(|v| !v.is_array()) {
        return Err(error("Unknown memory archive; preserved unchanged"));
    }
    let mut archive = project
        .get("memoryArchive")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if snapshot
        .project
        .get("memory")
        .is_some_and(|v| !v.is_array() && !v.is_null())
    {
        archive
            .push(json!({"raw":snapshot.project["memory"],"at":now,"operation":args["operation"]}));
    }
    let mut kept = Vec::new();
    for original in snapshot.project["memory"].as_array().into_iter().flatten() {
        if let Some(updated) = next.iter().find(|v| v["id"] == original["id"])
            && !kept.iter().any(|v: &Value| v["id"] == original["id"])
        {
            let mut full = original.clone();
            if let Some(map) = full.as_object_mut() {
                for key in [
                    "id",
                    "kind",
                    "text",
                    "why",
                    "author",
                    "createdAt",
                    "updatedAt",
                    "resolved",
                ] {
                    map.remove(key);
                }
                for (k, v) in updated.as_object().unwrap() {
                    map.insert(k.clone(), v.clone());
                }
            }
            kept.push(full.clone());
            if full == *original {
                continue;
            }
        }
        archive.push(json!({"entry":original,"at":now,"operation":args["operation"]}));
    }
    for item in &next {
        if !kept.iter().any(|v| v["id"] == item["id"]) {
            kept.push(item.clone());
        }
    }
    project["memory"] = json!(kept);
    project["memoryArchive"] = json!(archive);
    host.publish(&snapshot, &project)?;
    Ok(json!({"ok":true,"entry":entry,"total":next.len()}))
}
