//! The evaluable part of a layer, derived from its JSON.
//!
//! The spike mirrors layers with transform channels and numeric keyframes
//! with speed and influence ease. Anything outside that scope (expressions,
//! springs, discrete values, keys saved before the speed/influence model) is
//! marked unsupported and answers NaN, never a guess.

use crate::ease::{Ease, Interp, Key, Track, make_ease};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum PropEval {
    Static(f64),
    Track(Track),
    Unsupported(&'static str),
}

impl PropEval {
    /// `PM.evP` for a mirrored property at layer-local time.
    #[inline]
    pub fn value(&self, t_local: f64) -> f64 {
        match self {
            PropEval::Static(v) => *v,
            PropEval::Track(track) => track.value(t_local),
            PropEval::Unsupported(_) => f64::NAN,
        }
    }

    pub fn from_json(prop: &Value) -> PropEval {
        let Some(obj) = prop.as_object() else {
            return PropEval::Unsupported("property is not an object");
        };
        if let Some(expr) = obj.get("expr") {
            // `if (prop.expr)`: only a truthy expression runs.
            let truthy = match expr {
                Value::Null => false,
                Value::Bool(b) => *b,
                Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
                Value::String(s) => !s.is_empty(),
                _ => true,
            };
            if truthy {
                return PropEval::Unsupported("expressions are outside the spike");
            }
        }
        let keys = obj.get("kf").and_then(Value::as_array);
        match keys {
            Some(keys) if !keys.is_empty() => match track_from_json(keys) {
                Ok(track) => PropEval::Track(track),
                Err(reason) => PropEval::Unsupported(reason),
            },
            _ => match obj.get("v") {
                Some(Value::Number(n)) => PropEval::Static(n.as_f64().unwrap_or(f64::NAN)),
                _ => PropEval::Unsupported("non-numeric static value"),
            },
        }
    }
}

fn finite(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64).filter(|v| v.is_finite())
}

fn ease_from_json(value: Option<&Value>) -> Option<Ease> {
    let obj = value?.as_object()?;
    let speed = finite(obj.get("speed"))?;
    let influence = finite(obj.get("influence"))?;
    Some(make_ease(speed, influence))
}

fn interp_from_json(value: Option<&Value>) -> Option<Interp> {
    match value?.as_str()? {
        "linear" => Some(Interp::Linear),
        "bezier" => Some(Interp::Bezier),
        "hold" => Some(Interp::Hold),
        _ => None,
    }
}

fn track_from_json(keys: &[Value]) -> Result<Track, &'static str> {
    let mut out = Vec::with_capacity(keys.len());
    for key in keys {
        let obj = key.as_object().ok_or("keyframe is not an object")?;
        if obj
            .get("spring")
            .is_some_and(|s| !s.is_null() && s != &Value::Bool(false))
        {
            return Err("spring keys are outside the spike");
        }
        let t = obj
            .get("t")
            .and_then(Value::as_f64)
            .ok_or("keyframe time is not a number")?;
        let v = match obj.get("v") {
            Some(Value::Number(n)) => n.as_f64().ok_or("keyframe value is not a number")?,
            _ => return Err("discrete keyframe values are outside the spike"),
        };
        // hasNativeEase: older files carry eo/ei handles and are converted by
        // adoptTemporalEase, which the spike does not port.
        let (Some(in_interp), Some(out_interp), Some(in_ease), Some(out_ease)) = (
            interp_from_json(obj.get("inInterp")),
            interp_from_json(obj.get("outInterp")),
            ease_from_json(obj.get("inEase")),
            ease_from_json(obj.get("outEase")),
        ) else {
            return Err("keys without speed/influence ease are outside the spike");
        };
        out.push(Key {
            t,
            v,
            in_interp,
            out_interp,
            in_ease,
            out_ease,
            auto_bezier: obj.get("autoBezier") == Some(&Value::Bool(true)),
            continuous: obj.get("continuous") == Some(&Value::Bool(true)),
        });
    }
    // The document keeps keys sorted (PM.normalizeKeyframes); a stable sort
    // only guards hand-written input.
    if out.windows(2).any(|w| w[0].t > w[1].t) {
        out.sort_by(|a, b| a.t.total_cmp(&b.t));
    }
    Ok(Track::new(out))
}

/// Everything the evaluator needs from one layer.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerData {
    pub id: Option<Box<str>>,
    pub is_group: bool,
    pub from: f64,
    pub parent: Option<Box<str>>,
    pub group: Option<Box<str>>,
    pub channels: Vec<(Box<str>, PropEval)>,
}

/// JS `T - L.from` coerces: null is 0, a missing or non-numeric field is NaN.
fn js_number(value: Option<&Value>) -> f64 {
    match value {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        _ => f64::NAN,
    }
}

/// `if (!L.parent)`: only a non-empty string links.
fn link(value: Option<&Value>) -> Option<Box<str>> {
    match value {
        Some(Value::String(s)) if !s.is_empty() => Some(s.as_str().into()),
        _ => None,
    }
}

impl LayerData {
    pub fn from_json(layer: &Value) -> LayerData {
        let obj = layer.as_object();
        let get = |k: &str| obj.and_then(|o| o.get(k));
        let channels = get("p")
            .and_then(Value::as_object)
            .map(|p| {
                p.iter()
                    .map(|(k, v)| (k.as_str().into(), PropEval::from_json(v)))
                    .collect()
            })
            .unwrap_or_default();
        LayerData {
            id: get("id").and_then(Value::as_str).map(Into::into),
            is_group: get("type").and_then(Value::as_str) == Some("group"),
            from: js_number(get("from")),
            parent: link(get("parent")),
            group: link(get("group")),
            channels,
        }
    }

    pub fn channel_index(&self, key: &str) -> Option<usize> {
        self.channels.iter().position(|(k, _)| &**k == key)
    }

    /// `PM.ev(L, key, T)`: a missing channel answers 0.
    pub fn ev(&self, key: &str, time: f64) -> f64 {
        match self.channel_index(key) {
            Some(i) => self.channels[i].1.value(time - self.from),
            None => 0.0,
        }
    }

    pub fn unsupported(&self) -> impl Iterator<Item = (&str, &'static str)> {
        self.channels.iter().filter_map(|(k, p)| match p {
            PropEval::Unsupported(r) => Some((&**k, *r)),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn derives_transform_channels() {
        let layer = json!({
            "id": "L1", "type": "shape", "from": 1.5, "parent": null, "group": "G1",
            "p": {
                "position.x": {"v": 10, "kf": [], "expr": null},
                "opacity": {"v": 100, "kf": [
                    {"t": 0, "v": 0, "i": "k1", "inInterp": "linear", "outInterp": "linear",
                     "inEase": {"speed": 0, "influence": 33.3}, "outEase": {"speed": 0, "influence": 33.3},
                     "autoBezier": false, "continuous": false},
                    {"t": 1, "v": 100, "i": "k2", "inInterp": "linear", "outInterp": "linear",
                     "inEase": {"speed": 0, "influence": 33.3}, "outEase": {"speed": 0, "influence": 33.3},
                     "autoBezier": false, "continuous": false}
                ], "expr": null}
            }
        });
        let data = LayerData::from_json(&layer);
        assert_eq!(data.id.as_deref(), Some("L1"));
        assert_eq!(data.group.as_deref(), Some("G1"));
        assert_eq!(data.parent, None);
        assert_eq!(data.ev("position.x", 0.0), 10.0);
        assert_eq!(data.ev("opacity", 2.0), 50.0);
        assert_eq!(data.ev("missing", 2.0), 0.0);
    }

    #[test]
    fn marks_out_of_scope_properties() {
        let expr = PropEval::from_json(&json!({"v": 1, "kf": [], "expr": "time*2"}));
        assert!(matches!(expr, PropEval::Unsupported(_)));
        assert!(expr.value(0.0).is_nan());
        let legacy = PropEval::from_json(
            &json!({"v": 1, "kf": [{"t": 0, "v": 1, "eo": [0.33, 0], "ei": [0.67, 1]}]}),
        );
        assert!(matches!(legacy, PropEval::Unsupported(_)));
        let text = PropEval::from_json(&json!({"v": "a", "kf": []}));
        assert!(matches!(text, PropEval::Unsupported(_)));
        let empty_expr = PropEval::from_json(&json!({"v": 3, "kf": [], "expr": ""}));
        assert_eq!(empty_expr, PropEval::Static(3.0));
    }
}
