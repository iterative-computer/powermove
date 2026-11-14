use base64::Engine;
use pm_recorder::{
    Affine, Canvas, Cap, Direction, FillRule, Gradient, GradientKind, ImageData, Join, Paint,
    Pattern, Point, Rect, Repetition, Source, TextAlign, TextBaseline, TextProvider, Vec2,
};
use pm_runtime_v8::*;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex},
};

/// A kept document, its open composition, and how many rasters keep it.
type Pin = (Arc<pm_model::Document>, String, usize);
/// Documents kept for windowed base rasters at most (distinct revisions).
pub(crate) const MAX_PINNED_REVISIONS: usize = 8;
/// Windowed base rasters keeping a document at most: the realm releases
/// them as it collects them; past this, bases are read whole instead.
const MAX_PINS: usize = 4096;

#[derive(Default)]
struct State {
    next: u64,
    paints: BTreeMap<String, Paint>,
    surfaces: BTreeMap<String, Canvas>,
    pins: BTreeMap<(String, u64), (Source, usize)>,
    missing: BTreeSet<String>,
    dom: Vec<Value>,
    bindings: BTreeMap<u64, String>,
    dom_sequence: u64,
    /// Decoded images by id (`image-N`, `svg-N` for vector sources).
    images: BTreeMap<String, crate::image::Encoded>,
    /// Bytes held by images and their SVG rasters, against `IMAGE_BUDGET`.
    image_bytes: u64,
    /// An SVG raster pin's bytes and its raster-to-intrinsic scale per axis.
    rasters: BTreeMap<(String, u64), (u64, f64, f64)>,
    /// Videos `<video>` elements opened (`video-N`), for their frames.
    videos: BTreeMap<String, crate::video::Video>,
}
/// Decoded image memory one realm may hold at once.
const IMAGE_BUDGET: u64 = 1 << 30;
/// SVG rasters are cached in quarter-scale steps: version `q` is `q / 4` scale.
const SVG_STEPS: f64 = 4.;
/// Everything a host owner hears from its realm, without waiting on it.
pub trait RealmEvents: Send + Sync + 'static {
    fn health(&self, _id: &str, _health: &Value) {}
    fn toast(&self, _text: &str, _options: &Value) {}
    /// A question for the main thread; the answer returns through the inbox.
    fn request(&self, _request: Value) {}
    fn set_enabled(&self, _id: &str, _enabled: bool) {}
    fn contributions(&self, _value: Value) {}
    fn dom_ready(&self) {}
    fn canvases_ready(&self) {}
    fn ready(&self) {}
    fn activating(&self, _id: Option<&str>) {}
    /// A task has run past the watchdog's limit; the user decides (D11).
    fn unresponsive(&self, _execution: u64) {}
    /// A synchronous layout read of measure `request` (already sent with the
    /// realm's last batch), `answering` when a raster answer makes it. Blocks
    /// the realm's worker only.
    fn measure_sync(&self, _request: u64, _answering: bool) -> SyncLayout {
        SyncLayout::NoPage
    }
}
/// How a synchronous layout read ended.
pub enum SyncLayout {
    /// The page's `layout` answer.
    Answer(Value),
    /// No page reads this realm's batches, so none will answer.
    NoPage,
    /// A page reads them but did not answer in time.
    TimedOut,
}
impl SyncLayout {
    fn into_json(self) -> Value {
        match self {
            Self::Answer(layout) => layout,
            Self::NoPage => json!({"unavailable": "no-page"}),
            Self::TimedOut => json!({"unavailable": "timeout"}),
        }
    }
}
pub struct NoEvents;
impl RealmEvents for NoEvents {}
/// Canvas text shapes against the app's font service.
pub type CanvasText = Box<dyn TextProvider + Send>;
pub struct NativeHost {
    state: Mutex<State>,
    kernel: Mutex<crate::adapter::KernelAdapter>,
    /// Documents windowed base rasters were made from, by revision, with how
    /// many such rasters the realm still holds: loading one later paints the
    /// frame it was returned for, not the project as it is now.
    pins: Mutex<HashMap<u64, Pin>>,
    transport: crate::transport::Transport,
    events: Box<dyn RealmEvents>,
    text: Mutex<Option<CanvasText>>,
    frames: Mutex<Option<crate::FrameRenderer>>,
    rasters: Mutex<Option<crate::RasterHooks>>,
    effects: Mutex<Option<crate::EffectsSlot>>,
    /// The app's pasteboard (`navigator.clipboard`): a window's realm only.
    clipboard: Mutex<Option<crate::TextClipboard>>,
    /// [`crate::code_identity`] of the records this realm runs (0: unknown).
    code: std::sync::atomic::AtomicU64,
    /// Values for extensions' declared `vars`, staged by the owner for a
    /// load it is about to ask of the realm, by single-use token.
    vars: Mutex<Vec<(String, Value)>>,
    /// The capture (`renderFrameTo`, `PM.Export.snapshot`) the realm waits for.
    capture: Mutex<Option<Capture>>,
    /// While a capture runs, override asks for this realm come here: the
    /// waiting realm answers them inline (the capture draws its own project).
    nested: Mutex<Option<std::sync::mpsc::Sender<CaptureEvent>>>,
}
pub(crate) enum CaptureEvent {
    Frame(std::result::Result<Vec<u8>, String>),
    Asks(u64, Value),
}
struct Capture {
    method: String,
    events: std::sync::mpsc::Receiver<CaptureEvent>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
/// Straight RGBA8 to premultiplied, as a canvas's backing store holds it.
/// Straight RGBA8 rows put into a canvas and read back as an answer reads
/// a canvas: premultiplied rows.
pub(crate) fn canvas_round_trip(width: u32, height: u32, straight: Vec<u8>) -> Result<Vec<u8>> {
    let pixels = ImageData::from_rgba(width, height, straight).map_err(host_error)?;
    let mut canvas = Canvas::new(0, width, height).map_err(host_error)?;
    canvas
        .put_image_data(pixels, 0, 0, None)
        .map_err(host_error)?;
    let read = canvas
        .get_image_data(0, 0, width as i32, height as i32)
        .map_err(host_error)?;
    Ok(premultiply(read.bytes()))
}
fn premultiply(straight: &[u8]) -> Vec<u8> {
    let mut out = straight.to_vec();
    for px in out.chunks_exact_mut(4) {
        let a = u32::from(px[3]);
        for c in &mut px[..3] {
            *c = ((u32::from(*c) * a + 127) / 255) as u8;
        }
    }
    out
}
pub(crate) fn host_error(error: pm_recorder::Error) -> Error {
    Error::Host(format!("recorder: {error}"))
}
fn num(args: &[Value], index: usize) -> Result<f64> {
    args.get(index)
        .and_then(Value::as_f64)
        .ok_or(Error::Invalid("numeric recorder argument"))
}
fn dimension(value: Option<&Value>) -> Result<u32> {
    value
        .and_then(Value::as_u64)
        .and_then(|n| n.try_into().ok())
        .ok_or(Error::Invalid("canvas dimension"))
}
fn boolean(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_f64().is_some_and(|n| n != 0.),
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}
fn fill_rule(value: Option<&Value>) -> Result<FillRule> {
    match value.and_then(Value::as_str) {
        None if value.is_none() => Ok(FillRule::NonZero),
        Some("nonzero") => Ok(FillRule::NonZero),
        Some("evenodd") => Ok(FillRule::EvenOdd),
        _ => Err(Error::Invalid("canvas winding rule")),
    }
}
fn numbers(value: &Value) -> Result<Vec<f64>> {
    value
        .as_array()
        .ok_or(Error::Invalid("numeric array"))?
        .iter()
        .map(|v| v.as_f64().ok_or(Error::Invalid("numeric array")))
        .collect()
}
fn matrix(values: &[Value]) -> Result<Affine> {
    let mut coefficients = [0.; 6];
    for (index, item) in coefficients.iter_mut().enumerate() {
        *item = num(values, index)?;
    }
    Ok(Affine::new(coefficients))
}
pub(crate) fn encode_png(image: &ImageData) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, image.width(), image.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = encoder
            .write_header()
            .map_err(|e| Error::Host(e.to_string()))?;
        writer
            .write_image_data(image.bytes())
            .map_err(|e| Error::Host(e.to_string()))?;
    }
    Ok(bytes)
}
impl NativeHost {
    pub fn new(
        kernel: crate::adapter::KernelAdapter,
        transport: crate::transport::Transport,
        events: Box<dyn RealmEvents>,
    ) -> Self {
        Self {
            state: Mutex::new(State::default()),
            kernel: Mutex::new(kernel),
            pins: Mutex::default(),
            transport,
            events,
            text: Mutex::new(None),
            frames: Mutex::new(None),
            rasters: Mutex::new(None),
            effects: Mutex::new(None),
            clipboard: Mutex::new(None),
            code: std::sync::atomic::AtomicU64::new(0),
            vars: Mutex::default(),
            capture: Mutex::new(None),
            nested: Mutex::new(None),
        }
    }
    /// An override ask while this realm waits for a capture: handed to the
    /// wait (true), or not (false: the realm is free; use its inbox).
    pub(crate) fn nested_ask(&self, batch: u64, ask: Value) -> std::result::Result<(), Value> {
        match self.nested.lock().unwrap().as_ref() {
            Some(tx) => tx
                .send(CaptureEvent::Asks(batch, ask))
                .map_err(|e| match e.0 {
                    CaptureEvent::Asks(_, ask) => ask,
                    CaptureEvent::Frame(_) => Value::Null,
                }),
            None => Err(ask),
        }
    }
    /// While the realm waits for a capture: the document and revision it is at.
    pub(crate) fn capturing_at(&self) -> Option<(u64, u64)> {
        self.nested.lock().unwrap().as_ref()?;
        self.kernel.try_lock().ok()?.sent_revision()
    }
    fn end_capture(&self) {
        *self.nested.lock().unwrap() = None;
        if let Some(capture) = self.capture.lock().unwrap().take() {
            capture
                .stop
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }
    pub fn set_rasters(&self, hooks: Option<crate::RasterHooks>) {
        *self.rasters.lock().unwrap() = hooks;
    }
    /// Stages `values` (`{extension id: {key: value}}`, chosen by the owner
    /// from the manifests it read) for the load the owner asks for next, and
    /// returns the token that load takes them with, once. The realm has no
    /// other way to values: it cannot name an extension or a key, and a
    /// token it has not been handed by the owner's own call answers nothing.
    pub fn stage_vars(&self, values: Value) -> String {
        use std::hash::BuildHasher;
        let half = || std::collections::hash_map::RandomState::new().hash_one(0u8);
        let token = format!("{:016x}{:016x}", half(), half());
        let mut staged = self.vars.lock().unwrap();
        // Loads the realm never ran (it was stopped first) leave theirs.
        if staged.len() >= 8 {
            staged.remove(0);
        }
        staged.push((token.clone(), values));
        token
    }
    /// The records this realm boots with or updates to, before it evaluates
    /// them: what its `producing` calls report as the wrappers' code.
    pub fn set_code(&self, records: &Value) {
        self.code.store(
            crate::code_identity(records),
            std::sync::atomic::Ordering::Release,
        );
    }
    /// A recorder canvas holding straight RGBA8 pixels, as a surface id.
    fn canvas_of(&self, width: u32, height: u32, rgba: Vec<u8>) -> Result<String> {
        crate::image::rgba_bytes(u64::from(width), u64::from(height))?;
        let pixels = ImageData::from_rgba(width, height, rgba).map_err(host_error)?;
        let mut state = self.state.lock().unwrap();
        state.next += 1;
        let id = state.next;
        let mut canvas = Canvas::new(id, width, height).map_err(host_error)?;
        canvas
            .put_image_data(pixels, 0, 0, None)
            .map_err(host_error)?;
        state.surfaces.insert(id.to_string(), canvas);
        Ok(id.to_string())
    }
    /// `render.raster(layer, scale, time, uploaded, crop)`'s base: the layer as
    /// the engine paints it, with today's geometry, as a recorder canvas.
    /// Args: `[{id, content?, composition?}, scale, time, crop?]`.
    fn base_raster(&self, args: &Value, revision: Option<u64>) -> Result<Value> {
        let Some(reply) = self.base_reply(args, revision, None)? else {
            return Ok(Value::Null);
        };
        let surface = self.canvas_of(reply.width, reply.height, reply.straight_rgba)?;
        Ok(
            json!({"kind":"recorder-surface","surface":surface,"width":reply.width,
            "height":reply.height,"w":reply.size.0,"h":reply.size.1,
            "anchorX":reply.anchor.0,"anchorY":reply.anchor.1,"uv":reply.uv,
            "selection":reply.selection.map(|[x0,y0,x1,y1]| json!({"x0":x0,"y0":y0,"x1":x1,"y1":y1})),
            "fontOffset":reply.font_offset.map(|(x,y)| json!({"x":x,"y":y}))}),
        )
    }
    /// Keeps the project's document as it is now while a windowed base
    /// raster answered from it lives (one count per raster): its revision,
    /// or `None` without a project or past the caps (the realm then reads
    /// that raster whole now, as an override's canvas).
    fn pin(&self) -> Option<u64> {
        let (revision, document, open) = self.kernel.lock().unwrap().revision_document().ok()?;
        let mut pins = self.pins.lock().unwrap();
        let held: usize = pins.values().map(|pin| pin.2).sum();
        if held >= MAX_PINS || (!pins.contains_key(&revision) && pins.len() >= MAX_PINNED_REVISIONS)
        {
            return None;
        }
        pins.entry(revision).or_insert((document, open, 0)).2 += 1;
        Some(revision)
    }
    /// The engine's base raster for `render.raster`'s `args`, or only its
    /// `window` (of the whole raster): of the pinned document of `revision`
    /// if there is one, else of the project as it is.
    fn base_reply(
        &self,
        args: &Value,
        revision: Option<u64>,
        window: Option<[u32; 4]>,
    ) -> Result<Option<crate::BaseRasterReply>> {
        let hooks = self.rasters.lock().unwrap().clone().ok_or_else(|| {
            Error::Host("render.raster needs the engine, which this host does not have".into())
        })?;
        let spec = &args[0];
        let pinned = revision.and_then(|r| {
            let pins = self.pins.lock().unwrap();
            pins.get(&r)
                .map(|(document, open, _)| (document.clone(), open.clone()))
        });
        let (document, open) = match pinned {
            Some(pinned) => pinned,
            None => self.kernel.lock().unwrap().document()?,
        };
        let number = |i: usize, default: f64| {
            args.get(i)
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite())
                .unwrap_or(default)
        };
        let crop = args.get(3).and_then(|c| {
            let at = |k: &str| c[k].as_f64().filter(|v| *v >= 0.).map(|v| v as u32);
            Some([at("x")?, at("y")?, at("width")?, at("height")?])
        });
        let content = match &spec["content"] {
            Value::Object(_) => Some(
                pm_model::json::parse(&spec["content"].to_string())
                    .map_err(|_| Error::Invalid("raster layer object"))?,
            ),
            _ => None,
        };
        (hooks.base)(crate::BaseRasterRequest {
            document,
            composition: spec["composition"].as_str().map_or(open, str::to_owned),
            layer: spec["id"].as_str().unwrap_or_default().to_owned(),
            content,
            seconds: number(2, self.transport.time()),
            scale: number(1, 1.).max(1e-6),
            crop,
            window,
        })
        .map_err(Error::Host)
    }
    /// An untouched base raster an override returned without the frame's
    /// crop (`{baseWindow: {args, window}}`): the whole raster's answer with
    /// only the window painted, its pixels what the whole canvas's answer
    /// has there (the same canvas round trip, by pixel).
    fn base_window_answer(&self, request: &Value) -> crate::RasterAnswer {
        let at = |k: &str| {
            request["window"][k]
                .as_f64()
                .filter(|v| *v >= 0.)
                .map(|v| v as u32)
        };
        let Some(window) = (|| Some([at("x")?, at("y")?, at("width")?, at("height")?]))() else {
            return crate::RasterAnswer::Unavailable("The raster window is not readable".into());
        };
        let reply =
            match self.base_reply(&request["args"], request["revision"].as_u64(), Some(window)) {
                Ok(Some(reply)) => reply,
                // As the canvas of a base raster that drew nothing.
                Ok(None) => {
                    return crate::RasterAnswer::Failed(
                        "The override returned a canvas the engine cannot read".into(),
                    );
                }
                Err(error) => return crate::RasterAnswer::Unavailable(error.to_string()),
            };
        let held = reply.window.unwrap_or([0, 0, reply.width, reply.height]);
        let pixels = if held[2] == 0 || held[3] == 0 {
            Ok(Vec::new())
        } else {
            canvas_round_trip(held[2], held[3], reply.straight_rgba)
        };
        match pixels {
            Ok(pixels) => crate::RasterAnswer::Raster {
                width: reply.width,
                height: reply.height,
                premultiplied_rgba: pixels,
                size: [reply.size.0, reply.size.1],
                anchor: [reply.anchor.0, reply.anchor.1],
                uv: reply.uv,
                window: Some(held),
            },
            Err(error) => crate::RasterAnswer::Unavailable(error.to_string()),
        }
    }
    /// One batch of override answers: `{batch, answers: [{native} | {hidden} |
    /// {retry} | {error} | {surface, w, h, anchorX, anchorY, uv?}]}`, the
    /// canvases read here, on the realm worker.
    fn raster_answers(&self, input: &Value) -> Result<Value> {
        let Some(hooks) = self.rasters.lock().unwrap().clone() else {
            return Ok(Value::Null);
        };
        let batch = input["batch"]
            .as_u64()
            .ok_or(Error::Invalid("raster batch"))?;
        let items = input["answers"]
            .as_array()
            .ok_or(Error::Invalid("raster answers"))?;
        // Base windows paint on the engine before the canvases are read.
        let mut windows: Vec<Option<crate::RasterAnswer>> = items
            .iter()
            .map(|item| {
                item["baseWindow"]
                    .is_object()
                    .then(|| self.base_window_answer(&item["baseWindow"]))
            })
            .collect();
        let mut answers = Vec::with_capacity(items.len());
        let mut state = self.state.lock().unwrap();
        for (i, item) in items.iter().enumerate() {
            if let Some(answer) = windows[i].take() {
                answers.push(answer);
                continue;
            }
            let surface = item["surface"]
                .as_str()
                .and_then(|id| state.surfaces.get_mut(id));
            let number = |k: &str, default: f64| {
                item[k]
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .unwrap_or(default)
            };
            let answer = match surface {
                _ if item["native"] == true => crate::RasterAnswer::Native,
                _ if item["hidden"] == true => crate::RasterAnswer::Hidden,
                _ if item["retry"] == true => crate::RasterAnswer::Retry,
                _ if item["moved"] == true => crate::RasterAnswer::Moved,
                // The override's own throw (or unreadable result) is
                // deterministic; anything else the realm reports is not.
                _ if item["error"].is_string() => {
                    let error = item["error"].as_str().unwrap_or_default().into();
                    if item["threw"] == true {
                        crate::RasterAnswer::Failed(error)
                    } else {
                        crate::RasterAnswer::Unavailable(error)
                    }
                }
                // An empty canvas draws nothing, as on main.
                Some(canvas) if canvas.size().0 == 0 || canvas.size().1 == 0 => {
                    crate::RasterAnswer::Hidden
                }
                Some(canvas) => {
                    let (width, height) = canvas.size();
                    match canvas.get_image_data(0, 0, width as i32, height as i32) {
                        Ok(pixels) => crate::RasterAnswer::Raster {
                            window: None,
                            width,
                            height,
                            premultiplied_rgba: premultiply(pixels.bytes()),
                            size: [
                                number("w", f64::from(width)),
                                number("h", f64::from(height)),
                            ],
                            anchor: [number("anchorX", 0.), number("anchorY", 0.)],
                            uv: match item["uv"].as_array().map(Vec::as_slice) {
                                Some([a, b, c, d]) => {
                                    [a, b, c, d].map(|v| v.as_f64().unwrap_or(0.))
                                }
                                _ => [0., 0., 1., 1.],
                            },
                        },
                        Err(error) => crate::RasterAnswer::Unavailable(error.to_string()),
                    }
                }
                None => crate::RasterAnswer::Unavailable("The override's canvas is gone".into()),
            };
            answers.push(answer);
        }
        drop(state);
        (hooks.answers)(batch, answers);
        Ok(Value::Null)
    }
    #[cfg(test)]
    pub(crate) fn kernel_size(&self) -> (f64, f64) {
        self.kernel.lock().unwrap().size().unwrap()
    }
    /// A window realm's effect clipboard (an export realm has none).
    pub fn set_clipboard(&self, clipboard: Option<crate::TextClipboard>) {
        *self.clipboard.lock().unwrap() = clipboard;
    }
    pub fn set_effects(&self, effects: Option<crate::EffectsSlot>) {
        *self.effects.lock().unwrap() = effects;
    }
    pub fn set_frames(&self, frames: Option<crate::FrameRenderer>) {
        *self.frames.lock().unwrap() = frames;
    }
    /// `render.renderFrameTo(time, width, height)` and `PM.Export.snapshot(time,
    /// maxWidth)`: an engine render of the open composition. The frame becomes a
    /// recorder canvas (pixels the extension can read) or a JPEG data URL.
    fn render_query(&self, method: &str, args: &Value, budget: &HostBudget) -> Result<Value> {
        let renderer = self.frames.lock().unwrap().clone().ok_or_else(|| {
            Error::Host(format!(
                "{method} needs the engine, which this host does not have"
            ))
        })?;
        let number = |i: usize| args.get(i).and_then(Value::as_f64);
        let time = number(0).filter(|t| t.is_finite()).unwrap_or(0.).max(0.);
        let (width, height, motion_blur) = match method {
            // renderFrameTo(time, width, height, {mblur}): motion blur unless off.
            "render.renderFrameTo" => (
                number(1).ok_or(Error::Invalid("render width"))?,
                number(2).ok_or(Error::Invalid("render height"))?,
                args.get(3).and_then(|o| o.get("mblur")) != Some(&Value::Bool(false)),
            ),
            _ => {
                // Snapshot: scaled down to at most `maxWidth`, never up; no blur.
                let (w, h) = self.kernel.lock().unwrap().size()?;
                let scale = (number(1).unwrap_or(480.) / w).min(1.);
                (
                    (w * scale).round().max(2.),
                    (h * scale).round().max(2.),
                    false,
                )
            }
        };
        let side = |v: f64| {
            (v.is_finite() && (1. ..=16384.).contains(&v.round())).then(|| v.round() as u32)
        };
        let (Some(width), Some(height)) = (side(width), side(height)) else {
            return Err(Error::Invalid("render size"));
        };
        crate::image::rgba_bytes(u64::from(width), u64::from(height))?;
        // The capture renders on its own thread; this realm waits for it, and
        // answers the overrides it asks for meanwhile (`wait_capture`).
        self.end_capture();
        let (tx, events) = std::sync::mpsc::channel();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = stop.clone();
        *self.nested.lock().unwrap() = Some(tx.clone());
        let published = self.kernel.lock().unwrap().capture_publication();
        std::thread::Builder::new()
            .name("pm-compat-capture".into())
            .spawn(move || {
                let stop = || stopped.load(std::sync::atomic::Ordering::Acquire);
                let png = renderer(&crate::FrameRequest {
                    time,
                    width,
                    height,
                    motion_blur,
                    stop: &stop,
                    published,
                });
                let _ = tx.send(CaptureEvent::Frame(png));
            })
            .map_err(|e| Error::Host(e.to_string()))?;
        *self.capture.lock().unwrap() = Some(Capture {
            method: method.to_owned(),
            events,
            stop,
        });
        self.wait_capture(budget)
    }
    /// The capture's frame, or the next override ask it has for this realm
    /// (`{kind: "raster-asks", batches}`: the realm answers, then continues).
    fn wait_capture(&self, budget: &HostBudget) -> Result<Value> {
        use std::sync::mpsc::RecvTimeoutError;
        loop {
            let event = {
                let capture = self.capture.lock().unwrap();
                let capture = capture
                    .as_ref()
                    .ok_or(Error::Invalid("no capture is running"))?;
                capture
                    .events
                    .recv_timeout(std::time::Duration::from_millis(20))
            };
            match event {
                Ok(CaptureEvent::Asks(batch, ask)) => {
                    return Ok(json!({"kind":"raster-asks","batches":[{
                        "batch": batch, "needs": ask["needs"], "ask": ask["ask"]}]}));
                }
                Ok(CaptureEvent::Frame(png)) => {
                    let method = self
                        .capture
                        .lock()
                        .unwrap()
                        .as_ref()
                        .map(|c| c.method.clone())
                        .unwrap_or_default();
                    self.end_capture();
                    let png = png.map_err(Error::Host)?;
                    budget.check()?;
                    return self.decode_frame(&method, &png);
                }
                Err(RecvTimeoutError::Timeout) => {
                    if let Err(error) = budget.check() {
                        self.end_capture();
                        return Err(error);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.end_capture();
                    return Err(Error::Host("The capture stopped".into()));
                }
            }
        }
    }
    fn decode_frame(&self, method: &str, png: &[u8]) -> Result<Value> {
        let decoder = png::Decoder::new(std::io::Cursor::new(png));
        let mut reader = decoder
            .read_info()
            .map_err(|e| Error::Host(e.to_string()))?;
        let mut rgba = vec![
            0;
            reader
                .output_buffer_size()
                .ok_or(Error::Quota("render frame"))?
        ];
        let info = reader
            .next_frame(&mut rgba)
            .map_err(|e| Error::Host(e.to_string()))?;
        rgba.truncate(info.buffer_size());
        if info.bit_depth != png::BitDepth::Eight {
            return Err(Error::Host("engine frame is not 8-bit".into()));
        }
        let rgba = match info.color_type {
            png::ColorType::Rgba => rgba,
            // An opaque frame: RGB, alpha 255.
            png::ColorType::Rgb => rgba
                .chunks_exact(3)
                .flat_map(|px| [px[0], px[1], px[2], 255])
                .collect(),
            _ => return Err(Error::Host("engine frame is not RGB".into())),
        };
        let pixels = ImageData::from_rgba(info.width, info.height, rgba).map_err(host_error)?;
        if method == "render.snapshot" {
            return Ok(json!(format!(
                "data:image/jpeg;base64,{}",
                base64::engine::general_purpose::STANDARD
                    .encode(crate::image::jpeg(&pixels, 0.74)?)
            )));
        }
        let mut state = self.state.lock().unwrap();
        state.next += 1;
        let id = state.next;
        let mut canvas = Canvas::new(id, info.width, info.height).map_err(host_error)?;
        canvas
            .put_image_data(pixels, 0, 0, None)
            .map_err(host_error)?;
        state.surfaces.insert(id.to_string(), canvas);
        Ok(
            json!({"kind":"recorder-surface","surface":id.to_string(),"width":info.width,"height":info.height}),
        )
    }
    pub fn set_text(&self, text: CanvasText) {
        *self.text.lock().unwrap() = Some(text);
    }
    /// The realm's first publication, built outside JavaScript time.
    pub fn stage(&self) -> Result<()> {
        self.kernel.lock().unwrap().stage()
    }
    pub fn transport(&self) -> &crate::transport::Transport {
        &self.transport
    }
    /// Reply form of an external closure a native undo asked this realm to run.
    pub fn continuation(&self, continuation: &pm_kernel::Continuation) -> Result<Value> {
        self.kernel.lock().unwrap().continuation(continuation)
    }
    pub fn handle_name(&self, handle: u64) -> Option<String> {
        self.kernel.lock().unwrap().handle_name(handle)
    }
    /// This realm is finished; see `KernelAdapter::seal`.
    pub fn seal(&self) {
        self.kernel.lock().unwrap().seal();
    }
    pub fn diagnostics(&self) -> Value {
        let state = self.state.lock().unwrap();
        json!({"missing":state.missing,"surfaces":state.surfaces.len(),"pins":state.pins.len(),"dom_pending":state.dom.len()})
    }
    pub fn take_dom_batches(&self) -> Vec<Value> {
        std::mem::take(&mut self.state.lock().unwrap().dom)
    }
    pub fn canvas_frames(&self) -> Result<Vec<pm_compat_host::CanvasFrame>> {
        self.changed_canvas_frames(&BTreeMap::new())
    }
    /// Bound canvases whose version differs from `known`. Pixels are read under
    /// the lock V8's recorder calls need; only those that changed, and the PNG
    /// is encoded after it is released.
    pub fn changed_canvas_frames(
        &self,
        known: &BTreeMap<(u64, String), u64>,
    ) -> Result<Vec<pm_compat_host::CanvasFrame>> {
        let mut state = self.state.lock().unwrap();
        let bindings = state.bindings.clone();
        let sequence = state.dom_sequence;
        if sequence == 0 && !bindings.is_empty() {
            return Err(Error::Invalid("canvas DOM batch is not flushed"));
        }
        let mut changed = vec![];
        let mut images = vec![];
        for (node, id) in bindings {
            if let Some(image) = state.images.get(&id) {
                if known.get(&(node, id.clone())) != Some(&1) {
                    images.push(pm_compat_host::CanvasFrame {
                        generation: 1,
                        node,
                        dom_sequence: sequence,
                        surface: id,
                        version: 1,
                        width: image.width,
                        height: image.height,
                        png: image.bytes.clone(),
                        mime: image.mime.as_str().into(),
                    });
                }
                continue;
            }
            let canvas = state
                .surfaces
                .get_mut(&id)
                .ok_or(Error::Invalid("bound canvas retired"))?;
            let version = canvas.key().version;
            if known.get(&(node, id.clone())) == Some(&version) {
                continue;
            }
            let (width, height) = canvas.size();
            let source = if width == 0 || height == 0 {
                None
            } else {
                Some(canvas.snapshot().map_err(host_error)?)
            };
            changed.push((node, id, version, width, height, source));
        }
        drop(state);
        changed
            .into_iter()
            .map(|(node, id, version, width, height, source)| {
                let (version, png) = match source {
                    Some(source) => (source.key.version, encode_png(&source.pixels)?),
                    None => (version, Vec::new()),
                };
                Ok(pm_compat_host::CanvasFrame {
                    generation: 1,
                    node,
                    dom_sequence: sequence,
                    surface: id,
                    version,
                    width,
                    height,
                    png: png.into(),
                    mime: "image/png".into(),
                })
            })
            .chain(images.into_iter().map(Ok))
            .collect()
    }
    fn recorder(&self, operation: &str, input: &Value) -> Result<Value> {
        let args = input["args"]
            .as_array()
            .ok_or(Error::Invalid("recorder argument array"))?;
        let mut state = self.state.lock().unwrap();
        if operation == "recorder.create" {
            let (w, h) = (dimension(args.first())?, dimension(args.get(1))?);
            state.next += 1;
            let id = state.next;
            let key = id.to_string();
            state
                .surfaces
                .insert(key.clone(), Canvas::new(id, w, h).map_err(host_error)?);
            return Ok(json!(key));
        }
        if operation == "recorder.releaseVersion" {
            let version = args.first().ok_or(Error::Invalid("source version"))?;
            let id = version["surface"]
                .as_str()
                .ok_or(Error::Invalid("source id"))?
                .to_string();
            let version = version["version"]
                .as_u64()
                .ok_or(Error::Invalid("source version"))?;
            let entry = state
                .pins
                .get_mut(&(id.clone(), version))
                .ok_or(Error::Invalid("unretained source"))?;
            entry.1 -= 1;
            if entry.1 == 0 {
                state.pins.remove(&(id.clone(), version));
                if let Some((charge, ..)) = state.rasters.remove(&(id, version)) {
                    state.image_bytes = state.image_bytes.saturating_sub(charge);
                }
            }
            return Ok(Value::Null);
        }
        let id = args
            .first()
            .and_then(Value::as_str)
            .ok_or(Error::Invalid("surface id"))?
            .to_string();
        if operation == "recorder.release" && id.starts_with(['i', 's']) {
            let image = state
                .images
                .remove(&id)
                .ok_or(Error::Invalid("unknown image"))?;
            state.image_bytes = state.image_bytes.saturating_sub(image.cost);
            return Ok(Value::Null);
        }
        if operation == "recorder.retainVersion" && state.images.contains_key(&id) {
            let State {
                images,
                pins,
                rasters,
                image_bytes,
                ..
            } = &mut *state;
            let image = &images[&id];
            // A vector image rasterizes at the scale it is drawn, in quarter steps.
            let version = if image.is_vector() {
                (args
                    .get(1)
                    .and_then(Value::as_f64)
                    .unwrap_or(1.)
                    .clamp(0.25, 64.)
                    * SVG_STEPS)
                    .ceil() as u64
            } else {
                1
            };
            if let Some(pin) = pins.get_mut(&(id.clone(), version)) {
                pin.1 += 1;
            } else {
                let size = image.raster_size(version as f64 / SVG_STEPS);
                // A new SVG raster is admitted before it is allocated.
                let charge = match size {
                    Some((w, h)) => crate::image::rgba_bytes(u64::from(w), u64::from(h))?,
                    None => 0,
                };
                let total = image_bytes
                    .checked_add(charge)
                    .filter(|total| *total <= IMAGE_BUDGET)
                    .ok_or(Error::Quota("image memory"))?;
                let pixels = image.pixels(size)?;
                *image_bytes = total;
                if let Some((w, h)) = size {
                    let (nw, nh) = image.natural();
                    rasters.insert(
                        (id.clone(), version),
                        (charge, f64::from(w) / nw, f64::from(h) / nh),
                    );
                }
                let number = id[id.find('-').map_or(0, |i| i + 1)..].parse().unwrap_or(0);
                let source = Source {
                    key: pm_recorder::SourceKey {
                        id: number,
                        version,
                    },
                    pixels,
                };
                pins.insert((id.clone(), version), (source, 1));
            }
            return Ok(json!({"surface":id,"version":version}));
        }
        if operation == "recorder.release" {
            state
                .surfaces
                .remove(&id)
                .ok_or(Error::Invalid("unknown surface"))?;
            return Ok(Value::Null);
        }
        if operation == "recorder.retainVersion" {
            let canvas = state
                .surfaces
                .get_mut(&id)
                .ok_or(Error::Invalid("unknown surface"))?;
            let source = canvas.snapshot().map_err(host_error)?;
            let version = source.key.version;
            state
                .pins
                .entry((id.clone(), version))
                .and_modify(|v| v.1 += 1)
                .or_insert((source, 1));
            return Ok(json!({"surface":id,"version":version}));
        }
        if operation == "recorder.reset" {
            let (w, h) = (dimension(args.get(1))?, dimension(args.get(2))?);
            state
                .surfaces
                .get_mut(&id)
                .ok_or(Error::Invalid("unknown surface"))?
                .resize(w, h)
                .map_err(host_error)?;
            return Ok(Value::Null);
        }
        let name = args
            .get(1)
            .and_then(Value::as_str)
            .ok_or(Error::Invalid("recorder method"))?;
        let values = args
            .get(2)
            .and_then(Value::as_array)
            .ok_or(Error::Invalid("recorder values"))?;
        let source = if name == "drawImage" {
            let key = values.first().ok_or(Error::Invalid("draw source"))?;
            Some(
                state
                    .pins
                    .get(&(
                        key["surface"]
                            .as_str()
                            .ok_or(Error::Invalid("draw source"))?
                            .into(),
                        key["version"]
                            .as_u64()
                            .ok_or(Error::Invalid("draw version"))?,
                    ))
                    .ok_or(Error::Invalid("unretained draw source"))?
                    .0
                    .clone(),
            )
        } else {
            None
        };
        let pattern = if name == "createPattern" {
            let key = values.first().ok_or(Error::Invalid("pattern source"))?;
            state
                .pins
                .get(&(
                    key["surface"].as_str().unwrap_or_default().into(),
                    key["version"].as_u64().unwrap_or(0),
                ))
                .map(|pin| pin.0.clone())
        } else {
            None
        };
        let State {
            surfaces,
            paints,
            next,
            rasters,
            ..
        } = &mut *state;
        let handle = |paints: &mut BTreeMap<String, Paint>, next: &mut u64, paint: Paint| {
            *next += 1;
            let key = format!("paint-{next}");
            paints.insert(key.clone(), paint);
            json!(key)
        };
        let canvas = surfaces
            .get_mut(&id)
            .ok_or(Error::Invalid("unknown surface"))?;
        let mut supported = true;
        let result = match (operation, name) {
            ("recorder.read", "get") => match values.first().and_then(Value::as_str) {
                Some("globalAlpha") => json!(canvas.state().alpha),
                Some("lineWidth") => json!(canvas.state().line.width),
                Some("miterLimit") => json!(canvas.state().line.miter_limit),
                Some("lineDashOffset") => json!(canvas.state().line.dash_offset),
                Some("imageSmoothingEnabled") => json!(canvas.state().image_smoothing),
                Some("lineCap") => json!(match canvas.state().line.start_cap {
                    Cap::Butt => "butt",
                    Cap::Round => "round",
                    Cap::Square => "square",
                }),
                Some("lineJoin") => json!(match canvas.state().line.join {
                    Join::Miter => "miter",
                    Join::Round => "round",
                    Join::Bevel => "bevel",
                }),
                _ => {
                    supported = false;
                    Value::Null
                }
            },
            // Hints a CPU recorder already meets (willReadFrequently,
            // desynchronized) and the default sRGB, alpha, unorm8 context.
            ("recorder.command", "context")
                if values.first().and_then(Value::as_object).is_some_and(|o| {
                    o.iter().all(|(key, value)| match key.as_str() {
                        "willReadFrequently" | "desynchronized" => true,
                        "alpha" => value.as_bool() != Some(false),
                        "colorSpace" => value == "srgb",
                        "colorType" => value == "unorm8",
                        _ => false,
                    })
                }) =>
            {
                Value::Null
            }
            ("recorder.command", "set") => {
                let property = values
                    .first()
                    .and_then(Value::as_str)
                    .ok_or(Error::Invalid("canvas property"))?;
                let paint = values
                    .get(1)
                    .and_then(|v| v["paint"].as_str())
                    .map(|handle| {
                        paints
                            .get(handle)
                            .cloned()
                            .ok_or(Error::Invalid("unknown canvas paint"))
                    })
                    .transpose()?;
                match property {
                    "fillStyle" => {
                        if let Some(paint) = paint {
                            canvas.set_fill(paint);
                        } else {
                            canvas
                                .set_fill_color(
                                    values
                                        .get(1)
                                        .and_then(Value::as_str)
                                        .ok_or(Error::Invalid("fill color"))?,
                                )
                                .map_err(host_error)?
                        }
                    }
                    "strokeStyle" => {
                        if let Some(paint) = paint {
                            canvas.set_stroke(paint);
                        } else {
                            canvas
                                .set_stroke_color(
                                    values
                                        .get(1)
                                        .and_then(Value::as_str)
                                        .ok_or(Error::Invalid("stroke color"))?,
                                )
                                .map_err(host_error)?
                        }
                    }
                    "font" => canvas
                        .set_font(
                            values
                                .get(1)
                                .and_then(Value::as_str)
                                .ok_or(Error::Invalid("canvas font"))?,
                        )
                        .map_err(host_error)?,
                    "textAlign" => match values.get(1).and_then(Value::as_str) {
                        Some("start") => canvas.set_text_align(TextAlign::Start),
                        Some("end") => canvas.set_text_align(TextAlign::End),
                        Some("left") => canvas.set_text_align(TextAlign::Left),
                        Some("right") => canvas.set_text_align(TextAlign::Right),
                        Some("center") => canvas.set_text_align(TextAlign::Center),
                        _ => (),
                    },
                    "textBaseline" => match values.get(1).and_then(Value::as_str) {
                        Some("top") => canvas.set_text_baseline(TextBaseline::Top),
                        Some("hanging") => canvas.set_text_baseline(TextBaseline::Hanging),
                        Some("middle") => canvas.set_text_baseline(TextBaseline::Middle),
                        Some("alphabetic") => canvas.set_text_baseline(TextBaseline::Alphabetic),
                        Some("ideographic") => canvas.set_text_baseline(TextBaseline::Ideographic),
                        Some("bottom") => canvas.set_text_baseline(TextBaseline::Bottom),
                        _ => (),
                    },
                    "direction" => match values.get(1).and_then(Value::as_str) {
                        Some("ltr") => canvas.set_direction(Direction::Ltr),
                        Some("rtl") => canvas.set_direction(Direction::Rtl),
                        _ => (),
                    },
                    "globalAlpha" => canvas.set_global_alpha(num(values, 1)?),
                    "lineWidth" => canvas.set_line_width(num(values, 1)?),
                    "miterLimit" => canvas.set_miter_limit(num(values, 1)?),
                    "lineDashOffset" => canvas.set_line_dash_offset(num(values, 1)?),
                    "imageSmoothingEnabled" => canvas.set_image_smoothing(
                        values
                            .get(1)
                            .and_then(Value::as_bool)
                            .ok_or(Error::Invalid("smoothing"))?,
                    ),
                    "lineCap" => match values.get(1).and_then(Value::as_str) {
                        Some("butt") => canvas.set_line_cap(Cap::Butt),
                        Some("round") => canvas.set_line_cap(Cap::Round),
                        Some("square") => canvas.set_line_cap(Cap::Square),
                        _ => (), // Canvas ignores invalid enumerated values.
                    },
                    "lineJoin" => match values.get(1).and_then(Value::as_str) {
                        Some("miter") => canvas.set_line_join(Join::Miter),
                        Some("round") => canvas.set_line_join(Join::Round),
                        Some("bevel") => canvas.set_line_join(Join::Bevel),
                        _ => (),
                    },
                    "globalCompositeOperation" => canvas.set_composite(
                        values
                            .get(1)
                            .and_then(Value::as_str)
                            .ok_or(Error::Invalid("composite"))?,
                    ),
                    _ => supported = false,
                }
                Value::Null
            }
            ("recorder.command", "save") => {
                canvas.save();
                Value::Null
            }
            ("recorder.command", "restore") => {
                canvas.restore();
                Value::Null
            }
            ("recorder.command", "reset") => {
                canvas.reset().map_err(host_error)?;
                Value::Null
            }
            ("recorder.command", "beginPath") => {
                canvas.begin_path();
                Value::Null
            }
            ("recorder.command", "closePath") => {
                canvas.close_path();
                Value::Null
            }
            ("recorder.command", "moveTo") => {
                canvas.move_to(num(values, 0)?, num(values, 1)?);
                Value::Null
            }
            ("recorder.command", "lineTo") => {
                canvas.line_to(num(values, 0)?, num(values, 1)?);
                Value::Null
            }
            ("recorder.command", "quadraticCurveTo") => {
                canvas.quadratic_curve_to(
                    Point::new(num(values, 0)?, num(values, 1)?),
                    Point::new(num(values, 2)?, num(values, 3)?),
                );
                Value::Null
            }
            ("recorder.command", "bezierCurveTo") => {
                canvas.bezier_curve_to(
                    Point::new(num(values, 0)?, num(values, 1)?),
                    Point::new(num(values, 2)?, num(values, 3)?),
                    Point::new(num(values, 4)?, num(values, 5)?),
                );
                Value::Null
            }
            ("recorder.command", "arc") => {
                canvas
                    .arc(
                        Point::new(num(values, 0)?, num(values, 1)?),
                        num(values, 2)?,
                        num(values, 3)?,
                        num(values, 4)?,
                        boolean(values.get(5)),
                    )
                    .map_err(host_error)?;
                Value::Null
            }
            ("recorder.command", "arcTo") => {
                canvas
                    .arc_to(
                        Point::new(num(values, 0)?, num(values, 1)?),
                        Point::new(num(values, 2)?, num(values, 3)?),
                        num(values, 4)?,
                    )
                    .map_err(host_error)?;
                Value::Null
            }
            ("recorder.command", "ellipse") => {
                canvas
                    .ellipse(
                        Point::new(num(values, 0)?, num(values, 1)?),
                        Vec2::new(num(values, 2)?, num(values, 3)?),
                        num(values, 4)?,
                        num(values, 5)?,
                        num(values, 6)?,
                        boolean(values.get(7)),
                    )
                    .map_err(host_error)?;
                Value::Null
            }
            ("recorder.command", "setTransform" | "transform") => {
                let transform = matrix(values)?;
                if name == "setTransform" {
                    canvas.set_transform(transform)
                } else {
                    canvas.transform(transform)
                }
                Value::Null
            }
            ("recorder.command", "setLineDash") => {
                canvas.set_line_dash(&numbers(
                    values.first().ok_or(Error::Invalid("dash array"))?,
                )?);
                Value::Null
            }
            ("recorder.read", "getLineDash") => json!(canvas.state().line.dash_pattern.as_slice()),
            ("recorder.read", "getTransform") => {
                let [a, b, c, d, e, f] = canvas.state().transform.as_coeffs();
                json!({"a":a,"b":b,"c":c,"d":d,"e":e,"f":f,"is2D":true})
            }
            ("recorder.read", "isPointInPath" | "isPointInStroke") => {
                let point = Point::new(num(values, 0)?, num(values, 1)?);
                json!(if name == "isPointInPath" {
                    canvas.is_point_in_path(point, fill_rule(values.get(2))?)
                } else {
                    canvas.is_point_in_stroke(point)
                })
            }
            ("recorder.read", "toDataURL")
                if values
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or("image/png")
                    == "image/png" =>
            {
                let (width, height) = canvas.size();
                if width == 0 || height == 0 {
                    json!("data:,")
                } else {
                    let source = canvas.snapshot().map_err(host_error)?;
                    json!(format!(
                        "data:image/png;base64,{}",
                        base64::engine::general_purpose::STANDARD
                            .encode(encode_png(&source.pixels)?)
                    ))
                }
            }
            ("recorder.command", "translate") => {
                canvas.translate(num(values, 0)?, num(values, 1)?);
                Value::Null
            }
            ("recorder.command", "scale") => {
                canvas.scale(num(values, 0)?, num(values, 1)?);
                Value::Null
            }
            ("recorder.command", "rotate") => {
                canvas.rotate(num(values, 0)?);
                Value::Null
            }
            ("recorder.command", "resetTransform") => {
                canvas.reset_transform();
                Value::Null
            }
            ("recorder.command", "rect") => {
                canvas.rect(
                    num(values, 0)?,
                    num(values, 1)?,
                    num(values, 2)?,
                    num(values, 3)?,
                );
                Value::Null
            }
            ("recorder.command", "roundRect") => {
                // Radii: a number, a DOMPointInit, or a list of 1–4 of them
                // (omitted: 0), as the HTML spec reads them.
                let corner = |value: &Value| -> Result<Vec2> {
                    match value {
                        Value::Number(n) => {
                            let r = n.as_f64().unwrap_or(f64::NAN);
                            Ok(Vec2::new(r, r))
                        }
                        Value::Object(point) => {
                            let axis = |key| point.get(key).and_then(Value::as_f64).unwrap_or(0.);
                            Ok(Vec2::new(axis("x"), axis("y")))
                        }
                        _ => Err(Error::Invalid("roundRect radius")),
                    }
                };
                let radii = match values.get(4) {
                    None | Some(Value::Null) => vec![Vec2::ZERO],
                    Some(Value::Array(list)) => list.iter().map(corner).collect::<Result<_>>()?,
                    Some(value) => vec![corner(value)?],
                };
                canvas
                    .round_rect(
                        num(values, 0)?,
                        num(values, 1)?,
                        num(values, 2)?,
                        num(values, 3)?,
                        &radii,
                    )
                    .map_err(host_error)?;
                Value::Null
            }
            ("recorder.command", "fillRect" | "strokeRect" | "clearRect") => {
                let (x, y, w, h) = (
                    num(values, 0)?,
                    num(values, 1)?,
                    num(values, 2)?,
                    num(values, 3)?,
                );
                match name {
                    "fillRect" => canvas.fill_rect(x, y, w, h),
                    "strokeRect" => canvas.stroke_rect(x, y, w, h),
                    _ => canvas.clear_rect(x, y, w, h),
                }
                .map_err(host_error)?;
                Value::Null
            }
            ("recorder.command", "fill" | "stroke" | "clip")
                if values.len() <= 1 && values.first().is_none_or(Value::is_string) =>
            {
                let rule = fill_rule(values.first())?;
                match name {
                    "fill" => canvas.fill(rule),
                    "stroke" => canvas.stroke(),
                    _ => {
                        canvas.clip(rule);
                        Ok(())
                    }
                }
                .map_err(host_error)?;
                Value::Null
            }
            ("recorder.command", "drawImage") => {
                let source = source.unwrap();
                // An SVG raster's scale from intrinsic size, per axis (it may be
                // capped); the caller's coordinates are intrinsic, as in a browser.
                let (fx, fy) = values[0]["surface"]
                    .as_str()
                    .and_then(|id| rasters.get(&(id.to_owned(), source.key.version)))
                    .map_or((1., 1.), |(_, fx, fy)| (*fx, *fy));
                let (sw, sh) = (
                    source.pixels.width() as f64 / fx,
                    source.pixels.height() as f64 / fy,
                );
                let (src, dst) = match values.len() {
                    3 => (
                        Rect::new(0., 0., sw, sh),
                        Rect::from_origin_size(
                            Point::new(num(values, 1)?, num(values, 2)?),
                            (sw, sh),
                        ),
                    ),
                    5 => (
                        Rect::new(0., 0., sw, sh),
                        Rect::from_origin_size(
                            Point::new(num(values, 1)?, num(values, 2)?),
                            (num(values, 3)?, num(values, 4)?),
                        ),
                    ),
                    9 => (
                        Rect::from_origin_size(
                            Point::new(num(values, 1)?, num(values, 2)?),
                            (num(values, 3)?, num(values, 4)?),
                        ),
                        Rect::from_origin_size(
                            Point::new(num(values, 5)?, num(values, 6)?),
                            (num(values, 7)?, num(values, 8)?),
                        ),
                    ),
                    _ => return Err(Error::Invalid("drawImage arity")),
                };
                let src = Rect::new(src.x0 * fx, src.y0 * fy, src.x1 * fx, src.y1 * fy);
                canvas.draw_image(source, src, dst).map_err(host_error)?;
                Value::Null
            }
            ("recorder.read", "createLinearGradient") => {
                let gradient = Gradient::new(GradientKind::Linear(
                    Point::new(num(values, 0)?, num(values, 1)?),
                    Point::new(num(values, 2)?, num(values, 3)?),
                ))
                .map_err(host_error)?;
                handle(paints, next, Paint::Gradient(gradient))
            }
            ("recorder.read", "createRadialGradient") => {
                let gradient = Gradient::new(GradientKind::Radial {
                    start: Point::new(num(values, 0)?, num(values, 1)?),
                    r0: num(values, 2)?,
                    end: Point::new(num(values, 3)?, num(values, 4)?),
                    r1: num(values, 5)?,
                })
                .map_err(host_error)?;
                handle(paints, next, Paint::Gradient(gradient))
            }
            ("recorder.read", "createConicGradient") => {
                let gradient = Gradient::new(GradientKind::Conic {
                    angle: num(values, 0)?,
                    center: Point::new(num(values, 1)?, num(values, 2)?),
                })
                .map_err(host_error)?;
                handle(paints, next, Paint::Gradient(gradient))
            }
            ("recorder.read", "createPattern") => {
                let source = pattern.ok_or(Error::Invalid("pattern source"))?;
                let repetition = match values.get(1).and_then(Value::as_str) {
                    Some("repeat-x") => Repetition::RepeatX,
                    Some("repeat-y") => Repetition::RepeatY,
                    Some("no-repeat") => Repetition::NoRepeat,
                    _ => Repetition::Repeat,
                };
                handle(
                    paints,
                    next,
                    Paint::Pattern(Pattern::new(source, repetition)),
                )
            }
            ("recorder.command", "paint.addColorStop") => {
                let Some(Paint::Gradient(gradient)) = values
                    .first()
                    .and_then(Value::as_str)
                    .and_then(|h| paints.get(h))
                else {
                    return Err(Error::Invalid("gradient handle"));
                };
                gradient
                    .add_color_stop(
                        num(values, 1)?,
                        values
                            .get(2)
                            .and_then(Value::as_str)
                            .ok_or(Error::Invalid("stop color"))?,
                    )
                    .map_err(host_error)?;
                Value::Null
            }
            ("recorder.command", "putImageData") => {
                let data = &values[0];
                let (width, height) = (
                    dimension(data.get("width"))?,
                    dimension(data.get("height"))?,
                );
                let bytes: Vec<u8> = data["data"]
                    .as_array()
                    .ok_or(Error::Invalid("image data"))?
                    .iter()
                    .map(|v| v.as_u64().unwrap_or(0).min(255) as u8)
                    .collect();
                let image = ImageData::from_rgba(width, height, bytes).map_err(host_error)?;
                let dirty = (values.len() == 7)
                    .then(|| [3, 4, 5, 6].map(|i| values[i].as_i64().unwrap_or(0) as i32));
                canvas
                    .put_image_data(image, num(values, 1)? as i32, num(values, 2)? as i32, dirty)
                    .map_err(host_error)?;
                Value::Null
            }
            ("recorder.read", "createImageData") => {
                let (width, height) = (num(values, 0)? as u32, num(values, 1)? as u32);
                json!({"width":width,"height":height,"data":vec![0; width as usize * height as usize * 4]})
            }
            ("recorder.read", "measureText") => {
                let text: Vec<u16> = values
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .encode_utf16()
                    .collect();
                let mut provider = self.text.lock().unwrap();
                let provider = provider
                    .as_mut()
                    .ok_or_else(|| Error::Host("canvas text backend not attached".into()))?;
                let metrics = canvas
                    .measure_text(&text, &mut **provider)
                    .map_err(host_error)?;
                json!({"width":metrics.width,"actualBoundingBoxLeft":metrics.actual_bounding_box_left,
                    "actualBoundingBoxRight":metrics.actual_bounding_box_right,
                    "actualBoundingBoxAscent":metrics.actual_bounding_box_ascent,
                    "actualBoundingBoxDescent":metrics.actual_bounding_box_descent,
                    "fontBoundingBoxAscent":metrics.font_bounding_box_ascent,
                    "fontBoundingBoxDescent":metrics.font_bounding_box_descent})
            }
            ("recorder.command", "fillText" | "strokeText") => {
                let text: Vec<u16> = values
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .encode_utf16()
                    .collect();
                let (x, y) = (num(values, 1)?, num(values, 2)?);
                let width = values.get(3).and_then(Value::as_f64);
                let mut provider = self.text.lock().unwrap();
                let provider = provider
                    .as_mut()
                    .ok_or_else(|| Error::Host("canvas text backend not attached".into()))?;
                if name == "fillText" {
                    canvas.fill_text(&text, x, y, width, &mut **provider)
                } else {
                    canvas.stroke_text(&text, x, y, width, &mut **provider)
                }
                .map_err(host_error)?;
                Value::Null
            }
            ("recorder.read", "getImageData") => {
                let mut coordinates = [0_i32; 4];
                for (index, coordinate) in coordinates.iter_mut().enumerate() {
                    let n = num(values, index)?;
                    if n.fract() != 0. || n < i32::MIN as f64 || n > i32::MAX as f64 {
                        return Err(Error::Invalid("integer read rectangle"));
                    }
                    *coordinate = n as i32;
                }
                let [x, y, w, h] = coordinates;
                let image = canvas.get_image_data(x, y, w, h).map_err(host_error)?;
                json!({"width":image.width(),"height":image.height(),"data":image.bytes()})
            }
            _ => {
                supported = false;
                Value::Null
            }
        };
        if !supported {
            state.missing.insert(format!("{operation}.{name}"));
            return Err(Error::Host(
                "recorder operation backend not attached".into(),
            ));
        }
        Ok(result)
    }
}
impl EngineProxy for NativeHost {
    fn call_bytes(
        &self,
        _: &Generation,
        operation: &str,
        bytes: &[u8],
        input: &Value,
        budget: HostBudget,
    ) -> Result<Value> {
        budget.check()?;
        match operation {
            "image.load" => {
                let mut reserved = 0;
                // Reserved before decoding allocates; refused past the budget.
                let image = crate::image::Encoded::load(
                    bytes,
                    input["type"].as_str().unwrap_or(""),
                    |cost| {
                        let mut state = self.state.lock().unwrap();
                        state.image_bytes = state
                            .image_bytes
                            .checked_add(cost)
                            .filter(|total| *total <= IMAGE_BUDGET)
                            .ok_or(Error::Quota("image memory"))?;
                        reserved = cost;
                        Ok(())
                    },
                );
                let mut state = self.state.lock().unwrap();
                let image = match image {
                    Ok(image) => image,
                    Err(error) => {
                        // A failed decode returns its reservation.
                        state.image_bytes = state.image_bytes.saturating_sub(reserved);
                        return Err(error);
                    }
                };
                state.next += 1;
                let kind = if image.is_vector() { "svg" } else { "image" };
                let id = format!("{kind}-{}", state.next);
                let reply = json!({"surface":id,"width":image.width,"height":image.height,"type":image.mime});
                state.images.insert(id, image);
                Ok(reply)
            }
            "assets.stage" => {
                let token = crate::files::stage(bytes, input["name"].as_str().unwrap_or(""))?;
                Ok(json!({"token":token.to_string()}))
            }
            _ => Err(Error::Host(format!("{operation} is unavailable"))),
        }
    }
    fn read_bytes(
        &self,
        _: &Generation,
        operation: &str,
        input: &Value,
        budget: HostBudget,
    ) -> Result<Vec<u8>> {
        budget.check()?;
        match operation {
            "file.read" => {
                let token = input["token"]
                    .as_str()
                    .and_then(|t| t.parse().ok())
                    .ok_or(Error::Invalid("file token"))?;
                crate::files::read(token, budget.max_response_bytes)
            }
            _ => Err(Error::Host(format!("{operation} is unavailable"))),
        }
    }
    fn call_text(
        &self,
        generation: &Generation,
        operation: &str,
        input: &Value,
        budget: HostBudget,
    ) -> Result<String> {
        if operation == "kernel.publication"
            && let Some(staged) = self.kernel.lock().unwrap().take_staged()
        {
            return Ok(staged);
        }
        self.call(generation, operation, input, budget)
            .map(|value| value.to_string())
    }
    fn call(
        &self,
        _: &Generation,
        operation: &str,
        input: &Value,
        budget: HostBudget,
    ) -> Result<Value> {
        budget.check()?;
        match operation {
            // `<video>`: a held or staged file opened for its frames.
            "video.open" => {
                let token = input["token"]
                    .as_str()
                    .and_then(|t| t.parse().ok())
                    .ok_or(Error::Invalid("file token"))?;
                let path = crate::files::path(token)
                    .ok_or_else(|| Error::Host("The video's file is gone.".into()))?;
                let video = crate::video::Video::open(&path)?;
                let mut state = self.state.lock().unwrap();
                state.next += 1;
                let id = format!("video-{}", state.next);
                let reply = json!({"video": id, "duration": video.duration,
                    "width": video.width, "height": video.height});
                state.videos.insert(id, video);
                return Ok(reply);
            }
            // Its frame at a time, as an image the realm draws like any other.
            "video.frame" => {
                let id = input["video"].as_str().unwrap_or_default();
                let mut state = self.state.lock().unwrap();
                let video = state
                    .videos
                    .get(id)
                    .ok_or(Error::Invalid("unknown video"))?;
                let pixels = video.frame(input["time"].as_f64().unwrap_or(0.))?;
                let image = crate::image::Encoded::frame(pixels);
                state.image_bytes = state
                    .image_bytes
                    .checked_add(image.cost)
                    .filter(|total| *total <= IMAGE_BUDGET)
                    .ok_or(Error::Quota("image memory"))?;
                state.next += 1;
                let id = format!("image-{}", state.next);
                let reply = json!({"surface": id, "width": image.width, "height": image.height});
                state.images.insert(id, image);
                return Ok(reply);
            }
            "video.close" => {
                let id = input["video"].as_str().unwrap_or_default();
                self.state.lock().unwrap().videos.remove(id);
                return Ok(Value::Null);
            }
            "file.release" => {
                // The realm collected the last File for this picked token.
                if let Some(token) = input["token"].as_str().and_then(|t| t.parse().ok()) {
                    crate::files::release(token);
                }
                return Ok(Value::Null);
            }
            "transport.command" => {
                let base = self.kernel.lock().unwrap().timebase()?;
                return self.transport.command(input, base);
            }
            "kernel.call" => {
                let time = self.transport.time();
                return self.kernel.lock().unwrap().call(input, time);
            }
            "kernel.publication" => return self.kernel.lock().unwrap().publication(),
            "definitions.builtin" => return Ok(self.kernel.lock().unwrap().builtin_definitions()),
            "kernel.query"
                if matches!(
                    input["method"].as_str(),
                    Some("render.renderFrameTo" | "render.snapshot")
                ) =>
            {
                let method = input["method"].as_str().unwrap_or_default();
                return self.render_query(method, &input["args"]["args"], &budget);
            }
            "kernel.query" if input["method"] == "render.continue" => {
                return self.wait_capture(&budget);
            }
            "kernel.query" if input["method"] == "render.raster" => {
                return self
                    .base_raster(&input["args"]["args"], input["args"]["revision"].as_u64());
            }
            "kernel.query" if input["method"] == "render.rasterPin" => {
                return Ok(self.pin().map_or(Value::Null, |revision| json!(revision)));
            }
            "kernel.query" if input["method"] == "render.rasterUnpin" => {
                if let Some(revision) = input["args"]["revision"].as_u64() {
                    let mut pins = self.pins.lock().unwrap();
                    if let Some(pin) = pins.get_mut(&revision) {
                        pin.2 -= 1;
                        if pin.2 == 0 {
                            pins.remove(&revision);
                        }
                    }
                }
                return Ok(Value::Null);
            }
            "kernel.query" => return self.kernel.lock().unwrap().query(input),
            "compat.producing" => {
                if let Some(hooks) = self.rasters.lock().unwrap().clone() {
                    (hooks.producing)(
                        input["on"] == true,
                        input["epoch"].as_u64().unwrap_or(0),
                        self.code.load(std::sync::atomic::Ordering::Acquire),
                    );
                }
                return Ok(Value::Null);
            }
            "compat.answers" => return self.raster_answers(input),
            "fonts.families" => return Ok(json!(crate::font_families())),
            "clipboard.text" => {
                let clipboard = self.clipboard.lock().unwrap().clone().ok_or_else(|| {
                    Error::Host("The clipboard is not available to this extension here".into())
                })?;
                let text = clipboard(input["write"].as_str()).map_err(Error::Host)?;
                return Ok(match input.get("write") {
                    Some(_) => Value::Null,
                    None => Value::String(text),
                });
            }
            "effects.clipboard" => {
                let slot = self.effects.lock().unwrap().clone();
                let hook = slot.and_then(|slot| slot.lock().unwrap().clone());
                let hook = hook.ok_or_else(|| {
                    Error::Host("The effect clipboard belongs to the window's panels".into())
                })?;
                return hook(input["op"].as_str().unwrap_or_default()).map_err(Error::Host);
            }
            // The staged values, to whoever holds their token, once. Nothing
            // the caller says selects an extension or a key.
            "extension.vars" => {
                let mut staged = self.vars.lock().unwrap();
                let held = input["token"]
                    .as_str()
                    .and_then(|token| staged.iter().position(|(t, _)| t == token));
                return Ok(held.map_or_else(|| json!({}), |index| staged.remove(index).1));
            }
            "extension.health" => {
                self.events
                    .health(input["id"].as_str().unwrap_or_default(), &input["health"]);
                return Ok(Value::Null);
            }
            "ui.toast" => {
                self.events.toast(
                    input["text"].as_str().unwrap_or_default(),
                    &input["options"],
                );
                return Ok(Value::Null);
            }
            "ui.request" => {
                self.events.request(input.clone());
                return Ok(Value::Null);
            }
            "storage.set" => {
                let key = input["key"].as_str().ok_or(Error::Invalid("storage key"))?;
                self.kernel.lock().unwrap().store(key, &input["value"])?;
                return Ok(Value::Null);
            }
            "extension.enabled" => {
                self.events.set_enabled(
                    input["id"].as_str().unwrap_or_default(),
                    input["enabled"] == true,
                );
                return Ok(Value::Null);
            }
            "extensions.ready" => {
                self.events.ready();
                return Ok(Value::Null);
            }
            "extension.activating" => {
                self.events.activating(input["id"].as_str());
                return Ok(Value::Null);
            }
            "contributions.publish" => {
                self.events.contributions(input["value"].clone());
                return Ok(Value::Null);
            }
            "kernel.pin" | "kernel.unpin" => {
                let handle = input["handle"]
                    .as_str()
                    .ok_or(Error::Invalid("generation pin"))?;
                self.kernel
                    .lock()
                    .unwrap()
                    .pin(handle, operation == "kernel.pin");
                return Ok(Value::Null);
            }
            _ => (),
        }
        if operation == "dom.measureSync" {
            let request = input["request"]
                .as_u64()
                .ok_or(Error::Invalid("measure request"))?;
            let answering = input["answering"] == true;
            return Ok(self.events.measure_sync(request, answering).into_json());
        }
        if operation == "dom.batch" {
            let mut state = self.state.lock().unwrap();
            if state.dom.len() >= 64 {
                return Ok(json!(false));
            }
            state.dom_sequence = input["batch"]["seq"]
                .as_u64()
                .ok_or(Error::Invalid("DOM sequence"))?;
            state.dom.push(input["batch"].clone());
            drop(state);
            self.events.dom_ready();
            return Ok(json!(true));
        }
        if operation == "recorder.bind" {
            let mut state = self.state.lock().unwrap();
            let inputs = input["bindings"]
                .as_array()
                .filter(|items| items.len() <= 4096)
                .ok_or(Error::Invalid("canvas bindings"))?;
            let mut bindings = BTreeMap::new();
            for input in inputs {
                let node = input["node"]
                    .as_u64()
                    .filter(|id| *id > 5 && *id < (1 << 53))
                    .ok_or(Error::Invalid("canvas DOM id"))?;
                let id = input["surface"]
                    .as_str()
                    .ok_or(Error::Invalid("canvas surface"))?;
                if !(state.surfaces.contains_key(id) || state.images.contains_key(id))
                    || bindings.insert(node, id.into()).is_some()
                {
                    return Err(Error::Invalid("canvas binding conflict"));
                }
            }
            state.bindings = bindings;
            drop(state);
            // Bound pixels are read after this flush, by the owner, not in here.
            self.events.canvases_ready();
            return Ok(Value::Null);
        }
        if operation == "recorder.command" {
            // Canvas strokes change presented pixels without a DOM mutation.
            self.events.dom_ready();
        }
        if operation.starts_with("recorder.") {
            return self.recorder(operation, input);
        }
        self.state.lock().unwrap().missing.insert(operation.into());
        Err(Error::Host("compat backend not attached".into()))
    }
}
impl KernelHooks for NativeHost {
    fn load_storage(&self, _: &str) -> Result<Value> {
        Ok(self.kernel.lock().unwrap().storage())
    }
    fn commit_activation(
        &self,
        _: Option<&Generation>,
        _: &Generation,
        ops: &[Value],
        _: &Value,
    ) -> Result<()> {
        if !ops.is_empty() {
            return Err(Error::Host("kernel activation backend not attached".into()));
        }
        Ok(())
    }
    fn commit_retained(&self, _: &Generation, _: &[Value]) -> Result<()> {
        Err(Error::Host("kernel history backend not attached".into()))
    }
    fn revoke_generation(&self, _: &Generation) {}
    fn seal_open_gestures(&self, _: &Generation) {}
}
impl HostProxy for NativeHost {
    /// Local and trusted extensions hold no network restriction, as in the shipped app.
    /// An export's realm (a delivery) reads but never changes anything outside
    /// it: the window's realm already did whatever its extensions send, and an
    /// export must not do it again for every frame.
    fn fetch(
        &self,
        _: &Generation,
        request: FetchRequest,
        cancel: Cancellation,
    ) -> Result<Box<dyn HostRequest>> {
        if self.kernel.lock().unwrap().is_frozen()
            && !matches!(request.method.to_ascii_uppercase().as_str(), "GET" | "HEAD")
        {
            return Err(Error::Host(format!(
                "An export's extensions only read the network ({} refused while exporting)",
                request.method
            )));
        }
        crate::net::fetch(request, crate::net::Reach::Open, cancel)
    }
    fn console(&self, _: &Generation, _: &str, _: &str) {}
}
impl WatchdogHooks for NativeHost {
    fn notice(&self, _: &Generation, _: u64) {}
    /// Eight seconds on one task: ask whether to stop it (D11). A trusted
    /// extension is never stopped without the user saying so.
    fn prompt(&self, _: &Generation, execution: u64) -> WatchdogDecision {
        self.events.unresponsive(execution);
        WatchdogDecision::Wait
    }
}
