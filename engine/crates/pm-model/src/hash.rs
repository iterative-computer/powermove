//! The canonical document hash: SHA-256 of the canonical serialization
//! ([`crate::json::write`]), as 64 lowercase hex digits. The production
//! consistency check (section 6, item 7) compares this with the renderer's
//! `canonicalHash(canonicalProject(PM.proj))` from
//! `apps/desktop/src/shared/canonical-json.ts`.

use sha2::{Digest, Sha256};

use crate::json::write::{JsonTree, Sink, write_canonical};

struct HashSink(Sha256);

impl Sink for HashSink {
    fn push_str(&mut self, s: &str) {
        self.0.update(s.as_bytes());
    }
}

/// SHA-256 of the canonical serialization, without building the whole string.
pub fn canonical_digest<T: JsonTree>(value: &T) -> [u8; 32] {
    let mut sink = HashSink(Sha256::new());
    write_canonical(value, &mut sink);
    sink.0.finalize().into()
}

/// [`canonical_digest`] as lowercase hex.
pub fn canonical_hash<T: JsonTree>(value: &T) -> String {
    to_hex(&canonical_digest(value))
}

/// SHA-256 of raw bytes as lowercase hex (PMV4 checksums).
pub fn sha256_hex(bytes: &[u8]) -> String {
    to_hex(&Sha256::digest(bytes).into())
}

fn to_hex(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &b in digest {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::{Value, parse};

    #[test]
    fn hashes_the_canonical_bytes() {
        // sha256("{}") and sha256("") are well-known.
        assert_eq!(
            canonical_hash(&Value::Object(Default::default())),
            "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let a = parse(r#"{"b":[1,2],"a":"x"}"#).unwrap();
        let b = parse(r#"{"a":"x","b":[1.0,2e0]}"#).unwrap();
        assert_eq!(canonical_hash(&a), canonical_hash(&b));
        assert_eq!(canonical_hash(&a), sha256_hex(br#"{"a":"x","b":[1,2]}"#));
    }
}
