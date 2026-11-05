use pm_acceptance::{
    catalog::{Catalog, csv_rows},
    runner::{Filter, Status, execute, run},
};
#[test]
fn full_run_keeps_partial_evidence_separate_and_maps_every_spec() {
    let c = Catalog::embedded().unwrap();
    let r = run(&c, &Filter::default(), &[]).unwrap();
    assert_eq!(r.workflows.len(), 83);
    assert_eq!(r.specs.len(), 396);
    assert_eq!(r.counts(), (1424, 2, 3178));
    assert_eq!(r.exit_code(false), 1);
    assert_eq!(r.exit_code(true), 1);
    assert_eq!(
        r.workflows
            .iter()
            .filter(|w| w.status == Status::BlockedBy)
            .count(),
        81
    );
    assert_eq!(
        r.workflows.iter().find(|w| w.id == "W071").unwrap().status,
        Status::BlockedBy
    );
    assert_eq!(
        r.cases
            .iter()
            .filter(|x| x.kind == "original" && x.status == Status::Pass)
            .count(),
        21
    );
    assert_eq!(
        r.cases
            .iter()
            .filter(|x| x.kind == "routing-probe" && x.status == Status::Pass)
            .count(),
        1365
    );
    assert_eq!(
        r.cases.iter().filter(|x| x.kind == "binding").count(),
        195 * 9
    );
    assert!(
        r.cases
            .iter()
            .filter(|x| x.kind == "binding")
            .all(|x| x.status == Status::BlockedBy && x.needs_macos)
    );
    assert!(
        r.cases
            .iter()
            .filter(|x| x.kind == "test-double-probe")
            .all(|x| x.needs_macos)
    );
    assert_eq!(
        r.specs.iter().filter(|s| s.legacy.in_370_roster).count(),
        370
    );
    for spec in &r.specs {
        assert!(!spec.acceptance_ids.is_empty());
        assert_eq!(
            spec.acceptance_ids, spec.legacy.native_triage.acceptance_ids,
            "{} triage acceptance drift",
            spec.legacy.file
        );
        assert!(
            spec.legacy
                .native_triage
                .supporting_probe_ids
                .iter()
                .all(|id| spec.supporting_probe_ids.contains(id)),
            "historical triage links must be retained"
        );
        assert!(
            spec.acceptance_ids
                .iter()
                .all(|id| r.cases.iter().any(|c| &c.id == id)),
            "{}",
            spec.legacy.file
        );
        assert!(
            spec.supporting_probe_ids
                .iter()
                .all(|id| r.cases.iter().any(|c| &c.id == id))
        );
        assert_eq!(spec.replacement_status, Status::BlockedBy);
    }
    assert_eq!(
        r.e2e_csv(),
        include_str!("../data/legacy-v1.2.0-e2e-map.csv"),
        "regenerate e2e mapping after inventory/scenario changes"
    );
    let csv = csv_rows(&r.e2e_csv()).unwrap();
    assert_eq!(csv.len(), 397);
    assert!(csv.iter().all(|row| row.len() == 7));
    assert!(
        r.markdown()
            .contains("1424 pass / 2 fail / 3178 blocked-by")
    );
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["format"], "powermove.acceptance");
}

#[test]
fn every_remaining_original_names_the_exact_unverified_contract() {
    let catalog = Catalog::embedded().unwrap();
    let report = run(&catalog, &Filter::default(), &[]).unwrap();
    for workflow in &catalog.workflows {
        for contract in &workflow.cases {
            let result = report.cases.iter().find(|c| c.id == contract.id).unwrap();
            if result.status == Status::BlockedBy {
                assert!(
                    result.reason.contains(&contract.expected),
                    "{}",
                    contract.id
                );
                assert!(result.blocked_by.as_ref().is_some_and(|s| !s.is_empty()));
            }
        }
    }
    let pending = report.cases.iter().find(|c| c.id == "A514").unwrap();
    assert_eq!(pending.blocked_by.as_deref(), Some("mcp:replace_media"));
    assert!(
        pending
            .reason
            .contains("verified bytes, fingerprints, retained versions")
    );
    let pixels = report.cases.iter().find(|c| c.id == "A535").unwrap();
    assert_eq!(
        pixels.blocked_by.as_deref(),
        Some("mcp:sample_media_frames")
    );
    assert!(pixels.needs_macos);
}

#[test]
fn configured_blocker_snapshot_and_workflow_status_columns_cannot_drift() {
    // This double certifies metadata/snapshot determinism only. Actual worker
    // assertions execute in verify.sh, and failures stay failures there.
    struct Available;
    impl pm_acceptance::runtime::RuntimeHook for Available {
        fn has_compat_bundle(&self) -> bool {
            true
        }
        fn check(&self, id: &str) -> Result<(), String> {
            assert!(pm_acceptance::runtime::CASES.iter().any(|c| c.0 == id));
            Ok(())
        }
    }
    let report = pm_acceptance::runner::run_with_runtime(
        &Catalog::embedded().unwrap(),
        &Filter::default(),
        &[],
        Some(&Available),
    )
    .unwrap();
    if std::env::var_os("PM_UPDATE_BLOCKERS").is_some() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/legacy-v1.2.0-remaining-blockers.csv"
        );
        std::fs::write(path, report.blockers_csv()).unwrap();
        eprintln!("counts {:?}", report.counts());
        return;
    }
    assert_eq!(report.counts(), (1467, 2, 3135));
    assert_eq!(
        report.blockers_csv(),
        include_str!("../data/legacy-v1.2.0-remaining-blockers.csv")
    );
    let rows = csv_rows(&report.blockers_csv()).unwrap();
    assert_eq!(rows.len(), 3136);
    assert!(rows.iter().all(|r| r.len() == 6));
    for workflow in &report.workflows {
        let row = format!(
            "| {} | {} | {} | {} | {} |",
            workflow.id,
            if workflow.failed > 0 {
                "fail"
            } else {
                "blocked-by"
            },
            workflow.passed,
            workflow.failed,
            workflow.blocked
        );
        assert!(
            include_str!("../data/legacy-workflows-2026-11-05.md").contains(&row),
            "{}",
            workflow.id
        );
    }
}
#[test]
fn filtered_pass_does_not_claim_complete_workflow() {
    let c = Catalog::embedded().unwrap();
    let r = run(
        &c,
        &Filter {
            workflow: None,
            case: Some("A517".into()),
        },
        &[],
    )
    .unwrap();
    assert_eq!(r.scope, "filtered");
    assert_eq!(r.counts(), (1, 0, 0));
    assert!(r.workflows[0].reason.contains("not a full workflow"));
    assert!(
        r.specs
            .iter()
            .all(|s| s.replacement_status == Status::BlockedBy)
    );
}
#[test]
fn unknown_and_mismatched_filters_are_configuration_errors() {
    let c = Catalog::embedded().unwrap();
    for filter in [
        Filter {
            workflow: Some("W999".into()),
            case: None,
        },
        Filter {
            workflow: None,
            case: Some("A999".into()),
        },
        Filter {
            workflow: Some("W001".into()),
            case: Some("A517".into()),
        },
    ] {
        assert!(run(&c, &filter, &[]).is_err());
    }
}
#[test]
fn errors_and_panics_fail_instead_of_becoming_blockers() {
    for result in [
        execute("test", "W033", "test", "injected error", || {
            Err("synthetic fault".into())
        }),
        execute("test", "W033", "test", "injected panic", || {
            panic!("synthetic panic")
        }),
    ] {
        assert_eq!(result.status, Status::Fail);
        assert!(result.blocked_by.is_none());
        assert!(result.reason.contains("synthetic"));
    }
}
#[test]
fn cli_filters_formats_and_strict_exit_are_machine_drivable() {
    use std::process::Command;
    let bin = env!("CARGO_BIN_EXE_pm-acceptance");
    let output = Command::new(bin)
        .args(["--reference", "v1.2.0", "--case", "A517"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["cases"][0]["id"], "A517");
    let blocked = Command::new(bin)
        .args(["--reference", "v1.2.0", "--case", "K001.normal", "--strict"])
        .output()
        .unwrap();
    assert_eq!(blocked.status.code(), Some(2));
    let bad = Command::new(bin)
        .args(["--reference", "v1.2.0", "--case", "unknown"])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(3));
    for args in [
        vec!["--runtime-runner"],
        vec!["--compat-bundle", "synthetic.js"],
    ] {
        assert_eq!(
            Command::new(bin).args(args).output().unwrap().status.code(),
            Some(3)
        );
    }
    let unavailable = Command::new(bin)
        .args([
            "--reference",
            "v1.2.0",
            "--case",
            "W037.v8-reload",
            "--runtime-runner",
            "/no/such/native-backing",
        ])
        .output()
        .unwrap();
    assert_eq!(unavailable.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&unavailable.stdout).unwrap();
    assert_eq!(json["cases"][0]["status"], "fail");
    assert!(json["cases"][0]["blocked_by"].is_null());
    let catalog = Command::new(bin)
        .args(["--reference", "v1.2.0", "--format", "catalog"])
        .output()
        .unwrap();
    assert!(catalog.status.success());
    let json: serde_json::Value = serde_json::from_slice(&catalog.stdout).unwrap();
    assert_eq!(json["bindings"].as_array().unwrap().len(), 195);
}

#[test]
fn explicit_corpus_cli_uses_copies_and_reports_faults_without_source_paths() {
    use std::process::Command;
    let scratch = pm_acceptance::fixture::Scratch::new().unwrap();
    let path = scratch.0.join("synthetic.pmv");
    std::fs::write(&path, pm_acceptance::fixture::CORPUS[0].1).unwrap();
    let bin = env!("CARGO_BIN_EXE_pm-acceptance");
    let r = Command::new(bin)
        .args([
            "--reference",
            "v1.2.0",
            "--case",
            "W007.corpus-0000",
            "--corpus",
        ])
        .arg(&scratch.0)
        .output()
        .unwrap();
    assert!(r.status.success());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        pm_acceptance::fixture::CORPUS[0].1
    );
    std::fs::write(&path, b"invalid synthetic input").unwrap();
    let r = Command::new(bin)
        .args([
            "--reference",
            "v1.2.0",
            "--case",
            "W007.corpus-0000",
            "--corpus",
        ])
        .arg(&scratch.0)
        .output()
        .unwrap();
    assert_eq!(r.status.code(), Some(1));
    assert!(
        !String::from_utf8(r.stdout)
            .unwrap()
            .contains(scratch.0.to_str().unwrap())
    );
    assert_eq!(std::fs::read(path).unwrap(), b"invalid synthetic input");
}

#[test]
fn backing_groups_partition_every_blocker_without_hiding_failures() {
    let catalog = Catalog::embedded().unwrap();
    let report = run(&catalog, &Filter::default(), &[]).unwrap();
    let blocked = report
        .cases
        .iter()
        .filter(|c| c.status == Status::BlockedBy)
        .map(|c| c.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let grouped = report
        .blocker_groups
        .iter()
        .flat_map(|g| &g.case_ids)
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert_eq!(grouped.len(), blocked.len());
    assert_eq!(
        grouped
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        blocked
    );
    for w in &catalog.workflows {
        let case = report
            .cases
            .iter()
            .find(|c| c.id == format!("{}.contract", w.id))
            .unwrap();
        assert_eq!(case.status, Status::BlockedBy);
        assert!(case.reason.contains("**Expected:**"), "{}", w.id);
        assert!(
            !["shell-integration", "media-service", "compat-runtime"]
                .contains(&case.blocked_by.as_deref().unwrap())
        );
    }
    let curve = report
        .cases
        .iter()
        .find(|c| c.id == "W061.frozen-set-easing")
        .unwrap();
    assert_eq!(curve.status, Status::Pass);
}

#[test]
fn post_tag_memory_claim_is_reconciled_without_erasing_its_original_ids() {
    let report = run(
        &Catalog::embedded().unwrap(),
        &Filter {
            workflow: Some("W030".into()),
            case: None,
        },
        &[],
    )
    .unwrap();
    assert_eq!(
        pm_mcp::catalog::legacy_catalog()["tools"]
            .as_array()
            .unwrap()
            .len(),
        67
    );
    assert_eq!(pm_mcp::catalog::tools().len(), 68);
    assert!(pm_mcp::catalog::spec("project_memory").is_some());
    assert!(
        pm_mcp::catalog::legacy_catalog()["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["name"] != "project_memory")
    );
    assert!(
        !pm_mcp::catalog::legacy_catalog()["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "get_project_state")
            .unwrap()["description"]
            .as_str()
            .unwrap()
            .contains("memory")
    );
    for id in ["A511", "A512"] {
        let case = report.cases.iter().find(|c| c.id == id).unwrap();
        assert_eq!(case.status, Status::BlockedBy);
        assert_eq!(
            case.blocked_by.as_deref(),
            Some("memory-post-tag-requirement")
        );
        assert!(case.reason.contains("W1 misattributes"));
    }
}
