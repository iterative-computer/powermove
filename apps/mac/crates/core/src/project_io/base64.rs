//! Standard base64 decoding for legacy JSON projects, which embedded media as
//! `media: { <id>: { type, data: "<base64>" } }` inside the document.
//! Mirrors the old validator: length divisible by 4, `[A-Za-z0-9+/]*={0,2}`.

pub fn decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if bytes.len() % 4 != 0 {
        return None;
    }
    let pad = bytes.iter().rev().take_while(|b| **b == b'=').count();
    if pad > 2 {
        return None;
    }
    let body = &bytes[..bytes.len() - pad];
    let value = |b: u8| -> Option<u32> {
        match b {
            b'A'..=b'Z' => Some((b - b'A') as u32),
            b'a'..=b'z' => Some((b - b'a') as u32 + 26),
            b'0'..=b'9' => Some((b - b'0') as u32 + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0;
    for &b in body {
        acc = (acc << 6) | value(b)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn decodes_with_and_without_padding() {
        assert_eq!(decode("").unwrap(), b"");
        assert_eq!(decode("TQ==").unwrap(), b"M");
        assert_eq!(decode("TWE=").unwrap(), b"Ma");
        assert_eq!(decode("TWFu").unwrap(), b"Man");
        assert!(decode("TWF").is_none());
        assert!(decode("TW*u").is_none());
        assert!(decode("T===").is_none());
    }
}
