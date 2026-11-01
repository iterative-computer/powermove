use pm_mcp::{
    media::MediaRegistry,
    store::{self, StoreRead},
};
use pm_mcp_backends::{NativeMediaRegistry, NativeStoreReads};
use pm_promedia::media::{Artifact, MediaCatalog, MediaInfo, Rational};
use pm_services::{
    ServiceError, ServiceErrorCode,
    store::{Method, Request, Response, StoreTransport},
};
use std::sync::{Arc, Mutex};

type RequestLog = Arc<Mutex<Vec<(String, usize)>>>;

struct Transport {
    requests: RequestLog,
    fail: bool,
    oversized: bool,
}
impl StoreTransport for Transport {
    fn send(&mut self, request: Request, limit: usize) -> pm_services::Result<Response> {
        assert_eq!(request.method, Method::Get);
        assert!(request.body.is_none());
        self.requests
            .lock()
            .unwrap()
            .push((request.url.to_string(), limit));
        if self.fail {
            return Err(ServiceError::message(
                ServiceErrorCode::Offline,
                "private credential diagnostic must not cross MCP",
            ));
        }
        Ok(Response {
            status: 200,
            body: if self.oversized {
                vec![b'x'; limit + 1]
            } else {
                br#"{"items":[],"nextCursor":null}"#.to_vec()
            },
        })
    }
}
fn transport() -> (Transport, RequestLog) {
    let requests = Arc::default();
    (
        Transport {
            requests: Arc::clone(&requests),
            fail: false,
            oversized: false,
        },
        requests,
    )
}
#[test]
fn store_binding_uses_actual_native_service_transport_and_query_encoding() {
    let (t, requests) = transport();
    let host = NativeStoreReads::new("https://store.example", t).unwrap();
    assert_eq!(
        store::search(&host, &serde_json::json!({"query":"a & 世界"}))
            .unwrap()
            .to_string(),
        r#"{"items":[],"nextCursor":null}"#
    );
    assert_eq!(
        *requests.lock().unwrap(),
        vec![(
            "https://store.example/v1/store/extensions?q=a+%26+%E4%B8%96%E7%95%8C".into(),
            store::JSON_LIMIT
        )]
    );
}
#[test]
fn store_origin_is_validated_by_pm_services() {
    for origin in [
        "http://store.example",
        "https://store.example/path",
        "https://name:password@store.example",
        "https://store.example?token=value",
    ] {
        let (t, _) = transport();
        assert!(NativeStoreReads::new(origin, t).is_err());
    }
}
#[test]
fn service_failures_are_classified_without_exposing_diagnostics() {
    let (mut t, _) = transport();
    t.fail = true;
    let host = NativeStoreReads::new("https://store.example", t).unwrap();
    let e = host
        .get(store::Request {
            path: "/v1/store/extensions".into(),
            query: vec![],
            max_bytes: 64,
        })
        .err()
        .unwrap();
    assert_eq!(e.code, "store_error");
    assert!(e.message.contains("Offline"));
    assert!(!e.message.contains("private"));
    assert!(!e.message.contains("credential"));
}
#[test]
fn binding_refuses_cross_origin_routes_and_unbounded_reads() {
    let (t, requests) = transport();
    let host = NativeStoreReads::new("https://store.example", t).unwrap();
    for path in [
        "//other.example/path",
        "/v1/store/extensions?secret=bad",
        "/v1/store/extensions#fragment",
        "/unrelated",
        "/v1/store/../../installs",
        "/v1/store/%2e%2e/%2e%2e/installs",
    ] {
        assert!(
            host.get(store::Request {
                path: path.into(),
                query: vec![],
                max_bytes: 64
            })
            .is_err()
        );
    }
    assert!(
        host.get(store::Request {
            path: "/v1/store/extensions".into(),
            query: vec![],
            max_bytes: store::JSON_LIMIT + 1
        })
        .is_err()
    );
    assert!(requests.lock().unwrap().is_empty());
    let (mut t, _) = transport();
    t.oversized = true;
    let host = NativeStoreReads::new("https://store.example", t).unwrap();
    assert!(
        host.get(store::Request {
            path: "/v1/store/extensions".into(),
            query: vec![],
            max_bytes: 64
        })
        .is_err()
    );
}
fn info() -> MediaInfo {
    MediaInfo {
        width: 640,
        height: 360,
        frame_rate: Rational {
            numerator: 24,
            denominator: 1,
        },
        time_base: Rational {
            numerator: 1,
            denominator: 24000,
        },
        start_tick: 100,
        duration_ticks: 48000,
        alpha: false,
        audio_channels: 2,
        audio_sample_rate: 48000,
        colour: "synthetic-bt709".into(),
    }
}
#[test]
fn native_catalog_metadata_is_not_promoted_to_online_media() {
    let registry = NativeMediaRegistry::default();
    let mut catalog = MediaCatalog::new("project".into()).unwrap();
    catalog.register("asset".into(), [1; 32], info()).unwrap();
    let catalog = Arc::new(Mutex::new(catalog));
    registry.bind("project".into(), catalog.clone()).unwrap();
    let state = registry.state("project", "asset");
    assert!(!state.available);
    assert_eq!(state.duration, Some(2.));
    assert_eq!(state.audio_duration, Some(2.));
    assert!(state.has_audio);
    assert!(!registry.state("foreign", "asset").available);
    assert!(!registry.state("project", "unknown").available);
    catalog
        .lock()
        .unwrap()
        .relink_original(
            "asset",
            Artifact {
                path: "/synthetic/explicit-original.mov".into(),
                fingerprint: [1; 32],
                info: info(),
            },
        )
        .unwrap();
    assert!(registry.state("project", "asset").available);
    catalog
        .lock()
        .unwrap()
        .set_original_offline("asset")
        .unwrap();
    assert!(!registry.state("project", "asset").available);
    registry.retire("project").unwrap();
    assert_eq!(registry.state("project", "asset").duration, None);
}
