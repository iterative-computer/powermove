//! The CoreText reference rasterizer (macOS): glyphs through
//! `CTFontDrawGlyphs` into a 32-bit float premultiplied sRGB bitmap, with
//! font smoothing on or off, at exact subpixel positions. This optional
//! diagnostic supplies a platform witness for review; quality gates use
//! independent outline coverage. Production never renders with it.
//!
//! Ported from spike S4 (`docs/engine/spikes/S4.md`).

use std::collections::HashMap;
use std::ptr::NonNull;

use objc2_core_foundation::{
    CFArray, CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGAffineTransform,
    CGPoint,
};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGFont, CGImageAlphaInfo, CGImageByteOrderInfo,
    CGImageComponentInfo, kCGColorSpaceSRGB,
};
use objc2_core_text::{
    CTFont, CTFontDescriptor, CTFontManagerCreateFontDescriptorsFromData, kCTFontVariationAttribute,
};

/// One run to draw: a face's bytes, its size and user-space variation
/// coordinates, and glyphs at device positions (y down, on the baseline).
pub struct RefRun<'a> {
    pub data: &'a [u8],
    pub index: u32,
    pub size: f32,
    pub coords: &'a [(u32, f32)],
    pub glyphs: Vec<(u16, f64, f64)>,
    /// Straight sRGB RGBA.
    pub color: [f64; 4],
}

/// Which font bytes a cached CTFont was made from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Source {
    /// A caller's buffer by address (the reference renderer: the caller
    /// keeps its fonts alive and unchanged while this renderer lives).
    Address(usize),
    Identity(u64),
}

/// Font identity: source, face index, size bits, variation coordinates.
type FontKey = (Source, u32, u32, Vec<(u32, u32)>);
/// A variation instance's identity: source, face index, variation
/// coordinates. Instances do not depend on the strike size.
type VariantKey = (Source, u32, Vec<(u32, u32)>);

/// Coordinates by bits: equal keys are equal `f32` values, `-0.0` and NaN
/// payloads included, so a cached instance is the one a fresh call makes.
fn coords_key(coords: &[(u32, f32)]) -> Vec<(u32, u32)> {
    coords.iter().map(|(t, v)| (*t, v.to_bits())).collect()
}

// Native strikes can retain a whole font data provider independently of
// their tiny glyph masks. Share providers across sizes and bound both.
const FONT_SOURCE_BYTES: usize = 32 << 20;
const FONT_CACHE_ENTRIES: usize = 128;
// Variation instances (one per weight or axis setting, shared by every
// strike size). They share their source's provider; each is a small
// native object, so a few dozen are kilobytes.
const VARIANT_CACHE_ENTRIES: usize = 64;

/// A CoreText reference renderer (keeps fonts across calls).
#[derive(Default)]
pub struct CoreTextReference {
    descriptors: HashMap<(Source, u32), CFRetained<CTFontDescriptor>>,
    fonts: HashMap<FontKey, CFRetained<CTFont>>,
    graphics: HashMap<(Source, u32), CFRetained<CGFont>>,
    /// `CGFontCreateCopyWithVariations` of a `graphics` font, per
    /// coordinates (hidden system faces such as `.SFNS`).
    graphics_variants: HashMap<VariantKey, CFRetained<CGFont>>,
    /// A variable descriptor's instance at the default size with its
    /// variations applied, per coordinates (`variable_font`).
    variable_variants: HashMap<VariantKey, CFRetained<CTFont>>,
    source_bytes: usize,
}

impl CoreTextReference {
    #[cfg(test)]
    pub(super) fn retained_for_test(&self) -> (usize, usize) {
        (self.source_bytes, self.fonts.len())
    }

    /// Variation instances retained (`graphics_variants` + `variable_variants`).
    #[cfg(test)]
    pub(super) fn variants_for_test(&self) -> usize {
        self.graphics_variants.len() + self.variable_variants.len()
    }

    /// Addresses of the retained provider objects, to tell a kept provider
    /// from one made again from the same bytes.
    #[cfg(test)]
    #[allow(dead_code)] // used by the canvas2d tests
    pub(super) fn providers_for_test(&self) -> Vec<usize> {
        let mut out: Vec<usize> = self
            .graphics
            .values()
            .map(|f| CFRetained::as_ptr(f).as_ptr() as usize)
            .chain(
                self.descriptors
                    .values()
                    .map(|d| CFRetained::as_ptr(d).as_ptr() as usize),
            )
            .collect();
        out.sort_unstable();
        out
    }

    /// Makes room for one more variation instance.
    fn make_variant_room(&mut self) {
        if self.graphics_variants.len() + self.variable_variants.len() >= VARIANT_CACHE_ENTRIES {
            self.graphics_variants.clear();
            self.variable_variants.clear();
        }
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn font(&mut self, run: &RefRun<'_>) -> Option<CFRetained<CTFont>> {
        self.font_from(Source::Address(run.data.as_ptr() as usize), run)
    }

    pub(crate) fn font_of(&mut self, id: u64, run: &RefRun<'_>) -> Option<CFRetained<CTFont>> {
        self.font_from(Source::Identity(id), run)
    }

    fn font_from(&mut self, id: Source, run: &RefRun<'_>) -> Option<CFRetained<CTFont>> {
        let key = (id, run.index, run.size.to_bits(), coords_key(run.coords));
        if let Some(f) = self.fonts.get(&key) {
            return Some(f.clone());
        }
        let source = (id, run.index);
        let new_source =
            !self.descriptors.contains_key(&source) && !self.graphics.contains_key(&source);
        if new_source
            && (self.source_bytes.saturating_add(run.data.len()) > FONT_SOURCE_BYTES
                || self.descriptors.len() + self.graphics.len() >= FONT_CACHE_ENTRIES)
        {
            *self = Self::default();
        } else if self.fonts.len() >= FONT_CACHE_ENTRIES {
            // Strike eviction must keep the shared providers warm: copying a
            // multi-megabyte font again for every size causes allocation churn.
            self.fonts.clear();
        }
        // SkTypeface_Mac::MakeFromStream instances variable data through a
        // CTFont descriptor at its default size, then the scaler makes an
        // exact copy at the strike size. CGFontCreateCopyWithVariations is
        // a different instancing path and changes variable outlines.
        if let Some(font) = self.variable_font(id, run) {
            self.keep_font(key, font.clone(), run.data.len());
            return Some(font);
        }
        // Single fonts go through CGFont, so CoreText never looks the face
        // up by name (it substitutes Times for a hidden system font's
        // PostScript name, ".SFNS-Regular"); variations are applied by axis
        // name in `fvar` order.
        let single = read_fonts::FileRef::new(run.data)
            .map(|f| matches!(f, read_fonts::FileRef::Font(_)))
            .unwrap_or(false);
        if single {
            let font = self.graphics_font(id, run)?;
            self.keep_font(key, font.clone(), run.data.len());
            return Some(font);
        }
        let desc = match self.descriptors.entry((id, run.index)) {
            std::collections::hash_map::Entry::Occupied(e) => e.get().clone(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let data = CFData::from_bytes(run.data);
                let arr = unsafe { CTFontManagerCreateFontDescriptorsFromData(&data) };
                // SAFETY: the function returns an array of font descriptors.
                let arr: CFRetained<CFArray<CTFontDescriptor>> =
                    unsafe { CFRetained::cast_unchecked(arr) };
                {
                    let desc = arr.get(run.index as usize)?;
                    if run.data.len() <= FONT_SOURCE_BYTES {
                        self.source_bytes += run.data.len();
                        e.insert(desc.clone());
                    }
                    desc
                }
            }
        };
        let mut font =
            unsafe { CTFont::with_font_descriptor(&desc, f64::from(run.size), std::ptr::null()) };
        if !run.coords.is_empty() {
            let keys: Vec<CFRetained<CFNumber>> = run
                .coords
                .iter()
                .map(|(t, _)| CFNumber::new_i64(i64::from(*t)))
                .collect();
            let vals: Vec<CFRetained<CFNumber>> = run
                .coords
                .iter()
                .map(|(_, v)| CFNumber::new_f64(f64::from(*v)))
                .collect();
            let kr: Vec<&CFNumber> = keys.iter().map(|k| &**k).collect();
            let vr: Vec<&CFNumber> = vals.iter().map(|v| &**v).collect();
            let variations: CFRetained<CFDictionary<CFNumber, CFNumber>> =
                CFDictionary::from_slices(&kr, &vr);
            let attrs: CFRetained<CFDictionary<CFString, CFType>> = CFDictionary::from_slices(
                &[unsafe { kCTFontVariationAttribute }],
                &[variations.as_ref() as &CFType],
            );
            let var_desc = unsafe { CTFontDescriptor::with_attributes(attrs.as_opaque()) };
            font = unsafe {
                font.copy_with_attributes(f64::from(run.size), std::ptr::null(), Some(&var_desc))
            };
        }
        self.keep_font(key, font.clone(), run.data.len());
        Some(font)
    }

    fn keep_font(&mut self, key: FontKey, font: CFRetained<CTFont>, source_bytes: usize) {
        // Oversize sources still draw; no native object retains them here.
        if source_bytes <= FONT_SOURCE_BYTES {
            self.fonts.insert(key, font);
        }
    }

    fn variable_font(&mut self, id: Source, run: &RefRun<'_>) -> Option<CFRetained<CTFont>> {
        // The instance with the run's variations does not depend on the
        // strike size: make it once per coordinates and copy only the size
        // (and the optical size the strike asks for) below.
        let key = (id, run.index, coords_key(run.coords));
        let font = match self.variable_variants.get(&key) {
            Some(font) => font.clone(),
            None => {
                let font = self.variable_instance(id, run)?;
                // Oversize sources still draw; no native object retains them here.
                if run.data.len() <= FONT_SOURCE_BYTES {
                    self.make_variant_room();
                    self.variable_variants.insert(key, font.clone());
                }
                font
            }
        };
        let opsz = run
            .coords
            .iter()
            .find(|(t, _)| *t == u32::from_be_bytes(*b"opsz"))
            .map_or(unsafe { font.size() }, |(_, v)| *v as f64);
        let opsz_key = CFString::from_str("NSCTFontOpticalSizeAttribute");
        let tracking_key = CFString::from_str("NSCTFontUnscaledTrackingAttribute");
        let opsz_value = CFNumber::new_f64(opsz);
        let zero = CFNumber::new_i32(0);
        let attrs: CFRetained<CFDictionary<CFString, CFType>> = CFDictionary::from_slices(
            &[&*opsz_key, &*tracking_key],
            &[opsz_value.as_ref() as &CFType, zero.as_ref() as &CFType],
        );
        let desc = unsafe { CTFontDescriptor::with_attributes(attrs.as_opaque()) };
        Some(unsafe { font.copy_with_attributes(run.size as f64, std::ptr::null(), Some(&desc)) })
    }

    /// A variable descriptor's instance at the default size with the run's
    /// variations, or `None` when the face has no `fvar` or is hidden.
    fn variable_instance(&mut self, id: Source, run: &RefRun<'_>) -> Option<CFRetained<CTFont>> {
        use skrifa::{MetadataProvider, raw::TableProvider};
        let file = skrifa::FontRef::from_index(run.data, run.index).ok()?;
        let axes = file.fvar().ok()?.axes().ok()?;
        // Hidden system fonts must retain the CGFont identity: descriptor
        // matching can substitute Times for a hidden PostScript name.
        let hidden = file
            .localized_strings(skrifa::string::StringId::POSTSCRIPT_NAME)
            .any(|name| name.chars().next() == Some('.'));
        if hidden {
            return None;
        }
        let desc = match self.descriptors.entry((id, run.index)) {
            std::collections::hash_map::Entry::Occupied(e) => e.get().clone(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let data = CFData::from_bytes(run.data);
                let arr = unsafe { CTFontManagerCreateFontDescriptorsFromData(&data) };
                let arr: CFRetained<CFArray<CTFontDescriptor>> =
                    unsafe { CFRetained::cast_unchecked(arr) };
                {
                    let desc = arr.get(run.index as usize)?;
                    if run.data.len() <= FONT_SOURCE_BYTES {
                        self.source_bytes += run.data.len();
                        e.insert(desc.clone());
                    }
                    desc
                }
            }
        };
        let mut font = unsafe { CTFont::with_font_descriptor(&desc, 0.0, std::ptr::null()) };
        let keys: Vec<_> = axes
            .iter()
            .map(|a| CFNumber::new_i64(i64::from(u32::from_be_bytes(a.axis_tag().into_bytes()))))
            .collect();
        let values: Vec<_> = axes
            .iter()
            .map(|a| {
                let tag = u32::from_be_bytes(a.axis_tag().into_bytes());
                let value = run
                    .coords
                    .iter()
                    .rev()
                    .find(|(t, _)| *t == tag)
                    .map_or(a.default_value().to_f32(), |(_, v)| *v)
                    .clamp(a.min_value().to_f32(), a.max_value().to_f32());
                CFNumber::new_f64(value as f64)
            })
            .collect();
        let kr: Vec<_> = keys.iter().map(|k| &**k).collect();
        let vr: Vec<_> = values.iter().map(|v| &**v).collect();
        let variations = CFDictionary::from_slices(&kr, &vr);
        let attrs: CFRetained<CFDictionary<CFString, CFType>> = CFDictionary::from_slices(
            &[unsafe { kCTFontVariationAttribute }],
            &[variations.as_ref() as &CFType],
        );
        let var_desc = unsafe { CTFontDescriptor::with_attributes(attrs.as_opaque()) };
        font = unsafe { font.copy_with_attributes(0.0, std::ptr::null(), Some(&var_desc)) };
        Some(font)
    }

    fn graphics_font(&mut self, id: Source, run: &RefRun<'_>) -> Option<CFRetained<CTFont>> {
        // The variation copy does not depend on the strike size: make it
        // once per coordinates; only the CTFont below is per size.
        let cg = if run.coords.is_empty() {
            self.graphics_base(id, run)?
        } else {
            let key = (id, run.index, coords_key(run.coords));
            match self.graphics_variants.get(&key) {
                Some(cg) => cg.clone(),
                None => {
                    let base = self.graphics_base(id, run)?;
                    let cg = graphics_variant(&base, run)?;
                    // Oversize sources still draw; no native object retains them here.
                    if run.data.len() <= FONT_SOURCE_BYTES {
                        self.make_variant_room();
                        self.graphics_variants.insert(key, cg.clone());
                    }
                    cg
                }
            }
        };
        // CoreText sets the optical size from the point size unless the font
        // is created with an explicit one (the attribute the registry's
        // `sbix_em_advance` uses); keep the run's.
        let opsz = run
            .coords
            .iter()
            .find(|(t, _)| *t == u32::from_be_bytes(*b"opsz"))
            .map(|(_, v)| *v);
        let attrs = opsz.map(|v| {
            let key = CFString::from_str("NSCTFontOpticalSizeAttribute");
            let value = CFNumber::new_f64(f64::from(v));
            let dict: CFRetained<CFDictionary<CFString, CFType>> =
                CFDictionary::from_slices(&[&*key], &[value.as_ref() as &CFType]);
            unsafe { CTFontDescriptor::with_attributes(dict.as_opaque()) }
        });
        Some(unsafe {
            CTFont::with_graphics_font(&cg, f64::from(run.size), std::ptr::null(), attrs.as_deref())
        })
    }

    /// The face's CGFont without variations, sharing one provider per source.
    fn graphics_base(&mut self, id: Source, run: &RefRun<'_>) -> Option<CFRetained<CGFont>> {
        use objc2_core_graphics::CGDataProvider;
        let source = (id, run.index);
        Some(if let Some(cg) = self.graphics.get(&source) {
            cg.clone()
        } else {
            let data = CFData::from_bytes(run.data);
            let provider = CGDataProvider::with_cf_data(Some(&data))?;
            let cg = CGFont::with_data_provider(&provider)?;
            if run.data.len() <= FONT_SOURCE_BYTES {
                self.source_bytes += run.data.len();
                self.graphics.insert(source, cg.clone());
            }
            cg
        })
    }
}

/// `base` with the run's variations, applied by axis name in `fvar` order.
fn graphics_variant(base: &CGFont, run: &RefRun<'_>) -> Option<CFRetained<CGFont>> {
    use read_fonts::TableProvider;
    let font = read_fonts::FontRef::new(run.data).ok()?;
    let tags: Vec<u32> = font
        .fvar()
        .ok()
        .and_then(|f| {
            f.axes().ok().map(|a| {
                a.iter()
                    .map(|x| x.axis_tag().into_bytes())
                    .map(u32::from_be_bytes)
                    .collect()
            })
        })
        .unwrap_or_default();
    let axes = CGFont::variation_axes(Some(base))?;
    let mut names: Vec<CFRetained<CFString>> = Vec::new();
    let mut values: Vec<CFRetained<CFNumber>> = Vec::new();
    for (i, tag) in tags.iter().enumerate() {
        let Some(&(_, v)) = run.coords.iter().find(|(t, _)| t == tag) else {
            continue;
        };
        // SAFETY: CGFontCopyVariationAxes returns an array of dictionaries.
        let axis: Option<CFRetained<CFDictionary<CFString, CFType>>> = unsafe {
            let arr: &CFArray<CFDictionary<CFString, CFType>> = axes.cast_unchecked();
            arr.get(i)
        };
        let Some(axis) = axis else { continue };
        let name_key = unsafe { objc2_core_graphics::kCGFontVariationAxisName };
        let Some(name) = axis.get(name_key) else {
            continue;
        };
        let Ok(name) = name.downcast::<CFString>() else {
            continue;
        };
        names.push(name);
        values.push(CFNumber::new_f64(f64::from(v)));
    }
    let nr: Vec<&CFString> = names.iter().map(|k| &**k).collect();
    let vr: Vec<&CFNumber> = values.iter().map(|v| &**v).collect();
    let dict: CFRetained<CFDictionary<CFString, CFNumber>> = CFDictionary::from_slices(&nr, &vr);
    unsafe { CGFont::new_copy_with_variations(Some(base), Some(dict.as_opaque())) }
}

impl CoreTextReference {
    /// Draws `runs` in order into a transparent `width` × `height` bitmap;
    /// returns premultiplied f32 RGBA, top-down.
    pub fn render(
        &mut self,
        runs: &[RefRun<'_>],
        width: u32,
        height: u32,
        smoothing: bool,
    ) -> Vec<f32> {
        self.render_posed(runs, &[], width, height, smoothing)
    }

    /// [`Self::render`] with each run drawn through its own affine transform
    /// (`[a, b, c, d, e, f]` in top-down pixel space, applied after the
    /// glyph placement; runs past the end of `matrices` draw untransformed).
    /// The reference for posed (rotated, scaled, skewed) glyphs.
    pub fn render_posed(
        &mut self,
        runs: &[RefRun<'_>],
        matrices: &[[f64; 6]],
        width: u32,
        height: u32,
        smoothing: bool,
    ) -> Vec<f32> {
        let (w, h) = (width as usize, height as usize);
        let mut data = vec![0.0f32; w * h * 4];
        let Some(space) = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB })) else {
            return data;
        };
        let info = CGImageAlphaInfo::PremultipliedLast.0
            | CGImageComponentInfo::Float.0
            | CGImageByteOrderInfo::Order32Little.0;
        let Some(ctx) = (unsafe {
            CGBitmapContextCreate(
                data.as_mut_ptr().cast(),
                w,
                h,
                32,
                w * 16,
                Some(&space),
                info,
            )
        }) else {
            return data;
        };
        let c = Some(&*ctx);
        CGContext::set_should_antialias(c, true);
        CGContext::set_allows_font_smoothing(c, smoothing);
        CGContext::set_should_smooth_fonts(c, smoothing);
        CGContext::set_allows_font_subpixel_positioning(c, true);
        CGContext::set_should_subpixel_position_fonts(c, true);
        CGContext::set_allows_font_subpixel_quantization(c, false);
        CGContext::set_should_subpixel_quantize_fonts(c, false);
        // Glyphs draw in CoreGraphics' native bottom-left space (a flipped
        // CTM with a flipped text matrix draws nothing).
        CGContext::set_text_matrix(
            c,
            CGAffineTransform {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                tx: 0.0,
                ty: 0.0,
            },
        );
        for (i, run) in runs.iter().enumerate() {
            let Some(font) = self.font(run) else { continue };
            CGContext::set_rgb_fill_color(
                c,
                run.color[0],
                run.color[1],
                run.color[2],
                run.color[3],
            );
            let glyphs: Vec<u16> = run.glyphs.iter().map(|g| g.0).collect();
            let pos: Vec<CGPoint> = run
                .glyphs
                .iter()
                .map(|g| CGPoint {
                    x: g.1,
                    y: h as f64 - g.2,
                })
                .collect();
            if glyphs.is_empty() {
                continue;
            }
            // Top-down M in bottom-up space is F·M·F (F flips y about h).
            let posed = matrices
                .get(i)
                .filter(|m| **m != [1.0, 0.0, 0.0, 1.0, 0.0, 0.0])
                .map(|m| CGAffineTransform {
                    a: m[0],
                    b: -m[1],
                    c: -m[2],
                    d: m[3],
                    tx: m[4] + m[2] * h as f64,
                    ty: h as f64 - m[5] - m[3] * h as f64,
                });
            if let Some(t) = posed {
                CGContext::save_g_state(c);
                CGContext::concat_ctm(c, t);
            }
            unsafe {
                font.draw_glyphs(
                    NonNull::new(glyphs.as_ptr() as *mut u16).expect("not empty"),
                    NonNull::new(pos.as_ptr() as *mut CGPoint).expect("not empty"),
                    glyphs.len(),
                    &ctx,
                )
            };
            if posed.is_some() {
                CGContext::restore_g_state(c);
            }
        }
        CGContext::flush(c);
        drop(ctx);
        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Animated text in nested compositions asks for many exact strike sizes.
    /// Small glyph masks must not hide whole font-file copies per strike.
    #[test]
    fn animated_strikes_bound_native_font_data() {
        // Static CoreGraphics faces copy a provider per strike; variable
        // descriptors share their provider but still retain native strikes.
        let faces = [
            std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf").expect("Arial"),
            include_bytes!("../../tests/fixtures/Geist-Variable.ttf").to_vec(),
        ];
        for data in faces {
            let mut cache = CoreTextReference::new();
            for step in 0..600 {
                let run = RefRun {
                    data: &data,
                    index: 0,
                    size: 20.0 + step as f32 * 0.125,
                    coords: &[],
                    glyphs: vec![],
                    color: [1.; 4],
                };
                assert!(cache.font_of(1, &run).is_some());
                assert!(cache.fonts.len() <= 128);
            }
            assert_eq!(cache.source_bytes, data.len());
            assert_eq!(cache.graphics.len() + cache.descriptors.len(), 1);
        }
    }

    /// Oversize font files still render, but must never become cache residents.
    #[test]
    fn oversize_font_sources_render_without_retention() {
        let mut data =
            std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf").expect("Arial");
        data.resize(FONT_SOURCE_BYTES + 1, 0);
        let mut cache = CoreTextReference::new();
        for size in [20., 40.] {
            let run = RefRun {
                data: &data,
                index: 0,
                size,
                coords: &[],
                glyphs: vec![],
                color: [1.; 4],
            };
            assert!(cache.font_of(1, &run).is_some());
            assert_eq!(cache.source_bytes, 0);
            assert!(cache.fonts.is_empty());
            assert!(cache.graphics.is_empty());
            assert!(cache.descriptors.is_empty());
            assert_eq!(cache.variants_for_test(), 0);
        }
    }

    #[test]
    fn changing_font_sources_stays_within_the_byte_budget() {
        let data = std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf").expect("Arial");
        let mut cache = CoreTextReference::new();
        for id in 0..600 {
            let run = RefRun {
                data: &data,
                index: 0,
                size: 40.,
                coords: &[],
                glyphs: vec![],
                color: [1.; 4],
            };
            assert!(cache.font_of(id, &run).is_some());
            assert!(cache.source_bytes <= FONT_SOURCE_BYTES);
            assert!(cache.graphics.len() + cache.descriptors.len() <= FONT_CACHE_ENTRIES);
            assert!(cache.fonts.len() <= FONT_CACHE_ENTRIES);
        }
    }

    const WGHT: u32 = u32::from_be_bytes(*b"wght");
    const OPSZ: u32 = u32::from_be_bytes(*b"opsz");

    /// Advances and glyph boxes of `font`, as bits.
    fn metrics(font: &CTFont, glyphs: &[u16]) -> Vec<u64> {
        use objc2_core_foundation::{CGRect, CGSize};
        use objc2_core_text::CTFontOrientation;
        let n = glyphs.len();
        let mut advances = vec![
            CGSize {
                width: 0.,
                height: 0.
            };
            n
        ];
        let mut rects = vec![CGRect::default(); n];
        let ptr = NonNull::new(glyphs.as_ptr() as *mut u16).expect("not empty");
        let (total, union) = unsafe {
            (
                font.advances_for_glyphs(
                    CTFontOrientation::Horizontal,
                    ptr,
                    advances.as_mut_ptr(),
                    n as isize,
                ),
                font.bounding_rects_for_glyphs(
                    CTFontOrientation::Horizontal,
                    ptr,
                    rects.as_mut_ptr(),
                    n as isize,
                ),
            )
        };
        let mut out = vec![unsafe { font.size() }.to_bits(), total.to_bits()];
        for r in std::iter::once(union).chain(rects) {
            out.extend([r.origin.x, r.origin.y, r.size.width, r.size.height].map(f64::to_bits));
        }
        for a in advances {
            out.extend([a.width.to_bits(), a.height.to_bits()]);
        }
        out
    }

    /// Fonts made from cached, per-coordinates variation instances are the
    /// fonts a fresh CoreTextReference makes: the same size, advances and
    /// glyph boxes, and the same reference-rendered pixels, for SF Pro
    /// (hidden: the CGFont variation path) and a visible variable face (the
    /// descriptor path), across weights, the optical size axis and the
    /// fixture's animated fractional sizes.
    #[test]
    fn cached_variation_instances_make_the_fonts_fresh_ones_make() {
        let faces = [
            std::fs::read("/System/Library/Fonts/SFNS.ttf").expect("SF Pro"),
            include_bytes!("../../tests/fixtures/Geist-Variable.ttf").to_vec(),
        ];
        for data in &faces {
            let glyphs: Vec<u16> = {
                use read_fonts::TableProvider;
                let cmap = read_fonts::FontRef::new(data).unwrap().cmap().unwrap();
                "Powermove GOOD TIMES"
                    .chars()
                    .filter_map(|c| cmap.map_codepoint(c as u32))
                    .map(|g| g.to_u32() as u16)
                    .collect()
            };
            let mut warm = CoreTextReference::new();
            let coords: Vec<Vec<(u32, f32)>> = [400f32, 450., 500., 550., 600., 650.]
                .map(|w| vec![(WGHT, w)])
                .into_iter()
                .chain([vec![(WGHT, 500.), (OPSZ, 20.)], vec![]])
                .collect();
            for pass in 0..2 {
                for coords in &coords {
                    for pt in [25.9f64, 62.9] {
                        for k in 0..8 {
                            let scale = 0.25 * (1.05 + (0.93 - 1.05) * k as f64 / 7.) * 1.5;
                            let size = (pt * scale) as f32;
                            let run = RefRun {
                                data,
                                index: 0,
                                size,
                                coords,
                                glyphs: glyphs
                                    .iter()
                                    .enumerate()
                                    .map(|(i, &g)| {
                                        (g, 2.25 + i as f64 * f64::from(size) * 0.7, 40.)
                                    })
                                    .collect(),
                                color: [0.1, 0.2, 0.3, 1.],
                            };
                            let what = format!("{coords:?} {size} pass {pass}");
                            let mut fresh = CoreTextReference::new();
                            let a = warm.font_of(1, &run).expect("warm font");
                            let b = fresh.font_of(1, &run).expect("fresh font");
                            assert_eq!(metrics(&a, &glyphs), metrics(&b, &glyphs), "{what}");
                            for &g in &glyphs {
                                let (pa, pb) = unsafe {
                                    (
                                        a.path_for_glyph(g, std::ptr::null()),
                                        b.path_for_glyph(g, std::ptr::null()),
                                    )
                                };
                                let same = match (&pa, &pb) {
                                    (None, None) => true,
                                    (Some(pa), Some(pb)) => {
                                        objc2_core_graphics::CGPath::equal_to_path(
                                            Some(pa),
                                            Some(pb),
                                        )
                                    }
                                    _ => false,
                                };
                                assert!(same, "{what}: outline of g{g}");
                            }
                            let width = (glyphs.len() as f32 * size * 0.7) as u32 + 16;
                            let bits = |img: Vec<f32>| {
                                img.into_iter().map(f32::to_bits).collect::<Vec<_>>()
                            };
                            let a = bits(warm.render(std::slice::from_ref(&run), width, 56, true));
                            let b = bits(CoreTextReference::new().render(
                                std::slice::from_ref(&run),
                                width,
                                56,
                                true,
                            ));
                            assert!(a == b, "{what}: reference pixels");
                            assert!(a.iter().any(|&v| v != 0), "{what}: ink");
                        }
                    }
                }
            }
            // At most one instance per coordinates for
            // each of the two sources `font_of` and `render` (`font`) use.
            assert!(warm.variants_for_test() <= 2 * coords.len());
            assert!(warm.fonts.len() <= FONT_CACHE_ENTRIES);
        }
    }

    /// Thousands of distinct weights keep the instance cache bounded.
    #[test]
    fn variation_instances_stay_bounded() {
        let data = std::fs::read("/System/Library/Fonts/SFNS.ttf").expect("SF Pro");
        let mut cache = CoreTextReference::new();
        for i in 0..2000 {
            let coords = [(WGHT, 100. + i as f32 * 0.4)];
            let run = RefRun {
                data: &data,
                index: 0,
                size: 18.5,
                coords: &coords,
                glyphs: vec![],
                color: [1.; 4],
            };
            assert!(cache.font_of(1, &run).is_some());
            assert!(cache.variants_for_test() <= VARIANT_CACHE_ENTRIES);
            assert!(cache.fonts.len() <= FONT_CACHE_ENTRIES);
            assert_eq!(cache.source_bytes, data.len());
        }
    }

    /// Glyphs land where they are placed, and smoothing adds ink.
    #[test]
    fn glyphs_draw_at_their_positions() {
        let data = std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf").expect("Arial");
        let gid = {
            use read_fonts::TableProvider;
            read_fonts::FontRef::new(&data)
                .unwrap()
                .cmap()
                .unwrap()
                .map_codepoint('H' as u32)
                .unwrap()
                .to_u32() as u16
        };
        let run = |x: f64| RefRun {
            data: &data,
            index: 0,
            size: 40.0,
            coords: &[],
            glyphs: vec![(gid, x, 50.0)],
            color: [1.0; 4],
        };
        let mut r = CoreTextReference::new();
        let plain = r.render(&[run(10.0)], 64, 64, false);
        let smooth = r.render(&[run(10.0)], 64, 64, true);
        let ink = |img: &[f32], rows: std::ops::Range<usize>| -> f32 {
            rows.flat_map(|y| (0..64).map(move |x| img[(y * 64 + x) * 4 + 3]))
                .sum()
        };
        assert!(ink(&plain, 22..50) > 100.0);
        assert!(ink(&plain, 52..64) < 1.0 && ink(&plain, 0..18) < 1.0);
        assert!(ink(&smooth, 0..64) > ink(&plain, 0..64));
    }
}
