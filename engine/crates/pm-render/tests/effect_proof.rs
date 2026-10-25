//! L6's effect proof (docs/engine/gate-policy.md, "Effects"): every corpus
//! layer with a built-in effect chain the reference models, drawn with and
//! without its effects at float depth; the chain applied in f64 by
//! `pm_reference::effects` to the engine's own render without effects must
//! equal the engine's render with them. The layer's content (resampled
//! images, text) is other gates' proof; this one proves the effect formulas
//! and their parameters on every layer type the corpus draws them on.
//!
//! Runs on the main thread (the CoreText font registry for text layers).
//! `PM_ENGINE_CORPUS` and `PM_RENDER_ONLY` as `tests/corpus.rs`;
//! `PM_L6_EFFECT_PROOF=<file>` writes one JSON row per proved chain.

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::{RenderRequest, Renderer, all_builtin_definitions};
use support::*;

fn unit32(v: &[f32]) -> Vec<f64> {
    v.iter().map(|c| f64::from(*c)).collect()
}

/// Per-pixel error bound of the effect proof, in 8-bit code values: the
/// GPU's bilinear filter weighs texels with fixed-point fractions (8 bits
/// on this hardware, an error up to 1/512 of a texel difference per
/// sample), summed over a chain's passes.
const EFFECT_PROOF_CV: f64 = 1.0;

/// L6's effect proof (gate-policy.md, "Effects"): for every layer with a
/// built-in effect chain the reference models, the chain applied in f64 by
/// `pm_reference::effects` to the engine's own render of the layer without
/// its effects equals the engine's render with them, at float depth. The
/// layer's content (resampled images, text) is other lanes' proof; this
/// one proves the effect formulas and their parameters on every layer
/// type the corpus draws them on.
fn prove_effect_chains(
    renderer: &mut Renderer,
    project: &Project,
    sample: &Sample,
    rows: &mut Option<std::fs::File>,
) -> (Vec<String>, usize) {
    use pm_eval::{Comp, Evaluator, Scope, node};
    use std::io::Write;
    let comp = Comp::resolve(&project.document, &project.composition).expect("composition");
    let seconds = comp.seconds_at(ticks(sample.time).get());
    let mut evaluator =
        Evaluator::new().with_expressions(std::sync::Arc::new(pm_render::expr::PmExprEngine));
    let mut scope = Scope::new(&mut evaluator, &comp, &comp);
    let mut failures = Vec::new();
    let mut proved = 0;
    let mut request =
        RenderRequest::sized(Size::new(sample.width, sample.height), Precision::Float32);
    request.output_kind = pm_render::OutputKind::Final;
    request.background = false;
    let (w, h) = (sample.width as usize, sample.height as usize);
    for (index, layer) in comp.layers().iter().enumerate() {
        let ty = pm_render::js::member_string(layer, "type").unwrap_or_default();
        if matches!(
            ty.as_str(),
            "null" | "audio" | "adjustment" | "group" | "precomp"
        ) || node::truthy(node::member(Some(layer), "group"))
            || !scope.active(layer, seconds).expect("layer active")
            || scope.is_3d_layer(layer).expect("3D property")
            || !scope
                .has_renderable_effects(layer, seconds)
                .expect("effects")
        {
            continue;
        }
        let Some(chain) =
            reference::effect_chain(&mut scope, renderer.effects(), layer, seconds).expect("chain")
        else {
            continue;
        };
        let label = format!("{} t={} layer={index} ({ty})", project.name, sample.time);
        let mut draw = |effects: bool| {
            let doc = reference::isolated_effect_layer(&project.document, index, effects);
            let frame = renderer
                .render(
                    &doc,
                    &project.composition,
                    ticks(sample.time),
                    &request,
                    &project.resolver,
                )
                .map_err(|e| format!("{label}: render: {e}"))?;
            if frame.color.format() != wgpu::TextureFormat::Rgba32Float
                || !frame.failures.is_empty()
            {
                return Err(format!(
                    "{label}: float storage or layer failure: {:?}",
                    frame.failures
                ));
            }
            Ok(unit32(&renderer.read_rgba(&frame).expect("readback")))
        };
        let (input, output) = match (draw(false), draw(true)) {
            (Ok(i), Ok(o)) => (i, o),
            (Err(e), _) | (_, Err(e)) => {
                failures.push(e);
                continue;
            }
        };
        let layer_px: Vec<(pm_reference::composite::Rgba, bool)> = input
            .chunks_exact(4)
            .map(|p| ([p[0], p[1], p[2], p[3]], false))
            .collect();
        let Some((image, open, _)) =
            reference::apply_effects(&chain, &layer_px, w, h, seconds, Precision::Float32)
        else {
            continue;
        };
        let mut errors = Vec::with_capacity(w * h);
        for i in 0..w * h {
            let e = [
                output[i * 4],
                output[i * 4 + 1],
                output[i * 4 + 2],
                output[i * 4 + 3],
            ];
            let want = match &open[i] {
                Some(o) => o.nearest(e),
                None => image.at(i % w, i / w),
            };
            let cv = (0..4)
                .map(|k| (e[k] - want[k]).abs() * 255.0)
                .fold(0.0, f64::max);
            errors.push(cv);
        }
        let stats = pm_reference::compare::Stats::of(errors.clone());
        let over = errors.iter().filter(|e| **e > EFFECT_PROOF_CV).count();
        proved += 1;
        let ids: Vec<&str> = chain.iter().map(|(id, _)| id.as_str()).collect();
        if over > 0 {
            failures.push(format!(
                "{label} {ids:?}: {over} px over {EFFECT_PROOF_CV} CV from the f64 chain on the engine's input (max {:.3}, p99.9 {:.3}, mean {:.4})",
                stats.max, stats.p999, stats.mean
            ));
        }
        if let Some(file) = rows {
            let row = serde_json::json!({
                "time": sample.time,
                "layer_index": index, "type": ty, "chain": ids,
                "max_cv": stats.max, "p999_cv": stats.p999, "mean_cv": stats.mean, "over": over,
            });
            writeln!(file, "{row}").expect("effect proof evidence");
        }
    }
    (failures, proved)
}

fn main() {
    let captures = corpus_dirs();
    if captures.is_empty() {
        eprintln!("no parity corpus capture found; set PM_ENGINE_CORPUS to run this comparison");
        return;
    }
    let only = std::env::var("PM_RENDER_ONLY").ok();
    let mut rows = std::env::var_os("PM_L6_EFFECT_PROOF")
        .map(|path| std::fs::File::create(path).expect("effect proof output"));
    let mut renderer = Renderer::headless().expect("a Metal device");
    renderer
        .set_definitions(&all_builtin_definitions())
        .expect("proof definitions");
    let (mut failures, mut proved) = (Vec::new(), 0);
    for (capture, dir) in captures
        .iter()
        .flat_map(|c| projects(c).into_iter().map(move |d| (c.clone(), d)))
    {
        let project = load_project(&capture, &dir);
        // Text layers draw from the registry served as `pm-cli` serves it,
        // at the capture's device pixel ratio (tests/text_corpus.rs).
        if support::text::has_text(&project.document) {
            let dpr = std::fs::read_to_string(capture.join("manifest.json"))
                .ok()
                .and_then(|m| serde_json::from_str::<serde_json::Value>(&m).ok())
                .and_then(|m| m["gpu"]["devicePixelRatio"].as_f64())
                .unwrap_or(1.0) as f32;
            let server = std::sync::Arc::new(pm_text::served::DirectServer::new(
                pm_text::registry::CoreTextRegistry::new(),
                |r| r.generation(),
            ));
            renderer.set_text_fonts(Some(Box::new(pm_render::text::served::ServedFonts::new(
                server,
                std::sync::Arc::new(pm_render::text::served::FixedSeeds(
                    pm_text::compat::app_ui_font_cache_seeds(&[dpr]),
                )),
            ))));
        }
        if only
            .as_deref()
            .is_some_and(|o| !project.name.starts_with(o))
            || !project.missing_media.is_empty()
        {
            continue;
        }
        for sample in &project.samples {
            let (f, n) = prove_effect_chains(&mut renderer, &project, sample, &mut rows);
            failures.extend(f);
            proved += n;
        }
    }
    eprintln!(
        "L6 effect proof: {proved} layer chains; {} failures",
        failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
