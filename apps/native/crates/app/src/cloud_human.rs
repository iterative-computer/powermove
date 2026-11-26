//! Human verification (v1.2.0 `cloud/client.ts` + `human-verification.ts`):
//! the registry answers a protected route (sending a sign-in code, a first
//! publish) with 403 `human_verification_required`, a signed ticket and a
//! scope. The app opens `{origin}/v1/human?ticket=…` in the browser (the
//! page carries only the ticket, never the session or the email), polls
//! `POST /v1/human/result` until the check passes, then retries the request
//! once with `X-Powermove-Human: <clearance>`. The clearance is kept for its
//! validity under the profile (`cloud/human/`), never in the Keychain, and
//! the header only ever goes to the registry's own origin.
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

/// Shared between the account worker (which waits for the browser) and the
/// sign-in sheet (which shows "Verify in your browser…" and can cancel it).
#[derive(Debug, Default)]
pub struct HumanGate {
    verifying: AtomicBool,
    cancel: AtomicBool,
}
impl HumanGate {
    /// The browser check is open and the worker is waiting for it.
    pub fn verifying(&self) -> bool {
        self.verifying.load(Ordering::Relaxed)
    }
    /// Stops the wait: the request fails as unverified.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub(crate) fn start(&self) {
        self.cancel.store(false, Ordering::Relaxed);
        self.verifying.store(true, Ordering::Relaxed);
    }
    pub(crate) fn end(&self) {
        self.verifying.store(false, Ordering::Relaxed);
    }
    pub(crate) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// A 403 that asks for a human: its signed ticket and the scope a clearance
/// for it covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Challenge {
    pub ticket: String,
    pub scope: String,
}
pub(crate) fn challenge(status: u16, body: &[u8]) -> Option<Challenge> {
    if status != 403 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    if value["error"] != "human_verification_required" {
        return None;
    }
    let field = |key: &str| {
        value[key]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 4096)
            .map(str::to_owned)
    };
    Some(Challenge {
        ticket: field("ticket")?,
        scope: field("scope")?,
    })
}

/// When a clearance stops working (ms since 1970): its signed payload's
/// `expires` (`<base64url JSON>.<signature>`).
pub(crate) fn expires_ms(clearance: &str) -> Option<u64> {
    use base64::Engine as _;
    let payload = clearance.split('.').next()?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value["expires"].as_f64().map(|ms| ms as u64)
}

/// The clearances this profile remembers: one file per origin and scope.
pub(crate) struct Clearances {
    dir: PathBuf,
}
impl Clearances {
    pub(crate) fn new(profile: &Path) -> Self {
        Self {
            dir: profile.join("cloud/human"),
        }
    }
    fn file(&self, origin: &str, scope: &str) -> PathBuf {
        let key = blake3::hash(format!("{origin}\n{scope}").as_bytes());
        self.dir.join(format!("{}.json", key.to_hex()))
    }
    /// A clearance for `origin` and `scope` with more than a minute left.
    pub(crate) fn get(&self, origin: &str, scope: &str, now_ms: u64) -> Option<String> {
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(self.file(origin, scope)).ok()?).ok()?;
        (value["origin"] == origin
            && value["scope"] == scope
            && value["expires"]
                .as_u64()
                .is_some_and(|e| e > now_ms + 60_000))
        .then(|| value["clearance"].as_str().map(str::to_owned))
        .flatten()
    }
    /// Keeps `clearance` until its own expiry (nine minutes when it names
    /// none, as v1 kept one in memory).
    pub(crate) fn put(&self, origin: &str, scope: &str, clearance: &str, now_ms: u64) {
        let expires = expires_ms(clearance).unwrap_or(now_ms + 9 * 60_000);
        let file = self.file(origin, scope);
        let body = serde_json::json!({"origin": origin, "scope": scope,
            "clearance": clearance, "expires": expires});
        if std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        let temp = file.with_extension(format!("{}.tmp", std::process::id()));
        let written = {
            use std::io::Write as _;
            use std::os::unix::fs::OpenOptionsExt as _;
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&temp)
                .and_then(|mut f| f.write_all(body.to_string().as_bytes()))
        };
        if written.is_err() || std::fs::rename(&temp, &file).is_err() {
            let _ = std::fs::remove_file(&temp);
        }
    }
    pub(crate) fn forget(&self, origin: &str, scope: &str) {
        let _ = std::fs::remove_file(self.file(origin, scope));
    }
}
