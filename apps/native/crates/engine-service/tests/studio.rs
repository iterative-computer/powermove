//! Headless Metal integration. No AppKit window, screenshot or display capture.
#![cfg(feature = "kernel")]
use pm_engine_service::{Headless, Scene, studio};
use pm_protocol::{
    color::Precision,
    common::{Size, Ticks},
};
use pm_render::{OutputKind, RenderRequest, Renderer};
use serde_json::{Value, json};
use std::sync::Arc;

fn document(card: bool, opacity: f64) -> Arc<pm_model::Document> {
    use pm_scene3d::{schema::Role, service::new_layer};
    let mut comp = json!({"id":"test","w":64,"h":64,"fps":30,"dur":1,"backgroundFill":{"type":"none"},"layers":[]});
    let layer = |id, role, data, props| {
        new_layer(&comp, id, id, role, data, props, true, None, 0.).unwrap()
    };
    let camera = layer(
        "camera",
        Role::Camera,
        json!({"camera":{"p":{"fov":41.1120904,"targetX":0,"targetY":0,"targetZ":0}},"environment":{"samples":4,"p":{"ambient":0}}}),
        json!({"position.x":0,"position.y":0,"position.z":5}),
    );
    let light = layer(
        "light",
        Role::Light,
        json!({"light":{"type":"sun","p":{"intensity":0}}}),
        json!({}),
    );
    let mirror = layer(
        "mirror",
        Role::Object,
        json!({"object":{"source":{"primitive":"sphere","parameters":{"radius":1,"segments":48}},"material":{"p":{"color":"#F3F3F3","roughness":0.02,"metalness":1}}}}),
        json!({"position.x":0,"position.y":0,"position.z":0,"opacity":opacity}),
    );
    let mut layers = vec![camera, light, mirror];
    if card {
        layers.push(layer("card",Role::Object,json!({"object":{"source":{"mesh":{"positions":[-6,-6,6,6,-6,6,6,6,6,-6,6,6],"normals":[0,0,-1,0,0,-1,0,0,-1,0,0,-1],"indices":[0,2,1,0,3,2]}},"material":{"p":{"color":"#000000","emissive":"#FF0000","emissiveIntensity":0.5}}}}),json!({"position.x":0,"position.y":0,"position.z":0})));
    }
    comp["layers"] = json!(layers);
    let mut project = json!({"id":"project","compId":"test","comps":{"test":comp},"assets":{}});
    project["colorDepth"] = json!(16);
    Arc::new(
        pm_kernel::DocumentStore::new_untitled(
            pm_model::json::parse(&project.to_string()).unwrap(),
        )
        .snapshot()
        .document
        .as_ref()
        .clone(),
    )
}
fn request() -> RenderRequest {
    let mut r = RenderRequest::sized(Size::new(64, 64), Precision::Float16);
    r.export = true;
    r.output_kind = OutputKind::Final;
    r.background = false;
    r
}
fn draw(renderer: &mut Renderer, doc: Arc<pm_model::Document>) -> Vec<f32> {
    studio::attach(renderer, doc.clone(), &Default::default());
    let f = renderer
        .render(
            &doc,
            "test",
            Ticks::ZERO,
            &request(),
            &pm_render::MapResolver::default(),
        )
        .unwrap();
    assert!(f.failures.is_empty(), "{:?}", f.failures);
    renderer.read_rgba(&f).unwrap()
}
#[test]
fn metal_native_document_reflection_final_repeat_and_service_export() {
    let gpu = pm_gpu::Gpu::new("studio integration").unwrap();
    assert_eq!(gpu.info.backend, wgpu::Backend::Metal);
    let mut renderer = Renderer::new(gpu.device, gpu.queue);
    let dark = draw(&mut renderer, document(false, 100.));
    assert!(
        dark.chunks_exact(4)
            .all(|p| p[0] == 0. && p[1] == 0. && p[2] == 0.)
    );
    let doc = document(true, 100.);
    let before = renderer.studio_counters();
    let gpu_before = pm_perf::snapshot();
    let reflected = draw(&mut renderer, doc.clone());
    assert!(
        reflected
            .chunks_exact(4)
            .filter(|p| p[0] > 0.08 && p[1] == 0. && p[2] == 0.)
            .count()
            > 100,
        "off-camera red card must reflect"
    );
    let counters = renderer.studio_counters();
    assert_eq!(
        counters.traces - before.traces,
        1,
        "one full scene trace for both layers"
    );
    assert_eq!(counters.geometry_uploads - before.geometry_uploads, 1);
    eprintln!(
        "Studio additional scene work: {:?}",
        pm_perf::snapshot().since(&gpu_before)
    );
    let repeated = draw(&mut renderer, doc.clone());
    assert_eq!(
        renderer.studio_counters().geometry_uploads,
        counters.geometry_uploads,
        "same immutable geometry survives source replacement"
    );
    assert_eq!(reflected, repeated, "Final fixed-seed pixels");
    let half = draw(&mut renderer, document(true, 50.));
    assert!(
        reflected
            .chunks_exact(4)
            .zip(half.chunks_exact(4))
            .any(|(a, b)| a[3] > 0.9 && b[3] == 0.5)
    );
    let mut host = Headless::new().unwrap();
    let scene = Scene {
        doc,
        comp: "test".into(),
        media: Default::default(),
        content: 1,
    };
    assert_eq!(
        reflected,
        host.export_reference(&scene, Ticks::ZERO, Size::new(64, 64))
            .unwrap()
    );
    eprintln!("Studio counters: {counters:?}");
}
#[test]
fn missing_revision_and_resources_fail_visibly() {
    use studio::SceneSource;
    let doc = document(true, 100.);
    let source = studio::DocumentSource::new(doc.clone(), Default::default());
    let r = studio::SceneRequest {
        composition: "test".into(),
        revision: doc.revision() + 1,
        ticks: 0,
        width: 64,
        height: 64,
    };
    assert!(source.scene(&r).is_err());
    let mut project: Value = serde_json::from_str(&pm_model::json::to_canonical_string(
        &doc.to_value().unwrap(),
    ))
    .unwrap();
    let object = project["comps"]["test"]["layers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|l| l["id"] == "mirror")
        .unwrap();
    object["d"]["data"]["object"]["source"] = json!({"assetId":"unresolved"});
    let broken = pm_kernel::DocumentStore::new_untitled(
        pm_model::json::parse(&project.to_string()).unwrap(),
    )
    .snapshot()
    .document;
    let source = studio::DocumentSource::new(broken.clone(), Default::default());
    let mut request = r.clone();
    request.revision = broken.revision();
    assert!(
        source
            .scene(&request)
            .err()
            .unwrap()
            .contains("geometry is not loaded")
    );
}

#[test]
fn final_shutter_composition_encodes_deterministically_in_the_windowless_child() {
    encoded_repeat(document(true, 100.), Default::default());
}
fn encoded_repeat(doc: Arc<pm_model::Document>, media: pm_export::render::MediaPaths) {
    encoded_repeat_with_witness(doc, media, |pixels| {
        assert!(
            pixels
                .chunks_exact(4)
                .filter(|p| p[0] > 20 && p[1] == 0 && p[2] == 0)
                .count()
                > 100
        );
    });
}
fn encoded_repeat_with_witness(
    doc: Arc<pm_model::Document>,
    media: pm_export::render::MediaPaths,
    witness: impl Fn(&[u8]),
) {
    use pm_engine_service::jobs::{ExportHandle, ExportRequest};
    use pm_protocol::{
        color::OutputColor,
        common::{FrameRate, SessionId, TickRange},
        priority::{AlphaOutput, ExportFormat, ExportSettings, JobSpec, JobState, QualityTier},
    };
    let mut project: Value = serde_json::from_str(&pm_model::json::to_canonical_string(
        &doc.to_value().unwrap(),
    ))
    .unwrap();
    for layer in project["comps"]["test"]["layers"].as_array_mut().unwrap() {
        layer["mblur"] = json!(true);
    }
    let snapshot = pm_kernel::DocumentStore::new_untitled(
        pm_model::json::parse(&project.to_string()).unwrap(),
    )
    .snapshot();
    let settings = ExportSettings {
        format: ExportFormat::PngSequence { sixteen_bit: false },
        alpha: AlphaOutput::Straight,
        quality: QualityTier::High,
        size: Size::new(64, 64),
        frame_rate: FrameRate::new(30, 1),
        range: TickRange {
            start: Ticks::new(pm_protocol::common::TICKS_PER_SECOND / 30).unwrap(),
            end: Ticks::new(pm_protocol::common::TICKS_PER_SECOND / 10).unwrap(),
        },
        audio: false,
        audio_index_playhead: None,
        motion_blur: true,
        color: OutputColor::SrgbImage,
        precision: Precision::Float16,
        dither: false,
        draft_from_proxies: false,
        delivery: Default::default(),
    };
    let dir = tempfile::tempdir().unwrap();
    let mut outputs = vec![];
    for attempt in ["first", "second"] {
        let destination = dir.path().join(attempt);
        let request = ExportRequest::new(
            "studio-export".into(),
            &snapshot,
            JobSpec::Export {
                session: SessionId::new(1).unwrap(),
                composition: "test".into(),
                settings: settings.clone(),
            },
            pm_engine_service::MediaCatalog {
                originals: media.paths.clone(),
                proxies: Default::default(),
            },
            destination.clone(),
        )
        .unwrap();
        let result = ExportHandle::start(
            std::path::Path::new(env!("CARGO_BIN_EXE_pm-engine-service")),
            request,
        )
        .unwrap()
        .wait()
        .unwrap();
        assert_eq!(result.state, JobState::Done, "{result:?}");
        let mut frames = std::fs::read_dir(destination)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect::<Vec<_>>();
        frames.sort();
        assert_eq!(frames.len(), 2);
        outputs.push(
            frames
                .iter()
                .map(|path| {
                    let bytes = std::fs::read(path).unwrap();
                    let image = pm_render::scene3d::gltf::asset::decode(&bytes).unwrap();
                    assert_eq!((image.width, image.height), (64, 64));
                    witness(&image.rgba);
                    image.rgba
                })
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(
        outputs[0], outputs[1],
        "decoded Final/shutter/export pixels must repeat exactly"
    );
}

/// Real GLB bytes with two nodes sharing one indexed mesh and authored physical
/// material. This exercises the importer, not explicit-document mesh fallback.
fn imported_glb() -> Vec<u8> {
    let pm_model::Value::Object(source) = pm_scene3d::to_model(
        &json!({"primitive":"sphere","parameters":{"radius":1,"segments":48}}),
    ) else {
        unreachable!()
    };
    let g = pm_render::scene3d::model::source_of(&source)
        .unwrap()
        .geometry();
    let mut bin = vec![];
    for value in &g.positions {
        bin.extend(value.to_le_bytes());
    }
    let normal_offset = bin.len();
    for value in &g.normals {
        bin.extend(value.to_le_bytes());
    }
    let index_offset = bin.len();
    for value in &g.indices {
        bin.extend(value.to_le_bytes());
    }
    let mut header = serde_json::to_vec(&json!({
        "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0,1]}],
        "nodes":[{"name":"Main","mesh":0},{"name":"Small","mesh":0,"translation":[1.6,0,0],"scale":[0.25,0.25,0.25]}],
        "buffers":[{"byteLength":bin.len()}],
        "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":normal_offset},{"buffer":0,"byteOffset":normal_offset,"byteLength":index_offset-normal_offset},{"buffer":0,"byteOffset":index_offset,"byteLength":bin.len()-index_offset}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":g.positions.len()/3,"type":"VEC3","min":[-1,-1,-1],"max":[1,1,1]}, {"bufferView":1,"componentType":5126,"count":g.normals.len()/3,"type":"VEC3"},{"bufferView":2,"componentType":5125,"count":g.indices.len(),"type":"SCALAR"}],
        "materials":[{"name":"Imported Mirror","pbrMetallicRoughness":{"baseColorFactor":[0.9,0.9,0.9,1],"metallicFactor":1,"roughnessFactor":0.02}}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1},"indices":2,"material":0}]}]
    })).unwrap();
    while !header.len().is_multiple_of(4) {
        header.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let mut out = vec![];
    for word in [
        0x46546c67u32,
        2,
        (12 + 8 + header.len() + 8 + bin.len()) as u32,
        header.len() as u32,
        0x4e4f534a,
    ] {
        out.extend(word.to_le_bytes());
    }
    out.extend(header);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(0x004e4942u32.to_le_bytes());
    out.extend(bin);
    out
}
fn imported_document() -> Arc<pm_model::Document> {
    let doc = document(true, 100.);
    let mut project: Value = serde_json::from_str(&pm_model::json::to_canonical_string(
        &doc.to_value().unwrap(),
    ))
    .unwrap();
    let object = project["comps"]["test"]["layers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|l| l["id"] == "mirror")
        .unwrap();
    object["d"]["data"]["object"]["source"] = json!({"assetId":"imported"});
    object["d"]["data"]["object"]["useSourceMaterials"] = json!(true);
    for axis in ["x", "y", "z"] {
        object["p"][format!("scale.{axis}")]["v"] = json!(200);
    }
    // The imported physical material must win over this authored override.
    object["d"]["data"]["object"]["material"]["p"]["color"]["v"] = json!("#000000");
    project["assets"]["imported"] =
        json!({"id":"imported","kind":"model","format":"glb","name":"Renamed model"});
    pm_kernel::DocumentStore::new_untitled(pm_model::json::parse(&project.to_string()).unwrap())
        .snapshot()
        .document
}
#[test]
fn metal_imported_multi_object_reflection_preview_final_and_export() {
    use studio::SceneSource;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("asset-0.bin");
    std::fs::write(&path, imported_glb()).unwrap();
    let mut media = pm_export::render::MediaPaths::default();
    media.paths.insert("imported".into(), path);
    let doc = imported_document();
    let source = studio::DocumentSource::new(doc.clone(), media.clone());
    let input = source
        .scene(&studio::SceneRequest {
            composition: "test".into(),
            revision: doc.revision(),
            ticks: 0,
            width: 64,
            height: 64,
        })
        .unwrap();
    assert_eq!(
        input.instances.len(),
        3,
        "two imported parts plus the reflection card: {}",
        pm_model::json::to_canonical_string(&doc.to_value().unwrap())
    );
    assert!(
        Arc::ptr_eq(&input.instances[0].mesh, &input.instances[1].mesh),
        "imported instances share immutable geometry"
    );
    assert_ne!(input.instances[0].world.0, input.instances[1].world.0);
    assert_eq!(
        input.materials.as_ref().unwrap()[0].base[3],
        1.,
        "source metallic material"
    );
    let gpu = pm_gpu::Gpu::new("imported studio").unwrap();
    assert_eq!(gpu.info.backend, wgpu::Backend::Metal);
    let mut renderer = Renderer::new(gpu.device, gpu.queue);
    let mut outputs = vec![];
    for _ in 0..2 {
        studio::attach(&mut renderer, doc.clone(), &media);
        let frame = renderer
            .render(
                &doc,
                "test",
                Ticks::ZERO,
                &request(),
                &pm_render::MapResolver::default(),
            )
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        let pixels = renderer.read_rgba(&frame).unwrap();
        assert!(
            pixels
                .chunks_exact(4)
                .filter(|p| p[0] > 0.08 && p[1] == 0. && p[2] == 0.)
                .count()
                > 100,
            "imported metal must reflect the off-camera card; colored={}, max={:?}",
            pixels.chunks_exact(4).filter(|p| p[0] > 0.01).count(),
            pixels.chunks_exact(4).max_by(|a, b| a[0].total_cmp(&b[0]))
        );
        outputs.push(pixels);
    }
    assert_eq!(outputs[0], outputs[1], "fixed-seed imported Final");
    assert_eq!(
        renderer.studio_counters().blas_builds,
        2,
        "one imported mesh and one card"
    );
    let mut preview = request();
    preview.export = false;
    preview.output_kind = OutputKind::Preview;
    let frame = renderer
        .render(
            &doc,
            "test",
            Ticks::ZERO,
            &preview,
            &pm_render::MapResolver::default(),
        )
        .unwrap();
    assert!(frame.failures.is_empty());
    assert!(
        renderer
            .read_rgba(&frame)
            .unwrap()
            .chunks_exact(4)
            .any(|p| p[0] > 0.08)
    );
    let mut host = Headless::new().unwrap();
    assert_eq!(
        outputs[0],
        host.export_reference(
            &Scene {
                doc: doc.clone(),
                comp: "test".into(),
                media: media.clone(),
                content: 1
            },
            Ticks::ZERO,
            Size::new(64, 64)
        )
        .unwrap()
    );
    encoded_repeat(doc, media);
}

#[cfg(feature = "native-cad-tests")]
#[test]
fn metal_imported_step_keeps_xde_parts_preview_final_and_child_export() {
    use pm_scene3d::import::cad::{Format, OpenCascade, Settings, Tessellator};
    use studio::SceneSource;
    let dir = tempfile::tempdir().unwrap();
    assert!(
        std::process::Command::new(
            std::env::var_os("PM_SCENE3D_CAD_FIXTURES").expect("native CAD fixture generator")
        )
        .arg(dir.path())
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
    );
    let cad = OpenCascade {
        helper: std::env::var_os("PM_SCENE3D_CAD_HELPER")
            .expect("native OpenCascade helper")
            .into(),
        scratch_root: dir.path().canonicalize().unwrap(),
    };
    let step = std::fs::read(dir.path().join("product.step")).unwrap();
    let glb = cad
        .convert(Format::Step, &step, Settings::default())
        .unwrap();
    let path = dir.path().join("cad-cache.bin");
    std::fs::write(&path, glb).unwrap();
    let mut media = pm_export::render::MediaPaths::default();
    media.paths.insert("imported".into(), path);
    let mut value: Value = serde_json::from_str(&pm_model::json::to_canonical_string(
        &imported_document().to_value().unwrap(),
    ))
    .unwrap();
    value["assets"]["imported"]["name"] = json!("product.glb");
    value["assets"]["imported"]["format"] = json!("glb");
    let layers = value["comps"]["test"]["layers"].as_array_mut().unwrap();
    layers.retain(|layer| layer["id"] != "card");
    let camera = layers
        .iter_mut()
        .find(|layer| layer["id"] == "camera")
        .unwrap();
    camera["d"]["data"]["environment"]["p"]["ambient"]["v"] = json!(0.5);
    let doc =
        pm_kernel::DocumentStore::new_untitled(pm_model::json::parse(&value.to_string()).unwrap())
            .snapshot()
            .document;
    let source = studio::DocumentSource::new(doc.clone(), media.clone());
    let input = source
        .scene(&studio::SceneRequest {
            composition: "test".into(),
            revision: doc.revision(),
            ticks: 0,
            width: 64,
            height: 64,
        })
        .unwrap();
    assert!(input.instances.len() >= 2, "real CAD assembly parts");
    let materials = input.materials.as_ref().unwrap();
    assert!(materials.len() >= 3, "CAD face colors reach the renderer");
    assert!(materials.iter().any(|m| m.base[2] > m.base[0]));
    let gpu = pm_gpu::Gpu::new("native CAD Studio").unwrap();
    assert_eq!(gpu.info.backend, wgpu::Backend::Metal);
    let mut renderer = Renderer::new(gpu.device, gpu.queue);
    let mut output = vec![];
    for _ in 0..2 {
        studio::attach(&mut renderer, doc.clone(), &media);
        let frame = renderer
            .render(
                &doc,
                "test",
                Ticks::ZERO,
                &request(),
                &pm_render::MapResolver::default(),
            )
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        let pixels = renderer.read_rgba(&frame).unwrap();
        assert!(
            pixels
                .chunks_exact(4)
                .filter(|p| p[3] > 0. && p[..3].iter().any(|v| *v > 0.08))
                .count()
                > 100
        );
        output.push(pixels);
    }
    assert_eq!(output[0], output[1], "fixed-seed CAD Final");
    let mut preview = request();
    preview.export = false;
    preview.output_kind = OutputKind::Preview;
    let frame = renderer
        .render(
            &doc,
            "test",
            Ticks::ZERO,
            &preview,
            &pm_render::MapResolver::default(),
        )
        .unwrap();
    assert!(frame.failures.is_empty());
    assert!(
        renderer
            .read_rgba(&frame)
            .unwrap()
            .chunks_exact(4)
            .any(|p| p[..3].iter().any(|v| *v > 0.08))
    );
    let mut host = Headless::new().unwrap();
    assert_eq!(
        output[0],
        host.export_reference(
            &Scene {
                doc: doc.clone(),
                comp: "test".into(),
                media: media.clone(),
                content: 1
            },
            Ticks::ZERO,
            Size::new(64, 64)
        )
        .unwrap()
    );
    encoded_repeat_with_witness(doc, media, |pixels| {
        assert!(
            pixels
                .chunks_exact(4)
                .filter(|p| p[3] > 0 && p[..3].iter().any(|v| *v > 20))
                .count()
                > 100
        );
    });
}

#[test]
fn metal_imported_splats_keep_gaussian_pixels_layer_order_and_export() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("asset-0.bin");
    let mut bytes = vec![];
    for x in [-1f32, 0., 1.] {
        for v in [x, 0., 0., 0.15, 0.15, 0.15] {
            bytes.extend(v.to_le_bytes());
        }
        bytes.extend([0, 255, 0, 255, 255, 128, 128, 128]);
    }
    std::fs::write(&path, bytes).unwrap();
    let mut media = pm_export::render::MediaPaths::default();
    media.paths.insert("imported".into(), path);
    let source = imported_document();
    let mut value: Value = serde_json::from_str(&pm_model::json::to_canonical_string(
        &source.to_value().unwrap(),
    ))
    .unwrap();
    value["assets"]["imported"]["name"] = json!("capture.splat");
    value["assets"]["imported"]["format"] = json!("splat");
    value["comps"]["test"]["w"] = json!(65);
    value["comps"]["test"]["h"] = json!(65);
    let layers = value["comps"]["test"]["layers"].as_array_mut().unwrap();
    layers.retain(|l| l["id"] != "card");
    let model = layers.iter_mut().find(|l| l["id"] == "mirror").unwrap();
    for axis in ["x", "y", "z"] {
        model["p"][format!("scale.{axis}")]["v"] = json!(100);
    }
    let doc =
        pm_kernel::DocumentStore::new_untitled(pm_model::json::parse(&value.to_string()).unwrap())
            .snapshot()
            .document;
    let mut renderer = Renderer::headless().unwrap();
    studio::attach(&mut renderer, doc.clone(), &media);
    let mut request = request();
    request.output_size = Size::new(65, 65);
    let mut output = vec![];
    for _ in 0..2 {
        let frame = renderer
            .render(
                &doc,
                "test",
                Ticks::ZERO,
                &request,
                &pm_render::MapResolver::default(),
            )
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        let pixels = renderer.read_rgba(&frame).unwrap();
        assert_eq!(
            &pixels[(32 * 65 + 32) * 4..][..4],
            &[0., 1., 0., 1.],
            "source Gaussian at its center"
        );
        assert!(
            pixels.chunks_exact(4).any(|p| p[3] > 0. && p[3] < 0.5),
            "Gaussian falloff must survive"
        );
        assert!(pixels.chunks_exact(4).all(|p| p[0] == 0. && p[2] == 0.));
        output.push(pixels);
    }
    assert_eq!(output[0], output[1], "fixed imported splat repeat");
    assert_eq!(
        renderer.studio_counters().traces,
        0,
        "splats never enter the surface tracer"
    );
    request.export = false;
    request.output_kind = OutputKind::Preview;
    let frame = renderer
        .render(
            &doc,
            "test",
            Ticks::ZERO,
            &request,
            &pm_render::MapResolver::default(),
        )
        .unwrap();
    assert!(frame.failures.is_empty());
    assert!(
        renderer
            .read_rgba(&frame)
            .unwrap()
            .chunks_exact(4)
            .any(|p| p[1] > 0. && p[3] > 0.)
    );
    let mut host = Headless::new().unwrap();
    assert_eq!(
        output[0],
        host.export_reference(
            &Scene {
                doc: doc.clone(),
                comp: "test".into(),
                media: media.clone(),
                content: 1
            },
            Ticks::ZERO,
            Size::new(65, 65)
        )
        .unwrap()
    );
    encoded_repeat_with_witness(doc, media, |pixels| {
        assert!(
            pixels
                .chunks_exact(4)
                .filter(|p| p[1] > 20 && p[3] > 0)
                .count()
                > 100
        );
    });
}
