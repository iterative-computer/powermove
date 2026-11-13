//! Immutable source graphs, per-receiver clocks and bounded compositor captures.
use super::*;
use pm_eval::{Comp, Evaluator, Scope};
use serde_json::json;
use std::collections::{BTreeSet, HashSet};

#[derive(Clone)]
struct Child {
    receiver: String,
    local: bool,
    material: bool,
    clock: Clock,
}
#[derive(Clone)]
struct Clock {
    composition: String,
    duration: f64,
    fixed: bool,
    edges: Vec<f64>,
    children: Vec<Child>,
}
impl Clock {
    fn key(&self, document: &pm_model::Document, time: f64) -> Result<String, String> {
        let mut key = if !self.fixed || (time - self.duration).abs() <= 1e-6 {
            format!("frame:{:x}", time.to_bits())
        } else {
            format!(
                "still:{}",
                self.edges.iter().filter(|e| **e <= time).count()
            )
        };
        if self.children.is_empty() {
            return Ok(key);
        }
        let comp = Comp::resolve(document, &self.composition).map_err(|e| e.to_string())?;
        let open = pm_eval::node::string(document.root().and_then(|r| r.get("compId")))
            .map(|s| s.to_string_lossy());
        let project = open
            .and_then(|id| Comp::resolve(document, &id).ok())
            .unwrap_or_else(|| comp.clone());
        let mut evaluator =
            Evaluator::new().with_expressions(Arc::new(pm_render::expr::PmExprEngine));
        let mut scope = Scope::new(&mut evaluator, &comp, &project);
        for child in &self.children {
            let layer = comp
                .layers()
                .iter()
                .find(|l| pm_eval::node::is_str(l.get("id"), &child.receiver))
                .ok_or("Material clock receiver disappeared")?;
            let mut at = if child.local {
                time
            } else {
                scope
                    .source_time_with(
                        layer,
                        time,
                        if child.material {
                            pm_eval::retime::Retiming::Legacy
                        } else {
                            pm_eval::retime::Retiming::Engine
                        },
                        None,
                    )
                    .map_err(|e| e.message)?
            };
            if child.material && !child.local {
                let source =
                    Comp::resolve(document, &child.clock.composition).map_err(|e| e.to_string())?;
                let fps = pm_eval::node::number(source.member("fps"))
                    .unwrap_or(30.)
                    .max(1.);
                at = ((at * fps + 1e-6)
                    .floor()
                    .max(0.)
                    .min((source.duration() * fps).ceil().max(1.) - 1.))
                    / fps;
            }
            key.push(':');
            key.push_str(&child.clock.key(document, at)?);
        }
        Ok(key)
    }
}
pub(super) struct Plan {
    composition: String,
    layer: Option<String>,
    key: String,
    clock: Clock,
    files: Vec<String>,
    expressions: bool,
}
#[derive(Clone)]
struct Frame {
    key: String,
    pixels: Arc<Vec<u8>>,
}
pub(super) struct Bindings {
    request: SceneRequest,
    tier: Tier,
    frames: HashMap<(String, String), Result<Frame, String>>,
}
fn map_key(source: &Value) -> String {
    source.to_string()
}
fn references(layer: &Value) -> Vec<Value> {
    let object = &layer["d"]["data"]["object"];
    let mut refs = vec![];
    if let Some(id) = object["material"]["screen"].as_str() {
        refs.push(json!({"compositionId":id}));
    }
    let maps = std::iter::once(&object["material"]["maps"])
        .chain(
            object["sourceMaterials"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.values()),
        )
        .chain(
            pm_scene3d::extrusion::SURFACES
                .iter()
                .map(|name| &layer["d"]["extrusion"]["surfaces"][name]["maps"]),
        );
    for maps in maps {
        for source in maps.as_object().into_iter().flat_map(|m| m.values()) {
            if source.is_object()
                && (source["compositionId"].is_string() || source["layerId"].is_string())
            {
                refs.push(source.clone());
            }
        }
    }
    refs.sort_by_key(map_key);
    refs.dedup();
    refs
}
fn subtree(layers: &[Value], root: &str) -> Result<HashSet<String>, String> {
    let owner = layers
        .iter()
        .find(|l| l["id"] == root)
        .ok_or_else(|| format!("Material source layer is missing: {root}"))?;
    let mut ids = HashSet::from([root.to_string()]);
    if owner["type"] == "group" {
        loop {
            let mut changed = false;
            for layer in layers {
                if layer["group"].as_str().is_some_and(|g| ids.contains(g)) {
                    if let Some(id) = layer["id"].as_str() {
                        changed |= ids.insert(id.into());
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }
    Ok(ids)
}
fn constant(value: &Value) -> bool {
    match value {
        Value::Object(m) => {
            !m.get("expr")
                .is_some_and(|v| v.as_str().is_some_and(|s| !s.is_empty()))
                && !m
                    .get("kf")
                    .is_some_and(|v| v.as_array().is_some_and(|a| a.len() > 1))
                && m.values().all(constant)
        }
        Value::Array(a) => a.iter().all(constant),
        _ => true,
    }
}
fn has_expression(value: &Value) -> bool {
    match value {
        Value::Object(m) => {
            m.get("expr")
                .is_some_and(|v| v.as_str().is_some_and(|s| !s.is_empty()))
                || m.values().any(has_expression)
        }
        Value::Array(a) => a.iter().any(has_expression),
        _ => false,
    }
}
struct Planner<'a> {
    document: &'a pm_model::Document,
    receiver_comp: &'a str,
    receiver: &'a str,
    visiting: HashSet<(String, Option<String>)>,
    two_dimensional: bool,
}
impl Planner<'_> {
    fn build(&mut self, composition: &str, root: Option<&str>) -> Result<Plan, String> {
        let identity = (composition.to_string(), root.map(str::to_string));
        if self.visiting.len() >= 8 || !self.visiting.insert(identity.clone()) {
            return Err("Material source dependency cycle or excessive nesting".into());
        }
        let previous = self.two_dimensional;
        self.two_dimensional |= root.is_some();
        let result = self.build_inner(composition, root);
        self.two_dimensional = previous;
        self.visiting.remove(&identity);
        result
    }
    fn build_inner(&mut self, composition: &str, root: Option<&str>) -> Result<Plan, String> {
        let resolved = Comp::resolve(self.document, composition).map_err(|e| e.to_string())?;
        let comp =
            pm_scene3d::from_model(&resolved.node().to_value()).map_err(|e| e.to_string())?;
        let layers = comp["layers"]
            .as_array()
            .ok_or("Material composition has no layers")?;
        let mut drawn = match root {
            Some(root) => subtree(layers, root)?,
            None => layers
                .iter()
                .filter_map(|l| l["id"].as_str().map(str::to_string))
                .collect(),
        };
        if composition == self.receiver_comp && (root.is_none() || drawn.contains(self.receiver)) {
            return Err("Material source creates a receiver/composition cycle".into());
        }
        if let Some(root) = root {
            let layer = layers.iter().find(|l| l["id"] == root).unwrap();
            if !matches!(
                layer["type"].as_str(),
                Some("solid" | "shape" | "text" | "image" | "video" | "precomp" | "group")
            ) {
                return Err("Choose an eligible raster artwork or group source".into());
            }
        }
        if self.two_dimensional {
            if layers
                .iter()
                .filter(|l| l["id"].as_str().is_some_and(|id| drawn.contains(id)))
                .any(|l| l["threeD"] == true || pm_scene3d::schema::role(l).is_some())
            {
                return Err("Layer/group material sources cannot contain 3D descendants".into());
            }
        }
        // Draw dependencies may live outside the selected group. Parent/group
        // dependencies enter the signature, without drawing their material.
        loop {
            let mut add: Vec<String> = vec![];
            for layer in layers
                .iter()
                .filter(|l| l["id"].as_str().is_some_and(|id| drawn.contains(id)))
            {
                if let Some(id) = layer["matteSource"].as_str() {
                    add.push(id.to_string());
                }
                for matte in layer["matteSources"].as_array().into_iter().flatten() {
                    if let Some(id) = matte["source"].as_str() {
                        add.push(id.into());
                    }
                }
                for effect in layer["fx"].as_array().into_iter().flatten() {
                    for input in effect["inputs"]
                        .as_object()
                        .into_iter()
                        .flat_map(|m| m.values())
                    {
                        if let Some(id) = input["layerId"].as_str() {
                            add.push(id.into());
                        }
                    }
                }
            }
            let old = drawn.len();
            for id in add {
                drawn.extend(subtree(layers, &id)?);
            }
            if old == drawn.len() {
                break;
            }
        }
        if composition == self.receiver_comp && (root.is_none() || drawn.contains(self.receiver)) {
            return Err("Material source creates a receiver/composition cycle".into());
        }
        let mut included = drawn.clone();
        loop {
            let mut add: Vec<String> = vec![];
            for layer in layers
                .iter()
                .filter(|l| l["id"].as_str().is_some_and(|id| included.contains(id)))
            {
                for field in ["parent", "group"] {
                    if let Some(id) = layer[field].as_str() {
                        add.push(id.into());
                    }
                }
            }
            let old = included.len();
            included.extend(add);
            if included.len() == old {
                break;
            }
        }
        let selected: Vec<_> = layers
            .iter()
            .filter(|l| l["id"].as_str().is_some_and(|id| included.contains(id)))
            .collect();
        let mut fixed = true;
        let mut edges = vec![];
        let mut children = vec![];
        let mut child_keys = vec![];
        let mut files = BTreeSet::new();
        let mut expressions = false;
        for layer in &selected {
            expressions |= has_expression(layer);
            let mut candidate = (*layer).clone();
            if candidate["type"] == "precomp" {
                if let Some(data) = candidate["d"].as_object_mut() {
                    for key in ["trim", "speed", "timeRemap", "sourceTime"] {
                        data.remove(key);
                    }
                }
            }
            fixed &= constant(&candidate)
                && matches!(
                    layer["type"].as_str(),
                    Some("solid" | "shape" | "text" | "null" | "group" | "image" | "precomp")
                )
                && layer["transitionIn"].is_null()
                && layer["transitionOut"].is_null()
                && layer["fx"].as_array().is_none_or(Vec::is_empty)
                && layer["d"]["animators"].as_array().is_none_or(Vec::is_empty);
            let from = layer["from"].as_f64().unwrap_or(0.);
            let duration = layer["dur"].as_f64().unwrap_or(0.);
            edges.extend([from - 1e-6, from + duration - 1e-6]);
            if let Some(asset) = layer["d"]["asset"].as_str() {
                files.insert(asset.to_string());
                let record = self
                    .document
                    .root()
                    .and_then(|r| r.get("assets"))
                    .and_then(|a| a.get(asset));
                if record.is_some_and(|a| {
                    pm_eval::node::truthy(a.get("imageSequence"))
                        || pm_eval::node::is_str(a.get("format"), "svg")
                }) {
                    fixed = false;
                }
            }
            if !layer["id"].as_str().is_some_and(|id| drawn.contains(id)) {
                continue;
            }
            if let Some(child) = layer["d"]["comp"]
                .as_str()
                .filter(|_| layer["type"] == "precomp")
            {
                let child = self.build(child, None)?;
                files.extend(child.files.iter().cloned());
                expressions |= child.expressions;
                child_keys.push(child.key.clone());
                children.push(Child {
                    receiver: layer["id"].as_str().unwrap().into(),
                    local: false,
                    material: false,
                    clock: child.clock,
                });
            }
            for source in references(layer) {
                let local = source["layerId"].is_string();
                let comp = source["compositionId"].as_str().unwrap_or(composition);
                let child = self.build(comp, source["layerId"].as_str())?;
                files.extend(child.files.iter().cloned());
                expressions |= child.expressions;
                child_keys.push(child.key.clone());
                children.push(Child {
                    receiver: layer["id"].as_str().unwrap().into(),
                    local,
                    material: true,
                    clock: child.clock,
                });
            }
        }
        let mut digest = blake3::Hasher::new();
        digest.update(
            json!([
                composition,
                root,
                comp["w"],
                comp["h"],
                comp["fps"],
                comp["dur"],
                selected,
                child_keys
            ])
            .to_string()
            .as_bytes(),
        );
        for asset in &files {
            if let Some(record) = self
                .document
                .root()
                .and_then(|r| r.get("assets"))
                .and_then(|a| a.get(asset))
            {
                digest.update(pm_model::json::to_canonical_string(&record.to_value()).as_bytes());
            }
        }
        edges.sort_by(f64::total_cmp);
        edges.dedup_by(|a, b| a.to_bits() == b.to_bits());
        Ok(Plan {
            composition: composition.into(),
            layer: root.map(str::to_string),
            key: digest.finalize().to_hex().to_string(),
            files: files.into_iter().collect(),
            expressions,
            clock: Clock {
                composition: composition.into(),
                duration: resolved.duration(),
                fixed,
                edges,
                children,
            },
        })
    }
}
impl DocumentSource {
    pub(super) fn material_references(
        &self,
        request: &SceneRequest,
    ) -> Result<Vec<(String, Value)>, String> {
        // Selectors belong to this immutable document; file/font/time changes
        // affect captured pixels, not which receivers reference which sources.
        let mut cached=self.material_references.lock().map_err(|_|"Material reference cache poisoned")?;
        if let Some(references)=cached.get(&request.composition) { return Ok(references.as_ref().clone()); }
        let comp = Comp::resolve(&self.snapshot.document, &request.composition)
            .map_err(|e| e.to_string())?;
        let mut out = vec![];
        for layer in comp.layers() {
            let role = pm_eval::parenting::Role3d::of(layer);
            let extrusion = layer.get("d").and_then(|d| d.get("extrusion"));
            if role != Some(pm_eval::parenting::Role3d::Object)
                && !pm_eval::node::truthy(extrusion.and_then(|d| d.get("enabled")))
            {
                continue;
            }
            let layer = pm_scene3d::from_model(&layer.to_value()).map_err(|e| e.to_string())?;
            let id = layer["id"].as_str().ok_or("Material receiver has no ID")?;
            out.extend(
                references(&layer)
                    .into_iter()
                    .map(|source| (id.into(), source)),
            );
        }
        cached.insert(request.composition.clone(),Arc::new(out.clone()));
        Ok(out)
    }
    pub(super) fn captured_material(
        &self,
        request: &SceneRequest,
        owner: &str,
        source: &Value,
    ) -> Result<(String, Arc<Vec<u8>>), String> {
        let bindings = self
            .capture_bindings
            .lock()
            .map_err(|_| "Material capture bindings poisoned")?;
        let frame = bindings
            .as_ref()
            .filter(|b| b.request == *request)
            .and_then(|b| b.frames.get(&(owner.into(), map_key(source))))
            .ok_or("Live layer/group material capture is not ready")?
            .as_ref()
            .map_err(Clone::clone)?;
        Ok((format!("capture:{}", frame.key), frame.pixels.clone()))
    }
    pub(super) fn capture_materials(
        &self,
        renderer: &mut pm_render::Renderer,
        request: &SceneRequest,
        tier: Tier,
    ) -> Result<bool, String> {
        let refs = self.material_references(request)?;
        if refs.is_empty() {
            return Ok(false);
        }
        if renderer.material_capture_binding() != Some(self.capture_id) {
            if let Some(fonts) = &self.capture_fonts {
                renderer.set_text_fonts(Some(Box::new(pm_render::text::host::EngineFonts::new(
                    &fonts.broker,
                    &fonts.health,
                ))));
            } else {
                renderer
                    .set_text_fonts(self.capture_font_factory.as_ref().map(|factory| factory()));
            }
            renderer.set_document_effects(
                &self.snapshot.document,
                &self.capture_definitions.definitions,
            )?;
            renderer.set_material_capture_binding(self.capture_id);
        }
        let receiver = Comp::resolve(&self.snapshot.document, &request.composition)
            .map_err(|e| e.to_string())?;
        let mut evaluator =
            Evaluator::new().with_expressions(Arc::new(pm_render::expr::PmExprEngine));
        let project = self
            .snapshot
            .open_comp()
            .and_then(|id| Comp::resolve(&self.snapshot.document, &id).ok())
            .unwrap_or_else(|| receiver.clone());
        let mut scope = Scope::new(&mut evaluator, &receiver, &project);
        let time = request.ticks as f64 / pm_protocol::common::TICKS_PER_SECOND as f64;
        let mut frames = HashMap::new();
        let resolver = pm_render::MapResolver {
            paths: self.media.paths.clone(),
            originals: self.media.originals.clone(),
        };
        for (owner, source) in refs {
            let binding_key = (owner.clone(), map_key(&source));
            let result = (|| -> Result<Frame, String> {
                let plan = {
                    let mut plans = self
                        .capture_plans
                        .lock()
                        .map_err(|_| "Material source plans poisoned")?;
                    if let Some(plan) = plans.get(&binding_key) {
                        plan.clone()
                    } else {
                        let mut planner = Planner {
                            document: &self.snapshot.document,
                            receiver_comp: &request.composition,
                            receiver: &owner,
                            visiting: HashSet::new(),
                            two_dimensional: false,
                        };
                        let composition = source["compositionId"]
                            .as_str()
                            .unwrap_or(&request.composition);
                        let plan =
                            Arc::new(planner.build(composition, source["layerId"].as_str())?);
                        plans.insert(binding_key.clone(), plan.clone());
                        plan
                    }
                };
                let target = Comp::resolve(&self.snapshot.document, &plan.composition)
                    .map_err(|e| e.to_string())?;
                let fps = pm_eval::node::number(target.member("fps"))
                    .unwrap_or(30.)
                    .max(1.);
                let at = if plan.layer.is_some() {
                    (time * fps + 1e-6).floor() / fps
                } else {
                    let layer = receiver
                        .layers()
                        .iter()
                        .find(|l| pm_eval::node::is_str(l.get("id"), &owner))
                        .ok_or("Material receiver disappeared")?;
                    let mapped = scope
                        .source_time_with(layer, time, pm_eval::retime::Retiming::Legacy, None)
                        .map_err(|e| e.message)?;
                    ((mapped * fps + 1e-6)
                        .floor()
                        .max(0.)
                        .min((target.duration() * fps).ceil().max(1.) - 1.))
                        / fps
                };
                let mut key = plan.key.clone();
                if plan.expressions {
                    key.push_str(&format!(":revision:{}", self.snapshot.revision));
                }
                for asset in &plan.files {
                    key.push_str(asset);
                    if let Some(path) = self.media.paths.get(asset) {
                        key.push_str(&path.to_string_lossy());
                        if let Ok(metadata) = std::fs::metadata(path) {
                            key.push_str(&format!(
                                ":{}:{:?}",
                                metadata.len(),
                                metadata.modified().ok()
                            ));
                        }
                    }
                }
                let frame = renderer
                    .capture_material(
                        &self.snapshot.document,
                        &pm_render::MaterialCaptureRequest {
                            composition: plan.composition.clone(),
                            layer: plan.layer.clone(),
                            time: pm_protocol::common::Ticks::new(
                                (at * pm_protocol::common::TICKS_PER_SECOND as f64).round() as i64,
                            )
                            .map_err(|e| format!("{e:?}"))?,
                            width: 1024,
                            height: 1024,
                            source_key: key,
                            frame_key: plan.clock.key(&self.snapshot.document, at)?,
                            cache_bytes: 128 * 1024 * 1024,
                            final_output: matches!(tier, Tier::Final { .. }),
                        },
                        &resolver,
                    )
                    .map_err(|e| e.to_string())?;
                let pixels = frame.pixels.clone();
                Ok(Frame {
                    key: frame.key.clone(),
                    pixels,
                })
            })();
            frames.insert(binding_key, result);
        }
        let mut bindings = self
            .capture_bindings
            .lock()
            .map_err(|_| "Material capture bindings poisoned")?;
        let changed = bindings.as_ref().is_none_or(|old| {
            old.request != *request
                || old.tier != tier
                || old.frames.len() != frames.len()
                || frames
                    .iter()
                    .any(|(key, frame)| match (old.frames.get(key), frame) {
                        (Some(Ok(a)), Ok(b)) => a.key != b.key,
                        (Some(Err(a)), Err(b)) => a != b,
                        _ => true,
                    })
        });
        *bindings = Some(Bindings {
            request: request.clone(),
            tier,
            frames,
        });
        Ok(changed)
    }
}
