//! `PM.parseUniforms` (`legacy/gl/shaders.ts`), ported exactly: the
//! `@param` annotations of a shader layer's code, which
//! `UIState.shaderMeta.udefs` holds and the compositor re-reads to bind
//! `d.uniforms` (`compositor.ts` `uniformDefsFor`).
//!
//! ```js
//! const re = /uniform\s+(float|vec2|vec3|vec4|int|bool)\s+(\w+)\s*;\s*(?:\/\/\s*@param\s*([^\n]*))?/g;
//! ```
//!
//! JavaScript's `\s` (WhiteSpace and LineTerminator, U+FEFF included and
//! U+0085 excluded) and ASCII `\w` are spelled out, since the `regex`
//! crate's classes are Unicode's. `regex` returns the leftmost-first match a
//! backtracking engine returns for this pattern. `\s*` after `;` crosses
//! lines, so an annotation on the next line belongs to the uniform, as
//! today.

use std::sync::OnceLock;

use regex::Regex;

/// JavaScript's `\s`.
const WS: &str = r"[\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}]";

fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

fn pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(&format!(
            r"uniform{WS}+(float|vec2|vec3|vec4|int|bool){WS}+([A-Za-z0-9_]+){WS}*;{WS}*(?://{WS}*@param{WS}*([^\n]*))?"
        ))
        .expect("the parseUniforms pattern compiles")
    })
}

/// The GLSL type of an annotated uniform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UniformType {
    Float,
    Vec2,
    Vec3,
    Vec4,
    Int,
    Bool,
}

impl UniformType {
    fn parse(s: &str) -> UniformType {
        match s {
            "float" => UniformType::Float,
            "vec2" => UniformType::Vec2,
            "vec3" => UniformType::Vec3,
            "vec4" => UniformType::Vec4,
            "int" => UniformType::Int,
            _ => UniformType::Bool,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            UniformType::Float => "float",
            UniformType::Vec2 => "vec2",
            UniformType::Vec3 => "vec3",
            UniformType::Vec4 => "vec4",
            UniformType::Int => "int",
            UniformType::Bool => "bool",
        }
    }
}

/// The inspector control of an annotated uniform, and its default and range.
#[derive(Debug, Clone, PartialEq)]
pub enum Control {
    /// `vec3` with a `#` default: `uniform3f(hex2rgb(value))`.
    Color { default: String },
    /// `bool`: `uniform1i(value ? 1 : 0)`.
    Toggle { default: bool },
    /// Everything else: one `uniform1f(Number(value) || 0)`, so `vec2`,
    /// `vec3` without a color default, `vec4`, and `int` uniforms stay 0.
    Number { default: f64, min: f64, max: f64 },
}

/// One entry of `PM.parseUniforms(code)`.
#[derive(Debug, Clone, PartialEq)]
pub struct UniformDef {
    pub name: String,
    pub ty: UniformType,
    pub label: String,
    pub control: Control,
}

/// JavaScript's `parseFloat`: leading whitespace skipped, then the longest
/// prefix that is a decimal literal (or `Infinity`); `NaN` otherwise.
pub fn parse_float(s: &str) -> f64 {
    let s = s.trim_start_matches(is_js_space);
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut negative = false;
    if let Some(&c) = bytes.first()
        && (c == b'+' || c == b'-')
    {
        negative = c == b'-';
        i = 1;
    }
    if s[i..].starts_with("Infinity") {
        return if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    let digits = |from: usize| {
        bytes[from..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    };
    let start = i;
    let int = digits(i);
    i += int;
    let mut frac = 0;
    if bytes.get(i) == Some(&b'.') {
        frac = digits(i + 1);
        if int > 0 || frac > 0 {
            i += 1 + frac;
        }
    }
    if int == 0 && frac == 0 {
        return f64::NAN;
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(bytes.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        let exp = digits(j);
        if exp > 0 {
            i = j + exp;
        }
    }
    let literal = &s[start..i];
    let value: f64 = literal.parse().unwrap_or(f64::NAN);
    if negative { -value } else { value }
}

/// `name.replace(/^u/, '').replace(/([a-z])([A-Z])/g, '$1 $2')`.
fn label_of(name: &str) -> String {
    let base = name.strip_prefix('u').unwrap_or(name);
    let chars: Vec<char> = base.chars().collect();
    let mut out = String::with_capacity(base.len() + 4);
    let mut i = 0;
    while i < chars.len() {
        out.push(chars[i]);
        // A global replace resumes after each match, so `aBC` becomes
        // `a BC`, not `a B C`.
        if chars[i].is_ascii_lowercase() && chars.get(i + 1).is_some_and(|c| c.is_ascii_uppercase())
        {
            out.push(' ');
            out.push(chars[i + 1]);
            i += 2;
            continue;
        }
        i += 1;
    }
    out
}

fn reserved(name: &str) -> bool {
    matches!(
        name,
        "iResolution" | "iTime" | "iGlobalTime" | "iProgress" | "iFrame" | "iMouse"
    )
}

/// `PM.parseUniforms(code)`.
pub fn parse_uniforms(code: &str) -> Vec<UniformDef> {
    let mut out = Vec::new();
    for m in pattern().captures_iter(code) {
        let ty = UniformType::parse(&m[1]);
        let name = m[2].to_owned();
        if reserved(&name) {
            continue;
        }
        let annotation = m.get(3).map_or("", |a| a.as_str());
        let a: Vec<&str> = annotation
            .trim_matches(is_js_space)
            .split(is_js_space)
            .filter(|s| !s.is_empty())
            .collect();
        let control = if ty == UniformType::Vec3 && a.first().is_some_and(|s| s.starts_with('#')) {
            Control::Color {
                default: a[0].to_owned(),
            }
        } else if ty == UniformType::Bool {
            Control::Toggle {
                default: a.first() == Some(&"true"),
            }
        } else {
            let mut default = a.first().map_or(0.0, |s| parse_float(s));
            let min = a
                .get(1)
                .map_or(if default < 0.0 { default * 2.0 } else { 0.0 }, |s| {
                    parse_float(s)
                });
            // `Math.max(1, Math.abs(def) * 2)`: NaN when the default is.
            let max = a.get(2).map_or_else(
                || {
                    let twice = default.abs() * 2.0;
                    if twice.is_nan() {
                        f64::NAN
                    } else {
                        twice.max(1.0)
                    }
                },
                |s| parse_float(s),
            );
            if default.is_nan() {
                default = 0.0;
            }
            Control::Number { default, min, max }
        };
        out.push(UniformDef {
            label: label_of(&name),
            name,
            ty,
            control,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_float_reads_the_longest_decimal_prefix() {
        assert_eq!(parse_float("0.35"), 0.35);
        assert_eq!(parse_float("  -2.5e1xyz"), -25.0);
        assert_eq!(parse_float(".5"), 0.5);
        assert_eq!(parse_float("5."), 5.0);
        assert_eq!(parse_float("1e"), 1.0);
        assert_eq!(parse_float("1e+"), 1.0);
        assert_eq!(parse_float("+Infinityx"), f64::INFINITY);
        assert_eq!(parse_float("-Infinity"), f64::NEG_INFINITY);
        assert!(parse_float("#FF6B1A").is_nan());
        assert!(parse_float("").is_nan());
        assert!(parse_float(".").is_nan());
        assert!(parse_float("-").is_nan());
        assert!(parse_float("0x10") == 0.0);
        assert_eq!(parse_float("\u{FEFF}3"), 3.0);
    }

    #[test]
    fn the_template_annotations() {
        let code = "uniform float uSpeed;   // @param 0.35 0 3\nuniform float uScale;   // @param 3.0 0.5 12\nuniform vec3  uTint;    // @param #FF6B1A\n\nvoid main() {}";
        let defs = parse_uniforms(code);
        assert_eq!(defs.len(), 3);
        assert_eq!(defs[0].name, "uSpeed");
        assert_eq!(defs[0].label, "Speed");
        assert_eq!(
            defs[0].control,
            Control::Number {
                default: 0.35,
                min: 0.0,
                max: 3.0
            }
        );
        assert_eq!(
            defs[2].control,
            Control::Color {
                default: "#FF6B1A".into()
            }
        );
    }

    #[test]
    fn binding_quirks_follow_the_regex() {
        // An annotation on the next line belongs to the uniform (`\s*`
        // crosses lines); a missing one leaves the defaults.
        let defs = parse_uniforms(
            "uniform float uA;\n// @param 2 1 4\nuniform vec2 uOffset; // @param 0.5\nuniform int uCount;\nuniform bool uOn; // @param true\nuniform vec3 uDir; // @param 1 0 0\nuniform float iTime;\nuniform float uNeg; // @param -3\nuniform float uBad; // @param abc",
        );
        let by = |n: &str| defs.iter().find(|d| d.name == n).unwrap().control.clone();
        assert_eq!(
            by("uA"),
            Control::Number {
                default: 2.0,
                min: 1.0,
                max: 4.0
            }
        );
        assert_eq!(
            by("uOffset"),
            Control::Number {
                default: 0.5,
                min: 0.0,
                max: 1.0
            }
        );
        assert_eq!(
            by("uCount"),
            Control::Number {
                default: 0.0,
                min: 0.0,
                max: 1.0
            }
        );
        assert_eq!(by("uOn"), Control::Toggle { default: true });
        assert!(matches!(by("uDir"), Control::Number { default, .. } if default == 1.0));
        assert!(!defs.iter().any(|d| d.name == "iTime"));
        assert_eq!(
            by("uNeg"),
            Control::Number {
                default: -3.0,
                min: -6.0,
                max: 6.0
            }
        );
        match by("uBad") {
            Control::Number { default, min, max } => {
                assert_eq!(default, 0.0);
                assert_eq!(min, 0.0);
                assert!(max.is_nan());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn labels_split_camel_case_once_per_pair() {
        assert_eq!(label_of("uMyColor"), "My Color");
        assert_eq!(label_of("uaBC"), "a BC");
        assert_eq!(label_of("speed"), "speed");
        assert_eq!(label_of("uuX"), "u X");
    }

    #[test]
    fn no_word_boundary_before_uniform() {
        // `xuniform float y;` matches as today (the regex has no `\b`).
        let defs = parse_uniforms("// xuniform float uY; // @param 7\n");
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "uY");
    }
}
