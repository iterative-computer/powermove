//! The CoreText and CoreGraphics reference: glyphs through `CTFontDrawGlyphs`
//! and paths through `CGContext` into a 32-bit float premultiplied RGBA
//! bitmap (sRGB). Font smoothing is a parameter: macOS dilates glyphs when
//! smoothing is on, and the comparison measures which setting today's
//! Canvas2D output follows.

use crate::image::Image;
use crate::prepare::{Draw, FaceData, Prepared};
use crate::scene::Color;
use objc2_core_foundation::{CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGAffineTransform, CGPoint};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGImageByteOrderInfo, CGImageComponentInfo, CGContext, CGImageAlphaInfo, CGLineCap, CGLineJoin, kCGColorSpaceSRGB,
};
use objc2_core_text::{CTFont, CTFontDescriptor, CTFontManagerCreateFontDescriptorsFromData, kCTFontVariationAttribute};
use kurbo::PathEl;
use s4_layout::font::FaceId;
use std::collections::{BTreeMap, HashMap};
use std::ptr::NonNull;

pub struct CoreTextRenderer {
    pub smoothing: bool,
    descriptors: HashMap<FaceId, CFRetained<CTFontDescriptor>>,
    fonts: HashMap<(FaceId, u32, Vec<(u32, u32)>), CFRetained<CTFont>>,
}

impl CoreTextRenderer {
    pub fn new(smoothing: bool) -> Self {
        CoreTextRenderer { smoothing, descriptors: HashMap::new(), fonts: HashMap::new() }
    }

    fn font(&mut self, faces: &BTreeMap<FaceId, FaceData>, face: FaceId, size: f32, coords: &[(u32, f32)]) -> CFRetained<CTFont> {
        let key = (face, size.to_bits(), coords.iter().map(|(t, v)| (*t, v.to_bits())).collect::<Vec<_>>());
        if let Some(f) = self.fonts.get(&key) {
            return f.clone();
        }
        let desc = self
            .descriptors
            .entry(face)
            .or_insert_with(|| {
                let fd = &faces[&face];
                let data = CFData::from_bytes(&fd.data);
                let arr = unsafe { CTFontManagerCreateFontDescriptorsFromData(&data) };
                let arr: CFRetained<objc2_core_foundation::CFArray<CTFontDescriptor>> = unsafe { CFRetained::cast_unchecked(arr) };
                arr.get(fd.index as usize).expect("face index in collection")
            })
            .clone();
        let mut font = unsafe { CTFont::with_font_descriptor(&desc, size as f64, std::ptr::null()) };
        if !coords.is_empty() {
            let keys: Vec<CFRetained<CFNumber>> = coords.iter().map(|(t, _)| CFNumber::new_i64(*t as i64)).collect();
            let vals: Vec<CFRetained<CFNumber>> = coords.iter().map(|(_, v)| CFNumber::new_f64(*v as f64)).collect();
            let kr: Vec<&CFNumber> = keys.iter().map(|k| &**k).collect();
            let vr: Vec<&CFNumber> = vals.iter().map(|v| &**v).collect();
            let variations: CFRetained<CFDictionary<CFNumber, CFNumber>> = CFDictionary::from_slices(&kr, &vr);
            let attrs: CFRetained<CFDictionary<CFString, CFType>> =
                CFDictionary::from_slices(&[unsafe { kCTFontVariationAttribute }], &[variations.as_ref() as &CFType]);
            let var_desc = unsafe { CTFontDescriptor::with_attributes(attrs.as_opaque()) };
            font = unsafe { font.copy_with_attributes(size as f64, std::ptr::null(), Some(&var_desc)) };
        }
        self.fonts.insert(key, font.clone());
        font
    }

    pub fn render(&mut self, scene: &Prepared, faces: &BTreeMap<FaceId, FaceData>) -> Image {
        let (w, h) = (scene.width as usize, scene.height as usize);
        let mut img = Image::new(scene.width, scene.height);
        let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB })).unwrap();
        let info = CGImageAlphaInfo::PremultipliedLast.0 | CGImageComponentInfo::Float.0 | CGImageByteOrderInfo::Order32Little.0;
        let ctx = unsafe { CGBitmapContextCreate(img.data.as_mut_ptr().cast(), w, h, 32, w * 16, Some(&space), info) }.expect("float bitmap context");
        let c = Some(&*ctx);
        // Top-left origin for paths (glyphs: see below).
        CGContext::translate_ctm(c, 0.0, h as f64);
        CGContext::scale_ctm(c, 1.0, -1.0);
        CGContext::set_should_antialias(c, true);
        CGContext::set_allows_font_smoothing(c, self.smoothing);
        CGContext::set_should_smooth_fonts(c, self.smoothing);
        CGContext::set_allows_font_subpixel_positioning(c, true);
        CGContext::set_should_subpixel_position_fonts(c, true);
        CGContext::set_allows_font_subpixel_quantization(c, false);
        CGContext::set_should_subpixel_quantize_fonts(c, false);
        for draw in &scene.draws {
            match draw {
                Draw::Text { runs, color, stroke, stroke_color } => {
                    if *stroke > 0.0 {
                        // strokeText: the glyph outlines stroked at 2 × stroke.
                        set_stroke(c, stroke_color, *stroke * 2.0, true);
                        CGContext::set_miter_limit(c, 2.0);
                        for run in runs {
                            let font = self.font(faces, run.face, run.size, &run.coords);
                            for g in &run.glyphs {
                                let Some(p) = (unsafe { font.path_for_glyph(g.id as u16, std::ptr::null()) }) else { continue };
                                CGContext::save_g_state(c);
                                CGContext::translate_ctm(c, g.x, g.y);
                                CGContext::scale_ctm(c, 1.0, -1.0);
                                CGContext::add_path(c, Some(&p));
                                CGContext::restore_g_state(c);
                                CGContext::stroke_path(c);
                            }
                        }
                    }
                    CGContext::set_rgb_fill_color(c, color[0] as f64, color[1] as f64, color[2] as f64, color[3] as f64);
                    // CTFontDrawGlyphs draws nothing under a flipped CTM with a
                    // flipped text matrix (measured); glyphs draw in CoreGraphics'
                    // native bottom-left space instead.
                    CGContext::save_g_state(c);
                    CGContext::scale_ctm(c, 1.0, -1.0);
                    CGContext::translate_ctm(c, 0.0, -(h as f64));
                    CGContext::set_text_matrix(c, CGAffineTransform { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 });
                    for run in runs {
                        let font = self.font(faces, run.face, run.size, &run.coords);
                        let glyphs: Vec<u16> = run.glyphs.iter().map(|g| g.id as u16).collect();
                        let pos: Vec<CGPoint> = run.glyphs.iter().map(|g| CGPoint { x: g.x, y: h as f64 - g.y }).collect();
                        if glyphs.is_empty() {
                            continue;
                        }
                        unsafe { font.draw_glyphs(NonNull::new(glyphs.as_ptr() as *mut u16).unwrap(), NonNull::new(pos.as_ptr() as *mut CGPoint).unwrap(), glyphs.len(), &ctx) };
                    }
                    CGContext::restore_g_state(c);
                }
                Draw::Path { path, color, even_odd, stroke, round_join } => {
                    CGContext::begin_path(c);
                    for el in path.elements() {
                        match *el {
                            PathEl::MoveTo(p) => CGContext::move_to_point(c, p.x, p.y),
                            PathEl::LineTo(p) => CGContext::add_line_to_point(c, p.x, p.y),
                            PathEl::QuadTo(a, p) => CGContext::add_quad_curve_to_point(c, a.x, a.y, p.x, p.y),
                            PathEl::CurveTo(a, b, p) => CGContext::add_curve_to_point(c, a.x, a.y, b.x, b.y, p.x, p.y),
                            PathEl::ClosePath => CGContext::close_path(c),
                        }
                    }
                    if *stroke > 0.0 {
                        set_stroke(c, color, *stroke, *round_join);
                        CGContext::stroke_path(c);
                    } else {
                        CGContext::set_rgb_fill_color(c, color[0] as f64, color[1] as f64, color[2] as f64, color[3] as f64);
                        if *even_odd { CGContext::eo_fill_path(c) } else { CGContext::fill_path(c) }
                    }
                }
            }
        }
        CGContext::flush(c);
        drop(ctx);
        img
    }
}

fn set_stroke(c: Option<&CGContext>, color: &Color, width: f64, round: bool) {
    CGContext::set_rgb_stroke_color(c, color[0] as f64, color[1] as f64, color[2] as f64, color[3] as f64);
    CGContext::set_line_width(c, width);
    CGContext::set_line_join(c, if round { CGLineJoin::Round } else { CGLineJoin::Miter });
    CGContext::set_line_cap(c, CGLineCap::Butt);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prepare::{GlyphRun, PGlyph};
    use std::sync::Arc;

    /// Glyphs land upright where the layout put them, and smoothing adds ink.
    #[test]
    fn glyphs_draw_at_their_positions() {
        let data = std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf").expect("Arial ships with macOS");
        let gid = read_fonts::FontRef::new(&data).ok().and_then(|f| {
            use read_fonts::TableProvider;
            f.cmap().ok()?.map_codepoint('H' as u32)
        });
        let gid = gid.expect("H").to_u32();
        let faces: BTreeMap<FaceId, FaceData> = [(0, FaceData { data: Arc::new(data), index: 0, postscript_name: "ArialMT".into(), synthetic_bold: false, synthetic_italic: false, bitmap: false })].into();
        let run = GlyphRun { face: 0, size: 40.0, coords: vec![], glyphs: vec![PGlyph { id: gid, x: 10.0, y: 50.0 }] };
        let scene = Prepared { id: "t".into(), group: "text".into(), width: 64, height: 64, draws: vec![Draw::Text { runs: vec![run], color: [1.0; 4], stroke: 0.0, stroke_color: [0.0, 0.0, 0.0, 1.0] }], glyphs: 1, path_segments: 0 };
        let plain = CoreTextRenderer::new(false).render(&scene, &faces);
        let smooth = CoreTextRenderer::new(true).render(&scene, &faces);
        let ink = |img: &Image, rows: std::ops::Range<usize>| -> f32 { rows.flat_map(|y| (0..64).map(move |x| (y, x))).map(|(y, x)| img.data[(y * 64 + x) * 4 + 3]).sum() };
        // "H" at 40 px with its baseline at y = 50 covers rows ~21..50.
        assert!(ink(&plain, 22..50) > 100.0, "glyph ink above the baseline");
        assert!(ink(&plain, 52..64) < 1.0 && ink(&plain, 0..18) < 1.0, "nothing below the baseline or above the cap height");
        assert!(ink(&smooth, 0..64) > ink(&plain, 0..64), "smoothing dilates");
    }
}

#[cfg(test)]
mod positioning {
    use super::*;
    use crate::prepare::{GlyphRun, PGlyph};
    use std::sync::Arc;

    /// Prints where CoreText puts a glyph for fractional origins (alpha
    /// centroid shift against the requested shift). Run with --ignored.
    #[test]
    #[ignore]
    fn coretext_origin_quantization() {
        let data = std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf").unwrap();
        let gid = {
            use read_fonts::TableProvider;
            read_fonts::FontRef::new(&data).unwrap().cmap().unwrap().map_codepoint('o' as u32).unwrap().to_u32()
        };
        let faces: BTreeMap<FaceId, FaceData> = [(0, FaceData { data: Arc::new(data), index: 0, postscript_name: "ArialMT".into(), synthetic_bold: false, synthetic_italic: false, bitmap: false })].into();
        for smooth in [false, true] {
            let mut r = CoreTextRenderer::new(smooth);
            let centroid = |r: &mut CoreTextRenderer, x: f64, y: f64| {
                let run = GlyphRun { face: 0, size: 24.0, coords: vec![], glyphs: vec![PGlyph { id: gid, x, y }] };
                let scene = Prepared { id: "t".into(), group: "text".into(), width: 64, height: 64, draws: vec![Draw::Text { runs: vec![run], color: [1.0; 4], stroke: 0.0, stroke_color: [0.0; 4] }], glyphs: 1, path_segments: 0 };
                let img = r.render(&scene, &faces);
                let (mut sx, mut sy, mut s) = (0.0, 0.0, 0.0);
                for yy in 0..64 {
                    for xx in 0..64 {
                        let a = img.data[(yy * 64 + xx) * 4 + 3] as f64;
                        sx += a * xx as f64;
                        sy += a * yy as f64;
                        s += a;
                    }
                }
                (sx / s, sy / s)
            };
            let (x0, y0) = centroid(&mut r, 20.0, 40.0);
            let mut line = format!("smooth {smooth}:");
            for i in 0..20 {
                let f = i as f64 * 0.05;
                let (x, _) = centroid(&mut r, 20.0 + f, 40.0);
                let (_, y) = centroid(&mut r, 20.0, 40.0 + f);
                line += &format!("\n  f {f:.2}: dx {:.3} dy {:.3}", x - x0, y - y0);
            }
            eprintln!("{line}");
        }
        // Where smoothing puts the extra ink: centroid and ink-weighted edge
        // moments of smoothed minus plain, for an "o" at several sizes.
        for size in [12.0f32, 24.0, 48.0, 96.0] {
            let render = |smooth: bool| {
                let run = GlyphRun { face: 0, size, coords: vec![], glyphs: vec![PGlyph { id: gid, x: 20.0, y: 140.0 }] };
                let scene = Prepared { id: "t".into(), group: "text".into(), width: 160, height: 160, draws: vec![Draw::Text { runs: vec![run], color: [1.0; 4], stroke: 0.0, stroke_color: [0.0; 4] }], glyphs: 1, path_segments: 0 };
                CoreTextRenderer::new(smooth).render(&scene, &faces)
            };
            let (p, q) = (render(false), render(true));
            let stat = |img: &Image| {
                let (mut sx, mut sy, mut s) = (0.0, 0.0, 0.0);
                for yy in 0..160 {
                    for xx in 0..160 {
                        let a = img.data[(yy * 160 + xx) * 4 + 3] as f64;
                        sx += a * xx as f64;
                        sy += a * yy as f64;
                        s += a;
                    }
                }
                (sx / s, sy / s, s)
            };
            let (a, b) = (stat(&p), stat(&q));
            eprintln!("size {size}: ink plain {:.1} smooth {:.1} (+{:.1}%), centroid shift dx {:.3} dy {:.3}", a.2, b.2, 100.0 * (b.2 / a.2 - 1.0), b.0 - a.0, b.1 - a.1);
        }
    }
}
