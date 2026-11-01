use pm_acceptance::catalog::{Catalog, csv_rows};
#[test]
fn inventory_is_complete_and_appendix_is_not_promoted() {
    let c = Catalog::embedded().unwrap();
    assert_eq!(c.workflows.len(), 83);
    assert_eq!(
        c.workflows.iter().map(|w| w.cases.len()).sum::<usize>(),
        1344
    );
    assert_eq!(c.bindings.len(), 195);
    assert_eq!(c.specs.len(), 396);
    assert_eq!(c.specs.iter().filter(|s| s.in_370_roster).count(), 370);
    assert_eq!(c.specs.iter().filter(|s| s.in_tag).count(), 289);
    assert!(
        !c.workflows
            .iter()
            .flat_map(|w| &w.cases)
            .any(|c| c.id == "A017")
    );
    assert!(c.workflows.last().unwrap().cases.is_empty());
}
#[test]
fn binding_arguments_and_exceptions_are_retained() {
    let c = Catalog::embedded().unwrap();
    let k = &c.bindings[193];
    assert_eq!(k.id, "K194");
    assert_eq!(k.priority, 50);
    assert!(k.in_fields);
    assert_eq!(c.bindings[194].priority, 0);
    assert!(
        c.bindings
            .iter()
            .any(|k| !k.args.as_array().unwrap().is_empty())
    );
    assert_eq!(c.bindings[191].chord, "cmd+`");
}
#[test]
fn csv_handles_real_provenance_and_rejects_truncation() {
    assert_eq!(
        csv_rows("a,b\r\n\"line\nwith, commas and \"\"quotes\"\"\",\"\"\r\n").unwrap(),
        vec![
            vec!["a", "b"],
            vec!["line\nwith, commas and \"quotes\"", ""]
        ]
    );
    for malformed in ["\"missing", "a\"b", "\"a\"x"] {
        assert!(csv_rows(malformed).is_err());
    }
}
#[test]
fn invalid_references_and_duplicates_fail_closed() {
    let mut c = Catalog::embedded().unwrap();
    c.bindings[0].workflow = "W999".into();
    assert!(c.validate().is_err());
    let mut c = Catalog::embedded().unwrap();
    c.specs.push(c.specs[0].clone());
    assert!(c.validate().is_err());
}

#[test]
fn every_legacy_spec_has_explicit_triage_and_cannot_be_silently_retired() {
    let c = Catalog::embedded().unwrap();
    for s in &c.specs {
        assert_eq!(s.native_triage.status, "blocked-by");
        assert!(!s.native_triage.acceptance_ids.is_empty());
        assert!(!s.native_triage.blocked_by.is_empty());
    }
    for bad in ["untriaged", "pass"] {
        let mut changed = c.clone();
        changed.specs[0].native_triage.status = bad.into();
        assert!(changed.validate().is_err());
    }
    let mut changed = c;
    changed.specs[0].native_triage.acceptance_ids.clear();
    assert!(changed.validate().is_err());
}
