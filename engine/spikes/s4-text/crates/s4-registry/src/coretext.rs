use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSFont, NSFontManager};
use objc2_core_foundation::{
    CFArray, CFCharacterSet, CFData, CFDictionary, CFNumber, CFRange, CFRetained, CFString, CFType, CFURL,
};
use objc2_core_text::{
    CTFont, CTFontCollection, CTFontDescriptor, CTFontManagerCreateFontDescriptorFromData, CTFontSymbolicTraits,
    CTFontTableOptions, CTFontUIFontType, kCTFontCascadeListAttribute, kCTFontFamilyNameAttribute,
    kCTFontFamilyNameKey, kCTFontNameAttribute, kCTFontSymbolicTrait, kCTFontTraitsAttribute, kCTFontURLAttribute,
    kCTFontVariationAttribute, kCTFontVariationAxisIdentifierKey, kCTFontVariationAxisMaximumValueKey,
    kCTFontVariationAxisMinimumValueKey, kCTFontWeightTrait,
};
use objc2_foundation::{NSArray, NSNumber, NSString};
use s4_layout::font::{FaceId, FaceInfo, FontBytes, FontDesc, FontResolver};
use s4_layout::replay::{Answer, Query, ResolverLog};
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

pub struct CoreTextRegistry {
    fm: Retained<NSFontManager>,
    faces: Vec<Face>,
    by_key: HashMap<String, FaceId>,
    unavailable: HashSet<String>,
    family_cache: HashMap<(String, u32, bool, u32), Option<FaceId>>,
    bytes: HashMap<PathBuf, FontBytes>,
    table_bytes: HashMap<String, FontBytes>,
    /// Every query and its answer, for `ReplayResolver`.
    pub log: ResolverLog,
}

impl CoreTextRegistry {
    /// Must run on the main thread (AppKit's shared font manager).
    pub fn new() -> Self {
        let mtm = MainThreadMarker::new().expect("CoreTextRegistry must be created on the main thread");
        CoreTextRegistry {
            fm: NSFontManager::sharedFontManager(mtm),
            faces: Vec::new(),
            by_key: HashMap::new(),
            unavailable: HashSet::new(),
            family_cache: HashMap::new(),
            bytes: HashMap::new(),
            table_bytes: HashMap::new(),
            log: ResolverLog::default(),
        }
    }

    pub fn faces(&self) -> impl Iterator<Item = (FaceId, &FaceInfo)> {
        self.faces.iter().enumerate().map(|(i, f)| (i as FaceId, &f.info))
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
        if found.is_none() {
            if let Some(ps) = self.match_by_postscript_ns(family, size) {
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
        if traits != 0 {
            if let Some(f) = unsafe { font.copy_with_symbolic_traits(size, std::ptr::null(), CTFontSymbolicTraits(traits), CTFontSymbolicTraits(traits)) } {
                font = f;
            }
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
                if let (Some(id), Some(min), Some(max)) = (id, min, max) {
                    if id as u32 == u32::from_be_bytes(*b"wght") {
                        weight = weight.clamp(min, max);
                    }
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

    // ------------------------------------------------------------ faces

    fn font_bytes(&mut self, ct: &CTFont) -> (FontBytes, u32) {
        let ps = unsafe { ct.post_script_name() }.to_string();
        let cg_ps = unsafe { ct.graphics_font(std::ptr::null_mut()) };
        let cg_ps = objc2_core_graphics::CGFont::post_script_name(Some(&cg_ps)).map(|s| s.to_string()).unwrap_or_default();
        let url = unsafe { ct.attribute(kCTFontURLAttribute) }.and_then(|u| u.downcast::<CFURL>().ok()).and_then(|u| u.to_file_path());
        if let Some(path) = url {
            let data = match self.bytes.get(&path) {
                Some(d) => Some(d.clone()),
                None => std::fs::read(&path).ok().map(|v| {
                    let d = FontBytes(Arc::new(v));
                    self.bytes.insert(path.clone(), d.clone());
                    d
                }),
            };
            if let Some(data) = data {
                if let Some(index) = face_index(data.as_slice(), &[&ps, &cg_ps]) {
                    return (data, index);
                }
            }
        }
        // No file (or no matching face in it): rebuild an sfnt from the
        // tables CoreText serves, as Skia does for HarfBuzz on macOS.
        if let Some(d) = self.table_bytes.get(&ps) {
            return (d.clone(), 0);
        }
        let d = FontBytes(Arc::new(sfnt_from_tables(ct)));
        self.table_bytes.insert(ps, d.clone());
        (d, 0)
    }

    fn intern(&mut self, ct: CFRetained<CTFont>, synthetic_bold: bool, synthetic_italic: bool, custom: Option<(FontBytes, u32)>) -> FaceId {
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
        let custom_key = custom.as_ref().map(|(d, i)| format!("custom:{:p}:{i}", Arc::as_ptr(&d.0))).unwrap_or_default();
        let typeface_key = format!("{ps}|{coords:?}{custom_key}");
        let key = format!("{typeface_key}|{synthetic_bold}|{synthetic_italic}");
        if let Some(&id) = self.by_key.get(&key) {
            return id;
        }
        let (data, index) = match custom {
            Some(c) => c,
            None => self.font_bytes(&ct),
        };
        let synthesized_hyphens = [hyphen_glyph(&ct, data.as_slice(), index, 0x2010), hyphen_glyph(&ct, data.as_slice(), index, 0x2011)];
        let fallback_key = fallback_identifier(&ct);
        let info = FaceInfo {
            data,
            index,
            coords,
            synthetic_bold,
            synthetic_italic,
            custom: !custom_key.is_empty(),
            synthesized_hyphens,
            postscript_name: ps,
            typeface_key,
            fallback_key,
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
        if let Some(r) = self.family_cache.get(&key) {
            if r.is_some() || !allow_alternate {
                return *r;
            }
        }
        let mut result = self.create_platform_data(family, desc);
        self.family_cache.insert(key.clone(), result);
        if result.is_none() && allow_alternate {
            if let Some(alt) = alternate_family(family) {
                result = self.get_font_data(alt, desc, false);
                if result.is_some() {
                    self.family_cache.insert(key, result);
                }
            }
        }
        result
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
        if is_color_emoji && s4_layout::blink::chars::is_emoji(ch) {
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
        if (traits & mask) != (sub_traits & mask) || weight != sub_weight {
            if let Some(best) = copy_with_traits_and_weight(&substitute, traits, weight, size) {
                let best_traits = symbolic(&best);
                let best_weight = ct_font_weight(&best);
                let charset: CFRetained<CFCharacterSet> = unsafe { best.character_set() };
                if (best_traits != sub_traits || best_weight != sub_weight) && charset.is_long_character_member(ch) {
                    substitute = best;
                    sub_traits = symbolic(&substitute);
                }
            }
        }
        let synthetic_bold = traits & TRAIT_BOLD != 0 && sub_traits & TRAIT_BOLD == 0;
        let synthetic_italic = traits & TRAIT_ITALIC != 0 && sub_traits & TRAIT_ITALIC == 0;
        Some(self.intern(substitute, synthetic_bold, synthetic_italic, None))
    }

    fn record(&mut self, q: Query, a: Option<FaceId>) -> Option<FaceId> {
        self.log.answers.insert(q, Answer { face: a });
        a
    }
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
        let a = self.get_font_data(family, desc, true);
        self.record(Query::family(family, desc), a)
    }

    fn standard(&mut self, desc: &FontDesc) -> Option<FaceId> {
        // Chrome's macOS default standard font (locale_settings_mac.grd).
        let a = self.get_font_data("Times", desc, true);
        self.record(Query::standard(desc), a)
    }

    fn fallback_for_char(&mut self, desc: &FontDesc, ch: u32, primary: FaceId, emoji_presentation: bool) -> Option<FaceId> {
        let a = if emoji_presentation {
            match self.get_font_data("Apple Color Emoji", desc, true) {
                Some(f) => Some(f),
                None => self.alternate_for_char(desc, ch, primary),
            }
        } else {
            self.alternate_for_char(desc, ch, primary)
        };
        self.record(Query::fallback(desc, ch, primary, emoji_presentation), a)
    }

    fn last_resort(&mut self, desc: &FontDesc) -> Option<FaceId> {
        let a = self.get_font_data("Times", desc, true).or_else(|| self.get_font_data("Lucida Grande", desc, true));
        self.record(Query::last_resort(desc), a)
    }

    fn em_advance(&mut self, face: FaceId, size: f32) -> Option<f32> {
        let key = (face, size.to_bits());
        if let Some(v) = self.log.em_advances.get(&key) {
            return *v;
        }
        let v = sbix_em_advance(&self.faces[face as usize].ct, self.faces[face as usize].info.data.as_slice(), self.faces[face as usize].info.index, size);
        self.log.em_advances.insert(key, v);
        v
    }

    fn register_custom(&mut self, data: FontBytes, index: u32) -> FaceId {
        // Blink sanitizes web fonts with OTS before CoreText sees them.
        let data = FontBytes(Arc::new(s4_layout::ots::sanitize(data.as_slice(), index).expect("custom font data is not a font")));
        let index = 0;
        let cfdata = CFData::from_bytes(data.as_slice());
        let desc = unsafe { CTFontManagerCreateFontDescriptorFromData(&cfdata) }.expect("custom font data is not a font");
        let ct = unsafe { CTFont::with_font_descriptor(&desc, 16.0, std::ptr::null()) };
        let id = self.intern(ct, false, false, Some((data, index)));
        self.log.custom.push(id);
        id
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
            let raw = unsafe { objc2_core_foundation::CFArrayGetValueAtIndex(&tags, i as isize) } as usize as u32;
            if let Some(data) = unsafe { ct.table(raw, CTFontTableOptions(0)) } {
                tables.push((raw, data.to_vec()));
            }
        }
    }
    build_sfnt(&mut tables)
}

pub(crate) fn build_sfnt(tables: &mut [(u32, Vec<u8>)]) -> Vec<u8> {
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
    out.extend_from_slice(&(if is_cff { 0x4F54544Fu32 } else { 0x00010000u32 }).to_be_bytes());
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&entry_selector.to_be_bytes());
    out.extend_from_slice(&range_shift.to_be_bytes());
    let mut offset = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, data) in tables.iter() {
        let sum = checksum(data);
        out.extend_from_slice(&tag.to_be_bytes());
        out.extend_from_slice(&sum.to_be_bytes());
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
