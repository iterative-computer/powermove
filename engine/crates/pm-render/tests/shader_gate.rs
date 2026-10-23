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
//! Judged by section 25's Standard class on 8-bit values (≤ 1 CV for 99.9%
//! of pixels, ≤ 4 CV for every pixel), plus the same rule on float values
//! scaled by max(1, |reference|) for values above 1.0. Without a corpus the
//! test reports and passes vacuously.

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

/// Section 25, Standard class, on 8-bit values (NaN on one side counts as
/// 255), and the same rule on float values scaled by max(1, |reference|).
#[derive(Debug, Clone, Copy, Default)]
struct Metrics {
    over_one: usize,
    max_cv: u32,
    hdr_over_one: usize,
    hdr_max_cv: f64,
    pixels: usize,
}

impl Metrics {
    fn pass(&self) -> bool {
        let allowed = self.pixels as f64 * 0.001;
        self.max_cv <= 4
            && (self.over_one as f64) <= allowed
            && self.hdr_max_cv <= 4.0
            && (self.hdr_over_one as f64) <= allowed
    }
}

/// `engine` top-down f32 against `reference` binary16 rows in GL order.
fn compare(engine: &[f32], reference: &[u16], width: usize, flip: bool) -> Metrics {
    let q = |v: f32| -> i32 {
        if v.is_nan() {
            -1000
        } else {
            (v.clamp(0.0, 1.0) * 255.0).round() as i32
        }
    };
    let height = reference.len() / 4 / width;
    let mut m = Metrics {
        pixels: reference.len() / 4,
        ..Default::default()
    };
    for y in 0..height {
        let ry = if flip { height - 1 - y } else { y };
        for x in 0..width {
            let a = &engine[(y * width + x) * 4..][..4];
            let b = &reference[(ry * width + x) * 4..][..4];
            let mut worst = 0u32;
            let mut worst_hdr = 0.0f64;
            for c in 0..4 {
                let r = pm_gpu::readback::half_to_f32(b[c]);
                let e = a[c];
                let d = if e.is_nan() != r.is_nan() {
                    255
                } else {
                    (q(e) - q(r)).unsigned_abs()
                };
                worst = worst.max(d);
                let hdr = if e.is_nan() || r.is_nan() {
                    if e.is_nan() == r.is_nan() { 0.0 } else { 255.0 }
                } else {
                    f64::from((e - r).abs()) / f64::from(r.abs().max(1.0)) * 255.0
                };
                worst_hdr = worst_hdr.max(hdr);
            }
            if worst > 1 {
                m.over_one += 1;
            }
            m.max_cv = m.max_cv.max(worst);
            if worst_hdr > 1.0 + 1e-6 {
                m.hdr_over_one += 1;
            }
            m.hdr_max_cv = m.hdr_max_cv.max(worst_hdr);
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
    /// A real user program's runtime parity verdict once its variants ran
    /// (`NativeVerified`, `Runner`, the static seed, or refused by both).
    #[serde(rename = "runtimeVerdict", skip_serializing_if = "Option::is_none")]
    runtime_verdict: Option<String>,
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
/// --fallback <corpus>/fallback-effects`) for effects and transitions WebGL's
/// runner draws (`pm_render::effects::ANGLE_ONLY`): a request without an
/// answer is written for the tool and fails by name. No placeholder can
/// accidentally pass a transparent variant or create a false parity pin.
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

/// A real user program's verdict after its variants ran through the probe:
/// it must end decided (`NativeVerified` or pinned to the runner), refused
/// by both, or on the static ANGLE-only seed; `Unknown` fails the gate.
fn settle_user_verdict(
    tally: &mut Tally,
    case: &Value,
    def: Option<&DefinitionSnapshot>,
    via_runner: &mut Renderer,
    renderer: &mut Renderer,
) {
    let shown = if case["private"].as_bool() == Some(true) {
        "local (private)"
    } else {
        case["id"].as_str().unwrap()
    };
    let frag = case["frag"].as_str().unwrap();
    let verdict = if tally.both_refuse == tally.variants {
        "RefusedByBoth".to_owned()
    } else if matches!(case["kind"].as_str(), Some("effect" | "transition")) {
        if def.is_some_and(|d| via_runner.effects().angle_only(d.kind, d.id.as_str())) {
            "runner (static seed)".to_owned()
        } else {
            format!(
                "{:?}",
                via_runner
                    .effects_mut()
                    .parity
                    .verdict_at_precision(frag, Precision::Float16)
            )
        }
    } else {
        format!(
            "{:?}",
            renderer
                .effects_mut()
                .parity
                .verdict_at_precision(frag, Precision::Float16)
        )
    };
    if verdict == "Unknown" {
        tally.failures.push(format!(
            "{shown}: the runtime parity probe never judged it (Unknown)"
        ));
    }
    tally.runtime_verdict = Some(verdict);
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
    // Effects and transitions WebGL's runner draws (ANGLE-only): a second
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
    // The last user program run: its verdict is settled before the next
    // case can reconfigure the probe.
    let mut last_user: Option<usize> = None;
    for (n, (case, def)) in cases.iter().zip(&definitions).enumerate() {
        if let Some(u) = last_user.take() {
            let tally = programs.get_mut(cases[u]["id"].as_str().unwrap()).unwrap();
            settle_user_verdict(
                tally,
                &cases[u],
                definitions[u].as_ref(),
                &mut via_runner,
                &mut renderer,
            );
        }
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
        // Keep decisions across the variants of one real user program.
        let is_user = case["group"].as_str() == Some("user-setups");
        if is_user {
            last_user = Some(n);
        }
        if !is_user {
            via_runner.effects_mut().parity.configure(None, None);
            renderer.effects_mut().parity.configure(None, None);
        } else if !via_runner.effects().parity.enabled() {
            via_runner
                .effects_mut()
                .parity
                .configure(Some("shader-gate real user programs".into()), None);
            renderer
                .effects_mut()
                .parity
                .configure(Some("shader-gate real user programs".into()), None);
        }
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
                if is_user {
                    via_runner
                        .effects_mut()
                        .parity
                        .recheck(case["frag"].as_str().unwrap());
                }
                let name = variant["name"].as_str().unwrap();
                let Some(reference) = read_f16(
                    &dir.join("webgl-metal")
                        .join(&folder)
                        .join(format!("{name}.f16")),
                ) else {
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
                let user_setup = case["group"].as_str() == Some("user-setups");
                let angle_only = user_setup
                    || needs_runner
                    || renderer.effects().angle_only(dkind, def.id.as_str());
                let (renderer, images) = if angle_only {
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
                        let m = compare(&pixels, &reference, width, false);
                        let unstable = (!m.pass())
                            .then(|| webgl_unstable(&dir, &folder, name, width))
                            .flatten();
                        if unstable.is_some() {
                            dump_unstable(shown, name, &pixels, &reference, width, false);
                        }
                        record(tally, shown, id, name, m, angle_only, unstable);
                    }
                    Err(e) => tally.failures.push(format!("{shown} {name}: {e}")),
                }
            }
            continue;
        }
        // Shader layers and extension fragment layers: the whole render path.
        for variant in variants {
            if is_user {
                renderer
                    .effects_mut()
                    .parity
                    .recheck(case["frag"].as_str().unwrap());
            }
            let name = variant["name"].as_str().unwrap();
            let doc = layer_document(case, variant, def.as_ref().map(|d| d.id.as_str()));
            renderer.set_shader_fallback_runner(is_user);
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
                    let Some(reference) = read_f16(
                        &dir.join("webgl-metal")
                            .join(&folder)
                            .join(format!("{name}.f16")),
                    ) else {
                        tally
                            .failures
                            .push(format!("{shown} {name}: no WebGL output"));
                        continue;
                    };
                    let width = variant["width"].as_u64().unwrap() as usize;
                    let m = compare(&pixels, &reference, width, true);
                    let unstable = (!m.pass())
                        .then(|| webgl_unstable(&dir, &folder, name, width))
                        .flatten();
                    if unstable.is_some() {
                        dump_unstable(shown, name, &pixels, &reference, width, true);
                    }
                    if !webgl_draws && m.pass() {
                        tally.draw_error_matched.push(format!(
                            "{shown} {name}: WebGL's draw raises GL error {}; both show the cleared target",
                            webgl["glError"]
                        ));
                    }
                    record(tally, shown, id, name, m, via_fallback, unstable);
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
    if let Some(u) = last_user.take() {
        let tally = programs.get_mut(cases[u]["id"].as_str().unwrap()).unwrap();
        settle_user_verdict(
            tally,
            &cases[u],
            definitions[u].as_ref(),
            &mut via_runner,
            &mut renderer,
        );
    }
    let verdicts: BTreeMap<&str, usize> = programs
        .values()
        .filter_map(|t| t.runtime_verdict.as_deref())
        .fold(BTreeMap::new(), |mut m, v| {
            *m.entry(v).or_default() += 1;
            m
        });
    if !verdicts.is_empty() {
        eprintln!("user-setups runtime parity verdicts: {verdicts:?}");
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
        "| Group | Variants | Match WebGL | via fallback runner | Both refuse | Awaiting runner | Known open | Failures |"
    );
    for (group, t) in &groups {
        eprintln!(
            "| {group} | {} | {} | {} | {} | {} | {} | {} |",
            t.variants,
            t.matched,
            t.via_fallback,
            t.both_refuse,
            t.awaiting_runner.len(),
            t.known.len(),
            t.failures.len()
        );
        for k in &t.known {
            eprintln!("  known: {k}");
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

/// Whether WebGL's own output for `folder/name` moves past the tolerance
/// under one of the perturbed runs (`webgl.ts --perturb-input`,
/// `--perturb`, `--perturb -1`): the WebGL-unstable class of amendment 9.
fn webgl_unstable(dir: &Path, folder: &str, name: &str, width: usize) -> Option<String> {
    let base = read_f16(
        &dir.join("webgl-metal")
            .join(folder)
            .join(format!("{name}.f16")),
    )?;
    let base: Vec<f32> = base
        .iter()
        .map(|h| pm_gpu::readback::half_to_f32(*h))
        .collect();
    for run in [
        "webgl-metal-input-perturbed",
        "webgl-metal-perturbed",
        "webgl-metal-perturbed-1",
    ] {
        let Some(other) = read_f16(&dir.join(run).join(folder).join(format!("{name}.f16"))) else {
            continue;
        };
        let m = compare(&base, &other, width, false);
        if !m.pass() {
            return Some(format!(
                "{run}: {} px over 1 CV, max {} CV",
                m.over_one, m.max_cv
            ));
        }
    }
    None
}

/// Side-by-side images of a WebGL-unstable variant for review (amendment
/// 22: escalated with a comparison), written only when
/// `PM_SHADER_GATE_DIFFS` names a folder outside the repository: the
/// engine's output, WebGL's, and the differing pixels in red over a dimmed
/// copy of WebGL's, each opaque over black.
fn dump_unstable(
    shown: &str,
    name: &str,
    engine: &[f32],
    reference: &[u16],
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
            let w: Vec<f32> = reference[i..i + 4]
                .iter()
                .map(|h| pm_gpu::readback::half_to_f32(*h))
                .collect();
            ours.extend(opaque(e[0], e[1], e[2]));
            theirs.extend(opaque(w[0], w[1], w[2]));
            let differs = e
                .iter()
                .zip(&w)
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

/// Judges one compared variant. A variant that differs from WebGL fails,
/// whether or not WebGL moves against its own perturbed runs
/// (`webgl_unstable`, given as `unstable`): amendment 22 and decision DEFS-2
/// accept no difference on level 1 content, and a program whose native
/// translation differs draws through the runner (`effects::ANGLE_ONLY`), so
/// the gate must fail to name it. The perturbation result is kept in the
/// failure as the explanation (and `PM_SHADER_GATE_DIFFS` gets the
/// side-by-side). Only the documented undefined-behavior probes
/// (`known_open`, which are not user content) are listed apart.
#[allow(clippy::too_many_arguments)]
fn record(
    tally: &mut Tally,
    shown: &str,
    id: &str,
    name: &str,
    m: Metrics,
    via_fallback: bool,
    unstable: Option<String>,
) {
    if m.pass() {
        tally.matched += 1;
        if via_fallback {
            tally.via_fallback += 1;
        }
    } else if let Some(why) = known_open(id) {
        tally.known.push(format!(
            "{shown} {name}: {why} ({} px over 1 CV, max {} CV)",
            m.over_one, m.max_cv
        ));
    } else {
        tally.failures.push(format!(
            "{shown} {name}: {} px over 1 CV (max {} CV); above-1.0 rule {} px, max {:.2} CV{}{}",
            m.over_one,
            m.max_cv,
            m.hdr_over_one,
            m.hdr_max_cv,
            if via_fallback {
                " (fallback runner)"
            } else {
                ""
            },
            match unstable {
                Some(why) => format!(
                    "; WebGL also moves against itself ({why}): ill-conditioned in WebGL, \
                     so it must draw as WebGL does (add its body fingerprint to \
                     pm_render::effects::ANGLE_ONLY)"
                ),
                None => String::new(),
            }
        ));
    }
}

#[test]
fn a_difference_webgl_is_unstable_under_fails_the_gate() {
    // The perturbation class used to be listed apart as "WebGL-unstable"
    // and passed the gate; it is a failure like any other difference.
    let differs = Metrics {
        over_one: 40,
        max_cv: 9,
        hdr_over_one: 0,
        hdr_max_cv: 0.0,
        pixels: 36_864,
    };
    assert!(!differs.pass());
    let mut tally = Tally::default();
    record(
        &mut tally,
        "store/acme/glass",
        "store/acme/glass",
        "viewport",
        differs,
        false,
        Some("webgl-metal-input-perturbed: 52 px over 1 CV, max 11 CV".into()),
    );
    assert_eq!(tally.matched, 0);
    assert_eq!(tally.failures.len(), 1, "{:?}", tally.failures);
    assert!(
        tally.failures[0].contains("ANGLE_ONLY"),
        "{}",
        tally.failures[0]
    );
    // Through the runner too (an ANGLE-only program the runner drew
    // differently).
    let mut tally = Tally::default();
    record(
        &mut tally,
        "store/acme/glass",
        "store/acme/glass",
        "viewport",
        differs,
        true,
        Some("webgl-metal-perturbed: 3 px over 1 CV, max 2 CV".into()),
    );
    assert_eq!(tally.failures.len(), 1);
    assert_eq!(tally.via_fallback, 0);
    // A match stays a match, an undefined-behavior probe stays apart.
    let same = Metrics {
        over_one: 0,
        max_cv: 0,
        hdr_over_one: 0,
        hdr_max_cv: 0.0,
        pixels: 36_864,
    };
    let mut tally = Tally::default();
    record(&mut tally, "x", "x", "v", same, false, None);
    record(
        &mut tally,
        "generated-s3/ub-divide-by-zero",
        "generated-s3/ub-divide-by-zero",
        "v",
        differs,
        false,
        None,
    );
    assert_eq!(
        (tally.matched, tally.known.len(), tally.failures.len()),
        (1, 1, 0)
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

/// A real user program the runtime probe never judged (left `Unknown`)
/// fails the gate; a judged one, or one both refuse, passes.
#[test]
fn a_user_program_the_probe_never_judged_fails_the_gate() {
    let mut via_runner = Renderer::headless().expect("a GPU");
    let mut renderer = Renderer::headless().expect("a GPU");
    for r in [&mut via_runner, &mut renderer] {
        r.effects_mut()
            .parity
            .configure(Some("gate verdict test".into()), None);
    }
    let effect =
        json!({"id": "user/fx", "kind": "effect", "frag": "never judged", "private": true});
    let mut tally = Tally {
        variants: 2,
        ..Tally::default()
    };
    settle_user_verdict(&mut tally, &effect, None, &mut via_runner, &mut renderer);
    assert_eq!(tally.runtime_verdict.as_deref(), Some("Unknown"));
    assert_eq!(tally.failures.len(), 1, "{:?}", tally.failures);
    assert!(
        tally.failures[0].starts_with("local (private)"),
        "private ids stay out of the output"
    );
    let px = FallbackPixels {
        rgba32f: None,
        width: 1,
        height: 1,
        rgba16f: vec![0; 4],
    };
    via_runner
        .effects_mut()
        .parity
        .judge("working-depth-v1:float16\0judged", &px, &px);
    let judged = json!({"id": "user/ok", "kind": "transition", "frag": "judged"});
    let mut tally = Tally {
        variants: 1,
        ..Tally::default()
    };
    settle_user_verdict(&mut tally, &judged, None, &mut via_runner, &mut renderer);
    assert_eq!(tally.runtime_verdict.as_deref(), Some("NativeVerified"));
    assert!(tally.failures.is_empty());
    let layer = json!({"id": "user/layer", "kind": "layer", "frag": "refused"});
    let mut tally = Tally {
        variants: 3,
        both_refuse: 3,
        ..Tally::default()
    };
    settle_user_verdict(&mut tally, &layer, None, &mut via_runner, &mut renderer);
    assert_eq!(tally.runtime_verdict.as_deref(), Some("RefusedByBoth"));
    assert!(tally.failures.is_empty());
}
