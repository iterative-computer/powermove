use super::test_storage;
use crate::{diagnostics, storage};
use serde_json::Value;
use std::fs;

#[test]
fn diagnostics_bundles_omit_log_text_and_arbitrary_profile_content() {
    let (temp, mut storage) = test_storage();
    struct Redactor;
    impl diagnostics::LogRedactor for Redactor {
        fn redact(&mut self, text: &str) -> String {
            text.replace("private text", "redacted")
        }
    }
    diagnostics::Diagnostics::log(
        &mut storage,
        &mut Redactor,
        diagnostics::Level::Error,
        "probe-failed",
        "private text\nline",
    )
    .unwrap();
    fs::write(temp.path().join("unrelated"), b"must stay private").unwrap();
    let bundle =
        String::from_utf8(diagnostics::Diagnostics::bundle(&storage, &[]).unwrap()).unwrap();
    assert!(bundle.contains("probe-failed"));
    assert!(
        !bundle.contains("private text")
            && !bundle.contains("redacted")
            && !bundle.contains("must stay private")
    );
}
#[test]
fn crash_reports_are_local_structured_and_recovery_is_archived() {
    let (_temp, mut storage) = test_storage();
    let mut crash = diagnostics::CrashRecord {
        id: "crash-1".into(),
        process: "engine".into(),
        reason: "panic".into(),
        recovered: false,
        recovered_document: None,
    };
    diagnostics::Diagnostics::crash(&mut storage, crash.clone()).unwrap();
    crash.recovered = true;
    diagnostics::Diagnostics::crash(&mut storage, crash.clone()).unwrap();
    assert_eq!(
        diagnostics::Diagnostics::crash_status(&storage, "crash-1").unwrap(),
        Some(crash)
    );
    let bundle: Value = serde_json::from_slice(
        &diagnostics::Diagnostics::bundle(&storage, &["crash-1".into()]).unwrap(),
    )
    .unwrap();
    assert_eq!(bundle["crashes"][0]["recovered"], true);
    assert_eq!(
        storage::generations(
            &storage
                .directory("settings", "services.crash.crash-1")
                .unwrap()
        )
        .unwrap()
        .len(),
        2
    );
}
#[test]
fn crash_and_log_fields_reject_freeform_path_or_token_codes() {
    let (_temp, mut storage) = test_storage();
    let crash = diagnostics::CrashRecord {
        id: "crash".into(),
        process: "/private/path".into(),
        reason: "token=anything".into(),
        recovered: false,
        recovered_document: None,
    };
    assert!(diagnostics::Diagnostics::crash(&mut storage, crash).is_err());
    assert!(
        diagnostics::Diagnostics::log(
            &mut storage,
            &mut diagnostics::OmitText,
            diagnostics::Level::Info,
            "../invalid",
            "hello"
        )
        .is_err()
    );
}

#[test]
fn old_crash_records_load_without_a_recovered_document_name() {
    let record = serde_json::json!({"id":"synthetic", "process":"kernel", "reason":"terminated", "recovered":false});
    let crash: diagnostics::CrashRecord = serde_json::from_value(record.clone()).unwrap();
    assert_eq!(crash.recovered_document, None);
    assert_eq!(serde_json::to_value(crash).unwrap(), record);
}

#[test]
fn recovered_document_name_is_local_durable_and_omitted_from_bundles() {
    let (temp, mut store) = test_storage();
    let original = diagnostics::CrashRecord {
        id: "synthetic".into(),
        process: "kernel".into(),
        reason: "terminated".into(),
        recovered: false,
        recovered_document: None,
    };
    diagnostics::Diagnostics::crash(&mut store, original.clone()).unwrap();
    let recovered = diagnostics::CrashRecord {
        recovered: true,
        recovered_document: Some("Synthetic Film 日本語".into()),
        ..original.clone()
    };
    diagnostics::Diagnostics::crash(&mut store, recovered.clone()).unwrap();
    let dir = store
        .directory("settings", "services.crash.synthetic")
        .unwrap();
    let versions = storage::generations(&dir).unwrap();
    assert_eq!(versions.len(), 2);
    let prior: Value = serde_json::from_slice(&fs::read(&versions[0].1).unwrap()).unwrap();
    assert_eq!(prior["value"], serde_json::to_value(original).unwrap());
    drop(store);
    let store = storage::ServiceStorage::open(temp.path().join("services")).unwrap();
    assert_eq!(
        diagnostics::Diagnostics::crash_status(&store, "synthetic").unwrap(),
        Some(recovered)
    );
    let bundle: Value = serde_json::from_slice(
        &diagnostics::Diagnostics::bundle(&store, &["synthetic".into()]).unwrap(),
    )
    .unwrap();
    assert!(bundle["crashes"][0].get("recovered_document").is_none());
    assert!(!bundle.to_string().contains("Synthetic Film"));
    assert_eq!(bundle["crashes"][0]["recovered"], true);
}
#[test]
fn error_log_rows_persist_in_order_and_clear_hides_older_rows() {
    let (_temp, mut storage) = test_storage();
    let mut omit = diagnostics::OmitText;
    struct Keep;
    impl diagnostics::LogRedactor for Keep {
        fn redact(&mut self, text: &str) -> String {
            text.into()
        }
    }
    for text in ["one", "two", "three"] {
        diagnostics::Diagnostics::log(
            &mut storage,
            &mut Keep,
            diagnostics::Level::Error,
            "Project",
            text,
        )
        .unwrap();
    }
    let rows = diagnostics::Diagnostics::recent(&storage, 2).unwrap();
    assert_eq!(
        rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
        ["two", "three"]
    );
    assert!(rows.iter().all(|r| r.at_ms > 0 && r.code == "Project"));
    std::thread::sleep(std::time::Duration::from_millis(2));
    diagnostics::Diagnostics::clear(&mut storage).unwrap();
    assert!(
        diagnostics::Diagnostics::recent(&storage, 10)
            .unwrap()
            .is_empty()
    );
    std::thread::sleep(std::time::Duration::from_millis(2));
    diagnostics::Diagnostics::log(
        &mut storage,
        &mut omit,
        diagnostics::Level::Warn,
        "Open",
        "x",
    )
    .unwrap();
    let rows = diagnostics::Diagnostics::recent(&storage, 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].text, "[text omitted]");
}
#[test]
fn crash_index_lists_each_crash_once_for_the_bundle() {
    let (_temp, mut storage) = test_storage();
    let crash = |id: &str| diagnostics::CrashRecord {
        id: id.into(),
        process: "shell".into(),
        reason: "unknown".into(),
        recovered: false,
        recovered_document: Some("Launch Film".into()),
    };
    diagnostics::Diagnostics::crash(&mut storage, crash("a")).unwrap();
    diagnostics::Diagnostics::crash(&mut storage, crash("b")).unwrap();
    diagnostics::Diagnostics::crash(&mut storage, crash("a")).unwrap();
    let ids = diagnostics::Diagnostics::crashes(&storage).unwrap();
    assert_eq!(ids, ["a", "b"]);
    let bundle =
        String::from_utf8(diagnostics::Diagnostics::bundle(&storage, &ids).unwrap()).unwrap();
    assert!(bundle.contains("\"shell\"") && !bundle.contains("Launch Film"));
}
