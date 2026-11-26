//! The OpenAI credential Codex and Powermove share. Codex 0.169 keeps its
//! login in the system keyring when its home's `config.toml` says
//! `cli_auth_credentials_store = "keyring"`: one generic password, service
//! `Codex Auth`, account `cli|<first 16 hex digits of SHA-256 of the
//! canonical CODEX_HOME>`, holding Codex's own `auth.json` document (an API
//! key, or ChatGPT tokens with the account id).
//!
//! Codex is the only writer and the only refresher: it signs in, signs out,
//! saves the API key and refreshes the rotating refresh token (behind its own
//! lock and a compare-and-save). Powermove only reads the item, for Settings
//! › Providers (who is signed in, the key masked). Aero never uses it.
use crate::edge::Secrets;
use crate::launch::Environment;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const SERVICE: &str = "Codex Auth";

/// The keyring service. Development builds may name a lane-only one
/// (`PM_CODEX_KEYRING_SERVICE`, used with a fake Codex), so a lane never
/// touches a real `Codex Auth` item.
pub fn service(env: &Environment) -> String {
    env.vars
        .get("PM_CODEX_KEYRING_SERVICE")
        .filter(|s| cfg!(debug_assertions) && !s.is_empty())
        .cloned()
        .unwrap_or_else(|| SERVICE.into())
}
/// Codex's `compute_store_key`.
pub fn store_key(codex_home: &Path) -> String {
    let path = codex_home
        .canonicalize()
        .unwrap_or_else(|_| codex_home.to_path_buf());
    let hash = format!("{:x}", Sha256::digest(path.to_string_lossy().as_bytes()));
    format!("cli|{}", &hash[..16])
}

/// What Powermove reads from the shared item. Never logged.
#[derive(Clone, PartialEq, Eq)]
pub struct CodexAuth {
    /// ChatGPT sign-in: the bearer for chatgpt.com and its account id.
    pub chatgpt: Option<(String, Option<String>)>,
    /// The OpenAI API key, when that is how Codex signed in.
    pub api_key: Option<String>,
}
impl std::fmt::Debug for CodexAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CodexAuth {{ chatgpt: {}, api_key: {} }}",
            self.chatgpt.is_some(),
            self.api_key.is_some()
        )
    }
}
impl CodexAuth {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let v: Value = serde_json::from_slice(bytes).ok()?;
        let tokens = &v["tokens"];
        let access = tokens["access_token"].as_str().filter(|s| !s.is_empty());
        let chatgpt = access.map(|a| {
            (
                a.to_owned(),
                tokens["account_id"].as_str().map(str::to_owned),
            )
        });
        Some(Self {
            chatgpt,
            // A ChatGPT login may also carry an exchanged key; the key is the
            // method only when there are no ChatGPT tokens.
            api_key: v["OPENAI_API_KEY"]
                .as_str()
                .filter(|k| !k.is_empty() && access.is_none())
                .map(str::to_owned),
        })
    }
}

pub fn codex_home(env: &Environment) -> PathBuf {
    env.profile.join("codex-runtime")
}

/// The shared item as it is now.
pub fn read(secrets: &dyn Secrets, env: &Environment) -> Result<Option<CodexAuth>, String> {
    let bytes = secrets.read_codex(&service(env), &store_key(&codex_home(env)))?;
    Ok(bytes.as_deref().and_then(CodexAuth::parse))
}

/// The OpenAI identity generation: a sign-in or sign-out through Settings ›
/// Providers moves it on. A live Codex run's watcher stops it when the
/// generation moves, so a turn never carries on under another sign-in.
static GENERATION: AtomicU64 = AtomicU64::new(1);
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}
/// A new sign-in or a sign-out: every turn on the old one ends, and the
/// refresh broker forgets what it kept for it.
pub fn identity_changed() {
    let next = GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    crate::refresh_broker::forget(next);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_store_key_is_codexs() {
        // sha256("/nonexistent/codex-home") — a path canonicalize leaves alone.
        let key = store_key(Path::new("/nonexistent/codex-home"));
        let expected = format!("{:x}", Sha256::digest(b"/nonexistent/codex-home"));
        assert_eq!(key, format!("cli|{}", &expected[..16]));
        assert_eq!(key.len(), 20);
    }

    #[test]
    fn the_item_reads_as_chatgpt_or_as_a_key() {
        use base64::Engine as _;
        let claims =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"exp":4102444800}"#);
        let access = format!("h.{claims}.s");
        let chatgpt = json!({"auth_mode":"chatgpt","OPENAI_API_KEY":"sk-exchanged","tokens":{"id_token":"x","access_token":access,"refresh_token":"r","account_id":"acct"}});
        let auth = CodexAuth::parse(chatgpt.to_string().as_bytes()).unwrap();
        assert_eq!(auth.chatgpt, Some((access.clone(), Some("acct".into()))));
        assert!(auth.api_key.is_none());
        let key = json!({"auth_mode":"apikey","OPENAI_API_KEY":"sk-test-1234"});
        let auth = CodexAuth::parse(key.to_string().as_bytes()).unwrap();
        assert!(auth.chatgpt.is_none());
        assert_eq!(auth.api_key.as_deref(), Some("sk-test-1234"));
    }
}
