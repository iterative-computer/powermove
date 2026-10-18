//! Writes Jude's text review sheets (plan section 12: "CoreText
//! side-by-side corpus reviewed by Jude"): for every text case of a
//! rendering capture (`text-raster-*`), one PNG with four rows at the same
//! pixels, and an `index.html` with their scores:
//!
//! 1. today's Canvas2D (the capture),
//! 2. the engine at `compat` level 1 (Canvas2D profile),
//! 3. CoreText with font smoothing (the macOS look),
//! 4. the engine at level 2 (CoreText profile, exact positions).
//!
//! Images are written outside the repository, to
//! `~/Library/Application Support/Powermove Engine Corpus/review/text-<stamp>/`
//! (or the directory given). Each row is composited over a neutral
//! background that contrasts with the text color, and shown at 2× for
//! sizes under 24 px.
//!
//! cargo run --release -p pm-render --example text_review -- [capture dir] [out dir]

#![recursion_limit = "256"]

#[cfg(target_os = "macos")]
mod review {
    use kurbo::Affine;
    use pm_protocol::color::Precision;
    use pm_render::Renderer;
    use pm_text::compat::TextContent;
    use pm_text::render::bitmap::Bitmaps;
    use pm_text::render::coretext::{CoreTextReference, RefRun};
    use pm_text::render::outline::{Coords, FaceRef, Outlines};
    use pm_text::render::profile::Profile;
    use pm_text::render::scene::{Painter, Run, TextScene};
    use serde_json::{Value, json};
    use std::fmt::Write as _;
    use std::path::PathBuf;

    struct Fonts(pm_eval::text_measure::CompatMeasure<pm_text::registry::CoreTextRegistry>);
    // SAFETY: the tool is single-threaded and runs on the main thread.
    unsafe impl Send for Fonts {}
    impl pm_render::text::TextFonts for Fonts {
        fn measure(&mut self) -> &mut dyn pm_eval::text_geom::TextMeasure {
            &mut self.0
        }
        fn line_runs(&mut self, d: &TextContent) -> Vec<pm_text::compat::LineRuns> {
            self.0.compat().line_runs(d)
        }
        fn text_runs(
            &mut self,
            d: &TextContent,
            text: &str,
            tracking: f64,
        ) -> Vec<pm_text::compat::GlyphRun> {
            self.0.compat().text_runs(d, text, tracking).0
        }
        fn face(&mut self, id: pm_text::font::FaceId) -> &pm_text::font::FaceInfo {
            use pm_text::font::FontResolver;
            self.0.compat().resolver().face(id)
        }
        fn em_advance(&mut self, face: pm_text::font::FaceId, size: f32) -> Option<f32> {
            use pm_text::font::FontResolver;
            self.0.compat().resolver_mut().em_advance(face, size)
        }
    }

    fn ch(v: Value) -> Value {
        json!({ "v": v, "kf": [], "expr": null })
    }

    fn document(d: &Value) -> pm_model::Document {
        let p = json!({
            "perspective": ch(json!(50)), "position.z": ch(json!(0)), "anchor.z": ch(json!(0)),
            "scale.z": ch(json!(100)), "rotation.x": ch(json!(0)), "rotation.y": ch(json!(0)),
            "orientation.x": ch(json!(0)), "orientation.y": ch(json!(0)), "orientation.z": ch(json!(0)),
            "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(960)),
            "position.y": ch(json!(540)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
            "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
        });
        let root = json!({
            "id": "P", "name": "review", "w": 1920, "h": 1080, "fps": 30, "dur": 10, "bg": "#000000",
            "compId": "P-root", "compName": "review", "markers": [], "work": [0, 10], "params": {},
            "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "assets": {}, "comps": {},
            "layers": [{
                "id": "l", "type": "text", "name": "l", "from": 0, "dur": 10, "on": true, "lock": false,
                "shy": false, "collapsed": true, "threeD": false, "solo": false, "color": "#4C8DFF",
                "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
                "p": p, "fx": [], "transitionIn": null, "transitionOut": null, "masks": [], "d": d
            }]
        });
        pm_model::Document::new(Some(pm_model::json::parse(&root.to_string()).unwrap()), 1)
    }

    /// Premultiplied RGBA (0..1) over an opaque background, RGB8.
    fn over(px: &[f32], bg: [f32; 3]) -> Vec<u8> {
        px.chunks_exact(4)
            .flat_map(|p| {
                let a = p[3];
                std::array::from_fn::<u8, 3, _>(|c| {
                    ((p[c] + bg[c] * (1.0 - a)) * 255.0)
                        .round()
                        .clamp(0.0, 255.0) as u8
                })
            })
            .collect()
    }

    fn png(path: &std::path::Path, w: u32, h: u32, rgb: &[u8]) {
        let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
        let mut e = png::Encoder::new(file, w, h);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(rgb).unwrap();
    }

    pub fn run(capture: PathBuf, out: PathBuf) {
        let cases: Value =
            serde_json::from_str(&std::fs::read_to_string(capture.join("cases.json")).unwrap())
                .unwrap();
        let manifest: Value =
            serde_json::from_str(&std::fs::read_to_string(capture.join("manifest.json")).unwrap())
                .unwrap();
        let corpus = pm_text::corpus::CorpusFile::from_value(&json!({
            "variableSources": cases["variableSources"], "projectFonts": cases["projectFonts"], "cases": [],
        }))
        .unwrap();
        let mut renderer = Renderer::headless().expect("a Metal device");
        renderer.set_text_fonts(Some(Box::new(Fonts(
            pm_eval::text_measure::CompatMeasure::new(
                pm_text::corpus::native_registry(&corpus).unwrap(),
            ),
        ))));
        let mut fonts = Fonts(pm_eval::text_measure::CompatMeasure::new(
            pm_text::corpus::native_registry(&corpus).unwrap(),
        ));
        let mut reference = CoreTextReference::new();
        let (mut outlines, mut bitmaps) = (Outlines::new(), Bitmaps::new());
        std::fs::create_dir_all(&out).unwrap();
        let mut html = String::from(
            "<!doctype html><meta charset=utf-8><title>Text review</title><style>body{font:13px system-ui;background:#111;color:#ddd}figure{margin:16px 0}img{image-rendering:pixelated;display:block}figcaption{margin:4px 0 12px}</style>\
             <h1>Text review</h1><p>Rows, top to bottom: today's Canvas2D, engine level 1 (CoreGraphics glyph masks with Canvas2D's tables on macOS; the Canvas2D profile for what they do not cover), CoreText with font smoothing, engine level 2 (CoreText profile). Scores are ink SSIM.</p>\
             <p>Cases marked <em>emoji</em> hold color bitmap glyphs (Apple Color Emoji). At both levels the engine places and sizes them as Canvas2D does (amendment 10), not as CoreText does, so their level 2 vs CoreText score is low by design and is not a failure: compare their level 2 row with the Canvas2D row instead.</p>",
        );
        let mut n = 0;
        for (case, shot) in cases["cases"]
            .as_array()
            .unwrap()
            .iter()
            .zip(manifest["cases"].as_array().unwrap())
        {
            if case["kind"] != "text" || shot.get("error").is_some() {
                continue;
            }
            let id = case["id"].as_str().unwrap();
            let (w, h) = (
                shot["width"].as_u64().unwrap() as u32,
                shot["height"].as_u64().unwrap() as u32,
            );
            if w > 2400 {
                continue;
            }
            let g = &shot["geometry"];
            let density = g["density"].as_f64().unwrap();
            let (gx, pad) = (g["x"].as_f64().unwrap(), g["pad"].as_f64().unwrap());
            let d = &case["d"];
            let color = pm_text::render::color::parse_or(
                d["color"].as_str().unwrap_or("#fff"),
                [0.0, 0.0, 0.0, 1.0],
            );
            let luma = 0.2126 * color[0] + 0.7152 * color[1] + 0.0722 * color[2];
            let bg = if luma > 0.5 {
                [0.13f32; 3]
            } else {
                [0.88f32; 3]
            };
            let reference_px: Vec<f32> = std::fs::read(capture.join(format!("{id}.rgba")))
                .unwrap()
                .iter()
                .map(|v| f32::from(*v) / 255.0)
                .collect();
            // Level 1 through the renderer.
            let mut level1 = vec![0.0f32; (w * h * 4) as usize];
            if let Ok(Some(r)) = renderer.text_raster(
                &document(d),
                "P-root",
                "l",
                0.0,
                case["scale"].as_f64().unwrap(),
                Precision::Float32,
            ) {
                let [x0, y0, x1, _] = r.plan.crop.map(|v| v as usize);
                let cw = x1 - x0;
                for (row, chunk) in r.pixels.chunks_exact(cw * 4).enumerate() {
                    let y = y0 + row;
                    if y >= h as usize {
                        break;
                    }
                    let n = cw.min(w as usize - x0);
                    level1[(y * w as usize + x0) * 4..(y * w as usize + x0 + n) * 4]
                        .copy_from_slice(&chunk[..n * 4]);
                }
            }
            // Level 2 and CoreText at the same glyph positions.
            let tc = TextContent::from_json(d);
            let lines = {
                use pm_render::text::TextFonts;
                fonts.line_runs(&tc)
            };
            let faces: Vec<Vec<pm_text::font::FaceInfo>> = lines
                .iter()
                .map(|l| {
                    l.runs
                        .iter()
                        .map(|r| {
                            use pm_render::text::TextFonts;
                            fonts.face(r.face).clone()
                        })
                        .collect()
                })
                .collect();
            let mut scene = TextScene {
                width: w,
                height: h,
                items: Vec::new(),
            };
            let mut painter = Painter {
                outlines: &mut outlines,
                bitmaps: &mut bitmaps,
                masks: None,
                profile: Profile::CORETEXT,
                unsupported: Vec::new(),
            };
            let mut refs: Vec<RefRun<'_>> = Vec::new();
            for (line, faces) in lines.iter().zip(&faces) {
                let runs: Vec<Run<'_>> = line
                    .runs
                    .iter()
                    .zip(faces)
                    .map(|(r, f)| Run {
                        face: FaceRef::of(f),
                        size: r.size,
                        coords: Coords::User(&r.coords),
                        synthetic_bold: r.synthetic_bold,
                        synthetic_italic: r.synthetic_italic,
                        oblique: 0.0,
                        glyphs: &r.glyphs,
                        bitmap_size: None,
                    })
                    .collect();
                painter.fill_text(
                    &mut scene.items,
                    &runs,
                    (gx, pad),
                    Affine::scale(density),
                    color,
                    1.0,
                    0.0,
                );
                for (r, f) in line.runs.iter().zip(faces) {
                    refs.push(RefRun {
                        data: f.data.as_slice(),
                        index: f.index,
                        size: (f64::from(r.size) * density) as f32,
                        coords: &r.coords,
                        glyphs: r
                            .glyphs
                            .iter()
                            .map(|p| (p.id as u16, (gx + p.x) * density, (pad + p.y) * density))
                            .collect(),
                        color,
                    });
                }
            }
            let level2 = renderer
                .paint_text_scene(scene, Precision::Float32)
                .unwrap();
            let coretext = reference.render(&refs, w, h, true);
            let to8 = |p: &[f32]| -> Vec<u8> {
                p.iter()
                    .map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8)
                    .collect()
            };
            let (s1, ..) = pm_render::parity::text_ssim(
                &to8(&reference_px),
                &to8(&level1),
                w as usize,
                h as usize,
            );
            let (s2, ..) = pm_render::parity::text_ssim(
                &to8(&coretext),
                &to8(&level2),
                w as usize,
                h as usize,
            );
            // Stack the rows, 2× below 24 px.
            let zoom = if d["size"].as_f64().unwrap_or(16.0) * density < 24.0 {
                2
            } else {
                1
            };
            let (zw, zh) = (w * zoom, h * zoom);
            let mut sheet = Vec::with_capacity((zw * zh * 4 * 3) as usize);
            for img in [&reference_px, &level1, &coretext, &level2] {
                let rgb = over(img, bg);
                for y in 0..zh {
                    for x in 0..zw {
                        let o = (((y / zoom) * w + x / zoom) * 3) as usize;
                        sheet.extend_from_slice(&rgb[o..o + 3]);
                    }
                }
            }
            let emoji = faces
                .iter()
                .flatten()
                .any(|f| pm_text::render::bitmap::has_bitmaps(FaceRef::of(f)));
            let note = if emoji {
                " (emoji: Canvas2D placement at level 2 by design; compare row 4 with row 1)"
            } else {
                ""
            };
            let name = format!("{id}.png");
            png(&out.join(&name), zw, zh * 4, &sheet);
            let _ = write!(
                html,
                "<figure><img src=\"{name}\" width=\"{zw}\"><figcaption>{id} — {} {} px, weight {}, density {density}: level 1 vs Canvas2D {s1:.4}, level 2 vs CoreText {s2:.4}{note}</figcaption></figure>",
                d["font"].as_str().unwrap_or(""),
                d["size"],
                d["weight"]
            );
            n += 1;
        }
        std::fs::write(out.join("index.html"), html).unwrap();
        println!("wrote {n} review sheets to {}", out.display());
    }
}

#[cfg(target_os = "macos")]
fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(std::env::var("HOME").unwrap())
        .join("Library/Application Support/Powermove Engine Corpus");
    let capture = args
        .next()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let mut caps: Vec<_> = std::fs::read_dir(&root)
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("text-raster-"))
                })
                .collect();
            caps.sort();
            caps.pop().expect("a text-raster capture")
        });
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let out = args
        .next()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("review").join(format!("text-{stamp}")));
    review::run(capture, out);
}

#[cfg(not(target_os = "macos"))]
fn main() {}
