//! Settings › Providers as the Powermove harness (Aero) runs on them:
//! OpenAI (Codex's ChatGPT sign-in, or an OpenAI API key Codex saved),
//! Anthropic (an API key, on Anthropic Messages with each model's thinking
//! controls),
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

/// How a Claude model takes the picker's level (the claude-api skill's model
/// table, 2026-11-03).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Thinking {
    /// `thinking: adaptive`, its summary shown, and `output_config.effort`.
    Adaptive,
    /// Opus 4.6 and Sonnet 4.6: the same, whose summary is already the default.
    AdaptiveSummarized,
    /// Opus 4.5: a thinking budget, and the effort it also takes.
    BudgetAndEffort,
    /// Sonnet 4.5 and Haiku 4.5: a thinking budget only.
    Budget,
}
/// Model family → its control → the levels the picker offers, in order. The
/// one place Claude levels are decided (lane NOTES has the same table).
/// Families not listed (retired models) are offered no level.
const CLAUDE: &[(&[&str], Thinking, &[&str])] = &[
    (
        &[
            "claude-fable-5",
            "claude-mythos-5",
            "claude-opus-5",
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-sonnet-5",
            "claude-haiku-5",
        ],
        Thinking::Adaptive,
        &["low", "medium", "high", "xhigh", "max"],
    ),
    (
        &["claude-opus-4-6", "claude-sonnet-4-6"],
        Thinking::AdaptiveSummarized,
        &["low", "medium", "high", "max"],
    ),
    (
        &["claude-opus-4-5"],
        Thinking::BudgetAndEffort,
        &["low", "medium", "high"],
    ),
    (
        &["claude-sonnet-4-5", "claude-haiku-4-5"],
        Thinking::Budget,
        &["low", "medium", "high"],
    ),
];
/// The output limit every listed Claude model is sent: room for thinking at
/// the top levels, above each budget below, within each model's own limit
/// (64K on the 4.5 models).
pub const CLAUDE_MAX_TOKENS: u32 = 64_000;
// The largest thinking budget below (High) must stay under it.
const _: () = assert!(32_768 < CLAUDE_MAX_TOKENS);
fn claude(model: &str) -> Option<(Thinking, &'static [&'static str])> {
    CLAUDE.iter().find_map(|(families, thinking, levels)| {
        families
            .iter()
            .any(|f| {
                model
                    .strip_prefix(f)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
            })
            .then_some((*thinking, *levels))
    })
}
/// The levels the picker offers for Claude `model`.
pub fn claude_efforts(model: &str) -> Vec<String> {
    claude(model).map_or_else(Vec::new, |(_, levels)| {
        levels.iter().map(|l| (*l).to_owned()).collect()
    })
}
/// The Anthropic Messages fields that choose `level` on `model`.
pub fn claude_effort_fields(model: &str, level: &str) -> Option<Value> {
    let (thinking, levels) = claude(model)?;
    levels.contains(&level).then_some(())?;
    let budget = match level {
        "low" => 4_096,
        "medium" => 16_384,
        _ => 32_768,
    };
    Some(match thinking {
        Thinking::Adaptive => json!({"thinking":{"type":"adaptive","display":"summarized"},
            "output_config":{"effort":level}}),
        Thinking::AdaptiveSummarized => {
            json!({"thinking":{"type":"adaptive"},"output_config":{"effort":level}})
        }
        Thinking::BudgetAndEffort => json!({"thinking":{"type":"enabled","budget_tokens":budget},
            "output_config":{"effort":level}}),
        Thinking::Budget => json!({"thinking":{"type":"enabled","budget_tokens":budget}}),
    })
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
                    claude_efforts(id)
                },
                default: false,
                aliases: vec![],
                // Claude's own default is lower on some models (medium on
                // Opus 5.5); agent work runs at high.
                default_effort: (source == Source::AnthropicKey
                    && claude_efforts(id).iter().any(|l| l == "high"))
                .then(|| "high".to_owned()),
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
    let net: Arc<dyn Net> = Arc::new(crate::edge::Idle {
        inner: env
            .net
            .clone()
            .ok_or("Powermove's network client is unavailable.")?,
        after: crate::edge::STREAM_IDLE,
    });
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
            let identity = Arc::new(crate::codex_auth::Identity::new());
            let scope = identity.clone();
            Live {
                identity: Some(identity),
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
                    // The account this turn started on, never another one (P1-4).
                    scope.check(account.as_deref().unwrap_or("chatgpt"))?;
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
            let removed = || "Add an OpenAI API key in Settings › Providers.".to_owned();
            crate::codex_auth::read(secrets.as_ref(), env)?
                .and_then(|a| a.api_key)
                .ok_or_else(removed)?;
            let env = env.clone();
            let identity = Arc::new(crate::codex_auth::Identity::new());
            let scope = identity.clone();
            Live {
                identity: Some(identity),
                provider: "openai".into(),
                configured: configured("openai", OPENAI_API, &effort_list(effort)),
                model: model.into(),
                catalog: None,
                net,
                // Read for every request: a removed or replaced key is never used.
                credentials: Arc::new(move |_| {
                    let key = crate::codex_auth::read(secrets.as_ref(), &env)?
                        .and_then(|a| a.api_key)
                        .ok_or_else(removed)?;
                    scope.check(&key_id(&key))?;
                    Ok((
                        "https://api.openai.com".into(),
                        vec![("authorization".into(), format!("Bearer {key}"))],
                    ))
                }),
            }
        }
        Source::AnthropicKey => {
            let removed = || "Add an Anthropic API key in Settings › Providers.".to_owned();
            let started = key_id(&anthropic_key(secrets.as_ref(), env).ok_or_else(removed)?);
            let env = env.clone();
            Live {
                identity: None,
                provider: "anthropic".into(),
                configured: anthropic_configured(model, effort),
                model: model.into(),
                catalog: None,
                net,
                credentials: Arc::new(move |_| {
                    let key = anthropic_key(secrets.as_ref(), &env).ok_or_else(removed)?;
                    if key_id(&key) != started {
                        return Err("The Anthropic key changed during this turn. Send it again to use the new key.".into());
                    }
                    // The core adds `anthropic-version`; the key goes as
                    // Anthropic reads it, never as a bearer.
                    Ok((
                        "https://api.anthropic.com".into(),
                        vec![("x-api-key".into(), key)],
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
                identity: None,
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

/// The core's Anthropic Messages connection for Claude `model`: every level
/// the model offers with its fields (`claude_effort_fields`), the output
/// limit, and prompt caching (the automatic breakpoint moves to the newest
/// block each turn, so the tools, system prompt and history read from the
/// cache). A level the model doesn't offer is sent as the nearest one it does.
/// Tool choice is left to the model: Claude's current models refuse a forced one.
fn anthropic_configured(model: &str, effort: &str) -> String {
    let levels = claude_efforts(model);
    let mut efforts: serde_json::Map<String, Value> = levels
        .iter()
        .filter_map(|l| Some((l.clone(), claude_effort_fields(model, l)?)))
        .collect();
    if !effort.is_empty()
        && !efforts.contains_key(effort)
        && let Some(nearest) = crate::compatible::nearest(&levels, Some(effort))
    {
        let fields = efforts[&nearest].clone();
        efforts.insert(effort.to_owned(), fields);
    }
    let mut entry = json!({"supports_tool_use": true, "supports_vision": true,
        "request": {"cache_control": {"type": "ephemeral"}}});
    if !efforts.is_empty() {
        entry["max_output_tokens"] = CLAUDE_MAX_TOKENS.into();
        entry["reasoning_efforts"] = efforts.into();
    }
    json!({"anthropic": {"protocol":"anthropic-messages","base_url":ANTHROPIC_API,
        "auth":{"type":"none"},"tool_choice_mode":"omit","model_metadata":{model: entry}}})
    .to_string()
}

/// A key's identity for comparing, never the key itself.
fn key_id(key: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("key:{:x}", Sha256::digest(key.as_bytes()))
}

/// fx's Codex reviewer model: the core refuses a Codex catalog that does
/// not list it (`vendor/fx/src/gateway/openai_codex_models.zig:20`, `:141`),
/// and a refused catalog drops the picked effort at every level.
const CODEX_REVIEWER: &str = "gpt-5.6-luna";

/// What the core's Codex catalog asks for (the npm version lookup, then
/// `backend-api/codex/models`), answered here with the chosen model and its
/// one level (the account's list already chose them), plus the reviewer
/// model the core requires.
fn codex_catalog(model: &str, effort: &str) -> Value {
    let levels: Vec<Value> = if effort.is_empty() {
        vec![]
    } else {
        vec![json!({"effort": effort})]
    };
    let entry = |slug: &str, priority: u32, levels: Vec<Value>| {
        json!({"slug":slug,"visibility":"list","supported_in_api":true,"priority":priority,
            "supported_reasoning_levels":levels,"additional_speed_tiers":[],
            "input_modalities":["text","image"],"context_window":272000})
    };
    let mut models = vec![entry(model, 1, levels)];
    if model != CODEX_REVIEWER {
        models.push(entry(CODEX_REVIEWER, 2, vec![]));
    }
    json!({"version": crate::runtime::CODEX_VERSION, "models": models})
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

    /// One table decides Claude's levels: each family the levels its API
    /// takes, the fields for each, and a budget under the output limit.
    #[test]
    fn each_claude_family_takes_the_levels_and_fields_its_api_documents() {
        let five = ["low", "medium", "high", "xhigh", "max"];
        for model in [
            "claude-opus-5-5",
            "claude-opus-5",
            "claude-fable-5-2",
            "claude-sonnet-5-5",
            "claude-haiku-5-5",
            "claude-opus-4-8",
            "claude-opus-4-7",
        ] {
            assert_eq!(claude_efforts(model), five, "{model}");
        }
        assert_eq!(
            claude_efforts("claude-sonnet-4-6"),
            ["low", "medium", "high", "max"]
        );
        assert_eq!(
            claude_efforts("claude-haiku-4-5-20251001"),
            ["low", "medium", "high"]
        );
        for retired in [
            "claude-3-7-sonnet-20250219",
            "claude-opus-4-1-20250805",
            "gpt-6-sol",
        ] {
            assert!(claude_efforts(retired).is_empty(), "{retired}");
        }
        assert_eq!(
            claude_effort_fields("claude-opus-5-5", "xhigh"),
            Some(
                json!({"thinking":{"type":"adaptive","display":"summarized"},
                "output_config":{"effort":"xhigh"}})
            )
        );
        assert_eq!(
            claude_effort_fields("claude-opus-4-6", "max"),
            Some(json!({"thinking":{"type":"adaptive"},"output_config":{"effort":"max"}}))
        );
        assert_eq!(
            claude_effort_fields("claude-opus-4-5-20251101", "medium"),
            Some(json!({"thinking":{"type":"enabled","budget_tokens":16384},
                "output_config":{"effort":"medium"}}))
        );
        assert_eq!(
            claude_effort_fields("claude-haiku-4-5", "high"),
            Some(json!({"thinking":{"type":"enabled","budget_tokens":32768}}))
        );
        assert_eq!(claude_effort_fields("claude-opus-4-6", "xhigh"), None);
    }
}
