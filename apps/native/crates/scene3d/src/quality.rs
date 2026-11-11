//! Per-composition Studio quality. Export overrides resolve without writing it.
use crate::{Error, Result, fail};
use serde_json::{Value, json};
pub fn preset(name: &str) -> Result<Value> {
    let (noise, min, max, time, bounces, caustics, blur) = match name {
        "draft" => (0.12, 4, 16, 0.5, 4, false, 1),
        "preview" => (0.06, 16, 64, 2., 6, false, 2),
        "production" => (0.03, 32, 256, 3., 8, true, 2),
        "final" => (0.015, 512, 1024, 20., 8, true, 2),
        _ => return fail("Unknown Studio quality preset"),
    };
    Ok(
        json!({"preset":name,"noiseThreshold":noise,"minSamples":min,"maxSamples":max,"timeLimitSeconds":time,"maxBounces":bounces,"caustics":caustics,"denoise":true,"motionBlurSamples":blur}),
    )
}
pub fn get(comp: &Value) -> Result<Value> {
    match comp.get("studioQuality") {
        Some(v) => validate(v.clone()),
        None => preset("production"),
    }
}
pub fn resolve(current: &Value, update: &Value) -> Result<Value> {
    if let Some(name) = update.as_str() {
        return preset(name);
    }
    let patch = update
        .as_object()
        .ok_or_else(|| Error("Expected Studio quality preset or patch".into()))?;
    let mut out = if let Some(name) = patch.get("preset").and_then(Value::as_str) {
        if name == "custom" {
            current.clone()
        } else {
            preset(name)?
        }
    } else {
        current.clone()
    };
    for (k, v) in patch {
        out[k] = v.clone();
    }
    if patch.keys().any(|k| k != "preset") {
        out["preset"] = json!("custom");
    }
    validate(out)
}
pub fn validate(v: Value) -> Result<Value> {
    crate::schema::strict(
        &v,
        &[
            "preset",
            "noiseThreshold",
            "minSamples",
            "maxSamples",
            "timeLimitSeconds",
            "maxBounces",
            "caustics",
            "denoise",
            "motionBlurSamples",
        ],
    )?;
    if !v["preset"]
        .as_str()
        .is_some_and(|s| ["draft", "preview", "production", "final", "custom"].contains(&s))
    {
        return fail("Invalid Studio quality preset");
    }
    for (key, lo, hi, integer) in [
        ("noiseThreshold", 0., 1., false),
        ("minSamples", 1., 4096., true),
        ("maxSamples", 1., 4096., true),
        ("timeLimitSeconds", 0., 3600., false),
        ("maxBounces", 1., 32., true),
        ("motionBlurSamples", 1., 64., true),
    ] {
        if !v[key]
            .as_f64()
            .is_some_and(|n| n.is_finite() && n >= lo && n <= hi && (!integer || n.fract() == 0.))
        {
            return fail(format!("Invalid Studio quality {key}"));
        }
    }
    if v["minSamples"].as_f64() > v["maxSamples"].as_f64() {
        return fail("Minimum samples must not exceed maximum samples");
    }
    for key in ["caustics", "denoise"] {
        if !v[key].is_boolean() {
            return fail(format!("Invalid Studio quality {key}"));
        }
    }
    Ok(v)
}
