//! Font faces, the resolver boundary, and per-size prepared faces.
//!
//! The resolver is the font registry's interface (plan section 12): native
//! builds answer it with CoreText (`registry`); `pm-wasm` answers it from the
//! answers and metrics-and-shaping subsets main serves (`answers`). Layout
//! itself never touches a platform API.

use read_fonts::model::{Blob, Source};
use read_fonts::tables::fvar::VariationAxisRecord;
use read_fonts::types::Tag;
use read_fonts::{FontRef, TableProvider};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

pub type FaceId = u32;

/// Shared font file bytes (owned, or a read-only mapping of a system font file).
#[derive(Clone)]
pub struct FontBytes(Arc<dyn AsRef<[u8]> + Send + Sync>);

impl FontBytes {
    pub fn new(bytes: Vec<u8>) -> Self {
        FontBytes(Arc::new(bytes))
    }

    pub fn from_shared(shared: Arc<dyn AsRef<[u8]> + Send + Sync>) -> Self {
        FontBytes(shared)
    }

    pub fn as_slice(&self) -> &[u8] {
        (*self.0).as_ref()
    }

    pub fn shared(&self) -> Arc<dyn AsRef<[u8]> + Send + Sync> {
        self.0.clone()
    }

    /// Identity of the underlying buffer (two clones share it).
    pub fn ptr_id(&self) -> usize {
        self.as_slice().as_ptr() as usize
    }
}

/// Facts about a face that layout needs from tables a metrics-and-shaping
/// subset drops. The registry reads them from the full font.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaceFacts {
    /// `sbix` present (CoreText sizes color glyph advances by strike).
    pub has_sbix: bool,
    /// Any color glyph table (`sbix`, `COLR`, `CBDT`, `SVG `): Blink's
    /// variation selector handling depends on it.
    pub has_color_table: bool,
    /// Advances vary through `gvar` phantom points (no `HVAR`); CoreText
    /// rounds those to whole units.
    pub phantom_advances: bool,
}

impl FaceFacts {
    pub fn of(data: &[u8], index: u32) -> FaceFacts {
        let Ok(f) = FontRef::from_index(data, index) else { return FaceFacts::default() };
        let has = |t: &[u8; 4]| f.table_data(Tag::new(t)).is_some();
        let has_sbix = has(b"sbix");
        FaceFacts {
            has_sbix,
            has_color_table: has_sbix || has(b"COLR") || has(b"CBDT") || has(b"SVG "),
            phantom_advances: has(b"fvar") && has(b"gvar") && !has(b"HVAR"),
        }
    }
}

/// One platform typeface as Blink's `FontPlatformData` sees it.
#[derive(Clone)]
pub struct FaceInfo {
    pub data: FontBytes,
    pub index: u32,
    /// The registry's key for the font file the face reads (a source or
    /// content hash, see `registry::blob_key`). Subsets are served by
    /// `(blob, index)`.
    pub blob: String,
    /// Variation coordinates carried by the platform typeface (the CTFont's
    /// variation dictionary), before CSS optical sizing.
    pub coords: Vec<(u32, f32)>,
    pub synthetic_bold: bool,
    pub synthetic_italic: bool,
    /// A web font created from bytes (`FontFace`), as opposed to an installed font.
    pub custom: bool,
    /// The web font's descriptors (`custom` faces only; `None` means
    /// `VARIABLE_FACE_DESCRIPTORS`).
    pub web: Option<WebDescriptors>,
    /// Glyphs CoreText synthesizes for U+2010 and U+2011 when the cmap lacks them.
    pub synthesized_hyphens: [Option<u16>; 2],
    pub postscript_name: String,
    /// Stable identity used for de-duplication in the fallback iterator
    /// (Skia's typeface unique id).
    pub typeface_key: String,
    /// Blink's `CharacterFallbackKey` font identifier for this face as the
    /// font being substituted (PostScript name, or UI usage and language for
    /// hidden system fonts); `None` disables the ideograph fallback cache.
    pub fallback_key: Option<String>,
    pub facts: FaceFacts,
}

/// The parts of Blink's `FontDescription` that font resolution depends on.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FontDesc {
    pub weight: f32,
    pub italic: bool,
    /// Computed (and specified) size in CSS px.
    pub size: f32,
}

/// The font registry boundary.
pub trait FontResolver {
    fn face(&self, id: FaceId) -> &FaceInfo;
    /// `FontCache::GetFontData(description, family)`, generic families
    /// included. `None` when the family is not available.
    fn family(&mut self, family: &str, desc: &FontDesc) -> Option<FaceId>;
    /// The user's standard font (`-webkit-standard`).
    fn standard(&mut self, desc: &FontDesc) -> Option<FaceId>;
    /// `FontCache::PlatformFallbackFontForCharacter`.
    fn fallback_for_char(&mut self, desc: &FontDesc, ch: u32, primary: FaceId, emoji_presentation: bool) -> Option<FaceId>;
    /// `FontCache::GetLastResortFallbackFont`.
    fn last_resort(&mut self, desc: &FontDesc) -> Option<FaceId>;
    /// The variable-font source today's `typography/font-renderer.ts`
    /// registers as a `FontFace` when a text layer sets `fontAxis.*` values:
    /// a project font of that family, or the installed family's first face
    /// with an `fvar` table (`inspectFont`), sanitized as Blink sanitizes web
    /// fonts. The layout fills in the face's coordinates per request.
    fn variable_face(&mut self, family: &str) -> Option<FaceId>;
    /// The advance CoreText reports for a full-em glyph of an `sbix` face at
    /// `size` (with Skia's untracked exact-copy attributes). CoreText sizes
    /// color glyph advances by strike, which no table describes; the
    /// registry serves the value. `None` means advances follow `hmtx`.
    fn em_advance(&mut self, _face: FaceId, _size: f32) -> Option<f32> {
        None
    }
}

/// An OpenType tag from its four characters (shorter strings are padded
/// with spaces, longer ones truncated; callers validate axis tags first).
pub fn tag(s: &str) -> u32 {
    let mut b = [b' '; 4];
    for (d, s) in b.iter_mut().zip(s.bytes()) {
        *d = s;
    }
    u32::from_be_bytes(b)
}

pub const WGHT: u32 = u32::from_be_bytes(*b"wght");
pub const WDTH: u32 = u32::from_be_bytes(*b"wdth");
pub const SLNT: u32 = u32::from_be_bytes(*b"slnt");
pub const OPSZ: u32 = u32::from_be_bytes(*b"opsz");

#[derive(Clone, Copy, Debug)]
pub struct Axis {
    pub tag: u32,
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

pub fn axes(data: &[u8], index: u32) -> Vec<Axis> {
    let Ok(font) = FontRef::from_index(data, index) else { return Vec::new() };
    let Ok(fvar) = font.fvar() else { return Vec::new() };
    let Ok(records) = fvar.axes() else { return Vec::new() };
    records
        .iter()
        .map(|r: &VariationAxisRecord| Axis {
            tag: u32::from_be_bytes(r.axis_tag().to_be_bytes()),
            min: r.min_value().to_f32(),
            default: r.default_value().to_f32(),
            max: r.max_value().to_f32(),
        })
        .collect()
}

/// A face prepared for shaping at one size: the read-fonts model font at its
/// final variation coordinates plus the facts Blink derives from the typeface.
pub struct PreparedFace {
    pub font: read_fonts::model::Font,
    /// The same face at its default location (HarfBuzz's unvaried `hmtx`).
    pub default_font: Option<read_fonts::model::Font>,
    pub upem: u16,
    pub size: f32,
    pub has_trak: bool,
    pub has_sbix: bool,
    pub has_color_table: bool,
    /// Varied advances come from `gvar` phantom points (no `HVAR`), which
    /// CoreText rounds to whole font units.
    pub phantom_advances: bool,
    pub space_glyph: Option<u32>,
    pub synthetic_bold: bool,
    pub synthesized_hyphens: [Option<u16>; 2],
    pub coords: Vec<(u32, f32)>,
    /// See `FontResolver::em_advance`.
    pub em_advance: Option<f32>,
}

fn blob_source(data: &FontBytes) -> Source {
    Source::Blob(Blob::from(data.shared()))
}

/// Final coordinates for an installed face (`FontPlatformDataFromCTFont`):
/// the typeface's own coordinates, then `opsz` set to the specified size
/// (`specified`, not the effective size the face is measured at) when the
/// font has that axis (font-optical-sizing: auto).
pub fn platform_coords(info: &FaceInfo, specified: f32) -> Vec<(u32, f32)> {
    let axes = axes(info.data.as_slice(), info.index);
    if axes.is_empty() {
        return Vec::new();
    }
    let mut coords: Vec<(u32, f32)> = axes
        .iter()
        .map(|a| {
            let v = info.coords.iter().find(|(t, _)| *t == a.tag).map(|(_, v)| *v).unwrap_or(a.default);
            (a.tag, v)
        })
        .collect();
    for (t, v) in coords.iter_mut() {
        if *t == OPSZ {
            let a = axes.iter().find(|a| a.tag == OPSZ).expect("opsz axis");
            *v = specified.clamp(a.min, a.max);
        }
    }
    coords
}

/// A web font's `FontFace` descriptors, as Blink's font selection
/// capabilities: ranges for weight and stretch (percent) and the slope range
/// in CSS degrees (`None`: the descriptor equals the requested style, as
/// font-renderer.ts creates one face per style).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebDescriptors {
    pub weight: (f32, f32),
    pub stretch: (f32, f32),
    pub slope: Option<(f32, f32)>,
}

/// The descriptors font-renderer.ts gives every variable `FontFace`:
/// `weight: 1 1000`, `stretch: 1% 1000%`, style equal to the request.
pub const VARIABLE_FACE_DESCRIPTORS: WebDescriptors = WebDescriptors { weight: (1.0, 1000.0), stretch: (1.0, 1000.0), slope: None };

/// CSS slope of a request: normal 0, italic 14 degrees (the value Blink
/// maps to `slnt` for web fonts).
pub fn requested_slope(desc: &FontDesc) -> f32 {
    if desc.italic { 14.0 } else { 0.0 }
}

/// Final coordinates for a web font (`FontCustomPlatformData::GetFontPlatformData`):
/// the request's weight, stretch, and slope clamped to the face's descriptor
/// ranges, then the face's `variationSettings` (`settings`; the last value for
/// an axis wins), then `opsz` set to the specified size
/// (`adjusted_specified_size`) unless the settings name it.
pub fn custom_coords(info: &FaceInfo, desc: &FontDesc, specified: f32, settings: &[(u32, f32)]) -> Vec<(u32, f32)> {
    let axes = axes(info.data.as_slice(), info.index);
    if axes.is_empty() {
        return Vec::new();
    }
    let d = info.web.unwrap_or(VARIABLE_FACE_DESCRIPTORS);
    let weight = desc.weight.clamp(d.weight.0, d.weight.1);
    let width = 100.0f32.clamp(d.stretch.0, d.stretch.1);
    let request = requested_slope(desc);
    let slope = match d.slope {
        Some((lo, hi)) => request.clamp(lo, hi),
        None => request,
    };
    let mut requested: Vec<(u32, f32)> = vec![(WGHT, weight), (WDTH, width), (SLNT, -slope)];
    let mut explicit_opsz = false;
    for &(t, v) in settings {
        if t == OPSZ {
            explicit_opsz = true;
        }
        requested.push((t, v));
    }
    if !explicit_opsz {
        requested.push((OPSZ, specified));
    }
    axes.iter()
        .map(|a| {
            // Skia: the last value given for an axis wins; values are clamped to the axis.
            let v = requested.iter().rev().find(|(t, _)| *t == a.tag).map(|(_, v)| *v).unwrap_or(a.default);
            (a.tag, v.clamp(a.min, a.max))
        })
        .collect()
}

/// Final coordinates for `face` at `desc`: web fonts (project fonts and
/// variable `FontFace`s) through `custom_coords`, installed fonts through
/// `platform_coords`.
pub fn face_coords(info: &FaceInfo, desc: &FontDesc, settings: &[(u32, f32)]) -> Vec<(u32, f32)> {
    if info.custom { custom_coords(info, desc, desc.size, settings) } else { platform_coords(info, desc.size) }
}

pub fn prepare(info: &FaceInfo, coords: Vec<(u32, f32)>, size: f32, em_advance: Option<f32>) -> Option<PreparedFace> {
    let base = read_fonts::model::Font::new(blob_source(&info.data), info.index)?;
    let (font, default_font) = if coords.is_empty() {
        (base, None)
    } else {
        let varied = base.instance_builder().variations(coords.iter().map(|&(t, v)| (Tag::from_u32(t), v))).build();
        (varied, Some(base))
    };
    let fref = FontRef::from_index(info.data.as_slice(), info.index).ok()?;
    let has = |t: &[u8; 4]| fref.table_data(Tag::new(t)).is_some();
    let has_trak = has(b"trak");
    let has_sbix = info.facts.has_sbix;
    let has_color_table = info.facts.has_color_table;
    let phantom_advances = !coords.is_empty() && info.facts.phantom_advances;
    let upem = font.units_per_em();
    let space_glyph = font.charmap().map_unicode(0x20u32).map(|g| g.to_u32());
    Some(PreparedFace {
        font,
        default_font,
        upem,
        size,
        has_trak,
        has_sbix,
        has_color_table,
        phantom_advances,
        space_glyph,
        synthetic_bold: info.synthetic_bold,
        synthesized_hyphens: info.synthesized_hyphens,
        coords,
        em_advance: if has_sbix { em_advance } else { None },
    })
}

/// (face, size bits, coordinate bits).
type PreparedKey = (FaceId, u32, Vec<(u32, u32)>);

/// Caches prepared faces by (face, size, coordinates).
#[derive(Default)]
pub struct PreparedCache {
    map: HashMap<PreparedKey, Arc<PreparedFace>>,
}

impl PreparedCache {
    /// `em_advance` is asked only when the face is prepared for the first
    /// time and has an `sbix` table.
    pub fn get(
        &mut self,
        id: FaceId,
        info: &FaceInfo,
        coords: Vec<(u32, f32)>,
        size: f32,
        em_advance: impl FnOnce() -> Option<f32>,
    ) -> Option<Arc<PreparedFace>> {
        let key = (id, size.to_bits(), coords.iter().map(|&(t, v)| (t, v.to_bits())).collect::<Vec<_>>());
        if let Some(p) = self.map.get(&key) {
            return Some(p.clone());
        }
        let em = if info.facts.has_sbix { em_advance() } else { None };
        let p = Arc::new(prepare(info, coords, size, em)?);
        self.map.insert(key, p.clone());
        Some(p)
    }
}
