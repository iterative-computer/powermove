//! S070–S072/S083–S086. Sources: v1.2.0 main/cloud/auth.ts,
//! shared/compatible-provider.ts, renderer/cloud/SignInSheet.svelte.
use super::{Flight, Ticket};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

/// Secrets cannot enter model serialization or derived diagnostics.
pub struct Secret(String);
impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }
    /// Only the explicit worker transport/credential capability should call this.
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

pub const BROWSER_EXPIRY_MS: u64 = 600_000;
pub const RESEND_AFTER_MS: u64 = 30_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub email: String,
    pub handle: Option<String>,
    pub image: Option<String>,
    pub remember_installs: bool,
}
impl Account {
    /// Tagged publisher identity, rather than provider display name.
    pub fn identity(&self) -> String {
        self.handle
            .as_ref()
            .filter(|h| !h.is_empty())
            .map(|h| format!("@{h}"))
            .unwrap_or_else(|| {
                self.email
                    .split('@')
                    .next()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&self.email)
                    .into()
            })
    }
}

fn base64url(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let a = chunk[0] as usize;
        let b = chunk.get(1).copied().unwrap_or(0) as usize;
        let c = chunk.get(2).copied().unwrap_or(0) as usize;
        out.push(TABLE[a >> 2] as char);
        out.push(TABLE[((a & 3) << 4) | (b >> 4)] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((b & 15) << 2) | (c >> 6)] as char);
        }
        if chunk.len() > 2 {
            out.push(TABLE[c & 63] as char);
        }
    }
    out
}

pub fn registry_origin(raw: &str) -> Option<String> {
    let url = Url::parse(raw).ok()?;
    (url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none())
    .then(|| url.origin().ascii_serialization())
}

struct PendingBrowser {
    state: String,
    verifier: Secret,
    expires: u64,
    origin: String,
}
#[derive(Default)]
pub struct BrowserAuth {
    pending: Option<PendingBrowser>,
}
#[derive(Debug)]
pub struct Exchange {
    pub origin: String,
    pub state: Secret,
    pub token: Secret,
    pub verifier: Secret,
}
impl BrowserAuth {
    /// S083: no sheet means the return link is only an Error Log fact. Callers
    /// must also cancel the pending browser flow when the sheet closes.
    pub fn receive_for_sheet(
        &mut self,
        raw: &str,
        origin: &str,
        now: u64,
        sheet_open: bool,
    ) -> Option<Exchange> {
        if !sheet_open {
            return None;
        }
        self.receive(raw, origin, now)
    }
    /// Host must provide fresh CSPRNG output, never a timestamp/UUID/JS random.
    /// The verifier challenge hashes the raw random bytes, as the tagged server
    /// does, rather than hashing their base64 spelling (N3 protocol intent).
    pub fn begin(
        &mut self,
        origin: &str,
        random_verifier: [u8; 32],
        random_state: [u8; 16],
        now: u64,
    ) -> Option<Url> {
        let origin = registry_origin(origin)?;
        let verifier = Secret(base64url(&random_verifier));
        let state: String = random_state.iter().map(|b| format!("{b:02x}")).collect();
        let challenge = base64url(&Sha256::digest(random_verifier));
        let mut url = Url::parse(&format!("{origin}/v1/auth/desktop")).ok()?;
        url.query_pairs_mut().extend_pairs([
            ("provider", "google"),
            ("state", &state),
            ("challenge", &challenge),
        ]);
        self.pending = Some(PendingBrowser {
            state,
            verifier,
            expires: now.saturating_add(BROWSER_EXPIRY_MS),
            origin,
        });
        Some(url)
    }
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }
    pub fn cancel(&mut self) {
        self.pending = None;
    }
    /// Wrong/malformed links leave a real sign-in pending. A matching link is
    /// consumed before exchange, including failure or registry change.
    pub fn receive(&mut self, raw: &str, current_origin: &str, now: u64) -> Option<Exchange> {
        // Reuse the native LaunchServices admission boundary (no confused
        // authority, duplicate callback fields or control characters).
        let link = crate::entrypoints::AuthLink::parse(raw).ok()?;
        let state = link.state().to_owned();
        let token = link.token().to_owned();
        if state.is_empty()
            || token.is_empty()
            || state.encode_utf16().count() > 128
            || token.encode_utf16().count() > 4096
        {
            return None;
        }
        let pending = self.pending.as_ref()?;
        if pending.expires <= now {
            self.cancel();
            return None;
        }
        // The state has fixed length; never include either side in an error.
        if state.len() != pending.state.len()
            || state
                .bytes()
                .zip(pending.state.bytes())
                .fold(0, |diff, (a, b)| diff | (a ^ b))
                != 0
        {
            return None;
        }
        let pending = self.pending.take()?;
        if registry_origin(current_origin).as_deref() != Some(&pending.origin) {
            return None;
        }
        Some(Exchange {
            origin: pending.origin,
            state: Secret(state),
            token: Secret(token),
            verifier: pending.verifier,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignInStep {
    Start,
    Browser,
    Code,
    Handle,
    Complete,
    Closed,
}
#[derive(Debug)]
pub struct SignIn {
    pub step: SignInStep,
    pub error_code: Option<String>,
    resend_at: u64,
    flight: Flight,
}
impl SignIn {
    pub fn new(account: Option<&Account>) -> Self {
        Self {
            step: match account {
                Some(a) if a.handle.is_none() => SignInStep::Handle,
                Some(_) => SignInStep::Complete,
                None => SignInStep::Start,
            },
            error_code: None,
            resend_at: 0,
            flight: Flight::default(),
        }
    }
    pub fn begin(&mut self, step: SignInStep) -> Option<Ticket> {
        if matches!(self.step, SignInStep::Closed | SignInStep::Complete)
            || self.flight.current().is_some()
        {
            return None;
        }
        if step == SignInStep::Browser {
            self.step = step;
        }
        self.error_code = None;
        Some(self.flight.start())
    }
    pub fn sent(&mut self, ticket: Ticket, now: u64, result: Result<(), String>) -> bool {
        if !self.flight.finish(ticket) {
            return false;
        }
        match result {
            Ok(()) => {
                self.step = SignInStep::Code;
                self.resend_at = now.saturating_add(RESEND_AFTER_MS);
            }
            Err(code) => self.error_code = Some(code),
        }
        true
    }
    pub fn resend_ready(&self, now: u64) -> bool {
        self.step == SignInStep::Code && now >= self.resend_at && self.flight.current().is_none()
    }
    pub fn arrived(&mut self, ticket: Ticket, account: Result<Account, String>) -> bool {
        if !self.flight.finish(ticket) {
            return false;
        }
        match account {
            Ok(a) => {
                self.step = if a.handle.is_some() {
                    SignInStep::Complete
                } else {
                    SignInStep::Handle
                }
            }
            Err(code) => self.error_code = Some(code),
        }
        true
    }
    pub fn back(&mut self) {
        self.flight.cancel();
        self.step = SignInStep::Start;
        self.error_code = None;
    }
    pub fn close(&mut self) {
        self.flight.cancel();
        self.step = SignInStep::Closed;
    }
}
pub fn valid_handle(s: &str) -> bool {
    (2..=39).contains(&s.len())
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
pub fn otp(raw: &str) -> String {
    raw.chars().filter(char::is_ascii_digit).take(6).collect()
}

/// Exact shared providerUrl normalization; plaintext HTTP is local-only.
pub fn provider_url(raw: &str) -> Option<String> {
    let text = raw.trim();
    let lower = text.to_ascii_lowercase();
    let bare_local = ["localhost", "127.0.0.1", "[::1]"].iter().any(|host| {
        lower
            .strip_prefix(host)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(':') || rest.starts_with('/'))
    });
    // Owner's c7f46a main reference accepts copied bare server addresses.
    let qualified = if text.contains("://") {
        text.to_owned()
    } else if bare_local {
        format!("http://{text}")
    } else {
        format!("https://{text}")
    };
    let mut url = Url::parse(&qualified).ok()?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.host_str().is_none()
        || !(url.scheme() == "https" || local && url.scheme() == "http")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let path = url
        .path()
        .trim_end_matches('/')
        .strip_suffix("/chat/completions")
        .unwrap_or(url.path().trim_end_matches('/'))
        .to_owned();
    url.set_path(if path.is_empty() { "/v1" } else { &path });
    Some(url.as_str().trim_end_matches('/').into())
}

/// Worker-owned prepared configuration. A saved key is reusable only for the
/// exact normalized endpoint; blank input must not forward it to another host.
#[derive(Debug)]
pub struct ProviderInput {
    pub base_url: String,
    pub model: String,
    pub vision: bool,
    pub key: Option<Secret>,
}
#[derive(Debug)]
pub enum KeyEdit {
    KeepSaved,
    Replace(Secret),
    Remove,
}
impl ProviderInput {
    /// S071 distinguishes an untouched Saved placeholder from explicitly
    /// clearing the field. Endpoint changes cannot reuse another endpoint's key.
    pub fn with_key_edit(
        base_url: &str,
        model: &str,
        vision: bool,
        edit: KeyEdit,
        saved: Option<(&str, &str)>,
    ) -> Option<Self> {
        match edit {
            KeyEdit::KeepSaved => Self::prepare(base_url, model, vision, "", saved),
            KeyEdit::Replace(key) => Self::prepare(base_url, model, vision, key.expose(), None),
            KeyEdit::Remove => Self::prepare(base_url, model, vision, "", None),
        }
    }
    pub fn prepare(
        base_url: &str,
        model: &str,
        vision: bool,
        entered: &str,
        saved: Option<(&str, &str)>,
    ) -> Option<Self> {
        if base_url.encode_utf16().count() > 2000
            || model.encode_utf16().count() > 200
            || entered.encode_utf16().count() > 8192
            || model.trim().is_empty()
        {
            return None;
        }
        let base_url = provider_url(base_url)?;
        let key = if !entered.trim().is_empty() {
            Some(Secret(entered.trim().into()))
        } else {
            saved
                .filter(|(url, _)| provider_url(url).as_ref() == Some(&base_url))
                .map(|(_, key)| Secret(key.into()))
        };
        Some(Self {
            base_url,
            model: model.trim().into(),
            vision,
            key,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibleConfig {
    pub base_url: String,
    pub model: String,
    pub vision: bool,
    pub has_key: bool,
}
#[derive(Debug, Default)]
pub struct CompatibleConnection {
    pub active: Option<CompatibleConfig>,
    pub fields: Option<CompatibleConfig>,
    pub error: Option<crate::ServiceErrorCode>,
    pub edited: bool,
    flight: Flight,
}
impl CompatibleConnection {
    pub fn edit(&mut self, fields: CompatibleConfig) -> bool {
        if self.flight.current().is_some() {
            return false;
        }
        self.fields = Some(fields);
        self.edited = true;
        true
    }
    pub fn test(&mut self) -> Option<Ticket> {
        if self.flight.current().is_some() {
            return None;
        }
        self.error = None;
        Some(self.flight.start())
    }
    /// A failed test retains the previous working connection and entered fields.
    pub fn receive(&mut self, ticket: Ticket, result: crate::Result<CompatibleConfig>) -> bool {
        if !self.flight.finish(ticket) {
            return false;
        }
        match result {
            Ok(config) => {
                self.fields = Some(config.clone());
                self.active = Some(config);
                self.edited = false;
            }
            Err(error) => self.error = Some(error.code),
        }
        true
    }
    /// Disconnect is reconnectable: fields/key facts remain. Keychain removal
    /// happens only through an explicit KeyEdit::Remove and successful test.
    pub fn disconnected(&mut self) {
        self.flight.cancel();
        self.active = None;
    }
}

pub fn allowed_codex_auth_url(raw: &str) -> bool {
    let Ok(url) = Url::parse(raw) else {
        return false;
    };
    let host = url.host_str().unwrap_or("");
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && ["chatgpt.com", "openai.com"]
            .iter()
            .any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// A runtime session payload, accessible only to the worker credential adapter.
/// It cannot be serialized into a surface inspection or printed in diagnostics.
pub struct SessionPayload(serde_json::Value);
impl SessionPayload {
    pub fn new(value: serde_json::Value) -> Self {
        Self(value)
    }
    pub fn expose(&self) -> &serde_json::Value {
        &self.0
    }
}
impl std::fmt::Debug for SessionPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted session]")
    }
}

/// Concrete HTTP and Keychain implementations belong to the application host.
/// Every method runs on ServiceWorker. Registry changes must use the same worker
/// so exchange/save/refresh cannot interleave with a change of credential origin.
pub trait CloudHost: Send + 'static {
    fn origin(&self) -> &str;
    fn exchange(&mut self, exchange: Exchange) -> crate::Result<SessionPayload>;
    fn verify_email(&mut self, email: &str, code: &Secret) -> crate::Result<SessionPayload>;
    fn send_email(&mut self, email: &str) -> crate::Result<()>;
    fn save_session(&mut self, session: SessionPayload, provider: &str) -> crate::Result<()>;
    fn refresh_account(&mut self) -> crate::Result<Option<Account>>;
    fn claim_handle(&mut self, handle: &str) -> crate::Result<()>;
    fn remember_installs(&mut self, remember: bool) -> crate::Result<()>;
    fn sign_out(&mut self) -> crate::Result<()>;
}
pub enum CloudWork {
    Exchange(Exchange),
    SendEmail { email: String },
    VerifyEmail { email: String, code: Secret },
    ClaimHandle { handle: String },
    RememberInstalls(bool),
    SignOut,
}
#[derive(Debug)]
pub enum CloudReply {
    Sent,
    Account(Option<Account>),
    SignedOut,
}
pub struct CloudTask {
    pub ticket: Ticket,
    origin: String,
    work: CloudWork,
}
pub struct CloudCompletion {
    pub ticket: Ticket,
    pub result: crate::Result<CloudReply>,
}
impl CloudTask {
    pub fn new(ticket: Ticket, origin: &str, work: CloudWork) -> Option<Self> {
        Some(Self {
            ticket,
            origin: registry_origin(origin)?,
            work,
        })
    }
    pub fn run(self, host: &mut impl CloudHost) -> CloudCompletion {
        let result = if registry_origin(host.origin()).as_ref() != Some(&self.origin) {
            Err(crate::ServiceError::message(
                crate::ServiceErrorCode::PermissionDenied,
                "registry changed before account action",
            ))
        } else {
            self.work.run(host)
        };
        CloudCompletion {
            ticket: self.ticket,
            result,
        }
    }
}
impl CloudWork {
    /// Submit via ServiceWorker; replies carry account facts, never credentials.
    /// A failed save/refresh/claim/sign-out is returned unchanged as an error.
    fn run(self, host: &mut impl CloudHost) -> crate::Result<CloudReply> {
        match self {
            Self::SendEmail { email } => {
                host.send_email(&email)?;
                Ok(CloudReply::Sent)
            }
            Self::Exchange(exchange) => {
                if registry_origin(host.origin()).as_deref() != Some(&exchange.origin) {
                    return Err(crate::ServiceError::message(
                        crate::ServiceErrorCode::PermissionDenied,
                        "registry changed before exchange",
                    ));
                }
                let session = host.exchange(exchange)?;
                host.save_session(session, "google")?;
                Ok(CloudReply::Account(host.refresh_account()?))
            }
            Self::VerifyEmail { email, code } => {
                if code.expose().len() != 6 || !code.expose().bytes().all(|b| b.is_ascii_digit()) {
                    return Err(crate::ServiceError::message(
                        crate::ServiceErrorCode::InvalidInput,
                        "invalid verification code",
                    ));
                }
                let session = host.verify_email(&email, &code)?;
                host.save_session(session, "email")?;
                Ok(CloudReply::Account(host.refresh_account()?))
            }
            Self::ClaimHandle { handle } => {
                if !valid_handle(&handle) {
                    return Err(crate::ServiceError::message(
                        crate::ServiceErrorCode::InvalidInput,
                        "invalid publisher handle",
                    ));
                }
                host.claim_handle(&handle)?;
                Ok(CloudReply::Account(host.refresh_account()?))
            }
            Self::RememberInstalls(value) => {
                host.remember_installs(value)?;
                Ok(CloudReply::Account(host.refresh_account()?))
            }
            Self::SignOut => {
                host.sign_out()?;
                Ok(CloudReply::SignedOut)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn auth() -> (BrowserAuth, Url) {
        let mut auth = BrowserAuth::default();
        let url = auth
            .begin("https://example.com", [0; 32], [1; 16], 0)
            .unwrap();
        (auth, url)
    }
    fn link(url: &Url) -> String {
        let state = url.query_pairs().find(|(k, _)| k == "state").unwrap().1;
        format!("powermove://auth?state={state}&token=test")
    }
    #[test]
    fn challenge_hashes_raw_random_bytes() {
        let (_, url) = auth();
        let challenge = url.query_pairs().find(|(k, _)| k == "challenge").unwrap().1;
        assert_eq!(challenge, base64url(&Sha256::digest([0; 32])));
        assert_ne!(
            challenge,
            base64url(&Sha256::digest(base64url(&[0; 32]).as_bytes()))
        );
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
    }
    #[test]
    fn stray_link_cannot_cancel_and_matching_exchange_is_once() {
        let (mut a, url) = auth();
        for bad in [
            "https://auth?state=x&token=t",
            "powermove://auth?state=x&token=t",
            "powermove://auth?state=&token=t",
        ] {
            assert!(a.receive(bad, "https://example.com", 1).is_none());
            assert!(a.has_pending());
        }
        let exchange = a.receive(&link(&url), "https://example.com", 2).unwrap();
        assert_eq!(exchange.verifier.expose(), base64url(&[0; 32]));
        assert!(!format!("{exchange:?}").contains("test"));
        assert!(a.receive(&link(&url), "https://example.com", 3).is_none());
    }
    #[test]
    fn expiry_and_registry_switch_consume_without_exchange() {
        for (origin, now) in [
            ("https://other.example", 1),
            ("https://example.com", BROWSER_EXPIRY_MS),
        ] {
            let (mut a, u) = auth();
            assert!(a.receive(&link(&u), origin, now).is_none());
            assert!(!a.has_pending());
        }
    }
    #[test]
    fn second_sign_in_replaces_first_and_sign_out_cancels() {
        let (mut a, u) = auth();
        a.begin("https://example.com", [2; 32], [2; 16], 10)
            .unwrap();
        assert!(a.receive(&link(&u), "https://example.com", 11).is_none());
        assert!(a.has_pending());
        a.cancel();
        assert!(!a.has_pending());
    }
    #[test]
    fn sheet_cooldown_resume_and_stale_results() {
        let mut s = SignIn::new(None);
        let t = s.begin(SignInStep::Code).unwrap();
        assert!(s.begin(SignInStep::Code).is_none());
        assert!(s.sent(t, 10, Ok(())));
        assert!(!s.resend_ready(30_009));
        assert!(s.resend_ready(30_010));
        let t = s.begin(SignInStep::Handle).unwrap();
        s.back();
        assert!(!s.arrived(t, Err("offline".into())));
        let a = Account {
            email: "test@example.com".into(),
            handle: None,
            image: None,
            remember_installs: false,
        };
        assert_eq!(SignIn::new(Some(&a)).step, SignInStep::Handle);
        assert_eq!(a.identity(), "test");
        let t = s.begin(SignInStep::Code).unwrap();
        s.close();
        assert!(!s.sent(t, 0, Ok(())));
    }
    #[test]
    fn handle_and_code_follow_tag_grammar() {
        for s in ["ab", "a-", "12"] {
            assert!(valid_handle(s));
        }
        for s in ["a", "Ab", "-ab", "a_b", "éx"] {
            assert!(!valid_handle(s));
        }
        assert_eq!(otp("x1 23-4567９"), "123456");
    }
    #[test]
    fn provider_normalization_and_secret_isolation() {
        assert_eq!(
            provider_url("https://example.com/chat/completions/"),
            Some("https://example.com/v1".into())
        );
        assert_eq!(
            provider_url("http://[::1]:11434/prefix/v1/"),
            Some("http://[::1]:11434/prefix/v1".into())
        );
        for raw in [
            "http://example.com/v1",
            "https://user@example.com",
            "https://example.com/?a=b",
            "file:///tmp/x",
        ] {
            assert!(provider_url(raw).is_none());
        }
        let a = ProviderInput::prepare(
            "https://example.com/",
            " m ",
            true,
            "",
            Some(("https://example.com/v1", "saved")),
        )
        .unwrap();
        assert_eq!(a.key.unwrap().expose(), "saved");
        assert!(
            ProviderInput::prepare(
                "https://other.example",
                "m",
                false,
                "",
                Some(("https://example.com", "saved"))
            )
            .unwrap()
            .key
            .is_none()
        );
        assert!(ProviderInput::prepare("https://example.com", " ", false, "", None).is_none());
    }
    #[test]
    fn codex_browser_hosts_have_label_boundaries() {
        assert!(allowed_codex_auth_url("https://auth.openai.com/login"));
        for raw in [
            "https://openai.com.evil.example",
            "https://evilopenai.com",
            "http://chatgpt.com",
            "https://user@openai.com",
        ] {
            assert!(!allowed_codex_auth_url(raw));
        }
    }
    #[derive(Default)]
    struct Host {
        calls: Vec<&'static str>,
        fail_save: bool,
    }
    impl CloudHost for Host {
        fn origin(&self) -> &str {
            "https://example.com"
        }
        fn exchange(&mut self, _: Exchange) -> crate::Result<SessionPayload> {
            self.calls.push("exchange");
            Ok(SessionPayload::new(serde_json::json!({})))
        }
        fn verify_email(&mut self, _: &str, _: &Secret) -> crate::Result<SessionPayload> {
            self.calls.push("verify");
            Ok(SessionPayload::new(serde_json::json!({})))
        }
        fn send_email(&mut self, _: &str) -> crate::Result<()> {
            self.calls.push("send");
            Ok(())
        }
        fn save_session(&mut self, _: SessionPayload, _: &str) -> crate::Result<()> {
            self.calls.push("save");
            if self.fail_save {
                Err(crate::ServiceError::message(
                    crate::ServiceErrorCode::DiskFull,
                    "credential save failed",
                ))
            } else {
                Ok(())
            }
        }
        fn refresh_account(&mut self) -> crate::Result<Option<Account>> {
            self.calls.push("refresh");
            Ok(None)
        }
        fn claim_handle(&mut self, _: &str) -> crate::Result<()> {
            self.calls.push("claim");
            Ok(())
        }
        fn remember_installs(&mut self, _: bool) -> crate::Result<()> {
            self.calls.push("remember");
            Ok(())
        }
        fn sign_out(&mut self) -> crate::Result<()> {
            self.calls.push("sign_out");
            Ok(())
        }
    }
    fn task(work: CloudWork) -> CloudTask {
        CloudTask::new(Flight::default().start(), "https://example.com", work).unwrap()
    }
    #[test]
    fn auth_worker_saves_before_refresh_and_stops_on_failed_save() {
        let mut host = Host::default();
        let reply = task(CloudWork::VerifyEmail {
            email: "test@example.com".into(),
            code: Secret::new("123456".into()),
        })
        .run(&mut host);
        assert!(matches!(reply.result, Ok(CloudReply::Account(None))));
        assert_eq!(host.calls, vec!["verify", "save", "refresh"]);
        host.calls.clear();
        host.fail_save = true;
        let reply = task(CloudWork::VerifyEmail {
            email: "test@example.com".into(),
            code: Secret::new("123456".into()),
        })
        .run(&mut host);
        assert_eq!(
            reply.result.unwrap_err().code,
            crate::ServiceErrorCode::DiskFull
        );
        assert_eq!(host.calls, vec!["verify", "save"]);
    }
    #[test]
    fn account_worker_binds_all_actions_to_registry_and_validates_inputs() {
        let mut host = Host::default();
        let ticket = Flight::default().start();
        let reply = CloudTask::new(
            ticket,
            "https://other.example",
            CloudWork::SendEmail {
                email: "test@example.com".into(),
            },
        )
        .unwrap()
        .run(&mut host);
        assert_eq!(reply.ticket, ticket);
        assert_eq!(
            reply.result.unwrap_err().code,
            crate::ServiceErrorCode::PermissionDenied
        );
        assert!(host.calls.is_empty());
        assert!(
            task(CloudWork::ClaimHandle { handle: "A".into() })
                .run(&mut host)
                .result
                .is_err()
        );
        assert!(
            task(CloudWork::VerifyEmail {
                email: "test@example.com".into(),
                code: Secret::new("invalid".into())
            })
            .run(&mut host)
            .result
            .is_err()
        );
        assert!(host.calls.is_empty());
        assert!(
            task(CloudWork::ClaimHandle {
                handle: "valid".into()
            })
            .run(&mut host)
            .result
            .is_ok()
        );
        assert_eq!(host.calls, vec!["claim", "refresh"]);
    }
    #[test]
    fn compatible_failed_test_keeps_previous_connection_and_explicit_clear_removes_key() {
        let old = CompatibleConfig {
            base_url: "https://example.com/v1".into(),
            model: "old".into(),
            vision: false,
            has_key: true,
        };
        let mut c = CompatibleConnection {
            active: Some(old.clone()),
            fields: Some(old.clone()),
            ..CompatibleConnection::default()
        };
        let mut fields = old.clone();
        fields.model = "new".into();
        assert!(c.edit(fields.clone()));
        assert_eq!(c.active, Some(old.clone()));
        let t = c.test().unwrap();
        assert!(!c.edit(old.clone()));
        c.receive(
            t,
            Err(crate::ServiceError::message(
                crate::ServiceErrorCode::Offline,
                "fixture offline",
            )),
        );
        assert_eq!(c.active, Some(old));
        assert_eq!(c.fields, Some(fields.clone()));
        let t = c.test().unwrap();
        c.receive(t, Ok(fields.clone()));
        assert_eq!(c.active, Some(fields.clone()));
        c.disconnected();
        assert!(c.active.is_none());
        assert_eq!(c.fields, Some(fields));
        assert!(
            ProviderInput::with_key_edit(
                "https://example.com",
                "m",
                false,
                KeyEdit::Remove,
                Some(("https://example.com", "saved"))
            )
            .unwrap()
            .key
            .is_none()
        );
    }
    #[test]
    fn closed_sheet_never_exchanges_browser_return() {
        let (mut auth, url) = auth();
        assert!(
            auth.receive_for_sheet(&link(&url), "https://example.com", 1, false)
                .is_none()
        );
        assert!(
            auth.receive_for_sheet(&link(&url), "https://example.com", 2, true)
                .is_some()
        );
    }
}
