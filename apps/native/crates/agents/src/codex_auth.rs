//! The OpenAI credential Codex and Powermove share. Codex 0.169 keeps its
//! login in the system keyring when its home's `config.toml` says
//! `cli_auth_credentials_store = "keyring"`: one generic password, service
//! `Codex Auth`, account `cli|<first 16 hex digits of SHA-256 of the
//! canonical CODEX_HOME>`, holding Codex's own `auth.json` document (an API
//! key, or ChatGPT tokens with the account id).
//!
//! Codex is the only writer and the only refresher: it signs in, signs out,
//! saves the API key and refreshes the rotating refresh token (behind its own
//! lock and a compare-and-save). Powermove only reads the item. When the
//! access token is within five minutes of expiring, or the backend refused
//! it, Powermove asks Codex to refresh (App Server `account/read` with
//! `refreshToken`) under one process-wide lock and reads the item again, so
//! Powermove never spends a refresh token and two refreshes never race.
use crate::edge::Secrets;
use crate::launch::Environment;
use crate::runtime::Runtime;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

pub const SERVICE: &str = "Codex Auth";
/// Codex refreshes an access token this close to its expiry; so does Powermove's ask.
const WINDOW_SECS: u64 = 300;

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
    let path = codex_home.canonicalize().unwrap_or_else(|_| codex_home.to_path_buf());
    let hash = format!("{:x}", Sha256::digest(path.to_string_lossy().as_bytes()));
    format!("cli|{}", &hash[..16])
}

/// What Powermove reads from the shared item. Never logged.
#[derive(Clone, PartialEq, Eq)]
pub struct CodexAuth {
    /// ChatGPT sign-in: the bearer for chatgpt.com and its account id.
    pub chatgpt: Option<(String, Option<String>)>,
    /// Unix seconds, from the access token.
    pub expires_at: Option<u64>,
    /// The OpenAI API key, when that is how Codex signed in.
    pub api_key: Option<String>,
}
impl std::fmt::Debug for CodexAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CodexAuth {{ chatgpt: {}, api_key: {} }}", self.chatgpt.is_some(), self.api_key.is_some())
    }
}
impl CodexAuth {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let v: Value = serde_json::from_slice(bytes).ok()?;
        let tokens = &v["tokens"];
        let access = tokens["access_token"].as_str().filter(|s| !s.is_empty());
        let chatgpt = access.map(|a| (a.to_owned(), tokens["account_id"].as_str().map(str::to_owned)));
        Some(Self {
            expires_at: access.and_then(jwt_exp),
            chatgpt,
            // A ChatGPT login may also carry an exchanged key; the key is the
            // method only when there are no ChatGPT tokens.
            api_key: v["OPENAI_API_KEY"].as_str().filter(|k| !k.is_empty() && access.is_none()).map(str::to_owned),
        })
    }
    fn fresh(&self) -> bool {
        self.chatgpt.is_none() || self.expires_at.is_some_and(|exp| exp > now() + WINDOW_SECS)
    }
}
fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}
fn jwt_exp(token: &str) -> Option<u64> {
    use base64::Engine as _;
    let claims = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(claims.trim_end_matches('=')).ok()?;
    serde_json::from_slice::<Value>(&bytes).ok()?["exp"].as_u64()
}

pub fn codex_home(env: &Environment) -> PathBuf {
    env.profile.join("codex-runtime")
}

/// The shared item as it is now.
pub fn read(secrets: &dyn Secrets, env: &Environment) -> Result<Option<CodexAuth>, String> {
    let bytes = secrets.read(&service(env), &store_key(&codex_home(env)))?;
    Ok(bytes.as_deref().and_then(CodexAuth::parse))
}

/// Moves a login an earlier build kept in `auth.json` (file store) into the
/// keyring item, once, and removes the file. Never overwrites an item.
pub fn migrate(secrets: &dyn Secrets, env: &Environment, home: &Path) -> Result<(), String> {
    let file = home.join("auth.json");
    let Ok(bytes) = std::fs::read(&file) else { return Ok(()) };
    let key = store_key(home);
    if secrets.read(&service(env), &key)?.is_none() {
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| "Codex's saved sign-in is unreadable.")?;
        secrets.write(&service(env), &key, value.to_string().as_bytes())?;
    }
    std::fs::remove_file(&file).map_err(|e| e.to_string())
}

static REFRESH: Mutex<()> = Mutex::new(());

/// A usable credential: the item as it is, or after Codex refreshed it.
/// `rejected` is the bearer a backend just refused (a 401): refresh unless
/// the item already holds another one.
pub fn current(secrets: &dyn Secrets, env: &Environment, rejected: Option<&str>) -> Result<CodexAuth, String> {
    let signed_out = || "Sign in to OpenAI in Settings › Providers.".to_owned();
    let _one = REFRESH.lock().unwrap_or_else(|e| e.into_inner());
    let auth = read(secrets, env)?.ok_or_else(signed_out)?;
    let stale = match (&auth.chatgpt, rejected) {
        (Some((access, _)), Some(bad)) => access == bad,
        _ => !auth.fresh(),
    };
    if !stale {
        return Ok(auth);
    }
    ask_codex_to_refresh(env)?;
    let auth = read(secrets, env)?.ok_or_else(signed_out)?;
    match (&auth.chatgpt, rejected) {
        (Some((access, _)), Some(bad)) if access == bad => Err(expired()),
        _ if !auth.fresh() => Err(expired()),
        _ => Ok(auth),
    }
}
fn expired() -> String {
    "Your ChatGPT sign-in expired. Sign in to OpenAI again in Settings › Providers.".into()
}

fn ask_codex_to_refresh(env: &Environment) -> Result<(), String> {
    let binary = env.binary(Runtime::Codex).ok_or("Powermove's built-in Codex runtime is missing.")?;
    let mut peer = crate::account::Peer::start(&binary, &["app-server"], env.codex_vars()?)?;
    let wait = Duration::from_secs(45);
    peer.request(
        "initialize",
        json!({"clientInfo":{"name":"powermove","title":"Powermove","version":env.app_version},
            "capabilities":{"experimentalApi":true}}),
        wait,
    )?;
    peer.notify("initialized", json!({}))?;
    peer.request("account/read", json!({"refreshToken": true}), wait)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"exp":4102444800}"#);
        let access = format!("h.{claims}.s");
        let chatgpt = json!({"auth_mode":"chatgpt","OPENAI_API_KEY":"sk-exchanged","tokens":{"id_token":"x","access_token":access,"refresh_token":"r","account_id":"acct"}});
        let auth = CodexAuth::parse(chatgpt.to_string().as_bytes()).unwrap();
        assert_eq!(auth.chatgpt, Some((access.clone(), Some("acct".into()))));
        assert_eq!(auth.expires_at, Some(4102444800));
        assert!(auth.api_key.is_none() && auth.fresh());
        let key = json!({"auth_mode":"apikey","OPENAI_API_KEY":"sk-test-1234"});
        let auth = CodexAuth::parse(key.to_string().as_bytes()).unwrap();
        assert!(auth.chatgpt.is_none());
        assert_eq!(auth.api_key.as_deref(), Some("sk-test-1234"));
    }
}
