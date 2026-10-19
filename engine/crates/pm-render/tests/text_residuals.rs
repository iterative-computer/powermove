//! Contract tests for the FONTS-R gate, independent of private captures.
#[path = "support/text_residuals.rs"]
mod residuals;

use residuals::{Key, Outcome, Ratchet};
use serde_json::{Value, json};

fn fixture() -> Value {
    json!({"kind":"RATCHET", "version":1, "decision":"FONTS-R", "policy":"lower or remove only",
        "cases":[{"gate":"raster", "id":"edge", "pixelsOverBound":3, "maxCv":4,
            "cause":"Synthetic edge coverage test.", "sideBySide":"edge.png"}]})
}

fn ratchet() -> Ratchet {
    Ratchet::from_json(&fixture().to_string()).unwrap()
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
        Ratchet::from_json(&f.to_string())
            .unwrap()
            .check(Key::raster("edge"), 2, 3),
        Ok(Outcome::Ratcheted)
    );
    f["cases"] = json!([]);
    assert_eq!(
        Ratchet::from_json(&f.to_string())
            .unwrap()
            .check(Key::raster("edge"), 0, 1),
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
    let mut r = Ratchet::from_json(&f.to_string()).unwrap();
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
        assert!(Ratchet::from_json(&f.to_string()).is_err());
    }
    for name in ["/capture.png", "../capture.png", "C:\\capture.png"] {
        let mut f = fixture();
        f["cases"][0]["sideBySide"] = json!(name);
        assert!(Ratchet::from_json(&f.to_string()).is_err());
    }
    let mut f = fixture();
    f["cases"][0]["pixelsOverBound"] = json!(0);
    assert!(Ratchet::from_json(&f.to_string()).is_err());
    let mut f = fixture();
    f["cases"]
        .as_array_mut()
        .unwrap()
        .push(fixture()["cases"][0].clone());
    assert!(Ratchet::from_json(&f.to_string()).is_err());
    assert!(Ratchet::from_json(include_str!("fixtures/text-residuals.json")).is_ok());
}
