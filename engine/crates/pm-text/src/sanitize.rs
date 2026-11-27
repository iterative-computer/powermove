//! The table-level effect of the OpenType Sanitizer on `FontFace` data.
//! Blink runs every web font through OTS (`web_font_decoder.cc`): known
//! tables are sanitized and kept, the pass-through list (color, variations,
//! OpenType layout) is kept as is, and every other table is dropped. On
//! macOS this removes AAT tables (`morx`, `kerx`, `trak`, ...) and Apple
//! `kern` version 1, so a system font registered from bytes shapes without
//! them. Today's variable-font path registers system font files this way.
//! This models table selection only; OTS's in-table repairs do not change
//! advances or shaping for well-formed fonts.

use crate::sfnt::build_sfnt;
use read_fonts::{FileRef, TableProvider};

const KEPT: &[&[u8; 4]] = &[
    // Sanitized by OTS.
    b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post", b"loca", b"glyf",
    b"CFF ", b"cvt ", b"fpgm", b"prep", b"gasp", b"hdmx", b"kern", b"LTSH", b"VDMX", b"vhea",
    b"vmtx", b"VORG", b"MATH", b"Silf", b"Sill", b"Glat", b"Gloc", b"Feat",
    // Blink's pass-through list.
    b"CBDT", b"CBLC", b"COLR", b"CPAL", b"CFF2", b"sbix", b"STAT", b"avar", b"BASE", b"cvar",
    b"fvar", b"gvar", b"HVAR", b"MVAR", b"VVAR", b"GDEF", b"GPOS", b"GSUB",
];

/// Rebuilds face `index` of `data` with the tables OTS would keep, as a
/// single-face sfnt. `None` when the data is not a font.
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
        let Some(bytes) = font.table_data(tag) else {
            continue;
        };
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
