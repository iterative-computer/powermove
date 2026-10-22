//! Text and captions (plan sections 10 and 12, amendments 10 and 19).
//! Design notes: `docs/engine/text-render.md`.
//!
//! A text or captions layer draws as today's compositor draws it: its
//! canvas raster (`rasterText`, `rasterAnimatedText`, or the captions
//! rasterizer's `draw`) at the density `continuousRasterScale` asks for,
//! sampled through the layer's quad. The raster is a [`TextScene`] painted
//! by platform masks, native Graphite primitives and Vello coverage through
//! the ordered paint pass ([`gpu`]): N32 for level 1, float for level 2.
//! Cached rasters retain only the ink crop; native primitives keep the full
//! canvas viewport while drawing so interpolation and AA stay unchanged.
//!
//! Layout is `pm-text`'s level 1 (`compat`) through the host's
//! [`TextFonts`]; word states, animator poses, raster boxes, and font
//! anchors are `pm-eval`'s.

mod canvas;
pub mod gpu;
mod graphite;
pub mod served;

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use kurbo::{Affine, PathEl};
use pm_eval::captions::CaptionFrame;
use pm_eval::text_geom::{TextMeasure, text_content, text_geometry};
use pm_eval::transform::Mat2d;
use pm_eval::{Content, JsResult, JsValue, Scope};
use pm_model::{JsString, Node};
use pm_text::compat::{GlyphRun, LineRuns, TextContent};
use pm_text::font::{FaceId, FaceInfo, FontResolver};
use pm_text::render::bitmap::Bitmaps;
use pm_text::render::color::{self, Rgba};
use pm_text::render::outline::Outlines;
use pm_text::render::profile::Profile;
use pm_text::render::scene::{ItemContent, Painter, PathStyle, TextScene, fill_path, rounded_rect};

/// The fonts a renderer lays text out with: `pm-text`'s level 1 layout
/// over the host's font registry (full font files, so outlines are
/// available). The interactive and job engines answer it from their font
/// service; tests use the CoreText registry.
pub trait TextFonts: Send {
    /// The measuring canvas `pm-eval` uses for geometry, word layout, and
    /// glyph units.
    fn measure(&mut self) -> &mut dyn TextMeasure;
    /// `rasterText`'s glyphs for `d` (wrapped lines, `fillText` per line).
    fn line_runs(&mut self, d: &TextContent) -> Vec<LineRuns>;
    /// `fillText(text, 0, 0)` with `textAlign = 'left'` and `letterSpacing
    /// = tracking px`.
    fn text_runs(&mut self, d: &TextContent, text: &str, tracking: f64) -> Vec<GlyphRun>;
    /// The face the runs name.
    fn face(&mut self, id: FaceId) -> &FaceInfo;
    /// CoreText's advance of a full-em glyph of an `sbix` face at `size`
    /// (`FontResolver::em_advance`): the size color bitmap glyphs draw at.
    fn em_advance(&mut self, face: FaceId, size: f32) -> Option<f32> {
        let _ = (face, size);
        None
    }
    /// Changes whenever the registry's fonts change (project fonts, sources);
    /// cached rasters of an older generation are dropped.
    fn generation(&self) -> u64 {
        0
    }
    /// Called before each text or captions layer is laid out: fonts whose
    /// registry changed start over here (never in the middle of a layer,
    /// whose runs name the current faces).
    fn sync(&mut self) {}
    /// Registry questions left unanswered since the last call (a served
    /// registry that did not answer): the layer that met them is refused,
    /// never drawn with another font.
    fn take_failures(&mut self) -> Vec<String> {
        Vec::new()
    }
    /// What the next layers are laid out for (the window whose font cache
    /// seeds they measure with); applied at the next [`TextFonts::sync`].
    fn set_seed_scope(&mut self, scope: served::SeedScope) {
        let _ = scope;
    }
    /// Level 2 layout for `compat` ≥ 2 text layers, when the host provides
    /// it (`None`: such layers are refused).
    fn level2(&mut self) -> Option<&mut Level2Fonts> {
        None
    }
}

/// Level 2 layout (Parley, `pm_text::level2`) over full fonts the host
/// added: the engine lays out and draws from the same bytes.
pub struct Level2Fonts {
    pub engine: pm_text::level2::Level2,
    faces: std::collections::HashMap<String, pm_text::font::FontBytes>,
}

impl Default for Level2Fonts {
    fn default() -> Self {
        Level2Fonts {
            engine: pm_text::level2::Level2::new(),
            faces: std::collections::HashMap::new(),
        }
    }
}

impl Level2Fonts {
    /// Adds one face (a single-face font file) under `key` for `family`.
    pub fn add_font(&mut self, key: &str, data: pm_text::font::FontBytes, family: &str) -> usize {
        let n = self.engine.add_font(key, data.clone(), family);
        self.faces.insert(key.to_owned(), data);
        n
    }

    pub fn set_manifest(&mut self, manifest: &pm_text::registry::FallbackManifest) {
        self.engine.set_manifest(manifest);
    }

    pub fn face(&self, key: &str) -> Option<&pm_text::font::FontBytes> {
        self.faces.get(key)
    }
}

impl<R: FontResolver + Send> TextFonts for pm_eval::text_measure::CompatMeasure<R> {
    fn measure(&mut self) -> &mut dyn TextMeasure {
        self
    }

    fn line_runs(&mut self, d: &TextContent) -> Vec<LineRuns> {
        self.compat().line_runs(d)
    }

    fn text_runs(&mut self, d: &TextContent, text: &str, tracking: f64) -> Vec<GlyphRun> {
        self.compat().text_runs(d, text, tracking).0
    }

    fn face(&mut self, id: FaceId) -> &FaceInfo {
        self.compat().resolver().face(id)
    }

    fn em_advance(&mut self, face: FaceId, size: f32) -> Option<f32> {
        self.compat().resolver_mut().em_advance(face, size)
    }
}

/// Level 1 glyph masks as Canvas2D gets them on this platform: CoreGraphics
/// on macOS (`pm_text::render::cgmask`); elsewhere none (outlines with the
/// Canvas2D profile).
fn platform_masks() -> Option<Box<dyn pm_text::render::scene::MaskSource + Send>> {
    #[cfg(target_os = "macos")]
    {
        Some(Box::new(pm_text::render::cgmask::CgMasks::new()))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Everything text rendering keeps across frames.
pub struct TextContext {
    pub(crate) fonts: Option<Box<dyn TextFonts>>,
    outlines: Outlines,
    bitmaps: Bitmaps,
    /// Level 1 glyph masks from the platform rasterizer (CoreGraphics on
    /// macOS), as Canvas2D draws them.
    masks: Option<Box<dyn pm_text::render::scene::MaskSource + Send>>,
    /// The level 1 profile (Canvas2D).
    profile: Profile,
    generation: u64,
    /// The fonts' own generation when last synced.
    fonts_generation: u64,
    captions_hidden: bool,
    unsupported: Vec<String>,
}

impl Default for TextContext {
    fn default() -> Self {
        TextContext {
            fonts: None,
            outlines: Outlines::new(),
            bitmaps: Bitmaps::new(),
            masks: platform_masks(),
            profile: Profile::CANVAS2D,
            generation: 0,
            fonts_generation: 0,
            captions_hidden: false,
            unsupported: Vec::new(),
        }
    }
}

impl TextContext {
    pub fn set_fonts(&mut self, fonts: Option<Box<dyn TextFonts>>) {
        self.fonts = fonts;
        self.forget_glyphs();
        self.generation += 1;
        self.fonts_generation = self.fonts.as_ref().map_or(0, |f| f.generation());
    }

    /// Before a layer: the fonts catch up with their registry, and a new
    /// font generation makes every later raster key new (cached rasters of
    /// the old fonts are never reused).
    fn sync_fonts(&mut self) {
        if let Some(fonts) = self.fonts.as_deref_mut() {
            fonts.sync();
            let g = fonts.generation();
            if g != self.fonts_generation {
                self.fonts_generation = g;
                self.generation += 1;
                self.forget_glyphs();
            }
        }
    }

    /// Drops every glyph drawn from the previous fonts: outlines, color
    /// bitmaps, platform masks and their CTFonts. Glyph caches key on
    /// `FontBytes::id`, which is never reused, so a replaced font can never
    /// hit an old entry; this releases the old entries.
    fn forget_glyphs(&mut self) {
        self.outlines.clear();
        self.bitmaps.clear();
        if let Some(masks) = self.masks.as_deref_mut() {
            masks.reset();
        }
    }

    pub fn has_fonts(&self) -> bool {
        self.fonts.is_some()
    }

    /// Where level 1 glyph masks come from (`None`: outlines with the
    /// profile's stem darkening).
    pub fn set_masks(&mut self, masks: Option<Box<dyn pm_text::render::scene::MaskSource + Send>>) {
        self.masks = masks;
        self.generation += 1;
    }

    /// The level 1 profile (tests and the fitting tool).
    pub fn set_profile(&mut self, profile: Profile) {
        self.profile = profile;
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// `PM.captionsHidden`: an export without burned-in captions draws no
    /// captions layer.
    pub fn set_captions_hidden(&mut self, hidden: bool) {
        self.captions_hidden = hidden;
    }

    pub fn captions_hidden(&self) -> bool {
        self.captions_hidden
    }

    /// What the last layer built could not draw.
    pub(crate) fn take_unsupported(&mut self) -> Vec<String> {
        if let Some(fonts) = self.fonts.as_deref_mut() {
            self.unsupported.extend(fonts.take_failures());
        }
        std::mem::take(&mut self.unsupported)
    }
}

/// One text raster in the plan: the scene of a canvas, and the part of it
/// the texture holds (`crop`, whole device pixels of the canvas).
#[derive(Debug, Clone)]
pub struct TextPlan {
    pub scene: Arc<TextScene>,
    /// Compat level 1 is an N32 Canvas2D backing store, even when the
    /// composition that samples it has a floating-point working format.
    pub canvas_n32: bool,
    pub crop: [u32; 4],
    /// Content key of the painted pixels (scene, crop, and the font
    /// generation): equal keys paint equal rasters.
    pub key: [u64; 2],
}

impl PartialEq for TextPlan {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl TextPlan {
    /// A plan of a whole scene (no crop).
    pub fn whole(scene: TextScene) -> TextPlan {
        let crop = [0, 0, scene.width.max(1), scene.height.max(1)];
        let key = scene_key(&scene, crop, u64::MAX);
        TextPlan {
            scene: Arc::new(scene),
            canvas_n32: false,
            crop,
            key,
        }
    }

    pub fn width(&self) -> u32 {
        self.crop[2] - self.crop[0]
    }

    pub fn height(&self) -> u32 {
        self.crop[3] - self.crop[1]
    }
}

/// A text layer's quad: the plan and its placement in layer pixels.
pub(crate) struct TextQuad {
    pub plan: TextPlan,
    pub w: f64,
    pub h: f64,
    pub ax: f64,
    pub ay: f64,
}

/// `continuousRasterScale(matrix)` (`legacy/gl/compositor.ts`): the layer's
/// largest singular value, in quarter steps, within 0.25 to 32.
pub fn continuous_raster_scale(m: &Mat2d) -> f64 {
    let or0 = |v: f64| if v.is_nan() { 0.0 } else { v };
    let (m0, m1, m2, m3) = (or0(m[0]), or0(m[1]), or0(m[2]), or0(m[3]));
    let aa = m0 * m0 + m1 * m1;
    let bb = m0 * m2 + m1 * m3;
    let cc = m2 * m2 + m3 * m3;
    let disc = libm::sqrt(js_max(0.0, (aa - cc) * (aa - cc) + 4.0 * bb * bb));
    let largest = libm::sqrt(js_max(0.0, (aa + cc + disc) / 2.0));
    let requested = largest * 1.0;
    let bounded = js_max(0.25, js_min(32.0, requested));
    libm::ceil(bounded * 4.0 - 1e-9) / 4.0
}

fn js_max(a: f64, b: f64) -> f64 {
    crate::js::max(a, b)
}

fn js_min(a: f64, b: f64) -> f64 {
    crate::js::min(a, b)
}

fn get<'c>(d: &'c Content, key: &str) -> &'c JsValue {
    static UNDEFINED: JsValue = JsValue::Undefined;
    d.get(&JsString::from(key)).unwrap_or(&UNDEFINED)
}

/// Canvas size for a box at a density: `max(1, ceil(n × density))`.
fn canvas_px(n: f64, density: f64) -> u32 {
    let v = libm::ceil(n * density);
    if v.is_finite() && v >= 1.0 {
        v.min(f64::from(u32::MAX)) as u32
    } else {
        1
    }
}

/// `c.fillStyle = value` on a context whose style is `previous`. A string
/// the setter rejects keeps `previous`; a CSS color the parser does not
/// reproduce is recorded in `unsupported` (the layer is refused).
fn fill_style(value: &JsValue, previous: Rgba, unsupported: &mut Vec<String>) -> Rgba {
    // Canvas stringifies non-strings (`undefined`, `null` and numbers never
    // parse as colors).
    let s = match value {
        JsValue::String(s) => s.to_string_lossy(),
        other => other.to_js_string().to_string_lossy(),
    };
    css_color(&s, previous, unsupported)
}

/// [`color::classify`] as a canvas setter on a context whose style is
/// `previous`.
fn css_color(s: &str, previous: Rgba, unsupported: &mut Vec<String>) -> Rgba {
    match color::classify(s) {
        Ok(c) => c,
        Err(color::Rejected::Invalid) => previous,
        Err(color::Rejected::Unsupported) => {
            let what = format!("text color {s:?}");
            if !unsupported.contains(&what) {
                unsupported.push(what);
            }
            previous
        }
    }
}

/// `hexToRgba(hex, opacity)` of `captions/render.ts` as the canvas setter
/// takes the `rgba(...)` string it builds: `None` when the string holds a
/// `NaN` (a short or non-hex code) and the setter keeps its previous style.
fn hex_to_rgba(hex: &str, opacity: f64) -> Option<Rgba> {
    // `hex.replace('#', '')` (the first `#`), sliced in UTF-16 units.
    let value: Vec<u16> = hex.replacen('#', "", 1).encode_utf16().collect();
    let part = |a: usize, b: usize| -> f64 {
        let s = String::from_utf16_lossy(value.get(a..b.min(value.len())).unwrap_or(&[]));
        pm_eval::js_sem::parse_int(&JsString::from(s.as_str()), 16)
    };
    let (r, g, b) = (part(0, 2), part(2, 4), part(4, 6));
    let a = if value.len() >= 8 {
        part(6, 8) / 255.0
    } else {
        1.0
    };
    let alpha = libm::round(a * opacity * 1000.0) / 1000.0;
    // `rgba(NaN,...)` and `Infinity` are not CSS numbers; the integer
    // channels and the alpha clamp to their ranges.
    if ![r, g, b, alpha].iter().all(|v| v.is_finite()) {
        return None;
    }
    let ch = |v: f64| v.clamp(0.0, 255.0) / 255.0;
    Some([ch(r), ch(g), ch(b), alpha.clamp(0.0, 1.0)])
}

/// Hashes the scene into a 128-bit content key.
fn scene_key(scene: &TextScene, crop: [u32; 4], generation: u64) -> [u64; 2] {
    let mut out = [0u64; 2];
    for (i, o) in out.iter_mut().enumerate() {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (i as u64, generation, scene.width, scene.height, crop).hash(&mut h);
        for item in &scene.items {
            if matches!(item.content, ItemContent::GlyphMask { .. }) {
                // Hash the paint uploaded to the shader. CSS Color 4 system
                // colors already arrive as f32; equivalent legacy syntax
                // must have the same key after conversion to SkColor4f.
                let alpha = (item.color[3] * item.opacity) as f32;
                for c in [
                    item.color[0] as f32 * alpha,
                    item.color[1] as f32 * alpha,
                    item.color[2] as f32 * alpha,
                    alpha,
                ] {
                    c.to_bits().hash(&mut h);
                }
            } else {
                for c in item.color {
                    c.to_bits().hash(&mut h);
                }
                item.opacity.to_bits().hash(&mut h);
            }
            item.blur.to_bits().hash(&mut h);
            match &item.content {
                ItemContent::Coverage { paths, curve } => {
                    0u8.hash(&mut h);
                    for v in curve {
                        v.to_bits().hash(&mut h);
                    }
                    for p in paths {
                        for c in p.transform.as_coeffs() {
                            c.to_bits().hash(&mut h);
                        }
                        match &p.style {
                            PathStyle::Fill => 0u8.hash(&mut h),
                            PathStyle::Stroke(s) => {
                                1u8.hash(&mut h);
                                s.width.to_bits().hash(&mut h);
                                s.miter_limit.to_bits().hash(&mut h);
                            }
                        }
                        for el in p.path.elements() {
                            let pts: &[kurbo::Point] = match el {
                                PathEl::MoveTo(p) => {
                                    1u8.hash(&mut h);
                                    std::slice::from_ref(p)
                                }
                                PathEl::LineTo(p) => {
                                    2u8.hash(&mut h);
                                    std::slice::from_ref(p)
                                }
                                PathEl::QuadTo(a, b) => {
                                    3u8.hash(&mut h);
                                    for q in [a, b] {
                                        q.x.to_bits().hash(&mut h);
                                        q.y.to_bits().hash(&mut h);
                                    }
                                    &[]
                                }
                                PathEl::CurveTo(a, b, c) => {
                                    4u8.hash(&mut h);
                                    for q in [a, b, c] {
                                        q.x.to_bits().hash(&mut h);
                                        q.y.to_bits().hash(&mut h);
                                    }
                                    &[]
                                }
                                PathEl::ClosePath => {
                                    5u8.hash(&mut h);
                                    &[]
                                }
                            };
                            for q in pts {
                                q.x.to_bits().hash(&mut h);
                                q.y.to_bits().hash(&mut h);
                            }
                        }
                    }
                }
                ItemContent::Image { image, transform }
                | ItemContent::GlyphMask { image, transform, .. }
                | ItemContent::Graphite {
                    image, transform, ..
                } => {
                    (if matches!(item.content, ItemContent::GlyphMask { .. }) {
                        2u8
                    } else {
                        1u8
                    })
                    .hash(&mut h);
                    if let ItemContent::Graphite { raster, .. } = &item.content {
                        3u8.hash(&mut h);
                        raster.hash_content(&mut h);
                    }
                    image.width.hash(&mut h);
                    image.height.hash(&mut h);
                    image.data.hash(&mut h);
                    for c in transform.as_coeffs() {
                        c.to_bits().hash(&mut h);
                    }
                }
            }
        }
        *o = h.finish();
    }
    out
}

impl TextContext {
    fn painter(
        &mut self,
        viewport: (u32, u32),
    ) -> (Painter<'_>, &mut dyn TextFonts, &mut Vec<String>) {
        let fonts = self.fonts.as_deref_mut().expect("checked by the caller");
        (
            Painter {
                outlines: &mut self.outlines,
                bitmaps: &mut self.bitmaps,
                // Platform masks are Canvas2D's look: level 1 only (level 2
                // captions swap in the CoreText profile and draw outlines).
                masks: if self.profile == Profile::CANVAS2D {
                    self.masks
                        .as_deref_mut()
                        .map(|m| m as &mut dyn pm_text::render::scene::MaskSource)
                } else {
                    None
                },
                viewport: Some(viewport),
                path_rasterizer: Some(&pm_vector::canvas2d::text::RASTERIZER),
                profile: self.profile.clone(),
                unsupported: Vec::new(),
            },
            fonts,
            &mut self.unsupported,
        )
    }

    /// The plan of a scene: its ink crop and content key, or `None` when
    /// nothing draws.
    fn finish(&self, scene: TextScene, canvas_n32: bool) -> Option<TextPlan> {
        let crop = scene.ink()?;
        let mut key = scene_key(&scene, crop, self.generation);
        key[0] ^= u64::from(canvas_n32);
        Some(TextPlan {
            scene: Arc::new(scene),
            canvas_n32,
            crop,
            key,
        })
    }

    /// The quad of a canvas of `canvas_w` × `canvas_h` px standing for a
    /// `w` × `h` layer-px box anchored at `(ax, ay)`, cropped to `plan`.
    fn quad(plan: TextPlan, w: f64, h: f64, ax: f64, ay: f64) -> TextQuad {
        let (cw, ch) = (f64::from(plan.scene.width), f64::from(plan.scene.height));
        // A canvas pixel spans w / canvas_w layer px (the canvas is
        // stretched over the box, as the compositor's quad stretches it).
        let (sx, sy) = (w / cw, h / ch);
        let [x0, y0, x1, y1] = plan.crop.map(f64::from);
        TextQuad {
            plan,
            w: (x1 - x0) * sx,
            h: (y1 - y0) * sy,
            ax: ax - x0 * sx,
            ay: ay - y0 * sy,
        }
    }

    /// A text layer at `time` drawn at raster scale `scale`
    /// (`continuousRasterScale` of its scaled world matrix).
    pub(crate) fn text_layer<'d>(
        &mut self,
        scope: &mut Scope<'_, 'd>,
        layer: &'d Node,
        time: f64,
        scale: f64,
    ) -> JsResult<Option<TextQuad>> {
        self.sync_fonts();
        let d = scope.resolved_text_content(layer, time)?;
        let stored = pm_eval::node::member(Some(layer), "d");
        let has = |key: &str| {
            pm_eval::node::member(stored, key)
                .map(|n| pm_eval::node::get(n, b"length").truthy())
                .unwrap_or(false)
        };
        let raster = {
            let fonts = self.fonts.as_deref_mut().expect("checked by the caller");
            scope.text_raster(layer, time, fonts.measure())?
        };
        if has("animators") || has("styles") {
            let tc = text_content(&d);
            let fonts = self.fonts.as_deref_mut().expect("checked by the caller");
            let layout = fonts.measure().glyph_layout(&tc);
            let state = scope.text_animation_state(layer, time, &layout)?;
            let animated = scope.animated_raster_glyphs(&d, &layout, &state, fonts.measure());
            if let Some(a) = animated {
                let w = js_max(1.0, a.x1 - a.x0);
                let h = js_max(1.0, a.y1 - a.y0);
                let density = js_min(js_min(scale, 8192.0 / w), 8192.0 / h);
                let (cw, ch) = (canvas_px(w, density), canvas_px(h, density));
                let mut scene = TextScene {
                    width: cw,
                    height: ch,
                    items: Vec::new(),
                };
                let (mut painter, fonts, unsupported) = self.painter((cw, ch));
                for (g, baseline) in a.glyphs.iter().zip(&a.baselines) {
                    if g.opacity.is_nan() || g.opacity <= 0.0 {
                        continue;
                    }
                    let m = canvas::transform(
                        canvas::translate(
                            canvas::narrow(Affine::new([
                                density,
                                0.0,
                                0.0,
                                density,
                                -a.x0 * density,
                                -a.y0 * density,
                            ])),
                            g.unit.x,
                            *baseline,
                        ),
                        g.matrix,
                    );
                    let fill = match &g.color {
                        Some(c) => JsValue::String(c.clone()),
                        None => get(&g.style, "color").clone(),
                    };
                    // Each glyph draws between `save()` and `restore()`: a
                    // rejected color leaves the canvas default, black.
                    let style_color = fill_style(&fill, color::BLACK, &mut painter.unsupported);
                    let blur = if g.blur > 0.01 { g.blur * density } else { 0.0 };
                    let style = text_content(&g.style);
                    let runs = fonts.text_runs(&style, &g.unit.text, 0.0);
                    let faces = run_faces(fonts, &runs, m);
                    let refs = run_refs(&runs, &faces);
                    painter.fill_text(
                        &mut scene.items,
                        &refs,
                        (0.0, 0.0),
                        m,
                        style_color,
                        g.opacity,
                        blur,
                    );
                }
                unsupported.append(&mut painter.unsupported);
                let anchor = (raster.anchor_x, raster.anchor_y);
                return Ok(self
                    .finish(scene, true)
                    .map(|p| Self::quad(p, raster.w, raster.h, anchor.0, anchor.1)));
            }
        }
        let fonts = self.fonts.as_deref_mut().expect("checked by the caller");
        let g = text_geometry(&d, fonts.measure());
        let density = g.density(scale);
        let (cw, ch) = (canvas_px(g.w, density), canvas_px(g.h, density));
        let tc = text_content(&d);
        let fill = {
            let c = get(&d, "color");
            if c.truthy() {
                c.clone()
            } else {
                JsValue::from("#fff")
            }
        };
        let fill = fill_style(&fill, color::BLACK, &mut self.unsupported);
        let mut scene = TextScene {
            width: cw,
            height: ch,
            items: Vec::new(),
        };
        let (mut painter, fonts, unsupported) = self.painter((cw, ch));
        let lines = fonts.line_runs(&tc);
        let m = Affine::scale(density);
        for (row, line) in lines.iter().enumerate() {
            // Blink's DrawTextInternal converts the canvas point to float,
            // then subtracts the double alignment width. The blob's glyphs
            // remain relative to (0,0): baking this origin into each glyph
            // changes SDF creation-space and path-matrix rounding.
            let runs = fonts.text_runs(&tc, &line.text, tc.tracking.unwrap_or(0.0));
            let origin = (
                f64::from((f64::from(g.x as f32) + line.x) as f32),
                f64::from(((g.pad + row as f64 * tc.line_height()) + tc.size() * 0.82) as f32),
            );
            let faces = run_faces(fonts, &runs, m);
            let refs = run_refs(&runs, &faces);
            painter.fill_text(&mut scene.items, &refs, origin, m, fill, 1.0, 0.0);
        }
        unsupported.append(&mut painter.unsupported);
        Ok(self
            .finish(scene, true)
            .map(|p| Self::quad(p, raster.w, raster.h, raster.anchor_x, raster.anchor_y)))
    }

    /// A `compat` level 2 text layer at output resolution: level 2 layout
    /// (Parley) drawn with the CoreText profile at exact positions through
    /// `m` (layer pixels to target pixels, size anchor included) into a
    /// `target`-sized scene, cropped to its ink.
    pub(crate) fn text_layer_level2<'d>(
        &mut self,
        scope: &mut Scope<'_, 'd>,
        layer: &'d Node,
        time: f64,
        m: Affine,
        target: (u32, u32),
    ) -> JsResult<Option<TextPlan>> {
        self.sync_fonts();
        let stored = pm_eval::node::member(Some(layer), "d");
        let has = |key: &str| {
            pm_eval::node::member(stored, key)
                .map(|n| pm_eval::node::get(n, b"length").truthy())
                .unwrap_or(false)
        };
        if has("animators") || has("styles") {
            self.unsupported
                .push("animated or range-styled text at compat level 2".into());
            return Ok(None);
        }
        if pm_eval::node::member(stored, "fontAnchorBounds")
            .is_some_and(|b| pm_eval::node::truthy(Some(b)))
        {
            self.unsupported
                .push("typography anchoring at compat level 2".into());
            return Ok(None);
        }
        let d = scope.resolved_text_content(layer, time)?;
        let tc = text_content(&d);
        let fill = {
            let c = get(&d, "color");
            let fallback = JsValue::from("#fff");
            fill_style(
                if c.truthy() { c } else { &fallback },
                color::BLACK,
                &mut self.unsupported,
            )
        };
        let fonts = self.fonts.as_deref_mut().expect("checked by the caller");
        let Some(l2) = fonts.level2() else {
            self.unsupported
                .push("compat level 2 text without level 2 fonts".into());
            return Ok(None);
        };
        let layout = l2.engine.layout(&tc);
        if layout.missing_glyphs > 0 {
            self.unsupported
                .push("compat level 2 text whose fonts are not loaded".into());
            return Ok(None);
        }
        let faces: Vec<Option<pm_text::font::FontBytes>> = layout
            .runs
            .iter()
            .map(|r| l2.face(&r.font).cloned())
            .collect();
        let mut scene = TextScene {
            width: target.0,
            height: target.1,
            items: Vec::new(),
        };
        let mut painter = Painter {
            outlines: &mut self.outlines,
            bitmaps: &mut self.bitmaps,
            masks: None,
            viewport: None,
            path_rasterizer: None,
            profile: Profile::CORETEXT,
            unsupported: Vec::new(),
        };
        for (run, data) in layout.runs.iter().zip(&faces) {
            let Some(data) = data else {
                self.unsupported
                    .push(format!("compat level 2 font {} not loaded", run.font));
                return Ok(None);
            };
            let face = pm_text::render::outline::FaceRef {
                data: data.as_slice(),
                index: 0,
                id: data.id(),
            };
            let refs = [pm_text::render::scene::Run {
                face,
                size: run.size,
                coords: pm_text::render::outline::Coords::Normalized(&run.coords),
                synthetic_bold: run.synthetic_bold,
                synthetic_italic: false,
                oblique: f64::from(run.skew),
                glyphs: &run.glyphs,
                bitmap_size: None,
            }];
            painter.fill_text(&mut scene.items, &refs, (0.0, 0.0), m, fill, 1.0, 0.0);
        }
        self.unsupported.append(&mut painter.unsupported);
        Ok(self.finish(scene, false))
    }

    /// A `compat` level 2 captions layer at output resolution: the same
    /// frame (layout, word states, box) drawn with the CoreText profile at
    /// exact positions through `m` (layer pixels to target pixels).
    pub(crate) fn captions_layer_level2<'d>(
        &mut self,
        scope: &mut Scope<'_, 'd>,
        layer: &'d Node,
        time: f64,
        m: Affine,
        target: (u32, u32),
    ) -> JsResult<Option<TextPlan>> {
        self.sync_fonts();
        let frame = {
            let fonts = self.fonts.as_deref_mut().expect("checked by the caller");
            scope.caption_frame(layer, time, fonts.measure())?
        };
        let Some(frame) = frame else {
            return Ok(None);
        };
        let base = m * Affine::translate((frame.block.x0, frame.block.y0));
        let saved = std::mem::replace(&mut self.profile, Profile::CORETEXT);
        let mut scene = TextScene {
            width: target.0,
            height: target.1,
            items: Vec::new(),
        };
        self.caption_items(&frame, base, &mut scene);
        self.profile = saved;
        Ok(self.finish(scene, false))
    }

    /// A captions layer at `time` drawn at raster scale `scale`.
    pub(crate) fn captions_layer<'d>(
        &mut self,
        scope: &mut Scope<'_, 'd>,
        layer: &'d Node,
        time: f64,
        scale: f64,
    ) -> JsResult<Option<TextQuad>> {
        self.sync_fonts();
        let frame = {
            let fonts = self.fonts.as_deref_mut().expect("checked by the caller");
            scope.caption_frame(layer, time, fonts.measure())?
        };
        let Some(frame) = frame else {
            return Ok(None);
        };
        let (scene, w, h, ax, ay) = self.caption_scene(&frame, scale);
        Ok(self
            .finish(scene, true)
            .map(|p| Self::quad(p, w, h, ax, ay)))
    }

    /// The captions rasterizer's `draw(frame, scale)` as a scene, with the
    /// raster box and anchor.
    pub fn caption_scene(
        &mut self,
        f: &CaptionFrame,
        scale: f64,
    ) -> (TextScene, f64, f64, f64, f64) {
        let block = f.block;
        let margin = f.margin;
        let w = block.x1 - block.x0 + margin * 2.0;
        let h = block.y1 - block.y0 + margin * 2.0;
        let density = js_max(0.05, js_min(js_min(scale, 8192.0 / w), 8192.0 / h));
        let mut scene = TextScene {
            width: canvas_px(w, density),
            height: canvas_px(h, density),
            items: Vec::new(),
        };
        let m0 = canvas::translate(
            canvas::scale(Affine::IDENTITY, density, density),
            margin,
            margin,
        );
        self.caption_items(f, m0, &mut scene);
        (scene, w, h, margin - block.x0, margin - block.y0)
    }

    /// The captions rasterizer's draws of frame `f` through `m0` (block
    /// pixels, origin at the block's top-left corner, to device pixels).
    fn caption_items(&mut self, f: &CaptionFrame, m0: Affine, scene: &mut TextScene) {
        let style = &f.style;
        let layout = &f.layout;
        let block = f.block;
        let pad = if style.boxed { style.box_padding } else { 0.0 };
        // The style each word's `save()` restores: the box fill once the box
        // is drawn, else the canvas default, black.
        let mut restored = color::BLACK;
        if style.boxed && style.box_opacity > 0.0 {
            // A rejected box color fills with the canvas default, black.
            let fill = hex_to_rgba(
                &style.box_color.to_string_lossy(),
                style.box_opacity / 100.0,
            )
            .unwrap_or(color::BLACK);
            restored = fill;
            let path = rounded_rect(
                0.0,
                0.0,
                block.x1 - block.x0,
                block.y1 - block.y0,
                style.box_radius,
            );
            // With platform masks (Canvas2D's look), the box is filled as
            // the GPU canvas fills paths: 4× multisampled.
            let item = if self.masks.is_some() && self.profile == Profile::CANVAS2D {
                pm_text::render::scene::fill_rounded_rect_msaa(
                    (
                        0.0,
                        0.0,
                        block.x1 - block.x0,
                        block.y1 - block.y0,
                        style.box_radius,
                    ),
                    m0,
                    fill,
                    1.0,
                )
            } else {
                fill_path(path, m0, fill, 1.0)
            };
            if let Some(item) = item {
                scene.items.push(item);
            }
        }
        let content = pm_eval::captions::measure_content(style, layout.size);
        let tracking = content.tracking.unwrap_or(0.0);
        let ascent = layout.size * 0.82 + (layout.line_height - layout.size) / 2.0;
        let stroke = if style.stroke > 0.0 {
            style.stroke * layout.size / style.size
        } else {
            0.0
        };
        // `strokeStyle` is set inside each word's `save()`; a rejected color
        // keeps the canvas default, black.
        let stroke_color =
            hex_to_rgba(&style.stroke_color.to_string_lossy(), 1.0).unwrap_or(color::BLACK);
        let canvas_level1 = self.profile == Profile::CANVAS2D;
        let (mut painter, fonts, unsupported) = self.painter((scene.width, scene.height));
        for line in &layout.lines {
            for word in &line.words {
                let state =
                    f.states
                        .get(word.index)
                        .copied()
                        .unwrap_or(pm_eval::captions::WordState {
                            alpha: 1.0,
                            scale: 1.0,
                            highlighted: false,
                        });
                if state.alpha <= 0.0 {
                    continue;
                }
                let x = pad + word.x;
                let y = pad + line.y + ascent;
                let mut m = m0;
                if state.scale != 1.0 {
                    let cx = x + word.width / 2.0;
                    let cy = pad + line.y + layout.line_height / 2.0;
                    m = if canvas_level1 {
                        canvas::translate(
                            canvas::scale(canvas::translate(m, cx, cy), state.scale, state.scale),
                            -cx,
                            -cy,
                        )
                    } else {
                        m * Affine::translate((cx, cy))
                            * Affine::scale(state.scale)
                            * Affine::translate((-cx, -cy))
                    };
                }
                let text = word.text.to_string_lossy();
                let runs = fonts.text_runs(&content, &text, tracking);
                let faces = run_faces(fonts, &runs, m);
                let refs = run_refs(&runs, &faces);
                if stroke > 0.0 {
                    painter.stroke_text(
                        &mut scene.items,
                        &refs,
                        (x, y),
                        m,
                        stroke * 2.0,
                        stroke_color,
                        state.alpha,
                    );
                }
                let fill = if state.highlighted {
                    &style.highlight_color
                } else {
                    &style.fill
                };
                let fill = css_color(&fill.to_string_lossy(), restored, &mut painter.unsupported);
                painter.fill_text(&mut scene.items, &refs, (x, y), m, fill, state.alpha, 0.0);
            }
        }
        unsupported.append(&mut painter.unsupported);
    }
}

/// The faces of `runs` and, for color bitmap faces, the size their glyphs
/// draw at under the device transform `m`.
fn run_faces(
    fonts: &mut dyn TextFonts,
    runs: &[GlyphRun],
    m: Affine,
) -> Vec<(FaceInfo, Option<f32>)> {
    let [a, b, c, d, _, _] = m.as_coeffs();
    let scale = (a * d - b * c).abs().sqrt();
    runs.iter()
        .map(|r| {
            let info = fonts.face(r.face).clone();
            let bitmap = if info.facts.has_sbix {
                fonts.em_advance(r.face, (f64::from(r.size) * scale) as f32)
            } else {
                None
            };
            (info, bitmap)
        })
        .collect()
}

fn run_refs<'a>(
    runs: &'a [GlyphRun],
    faces: &'a [(FaceInfo, Option<f32>)],
) -> Vec<pm_text::render::scene::Run<'a>> {
    runs.iter()
        .zip(faces)
        .map(|(r, (f, bitmap))| pm_text::render::scene::Run {
            face: pm_text::render::outline::FaceRef::of(f),
            size: r.size,
            coords: pm_text::render::outline::Coords::User(&r.coords),
            synthetic_bold: r.synthetic_bold,
            synthetic_italic: r.synthetic_italic,
            oblique: 0.0,
            glyphs: &r.glyphs,
            bitmap_size: *bitmap,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raster_scale_matches_the_compositor() {
        assert_eq!(
            continuous_raster_scale(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            1.0
        );
        assert_eq!(
            continuous_raster_scale(&[0.5, 0.0, 0.0, 0.5, 9.0, 9.0]),
            0.5
        );
        assert_eq!(
            continuous_raster_scale(&[1.1, 0.0, 0.0, 1.1, 0.0, 0.0]),
            1.25
        );
        assert_eq!(
            continuous_raster_scale(&[0.01, 0.0, 0.0, 0.01, 0.0, 0.0]),
            0.25
        );
        assert_eq!(
            continuous_raster_scale(&[100.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            32.0
        );
        let r = std::f64::consts::FRAC_PI_4;
        let m = [2.0 * r.cos(), 2.0 * r.sin(), -r.sin(), r.cos(), 0.0, 0.0];
        assert_eq!(continuous_raster_scale(&m), 2.0);
        assert_eq!(
            continuous_raster_scale(&[f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]),
            1.0
        );
    }

    #[test]
    fn caption_colors_follow_hex_to_rgba() {
        assert_eq!(hex_to_rgba("#000000", 0.72), Some([0.0, 0.0, 0.0, 0.72]));
        assert_eq!(hex_to_rgba("#00000099", 1.0), Some([0.0, 0.0, 0.0, 0.6]));
        assert_eq!(hex_to_rgba("#FFFFFF", 0.94), Some([1.0, 1.0, 1.0, 0.94]));
        assert_eq!(hex_to_rgba("FF0000", 2.0), Some([1.0, 0.0, 0.0, 1.0]));
        // `rgba(255,15,NaN,1)`, `rgba(NaN,...)` and `rgba(...,NaN)`: the
        // canvas rejects them.
        assert_eq!(hex_to_rgba("#fff", 1.0), None);
        assert_eq!(hex_to_rgba("red", 1.0), None);
        assert_eq!(hex_to_rgba("#000000zz", 1.0), None);
        assert_eq!(hex_to_rgba("#000000", f64::NAN), None);
        // `parseInt` takes a hex prefix: `1g` is 1.
        assert_eq!(
            hex_to_rgba("#1g0000", 1.0),
            Some([1.0 / 255.0, 0.0, 0.0, 1.0])
        );
    }

    /// A font change (a new generation from the registry, or new fonts)
    /// drops every glyph cache, platform masks and their CTFonts included,
    /// and makes later raster keys new.
    #[test]
    fn font_changes_drop_glyph_caches() {
        use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
        struct Fonts(Arc<AtomicU64>);
        impl TextFonts for Fonts {
            fn measure(&mut self) -> &mut dyn TextMeasure {
                unreachable!("not laid out")
            }
            fn line_runs(&mut self, _: &TextContent) -> Vec<LineRuns> {
                unreachable!("not laid out")
            }
            fn text_runs(&mut self, _: &TextContent, _: &str, _: f64) -> Vec<GlyphRun> {
                unreachable!("not laid out")
            }
            fn face(&mut self, _: FaceId) -> &FaceInfo {
                unreachable!("not laid out")
            }
            fn generation(&self) -> u64 {
                self.0.load(Ordering::SeqCst)
            }
        }
        struct Masks(Arc<AtomicUsize>);
        impl pm_text::render::scene::MaskSource for Masks {
            fn mask(
                &mut self,
                _: pm_text::render::outline::FaceRef<'_>,
                _: &[(u32, f32)],
                _: u16,
                _: f32,
                _: pm_text::render::scene::MaskStyle,
                _: kurbo::Rect,
            ) -> Option<Arc<pm_text::render::scene::GlyphMask>> {
                None
            }
            fn preblend(&self, _: [f64; 3]) -> [u8; 256] {
                [0; 256]
            }
            fn reset(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let generation = Arc::new(AtomicU64::new(1));
        let resets = Arc::new(AtomicUsize::new(0));
        let mut cx = TextContext::default();
        cx.set_masks(Some(Box::new(Masks(resets.clone()))));
        cx.set_fonts(Some(Box::new(Fonts(generation.clone()))));
        assert_eq!(
            resets.load(Ordering::SeqCst),
            1,
            "new fonts reset the masks"
        );
        let key = cx.generation;
        cx.sync_fonts();
        assert_eq!((resets.load(Ordering::SeqCst), cx.generation), (1, key));
        generation.store(2, Ordering::SeqCst);
        cx.sync_fonts();
        assert_eq!(
            resets.load(Ordering::SeqCst),
            2,
            "a new generation resets the masks"
        );
        assert_ne!(cx.generation, key, "and re-keys rasters");
        cx.sync_fonts();
        assert_eq!(resets.load(Ordering::SeqCst), 2);
    }
}
