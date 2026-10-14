//! Supervised runs ask the person before anything the sandbox blocks (port of
//! `agent-approvals.ts`). Each ask is a held question card on the run's
//! trace; `codex:answer` settles it, and a run that ends denies the rest.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::RunEvents;

pub const ALLOW_ONCE: &str = "Allow once";
pub const ALLOW_RUN: &str = "Allow for this run";
pub const DENY: &str = "Deny";
const QUESTION_ID: &str = "decision";
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(3_600);
const DETAIL_CHARS: usize = 2_000;

/// An action the Project sandbox would block, described for the person.
pub struct ApprovalRequest {
    pub title: String,
    pub detail: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allowed,
    Denied(String),
}

struct Held {
    run_id: String,
    settle: oneshot::Sender<Decision>,
}

#[derive(Default)]
pub struct Approvals {
    held: Mutex<HashMap<String, Held>>,
    allowed_runs: Mutex<HashSet<String>>,
}

fn fenced(text: &str) -> String {
    let clipped = if text.chars().count() > DETAIL_CHARS { format!("{}…", text.chars().take(DETAIL_CHARS).collect::<String>()) } else { text.to_owned() };
    let longest = clipped.split(|character| character != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat((longest + 1).max(3));
    format!("{fence}\n{clipped}\n{fence}")
}

impl Approvals {
    pub async fn request(&self, run_id: &str, events: &RunEvents, request: ApprovalRequest, cancel: &CancellationToken) -> Decision {
        if self.allowed_runs.lock().contains(run_id) {
            return Decision::Allowed;
        }
        if cancel.is_cancelled() {
            return Decision::Denied("The request was cancelled.".into());
        }
        let item_id = format!("approval-{}", uuid::Uuid::new_v4());
        let (sender, receiver) = oneshot::channel();
        self.held.lock().insert(item_id.clone(), Held { run_id: run_id.to_owned(), settle: sender });
        let reason = request.reason.map(|reason| format!("\n\n{}", reason.chars().take(600).collect::<String>())).unwrap_or_default();
        events.trace(json!({
            "kind": "question", "itemId": item_id, "transport": "reply", "blocking": true,
            "questions": [{
                "id": QUESTION_ID,
                "header": "Approval needed",
                "question": format!("{}\n\n{}{reason}", request.title, fenced(&request.detail)),
                "options": [
                    { "label": ALLOW_ONCE, "description": "" },
                    { "label": ALLOW_RUN, "description": "Don’t ask again until this run ends" },
                    { "label": DENY, "description": "" }
                ],
                "allowOther": true,
                "secret": false
            }]
        }));
        let closed = |message: &str| {
            self.held.lock().remove(&item_id);
            events.trace(json!({ "kind": "question-closed", "itemId": item_id }));
            Decision::Denied(message.to_owned())
        };
        tokio::select! {
            decision = receiver => decision.unwrap_or_else(|_| Decision::Denied("The run ended.".into())),
            () = cancel.cancelled() => closed("The request was cancelled."),
            () = tokio::time::sleep(APPROVAL_TIMEOUT) => closed("Nobody answered the approval request. Continue without it."),
        }
    }

    /// Settle a held approval from its card. A typed answer declines with that note.
    pub fn answer(&self, run_id: &str, item_id: &str, answers: &Value) -> bool {
        let held = {
            let mut held = self.held.lock();
            match held.get(item_id) {
                Some(entry) if entry.run_id == run_id => held.remove(item_id),
                _ => None,
            }
        };
        let Some(held) = held else { return false };
        let chosen: Vec<String> = answers[QUESTION_ID].as_array().into_iter().flatten().filter_map(Value::as_str).map(|answer| answer.trim().to_owned()).filter(|answer| !answer.is_empty()).collect();
        if chosen.iter().any(|answer| answer == ALLOW_RUN) {
            self.allowed_runs.lock().insert(run_id.to_owned());
        }
        let decision = if chosen.iter().any(|answer| answer == ALLOW_ONCE || answer == ALLOW_RUN) {
            Decision::Allowed
        } else {
            let note = chosen.iter().filter(|answer| *answer != DENY).cloned().collect::<Vec<_>>().join(", ");
            Decision::Denied(if note.is_empty() {
                "The user declined. Do not retry it; continue inside the sandbox or explain what they need to do themselves.".into()
            } else {
                format!("The user declined and said: {note}")
            })
        };
        let _ = held.settle.send(decision);
        true
    }

    /// The run ended: deny anything still waiting and forget its standing allowance.
    pub fn close_run(&self, run_id: &str) {
        self.allowed_runs.lock().remove(run_id);
        let mut held = self.held.lock();
        let ids: Vec<String> = held.iter().filter(|(_, entry)| entry.run_id == run_id).map(|(id, _)| id.clone()).collect();
        for id in ids {
            if let Some(entry) = held.remove(&id) {
                let _ = entry.settle.send(Decision::Denied("The run ended.".into()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn asks_once_then_remembers_the_run() {
        let approvals = Arc::new(Approvals::default());
        let (events, mut steps) = RunEvents::channel();
        let cancel = CancellationToken::new();
        let asking = {
            let approvals = approvals.clone();
            let events = events.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move { approvals.request("run-1", &events, ApprovalRequest { title: "Run?".into(), detail: "rm x".into(), reason: None }, &cancel).await })
        };
        let (_, step) = steps.recv().await.unwrap();
        let item = step["itemId"].as_str().unwrap().to_owned();
        assert!(!approvals.answer("run-2", &item, &json!({ "decision": [ALLOW_ONCE] })));
        assert!(approvals.answer("run-1", &item, &json!({ "decision": [ALLOW_RUN] })));
        assert_eq!(asking.await.unwrap(), Decision::Allowed);
        let again = approvals.request("run-1", &events, ApprovalRequest { title: "Again".into(), detail: String::new(), reason: None }, &cancel).await;
        assert_eq!(again, Decision::Allowed);
        approvals.close_run("run-1");
    }

    #[tokio::test]
    async fn a_typed_answer_declines_with_the_note() {
        let approvals = Arc::new(Approvals::default());
        let (events, mut steps) = RunEvents::channel();
        let cancel = CancellationToken::new();
        let asking = {
            let approvals = approvals.clone();
            tokio::spawn(async move { approvals.request("run-1", &events, ApprovalRequest { title: "Run?".into(), detail: "x".into(), reason: Some("why".into()) }, &cancel).await })
        };
        let (_, step) = steps.recv().await.unwrap();
        assert!(step["questions"][0]["question"].as_str().unwrap().contains("```\nx\n```"));
        approvals.answer("run-1", step["itemId"].as_str().unwrap(), &json!({ "decision": ["use the other folder"] }));
        assert_eq!(asking.await.unwrap(), Decision::Denied("The user declined and said: use the other folder".into()));
    }
}
