//! The section 11 shader gate (plan sections 11 and 25, amendment 9):
//! every GLSL source in the repository, the Store, the docs samples, and the
//! generated agent corpus, rendered by the engine's production path and
//! compared with its WebGL2 output.
//!
//! The corpus is `tools/shaders/collect.ts` (outside the repository: the
//! newest `shaders-*` folder in the engine corpus folder with a
//! `webgl-metal/status.json`, or `PM_SHADER_GATE_CORPUS`), with references
//! from `tools/effects/webgl.ts --corpus <dir>` (ANGLE on Metal).
//!
//! - **Shader layers and extension fragment layers** render through
//!   `Renderer::render` on a document holding the layer (the plan's uniform
//!   calls from `d.uniforms` or the definition's parameters, the program from
//!   `pm_glsl`, the target, and the composite into the frame), so the
//!   comparison covers the whole path, not just the program.
//! - **Effects and transitions** run through the effects lane's production
//!   passes (`effect_step`/`transition_step`, `Renderer::run_effects` /
//!   `run_transition`).
//! - Programs the native pipeline cannot translate render through the
//!   decision 19 fallback runner: the test writes each
//!   `FallbackRequest` to `<corpus>/fallback/requests/`, and when
//!   `tools/shaders/fallback-runner.ts` (the app's runner in headless
//!   Chromium) has answered it in `<corpus>/fallback/pixels/`, renders the
//!   frame with those pixels and compares it like any other.
//!
//! G-E1 (docs/engine/gate-policy.md): the native translation executes the
//! author's formula at f32 per GLSL ES, so it is judged against WebGL by an
//! f32-justified bound at every pixel: ≤ 1 CV on 8-bit codes, and for
//! values above 1.0 at most 1.5 binary16 steps (2⁻¹⁰ relative) per stored
//! pass: the engine stores the nearest half where Metal's WebGL truncates
//! (one step), plus a sub-ulp f32 difference (half a step). Pixels where WebGL
//! itself moves past that bound under a one-step change of its input or
//! parameters or coordinates (`webgl.ts --perturb-input`, `--perturb`,
//! `--perturb -1`, `--perturb-geometry`, `--perturb-storage`) are
//! ill-conditioned in WebGL: neither renderer is right there, so they
//! are counted, not judged. Compile and refusal verdicts stay exact. This
//! offline gate replaces the app's first-use native/ANGLE probe (E-R1).
//! Without a corpus the test reports and passes vacuously.

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pm_eval::JsValue;
use pm_gpu::Target;
use pm_protocol::color::Precision;
use pm_protocol::common::{Seq, Size, Ticks};
use pm_protocol::definitions::{DefinitionSetSnapshot, DefinitionSnapshot};
use pm_protocol::error::ErrorCode;
use pm_render::effects::Status;
use pm_render::effects::plan::{effect_step, transition_step};
use pm_render::shader_layers::{FallbackPixels, FallbackRequest};
use pm_render::{Feature, MapResolver, RenderError, RenderRequest, Renderer};
use serde_json::{Value, json};

fn corpus_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PM_SHADER_GATE_CORPUS") {
        return Some(PathBuf::from(p));
    }
    let base = PathBuf::from(std::env::var_os("HOME")?)
        .join("Library/Application Support/Powermove Engine Corpus");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&base)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("shaders-"))
                && p.join("webgl-metal/status.json").exists()
        })
        .collect();
    dirs.sort();
    dirs.pop()
}

fn read_f16(path: &Path) -> Option<Vec<u16>> {
    let bytes = std::fs::read(path).ok()?;
    Some(
        bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect(),
    )
}

fn upload(renderer: &Renderer, width: u32, height: u32, halfs: &[u16]) -> Target {
    let target = Target::new(
        renderer.device(),
        "gate input",
        width,
        height,
        wgpu::TextureFormat::Rgba16Float,
        pm_gpu::target::working_usage(),
    );
    let bytes: Vec<u8> = halfs.iter().flat_map(|h| h.to_le_bytes()).collect();
    renderer.queue().write_texture(
        target.texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 8),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    target
}

fn js(v: &Value) -> Option<JsValue> {
    match v {
        Value::Null => None,
        Value::Bool(b) => Some(JsValue::Bool(*b)),
        Value::Number(n) => Some(JsValue::Number(n.as_f64().unwrap_or(f64::NAN))),
        Value::String(s) => Some(JsValue::from(s.as_str())),
        _ => Some(JsValue::Undefined),
    }
}

/// One binary16 step relative to the value: the storage bound of the
/// RGBA16F targets for values above 1.0.
const HALF_STEP: f64 = 1.0 / 1024.0;

/// Binary16 steps one stored pass may differ by: the nearest half against
/// WebGL's truncation (one step) and a sub-ulp f32 difference (half a step).
const STEPS_PER_STORE: f64 = 1.5;

/// G-E1 per pixel: `engine` against `reference`. Pixels in `unstable` are
/// counted apart, never judged.
#[derive(Debug, Clone, Copy, Default)]
struct Metrics {
    /// Judged pixels past the bound.
    over: usize,
    /// The largest 8-bit difference at a judged pixel.
    max_cv: u32,
    /// The largest relative difference above 1.0 at a judged pixel, in
    /// binary16 steps.
    max_half_steps: f64,
    /// Pixels WebGL itself moves at (not judged).
    unstable: usize,
    pixels: usize,
    /// WebGL against its own perturbed runs, the largest over the runs at
    /// every pixel: (pixels past the bound, largest 8-bit difference).
    spread: (usize, u32),
}

impl Metrics {
    fn pass(&self) -> bool {
        self.over == 0
    }

    /// An ill-conditioned program (E-R2): WebGL itself moves visibly under a
    /// one-step change of its input, parameters, coordinates or storage, and
    /// native differs from WebGL at no more pixels than WebGL differs from
    /// itself. The size of a difference there is not a measure: a chaotic
    /// program (a bisection on bilinear alpha) jumps by whole steps either way.
    fn within_webgl_spread(&self) -> bool {
        self.spread.1 > 1 && self.over <= self.spread.0
    }
}

/// Whether two pixels differ past G-E1's bound for a program of `stores`
/// stored passes, and by how much (8-bit codes, half steps above 1.0). NaN
/// on one side is past it.
fn differs(e: &[f32], r: &[f32], stores: u32) -> (bool, u32, f64) {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
    let (mut cv, mut steps) = (0u32, 0.0f64);
    for c in 0..4 {
        if e[c].is_nan() || r[c].is_nan() {
            if e[c].is_nan() != r[c].is_nan() {
                return (true, 255, f64::INFINITY);
            }
            continue;
        }
        cv = cv.max((q(e[c]) - q(r[c])).unsigned_abs());
        if r[c].abs() > 1.0 || e[c].abs() > 1.0 {
            let rel = f64::from((e[c] - r[c]).abs()) / f64::from(r[c].abs().max(1.0));
            steps = steps.max(rel / HALF_STEP);
        }
    }
    let bound = STEPS_PER_STORE * f64::from(stores.max(1));
    (cv > 1 || steps > bound + 1e-9, cv, steps)
}

fn halves(reference: &[u16]) -> Vec<f32> {
    reference
        .iter()
        .map(|h| pm_gpu::readback::half_to_f32(*h))
        .collect()
}

/// WebGL's output for one variant (GL row order) and what its perturbed
/// runs say about it.
struct WebGl {
    reference: Vec<f32>,
    /// The same program with unrounded RGBA32F intermediates
    /// (`--perturb-storage`): WebGL at a higher storage precision.
    precise: Option<Vec<f32>>,
    /// Pixels WebGL moves at under a perturbation, grown by one pixel.
    unstable: Option<Vec<bool>>,
    /// WebGL against its perturbed runs at every pixel, the largest over the
    /// runs: counted the way `compare` counts the engine.
    spread: (usize, u32),
}

/// `engine` top-down f32 against WebGL (`flip`: the engine's rows are
/// top-down) for a program of `stores` stored passes. A pixel is within
/// G-E1 when it is within the bound of WebGL at either storage precision.
fn compare(engine: &[f32], webgl: &WebGl, width: usize, flip: bool, stores: u32) -> Metrics {
    let height = webgl.reference.len() / 4 / width;
    let mut m = Metrics {
        pixels: webgl.reference.len() / 4,
        spread: webgl.spread,
        ..Default::default()
    };
    for y in 0..height {
        let ry = if flip { height - 1 - y } else { y };
        for x in 0..width {
            let at = ry * width + x;
            if webgl.unstable.as_ref().is_some_and(|u| u[at]) {
                m.unstable += 1;
                continue;
            }
            let e = &engine[(y * width + x) * 4..][..4];
            let (past, cv, steps) = differs(e, &webgl.reference[at * 4..][..4], stores);
            let past = past
                && webgl
                    .precise
                    .as_ref()
                    .is_none_or(|p| differs(e, &p[at * 4..][..4], stores).0);
            m.over += usize::from(past);
            if past {
                m.max_cv = m.max_cv.max(cv);
                m.max_half_steps = m.max_half_steps.max(steps);
            }
        }
    }
    m
}

fn ch(v: Value) -> Value {
    json!({ "v": v, "kf": [], "expr": null })
}

/// A document of one layer in a `w` × `h` composition at `fps`, as the app
/// stores it.
fn layer_document(
    case: &Value,
    variant: &Value,
    definition_id: Option<&str>,
) -> pm_model::Document {
    let (w, h) = (
        variant["width"].as_u64().unwrap(),
        variant["height"].as_u64().unwrap(),
    );
    let from = variant["from"].as_f64().unwrap();
    let dur = variant["dur"].as_f64().unwrap();
    let fps = variant["fps"].as_f64().unwrap();
    let d = match case["kind"].as_str().unwrap() {
        "shader" => {
            let uniforms: serde_json::Map<String, Value> = variant["uniforms"]
                .as_object()
                .map(|u| u.iter().map(|(k, v)| (k.clone(), ch(v.clone()))).collect())
                .unwrap_or_default();
            json!({ "code": case["code"], "uniforms": uniforms })
        }
        _ => {
            let definition = &case["definition"];
            let mut params = serde_json::Map::new();
            for (spec, value) in definition["params"]
                .as_array()
                .unwrap()
                .iter()
                .zip(variant["params"].as_array().unwrap())
            {
                if !value.is_null() {
                    params.insert(spec["key"].as_str().unwrap().into(), ch(value.clone()));
                }
            }
            json!({ "definition": definition_id.unwrap(), "version": definition["version"], "params": params, "data": {} })
        }
    };
    let transform = json!({
        "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(0)),
        "position.y": ch(json!(0)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
        "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
    });
    let layer = json!({
        "id": "L1", "type": case["kind"].as_str().map(|k| if k == "shader" { "shader" } else { "extension" }),
        "name": "gate", "from": from, "dur": dur, "on": true, "lock": false, "threeD": false, "solo": false,
        "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
        "p": transform, "fx": [], "transitionIn": null, "transitionOut": null, "masks": [], "d": d
    });
    let doc = json!({
        "id": "Pgate", "name": "gate", "w": w, "h": h, "fps": fps, "dur": from + dur + 1.0, "bg": "#000000",
        "backgroundFill": { "type": "none", "angle": 0, "stops": [{ "id": "s1", "color": "#000000", "position": 0 }] },
        "compId": "Pgate-root", "compName": "gate", "markers": [], "work": [0, from + dur], "params": {},
        "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "layers": [layer], "assets": {}, "comps": {}
    });
    let value = pm_model::json::parse(&doc.to_string()).unwrap();
    pm_model::Document::new(Some(value), 1)
}

fn ticks(seconds: f64) -> Ticks {
    Ticks::new((seconds * pm_time::TICKS_PER_SECOND as f64).round() as i64).unwrap()
}

#[derive(Default, serde::Serialize)]
struct Tally {
    variants: usize,
    matched: usize,
    via_fallback: usize,
    both_refuse: usize,
    awaiting_runner: Vec<String>,
    failures: Vec<String>,
    known: Vec<String>,
    /// Matched variants whose WebGL draw raised a GL error (both show the
    /// cleared target); listed for the record, counted as matches.
    draw_error_matched: Vec<String>,
    /// Pixels WebGL itself moves at, over all variants (not judged).
    unstable_pixels: usize,
    /// Variants of ill-conditioned programs whose native output differs
    /// from WebGL within WebGL's own spread (E-R2).
    ill_conditioned: Vec<String>,
}

/// Probes and cases whose WebGL and engine answers differ for a documented
/// reason (S3.md, "Coverage sweep"; docs/engine/shaders.md).
fn known_open(id: &str) -> Option<&'static str> {
    if matches!(
        id,
        "generated-s3/ub-pow-normalize-edge"
            | "generated-s3/ub-texel-fetch-out-of-range"
            | "generated-s3/ub-divide-by-zero"
    ) {
        return Some("undefined behavior in GL");
    }
    None
}

enum Outcome {
    Rendered(Vec<f32>),
    Pending(Vec<FallbackRequest>),
    Refused(String),
    Error(String),
}

fn render_layer(renderer: &mut Renderer, doc: &pm_model::Document, variant: &Value) -> Outcome {
    let size = Size::new(
        variant["width"].as_u64().unwrap() as u32,
        variant["height"].as_u64().unwrap() as u32,
    );
    let mut request = RenderRequest::sized(size, Precision::Float16);
    request.background = false;
    let time = ticks(variant["time"].as_f64().unwrap());
    match renderer.render(doc, "Pgate-root", time, &request, &MapResolver::default()) {
        Ok(frame) => {
            if let Some(f) = frame.failures.first() {
                return Outcome::Refused(f.message.clone());
            }
            Outcome::Rendered(renderer.read_rgba(&frame).unwrap())
        }
        Err(RenderError::ShaderFallbackPending(requests)) => Outcome::Pending(requests),
        Err(e) => Outcome::Error(e.to_string()),
    }
}

/// The answers of the app's runner (`tools/shaders/fallback-runner.ts
/// --fallback <corpus>/fallback-effects`) for effects and transitions the
/// engine cannot translate (E-K6): a request without an answer is written
/// for the tool and fails by name. No placeholder can pass a transparent
/// variant.
struct EffectRunner {
    dir: PathBuf,
    missing: std::sync::Mutex<usize>,
}

impl pm_render::shader_layers::FallbackHost for EffectRunner {
    fn run(
        &self,
        job: &pm_render::shader_layers::FallbackJob,
    ) -> Result<pm_render::shader_layers::FallbackPixels, pm_render::shader_layers::FallbackFailure>
    {
        let r = &job.request;
        if let Some(rgba16f) = read_f16(&self.dir.join("pixels").join(format!("{}.f16", r.key))) {
            return Ok(pm_render::shader_layers::FallbackPixels {
                rgba32f: None,
                width: r.width,
                height: r.height,
                rgba16f,
            });
        }
        let requests = self.dir.join("requests");
        std::fs::create_dir_all(&requests).unwrap();
        for (i, t) in job.textures.iter().enumerate() {
            let bytes: Vec<u8> = t.rgba16f.iter().flat_map(|v| v.to_le_bytes()).collect();
            std::fs::write(requests.join(format!("{}-{i}.f16", r.key)), bytes).unwrap();
        }
        std::fs::write(
            requests.join(format!("{}.json", r.key)),
            serde_json::to_vec(r).unwrap(),
        )
        .unwrap();
        *self.missing.lock().unwrap() += 1;
        Err(pm_render::shader_layers::FallbackFailure {
            reason: "unavailable".into(),
            message: "the gate has not captured this ANGLE request yet".into(),
        })
    }
}

#[test]
fn every_shader_matches_webgl() {
    let Some(dir) = corpus_dir() else {
        eprintln!(
            "no shader gate corpus with WebGL references; skipped (run tools/shaders/collect.ts and tools/effects/webgl.ts --corpus)"
        );
        return;
    };
    let corpus: Value =
        serde_json::from_slice(&std::fs::read(dir.join("corpus.json")).unwrap()).unwrap();
    let status: Value =
        serde_json::from_slice(&std::fs::read(dir.join("webgl-metal/status.json")).unwrap())
            .unwrap();
    let requests_dir = dir.join("fallback/requests");
    let pixels_dir = dir.join("fallback/pixels");
    std::fs::create_dir_all(&requests_dir).unwrap();
    let mut renderer = Renderer::headless().expect("a GPU");
    let mut images: BTreeMap<String, Target> = BTreeMap::new();
    for image in corpus["images"].as_array().unwrap() {
        let (w, h) = (
            image["width"].as_u64().unwrap() as u32,
            image["height"].as_u64().unwrap() as u32,
        );
        let halfs = read_f16(&dir.join(image["file"].as_str().unwrap())).unwrap();
        images.insert(
            image["name"].as_str().unwrap().into(),
            upload(&renderer, w, h, &halfs),
        );
    }
    // Every definition under a unique id (Store releases of one extension
    // share theirs), with the built-ins.
    let cases = corpus["cases"].as_array().unwrap();
    let user_setups = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library/Application Support/Powermove Engine Corpus/user-setups"));
    let present = user_setups
        .as_ref()
        .and_then(|root| std::fs::read_dir(root).ok())
        .is_some_and(|dirs| {
            dirs.filter_map(Result::ok)
                .any(|d| d.path().join("extensions").is_dir())
        });
    if present {
        assert!(
            cases
                .iter()
                .any(|c| c["group"].as_str() == Some("user-setups")),
            "private user-setups corpus is present but absent from this shader capture; rerun tools/shaders/collect.ts through the real extension loader"
        );
    } else {
        eprintln!("SKIP user-setups: private corpus directory with extension sources is absent");
    }

    let mut definitions: Vec<Option<DefinitionSnapshot>> = Vec::new();
    let mut set = pm_render::all_builtin_definitions();
    for (n, case) in cases.iter().enumerate() {
        if case["definition"].is_null() {
            definitions.push(None);
            continue;
        }
        let mut def: DefinitionSnapshot =
            serde_json::from_value(case["definition"].clone()).unwrap();
        def.id = format!("gate.{n}").as_str().into();
        set.definitions.push(def.clone());
        definitions.push(Some(def));
    }
    set.generation = Seq::new(1).unwrap();
    // Effects and transitions the engine cannot translate (E-K6): a second
    // renderer with the runner's answers as its host, so the layers'
    // fallback path above is unchanged.
    let effect_runner = std::sync::Arc::new(EffectRunner {
        dir: dir.join("fallback-effects"),
        missing: std::sync::Mutex::new(0),
    });
    let mut via_runner = Renderer::headless().expect("a GPU");
    via_runner.set_fallback_host(Some(effect_runner.clone()));
    let _ = via_runner.set_definitions(&set).unwrap();
    let mut runner_images: BTreeMap<String, Target> = BTreeMap::new();
    for image in corpus["images"].as_array().unwrap() {
        let (w, h) = (
            image["width"].as_u64().unwrap() as u32,
            image["height"].as_u64().unwrap() as u32,
        );
        let halfs = read_f16(&dir.join(image["file"].as_str().unwrap())).unwrap();
        runner_images.insert(
            image["name"].as_str().unwrap().into(),
            upload(&via_runner, w, h, &halfs),
        );
    }
    let _ = renderer
        .set_definitions(&DefinitionSetSnapshot { ..set })
        .unwrap();
    let filter = std::env::var("PM_SHADER_FILTER").ok();
    let mut programs: BTreeMap<String, Tally> = BTreeMap::new();
    let mut requests_written = 0;
    for (case, def) in cases.iter().zip(&definitions) {
        let id = case["id"].as_str().unwrap();
        if filter.as_deref().is_some_and(|f| !id.contains(f)) {
            continue;
        }
        let shown = if case["private"].as_bool() == Some(true) {
            "local (private)"
        } else {
            id
        };
        let kind = case["kind"].as_str().unwrap();
        let tally = programs.entry(id.to_owned()).or_default();
        let webgl = &status["cases"][id];
        let webgl_links = webgl["compiled"].as_bool() == Some(true);
        let webgl_draws = webgl_links && webgl["glError"].is_null();
        let variants = case["variants"].as_array().unwrap();
        tally.variants += variants.len();
        let folder = id.replace('/', "__");
        // Effects and transitions: the effects lane's production passes.
        if matches!(kind, "effect" | "transition") {
            let def = def.as_ref().unwrap();
            let dkind = def.kind;
            let ready = matches!(
                renderer.effects().status(dkind, def.id.as_str()),
                Some(Status::Ready)
            );
            // A program the native pipeline cannot translate draws through
            // the runner (decision 19), judged against WebGL like the rest.
            let needs_runner = matches!(
                renderer.effects().status(dkind, def.id.as_str()),
                Some(Status::NeedsFallback(_))
            );
            if !webgl_links || !(ready || needs_runner) {
                match (
                    webgl_draws,
                    renderer.effects().status(dkind, def.id.as_str()),
                ) {
                    (false, Some(Status::Refused(_))) => tally.both_refuse += variants.len(),
                    (w, s) => tally.failures.push(format!(
                        "{shown}: WebGL draws {w}, engine {:?}",
                        s.map(|s| match s {
                            Status::Ready => "ready".to_owned(),
                            Status::Refused(f) | Status::NeedsFallback(f) =>
                                format!("{}: {}", f.stage.name(), f.message),
                        })
                    )),
                }
                continue;
            }
            for variant in variants {
                // Independent variants must request their own reference even
                // when the preceding variant's reference is not captured yet.
                via_runner.effects_mut().fallback = Default::default();
                via_runner.set_shader_fallback_runner(true);
                let name = variant["name"].as_str().unwrap();
                let stores = u32::from(def.flags.passes.max(1));
                let width = variant["width"].as_u64().unwrap() as usize;
                let Some(output) = webgl_reference(&dir, &folder, name, width, stores) else {
                    tally
                        .failures
                        .push(format!("{shown} {name}: no WebGL output"));
                    continue;
                };
                let values: Vec<Option<JsValue>> = variant["params"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(js)
                    .collect();
                let time = variant["time"].as_f64().unwrap();
                let (renderer, images) = if needs_runner {
                    (&mut via_runner, &runner_images)
                } else {
                    (&mut renderer, &images)
                };
                let out = if kind == "effect" {
                    let step = effect_step(def, "fx", &values);
                    let input = &images[variant["input"].as_str().unwrap()];
                    let backdrop = variant["backdrop"].as_str().map(|b| &images[b]);
                    let region = variant["region"]
                        .as_array()
                        .map(|r| (r[0].as_u64().unwrap() as u32, r[1].as_u64().unwrap() as u32));
                    let full = variant["full"]
                        .as_array()
                        .map(|r| (r[0].as_u64().unwrap() as u32, r[1].as_u64().unwrap() as u32))
                        .unwrap();
                    renderer.run_effects(
                        input,
                        backdrop,
                        &[step],
                        time,
                        region,
                        full,
                        Precision::Float16,
                    )
                } else {
                    let step =
                        transition_step(def, true, variant["progress"].as_f64().unwrap(), &values);
                    let from = &images[variant["from_"].as_str().unwrap()];
                    let to = &images[variant["to"].as_str().unwrap()];
                    renderer.run_transition(from, to, &step, time, Precision::Float16)
                };
                match out {
                    Ok(t) => {
                        let pixels = pm_gpu::readback::read_f32(
                            renderer.device(),
                            renderer.queue(),
                            &t.texture,
                        )
                        .unwrap();
                        let width = t.width() as usize;
                        let m = compare(&pixels, &output, width, false, stores);
                        if m.unstable > 0 || !m.pass() {
                            dump_unstable(shown, name, &pixels, &output.reference, width, false);
                        }
                        record(tally, shown, id, name, m, needs_runner);
                    }
                    Err(e) => tally.failures.push(format!("{shown} {name}: {e}")),
                }
            }
            continue;
        }
        // Shader layers and extension fragment layers: the whole render path.
        for variant in variants {
            let name = variant["name"].as_str().unwrap();
            let doc = layer_document(case, variant, def.as_ref().map(|d| d.id.as_str()));
            renderer.set_shader_fallback_runner(false);
            let mut outcome = render_layer(&mut renderer, &doc, variant);
            let mut via_fallback = false;
            if matches!(&outcome, Outcome::Pending(_))
                || matches!(&outcome, Outcome::Error(e) if e.contains("fallback runner"))
            {
                // Not headless-renderable: through the runner.
                renderer.set_shader_fallback_runner(true);
                outcome = render_layer(&mut renderer, &doc, variant);
                if let Outcome::Pending(requests) = &outcome {
                    let mut answered = true;
                    for r in requests {
                        let file = requests_dir.join(format!("{}.json", r.key));
                        if !file.exists() {
                            std::fs::write(&file, serde_json::to_vec(r).unwrap()).unwrap();
                            requests_written += 1;
                        }
                        match read_f16(&pixels_dir.join(format!("{}.f16", r.key))) {
                            Some(px) => renderer.put_fallback_pixels(
                                r.key.clone(),
                                FallbackPixels {
                                    rgba32f: None,
                                    width: r.width,
                                    height: r.height,
                                    rgba16f: px,
                                },
                            ),
                            None => answered = false,
                        }
                    }
                    if answered {
                        outcome = render_layer(&mut renderer, &doc, variant);
                        via_fallback = true;
                    } else {
                        tally.awaiting_runner.push(format!(
                            "{shown} {name}: needs the fallback runner ({}: {})",
                            requests[0].stage, requests[0].message
                        ));
                        continue;
                    }
                }
            }
            match outcome {
                Outcome::Rendered(pixels) => {
                    if !webgl_draws && !webgl_links {
                        tally.failures.push(format!(
                            "{shown} {name}: WebGL does not link it ({}), the engine draws it",
                            webgl["log"].as_str().unwrap_or("link error")
                        ));
                        continue;
                    }
                    // A program WebGL links but whose draw raises a GL error
                    // leaves the cleared target, which is what WebGL's
                    // output holds and what today's compositor shows: the
                    // engine must show the same (judged below like any
                    // other output; a visible difference fails).
                    let width = variant["width"].as_u64().unwrap() as usize;
                    let Some(output) = webgl_reference(&dir, &folder, name, width, 1) else {
                        tally
                            .failures
                            .push(format!("{shown} {name}: no WebGL output"));
                        continue;
                    };
                    let m = compare(&pixels, &output, width, true, 1);
                    if m.unstable > 0 || !m.pass() {
                        dump_unstable(shown, name, &pixels, &output.reference, width, true);
                    }
                    if !webgl_draws && m.pass() {
                        tally.draw_error_matched.push(format!(
                            "{shown} {name}: WebGL's draw raises GL error {}; both show the cleared target",
                            webgl["glError"]
                        ));
                    }
                    record(tally, shown, id, name, m, via_fallback);
                }
                Outcome::Refused(_) if !webgl_draws => tally.both_refuse += 1,
                Outcome::Refused(message) => tally.failures.push(format!(
                    "{shown} {name}: WebGL draws it, the engine refuses: {message}"
                )),
                Outcome::Pending(_) => unreachable!("handled above"),
                Outcome::Error(e) => tally.failures.push(format!("{shown} {name}: {e}")),
            }
        }
    }
    if let Ok(path) = std::env::var("PM_SHADER_GATE_REPORT") {
        let path = PathBuf::from(path);
        assert!(
            path.is_absolute()
                && !path.starts_with(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .ancestors()
                        .nth(3)
                        .unwrap()
                ),
            "gate reports must stay outside the repository"
        );
        let report = serde_json::to_value(&programs).unwrap();
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    let mut groups: BTreeMap<String, Tally> = BTreeMap::new();
    for case in cases {
        let Some(t) = programs.get(case["id"].as_str().unwrap()) else {
            continue;
        };
        let total = groups
            .entry(case["group"].as_str().unwrap().to_owned())
            .or_default();
        total.variants += t.variants;
        total.matched += t.matched;
        total.via_fallback += t.via_fallback;
        total.both_refuse += t.both_refuse;
        total.unstable_pixels += t.unstable_pixels;
        total
            .ill_conditioned
            .extend(t.ill_conditioned.iter().cloned());
        total
            .awaiting_runner
            .extend(t.awaiting_runner.iter().cloned());
        total.failures.extend(t.failures.iter().cloned());
        total.known.extend(t.known.iter().cloned());
        total
            .draw_error_matched
            .extend(t.draw_error_matched.iter().cloned());
    }
    let mut failed = 0;
    eprintln!("shader gate corpus {}", dir.display());
    eprintln!(
        "| Group | Variants | Within G-E1 | via fallback runner | Both refuse | WebGL-unstable px | Awaiting runner | Known open | Failures |"
    );
    for (group, t) in &groups {
        eprintln!(
            "| {group} | {} | {} | {} | {} | {} | {} | {} | {} |",
            t.variants,
            t.matched,
            t.via_fallback,
            t.both_refuse,
            t.unstable_pixels,
            t.awaiting_runner.len(),
            t.known.len(),
            t.failures.len()
        );
        for k in &t.known {
            eprintln!("  known: {k}");
        }
        for k in &t.ill_conditioned {
            eprintln!("  ill-conditioned: {k}");
        }
        for d in &t.draw_error_matched {
            eprintln!("  draw error: {d}");
        }
        for a in &t.awaiting_runner {
            eprintln!("  runner: {a}");
        }
        for f in &t.failures {
            eprintln!("  FAIL: {f}");
        }
        // A variant the runner has not answered is not judged: it fails
        // until `fallback-runner.ts` answers it (nothing passes unverified).
        failed += t.failures.len() + t.awaiting_runner.len();
    }
    if requests_written > 0 {
        eprintln!(
            "{requests_written} fallback requests written to {} (answer them with tools/shaders/fallback-runner.ts)",
            requests_dir.display()
        );
    }
    let effect_requests = *effect_runner.missing.lock().unwrap();
    if effect_requests > 0 {
        failed += effect_requests;
        eprintln!(
            "{effect_requests} effect requests written to {} (answer them with tools/shaders/fallback-runner.ts --fallback <that dir>)",
            effect_runner.dir.display()
        );
    }
    assert_eq!(failed, 0, "{failed} shader gate failures");
}

/// WebGL's output for `folder/name` and its perturbed runs (`webgl.ts
/// --perturb-input`, `--perturb`, `--perturb -1`, `--perturb-geometry`,
/// `--perturb-storage`). A pixel whose output moves past G-E1's bound under
/// one of them is ill-conditioned in WebGL, per program and variant rather
/// than per name; an edge WebGL cannot place to a pixel also shades its
/// antialiased neighbours, so the set grows by one pixel.
fn webgl_reference(
    dir: &Path,
    folder: &str,
    name: &str,
    width: usize,
    stores: u32,
) -> Option<WebGl> {
    let run = |run: &str| {
        read_f16(&dir.join(run).join(folder).join(format!("{name}.f16"))).map(|h| halves(&h))
    };
    let reference = run("webgl-metal")?;
    let pixels = reference.len() / 4;
    let mut moved: Option<Vec<bool>> = None;
    let mut runs = Vec::new();
    for name in [
        "webgl-metal-input-perturbed",
        "webgl-metal-perturbed",
        "webgl-metal-perturbed-1",
        "webgl-metal-geometry-perturbed",
        "webgl-metal-storage-perturbed",
    ] {
        let Some(other) = run(name).filter(|o| o.len() == reference.len()) else {
            continue;
        };
        let m = moved.get_or_insert_with(|| vec![false; pixels]);
        for (i, u) in m.iter_mut().enumerate() {
            *u |= differs(&reference[i * 4..][..4], &other[i * 4..][..4], stores).0;
        }
        runs.push(other);
    }
    let mut spread = (0, 0);
    for other in &runs {
        let (mut over, mut max) = (0, 0);
        for i in 0..pixels {
            let (past, cv, _) = differs(&reference[i * 4..][..4], &other[i * 4..][..4], stores);
            over += usize::from(past);
            max = max.max(if past { cv } else { 0 });
        }
        spread = (spread.0.max(over), spread.1.max(max));
    }
    let unstable = moved.filter(|_| width > 0).map(|m| {
        let height = m.len() / width;
        pm_reference::compare::dilate(&m, width, height, 1)
    });
    Some(WebGl {
        precise: run("webgl-metal-storage-perturbed").filter(|o| o.len() == reference.len()),
        reference,
        unstable,
        spread,
    })
}

/// Side-by-side images of a failing or WebGL-unstable variant for review (amendment
/// 22: escalated with a comparison), written only when
/// `PM_SHADER_GATE_DIFFS` names a folder outside the repository: the
/// engine's output, WebGL's, and the differing pixels in red over a dimmed
/// copy of WebGL's, each opaque over black.
fn dump_unstable(
    shown: &str,
    name: &str,
    engine: &[f32],
    reference: &[f32],
    width: usize,
    flip: bool,
) {
    let Some(dir) = std::env::var_os("PM_SHADER_GATE_DIFFS").map(PathBuf::from) else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let height = engine.len() / 4 / width.max(1);
    // Both top-down: the engine's layer frames are top-down; WebGL's targets
    // (and the effect passes) are in GL row order.
    let row = |y: usize| if flip { height - 1 - y } else { y };
    let opaque = |r: f32, g: f32, b: f32| {
        [
            (r.clamp(0.0, 1.0) * 255.0).round() as u8,
            (g.clamp(0.0, 1.0) * 255.0).round() as u8,
            (b.clamp(0.0, 1.0) * 255.0).round() as u8,
            255,
        ]
    };
    let (mut ours, mut theirs, mut diff) = (Vec::new(), Vec::new(), Vec::new());
    for y in 0..height {
        for x in 0..width {
            let e = &engine[(y * width + x) * 4..][..4];
            let i = (row(y) * width + x) * 4;
            let w = &reference[i..i + 4];
            ours.extend(opaque(e[0], e[1], e[2]));
            theirs.extend(opaque(w[0], w[1], w[2]));
            let differs = e
                .iter()
                .zip(w)
                .any(|(a, b)| ((a.clamp(0.0, 1.0) - b.clamp(0.0, 1.0)) * 255.0).abs() > 1.0);
            diff.extend(if differs {
                [255, 0, 0, 255]
            } else {
                opaque(w[0] * 0.3, w[1] * 0.3, w[2] * 0.3)
            });
        }
    }
    let stem = format!("{}-{name}", shown.replace('/', "_"));
    let (w, h) = (width as u32, height as u32);
    support::write_png(&dir.join(format!("{stem}-engine.png")), w, h, &ours);
    support::write_png(&dir.join(format!("{stem}-webgl.png")), w, h, &theirs);
    support::write_png(&dir.join(format!("{stem}-diff.png")), w, h, &diff);
}

/// Judges one compared variant by G-E1: every pixel WebGL is stable at is
/// within the bound. Only the documented undefined-behavior probes
/// (`known_open`, which are not user content) are listed apart.
fn record(tally: &mut Tally, shown: &str, id: &str, name: &str, m: Metrics, via_fallback: bool) {
    tally.unstable_pixels += m.unstable;
    if m.pass() || m.within_webgl_spread() {
        tally.matched += 1;
        if via_fallback {
            tally.via_fallback += 1;
        }
        if !m.pass() {
            tally.ill_conditioned.push(format!(
                "{shown} {name}: {} px past the bound (max {} CV), within WebGL's own spread ({} px, max {} CV)",
                m.over, m.max_cv, m.spread.0, m.spread.1
            ));
        }
    } else if let Some(why) = known_open(id) {
        tally.known.push(format!(
            "{shown} {name}: {why} ({} px past the bound, max {} CV)",
            m.over, m.max_cv
        ));
    } else {
        tally.failures.push(format!(
            "{shown} {name}: {} of {} px past the bound (max {} CV, {:.2} half steps above 1.0; {} WebGL-unstable px not judged){}",
            m.over,
            m.pixels,
            m.max_cv,
            m.max_half_steps,
            m.unstable,
            if via_fallback { " (fallback runner)" } else { "" },
        ));
    }
}

#[test]
fn g_e1_bounds_stable_pixels_and_counts_unstable_ones() {
    let webgl = |reference: Vec<f32>, unstable: Option<Vec<bool>>| WebGl {
        reference,
        precise: None,
        unstable,
        spread: (0, 0),
    };
    // WebGL: two pixels; the engine is one code off at the first and far off
    // at the second.
    let two = vec![0.5, 0.5, 0.5, 1.0, 0.2, 0.2, 0.2, 1.0];
    let engine = [0.5 + 1.0 / 255.0, 0.5, 0.5, 1.0, 0.6, 0.2, 0.2, 1.0];
    let m = compare(&engine, &webgl(two.clone(), None), 2, false, 1);
    assert_eq!((m.over, m.max_cv), (1, 102));
    assert!(!m.pass());
    // Where WebGL moves against itself the pixel is counted, not judged.
    let m = compare(&engine, &webgl(two.clone(), Some(vec![false, true])), 2, false, 1);
    assert!(m.pass());
    assert_eq!(m.unstable, 1);
    // Within the bound of WebGL with unrounded intermediates passes too.
    let mut precise = webgl(two.clone(), None);
    precise.precise = Some(vec![0.5, 0.5, 0.5, 1.0, 0.6, 0.2, 0.2, 1.0]);
    assert!(compare(&engine, &precise, 2, false, 1).pass());
    // Above 1.0: 1.5 binary16 steps per stored pass (a step at 4.0 is 4/1024).
    let hdr = webgl(vec![4.0, 4.0, 4.0, 1.0], None);
    let at = |steps: f32| [4.0 + steps * 4.0 / 1024.0, 4.0, 4.0, 1.0];
    assert!(compare(&at(1.5), &hdr, 1, false, 1).pass());
    assert!(!compare(&at(2.0), &hdr, 1, false, 1).pass());
    assert!(compare(&at(2.0), &hdr, 1, false, 2).pass());
    assert!(!compare(&at(3.5), &hdr, 1, false, 2).pass());
    // NaN against a value fails.
    let first = webgl(two[..4].to_vec(), None);
    assert!(!compare(&[f32::NAN, 0.5, 0.5, 1.0], &first, 1, false, 1).pass());
    // A failure names its counts; a known undefined-behavior probe stays
    // apart; an ill-conditioned program within WebGL's own spread passes.
    let mut tally = Tally::default();
    let failing = compare(&engine, &webgl(two.clone(), None), 2, false, 1);
    record(&mut tally, "x", "x", "v", failing, false);
    record(
        &mut tally,
        "generated-s3/ub-divide-by-zero",
        "generated-s3/ub-divide-by-zero",
        "v",
        failing,
        false,
    );
    let mut chaotic = webgl(two, None);
    chaotic.spread = (3, 120);
    record(&mut tally, "y", "y", "v", compare(&engine, &chaotic, 2, false, 1), false);
    assert_eq!(
        (tally.matched, tally.known.len(), tally.failures.len(), tally.ill_conditioned.len()),
        (1, 1, 1, 1)
    );
}

#[test]
fn layers_needing_the_runner_are_not_headless_renderable() {
    // A shader layer whose only sampler is a samplerCube: WebGL samples
    // (0, 0, 0, 1) and the bind check refuses it (amendment 9).
    let case = json!({
        "kind": "shader",
        "code": "uniform samplerCube uEnv;\nvoid main(){ fragColor = vec4(texture(uEnv, vec3(uv, 1.0)).rgb + vec3(uv, 0.0), 1.0); }",
    });
    let variant = json!({ "width": 64, "height": 36, "time": 1.0, "from": 0.0, "dur": 4.0, "fps": 30.0, "uniforms": {} });
    let doc = layer_document(&case, &variant, None);
    let issues = pm_render::unsupported(&doc, "Pgate-root");
    assert!(
        issues
            .iter()
            .any(|u| matches!(&u.feature, Feature::ShaderFallback(_))),
        "{issues:?}"
    );
    let mut renderer = Renderer::headless().unwrap();
    match render_layer(&mut renderer, &doc, &variant) {
        Outcome::Error(e) => assert!(e.contains("fallback runner"), "{e}"),
        _ => panic!("expected routing without a runner"),
    }
    renderer.set_shader_fallback_runner(true);
    let Outcome::Pending(requests) = render_layer(&mut renderer, &doc, &variant) else {
        panic!("expected a fallback request");
    };
    // Reported, not only marked (plan section 11): the Shader panel hears
    // that the layer draws through the runner and will not export headless.
    let reports = renderer.layer_reports(&doc, "Pgate-root");
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].code, ErrorCode::NotHeadlessRenderable);
    assert!(
        reports[0].message.contains("WebGL fallback")
            && reports[0].message.contains("headless exports")
            && reports[0].message.contains("bind"),
        "{}",
        reports[0].message
    );
    assert_eq!(requests.len(), 1);
    let r = &requests[0];
    assert_eq!((r.width, r.height), (64, 36));
    assert_eq!(r.stage, "bind");
    assert!(
        r.fragment.ends_with(
            "fragColor = vec4(texture(uEnv, vec3(uv, 1.0)).rgb + vec3(uv, 0.0), 1.0); }"
        )
    );
    assert!(
        r.calls
            .iter()
            .any(|c| c.name == "iFrame" && c.function == "1i" && c.v == [30.0])
    );
    // The runner's pixels (here a constant) are drawn as the layer content.
    let px: Vec<u16> = (0..64 * 36)
        .flat_map(|_| [0x3800u16, 0x3400, 0x3000, 0x3c00])
        .collect();
    renderer.put_fallback_pixels(
        r.key.clone(),
        FallbackPixels {
            rgba32f: None,
            width: 64,
            height: 36,
            rgba16f: px,
        },
    );
    let Outcome::Rendered(frame) = render_layer(&mut renderer, &doc, &variant) else {
        panic!("expected a frame with the runner's pixels");
    };
    assert_eq!(&frame[..4], &[0.5, 0.25, 0.125, 1.0]);
    assert!(frame.chunks_exact(4).all(|p| p == [0.5, 0.25, 0.125, 1.0]));
    // A frame showing the same pixels again draws the copy already on the
    // GPU.
    let Outcome::Rendered(again) = render_layer(&mut renderer, &doc, &variant) else {
        panic!("expected the frame again");
    };
    assert_eq!(again, frame);
    assert_eq!(renderer.shader_state().fallback_uploads(), 1);
}

#[test]
fn user_program_ids_never_receive_the_generated_undefined_behavior_exemption() {
    for id in [
        "local/sub-title",
        "store/hub-effect",
        "local/undefined-behavior",
        "store/ub-pow-normalize-edge",
    ] {
        assert_eq!(known_open(id), None, "{id}");
    }
    assert!(known_open("generated-s3/ub-divide-by-zero").is_some());
}
