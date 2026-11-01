//! Chromium localStorage `_origin\0<encoded key>` records. Export every origin
//! and arbitrary extension key, not merely a Powermove allowlist.
use crate::{Result, leveldb::Database, snapshot::Snapshot, wire::utf16};
use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub origin: String,
    pub key: String,
    pub value: String,
    /// Convenience only: the exact DOM string remains authoritative.
    pub json: Option<Value>,
}
pub fn read(snapshot: &Snapshot) -> Result<Vec<Entry>> {
    let db = Database::read(snapshot)?;
    ensure!(
        db.comparator == "leveldb.BytewiseComparator",
        "unexpected localStorage comparator"
    );
    let mut out = Vec::new();
    for (key, value) in db.entries {
        if key.first() != Some(&b'_') {
            continue;
        }
        let split = key[1..]
            .iter()
            .position(|b| *b == 0)
            .context("localStorage key has no origin separator")?
            + 1;
        let origin = String::from_utf8(key[1..split].to_vec())?;
        let key = string(&key[split + 1..])?;
        let value = string(&value)?;
        let json = serde_json::from_str(&value).ok();
        out.push(Entry {
            origin,
            key,
            value,
            json,
        });
    }
    Ok(out)
}
fn string(bytes: &[u8]) -> Result<String> {
    match bytes.split_first() {
        Some((0, rest)) => utf16(rest, false),
        Some((1, rest)) => Ok(rest.iter().map(|b| char::from(*b)).collect()),
        _ => bail!("unknown localStorage string encoding"),
    }
}
