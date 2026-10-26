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
    /// The object of `members[start..]`, in source order (the last duplicate
    /// key wins), which it moves out; earlier members stay.
    fn object(&mut self, members: &mut Vec<(JsString, Self::Out)>, start: usize) -> Self::Out;
    /// The value at byte `pos` when it was built ahead of this parse, with the
    /// offset just past its text (the last pass of [`parse_bytes_parallel`]).
    #[inline(always)]
    fn prebuilt(&mut self, _pos: usize) -> Option<(Self::Out, usize)> {
        None
    }
}

#[derive(Clone)]
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
    fn object(&mut self, members: &mut Vec<(JsString, Value)>, start: usize) -> Value {
        Value::Object(Map::from_members(members, start))
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
    let mut parser = Parser::new(text.as_bytes(), 0, 0);
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

/// Inputs smaller than this parse on one thread.
const PARALLEL_MIN_BYTES: usize = 8 << 20;
/// The containers a parallel parse hands to its workers: the outermost whose
/// text is at most `UNIT_MAX_BYTES` long, if it is at least `UNIT_MIN_BYTES`.
const UNIT_MAX_BYTES: usize = 1 << 20;
const UNIT_MIN_BYTES: usize = 16 << 10;

/// A container a worker parses: its text is `start..end`, inside `depth`
/// enclosing containers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unit {
    pub start: usize,
    pub end: usize,
    pub depth: usize,
}

/// The units of a parallel parse, in source order, found by one pass over
/// the text that follows only strings and brackets. `None` when that pass
/// finds the text malformed (the sequential parse reports how).
fn units(bytes: &[u8]) -> Option<Vec<Unit>> {
    let mut scanner = UnitScanner::default();
    let mut units = scanner.feed(bytes);
    units.extend(scanner.finish()?);
    Some(units)
}

/// The units of a parallel parse found as the text arrives: bytes are fed in
/// order, in pieces of any size, and a unit is handed out once no container
/// still open could hold it. Together with [`UnitScanner::finish`], exactly
/// the units of the whole text, in source order.
#[derive(Default)]
pub struct UnitScanner {
    /// Bytes fed so far.
    pos: usize,
    /// Where the containers still open start, outermost first.
    open: Vec<usize>,
    /// Units a container still open may yet hold, in source order.
    pending: Vec<Unit>,
    string: bool,
    escape: bool,
    broken: bool,
}

impl UnitScanner {
    /// Scans the next bytes; the units that became final, in source order.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Unit> {
        if self.broken {
            return Vec::new();
        }
        let base = self.pos;
        let mut i = 0;
        let mut closed = false;
        while i < bytes.len() {
            if self.string {
                if self.escape {
                    self.escape = false;
                } else {
                    match bytes[i] {
                        b'"' => self.string = false,
                        b'\\' => self.escape = true,
                        _ => {}
                    }
                }
                i += 1;
                continue;
            }
            match bytes[i] {
                b'"' => self.string = true,
                b'{' | b'[' => {
                    if self.open.len() >= MAX_DEPTH {
                        self.broken = true;
                        return Vec::new();
                    }
                    self.open.push(base + i);
                }
                b'}' | b']' => {
                    let Some(start) = self.open.pop() else {
                        self.broken = true;
                        return Vec::new();
                    };
                    let end = base + i + 1;
                    if end - start <= UNIT_MAX_BYTES {
                        // It contains the units found since it opened.
                        while self.pending.last().is_some_and(|unit| unit.start > start) {
                            self.pending.pop();
                        }
                        if end - start >= UNIT_MIN_BYTES {
                            self.pending.push(Unit {
                                start,
                                end,
                                depth: self.open.len(),
                            });
                        }
                    }
                    closed = true;
                }
                _ => {}
            }
            i += 1;
        }
        self.pos = base + bytes.len();
        if !closed && self.pending.is_empty() {
            return Vec::new();
        }
        // A unit is final once the innermost open container around it is
        // already longer than a unit may be (or none is open around it).
        let mut done = 0;
        while let Some(unit) = self.pending.get(done) {
            let around = self.open.partition_point(|&start| start < unit.start);
            if around > 0 && self.pos - self.open[around - 1] <= UNIT_MAX_BYTES {
                break;
            }
            done += 1;
        }
        self.pending.drain(..done).collect()
    }

    /// The first byte a unit not yet handed out can start at: the text
    /// before it is no longer needed for units.
    pub fn needed_from(&self) -> usize {
        let open = self
            .open
            .iter()
            .copied()
            .find(|&start| self.pos - start <= UNIT_MAX_BYTES)
            .unwrap_or(self.pos);
        self.pending
            .first()
            .map_or(open, |unit| unit.start.min(open))
    }

    /// After the last byte: the units still held, or `None` when the text is
    /// malformed (the sequential parse reports how).
    pub fn finish(self) -> Option<Vec<Unit>> {
        (!self.broken && !self.string && self.open.is_empty()).then_some(self.pending)
    }
}

/// A unit's value, parsed from its own text ahead of the parse of the whole
/// (as the text arrives), and the SHA-256 of that text. The whole parse uses
/// it only where the bytes there have the same digest.
pub struct Prebuilt<T> {
    pub unit: Unit,
    pub digest: [u8; 32],
    pub value: T,
}

/// Parses the text of one unit (`text`, inside `depth` enclosing containers)
/// as [`parse_bytes_parallel`]'s workers do; `None` when it is not exactly one
/// well-formed UTF-8 JSON value.
pub fn parse_unit_with<B: TreeBuilder>(
    text: &[u8],
    depth: usize,
    builder: &mut B,
) -> Option<B::Out> {
    core::str::from_utf8(text).ok()?;
    let mut parser = Parser::new(text, 0, depth);
    let value = parser.value(builder).ok()?;
    (parser.pos == text.len()).then_some(value)
}

/// The skeleton pass of a parallel parse: `inner` builds everything but the
/// units, which come from `parsed`.
struct Assemble<'u, B: TreeBuilder> {
    inner: &'u mut B,
    units: core::iter::Peekable<
        core::iter::Zip<core::slice::Iter<'u, Unit>, std::vec::IntoIter<B::Out>>,
    >,
}

impl<B: TreeBuilder> TreeBuilder for Assemble<'_, B> {
    type Out = B::Out;
    fn null(&mut self) -> B::Out {
        self.inner.null()
    }
    fn bool(&mut self, b: bool) -> B::Out {
        self.inner.bool(b)
    }
    fn number(&mut self, n: f64) -> B::Out {
        self.inner.number(n)
    }
    fn string(&mut self, s: JsString) -> B::Out {
        self.inner.string(s)
    }
    fn array(&mut self, items: Vec<B::Out>) -> B::Out {
        self.inner.array(items)
    }
    fn object(&mut self, members: &mut Vec<(JsString, B::Out)>, start: usize) -> B::Out {
        self.inner.object(members, start)
    }
    fn prebuilt(&mut self, pos: usize) -> Option<(B::Out, usize)> {
        let (unit, _) = self.units.peek()?;
        if unit.start != pos {
            return None;
        }
        let (unit, value) = self.units.next()?;
        Some((value, unit.end))
    }
}

/// [`parse_bytes_with`] on several threads ([`crate::parallel`]) for a large
/// input: the same tree, built by the same builder calls, and the same
/// errors. Containers of up to a mebibyte of text are parsed by workers, then
/// one pass parses the rest around them. Anything unexpected (malformed text)
/// parses again on one thread, which reports exactly what
/// [`parse_bytes_with`] would.
pub fn parse_bytes_parallel<B: TreeBuilder + Clone + Send + Sync>(
    bytes: &[u8],
    builder: &mut B,
) -> Result<B::Out, ParseError>
where
    B::Out: Send,
{
    parse_bytes_parallel_prebuilt(bytes, builder, Vec::new(), false)
}

/// [`parse_bytes_parallel`] with units already parsed as the text arrived
/// ([`UnitScanner`], [`parse_unit_with`]). With `complete`, `prebuilt` are
/// every unit of the text, so it is not scanned again. A unit whose text is
/// not exactly the bytes at its place is parsed again here; any unit set the
/// whole parse cannot place falls back to one thread, as malformed text does.
pub fn parse_bytes_parallel_prebuilt<B: TreeBuilder + Clone + Send + Sync>(
    bytes: &[u8],
    builder: &mut B,
    prebuilt: Vec<Prebuilt<B::Out>>,
    complete: bool,
) -> Result<B::Out, ParseError>
where
    B::Out: Send,
{
    let text = core::str::from_utf8(bytes).map_err(|e| ParseError {
        offset: e.valid_up_to(),
        message: "invalid UTF-8",
    })?;
    if prebuilt.is_empty() && (bytes.len() < PARALLEL_MIN_BYTES || crate::parallel::workers() < 2) {
        return parse_with(text, builder);
    }
    let units = if complete {
        Some(prebuilt.iter().map(|p| p.unit).collect::<Vec<_>>())
    } else {
        units(bytes)
    };
    let Some(units) = units.filter(|units| units.len() > 1) else {
        return parse_with(text, builder);
    };
    // The prebuilt value of each unit, where its text is the text there.
    let mut given: Vec<Option<B::Out>> = (0..units.len()).map(|_| None).collect();
    let mut digests: Vec<Option<[u8; 32]>> = vec![None; units.len()];
    let mut next = 0;
    for p in prebuilt {
        while next < units.len() && units[next].start < p.unit.start {
            next += 1;
        }
        if next < units.len() && units[next] == p.unit {
            given[next] = Some(p.value);
            digests[next] = Some(p.digest);
        }
    }
    let exact = crate::parallel::map(units.len(), |i| {
        let unit = &units[i];
        digests[i].is_some_and(|digest| {
            unit.end <= bytes.len() && digest == crate::hash::sha256(&bytes[unit.start..unit.end])
        })
    });
    let shared = &*builder;
    let parsed = crate::parallel::map(units.len(), |i| {
        if exact[i] {
            return Some(None);
        }
        let unit = &units[i];
        if unit.end > bytes.len() {
            return None;
        }
        let mut parser = Parser::new(bytes, unit.start, unit.depth);
        let value = parser.value(&mut shared.clone()).ok()?;
        (parser.pos == unit.end).then_some(Some(value))
    });
    let Some(parsed) = parsed.into_iter().collect::<Option<Vec<_>>>() else {
        return parse_with(text, builder);
    };
    let values: Vec<B::Out> = parsed
        .into_iter()
        .zip(given)
        .zip(exact)
        .map(|((parsed, given), exact)| {
            if exact {
                given.expect("exact units are given")
            } else {
                parsed.expect("parsed here")
            }
        })
        .collect();
    let mut assemble = Assemble {
        inner: builder,
        units: units.iter().zip(values).peekable(),
    };
    match parse_with(text, &mut assemble) {
        Ok(value) if assemble.units.peek().is_none() => Ok(value),
        _ => parse_with(text, builder),
    }
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

struct Parser<'a, O> {
    bytes: &'a [u8],
    pos: usize,
    depth: usize,
    /// The members and elements of the containers being parsed, innermost
    /// last. Each container's move out at its end into a vector of exactly
    /// their number (an object's, its values): one allocation, where growing
    /// one vector per container reallocated, and an object's pairs had a
    /// vector of their own besides its values'.
    members: Vec<(JsString, O)>,
    items: Vec<O>,
}

impl<'a, O> Parser<'a, O> {
    fn new(bytes: &'a [u8], pos: usize, depth: usize) -> Self {
        Parser {
            bytes,
            pos,
            depth,
            members: Vec::new(),
            items: Vec::new(),
        }
    }

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

    fn value<B: TreeBuilder<Out = O>>(&mut self, b: &mut B) -> Result<O, ParseError> {
        if let Some((value, end)) = b.prebuilt(self.pos) {
            self.pos = end;
            return Ok(value);
        }
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

    fn object<B: TreeBuilder<Out = O>>(&mut self, b: &mut B) -> Result<O, ParseError> {
        self.enter()?;
        self.pos += 1; // {
        // Members are collected, then sorted once (already sorted in
        // canonical JSON) into a map holding exactly them.
        let base = self.members.len();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(b.object(&mut self.members, base));
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
            self.members.push((key, value));
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
        Ok(b.object(&mut self.members, base))
    }

    fn array<B: TreeBuilder<Out = O>>(&mut self, b: &mut B) -> Result<O, ParseError> {
        self.enter()?;
        self.pos += 1; // [
        let base = self.items.len();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(b.array(Vec::new()));
        }
        loop {
            self.skip_whitespace();
            let item = self.value(b)?;
            self.items.push(item);
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
        let items = self.items.drain(base..).collect();
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

    /// About 12 MB of layers of keyframes: units on both sides of their
    /// bounds, duplicate keys, escapes, and text outside any unit.
    fn large_document() -> String {
        let layer = |i: usize| {
            let frames: Vec<String> = (0..[3, 400, 9000][i % 3])
                .map(|k| format!(r#"{{"t":{k},"v":[{k}.25,-{i}e-3,"a\"\\\u00e9\ud800"],"t":{k}}}"#))
                .collect();
            format!(
                r#"{{"id":"L{i}","keys":[{}],"id":"M{i}","x":{{"y":[[],{{}}]}}}}"#,
                frames.join(",")
            )
        };
        let layers: Vec<String> = (0..60).map(layer).collect();
        format!(
            r#" {{ "name" : "p\n", "comps":{{"c":{{"layers":[ {} ]}}}}, "tail": [1, 2, 3] }} "#,
            layers.join(" , ")
        )
    }

    #[test]
    fn parallel_parsing_builds_the_same_tree() {
        let text = large_document();
        assert!(text.len() > PARALLEL_MIN_BYTES);
        assert!(units(text.as_bytes()).is_some_and(|units| units.len() > 8));
        let parallel = parse_bytes_parallel(text.as_bytes(), &mut ValueBuilder).unwrap();
        assert!(parallel == parse(&text).unwrap());
        assert_eq!(
            parallel
                .get("comps")
                .unwrap()
                .get("c")
                .unwrap()
                .get("layers")
                .unwrap()
                .as_array()
                .unwrap()[4]
                .get("id")
                .unwrap()
                .as_str(),
            Some("M4")
        );
    }

    /// Feeds `text` to a scanner in pieces of the given sizes, keeping only
    /// the bytes it still needs, as a stream reader does: every unit, and the
    /// text each one had when it was handed out.
    fn scan_in_pieces(text: &[u8], sizes: &[usize]) -> Option<Vec<(Unit, Vec<u8>)>> {
        let mut scanner = UnitScanner::default();
        let (mut kept, mut base, mut at, mut out) = (Vec::new(), 0, 0, Vec::new());
        let mut take = |units: Vec<Unit>, kept: &Vec<u8>, base: usize| {
            for unit in units {
                assert!(unit.start >= base, "a unit's text was dropped");
                out.push((unit, kept[unit.start - base..unit.end - base].to_vec()));
            }
        };
        for &size in sizes.iter().cycle() {
            if at >= text.len() {
                break;
            }
            let piece = &text[at..(at + size).min(text.len())];
            at += piece.len();
            kept.extend_from_slice(piece);
            let units = scanner.feed(piece);
            take(units, &kept, base);
            let from = scanner.needed_from();
            kept.drain(..from - base);
            base = from;
        }
        let rest = scanner.finish()?;
        take(rest, &kept, base);
        Some(out)
    }

    #[test]
    fn units_found_as_the_text_arrives_are_the_units_of_the_whole_text() {
        let text = large_document();
        let whole = units(text.as_bytes()).unwrap();
        for sizes in [
            &[7usize, 1 << 12, 1, 3][..],
            &[1 << 20],
            &[text.len()],
            &[65_537, 13],
        ] {
            let scanned = scan_in_pieces(text.as_bytes(), sizes).unwrap();
            assert_eq!(
                scanned.iter().map(|(unit, _)| *unit).collect::<Vec<_>>(),
                whole,
                "{sizes:?}"
            );
            for (unit, kept) in &scanned {
                assert_eq!(kept.as_slice(), &text.as_bytes()[unit.start..unit.end]);
            }
        }
        // Malformed text: no units at the end.
        assert!(scan_in_pieces(&text.as_bytes()[..text.len() - 3], &[1 << 16]).is_none());
        assert!(scan_in_pieces(b"[1]]", &[1]).is_none());
    }

    #[test]
    fn prebuilt_units_build_the_same_tree_and_mismatched_ones_are_parsed_again() {
        let text = large_document();
        let bytes = text.as_bytes();
        let expected = parse(&text).unwrap();
        let prebuilt = |change: &dyn Fn(usize, &mut Prebuilt<Value>)| -> Vec<Prebuilt<Value>> {
            scan_in_pieces(bytes, &[1 << 16])
                .unwrap()
                .into_iter()
                .enumerate()
                .map(|(i, (unit, text))| {
                    let value = parse_unit_with(&text, unit.depth, &mut ValueBuilder).unwrap();
                    let digest = crate::hash::sha256(&text);
                    let mut p = Prebuilt {
                        unit,
                        digest,
                        value,
                    };
                    change(i, &mut p);
                    p
                })
                .collect()
        };
        // Every unit given: the text is not scanned again.
        assert!(
            parse_bytes_parallel_prebuilt(bytes, &mut ValueBuilder, prebuilt(&|_, _| {}), true)
                .unwrap()
                == expected
        );
        // A unit whose text was not the bytes there (another file, a changed
        // byte) is never used, though its value would place.
        let stale = prebuilt(&|i, p| {
            if i == 3 {
                p.digest[5] ^= 1;
                p.value = Value::Null;
            }
        });
        assert!(
            parse_bytes_parallel_prebuilt(bytes, &mut ValueBuilder, stale, true).unwrap()
                == expected
        );
        // Some units given, the rest scanned and parsed here.
        let some: Vec<_> = prebuilt(&|_, _| {}).into_iter().step_by(3).collect();
        assert!(
            parse_bytes_parallel_prebuilt(bytes, &mut ValueBuilder, some, false).unwrap()
                == expected
        );
        // A unit list that cannot be placed falls back to one thread.
        let mut moved = prebuilt(&|_, _| {});
        moved[2].unit.start += 1;
        assert!(
            parse_bytes_parallel_prebuilt(bytes, &mut ValueBuilder, moved, true).unwrap()
                == expected
        );
        // A unit's own text must be one whole value.
        assert!(parse_unit_with(b"[1,2] ", 0, &mut ValueBuilder).is_none());
        assert!(parse_unit_with(b"[\"\xff\"]", 0, &mut ValueBuilder).is_none());
    }

    #[test]
    fn parallel_parsing_reports_the_sequential_error() {
        let text = large_document();
        let at = |needle: &str, nth: usize| text.match_indices(needle).nth(nth).unwrap().0;
        let mut broken = Vec::new();
        // Inside a unit: a bad token, a control character, a mismatched
        // bracket, an unterminated document.
        for (needle, nth, with) in [
            (r#""t":7,"#, 25, "\"t\":07,"),
            (r#""t":8,"#, 30, "\"t\":\"\u{1}\","),
            ("]}", 50, "}}"),
            ("]", 30, "]]"),
        ] {
            let mut t = text.clone();
            let i = at(needle, nth);
            t.replace_range(i..i + needle.len(), with);
            broken.push(t);
        }
        // In the text around the units.
        let mut t = text.clone();
        t.replace_range(at(r#""tail""#, 0)..at(r#""tail""#, 0) + 6, "tail");
        broken.push(t);
        broken.push(text[..text.len() - 3].to_owned());
        broken.push(text.replacen(
            r#""keys":["#,
            &format!(r#""keys":{}["#, "[".repeat(MAX_DEPTH)),
            1,
        ));
        for t in broken {
            let sequential = parse(&t).unwrap_err();
            assert_eq!(
                parse_bytes_parallel(t.as_bytes(), &mut ValueBuilder).unwrap_err(),
                sequential
            );
        }
    }

    #[test]
    fn bounds_nesting() {
        let deep = "[".repeat(MAX_DEPTH) + &"]".repeat(MAX_DEPTH);
        assert!(parse(&deep).is_ok());
        let deeper = "[".repeat(MAX_DEPTH + 1) + &"]".repeat(MAX_DEPTH + 1);
        assert_eq!(parse(&deeper).unwrap_err().message, "JSON nests too deeply");
    }
}
