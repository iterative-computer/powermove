//! The first-run stage's pictures, drawn by the engine on a worker of their
//! own (`Headless::draw_srgb`) into textures the stage paints: the room's
//! light, a composition built here, and the film's project as the editor's
//! viewer shows it. Nothing is read back; the stage's GPU waits on the
//! worker's event.
use pm_engine_service::{FrameSpec, Headless, Moment, Output, Scene};
use pm_protocol::common::{TICKS_PER_SECOND, Ticks};
use pm_shell::stage::{Picture, Pictures};
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

/// The room's composition outlasts the film; the card holds its last frame.
const ROOM_SECONDS: f64 = 26.;

// ---------- Compositions ----------

fn ch(v: Value) -> Value {
    json!({ "v": v, "kf": [], "expr": null })
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

/// The film's project: the one the editor opens and the agent changes. Two
/// shots cut back to back (where the light leak goes), a title over the
/// first, a few graphic layers: a timeline with something on it.
pub fn film() -> Value {
    let (w, h) = (1920., 1080.);
    let dur = 8.;
    let a = (0., 4.);
    let b = (4., 4.);
    let mut dusk = solid("Dusk", "#000000", (w, h), a);
    dusk["fx"] = json!([
        gradient("#5b2f22", "#120a0b", 160., false, (0., 0.), 100.),
        grain(8., 1.3)
    ]);
    let mut title = text(
        "Title",
        "Night Drive",
        170.,
        700,
        "#f6efe6",
        (w / 2., h * 0.56),
        a,
    );
    title["p"]["opacity"] = keys(&[(0., 0.), (0.8, 100.), (3.4, 100.), (4., 0.)]);
    title["p"]["scale.x"] = keys(&[(0., 94.), (4., 100.)]);
    title["p"]["scale.y"] = keys(&[(0., 94.), (4., 100.)]);
    let mut sun = shape(
        "Sun",
        "ellipse",
        "#ff9b5e",
        (260., 260.),
        (w * 0.5, h * 0.78),
        a,
    );
    sun["p"]["position.y"] = keys(&[(0., h * 0.86), (4., h * 0.74)]);
    sun["fx"] = json!([fx(
        "glow",
        "glow",
        json!({
            "threshold": ch(json!(20)), "radius": ch(json!(120)), "intensity": ch(json!(90))
        })
    )]);
    let mut city = solid("City", "#000000", (w, h), b);
    city["fx"] = json!([
        gradient("#1b2b3f", "#06080c", 180., false, (0., 0.), 100.),
        grain(8., 1.3)
    ]);
    let mut towers = Vec::new();
    for (i, (x, height, colour)) in [
        (0.18, 0.46, "#2c3e56"),
        (0.33, 0.62, "#364b66"),
        (0.5, 0.38, "#24344a"),
        (0.66, 0.7, "#3d5574"),
        (0.82, 0.5, "#2a3b52"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut tower = shape(
            &format!("Tower {}", i + 1),
            "rect",
            colour,
            (w * 0.12, h * height),
            (w * x, h * (1. - height / 2.)),
            b,
        );
        tower["p"]["position.x"] = keys(&[(4., w * x + 40.), (8., w * x - 40.)]);
        towers.push(tower);
    }
    let mut lights = text(
        "Lights",
        "· · · · · · · · ·",
        90.,
        700,
        "#ffd28a",
        (w / 2., h * 0.3),
        b,
    );
    lights["p"]["opacity"] = keys(&[(4., 0.), (5., 80.)]);
    let mut layers = vec![title, sun, dusk, lights];
    layers.extend(towers);
    layers.push(city);
    let mut project = project(w, h, dur, layers);
    project["name"] = json!("Night Drive");
    project["compName"] = json!("Night Drive");
    project
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
fn scenes(room_size: (f64, f64), film: &Value) -> anyhow::Result<HashMap<Picture, Scene>> {
    Ok(HashMap::from([
        (Picture::Backdrop, scene(room(room_size.0, room_size.1))?),
        (Picture::Viewer, scene(film.clone())?),
    ]))
}

// ---------- The worker ----------

/// What the worker is asked: a frame, or the film's project as it is now.
enum Job {
    Draw(Request),
    /// The film's viewer draws this scene with these definitions from now
    /// on (the agent changed the project, or an extension it wrote arrived).
    Viewer(Scene, Arc<pm_engine_service::resources::ResourceSnapshot>),
}
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
    tx: Option<mpsc::Sender<Job>>,
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
    /// `film` is the film's project, as the editor's viewer shows it.
    pub fn start(
        fonts: pm_render::text::host::RenderFonts,
        shader_cache: Option<std::path::PathBuf>,
        room: (f64, f64),
        film: Value,
    ) -> Self {
        let (tx, requests) = mpsc::channel::<Job>();
        let (replies, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("stage-pictures".into())
            .spawn(move || {
                let setup = Headless::for_surfaces(fonts, shader_cache)
                    .and_then(|engine| Ok((engine, scenes(room, &film)?)));
                let (mut engine, mut scenes) = match setup {
                    Ok(v) => v,
                    Err(error) => {
                        let _ = replies.send(Reply::Failed(format!("{error:#}")));
                        return;
                    }
                };
                let _ = replies.send(Reply::Ready(engine.metal_device()));
                while let Ok(job) = requests.recv() {
                    let request = match job {
                        Job::Draw(request) => request,
                        Job::Viewer(scene, resources) => {
                            engine.set_resources(resources);
                            scenes.insert(Picture::Viewer, scene);
                            continue;
                        }
                    };
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
    /// The film's viewer pictures `project` (a kernel snapshot of the film's
    /// project) with `resources` (what its extensions contribute) from the
    /// next frame on.
    pub fn set_viewer(
        &mut self,
        project: &pm_kernel::Published,
        resources: Arc<pm_engine_service::resources::ResourceSnapshot>,
    ) {
        let Some(comp) = project.snapshot.open_comp() else {
            return;
        };
        let scene = Scene {
            doc: project.snapshot.document.clone(),
            comp,
            media: Default::default(),
            content: project.snapshot.doc_id,
        };
        if let Some(tx) = &self.tx
            && tx.send(Job::Viewer(scene, resources)).is_ok()
            && let Some(stream) = self.streams.get_mut(&Picture::Viewer)
        {
            // The frame held is the old project's: the next ask draws again.
            stream.latest = stream.latest.take().map(|(_, texture)| (f64::NAN, texture));
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
        if !current
            && stream.drawing.is_none()
            && let Some(slot) = stream
                .slots
                .iter()
                .position(|s| s.free.load(Ordering::Acquire))
        {
            stream.slots[slot].free.store(false, Ordering::Release);
            self.value += 1;
            stream.drawing = Some((t, slot, self.value));
            let sent = tx.send(Job::Draw(Request {
                what,
                t,
                target: stream.slots[slot].texture.clone(),
                event: event.clone(),
                value: self.value,
                measure: self.measure,
            }));
            if sent.is_err() {
                stream.drawing = None;
                stream.slots[slot].free.store(true, Ordering::Release);
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
        // The editor is gone: its textures go; the room keeps its last frame.
        self.streams.retain(|what, _| *what == Picture::Backdrop);
    }
}
impl Drop for EnginePictures {
    fn drop(&mut self) {
        // The worker ends with its channel, and its engine with it.
        self.tx.take();
    }
}
