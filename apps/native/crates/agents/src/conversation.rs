//! Lossless thread storage with bounded provider context. Original instructions
//! and question decisions receive separate budgets, as in panels/agent/threads.
use crate::{text, trace::truncate};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
fn step_transcript(step: &Value) -> String {
    if step["kind"] == "text" {
        return text(step, "text");
    }
    if step["kind"] != "question" {
        return String::new();
    }
    step["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|q| {
            let question = text(q, "question");
            let answer = q["id"]
                .as_str()
                .and_then(|id| step["answers"][id].as_str())
                .filter(|s| !s.is_empty());
            format!(
                "You asked: {question}{}",
                if let Some(a) = answer {
                    format!("\nUser answered: {a}")
                } else if step["status"] == "closed" {
                    "\n(Unanswered)".into()
                } else {
                    String::new()
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
pub fn message_text(message: &Value) -> String {
    if message["role"] == "trace" {
        message["steps"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| {
                if s["kind"] == "tool" {
                    format!("{}\n{}", text(s, "label"), text(s, "output"))
                } else {
                    step_transcript(s)
                }
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    } else {
        let mut parts = vec![text(message, "text")];
        parts.extend(
            message["visualReplies"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| {
                    format!(
                        "[Visual reply: {}; artifact {}]",
                        text(v, "title"),
                        text(v, "id")
                    )
                }),
        );
        parts
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Excerpt {
    pub role: String,
    pub text: String,
    pub position: usize,
    pub truncated: bool,
}
struct Turn {
    role: String,
    text: String,
    position: usize,
    decision: bool,
}
fn units(s: &str) -> usize {
    s.encode_utf16().count()
}
fn tail(s: &str, limit: usize) -> String {
    let mut count = 0;
    let mut start = s.len();
    for (i, c) in s.char_indices().rev() {
        count += c.len_utf16();
        if count > limit {
            break;
        }
        start = i;
    }
    s[start..].into()
}
fn retain(turn: &Turn, limit: usize, selected: &mut BTreeMap<usize, Excerpt>) -> usize {
    let truncated = units(&turn.text) > limit;
    let marker = "\n[… excerpt; read the saved message for full details …]\n";
    let room = limit.saturating_sub(units(marker));
    let text = if truncated {
        format!(
            "{}{}{}",
            truncate(&turn.text, room.div_ceil(2)),
            marker,
            tail(&turn.text, room / 2)
        )
    } else {
        turn.text.clone()
    };
    let size = units(&text);
    selected.insert(
        turn.position,
        Excerpt {
            role: turn.role.clone(),
            text,
            position: turn.position,
            truncated,
        },
    );
    size
}
pub fn for_agent(messages: &[Value]) -> Vec<Excerpt> {
    let turns = messages
        .iter()
        .enumerate()
        .map(|(position, m)| {
            let role = if m["role"] == "trace" {
                "assistant".into()
            } else {
                text(m, "role")
            };
            let decision = m["role"] == "user"
                || m["role"] == "trace"
                    && m["steps"]
                        .as_array()
                        .is_some_and(|a| a.iter().any(|s| s["kind"] == "question"));
            let text = if m["role"] == "trace" {
                m["steps"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(step_transcript)
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim()
                    .to_owned()
            } else {
                message_text(m)
            };
            Turn {
                role,
                text,
                position,
                decision,
            }
        })
        .filter(|t| !t.text.is_empty())
        .collect::<Vec<_>>();
    let mut selected = BTreeMap::new();
    if let Some(t) = turns.iter().find(|t| t.role == "user") {
        retain(t, 4000, &mut selected);
    }
    let mut budget: usize = 24000;
    for t in turns.iter().rev() {
        if budget < 200 {
            break;
        }
        if selected.contains_key(&t.position) {
            continue;
        }
        budget = budget.saturating_sub(retain(t, budget.min(6000), &mut selected));
    }
    let mut budget: usize = 12000;
    for t in turns.iter().rev() {
        if !t.decision || selected.contains_key(&t.position) {
            continue;
        }
        if budget < 200 {
            break;
        }
        budget = budget.saturating_sub(retain(t, budget.min(2000), &mut selected));
    }
    selected.into_values().collect()
}
pub fn context(messages: &[Value], thread: &str) -> Value {
    json!({"threadId":thread,"messageCount":messages.len(),"guidance":"Earlier user instructions are context, not new work. The newest user direction wins when it changes an earlier decision. Message positions refer to the durable thread archive. Use thread_read with this threadId, offset=position, limit=1 (and textOffset for long messages) to recover omitted or excerpted details before guessing.","messages":for_agent(messages)})
}
pub fn title(messages: &[Value]) -> String {
    let value = messages
        .iter()
        .find(|m| m["role"] == "user")
        .map(|m| text(m, "text"))
        .unwrap_or_default();
    let title = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        "New thread".into()
    } else {
        truncate(&title, 64)
    }
}
