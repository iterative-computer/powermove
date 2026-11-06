use base64::Engine;
use pm_recorder::{Affine, Canvas, Cap, FillRule, ImageData, Join, Point, Rect, Source, Vec2};
use pm_runtime_v8::*;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
};

#[derive(Default)]
struct State {
    storage: Value,
    next: u64,
    surfaces: BTreeMap<String, Canvas>,
    pins: BTreeMap<(String, u64), (Source, usize)>,
    missing: BTreeSet<String>,
    dom: Vec<Value>,
    invalidated: bool,
    bindings: BTreeMap<u64, String>,
    dom_sequence: u64,
}
/// Everything a host owner hears from its realm, without waiting on it.
pub trait RealmEvents: Send + Sync + 'static {
    fn health(&self, _id: &str, _health: &Value) {}
    fn toast(&self, _text: &str, _options: &Value) {}
    fn contributions(&self, _value: Value) {}
    fn dom_ready(&self) {}
    fn presentation(&self) {}
    fn ready(&self) {}
    fn activating(&self, _id: Option<&str>) {}
    fn watchdog_stop(&self) {}
    fn panel(&self, _request: &Value) -> Result<Value> {
        Err(Error::Host("panel host not attached".into()))
    }
}
pub struct NoEvents;
impl RealmEvents for NoEvents {}
pub struct NativeHost {
    state: Mutex<State>,
    kernel: Mutex<crate::adapter::KernelAdapter>,
    transport: crate::transport::Transport,
    events: Box<dyn RealmEvents>,
}
fn host_error(error: pm_recorder::Error) -> Error {
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
            transport,
            events,
        }
    }
    pub fn snapshot(&self) -> Result<Value> {
        self.kernel.lock().unwrap().snapshot()
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
    pub fn take_presentation_request(&self) -> bool {
        std::mem::take(&mut self.state.lock().unwrap().invalidated)
    }
    pub fn diagnostics(&self) -> Value {
        let state = self.state.lock().unwrap();
        json!({"missing":state.missing,"surfaces":state.surfaces.len(),"pins":state.pins.len(),"dom_pending":state.dom.len()})
    }
    pub fn take_dom_batches(&self) -> Vec<Value> {
        std::mem::take(&mut self.state.lock().unwrap().dom)
    }
    pub fn canvas_frames(&self) -> Result<Vec<pm_compat_host::CanvasFrame>> {
        let mut state = self.state.lock().unwrap();
        let bindings = state.bindings.clone();
        let sequence = state.dom_sequence;
        if sequence == 0 && !bindings.is_empty() {
            return Err(Error::Invalid("canvas DOM batch is not flushed"));
        }
        bindings
            .into_iter()
            .map(|(node, id)| {
                let canvas = state
                    .surfaces
                    .get_mut(&id)
                    .ok_or(Error::Invalid("bound canvas retired"))?;
                let (width, height) = canvas.size();
                let (version, png) = if width == 0 || height == 0 {
                    (canvas.key().version, Vec::new())
                } else {
                    let source = canvas.snapshot().map_err(host_error)?;
                    (source.key.version, encode_png(&source.pixels)?)
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
                })
            })
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
                state.pins.remove(&(id, version));
            }
            return Ok(Value::Null);
        }
        let id = args
            .first()
            .and_then(Value::as_str)
            .ok_or(Error::Invalid("surface id"))?
            .to_string();
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
        let canvas = state
            .surfaces
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
            ("recorder.command", "context")
                if values
                    .first()
                    .is_some_and(|v| v.as_object().is_some_and(|v| v.is_empty())) =>
            {
                Value::Null
            }
            ("recorder.command", "set") => {
                let property = values
                    .first()
                    .and_then(Value::as_str)
                    .ok_or(Error::Invalid("canvas property"))?;
                match property {
                    "fillStyle" => canvas
                        .set_fill_color(
                            values
                                .get(1)
                                .and_then(Value::as_str)
                                .ok_or(Error::Invalid("fill color"))?,
                        )
                        .map_err(host_error)?,
                    "strokeStyle" => canvas
                        .set_stroke_color(
                            values
                                .get(1)
                                .and_then(Value::as_str)
                                .ok_or(Error::Invalid("stroke color"))?,
                        )
                        .map_err(host_error)?,
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
                let (sw, sh) = (source.pixels.width() as f64, source.pixels.height() as f64);
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
                canvas.draw_image(source, src, dst).map_err(host_error)?;
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
    fn call(
        &self,
        _: &Generation,
        operation: &str,
        input: &Value,
        budget: HostBudget,
    ) -> Result<Value> {
        budget.check()?;
        match operation {
            "transport.command" => {
                let base = self.kernel.lock().unwrap().timebase()?;
                return self.transport.command(input, base);
            }
            "presentation.invalidate" => {
                self.state.lock().unwrap().invalidated = true;
                self.events.presentation();
                return Ok(Value::Null);
            }
            "kernel.call" => {
                let time = self.transport.time();
                return self.kernel.lock().unwrap().call(input, time);
            }
            "kernel.publication" => return self.kernel.lock().unwrap().publication(),
            "kernel.query" => return self.kernel.lock().unwrap().query(input),
            "extension.health" => {
                self.events.health(
                    input["id"].as_str().unwrap_or_default(),
                    &input["health"],
                );
                return Ok(Value::Null);
            }
            "ui.toast" => {
                self.events
                    .toast(input["text"].as_str().unwrap_or_default(), &input["options"]);
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
            "panel.request" => return self.events.panel(input),
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
                if !state.surfaces.contains_key(id) || bindings.insert(node, id.into()).is_some() {
                    return Err(Error::Invalid("canvas binding conflict"));
                }
            }
            state.bindings = bindings;
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
        let storage = self.state.lock().unwrap().storage.clone();
        Ok(if storage.is_null() {
            json!({})
        } else {
            storage
        })
    }
    fn commit_activation(
        &self,
        _: Option<&Generation>,
        _: &Generation,
        ops: &[Value],
        storage: &Value,
    ) -> Result<()> {
        if !ops.is_empty() {
            return Err(Error::Host("kernel activation backend not attached".into()));
        }
        self.state.lock().unwrap().storage = storage.clone();
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
    fn fetch(
        &self,
        _: &Generation,
        request: FetchRequest,
        cancel: Cancellation,
    ) -> Result<Box<dyn HostRequest>> {
        crate::net::fetch(request, cancel)
    }
    fn console(&self, _: &Generation, _: &str, _: &str) {}
}
impl WatchdogHooks for NativeHost {
    fn notice(&self, _: &Generation, _: u64) {}
    /// Eight seconds on one task: stop it and rebuild the realm without it.
    fn prompt(&self, _: &Generation, _: u64) -> WatchdogDecision {
        self.events.watchdog_stop();
        WatchdogDecision::Stop
    }
}
