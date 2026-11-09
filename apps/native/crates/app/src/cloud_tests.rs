//! The Powermove Cloud host against the fixture registry, through the same
//! `pm-services` work the sign-in sheets submit. The session lives in a
//! memory vault here; the app's is the Keychain.
use crate::cloud::{Http, Vault};
use crate::store_window_tests::{Account as Fixture, CODE, serve_with};
use pm_services::surfaces::{
    Flight,
    agent_auth::{Account, BrowserAuth, CloudReply, CloudTask, CloudWork, Secret},
};
use std::sync::{Arc, Mutex};

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
