//! Declarative audio effects. Validation and binding happen on the control
//! thread; only resolved, bounded processor settings enter the DSP engine.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_NODES: usize = 32;
pub const MAX_PARAMETERS: usize = 64;
pub const MAX_DELAY_SECONDS: f64 = 2.0;
/// Total stereo delay storage per effect, at the maximum supported rate.
pub const MAX_DELAY_FRAMES: usize = 768_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub id: String,
    pub label: String,
    pub group: String,
    pub params: Vec<Parameter>,
    pub graph: Vec<Processor>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Parameter {
    #[serde(rename = "number")]
    Number {
        k: String,
        label: String,
        min: f64,
        max: f64,
        unit: String,
        def: f64,
    },
    #[serde(rename = "toggle")]
    Boolean { k: String, label: String, def: bool },
    #[serde(rename = "select")]
    Enum {
        k: String,
        label: String,
        options: Vec<String>,
        def: String,
    },
}

impl Parameter {
    pub fn key(&self) -> &str {
        match self {
            Self::Number { k, .. } | Self::Boolean { k, .. } | Self::Enum { k, .. } => k,
        }
    }
    pub fn default_value(&self) -> Value {
        match self {
            Self::Number { def, .. } => Value::from(*def),
            Self::Boolean { def, .. } => Value::from(*def),
            Self::Enum { def, .. } => Value::from(def.clone()),
        }
    }
    pub fn validate_value(&self, value: &Value) -> Result<(), Error> {
        let valid = match self {
            Self::Number { min, max, .. } => value
                .as_f64()
                .is_some_and(|v| v.is_finite() && v >= *min && v <= *max),
            Self::Boolean { .. } => value.is_boolean(),
            Self::Enum { options, .. } => value
                .as_str()
                .is_some_and(|v| options.iter().any(|o| o == v)),
        };
        if valid {
            Ok(())
        } else {
            Err(Error(format!(
                "params.{}: invalid value or outside declared range",
                self.key()
            )))
        }
    }
}

/// Exactly one constant or parameter reference, never an expression or callback.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Binding<T> {
    Constant { value: T },
    Parameter { param: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Filter {
    LowPass,
    HighPass,
    Peaking,
    LowShelf,
    HighShelf,
    Notch,
}
impl Filter {
    fn name(self) -> &'static str {
        match self {
            Self::LowPass => "lowPass",
            Self::HighPass => "highPass",
            Self::Peaking => "peaking",
            Self::LowShelf => "lowShelf",
            Self::HighShelf => "highShelf",
            Self::Notch => "notch",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Processor {
    Gain {
        #[serde(rename = "gainDb")]
        gain_db: Binding<f64>,
    },
    Biquad {
        mode: Binding<Filter>,
        frequency: Binding<f64>,
        q: Binding<f64>,
        #[serde(rename = "gainDb")]
        gain_db: Binding<f64>,
    },
    Compressor {
        threshold: Binding<f64>,
        ratio: Binding<f64>,
        attack: Binding<f64>,
        release: Binding<f64>,
        knee: Binding<f64>,
        makeup: Binding<f64>,
    },
    Limiter {
        ceiling: Binding<f64>,
        release: Binding<f64>,
    },
    Delay {
        time: Binding<f64>,
        feedback: Binding<f64>,
        mix: Binding<f64>,
        #[serde(rename = "maxTime")]
        max_time: f64,
    },
    /// A smoothed boolean signal gate; downstream processors still run.
    Gate { enabled: Binding<bool> },
}

/// Resolved settings. Fields remain private to construction through validation.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings(pub(crate) Vec<Resolved>);
impl Settings {
    pub fn nodes(&self) -> &[Resolved] {
        &self.0
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Resolved {
    Gain {
        gain_db: f64,
    },
    Biquad {
        mode: Filter,
        frequency: f64,
        q: f64,
        gain_db: f64,
    },
    Compressor {
        threshold: f64,
        ratio: f64,
        attack: f64,
        release: f64,
        knee: f64,
        makeup: f64,
    },
    Limiter {
        ceiling: f64,
        release: f64,
    },
    Delay {
        time: f64,
        feedback: f64,
        mix: f64,
        max_time: f64,
    },
    Gate {
        enabled: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
fn fail<T>(field: &str, reason: &str) -> Result<T, Error> {
    Err(Error(format!("{field}: {reason}")))
}
fn text(field: &str, value: &str) -> Result<(), Error> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        fail(field, "expected 1..128 printable bytes")
    } else {
        Ok(())
    }
}
fn range(field: &str, value: f64, lo: f64, hi: f64) -> Result<f64, Error> {
    if !value.is_finite() || value < lo || value > hi {
        fail(field, &format!("expected finite number in {lo}..={hi}"))
    } else {
        Ok(value)
    }
}

/// A validated immutable registration. Callers cannot manufacture one without
/// validating every possible parameter binding, including its entire range.
#[derive(Clone, Debug, PartialEq)]
pub struct Validated(Definition);
impl Validated {
    pub fn definition(&self) -> &Definition {
        &self.0
    }
    pub fn resolve(&self, values: &BTreeMap<String, Value>) -> Result<Settings, Error> {
        for (key, value) in values {
            let p = self
                .0
                .params
                .iter()
                .find(|p| p.key() == key)
                .ok_or_else(|| Error(format!("params.{key}: unknown parameter")))?;
            p.validate_value(value)?;
        }
        self.0.resolve(values)
    }
}
impl Definition {
    pub fn validate(self) -> Result<Validated, Error> {
        text("id", &self.id)?;
        if self.id.len() > 64
            || !self.id.as_bytes()[0].is_ascii_alphabetic()
            || !self
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        {
            return fail("id", "expected stable ASCII identifier");
        }
        text("label", &self.label)?;
        text("group", &self.group)?;
        if self.params.len() > MAX_PARAMETERS {
            return fail("params", "too many parameters");
        }
        if self.graph.is_empty() || self.graph.len() > MAX_NODES {
            return fail("graph", "expected 1..32 nodes");
        }
        let mut keys = BTreeSet::new();
        for (i, p) in self.params.iter().enumerate() {
            let field = format!("params[{i}]");
            text(&format!("{field}.k"), p.key())?;
            if !p.key().as_bytes()[0].is_ascii_lowercase()
                || !p.key().bytes().skip(1).all(|c| c.is_ascii_alphanumeric())
            {
                return fail(
                    &format!("{field}.k"),
                    "expected lower camel case parameter key",
                );
            }
            if !keys.insert(p.key()) {
                return fail(&format!("{field}.k"), "duplicate key");
            }
            match p {
                Parameter::Number {
                    label,
                    min,
                    max,
                    unit,
                    def,
                    ..
                } => {
                    text(&format!("{field}.label"), label)?;
                    text(&format!("{field}.unit"), unit)?;
                    range(&format!("{field}.min"), *min, -1e9, 1e9)?;
                    range(&format!("{field}.max"), *max, *min, 1e9)?;
                    range(&format!("{field}.def"), *def, *min, *max)?;
                }
                Parameter::Boolean { label, .. } => text(&format!("{field}.label"), label)?,
                Parameter::Enum {
                    label,
                    options,
                    def,
                    ..
                } => {
                    text(&format!("{field}.label"), label)?;
                    if options.is_empty() || options.len() > 32 {
                        return fail(&format!("{field}.options"), "expected 1..32 options");
                    }
                    let mut seen = BTreeSet::new();
                    for option in options {
                        text(&format!("{field}.options"), option)?;
                        if !seen.insert(option) {
                            return fail(&format!("{field}.options"), "duplicate option");
                        }
                    }
                    if !options.contains(def) {
                        return fail(&format!("{field}.def"), "not an option");
                    }
                }
            }
        }
        self.resolve(&BTreeMap::new())?;
        Ok(Validated(self))
    }
    fn parameter(&self, field: &str, key: &str) -> Result<&Parameter, Error> {
        self.params
            .iter()
            .find(|p| p.key() == key)
            .ok_or_else(|| Error(format!("{field}.param: unknown parameter {key}")))
    }
    fn number(
        &self,
        field: &str,
        b: &Binding<f64>,
        lo: f64,
        hi: f64,
        values: &BTreeMap<String, Value>,
    ) -> Result<f64, Error> {
        let v = match b {
            Binding::Constant { value } => *value,
            Binding::Parameter { param } => match self.parameter(field, param)? {
                Parameter::Number { min, max, def, .. } => {
                    range(&format!("{field}.param.min"), *min, lo, hi)?;
                    range(&format!("{field}.param.max"), *max, lo, hi)?;
                    values.get(param).and_then(Value::as_f64).unwrap_or(*def)
                }
                _ => return fail(field, "requires number parameter"),
            },
        };
        range(field, v, lo, hi)
    }
    fn resolve(&self, values: &BTreeMap<String, Value>) -> Result<Settings, Error> {
        let mut nodes = Vec::with_capacity(self.graph.len());
        let mut delay_frames = 0;
        for (i, node) in self.graph.iter().enumerate() {
            let field = format!("graph[{i}]");
            let n = |k: &str, b: &Binding<f64>, lo, hi| {
                self.number(&format!("{field}.{k}"), b, lo, hi, values)
            };
            nodes.push(match node {
                Processor::Gain { gain_db } => Resolved::Gain {
                    gain_db: n("gainDb", gain_db, -96.0, 24.0)?,
                },
                Processor::Biquad {
                    mode,
                    frequency,
                    q,
                    gain_db,
                } => {
                    let mode = match mode {
                        Binding::Constant { value } => *value,
                        Binding::Parameter { param } => match self
                            .parameter(&format!("{field}.mode"), param)?
                        {
                            Parameter::Enum { options, def, .. } => {
                                for o in options {
                                    if ![
                                        Filter::LowPass,
                                        Filter::HighPass,
                                        Filter::Peaking,
                                        Filter::LowShelf,
                                        Filter::HighShelf,
                                        Filter::Notch,
                                    ]
                                    .iter()
                                    .any(|m| m.name() == o)
                                    {
                                        return fail(
                                            &format!("{field}.mode.param.options"),
                                            "invalid filter mode",
                                        );
                                    }
                                }
                                serde_json::from_value(
                                    values
                                        .get(param)
                                        .cloned()
                                        .unwrap_or_else(|| Value::from(def.clone())),
                                )
                                .map_err(|_| Error(format!("{field}.mode: invalid filter mode")))?
                            }
                            _ => return fail(&format!("{field}.mode"), "requires enum parameter"),
                        },
                    };
                    Resolved::Biquad {
                        mode,
                        frequency: n("frequency", frequency, 10.0, 20_000.0)?,
                        q: n("q", q, 0.1, 20.0)?,
                        gain_db: n("gainDb", gain_db, -24.0, 24.0)?,
                    }
                }
                Processor::Compressor {
                    threshold,
                    ratio,
                    attack,
                    release,
                    knee,
                    makeup,
                } => Resolved::Compressor {
                    threshold: n("threshold", threshold, -96.0, 0.0)?,
                    ratio: n("ratio", ratio, 1.0, 100.0)?,
                    attack: n("attack", attack, 0.0001, 1.0)?,
                    release: n("release", release, 0.001, 10.0)?,
                    knee: n("knee", knee, 0.0, 24.0)?,
                    makeup: n("makeup", makeup, 0.0, 24.0)?,
                },
                Processor::Limiter { ceiling, release } => Resolved::Limiter {
                    ceiling: n("ceiling", ceiling, -96.0, 0.0)?,
                    release: n("release", release, 0.001, 10.0)?,
                },
                Processor::Delay {
                    time,
                    feedback,
                    mix,
                    max_time,
                } => {
                    range(
                        &format!("{field}.maxTime"),
                        *max_time,
                        1.0 / 8000.0,
                        MAX_DELAY_SECONDS,
                    )?;
                    delay_frames += (*max_time * 192_000.0).ceil() as usize + 1;
                    if delay_frames > MAX_DELAY_FRAMES {
                        return fail(
                            &format!("{field}.maxTime"),
                            "total delay memory exceeds budget",
                        );
                    }
                    Resolved::Delay {
                        time: n("time", time, 1.0 / 8000.0, *max_time)?,
                        feedback: n("feedback", feedback, -0.95, 0.95)?,
                        mix: n("mix", mix, 0.0, 1.0)?,
                        max_time: *max_time,
                    }
                }
                Processor::Gate { enabled } => Resolved::Gate {
                    enabled: match enabled {
                        Binding::Constant { value } => *value,
                        Binding::Parameter { param } => {
                            match self.parameter(&format!("{field}.enabled"), param)? {
                                Parameter::Boolean { def, .. } => {
                                    values.get(param).and_then(Value::as_bool).unwrap_or(*def)
                                }
                                _ => {
                                    return fail(
                                        &format!("{field}.enabled"),
                                        "requires boolean parameter",
                                    );
                                }
                            }
                        }
                    },
                },
            });
        }
        Ok(Settings(nodes))
    }
}

/// Registration boundary for untrusted JSON. Prefix structural decoding
/// errors with their parameter/node path before semantic validation.
pub fn parse(value: Value) -> Result<Validated, Error> {
    let object = value
        .as_object()
        .ok_or_else(|| Error("definition: expected object".into()))?;
    for key in ["id", "label", "group", "params", "graph"] {
        if !object.contains_key(key) {
            return fail(key, "required field");
        }
    }
    for key in ["id", "label", "group"] {
        if !object[key].is_string() {
            return fail(key, "expected string");
        }
    }
    for key in ["params", "graph"] {
        let list = object[key]
            .as_array()
            .ok_or_else(|| Error(format!("{key}: expected array")))?;
        let max = if key == "params" {
            MAX_PARAMETERS
        } else {
            MAX_NODES
        };
        if list.len() > max {
            return fail(key, "count exceeds limit");
        }
        for (i, item) in list.iter().enumerate() {
            let field = format!("{key}[{i}]");
            if key == "params" {
                for name in ["k", "label", "type"] {
                    if !item[name].is_string() {
                        return fail(&format!("{field}.{name}"), "expected string");
                    }
                }
                if !["number", "toggle", "select"]
                    .contains(&item["type"].as_str().unwrap_or_default())
                {
                    return fail(&format!("{field}.type"), "unknown parameter type");
                }
                if item["type"] == "select" {
                    let options = item["options"]
                        .as_array()
                        .ok_or_else(|| Error(format!("{field}.options: expected array")))?;
                    for (index, option) in options.iter().enumerate() {
                        if !option.is_string() {
                            return fail(
                                &format!("{field}.options[{index}]"),
                                "expected enum string",
                            );
                        }
                    }
                }
                if item["type"] == "number" {
                    for name in ["min", "max", "def"] {
                        if item[name].as_f64().is_none() {
                            return fail(&format!("{field}.{name}"), "expected finite number");
                        }
                    }
                    if !item["unit"].is_string() {
                        return fail(&format!("{field}.unit"), "expected string");
                    }
                } else if item["type"] == "toggle" && !item["def"].is_boolean() {
                    return fail(&format!("{field}.def"), "expected boolean");
                } else if item["type"] == "select" && !item["def"].is_string() {
                    return fail(&format!("{field}.def"), "expected enum string");
                }
                serde_json::from_value::<Parameter>(item.clone())
                    .map_err(|e| Error(format!("{field}: {e}")))?;
            } else {
                if !item["type"].is_string() {
                    return fail(&format!("{field}.type"), "expected processor type");
                }
                if !["gain", "biquad", "compressor", "limiter", "delay", "gate"]
                    .contains(&item["type"].as_str().unwrap_or_default())
                {
                    return fail(&format!("{field}.type"), "unknown processor type");
                }
                if let Some(node) = item.as_object() {
                    for (name, binding) in node {
                        if name == "maxTime" && binding.as_f64().is_none() {
                            return fail(&format!("{field}.maxTime"), "expected finite number");
                        }
                        if name != "type" && name != "maxTime" {
                            let parsed = if name == "mode" {
                                serde_json::from_value::<Binding<Filter>>(binding.clone())
                                    .map(|_| ())
                            } else if name == "enabled" {
                                serde_json::from_value::<Binding<bool>>(binding.clone()).map(|_| ())
                            } else {
                                serde_json::from_value::<Binding<f64>>(binding.clone()).map(|_| ())
                            };
                            parsed.map_err(|e| Error(format!("{field}.{name}: {e}")))?;
                            let binding = binding.as_object().ok_or_else(|| {
                                Error(format!(
                                    "{field}.{name}: expected constant or parameter binding"
                                ))
                            })?;
                            if binding.len() != 1
                                || !(binding.contains_key("param") || binding.contains_key("value"))
                            {
                                return fail(
                                    &format!("{field}.{name}"),
                                    "expected exactly one of value or param",
                                );
                            }
                            if binding.get("value").is_some_and(Value::is_null) {
                                return fail(
                                    &format!("{field}.{name}.value"),
                                    "null is not a setting",
                                );
                            }
                        }
                    }
                }
                serde_json::from_value::<Processor>(item.clone())
                    .map_err(|e| Error(format!("{field}: {e}")))?;
            }
        }
    }
    serde_json::from_value::<Definition>(value)
        .map_err(|e| Error(format!("definition: {e}")))?
        .validate()
}

/// Generated from the shipped extension through its public activation API,
/// exactly as visual built-ins are data at native startup.
pub fn builtins() -> &'static [Validated] {
    static DEFINITIONS: std::sync::OnceLock<Vec<Validated>> = std::sync::OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        serde_json::from_str::<Vec<Definition>>(include_str!("audio-builtins.json"))
            .expect("generated audio definitions")
            .into_iter()
            .map(|d| d.validate().expect("valid built-in audio definition"))
            .collect()
    })
}
