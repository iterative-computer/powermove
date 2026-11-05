//! Kept separate from the acceptance lane's 396-file inventory.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const REFERENCE: &str = "07ed5fa17ee640659cb075f9cad1af2095011d60";
#[derive(Debug, Deserialize, Serialize)]
pub struct Source {
    pub file: String,
    pub sha256: String,
    pub declarations: usize,
    pub guards: Vec<String>,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct LegacyCase {
    pub id: String,
    pub file: String,
    pub line: usize,
    pub title: String,
    pub registration: String,
    pub parameters: Vec<String>,
    #[serde(rename = "parameterDefinitions")]
    pub parameter_definitions: Vec<String>,
    pub guards: Vec<String>,
    pub sha256: String,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Inventory {
    pub revision: String,
    pub sources: Vec<Source>,
    pub cases: Vec<LegacyCase>,
}
pub fn frozen() -> Inventory {
    serde_json::from_str(include_str!("frozen.json")).expect("frozen declaration inventory")
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Backing {
    pub missing: String,
    pub evidence: String,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Binding {
    pub check: String,
    pub sources: Vec<String>,
    pub coverage: String,
    pub kind: String,
    pub scope: String,
}
pub fn backings() -> BTreeMap<String, Backing> {
    serde_json::from_str(include_str!("backings.json")).expect("missing capabilities")
}
pub fn bindings() -> Vec<Binding> {
    serde_json::from_str(include_str!("traceability.json")).expect("native check links")
}

/// Validate the ledger against the acceptance lane's existing file-level
/// catalog. No planned test, helper regex match or post-tag case becomes a pass.
pub fn validate() -> Result<(), String> {
    let frozen = frozen();
    let catalog = pm_acceptance::catalog::Catalog::embedded()?;
    let backings = backings();
    let bindings = bindings();
    let declaration_rows =
        pm_acceptance::catalog::csv_rows(include_str!("../../data/e2e-declarations.csv"))?;
    if declaration_rows.len() != frozen.cases.len() + 1 {
        return Err("frozen declaration map cardinality drift".into());
    }
    if frozen.revision != REFERENCE {
        return Err("unexpected frozen revision".into());
    }
    let mut ids = BTreeSet::new();
    let mut counts = BTreeMap::<&str, usize>::new();
    for case in &frozen.cases {
        if !ids.insert(case.id.as_str())
            || case.id != format!("{}#L{}", case.file, case.line)
            || case.title.is_empty()
            || !is_hash(&case.sha256)
            || case.registration != "test"
        {
            return Err(format!("invalid declaration {}", case.id));
        }
        *counts.entry(&case.file).or_default() += 1;
        let spec = catalog
            .specs
            .iter()
            .find(|s| s.file == case.file && s.in_tag)
            .ok_or_else(|| format!("missing tag spec {}", case.file))?;
        for workflow in &spec.workflows {
            let missing = backings
                .get(workflow)
                .ok_or_else(|| format!("missing backing {workflow}"))?;
            if missing.missing.is_empty() || missing.evidence.is_empty() {
                return Err(format!("empty backing {workflow}"));
            }
        }
        let rows = declaration_rows
            .iter()
            .skip(1)
            .filter(|row| row.first() == Some(&case.id))
            .collect::<Vec<_>>();
        let expected_checks = bindings
            .iter()
            .filter(|b| b.sources.contains(&case.id))
            .map(|b| b.check.as_str())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(";");
        if rows.len() != 1
            || rows[0].len() != 7
            || rows[0][1] != case.file
            || rows[0][2] != case.line.to_string()
            || rows[0][3] != case.sha256
            || rows[0][4] != expected_checks
            || rows[0][5] != spec.workflows.join(";")
            || rows[0][6] != case.title
        {
            return Err(format!("frozen declaration map drift {}", case.id));
        }
    }
    let mut files = BTreeSet::new();
    for source in &frozen.sources {
        if !files.insert(source.file.as_str())
            || !is_hash(&source.sha256)
            || counts.get(source.file.as_str()).copied().unwrap_or(0) != source.declarations
        {
            return Err(format!("invalid source {}", source.file));
        }
    }
    if counts.keys().any(|file| !files.contains(file)) {
        return Err("declaration outside source roster".into());
    }
    let mut native = BTreeSet::new();
    for binding in &bindings {
        if !native.insert(binding.check.as_str())
            || binding.sources.is_empty()
            || binding.coverage.is_empty()
            || !["headless-contract", "trait-double"].contains(&binding.kind.as_str())
            || !["complete-headless", "supporting"].contains(&binding.scope.as_str())
            || (binding.kind == "trait-double" && binding.scope != "supporting")
            || binding
                .sources
                .iter()
                .any(|source| !ids.contains(source.as_str()))
        {
            return Err(format!("invalid native binding {}", binding.check));
        }
    }
    let registered = crate::checks::ALL
        .iter()
        .map(|c| c.id)
        .collect::<BTreeSet<_>>();
    if native != registered {
        return Err("native registry/traceability drift".into());
    }
    let actual_specs = counts.keys().copied().collect::<BTreeSet<_>>();
    let tag_specs = catalog
        .specs
        .iter()
        .filter(|s| s.in_tag)
        .map(|s| s.file.as_str())
        .collect::<BTreeSet<_>>();
    if actual_specs != tag_specs {
        return Err("tag spec roster drift".into());
    }
    Ok(())
}
fn is_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Debug, Serialize)]
pub struct CheckResult {
    pub id: String,
    pub status: pm_acceptance::runner::Status,
    pub kind: String,
    pub scope: String,
    pub reason: String,
    pub coverage: String,
    pub sources: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct MissingCapability {
    pub id: String,
    pub capability: String,
    pub evidence: String,
}
#[derive(Debug, Serialize)]
pub struct DeclarationResult {
    pub source: LegacyCase,
    pub status: pm_acceptance::runner::Status,
    pub scope: &'static str,
    pub supporting_checks: Vec<String>,
    pub missing: Vec<MissingCapability>,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub format: &'static str,
    pub reference: &'static str,
    pub platform: &'static str,
    pub scope: &'static str,
    pub source_files: usize,
    pub spec_files: usize,
    pub declarations: usize,
    pub parameterized_declarations: usize,
    pub checks: Vec<CheckResult>,
    pub cases: Vec<DeclarationResult>,
}
impl Report {
    pub fn exit_code(&self) -> i32 {
        use pm_acceptance::runner::Status;
        if self.checks.iter().any(|c| c.status == Status::Fail) {
            1
        } else if self.cases.iter().any(|c| c.status == Status::BlockedBy) {
            2
        } else {
            0
        }
    }
}
pub fn run(filter: Option<&str>) -> Result<Report, String> {
    validate()?;
    if filter.is_some_and(|id| !crate::checks::ALL.iter().any(|c| c.id == id)) {
        return Err("unknown native check ID".into());
    }
    let frozen = frozen();
    let backings = backings();
    let bindings = bindings();
    let catalog = pm_acceptance::catalog::Catalog::embedded()?;
    let checks = crate::checks::ALL
        .iter()
        .filter(|c| filter.is_none_or(|id| id == c.id))
        .map(|check| {
            let binding = bindings.iter().find(|b| b.check == check.id).unwrap();
            let result =
                pm_acceptance::runner::execute(check.id, "e2e", &binding.kind, check.id, check.run);
            CheckResult {
                id: check.id.into(),
                status: result.status,
                kind: binding.kind.clone(),
                scope: binding.scope.clone(),
                reason: result.reason,
                coverage: binding.coverage.clone(),
                sources: binding.sources.clone(),
            }
        })
        .collect::<Vec<_>>();
    let source_files = frozen.sources.len();
    let spec_files = frozen.sources.iter().filter(|s| s.declarations > 0).count();
    let declarations = frozen.cases.len();
    let parameterized_declarations = frozen
        .cases
        .iter()
        .filter(|c| !c.parameters.is_empty())
        .count();
    let cases = frozen
        .cases
        .into_iter()
        .filter(|case| filter.is_none() || checks.iter().any(|c| c.sources.contains(&case.id)))
        .map(|case| {
            let spec = catalog.specs.iter().find(|s| s.file == case.file).unwrap();
            let complete = bindings
                .iter()
                .filter(|b| b.scope == "complete-headless" && b.sources.contains(&case.id))
                .collect::<Vec<_>>();
            let mut missing = if complete.is_empty() {
                spec.workflows
                    .iter()
                    .map(|id| {
                        let backing = &backings[id];
                        MissingCapability {
                            id: id.clone(),
                            capability: backing.missing.clone(),
                            evidence: backing.evidence.clone(),
                        }
                    })
                    .collect::<Vec<_>>()
            } else {
                vec![]
            };
            let supporting = checks
                .iter()
                .filter(|c| c.sources.contains(&case.id))
                .collect::<Vec<_>>();
            let failed = supporting
                .iter()
                .filter(|c| c.status == pm_acceptance::runner::Status::Fail)
                .collect::<Vec<_>>();
            for check in &failed {
                missing.push(MissingCapability {
                    id: check.id.clone(),
                    capability: check.reason.clone(),
                    evidence: case.id.clone(),
                });
            }
            for required in &complete {
                if !supporting.iter().any(|check| check.id == required.check) {
                    missing.push(MissingCapability {
                        id: required.check.clone(),
                        capability:
                            "Required parameter variant was not executed in this filtered run"
                                .into(),
                        evidence: case.id.clone(),
                    });
                }
            }
            // Only the explicitly reviewed data/behavior-only declarations can
            // pass as complete headless equivalents. A domain subset or a
            // trait double never certifies a remaining UI/pixel assertion.
            DeclarationResult {
                status: if !failed.is_empty() {
                    pm_acceptance::runner::Status::Fail
                } else if !complete.is_empty() && missing.is_empty() {
                    pm_acceptance::runner::Status::Pass
                } else {
                    pm_acceptance::runner::Status::BlockedBy
                },
                scope: if complete.is_empty() {
                    "supporting-only"
                } else {
                    "complete-headless"
                },
                supporting_checks: supporting.iter().map(|c| c.id.clone()).collect(),
                missing,
                source: case,
            }
        })
        .collect();
    Ok(Report {
        format: "powermove.native-e2e.v1",
        reference: REFERENCE,
        platform: std::env::consts::OS,
        scope: if filter.is_some() {
            "filtered-supporting-check"
        } else {
            "frozen-declaration-inventory"
        },
        source_files,
        spec_files,
        declarations,
        parameterized_declarations,
        checks,
        cases,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_frozen_declaration_has_exact_provenance_and_a_capability() {
        validate().unwrap();
        let inventory = frozen();
        assert_eq!((inventory.sources.len(), inventory.cases.len()), (290, 614));
        assert_eq!(
            inventory
                .sources
                .iter()
                .filter(|s| s.declarations > 0)
                .count(),
            289
        );
        assert_eq!(
            inventory
                .cases
                .iter()
                .filter(|s| !s.parameters.is_empty())
                .count(),
            80
        );
        let non_test = inventory
            .sources
            .iter()
            .find(|s| s.declarations == 0)
            .unwrap();
        assert_eq!(
            non_test.file,
            "apps/desktop/e2e/fixtures/typewriter-effect/cursor.ts"
        );
    }
    #[test]
    fn unknown_check_cannot_silently_run_nothing() {
        assert!(run(Some("nonexistent-check")).is_err());
    }
    #[test]
    fn a_passing_double_never_turns_into_a_legacy_pass() {
        let report = run(Some("surface_stable_ids")).unwrap();
        assert_eq!(report.checks.len(), 1);
        assert_eq!(report.checks[0].status, pm_acceptance::runner::Status::Pass);
        assert!(
            report
                .cases
                .iter()
                .all(|c| c.status == pm_acceptance::runner::Status::BlockedBy
                    && !c.missing.is_empty())
        );
        assert_eq!(report.exit_code(), 2);
    }
    #[test]
    fn repaired_easing_probe_does_not_certify_the_complete_declaration() {
        let report = run(Some("easing_set_batch")).unwrap();
        assert_eq!(report.checks.len(), 1);
        assert_eq!(report.cases.len(), 1);
        assert_eq!(report.checks[0].status, pm_acceptance::runner::Status::Pass);
        assert_eq!(
            report.cases[0].status,
            pm_acceptance::runner::Status::BlockedBy
        );
        assert!(!report.cases[0].missing.is_empty());
        assert_eq!(report.exit_code(), 2);
    }
    #[test]
    fn one_passing_parameter_variant_does_not_certify_the_declaration() {
        let report = run(Some("easing_replace_batch")).unwrap();
        assert_eq!(report.checks[0].status, pm_acceptance::runner::Status::Pass);
        assert_eq!(
            report.cases[0].status,
            pm_acceptance::runner::Status::BlockedBy
        );
        assert_eq!(report.cases[0].missing[0].id, "easing_set_batch");
        assert_eq!(report.exit_code(), 2);
    }
    #[test]
    fn a_complete_headless_equivalent_can_pass_without_claiming_host_parity() {
        let report = run(Some("selection_preserves_redo")).unwrap();
        assert_eq!(report.cases[0].status, pm_acceptance::runner::Status::Pass);
        assert_eq!(report.cases[0].scope, "complete-headless");
        assert!(report.cases[0].missing.is_empty());
        assert_eq!(report.exit_code(), 0);
    }
}
