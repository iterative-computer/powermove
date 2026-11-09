//! The Powermove Cloud host against the fixture registry, through the same
//! `pm-services` work the sign-in sheets submit. The session lives in a
//! memory vault here; the app's is the Keychain.
use crate::cloud::{Http, Vault};
use crate::store_window_tests::{Account as Fixture, CODE, serve_with};
use pm_services::surfaces::{
    Flight,
    agent_auth::{Account, BrowserAuth, CloudReply, CloudTask, CloudWork, Secret},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
struct Memory(Arc<Mutex<Option<Vec<u8>>>>);
impl Vault for Memory {
    fn read(&self) -> Result<Option<Vec<u8>>, String> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        *self.0.lock().unwrap() = Some(bytes.to_vec());
        Ok(())
    }
    fn delete(&mut self) -> Result<(), String> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }
}
struct Setup {
    origin: &'static str,
    host: Http,
    vault: Memory,
    fixture: Arc<Mutex<Fixture>>,
    _profile: tempfile::TempDir,
}
fn setup() -> Setup {
    let fixture = Arc::new(Mutex::new(Fixture::default()));
    let origin: &'static str = Box::leak(serve_with(0, fixture.clone()).into_boxed_str());
    let vault = Memory::default();
    let profile = tempfile::tempdir().unwrap();
    Setup {
        origin,
        host: Http::new(origin, Box::new(vault.clone()), profile.path().to_owned()),
        vault,
        fixture,
        _profile: profile,
    }
}
fn run(s: &mut Setup, work: CloudWork) -> pm_services::Result<CloudReply> {
    let task = CloudTask::new(Flight::default().start(), s.origin, work).unwrap();
    task.run(&mut s.host).result
}
fn account(reply: pm_services::Result<CloudReply>) -> Option<Account> {
    match reply.unwrap() {
        CloudReply::Account(account) => account,
        other => panic!("{other:?}"),
    }
}

#[test]
fn email_sign_in_handle_remember_and_sign_out() {
    let mut s = setup();
    run(
        &mut s,
        CloudWork::SendEmail {
            email: "person@example.com".into(),
        },
    )
    .unwrap();
    // A wrong code is the registry's `unauthorized`, which the sheet words.
    let wrong = run(
        &mut s,
        CloudWork::VerifyEmail {
            email: "person@example.com".into(),
            code: Secret::new("000000".into()),
        },
    );
    assert_eq!(wrong.unwrap_err().to_string(), "api:unauthorized");
    let signed = account(run(
        &mut s,
        CloudWork::VerifyEmail {
            email: "person@example.com".into(),
            code: Secret::new(CODE.into()),
        },
    ))
    .unwrap();
    assert_eq!(signed.email, "person@example.com");
    assert_eq!(signed.handle, None);
    // The session is sealed with its origin, and never in the account facts.
    let sealed: serde_json::Value =
        serde_json::from_slice(&s.vault.read().unwrap().unwrap()).unwrap();
    assert_eq!(sealed["origin"], s.origin.trim_end_matches('/'));
    assert!(!format!("{signed:?}").contains("fixture-session"));
    let taken = run(
        &mut s,
        CloudWork::ClaimHandle {
            handle: "taken".into(),
        },
    );
    assert_eq!(taken.unwrap_err().to_string(), "api:handle_taken");
    let named = account(run(
        &mut s,
        CloudWork::ClaimHandle {
            handle: "maker".into(),
        },
    ))
    .unwrap();
    assert_eq!(named.identity(), "@maker");
    let remembered = account(run(&mut s, CloudWork::RememberInstalls(true))).unwrap();
    assert!(remembered.remember_installs);
    s.host
        .record_install(
            "5a1c0000-0000-4000-8000-000000000001",
            Some("5a1c0000-0000-4000-8000-0000000000a2"),
        )
        .unwrap();
    assert_eq!(s.fixture.lock().unwrap().installs.len(), 1);
    s.host
        .record_install("5a1c0000-0000-4000-8000-000000000001", None)
        .unwrap();
    assert!(s.fixture.lock().unwrap().installs.is_empty());
    assert!(matches!(
        run(&mut s, CloudWork::SignOut),
        Ok(CloudReply::SignedOut)
    ));
    assert!(s.vault.read().unwrap().is_none());
    assert!(!s.host.signed_in());
}

#[test]
fn browser_sign_in_exchanges_the_return_link_with_its_verifier_once() {
    let mut s = setup();
    let mut auth = BrowserAuth::default();
    let url = auth.begin(s.origin, [3; 32], [4; 16], 0).unwrap();
    assert!(
        url.as_str()
            .starts_with(&format!("{}v1/auth/desktop?", s.origin))
    );
    // The registry's page answers with the return link a browser follows.
    let page = pm_compat_runner::net::send(
        pm_runtime_v8::FetchRequest {
            url: url.to_string(),
            method: "GET".into(),
            headers: vec![],
            body: vec![],
            max_response_bytes: 4096,
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(10),
        },
        pm_compat_runner::net::Reach::Origin(s.origin),
    );
    // NSURLSession may refuse the custom-scheme hop; the fixture recorded the
    // challenge either way, and its link is state + browser token.
    drop(page);
    let state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let link = format!("powermove://auth?state={state}&token=browser-{state}");
    // A link for another sign-in leaves this one waiting.
    assert!(
        auth.receive("powermove://auth?state=00&token=x", s.origin, 1)
            .is_none()
    );
    let exchange = auth.receive(&link, s.origin, 1).unwrap();
    assert!(auth.receive(&link, s.origin, 2).is_none());
    let signed = account(run(&mut s, CloudWork::Exchange(exchange))).unwrap();
    assert_eq!(signed.email, "fixture@example.com");
    // Delete Account ends every session.
    s.host.delete_account().unwrap();
    assert!(s.fixture.lock().unwrap().deleted);
    assert!(!s.host.signed_in());
}

#[test]
fn an_ended_session_reads_as_signed_out_and_is_forgotten() {
    let mut s = setup();
    run(
        &mut s,
        CloudWork::SendEmail {
            email: "person@example.com".into(),
        },
    )
    .unwrap();
    account(run(
        &mut s,
        CloudWork::VerifyEmail {
            email: "person@example.com".into(),
            code: Secret::new(CODE.into()),
        },
    ));
    // The registry forgets every session (another Mac deleted the account).
    s.fixture.lock().unwrap().sessions_clear();
    assert!(
        account(Ok(CloudReply::Account(
            pm_services::surfaces::agent_auth::CloudHost::refresh_account(&mut s.host).unwrap()
        )))
        .is_none()
    );
    assert!(s.vault.read().unwrap().is_none());
}

/// A signed-in maker with a handle, and an extension made on this Mac.
fn maker() -> (Setup, std::path::PathBuf) {
    let mut s = setup();
    run(
        &mut s,
        CloudWork::SendEmail {
            email: "maker@example.com".into(),
        },
    )
    .unwrap();
    account(run(
        &mut s,
        CloudWork::VerifyEmail {
            email: "maker@example.com".into(),
            code: Secret::new(CODE.into()),
        },
    ));
    account(run(
        &mut s,
        CloudWork::ClaimHandle {
            handle: "maker".into(),
        },
    ));
    let profile = s._profile.path().to_owned();
    let folder = profile.join("extensions/my-glow");
    std::fs::create_dir_all(folder.join("src")).unwrap();
    std::fs::write(
        folder.join("manifest.json"),
        "{\n  \"id\": \"my-glow\",\n  \"name\": \"My Glow\",\n  \"version\": \"0.9.0\",\n  \"apiVersion\": 3,\n  \"permissions\": []\n}\n",
    )
    .unwrap();
    std::fs::write(
        folder.join("src/index.ts"),
        "export default function () {}\n",
    )
    .unwrap();
    std::fs::write(folder.join(".DS_Store"), "never published").unwrap();
    (s, profile)
}
fn publish(s: &mut Setup, profile: &std::path::Path, version: &str) -> Result<String, String> {
    use pm_services::surfaces::publishing::Draft;
    let prepared = crate::publish::prepare(&mut s.host, profile, "my-glow", true)?;
    let mut draft = Draft::for_plan(&prepared.plan);
    draft.version = version.into();
    draft.name = "My Glow".into();
    draft.category = "effects".into();
    crate::publish::draft_reasons(&prepared, &mut draft);
    let form = draft
        .form(&prepared.plan)
        .map_err(|p| format!("{:?}", p[0].problem))?;
    let frozen = crate::publish::freeze(&prepared, &form, 1_800_000_000)?;
    assert_eq!(
        frozen.confirmation().0,
        format!("Publish maker/my-glow {version}?")
    );
    crate::publish::upload(&mut s.host, profile, &prepared, frozen)
}

#[test]
fn publish_uploads_verified_objects_then_an_update_follows_its_commit() {
    let (mut s, profile) = maker();
    let plan = crate::publish::prepare(&mut s.host, &profile, "my-glow", true)
        .unwrap()
        .plan;
    assert!(plan.first_publish);
    assert_eq!(plan.coordinate, "maker/my-glow");
    assert_eq!(plan.suggested_version, "0.9.0");
    assert_eq!(
        publish(&mut s, &profile, "0.9.0").unwrap(),
        "Published maker/my-glow 0.9.0."
    );
    {
        let fixture = s.fixture.lock().unwrap();
        let releases = &fixture.releases["maker/my-glow"];
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].3, None);
        // Two blobs, two trees and the commit, each inflated and verified.
        assert_eq!(fixture.objects.len(), 5);
        assert!(
            !fixture
                .objects
                .values()
                .any(|o| o.ends_with(b"never published"))
        );
    }
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(profile.join("extensions-provenance.json")).unwrap())
            .unwrap();
    assert_eq!(record["my-glow"]["published"]["version"], "0.9.0");
    // An update: the Store's version is the floor, the commit follows the last.
    std::fs::write(
        profile.join("extensions/my-glow/src/index.ts"),
        "export default 2;\n",
    )
    .unwrap();
    let plan = crate::publish::prepare(&mut s.host, &profile, "my-glow", true)
        .unwrap()
        .plan;
    assert!(!plan.first_publish);
    assert_eq!(plan.last_version.as_deref(), Some("0.9.0"));
    assert_eq!(plan.suggested_version, "0.9.1");
    assert_eq!(
        publish(&mut s, &profile, "0.9.0").unwrap_err(),
        "VersionNotHigher"
    );
    assert_eq!(
        publish(&mut s, &profile, "0.9.1").unwrap(),
        "Published maker/my-glow 0.9.1."
    );
    let fixture = s.fixture.lock().unwrap();
    let releases = &fixture.releases["maker/my-glow"];
    assert_eq!(releases[1].3.as_deref(), Some(releases[0].1.as_str()));
    // The published version went back into the folder's manifest.json.
    let manifest =
        std::fs::read_to_string(profile.join("extensions/my-glow/manifest.json")).unwrap();
    assert!(manifest.contains("\"version\": \"0.9.1\""));
    assert!(manifest.find("\"id\"").unwrap() < manifest.find("\"version\"").unwrap());
}

#[test]
fn publish_refuses_secrets_undeclared_permissions_store_installs_and_a_signed_out_mac() {
    let (mut s, profile) = maker();
    let folder = profile.join("extensions/my-glow");
    std::fs::write(
        folder.join("src/key.ts"),
        format!("const k = \"AKIA{}\";\n", "A".repeat(16)),
    )
    .unwrap();
    let plan = crate::publish::prepare(&mut s.host, &profile, "my-glow", true)
        .unwrap()
        .plan;
    assert!(
        crate::publish::plan_problem(&plan)
            .unwrap()
            .contains("looks like a secret")
    );
    std::fs::write(folder.join("src/key.ts"), "await fetch(url)\n").unwrap();
    let plan = crate::publish::prepare(&mut s.host, &profile, "my-glow", true)
        .unwrap()
        .plan;
    assert!(
        crate::publish::plan_problem(&plan)
            .unwrap()
            .contains("network permission")
    );
    // A random-looking value with its reason in the code publishes as a waiver.
    std::fs::write(
        folder.join("src/key.ts"),
        "const id = \"Zq8Xw2Lp9Rt4Vn7Ks1Mb6Jh3Gf5Dc0Ae2Bq9Wy\"; // powermove-secret-ok: public sample id\n",
    )
    .unwrap();
    assert!(publish(&mut s, &profile, "0.9.0").is_ok());
    // Never an extension the Store installed, and never signed out.
    let store = profile.join("extensions/from-store");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(store.join(".powermove-store.json"), "{}").unwrap();
    assert!(crate::publish::prepare(&mut s.host, &profile, "from-store", true).is_err());
    assert!(matches!(
        run(&mut s, CloudWork::SignOut),
        Ok(CloudReply::SignedOut)
    ));
    assert_eq!(
        crate::publish::prepare(&mut s.host, &profile, "my-glow", true)
            .err()
            .as_deref(),
        Some("Sign in to publish.")
    );
}

#[test]
fn a_model_provider_is_discovered_tested_and_saved_with_its_key_in_the_vault() {
    use crate::providers::Host;
    use crate::store_window_tests::MODEL_KEY;
    use pm_services::surfaces::{
        agent_auth::ProviderInput,
        providers::{ConfigureWork, DiscoveryWork},
    };
    let fixture = Arc::new(Mutex::new(Fixture::default()));
    let origin = serve_with(0, fixture);
    let endpoint = format!("{origin}v1");
    let profile = tempfile::tempdir().unwrap();
    let vault = Memory::default();
    let keys = vault.clone();
    let mut host = Host {
        profile: profile.path().to_owned(),
        keys: Box::new(move |_| Box::new(keys.clone())),
    };
    let ticket = || Flight::default().start();
    // Discovery lists models that can call tools, with what they read.
    let found = DiscoveryWork {
        ticket: ticket(),
        endpoint: endpoint.clone(),
        key: None,
        has_saved_key: false,
    }
    .run(&mut host)
    .result
    .unwrap();
    let ids: Vec<_> = found.models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["fixture-chat", "fixture-vision"]);
    assert_eq!(found.hidden, 1);
    assert_eq!(found.models[1].vision, Some(true));
    // A wrong key is refused and nothing is saved.
    let input = ProviderInput::prepare(&endpoint, "fixture-chat", false, "wrong", None).unwrap();
    let refused = ConfigureWork {
        ticket: ticket(),
        input,
    }
    .run(&mut host)
    .result;
    assert_eq!(
        refused.unwrap_err().code,
        pm_services::ServiceErrorCode::PermissionDenied
    );
    assert!(host.saved().is_none());
    // The right one connects after a real chat completion.
    let input = ProviderInput::prepare(&endpoint, "fixture-vision", true, MODEL_KEY, None).unwrap();
    let config = ConfigureWork {
        ticket: ticket(),
        input,
    }
    .run(&mut host)
    .result
    .unwrap();
    assert!(config.has_key);
    let saved = host.saved().unwrap();
    assert_eq!(saved["model"], "fixture-vision");
    assert_eq!(saved["vision"], true);
    // The key is in the vault, never in the saved connection.
    assert_eq!(host.saved_key(&endpoint).as_deref(), Some(MODEL_KEY));
    let file = std::fs::read_to_string(profile.path().join("agent-provider.json")).unwrap();
    assert!(!file.contains(MODEL_KEY));
    // Keeping the saved key: a blank entry reuses it for the same address only.
    let input = ProviderInput::prepare(
        &endpoint,
        "fixture-chat",
        false,
        "",
        Some((&endpoint, MODEL_KEY)),
    )
    .unwrap();
    assert!(
        ConfigureWork {
            ticket: ticket(),
            input
        }
        .run(&mut host)
        .result
        .is_ok()
    );
    host.disconnect().unwrap();
    assert!(host.saved().is_none());
    assert_eq!(host.saved_key(&endpoint).as_deref(), Some(MODEL_KEY));
}

#[test]
fn agent_apps_get_powermove_added_and_removed_in_their_own_settings_only() {
    use crate::agent_clients::{ProfileConsent, change, registration};
    use pm_services::surfaces::client_consent::{Answer, ConsentStore, STORE_KEY};
    let home = tempfile::tempdir().unwrap();
    let profile = tempfile::tempdir().unwrap();
    let applications = home.path().join("Applications");
    std::fs::create_dir_all(home.path().join(".cursor")).unwrap();
    std::fs::create_dir_all(home.path().join("Library/Application Support/Claude")).unwrap();
    // A hand-made server in Claude's settings must survive.
    let claude = home
        .path()
        .join("Library/Application Support/Claude/claude_desktop_config.json");
    std::fs::write(
        &claude,
        r#"{"mcpServers":{"mine":{"command":"/usr/bin/true"}}}"#,
    )
    .unwrap();
    let app = PathBuf::from("/Applications/Powermove.app/Contents/MacOS/powermove-native");
    let r = registration(profile.path(), home.path(), &applications, app).unwrap();
    let state = |r: &pm_mcp::registration::Registration, id: &str| {
        r.inspect()["clients"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id)
            .unwrap()["state"]
            .clone()
    };
    assert_eq!(state(&r, "cursor"), "available");
    assert_eq!(state(&r, "codex"), "not-installed");
    assert_eq!(
        change(&r, "cursor", true).unwrap(),
        "Restart Cursor to use it."
    );
    let cursor: serde_json::Value =
        serde_json::from_slice(&std::fs::read(home.path().join(".cursor/mcp.json")).unwrap())
            .unwrap();
    assert_eq!(
        cursor["mcpServers"]["powermove"]["args"][0],
        "--powermove-mcp"
    );
    assert_eq!(state(&r, "cursor"), "connected");
    change(&r, "claude-desktop", true).unwrap();
    let claude: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&claude).unwrap()).unwrap();
    assert!(claude["mcpServers"]["mine"].is_object());
    assert!(claude["mcpServers"]["powermove"].is_object());
    assert_eq!(change(&r, "cursor", false).unwrap(), "Removed.");
    assert_eq!(state(&r, "cursor"), "available");
    // Claude's question is answered once per profile.
    let mut store = ProfileConsent(profile.path().to_owned());
    assert!(store.record_once(STORE_KEY, Answer::Added).unwrap().fresh);
    let again = store.record_once(STORE_KEY, Answer::Declined).unwrap();
    assert!(!again.fresh);
    assert_eq!(again.answer, Answer::Added);
}
