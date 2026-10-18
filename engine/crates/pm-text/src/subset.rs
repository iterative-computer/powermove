//! Metrics-and-shaping subsets (plan section 12, amended in section 26
//! item 10): the tables `pm-wasm` needs to lay text out, without outlines,
//! bitmaps, or hinting. Main serves these to the renderer; the layout they
//! produce must be identical to the layout of the full font (checked by the
//! corpus regression test's subset replay and the WASM identity run).
//!
//! A variable font whose advances vary through `gvar` phantom points (no
//! `HVAR`) would lose its advance variations with `glyf` and `gvar` gone, so
//! the subset carries an `HVAR` synthesized from the phantom-point deltas.
//! Facts the layout needs about dropped tables (color tables present,
//! advances rounded by CoreText) travel in `FaceFacts`, not in the subset.

use crate::sfnt::build_sfnt;
use read_fonts::tables::glyf::{CompositeGlyphFlags, Glyf, Glyph};
use read_fonts::tables::gvar::Gvar;
use read_fonts::tables::loca::Loca;
use read_fonts::types::{F2Dot14, GlyphId, Tag};
use read_fonts::{FileRef, FontRef, ReadError, TableProvider};
use std::collections::HashMap;

/// The section 12 table list plus the four tables the section 26 amendment
/// adds (measured in the S4 report: without them 216 of 5,110 corpus cases
/// change):
/// - `maxp`: the glyph count every metrics table is sized by;
/// - `feat`: HarfBuzz and HarfRust map OpenType features to `morx`
///   selectors only for feature types listed in `feat` (Helvetica Neue's
///   ligatures and kerning disappear without it);
/// - `STAT`: `trak` is applied only to "modern" fonts, detected by `STAT`
///   (SF Pro's tracking disappears without it);
/// - `ankr`: anchor points for `kerx` format 4 attachment.
pub const TABLES: &[&[u8; 4]] = &[
    b"cmap", b"hmtx", b"hhea", b"OS/2", b"head", b"name", b"GSUB", b"GPOS", b"GDEF", b"kern",
    b"morx", b"kerx", b"trak", b"fvar", b"avar", b"HVAR", b"MVAR", b"maxp", b"feat", b"STAT",
    b"ankr",
];

/// What a subset contains, for the report.
#[derive(Clone, Debug, Default)]
pub struct SubsetInfo {
    pub bytes: usize,
    pub tables: Vec<String>,
    pub synthesized_hvar: bool,
}

/// Builds the metrics-and-shaping subset of face `index` of `data` as a
/// single-face sfnt (its index is 0).
/// Face `index` of `data` whole, outlines included, as a single-face font
/// (read at index 0): a single-face file's own bytes, or every table of the
/// collection's face assembled into one sfnt. What a host that measures
/// glyph ink (text bounds, raster placement) loads instead of the
/// metrics-and-shaping subset.
pub fn whole_face(data: &[u8], index: u32) -> Result<Vec<u8>, ReadError> {
    match FileRef::new(data)? {
        FileRef::Font(_) if index == 0 => Ok(data.to_vec()),
        FileRef::Font(_) => Err(ReadError::OutOfBounds),
        FileRef::Collection(collection) => {
            let font = collection.get(index)?;
            let mut tables: Vec<(u32, Vec<u8>)> = font
                .table_directory()
                .table_records()
                .iter()
                .filter_map(|record| {
                    let tag = record.tag();
                    font.table_data(tag)
                        .map(|bytes| (u32::from_be_bytes(tag.to_be_bytes()), bytes.as_bytes().to_vec()))
                })
                .collect();
            Ok(build_sfnt(&mut tables))
        }
    }
}

pub fn metrics_subset(data: &[u8], index: u32) -> Result<(Vec<u8>, SubsetInfo), ReadError> {
    let file = FileRef::new(data)?;
    let font = file
        .fonts()
        .nth(index as usize)
        .ok_or(ReadError::OutOfBounds)??;
    let mut tables: Vec<(u32, Vec<u8>)> = Vec::new();
    for record in font.table_directory().table_records() {
        let raw = record.tag().to_be_bytes();
        if !TABLES.iter().any(|t| **t == raw) {
            continue;
        }
        if let Some(bytes) = font.table_data(record.tag()) {
            tables.push((u32::from_be_bytes(raw), bytes.as_bytes().to_vec()));
        }
    }
    let has = |t: &[u8; 4]| font.table_data(Tag::new(t)).is_some();
    let mut synthesized_hvar = false;
    if has(b"fvar") && has(b"gvar") && !has(b"HVAR") {
        tables.push((u32::from_be_bytes(*b"HVAR"), synthesize_hvar(&font)?));
        synthesized_hvar = true;
    }
    let mut names: Vec<String> = tables
        .iter()
        .map(|(t, _)| String::from_utf8_lossy(&t.to_be_bytes()).into_owned())
        .collect();
    names.sort();
    let out = build_sfnt(&mut tables);
    let info = SubsetInfo {
        bytes: out.len(),
        tables: names,
        synthesized_hvar,
    };
    Ok((out, info))
}

/// The glyph whose phantom points carry `gid`'s metrics and its point count
/// (gvar: the last `USE_MY_METRICS` component of a composite, recursively).
fn metrics_glyph(
    glyf: &Glyf,
    loca: &Loca,
    gid: GlyphId,
    depth: usize,
) -> Result<(GlyphId, usize), ReadError> {
    if depth > 64 {
        return Err(ReadError::MalformedData("composite nesting"));
    }
    match loca.get_glyf(gid, glyf)? {
        None => Ok((gid, 0)),
        Some(Glyph::Simple(s)) => Ok((gid, s.num_points())),
        Some(Glyph::Composite(c)) => {
            let mut count = 0;
            let mut inherit = None;
            for (component, flags) in c.component_glyphs_and_flags() {
                count += 1;
                if flags.contains(CompositeGlyphFlags::USE_MY_METRICS) {
                    inherit = Some(component);
                }
            }
            match inherit {
                Some(component) => metrics_glyph(glyf, loca, component.into(), depth + 1),
                None => Ok((gid, count)),
            }
        }
    }
}

type RegionKey = Vec<(i16, i16, i16)>;

/// An `HVAR` (implicit glyph mapping, one item variation data) whose
/// advance deltas equal the `gvar` right-minus-left phantom deltas, tuple by
/// tuple. Each `gvar` tuple becomes one region, so the interpolated delta at
/// any location is the one `gvar` produces.
fn synthesize_hvar(font: &FontRef) -> Result<Vec<u8>, ReadError> {
    let gvar: Gvar = font.gvar()?;
    let glyf = font.glyf()?;
    let loca = font.loca(None)?;
    let num_glyphs = font.maxp()?.num_glyphs() as usize;
    let axis_count = gvar.axis_count() as usize;
    let mut regions: Vec<RegionKey> = Vec::new();
    let mut region_ids: HashMap<RegionKey, usize> = HashMap::new();
    let mut rows: Vec<HashMap<usize, i32>> = vec![HashMap::new(); num_glyphs];
    for (gid, row) in rows.iter_mut().enumerate() {
        let gid = GlyphId::new(gid as u32);
        let Ok((source, points)) = metrics_glyph(&glyf, &loca, gid, 0) else {
            continue;
        };
        let Some(var) = gvar.glyph_variation_data(source)? else {
            continue;
        };
        let (left, right) = (points, points + 1);
        for tuple in var.tuples() {
            let (mut dl, mut dr) = (0i32, 0i32);
            for d in tuple.deltas() {
                let p = d.position as usize;
                if p == left {
                    dl = d.x_delta;
                } else if p == right {
                    dr = d.x_delta;
                }
            }
            let delta = dr - dl;
            if delta == 0 {
                continue;
            }
            let peak = tuple.peak();
            let inter = tuple.intermediate_start().zip(tuple.intermediate_end());
            let key: RegionKey = (0..axis_count)
                .map(|a| {
                    let p = peak.get(a).unwrap_or(F2Dot14::ZERO).to_bits();
                    match &inter {
                        Some((s, e)) => (
                            s.get(a).unwrap_or(F2Dot14::ZERO).to_bits(),
                            p,
                            e.get(a).unwrap_or(F2Dot14::ZERO).to_bits(),
                        ),
                        None => (p.min(0), p, p.max(0)),
                    }
                })
                .collect();
            let next = regions.len();
            let id = *region_ids.entry(key.clone()).or_insert_with(|| {
                regions.push(key);
                next
            });
            *row.entry(id).or_insert(0) += delta;
        }
    }
    let long = rows
        .iter()
        .flat_map(|r| r.values())
        .any(|d| *d < i16::MIN as i32 || *d > i16::MAX as i32);
    let region_count = regions.len();

    let mut ivs = Vec::new();
    // ItemVariationStore header: format, region list offset, one data offset.
    let region_list_offset = 2 + 4 + 2 + 4;
    let region_list_len = 4 + region_count * axis_count * 6;
    let data_offset = region_list_offset + region_list_len;
    ivs.extend_from_slice(&1u16.to_be_bytes());
    ivs.extend_from_slice(&(region_list_offset as u32).to_be_bytes());
    ivs.extend_from_slice(&1u16.to_be_bytes());
    ivs.extend_from_slice(&(data_offset as u32).to_be_bytes());
    ivs.extend_from_slice(&(axis_count as u16).to_be_bytes());
    ivs.extend_from_slice(&(region_count as u16).to_be_bytes());
    for r in &regions {
        for (s, p, e) in r {
            ivs.extend_from_slice(&s.to_be_bytes());
            ivs.extend_from_slice(&p.to_be_bytes());
            ivs.extend_from_slice(&e.to_be_bytes());
        }
    }
    // ItemVariationData: every region is a "word" column (16-bit, or 32-bit
    // with LONG_WORDS).
    ivs.extend_from_slice(&(num_glyphs as u16).to_be_bytes());
    let word_count = region_count as u16 | if long { 0x8000 } else { 0 };
    ivs.extend_from_slice(&word_count.to_be_bytes());
    ivs.extend_from_slice(&(region_count as u16).to_be_bytes());
    for i in 0..region_count {
        ivs.extend_from_slice(&(i as u16).to_be_bytes());
    }
    for row in &rows {
        for i in 0..region_count {
            let d = row.get(&i).copied().unwrap_or(0);
            if long {
                ivs.extend_from_slice(&d.to_be_bytes());
            } else {
                ivs.extend_from_slice(&(d as i16).to_be_bytes());
            }
        }
    }

    let mut hvar = Vec::new();
    hvar.extend_from_slice(&1u16.to_be_bytes());
    hvar.extend_from_slice(&0u16.to_be_bytes());
    hvar.extend_from_slice(&20u32.to_be_bytes()); // item variation store
    hvar.extend_from_slice(&0u32.to_be_bytes()); // advance mapping: implicit
    hvar.extend_from_slice(&0u32.to_be_bytes()); // lsb mapping
    hvar.extend_from_slice(&0u32.to_be_bytes()); // rsb mapping
    hvar.extend_from_slice(&ivs);
    Ok(hvar)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use read_fonts::model::{Blob, Font, Source};
    use std::sync::Arc;

    fn model(bytes: Vec<u8>) -> Font {
        let shared: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(bytes);
        Font::new(Source::Blob(Blob::from(shared)), 0).unwrap()
    }

    /// Skia ships with macOS and is the system's `gvar`-only variable font.
    #[test]
    fn synthesized_hvar_matches_gvar_phantom_advances() {
        let full = std::fs::read("/System/Library/Fonts/Supplemental/Skia.ttf")
            .expect("Skia.ttf ships with macOS");
        let (sub, info) = metrics_subset(&full, 0).unwrap();
        assert!(info.synthesized_hvar);
        assert!(
            sub.len() < full.len() / 2,
            "subset {} of {}",
            sub.len(),
            full.len()
        );
        let (a, b) = (model(full), model(sub));
        let n = FontRef::new(&std::fs::read("/System/Library/Fonts/Supplemental/Skia.ttf").unwrap())
            .unwrap()
            .maxp()
            .unwrap()
            .num_glyphs() as u32;
        let mut worst = 0f64;
        for wght in [0.48f32, 0.7, 1.0, 1.3, 1.9, 2.5, 3.2] {
            for wdth in [0.62f32, 0.8, 1.0, 1.15, 1.3] {
                let at = |f: &Font| {
                    f.instance_builder()
                        .variations([(Tag::new(b"wght"), wght), (Tag::new(b"wdth"), wdth)])
                        .build()
                };
                let (fa, fb) = (at(&a), at(&b));
                for g in 0..n {
                    let g = GlyphId::new(g);
                    let x = fa.glyph_metrics().h_advance_exact(g).to_f64();
                    let y = fb.glyph_metrics().h_advance_exact(g).to_f64();
                    worst = worst.max((x - y).abs());
                }
            }
        }
        assert!(
            worst < 1.0 / 1024.0,
            "worst advance difference {worst} units"
        );
    }
}
