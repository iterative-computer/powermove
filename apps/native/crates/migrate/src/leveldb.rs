//! A full physical scan, rather than a LevelDB sorted lookup. `idb_cmp1` is
//! deliberately NOT replaced with a byte comparator. The manifest selects the
//! live SST set; sequence numbers and tombstones select the latest exact key.
//! Chromium's canonical media string keys have a unique byte encoding. Index
//! entries are retained opaquely and never compared or used to seek records.
use crate::{
    Result,
    snapshot::Snapshot,
    wire::{Cursor, MAX_VALUE, unmask},
};
use anyhow::{Context, bail, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

const BLOCK: usize = 32768;
const MAGIC: u64 = 0xdb4775248b80fb57;
const MAX_MAP: usize = 256 * 1024 * 1024;

#[derive(Debug)]
pub struct Database {
    pub comparator: String,
    pub entries: BTreeMap<Vec<u8>, Vec<u8>>,
    pub recovered_tail_bytes: u64,
}

#[derive(Default)]
struct Manifest {
    comparator: Option<String>,
    log: u64,
    previous_log: u64,
    files: BTreeMap<u64, u64>,
}
impl Manifest {
    fn edit(&mut self, bytes: &[u8]) -> Result<()> {
        let mut c = Cursor(bytes);
        while !c.0.is_empty() {
            match c.var()? {
                1 => {
                    self.comparator = Some(
                        String::from_utf8(c.bytes()?.to_vec())
                            .context("invalid comparator name")?,
                    )
                }
                2 => self.log = c.var()?,
                3 | 4 => {
                    c.var()?;
                }
                5 => {
                    c.var()?;
                    c.bytes()?;
                }
                6 => {
                    c.var()?;
                    let id = c.var()?;
                    self.files.remove(&id);
                }
                7 => {
                    c.var()?;
                    let id = c.var()?;
                    let size = c.var()?;
                    c.bytes()?;
                    c.bytes()?;
                    self.files.insert(id, size);
                }
                9 => self.previous_log = c.var()?,
                _ => bail!("unsupported LevelDB manifest tag"),
            }
        }
        Ok(())
    }
}

impl Database {
    pub fn read(snapshot: &Snapshot) -> Result<Self> {
        snapshot.verify_leveldb()?;
        let dir = snapshot.leveldb();
        let current = std::fs::read(dir.join("CURRENT"))?;
        ensure!(
            current.len() < 100 && current.ends_with(b"\n"),
            "invalid CURRENT"
        );
        let name = std::str::from_utf8(&current[..current.len() - 1])?;
        ensure!(
            name.starts_with("MANIFEST-") && name[9..].bytes().all(|b| b.is_ascii_digit()),
            "invalid manifest name"
        );
        let mut manifest = Manifest::default();
        let tail = log_records(&dir.join(name), |b| manifest.edit(b))?;
        ensure!(tail == 0, "incomplete manifest snapshot");
        let comparator = manifest.comparator.context("missing comparator")?;
        ensure!(
            matches!(
                comparator.as_str(),
                "idb_cmp1" | "leveldb.BytewiseComparator"
            ),
            "unsupported comparator"
        );
        let mut latest = Latest::default();
        for (number, size) in manifest.files {
            let ldb = dir.join(format!("{number:06}.ldb"));
            let file = if ldb.exists() {
                ldb
            } else {
                dir.join(format!("{number:06}.sst"))
            };
            ensure!(
                std::fs::metadata(&file)?.len() == size,
                "table size differs from manifest"
            );
            scan_table(&file, |key, value| {
                ensure!(key.len() >= 8, "short internal key");
                let (key, trailer) = key.split_at(key.len() - 8);
                let tag = u64::from_le_bytes(trailer.try_into()?);
                latest.put(key, tag >> 8, tag as u8, value)
            })?;
        }
        let mut logs = BTreeSet::new();
        for e in std::fs::read_dir(&dir)? {
            let name = e?.file_name();
            if let Some(n) = name
                .to_str()
                .and_then(|s| s.strip_suffix(".log"))
                .and_then(|s| s.parse::<u64>().ok())
                && (n >= manifest.log || n == manifest.previous_log)
            {
                logs.insert(n);
            }
        }
        let mut recovered_tail_bytes = 0;
        for n in logs {
            recovered_tail_bytes += log_records(&dir.join(format!("{n:06}.log")), |record| {
                ensure!(record.len() >= 12, "short write batch");
                let seq = u64::from_le_bytes(record[..8].try_into()?);
                let count = u32::from_le_bytes(record[8..12].try_into()?);
                let mut c = Cursor(&record[12..]);
                for i in 0..count {
                    let kind = c.byte()?;
                    let key = c.bytes()?;
                    let value = match kind {
                        0 => &[][..],
                        1 => c.bytes()?,
                        _ => bail!("unsupported write batch operation"),
                    };
                    latest.put(
                        key,
                        seq.checked_add(u64::from(i)).context("sequence overflow")?,
                        kind,
                        value,
                    )?;
                }
                c.finish()
            })?;
        }
        Ok(Self {
            comparator,
            entries: latest
                .values
                .into_iter()
                .filter_map(|(k, (_, kind, v))| (kind == 1).then_some((k, v)))
                .collect(),
            recovered_tail_bytes,
        })
    }
}

#[derive(Default)]
struct Latest {
    values: BTreeMap<Vec<u8>, (u64, u8, Vec<u8>)>,
    bytes: usize,
}
impl Latest {
    fn put(&mut self, key: &[u8], sequence: u64, kind: u8, value: &[u8]) -> Result<()> {
        ensure!(
            kind <= 1 && sequence < (1u64 << 56),
            "invalid internal key tag"
        );
        if let Some((old, old_kind, old_value)) = self.values.get(key) {
            if *old > sequence {
                return Ok(());
            }
            if *old == sequence {
                ensure!(
                    *old_kind == kind && old_value == value,
                    "conflicting sequence numbers"
                );
                return Ok(());
            }
            self.bytes -= key.len() + old_value.len() + 64;
        }
        self.bytes = self
            .bytes
            .checked_add(key.len() + value.len() + 64)
            .context("metadata size overflow")?;
        ensure!(
            self.bytes <= MAX_MAP,
            "LevelDB metadata exceeds 256 MiB bound; logical export required"
        );
        self.values
            .insert(key.to_vec(), (sequence, kind, value.to_vec()));
        Ok(())
    }
}

// LevelDB's 32 KiB physical journals. A final torn WAL append isn't a committed
// batch; earlier complete records survive. A CRC error is never ignored.
fn log_records(path: &Path, mut visit: impl FnMut(&[u8]) -> Result<()>) -> Result<u64> {
    let mut file = File::open(path)?;
    let mut block = [0u8; BLOCK];
    let mut pending = Vec::new();
    let mut fragmented = false;
    loop {
        let mut size = 0;
        while size < BLOCK {
            let n = file.read(&mut block[size..])?;
            if n == 0 {
                break;
            }
            size += n;
        }
        if size == 0 {
            return Ok(pending.len() as u64);
        }
        let mut offset = 0;
        while offset + 7 <= size {
            let h = &block[offset..offset + 7];
            let crc = u32::from_le_bytes(h[..4].try_into()?);
            let len = u16::from_le_bytes(h[4..6].try_into()?) as usize;
            let kind = h[6];
            if crc == 0 && len == 0 && kind == 0 {
                ensure!(
                    block[offset..size].iter().all(|v| *v == 0),
                    "invalid journal padding"
                );
                break;
            }
            if offset + 7 + len > size {
                ensure!(size < BLOCK, "record crosses journal block");
                return Ok((pending.len() + size - offset) as u64);
            }
            let payload = &block[offset + 7..offset + 7 + len];
            let actual = crc32c::crc32c_append(crc32c::crc32c(&[kind]), payload);
            ensure!(unmask(crc) == actual, "journal checksum mismatch");
            match kind {
                1 => {
                    ensure!(!fragmented, "unfinished fragmented record");
                    visit(payload)?;
                }
                2 => {
                    ensure!(!fragmented, "nested journal fragment");
                    pending.extend_from_slice(payload);
                    fragmented = true;
                }
                3 | 4 => {
                    ensure!(fragmented, "orphan journal fragment");
                    ensure!(
                        pending.len() + payload.len() <= MAX_VALUE,
                        "journal record exceeds metadata bound"
                    );
                    pending.extend_from_slice(payload);
                    if kind == 4 {
                        visit(&pending)?;
                        pending.clear();
                        fragmented = false;
                    }
                }
                _ => bail!("unsupported journal record type"),
            }
            offset += 7 + len;
        }
        if size < BLOCK {
            // A short nonzero header at EOF is a torn append. Full-block tails
            // shorter than a header are the prescribed zero-filled padding.
            return Ok((pending.len()
                + if block[offset..size].iter().any(|b| *b != 0) {
                    size - offset
                } else {
                    0
                }) as u64);
        }
        ensure!(
            block[offset..size].iter().all(|b| *b == 0),
            "invalid journal block tail"
        );
    }
}

fn handle(c: &mut Cursor<'_>) -> Result<(u64, u64)> {
    Ok((c.var()?, c.var()?))
}
fn table_block(file: &mut File, h: (u64, u64), size: u64) -> Result<Vec<u8>> {
    let (offset, len) = h;
    ensure!(
        len <= MAX_VALUE as u64
            && offset
                .checked_add(len)
                .and_then(|n| n.checked_add(5))
                .is_some_and(|n| n <= size - 48),
        "table block outside file or exceeds bound"
    );
    file.seek(SeekFrom::Start(offset))?;
    let mut raw = vec![0u8; len as usize + 5];
    file.read_exact(&mut raw)?;
    let crc = u32::from_le_bytes(raw[len as usize + 1..].try_into()?);
    ensure!(
        unmask(crc) == crc32c::crc32c(&raw[..len as usize + 1]),
        "table block checksum mismatch"
    );
    match raw[len as usize] {
        0 => {
            raw.truncate(len as usize);
            Ok(raw)
        }
        1 => {
            let compressed = &raw[..len as usize];
            ensure!(
                snap::raw::decompress_len(compressed)? <= MAX_VALUE,
                "Snappy block exceeds bound"
            );
            Ok(snap::raw::Decoder::new().decompress_vec(compressed)?)
        }
        _ => bail!("unsupported table compression"),
    }
}
fn block_entries(block: &[u8], mut visit: impl FnMut(&[u8], &[u8]) -> Result<()>) -> Result<()> {
    ensure!(block.len() >= 8, "short table block");
    let count = u32::from_le_bytes(block[block.len() - 4..].try_into()?) as usize;
    ensure!(
        count > 0 && count <= (block.len() - 4) / 4,
        "invalid restart count"
    );
    let end = block.len() - 4 - count * 4;
    let mut restarts = BTreeSet::new();
    let mut previous = None;
    for b in block[end..block.len() - 4].chunks_exact(4) {
        let at = u32::from_le_bytes(b.try_into()?) as usize;
        ensure!(
            at <= end && previous.is_none_or(|p| at > p),
            "invalid restart offset"
        );
        restarts.insert(at);
        previous = Some(at);
    }
    ensure!(restarts.contains(&0), "missing first restart");
    let mut c = Cursor(&block[..end]);
    let mut key = Vec::new();
    while !c.0.is_empty() {
        let at = end - c.0.len();
        let shared = c.len()?;
        let unshared = c.len()?;
        let value_len = c.len()?;
        ensure!(
            shared <= key.len() && shared + unshared <= MAX_VALUE,
            "invalid compressed key"
        );
        if restarts.remove(&at) {
            ensure!(shared == 0, "restart has a shared prefix");
        }
        key.truncate(shared);
        key.extend_from_slice(c.take(unshared)?);
        visit(&key, c.take(value_len)?)?;
    }
    restarts.remove(&end); // empty block's sole zero restart
    ensure!(restarts.is_empty(), "restart is not at an entry boundary");
    Ok(())
}
fn scan_table(path: &Path, mut visit: impl FnMut(&[u8], &[u8]) -> Result<()>) -> Result<()> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    ensure!(size >= 48, "short table");
    file.seek(SeekFrom::End(-48))?;
    let mut footer = [0; 48];
    file.read_exact(&mut footer)?;
    ensure!(
        u64::from_le_bytes(footer[40..].try_into()?) == MAGIC,
        "table magic mismatch"
    );
    let mut c = Cursor(&footer[..40]);
    let meta = handle(&mut c)?;
    let index = handle(&mut c)?;
    let metadata = table_block(&mut file, meta, size)?;
    block_entries(&metadata, |_, _| Ok(()))?;
    let index = table_block(&mut file, index, size)?;
    let mut seen = BTreeSet::new();
    block_entries(&index, |_, value| {
        let mut c = Cursor(value);
        let h = handle(&mut c)?;
        c.finish()?;
        ensure!(seen.insert(h.0), "duplicate table block");
        let data = table_block(&mut file, h, size)?;
        block_entries(&data, &mut visit)
    })
}
