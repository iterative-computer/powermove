//! Effects, transitions, and the coverage sweep against WebGL2 (plan
//! section 11's shader gate; section 25's tolerances; amendment 9).
//!
//! The corpus is `tools/effects/collect.ts` (outside the repository: the
//! newest `glsl-*` capture in the engine corpus folder, or
//! `PM_SHADER_CORPUS`), with WebGL2 references from `tools/effects/webgl.ts`
//! (ANGLE on Metal). Every case runs through the production path: the
//! kernel's definition snapshot, `pm_glsl` translation, the parameter
//! conversion of `effects::plan`, and `Renderer::run_effects` /
//! `run_transition` (the passes `render` uses), in RGBA16F as the
//! compositor. Without the corpus the test reports and passes vacuously.
//!
//! Judged by section 25's Standard class on 8-bit values (≤ 1 CV for 99.9%
//! of pixels, ≤ 4 CV for every pixel), plus, for inputs above 1.0, the same
//! rule on float values scaled by max(1, |reference|) (values above 1.0 are
//! not clamped by legacy GLSL, so an 8-bit comparison alone would hide them).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pm_eval::JsValue;
use pm_gpu::Target;
use pm_protocol::color::Precision;
use pm_protocol::common::Seq;
use pm_protocol::definitions::{DefinitionKind, DefinitionSetSnapshot, DefinitionSnapshot};
use pm_render::Renderer;
use pm_render::effects::Status;
use pm_render::effects::plan::{effect_step, transition_step};

fn corpus_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PM_SHADER_CORPUS") {
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
                .is_some_and(|n| n.starts_with("glsl-"))
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
        "test input",
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

fn js(v: &serde_json::Value) -> Option<JsValue> {
    match v {
        serde_json::Value::Null => None,
        serde_json::Value::Bool(b) => Some(JsValue::Bool(*b)),
        serde_json::Value::Number(n) => Some(JsValue::Number(n.as_f64().unwrap_or(f64::NAN))),
        serde_json::Value::String(s) => Some(JsValue::from(s.as_str())),
        _ => Some(JsValue::Undefined),
    }
}

/// Section 25, Standard class, on 8-bit values (NaN on one side counts as 255).
#[derive(Debug, Clone, Copy, Default)]
struct Metrics {
    over_one: usize,
    max_cv: u32,
    /// The same rule on float values scaled by max(1, |reference|), in CV.
    hdr_over_one: usize,
    hdr_max_cv: f64,
    pixels: usize,
    /// Pixels whose four values equal the reference's binary16 values bit
    /// for bit (`PM_SHADER_STATS`).
    exact: usize,
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

fn compare(engine: &[f32], reference: &[u16]) -> Metrics {
    let q = |v: f32| -> i32 {
        if v.is_nan() {
            -1000
        } else {
            (v.clamp(0.0, 1.0) * 255.0).round() as i32
        }
    };
    let mut m = Metrics {
        pixels: reference.len() / 4,
        ..Default::default()
    };
    for (p, (a, b)) in engine
        .chunks_exact(4)
        .zip(reference.chunks_exact(4))
        .enumerate()
    {
        let _ = p;
        let mut worst = 0u32;
        let mut worst_hdr = 0.0f64;
        if (0..4).all(|c| pm_gpu::readback::f32_to_half(a[c]) == b[c]) {
            m.exact += 1;
        }
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
    m
}

/// Probes whose WebGL and engine answers differ for a known, documented
/// reason (S3.md, "Coverage sweep"; docs/engine/effects.md).
fn known_open(id: &str) -> Option<&'static str> {
    let name = id.rsplit('/').next().unwrap_or(id);
    if id.starts_with("probe/types/ternary")
        || id.contains("ternary-struct")
        || id.contains("ternary-array")
    {
        return Some(
            "ternary on a struct or array: WebGL refuses, the engine accepts (needs expression types)",
        );
    }
    if id.starts_with("probe/ub/") {
        return Some("undefined behavior in GL");
    }
    if name == "output-vec3" {
        return Some(
            "out vec3: GL leaves alpha undefined (ANGLE writes small garbage at some pixels)",
        );
    }
    None
}

#[derive(Default)]
struct Tally {
    variants: usize,
    matched: usize,
    both_refuse: usize,
    /// Variants WebGL draws that run in the fallback runner (decision 19),
    /// compared with WebGL by `fallback_effects_match_webgl`.
    fallback: usize,
    failures: Vec<String>,
    known: Vec<String>,
}

#[test]
fn effects_transitions_and_probes_match_webgl() {
    let Some(dir) = corpus_dir() else {
        eprintln!(
            "no shader corpus with WebGL references; skipped (run tools/effects/collect.ts and webgl.ts)"
        );
        return;
    };
    let corpus: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("corpus.json")).unwrap()).unwrap();
    let status: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("webgl-metal/status.json")).unwrap())
            .unwrap();
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
    let cases = corpus["cases"].as_array().unwrap();
    let definitions: Vec<DefinitionSnapshot> = cases
        .iter()
        .map(|c| serde_json::from_value(c["definition"].clone()).unwrap())
        .collect();
    renderer
        .set_definitions(&DefinitionSetSnapshot {
            generation: Seq::new(1).unwrap(),
            definitions: definitions.clone(),
            missing: vec![],
        })
        .unwrap();
    let filter = std::env::var("PM_SHADER_FILTER").ok();
    // Per-variant measurements, bit-exact pixels included.
    let stats = std::env::var_os("PM_SHADER_STATS").is_some();
    let mut groups: BTreeMap<String, Tally> = BTreeMap::new();
    for (case, def) in cases.iter().zip(&definitions) {
        let id = case["id"].as_str().unwrap();
        if filter.as_deref().is_some_and(|f| !id.contains(f)) {
            continue;
        }
        let tally = groups
            .entry(case["group"].as_str().unwrap().into())
            .or_default();
        let webgl = &status["cases"][id];
        let webgl_links = webgl["compiled"].as_bool() == Some(true);
        let webgl_draws = webgl_links && webgl["glError"].is_null();
        let kind = def.kind;
        let ready = matches!(
            renderer.effects().status(kind, def.id.as_str()),
            Some(Status::Ready)
        );
        let variants = case["variants"].as_array().unwrap();
        tally.variants += variants.len();
        // WebGL links but its draw fails (nothing renders today): the engine
        // must not render it either; it refuses when it builds the pipeline.
        if webgl_links && !webgl_draws && ready {
            let def_input = &images["layer"];
            let step = effect_step(def, "fx", &[]);
            match renderer.run_effects(
                def_input,
                None,
                &[step],
                0.0,
                None,
                (def_input.width(), def_input.height()),
                Precision::Float16,
            ) {
                Err(_) => tally.both_refuse += variants.len(),
                Ok(_) => tally.failures.push(format!(
                    "{id}: WebGL's draw fails ({}), the engine draws",
                    webgl["glError"]
                )),
            }
            continue;
        }
        // Programs WebGL draws that the native pipeline cannot translate
        // run in the app's fallback runner with their textures (decision
        // 19, amendment 21): drawn, compared by `fallback_effects_match_webgl`.
        if webgl_draws
            && matches!(
                renderer.effects().status(kind, def.id.as_str()),
                Some(Status::NeedsFallback(_))
            )
        {
            tally.fallback += variants.len();
            continue;
        }
        if !webgl_draws || !ready {
            let reason = known_open(id);
            match (webgl_draws, ready, reason) {
                (false, false, _) => tally.both_refuse += variants.len(),
                (_, _, Some(why)) => tally.known.push(format!("{id}: {why}")),
                (true, false, None) => tally.failures.push(format!(
                    "{id}: WebGL draws, the engine refuses: {:?}",
                    renderer.effects().status(kind, def.id.as_str())
                )),
                (false, true, None) => tally.failures.push(format!(
                    "{id}: WebGL refuses ({}), the engine accepts",
                    webgl["log"].as_str().map_or_else(
                        || format!("draw error {}", webgl["glError"]),
                        |l| l.lines().next().unwrap_or("").to_owned()
                    )
                )),
                (true, true, _) => unreachable!(),
            }
            continue;
        }
        for variant in variants {
            let name = variant["name"].as_str().unwrap();
            let reference = dir
                .join("webgl-metal")
                .join(id.replace('/', "__"))
                .join(format!("{name}.f16"));
            let Some(reference) = read_f16(&reference) else {
                tally.failures.push(format!("{id} {name}: no WebGL output"));
                continue;
            };
            let values: Vec<Option<JsValue>> = variant["params"]
                .as_array()
                .unwrap()
                .iter()
                .map(js)
                .collect();
            let time = variant["time"].as_f64().unwrap();
            let out = match kind {
                DefinitionKind::Effect => {
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
                }
                _ => {
                    let step =
                        transition_step(def, true, variant["progress"].as_f64().unwrap(), &values);
                    let from = &images[variant["from"].as_str().unwrap()];
                    let to = &images[variant["to"].as_str().unwrap()];
                    renderer.run_transition(from, to, &step, time, Precision::Float16)
                }
            };
            let out = match out {
                Ok(t) => t,
                Err(e) => {
                    tally.failures.push(format!("{id} {name}: {e}"));
                    continue;
                }
            };
            let pixels =
                pm_gpu::readback::read_f32(renderer.device(), renderer.queue(), &out.texture)
                    .unwrap();
            let m = compare(&pixels, &reference);
            if stats {
                eprintln!(
                    "{id} {name}: exact {:.3}% ({} of {} px), {} px over 1 CV, max {} CV, above-1.0 max {:.3} CV",
                    100.0 * m.exact as f64 / m.pixels.max(1) as f64,
                    m.exact,
                    m.pixels,
                    m.over_one,
                    m.max_cv,
                    m.hdr_max_cv
                );
            }
            if m.pass() {
                tally.matched += 1;
            } else if let Some(why) = known_open(id) {
                tally.known.push(format!(
                    "{id} {name}: {why} ({} px over 1 CV, max {} CV)",
                    m.over_one, m.max_cv
                ));
            } else {
                tally.failures.push(format!(
                    "{id} {name}: {} px over 1 CV (max {} CV); above-1.0 rule {} px, max {:.2} CV",
                    m.over_one, m.max_cv, m.hdr_over_one, m.hdr_max_cv
                ));
            }
        }
    }
    let mut failed = 0;
    eprintln!("shader corpus {}", dir.display());
    eprintln!(
        "| Group | Variants | Match WebGL | Fallback runner | Both refuse | Known open | Failures |"
    );
    for (group, t) in &groups {
        eprintln!(
            "| {group} | {} | {} | {} | {} | {} | {} |",
            t.variants,
            t.matched,
            t.fallback,
            t.both_refuse,
            t.known.len(),
            t.failures.len()
        );
        for k in &t.known {
            eprintln!("  known: {k}");
        }
        for f in &t.failures {
            eprintln!("  FAIL: {f}");
        }
        failed += t.failures.len();
    }
    assert_eq!(failed, 0, "{failed} shader parity failures");
}

/// Amendment 9's WebGL-unstable class: WebGL against itself with every
/// input texel one binary16 step away from zero (`webgl.ts
/// --perturb-input`). A variant whose output moves past the Standard
/// tolerance under that is ill-conditioned in WebGL: any renderer whose
/// input differs at rounding level (pm-render's layer buffer is within
/// 1 CV of today's, not bit-identical) differs there too. Reports every
/// such built-in variant.
#[test]
fn webgl_instability_under_a_one_step_input_perturbation() {
    let Some(dir) = corpus_dir() else {
        eprintln!("no shader corpus; skipped");
        return;
    };
    let perturbed = dir.join("webgl-metal-input-perturbed");
    if !perturbed.exists() {
        eprintln!("no input-perturbed WebGL run (webgl.ts --perturb-input); skipped");
        return;
    }
    let corpus: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("corpus.json")).unwrap()).unwrap();
    let mut unstable: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut compared = 0;
    for case in corpus["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let folder = id.replace('/', "__");
        for variant in case["variants"].as_array().unwrap() {
            let name = variant["name"].as_str().unwrap();
            let (Some(a), Some(b)) = (
                read_f16(
                    &dir.join("webgl-metal")
                        .join(&folder)
                        .join(format!("{name}.f16")),
                ),
                read_f16(&perturbed.join(&folder).join(format!("{name}.f16"))),
            ) else {
                continue;
            };
            compared += 1;
            let a: Vec<f32> = a
                .iter()
                .map(|h| pm_gpu::readback::half_to_f32(*h))
                .collect();
            let m = compare(&a, &b);
            if !m.pass() {
                unstable.entry(id.to_owned()).or_default().push(format!(
                    "{name}: {} px over 1 CV, max {} CV",
                    m.over_one, m.max_cv
                ));
            }
        }
    }
    eprintln!("{compared} variants compared with their input-perturbed WebGL output");
    for (id, variants) in &unstable {
        eprintln!("WebGL-unstable: {id}");
        for v in variants {
            eprintln!("  {v}");
        }
    }
    if compared > 0 {
        assert!(
            unstable.contains_key("builtin/effect/posterize"),
            "Posterize is a step function of its input; expected it to move under a one-step perturbation"
        );
        // The built-ins exposed to rounding-level drift are the ones known:
        // a new one needs the same proof as these (bit-exact against WebGL
        // at every corpus parameter set, `PM_SHADER_STATS`), not a list
        // that excuses it.
        let builtins: Vec<&str> = unstable
            .keys()
            .filter_map(|id| id.strip_prefix("builtin/effect/"))
            .collect();
        for id in &builtins {
            assert!(
                EXPOSED_BUILTINS.contains(id),
                "built-in {id} is ill-conditioned in WebGL; prove it bit-exact and add it to EXPOSED_BUILTINS"
            );
        }
    }
}

/// Built-in effects in amendment 9's WebGL-unstable class: today's WebGL
/// output itself moves past the Standard tolerance when its input moves by
/// one binary16 step (Posterize and Levels are step functions at some
/// parameters, Color's hue rotation at saturated extremes). Where the
/// engine's input to them or its arithmetic differed from today's at
/// rounding level, their output differed at isolated pixels. Since
/// 2026-10-19 neither does: their inputs (Level 1 stills as `Rgba8Unorm`)
/// and their programs' built-in calls (`pm_render::glsl`) match today's, and
/// all three are bit-exact against WebGL at every corpus parameter set. No
/// test lists their frames apart; this records which built-ins are exposed
/// if either drifts.
const EXPOSED_BUILTINS: &[&str] = &["posterize", "levels", "color"];

/// The corpus's fallback runner: answers a request with the pixels the
/// app's runner drew for it in Chromium (`tools/shaders/fallback-runner.ts
/// --fallback <dir>`), or writes the request and its textures for that
/// tool and answers with transparent pixels (counted as waiting, so the
/// test fails asking for the tool; a failure would turn the program off for
/// the variants after it and leave their requests unwritten).
struct CorpusRunner {
    dir: PathBuf,
    missing: std::sync::Mutex<usize>,
}

impl pm_render::shader_layers::FallbackHost for CorpusRunner {
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
        Ok(pm_render::shader_layers::FallbackPixels {
            rgba32f: None,
            width: r.width,
            height: r.height,
            rgba16f: vec![0; r.width as usize * r.height as usize * 4],
        })
    }
}

/// Decision 19 for effects and transitions (amendment 21: drawn, not
/// routed): every case WebGL draws that the native pipeline cannot
/// translate (amendment 9's samplers other than a float `sampler2D`) runs
/// in the app's own fallback runner with its textures, and the result,
/// composed back by the engine, matches the compositor's WebGL output by
/// the same rule as every other case. Two runs: the first writes the
/// requests (`<corpus>/fallback-effects/requests`), the tool answers them
/// in headless Chromium, the second compares.
#[test]
fn fallback_effects_match_webgl() {
    let Some(dir) = corpus_dir() else {
        eprintln!("no shader corpus; skipped");
        return;
    };
    let corpus: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("corpus.json")).unwrap()).unwrap();
    let status: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("webgl-metal/status.json")).unwrap())
            .unwrap();
    let mut renderer = Renderer::headless().expect("a GPU");
    let host = std::sync::Arc::new(CorpusRunner {
        dir: dir.join("fallback-effects"),
        missing: std::sync::Mutex::new(0),
    });
    renderer.set_fallback_host(Some(host.clone()));
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
    let cases = corpus["cases"].as_array().unwrap();
    let definitions: Vec<DefinitionSnapshot> = cases
        .iter()
        .map(|c| serde_json::from_value(c["definition"].clone()).unwrap())
        .collect();
    renderer
        .set_definitions(&DefinitionSetSnapshot {
            generation: Seq::new(1).unwrap(),
            definitions: definitions.clone(),
            missing: vec![],
        })
        .unwrap();
    let (mut variants, mut matched) = (0, 0);
    let mut failures = Vec::new();
    for (case, def) in cases.iter().zip(&definitions) {
        let id = case["id"].as_str().unwrap();
        let webgl = &status["cases"][id];
        let webgl_draws = webgl["compiled"].as_bool() == Some(true) && webgl["glError"].is_null();
        // Programs the native pipeline cannot translate (E-K6).
        let needs = matches!(
            renderer.effects().status(def.kind, def.id.as_str()),
            Some(Status::NeedsFallback(_))
        );
        if !webgl_draws || !needs {
            continue;
        }
        for variant in case["variants"].as_array().unwrap() {
            variants += 1;
            let name = variant["name"].as_str().unwrap();
            let reference = dir
                .join("webgl-metal")
                .join(id.replace('/', "__"))
                .join(format!("{name}.f16"));
            let Some(reference) = read_f16(&reference) else {
                failures.push(format!("{id} {name}: no WebGL output"));
                continue;
            };
            let values: Vec<Option<JsValue>> = variant["params"]
                .as_array()
                .unwrap()
                .iter()
                .map(js)
                .collect();
            let time = variant["time"].as_f64().unwrap();
            let out = match def.kind {
                DefinitionKind::Effect => {
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
                }
                _ => {
                    let step =
                        transition_step(def, true, variant["progress"].as_f64().unwrap(), &values);
                    let from = &images[variant["from"].as_str().unwrap()];
                    let to = &images[variant["to"].as_str().unwrap()];
                    renderer.run_transition(from, to, &step, time, Precision::Float16)
                }
            };
            let out = match out {
                Ok(t) => t,
                Err(e) => {
                    failures.push(format!("{id} {name}: {e}"));
                    continue;
                }
            };
            let pixels =
                pm_gpu::readback::read_f32(renderer.device(), renderer.queue(), &out.texture)
                    .unwrap();
            let m = compare(&pixels, &reference);
            if std::env::var_os("PM_SHADER_STATS").is_some() {
                eprintln!(
                    "{id} {name} (runner): exact {:.3}% ({} of {} px), {} px over 1 CV, max {} CV",
                    100.0 * m.exact as f64 / m.pixels.max(1) as f64,
                    m.exact,
                    m.pixels,
                    m.over_one,
                    m.max_cv
                );
            }
            if m.pass() {
                matched += 1;
            } else {
                failures.push(format!(
                    "{id} {name}: {} px over 1 CV (max {} CV)",
                    m.over_one, m.max_cv
                ));
            }
        }
    }
    let missing = *host.missing.lock().unwrap();
    eprintln!(
        "fallback runner cases: {variants} variants, {matched} match WebGL, {} failures, {missing} requests waiting",
        failures.len()
    );
    for f in &failures {
        eprintln!("  {f}");
    }
    assert_eq!(
        missing,
        0,
        "{missing} requests written to {}: run `pm-electron-lock bun engine/crates/pm-render/tools/shaders/fallback-runner.ts --fallback <that dir>` and run this test again",
        host.dir.display()
    );
    assert!(variants > 0, "no case needs the fallback runner");
    assert!(
        failures.is_empty(),
        "{} fallback cases differ from WebGL",
        failures.len()
    );
}
