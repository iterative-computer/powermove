//! Explicit choices and typed answers, never inferred approval. Source:
//! v1.2.0 QuestionCard.svelte:17–45; spatial.ts:1917–1951.
use crate::{Error, Result};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub type Answers = BTreeMap<String, Vec<String>>;
#[derive(Clone, Default)]
pub struct QuestionDraft {
    chosen: Answers,
    typed: BTreeMap<String, String>,
}
impl QuestionDraft {
    fn item<'a>(step: &'a Value, id: &str) -> Result<&'a Value> {
        if step["status"] != "open" {
            return Err(Error::Stale);
        }
        step["questions"]
            .as_array()
            .and_then(|a| a.iter().find(|v| v["id"] == id))
            .ok_or_else(|| Error::UnknownId(id.into()))
    }
    pub fn choose(&mut self, step: &Value, id: &str, label: &str) -> Result<()> {
        let item = Self::item(step, id)?;
        if !item["options"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v["label"] == label))
        {
            return Err(Error::Invalid("question option"));
        }
        let chosen = self.chosen.entry(id.into()).or_default();
        if let Some(i) = chosen.iter().position(|s| s == label) {
            chosen.remove(i);
        } else {
            if item["multiSelect"] != true {
                chosen.clear();
            }
            chosen.push(label.into());
        }
        Ok(())
    }
    pub fn type_answer(&mut self, step: &Value, id: &str, text: String) -> Result<()> {
        if Self::item(step, id)?["allowOther"] != true {
            return Err(Error::Invalid("free text disabled"));
        }
        self.typed.insert(id.into(), text);
        Ok(())
    }
    pub fn answers(&self, step: &Value) -> Answers {
        step["questions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| {
                let id = item["id"].as_str()?;
                let mut answers = self.chosen.get(id).cloned().unwrap_or_default();
                if let Some(text) = self
                    .typed
                    .get(id)
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                {
                    answers.push(text.into());
                }
                Some((id.into(), answers))
            })
            .collect()
    }
    pub fn submit(&self, step: &Value, skip: bool) -> Result<Answers> {
        if step["status"] != "open" {
            return Err(Error::Stale);
        }
        if skip {
            if step["transport"] != "reply" {
                return Err(Error::Invalid("message question cannot skip"));
            }
            return Ok(Answers::new());
        }
        let answers = self.answers(step);
        if !answers.values().any(|a| !a.is_empty()) {
            return Err(Error::Invalid("empty answer"));
        }
        Ok(answers)
    }
}
/// Only this masked acknowledgement is eligible for transcripts/persistence.
pub fn shown_answers(step: &Value, answers: &Answers) -> Value {
    let mut shown = serde_json::Map::new();
    for item in step["questions"].as_array().into_iter().flatten() {
        if let Some(id) = item["id"].as_str() {
            let answer = answers
                .get(id)
                .map(|v| {
                    v.iter()
                        .filter(|s| !s.is_empty())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            shown.insert(
                id.into(),
                json!(if item["secret"] == true && !answer.is_empty() {
                    "••••••"
                } else {
                    &answer
                }),
            );
        }
    }
    Value::Object(shown)
}

impl std::fmt::Debug for QuestionDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuestionDraft")
            .field("chosen_questions", &self.chosen.len())
            .field("typed_questions", &self.typed.len())
            .finish()
    }
}
