//! A minimal JSON scanner for id-keyed reconciliation.
//!
//! It splits a JSON array into element byte ranges and finds each object
//! element's top-level `"id"` string without building values. Elements whose
//! bytes are unchanged are never parsed; changed elements are parsed (and so
//! fully validated) by `serde_json`.

use memchr::memchr2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanError {
    pub offset: usize,
    pub what: &'static str,
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JSON scan error at byte {}: {}", self.offset, self.what)
    }
}

impl std::error::Error for ScanError {}

type Result<T> = std::result::Result<T, ScanError>;

fn err<T>(offset: usize, what: &'static str) -> Result<T> {
    Err(ScanError { offset, what })
}

#[inline]
pub fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\n' | b'\r' | b'\t') {
        i += 1;
    }
    i
}

/// `i` points at the opening quote. Returns the index after the closing quote
/// and whether the string contains escapes.
#[inline]
pub fn skip_string(bytes: &[u8], i: usize) -> Result<(usize, bool)> {
    debug_assert_eq!(bytes[i], b'"');
    let mut j = i + 1;
    let mut escaped = false;
    loop {
        match memchr2(b'"', b'\\', &bytes[j..]) {
            None => return err(i, "unterminated string"),
            Some(k) => {
                let at = j + k;
                if bytes[at] == b'"' {
                    return Ok((at + 1, escaped));
                }
                escaped = true;
                if at + 1 >= bytes.len() {
                    return err(at, "unterminated escape");
                }
                j = at + 2;
            }
        }
    }
}

/// Skip one JSON value starting at `i` (no leading whitespace). Returns the
/// index just past it. Scalars are delimited, not validated.
pub fn skip_value(bytes: &[u8], i: usize) -> Result<usize> {
    if i >= bytes.len() {
        return err(i, "expected a value");
    }
    match bytes[i] {
        b'"' => Ok(skip_string(bytes, i)?.0),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut j = i;
            while j < bytes.len() {
                match bytes[j] {
                    b'"' => {
                        j = skip_string(bytes, j)?.0;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Ok(j + 1);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            err(i, "unterminated container")
        }
        _ => {
            let mut j = i;
            while j < bytes.len()
                && !matches!(bytes[j], b',' | b']' | b'}' | b' ' | b'\n' | b'\r' | b'\t')
            {
                j += 1;
            }
            if j == i {
                err(i, "expected a value")
            } else {
                Ok(j)
            }
        }
    }
}

/// Find the top-level `"id"` string of the object starting at `i`. Returns
/// the raw string span (without quotes), whether it has escapes, and the end
/// of the object.
/// Raw span of a string body: (start, end, has escapes).
pub type StrSpan = (usize, usize, bool);

pub fn object_id_and_end(bytes: &[u8], i: usize) -> Result<(Option<StrSpan>, usize)> {
    debug_assert_eq!(bytes[i], b'{');
    let mut id = None;
    let mut j = skip_ws(bytes, i + 1);
    if j < bytes.len() && bytes[j] == b'}' {
        return Ok((None, j + 1));
    }
    loop {
        if j >= bytes.len() || bytes[j] != b'"' {
            return err(j, "expected an object key");
        }
        let (key_end, key_escaped) = skip_string(bytes, j)?;
        let is_id = !key_escaped && &bytes[j + 1..key_end - 1] == b"id";
        j = skip_ws(bytes, key_end);
        if j >= bytes.len() || bytes[j] != b':' {
            return err(j, "expected ':'");
        }
        j = skip_ws(bytes, j + 1);
        let value_start = j;
        if is_id && j < bytes.len() && bytes[j] == b'"' {
            let (end, escaped) = skip_string(bytes, j)?;
            // JSON.parse keeps the last duplicate key; so do we.
            id = Some((value_start + 1, end - 1, escaped));
            j = end;
        } else {
            if is_id {
                id = None;
            }
            j = skip_value(bytes, j)?;
        }
        j = skip_ws(bytes, j);
        if j >= bytes.len() {
            return err(j, "unterminated object");
        }
        match bytes[j] {
            b',' => j = skip_ws(bytes, j + 1),
            b'}' => return Ok((id, j + 1)),
            _ => return err(j, "expected ',' or '}'"),
        }
    }
}

/// Decode a JSON string body (between quotes) that contains escapes.
pub fn unescape(raw: &[u8]) -> Option<String> {
    let mut quoted = Vec::with_capacity(raw.len() + 2);
    quoted.push(b'"');
    quoted.extend_from_slice(raw);
    quoted.push(b'"');
    serde_json::from_slice::<String>(&quoted).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_ids_and_ends() {
        let s = br#"{"name":"a","id":"L1","p":{"id":"inner"},"x":[1,{"id":2}]} "#;
        let (id, end) = object_id_and_end(s, 0).unwrap();
        let (a, b, esc) = id.unwrap();
        assert_eq!(&s[a..b], b"L1");
        assert!(!esc);
        assert_eq!(end, s.len() - 1);
    }

    #[test]
    fn strings_with_escapes_and_brackets() {
        let s = br#"{"id":"a\"b","t":"}]{[\\"}"#;
        let (id, end) = object_id_and_end(s, 0).unwrap();
        let (a, b, esc) = id.unwrap();
        assert!(esc);
        assert_eq!(unescape(&s[a..b]).unwrap(), "a\"b");
        assert_eq!(end, s.len());
    }

    #[test]
    fn skips_scalars_and_rejects_garbage() {
        assert_eq!(skip_value(b"123,", 0).unwrap(), 3);
        assert_eq!(skip_value(b"true]", 0).unwrap(), 4);
        assert!(skip_value(b"\"abc", 0).is_err());
        assert!(skip_value(b"{\"a\":[1,2", 0).is_err());
        assert!(object_id_and_end(b"{\"a\" 1}", 0).is_err());
    }
}
