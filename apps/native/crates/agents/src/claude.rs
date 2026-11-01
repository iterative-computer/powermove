//! Claude's stream-json and stdio control protocol. One client per CLI run.
use crate::{
    Access, Effects, Event, RunResult,
    approvals::{Approvals, Authority, question_card},
    codex::clip,
    text,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub fn user_message(prompt: &str, images: &[String]) -> Value {
    let prompt = if images.is_empty() {
        prompt.into()
    } else {
        format!(
            "{prompt}\n\nREFERENCE IMAGES\nInspect these files with the Read tool:\n{}",
            images
                .iter()
                .map(|p| format!("- {p}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    json!({"type":"user","session_id":"","parent_tool_use_id":null,"message":{"role":"user","content":[{"type":"text","text":prompt}]}})
}
struct Held {
    control: String,
    input: Value,
    texts: BTreeMap<String, String>,
    approval: bool,
}
struct Tool {
    id: String,
    name: String,
    input: String,
}
pub struct Client {
    run: String,
    authority: Authority,
    held: BTreeMap<String, Held>,
    approvals: Approvals,
    session: Option<String>,
    streaming: bool,
    tools: BTreeMap<u64, Tool>,
    hidden: BTreeSet<String>,
    compact: Option<String>,
    compactions: usize,
    done: bool,
    deadline: u64,
}
impl Client {
    pub fn run_id(&self) -> &str {
        &self.run
    }

    pub fn new(run: String, authority: Authority, now: u64) -> Self {
        Self {
            run,
            authority,
            held: BTreeMap::new(),
            approvals: Approvals::default(),
            session: None,
            streaming: false,
            tools: BTreeMap::new(),
            hidden: BTreeSet::new(),
            compact: None,
            compactions: 0,
            done: false,
            deadline: now.saturating_add(3_600_000),
        }
    }
    pub fn receive(&mut self, v: Value) -> Effects {
        let mut e = Effects::default();
        if self.done {
            return e;
        }
        if let Some(s) = v["session_id"].as_str().filter(|s| crate::valid_id(s, 200))
            && self.session.as_deref() != Some(s)
        {
            self.session = Some(s.into());
            e.events.push(Event::Session {
                run: self.run.clone(),
                id: s.into(),
            });
        }
        match v["type"].as_str().unwrap_or("") {
            "control_request" | "control_cancel_request" => self.control(&v, &mut e),
            "system" => match v["subtype"].as_str().unwrap_or("") {
                "status" if v["status"] == "compacting" => {
                    if self.compact.is_none() {
                        self.compact_start(&mut e);
                    }
                }
                "compact_boundary" => {
                    if self.compact.is_none() {
                        self.compact_start(&mut e);
                    }
                    let id = self.compact.take().unwrap();
                    e.trace(&self.run,json!({"kind":"tool-end","itemId":id,"isError":false,"output":"Earlier context summarized"}));
                }
                "api_retry" => {
                    let a = v["attempt"].as_u64().unwrap_or(0);
                    let m = v["max_retries"].as_u64().unwrap_or(0);
                    e.progress(
                        &self.run,
                        if a > 0 && m > 0 {
                            format!("Retrying the request ({a} of {m})…")
                        } else {
                            "Retrying the request…".into()
                        },
                    );
                }
                _ => {}
            },
            "stream_event" => self.stream(&v["event"], &mut e),
            "assistant" => {
                for b in v["message"]["content"].as_array().into_iter().flatten() {
                    if !self.streaming {
                        self.block(b, &mut e);
                    } else if b["type"] == "text" {
                        let progress = text(b, "text")
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ");
                        if !progress.is_empty() {
                            e.progress(&self.run, progress);
                        }
                    }
                }
            }
            "user" => {
                for b in v["message"]["content"].as_array().into_iter().flatten() {
                    if b["type"] == "tool_result" && !self.hidden.contains(&text(b, "tool_use_id"))
                    {
                        let mut trace = json!({"kind":"tool-end","itemId":b["tool_use_id"],"isError":b["is_error"]==true});
                        if let Some(output) = b.get("content") {
                            trace["output"] = json!(crate::trace::excerpt(output));
                        }
                        e.trace(&self.run, trace);
                    }
                }
            }
            "result" => {
                let failed = v["is_error"] == true
                    || v["subtype"] == "error"
                    || v["subtype"]
                        .as_str()
                        .is_some_and(|s| s.starts_with("error_"));
                let result = if failed {
                    RunResult::failure(
                        if text(&v, "result").is_empty() {
                            "Claude generation failed.".into()
                        } else {
                            text(&v, "result")
                        },
                        false,
                    )
                } else {
                    let output = v
                        .get("structured_output")
                        .filter(|v| !v.is_null())
                        .map(Value::to_string)
                        .unwrap_or_else(|| text(&v, "result"));
                    if output.is_empty() {
                        RunResult::failure("Claude generation completed without a result.", false)
                    } else {
                        RunResult::success(output, self.authority.access)
                    }
                };
                self.finish(result, &mut e);
            }
            _ => e.events.push(Event::Notification(v)),
        }
        e
    }
    fn compact_start(&mut self, e: &mut Effects) {
        self.compactions += 1;
        let id = format!("claude-compact-{}", self.compactions);
        self.compact = Some(id.clone());
        e.trace(&self.run,json!({"kind":"tool-start","itemId":id,"toolName":"compact","label":"Compacting context"}));
    }
    fn control(&mut self, v: &Value, e: &mut Effects) {
        let Some(id) = v["request_id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 200)
        else {
            return;
        };
        if v["type"] == "control_cancel_request" {
            let items = self
                .held
                .iter()
                .filter(|(_, h)| h.control == id)
                .map(|(k, _)| k.clone())
                .collect::<Vec<_>>();
            for item in items {
                self.held.remove(&item);
                e.trace(&self.run, json!({"kind":"question-closed","itemId":item}));
            }
            return;
        }
        let r = &v["request"];
        let input = r
            .get("input")
            .filter(|v| v.is_object())
            .cloned()
            .unwrap_or(json!({}));
        if r["subtype"] != "can_use_tool" {
            e.outgoing.push(json!({"type":"control_response","response":{"subtype":"error","request_id":id,"error":"Unsupported by Powermove."}}));
            return;
        }
        let item = text(r, "tool_use_id");
        if r["tool_name"] == "AskUserQuestion" && !item.is_empty() && !self.held.contains_key(&item)
        {
            let mut texts = BTreeMap::new();
            let questions=input["questions"].as_array().into_iter().flatten().take(6).enumerate().filter_map(|(i,q)|{
                let question=clip(&q["question"],1000);if question.is_empty(){return None;}
                let qid=format!("{item}-{i}");texts.insert(qid.clone(),text(q,"question"));
                let opts=q["options"].as_array().into_iter().flatten().take(8).filter_map(|o|{let label=clip(&o["label"],160);if label.is_empty(){None}else{Some(json!({"label":label,"description":clip(&o["description"],400)}))}}).collect::<Vec<_>>();
                let mut v=json!({"id":qid,"header":clip(&q["header"],80),"question":question,"options":opts,"allowOther":true,"secret":false});if q["multiSelect"]==true{v["multiSelect"]=json!(true);}Some(v)
            }).collect::<Vec<_>>();
            if !questions.is_empty() {
                self.hidden.insert(item.clone());
                self.held.insert(
                    item.clone(),
                    Held {
                        control: id.into(),
                        input,
                        texts,
                        approval: false,
                    },
                );
                e.trace(&self.run,json!({"kind":"question","itemId":item,"questions":questions,"transport":"reply","blocking":true}));
                return;
            }
        }
        if self.authority.claude_asks() {
            if self.approvals.allowed(&self.run) {
                respond(id, json!({"behavior":"allow","updatedInput":input}), e);
                return;
            }
            let item = format!("approval-{id}");
            if self.held.contains_key(&item) {
                return;
            }
            let tool = r["display_name"]
                .as_str()
                .or_else(|| r["tool_name"].as_str())
                .unwrap_or("a tool");
            let (title, detail, reason) =
                if r["tool_name"] == "Bash" && input["command"].is_string() {
                    (
                        if input["dangerouslyDisableSandbox"] == true {
                            "Run a command outside the project sandbox?".into()
                        } else {
                            "Run this command?".into()
                        },
                        text(&input, "command"),
                        clip(&input["description"], 600),
                    )
                } else if let Some(path) = input["file_path"]
                    .as_str()
                    .or_else(|| input["notebook_path"].as_str())
                {
                    (
                        format!("Allow {tool} to change a file outside the project?"),
                        path.into(),
                        String::new(),
                    )
                } else {
                    (
                        format!("Allow {tool}?"),
                        serde_json::to_string_pretty(&input).unwrap(),
                        String::new(),
                    )
                };
            e.trace(&self.run, question_card(&item, &title, &detail, &reason));
            self.held.insert(
                item,
                Held {
                    control: id.into(),
                    input,
                    texts: BTreeMap::new(),
                    approval: true,
                },
            );
            return;
        }
        respond(
            id,
            json!({"behavior":"deny","message":format!("Powermove does not allow {} in this run.",r["tool_name"].as_str().unwrap_or("this tool"))}),
            e,
        );
    }
    pub fn answer(
        &mut self,
        item: &str,
        answers: &BTreeMap<String, Vec<String>>,
    ) -> Option<Effects> {
        let h = self.held.remove(item)?;
        let mut e = Effects::default();
        if h.approval {
            let (allowed, message) = self.approvals.decide(
                &self.run,
                answers.get("decision").map(Vec::as_slice).unwrap_or(&[]),
            );
            respond(
                &h.control,
                if allowed {
                    json!({"behavior":"allow","updatedInput":h.input})
                } else {
                    json!({"behavior":"deny","message":message})
                },
                &mut e,
            );
        } else {
            let a = h
                .texts
                .iter()
                .filter_map(|(id, text)| {
                    let chosen = answers
                        .get(id)
                        .into_iter()
                        .flatten()
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(", ");
                    if chosen.is_empty() {
                        None
                    } else {
                        Some((text.clone(), json!(chosen)))
                    }
                })
                .collect::<serde_json::Map<_, _>>();
            if a.is_empty() {
                respond(
                    &h.control,
                    json!({"behavior":"deny","message":"The user skipped the question. Continue with your best judgement."}),
                    &mut e,
                );
            } else {
                let mut input = h.input;
                input["answers"] = json!(a);
                respond(
                    &h.control,
                    json!({"behavior":"allow","updatedInput":input}),
                    &mut e,
                );
            }
        }
        Some(e)
    }
    fn stream(&mut self, v: &Value, e: &mut Effects) {
        match v["type"].as_str().unwrap_or("") {
            "message_start" => {
                self.streaming = true;
                self.tools.clear();
            }
            "content_block_start" => {
                let b = &v["content_block"];
                if b["type"] == "tool_use" {
                    let id = text(b, "id");
                    let name = text(b, "name");
                    if name == "AskUserQuestion" {
                        self.hidden.insert(id.clone());
                    }
                    if let Some(index) = v["index"].as_u64()
                        && self.tools.len() < 64
                    {
                        if name != "AskUserQuestion" {
                            e.trace(&self.run,json!({"kind":"tool-start","itemId":id,"toolName":name.to_lowercase(),"label":name}));
                        }
                        self.tools.insert(
                            index,
                            Tool {
                                id,
                                name,
                                input: String::new(),
                            },
                        );
                    }
                } else if b["type"] == "text" && !text(b, "text").is_empty() {
                    e.trace(&self.run, json!({"kind":"answer","text":b["text"]}));
                }
            }
            "content_block_delta" => {
                let d = &v["delta"];
                match d["type"].as_str().unwrap_or("") {
                    "text_delta" => e.trace(
                        &self.run,
                        json!({"kind":"answer","text":crate::trace::fragment(&d["text"])}),
                    ),
                    "thinking_delta" => e.trace(
                        &self.run,
                        json!({"kind":"thought","text":crate::trace::fragment(&d["thinking"])}),
                    ),
                    "input_json_delta" => {
                        if let Some(t) = v["index"].as_u64().and_then(|i| self.tools.get_mut(&i))
                            && t.input.len() < 65536
                        {
                            t.input.push_str(d["partial_json"].as_str().unwrap_or(""));
                            t.input.truncate(t.input.floor_char_boundary(65536));
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                if let Some(t) = v["index"].as_u64().and_then(|i| self.tools.remove(&i))
                    && t.name != "AskUserQuestion"
                {
                    let input = serde_json::from_str(&t.input).unwrap_or(json!({}));
                    self.block(
                        &json!({"type":"tool_use","id":t.id,"name":t.name,"input":input}),
                        e,
                    );
                }
            }
            _ => {}
        }
    }
    fn block(&mut self, b: &Value, e: &mut Effects) {
        match b["type"].as_str().unwrap_or("") {
            "text" => e.trace(&self.run, json!({"kind":"answer","text":b["text"]})),
            "thinking" => e.trace(&self.run, json!({"kind":"thought","text":b["thinking"]})),
            "tool_use" => {
                let name = text(b, "name");
                if name == "AskUserQuestion" {
                    self.hidden.insert(text(b, "id"));
                    return;
                }
                let tool = name.to_lowercase();
                let label = crate::trace::label(&name);
                let mut trace =
                    json!({"kind":"tool-start","itemId":b["id"],"toolName":tool,"label":label});
                let detail = crate::trace::detail(&name, &b["input"]);
                if !detail.is_empty() {
                    trace["detail"] = json!(detail);
                }
                if let Some(subject) = crate::trace::subject(&name, &b["input"]) {
                    trace["subject"] = subject;
                }
                e.trace(&self.run, trace);
                e.progress(&self.run, format!("Using {label}…"));
            }
            _ => {}
        }
    }
    fn finish(&mut self, result: RunResult, e: &mut Effects) {
        if self.done {
            return;
        }
        self.done = true;
        for (item, h) in std::mem::take(&mut self.held) {
            respond(
                &h.control,
                json!({"behavior":"deny","message":"The run ended."}),
                e,
            );
            e.trace(&self.run, json!({"kind":"question-closed","itemId":item}));
        }
        self.approvals.close(&self.run);
        e.close_stdin = true;
        e.events.push(Event::Completed {
            run: self.run.clone(),
            result,
        });
    }
    pub fn cancel(&mut self) -> Effects {
        let mut e = Effects::default();
        self.finish(
            RunResult::failure("The Claude run was cancelled.", true),
            &mut e,
        );
        e
    }
    pub fn stopped(&mut self, reason: &str) -> Effects {
        let mut e = Effects::default();
        self.finish(RunResult::failure(reason, false), &mut e);
        e.events.push(Event::Stopped(reason.into()));
        e
    }
    pub fn tick(&mut self, now: u64) -> Effects {
        let mut e = Effects::default();
        if now >= self.deadline {
            self.finish(
                RunResult::failure("The coding agent took too long to respond.", false),
                &mut e,
            );
        }
        e
    }
}
fn respond(id: &str, response: Value, e: &mut Effects) {
    e.outgoing.push(json!({"type":"control_response","response":{"subtype":"success","request_id":id,"response":response}}));
}

/// Documented native CLI flags only. MCP configuration is passed as JSON,
/// never via a shell. The caller supplies the same isolated runtime home.
pub struct Options {
    pub authority: Authority,
    pub schema: Value,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub session: Option<String>,
    pub mcp: Value,
    pub extensions: Option<String>,
    pub instructions: String,
    pub project_tools: Vec<String>,
    pub editor_tools: Vec<String>,
}
impl Options {
    pub fn argv(&self) -> Result<Vec<String>, String> {
        if self
            .session
            .as_ref()
            .is_some_and(|s| !crate::valid_id(s, 200) || s.starts_with(['-', '_']))
        {
            return Err("Invalid Claude session id".into());
        }
        let mut args = vec![
            "--print".into(),
            "--input-format".into(),
            "stream-json".into(),
            "--output-format".into(),
            "stream-json".into(),
            "--permission-prompt-tool".into(),
            "stdio".into(),
            "--include-partial-messages".into(),
            "--verbose".into(),
            "--mcp-config".into(),
            self.mcp.to_string(),
            "--json-schema".into(),
            self.schema.to_string(),
        ];
        if let Some(m) = &self.model {
            args.extend(["--model".into(), m.clone()]);
        }
        if let Some(m) = &self.effort {
            args.extend(["--effort".into(), m.clone()]);
        }
        if self.authority.access == Access::Editor {
            args.push("--no-session-persistence".into());
        }
        if let Some(s) = &self.session {
            args.extend(["--resume".into(), s.clone()]);
        }
        if self.authority.computer_allowed() {
            args.extend([
                "--dangerously-skip-permissions".into(),
                "--tools".into(),
                "default".into(),
            ]);
        } else {
            let editor = self.authority.access == Access::Editor;
            let mut sandbox = json!({"enabled":true,"autoAllowBashIfSandboxed":!editor,"allowUnsandboxedCommands":!editor,"failIfUnavailable":true});
            if !editor {
                sandbox["network"] = json!({"allowedDomains":["*"],"allowLocalBinding":true});
            }
            let base = if editor {
                "Read,Glob,Grep,Skill,Agent,Task"
            } else {
                "Read,Glob,Grep,WebSearch,WebFetch,Skill,Agent,Task"
            };
            let mut allowed = vec![base.to_string()];
            allowed.extend(if editor {
                self.editor_tools.clone()
            } else {
                self.project_tools.clone()
            });
            args.extend([
                "--permission-mode".into(),
                if editor {
                    "default".into()
                } else {
                    "acceptEdits".into()
                },
                "--settings".into(),
                json!({"sandbox":sandbox}).to_string(),
                "--tools".into(),
                "default".into(),
                "--allowedTools".into(),
                allowed.join(","),
            ]);
            if editor {
                args.extend([
                    "--disallowedTools".into(),
                    "Bash,Monitor,PowerShell,Write,Edit,NotebookEdit".into(),
                ]);
            }
        }
        if let Some(dir) = &self.extensions {
            args.extend(["--add-dir".into(), dir.clone()]);
        }
        args.extend(["--system-prompt".into(), self.instructions.clone()]);
        Ok(args)
    }
}
