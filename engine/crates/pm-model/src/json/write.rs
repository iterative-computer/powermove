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
pub trait JsonTree: Sized {
    fn view(&self) -> View<'_, Self>;
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
    write_tree(value, &mut buffer, &mut Whole);
    buffer.flush();
}

/// The canonical serialization of `value` with the members `split` picks
/// (array elements and object values, asked before each is written; never
/// inside a picked one) left out: the text around them, one piece more than
/// members picked, and those members, in order. Writing each member's
/// canonical serialization between the pieces gives [`write_canonical`]'s.
pub(crate) fn write_canonical_split<'a, T: JsonTree>(
    value: &'a T,
    split: impl FnMut(&'a T) -> bool,
) -> (Vec<String>, Vec<&'a T>) {
    let mut pieces = Pieces::default();
    let mut cut = Split {
        members: Vec::new(),
        split,
    };
    let mut buffer = Buffered {
        sink: &mut pieces,
        buf: String::with_capacity(8192),
    };
    write_tree(value, &mut buffer, &mut cut);
    buffer.flush();
    pieces.texts.push(pieces.current);
    (pieces.texts, cut.members)
}

/// Decides, before a member is written, whether it is written elsewhere.
trait Cut<'a, T, S: Sink> {
    fn cut(&mut self, member: &'a T, out: &mut Buffered<'_, S>) -> bool;
}

/// Every member written in place.
struct Whole;

impl<T, S: Sink> Cut<'_, T, S> for Whole {
    #[inline(always)]
    fn cut(&mut self, _: &T, _: &mut Buffered<'_, S>) -> bool {
        false
    }
}

#[derive(Default)]
struct Pieces {
    texts: Vec<String>,
    current: String,
}

impl Sink for Pieces {
    fn push_str(&mut self, s: &str) {
        self.current.push_str(s);
    }
}

struct Split<'a, T, F> {
    members: Vec<&'a T>,
    split: F,
}

impl<'a, T, F: FnMut(&'a T) -> bool> Cut<'a, T, Pieces> for Split<'a, T, F> {
    fn cut(&mut self, member: &'a T, out: &mut Buffered<'_, Pieces>) -> bool {
        if !(self.split)(member) {
            return false;
        }
        out.flush();
        let text = core::mem::take(&mut out.sink.current);
        out.sink.texts.push(text);
        self.members.push(member);
        true
    }
}

struct Buffered<'a, S: Sink> {
    sink: &'a mut S,
    buf: String,
}

impl<S: Sink> Buffered<'_, S> {
    fn push(&mut self, mut s: &str) {
        while !s.is_empty() {
            let available = 8192usize.saturating_sub(self.buf.len());
            let mut end = available.min(s.len());
            while !s.is_char_boundary(end) {
                end -= 1;
            }
            if end == 0 {
                self.flush();
                continue;
            }
            self.buf.push_str(&s[..end]);
            s = &s[end..];
            if self.buf.len() >= 8192 {
                self.flush();
            }
        }
    }

    fn push_char(&mut self, c: char) {
        self.buf.push(c);
        if self.buf.len() >= 8192 {
            self.flush();
        }
    }

    fn unicode_escape(&mut self, unit: u32) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        self.push("\\u");
        for shift in [12, 8, 4, 0] {
            self.push_char(HEX[((unit >> shift) & 0xf) as usize] as char);
        }
    }

    fn escaped_char(&mut self, c: char) {
        match c {
            '"' => self.push("\\\""),
            '\\' => self.push("\\\\"),
            '\u{8}' => self.push("\\b"),
            '\t' => self.push("\\t"),
            '\n' => self.push("\\n"),
            '\u{c}' => self.push("\\f"),
            '\r' => self.push("\\r"),
            c if (c as u32) < 0x20 => self.unicode_escape(c as u32),
            c => self.push_char(c),
        }
    }

    fn quoted(&mut self, text: &JsString) {
        self.push_char('"');
        if let Some(text) = text.as_str() {
            let mut start = 0;
            for (i, &b) in text.as_bytes().iter().enumerate() {
                if b < 0x20 || b == b'"' || b == b'\\' {
                    self.push(&text[start..i]);
                    self.escaped_char(b as char);
                    start = i + 1;
                }
            }
            self.push(&text[start..]);
        } else {
            for cp in text.code_points() {
                match char::from_u32(cp) {
                    Some(c) => self.escaped_char(c),
                    None => self.unicode_escape(cp),
                }
            }
        }
        self.push_char('"');
    }

    fn flush(&mut self) {
        if !self.buf.is_empty() {
            self.sink.push_str(&self.buf);
            self.buf.clear();
        }
    }
}

fn write_tree<'a, T: JsonTree, S: Sink, C: Cut<'a, T, S>>(
    value: &'a T,
    out: &mut Buffered<'_, S>,
    cut: &mut C,
) {
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
        View::String(s) => out.quoted(s),
        View::Array(items) => {
            out.push_char('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_char(',');
                }
                if !cut.cut(item, out) {
                    write_tree(item, out, cut);
                }
            }
            out.push_char(']');
        }
        View::Object(map) => {
            out.push_char('{');
            let mut first = true;
            let mut entry = |key: &JsString, child: &'a T, out: &mut Buffered<'_, S>| {
                if !first {
                    out.push_char(',');
                }
                first = false;
                out.quoted(key);
                out.push_char(':');
                if !cut.cut(child, out) {
                    write_tree(child, out, cut);
                }
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

    #[derive(Default)]
    struct Chunked {
        text: String,
        peak: usize,
    }
    impl Sink for Chunked {
        fn push_str(&mut self, text: &str) {
            self.peak = self.peak.max(text.len());
            self.text.push_str(text);
        }
    }

    #[test]
    fn streams_giant_utf8_values_and_keys_in_bounded_chunks() {
        let text = JsString::from(format!(
            "{}{}\"\\\0\u{2028}",
            "x".repeat(8191),
            "😀".repeat(512 * 1024)
        ));
        let mut expected = String::new();
        push_quoted(&text, &mut expected);
        let mut sink = Chunked::default();
        write_canonical(&Value::String(text.clone()), &mut sink);
        assert_eq!(sink.text, expected);
        assert!(
            sink.peak <= 8192 + 32,
            "large string scratch escaped the chunk bound: {}",
            sink.peak
        );
        let mut map = super::super::Map::new();
        map.insert(text, Value::Number(1.0));
        let mut sink = Chunked::default();
        write_canonical(&Value::Object(map), &mut sink);
        assert_eq!(sink.text, format!("{{{expected}:1}}"));
        assert!(sink.peak <= 8192 + 32);
    }

    #[test]
    fn streamed_wtf8_quoting_preserves_every_utf16_unit() {
        let text = JsString::from_utf16_units(&(0..=u16::MAX).collect::<Vec<_>>());
        let mut expected = String::new();
        push_quoted(&text, &mut expected);
        let mut sink = Chunked::default();
        write_canonical(&Value::String(text), &mut sink);
        assert_eq!(sink.text, expected);
        assert!(sink.peak <= 8192 + 32);
    }

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
