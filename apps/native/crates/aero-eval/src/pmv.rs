//! Read the project document out of a `.pmv`: plain JSON, or a PMV4 container
//! (`PMV4\n`, document, media, index, 64-byte footer), as
//! `acceptance/data/fixtures/generate-portable.py` writes it. Read-only; the
//! kernel's reader stays the authority, this is only for grading.
use serde_json::Value;

const FOOTER: usize = 64;

pub fn read_document(bytes: &[u8]) -> Result<Value, String> {
    let body = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    if !body.starts_with(b"PMV4\n") {
        return serde_json::from_slice(body).map_err(|e| format!("project JSON: {e}"));
    }
    if bytes.len() < 5 + FOOTER {
        return Err("PMV4 file is too short".into());
    }
    let footer = &bytes[bytes.len() - FOOTER..];
    if &footer[..8] != b"PMV4END\n" {
        return Err("PMV4 footer magic missing".into());
    }
    let index_at = f64::from_le_bytes(footer[8..16].try_into().unwrap());
    let index_len = u32::from_le_bytes(footer[16..20].try_into().unwrap()) as usize;
    if !(index_at.is_finite() && index_at >= 5.0 && index_at.fract() == 0.0) {
        return Err("PMV4 index offset is invalid".into());
    }
    let index_at = index_at as usize;
    let index = bytes
        .get(index_at..index_at + index_len)
        .filter(|_| index_at + index_len <= bytes.len() - FOOTER)
        .ok_or("PMV4 index lies outside the file")?;
    if crate::sha256_hex(index) != hex(&footer[32..64]) {
        return Err("PMV4 index checksum mismatch".into());
    }
    let index: Value = serde_json::from_slice(index).map_err(|e| format!("PMV4 index: {e}"))?;
    let doc = &index["document"];
    let (Some(at), Some(len)) = (doc["offset"].as_u64(), doc["length"].as_u64()) else {
        return Err("PMV4 index has no document record".into());
    };
    let document = bytes
        .get(at as usize..(at + len) as usize)
        .filter(|_| ((at + len) as usize) <= index_at)
        .ok_or("PMV4 document lies outside the body")?;
    if doc["sha256"].as_str() != Some(crate::sha256_hex(document).as_str()) {
        return Err("PMV4 document checksum mismatch".into());
    }
    serde_json::from_slice(document).map_err(|e| format!("PMV4 document: {e}"))
}

/// The root composition of a document: PMV4 documents wrap it as `proj`.
pub fn project_of(document: &Value) -> &Value {
    match document.get("proj") {
        Some(p) if p.is_object() => p,
        _ => document,
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}
