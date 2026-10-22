//! Files the 3D pass reads and the GPU resources it keeps (plan section 8):
//! material maps with transparency are routed before a frame renders, and
//! one met at render time routes the frame instead of failing the layer;
//! offline, unreadable, and vanished maps draw the model without the map
//! with a `Missing texture` warning, as today's runtime does; OBJ geometry
//! follows the file version; meshes and maps no recent 3D frame drew are
//! released.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::plan::{CompPlan, Content, Source};
use pm_render::scene3d::{Error3d, Object3dPlan, Scene3dRenderer};
use pm_render::{MapResolver, RenderError, RenderRequest, Renderer};
use serde_json::{Value, json};
use support::doc::*;
use support::ticks;

const W: u32 = 160;
const H: u32 = 90;

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("pm-render-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A 16 × 16 checker of two colors; `alpha` on one pixel.
fn write_map(path: &Path, a: [u8; 3], b: [u8; 3], alpha: u8) {
    let mut rgba = Vec::new();
    for y in 0..16 {
        for x in 0..16 {
            let c = if (x / 4 + y / 4) % 2 == 0 { a } else { b };
            rgba.extend(c);
            rgba.push(if (x, y) == (3, 5) { alpha } else { 255 });
        }
    }
    let file = std::fs::File::create(path).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(file), 16, 16);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(&rgba).unwrap();
}

/// A lit box whose material's color map is the image asset `tex` (when
/// `map`), filling most of the frame.
fn textured(map: bool) -> Value {
    let mut o = object3d("o", "box", json!({}), "#FFFFFF", W, H, 2.0);
    set(&mut o, "rotation.y", json!(30));
    set(&mut o, "rotation.x", json!(20));
    if map {
        o["d"]["data"]["object"]["material"]["maps"] = json!({ "color": "tex" });
    }
    let mut doc = project(
        W,
        H,
        2.0,
        solid_fill("#000000"),
        vec![
            camera("cam", [0.0, 0.0, 3.0], false, 2.0),
            sun("sun", [-3.0, 5.0, 4.0], false, 2.0),
            o,
        ],
    );
    doc["assets"]["tex"] =
        json!({ "id": "tex", "kind": "image", "format": "png", "name": "tex.png" });
    doc
}

fn resolver(id: &str, path: &Path) -> MapResolver {
    let mut r = MapResolver::default();
    r.paths.insert(id.into(), path.to_path_buf());
    r
}

fn request() -> RenderRequest {
    RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float16)
}

fn render(r: &mut Renderer, doc: &Value, assets: &MapResolver) -> Result<Vec<f32>, RenderError> {
    let frame = r.render(&load(doc), "Ptest-root", ticks(0.0), &request(), assets)?;
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    Ok(r.read_rgba(&frame).unwrap())
}

/// The 3D object renders of a frame's plan.
fn object_plans(r: &mut Renderer, doc: &Value, assets: &MapResolver) -> Vec<Object3dPlan> {
    let (plan, failures) = r
        .plan(&load(doc), "Ptest-root", ticks(0.0), &request(), assets)
        .unwrap();
    assert!(failures.is_empty(), "{failures:?}");
    fn collect(plan: &CompPlan, out: &mut Vec<Object3dPlan>) {
        for d in &plan.draws {
            if let Content::Texture {
                source: Source::Object3d(o),
                ..
            } = &d.content
            {
                out.push((**o).clone());
            }
        }
    }
    let mut out = Vec::new();
    collect(&plan, &mut out);
    out
}

#[test]
fn maps_with_transparency_draw_and_missing_maps_draw_without_them() {
    let dir = Dir::new("3d-maps");
    let opaque = dir.0.join("opaque.png");
    let clear = dir.0.join("clear.png");
    write_map(&opaque, [255, 40, 40], [40, 40, 255], 255);
    write_map(&clear, [255, 40, 40], [40, 40, 255], 128);
    let mut r = Renderer::headless().unwrap();
    let plain = render(&mut r, &textured(false), &MapResolver::default()).unwrap();

    // An opaque map draws, and changes the model.
    let doc = load(&textured(true));
    let with = resolver("tex", &opaque);
    assert!(pm_render::unsupported_with_media(&doc, "Ptest-root", &with).is_empty());
    let mapped = render(&mut r, &textured(true), &with).unwrap();
    assert_ne!(mapped, plain, "the map colors the model");

    // A map with translucent pixels draws too: three.js samples Chromium's
    // premultiplied bitmap (tests/threed_maps.rs), so the model darkens
    // where the map is translucent; nothing is listed.
    let with_clear = resolver("tex", &clear);
    assert!(pm_render::unsupported_with_media(&doc, "Ptest-root", &with_clear).is_empty());
    let translucent = render(&mut r, &textured(true), &with_clear).unwrap();
    assert_ne!(
        translucent, mapped,
        "premultiplied texels differ from the opaque map's"
    );
    assert_ne!(translucent, plain);

    // Offline (the resolver still names a path) and unreadable maps: the
    // model draws without the map, the check does not route, and the scene
    // warns as today's runtime does.
    let corrupt = dir.0.join("corrupt.png");
    std::fs::write(&corrupt, b"not a png").unwrap();
    for missing in [dir.0.join("offline.png"), corrupt] {
        let assets = resolver("tex", &missing);
        assert!(
            pm_render::unsupported_with_media(&doc, "Ptest-root", &assets).is_empty(),
            "{}",
            missing.display()
        );
        let pixels = render(&mut r, &textured(true), &assets).unwrap();
        assert_eq!(pixels, plain, "{} draws as no map", missing.display());
        let plans = object_plans(&mut r, &textured(true), &assets);
        assert_eq!(plans.len(), 1);
        assert_eq!(
            plans[0].scene.warnings,
            vec!["Missing texture: tex".to_owned()]
        );
    }
}

/// Renders `plan` with the 3D pass alone.
fn render_plan(
    gpu: &pm_gpu::Gpu,
    r: &mut Scene3dRenderer,
    plan: &Object3dPlan,
) -> Result<Vec<u8>, Error3d> {
    let mut pool = pm_gpu::TargetPool::new();
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let out = r.render(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &mut pool,
        plan,
        false,
    )?;
    gpu.queue.submit([encoder.finish()]);
    Ok(pm_gpu::readback::read_raw(&gpu.device, &gpu.queue, &out.color.texture).unwrap())
}

#[test]
fn maps_that_change_after_planning_route_or_drop() {
    let dir = Dir::new("3d-maps-late");
    let path = dir.0.join("tex.png");
    write_map(&path, [255, 40, 40], [40, 40, 255], 255);
    let mut r = Renderer::headless().unwrap();
    let plan = object_plans(&mut r, &textured(true), &resolver("tex", &path))
        .pop()
        .unwrap();
    let bare = object_plans(&mut r, &textured(false), &MapResolver::default())
        .pop()
        .unwrap();
    assert!(plan.scene.warnings.is_empty());
    let gpu = pm_gpu::Gpu::new("pm-render 3d map test").expect("a Metal device");
    let mut scene3d = Scene3dRenderer::default();
    let mapped = render_plan(&gpu, &mut scene3d, &plan).unwrap();
    let plain = render_plan(&gpu, &mut scene3d, &bare).unwrap();
    assert_ne!(mapped, plain);

    // The file gains transparency after the check: the render draws its
    // new texels (premultiplied, as today's bitmap), not a failure.
    write_map(&path, [255, 40, 40], [40, 40, 255], 0);
    let mut fresh = Scene3dRenderer::default();
    let cleared = render_plan(&gpu, &mut fresh, &plan).expect("a translucent map draws");
    assert_ne!(cleared, mapped);

    // The file vanishes after the plan: the model draws without the map.
    std::fs::remove_file(&path).unwrap();
    let mut fresh = Scene3dRenderer::default();
    let vanished = render_plan(&gpu, &mut fresh, &plan).unwrap();
    assert!(vanished == plain, "a vanished map draws as no map");
}

const CUBE_OBJ: &str = "v -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\nv -1 -1 1\nv 1 -1 1\nv 1 1 1\nv -1 1 1\n\
f 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 4 1 5 8\n";

/// A thin slab in place of the cube (the same path, re-saved).
const SLAB_OBJ: &str = "v -1.5 -0.2 -0.2\nv 1.5 -0.2 -0.2\nv 1.5 0.2 -0.2\nv -1.5 0.2 -0.2\n\
v -1.5 -0.2 0.2\nv 1.5 -0.2 0.2\nv 1.5 0.2 0.2\nv -1.5 0.2 0.2\n\
f 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 4 1 5 8\n# a re-saved model\n";

fn obj_doc() -> Value {
    let mut o = object3d("o", "box", json!({}), "#FF4040", W, H, 2.0);
    o["d"]["data"]["object"]["source"] = json!({ "assetId": "model" });
    set(&mut o, "rotation.y", json!(25));
    let mut doc = project(
        W,
        H,
        2.0,
        solid_fill("#000000"),
        vec![
            camera("cam", [0.0, 0.0, 6.0], false, 2.0),
            sun("sun", [-3.0, 5.0, 4.0], false, 2.0),
            o,
        ],
    );
    doc["assets"]["model"] =
        json!({ "id": "model", "kind": "model", "format": "obj", "name": "model.obj" });
    doc
}

/// Legacy parseScene's 500,000 scene-geometry budget counts only inline
/// geometry. A 600,001-vertex imported model draws in three placements below
/// SceneRuntime's separate two-million imported-vertex limit. A fourth
/// placement crosses that legacy limit and fails visibly at the layer.
#[test]
fn imported_model_above_inline_budget_keeps_all_placements() {
    const VERTICES: usize = 600_001;
    let dir = Dir::new("3d-imported-budget");
    let path = dir.0.join("model.glb");
    let mut bin = vec![0_u8; VERTICES * 12 + 8];
    for (i, value) in [-1.0_f32, -1.0, 0.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0]
        .into_iter()
        .enumerate()
    {
        bin[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    let index_offset = VERTICES * 12;
    bin[index_offset..index_offset + 6].copy_from_slice(&[0, 0, 1, 0, 2, 0]);
    let graph = json!({
        "asset": {"version": "2.0"}, "scene": 0,
        "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
        "buffers": [{"byteLength": VERTICES * 12 + 6}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": VERTICES * 12},
            {"buffer": 0, "byteOffset": index_offset, "byteLength": 6}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": VERTICES,
                "type": "VEC3", "min": [-1, -1, 0], "max": [1, 1, 0]},
            {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}
        ]
    });
    let mut json = serde_json::to_vec(&graph).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut glb = Vec::with_capacity(12 + 8 + json.len() + 8 + bin.len());
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2_u32.to_le_bytes());
    glb.extend_from_slice(&(12_u32 + 8 + json.len() as u32 + 8 + bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&bin);
    std::fs::write(&path, glb).unwrap();

    let mut doc = obj_doc();
    doc["assets"]["model"]["format"] = json!("glb");
    doc["assets"]["model"]["name"] = json!("model.glb");
    let base = doc["layers"][2].clone();
    for i in 1..3 {
        let mut layer = base.clone();
        layer["id"] = json!(format!("o{i}"));
        set(&mut layer, "position.x", json!(i as f64 - 1.5));
        doc["layers"].as_array_mut().unwrap().push(layer);
    }
    let mut renderer = Renderer::headless().unwrap();
    let plans = object_plans(&mut renderer, &doc, &resolver("model", &path));
    assert_eq!(plans.len(), 3);
    assert!(
        plans
            .iter()
            .all(|plan| Arc::ptr_eq(&plans[0].scene, &plan.scene))
    );
    let imported: Vec<_> = plans[0]
        .scene
        .objects
        .iter()
        .filter(|object| matches!(object.id.as_str(), "o" | "o1" | "o2"))
        .collect();
    assert_eq!(imported.len(), 3);
    assert!(imported.iter().all(|object| object.meshes.len() == 1));
    assert!(
        imported
            .iter()
            .all(|object| object.meshes[0].geometry.geometry.vertex_count() == VERTICES)
    );

    let mut fourth = base;
    fourth["id"] = json!("o3");
    set(&mut fourth, "position.x", json!(1.5));
    doc["layers"].as_array_mut().unwrap().push(fourth);
    let (_, failures) = renderer
        .plan(
            &load(&doc),
            "Ptest-root",
            ticks(0.0),
            &request(),
            &resolver("model", &path),
        )
        .unwrap();
    assert!(!failures.is_empty());
    assert!(
        failures
            .iter()
            .all(|failure| failure.message.contains("two million vertices"))
    );
}

#[test]
fn obj_source_vertices_count_toward_the_legacy_imported_scene_limit() {
    // The OBJ asset store reports sourceVertexCount, rather than the three
    // expanded draw vertices of this single face. SceneRuntime.sync sums that
    // asset.vertices field for every placement, including repeated assets.
    const VERTICES: usize = 500_001;
    let dir = Dir::new("3d-obj-placed-budget");
    let path = dir.0.join("model.obj");
    let mut source = String::from("v -1 -1 0\nv 1 -1 0\nv 0 1 0\n");
    for _ in 3..VERTICES {
        source.push_str("v 0 0 0\n");
    }
    source.push_str("f 1 2 3\n");
    std::fs::write(&path, source).unwrap();
    let mut doc = obj_doc();
    let base = doc["layers"][2].clone();
    for i in 1..4 {
        let mut layer = base.clone();
        layer["id"] = json!(format!("o{i}"));
        doc["layers"].as_array_mut().unwrap().push(layer);
    }
    let mut renderer = Renderer::headless().unwrap();
    let (_, failures) = renderer
        .plan(
            &load(&doc),
            "Ptest-root",
            ticks(0.0),
            &request(),
            &resolver("model", &path),
        )
        .unwrap();
    assert!(!failures.is_empty());
    assert!(
        failures
            .iter()
            .all(|failure| failure.message.contains("two million vertices"))
    );
}

#[test]
fn obj_geometry_follows_the_file_version() {
    let dir = Dir::new("3d-obj-version");
    let path = dir.0.join("model.obj");
    let assets = resolver("model", &path);
    std::fs::write(&path, CUBE_OBJ).unwrap();
    let mut r = Renderer::headless().unwrap();
    let cube = render(&mut r, &obj_doc(), &assets).unwrap();
    let key = |r: &mut Renderer| {
        let plans = object_plans(r, &obj_doc(), &assets);
        plans[0].scene.objects[0].meshes[0].geometry.key
    };
    let cube_key = key(&mut r);
    assert_eq!(cube_key, key(&mut r), "one file version, one key");

    // The file is re-saved with other geometry while the renderer keeps
    // its GPU meshes: the new mesh is drawn, as a fresh renderer draws it.
    std::fs::write(&path, SLAB_OBJ).unwrap();
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(later)
        .unwrap();
    let slab = render(&mut r, &obj_doc(), &assets).unwrap();
    assert_ne!(key(&mut r), cube_key);
    assert_ne!(slab, cube, "the re-saved model is drawn");
    let mut fresh = Renderer::headless().unwrap();
    assert!(
        render(&mut fresh, &obj_doc(), &assets).unwrap() == slab,
        "the cached renderer draws what a fresh one draws"
    );
}

#[test]
fn meshes_and_maps_no_recent_frame_draws_are_released() {
    let dir = Dir::new("3d-resident");
    let mut r = Renderer::headless().unwrap();
    // Every frame a new geometry (an edited box) and a new map file.
    let mut peak = (0, 0);
    for i in 0..12 {
        let path = dir.0.join(format!("map{i}.png"));
        write_map(&path, [255, 40, 40], [40, 40, 10 * i as u8], 255);
        let mut doc = textured(true);
        doc["layers"][2]["d"]["data"]["object"]["source"]["parameters"] =
            json!({ "width": 1.0 + 0.05 * f64::from(i) });
        render(&mut r, &doc, &resolver("tex", &path)).unwrap();
        let resident = r.scene3d_resident();
        peak = (peak.0.max(resident.meshes), peak.1.max(resident.maps));
        assert!(resident.mesh_bytes > 0 && resident.map_bytes > 0);
    }
    // At most the last two 3D frames' geometry and maps stay.
    assert!(peak.0 <= 2 && peak.1 <= 2, "peak {peak:?}");

    // Frames without the map release it; geometry still drawn stays.
    let mut doc = textured(false);
    doc["layers"][2]["d"]["data"]["object"]["source"]["parameters"] = json!({ "width": 2.0 });
    for _ in 0..2 {
        render(&mut r, &doc, &MapResolver::default()).unwrap();
    }
    let resident = r.scene3d_resident();
    assert_eq!((resident.meshes, resident.maps), (1, 0), "{resident:?}");
    assert_eq!(resident.map_bytes, 0);
}

#[test]
fn shared_scene_maps_load_once_per_frame() {
    // Two object layers share one scene: the maps are resolved once and
    // the layers' renders agree with a single-layer render of each.
    let dir = Dir::new("3d-maps-shared");
    let path = dir.0.join("tex.png");
    write_map(&path, [255, 200, 40], [40, 200, 255], 255);
    let mut doc = textured(true);
    let mut second = doc["layers"][2].clone();
    second["id"] = json!("o2");
    set(&mut second, "position.x", json!(1.2));
    doc["layers"].as_array_mut().unwrap().push(second);
    let mut r = Renderer::headless().unwrap();
    let assets = resolver("tex", &path);
    render(&mut r, &doc, &assets).unwrap();
    let plans = object_plans(&mut r, &doc, &assets);
    assert_eq!(plans.len(), 2);
    assert!(Arc::ptr_eq(&plans[0].scene, &plans[1].scene));
    assert_eq!(r.scene3d_resident().maps, 1, "one texture for both layers");
}

/// A primitive the scene schema does not know fails the composition's
/// scene, as today's `compositionScene` throws in `parseScene`
/// (`corrupt-scenes.test.ts`): nothing is listed, every object layer fails
/// and draws nothing, and the rest of the frame draws.
#[test]
fn unknown_primitives_fail_the_composition_scene_as_today() {
    let mut r = Renderer::headless().unwrap();
    let mut knot = textured(false);
    let i = knot["layers"]
        .as_array()
        .unwrap()
        .iter()
        .position(|l| l["id"] == "o")
        .unwrap();
    knot["layers"][i]["d"]["data"]["object"]["source"] = json!({ "primitive": "knot" });
    let doc = load(&knot);
    assert!(
        pm_render::unsupported_with_media(&doc, "Ptest-root", &MapResolver::default()).is_empty()
    );
    let frame = r
        .render(
            &doc,
            "Ptest-root",
            ticks(0.0),
            &request(),
            &MapResolver::default(),
        )
        .unwrap();
    let failed: Vec<&str> = frame.failures.iter().map(|f| f.layer.as_str()).collect();
    assert_eq!(failed, vec!["o"]);
    let drawn = r.read_rgba(&frame).unwrap();
    let mut without = textured(false);
    without["layers"][i]["on"] = json!(false);
    let empty = render(&mut r, &without, &MapResolver::default()).unwrap();
    assert_eq!(drawn, empty, "the object layer draws nothing");
}

#[test]
fn foreign_schema_corruption_fails_the_composition_scene() {
    let mut renderer = Renderer::headless().unwrap();
    let cases: [(&str, Value); 4] = [
        (
            "strict object keys",
            json!({"primitive":"box", "foreign": true}),
        ),
        (
            "primitive parameter range",
            json!({"primitive":"sphere", "parameters":{"radius":-1}}),
        ),
        (
            "mesh triangle indices",
            json!({"mesh":{"positions":[0,0,0, 1,0,0, 0,1,0], "indices":[0,1,3]}}),
        ),
        ("unknown source", json!({"primitive":"knot"})),
    ];
    for (label, source) in cases {
        let mut doc = textured(false);
        let i = doc["layers"]
            .as_array()
            .unwrap()
            .iter()
            .position(|l| l["id"] == "o")
            .unwrap();
        doc["layers"][i]["d"]["data"]["object"]["source"] = source;
        let frame = renderer
            .render(
                &load(&doc),
                "Ptest-root",
                ticks(0.0),
                &request(),
                &MapResolver::default(),
            )
            .unwrap();
        assert!(
            frame.failures.iter().any(|f| f.layer == "o"),
            "{label}: {:?}",
            frame.failures
        );
    }
}

/// A morph target with no POSITION is a valid imported model. Three r180
/// fails only when WebGLMorphtargets packs it for a draw, so the failure must
/// stay on the object layer rather than becoming a missing-model placeholder.
#[test]
fn normal_or_color_only_morph_fails_when_drawn_not_when_imported() {
    fn glb(target: &str) -> Vec<u8> {
        let mut bin = Vec::new();
        for value in [-1.0_f32, -1.0, 0.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0] {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        bin.extend_from_slice(&[0; 36]);
        let mut graph = json!({
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [{"mesh": 0}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0},
                "targets": [{}]}]}],
            "buffers": [{"byteLength": 72}],
            "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36},
                {"buffer": 0, "byteOffset": 36, "byteLength": 36}],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                    "min": [-1, -1, 0], "max": [1, 1, 0]},
                {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"}
            ]
        });
        graph["meshes"][0]["primitives"][0]["targets"][0][target] = json!(1);
        let mut json = serde_json::to_vec(&graph).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2_u32.to_le_bytes());
        out.extend_from_slice(
            &(12_u32 + 8 + json.len() as u32 + 8 + bin.len() as u32).to_le_bytes(),
        );
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&json);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin);
        out
    }

    let dir = Dir::new("3d-morph-no-position");
    let mut doc = obj_doc();
    doc["assets"]["model"]["format"] = json!("glb");
    let mut renderer = Renderer::headless().unwrap();
    for target in ["NORMAL", "COLOR_0"] {
        let path = dir.0.join(format!("{target}.glb"));
        std::fs::write(&path, glb(target)).unwrap();
        doc["assets"]["model"]["name"] = json!(format!("{target}.glb"));
        let assets = resolver("model", &path);
        let plans = object_plans(&mut renderer, &doc, &assets);
        assert_eq!(plans.len(), 1, "{target}: imported object layer");
        assert_eq!(
            plans[0].scene.objects[0].meshes.len(),
            1,
            "{target}: imported model is present, not a placeholder"
        );
        let frame = renderer
            .render(&load(&doc), "Ptest-root", ticks(0.0), &request(), &assets)
            .unwrap();
        assert_eq!(frame.failures.len(), 1, "{target}: {:?}", frame.failures);
        assert_eq!(frame.failures[0].layer, "o");
        assert!(
            frame.failures[0].message.contains("reading 'count'"),
            "{target}: {:?}",
            frame.failures
        );
    }
}

/// The plan retains a short-UV glTF mesh even when its scene override map
/// would bind UV at draw time. If that map vanishes after planning, the final
/// material no longer binds UV and the same mesh must draw normally.
#[test]
fn short_uv_model_draws_when_override_map_fails_after_planning() {
    let dir = Dir::new("3d-short-uv-late-map");
    let model_path = dir.0.join("short-uv.glb");
    let map_path = dir.0.join("map.png");
    let mut bin = Vec::new();
    for value in [-1.0_f32, -1.0, 0.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0] {
        bin.extend_from_slice(&value.to_le_bytes());
    }
    for value in [0.0_f32, 0.0, 1.0, 0.0] {
        bin.extend_from_slice(&value.to_le_bytes());
    }
    let mut json = serde_json::to_vec(&json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "TEXCOORD_0": 1}}]}],
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36},
            {"buffer": 0, "byteOffset": 36, "byteLength": 16}],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                "min": [-1, -1, 0], "max": [1, 1, 0]},
            {"bufferView": 1, "componentType": 5126, "count": 2, "type": "VEC2"}
        ]
    }))
    .unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut glb = Vec::new();
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2_u32.to_le_bytes());
    glb.extend_from_slice(&(12_u32 + 8 + json.len() as u32 + 8 + bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&bin);
    std::fs::write(&model_path, glb).unwrap();
    write_map(&map_path, [255, 255, 255], [255, 255, 255], 255);

    let mut doc = obj_doc();
    doc["assets"]["model"]["format"] = json!("glb");
    doc["assets"]["model"]["name"] = json!("short-uv.glb");
    doc["assets"]["tex"] =
        json!({"id": "tex", "kind": "image", "format": "png", "name": "map.png"});
    doc["layers"][2]["d"]["data"]["object"]["material"]["maps"] = json!({"color": "tex"});
    let mut assets = resolver("model", &model_path);
    assets.paths.insert("tex".into(), map_path.clone());
    let mut planner = Renderer::headless().unwrap();
    let mapped = object_plans(&mut planner, &doc, &assets).pop().unwrap();
    assert_eq!(
        mapped.scene.objects[0].meshes.len(),
        1,
        "plan retains model mesh"
    );
    let mut bare_doc = doc.clone();
    bare_doc["layers"][2]["d"]["data"]["object"]["material"]["maps"] = json!({});
    let bare = object_plans(&mut planner, &bare_doc, &assets)
        .pop()
        .unwrap();
    let gpu = pm_gpu::Gpu::new("pm-render short UV late map test").unwrap();
    let expected = render_plan(&gpu, &mut Scene3dRenderer::default(), &bare).unwrap();
    let hidden = render_plan(&gpu, &mut Scene3dRenderer::default(), &mapped).unwrap();
    assert_ne!(hidden, expected, "a loaded map binds the short UV");

    // Chromium's depth program also binds an opaque map's UV. A short
    // mapped caster therefore contributes neither color nor a shadow.
    let mut shadow_doc = doc.clone();
    shadow_doc["layers"][0]["d"]["data"]["environment"]["shadows"] = json!(true);
    shadow_doc["layers"][1]["d"]["data"]["light"]["castShadow"] = json!(true);
    set(&mut shadow_doc["layers"][1], "position.x", json!(0));
    set(&mut shadow_doc["layers"][1], "position.y", json!(0));
    set(&mut shadow_doc["layers"][1], "position.z", json!(4));
    set(&mut shadow_doc["layers"][2], "rotation.y", json!(0));
    let mut receiver = object3d("receiver", "plane", json!({}), "#FFFFFF", W, H, 2.0);
    set(&mut receiver, "position.z", json!(-0.7));
    receiver["d"]["data"]["object"]["castShadow"] = json!(false);
    shadow_doc["layers"].as_array_mut().unwrap().push(receiver);
    let receiver_plan = |planner: &mut Renderer, doc: &Value| {
        object_plans(planner, doc, &assets)
            .into_iter()
            .find(|plan| plan.layer == "receiver")
            .unwrap()
    };
    let cast = receiver_plan(&mut planner, &shadow_doc);
    shadow_doc["layers"][2]["d"]["data"]["object"]["castShadow"] = json!(false);
    let no_cast = receiver_plan(&mut planner, &shadow_doc);
    let cast = render_plan(&gpu, &mut Scene3dRenderer::default(), &cast).unwrap();
    let no_cast = render_plan(&gpu, &mut Scene3dRenderer::default(), &no_cast).unwrap();
    assert!(no_cast.iter().any(|v| *v != 0), "receiver is visible");
    assert!(
        cast == no_cast,
        "opaque mapped short UV cannot cast a depth shadow"
    );

    std::fs::remove_file(&map_path).unwrap();
    let recovered = render_plan(&gpu, &mut Scene3dRenderer::default(), &mapped).unwrap();
    assert_eq!(recovered, expected, "late map failure leaves UV inactive");
}

/// A short NORMAL rejects the Standard color draw, but r180's shadow
/// MeshDepthMaterial has no normal input. The caster must remain available
/// to the shadow pass even though its own color pass cannot draw it.
#[test]
fn short_normal_model_still_casts_a_shadow() {
    let dir = Dir::new("3d-short-normal-shadow");
    let model_path = dir.0.join("short-normal.glb");
    let mut bin = Vec::new();
    for value in [
        -1.0_f32, -1.0, 0.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0,
    ] {
        bin.extend_from_slice(&value.to_le_bytes());
    }
    let mut json = serde_json::to_vec(&json!({
        "asset": {"version": "2.0"}, "scene": 0,
        "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1}}]}],
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36},
            {"buffer": 0, "byteOffset": 36, "byteLength": 24}],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                "min": [-1, -1, 0], "max": [1, 1, 0]},
            {"bufferView": 1, "componentType": 5126, "count": 2, "type": "VEC3"}
        ]
    }))
    .unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut glb = Vec::new();
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2_u32.to_le_bytes());
    glb.extend_from_slice(&(12_u32 + 8 + json.len() as u32 + 8 + bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&bin);
    std::fs::write(&model_path, glb).unwrap();

    let mut doc = obj_doc();
    doc["assets"]["model"]["format"] = json!("glb");
    doc["assets"]["model"]["name"] = json!("short-normal.glb");
    doc["layers"][0]["d"]["data"]["environment"]["shadows"] = json!(true);
    doc["layers"][1]["d"]["data"]["light"]["castShadow"] = json!(true);
    set(&mut doc["layers"][1], "position.x", json!(0));
    set(&mut doc["layers"][1], "position.y", json!(0));
    set(&mut doc["layers"][1], "position.z", json!(4));
    set(&mut doc["layers"][2], "rotation.y", json!(0));
    let mut receiver = object3d("receiver", "plane", json!({}), "#FFFFFF", W, H, 2.0);
    set(&mut receiver, "position.z", json!(-0.7));
    receiver["d"]["data"]["object"]["castShadow"] = json!(false);
    doc["layers"].as_array_mut().unwrap().push(receiver);
    let assets = resolver("model", &model_path);
    let mut planner = Renderer::headless().unwrap();
    let with_shadow = object_plans(&mut planner, &doc, &assets);
    assert_eq!(with_shadow.len(), 2);
    assert_eq!(with_shadow[0].scene.objects[0].meshes.len(), 1);
    let mut without_doc = doc.clone();
    without_doc["layers"][2]["d"]["data"]["object"]["castShadow"] = json!(false);
    let without_shadow = object_plans(&mut planner, &without_doc, &assets);
    let gpu = pm_gpu::Gpu::new("pm-render short normal shadow test").unwrap();
    // Plans are in compositor draw order, not source-layer order. Select
    // the receiver explicitly; the rejected caster's own output is blank.
    let receiver = |plans: &[Object3dPlan]| {
        plans
            .iter()
            .find(|plan| plan.layer == "receiver")
            .unwrap()
            .clone()
    };
    let lit = render_plan(
        &gpu,
        &mut Scene3dRenderer::default(),
        &receiver(&with_shadow),
    )
    .unwrap();
    let unshadowed = render_plan(
        &gpu,
        &mut Scene3dRenderer::default(),
        &receiver(&without_shadow),
    )
    .unwrap();
    assert!(unshadowed.iter().any(|v| *v != 0), "receiver is visible");
    assert!(
        lit != unshadowed,
        "short NORMAL color draw still casts a depth shadow"
    );
}
