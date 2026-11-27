//! Exercises the production native chrome passes, including a selected mesh
//! behind an occluder. Transport values must survive half-float readback.
mod support;
use pm_gpu::Target;
use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_protocol::frame::ChromeSelection;
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Value, json};
use support::{doc::*, ticks};

#[test]
fn native_depth_ids_and_unoccluded_selection_share_one_view() {
    let (w, h) = (160, 90);
    let behind = object3d("behind", "box", json!({}), "#00FF00", w, h, 2.0);
    let mut front = object3d("front", "box", json!({}), "#FF0000", w, h, 2.0);
    set(&mut front, "position.z", json!(0.75));
    let doc = load(&project(
        w,
        h,
        2.0,
        solid_fill("#000000"),
        vec![camera("cam", [0.0, 0.0, 3.0], false, 2.0), behind, front],
    ));
    let mut r = Renderer::headless().unwrap();
    let target = |name| {
        Target::new(
            r.device(),
            name,
            w,
            h,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
        )
    };
    let depth = target("viewport depth packet");
    let coverage = target("viewport object IDs and selection");
    let (legend, glyphs, selection) = r
        .render_viewport_chrome(
            &doc,
            "Ptest-root",
            ticks(0.0),
            &MapResolver::default(),
            None,
            Some(&ChromeSelection {
                selected: vec!["behind".into()],
                active: vec![],
            }),
            (w, h),
            [0.0, 0.0, f64::from(w), f64::from(h)],
            &depth.view,
            &coverage.view,
        )
        .unwrap();
    assert_eq!(legend, vec!["behind", "front"]);
    assert_eq!(glyphs.len(), 1);
    assert_eq!(glyphs[0].id, "cam");
    assert_eq!(selection.selected, vec!["behind"]);
    let d = pm_gpu::readback::read_f32(r.device(), r.queue(), &depth.texture).unwrap();
    let c = pm_gpu::readback::read_f32(r.device(), r.queue(), &coverage.texture).unwrap();
    let at = ((h / 2 * w + w / 2) * 4) as usize;
    let bits = (d[at] * 2048.0).round() as u32
        | ((d[at + 1] * 2048.0).round() as u32) << 11
        | ((d[at + 2] * 2048.0).round() as u32) << 22;
    let window_depth = f32::from_bits(bits);
    assert!(
        window_depth > 0.0 && window_depth < 1.0,
        "finite native window depth: {window_depth}"
    );
    let id = (c[at] * 2048.0).round() as usize | ((c[at + 1] * 2048.0).round() as usize) << 11;
    assert_eq!(legend[id - 1], "front", "topmost ID");
    assert_eq!(
        c[at + 2],
        0.5,
        "selection behind the occluder is still covered"
    );
    assert_eq!(c[3], 1.0);
}

/// A one-triangle GLB filling the view; `normals` NORMAL entries for its
/// three vertices, optionally a `SkinnedMesh` and/or KHR_materials_unlit.
fn triangle_glb(normals: usize, skinned: bool, unlit: bool) -> Vec<u8> {
    let mut bin = Vec::new();
    let mut push = |values: &[f32]| {
        for v in values {
            bin.extend_from_slice(&v.to_le_bytes());
        }
    };
    push(&[-3.0, -3.0, 0.0, 3.0, -3.0, 0.0, 0.0, 3.0, 0.0]);
    push(&[0.0, 0.0, 1.0].repeat(normals));
    let mut views = vec![
        json!({"buffer": 0, "byteOffset": 0, "byteLength": 36}),
        json!({"buffer": 0, "byteOffset": 36, "byteLength": 12 * normals}),
    ];
    let mut accessors = vec![
        json!({"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
            "min": [-3, -3, 0], "max": [3, 3, 0]}),
        json!({"bufferView": 1, "componentType": 5126, "count": normals, "type": "VEC3"}),
    ];
    let mut attributes = json!({"POSITION": 0, "NORMAL": 1});
    let mut node = json!({"mesh": 0});
    let mut nodes_extra = Vec::new();
    if skinned {
        let joints = bin.len();
        bin.extend_from_slice(&[0_u8; 12]);
        let weights = bin.len();
        for _ in 0..3 {
            for v in [1.0_f32, 0.0, 0.0, 0.0] {
                bin.extend_from_slice(&v.to_le_bytes());
            }
        }
        views.push(json!({"buffer": 0, "byteOffset": joints, "byteLength": 12}));
        views.push(json!({"buffer": 0, "byteOffset": weights, "byteLength": 48}));
        accessors.push(json!({"bufferView": 2, "componentType": 5121, "count": 3, "type": "VEC4"}));
        accessors.push(json!({"bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC4"}));
        attributes["JOINTS_0"] = json!(2);
        attributes["WEIGHTS_0"] = json!(3);
        node["skin"] = json!(0);
        nodes_extra.push(json!({"name": "joint"}));
    }
    let mut primitive = json!({"attributes": attributes});
    let mut gltf = json!({
        "asset": {"version": "2.0"}, "scene": 0,
        "meshes": [{"primitives": []}],
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": views,
        "accessors": accessors,
    });
    if unlit {
        primitive["material"] = json!(0);
        gltf["extensionsUsed"] = json!(["KHR_materials_unlit"]);
        gltf["materials"] = json!([{
            "pbrMetallicRoughness": {"baseColorFactor": [1, 0, 0, 1]},
            "extensions": {"KHR_materials_unlit": {}}
        }]);
    }
    gltf["meshes"][0]["primitives"] = json!([primitive]);
    let mut nodes = vec![node];
    nodes.extend(nodes_extra);
    gltf["scenes"] = json!([{"nodes": (0..nodes.len()).collect::<Vec<_>>()}]);
    gltf["nodes"] = Value::Array(nodes);
    if skinned {
        gltf["skins"] = json!([{"joints": [1]}]);
    }
    let mut text = serde_json::to_vec(&gltf).unwrap();
    while !text.len().is_multiple_of(4) {
        text.push(b' ');
    }
    let mut glb = Vec::new();
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2_u32.to_le_bytes());
    glb.extend_from_slice(&(12_u32 + 8 + text.len() as u32 + 8 + bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(&(text.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&text);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&bin);
    glb
}

/// Lit pixels of the color frame, and the covered/ID'd/selected pixels of
/// the native chrome planes, for one GLB on a black background.
fn color_and_chrome(glb: &[u8], name: &str) -> (usize, usize, usize, usize) {
    let (w, h) = (96, 64);
    struct TempModel(std::path::PathBuf);
    impl Drop for TempModel {
        fn drop(&mut self) {
            let result = std::fs::remove_dir_all(&self.0);
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = TempModel(std::env::temp_dir().join(format!(
        "pm-render-chrome-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )));
    std::fs::create_dir_all(&dir.0).unwrap();
    let path = dir.0.join("model.glb");
    std::fs::write(&path, glb).unwrap();
    let mut o = object3d("model", "box", json!({}), "#FFFFFF", w, h, 2.0);
    o["d"]["data"]["object"]["source"] = json!({ "assetId": "model" });
    // The file's own (unlit or default Standard) material draws.
    o["d"]["data"]["object"]["useSourceMaterials"] = json!(true);
    let mut doc = project(
        w,
        h,
        2.0,
        solid_fill("#000000"),
        vec![
            camera("cam", [0.0, 0.0, 6.0], false, 2.0),
            sun("sun", [0.0, 0.0, 5.0], false, 2.0),
            o,
        ],
    );
    doc["assets"]["model"] =
        json!({ "id": "model", "kind": "model", "format": "glb", "name": "model.glb" });
    let doc = load(&doc);
    let mut assets = MapResolver::default();
    assets.paths.insert("model".into(), path);
    let mut r = Renderer::headless().unwrap();
    let frame = r
        .render(
            &doc,
            "Ptest-root",
            ticks(0.0),
            &RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float16),
            &assets,
        )
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let rgba = r.read_rgba(&frame).unwrap();
    let lit = rgba
        .chunks(4)
        .filter(|p| p[0] > 0.01 || p[1] > 0.01 || p[2] > 0.01)
        .count();
    let target = |label| {
        Target::new(
            r.device(),
            label,
            w,
            h,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
        )
    };
    let depth = target("viewport depth packet");
    let coverage = target("viewport object IDs and selection");
    r.render_viewport_chrome(
        &doc,
        "Ptest-root",
        ticks(0.0),
        &assets,
        None,
        Some(&ChromeSelection {
            selected: vec!["model".into()],
            active: vec![],
        }),
        (w, h),
        [0.0, 0.0, f64::from(w), f64::from(h)],
        &depth.view,
        &coverage.view,
    )
    .unwrap();
    let d = pm_gpu::readback::read_f32(r.device(), r.queue(), &depth.texture).unwrap();
    let c = pm_gpu::readback::read_f32(r.device(), r.queue(), &coverage.texture).unwrap();
    let far = 1.0_f32.to_bits();
    let depth_px = d
        .chunks(4)
        .filter(|p| {
            let bits = (p[0] * 2048.0).round() as u32
                | ((p[1] * 2048.0).round() as u32) << 11
                | ((p[2] * 2048.0).round() as u32) << 22;
            bits != far
        })
        .count();
    let ids = c.chunks(4).filter(|p| p[0] != 0.0 || p[1] != 0.0).count();
    let mask = c.chunks(4).filter(|p| p[2] != 0.0).count();
    (lit, depth_px, ids, mask)
}

/// three r180's `meshbasic_vert` binds `normal` under `USE_SKINNING`, so
/// today's MeshBasicMaterial overlay overrides (grid-occlusion depth and
/// outline mask, `overlay.ts`) and a skinned unlit color draw are rejected
/// by WebGL for a short NORMAL (GL error 1282, captured in Electron 44 by
/// `e2e/scene3d-short-attributes.spec.ts`). Unskinned, Basic leaves
/// `normal` inactive and draws.
#[test]
fn skinned_short_normal_rejects_basic_chrome_and_unlit_draws() {
    // Controls: complete normals draw everywhere.
    for (skinned, unlit) in [(true, true), (true, false)] {
        let (lit, depth, ids, mask) = color_and_chrome(&triangle_glb(3, skinned, unlit), "full");
        assert!(
            lit > 0 && depth > 0 && ids > 0 && mask > 0,
            "full normal skinned={skinned} unlit={unlit}: {lit} {depth} {ids} {mask}"
        );
    }
    // Skinned: every MeshBasic program (unlit color, chrome depth/ID/mask)
    // consumes the short NORMAL; the Standard color draw does as well.
    for unlit in [true, false] {
        let (lit, depth, ids, mask) =
            color_and_chrome(&triangle_glb(2, true, unlit), "skinned-short");
        assert_eq!(
            (lit, depth, ids, mask),
            (0, 0, 0, 0),
            "skinned short unlit={unlit}"
        );
    }
    // Unskinned unlit draws; the overrides draw for an unskinned Standard
    // mesh whose own color draw is rejected.
    let (lit, depth, ids, mask) = color_and_chrome(&triangle_glb(2, false, true), "unlit-short");
    assert!(
        lit > 0 && depth > 0 && ids > 0 && mask > 0,
        "unskinned unlit short normal: {lit} {depth} {ids} {mask}"
    );
    let (lit, depth, ids, mask) = color_and_chrome(&triangle_glb(2, false, false), "lit-short");
    assert_eq!(lit, 0, "unskinned Standard short normal color");
    assert!(
        depth > 0 && ids > 0 && mask > 0,
        "unskinned Standard short normal chrome"
    );
}
