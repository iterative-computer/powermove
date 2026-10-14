//! The text layer content the level 1 layout reads: `layer.d` after
//! raster.ts `resolvedTextContent` (animated box sizes and font axes already
//! evaluated, `paragraph` set from `boxWidth`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextContent {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub font: Option<String>,
    #[serde(default)]
    pub weight: Option<f64>,
    #[serde(default)]
    pub size: Option<f64>,
    #[serde(default)]
    pub tracking: Option<f64>,
    #[serde(default)]
    pub leading: Option<f64>,
    #[serde(default)]
    pub align: Option<String>,
    #[serde(default)]
    pub italic: Option<bool>,
    #[serde(default)]
    pub box_width: Option<f64>,
    #[serde(default)]
    pub box_height: Option<f64>,
    #[serde(default)]
    pub paragraph: Option<bool>,
    /// `fontAxis.<tag>` entries with finite numeric values, sorted by tag
    /// (`variationEntries`).
    #[serde(skip)]
    pub axes: Vec<(String, f64)>,
}

/// `isAxisTag`: four printable ASCII characters.
pub fn is_axis_tag(t: &str) -> bool {
    t.len() == 4 && t.bytes().all(|b| (0x20..=0x7E).contains(&b))
}

fn number(v: &Value) -> Option<f64> {
    v.as_f64()
}

/// JavaScript truthiness for the boolean fields (`d.italic ? … : …`).
fn truthy(v: &Value) -> Option<bool> {
    match v {
        Value::Null => None,
        Value::Bool(b) => Some(*b),
        Value::Number(n) => Some(n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan())),
        Value::String(s) => Some(!s.is_empty()),
        Value::Array(_) | Value::Object(_) => Some(true),
    }
}

impl TextContent {
    /// Reads a resolved content object as the legacy code reads it: missing
    /// and non-numeric fields fall back exactly where `||` and `Number()`
    /// would, `text` is `String(d.text ?? '')`, and every `fontAxis.<tag>`
    /// key with a finite number is an axis.
    pub fn from_json(v: &Value) -> TextContent {
        let get = |k: &str| v.get(k).filter(|x| !x.is_null());
        let text = match get("text") {
            None => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(n)) => js_number_string(n.as_f64().unwrap_or(f64::NAN)),
            Some(Value::Bool(b)) => b.to_string(),
            Some(other) => other.to_string(),
        };
        let mut axes: Vec<(String, f64)> = v
            .as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(k, x)| {
                        let t = k.strip_prefix("fontAxis.")?;
                        let n = x.as_f64().filter(|n| n.is_finite())?;
                        is_axis_tag(t).then(|| (t.to_string(), n))
                    })
                    .collect()
            })
            .unwrap_or_default();
        axes.sort_by(|a, b| a.0.cmp(&b.0));
        TextContent {
            text,
            font: get("font").map(|f| match f {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            }),
            weight: get("weight").and_then(number),
            size: get("size").and_then(number),
            tracking: get("tracking").and_then(number),
            leading: get("leading").and_then(number),
            align: get("align").and_then(|a| a.as_str()).map(str::to_string),
            italic: get("italic").and_then(truthy),
            box_width: get("boxWidth").and_then(number),
            box_height: get("boxHeight").and_then(number),
            paragraph: get("paragraph").and_then(truthy),
            axes,
        }
    }

    /// `Math.max(1, Number(d.size) || 16)`.
    pub fn size(&self) -> f64 {
        let s = self
            .size
            .filter(|s| *s != 0.0 && !s.is_nan())
            .unwrap_or(16.0);
        s.max(1.0)
    }

    /// `size * (d.leading || 1.15)`.
    pub fn line_height(&self) -> f64 {
        let l = self
            .leading
            .filter(|l| *l != 0.0 && !l.is_nan())
            .unwrap_or(1.15);
        self.size() * l
    }

    pub fn align(&self) -> Align {
        match self.align.as_deref() {
            Some("center") => Align::Center,
            Some("right") => Align::Right,
            _ => Align::Left,
        }
    }

    pub fn utf16(&self) -> Vec<u16> {
        self.text.encode_utf16().collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Align {
    Left,
    Center,
    Right,
}

/// `String(n)` for the numbers JSON can carry (integers print without a
/// fraction; others use the shortest round-trip form, as JavaScript does for
/// the magnitudes text content holds).
fn js_number_string(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e21 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_resolved_content_like_the_legacy_code() {
        let d = TextContent::from_json(&json!({
            "text": "Hi", "size": 0, "weight": 700, "italic": 1, "boxWidth": 120, "paragraph": true,
            "fontAxis.wght": 650, "fontAxis.toolong": 1, "fontAxis.opsz": "x", "fontAxis.GRAD": 10
        }));
        assert_eq!(d.text, "Hi");
        assert_eq!(d.size(), 16.0);
        assert_eq!(d.italic, Some(true));
        assert_eq!(
            d.axes,
            vec![("GRAD".to_string(), 10.0), ("wght".to_string(), 650.0)]
        );
        assert_eq!(TextContent::from_json(&json!({"text": 42})).text, "42");
        assert_eq!(
            TextContent::from_json(&json!({})).line_height(),
            16.0 * 1.15
        );
    }
}
