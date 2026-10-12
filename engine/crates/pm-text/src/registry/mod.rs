//! The native font registry (plan section 12): one authority for system
//! fonts, project fonts, and fallback.
//!
//! - `CoreTextRegistry` (macOS) answers the layout's resolver queries with
//!   the CoreText and AppKit calls Blink's macOS font matching makes, in the
//!   same order, so level 1 layout resolves fonts as today's Canvas2D does.
//!   It enumerates installed families, holds project fonts, produces
//!   metrics-and-shaping subsets, and builds the fallback manifest level 2
//!   layout uses.
//! - Platform-independent parts live here: blob keys, project font
//!   selection (Blink's `FontSelectionAlgorithm`), and the manifest types.
//!
//! Before Phase 2 the registry runs in Electron main through `pm-bridge`;
//! from Phase 2 in the engine. Main serves subsets to `pm-wasm` either way.

#[cfg(all(target_os = "macos", feature = "system"))]
mod coretext;
#[cfg(all(target_os = "macos", feature = "system"))]
pub use coretext::CoreTextRegistry;

use crate::font::{FontDesc, WebDescriptors};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A blob key from content: the first 16 bytes of SHA-256, hex.
pub fn content_key(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes)[..16])
}

/// A blob key for an installed font file from its source facts (path, size,
/// modification time), so keys are stable without hashing large system
/// fonts. A changed file gets a new key and new subsets.
pub fn source_key(path: &str, len: u64, modified_ns: u128) -> String {
    let mut h = Sha256::new();
    h.update(b"pm-text source\0");
    h.update(path.as_bytes());
    h.update([0]);
    h.update(len.to_be_bytes());
    h.update(modified_ns.to_be_bytes());
    hex(&h.finalize()[..16])
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A project font's `FontFace` style descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ProjectStyle {
    Normal,
    Italic,
    /// `oblique <lo>deg <hi>deg`.
    Oblique { lo: f32, hi: f32 },
}

/// Descriptors of a project font face (`FontFace(family, data, {weight, style, stretch})`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDescriptors {
    /// `weight: <lo> <hi>` (a single weight has lo == hi).
    pub weight: (f32, f32),
    pub style: ProjectStyle,
    /// `stretch` in percent.
    #[serde(default = "normal_stretch")]
    pub stretch: (f32, f32),
}

fn normal_stretch() -> (f32, f32) {
    (100.0, 100.0)
}

impl Default for ProjectDescriptors {
    fn default() -> Self {
        ProjectDescriptors { weight: (400.0, 400.0), style: ProjectStyle::Normal, stretch: normal_stretch() }
    }
}

/// Blink's italic slope value and threshold (`font_selection_types.h`).
pub const ITALIC_SLOPE: f32 = 20.0;

impl ProjectDescriptors {
    /// Blink's selection capabilities for the face (slope in CSS degrees).
    pub fn slope(&self) -> (f32, f32) {
        match self.style {
            ProjectStyle::Normal => (0.0, 0.0),
            ProjectStyle::Italic => (ITALIC_SLOPE, ITALIC_SLOPE),
            ProjectStyle::Oblique { lo, hi } => (lo.min(hi), lo.max(hi)),
        }
    }

    pub fn web(&self) -> WebDescriptors {
        let slope = match self.style {
            // Blink maps italic to the `slnt` value it uses for every italic
            // request (see `font::requested_slope`).
            ProjectStyle::Italic => (14.0, 14.0),
            _ => self.slope(),
        };
        WebDescriptors { weight: self.weight, stretch: self.stretch, slope: Some(slope) }
    }
}

#[derive(Clone, Copy, Debug)]
struct Range(f32, f32);

impl Range {
    fn includes(&self, v: f32) -> bool {
        self.0 <= v && v <= self.1
    }
}

#[derive(Clone, Copy, Debug)]
struct Request {
    weight: f32,
    width: f32,
    slope: f32,
}

/// Blink's `FontSelectionAlgorithm` (CSS Fonts 4 §5.2) over a family's
/// project faces: stretch, then style, then weight, each by distance; the
/// first best face wins ties. Returns the index of the chosen face.
pub fn select_face(faces: &[ProjectDescriptors], desc: &FontDesc) -> Option<usize> {
    if faces.is_empty() {
        return None;
    }
    let req = Request { weight: desc.weight, width: 100.0, slope: if desc.italic { ITALIC_SLOPE } else { 0.0 } };
    let caps: Vec<(Range, Range, Range)> = faces
        .iter()
        .map(|f| (Range(f.stretch.0, f.stretch.1), Range(f.slope().0, f.slope().1), Range(f.weight.0, f.weight.1)))
        .collect();
    let bounds = caps.iter().fold((Range(f32::MAX, f32::MIN), Range(f32::MAX, f32::MIN), Range(f32::MAX, f32::MIN)), |b, c| {
        (Range(b.0.0.min(c.0.0), b.0.1.max(c.0.1)), Range(b.1.0.min(c.1.0), b.1.1.max(c.1.1)), Range(b.2.0.min(c.2.0), b.2.1.max(c.2.1)))
    });
    let stretch = |w: Range| -> f32 {
        if w.includes(req.width) {
            return 0.0;
        }
        if req.width > 100.0 {
            if w.0 > req.width {
                return w.0 - req.width;
            }
            return req.width.max(bounds.0.1) - w.1;
        }
        if w.1 < req.width {
            return req.width - w.1;
        }
        w.0 - req.width.min(bounds.0.0)
    };
    let style = |s: Range| -> f32 {
        if s.includes(req.slope) {
            return 0.0;
        }
        if req.slope >= ITALIC_SLOPE {
            if s.0 > req.slope {
                return s.0 - req.slope;
            }
            return req.slope.max(bounds.1.1) - s.1;
        }
        if req.slope >= 0.0 {
            if s.1 >= 0.0 && s.1 < req.slope {
                return req.slope - s.1;
            }
            if s.0 > req.slope {
                return s.0;
            }
            return req.slope.max(bounds.1.1) - s.1;
        }
        if req.slope > -ITALIC_SLOPE {
            if s.0 > req.slope && s.0 <= 0.0 {
                return s.0 - req.slope;
            }
            if s.1 < req.slope {
                return -s.1;
            }
            return s.0 - req.slope.min(bounds.1.0);
        }
        if s.1 < req.slope {
            return req.slope - s.1;
        }
        s.0 - req.slope.min(bounds.1.0)
    };
    let weight = |w: Range| -> f32 {
        if w.includes(req.weight) {
            return 0.0;
        }
        if (400.0..=500.0).contains(&req.weight) {
            if w.0 > req.weight && w.0 <= 500.0 {
                return w.0 - req.weight;
            }
            if w.1 < req.weight {
                return 500.0 - w.1;
            }
            return w.0 - req.weight.min(bounds.2.0);
        }
        if req.weight < 400.0 {
            if w.1 < req.weight {
                return req.weight - w.1;
            }
            return w.0 - req.weight.min(bounds.2.0);
        }
        if w.0 > req.weight {
            return w.0 - req.weight;
        }
        req.weight.max(bounds.2.1) - w.1
    };
    let mut best = 0;
    for i in 1..caps.len() {
        let (a, b) = (caps[i], caps[best]);
        let key = |c: (Range, Range, Range)| (stretch(c.0), style(c.1), weight(c.2));
        let (ka, kb) = (key(a), key(b));
        if ka.0 < kb.0 || (ka.0 == kb.0 && (ka.1 < kb.1 || (ka.1 == kb.1 && ka.2 < kb.2))) {
            best = i;
        }
    }
    Some(best)
}

/// Blink's synthesis decision for a chosen web face (`CSSSegmentedFontFace::GetFontData`):
/// bold when every weight the face covers is below 600 and the request is
/// at least 600; italic when the face is upright and the request is italic.
pub fn web_synthesis(face: &ProjectDescriptors, desc: &FontDesc) -> (bool, bool) {
    let bold = face.weight.1 < 600.0 && desc.weight >= 600.0;
    let italic = face.slope() == (0.0, 0.0) && desc.italic;
    (bold, italic)
}

/// One installed or project family, for the font picker and the
/// `fontFamilies` IPC.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyInfo {
    pub name: String,
    pub styles: Vec<StyleInfo>,
    /// Any face has an `fvar` table.
    pub variable: bool,
    /// A project font rather than an installed one.
    pub project: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleInfo {
    /// The face's style name ("Bold Italic").
    pub name: String,
    pub postscript_name: String,
    /// CSS weight (100..900).
    pub weight: f32,
    pub italic: bool,
}

/// The fallback order level 2 layout gives Parley: per script (ISO 15924)
/// and language, the families CoreText falls back to for a sample
/// character, plus the emoji family and the last resort. Built natively
/// from CoreText; `pm-wasm` configures its font collection from it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FallbackManifest {
    /// The family the manifest's queries substitute for.
    pub base_family: String,
    pub scripts: Vec<ScriptFallback>,
    pub emoji: Option<String>,
    pub last_resort: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptFallback {
    /// ISO 15924 code ("Arab", "Hani").
    pub script: String,
    /// BCP 47 language the families were chosen for (`None` for the script's default).
    pub language: Option<String>,
    pub families: Vec<String>,
}

/// Sample characters CoreText is asked about per script and language when
/// the manifest is built.
pub const MANIFEST_SAMPLES: &[(&str, Option<&str>, &str)] = &[
    ("Latn", None, "A"),
    ("Grek", None, "Ω"),
    ("Cyrl", None, "Ж"),
    ("Arab", None, "ب"),
    ("Hebr", None, "ש"),
    ("Deva", None, "क"),
    ("Beng", None, "ক"),
    ("Taml", None, "த"),
    ("Thai", None, "ก"),
    ("Hang", Some("ko"), "한"),
    ("Hira", Some("ja"), "あ"),
    ("Kana", Some("ja"), "カ"),
    ("Hani", Some("zh-Hans"), "编"),
    ("Hani", Some("zh-Hant"), "編"),
    ("Hani", Some("ja"), "編"),
    ("Syrc", None, "ܐ"),
    ("Ethi", None, "አ"),
    ("Armn", None, "Ա"),
    ("Geor", None, "ა"),
    ("Khmr", None, "ក"),
    ("Mymr", None, "က"),
    ("Sinh", None, "අ"),
    ("Tibt", None, "ཀ"),
    ("Zsym", None, "→"),
    ("Zmth", None, "∑"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn face(lo: f32, hi: f32, style: ProjectStyle) -> ProjectDescriptors {
        ProjectDescriptors { weight: (lo, hi), style, stretch: (100.0, 100.0) }
    }

    fn d(weight: f32, italic: bool) -> FontDesc {
        FontDesc { weight, italic, size: 16.0 }
    }

    #[test]
    fn css_weight_matching() {
        let faces = [face(300.0, 300.0, ProjectStyle::Normal), face(400.0, 400.0, ProjectStyle::Normal), face(700.0, 700.0, ProjectStyle::Normal)];
        // 400..500: the exact face, then lighter faces from 500 down.
        assert_eq!(select_face(&faces, &d(400.0, false)), Some(1));
        assert_eq!(select_face(&faces, &d(500.0, false)), Some(1));
        // Below 400: lighter first.
        assert_eq!(select_face(&faces, &d(350.0, false)), Some(0));
        assert_eq!(select_face(&faces, &d(200.0, false)), Some(0));
        // Above 500: heavier first.
        assert_eq!(select_face(&faces, &d(600.0, false)), Some(2));
        assert_eq!(select_face(&faces, &d(900.0, false)), Some(2));
        // A variable face covering the request wins outright.
        let faces = [face(400.0, 400.0, ProjectStyle::Normal), face(100.0, 900.0, ProjectStyle::Normal)];
        assert_eq!(select_face(&faces, &d(650.0, false)), Some(1));
    }

    #[test]
    fn style_before_weight_and_synthesis() {
        let faces = [face(400.0, 400.0, ProjectStyle::Normal), face(700.0, 700.0, ProjectStyle::Italic)];
        assert_eq!(select_face(&faces, &d(400.0, true)), Some(1));
        assert_eq!(select_face(&faces, &d(700.0, false)), Some(0));
        assert_eq!(web_synthesis(&faces[0], &d(700.0, true)), (true, true));
        assert_eq!(web_synthesis(&faces[1], &d(700.0, true)), (false, false));
    }

    #[test]
    fn keys_are_stable() {
        assert_eq!(content_key(b"abc"), "ba7816bf8f01cfea414140de5dae2223");
        assert_ne!(source_key("/a", 1, 2), source_key("/a", 1, 3));
    }
}
