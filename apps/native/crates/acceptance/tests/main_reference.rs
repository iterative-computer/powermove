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
    assert_eq!(r.exit_code(true), 2);
    let retired = r
        .cases
        .iter()
        .find(|c| c.id == "MAIN.W072.contract")
        .unwrap();
    assert!(retired.reason.contains("removed action stays absent"));
    assert_eq!(
        retired.blocked_by.as_deref(),
        Some("delta-data:archived-web-export-preferences")
    );
    assert_eq!(r.counts(), (1701, 0, 2644));
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
    for id in ["MAIN.MCP.edit_3d.catalog", "MAIN.MCP.tools.identity"] {
        assert_eq!(
            r.cases.iter().find(|c| c.id == id).unwrap().status,
            Status::Pass,
            "{id}"
        );
    }
    for id in [
        "MAIN.curve.set",
        "MAIN.KEY.133.definition",
        "MAIN.KEY.196.definition",
        "MAIN.KEY.196.route-normal",
        "MAIN.KEY.196.route-held",
        "MAIN.KEY.196.route-extra-modifier",
        "MAIN.KEY.196.route-input",
        "MAIN.KEY.196.route-contenteditable",
        "MAIN.KEY.196.route-prior-decline",
        "MAIN.KEY.196.route-key-up",
        "MAIN.KEY.198.definition",
        "MAIN.MCP.export_video.catalog",
        "MAIN.MCP.get_project_state.catalog",
        "MAIN.MCP.project_memory.catalog",
        "MAIN.A07.project-memory-call",
        "MAIN.A07.memory-index",
    ] {
        assert_eq!(
            r.cases.iter().find(|case| case.id == id).unwrap().status,
            Status::Pass,
            "{id}"
        );
    }
    assert_eq!(r.compatibility.as_ref().unwrap().counts(), (1426, 0, 3178));
    for id in ["A506", "W007.mcp-utf16-ingress"] {
        assert!(
            r.compatibility_required_ids
                .iter()
                .any(|required| required == id)
        );
        assert_eq!(
            r.compatibility
                .as_ref()
                .unwrap()
                .cases
                .iter()
                .find(|c| c.id == id)
                .unwrap()
                .status,
            Status::Fail
        );
    }
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
    // The tracked census is configured with the actual file-backed source
    // worker; this kernel-only run must keep that one worker-dependent body
    // blocked. Every other row remains identical.
    let rows = |text: &str| {
        pm_acceptance::catalog::csv_rows(text)
            .unwrap()
            .into_iter()
            .filter(|row| row[0] != "MAIN.MCP.fork_builtin_extension.behavior")
            .collect::<Vec<_>>()
    };
    assert_eq!(
        rows(&r.blockers_csv()),
        rows(include_str!("../data/remaining-blockers.csv"))
    );
    assert_eq!(
        r.cases
            .iter()
            .find(|c| c.id == "MAIN.MCP.fork_builtin_extension.behavior")
            .unwrap()
            .status,
        Status::BlockedBy
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
    assert_eq!(output.status.code(), Some(0));
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

#[test]
fn old_ui_gates_stay_historical_while_required_legacy_inputs_gate_strict_mode() {
    let catalog = Catalog::embedded().unwrap();
    let mut main = main_reference::run(
        &catalog,
        &Filter {
            workflow: None,
            case: Some("MAIN.MCP.list_projects.catalog".into()),
        },
        &[],
        None,
    )
    .unwrap();
    let legacy = pm_acceptance::runner::run(
        &catalog,
        &Filter {
            workflow: None,
            case: Some("K001.normal".into()),
        },
        &[],
    )
    .unwrap();
    main.compatibility = Some(legacy);
    assert_eq!(
        main.exit_code(true),
        0,
        "superseded UI gate must not restore the tag parity target"
    );
    let legacy = pm_acceptance::runner::run(
        &catalog,
        &Filter {
            workflow: None,
            case: Some("W037.v8-reload".into()),
        },
        &[],
    )
    .unwrap();
    main.compatibility = Some(legacy);
    main.compatibility_required_ids = vec!["W037.v8-reload".into()];
    assert_eq!(main.exit_code(true), 2);
    main.compatibility.as_mut().unwrap().cases[0].status = Status::Fail;
    assert_eq!(main.exit_code(false), 1);
}

#[test]
fn studio_overlays_preserve_all_main_operations_and_unrelated_contracts() {
    let bare = &main_reference::snapshot()["catalog"];
    let expected = main_reference::expected_catalog();
    assert_eq!(
        bare["tools"].as_array().unwrap().len(),
        expected["tools"].as_array().unwrap().len()
    );
    for tool in bare["tools"].as_array().unwrap() {
        let actual = expected["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == tool["name"])
            .unwrap();
        if tool["name"] == "edit_3d" {
            let base = &tool["inputSchema"];
            let studio = &actual["inputSchema"];
            for (field, schema) in base["properties"].as_object().unwrap() {
                if field == "operation" {
                    for operation in schema["enum"].as_array().unwrap() {
                        assert!(
                            studio["properties"][field]["enum"]
                                .as_array()
                                .unwrap()
                                .contains(operation)
                        );
                    }
                } else {
                    assert_eq!(schema, &studio["properties"][field], "{field}");
                }
            }
            for field in ["required", "additionalProperties", "type"] {
                assert_eq!(base[field], studio[field]);
            }
            assert_eq!(
                studio["properties"]["operation"]["enum"]
                    .as_array()
                    .unwrap()
                    .len(),
                19
            );
        } else {
            assert_eq!(tool, actual);
        }
    }
    for field in [
        "inspection",
        "app",
        "instructions",
        "commandFields",
        "easePresets",
    ] {
        assert_eq!(bare[field], expected[field]);
    }
}
