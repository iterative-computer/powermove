//! Settings › Providers against local fakes (tests/fakes): Codex's ChatGPT
//! sign-in fills the one Keychain item Codex and Powermove read; Codex alone
//! refreshes, concurrent asks never spend a refresh token twice; Powermove's
//! core runs on ChatGPT, an OpenAI key, an Anthropic key and a custom
//! endpoint with credentials added at its fetch edge; signing out takes the
//! credential from both harnesses. Keychain items are lane-only (a test
//! service name with this process id) and deleted at the end. Nothing here
//! reaches the network.
use pm_agents::account::{Accounts, State};
use pm_agents::edge::{Loopback, NetRequest, Reroute, Secrets, fetch};
use pm_agents::launch::Environment;
use pm_agents::{Provider, aero, codex_auth, sources};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The login Keychain through `security`, as the fake Codex writes it.
#[derive(Debug)]
struct Cli;
impl Secrets for Cli {
    fn read(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
        let out = Command::new("security")
            .args(["find-generic-password", "-s", service, "-a", account, "-w"])
            .output()
            .map_err(|e| e.to_string())?;
        Ok(out.status.success().then(|| out.stdout.trim_ascii_end().to_vec()))
    }
    fn write(&self, service: &str, account: &str, bytes: &[u8]) -> Result<(), String> {
        let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        let ok = Command::new("security")
            .args(["add-generic-password", "-U", "-A", "-s", service, "-a", account, "-w", text])
            .output()
            .map_err(|e| e.to_string())?
            .status
            .success();
        ok.then_some(()).ok_or("security add failed".into())
    }
    fn delete(&self, service: &str, account: &str) -> Result<(), String> {
        let _ = Command::new("security").args(["delete-generic-password", "-s", service, "-a", account]).output();
        Ok(())
    }
}

#[derive(Debug)]
struct Custom(String);
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
        Some("custom-fake-key".into())
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
}
impl Drop for Fakes {
    fn drop(&mut self) {
        let _ = self.server.kill();
        let _ = self.server.wait();
        let home = codex_auth::codex_home(&self.env);
        let _ = Cli.delete(&codex_auth::service(&self.env), &codex_auth::store_key(&home));
        let _ = Cli.delete(&sources::anthropic_service(&self.env), "api-key");
        let _ = std::fs::remove_dir_all(&self.env.profile);
    }
}
fn fakes(ttl: u64) -> Fakes {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fakes");
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let server = Command::new("python3")
        .arg(dir.join("fake_backends.py"))
        .args([port.to_string(), ttl.to_string()])
        .spawn()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let profile = std::env::temp_dir().join(format!("pm-providers-{}-{port}", std::process::id()));
    std::fs::create_dir_all(profile.join("home")).unwrap();
    let vars = BTreeMap::from([
        ("PATH".to_owned(), std::env::var("PATH").unwrap_or_default()),
        // Never the person's own Codex home or login.
        ("HOME".to_owned(), profile.join("home").display().to_string()),
        ("PM_CODEX_KEYRING_SERVICE".to_owned(), format!("pm-aero-picker-test Codex Auth {}", std::process::id())),
        ("PM_FAKE_AUTH".to_owned(), format!("{base}/auth")),
        ("CODEX_BINARY".to_owned(), dir.join("fake_codex.py").display().to_string()),
        ("PM_AGENT_FAKE_BROWSER".to_owned(), "1".to_owned()),
        (
            "PM_AGENT_FAKES".to_owned(),
            format!("https://chatgpt.com={base}/chatgpt https://api.openai.com={base}/openai https://api.anthropic.com={base}/anthropic"),
        ),
    ]);
    let mut env = Environment {
        profile: profile.clone(),
        app_version: "test".into(),
        executable: "/bin/false".into(),
        keychain_prefix: format!("pm-aero-picker-test-{}", std::process::id()),
        secrets: Some(Arc::new(Cli)),
        compatible: Some(Arc::new(Custom(format!("{base}/custom/v1")))),
        // The development switch: Providers, and Codex's login in the Keychain.
        aero: Some(aero::Launcher(Arc::new(|_, _, _| Err("unused".into())))),
        ..Environment::default()
    };
    env.net = Some(Reroute::from_env(&vars, Arc::new(Loopback)));
    env.vars = vars;
    let deadline = Instant::now() + Duration::from_secs(10);
    while fetch(&Loopback, &NetRequest { method: "GET", url: &format!("{base}/stats"), headers: &[], body: b"" }).is_err() {
        assert!(Instant::now() < deadline, "the fake backends did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    Fakes { server, base, env }
}
fn stats(base: &str) -> Value {
    let (_, body) = fetch(&Loopback, &NetRequest { method: "GET", url: &format!("{base}/stats"), headers: &[], body: b"" }).unwrap();
    serde_json::from_slice(&body).unwrap()
}
/// The person's browser on the fake issuer: follow its redirect to Codex's loopback callback.
fn browser() -> Arc<pm_agents::account::Opener> {
    Arc::new(|url: &str| {
        let url = url.to_owned();
        std::thread::spawn(move || {
            let (_, body) = fetch(&Loopback, &NetRequest { method: "GET", url: &url, headers: &[], body: b"" }).unwrap();
            let next = String::from_utf8(body).unwrap().lines().next().unwrap().to_owned();
            let _ = fetch(&Loopback, &NetRequest { method: "GET", url: &next, headers: &[], body: b"" });
        });
        Ok(())
    })
}
fn wait(accounts: &Accounts, provider: Provider, state: State) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while accounts.status(provider).state != state {
        assert!(Instant::now() < deadline, "waiting for {state:?}: {:?}", accounts.status(provider));
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
/// One Powermove core turn on `model` (no tools: the evaluator path).
fn powermove_says(env: &Environment, model: &str, effort: &str) -> Result<String, String> {
    let live = sources::live(env, model, effort)?;
    let scope = env.profile.join(format!("turn-{}", model.replace('/', "-")));
    std::fs::create_dir_all(&scope).unwrap();
    aero::evaluate(&aero::Model::live(live), &scope, "Say hello.", &[])
}

#[test]
fn chatgpt_sign_in_is_shared_refreshed_by_codex_alone_and_signed_out_for_both() {
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
    until("Codex's models", || !accounts.models(Provider::Chatgpt).is_empty());
    // 2. One Keychain item: Codex wrote it, Powermove reads the same one; no auth.json.
    let auth = codex_auth::read(&Cli, env).unwrap().expect("the shared item");
    assert_eq!(auth.chatgpt.as_ref().unwrap().1.as_deref(), Some("acct-fake-1"));
    assert!(!codex_auth::codex_home(env).join("auth.json").exists());
    let config = std::fs::read_to_string(codex_auth::codex_home(env).join("config.toml")).unwrap();
    assert!(config.contains("cli_auth_credentials_store = \"keyring\""), "{config}");

    // 3–4. Powermove runs on it: the bearer and account id only at chatgpt.com's edge.
    let text = powermove_says(env, "chatgpt/gpt-fake-sol", "medium").unwrap();
    assert!(text.contains("Hello from the fake provider"), "{text}");
    let seen = stats(&f.base);
    let codex_calls: Vec<_> = seen["requests"].as_array().unwrap().iter()
        .filter(|r| r["path"] == "/chatgpt/backend-api/codex/responses").collect();
    assert!(!codex_calls.is_empty());
    assert!(codex_calls.iter().all(|r| r["authorization"].as_str().unwrap().starts_with("Bearer ") && r["account"] == "acct-fake-1"));

    // Refresh ownership: the token is inside Codex's five-minute window, so
    // every ask refreshes; eight at once go one by one through Codex, each on
    // the newest refresh token, and none is ever spent twice.
    let before = stats(&f.base)["refreshes"].as_u64().unwrap();
    let asks: Vec<_> = (0..8)
        .map(|_| {
            let env = env.clone();
            std::thread::spawn(move || codex_auth::current(&Cli, &env, None).map(|a| a.chatgpt.unwrap().0))
        })
        .collect();
    let tokens: Vec<String> = asks.into_iter().map(|t| t.join().unwrap().unwrap()).collect();
    let seen = stats(&f.base);
    assert_eq!(seen["reused"], 0, "a refresh token was spent twice: {seen}");
    assert!(seen["refreshes"].as_u64().unwrap() >= before + 1);
    let latest = codex_auth::read(&Cli, env).unwrap().unwrap().chatgpt.unwrap().0;
    assert!(tokens.contains(&latest));
    // A backend refusing the bearer asks once more; the item moves on.
    let again = codex_auth::current(&Cli, env, Some(&latest)).unwrap().chatgpt.unwrap().0;
    assert_ne!(again, latest);
    assert_eq!(stats(&f.base)["reused"], 0);

    // 5. Sign out: the item is gone for Codex and for Powermove.
    accounts.disconnect(Provider::Chatgpt);
    wait(&accounts, Provider::Chatgpt, State::Disconnected);
    assert!(codex_auth::read(&Cli, env).unwrap().is_none());
    assert!(accounts.models(Provider::Chatgpt).is_empty());
    let refused = powermove_says(env, "chatgpt/gpt-fake-sol", "").unwrap_err();
    assert!(refused.contains("Settings › Providers") || refused.contains("401"), "{refused}");
    accounts.close();
}

#[test]
fn an_openai_key_an_anthropic_key_and_a_custom_endpoint_each_run_powermove() {
    if !pm_aero_core::available() {
        return;
    }
    let f = fakes(3600);
    let env = &f.env;
    let accounts = Accounts::new(env.clone(), browser(), Arc::new(|| {}));

    // OpenAI API key: Codex saves it in the shared item; Powermove lists the key's chat models.
    accounts.connect_openai_key("sk-fake-openai-123456".into());
    wait(&accounts, Provider::Chatgpt, State::Connected);
    assert_eq!(accounts.status(Provider::Chatgpt).key.as_deref(), Some("sk-…3456"));
    let ids: Vec<_> = accounts.openai_key_models().into_iter().map(|m| m.id).collect();
    assert_eq!(ids, ["gpt-fake-key"]);
    assert_eq!(codex_auth::read(&Cli, env).unwrap().unwrap().api_key.as_deref(), Some("sk-fake-openai-123456"));
    let text = powermove_says(env, "openai/gpt-fake-key", "").unwrap();
    assert!(text.contains("Hello from the fake provider"), "{text}");
    accounts.disconnect(Provider::Chatgpt);
    wait(&accounts, Provider::Chatgpt, State::Disconnected);
    assert!(accounts.openai_key_models().is_empty());

    // Anthropic API key: the Keychain, its model list, Powermove through the compatibility endpoint.
    assert!(accounts.set_anthropic_key("not a key".into()).is_err());
    accounts.set_anthropic_key("sk-ant-fake-abcdef12".into()).unwrap();
    until("the Anthropic models", || !accounts.anthropic().models.is_empty());
    assert_eq!(accounts.anthropic().masked.as_deref(), Some("sk-…ef12"));
    let text = powermove_says(env, "anthropic/claude-fake-opus", "").unwrap();
    assert!(text.contains("Hello from the fake provider"), "{text}");
    let anthropic_calls: Vec<_> = stats(&f.base)["requests"].as_array().unwrap().iter()
        .filter(|r| r["path"] == "/anthropic/v1/chat/completions").cloned().collect();
    assert!(!anthropic_calls.is_empty() && anthropic_calls.iter().all(|r| r["x-api-key"] == true));
    accounts.remove_anthropic_key().unwrap();
    assert!(accounts.anthropic().masked.is_none());
    assert!(powermove_says(env, "anthropic/claude-fake-opus", "").unwrap_err().contains("Anthropic API key"));

    // Custom: the saved OpenAI-compatible endpoint, Powermove only.
    let text = powermove_says(env, "custom/custom-fake-model", "").unwrap();
    assert!(text.contains("Hello from the fake provider"), "{text}");
    accounts.close();
}
