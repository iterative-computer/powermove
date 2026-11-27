//! A01 backing at c7f46a05641b23944d3692c9d80f9677c7e1ef1d:
//! shared/compatible-provider.ts and main/compatible-provider.ts.
//! Discovery/context-warning presentation follows S071 at 78294c4c9.
//! This module exposes facts; native hosts render existing kit/tokens.
use super::{
    Flight, Ticket,
    agent_auth::{Secret, provider_url},
};
use crate::{Result, ServiceError, ServiceErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetKind {
    Local,
    Cloud,
    Other,
}
#[derive(Debug, Clone, Copy)]
pub struct Preset {
    pub id: &'static str,
    pub endpoint: &'static str,
    pub kind: PresetKind,
}
pub const PRESETS: &[Preset] = &[
    Preset {
        id: "ollama",
        endpoint: "http://localhost:11434/v1",
        kind: PresetKind::Local,
    },
    Preset {
        id: "lmstudio",
        endpoint: "http://localhost:1234/v1",
        kind: PresetKind::Local,
    },
    Preset {
        id: "llamacpp",
        endpoint: "http://localhost:8080/v1",
        kind: PresetKind::Local,
    },
    Preset {
        id: "jan",
        endpoint: "http://localhost:1337/v1",
        kind: PresetKind::Local,
    },
    Preset {
        id: "openrouter",
        endpoint: "https://openrouter.ai/api/v1",
        kind: PresetKind::Cloud,
    },
    Preset {
        id: "openai",
        endpoint: "https://api.openai.com/v1",
        kind: PresetKind::Cloud,
    },
    Preset {
        id: "anthropic",
        endpoint: "https://api.anthropic.com/v1",
        kind: PresetKind::Cloud,
    },
    Preset {
        id: "gemini",
        endpoint: "https://generativelanguage.googleapis.com/v1beta/openai",
        kind: PresetKind::Cloud,
    },
    Preset {
        id: "groq",
        endpoint: "https://api.groq.com/openai/v1",
        kind: PresetKind::Cloud,
    },
    Preset {
        id: "deepseek",
        endpoint: "https://api.deepseek.com/v1",
        kind: PresetKind::Cloud,
    },
    Preset {
        id: "xai",
        endpoint: "https://api.x.ai/v1",
        kind: PresetKind::Cloud,
    },
    Preset {
        id: "mistral",
        endpoint: "https://api.mistral.ai/v1",
        kind: PresetKind::Cloud,
    },
    Preset {
        id: "other",
        endpoint: "",
        kind: PresetKind::Other,
    },
];
pub fn preset_for(endpoint: &str) -> &'static Preset {
    let normalized = provider_url(endpoint);
    PRESETS
        .iter()
        .find(|p| !p.endpoint.is_empty() && normalized.as_deref() == Some(p.endpoint))
        .unwrap_or(PRESETS.last().unwrap())
}
pub fn is_local(endpoint: &str) -> bool {
    provider_url(endpoint)
        .and_then(|s| url::Url::parse(&s).ok())
        .is_some_and(|url| matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
}

/// K is an opaque encrypted payload/Keychain reference, never serialized here.
/// Keep the 24 recent addresses active as main does, retaining displaced entries
/// in an archive (N4). Switching endpoints never forwards a different key.
pub struct AddressKeys<K> {
    active: Vec<(String, K)>,
    archived: Vec<(String, K)>,
    disabled: BTreeSet<String>,
}
impl<K> Default for AddressKeys<K> {
    fn default() -> Self {
        Self {
            active: vec![],
            archived: vec![],
            disabled: BTreeSet::new(),
        }
    }
}
impl<K> AddressKeys<K> {
    pub fn get(&self, endpoint: &str) -> Option<&K> {
        let endpoint = provider_url(endpoint)?;
        if self.disabled.contains(&endpoint) {
            return None;
        }
        self.active
            .iter()
            .rev()
            .chain(self.archived.iter().rev())
            .find(|(url, _)| url == &endpoint)
            .map(|(_, key)| key)
    }
    pub fn remember(&mut self, endpoint: &str, key: K) -> std::result::Result<(), K> {
        let Some(endpoint) = provider_url(endpoint) else {
            return Err(key);
        };
        self.disabled.remove(&endpoint);
        if let Some(i) = self.active.iter().position(|(url, _)| url == &endpoint) {
            self.archived.push(self.active.remove(i));
        }
        self.active.push((endpoint, key));
        while self.active.len() > 24 {
            self.archived.push(self.active.remove(0));
        }
        Ok(())
    }
    pub fn archive(&mut self, endpoint: &str) -> bool {
        let Some(endpoint) = provider_url(endpoint) else {
            return false;
        };
        self.disabled.insert(endpoint.clone());
        if let Some(i) = self.active.iter().position(|(url, _)| url == &endpoint) {
            self.archived.push(self.active.remove(i));
        }
        true
    }
    pub fn counts(&self) -> (usize, usize) {
        (self.active.len(), self.archived.len())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub vision: Option<bool>,
    pub tools: Option<bool>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelList {
    pub models: Vec<ModelInfo>,
    pub hidden: usize,
    pub has_key: bool,
}
fn valid_id(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| {
        !s.trim().is_empty()
            && s.encode_utf16().count() <= 200
            && !s.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
    })
}
pub fn models(
    payload: &Value,
    ollama: &BTreeMap<String, Vec<String>>,
    has_key: bool,
) -> Result<ModelList> {
    let data = payload["data"].as_array().ok_or_else(|| {
        ServiceError::message(
            ServiceErrorCode::InvalidData,
            "provider returned no model list",
        )
    })?;
    let mut seen = BTreeSet::new();
    let mut result = ModelList {
        models: vec![],
        hidden: 0,
        has_key,
    };
    for entry in data.iter().filter(|v| v.is_object()).take(2000) {
        let Some(id) = valid_id(&entry["id"]) else {
            continue;
        };
        if !seen.insert(id) {
            continue;
        }
        let caps = ollama.get(id);
        let parameters = entry["supported_parameters"].as_array();
        let tools = caps
            .map(|c| c.iter().any(|s| s == "tools"))
            .or_else(|| parameters.map(|p| p.iter().any(|s| s == "tools")));
        if tools == Some(false) {
            result.hidden += 1;
            continue;
        }
        let vision = caps.map(|c| c.iter().any(|s| s == "vision")).or_else(|| {
            entry["architecture"]["input_modalities"]
                .as_array()
                .map(|m| m.iter().any(|s| s == "image"))
        });
        result.models.push(ModelInfo {
            id: id.into(),
            tools,
            vision,
        });
    }
    Ok(result)
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextWarning {
    pub model: String,
    pub tokens: u64,
    pub minimum: u64,
    pub recommended: u64,
}
pub fn ollama_context(payload: &Value, model: &str) -> Option<ContextWarning> {
    let same = |value: &Value| {
        value.as_str().is_some_and(|name| {
            name == model || name == format!("{model}:latest") || format!("{name}:latest") == model
        })
    };
    let running = payload["models"]
        .as_array()?
        .iter()
        .find(|item| same(&item["name"]) || same(&item["model"]))?;
    let tokens = running["context_length"]
        .as_u64()
        .or_else(|| running["context_length"].as_str()?.parse().ok())?;
    (tokens > 0 && tokens < 16384).then(|| ContextWarning {
        model: model.into(),
        tokens,
        minimum: 16384,
        recommended: 32768,
    })
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderFailure {
    pub status: u16,
    pub detail: String,
    pub tools_unsupported: bool,
}
pub fn http_failure(status: u16, body: &str) -> ProviderFailure {
    let body: String = body.chars().take(16000).collect();
    let text = match serde_json::from_str::<Value>(&body) {
        Ok(value) => {
            let error = value
                .get("error")
                .or_else(|| value.get("message"))
                .or_else(|| value.get("detail"));
            error
                .and_then(|e| {
                    e.as_str()
                        .or_else(|| e.get("message").and_then(Value::as_str))
                })
                .unwrap_or("")
                .to_owned()
        }
        Err(_) => body,
    };
    let text = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>();
    let text = if text.to_ascii_lowercase().starts_with("<!doctype")
        || text.to_ascii_lowercase().starts_with("<html")
    {
        String::new()
    } else {
        text
    };
    let detail = if text.encode_utf16().count() > 240 {
        let mut s = String::new();
        for c in text.chars() {
            if s.encode_utf16().count() + c.len_utf16() > 239 {
                break;
            }
            s.push(c);
        }
        s.push('…');
        s
    } else {
        text
    };
    let lower = detail.to_ascii_lowercase();
    let tools_unsupported = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|s| s == "tool" || s == "tools")
        && ["support", "enable", "allow"]
            .iter()
            .any(|s| lower.contains(s));
    ProviderFailure {
        status,
        detail,
        tools_unsupported,
    }
}

pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}
pub trait DiscoveryHost: Send + 'static {
    /// Credential only for this normalized endpoint; refuse redirects, enforce
    /// 15 s and 20M-character response bounds while streaming. Never log keys.
    fn list(&mut self, endpoint: &str, key: Option<&Secret>) -> Result<Response>;
    /// Native worker probes /api/version, then /api/show for at most 60 IDs,
    /// concurrently with 1.5/5 s limits. Unknown capabilities remain unknown.
    fn ollama_capabilities(
        &mut self,
        endpoint: &str,
        ids: &[String],
    ) -> Result<BTreeMap<String, Vec<String>>>;
    fn sort_local(&self, models: &mut [ModelInfo]);
}
pub struct DiscoveryWork {
    pub ticket: Ticket,
    pub endpoint: String,
    pub key: Option<Secret>,
    pub has_saved_key: bool,
}
pub struct DiscoveryReply {
    pub ticket: Ticket,
    pub endpoint: String,
    pub result: Result<ModelList>,
}
impl DiscoveryWork {
    pub fn run(self, host: &mut impl DiscoveryHost) -> DiscoveryReply {
        let result = (|| {
            let endpoint = provider_url(&self.endpoint).ok_or_else(|| {
                ServiceError::message(ServiceErrorCode::InvalidInput, "invalid provider endpoint")
            })?;
            let response = host.list(&endpoint, self.key.as_ref())?;
            if matches!(response.status, 404 | 405) {
                return Ok(ModelList {
                    models: vec![],
                    hidden: 0,
                    has_key: self.has_saved_key,
                });
            }
            if response.status != 200 {
                return Err(ServiceError::message(
                    if matches!(response.status, 401 | 403) {
                        ServiceErrorCode::PermissionDenied
                    } else {
                        ServiceErrorCode::Unavailable
                    },
                    "provider model request failed",
                ));
            }
            if response.body.len() > 80_000_000 {
                return Err(ServiceError::message(
                    ServiceErrorCode::TooLarge,
                    "provider model list too large",
                ));
            }
            let text = std::str::from_utf8(&response.body).map_err(|_| {
                ServiceError::message(ServiceErrorCode::InvalidData, "invalid provider response")
            })?;
            if text.encode_utf16().count() > 20_000_000 {
                return Err(ServiceError::message(
                    ServiceErrorCode::TooLarge,
                    "provider model list too large",
                ));
            }
            let payload: Value = serde_json::from_str(text).map_err(|_| {
                ServiceError::message(ServiceErrorCode::InvalidData, "invalid provider model list")
            })?;
            let entries = payload["data"].as_array().ok_or_else(|| {
                ServiceError::message(
                    ServiceErrorCode::InvalidData,
                    "provider returned no model list",
                )
            })?;
            let mut seen = BTreeSet::new();
            let ids = entries
                .iter()
                .filter(|v| v.is_object())
                .take(2000)
                .filter_map(|v| valid_id(&v["id"]))
                .filter(|id| seen.insert(*id))
                .take(60)
                .map(String::from)
                .collect::<Vec<_>>();
            let caps = if is_local(&endpoint) {
                host.ollama_capabilities(&endpoint, &ids)
                    .unwrap_or_default()
            } else {
                BTreeMap::new()
            };
            let mut result = models(&payload, &caps, self.has_saved_key)?;
            if is_local(&endpoint) {
                host.sort_local(&mut result.models);
            }
            Ok(result)
        })();
        DiscoveryReply {
            ticket: self.ticket,
            endpoint: self.endpoint,
            result,
        }
    }
}
#[derive(Debug, Default)]
pub struct Discovery {
    flight: Flight,
    endpoint: Option<String>,
    pub result: Option<ModelList>,
    pub error: Option<ServiceErrorCode>,
}
impl Discovery {
    pub fn start(
        &mut self,
        endpoint: &str,
        key: Option<Secret>,
        has_saved_key: bool,
    ) -> Option<DiscoveryWork> {
        let endpoint = provider_url(endpoint)?;
        if self.endpoint.as_ref() != Some(&endpoint) {
            self.result = None;
        }
        self.endpoint = Some(endpoint.clone());
        self.error = None;
        Some(DiscoveryWork {
            ticket: self.flight.start(),
            endpoint,
            key,
            has_saved_key,
        })
    }
    pub fn receive(&mut self, reply: DiscoveryReply) -> bool {
        if !self.flight.finish(reply.ticket) || self.endpoint.as_ref() != Some(&reply.endpoint) {
            return false;
        }
        match reply.result {
            Ok(result) => self.result = Some(result),
            Err(error) => self.error = Some(error.code),
        }
        true
    }
    pub fn cancel(&mut self) {
        self.flight.cancel();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Configured {
    pub endpoint: String,
    pub model: String,
    pub vision: bool,
    pub has_key: bool,
    pub models: Vec<String>,
    pub warning: Option<ContextWarning>,
}
pub trait ConfigureHost: Send + 'static {
    /// POST /chat/completions; no redirects, 60 s timeout, bounded response.
    fn test(&mut self, endpoint: &str, key: Option<&Secret>, body: Value) -> Result<Response>;
    /// Optional /models lookup: 10 s; failure falls back to the configured model.
    fn models(&mut self, endpoint: &str, key: Option<&Secret>) -> Result<Value>;
    /// Only after actual Ollama detection. Failures produce no inferred warning.
    fn running_ollama(&mut self, endpoint: &str) -> Result<Option<Value>>;
    /// Atomic config + per-address secure key publication, archiving replaced
    /// records. Encryption/Keychain failure leaves the old connection intact.
    fn publish(&mut self, config: &Configured, key: Option<&Secret>) -> Result<()>;
}
pub struct ConfigureWork {
    pub ticket: Ticket,
    pub input: super::agent_auth::ProviderInput,
}
pub struct ConfigureReply {
    pub ticket: Ticket,
    pub result: Result<Configured>,
}
impl ConfigureWork {
    pub fn run(self, host: &mut impl ConfigureHost) -> ConfigureReply {
        let result = (|| {
            let input = self.input;
            let body = serde_json::json!({"model":input.model,"messages":[{"role":"user","content":"Reply with OK."}],"tools":[{"type":"function","function":{"name":"powermove_check","description":"Not needed; reply with text.","parameters":{"type":"object","properties":{}}}}],"stream":false});
            let response = host.test(&input.base_url, input.key.as_ref(), body)?;
            if response.status != 200 {
                return Err(ServiceError::message(
                    if matches!(response.status, 401 | 403) {
                        ServiceErrorCode::PermissionDenied
                    } else {
                        ServiceErrorCode::Unavailable
                    },
                    "provider connection test failed",
                ));
            }
            if response.body.len() > 2_000_000 {
                return Err(ServiceError::message(
                    ServiceErrorCode::TooLarge,
                    "provider test response too large",
                ));
            }
            let response: Value = serde_json::from_slice(&response.body).map_err(|_| {
                ServiceError::message(
                    ServiceErrorCode::InvalidData,
                    "invalid provider chat response",
                )
            })?;
            if !response["choices"][0]["message"].is_object() {
                return Err(ServiceError::message(
                    ServiceErrorCode::InvalidData,
                    "provider returned no chat message",
                ));
            }
            let mut names = vec![input.model.clone()];
            let mut seen = BTreeSet::from([input.model.clone()]);
            if let Ok(payload) = host.models(&input.base_url, input.key.as_ref())
                && let Some(entries) = payload["data"].as_array()
            {
                for id in entries.iter().filter_map(|v| valid_id(&v["id"])).take(1000) {
                    if seen.insert(id.into()) {
                        names.push(id.into());
                    }
                }
            }
            let warning = if is_local(&input.base_url) {
                host.running_ollama(&input.base_url)
                    .ok()
                    .flatten()
                    .and_then(|v| ollama_context(&v, &input.model))
            } else {
                None
            };
            let config = Configured {
                endpoint: input.base_url,
                model: input.model,
                vision: input.vision,
                has_key: input.key.is_some(),
                models: names,
                warning,
            };
            host.publish(&config, input.key.as_ref())?;
            Ok(config)
        })();
        ConfigureReply {
            ticket: self.ticket,
            result,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalServer {
    pub preset: String,
    pub endpoint: String,
    pub models: usize,
}
/// Fixed local presets only; no arbitrary LAN scan or optimistic installed flag.
pub fn detected_local(preset: &str, payload: &Value) -> Option<LocalServer> {
    let preset = PRESETS
        .iter()
        .find(|p| p.id == preset && p.kind == PresetKind::Local)?;
    Some(LocalServer {
        preset: preset.id.into(),
        endpoint: preset.endpoint.into(),
        models: payload["data"].as_array()?.len(),
    })
}

/// A03's changed Project Auto mode. The actual pm-agents argument/permission
/// owner must supply measured runtime support; unknown is never advertised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudePermissionMode {
    AcceptEdits,
    Auto,
    Unavailable,
}
pub fn claude_project_permission_mode(
    approval: Option<&str>,
    auto_supported: bool,
) -> ClaudePermissionMode {
    if approval.unwrap_or("auto") == "auto" {
        if auto_supported {
            ClaudePermissionMode::Auto
        } else {
            ClaudePermissionMode::Unavailable
        }
    } else {
        ClaudePermissionMode::AcceptEdits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn new_reference_accepts_bare_addresses_without_relaxing_transport() {
        assert_eq!(
            provider_url("localhost:11434"),
            Some("http://localhost:11434/v1".into())
        );
        assert_eq!(
            provider_url("example.com/gateway/chat/completions/"),
            Some("https://example.com/gateway".into())
        );
        assert!(provider_url("http://example.com").is_none());
        assert_eq!(preset_for("localhost:8080").id, "llamacpp");
        assert_eq!(
            PRESETS
                .iter()
                .filter(|p| p.kind == PresetKind::Local)
                .count(),
            4
        );
    }
    #[test]
    fn address_keys_survive_switch_and_legacy_migration_and_retirement() {
        let mut keys = AddressKeys::default();
        keys.remember("https://example.com", "legacy").unwrap();
        keys.remember("localhost:11434", "local").unwrap();
        assert_eq!(keys.get("https://example.com/v1"), Some(&"legacy"));
        assert!(keys.get("https://other.example").is_none());
        for i in 0..25 {
            keys.remember(&format!("https://server{i}.example"), "reference")
                .unwrap();
        }
        assert_eq!(keys.counts().0, 24);
        assert_eq!(keys.get("https://example.com"), Some(&"legacy"));
        assert!(keys.counts().1 > 0);
    }
    #[test]
    fn model_discovery_filters_explicitly_unsupported_tools_and_retains_unknowns() {
        let payload = json!({"data":[{"id":"chat-only","supported_parameters":[]},{"id":"tools","supported_parameters":["tools"],"architecture":{"input_modalities":["text","image"]}},{"id":"unknown"},{"id":"tools"},{"id":"bad\nname"}]});
        let result = models(&payload, &BTreeMap::new(), true).unwrap();
        assert_eq!(result.hidden, 1);
        assert_eq!(result.models.len(), 2);
        assert_eq!(result.models[0].vision, Some(true));
        assert_eq!(result.models[1].tools, None);
        assert!(result.has_key);
    }
    #[test]
    fn ollama_caps_override_metadata_and_context_matches_default_tag() {
        let payload = json!({"data":[{"id":"embed"},{"id":"agent"}]});
        let caps = BTreeMap::from([
            ("embed".into(), vec!["embedding".into()]),
            ("agent".into(), vec!["tools".into(), "vision".into()]),
        ]);
        let result = models(&payload, &caps, false).unwrap();
        assert_eq!(result.hidden, 1);
        assert_eq!(result.models[0].id, "agent");
        let running = json!({"models":[{"name":"agent:latest","context_length":4096}]});
        assert_eq!(ollama_context(&running, "agent").unwrap().tokens, 4096);
        assert!(ollama_context(&running, "other").is_none());
        assert!(
            ollama_context(
                &json!({"models":[{"name":"agent","context_length":16384}]}),
                "agent"
            )
            .is_none()
        );
    }
    #[test]
    fn provider_details_are_inert_bounded_and_tools_failure_is_classified() {
        let failure = http_failure(
            400,
            "{\"error\":{\"message\":\"model does not support tools\\nchoose another\"}}",
        );
        assert!(failure.tools_unsupported);
        assert!(!failure.detail.contains('\n'));
        assert_eq!(http_failure(500, "<html>gateway</html>").detail, "");
        assert!(
            http_failure(500, &"😀".repeat(300))
                .detail
                .encode_utf16()
                .count()
                <= 240
        );
    }
    struct Host {
        body: Value,
        status: u16,
    }
    impl DiscoveryHost for Host {
        fn list(&mut self, endpoint: &str, _: Option<&Secret>) -> Result<Response> {
            assert!(endpoint.ends_with("/v1"));
            Ok(Response {
                status: self.status,
                body: serde_json::to_vec(&self.body).unwrap(),
            })
        }
        fn ollama_capabilities(
            &mut self,
            _: &str,
            _: &[String],
        ) -> Result<BTreeMap<String, Vec<String>>> {
            Ok(BTreeMap::new())
        }
        fn sort_local(&self, models: &mut [ModelInfo]) {
            models.sort_by(|a, b| a.id.cmp(&b.id));
        }
    }
    #[test]
    fn worker_reply_is_bound_to_latest_endpoint_and_unsupported_listing_is_not_failure() {
        let mut d = Discovery::default();
        let old = d.start("localhost:11434", None, false).unwrap();
        let new = d.start("https://example.com", None, false).unwrap();
        let mut host = Host {
            body: json!({"data":[{"id":"m"}]}),
            status: 200,
        };
        assert!(!d.receive(old.run(&mut host)));
        assert!(d.receive(new.run(&mut host)));
        assert_eq!(d.result.as_ref().unwrap().models.len(), 1);
        let work = d.start("https://example.com", None, false).unwrap();
        host.status = 404;
        assert!(d.receive(work.run(&mut host)));
        assert!(d.result.unwrap().models.is_empty());
    }
    struct ConfigureDouble {
        calls: Vec<&'static str>,
        status: u16,
        fail_publish: bool,
    }
    impl ConfigureHost for ConfigureDouble {
        fn test(&mut self, _: &str, _: Option<&Secret>, body: Value) -> Result<Response> {
            self.calls.push("test");
            assert_eq!(body["tools"][0]["function"]["name"], "powermove_check");
            assert_eq!(body["stream"], false);
            Ok(Response {
                status: self.status,
                body: serde_json::to_vec(&json!({"choices":[{"message":{"content":"OK"}}]}))
                    .unwrap(),
            })
        }
        fn models(&mut self, _: &str, _: Option<&Secret>) -> Result<Value> {
            self.calls.push("models");
            Err(ServiceError::message(
                ServiceErrorCode::Offline,
                "optional discovery failed",
            ))
        }
        fn running_ollama(&mut self, _: &str) -> Result<Option<Value>> {
            self.calls.push("context");
            Ok(Some(
                json!({"models":[{"name":"m:latest","context_length":4096}]}),
            ))
        }
        fn publish(&mut self, _: &Configured, _: Option<&Secret>) -> Result<()> {
            self.calls.push("publish");
            if self.fail_publish {
                Err(ServiceError::message(
                    ServiceErrorCode::DiskFull,
                    "fixture key save failed",
                ))
            } else {
                Ok(())
            }
        }
    }
    fn configure() -> ConfigureWork {
        ConfigureWork {
            ticket: Flight::default().start(),
            input: super::super::agent_auth::ProviderInput::prepare(
                "localhost:11434",
                "m",
                false,
                "",
                None,
            )
            .unwrap(),
        }
    }
    #[test]
    fn configuration_tests_tool_acceptance_before_optional_discovery_and_durable_publication() {
        let mut host = ConfigureDouble {
            calls: vec![],
            status: 200,
            fail_publish: false,
        };
        let reply = configure().run(&mut host);
        let config = reply.result.unwrap();
        assert_eq!(host.calls, vec!["test", "models", "context", "publish"]);
        assert_eq!(config.models, vec!["m"]);
        assert_eq!(config.warning.unwrap().tokens, 4096);
    }
    #[test]
    fn failed_test_and_failed_secure_publication_do_not_create_a_connection() {
        let mut host = ConfigureDouble {
            calls: vec![],
            status: 401,
            fail_publish: false,
        };
        assert_eq!(
            configure().run(&mut host).result.unwrap_err().code,
            ServiceErrorCode::PermissionDenied
        );
        assert_eq!(host.calls, vec!["test"]);
        host.status = 200;
        host.fail_publish = true;
        assert_eq!(
            configure().run(&mut host).result.unwrap_err().code,
            ServiceErrorCode::DiskFull
        );
    }
    #[test]
    fn explicit_key_removal_never_resurrects_retired_keys() {
        let mut keys = AddressKeys::default();
        keys.remember("https://example.com", "old").unwrap();
        assert!(keys.archive("https://example.com"));
        assert!(keys.get("https://example.com").is_none());
        assert_eq!(keys.counts().1, 1);
        keys.remember("https://example.com", "new").unwrap();
        assert_eq!(keys.get("https://example.com"), Some(&"new"));
    }
    #[test]
    fn only_measured_local_servers_and_runtime_capabilities_are_offered() {
        assert_eq!(
            detected_local("ollama", &json!({"data":[{},{}]}))
                .unwrap()
                .models,
            2
        );
        assert!(detected_local("openai", &json!({"data":[]})).is_none());
        assert_eq!(
            claude_project_permission_mode(None, false),
            ClaudePermissionMode::Unavailable
        );
        assert_eq!(
            claude_project_permission_mode(None, true),
            ClaudePermissionMode::Auto
        );
        assert_eq!(
            claude_project_permission_mode(Some("supervised"), true),
            ClaudePermissionMode::AcceptEdits
        );
    }
}
