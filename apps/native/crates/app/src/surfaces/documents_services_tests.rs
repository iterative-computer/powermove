//! Tests against the real pm-services types, compiled by documents_check.rs.
//! No service diagnostic or raw path is used as presentation copy.
use pm_documents_check::{Action, FileFailure, service_error};
use pm_services::{ServiceError, ServiceErrorCode, entrypoints::FileOpenFailure};
#[test]
fn every_real_service_code_matches_the_reviewed_wire_table() {
    for (code, wire) in [
        (ServiceErrorCode::Offline, "offline"),
        (ServiceErrorCode::NotFound, "not_found"),
        (ServiceErrorCode::TooLarge, "too_large"),
        (ServiceErrorCode::InvalidProject, "invalid_project"),
        (ServiceErrorCode::PermissionDenied, "permission_denied"),
        (ServiceErrorCode::DiskFull, "disk_full"),
        (ServiceErrorCode::InvalidInput, "invalid_input"),
        (ServiceErrorCode::InvalidData, "invalid_data"),
        (ServiceErrorCode::Unavailable, "unavailable"),
        (ServiceErrorCode::Unknown, "unknown"),
    ] {
        assert_eq!(serde_json::to_value(code).unwrap().as_str(), Some(wire));
        assert!(!service_error(wire, "Film").0.is_empty());
    }
}
#[test]
fn file_failure_preserves_existing_or_missing_path_and_never_shows_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let path = root.join("synthetic.pmv");
    for exists in [false, true] {
        if exists {
            std::fs::write(&path, b"synthetic invalid project").unwrap();
        }
        let failure = FileOpenFailure {
            path: path.clone(),
            code: ServiceErrorCode::InvalidProject,
        };
        let error = ServiceError::message(failure.code, "SYNTHETIC_DIAGNOSTIC_NEVER_UI");
        let code = serde_json::to_value(error.code).unwrap();
        let ui = FileFailure::from_service(
            failure.path.clone(),
            "Film",
            code.as_str().unwrap(),
            failure.path.exists(),
        );
        assert_eq!(ui.path, path);
        assert_eq!(ui.finder_path(), exists.then_some(path.as_path()));
        assert_eq!(
            ui.alert.message,
            "“Film” can’t be opened because it isn’t a Powermove project."
        );
        assert!(!format!("{:?}", ui.alert).contains("SYNTHETIC_DIAGNOSTIC_NEVER_UI"));
        assert_eq!(
            ui.alert
                .buttons
                .iter()
                .any(|b| b.action == Action::ShowInFinder),
            exists
        );
    }
}
#[test]
fn unknown_future_service_code_uses_generic_copy_without_leaking_code() {
    let (message, info) = service_error("future_code_with_diagnostics", "Film");
    assert_eq!(message, "The operation couldn’t be completed.");
    assert_eq!(
        info,
        "Try again. If it keeps happening, choose Window ▸ Error Log for details."
    );
}
