//! Run-scoped authority. No decision is persisted across a run boundary.
use crate::{Access, ApprovalMode};
use serde_json::{Value, json};
use std::collections::BTreeSet;
#[derive(Clone, Debug)]
pub struct Authority {
    pub access: Access,
    pub approval: ApprovalMode,
    computer_consent: bool,
}
impl Authority {
    pub fn new(
        access: Access,
        approval: ApprovalMode,
        fresh_computer_consent: bool,
    ) -> Result<Self, String> {
        if access == Access::Computer && !fresh_computer_consent {
            return Err("Computer access requires fresh approval for this run.".into());
        }
        Ok(Self {
            access,
            approval,
            computer_consent: fresh_computer_consent,
        })
    }
    pub fn computer_allowed(&self) -> bool {
        self.access == Access::Computer && self.computer_consent
    }
    /// Claude has no auto-reviewer; both project approval modes ask the user.
    pub fn claude_asks(&self) -> bool {
        self.access == Access::Project
    }
}
#[derive(Default)]
pub struct Approvals {
    allowed_runs: BTreeSet<String>,
}
impl Approvals {
    pub fn allowed(&self, run: &str) -> bool {
        self.allowed_runs.contains(run)
    }
    pub fn decide(&mut self, run: &str, answers: &[String]) -> (bool, String) {
        let chosen: Vec<_> = answers
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if chosen.contains(&"Allow for this run") {
            self.allowed_runs.insert(run.into());
        }
        if chosen.contains(&"Allow once") || chosen.contains(&"Allow for this run") {
            return (true, String::new());
        }
        let note = chosen
            .into_iter()
            .filter(|s| *s != "Deny")
            .collect::<Vec<_>>()
            .join(", ");
        (
            false,
            if note.is_empty() {
                "The user declined. Do not retry it; continue inside the sandbox or explain what they need to do themselves.".into()
            } else {
                format!("The user declined and said: {note}")
            },
        )
    }
    pub fn close(&mut self, run: &str) {
        self.allowed_runs.remove(run);
    }
}
pub fn question_card(item_id: &str, title: &str, detail: &str, reason: &str) -> Value {
    let clipped: String = detail.chars().take(2000).collect();
    let longest = clipped.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(3.max(longest + 1));
    let suffix = if detail.chars().count() > 2000 {
        "…"
    } else {
        ""
    };
    let reason = if reason.is_empty() {
        String::new()
    } else {
        format!("\n\n{}", reason.chars().take(600).collect::<String>())
    };
    json!({"kind":"question","itemId":item_id,"transport":"reply","blocking":true,"questions":[{"id":"decision","header":"Approval needed","question":format!("{title}\n\n{fence}\n{clipped}{suffix}\n{fence}{reason}"),"options":[{"label":"Allow once","description":""},{"label":"Allow for this run","description":"Don’t ask again until this run ends"},{"label":"Deny","description":""}],"allowOther":true,"secret":false}]})
}
