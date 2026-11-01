use crate::{
    Result,
    leveldb::Database,
    snapshot::{Snapshot, open_regular},
    structured_clone,
    wire::{Cursor, MAX_VALUE, utf16},
};
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, io::Read, path::PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExternalObject {
    pub number: u64,
    pub mime: String,
    pub size: u64,
    pub file_name: Option<String>,
    pub last_modified_micros: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub database: u64,
    pub store: u64,
    pub store_name: String,
    pub key: String,
    pub record_version: u64,
    pub value: Value,
    pub external: Vec<ExternalObject>,
    pub payload_index: usize,
    pub poster: bool,
}
impl Record {
    pub fn payload(&self) -> &ExternalObject {
        &self.external[self.payload_index]
    }
    pub fn payload_path(&self, snapshot: &Snapshot) -> Result<Option<PathBuf>> {
        snapshot.blob_path(self.database, self.payload().number)
    }
    pub fn revision(&self) -> Option<&str> {
        self.value.get("revision").and_then(Value::as_str)
    }
    pub fn fingerprint(&self) -> Option<&str> {
        self.value.get("fingerprint").and_then(Value::as_str)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ObjectStoreMetadata {
    pub database: u64,
    pub id: u64,
    pub name: Option<String>,
    pub key_path: Option<Value>,
    pub auto_increment: Option<bool>,
    /// Raw values for every metadata field, including obsolete/unknown ones.
    pub raw: BTreeMap<u8, String>,
}
#[derive(Debug)]
pub struct MediaDatabase {
    pub schema_version: u64,
    pub database_versions: BTreeMap<u64, u64>,
    pub object_stores: Vec<ObjectStoreMetadata>,
    pub records: Vec<Record>,
    pub recovered_tail_bytes: u64,
}

pub type KeyPrefix = (u64, u64, u64);
pub fn prefix(bytes: &[u8]) -> Result<(KeyPrefix, &[u8])> {
    let mut c = Cursor(bytes);
    let h = c.byte()?;
    let mut read = |n: usize| -> Result<u64> {
        let mut b = [0; 8];
        b[..n].copy_from_slice(c.take(n)?);
        Ok(u64::from_le_bytes(b))
    };
    let db = read(((h >> 5) & 7) as usize + 1)?;
    let store = read(((h >> 2) & 7) as usize + 1)?;
    let index = read((h & 3) as usize + 1)?;
    Ok(((db, store, index), c.0))
}
fn integer(bytes: &[u8]) -> Result<u64> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= 8,
        "invalid metadata integer"
    );
    let mut b = [0; 8];
    b[..bytes.len()].copy_from_slice(bytes);
    Ok(u64::from_le_bytes(b))
}

pub fn read(snapshot: &Snapshot) -> Result<MediaDatabase> {
    let db = Database::read(snapshot)?;
    ensure!(db.comparator == "idb_cmp1", "IndexedDB requires idb_cmp1");
    let mut names = BTreeMap::new();
    let mut stores = BTreeMap::new();
    let mut versions = BTreeMap::new();
    let mut schema = None;
    let mut store_metadata: BTreeMap<(u64, u64), ObjectStoreMetadata> = BTreeMap::new();
    for (key, value) in &db.entries {
        let ((database, store, index), suffix) = prefix(key)?;
        if (database, store, index) == (0, 0, 0) && suffix.first() == Some(&50) {
            ensure!(
                suffix.get(1) == Some(&0),
                "unfinished IndexedDB scope; capture after Electron transaction recovery"
            );
        }
        if (database, store, index) == (0, 0, 0) && suffix == [0] {
            schema = Some(integer(value)?);
        }
        if (database, store, index) == (0, 0, 0) && suffix.first() == Some(&201) {
            let mut c = Cursor(&suffix[1..]);
            let _origin = c.utf16()?;
            let name = c.utf16()?;
            c.finish()?;
            let id = integer(value)?;
            names.insert(id, name);
        }
        if database > 0 && store == 0 && index == 0 {
            let mut c = Cursor(suffix);
            match c.byte()? {
                1 => {
                    names.insert(database, utf16(value, true)?);
                }
                4 => {
                    let mut v = Cursor(value);
                    versions.insert(database, v.var()?);
                    v.finish()?;
                }
                50 => {
                    let s = c.var()?;
                    let kind = c.byte()?;
                    c.finish()?;
                    let metadata = store_metadata.entry((database, s)).or_insert_with(|| {
                        ObjectStoreMetadata {
                            database,
                            id: s,
                            ..ObjectStoreMetadata::default()
                        }
                    });
                    metadata.raw.insert(kind, hex::encode(value));
                    match kind {
                        0 => {
                            let name = utf16(value, true)?;
                            stores.insert((database, s), name.clone());
                            metadata.name = Some(name);
                        }
                        1 => metadata.key_path = Some(key_path(value)?),
                        2 => {
                            ensure!(value.len() == 1, "invalid auto-increment flag");
                            metadata.auto_increment = Some(value[0] != 0);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
    let schema = schema.context("missing IndexedDB schema")?;
    ensure!(
        (4..=5).contains(&schema),
        "unsupported IndexedDB schema; logical export required"
    );
    let selected: Vec<u64> = names
        .iter()
        .filter_map(|(id, name)| (name == "powermove-media").then_some(*id))
        .collect();
    // An absent database is different from a corrupt one; an empty fresh profile
    // is valid, but every record in a selected database must be decodable.
    let mut records = Vec::new();
    for (key, value) in &db.entries {
        let ((database, store, index), suffix) = prefix(key)?;
        if !selected.contains(&database) || store == 0 || index != 1 {
            continue;
        }
        let mut k = Cursor(suffix);
        ensure!(k.byte()? == 1, "non-string media primary key");
        let key = k.utf16()?;
        k.finish()?;
        let mut v = Cursor(value);
        let record_version = v.var()?;
        let mut blob_key = encode_prefix(database, store, 3);
        blob_key.extend_from_slice(suffix);
        let external = match db.entries.get(&blob_key) {
            Some(b) => external_objects(b)?,
            None => Vec::new(),
        };
        let unwrapped = unwrap_value(v.0, &external, snapshot, database, 0)?;
        let value = structured_clone::decode(&unwrapped)?;
        ensure!(
            value.get("id").and_then(Value::as_str) == Some(&key),
            "record id differs from primary key"
        );
        let blob = value.get("blob").context("media record has no Blob")?;
        let payload_index = usize::try_from(
            blob.get("$blob")
                .and_then(Value::as_u64)
                .context("media payload is not a Blob/File reference")?,
        )?;
        let payload = external
            .get(payload_index)
            .context("Blob index outside external object table")?;
        ensure!(
            blob.get("$file").and_then(Value::as_bool) == Some(payload.file_name.is_some()),
            "Blob/File descriptor kind mismatch"
        );
        if let Some(size) = value.get("size").and_then(Value::as_u64) {
            ensure!(size == payload.size, "Blob size differs from record");
        }
        records.push(Record {
            database,
            store,
            store_name: stores
                .get(&(database, store))
                .context("missing object-store metadata")?
                .clone(),
            poster: key.ends_with(":poster"),
            key,
            record_version,
            value,
            external,
            payload_index,
        });
    }
    Ok(MediaDatabase {
        schema_version: schema,
        database_versions: versions
            .into_iter()
            .filter(|(id, _)| selected.contains(id))
            .collect(),
        object_stores: store_metadata
            .into_values()
            .filter(|s| selected.contains(&s.database))
            .collect(),
        records,
        recovered_tail_bytes: db.recovered_tail_bytes,
    })
}
fn external_objects(bytes: &[u8]) -> Result<Vec<ExternalObject>> {
    let mut c = Cursor(bytes);
    let mut out = Vec::new();
    while !c.0.is_empty() {
        let kind = c.byte()?;
        ensure!(kind <= 1, "unsupported external object kind");
        let number = c.var()?;
        ensure!(number > 0, "invalid blob number");
        let mime = c.utf16()?;
        let size = c.var()?;
        let (file_name, last_modified_micros) = if kind == 1 {
            (Some(c.utf16()?), Some(c.var()?))
        } else {
            (None, None)
        };
        out.push(ExternalObject {
            number,
            mime,
            size,
            file_name,
            last_modified_micros,
        });
        ensure!(out.len() <= 100_000, "too many external objects");
    }
    Ok(out)
}
fn unwrap_value(
    bytes: &[u8],
    external: &[ExternalObject],
    snapshot: &Snapshot,
    database: u64,
    depth: usize,
) -> Result<Vec<u8>> {
    ensure!(depth < 4, "nested structured-clone wrapper exceeds bound");
    if bytes.starts_with(&[255, 17, 1]) {
        let mut c = Cursor(&bytes[3..]);
        let size = c.len()?;
        let index = usize::try_from(c.var()?)?;
        c.finish()?;
        ensure!(
            index + 1 == external.len(),
            "wrapper is not the last external object"
        );
        let b = external.get(index).context("invalid wrapper Blob index")?;
        ensure!(b.size == size as u64, "wrapper Blob size mismatch");
        let path = snapshot
            .blob_path(database, b.number)?
            .context("missing wrapper Blob in snapshot")?;
        let mut content = Vec::new();
        open_regular(&path)?
            .take(MAX_VALUE as u64 + 1)
            .read_to_end(&mut content)?;
        ensure!(content.len() == size, "wrapper Blob length mismatch");
        return unwrap_value(&content, external, snapshot, database, depth + 1);
    }
    if bytes.starts_with(&[255, 17, 2]) {
        ensure!(
            snap::raw::decompress_len(&bytes[3..])? <= MAX_VALUE,
            "structured-clone decompression exceeds bound"
        );
        let content = snap::raw::Decoder::new().decompress_vec(&bytes[3..])?;
        return unwrap_value(&content, external, snapshot, database, depth + 1);
    }
    ensure!(bytes.len() <= MAX_VALUE, "structured clone exceeds bound");
    Ok(bytes.to_vec())
}

pub fn encode_prefix(db: u64, store: u64, index: u64) -> Vec<u8> {
    fn width(n: u64) -> usize {
        ((64 - n.leading_zeros()).max(1) as usize).div_ceil(8)
    }
    let (d, s, i) = (width(db), width(store), width(index));
    assert!(i <= 4);
    let mut out = vec![(((d - 1) << 5) | ((s - 1) << 2) | (i - 1)) as u8];
    out.extend_from_slice(&db.to_le_bytes()[..d]);
    out.extend_from_slice(&store.to_le_bytes()[..s]);
    out.extend_from_slice(&index.to_le_bytes()[..i]);
    out
}

/// Validate the *syntax* of the legacy sampled identity, without pretending it
/// is a full-file hash. Matching candidates are checked by `migration`.
pub fn v2_fingerprint(value: &str) -> Result<(u64, &str)> {
    let mut p = value.split(':');
    ensure!(p.next() == Some("v2"), "unsupported legacy fingerprint");
    let size = p
        .next()
        .context("missing fingerprint size")?
        .parse::<u64>()?;
    let hash = p.next().context("missing fingerprint hash")?;
    ensure!(
        p.next().is_none()
            && hash.len() == 64
            && hash
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "invalid sampled fingerprint"
    );
    Ok((size, hash))
}

fn key_path(bytes: &[u8]) -> Result<Value> {
    if bytes.len() < 3 || bytes[..2] != [0, 0] {
        return Ok(Value::String(utf16(bytes, true)?));
    }
    let mut c = Cursor(&bytes[2..]);
    let value = match c.byte()? {
        0 => Value::Null,
        1 => Value::String(c.utf16()?),
        2 => {
            let n = c.len()?;
            ensure!(n <= 100_000, "key-path array too long");
            let mut a = Vec::new();
            for _ in 0..n {
                a.push(Value::String(c.utf16()?));
            }
            Value::Array(a)
        }
        _ => anyhow::bail!("unknown IndexedDB key-path type"),
    };
    c.finish()?;
    Ok(value)
}
