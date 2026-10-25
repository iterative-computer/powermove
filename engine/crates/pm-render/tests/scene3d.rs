//! The native 3D scene pass against three.js r180 (section 25's 3D class;
//! amendment 11): every case of `tests/fixtures/scene3d-cases.json` (the
//! sweep of `tools/three-ref/cases.ts`: lights, shadows, transparency,
//! cameras, sources, maps, slots, depth sets, editor looks, a legacy scene)
//! is rendered by the pass and compared with three.js's own render of the
//! same scene through the production `SceneRuntime`
//! (`tools/three-ref/run-scenes.ts`, captured outside the repository).
//! Also checks that the draws are three.js's: every draw of ours has a
//! recorded draw with the same program (byte for byte, but for what
//! three.js's program cache makes history-dependent; see `normalized`) and
//! equal uniform values in f32, and every recorded draw that writes color
//! or depth is one of ours.
//!
//! `PM_SCENE3D_REF=<dir>` picks a capture (default: the newest
//! `scene3d-*`); `PM_SCENE3D_ONLY=<prefix>` limits the cases;
//! `PM_RENDER_DIFFS=<dir>` writes ours and theirs for failing outputs.

#[path = "shared/scene3d_reference.rs"]
mod float_reference;
mod support;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pm_model::json::{Map, Value};
use pm_reference::compare::{Bounds, compare, geometry, same_geometry};
use pm_render::scene3d::describe::{self, DrawDesc, Layer3d, PassDesc};
use pm_render::scene3d::model::{
    Camera3d, Evaluated, LayerFacts, Look, Placement, Projection, Scene3d, SceneBuilder,
    TextureSource,
};
use pm_render::scene3d::{MathMode, Object3dPlan, Render3d, Scene3dRenderer};
use pm_render::three::math::{self, Mat4};
use pm_render::three::program::{ToneMapping, chunk, sources};

fn fixture() -> Vec<Value> {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/scene3d-cases.json"
    ))
    .unwrap();
    let Value::Object(doc) = pm_model::json::parse(&text).unwrap() else {
        panic!("fixture")
    };
    match doc.get("cases") {
        Some(Value::Array(a)) => a.clone(),
        _ => panic!("cases"),
    }
}

fn capture() -> Option<PathBuf> {
    std::env::var_os("PM_SCENE3D_REF")
        .map(PathBuf::from)
        .or_else(|| support::newest_capture("scene3d-"))
}

fn obj(v: &Value) -> &Map {
    match v {
        Value::Object(m) => m,
        _ => panic!("object expected"),
    }
}

fn get<'a>(m: &'a Map, k: &str) -> Option<&'a Value> {
    m.get(k)
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

fn v3(v: &Value) -> [f64; 3] {
    let a = nums(v);
    [a[0], a[1], a[2]]
}

fn text(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}

struct Textures(HashMap<String, PathBuf>);

impl TextureSource for Textures {
    fn image(&self, asset: &str) -> Option<PathBuf> {
        self.0.get(asset).cloned()
    }
}

/// One case's scene per distinct output view (look, editor camera).
struct Case {
    id: String,
    outputs: Vec<(Object3dPlan, usize)>,
}

fn editor_camera(e: &Map) -> Camera3d {
    let ortho = text(get(e, "projection")).as_deref() == Some("orthographic");
    let (near, far, zoom) = (num(at(e, "near")), num(at(e, "far")), num(at(e, "zoom")));
    Camera3d {
        world: math::camera_world(v3(at(e, "position")), v3(at(e, "target")), v3(at(e, "up"))),
        projection: if ortho {
            Projection::Orthographic { zoom, near, far }
        } else {
            Projection::Perspective {
                fov: num(at(e, "fov")),
                zoom,
                near,
                far,
            }
        },
    }
}

fn case(c: &Value, tex: &Path) -> Case {
    let c = obj(c);
    let id = text(get(c, "id")).unwrap();
    let (w, h) = (num(at(c, "width")) as u32, num(at(c, "height")) as u32);
    let scene = obj(at(c, "scene"));
    let matrices: HashMap<String, Mat4> = get(c, "matrices")
        .map(|m| {
            obj(m)
                .iter()
                .map(|(k, v)| {
                    let a = nums(v);
                    (k.to_string_lossy(), a.try_into().unwrap())
                })
                .collect()
        })
        .unwrap_or_default();
    let facts: HashMap<String, LayerFacts> = get(c, "layers")
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
    let textures = Textures(
        get(c, "textures")
            .map(|m| {
                obj(m)
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.to_string_lossy(),
                            tex.join(format!("{}.png", text(Some(v)).unwrap())),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
    );
    let pose = get(c, "cameraPose").map(|p| {
        let p = obj(p);
        (v3(at(p, "position")), v3(at(p, "target")), v3(at(p, "up")))
    });
    let lights = HashMap::new();
    let placement = if matrices.is_empty() {
        Placement::Channels
    } else {
        Placement::Composition {
            objects: &matrices,
            lights: &lights,
            camera: pose,
        }
    };
    let base = SceneBuilder::default()
        .build(
            scene,
            w,
            h,
            &placement,
            &facts,
            &mut Evaluated,
            &textures,
            &pm_render::scene3d::model::NoModels,
        )
        .unwrap_or_else(|e| panic!("{id}: {e}"));
    let mut scenes: Vec<(String, Arc<Scene3d>)> = Vec::new();
    let mut outputs = Vec::new();
    let Value::Array(list) = &at(c, "outputs") else {
        panic!("outputs")
    };
    for (k, o) in list.iter().enumerate() {
        let o = obj(o);
        let look = get(o, "look").map(obj);
        let flag = |n: &str| look.is_some_and(|l| matches!(l.get(n), Some(Value::Bool(true))));
        let look = Look {
            wireframe: flag("wireframe"),
            xray: flag("xray"),
        };
        let camera = get(o, "editorCamera").map(|e| editor_camera(obj(e)));
        let key = format!("{look:?}{camera:?}");
        let scene = match scenes.iter().find(|(k, _)| *k == key) {
            Some((_, s)) => s.clone(),
            None => {
                let mut s = base.clone();
                if let Some(c) = camera {
                    s.camera = c;
                }
                s.apply_look(look);
                let s = Arc::new(s);
                scenes.push((key, s.clone()));
                s
            }
        };
        let index = |name: &str| scene.objects.iter().position(|o| o.id == name).unwrap();
        let kind = match text(get(o, "object")) {
            Some(object) => {
                let mut set: Vec<usize> = match get(o, "depthIds") {
                    Some(Value::Array(ids)) => {
                        ids.iter().map(|i| index(&text(Some(i)).unwrap())).collect()
                    }
                    _ => Vec::new(),
                };
                set.sort_unstable();
                Render3d::Layer {
                    object: index(&object),
                    depth_set: set,
                    background: matches!(get(o, "background"), Some(Value::Bool(true))),
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
    Case { id, outputs }
}

fn selected() -> Option<(PathBuf, Vec<Case>)> {
    let dir = capture()?;
    let only = std::env::var("PM_SCENE3D_ONLY").ok();
    let cases = fixture()
        .iter()
        .filter(|c| {
            only.as_deref()
                .is_none_or(|o| text(obj(c).get("id")).unwrap().starts_with(o))
        })
        .map(|c| case(c, &dir.join("tex")))
        .collect();
    Some((dir, cases))
}

#[test]
fn sweep_resolves_finite_linear_color() {
    let Some((dir, cases)) = selected() else {
        eprintln!("no scene3d scene assets; run tools/three-ref/run-scenes.ts");
        return;
    };
    let gpu = pm_gpu::Gpu::new("pm-render 3d linear sweep").expect("a Metal device");
    let mut renderer = Scene3dRenderer::default();
    renderer.set_math_mode(MathMode::Safe);
    let mut pool = pm_gpu::TargetPool::new();
    let mut compared = 0;
    for c in &cases {
        for (plan, _) in &c.outputs {
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
            assert!(
                color.chunks_exact(4).any(|p| p[3] > 0.0),
                "{}: no object",
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
    eprintln!("{compared} linear 3D outputs; assets in {}", dir.display());
    assert!(compared > 50);
}

/// G-D1: native and today's displayed outputs against an independent r180
/// RGBA32F scene at 8x, resolved in linear light and displayed through f64
/// ACES and sRGB. Geometry is also checked from the untoned alpha planes;
/// document picks and the real compositor seam have their own direct tests.
#[test]
fn displayed_pixels_prefer_supersampled_float_reference() {
    let Some((capture, cases)) = selected() else {
        eprintln!("no scene3d scene assets; run tools/three-ref/run-scenes.ts");
        return;
    };
    let path = std::env::var_os("PM_SCENE3D_FLOAT_REF")
        .map(PathBuf::from)
        .expect("G-D1 requires PM_SCENE3D_FLOAT_REF (examples/scene3d_float_reference.ts)");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(path.join("manifest.json")).expect("float reference manifest"),
    )
    .unwrap();
    assert_eq!(manifest["three"], "0.180.0");
    assert_eq!(manifest["scale"], 8);
    assert_eq!(manifest["format"], "rgba32float");
    assert_eq!(
        manifest["captureId"].as_str(),
        capture.file_name().and_then(|n| n.to_str())
    );
    let gpu = pm_gpu::Gpu::new("pm-render 3d independent float reference").expect("a Metal device");
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
            let actual = float_reference::top_down_linear(
                pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &out.color.texture).unwrap(),
                w as usize,
                h as usize,
            );
            assert_eq!(actual.len(), reference.len());
            assert!(
                reference.iter().all(|v| v.is_finite()),
                "{} reference nonfinite",
                plan.layer
            );
            let actual_alpha: Vec<_> = actual.chunks_exact(4).map(|p| f64::from(p[3])).collect();
            let reference_alpha: Vec<_> =
                reference.chunks_exact(4).map(|p| f64::from(p[3])).collect();
            if let Err(error) = same_geometry(
                &geometry(&actual_alpha, w as usize),
                &geometry(&reference_alpha, w as usize),
                1e-3,
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
            let classes = float_reference::classes(&modeled, w as usize, h as usize);
            let report = compare(&engine, &today, &modeled, &classes);
            eprintln!("float display {}:{} {report}", c.id, k);
            for failure in report.failures(&Bounds::HALF_FLOAT) {
                failures.push(format!("{}:{} {failure}", c.id, k));
            }
            compared += 1;
            pool.give(out.color);
        }
    }
    let selected_outputs: usize = cases.iter().map(|case| case.outputs.len()).sum();
    assert!(
        selected_outputs > 0,
        "no independent scene references selected"
    );
    assert_eq!(compared, selected_outputs);
    if std::env::var("PM_SCENE3D_ONLY").is_err() {
        assert!(
            compared >= 70,
            "only {compared} independent references compared"
        );
    }
    assert!(
        failures.is_empty(),
        "G-D1 independent displayed reference deviations:\n{}",
        failures.join("\n")
    );
}

/// The draws our pass issues for each output, as three.js issues them: the
/// prepass only when the previous output's set differs.
fn our_draws(c: &Case) -> Vec<(usize, Vec<DrawDesc>)> {
    let mut out = Vec::new();
    let mut previous: Option<(*const Scene3d, Vec<usize>)> = None;
    for (plan, k) in &c.outputs {
        let mut draws = Vec::new();
        let mut push = |p: PassDesc| match p {
            PassDesc::Shadow { views, .. } => {
                for (_, d) in views {
                    draws.extend(d);
                }
            }
            PassDesc::Transmission { opaque, back, .. } => {
                draws.extend(opaque);
                draws.extend(back);
            }
            PassDesc::Prepass(d) | PassDesc::Color { draws: d, .. } => draws.extend(d),
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
                push(describe::color(&layer).unwrap());
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

/// Program text with what three.js's program cache makes history-dependent
/// and pixel-irrelevant normalized: `SHADER_NAME` (programs are shared by
/// materials whose parameters agree; the name is the first one's), and for
/// the shadow pass's shared depth and distance materials, the side defines
/// and light counts (their programs are not rebuilt when `side` or the light
/// state changes; neither shader reads them).
fn normalized(src: &str, shadow: bool) -> String {
    src.lines()
        .filter(|l| !l.starts_with("#define SHADER_NAME"))
        .filter(|l| !(shadow && (*l == "#define DOUBLE_SIDED" || *l == "#define FLIP_SIDED")))
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

/// Put back only r180's former output transfer and packed shadow storage
/// when comparing linked program text. The live program keeps linear output
/// and hardware depth; all geometry, lighting, material, map and clipping
/// parameters still generate source from the same `d.params` and must equal
/// the independently recorded Three.js program byte for byte.
fn reference_program_sources(d: &DrawDesc) -> (String, String) {
    let mut p = d.params.clone();
    let shadow = matches!(
        p.shader_type.as_str(),
        "MeshDepthMaterial" | "MeshDistanceMaterial"
    );
    match p.shader_type.as_str() {
        "MeshDepthMaterial" => {
            p.fragment_shader = chunk("depth_frag").expect("r180 depth fragment").into();
            p.use_depth_packing = true;
            p.depth_packing = 3201;
        }
        "MeshDistanceMaterial" => {
            p.fragment_shader = chunk("distanceRGBA_frag")
                .expect("r180 distance fragment")
                .into();
        }
        _ => {
            p.tone_mapping = ToneMapping::AcesFilmic;
            p.output_srgb = true;
        }
    }
    let (vertex, fragment) = sources(&p).expect("r180 reference program");
    let fragment = if shadow {
        // r180's linear shadow target included a transfer function with an
        // identity Rec.709 matrix. The live depth texture needs no color
        // transfer. Reconstitute this output-only source before the exact
        // program comparison; no geometry or material code is normalized.
        float_reference::restore_linear_output(fragment)
    } else {
        fragment
    };
    (vertex, fragment)
}

fn f32s(v: &serde_json::Value) -> Vec<f32> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap() as f32)
        .collect()
}

/// Every draw the pass issues uses a program byte-identical to one three.js
/// linked (after the normalizations in `normalized`), with every uniform
/// three.js set equal as f32, in three.js's order; every recorded draw that
/// writes color or depth is matched. Samplers are compared by the pixel
/// sweep, as is the shared shadow materials' stale `mapTransform`.
#[test]
fn programs_and_uniforms_equal_three_js() {
    let Some((dir, cases)) = selected() else {
        eprintln!("no scene3d reference capture; run tools/three-ref/run-scenes.ts");
        return;
    };
    let mut problems = Vec::new();
    let (mut draws_matched, mut values) = (0, 0);
    let mut unmatched_recorded = 0;
    // Values equal as f32, and matched draws issued out of three.js's
    // order (both must be complete).
    let mut exact = 0usize;
    let mut out_of_order = Vec::new();
    for c in &cases {
        for (k, ours) in our_draws(c) {
            let record: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(dir.join(&c.id).join(format!("{k}.json"))).unwrap(),
            )
            .unwrap();
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
            let mut last_matched: Option<usize> = None;
            // A draw with neither color nor depth writes changes nothing (the
            // prepass's transparent and fading models); every other draw
            // three.js issued must be one of ours.
            let effective = |r: &serde_json::Value| {
                r["depthMask"].as_bool() == Some(true)
                    || r["colorMask"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|b| b.as_bool() == Some(true))
            };
            for (n, d) in ours.iter().enumerate() {
                let (vs, fs) = reference_program_sources(d);
                let shadow = matches!(
                    d.params.shader_type.as_str(),
                    "MeshDepthMaterial" | "MeshDistanceMaterial"
                );
                let (vs, fs) = (normalized(&vs, shadow), normalized(&fs, shadow));
                let ours_f32 = |name: &str| {
                    d.uniforms
                        .iter()
                        .find(|(k, _)| k == name)
                        .map(|(_, v)| v.iter().map(|x| *x as f32).collect::<Vec<f32>>())
                };
                // Draws are identified by program, phase, and the matrices
                // the program uses (inactive uniforms are not recorded).
                let keys = [
                    "modelMatrix",
                    "modelViewMatrix",
                    "projectionMatrix",
                    "viewMatrix",
                ];
                let same = |r: &serde_json::Value| {
                    keys.iter()
                        .all(|k| match (r["uniforms"].get(*k), ours_f32(k)) {
                            (Some(x), Some(m)) => f32s(x) == m,
                            _ => true,
                        })
                };
                let candidates = |r: &serde_json::Value| {
                    let color = r["colorMask"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|b| b.as_bool() == Some(true));
                    let (rv, rf) = &programs[&r["program"].as_u64().unwrap()];
                    // r180 packed shadow depth into RGBA and kept its color
                    // mask on; hardware depth replaces only that storage.
                    (if shadow {
                        r["depthMask"].as_bool() == Some(true)
                    } else {
                        color == d.state.color_write
                    }) && normalized(rv, shadow) == vs
                        && normalized(rf, shadow) == fs
                };
                let found = recorded
                    .iter()
                    .enumerate()
                    .position(|(i, r)| !used[i] && candidates(r) && same(r));
                let Some(i) = found else {
                    let same_program = recorded.iter().find(|r| candidates(r));
                    let programs_equal = programs
                        .values()
                        .any(|(v, f)| normalized(v, shadow) == vs && normalized(f, shadow) == fs);
                    let detail = match same_program {
                        Some(r) => format!(
                            "no recorded draw with these matrices (differing: {:?})",
                            keys.iter()
                                .filter(|k| match (r["uniforms"].get(**k), ours_f32(k)) {
                                    (Some(x), Some(m)) => f32s(x) != m,
                                    _ => false,
                                })
                                .collect::<Vec<_>>()
                        ),
                        None if programs_equal => {
                            "a recorded program equals ours but not for this phase".to_owned()
                        }
                        None => {
                            if std::env::var_os("PM_SCENE3D_PROGRAMS").is_some() {
                                let dir = std::env::var_os("PM_SCENE3D_PROGRAMS")
                                    .map(std::path::PathBuf::from)
                                    .expect("program dump directory");
                                std::fs::create_dir_all(&dir).unwrap();
                                std::fs::write(dir.join(format!("{}-{k}-{n}.vs", c.id)), &vs)
                                    .unwrap();
                                std::fs::write(dir.join(format!("{}-{k}-{n}.fs", c.id)), &fs)
                                    .unwrap();
                                for (id, (rv, rf)) in &programs {
                                    std::fs::write(
                                        dir.join(format!("{}-{k}-rec{id}.vs", c.id)),
                                        rv,
                                    )
                                    .unwrap();
                                    std::fs::write(
                                        dir.join(format!("{}-{k}-rec{id}.fs", c.id)),
                                        rf,
                                    )
                                    .unwrap();
                                }
                            }
                            "no recorded program equals ours".to_owned()
                        }
                    };
                    problems.push(format!(
                        "{} output {k} draw {n} ({} {:?}, color {}): {detail}",
                        c.id, d.params.shader_type, d.params.shader_name, d.state.color_write,
                    ));
                    continue;
                };
                used[i] = true;
                draws_matched += 1;
                if last_matched.is_some_and(|l| i < l) {
                    out_of_order.push(format!("{} output {k} draw {n}", c.id));
                    problems.push(format!(
                        "{} output {k} draw {n}: issued out of three.js's order",
                        c.id
                    ));
                }
                last_matched = Some(i);
                let textures: Vec<&str> = d.textures.iter().map(|(n, _)| n.as_str()).collect();
                for (name, value) in recorded[i]["uniforms"].as_object().unwrap() {
                    let base = name.split('[').next().unwrap();
                    let sampler = textures.iter().any(|t| t.split('[').next() == Some(base))
                        || [
                            "map",
                            "normalMap",
                            "roughnessMap",
                            "metalnessMap",
                            "emissiveMap",
                            "aoMap",
                            "directionalShadowMap",
                            "spotShadowMap",
                            "pointShadowMap",
                            "ltc_1",
                            "ltc_2",
                        ]
                        .contains(&base);
                    // The shared shadow materials keep the uniforms of the
                    // last refresh (three.js refreshes a material's uniforms
                    // only when it changes program or version), and their
                    // fragment samples `map` without using it.
                    if sampler || (shadow && name == "mapTransform") {
                        continue;
                    }
                    let theirs = f32s(value);
                    // An array uploaded at once (`name[0]` with every element)
                    // equals our elements in order.
                    let mine = match name.strip_suffix("[0]") {
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
                                d.uniforms
                                    .iter()
                                    .find(|(k, _)| k == stem || k == name)
                                    .map(|(_, v)| v.iter().map(|x| *x as f32).collect())
                            } else {
                                Some(all)
                            }
                        }
                        _ => d
                            .uniforms
                            .iter()
                            .find(|(k, _)| k == name || *k == format!("{name}[0]"))
                            .map(|(_, v)| v.iter().map(|x| *x as f32).collect::<Vec<f32>>()),
                    };
                    let Some(mine) = mine else {
                        problems.push(format!(
                            "{} output {k} draw {n}: three.js sets {name}, the pass does not",
                            c.id
                        ));
                        continue;
                    };
                    let len = theirs.len().min(mine.len());
                    if mine.len() < theirs.len() {
                        problems.push(format!(
                            "{} output {k} draw {n}: {name} has {} values, three.js {}",
                            c.id,
                            mine.len(),
                            theirs.len()
                        ));
                    }
                    for (a, b) in mine[..len].iter().zip(&theirs[..len]) {
                        values += 1;
                        // Equal as f32 (a NaN equals the same NaN).
                        if a.to_bits() == b.to_bits() || a == b {
                            exact += 1;
                        } else {
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
                if !used[i] && effective(r) {
                    unmatched_recorded += 1;
                    problems.push(format!(
                        "{} output {k}: recorded draw {i} (program {}) has no counterpart",
                        c.id, r["program"]
                    ));
                }
            }
        }
    }
    eprintln!(
        "{draws_matched} draws matched, {values} uniform values compared ({exact} equal as f32), {unmatched_recorded} effective recorded draws unmatched, {} matched draws out of three.js's order",
        out_of_order.len()
    );
    if !problems.is_empty() {
        let mut p = problems.clone();
        p.dedup();
        panic!(
            "{} problems:\n{}",
            p.len(),
            p.iter().take(80).cloned().collect::<Vec<_>>().join("\n")
        );
    }
}

#[test]
fn programs_come_from_the_disk_cache_and_keys_follow_content() {
    // A scene with shadows of every kind and maps (no capture needed).
    let Some(c) = fixture()
        .iter()
        .find(|c| text(obj(c).get("id")) == Some("five-casting-lights".into()))
        .map(|c| case(c, Path::new("/nonexistent")))
    else {
        panic!("case")
    };
    let gpu = pm_gpu::Gpu::new("pm-render 3d cache test").expect("a Metal device");
    let dir = std::env::temp_dir().join(format!("pm-render-programs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let plan = &c.outputs[0].0;
    let render = |r: &mut Scene3dRenderer| {
        let mut pool = pm_gpu::TargetPool::new();
        let t0 = std::time::Instant::now();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let out = r
            .render(
                &gpu.device,
                &gpu.queue,
                &mut encoder,
                &mut pool,
                plan,
                false,
            )
            .unwrap();
        gpu.queue.submit([encoder.finish()]);
        let bytes =
            pm_gpu::readback::read_raw(&gpu.device, &gpu.queue, &out.color.texture).unwrap();
        (bytes, t0.elapsed())
    };
    let mut cold = Scene3dRenderer::default();
    cold.set_program_cache(Some(dir.clone()));
    let (a, translate) = render(&mut cold);
    assert_eq!(cold.program_cache_stats(), (0, 0));
    let stored = std::fs::read_dir(&dir).unwrap().count();
    assert!(
        stored >= 3,
        "standard, depth, and distance programs stored ({stored})"
    );
    let mut warm = Scene3dRenderer::default();
    warm.set_program_cache(Some(dir.clone()));
    let (b, cached) = render(&mut warm);
    assert_eq!(warm.program_cache_stats(), (stored as u64, 0));
    assert!(a == b, "cached programs render the same pixels");
    eprintln!("first frame: translated {translate:?}, from the disk cache {cached:?}");
    for e in std::fs::read_dir(&dir).unwrap() {
        std::fs::write(e.unwrap().path(), "{").unwrap();
    }
    let mut again = Scene3dRenderer::default();
    again.set_program_cache(Some(dir.clone()));
    let (d, _) = render(&mut again);
    assert_eq!(
        again.program_cache_stats(),
        (0, 0),
        "damaged entries are misses"
    );
    assert!(a == d);
    std::fs::remove_dir_all(&dir).unwrap();

    // Content keys: equal for equal renders, different when any input moves.
    let key = plan.content_key();
    assert_eq!(key, plan.clone().content_key());
    let mut moved = (*plan.scene).clone();
    moved.lights[0].intensity *= 1.01;
    let other = Object3dPlan {
        scene: Arc::new(moved),
        ..plan.clone()
    };
    assert_ne!(key, other.content_key());
    let mut bg = plan.clone();
    if let Render3d::Layer { background, .. } = &mut bg.kind {
        *background = !*background;
    }
    assert_ne!(key, bg.content_key());
}
