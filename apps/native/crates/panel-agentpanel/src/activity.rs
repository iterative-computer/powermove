//! Ordered, id-addressable, immutable row publications. Windowing is by logical
//! row, never by pixels. Hosts measure/layout only the requested window.
use crate::{Error, Result};
use serde_json::{Value, json};
use std::{collections::HashMap, ops::Range, sync::Arc};

#[derive(Clone, Debug)]
pub struct Row {
    pub id: String,
    pub value: Arc<Value>,
    pub revision: u64,
}
#[derive(Clone, Debug, Default)]
pub struct Rows {
    order: Vec<Row>,
    index: HashMap<String, usize>,
}
impl Rows {
    pub fn len(&self) -> usize {
        self.order.len()
    }
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
    pub fn get(&self, id: &str) -> Option<&Row> {
        self.index.get(id).map(|i| &self.order[*i])
    }
    pub fn last(&self) -> Option<&Row> {
        self.order.last()
    }
    pub fn at(&self, index: usize) -> Option<&Row> {
        self.order.get(index)
    }
    pub fn page(&self, range: Range<usize>) -> &[Row] {
        let start = range.start.min(self.len());
        &self.order[start..range.end.max(start).min(self.len())]
    }
    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.order.iter().map(|r| r.value.as_ref())
    }
    /// A row keeps its position/id and Arc on no-op updates, including streaming.
    pub fn upsert(&mut self, id: String, value: Value) -> Result<bool> {
        if id.is_empty() || !value.is_object() {
            return Err(Error::Invalid("row"));
        }
        if let Some(index) = self.index.get(&id).copied() {
            let old = &mut self.order[index];
            if *old.value == value {
                return Ok(false);
            }
            old.value = Arc::new(value);
            old.revision += 1;
        } else {
            self.index.insert(id.clone(), self.order.len());
            self.order.push(Row {
                id,
                value: Arc::new(value),
                revision: 1,
            });
        }
        Ok(true)
    }
    pub fn update(&mut self, id: &str, f: impl FnOnce(&mut Value)) -> Result<bool> {
        let row = self.get(id).ok_or_else(|| Error::UnknownId(id.into()))?;
        let mut next = (*row.value).clone();
        f(&mut next);
        self.upsert(id.into(), next)
    }
}
#[derive(Clone, Debug, Default)]
pub struct Trace {
    pub rows: Rows,
    serial: u64,
    answer_sources: HashMap<String, String>,
    pub placements: Vec<String>,
}
impl Trace {
    fn next_id(&mut self, prefix: &str) -> String {
        loop {
            self.serial += 1;
            let id = format!("{prefix}-{}", self.serial);
            if self.rows.get(&id).is_none() {
                return id;
            }
        }
    }
    pub fn finish_thought(&mut self, now: u64) -> Result<()> {
        if let Some(row) = self.rows.last()
            && row.value["kind"] == "thought"
            && row.value["live"] == true
        {
            let id = row.id.clone();
            self.rows.update(&id, |v| {
                v["live"] = json!(false);
                v["endedAt"] = json!(now);
            })?;
        }
        Ok(())
    }
    /// Provider events emitted by pm-agents; no source code is evaluated here.
    /// UI-placement metadata filtering belongs to the provider/placement adapter.
    /// Full thought/text/output bytes are retained; the view may request excerpts.
    pub fn receive(&mut self, event: &Value, now: u64) -> Result<()> {
        let kind = event["kind"].as_str().ok_or(Error::Invalid("trace kind"))?;
        match kind {
            "thought" | "answer" => {
                let text = event["text"].as_str().ok_or(Error::Invalid("trace text"))?;
                if text.is_empty() {
                    return Ok(());
                }
                if kind == "thought" && crate::placement::is_message(text) {
                    return Ok(());
                }
                let field = if kind == "thought" { "label" } else { "text" };
                let row_kind = if kind == "thought" { "thought" } else { "text" };
                if kind == "answer" {
                    self.finish_thought(now)?;
                }
                let id = match self.rows.last() {
                    Some(r)
                        if r.value["kind"] == row_kind
                            && (kind != "thought" || r.value["live"] == true) =>
                    {
                        r.id.clone()
                    }
                    _ => {
                        let id = self.next_id(row_kind);
                        let value = if kind == "thought" {
                            json!({"id":id,"kind":row_kind,"label":"","live":true,"startedAt":now})
                        } else {
                            json!({"id":id,"kind":row_kind,"text":""})
                        };
                        self.rows.upsert(id.clone(), value)?;
                        id
                    }
                };
                if kind == "answer" {
                    let mut raw = self.answer_sources.get(&id).cloned().unwrap_or_else(|| {
                        self.rows.get(&id).expect("text row").value["text"]
                            .as_str()
                            .unwrap_or_default()
                            .into()
                    });
                    if !raw.is_empty() && crate::placement::is_message(text) {
                        raw.push('\n');
                    }
                    raw.push_str(text);
                    let split = crate::placement::split(&raw, true);
                    for message in split.messages {
                        if !self.placements.contains(&message) {
                            self.placements.push(message);
                        }
                    }
                    self.answer_sources.insert(id.clone(), raw);
                    self.rows.update(&id, |v| v["text"] = json!(split.text))?;
                } else {
                    self.rows.update(&id, |v| {
                        let mut s = v[field].as_str().unwrap_or_default().to_owned();
                        s.push_str(text);
                        v[field] = json!(s);
                    })?;
                }
            }
            "tool-start" => {
                let id = event_id(event)?;
                self.finish_thought(now)?;
                if self.rows.get(id).is_some() {
                    self.rows.update(id, |v| {
                        if let Some(label) = event.get("label") {
                            v["label"] = label.clone();
                        }
                        if let Some(detail) = event.get("detail") {
                            v["detail"] = detail.clone();
                        }
                    })?;
                } else {
                    let mut row = event.clone();
                    row["kind"] = json!("tool");
                    row["id"] = json!(id);
                    row.as_object_mut().expect("object").remove("itemId");
                    row["status"] = json!("running");
                    row["startedAt"] = json!(now);
                    self.rows.upsert(id.into(), row)?;
                }
            }
            "tool-end" => {
                let id = event_id(event)?;
                self.rows.update(id, |v| {
                    v["status"] = json!(if event["isError"] == true {
                        "error"
                    } else {
                        "done"
                    });
                    v["endedAt"] = json!(now);
                    if let Some(output) = event.get("output") {
                        v["output"] = output.clone();
                    }
                    if let Some(outcome) = event.get("outcome") {
                        v["outcome"] = outcome.clone();
                    } else if let Some(outcome) =
                        media_outcome(v["toolName"].as_str().unwrap_or_default(), &event["output"])
                    {
                        v["outcome"] = outcome["state"].clone();
                        if let Some(progress) = outcome.get("progress") {
                            v["outcomeProgress"] = progress.clone();
                        }
                        v["status"] = json!("done");
                    }
                })?;
            }
            "question" => {
                let id = event_id(event)?;
                if self.rows.get(id).is_some() {
                    return Ok(());
                }
                if event["questions"].as_array().is_none_or(|q| q.is_empty()) {
                    return Err(Error::Invalid("questions"));
                }
                self.finish_thought(now)?;
                let mut row = event.clone();
                row["id"] = json!(id);
                row["status"] = json!("open");
                let reply = event["transport"] == "reply";
                row["transport"] = json!(if reply { "reply" } else { "message" });
                row["blocking"] = json!(reply && event["blocking"] == true);
                row.as_object_mut().expect("object").remove("itemId");
                self.rows.upsert(id.into(), row)?;
            }
            "question-closed" => {
                let id = event_id(event)?;
                if self
                    .rows
                    .get(id)
                    .is_some_and(|r| r.value["status"] == "open")
                {
                    self.rows.update(id, |v| v["status"] = json!("closed"))?;
                }
            }
            "task" => {
                let task = &event["task"];
                let id = task["taskId"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or(Error::Invalid("task id"))?;
                self.finish_thought(now)?;
                let status = match task["status"].as_str().unwrap_or_default() {
                    "queued" | "running" | "waiting" => "running",
                    "failed" | "interrupted" => "error",
                    _ => "done",
                };
                let summary = task["summary"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| task["progress"].as_str().unwrap_or_default());
                let output = serde_json::from_str::<Value>(summary)
                    .ok()
                    .filter(|v| v["summary"].is_string())
                    .map(|v| {
                        std::iter::once(v["summary"].as_str().unwrap().to_owned())
                            .chain(
                                v["notes"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter_map(Value::as_str)
                                    .map(str::to_owned),
                            )
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    })
                    .unwrap_or_else(|| summary.into());
                self.rows.upsert(id.into(), json!({"kind":"tool","id":id,"toolName":"delegate_task","label":task["title"],"status":status,"task":task,"output":output,"startedAt":task["startedAt"],"endedAt":task["endedAt"]}))?;
            }
            _ => return Err(Error::Invalid("unsupported trace event")),
        }
        Ok(())
    }
    pub fn close_held_questions(&mut self) -> Result<()> {
        let ids: Vec<_> = self
            .rows
            .order
            .iter()
            .filter(|r| {
                r.value["kind"] == "question"
                    && r.value["transport"] == "reply"
                    && r.value["status"] == "open"
            })
            .map(|r| r.id.clone())
            .collect();
        for id in ids {
            self.rows.update(&id, |v| v["status"] = json!("closed"))?;
        }
        Ok(())
    }
    pub fn seal(&mut self, now: u64) -> Result<()> {
        self.finish_thought(now)?;
        for (id, raw) in &self.answer_sources {
            let split = crate::placement::split(raw, false);
            self.rows.update(id, |v| v["text"] = json!(split.text))?;
        }
        let ids: Vec<_> = self
            .rows
            .order
            .iter()
            .filter(|r| r.value["status"] == "running")
            .map(|r| r.id.clone())
            .collect();
        for id in ids {
            self.rows.update(&id, |v| {
                v["status"] = json!(if v["kind"] == "question" {
                    "closed"
                } else {
                    "continued"
                })
            })?;
        }
        Ok(())
    }
}
fn event_id(event: &Value) -> Result<&str> {
    event["itemId"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or(Error::Invalid("trace item id"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupKind {
    Text,
    Thought,
    Question,
    Tools,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkStatus {
    Running,
    Error,
    Partial,
    Done,
}
#[derive(Clone, Debug)]
pub struct Group {
    pub id: String,
    pub kind: GroupKind,
    pub rows: Range<usize>,
    pub tool_count: usize,
    pub task_count: usize,
    pub failed_count: usize,
    pub success_count: usize,
    pub status: WorkStatus,
}
/// A semantic snapshot index, built once on a worker, then paged without
/// materializing its rows. Source: activity-rows.ts:204–335. No labels/glyphs.
#[derive(Clone, Debug, Default)]
pub struct GroupIndex {
    groups: Vec<Group>,
    pub newest_active: Option<String>,
}
impl GroupIndex {
    pub fn build(rows: &Rows) -> Self {
        let mut out = Self::default();
        for (index, row) in rows.order.iter().enumerate() {
            let kind = match row.value["kind"].as_str() {
                Some("text") => GroupKind::Text,
                Some("thought") => GroupKind::Thought,
                Some("question") => GroupKind::Question,
                Some("tool") => GroupKind::Tools,
                _ => GroupKind::Unknown,
            };
            if matches!(kind, GroupKind::Text | GroupKind::Thought)
                && row.value[if kind == GroupKind::Text {
                    "text"
                } else {
                    "label"
                }]
                .as_str()
                .is_none_or(|s| s.trim().is_empty())
            {
                continue;
            }
            let merge = kind == GroupKind::Tools
                && out
                    .groups
                    .last()
                    .is_some_and(|g| g.kind == GroupKind::Tools);
            if !merge {
                out.groups.push(Group {
                    id: row.id.clone(),
                    kind,
                    rows: index..index,
                    tool_count: 0,
                    task_count: 0,
                    failed_count: 0,
                    success_count: 0,
                    status: WorkStatus::Done,
                });
            }
            let group = out.groups.last_mut().expect("pushed");
            group.rows.end = index + 1;
            if kind == GroupKind::Tools {
                group.tool_count += 1;
                group.task_count += usize::from(row.value["task"].is_object());
                group.failed_count += usize::from(row.value["status"] == "error");
                group.success_count += usize::from(row.value["status"] == "done");
                if row.value["status"] == "running" || group.status == WorkStatus::Running {
                    group.status = WorkStatus::Running;
                } else if group.failed_count == group.tool_count {
                    group.status = WorkStatus::Error;
                } else if group.failed_count > 0 {
                    group.status = WorkStatus::Partial;
                }
            } else if kind == GroupKind::Thought && row.value["live"] == true {
                group.status = WorkStatus::Running;
            }
        }
        out.newest_active = out
            .groups
            .iter()
            .rev()
            .find(|g| g.status == WorkStatus::Running)
            .map(|g| g.id.clone());
        out
    }
    pub fn len(&self) -> usize {
        self.groups.len()
    }
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }
    pub fn page(&self, range: Range<usize>) -> &[Group] {
        let start = range.start.min(self.len());
        &self.groups[start..range.end.max(start).min(self.len())]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptRef {
    /// Scoped identity remains unique even for duplicate ids in legacy traces.
    pub id: String,
    pub message: String,
    pub step_index: Option<usize>,
}
/// Flattened transcript snapshot. Build after structural conversation changes
/// on a worker; keep the original Rows/Arcs alive. Paging/resolution never clones
/// a whole trace just to show one old tool/question among 60k archived steps.
#[derive(Clone, Debug, Default)]
pub struct TranscriptIndex {
    rows: Vec<TranscriptRef>,
    ids: HashMap<String, usize>,
}
impl TranscriptIndex {
    pub fn build(messages: &Rows) -> Self {
        let mut out = Self::default();
        for message in &messages.order {
            if message.value["role"] == "trace"
                && let Some(steps) = message.value["steps"].as_array()
            {
                for (index, step) in steps.iter().enumerate() {
                    let id = format!(
                        "{}/step-{index}:{}",
                        message.id,
                        step["id"].as_str().unwrap_or_default()
                    );
                    out.ids.insert(id.clone(), out.rows.len());
                    out.rows.push(TranscriptRef {
                        id,
                        message: message.id.clone(),
                        step_index: Some(index),
                    });
                }
            } else {
                out.ids.insert(message.id.clone(), out.rows.len());
                out.rows.push(TranscriptRef {
                    id: message.id.clone(),
                    message: message.id.clone(),
                    step_index: None,
                });
            }
        }
        out
    }
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub fn get(&self, id: &str) -> Option<&TranscriptRef> {
        self.ids.get(id).map(|i| &self.rows[*i])
    }
    pub fn page(&self, range: Range<usize>) -> &[TranscriptRef] {
        let start = range.start.min(self.len());
        &self.rows[start..range.end.max(start).min(self.len())]
    }
    pub fn resolve<'a>(&self, row: &TranscriptRef, messages: &'a Rows) -> Option<&'a Value> {
        let message = messages.get(&row.message)?;
        match row.step_index {
            Some(index) => message.value["steps"].as_array()?.get(index),
            None => Some(message.value.as_ref()),
        }
    }
}

/// Source: shared/media-tools.ts:278–290. A successful transcription tool call
/// may still be pending, and model-required is a recoverable outcome, not a
/// completed transcript or an unexplained failure. Keep labels with the view.
pub fn media_outcome(tool: &str, output: &Value) -> Option<Value> {
    let tool = tool.trim().to_lowercase();
    let short = if tool.starts_with("mcp__") {
        tool.rsplit("__").next()?
    } else {
        &tool
    };
    if short != "transcribe_media" {
        return None;
    }
    let text = output.as_str()?;
    let (prefix, tail) = text.split_once('{')?;
    if prefix.encode_utf16().count() > 80 {
        return None;
    }
    let tail = tail
        .trim_start()
        .strip_prefix("\"status\"")?
        .trim_start()
        .strip_prefix(':')?
        .trim_start();
    let status = tail.strip_prefix('"')?.split('"').next()?;
    if status == "model-required" {
        return Some(json!({"state":"needs-model"}));
    }
    if status != "transcribing" {
        return None;
    }
    let mut value = json!({"state":"pending"});
    if let Some((_, tail)) = text.split_once("\"progress\"")
        && let Some(tail) = tail.trim_start().strip_prefix(':')
    {
        let tail = tail.trim_start();
        let mut dot = false;
        let mut count = 0;
        let number: String = tail
            .chars()
            .take_while(|c| {
                if c.is_ascii_digit() {
                    count += 1;
                    true
                } else if *c == '.' && !dot {
                    dot = true;
                    true
                } else {
                    false
                }
            })
            .collect();
        if count > 0
            && let Ok(progress) = number.parse::<f64>()
            && progress > 0.0
            && progress <= 1.0
        {
            value["progress"] = json!(progress);
        }
    }
    Some(value)
}
