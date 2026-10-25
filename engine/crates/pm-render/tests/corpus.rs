//! The render gate on the parity corpus (docs/engine/gate-policy.md): every
//! corpus project this renderer supports (by `unsupported_with_media`) is
//! rendered at every captured sample. Projects it does not support are
//! listed with the reason and skipped.
//!
//! Each frame is compared with the capture ("today") and with the
//! reference render of the same document (`support::reference`), per pixel
//! class (`pm_reference::compare`): interior pixels within 1 CV of today;
//! edge pixels at least as close to the reference as today, within the
//! frames' rounding; pixels the reference does not model yet by the
//! previous bound against today (section 25's Standard class for opaque
//! frames, the alpha class for transparent ones). Transparent frames the
//! reference models also keep their geometry (alpha area and centroid
//! within 1e-3), and the paused viewer's frame equals the export's.
//!
//! Projects with video use section 25's video class: the legacy frames come
//! from Chromium, so only natively decoded sources (no playback proxy) are
//! compared, on patch interiors, within 2 CV. `hevc-alpha.mov` is
//! interpreted as it was encoded (BT.709 coefficients, straight alpha; see
//! below).
//!
//! Projects with 3D object layers use section 25's 3D class on the 3D
//! pixels: the pixels where a 3D surface is on top (the depth and id
//! surface) grown by one pixel, plus the MSAA edges (pixels that change when
//! the 3D pass renders without MSAA), are judged by section 2's 3D bar
//! (`pm_render::parity`); every other pixel keeps the Standard class.
//!
//! `PM_ENGINE_CORPUS` picks captures (comma-separated; default: the newest
//! `v1-*` and the newest `render-*`);
//! `PM_RENDER_ONLY` limits to project names with a prefix;
//! `PM_RENDER_DIFFS=<dir>` writes engine and today's PNGs of failures there.

#[path = "support/pending.rs"]
mod pending;
#[path = "support/quality.rs"]
mod quality;
use quality::reference_problems;
mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_reference::compare::{Bounds, compare, geometry, identical, same_geometry};
use pm_render::parity::{bar_3d, dilate, msaa_edges};
use pm_render::{RenderRequest, Renderer, all_builtin_definitions, unsupported_with_definitions};
use support::*;

/// 8-bit RGBA as 0…1.
fn unit8(v: &[u8]) -> Vec<f64> {
    v.iter().map(|c| f64::from(*c) / 255.0).collect()
}

fn unit32(v: &[f32]) -> Vec<f64> {
    v.iter().map(|c| f64::from(*c)).collect()
}

fn unmodelled(r: &reference::Reference) -> String {
    if r.unmodelled.is_empty() {
        String::new()
    } else {
        format!(" (no reference: {})", r.unmodelled.join(", "))
    }
}

/// `render/still-*` projects and whether the renderer draws them.
const STILL_CLASSES: &[(&str, bool)] = &[
    ("render/still-gray-alpha-png", true),
    ("render/still-indexed-png", true),
    ("render/still-photo-gray-jpg", true),
    ("render/still-photo-heic", true),
    ("render/still-photo-lossy-webp", true),
    ("render/still-photo-lossless-opaque-webp", true),
    ("render/still-photo-tiff", true),
    ("render/still-16bit-png", true),
    ("render/still-p3-heic", true),
    ("render/still-p3-jpg", true),
    ("render/still-p3-png", true),
    ("render/still-photo-420-jpg", true),
    ("render/still-photo-422-jpg", true),
    ("render/still-photo-422-tall-mcu-jpg", true),
    ("render/still-photo-444-jpg", true),
    ("render/still-photo-444-tall-mcu-jpg", true),
    ("render/still-photo-alpha-webp", true),
    ("render/still-photo-avif", true),
    ("render/still-photo-lossless-webp", true),
    ("render/still-photo-orientation-6-jpg", true),
    ("render/still-photo-progressive-jpg", true),
];

/// Video-class projects drawing video minified under a projection, where
/// patch interiors of today's frame average source pixels across chroma
/// edges. The class restates today's reading (`support::restate_today`)
/// except its chroma filter; these projects draw a rendition carrying
/// today's filter (`support::todays_chroma_rendition`), so the rest is held
/// to the class. The engine's own filter is proven against 4:4:4 truth by
/// tests/chroma_truth.rs.
const MINIFIED_VIDEO: &[&str] = &["space25d/precomps"];

/// A scratch directory removed when dropped.
struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tempdir_guard(name: &str) -> TempDir {
    let dir = std::env::temp_dir().join(format!(
        "pm-render-corpus-{}-{}",
        std::process::id(),
        name.replace('/', "_")
    ));
    std::fs::create_dir_all(&dir).unwrap();
    TempDir(dir)
}

/// Optional anonymous per-frame evidence for paired branch attribution.
fn record_frame_result(file: &mut Option<std::fs::File>, id: &str, failed: bool, pixels: &[u8]) {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    if let Some(file) = file {
        let row = serde_json::json!({
            "frame": id, "status": if failed { "fail" } else { "pass" },
            "pixel_sha256": format!("{:x}", Sha256::digest(pixels)),
        });
        writeln!(file, "{row}").expect("frame attribution evidence");
    }
}

fn sample_frame_ids(project: &str, time: f64, width: u32, height: u32) -> [String; 4] {
    ["opaque", "transparent", "3d-opaque", "3d-transparent"]
        .map(|side| pending::frame_id(project, time, width, height, side))
}

fn has_pending_sample(
    frames: &std::collections::BTreeSet<String>,
    project: &str,
    time: f64,
    width: u32,
    height: u32,
) -> bool {
    sample_frame_ids(project, time, width, height)
        .iter()
        .any(|id| frames.contains(id))
}

/// Isolate every active L3 layer whose own edge can be hidden by later-lane
/// composition. The variant retains authored transform, mask and shutter
/// channels, while removing surrounding layers and L6 effects. It is run in
/// addition to the full-frame pending check; it cannot exempt that check.
fn prove_own_2d_layers(
    renderer: &mut Renderer,
    project: &Project,
    sample: &Sample,
    full_pending: bool,
    rows: &mut Option<std::fs::File>,
) -> Vec<String> {
    use pm_eval::{Comp, Evaluator, Scope, node};
    use pm_reference::compare::{geometry, same_geometry};
    use std::io::Write;
    let comp = Comp::resolve(&project.document, &project.composition).expect("composition");
    let seconds = comp.seconds_at(ticks(sample.time).get());
    let mut evaluator =
        Evaluator::new().with_expressions(std::sync::Arc::new(pm_render::expr::PmExprEngine));
    let mut scope = Scope::new(&mut evaluator, &comp, &comp);
    let mut failures = Vec::new();
    let mut request =
        RenderRequest::sized(Size::new(sample.width, sample.height), Precision::Float32);
    request.output_kind = pm_render::OutputKind::Final;
    request.background = false;
    request.motion_blur = pm_render::motion::MotionBlur::Samples(20);
    let solo = comp
        .layers()
        .iter()
        .any(|layer| node::truthy(node::member(Some(layer), "solo")));
    for (index, layer) in comp.layers().iter().enumerate() {
        let ty = pm_render::js::member_string(layer, "type").unwrap_or_default();
        if !matches!(ty.as_str(), "shape" | "solid") {
            continue;
        }
        if (solo && !node::truthy(node::member(Some(layer), "solo")))
            || !scope.active(layer, seconds).expect("layer active")
        {
            continue;
        }
        let has_masks = pm_render::composite::build::has_masks(&mut scope, layer, seconds)
            .expect("enabled masks");
        let motion = scope
            .evaluated(
                layer,
                node::member(Some(layer), "mblur"),
                seconds,
                "l.mblur",
            )
            .expect("motion channel")
            .truthy();
        let grouped = node::member(Some(layer), "group")
            .is_some_and(|v| !matches!(v.kind(), pm_model::NodeKind::Null));
        let is_3d = scope.is_3d_layer(layer).expect("3D property");
        if !(has_masks || motion || grouped || full_pending) || is_3d {
            continue;
        }
        let isolated = reference::isolated_own_layer(&project.document, index);
        let wanted = reference::render(
            &isolated,
            &project.composition,
            ticks(sample.time),
            sample.width,
            sample.height,
            false,
            20,
            renderer.effects(),
            &project.resolver,
        );
        let label = format!("{} t={} layer={index} ({ty})", project.name, sample.time);
        if wanted.modelled() != 1.0 {
            failures.push(format!(
                "{label}: own-layer reference incomplete: {:?}",
                wanted.unmodelled
            ));
            continue;
        }
        let frame = match renderer.render(
            &isolated,
            &project.composition,
            ticks(sample.time),
            &request,
            &project.resolver,
        ) {
            Ok(frame) => frame,
            Err(e) => {
                failures.push(format!("{label}: isolated render: {e}"));
                continue;
            }
        };
        if frame.color.format() != wgpu::TextureFormat::Rgba32Float || !frame.failures.is_empty() {
            failures.push(format!(
                "{label}: native float storage or layer failure: {:?} {:?}",
                frame.color.format(),
                frame.failures
            ));
            continue;
        }
        let native = unit32(&renderer.read_rgba(&frame).expect("isolated float readback"));
        let max_cv = native
            .iter()
            .zip(&wanted.rgba)
            .map(|(a, b)| (a - b).abs() * 255.0)
            .fold(0.0_f64, f64::max);
        let width = sample.width as usize;
        let alpha = |rgba: &[f64]| rgba.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>();
        let shape = same_geometry(
            &geometry(&alpha(&native), width),
            &geometry(&wanted.alpha(), width),
            1e-3,
        );
        let pass = max_cv <= 1.0 + 1e-9 && shape.is_ok();
        if !pass {
            failures.push(format!(
                "{label}: own-layer max {max_cv:.6} CV (bar 1); geometry {shape:?}"
            ));
        }
        if let Some(file) = rows {
            let classes = wanted.classes();
            let row = serde_json::json!({
                "frame": pending::frame_id(&project.name, sample.time, sample.width, sample.height, "own-layer"),
                "pending_frame_ids": sample_frame_ids(&project.name, sample.time, sample.width, sample.height),
                "layer_index": index, "type": ty, "masks": has_masks, "motion": motion,
                "grouped": grouped, "space": "2d-precomposition", "float_storage": "rgba32float",
                "max_cv": max_cv, "geometry": shape.err(), "pass": pass,
                "measurement_sha256": pending::measurement(&native, &[], &wanted.rgba, &classes),
            });
            writeln!(file, "{row}").expect("own-layer evidence");
        }
    }
    failures
}

/// A 2.5D solid's intrinsic float source, before the L4 plane projection.
/// The source keeps authored content, opacity, masks and shutter timing.
fn prove_local_3d_solids(
    renderer: &mut Renderer,
    project: &Project,
    sample: &Sample,
    rows: &mut Option<std::fs::File>,
) -> Vec<String> {
    use pm_eval::{Comp, Evaluator, Scope};
    use pm_reference::compare::{geometry, same_geometry};
    use std::io::Write;
    let comp = Comp::resolve(&project.document, &project.composition).expect("composition");
    let seconds = comp.seconds_at(ticks(sample.time).get());
    let mut evaluator =
        Evaluator::new().with_expressions(std::sync::Arc::new(pm_render::expr::PmExprEngine));
    let mut scope = Scope::new(&mut evaluator, &comp, &comp);
    let mut failures = Vec::new();
    let mut request =
        RenderRequest::sized(Size::new(sample.width, sample.height), Precision::Float32);
    request.output_kind = pm_render::OutputKind::Final;
    request.background = false;
    request.motion_blur = pm_render::motion::MotionBlur::Samples(20);
    for (index, layer) in comp.layers().iter().enumerate() {
        if pm_render::js::member_string(layer, "type").as_deref() != Some("solid")
            || !scope.is_3d_layer(layer).expect("3D property")
            || !scope.active(layer, seconds).expect("layer active")
        {
            continue;
        }
        let isolated = reference::isolated_local_solid(&project.document, index);
        let wanted = reference::render(
            &isolated,
            &project.composition,
            ticks(sample.time),
            sample.width,
            sample.height,
            false,
            20,
            renderer.effects(),
            &project.resolver,
        );
        let label = format!(
            "{} t={} local 2.5D solid layer={index}",
            project.name, sample.time
        );
        if wanted.modelled() != 1.0 {
            failures.push(format!(
                "{label}: intrinsic solid reference incomplete: {:?}",
                wanted.unmodelled
            ));
            continue;
        }
        let frame = match renderer.render(
            &isolated,
            &project.composition,
            ticks(sample.time),
            &request,
            &project.resolver,
        ) {
            Ok(frame) => frame,
            Err(e) => {
                failures.push(format!("{label}: intrinsic solid render: {e}"));
                continue;
            }
        };
        if frame.color.format() != wgpu::TextureFormat::Rgba32Float || !frame.failures.is_empty() {
            failures.push(format!(
                "{label}: native float storage or layer failure: {:?} {:?}",
                frame.color.format(),
                frame.failures
            ));
            continue;
        }
        let native = unit32(
            &renderer
                .read_rgba(&frame)
                .expect("intrinsic solid float readback"),
        );
        let native_alpha: f64 = native.chunks_exact(4).map(|p| p[3]).sum();
        let reference_alpha: f64 = wanted.rgba.chunks_exact(4).map(|p| p[3]).sum();
        if native_alpha <= 0.0 || reference_alpha <= 0.0 {
            failures.push(format!(
                "{label}: intrinsic solid proof has no painted pixels (native {native_alpha}, reference {reference_alpha})"
            ));
            continue;
        }
        let max_cv = native
            .iter()
            .zip(&wanted.rgba)
            .map(|(a, b)| (a - b).abs() * 255.0)
            .fold(0.0_f64, f64::max);
        let alpha = |rgba: &[f64]| rgba.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>();
        let shape = same_geometry(
            &geometry(&alpha(&native), sample.width as usize),
            &geometry(&wanted.alpha(), sample.width as usize),
            1e-3,
        );
        let pass = max_cv <= 1.0 + 1e-9 && shape.is_ok();
        if !pass {
            failures.push(format!(
                "{label}: intrinsic solid max {max_cv:.6} CV (bar 1); geometry {shape:?}"
            ));
        }
        if let Some(file) = rows {
            let row = serde_json::json!({
                "frame": pending::frame_id(&project.name, sample.time, sample.width, sample.height, "own-local-solid"),
                "pending_frame_ids": sample_frame_ids(&project.name, sample.time, sample.width, sample.height),
                "layer_index": index, "type": "solid", "space": "local-2.5d-preprojection",
                "source_pose": "canonical-local", "shutter_samples": 20,
                "float_storage": "rgba32float", "max_cv": max_cv,
                "native_alpha_sum": native_alpha, "reference_alpha_sum": reference_alpha,
                "geometry": shape.err(), "pass": pass,
                "measurement_sha256": pending::measurement(&native, &[], &wanted.rgba, &wanted.classes()),
            });
            writeln!(file, "{row}").expect("local solid evidence");
        }
    }
    failures
}

/// Follow active precomps only when a pending root frame has no drawable L3
/// layer of its own. Child source time uses the renderer's retiming and keep
/// frame-rate rules at every authored parent shutter sample. This proves the
/// child shape/solid source in its own composition geometry, before L4 places
/// and resamples the precomp. A cycle ends at the first repeated composition.
fn prove_nested_own_layers(
    renderer: &mut Renderer,
    project: &Project,
    sample: &Sample,
    rows: &mut Option<std::fs::File>,
) -> Vec<String> {
    use pm_eval::{Comp, Evaluator, Scope, node};
    use pm_reference::compare::{geometry, same_geometry};
    use sha2::{Digest, Sha256};
    use std::io::Write;

    struct NestedProof<'a> {
        renderer: &'a mut Renderer,
        project: &'a Project,
        root_sample: &'a Sample,
        rows: &'a mut Option<std::fs::File>,
        failures: Vec<String>,
        painted: usize,
    }

    impl NestedProof<'_> {
        fn visit(
            &mut self,
            comp_id: &str,
            seconds: f64,
            ancestry: &mut Vec<String>,
            precomp_indices: &mut Vec<usize>,
            shutter_trace: &mut Vec<(usize, usize)>,
        ) {
            let comp = Comp::resolve(&self.project.document, comp_id).expect("source composition");
            let open = Comp::resolve(&self.project.document, &self.project.composition)
                .expect("open composition");
            let mut evaluator = Evaluator::new()
                .with_expressions(std::sync::Arc::new(pm_render::expr::PmExprEngine));
            let mut scope = Scope::new(&mut evaluator, &comp, &open);
            let solo = comp
                .layers()
                .iter()
                .any(|layer| node::truthy(node::member(Some(layer), "solo")));
            let width = node::to_number(comp.member("w")) as u32;
            let height = node::to_number(comp.member("h")) as u32;
            if width == 0 || height == 0 {
                self.failures.push(format!(
                    "{}: nested composition has no source geometry",
                    self.project.name
                ));
                return;
            }
            for (index, layer) in comp.layers().iter().enumerate() {
                if (solo && !node::truthy(node::member(Some(layer), "solo")))
                    || !scope.active(layer, seconds).expect("source layer active")
                {
                    continue;
                }
                let ty = pm_render::js::member_string(layer, "type").unwrap_or_default();
                if matches!(ty.as_str(), "shape" | "solid") && ancestry.len() > 1 {
                    let label = format!(
                        "{} t={} nested source depth={} layer={index} ({ty})",
                        self.project.name,
                        self.root_sample.time,
                        ancestry.len() - 1
                    );
                    if scope.is_3d_layer(layer).expect("source 3D property") {
                        self.failures.push(format!(
                            "{label}: 3D child source needs local preprojection proof"
                        ));
                        continue;
                    }
                    let isolated = reference::isolated_own_layer_in_comp(
                        &self.project.document,
                        comp_id,
                        index,
                    );
                    let mut request =
                        RenderRequest::sized(Size::new(width, height), Precision::Float32);
                    request.output_kind = pm_render::OutputKind::Final;
                    request.background = false;
                    request.motion_blur = pm_render::motion::MotionBlur::Samples(20);
                    let wanted = reference::render(
                        &isolated,
                        comp_id,
                        ticks(seconds),
                        width,
                        height,
                        false,
                        20,
                        self.renderer.effects(),
                        &self.project.resolver,
                    );
                    if wanted.modelled() != 1.0 {
                        self.failures.push(format!(
                            "{label}: nested source reference incomplete: {:?}",
                            wanted.unmodelled
                        ));
                        continue;
                    }
                    let frame = match self.renderer.render(
                        &isolated,
                        comp_id,
                        ticks(seconds),
                        &request,
                        &self.project.resolver,
                    ) {
                        Ok(frame) => frame,
                        Err(e) => {
                            self.failures
                                .push(format!("{label}: nested source render: {e}"));
                            continue;
                        }
                    };
                    if frame.color.format() != wgpu::TextureFormat::Rgba32Float
                        || !frame.failures.is_empty()
                    {
                        self.failures.push(format!(
                            "{label}: native float storage or layer failure: {:?} {:?}",
                            frame.color.format(),
                            frame.failures
                        ));
                        continue;
                    }
                    let native = unit32(
                        &self
                            .renderer
                            .read_rgba(&frame)
                            .expect("nested float readback"),
                    );
                    let native_alpha: f64 = native.chunks_exact(4).map(|p| p[3]).sum();
                    let reference_alpha: f64 = wanted.rgba.chunks_exact(4).map(|p| p[3]).sum();
                    // A source outside its own canvas contributes no pixels to
                    // the parent; it cannot count as proof of painted content.
                    if native_alpha <= 0.0 && reference_alpha <= 0.0 {
                        continue;
                    }
                    if native_alpha <= 0.0 || reference_alpha <= 0.0 {
                        self.failures.push(format!(
                        "{label}: only one source painted (native {native_alpha}, reference {reference_alpha})"
                    ));
                        continue;
                    }
                    let max_cv = native
                        .iter()
                        .zip(&wanted.rgba)
                        .map(|(a, b)| (a - b).abs() * 255.0)
                        .fold(0.0_f64, f64::max);
                    let alpha =
                        |rgba: &[f64]| rgba.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>();
                    let shape = same_geometry(
                        &geometry(&alpha(&native), width as usize),
                        &geometry(&wanted.alpha(), width as usize),
                        1e-3,
                    );
                    let pass = max_cv <= 1.0 + 1e-9 && shape.is_ok();
                    if !pass {
                        self.failures.push(format!(
                            "{label}: nested source max {max_cv:.6} CV (bar 1); geometry {shape:?}"
                        ));
                    }
                    self.painted += 1;
                    if let Some(file) = self.rows {
                        let comp_hash = format!("{:x}", Sha256::digest(comp_id.as_bytes()));
                        let row = serde_json::json!({
                            "frame": pending::frame_id(&self.project.name, self.root_sample.time, self.root_sample.width, self.root_sample.height, "own-nested-source"),
                            "pending_frame_ids": sample_frame_ids(&self.project.name, self.root_sample.time, self.root_sample.width, self.root_sample.height),
                            "composition_sha256": comp_hash, "precomp_indices": precomp_indices,
                            "parent_shutter_trace": shutter_trace, "child_time_seconds": seconds,
                            "layer_index": index, "type": ty,
                            "space": "nested-local-precomposition", "source_size": [width, height],
                            "float_storage": "rgba32float", "shutter_samples": 20,
                            "native_alpha_sum": native_alpha, "reference_alpha_sum": reference_alpha,
                            "max_cv": max_cv, "geometry": shape.err(), "pass": pass,
                            "measurement_sha256": pending::measurement(&native, &[], &wanted.rgba, &wanted.classes()),
                        });
                        writeln!(file, "{row}").expect("nested source evidence");
                    }
                } else if ty == "precomp" {
                    let child_id = node::member(node::member(Some(layer), "d"), "comp")
                        .and_then(|v| node::string(Some(v)))
                        .map(|s| s.to_string_lossy());
                    let Some(child_id) = child_id else { continue };
                    if ancestry.contains(&child_id) {
                        continue;
                    }
                    let Ok(child) = Comp::resolve(&self.project.document, &child_id) else {
                        continue;
                    };
                    let blurred = scope
                        .evaluated(
                            layer,
                            node::member(Some(layer), "mblur"),
                            seconds,
                            "l.mblur",
                        )
                        .expect("precomp motion channel")
                        .truthy();
                    let times = if blurred {
                        let shutter = pm_render::motion::shutter_frames(comp.node())
                            / pm_render::motion::level_fps(&comp);
                        (0..20)
                            .map(|s| seconds + ((f64::from(s) + 0.5) / 20.0 - 0.5) * shutter)
                            .collect::<Vec<_>>()
                    } else {
                        vec![seconds]
                    };
                    for (shutter_index, parent_time) in times.iter().copied().enumerate() {
                        if !scope
                            .active(layer, parent_time)
                            .expect("precomp shutter activity")
                        {
                            continue;
                        }
                        let child_time = scope
                            .source_time(layer, parent_time, None)
                            .expect("authored precomp source time");
                        let child_time =
                            pm_render::retime::keep_frame_rate(layer, &child, child_time);
                        ancestry.push(child_id.clone());
                        precomp_indices.push(index);
                        shutter_trace.push((shutter_index, times.len()));
                        self.visit(
                            &child_id,
                            child_time,
                            ancestry,
                            precomp_indices,
                            shutter_trace,
                        );
                        shutter_trace.pop();
                        precomp_indices.pop();
                        ancestry.pop();
                    }
                }
            }
        }
    }

    let comp = Comp::resolve(&project.document, &project.composition).expect("open composition");
    let seconds = comp.seconds_at(ticks(sample.time).get());
    let mut evaluator =
        Evaluator::new().with_expressions(std::sync::Arc::new(pm_render::expr::PmExprEngine));
    let mut scope = Scope::new(&mut evaluator, &comp, &comp);
    let solo = comp
        .layers()
        .iter()
        .any(|layer| node::truthy(node::member(Some(layer), "solo")));
    let root_has_own = comp.layers().iter().any(|layer| {
        matches!(
            pm_render::js::member_string(layer, "type").as_deref(),
            Some("shape" | "solid")
        ) && (!solo || node::truthy(node::member(Some(layer), "solo")))
            && scope.active(layer, seconds).expect("root layer active")
    });
    if root_has_own {
        return Vec::new();
    }
    let mut proof = NestedProof {
        renderer,
        project,
        root_sample: sample,
        rows,
        failures: Vec::new(),
        painted: 0,
    };
    proof.visit(
        &project.composition,
        seconds,
        &mut vec![project.composition.clone()],
        &mut Vec::new(),
        &mut Vec::new(),
    );
    // A project with no shape or solid anywhere (generated layers only)
    // has no L3 source to prove.
    let has_l3_source = || {
        let root = project.document.root().expect("project root");
        let lists = std::iter::once(root).chain(
            match node::member(Some(root), "comps").map(pm_model::Node::kind) {
                Some(pm_model::NodeKind::Object(comps)) => comps.values().collect::<Vec<_>>(),
                _ => Vec::new(),
            },
        );
        lists
            .flat_map(|c| node::array(node::member(Some(c), "layers")).unwrap_or(&[]))
            .any(|l| {
                matches!(
                    pm_render::js::member_string(l, "type").as_deref(),
                    Some("shape" | "solid")
                )
            })
    };
    if proof.painted == 0 && has_l3_source() {
        proof.failures.push(format!(
            "{} t={}: pending precomp has no painted nested L3 source proof",
            project.name, sample.time
        ));
    }
    proof.failures
}

/// Native float vector paint against f64 coverage before 2.5D projection.
/// Every authored shutter sample is retained; L4 still owns the plane.
fn prove_local_3d_vectors(
    renderer: &Renderer,
    project: &Project,
    sample: &Sample,
    rows: &mut Option<std::fs::File>,
) -> Vec<String> {
    use pm_eval::{Comp, Evaluator, Scope, node};
    use pm_reference::compare::{geometry, same_geometry};
    use pm_render::vector::{VectorQuad, VectorSource, cache::VectorCache};
    use pm_vector::kurbo::Affine;
    use pm_vector::render::scene::PixelRect;
    use std::io::Write;

    let comp = Comp::resolve(&project.document, &project.composition).expect("composition");
    let seconds = comp.seconds_at(ticks(sample.time).get());
    let mut evaluator =
        Evaluator::new().with_expressions(std::sync::Arc::new(pm_render::expr::PmExprEngine));
    let mut scope = Scope::new(&mut evaluator, &comp, &comp);
    let mut cache = VectorCache::new();
    let mut failures = Vec::new();
    for (index, layer) in comp.layers().iter().enumerate() {
        if pm_render::js::member_string(layer, "type").as_deref() != Some("shape")
            || !scope.is_3d_layer(layer).expect("3D property")
        {
            continue;
        }
        let label = format!(
            "{} t={} local 2.5D vector layer={index}",
            project.name, sample.time
        );
        if !scope.active(layer, seconds).expect("layer active") {
            continue;
        }
        let blurred = scope
            .evaluated(
                layer,
                node::member(Some(layer), "mblur"),
                seconds,
                "l.mblur",
            )
            .expect("motion channel")
            .truthy();
        let times = if blurred {
            let shutter = pm_render::motion::shutter_frames(comp.node())
                / pm_render::motion::level_fps(&comp);
            (0..20)
                .map(|s| seconds + ((f64::from(s) + 0.5) / 20.0 - 0.5) * shutter)
                .collect::<Vec<_>>()
        } else {
            vec![seconds]
        };
        let mut scenes = Vec::new();
        let mut bounds: Option<PixelRect> = None;
        for &time in &times {
            let content = scope
                .ev
                .resolve_content(scope.comp, layer, time)
                .expect("shape content");
            let quad = pm_render::vector::plan_shape(
                &mut scope,
                layer,
                &content,
                time,
                &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                PixelRect::new(-(1 << 29), -(1 << 29), 1 << 30, 1 << 30),
            )
            .expect("local vector plan");
            if let Some(VectorQuad::Device {
                source: VectorSource::Scene { scene, rect },
                ..
            }) = quad
            {
                bounds = Some(bounds.map_or(rect, |b| b.union(rect)));
                scenes.push(scene);
            }
        }
        let Some(rect) = bounds else { continue };
        if rect.width == 0 || rect.height == 0 {
            continue;
        }
        let (w, h) = (rect.width as usize, rect.height as usize);
        let mut native = vec![0.0; w * h * 4];
        let mut exact = vec![0.0; w * h * 4];
        for scene in &scenes {
            let source = VectorSource::Scene {
                scene: scene.clone(),
                rect,
            };
            let view = cache
                .resolve(
                    renderer.device(),
                    renderer.queue(),
                    &source,
                    Precision::Float32,
                )
                .expect("native local vector");
            assert_eq!(view.texture().format(), wgpu::TextureFormat::Rgba32Float);
            let pixels =
                pm_gpu::readback::read_f32(renderer.device(), renderer.queue(), view.texture())
                    .expect("native local vector readback");
            let mut shifted = scene.clone();
            for item in &mut shifted.items {
                item.transform =
                    Affine::translate((-f64::from(rect.x), -f64::from(rect.y))) * item.transform;
            }
            let reference = reference::scene_pixels(&shifted, w, h).expect("f64 local vector");
            for (i, (&n, r)) in pixels
                .iter()
                .zip(reference.iter().flat_map(|p| p.0))
                .enumerate()
            {
                native[i] += f64::from(n) / times.len() as f64;
                exact[i] += r / times.len() as f64;
            }
        }
        let max_cv = native
            .iter()
            .zip(&exact)
            .map(|(a, b)| (a - b).abs() * 255.0)
            .fold(0.0_f64, f64::max);
        let alpha = |rgba: &[f64]| rgba.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>();
        let shape = same_geometry(
            &geometry(&alpha(&native), w),
            &geometry(&alpha(&exact), w),
            1e-3,
        );
        let pass = max_cv <= 1.0 + 1e-9 && shape.is_ok();
        if !pass {
            failures.push(format!(
                "{label}: local preprojection max {max_cv:.6} CV (bar 1); geometry {shape:?}"
            ));
        }
        if let Some(file) = rows {
            let row = serde_json::json!({
                "frame": pending::frame_id(&project.name, sample.time, sample.width, sample.height, "own-local-vector"),
                "pending_frame_ids": sample_frame_ids(&project.name, sample.time, sample.width, sample.height),
                "layer_index": index, "type": "shape", "space": "local-2.5d-preprojection",
                "shutter_samples": times.len(), "float_storage": "rgba32float", "max_cv": max_cv,
                "geometry": shape.err(), "pass": pass,
                "measurement_sha256": pending::measurement(&native, &[], &exact,
                    &vec![pm_reference::compare::Class::Edge; w*h]),
            });
            writeln!(file, "{row}").expect("local vector evidence");
        }
    }
    failures
}

/// The native R16float path-mask canvas against independent f64 coverage,
/// before a 2.5D inverse-plane sample. It uses the authored center time.
fn prove_path_mask_canvases(
    renderer: &Renderer,
    project: &Project,
    sample: &Sample,
    rows: &mut Option<std::fs::File>,
) -> Vec<String> {
    use pm_eval::{Comp, Evaluator, Scope, node};
    use pm_reference::compare::{geometry, same_geometry};
    use pm_render::composite::build::{MaskRoute, masks};
    use pm_render::composite::path_masks::PathMaskPipelines;
    use std::io::Write;

    let comp = Comp::resolve(&project.document, &project.composition).expect("composition");
    let seconds = comp.seconds_at(ticks(sample.time).get());
    let (sx, sy) = pm_render::plan::derived_scale(&comp, sample.width, sample.height);
    let mut evaluator =
        Evaluator::new().with_expressions(std::sync::Arc::new(pm_render::expr::PmExprEngine));
    let mut scope = Scope::new(&mut evaluator, &comp, &comp);
    let mut failures = Vec::new();
    for (index, layer) in comp.layers().iter().enumerate() {
        let ty = pm_render::js::member_string(layer, "type").unwrap_or_default();
        if !matches!(ty.as_str(), "shape" | "solid")
            || node::array(node::member(Some(layer), "masks")).is_none_or(|m| m.is_empty())
            || !scope.active(layer, seconds).expect("layer active")
        {
            continue;
        }
        let is_3d = scope.is_3d_layer(layer).expect("3D property");
        let world = if is_3d {
            [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
        } else {
            let m = scope.world_matrix(layer, seconds).expect("world transform");
            [
                m[0] * sx,
                m[1] * sy,
                m[2] * sx,
                m[3] * sy,
                m[4] * sx,
                m[5] * sy,
            ]
        };
        let MaskRoute::Path(route) = masks(
            &mut scope,
            layer,
            seconds,
            world,
            None,
            sample.width,
            sample.height,
        )
        .expect("mask route") else {
            continue;
        };
        let path = route.path.expect("path route has coverage");
        let pipelines = PathMaskPipelines::new(renderer.device());
        let mut encoder = renderer
            .device()
            .create_command_encoder(&Default::default());
        let texture = pipelines.coverage(renderer.device(), &mut encoder, &path);
        assert_eq!(texture.format(), wgpu::TextureFormat::R16Float);
        renderer.queue().submit([encoder.finish()]);
        let raw = pm_gpu::readback::read_raw(renderer.device(), renderer.queue(), &texture)
            .expect("native float path-mask coverage");
        let native = raw
            .chunks_exact(2)
            .map(|c| {
                f64::from(pm_gpu::readback::half_to_f32(u16::from_le_bytes([
                    c[0], c[1],
                ])))
            })
            .collect::<Vec<_>>();
        let exact = support::mask_reference::path_coverage(
            &path.masks,
            world,
            sample.width as usize,
            sample.height as usize,
        );
        // G-V2's published R16float store, interpolation, Gaussian-tail and
        // exact-area error budget; each successive mask contributes once.
        let n = path.masks.masks.len();
        let bound =
            255.0 * (2f64.powi(-12) + n as f64 * (2f64.powi(-12) + 1.4e-5 + 1e-4 + 6e-5 + 8e-5));
        let max_cv = native
            .iter()
            .zip(&exact)
            .map(|(a, b)| (a - b).abs() * 255.0)
            .fold(0.0_f64, f64::max);
        // A float mask retains sub-CV coverage that an 8-bit canvas would
        // quantize away. Require that observable fact where it occurs.
        let fine = |pixels: &[f64]| {
            pixels
                .iter()
                .filter(|&&v| (v * 255.0 - (v * 255.0).round()).abs() > 1e-3)
                .count()
        };
        let (native_fine, reference_fine) = (fine(&native), fine(&exact));
        let no_feather = path.masks.masks.iter().all(|m| m.feather == 0.0);
        let shape = no_feather.then(|| {
            same_geometry(
                &geometry(&native, sample.width as usize),
                &geometry(&exact, sample.width as usize),
                1e-3,
            )
        });
        let pass = max_cv <= bound
            && shape.as_ref().is_none_or(Result::is_ok)
            && (reference_fine == 0 || native_fine > 0);
        let label = format!("{} t={} path-mask layer={index}", project.name, sample.time);
        if !pass {
            failures.push(format!("{label}: native float mask max {max_cv:.6} CV (G-V2 bound {bound:.6}); geometry {shape:?}"));
        }
        if let Some(file) = rows {
            let row = serde_json::json!({
                "frame": pending::frame_id(&project.name, sample.time, sample.width, sample.height, "own-path-mask"),
                "pending_frame_ids": sample_frame_ids(&project.name, sample.time, sample.width, sample.height),
                "layer_index": index, "type": ty,
                "space": if is_3d { "local-2.5d-preprojection" } else { "2d-precomposition" },
                "mask_center_seconds": seconds, "mask_count": n, "native_storage": "r16float",
                "native_sub_cv_samples": native_fine, "reference_sub_cv_samples": reference_fine,
                "max_cv": max_cv, "bound_cv": bound,
                "geometry": shape.and_then(Result::err), "pass": pass,
                "measurement_sha256": pending::measurement(&native, &[], &exact,
                    &vec![pm_reference::compare::Class::Edge; native.len()]),
            });
            writeln!(file, "{row}").expect("path-mask evidence");
        }
    }
    failures
}

#[test]
fn corpus_own_layers_reference_gate() {
    let captures = corpus_dirs();
    if captures.is_empty() {
        eprintln!("no parity corpus capture found; set PM_ENGINE_CORPUS to run this comparison");
        return;
    }
    let only = std::env::var("PM_RENDER_ONLY").ok();
    let pending: std::collections::BTreeSet<String> =
        serde_json::from_str::<Vec<pending::Entry>>(include_str!("fixtures/corpus-pending.json"))
            .expect("pending fixture")
            .into_iter()
            .map(|row| row.frame)
            .collect();
    let mut rows = std::env::var_os("PM_L3_LAYER_PROOF")
        .map(|path| std::fs::File::create(path).expect("own-layer proof output"));
    let mut renderer = Renderer::headless().expect("a Metal device");
    renderer
        .set_definitions(&all_builtin_definitions())
        .expect("proof definitions");
    let mut failures = Vec::new();
    let mut samples = 0;
    for (capture, dir) in captures
        .iter()
        .flat_map(|c| projects(c).into_iter().map(move |d| (c.clone(), d)))
    {
        let project = load_project(&capture, &dir);
        if only
            .as_deref()
            .is_some_and(|o| !project.name.starts_with(o))
            || !project.missing_media.is_empty()
        {
            continue;
        }
        for sample in &project.samples {
            samples += 1;
            let full_pending = has_pending_sample(
                &pending,
                &project.name,
                sample.time,
                sample.width,
                sample.height,
            );
            failures.extend(prove_own_2d_layers(
                &mut renderer,
                &project,
                sample,
                full_pending,
                &mut rows,
            ));
            failures.extend(prove_local_3d_solids(
                &mut renderer,
                &project,
                sample,
                &mut rows,
            ));
            failures.extend(prove_local_3d_vectors(
                &renderer, &project, sample, &mut rows,
            ));
            failures.extend(prove_path_mask_canvases(
                &renderer, &project, sample, &mut rows,
            ));
            if full_pending {
                failures.extend(prove_nested_own_layers(
                    &mut renderer,
                    &project,
                    sample,
                    &mut rows,
                ));
            }
        }
    }
    eprintln!(
        "L3 own-layer proof visited {samples} corpus samples; {} failures",
        failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn corpus_render_parity() {
    use sha2::{Digest, Sha256};
    let mut frame_results = std::env::var_os("PM_CORPUS_FRAME_RESULTS")
        .map(|path| std::fs::File::create(path).expect("frame attribution output"));
    let captures = corpus_dirs();
    if captures.is_empty() {
        eprintln!("no parity corpus capture found; set PM_ENGINE_CORPUS to run this comparison");
        return;
    }
    let only = std::env::var("PM_RENDER_ONLY").ok();
    let diffs = std::env::var("PM_RENDER_DIFFS")
        .ok()
        .map(std::path::PathBuf::from);
    let mut renderer = Renderer::headless().expect("a Metal device");
    // The built-in effects, transitions, and extension layers as the kernel
    // snapshots them (the captures load only built-in and missing
    // definitions).
    renderer
        .set_definitions(&all_builtin_definitions())
        .expect("built-in definitions");
    let mut compared = 0;
    // Opaque frames the reference models in full, and in part.
    let (mut complete, mut partial) = (0, 0);
    let mut failed = Vec::new();
    let mut pending = pending::Audit::read(include_str!("fixtures/corpus-pending.json"));
    let mut skipped = Vec::new();
    let mut seen = std::collections::BTreeMap::new();
    for (capture, dir) in captures
        .iter()
        .flat_map(|c| projects(c).into_iter().map(move |d| (c.clone(), d)))
    {
        let p = load_project(&capture, &dir);
        if only.as_deref().is_some_and(|o| !p.name.starts_with(o)) {
            continue;
        }
        if !p.missing_media.is_empty() {
            skipped.push((
                p.name.clone(),
                format!("media not available: {:?}", p.missing_media),
            ));
            continue;
        }
        // Text and captions are judged by the text class (tests/text_corpus.rs).
        if support::text::has_text(&p.document) {
            skipped.push((p.name.clone(), "text class: tests/text_corpus.rs".into()));
            continue;
        }
        // The capture app had only the built-in definitions: any other
        // extension layer definition was missing there (the checkerboard).
        let mut set = all_builtin_definitions();
        set.missing = support::extension_definitions(&p.document)
            .into_iter()
            .filter(|id| !set.definitions.iter().any(|d| d.id.as_str() == id))
            .map(|id| id.as_str().into())
            .collect();
        renderer
            .set_definitions(&set)
            .expect("built-in definitions");
        let issues = unsupported_with_definitions(
            &p.document,
            &p.composition,
            Some(&p.resolver),
            renderer.effects(),
        );
        seen.insert(p.name.clone(), issues.is_empty());
        // Amendment 21: these classes are drawn natively; routing one of
        // them is a failure, not a skip.
        let forbidden: Vec<String> = issues
            .iter()
            .filter(|u| support::routing_forbidden(&u.feature))
            .map(|u| u.to_string())
            .collect();
        if !forbidden.is_empty() {
            failed.push(format!("{}: routed for {}", p.name, forbidden.join(", ")));
        }
        if !issues.is_empty() {
            let mut reasons: Vec<String> = issues.iter().map(|u| u.feature.to_string()).collect();
            reasons.dedup();
            skipped.push((p.name.clone(), reasons.join(", ")));
            continue;
        }
        // Video sources: natively decoded only, interpreted as encoded.
        let mut video = false;
        let mut proxied = false;
        let root = p.document.root().unwrap();
        if let Some(assets) = root.get("assets")
            && let pm_model::NodeKind::Object(map) = assets.kind()
        {
            {
                for (id, record) in map.iter() {
                    let id = id.to_string_lossy();
                    let kind = record.get("kind").and_then(|k| match k.kind() {
                        pm_model::NodeKind::String(s) => Some(s.to_string_lossy()),
                        _ => None,
                    });
                    if kind.as_deref() != Some("video") {
                        continue;
                    }
                    video = true;
                    if matches!(
                        record.get("playbackProxy").map(|n| n.kind()),
                        Some(pm_model::NodeKind::Bool(true))
                    ) {
                        proxied = true;
                    }
                    let Some(path) = p.resolver.paths.get(&id) else {
                        continue;
                    };
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default();
                    let read = renderer.video_color(path).expect("video color");
                    let mut c = read;
                    match name {
                        // Encoded with straight alpha (its color is red where
                        // alpha is 0); the probe applies Apple's premultiplied
                        // default for HEVC with alpha. Chromium reads it as
                        // straight (and with BT.601 coefficients, as the
                        // untagged rule does).
                        "hevc-alpha.mov" => {
                            c.alpha = pm_protocol::color::AlphaMode::Straight;
                        }
                        // Untagged SD: section 9 reads BT.601; Chromium reads
                        // untagged VP9 and AV1 with BT.709 coefficients
                        // (measured here; H.264, HEVC, and VP8 read BT.601 in
                        // both). A recorded deviation (docs/engine/pm-render.md).
                        "av1.mp4" | "vp9-opus.webm" => {
                            c.matrix = pm_protocol::color::MatrixCoefficients::Bt709;
                        }
                        _ => {}
                    }
                    // Today's readings the engine corrects (SD primaries,
                    // partly tagged SD H.264), restated so this gate keeps
                    // judging everything else.
                    support::restate_today(path, &mut c);
                    if c != read {
                        renderer.interpret(path, Some(c));
                    }
                }
            }
        }
        if proxied {
            skipped.push((
                p.name.clone(),
                "the legacy frame shows a playback proxy (not a native decode)".into(),
            ));
            continue;
        }
        let renditions = tempdir_guard(&p.name);
        let mut resolver = pm_render::MapResolver {
            paths: p.resolver.paths.clone(),
        };
        if video {
            // Media reached without an asset record (generated corpus clips).
            support::restate_today_for_videos(&mut renderer, &p.resolver);
            if MINIFIED_VIDEO.contains(&p.name.as_str()) {
                for path in resolver.paths.values_mut() {
                    let Ok(read) = renderer.video_color(path) else {
                        continue;
                    };
                    let rendition = support::todays_chroma_rendition(path, &renditions.0);
                    let mut c = renderer.video_color(&rendition).unwrap();
                    (c.primaries, c.matrix, c.range) = (read.primaries, read.matrix, read.range);
                    renderer.interpret(&rendition, Some(c));
                    *path = rendition;
                }
            }
        }
        let mut geometry_document = None;
        for s in &p.samples {
            let failed_before = failed.len();
            let frame_id = format!(
                "{:x}",
                Sha256::digest(format!("{}|{}|{}", capture.display(), p.name, s.time).as_bytes(),)
            );
            // The capture's motion blur: 16 samples opaque, 20 transparent.
            let mut request =
                RenderRequest::sized(Size::new(s.width, s.height), Precision::Float16);
            // Use the export's canvas policy; source timing is shared with
            // the viewer and holds the last source frame without padding.
            request.output_kind = pm_render::OutputKind::Final;
            request.motion_blur = pm_render::motion::MotionBlur::Samples(16);
            let frame = match renderer.render(
                &p.document,
                &p.composition,
                ticks(s.time),
                &request,
                &resolver,
            ) {
                Ok(f) => f,
                Err(e) => {
                    failed.push(format!("{} t={}: render error {e}", p.name, s.time));
                    record_frame_result(&mut frame_results, &frame_id, true, &[]);
                    continue;
                }
            };
            if !frame.failures.is_empty() {
                failed.push(format!(
                    "{} t={}: layer failures {:?}",
                    p.name, s.time, frame.failures
                ));
            }
            let rgba = renderer.read_rgba(&frame).unwrap();
            // Preview == export (gate-policy.md).
            let (preview, export) = reference::preview_and_export(
                &mut renderer,
                &p.document,
                &p.composition,
                ticks(s.time),
                request.output_size,
                request.motion_blur,
                // The gated frame's media (a minified video's rendition).
                &resolver,
            );
            // The gated frame is the export's.
            if let Err(e) = identical(&export, &rgba) {
                failed.push(format!(
                    "{} t={}: gated frame vs export: {e}",
                    p.name, s.time
                ));
            }
            if let Err(e) = identical(&preview, &export) {
                failed.push(format!("{} t={}: {e}", p.name, s.time));
            }
            let ours = opaque8(&rgba);
            let (w, h, reference) =
                support::reference_substitutions::opaque_reference(&capture, &p.name, s)
                    .unwrap_or_else(|error| panic!("{} t={}: {error}", p.name, s.time));
            assert_eq!((w, h), (s.width, s.height));
            // Keep reviewable native/reference evidence even when an explicit
            // owner-approved substitution passes its ordinary comparison.
            if s.reference_substitution.is_some()
                && let Some(dir) = &diffs
            {
                std::fs::create_dir_all(dir).unwrap();
                let stem = format!("{}-{}-COMPCLOSE-1", p.name.replace('/', "_"), s.time);
                write_png(&dir.join(format!("{stem}-engine.png")), w, h, &ours);
                write_png(&dir.join(format!("{stem}-reference.png")), w, h, &reference);
            }
            if video {
                let mask = interior(&reference, w, h);
                let (max, n) = masked_max(&ours, &reference, &mask);
                compared += 1;
                let line = format!(
                    "{:<40} t={:<6} {:>7} interior px (video class)  max {}",
                    p.name, s.time, n, max
                );
                eprintln!("{line}");
                if max > 2 || n == 0 {
                    failed.push(line);
                    if let Some(dir) = &diffs {
                        std::fs::create_dir_all(dir).unwrap();
                        let stem = format!("{}-{}", p.name.replace('/', "_"), s.time);
                        write_png(&dir.join(format!("{stem}-engine.png")), w, h, &ours);
                        write_png(&dir.join(format!("{stem}-legacy.png")), w, h, &reference);
                    }
                }
                record_frame_result(
                    &mut frame_results,
                    &frame_id,
                    failed.len() > failed_before,
                    &ours,
                );
                continue;
            }
            // 3D class: the 3D pixels and MSAA edges, measured on our renders.
            let three_d = if support::has_3d_objects(&p.document) {
                let mut with_ids = request.clone();
                with_ids.depth_id = true;
                let ids = renderer
                    .render(
                        &p.document,
                        &p.composition,
                        ticks(s.time),
                        &with_ids,
                        &p.resolver,
                    )
                    .expect("depth and id render");
                let surface = renderer.read_depth_id(&ids).unwrap().unwrap();
                renderer.set_3d_msaa(false);
                let plain = renderer
                    .render(
                        &p.document,
                        &p.composition,
                        ticks(s.time),
                        &request,
                        &p.resolver,
                    )
                    .expect("render without MSAA");
                renderer.set_3d_msaa(true);
                let plain = opaque8(&renderer.read_rgba(&plain).unwrap());
                let (w, h) = (s.width as usize, s.height as usize);
                let object: Vec<bool> = surface
                    .chunks_exact(4)
                    .map(|px| px[0].is_finite())
                    .collect();
                let edges = msaa_edges(&ours, &plain, w, h);
                let class: Vec<bool> = dilate(&object, w, h)
                    .iter()
                    .zip(&edges)
                    .map(|(a, b)| *a || *b)
                    .collect();
                Some((object, edges, class))
            } else {
                None
            };
            // Transparent captures (`renderToPixels(…, {transparent: true})`):
            // premultiplied float values, section 25's alpha class (within
            // 1 CV of the reference).
            if let Some(float) = &s.float {
                let mut request = request;
                request.background = false;
                request.motion_blur = pm_render::motion::MotionBlur::Samples(20);
                let frame = renderer
                    .render(
                        &p.document,
                        &p.composition,
                        ticks(s.time),
                        &request,
                        &p.resolver,
                    )
                    .expect("transparent render");
                let ours = renderer.read_rgba(&frame).unwrap();
                let today = read_float(float);
                match &three_d {
                    // The 3D class (section 25) until the 3D lane converts it.
                    Some((object, edges, class)) => {
                        let rest: Vec<bool> = class.iter().map(|c| !c).collect();
                        let (w, h) = (s.width as usize, s.height as usize);
                        let b = bar_3d(&rgba8(&today), &rgba8(&ours), edges, object, w, h);
                        let max = float_max_cv_masked(&ours, &today, &rest);
                        let line = format!(
                            "{:<40} t={:<6} transparent float  max {max:.4} CV outside 3D; 3D bar ≤2 {:.4}% max outside edges {} SSIM {:.5}",
                            p.name,
                            s.time,
                            100.0 * b.fraction_within_2,
                            b.max_cv_outside_edges,
                            b.object_ssim.unwrap_or(f64::NAN)
                        );
                        eprintln!("{line}");
                        let checks = if max <= 1.0 && b.pass() {
                            Vec::new()
                        } else {
                            vec![format!(
                                "3D alpha: outside max {max:.9} CV; fraction within 2 {:.12}; max outside edges {}; SSIM {:?}",
                                b.fraction_within_2, b.max_cv_outside_edges, b.object_ssim
                            )]
                        };
                        let classes: Vec<_> = class
                            .iter()
                            .map(|v| {
                                if *v {
                                    pm_reference::compare::Class::Resampled
                                } else {
                                    pm_reference::compare::Class::Unreferenced
                                }
                            })
                            .collect();
                        let owned = pending.check(
                            pending::Entry {
                                frame: pending::frame_id(
                                    &p.name,
                                    s.time,
                                    s.width,
                                    s.height,
                                    "3d-transparent",
                                ),
                                owner: "L4".into(),
                                reason: "3D surfaces and composition".into(),
                                checks: checks.clone(),
                                measurement_sha256: pending::measurement(
                                    &unit32(&ours),
                                    &unit32(&today),
                                    &[],
                                    &classes,
                                ),
                            },
                            true,
                        );
                        if !checks.is_empty() && !owned {
                            failed.push(line);
                        }
                    }
                    None => {
                        let r = reference::render(
                            &p.document,
                            &p.composition,
                            ticks(s.time),
                            s.width,
                            s.height,
                            false,
                            20,
                            renderer.effects(),
                            &p.resolver,
                        );
                        let (engine, today) = (unit32(&ours), unit32(&today));
                        let r = r.resolved(&engine);
                        let report = compare(&engine, &today, &r.rgba, &r.classes());
                        let mut problems = reference_problems(
                            &report,
                            &Bounds::HALF_FLOAT,
                            &engine,
                            &today,
                            &r.rgba,
                            &r.classes(),
                        );
                        let mut shape = String::new();
                        if r.modelled() == 1.0 {
                            let alpha =
                                |f: &[f64]| f.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>();
                            let wanted = geometry(&r.alpha(), r.width);
                            // Match the CLI's --precision=32: change only
                            // this render copy's project setting. Requests
                            // cannot override the saved project's depth.
                            let document = geometry_document.get_or_insert_with(|| {
                                let mut root = p.document.to_value().expect("project root");
                                let pm_model::Value::Object(map) = &mut root else {
                                    panic!("project root is not an object");
                                };
                                map.insert("colorDepth", pm_model::Value::Number(32.0));
                                pm_model::Document::new(Some(root), p.document.revision())
                            });
                            let mut exact = request.clone();
                            exact.precision = Precision::Float32;
                            assert_eq!(
                                renderer.device_target_format(document, &exact),
                                wgpu::TextureFormat::Rgba32Float,
                                "32-bit project geometry requires float32 filtering and blending"
                            );
                            let f32_frame = renderer
                                .render(
                                    document,
                                    &p.composition,
                                    ticks(s.time),
                                    &exact,
                                    &p.resolver,
                                )
                                .expect("f32 transparent render");
                            assert_eq!(f32_frame.color.format(), wgpu::TextureFormat::Rgba32Float);
                            let ours = geometry(
                                &alpha(&unit32(&renderer.read_rgba(&f32_frame).unwrap())),
                                r.width,
                            );
                            let theirs = geometry(&alpha(&today), r.width);
                            shape = format!(
                                " geometry: area {:.3} engine {:.3} today {:.3}",
                                wanted.area, ours.area, theirs.area
                            );
                            if let Err(e) = same_geometry(&ours, &wanted, 1e-3) {
                                problems.push(format!("engine {e}"));
                            }
                        }
                        let line = format!(
                            "{:<40} t={:<6} transparent float{shape}{}{report}",
                            p.name,
                            s.time,
                            unmodelled(&r)
                        );
                        eprintln!("{line}");
                        let eligible = !r.unmodelled.is_empty()
                            && problems.iter().all(|p| p.starts_with("unreferenced:"));
                        let owner = if r.unmodelled.iter().any(|r| r == "effects") {
                            "L6"
                        } else {
                            "L4"
                        };
                        let owned = pending.check(
                            pending::Entry {
                                frame: pending::frame_id(
                                    &p.name,
                                    s.time,
                                    s.width,
                                    s.height,
                                    "transparent",
                                ),
                                owner: owner.into(),
                                reason: r.unmodelled.join(", "),
                                checks: problems.clone(),
                                measurement_sha256: pending::measurement(
                                    &engine,
                                    &today,
                                    &r.rgba,
                                    &r.classes(),
                                ),
                            },
                            eligible,
                        );
                        if !problems.is_empty() && !owned {
                            failed.push(format!("{line}\n      {}", problems.join("\n      ")));
                        }
                    }
                }
            }
            compared += 1;
            let (line, pass) = match &three_d {
                // The 3D class (section 25) until the 3D lane converts it.
                Some((object, edges, class)) => {
                    let rest: Vec<bool> = class.iter().map(|c| !c).collect();
                    let (w, h) = (s.width as usize, s.height as usize);
                    let d = Diff::of_masked(&ours, &reference, &rest);
                    let b = bar_3d(&reference, &ours, edges, object, w, h);
                    let line = format!(
                        "{:<40} t={:<6} {:>7} px  ≤1 {:>7.3}%  max {} (2D); 3D bar ≤2 {:.4}% max {} max outside edges {} SSIM {:.5}",
                        p.name,
                        s.time,
                        d.pixels,
                        100.0 * d.fraction_within_one(),
                        d.max,
                        100.0 * b.fraction_within_2,
                        b.max_cv,
                        b.max_cv_outside_edges,
                        b.object_ssim.unwrap_or(f64::NAN)
                    );
                    let checks = if d.passes() && b.pass() {
                        Vec::new()
                    } else {
                        vec![format!(
                            "3D opaque: rest fraction within 1 {:.12}; rest max {}; fraction within 2 {:.12}; max outside edges {}; SSIM {:?}",
                            d.fraction_within_one(),
                            d.max,
                            b.fraction_within_2,
                            b.max_cv_outside_edges,
                            b.object_ssim
                        )]
                    };
                    let classes: Vec<_> = class
                        .iter()
                        .map(|v| {
                            if *v {
                                pm_reference::compare::Class::Resampled
                            } else {
                                pm_reference::compare::Class::Unreferenced
                            }
                        })
                        .collect();
                    let owned = pending.check(
                        pending::Entry {
                            frame: pending::frame_id(
                                &p.name,
                                s.time,
                                s.width,
                                s.height,
                                "3d-opaque",
                            ),
                            owner: "L4".into(),
                            reason: "3D surfaces and composition".into(),
                            checks: checks.clone(),
                            measurement_sha256: pending::measurement(
                                &unit8(&ours),
                                &unit8(&reference),
                                &[],
                                &classes,
                            ),
                        },
                        true,
                    );
                    (line, checks.is_empty() || owned)
                }
                None => {
                    let r = reference::render(
                        &p.document,
                        &p.composition,
                        ticks(s.time),
                        s.width,
                        s.height,
                        true,
                        16,
                        renderer.effects(),
                        &p.resolver,
                    );
                    let modelled = r.modelled();
                    complete += usize::from(modelled == 1.0);
                    partial += usize::from(modelled > 0.0 && modelled < 1.0);
                    let (engine, today) = (unit8(&ours), unit8(&reference));
                    let opaque = r.opaque_resolved(&engine);
                    if let Some(dir) = &diffs {
                        // The reference beside the engine's and today's
                        // frames; unreferenced pixels in magenta.
                        std::fs::create_dir_all(dir).unwrap();
                        let stem = format!("{}-{}", p.name.replace('/', "_"), s.time);
                        let bytes: Vec<u8> = opaque
                            .rgba
                            .chunks_exact(4)
                            .flat_map(|c| {
                                if c[3].is_nan() {
                                    [255, 0, 255, 255]
                                } else {
                                    let b = |v: f64| (v * 255.0).round() as u8;
                                    [b(c[0]), b(c[1]), b(c[2]), 255]
                                }
                            })
                            .collect();
                        write_png(&dir.join(format!("{stem}-reference.png")), w, h, &bytes);
                    }
                    let report = compare(&engine, &today, &opaque.rgba, &opaque.classes());
                    let problems = reference_problems(
                        &report,
                        &Bounds::EIGHT_BIT,
                        &engine,
                        &today,
                        &opaque.rgba,
                        &opaque.classes(),
                    );
                    let line = format!("{:<40} t={:<6}{}{report}", p.name, s.time, unmodelled(&r));
                    let eligible = !r.unmodelled.is_empty()
                        && problems.iter().all(|p| p.starts_with("unreferenced:"));
                    let owner = if r.unmodelled.iter().any(|r| r == "effects") {
                        "L6"
                    } else {
                        "L4"
                    };
                    let owned = pending.check(
                        pending::Entry {
                            frame: pending::frame_id(&p.name, s.time, s.width, s.height, "opaque"),
                            owner: owner.into(),
                            reason: r.unmodelled.join(", "),
                            checks: problems.clone(),
                            measurement_sha256: pending::measurement(
                                &engine,
                                &today,
                                &opaque.rgba,
                                &opaque.classes(),
                            ),
                        },
                        eligible,
                    );
                    let pass = problems.is_empty() || owned;
                    let line = if pass {
                        line
                    } else {
                        format!("{line}\n      {}", problems.join("\n      "))
                    };
                    (line, pass)
                }
            };
            eprintln!("{line}");
            if !pass {
                failed.push(line);
                // Side by side: the engine's frame, today's, and the pixels
                // more than 1 CV apart in red over a dimmed copy of today's.
                if let Some(dir) = &diffs {
                    std::fs::create_dir_all(dir).unwrap();
                    let stem = format!("{}-{}", p.name.replace('/', "_"), s.time);
                    write_png(&dir.join(format!("{stem}-engine.png")), w, h, &ours);
                    write_png(&dir.join(format!("{stem}-legacy.png")), w, h, &reference);
                    let marked: Vec<u8> = ours
                        .chunks_exact(4)
                        .zip(reference.chunks_exact(4))
                        .flat_map(|(a, b)| {
                            if a.iter().zip(b).any(|(x, y)| x.abs_diff(*y) > 1) {
                                [255, 0, 0, 255]
                            } else {
                                [b[0] / 3, b[1] / 3, b[2] / 3, 255]
                            }
                        })
                        .collect();
                    write_png(&dir.join(format!("{stem}-diff.png")), w, h, &marked);
                }
            }
            record_frame_result(
                &mut frame_results,
                &frame_id,
                failed.len() > failed_before,
                &ours,
            );
        }
    }
    failed.extend(pending.finish(only.is_none()));
    eprintln!("\nskipped (unsupported by this renderer):");
    for (name, why) in &skipped {
        eprintln!("  {name:<40} {why}");
    }
    eprintln!(
        "\ncompared {compared} frames (reference: {complete} opaque frames in full, {partial} in part); {} failed",
        failed.len()
    );
    for f in &failed {
        eprintln!("  FAILED {f}");
    }
    assert!(compared > 0, "no supported corpus project was compared");
    // The still classes (docs/engine/pm-render.md, "Image classes"): the
    // ones that matched are drawn, the others routed. A render capture must
    // hold every `render/still-*` project, so the pin cannot pass vacuously.
    let render_capture = captures.iter().any(|c| {
        c.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("render-"))
    });
    for (name, drawn) in STILL_CLASSES {
        if only.as_deref().is_some_and(|o| !name.starts_with(o)) {
            continue;
        }
        match seen.get(*name) {
            Some(supported) => assert_eq!(supported, drawn, "{name}: routing changed"),
            None => assert!(
                !render_capture,
                "{name}: not in the render capture (or its media is missing); recapture with \
                 `bun engine/corpus/src/capture.ts --only render/ --skip-real --skip-scenarios --label render`"
            ),
        }
    }
    assert!(
        failed.is_empty(),
        "{} frames outside the render class",
        failed.len()
    );
}

#[test]
fn interior_reference_proof_rejects_wrong_authored_paint() {
    use pm_reference::compare::{Class, compare};
    let today = [1.0, 1.0, 1.0, 1.0];
    let wanted = [0.0, 0.0, 0.0, 1.0];
    let report = compare(&wanted, &today, &wanted, &[Class::Interior]);
    assert!(
        reference_problems(
            &report,
            &Bounds::EIGHT_BIT,
            &wanted,
            &today,
            &wanted,
            &[Class::Interior]
        )
        .is_empty()
    );
    let wrong = [0.01, 0.0, 0.0, 1.0];
    let report = compare(&wrong, &today, &wanted, &[Class::Interior]);
    assert!(
        !reference_problems(
            &report,
            &Bounds::EIGHT_BIT,
            &wrong,
            &today,
            &wanted,
            &[Class::Interior]
        )
        .is_empty()
    );
    // A wrong legacy pixel elsewhere cannot hide a local regression.
    let report = compare(&wrong, &wanted, &wanted, &[Class::Interior]);
    assert!(
        !reference_problems(
            &report,
            &Bounds::EIGHT_BIT,
            &wrong,
            &wanted,
            &wanted,
            &[Class::Interior]
        )
        .is_empty()
    );
}

#[cfg(test)]
mod pending_tests {
    use super::pending::{Audit, Entry};
    fn row() -> Entry {
        Entry {
            frame: "anonymous".into(),
            owner: "L6".into(),
            reason: "effects".into(),
            checks: vec!["unreferenced: max 7 CV".into()],
            measurement_sha256: "measured".into(),
        }
    }
    fn audit() -> Audit {
        Audit::read(&serde_json::to_string(&[row()]).unwrap())
    }
    #[test]
    fn only_unchanged_owned_measurements_are_pending() {
        let mut a = audit();
        assert!(a.check(row(), true));
        assert!(a.finish(true).is_empty());
        let mut a = audit();
        let mut worse = row();
        worse.measurement_sha256 = "different".into();
        assert!(!a.check(worse, true));
        assert!(!a.finish(true).is_empty());
        let mut a = audit();
        assert!(!a.check(row(), false));
        assert!(!a.finish(true).is_empty());
    }
    #[test]
    fn unknown_resolved_and_unmeasured_are_not_exempt() {
        let mut a = audit();
        let mut new = row();
        new.frame = "new".into();
        assert!(!a.check(new, true));
        assert!(!a.finish(true).is_empty());
        let mut a = audit();
        let mut solved = row();
        solved.checks.clear();
        assert!(!a.check(solved, true));
        assert!(!a.finish(true).is_empty());
        assert!(!audit().finish(true).is_empty());
    }
}

#[test]
fn text_context_pending_requires_independent_local_proof() {
    use quality::text_context_is_owned_gap as eligible;
    assert!(eligible(false, true, 100, 100));
    assert!(eligible(false, false, 100, 0));
    assert!(!eligible(false, false, 100, 1));
    assert!(!eligible(true, true, 100, 0));
    assert!(!eligible(false, false, 0, 0));
}

#[test]
fn isolated_text_pairing_keeps_authored_values_exact() {
    use serde_json::json;
    let hash = |v: &serde_json::Value| {
        let value = pm_model::json::parse(&v.to_string()).unwrap();
        let doc = pm_model::Document::new(Some(value), 0);
        support::text::authored_isolation_fingerprint(&doc).unwrap()
    };
    let full = json!({"layers": [
        {"id":"ink", "type":"text", "threeD":true, "mblur":true,
         "d":{"text":"Authored", "size":32}, "p":{"position.x":{"v":20}},
         "fx":[{"type":"blur", "p":{"radius":{"kf":[{"i":"capture-a", "t":1, "v":2, "e":"linear"}]}}}]},
        {"id":"back", "type":"solid"}
    ]});
    let mut isolated = full.clone();
    isolated["layers"].as_array_mut().unwrap().pop();
    isolated["layers"][0]["threeD"] = json!(false);
    isolated["layers"][0]["mblur"] = json!(false);
    isolated["layers"][0]["fx"][0]["p"]["radius"]["kf"][0]["i"] = json!("capture-b");
    assert_eq!(hash(&full), hash(&isolated));
    for property in ["t", "v", "e"] {
        let mut changed = isolated.clone();
        changed["layers"][0]["fx"][0]["p"]["radius"]["kf"][0][property] = json!(9);
        assert_ne!(hash(&full), hash(&changed), "authored keyframe {property}");
    }
    isolated["layers"][0]["d"]["text"] = json!("Changed");
    assert_ne!(hash(&full), hash(&isolated), "authored text");
}

#[test]
fn corrected_interior_proof_is_local_to_pixels_outside_the_legacy_bound() {
    use pm_reference::compare::{Class, compare};
    let classes = [Class::Interior; 2];
    let wanted = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    let today = [1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    let mut engine = wanted;
    engine[4] = 0.8 / 255.0;
    let report = compare(&engine, &today, &wanted, &classes);
    assert!(
        reference_problems(
            &report,
            &Bounds::EIGHT_BIT,
            &engine,
            &today,
            &wanted,
            &classes
        )
        .is_empty()
    );
    engine[4] = 2.0 / 255.0;
    let report = compare(&engine, &today, &wanted, &classes);
    assert!(
        !reference_problems(
            &report,
            &Bounds::EIGHT_BIT,
            &engine,
            &today,
            &wanted,
            &classes
        )
        .is_empty()
    );
}
