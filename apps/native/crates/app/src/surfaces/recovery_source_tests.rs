//! Integration target registered by recovery_check.rs's disposable manifest.
//! It proves the serialized seam against real backing crate types, while the
//! app's production manifest does not need new cross-lane dependencies.
use pm_recovery_surfaces::{recovery::panels::*, recovery_wire};
use serde_json::to_value;
#[test]
fn every_real_store_status_and_failure_maps_to_the_surface() {
    use pm_store_host::view as host;
    for (source, expected) in [
        (host::ViewStatus::Starting, ViewStatus::Starting),
        (host::ViewStatus::Ready, ViewStatus::Ready),
        (host::ViewStatus::Sealing, ViewStatus::Sealing),
        (host::ViewStatus::Stopping, ViewStatus::Stopping),
        (host::ViewStatus::StopFailed, ViewStatus::StopFailed),
        (
            host::ViewStatus::WaitingToRestart,
            ViewStatus::WaitingToRestart,
        ),
        (host::ViewStatus::Disabled, ViewStatus::Disabled),
        (host::ViewStatus::Closed, ViewStatus::Closed),
    ] {
        assert_eq!(
            recovery_wire::view_status(&to_value(source).unwrap()),
            Some(expected)
        );
    }
    for (source, expected) in [
        (host::Failure::Crash, Failure::Crash),
        (host::Failure::Hang, Failure::Hang),
        (host::Failure::UserStop, Failure::UserStop),
        (host::Failure::ReadyTimeout, Failure::ReadyTimeout),
        (host::Failure::PlatformFailure, Failure::PlatformFailure),
    ] {
        assert_eq!(
            recovery_wire::view_failure(&to_value(source).unwrap()),
            Some(expected)
        );
    }
}
#[test]
fn every_real_service_error_code_has_catalog_copy() {
    use pm_services::error::ServiceErrorCode::*;
    for code in [
        Offline,
        NotFound,
        TooLarge,
        InvalidProject,
        PermissionDenied,
        DiskFull,
        InvalidInput,
        InvalidData,
        Unavailable,
        Unknown,
    ] {
        let mapped = recovery_wire::service_code(&to_value(code).unwrap()).unwrap();
        assert!(
            !pm_recovery_surfaces::recovery::notices::service_copy(mapped, Some("Synthetic"))
                .message
                .is_empty()
        );
    }
}
#[test]
fn crash_record_recovery_name_is_the_only_display_input() {
    let mut record = pm_services::diagnostics::CrashRecord {
        id: "synthetic".into(),
        process: "engine".into(),
        reason: "out-of-memory".into(),
        recovered: true,
        recovered_document: Some("Launch Film".into()),
    };
    let banner = recovery_wire::crash_banner(&to_value(&record).unwrap(), false).unwrap();
    assert_eq!(
        banner.text,
        "Powermove recovered “Launch Film” after an unexpected quit."
    );
    assert!(!banner.text.contains("out-of-memory"));
    record.recovered = false;
    assert!(recovery_wire::crash_banner(&to_value(record).unwrap(), false).is_none());
}
#[test]
fn malformed_or_unknown_source_wire_is_rejected() {
    for value in [
        serde_json::json!(null),
        serde_json::json!({"reason":"private"}),
        serde_json::json!("new-unrecognized-state"),
    ] {
        assert!(recovery_wire::view_status(&value).is_none());
        assert!(recovery_wire::view_failure(&value).is_none());
        assert!(recovery_wire::service_code(&value).is_none());
        assert!(recovery_wire::crash_banner(&value, false).is_none());
    }
}
