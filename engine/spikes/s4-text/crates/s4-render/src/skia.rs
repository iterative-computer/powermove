//! Skia Graphite on Metal (skia-safe 0.153, Skia m153) into a half-float
//! (`RGBAF16`) surface. Typefaces come from the same font bytes through the
//! platform font manager, which on macOS is CoreText-backed: Skia's
//! CoreText scaler rasterizes small glyphs, as Chromium's does for Canvas2D;
//! large glyphs and paths go through Graphite's path renderers.

use crate::image::Image;
use crate::prepare::{Draw, FaceData, Prepared};
use crate::scene::Color;
use crate::vello_r::FrameTime;
use kurbo::PathEl;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{MTLCommandQueue, MTLCreateSystemDefaultDevice, MTLDevice};
use s4_layout::font::FaceId;
use skia_safe::font_arguments::{VariationPosition, variation_position::Coordinate};
use skia_safe::gpu::graphite::{self, mtl};
use skia_safe::{AlphaType, Color4f, ColorSpace, ColorType, Data, Font, FontArguments, FontHinting, FontMgr, FourByteTag, ImageInfo, Paint, PaintJoin, PaintStyle, PathBuilder, PathFillType, Point, Typeface, font::Edging};
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

pub struct SkiaGraphite {
    _device: Retained<ProtocolObject<dyn MTLDevice>>,
    _queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    _backend: mtl::BackendContext,
    context: graphite::Context,
    recorder: graphite::Recorder,
    font_mgr: FontMgr,
    typefaces: HashMap<FaceId, Typeface>,
    variants: HashMap<(FaceId, Vec<(u32, u32)>), Typeface>,
    surface: Option<skia_safe::Surface>,
}

fn color4f(c: &Color) -> Color4f {
    Color4f::new(c[0], c[1], c[2], c[3])
}

fn paint(c: &Color) -> Paint {
    let mut p = Paint::new(color4f(c), Some(&ColorSpace::new_srgb()));
    p.set_anti_alias(true);
    p
}

impl SkiaGraphite {
    pub fn new() -> Self {
        let device = MTLCreateSystemDefaultDevice().expect("Metal device");
        let queue = device.newCommandQueue().expect("Metal queue");
        let backend = unsafe {
            mtl::BackendContext::new(Retained::as_ptr(&device) as *mut std::ffi::c_void, Retained::as_ptr(&queue) as *mut std::ffi::c_void)
        };
        let mut context = mtl::context_factory::make_metal(&backend, None).expect("Graphite Metal context");
        let recorder = context.make_recorder(None).expect("Graphite recorder");
        SkiaGraphite { _device: device, _queue: queue, _backend: backend, context, recorder, font_mgr: FontMgr::new(), typefaces: HashMap::new(), variants: HashMap::new(), surface: None }
    }

    fn typeface(&mut self, faces: &BTreeMap<FaceId, FaceData>, face: FaceId, coords: &[(u32, f32)]) -> Typeface {
        let key = (face, coords.iter().map(|(t, v)| (*t, v.to_bits())).collect::<Vec<_>>());
        if let Some(t) = self.variants.get(&key) {
            return t.clone();
        }
        let base = self
            .typefaces
            .entry(face)
            .or_insert_with(|| {
                let f = &faces[&face];
                // Skia's CoreText font manager loads only face 0 of data;
                // collection members are rewritten as a single-face sfnt
                // with the same tables.
                let bytes = if f.index == 0 { f.data.to_vec() } else { single_face(&f.data, f.index) };
                self.font_mgr.new_from_data(Data::new_copy(&bytes), 0).unwrap_or_else(|| panic!("Skia cannot read {}", f.postscript_name))
            })
            .clone();
        let tf = if coords.is_empty() {
            base
        } else {
            let cs: Vec<Coordinate> = coords.iter().map(|(t, v)| Coordinate { axis: FourByteTag::new(*t), value: *v }).collect();
            base.clone_with_arguments(&FontArguments::new().set_variation_design_position(VariationPosition { coordinates: &cs })).expect("variation instance")
        };
        self.variants.insert(key, tf.clone());
        tf
    }

    fn ensure_surface(&mut self, w: u32, h: u32) {
        if self.surface.as_ref().is_some_and(|s| s.width() == w as i32 && s.height() == h as i32) {
            return;
        }
        let info = ImageInfo::new((w as i32, h as i32), ColorType::RGBAF16, AlphaType::Premul, Some(ColorSpace::new_srgb()));
        self.surface = Some(graphite::surfaces::render_target(&mut self.recorder, &info, skia_safe::gpu::Mipmapped::No, None, Some("s4")).expect("RGBAF16 Graphite surface"));
    }

    fn draw(&mut self, scene: &Prepared, faces: &BTreeMap<FaceId, FaceData>) {
        self.ensure_surface(scene.width, scene.height);
        // Resolve typefaces first; the canvas borrows the surface.
        let mut fonts = Vec::new();
        for draw in &scene.draws {
            if let Draw::Text { runs, .. } = draw {
                for run in runs {
                    let tf = self.typeface(faces, run.face, &run.coords);
                    let mut font = Font::from_typeface(tf, run.size);
                    // Chromium's Canvas2D text on macOS: subpixel positioning,
                    // grayscale antialiasing, no hinting, linear metrics.
                    font.set_subpixel(true);
                    font.set_edging(Edging::AntiAlias);
                    font.set_hinting(FontHinting::None);
                    font.set_linear_metrics(true);
                    fonts.push(font);
                }
            }
        }
        let canvas = self.surface.as_mut().unwrap().canvas();
        canvas.clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
        let mut fi = 0;
        for draw in &scene.draws {
            match draw {
                Draw::Text { runs, color, stroke, stroke_color } => {
                    let first = fi;
                    for pass in [true, false] {
                        if pass && *stroke <= 0.0 {
                            continue;
                        }
                        fi = first;
                        let mut p = paint(if pass { stroke_color } else { color });
                        if pass {
                            p.set_style(PaintStyle::Stroke);
                            p.set_stroke_width((*stroke * 2.0) as f32);
                            p.set_stroke_join(PaintJoin::Round);
                            p.set_stroke_miter(2.0);
                        }
                        for run in runs {
                            let ids: Vec<u16> = run.glyphs.iter().map(|g| g.id as u16).collect();
                            let pos: Vec<Point> = run.glyphs.iter().map(|g| Point::new(g.x as f32, g.y as f32)).collect();
                            canvas.draw_glyphs_at(&ids, pos.as_slice(), Point::new(0.0, 0.0), &fonts[fi], &p);
                            fi += 1;
                        }
                    }
                }
                Draw::Path { path, color, even_odd, stroke, round_join } => {
                    let mut b = PathBuilder::new();
                    for el in path.elements() {
                        match *el {
                            PathEl::MoveTo(p) => _ = b.move_to((p.x as f32, p.y as f32)),
                            PathEl::LineTo(p) => _ = b.line_to((p.x as f32, p.y as f32)),
                            PathEl::QuadTo(a, p) => _ = b.quad_to((a.x as f32, a.y as f32), (p.x as f32, p.y as f32)),
                            PathEl::CurveTo(a, c, p) => _ = b.cubic_to((a.x as f32, a.y as f32), (c.x as f32, c.y as f32), (p.x as f32, p.y as f32)),
                            PathEl::ClosePath => _ = b.close(),
                        }
                    }
                    let mut sp = b.detach();
                    sp.set_fill_type(if *even_odd { PathFillType::EvenOdd } else { PathFillType::Winding });
                    let mut p = paint(color);
                    if *stroke > 0.0 {
                        p.set_style(PaintStyle::Stroke);
                        p.set_stroke_width(*stroke as f32);
                        p.set_stroke_join(if *round_join { PaintJoin::Round } else { PaintJoin::Miter });
                    }
                    canvas.draw_path(&sp, &p);
                }
            }
        }
    }

    fn submit(&mut self) {
        let mut recording = self.recorder.snap().expect("recording");
        self.context.insert_recording(&graphite::InsertRecordingInfo::new(&mut recording));
        assert!(self.context.submit_and_wait(), "Graphite submit");
    }

    pub fn frame(&mut self, scene: &Prepared, faces: &BTreeMap<FaceId, FaceData>) -> FrameTime {
        let started = Instant::now();
        self.draw(scene, faces);
        let encode = started.elapsed();
        self.submit();
        FrameTime { encode, total: started.elapsed() }
    }

    pub fn render(&mut self, scene: &Prepared, faces: &BTreeMap<FaceId, FaceData>) -> Image {
        self.frame(scene, faces);
        let (w, h) = (scene.width as i32, scene.height as i32);
        let info = ImageInfo::new((w, h), ColorType::RGBAF32, AlphaType::Premul, Some(ColorSpace::new_srgb()));
        let mut img = Image::new(scene.width, scene.height);
        let bytes: &mut [u8] = bytemuck_f32(&mut img.data);
        let ok = self.context.read_pixels(self.surface.as_mut().unwrap(), &info, bytes, w as usize * 16, (0, 0));
        assert!(ok, "Graphite read_pixels");
        img
    }
}

impl Default for SkiaGraphite {
    fn default() -> Self {
        Self::new()
    }
}

fn single_face(data: &[u8], index: u32) -> Vec<u8> {
    let file = read_fonts::FileRef::new(data).expect("font file");
    let font = file.fonts().nth(index as usize).expect("face index").expect("face");
    let mut tables: Vec<(u32, Vec<u8>)> = font
        .table_directory()
        .table_records()
        .iter()
        .filter_map(|r| Some((u32::from_be_bytes(r.tag().to_be_bytes()), font.table_data(r.tag())?.as_bytes().to_vec())))
        .collect();
    s4_layout::ots::build_sfnt(&mut tables)
}

fn bytemuck_f32(v: &mut [f32]) -> &mut [u8] {
    // f32 has no padding and u8 has alignment 1.
    unsafe { std::slice::from_raw_parts_mut(v.as_mut_ptr().cast::<u8>(), std::mem::size_of_val(v)) }
}
