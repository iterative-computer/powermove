//! Window ▸ Store against a fixture Store on this Mac that follows the
//! registry's wire contract (packages/registry/src/wire) and signs its
//! releases with a test key, driven by clicks as a person would.
use crate::store_window::{Page, Services, StoreWindow};
use ed25519_dalek::{Signer, SigningKey};
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use pm_compat_runner::store_install;
use pm_design::{Accessibility, Scheme, Tokens};
use pm_kernel::{Kernel, KernelConfig};
use pm_store_host::{
    artifact::{FileSeal, ReleaseStatement, SignedRelease, TrustStore},
    identity::ExtensionId,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const GLOW_REPO: &str = "5a1c0000-0000-4000-8000-000000000001";
pub const NOTES_REPO: &str = "5a1c0000-0000-4000-8000-000000000002";
pub const GLOW_OLD: &str = "5a1c0000-0000-4000-8000-0000000000a1";
pub const GLOW_NEW: &str = "5a1c0000-0000-4000-8000-0000000000a2";
pub const NOTES_ONE: &str = "5a1c0000-0000-4000-8000-0000000000b1";
const PUBLISHER: &str = "fixture-publisher";
const KEY_ID: &str = "fixture-key";
const SEED: [u8; 32] = [9; 32];

/// A release: (id, repo, extension id, name, version, published).
type Release = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);
const RELEASES: [Release; 3] = [
    (
        GLOW_OLD,
        GLOW_REPO,
        "soft-glow",
        "Soft Glow",
        "1.0.0",
        "2026-10-01T09:00:00Z",
    ),
    (
        GLOW_NEW,
        GLOW_REPO,
        "soft-glow",
        "Soft Glow",
        "1.1.0",
        "2026-11-02T09:00:00Z",
    ),
    (
        NOTES_ONE,
        NOTES_REPO,
        "shot-notes",
        "Shot Notes",
        "2.0.0",
        "2026-09-19T09:00:00Z",
    ),
];
fn files(release: &Release) -> Vec<(String, Vec<u8>)> {
    let (_, _, id, name, version, _) = *release;
    let manifest = json!({"id":id,"name":name,"version":version,"apiVersion":3,"permissions":[]});
    let source = if id == "soft-glow" {
        "export default function(api: any){api.effects.register({id:'glow',label:'Soft Glow',group:'Stylize',params:[{k:'amount',label:'Amount',min:0,max:1,def:0.5}],frag:'o = src(v_uv) * (1.0 + u_amount);'});}"
    } else {
        "export default function(api: any){api.commands.register({id:'note',label:'Add Shot Note',run(){}});}"
    };
    vec![
        ("manifest.json".into(), manifest.to_string().into_bytes()),
        ("index.ts".into(), source.as_bytes().to_vec()),
    ]
}
fn archive(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    ));
    for (name, bytes) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, name, bytes.as_slice())
            .unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap()
}
fn signed(release: &Release) -> (SignedRelease, Vec<u8>) {
    let files = files(release);
    let bytes = archive(&files);
    let statement = ReleaseStatement {
        format: 1,
        key_id: KEY_ID.into(),
        publisher: PUBLISHER.into(),
        extension: ExtensionId::new(release.2).unwrap(),
        release_id: release.0.into(),
        archive_sha256: format!("{:x}", Sha256::digest(&bytes)),
        files: files
            .iter()
            .map(|(name, data)| {
                let seal = FileSeal {
                    sha256: format!("{:x}", Sha256::digest(data)),
                    bytes: data.len() as u64,
                };
                (name.clone(), seal)
            })
            .collect(),
    };
    let signature = SigningKey::from_bytes(&SEED)
        .sign(&statement.signing_bytes().unwrap())
        .to_vec();
    (
        SignedRelease {
            statement,
            signature,
        },
        bytes,
    )
}
fn summary(release: &Release) -> Value {
    json!({"id":release.0,"version":release.4,"publishedAt":release.5,"apiVersion":3,"yankedAt":null})
}
fn listing(repo: &str) -> Value {
    let latest = RELEASES.iter().rev().find(|r| r.1 == repo).unwrap();
    let (category, tagline, permissions) = if repo == GLOW_REPO {
        (
            "effects",
            "A soft bloom for highlights.",
            json!(["network"]),
        )
    } else {
        ("panels", "Notes beside every shot.", json!([]))
    };
    json!({"repoId":repo,"owner":{"id":"5a1c0000-0000-4000-8000-0000000000ff","handle":"fixture","tombstoned":false,"verified":true},
        "slug":latest.2,"name":latest.3,"tagline":tagline,"category":category,"iconUrl":null,"visibility":"public",
        "latest":summary(latest),"permissions":permissions,"installCount":3,"forkCount":0,"licence":"MIT",
        "forkedFrom":null,"createdAt":"2026-09-01T00:00:00Z","updatedAt":latest.5})
}
fn detail(repo: &str) -> Value {
    let mut value = listing(repo);
    value["about"] = Value::Null;
    value["moderation"] = json!("none");
    value["releases"] = RELEASES
        .iter()
        .rev()
        .filter(|r| r.1 == repo)
        .map(|r| {
            let mut release = summary(r);
            release["repoId"] = json!(repo);
            release["notes"] = json!(if r.4 == "1.1.0" {
                "Smoother falloff."
            } else {
                "First release."
            });
            release["manifest"] = json!({"id":r.2,"name":r.3,"version":r.4,"apiVersion":3,
                "contributes":[if repo == GLOW_REPO {"effects"} else {"commands"}],
                "vars":[],"permissions":[],"links":[],"forkedFrom":null,
                "description":"Synthetic fixture extension."});
            release
        })
        .collect();
    value
}
/// The account side of the fixture: one person, sessions by bearer token.
#[derive(Default)]
pub struct Account {
    challenges: std::collections::BTreeMap<String, String>,
    sessions: std::collections::BTreeSet<String>,
    email: String,
    handle: Option<String>,
    remember: bool,
    pub installs: Vec<(String, String)>,
    pub deleted: bool,
}
impl Account {
    pub fn sessions_clear(&mut self) {
        self.sessions.clear();
    }
}
/// The emailed code the fixture accepts.
pub const CODE: &str = "424242";
fn cloud(
    method: &str,
    path: &str,
    head: &str,
    body: &[u8],
    account: &Mutex<Account>,
) -> Option<(u16, Vec<u8>)> {
    use base64::Engine;
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let request: Value = serde_json::from_slice(body).unwrap_or_default();
    let mut a = account.lock().unwrap();
    let bearer = head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.eq_ignore_ascii_case("authorization")
            .then(|| v.trim().strip_prefix("Bearer ").map(str::to_owned))?
    });
    let authed = bearer.as_ref().is_some_and(|t| a.sessions.contains(t));
    let json = |v: Value| Some((200, v.to_string().into_bytes()));
    let fail =
        |status: u16, code: &str| Some((status, json!({ "error": code }).to_string().into_bytes()));
    let session = |a: &mut Account| {
        let token = format!("fixture-session-{}", a.sessions.len() + 1);
        a.sessions.insert(token.clone());
        json!({"token":token,"expiresAt":"2099-01-01T00:00:00.000Z"})
    };
    let me = |a: &Account| {
        json!({"user":{"id":"fixture-user","name":null,"email":a.email,"image":null},
        "publisher":a.handle.as_ref().map(|h| json!({"id":"5a1c0000-0000-4000-8000-0000000000ee","handle":h,"tombstoned":false,"verified":false})),
        "settings":{"rememberInstalls":a.remember}})
    };
    match (method, path) {
        // The registry's Google page, played by the review: it answers with
        // the return link a browser would follow.
        ("GET", "/v1/auth/desktop") => {
            let pairs: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
                .into_owned()
                .collect();
            let get = |k: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| key == k)
                    .map(|(_, v)| v.clone())
            };
            let (state, challenge) = (get("state")?, get("challenge")?);
            a.challenges.insert(state.clone(), challenge);
            Some((
                302,
                format!("powermove://auth?state={state}&token=browser-{state}").into_bytes(),
            ))
        }
        ("POST", "/v1/auth/desktop/exchange") => {
            let state = request["state"].as_str()?.to_owned();
            let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(request["verifier"].as_str()?)
                .ok()?;
            let challenge =
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(&verifier));
            if a.challenges.remove(&state) != Some(challenge)
                || request["token"].as_str() != Some(&format!("browser-{state}"))
            {
                return fail(401, "unauthorized");
            }
            a.email = "fixture@example.com".into();
            a.deleted = false;
            json(session(&mut a))
        }
        ("POST", "/v1/auth/email/send") => {
            a.email = request["email"].as_str()?.to_owned();
            json(json!({"ok":true}))
        }
        ("POST", "/v1/auth/email/verify") => {
            if request["otp"].as_str() != Some(CODE) || request["email"] != a.email.as_str() {
                return fail(401, "unauthorized");
            }
            a.deleted = false;
            json(session(&mut a))
        }
        (_, p)
            if p.starts_with("/v1/me")
                || p.starts_with("/v1/installs")
                || p == "/v1/auth/sign-out" =>
        {
            if !authed {
                return fail(401, "unauthorized");
            }
            match (method, p) {
                ("GET", "/v1/me") => json(me(&a)),
                ("DELETE", "/v1/me") => {
                    a.sessions.clear();
                    a.deleted = true;
                    a.handle = None;
                    Some((204, vec![]))
                }
                ("POST", "/v1/me/handle") => match request["handle"].as_str()? {
                    "taken" => fail(409, "handle_taken"),
                    "powermove" => fail(400, "handle_reserved"),
                    handle => {
                        a.handle = Some(handle.to_owned());
                        json(json!({"publisher":me(&a)["publisher"]}))
                    }
                },
                ("PATCH", "/v1/me/settings") => {
                    a.remember = request["rememberInstalls"].as_bool()?;
                    if !a.remember {
                        a.installs.clear();
                    }
                    json(json!({"settings":{"rememberInstalls":a.remember}}))
                }
                ("POST", "/v1/auth/sign-out") => {
                    if let Some(token) = &bearer {
                        a.sessions.remove(token);
                    }
                    json(json!({"ok":true}))
                }
                ("POST", "/v1/installs") => {
                    a.installs.push((
                        request["repoId"].as_str()?.to_owned(),
                        request["releaseId"].as_str()?.to_owned(),
                    ));
                    Some((204, vec![]))
                }
                ("DELETE", p) => {
                    let repo = p.strip_prefix("/v1/installs/")?;
                    a.installs.retain(|(r, _)| r != repo);
                    Some((204, vec![]))
                }
                _ => None,
            }
        }
        _ => None,
    }
}
fn route(method: &str, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let json = |v: Value| (200, v.to_string().into_bytes());
    let all = [GLOW_REPO, NOTES_REPO];
    match (method, path) {
        ("GET", "/v1/store/browse") => json(json!({"sections":[
            {"id":"featured","title":"Featured","items":[listing(GLOW_REPO),listing(NOTES_REPO)]},
            {"id":"new","title":"New","items":[listing(GLOW_REPO)]}]})),
        ("GET", "/v1/store/extensions") => {
            let pairs: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
                .into_owned()
                .collect();
            let get = |k: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| key == k)
                    .map(|(_, v)| v.clone())
            };
            let items: Vec<Value> = all
                .iter()
                .map(|repo| listing(repo))
                .filter(|l| get("category").is_none_or(|c| l["category"] == c.as_str()))
                .filter(|l| {
                    get("q").is_none_or(|q| {
                        l["name"]
                            .as_str()
                            .unwrap()
                            .to_lowercase()
                            .contains(&q.to_lowercase())
                    })
                })
                .collect();
            json(json!({"items":items,"nextCursor":null}))
        }
        ("POST", "/v1/store/versions") => {
            let request: Value = serde_json::from_slice(body).unwrap_or_default();
            let items: Vec<Value> = request["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|item| {
                    let repo = item["repoId"].as_str().unwrap_or_default();
                    let latest = RELEASES.iter().rev().find(|r| r.1 == repo);
                    json!({"repoId":repo,"state":if latest.is_some() {"ok"} else {"removed"},
                        "ownerPublisherId":"5a1c0000-0000-4000-8000-0000000000ff","current":{"yanked":false},
                        "latest":latest.map(|r| json!({"releaseId":r.0,"version":r.4,"treeSha":"a".repeat(40),
                            "tarSha256":format!("{:x}", Sha256::digest(signed(r).1)),"apiVersion":3})),
                        "handle":"fixture","slug":latest.map(|r| r.2)})
                })
                .collect();
            json(json!({ "items": items }))
        }
        ("GET", path) => {
            if let Some(rest) = path.strip_prefix("/v1/store/x/fixture/") {
                return match all.iter().find(|repo| listing(repo)["slug"] == rest) {
                    Some(repo) => json(detail(repo)),
                    None => (404, br#"{"error":"not_found"}"#.to_vec()),
                };
            }
            let release = path
                .strip_prefix("/v1/store/releases/")
                .and_then(|rest| rest.split_once('/'))
                .and_then(|(id, what)| Some((RELEASES.iter().find(|r| r.0 == id)?, what)));
            match release {
                Some((release, "signature")) => {
                    json(serde_json::to_value(signed(release).0).unwrap())
                }
                Some((release, "tar")) => (200, signed(release).1),
                _ => (404, br#"{"error":"not_found"}"#.to_vec()),
            }
        }
        _ => (404, br#"{"error":"not_found"}"#.to_vec()),
    }
}
/// Serves the fixture Store on 127.0.0.1; returns its origin.
pub fn serve(port: u16) -> String {
    serve_with(port, Arc::default())
}
/// As [`serve`], with the account the test reads back.
pub fn serve_with(port: u16, account: Arc<Mutex<Account>>) -> String {
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
    let origin = format!(
        "http://127.0.0.1:{}/",
        listener.local_addr().unwrap().port()
    );
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let account = account.clone();
            std::thread::spawn(move || {
                let mut stream = stream;
                let mut data = Vec::new();
                let mut buffer = [0; 8192];
                let (head, body) = loop {
                    let n = stream.read(&mut buffer).unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    data.extend_from_slice(&buffer[..n]);
                    if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&data[..end]).into_owned();
                        let length = head
                            .lines()
                            .find_map(|l| {
                                let (k, v) = l.split_once(':')?;
                                k.eq_ignore_ascii_case("content-length")
                                    .then(|| v.trim().parse::<usize>().ok())?
                            })
                            .unwrap_or(0);
                        while data.len() < end + 4 + length {
                            let n = stream.read(&mut buffer).unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            data.extend_from_slice(&buffer[..n]);
                        }
                        break (head, data[end + 4..].to_vec());
                    }
                };
                let mut line = head.lines().next().unwrap_or_default().split(' ');
                let (method, path) = (line.next().unwrap_or(""), line.next().unwrap_or(""));
                let (status, reply) = cloud(method, path, &head, &body, &account)
                    .unwrap_or_else(|| route(method, path, &body));
                // A redirect's body is where it goes.
                let location = (status == 302)
                    .then(|| format!("Location: {}\r\n", String::from_utf8_lossy(&reply)))
                    .unwrap_or_default();
                let kind = if path.ends_with("/tar") {
                    "application/gzip"
                } else {
                    "application/json"
                };
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 {status} OK\r\n{location}Content-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        reply.len()
                    )
                    .as_bytes(),
                );
                let _ = stream.write_all(&reply);
            });
        }
    });
    origin
}
/// `publisher:key-id:<hex>`, the `--dev-store-key` a development build takes.
pub fn key_spec() -> String {
    let key = SigningKey::from_bytes(&SEED).verifying_key();
    let hex: String = key.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
    format!("{PUBLISHER}:{KEY_ID}:{hex}")
}

/// For on-screen reviews: `cargo test -p pm-native-app serve_fixture_store --
/// --ignored --nocapture` serves until stopped and prints the arguments.
#[test]
#[ignore]
fn serve_fixture_store() {
    let port = std::env::var("PM_FIXTURE_STORE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(4170);
    let origin = serve(port);
    println!("--dev-store-origin {origin} --dev-store-key {}", key_spec());
    println!("emailed sign-in code: {CODE}");
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

struct Fixture {
    profile: tempfile::TempDir,
    kernel: Arc<Mutex<Kernel>>,
    origin: &'static str,
}
fn fixture() -> Fixture {
    Fixture {
        profile: tempfile::tempdir().unwrap(),
        kernel: Arc::new(Mutex::new(Kernel::new(KernelConfig::default()))),
        origin: Box::leak(serve(0).into_boxed_str()),
    }
}
fn services(f: &Fixture, signed: bool) -> Services {
    let origin = f.origin;
    let mut trust = TrustStore::default();
    trust
        .trust(
            PUBLISHER.into(),
            KEY_ID.into(),
            SigningKey::from_bytes(&SEED).verifying_key(),
        )
        .unwrap();
    Services {
        client: Arc::new(move || {
            let transport: Box<dyn pm_services::store::StoreTransport + Send> =
                Box::new(Loopback { origin });
            pm_services::store::StoreClient::new(origin, transport)
        }),
        kernel: f.kernel.clone(),
        profile: f.profile.path().canonicalize().unwrap(),
        source: signed.then(|| Arc::new(store_install::Source { origin, trust })),
    }
}
/// The window's transport is the system HTTP stack held to one origin; the
/// fixture reaches it the same way.
struct Loopback {
    origin: &'static str,
}
impl pm_services::store::StoreTransport for Loopback {
    fn send(
        &mut self,
        request: pm_services::store::Request,
        max: usize,
    ) -> pm_services::Result<pm_services::store::Response> {
        let post = request.method == pm_services::store::Method::PostVersions;
        let response = pm_compat_runner::net::send(
            pm_runtime_v8::FetchRequest {
                url: request.url.into(),
                method: if post { "POST" } else { "GET" }.into(),
                headers: vec![("Content-Type".into(), "application/json".into())],
                body: request
                    .body
                    .map(|b| serde_json::to_vec(&b).unwrap())
                    .unwrap_or_default(),
                max_response_bytes: max,
                deadline: Instant::now() + Duration::from_secs(10),
            },
            pm_compat_runner::net::Reach::Origin(self.origin),
        )
        .map_err(|e| {
            pm_services::ServiceError::message(
                pm_services::ServiceErrorCode::Offline,
                e.to_string(),
            )
        })?;
        Ok(pm_services::store::Response {
            status: response.status,
            body: response.body,
        })
    }
}
fn open<'a>(
    cx: &'a mut TestAppContext,
    services: Services,
) -> (Entity<StoreWindow>, &'a mut VisualTestContext) {
    let tokens = Tokens::new(Scheme::Light, Accessibility::default());
    cx.add_window_view(move |_, cx| StoreWindow::new(services, tokens, cx))
}
/// Waits on the worker threads for real; the test scheduler can't see them.
fn settle(
    view: &Entity<StoreWindow>,
    cx: &mut VisualTestContext,
    what: &str,
    done: impl Fn(&StoreWindow) -> bool,
) {
    let start = Instant::now();
    loop {
        cx.run_until_parked();
        if view.read_with(cx, |v, _| done(v)) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "never settled: {what}: {:?} {:?}",
            view.read_with(cx, |v, _| v.detail.as_ref().map(|d| d.1.clone())),
            view.read_with(cx, |v, _| v.message.clone())
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not on screen"));
    cx.simulate_click(bounds.center(), Modifiers::default());
    cx.run_until_parked();
}
fn ready<T>(load: &crate::store_window::Load<T>) -> bool {
    matches!(load, crate::store_window::Load::Ready(_))
}

#[::core::prelude::v1::test]
fn browse_open_an_extension_and_install_it_with_a_click() {
    let f = fixture();
    let mut cx = TestAppContext::single();
    let (view, cx) = open(&mut cx, services(&f, true));
    settle(&view, cx, "storefront", |v| {
        ready(&v.shelves) && ready(&v.library)
    });
    click(cx, "store-row-soft-glow");
    settle(&view, cx, "detail", |v| {
        v.detail.as_ref().is_some_and(|(_, load)| ready(load))
    });
    // Every release is listed, newest first; the action is Install.
    assert!(cx.debug_bounds("store-version-1.1.0").is_some());
    assert!(cx.debug_bounds("store-version-1.0.0").is_some());
    let action = view.read_with(cx, |v, _| {
        let crate::store_window::Load::Ready(detail) = &v.detail.as_ref().unwrap().1 else {
            unreachable!()
        };
        v.action(detail)
    });
    assert_eq!(action.label(), "Install");
    click(cx, "store-action");
    settle(&view, cx, "install", |v| {
        v.busy.is_none() && v.message.is_some()
    });
    assert_eq!(
        view.read_with(cx, |v, _| v.message.clone()).as_deref(),
        Some("Soft Glow is installed.")
    );
    // The install is a sealed Store folder that discovery finds with its marker.
    let found = pm_compat_runner::discovery::scan(&[f.profile.path().join("extensions")]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].store.as_ref().unwrap()["releaseId"], GLOW_NEW);
    settle(
        &view,
        cx,
        "library",
        |v| matches!(&v.library, crate::store_window::Load::Ready(items) if items.len() == 1),
    );
    // The page now reads Installed, with Uninstall beside it.
    assert!(cx.debug_bounds("store-uninstall").is_some());
    click(cx, "store-back");
    assert!(view.read_with(cx, |v, _| v.detail.is_none()));
}

#[::core::prelude::v1::test]
fn kinds_and_search_narrow_the_catalog() {
    let f = fixture();
    let mut cx = TestAppContext::single();
    let (view, cx) = open(&mut cx, services(&f, true));
    settle(&view, cx, "storefront", |v| ready(&v.shelves));
    click(cx, "store-section-Panels");
    settle(&view, cx, "panels", |v| {
        v.page == Page::Kind(2) && cx_rows(v) == ["shot-notes"]
    });
    view.update(cx, |v, cx| {
        v.set_page(Page::Discover, cx);
        v.set_query("glow", cx);
    });
    settle(&view, cx, "search", |v| cx_rows(v) == ["soft-glow"]);
    assert!(cx.debug_bounds("store-row-soft-glow").is_some());
    view.update(cx, |v, cx| v.set_query("nothing like it", cx));
    settle(&view, cx, "no results", |v| cx_rows(v).is_empty());
}
fn cx_rows(v: &StoreWindow) -> Vec<String> {
    v.rows_for_tests()
}

#[::core::prelude::v1::test]
fn library_shows_an_update_notice_and_updates_then_uninstall_archives() {
    let f = fixture();
    let profile = f.profile.path().canonicalize().unwrap();
    // An earlier install of 1.0.0.
    let source = services(&f, true).source.unwrap();
    store_install::install_from(
        &source,
        f.kernel.as_ref(),
        &profile,
        "native",
        GLOW_OLD,
        GLOW_REPO,
    )
    .unwrap();
    let mut cx = TestAppContext::single();
    let (view, cx) = open(&mut cx, services(&f, true));
    click(cx, "store-section-Library");
    settle(&view, cx, "update notice", |v| {
        matches!(&v.library, crate::store_window::Load::Ready(items)
            if items.len() == 1 && items[0].update.as_ref().is_some_and(|u| u.0 == "1.1.0"))
    });
    click(cx, "store-update-soft-glow");
    settle(&view, cx, "update", |v| {
        v.busy.is_none()
            && matches!(&v.library, crate::store_window::Load::Ready(items)
                if items.len() == 1 && items[0].version == "1.1.0" && items[0].update.is_none())
    });
    // The replaced 1.0.0 folder is archived, never deleted.
    assert_eq!(
        std::fs::read_dir(profile.join("extensions/.archive"))
            .unwrap()
            .count(),
        1
    );
    // Uninstall (what the confirmation's Uninstall runs) archives it too.
    view.update(cx, |v, cx| {
        v.uninstall(
            "soft-glow".into(),
            "Soft Glow".into(),
            Some(GLOW_REPO.into()),
            cx,
        )
    });
    settle(&view, cx, "uninstall", |v| {
        v.busy.is_none()
            && matches!(&v.library, crate::store_window::Load::Ready(items) if items.is_empty())
    });
    assert!(!profile.join("extensions/soft-glow").exists());
    assert_eq!(
        std::fs::read_dir(profile.join("extensions/.archive"))
            .unwrap()
            .count(),
        2
    );
}

#[::core::prelude::v1::test]
fn a_build_without_a_store_key_refuses_installs_and_says_why() {
    let f = fixture();
    let mut cx = TestAppContext::single();
    let (view, cx) = open(&mut cx, services(&f, false));
    settle(&view, cx, "storefront", |v| {
        ready(&v.shelves) && ready(&v.library)
    });
    click(cx, "store-row-shot-notes");
    settle(&view, cx, "detail", |v| {
        v.detail.as_ref().is_some_and(|(_, load)| ready(load))
    });
    click(cx, "store-action");
    assert!(view.read_with(cx, |v, _| v.busy.is_none()));
    assert!(!f.profile.path().join("extensions/shot-notes").exists());
    assert!(cx.debug_bounds("store-signing").is_some());
}
