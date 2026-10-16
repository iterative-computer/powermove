//! The CoreText reference rasterizer (macOS): glyphs through
//! `CTFontDrawGlyphs` into a 32-bit float premultiplied sRGB bitmap, with
//! font smoothing on (the macOS look, level 2's target) or off, at exact
//! subpixel positions. It draws what the engine is compared against in the
//! level 2 gate and in Jude's side-by-side review sheets; the engine never
//! renders with it (S4: coverage comes from Skrifa outlines on every host).
//!
//! Ported from spike S4 (`engine/spikes/s4-text/crates/s4-render`).

use std::collections::HashMap;
use std::ptr::NonNull;

use objc2_core_foundation::{
    CFArray, CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGAffineTransform,
    CGPoint,
};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImageAlphaInfo, CGImageByteOrderInfo,
    CGImageComponentInfo, kCGColorSpaceSRGB,
};
use objc2_core_text::{
    CTFont, CTFontDescriptor, CTFontManagerCreateFontDescriptorsFromData,
    kCTFontVariationAttribute,
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

/// A CoreText reference renderer (keeps fonts across calls).
#[derive(Default)]
pub struct CoreTextReference {
    descriptors: HashMap<(usize, u32), CFRetained<CTFontDescriptor>>,
    fonts: HashMap<(usize, u32, u32, Vec<(u32, u32)>), CFRetained<CTFont>>,
}

impl CoreTextReference {
    pub fn new() -> Self {
        Self::default()
    }

    fn font(&mut self, run: &RefRun<'_>) -> Option<CFRetained<CTFont>> {
        let id = run.data.as_ptr() as usize;
        let key = (
            id,
            run.index,
            run.size.to_bits(),
            run.coords.iter().map(|(t, v)| (*t, v.to_bits())).collect::<Vec<_>>(),
        );
        if let Some(f) = self.fonts.get(&key) {
            return Some(f.clone());
        }
        // Single fonts go through CGFont, so CoreText never looks the face
        // up by name (it substitutes Times for a hidden system font's
        // PostScript name, ".SFNS-Regular"); variations are applied by axis
        // name in `fvar` order.
        let single = read_fonts::FileRef::new(run.data)
            .map(|f| matches!(f, read_fonts::FileRef::Font(_)))
            .unwrap_or(false);
        if single {
            let font = self.graphics_font(run)?;
            self.fonts.insert(key, font.clone());
            return Some(font);
        }
        if !self.descriptors.contains_key(&(id, run.index)) {
            let data = CFData::from_bytes(run.data);
            let arr = unsafe { CTFontManagerCreateFontDescriptorsFromData(&data) };
            // SAFETY: the function returns an array of font descriptors.
            let arr: CFRetained<CFArray<CTFontDescriptor>> = unsafe { CFRetained::cast_unchecked(arr) };
            let d = arr.get(run.index as usize)?;
            self.descriptors.insert((id, run.index), d);
        }
        let desc = self.descriptors[&(id, run.index)].clone();
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
        self.fonts.insert(key, font.clone());
        Some(font)
    }

    fn graphics_font(&self, run: &RefRun<'_>) -> Option<CFRetained<CTFont>> {
        use objc2_core_graphics::{CGDataProvider, CGFont};
        let data = CFData::from_bytes(run.data);
        let provider = CGDataProvider::with_cf_data(Some(&data))?;
        let mut cg = CGFont::with_data_provider(&provider)?;
        if !run.coords.is_empty() {
            use read_fonts::TableProvider;
            let font = read_fonts::FontRef::new(run.data).ok()?;
            let tags: Vec<u32> = font
                .fvar()
                .ok()
                .and_then(|f| f.axes().ok().map(|a| a.iter().map(|x| x.axis_tag().into_bytes()).map(u32::from_be_bytes).collect()))
                .unwrap_or_default();
            let axes = CGFont::variation_axes(Some(&cg))?;
            let mut names: Vec<CFRetained<CFString>> = Vec::new();
            let mut values: Vec<CFRetained<CFNumber>> = Vec::new();
            for (i, tag) in tags.iter().enumerate() {
                let Some(&(_, v)) = run.coords.iter().find(|(t, _)| t == tag) else { continue };
                // SAFETY: CGFontCopyVariationAxes returns an array of dictionaries.
                let axis: Option<CFRetained<CFDictionary<CFString, CFType>>> = unsafe {
                    let arr: &CFArray<CFDictionary<CFString, CFType>> = axes.cast_unchecked();
                    arr.get(i)
                };
                let Some(axis) = axis else { continue };
                let name_key = unsafe { objc2_core_graphics::kCGFontVariationAxisName };
                let Some(name) = axis.get(name_key) else { continue };
                let Ok(name) = name.downcast::<CFString>() else { continue };
                names.push(name);
                values.push(CFNumber::new_f64(f64::from(v)));
            }
            let nr: Vec<&CFString> = names.iter().map(|k| &**k).collect();
            let vr: Vec<&CFNumber> = values.iter().map(|v| &**v).collect();
            let dict: CFRetained<CFDictionary<CFString, CFNumber>> = CFDictionary::from_slices(&nr, &vr);
            cg = unsafe { CGFont::new_copy_with_variations(Some(&cg), Some(dict.as_opaque())) }?;
        }
        Some(unsafe { CTFont::with_graphics_font(&cg, f64::from(run.size), std::ptr::null(), None) })
    }

    /// Draws `runs` in order into a transparent `width` × `height` bitmap;
    /// returns premultiplied f32 RGBA, top-down.
    pub fn render(&mut self, runs: &[RefRun<'_>], width: u32, height: u32, smoothing: bool) -> Vec<f32> {
        let (w, h) = (width as usize, height as usize);
        let mut data = vec![0.0f32; w * h * 4];
        let Some(space) = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB })) else {
            return data;
        };
        let info = CGImageAlphaInfo::PremultipliedLast.0
            | CGImageComponentInfo::Float.0
            | CGImageByteOrderInfo::Order32Little.0;
        let Some(ctx) = (unsafe {
            CGBitmapContextCreate(data.as_mut_ptr().cast(), w, h, 32, w * 16, Some(&space), info)
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
        for run in runs {
            let Some(font) = self.font(run) else { continue };
            CGContext::set_rgb_fill_color(c, run.color[0], run.color[1], run.color[2], run.color[3]);
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
            unsafe {
                font.draw_glyphs(
                    NonNull::new(glyphs.as_ptr() as *mut u16).expect("not empty"),
                    NonNull::new(pos.as_ptr() as *mut CGPoint).expect("not empty"),
                    glyphs.len(),
                    &ctx,
                )
            };
        }
        CGContext::flush(c);
        drop(ctx);
        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            rows.flat_map(|y| (0..64).map(move |x| img[(y * 64 + x) * 4 + 3])).sum()
        };
        assert!(ink(&plain, 22..50) > 100.0);
        assert!(ink(&plain, 52..64) < 1.0 && ink(&plain, 0..18) < 1.0);
        assert!(ink(&smooth, 0..64) > ink(&plain, 0..64));
    }
}
