//! Settings › Providers against local fakes (tests/fakes): Codex's ChatGPT
//! sign-in or API key fills Codex's own Keychain item, Codex alone refreshes
//! and two Codex processes never spend a refresh token twice; Claude Code's
//! Anthropic key lists its models; Aero runs only on the Powermove account's
//! session (Powermove inference), never on those keys or a custom endpoint.
//! Every Keychain item lives in the run's fake Keychain file (no real
//! keychain, no `security`, no dialog), deleted at the end. Nothing here
//! reaches the network.
use pm_agents::account::{Accounts, State};
use pm_agents::edge::{FakeKeychain, Loopback, NetRequest, Reroute, fetch};
use pm_agents::inference::{PowermoveAccount, Session, Standing};
use pm_agents::launch::Environment;
use pm_agents::{Provider, aero, codex_auth, keys};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A custom endpoint and the key saved for it.
#[derive(Debug)]
struct Custom(String, String);
impl pm_agents::compatible::ChatHost for Custom {
    fn saved(&self) -> Option<pm_agents::compatible::Saved> {
        Some(pm_agents::compatible::Saved {
            base_url: self.0.clone(),
            model: "custom-fake-model".into(),
            vision: false,
            models: vec![],
        })
    }
    fn key(&self, _: &str) -> Option<String> {
        Some(self.1.clone())
    }
    fn stream(
        &self,
        _: &str,
        _: Option<&str>,
        _: Vec<u8>,
        _: &std::sync::atomic::AtomicBool,
        _: &mut dyn FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<u16, String> {
        Err("unused".into())
    }
}

struct Fakes {
    server: Child,
    base: String,
    env: Environment,
    keychain: PathBuf,
}
impl Drop for Fakes {
    fn drop(&mut self) {
        let _ = self.server.kill();
        let _ = self.server.wait();
        let _ = std::fs::remove_file(&self.keychain);
        let _ = std::fs::remove_file(self.keychain.with_extension("json.lock"));
        let _ = std::fs::remove_dir_all(&self.env.profile);
    }
}
/// This run's fake Keychain (PM_TEST_KEYCHAIN_DIR, else the temp folder): a
/// JSON file the fake Codex and the code under test share, never a real
/// keychain, so nothing here can show a dialog.
fn test_keychain(port: u16) -> PathBuf {
    let dir =
        std::env::var_os("PM_TEST_KEYCHAIN_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    dir.join(format!("pm-providers-{}-{port}.json", std::process::id()))
}
/// The tests share process-wide state (the OpenAI identity generation, the
/// refresh broker): one at a time.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}
fn fakes(ttl: u64) -> Fakes {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fakes");
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let keychain = test_keychain(port);
    let base = format!("http://127.0.0.1:{port}");
    let profile = std::env::temp_dir().join(format!("pm-providers-{}-{port}", std::process::id()));
    std::fs::create_dir_all(profile.join("home")).unwrap();
    let log = std::fs::File::create(profile.join("fake-backends.log")).unwrap();
    let server = Command::new("python3")
        .arg(dir.join("fake_backends.py"))
        .args([port.to_string(), ttl.to_string()])
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let vars = BTreeMap::from([
        ("PATH".to_owned(), std::env::var("PATH").unwrap_or_default()),
        // Never the person's own Codex home or login.
        (
            "HOME".to_owned(),
            profile.join("home").display().to_string(),
        ),
        (
            "PM_CODEX_KEYRING_SERVICE".to_owned(),
            format!("pm-aero-picker-test Codex Auth {}", std::process::id()),
        ),
        ("PM_FAKE_AUTH".to_owned(), format!("{base}/auth")),
        (
            "CODEX_BINARY".to_owned(),
            dir.join("fake_codex.py").display().to_string(),
        ),
        ("PM_AGENT_FAKE_BROWSER".to_owned(), "1".to_owned()),
        (
            "PM_FAKE_CODEX_LOG".to_owned(),
            profile.join("fake-codex.log").display().to_string(),
        ),
        (
            "PM_AGENT_FAKE_KEYCHAIN".to_owned(),
            keychain.display().to_string(),
        ),
        // Aero's base: the fakes' Powermove inference (development builds).
        (
            pm_agents::inference::TEST_ENDPOINT.to_owned(),
            format!("{base}/aero/v1/aero"),
        ),
        (
            "PM_AGENT_FAKES".to_owned(),
            format!(
                "https://chatgpt.com={base}/chatgpt https://api.openai.com={base}/openai https://api.anthropic.com={base}/anthropic https://auth.openai.com={base}/auth"
            ),
        ),
    ]);
    let mut env = Environment {
        profile: profile.clone(),
        app_version: "test".into(),
        executable: "/bin/false".into(),
        keychain_prefix: format!("pm-aero-picker-test-{}", std::process::id()),
        secrets: Some(Arc::new(FakeKeychain(keychain.clone()))),
        compatible: Some(Arc::new(Custom(
            format!("{base}/custom/v1"),
            "custom-fake-key".into(),
        ))),
        // The development switch: Providers, and Codex's login in the Keychain.
        aero: Some(aero::Launcher(Arc::new(|_, _, _| Err("unused".into())))),
        powermove: Some(Arc::new(Signed::new(AERO_SESSION))),
        ..Environment::default()
    };
    env.net = Some(Reroute::from_env(&vars, Arc::new(Loopback)));
    env.vars = vars;
    let deadline = Instant::now() + Duration::from_secs(10);
    while fetch(
        &Loopback,
        &NetRequest {
            method: "GET",
            url: &format!("{base}/stats"),
            headers: &[],
            body: b"",
        },
    )
    .is_err()
    {
        assert!(Instant::now() < deadline, "the fake backends did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    Fakes {
        server,
        base,
        env,
        keychain,
    }
}
fn stats(base: &str) -> Value {
    let (_, body) = fetch(
        &Loopback,
        &NetRequest {
            method: "GET",
            url: &format!("{base}/stats"),
            headers: &[],
            body: b"",
        },
    )
    .unwrap();
    serde_json::from_slice(&body).unwrap()
}
/// Sets the fake server's test knobs (`/control`).
fn control(base: &str, knobs: Value) {
    let body = knobs.to_string();
    let request = NetRequest {
        method: "POST",
        url: &format!("{base}/control"),
        headers: &[("Content-Type".to_owned(), "application/json".to_owned())],
        body: body.as_bytes(),
    };
    fetch(&Loopback, &request).unwrap();
}
/// A Codex run's own App Server, as a Codex-harness run starts it (same
/// environment), asking Codex to refresh. Its replies arrive on the receiver.
fn codex_run_refreshing(env: &Environment) -> (Child, std::sync::mpsc::Receiver<String>) {
    use std::io::{BufRead as _, Write as _};
    let binary = PathBuf::from(&env.vars["CODEX_BINARY"]);
    let mut child = Command::new(binary)
        .arg("app-server")
        .env_clear()
        .envs(env.codex_vars().unwrap())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, r#"{{"method":"initialize","id":1,"params":{{}}}}"#).unwrap();
    writeln!(
        stdin,
        r#"{{"method":"account/read","id":2,"params":{{"refreshToken":true}}}}"#
    )
    .unwrap();
    std::mem::forget(stdin);
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            let _ = tx.send(line);
        }
    });
    (child, rx)
}
/// The person's browser on the fake issuer: follow its redirect to Codex's loopback callback.
fn browser() -> Arc<pm_agents::account::Opener> {
    Arc::new(|url: &str| {
        let url = url.to_owned();
        std::thread::spawn(move || {
            let (_, body) = fetch(
                &Loopback,
                &NetRequest {
                    method: "GET",
                    url: &url,
                    headers: &[],
                    body: b"",
                },
            )
            .unwrap();
            let next = String::from_utf8(body)
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .to_owned();
            let _ = fetch(
                &Loopback,
                &NetRequest {
                    method: "GET",
                    url: &next,
                    headers: &[],
                    body: b"",
                },
            );
        });
        Ok(())
    })
}
fn wait(accounts: &Accounts, provider: Provider, state: State) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while accounts.status(provider).state != state {
        assert!(
            Instant::now() < deadline,
            "waiting for {state:?}: {:?}",
            accounts.status(provider)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
fn until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}
/// The person's Powermove session in these tests: never sent to the fakes,
/// which stand in for Aero through the test endpoint and are sent the
/// development credential instead.
const AERO_SESSION: &str = "pm-session-real-secret";
const DEV: &str = pm_agents::inference::DEVELOPMENT_CREDENTIAL;
/// The Powermove account: signed in with a session, which a test may
/// replace (another account) or take away (a sign-out).
#[derive(Debug)]
struct Signed(std::sync::Mutex<Option<String>>);
impl Signed {
    fn new(token: &str) -> Self {
        Self(std::sync::Mutex::new(Some(token.into())))
    }
    fn set(&self, token: Option<&str>) {
        *self.0.lock().unwrap() = token.map(str::to_owned);
    }
}
impl PowermoveAccount for Signed {
    fn session(&self) -> Option<Session> {
        Some(Session {
            origin: "https://cloud.trypowermove.com".into(),
            token: self.0.lock().unwrap().clone()?,
        })
    }
}
/// One Aero core turn on `model` (no tools: the evaluator path).
fn powermove_says(env: &Environment, model: &str, effort: &str) -> Result<String, String> {
    let live = pm_agents::inference::live(env, model, effort)?;
    let scope = env
        .profile
        .join(format!("turn-{}", model.replace('/', "-")));
    std::fs::create_dir_all(&scope).unwrap();
    aero::evaluate(&aero::Model::live(live), &scope, "Say hello.", &[])
}

#[test]
fn chatgpt_sign_in_is_codexs_own_item_and_signed_out_from_it() {
    let _serial = serial();
    if !pm_aero_core::available() {
        return;
    }
    let f = fakes(120);
    let env = &f.env;
    let accounts = Accounts::new(env.clone(), browser(), Arc::new(|| {}));
    accounts.refresh(Provider::Chatgpt);
    wait(&accounts, Provider::Chatgpt, State::Disconnected);

    // 1. Sign in to OpenAI with ChatGPT (Codex's route, the fake issuer).
    accounts.connect(Provider::Chatgpt);
    wait(&accounts, Provider::Chatgpt, State::Connected);
    let status = accounts.status(Provider::Chatgpt);
    assert_eq!(status.email.as_deref(), Some("owner@example.com"));
    assert!(status.key.is_none());
    until("Codex's models", || {
        !accounts.models(Provider::Chatgpt).is_empty()
    });
    // 2. One Keychain item: Codex wrote it, Settings reads the same one; no auth.json.
    let auth = codex_auth::read(f.env.secrets.as_deref().unwrap(), env)
        .unwrap()
        .expect("the shared item");
    assert_eq!(
        auth.chatgpt.as_ref().unwrap().1.as_deref(),
        Some("acct-fake-1")
    );
    assert!(!codex_auth::codex_home(env).join("auth.json").exists());
    let config = std::fs::read_to_string(codex_auth::codex_home(env).join("config.toml")).unwrap();
    assert!(
        config.contains("cli_auth_credentials_store = \"keyring\""),
        "{config}"
    );

    // 3. Sign out: the item is gone.
    accounts.disconnect(Provider::Chatgpt);
    wait(&accounts, Provider::Chatgpt, State::Disconnected);
    assert!(
        codex_auth::read(f.env.secrets.as_deref().unwrap(), env)
            .unwrap()
            .is_none()
    );
    assert!(accounts.models(Provider::Chatgpt).is_empty());
    accounts.close();
}

#[test]
fn an_openai_key_is_codexs_and_an_anthropic_key_lists_claude_codes_models() {
    let _serial = serial();
    let f = fakes(3600);
    let env = &f.env;
    let accounts = Accounts::new(env.clone(), browser(), Arc::new(|| {}));

    // OpenAI API key: Codex saves it in its item; Settings shows it masked.
    accounts.connect_openai_key("sk-fake-openai-123456".into());
    wait(&accounts, Provider::Chatgpt, State::Connected);
    assert_eq!(
        accounts.status(Provider::Chatgpt).key.as_deref(),
        Some("sk-…3456")
    );
    assert_eq!(
        codex_auth::read(f.env.secrets.as_deref().unwrap(), env)
            .unwrap()
            .unwrap()
            .api_key
            .as_deref(),
        Some("sk-fake-openai-123456")
    );
    accounts.disconnect(Provider::Chatgpt);
    wait(&accounts, Provider::Chatgpt, State::Disconnected);

    // Anthropic API key: the Keychain and its model list, for Claude Code.
    assert!(accounts.set_anthropic_key("not a key".into()).is_err());
    accounts
        .set_anthropic_key("sk-ant-fake-abcdef12".into())
        .unwrap();
    until("the Anthropic models", || {
        !accounts.anthropic().models.is_empty()
    });
    assert_eq!(accounts.anthropic().masked.as_deref(), Some("sk-…ef12"));
    accounts.remove_anthropic_key().unwrap();
    until("the key's removal", || {
        accounts.anthropic().masked.is_none()
    });
    accounts.close();
}

/// Aero on the Powermove account: its standing follows the backend's usage
/// answer (a plan with its catalogue, none, an offer, no route yet), and a
/// turn runs on the session only, never on Codex's OpenAI key, Claude Code's
/// Anthropic key or the saved custom endpoint, which are all there.
#[test]
fn aero_runs_only_on_the_powermove_session_and_follows_its_plan() {
    let _serial = serial();
    if !pm_aero_core::available() {
        return;
    }
    let f = fakes(3600);
    let env = &f.env;
    let accounts = Accounts::new(env.clone(), browser(), Arc::new(|| {}));
    accounts.connect_openai_key("sk-fake-openai-123456".into());
    wait(&accounts, Provider::Chatgpt, State::Connected);
    accounts
        .set_anthropic_key("sk-ant-fake-abcdef12".into())
        .unwrap();
    until("the Anthropic models", || {
        !accounts.anthropic().models.is_empty()
    });

    let Standing::Ready { usage, models } = pm_agents::inference::standing(env) else {
        panic!("a plan: {:?}", pm_agents::inference::standing(env));
    };
    assert_eq!(usage.plan.name, "Aerolite");
    assert_eq!(usage.facts()[0].1, "812 of 1,000 left");
    assert_eq!(models[0].id, "pm/anthropic/claude-opus-5.5");
    assert_eq!(models[0].default_effort.as_deref(), Some("high"));
    accounts.refresh_aero();
    until("Aero's standing", || {
        matches!(accounts.aero(), Standing::Ready { .. })
    });

    let before = stats(&f.base)["requests"].as_array().unwrap().len();
    // Old route ids (an OpenAI key's, a Custom endpoint's) never run on Aero.
    for model in ["openai/gpt-fake-key", "custom/custom-fake-model"] {
        assert!(powermove_says(env, model, "high").is_err(), "{model}");
    }
    for model in ["pm/anthropic/claude-opus-5.5", "pm/openai/gpt-6-sol"] {
        let text = powermove_says(env, model, "high").unwrap_or_else(|e| panic!("{model}: {e}"));
        assert!(text.contains("Hello from the fake provider"), "{text}");
    }
    let seen = stats(&f.base);
    let turns: Vec<&Value> = seen["requests"].as_array().unwrap()[before..]
        .iter()
        .filter(|r| !matches!(r["path"].as_str(), Some("/stats" | "/control")))
        .collect();
    assert!(!turns.is_empty());
    for request in &turns {
        assert_eq!(
            request["path"], "/aero/v1/aero/chat/completions",
            "{request}"
        );
        assert_eq!(request["authorization"], "Bearer pm-aero-d", "{request}");
        assert_eq!(request["x-api-key"], false, "{request}");
    }
    for sent in seen["raw"].as_array().unwrap() {
        let sent = sent.as_str().unwrap();
        if sent.starts_with("POST /aero/") {
            for key in [
                "sk-fake-openai-123456",
                "sk-ant-fake-abcdef12",
                "custom-fake-key",
                AERO_SESSION,
            ] {
                assert!(!sent.contains(key), "{key} reached Aero's request");
            }
        }
    }

    // No plan, an offer, no route yet: honest, no models; signed out: none.
    control(&f.base, serde_json::json!({"aero": "none"}));
    assert_eq!(pm_agents::inference::standing(env), Standing::NoPlan(None));
    control(&f.base, serde_json::json!({"aero": "offer"}));
    assert!(matches!(
        pm_agents::inference::standing(env),
        Standing::NoPlan(Some(offer)) if offer.label == "Get Aerolite"
    ));
    control(&f.base, serde_json::json!({"aero": "missing"}));
    assert_eq!(pm_agents::inference::standing(env), Standing::NoPlan(None));
    // Out of credits: the turn ends in Aero's words.
    control(&f.base, serde_json::json!({"aero": "out"}));
    let model = aero::Model::live(
        pm_agents::inference::live(env, "pm/anthropic/claude-opus-5.5", "").unwrap(),
    );
    let scope = env.profile.join("turn-spent");
    std::fs::create_dir_all(&scope).unwrap();
    assert!(aero::evaluate(&model, &scope, "Say hello.", &[]).is_err());
    let spent = model.take_failure().unwrap_or_default();
    assert!(spent.contains("out of Aero credits"), "{spent}");
    control(&f.base, serde_json::json!({"aero": "plan"}));
    let signed_out = Environment {
        powermove: None,
        ..env.clone()
    };
    assert_eq!(
        pm_agents::inference::standing(&signed_out),
        Standing::SignedOut
    );
    assert!(powermove_says(&signed_out, "pm/anthropic/claude-opus-5.5", "").is_err());
    accounts.close();
}

// ---- Fix round 1: one regression test per P1 (Sol's review) ----

fn read_store(f: &Fakes) -> String {
    std::fs::read_to_string(&f.keychain).unwrap_or_default()
}
fn signed_in(f: &Fakes) -> Arc<Accounts> {
    let accounts = Accounts::new(f.env.clone(), browser(), Arc::new(|| {}));
    accounts.refresh(Provider::Chatgpt);
    wait(&accounts, Provider::Chatgpt, State::Disconnected);
    accounts.connect(Provider::Chatgpt);
    wait(&accounts, Provider::Chatgpt, State::Connected);
    accounts
}
/// Every file under `dir`, read whole.
fn every_file(dir: &std::path::Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = vec![];
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(every_file(&path));
        } else if let Ok(bytes) = std::fs::read(&path) {
            out.push((path, bytes));
        }
    }
    out
}

/// Item 1 (the Chief's sentinel test): with the switch on, the person's own
/// `~/.codex/auth.json` is never opened and its tokens are never used,
/// across launch, sign-in, a Codex refresh and sign-out.
#[test]
fn with_the_switch_on_the_owners_codex_login_is_never_opened_or_used() {
    let _serial = serial();
    use std::os::unix::fs::PermissionsExt as _;
    if !pm_aero_core::available() {
        return;
    }
    let f = fakes(120);
    let env = &f.env;
    let codex = env.profile.join("home/.codex");
    std::fs::create_dir_all(&codex).unwrap();
    let sentinels = [
        "SENTINEL-ACCESS-9f2c1",
        "SENTINEL-REFRESH-7b4e0",
        "SENTINEL-ID-3d81a",
        "sk-SENTINEL-KEY-55aa",
    ];
    let owner = codex.join("auth.json");
    std::fs::write(
        &owner,
        serde_json::json!({"auth_mode":"chatgpt","OPENAI_API_KEY":sentinels[3],"tokens":{
            "access_token":sentinels[0],"refresh_token":sentinels[1],"id_token":sentinels[2],"account_id":"acct-owner"}})
        .to_string(),
    )
    .unwrap();
    // Unreadable: any open of it fails (a FIFO would hang the test instead).
    std::fs::set_permissions(&owner, std::fs::Permissions::from_mode(0o000)).unwrap();
    let before = std::fs::metadata(&owner).unwrap();

    // Launch, sign-in, a Codex run's refresh, sign-out.
    env.codex_vars()
        .expect("launch never opens the owner's login");
    let accounts = signed_in(&f);
    let (mut run, replies) = codex_run_refreshing(env);
    until("the Codex run's refresh", || {
        replies.try_iter().any(|line| line.contains(r#""id": 2"#))
    });
    let _ = run.kill();
    assert!(stats(&f.base)["refreshes"].as_u64().unwrap() >= 1);
    accounts.disconnect(Provider::Chatgpt);
    wait(&accounts, Provider::Chatgpt, State::Disconnected);

    let after = std::fs::metadata(&owner).unwrap();
    assert_eq!(
        after.permissions().mode() & 0o777,
        0,
        "the owner's file was changed"
    );
    assert_eq!(after.modified().unwrap(), before.modified().unwrap());
    assert_eq!(after.len(), before.len());
    std::fs::set_permissions(&owner, std::fs::Permissions::from_mode(0o600)).unwrap();
    let raw = stats(&f.base)["raw"].to_string();
    let logs = ["fake-backends.log", "fake-codex.log"]
        .map(|name| std::fs::read_to_string(env.profile.join(name)).unwrap_or_default())
        .join("\n");
    let runtime = every_file(&codex_auth::codex_home(env));
    for sentinel in sentinels {
        assert!(
            !read_store(&f).contains(sentinel),
            "{sentinel} reached the fake Keychain"
        );
        assert!(!raw.contains(sentinel), "{sentinel} reached a fake server");
        assert!(!logs.contains(sentinel), "{sentinel} reached a log");
        for (path, bytes) in &runtime {
            assert!(
                !String::from_utf8_lossy(bytes).contains(sentinel),
                "{sentinel} reached {}",
                path.display()
            );
        }
    }
    accounts.close();
}

fn switched_off(env: &Environment) -> Environment {
    Environment {
        aero: None,
        ..env.clone()
    }
}

/// Item 2: a file login this profile already had is parked, never moved into
/// the Keychain or deleted, and is back as it was when the switch is off.
#[test]
fn a_profile_file_login_is_parked_unchanged_and_back_when_the_switch_is_off() {
    let _serial = serial();
    let f = fakes(3600);
    let home = codex_auth::codex_home(&f.env);
    std::fs::create_dir_all(&home).unwrap();
    let login = r#"{"auth_mode":"chatgpt","tokens":{"access_token":"file-a","refresh_token":"file-r","id_token":"x","account_id":"acct-file"}}"#;
    std::fs::write(home.join("auth.json"), login).unwrap();
    f.env.codex_vars().unwrap();
    assert!(
        !home.join("auth.json").exists(),
        "Codex's Keychain store would delete it"
    );
    assert!(
        codex_auth::read(f.env.secrets.as_deref().unwrap(), &f.env)
            .unwrap()
            .is_none(),
        "nothing is migrated"
    );
    assert!(!read_store(&f).contains("file-r"));
    switched_off(&f.env).codex_vars().unwrap();
    assert_eq!(
        std::fs::read_to_string(home.join("auth.json")).unwrap(),
        login
    );
    let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(
        config.contains("cli_auth_credentials_store = \"file\""),
        "{config}"
    );
}

/// Item 2: an existing (here unreadable) store item is never overwritten or
/// trusted, and the file login still comes back.
#[test]
fn an_existing_store_item_is_never_overwritten_and_the_file_login_still_comes_back() {
    let _serial = serial();
    let f = fakes(3600);
    let home = codex_auth::codex_home(&f.env);
    std::fs::create_dir_all(&home).unwrap();
    let secrets = f.env.secrets.as_deref().unwrap();
    let key = codex_auth::store_key(&home);
    secrets
        .write(&codex_auth::service(&f.env), &key, b"not a login")
        .unwrap();
    std::fs::write(
        home.join("auth.json"),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-file"}"#,
    )
    .unwrap();
    f.env.codex_vars().unwrap();
    assert_eq!(
        secrets
            .read(&codex_auth::service(&f.env), &key)
            .unwrap()
            .unwrap(),
        b"not a login"
    );
    switched_off(&f.env).codex_vars().unwrap();
    assert!(
        std::fs::read_to_string(home.join("auth.json"))
            .unwrap()
            .contains("sk-file")
    );
}

/// Item 2: a file-store process that writes its login while the switch is on
/// (the concurrent write) wins: that login is the one that comes back.
#[test]
fn a_file_login_written_while_the_switch_is_on_is_the_one_that_comes_back() {
    let _serial = serial();
    let f = fakes(3600);
    let home = codex_auth::codex_home(&f.env);
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("auth.json"), r#"{"OPENAI_API_KEY":"sk-older"}"#).unwrap();
    f.env.codex_vars().unwrap();
    std::fs::write(home.join("auth.json"), r#"{"OPENAI_API_KEY":"sk-newer"}"#).unwrap();
    // Launches race the writer: every store change holds the store lock.
    let launches: Vec<_> = (0..6)
        .map(|_| {
            let env = f.env.clone();
            std::thread::spawn(move || env.codex_vars().map(|_| ()))
        })
        .collect();
    for launch in launches {
        launch.join().unwrap().unwrap();
    }
    switched_off(&f.env).codex_vars().unwrap();
    assert!(
        std::fs::read_to_string(home.join("auth.json"))
            .unwrap()
            .contains("sk-newer")
    );
}

/// Item 3: a Codex run's App Server and a Powermove refresh ask at the same
/// moment (both read the same refresh token; the fake issuer holds the first
/// exchange until a second arrives): the token is spent once.
#[test]
fn two_codex_runs_never_spend_the_same_refresh_token() {
    let _serial = serial();
    let f = fakes(120);
    let accounts = signed_in(&f);
    control(&f.base, serde_json::json!({"refresh_barrier": 2}));
    let before = stats(&f.base)["refreshes"].as_u64().unwrap();
    let (mut one, first) = codex_run_refreshing(&f.env);
    let (mut two, second) = codex_run_refreshing(&f.env);
    let answered = |replies: &std::sync::mpsc::Receiver<String>| {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !replies.try_iter().any(|line| line.contains(r#""id": 2"#)) {
            assert!(
                Instant::now() < deadline,
                "a Codex run's refresh never answered"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    answered(&first);
    answered(&second);
    let _ = one.kill();
    let _ = two.kill();
    let seen = stats(&f.base);
    assert_eq!(seen["reused"], 0, "a refresh token was spent twice: {seen}");
    assert_eq!(seen["refreshes"].as_u64().unwrap(), before + 1, "{seen}");
    control(&f.base, serde_json::json!({"refresh_barrier": 0}));
    accounts.close();
}

/// Item 3: the process that asked for a refresh dies mid-exchange, so the
/// outcome is uncertain to it; the next ask with the same token gets the one
/// exchange's answer instead of spending the token again.
#[test]
fn an_uncertain_refresh_is_answered_once_never_spent_twice() {
    let _serial = serial();
    if !pm_aero_core::available() {
        return;
    }
    let f = fakes(120);
    let accounts = signed_in(&f);
    control(&f.base, serde_json::json!({"refresh_delay": 2}));
    let before = stats(&f.base)["refreshes"].as_u64().unwrap();
    let (mut run, _) = codex_run_refreshing(&f.env);
    std::thread::sleep(Duration::from_millis(800));
    let _ = run.kill();
    let _ = run.wait();
    let (mut again, replies) = codex_run_refreshing(&f.env);
    until("the second run's refresh", || {
        replies.try_iter().any(|line| line.contains(r#""id": 2"#))
    });
    let _ = again.kill();
    let seen = stats(&f.base);
    assert_eq!(seen["reused"], 0, "a refresh token was spent twice: {seen}");
    assert_eq!(seen["refreshes"].as_u64().unwrap(), before + 1, "{seen}");
    control(&f.base, serde_json::json!({"refresh_delay": 0}));
    accounts.close();
}

/// A turn carries on only on the Powermove session it started on: another
/// account, a sign-out or a session the backend refused is never sent on.
#[test]
fn an_aero_turn_never_continues_on_another_powermove_session() {
    let _serial = serial();
    let mut f = fakes(3600);
    let account = Arc::new(Signed::new(AERO_SESSION));
    f.env.powermove = Some(account.clone());
    let live = pm_agents::inference::live(&f.env, "pm/anthropic/claude-opus-5.5", "").unwrap();
    let (origin, first) = (live.credentials)(None).unwrap();
    assert_eq!(origin, f.base);
    assert!(first.contains(&("authorization".into(), format!("Bearer {DEV}"))));
    assert!(
        (live.credentials)(Some(DEV)).is_err(),
        "a refused credential is sent again"
    );
    account.set(Some("pm-session-other"));
    assert!(
        (live.credentials)(None).is_err(),
        "the turn went on under another account"
    );
    account.set(None);
    assert_eq!(
        (live.credentials)(None).err().as_deref(),
        Some(pm_agents::inference::SIGN_IN)
    );
}

/// Item 5: Sign Out succeeds only once the shared item is gone (here Codex
/// answers the logout but leaves the item).
#[test]
fn sign_out_is_reported_only_once_the_shared_sign_in_is_gone() {
    let _serial = serial();
    let mut f = fakes(3600);
    f.env
        .vars
        .insert("PM_FAKE_CODEX_KEEP_ON_LOGOUT".into(), "1".into());
    let accounts = signed_in(&f);
    let generation = codex_auth::generation();
    accounts.disconnect(Provider::Chatgpt);
    until("the sign-out to settle", || {
        accounts.status(Provider::Chatgpt).detail.is_some()
            || accounts.status(Provider::Chatgpt).state == State::Disconnected
    });
    let status = accounts.status(Provider::Chatgpt);
    assert_eq!(
        status.state,
        State::Connected,
        "reported signed out with the item still there: {status:?}"
    );
    assert_eq!(
        codex_auth::generation(),
        generation,
        "live runs were told it was gone"
    );
    accounts.close();
}

/// A store that counts every read, write and delete.
#[derive(Debug)]
struct Counting(FakeKeychain, std::sync::atomic::AtomicUsize);
impl pm_agents::edge::Secrets for Counting {
    fn read(&self, s: &str, a: &str) -> Result<Option<Vec<u8>>, String> {
        self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.0.read(s, a)
    }
    fn write(&self, s: &str, a: &str, b: &[u8]) -> Result<(), String> {
        self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.0.write(s, a, b)
    }
    fn delete(&self, s: &str, a: &str) -> Result<(), String> {
        self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.0.delete(s, a)
    }
}

/// Item 6: with the switch off, a populated store is never read, no key
/// catalog is fetched, and Codex keeps today's file store.
#[test]
fn switch_off_reads_no_provider_credentials_and_keeps_todays_codex_store() {
    let _serial = serial();
    let f = fakes(3600);
    let store = FakeKeychain(f.keychain.clone());
    use pm_agents::edge::Secrets as _;
    store
        .write(
            &keys::anthropic_service(&f.env),
            "api-key",
            b"sk-ant-fake-abcdef12",
        )
        .unwrap();
    store
        .write(
            &codex_auth::service(&f.env),
            &codex_auth::store_key(&codex_auth::codex_home(&f.env)),
            br#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-fake-openai-123456"}"#,
        )
        .unwrap();
    let counting = Arc::new(Counting(
        FakeKeychain(f.keychain.clone()),
        Default::default(),
    ));
    let env = Environment {
        aero: None,
        secrets: Some(counting.clone()),
        ..f.env.clone()
    };
    let accounts = Accounts::new(env.clone(), browser(), Arc::new(|| {}));
    accounts.refresh_anthropic();
    let vars = env.codex_vars().unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        counting.1.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "a credential was read"
    );
    assert!(accounts.anthropic().masked.is_none());
    assert!(!vars.contains_key("CODEX_REFRESH_TOKEN_URL_OVERRIDE"));
    let config = std::fs::read_to_string(codex_auth::codex_home(&env).join("config.toml")).unwrap();
    assert!(
        config.contains("cli_auth_credentials_store = \"file\""),
        "{config}"
    );
    let fetched = stats(&f.base)["requests"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["path"].as_str().is_some_and(|p| p.ends_with("/models")));
    assert!(!fetched, "a key catalog was fetched");
    accounts.close();
}

/// A project session for a Powermove turn with nothing to edit.
struct Quiet;
impl aero::Session for Quiet {
    fn context(&mut self) -> Result<Value, String> {
        Ok(serde_json::json!({"composition": {"revision": 1}, "layers": []}))
    }
    fn call(
        &mut self,
        _: &str,
        _: &str,
        _: &mut dyn FnMut(&str) -> bool,
    ) -> Result<aero::ToolResult, String> {
        Ok(aero::ToolResult::text("ok"))
    }
    fn failures(&mut self) -> Vec<aero::Failure> {
        vec![]
    }
    fn finish(&mut self) -> Result<(), String> {
        Ok(())
    }
}

/// Item 7: a backend error that repeats the request's credential never
/// reaches the core. A whole Aero turn runs on a session the fake echoes in
/// its error: it appears in none of the run's events, its result, or the
/// core's saved session (the journal it writes).
#[test]
fn a_provider_error_that_echoes_the_key_never_reaches_the_core() {
    if !pm_aero_core::available() {
        return;
    }
    let _serial = serial();
    let mut f = fakes(3600);
    f.env.powermove = Some(Arc::new(Signed::new(AERO_SESSION)));
    control(&f.base, serde_json::json!({"aero": "echo"}));
    let live = pm_agents::inference::live(&f.env, "pm/anthropic/claude-opus-5.5", "").unwrap();
    let scope = f.env.profile.join("aero-session");
    std::fs::create_dir_all(&scope).unwrap();
    let handle = aero::start(
        pm_agents::launch::Request {
            run: "echo-run".into(),
            prompt: "Say hello.".into(),
            provider: Provider::Aero,
            model: Some("pm/anthropic/claude-opus-5.5".into()),
            effort: None,
            access: pm_agents::Access::Project,
            approval: Default::default(),
            computer_consent: false,
            workspace: scope.clone(),
            images: vec![],
            role: None,
            files: vec![],
            project: Some("synthetic".into()),
        },
        "echo-session".into(),
        scope.clone(),
        aero::Options {
            model: aero::Model::live(live),
            wall: Duration::from_secs(30),
            skip_gate: true,
            skip_review: true,
            ..aero::Options::development()
        },
        |_| Ok(Box::new(Quiet) as Box<dyn aero::Session>),
    );
    let mut events = String::new();
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        assert!(Instant::now() < deadline, "the turn never ended: {events}");
        match handle.try_event() {
            Ok(event) => {
                let ended = matches!(
                    event,
                    pm_agents::Event::Completed { .. } | pm_agents::Event::Stopped(_)
                );
                events.push_str(&format!("{event:?}\n"));
                if ended {
                    break;
                }
            }
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    assert!(
        stats(&f.base)["raw"].to_string().contains(DEV),
        "the fake never saw the key"
    );
    assert!(
        !events.contains(DEV) && !events.contains(AERO_SESSION),
        "the key reached the run's events: {events}"
    );
    for (path, bytes) in every_file(&scope) {
        assert!(
            !String::from_utf8_lossy(&bytes).contains(DEV),
            "the key reached the core's saved session: {}",
            path.display()
        );
    }
}

/// P1 (Sol, PM-750): the test endpoint is sent the development credential,
/// never the person's Powermove session, on all three of Aero's calls.
#[test]
fn the_test_endpoint_is_sent_a_development_credential_never_the_session() {
    let _serial = serial();
    if !pm_aero_core::available() {
        return;
    }
    let f = fakes(3600);
    assert!(matches!(
        pm_agents::inference::standing(&f.env),
        Standing::Ready { .. }
    ));
    powermove_says(&f.env, "pm/anthropic/claude-opus-5.5", "high").unwrap();
    let raw = stats(&f.base)["raw"].to_string();
    assert!(raw.contains(&format!("Bearer {DEV}")), "the fakes saw no credential");
    assert!(!raw.contains(AERO_SESSION), "the session reached the test endpoint");
}

/// P1 (Sol, PM-750): signing out while Aero's reply streams stops that
/// stream at once, and its reply (and any tool call in it) is discarded.
#[test]
fn signing_out_mid_turn_stops_the_held_stream_and_runs_none_of_its_reply() {
    let _serial = serial();
    if !pm_aero_core::available() {
        return;
    }
    let mut f = fakes(3600);
    let account = Arc::new(Signed::new(AERO_SESSION));
    f.env.powermove = Some(account.clone());
    control(&f.base, serde_json::json!({"hold_seconds": 8}));
    let env = f.env.clone();
    let started = Instant::now();
    let turn = std::thread::spawn(move || powermove_says(&env, "pm/anthropic/claude-opus-5.5", ""));
    until("the held request", || {
        stats(&f.base)["requests"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["path"] == "/aero/v1/aero/chat/completions")
    });
    account.set(None);
    let result = turn.join().unwrap();
    control(&f.base, serde_json::json!({"hold_seconds": 0}));
    assert!(result.is_err(), "the turn carried on after sign-out: {result:?}");
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "it waited for the held reply"
    );
}

/// P1 (Sol, PM-750): Aero's Try Again reads its plan and catalogue again,
/// so a failed read followed by a good one makes Aero ready.
#[test]
fn aeros_try_again_reads_its_plan_and_models_again() {
    let _serial = serial();
    let f = fakes(3600);
    control(&f.base, serde_json::json!({"aero": "error"}));
    let accounts = Accounts::new(f.env.clone(), browser(), Arc::new(|| {}));
    accounts.refresh_aero();
    until("the failed read", || {
        matches!(accounts.aero(), Standing::Unreachable(_))
    });
    control(&f.base, serde_json::json!({"aero": "plan"}));
    // What the panel's Try Again asks for.
    accounts.refresh(Provider::Aero);
    until("the retry", || matches!(accounts.aero(), Standing::Ready { .. }));
    accounts.close();
}
