//! The Powermove Cloud account (S083–S086): sign in with Google through the
//! browser (PKCE, the `powermove://auth` return link) or with an emailed
//! code, choose a publisher handle, Remember Installs, sign out and delete the
//! account. Protocol state lives in `pm-services` (`agent_auth`); this is the
//! host: the registry over the system HTTP stack (bearer only to the registry
//! origin, never across a foreign redirect), the session in the macOS
//! Keychain, AppKit sheets for the steps, and Settings › Account rows through
//! the built-in facts. Every request runs on one service worker, so an
//! exchange, its save and the refresh never interleave.
use crate::cloud_human::{self, HumanGate};
use gpui::{App, AsyncApp, Global};
use objc2::rc::Retained;
use objc2_app_kit::NSWindow;
use pm_agents::inference::{Billing, BillingAnswer};
use pm_compat_runner::store_install;
use pm_services::{
    ServiceError, ServiceErrorCode,
    surfaces::{
        agent_auth::{
            Account, BrowserAuth, CloudHost, CloudReply, CloudTask, CloudWork, Exchange, Secret,
            SessionPayload, valid_handle,
        },
        copy::{self, AuthError},
    },
    worker::ServiceWorker,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

// ---- The session in the Keychain ----

/// Where the session token lives. Implementations never log what they hold.
pub trait Vault: Send {
    fn read(&self) -> Result<Option<Vec<u8>>, String>;
    fn write(&mut self, bytes: &[u8]) -> Result<(), String>;
    fn delete(&mut self) -> Result<(), String>;
}
/// Generic-password items by service and account (the macOS Keychain; a
/// memory map in tests).
pub trait Items: Send {
    fn read(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, String>;
    fn write(&mut self, service: &str, account: &str, bytes: &[u8]) -> Result<(), String>;
    fn delete(&mut self, service: &str, account: &str) -> Result<(), String>;
}
/// A generic password named after this app's bundle id, so a beta, a
/// release and each development bundle keep their own session. A release
/// build's session also reads the items the first launch migrated from the
/// shipped app (every location it registered in the profile, one per
/// migrated version); delete removes all of them, so a signed-out Mac stays
/// signed out.
pub struct Keychain {
    service: String,
    account: String,
    /// The profile whose registered migrated sessions are read and deleted.
    migrated: Option<PathBuf>,
    items: Box<dyn Items>,
}
impl Keychain {
    #[cfg(target_os = "macos")]
    pub fn for_app(profile: &Path) -> Self {
        let mut keychain = Self::named("Powermove Cloud", "session");
        // Settings › Extensions › Sign-Ins from the Original App. Release builds
        // only: a development build never reads or touches a real account.
        if !cfg!(debug_assertions) {
            keychain.migrated = Some(profile.to_owned());
        }
        keychain
    }
    /// `<bundle id> <name>` / `account`.
    #[cfg(target_os = "macos")]
    pub fn named(name: &str, account: &str) -> Self {
        let id = crate::platform::bundle_id().unwrap_or_else(|| "com.powermove.native.dev".into());
        Self::with_items(
            format!("{id} {name}"),
            account.into(),
            None,
            Box::new(keychain::Mac),
        )
    }
    pub fn with_items(
        service: String,
        account: String,
        migrated: Option<PathBuf>,
        items: Box<dyn Items>,
    ) -> Self {
        Self {
            service,
            account,
            migrated,
            items,
        }
    }
}
impl Keychain {
    /// Where the first launch put the shipped app's Cloud session, newest
    /// registration first.
    fn migrated(&self) -> Result<Vec<(String, String)>, String> {
        let Some(profile) = &self.migrated else {
            return Ok(Vec::new());
        };
        let locators = pm_kernel::profile::registered_secrets(profile, "cloud/session")
            .map_err(|e| e.to_string())?;
        Ok(locators
            .into_iter()
            .rev()
            .map(|l| (l.service, l.account))
            .collect())
    }
}
impl Vault for Keychain {
    fn read(&self) -> Result<Option<Vec<u8>>, String> {
        if let Some(bytes) = self.items.read(&self.service, &self.account)? {
            return Ok(Some(bytes));
        }
        for (service, account) in self.migrated()? {
            if let Some(bytes) = self.items.read(&service, &account)? {
                return Ok(Some(bytes));
            }
        }
        Ok(None)
    }
    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.items.write(&self.service, &self.account, bytes)
    }
    /// Every item `read` could return; each is tried, the first failure reported.
    fn delete(&mut self) -> Result<(), String> {
        let mut outcome = self.items.delete(&self.service, &self.account);
        match self.migrated() {
            Ok(locations) => {
                for (service, account) in locations {
                    let deleted = self.items.delete(&service, &account);
                    outcome = outcome.and(deleted);
                }
            }
            Err(error) => outcome = outcome.and(Err(error)),
        }
        outcome
    }
}

#[cfg(target_os = "macos")]
pub(crate) mod keychain {
    use std::{ffi::c_void, ptr};
    type Item = *mut c_void;
    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SecKeychainFindGenericPassword(
            keychain: Item,
            service_len: u32,
            service: *const u8,
            account_len: u32,
            account: *const u8,
            len: *mut u32,
            data: *mut *mut c_void,
            item: *mut Item,
        ) -> i32;
        fn SecKeychainItemFreeContent(attrs: *mut c_void, data: *mut c_void) -> i32;
        fn SecKeychainAddGenericPassword(
            keychain: Item,
            service_len: u32,
            service: *const u8,
            account_len: u32,
            account: *const u8,
            len: u32,
            data: *const c_void,
            item: *mut Item,
        ) -> i32;
        fn SecKeychainItemDelete(item: Item) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(item: *const c_void);
    }
    const NOT_FOUND: i32 = -25300;
    /// The password, and the item when `keep` asks for it (released by the caller).
    fn find(service: &str, account: &str, keep: bool) -> Result<Option<(Vec<u8>, Item)>, String> {
        let (mut len, mut data, mut item) = (0u32, ptr::null_mut(), ptr::null_mut());
        // SAFETY: explicit lengths; the data is owned by Security until freed.
        let status = unsafe {
            SecKeychainFindGenericPassword(
                ptr::null_mut(),
                service.len() as u32,
                service.as_ptr(),
                account.len() as u32,
                account.as_ptr(),
                &mut len,
                &mut data,
                if keep { &mut item } else { ptr::null_mut() },
            )
        };
        if status == NOT_FOUND {
            return Ok(None);
        }
        if status != 0 {
            return Err(format!("Keychain read failed ({status})"));
        }
        let bytes = if len == 0 || data.is_null() {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len as usize) }.to_vec()
        };
        unsafe { SecKeychainItemFreeContent(ptr::null_mut(), data) };
        Ok(Some((bytes, item)))
    }
    /// Every Keychain call goes through here first. A development build
    /// launched against the local fakes answers from their fake store, and a
    /// missing or invalid fake-store setting closes the Keychain instead of
    /// falling through to the real one (`pm_agents::edge::keychain_mode`).
    fn fake() -> Option<Box<dyn pm_agents::edge::Secrets>> {
        use pm_agents::edge::{KeychainMode, RefusedKeychain, keychain_mode};
        let vars = std::env::vars().filter(|(k, _)| k.starts_with("PM_")).collect();
        match keychain_mode(&vars) {
            KeychainMode::Real => None,
            KeychainMode::Fake(store) => Some(Box::new(store)),
            KeychainMode::Refused(why) => Some(Box::new(RefusedKeychain(why))),
        }
    }
    pub fn read(service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
        use pm_agents::edge::Secrets as _;
        if let Some(fake) = fake() {
            return fake.read(service, account);
        }
        Ok(find(service, account, false)?.map(|(bytes, _)| bytes))
    }
    pub fn delete(service: &str, account: &str) -> Result<(), String> {
        use pm_agents::edge::Secrets as _;
        if let Some(fake) = fake() {
            return fake.delete(service, account);
        }
        if let Some((_, item)) = find(service, account, true)? {
            let status = unsafe { SecKeychainItemDelete(item) };
            unsafe { CFRelease(item) };
            if status != 0 {
                return Err(format!("Keychain delete failed ({status})"));
            }
        }
        Ok(())
    }
    pub fn write(service: &str, account: &str, bytes: &[u8]) -> Result<(), String> {
        use pm_agents::edge::Secrets as _;
        if let Some(fake) = fake() {
            return fake.write(service, account, bytes);
        }
        delete(service, account)?;
        let status = unsafe {
            SecKeychainAddGenericPassword(
                ptr::null_mut(),
                service.len() as u32,
                service.as_ptr(),
                account.len() as u32,
                account.as_ptr(),
                bytes.len() as u32,
                bytes.as_ptr().cast(),
                ptr::null_mut(),
            )
        };
        if status != 0 {
            return Err(format!("Keychain write failed ({status})"));
        }
        Ok(())
    }
    pub struct Mac;
    impl super::Items for Mac {
        fn read(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
            read(service, account)
        }
        fn write(&mut self, service: &str, account: &str, bytes: &[u8]) -> Result<(), String> {
            write(service, account, bytes)
        }
        fn delete(&mut self, service: &str, account: &str) -> Result<(), String> {
            delete(service, account)
        }
    }
}
/// `{origin, token, expiresAt}`, as the shipped app sealed it.
fn stored_token(vault: &dyn Vault, origin: &str) -> Option<Secret> {
    let bytes = vault.read().ok()??;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    let same = value["origin"].as_str().map(|o| o.trim_end_matches('/'))
        == Some(origin.trim_end_matches('/'));
    let live = value["expiresAt"]
        .as_str()
        .and_then(unix_seconds)
        .is_some_and(|expires| expires > now_ms() / 1000);
    (same && live)
        .then(|| value["token"].as_str().map(|t| Secret::new(t.to_owned())))
        .flatten()
}
/// Seconds since 1970 of `YYYY-MM-DDTHH:MM:SS…` (UTC).
fn unix_seconds(iso: &str) -> Option<u64> {
    let (date, time) = iso.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<u32>().ok());
    let day = pm_services::surfaces::store::civil_days((d.next()?? as i32, d.next()??, d.next()??));
    let mut t = time.get(..8)?.split(':').map(|p| p.parse::<u64>().ok());
    let seconds = t.next()?? * 3600 + t.next()?? * 60 + t.next()??;
    u64::try_from(day).ok().map(|d| d * 86_400 + seconds)
}
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

// ---- The registry ----

/// How long a registry request may take.
const REQUEST_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);
/// Telling the registry about a sign-out the Mac has already done.
const REVOKE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// The registry for the account: the Store's origin.
pub struct Http {
    origin: &'static str,
    vault: Box<dyn Vault>,
    profile: PathBuf,
    /// The browser check a protected route may ask for (`cloud_human`).
    human: std::sync::Arc<HumanGate>,
    /// Opens the check's page (the system browser; a lane build logs it).
    opener: std::sync::Arc<dyn Fn(&str) -> bool + Send + Sync>,
    /// How often the check's result is asked for, and for how long.
    poll: std::time::Duration,
    patience: std::time::Duration,
}
impl Http {
    pub fn new(origin: &'static str, vault: Box<dyn Vault>, profile: PathBuf) -> Self {
        Self {
            origin,
            vault,
            profile,
            human: Default::default(),
            opener: std::sync::Arc::new(crate::platform::open_url),
            poll: std::time::Duration::from_millis(1500),
            patience: std::time::Duration::from_secs(180),
        }
    }
    /// The gate the sign-in sheet watches and cancels.
    pub(crate) fn with_gate(mut self, gate: std::sync::Arc<HumanGate>) -> Self {
        self.human = gate;
        self
    }
    /// Tests: the browser stand-in and a quick poll.
    #[cfg(test)]
    pub(crate) fn with_browser(
        mut self,
        opener: impl Fn(&str) -> bool + Send + Sync + 'static,
        poll: std::time::Duration,
        patience: std::time::Duration,
    ) -> Self {
        self.opener = std::sync::Arc::new(opener);
        self.poll = poll;
        self.patience = patience;
        self
    }
    /// One request; a failure names only the registry's error code
    /// (`api:handle_taken`) or the transport, never a body or a token.
    pub(crate) fn call(
        &mut self,
        method: &str,
        path: &str,
        body: Option<Value>,
        auth: bool,
    ) -> pm_services::Result<Value> {
        let kind = body.as_ref().map(|_| "application/json".to_owned());
        let bytes = body.map(|b| b.to_string().into_bytes()).unwrap_or_default();
        self.send(method, path, bytes, kind, auth)
    }
    fn send(
        &mut self,
        method: &str,
        path: &str,
        body: Vec<u8>,
        kind: Option<String>,
        auth: bool,
    ) -> pm_services::Result<Value> {
        let bearer = if auth {
            Some(
                stored_token(self.vault.as_ref(), self.origin).ok_or_else(|| {
                    ServiceError::message(ServiceErrorCode::PermissionDenied, "api:unauthorized")
                })?,
            )
        } else {
            None
        };
        self.send_as(method, path, body, kind, bearer, REQUEST_DEADLINE)
    }
    /// One request with `bearer` (or none), given up after `deadline`.
    fn send_as(
        &mut self,
        method: &str,
        path: &str,
        body: Vec<u8>,
        kind: Option<String>,
        bearer: Option<Secret>,
        deadline: std::time::Duration,
    ) -> pm_services::Result<Value> {
        #[cfg(debug_assertions)]
        if review_cloud() {
            return review::answer(method, path, &body);
        }
        let response = self.fetch(method, path, &body, &kind, bearer.as_ref(), None, deadline)?;
        // A protected route asks for a human: the browser check once, then
        // the request once more with its clearance (never a loop).
        let response = match cloud_human::challenge(response.status, &response.body) {
            Some(challenge) => {
                let store = cloud_human::Clearances::new(&self.profile);
                let clearance = match store.get(self.origin, &challenge.scope, now_ms()) {
                    Some(clearance) => clearance,
                    None => {
                        let clearance = self.verify_human(&challenge.ticket)?;
                        store.put(self.origin, &challenge.scope, &clearance, now_ms());
                        clearance
                    }
                };
                let retried = self.fetch(
                    method,
                    path,
                    &body,
                    &kind,
                    bearer.as_ref(),
                    Some(&clearance),
                    deadline,
                )?;
                if retried.status == 403 {
                    store.forget(self.origin, &challenge.scope);
                }
                retried
            }
            None => response,
        };
        let value: Value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
        if (200..300).contains(&response.status) {
            return Ok(value);
        }
        let code = value["error"]
            .as_str()
            .or_else(|| value["error"]["code"].as_str())
            .filter(|c| c.len() <= 40 && c.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
            .unwrap_or("unknown");
        let kind = match response.status {
            401 | 403 => ServiceErrorCode::PermissionDenied,
            404 | 410 => ServiceErrorCode::NotFound,
            408 | 502..=504 => ServiceErrorCode::Offline,
            400 | 409 | 422 => ServiceErrorCode::InvalidInput,
            _ => ServiceErrorCode::Unavailable,
        };
        Err(ServiceError::message(kind, format!("api:{code}")))
    }
    /// One request to the registry's own origin (the only place a bearer or
    /// a human clearance is ever sent).
    #[allow(clippy::too_many_arguments)]
    fn fetch(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        kind: &Option<String>,
        bearer: Option<&Secret>,
        human: Option<&str>,
        deadline: std::time::Duration,
    ) -> pm_services::Result<pm_runtime_v8::FetchResponse> {
        let mut headers = vec![(
            "X-Powermove-Client".to_owned(),
            format!("desktop/{}", env!("CARGO_PKG_VERSION")),
        )];
        if let Some(kind) = kind {
            headers.push(("Content-Type".into(), kind.clone()));
        }
        if let Some(token) = bearer {
            headers.push(("Authorization".into(), format!("Bearer {}", token.expose())));
        }
        if let Some(clearance) = human {
            headers.push(("X-Powermove-Human".into(), clearance.to_owned()));
        }
        pm_compat_runner::net::send(
            pm_runtime_v8::FetchRequest {
                url: format!("{}{}", self.origin.trim_end_matches('/'), path),
                method: method.into(),
                headers,
                body: body.to_vec(),
                max_response_bytes: 1024 * 1024,
                deadline: std::time::Instant::now() + deadline,
            },
            pm_compat_runner::net::Reach::Origin(self.origin),
        )
        .map_err(|_| ServiceError::message(ServiceErrorCode::Offline, "network"))
    }
    /// The browser check: its page opens with the ticket only, and its result
    /// is asked for until it passes, fails, the sheet cancels it or three
    /// minutes go by.
    fn verify_human(&mut self, ticket: &str) -> pm_services::Result<String> {
        let unverified =
            || ServiceError::message(ServiceErrorCode::PermissionDenied, "api:human_unverified");
        let page = url::Url::parse_with_params(
            &format!("{}/v1/human", self.origin.trim_end_matches('/')),
            [("ticket", ticket)],
        )
        .map_err(|_| unverified())?;
        let gate = self.human.clone();
        gate.start();
        struct End<'a>(&'a HumanGate);
        impl Drop for End<'_> {
            fn drop(&mut self) {
                self.0.end();
            }
        }
        let _end = End(&gate);
        if !(self.opener)(page.as_str()) {
            return Err(unverified());
        }
        let ask = json!({ "ticket": ticket }).to_string().into_bytes();
        let kind = Some("application/json".to_owned());
        let until = std::time::Instant::now() + self.patience;
        while std::time::Instant::now() < until {
            // Wait, a slice at a time, so Cancel stops it at once.
            let next = std::time::Instant::now() + self.poll;
            while std::time::Instant::now() < next {
                if gate.cancelled() {
                    return Err(ServiceError::message(
                        ServiceErrorCode::PermissionDenied,
                        "api:human_cancelled",
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(50).min(self.poll));
            }
            let Ok(result) = self.fetch(
                "POST",
                "/v1/human/result",
                &ask,
                &kind,
                None,
                None,
                std::time::Duration::from_secs(10),
            ) else {
                continue;
            };
            match result.status {
                204 => continue,
                200..=299 => {
                    let value: Value = serde_json::from_slice(&result.body).unwrap_or_default();
                    return value["clearance"]
                        .as_str()
                        .filter(|c| !c.is_empty() && c.len() <= 2048)
                        .map(str::to_owned)
                        .ok_or_else(unverified);
                }
                _ => break,
            }
        }
        Err(unverified())
    }
    /// `POST` of git loose objects as `multipart/form-data`, one part per
    /// object named by its id (the registry's upload route).
    pub(crate) fn multipart(
        &mut self,
        path: &str,
        parts: &[(String, Vec<u8>)],
    ) -> pm_services::Result<Value> {
        let boundary = format!("powermove-{:016x}", now_ms());
        let mut body = Vec::new();
        for (sha, bytes) in parts {
            body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{sha}\"; filename=\"{sha}\"\r\nContent-Type: application/x-git-loose-object\r\n\r\n").as_bytes());
            body.extend_from_slice(bytes);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        self.send(
            "POST",
            path,
            body,
            Some(format!("multipart/form-data; boundary={boundary}")),
            true,
        )
    }
    fn cache_path(&self) -> PathBuf {
        self.profile.join("cloud/account.json")
    }
    /// Forgets the session on this Mac; a Keychain item that stays is an
    /// error (`keychain`), never a quiet sign-in on the next launch.
    fn forget(&mut self) -> pm_services::Result<()> {
        let _ = std::fs::remove_file(self.cache_path());
        self.vault
            .delete()
            .map_err(|_| ServiceError::message(ServiceErrorCode::Unavailable, "keychain"))
    }
    pub fn signed_in(&self) -> bool {
        stored_token(self.vault.as_ref(), self.origin).is_some()
    }
    /// The session Aero runs on, with the origin it belongs to. A
    /// development review account has no inference origin ("review").
    fn session(&self) -> Option<pm_agents::inference::Session> {
        let token = stored_token(self.vault.as_ref(), self.origin)?;
        #[cfg(debug_assertions)]
        if review_cloud() {
            return Some(pm_agents::inference::Session {
                origin: "review".into(),
                token: token.expose().to_owned(),
            });
        }
        Some(pm_agents::inference::Session {
            origin: self.origin.into(),
            token: token.expose().to_owned(),
        })
    }
    /// `DELETE /v1/me`, then the session is forgotten.
    pub fn delete_account(&mut self) -> pm_services::Result<()> {
        self.call("DELETE", "/v1/me", None, true)?;
        self.forget()
    }
    /// `GET /v2/billing`: the plan, this period's credits and what can be
    /// bought. A backend without the route says Missing.
    fn billing(&mut self) -> pm_services::Result<BillingAnswer> {
        match self.call("GET", pm_agents::inference::BILLING, None, true) {
            Ok(value) => pm_agents::inference::parse_billing(200, value.to_string().as_bytes())
                .map_err(|e| ServiceError::message(ServiceErrorCode::InvalidData, e)),
            Err(error) if error.code == ServiceErrorCode::NotFound => Ok(BillingAnswer::Missing),
            Err(error) if error.code == ServiceErrorCode::PermissionDenied => {
                Ok(BillingAnswer::SignedOut)
            }
            Err(error) => Err(error),
        }
    }
    /// `POST /v2/billing/checkout` or `/portal`: the Stripe page to open,
    /// when it is one of Stripe's (or, in a development build, the local
    /// stack's own page).
    fn billing_page(&mut self, page: &str, plan: Option<&str>) -> pm_services::Result<String> {
        let body = plan.map(|plan| json!({ "plan": plan }));
        let path = format!("{}/{page}", pm_agents::inference::BILLING);
        let value = self.call("POST", &path, body.or(Some(json!({}))), true)?;
        value["url"]
            .as_str()
            .and_then(billing_url)
            .ok_or_else(|| ServiceError::message(ServiceErrorCode::InvalidData, "billing_url"))
    }
    /// Remember Installs: what the Store installed or removed on this Mac.
    pub fn record_install(&mut self, repo: &str, release: Option<&str>) -> pm_services::Result<()> {
        match release {
            Some(release) => self
                .call(
                    "POST",
                    "/v1/installs",
                    Some(json!({"repoId":repo,"releaseId":release})),
                    true,
                )
                .map(drop),
            None => self
                .call("DELETE", &format!("/v1/installs/{repo}"), None, true)
                .map(drop),
        }
    }
}
impl CloudHost for Http {
    fn origin(&self) -> &str {
        self.origin
    }
    fn exchange(&mut self, exchange: Exchange) -> pm_services::Result<SessionPayload> {
        let body = json!({"state":exchange.state.expose(),"token":exchange.token.expose(),
            "verifier":exchange.verifier.expose()});
        self.call("POST", "/v1/auth/desktop/exchange", Some(body), false)
            .map(SessionPayload::new)
    }
    fn verify_email(&mut self, email: &str, code: &Secret) -> pm_services::Result<SessionPayload> {
        let body = json!({"email":email,"otp":code.expose()});
        self.call("POST", "/v1/auth/email/verify", Some(body), false)
            .map(SessionPayload::new)
    }
    fn send_email(&mut self, email: &str) -> pm_services::Result<()> {
        self.call(
            "POST",
            "/v1/auth/email/send",
            Some(json!({ "email": email })),
            false,
        )
        .map(drop)
    }
    fn save_session(&mut self, session: SessionPayload, _: &str) -> pm_services::Result<()> {
        let session = session.expose();
        let (Some(token), Some(expires)) =
            (session["token"].as_str(), session["expiresAt"].as_str())
        else {
            return Err(ServiceError::message(
                ServiceErrorCode::InvalidData,
                "session",
            ));
        };
        let sealed =
            json!({"origin":self.origin.trim_end_matches('/'),"token":token,"expiresAt":expires});
        self.vault
            .write(sealed.to_string().as_bytes())
            .map_err(|e| ServiceError::message(ServiceErrorCode::Unavailable, e))
    }
    fn refresh_account(&mut self) -> pm_services::Result<Option<Account>> {
        if !self.signed_in() {
            return Ok(None);
        }
        let me = match self.call("GET", "/v1/me", None, true) {
            Ok(me) => me,
            // The registry ended the session.
            Err(error) if error.to_string() == "api:unauthorized" => {
                self.forget()?;
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        // Telemetry joins this install to the account's others by its
        // opaque id (never the e-mail).
        if let Some(id) = me["user"]["id"].as_str() {
            pm_telemetry::identify(id);
        }
        let account = Account {
            email: me["user"]["email"].as_str().unwrap_or_default().to_owned(),
            handle: me["publisher"]["handle"].as_str().map(str::to_owned),
            image: me["user"]["image"].as_str().map(str::to_owned),
            remember_installs: me["settings"]["rememberInstalls"].as_bool().unwrap_or(true),
        };
        let cache = self.cache_path();
        if let Some(parent) = cache.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(
            cache,
            json!({"origin":self.origin,"account":account}).to_string(),
        );
        Ok(Some(account))
    }
    fn claim_handle(&mut self, handle: &str) -> pm_services::Result<()> {
        self.call(
            "POST",
            "/v1/me/handle",
            Some(json!({ "handle": handle })),
            true,
        )
        .map(drop)
    }
    fn remember_installs(&mut self, remember: bool) -> pm_services::Result<()> {
        self.call(
            "PATCH",
            "/v1/me/settings",
            Some(json!({ "rememberInstalls": remember })),
            true,
        )
        .map(drop)
    }
    /// This Mac forgets the session first, so Sign Out never waits on the
    /// network; then the registry is told, best effort, with the token it
    /// held (an env session's too: it is gone for the life of the process).
    fn sign_out(&mut self) -> pm_services::Result<()> {
        let token = stored_token(self.vault.as_ref(), self.origin);
        let forgotten = self.forget();
        if let Some(token) = token {
            let _ = self.send_as(
                "POST",
                "/v1/auth/sign-out",
                Vec::new(),
                None,
                Some(token),
                REVOKE_DEADLINE,
            );
        }
        forgotten
    }
}
/// The account cached for offline display, when it belongs to `origin`.
fn cached(profile: &Path, origin: &str) -> Option<Account> {
    let value: Value =
        serde_json::from_slice(&std::fs::read(profile.join("cloud/account.json")).ok()?).ok()?;
    (value["origin"] == origin)
        .then(|| serde_json::from_value(value["account"].clone()).ok())
        .flatten()
}

// ---- App state ----

/// A sign-in with Google, waiting for its `powermove://auth` return link:
/// told the account's email, or the error.
type Google = Box<dyn FnOnce(Result<String, String>, &mut App)>;
struct Cloud {
    profile: PathBuf,
    origin: &'static str,
    account: Option<Account>,
    signed_in: bool,
    revision: u64,
    worker: ServiceWorker<Http>,
    browser: BrowserAuth,
    google: Option<Google>,
    busy: bool,
    billing: Billed,
    /// The browser check the worker may be waiting on.
    human: std::sync::Arc<HumanGate>,
}
impl Global for Cloud {}
/// The account's billing, as Settings › Account shows it.
#[derive(Default)]
struct Billed {
    state: Option<Result<Option<Billing>, ()>>,
    /// Checkout or the Portal is open in the browser.
    waiting: Option<&'static str>,
    /// The last Subscribe or Manage Billing that failed, in words.
    error: Option<String>,
    read_at: Option<std::time::Instant>,
    reading: bool,
}

/// Starts the account service for this profile and refreshes a stored session.
pub fn init(profile: PathBuf, cx: &mut App) {
    let origin = store_install::origin();
    let worker_profile = profile.clone();
    init_with(
        profile,
        origin,
        move || {
            // A local development stack's session, given at launch
            // (`PM_CLOUD_SESSION`), lives in memory, never the Keychain. The
            // worker reads it once: after Sign Out it is gone for good.
            if let Some(sealed) = local_session(origin) {
                return Http::new(origin, Box::new(Memory(Some(sealed))), worker_profile);
            }
            // A development review's account lives in memory, never the Keychain.
            #[cfg(debug_assertions)]
            if review_cloud() {
                return Http::new(
                    origin,
                    Box::new(review::Memory::seeded(origin)),
                    worker_profile,
                );
            }
            Http::new(
                origin,
                Box::new(Keychain::for_app(&worker_profile)),
                worker_profile,
            )
        },
        cx,
    );
}
/// `init` on the host `host` makes (on the worker) for `origin`.
pub(crate) fn init_with(
    profile: PathBuf,
    origin: &'static str,
    host: impl FnOnce() -> Http + Send + 'static,
    cx: &mut App,
) {
    if cx.has_global::<Cloud>() {
        return;
    }
    let human = std::sync::Arc::new(HumanGate::default());
    let gate = human.clone();
    let Ok(worker) = ServiceWorker::start(move || Ok(host().with_gate(gate))) else {
        return;
    };
    cx.set_global(Cloud {
        account: cached(&profile, origin),
        profile,
        origin,
        signed_in: false,
        revision: 1,
        worker,
        browser: BrowserAuth::default(),
        google: None,
        busy: false,
        billing: Billed::default(),
        human,
    });
    follow_activation(cx);
    // Whether a session is stored, and who it is, off the main thread.
    if let Ok(pending) = cx.global::<Cloud>().worker.submit(|host| {
        let signed = host.signed_in();
        Ok((
            signed,
            if signed {
                host.refresh_account()
            } else {
                Ok(None)
            },
        ))
    }) {
        cx.spawn(async move |cx| {
            let Some((signed, account)) = wait(pending, cx).await else {
                return;
            };
            cx.update(|cx| {
                let cloud = cx.global_mut::<Cloud>();
                cloud.signed_in = signed;
                // Offline: the cached account stays for display.
                if let Ok(account) = account {
                    cloud.account = account.or(cloud.account.take().filter(|_| signed));
                }
                if !signed {
                    cloud.account = None;
                }
                changed(cx);
                if signed {
                    refresh_billing(cx);
                }
            });
        })
        .detach();
    }
}
/// A session held in memory only (a local stack's, a review's).
#[derive(Default)]
pub(crate) struct Memory(pub(crate) Option<Vec<u8>>);
impl Vault for Memory {
    fn read(&self) -> Result<Option<Vec<u8>>, String> {
        Ok(self.0.clone())
    }
    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.0 = Some(bytes.to_vec());
        Ok(())
    }
    fn delete(&mut self) -> Result<(), String> {
        self.0 = None;
        Ok(())
    }
}
/// The session `PM_CLOUD_SESSION` gives a build on a local development
/// stack (debug, or the Sandbox's `dev-cloud-origin`), sealed for `origin`:
/// signed in as that stack's account from the start. Never on the real
/// cloud, never in a shipped release.
fn local_session(origin: &str) -> Option<Vec<u8>> {
    local_session_from(origin, std::env::var("PM_CLOUD_SESSION").ok())
}
pub(crate) fn local_session_from(origin: &str, token: Option<String>) -> Option<Vec<u8>> {
    if !store_install::LOCAL_CLOUD || !pm_agents::inference::local_origin(origin) {
        return None;
    }
    let token =
        token.filter(|t| !t.is_empty() && t.len() <= 4096 && !t.chars().any(char::is_control))?;
    Some(
        json!({"origin":origin,"token":token,"expiresAt":"2099-01-01T00:00:00.000Z"})
            .to_string()
            .into_bytes(),
    )
}
async fn wait<T>(pending: pm_services::worker::Pending<T>, cx: &mut AsyncApp) -> Option<T> {
    loop {
        match pending.try_take() {
            Ok(Some(value)) => return Some(value),
            Ok(None) => {}
            Err(_) => return None,
        }
        cx.background_executor()
            .timer(std::time::Duration::from_millis(16))
            .await;
    }
}
/// The Powermove account as Aero reads it: the session, asked of the
/// account's own worker each time (never cached), so a sign-out or another
/// account is seen by the next request. Called on workers, never on main.
#[derive(Clone)]
pub struct AeroAccount(ServiceWorker<Http>);
impl std::fmt::Debug for AeroAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AeroAccount")
    }
}
impl pm_agents::inference::PowermoveAccount for AeroAccount {
    fn session(&self) -> Option<pm_agents::inference::Session> {
        let pending = self.0.submit(|host| Ok(host.session())).ok()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match pending.try_take() {
                Ok(Some(session)) => return session,
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                _ => return None,
            }
        }
    }
}
/// The account Aero runs on, once the account service has started.
pub fn aero_account(
    cx: &App,
) -> Option<std::sync::Arc<dyn pm_agents::inference::PowermoveAccount>> {
    let cloud = cx.try_global::<Cloud>()?;
    Some(std::sync::Arc::new(AeroAccount(cloud.worker.clone())))
}
/// Settings › Account reads this through the built-in facts.
pub fn fact(cx: &App) -> Value {
    let Some(cloud) = cx.try_global::<Cloud>() else {
        return Value::Null;
    };
    let account = cloud.account.as_ref().filter(|_| cloud.signed_in);
    json!({"revision":cloud.revision,"signedIn":account.is_some(),
        "identity":account.map(Account::identity),"email":account.map(|a| a.email.clone()),
        "rememberInstalls":account.map(|a| a.remember_installs),"busy":cloud.busy,
        "billing":account.map(|_| billing_fact(&cloud.billing))})
}
/// Settings › Account's plan card and credits (`pm_builtins` builds the rows).
fn billing_fact(billed: &Billed) -> Value {
    use pm_agents::inference::{Status, short_day};
    let billing = match &billed.state {
        None => return json!({"state":"loading"}),
        Some(Err(())) => return json!({"state":"error"}),
        Some(Ok(None)) => return json!({"state":"missing"}),
        Some(Ok(Some(billing))) => billing,
    };
    json!({"state":"ready","subscribed":billing.subscribed(),
        "plan":billing.plan.as_ref().map(|p| json!({"id":p.id,"name":p.name,"priceCents":p.price_cents})),
        "status":billing.status.map(|s| match s {
            Status::Active => "active",
            Status::PastDue => "past_due",
            Status::Canceled => "canceled",
        }),
        "cancelAtPeriodEnd":billing.cancel_at_period_end,
        "credits":{"included":billing.included,"used":billing.used,"remaining":billing.remaining},
        "resets":billing.resets().map(short_day),
        // The period's length in days: the daily chart's slots.
        "periodDays":billing.period.map(|(start, end)| {
            let days = |d: pm_agents::inference::Day| pm_services::surfaces::store::civil_days(d);
            (days(end) - days(start)).max(0)
        }),
        "daily":billing.daily.iter().map(|(day, credits)| json!([short_day(*day), credits])).collect::<Vec<_>>(),
        "plans":billing.plans.iter().map(|p| json!({"id":p.id,"name":p.name,"priceCents":p.price_cents,"credits":p.credits})).collect::<Vec<_>>(),
        "upgrade":billing.upgrade().map(|p| json!({"id":p.id,"name":p.name,"credits":p.credits})),
        "waiting":billed.waiting,"error":billed.error})
}
/// Every editor republishes its built-ins, so Settings shows the change.
fn changed(cx: &mut App) {
    cx.global_mut::<Cloud>().revision += 1;
    // Deferred: a command arrives inside its editor's update, and every
    // editor (that one too) republishes Settings.
    cx.defer(|cx| {
        for editor in crate::entry::editors(cx) {
            editor.update(cx, |c, cx| c.update_builtins(cx));
        }
    });
}

/// The email code's two steps for the first run's account step, on the same
/// worker, host and session store as the sign-in sheet: `send_code` asks the
/// registry to email a code; `verify_code` checks it, saves the session and
/// reads the account. `done` gets the registry's error code on a failure
/// (`api:unauthorized` wrong, `api:gone` expired, `network`).
pub fn send_code(
    email: String,
    cx: &mut App,
    done: impl FnOnce(Result<(), String>, &mut App) + 'static,
) {
    run_code(CloudWork::SendEmail { email }, cx, move |result, cx| {
        done(result.map(drop), cx)
    });
}
pub fn verify_code(
    email: String,
    code: String,
    cx: &mut App,
    done: impl FnOnce(Result<String, String>, &mut App) + 'static,
) {
    let work = CloudWork::VerifyEmail {
        email: email.clone(),
        code: Secret::new(code),
    };
    run_code(work, cx, move |result, cx| {
        done(
            result.map(|reply| match reply {
                CloudReply::Account(Some(account)) if !account.email.is_empty() => account.email,
                _ => email,
            }),
            cx,
        )
    });
}
fn run_code(
    work: CloudWork,
    cx: &mut App,
    done: impl FnOnce(Result<CloudReply, String>, &mut App) + 'static,
) {
    let Some(cloud) = cx.try_global::<Cloud>() else {
        return done(Err("unavailable".into()), cx);
    };
    let ticket = pm_services::surfaces::Flight::default().start();
    let Some(task) = CloudTask::new(ticket, cloud.origin, work) else {
        return done(Err("unavailable".into()), cx);
    };
    let Ok(pending) = cloud.worker.submit(move |host| Ok(task.run(host))) else {
        return done(Err("unavailable".into()), cx);
    };
    cx.spawn(async move |cx| {
        let result = wait(pending, cx)
            .await
            .map(|done| done.result.map_err(|e| e.to_string()))
            .unwrap_or_else(|| Err("unavailable".into()));
        cx.update(|cx| {
            if let Ok(CloudReply::Account(account)) = &result {
                let cloud = cx.global_mut::<Cloud>();
                cloud.signed_in = account.is_some();
                cloud.account = account.clone();
                changed(cx);
                refresh_billing(cx);
            }
            done(result, cx);
        });
    })
    .detach();
}
/// The sign-in sheet's sentence for a failed send (`verifying: false`) or
/// check of a code, for the first run's account step.
/// The account worker is waiting for the browser check.
pub(crate) fn verifying_human(cx: &App) -> bool {
    cx.try_global::<Cloud>()
        .is_some_and(|c| c.human.verifying())
}
/// The sign-in sheet's Cancel while the browser check is open.
pub(crate) fn cancel_human(cx: &App) {
    if let Some(cloud) = cx.try_global::<Cloud>() {
        cloud.human.cancel();
    }
}
pub(crate) fn code_error(code: &str, verifying: bool) -> String {
    if matches!(code, "api:human_unverified" | "api:human_cancelled") {
        return "Verification wasn’t completed. Try again.".into();
    }
    let error = match (verifying, code) {
        (_, "api:rate_limited") => AuthError::RateLimited,
        (_, "api:client_too_old") => AuthError::OldClient,
        (false, "network") => AuthError::Offline,
        (false, _) => AuthError::EmailRejected,
        (true, "api:unauthorized" | "api:bad_request") => AuthError::WrongCode,
        (true, "api:gone") => AuthError::ExpiredCode,
        (true, _) => AuthError::CheckCode,
    };
    copy::auth_error(error, "", "")
}
/// Development builds: the account step against a review backend
/// (`PM_NATIVE_REVIEW_CLOUD=1`): no network, no Keychain; signed in from the
/// start with `PM_NATIVE_REVIEW_SIGNED_IN=<email>`.
#[cfg(debug_assertions)]
fn review_cloud() -> bool {
    std::env::var_os("PM_NATIVE_REVIEW_CLOUD").is_some()
}
/// The review backend: code 123456 signs in, 000000 is wrong, 111111 has
/// expired; an address at `offline.example` can't be reached; the handle
/// `taken` is someone else's. The session stays in memory.
#[cfg(debug_assertions)]
mod review {
    use super::*;
    use std::sync::Mutex;
    static EMAIL: Mutex<String> = Mutex::new(String::new());
    /// The review account's billing: free, aerolite, aero, cancelling,
    /// past_due or out (`PM_NATIVE_REVIEW_BILLING`, or a review link).
    static BILLING: std::sync::LazyLock<Mutex<String>> = std::sync::LazyLock::new(|| {
        Mutex::new(std::env::var("PM_NATIVE_REVIEW_BILLING").unwrap_or_else(|_| "free".into()))
    });
    pub fn set_billing(state: &str) {
        *BILLING.lock().unwrap() = state.to_owned();
    }
    /// `GET /v2/billing` for a review state, shaped as the contract says.
    fn billing(state: &str) -> Value {
        let plans = json!([{"id":"aerolite","name":"Aerolite","priceCents":999,"credits":1000},
            {"id":"aero","name":"Aero","priceCents":1999,"credits":2500}]);
        if state == "free" {
            return json!({"plan":null,"status":null,"cancelAtPeriodEnd":false,
                "credits":{"included":0,"used":0,"remaining":0},"period":null,"daily":[],"plans":plans});
        }
        let aero = state == "aero";
        let (id, name, price, included) = if aero {
            ("aero", "Aero", 1999, 2500)
        } else {
            ("aerolite", "Aerolite", 999, 1000)
        };
        let mut spent: Vec<u64> = [40, 12, 0, 66, 18, 0, 0, 24, 9, 5, 0, 14, 0, 0]
            .iter()
            .map(|c| if aero { c * 3 } else { *c })
            .collect();
        if state == "out" {
            let before: u64 = spent[..spent.len() - 1].iter().sum();
            *spent.last_mut().unwrap() = included - before;
        }
        let used: u64 = spent.iter().sum();
        let daily: Vec<Value> = spent
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let date = if i < 6 {
                    format!("2026-11-{}", 25 + i)
                } else {
                    format!("2026-12-{:02}", i - 5)
                };
                json!({"date": date, "credits": c})
            })
            .collect();
        json!({"plan":{"id":id,"name":name,"priceCents":price,"interval":"month"},
            "status":if state == "past_due" { "past_due" } else { "active" },
            "cancelAtPeriodEnd":state == "cancelling",
            "credits":{"included":included,"used":used,"remaining":included - used},
            "period":{"start":"2026-11-25T00:00:00Z","end":"2026-12-25T00:00:00Z"},
            "daily":daily,"plans":plans})
    }
    static HANDLE: Mutex<String> = Mutex::new(String::new());
    pub(super) use super::Memory;
    impl Memory {
        /// Signed in from the start as `PM_NATIVE_REVIEW_SIGNED_IN` (an
        /// email), when the review launch names one; else signed out.
        pub fn seeded(origin: &str) -> Self {
            let Ok(email) = std::env::var("PM_NATIVE_REVIEW_SIGNED_IN") else {
                return Self::default();
            };
            *EMAIL.lock().unwrap() = email;
            Self(Some(
                json!({"origin":origin,"token":"review-session","expiresAt":"2099-01-01T00:00:00.000Z"})
                    .to_string()
                    .into_bytes(),
            ))
        }
    }
    pub fn answer(method: &str, path: &str, body: &[u8]) -> pm_services::Result<Value> {
        let body: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let fail = |kind, code: &str| Err(ServiceError::message(kind, code.to_owned()));
        // A beat, as a network would take.
        std::thread::sleep(std::time::Duration::from_millis(700));
        match (method, path) {
            ("POST", "/v1/auth/email/send") => {
                let email = body["email"].as_str().unwrap_or_default();
                if email.ends_with("@offline.example") {
                    return fail(ServiceErrorCode::Offline, "network");
                }
                *EMAIL.lock().unwrap() = email.to_owned();
                Ok(json!({ "ok": true }))
            }
            ("POST", "/v1/auth/email/verify") => match body["otp"].as_str() {
                Some("123456") => {
                    Ok(json!({"token":"review-session","expiresAt":"2099-01-01T00:00:00.000Z"}))
                }
                Some("111111") => fail(ServiceErrorCode::NotFound, "api:gone"),
                _ => fail(ServiceErrorCode::PermissionDenied, "api:unauthorized"),
            },
            ("GET", "/v2/billing") => Ok(billing(&BILLING.lock().unwrap())),
            // Checkout "paid" at once (as its webhook would land before the
            // browser comes back); the Portal changes nothing.
            ("POST", "/v2/billing/checkout") => {
                let plan = body["plan"].as_str().unwrap_or_default().to_owned();
                if !matches!(plan.as_str(), "aerolite" | "aero") {
                    return fail(ServiceErrorCode::InvalidInput, "api:bad_plan");
                }
                if *BILLING.lock().unwrap() != "free" {
                    return fail(ServiceErrorCode::InvalidInput, "api:has_plan");
                }
                *BILLING.lock().unwrap() = plan.clone();
                Ok(json!({"url": format!("https://checkout.stripe.com/c/pay/cs_review_{plan}")}))
            }
            ("POST", "/v2/billing/portal") => {
                Ok(json!({"url": "https://billing.stripe.com/p/session/review"}))
            }
            ("GET", "/v1/me") => {
                let handle = HANDLE.lock().unwrap().clone();
                Ok(json!({"user":{"id":"review","email":EMAIL.lock().unwrap().clone()},
                    "publisher":{"handle":(!handle.is_empty()).then_some(handle)}}))
            }
            // A new account's handle; "taken" is someone else's.
            ("POST", "/v1/me/handle") => match body["handle"].as_str() {
                Some("taken") => fail(ServiceErrorCode::InvalidInput, "api:handle_taken"),
                Some(handle) => {
                    *HANDLE.lock().unwrap() = handle.to_owned();
                    Ok(json!({ "ok": true }))
                }
                None => fail(ServiceErrorCode::InvalidInput, "api:handle_invalid"),
            },
            _ => Ok(json!({ "ok": true })),
        }
    }
}
/// Development review link `powermove://review/billing/<state>`: the review
/// account's billing changes (as its webhooks would) and is read again.
#[cfg(debug_assertions)]
pub fn review_billing(state: &str, cx: &mut App) {
    if review_cloud() {
        review::set_billing(state);
        refresh_billing(cx);
    }
}
/// Who is signed in, if anyone: the account's email (its handle if it has
/// no email), as the first run's account step names it.
pub fn identity(cx: &App) -> Option<String> {
    let cloud = cx.try_global::<Cloud>()?;
    let account = cloud.account.as_ref().filter(|_| cloud.signed_in)?;
    Some(if account.email.is_empty() {
        account.identity()
    } else {
        account.email.clone()
    })
}
/// Calls `f` with who is signed in whenever the account changes.
pub fn on_change(cx: &mut App, f: impl Fn(Option<String>, &mut App) + 'static) {
    cx.observe_global::<Cloud>(move |cx| f(identity(cx), cx))
        .detach();
}
pub(crate) fn sign_in_available(cx: &App) -> bool {
    cx.has_global::<Cloud>()
}

/// Settings › Account › Sign In…, Home's and the Store's: the sign-in sheet
/// on `window` (`sign_in`, the first run's sign-in in a sheet).
pub fn open_sign_in(window: Retained<NSWindow>, cx: &mut App) {
    if cx.has_global::<Cloud>() {
        crate::sign_in::open(window, cx);
    }
}
/// Who is signed in has no publisher handle yet (a new account).
pub(crate) fn needs_handle(cx: &App) -> bool {
    cx.try_global::<Cloud>()
        .and_then(|c| c.account.as_ref().filter(|_| c.signed_in))
        .is_some_and(|a| a.handle.as_deref().is_none_or(str::is_empty))
}
/// The sign-in sheet's handle step: claim `handle` for the account.
/// `done` gets the registry's error code on a failure.
pub(crate) fn claim_handle(
    handle: String,
    cx: &mut App,
    done: impl FnOnce(Result<(), String>, &mut App) + 'static,
) {
    run_code(CloudWork::ClaimHandle { handle }, cx, move |result, cx| {
        done(result.map(drop), cx)
    });
}
/// The handle step's sentence for a failure.
pub(crate) fn handle_error(code: &str, handle: &str) -> String {
    let error = match code {
        "api:rate_limited" => AuthError::RateLimited,
        "api:client_too_old" => AuthError::OldClient,
        "api:handle_taken" => AuthError::HandleTaken,
        "api:handle_reserved" => AuthError::HandleReserved,
        "api:handle_invalid" => AuthError::InvalidHandle,
        _ => AuthError::SaveHandle,
    };
    copy::auth_error(error, "Google", handle)
}
pub(crate) fn invalid_handle(handle: &str) -> Option<String> {
    (!valid_handle(handle)).then(|| copy::auth_error(AuthError::InvalidHandle, "", handle))
}
/// Continue with Google: the registry's page opens in the browser and the
/// `powermove://auth` return link finishes the sign-in, telling `done` the
/// account's email or why not. `Err` here: the browser could not open.
pub(crate) fn sign_in_with_google(
    cx: &mut App,
    done: impl FnOnce(Result<String, String>, &mut App) + 'static,
) -> Result<(), String> {
    let mut verifier = [0u8; 32];
    let mut state = [0u8; 16];
    if !random(&mut verifier) || !random(&mut state) {
        return Err(copy::auth_error(AuthError::Browser, "Google", ""));
    }
    let cloud = cx.global_mut::<Cloud>();
    let origin = cloud.origin;
    let Some(url) = cloud.browser.begin(origin, verifier, state, now_ms()) else {
        return Err(copy::auth_error(AuthError::Browser, "Google", ""));
    };
    // A development build reading a fixture Store never opens a browser over
    // the person's work: the review plays the browser from this file.
    if store_install::is_dev() {
        let _ = std::fs::write(cloud.profile.join("dev-sign-in-url.txt"), url.as_str());
    } else if !crate::platform::open_url(url.as_str()) {
        cloud.browser.cancel();
        return Err(copy::auth_error(AuthError::Browser, "Google", ""));
    }
    cloud.google = Some(Box::new(done));
    Ok(())
}
/// The sheet closed or went back while Google was open: a late return link
/// finds nothing waiting.
pub(crate) fn cancel_google(cx: &mut App) {
    if cx.has_global::<Cloud>() {
        let cloud = cx.global_mut::<Cloud>();
        cloud.google = None;
        cloud.browser.cancel();
    }
}
fn random(bytes: &mut [u8]) -> bool {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(bytes))
        .is_ok()
}
/// A `powermove://auth` return link (entrypoints). Only a Google sign-in
/// waiting for it takes it; anything else stays an Error Log line.
pub fn receive_link(state: &str, token: &str, cx: &mut App) -> bool {
    if !cx.has_global::<Cloud>() {
        return false;
    }
    let cloud = cx.global_mut::<Cloud>();
    let waiting = cloud.google.is_some();
    let raw = url::Url::parse_with_params("powermove://auth", [("state", state), ("token", token)])
        .map(String::from)
        .unwrap_or_default();
    let origin = cloud.origin;
    let Some(exchange) = cloud
        .browser
        .receive_for_sheet(&raw, origin, now_ms(), waiting)
    else {
        return false;
    };
    let Some(done) = cloud.google.take() else {
        return false;
    };
    run_code(CloudWork::Exchange(exchange), cx, move |result, cx| {
        done(
            result
                .map(|reply| match reply {
                    CloudReply::Account(Some(account)) => account.email,
                    _ => String::new(),
                })
                .map_err(|_| copy::auth_error(AuthError::ReturnLink, "Google", "")),
            cx,
        )
    });
    true
}
/// Runs one Settings › Account action on the worker and applies its reply.
fn submit(work: CloudWork, cx: &mut App) {
    let cloud = cx.global_mut::<Cloud>();
    let ticket = pm_services::surfaces::Flight::default().start();
    let Some(task) = CloudTask::new(ticket, cloud.origin, work) else {
        // An origin this build can't reach for account changes: said, never
        // a button that quietly does nothing.
        crate::alert::note(&explain_settings("api:unknown"), "");
        return;
    };
    cloud.busy = true;
    let Ok(pending) = cloud.worker.submit(move |host| Ok(task.run(host))) else {
        cloud.busy = false;
        return;
    };
    cx.spawn(async move |cx| {
        let Some(done) = wait(pending, cx).await else {
            return;
        };
        cx.update(|cx| complete(done, cx));
    })
    .detach();
}
fn complete(done: pm_services::surfaces::agent_auth::CloudCompletion, cx: &mut App) {
    let cloud = cx.global_mut::<Cloud>();
    cloud.busy = false;
    match &done.result {
        Ok(CloudReply::Account(account)) => {
            cloud.signed_in = account.is_some();
            cloud.account = account.clone();
        }
        Ok(CloudReply::SignedOut) => {
            cloud.signed_in = false;
            cloud.account = None;
        }
        _ => {}
    }
    if let Err(error) = &done.result {
        crate::alert::note(&explain_settings(&error.to_string()), "");
    }
    changed(cx);
    refresh_billing(cx);
}
fn explain_settings(code: &str) -> String {
    if code == "network" {
        "Can’t reach Powermove Cloud. Check your connection and try again.".into()
    } else if code == "keychain" {
        "Powermove couldn’t remove your sign-in from the Keychain. Try again.".into()
    } else {
        "Your account couldn’t be updated. Try again.".into()
    }
}

// ---- Billing (Settings › Account) ----

/// Reads the account's billing on the worker; Settings › Account and Aero
/// follow the answer. Signed out: forgotten.
pub fn refresh_billing(cx: &mut App) {
    if !cx.has_global::<Cloud>() {
        return;
    }
    let cloud = cx.global_mut::<Cloud>();
    if !cloud.signed_in {
        cloud.billing = Billed::default();
        return;
    }
    if cloud.billing.reading {
        return;
    }
    cloud.billing.reading = true;
    let Ok(pending) = cloud.worker.submit(|host| Ok(host.billing())) else {
        cloud.billing.reading = false;
        return;
    };
    cx.spawn(async move |cx| {
        let answer = wait(pending, cx).await;
        cx.update(|cx| {
            let cloud = cx.global_mut::<Cloud>();
            cloud.billing.reading = false;
            cloud.billing.read_at = Some(std::time::Instant::now());
            match answer {
                Some(Ok(BillingAnswer::Billing(billing))) => {
                    // A plan bought: the wait for Checkout is over.
                    if billing.subscribed() && cloud.billing.waiting == Some("checkout") {
                        cloud.billing.waiting = None;
                    }
                    cloud.billing.state = Some(Ok(Some(billing)));
                }
                Some(Ok(BillingAnswer::Missing)) => cloud.billing.state = Some(Ok(None)),
                // The session ended: the account's own refresh signs out.
                Some(Ok(BillingAnswer::SignedOut)) => cloud.billing = Billed::default(),
                _ => {
                    // Offline: what was read stays; nothing read yet is an error.
                    if !matches!(cloud.billing.state, Some(Ok(Some(_)))) {
                        cloud.billing.state = Some(Err(()));
                    }
                }
            }
            changed(cx);
        });
    })
    .detach();
}
/// Subscribe: Stripe Checkout for `plan` opens in the browser, and the page
/// waits for it (the return link, or the app coming back).
pub fn subscribe(plan: String, cx: &mut App) {
    open_billing_page("checkout", Some(plan), cx);
}
/// Manage Billing: the Stripe Customer Portal opens in the browser.
pub fn manage_billing(cx: &mut App) {
    open_billing_page("portal", None, cx);
}
fn open_billing_page(page: &'static str, plan: Option<String>, cx: &mut App) {
    if !cx.has_global::<Cloud>() {
        return;
    }
    let cloud = cx.global_mut::<Cloud>();
    if cloud.busy || !cloud.signed_in {
        return;
    }
    cloud.busy = true;
    cloud.billing.error = None;
    let Ok(pending) = cloud
        .worker
        .submit(move |host| Ok(host.billing_page(page, plan.as_deref())))
    else {
        cloud.busy = false;
        return;
    };
    changed(cx);
    cx.spawn(async move |cx| {
        let answer = wait(pending, cx).await;
        cx.update(|cx| {
            let cloud = cx.global_mut::<Cloud>();
            cloud.busy = false;
            match answer {
                Some(Ok(url)) if crate::platform::open_url(&url) => {
                    cloud.billing.waiting = Some(page);
                }
                Some(Ok(_)) => {
                    cloud.billing.error =
                        Some("Powermove couldn’t open your browser. Try again.".into());
                }
                Some(Err(error)) => {
                    let code = error.to_string();
                    cloud.billing.error = Some(billing_error(&code).into());
                    // Already on a plan (bought in another window): read it.
                    if code == "api:has_plan" {
                        changed(cx);
                        return refresh_billing(cx);
                    }
                }
                None => cloud.billing.error = Some(billing_error("network").into()),
            }
            changed(cx);
        });
    })
    .detach();
}
fn billing_error(code: &str) -> &'static str {
    match code {
        "network" => "Can’t reach Powermove. Check your connection and try again.",
        "api:has_plan" => "You already have a plan. Manage it with Manage Billing.",
        "api:no_billing_account" => "There’s no billing to manage yet. Choose a plan first.",
        "api:rate_limited" => "Too many tries. Wait a moment and try again.",
        _ => "Billing couldn’t open. Try again.",
    }
}
/// Waiting for Checkout: Cancel stops waiting (the browser's page stays).
pub fn cancel_checkout(cx: &mut App) {
    if cx.has_global::<Cloud>() {
        cx.global_mut::<Cloud>().billing.waiting = None;
        changed(cx);
    }
}
/// The Stripe page a billing call may open: Checkout or the Portal on
/// Stripe's own hosts over https, or a local stack's page in a build that
/// may use one. Kept canonical; nothing else is ever opened.
fn billing_url(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw).ok()?;
    let plain = url.username().is_empty() && url.password().is_none() && url.fragment().is_none();
    let stripe = url.scheme() == "https"
        && url.port().is_none()
        && matches!(
            url.host_str(),
            Some("checkout.stripe.com" | "billing.stripe.com")
        );
    let local = pm_agents::inference::LOCAL_ORIGINS
        && url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "localhost"))
        && url.port().is_some();
    (plain && (stripe || local)).then(|| url.to_string())
}
/// `powermove://billing/return?status=success|cancel|portal`: Checkout
/// paid or cancelled, or the Portal closed, when the link is exactly that.
pub fn billing_return(raw: &str) -> Option<&'static str> {
    let url = url::Url::parse(raw).ok()?;
    if url.scheme() != "powermove"
        || url.host_str() != Some("billing")
        || url.path() != "/return"
        || !url.username().is_empty()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let mut pairs = url.query_pairs();
    let (key, value) = pairs.next()?;
    if key != "status" || pairs.next().is_some() {
        return None;
    }
    match value.as_ref() {
        "success" => Some("success"),
        "cancel" => Some("cancel"),
        "portal" => Some("portal"),
        _ => None,
    }
}
/// The browser came back from Checkout or the Portal: stop waiting, read
/// the plan again, and Settings › Account comes forward.
pub fn billing_returned(cx: &mut App) {
    if cx.has_global::<Cloud>() {
        cx.global_mut::<Cloud>().billing.waiting = None;
        refresh_billing(cx);
        changed(cx);
    }
    crate::settings_window::show_page("account", cx);
}
/// The app became active: while Checkout or the Portal is open, or when
/// the plan was read a while ago, it is read again.
fn follow_activation(cx: &mut App) {
    use objc2_app_kit::NSApplicationDidBecomeActiveNotification;
    let (tx, rx) = async_channel::unbounded::<()>();
    let block = block2::RcBlock::new(
        move |_: std::ptr::NonNull<objc2_foundation::NSNotification>| {
            let _ = tx.try_send(());
        },
    );
    // SAFETY: a notification name constant; the observer lives for the process.
    let token = unsafe {
        objc2_foundation::NSNotificationCenter::defaultCenter()
            .addObserverForName_object_queue_usingBlock(
                Some(NSApplicationDidBecomeActiveNotification),
                None,
                None,
                &block,
            )
    };
    std::mem::forget(token);
    cx.spawn(async move |cx| {
        while rx.recv().await.is_ok() {
            cx.update(|cx| {
                let Some(cloud) = cx.try_global::<Cloud>() else {
                    return;
                };
                let stale = cloud
                    .billing
                    .read_at
                    .is_none_or(|at| at.elapsed() > std::time::Duration::from_secs(300));
                if cloud.billing.waiting.is_some() || stale {
                    refresh_billing(cx);
                }
            });
        }
    })
    .detach();
}

/// Settings › Account › Sign Out, and Home's account menu. Signing out is
/// this Mac forgetting the session: it never depends on the registry's
/// origin or on the network (a development Sandbox on a local stack's
/// `http://` origin signs out like any other build). A Google sign-in
/// waiting for its return link is dropped with it.
pub fn sign_out(cx: &mut App) {
    if !cx.has_global::<Cloud>() {
        return;
    }
    let cloud = cx.global_mut::<Cloud>();
    cloud.google = None;
    cloud.browser.cancel();
    cloud.busy = true;
    let ticket = pm_services::surfaces::Flight::default().start();
    let Ok(pending) = cloud.worker.submit(move |host| {
        Ok(pm_services::surfaces::agent_auth::CloudCompletion {
            ticket,
            result: host.sign_out().map(|()| CloudReply::SignedOut),
        })
    }) else {
        cloud.busy = false;
        return;
    };
    cx.spawn(async move |cx| {
        let Some(done) = wait(pending, cx).await else {
            return;
        };
        cx.update(|cx| complete(done, cx));
    })
    .detach();
}
/// Settings › Account › Remember Installs on This Account.
pub fn set_remember(remember: bool, cx: &mut App) {
    if cx.has_global::<Cloud>() {
        submit(CloudWork::RememberInstalls(remember), cx);
    }
}
/// Settings › Account › Delete Account…, after its confirmation.
pub fn delete_account(owner: Retained<NSWindow>, cx: &mut App) {
    use crate::alert::{Alert, Button};
    let confirm = copy::delete_account();
    let rx = Alert::new(
        confirm.message,
        confirm.informative,
        vec![
            Button::new("Cancel", false).default().cancel(),
            Button::new("Delete Account", true).destructive(),
        ],
    )
    .sheet(&owner);
    cx.spawn(async move |cx| {
        if rx.recv().await != Ok(Some(true)) {
            return;
        }
        cx.update(|cx| {
            let pending = cx
                .global::<Cloud>()
                .worker
                .submit(|host| host.delete_account());
            let Ok(pending) = pending else {
                return;
            };
            cx.spawn(async move |cx| {
                let result = wait(pending, cx).await;
                cx.update(|cx| {
                    let cloud = cx.global_mut::<Cloud>();
                    if result.is_some() {
                        cloud.signed_in = false;
                        cloud.account = None;
                    } else {
                        crate::alert::note(&explain_settings("api:unknown"), "");
                    }
                    changed(cx);
                });
            })
            .detach();
        });
    })
    .detach();
}
/// A Store install or uninstall on an account that remembers installs (v1
/// `addInstall`/`deleteInstall`). Worker threads only; quiet on failure.
pub fn remember_install(profile: &Path, repo: &str, release: Option<&str>) {
    let mut http = Http::new(
        store_install::origin(),
        Box::new(Keychain::for_app(profile)),
        profile.to_owned(),
    );
    let remember = cached(profile, store_install::origin()).is_some_and(|a| a.remember_installs);
    if remember && http.signed_in() {
        let _ = http.record_install(repo, release);
    }
}

#[cfg(test)]
mod billing_tests {
    use super::{billing_return, billing_url};

    /// Only Stripe's Checkout and Portal over https open; a local stack's
    /// page only in a build that may use one.
    #[test]
    fn only_stripes_pages_open_from_billing() {
        for ok in [
            "https://checkout.stripe.com/c/pay/cs_test_1",
            "https://billing.stripe.com/p/session/test_1",
        ] {
            assert_eq!(billing_url(ok).as_deref(), Some(ok));
        }
        for refused in [
            "http://checkout.stripe.com/c/pay/cs_test_1",
            "https://checkout.stripe.com.evil.example/c",
            "https://evil.example/?u=https://checkout.stripe.com",
            "https://user@checkout.stripe.com/c",
            "https://checkout.stripe.com:8443/c",
            "https://stripe.com/c",
            "javascript:alert(1)",
            "file:///etc/passwd",
        ] {
            assert_eq!(billing_url(refused), None, "{refused}");
        }
        let local = billing_url("http://127.0.0.1:8787/v2/dev/checkout?s=1");
        assert_eq!(local.is_some(), pm_agents::inference::LOCAL_ORIGINS);
    }

    /// A shipped release never opens a local page from billing.
    #[cfg(all(not(debug_assertions), not(feature = "dev-cloud-origin")))]
    #[test]
    fn a_shipped_release_opens_only_stripe() {
        assert_eq!(billing_url("http://127.0.0.1:8787/checkout"), None);
        assert_eq!(billing_url("http://localhost:8787/checkout"), None);
    }

    #[test]
    fn the_billing_return_link_is_exactly_success_or_cancel() {
        for status in ["success", "cancel", "portal"] {
            let link = format!("powermove://billing/return?status={status}");
            assert_eq!(billing_return(&link), Some(status));
        }
        for other in [
            "powermove://billing/return",
            "powermove://billing/return?status=paid",
            "powermove://billing/return?status=success&x=1",
            "powermove://billing/return?status=success&status=portal",
            "powermove://billing/elsewhere?status=success",
            "powermove://auth?state=s&token=t",
            "https://billing/return?status=success",
        ] {
            assert_eq!(billing_return(other), None, "{other}");
        }
    }
}
