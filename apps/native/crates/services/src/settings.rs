//! App and extension preferences remain kernel-owned. Service-local durable
//! metadata uses ServiceStorage; it is not a second app settings authority.
use anyhow::{Result, ensure};
use serde_json::Value;

pub trait SettingsHook {
    fn get(&mut self, key: &str) -> Result<Option<Value>>;
    /// Apply through the kernel state transaction and return its publication revision.
    /// None archives/deletes a key; Some(Null) is an explicit JSON null.
    fn apply(&mut self, key: &str, value: Option<Value>) -> Result<u64>;
}
pub struct SettingsService<H>(pub H);
impl<H: SettingsHook> SettingsService<H> {
    pub fn get(&mut self, key: &str) -> Result<Option<Value>> {
        validate(key)?;
        self.0.get(key)
    }
    pub fn set(&mut self, key: &str, value: Value) -> Result<u64> {
        validate(key)?;
        ensure!(
            serde_json::to_vec(&value)?.len() <= crate::storage::MAX_SETTING_BYTES,
            "setting exceeds 32 MiB"
        );
        self.0.apply(key, Some(value))
    }
    pub fn archive(&mut self, key: &str) -> Result<u64> {
        validate(key)?;
        self.0.apply(key, None)
    }
}
fn validate(key: &str) -> Result<()> {
    ensure!(
        !key.is_empty() && key.len() <= 512 && !key.chars().any(char::is_control),
        "invalid settings key"
    );
    Ok(())
}
