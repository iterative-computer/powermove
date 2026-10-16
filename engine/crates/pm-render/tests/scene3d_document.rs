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
fn linear_blending_decodes_the_3d_canvas() {
    use pm_render::scene3d::linear::{LinearPass, reference};
    let gpu = pm_gpu::Gpu::new("pm-render 3d linear").unwrap();
    // Every 8-bit code at several alphas, premultiplied (code ≤ alpha).
    let mut px = Vec::new();
    for a in [255u8, 200, 128, 17, 1, 0] {
        for c in 0..=255u8 {
            let v = (u16::from(c) * u16::from(a) / 255) as u8;
            px.extend_from_slice(&[v, v / 2, c.min(a), a]);
        }
    }
    let n = (px.len() / 4) as u32;
    let texture = wgpu::util::DeviceExt::create_texture_with_data(
        &gpu.device,
        &gpu.queue,
        &wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: n,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &px,
    );
    let source = pm_gpu::Target::wrap(texture);
    let mut pool = pm_gpu::TargetPool::new();
    let mut pass = LinearPass::default();
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let out = pass.convert(
        &gpu.device,
        &mut encoder,
        &mut pool,
        &source,
        wgpu::TextureFormat::Rgba32Float,
    );
    gpu.queue.submit([encoder.finish()]);
    let got = pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &out.texture).unwrap();
    let mut worst = 0f64;
    for (i, p) in px.chunks_exact(4).enumerate() {
        let want = reference([p[0], p[1], p[2], p[3]]);
        for c in 0..4 {
            worst = worst.max((f64::from(got[i * 4 + c]) - want[c]).abs());
        }
    }
    // f32 against the f64 reference.
    assert!(worst < 2e-6, "worst {worst}");
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
