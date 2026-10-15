//! Fonts for the pm-wasm runs: registry answers from the native registry
//! (CoreText), as main serves them to the renderer (`engine-fonts:answer`),
//! and face bytes, either whole faces (outlines, so ink is answered) or the
//! metrics-and-shaping subsets the app serves (`engine-fonts:subset`).

use std::collections::BTreeMap;
use std::sync::Arc;

use pm_text::answers::{Answers, Query};
use pm_text::registry::CoreTextRegistry;

/// One face of a font file as a standalone font: the face's own table
/// directory with its tables copied. A plain sfnt is returned as is (face 0).
/// pm-wasm reads the faces it is given at index 0, so faces of collections
/// (`.ttc`) must be extracted.
pub fn extract_face(data: &[u8], index: u32) -> Result<Vec<u8>, String> {
    let u32_at = |at: usize| -> Result<u32, String> {
        data.get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or_else(|| format!("truncated font at {at}"))
    };
    let u16_at = |at: usize| -> Result<u16, String> {
        data.get(at..at + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
            .ok_or_else(|| format!("truncated font at {at}"))
    };
    if data.get(0..4) != Some(b"ttcf") {
        if index != 0 {
            return Err(format!("face {index} of a single-face font"));
        }
        return Ok(data.to_vec());
    }
    let count = u32_at(8)?;
    if index >= count {
        return Err(format!("face {index} of a collection of {count}"));
    }
    let start = u32_at(12 + 4 * index as usize)? as usize;
    let version = u32_at(start)?;
    let tables = u16_at(start + 4)? as usize;
    let mut records: Vec<([u8; 4], u32, usize, usize)> = Vec::with_capacity(tables);
    for i in 0..tables {
        let at = start + 12 + 16 * i;
        let tag: [u8; 4] = data
            .get(at..at + 4)
            .ok_or("truncated table directory")?
            .try_into()
            .map_err(|_| "tag")?;
        records.push((
            tag,
            u32_at(at + 4)?,
            u32_at(at + 8)? as usize,
            u32_at(at + 12)? as usize,
        ));
    }
    // OpenType's binary search header for the directory.
    let mut entry_selector = 0u16;
    while (2usize << entry_selector) <= tables {
        entry_selector += 1;
    }
    let search_range = (1u16 << entry_selector) * 16;
    let range_shift = (tables as u16) * 16 - search_range;
    let mut out = Vec::new();
    out.extend_from_slice(&version.to_be_bytes());
    out.extend_from_slice(&(tables as u16).to_be_bytes());
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&entry_selector.to_be_bytes());
    out.extend_from_slice(&range_shift.to_be_bytes());
    let mut offset = 12 + 16 * tables;
    let mut body = Vec::new();
    for (tag, checksum, at, len) in &records {
        let bytes = data
            .get(*at..*at + *len)
            .ok_or_else(|| format!("table {} out of bounds", String::from_utf8_lossy(tag)))?;
        out.extend_from_slice(tag);
        out.extend_from_slice(&checksum.to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(*len as u32).to_be_bytes());
        body.extend_from_slice(bytes);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        offset = 12 + 16 * tables + body.len();
    }
    out.extend_from_slice(&body);
    Ok(out)
}

/// The registry and what a project's queries needed from it.
pub struct FontService {
    pub registry: CoreTextRegistry,
    pub subsets: bool,
    /// Bytes per (blob, face index), as pm-wasm receives them.
    cache: BTreeMap<(String, u32), Arc<Vec<u8>>>,
}

impl FontService {
    pub fn new(subsets: bool) -> FontService {
        FontService {
            registry: CoreTextRegistry::new(),
            subsets,
            cache: BTreeMap::new(),
        }
    }

    pub fn answer(&mut self, queries: &[Query]) -> Result<Answers, String> {
        self.registry.answer(queries).map_err(|e| e.to_string())
    }

    pub fn face(&mut self, blob: &str, index: u32) -> Result<Arc<Vec<u8>>, String> {
        let key = (blob.to_owned(), index);
        if let Some(b) = self.cache.get(&key) {
            return Ok(b.clone());
        }
        let bytes = if self.subsets {
            self.registry
                .subset(blob, index)
                .map_err(|e| e.to_string())?
        } else {
            let data = self
                .registry
                .blob(blob)
                .ok_or_else(|| format!("unknown font blob {blob}"))?;
            Arc::new(extract_face(data.as_slice(), index)?)
        };
        self.cache.insert(key, bytes.clone());
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(tag: &[u8; 4], body: &[u8]) -> ([u8; 4], Vec<u8>) {
        (*tag, body.to_vec())
    }

    /// A collection of two faces sharing one table.
    fn collection() -> Vec<u8> {
        let shared = table(b"cmap", b"shared!");
        let faces = [
            vec![table(b"head", b"face0"), shared.clone()],
            vec![table(b"head", b"face-one"), shared],
        ];
        let mut out = b"ttcf".to_vec();
        out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        out.extend_from_slice(&2u32.to_be_bytes());
        let dir0 = 12 + 8;
        let dir1 = dir0 + 12 + 16 * 2;
        out.extend_from_slice(&(dir0 as u32).to_be_bytes());
        out.extend_from_slice(&(dir1 as u32).to_be_bytes());
        let mut data_at = dir1 + 12 + 16 * 2;
        let mut data = Vec::new();
        let mut dirs = Vec::new();
        let mut shared_at = None;
        for face in &faces {
            let mut d = Vec::new();
            d.extend_from_slice(&0x0001_0000u32.to_be_bytes());
            d.extend_from_slice(&2u16.to_be_bytes());
            d.extend_from_slice(&[0; 6]);
            for (tag, body) in face {
                let at = match shared_at {
                    Some(at) if tag == b"cmap" => at,
                    _ => {
                        let at = data_at;
                        data.extend_from_slice(body);
                        data_at += body.len();
                        if tag == b"cmap" {
                            shared_at = Some(at);
                        }
                        at
                    }
                };
                d.extend_from_slice(tag);
                d.extend_from_slice(&7u32.to_be_bytes());
                d.extend_from_slice(&(at as u32).to_be_bytes());
                d.extend_from_slice(&(body.len() as u32).to_be_bytes());
            }
            dirs.push(d);
        }
        for d in dirs {
            out.extend_from_slice(&d);
        }
        out.extend_from_slice(&data);
        out
    }

    fn tables(font: &[u8]) -> Vec<(String, Vec<u8>)> {
        let n = u16::from_be_bytes([font[4], font[5]]) as usize;
        (0..n)
            .map(|i| {
                let r = &font[12 + 16 * i..];
                let at = u32::from_be_bytes([r[8], r[9], r[10], r[11]]) as usize;
                let len = u32::from_be_bytes([r[12], r[13], r[14], r[15]]) as usize;
                assert_eq!(at % 4, 0, "tables are 4-byte aligned");
                (
                    String::from_utf8_lossy(&r[..4]).into_owned(),
                    font[at..at + len].to_vec(),
                )
            })
            .collect()
    }

    #[test]
    fn extracts_each_face_of_a_collection() {
        let c = collection();
        let f1 = extract_face(&c, 1).unwrap();
        assert_eq!(&f1[..4], &0x0001_0000u32.to_be_bytes());
        assert_eq!(
            tables(&f1),
            [
                ("head".to_owned(), b"face-one".to_vec()),
                ("cmap".to_owned(), b"shared!".to_vec())
            ]
        );
        let f0 = extract_face(&c, 0).unwrap();
        assert_eq!(tables(&f0)[0], ("head".to_owned(), b"face0".to_vec()));
        assert!(extract_face(&c, 2).is_err());
        // A plain font is its own face 0.
        assert_eq!(extract_face(&f1, 0).unwrap(), f1);
        assert!(extract_face(&f1, 1).is_err());
    }
}
