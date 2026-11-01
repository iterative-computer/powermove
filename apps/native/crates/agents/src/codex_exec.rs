//! Frozen autonomous Codex CLI contract. App Server remains the editor transport.
use crate::{
    Access, ApprovalMode, Effects, Event, RunResult, approvals::Authority, codex::tool_trace, text,
};
use serde_json::{Value, json};

pub struct Options {
    pub authority: Authority,
    pub extensions: String,
    pub schema_path: String,
    pub output_path: String,
    pub prompt: String,
    pub instructions: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub session: Option<String>,
    pub image_paths: Vec<String>,
    pub shell_network: bool,
    /// Generated MCP `--config` pairs from the Rust MCP host; never a JS shim.
    pub mcp_config: Vec<String>,
}
impl Options {
    pub fn argv(&self) -> Result<Vec<String>, String> {
        if self.authority.access == Access::Editor {
            return Err("Use App Server for editor runs".into());
        }
        if self
            .session
            .as_ref()
            .is_some_and(|s| !crate::valid_id(s, 200) || !s.as_bytes()[0].is_ascii_alphanumeric())
        {
            return Err("Invalid Codex session id.".into());
        }
        let mut a = vec!["--search".into()];
        if self.authority.computer_allowed() {
            a.push("--dangerously-bypass-approvals-and-sandbox".into());
        }
        a.extend(["--add-dir".into(), self.extensions.clone(), "exec".into()]);
        if self.session.is_some() {
            a.push("resume".into());
        }
        a.push("--skip-git-repo-check".into());
        if !self.authority.computer_allowed() {
            let mut config = if self.authority.approval == ApprovalMode::Supervised {
                vec!["approval_policy=\"never\"".into()]
            } else {
                vec![
                    "approvals_reviewer=\"auto_review\"".into(),
                    "approval_policy=\"on-request\"".into(),
                ]
            };
            config.extend([
                "default_permissions=\"powermove\"".into(),
                "permissions.powermove.extends=\":workspace\"".into(),
            ]);
            if self.shell_network {
                config.push("permissions.powermove.network.enabled=true".into());
            }
            for c in config {
                a.extend(["--config".into(), c]);
            }
        }
        a.extend(self.mcp_config.clone());
        a.extend([
            "--output-schema".into(),
            self.schema_path.clone(),
            "--output-last-message".into(),
            self.output_path.clone(),
            "--json".into(),
        ]);
        if let Some(m) = &self.model {
            a.extend(["--model".into(), m.clone()]);
        }
        if let Some(e) = &self.effort {
            a.extend([
                "--config".into(),
                format!("model_reasoning_effort={}", json!(e)),
            ]);
        }
        for p in &self.image_paths {
            a.extend(["--image".into(), p.clone()]);
        }
        a.push("--".into());
        if let Some(s) = &self.session {
            a.push(s.clone());
        }
        a.push(format!(
            "{}\n\nUSER REQUEST\n{}",
            self.instructions, self.prompt
        ));
        Ok(a)
    }
}
pub struct Client {
    run: String,
    access: Access,
    session: Option<String>,
    final_text: String,
    observed_work: bool,
    reconnecting: bool,
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
            access: authority.access,
            session: None,
            final_text: String::new(),
            observed_work: false,
            reconnecting: false,
            done: false,
            deadline: now.saturating_add(3_600_000),
        }
    }
    /// A missing resume handle is retriable ONLY before any provider work.
    pub fn can_retry_missing_resume(&self) -> bool {
        !self.observed_work
    }
    pub fn receive(&mut self, v: Value) -> Effects {
        let mut e = Effects::default();
        if self.done {
            return e;
        }
        let ty = text(&v, "type");
        let item = &v["item"];
        let kind = text(item, "type");
        if ty == "thread.started"
            && self.session.is_none()
            && let Some(id) = v["thread_id"].as_str().filter(|s| crate::valid_id(s, 200))
        {
            self.session = Some(id.into());
            e.events.push(Event::Session {
                run: self.run.clone(),
                id: id.into(),
            });
        }
        if ty == "item.started" || ty == "item.completed" {
            self.observed_work = true;
        }
        if ty == "error" {
            let message = text(&v, "message");
            if message.to_lowercase().starts_with("reconnecting") {
                if !self.reconnecting {
                    e.progress(&self.run, "Connection interrupted. Reconnecting…");
                }
                self.reconnecting = true;
            } else {
                e.progress(&self.run, message);
            }
            return e;
        }
        if self.reconnecting
            && (ty == "item.started" || ty == "item.completed" || ty == "turn.completed")
        {
            self.reconnecting = false;
            e.progress(&self.run, "Connected. Continuing…");
        }
        if ty == "item.started" {
            let progress = match kind.as_str() {
                "command_execution" => Some("Working with project files and shell tools…"),
                "web_search" => Some("Researching on the web…"),
                "computer_use" => Some("Operating an application on this Mac…"),
                "image_generation" => Some("Generating a visual deliverable…"),
                "file_change" => Some("Preparing project files…"),
                _ => None,
            };
            if let Some(p) = progress {
                e.progress(&self.run, p);
            }
        }
        if ty == "item.completed" && (kind == "reasoning" || kind == "agent_message") {
            let raw = text(item, "text");
            if kind == "agent_message" {
                self.final_text = raw.clone();
            }
            let display = match serde_json::from_str::<Value>(&raw) {
                Ok(v) => ["message", "summary", "critique", "status"]
                    .iter()
                    .find_map(|k| v[k].as_str())
                    .unwrap_or("")
                    .to_owned(),
                Err(_) => raw,
            };
            if !display.is_empty() {
                e.progress(
                    &self.run,
                    display.split_whitespace().collect::<Vec<_>>().join(" "),
                );
                e.trace(
                    &self.run,
                    json!({"kind":if kind=="reasoning"{"thought"}else{"answer"},"text":display}),
                );
            }
        } else if (ty == "item.started" || ty == "item.completed")
            && let Some(t) = tool_trace(item, ty == "item.completed")
        {
            e.trace(&self.run, t);
        }
        if ty == "turn.failed" {
            let msg = v["error"]["message"]
                .as_str()
                .unwrap_or("Codex generation failed.");
            self.finish(RunResult::failure(msg, false), &mut e);
        }
        e
    }
    /// The worker calls this with the private output-last-message file only
    /// after the child exits. Nonzero exits never turn partial prose into success.
    pub fn exited(&mut self, code: Option<i32>, output: Option<String>) -> Effects {
        let mut e = Effects::default();
        let value = output.unwrap_or_else(|| self.final_text.clone());
        let result = if code == Some(0) && !value.is_empty() {
            RunResult::success(value, self.access)
        } else {
            RunResult::failure(
                "Codex generation failed or completed without a result.",
                false,
            )
        };
        self.finish(result, &mut e);
        e
    }
    fn finish(&mut self, result: RunResult, e: &mut Effects) {
        if !self.done {
            self.done = true;
            e.events.push(Event::Completed {
                run: self.run.clone(),
                result,
            });
            e.close_stdin = true;
        }
    }
    pub fn cancel(&mut self) -> Effects {
        let mut e = Effects::default();
        self.finish(
            RunResult::failure("The Codex run was cancelled.", true),
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
