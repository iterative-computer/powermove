//! Inspector shader annotations and authored uniform reconciliation. This is
//! the pinned inspector's Number-based parser, independent of renderer setup.
use super::values;
use pm_model::{JsString, Map, Value};

pub const TEMPLATE: &str = r#"// Powermove shader layer — GLSL ES 3.0
// uv is 0..1 across the layer. Declare uniforms with @param to get inspector controls.
uniform float uSpeed;   // @param 0.35 0 3
uniform float uScale;   // @param 3.0 0.5 12
uniform vec3  uTint;    // @param #FF6B1A

void main() {
  vec2 p = (uv - 0.5) * vec2(iResolution.x / iResolution.y, 1.0);
  float t = iTime * uSpeed;
  float n = fbm(p * uScale + vec2(t, t * 0.6));
  n += 0.35 * fbm(p * uScale * 2.4 - t * 0.4);
  float band = smoothstep(0.28, 0.72, n);
  vec3 col = mix(vec3(0.03, 0.03, 0.04), uTint, band);
  col += pow(band, 6.0) * 0.7;
  float vig = 1.0 - 0.7 * dot(p, p);
  fragColor = vec4(col * vig, 1.0);
}"#;

fn space(c: u16) -> bool {
    matches!(c, 9..=13 | 32 | 0xa0 | 0x1680 | 0x2000..=0x200a | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000 | 0xfeff)
}
fn skip_space(units: &[u16], mut i: usize) -> usize {
    while units.get(i).is_some_and(|c| space(*c)) {
        i += 1;
    }
    i
}
fn literal(units: &[u16], i: usize, text: &str) -> bool {
    units
        .get(i..i + text.len())
        .is_some_and(|a| a.iter().copied().eq(text.bytes().map(u16::from)))
}
/// Mirror the inspector's global regular expression over UTF-16, including its
/// unanchored matches, JavaScript whitespace and next-line annotations. Lone
/// surrogates in code and color defaults remain their original code units.
pub fn parameters(code: &JsString) -> Vec<Value> {
    let units: Vec<_> = code.utf16_units().collect();
    let mut output = Vec::new();
    let mut at = 0;
    while at < units.len() {
        if !literal(&units, at, "uniform") {
            at += 1;
            continue;
        }
        let start = at;
        at += 1;
        let mut i = start + 7;
        let begin = i;
        i = skip_space(&units, i);
        if i == begin {
            continue;
        }
        let Some(ty) = ["float", "vec2", "vec3", "vec4", "int", "bool"]
            .into_iter()
            .find(|ty| {
                literal(&units, i, ty) && units.get(i + ty.len()).is_some_and(|c| space(*c))
            })
        else {
            continue;
        };
        i = skip_space(&units, i + ty.len());
        let begin = i;
        while units
            .get(i)
            .is_some_and(|c| matches!(c,48..=57|65..=90|95|97..=122))
        {
            i += 1;
        }
        if i == begin {
            continue;
        }
        let name = JsString::from_utf16_units(&units[begin..i]);
        i = skip_space(&units, i);
        if units.get(i) != Some(&59) {
            continue;
        }
        i = skip_space(&units, i + 1);
        let mut annotation = &[][..];
        if literal(&units, i, "//") {
            let j = skip_space(&units, i + 2);
            if literal(&units, j, "@param") {
                let begin = skip_space(&units, j + 6);
                i = begin;
                while units.get(i).is_some_and(|c| *c != 10) {
                    i += 1;
                }
                annotation = &units[begin..i];
            }
        }
        at = i;
        if matches!(
            name.as_str(),
            Some("iResolution" | "iTime" | "iGlobalTime" | "iProgress" | "iFrame" | "iMouse")
        ) {
            continue;
        }
        let tokens: Vec<_> = annotation
            .split(|c| space(*c))
            .filter(|s| !s.is_empty())
            .collect();
        let number = |n: usize| {
            tokens.get(n).map(|s| {
                pm_model::json::string_to_number(&JsString::from_utf16_units(s).to_string_lossy())
            })
        };
        let mut m = Map::new();
        m.insert("name", Value::from(name.clone()));
        m.insert("type", Value::from(ty));
        let text = name.as_str().unwrap();
        let chars: Vec<_> = text.strip_prefix('u').unwrap_or(text).chars().collect();
        let mut label = String::new();
        let mut n = 0;
        while n < chars.len() {
            label.push(chars[n]);
            if chars[n].is_ascii_lowercase()
                && chars.get(n + 1).is_some_and(char::is_ascii_uppercase)
            {
                label.push(' ');
                n += 1;
                label.push(chars[n]);
            }
            n += 1;
        }
        m.insert("label", Value::from(label));
        if ty == "vec3" && tokens.first().is_some_and(|s| s.first() == Some(&35)) {
            m.insert("control", Value::from("color"));
            m.insert("def", Value::from(JsString::from_utf16_units(tokens[0])));
        } else if ty == "bool" {
            m.insert("control", Value::from("toggle"));
            m.insert(
                "def",
                Value::Bool(
                    tokens
                        .first()
                        .is_some_and(|s| s.iter().copied().eq("true".bytes().map(u16::from))),
                ),
            );
        } else {
            let default = number(0).filter(|n| !n.is_nan()).unwrap_or(0.);
            m.insert("control", Value::from("num"));
            m.insert("def", Value::Number(default));
            m.insert(
                "min",
                Value::Number(number(1).unwrap_or(if default < 0. { default * 2. } else { 0. })),
            );
            m.insert(
                "max",
                Value::Number(number(2).unwrap_or_else(|| (default.abs() * 2.).max(1.))),
            );
        }
        output.push(Value::Object(m));
    }
    output
}

pub(crate) fn sync(layer: &mut Value) {
    let layer = values::obj(layer);
    if layer.get("d").and_then(Value::as_object).is_none() {
        layer.insert("d", Value::Object(Map::new()));
    }
    let content = layer.get_mut("d").unwrap();
    let code = content
        .get("code")
        .filter(|v| !matches!(v, Value::Null))
        .map(Value::to_js_string)
        .unwrap_or_default();
    let definitions = parameters(&code);
    let d = values::obj(content);
    let mut uniforms = d
        .get("uniforms")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for definition in &definitions {
        let key = definition.get("name").unwrap().as_js_string().unwrap();
        if !uniforms.get_key(key).is_some_and(Value::is_truthy) {
            uniforms.insert(
                key.clone(),
                values::property(definition.get("def").unwrap().clone()),
            );
        }
    }
    uniforms.as_members_mut().retain(|key, _| {
        definitions
            .iter()
            .any(|d| d.get("name").and_then(Value::as_js_string) == Some(key))
    });
    d.insert("uniforms", Value::Object(uniforms));
}
