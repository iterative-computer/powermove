//! The first-run stage's room, drawn by the engine on a worker of its own
//! (`Headless::draw_srgb`) into textures the stage paints: a dark ground
//! with a soft falloff and grain, and the light the mark rises in. Nothing
//! is read back; the stage's GPU waits on the worker's event.
use pm_engine_service::{FrameSpec, Headless, Moment, Output, Scene};
use pm_protocol::common::{TICKS_PER_SECOND, Ticks};
use pm_shell::stage::{Pictures, cue::CueSheet};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

/// The room's composition outlasts the intro; the window holds its last frame.
const ROOM_SECONDS: f64 = 12.;

// ---------- The room ----------

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
fn project(w: f64, h: f64, dur: f64, layers: Vec<Value>) -> Value {
    json!({
        "id": "Pstage", "name": "Stage", "w": w, "h": h, "fps": 60, "dur": dur, "bg": "#000000",
        "backgroundFill": { "type": "solid", "angle": 0, "stops": [{ "id": "s1", "color": "#000000", "position": 0 }] },
        "compId": "Pstage-root", "compName": "Stage",
        "markers": [], "work": [0, dur], "params": {}, "revision": 0, "edits": [],
        "created": 0, "shutter": 0.5, "layers": layers, "assets": {}, "comps": {}
    })
}

/// The room: a dark ground with a soft falloff and grain, and the mark's
/// light: a glow at the bottom edge that rises with the mark into the
/// centre, then softens and spreads before the window is drawn.
pub fn room(w: f64, h: f64) -> Value {
    let sheet = CueSheet::default();
    let all = (0., ROOM_SECONDS);
    let mut ground = solid("ground", "#000000", (w, h), all);
    ground["fx"] = json!([
        gradient("#151413", "#040505", 0., true, (0., -10.), 120.),
        grain(8., 1.5)
    ]);
    let mut light = solid("light", "#000000", (w, h), all);
    light["blend"] = json!("screen");
    let (edge, rise, settled, spread, window) = (
        sheet.edge_light,
        sheet.rise.0,
        sheet.rise.1,
        sheet.spread,
        sheet.window,
    );
    let mut glow = gradient("#8a7766", "#000000", 0., true, (0., 52.), 38.);
    // The light's centre climbs with the mark (gradient centre, % of the
    // half-height, + down), and widens as it softens before the window.
    glow["p"]["centerY"] = keys(&[
        (0., 104.),
        (edge, 98.),
        (rise, 92.),
        (settled, -12.),
        (window + 1., -12.),
    ]);
    glow["p"]["spread"] = keys(&[
        (0., 30.),
        (edge, 34.),
        (settled, 44.),
        (spread, 52.),
        (window, 110.),
        (window + 1., 120.),
    ]);
    light["fx"] = json!([glow]);
    light["p"]["opacity"] = keys(&[
        (0., 0.),
        (edge - 0.4, 0.),
        (edge + 0.6, 55.),
        (rise + 0.4, 95.),
        (settled, 70.),
        (spread + 0.5, 60.),
        (window + 0.6, 26.),
    ]);
    project(w, h, ROOM_SECONDS, vec![light, ground])
}

fn scene(project: Value) -> anyhow::Result<Scene> {
    let value = pm_model::json::parse(&project.to_string())?;
    let snapshot = pm_kernel::document::DocumentStore::new_untitled(value).snapshot();
    Ok(Scene {
        doc: snapshot.document.clone(),
        comp: snapshot
            .open_comp()
            .ok_or_else(|| anyhow::anyhow!("the room has no composition"))?,
        media: Default::default(),
        content: snapshot.doc_id,
    })
}

// ---------- The worker ----------

struct Request {
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

/// The engine's room for the stage.
pub struct EnginePictures {
    tx: Option<mpsc::Sender<Request>>,
    rx: mpsc::Receiver<Reply>,
    device: Option<metal::Device>,
    event: Option<metal::Event>,
    value: u64,
    size: (u32, u32),
    slots: Vec<Slot>,
    latest: Option<(f64, Arc<gpui::NativeTexture>)>,
    /// The time and slot of the frame being drawn.
    drawing: Option<(f64, usize)>,
    /// Recordings and captures: each asked-for frame is waited for, with
    /// `pump` run meanwhile (the font registry answers on this thread).
    pub exact: Option<Box<dyn FnMut()>>,
    /// Timing runs: each frame waits for the GPU (`measured` collects).
    pub measure: bool,
    pub measured: Vec<f64>,
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
            .name("stage-room".into())
            .spawn(move || {
                let setup = Headless::for_surfaces(fonts, shader_cache)
                    .and_then(|engine| Ok((engine, scene(self::room(room.0, room.1))?)));
                let (mut engine, scene) = match setup {
                    Ok(v) => v,
                    Err(error) => {
                        let _ = replies.send(Reply::Failed(format!("{error:#}")));
                        return;
                    }
                };
                let _ = replies.send(Reply::Ready(engine.metal_device()));
                while let Ok(request) = requests.recv() {
                    let started = std::time::Instant::now();
                    let drawn = draw(&mut engine, &scene, &request);
                    let ms = match (&drawn, request.measure) {
                        (Ok(command), true) => {
                            command.wait_until_completed();
                            Some(started.elapsed().as_secs_f64() * 1e3)
                        }
                        _ => None,
                    };
                    let _ = replies.send(Reply::Drawn {
                        t: request.t,
                        value: request.value,
                        error: drawn.err().map(|e| format!("{e:#}")),
                        ms,
                    });
                }
            })
            .expect("the stage's room worker starts");
        Self {
            tx: Some(tx),
            rx,
            device: None,
            event: None,
            value: 0,
            size: (0, 0),
            slots: Vec::new(),
            latest: None,
            drawing: None,
            exact: None,
            measure: false,
            measured: Vec::new(),
            failed: None,
        }
    }
    /// Why the engine could not draw the room, if it could not.
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
                eprintln!("The first-run stage draws its room without the engine: {error}");
                self.failed = Some(error);
            }
            Reply::Drawn {
                t,
                value,
                error,
                ms,
            } => {
                if let Some(ms) = ms {
                    self.measured.push(ms);
                }
                let Some((_, slot)) = self.drawing.take() else {
                    return;
                };
                let slot = &self.slots[slot];
                let texture = match error {
                    None => gpui::NativeTexture::new(
                        slot.texture.clone(),
                        self.event.clone().unwrap(),
                        value,
                        Arc::new(Lease(slot.free.clone())),
                    )
                    .map_err(str::to_owned),
                    Some(error) => Err(error),
                };
                match texture {
                    Ok(texture) => self.latest = Some((t, texture)),
                    Err(error) => {
                        slot.free.store(true, Ordering::Release);
                        if self.failed.is_none() {
                            eprintln!("The stage's room failed: {error}");
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
fn draw(engine: &mut Headless, scene: &Scene, r: &Request) -> anyhow::Result<metal::CommandBuffer> {
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
    fn picture(&mut self, t: f64, size: (u32, u32)) -> Option<Arc<gpui::NativeTexture>> {
        self.drain();
        if self.device.is_none() && self.exact.is_some() && self.failed.is_none() {
            // A recording waits for the engine to start.
            while self.device.is_none() && self.failed.is_none() && self.wait() {}
        }
        let (Some(device), Some(event), Some(tx)) = (&self.device, &self.event, &self.tx) else {
            return None;
        };
        if self.size != size {
            self.size = size;
            self.slots = (0..3)
                .map(|_| Slot {
                    texture: Self::texture(device, size),
                    free: Arc::new(AtomicBool::new(true)),
                })
                .collect();
            self.latest = None;
            self.drawing = None;
        }
        let current = self
            .latest
            .as_ref()
            .is_some_and(|(at, _)| (*at - t).abs() < 1e-6);
        if !current
            && self.drawing.is_none()
            && let Some(slot) = self
                .slots
                .iter()
                .position(|s| s.free.load(Ordering::Acquire))
        {
            self.slots[slot].free.store(false, Ordering::Release);
            self.value += 1;
            self.drawing = Some((t, slot));
            let sent = tx.send(Request {
                t,
                target: self.slots[slot].texture.clone(),
                event: event.clone(),
                value: self.value,
                measure: self.measure,
            });
            if sent.is_err() {
                self.drawing = None;
                self.slots[slot].free.store(true, Ordering::Release);
            }
        }
        if self.exact.is_some() {
            while self.drawing.is_some() && self.wait() {}
        }
        self.latest.as_ref().map(|(_, texture)| texture.clone())
    }
}
impl Drop for EnginePictures {
    fn drop(&mut self) {
        // The worker ends with its channel, and its engine with it.
        self.tx.take();
    }
}
