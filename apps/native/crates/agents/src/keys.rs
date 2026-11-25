//! The API keys Settings › Providers keeps for Claude Code and Codex: an
//! Anthropic key (Claude Code runs on it while signed out of Claude) and the
//! OpenAI key Codex saved in its own Keychain item (shown masked). Aero never
//! reads either: it runs only on Powermove inference (`crate::inference`).
use crate::account::Model;
use crate::edge::{Net, NetRequest, Secrets, fetch};
use crate::launch::Environment;
use serde_json::Value;

pub const ANTHROPIC_API: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Model family → the levels the picker offers Claude Code on it, in order
/// (the claude-api skill's model table, 2026-11-03). Families not listed
/// (retired models) are offered no level.
const CLAUDE: &[(&[&str], &[&str])] = &[
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
        &["low", "medium", "high", "xhigh", "max"],
    ),
    (
        &["claude-opus-4-6", "claude-sonnet-4-6"],
        &["low", "medium", "high", "max"],
    ),
    (
        &[
            "claude-opus-4-5",
            "claude-sonnet-4-5",
            "claude-haiku-4-5",
        ],
        &["low", "medium", "high"],
    ),
];
/// The levels the picker offers for Claude `model`.
pub fn claude_efforts(model: &str) -> Vec<String> {
    CLAUDE
        .iter()
        .find(|(families, _)| {
            families.iter().any(|f| {
                model
                    .strip_prefix(f)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
            })
        })
        .map_or_else(Vec::new, |(_, levels)| {
            levels.iter().map(|l| (*l).to_owned()).collect()
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

/// GET `/models` with the Anthropic key: the models Claude Code can run on
/// it (ids in the server's order), each with its levels.
pub fn anthropic_models(net: &dyn Net, key: &str) -> Result<Vec<Model>, String> {
    let url = format!("{ANTHROPIC_API}/models?limit=100");
    let headers = vec![
        ("x-api-key".to_owned(), key.to_owned()),
        ("anthropic-version".to_owned(), ANTHROPIC_VERSION.to_owned()),
    ];
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
            let efforts = claude_efforts(id);
            Some(Model {
                id: id.into(),
                label: m["display_name"]
                    .as_str()
                    .map_or_else(|| crate::account::codex_model_label(id), str::to_owned),
                // Claude's own default is lower on some models (medium on
                // Opus 5.5); agent work runs at high.
                default_effort: efforts.iter().any(|l| l == "high").then(|| "high".to_owned()),
                efforts,
                default: false,
                aliases: vec![],
            })
        })
        .collect::<Vec<_>>();
    if models.is_empty() {
        Err("The key lists no models Claude Code can run.".into())
    } else {
        Ok(models)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_masked() {
        assert_eq!(mask("sk-ant-api03-abcdefa1b2"), "sk-…a1b2");
        assert_eq!(mask("short"), "••••");
        assert!(valid_key("sk-ant-fake-abcdef12") && !valid_key("two words here"));
    }

    /// One table decides Claude Code's levels on an Anthropic key.
    #[test]
    fn each_claude_family_offers_the_levels_its_api_documents() {
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
    }
}
