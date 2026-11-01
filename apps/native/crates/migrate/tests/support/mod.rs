#![allow(dead_code)]
use pm_native_migrate::{
    Result,
    snapshot::{BlobCopy, Snapshot},
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub fn fixture() -> PathBuf {
    // Independent of the directory cargo runs from (file!() is relative to
    // whichever workspace root invoked rustc).
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/electron")
}
pub fn fixture_snapshot(temp: &Path) -> Snapshot {
    Snapshot::capture(
        &fixture().join("indexeddb"),
        Some(&fixture().join("blobs")),
        &temp.join("snapshot"),
        BlobCopy::All,
    )
    .unwrap()
}
pub fn var(mut n: u64) -> Vec<u8> {
    let mut o = Vec::new();
    loop {
        let b = (n & 127) as u8;
        n >>= 7;
        o.push(b | if n > 0 { 128 } else { 0 });
        if n == 0 {
            return o;
        }
    }
}
pub fn data(b: &[u8]) -> Vec<u8> {
    let mut o = var(b.len() as u64);
    o.extend_from_slice(b);
    o
}
pub fn string_be(s: &str) -> Vec<u8> {
    let mut o = var(s.encode_utf16().count() as u64);
    for c in s.encode_utf16() {
        o.extend_from_slice(&c.to_be_bytes());
    }
    o
}
pub fn mask(n: u32) -> u32 {
    n.rotate_right(15).wrapping_add(0xa282ead8)
}
pub fn append_log(path: &Path, payload: &[u8]) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    let mut pos = file.metadata().unwrap().len() as usize;
    let mut at = 0;
    let mut first = true;
    loop {
        let rem = 32768 - pos % 32768;
        if rem < 7 {
            file.write_all(&vec![0; rem]).unwrap();
            pos += rem;
        }
        let n = (32768 - pos % 32768 - 7).min(payload.len() - at);
        let last = at + n == payload.len();
        let kind = match (first, last) {
            (true, true) => 1,
            (true, false) => 2,
            (false, false) => 3,
            (false, true) => 4,
        };
        let part = &payload[at..at + n];
        let crc = mask(crc32c::crc32c_append(crc32c::crc32c(&[kind]), part));
        file.write_all(&crc.to_le_bytes()).unwrap();
        file.write_all(&(n as u16).to_le_bytes()).unwrap();
        file.write_all(&[kind]).unwrap();
        file.write_all(part).unwrap();
        pos += 7 + n;
        at += n;
        first = false;
        if last {
            break;
        }
    }
    file.sync_all().unwrap();
}
pub fn batch(seq: u64, entries: &[(Vec<u8>, Option<Vec<u8>>)]) -> Vec<u8> {
    let mut o = seq.to_le_bytes().to_vec();
    o.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (k, v) in entries {
        o.push(u8::from(v.is_some()));
        o.extend(data(k));
        if let Some(v) = v {
            o.extend(data(v));
        }
    }
    o
}
pub fn database(dir: &Path, comparator: &str, entries: &[(Vec<u8>, Option<Vec<u8>>)]) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("CURRENT"), b"MANIFEST-000001\n").unwrap();
    let mut manifest = vec![1];
    manifest.extend(data(comparator.as_bytes()));
    manifest.extend([2, 3, 3, 4, 4, 0]);
    append_log(&dir.join("MANIFEST-000001"), &manifest);
    append_log(&dir.join("000003.log"), &batch(1, entries));
}
pub fn capture(dir: &Path, out: &Path) -> Snapshot {
    Snapshot::capture(dir, None, out, BlobCopy::All).unwrap()
}
fn block(entries: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut prev = Vec::new();
    for (k, v) in entries {
        let shared = k.iter().zip(&prev).take_while(|(a, b)| a == b).count();
        out.extend(var(shared as u64));
        out.extend(var((k.len() - shared) as u64));
        out.extend(var(v.len() as u64));
        out.extend_from_slice(&k[shared..]);
        out.extend_from_slice(v);
        prev = k.clone();
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out
}
fn add_block(out: &mut Vec<u8>, bytes: &[u8], compress: bool) -> Vec<u8> {
    let raw = if compress {
        snap::raw::Encoder::new().compress_vec(bytes).unwrap()
    } else {
        bytes.to_vec()
    };
    let mut handle = var(out.len() as u64);
    handle.extend(var(raw.len() as u64));
    out.extend_from_slice(&raw);
    out.push(u8::from(compress));
    let crc = mask(crc32c::crc32c(&out[out.len() - raw.len() - 1..]));
    out.extend_from_slice(&crc.to_le_bytes());
    handle
}
pub fn table(
    dir: &Path,
    number: u64,
    entries: &[(Vec<u8>, u64, u8, Vec<u8>)],
    compress: bool,
) -> u64 {
    let mut out = Vec::new();
    let mut index = Vec::new();
    for chunk in entries.chunks(2) {
        let rows: Vec<_> = chunk
            .iter()
            .map(|(k, s, t, v)| {
                let mut k = k.clone();
                k.extend_from_slice(&((s << 8) | u64::from(*t)).to_le_bytes());
                (k, v.clone())
            })
            .collect();
        let h = add_block(&mut out, &block(&rows), compress);
        index.push((rows.last().unwrap().0.clone(), h));
    }
    let meta = add_block(&mut out, &block(&[]), false);
    let idx = add_block(&mut out, &block(&index), false);
    let mut footer = meta;
    footer.extend(idx);
    footer.resize(40, 0);
    footer.extend_from_slice(&0xdb4775248b80fb57u64.to_le_bytes());
    out.extend(footer);
    fs::write(dir.join(format!("{number:06}.ldb")), &out).unwrap();
    out.len() as u64
}
pub fn add_table(dir: &Path, n: u64, size: u64) {
    let mut e = vec![7, 0];
    e.extend(var(n));
    e.extend(var(size));
    e.extend(data(b"first"));
    e.extend(data(b"last"));
    append_log(&dir.join("MANIFEST-000001"), &e);
}
pub fn assert_error<T>(result: Result<T>, text: &str) {
    let err = result.err().expect("expected failure");
    assert!(format!("{err:#}").contains(text), "{err:#}");
}
