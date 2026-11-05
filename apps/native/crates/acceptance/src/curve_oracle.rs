//! Frozen authored Glide curve oracle, independent of native preset tables.
//! Both operation variants must preserve the complete authored curve.
use crate::fixture::{Fixture, without_revision};
use pm_kernel::DocumentStore;
use serde_json::{Value, json};
type Check = Result<(), String>;
fn evaluate(doc: Value, id: &str, path: &str, time: f64) -> f64 {
    let mut store = DocumentStore::new_untitled(pm_model::json::parse(&doc.to_string()).unwrap());
    store
        .queries()
        .evaluate(None, id, path, time)
        .unwrap()
        .as_f64()
        .unwrap()
}

/// agent-keyframe-easing.spec.ts:5: keep BOTH generated operation variants.
pub fn easing(operation: &str) -> Check {
    let mut f = Fixture::new()?;
    f.add("easing")?;
    f.call("finish_session", json!({"commit":true}))?;
    let before = without_revision(f.document()?);
    f.call("start_session", json!({"prompt":"Synthetic frozen easing"}))?;
    let keys = json!([
        {"time":0,"value":10,"ease":"glide"},
        {"time":2,"value":110,"ease":"glide"},
        {"time":3,"value":-40,"ease":"glide"}
    ]);
    let commands = if operation == "replace_keyframes" {
        json!([{"type":operation,"target":"easing","path":"rotation","keyframes":keys}])
    } else {
        Value::Array(
            keys.as_array()
                .unwrap()
                .iter()
                .map(|k| {
                    json!({
                        "type":operation,"target":"easing","path":"rotation","mode":"keyframe",
                        "time":k["time"],"value":k["value"],"ease":k["ease"]
                    })
                })
                .collect(),
        )
    };
    f.edit(commands)?;
    // Independent f64 inversion of the frozen glide [0.16, 0.84, 0.24, 1].
    // Expected values never come from pm-eval or the native preset table.
    let bezier = |a: f64, b: f64, u: f64| {
        3. * (1. - u).powi(2) * u * a + 3. * (1. - u) * u * u * b + u.powi(3)
    };
    let (mut lo, mut hi) = (0., 1.);
    for _ in 0..80 {
        let mid = (lo + hi) / 2.;
        if bezier(0.16, 0.24, mid) < 0.5 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let eased = bezier(0.84, 1., (lo + hi) / 2.);
    for (t, expected) in [(1., 10. + 100. * eased), (2.5, 110. - 150. * eased)] {
        let actual = evaluate(f.document()?, "easing", "rotation", t);
        if (actual - expected).abs() >= 1e-4 {
            return Err(format!(
                "{operation}: rotation at {t}s is {actual}, expected {expected}; complete glide curve was not preserved"
            ));
        }
    }
    f.call("finish_session", json!({"commit":false}))?;
    assert_eq!(without_revision(f.document()?), before);
    Ok(())
}

pub fn set() -> Check {
    easing("set_property")
}
pub fn replace() -> Check {
    easing("replace_keyframes")
}
