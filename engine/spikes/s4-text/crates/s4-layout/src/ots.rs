//! The table-level effect of the OpenType Sanitizer on `FontFace` data.
//! Blink runs every web font through OTS (`web_font_decoder.cc`): known
//! tables are sanitized and kept, the pass-through list (color, variations,
//! OpenType layout) is kept as is, and every other table is dropped. On
//! macOS this removes AAT tables (`morx`, `kerx`, `trak`, ...) and Apple
//! `kern` version 1, so a system font registered from bytes shapes without
//! them. Today's variable-font path registers system font files this way.
//! This models table selection only; OTS's in-table repairs do not change
//! advances or shaping for well-formed fonts.

use read_fonts::{FileRef, TableProvider};

const KEPT: &[&[u8; 4]] = &[
    // Sanitized by OTS.
    b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post", b"loca", b"glyf", b"CFF ", b"cvt ",
    b"fpgm", b"prep", b"gasp", b"hdmx", b"kern", b"LTSH", b"VDMX", b"vhea", b"vmtx", b"VORG", b"MATH", b"Silf",
    b"Sill", b"Glat", b"Gloc", b"Feat",
    // Blink's pass-through list.
    b"CBDT", b"CBLC", b"COLR", b"CPAL", b"CFF2", b"sbix", b"STAT", b"avar", b"BASE", b"cvar", b"fvar", b"gvar",
    b"HVAR", b"MVAR", b"VVAR", b"GDEF", b"GPOS", b"GSUB",
];

/// Rebuilds face `index` of `data` with the tables OTS would keep.
pub fn sanitize(data: &[u8], index: u32) -> Option<Vec<u8>> {
    let file = FileRef::new(data).ok()?;
    let font = file.fonts().nth(index as usize)?.ok()?;
    let mut tables: Vec<(u32, Vec<u8>)> = Vec::new();
    for record in font.table_directory().table_records() {
        let tag = record.tag();
        let raw = tag.to_be_bytes();
        if !KEPT.iter().any(|t| **t == raw) {
            continue;
        }
        let Some(bytes) = font.table_data(tag) else { continue };
        let bytes = bytes.as_bytes();
        // OTS keeps only Microsoft `kern` (version 0).
        if &raw == b"kern" && (bytes.len() < 2 || u16::from_be_bytes([bytes[0], bytes[1]]) != 0) {
            continue;
        }
        tables.push((u32::from_be_bytes(raw), bytes.to_vec()));
    }
    let _ = font.head().ok()?;
    Some(build_sfnt(&mut tables))
}

/// Assembles an sfnt from (tag, data) pairs.
pub fn build_sfnt(tables: &mut [(u32, Vec<u8>)]) -> Vec<u8> {
    tables.sort_by_key(|t| t.0);
    let n = tables.len() as u16;
    let mut entry_selector = 0u16;
    while (1u32 << (entry_selector + 1)) <= n as u32 {
        entry_selector += 1;
    }
    let search_range = (1u16 << entry_selector) * 16;
    let range_shift = n * 16 - search_range;
    let is_cff = tables.iter().any(|t| t.0 == u32::from_be_bytes(*b"CFF ") || t.0 == u32::from_be_bytes(*b"CFF2"));
    let mut out = Vec::new();
    out.extend_from_slice(&(if is_cff { 0x4F54_544Fu32 } else { 0x0001_0000u32 }).to_be_bytes());
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&entry_selector.to_be_bytes());
    out.extend_from_slice(&range_shift.to_be_bytes());
    let mut offset = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, data) in tables.iter() {
        out.extend_from_slice(&tag.to_be_bytes());
        out.extend_from_slice(&checksum(data).to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        body.extend_from_slice(data);
        let pad = (4 - data.len() % 4) % 4;
        body.extend(std::iter::repeat_n(0u8, pad));
        offset += data.len() + pad;
    }
    out.extend_from_slice(&body);
    out
}

fn checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    for chunk in data.chunks(4) {
        let mut b = [0u8; 4];
        b[..chunk.len()].copy_from_slice(chunk);
        sum = sum.wrapping_add(u32::from_be_bytes(b));
    }
    sum
}
