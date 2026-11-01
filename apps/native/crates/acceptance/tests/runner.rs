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
    assert_eq!(r.counts(), (1399, 1, 3175));
    assert_eq!(r.exit_code(false), 1);
    assert_eq!(r.exit_code(true), 1);
    assert_eq!(
        r.workflows
            .iter()
            .filter(|w| w.status == Status::BlockedBy)
            .count(),
        82
    );
    assert_eq!(
        r.workflows.iter().find(|w| w.id == "W071").unwrap().status,
        Status::Fail
    );
    assert_eq!(
        r.cases
            .iter()
            .filter(|x| x.kind == "original" && x.status == Status::Pass)
            .count(),
        7
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
        include_str!("../data/e2e-map.csv"),
        "regenerate e2e mapping after inventory/scenario changes"
    );
    let csv = csv_rows(&r.e2e_csv()).unwrap();
    assert_eq!(csv.len(), 397);
    assert!(csv.iter().all(|row| row.len() == 7));
    assert!(
        r.markdown()
            .contains("1399 pass / 1 fail / 3175 blocked-by")
    );
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["format"], "powermove.acceptance");
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
    let output = Command::new(bin).args(["--case", "A517"]).output().unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["cases"][0]["id"], "A517");
    let blocked = Command::new(bin)
        .args(["--case", "K001.normal", "--strict"])
        .output()
        .unwrap();
    assert_eq!(blocked.status.code(), Some(2));
    let bad = Command::new(bin)
        .args(["--case", "unknown"])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(3));
    let catalog = Command::new(bin)
        .args(["--format", "catalog"])
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
        .args(["--case", "W007.corpus-0000", "--corpus"])
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
        .args(["--case", "W007.corpus-0000", "--corpus"])
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
