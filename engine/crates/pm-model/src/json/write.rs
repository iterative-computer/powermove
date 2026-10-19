//! Canonical JSON serialization.
//!
//! # The canonical form (normative; mirrored by `apps/desktop/src/shared/canonical-json.ts`)
//!
//! The canonical serialization of a value is the output of `JSON.stringify(value)`
//! with no indentation, except that **every object's keys are written in
//! ascending order of their UTF-16 code units** (the order of JavaScript's
//! default `Array.prototype.sort()` on the key strings). Precisely:
//!
//! * `null`, `true`, `false` as literals.
//! * Numbers as `Number::toString` (ECMA-262 §6.1.6.1.20): shortest round-trip
//!   digits; `-0` is `0`; exponent notation `1e+21` / `1.5e-7` outside
//!   `[1e-7, 1e21)`; `NaN` and `±Infinity` are `null`.
//! * Strings as `QuoteJSONString` (ECMA-262 §25.5.2.3): `"` and `\` escaped as
//!   `\"` and `\\`; U+0008, U+0009, U+000A, U+000C, U+000D as `\b \t \n \f \r`;
//!   other code units below U+0020 as `\u00xx` with lowercase hex; unpaired
//!   surrogates as `\udxxx` with lowercase hex; every other code point written
//!   literally (including `/`, U+007F, U+2028, U+2029).
//! * Arrays as `[` elements joined by `,` `]`; objects as `{` `"key":value`
//!   pairs joined by `,` `}`; no whitespace anywhere.
//! * Text is encoded as UTF-8 (well-formed, because surrogates are escaped).
//!
//! The canonical hash ([`crate::hash`]) is the SHA-256 of those UTF-8 bytes,
//! written as 64 lowercase hex digits.

use super::jsnum::push_js_number;
use super::string::utf16_order_matches_bytes;
use super::{JsString, Value};

/// A read-only view over a JSON-shaped tree, so one writer serves [`Value`] and
/// the revisioned document tree.
pub trait JsonTree {
    type Child: JsonTree;
    fn view(&self) -> View<'_, Self::Child>;
}

/// One node of a [`JsonTree`].
pub enum View<'a, C> {
    Null,
    Bool(bool),
    Number(f64),
    String(&'a JsString),
    Array(&'a [C]),
    Object(&'a super::ObjectMap<C>),
}

impl JsonTree for Value {
    type Child = Value;
    fn view(&self) -> View<'_, Value> {
        match self {
            Value::Null => View::Null,
            Value::Bool(b) => View::Bool(*b),
            Value::Number(n) => View::Number(*n),
            Value::String(s) => View::String(s),
            Value::Array(items) => View::Array(items),
            Value::Object(map) => View::Object(map.as_members()),
        }
    }
}

/// Receives serialized text.
pub trait Sink {
    fn push_str(&mut self, s: &str);
}

impl Sink for String {
    fn push_str(&mut self, s: &str) {
        String::push_str(self, s);
    }
}

/// The canonical serialization as a `String`.
pub fn to_canonical_string<T: JsonTree>(value: &T) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

/// Writes the canonical serialization into `sink`.
pub fn write_canonical<T: JsonTree, S: Sink>(value: &T, sink: &mut S) {
    let mut buffer = Buffered {
        sink,
        buf: String::with_capacity(8192),
    };
    write_tree(value, &mut buffer);
    buffer.flush();
}

struct Buffered<'a, S: Sink> {
    sink: &'a mut S,
    buf: String,
}

impl<S: Sink> Buffered<'_, S> {
    fn push(&mut self, s: &str) {
        self.buf.push_str(s);
        if self.buf.len() >= 8192 {
            self.flush();
        }
    }

    fn push_char(&mut self, c: char) {
        self.buf.push(c);
    }

    fn flush(&mut self) {
        if !self.buf.is_empty() {
            self.sink.push_str(&self.buf);
            self.buf.clear();
        }
    }
}

fn write_tree<T: JsonTree, S: Sink>(value: &T, out: &mut Buffered<'_, S>) {
    match value.view() {
        View::Null => out.push("null"),
        View::Bool(true) => out.push("true"),
        View::Bool(false) => out.push("false"),
        View::Number(n) => {
            if n.is_finite() {
                push_js_number(n, &mut out.buf);
            } else {
                out.push("null");
            }
        }
        View::String(s) => push_quoted(s, &mut out.buf),
        View::Array(items) => {
            out.push_char('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_char(',');
                }
                write_tree(item, out);
            }
            out.push_char(']');
        }
        View::Object(map) => {
            out.push_char('{');
            let mut first = true;
            let mut entry = |key: &JsString, child: &T::Child, out: &mut Buffered<'_, S>| {
                if !first {
                    out.push_char(',');
                }
                first = false;
                push_quoted(key, &mut out.buf);
                out.push_char(':');
                write_tree(child, out);
            };
            if map.keys().all(|k| utf16_order_matches_bytes(k.as_bytes())) {
                for (key, child) in map {
                    entry(key, child, out);
                }
            } else {
                let mut entries: Vec<_> = map.iter().collect();
                entries.sort_by(|a, b| a.0.cmp_utf16(b.0));
                for (key, child) in entries {
                    entry(key, child, out);
                }
            }
            out.push_char('}');
        }
    }
    if out.buf.len() >= 8192 {
        out.flush();
    }
}

/// Appends `QuoteJSONString(s)`.
pub fn push_quoted(s: &JsString, out: &mut String) {
    out.push('"');
    match s.as_str() {
        Some(text) => push_escaped_str(text, out),
        None => {
            for cp in s.code_points() {
                match char::from_u32(cp) {
                    Some(c) => push_escaped_char(c, out),
                    None => push_unicode_escape(cp, out), // unpaired surrogate
                }
            }
        }
    }
    out.push('"');
}

fn push_escaped_str(text: &str, out: &mut String) {
    let bytes = text.as_bytes();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b < 0x20 || b == b'"' || b == b'\\' {
            out.push_str(&text[start..i]);
            push_escaped_char(b as char, out);
            start = i + 1;
        }
    }
    out.push_str(&text[start..]);
}

fn push_escaped_char(c: char, out: &mut String) {
    match c {
        '"' => out.push_str("\\\""),
        '\\' => out.push_str("\\\\"),
        '\u{8}' => out.push_str("\\b"),
        '\t' => out.push_str("\\t"),
        '\n' => out.push_str("\\n"),
        '\u{c}' => out.push_str("\\f"),
        '\r' => out.push_str("\\r"),
        c if (c as u32) < 0x20 => push_unicode_escape(c as u32, out),
        c => out.push(c),
    }
}

fn push_unicode_escape(unit: u32, out: &mut String) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push_str("\\u");
    for shift in [12, 8, 4, 0] {
        out.push(HEX[((unit >> shift) & 0xf) as usize] as char);
    }
}

#[cfg(test)]
mod tests {
    use super::super::parse;
    use super::*;

    #[test]
    fn writes_json_stringify_bytes_with_sorted_keys() {
        let v = parse(r#"{"b":1,"a":[true,null,-0,1e21,0.1,"x"],"10":2,"2":3}"#).unwrap();
        assert_eq!(
            to_canonical_string(&v),
            r#"{"10":2,"2":3,"a":[true,null,0,1e+21,0.1,"x"],"b":1}"#
        );
    }

    #[test]
    fn escapes_like_quote_json_string() {
        let v = parse(r#""q\" b\\ \b\f\n\r\t \u0000\u001f\u007f /   \ud800 \udc00x 😀""#).unwrap();
        assert_eq!(
            to_canonical_string(&v),
            "\"q\\\" b\\\\ \\b\\f\\n\\r\\t \\u0000\\u001f\u{7f} / \u{2028} \\ud800 \\udc00x 😀\""
        );
    }

    #[test]
    fn non_finite_numbers_are_null() {
        let v = Value::Array(vec![Value::Number(f64::NAN), Value::Number(f64::INFINITY)]);
        assert_eq!(to_canonical_string(&v), "[null,null]");
    }

    #[test]
    fn sorts_keys_by_utf16_units() {
        let mut map = super::super::Map::new();
        map.insert("\u{ffff}", Value::Number(1.0));
        map.insert("\u{10000}", Value::Number(2.0));
        map.insert("a", Value::Number(3.0));
        assert_eq!(
            to_canonical_string(&Value::Object(map)),
            "{\"a\":3,\"\u{10000}\":2,\"\u{ffff}\":1}"
        );
    }
}
