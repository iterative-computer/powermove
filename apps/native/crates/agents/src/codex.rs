//! Multiplexed App Server state machine. No IO, clocks or UI dependencies.
use crate::{
    Access, Effects, Event, RunResult,
    protocol::{initialize, method_is, normalized_type},
    text,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const CONTEXT_EXHAUSTED: &str = "This conversation is too long for the model, even after compacting it. Start a new thread to continue.";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    pub cwd: String,
    pub prompt: String,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub schema: Option<Value>,
    #[serde(default)]
    pub image_paths: Vec<String>,
    #[serde(default = "empty_config")]
    pub config: Value,
    /// Explicit resume is for a caller which has NOT already supplied history.
    pub resume_thread: Option<String>,
}
fn empty_config() -> Value {
    json!({"mcp_servers":{}})
}
#[derive(Clone)]
struct Turn {
    run: Run,
    thread: Option<String>,
    turn: Option<String>,
    final_text: String,
    streamed: BTreeSet<String>,
    unscoped: bool,
    question_messages: BTreeSet<String>,
    questions: BTreeMap<String, Value>,
    queued: VecDeque<Value>,
    reconnecting: bool,
    cancelled: bool,
    reported: bool,
    deadline: u64,
}
#[derive(Clone, Debug)]
enum Purpose {
    Initialize,
    Thread(String),
    Turn(String),
    Interrupt(String),
    Other,
}
struct Pending {
    purpose: Purpose,
    method: String,
    deadline: u64,
}
#[derive(Default)]
pub struct Client {
    ready: bool,
    started: bool,
    next: u64,
    pending: BTreeMap<u64, Pending>,
    turns: BTreeMap<String, Turn>,
}
impl Client {
    pub fn start(&mut self, now: u64) -> Effects {
        let mut e = Effects::default();
        if !self.started {
            self.started = true;
            self.request("initialize", initialize(), Purpose::Initialize, now, &mut e);
        }
        e
    }
    fn request(
        &mut self,
        method: &str,
        params: Value,
        purpose: Purpose,
        now: u64,
        e: &mut Effects,
    ) {
        self.next += 1;
        self.pending.insert(
            self.next,
            Pending {
                purpose,
                method: method.into(),
                deadline: now.saturating_add(30_000),
            },
        );
        e.outgoing
            .push(json!({"method":method,"id":self.next,"params":params}));
    }
    pub fn run(&mut self, run: Run, now: u64) -> Result<Effects, String> {
        if !crate::valid_id(&run.id, 200) {
            return Err("Invalid run id".into());
        }
        if self.turns.contains_key(&run.id) {
            return Err(format!("A Codex run with id {} is already active.", run.id));
        }
        let id = run.id.clone();
        self.turns.insert(
            id.clone(),
            Turn {
                run,
                thread: None,
                turn: None,
                final_text: String::new(),
                streamed: BTreeSet::new(),
                unscoped: false,
                question_messages: BTreeSet::new(),
                questions: BTreeMap::new(),
                queued: VecDeque::new(),
                reconnecting: false,
                cancelled: false,
                reported: false,
                deadline: now.saturating_add(3_600_000),
            },
        );
        let mut e = self.start(now);
        if self.ready {
            self.start_thread(&id, now, &mut e);
        }
        Ok(e)
    }
    fn start_thread(&mut self, id: &str, now: u64, e: &mut Effects) {
        let run = &self.turns[id].run;
        let mut p = json!({"cwd":run.cwd,"approvalPolicy":"never","sandbox":"read-only","ephemeral":true,"config":run.config,"serviceName":"powermove"});
        if let Some(m) = &run.model {
            p["model"] = json!(m);
        }
        let method = if let Some(thread) = &run.resume_thread {
            p["threadId"] = json!(thread);
            "thread/resume"
        } else {
            "thread/start"
        };
        self.request(method, p, Purpose::Thread(id.into()), now, e);
    }
    fn input(prompt: &str, images: &[String]) -> Value {
        let mut input = vec![json!({"type":"text","text":prompt})];
        input.extend(images.iter().map(|p| json!({"type":"localImage","path":p})));
        json!(input)
    }
    fn start_turn(&mut self, id: &str, now: u64, e: &mut Effects) {
        let t = &self.turns[id];
        let mut p = json!({"threadId":t.thread,"input":Self::input(&t.run.prompt,&t.run.image_paths),"cwd":t.run.cwd,"approvalPolicy":"never","sandboxPolicy":{"type":"readOnly"},"outputSchema":t.run.schema.as_ref().unwrap_or(&json!({"type":"object"}))});
        if let Some(m) = &t.run.model {
            p["model"] = json!(m);
        }
        if let Some(m) = &t.run.reasoning_effort {
            p["effort"] = json!(m);
        }
        self.request("turn/start", p, Purpose::Turn(id.into()), now, e);
    }
    pub fn steer(
        &mut self,
        id: &str,
        prompt: &str,
        images: &[String],
        now: u64,
    ) -> Result<Effects, String> {
        let t = self.turns.get_mut(id).ok_or("Unknown active run")?;
        if t.cancelled {
            return Err("The Codex run was cancelled.".into());
        }
        let input = Self::input(prompt, images);
        let mut e = Effects::default();
        if t.turn.is_none() {
            t.queued.push_back(input);
        } else {
            let p = json!({"threadId":t.thread,"expectedTurnId":t.turn,"input":input});
            self.request("turn/steer", p, Purpose::Other, now, &mut e);
        }
        Ok(e)
    }
    pub fn answer(
        &mut self,
        id: &str,
        item: &str,
        answers: &BTreeMap<String, Vec<String>>,
    ) -> Option<Effects> {
        let rpc = self.turns.get_mut(id)?.questions.remove(item)?;
        let answers: serde_json::Map<_, _> = answers
            .iter()
            .map(|(k, v)| (k.clone(), json!({"answers":v})))
            .collect();
        Some(Effects {
            outgoing: vec![json!({"id":rpc,"result":{"answers":answers}})],
            ..Effects::default()
        })
    }
    pub fn cancel(&mut self, id: &str, now: u64) -> Option<Effects> {
        let t = self.turns.get_mut(id)?;
        if t.cancelled {
            return Some(Effects::default());
        }
        t.cancelled = true;
        t.queued.clear();
        let mut e = Effects::default();
        for rpc in std::mem::take(&mut t.questions).into_values() {
            e.outgoing.push(json!({"id":rpc,"result":{"answers":{}}}));
        }
        if t.turn.is_some() {
            // Cancellation is complete when interruption settles, not when the
            // request enters our writer queue. No UI thread waits for that RPC.
            let p = json!({"threadId":t.thread,"turnId":t.turn});
            self.request(
                "turn/interrupt",
                p,
                Purpose::Interrupt(id.into()),
                now,
                &mut e,
            );
        } else if t.thread.is_some() {
            // turn/start is in flight. Report once now, but retain the turn as a
            // cleanup tombstone so its eventual identity can be interrupted.
            t.reported = true;
            e.events.push(Event::Completed {
                run: id.into(),
                result: cancelled(),
            });
        } else {
            self.finish(id, cancelled(), &mut e);
        }
        Some(e)
    }
    fn finish(&mut self, id: &str, result: RunResult, e: &mut Effects) {
        if let Some(t) = self.turns.remove(id) {
            for rpc in t.questions.into_values() {
                e.outgoing.push(json!({"id":rpc,"result":{"answers":{}}}));
            }
            if !t.reported {
                e.events.push(Event::Completed {
                    run: id.into(),
                    result,
                });
            }
        }
        // Drop run-owned requests at the same terminal boundary.
        self.pending.retain(|_, p| match &p.purpose {
            Purpose::Initialize | Purpose::Other => true,
            Purpose::Thread(r) | Purpose::Turn(r) | Purpose::Interrupt(r) => r != id,
        });
    }
    pub fn stopped(&mut self, reason: &str) -> Effects {
        let mut e = Effects::default();
        for id in self.turns.keys().cloned().collect::<Vec<_>>() {
            self.finish(&id, RunResult::failure(reason, false), &mut e);
        }
        self.pending.clear();
        self.ready = false;
        self.started = false;
        e.events.push(Event::Stopped(reason.into()));
        e
    }
    pub fn tick(&mut self, now: u64) -> Effects {
        let mut e = Effects::default();
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, p)| now >= p.deadline)
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            if let Some(p) = self.pending.remove(&id) {
                let msg = format!("Codex App Server timed out while handling {}.", p.method);
                match p.purpose {
                    Purpose::Initialize => {
                        e.append(self.stopped(&msg));
                    }
                    Purpose::Thread(r) | Purpose::Turn(r) => {
                        self.finish(&r, RunResult::failure(msg, false), &mut e)
                    }
                    Purpose::Interrupt(r) => self.finish(&r, cancelled(), &mut e),
                    Purpose::Other => e.events.push(Event::Warning(msg)),
                }
            }
        }
        for id in self
            .turns
            .iter()
            .filter(|(_, t)| now >= t.deadline)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>()
        {
            if let Some(t) = self.turns.get(&id)
                && t.turn.is_some()
            {
                let p = json!({"threadId":t.thread,"turnId":t.turn});
                self.request("turn/interrupt", p, Purpose::Other, now, &mut e);
            }
            self.finish(
                &id,
                RunResult::failure("The coding agent took too long to respond.", false),
                &mut e,
            );
        }
        e
    }
    pub fn receive(&mut self, msg: Value, now: u64) -> Effects {
        let mut e = Effects::default();
        // Server requests take precedence over response IDs, which may collide.
        if (msg["id"].is_number()
            || msg["id"]
                .as_str()
                .is_some_and(|s| !s.is_empty() && s.len() <= 200))
            && msg["method"].is_string()
        {
            self.server_request(&msg, &mut e);
            return e;
        }
        if let Some(id) = msg["id"].as_u64() {
            let Some(p) = self.pending.remove(&id) else {
                return e;
            };
            if msg["error"].is_object() {
                let err = app_error(&msg, "Codex App Server request failed.");
                match p.purpose {
                    Purpose::Initialize => e.append(self.stopped(&err)),
                    Purpose::Thread(r) | Purpose::Turn(r) => {
                        self.finish(&r, RunResult::failure(err, false), &mut e)
                    }
                    Purpose::Interrupt(r) => self.finish(&r, cancelled(), &mut e),
                    Purpose::Other => e.events.push(Event::Warning(err)),
                }
                return e;
            }
            match p.purpose {
                Purpose::Initialize => {
                    self.ready = true;
                    e.outgoing.push(json!({"method":"initialized","params":{}}));
                    e.events.push(Event::Ready);
                    for r in self.turns.keys().cloned().collect::<Vec<_>>() {
                        self.start_thread(&r, now, &mut e);
                    }
                }
                Purpose::Thread(r) => {
                    if let Some(thread) = msg["result"]["thread"]["id"]
                        .as_str()
                        .filter(|s| !s.is_empty() && s.len() <= 200)
                    {
                        if let Some(t) = self.turns.get_mut(&r) {
                            t.thread = Some(thread.into());
                            e.events.push(Event::Session {
                                run: r.clone(),
                                id: thread.into(),
                            });
                            self.start_turn(&r, now, &mut e);
                        }
                    } else {
                        self.finish(
                            &r,
                            RunResult::failure(
                                "Codex App Server did not return a thread id.",
                                false,
                            ),
                            &mut e,
                        );
                    }
                }
                Purpose::Turn(r) => {
                    if let Some(turn) = msg["result"]["turn"]["id"]
                        .as_str()
                        .filter(|s| !s.is_empty() && s.len() <= 200)
                    {
                        if let Some(t) = self.turns.get_mut(&r) {
                            t.turn = Some(turn.into());
                            if t.cancelled {
                                let p = json!({"threadId":t.thread,"turnId":turn});
                                self.request(
                                    "turn/interrupt",
                                    p,
                                    Purpose::Interrupt(r.clone()),
                                    now,
                                    &mut e,
                                );
                            } else {
                                let thread = t.thread.clone();
                                let queued = std::mem::take(&mut t.queued);
                                for input in queued {
                                    self.request("turn/steer",json!({"threadId":thread,"expectedTurnId":turn,"input":input}),Purpose::Other,now,&mut e);
                                }
                            }
                        }
                    } else {
                        self.finish(
                            &r,
                            RunResult::failure("Codex App Server did not return a turn id.", false),
                            &mut e,
                        );
                    }
                }
                Purpose::Interrupt(r) => self.finish(&r, cancelled(), &mut e),
                Purpose::Other => {}
            }
            return e;
        }
        self.notification(&msg, &mut e);
        e
    }
    fn find(&self, p: &Value) -> Option<String> {
        let thread = p["threadId"].as_str()?;
        let turn = p["turnId"].as_str().or_else(|| p["turn"]["id"].as_str());
        self.turns
            .iter()
            .find(|(_, t)| {
                !t.cancelled
                    && t.thread.as_deref() == Some(thread)
                    && (turn.is_none() || t.turn.is_none() || t.turn.as_deref() == turn)
            })
            .map(|(id, _)| id.clone())
    }
    fn server_request(&mut self, msg: &Value, e: &mut Effects) {
        let m = msg["method"].as_str().unwrap_or("");
        let p = &msg["params"];
        let rpc = &msg["id"];
        if method_is(m, "item/tool/requestUserInput") {
            let questions = reply_questions(&p["questions"]);
            let item = text(p, "itemId");
            if let Some(r) = self
                .find(p)
                .filter(|_| !item.is_empty() && !questions.is_empty())
            {
                if let Some(old) = self
                    .turns
                    .get_mut(&r)
                    .unwrap()
                    .questions
                    .insert(item.clone(), rpc.clone())
                {
                    e.outgoing.push(json!({"id":old,"result":{"answers":{}}}));
                }
                e.trace(&r,json!({"kind":"question","itemId":item,"questions":questions,"transport":"reply","blocking":p["isBlocking"]!=false}));
            } else {
                e.outgoing.push(json!({"id":rpc,"result":{"answers":{}}}));
            }
        } else if method_is(m, "mcpServer/elicitation/request") {
            e.outgoing
                .push(json!({"id":rpc,"result":{"action":"decline","content":null,"_meta":null}}));
        } else {
            e.outgoing.push(json!({"id":rpc,"error":{"code":-32601,"message":format!("Powermove does not handle {m}.")}}));
        }
    }
    fn notification(&mut self, msg: &Value, e: &mut Effects) {
        let m = msg["method"].as_str().unwrap_or("");
        let p = &msg["params"];
        if method_is(m, "warning") {
            let message = text(p, "message")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            for (id, t) in &mut self.turns {
                if t.cancelled {
                    continue;
                }
                if p["threadId"]
                    .as_str()
                    .is_none_or(|th| t.thread.as_deref() == Some(th))
                {
                    if connection_retry(&message) {
                        if !t.reconnecting {
                            e.progress(id, "Connection interrupted. Reconnecting…");
                        }
                        t.reconnecting = true;
                    } else {
                        e.progress(id, message.clone());
                    }
                }
            }
            return;
        }
        let Some(id) = self.find(p) else {
            e.events.push(Event::Notification(msg.clone()));
            return;
        };
        let t = self.turns.get_mut(&id).unwrap();
        if method_is(m, "serverRequest/resolved") {
            let items = t
                .questions
                .iter()
                .filter(|(_, v)| **v == p["requestId"])
                .map(|(k, _)| k.clone())
                .collect::<Vec<_>>();
            for item in items {
                t.questions.remove(&item);
                e.trace(&id, json!({"kind":"question-closed","itemId":item}));
            }
            return;
        }
        if p["turnId"].as_str().is_none()
            && !(method_is(m, "turn/completed") && p["turn"]["id"].is_string())
        {
            e.events.push(Event::Notification(msg.clone()));
            return;
        }
        if method_is(m, "error") {
            if p["willRetry"] == true {
                if !t.reconnecting {
                    e.progress(&id, "Connection interrupted. Reconnecting…");
                }
                t.reconnecting = true;
            }
            return;
        }
        if method_is(m, "model/rerouted") {
            e.progress(&id, format!("Continuing on {}", text(p, "toModel")));
            return;
        }
        let resumed = ((method_is(m, "item/started") || method_is(m, "item/completed"))
            && normalized_type(&p["item"]["type"]) != "error")
            || (method_is(m, "turn/completed") && p["turn"]["status"] == "completed")
            || (m.contains("Delta") || m.ends_with("/delta"))
                && p["delta"].as_str().is_some_and(|s| !s.is_empty());
        if t.reconnecting && resumed {
            t.reconnecting = false;
            e.progress(&id, "Connected. Continuing…");
        }
        if method_is(m, "item/agentMessage/delta") {
            let item = text(p, "itemId");
            if t.question_messages.contains(&item) {
                return;
            }
            if item.is_empty() {
                t.unscoped = true;
            } else {
                t.streamed.insert(item);
            }
            if p["delta"].is_string() {
                e.trace(
                    &id,
                    json!({"kind":"answer","text":crate::trace::fragment(&p["delta"])}),
                );
            }
            return;
        }
        if method_is(m, "item/reasoning/textDelta")
            || method_is(m, "item/reasoning/summaryTextDelta")
        {
            e.trace(
                &id,
                json!({"kind":"thought","text":crate::trace::fragment(&p["delta"])}),
            );
            if method_is(m, "item/reasoning/summaryTextDelta") {
                e.progress(
                    &id,
                    text(p, "delta")
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
            return;
        }
        if method_is(m, "item/started") || method_is(m, "item/completed") {
            let item = &p["item"];
            let ty = normalized_type(&item["type"]);
            let iid = text(item, "id");
            if ty == "agentmessage" {
                let questions = message_questions(&iid, &item["questions"]);
                if !questions.is_empty() {
                    if t.question_messages.insert(iid.clone()) {
                        e.trace(&id,json!({"kind":"question","itemId":iid,"questions":questions,"transport":"message","blocking":false}));
                    }
                    return;
                }
                if method_is(m, "item/completed") && item["text"].is_string() {
                    if item["delivery"] != "async" {
                        t.final_text = text(item, "text");
                    }
                    if !t.unscoped && !t.streamed.contains(&iid) {
                        e.trace(
                            &id,
                            json!({"kind":"answer","text":crate::trace::fragment(&item["text"])}),
                        );
                    }
                }
            } else if let Some(trace) = tool_trace(item, method_is(m, "item/completed")) {
                e.trace(&id, trace);
            }
            return;
        }
        if method_is(m, "turn/completed") {
            let result = match p["turn"]["status"].as_str() {
                Some("completed") if !t.final_text.is_empty() => {
                    RunResult::success(t.final_text.clone(), Access::Editor)
                }
                Some("completed") => {
                    RunResult::failure("ChatGPT generation completed without a result.", false)
                }
                Some("interrupted") => RunResult::failure("The Codex run was cancelled.", true),
                _ => RunResult::failure(app_error(&p["turn"], "ChatGPT generation failed."), false),
            };
            self.finish(&id, result, e);
            return;
        }
        e.events.push(Event::Notification(msg.clone()));
    }
}
fn app_error(v: &Value, fallback: &str) -> String {
    if v["error"]["codexErrorInfo"] == "contextWindowExceeded" {
        CONTEXT_EXHAUSTED.into()
    } else {
        v["error"]["message"].as_str().unwrap_or(fallback).into()
    }
}
pub(crate) fn clip(v: &Value, limit: usize) -> String {
    let clean = v
        .as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| !matches!(*c,'\0'..='\u{8}'|'\u{b}'..='\u{1f}'|'\u{7f}'))
        .collect::<String>();
    crate::trace::truncate(clean.trim(), limit)
}
fn connection_retry(message: &str) -> bool {
    let m = message.trim_start().to_lowercase();
    m.starts_with("reconnecting...")
        || m.starts_with("reconnecting…")
        || m.starts_with("falling back from websockets to https transport.")
}
fn options(v: &Value, strings: bool) -> Vec<Value> {
    v.as_array().into_iter().flatten().take(8).filter_map(|o| { let label=clip(if strings {o}else{&o["label"]},160); if label.is_empty(){None}else{Some(json!({"label":label,"description":if strings {String::new()} else {clip(&o["description"],400)}}))} }).collect()
}
fn reply_questions(v: &Value) -> Vec<Value> {
    v.as_array().into_iter().flatten().take(6).filter_map(|q| { let question=clip(&q["question"],1000); if question.is_empty() || !q["id"].is_string(){None}else{Some(json!({"id":q["id"],"header":clip(&q["header"],80),"question":question,"options":options(&q["options"],false),"allowOther":q["isOther"]==true || options(&q["options"],false).is_empty(),"secret":q["isSecret"]==true}))} }).collect()
}
fn message_questions(item: &str, v: &Value) -> Vec<Value> {
    v.as_array().into_iter().flatten().take(6).enumerate().filter_map(|(i,q)| { let question=clip(&q["title"],1000); if question.is_empty(){None}else{Some(json!({"id":format!("{item}-{i}"),"header":"","question":question,"options":options(&q["options"],true),"allowOther":true,"secret":false}))} }).collect()
}
pub(crate) fn tool_trace(item: &Value, end: bool) -> Option<Value> {
    use crate::trace;
    let id = trace::truncate(item["id"].as_str()?, 120);
    if id.is_empty() {
        return None;
    }
    let ty = normalized_type(&item["type"]);
    let files = item["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let s = c.as_str().or_else(|| c["path"].as_str()).unwrap_or("");
            s.chars()
                .filter(|c| !c.is_control())
                .collect::<String>()
                .split(['\\', '/'])
                .rfind(|s| !s.is_empty())
                .unwrap_or("")
                .to_owned()
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    let (name, label, detail) = match ty.as_str() {
        "contextcompaction" => ("compact".into(), "Compacting context".into(), String::new()),
        "commandexecution" => (
            "bash".into(),
            "Run".into(),
            trace::detail("Bash", &json!({"command":item["command"]})),
        ),
        "filechange" => (
            "edit".into(),
            "Edit".into(),
            trace::detail("Edit", &json!({"file_path":files.join(", ")})),
        ),
        "websearch" => (
            "search".into(),
            "Search".into(),
            trace::detail("WebSearch", &json!({"query":item["query"]})),
        ),
        "imagegeneration" => ("image".into(), "Image".into(), String::new()),
        "computeruse" => ("computer".into(), "Computer".into(), String::new()),
        "imageview" => (
            "view_image".into(),
            "View image".into(),
            text(item, "path")
                .split(['\\', '/'])
                .next_back()
                .unwrap_or("")
                .into(),
        ),
        "sleep" => (
            "wait".into(),
            "Wait".into(),
            item["durationMs"]
                .as_f64()
                .filter(|n| *n >= 500.0)
                .map(|n| format!("{}s", (n / 1000.0).round()))
                .unwrap_or_default(),
        ),
        "collabagenttoolcall" | "collabtoolcall" => {
            let label = match item["tool"].as_str().unwrap_or("") {
                "spawnAgent" => "Start subagent",
                "sendInput" | "sendMessage" | "followupTask" | "resumeAgent" => "Message subagent",
                "wait" => "Wait for subagents",
                "closeAgent" => "Close subagent",
                "interruptAgent" => "Stop subagent",
                "listAgents" => "List subagents",
                _ => "Subagent",
            };
            (
                "agent".into(),
                label.into(),
                trace::detail("Task", &json!({"description":item["prompt"]})),
            )
        }
        "mcptoolcall" | "dynamictoolcall" => {
            let tool = item["tool"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("mcp");
            (
                trace::truncate(tool, 120),
                trace::label(tool),
                trace::detail(tool, &json!({"tool":tool})),
            )
        }
        _ => return None,
    };
    if !end {
        let mut v = json!({"kind":"tool-start","itemId":id,"toolName":name,"label":label});
        if !detail.is_empty() {
            v["detail"] = json!(detail);
        }
        if let Some(subject) = trace::subject(&name, &item["arguments"]) {
            v["subject"] = subject;
        }
        return Some(v);
    }
    let failed = matches!(
        item["status"].as_str(),
        Some("failed" | "error" | "declined")
    );
    let raw = if failed {
        if item["error"].is_null() {
            item["status"].clone()
        } else {
            item["error"].clone()
        }
    } else {
        match ty.as_str() {
            "contextcompaction" => json!("Earlier context summarized"),
            "filechange" => json!(if files.len() > 3 {
                format!("{} files changed", files.len())
            } else {
                files.join(", ")
            }),
            "websearch" => item["query"].clone(),
            "imageview" | "sleep" | "collabagenttoolcall" | "collabtoolcall" => Value::Null,
            "dynamictoolcall" => json!(
                item["contentItems"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            _ => item
                .get("aggregatedOutput")
                .or_else(|| item.get("aggregated_output"))
                .or_else(|| item.get("result"))
                .or_else(|| item.get("output"))
                .cloned()
                .unwrap_or(Value::Null),
        }
    };
    let output = trace::excerpt(&raw);
    let mut v = json!({"kind":"tool-end","itemId":id,"isError":failed});
    if !output.is_empty() {
        v["output"] = json!(output);
    }
    Some(v)
}

fn cancelled() -> RunResult {
    RunResult::failure("The Codex run was cancelled.", true)
}
