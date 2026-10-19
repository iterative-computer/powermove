//! Contract tests for the FONTS-R gate, independent of private captures.
#[path = "support/text_residuals.rs"]
mod residuals;

use residuals::{APPROVED, Approved, Gate, Key, Outcome, Ratchet};
use serde_json::{Value, json};

/// The synthetic approved table the logic tests use.
const EDGE: [Approved; 1] = [residuals::raster("edge", 3, 4)];
const EDGE_CORPUS: [Approved; 1] = [residuals::corpus(
    "edge",
    "group/project",
    false,
    16.0,
    3,
    4,
)];

fn load(f: &Value) -> Result<Ratchet, String> {
    Ratchet::from_json_with(&f.to_string(), &EDGE)
}

fn real() -> Value {
    serde_json::from_str(include_str!("fixtures/text-residuals.json")).unwrap()
}

fn fixture() -> Value {
    json!({"kind":"RATCHET", "version":1, "decision":"FONTS-R", "policy":"lower or remove only",
        "cases":[{"gate":"raster", "id":"edge", "pixelsOverBound":3, "maxCv":4,
            "cause":"Synthetic edge coverage test.", "sideBySide":"edge.png"}]})
}

fn ratchet() -> Ratchet {
    load(&fixture()).unwrap()
}

#[test]
fn unlisted_cases_keep_their_original_strict_bounds() {
    let mut r = ratchet();
    assert_eq!(r.check(Key::raster("new"), 0, 1), Ok(Outcome::Strict));
    assert!(r.check(Key::raster("new"), 1, 2).is_err());
    assert_eq!(
        r.check(Key::corpus("new", false, 0.0), 0, 2),
        Ok(Outcome::Strict)
    );
    assert!(r.check(Key::corpus("new", false, 0.0), 1, 3).is_err());
}

#[test]
fn listed_measurements_must_equal_both_budget_dimensions() {
    assert_eq!(
        ratchet().check(Key::raster("edge"), 3, 4),
        Ok(Outcome::Ratcheted)
    );
    for (pixels, max) in [(4, 4), (3, 5), (2, 4), (3, 3), (0, 1), (2, 5)] {
        assert!(
            ratchet().check(Key::raster("edge"), pixels, max).is_err(),
            "{pixels}, {max}"
        );
    }
}

#[test]
fn improvements_require_lowering_or_removing_the_entry() {
    let mut f = fixture();
    f["cases"][0]["pixelsOverBound"] = json!(2);
    f["cases"][0]["maxCv"] = json!(3);
    assert_eq!(
        load(&f).unwrap().check(Key::raster("edge"), 2, 3),
        Ok(Outcome::Ratcheted)
    );
    f["cases"] = json!([]);
    assert_eq!(
        load(&f).unwrap().check(Key::raster("edge"), 0, 1),
        Ok(Outcome::Strict)
    );
}

#[test]
fn selected_listed_cases_cannot_be_skipped() {
    let mut r = ratchet();
    assert_eq!(r.missing([Key::raster("edge")]).len(), 1);
    assert!(r.missing([Key::raster("unselected")]).is_empty());
    r.check(Key::raster("edge"), 3, 4).unwrap();
    assert!(r.missing([Key::raster("edge")]).is_empty());
}

#[test]
fn corpus_identity_cannot_share_a_budget_with_another_project_time_or_pass() {
    let mut f = fixture();
    f["cases"][0]["gate"] = json!("corpus");
    f["cases"][0]["corpus"] = json!({"project":"group/project", "isolated":false, "time":16});
    let mut r = Ratchet::from_json_with(&f.to_string(), &EDGE_CORPUS).unwrap();
    assert_eq!(
        r.check(Key::corpus("group/project", false, 16.0), 3, 4),
        Ok(Outcome::Ratcheted)
    );
    for key in [
        Key::corpus("group_project", false, 16.0),
        Key::corpus("group/project", true, 16.0),
        Key::corpus("group/project", false, 18.0),
        Key::raster("edge"),
    ] {
        assert!(r.check(key, 3, 4).is_err());
    }
}

#[test]
fn invalid_metadata_duplicates_and_non_filename_review_paths_are_rejected() {
    for field in ["kind", "decision"] {
        let mut f = fixture();
        f[field] = json!("wrong");
        assert!(load(&f).is_err());
    }
    for name in ["/capture.png", "../capture.png", "C:\\capture.png"] {
        let mut f = fixture();
        f["cases"][0]["sideBySide"] = json!(name);
        assert!(load(&f).is_err());
    }
    let mut f = fixture();
    f["cases"][0]["pixelsOverBound"] = json!(0);
    assert!(load(&f).is_err());
    let mut f = fixture();
    f["cases"]
        .as_array_mut()
        .unwrap()
        .push(fixture()["cases"][0].clone());
    assert!(load(&f).is_err());
    assert!(Ratchet::from_json(include_str!("fixtures/text-residuals.json")).is_ok());
}

#[test]
fn every_listed_case_of_a_gate_must_be_measured_in_a_full_run() {
    let mut r = ratchet();
    assert_eq!(r.unmeasured(Gate::Raster).len(), 1);
    assert!(r.unmeasured(Gate::Corpus).is_empty());
    r.check(Key::raster("other"), 0, 0).unwrap();
    assert_eq!(r.unmeasured(Gate::Raster).len(), 1);
    r.check(Key::raster("edge"), 3, 4).unwrap();
    assert!(r.unmeasured(Gate::Raster).is_empty());
}

#[test]
fn the_real_fixture_loads_within_the_approved_ceilings() {
    let r = Ratchet::load();
    let fixture = real();
    let entries = fixture["cases"].as_array().unwrap();
    for (gate, name) in [(Gate::Raster, "raster"), (Gate::Corpus, "corpus")] {
        assert_eq!(
            r.unmeasured(gate).len(),
            entries.iter().filter(|e| e["gate"] == name).count()
        );
    }
    let mut ids: Vec<_> = APPROVED.iter().map(|a| a.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), APPROVED.len());
    for a in APPROVED {
        let strict = if a.corpus.is_some() { 2 } else { 1 };
        assert!(a.pixels_over_bound > 0 && a.max_cv > strict, "{}", a.id);
    }
}

#[test]
fn raising_any_budget_fails_without_a_change_to_the_approved_table() {
    let n = real()["cases"].as_array().unwrap().len();
    for i in 0..n {
        for field in ["pixelsOverBound", "maxCv"] {
            let mut f = real();
            let v = f["cases"][i][field].as_u64().unwrap();
            f["cases"][i][field] = json!(v + 1);
            assert!(
                Ratchet::from_json(&f.to_string()).is_err(),
                "{} {field} raised",
                f["cases"][i]["id"]
            );
        }
    }
}

#[test]
fn adding_a_case_fails_without_a_change_to_the_approved_table() {
    let mut f = real();
    let mut extra = f["cases"][0].clone();
    extra["id"] = json!("caption-paper-0463");
    extra["sideBySide"] = json!("caption-paper-0463.png");
    f["cases"].as_array_mut().unwrap().push(extra);
    assert!(Ratchet::from_json(&f.to_string()).is_err());
}

#[test]
fn an_approved_budget_cannot_be_moved_to_another_measurement() {
    let corpus = real()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .position(|c| c["gate"] == "corpus")
        .unwrap();
    let isolated = real()["cases"][corpus]["corpus"]["isolated"]
        .as_bool()
        .unwrap();
    for (field, value) in [
        ("time", json!(20)),
        ("isolated", json!(!isolated)),
        ("project", json!("real/other")),
    ] {
        let mut f = real();
        f["cases"][corpus]["corpus"][field] = value;
        assert!(Ratchet::from_json(&f.to_string()).is_err(), "{field}");
    }
    let mut f = real();
    f["cases"][0]["gate"] = json!("corpus");
    f["cases"][0]["corpus"] =
        json!({"project": "real/powermove-ad", "isolated": false, "time": 16});
    assert!(Ratchet::from_json(&f.to_string()).is_err());
}

#[test]
fn lowering_or_removing_budgets_stays_possible_by_editing_the_fixture() {
    let n = real()["cases"].as_array().unwrap().len();
    for i in 0..n {
        let mut f = real();
        let pixels = f["cases"][i]["pixelsOverBound"].as_u64().unwrap();
        if pixels > 1 {
            f["cases"][i]["pixelsOverBound"] = json!(pixels - 1);
            assert!(Ratchet::from_json(&f.to_string()).is_ok());
        }
        let mut f = real();
        f["cases"].as_array_mut().unwrap().remove(i);
        assert!(Ratchet::from_json(&f.to_string()).is_ok());
    }
}

#[test]
fn retired_exact_cases_cannot_be_revived_by_editing_the_fixture() {
    for id in [
        "colr-0033",
        "colr-0034",
        "caption-pop-0455",
        "caption-pop-0456",
        "caption-pop-0458",
        "caption-pop-0459",
        "caption-stroke-0491",
        "caption-whisper-0442",
        "caption-whisper-0443",
        "caption-whisper-0444",
    ] {
        let mut f = real();
        f["cases"].as_array_mut().unwrap().push(json!({
            "gate":"raster","id":id,"pixelsOverBound":1,"maxCv":2,
            "cause":"An exact case must stay on the strict bound.",
            "sideBySide":format!("{id}.png")
        }));
        assert!(
            Ratchet::from_json(&f.to_string()).is_err(),
            "retired case {id} revived"
        );
    }
}
