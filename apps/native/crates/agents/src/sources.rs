//! Settings › Providers as the Powermove harness (Aero) runs on them:
//! OpenAI (Codex's ChatGPT sign-in, or an OpenAI API key Codex saved),
//! Anthropic (an API key, through Anthropic's OpenAI-compatible endpoint),
//! Custom (the saved OpenAI-compatible connection). A Powermove model id
//! names its source: `chatgpt/<slug>`, `openai/<id>`, `anthropic/<id>`,
//! `custom/<id>`. Credentials stay here and in the Keychain; the core gets
//! a provider description without any, and the fetch edge adds them for
//! the provider's own origin only.
use crate::account::Model;
use crate::aero::Live;
use crate::edge::{Net, NetRequest, Secrets, fetch};
use crate::launch::Environment;
use serde_json::{Value, json};
use std::sync::Arc;

/// The development switch's demo model (`PM_AERO_FAKE_MODEL=1` lists it).
pub const FAKE_MODEL: &str = "fake/model";
pub const OPENAI_API: &str = "https://api.openai.com/v1";
pub const ANTHROPIC_API: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Where a Powermove model runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    ChatGpt,
    OpenAiKey,
    AnthropicKey,
    Custom,
}
impl Source {
    pub fn prefix(self) -> &'static str {
        match self {
            Self::ChatGpt => "chatgpt",
            Self::OpenAiKey => "openai",
            Self::AnthropicKey => "anthropic",
            Self::Custom => "custom",
        }
    }
    /// The picker's group (the provider in Settings › Providers).
    pub fn group(self) -> &'static str {
        match self {
            Self::ChatGpt | Self::OpenAiKey => "OpenAI",
            Self::AnthropicKey => "Anthropic",
            Self::Custom => "Custom",
        }
    }
    pub fn of(id: &str) -> Option<(Self, &str)> {
        let (prefix, model) = id.split_once('/')?;
        let source = [
            Self::ChatGpt,
            Self::OpenAiKey,
            Self::AnthropicKey,
            Self::Custom,
        ]
        .into_iter()
        .find(|s| s.prefix() == prefix)?;
        (!model.is_empty()).then_some((source, model))
    }
}

/// `sk-…a1b2`: a key as Settings shows it.
pub fn mask(key: &str) -> String {
    let tail: String = key
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let head: String = key.chars().take(3).collect();
    if key.chars().count() <= 8 {
        "••••".into()
    } else {
        format!("{head}…{tail}")
    }
}

pub fn anthropic_service(env: &Environment) -> String {
    format!("{} Anthropic API Key", env.keychain_prefix)
}
pub fn anthropic_key(secrets: &dyn Secrets, env: &Environment) -> Option<String> {
    let bytes = secrets.read(&anthropic_service(env), "api-key").ok()??;
    String::from_utf8(bytes).ok().filter(|k| !k.is_empty())
}
/// A plausible key: what a person pasted, trimmed, one line, no spaces.
pub fn valid_key(key: &str) -> bool {
    (8..=512).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_graphic())
}

/// GET `{api}/models`, the key's own list (ids in the server's order).
pub fn key_models(net: &dyn Net, source: Source, key: &str) -> Result<Vec<Model>, String> {
    let (url, headers) = match source {
        Source::OpenAiKey => (
            format!("{OPENAI_API}/models"),
            vec![("Authorization".to_owned(), format!("Bearer {key}"))],
        ),
        Source::AnthropicKey => (
            format!("{ANTHROPIC_API}/models?limit=100"),
            vec![
                ("x-api-key".to_owned(), key.to_owned()),
                ("anthropic-version".to_owned(), ANTHROPIC_VERSION.to_owned()),
            ],
        ),
        _ => return Err("Only API keys list models here.".into()),
    };
    let (status, body) = fetch(
        net,
        &NetRequest {
            method: "GET",
            url: &url,
            headers: &headers,
            body: b"",
        },
    )?;
    if status == 401 || status == 403 {
        return Err("The API key was refused.".into());
    }
    if status != 200 {
        return Err(format!("The model list answered HTTP {status}."));
    }
    let value: Value =
        serde_json::from_slice(&body).map_err(|_| "The model list is unreadable.")?;
    let models = value["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = m["id"]
                .as_str()
                .filter(|id| crate::launch::is_valid_model_id(id))?;
            if source == Source::OpenAiKey && !chat_model(id) {
                return None;
            }
            Some(Model {
                id: id.into(),
                label: m["display_name"]
                    .as_str()
                    .map_or_else(|| crate::account::codex_model_label(id), str::to_owned),
                efforts: if source == Source::OpenAiKey {
                    crate::compatible::efforts(id)
                } else {
                    vec![]
                },
                default: false,
                aliases: vec![],
                default_effort: None,
            })
        })
        .collect::<Vec<_>>();
    if models.is_empty() {
        Err("The key lists no models Powermove can run.".into())
    } else {
        Ok(models)
    }
}
/// OpenAI's list holds every kind of model; Powermove runs the chat ones.
fn chat_model(id: &str) -> bool {
    let chat = id.starts_with("gpt-")
        || (id.starts_with('o') && id[1..].starts_with(|c: char| c.is_ascii_digit()));
    let other = [
        "audio",
        "realtime",
        "image",
        "tts",
        "transcribe",
        "embedding",
        "search",
        "moderation",
        "instruct",
    ];
    chat && !other.iter().any(|o| id.contains(o))
}

/// The model edge for Powermove model `id` at `effort`, with the signed-in
/// method's credential, or why it can't run.
pub fn live(env: &Environment, id: &str, effort: &str) -> Result<Live, String> {
    let (source, model) = Source::of(id).ok_or("Choose a model in the picker.")?;
    let secrets = env.secrets.clone().ok_or("The Keychain is unavailable.")?;
    let net = env
        .net
        .clone()
        .ok_or("Powermove's network client is unavailable.")?;
    let metadata = |efforts: &[String]| {
        let mut entry = json!({"supports_tool_use": true, "supports_vision": true});
        if !efforts.is_empty() {
            entry["reasoning_efforts"] = efforts
                .iter()
                .map(|e| (e.clone(), json!({"reasoning_effort": e})))
                .collect::<serde_json::Map<_, _>>()
                .into();
        }
        entry
    };
    let configured = |name: &str, base: &str, efforts: &[String]| {
        json!({name: {"protocol":"openai-chat-completions","base_url":base,"auth":{"type":"none"},
            "tool_choice_mode":"send","model_metadata":{model: metadata(efforts)}}})
        .to_string()
    };
    let effort_list = |e: &str| {
        if e.is_empty() {
            vec![]
        } else {
            vec![e.to_owned()]
        }
    };
    Ok(match source {
        Source::ChatGpt => {
            let env = env.clone();
            Live {
                provider: "codex".into(),
                configured: String::new(),
                model: model.into(),
                catalog: Some(codex_catalog(model, effort)),
                net,
                credentials: Arc::new(move |rejected| {
                    let auth = crate::codex_auth::current(secrets.as_ref(), &env, rejected)?;
                    let (access, account) = auth
                        .chatgpt
                        .ok_or("OpenAI is signed in with an API key, not ChatGPT.")?;
                    let mut headers =
                        vec![("authorization".to_owned(), format!("Bearer {access}"))];
                    if let Some(account) = account {
                        headers.push(("chatgpt-account-id".into(), account));
                    }
                    Ok(("https://chatgpt.com".into(), headers))
                }),
            }
        }
        Source::OpenAiKey => {
            let key = crate::codex_auth::read(secrets.as_ref(), env)?
                .and_then(|a| a.api_key)
                .ok_or("Add an OpenAI API key in Settings › Providers.")?;
            Live {
                provider: "openai".into(),
                configured: configured("openai", OPENAI_API, &effort_list(effort)),
                model: model.into(),
                catalog: None,
                net,
                credentials: Arc::new(move |_| {
                    Ok((
                        "https://api.openai.com".into(),
                        vec![("authorization".into(), format!("Bearer {key}"))],
                    ))
                }),
            }
        }
        Source::AnthropicKey => {
            let key = anthropic_key(secrets.as_ref(), env)
                .ok_or("Add an Anthropic API key in Settings › Providers.")?;
            Live {
                provider: "anthropic".into(),
                configured: configured("anthropic", ANTHROPIC_API, &[]),
                model: model.into(),
                catalog: None,
                net,
                credentials: Arc::new(move |_| {
                    Ok((
                        "https://api.anthropic.com".into(),
                        vec![
                            ("authorization".into(), format!("Bearer {key}")),
                            ("x-api-key".into(), key.clone()),
                        ],
                    ))
                }),
            }
        }
        Source::Custom => {
            let host = env
                .compatible
                .clone()
                .ok_or("Set up a custom endpoint in Settings › Providers.")?;
            let saved = host
                .saved()
                .ok_or("Set up a custom endpoint in Settings › Providers.")?;
            let key = host.key(&saved.base_url);
            let origin = pm_aero_core::origin_of(&saved.base_url)
                .ok_or("The custom endpoint's address is invalid.")?;
            Live {
                provider: "custom".into(),
                configured: configured(
                    "custom",
                    saved.base_url.trim_end_matches('/'),
                    &effort_list(effort),
                ),
                model: model.into(),
                catalog: None,
                net,
                credentials: Arc::new(move |_| {
                    Ok((
                        origin.clone(),
                        key.iter()
                            .map(|k| ("authorization".to_owned(), format!("Bearer {k}")))
                            .collect(),
                    ))
                }),
            }
        }
    })
}

/// What the core's Codex catalog asks for (the npm version lookup, then
/// `backend-api/codex/models`), answered here with the chosen model alone:
/// the account's list already chose it, and nothing else may run.
fn codex_catalog(model: &str, effort: &str) -> Value {
    let levels: Vec<Value> = if effort.is_empty() {
        vec![]
    } else {
        vec![json!({"effort": effort})]
    };
    json!({"version": crate::runtime::CODEX_VERSION, "models":[{"slug":model,"visibility":"list","supported_in_api":true,
        "priority":1,"supported_reasoning_levels":levels,"additional_speed_tiers":[],"input_modalities":["text","image"],
        "context_window":272000}]})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_name_their_source_and_keys_are_masked() {
        assert_eq!(
            Source::of("chatgpt/gpt-6-sol"),
            Some((Source::ChatGpt, "gpt-6-sol"))
        );
        assert_eq!(
            Source::of("anthropic/claude-opus-5-5").map(|s| s.0.group()),
            Some("Anthropic")
        );
        assert_eq!(Source::of("gpt-6-sol"), None);
        assert_eq!(Source::of("custom/"), None);
        assert_eq!(mask("sk-ant-api03-abcdefa1b2"), "sk-…a1b2");
        assert_eq!(mask("short"), "••••");
        assert!(
            chat_model("gpt-6-sol")
                && chat_model("o4-mini")
                && !chat_model("gpt-4o-realtime")
                && !chat_model("omni-moderation")
        );
    }
}
