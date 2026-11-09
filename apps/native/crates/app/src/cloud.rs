//! The Powermove Cloud account (S083–S086): sign in with Google through the
//! browser (PKCE, the `powermove://auth` return link) or with an emailed
//! code, choose a publisher handle, Remember Installs, sign out and delete the
//! account. Protocol state lives in `pm-services` (`agent_auth`); this is the
//! host: the registry over the system HTTP stack (bearer only to the registry
//! origin, never across a foreign redirect), the session in the macOS
//! Keychain, AppKit sheets for the steps, and Settings › Accounts rows through
//! the built-in facts. Every request runs on one service worker, so an
//! exchange, its save and the refresh never interleave.
use crate::sheets::{OpenSheet, from_surface};
use gpui::{App, AsyncApp, Global};
use objc2::rc::Retained;
use objc2_app_kit::NSWindow;
use pm_compat_runner::store_install;
use pm_services::{
    ServiceError, ServiceErrorCode,
    surfaces::{
        agent_auth::{
            Account, BrowserAuth, CloudHost, CloudReply, CloudTask, CloudWork, Exchange, Secret,
            SessionPayload, SignIn, SignInStep, otp, valid_handle,
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
/// A generic password named after this app's bundle id, so a beta, a
/// release and each development bundle keep their own session.
pub struct Keychain {
    service: String,
}
impl Keychain {
    pub fn for_app() -> Self {
        let id = crate::platform::bundle_id().unwrap_or_else(|| "com.powermove.native.dev".into());
        Self {
            service: format!("{id} Powermove Cloud"),
        }
    }
}
const ACCOUNT: &str = "session";
#[cfg(target_os = "macos")]
mod keychain {
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
    pub fn read(service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(find(service, account, false)?.map(|(bytes, _)| bytes))
    }
    pub fn delete(service: &str, account: &str) -> Result<(), String> {
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
}
#[cfg(target_os = "macos")]
impl Vault for Keychain {
    fn read(&self) -> Result<Option<Vec<u8>>, String> {
        if let Some(bytes) = keychain::read(&self.service, ACCOUNT)? {
            return Ok(Some(bytes));
        }
        // A session the first launch brought from the shipped app (Settings ›
        // Accounts › Sign-Ins from the Original App). Release builds only: a
        // development build never reads a real account.
        if cfg!(debug_assertions) {
            return Ok(None);
        }
        let migrated = format!("{} migration secrets", pm_first_launch::BETA_BUNDLE_ID);
        let account = blake3::hash(b"cloud/session").to_hex().to_string();
        keychain::read(&migrated, &account)
    }
    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        keychain::write(&self.service, ACCOUNT, bytes)
    }
    fn delete(&mut self) -> Result<(), String> {
        keychain::delete(&self.service, ACCOUNT)
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

/// The registry for the account: the Store's origin.
pub struct Http {
    origin: &'static str,
    vault: Box<dyn Vault>,
    profile: PathBuf,
}
impl Http {
    pub fn new(origin: &'static str, vault: Box<dyn Vault>, profile: PathBuf) -> Self {
        Self {
            origin,
            vault,
            profile,
        }
    }
    /// One request; a failure names only the registry's error code
    /// (`api:handle_taken`) or the transport, never a body or a token.
    fn call(
        &mut self,
        method: &str,
        path: &str,
        body: Option<Value>,
        auth: bool,
    ) -> pm_services::Result<Value> {
        let mut headers = vec![(
            "X-Powermove-Client".to_owned(),
            format!("desktop/{}", env!("CARGO_PKG_VERSION")),
        )];
        if body.is_some() {
            headers.push(("Content-Type".into(), "application/json".into()));
        }
        if auth {
            let token = stored_token(self.vault.as_ref(), self.origin).ok_or_else(|| {
                ServiceError::message(ServiceErrorCode::PermissionDenied, "api:unauthorized")
            })?;
            headers.push(("Authorization".into(), format!("Bearer {}", token.expose())));
        }
        let response = pm_compat_runner::net::send(
            pm_runtime_v8::FetchRequest {
                url: format!("{}{}", self.origin.trim_end_matches('/'), path),
                method: method.into(),
                headers,
                body: body.map(|b| b.to_string().into_bytes()).unwrap_or_default(),
                max_response_bytes: 1024 * 1024,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(30),
            },
            pm_compat_runner::net::Reach::Origin(self.origin),
        )
        .map_err(|_| ServiceError::message(ServiceErrorCode::Offline, "network"))?;
        let value: Value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
        if (200..300).contains(&response.status) {
            return Ok(value);
        }
        let code = value["error"]
            .as_str()
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
    fn cache_path(&self) -> PathBuf {
        self.profile.join("cloud/account.json")
    }
    fn forget(&mut self) {
        let _ = self.vault.delete();
        let _ = std::fs::remove_file(self.cache_path());
    }
    pub fn signed_in(&self) -> bool {
        stored_token(self.vault.as_ref(), self.origin).is_some()
    }
    /// `DELETE /v1/me`, then the session is forgotten.
    pub fn delete_account(&mut self) -> pm_services::Result<()> {
        self.call("DELETE", "/v1/me", None, true)?;
        self.forget();
        Ok(())
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
                self.forget();
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
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
    fn sign_out(&mut self) -> pm_services::Result<()> {
        // Best effort at the registry; this Mac forgets the session either way.
        let _ = self.call("POST", "/v1/auth/sign-out", None, true);
        self.forget();
        Ok(())
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

struct Flow {
    signin: SignIn,
    email: String,
    error: Option<String>,
    sheet: Option<OpenSheet>,
    window: Retained<NSWindow>,
    /// The handle the person typed, for a "taken" message.
    handle: String,
}
struct Cloud {
    profile: PathBuf,
    origin: &'static str,
    account: Option<Account>,
    signed_in: bool,
    revision: u64,
    worker: ServiceWorker<Http>,
    browser: BrowserAuth,
    flow: Option<Flow>,
    busy: bool,
}
impl Global for Cloud {}

/// Starts the account service for this profile and refreshes a stored session.
pub fn init(profile: PathBuf, cx: &mut App) {
    if cx.has_global::<Cloud>() {
        return;
    }
    let origin = store_install::origin();
    let worker_profile = profile.clone();
    let Ok(worker) = ServiceWorker::start(move || {
        Ok(Http::new(
            origin,
            Box::new(Keychain::for_app()),
            worker_profile,
        ))
    }) else {
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
        flow: None,
        busy: false,
    });
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
            let _ = cx.update(|cx| {
                let cloud = cx.global_mut::<Cloud>();
                cloud.signed_in = signed;
                match account {
                    Ok(account) => {
                        cloud.account = account.or(cloud.account.take().filter(|_| signed))
                    }
                    // Offline: the cached account stays for display.
                    Err(_) => {}
                }
                if !signed {
                    cloud.account = None;
                }
                changed(cx);
            });
        })
        .detach();
    }
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
/// Settings › Accounts reads this through the built-in facts.
pub fn fact(cx: &App) -> Value {
    let Some(cloud) = cx.try_global::<Cloud>() else {
        return Value::Null;
    };
    let account = cloud.account.as_ref().filter(|_| cloud.signed_in);
    json!({"revision":cloud.revision,"signedIn":account.is_some(),
        "identity":account.map(Account::identity),"email":account.map(|a| a.email.clone()),
        "rememberInstalls":account.map(|a| a.remember_installs),"busy":cloud.busy})
}
/// Every editor republishes its built-ins, so Settings shows the change.
fn changed(cx: &mut App) {
    cx.global_mut::<Cloud>().revision += 1;
    for editor in crate::entry::editors(cx) {
        editor.update(cx, |c, cx| c.update_builtins(cx));
    }
}

/// Settings › Accounts › Sign In…: the first step's sheet on `window`.
pub fn open_sign_in(window: Retained<NSWindow>, cx: &mut App) {
    if !cx.has_global::<Cloud>() {
        return;
    }
    let cloud = cx.global_mut::<Cloud>();
    if cloud.flow.is_some() {
        return;
    }
    let account = cloud.account.clone().filter(|_| cloud.signed_in);
    cloud.flow = Some(Flow {
        signin: SignIn::new(account.as_ref()),
        email: account.map(|a| a.email).unwrap_or_default(),
        error: None,
        sheet: None,
        window,
        handle: String::new(),
    });
    present(cx);
}
fn surface(flow: &Flow) -> Option<crate::dialogs::Surface> {
    use crate::dialogs::{Button, DialogId, Field, NativePiece, Surface, WindowId};
    let message = copy::sign_in(flow.signin.step, false, "Google", &flow.email)?;
    let button = |id: &'static str, label: &'static str, default: bool| Button {
        id,
        label,
        default,
        destructive: false,
        enabled: true,
    };
    let field = |id: &str, label: &str, value: &str, placeholder: &'static str| Field {
        placeholder: Some(placeholder),
        ..Field::new(id, label, NativePiece::TextField, vec![value.into()])
    };
    let (fields, buttons) = match flow.signin.step {
        SignInStep::Start => (
            vec![field("email", "Email", &flow.email, "name@example.com")],
            vec![
                Button::cancel(false),
                button("google", "Continue with Google", false),
                button("apply", "Continue with Email", true),
            ],
        ),
        SignInStep::Browser => (vec![], vec![Button::cancel(true)]),
        SignInStep::Code => (
            vec![field("code", "Code", "", "6-digit code")],
            vec![
                button("back", "Back", false),
                button("resend", "Resend Code", false),
                button("apply", "Continue", true),
            ],
        ),
        SignInStep::Handle => (
            vec![field("handle", "Handle", &flow.handle, "your-name")],
            vec![
                button("cancel", "Not Now", false),
                button("apply", "Choose Handle", true),
            ],
        ),
        SignInStep::Complete | SignInStep::Closed => return None,
    };
    let mut surface = Surface::sheet(
        DialogId("cloud-sign-in".into()),
        WindowId("cloud".into()),
        &message.title,
        fields,
        "Continue",
    );
    surface.line = Some(message.line);
    surface.buttons = buttons;
    surface.warning = flow.error.clone();
    Some(surface)
}
/// Shows the flow's current step, replacing the sheet of the last one.
fn present(cx: &mut App) {
    let Some(flow) = cx.global_mut::<Cloud>().flow.as_mut() else {
        return;
    };
    if let Some(sheet) = flow.sheet.take() {
        sheet.close();
    }
    let Some(surface) = surface(flow) else {
        finish(cx);
        return;
    };
    let window = flow.window.clone();
    let tokens = pm_shell::tokens();
    let open = OpenSheet::present(
        from_surface(&surface, &tokens),
        &window,
        surface.first_selected.as_deref(),
    );
    let actions = open.actions.clone();
    if let Some(flow) = cx.global_mut::<Cloud>().flow.as_mut() {
        flow.sheet = Some(open);
    }
    cx.spawn(async move |cx| {
        // Buttons only: a field also sends its id when editing ends. The
        // loop ends when this step's sheet is replaced or closed.
        while let Ok(action) = actions.recv().await {
            if ["cancel", "apply", "google", "back", "resend"].contains(&action.as_str()) {
                cx.update(|cx| act(&action, cx));
            }
        }
    })
    .detach();
}
fn finish(cx: &mut App) {
    let cloud = cx.global_mut::<Cloud>();
    if let Some(flow) = cloud.flow.take()
        && let Some(sheet) = flow.sheet
    {
        sheet.close();
    }
    cloud.browser.cancel();
    changed(cx);
}
/// A button of the current step.
fn act(action: &str, cx: &mut App) {
    let Some(flow) = cx.global_mut::<Cloud>().flow.as_mut() else {
        return;
    };
    let text = |flow: &Flow, id: &str| {
        flow.sheet
            .as_ref()
            .map(|s| s.text(id).trim().to_owned())
            .unwrap_or_default()
    };
    match (flow.signin.step, action) {
        (_, "cancel") => {
            flow.signin.close();
            finish(cx);
        }
        (SignInStep::Code, "back") => {
            flow.signin.back();
            flow.error = None;
            present(cx);
        }
        (SignInStep::Start, "google") => {
            flow.email = text(flow, "email");
            browser(cx);
        }
        (SignInStep::Start, "apply") | (SignInStep::Code, "resend") => {
            if flow.signin.step == SignInStep::Start {
                flow.email = text(flow, "email");
            }
            let email = flow.email.clone();
            if !email.contains('@') || email.len() > 254 {
                flow.error = Some(copy::auth_error(AuthError::EmailRejected, "", ""));
                present(cx);
                return;
            }
            submit(CloudWork::SendEmail { email }, Step::Send, cx);
        }
        (SignInStep::Code, "apply") => {
            let code = otp(&text(flow, "code"));
            if code.len() != 6 {
                flow.error = Some(copy::auth_error(AuthError::WrongCode, "", ""));
                present(cx);
                return;
            }
            let email = flow.email.clone();
            submit(
                CloudWork::VerifyEmail {
                    email,
                    code: Secret::new(code),
                },
                Step::Arrive,
                cx,
            );
        }
        (SignInStep::Handle, "apply") => {
            let handle = text(flow, "handle").trim_start_matches('@').to_lowercase();
            flow.handle = handle.clone();
            if !valid_handle(&handle) {
                flow.error = Some(copy::auth_error(AuthError::InvalidHandle, "", &handle));
                present(cx);
                return;
            }
            submit(CloudWork::ClaimHandle { handle }, Step::Arrive, cx);
        }
        _ => {}
    }
}
/// Continue with Google: the registry's page opens in the browser and comes
/// back through `powermove://auth`.
fn browser(cx: &mut App) {
    let mut verifier = [0u8; 32];
    let mut state = [0u8; 16];
    if !random(&mut verifier) || !random(&mut state) {
        return;
    }
    let cloud = cx.global_mut::<Cloud>();
    let origin = cloud.origin;
    let Some(url) = cloud.browser.begin(origin, verifier, state, now_ms()) else {
        return;
    };
    let profile = cloud.profile.clone();
    let Some(flow) = cloud.flow.as_mut() else {
        return;
    };
    let Some(ticket) = flow.signin.begin(SignInStep::Browser) else {
        return;
    };
    // Nothing to wait for: the return link continues the flow.
    let _ = ticket;
    flow.error = None;
    // A development build reading a fixture Store never opens a browser over
    // the person's work: the review plays the browser from this file.
    if store_install::is_dev() {
        let _ = std::fs::write(profile.join("dev-sign-in-url.txt"), url.as_str());
    } else if !crate::platform::open_url(url.as_str()) {
        flow.error = Some(copy::auth_error(AuthError::Browser, "Google", ""));
    }
    present(cx);
}
fn random(bytes: &mut [u8]) -> bool {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(bytes))
        .is_ok()
}
/// A `powermove://auth` return link (entrypoints). Only a sign-in waiting in
/// its browser step takes it; anything else stays an Error Log line.
pub fn receive_link(state: &str, token: &str, cx: &mut App) -> bool {
    if !cx.has_global::<Cloud>() {
        return false;
    }
    let cloud = cx.global_mut::<Cloud>();
    let waiting = cloud
        .flow
        .as_ref()
        .is_some_and(|f| f.signin.step == SignInStep::Browser);
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
    submit(CloudWork::Exchange(exchange), Step::Arrive, cx);
    true
}
#[derive(Clone, Copy)]
enum Step {
    Send,
    Arrive,
    Settings,
}
/// Runs one account action on the worker and applies its reply.
fn submit(work: CloudWork, step: Step, cx: &mut App) {
    let cloud = cx.global_mut::<Cloud>();
    let ticket = match cloud.flow.as_mut() {
        Some(flow) if !matches!(step, Step::Settings) => {
            // The browser step already holds its flight; the exchange reuses
            // a fresh one so a late reply of an older attempt is refused.
            if flow.signin.step == SignInStep::Browser {
                flow.signin.back();
                flow.signin.step = SignInStep::Browser;
            }
            flow.signin.begin(SignInStep::Code)
        }
        _ => Some(pm_services::surfaces::Flight::default().start()),
    };
    let Some(ticket) = ticket else {
        return;
    };
    let Some(task) = CloudTask::new(ticket, cloud.origin, work) else {
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
        let _ = cx.update(|cx| complete(done, step, cx));
    })
    .detach();
}
fn complete(done: pm_services::surfaces::agent_auth::CloudCompletion, step: Step, cx: &mut App) {
    let cloud = cx.global_mut::<Cloud>();
    cloud.busy = false;
    let error = done.result.as_ref().err().map(|e| e.to_string());
    match (&done.result, cloud.flow.as_mut()) {
        (Ok(CloudReply::Account(account)), _) => {
            cloud.signed_in = account.is_some();
            cloud.account = account.clone();
        }
        (Ok(CloudReply::SignedOut), _) => {
            cloud.signed_in = false;
            cloud.account = None;
        }
        _ => {}
    }
    if let Some(flow) = cloud.flow.as_mut()
        && !matches!(step, Step::Settings)
    {
        let applied = match (step, done.result) {
            (Step::Send, result) => flow.signin.sent(
                done.ticket,
                now_ms(),
                result.map(drop).map_err(|e| e.to_string()),
            ),
            (_, Ok(CloudReply::Account(Some(account)))) => {
                flow.signin.arrived(done.ticket, Ok(account))
            }
            (_, result) => flow.signin.arrived(
                done.ticket,
                Err(result.err().map(|e| e.to_string()).unwrap_or_default()),
            ),
        };
        if applied {
            flow.error = error.map(|code| explain(&code, flow));
            present(cx);
        }
    } else if let Some(code) = error {
        crate::alert::note(&explain_settings(&code), "");
    }
    changed(cx);
}
/// The sheet's sentence for a failure, by step (v1.2.0 SignInSheet).
fn explain(code: &str, flow: &Flow) -> String {
    let error = match (flow.signin.step, code) {
        (_, "api:rate_limited") => AuthError::RateLimited,
        (_, "api:client_too_old") => AuthError::OldClient,
        (SignInStep::Start, "network") => AuthError::Offline,
        (SignInStep::Start, _) => AuthError::EmailRejected,
        (SignInStep::Code, "api:unauthorized" | "api:bad_request") => AuthError::WrongCode,
        (SignInStep::Code, "api:gone") => AuthError::ExpiredCode,
        (SignInStep::Code, _) => AuthError::CheckCode,
        (SignInStep::Handle, "api:handle_taken") => AuthError::HandleTaken,
        (SignInStep::Handle, "api:handle_reserved") => AuthError::HandleReserved,
        (SignInStep::Handle, "api:handle_invalid") => AuthError::InvalidHandle,
        (SignInStep::Handle, _) => AuthError::SaveHandle,
        _ => AuthError::ReturnLink,
    };
    copy::auth_error(error, "Google", &flow.handle)
}
fn explain_settings(code: &str) -> String {
    if code == "network" {
        "Can’t reach Powermove Cloud. Check your connection and try again.".into()
    } else {
        "Your account couldn’t be updated. Try again.".into()
    }
}

/// Settings › Accounts › Sign Out.
pub fn sign_out(cx: &mut App) {
    if cx.has_global::<Cloud>() {
        submit(CloudWork::SignOut, Step::Settings, cx);
    }
}
/// Settings › Accounts › Remember Installs on This Account.
pub fn set_remember(remember: bool, cx: &mut App) {
    if cx.has_global::<Cloud>() {
        submit(CloudWork::RememberInstalls(remember), Step::Settings, cx);
    }
}
/// Settings › Accounts › Delete Account…, after its confirmation.
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
        let _ = cx.update(|cx| {
            let pending = cx
                .global::<Cloud>()
                .worker
                .submit(|host| host.delete_account());
            let Ok(pending) = pending else {
                return;
            };
            cx.spawn(async move |cx| {
                let result = wait(pending, cx).await;
                let _ = cx.update(|cx| {
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
        Box::new(Keychain::for_app()),
        profile.to_owned(),
    );
    let remember = cached(profile, store_install::origin()).is_some_and(|a| a.remember_installs);
    if remember && http.signed_in() {
        let _ = http.record_install(repo, release);
    }
}
