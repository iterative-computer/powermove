//! The 3D scene pass through documents (plan section 8): composition
//! ordering of 3D runs, depth sets split by 2D layers, the depth and id
//! surface of every 3D layer (top-level and inside precompositions), the
//! view override (editor looks and camera), and 3D output with linear
//! blending on.

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_protocol::frame::{EditorCamera, Projection, Shading3d, ViewOverride};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Value, json};
use support::doc::*;
use support::ticks;

const W: u32 = 320;
const H: u32 = 180;

fn boxes(extra: Vec<Value>, a_z: f64, b_z: f64) -> Value {
    // `a` is above `b` in the stack; positions in world units.
    let mut a = object3d("a", "box", json!({}), "#FF4040", W, H, 2.0);
    set(&mut a, "position.y", json!(0.6));
    set(&mut a, "position.z", json!(a_z));
    let mut b = object3d(
        "b",
        "box",
        json!({ "width": 1.6, "height": 0.6, "depth": 1.0 }),
        "#4040FF",
        W,
        H,
        2.0,
    );
    set(&mut b, "position.z", json!(b_z));
    let mut layers = vec![
        camera("cam", [0.0, 0.0, 6.0], false, 2.0),
        sun("sun", [-3.0, 5.0, 4.0], false, 2.0),
        a,
    ];
    layers.extend(extra);
    layers.push(b);
    project(W, H, 2.0, solid_fill("#000000"), layers)
}

/// `(depth, legend index, covered)` at a pixel of the depth and id surface.
fn surface(r: &mut Renderer, doc: &Value) -> (Vec<f32>, Vec<String>, Vec<f32>) {
    let doc = load(doc);
    let mut request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float16);
    request.depth_id = true;
    let frame = r
        .render(
            &doc,
            "Ptest-root",
            ticks(0.0),
            &request,
            &MapResolver::default(),
        )
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let ids = r.read_depth_id(&frame).unwrap().unwrap();
    let color = r.read_rgba(&frame).unwrap();
    (ids, frame.legend, color)
}

fn at(s: &[f32], x: u32, y: u32) -> (f32, f32, f32) {
    let i = ((y * W + x) * 4) as usize;
    (s[i], s[i + 1], s[i + 3])
}

#[test]
fn runs_sort_by_camera_depth_and_the_surface_is_upright() {
    let mut r = Renderer::headless().unwrap();
    // `b` (lower in the stack) is nearer the camera: within a 3D run it
    // sorts on top, so it hides `a` where they overlap.
    let (s, legend, _) = surface(&mut r, &boxes(vec![], -2.0, 1.5));
    let index = |id: &str| legend.iter().position(|l| l == id).unwrap() as f32;
    let (d, id, a) = at(&s, W / 2, H / 2);
    assert_eq!((id, a), (index("b"), 1.0));
    // Depth of b's front face: 6 − 1.5 − 0.5.
    assert!((d - 4.0).abs() < 0.01, "{d}");
    // Where both cover (row 84), b is on top.
    assert_eq!(at(&s, W / 2, 84).1, index("b"));
    // Above b, only a (raised by 0.6, farther away): a top-down surface
    // finds it in the upper half of the frame, never the lower.
    let rows = |range: std::ops::Range<u32>| -> Vec<u32> {
        range
            .filter(|&y| at(&s, W / 2, y).1 == index("a") && at(&s, W / 2, y).2 == 1.0)
            .collect()
    };
    let (upper, lower) = (rows(0..H / 2), rows(H / 2..H));
    assert!(!upper.is_empty() && lower.is_empty(), "{upper:?} {lower:?}");
    // Inside the face (edge pixels covered by MSAA have no single-sample depth).
    let (d, _, _) = at(&s, W / 2, upper[upper.len() / 2]);
    assert!(
        (d - 7.5).abs() < 0.05,
        "a's front face at 6 + 2 − 0.5, got {d}"
    );
}

#[test]
fn a_2d_layer_splits_depth_sets() {
    let mut r = Renderer::headless().unwrap();
    // A small 2D solid between them (off to the side): `a` and `b` are in
    // different sets and different runs, so the stack order holds and b's
    // depth does not occlude a.
    let barrier = layer(
        "wall",
        "solid",
        json!({ "color": "#00FF00", "w": 10, "h": 10 }),
        5.0,
        5.0,
        2.0,
    );
    let (s, legend, _) = surface(&mut r, &boxes(vec![barrier], -2.0, 1.5));
    let index = |id: &str| legend.iter().position(|l| l == id).unwrap() as f32;
    let (d, id, a) = at(&s, W / 2, 84);
    assert_eq!((id, a), (index("a"), 1.0), "a draws over b");
    assert!((d - 7.5).abs() < 0.05, "{d}");
    assert_eq!(at(&s, 5, 5).1, index("wall"));
    assert!(at(&s, 5, 5).0.is_infinite());
}

#[test]
fn three_d_inside_a_precomp_reports_its_depth() {
    let mut r = Renderer::headless().unwrap();
    let mut doc = project(W, H, 2.0, solid_fill("#000000"), vec![]);
    let inner = boxes(vec![], -2.0, 1.5);
    composition(
        &mut doc,
        "inner",
        W,
        H,
        2.0,
        inner["layers"].as_array().unwrap().clone(),
    );
    let mut pc = layer(
        "pc",
        "precomp",
        json!({ "comp": "inner", "w": W, "h": H }),
        0.0,
        0.0,
        2.0,
    );
    pc["d"]["comp"] = json!("inner");
    doc["layers"] = json!([pc]);
    let (s, legend, _) = surface(&mut r, &doc);
    let (d, id, a) = at(&s, W / 2, H / 2);
    assert_eq!((legend[id as usize].as_str(), a), ("pc", 1.0));
    assert!(
        (d - 4.0).abs() < 0.01,
        "the nested surface's depth, got {d}"
    );
    // Off center: `a` (raised, farther away) shows above b in the upper
    // half only, at its own depth, so the nested surface is upright too.
    let a_rows: Vec<u32> = (0..H)
        .filter(|&y| {
            let (d, _, a) = at(&s, W / 2, y);
            a == 1.0 && (d - 7.5).abs() < 0.05
        })
        .collect();
    assert!(
        !a_rows.is_empty() && a_rows.iter().all(|&y| y < H / 2),
        "{a_rows:?}"
    );
    // Outside the models: the precomp covers (its background) without 3D.
    assert!(at(&s, 2, 2).0.is_infinite());
}

fn render(r: &mut Renderer, doc: &Value) -> Vec<f32> {
    let doc = load(doc);
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float16);
    let frame = r
        .render(
            &doc,
            "Ptest-root",
            ticks(0.0),
            &request,
            &MapResolver::default(),
        )
        .unwrap();
    r.read_rgba(&frame).unwrap()
}

#[test]
fn view_overrides_change_only_viewer_renders() {
    let doc = boxes(vec![], -2.0, 1.5);
    let mut r = Renderer::headless().unwrap();
    let plain = render(&mut r, &doc);
    let view = |shading: Shading3d, xray: bool, camera: Option<EditorCamera>| ViewOverride {
        camera,
        shading,
        xray,
        local_view: Vec::new(),
    };
    r.set_view(Some(&view(Shading3d::Wireframe, false, None)));
    let wire = render(&mut r, &doc);
    assert_ne!(wire, plain, "wireframe draws edges");
    // An export request never draws the view override.
    let mut export = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float16);
    export.export = true;
    let frame = r
        .render(
            &load(&doc),
            "Ptest-root",
            ticks(0.0),
            &export,
            &MapResolver::default(),
        )
        .unwrap();
    assert_eq!(
        r.read_rgba(&frame).unwrap(),
        plain,
        "exports ignore the view"
    );
    assert_eq!(render(&mut r, &doc), wire, "the viewer keeps it");
    r.set_view(Some(&view(Shading3d::Solid, true, None)));
    let xray = render(&mut r, &doc);
    assert_ne!(xray, plain, "X-ray makes models see-through");
    // X-ray applies in the draft shadings only.
    r.set_view(Some(&view(Shading3d::Material, true, None)));
    assert_eq!(render(&mut r, &doc), plain);
    // An editor camera looking from the side.
    let mut m = [0.0; 16];
    let world = pm_render::three::math::camera_world([6.0, 0.0, 0.0], [0.0; 3], [0.0, 1.0, 0.0]);
    m.copy_from_slice(&pm_render::three::math::invert(&world));
    r.set_view(Some(&view(
        Shading3d::Solid,
        false,
        Some(EditorCamera {
            view: m,
            projection: Projection::Perspective {
                fov_y_degrees: 45.0,
                near: 0.01,
                far: 1000.0,
            },
        }),
    )));
    assert_ne!(render(&mut r, &doc), plain);
    // Local View: only `a` is drawn.
    r.set_view(Some(&ViewOverride {
        camera: None,
        shading: Shading3d::Solid,
        xray: false,
        local_view: vec!["a".into()],
    }));
    let local = render(&mut r, &doc);
    let px = |c: &[f32], x: u32, y: u32| {
        let i = ((y * W + x) * 4) as usize;
        [c[i], c[i + 1], c[i + 2]]
    };
    assert!(px(&plain, W / 2, H / 2)[2] > 0.1, "b is blue in the center");
    assert!(
        px(&local, W / 2, H / 2)[2] < px(&plain, W / 2, H / 2)[2],
        "b is hidden in Local View"
    );
    r.set_view(None);
    assert_eq!(render(&mut r, &doc), plain, "no override renders as before");
}

#[test]
fn editor_camera_projection_follows_the_contract() {
    let v = pm_render::scene3d::View3d::from_override(&ViewOverride {
        camera: Some(EditorCamera {
            view: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -10.0, 1.0,
            ],
            projection: Projection::Orthographic {
                half_height: 1.5,
                near: 0.01,
                far: 400.0,
            },
        }),
        shading: Shading3d::Wireframe,
        xray: true,
        local_view: vec!["x".into()],
    });
    let c = v.camera.unwrap();
    // The world matrix is the inverse view: the camera sits at z = 10.
    assert!((c.world[14] - 10.0).abs() < 1e-12);
    // `resizeCamera` fixes the half-height at 3: zoom = 3 / half-height.
    assert_eq!(
        c.projection,
        pm_render::scene3d::Projection::Orthographic {
            zoom: 2.0,
            near: 0.01,
            far: 400.0
        }
    );
    assert!(v.look.wireframe && v.look.xray);
    assert_eq!(v.local, Some(vec!["x".to_owned()]));
}

#[test]
fn linear_blending_decodes_the_3d_canvas_once() {
    // The 3D pass's E-encoded output is decoded once when drawn with linear
    // blending on, and the frame is encoded with E on output. A single 3D
    // layer is the bottom of its set and draws the solid 3D background, so
    // its output is opaque everywhere and comes out as with linear blending
    // off (a second decode would darken every lit pixel).
    let frame = |r: &mut Renderer, on: bool| {
        let mut cam = camera("cam", [0.0, 0.0, 6.0], false, 2.0);
        cam["d"]["data"]["environment"]["background"] = json!("#203040");
        let mut doc = project(
            W,
            H,
            2.0,
            solid_fill("#000000"),
            vec![
                cam,
                sun("sun", [-3.0, 5.0, 4.0], false, 2.0),
                object3d("a", "sphere", json!({}), "#FF4040", W, H, 2.0),
            ],
        );
        doc["color"] = json!({ "mode": "standard", "linearBlending": on });
        let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32);
        let frame = r
            .render(
                &load(&doc),
                "Ptest-root",
                ticks(0.0),
                &request,
                &MapResolver::default(),
            )
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        r.read_rgba(&frame).unwrap()
    };
    let mut r = Renderer::headless().unwrap();
    let off = frame(&mut r, false);
    let on = frame(&mut r, true);
    let (mut lit, mut worst) = (0usize, 0f32);
    for (a, b) in off.chunks_exact(4).zip(on.chunks_exact(4)) {
        assert_eq!((a[3], b[3]), (1.0, 1.0));
        if a[0].max(a[1]).max(a[2]) > 0.1 {
            lit += 1;
        }
        for c in 0..3 {
            worst = worst.max((a[c] - b[c]).abs());
        }
    }
    assert!(lit > 1000, "{lit} lit pixels");
    // f32 decode and encode of 8-bit E values.
    assert!(worst < 1e-4, "worst {worst}");
}

const CUBE_OBJ: &str = "v -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\nv -1 -1 1\nv 1 -1 1\nv 1 1 1\nv -1 1 1\n\
f 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 4 1 5 8\n";

#[test]
fn obj_models_and_mesh_layers_render() {
    let dir = std::env::temp_dir().join(format!("pm-render-obj-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cube.obj");
    std::fs::write(&path, CUBE_OBJ).unwrap();
    let mut resolver = MapResolver::default();
    resolver.paths.insert("cube".into(), path.clone());
    let record = json!({ "id": "cube", "kind": "model", "format": "obj", "name": "cube.obj" });
    // A 3D object whose source is an OBJ model asset: its parsed triangles
    // with the object's material.
    let mut obj = object3d("o", "box", json!({}), "#FF4040", W, H, 2.0);
    obj["d"]["data"]["object"]["source"] = json!({ "assetId": "cube" });
    let mut doc = project(
        W,
        H,
        2.0,
        solid_fill("#000000"),
        vec![
            camera("cam", [0.0, 0.0, 6.0], false, 2.0),
            sun("sun", [-3.0, 5.0, 4.0], false, 2.0),
            obj,
        ],
    );
    doc["assets"]["cube"] = record.clone();
    let mut r = Renderer::headless().unwrap();
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float16);
    let frame = r
        .render(&load(&doc), "Ptest-root", ticks(0.0), &request, &resolver)
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let color = r.read_rgba(&frame).unwrap();
    let center = ((H / 2 * W + W / 2) * 4) as usize;
    assert!(
        color[center] > 0.2 && color[center + 3] == 1.0,
        "the red cube is drawn"
    );
    // Without the asset record the object is the orange wireframe box, whose
    // center is empty.
    doc["assets"] = json!({});
    let frame = r
        .render(&load(&doc), "Ptest-root", ticks(0.0), &request, &resolver)
        .unwrap();
    let missing = r.read_rgba(&frame).unwrap();
    let lit = |c: &[f32]| c.chunks_exact(4).filter(|p| p[0] > 0.2).count();
    assert!(lit(&missing) > 0, "the box's edges are drawn");
    assert!(
        lit(&missing) * 2 < lit(&color),
        "missing models draw only edges: {} vs {}",
        lit(&missing),
        lit(&color)
    );

    // A legacy mesh layer: routed without a definition snapshot, drawn with
    // its fixed shading given one.
    let mut layer = layer(
        "m",
        "extension",
        json!({
            "definition": "acme.obj-viewer", "version": 1, "w": 160, "h": 90, "params": {},
            "data": { "model": "cube" }
        }),
        f64::from(W) / 2.0,
        f64::from(H) / 2.0,
        2.0,
    );
    layer["p"]["anchor.x"] = ch(json!(80));
    layer["p"]["anchor.y"] = ch(json!(45));
    let mut doc = project(W, H, 2.0, solid_fill("#000000"), vec![layer]);
    doc["assets"]["cube"] = record;
    let doc = load(&doc);
    let mut r = Renderer::headless().unwrap();
    assert!(
        r.render(&doc, "Ptest-root", ticks(0.0), &request, &resolver)
            .is_err()
    );
    let mut defs = std::collections::HashMap::new();
    defs.insert(
        "acme.obj-viewer".to_owned(),
        pm_render::scene3d::legacy::MeshDefinition {
            asset_field: "model".into(),
            defaults: Default::default(),
        },
    );
    r.set_mesh_definitions(defs);
    let frame = r
        .render(&doc, "Ptest-root", ticks(0.0), &request, &resolver)
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let c = r.read_rgba(&frame).unwrap();
    // Inside the layer: the background #0C0D12 or the shaded mesh; outside:
    // the composition background.
    let px = |x: u32, y: u32| {
        let i = ((y * W + x) * 4) as usize;
        [c[i], c[i + 1], c[i + 2]]
    };
    let corner = px(W / 2 - 78, H / 2 - 43);
    assert!((corner[2] - 0x12 as f32 / 255.0).abs() < 2e-3, "{corner:?}");
    assert!(px(W / 2, H / 2)[0] > 0.2, "the mesh shading at the center");
    assert_eq!(px(2, 2), [0.0, 0.0, 0.0]);
    // The built-in OBJ model layer needs no snapshot, and the static check
    // agrees.
    let mut builtin = support::doc::layer(
        "m",
        "extension",
        json!({
            "definition": "powermove.3d.obj-model", "version": 1, "w": 160, "h": 90, "params": {},
            "data": { "assetId": "cube" }
        }),
        0.0,
        0.0,
        2.0,
    );
    builtin["p"]["anchor.x"] = ch(json!(0));
    let mut doc = project(W, H, 2.0, solid_fill("#000000"), vec![builtin]);
    doc["assets"]["cube"] =
        json!({ "id": "cube", "kind": "model", "format": "obj", "name": "cube.obj" });
    let doc = load(&doc);
    assert!(pm_render::unsupported(&doc, "Ptest-root").is_empty());
    let mut r = Renderer::headless().unwrap();
    let frame = r
        .render(&doc, "Ptest-root", ticks(0.0), &request, &resolver)
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The 3D pass's frame time at 1920 × 1080 with shadows of every kind:
/// six models and a floor in one depth set, a casting sun, spot, and point
/// light (the point light's 4096 × 2048 cube atlas), 4× MSAA, the shared
/// prepass rendered once per frame and every layer's color pass. Release
/// build: `cargo test --release -p pm-render --test scene3d_document -- --ignored --nocapture`.
#[test]
#[ignore]
fn frame_time_3d_shadows_at_1080p() {
    let mut layers = vec![camera("cam", [0.0, 2.0, 7.0], true, 4.0)];
    let mut light = |id: &str, ty: &str, pos: [f64; 3]| {
        let mut l = sun(id, pos, true, 4.0);
        l["d"]["data"]["light"]["type"] = json!(ty);
        l["d"]["data"]["light"]["p"]["intensity"] = ch(json!(if ty == "sun" { 1.5 } else { 20.0 }));
        layers.push(l);
    };
    light("sun", "sun", [-3.0, 5.0, 4.0]);
    light("spot", "spot", [2.0, 4.0, 2.0]);
    light("bulb", "point", [0.0, 2.0, 1.0]);
    for (i, p) in ["box", "sphere", "cylinder", "cone", "torus", "capsule"]
        .iter()
        .enumerate()
    {
        let mut o = object3d(&format!("o{i}"), p, json!({}), "#C7C4FF", 1920, 1080, 4.0);
        set(&mut o, "position.x", json!(i as f64 * 1.2 - 3.0));
        set(
            &mut o,
            "rotation.y",
            keys(json!(0), &[(0.0, json!(0)), (4.0, json!(360))]),
        );
        layers.push(o);
    }
    let mut floor = object3d(
        "floor",
        "plane",
        json!({ "width": 14, "height": 10 }),
        "#808080",
        1920,
        1080,
        4.0,
    );
    set(&mut floor, "position.y", json!(-0.6));
    set(&mut floor, "rotation.x", json!(-90));
    layers.push(floor);
    let doc = load(&project(1920, 1080, 4.0, solid_fill("#000000"), layers));
    let mut r = Renderer::headless().unwrap();
    let request = RenderRequest::at_scale(Size::new(1920, 1080), 1.0, Precision::Float16);
    let mut times = Vec::new();
    for i in 0..45 {
        let t0 = std::time::Instant::now();
        let frame = r
            .render(
                &doc,
                "Ptest-root",
                ticks(f64::from(i) / 30.0),
                &request,
                &MapResolver::default(),
            )
            .unwrap();
        let _ = r.device().poll(wgpu::PollType::wait_indefinitely());
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        if i >= 5 {
            times.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
    }
    times.sort_by(f64::total_cmp);
    eprintln!(
        "3D 1920×1080, 7 layers, 3 shadow lights: median {:.2} ms, p95 {:.2} ms (CPU plan + GPU)",
        times[times.len() / 2],
        times[times.len() * 95 / 100]
    );
}

#[test]
fn frame_requests_carry_the_view_except_for_final_purposes() {
    use pm_protocol::frame::FramePurpose;
    let mut request = pm_protocol::samples::viewer_frame_request();
    request.view = Some(ViewOverride {
        camera: None,
        shading: Shading3d::Wireframe,
        xray: false,
        local_view: Vec::new(),
    });
    request.region = None;
    // The plain conversion refuses a view (callers that ignore it route).
    assert!(RenderRequest::from_frame_request(&request).is_err());
    let (r, view) = RenderRequest::from_frame_request_with_view(&request).unwrap();
    assert_eq!(view.unwrap().shading, Shading3d::Wireframe);
    assert_eq!(r.output_size, request.output_size);
    // An export never applies the view.
    let mut export = request.clone();
    export.purpose = FramePurpose::Export;
    export.priority = pm_protocol::priority::PriorityClass::UserJob;
    export.resolution = pm_protocol::frame::PreviewResolution::Full;
    export.media = pm_protocol::frame::MediaPreference::Originals;
    export.color = pm_protocol::color::OutputColor::SrgbImage;
    export.motion_blur = pm_protocol::frame::MotionBlurKind::Export;
    export.depth_id = false;
    export.overlay = false;
    export.draft_3d = false;
    export.draft_render = false;
    let (r, view) = RenderRequest::from_frame_request_with_view(&export).unwrap();
    assert!(view.is_none() && r.export, "exports ignore view overrides");
}

/// The corpus legacy scene layer (`3d/legacy-scene`): a cube and a torus
/// parented to it, a sun, and a perspective camera.
fn legacy_scene_layer() -> Value {
    let project: Value = serde_json::from_str(include_str!(
        "../../pm-model/tests/fixtures/corpus/3d-legacy-scene.json"
    ))
    .unwrap();
    let d = project["layers"][0]["d"].clone();
    assert_eq!(d["definition"], "powermove.3d.scene");
    let mut l = layer("s", "extension", d, 0.0, 0.0, 2.0);
    l["p"]["anchor.x"] = ch(json!(0));
    l["p"]["anchor.y"] = ch(json!(0));
    l
}

fn features(doc: &pm_model::Document) -> Vec<String> {
    pm_render::unsupported(doc, "Ptest-root")
        .into_iter()
        .map(|u| u.feature.to_string())
        .collect()
}

#[test]
fn the_static_check_looks_inside_legacy_scenes() {
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float16);
    let mut r = Renderer::headless().unwrap();
    let mut render = |doc: &pm_model::Document| {
        r.render(
            doc,
            "Ptest-root",
            ticks(0.0),
            &request,
            &MapResolver::default(),
        )
        .map(|f| f.failures.is_empty())
    };
    let base = || project(W, H, 2.0, solid_fill("#000000"), vec![legacy_scene_layer()]);
    // As captured: nothing listed, and it renders.
    let doc = load(&base());
    assert_eq!(features(&doc), Vec::<String>::new());
    assert!(render(&doc).unwrap());

    // Each change is listed, and the ones the 3D pass cannot build also
    // fail at render time: the routing decision holds at every frame.
    let scene = "/layers/0/d/data/scene";
    type Change = Box<dyn Fn(&mut Value)>;
    let cases: Vec<(&str, Change, bool)> = vec![
        (
            "3D: imported models (legacy scene object cube)",
            Box::new(move |doc: &mut Value| {
                *doc.pointer_mut(&format!("{scene}/objects/0/source"))
                    .unwrap() = json!({ "assetId": "m" });
                doc["assets"]["m"] =
                    json!({ "id": "m", "kind": "model", "format": "glb", "name": "m.glb" });
            }),
            true,
        ),
        (
            "3D: knot primitives (legacy scene object ring)",
            Box::new(move |doc: &mut Value| {
                *doc.pointer_mut(&format!("{scene}/objects/1/source"))
                    .unwrap() = json!({ "primitive": "knot" });
            }),
            true,
        ),
        (
            "3D: hemisphere lights (legacy scene light key)",
            Box::new(move |doc: &mut Value| {
                *doc.pointer_mut(&format!("{scene}/lights/0/type")).unwrap() = json!("hemisphere");
            }),
            true,
        ),
        (
            "3D: model-group recipe parts (legacy scene object cube)",
            Box::new(move |doc: &mut Value| {
                doc.pointer_mut(&format!("{scene}/objects/0"))
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .insert(
                        "generation".into(),
                        json!({ "groupId": "g", "partId": "p" }),
                    );
            }),
            false,
        ),
        (
            "3D: SVG texture maps (legacy scene object ring)",
            Box::new(move |doc: &mut Value| {
                *doc.pointer_mut(&format!("{scene}/objects/1/material/maps"))
                    .unwrap() = json!({ "color": "i" });
                doc["assets"]["i"] =
                    json!({ "id": "i", "kind": "image", "format": "svg", "name": "i.svg" });
            }),
            false,
        ),
        (
            "3D: 3D texture map image (media: an image of unrecorded format) (legacy scene object ring)",
            Box::new(move |doc: &mut Value| {
                *doc.pointer_mut(&format!("{scene}/objects/1/material/maps"))
                    .unwrap() = json!({ "roughness": "i" });
                doc["assets"]["i"] = json!({ "id": "i", "kind": "image", "name": "i" });
            }),
            false,
        ),
        (
            "3D: a legacy 3D scene without a scene object",
            Box::new(move |doc: &mut Value| {
                *doc.pointer_mut("/layers/0/d/data/scene").unwrap() = json!(null);
            }),
            true,
        ),
    ];
    for (want, change, fails) in cases {
        let mut doc = base();
        change(&mut doc);
        let doc = load(&doc);
        assert_eq!(features(&doc), vec![want.to_owned()]);
        if fails {
            assert!(
                !render(&doc).unwrap_or(false),
                "{want}: the render cannot draw it"
            );
        }
    }
    // A model asset without a record draws the orange box: not listed.
    let mut doc = base();
    *doc.pointer_mut(&format!("{scene}/objects/0/source"))
        .unwrap() = json!({ "assetId": "gone" });
    let doc = load(&doc);
    assert_eq!(features(&doc), Vec::<String>::new());
    assert!(render(&doc).unwrap());

    // 3D light layers meet the same light check.
    let mut light = sun("sun", [-3.0, 5.0, 4.0], false, 2.0);
    light["d"]["data"]["light"]["type"] = json!("hemisphere");
    let doc = load(&project(
        W,
        H,
        2.0,
        solid_fill("#000000"),
        vec![
            camera("cam", [0.0, 0.0, 6.0], false, 2.0),
            light,
            object3d("a", "box", json!({}), "#FF4040", W, H, 2.0),
        ],
    ));
    assert_eq!(features(&doc), vec!["3D: hemisphere lights".to_owned()]);
    assert!(!render(&doc).unwrap_or(false));
}
