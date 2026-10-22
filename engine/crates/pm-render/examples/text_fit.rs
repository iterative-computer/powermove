//! Fits the level 1 (Canvas2D) stem-darkening profile against a capture of
//! the fitting set (`pm-text/capture/render.ts --set fit`), through the
//! production raster path (`Renderer::text_raster`).
//!
//! cargo run --release -p pm-render --example text_fit -- <capture dir> <mode> [profile.json]
//!
//! Modes:
//! - `sizes`: per device size, coordinate search of expansion x and y, the
//!   vertical offset, and the coverage exponent on the white `fit` cases;
//!   prints the table and writes `<capture>/fit-profile.json`.
//! - `dark`: the dark-paint exponent on `fit-color`.
//! - `stroke`: stroke widening and exponent on `fit-stroke`.
//! - `emoji`: ink boxes of `fit-emoji` (reference against ours).
//! - `score`: every group with the given profile.
//!
//! Runs on the main thread (the CoreText registry).

#[cfg(target_os = "macos")]
mod fit {
    use pm_protocol::color::Precision;
    use pm_render::Renderer;
    use pm_text::render::profile::{Point, Profile};
    use serde_json::{Value, json};
    use std::borrow::Cow;
    use std::path::{Path, PathBuf};

    pub struct MainThreadFonts(
        pub pm_eval::text_measure::CompatMeasure<pm_text::registry::CoreTextRegistry>,
    );
    // SAFETY: the tool is single-threaded and runs on the main thread.
    unsafe impl Send for MainThreadFonts {}
    impl pm_render::text::TextFonts for MainThreadFonts {
        fn measure(&mut self) -> &mut dyn pm_eval::text_geom::TextMeasure {
            &mut self.0
        }
        fn line_runs(
            &mut self,
            d: &pm_text::compat::TextContent,
        ) -> Vec<pm_text::compat::LineRuns> {
            self.0.compat().line_runs(d)
        }
        fn text_runs(
            &mut self,
            d: &pm_text::compat::TextContent,
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

    pub struct Case {
        pub id: String,
        pub group: String,
        pub size: f64,
        pub color: String,
        pub doc: pm_model::Document,
        pub layer: &'static str,
        pub time: f64,
        pub scale: f64,
        pub width: usize,
        pub height: usize,
        pub reference: Vec<u8>,
    }

    fn ch(v: Value) -> Value {
        json!({ "v": v, "kf": [], "expr": null })
    }

    fn document(w: u32, h: u32, ty: &str, d: Value) -> pm_model::Document {
        let p = json!({
            "perspective": ch(json!(50)), "position.z": ch(json!(0)), "anchor.z": ch(json!(0)),
            "scale.z": ch(json!(100)), "rotation.x": ch(json!(0)), "rotation.y": ch(json!(0)),
            "orientation.x": ch(json!(0)), "orientation.y": ch(json!(0)), "orientation.z": ch(json!(0)),
            "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(w / 2)),
            "position.y": ch(json!(h / 2)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
            "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
        });
        let layer = json!({
            "id": "l", "type": ty, "name": "l", "from": 0, "dur": 10, "on": true, "lock": false,
            "shy": false, "collapsed": true, "threeD": false, "solo": false, "color": "#4C8DFF",
            "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
            "p": p, "fx": [], "transitionIn": null, "transitionOut": null, "masks": [], "d": d
        });
        let root = json!({
            "id": "P", "name": "fit", "w": w, "h": h, "fps": 30, "dur": 10, "bg": "#000000",
            "compId": "P-root", "compName": "fit", "markers": [], "work": [0, 10], "params": {},
            "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "layers": [layer],
            "assets": {}, "comps": {}
        });
        pm_model::Document::new(Some(pm_model::json::parse(&root.to_string()).unwrap()), 1)
    }

    pub fn load(dir: &Path) -> (Renderer, Vec<Case>) {
        let cases: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("cases.json")).unwrap())
                .unwrap();
        let manifest: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap())
                .unwrap();
        let corpus = pm_text::corpus::CorpusFile::from_value(&json!({
            "variableSources": cases["variableSources"], "projectFonts": cases["projectFonts"], "cases": [],
        }))
        .unwrap();
        let registry = pm_text::corpus::native_registry(&corpus).expect("registry");
        let mut renderer = Renderer::headless().expect("a Metal device");
        renderer.set_text_fonts(Some(Box::new(MainThreadFonts(
            pm_eval::text_measure::CompatMeasure::new(registry),
        ))));
        let mut out = Vec::new();
        for (c, shot) in cases["cases"]
            .as_array()
            .unwrap()
            .iter()
            .zip(manifest["cases"].as_array().unwrap())
        {
            if shot.get("empty").is_some() || shot.get("error").is_some() {
                continue;
            }
            let id = c["id"].as_str().unwrap().to_owned();
            let (doc, layer, time, size, color) = if c["kind"] == "text" {
                (
                    document(1920, 1080, "text", c["d"].clone()),
                    "l",
                    0.0,
                    c["d"]["size"].as_f64().unwrap() * c["scale"].as_f64().unwrap(),
                    c["d"]["color"].as_str().unwrap_or("#FFFFFF").to_owned(),
                )
            } else {
                (
                    document(
                        c["compositionWidth"].as_u64().unwrap() as u32,
                        c["compositionHeight"].as_u64().unwrap() as u32,
                        "captions",
                        c["content"].clone(),
                    ),
                    "l",
                    c["local"].as_f64().unwrap(),
                    c["content"]["style"]["size"].as_f64().unwrap() * c["scale"].as_f64().unwrap(),
                    "#FFFFFF".to_owned(),
                )
            };
            out.push(Case {
                group: c["group"].as_str().unwrap().to_owned(),
                size,
                color,
                doc,
                layer,
                time,
                scale: c["scale"].as_f64().unwrap(),
                width: shot["width"].as_u64().unwrap() as usize,
                height: shot["height"].as_u64().unwrap() as usize,
                reference: std::fs::read(dir.join(format!("{id}.rgba"))).unwrap(),
                id,
            });
        }
        (renderer, out)
    }

    pub fn ours(renderer: &mut Renderer, c: &Case) -> Vec<u8> {
        let mut out = vec![0u8; c.width * c.height * 4];
        let Some(r) = renderer
            .text_raster(
                &c.doc,
                "P-root",
                c.layer,
                c.time,
                c.scale,
                Precision::Float16,
            )
            .unwrap()
        else {
            return out;
        };
        let [x0, y0, x1, y1] = r.plan.crop.map(|v| v as usize);
        let w = x1 - x0;
        for y in y0..y1.min(c.height) {
            for x in x0..x1.min(c.width) {
                for k in 0..4 {
                    let v = r.pixels[((y - y0) * w + (x - x0)) * 4 + k];
                    out[(y * c.width + x) * 4 + k] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        out
    }

    pub fn score(renderer: &mut Renderer, c: &Case) -> (f64, f64) {
        let o = ours(renderer, c);
        let (ink, _, ratio) = pm_render::parity::text_ssim(&c.reference, &o, c.width, c.height);
        (ink, ratio)
    }

    fn mean(renderer: &mut Renderer, cases: &[&Case]) -> f64 {
        let s: f64 = cases.iter().map(|c| score(renderer, c).0).sum();
        s / cases.len().max(1) as f64
    }

    fn constant(base: &Profile, p: Point) -> Profile {
        Profile {
            points: Cow::Owned(vec![p]),
            ..base.clone()
        }
    }

    /// Coordinate search maximizing `f` over `x` with initial steps `step`.
    fn search(
        x: &mut [f64],
        step: &mut [f64],
        lo: &[f64],
        hi: &[f64],
        f: &mut dyn FnMut(&[f64]) -> f64,
    ) -> f64 {
        let mut best = f(x);
        for _ in 0..60 {
            let mut improved = false;
            for i in 0..x.len() {
                for dir in [1.0, -1.0] {
                    let mut y = x.to_vec();
                    y[i] = (y[i] + dir * step[i]).clamp(lo[i], hi[i]);
                    if y[i] == x[i] {
                        continue;
                    }
                    let v = f(&y);
                    if v > best + 1e-6 {
                        best = v;
                        x.copy_from_slice(&y);
                        improved = true;
                        break;
                    }
                }
            }
            if !improved {
                for s in step.iter_mut() {
                    *s /= 2.0;
                }
                if step.iter().all(|s| *s < 0.004) {
                    break;
                }
            }
        }
        best
    }

    pub fn run(dir: PathBuf, mode: &str, profile: Option<Profile>) {
        let (mut renderer, cases) = load(&dir);
        let base = profile.unwrap_or(Profile::CANVAS2D);
        renderer.text().set_profile(base.clone());
        match mode {
            "score" => {
                let mut groups: std::collections::BTreeMap<String, Vec<f64>> = Default::default();
                for c in &cases {
                    let (ink, ratio) = score(&mut renderer, c);
                    println!(
                        "{:<18} {:>6.1}px {:<8} ink {ink:.4} ratio {ratio:.3}",
                        c.id, c.size, c.color
                    );
                    groups.entry(c.group.clone()).or_default().push(ink);
                }
                for (g, v) in groups {
                    let min = v.iter().cloned().fold(1.0, f64::min);
                    println!(
                        "{g:<12} n {:>4} min {min:.4} mean {:.4}",
                        v.len(),
                        v.iter().sum::<f64>() / v.len() as f64
                    );
                }
            }
            "sizes" => {
                let mut sizes: Vec<f64> = cases
                    .iter()
                    .filter(|c| c.group == "fit")
                    .map(|c| c.size)
                    .collect();
                sizes.sort_by(|a, b| a.partial_cmp(b).unwrap());
                sizes.dedup();
                let mut table = Vec::new();
                for size in sizes {
                    let set: Vec<&Case> = cases
                        .iter()
                        .filter(|c| c.group == "fit" && c.size == size)
                        .collect();
                    let start = base.at(size);
                    renderer.text().set_profile(base.clone());
                    let before = mean(&mut renderer, &set);
                    let _ = start;
                    let mut x = vec![0.05, 0.05, -0.05, 1.0];
                    let mut step = vec![0.04, 0.04, 0.04, 0.1];
                    let best = search(
                        &mut x,
                        &mut step,
                        &[0.0, 0.0, -1.0, 0.3],
                        &[1.0, 1.0, 1.0, 3.0],
                        &mut |v| {
                            let p = Point {
                                size,
                                expand_x: v[0],
                                expand_y: v[1],
                                dy: v[2],
                                gamma: v[3],
                            };
                            renderer.text().set_profile(constant(&base, p));
                            mean(&mut renderer, &set)
                        },
                    );
                    println!(
                        "size {size:>5}: before {before:.4} after {best:.4}  ex {:.3} ey {:.3} dy {:.3} gamma {:.3}",
                        x[0], x[1], x[2], x[3]
                    );
                    table.push(Point {
                        size,
                        expand_x: x[0],
                        expand_y: x[1],
                        dy: x[2],
                        gamma: x[3],
                    });
                }
                let fitted = Profile {
                    points: Cow::Owned(table),
                    ..base.clone()
                };
                std::fs::write(
                    dir.join("fit-profile.json"),
                    serde_json::to_string_pretty(&fitted).unwrap(),
                )
                .unwrap();
                println!("wrote {}", dir.join("fit-profile.json").display());
            }
            "dark" => {
                let set: Vec<&Case> = cases.iter().filter(|c| c.group == "fit-color").collect();
                let mut x = vec![base.mix];
                let mut step = vec![0.2];
                let best = search(&mut x, &mut step, &[0.0], &[6.0], &mut |v| {
                    renderer.text().set_profile(Profile {
                        mix: v[0],
                        ..base.clone()
                    });
                    mean(&mut renderer, &set)
                });
                println!("mix {:.3}: mean {best:.4}", x[0]);
                renderer.text().set_profile(Profile {
                    mix: x[0],
                    ..base.clone()
                });
                for c in &set {
                    let (ink, ratio) = score(&mut renderer, c);
                    println!(
                        "{:<18} {:>5}px {} ink {ink:.4} ratio {ratio:.3}",
                        c.id, c.size, c.color
                    );
                }
            }
            "probe" => {
                // Edge profiles of Menlo's full block: reference against
                // plain coverage at Skia's positions.
                let plain = Profile {
                    points: Cow::Owned(vec![Point {
                        size: 0.0,
                        expand_x: 0.0,
                        expand_y: 0.0,
                        dy: 0.0,
                        gamma: 1.0,
                    }]),
                    snap: pm_text::render::profile::Snap::QuarterRound,
                    path_above: None,
                    large_above: None,
                    large: Point {
                        size: 0.0,
                        expand_x: 0.0,
                        expand_y: 0.0,
                        dy: 0.0,
                        gamma: 1.0,
                    },
                    light: Cow::Borrowed(&pm_text::render::profile::LINEAR),
                    dark: Cow::Borrowed(&pm_text::render::profile::LINEAR),
                    mix: 1.0,
                    fake_bold: 0.0,
                    fake_bold_y: 1.0,
                    stroke: base.stroke,
                    ..base.clone()
                };
                renderer.text().set_profile(plain);
                for c in &cases {
                    let o = ours(&mut renderer, c);
                    let w = c.width;
                    // The first block: rows where our plain alpha is full in its middle column.
                    let alpha = |img: &[u8], x: usize, y: usize| {
                        f64::from(img[(y * w + x) * 4 + 3]) / 255.0
                    };
                    let row: Vec<usize> = (0..c.height)
                        .filter(|&y| (0..w).any(|x| alpha(&o, x, y) > 0.99))
                        .collect();
                    if row.is_empty() {
                        continue;
                    }
                    let ym = row[row.len() / 2];
                    let cols: Vec<usize> = (0..w)
                        .filter(|&x| (0..c.height).any(|y| alpha(&o, x, y) > 0.99))
                        .collect();
                    let xm = cols[0] + 2;
                    let xs: Vec<String> = (cols[0].saturating_sub(3)..cols[0] + 2)
                        .map(|x| {
                            format!("{:.2}/{:.2}", alpha(&o, x, ym), alpha(&c.reference, x, ym))
                        })
                        .collect();
                    let top = row[0];
                    let ys: Vec<String> = (top.saturating_sub(3)..top + 2)
                        .map(|y| {
                            format!("{:.2}/{:.2}", alpha(&o, xm, y), alpha(&c.reference, xm, y))
                        })
                        .collect();
                    let bot = *row.last().unwrap();
                    let yb: Vec<String> = (bot - 1..(bot + 4).min(c.height))
                        .map(|y| {
                            format!("{:.2}/{:.2}", alpha(&o, xm, y), alpha(&c.reference, xm, y))
                        })
                        .collect();
                    println!(
                        "{:<16} {:>6.2}px {}  L[{}]  T[{}]  B[{}]",
                        c.id,
                        c.size,
                        c.color,
                        xs.join(" "),
                        ys.join(" "),
                        yb.join(" ")
                    );
                    // Edge positions: rows above `bot` summed as coverage (plain: linear; reference: inverse white curve).
                    let inv = |a: f64| -> f64 {
                        let curve = [
                            0.0, 0.10, 0.20, 0.29, 0.39, 0.48, 0.56, 0.64, 0.71, 0.78, 0.84, 0.88,
                            0.93, 0.96, 0.98, 1.0, 1.0f64,
                        ];
                        if a >= 1.0 {
                            return 0.9375;
                        }
                        for i in 0..16 {
                            if a <= curve[i + 1] {
                                let t = (a - curve[i]) / (curve[i + 1] - curve[i]).max(1e-9);
                                return (i as f64 + t) / 16.0;
                            }
                        }
                        1.0
                    };
                    let top_of = |img: &[u8], lin: bool| -> f64 {
                        let mut sum = 0.0;
                        for y in 0..=bot {
                            let a = alpha(img, xm, y);
                            sum += if lin {
                                a
                            } else if a >= 0.999 {
                                1.0
                            } else {
                                inv(a)
                            };
                        }
                        bot as f64 + 1.0 - sum
                    };
                    let bot_of = |img: &[u8], lin: bool| -> f64 {
                        let mut sum = 0.0;
                        for y in top..c.height {
                            let a = alpha(img, xm, y);
                            sum += if lin {
                                a
                            } else if a >= 0.999 {
                                1.0
                            } else {
                                inv(a)
                            };
                        }
                        top as f64 + sum
                    };
                    println!(
                        "   edges: plain top {:.3} bottom {:.3} | ref top {:.3} bottom {:.3} | dtop {:+.3} dbot {:+.3}",
                        top_of(&o, true),
                        bot_of(&o, true),
                        top_of(&c.reference, false),
                        bot_of(&c.reference, false),
                        top_of(&c.reference, false) - top_of(&o, true),
                        bot_of(&c.reference, false) - bot_of(&o, true)
                    );
                }
            }
            "large" => {
                for c in cases.iter().filter(|c| c.group.starts_with("probe-large")) {
                    let mut mask = base.clone();
                    mask.path_above = None;
                    renderer.text().set_profile(mask);
                    let (m, mr) = score(&mut renderer, c);
                    let mut path = base.clone();
                    path.path_above = Some(0.0);
                    renderer.text().set_profile(path);
                    let (p, pr) = score(&mut renderer, c);
                    println!(
                        "{:<22} {:>5}px mask {m:.4} ({mr:.3})  path {p:.4} ({pr:.3})",
                        c.id, c.size
                    );
                }
            }
            "mid" => {
                // Glyphs between 160 and 256 px: fit a plain-curve regime.
                let set: Vec<&Case> = cases
                    .iter()
                    .filter(|c| c.group == "probe-large-h" && c.size > 162.0 && c.size <= 250.0)
                    .collect();
                let mut x = vec![0.0, 0.0, 0.0, 1.0];
                let mut step = vec![0.1, 0.1, 0.1, 0.2];
                let best = search(
                    &mut x,
                    &mut step,
                    &[-0.5, -0.5, -1.0, 0.3],
                    &[1.0, 1.0, 1.0, 3.0],
                    &mut |v| {
                        let mut prof = base.clone();
                        prof.large_above = Some(162.0);
                        prof.large = Point {
                            size: 200.0,
                            expand_x: v[0],
                            expand_y: v[1],
                            dy: v[2],
                            gamma: v[3],
                        };
                        renderer.text().set_profile(prof);
                        mean(&mut renderer, &set)
                    },
                );
                println!(
                    "mid: ex {:.3} ey {:.3} dy {:.3} gamma {:.3} mean {best:.4}",
                    x[0], x[1], x[2], x[3]
                );
            }
            "bold" => {
                let set: Vec<&Case> = cases.iter().filter(|c| c.group == "synthetic").collect();
                let mut x = vec![base.fake_bold, base.fake_bold_y];
                let mut step = vec![0.002, 0.2];
                let best = search(&mut x, &mut step, &[0.0, 0.0], &[0.05, 2.0], &mut |v| {
                    renderer.text().set_profile(Profile {
                        fake_bold: v[0],
                        fake_bold_y: v[1],
                        ..base.clone()
                    });
                    mean(&mut renderer, &set)
                });
                println!("fake_bold {:.5} y {:.3}: mean {best:.4}", x[0], x[1]);
                for c in set
                    .iter()
                    .filter(|c| c.id.ends_with('4') || c.id.ends_with('6') || c.id.ends_with('8'))
                {
                    let mut y = vec![0.006, 1.0];
                    let mut st = vec![0.002, 0.2];
                    let b = search(&mut y, &mut st, &[0.0, 0.0], &[0.05, 2.0], &mut |v| {
                        renderer.text().set_profile(Profile {
                            fake_bold: v[0],
                            fake_bold_y: v[1],
                            ..base.clone()
                        });
                        score(&mut renderer, c).0
                    });
                    println!(
                        "  {} {}px best {:.5} × size, y {:.3}: ink {b:.4}",
                        c.id, c.size, y[0], y[1]
                    );
                }
                renderer.text().set_profile(Profile {
                    fake_bold: x[0],
                    ..base.clone()
                });
                for c in &set {
                    let (ink, ratio) = score(&mut renderer, c);
                    println!("{:<18} {:>5}px ink {ink:.4} ratio {ratio:.3}", c.id, c.size);
                }
            }
            "stroke" => {
                let set: Vec<&Case> = cases.iter().filter(|c| c.group == "fit-stroke").collect();
                let mut x = vec![base.stroke.widen, base.stroke.gamma];
                let mut step = vec![0.05, 0.1];
                let best = search(&mut x, &mut step, &[-0.5, 0.3], &[0.5, 3.0], &mut |v| {
                    let mut p = base.clone();
                    p.stroke.widen = v[0];
                    p.stroke.gamma = v[1];
                    renderer.text().set_profile(p);
                    mean(&mut renderer, &set)
                });
                println!("stroke widen {:.3} gamma {:.3}: mean {best:.4}", x[0], x[1]);
            }
            "emoji" => {
                let mut reg = pm_text::registry::CoreTextRegistry::new();
                use pm_text::font::FontResolver;
                let desc = pm_text::font::FontDesc {
                    weight: 500.0,
                    italic: false,
                    size: 16.0,
                };
                let primary = reg.family("Helvetica", &desc).unwrap();
                let face = reg
                    .fallback_for_char(&desc, 0x1F600, primary, true)
                    .unwrap();
                for s in 6..=40 {
                    println!("em_advance({s}) = {:?}", reg.em_advance(face, s as f32));
                }
                for c in cases.iter().filter(|c| c.group == "fit-emoji") {
                    let o = ours(&mut renderer, c);
                    let bbox = |img: &[u8]| {
                        let (mut x0, mut y0, mut x1, mut y1, mut sx, mut sy, mut sa) =
                            (usize::MAX, usize::MAX, 0, 0, 0.0, 0.0, 0.0);
                        for y in 0..c.height {
                            for x in 0..c.width {
                                let a = f64::from(img[(y * c.width + x) * 4 + 3]);
                                if a > 0.0 {
                                    x0 = x0.min(x);
                                    y0 = y0.min(y);
                                    x1 = x1.max(x + 1);
                                    y1 = y1.max(y + 1);
                                    sx += a * x as f64;
                                    sy += a * y as f64;
                                    sa += a;
                                }
                            }
                        }
                        (x0, y0, x1, y1, sx / sa, sy / sa, sa / 255.0)
                    };
                    let (ink, _, _) =
                        pm_render::parity::text_ssim(&c.reference, &o, c.width, c.height);
                    println!(
                        "{:<16} {:>3}px legacy {:?}\n                       ours   {:?}  ink {ink:.3}",
                        c.id,
                        c.size,
                        bbox(&c.reference),
                        bbox(&o)
                    );
                }
            }
            other => panic!("unknown mode {other}"),
        }
    }
}

#[cfg(target_os = "macos")]
fn main() {
    let mut args = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(args.next().expect("capture dir"));
    let mode = args.next().unwrap_or_else(|| "score".into());
    let profile = args.next().map(|p| {
        serde_json::from_str::<pm_text::render::profile::Profile>(
            &std::fs::read_to_string(p).unwrap(),
        )
        .unwrap()
    });
    fit::run(dir, &mode, profile);
}

#[cfg(not(target_os = "macos"))]
fn main() {}
