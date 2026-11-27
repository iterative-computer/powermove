use crate::{Error, Result, hex};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Exact shipped registry grammar. Deserialize validates before IDs reach paths.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ExtensionId(String);
impl ExtensionId {
    pub fn new(id: impl Into<String>) -> Result<Self> {
        let id = id.into();
        if !(2..=64).contains(&id.len())
            || !id.as_bytes()[0].is_ascii_alphanumeric()
            || !id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(Error::Invalid("extension id"));
        }
        Ok(Self(id))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for ExtensionId {
    type Error = Error;
    fn try_from(s: String) -> Result<Self> {
        Self::new(s)
    }
}
impl From<ExtensionId> for String {
    fn from(id: ExtensionId) -> Self {
        id.0
    }
}

/// Stable profile-scoped key; never derive it from a release, panel or PID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataStoreIdentity {
    pub extension: ExtensionId,
    pub key: String,
    pub schema: u32,
}
impl DataStoreIdentity {
    pub fn new(profile_namespace: &str, extension: ExtensionId) -> Result<Self> {
        if profile_namespace.is_empty() {
            return Err(Error::Invalid("profile namespace"));
        }
        let mut h = Sha256::new();
        h.update(b"powermove.store-data.v1\0");
        for s in [profile_namespace, extension.as_str()] {
            h.update((s.len() as u64).to_be_bytes());
            h.update(s.as_bytes());
        }
        Ok(Self {
            extension,
            key: hex(&h.finalize()),
            schema: 1,
        })
    }
    pub fn kernel_storage_key(&self) -> String {
        format!("ext.{}", self.extension.as_str())
    }
    /// UUID spelling for a WKWebsiteDataStore registry adapter. Full key stays
    /// authoritative; adapters must detect truncated UUID collisions.
    pub fn webkit_uuid(&self) -> Result<String> {
        if self.key.len() != 64 || !self.key.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Invalid("data store key"));
        }
        let k = &self.key;
        Ok(format!(
            "{}-{}-{}-{}-{}",
            &k[..8],
            &k[8..12],
            &k[12..16],
            &k[16..20],
            &k[20..32]
        ))
    }
}

/// v1.2.0 uses a shared Chromium profile plus an app:// host, not a per-ID
/// persist: partition. Snapshot reader ownership is first-launch/migrate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyImportPlan {
    pub extension: ExtensionId,
    pub origin: String,
    pub json_storage_key: String,
    pub snapshot_shared_profile: bool,
    pub preserve_unknown_records: bool,
    pub import_cookies_into_renderer: bool,
}
impl LegacyImportPlan {
    pub fn v1_2_0(id: ExtensionId) -> Self {
        let normalized = id
            .as_str()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        let slug = normalized.chars().take(20).collect::<String>();
        let slug = slug.trim_matches('-');
        let digest = Sha256::digest(id.as_str().as_bytes());
        let alphabet = b"abcdefghijklmnopqrstuvwxyz234567";
        let mut code = String::new();
        for index in 0..26 {
            let bit = index * 5;
            let pair = u16::from(digest[bit / 8]) << 8 | u16::from(digest[bit / 8 + 1]);
            code.push(alphabet[((pair >> (11 - bit % 8)) & 31) as usize] as char);
        }
        let host = if slug.is_empty() {
            format!("x-{code}")
        } else {
            format!("x-{slug}-{code}")
        };
        Self {
            json_storage_key: format!("ext.{}", id.as_str()),
            extension: id,
            origin: format!("app://{host}"),
            snapshot_shared_profile: true,
            preserve_unknown_records: true,
            import_cookies_into_renderer: false,
        }
    }
}
