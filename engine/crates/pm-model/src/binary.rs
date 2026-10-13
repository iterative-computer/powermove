//! The canonical binary patch format, version 1 (contract 5,
//! `PatchEncoding::CanonicalV1`; `docs/engine/rfcs/05-patches.md`).
//!
//! The body of every `PatchBatch`: a list of [`Op`]s (leaf patches and
//! id-keyed structural operations). `apps/desktop/src/shared/patch-binary.ts`
//! is the renderer's encoder; this module encodes and decodes, and the
//! cross-language golden vectors (`tests/vectors/patch-binary-vectors.json`)
//! pin both to the same bytes.
//!
//! ```text
//! body     = magic count op*              magic = 50 4D 50 01 ("PMP", version 1)
//! count    = u32                          number of operations
//! op       = 0x00 path                    leaf delete     {path, exists: false}
//!          | 0x01 path json               leaf set        {path, exists: true, value}
//!          | 0x02 path                    leaf set        {path, exists: true} (value undefined)
//!          | 0x10 path u32 str*           order           {path, ids}
//!          | 0x11 path u32 json           insert          {path, index, value}
//!          | 0x12 path str                delete by id    {path, id}
//! path     = u32 part*                    number of parts, then the parts
//! part     = 0x00 str                     string key
//!          | 0x01 u32                     integer index 0 to 2^32 - 1
//!          | 0x02 f64                     any other non-negative integer (-0, or 2^32 and above)
//! str      = u32 bytes                    WTF-8 (UTF-8, plus unpaired surrogates as 3 bytes)
//! json     = u32 bytes                    UTF-8 JSON text
//! ```
//!
//! Integers are little-endian; `u32` lengths count bytes. Every number in a
//! path is a non-negative integer, as `isPatchList` requires.
//!
//! **Canonical form.** Encoders write JSON text as the canonical
//! serialization (`canonicalStringify`, sorted keys) and choose the shortest
//! part form (`0x01` for every index that fits in 32 bits). Two equal
//! operation lists therefore encode to the same bytes, and every body
//! re-encodes to itself ([`is_canonical`]). Decoders reject every structural
//! deviation (unknown tags, truncation, trailing bytes, a non-canonical part
//! form, invalid WTF-8 or UTF-8, an insert whose value is not an object with a
//! string `id`) but accept any valid JSON text, so a producer may hand over
//! JSON it already holds; [`is_canonical`] tells the two apart.
//!
//! Values are JSON, not a binary value encoding, because the renderer
//! already produces JSON text and the mirror's parser is the one tested
//! against `JSON.parse`; the framing lets a receiver skip or compare a value
//! without scanning it.

use core::fmt;

use crate::json::{self, JsString, Value, parse_bytes};
use crate::ops::Op;
use crate::patch::{Patch, PathPart, record_id};

/// The first four bytes of a version 1 body.
pub const MAGIC: [u8; 4] = *b"PMP\x01";

/// Operation tags.
pub mod tag {
    pub const LEAF_DELETE: u8 = 0x00;
    pub const LEAF_SET: u8 = 0x01;
    pub const LEAF_SET_UNDEFINED: u8 = 0x02;
    pub const ORDER: u8 = 0x10;
    pub const INSERT: u8 = 0x11;
    pub const DELETE_ID: u8 = 0x12;
}

/// Path part tags.
pub mod part {
    pub const KEY: u8 = 0x00;
    pub const INDEX: u8 = 0x01;
    pub const NUMBER: u8 = 0x02;
}

/// The smallest encoded operation: a tag and an empty path.
const MIN_OP_BYTES: usize = 5;

/// Why operations could not be encoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodeError {
    /// The operation that failed.
    pub index: usize,
    pub message: &'static str,
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "operation {}: {}", self.index, self.message)
    }
}

impl std::error::Error for EncodeError {}

/// Why a body could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError {
    /// Byte offset of the problem.
    pub offset: usize,
    pub message: &'static str,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for DecodeError {}

// ── Encoding ────────────────────────────────────────────────────────────────

struct Writer {
    out: Vec<u8>,
    index: usize,
}

impl Writer {
    fn err(&self, message: &'static str) -> EncodeError {
        EncodeError {
            index: self.index,
            message,
        }
    }

    fn u32(&mut self, n: usize) -> Result<(), EncodeError> {
        let n = u32::try_from(n).map_err(|_| self.err("a length exceeds 2^32 - 1"))?;
        self.out.extend_from_slice(&n.to_le_bytes());
        Ok(())
    }

    fn bytes(&mut self, bytes: &[u8]) -> Result<(), EncodeError> {
        self.u32(bytes.len())?;
        self.out.extend_from_slice(bytes);
        Ok(())
    }

    fn path(&mut self, path: &[PathPart]) -> Result<(), EncodeError> {
        self.u32(path.len())?;
        for p in path {
            match p {
                PathPart::Key(k) => {
                    self.out.push(part::KEY);
                    self.bytes(k.as_bytes())?;
                }
                PathPart::Index(n) => {
                    let n = *n;
                    if !(n.is_finite() && n >= 0.0 && n.fract() == 0.0) {
                        return Err(self.err("path numbers must be non-negative integers"));
                    }
                    if n.is_sign_positive() && n <= u32::MAX as f64 {
                        self.out.push(part::INDEX);
                        self.out.extend_from_slice(&(n as u32).to_le_bytes());
                    } else {
                        self.out.push(part::NUMBER);
                        self.out.extend_from_slice(&n.to_le_bytes());
                    }
                }
            }
        }
        Ok(())
    }

    fn json(&mut self, value: &Value) -> Result<(), EncodeError> {
        let text = json::to_canonical_string(value);
        self.bytes(text.as_bytes())
    }
}

/// Encodes operations in the canonical form.
pub fn encode(ops: &[Op]) -> Result<Vec<u8>, EncodeError> {
    let mut w = Writer {
        out: Vec::with_capacity(64),
        index: 0,
    };
    w.out.extend_from_slice(&MAGIC);
    w.u32(ops.len())?;
    for (index, op) in ops.iter().enumerate() {
        w.index = index;
        match op {
            Op::Leaf(Patch {
                path,
                exists: false,
                ..
            }) => {
                w.out.push(tag::LEAF_DELETE);
                w.path(path)?;
            }
            Op::Leaf(Patch {
                path,
                exists: true,
                value: Some(value),
            }) => {
                w.out.push(tag::LEAF_SET);
                w.path(path)?;
                w.json(value)?;
            }
            Op::Leaf(Patch {
                path,
                exists: true,
                value: None,
            }) => {
                w.out.push(tag::LEAF_SET_UNDEFINED);
                w.path(path)?;
            }
            Op::Order { path, ids } => {
                w.out.push(tag::ORDER);
                w.path(path)?;
                w.u32(ids.len())?;
                for id in ids {
                    w.bytes(id.as_bytes())?;
                }
            }
            Op::Insert { path, index, value } => {
                if record_id(value).is_none() {
                    return Err(w.err("an inserted value must be an object with a string id"));
                }
                w.out.push(tag::INSERT);
                w.path(path)?;
                w.out.extend_from_slice(&index.to_le_bytes());
                w.json(value)?;
            }
            Op::Delete { path, id } => {
                w.out.push(tag::DELETE_ID);
                w.path(path)?;
                w.bytes(id.as_bytes())?;
            }
        }
    }
    Ok(w.out)
}

// ── Decoding ────────────────────────────────────────────────────────────────

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn err(&self, message: &'static str) -> DecodeError {
        DecodeError {
            offset: self.at,
            message,
        }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self
            .at
            .checked_add(n)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| self.err("truncated body"))?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn f64(&mut self) -> Result<f64, DecodeError> {
        let b = self.take(8)?;
        Ok(f64::from_le_bytes(b.try_into().expect("8 bytes")))
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.at
    }

    fn chunk(&mut self) -> Result<&'a [u8], DecodeError> {
        let len = self.u32()? as usize;
        self.take(len)
    }

    fn string(&mut self) -> Result<JsString, DecodeError> {
        let start = self.at;
        let bytes = self.chunk()?;
        JsString::from_wtf8(bytes).ok_or(DecodeError {
            offset: start,
            message: "invalid WTF-8 string",
        })
    }

    fn json(&mut self) -> Result<Value, DecodeError> {
        let start = self.at + 4;
        let bytes = self.chunk()?;
        parse_bytes(bytes).map_err(|e| DecodeError {
            offset: start + e.offset,
            message: e.message,
        })
    }

    /// A count of items that each take at least `min` bytes, bounded by the
    /// bytes left so a corrupt count cannot allocate.
    fn count(&mut self, min: usize) -> Result<usize, DecodeError> {
        let n = self.u32()? as usize;
        if n.saturating_mul(min) > self.remaining() {
            return Err(self.err("count exceeds the remaining bytes"));
        }
        Ok(n)
    }

    fn path(&mut self) -> Result<Vec<PathPart>, DecodeError> {
        let n = self.count(5)?;
        let mut path = Vec::with_capacity(n);
        for _ in 0..n {
            let at = self.at;
            path.push(match self.u8()? {
                part::KEY => PathPart::Key(self.string()?),
                part::INDEX => PathPart::Index(self.u32()? as f64),
                part::NUMBER => {
                    let n = self.f64()?;
                    let integer = n.is_finite() && n >= 0.0 && n.fract() == 0.0;
                    let fits = n.is_sign_positive() && n <= u32::MAX as f64;
                    if !integer {
                        return Err(DecodeError {
                            offset: at,
                            message: "path numbers must be non-negative integers",
                        });
                    }
                    if fits {
                        return Err(DecodeError {
                            offset: at,
                            message: "non-canonical path number (fits the index form)",
                        });
                    }
                    PathPart::Index(n)
                }
                _ => {
                    return Err(DecodeError {
                        offset: at,
                        message: "unknown path part tag",
                    });
                }
            });
        }
        Ok(path)
    }
}

/// Decodes a version 1 body.
pub fn decode(bytes: &[u8]) -> Result<Vec<Op>, DecodeError> {
    let mut r = Reader { bytes, at: 0 };
    if r.take(4).map_err(|_| r.err("missing magic"))? != MAGIC {
        return Err(DecodeError {
            offset: 0,
            message: "not a version 1 patch body",
        });
    }
    let count = r.count(MIN_OP_BYTES)?;
    let mut ops = Vec::with_capacity(count);
    for _ in 0..count {
        let at = r.at;
        let op = match r.u8()? {
            tag::LEAF_DELETE => Op::Leaf(Patch::delete(r.path()?)),
            tag::LEAF_SET => {
                let path = r.path()?;
                Op::Leaf(Patch::set(path, r.json()?))
            }
            tag::LEAF_SET_UNDEFINED => Op::Leaf(Patch {
                path: r.path()?,
                exists: true,
                value: None,
            }),
            tag::ORDER => {
                let path = r.path()?;
                let n = r.count(4)?;
                let mut ids = Vec::with_capacity(n);
                for _ in 0..n {
                    ids.push(r.string()?);
                }
                Op::Order { path, ids }
            }
            tag::INSERT => {
                let path = r.path()?;
                let index = r.u32()?;
                let value_at = r.at;
                let value = r.json()?;
                if record_id(&value).is_none() {
                    return Err(DecodeError {
                        offset: value_at,
                        message: "an inserted value must be an object with a string id",
                    });
                }
                Op::Insert { path, index, value }
            }
            tag::DELETE_ID => {
                let path = r.path()?;
                Op::Delete {
                    path,
                    id: r.string()?,
                }
            }
            _ => {
                return Err(DecodeError {
                    offset: at,
                    message: "unknown operation tag",
                });
            }
        };
        ops.push(op);
    }
    if r.remaining() != 0 {
        return Err(r.err("trailing bytes after the last operation"));
    }
    Ok(ops)
}

/// Whether `bytes` is a valid body in canonical form (it decodes and
/// re-encodes to the same bytes).
pub fn is_canonical(bytes: &[u8]) -> bool {
    decode(bytes)
        .ok()
        .and_then(|ops| encode(&ops).ok())
        .is_some_and(|again| again == bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::parse;
    use crate::path;

    #[test]
    fn round_trips_every_kind() {
        let ops = vec![
            Op::Leaf(Patch::set(
                path!["comps", "c1", "layers", 3, "p"],
                parse(r#"{"b":1,"a":[true,null]}"#).unwrap(),
            )),
            Op::Leaf(Patch::delete(path!["x"])),
            Op::Leaf(Patch {
                path: path!["y"].to_vec(),
                exists: true,
                value: None,
            }),
            Op::Order {
                path: path!["l"].to_vec(),
                ids: vec!["b".into(), JsString::from_utf16_units(&[0xd800])],
            },
            Op::Insert {
                path: vec![],
                index: 7,
                value: parse(r#"{"id":"n"}"#).unwrap(),
            },
            Op::Delete {
                path: vec![PathPart::Index(-0.0), PathPart::Index(4294967296.0)],
                id: "z".into(),
            },
        ];
        let bytes = encode(&ops).unwrap();
        assert_eq!(&bytes[..4], b"PMP\x01");
        let back = decode(&bytes).unwrap();
        assert_eq!(back, ops);
        // -0 survives as a distinct part form.
        let Op::Delete { path, .. } = &back[5] else {
            panic!()
        };
        assert!(matches!(path[0], PathPart::Index(n) if n == 0.0 && n.is_sign_negative()));
        assert!(is_canonical(&bytes));
        // Every truncation fails cleanly.
        for end in 0..bytes.len() {
            assert!(decode(&bytes[..end]).is_err(), "truncated at {end}");
        }
    }

    #[test]
    fn rejects_non_canonical_and_invalid_input() {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.push(tag::LEAF_DELETE);
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.push(part::NUMBER);
        bytes.extend_from_slice(&3.0f64.to_le_bytes());
        assert_eq!(
            decode(&bytes).unwrap_err().message,
            "non-canonical path number (fits the index form)"
        );
        let mut fraction = bytes.clone();
        let n = fraction.len();
        fraction[n - 8..].copy_from_slice(&1.5f64.to_le_bytes());
        assert!(decode(&fraction).is_err());
        assert!(encode(&[Op::Leaf(Patch::delete(vec![PathPart::Index(1.5)]))]).is_err());
        assert!(
            encode(&[Op::Insert {
                path: vec![],
                index: 0,
                value: Value::Null
            }])
            .is_err()
        );
        // Non-canonical JSON decodes but is not canonical.
        let ops = [Op::Leaf(Patch::set(
            path!["a"],
            parse(r#"{"a":1,"b":2}"#).unwrap(),
        ))];
        let canonical = encode(&ops).unwrap();
        let text = br#"{"b":2,"a":1}"#;
        let mut loose = canonical[..canonical.len() - text.len()].to_vec();
        loose.extend_from_slice(text);
        assert_eq!(decode(&loose).unwrap(), ops);
        assert!(!is_canonical(&loose));
        assert!(is_canonical(&canonical));
        // A huge count cannot allocate.
        let mut huge = MAGIC.to_vec();
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&huge).is_err());
    }
}
