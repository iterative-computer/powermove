//! Identifier and size rules shared with `src/shared/ipc.ts` and the
//! extension manifest. Paths are only ever derived from values that pass these.

/// `PROJECT_ID = /^[A-Za-z0-9_-]{1,120}$/`
pub fn is_project_id(value: &str) -> bool {
    (1..=120).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// `EXTENSION_ID = /^[a-z0-9][a-z0-9-]{1,63}$/`
pub fn is_extension_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (2..=64).contains(&bytes.len())
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes.iter().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

/// `LIMITS` from ipc.ts.
pub const FILE_SAVE_BYTES: usize = 256 * 1024 * 1024;
pub const STORE_VALUE_BYTES: usize = 32 * 1024 * 1024;
pub const LOG_CHARS: usize = 8_000;
/// Chunk size the renderer uploads and reads in (`FILE_CHUNK_BYTES`).
pub const FILE_CHUNK_BYTES: usize = 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_ids() {
        assert!(is_project_id("abc_DEF-123"));
        assert!(!is_project_id("../x"));
        assert!(!is_project_id(""));
        assert!(is_extension_id("timeline"));
        assert!(!is_extension_id("-x"));
        assert!(!is_extension_id("Timeline"));
    }
}
