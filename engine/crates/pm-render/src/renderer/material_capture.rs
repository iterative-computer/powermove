//! Bounded, source-local material captures through the ordinary compositor.
use super::*;
use pm_eval::evaluator::{EvalHooks, PropEval};
use pm_eval::js_sem::JsResult;
use pm_eval::transform::{invert, mul};
use pm_eval::{Scope, node};
use pm_fx_defs::spatial::Extent;
use pm_model::{JsString, Node};
use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;

const MAX_PIXELS: u64 = 4_194_304;
const MAX_CACHE: u64 = 128 * 1024 * 1024;
#[derive(Clone, Debug)]
pub struct MaterialCaptureRequest {
    pub composition: String,
    pub layer: Option<String>,
    pub time: Ticks,
    pub width: u32,
    pub height: u32,
    /// Only the supplying graph, assets and animation; receiver edits excluded.
    pub source_key: String,
    pub frame_key: String,
    /// Zero disables pixel retention, while transient limits still apply.
    pub cache_bytes: u64,
    pub final_output: bool,
}
#[derive(Clone, Debug)]
pub struct MaterialPixels {
    pub key: String,
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<Vec<u8>>,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct MaterialCaptureStats {
    pub captures: u64,
    pub cache_hits: u64,
    pub retained_bytes: u64,
}
#[derive(Clone, Default)]
struct Visibility {
    composition: String,
    root: String,
}
#[derive(Default)]
struct CaptureHooks(Mutex<Visibility>);
fn id(layer: &Node) -> String {
    node::string(node::member(Some(layer), "id"))
        .map(|s| s.to_string_lossy())
        .unwrap_or_default()
}
fn kind(layer: &Node) -> String {
    crate::js::layer_type(layer)
}
impl EvalHooks for CaptureHooks {
    fn active(
        &self,
        cx: &Comp<'_>,
        layer: Option<&Node>,
        time: f64,
        base: bool,
        eval: &mut dyn PropEval,
    ) -> JsResult<bool> {
        let context = self.0.lock().expect("capture visibility").clone();
        if cx.id().to_string_lossy() != context.composition || context.root.is_empty() {
            return Ok(base);
        }
        let Some(layer) = layer else { return Ok(base) };
        let own = id(layer) == context.root;
        let mut groups = vec![];
        let mut next =
            node::string(node::member(Some(layer), "group")).map(|s| s.to_string_lossy());
        let mut seen = HashSet::new();
        let mut inside = own;
        while let Some(group_id) = next {
            if !seen.insert(group_id.clone()) {
                return Err(pm_eval::JsThrow::type_error("Material source group cycle"));
            }
            let Some(group) = cx.layers().iter().rev().find(|l| id(l) == group_id) else {
                break;
            };
            if group_id == context.root {
                inside = true;
                break;
            }
            groups.push(group);
            next = node::string(node::member(Some(group), "group")).map(|s| s.to_string_lossy());
        }
        if !inside {
            return Ok(base);
        }
        let mut on = |layer: &Node| -> JsResult<bool> {
            let value = node::member(Some(layer), "on");
            if node::is_property(value) {
                Ok(eval
                    .ev_p(cx, Some(layer), value, time, &JsString::from("l.on"))?
                    .truthy())
            } else {
                Ok(node::truthy(value))
            }
        };
        if !own {
            if !on(layer)? {
                return Ok(false);
            }
            for group in groups {
                if !on(group)? {
                    return Ok(false);
                }
            }
        }
        let start = node::to_number(node::member(Some(layer), "from"));
        let duration = node::to_number(node::member(Some(layer), "dur"));
        if !start.is_finite() || !duration.is_finite() || duration <= 0. || !time.is_finite() {
            return Ok(false);
        }
        if kind(layer) == "group" {
            return Ok(true);
        }
        let end = start + duration;
        let comp_end = cx.duration();
        Ok(time >= start - 1e-6
            && (time < end - 1e-6
                || (comp_end.is_finite()
                    && (time - comp_end).abs() <= 1e-6
                    && (end - comp_end).abs() <= 1e-6)))
    }
}
#[derive(Default)]
pub(super) struct State {
    pub plan: Option<plan::MaterialPlan>,
    pub scale: Option<(f64, f64)>,
    binding: Option<u64>,
    frames: VecDeque<(String, Arc<MaterialPixels>)>,
    bounds: VecDeque<(String, Extent)>,
    evaluator: Option<Evaluator>,
    hooks: Arc<CaptureHooks>,
    pub stats: MaterialCaptureStats,
}
impl State {
    pub fn trim(&mut self, budget: u64) {
        while self.stats.retained_bytes > budget {
            let Some((_, frame)) = self.frames.pop_front() else {
                break;
            };
            self.stats.retained_bytes = self
                .stats
                .retained_bytes
                .saturating_sub(frame.pixels.len() as u64);
        }
    }
}
#[derive(Default)]
struct MissingMeasure(bool);
impl pm_eval::text_geom::TextMeasure for MissingMeasure {
    fn measure_lines(
        &mut self,
        _: &pm_text::compat::TextContent,
    ) -> Vec<pm_eval::text_geom::MeasuredLine> {
        self.0 = true;
        vec![]
    }
    fn plain_width(&mut self, _: &pm_text::compat::TextContent, _: &str) -> f64 {
        self.0 = true;
        f64::NAN
    }
    fn glyph_layout(&mut self, _: &pm_text::compat::TextContent) -> pm_text::compat::GlyphLayout {
        self.0 = true;
        Default::default()
    }
    fn spaced_width(&mut self, _: &pm_text::compat::TextContent, _: &str) -> f64 {
        self.0 = true;
        f64::NAN
    }
}
fn animated(node: &Node) -> bool {
    match node.kind() {
        pm_model::NodeKind::Object(map) => map.iter().any(|(key, value)| {
            (key.as_str() == Some("kf") && node::array(Some(value)).is_some_and(|a| !a.is_empty()))
                || (key.as_str() == Some("expr")
                    && node::string(Some(value)).is_some_and(|s| !s.as_bytes().is_empty()))
                || animated(value)
        }),
        pm_model::NodeKind::Array(a) => a.iter().any(animated),
        _ => false,
    }
}
fn key_times(node: &Node, start: f64, lo: f64, hi: f64, times: &mut Vec<f64>) {
    if let Some(keys) = node::array(node::member(Some(node), "kf")) {
        for key in keys {
            let t = start + node::to_number(node::member(Some(key), "t"));
            if t.is_finite() && t >= lo && t <= hi {
                times.push(t);
            }
        }
    }
    match node.kind() {
        pm_model::NodeKind::Object(map) => {
            for (_, value) in map.iter() {
                key_times(value, start, lo, hi, times)
            }
        }
        pm_model::NodeKind::Array(a) => {
            for value in a.iter() {
                key_times(value, start, lo, hi, times)
            }
        }
        _ => {}
    }
}
fn source_layers<'d>(comp: &Comp<'d>, root: &'d Node) -> Result<Vec<&'d Node>, RenderError> {
    let mut ids = HashSet::from([id(root)]);
    let mut layers = vec![root];
    if kind(root) == "group" {
        loop {
            let mut changed = false;
            for layer in comp.layers() {
                let group =
                    node::string(node::member(Some(layer), "group")).map(|s| s.to_string_lossy());
                if group.is_some_and(|g| ids.contains(&g)) && ids.insert(id(layer)) {
                    layers.push(layer);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }
    if layers.iter().any(|l| {
        node::truthy(node::member(Some(l), "threeD")) || pm_eval::parenting::Role3d::of(l).is_some()
    }) {
        return Err(RenderError::Request(
            "Layer/group material sources cannot contain 3D descendants".into(),
        ));
    }
    Ok(layers)
}
impl Renderer {
    pub fn material_capture_binding(&self) -> Option<u64> {
        self.material.binding
    }
    pub fn set_material_capture_binding(&mut self, binding: u64) {
        self.material.binding = Some(binding);
    }
    pub fn material_capture_stats(&self) -> MaterialCaptureStats {
        self.material.stats
    }
    fn material_bounds<'d>(
        &mut self,
        document: &'d Document,
        comp: &Comp<'d>,
        root: &'d Node,
        time: f64,
        key: &str,
    ) -> Result<Extent, RenderError> {
        if let Some(index) = self.material.bounds.iter().position(|(k, _)| k == key) {
            let entry = self.material.bounds.remove(index).unwrap();
            let bounds = entry.1;
            self.material.bounds.push_back(entry);
            return Ok(bounds);
        }
        let layers = source_layers(comp, root)?;
        let group = kind(root) == "group";
        let source_animated = layers.iter().any(|layer| animated(layer));
        let open = capability::open_composition_id(document.root().unwrap());
        let project = open
            .and_then(|s| Comp::resolve(document, &s).ok())
            .unwrap_or_else(|| comp.clone());
        let mut scope = Scope::new(&mut self.evaluator, comp, &project);
        let mut missing = MissingMeasure::default();
        let measure: &mut dyn pm_eval::text_geom::TextMeasure = match self.text.fonts.as_deref_mut()
        {
            Some(fonts) => fonts.measure(),
            None => &mut missing,
        };
        let fps = node::number(comp.member("fps")).unwrap_or(30.).max(1.);
        let start = node::number(node::member(Some(root), "from"))
            .unwrap_or(0.)
            .max(0.);
        let end = (start
            + node::number(node::member(Some(root), "dur")).unwrap_or(comp.duration()))
        .min(comp.duration())
        .max(start);
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        let mut work = 0usize;
        for layer in &layers {
            if group
                && matches!(
                    kind(layer).as_str(),
                    "group" | "null" | "audio" | "adjustment"
                )
            {
                continue;
            }
            let mut times = vec![time];
            if group {
                let from = node::number(node::member(Some(layer), "from")).unwrap_or(0.);
                let lo = start.max(from);
                let hi =
                    end.min(from + node::number(node::member(Some(layer), "dur")).unwrap_or(0.));
                if hi < lo {
                    continue;
                }
                times = vec![lo, hi];
                // Static geometry needs one sample. Animated sources retain the
                // source contract's frame grid plus every subframe key time.
                if source_animated {
                    let a = (lo * fps).ceil() as i64;
                    let b = (hi * fps).floor() as i64;
                    if b.saturating_sub(a) > 1_000_000 {
                        return Err(RenderError::Request(
                            "Material bounds exceed the evaluation budget".into(),
                        ));
                    }
                    times.extend((a..=b).map(|f| f as f64 / fps));
                    for dependency in &layers {
                        let start = node::number(dependency.get("from")).unwrap_or(0.);
                        key_times(dependency, start, lo, hi, &mut times);
                    }
                }
            }
            times.sort_by(f64::total_cmp);
            times.dedup_by(|a, b| a.to_bits() == b.to_bits());
            for at in times {
                work += 1;
                if work > 1_000_000 {
                    return Err(RenderError::Request(
                        "Material bounds exceed the evaluation budget".into(),
                    ));
                }
                let Some(b) = scope
                    .bounds(layer, at, measure, &mut pm_eval::bounds::NoHost)
                    .map_err(|e| RenderError::Request(e.to_string()))?
                else {
                    continue;
                };
                let local = if group {
                    let root = scope
                        .world_matrix(root, at)
                        .map_err(|e| RenderError::Request(e.to_string()))?;
                    let Some(inverse) = invert(&root) else {
                        continue;
                    };
                    mul(
                        &inverse,
                        &scope
                            .world_matrix(layer, at)
                            .map_err(|e| RenderError::Request(e.to_string()))?,
                    )
                } else {
                    [1., 0., 0., 1., 0., 0.]
                };
                for x in [b.x0, b.x1] {
                    for y in [b.y0, b.y1] {
                        let p = [
                            local[0] * x + local[2] * y + local[4],
                            local[1] * x + local[3] * y + local[5],
                        ];
                        if p.iter().any(|v| !v.is_finite()) {
                            return Err(RenderError::Request(
                                "Material bounds are not finite".into(),
                            ));
                        }
                        bounds[0] = bounds[0].min(p[0]);
                        bounds[1] = bounds[1].min(p[1]);
                        bounds[2] = bounds[2].max(p[0]);
                        bounds[3] = bounds[3].max(p[1]);
                    }
                }
            }
        }
        if missing.0 {
            return Err(RenderError::Request(
                "Material text bounds need the native font service".into(),
            ));
        }
        let bounds = if bounds[0].is_finite() {
            Extent {
                x: bounds[0],
                y: bounds[1],
                width: (bounds[2] - bounds[0]).max(1.),
                height: (bounds[3] - bounds[1]).max(1.),
            }
        } else {
            Extent {
                x: 0.,
                y: 0.,
                width: 1.,
                height: 1.,
            }
        };
        self.material.bounds.push_back((key.into(), bounds));
        if self.material.bounds.len() > 64 {
            self.material.bounds.pop_front();
        }
        Ok(bounds)
    }
    /// Capture one supplying composition or a 2D layer/group subtree. Pixels
    /// are straight display-encoded RGB, top-down, without the receiving layer.
    pub fn capture_material(
        &mut self,
        document: &Document,
        request: &MaterialCaptureRequest,
        assets: &dyn AssetResolver,
    ) -> Result<Arc<MaterialPixels>, RenderError> {
        if request.width == 0
            || request.height == 0
            || request.width > 8192
            || request.height > 8192
            || u64::from(request.width) * u64::from(request.height) > MAX_PIXELS
        {
            return Err(RenderError::Request(
                "Material capture exceeds the pixel and memory budget".into(),
            ));
        }
        let color = project_settings(
            document
                .root()
                .ok_or_else(|| RenderError::Request("No material project".into()))?,
        )?;
        let font = self.text.fonts.as_ref().map_or(0, |f| f.generation());
        let definitions = self.definitions();
        use sha2::{Digest, Sha256};
        let mut digest = Sha256::new();
        digest.update(request.source_key.as_bytes());
        digest.update(format!("{color:?}:{font}").as_bytes());
        digest.update(serde_json::to_vec(&definitions.definitions).unwrap_or_default());
        digest.update(serde_json::to_vec(&definitions.missing).unwrap_or_default());
        let stamp = format!("{:x}", digest.finalize());
        let key = format!(
            "{stamp}:{}:{}x{}:{}",
            request.frame_key, request.width, request.height, request.final_output
        );
        let budget = request.cache_bytes.min(MAX_CACHE);
        self.material.trim(budget);
        if budget > 0
            && let Some(index) = self.material.frames.iter().position(|(k, _)| k == &key)
        {
            let entry = self.material.frames.remove(index).unwrap();
            let frame = entry.1.clone();
            self.material.frames.push_back(entry);
            self.material.stats.cache_hits += 1;
            return Ok(frame);
        }
        let comp = Comp::resolve(document, &request.composition)
            .map_err(|_| RenderError::Composition(request.composition.clone()))?;
        let root = request
            .layer
            .as_ref()
            .map(|id| {
                comp.layers()
                    .iter()
                    .find(|l| self::id(l) == *id)
                    .ok_or_else(|| RenderError::Request(format!("Material layer is missing: {id}")))
            })
            .transpose()?;
        let seconds = comp.seconds_at(request.time.get());
        let bounds = match root {
            Some(root) => {
                let bounds_key = if kind(root) == "group" {
                    stamp.clone()
                } else {
                    format!("{stamp}:{}", request.frame_key)
                };
                self.material_bounds(document, &comp, root, seconds, &bounds_key)?
            }
            None => Extent {
                x: 0.,
                y: 0.,
                width: node::number(comp.member("w")).unwrap_or(1.).max(1.),
                height: node::number(comp.member("h")).unwrap_or(1.).max(1.),
            },
        };
        let inverse = if let Some(root) = root {
            let project = capability::open_composition_id(document.root().unwrap())
                .and_then(|id| Comp::resolve(document, &id).ok())
                .unwrap_or_else(|| comp.clone());
            let mut scope = Scope::new(&mut self.evaluator, &comp, &project);
            invert(
                &scope
                    .world_matrix(root, seconds)
                    .map_err(|e| RenderError::Request(e.to_string()))?,
            )
            .unwrap_or([0.; 6])
        } else {
            [1., 0., 0., 1., 0., 0.]
        };
        let previous_plan = self.material.plan;
        let previous_scale = self.material.scale;
        self.material.scale = Some((
            request.width as f64 / bounds.width,
            request.height as f64 / bounds.height,
        ));
        self.material.plan = root.map(|root| plan::MaterialPlan {
            composition: std::ptr::from_ref(comp.node()) as usize,
            layer: std::ptr::from_ref(root) as usize,
            inverse,
            bounds,
            group: kind(root) == "group",
        });
        let visibility = Visibility {
            composition: request.composition.clone(),
            root: request.layer.clone().unwrap_or_default(),
        };
        *self.material.hooks.0.lock().expect("capture visibility") = visibility;
        let evaluator = self.material.evaluator.take().unwrap_or_else(|| {
            Evaluator::new()
                .with_expressions(Arc::new(PmExprEngine))
                .with_hooks(self.material.hooks.clone())
        });
        let original = std::mem::replace(&mut self.evaluator, evaluator);
        let reused = self.reuse_enabled;
        self.reuse_enabled = false;
        let mut render = RenderRequest::sized(
            pm_protocol::common::Size::new(request.width, request.height),
            Precision::Float16,
        );
        render.background = false;
        render.export = request.final_output;
        render.output_kind = if request.final_output {
            crate::OutputKind::Final
        } else {
            crate::OutputKind::Preview
        };

        let result = self.render(
            document,
            &request.composition,
            request.time,
            &render,
            assets,
        );
        self.material.evaluator = Some(std::mem::replace(&mut self.evaluator, original));
        self.reuse_enabled = reused;
        self.material.plan = previous_plan;
        self.material.scale = previous_scale;
        let frame = result?;
        if !frame.failures.is_empty() {
            let error = frame
                .failures
                .iter()
                .map(|f| format!("{}: {}", f.layer, f.message))
                .collect::<Vec<_>>()
                .join("; ");
            self.recycle(frame);
            return Err(RenderError::Request(error));
        }
        let read = self.read_rgba(&frame);
        self.recycle(frame);
        let read = read?;
        let mut pixels = Vec::with_capacity(read.len());
        for p in read.chunks_exact(4) {
            let alpha = p[3] as f64;
            for channel in &p[..3] {
                let value = if alpha > 0. {
                    pm_color::transfer::working_to_display(color.working, *channel as f64 / alpha)
                } else {
                    0.
                };
                pixels.push((value * 255.).round().clamp(0., 255.) as u8);
            }
            pixels.push((alpha * 255.).round().clamp(0., 255.) as u8);
        }
        let frame = Arc::new(MaterialPixels {
            key: key.clone(),
            width: request.width,
            height: request.height,
            pixels: Arc::new(pixels),
        });
        self.material.stats.captures += 1;
        if budget > 0 && frame.pixels.len() as u64 <= budget {
            self.material.stats.retained_bytes += frame.pixels.len() as u64;
            self.material.frames.push_back((key, frame.clone()));
            self.material.trim(budget);
        }
        Ok(frame)
    }
}
