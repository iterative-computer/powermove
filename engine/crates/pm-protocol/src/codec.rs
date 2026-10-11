//! The MessagePack codec (contract 9).
//!
//! Envelopes encode as MessagePack with structs as maps keyed by their
//! camelCase field names (`rmp_serde::to_vec_named`), so the TypeScript side
//! decodes them with any conforming MessagePack decoder into objects that
//! match the generated types. Decoding is bounded (size, nesting depth, no
//! trailing bytes), checks safe-integer ranges, and runs every contract
//! invariant: the decoder is an untrusted-input surface and is fuzzed.

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

/// Validates and encodes an envelope. Invalid messages fail at the sender.
pub fn encode(envelope: &Envelope) -> Result<Vec<u8>, CodecError> {
    envelope.validate()?;
    encode_unvalidated(envelope)
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

/// Decodes with size, depth, and trailing-byte checks but without contract
/// validation.
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
    Ok(envelope)
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
