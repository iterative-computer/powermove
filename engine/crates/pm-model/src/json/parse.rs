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

/// Parses JSON text exactly as `JSON.parse` does for a JavaScript string with
/// the same code points: RFC 8259 grammar, the last duplicate key wins, numbers
/// are correctly rounded `f64`s (overflow becomes ±Infinity), and `\uXXXX`
/// escapes may produce lone surrogates.
pub fn parse(text: &str) -> Result<Value, ParseError> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        pos: 0,
        depth: 0,
    };
    parser.skip_whitespace();
    let value = parser.value()?;
    parser.skip_whitespace();
    if parser.pos != parser.bytes.len() {
        return Err(parser.error("unexpected data after JSON value"));
    }
    Ok(value)
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

    fn expect_literal(
        &mut self,
        literal: &'static [u8],
        value: Value,
    ) -> Result<Value, ParseError> {
        if self.bytes[self.pos..].starts_with(literal) {
            self.pos += literal.len();
            Ok(value)
        } else {
            Err(self.error("unexpected token"))
        }
    }

    fn value(&mut self) -> Result<Value, ParseError> {
        match self.peek() {
            None => Err(self.error("unexpected end of JSON input")),
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b't') => self.expect_literal(b"true", Value::Bool(true)),
            Some(b'f') => self.expect_literal(b"false", Value::Bool(false)),
            Some(b'n') => self.expect_literal(b"null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
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

    fn object(&mut self) -> Result<Value, ParseError> {
        self.enter()?;
        self.pos += 1; // {
        let mut map = Map::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(Value::Object(map));
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
            let value = self.value()?;
            map.insert(key, value);
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
        Ok(Value::Object(map))
    }

    fn array(&mut self) -> Result<Value, ParseError> {
        self.enter()?;
        self.pos += 1; // [
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value()?);
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
        Ok(Value::Array(items))
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

    fn number(&mut self) -> Result<Value, ParseError> {
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
        Ok(Value::Number(n))
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
