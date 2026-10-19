//! `JSON.parse` semantics.

use core::fmt;

use super::string::Wtf8Builder;
use super::{JsString, Map, Value};

/// Maximum nesting depth. Project documents nest about 15 levels; the limit
/// keeps recursion (here, in serialization, and in drop) bounded on every
/// target, including WebAssembly's 1 MiB stack.
pub const MAX_DEPTH: usize = 512;

/// A JSON syntax error with its byte offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub offset: usize,
    pub message: &'static str,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for ParseError {}

/// What the parser builds: a [`Value`] tree, or another tree the same
/// parse can produce directly (the revisioned document, without the
/// intermediate `Value` a large document would need memory for twice).
pub trait TreeBuilder {
    type Out;
    fn null(&mut self) -> Self::Out;
    fn bool(&mut self, b: bool) -> Self::Out;
    fn number(&mut self, n: f64) -> Self::Out;
    fn string(&mut self, s: JsString) -> Self::Out;
    fn array(&mut self, items: Vec<Self::Out>) -> Self::Out;
    /// Members in source order; the last duplicate key wins.
    fn object(&mut self, members: Vec<(JsString, Self::Out)>) -> Self::Out;
}

struct ValueBuilder;

impl TreeBuilder for ValueBuilder {
    type Out = Value;
    fn null(&mut self) -> Value {
        Value::Null
    }
    fn bool(&mut self, b: bool) -> Value {
        Value::Bool(b)
    }
    fn number(&mut self, n: f64) -> Value {
        Value::Number(n)
    }
    fn string(&mut self, s: JsString) -> Value {
        Value::String(s)
    }
    fn array(&mut self, items: Vec<Value>) -> Value {
        Value::Array(items)
    }
    fn object(&mut self, members: Vec<(JsString, Value)>) -> Value {
        Value::Object(members.into_iter().collect::<Map>())
    }
}

/// Parses JSON text exactly as `JSON.parse` does for a JavaScript string with
/// the same code points: RFC 8259 grammar, the last duplicate key wins, numbers
/// are correctly rounded `f64`s (overflow becomes ±Infinity), and `\uXXXX`
/// escapes may produce lone surrogates.
pub fn parse(text: &str) -> Result<Value, ParseError> {
    parse_with(text, &mut ValueBuilder)
}

/// [`parse`] into the tree `builder` makes.
pub fn parse_with<B: TreeBuilder>(text: &str, builder: &mut B) -> Result<B::Out, ParseError> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        pos: 0,
        depth: 0,
    };
    parser.skip_whitespace();
    let value = parser.value(builder)?;
    parser.skip_whitespace();
    if parser.pos != parser.bytes.len() {
        return Err(parser.error("unexpected data after JSON value"));
    }
    Ok(value)
}

/// [`parse_bytes`] into the tree `builder` makes.
pub fn parse_bytes_with<B: TreeBuilder>(
    bytes: &[u8],
    builder: &mut B,
) -> Result<B::Out, ParseError> {
    let text = core::str::from_utf8(bytes).map_err(|e| ParseError {
        offset: e.valid_up_to(),
        message: "invalid UTF-8",
    })?;
    parse_with(text, builder)
}

/// Parses UTF-8 bytes; invalid sequences fail (use [`parse_file_text`] for
/// `TextDecoder` behaviour).
pub fn parse_bytes(bytes: &[u8]) -> Result<Value, ParseError> {
    let text = core::str::from_utf8(bytes).map_err(|e| ParseError {
        offset: e.valid_up_to(),
        message: "invalid UTF-8",
    })?;
    parse(text)
}

/// Decodes bytes the way the app reads files (`new TextDecoder().decode`: a
/// leading UTF-8 byte order mark is dropped and invalid sequences become
/// U+FFFD), then parses.
pub fn parse_file_text(bytes: &[u8]) -> Result<Value, ParseError> {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let text = String::from_utf8_lossy(bytes);
    parse(&text)
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn error(&self, message: &'static str) -> ParseError {
        ParseError {
            offset: self.pos,
            message,
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(&b) = self.bytes.get(self.pos) {
            if matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn expect_literal(&mut self, literal: &'static [u8]) -> Result<(), ParseError> {
        if self.bytes[self.pos..].starts_with(literal) {
            self.pos += literal.len();
            Ok(())
        } else {
            Err(self.error("unexpected token"))
        }
    }

    fn value<B: TreeBuilder>(&mut self, b: &mut B) -> Result<B::Out, ParseError> {
        match self.peek() {
            None => Err(self.error("unexpected end of JSON input")),
            Some(b'{') => self.object(b),
            Some(b'[') => self.array(b),
            Some(b'"') => {
                let s = self.string()?;
                Ok(b.string(s))
            }
            Some(b't') => self.expect_literal(b"true").map(|()| b.bool(true)),
            Some(b'f') => self.expect_literal(b"false").map(|()| b.bool(false)),
            Some(b'n') => self.expect_literal(b"null").map(|()| b.null()),
            Some(b'-' | b'0'..=b'9') => {
                let n = self.number()?;
                Ok(b.number(n))
            }
            Some(_) => Err(self.error("unexpected token")),
        }
    }

    fn enter(&mut self) -> Result<(), ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.error("JSON nests too deeply"));
        }
        Ok(())
    }

    fn object<B: TreeBuilder>(&mut self, b: &mut B) -> Result<B::Out, ParseError> {
        self.enter()?;
        self.pos += 1; // {
        // Members are collected, then sorted once (already sorted in
        // canonical JSON) into a map holding exactly them.
        let mut members: Vec<(JsString, B::Out)> = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(b.object(members));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(self.error("expected a property name"));
            }
            let key = self.string()?;
            self.skip_whitespace();
            if self.peek() != Some(b':') {
                return Err(self.error("expected ':' after a property name"));
            }
            self.pos += 1;
            self.skip_whitespace();
            let value = self.value(b)?;
            members.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
        self.depth -= 1;
        Ok(b.object(members))
    }

    fn array<B: TreeBuilder>(&mut self, b: &mut B) -> Result<B::Out, ParseError> {
        self.enter()?;
        self.pos += 1; // [
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(b.array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value(b)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
        self.depth -= 1;
        items.shrink_to_fit();
        Ok(b.array(items))
    }

    fn hex4(&mut self) -> Result<u16, ParseError> {
        let digits = self
            .bytes
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| self.error("bad Unicode escape"))?;
        let mut value = 0u16;
        for &d in digits {
            let v = (d as char)
                .to_digit(16)
                .ok_or_else(|| self.error("bad Unicode escape"))?;
            value = value * 16 + v as u16;
        }
        self.pos += 4;
        Ok(value)
    }

    fn string(&mut self) -> Result<JsString, ParseError> {
        self.pos += 1; // opening quote
        let start = self.pos;
        // Fast path: no escapes.
        while let Some(&b) = self.bytes.get(self.pos) {
            match b {
                b'"' => {
                    let text = core::str::from_utf8(&self.bytes[start..self.pos])
                        .map_err(|_| self.error("invalid UTF-8"))?;
                    self.pos += 1;
                    return Ok(JsString::from(text));
                }
                b'\\' => break,
                0x00..=0x1f => return Err(self.error("bad control character in string literal")),
                _ => self.pos += 1,
            }
        }
        let mut builder = Wtf8Builder::with_capacity(self.pos - start + 16);
        builder.push_str(
            core::str::from_utf8(&self.bytes[start..self.pos])
                .map_err(|_| self.error("invalid UTF-8"))?,
        );
        loop {
            let Some(&b) = self.bytes.get(self.pos) else {
                return Err(self.error("unterminated string"));
            };
            match b {
                b'"' => {
                    self.pos += 1;
                    return Ok(builder.finish());
                }
                b'\\' => {
                    self.pos += 1;
                    let Some(&escape) = self.bytes.get(self.pos) else {
                        return Err(self.error("unterminated string"));
                    };
                    self.pos += 1;
                    match escape {
                        b'"' => builder.push_char('"'),
                        b'\\' => builder.push_char('\\'),
                        b'/' => builder.push_char('/'),
                        b'b' => builder.push_char('\u{8}'),
                        b'f' => builder.push_char('\u{c}'),
                        b'n' => builder.push_char('\n'),
                        b'r' => builder.push_char('\r'),
                        b't' => builder.push_char('\t'),
                        b'u' => {
                            let unit = self.hex4()?;
                            builder.push_unit(unit);
                        }
                        _ => {
                            self.pos -= 1;
                            return Err(self.error("bad escaped character"));
                        }
                    }
                }
                0x00..=0x1f => return Err(self.error("bad control character in string literal")),
                _ => {
                    // Copy one UTF-8 scalar (the input is valid UTF-8).
                    let len = utf8_len(b);
                    let chunk = self
                        .bytes
                        .get(self.pos..self.pos + len)
                        .and_then(|c| core::str::from_utf8(c).ok())
                        .ok_or_else(|| self.error("invalid UTF-8"))?;
                    builder.push_str(chunk);
                    self.pos += len;
                }
            }
        }
    }

    fn number(&mut self) -> Result<f64, ParseError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return Err(self.error("no number after minus sign")),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error("unterminated fractional number"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error("exponent part is missing a number"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let text = core::str::from_utf8(&self.bytes[start..self.pos]).expect("ASCII");
        // Correctly rounded; out-of-range magnitudes become ±inf or ±0 as in JS.
        let n: f64 = text.parse().map_err(|_| self.error("bad number"))?;
        Ok(n)
    }
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_javascript_values() {
        let v = parse(
            r#" {"a": [1, -0, 2.5e3, true, false, null], "b": "x\u00e9\ud83d\ude00", "a": 7} "#,
        )
        .unwrap();
        assert_eq!(v.get("a"), Some(&Value::Number(7.0))); // last duplicate wins
        assert_eq!(v.get("b").unwrap().as_str(), Some("xé😀"));
        let arr = parse("[-0, 1e400, -1e400, 1e-400]").unwrap();
        let arr = arr.as_array().unwrap();
        assert!(arr[0].as_f64().unwrap().is_sign_negative());
        assert_eq!(arr[1], Value::Number(f64::INFINITY));
        assert_eq!(arr[2], Value::Number(f64::NEG_INFINITY));
        assert_eq!(arr[3], Value::Number(0.0));
    }

    #[test]
    fn keeps_lone_surrogates() {
        let v = parse(r#""a\ud800b""#).unwrap();
        let s = v.as_js_string().unwrap();
        assert!(s.has_lone_surrogates());
        assert_eq!(
            s.utf16_units().collect::<Vec<_>>(),
            vec![0x61, 0xd800, 0x62]
        );
    }

    #[test]
    fn rejects_what_json_parse_rejects() {
        for text in [
            "",
            " ",
            "{",
            "[1,]",
            "{\"a\":1,}",
            "01",
            "1.",
            ".5",
            "+1",
            "-",
            "1e",
            "NaN",
            "Infinity",
            "'a'",
            "\"\t\"",
            "\"\\x\"",
            "\"\\u12\"",
            "[1 2]",
            "{\"a\" 1}",
            "{a:1}",
            "tru",
            "nul",
            "1 2",
            "\u{feff}1",
            "[\"\u{1}\"]",
        ] {
            assert!(parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn file_text_drops_bom_and_replaces_invalid_utf8() {
        let v = parse_file_text(b"\xef\xbb\xbf\"a\xffb\"").unwrap();
        assert_eq!(v.as_str(), Some("a\u{fffd}b"));
        assert!(parse_bytes(b"\"a\xffb\"").is_err());
    }

    #[test]
    fn bounds_nesting() {
        let deep = "[".repeat(MAX_DEPTH) + &"]".repeat(MAX_DEPTH);
        assert!(parse(&deep).is_ok());
        let deeper = "[".repeat(MAX_DEPTH + 1) + &"]".repeat(MAX_DEPTH + 1);
        assert_eq!(parse(&deeper).unwrap_err().message, "JSON nests too deeply");
    }
}
