//! The delegation catalog (port of `orchestration-providers.ts` and the
//! effort rules of `shared/agent-models.ts`): which providers can run child
//! tasks, with the same models and reasoning options as the model picker.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

use super::request::{EFFORTS, Provider};

/// Bundled entries keep the catalog usable when live discovery is unavailable.
const CHATGPT_MODELS: [(&str, &str); 7] = [
    ("gpt-6-astra", "GPT 6 Astra"),
    ("gpt-6.1-sol", "GPT 6.1 Sol"),
    ("gpt-6-sol", "GPT 6 Sol"),
    ("gpt-6-luna", "GPT 6 Luna"),
    ("gpt-5.6-sol", "5.6 Sol"),
    ("gpt-5.6-terra", "5.6 Terra"),
    ("gpt-5.6-luna", "5.6 Luna"),
];
const CLAUDE_MODELS: [(&str, &str); 15] = [
    ("claude-fable-5-1", "Fable 5.1"),
    ("claude-opus-5-5", "Opus 5.5"),
    ("claude-sonnet-5-5", "Sonnet 5.5"),
    ("claude-opus-5", "Opus 5"),
    ("claude-sonnet-5", "Sonnet 5"),
    ("claude-haiku-4-5-20251001", "Haiku 4.5"),
    ("claude-fable-5", "Fable 5"),
    ("claude-opus-4-8", "Opus 4.8"),
    ("claude-opus-4-7", "Opus 4.7"),
    ("claude-opus-4-6", "Opus 4.6"),
    ("claude-sonnet-4-6", "Sonnet 4.6"),
    ("sonnet", "Sonnet (latest)"),
    ("opus", "Opus (latest)"),
    ("fable", "Fable (latest)"),
    ("haiku", "Haiku (latest)"),
];
const STANDARD: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

pub fn provider_id(provider: Provider) -> &'static str {
    match provider {
        Provider::ChatGpt => "chatgpt",
        Provider::Claude => "claude",
        Provider::Compatible => "compatible",
    }
}

pub fn parse_provider(id: &str) -> Option<Provider> {
    match id {
        "chatgpt" => Some(Provider::ChatGpt),
        "claude" => Some(Provider::Claude),
        "compatible" => Some(Provider::Compatible),
        _ => None,
    }
}

/// `modelEfforts` for models without discovered efforts.
pub fn model_efforts(provider: Provider, model: Option<&str>) -> Vec<&'static str> {
    static RULES: LazyLock<[(Regex, &'static [&'static str]); 6]> = LazyLock::new(|| {
        [
            (Regex::new(r"^gpt-6-(?:sol|luna)(?:-|$)").unwrap(), &["none", "low", "medium", "high", "xhigh", "max"]),
            (Regex::new(r"^gpt-(?:6-astra|5\.6)(?:-|$)").unwrap(), &STANDARD),
            (Regex::new(r"^gpt-5\.[2345]-chat(?:-|$)").unwrap(), &[]),
            (Regex::new(r"^gpt-5\.[245]-pro(?:-|$)").unwrap(), &["medium", "high", "xhigh"]),
            (Regex::new(r"^gpt-5\.[2345](?:-|$)").unwrap(), &["low", "medium", "high", "xhigh"]),
            (Regex::new(r"^(?:o[134]|gpt-oss)(?:-|$)").unwrap(), &["low", "medium", "high"]),
        ]
    });
    let model = model.unwrap_or_default();
    match provider {
        Provider::ChatGpt if model == "gpt-6-sol" || model == "gpt-6.1-sol" => [&STANDARD[..], &["ultra"]].concat(),
        Provider::ChatGpt => STANDARD.to_vec(),
        // Compatible services also host models without this OpenAI parameter.
        Provider::Compatible => RULES.iter().find(|(rule, _)| rule.is_match(model)).map_or_else(Vec::new, |(_, efforts)| efforts.to_vec()),
        Provider::Claude if model.contains("haiku") => Vec::new(),
        Provider::Claude if model == "claude-opus-4-6" || model == "claude-sonnet-4-6" => vec!["low", "medium", "high", "max"],
        Provider::Claude => STANDARD.to_vec(),
    }
}

/// `modelEffort`: the requested effort, or the nearest lower one the model supports.
pub fn model_effort(provider: Provider, model: Option<&str>, effort: Option<&str>) -> Option<String> {
    let effort = effort?;
    let supported = model_efforts(provider, model);
    if supported.is_empty() {
        return None;
    }
    if supported.contains(&effort) {
        return Some(effort.to_owned());
    }
    let rank = EFFORTS.iter().position(|item| *item == effort).unwrap_or(0);
    let lower = EFFORTS.iter().take(rank + 1).rfind(|item| supported.contains(item));
    Some(lower.unwrap_or(&supported[0]).to_string())
}

fn effort_label(id: &str) -> String {
    if id == "xhigh" {
        return "Extra High".into();
    }
    let mut characters = id.chars();
    characters.next().map(|first| first.to_uppercase().chain(characters).collect()).unwrap_or_default()
}

/// What the host knows about each provider when building the catalog.
pub struct ProviderInputs {
    pub chatgpt_connected: bool,
    /// `chatgpt:models` (`{ id, label, reasoningEfforts }`); empty when undiscovered.
    pub chatgpt_models: Vec<Value>,
    pub claude_connected: bool,
    pub claude_models: Vec<Value>,
    /// The saved compatible connection (`compatible:status`).
    pub compatible: Option<Value>,
}

fn efforts_of(entry: &Value) -> Vec<String> {
    entry["reasoningEfforts"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect()
}

/// `orchestrationProviders`: the `OrchestrationProvider[]` that
/// `orchestrator_capabilities` returns and child targets are checked against.
pub fn providers(inputs: &ProviderInputs) -> Vec<Value> {
    let mut out = Vec::new();
    for provider in [Provider::ChatGpt, Provider::Claude, Provider::Compatible] {
        // (id, label, discovered efforts)
        let models: Vec<(String, String, Option<Vec<String>>)> = match provider {
            Provider::ChatGpt => {
                // Discovered models replace bundled entries by id and add new ones.
                let discovered = &inputs.chatgpt_models;
                let mut models: Vec<_> = CHATGPT_MODELS
                    .iter()
                    .map(|(id, label)| match discovered.iter().find(|model| model["id"] == *id) {
                        Some(model) => (id.to_string(), model["label"].as_str().unwrap_or(label).to_owned(), Some(efforts_of(model))),
                        None => (id.to_string(), label.to_string(), None),
                    })
                    .collect();
                for model in discovered {
                    if let Some(id) = model["id"].as_str().filter(|id| !CHATGPT_MODELS.iter().any(|(bundled, _)| bundled == id)) {
                        models.push((id.to_owned(), model["label"].as_str().unwrap_or(id).to_owned(), Some(efforts_of(model))));
                    }
                }
                models
            }
            Provider::Claude => {
                let discovered = &inputs.claude_models;
                let mut models: Vec<_> = CLAUDE_MODELS
                    .iter()
                    .map(|(id, label)| (id.to_string(), label.to_string(), discovered.iter().find(|model| model["id"] == *id).map(efforts_of)))
                    .collect();
                for model in discovered {
                    if let Some(id) = model["id"].as_str().filter(|id| !CLAUDE_MODELS.iter().any(|(bundled, _)| bundled == id)) {
                        models.push((id.to_owned(), model["label"].as_str().unwrap_or(id).to_owned(), Some(efforts_of(model))));
                    }
                }
                models
            }
            Provider::Compatible => {
                let config = inputs.compatible.as_ref();
                let configured = config.and_then(|config| config["model"].as_str()).filter(|model| !model.is_empty());
                let mut ids: Vec<String> = Vec::new();
                if let Some(model) = configured {
                    ids.push(model.to_owned());
                    for id in config.and_then(|config| config["models"].as_array()).into_iter().flatten().filter_map(Value::as_str) {
                        if !ids.iter().any(|known| known == id) {
                            ids.push(id.to_owned());
                        }
                    }
                }
                ids.into_iter().map(|id| (id.clone(), id, None)).collect()
            }
        };
        let connected = match provider {
            Provider::ChatGpt => inputs.chatgpt_connected,
            Provider::Claude => inputs.claude_connected,
            Provider::Compatible => !models.is_empty(),
        };
        let models: Vec<Value> = models
            .into_iter()
            .map(|(id, label, discovered)| {
                let efforts: Vec<String> = discovered.unwrap_or_else(|| model_efforts(provider, Some(&id)).into_iter().map(str::to_owned).collect());
                let options = if efforts.is_empty() {
                    json!([])
                } else {
                    json!([{ "id": "reasoningEffort", "label": "Reasoning", "type": "select", "options": efforts.iter().map(|id| json!({ "id": id, "label": effort_label(id) })).collect::<Vec<_>>() }])
                };
                json!({ "id": id, "label": label, "options": options })
            })
            .collect();
        out.push(json!({
            "providerInstanceId": provider_id(provider),
            "driverKind": if provider == Provider::ChatGpt { "codex" } else { provider_id(provider) },
            "displayName": match provider { Provider::ChatGpt => "ChatGPT", Provider::Claude => "Claude", Provider::Compatible => "Connected model" },
            "canRunChildTask": connected,
            "canRunCrossProviderChildTask": connected,
            "constraints": if connected { json!([]) } else { json!(["Connect this provider in Settings before delegating work."]) },
            "models": models,
        }));
    }
    out
}

/// A validated `orchestrationTarget`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Target {
    pub provider: Option<Provider>,
    pub model: Option<String>,
    pub effort: Option<String>,
}

impl Target {
    pub fn is_empty(&self) -> bool {
        self.provider.is_none() && self.model.is_none() && self.effort.is_none()
    }

    /// `orchestrationTarget.parse` (strict).
    pub fn parse(value: &Value) -> anyhow::Result<Self> {
        let Some(map) = value.as_object() else { anyhow::bail!("target must be an object.") };
        if let Some(key) = map.keys().find(|key| !["providerInstanceId", "model", "options"].contains(&key.as_str())) {
            anyhow::bail!("Unknown target field: {key}");
        }
        let provider = match map.get("providerInstanceId") {
            None => None,
            Some(value) => Some(value.as_str().and_then(parse_provider).ok_or_else(|| anyhow::anyhow!("providerInstanceId must be chatgpt, claude or compatible."))?),
        };
        let model = match map.get("model") {
            None => None,
            Some(value) => Some(value.as_str().filter(|model| (1..=200).contains(&model.chars().count())).ok_or_else(|| anyhow::anyhow!("model must be 1-200 characters."))?.to_owned()),
        };
        let effort = match map.get("options") {
            None => None,
            Some(Value::Object(options)) => {
                if options.keys().any(|key| key != "reasoningEffort") {
                    anyhow::bail!("target.options only supports reasoningEffort.");
                }
                match options.get("reasoningEffort") {
                    None => None,
                    Some(value) => Some(value.as_str().filter(|effort| EFFORTS.contains(effort)).ok_or_else(|| anyhow::anyhow!("Unknown reasoningEffort."))?.to_owned()),
                }
            }
            Some(_) => anyhow::bail!("target.options must be an object."),
        };
        Ok(Self { provider, model, effort })
    }

    /// The canonical JSON used in retry fingerprints.
    pub fn to_json(&self) -> Value {
        let mut value = json!({});
        if let Some(provider) = self.provider {
            value["providerInstanceId"] = json!(provider_id(provider));
        }
        if let Some(model) = &self.model {
            value["model"] = json!(model);
        }
        if let Some(effort) = &self.effort {
            value["options"] = json!({ "reasoningEffort": effort });
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn efforts_follow_the_picker_rules() {
        assert_eq!(model_efforts(Provider::ChatGpt, Some("gpt-6-sol")).last(), Some(&"ultra"));
        assert!(model_efforts(Provider::Claude, Some("haiku")).is_empty());
        assert_eq!(model_efforts(Provider::Compatible, Some("o3-mini")), ["low", "medium", "high"]);
        assert!(model_efforts(Provider::Compatible, Some("llama3")).is_empty());
        assert_eq!(model_effort(Provider::Claude, Some("claude-opus-4-6"), Some("xhigh")).as_deref(), Some("high"));
        assert_eq!(model_effort(Provider::Compatible, Some("llama3"), Some("high")), None);
    }

    #[test]
    fn catalog_marks_disconnected_providers() {
        let providers = providers(&ProviderInputs {
            chatgpt_connected: true,
            chatgpt_models: vec![json!({ "id": "gpt-7", "label": "GPT 7", "reasoningEfforts": ["low"] })],
            claude_connected: false,
            claude_models: vec![],
            compatible: Some(json!({ "model": "llama3", "models": ["llama3", "qwen"] })),
        });
        assert_eq!(providers[0]["canRunChildTask"], true);
        let gpt7 = providers[0]["models"].as_array().unwrap().iter().find(|model| model["id"] == "gpt-7").unwrap();
        assert_eq!(gpt7["options"][0]["options"][0]["id"], "low");
        assert_eq!(providers[1]["canRunChildTask"], false);
        assert_eq!(providers[2]["models"].as_array().unwrap().len(), 2);
        assert!(Target::parse(&json!({ "providerInstanceId": "x" })).is_err());
        assert!(Target::parse(&json!({ "extra": 1 })).is_err());
    }
}
