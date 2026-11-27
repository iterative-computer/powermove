//! Standard base64 decoding for legacy JSON projects, which embedded media as
//! `media: { <id>: { type, data: "<base64>" } }` inside the document.
//! Mirrors the old validator: length divisible by 4, `[A-Za-z0-9+/]*={0,2}`.

use base64::{
    Engine,
    alphabet::STANDARD,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};

/// The old validator ignored the unused bits of the last character.
const LEGACY: GeneralPurpose = GeneralPurpose::new(
    &STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::RequireCanonical)
        .with_decode_allow_trailing_bits(true),
);

pub fn decode(text: &str) -> Option<Vec<u8>> {
    LEGACY.decode(text).ok()
}

pub fn encode(bytes: &[u8]) -> String {
    LEGACY.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    #[test]
    fn decodes_with_and_without_padding() {
        assert_eq!(decode("").unwrap(), b"");
        assert_eq!(decode("TQ==").unwrap(), b"M");
        assert_eq!(decode("TWE=").unwrap(), b"Ma");
        assert_eq!(decode("TWFu").unwrap(), b"Man");
        assert!(decode("TWF").is_none());
        assert!(decode("TW*u").is_none());
        assert!(decode("T===").is_none());
        assert!(decode("====").is_none());
        assert!(decode("T=Q=").is_none());
        assert_eq!(decode("TR==").unwrap(), b"M");
        assert_eq!(encode(b"Ma"), "TWE=");
    }
}
