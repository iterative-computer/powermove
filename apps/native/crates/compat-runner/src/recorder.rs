use pm_recorder::{Canvas, FillRule, Point, Rect, Source};
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
}
pub struct NativeHost {
    state: Mutex<State>,
    kernel: Mutex<crate::kernel::KernelAdapter>,
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
impl NativeHost {
    pub fn new(initial: &Value) -> Result<Self> {
        Ok(Self {
            state: Mutex::new(State::default()),
            kernel: Mutex::new(crate::kernel::KernelAdapter::new(initial)?),
        })
    }
    pub fn publication(&self) -> Value {
        self.kernel.lock().unwrap().publication()
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
                let rule = if values.first().and_then(Value::as_str) == Some("evenodd") {
                    FillRule::EvenOdd
                } else {
                    FillRule::NonZero
                };
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
            "presentation.invalidate" => {
                self.state.lock().unwrap().invalidated = true;
                return Ok(Value::Null);
            }
            "kernel.call" => return self.kernel.lock().unwrap().call(input),
            "kernel.query" => return self.kernel.lock().unwrap().query(input),
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
            state.dom.push(input["batch"].clone());
            return Ok(json!(true));
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
    fn fetch(
        &self,
        _: &Generation,
        _: FetchRequest,
        _: Cancellation,
    ) -> Result<Box<dyn HostRequest>> {
        self.state.lock().unwrap().missing.insert("fetch".into());
        Err(Error::Host(
            "network permission adapter not attached".into(),
        ))
    }
    fn console(&self, _: &Generation, _: &str, _: &str) {}
}
impl WatchdogHooks for NativeHost {
    fn notice(&self, _: &Generation, _: u64) {}
    fn prompt(&self, _: &Generation, _: u64) -> WatchdogDecision {
        WatchdogDecision::Stop
    }
}
