//! Strict supported primitive properties. Unknown legacy wire properties remain diagnostic-only;
//! new authors put extension data in metadata. Native behavior never reads unknown properties.
use serde_json::{Map, Value};

pub fn validate(kind: &str, key: &str, value: &Value) -> Result<(), String> {
    if value.is_null() {
        return Ok(());
    }
    let allowed = match key {
        "min" | "max" | "speed" | "precision" | "unit" | "checked" => kind == "input",
        "step" => matches!(kind, "input" | "div"),
        "src" => matches!(kind, "img" | "webview"),
        "alt" => kind == "img",
        "anchor" | "placement" => kind == "anchored",
        "count" | "range" | "itemSize" => kind == "virtual-list",
        "d" | "points" => kind == "path",
        "resource" => matches!(kind, "img" | "canvas" | "frame"),
        "surface" => matches!(kind, "canvas" | "frame" | "Glass"),
        "time" => kind == "frame",
        "source" => matches!(kind, "markdown" | "code" | "webview"),
        "language" => kind == "code",
        "material" => matches!(kind, "Glass" | "material"),
        "styleRules" => kind == "root",
        _ => true,
    };
    if !allowed {
        return Err(format!("{key} is not supported on {kind}"));
    }
    let valid = match key {
        "metadata" => value.is_object(),
        "resource" | "surface" | "source" => value.is_string(),
        "time" => value.as_f64().is_some_and(f64::is_finite),
        "count" | "anchor" if value.is_number() => {
            value.as_u64().is_some_and(|n| n <= 9_007_199_254_740_991)
        }
        "anchor" => value.as_object().is_some_and(point),
        "points" => value
            .as_array()
            .is_some_and(|points| points.iter().all(|p| p.as_object().is_some_and(point))),
        "range" => range(value).is_some(),
        "itemSize" => value.as_f64().is_some_and(|n| n.is_finite() && n > 0.0),
        "precision" => value.as_u64().is_some_and(|n| n <= 9_007_199_254_740_991),
        "tabIndex" => value.as_i64().is_some(),
        "step" | "speed" => value.as_f64().is_some_and(|n| n.is_finite() && n > 0.0),
        "preventDefault" => value.as_object().is_some_and(|flags| {
            flags.values().all(|v| {
                v.is_boolean() || v.as_array().is_some_and(|a| a.iter().all(Value::is_string))
            })
        }),
        "options" => value.as_array().is_some_and(|a| {
            a.iter().all(|v| {
                v.is_string()
                    || v.as_object().is_some_and(|o| {
                        o.contains_key("v")
                            && o.get("label").is_some_and(Value::is_string)
                            && o.get("disabled").is_none_or(Value::is_boolean)
                            && o.keys()
                                .all(|k| matches!(k.as_str(), "v" | "label" | "disabled"))
                    })
            })
        }),
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(format!("invalid {key}"))
    }
}
fn point(p: &Map<String, Value>) -> bool {
    p.len() == 2
        && ["x", "y"].iter().all(|k| {
            p.get(*k)
                .and_then(Value::as_f64)
                .is_some_and(f64::is_finite)
        })
}
pub fn range(value: &Value) -> Option<(u64, u64)> {
    let a = value.as_array()?;
    if a.len() != 2 {
        return None;
    }
    let start = a[0].as_u64()?;
    let end = a[1].as_u64()?;
    (start <= end && end <= 9_007_199_254_740_991).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use crate::Surface;
    #[test]
    fn strict_primitive_shapes_and_ranges_reject_atomically() {
        for (kind, patch) in [
            ("div", r#"{"anchor":1}"#),
            ("path", r#"{"points":[{"x":1}]}"#),
            ("frame", r#"{"resource":2}"#),
            ("code", r#"{"source":false}"#),
            ("virtual-list", r#"{"range":[4,2]}"#),
            ("virtual-list", r#"{"count":2,"range":[0,3]}"#),
            ("virtual-list", r#"{"count":3,"range":[0,2],"itemSize":0}"#),
            ("anchored", r#"{"anchor":{"x":0,"y":1,"z":2}}"#),
            ("input", r#"{"precision":1.5}"#),
            ("button", r#"{"preventDefault":{"click":12}}"#),
        ] {
            let mut surface = Surface::default();
            let wire = format!(
                r#"{{"baseRevision":0,"ops":[{{"op":"create","id":1,"type":"{kind}"}},{{"op":"setProps","id":1,"patch":{patch}}},{{"op":"insertBefore","parent":0,"id":1,"before":null}}]}}"#
            );
            assert!(surface.apply_json(&wire).is_err(), "{wire}");
            assert_eq!(surface.revision(), 0);
            assert_eq!(surface.nodes().len(), 1);
        }
    }
    #[test]
    fn point_anchor_and_metadata_accept_without_id_lookup() {
        let mut surface = Surface::default();
        let diagnostics = surface.apply_json(r#"{"baseRevision":0,"ops":[{"op":"create","id":1,"type":"anchored"},{"op":"setProps","id":1,"patch":{"anchor":{"x":0.25,"y":-3},"metadata":{"exact":false}}},{"op":"insertBefore","parent":0,"id":1,"before":null}]}"#).unwrap();
        assert!(diagnostics.is_empty());
    }
}
