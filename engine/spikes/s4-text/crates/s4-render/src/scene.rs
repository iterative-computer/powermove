//! The S4 render scenes. One definition feeds every renderer: the Rust
//! renderers read it directly and the Canvas2D page reads `out/scenes.json`.
//!
//! Text items are text layer contents (`layer.d`) drawn the way
//! raster.ts `rasterText` draws them at density 1; path items are SVG path
//! data drawn as fills or strokes. Everything paints on a transparent
//! canvas, so white content's alpha is its coverage.

use kurbo::{BezPath, Circle, Ellipse, Point, Rect, RoundedRect, Shape, Vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::f64::consts::PI;

pub type Color = [f32; 4];
pub const WHITE: Color = [1.0, 1.0, 1.0, 1.0];
pub const BLACK: Color = [0.0, 0.0, 0.0, 1.0];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Item {
    /// `d` is a text layer content; (x, y) is the alignment origin and the
    /// top of the first line (baseline at y + size × 0.82).
    #[serde(rename_all = "camelCase")]
    Text { d: Value, x: f64, y: f64, color: Color, stroke: f64, stroke_color: Color },
    /// SVG path data at offset (x, y); `stroke` > 0 strokes with that width.
    #[serde(rename_all = "camelCase")]
    Path { d: String, x: f64, y: f64, color: Color, rule: String, stroke: f64, join: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scene {
    pub id: String,
    /// `text`, `path`, or `stress`.
    pub group: String,
    pub width: u32,
    pub height: u32,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneFile {
    pub scenes: Vec<Scene>,
    pub stress: Vec<Scene>,
    pub stress_frames: u32,
}

const PANGRAM: &str = "The quick brown fox jumps over the lazy dog. 0123456789";
const KERN: &str = "AVATAR Tower WAVE Yo To fi fl ffi";

fn text(d: Value, x: f64, y: f64) -> Item {
    Item::Text { d, x, y, color: WHITE, stroke: 0.0, stroke_color: BLACK }
}

fn content(font: &str, weight: f64, size: f64, s: &str) -> Value {
    json!({ "text": s, "font": font, "weight": weight, "size": size, "tracking": 0, "leading": 1.15, "align": "left", "italic": false, "boxWidth": 0, "boxHeight": 0, "paragraph": false })
}

fn with(mut d: Value, key: &str, v: Value) -> Value {
    d[key] = v;
    d
}

fn path(p: &BezPath, x: f64, y: f64) -> Item {
    Item::Path { d: p.to_svg(), x, y, color: WHITE, rule: "nonzero".into(), stroke: 0.0, join: "round".into() }
}

fn stroked(p: &BezPath, x: f64, y: f64, width: f64) -> Item {
    Item::Path { d: p.to_svg(), x, y, color: WHITE, rule: "nonzero".into(), stroke: width, join: "round".into() }
}

/// Stacks text lines vertically from y = 8 and returns the scene height.
fn stack(items: &mut Vec<Item>, rows: Vec<Value>, x: f64) -> u32 {
    let mut y = 8.0;
    for d in rows {
        let size = d["size"].as_f64().unwrap();
        let lead = d["leading"].as_f64().unwrap_or(1.15);
        items.push(text(d, x, y));
        y += (size * lead).ceil() + 6.0;
    }
    (y + 8.0).ceil() as u32
}

fn text_scene(id: &str, width: u32, rows: Vec<Value>) -> Scene {
    let mut items = Vec::new();
    let height = stack(&mut items, rows, 12.25);
    Scene { id: id.into(), group: "text".into(), width, height, items }
}

/// raster.ts polygon and star: n points, outer radius R, inner R × 0.46.
fn star(n: usize, r: f64, inner: f64, c: Point) -> BezPath {
    let mut p = BezPath::new();
    let count = n * if inner < 1.0 { 2 } else { 1 };
    for i in 0..count {
        let rr = if inner < 1.0 && i % 2 == 1 { r * inner } else { r };
        let a = -PI / 2.0 + i as f64 * 2.0 * PI / count as f64;
        let pt = c + Vec2::new(a.cos() * rr, a.sin() * rr);
        if i == 0 { p.move_to(pt) } else { p.line_to(pt) }
    }
    p.close_path();
    p
}

fn siemens(c: Point, r: f64, spokes: usize) -> BezPath {
    let mut p = BezPath::new();
    for i in 0..spokes {
        let a0 = i as f64 * 2.0 * PI / spokes as f64;
        let a1 = a0 + PI / spokes as f64;
        p.move_to(c);
        p.line_to(c + Vec2::new(a0.cos() * r, a0.sin() * r));
        p.line_to(c + Vec2::new(a1.cos() * r, a1.sin() * r));
        p.close_path();
    }
    p
}

/// A closed blob of `n` cubic segments around `c` (deterministic wobble).
fn blob(c: Point, r: f64, n: usize, wobble: f64) -> BezPath {
    let pt = |t: f64| {
        let rr = r * (1.0 + wobble * (7.0 * t).sin() * (3.0 * t + 1.0).cos() + 0.5 * wobble * (23.0 * t).sin());
        c + Vec2::new(t.cos() * rr, t.sin() * rr)
    };
    let mut p = BezPath::new();
    let step = 2.0 * PI / n as f64;
    p.move_to(pt(0.0));
    for i in 0..n {
        let t0 = i as f64 * step;
        let (a, b, d) = (pt(t0 + step / 3.0), pt(t0 + 2.0 * step / 3.0), pt(t0 + step));
        p.curve_to(a, b, d);
    }
    p.close_path();
    p
}

/// An Archimedean spiral polyline of `n` segments.
fn spiral(c: Point, r: f64, turns: f64, n: usize) -> BezPath {
    let mut p = BezPath::new();
    for i in 0..=n {
        let t = i as f64 / n as f64;
        let a = t * turns * 2.0 * PI;
        let pt = c + Vec2::new(a.cos() * r * t, a.sin() * r * t);
        if i == 0 { p.move_to(pt) } else { p.line_to(pt) }
    }
    p
}

fn shapes_path() -> Vec<BezPath> {
    vec![
        Rect::new(0.0, 0.0, 120.0, 80.0).to_path(1e-3),
        RoundedRect::new(0.0, 0.0, 140.0, 90.0, 24.0).to_path(1e-3),
        Ellipse::new((60.0, 45.0), (60.0, 45.0), 0.0).to_path(1e-3),
        star(5, 50.0, 0.46, Point::new(50.0, 50.0)),
        star(6, 50.0, 1.0, Point::new(50.0, 50.0)),
        star(12, 50.0, 0.46, Point::new(50.0, 50.0)),
    ]
}

pub fn scenes() -> SceneFile {
    let mut scenes = Vec::new();
    let sf = "SF Pro Display";
    scenes.push(text_scene("sf-small", 760, [9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 16.0, 18.0].iter().map(|&s| content(sf, 400.0, s, PANGRAM)).collect()));
    scenes.push(text_scene("sf-small-semibold", 760, [9.0, 11.0, 13.0, 15.0, 17.0].iter().map(|&s| content(sf, 600.0, s, PANGRAM)).collect()));
    scenes.push(text_scene("sf-medium", 1100, [20.0, 24.0, 32.0, 40.0].iter().flat_map(|&s| [content(sf, 400.0, s, KERN), content(sf, 700.0, s, KERN)]).collect()));
    scenes.push(text_scene("sf-display", 1400, [64.0, 96.0, 144.0].iter().map(|&s| content(sf, 600.0, s, "Powermove Ag")).collect()));
    scenes.push(text_scene("sf-huge", 1200, vec![content(sf, 700.0, 400.0, "Ag&")]));
    let fams = ["Helvetica Neue", "Georgia", "Times New Roman", "Menlo", "Avenir Next", "Futura", "Didot", "Baskerville", "Gill Sans", "American Typewriter"];
    scenes.push(text_scene("families-small", 760, fams.iter().map(|f| content(f, 400.0, 14.0, PANGRAM)).collect()));
    scenes.push(text_scene("families-large", 1500, fams.iter().map(|f| content(f, 400.0, 48.0, "Hamburgefonstiv 123")).collect()));
    scenes.push(text_scene("weights", 900, (1..=9).map(|w| content(sf, w as f64 * 100.0, 28.0, "Variable weight Hamburgefonstiv")).collect()));
    scenes.push(text_scene("new-york", 1100, [12.0, 24.0, 48.0].iter().map(|&s| content("New York", 400.0, s, "Optical sizes in New York")).collect()));
    scenes.push(text_scene("italic", 900, vec![with(content("Helvetica Neue", 400.0, 24.0, PANGRAM), "italic", json!(true)), with(content("Georgia", 400.0, 24.0, PANGRAM), "italic", json!(true))]));
    scenes.push(text_scene("tracking", 1400, [-1.0, 3.0, 12.0].iter().map(|&t| with(content(sf, 500.0, 24.0, "Tracking changes letter spacing"), "tracking", json!(t))).collect()));
    scenes.push(text_scene("cjk", 900, vec![content("PingFang SC", 400.0, 24.0, "视频编辑软件让创作更简单。"), content("PingFang SC", 600.0, 48.0, "视频编辑软件"), content("Hiragino Sans", 400.0, 24.0, "動画編集はとても楽しいです。カタカナ"), content("Apple SD Gothic Neo", 400.0, 24.0, "한국어 텍스트 레이아웃 테스트")]));
    scenes.push(text_scene("rtl", 900, vec![content("Geeza Pro", 400.0, 36.0, "مرحبا بالعالم في باورموف"), content("Arial Hebrew", 400.0, 32.0, "שלום עולם")]));
    scenes.push(text_scene("indic-thai", 900, vec![content("Kohinoor Devanagari", 400.0, 32.0, "नमस्ते दुनिया क्षत्रिय"), content("Thonburi", 400.0, 32.0, "สวัสดีชาวโลก ภาษาไทย")]));
    scenes.push(text_scene("emoji", 900, vec![content(sf, 400.0, 48.0, "Hi 👋🏽 team 🎬🍿❤️"), content(sf, 400.0, 20.0, "Small 🎥 emoji 👨‍👩‍👧‍👦 🇰🇷")]));
    {
        let mut items = Vec::new();
        let mut d = content(sf, 700.0, 64.0, "Caption stroke Ag");
        d["leading"] = json!(1.2);
        items.push(Item::Text { d, x: 24.5, y: 16.0, color: WHITE, stroke: 6.0, stroke_color: BLACK });
        let d2 = content("Helvetica Neue", 700.0, 32.0, "Smaller caption with a 3 px stroke");
        items.push(Item::Text { d: d2, x: 24.0, y: 120.0, color: [1.0, 0.85, 0.2, 1.0], stroke: 3.0, stroke_color: BLACK });
        scenes.push(Scene { id: "caption-stroke".into(), group: "text".into(), width: 900, height: 180, items });
    }

    // Paths.
    let mut items = Vec::new();
    for (i, p) in shapes_path().iter().enumerate() {
        items.push(path(p, 16.3 + i as f64 * 160.0, 16.6));
    }
    scenes.push(Scene { id: "shapes-fill".into(), group: "path".into(), width: 980, height: 140, items });
    let mut items = Vec::new();
    for (row, w) in [1.0, 3.0, 12.0].iter().enumerate() {
        for (i, p) in shapes_path().iter().enumerate() {
            items.push(stroked(p, 16.3 + i as f64 * 160.0, 16.6 + row as f64 * 130.0, *w));
        }
    }
    for (i, w) in [0.5, 1.0, 1.5].iter().enumerate() {
        let mut l = BezPath::new();
        l.move_to((0.0, 0.0));
        l.line_to((900.0, 37.0 + i as f64 * 11.0));
        items.push(stroked(&l, 16.0, 420.0 + i as f64 * 20.0, *w));
    }
    scenes.push(Scene { id: "shapes-stroke".into(), group: "path".into(), width: 980, height: 520, items });
    scenes.push(Scene { id: "siemens".into(), group: "path".into(), width: 520, height: 520, items: vec![path(&siemens(Point::new(260.0, 260.0), 250.0, 72), 0.0, 0.0)] });
    {
        let mut items = vec![path(&blob(Point::new(300.0, 300.0), 220.0, 400, 0.18), 0.0, 0.0)];
        // A heptagram drawn as one self-intersecting contour, under both
        // fill rules.
        let mut eo = BezPath::new();
        for i in 0..7 {
            let a = -PI / 2.0 + (i * 3 % 7) as f64 * 2.0 * PI / 7.0;
            let pt = Point::new(a.cos() * 200.0, a.sin() * 200.0);
            if i == 0 { eo.move_to(pt) } else { eo.line_to(pt) }
        }
        eo.close_path();
        items.push(Item::Path { d: eo.to_svg(), x: 860.0, y: 300.0, color: WHITE, rule: "evenodd".into(), stroke: 0.0, join: "round".into() });
        items.push(Item::Path { d: eo.to_svg(), x: 1300.0, y: 300.0, color: WHITE, rule: "nonzero".into(), stroke: 0.0, join: "round".into() });
        scenes.push(Scene { id: "large-paths".into(), group: "path".into(), width: 1540, height: 600, items });
    }
    {
        let mut items = Vec::new();
        for i in 0..24 {
            let r = 0.5 + i as f64 * 0.25;
            items.push(path(&Circle::new((0.0, 0.0), r).to_path(1e-3), 10.0 + i as f64 * 14.0 + i as f64 * 0.1, 12.0));
        }
        scenes.push(Scene { id: "tiny".into(), group: "path".into(), width: 360, height: 24, items });
    }

    scenes.extend(fit_scenes());
    SceneFile { scenes, stress: stress(), stress_frames: 30 }
}

/// Stem-darkening fit: one scene per size, four families.
pub const FIT_SIZES: &[f64] = &[9.0, 11.0, 13.0, 16.0, 20.0, 24.0, 32.0, 48.0, 64.0, 96.0, 144.0];

fn fit_scenes() -> Vec<Scene> {
    FIT_SIZES
        .iter()
        .map(|&size| {
            let s = if size >= 48.0 { "Hamburgefonstiv" } else { "Hamburgefonstiv quick brown fox 0123" };
            let rows = ["SF Pro Display", "Helvetica Neue", "Georgia", "PingFang SC"]
                .iter()
                .map(|f| content(f, 400.0, size, if *f == "PingFang SC" { "视频编辑软件让创作更简单" } else { s }))
                .collect();
            let mut sc = text_scene(&format!("fit-{size}"), (size * 16.0 + 40.0).ceil().min(2400.0) as u32, rows);
            sc.group = "fit".into();
            sc
        })
        .collect()
}

const SUB: &str = "Captions keep every spoken word readable at any size";

/// 4K stress scenes: caption-heavy frames and large paths.
pub fn stress() -> Vec<Scene> {
    let mut out = Vec::new();
    // 24 caption blocks of two lines, 72 px bold with a 6 px stroke over a
    // translucent rounded box, as captions/render.ts draws them.
    let mut items = Vec::new();
    for b in 0..24 {
        let (col, row) = (b % 2, b / 2);
        let x = 60.0 + col as f64 * 1890.0;
        let y = 20.0 + row as f64 * 176.0;
        let box_path = RoundedRect::new(0.0, 0.0, 1800.0, 168.0, 18.0).to_path(1e-3);
        items.push(Item::Path { d: box_path.to_svg(), x, y, color: [0.0, 0.0, 0.0, 0.5], rule: "nonzero".into(), stroke: 0.0, join: "round".into() });
        let font = ["SF Pro Display", "Helvetica Neue", "Avenir Next"][b % 3];
        let mut d = content(font, 700.0, 72.0, &format!("{SUB} {b}\nsecond line of block {b} stays readable"));
        d["leading"] = json!(1.1);
        items.push(Item::Text { d, x: x + 24.0, y: y + 4.0, color: WHITE, stroke: 6.0, stroke_color: BLACK });
    }
    out.push(Scene { id: "stress-captions-4k".into(), group: "stress".into(), width: 3840, height: 2160, items });

    // Dense 36 px subtitles: 56 lines of ~110 characters, Latin and CJK.
    let mut items = Vec::new();
    for l in 0..56 {
        let s = if l % 4 == 3 { "视频编辑软件让创作更简单。动画编集はとても楽しいです。한국어 텍스트 레이아웃 테스트입니다 视频编辑软件让创作更简单。".to_string() } else { format!("{l:02} {PANGRAM} {SUB}.") };
        items.push(text(content("SF Pro Text", 400.0, 36.0, &s), 24.0, 12.0 + l as f64 * 38.0));
    }
    out.push(Scene { id: "stress-subtitles-4k".into(), group: "stress".into(), width: 3840, height: 2160, items });

    // Large paths: a 100,000-segment stroked spiral, a 4,000-cubic blob, and a
    // 360-spoke Siemens star, all frame-sized.
    let items = vec![
        Item::Path { d: blob(Point::new(1920.0, 1080.0), 900.0, 4000, 0.2).to_svg(), x: 0.0, y: 0.0, color: [0.2, 0.5, 1.0, 1.0], rule: "nonzero".into(), stroke: 0.0, join: "round".into() },
        path(&siemens(Point::new(1920.0, 1080.0), 1000.0, 360), 0.0, 0.0),
        Item::Path { d: spiral(Point::new(1920.0, 1080.0), 1060.0, 120.0, 100_000).to_svg(), x: 0.0, y: 0.0, color: [1.0, 0.3, 0.2, 1.0], rule: "nonzero".into(), stroke: 2.0, join: "round".into() },
    ];
    out.push(Scene { id: "stress-paths-4k".into(), group: "stress".into(), width: 3840, height: 2160, items });
    out
}
