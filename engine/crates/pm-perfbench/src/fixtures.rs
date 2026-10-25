//! Public, synthetic bench fixtures, generated deterministically in code.
//!
//! [`synthetic`] is the Rust twin of the app's synthetic project
//! (`apps/desktop/src/main/engine/integration/synthetic-project.ts`, the
//! Phase 2 performance rows): solids and images with static and keyframed
//! transforms and opacity, groups of 20, precomps of 10 layers used twice,
//! and parenting through nulls, with the same seeded generator (mulberry32)
//! and the same proportions. The images are generated PNGs in a temporary
//! directory that is removed when the [`Fixture`] drops.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

pub fn ch(v: Value) -> Value {
    json!({ "v": v, "kf": [], "expr": null })
}

/// Keyframes `(t, v)` with the curve `ease` (`eo`, `ei` handles).
pub fn keys(v: Value, keys: &[(f64, Value)], ease: ([f64; 2], [f64; 2])) -> Value {
    let kf: Vec<Value> = keys
        .iter()
        .enumerate()
        .map(|(i, (t, v))| {
            json!({ "t": t, "v": v, "i": format!("k{i}"), "eo": ease.0, "ei": ease.1, "hold": false })
        })
        .collect();
    json!({ "v": v, "kf": kf, "expr": null })
}

/// The transform channels of a layer (`mkLayer` defaults).
pub fn transform(x: f64, y: f64) -> Value {
    json!({
        "perspective": ch(json!(50)), "position.z": ch(json!(0)), "anchor.z": ch(json!(0)),
        "scale.z": ch(json!(100)), "rotation.x": ch(json!(0)), "rotation.y": ch(json!(0)),
        "orientation.x": ch(json!(0)), "orientation.y": ch(json!(0)), "orientation.z": ch(json!(0)),
        "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(x)),
        "position.y": ch(json!(y)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
        "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
    })
}

/// A layer of `ty` with content `d`, at `(x, y)`.
pub fn layer(id: &str, ty: &str, d: Value, x: f64, y: f64, dur: f64) -> Value {
    json!({
        "id": id, "type": ty, "name": id, "from": 0, "dur": dur, "on": true, "lock": false,
        "shy": false, "collapsed": true, "threeD": false, "solo": false, "color": "#4C8DFF",
        "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
        "p": transform(x, y), "fx": [], "transitionIn": null, "transitionOut": null,
        "masks": [], "d": d, "locked_intent": {}
    })
}

pub fn set(layer: &mut Value, path: &str, v: Value) {
    layer["p"][path] = if v.is_object() { v } else { ch(v) };
}

pub fn solid_fill(color: &str) -> Value {
    json!({ "type": "solid", "angle": 0, "stops": [{ "id": "s1", "color": color, "position": 0 }] })
}

/// A project whose open composition is `w` × `h` at `fps`.
pub fn project(w: u32, h: u32, fps: f64, dur: f64, background: Value, layers: Vec<Value>) -> Value {
    json!({
        "id": "Pbench", "name": "bench", "w": w, "h": h, "fps": fps, "dur": dur, "bg": "#141821",
        "backgroundFill": background, "compId": "Pbench-root", "compName": "bench",
        "markers": [], "work": [0, dur], "params": {}, "revision": 0, "edits": [],
        "created": 0, "shutter": 0.5, "layers": layers, "assets": {}, "comps": {}
    })
}

/// The open composition's id in every fixture.
pub const ROOT: &str = "Pbench-root";

pub fn load(doc: &Value) -> pm_model::Document {
    let value = pm_model::json::parse(&doc.to_string()).expect("a fixture parses");
    pm_model::Document::new(Some(value), 1)
}

/// A temporary directory removed on drop (success or panic).
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> TempDir {
        let d = std::env::temp_dir().join(format!("pm-perfbench-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("a temporary directory");
        TempDir(d)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A generated document and the files its assets resolve to.
pub struct Fixture {
    pub doc: Value,
    pub media: std::collections::BTreeMap<String, PathBuf>,
    pub summary: Value,
    /// Holds the generated files.
    pub dir: Option<TempDir>,
}

/// The app's synthetic generator (mulberry32).
struct Random(u32);

impl Random {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b79f5);
        let s = self.0;
        let mut t = (s ^ (s >> 15)).wrapping_mul(1 | s);
        t = (t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t))) ^ t;
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }

    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.next() * xs.len() as f64) as usize]
    }
}

const PALETTE: [&str; 10] = [
    "#E8553A", "#F2A541", "#3FA7D6", "#59CD90", "#EE6352", "#FAC05E", "#8C5383", "#2E86AB",
    "#F6F5AE", "#D1D1D1",
];

/// The four curves the app's generator picks from (linear, easeOut,
/// easeInOut, power), as keyframe handles.
const EASES: [([f64; 2], [f64; 2]); 4] = [
    ([0.0, 0.0], [1.0, 1.0]),
    ([0.0, 0.0], [0.58, 1.0]),
    ([0.42, 0.0], [0.58, 1.0]),
    ([0.7, 0.0], [0.84, 0.0]),
];

/// A 256 × 256 gradient PNG with a white disc (the app generator's images).
pub fn write_png(path: &Path, i: usize) {
    let hex = |s: &str| {
        let v = u32::from_str_radix(&s[1..], 16).unwrap();
        [(v >> 16) as f64, ((v >> 8) & 255) as f64, (v & 255) as f64]
    };
    let (a, b) = (hex(PALETTE[i % 10]), hex(PALETTE[(i + 3) % 10]));
    let r = 64.0 + i as f64 * 8.0;
    let mut rgba = Vec::with_capacity(256 * 256 * 4);
    for y in 0..256 {
        for x in 0..256 {
            let t = (x + y) as f64 / 510.0;
            let mut c = [0.0; 3];
            for k in 0..3 {
                c[k] = a[k] + (b[k] - a[k]) * t;
            }
            let (dx, dy) = (x as f64 + 0.5 - 128.0, y as f64 + 0.5 - 128.0);
            if dx * dx + dy * dy <= r * r {
                for v in &mut c {
                    *v = *v * 0.2 + 255.0 * 0.8;
                }
            }
            rgba.extend(c.iter().map(|v| v.round() as u8));
            rgba.push(255);
        }
    }
    let file = std::fs::File::create(path).expect("create a PNG");
    let mut e = png::Encoder::new(std::io::BufWriter::new(file), 256, 256);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()
        .and_then(|mut w| w.write_image_data(&rgba))
        .expect("write a PNG");
}

/// Options of [`synthetic`].
#[derive(Clone, Copy, Debug)]
pub struct Synthetic {
    pub layers: usize,
    pub images: usize,
    pub fps: f64,
    pub w: u32,
    pub h: u32,
    pub dur: f64,
    pub seed: u32,
    /// Leaf layers are shape layers (rect, ellipse, polygon, star; some
    /// stroked) instead of solids.
    pub shapes: bool,
}

impl Synthetic {
    pub fn new(layers: usize) -> Synthetic {
        Synthetic {
            layers,
            images: 2,
            fps: 30.0,
            w: 1920,
            h: 1080,
            dur: 10.0,
            seed: 1,
            shapes: false,
        }
    }
}

struct Gen {
    rng: Random,
    shapes: bool,
    fps: f64,
    dur: f64,
    by_type: std::collections::BTreeMap<String, usize>,
    keyframed: usize,
    next_id: usize,
    image_ids: Vec<String>,
}

impl Gen {
    fn fresh(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}{}", self.next_id)
    }

    fn count(&mut self, ty: &str) {
        *self.by_type.entry(ty.to_owned()).or_default() += 1;
    }

    fn animate(&mut self, layer: &mut Value, path: &str, values: &[f64]) {
        let n = values.len();
        let ease = *self.rng.pick(&EASES);
        let (fps, dur) = (self.fps, self.dur);
        let snap = |t: f64| (t * fps).round() / fps;
        let ks: Vec<(f64, Value)> = values
            .iter()
            .enumerate()
            .map(|(i, v)| (snap(i as f64 * dur / (n.max(2) - 1) as f64), json!(v)))
            .collect();
        let v = layer["p"][path]["v"].clone();
        layer["p"][path] = keys(v, &ks, ease);
        self.keyframed += 1;
    }

    /// A slice-feature leaf layer (solid or image) in a `cw` × `ch` composition.
    fn leaf(&mut self, cw: f64, ch: f64, index: usize) -> Value {
        let id = self.fresh("L");
        let use_image = !self.image_ids.is_empty() && self.rng.next() < 0.06;
        let size = 24 + (self.rng.next() * 120.0) as u32;
        let dur = self.dur;
        let mut l = if use_image {
            let asset = self.rng.pick(&self.image_ids).clone();
            let mut l = layer(
                &id,
                "image",
                json!({ "asset": asset, "fit": "contain", "w": size, "h": size }),
                0.0,
                0.0,
                dur,
            );
            l["name"] = json!(format!("Image {index}"));
            l
        } else if self.shapes {
            let color = *self.rng.pick(&PALETTE);
            let shape = *self.rng.pick(&["rect", "ellipse", "polygon", "star"]);
            let stroke = if self.rng.next() < 0.4 {
                2 + (self.rng.next() * 6.0) as u32
            } else {
                0
            };
            let mut l = layer(
                &id,
                "shape",
                json!({ "shape": shape, "w": size, "h": size, "color": color, "radius": size / 8,
                        "stroke": stroke, "strokeColor": "#F2F2F2", "points": 5 }),
                0.0,
                0.0,
                dur,
            );
            l["name"] = json!(format!("Shape {index}"));
            l
        } else {
            let color = *self.rng.pick(&PALETTE);
            let hh = 8.max((f64::from(size) * (0.4 + self.rng.next())) as u32);
            let mut l = layer(
                &id,
                "solid",
                json!({ "color": color, "w": size, "h": hh }),
                0.0,
                0.0,
                dur,
            );
            l["name"] = json!(format!("Solid {index}"));
            l
        };
        let ty = l["type"].as_str().unwrap().to_owned();
        self.count(&ty);
        let x = (self.rng.next() * cw).round();
        let y = (self.rng.next() * ch).round();
        set(&mut l, "position.x", json!(x));
        set(&mut l, "position.y", json!(y));
        set(
            &mut l,
            "opacity",
            json!(40.0 + (self.rng.next() * 60.0).round()),
        );
        set(
            &mut l,
            "rotation",
            json!((self.rng.next() * 360.0 - 180.0).round()),
        );
        let roll = self.rng.next();
        if roll < 0.25 {
            let to = (self.rng.next() * cw).round();
            self.animate(&mut l, "position.x", &[x, to, x]);
        } else if roll < 0.4 {
            let to = (self.rng.next() * 360.0).round();
            self.animate(&mut l, "rotation", &[0.0, to]);
        } else if roll < 0.5 {
            self.animate(&mut l, "opacity", &[100.0, 20.0, 100.0]);
        } else if roll < 0.58 {
            let sx = 60.0 + (self.rng.next() * 80.0).round();
            self.animate(&mut l, "scale.x", &[100.0, sx]);
            let sy = 60.0 + (self.rng.next() * 80.0).round();
            self.animate(&mut l, "scale.y", &[100.0, sy]);
        }
        l
    }
}

/// The synthetic project of `o.layers` layers (every composition counted).
pub fn synthetic(o: Synthetic) -> Fixture {
    let total = o.layers.max(1);
    let (w, h, fps, dur) = (f64::from(o.w), f64::from(o.h), o.fps, o.dur);
    let mut doc = project(o.w, o.h, fps, dur, solid_fill("#141821"), vec![]);

    let dir = (o.images > 0).then(|| TempDir::new("synthetic"));
    let mut media = std::collections::BTreeMap::new();
    let mut image_ids = Vec::new();
    for i in 0..o.images {
        let id = format!("image-{}", (b'a' + (i % 26) as u8) as char);
        let path = dir.as_ref().unwrap().path().join(format!("{id}.png"));
        write_png(&path, i);
        doc["assets"][&id] = json!({
            "id": id, "name": format!("{id}.png"), "kind": "image", "format": "png",
            "w": 256, "h": 256, "dur": 0, "fps": 30, "persisted": true
        });
        media.insert(id.clone(), path);
        image_ids.push(id);
    }
    let mut g = Gen {
        rng: Random(o.seed),
        shapes: o.shapes,
        fps,
        dur,
        by_type: Default::default(),
        keyframed: 0,
        next_id: 0,
        image_ids,
    };

    let mut made = 0usize;
    let mut layers: Vec<Value> = Vec::new();
    let mut backdrop = layer(
        "backdrop",
        "solid",
        json!({ "color": "#1E2430", "w": o.w, "h": o.h }),
        0.0,
        0.0,
        dur,
    );
    backdrop["name"] = json!("Backdrop");
    g.count("solid");
    made += 1;

    // Precomps: one composition of 10 layers per 100 layers, each used twice.
    let mut compositions = 0usize;
    let mut c = 0;
    while c < total / 100 && made + 12 <= total {
        let comp_id = format!("Pbench-comp{}", c + 1);
        let inner: Vec<Value> = (0..10).map(|i| g.leaf(480.0, 270.0, made + i)).collect();
        doc["comps"][&comp_id] = json!({
            "id": comp_id, "name": format!("Nested {}", c + 1), "w": 480, "h": 270, "fps": fps,
            "dur": dur, "bg": "#000000",
            "backgroundFill": { "type": "none", "stops": [], "angle": 0 },
            "layers": inner, "markers": []
        });
        compositions += 1;
        made += 10;
        for k in 0..2 {
            let id = g.fresh("P");
            let x = (240.0 + g.rng.next() * (w - 480.0)).round();
            let y = (135.0 + g.rng.next() * (h - 270.0)).round();
            let mut pre = layer(
                &id,
                "precomp",
                json!({ "comp": comp_id, "w": 480, "h": 270 }),
                x,
                y,
                dur,
            );
            set(
                &mut pre,
                "opacity",
                json!(70.0 + (g.rng.next() * 30.0).round()),
            );
            if k == 1 {
                g.animate(&mut pre, "rotation", &[0.0, 30.0]);
            }
            g.count("precomp");
            layers.push(pre);
            made += 1;
        }
        c += 1;
    }

    // Nulls that parent a few layers each.
    let mut nulls = Vec::new();
    let mut i = 0;
    while i < (total / 125).max(1) && made < total {
        let id = g.fresh("N");
        let (x, y) = ((g.rng.next() * w).round(), (g.rng.next() * h).round());
        let mut n = layer(&id, "null", json!({}), x, y, dur);
        let to = *g.rng.pick(&[-20.0, 20.0, 45.0]);
        g.animate(&mut n, "rotation", &[0.0, to]);
        g.count("null");
        nulls.push(id);
        layers.push(n);
        made += 1;
        i += 1;
    }

    // Groups of 20 leaves; the rest at the root.
    let mut groups = 0usize;
    while made < total {
        let room = total - made;
        if room >= 21 && g.rng.next() < 0.5 {
            let gid = g.fresh("G");
            let mut group = layer(&gid, "group", json!({}), 0.0, 0.0, dur);
            set(
                &mut group,
                "opacity",
                json!(80.0 + (g.rng.next() * 20.0).round()),
            );
            g.count("group");
            layers.push(group);
            made += 1;
            groups += 1;
            for _ in 0..20 {
                let mut child = g.leaf(w, h, made);
                child["group"] = json!(gid);
                layers.push(child);
                made += 1;
            }
        } else {
            let mut l = g.leaf(w, h, made);
            if !nulls.is_empty() && g.rng.next() < 0.08 {
                l["parent"] = json!(g.rng.pick(&nulls));
            }
            layers.push(l);
            made += 1;
        }
    }
    // The first layer is on top; the backdrop goes last.
    layers.push(backdrop);
    doc["layers"] = Value::Array(layers);
    Fixture {
        summary: json!({
            "layers": made, "byType": g.by_type, "groups": groups, "compositions": compositions,
            "keyframed": g.keyframed, "images": o.images
        }),
        doc,
        media,
        dir,
    }
}

/// One moving, rotating SVG image whose source stays byte-identical: a
/// gradient card, a clip path, a blurred shape and 60 small motifs.
pub fn animated_svg() -> Fixture {
    use std::fmt::Write;
    let mut art = String::from(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180" viewBox="0 0 320 180"><defs><linearGradient id="g"><stop stop-color="#3FCF8E"/><stop offset="1" stop-color="#4C8DFF"/></linearGradient><clipPath id="c"><circle cx="250" cy="90" r="60"/></clipPath><filter id="b"><feGaussianBlur stdDeviation="3"/></filter></defs><rect x="8" y="8" width="304" height="164" rx="24" fill="url(#g)"/><path d="M20 145 L86 32 L154 132 L218 26 L300 145 Z" fill="#F2A541" fill-opacity=".8"/><g clip-path="url(#c)"><rect x="190" width="130" height="180" fill="#FFD23F"/></g><circle cx="162" cy="88" r="39" fill="#EE6352" fill-opacity=".7" filter="url(#b)"/>"##,
    );
    for row in 0..6 {
        for col in 0..10 {
            let (x, y) = (20 + col * 30, 24 + row * 26);
            let color = ["#FCE6A7", "#F5A4AE", "#A9E9F0"][(row + col) % 3];
            write!(
                art,
                "<path d=\"M{x} {y} l7 -8 l7 8 l-7 8 Z\" fill=\"{color}\" opacity=\".72\"/><circle cx=\"{x}\" cy=\"{y}\" r=\"3\" fill=\"#243146\"/>"
            )
            .expect("append SVG motif");
        }
    }
    art.push_str("</svg>");
    let dir = TempDir::new("animated-svg");
    let path = dir.path().join("art.svg");
    std::fs::write(&path, art).expect("write SVG bench source");
    let mut image = layer(
        "animated-svg",
        "image",
        json!({ "asset": "art", "fit": "contain", "w": 180, "h": 101.25 }),
        320.0,
        180.0,
        10.0,
    );
    set(&mut image, "scale.x", json!(125));
    set(&mut image, "scale.y", json!(125));
    set(
        &mut image,
        "position.x",
        keys(json!(320), &[(0.0, json!(260)), (3.0, json!(380))], EASES[0]),
    );
    set(
        &mut image,
        "rotation",
        keys(json!(0), &[(0.0, json!(-12)), (3.0, json!(18))], EASES[0]),
    );
    let mut doc = project(640, 360, 30.0, 10.0, solid_fill("#141821"), vec![image]);
    doc["assets"]["art"] = json!({
        "id": "art", "name": "art.svg", "kind": "image", "format": "svg",
        "w": 320, "h": 180, "dur": 0, "fps": 30, "persisted": true
    });
    Fixture {
        doc,
        media: std::collections::BTreeMap::from([("art".into(), path)]),
        summary: json!({ "source": "animated SVG", "size": [320, 180], "box": [180, 101.25], "motion": "translate and rotate" }),
        dir: Some(dir),
    }
}

/// A text layer document for typing (`text`, 96 px title, over a backdrop).
pub fn text_doc(text: &str) -> Value {
    let mut l = layer(
        "t",
        "text",
        json!({ "text": text, "font": "SF Pro Display", "weight": 600, "size": 96, "tracking": -2,
            "leading": 1.1, "color": "#FFFFFF", "align": "center", "italic": false,
            "boxWidth": 0, "boxHeight": 0 }),
        960.0,
        540.0,
        10.0,
    );
    l["name"] = json!("Title");
    let back = layer(
        "back",
        "solid",
        json!({ "color": "#202436", "w": 1920, "h": 1080 }),
        0.0,
        0.0,
        10.0,
    );
    project(1920, 1080, 30.0, 10.0, solid_fill("#101418"), vec![l, back])
}

/// A video layer under an animated solid (`asset` "clip": the public 320 ×
/// 180, 2 s H.264 fixture, [`video_fixture`]).
pub fn video_doc() -> Value {
    let mut v = layer(
        "v",
        "video",
        json!({ "asset": "clip", "fit": "contain", "w": 1920, "h": 1080, "trim": 0, "speed": 1 }),
        0.0,
        0.0,
        2.0,
    );
    set(&mut v, "opacity", json!(90));
    let mut title = layer(
        "s",
        "solid",
        json!({ "color": "#E8553A", "w": 400, "h": 120 }),
        760.0,
        880.0,
        2.0,
    );
    set(
        &mut title,
        "position.x",
        keys(
            json!(760),
            &[(0.0, json!(200)), (2.0, json!(1300))],
            EASES[2],
        ),
    );
    let mut doc = project(1920, 1080, 30.0, 2.0, solid_fill("#000000"), vec![title, v]);
    doc["assets"]["clip"] = json!({
        "id": "clip", "name": "clip.mp4", "kind": "video", "format": "mp4", "w": 320, "h": 180,
        "dur": 2, "fps": 30, "persisted": true
    });
    doc
}

/// The public H.264 fixture the scrub bench seeks in.
pub fn video_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../apps/desktop/e2e/fixtures/h264-aac.mp4")
}

/// An unindexed glTF binary of `vertices` positions (a multiple of 6): a
/// wavy sheet of grid cells, two triangles each, as the 3D lanes' synthetic
/// 600,000-vertex model.
pub fn write_glb(path: &Path, vertices: usize) {
    let cells = vertices / 6;
    let cols = 500usize;
    let rows = cells.div_ceil(cols);
    let mut positions = Vec::with_capacity(cells * 72);
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    let at = |c: usize, r: usize| {
        let x = c as f32 / cols as f32 * 2.0 - 1.0;
        let y = r as f32 / rows as f32 * 2.0 - 1.0;
        [x, y, ((x * 9.0).sin() * (y * 7.0).cos()) * 0.08]
    };
    for cell in 0..cells {
        let (c, r) = (cell % cols, cell / cols);
        let (a, b, d, e) = (at(c, r), at(c + 1, r), at(c, r + 1), at(c + 1, r + 1));
        for p in [a, b, d, b, e, d] {
            for (k, v) in p.into_iter().enumerate() {
                positions.extend_from_slice(&v.to_le_bytes());
                lo[k] = lo[k].min(v);
                hi[k] = hi[k].max(v);
            }
        }
    }
    let count = cells * 6;
    let bin = positions;
    let graph = json!({
        "asset": {"version": "2.0"}, "scene": 0,
        "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": bin.len()}],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": count, "type": "VEC3",
                "min": lo, "max": hi}
        ]
    });
    let mut json = serde_json::to_vec(&graph).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut glb = Vec::with_capacity(28 + json.len() + bin.len());
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2_u32.to_le_bytes());
    glb.extend_from_slice(&(12_u32 + 8 + json.len() as u32 + 8 + bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&bin);
    std::fs::write(path, glb).expect("write the model");
}

/// A 3D scene: a camera with shadows, a shadow-casting sun, and the
/// imported model (`asset` "model"), turning over the 2 s.
pub fn model_doc() -> Value {
    let ext = |id: &str, def: &str, data: Value, p: [f64; 3]| {
        let mut l = layer(
            id,
            "extension",
            json!({ "definition": def, "version": 1, "w": 0, "h": 0, "params": {}, "data": data }),
            p[0],
            p[1],
            2.0,
        );
        l["threeD"] = json!(true);
        set(&mut l, "position.z", json!(p[2]));
        l
    };
    let camera = ext(
        "cam",
        "powermove.3d.camera",
        json!({ "camera": { "projection": "perspective", "p": {
                "targetX": ch(json!(0)), "targetY": ch(json!(0)), "targetZ": ch(json!(0)),
                "fov": ch(json!(45)), "zoom": ch(json!(1)), "near": ch(json!(0.01)), "far": ch(json!(1000))
            } },
            "environment": { "p": {
                "ambient": ch(json!(0.5)), "ambientColor": ch(json!("#FFFFFF")), "exposure": ch(json!(1))
            }, "background": null, "shadows": true } }),
        [0.0, 0.0, 3.5],
    );
    let sun = ext(
        "sun",
        "powermove.3d.light",
        json!({ "light": { "type": "sun", "castShadow": true, "p": {
            "targetX": ch(json!(0)), "targetY": ch(json!(0)), "targetZ": ch(json!(0)),
            "color": ch(json!("#FFFFFF")), "intensity": ch(json!(3)), "distance": ch(json!(0)),
            "decay": ch(json!(2)), "angle": ch(json!(45)), "penumbra": ch(json!(0.4)),
            "width": ch(json!(1)), "height": ch(json!(1))
        } } }),
        [-3.0, 5.0, 4.0],
    );
    let mut model = layer(
        "model",
        "extension",
        json!({
            "definition": "powermove.3d.object", "version": 1, "w": 1920, "h": 1080, "params": {},
            "data": { "object": {
                "source": { "assetId": "model" },
                "material": { "p": {
                    "color": ch(json!("#D8DEE9")), "roughness": ch(json!(0.4)), "metalness": ch(json!(0.1)),
                    "emissive": ch(json!("#000000")), "emissiveIntensity": ch(json!(1)), "opacity": ch(json!(1))
                }, "maps": {}, "doubleSided": true },
                "useSourceMaterials": false, "slots": [], "castShadow": true, "receiveShadow": true
            } }
        }),
        0.0,
        0.0,
        2.0,
    );
    model["threeD"] = json!(true);
    for k in ["scale.x", "scale.y", "scale.z"] {
        set(&mut model, k, json!(100));
    }
    set(
        &mut model,
        "rotation.y",
        keys(json!(0), &[(0.0, json!(-20)), (2.0, json!(20))], EASES[0]),
    );
    set(&mut model, "rotation.x", json!(-25));
    let mut doc = project(
        1920,
        1080,
        30.0,
        2.0,
        solid_fill("#0B0D12"),
        vec![camera, sun, model],
    );
    doc["assets"]["model"] =
        json!({ "id": "model", "kind": "model", "format": "glb", "name": "model.glb" });
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generator_is_the_apps_mulberry32() {
        // mulberry32 seeded with 1, as synthetic-project.ts's `random`.
        let mut r = Random(1);
        assert_eq!(r.next(), 0.6270739405881613);
    }

    #[test]
    fn synthetic_counts_every_layer() {
        let f = synthetic(Synthetic {
            images: 0,
            ..Synthetic::new(500)
        });
        assert_eq!(f.summary["layers"], 500);
        let root = f.doc["layers"].as_array().unwrap().len();
        let nested: usize = f.doc["comps"]
            .as_object()
            .unwrap()
            .values()
            .map(|c| c["layers"].as_array().unwrap().len())
            .sum();
        assert_eq!(root + nested, 500);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod model_tests {
    #[test]
    fn the_generated_model_imports_with_every_vertex() {
        let dir = super::TempDir::new("glb-test");
        let path = dir.path().join("m.glb");
        super::write_glb(&path, 600_000);
        let asset = pm_render::scene3d::gltf::asset::load(&path).expect("the model loads");
        assert_eq!(asset.model.vertices, 600_000);
    }
}
