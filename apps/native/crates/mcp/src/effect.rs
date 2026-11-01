//! Native port of the frozen registration validator (`kernel/glsl.ts`).
//! Validation does not register code, compile GLSL, or enter a JS realm.
use crate::{Error, Result, model};
use serde_json::{Value, json};
use std::collections::HashSet;

fn string(v: Option<&Value>) -> String {
    v.map_or_else(
        || "undefined".into(),
        |v| model(v).to_js_string().to_string_lossy(),
    )
}
fn matches(pattern: &str, value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| regex::Regex::new(pattern).unwrap().is_match(s))
}
/// Same JSON admission, order and diagnostics as validateEffect at v1.2.0.
pub fn validate(def: &Value) -> Result<Value> {
    let bad = |message: String| Error::invalid(message);
    if !def.is_object() && !def.is_array() {
        return Err(bad("effect: definition must be an object".into()));
    }
    let id = string(def.get("id"));
    let what = format!("effect \"{id}\"");
    if !matches(r"^[a-zA-Z][a-zA-Z0-9._-]{0,63}$", &def["id"]) {
        return Err(bad(format!("effect: invalid id \"{id}\"")));
    }
    for field in ["label", "group"] {
        if def[field].as_str().is_none_or(str::is_empty) {
            return Err(bad(format!("{what}: \"{field}\" is required")));
        }
    }
    if !def["backdrop"].is_null() && !def["backdrop"].is_boolean() {
        return Err(bad(format!("{what}: \"backdrop\" must be a boolean")));
    }
    let passes = def["passes"]
        .as_f64()
        .or_else(|| def["passes"].is_null().then_some(1.));
    if passes.is_none_or(|n| n.fract() != 0. || !(1. ..=8.).contains(&n)) {
        return Err(bad(format!("{what}: \"passes\" must be an integer 1..8")));
    }
    let params = def["params"]
        .as_array()
        .ok_or_else(|| bad(format!("{what}: \"params\" must be an array")))?;
    let mut seen = HashSet::new();
    for param in params {
        if !param.is_object() && !param.is_array() {
            return Err(bad(format!("{what}: each param must be an object")));
        }
        if !matches(r"^[a-z][a-zA-Z0-9]*$", &param["k"]) {
            return Err(bad(format!(
                "{what}: invalid param key \"{}\" (expected /^[a-z][a-zA-Z0-9]*$/)",
                string(param.get("k"))
            )));
        }
        let key = param["k"].as_str().unwrap();
        if !seen.insert(key) {
            return Err(bad(format!("{what}: duplicate param key \"{key}\"")));
        }
    }
    let frag = def["frag"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad(format!("{what}: \"frag\" must be a non-empty string")))?;
    let len = frag.encode_utf16().count();
    if len > 65536 {
        return Err(bad(format!(
            "{what}: shader is too large ({len} > 65536 bytes)"
        )));
    }
    Ok(
        json!({"valid":true,"id":id,"parameterCount":params.len(),"note":"Definition validation only. This does not register the effect or compile/render its shader."}),
    )
}
