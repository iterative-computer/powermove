//! Durable, provider-independent task reservations and delivery barriers.
//! Invoke on the coordinator worker; Host storage is serialized by the kernel
//! actor. Returned commands are work to perform, never claimed capabilities.
use crate::{
    Provider, RunResult, error,
    monitor::{Monitor, Status, Watch},
    trace,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::PathBuf,
};

#[derive(Clone, Debug)]
pub struct Selection {
    pub provider: Provider,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}
pub trait Host {
    fn load(&mut self, path: &std::path::Path) -> io::Result<Option<Value>>;
    /// Compare the current archive to expected on the actor, then acknowledge
    /// durable publication. A stale scope must fail without overwriting work.
    /// The application also holds one execution lease per conversation scope.
    fn save(
        &mut self,
        path: &std::path::Path,
        expected: Option<&Value>,
        archive: &Value,
    ) -> io::Result<()>;
    /// Use the actual configured provider/model catalog. Missing providers must
    /// fail; this library does not invent a catalog or run an unavailable model.
    fn select(&mut self, parent: &Value, target: Option<&Value>) -> io::Result<Selection>;
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Delegate {
    pub task: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Value>,
    pub client_request_id: Option<String>,
}
/// IDs are allocated by the host before admission; rejected/retried reservations
/// never consume another provider turn. IDs cannot alias active or saved work.
pub struct Ids {
    pub task: String,
    pub thread: String,
    pub run: String,
}
#[derive(Debug, PartialEq)]
pub enum Command {
    Start {
        request: Value,
        root: Value,
        role: Option<String>,
    },
    Cancel {
        run: String,
    },
    /// Results are untrusted task output. The host constructs the frozen
    /// continuation prompt and reuses the parent's existing workspace/session.
    Continue {
        delivery: String,
        run: String,
        tasks: Vec<Value>,
        watched: Vec<Value>,
    },
}
#[derive(Debug, PartialEq)]
pub enum Barrier {
    Complete,
    Waiting,
    Continue(Command),
}
pub struct Scope {
    path: PathBuf,
    root: Value,
    archive: Value,
    persisted: Option<Value>,
    requests: BTreeMap<String, Value>,
    stopped: BTreeSet<String>,
}
fn id(request: &Value, key: &str) -> io::Result<String> {
    request[key]
        .as_str()
        .filter(|s| crate::valid_id(s, 200))
        .map(str::to_owned)
        .ok_or_else(|| error(format!("Invalid {key}")))
}
fn records(archive: &Value) -> &[Value] {
    archive["records"].as_array().unwrap()
}
fn terminal(record: &Value) -> bool {
    matches!(
        record["task"]["status"].as_str(),
        Some("completed" | "failed" | "cancelled" | "interrupted")
    )
}
fn summary(result: &RunResult) -> (&str, &'static str) {
    match result {
        RunResult::Success { text, .. } => (text, "completed"),
        RunResult::Failure {
            error, cancelled, ..
        } => (error, if *cancelled { "cancelled" } else { "failed" }),
    }
}
impl Scope {
    pub fn load(host: &mut impl Host, root: Value, now: u64) -> io::Result<Self> {
        let project = id(&root, "projectId")?;
        let thread = id(&root, "threadId")?;
        let run = id(&root, "id")?;
        let hash = |s: &str| format!("{:x}", Sha256::digest(s.as_bytes()));
        let path = PathBuf::from("Agent Tasks")
            .join(hash(&project))
            .join(format!("{}.json", hash(&format!("{project}/{thread}"))));
        let mut persisted = host.load(&path)?;
        let mut archive = persisted
            .clone()
            .unwrap_or_else(|| json!({"version":1,"rootThreadId":thread,"records":[]}));
        validate(&archive)?;
        if archive.get("rootThreadId").is_some_and(|id| id != &thread) {
            return Err(error("Task archive belongs to another conversation."));
        }
        let stopped = if archive["nativeRootStop"] == run {
            BTreeSet::from([run.clone()])
        } else {
            BTreeSet::new()
        };
        let mut changed = false;
        for record in archive["records"].as_array_mut().unwrap() {
            if !terminal(record) {
                record["task"]["status"] = json!("interrupted");
                record["task"]["summary"] = json!(
                    "Powermove closed before this task finished. Delegate a new task to resume the work."
                );
                record["task"]["workState"] = json!("result_available");
                record["task"]["hasPendingChildRuns"] = json!(false);
                record["task"]["endedAt"] = json!(now);
                changed = true;
            }
        }
        if changed {
            host.save(&path, persisted.as_ref(), &archive)?;
            persisted = Some(archive.clone());
        }
        Ok(Self {
            path,
            root: root.clone(),
            archive,
            persisted,
            requests: BTreeMap::from([(run, root)]),
            stopped,
        })
    }
    pub fn raw(&self) -> &Value {
        &self.archive
    }
    /// The root or a task run of this scope.
    pub fn has_run(&self, run: &str) -> bool {
        self.requests.contains_key(run)
    }
    pub fn request(&self, run: &str) -> Option<&Value> {
        self.requests.get(run)
    }
    /// Replace a task run's request (task_send's new direction or settings).
    pub fn set_request(&mut self, run: &str, request: Value) {
        if let Some(current) = self.requests.get_mut(run) {
            *current = request;
        }
    }
    /// The task a run carries out (`None` for the root).
    pub fn task_of(&self, run: &str) -> Option<&Value> {
        records(&self.archive)
            .iter()
            .find(|r| r["task"]["childRunId"] == run)
            .map(|r| &r["task"])
    }
    /// The task `parent` owns, by id.
    pub fn owned(&self, parent: &str, task: &str) -> io::Result<&Value> {
        let index = self.owned_index(parent, task)?;
        Ok(&records(&self.archive)[index]["task"])
    }
    /// Live state (status, progress) shown on the task row; saved with the
    /// next publication. Terminal records never change.
    pub fn note(&mut self, run: &str, change: impl FnOnce(&mut Value)) -> Option<Value> {
        let record = self.archive["records"]
            .as_array_mut()?
            .iter_mut()
            .find(|r| r["task"]["childRunId"] == run && !terminal(r))?;
        change(&mut record["task"]);
        Some(record["task"].clone())
    }
    /// Whether any of this root's tasks still runs.
    pub fn working(&self) -> bool {
        records(&self.archive)
            .iter()
            .any(|r| r["task"]["requestId"] == self.root["id"] && !terminal(r))
    }
    /// The fingerprint a task_send `clientRequestId` was first used with.
    pub fn update_seen(&self, parent: &str, task: &str, key: &str) -> io::Result<Option<String>> {
        let index = self.owned_index(parent, task)?;
        Ok(records(&self.archive)[index]["updates"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|u| u["id"] == key)
            .and_then(|u| u["fingerprint"].as_str().map(str::to_owned)))
    }
    /// Durably record a delivered task_send (main's `updates`, at most 64).
    pub fn record_update(
        &mut self,
        host: &mut impl Host,
        parent: &str,
        task: &str,
        key: Option<(&str, &str)>,
        selection: &Selection,
    ) -> io::Result<Value> {
        let index = self.owned_index(parent, task)?;
        let mut archive = self.archive.clone();
        let record = &mut archive["records"][index];
        if let Some((id, fingerprint)) = key {
            if record.get("updates").is_none() {
                record["updates"] = json!([]);
            }
            let updates = record["updates"]
                .as_array_mut()
                .ok_or_else(|| error("Malformed task updates"))?;
            if updates.len() >= 64 {
                return Err(error(
                    "A task can receive up to 64 updates. Delegate a new task for more work.",
                ));
            }
            updates.push(json!({"id":id,"fingerprint":fingerprint}));
        }
        let task = &mut record["task"];
        task["providerInstanceId"] = json!(selection.provider);
        task["model"] = json!(selection.model);
        task["reasoningEffort"] = json!(selection.reasoning_effort);
        task["progress"] = json!("Updating the task with new direction…");
        let snapshot = task.clone();
        self.publish(host, archive)?;
        Ok(snapshot)
    }
    /// task_cancel acknowledges a result, terminal or not.
    pub fn acknowledge(
        &mut self,
        host: &mut impl Host,
        parent: &str,
        task: &str,
    ) -> io::Result<()> {
        let index = self.owned_index(parent, task)?;
        if records(&self.archive)[index]["acknowledged"] == true {
            return Ok(());
        }
        let mut archive = self.archive.clone();
        archive["records"][index]["acknowledged"] = json!(true);
        self.publish(host, archive)
    }
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
    fn publish(&mut self, host: &mut impl Host, archive: Value) -> io::Result<()> {
        validate(&archive)?;
        host.save(&self.path, self.persisted.as_ref(), &archive)?;
        self.persisted = Some(archive.clone());
        self.archive = archive;
        Ok(())
    }
    fn parent(&self, run: &str) -> io::Result<&Value> {
        if self.stopped.contains(run)
            || records(&self.archive)
                .iter()
                .any(|r| r["task"]["childRunId"] == run && terminal(r))
        {
            return Err(error("This agent run is no longer active."));
        }
        self.requests
            .get(run)
            .ok_or_else(|| error("This agent run is no longer active."))
    }
    fn owned_index(&self, parent: &str, task: &str) -> io::Result<usize> {
        let request = self.parent(parent)?;
        records(&self.archive)
            .iter()
            .position(|r| {
                r["task"]["taskId"] == task && r["task"]["parentThreadId"] == request["threadId"]
            })
            .ok_or_else(|| error("This task does not belong to the calling agent."))
    }
    pub fn delegate(
        &mut self,
        host: &mut impl Host,
        parent: &str,
        mut input: Delegate,
        ids: Ids,
        now: u64,
    ) -> io::Result<(Value, Option<Command>)> {
        let request = self.parent(parent)?.clone();
        if let Some(target) = &input.target {
            validate_target(target)?;
        }
        input.task = input.task.trim().into();
        if input.task.is_empty() || input.task.encode_utf16().count() > 200_000 {
            return Err(error("Invalid task brief"));
        }
        if let Some(title) = &mut input.title {
            *title = title.trim().into();
            if title.is_empty() || title.encode_utf16().count() > 160 {
                return Err(error("Invalid task title"));
            }
        }
        if input.role.as_deref().is_some_and(|r| {
            !matches!(
                r,
                "implementation" | "research" | "review" | "design" | "test" | "general"
            )
        }) {
            return Err(error("Invalid task role"));
        }
        if input
            .client_request_id
            .as_ref()
            .is_some_and(|key| key.is_empty() || key.encode_utf16().count() > 240)
        {
            return Err(error("Invalid clientRequestId"));
        }
        let mut fingerprint = json!({"task":input.task});
        for (key, value) in [
            ("title", input.title.as_ref().map(|v| json!(v))),
            ("role", input.role.as_ref().map(|v| json!(v))),
            ("target", input.target.clone()),
        ] {
            if let Some(value) = value {
                fingerprint[key] = value;
            }
        }
        let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&fingerprint)?));
        if let Some(key) = &input.client_request_id
            && let Some(previous) = records(&self.archive).iter().find(|r| {
                r["task"]["parentThreadId"] == request["threadId"] && r["clientRequestId"] == *key
            })
        {
            if previous["fingerprint"] != fingerprint {
                return Err(error(
                    "This clientRequestId already belongs to a different task. Use a new key for a new round.",
                ));
            }
            return Ok((previous["task"].clone(), None));
        }
        let mut ancestor = parent;
        let mut depth = 0;
        let mut parent_task = None;
        while let Some(record) = records(&self.archive)
            .iter()
            .find(|r| r["task"]["childRunId"] == ancestor)
        {
            depth += 1;
            if depth == 1 {
                parent_task = Some(record["task"]["taskId"].clone());
            }
            if depth >= 4 {
                return Err(error("Subagents can nest up to 4 levels."));
            }
            let parent_thread = &record["task"]["parentThreadId"];
            ancestor = self
                .requests
                .iter()
                .find(|(_, r)| &r["threadId"] == parent_thread)
                .map(|(id, _)| id.as_str())
                .ok_or_else(|| error("Missing parent run"))?;
        }
        let current = records(&self.archive)
            .iter()
            .filter(|r| r["task"]["requestId"] == self.root["id"])
            .collect::<Vec<_>>();
        if current.len() >= 32 {
            return Err(error("A run can delegate up to 32 tasks."));
        }
        if current.iter().filter(|r| !terminal(r)).count() >= 8 {
            return Err(error(
                "Up to 8 subagents can work at once. Wait for a task to finish before delegating another.",
            ));
        }
        if records(&self.archive).len() >= 512 {
            return Err(error("Archive task history before delegating more work."));
        }
        if [&ids.task, &ids.thread, &ids.run]
            .iter()
            .any(|s| !crate::valid_id(s, 120))
            || self
                .requests
                .values()
                .any(|r| r["id"] == ids.run || r["threadId"] == ids.thread)
            || records(&self.archive).iter().any(|r| {
                r["task"]["taskId"] == ids.task
                    || r["task"]["childRunId"] == ids.run
                    || r["task"]["childThreadId"] == ids.thread
            })
        {
            return Err(error("Duplicate or invalid task identity"));
        }
        let selection = host.select(&request, input.target.as_ref())?;
        let title = input.title.unwrap_or_else(|| {
            trace::truncate(
                &input.task.split_whitespace().collect::<Vec<_>>().join(" "),
                160,
            )
        });
        let task = json!({"taskId":ids.task,"parentThreadId":request["threadId"],"childThreadId":ids.thread,"childRunId":ids.run,"parentTaskId":parent_task,"requestId":self.root["id"],"title":title,"providerInstanceId":selection.provider,"model":selection.model,"reasoningEffort":selection.reasoning_effort,"status":"queued","workState":"working","hasPendingChildRuns":false,"summary":null,"progress":"","startedAt":now,"waitTimedOut":false});
        let mut record = json!({"task":task,"acknowledged":false,"fingerprint":fingerprint});
        if let Some(key) = input.client_request_id {
            record["clientRequestId"] = json!(key);
        }
        let mut archive = self.archive.clone();
        archive["records"].as_array_mut().unwrap().push(record);
        self.publish(host, archive)?;
        let mut child = request;
        child["id"] = json!(ids.run);
        child["threadId"] = json!(ids.thread);
        child["prompt"] = json!(input.task);
        child["provider"] = json!(selection.provider);
        child["model"] = json!(selection.model);
        child["reasoningEffort"] = json!(selection.reasoning_effort);
        self.requests.insert(ids.run, child.clone());
        Ok((
            task,
            Some(Command::Start {
                request: child,
                root: self.root.clone(),
                role: input.role,
            }),
        ))
    }
    /// A terminal result is immutable. Failed publication leaves the reservation
    /// unchanged so the worker can retry the same completion instead of losing it.
    pub fn finish(
        &mut self,
        host: &mut impl Host,
        run: &str,
        result: &RunResult,
        now: u64,
    ) -> io::Result<()> {
        let index = records(&self.archive)
            .iter()
            .position(|r| r["task"]["childRunId"] == run)
            .ok_or_else(|| error("Unknown task run"))?;
        if terminal(&records(&self.archive)[index]) {
            return Ok(());
        }
        let request = self
            .requests
            .get(run)
            .ok_or_else(|| error("Missing task request"))?;
        if records(&self.archive)
            .iter()
            .any(|r| r["task"]["parentThreadId"] == request["threadId"] && !terminal(r))
        {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Subagent children have not settled.",
            ));
        }
        let stopped = RunResult::failure("The task was stopped.", true);
        let (text, status) = summary(if self.stopped.contains(run) {
            &stopped
        } else {
            result
        });
        let mut archive = self.archive.clone();
        let task = &mut archive["records"][index]["task"];
        task["status"] = json!(status);
        task["summary"] = json!(trace::truncate(text, 200_000));
        task["endedAt"] = json!(now);
        task["workState"] = json!("result_available");
        task["hasPendingChildRuns"] = json!(false);
        self.publish(host, archive)
    }
    pub fn status(&mut self, host: &mut impl Host, parent: &str, task: &str) -> io::Result<Value> {
        let index = self.owned_index(parent, task)?;
        if terminal(&records(&self.archive)[index]) {
            let mut archive = self.archive.clone();
            archive["records"][index]["acknowledged"] = json!(true);
            self.publish(host, archive)?;
        }
        Ok(records(&self.archive)[index]["task"].clone())
    }
    /// Stop only descendants, never independent ordinary threads. Terminal
    /// completion remains pending until each provider actually acknowledges it.
    pub fn cancel(
        &mut self,
        host: &mut impl Host,
        parent: &str,
        task: &str,
    ) -> io::Result<Vec<Command>> {
        let index = self.owned_index(parent, task)?;
        let mut runs = BTreeSet::from([records(&self.archive)[index]["task"]["childRunId"]
            .as_str()
            .unwrap()
            .to_owned()]);
        loop {
            let old = runs.len();
            for r in records(&self.archive) {
                if self.requests.iter().any(|(id, req)| {
                    runs.contains(id) && req["threadId"] == r["task"]["parentThreadId"]
                }) {
                    runs.insert(r["task"]["childRunId"].as_str().unwrap().into());
                }
            }
            if runs.len() == old {
                break;
            }
        }
        let mut archive = self.archive.clone();
        let mut commands = vec![];
        for record in archive["records"].as_array_mut().unwrap() {
            let run = record["task"]["childRunId"].as_str().unwrap().to_owned();
            if runs.contains(&run) && !terminal(record) {
                record["acknowledged"] = json!(true);
                if !self.stopped.contains(&run) {
                    commands.push(Command::Cancel { run });
                }
            }
        }
        self.publish(host, archive)?;
        self.stopped.extend(runs);
        Ok(commands)
    }
    /// The person stopped a task from its row (main's `cancelTask`): it and
    /// its descendants stop, and its owner still receives the outcome.
    pub fn stop_task(&mut self, host: &mut impl Host, task: &str) -> io::Result<Vec<Command>> {
        let parent_thread = records(&self.archive)
            .iter()
            .find(|r| r["task"]["taskId"] == task)
            .map(|r| r["task"]["parentThreadId"].clone())
            .ok_or_else(|| error("This task does not belong to the active conversation."))?;
        let owner = self
            .requests
            .iter()
            .find(|(_, r)| r["threadId"] == parent_thread)
            .map(|(id, _)| id.clone())
            .ok_or_else(|| error("The agent that started this subagent has finished."))?;
        let commands = self.cancel(host, &owner, task)?;
        let index = self.owned_index(&owner, task)?;
        let mut archive = self.archive.clone();
        archive["records"][index]["acknowledged"] = json!(false);
        self.publish(host, archive)?;
        Ok(commands)
    }
    /// Run after a parent turn. A result is delivered once, after all children
    /// settle. Status reads acknowledge terminal tasks; timeouts never cancel.
    pub fn after_turn(&mut self, host: &mut impl Host, parent: &str) -> io::Result<Barrier> {
        self.after_turn_with_watches(host, parent, &Monitor::default(), &[])
    }
    pub fn after_turn_with_watches(
        &mut self,
        host: &mut impl Host,
        parent: &str,
        monitor: &Monitor,
        watches: &[Watch],
    ) -> io::Result<Barrier> {
        let request = self.parent(parent)?;
        if watches.iter().any(|w| {
            monitor.is_watching(w)
                && !monitor.owns_watch(self.root["projectId"].as_str().unwrap(), parent, w)
        }) {
            return Err(error("This watch does not belong to the calling agent."));
        }
        if let Some(pending) = self
            .archive
            .get("nativePendingDeliveries")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|v| v["run"] == parent && v["acknowledged"] == false)
        {
            return Ok(Barrier::Continue(Command::Continue {
                delivery: pending["id"]
                    .as_str()
                    .ok_or_else(|| error("Malformed delivery id"))?
                    .into(),
                run: parent.into(),
                watched: pending
                    .get("watched")
                    .cloned()
                    .unwrap_or_else(|| json!([]))
                    .as_array()
                    .ok_or_else(|| error("Malformed watched results"))?
                    .clone(),
                tasks: pending["tasks"]
                    .as_array()
                    .ok_or_else(|| error("Malformed delivery tasks"))?
                    .clone(),
            }));
        }
        let children = records(&self.archive)
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                r["task"]["parentThreadId"] == request["threadId"]
                    && r["task"]["requestId"] == self.root["id"]
            })
            .collect::<Vec<_>>();
        if children.iter().any(|(_, r)| !terminal(r))
            || watches
                .iter()
                .any(|w| monitor.is_watching(w) && monitor.result(w).is_none())
        {
            return Ok(Barrier::Waiting);
        }
        let tasks = children
            .into_iter()
            .filter(|(_, r)| r["acknowledged"] != true)
            .map(|(_, r)| r["task"].clone())
            .collect::<Vec<_>>();
        let watched = watches
            .iter()
            .filter_map(|w| monitor.result(w))
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()?;
        if tasks.is_empty() && watched.is_empty() {
            return Ok(Barrier::Complete);
        }
        let delivery = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(
                &json!({"run":parent,"tasks":tasks,"watched":watched})
            )?)
        );
        let mut archive = self.archive.clone();
        if archive.get("nativePendingDeliveries").is_none() {
            archive["nativePendingDeliveries"] = json!([]);
        }
        archive["nativePendingDeliveries"]
            .as_array_mut()
            .ok_or_else(|| error("Unknown delivery archive"))?
            .push(json!({"id":delivery,"run":parent,"tasks":tasks,"watched":watched,"acknowledged":false}));
        self.publish(host, archive)?;
        Ok(Barrier::Continue(Command::Continue {
            delivery,
            run: parent.into(),
            tasks,
            watched,
        }))
    }
    /// Persist a root stop before asking any provider to stop. Ordinary
    /// conversations are not in this scope and are never cancelled here.
    pub fn stop(&mut self, host: &mut impl Host) -> io::Result<Vec<Command>> {
        let root = id(&self.root, "id")?;
        let mut archive = self.archive.clone();
        archive["nativeRootStop"] = json!(root);
        let mut runs = vec![root];
        for record in archive["records"].as_array_mut().unwrap() {
            if record["task"]["requestId"] == self.root["id"] && !terminal(record) {
                record["acknowledged"] = json!(true);
                runs.push(record["task"]["childRunId"].as_str().unwrap().into());
            }
        }
        self.publish(host, archive)?;
        let commands = runs
            .iter()
            .filter(|run| !self.stopped.contains(*run))
            .cloned()
            .map(|run| Command::Cancel { run })
            .collect();
        self.stopped.extend(runs);
        Ok(commands)
    }
    /// Durably record that the host is handing `delivery` to `parent` (its
    /// continuation turn), before it starts that turn. `false` when it already
    /// was: the handoff happened, so the host must not start it again.
    pub fn accept_delivery(
        &mut self,
        host: &mut impl Host,
        parent: &str,
        delivery: &str,
    ) -> io::Result<bool> {
        self.parent(parent)?;
        let mut archive = self.archive.clone();
        let pending = archive["nativePendingDeliveries"]
            .as_array_mut()
            .ok_or_else(|| error("Unknown delivery"))?
            .iter_mut()
            .find(|d| d["id"] == delivery && d["run"] == parent)
            .ok_or_else(|| error("Unknown delivery"))?;
        if pending["accepted"] == true {
            return Ok(false);
        }
        pending["accepted"] = json!(true);
        self.publish(host, archive)?;
        Ok(true)
    }
    /// Release only the run-specific leases contained in a durably accepted
    /// continuation, leaving unrelated/new watches in place.
    pub fn acknowledge_with_watches(
        &mut self,
        host: &mut impl Host,
        parent: &str,
        delivery: &str,
        monitor: &mut Monitor,
        watches: &[Watch],
    ) -> io::Result<()> {
        if watches.iter().any(|w| {
            monitor.is_watching(w)
                && !monitor.owns_watch(self.root["projectId"].as_str().unwrap(), parent, w)
        }) {
            return Err(error("This watch does not belong to the calling agent."));
        }
        let pending = self.archive["nativePendingDeliveries"]
            .as_array()
            .and_then(|items| {
                items
                    .iter()
                    .find(|d| d["id"] == delivery && d["run"] == parent)
            })
            .ok_or_else(|| error("Unknown delivery"))?;
        let watched: Vec<Value> =
            serde_json::from_value(pending.get("watched").cloned().unwrap_or_else(|| json!([])))?;
        self.acknowledge_delivery(host, parent, delivery)?;
        for lease in watches {
            if monitor
                .result(lease)
                .is_some_and(|r| serde_json::to_value(r).is_ok_and(|r| watched.contains(&r)))
            {
                monitor.release(lease);
            }
        }
        Ok(())
    }
    /// Call after the application has durably accepted this delivery ID in the
    /// parent's history/outbox. Replaying an unacknowledged command is expected;
    /// the host deduplicates by ID rather than dropping a result after a crash.
    pub fn acknowledge_delivery(
        &mut self,
        host: &mut impl Host,
        parent: &str,
        delivery: &str,
    ) -> io::Result<()> {
        self.parent(parent)?;
        let mut archive = self.archive.clone();
        let pending = archive["nativePendingDeliveries"]
            .as_array_mut()
            .ok_or_else(|| error("Unknown delivery"))?
            .iter_mut()
            .find(|d| d["id"] == delivery && d["run"] == parent)
            .ok_or_else(|| error("Unknown delivery"))?;
        let tasks = pending["tasks"]
            .as_array()
            .ok_or_else(|| error("Malformed delivery"))?
            .clone();
        pending["acknowledged"] = json!(true);
        for record in archive["records"].as_array_mut().unwrap() {
            if tasks
                .iter()
                .any(|task| task["taskId"] == record["task"]["taskId"])
            {
                record["acknowledged"] = json!(true);
            }
        }
        self.publish(host, archive)
    }
}
/// Main's task_send fingerprint: `{message, target}` without absent fields.
pub fn update_fingerprint(message: Option<&str>, target: Option<&Value>) -> String {
    let mut value = json!({});
    if let Some(message) = message {
        value["message"] = json!(message);
    }
    if let Some(target) = target {
        value["target"] = target.clone();
    }
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}
fn validate(archive: &Value) -> io::Result<()> {
    if serde_json::to_vec(archive)?.len() > crate::persistence::MAX_STORE_BYTES {
        return Err(error("Task archive exceeds 32 MiB"));
    }
    if archive
        .get("nativePendingDeliveries")
        .is_some_and(|v| !v.is_array())
    {
        return Err(error("Unknown delivery archive; preserved unchanged"));
    }
    if let Some(stop) = archive.get("nativeRootStop")
        && !stop.as_str().is_some_and(|s| crate::valid_id(s, 200))
    {
        return Err(error("Unknown root stop record; preserved unchanged"));
    }
    if let Some(pending) = archive
        .get("nativePendingDeliveries")
        .and_then(Value::as_array)
    {
        let mut ids = BTreeSet::new();
        for delivery in pending {
            let delivery_id = delivery["id"]
                .as_str()
                .filter(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
                .ok_or_else(|| error("Malformed delivery id; preserved unchanged"))?;
            id(delivery, "run")?;
            if !ids.insert(delivery_id)
                || !delivery["acknowledged"].is_boolean()
                || !delivery["tasks"].is_array()
                || delivery.get("watched").is_some_and(|v| !v.is_array())
            {
                return Err(error("Malformed delivery; preserved unchanged"));
            }
        }
    }
    let Some(records) = archive["records"].as_array().filter(|r| r.len() <= 512) else {
        return Err(error(
            "Saved subagent tasks could not be read. Their file has been preserved.",
        ));
    };
    if archive["version"] != 1 || !archive.is_object() {
        return Err(error("Unknown task archive"));
    }
    let mut tasks = BTreeSet::new();
    let mut runs = BTreeSet::new();
    for record in records {
        let task = &record["task"];
        for key in [
            "taskId",
            "parentThreadId",
            "childThreadId",
            "childRunId",
            "requestId",
        ] {
            id(task, key)?;
        }
        if !tasks.insert(task["taskId"].as_str().unwrap())
            || !runs.insert(task["childRunId"].as_str().unwrap())
            || !record["acknowledged"].is_boolean()
            || !record["fingerprint"]
                .as_str()
                .is_some_and(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
            || serde_json::from_value::<Status>(task["status"].clone()).is_err()
        {
            return Err(error("Malformed task archive; preserved unchanged"));
        }
    }
    Ok(())
}

fn validate_target(target: &Value) -> io::Result<()> {
    let object = target
        .as_object()
        .ok_or_else(|| error("Invalid target settings"))?;
    if object
        .keys()
        .any(|k| !matches!(k.as_str(), "providerInstanceId" | "model" | "options"))
    {
        return Err(error("Unknown target setting"));
    }
    if object
        .get("providerInstanceId")
        .is_some_and(|v| serde_json::from_value::<Provider>(v.clone()).is_err())
    {
        return Err(error("Invalid target provider"));
    }
    if object.get("model").is_some_and(|v| {
        !v.as_str()
            .is_some_and(|s| !s.is_empty() && s.encode_utf16().count() <= 200)
    }) {
        return Err(error("Invalid target model"));
    }
    if let Some(options) = object.get("options") {
        let options = options
            .as_object()
            .ok_or_else(|| error("Invalid target options"))?;
        if options.keys().any(|k| k != "reasoningEffort")
            || options.get("reasoningEffort").is_some_and(|v| {
                !matches!(
                    v.as_str(),
                    Some("none" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra")
                )
            })
        {
            return Err(error("Invalid reasoning effort"));
        }
    }
    Ok(())
}
