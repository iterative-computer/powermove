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
