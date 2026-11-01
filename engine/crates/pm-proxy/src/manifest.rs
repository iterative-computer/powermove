use crate::{ProxyError, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceRate {
    pub num: u32,
    pub den: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceFrame {
    pub path: PathBuf,
    pub hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceManifest {
    pub powermove_sequence: u8,
    pub rate: SequenceRate,
    pub frames: Vec<SequenceFrame>,
}

impl SequenceManifest {
    /// Stable across moves and relinks: only rational rate and ordered raw
    /// frame content hashes participate. The input strings are normalized to
    /// lowercase hex before hashing.
    pub fn content_hash(&self) -> Result<String> {
        if self.powermove_sequence != 1
            || self.rate.num == 0
            || self.rate.den == 0
            || self.rate.num > i32::MAX as u32
            || self.frames.is_empty()
            || self.frames.len() > 1_000_000
        {
            return Err(ProxyError::InvalidMedia(
                "invalid image-sequence manifest".into(),
            ));
        }
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"pm-sequence-v1\n");
        hasher.update(format!("{}/{}\n", self.rate.num, self.rate.den).as_bytes());
        for frame in &self.frames {
            if !frame.path.is_absolute()
                || frame.hash.len() != 64
                || !frame.hash.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(ProxyError::InvalidMedia(
                    "sequence frame requires an absolute path and BLAKE3 hash".into(),
                ));
            }
            hasher.update(frame.hash.to_ascii_lowercase().as_bytes());
            hasher.update(b"\n");
        }
        Ok(hasher.finalize().to_hex().to_string())
    }
}
