//! Imported glTF models in the native 3D pass against three.js r180
//! (section 25's 3D class; docs/engine/gltf.md): every case of
//! `tests/fixtures/scene3d-gltf-cases.json` (`tools/three-ref/gltf-cases.ts`:
//! the S7 synthetic models and the Khronos sample models r180 loads, at
//! several animation times, with source and object materials, slots,
//! editor looks, depth sets, and a legacy scene) is rendered by the pass
//! and compared with three.js's render through the production
//! `SceneRuntime` and importer (`tools/three-ref/run-gltf.ts`, captured
//! outside the repository). Also compared: posed drawables (world, bone
//! and instance matrices, morph influences, embedded lights), programs and
//! uniforms of every draw, and picks on ray grids.
//!
//! `PM_GLTF_REF=<dir>` picks a capture (default: the newest
//! `gltf-scene3d-*`); `PM_GLTF_ONLY=<prefix>` limits the cases;
//! `PM_RENDER_DIFFS=<dir>` writes ours and theirs for failing outputs.

#[path = "shared/scene3d_reference.rs"]
mod float_reference;
mod support;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pm_model::json::{Map, Value};
use pm_reference::compare::{Bounds, Class, compare, geometry, same_sampled_geometry};
use pm_render::scene3d::describe::{self, DrawDesc, Layer3d, PassDesc};
use pm_render::scene3d::model::{
    Evaluated, LayerFacts, Look, MaterialKind, ModelLookup, ModelSource, NoTextures, Placement,
    Scene3d, SceneBuilder,
};
use pm_render::scene3d::{MathMode, Object3dPlan, Render3d, Scene3dRenderer};
use pm_render::three::materials::Side;
use pm_render::three::math::Mat4;
use pm_render::three::program::{ToneMapping, chunk, sources};

fn fixture() -> Vec<Value> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/scene3d-gltf-cases.json"
    );
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Value::Object(doc) = pm_model::json::parse(&text).unwrap() else {
        panic!("fixture")
    };
    match doc.get("cases") {
        Some(Value::Array(a)) => a.clone(),
        _ => panic!("cases"),
    }
}

fn capture() -> Option<PathBuf> {
    std::env::var_os("PM_GLTF_REF")
        .map(PathBuf::from)
        .or_else(|| support::newest_capture("gltf-scene3d-"))
}

fn obj(v: &Value) -> &Map {
    match v {
        Value::Object(m) => m,
        _ => panic!("object expected"),
    }
}

fn at<'a>(m: &'a Map, k: &str) -> &'a Value {
    m.get(k).unwrap_or_else(|| panic!("{k} missing"))
}

fn num(v: &Value) -> f64 {
    match v {
        Value::Number(n) => *n,
        _ => panic!("number expected"),
    }
}

fn nums(v: &Value) -> Vec<f64> {
    match v {
        Value::Array(a) => a.iter().map(num).collect(),
        _ => panic!("array expected"),
    }
}

fn text(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}

/// Model assets: id → file of the capture.
struct Models(HashMap<String, PathBuf>);

impl ModelSource for Models {
    fn model(&self, asset: &str) -> ModelLookup {
        match self.0.get(asset) {
            Some(p) => pm_render::scene3d::gltf::lookup(p),
            None => ModelLookup::Missing,
        }
    }
}

struct Case {
    id: String,
    time: f64,
    times: HashMap<String, f64>,
    matrices: HashMap<String, Mat4>,
    models: Models,
    outputs: Vec<(Object3dPlan, usize)>,
}

fn case(c: &Value, dir: &Path) -> Result<Case, String> {
    let c = obj(c);
    let id = text(c.get("id")).unwrap();
    let (w, h) = (num(at(c, "width")) as u32, num(at(c, "height")) as u32);
    let scene = obj(at(c, "scene"));
    let matrices: HashMap<String, Mat4> = c
        .get("matrices")
        .map(|m| {
            obj(m)
                .iter()
                .map(|(k, v)| (k.to_string_lossy(), nums(v).try_into().unwrap()))
                .collect()
        })
        .unwrap_or_default();
    let facts: HashMap<String, LayerFacts> = c
        .get("layers")
        .map(|m| {
            obj(m)
                .iter()
                .map(|(k, v)| {
                    let f = obj(v);
                    (
                        k.to_string_lossy(),
                        LayerFacts {
                            opacity: num(at(f, "opacity")),
                            occludes: matches!(f.get("occludes"), Some(Value::Bool(true))),
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let models = Models(
        obj(at(c, "models"))
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string_lossy(),
                    dir.join("models").join(text(Some(v)).unwrap()),
                )
            })
            .collect(),
    );
    let time = num(at(c, "time"));
    let times: HashMap<String, f64> = c
        .get("animationTimes")
        .map(|m| {
            obj(m)
                .iter()
                .map(|(k, v)| (k.to_string_lossy(), num(v)))
                .collect()
        })
        .unwrap_or_default();
    let lights = HashMap::new();
    let placement = if matrices.is_empty() {
        Placement::Channels
    } else {
        Placement::Composition {
            objects: &matrices,
            lights: &lights,
            camera: None,
        }
    };
    let mut builder = SceneBuilder::default();
    builder.set_animation_times(time, times.clone());
    let base = builder.build(
        scene,
        w,
        h,
        &placement,
        &facts,
        &mut Evaluated,
        &NoTextures,
        &models,
    )?;
    let mut scenes: Vec<(String, Arc<Scene3d>)> = Vec::new();
    let mut outputs = Vec::new();
    let Value::Array(list) = at(c, "outputs") else {
        panic!("outputs")
    };
    for (k, o) in list.iter().enumerate() {
        let o = obj(o);
        let look = o.get("look").map(obj);
        let flag = |n: &str| look.is_some_and(|l| matches!(l.get(n), Some(Value::Bool(true))));
        let look = Look {
            wireframe: flag("wireframe"),
            xray: flag("xray"),
        };
        let key = format!("{look:?}");
        let scene = match scenes.iter().find(|(k, _)| *k == key) {
            Some((_, s)) => s.clone(),
            None => {
                let mut s = base.clone();
                s.apply_look(look);
                let s = Arc::new(s);
                scenes.push((key, s.clone()));
                s
            }
        };
        let index = |name: &str| scene.objects.iter().position(|o| o.id == name).unwrap();
        let kind = match text(o.get("object")) {
            Some(object) => {
                let mut set: Vec<usize> = match o.get("depthIds") {
                    Some(Value::Array(ids)) => {
                        ids.iter().map(|i| index(&text(Some(i)).unwrap())).collect()
                    }
                    _ => Vec::new(),
                };
                set.sort_unstable();
                Render3d::Layer {
                    object: index(&object),
                    depth_set: set,
                    background: matches!(o.get("background"), Some(Value::Bool(true))),
                }
            }
            None => Render3d::Whole,
        };
        outputs.push((
            Object3dPlan {
                layer: format!("{id}/{k}"),
                scene,
                kind,
            },
            k,
        ));
    }
    Ok(Case {
        id,
        time,
        times,
        matrices,
        models,
        outputs,
    })
}

/// The selected cases of the capture; refused models (the importer threw)
/// must also be refused here.
fn selected() -> Option<(PathBuf, Vec<Case>)> {
    let dir = capture()?;
    let only = std::env::var("PM_GLTF_ONLY").ok();
    let mut out = Vec::new();
    for c in fixture() {
        let id = text(obj(&c).get("id")).unwrap();
        if only.as_deref().is_some_and(|o| !id.starts_with(o)) {
            continue;
        }
        let refused = dir.join(format!("{id}.refused.txt")).exists();
        // The reader refuses a file it does not load (the missing-model
        // box, as today's asset without a scene).
        let reader_refuses = |c: &Case| {
            c.models.0.values().any(|p| {
                p.exists() && !matches!(pm_render::scene3d::gltf::lookup(p), ModelLookup::Gltf(_))
            })
        };
        match case(&c, &dir) {
            Ok(c) if refused && reader_refuses(&c) => eprintln!("{id}: refused by both"),
            Ok(_) if refused => panic!("{id}: three.js refused the model; the engine loads it"),
            Ok(c) if reader_refuses(&c) => {
                panic!("{id}: the engine refuses a model three.js loads")
            }
            Ok(c) => out.push(c),
            Err(e) if refused => eprintln!("{id}: refused by both ({e})"),
            Err(e) => panic!("{id}: {e}"),
        }
    }
    Some((dir, out))
}

#[test]
fn gltf_sweep_resolves_finite_linear_color() {
    let Some((_, cases)) = selected() else {
        eprintln!("no glTF scene assets; run tools/three-ref/run-gltf.ts");
        return;
    };
    let gpu = pm_gpu::Gpu::new("pm-render glTF linear sweep").expect("a Metal device");
    let mut renderer = Scene3dRenderer::default();
    renderer.set_math_mode(MathMode::Safe);
    let mut pool = pm_gpu::TargetPool::new();
    let mut compared = 0;
    for c in &cases {
        for (plan, _) in &c.outputs {
            renderer.begin_frame();
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            let out = renderer
                .render(&gpu.device, &gpu.queue, &mut encoder, &mut pool, plan, true)
                .unwrap_or_else(|e| panic!("{}: {e}", plan.layer));
            gpu.queue.submit([encoder.finish()]);
            assert_eq!(out.color.format(), wgpu::TextureFormat::Rgba16Float);
            assert!(matches!(
                out.color_space,
                pm_render::scene3d::OutputColor3d::LinearScene { .. }
            ));
            let color =
                pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &out.color.texture).unwrap();
            assert!(
                color
                    .chunks_exact(4)
                    .all(|p| p.iter().all(|v| v.is_finite()) && (0.0..=1.0).contains(&p[3])),
                "{}: finite premultiplied color",
                plan.layer
            );
            let depth = pm_gpu::readback::read_f32(
                &gpu.device,
                &gpu.queue,
                &out.depth.as_ref().unwrap().texture,
            )
            .unwrap();
            assert_eq!(depth.len(), color.len() / 4, "{}: depth size", plan.layer);
            compared += 1;
            pool.give(out.color);
            pool.give(out.depth.unwrap());
        }
    }
    assert!(compared > 0);
}

/// G-D1: native and today's displayed imported-model outputs against an
/// independent r180 RGBA32F scene at 8x, resolved in linear light and
/// displayed through f64 ACES and sRGB. Poses and ray-grid picks are checked
/// by their separate exact tests.
#[test]
fn gltf_displayed_pixels_prefer_supersampled_float_reference() {
    let Some((capture, cases)) = selected() else {
        eprintln!("no glTF scene assets; run tools/three-ref/run-gltf.ts");
        return;
    };
    let path = std::env::var_os("PM_GLTF_FLOAT_REF")
        .map(PathBuf::from)
        .expect("G-D1 requires PM_GLTF_FLOAT_REF (examples/scene3d_float_reference.ts --gltf)");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(path.join("manifest.json")).expect("glTF float reference manifest"),
    )
    .unwrap();
    assert_eq!(manifest["three"], "0.180.0");
    assert_eq!(manifest["scale"], 8);
    assert_eq!(manifest["format"], "rgba32float");
    assert_eq!(
        manifest["captureId"].as_str(),
        capture.file_name().and_then(|n| n.to_str())
    );
    let gpu =
        pm_gpu::Gpu::new("pm-render glTF independent float reference").expect("a Metal device");
    let mut renderer = Scene3dRenderer::default();
    renderer.set_math_mode(MathMode::Safe);
    let mut pool = pm_gpu::TargetPool::new();
    let mut compared = 0;
    let mut failures = Vec::new();
    for c in &cases {
        for (plan, k) in &c.outputs {
            let (w, h) = plan.scene.canvas();
            let bytes = std::fs::read(path.join(&c.id).join(format!("{k}.rgba32f")))
                .unwrap_or_else(|e| panic!("{}/{} reference: {e}", c.id, k));
            assert_eq!(bytes.len(), w as usize * h as usize * 16, "{}/{}", c.id, k);
            let reference: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            renderer.begin_frame();
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            let out = renderer
                .render(
                    &gpu.device,
                    &gpu.queue,
                    &mut encoder,
                    &mut pool,
                    plan,
                    false,
                )
                .unwrap_or_else(|e| panic!("{}: {e}", plan.layer));
            gpu.queue.submit([encoder.finish()]);
            let mut raw =
                pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &out.color.texture).unwrap();
            if let pm_render::scene3d::OutputColor3d::LinearScene { weighted: true, .. } =
                out.color_space
            {
                pm_render::scene3d::karis::unweight(&mut raw, plan.scene.exposure as f32);
            }
            let actual = float_reference::top_down_linear(raw, w as usize, h as usize);
            assert_eq!(actual.len(), reference.len());
            assert!(
                reference.iter().all(|v| v.is_finite()),
                "{} reference nonfinite",
                plan.layer
            );
            let actual_alpha: Vec<_> = actual.chunks_exact(4).map(|p| f64::from(p[3])).collect();
            let reference_alpha: Vec<_> =
                reference.chunks_exact(4).map(|p| f64::from(p[3])).collect();
            if let Err(error) = same_sampled_geometry(
                &geometry(&actual_alpha, w as usize),
                &geometry(&reference_alpha, w as usize),
                &reference_alpha,
                w as usize,
                4,
            ) {
                failures.push(format!("{}:{} {error}", c.id, k));
            }
            let background = match &plan.kind {
                Render3d::Whole
                | Render3d::Layer {
                    background: true, ..
                } => plan.scene.background,
                _ => None,
            };
            let modeled = float_reference::display(&reference, plan.scene.exposure, background);
            let engine = float_reference::display(&actual, plan.scene.exposure, background);
            let (tw, th, today_bytes) =
                support::read_png(&capture.join(&c.id).join(format!("{k}.msaa.png")));
            assert_eq!((tw, th), (w, h), "{}:{} today size", c.id, k);
            let today: Vec<_> = today_bytes
                .into_iter()
                .map(|v| f64::from(v) / 255.0)
                .collect();
            let classes = float_reference::classes(&modeled, &reference, w as usize, h as usize);
            let report = compare(&engine, &today, &modeled, &classes);
            eprintln!("glTF float display {}:{} {report}", c.id, k);
            // A 3D correction (quality ledger, "3D output and Library
            // cards") moves interior shading away from today only toward
            // the reference: accepted where no interior pixel ends further
            // from it than today's by more than the interior bound.
            let corrected = report.class(Class::Interior).excess.max <= Bounds::MSAA4.interior_cv;
            for failure in report.failures(&Bounds::MSAA4) {
                if corrected && failure.starts_with("interior:") {
                    eprintln!("corrected interior {}:{} {failure}", c.id, k);
                    continue;
                }
                failures.push(format!("{}:{} {failure}", c.id, k));
            }
            compared += 1;
            pool.give(out.color);
        }
    }
    let selected_outputs: usize = cases.iter().map(|case| case.outputs.len()).sum();
    assert!(
        selected_outputs > 0,
        "no independent glTF references selected"
    );
    assert_eq!(compared, selected_outputs);
    if std::env::var("PM_GLTF_ONLY").is_err() {
        assert!(
            compared >= 150,
            "only {compared} independent glTF references compared"
        );
    }
    assert!(
        failures.is_empty(),
        "G-D1 independent displayed reference deviations:\n{}",
        failures.join("\n")
    );
}

fn close(a: f64, b: f64, rel: f64) -> bool {
    a == b || (a - b).abs() <= rel * a.abs().max(b.abs()).max(1.0)
}

/// Posed drawables and embedded lights equal three.js's: world matrices,
/// morph influences, bind matrices, bone matrices (f32), instance
/// matrices (f32), light positions and targets.
#[test]
fn gltf_poses_equal_three_js() {
    let Some((dir, cases)) = selected() else {
        eprintln!("no glTF reference capture; run tools/three-ref/run-gltf.ts");
        return;
    };
    let mut problems = Vec::new();
    let (mut meshes, mut values, mut worst) = (0usize, 0usize, 0f64);
    for c in &cases {
        let path = dir.join(&c.id).join("pose.json");
        let Ok(t) = std::fs::read_to_string(&path) else {
            continue;
        };
        let pose: serde_json::Value = serde_json::from_str(&t).unwrap();
        let scene = &c.outputs[0].0.scene;
        for (layer, rec) in pose.as_object().unwrap() {
            let Some(o) = scene.objects.iter().find(|o| &o.id == layer) else {
                problems.push(format!("{}: no object {layer}", c.id));
                continue;
            };
            let recorded = rec["meshes"].as_array().unwrap();
            if recorded.len() != o.meshes.len() {
                problems.push(format!(
                    "{} {layer}: {} drawables, three.js {}",
                    c.id,
                    o.meshes.len(),
                    recorded.len()
                ));
                continue;
            }
            for (k, (m, r)) in o.meshes.iter().zip(recorded).enumerate() {
                meshes += 1;
                let mut cmp = |what: &str, ours: &[f64], theirs: &[f64], rel: f64| {
                    if ours.len() != theirs.len() {
                        problems.push(format!(
                            "{} {layer} #{k} {what}: {} values, three.js {}",
                            c.id,
                            ours.len(),
                            theirs.len()
                        ));
                        return;
                    }
                    for (a, b) in ours.iter().zip(theirs) {
                        values += 1;
                        let d = (a - b).abs() / a.abs().max(b.abs()).max(1.0);
                        worst = worst.max(d);
                        if !close(*a, *b, rel) {
                            problems.push(format!(
                                "{} {layer} #{k} {what}: ours {a} three.js {b}",
                                c.id
                            ));
                            return;
                        }
                    }
                };
                let f = |v: &serde_json::Value| -> Vec<f64> {
                    v.as_array()
                        .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(f64::NAN)).collect())
                        .unwrap_or_default()
                };
                cmp("matrixWorld", &m.world, &f(&r["matrixWorld"]), 1e-12);
                let g = m.gltf.as_ref().unwrap();
                if !r["influences"].is_null() {
                    cmp("influences", &g.influences, &f(&r["influences"]), 1e-12);
                }
                if let Some(s) = &g.skinning {
                    cmp(
                        "bindMatrixInverse",
                        &s.bind_matrix_inverse,
                        &f(&r["bindMatrixInverse"]),
                        1e-12,
                    );
                    let theirs = f(&r["boneMatrices"]);
                    let ours: Vec<f64> = s.bones[..theirs.len().min(s.bones.len())]
                        .iter()
                        .map(|v| f64::from(*v))
                        .collect();
                    cmp("boneMatrices", &ours, &theirs, 1e-6);
                }
                if let Some((_, i)) = &g.instances {
                    let ours: Vec<f64> = i
                        .matrices
                        .iter()
                        .flat_map(|m| m.iter().map(|v| f64::from(*v)))
                        .collect();
                    cmp("instanceMatrix", &ours, &f(&r["instanceMatrix"]), 0.0);
                }
                let kind = r["kind"].as_str().unwrap();
                let expect = match kind {
                    "points" => MaterialKind::Points,
                    "line" | "lineLoop" | "lineSegments" => MaterialKind::Line,
                    _ => o.meshes[k]
                        .materials
                        .iter()
                        .next()
                        .map_or(MaterialKind::Standard, |m| m.kind),
                };
                if let Some(mat) = m.materials.iter().next()
                    && matches!(expect, MaterialKind::Points | MaterialKind::Line)
                    && mat.kind != expect
                {
                    problems.push(format!(
                        "{} {layer} #{k}: material {:?}, three.js {kind}",
                        c.id, mat.kind
                    ));
                }
                if (kind == "skinned") != g.skinned {
                    problems.push(format!(
                        "{} {layer} #{k}: skinned {}, three.js {kind}",
                        c.id, g.skinned
                    ));
                }
            }
            // Embedded lights.
            let embedded: Vec<_> = scene
                .lights
                .iter()
                .filter(|l| l.id.starts_with(&format!("{layer}#light")))
                .collect();
            let rl = rec["lights"].as_array().unwrap();
            if embedded.len() != rl.len() {
                problems.push(format!(
                    "{} {layer}: {} embedded lights, three.js {}",
                    c.id,
                    embedded.len(),
                    rl.len()
                ));
            }
            for (l, r) in embedded.iter().zip(rl) {
                let mw: Vec<f64> = r["matrixWorld"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_f64().unwrap())
                    .collect();
                for k in 0..3 {
                    if !close(l.world[12 + k], mw[12 + k], 1e-12) {
                        problems.push(format!(
                            "{} {layer}: light position {:?} three.js {:?}",
                            c.id,
                            &l.world[12..15],
                            &mw[12..15]
                        ));
                        break;
                    }
                }
                if let Some(t) = r["target"].as_array() {
                    let t: Vec<f64> = t.iter().map(|x| x.as_f64().unwrap()).collect();
                    for k in 0..3 {
                        if !close(l.target[k], t[12 + k], 1e-12) {
                            problems.push(format!(
                                "{} {layer}: light target {:?} three.js {:?}",
                                c.id,
                                l.target,
                                &t[12..15]
                            ));
                            break;
                        }
                    }
                }
                if !close(l.intensity, r["intensity"].as_f64().unwrap(), 0.0) {
                    problems.push(format!("{} {layer}: light intensity", c.id));
                }
            }
        }
    }
    eprintln!(
        "{meshes} drawables, {values} pose values compared; largest relative difference {worst:e}"
    );
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems
            .iter()
            .take(60)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The draws of each output, as three.js issues them.
fn our_draws(c: &Case) -> Vec<(usize, Vec<(DrawDesc, bool)>)> {
    let mut out = Vec::new();
    let mut previous: Option<(*const Scene3d, Vec<usize>)> = None;
    for (plan, k) in &c.outputs {
        let mut draws = Vec::new();
        let mut push = |p: PassDesc| match p {
            PassDesc::Shadow { views, .. } => {
                for (_, d) in views {
                    draws.extend(d.into_iter().map(|draw| (draw, false)));
                }
            }
            PassDesc::Transmission { opaque, back, .. } => {
                draws.extend(opaque.into_iter().map(|draw| (draw, true)));
                draws.extend(back.into_iter().map(|draw| (draw, true)));
            }
            PassDesc::Prepass(d) | PassDesc::Color { draws: d, .. } => {
                draws.extend(d.into_iter().map(|draw| (draw, false)));
            }
        };
        match &plan.kind {
            Render3d::Layer {
                object,
                depth_set,
                background,
            } => {
                let layer = Layer3d {
                    scene: &plan.scene,
                    object: *object,
                    depth_set,
                    background: *background,
                };
                let key = (Arc::as_ptr(&plan.scene), depth_set.clone());
                if previous.as_ref() != Some(&key) {
                    for p in describe::prepass(&layer).unwrap() {
                        push(p);
                    }
                    previous = Some(key);
                }
                for p in describe::color_passes(&layer).unwrap() {
                    push(p);
                }
            }
            Render3d::Whole => {
                previous = None;
                for p in describe::whole_scene(&plan.scene).unwrap() {
                    push(p);
                }
            }
            Render3d::ObjMesh(_) => {}
        }
        out.push((*k, draws));
    }
    out
}

fn normalized(src: &str, shadow: bool) -> String {
    src.lines()
        .filter(|l| !l.starts_with("#define SHADER_NAME"))
        .filter(|l| !(shadow && (*l == "#define DOUBLE_SIDED" || *l == "#define FLIP_SIDED")))
        // The shared depth and distance materials keep the map of their
        // first build (no `needsUpdate` when a caster's map changes); they
        // pack depth only, so the map is pixel-irrelevant without alpha test.
        .filter(|l| {
            !(shadow
                && (*l == "#define USE_MAP"
                    || l.starts_with("#define MAP_UV")
                    || l.starts_with("#define USE_UV")))
        })
        .map(|l| {
            if shadow && l.trim_start().starts_with("#if") {
                l.chars()
                    .map(|c| if c.is_ascii_digit() { 'N' } else { c })
                    .collect()
            } else {
                l.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Restore only r180's color-output transfer or packed shadow storage for
/// linked-program comparison. The transmission target's untoned program and
/// the canvas program are distinct draws, even when their matrices coincide.
/// Geometry, material, light, texture and clipping source remains exact.
fn reference_program_sources(d: &DrawDesc, transmission: bool) -> Vec<(String, String)> {
    let mut p = d.params.clone();
    match p.shader_type.as_str() {
        "MeshDepthMaterial" => {
            p.fragment_shader = chunk("depth_frag").expect("r180 depth fragment").into();
            p.use_depth_packing = true;
            p.depth_packing = 3201;
            let (vs, fs) = sources(&p).expect("r180 depth program");
            vec![(vs, float_reference::restore_linear_output(fs))]
        }
        "MeshDistanceMaterial" => {
            p.fragment_shader = chunk("distanceRGBA_frag")
                .expect("r180 distance fragment")
                .into();
            let (vs, fs) = sources(&p).expect("r180 distance program");
            vec![(vs, float_reference::restore_linear_output(fs))]
        }
        _ => {
            let (vs, fs) = sources(&p).expect("r180 transmission program");
            let linear = (vs, float_reference::restore_linear_output(fs));
            if transmission {
                return vec![linear];
            }
            p.tone_mapping = ToneMapping::AcesFilmic;
            p.output_srgb = true;
            vec![sources(&p).expect("r180 canvas program")]
        }
    }
}

fn f32s(v: &serde_json::Value) -> Vec<f32> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap_or(f64::NAN) as f32)
        .collect()
}

/// Every draw uses a program byte-identical to one three.js linked, with
/// every uniform three.js set equal as f32, in three.js's order; every
/// recorded draw that writes color or depth is one of ours (skinned meshes
/// three.js culls with its stale sphere excepted).
#[test]
fn gltf_programs_and_uniforms_equal_three_js() {
    let Some((dir, cases)) = selected() else {
        eprintln!("no glTF reference capture; run tools/three-ref/run-gltf.ts");
        return;
    };
    let mut problems = Vec::new();
    let (mut matched, mut values, mut extra, mut depth_phase) = (0, 0, 0, 0);
    for c in &cases {
        for (k, ours) in our_draws(c) {
            let Ok(t) = std::fs::read_to_string(dir.join(&c.id).join(format!("{k}.json"))) else {
                continue;
            };
            let record: serde_json::Value = serde_json::from_str(&t).unwrap();
            let programs: HashMap<u64, (String, String)> = record["programs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    (
                        p["id"].as_u64().unwrap(),
                        (
                            p["vertex"].as_str().unwrap().to_owned(),
                            p["fragment"].as_str().unwrap().to_owned(),
                        ),
                    )
                })
                .collect();
            let recorded = record["draws"].as_array().unwrap();
            let mut used = vec![false; recorded.len()];
            let mut last: Option<usize> = None;
            for (n, (d, transmission)) in ours.iter().enumerate() {
                let shadow = matches!(
                    d.params.shader_type.as_str(),
                    "MeshDepthMaterial" | "MeshDistanceMaterial"
                );
                let program_forms: Vec<_> = reference_program_sources(d, *transmission)
                    .into_iter()
                    .map(|(vs, fs)| (normalized(&vs, shadow), normalized(&fs, shadow)))
                    .collect();
                let program_matches = |v: &str, f: &str| {
                    program_forms.iter().any(|(vs, fs)| {
                        normalized(v, shadow) == *vs && normalized(f, shadow) == *fs
                    })
                };
                let mine = |name: &str| {
                    d.uniforms
                        .iter()
                        .find(|(k, _)| k == name)
                        .map(|(_, v)| v.iter().map(|x| *x as f32).collect::<Vec<f32>>())
                };
                let keys = [
                    "modelMatrix",
                    "modelViewMatrix",
                    "projectionMatrix",
                    "viewMatrix",
                ];
                let same = |r: &serde_json::Value| {
                    keys.iter().all(|k| match (r["uniforms"].get(*k), mine(k)) {
                        (Some(x), Some(m)) => f32s(x) == m,
                        _ => true,
                    })
                };
                let candidate = |r: &serde_json::Value| {
                    let color = r["colorMask"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|b| b.as_bool() == Some(true));
                    let (rv, rf) = &programs[&r["program"].as_u64().unwrap()];
                    // r180's packed shadow target wrote color; the live
                    // depth texture changes storage, not the draw's phase.
                    (if shadow {
                        r["depthMask"].as_bool() == Some(true)
                    } else {
                        color == d.state.color_write
                    }) && program_matches(rv, rf)
                };
                let Some(i) = recorded
                    .iter()
                    .enumerate()
                    .position(|(i, r)| !used[i] && candidate(r) && same(r))
                else {
                    let mesh = &c
                        .outputs
                        .iter()
                        .find(|(_, kk)| *kk == k)
                        .unwrap()
                        .0
                        .scene
                        .objects[d.object]
                        .meshes[d.mesh];
                    if mesh.gltf.as_ref().is_some_and(|g| g.skinned) {
                        extra += 1;
                        continue;
                    }
                    let program_known = programs.values().any(|(v, f)| program_matches(v, f));
                    if std::env::var_os("PM_GLTF_DEBUG").is_some() {
                        eprintln!(
                            "{} {k} draw {n}: object {} mesh {} model {:?}",
                            c.id,
                            d.object,
                            d.mesh,
                            mine("modelMatrix").map(|m| m[12..15].to_vec())
                        );
                        for (i, r) in recorded.iter().enumerate() {
                            if !used[i] && candidate(r) {
                                eprintln!(
                                    "   candidate {i}: {:?}",
                                    r["uniforms"]
                                        .get("modelMatrix")
                                        .map(|m| f32s(m)[12..15].to_vec())
                                );
                            }
                        }
                    }
                    if !program_known && std::env::var_os("PM_GLTF_PROGRAMS").is_some() {
                        let p = std::env::var_os("PM_GLTF_PROGRAMS")
                            .map(PathBuf::from)
                            .expect("program dump directory");
                        std::fs::create_dir_all(&p).unwrap();
                        let (vs, fs) = &program_forms[0];
                        std::fs::write(p.join(format!("{}-{k}-{n}.vs", c.id)), vs).unwrap();
                        std::fs::write(p.join(format!("{}-{k}-{n}.fs", c.id)), fs).unwrap();
                        for (id, (rv, rf)) in &programs {
                            std::fs::write(p.join(format!("{}-{k}-rec{id}.vs", c.id)), rv).unwrap();
                            std::fs::write(p.join(format!("{}-{k}-rec{id}.fs", c.id)), rf).unwrap();
                        }
                    }
                    problems.push(format!(
                        "{} output {k} draw {n} ({} {:?}, color {}): {}",
                        c.id,
                        d.params.shader_type,
                        d.params.shader_name,
                        d.state.color_write,
                        if program_known {
                            "no recorded draw with these matrices"
                        } else {
                            "no recorded program equals ours"
                        }
                    ));
                    continue;
                };
                used[i] = true;
                matched += 1;
                if std::env::var_os("PM_GLTF_ORDER").is_some() {
                    eprintln!(
                        "{} {k}: ours {n} -> recorded {i} {} {:?} color {}",
                        c.id, d.params.shader_type, d.params.shader_name, d.state.color_write
                    );
                }
                if last.is_some_and(|l| i < l) {
                    problems.push(format!(
                        "{} output {k} draw {n}: out of three.js's order",
                        c.id
                    ));
                }
                last = Some(i);
                let textures: Vec<&str> = d.textures.iter().map(|(n, _)| n.as_str()).collect();
                for (name, value) in recorded[i]["uniforms"].as_object().unwrap() {
                    let base = name.split('[').next().unwrap();
                    let sampler = textures.iter().any(|t| t.split('[').next() == Some(base))
                        || base.ends_with("Map")
                        || base == "map"
                        || matches!(
                            base,
                            "directionalShadowMap"
                                | "spotShadowMap"
                                | "pointShadowMap"
                                | "ltc_1"
                                | "ltc_2"
                                | "boneTexture"
                                | "morphTargetsTexture"
                                | "transmissionSamplerMap"
                        );
                    if sampler || (shadow && name == "mapTransform") {
                        continue;
                    }
                    let theirs = f32s(value);
                    let ours_v = match name.strip_suffix("[0]") {
                        Some(stem) if !name.contains('.') => {
                            let mut all = Vec::new();
                            for e in 0.. {
                                match d
                                    .uniforms
                                    .iter()
                                    .find(|(k, _)| *k == format!("{stem}[{e}]"))
                                {
                                    Some((_, v)) => all.extend(v.iter().map(|x| *x as f32)),
                                    None => break,
                                }
                            }
                            if all.is_empty() {
                                mine(stem).or_else(|| mine(name))
                            } else {
                                Some(all)
                            }
                        }
                        _ => mine(name).or_else(|| mine(&format!("{name}[0]"))),
                    };
                    let Some(ours_v) = ours_v else {
                        problems.push(format!(
                            "{} output {k} draw {n}: three.js sets {name}, the pass does not",
                            c.id
                        ));
                        continue;
                    };
                    // JSON writes Infinity (`attenuationDistance` of a volume
                    // without one) as null.
                    let infinite: Vec<bool> = value
                        .as_array()
                        .map(|a| a.iter().map(serde_json::Value::is_null).collect())
                        .unwrap_or_default();
                    for (e, (a, b)) in ours_v.iter().zip(&theirs).enumerate() {
                        values += 1;
                        if infinite.get(e) == Some(&true) && !a.is_finite() {
                            continue;
                        }
                        if !(a.to_bits() == b.to_bits() || a == b) {
                            problems.push(format!(
                                "{} output {k} draw {n}: {name} ours {a} three.js {b}",
                                c.id
                            ));
                            break;
                        }
                    }
                }
            }
            for (i, r) in recorded.iter().enumerate() {
                let effective = r["depthMask"].as_bool() == Some(true)
                    || r["colorMask"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|b| b.as_bool() == Some(true));
                // The depth phase's transmission pass (three.js runs it in
                // every render with transmissive objects): untoned shading
                // programs into a target the color phase clears and redraws.
                let (_, rf) = &programs[&r["program"].as_u64().unwrap()];
                let untoned = !rf.contains("#define TONE_MAPPING")
                    && !rf.contains("MeshDepthMaterial")
                    && !rf.contains("MeshDistanceMaterial");
                if !used[i] && effective && untoned {
                    depth_phase += 1;
                    continue;
                }
                if !used[i] && effective {
                    problems.push(format!(
                        "{} output {k}: recorded draw {i} (program {}) has no counterpart",
                        c.id, r["program"]
                    ));
                }
            }
        }
    }
    eprintln!(
        "{matched} draws matched, {values} uniform values compared, {extra} skinned draws three.js may cull, {depth_phase} depth-phase transmission draws"
    );
    if let Some(p) = std::env::var_os("PM_GLTF_PROBLEMS") {
        std::fs::write(p, problems.join("\n")).unwrap();
    }
    if !problems.is_empty() {
        problems.dedup();
        panic!(
            "{} problems:\n{}",
            problems.len(),
            problems
                .iter()
                .take(80)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

/// Picks on ray grids equal three.js's raycasts (same layer, object, and
/// face or point index; distance within 1e-6 relative).
#[test]
fn gltf_picks_equal_three_js() {
    use pm_geom3d::gltf::pick::{Picker, Ray, Side as PickSide};
    let Some((dir, cases)) = selected() else {
        eprintln!("no glTF reference capture; run tools/three-ref/run-gltf.ts");
        return;
    };
    let (mut rays, mut hits, mut agree) = (0usize, 0usize, 0usize);
    let mut problems = Vec::new();
    for c in &cases {
        let Ok(t) = std::fs::read_to_string(dir.join(&c.id).join("picks.json")) else {
            continue;
        };
        let rec: serde_json::Value = serde_json::from_str(&t).unwrap();
        let scene = &c.outputs[0].0.scene;
        // Every model layer: its picker, pose, and material sides.
        let mut layers = Vec::new();
        for (id, path) in &c.models.0 {
            let Some(o) = scene.objects.iter().find(|o| &o.id == id) else {
                continue;
            };
            let ModelLookup::Gltf(asset) = pm_render::scene3d::gltf::lookup(path) else {
                continue;
            };
            let picker = Picker::new(asset.model.clone());
            let time = c.times.get(id).copied().unwrap_or(c.time).max(0.0);
            let posed = picker.pose(&c.matrices[id], time);
            let drawables: Vec<usize> = asset
                .model
                .traversal()
                .into_iter()
                .filter(|&i| {
                    matches!(
                        asset.model.objects[i].kind,
                        pm_geom3d::gltf::ObjectKind::Drawable(_)
                    )
                })
                .collect();
            let sides: HashMap<usize, (PickSide, bool)> = drawables
                .iter()
                .zip(&o.meshes)
                .map(|(&i, m)| {
                    let mats: Vec<_> = m.materials.iter().collect();
                    let side = match mats.first().map(|m| m.side) {
                        Some(Side::Double) => PickSide::Double,
                        Some(Side::Back) => PickSide::Back,
                        _ => PickSide::Front,
                    };
                    let none = mats.iter().all(|m| m.opacity <= 0.0);
                    (i, (side, none))
                })
                .collect();
            layers.push((id.clone(), asset, picker, posed, sides, o.visible));
        }
        let near = rec["near"].as_f64().unwrap();
        let far = rec["far"].as_f64().unwrap_or(f64::INFINITY);
        for r in rec["rays"].as_array().unwrap() {
            rays += 1;
            let v3 = |v: &serde_json::Value| -> [f64; 3] {
                let a = v.as_array().unwrap();
                [
                    a[0].as_f64().unwrap(),
                    a[1].as_f64().unwrap(),
                    a[2].as_f64().unwrap(),
                ]
            };
            let ray = Ray {
                origin: v3(&r["origin"]),
                direction: v3(&r["direction"]),
            };
            let mut best: Option<(String, String, pm_geom3d::gltf::pick::Hit)> = None;
            for (id, asset, picker, posed, sides, visible) in &layers {
                if !visible {
                    continue;
                }
                let side = |i: usize| sides.get(&i).map_or(PickSide::Front, |s| s.0);
                let skip = |i: usize| sides.get(&i).is_none_or(|s| s.1);
                if let Some(h) = picker.pick(posed, &ray, near, far, &side, &skip)
                    && best.as_ref().is_none_or(|b| h.distance < b.2.distance)
                {
                    best = Some((id.clone(), asset.model.objects[h.object].name.clone(), h));
                }
            }
            let theirs = &r["hit"];
            match (&best, theirs.is_null()) {
                (None, true) => agree += 1,
                (Some((layer, name, h)), false) => {
                    hits += 1;
                    let d = theirs["distance"].as_f64().unwrap();
                    let index = theirs["faceIndex"].as_u64().or(theirs["index"].as_u64());
                    let ok = theirs["layer"].as_str() == Some(layer)
                        && theirs["object"].as_str() == Some(name)
                        && close(h.distance, d, 1e-6)
                        && index.is_none_or(|i| i == u64::from(h.index))
                        && theirs["instanceId"].as_u64() == h.instance.map(u64::from);
                    if ok {
                        agree += 1;
                    } else {
                        problems.push(format!(
                            "{} ray ({:.3}, {:.3}): ours {layer}/{name} at {} #{} {:?}, three.js {}",
                            c.id, r["x"], r["y"], h.distance, h.index, h.instance, theirs
                        ));
                    }
                }
                (Some((layer, name, h)), true) => problems.push(format!(
                    "{} ray ({}, {}): ours {layer}/{name} at {}, three.js none",
                    c.id, r["x"], r["y"], h.distance
                )),
                (None, false) => problems.push(format!(
                    "{} ray ({}, {}): ours none, three.js {}",
                    c.id, r["x"], r["y"], theirs
                )),
            }
        }
    }
    eprintln!("{rays} rays ({hits} hits): {agree} agree with three.js");
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems
            .iter()
            .take(60)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Content keys follow the pose (bone matrices, morph influences) and
/// stay cheap: the debug form names images, bones, morph targets, and
/// instances by content key instead of their data.
#[test]
fn content_keys_follow_the_pose() {
    let Some((_, cases)) = selected() else {
        eprintln!("no glTF reference capture; run tools/three-ref/run-gltf.ts");
        return;
    };
    let plan = |id: &str| {
        cases
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.outputs[0].0.clone())
    };
    let (Some(a), Some(b), Some(grid)) =
        (plan("skinned-0.35"), plan("skinned-0.7"), plan("pbr-ext"))
    else {
        return;
    };
    assert_eq!(a.content_key(), a.clone().content_key());
    assert_ne!(
        a.content_key(),
        b.content_key(),
        "a new pose is new content"
    );
    for p in [&a, &grid] {
        let text = format!("{:?}", p.scene);
        assert!(
            text.len() < 256 * 1024,
            "debug form of {} is {} bytes",
            p.layer,
            text.len()
        );
    }
}

/// Frame time of glTF layers at 1920 × 1080 (release; ignored): the
/// scene build (mixer, poses, bone textures) and the render of every
/// output (CPU and GPU, waiting for the device).
#[test]
#[ignore]
fn frame_time_gltf_at_1080p() {
    let Some((dir, _)) = selected() else {
        eprintln!("no glTF reference capture; run tools/three-ref/run-gltf.ts");
        return;
    };
    let gpu = pm_gpu::Gpu::new("pm-render glTF frame time").expect("a Metal device");
    let mut renderer = Scene3dRenderer::default();
    let mut pool = pm_gpu::TargetPool::new();
    for id in [
        "khr-BrainStem-0.8",
        "khr-Fox-0.7",
        "khr-ToyCar",
        "pbr-ext",
        "transmission",
    ] {
        let Some(raw) = fixture()
            .into_iter()
            .find(|c| text(obj(c).get("id")).as_deref() == Some(id))
        else {
            continue;
        };
        let mut raw = raw;
        if let Value::Object(m) = &mut raw {
            m.insert("width", Value::Number(1920.0));
            m.insert("height", Value::Number(1080.0));
        }
        let mut builds = Vec::new();
        let mut renders = Vec::new();
        for k in 0..12 {
            // A new time per frame (the mixer, bones, and morphs change).
            if let Value::Object(m) = &mut raw {
                m.insert("time", Value::Number(0.25 + 0.04 * f64::from(k)));
            }
            let t0 = std::time::Instant::now();
            let c = case(&raw, &dir).unwrap();
            builds.push(t0.elapsed().as_secs_f64() * 1e3);
            let t0 = std::time::Instant::now();
            renderer.begin_frame();
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            let mut outs = Vec::new();
            for (plan, _) in &c.outputs {
                outs.push(
                    renderer
                        .render(
                            &gpu.device,
                            &gpu.queue,
                            &mut encoder,
                            &mut pool,
                            plan,
                            false,
                        )
                        .unwrap(),
                );
            }
            gpu.queue.submit([encoder.finish()]);
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
            renders.push(t0.elapsed().as_secs_f64() * 1e3);
            for o in outs {
                pool.give(o.color);
            }
        }
        // The first frame translates programs and uploads.
        let first = renders.remove(0);
        builds.remove(0);
        renders.sort_by(|a, b| a.partial_cmp(b).unwrap());
        builds.sort_by(|a, b| a.partial_cmp(b).unwrap());
        eprintln!(
            "{id:<20} build median {:.2} ms; render median {:.2} ms p95 {:.2} ms (first {first:.0} ms)",
            builds[builds.len() / 2],
            renders[renders.len() / 2],
            renders[renders.len() * 95 / 100]
        );
    }
}
