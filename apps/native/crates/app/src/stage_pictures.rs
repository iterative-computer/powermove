//! The first-run stage's pictures, drawn by the engine: the room's light
//! and the four reels are Powermove compositions built here, rendered live
//! on a worker of their own (`Headless::draw_srgb`) into textures the stage
//! paints. Nothing is read back; the stage's GPU waits on the worker's event.
//!
//! Every key of the reels comes from `pm_shell::stage::beats`, which the
//! stage's furniture reads too.
use pm_engine_service::{FrameSpec, Headless, Moment, Output, Scene};
use pm_protocol::common::{TICKS_PER_SECOND, Ticks};
use pm_shell::stage::{Picture, Pictures, Reel, beats};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

/// The reels' composition size; the engine scales to the screen's pixels.
const REEL: (f64, f64) = (1920., 1080.);
/// The room's composition outlasts the film; the card holds its last frame.
const ROOM_SECONDS: f64 = 26.;

// ---------- Compositions ----------

fn ch(v: Value) -> Value {
    json!({ "v": v, "kf": [], "expr": null })
}
/// Keys sampled from `f` every `step` seconds over `(from, to)`, linear.
fn sampled(from: f64, to: f64, step: f64, f: impl Fn(f64) -> f64) -> Value {
    let n = ((to - from) / step).ceil() as usize;
    let kf: Vec<Value> = (0..=n)
        .map(|i| {
            let t = (from + i as f64 * step).min(to);
            json!({ "t": t, "v": f(t), "i": format!("k{i}"), "inInterp": "linear", "outInterp": "linear" })
        })
        .collect();
    json!({ "v": f(from), "kf": kf, "expr": null })
}
fn keys(points: &[(f64, f64)]) -> Value {
    let kf: Vec<Value> = points
        .iter()
        .enumerate()
        .map(|(i, (t, v))| {
            json!({ "t": t, "v": v, "i": format!("k{i}"), "inInterp": "bezier", "outInterp": "bezier",
                "inEase": {"speed": 0, "influence": 60}, "outEase": {"speed": 0, "influence": 60} })
        })
        .collect();
    json!({ "v": points[0].1, "kf": kf, "expr": null })
}
fn transform(x: f64, y: f64) -> Value {
    let mut p = serde_json::Map::new();
    for (k, v) in [
        ("perspective", 50.),
        ("position.z", 0.),
        ("anchor.z", 0.),
        ("scale.z", 100.),
        ("rotation.x", 0.),
        ("rotation.y", 0.),
        ("orientation.x", 0.),
        ("orientation.y", 0.),
        ("orientation.z", 0.),
        ("anchor.x", 0.),
        ("anchor.y", 0.),
        ("position.x", x),
        ("position.y", y),
        ("scale.x", 100.),
        ("scale.y", 100.),
        ("rotation", 0.),
        ("opacity", 100.),
        ("skew", 0.),
    ] {
        p.insert(k.into(), ch(json!(v)));
    }
    Value::Object(p)
}
fn layer(id: &str, ty: &str, d: Value, at: (f64, f64), span: (f64, f64)) -> Value {
    json!({
        "id": id, "type": ty, "name": id, "from": span.0, "dur": span.1, "on": true, "lock": false,
        "shy": false, "collapsed": true, "threeD": false, "solo": false, "color": "#8C8C8C",
        "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
        "p": transform(at.0, at.1), "fx": [], "transitionIn": null, "transitionOut": null,
        "masks": [], "d": d, "locked_intent": {}
    })
}
fn fx(id: &str, kind: &str, p: Value) -> Value {
    json!({ "id": id, "type": kind, "version": 1, "on": true, "p": p })
}
/// A layer's origin is its top-left corner until its anchor moves: solids
/// and shapes here are placed by their centres.
fn centred(mut l: Value, size: (f64, f64)) -> Value {
    l["p"]["anchor.x"] = ch(json!(size.0 / 2.));
    l["p"]["anchor.y"] = ch(json!(size.1 / 2.));
    l
}
fn solid(id: &str, color: &str, size: (f64, f64), span: (f64, f64)) -> Value {
    centred(
        layer(
            id,
            "solid",
            json!({ "w": size.0, "h": size.1, "color": color }),
            (size.0 / 2., size.1 / 2.),
            span,
        ),
        size,
    )
}
fn gradient(
    start: &str,
    end: &str,
    angle: f64,
    radial: bool,
    centre: (f64, f64),
    spread: f64,
) -> Value {
    fx(
        "gradient",
        "gradient",
        json!({
            "startColor": ch(json!(start)), "endColor": ch(json!(end)), "angle": ch(json!(angle)),
            "centerX": ch(json!(centre.0)), "centerY": ch(json!(centre.1)), "spread": ch(json!(spread)),
            "midpoint": ch(json!(50)), "radial": ch(json!(radial)), "shade": ch(json!(false)),
            "dither": ch(json!(3)), "amount": ch(json!(100))
        }),
    )
}
fn grain(amount: f64, size: f64) -> Value {
    fx(
        "grain",
        "grain",
        json!({ "amount": ch(json!(amount)), "size": ch(json!(size)) }),
    )
}
fn text(
    id: &str,
    words: &str,
    size: f64,
    weight: u32,
    color: &str,
    at: (f64, f64),
    span: (f64, f64),
) -> Value {
    layer(
        id,
        "text",
        json!({ "text": words, "font": pm_design::DISPLAY_FONT, "weight": weight, "size": size,
            "leading": 1.0, "tracking": -2, "color": color, "align": "center" }),
        at,
        span,
    )
}
fn shape(
    id: &str,
    kind: &str,
    color: &str,
    size: (f64, f64),
    at: (f64, f64),
    span: (f64, f64),
) -> Value {
    centred(
        layer(
            id,
            "shape",
            json!({ "shape": kind, "color": color, "w": size.0, "h": size.1, "radius": 0, "stroke": 0,
                "strokeColor": "#ffffff", "points": 5 }),
            at,
            span,
        ),
        size,
    )
}
fn project(w: f64, h: f64, dur: f64, layers: Vec<Value>) -> Value {
    json!({
        "id": "Pstage", "name": "Stage", "w": w, "h": h, "fps": 60, "dur": dur, "bg": "#000000",
        "backgroundFill": { "type": "solid", "angle": 0, "stops": [{ "id": "s1", "color": "#000000", "position": 0 }] },
        "compId": "Pstage-root", "compName": "Stage",
        "markers": [], "work": [0, dur], "params": {}, "revision": 0, "edits": [],
        "created": 0, "shutter": 0.5, "layers": layers, "assets": {}, "comps": {}
    })
}

/// The room: a dark ground with a soft falloff and grain, and the
/// projector's light, which gathers for the mark and settles for the reels.
pub fn room(w: f64, h: f64) -> Value {
    let all = (0., ROOM_SECONDS);
    let mut ground = solid("ground", "#000000", (w, h), all);
    ground["fx"] = json!([
        gradient("#1a1918", "#040505", 0., true, (0., -14.), 118.),
        grain(9., 1.5)
    ]);
    let mut light = solid("light", "#000000", (w, h), all);
    light["blend"] = json!("screen");
    light["fx"] = json!([gradient("#6d635a", "#000000", 0., true, (0., -6.), 62.)]);
    // The cue sheet's light: dark for the house lights, a bloom as the mark
    // arrives (4.4 s), lower for the reels (7.6 s), calm on the card (22 s).
    light["p"]["opacity"] = keys(&[
        (0., 0.),
        (1.4, 0.),
        (3.0, 22.),
        (4.4, 26.),
        (5.25, 92.),
        (7.2, 36.),
        (8.2, 16.),
        (21.4, 16.),
        (23.0, 30.),
    ]);
    project(w, h, ROOM_SECONDS, vec![light, ground])
}

/// Cut: three shots of two beats each, hard cuts on the beat.
pub fn cut() -> Value {
    let (w, h) = REEL;
    let s = beats::SHOT_SECONDS;
    let shots: [(&str, &str, &str, &str); beats::SHOTS] = [
        ("#24384c", "#0a0e13", "#f2eee6", "01"),
        ("#5a2a1e", "#120908", "#ff8a5c", "02"),
        ("#2b3a33", "#090c0b", "#d9f0e4", "03"),
    ];
    let mut layers = Vec::new();
    for (i, (top, bottom, ink, figure)) in shots.iter().enumerate() {
        let span = (i as f64 * s, s);
        let id = |part: &str| format!("shot{i}-{part}");
        let mut figure_layer = text(
            &id("figure"),
            figure,
            560.,
            800,
            ink,
            (w * 0.5, h * 0.62),
            span,
        );
        // A slow push on each shot, the next one starting wider.
        figure_layer["p"]["scale.x"] = keys(&[(span.0, 92.), (span.0 + s, 100.)]);
        figure_layer["p"]["scale.y"] = keys(&[(span.0, 92.), (span.0 + s, 100.)]);
        figure_layer["p"]["position.x"] = keys(&[
            (span.0, w * if i % 2 == 0 { 0.46 } else { 0.54 }),
            (span.0 + s, w * 0.5),
        ]);
        let mut bar = shape(
            &id("bar"),
            "rect",
            ink,
            (w * 0.42, 6.),
            (w * 0.5, h * 0.82),
            span,
        );
        bar["p"]["scale.x"] = keys(&[(span.0, 0.), (span.0 + s * 0.7, 100.)]);
        bar["p"]["opacity"] = ch(json!(55));
        let mut ground = solid(&id("ground"), "#000000", (w, h), span);
        ground["fx"] = json!([
            gradient(top, bottom, 160., false, (0., 0.), 100.),
            grain(10., 1.3)
        ]);
        layers.extend([figure_layer, bar, ground]);
    }
    project(w, h, beats::REEL_SECONDS, layers)
}

/// Ask: a title, then the Glow the agent builds on it.
pub fn ask() -> Value {
    let (w, h) = REEL;
    let all = (0., beats::REEL_SECONDS);
    let mut title = text(
        "title",
        "Night Drive",
        170.,
        700,
        "#f6efe6",
        (w / 2., h * 0.56),
        all,
    );
    let end = beats::REEL_SECONDS;
    title["fx"] = json!([fx(
        "glow",
        "glow",
        json!({
            "threshold": ch(json!(30)),
            "radius": sampled(0., end, 0.1, |t| beats::glow(t).1),
            "intensity": sampled(0., end, 0.1, |t| beats::glow(t).0),
        })
    )]);
    let mut halo = shape(
        "halo",
        "ellipse",
        "#ff9b5e",
        (w * 0.62, h * 0.34),
        (w / 2., h * 0.52),
        all,
    );
    halo["fx"] = json!([fx("blur", "blur", json!({ "amount": ch(json!(140)) }))]);
    halo["p"]["opacity"] = sampled(0., end, 0.1, |t| beats::glow(t).0 / 160. * 28.);
    let mut ground = solid("ground", "#000000", (w, h), all);
    ground["fx"] = json!([
        gradient("#141722", "#05060a", 180., false, (0., 0.), 100.),
        grain(8., 1.3)
    ]);
    project(w, h, end, vec![title, halo, ground])
}

/// Grade: a flat shot, and the grade sweeping across it behind a split.
pub fn grade() -> Value {
    let (w, h) = REEL;
    let end = beats::REEL_SECONDS;
    let all = (0., end);
    let [temperature, saturation, contrast, exposure] = beats::GRADE;
    let mut look = layer("grade", "adjustment", json!({}), (0., 0.), all);
    look["fx"] = json!([fx(
        "color",
        "color",
        json!({
            "exposure": ch(json!(exposure)), "contrast": ch(json!(contrast)),
            "saturation": ch(json!(saturation)), "hue": ch(json!(0)),
            "temperature": ch(json!(temperature))
        })
    )]);
    // The graded part grows from the left edge.
    look["masks"] = json!([{ "id": "split", "shape": "rect", "mode": "add", "on": true, "p": {
        "x": sampled(0., end, 0.05, |t| w * beats::split(t) / 2.),
        "y": ch(json!(h / 2.)),
        "w": sampled(0., end, 0.05, |t| w * beats::split(t)),
        "h": ch(json!(h)), "rotation": ch(json!(0)), "feather": ch(json!(0))
    } }]);
    let mut line = solid("split-line", "#f2eee6", (2., h), all);
    line["p"]["position.x"] = sampled(0., end, 0.05, |t| w * beats::split(t));
    line["p"]["opacity"] = sampled(0., end, 0.05, |t| {
        let s = beats::split(t);
        if s <= 0. || s >= 1. { 0. } else { 80. }
    });
    // The shot: a dusk sky, the sun low over a far ridge, a near one.
    let mut sky = solid("sky", "#000000", (w, h), all);
    sky["fx"] = json!([
        gradient("#8a8f95", "#c9b39c", 90., false, (0., 0.), 100.),
        grain(7., 1.2)
    ]);
    let mut sun = shape(
        "sun",
        "ellipse",
        "#f4e4cc",
        (150., 150.),
        (w * 0.64, h * 0.6),
        all,
    );
    sun["p"]["position.y"] = keys(&[(0., h * 0.6), (end, h * 0.63)]);
    let far = shape(
        "far",
        "rect",
        "#6f6c69",
        (w * 1.2, h * 0.3),
        (w * 0.5, h * 0.78),
        all,
    );
    let mut near = shape(
        "near",
        "rect",
        "#3d3a38",
        (w * 1.2, h * 0.24),
        (w * 0.5, h * 0.94),
        all,
    );
    near["p"]["rotation"] = ch(json!(-2.5));
    project(w, h, end, vec![line, look, near, far, sun, sky])
}

/// Depth: three objects under a sun, the camera orbiting them.
pub fn depth() -> Value {
    let (w, h) = REEL;
    let end = beats::REEL_SECONDS;
    let all = (0., end);
    let ext = |id: &str, def: &str, data: Value| {
        let mut l = layer(
            id,
            "extension",
            json!({ "definition": def, "version": 1, "w": 0, "h": 0, "params": {}, "data": data }),
            (0., 0.),
            all,
        );
        l["threeD"] = json!(true);
        l
    };
    let object = |id: &str, size: [f64; 3], color: &str, at: [f64; 3], turn: f64| {
        let mut l = ext(
            id,
            "powermove.3d.object",
            json!({ "object": {
                "source": { "primitive": "box", "parameters": { "width": size[0], "height": size[1], "depth": size[2] } },
                "material": { "p": {
                    "color": ch(json!(color)), "roughness": ch(json!(0.35)), "metalness": ch(json!(0.15)),
                    "emissive": ch(json!("#000000")), "emissiveIntensity": ch(json!(1)), "opacity": ch(json!(1))
                }, "maps": {}, "doubleSided": true },
                "useSourceMaterials": false, "slots": [], "castShadow": true, "receiveShadow": true
            } }),
        );
        l["d"]["w"] = json!(w);
        l["d"]["h"] = json!(h);
        l["p"]["position.x"] = ch(json!(at[0]));
        l["p"]["position.y"] = ch(json!(at[1]));
        l["p"]["position.z"] = ch(json!(at[2]));
        l["p"]["rotation.y"] = keys(&[(0., turn), (end, turn + 24.)]);
        l
    };
    let radius = 6.4;
    let mut camera = ext(
        "camera",
        "powermove.3d.camera",
        json!({
            "camera": { "projection": "perspective", "p": {
                "targetX": ch(json!(0)), "targetY": ch(json!(0.2)), "targetZ": ch(json!(0)),
                "fov": ch(json!(38)), "zoom": ch(json!(1)), "near": ch(json!(0.01)), "far": ch(json!(1000))
            } },
            "environment": { "p": {
                "ambient": ch(json!(0.35)), "ambientColor": ch(json!("#dfe6ee")), "exposure": ch(json!(1))
            }, "background": null, "shadows": true }
        }),
    );
    camera["p"]["position.x"] = sampled(0., end, 0.1, |t| {
        radius * beats::orbit(t).to_radians().sin()
    });
    camera["p"]["position.y"] = ch(json!(1.6));
    camera["p"]["position.z"] = sampled(0., end, 0.1, |t| {
        radius * beats::orbit(t).to_radians().cos()
    });
    let mut sun = ext(
        "sun",
        "powermove.3d.light",
        json!({ "light": { "type": "sun", "castShadow": true, "p": {
            "targetX": ch(json!(0)), "targetY": ch(json!(0)), "targetZ": ch(json!(0)),
            "color": ch(json!("#fff1de")), "intensity": ch(json!(3.2)), "distance": ch(json!(0)),
            "decay": ch(json!(2)), "angle": ch(json!(45)), "penumbra": ch(json!(0.4)),
            "width": ch(json!(1)), "height": ch(json!(1))
        } } }),
    );
    sun["p"]["position.x"] = ch(json!(-3.5));
    sun["p"]["position.y"] = ch(json!(5.));
    sun["p"]["position.z"] = ch(json!(3.));
    let floor = object("floor", [14., 0.1, 14.], "#2a2b2d", [0., -1.05, 0.], 0.);
    let tall = object("tall", [0.9, 2.1, 0.9], "#e8dccb", [-1.25, 0.05, -0.3], 18.);
    let wide = object("wide", [1.8, 0.7, 1.1], "#d06a43", [0.9, -0.65, 0.4], -12.);
    let cube = object("cube", [0.8, 0.8, 0.8], "#9fb4c4", [0.35, 0.45, -1.4], 40.);
    let mut ground = solid("ground", "#000000", (w, h), all);
    ground["fx"] = json!([gradient("#202327", "#08090a", 180., false, (0., 0.), 100.)]);
    project(
        w,
        h,
        end,
        vec![camera, sun, tall, wide, cube, floor, ground],
    )
}

fn scene(project: Value) -> anyhow::Result<Scene> {
    let value = pm_model::json::parse(&project.to_string())?;
    let snapshot = pm_kernel::document::DocumentStore::new_untitled(value).snapshot();
    Ok(Scene {
        doc: snapshot.document.clone(),
        comp: snapshot
            .open_comp()
            .ok_or_else(|| anyhow::anyhow!("the stage composition has no root"))?,
        media: Default::default(),
        content: snapshot.doc_id,
    })
}
fn scenes(room_size: (f64, f64)) -> anyhow::Result<HashMap<Picture, Scene>> {
    Ok(HashMap::from([
        (Picture::Backdrop, scene(room(room_size.0, room_size.1))?),
        (Picture::Reel(Reel::Cut), scene(cut())?),
        (Picture::Reel(Reel::Ask), scene(ask())?),
        (Picture::Reel(Reel::Grade), scene(grade())?),
        (Picture::Reel(Reel::Depth), scene(depth())?),
    ]))
}

// ---------- The worker ----------

struct Request {
    what: Picture,
    t: f64,
    target: metal::Texture,
    /// The stage's event: the worker signals `value` on it when drawn.
    event: metal::Event,
    value: u64,
    /// Timing runs: wait for the GPU and report the frame's whole cost.
    measure: bool,
}
enum Reply {
    Ready(metal::Device),
    Failed(String),
    Drawn {
        what: Picture,
        t: f64,
        value: u64,
        error: Option<String>,
        /// Measured runs: milliseconds from the request to the GPU's end.
        ms: Option<f64>,
    },
}
/// A pool texture's lease: free again once GPUI and the stage let go.
struct Lease(Arc<AtomicBool>);
impl std::fmt::Debug for Lease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StageLease")
    }
}
impl gpui::TextureLease for Lease {
    fn gpu_completed(&self, _: bool) {}
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
struct Slot {
    texture: metal::Texture,
    free: Arc<AtomicBool>,
}
#[derive(Default)]
struct Stream {
    size: (u32, u32),
    slots: Vec<Slot>,
    latest: Option<(f64, Arc<gpui::NativeTexture>)>,
    /// The time and slot of the frame being drawn.
    drawing: Option<(f64, usize, u64)>,
}

/// The engine's pictures for the stage.
pub struct EnginePictures {
    tx: Option<mpsc::Sender<Request>>,
    rx: mpsc::Receiver<Reply>,
    device: Option<metal::Device>,
    event: Option<metal::Event>,
    value: u64,
    streams: HashMap<Picture, Stream>,
    /// Recordings and captures: each asked-for frame is waited for, with
    /// `pump` run meanwhile (the font registry answers on this thread).
    pub exact: Option<Box<dyn FnMut()>>,
    /// Timing runs: each frame waits for the GPU (`measured` collects).
    pub measure: bool,
    pub measured: Vec<(Picture, f64)>,
    failed: Option<String>,
}
impl EnginePictures {
    /// Starts the worker; its engine is made there, off the main thread.
    /// `room` is the stage's size in points.
    pub fn start(
        fonts: pm_render::text::host::RenderFonts,
        shader_cache: Option<std::path::PathBuf>,
        room: (f64, f64),
    ) -> Self {
        let (tx, requests) = mpsc::channel::<Request>();
        let (replies, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("stage-pictures".into())
            .spawn(move || {
                let setup = Headless::for_surfaces(fonts, shader_cache)
                    .and_then(|engine| Ok((engine, scenes(room)?)));
                let (mut engine, scenes) = match setup {
                    Ok(v) => v,
                    Err(error) => {
                        let _ = replies.send(Reply::Failed(format!("{error:#}")));
                        return;
                    }
                };
                let _ = replies.send(Reply::Ready(engine.metal_device()));
                while let Ok(request) = requests.recv() {
                    let started = std::time::Instant::now();
                    let drawn = draw(&mut engine, &scenes, &request);
                    let ms = match (&drawn, request.measure) {
                        (Ok(command), true) => {
                            command.wait_until_completed();
                            Some(started.elapsed().as_secs_f64() * 1e3)
                        }
                        _ => None,
                    };
                    let _ = replies.send(Reply::Drawn {
                        what: request.what,
                        t: request.t,
                        value: request.value,
                        error: drawn.err().map(|e| format!("{e:#}")),
                        ms,
                    });
                }
            })
            .expect("the stage's picture worker starts");
        Self {
            tx: Some(tx),
            rx,
            device: None,
            event: None,
            value: 0,
            streams: HashMap::new(),
            exact: None,
            measure: false,
            measured: Vec::new(),
            failed: None,
        }
    }
    /// Why the engine could not draw the stage, if it could not.
    pub fn failure(&self) -> Option<&str> {
        self.failed.as_deref()
    }
    fn receive(&mut self, reply: Reply) {
        match reply {
            Reply::Ready(device) => {
                self.event = Some(device.new_event());
                self.device = Some(device);
            }
            Reply::Failed(error) => {
                eprintln!("The first-run stage draws without the engine: {error}");
                self.failed = Some(error);
            }
            Reply::Drawn {
                what,
                t,
                value,
                error,
                ms,
            } => {
                if let Some(ms) = ms {
                    self.measured.push((what, ms));
                }
                let Some(stream) = self.streams.get_mut(&what) else {
                    return;
                };
                let Some((_, slot, drawing)) = stream.drawing.take() else {
                    return;
                };
                debug_assert_eq!(drawing, value);
                let slot = &stream.slots[slot];
                match error {
                    None => {
                        let texture = gpui::NativeTexture::new(
                            slot.texture.clone(),
                            self.event.clone().unwrap(),
                            value,
                            Arc::new(Lease(slot.free.clone())),
                        );
                        match texture {
                            Ok(texture) => stream.latest = Some((t, texture)),
                            Err(error) => {
                                slot.free.store(true, Ordering::Release);
                                eprintln!("Stage picture refused: {error}");
                            }
                        }
                    }
                    Some(error) => {
                        slot.free.store(true, Ordering::Release);
                        if self.failed.is_none() {
                            eprintln!("Stage picture failed: {error}");
                        }
                        self.failed = Some(error);
                    }
                }
            }
        }
    }
    fn drain(&mut self) {
        while let Ok(reply) = self.rx.try_recv() {
            self.receive(reply);
        }
    }
    /// Waits for the worker's next reply, pumping the font registry.
    fn wait(&mut self) -> bool {
        loop {
            match self.rx.recv_timeout(Duration::from_millis(2)) {
                Ok(reply) => {
                    self.receive(reply);
                    return true;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Some(pump) = self.exact.as_mut() {
                        pump();
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return false,
            }
        }
    }
    fn texture(device: &metal::DeviceRef, (w, h): (u32, u32)) -> metal::Texture {
        let desc = metal::TextureDescriptor::new();
        desc.set_texture_type(metal::MTLTextureType::D2);
        desc.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
        desc.set_width(u64::from(w));
        desc.set_height(u64::from(h));
        desc.set_storage_mode(metal::MTLStorageMode::Private);
        desc.set_usage(metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead);
        device.new_texture(&desc)
    }
}
fn draw(
    engine: &mut Headless,
    scenes: &HashMap<Picture, Scene>,
    r: &Request,
) -> anyhow::Result<metal::CommandBuffer> {
    let scene = scenes
        .get(&r.what)
        .ok_or_else(|| anyhow::anyhow!("no composition for {:?}", r.what))?;
    let spec = FrameSpec {
        time: Ticks::new((r.t * TICKS_PER_SECOND as f64).round() as i64)?,
        output: Output {
            width: r.target.width() as u32,
            height: r.target.height() as u32,
            display: 0,
            headroom: 1.,
            crop: None,
        },
        moment: Moment::Playing,
        scale: 1.,
        decode_deadline: None,
        exact: false,
        settled: false,
    };
    engine.draw_srgb(scene, spec, &r.target, &r.event, r.value)
}

impl Pictures for EnginePictures {
    fn picture(
        &mut self,
        what: Picture,
        t: f64,
        size: (u32, u32),
    ) -> Option<Arc<gpui::NativeTexture>> {
        self.drain();
        if self.device.is_none() && self.exact.is_some() && self.failed.is_none() {
            // A recording waits for the engine to start.
            while self.device.is_none() && self.failed.is_none() && self.wait() {}
        }
        let (Some(device), Some(event), Some(tx)) = (&self.device, &self.event, &self.tx) else {
            return None;
        };
        let stream = self.streams.entry(what).or_default();
        if stream.size != size {
            // A new size: new textures (frames drawn at the old one stay
            // leased until let go).
            stream.size = size;
            stream.slots = (0..3)
                .map(|_| Slot {
                    texture: Self::texture(device, size),
                    free: Arc::new(AtomicBool::new(true)),
                })
                .collect();
            stream.latest = None;
            stream.drawing = None;
        }
        let current = stream
            .latest
            .as_ref()
            .is_some_and(|(at, _)| (*at - t).abs() < 1e-6);
        if !current && stream.drawing.is_none() {
            if let Some(slot) = stream
                .slots
                .iter()
                .position(|s| s.free.load(Ordering::Acquire))
            {
                stream.slots[slot].free.store(false, Ordering::Release);
                self.value += 1;
                stream.drawing = Some((t, slot, self.value));
                let sent = tx.send(Request {
                    what,
                    t,
                    target: stream.slots[slot].texture.clone(),
                    event: event.clone(),
                    value: self.value,
                    measure: self.measure,
                });
                if sent.is_err() {
                    stream.drawing = None;
                    stream.slots[slot].free.store(true, Ordering::Release);
                }
            }
        }
        if self.exact.is_some() {
            while self.streams.get(&what).is_some_and(|s| s.drawing.is_some()) && self.wait() {}
        }
        self.streams
            .get(&what)
            .and_then(|s| s.latest.as_ref().map(|(_, texture)| texture.clone()))
    }
    fn rest(&mut self) {
        self.drain();
        // The reels are over: their textures go; the room keeps its last frame.
        self.streams.retain(|what, _| *what == Picture::Backdrop);
    }
}
impl Drop for EnginePictures {
    fn drop(&mut self) {
        // The worker ends with its channel, and its engine with it.
        self.tx.take();
    }
}
