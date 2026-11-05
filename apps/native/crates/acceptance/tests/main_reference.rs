use pm_acceptance::{
    catalog::Catalog,
    main_reference::{self, REFERENCE},
    runner::{Filter, Status},
};
use std::collections::BTreeSet;
#[test]
fn source_snapshots_pin_main_without_discarding_legacy_rosters() {
    assert_eq!(main_reference::snapshot()["reference"], REFERENCE);
    assert_eq!(
        main_reference::snapshot()["catalog"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        68
    );
    assert_eq!(
        main_reference::snapshot()["bindings"]
            .as_array()
            .unwrap()
            .len(),
        198
    );
    assert_eq!(
        main_reference::declarations()["cases"]
            .as_array()
            .unwrap()
            .len(),
        719
    );
    assert_eq!(main_reference::deltas().len(), 57);
    let agents = main_reference::deltas()
        .iter()
        .filter(|r| r["id"].as_str().unwrap().starts_with('A'))
        .count();
    assert_eq!(agents, 10);
    for row in main_reference::deltas() {
        assert_eq!(row["reference"], REFERENCE);
        assert!(!row["owner"].as_str().unwrap().is_empty());
    }
}
#[test]
fn active_reference_reports_actual_main_failures_and_keeps_compatibility_separate() {
    let r =
        main_reference::run(&Catalog::embedded().unwrap(), &Filter::default(), &[], None).unwrap();
    assert_eq!(r.exit_code(true), 1);
    assert_eq!(r.counts(), (1651, 32, 2661));
    assert_eq!(r.specs.len(), 397);
    assert_eq!(r.specs.iter().filter(|s| s.in_legacy_396).count(), 396);
    assert_eq!(r.specs.iter().filter(|s| s.in_370_roster).count(), 370);
    assert_eq!(r.specs.iter().filter(|s| s.in_main).count(), 314);
    assert_eq!(
        r.cases
            .iter()
            .filter(|c| c.kind == "main-e2e-declaration")
            .count(),
        719
    );
    assert_eq!(
        r.cases
            .iter()
            .filter(|c| c.kind == "main-workflow-contract" && c.status == Status::BlockedBy)
            .count(),
        83
    );
    for id in [
        "MAIN.MCP.export_video.catalog",
        "MAIN.MCP.get_project_state.catalog",
        "MAIN.MCP.project_memory.catalog",
        "MAIN.A07.project-memory-call",
        "MAIN.A07.memory-index",
        "MAIN.curve.set",
    ] {
        assert_eq!(
            r.cases.iter().find(|c| c.id == id).unwrap().status,
            Status::Fail,
            "{id}"
        );
    }
    assert_eq!(r.compatibility.as_ref().unwrap().counts(), (1423, 1, 3179));
    let blocked = r
        .cases
        .iter()
        .filter(|c| c.status == Status::BlockedBy)
        .map(|c| c.id.as_str())
        .collect::<BTreeSet<_>>();
    let grouped = r
        .blocker_groups
        .iter()
        .flat_map(|g| g.case_ids.iter().map(String::as_str))
        .collect::<Vec<_>>();
    assert_eq!(grouped.len(), blocked.len());
    assert_eq!(grouped.into_iter().collect::<BTreeSet<_>>(), blocked);
    assert_eq!(
        pm_acceptance::catalog::csv_rows(&r.blockers_csv()).unwrap(),
        pm_acceptance::catalog::csv_rows(include_str!("../data/remaining-blockers.csv")).unwrap()
    );
    assert_eq!(
        pm_acceptance::catalog::csv_rows(&r.e2e_csv()).unwrap(),
        pm_acceptance::catalog::csv_rows(include_str!("../data/e2e-map.csv")).unwrap()
    );
    let expected_failures =
        pm_acceptance::catalog::csv_rows(include_str!("../data/failures.csv")).unwrap();
    let failure_ids = expected_failures
        .iter()
        .skip(1)
        .map(|row| row[0].as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        failure_ids,
        r.cases
            .iter()
            .filter(|c| c.status == Status::Fail)
            .map(|c| c.id.as_str())
            .collect::<BTreeSet<_>>()
    );
    let tracked = pm_acceptance::catalog::csv_rows(include_str!("../data/e2e-map.csv")).unwrap();
    assert_eq!(tracked.len(), 398);
    for spec in &r.specs {
        assert!(tracked.iter().skip(1).any(|row| row[0] == spec.file));
    }
}
#[test]
fn main_filtered_pass_blocker_and_invalid_filters_are_distinct() {
    let c = Catalog::embedded().unwrap();
    let one = |id: &str| {
        main_reference::run(
            &c,
            &Filter {
                workflow: None,
                case: Some(id.into()),
            },
            &[],
            None,
        )
    };
    let pass = one("MAIN.MCP.list_projects.catalog").unwrap();
    assert_eq!(pass.counts(), (1, 0, 0));
    assert_eq!(pass.exit_code(true), 0);
    assert!(pass.compatibility.is_none());
    let blocked = one("MAIN.DELTA.A01").unwrap();
    assert_eq!(blocked.counts(), (0, 0, 1));
    assert_eq!(blocked.exit_code(true), 2);
    assert!(one("A517").is_err());
    assert!(one("MAIN.nonexistent").is_err());
}
#[test]
fn cli_defaults_to_main_and_requires_explicit_legacy_selection() {
    let binary = env!("CARGO_BIN_EXE_pm-acceptance");
    let output = std::process::Command::new(binary)
        .args(["--case", "MAIN.MCP.project_memory.catalog", "--strict"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["reference"], REFERENCE);
    let output = std::process::Command::new(binary)
        .args(["--case", "MAIN.DELTA.A03", "--strict"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn preexisting_id_consumers_still_run_with_explicit_compatibility_metadata() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pm-acceptance"))
        .args(["--case", "W071.fractional-rate"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["reference"], main_reference::LEGACY_REFERENCE);
    assert_eq!(value["purpose"], "legacy-compatibility");
}
