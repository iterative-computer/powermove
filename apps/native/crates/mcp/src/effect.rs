//! Native port of the owner-pinned main registration validator (`kernel/glsl.ts`).
//! Shader compilation uses the same pipeline as rendering and needs no GPU.
mod shader;
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
/// Check registration metadata, then compile the effect program.
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
    if !def["coordinates"].is_null() && def["coordinates"] != "layer" {
        return Err(bad(format!("{what}: invalid effect coordinates")));
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
    if params.iter().filter(|p| reference(p)).count() > 4 {
        return Err(bad(format!("{what}: at most four layer inputs")));
    }
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
        if !param["type"].is_null()
            && !["number", "toggle", "color", "gradientStops", "layer", "path", "asset"]
                .iter()
                .any(|t| param["type"] == *t)
        {
            return Err(bad(format!("{what}: invalid parameter type")));
        }
        let context = &param["defaultFrom"];
        if !context.is_null()
            && (!numeric_param(param)
                || !["width", "height"]
                    .iter()
                    .any(|d| context["dimension"] == *d)
                || context["factor"].as_f64().is_none_or(|v| v.abs() > 1e6))
        {
            return Err(bad(format!("{what}: invalid contextual default")));
        }
    }
    if def.get("viewerHandles").is_some() {
        let handle_params =
            serde_json::from_value::<Vec<pm_fx_defs::Parameter>>(Value::Array(params.clone()))
                .map_err(|e| bad(format!("{what}: invalid handle parameters: {e}")))?;
        pm_fx_defs::handles::parse(def.get("viewerHandles"), &handle_params).map_err(bad)?;
    }
    for p in params.iter().filter(|p|p["type"]=="gradientStops") { pm_fx_defs::handles::validate_stops(&p["def"]).map_err(bad)?; }
    let mut generated = HashSet::<String>::new();
    for param in params.iter().filter(|p| reference(p)) {
        for suffix in [
            "UV",
            "FromUV",
            "Res",
            "Midpoint",
            "DomainMin",
            "DomainMax",
            "Size",
        ] {
            generated.insert(format!("{}{suffix}", param["k"].as_str().unwrap()));
        }
    }
    if def["coordinates"] == "layer" || params.iter().any(reference) {
        for key in ["layerUV", "ownerFromUV", "layerRes"] {
            generated.insert(key.into());
        }
    }
    if params
        .iter()
        .any(|p| p["k"].as_str().is_some_and(|k| generated.contains(k)))
    {
        return Err(bad(format!("{what}: layer input uniform name collision")));
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
    let passes = passes.unwrap();
    validate_contracts(def, params, passes, &what)?;
    let (source, offset) = shader::wrap(def, params, frag);
    compile_program(&id, passes as u8, &source, offset).map_err(bad)?;
    Ok(json!({"valid":true,"id":id,"parameterCount":params.len(),"compiledPasses":passes as u8}))
}

/// All passes run the same shader, with a different runtime value of u_pass.
pub fn compile_program(
    id: &str,
    passes: u8,
    source: &str,
    line_offset: u32,
) -> std::result::Result<(), String> {
    let Err(failure) = pm_glsl::translate(source, line_offset) else {
        return Ok(());
    };
    let pass = if passes <= 1 {
        "pass 1".to_owned()
    } else {
        format!("pass 1 through {passes}")
    };
    // A missing terminator may be reported at the generated closing brace.
    let body_end = source
        .strip_suffix("\n}\n")
        .filter(|_| line_offset > 0)
        .and_then(|body| (body.split('\n').count() as u32).checked_sub(line_offset));
    let details = if failure.diagnostics.is_empty() {
        failure.message
    } else {
        failure
            .diagnostics
            .iter()
            .map(|diagnostic| match diagnostic.line {
                Some(line) => {
                    let line = body_end.map_or(line, |end| line.min(end));
                    format!("line {line}: {}", diagnostic.message)
                }
                None => diagnostic.message.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    Err(format!(
        "effect \"{id}\" {pass}: GLSL compilation failed ({}): {details}",
        failure.stage.name(),
    ))
}

fn reference(param: &Value) -> bool {
    ["layer", "path", "asset"]
        .iter()
        .any(|t| param["type"] == *t)
}
fn numeric_param(param: &Value) -> bool {
    !truthy(&param["type"]) || param["type"] == "number"
}
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}
fn integer_range(value: &Value, min: f64, max: f64) -> bool {
    value
        .as_f64()
        .is_some_and(|n| n.fract() == 0.0 && n >= min && n <= max)
}
fn finite_range(value: &Value, min: f64, max: f64) -> bool {
    value.as_f64().is_some_and(|n| n >= min && n <= max)
}
fn version(value: &Value) -> Option<f64> {
    if value.is_null() {
        Some(1.0)
    } else {
        value
            .as_f64()
            .filter(|n| n.fract() == 0.0 && *n > 0.0 && *n <= 9_007_199_254_740_991.0)
    }
}
fn validate_contracts(def: &Value, params: &[Value], passes: f64, what: &str) -> Result<()> {
    let bad = |suffix: &str| Error::invalid(format!("{what}: {suffix}"));
    let numeric =
        |key: &Value| key.is_string() && params.iter().any(|p| p["k"] == *key && numeric_param(p));
    let typed =
        |key: &Value, kind: &str| params.iter().any(|p| p["k"] == *key && p["type"] == kind);
    let sampling = &def["sampling"];
    if truthy(sampling)
        && (!numeric(&sampling["radius"])
            || !sampling["support"]
                .as_f64()
                .is_some_and(|n| n > 0.0 && n <= 8.0)
            || !integer_range(&sampling["maxRadius"], 1.0, 64.0)
            || (!sampling["space"].is_null() && sampling["space"] != "layer")
            || (!sampling["firstPass"].is_null()
                && !integer_range(&sampling["firstPass"], 0.0, passes - 1.0)))
    {
        return Err(bad("invalid spatial sampling contract"));
    }
    let temporal = &def["temporal"];
    if truthy(temporal)
        && (!numeric(&temporal["step"])
            || !numeric(&temporal["count"])
            || !integer_range(&temporal["maxSamples"], 1.0, 32.0)
            || passes != 1.0
            || truthy(&def["backdrop"])
            || truthy(&def["keepOrig"])
            || truthy(sampling)
            || truthy(&def["grid"])
            || truthy(&def["coordinates"])
            || params.iter().any(reference)
            || def["sourceProcessing"]
                .as_object()
                .is_some_and(|o| !o.is_empty())
            || !truthy(&def["rawShader"]))
    {
        return Err(bad("invalid temporal contract"));
    }
    let source = &def["sourceProcessing"];
    if truthy(source) {
        if truthy(&source["geometry"])
            && source["geometry"].as_array().is_none_or(|a| {
                a.len() > 4
                    || a.iter().any(|k| {
                        !params
                            .iter()
                            .any(|p| p["k"] == *k && p["type"] == "path" && truthy(&p["pathBrush"]))
                    })
            })
        {
            return Err(bad("invalid source geometry contract"));
        }
        let empty = json!([]);
        let padding = if source["padding"].is_null() {
            &empty
        } else {
            &source["padding"]
        };
        if padding.as_array().is_none_or(|a| !a.iter().all(numeric))
            || !finite_range(
                source
                    .get("factor")
                    .filter(|v| !v.is_null())
                    .unwrap_or(&json!(1)),
                0.0,
                8.0,
            )
            || !finite_range(
                source
                    .get("pixels")
                    .filter(|v| !v.is_null())
                    .unwrap_or(&json!(0)),
                0.0,
                4096.0,
            )
            || (!source["when"].is_null() && !typed(&source["when"], "toggle"))
        {
            return Err(bad("invalid source bounds contract"));
        }
    }
    let grid = &def["grid"];
    if truthy(grid)
        && (passes != 2.0
            || !numeric(&grid["columns"])
            || !numeric(&grid["rows"])
            || (!grid["pointSample"].is_null() && !typed(&grid["pointSample"], "toggle")))
    {
        return Err(bad("invalid cell grid contract"));
    }
    if !def["viewportPaddingFactor"].is_null()
        && !def["viewportPaddingFactor"]
            .as_f64()
            .is_some_and(|v| v > 0.0 && v <= 8.0)
    {
        return Err(bad("invalid viewport padding factor"));
    }
    if !def["viewportPaddingPixels"].is_null()
        && !finite_range(&def["viewportPaddingPixels"], 0.0, 4096.0)
    {
        return Err(bad("invalid fixed viewport padding"));
    }
    for p in params {
        if p["type"] == "path" && truthy(&p["pathWidth"]) && !numeric(&p["pathWidth"]) {
            return Err(bad("invalid path width parameter"));
        }
    }
    for p in params {
        if p["type"] == "asset"
            && !p["assetKind"].is_null()
            && !["image", "lut"].iter().any(|k| p["assetKind"] == *k)
        {
            return Err(bad("invalid asset kind"));
        }
        if p["type"] == "path" && truthy(&p["pathBrush"]) {
            let b = &p["pathBrush"];
            if !["hardness", "spacing", "start", "end"]
                .iter()
                .all(|k| numeric(&b[k]))
                || !["allMasks", "sequential"]
                    .iter()
                    .all(|k| typed(&b[k], "toggle"))
            {
                return Err(bad("invalid path brush contract"));
            }
        }
    }
    let current = version(&def["version"]).ok_or_else(|| bad("invalid implementation version"))?;
    if !def["legacy"].is_null() {
        let revisions = def["legacy"]
            .as_array()
            .filter(|r| r.len() <= 32)
            .ok_or_else(|| bad("invalid historical revisions"))?;
        let mut seen = HashSet::new();
        for r in revisions {
            let v = version(&r["version"]);
            if !truthy(r)
                || r["id"] != def["id"]
                || truthy(&r["legacy"])
                || r["version"].is_null()
                || v.is_none_or(|v| v >= current)
                || !seen.insert(v.unwrap().to_bits())
            {
                return Err(bad("invalid historical revision"));
            }
            validate(r)?;
        }
    }
    if !def["actions"].is_null() {
        let actions = def["actions"]
            .as_array()
            .filter(|a| a.len() <= 16)
            .ok_or_else(|| bad("invalid parameter actions"))?;
        for a in actions {
            if a["kind"] != "swap"
                || a["label"].as_str().is_none_or(|s| s.is_empty())
                || a["keys"].as_array().is_none_or(|k| {
                    k.len() != 2 || k[0] == k[1] || k.iter().any(|key| !typed(key, "color"))
                })
            {
                return Err(bad("invalid color swap action"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod compile_tests {
    use super::*;

    fn effect(frag: &str) -> Value {
        json!({"id": "glass", "label": "Glass", "group": "Test", "params": [], "frag": frag})
    }

    #[test]
    fn rejects_shader_syntax_error_with_author_line() {
        let error = validate(&effect("\n\nvec4 broken = ;\n"))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("pass 1") && error.contains("line 3"),
            "{error}"
        );
        let error = validate(&effect("o = vec4(1.0)")).unwrap_err().to_string();
        assert!(error.contains("line 1"), "{error}");
    }

    #[test]
    fn rejects_undeclared_uniform_with_author_line() {
        let error = validate(&effect("\n\nvec4 broken = u_missing;\n"))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("pass 1") && error.contains("line 3"),
            "{error}"
        );
        assert!(error.contains("u_missing"), "{error}");
    }

    #[test]
    fn compiles_backdrop_shader_with_parameter_uniform() {
        let mut def = effect(
            "o = mix(bd(v_st), texture(u_orig, v_st), u_amount) * u_effectScale.x + float(u_pass) * src(v_st);",
        );
        def["passes"] = json!(2);
        def["keepOrig"] = json!(true);
        def["params"] = json!([{"k": "amount", "def": 0.5}]);
        def["backdrop"] = json!(true);
        let result = validate(&def).unwrap();
        assert_eq!(result["compiledPasses"], 2);
    }
}
