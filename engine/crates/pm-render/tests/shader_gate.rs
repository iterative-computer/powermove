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

#[derive(Default)]
struct Tally {
    variants: usize,
    matched: usize,
    via_fallback: usize,
    both_refuse: usize,
    awaiting_runner: Vec<String>,
    failures: Vec<String>,
    known: Vec<String>,
    /// Amendment 9: WebGL's own output moves past the tolerance under a
    /// one-step input or 2^-20 uniform perturbation; judged by review.
    unstable: Vec<String>,
}

/// Probes and cases whose WebGL and engine answers differ for a documented
/// reason (S3.md, "Coverage sweep"; docs/engine/shaders.md).
fn known_open(id: &str) -> Option<&'static str> {
    if id.contains("ub-") || id.contains("undefined-behavior") {
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
    let _ = renderer
        .set_definitions(&DefinitionSetSnapshot { ..set })
        .unwrap();
    let filter = std::env::var("PM_SHADER_FILTER").ok();
    let mut groups: BTreeMap<String, Tally> = BTreeMap::new();
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
        let tally = groups
            .entry(case["group"].as_str().unwrap().into())
            .or_default();
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
            if !webgl_draws || !ready {
                match (
                    webgl_draws,
                    renderer.effects().status(dkind, def.id.as_str()),
                ) {
                    (false, Some(Status::Refused(_))) => tally.both_refuse += variants.len(),
                    (false, Some(Status::Ready)) if webgl_links => {
                        tally.both_refuse += variants.len()
                    }
                    (true, Some(Status::NeedsFallback(f))) => tally.awaiting_runner.push(format!(
                        "{shown}: effect needs the fallback runner ({}: {})",
                        f.stage.name(),
                        f.message.lines().next().unwrap_or_default()
                    )),
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
                        record(tally, shown, id, name, m, false, unstable);
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
            if let Outcome::Error(e) = &outcome
                && e.contains("fallback runner")
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
                    if !webgl_draws {
                        tally.failures.push(format!(
                            "{shown} {name}: WebGL does not draw it ({}), the engine does",
                            webgl["log"].as_str().unwrap_or("draw error")
                        ));
                        continue;
                    }
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
    let mut failed = 0;
    eprintln!("shader gate corpus {}", dir.display());
    eprintln!(
        "| Group | Variants | Match WebGL | via fallback runner | Both refuse | Awaiting runner | WebGL-unstable | Known open | Failures |"
    );
    for (group, t) in &groups {
        eprintln!(
            "| {group} | {} | {} | {} | {} | {} | {} | {} | {} |",
            t.variants,
            t.matched,
            t.via_fallback,
            t.both_refuse,
            t.awaiting_runner.len(),
            t.unstable.len(),
            t.known.len(),
            t.failures.len()
        );
        for u in &t.unstable {
            eprintln!("  WebGL-unstable: {u}");
        }
        for k in &t.known {
            eprintln!("  known: {k}");
        }
        for a in &t.awaiting_runner {
            eprintln!("  runner: {a}");
        }
        for f in &t.failures {
            eprintln!("  FAIL: {f}");
        }
        failed += t.failures.len();
    }
    if requests_written > 0 {
        eprintln!(
            "{requests_written} fallback requests written to {} (answer them with tools/shaders/fallback-runner.ts)",
            requests_dir.display()
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

#[allow(clippy::too_many_arguments)]
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

fn record(
    tally: &mut Tally,
    shown: &str,
    id: &str,
    name: &str,
    m: Metrics,
    via_fallback: bool,
    unstable: Option<String>,
) {
    if !m.pass()
        && let Some(why) = unstable
    {
        tally.unstable.push(format!(
            "{shown} {name}: {} px over 1 CV (max {} CV); WebGL against itself {why}",
            m.over_one, m.max_cv
        ));
        return;
    }
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
            "{shown} {name}: {} px over 1 CV (max {} CV); above-1.0 rule {} px, max {:.2} CV{}",
            m.over_one,
            m.max_cv,
            m.hdr_over_one,
            m.hdr_max_cv,
            if via_fallback {
                " (fallback runner)"
            } else {
                ""
            }
        ));
    }
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
