use crate::error::Result;
use crate::store;
use serde_json::{Value, json};
use std::collections::VecDeque;

const REPO: &str = "11111111-1111-4111-8111-111111111111";
const OLD: &str = "22222222-2222-4222-8222-222222222222";
const NEW: &str = "33333333-3333-4333-8333-333333333333";
#[derive(Default)]
struct TransportDouble {
    requests: Vec<store::Request>,
    responses: VecDeque<store::Response>,
}
impl TransportDouble {
    fn json(values: Vec<Value>) -> Self {
        Self {
            responses: values
                .into_iter()
                .map(|value| store::Response {
                    status: 200,
                    body: serde_json::to_vec(&value).unwrap(),
                })
                .collect(),
            ..Default::default()
        }
    }
}
impl store::StoreTransport for TransportDouble {
    fn send(&mut self, request: store::Request, max: usize) -> Result<store::Response> {
        assert_eq!(max, 4 * 1024 * 1024);
        self.requests.push(request);
        self.responses
            .pop_front()
            .ok_or_else(|| crate::ServiceError::from(anyhow::anyhow!("unexpected request")))
    }
}
fn installed() -> store::InstalledRelease {
    store::InstalledRelease {
        repo_id: REPO.into(),
        release_id: OLD.into(),
    }
}
fn version_response() -> Value {
    json!({"items":[{"repoId":REPO,"state":"ok","current":{"yanked":false},"ownerPublisherId":REPO,"handle":"synthetic","slug":"synthetic-extension","latest":{"releaseId":NEW,"version":"1.2.3","treeSha":"a".repeat(40),"tarSha256":"b".repeat(64),"apiVersion":3}}]})
}
fn store_client(responses: Vec<Value>) -> store::StoreClient<TransportDouble> {
    store::StoreClient::new("https://registry.invalid", TransportDouble::json(responses)).unwrap()
}
#[test]
fn search_uses_frozen_route_categories_and_encoded_query() {
    let listing = json!({"repoId":REPO,"owner":{"handle":"synthetic"},"slug":"synthetic-extension","iconUrl":format!("/v1/store/icons/{}.png","a".repeat(64)),"future":{"kept":true}});
    let mut client = store_client(vec![json!({"items":[listing],"nextCursor":"next"})]);
    let result = client
        .search(store::Search {
            query: Some("noise & colour/日本語".into()),
            category: Some("effects".into()),
            sort: Some("name".into()),
            limit: Some(50),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(result.next_cursor.as_deref(), Some("next"));
    assert_eq!(result.items[0]["future"]["kept"], true);
    assert!(
        result.items[0]["iconUrl"]
            .as_str()
            .unwrap()
            .starts_with("https://registry.invalid/v1/store/icons/")
    );
    let request = &client.transport.requests[0];
    assert_eq!(request.method, store::Method::Get);
    assert_eq!(request.url.path(), "/v1/store/extensions");
    assert!(
        request
            .url
            .query_pairs()
            .any(|(key, value)| key == "q" && value == "noise & colour/日本語")
    );
}
#[test]
fn unsafe_icon_is_replaced_with_null() {
    for icon in [
        "https://other.invalid/icon.png",
        "/v1/store/icons/bad.png",
        "https://registry.invalid/v1/store/icons/a.png?credentials=x",
    ] {
        let mut client = store_client(vec![
            json!({"items":[{"repoId":REPO,"owner":{"handle":"synthetic"},"slug":"synthetic-extension","iconUrl":icon}],"nextCursor":null}),
        ]);
        assert!(client.search(store::Search::default()).unwrap().items[0]["iconUrl"].is_null());
    }
}
#[test]
fn store_rejects_credential_origins_and_invalid_queries_before_transport() {
    for origin in [
        "http://registry.invalid",
        "https://user:pass@registry.invalid",
        "https://registry.invalid/path",
        "https://registry.invalid?token=x",
    ] {
        assert!(store::StoreClient::new(origin, TransportDouble::default()).is_err());
    }
    let mut client = store_client(vec![]);
    assert!(
        client
            .search(store::Search {
                limit: Some(51),
                ..Default::default()
            })
            .is_err()
    );
    assert!(
        client
            .search(store::Search {
                category: Some("effect".into()),
                ..Default::default()
            })
            .is_err()
    );
    assert!(client.transport.requests.is_empty());
}
#[test]
fn store_http_errors_do_not_echo_server_bodies() {
    let transport = TransportDouble {
        responses: VecDeque::from([store::Response {
            status: 401,
            body: b"synthetic private server text".to_vec(),
        }]),
        ..Default::default()
    };
    let mut client = store::StoreClient::new("https://registry.invalid", transport).unwrap();
    let error = client
        .search(store::Search::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("401") && !error.contains("private"));
}
#[test]
fn store_rejects_oversized_body_and_invalid_json() {
    for body in [vec![b'x'; 4 * 1024 * 1024 + 1], b"bad metadata".to_vec()] {
        let transport = TransportDouble {
            responses: VecDeque::from([store::Response { status: 200, body }]),
            ..Default::default()
        };
        let mut client = store::StoreClient::new("https://registry.invalid", transport).unwrap();
        assert!(client.search(store::Search::default()).is_err());
    }
}
#[test]
fn versions_validate_batch_membership_and_preserve_removed_state() {
    let mut response = version_response();
    response["items"][0]["state"] = json!("removed");
    response["items"][0]["latest"] = Value::Null;
    let mut client = store_client(vec![response]);
    let result = client.versions(&[installed()]).unwrap();
    assert_eq!(result[0].state, "removed");
    let request = &client.transport.requests[0];
    assert_eq!(request.method, store::Method::PostVersions);
    assert_eq!(request.url.path(), "/v1/store/versions");
    assert_eq!(request.body.as_ref().unwrap()["items"][0]["repoId"], REPO);
    let mut response = version_response();
    response["items"][0]["repoId"] = json!(OLD);
    assert!(
        store_client(vec![response])
            .versions(&[installed()])
            .is_err()
    );
    assert!(
        store_client(vec![])
            .versions(&vec![installed(); 201])
            .is_err()
    );
}
#[test]
fn store_errors_have_typed_codes_without_server_body_copy() {
    use crate::ServiceErrorCode as Code;
    assert_eq!(
        store::StoreClient::new("http://registry.invalid", TransportDouble::default())
            .err()
            .unwrap()
            .code,
        Code::InvalidInput
    );
    for (status, code) in [
        (403, Code::PermissionDenied),
        (404, Code::NotFound),
        (413, Code::TooLarge),
        (503, Code::Offline),
        (500, Code::Unavailable),
    ] {
        let mut client = store::StoreClient::new(
            "https://registry.invalid",
            TransportDouble {
                responses: VecDeque::from([store::Response {
                    status,
                    body: b"synthetic private response".to_vec(),
                }]),
                ..Default::default()
            },
        )
        .unwrap();
        let error = client.search(store::Search::default()).unwrap_err();
        assert_eq!(error.code, code);
        assert!(!error.to_string().contains("synthetic private response"));
    }
    for (body, code) in [
        (vec![b'x'; 4 * 1024 * 1024 + 1], Code::TooLarge),
        (b"invalid JSON".to_vec(), Code::InvalidData),
    ] {
        let mut client = store::StoreClient::new(
            "https://registry.invalid",
            TransportDouble {
                responses: VecDeque::from([store::Response { status: 200, body }]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            client.search(store::Search::default()).unwrap_err().code,
            code
        );
    }
}
#[test]
fn nullable_store_fields_are_required_in_wire_metadata() {
    let mut client = store_client(vec![json!({"items":[]})]);
    assert!(client.search(store::Search::default()).is_err());
    let mut response = version_response();
    response["items"][0]
        .as_object_mut()
        .unwrap()
        .remove("latest");
    assert!(
        store_client(vec![response])
            .versions(&[installed()])
            .is_err()
    );
}
fn listing_dto(slug: &str) -> Value {
    json!({"repoId":REPO,"owner":{"id":REPO,"handle":"synthetic","tombstoned":false,"verified":true},
        "slug":slug,"name":"Synthetic","tagline":"A line","category":"effects","iconUrl":null,
        "latest":{"id":NEW,"version":"1.2.3","publishedAt":"2026-11-01T00:00:00Z","apiVersion":3,"yankedAt":null},
        "permissions":["network","full-access"],"installCount":7})
}
#[test]
fn browse_and_detail_use_their_routes_and_admit_every_listing() {
    let mut client = store_client(vec![
        json!({"sections":[{"id":"featured","title":"Featured","items":[listing_dto("one")]}]}),
    ]);
    let shelves = client.browse().unwrap();
    assert_eq!(shelves[0].title, "Featured");
    assert_eq!(client.transport.requests[0].url.path(), "/v1/store/browse");
    // A listing without a publisher is refused, not shown half-empty.
    let mut bad = listing_dto("two");
    bad["owner"] = Value::Null;
    let mut client = store_client(vec![
        json!({"sections":[{"id":"new","title":"New","items":[bad]}]}),
    ]);
    assert!(client.browse().is_err());

    let mut detail = listing_dto("synthetic-extension");
    detail["about"] = Value::Null;
    detail["releases"] = json!([
        {"id":NEW,"version":"1.2.3","publishedAt":"2026-11-01T00:00:00Z","yankedAt":null,"notes":"Faster","apiVersion":3,
         "manifest":{"description":"From the manifest","contributes":["effects","palette"],"vars":[{"key":"API_KEY","label":"Key"}],"links":["https://example.com"]}},
        {"id":OLD,"version":"1.0.0","publishedAt":"2026-10-01T00:00:00Z","yankedAt":"2026-10-02T00:00:00Z","notes":null,"apiVersion":3,"manifest":{}}
    ]);
    let mut client = store_client(vec![detail]);
    let value = client.detail("synthetic", "synthetic-extension").unwrap();
    assert_eq!(
        client.transport.requests[0].url.path(),
        "/v1/store/x/synthetic/synthetic-extension"
    );
    let detail = crate::surfaces::store::Detail::from_value(&value).unwrap();
    assert_eq!(detail.about.as_deref(), Some("From the manifest"));
    assert!(detail.declares_values);
    assert_eq!(detail.link_hosts, ["example.com"]);
    assert_eq!(detail.releases.len(), 2);
    assert!(detail.releases[1].withdrawn);
    assert_eq!(detail.listing.coordinate(), "synthetic/synthetic-extension");
    assert!(detail.listing.verified);
    assert_eq!(
        crate::surfaces::store::includes_text(&detail.contributes),
        "Effects, Command palette"
    );
    assert_eq!(
        crate::surfaces::store::permission_lines(&detail.listing.permissions),
        [
            ("Needs full access to Powermove", true),
            ("Uses the network", false)
        ]
    );
    // Handles and slugs are checked before anything is sent.
    let mut client = store_client(vec![]);
    assert!(client.detail("../x", "synthetic").is_err());
    assert!(client.detail("synthetic", "a/b").is_err());
    assert!(client.transport.requests.is_empty());
}
#[test]
fn relative_dates_read_like_the_shipped_store() {
    use crate::surfaces::store::{civil_days, civil_from_days, relative_date};
    let today = (2026, 11, 9);
    assert_eq!(relative_date("2026-11-09T10:00:00Z", today), "Today");
    assert_eq!(relative_date("2026-11-08T10:00:00Z", today), "Yesterday");
    assert_eq!(relative_date("2026-11-05T10:00:00Z", today), "4 days ago");
    assert_eq!(relative_date("2026-09-19T10:00:00Z", today), "Sep 19");
    assert_eq!(relative_date("2025-09-19T10:00:00Z", today), "Sep 19, 2025");
    assert_eq!(relative_date("nonsense", today), "");
    assert_eq!(civil_from_days(civil_days(today)), today);
    assert_eq!(civil_days((1970, 1, 1)), 0);
}
#[cfg(debug_assertions)]
#[test]
fn development_builds_take_a_loopback_fixture_store_only() {
    assert!(store::StoreClient::new("http://127.0.0.1:4100/", TransportDouble::default()).is_ok());
    for origin in [
        "http://localhost:4100/",
        "http://10.0.0.1/",
        "http://example.com/",
    ] {
        assert!(store::StoreClient::new(origin, TransportDouble::default()).is_err());
    }
}
