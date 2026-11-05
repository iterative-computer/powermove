//! Legacy-compatible logical store keys. Implement Store on the kernel actor;
//! this module never opens the library or becomes a second library writer.
use crate::{error, valid_id};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io};

pub const MAX_STORE_BYTES: usize = 32 * 1024 * 1024;
pub trait Store {
    fn get(&mut self, key: &str) -> io::Result<Option<Value>>;
    /// Must acknowledge durable publication, or return an error. Attachments are
    /// written before their referencing archive, so a crash cannot dangle them.
    fn set(&mut self, key: &str, value: &Value) -> io::Result<()>;
}
#[derive(Clone)]
pub struct Threads {
    project: String,
    archive: Value,
}
impl Threads {
    pub fn load(
        store: &mut impl Store,
        project: &str,
        blank_id: &str,
        now: u64,
    ) -> io::Result<Self> {
        if !valid_id(project, 200) || !valid_id(blank_id, 120) {
            return Err(error("Invalid project or thread id"));
        }
        let archive = store
            .get(&format!("agentThreads.{project}"))?
            .unwrap_or_else(
                || json!({"version":1,"activeId":blank_id,"threads":[new_thread(blank_id,now)]}),
            );
        // Refuse unknown/malformed archives without normalizing away their data.
        validate(&archive)?;
        Ok(Self {
            project: project.into(),
            archive,
        })
    }
    /// Execute on the kernel actor. Publish a candidate before updating the
    /// in-memory view, so failed durable writes cannot acknowledge lost history.
    /// Unknown fields and all existing payload references remain in the clone.
    pub fn update(
        &mut self,
        store: &mut impl Store,
        edit: impl FnOnce(&mut Value) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut candidate = Self {
            project: self.project.clone(),
            archive: self.archive.clone(),
        };
        edit(&mut candidate.archive)?;
        candidate.save(store)?;
        self.archive = candidate.archive;
        Ok(())
    }
    pub fn raw(&self) -> &Value {
        &self.archive
    }
    pub fn active_id(&self) -> &str {
        self.archive["activeId"].as_str().unwrap_or("")
    }
    pub fn thread(&self, id: &str) -> Option<&Value> {
        self.archive["threads"]
            .as_array()?
            .iter()
            .find(|v| v["id"] == id)
    }
    pub fn thread_mut(&mut self, id: &str) -> Option<&mut Value> {
        self.archive["threads"]
            .as_array_mut()?
            .iter_mut()
            .find(|v| v["id"] == id)
    }
    pub fn select(&mut self, id: &str) -> bool {
        if self.thread(id).is_none() {
            false
        } else {
            self.archive["activeId"] = json!(id);
            true
        }
    }
    pub fn create(&mut self, id: &str, now: u64) -> io::Result<&Value> {
        if !valid_id(id, 120) {
            return Err(error("Invalid thread id"));
        }
        let blank = self.archive["threads"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| {
                t["conversation"].as_array().is_some_and(Vec::is_empty)
                    && t["composerDraft"].as_str().unwrap_or("").trim().is_empty()
                    && t["attachments"].as_array().is_none_or(Vec::is_empty)
            })
            .map(|t| t["id"].as_str().unwrap().to_owned());
        if let Some(blank) = blank {
            self.select(&blank);
            return Ok(self.thread(&blank).unwrap());
        }
        if self.thread(id).is_some() {
            return Err(error("Duplicate thread id"));
        }
        self.archive["threads"]
            .as_array_mut()
            .unwrap()
            .insert(0, new_thread(id, now));
        self.select(id);
        Ok(self.thread(id).unwrap())
    }
    /// N4: the shipped remove operation becomes an archive operation. Its entire
    /// opaque record (including attachments/resume handles) remains reachable.
    pub fn archive_thread(&mut self, id: &str, replacement: &str, now: u64) -> io::Result<bool> {
        let Some(index) = self.archive["threads"]
            .as_array()
            .unwrap()
            .iter()
            .position(|t| t["id"] == id)
        else {
            return Ok(false);
        };
        if !valid_id(replacement, 120) || replacement == id || self.thread(replacement).is_some() {
            return Err(error("Invalid replacement id"));
        }
        if self
            .archive
            .get("archivedThreads")
            .is_some_and(|v| !v.is_array())
        {
            return Err(error("Unknown archivedThreads shape"));
        }
        let thread = self.archive["threads"]
            .as_array_mut()
            .unwrap()
            .remove(index);
        if self.archive.get("archivedThreads").is_none() {
            self.archive["archivedThreads"] = json!([]);
        }
        self.archive["archivedThreads"]
            .as_array_mut()
            .unwrap()
            .push(thread);
        if self.archive["threads"].as_array().unwrap().is_empty() {
            self.archive["threads"]
                .as_array_mut()
                .unwrap()
                .push(new_thread(replacement, now));
        }
        if self.active_id() == id {
            self.archive["activeId"] = self.archive["threads"]
                [index.min(self.archive["threads"].as_array().unwrap().len() - 1)]["id"]
                .clone();
        }
        Ok(true)
    }
    /// Hydration is a view. Missing payloads retain payloadStored and the exact
    /// metadata, allowing later media recovery instead of silently losing intent.
    pub fn hydrated(&self, store: &mut impl Store) -> io::Result<Value> {
        let mut v = self.archive.clone();
        visit_attachments(&mut v, &mut |item| {
            if item["payloadStored"] == true
                && let Some(id) = item["id"].as_str().filter(|s| valid_id(s, 120))
                && let Some(payload) = store.get(&format!("agentAttachment.{id}"))?
                && let Some(p) = payload.as_object()
            {
                for (k, v) in p {
                    item[k] = v.clone();
                }
            }
            Ok(())
        })?;
        Ok(v)
    }
    pub fn save(&self, store: &mut impl Store) -> io::Result<()> {
        validate(&self.archive)?;
        let mut archive = self.archive.clone();
        let mut payloads = BTreeMap::<String, Value>::new();
        visit_attachments(&mut archive, &mut |item| {
            let Some(id) = item["id"]
                .as_str()
                .filter(|s| valid_id(s, 120))
                .map(str::to_string)
            else {
                return Ok(());
            };
            let mut payload = serde_json::Map::new();
            for key in ["dataUrl", "dataBase64", "content"] {
                if let Some(v) = item.get(key).filter(|v| v.is_string()).cloned() {
                    payload.insert(key.into(), v);
                    item.as_object_mut().unwrap().remove(key);
                }
            }
            if !payload.is_empty() {
                let key = format!("agentAttachment.{id}");
                let payload = Value::Object(payload);
                check_size(&payload)?;
                if payloads.get(&key).is_some_and(|p| p != &payload) {
                    return Err(error("Conflicting attachment payloads"));
                }
                payloads.insert(key, payload);
                item["payloadStored"] = json!(true);
            }
            Ok(())
        })?;
        check_size(&archive)?;
        // Preflight every conflict before writing any payload. Existing immutable
        // attachment bytes are never replaced; a caller must mint a new id.
        let mut writes = vec![];
        for (key, value) in payloads {
            match store.get(&key)? {
                Some(old) if old != value => {
                    return Err(error("Attachment id already owns different bytes"));
                }
                Some(_) => {}
                None => writes.push((key, value)),
            }
        }
        for (key, value) in writes {
            store.set(&key, &value)?;
        }
        store.set(&format!("agentThreads.{}", self.project), &archive)
    }
}
fn check_size(v: &Value) -> io::Result<()> {
    if serde_json::to_vec(v)?.len() > MAX_STORE_BYTES {
        Err(error(
            "Agent store value exceeds 32 MiB; existing data retained",
        ))
    } else {
        Ok(())
    }
}
fn validate(v: &Value) -> io::Result<()> {
    if v["version"] != 1 || !v["activeId"].is_string() {
        return Err(error(
            "Unrecognized agent thread archive; preserved unchanged",
        ));
    }
    let threads = v["threads"]
        .as_array()
        .ok_or_else(|| error("Invalid threads array; preserved unchanged"))?;
    let mut seen = std::collections::BTreeSet::new();
    for t in threads {
        let Some(id) = t["id"].as_str().filter(|s| valid_id(s, 120)) else {
            return Err(error("Invalid thread id; preserved unchanged"));
        };
        if !seen.insert(id) || !t["conversation"].is_array() || !t["composerDraft"].is_string() {
            return Err(error("Invalid thread record; preserved unchanged"));
        }
    }
    if !threads.is_empty() && !threads.iter().any(|t| t["id"] == v["activeId"]) {
        return Err(error("Missing active thread; preserved unchanged"));
    }
    Ok(())
}
fn visit_attachments(
    v: &mut Value,
    f: &mut impl FnMut(&mut Value) -> io::Result<()>,
) -> io::Result<()> {
    for name in ["threads", "archivedThreads"] {
        if let Some(threads) = v.get_mut(name).and_then(Value::as_array_mut) {
            for t in threads {
                if let Some(a) = t.get_mut("attachments").and_then(Value::as_array_mut) {
                    for item in a {
                        if item.is_object() {
                            f(item)?;
                        }
                    }
                }
                if let Some(c) = t.get_mut("conversation").and_then(Value::as_array_mut) {
                    for m in c {
                        if let Some(a) = m.get_mut("attachments").and_then(Value::as_array_mut) {
                            for item in a {
                                if item.is_object() {
                                    f(item)?;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
pub fn new_thread(id: &str, now: u64) -> Value {
    json!({"id":id,"title":"New thread","updatedAt":now,"conversation":[],"composerDraft":"","attachments":[],"scope":"workspace"})
}
/// Find all lightweight attachment references, including archived messages.
pub fn attachment_ids(archive: &Value) -> Vec<String> {
    let mut a = archive.clone();
    let mut ids = std::collections::BTreeSet::new();
    let _ = visit_attachments(&mut a, &mut |v| {
        if v["payloadStored"] == true
            && let Some(id) = v["id"].as_str()
        {
            ids.insert(id.into());
        }
        Ok(())
    });
    ids.into_iter().collect()
}
