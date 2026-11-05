//! Static typed effect references. These are runtime requests, never animated
//! Channels. Persistence retains unknown records outside this validated view.
use crate::catalog::CatalogError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Space {
    #[default]
    Composition,
    Source,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Source,
    Masks,
    Effects,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LayerInput {
    pub layer_id: Option<String>,
    pub stage: Stage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<Space>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathKind {
    #[serde(rename = "path")]
    Path,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetKind {
    #[serde(rename = "asset")]
    Asset,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PathInput {
    pub kind: PathKind,
    pub layer_id: Option<String>,
    pub path_id: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssetInput {
    pub kind: AssetKind,
    pub asset_id: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EffectInput {
    Layer(LayerInput),
    Path(PathInput),
    Asset(AssetInput),
}
pub type Inputs = BTreeMap<String, EffectInput>;

fn whitespace(c: char) -> bool {
    matches!(c, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}
fn valid_id(id: &str) -> bool {
    id.encode_utf16().count() <= 256 && !id.trim_matches(whitespace).is_empty()
}
fn valid_optional_id(id: &Option<String>) -> bool {
    id.as_deref().is_none_or(valid_id)
}
fn valid_key(key: &str) -> bool {
    key.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && key.bytes().all(|c| c.is_ascii_alphanumeric())
        && !matches!(key, "constructor" | "prototype" | "__proto__")
}
pub fn validate_inputs(value: &Value) -> Result<Inputs, CatalogError> {
    let object = value
        .as_object()
        .ok_or_else(|| CatalogError("effect inputs must be an object".into()))?;
    if object.len() > 4 {
        return Err(CatalogError(
            "at most four effect references are supported".into(),
        ));
    }
    let mut result = BTreeMap::new();
    for (key, raw) in object {
        if !valid_key(key) {
            return Err(CatalogError(format!("invalid effect reference key {key}")));
        }
        // Option fields normally accept missing keys under serde. References
        // require explicit null sentinels, exactly as the extension contract.
        let fields = raw
            .as_object()
            .ok_or_else(|| CatalogError("invalid effect reference".into()))?;
        let required: &[&str] = match raw.get("kind").and_then(Value::as_str) {
            Some("path") => &["layerId", "pathId"],
            Some("asset") => &["assetId"],
            _ => &["layerId", "stage"],
        };
        if required.iter().any(|key| !fields.contains_key(*key)) {
            return Err(CatalogError("missing effect reference field".into()));
        }
        if fields.get("space").is_some_and(Value::is_null) {
            return Err(CatalogError("invalid effect reference space".into()));
        }
        let input: EffectInput = serde_json::from_value(raw.clone())
            .map_err(|_| CatalogError("invalid effect reference".into()))?;
        let valid = match &input {
            EffectInput::Layer(r) => valid_optional_id(&r.layer_id),
            EffectInput::Path(r) => {
                valid_optional_id(&r.layer_id)
                    && valid_optional_id(&r.path_id)
                    && r.layer_id.is_none() == r.path_id.is_none()
            }
            EffectInput::Asset(r) => valid_optional_id(&r.asset_id),
        };
        if !valid {
            return Err(CatalogError("invalid effect reference identity".into()));
        }
        result.insert(key.clone(), input);
    }
    Ok(result)
}

pub fn empty_reference(kind: &str) -> Option<EffectInput> {
    Some(match kind {
        "layer" => EffectInput::Layer(LayerInput {
            layer_id: None,
            stage: Stage::Source,
            space: None,
        }),
        "path" => EffectInput::Path(PathInput {
            kind: PathKind::Path,
            layer_id: None,
            path_id: None,
        }),
        "asset" => EffectInput::Asset(AssetInput {
            kind: AssetKind::Asset,
            asset_id: None,
        }),
        _ => return None,
    })
}

/// Only known layer identities change. Path and asset identities and explicit
/// empty selections survive. Deletion/Undo policy belongs to the single writer.
pub fn remap_layers(inputs: &mut Inputs, ids: &BTreeMap<String, String>) {
    for input in inputs.values_mut() {
        let id = match input {
            EffectInput::Layer(r) => &mut r.layer_id,
            EffectInput::Path(r) => &mut r.layer_id,
            EffectInput::Asset(_) => continue,
        };
        if let Some(mapped) = id.as_ref().and_then(|id| ids.get(id)) {
            *id = Some(mapped.clone());
        }
    }
}

/// Source/masks/effects, processing space, exact timestamp and exclusive prefix
/// all contribute to identity. No quantization to nominal video frames.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Request {
    pub layer_id: String,
    pub stage: Stage,
    pub space: Space,
    time_bits: Option<u64>,
    pub prefix: Option<usize>,
}
impl Request {
    pub fn new(
        layer_id: impl Into<String>,
        stage: Stage,
        space: Space,
        time: Option<f64>,
        prefix: Option<usize>,
    ) -> Result<Self, CatalogError> {
        if time.is_some_and(|t| !t.is_finite()) {
            return Err(CatalogError("non-finite input timestamp".into()));
        }
        if stage != Stage::Effects && prefix.is_some() {
            return Err(CatalogError("prefix requires effects stage".into()));
        }
        Ok(Self {
            layer_id: layer_id.into(),
            stage,
            space,
            time_bits: time.map(|t| if t == 0. { 0 } else { t.to_bits() }),
            prefix,
        })
    }
    pub fn time(&self) -> Option<f64> {
        self.time_bits.map(f64::from_bits)
    }
}

pub fn temporal_times(
    count: f64,
    step: f64,
    time: f64,
    max_samples: u32,
) -> Result<Vec<f64>, CatalogError> {
    let rounded = (count + 0.5).floor(); // ECMAScript Math.round, including negative halves.
    if !count.is_finite()
        || !step.is_finite()
        || count < 0.
        || rounded > f64::from(max_samples.min(32))
        || step.abs() > 3600.
        || !time.is_finite()
    {
        return Err(CatalogError(
            "temporal inputs exceed the sample or time limits".into(),
        ));
    }
    (0..rounded as u32)
        .map(|i| {
            let at = time + f64::from(i + 1) * step;
            if !at.is_finite() {
                Err(CatalogError("invalid temporal input timestamp".into()))
            } else {
                Ok(if at == 0. { 0. } else { at })
            }
        })
        .collect()
}

/// Ordinary stack + masks + ping-pong reserve six surfaces in addition to
/// exact input leases. Checked arithmetic refuses overflow before allocation.
pub fn assert_input_budget(
    count: usize,
    frame_bytes: u64,
    budget: u64,
) -> Result<(), CatalogError> {
    let bytes = u64::try_from(count)
        .ok()
        .and_then(|n| n.checked_add(6))
        .and_then(|n| n.checked_mul(frame_bytes));
    if bytes.is_none_or(|n| n > budget) {
        return Err(CatalogError(
            "layer inputs exceed framebuffer memory budget".into(),
        ));
    }
    Ok(())
}

/// Exact shutter phases; negative shutters keep their authored sample order.
/// Zero options use the reference's existing default (`||`), not a new UX rule.
pub fn motion_blur_times(
    time: f64,
    fps: f64,
    samples: Option<f64>,
    shutter: Option<f64>,
) -> Result<Vec<f64>, CatalogError> {
    let count = samples.filter(|v| *v != 0.).unwrap_or(10.);
    let shutter = shutter.filter(|v| *v != 0.).unwrap_or(0.5);
    if ![time, fps, count, shutter].iter().all(|v| v.is_finite()) {
        return Err(CatalogError("non-finite shutter sampling".into()));
    }
    let count = (count + 0.5).floor().clamp(1., 64.) as u32;
    let interval = shutter / fps.max(1.);
    (0..count)
        .map(|i| {
            let at = time + ((f64::from(i) + 0.5) / f64::from(count) - 0.5) * interval;
            if at.is_finite() {
                Ok(at)
            } else {
                Err(CatalogError("non-finite shutter timestamp".into()))
            }
        })
        .collect()
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("layer_id", &self.layer_id)
            .field("stage", &self.stage)
            .field("space", &self.space)
            .field("time", &self.time())
            .field("prefix", &self.prefix)
            .finish()
    }
}
