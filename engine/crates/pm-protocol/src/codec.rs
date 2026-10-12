//! The MessagePack codec (contract 9).
//!
//! Envelopes encode as MessagePack with structs as maps keyed by their
//! camelCase field names (`rmp_serde::to_vec_named`), so the TypeScript side
//! decodes them with any conforming MessagePack decoder into objects that
//! match the generated types. Decoding is bounded (size, nesting depth, no
//! trailing bytes), checks safe-integer ranges, rejects NaN and infinities,
//! and runs every contract invariant: the decoder is an untrusted-input
//! surface and is fuzzed.
//!
//! **Every float on the wire is finite.** NaN and infinities have no JSON
//! form (agent tools serialize these messages as JSON), `serde_json::Value`
//! silently turns them into `null`, and every range check would have to
//! special-case them. The codec enforces the rule on the encoded bytes, so it
//! covers every float field, including ones nested in `serde_json` values,
//! without a per-field check.

use crate::envelope::Envelope;
use crate::validate::{Validate, Violation};

/// Largest encoded envelope. Bulk data uses shared surfaces or cache files.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// Deepest nesting the decoder accepts.
pub const MAX_DEPTH: usize = 64;

/// Encode or decode failure.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("message is {0} bytes; the limit is {MAX_MESSAGE_BYTES}")]
    TooLarge(usize),
    #[error("message has {0} trailing bytes")]
    TrailingBytes(usize),
    #[error("non-finite float at byte {offset}")]
    NonFinite { offset: usize },
    #[error("encode failed: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("decode failed: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    #[error(transparent)]
    Invalid(#[from] Violation),
}

impl From<CodecError> for crate::error::EngineError {
    fn from(error: CodecError) -> Self {
        crate::error::EngineError::new(
            crate::error::ErrorCode::InvalidMessage,
            crate::error::ErrorScope::Connection,
            error.to_string(),
        )
    }
}

/// Validates and encodes an envelope. Invalid messages, including ones with
/// a NaN or infinite float anywhere, fail at the sender.
pub fn encode(envelope: &Envelope) -> Result<Vec<u8>, CodecError> {
    envelope.validate()?;
    let bytes = encode_unvalidated(envelope)?;
    check_finite(&bytes)?;
    Ok(bytes)
}

/// Encodes without validating (tests and fuzzing only).
pub fn encode_unvalidated(envelope: &Envelope) -> Result<Vec<u8>, CodecError> {
    let bytes = rmp_serde::to_vec_named(envelope)?;
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(CodecError::TooLarge(bytes.len()));
    }
    Ok(bytes)
}

/// Decodes and validates an envelope.
pub fn decode(bytes: &[u8]) -> Result<Envelope, CodecError> {
    let envelope = decode_unvalidated(bytes)?;
    envelope.validate()?;
    Ok(envelope)
}

/// Decodes with size, depth, trailing-byte, and finite-float checks but
/// without contract validation.
pub fn decode_unvalidated(bytes: &[u8]) -> Result<Envelope, CodecError> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(CodecError::TooLarge(bytes.len()));
    }
    let mut deserializer = rmp_serde::Deserializer::new(std::io::Cursor::new(bytes));
    deserializer.set_max_depth(MAX_DEPTH);
    let envelope: Envelope = serde::Deserialize::deserialize(&mut deserializer)?;
    let rest = bytes.len() - deserializer.position() as usize;
    if rest != 0 {
        return Err(CodecError::TrailingBytes(rest));
    }
    check_finite(bytes)?;
    Ok(envelope)
}

/// Rejects any MessagePack float32 or float64 that is NaN or infinite.
///
/// A linear scan: container headers carry only counts, so every value of a
/// well-formed message is visited in order without tracking nesting. Bytes
/// that end inside a value are reported as non-finite at that offset rather
/// than panicking (the deserializer has already rejected them by the time
/// [`decode_unvalidated`] calls this, so that path is defensive).
pub fn check_finite(bytes: &[u8]) -> Result<(), CodecError> {
    let mut at = 0usize;
    let payload = |at: usize, n: usize| -> Result<&[u8], CodecError> {
        bytes
            .get(at + 1..at + 1 + n)
            .ok_or(CodecError::NonFinite { offset: at })
    };
    let length = |at: usize, n: usize| -> Result<usize, CodecError> {
        let raw = payload(at, n)?;
        Ok(raw.iter().fold(0usize, |acc, b| (acc << 8) | *b as usize))
    };
    while at < bytes.len() {
        let marker = bytes[at];
        // Bytes after the marker: fixed payloads, then any variable body.
        let skip = match marker {
            // positive fixint, fixmap, fixarray, nil, bool, negative fixint
            0x00..=0x7f | 0x80..=0x9f | 0xc0..=0xc3 | 0xe0..=0xff => 0,
            0xa0..=0xbf => (marker & 0x1f) as usize,
            0xca => {
                let raw = payload(at, 4)?;
                if !f32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]).is_finite() {
                    return Err(CodecError::NonFinite { offset: at });
                }
                4
            }
            0xcb => {
                let raw = payload(at, 8)?;
                let mut be = [0u8; 8];
                be.copy_from_slice(raw);
                if !f64::from_be_bytes(be).is_finite() {
                    return Err(CodecError::NonFinite { offset: at });
                }
                8
            }
            0xcc | 0xd0 => 1,
            0xcd | 0xd1 => 2,
            0xce | 0xd2 => 4,
            0xcf | 0xd3 => 8,
            0xd4 => 2,
            0xd5 => 3,
            0xd6 => 5,
            0xd7 => 9,
            0xd8 => 17,
            0xc4 | 0xd9 => 1 + length(at, 1)?,
            0xc5 | 0xda => 2 + length(at, 2)?,
            0xc6 | 0xdb => 4 + length(at, 4)?,
            0xc7 => 2 + length(at, 1)?,
            0xc8 => 3 + length(at, 2)?,
            0xc9 => 5 + length(at, 4)?,
            0xdc | 0xde => 2,
            0xdd | 0xdf => 4,
            0xc1 => return Err(CodecError::NonFinite { offset: at }),
        };
        at = at
            .checked_add(1 + skip)
            .filter(|&next| next <= bytes.len())
            .ok_or(CodecError::NonFinite { offset: at })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{RequestId, SessionId};
    use crate::envelope::{Message, Request};

    #[test]
    fn rejects_trailing_bytes_and_oversized_input() {
        let envelope = Envelope::new(Message::Request {
            id: RequestId::ZERO,
            body: Request::CloseSession,
        })
        .with_session(SessionId::ZERO);
        let mut bytes = encode(&envelope).unwrap();
        assert_eq!(decode(&bytes).unwrap(), envelope);
        bytes.push(0xc0);
        assert!(matches!(decode(&bytes), Err(CodecError::TrailingBytes(1))));
        assert!(matches!(
            decode(&vec![0; MAX_MESSAGE_BYTES + 1]),
            Err(CodecError::TooLarge(_))
        ));
    }

    #[test]
    fn rejects_deep_nesting() {
        // 200 nested single-element arrays.
        let mut bytes = vec![0x91; 200];
        bytes.push(0xc0);
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn non_finite_floats_are_rejected_on_both_sides() {
        let finite = Envelope::new(Message::Request {
            id: RequestId::ZERO,
            body: Request::BlenderStatus,
        });
        let bytes = encode(&finite).unwrap();
        assert!(check_finite(&bytes).is_ok());
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut float = vec![0xcb];
            float.extend_from_slice(&bad.to_be_bytes());
            assert!(matches!(
                check_finite(&float),
                Err(CodecError::NonFinite { offset: 0 })
            ));
            let mut single = vec![0x92, 0x01, 0xca];
            single.extend_from_slice(&(bad as f32).to_be_bytes());
            assert!(matches!(
                check_finite(&single),
                Err(CodecError::NonFinite { offset: 2 })
            ));
        }
        // Floats inside strings and binary are bytes, not floats.
        let mut bin = vec![0xc4, 9, 0xcb];
        bin.extend_from_slice(&f64::NAN.to_be_bytes());
        assert!(check_finite(&bin).is_ok());
        let mut fixstr = vec![0xa1, 0xcb];
        fixstr.push(0xc0);
        assert!(check_finite(&fixstr).is_ok());
        // Truncated input reports instead of panicking.
        assert!(check_finite(&[0xcb, 0, 0]).is_err());
        assert!(check_finite(&[0xdb, 0xff, 0xff, 0xff, 0xff]).is_err());
    }

    #[test]
    fn invalid_messages_fail_at_the_sender() {
        let envelope = Envelope::new(Message::Request {
            id: RequestId::ZERO,
            body: Request::CloseSession,
        });
        assert!(matches!(encode(&envelope), Err(CodecError::Invalid(_))));
        let bytes = encode_unvalidated(&envelope).unwrap();
        assert!(matches!(decode(&bytes), Err(CodecError::Invalid(_))));
        assert!(decode_unvalidated(&bytes).is_ok());
    }
}
