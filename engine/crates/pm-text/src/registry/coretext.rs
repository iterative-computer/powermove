use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSFont, NSFontManager};
use objc2_core_foundation::{
    CFArray, CFCharacterSet, CFData, CFDictionary, CFNumber, CFRange, CFRetained, CFString, CFType, CFURL,
};
use objc2_core_text::{
    CTFont, CTFontCollection, CTFontDescriptor, CTFontManagerCopyAvailableFontFamilyNames,
    CTFontManagerCreateFontDescriptorFromData, CTFontSymbolicTraits, CTFontTableOptions, CTFontUIFontType,
    kCTFontCascadeListAttribute, kCTFontDisplayNameAttribute, kCTFontFamilyNameAttribute, kCTFontFamilyNameKey,
    kCTFontNameAttribute, kCTFontStyleNameAttribute, kCTFontStyleNameKey, kCTFontSymbolicTrait,
    kCTFontTraitsAttribute, kCTFontURLAttribute, kCTFontVariationAttribute, kCTFontVariationAxesAttribute,
    kCTFontVariationAxisIdentifierKey, kCTFontVariationAxisMaximumValueKey, kCTFontVariationAxisMinimumValueKey,
    kCTFontWeightTrait,
};
use objc2_foundation::{NSArray, NSNumber, NSString};
use super::{
    FaceSource, FallbackManifest, FamilyInfo, ManifestFace, MANIFEST_SAMPLES, ProjectDescriptors, ProjectStyle, ScriptFallback, StyleInfo,
    content_key, select_face, source_key, web_synthesis,
};
use crate::font::{FaceFacts, FaceId, FaceInfo, FontBytes, FontDesc, FontResolver, WebDescriptors};
use crate::sfnt::build_sfnt;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

const TRAIT_ITALIC: u32 = 1 << 0;
const TRAIT_BOLD: u32 = 1 << 1;
const TRAIT_EXPANDED: u32 = 1 << 5;
const TRAIT_CONDENSED: u32 = 1 << 6;
const IMPORTANT_TRAITS: u32 = TRAIT_ITALIC | TRAIT_BOLD | TRAIT_CONDENSED | TRAIT_EXPANDED;

const NORMAL_WEIGHT: i32 = 400;
const BOLD_THRESHOLD: f32 = 600.0;

/// `ToCSSFontWeight` (font_matcher_mac.mm).
fn to_css_weight(ct: f32) -> i32 {
    const W: [(f32, f32, i32); 9] = [
        (-1.0, -0.70, 100),
        (-0.70, -0.45, 200),
        (-0.45, -0.10, 300),
        (-0.10, 0.10, 400),
        (0.10, 0.27, 500),
        (0.27, 0.35, 600),
        (0.35, 0.50, 700),
        (0.50, 0.60, 800),
        (0.60, 1.0, 900),
    ];
    for (lo, hi, css) in W {
        if lo <= ct && ct <= hi {
            return css;
        }
    }
    NORMAL_WEIGHT
}

/// `ToCTFontWeight`.
fn to_ct_weight(css: i32) -> f32 {
    if css <= 50 || css >= 950 {
        return 0.0;
    }
    const W: [f32; 9] = [-0.80, -0.60, -0.40, 0.0, 0.23, 0.30, 0.40, 0.56, 0.62];
    W[((css - 50) / 100) as usize]
}

/// `AppKitToCSSFontWeight`.
fn appkit_to_css_weight(w: i32) -> i32 {
    if w < 0 {
        return NORMAL_WEIGHT;
    }
    if w < 7 { ((w - 1) * 100).max(100) } else { ((w - 2) * 100).min(900) }
}

fn better_weight_match(desired: i32, chosen: i32, candidate: i32, lower: i32, upper: i32) -> bool {
    if desired > upper {
        let ca = chosen >= desired;
        let na = candidate >= desired;
        if ca != na {
            return na;
        }
    } else if desired < lower {
        let cb = chosen <= desired;
        let nb = candidate <= desired;
        if cb != nb {
            return nb;
        }
    } else {
        let prio = |w: i32| {
            if w >= desired && w <= upper {
                0
            } else if w < desired {
                1
            } else {
                2
            }
        };
        let (cp, np) = (prio(chosen), prio(candidate));
        if cp != np {
            return np < cp;
        }
    }
    (candidate - desired).abs() < (chosen - desired).abs()
}

/// `BetterChoiceCT`.
fn better_choice_ct(desired_traits: u32, desired_weight: i32, chosen_traits: u32, chosen_weight: i32, cand_traits: u32, cand_weight: i32) -> bool {
    for mask in [TRAIT_CONDENSED, TRAIT_EXPANDED, TRAIT_ITALIC] {
        let desired = desired_traits & mask != 0;
        let chosen_unwanted = desired != (chosen_traits & mask != 0);
        let cand_unwanted = desired != (cand_traits & mask != 0);
        if !cand_unwanted && chosen_unwanted {
            return true;
        }
        if !chosen_unwanted && cand_unwanted {
            return false;
        }
    }
    better_weight_match(desired_weight, chosen_weight, cand_weight, 400, 500)
}

fn cfstr(s: &str) -> CFRetained<CFString> {
    CFString::from_str(s)
}

fn num_f32(v: &CFType) -> Option<f32> {
    v.downcast_ref::<CFNumber>().and_then(|n| n.as_f64()).map(|f| f as f32)
}

fn num_i64(v: &CFType) -> Option<i64> {
    v.downcast_ref::<CFNumber>().and_then(|n| n.as_i64())
}

/// (symbolic traits, CSS weight) from a traits dictionary.
fn traits_of(dict: &CFDictionary) -> (Option<u32>, Option<f32>) {
    // SAFETY: the traits dictionary maps CFString keys to CFNumber values.
    let dict: &CFDictionary<CFString, CFType> = unsafe { dict.cast_unchecked() };
    let sym = dict.get(unsafe { kCTFontSymbolicTrait }).and_then(|v| num_i64(&v)).map(|v| v as u32);
    let weight = dict.get(unsafe { kCTFontWeightTrait }).and_then(|v| num_f32(&v));
    (sym, weight)
}

fn descriptor_traits(d: &CTFontDescriptor) -> (Option<u32>, Option<f32>) {
    let Some(t) = (unsafe { d.attribute(kCTFontTraitsAttribute) }) else { return (None, None) };
    match t.downcast::<CFDictionary>() {
        Ok(dict) => traits_of(&dict),
        Err(_) => (None, None),
    }
}

fn ct_font_weight(f: &CTFont) -> f32 {
    let dict = unsafe { f.traits() };
    traits_of(&dict).1.unwrap_or(0.0)
}

fn symbolic(f: &CTFont) -> u32 {
    unsafe { f.symbolic_traits() }.0
}

/// A typeface the registry has handed out.
struct Face {
    info: FaceInfo,
    ct: CFRetained<CTFont>,
}

/// A project font face: a `FontFace` the project registers under a family
/// name. The bytes are sanitized as Blink sanitizes web fonts.
struct ProjectFace {
    family: String,
    descriptors: ProjectDescriptors,
    data: FontBytes,
    blob: String,
    ct: CFRetained<CTFont>,
}

/// Errors from registering fonts with the registry.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("the data is not a font CoreText and the web font sanitizer accept")]
    NotAFont,
    #[error("unknown face {0}")]
    UnknownFace(u32),
    #[error("unknown font blob {0}")]
    UnknownBlob(String),
    #[error("font blob {blob} has no face {index}: {source}")]
    Subset { blob: String, index: u32, source: read_fonts::ReadError },
}

pub struct CoreTextRegistry {
    fm: Retained<NSFontManager>,
    faces: Vec<Face>,
    by_key: HashMap<String, FaceId>,
    unavailable: HashSet<String>,
    family_cache: HashMap<(String, u32, bool, u32), Option<FaceId>>,
    /// Installed font files read so far, with their blob keys.
    files: HashMap<PathBuf, (FontBytes, String)>,
    /// Fonts CoreText serves only as tables, by PostScript name.
    table_fonts: HashMap<String, (FontBytes, String)>,
    /// Every blob a face reads, by key.
    blobs: HashMap<String, FontBytes>,
    em_advances: HashMap<(FaceId, u32), Option<f32>>,
    variable: HashMap<String, Option<FaceId>>,
    project: Vec<ProjectFace>,
    /// `PM.fontSource(family)` sources (project font bytes the variable-font
    /// path reads), by family.
    sources: HashMap<String, (FontBytes, String)>,
    subsets: HashMap<(String, u32), Arc<Vec<u8>>>,
    generation: u64,
}

impl CoreTextRegistry {
    /// Must run on the main thread (AppKit's shared font manager); Electron
    /// main's JavaScript thread is the process's main thread.
    pub fn new() -> Self {
        Self::try_new().expect("CoreTextRegistry must be created on the main thread")
    }

    /// `new`, or `None` off the main thread.
    pub fn try_new() -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        Some(CoreTextRegistry {
            fm: NSFontManager::sharedFontManager(mtm),
            faces: Vec::new(),
            by_key: HashMap::new(),
            unavailable: HashSet::new(),
            family_cache: HashMap::new(),
            files: HashMap::new(),
            table_fonts: HashMap::new(),
            blobs: HashMap::new(),
            em_advances: HashMap::new(),
            variable: HashMap::new(),
            project: Vec::new(),
            sources: HashMap::new(),
            subsets: HashMap::new(),
            generation: 0,
        })
    }

    /// Installed font files read so far, with their sizes.
    pub fn files_read(&self) -> Vec<(PathBuf, u64)> {
        let mut out: Vec<(PathBuf, u64)> = self.files.iter().map(|(p, (d, _))| (p.clone(), d.as_slice().len() as u64)).collect();
        out.sort();
        out
    }

    pub fn faces(&self) -> impl Iterator<Item = (FaceId, &FaceInfo)> {
        self.faces.iter().enumerate().map(|(i, f)| (i as FaceId, &f.info))
    }

    /// Increments whenever project fonts or sources change. Answers given
    /// under an older generation may be stale (a family that resolved to an
    /// installed font may now resolve to a project font); hosts drop them.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    // ------------------------------------------------------------ project fonts

    /// Registers a project font face under `family` (a `FontFace(family,
    /// data, descriptors)` added to the document). Project families take
    /// precedence over installed families of the same name, and the face is
    /// the family's variable-font source (`PM.fontSource`) as in player.ts.
    pub fn add_project_font(&mut self, family: &str, data: &[u8], index: u32, descriptors: ProjectDescriptors) -> Result<(), RegistryError> {
        let (data, blob, ct) = sanitized_web_font(data, index)?;
        self.sources.insert(family.to_string(), (data.clone(), blob.clone()));
        self.blobs.insert(blob.clone(), data.clone());
        self.project.push(ProjectFace { family: family.to_string(), descriptors, data, blob, ct });
        self.changed();
        Ok(())
    }

    /// Sets the variable-font source for `family` without registering a
    /// family (the `packagedSource` hook of `createVariableFontRenderer`).
    pub fn set_font_source(&mut self, family: &str, data: &[u8]) -> Result<(), RegistryError> {
        let (data, blob, _) = sanitized_web_font(data, 0)?;
        self.blobs.insert(blob.clone(), data.clone());
        self.sources.insert(family.to_string(), (data, blob));
        self.changed();
        Ok(())
    }

    /// Removes every project font and source (a project closed).
    pub fn clear_project_fonts(&mut self) {
        self.project.clear();
        self.sources.clear();
        self.changed();
    }

    fn changed(&mut self) {
        self.variable.clear();
        self.generation += 1;
    }

    /// `FontFaceCache::Get`: the project face for `family` at `desc`.
    fn project_face(&mut self, family: &str, desc: &FontDesc) -> Option<FaceId> {
        let candidates: Vec<usize> = (0..self.project.len()).filter(|&i| self.project[i].family.eq_ignore_ascii_case(family)).collect();
        if candidates.is_empty() {
            return None;
        }
        // Later faces with equal capabilities shadow earlier ones (the last
        // added face of a segmented face is tried first).
        let descriptors: Vec<ProjectDescriptors> = candidates.iter().rev().map(|&i| self.project[i].descriptors).collect();
        let chosen = candidates[candidates.len() - 1 - select_face(&descriptors, desc)?];
        let (bold, italic) = web_synthesis(&self.project[chosen].descriptors, desc);
        let p = &self.project[chosen];
        let (ct, data, blob, web) = (p.ct.clone(), p.data.clone(), p.blob.clone(), p.descriptors.web());
        Some(self.intern(ct, bold, italic, Some(Custom { data, index: 0, blob, web: Some(web) })))
    }

    // ------------------------------------------------------------ enumeration

    /// Installed families (`CTFontManagerCopyAvailableFontFamilyNames`)
    /// followed by project families not already listed, in order.
    pub fn family_names(&self) -> Vec<String> {
        let families = unsafe { CTFontManagerCopyAvailableFontFamilyNames() };
        // SAFETY: the array holds CFStrings.
        let families: CFRetained<CFArray<CFString>> = unsafe { CFRetained::cast_unchecked(families) };
        let mut out: Vec<String> = families.iter().map(|s| s.to_string()).collect();
        let mut seen: HashSet<String> = out.iter().map(|s| s.to_lowercase()).collect();
        for p in &self.project {
            if seen.insert(p.family.to_lowercase()) {
                out.push(p.family.clone());
            }
        }
        out
    }

    /// One family's faces and whether any of them is variable.
    pub fn family_info(&mut self, family: &str) -> Option<FamilyInfo> {
        let project: Vec<StyleInfo> = self
            .project
            .iter()
            .filter(|p| p.family.eq_ignore_ascii_case(family))
            .map(|p| StyleInfo {
                name: unsafe { p.ct.name(kCTFontStyleNameKey) }.map(|s| s.to_string()).unwrap_or_default(),
                postscript_name: unsafe { p.ct.post_script_name() }.to_string(),
                weight: p.descriptors.weight.0,
                italic: p.descriptors.style != ProjectStyle::Normal,
            })
            .collect();
        if !project.is_empty() {
            let variable = self.project.iter().filter(|p| p.family.eq_ignore_ascii_case(family)).any(|p| !crate::font::axes(p.data.as_slice(), 0).is_empty());
            return Some(FamilyInfo { name: family.to_string(), styles: project, variable, project: true });
        }
        let all = unsafe { CTFontCollection::from_available_fonts(None) };
        let list = unsafe { all.matching_font_descriptors_for_family(&cfstr(family), None) }?;
        // SAFETY: the array holds CTFontDescriptors.
        let list: CFRetained<CFArray<CTFontDescriptor>> = unsafe { CFRetained::cast_unchecked(list) };
        let mut styles = Vec::new();
        let mut variable = false;
        for desc in list.iter() {
            let (sym, weight) = descriptor_traits(&desc);
            let string = |key: &CFString| unsafe { desc.attribute(key) }.and_then(|v| v.downcast::<CFString>().ok()).map(|s| s.to_string()).unwrap_or_default();
            styles.push(StyleInfo {
                name: string(unsafe { kCTFontStyleNameAttribute }),
                postscript_name: string(unsafe { kCTFontNameAttribute }),
                weight: weight.map(to_css_weight).unwrap_or(NORMAL_WEIGHT) as f32,
                italic: sym.unwrap_or(0) & TRAIT_ITALIC != 0,
            });
            if !variable {
                variable = unsafe { desc.attribute(kCTFontVariationAxesAttribute) }.is_some();
            }
        }
        styles.sort_by(|a, b| a.weight.total_cmp(&b.weight).then(a.italic.cmp(&b.italic)));
        Some(FamilyInfo { name: family.to_string(), styles, variable, project: false })
    }

    /// The faces of `family` as font sources, for level 2 layout: a project
    /// family's faces, or the installed family's members. A hidden family
    /// CoreText does not list (the system UI font) contributes the faces
    /// Blink's matching picks for it, upright and italic.
    pub fn family_faces(&mut self, family: &str) -> Vec<FaceSource> {
        let project: Vec<FaceSource> = self
            .project
            .iter()
            .filter(|p| p.family.eq_ignore_ascii_case(family))
            .map(|p| FaceSource { blob: p.blob.clone(), index: 0, postscript_name: unsafe { p.ct.post_script_name() }.to_string() })
            .collect();
        if !project.is_empty() {
            return project;
        }
        let mut fonts: Vec<CFRetained<CTFont>> = Vec::new();
        let all = unsafe { CTFontCollection::from_available_fonts(None) };
        if let Some(list) = unsafe { all.matching_font_descriptors_for_family(&cfstr(family), None) } {
            // SAFETY: the array holds CTFontDescriptors.
            let list: CFRetained<CFArray<CTFontDescriptor>> = unsafe { CFRetained::cast_unchecked(list) };
            fonts.extend(list.iter().map(|d| unsafe { CTFont::with_font_descriptor(&d, 16.0, std::ptr::null()) }));
        }
        if fonts.is_empty() {
            for italic in [false, true] {
                let desc = FontDesc { weight: 400.0, italic, size: 16.0 };
                let found = if family == "system-ui" { self.match_system_ui(&desc) } else { self.match_font_family(family, &desc) };
                fonts.extend(found);
            }
        }
        let mut out: Vec<FaceSource> = Vec::new();
        for ct in fonts {
            let (_, index, blob) = self.font_bytes(&ct);
            let source = FaceSource { blob, index, postscript_name: unsafe { ct.post_script_name() }.to_string() };
            if !out.iter().any(|f| f.blob == source.blob && f.index == source.index) {
                out.push(source);
            }
        }
        out
    }

    /// The fallback order level 2 layout gives Parley, built from CoreText's
    /// substitutes for `MANIFEST_SAMPLES` with `base_family` as the font
    /// being substituted (`system-ui` for the system UI font).
    pub fn fallback_manifest(&mut self, base_family: &str) -> FallbackManifest {
        let mut manifest = FallbackManifest { base_family: base_family.to_string(), ..Default::default() };
        // Regular and bold bases, so the manifest carries both weights of
        // each substitute.
        let mut bases = Vec::new();
        for weight in [400.0, 700.0] {
            let desc = FontDesc { weight, italic: false, size: 16.0 };
            let base = if base_family == "system-ui" { self.match_system_ui(&desc) } else { self.match_font_family(base_family, &desc) };
            bases.extend(base);
        }
        let Some(first) = bases.first().cloned() else { return manifest };
        let base_name = unsafe { first.family_name() }.to_string();
        let mut faces: Vec<ManifestFace> = Vec::new();
        let mut add_face = |reg: &mut Self, family: &str, ct: &CTFont| {
            let (_, index, blob) = reg.font_bytes(ct);
            let source = FaceSource { blob, index, postscript_name: unsafe { ct.post_script_name() }.to_string() };
            let face = ManifestFace { family: family.to_string(), source };
            if !faces.contains(&face) {
                faces.push(face);
            }
        };
        for base in &bases {
            add_face(self, base_family, base);
        }
        for (script, language, sample) in MANIFEST_SAMPLES {
            let string = cfstr(sample);
            let range = CFRange { location: 0, length: string.length() };
            let lang = language.map(cfstr);
            let mut entry: Option<ScriptFallback> = None;
            for base in &bases {
                let font = unsafe { base.for_string_with_language(&string, range, lang.as_deref()) };
                let mut family = unsafe { font.family_name() }.to_string();
                if family.is_empty() || unsafe { font.post_script_name() }.to_string() == "LastResort" {
                    continue;
                }
                // The base covers the script itself: name it as the caller
                // does (a hidden system family has no public name).
                if family == base_name {
                    family = base_family.to_string();
                } else {
                    add_face(self, &family, &font);
                }
                entry.get_or_insert_with(|| ScriptFallback { script: script.to_string(), language: language.map(str::to_string), families: Vec::new() });
                let e = entry.as_mut().expect("entry");
                if !e.families.contains(&family) {
                    e.families.push(family);
                }
            }
            if let Some(e) = entry.filter(|e| !manifest.scripts.contains(e)) {
                manifest.scripts.push(e);
            }
        }
        for weight in [400.0, 700.0] {
            let desc = FontDesc { weight, italic: false, size: 16.0 };
            if let Some(f) = self.match_font_family("Apple Color Emoji", &desc) {
                let family = unsafe { f.family_name() }.to_string();
                add_face(self, &family, &f);
                manifest.emoji = Some(family);
            }
        }
        manifest.last_resort = self.match_font_family("Times", &FontDesc { weight: 400.0, italic: false, size: 16.0 }).map(|f| unsafe { f.family_name() }.to_string());
        manifest.faces = faces;
        manifest
    }

    // ------------------------------------------------------------ answers

    /// Answers a batch of resolver queries (a replay's misses, or the
    /// families a document names on open) with the records of every face
    /// they name. Queries naming a face this registry never handed out are
    /// refused: face ids are only meaningful to the registry that made them.
    pub fn answer(&mut self, queries: &[crate::answers::Query]) -> Result<crate::answers::Answers, RegistryError> {
        use crate::answers::Query;
        for q in queries {
            if let Query::Fallback { primary: id, .. } | Query::EmAdvance { face: id, .. } = q
                && *id as usize >= self.faces.len()
            {
                return Err(RegistryError::UnknownFace(*id));
            }
        }
        Ok(crate::answers::answer_batch(self, queries))
    }

    // ------------------------------------------------------------ bytes and subsets

    /// The bytes of a blob a face reads.
    pub fn blob(&self, key: &str) -> Option<FontBytes> {
        self.blobs.get(key).cloned()
    }

    /// The metrics-and-shaping subset of face `index` of blob `key` (cached).
    pub fn subset(&mut self, key: &str, index: u32) -> Result<Arc<Vec<u8>>, RegistryError> {
        if let Some(s) = self.subsets.get(&(key.to_string(), index)) {
            return Ok(s.clone());
        }
        let data = self.blobs.get(key).ok_or_else(|| RegistryError::UnknownBlob(key.to_string()))?;
        let (bytes, _) = crate::subset::metrics_subset(data.as_slice(), index).map_err(|source| RegistryError::Subset { blob: key.to_string(), index, source })?;
        let bytes = Arc::new(bytes);
        self.subsets.insert((key.to_string(), index), bytes.clone());
        Ok(bytes)
    }

    // ------------------------------------------------------------ faces

    fn file_bytes(&mut self, path: &std::path::Path) -> Option<(FontBytes, String)> {
        if let Some(f) = self.files.get(path) {
            return Some(f.clone());
        }
        let meta = std::fs::metadata(path).ok()?;
        let modified = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
        let key = source_key(&path.to_string_lossy(), meta.len(), modified);
        // System fonts live on the sealed system volume and are mapped;
        // other files are read, so a font replaced on disk cannot fault a
        // mapping under the registry.
        let data = if path.starts_with("/System/") {
            let file = std::fs::File::open(path).ok()?;
            // SAFETY: the sealed system volume is read-only.
            let map = unsafe { memmap2::Mmap::map(&file) }.ok()?;
            FontBytes::from_shared(Arc::new(MappedFont(map)))
        } else {
            FontBytes::new(std::fs::read(path).ok()?)
        };
        self.blobs.insert(key.clone(), data.clone());
        self.files.insert(path.to_path_buf(), (data.clone(), key.clone()));
        Some((data, key))
    }

    fn font_bytes(&mut self, ct: &CTFont) -> (FontBytes, u32, String) {
        let ps = unsafe { ct.post_script_name() }.to_string();
        let cg_ps = unsafe { ct.graphics_font(std::ptr::null_mut()) };
        let cg_ps = objc2_core_graphics::CGFont::post_script_name(Some(&cg_ps)).map(|s| s.to_string()).unwrap_or_default();
        let url = unsafe { ct.attribute(kCTFontURLAttribute) }.and_then(|u| u.downcast::<CFURL>().ok()).and_then(|u| u.to_file_path());
        if let Some(path) = url
            && let Some((data, key)) = self.file_bytes(&path)
                && let Some(index) = face_index(data.as_slice(), &[&ps, &cg_ps]) {
                    return (data, index, key);
                }
        // No file (or no matching face in it): rebuild an sfnt from the
        // tables CoreText serves, as Skia does for HarfBuzz on macOS.
        if let Some((d, key)) = self.table_fonts.get(&ps) {
            return (d.clone(), 0, key.clone());
        }
        let bytes = sfnt_from_tables(ct);
        let key = content_key(&bytes);
        let d = FontBytes::new(bytes);
        self.blobs.insert(key.clone(), d.clone());
        self.table_fonts.insert(ps, (d.clone(), key.clone()));
        (d, 0, key)
    }

    fn intern(&mut self, ct: CFRetained<CTFont>, synthetic_bold: bool, synthetic_italic: bool, custom: Option<Custom>) -> FaceId {
        let ps = unsafe { ct.post_script_name() }.to_string();
        let mut coords: Vec<(u32, f32)> = Vec::new();
        if let Some(var) = unsafe { ct.variation() } {
            let var: CFRetained<CFDictionary<CFNumber, CFNumber>> = unsafe { CFRetained::cast_unchecked(var) };
            let (keys, values) = var.to_vecs();
            for (k, v) in keys.iter().zip(values.iter()) {
                if let (Some(k), Some(v)) = (k.as_i64(), v.as_f64()) {
                    coords.push((k as u32, v as f32));
                }
            }
            coords.sort_by_key(|c| c.0);
        }
        let custom_key = custom.as_ref().map(|c| format!("custom:{}:{}:{:?}", c.blob, c.index, c.web)).unwrap_or_default();
        let typeface_key = format!("{ps}|{coords:?}{custom_key}");
        let key = format!("{typeface_key}|{synthetic_bold}|{synthetic_italic}");
        if let Some(&id) = self.by_key.get(&key) {
            return id;
        }
        let is_custom = custom.is_some();
        let (data, index, blob, web) = match custom {
            Some(c) => (c.data, c.index, c.blob, c.web),
            None => {
                let (d, i, b) = self.font_bytes(&ct);
                (d, i, b, None)
            }
        };
        let synthesized_hyphens = [hyphen_glyph(&ct, data.as_slice(), index, 0x2010), hyphen_glyph(&ct, data.as_slice(), index, 0x2011)];
        let fallback_key = fallback_identifier(&ct);
        let facts = FaceFacts::of(data.as_slice(), index);
        let info = FaceInfo {
            data,
            index,
            blob,
            coords,
            synthetic_bold,
            synthetic_italic,
            custom: is_custom,
            web,
            synthesized_hyphens,
            postscript_name: ps,
            typeface_key,
            fallback_key,
            facts,
        };
        let id = self.faces.len() as FaceId;
        self.faces.push(Face { info, ct });
        self.by_key.insert(key, id);
        id
    }

    /// `FontCache::CreateFontPlatformData` for a family name.
    fn create_platform_data(&mut self, family: &str, desc: &FontDesc) -> Option<FaceId> {
        if family.starts_with('.') {
            return None;
        }
        let ct = if family == "system-ui" { self.match_system_ui(desc) } else { self.match_font_family(family, desc) }?;
        let traits = symbolic(&ct);
        let synthetic_bold = desc.weight > 500.0 && traits & TRAIT_BOLD == 0;
        let synthetic_italic = desc.italic && traits & TRAIT_ITALIC == 0;
        Some(self.intern(ct, synthetic_bold, synthetic_italic, None))
    }

    /// `FontPlatformDataCache::GetOrCreateFontPlatformData` with alternates.
    fn get_font_data(&mut self, family: &str, desc: &FontDesc, allow_alternate: bool) -> Option<FaceId> {
        let key = (family.to_string(), desc.weight.to_bits(), desc.italic, desc.size.to_bits());
        if let Some(r) = self.family_cache.get(&key)
            && (r.is_some() || !allow_alternate) {
                return *r;
            }
        let mut result = self.create_platform_data(family, desc);
        self.family_cache.insert(key.clone(), result);
        if result.is_none() && allow_alternate
            && let Some(alt) = alternate_family(family) {
                result = self.get_font_data(alt, desc, false);
                if result.is_some() {
                    self.family_cache.insert(key, result);
                }
            }
        result
    }

    /// The installed variable-font source `inspectFont(family)` finds: the
    /// local fonts whose family, full name, or PostScript name equals
    /// `family` (trimmed, unquoted, case-insensitive), in PostScript-name
    /// order with "Regular" faces first; the first whose file has an `fvar`
    /// table. The whole file is the `FontFace` source (face 0 of a collection).
    fn installed_variable_source(&mut self, family: &str) -> Option<(FontBytes, String, CFRetained<CTFont>)> {
        let normalize = |s: &str| s.trim().trim_matches(|c| c == '"' || c == '\'').to_lowercase();
        let key = normalize(family);
        if key.is_empty() {
            return None;
        }
        let all = unsafe { CTFontCollection::from_available_fonts(None) };
        let list = unsafe { all.matching_font_descriptors() }?;
        // SAFETY: the array holds CTFontDescriptors.
        let list: CFRetained<CFArray<CTFontDescriptor>> = unsafe { CFRetained::cast_unchecked(list) };
        let mut candidates: Vec<(String, bool, PathBuf)> = Vec::new();
        for desc in list.iter() {
            let string = |k: &CFString| unsafe { desc.attribute(k) }.and_then(|v| v.downcast::<CFString>().ok()).map(|s| s.to_string());
            let ps = string(unsafe { kCTFontNameAttribute }).unwrap_or_default();
            let names = [string(unsafe { kCTFontFamilyNameAttribute }), string(unsafe { kCTFontDisplayNameAttribute }), Some(ps.clone())];
            if !names.iter().flatten().any(|n| normalize(n) == key) {
                continue;
            }
            let style = string(unsafe { kCTFontStyleNameAttribute }).or_else(|| string(unsafe { kCTFontDisplayNameAttribute })).unwrap_or_default();
            let Some(path) = unsafe { desc.attribute(kCTFontURLAttribute) }.and_then(|u| u.downcast::<CFURL>().ok()).and_then(|u| u.to_file_path()) else { continue };
            candidates.push((ps, style.to_lowercase().contains("regular"), path));
        }
        candidates.sort_by(|a, b| a.0.cmp(&b.0));
        candidates.sort_by_key(|c| !c.1);
        for (_, _, path) in candidates {
            let Some((data, _)) = self.file_bytes(&path) else { continue };
            let file = match read_fonts::FileRef::new(data.as_slice()) {
                Ok(f) => f,
                Err(_) => continue,
            };
            let variable = file.fonts().enumerate().any(|(i, _)| !crate::font::axes(data.as_slice(), i as u32).is_empty());
            if !variable {
                continue;
            }
            let Ok((data, blob, ct)) = sanitized_web_font(data.as_slice(), 0) else { continue };
            return Some((data, blob, ct));
        }
        None
    }

    // ------------------------------------------------------------ matching

    /// `BestStyleMatchForFamilyNS`.
    fn best_style_match_ns(&self, family: &str, desired_traits: u32, desired_weight: i32, size: f64) -> Option<CFRetained<CTFont>> {
        let fonts: Option<Retained<NSArray<NSArray>>> = self.fm.availableMembersOfFontFamily(&NSString::from_str(family));
        let fonts = match fonts {
            Some(f) if f.count() > 0 => f,
            _ => return self.best_style_match_ct(family, desired_traits, desired_weight, size),
        };
        let mut matched: Option<String> = None;
        let mut chosen_traits = 0u32;
        let mut chosen_weight = 0i32;
        for info in fonts.iter() {
            let Some(name) = info.objectAtIndex(0).downcast::<NSString>().ok() else { continue };
            let name = name.to_string();
            let mut cand_weight = NORMAL_WEIGHT;
            let mut cand_traits = 0u32;
            let mut got = false;
            let desc = unsafe { CTFontDescriptor::with_name_and_size(&cfstr(&name), 0.0) };
            let (sym, weight) = descriptor_traits(&desc);
            if let Some(w) = weight {
                cand_weight = to_css_weight(w);
                if let Some(s) = sym {
                    cand_traits = s & IMPORTANT_TRAITS;
                }
                got = true;
            }
            if !got {
                if let Ok(w) = info.objectAtIndex(2).downcast::<NSNumber>() {
                    cand_weight = appkit_to_css_weight(w.intValue());
                }
                if let Ok(t) = info.objectAtIndex(3).downcast::<NSNumber>() {
                    cand_traits = (t.intValue() as u32) & IMPORTANT_TRAITS;
                }
            }
            if matched.is_none() || better_choice_ct(desired_traits, desired_weight, chosen_traits, chosen_weight, cand_traits, cand_weight) {
                matched = Some(name);
                chosen_traits = cand_traits;
                chosen_weight = cand_weight;
                if chosen_weight == desired_weight && (chosen_traits & IMPORTANT_TRAITS) == (desired_traits & IMPORTANT_TRAITS) {
                    break;
                }
            }
        }
        let name = matched?;
        Some(unsafe { CTFont::with_name(&cfstr(&name), size, std::ptr::null()) })
    }

    /// `BestStyleMatchForFamily` (CoreText collection path).
    fn best_style_match_ct(&self, family: &str, desired_traits: u32, desired_weight: i32, size: f64) -> Option<CFRetained<CTFont>> {
        let all = unsafe { CTFontCollection::from_available_fonts(None) };
        let list = unsafe { all.matching_font_descriptors_for_family(&cfstr(family), None) }?;
        // SAFETY: the array holds CTFontDescriptors.
        let list: CFRetained<CFArray<CTFontDescriptor>> = unsafe { CFRetained::cast_unchecked(list) };
        if list.is_empty() {
            return None;
        }
        let mut matched: Option<CFRetained<CTFont>> = None;
        let (mut chosen_traits, mut chosen_weight) = (0u32, 0i32);
        for desc in list.iter() {
            let (sym, weight) = descriptor_traits(&desc);
            let cand_traits = sym.unwrap_or(0);
            let cand_weight = weight.map(to_css_weight).unwrap_or(NORMAL_WEIGHT);
            if matched.is_none() || better_choice_ct(desired_traits, desired_weight, chosen_traits, chosen_weight, cand_traits, cand_weight) {
                matched = Some(unsafe { CTFont::with_font_descriptor(&desc, size, std::ptr::null()) });
                chosen_traits = cand_traits;
                chosen_weight = cand_weight;
                if chosen_weight == desired_weight && (chosen_traits & IMPORTANT_TRAITS) == (desired_traits & IMPORTANT_TRAITS) {
                    return matched;
                }
            }
        }
        matched
    }

    /// `MatchByPostscriptNameNS`.
    fn match_by_postscript_ns(&self, family: &str, size: f64) -> Option<Retained<NSFont>> {
        let lower = family.to_lowercase();
        for name in self.fm.availableFonts().iter() {
            if name.to_string().to_lowercase() == lower {
                return NSFont::fontWithName_size(&name, size);
            }
        }
        None
    }

    fn desired_traits(desc: &FontDesc) -> u32 {
        let mut t = 0;
        if desc.weight >= BOLD_THRESHOLD {
            t |= TRAIT_BOLD;
        }
        if desc.italic {
            t |= TRAIT_ITALIC;
        }
        t
    }

    /// `MatchFontFamily` with the feature flags Chromium 152 ships with
    /// (AppKit style matching and AppKit PostScript matching).
    fn match_font_family(&mut self, family: &str, desc: &FontDesc) -> Option<CFRetained<CTFont>> {
        if family.is_empty() || self.unavailable.contains(family) {
            return None;
        }
        if family.eq_ignore_ascii_case("LastResort") {
            return None;
        }
        let size = desc.size as f64;
        let desired_traits = Self::desired_traits(desc);
        let desired_weight = desc.weight as i32;
        let mut found = self.best_style_match_ns(family, desired_traits, desired_weight, size);
        if found.is_none()
            && let Some(ps) = self.match_by_postscript_ns(family, size) {
                // NSFontTraitMask: italic 1, bold 2 (same bits as CoreText for these).
                let desired_ns = desired_traits & (TRAIT_ITALIC | TRAIT_BOLD);
                let traits = self.fm.traitsOfFont(&ps).0 as u32;
                if traits & desired_ns == desired_ns {
                    let converted = self.fm.convertFont_toHaveTrait(&ps, objc2_app_kit::NSFontTraitMask(desired_ns as _));
                    return Some(nsfont_to_ct(&converted));
                }
                let fam = ps.familyName().map(|s| s.to_string()).unwrap_or_default();
                return self.best_style_match_ns(&fam, desired_traits, desired_weight, size);
            }
        if found.is_none() {
            self.unavailable.insert(family.to_string());
        }
        found.take()
    }

    /// `MatchSystemUIFont`.
    fn match_system_ui(&self, desc: &FontDesc) -> Option<CFRetained<CTFont>> {
        let size = desc.size as f64;
        let mut font = unsafe { CTFont::new_ui_font_for_language(CTFontUIFontType::System, size, None) }?;
        let mut traits = 0u32;
        if desc.italic {
            traits |= TRAIT_ITALIC;
        }
        if desc.weight >= BOLD_THRESHOLD {
            traits |= TRAIT_BOLD;
        }
        if traits != 0
            && let Some(f) = unsafe { font.copy_with_symbolic_traits(size, std::ptr::null(), CTFontSymbolicTraits(traits), CTFontSymbolicTraits(traits)) } {
                font = f;
            }
        let mut weight = desc.weight;
        if weight == NORMAL_WEIGHT as f32 {
            return Some(font);
        }
        // ClampVariationValuesToFontAcceptableRange (weight only; width is normal).
        if let Some(axes) = unsafe { font.variation_axes() } {
            let axes: CFRetained<CFArray<CFDictionary<CFString, CFType>>> = unsafe { CFRetained::cast_unchecked(axes) };
            for axis in axes.iter() {
                let id = axis.get(unsafe { kCTFontVariationAxisIdentifierKey }).and_then(|v| num_i64(&v));
                let min = axis.get(unsafe { kCTFontVariationAxisMinimumValueKey }).and_then(|v| num_f32(&v));
                let max = axis.get(unsafe { kCTFontVariationAxisMaximumValueKey }).and_then(|v| num_f32(&v));
                if let (Some(id), Some(min), Some(max)) = (id, min, max)
                    && id as u32 == u32::from_be_bytes(*b"wght") {
                        weight = weight.clamp(min, max);
                    }
            }
        }
        let wght = CFNumber::new_i64(u32::from_be_bytes(*b"wght") as i64);
        let value = CFNumber::new_f32(weight);
        let variations: CFRetained<CFDictionary<CFNumber, CFNumber>> = CFDictionary::from_slices(&[&*wght], &[&*value]);
        let attrs: CFRetained<CFDictionary<CFString, CFType>> =
            CFDictionary::from_slices(&[unsafe { kCTFontVariationAttribute }], &[variations.as_ref() as &CFType]);
        let var_desc = unsafe { CTFontDescriptor::with_attributes(attrs.as_opaque()) };
        Some(unsafe { font.copy_with_attributes(size, std::ptr::null(), Some(&var_desc)) })
    }

    /// `GetAlternateFontPlatformData` and `GetSubstituteFont`.
    fn alternate_for_char(&mut self, desc: &FontDesc, ch: u32, primary: FaceId) -> Option<FaceId> {
        // Blink builds the query string with CFStringCreateWithBytes(UTF-32),
        // which rejects surrogate code points; CTFontCreateForString then
        // returns NULL and there is no substitute.
        if (0xD800..=0xDFFF).contains(&ch) || ch > 0x10FFFF {
            return None;
        }
        let size = desc.size as f64;
        let base = self.faces[primary as usize].ct.clone();
        let base_info = &self.faces[primary as usize].info;
        let (base_bold, base_italic) = (base_info.synthetic_bold, base_info.synthetic_italic);
        let string = char_string(ch);
        let range = CFRange { location: 0, length: string.length() };
        let mut substitute = unsafe { base.for_string(&string, range) };
        if unsafe { substitute.post_script_name() }.to_string() == "LastResort" {
            return None;
        }
        let fam = unsafe { substitute.name(kCTFontFamilyNameKey) }.map(|s| s.to_string()).unwrap_or_default();
        let is_color_emoji = fam.eq_ignore_ascii_case("Apple Color Emoji") || fam.eq_ignore_ascii_case(".Apple Color Emoji UI");
        if is_color_emoji && crate::blink::chars::is_emoji(ch) {
            let langs: CFRetained<CFArray<CFString>> = CFArray::from_retained_objects(&[cfstr("en")]);
            if let Some(cascade) = unsafe { substitute.default_cascade_list_for_languages(Some(langs.as_opaque())) } {
                let name = cfstr("Apple Symbols");
                let attrs: CFRetained<CFDictionary<CFString, CFType>> = CFDictionary::from_slices(
                    &[unsafe { kCTFontNameAttribute }, unsafe { kCTFontCascadeListAttribute }],
                    &[name.as_ref() as &CFType, cascade.as_ref() as &CFType],
                );
                let mono_desc = unsafe { CTFontDescriptor::with_attributes(attrs.as_opaque()) };
                let mono = unsafe { CTFont::with_font_descriptor(&mono_desc, size, std::ptr::null()) };
                substitute = unsafe { mono.for_string(&string, range) };
            }
        }
        let mut traits = symbolic(&base);
        if base_bold {
            traits |= TRAIT_BOLD;
        }
        if base_italic {
            traits |= TRAIT_ITALIC;
        }
        let weight = to_ct_weight(desc.weight as i32);
        let mut sub_traits = symbolic(&substitute);
        let sub_weight = ct_font_weight(&substitute);
        let mask = IMPORTANT_TRAITS;
        if ((traits & mask) != (sub_traits & mask) || weight != sub_weight)
            && let Some(best) = copy_with_traits_and_weight(&substitute, traits, weight, size) {
                let best_traits = symbolic(&best);
                let best_weight = ct_font_weight(&best);
                let charset: CFRetained<CFCharacterSet> = unsafe { best.character_set() };
                if (best_traits != sub_traits || best_weight != sub_weight) && charset.is_long_character_member(ch) {
                    substitute = best;
                    sub_traits = symbolic(&substitute);
                }
            }
        let synthetic_bold = traits & TRAIT_BOLD != 0 && sub_traits & TRAIT_BOLD == 0;
        let synthetic_italic = traits & TRAIT_ITALIC != 0 && sub_traits & TRAIT_ITALIC == 0;
        Some(self.intern(substitute, synthetic_bold, synthetic_italic, None))
    }
}

/// A face registered from bytes (a project font or a variable `FontFace`).
struct Custom {
    data: FontBytes,
    index: u32,
    blob: String,
    web: Option<WebDescriptors>,
}

/// A read-only mapping of a system font file.
struct MappedFont(memmap2::Mmap);

impl AsRef<[u8]> for MappedFont {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Sanitizes web font data as Blink does (OTS table selection), and creates
/// the CTFont Blink's platform data would hold for it.
fn sanitized_web_font(data: &[u8], index: u32) -> Result<(FontBytes, String, CFRetained<CTFont>), RegistryError> {
    let clean = crate::sanitize::sanitize(data, index).ok_or(RegistryError::NotAFont)?;
    let cfdata = CFData::from_bytes(&clean);
    let desc = unsafe { CTFontManagerCreateFontDescriptorFromData(&cfdata) }.ok_or(RegistryError::NotAFont)?;
    let ct = unsafe { CTFont::with_font_descriptor(&desc, 16.0, std::ptr::null()) };
    let blob = content_key(&clean);
    Ok((FontBytes::new(clean), blob, ct))
}

impl Default for CoreTextRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FontResolver for CoreTextRegistry {
    fn face(&self, id: FaceId) -> &FaceInfo {
        &self.faces[id as usize].info
    }

    fn family(&mut self, family: &str, desc: &FontDesc) -> Option<FaceId> {
        if let Some(f) = self.project_face(family, desc) {
            return Some(f);
        }
        self.get_font_data(family, desc, true)
    }

    fn standard(&mut self, desc: &FontDesc) -> Option<FaceId> {
        // Chrome's macOS default standard font (locale_settings_mac.grd).
        self.get_font_data("Times", desc, true)
    }

    fn fallback_for_char(&mut self, desc: &FontDesc, ch: u32, primary: FaceId, emoji_presentation: bool) -> Option<FaceId> {
        if emoji_presentation {
            match self.get_font_data("Apple Color Emoji", desc, true) {
                Some(f) => Some(f),
                None => self.alternate_for_char(desc, ch, primary),
            }
        } else {
            self.alternate_for_char(desc, ch, primary)
        }
    }

    fn last_resort(&mut self, desc: &FontDesc) -> Option<FaceId> {
        self.get_font_data("Times", desc, true).or_else(|| self.get_font_data("Lucida Grande", desc, true))
    }

    fn variable_face(&mut self, family: &str) -> Option<FaceId> {
        if let Some(v) = self.variable.get(family) {
            return *v;
        }
        let source = match self.sources.get(family) {
            Some((data, blob)) => {
                let (data, blob) = (data.clone(), blob.clone());
                let cfdata = CFData::from_bytes(data.as_slice());
                unsafe { CTFontManagerCreateFontDescriptorFromData(&cfdata) }
                    .map(|desc| (data, blob, unsafe { CTFont::with_font_descriptor(&desc, 16.0, std::ptr::null()) }))
            }
            None => self.installed_variable_source(family),
        };
        let face = source.map(|(data, blob, ct)| {
            self.blobs.insert(blob.clone(), data.clone());
            self.intern(ct, false, false, Some(Custom { data, index: 0, blob, web: None }))
        });
        self.variable.insert(family.to_string(), face);
        face
    }

    fn em_advance(&mut self, face: FaceId, size: f32) -> Option<f32> {
        let key = (face, size.to_bits());
        if let Some(v) = self.em_advances.get(&key) {
            return *v;
        }
        let f = &self.faces[face as usize];
        let v = sbix_em_advance(&f.ct, f.info.data.as_slice(), f.info.index, size);
        self.em_advances.insert(key, v);
        v
    }
}

/// `BuildIdentifierKey` (character_fallback_cache.mm).
fn fallback_identifier(ct: &CTFont) -> Option<String> {
    let ps = unsafe { ct.post_script_name() }.to_string();
    if ps.is_empty() {
        return None;
    }
    if !ps.starts_with('.') {
        return Some(ps);
    }
    let desc = unsafe { ct.font_descriptor() };
    let attrs = unsafe { desc.attributes() };
    let attrs: &CFDictionary<CFString, CFType> = unsafe { attrs.cast_unchecked() };
    let usage = attrs.get(&cfstr("NSCTFontUIUsageAttribute"))?.downcast::<CFString>().ok()?.to_string();
    let lang = attrs.get(&cfstr("CTFontDescriptorLanguageAttribute"))?.downcast::<CFString>().ok()?.to_string();
    Some(format!("UIFONTUSAGE:{usage}-LANG:{lang}"))
}

/// CoreText's advance for a full-em glyph of a color (`sbix`) font, created
/// the way Skia's scaler creates its CTFont (`SkCTFontCreateExactCopy`:
/// optical size = the base font's size, tracking off).
fn sbix_em_advance(ct: &CTFont, data: &[u8], index: u32, size: f32) -> Option<f32> {
    use read_fonts::{FontRef, TableProvider};
    let font = FontRef::from_index(data, index).ok()?;
    let upem = font.head().ok()?.units_per_em();
    let hmtx = font.hmtx().ok()?;
    // A glyph whose advance is exactly one em.
    let n = font.maxp().ok()?.num_glyphs();
    let glyph = (0..n).find(|&g| hmtx.advance(read_fonts::types::GlyphId::new(g as u32)) == Some(upem))?;
    let base = unsafe { ct.copy_with_attributes(size as f64, std::ptr::null(), None) };
    let opsz = CFNumber::new_f64(size as f64);
    let zero = CFNumber::new_i32(0);
    let attrs: CFRetained<CFDictionary<CFString, CFType>> = CFDictionary::from_slices(
        &[&*cfstr("NSCTFontOpticalSizeAttribute"), &*cfstr("NSCTFontUnscaledTrackingAttribute")],
        &[opsz.as_ref() as &CFType, zero.as_ref() as &CFType],
    );
    let desc = unsafe { CTFontDescriptor::with_attributes(attrs.as_opaque()) };
    let exact = unsafe { base.copy_with_attributes(size as f64, std::ptr::null(), Some(&desc)) };
    let glyphs = [glyph];
    let mut adv = [objc2_core_foundation::CGSize { width: 0.0, height: 0.0 }];
    unsafe {
        exact.advances_for_glyphs(
            objc2_core_text::CTFontOrientation::Horizontal,
            std::ptr::NonNull::new(glyphs.as_ptr() as *mut u16).unwrap(),
            adv.as_mut_ptr(),
            1,
        )
    };
    Some(adv[0].width as f32)
}

fn nsfont_to_ct(f: &NSFont) -> CFRetained<CTFont> {
    // NSFont is toll-free bridged to CTFont.
    let ptr = f as *const NSFont as *const CTFont;
    // SAFETY: toll-free bridging; retain to produce an owned reference.
    unsafe { CFRetained::retain(std::ptr::NonNull::new_unchecked(ptr as *mut CTFont)) }
}

fn alternate_family(family: &str) -> Option<&'static str> {
    let f = family.to_ascii_lowercase();
    match f.as_str() {
        "courier" => Some("Courier New"),
        "courier new" => Some("Courier"),
        "times" => Some("Times New Roman"),
        "times new roman" => Some("Times"),
        "arial" => Some("Helvetica"),
        "helvetica" => Some("Arial"),
        _ => None,
    }
}

fn char_string(ch: u32) -> CFRetained<CFString> {
    let s = char::from_u32(ch).map(|c| c.to_string()).unwrap_or_else(|| "\u{FFFD}".to_string());
    cfstr(&s)
}

/// `CreateCopyWithTraitsAndWeightFromFont`.
fn copy_with_traits_and_weight(font: &CTFont, traits: u32, weight: f32, size: f64) -> Option<CFRetained<CTFont>> {
    let family = unsafe { font.family_name() };
    if family.length() == 0 {
        return None;
    }
    let sym = CFNumber::new_i64(traits as i64);
    let w = CFNumber::new_f32(weight);
    let traits_dict: CFRetained<CFDictionary<CFString, CFType>> =
        CFDictionary::from_slices(&[unsafe { kCTFontSymbolicTrait }, unsafe { kCTFontWeightTrait }], &[sym.as_ref() as &CFType, w.as_ref() as &CFType]);
    let attrs: CFRetained<CFDictionary<CFString, CFType>> = CFDictionary::from_slices(
        &[unsafe { kCTFontFamilyNameAttribute }, unsafe { kCTFontTraitsAttribute }],
        &[family.as_ref() as &CFType, traits_dict.as_ref() as &CFType],
    );
    let desc = unsafe { CTFontDescriptor::with_attributes(attrs.as_opaque()) };
    Some(unsafe { CTFont::with_font_descriptor(&desc, size, std::ptr::null()) })
}

/// Index of the face whose PostScript name (name ID 6) matches one of `names`.
fn face_index(data: &[u8], names: &[&str]) -> Option<u32> {
    use read_fonts::{FileRef, TableProvider};
    let file = FileRef::new(data).ok()?;
    let fonts: Vec<_> = file.fonts().collect();
    for (i, f) in fonts.iter().enumerate() {
        let Ok(f) = f else { continue };
        let Ok(name) = f.name() else { continue };
        for rec in name.name_record() {
            if rec.name_id().to_u16() != 6 {
                continue;
            }
            if let Ok(s) = rec.string(name.string_data()) {
                let s = s.to_string();
                if names.iter().any(|n| *n == s) {
                    return Some(i as u32);
                }
            }
        }
    }
    if fonts.len() == 1 { Some(0) } else { None }
}

/// The glyph CoreText maps a hyphen code point to when the cmap lacks it
/// (Skia's `unicharToGlyph`), or `None` when the cmap has it.
fn hyphen_glyph(ct: &CTFont, data: &[u8], index: u32, ch: u32) -> Option<u16> {
    use read_fonts::{FontRef, TableProvider};
    let font = FontRef::from_index(data, index).ok()?;
    if font.cmap().ok().and_then(|c| c.map_codepoint(ch)).is_some_and(|g| g.to_u32() != 0) {
        return None;
    }
    let units = [ch as u16];
    let mut glyph = [0u16; 1];
    let ok = unsafe { ct.glyphs_for_characters(std::ptr::NonNull::new(units.as_ptr() as *mut u16).unwrap(), std::ptr::NonNull::new(glyph.as_mut_ptr()).unwrap(), 1) };
    if ok && glyph[0] != 0 { Some(glyph[0]) } else { None }
}

/// Builds an sfnt from `CTFontCopyTable` for every available table.
fn sfnt_from_tables(ct: &CTFont) -> Vec<u8> {
    let tags = unsafe { ct.available_tables(objc2_core_text::CTFontTableOptions(0)) };
    let mut tables: Vec<(u32, Vec<u8>)> = Vec::new();
    if let Some(tags) = tags {
        for i in 0..tags.len() {
            // The array stores tags as raw integers, not CF objects.
            let raw = unsafe { tags.value_at_index(i as isize) } as usize as u32;
            if let Some(data) = unsafe { ct.table(raw, CTFontTableOptions(0)) } {
                tables.push((raw, data.to_vec()));
            }
        }
    }
    build_sfnt(&mut tables)
}
