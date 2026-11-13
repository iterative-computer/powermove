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
    // Tiny deterministic transport fixtures choose their sample policy explicitly.
    comp["studioQuality"] = pm_scene3d::quality::resolve(
        &pm_scene3d::quality::preset("production").unwrap(),
        &json!({"minSamples":4,"maxSamples":4,"noiseThreshold":0,"caustics":false}),
    )
    .unwrap();
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
#[test]
fn viewer_surface_query_returns_layer_point_normal_and_rejects_stale_frames() {
    use pm_engine_service::{EngineService, Output, SurfaceQueryKind, ViewerConfig};
    let project = document(false, 100.).to_value().unwrap();
    let snapshot = pm_kernel::DocumentStore::new_untitled(project).snapshot();
    let service = EngineService::start(pm_gpu::Gpu::new("viewer surface query").unwrap()).unwrap();
    let viewer = service
        .viewer(ViewerConfig {
            snapshot,
            composition: "test".into(),
            media: Default::default(),
            output: Output {
                width: 64,
                height: 64,
                display: 1,
                headroom: 1.,
                crop: None,
            },
            layer: None,
            rate: pm_time::Rate::FPS_30,
            end: None,
            audio: false,
        })
        .unwrap();
    let stamp = viewer.stamp().unwrap();
    let receive = viewer
        .pick_surface(stamp, Ticks::ZERO, [0.5, 0.5], SurfaceQueryKind::Click)
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let reply = loop {
        match receive.try_recv() {
            Ok(reply) => break reply.unwrap(),
            Err(async_channel::TryRecvError::Empty) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "surface query did not settle"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(e) => panic!("surface query closed: {e}"),
        }
    };
    assert_eq!(reply.stamp, stamp);
    assert_eq!(reply.time, Ticks::ZERO);
    let hit = reply.hit.unwrap();
    assert_eq!(hit.layer, "mirror");
    assert!(hit.point[0].abs() < 0.01 && hit.point[1].abs() < 0.01 && hit.point[2] > 0.9);
    assert!(hit.normal[2] > 0.9);
    viewer.quality(pm_engine_service::Quality::Half).unwrap();
    assert!(
        viewer
            .pick_surface(stamp, Ticks::ZERO, [0.5, 0.5], SurfaceQueryKind::Click)
            .is_err()
    );
    assert!(
        viewer
            .pick_surface(
                viewer.stamp().unwrap(),
                Ticks::ZERO,
                [f32::NAN, 0.5],
                SurfaceQueryKind::Hover
            )
            .is_err()
    );
}
fn isolation_project() -> Value {
    let mut project: Value = serde_json::from_str(&pm_model::json::to_canonical_string(
        &document(false, 100.).to_value().unwrap(),
    ))
    .unwrap();
    let layers = project["comps"]["test"]["layers"].as_array_mut().unwrap();
    let good = layers.iter_mut().find(|l| l["id"] == "mirror").unwrap();
    good["d"]["data"]["object"]["material"]["p"]["metalness"] = json!(0);
    good["d"]["data"]["object"]["material"]["p"]["color"] = json!("#000000");
    good["d"]["data"]["object"]["material"]["p"]["emissive"] = json!("#00FF00");
    good["d"]["data"]["object"]["material"]["p"]["emissiveIntensity"] = json!(1);
    project
}
#[test]
fn a_broken_model_does_not_blank_its_healthy_sibling() {
    let mut project = isolation_project();
    let layers = project["comps"]["test"]["layers"].as_array_mut().unwrap();
    let good = layers.iter().find(|l| l["id"] == "mirror").unwrap();
    let mut broken = good.clone();
    broken["id"] = json!("broken");
    broken["d"]["data"]["object"]["source"] = json!({"assetId":"missing-model"});
    layers.push(broken);
    let doc = Arc::new(
        pm_kernel::DocumentStore::new_untitled(
            pm_model::json::parse(&project.to_string()).unwrap(),
        )
        .snapshot()
        .document
        .as_ref()
        .clone(),
    );
    let gpu = pm_gpu::Gpu::new("isolated Studio failure").unwrap();
    let mut renderer = Renderer::new(gpu.device, gpu.queue);
    studio::attach(&mut renderer, doc.clone(), &Default::default());
    let frame = renderer
        .render(
            &doc,
            "test",
            Ticks::ZERO,
            &request(),
            &pm_render::MapResolver::default(),
        )
        .unwrap();
    assert_eq!(frame.failures.len(), 1, "{:?}", frame.failures);
    assert_eq!(frame.failures[0].layer, "broken");
    assert!(frame.failures[0].message.contains("geometry is not loaded"));
    let pixels = renderer.read_rgba(&frame).unwrap();
    assert!(
        pixels
            .chunks_exact(4)
            .filter(|p| p[1] > 0.1 && p[1] > p[0] * 2. && p[3] > 0.5)
            .count()
            > 40,
        "healthy green geometry must remain visible"
    );
    project["comps"]["test"]["layers"]
        .as_array_mut()
        .unwrap()
        .retain(|l| l["id"] != "broken");
    let repaired = pm_kernel::DocumentStore::new_untitled(
        pm_model::json::parse(&project.to_string()).unwrap(),
    )
    .snapshot()
    .document;
    studio::attach(&mut renderer, repaired.clone(), &Default::default());
    let frame = renderer
        .render(
            &repaired,
            "test",
            Ticks::ZERO,
            &request(),
            &pm_render::MapResolver::default(),
        )
        .unwrap();
    assert!(
        frame.failures.is_empty(),
        "replacing a bad contribution must clear its failure: {:?}",
        frame.failures
    );
}
#[test]
fn failed_material_images_roll_back_their_atlas_and_keep_healthy_geometry() {
    use studio::SceneSource;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("paint.png");
    pm_studio3d_engine::gpu::write_png(&path, 1, 1, &[255, 255, 255, 255]).unwrap();
    let mut media = pm_export::render::MediaPaths::default();
    media.paths.insert("paint".into(), path);
    for maps in [
        json!({"color":"paint","normal":"missing-image"}),
        json!({"color":{"layerId":"unready-source"}}),
    ] {
        let mut project = isolation_project();
        let layers = project["comps"]["test"]["layers"].as_array_mut().unwrap();
        let mut bad = layers.iter().find(|l| l["id"] == "mirror").unwrap().clone();
        bad["id"] = json!("bad-image");
        bad["d"]["data"]["object"]["material"]["maps"] = maps;
        layers.insert(0, bad);
        let doc = pm_kernel::DocumentStore::new_untitled(
            pm_model::json::parse(&project.to_string()).unwrap(),
        )
        .snapshot()
        .document;
        let source = studio::DocumentSource::new(doc.clone(), media.clone());
        let req = studio::SceneRequest {
            composition: "test".into(),
            revision: doc.revision(),
            ticks: 0,
            width: 64,
            height: 64,
        };
        let input = source.scene(&req).unwrap();
        assert_eq!(input.instances.len(), 1);
        assert!(
            input.maps.is_none(),
            "failed image layers must not retain atlas slots"
        );
        assert_eq!(source.layer_failures(&req).unwrap()[0].layer, "bad-image");
        let mut renderer = Renderer::headless().unwrap();
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
        assert_eq!(frame.failures.len(), 1, "{:?}", frame.failures);
        assert_eq!(frame.failures[0].layer, "bad-image");
        let pixels = renderer.read_rgba(&frame).unwrap();
        assert!(
            pixels
                .chunks_exact(4)
                .filter(|p| p[1] > 0.1 && p[3] > 0.5)
                .count()
                > 40
        );
    }
}
#[test]
fn failed_world_and_scatter_dependencies_leave_the_rest_of_the_scene_visible() {
    use pm_scene3d::{schema::Role, service::new_layer};
    for scatter in [false, true] {
        let mut project = isolation_project();
        let comp = project["comps"]["test"].clone();
        let data = if scatter {
            json!({"object":{"source":{"primitive":"plane"},"scatter":{"model":"broken","surface":"mirror","p":{"count":1}}}})
        } else {
            json!({"object":{"source":{"primitive":"plane"},"world":{"mode":"hdri","source":"missing-hdri"}}})
        };
        let node = new_layer(
            &comp,
            "failed-node",
            "Failed node",
            Role::Object,
            data,
            json!({}),
            true,
            None,
            0.,
        )
        .unwrap();
        let layers = project["comps"]["test"]["layers"].as_array_mut().unwrap();
        if scatter {
            let mut bad = layers.iter().find(|l| l["id"] == "mirror").unwrap().clone();
            bad["id"] = json!("broken");
            bad["d"]["data"]["object"]["source"] = json!({"assetId":"missing-model"});
            layers.push(bad);
        }
        layers.push(node);
        let doc = pm_kernel::DocumentStore::new_untitled(
            pm_model::json::parse(&project.to_string()).unwrap(),
        )
        .snapshot()
        .document;
        let mut renderer = Renderer::headless().unwrap();
        studio::attach(&mut renderer, doc.clone(), &Default::default());
        let frame = renderer
            .render(
                &doc,
                "test",
                Ticks::ZERO,
                &request(),
                &pm_render::MapResolver::default(),
            )
            .unwrap();
        assert_eq!(
            frame.failures.len(),
            if scatter { 2 } else { 1 },
            "{:?}",
            frame.failures
        );
        assert!(frame.failures.iter().any(|f| f.layer == "failed-node"));
        assert!(frame.failures.iter().all(|f| f.layer != "mirror"));
        let pixels = renderer.read_rgba(&frame).unwrap();
        assert!(
            pixels
                .chunks_exact(4)
                .filter(|p| p[1] > 0.1 && p[3] > 0.5)
                .count()
                > 40
        );
    }
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
fn viewer_tiers_reach_the_retained_native_backend_without_document_edits() {
    let gpu = pm_gpu::Gpu::new("studio viewer tiers").unwrap();
    let mut renderer = Renderer::new(gpu.device, gpu.queue);
    let doc = document(true, 100.);
    let original = pm_model::json::to_canonical_string(&doc.to_value().unwrap());
    let scene = studio::SceneRequest {
        composition: "test".into(),
        revision: doc.revision(),
        ticks: 0,
        width: 64,
        height: 64,
    };
    let mut request = request();
    request.export = false;
    request.output_kind = OutputKind::Preview;
    let mut uploads = None;
    for (tier, samples) in [
        (studio::Preview::Live, None),
        (studio::Preview::Refine, Some(32)),
        (studio::Preview::Final, Some(4)),
        (studio::Preview::Live, None),
    ] {
        studio::attach_with_preview(
            &mut renderer,
            doc.clone(),
            &Default::default(),
            &scene,
            tier,
        )
        .unwrap();
        let f = renderer
            .render(
                &doc,
                "test",
                Ticks::ZERO,
                &request,
                &pm_render::MapResolver::default(),
            )
            .unwrap();
        assert!(f.failures.is_empty(), "{tier:?}: {:?}", f.failures);
        let c = renderer.studio_counters();
        if let Some(samples) = samples {
            assert_eq!(c.samples, samples, "{tier:?}");
        }
        if let Some(uploads) = uploads {
            assert_eq!(c.geometry_uploads, uploads);
        }
        uploads = Some(c.geometry_uploads);
    }
    assert_eq!(
        pm_model::json::to_canonical_string(&doc.to_value().unwrap()),
        original
    );
    assert!(studio::Preview::parse("fast").is_err());
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
    let input = source.scene(&request).unwrap();
    assert!(input.instances.iter().all(|i| i.layer != "mirror"));
    let failures = source.layer_failures(&request).unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].layer, "mirror");
    assert!(failures[0].message.contains("geometry is not loaded"));
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
                ..Default::default()
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
fn sparse_intrinsic_edits_override_source_identity_and_restore_original_materials() {
    use studio::SceneSource;
    let bytes = imported_glb();
    let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let mut header: Value = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
    // Final material table slot 0 derives from file material 1, not file slot 0.
    header["materials"] = json!([{}, {"pbrMetallicRoughness":{"baseColorFactor":[1,1,1,1],"metallicFactor":0.8,"roughnessFactor":0.1},"emissiveFactor":[1,0,0],"extensions":{"KHR_materials_emissive_strength":{"emissiveStrength":0}}}]);
    header["meshes"][0]["primitives"][0]["material"] = json!(1);
    let mut json_bytes = serde_json::to_vec(&header).unwrap();
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }
    let bin_chunk = &bytes[20 + length..];
    let mut edited = vec![];
    for word in [
        0x46546c67u32,
        2,
        (20 + json_bytes.len() + bin_chunk.len()) as u32,
        json_bytes.len() as u32,
        0x4e4f534a,
    ] {
        edited.extend(word.to_le_bytes());
    }
    edited.extend(json_bytes);
    edited.extend(bin_chunk);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sparse.bin");
    std::fs::write(&path, edited).unwrap();
    let mut media = pm_export::render::MediaPaths::default();
    media.paths.insert("imported".into(), path);
    let original = imported_document();
    let mut project: Value = serde_json::from_str(&pm_model::json::to_canonical_string(
        &original.to_value().unwrap(),
    ))
    .unwrap();
    let layer = project["comps"]["test"]["layers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|l| l["id"] == "mirror")
        .unwrap();
    layer["d"]["data"]["object"]["materialProperties"] = json!({"opacity":60,"roughness":0.6,"metallic":0.3,"emissionIntensity":2,"ambientResponse":25});
    layer["d"]["data"]["object"]["sourceMaterialProperties"] =
        json!({"0":{"opacity":10},"1":{"roughness":0.2,"emissionColor":"#00FF00"}});
    let doc = pm_kernel::DocumentStore::new_untitled(
        pm_model::json::parse(&project.to_string()).unwrap(),
    )
    .snapshot()
    .document;
    let source = studio::DocumentSource::new(doc.clone(), media.clone());
    let r = studio::SceneRequest {
        composition: "test".into(),
        revision: doc.revision(),
        ticks: 0,
        width: 64,
        height: 64,
    };
    let input = source.scene(&r).unwrap();
    let instance = input
        .instances
        .iter()
        .find(|i| i.layer == "mirror")
        .unwrap();
    let material = &input.materials.as_ref().unwrap()[instance.material];
    assert_eq!(material.extra[2], 0.6);
    assert_eq!(material.spec[0], 0.2);
    assert_eq!(material.base[3], 0.3);
    assert_eq!(&material.emit[..3], &[0., 2., 0.]);
    assert_eq!(material.policy[0], 0.25);
    let gpu = pm_gpu::Gpu::new("sparse source material").unwrap();
    let mut renderer = Renderer::new(gpu.device, gpu.queue);
    studio::attach(&mut renderer, doc.clone(), &media);
    let frame = renderer
        .render(&doc, "test", Ticks::ZERO, &request(), &media)
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let pixels = renderer.read_rgba(&frame).unwrap();
    assert!(
        pixels
            .chunks_exact(4)
            .filter(|p| p[1] > p[0] * 2. && p[1] > 0.2 && p[3] > 0.)
            .count()
            > 40
    );
    // A new immutable source without sparse records returns the file values.
    let source = studio::DocumentSource::new(original.clone(), media);
    let input = source
        .scene(&studio::SceneRequest {
            revision: original.revision(),
            ..r
        })
        .unwrap();
    let instance = input
        .instances
        .iter()
        .find(|i| i.layer == "mirror")
        .unwrap();
    let material = &input.materials.as_ref().unwrap()[instance.material];
    assert_eq!(material.extra[2], 1.);
    assert_eq!(material.spec[0], 0.1);
    assert_eq!(material.base[3], 0.8);
    assert_eq!(&material.emit[..3], &[0., 0., 0.]);
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
                media: Arc::new(media.clone()),
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
                content: 1,
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
                media: Arc::new(media.clone()),
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

fn environment_geometry_document(object: Value, props: Value) -> Arc<pm_model::Document> {
    let old = document(false, 100.);
    let mut project = pm_scene3d::from_model(&old.root().unwrap().to_value()).unwrap();
    let layer = pm_scene3d::service::new_layer(
        &project["comps"]["test"],
        "terrain",
        "Environment geometry",
        pm_scene3d::schema::Role::Object,
        json!({"object":object}),
        props,
        true,
        None,
        0.,
    )
    .unwrap();
    project["comps"]["test"]["layers"]
        .as_array_mut()
        .unwrap()
        .retain(|l| l["id"] != "mirror");
    project["comps"]["test"]["layers"]
        .as_array_mut()
        .unwrap()
        .push(layer);
    pm_kernel::DocumentStore::new_untitled(pm_scene3d::to_model(&project))
        .snapshot()
        .document
}
fn environment_input(doc: Arc<pm_model::Document>) -> studio::SceneInput {
    use studio::SceneSource;
    let r = studio::SceneRequest {
        composition: "test".into(),
        revision: doc.revision(),
        ticks: 0,
        width: 64,
        height: 64,
    };
    studio::DocumentSource::new(doc, Default::default())
        .scene(&r)
        .unwrap()
}
#[test]
fn terrain_signed_heights_regions_erosion_and_natural_colors_reach_native() {
    let terrain = json!({"p":{"resolution":2,"width":4,"depth":4,"amplitude":4},"heights":[-0.5,0,0.5,-0.5,0,0.5,-0.5,0,0.5],"regions":[{"p":{"minHeight":0,"maxHeight":3,"color":"#FF0000","roughness":0.3,"metalness":0.6}}]});
    let object = json!({"source":{"primitive":"plane"},"terrain":terrain});
    let doc = environment_geometry_document(object.clone(), json!({"position.x":3}));
    let input = environment_input(doc.clone());
    let instance = &input.instances[0];
    assert_eq!(input.instances.len(), 1);
    assert_eq!(instance.mesh.indices.len(), 8);
    assert_eq!(instance.mesh.positions[0].y, -2.);
    assert_eq!(instance.world.0[12], 3.);
    assert!(
        instance.mesh.triangle_materials.contains(&0)
            && instance.mesh.triangle_materials.contains(&1)
    );
    let palette = instance.material_palette.as_ref().unwrap();
    assert_eq!(
        input.materials.as_ref().unwrap()[palette[1]].base,
        [1., 0., 0., 0.6]
    );
    let second = environment_input(doc);
    assert!(Arc::ptr_eq(&instance.mesh, &second.instances[0].mesh));
    let mut colored = object;
    colored["terrain"]["p"]["natural"] = json!(true);
    colored["terrain"]["p"]["blend"] = json!(0.2);
    colored["terrain"]["p"]["erosion"] = json!(4);
    let input = environment_input(environment_geometry_document(colored, json!({})));
    let instance = &input.instances[0];
    assert_eq!(instance.mesh.colors.len(), 9);
    assert_ne!(instance.mesh.positions[0].y, -2.);
    assert_eq!(
        input.materials.as_ref().unwrap()[instance.material].policy[3],
        1.
    );
    let extracted = pm_studio3d_engine::extract::Extractor::default()
        .extract(&input)
        .unwrap();
    assert_ne!(
        extracted.prepared.materials[instance.material].policy[1] as u32 & 16,
        0
    );
}
#[test]
fn terrain_renders_in_native_viewer_and_final_without_a_proxy_plane() {
    let doc = environment_geometry_document(
        json!({"source":{"primitive":"plane"},"terrain":{"p":{"width":2,"depth":2,"resolution":2,"amplitude":0}}}),
        json!({"rotation.x":90}),
    );
    let mut renderer = Renderer::headless().unwrap();
    let final_pixels = draw(&mut renderer, doc.clone());
    assert!(final_pixels.chunks_exact(4).filter(|p| p[3] > 0.).count() > 500);
    let mut live = request();
    live.export = false;
    live.output_kind = OutputKind::Preview;
    let frame = renderer
        .render(
            &doc,
            "test",
            Ticks::ZERO,
            &live,
            &pm_render::MapResolver::default(),
        )
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    assert!(
        renderer
            .read_rgba(&frame)
            .unwrap()
            .chunks_exact(4)
            .filter(|p| p[3] > 0.)
            .count()
            > 500
    );
}
#[test]
fn all_six_nature_species_use_shared_seeded_geometry_and_local_materials() {
    for species in ["conifer", "broadleaf", "birch", "bush", "grass", "rock"] {
        let input = environment_input(environment_geometry_document(
            json!({"source":{"nature":species,"seed":41}}),
            json!({}),
        ));
        assert_eq!(input.instances.len(), 1);
        let mesh = &input.instances[0].mesh;
        assert!(!mesh.indices.is_empty(), "{species}");
        assert!(mesh.triangle_materials.iter().all(|i| *i < 2));
        assert_eq!(
            input.instances[0].material_palette.as_ref().unwrap().len(),
            2
        );
        pm_studio3d_engine::extract::Extractor::default()
            .extract(&input)
            .unwrap();
    }
}

#[test]
fn saved_quality_and_export_override_drive_samples_without_mutating_the_document() {
    use pm_studio3d_engine::compositor::Source;
    use studio::SceneSource;
    let old = document(true, 100.);
    let mut project = pm_scene3d::from_model(&old.root().unwrap().to_value()).unwrap();
    let saved = pm_scene3d::quality::resolve(
        &pm_scene3d::quality::preset("production").unwrap(),
        &json!({"minSamples":8,"maxSamples":8,"noiseThreshold":0,"maxBounces":2,
            "denoise":false,"caustics":false,"timeLimitSeconds":0.000001}),
    )
    .unwrap();
    project["comps"]["test"]["studioQuality"] = saved.clone();
    let doc = Arc::new(
        pm_kernel::DocumentStore::new_untitled(pm_scene3d::to_model(&project))
            .snapshot()
            .document
            .as_ref()
            .clone(),
    );
    let before = doc.to_value();
    let req = studio::SceneRequest {
        composition: "test".into(),
        revision: doc.revision(),
        ticks: 0,
        width: 64,
        height: 64,
    };
    let source = studio::DocumentSource::new(doc.clone(), Default::default());
    assert_eq!(source.final_samples(&req).unwrap(), 8);
    let q = source.final_quality(&req).unwrap().unwrap();
    assert_eq!(q.max_bounces, 2);
    assert!(!q.denoise && !q.caustics && q.half_resolution_guides);
    let overridden = studio::DocumentSource::new(doc.clone(), Default::default())
        .with_quality_override("test", json!({"maxSamples":4,"minSamples":4}))
        .unwrap();
    assert_eq!(overridden.final_samples(&req).unwrap(), 4);
    assert!(
        studio::DocumentSource::new(doc.clone(), Default::default())
            .with_quality_override("test", json!({"minSamples":20}))
            .is_err()
    );
    let final_source = studio::DocumentSource::new(doc.clone(), Default::default())
        .with_quality_override("test", json!("final"))
        .unwrap();
    assert!(
        !final_source
            .final_quality(&req)
            .unwrap()
            .unwrap()
            .half_resolution_guides
    );
    let mut renderer = Renderer::headless().unwrap();
    draw(&mut renderer, doc.clone());
    assert_eq!(
        renderer.studio_counters().samples,
        8,
        "a tiny saved preview time cap must not truncate the delivery"
    );
    studio::attach_with_quality(
        &mut renderer,
        doc.clone(),
        &Default::default(),
        "test",
        json!({"minSamples":4,"maxSamples":4}),
    )
    .unwrap();
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
    assert_eq!(renderer.studio_counters().samples, 4);
    assert_eq!(doc.to_value(), before);
}

#[test]
fn metal_persisted_fbx_uses_source_materials_and_windowless_export() {
    let original =
        include_bytes!("../../../../../engine/crates/pm-import-fbx/fixtures/tiny-static.fbx");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("retained.bin");
    std::fs::write(&path, original).unwrap();
    let old = imported_document();
    let mut project = pm_scene3d::from_model(&old.to_value().unwrap()).unwrap();
    project["assets"]["imported"]["format"] = json!("fbx");
    for layer in project["comps"]["test"]["layers"].as_array_mut().unwrap() {
        if layer["id"] == "camera" {
            layer["d"]["data"]["environment"]["p"]["ambient"]["v"] = json!(1);
        }
    }
    let doc = pm_kernel::DocumentStore::new_untitled(pm_scene3d::to_model(&project))
        .snapshot()
        .document;
    let mut media = pm_export::render::MediaPaths::default();
    media.paths.insert("imported".into(), path);
    encoded_repeat_with_witness(doc, media, |pixels| {
        assert!(
            pixels
                .chunks_exact(4)
                .filter(|p| p[3] > 20 && p[0] > p[1] && p[1] > p[2])
                .count()
                > 10,
            "FBX Coral material must reach the native compositor and exported image"
        );
    });
}

fn environment_document(visible: bool, model: bool) -> Arc<pm_model::Document> {
    let old = document(false, 100.);
    let mut project = pm_scene3d::from_model(&old.to_value().unwrap()).unwrap();
    let comp = &project["comps"]["test"];
    let world = pm_scene3d::service::new_layer(comp,"world","World",pm_scene3d::schema::Role::Object,
        json!({"object":{"source":{"primitive":"plane"},"world":{"mode":"sky","p":{"visible":visible,"sunIntensity":0}}}}),json!({}),true,None,0.).unwrap();
    let fog = pm_scene3d::service::new_layer(comp,"fog","Fog",pm_scene3d::schema::Role::Object,
        json!({"object":{"source":{"primitive":"plane"},"fog":{"p":{"width":2,"height":4,"depth":6,"density":0.01}}}}),
        json!({"rotation.y":90,"scale.x":200}),true,None,0.).unwrap();
    let layers = project["comps"]["test"]["layers"].as_array_mut().unwrap();
    if !model {
        layers.retain(|l| l["id"] != "mirror");
    }
    layers.extend([world, fog]);
    pm_kernel::DocumentStore::new_untitled(pm_scene3d::to_model(&project))
        .snapshot()
        .document
}

#[test]
fn world_and_transformed_fog_reach_transport_without_proxy_surfaces() {
    use studio::SceneSource;
    let doc = environment_document(true, true);
    let req = studio::SceneRequest {
        composition: "test".into(),
        revision: doc.revision(),
        ticks: 0,
        width: 64,
        height: 64,
    };
    let source = studio::DocumentSource::new(doc, Default::default());
    let input = source.scene(&req).unwrap();
    assert_eq!(
        input.instances.len(),
        1,
        "World and Fog have no proxy planes"
    );
    assert_eq!(input.scene.world.sky.unwrap().sun_intensity, 0.);
    assert_eq!(input.scene.world.volumes.len(), 1);
    let size = input.scene.world.volumes[0].size;
    assert!(
        (size.x - 3.).abs() < 0.001 && (size.y - 2.).abs() < 0.001 && (size.z - 2.).abs() < 0.001,
        "{size:?}"
    );
    let outputs = source.auxiliary_layers(&req).unwrap();
    assert!(outputs.iter().any(|o| o.id == "world" && o.include_void));
    assert!(outputs.iter().any(|o| o.id == "fog" && !o.include_void));
}

#[test]
fn world_alone_renders_a_sky_and_hiding_the_sky_preserves_fog() {
    let mut renderer = Renderer::headless().unwrap();
    let visible = draw(&mut renderer, environment_document(true, false));
    assert!(
        visible
            .chunks_exact(4)
            .filter(|p| p[3] > 0.9 && p[2] > 0.01)
            .count()
            > 4000,
        "a World-only composition must render its sky"
    );
    let doc = environment_document(true, false);
    studio::attach(&mut renderer, doc.clone(), &Default::default());
    let mut live = request();
    live.export = false;
    live.output_kind = OutputKind::Preview;
    let frame = renderer
        .render(
            &doc,
            "test",
            Ticks::ZERO,
            &live,
            &pm_render::MapResolver::default(),
        )
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    assert!(
        renderer
            .read_rgba(&frame)
            .unwrap()
            .chunks_exact(4)
            .filter(|p| p[3] > 0.9 && p[2] > 0.01)
            .count()
            > 4000,
        "Live must present the World background through its GPU surface too"
    );
    // World.visible hides the sky from the camera; it does not disable the
    // independent Fog layer or the environment lighting that illuminates it.
    let fog = draw(&mut renderer, environment_document(false, false));
    assert!(fog.chunks_exact(4).any(|p| p[3] > 0.001 && p[0] > 0.001));
    let mut project =
        pm_scene3d::from_model(&environment_document(false, false).to_value().unwrap()).unwrap();
    for layer in project["comps"]["test"]["layers"].as_array_mut().unwrap() {
        if layer["id"] == "fog" {
            layer["on"] = json!(false);
        }
    }
    let hidden = draw(&mut renderer, project_document(&project));
    assert!(
        hidden.chunks_exact(4).all(|p| p[3] == 0.),
        "a camera-hidden sky with its Fog layer disabled is transparent"
    );
}

#[test]
fn world_edits_update_pixels_and_reuse_surface_geometry() {
    let first = environment_document(true, true);
    let mut project = pm_scene3d::from_model(&first.to_value().unwrap()).unwrap();
    for layer in project["comps"]["test"]["layers"].as_array_mut().unwrap() {
        if layer["id"] == "world" {
            layer["d"]["data"]["object"]["world"]["p"]["overcast"]["v"] = json!(1);
        }
    }
    let cloudy = pm_kernel::DocumentStore::new_untitled(pm_scene3d::to_model(&project))
        .snapshot()
        .document;
    let mut renderer = Renderer::headless().unwrap();
    let a = draw(&mut renderer, first.clone());
    let before = renderer.studio_counters().geometry_uploads;
    let b = draw(&mut renderer, cloudy);
    assert_ne!(a, b, "updated sky must invalidate its cached trailer");
    assert_eq!(renderer.studio_counters().geometry_uploads, before);
    assert_eq!(
        draw(&mut renderer, first),
        a,
        "restored World is deterministic"
    );
}

#[test]
fn retained_hdr_environment_preserves_energy_and_renders_in_live_and_final() {
    use studio::SceneSource;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("retained-image.bin");
    std::fs::write(
        &path,
        b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 1 +X 1\n\x80\x40\x20\x83",
    )
    .unwrap();
    assert_eq!(studio::probe_environment(&path).unwrap(), [1, 1]);
    let old = environment_document(true, false);
    let mut project = pm_scene3d::from_model(&old.to_value().unwrap()).unwrap();
    project["assets"]["hdri"] =
        json!({"id":"hdri","kind":"image","name":"studio.hdr","format":"hdr"});
    let layers = project["comps"]["test"]["layers"].as_array_mut().unwrap();
    layers.retain(|l| l["id"] != "fog");
    for l in layers {
        if l["id"] == "world" {
            l["d"]["data"]["object"]["world"]["mode"] = json!("hdri");
            l["d"]["data"]["object"]["world"]["source"] = json!("hdri");
            l["d"]["data"]["object"]["world"]["p"]["intensity"]["v"] = json!(2);
        }
    }
    let doc = pm_kernel::DocumentStore::new_untitled(pm_scene3d::to_model(&project))
        .snapshot()
        .document;
    let mut media = pm_export::render::MediaPaths::default();
    media.paths.insert("hdri".into(), path);
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
    let map = input.scene.world.hdri.unwrap();
    assert_eq!(map.pixels[0].x, 4.);
    assert_eq!(map.intensity, 2.);
    let mut renderer = Renderer::headless().unwrap();
    studio::attach(&mut renderer, doc.clone(), &media);
    for export in [true, false] {
        let mut request = request();
        request.export = export;
        request.output_kind = if export {
            OutputKind::Final
        } else {
            OutputKind::Preview
        };
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
        assert!(
            renderer
                .read_rgba(&frame)
                .unwrap()
                .chunks_exact(4)
                .filter(|p| p[3] > 0.9 && p[0] > p[1] && p[1] > p[2])
                .count()
                > 4000,
            "retained HDRI must fill the World output in Live and Final"
        );
    }
}

#[test]
fn retained_height_assets_cache_decodes_and_invalidate_geometry_on_replacement() {
    use studio::SceneSource;
    let dir = tempfile::tempdir().unwrap();
    let paths = [
        dir.path().join("height-a.bin"),
        dir.path().join("height-b.bin"),
    ];
    for (path, pixel) in paths.iter().zip([[128, 64, 32, 128], [64, 32, 16, 128]]) {
        let mut bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 1 +X 1\n".to_vec();
        bytes.extend(pixel);
        std::fs::write(path, bytes).unwrap();
    }
    let old = environment_geometry_document(
        json!({"source":{"primitive":"plane"},"terrain":{"heightSource":"height","p":{"resolution":2,"amplitude":4}}}),
        json!({}),
    );
    let mut project = pm_scene3d::from_model(&old.to_value().unwrap()).unwrap();
    project["assets"]["height"] =
        json!({"id":"height","kind":"image","name":"height.hdr","format":"hdr"});
    let doc = pm_kernel::DocumentStore::new_untitled(pm_scene3d::to_model(&project))
        .snapshot()
        .document;
    let request = studio::SceneRequest {
        composition: "test".into(),
        revision: doc.revision(),
        ticks: 0,
        width: 64,
        height: 64,
    };
    let source = |path| {
        let mut media = pm_export::render::MediaPaths::default();
        media.paths.insert("height".into(), path);
        studio::DocumentSource::new(doc.clone(), media)
    };
    let a = source(paths[0].clone());
    let first = a.scene(&request).unwrap();
    let repeat = a.scene(&request).unwrap();
    assert!(Arc::ptr_eq(
        &first.instances[0].mesh,
        &repeat.instances[0].mesh
    ));
    let expected = 4. * (0.2126 * 0.5 + 0.7152 * 0.25 + 0.0722 * 0.125);
    assert!((first.instances[0].mesh.positions[0].y - expected).abs() < 1e-6);
    let second = source(paths[1].clone()).scene(&request).unwrap();
    assert!(!Arc::ptr_eq(
        &first.instances[0].mesh,
        &second.instances[0].mesh
    ));
    assert!((second.instances[0].mesh.positions[0].y - expected * 0.5).abs() < 1e-6);
}

fn scatter_document(nature: bool, density: f64, camera_z: f64) -> Arc<pm_model::Document> {
    let old = environment_geometry_document(
        json!({"source":{"primitive":"plane"},"terrain":{"p":{"width":4,"depth":4,"resolution":2,"amplitude":0}}}),
        json!({"position.y":1}),
    );
    let mut project = pm_scene3d::from_model(&old.to_value().unwrap()).unwrap();
    let comp = &project["comps"]["test"];
    let source = pm_scene3d::service::new_layer(comp, "prototype", "Prototype", pm_scene3d::schema::Role::Object,
        json!({"object":{"source":if nature {json!({"nature":"conifer","seed":41})} else {json!({"primitive":"sphere","parameters":{"radius":0.1,"segments":12}})},"material":{"p":{"color":"#FF0000","emissive":"#FF0000"}}}}),
        json!({"position.x":1000}), false, None, 0.).unwrap();
    let scatter = pm_scene3d::service::new_layer(comp, "scatter", "Scatter", pm_scene3d::schema::Role::Object,
        json!({"object":{"source":{"primitive":"plane"},"scatter":{"model":"prototype","surface":"terrain","p":{"count":100,"density":density,"seed":41,"scaleMin":1,"scaleMax":1,"rotationMin":0,"rotationMax":0}}}}),
        json!({}), true, None, 0.).unwrap();
    let layers = project["comps"]["test"]["layers"].as_array_mut().unwrap();
    for layer in layers.iter_mut() {
        if layer["id"] == "terrain" {
            layer["on"] = json!(false);
        }
        if layer["id"] == "camera" {
            layer["p"]["position.z"]["v"] = json!(camera_z);
        }
    }
    layers.extend([source, scatter]);
    pm_kernel::DocumentStore::new_untitled(pm_scene3d::to_model(&project))
        .snapshot()
        .document
}
#[test]
fn scatter_hidden_resources_produce_repeatable_shared_world_instances() {
    let doc = scatter_document(false, 1., 5.);
    let input = environment_input(doc.clone());
    assert_eq!(input.instances.len(), 100);
    assert!(input.instances.iter().all(|i| i.layer == "scatter"
        && (i.world.0[13] - 1.).abs() < 1e-6
        && i.world.0[12].abs() <= 2.
        && i.world.0[14].abs() <= 2.));
    assert!(
        input
            .instances
            .iter()
            .all(|i| Arc::ptr_eq(&i.mesh, &input.instances[0].mesh))
    );
    let second = environment_input(doc.clone());
    assert!(
        input
            .instances
            .iter()
            .zip(&second.instances)
            .all(|(a, b)| a.world.0 == b.world.0)
    );
    let mut extractor = pm_studio3d_engine::extract::Extractor::default();
    let extracted = extractor.extract(&input).unwrap();
    assert_eq!(extractor.builds, 1);
    assert_eq!(
        extracted.prepared.tris.len(),
        input.instances[0].mesh.indices.len()
    );
    assert_eq!(extracted.layers, ["scatter"]);
    let mut renderer = Renderer::headless().unwrap();
    let pixels = draw(&mut renderer, doc);
    assert!(
        pixels
            .chunks_exact(4)
            .filter(|p| p[0] > 0. && p[3] > 0.)
            .count()
            > 5
    );
}
#[test]
fn scatter_nature_lod_changes_geometry_without_rerolling_placements() {
    let near = environment_input(scatter_document(true, 1., 5.));
    let far = environment_input(scatter_document(true, 1., 10000.));
    assert_eq!(near.instances.len(), 100);
    assert_eq!(far.instances.len(), 100);
    assert!(
        near.instances
            .iter()
            .zip(&far.instances)
            .all(|(a, b)| a.world.0 == b.world.0)
    );
    assert!(far.instances[0].mesh.indices.len() < near.instances[0].mesh.indices.len());
    let mut extractor = pm_studio3d_engine::extract::Extractor::default();
    extractor.extract(&far).unwrap();
    assert_eq!(extractor.builds, 1, "100 trees share one resident far LOD");
}
#[test]
fn empty_scatter_is_transparent_in_final_and_live() {
    let doc = scatter_document(false, 0., 5.);
    assert!(environment_input(doc.clone()).instances.is_empty());
    let mut renderer = Renderer::headless().unwrap();
    assert!(
        draw(&mut renderer, doc.clone())
            .chunks_exact(4)
            .all(|p| p[3] == 0.)
    );
    let mut live = request();
    live.export = false;
    live.output_kind = OutputKind::Preview;
    let frame = renderer
        .render(
            &doc,
            "test",
            Ticks::ZERO,
            &live,
            &pm_render::MapResolver::default(),
        )
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    assert!(
        renderer
            .read_rgba(&frame)
            .unwrap()
            .chunks_exact(4)
            .all(|p| p[3] == 0.)
    );
}

fn material_option_document(options: Value) -> Arc<pm_model::Document> {
    let old = document(false, 100.);
    let mut project = pm_scene3d::from_model(&old.to_value().unwrap()).unwrap();
    let object = &mut project["comps"]["test"]["layers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|l| l["id"] == "mirror")
        .unwrap()["d"]["data"]["object"];
    object["material"]["p"]["color"]["v"] = json!("#FF0000");
    object["materialOptions"] = json!({"p":options});
    pm_kernel::DocumentStore::new_untitled(pm_scene3d::to_model(&project))
        .snapshot()
        .document
}
#[test]
fn material_options_drive_unlit_shadow_and_reflection_shader_policy() {
    let doc = material_option_document(
        json!({"acceptsLights":false,"acceptsShadows":false,"castsShadows":"off","appearsInReflections":false}),
    );
    let input = environment_input(doc.clone());
    let material = &input.materials.as_ref().unwrap()[input.instances[0].material];
    assert_eq!(material.policy[1] as u32 & 43, 40);
    assert_eq!(material.emit[..3], [1., 0., 0.]);
    let mut renderer = Renderer::headless().unwrap();
    let pixels = draw(&mut renderer, doc);
    let visible: Vec<_> = pixels.chunks_exact(4).filter(|p| p[3] > 0.9).collect();
    eprintln!(
        "unlit visible {} sample {:?}",
        visible.len(),
        visible.first()
    );
    assert!(
        visible
            .iter()
            .filter(|p| p[0] > 0.1 && p[0] > 4. * p[1] && p[0] > 4. * p[2])
            .count()
            > 100
    );
}

#[test]
fn shadow_only_objects_keep_transport_geometry_and_hide_from_primary_rays() {
    let doc = material_option_document(json!({"castsShadows":"only","acceptsLights":false}));
    let input = environment_input(doc.clone());
    assert!(!input.instances[0].primary_visible);
    let extracted = pm_studio3d_engine::extract::Extractor::default()
        .extract(&input)
        .unwrap();
    let base = extracted.prepared.lights.len() * 4;
    assert_eq!(extracted.prepared.sdata[base + 5][3], 1.);
    assert_eq!(
        extracted.prepared.materials[input.instances[0].material].policy[1] as u32 & 1,
        1
    );
    assert!(!extracted.prepared.tris.is_empty());
    let mut renderer = Renderer::headless().unwrap();
    assert!(draw(&mut renderer, doc).chunks_exact(4).all(|p| p[3] == 0.));
}
#[test]
fn reflection_visibility_excludes_an_off_camera_emitter() {
    let old = document(true, 100.);
    let mut project = pm_scene3d::from_model(&old.to_value().unwrap()).unwrap();
    let card = project["comps"]["test"]["layers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|l| l["id"] == "card")
        .unwrap();
    card["d"]["data"]["object"]["materialOptions"] = json!({"p":{"appearsInReflections":false}});
    let doc = pm_kernel::DocumentStore::new_untitled(pm_scene3d::to_model(&project))
        .snapshot()
        .document;
    let mut renderer = Renderer::headless().unwrap();
    assert!(
        draw(&mut renderer, old)
            .chunks_exact(4)
            .filter(|p| p[0] > 0.08)
            .count()
            > 100
    );
    assert!(
        draw(&mut renderer, doc)
            .chunks_exact(4)
            .all(|p| p[0] == 0. && p[1] == 0. && p[2] == 0.)
    );
}
#[test]
fn scatter_material_options_bind_per_owner_and_preserve_shared_palettes() {
    let old = scatter_document(true, 1., 5.);
    let mut project = pm_scene3d::from_model(&old.to_value().unwrap()).unwrap();
    let scatter = project["comps"]["test"]["layers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|l| l["id"] == "scatter")
        .unwrap();
    scatter["d"]["data"]["object"]["materialOptions"] =
        json!({"p":{"castsShadows":"only","acceptsLights":false,"acceptsShadows":false}});
    let doc = pm_kernel::DocumentStore::new_untitled(pm_scene3d::to_model(&project))
        .snapshot()
        .document;
    let input = environment_input(doc);
    let first = &input.instances[0];
    assert!(input.instances.iter().all(|i| !i.primary_visible
        && i.material == first.material
        && Arc::ptr_eq(
            i.material_palette.as_ref().unwrap(),
            first.material_palette.as_ref().unwrap()
        )));
    assert_eq!(
        input.materials.as_ref().unwrap()[first.material].policy[1] as u32 & 11,
        9
    );
    assert!(
        input.materials.as_ref().unwrap().len() <= 5,
        "100 trees share material records"
    );
}

fn camera_patch(
    doc: Arc<pm_model::Document>,
    projection: &str,
    p: Value,
    z: f64,
) -> Arc<pm_model::Document> {
    let mut project = pm_scene3d::from_model(&doc.to_value().unwrap()).unwrap();
    let camera = project["comps"]["test"]["layers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|l| l["id"] == "camera")
        .unwrap();
    camera["p"]["position.z"]["v"] = json!(z);
    let c = &mut camera["d"]["data"]["camera"];
    c["projection"] = json!(projection);
    for (key, value) in p.as_object().unwrap() {
        c["p"][key] = json!({"v":value,"kf":[],"expr":null});
    }
    pm_kernel::DocumentStore::new_untitled(pm_scene3d::to_model(&project))
        .snapshot()
        .document
}
#[test]
fn native_orthographic_camera_preserves_scale_when_dollied() {
    let base = material_option_document(json!({"acceptsLights":false}));
    let near = camera_patch(base.clone(), "orthographic", json!({"zoom":1}), 5.);
    let far = camera_patch(base, "orthographic", json!({"zoom":1}), 10.);
    assert_eq!(
        environment_input(near.clone()).view.orthographic_span,
        Some(3.)
    );
    let mut renderer = Renderer::headless().unwrap();
    let covered = |pixels: Vec<f32>| pixels.chunks_exact(4).filter(|p| p[3] > 0.9).count();
    let a = covered(draw(&mut renderer, near));
    let b = covered(draw(&mut renderer, far));
    assert!(a > 300);
    assert!(a.abs_diff(b) <= 4, "ortho coverage {a} vs {b}");
}
#[test]
fn native_iris_and_clip_channels_reach_shared_camera_transport() {
    let base = material_option_document(json!({"acceptsLights":false}));
    let lens = camera_patch(
        base.clone(),
        "perspective",
        json!({"fStop":0.3,"focusDistance":1,"irisShape":6,"irisRotation":30,"irisRoundness":0.2,"irisAspect":1.4}),
        5.,
    );
    let input = environment_input(lens.clone());
    assert_eq!(input.view.iris, [6., 30f32.to_radians(), 0.2, 1.4]);
    let circular = camera_patch(
        lens.clone(),
        "perspective",
        json!({"irisShape":0,"irisAspect":1}),
        5.,
    );
    let mut renderer = Renderer::headless().unwrap();
    assert_ne!(draw(&mut renderer, lens), draw(&mut renderer, circular));
    let clipped = camera_patch(base.clone(), "perspective", json!({"near":10,"far":20}), 5.);
    assert_eq!(
        environment_input(clipped.clone()).view.clip_planes,
        Some([10., 20.])
    );
    assert!(
        draw(&mut renderer, base)
            .chunks_exact(4)
            .any(|p| p[3] > 0.9)
    );
    assert!(
        draw(&mut renderer, clipped)
            .chunks_exact(4)
            .all(|p| p[3] == 0.)
    );
}

fn vertex_attribute_glb() -> Vec<u8> {
    let mut bin = vec![];
    let mut views = vec![];
    let mut accessors = vec![];
    for (values, kind) in [
        (vec![-1f32, -1., 0., 1., -1., 0., 0., 1., 0.], "VEC3"),
        (vec![0., 0., 1., 0., 0., 1., 0., 0., 1.], "VEC3"),
        (vec![0., 0., 0., 0., 0., 0.], "VEC2"),
        (vec![0., 1., 1., 1., 0., 0.], "VEC2"),
        (
            vec![1., 0., 0., 0.5, 1., 0., 0., 0.5, 1., 0., 0., 0.5],
            "VEC4",
        ),
    ] {
        let start = bin.len();
        for v in values {
            bin.extend(v.to_le_bytes());
        }
        views.push(json!({"buffer":0,"byteOffset":start,"byteLength":bin.len()-start}));
        accessors
            .push(json!({"bufferView":views.len()-1,"componentType":5126,"count":3,"type":kind}));
    }
    accessors[0]["min"] = json!([-1, -1, 0]);
    accessors[0]["max"] = json!([1, 1, 0]);
    let mut header = serde_json::to_vec(&json!({"asset":{"version":"2.0"},"extensionsUsed":["KHR_materials_unlit"],"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],"buffers":[{"byteLength":bin.len()}],"bufferViews":views,"accessors":accessors,"materials":[{"pbrMetallicRoughness":{"baseColorFactor":[1,1,1,1]},"extensions":{"KHR_materials_unlit":{}},"doubleSided":true,"alphaMode":"BLEND"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1,"TEXCOORD_0":2,"TEXCOORD_1":3,"COLOR_0":4},"material":0}]}]})).unwrap();
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
#[test]
fn imported_secondary_uvs_vertex_alpha_and_snapshot_geometry_identity_survive() {
    use studio::SceneSource;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("attributes.bin");
    std::fs::write(&path, vertex_attribute_glb()).unwrap();
    let mut media = pm_export::render::MediaPaths::default();
    media.paths.insert("imported".into(), path);
    let doc = imported_document();
    let scene_request = studio::SceneRequest {
        composition: "test".into(),
        revision: doc.revision(),
        ticks: 0,
        width: 64,
        height: 64,
    };
    let source = || studio::DocumentSource::new(doc.clone(), media.clone());
    let input = source().scene(&scene_request).unwrap();
    let material = input.instances[0].material;
    let mut extractor = pm_studio3d_engine::extract::Extractor::default();
    let extracted = extractor.extract(&input).unwrap();
    let triangle = extracted
        .prepared
        .tris
        .iter()
        .find(|t| t.uv2m[2] as usize == material)
        .unwrap();
    assert_eq!(triangle.uv01_secondary, [0., 1., 1., 1.]);
    assert_eq!(triangle.uv2_secondary[..2], [0., 0.]);
    assert_eq!(triangle.n[0][3].to_bits(), 0x800000ff);
    assert_ne!(
        extracted.prepared.materials[material].policy[1] as u32 & 16,
        0
    );
    let builds = extractor.builds;
    extractor
        .extract(&source().scene(&scene_request).unwrap())
        .unwrap();
    assert_eq!(
        extractor.builds, builds,
        "exact vertex data survives source replacement"
    );
    let mut renderer = Renderer::headless().unwrap();
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
            .filter(|p| p[0] > p[1] && (0.49..0.51).contains(&p[3]))
            .count()
            > 100
    );
}

fn physical_map_glb() -> Vec<u8> {
    let original = vertex_attribute_glb();
    let json_len = u32::from_le_bytes(original[12..16].try_into().unwrap()) as usize;
    let mut header: Value = serde_json::from_slice(&original[20..20 + json_len]).unwrap();
    let mut bin = original[28 + json_len..].to_vec();
    let image = include_bytes!("../../../../../engine/fixtures/still-red.png");
    let image_view = header["bufferViews"].as_array().unwrap().len();
    header["bufferViews"]
        .as_array_mut()
        .unwrap()
        .push(json!({"buffer":0,"byteOffset":bin.len(),"byteLength":image.len()}));
    bin.extend(image);
    header["buffers"][0]["byteLength"] = json!(bin.len());
    header["images"] = json!([{"bufferView":image_view,"mimeType":"image/png"}]);
    header["samplers"] = json!([{"wrapS":33071,"wrapT":33648},{"wrapS":10497,"wrapT":10497}]);
    header["textures"] = json!([{"source":0,"sampler":0},{"source":0,"sampler":1}]);
    let texture = json!({"index":0});
    header["materials"] = json!([{"doubleSided":true,"alphaMode":"MASK","alphaCutoff":0.3,"pbrMetallicRoughness":{"baseColorFactor":[1,1,1,1],"metallicFactor":0.2,"roughnessFactor":0.4,"baseColorTexture":{"index":0,"extensions":{"KHR_texture_transform":{"offset":[0.125,0.375],"scale":[2,3],"texCoord":1}}},"metallicRoughnessTexture":{"index":1}},"normalTexture":{"index":0,"scale":0.7},"occlusionTexture":{"index":0,"strength":0.8},"emissiveFactor":[1,0,0],"emissiveTexture":texture,"extensions":{"KHR_materials_clearcoat":{"clearcoatFactor":0.2,"clearcoatRoughnessFactor":0.3,"clearcoatTexture":texture,"clearcoatRoughnessTexture":texture,"clearcoatNormalTexture":texture},"KHR_materials_transmission":{"transmissionFactor":0.4,"transmissionTexture":texture},"KHR_materials_volume":{"thicknessFactor":0.2,"thicknessTexture":texture,"attenuationDistance":2,"attenuationColor":[0.3,0.5,0.7]},"KHR_materials_specular":{"specularFactor":0.6,"specularColorFactor":[0.7,0.8,0.9],"specularTexture":texture,"specularColorTexture":texture},"KHR_materials_sheen":{"sheenColorFactor":[0.1,0.2,0.3],"sheenRoughnessFactor":0.5,"sheenColorTexture":texture,"sheenRoughnessTexture":texture},"KHR_materials_iridescence":{"iridescenceFactor":0.6,"iridescenceIor":1.4,"iridescenceThicknessMinimum":120,"iridescenceThicknessMaximum":450,"iridescenceTexture":texture,"iridescenceThicknessTexture":texture},"KHR_materials_anisotropy":{"anisotropyStrength":0.2,"anisotropyRotation":0.3,"anisotropyTexture":texture},"KHR_materials_dispersion":{"dispersion":0.4}}}]);
    header["extensionsUsed"] = json!([
        "KHR_texture_transform",
        "KHR_materials_clearcoat",
        "KHR_materials_transmission",
        "KHR_materials_volume",
        "KHR_materials_specular",
        "KHR_materials_sheen",
        "KHR_materials_iridescence",
        "KHR_materials_anisotropy",
        "KHR_materials_dispersion"
    ]);
    let mut encoded = serde_json::to_vec(&header).unwrap();
    while !encoded.len().is_multiple_of(4) {
        encoded.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let mut out = vec![];
    for word in [
        0x46546c67u32,
        2,
        (12 + 8 + encoded.len() + 8 + bin.len()) as u32,
        encoded.len() as u32,
        0x4e4f534a,
    ] {
        out.extend(word.to_le_bytes());
    }
    out.extend(encoded);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(0x004e4942u32.to_le_bytes());
    out.extend(bin);
    out
}
#[test]
fn imported_physical_maps_keep_uv_sampling_optics_and_shared_pixels() {
    use studio::SceneSource;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("physical.bin");
    std::fs::write(&path, physical_map_glb()).unwrap();
    let mut media = pm_export::render::MediaPaths::default();
    media.paths.insert("imported".into(), path);
    let doc = imported_document();
    let source = studio::DocumentSource::new(doc.clone(), media.clone());
    let scene_request = studio::SceneRequest {
        composition: "test".into(),
        revision: doc.revision(),
        ticks: 0,
        width: 64,
        height: 64,
    };
    let input = source.scene(&scene_request).unwrap();
    assert_eq!(
        input.maps.as_ref().unwrap().layers.len(),
        1,
        "two samplers share one image layer"
    );
    let m = &input.materials.as_ref().unwrap()[input.instances[0].material];
    assert!(m.physical_maps.iter().all(|map| map.info[0] == 0.));
    assert_eq!(m.physical_maps[0].info, [0., 1., 1., 2.]);
    assert_eq!(m.physical_maps[0].u, [2., 0., 0.125, 0.]);
    assert_eq!(m.physical_maps[0].v, [0., 3., 0.375, 0.]);
    assert_eq!(m.physical_maps[2].info, [0., 0., 0., 0.]);
    assert_eq!(m.imported[2], [0.3, 0.5, 0.7, 2.]);
    assert_eq!(m.imported[3], [0.2, 0.3, 1.4, 120.]);
    assert_eq!(m.layers[2], 0.4);
    assert_eq!(m.extra[3], 0.2);
    assert_eq!(m.policy[2], 0.3);
    let mut renderer = Renderer::headless().unwrap();
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
    assert!(
        renderer
            .read_rgba(&frame)
            .unwrap()
            .chunks_exact(4)
            .any(|p| p[0] > 0. && p[3] > 0.)
    );
}

fn mapped_document(source: Value) -> Arc<pm_model::Document> {
    let old = environment_geometry_document(
        json!({"source":{"primitive":"plane","parameters":{"width":2,"height":2}},"material":{"p":{"color":"#FFFFFF"},"maps":{"color":source,"emissive":"paint","roughness":"paint","metalness":"paint","normal":"paint","ao":"paint"}},"materialOptions":{"p":{"acceptsLights":false}}}),
        json!({}),
    );
    let mut project = pm_scene3d::from_model(&old.to_value().unwrap()).unwrap();
    project["assets"]["paint"] = json!({"id":"paint","kind":"image","format":"png","name":"Paint"});
    pm_kernel::DocumentStore::new_untitled(pm_scene3d::to_model(&project))
        .snapshot()
        .document
}
#[test]
fn indexed_image_replacements_preserve_imported_sampling_and_physical_maps() {
    use studio::SceneSource;
    let dir = tempfile::tempdir().unwrap();
    let model = dir.path().join("physical.bin");
    let image = dir.path().join("paint.bin");
    std::fs::write(&model, physical_map_glb()).unwrap();
    std::fs::write(
        &image,
        include_bytes!("../../../../../engine/fixtures/still-red.png"),
    )
    .unwrap();
    let mut media = pm_export::render::MediaPaths::default();
    media.paths.insert("imported".into(), model);
    media.paths.insert("paint".into(), image);
    let original = imported_document();
    let request = studio::SceneRequest {
        composition: "test".into(),
        revision: original.revision(),
        ticks: 0,
        width: 64,
        height: 64,
    };
    let source = studio::DocumentSource::new(original.clone(), media.clone());
    let before = source.scene(&request).unwrap();
    let original_material = before.materials.as_ref().unwrap()[before.instances[0].material];
    let mut project = pm_scene3d::from_model(&original.to_value().unwrap()).unwrap();
    let layer = project["comps"]["test"]["layers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|l| l["id"] == "mirror")
        .unwrap();
    // A valid indexed replacement wins before an unresolved whole-model source is loaded.
    layer["d"]["data"]["object"]["material"]["maps"] = json!({"color":"missing"});
    layer["d"]["data"]["object"]["sourceMaterials"] = json!({"0":{"color":"paint","emissive":"paint","normal":"paint","roughness":"paint","metalness":"paint","ao":"paint"}});
    project["assets"]["paint"] = json!({"id":"paint","kind":"image","format":"png","name":"Paint"});
    let doc = pm_kernel::DocumentStore::new_untitled(pm_scene3d::to_model(&project))
        .snapshot()
        .document;
    let source = studio::DocumentSource::new(doc.clone(), media.clone());
    let input = source
        .scene(&studio::SceneRequest {
            revision: doc.revision(),
            ..request
        })
        .unwrap();
    let m = &input.materials.as_ref().unwrap()[input.instances[0].material];
    assert_eq!(input.maps.as_ref().unwrap().layers.len(), 2);
    for slot in 0..6 {
        assert_eq!(m.physical_maps[slot].info[0], 1.);
        assert_eq!(
            &m.physical_maps[slot].info[1..],
            &original_material.physical_maps[slot].info[1..]
        );
        assert_eq!(
            m.physical_maps[slot].u,
            original_material.physical_maps[slot].u
        );
        assert_eq!(
            m.physical_maps[slot].v,
            original_material.physical_maps[slot].v
        );
    }
    for slot in 6..18 {
        assert_eq!(
            m.physical_maps[slot].info,
            original_material.physical_maps[slot].info
        );
        assert_eq!(
            m.physical_maps[slot].u,
            original_material.physical_maps[slot].u
        );
        assert_eq!(
            m.physical_maps[slot].v,
            original_material.physical_maps[slot].v
        );
    }
    assert_eq!(&m.base[..3], &[1.; 3]);
    assert_eq!(&m.emit[..3], &[1.; 3]);
    let mut renderer = Renderer::headless().unwrap();
    studio::attach(&mut renderer, doc.clone(), &media);
    let frame = renderer
        .render(&doc, "test", Ticks::ZERO, &crate::request(), &media)
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    assert!(
        renderer
            .read_rgba(&frame)
            .unwrap()
            .chunks_exact(4)
            .any(|p| p[0] > 0. && p[3] > 0.)
    );
}
#[test]
fn authored_image_maps_render_from_retained_assets_and_share_one_image_layer() {
    use studio::SceneSource;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("paint.bin");
    std::fs::write(
        &path,
        include_bytes!("../../../../../engine/fixtures/still-red.png"),
    )
    .unwrap();
    let mut media = pm_export::render::MediaPaths::default();
    media.paths.insert("paint".into(), path);
    let doc = mapped_document(json!("paint"));
    let source = studio::DocumentSource::new(doc.clone(), media.clone());
    let scene_request = studio::SceneRequest {
        composition: "test".into(),
        revision: doc.revision(),
        ticks: 0,
        width: 64,
        height: 64,
    };
    let input = source.scene(&scene_request).unwrap();
    assert_eq!(input.maps.as_ref().unwrap().layers.len(), 1);
    let material = &input.materials.as_ref().unwrap()[input.instances[0].material];
    assert_eq!(material.maps, [0., 0., -1., 0.]);
    assert_eq!(material.maps2, [0., 0., 0., 1.]);
    let mut renderer = Renderer::headless().unwrap();
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
    assert!(
        renderer
            .read_rgba(&frame)
            .unwrap()
            .chunks_exact(4)
            .filter(|p| p[0] > 0.1 && p[0] > 4. * p[1] && p[3] > 0.9)
            .count()
            > 100
    );
    let source = studio::DocumentSource::new(doc, Default::default());
    source.scene(&scene_request).unwrap();
    let missing = source.layer_failures(&scene_request).unwrap()[0]
        .message
        .clone();
    assert!(
        missing.contains("Material image is not loaded"),
        "{missing}"
    );
    let live = mapped_document(json!({"layerId":"missing-artwork"}));
    let live_request = studio::SceneRequest {
        revision: live.revision(),
        ..scene_request
    };
    let source = studio::DocumentSource::new(live, media);
    source.scene(&live_request).unwrap();
    let error = source.layer_failures(&live_request).unwrap()[0]
        .message
        .clone();
    assert!(
        error.contains("Live layer/group material capture"),
        "{error}"
    );
}

#[test]
fn absent_saved_quality_uses_source_production_defaults_without_mutating_legacy_data() {
    use studio::SceneSource;
    let old = document(false, 100.);
    let mut legacy = pm_scene3d::from_model(&old.to_value().unwrap()).unwrap();
    legacy["comps"]["test"]
        .as_object_mut()
        .unwrap()
        .remove("studioQuality");
    let doc = pm_kernel::DocumentStore::new_untitled(pm_scene3d::to_model(&legacy))
        .snapshot()
        .document;
    let before = doc.to_value();
    let request = studio::SceneRequest {
        composition: "test".into(),
        revision: doc.revision(),
        ticks: 0,
        width: 64,
        height: 64,
    };
    let source = studio::DocumentSource::new(doc.clone(), Default::default());
    let quality = source
        .final_quality(&request)
        .unwrap()
        .expect("source default is Production");
    assert_eq!(
        (
            quality.min_samples,
            quality.max_samples,
            quality.max_bounces
        ),
        (8, 128, 8)
    );
    assert_eq!(quality.noise_threshold, 0.12);
    assert!(quality.denoise && quality.caustics);
    assert_eq!(doc.to_value(), before);
}

fn extrusion_test_fonts() -> Box<dyn pm_render::text::TextFonts> {
    use pm_text::font::*;
    struct Face(FaceInfo);
    impl FontResolver for Face {
        fn face(&self, _: FaceId) -> &FaceInfo {
            &self.0
        }
        fn family(&mut self, _: &str, _: &FontDesc) -> Option<FaceId> {
            Some(0)
        }
        fn standard(&mut self, _: &FontDesc) -> Option<FaceId> {
            Some(0)
        }
        fn fallback_for_char(
            &mut self,
            _: &FontDesc,
            _: u32,
            _: FaceId,
            _: bool,
        ) -> Option<FaceId> {
            None
        }
        fn last_resort(&mut self, _: &FontDesc) -> Option<FaceId> {
            Some(0)
        }
        fn variable_face(&mut self, _: &str) -> Option<FaceId> {
            Some(0)
        }
    }
    let data = FontBytes::new(
        include_bytes!("../../../../../engine/crates/pm-text/tests/fixtures/Geist-Variable.ttf")
            .to_vec(),
    );
    let facts = FaceFacts::of(data.as_slice(), 0);
    let face = FaceInfo {
        data,
        index: 0,
        blob: "Geist".into(),
        coords: vec![],
        synthetic_bold: false,
        synthetic_italic: false,
        custom: true,
        web: None,
        synthesized_hyphens: [None; 2],
        postscript_name: "Geist".into(),
        typeface_key: "Geist".into(),
        fallback_key: None,
        facts,
    };
    Box::new(pm_eval::text_measure::CompatMeasure::new(Face(face)))
}
#[test]
fn text_extrusion_reaches_native_geometry_four_surfaces_and_the_compositor() {
    use studio::SceneSource;
    let mut project: Value = serde_json::from_str(&pm_model::json::to_canonical_string(
        &document(false, 100.).to_value().unwrap(),
    ))
    .unwrap();
    let comp = &mut project["comps"]["test"];
    comp["layers"]
        .as_array_mut()
        .unwrap()
        .retain(|l| l["id"] != "mirror");
    let mut data = pm_scene3d::extrusion::parse(
        &json!({"enabled":true,"p":{"depth":10,"bevelDepth":2,"bevelStyle":"convex"}}),
    )
    .unwrap();
    for (i, surface) in pm_scene3d::extrusion::SURFACES.iter().enumerate() {
        data["surfaces"][surface] = json!({"maps":{},"properties":{"color":"#FF0000","emissionColor":"#FF0000","emissionIntensity":1,"roughness":0.2+i as f64*0.1}});
    }
    data = pm_scene3d::extrusion::parse(&data).unwrap();
    comp["layers"].as_array_mut().unwrap().push(json!({"id":"title","name":"Title","type":"text","on":true,"threeD":true,"from":0,"dur":1,"p":{"position.x":{"v":32,"kf":[]},"position.y":{"v":32,"kf":[]},"opacity":{"v":100,"kf":[]}},"d":{"text":"O","font":"Geist","size":30,"color":"#0000FF","extrusion":data}}));
    let doc = pm_kernel::DocumentStore::new_untitled(
        pm_model::json::parse(&project.to_string()).unwrap(),
    )
    .snapshot()
    .document;
    let source = studio::DocumentSource::new(doc.clone(), Default::default())
        .with_text_fonts(extrusion_test_fonts());
    let req = studio::SceneRequest {
        composition: "test".into(),
        revision: doc.revision(),
        ticks: 0,
        width: 64,
        height: 64,
    };
    let scene = source.scene(&req).unwrap();
    assert!(
        source.layer_failures(&req).unwrap().is_empty(),
        "{:?}",
        source.layer_failures(&req).unwrap()
    );
    let title = scene
        .instances
        .iter()
        .find(|i| i.layer == "title")
        .expect("extruded glyph");
    assert_eq!(title.material_palette.as_ref().unwrap().len(), 4);
    for index in 0..4 {
        assert!(
            title.mesh.triangle_materials.contains(&index),
            "surface {index}"
        );
    }
    assert!(title.mesh.positions.iter().any(|p| p.z == -10.));
    let again = source.scene(&req).unwrap();
    let other = again.instances.iter().find(|i| i.layer == "title").unwrap();
    assert!(Arc::ptr_eq(&title.mesh, &other.mesh));
    assert!(Arc::ptr_eq(
        title.attributes.as_ref().unwrap(),
        other.attributes.as_ref().unwrap()
    ));
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_text_fonts(Some(extrusion_test_fonts()));
    renderer.set_studio_source(
        Some(Arc::new(pm_studio3d_engine::compositor::Adapter(Arc::new(
            source,
        )))),
        pm_render::scene3d::provider::Settings {
            revision: doc.revision(),
            ..Default::default()
        },
    );
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
    let red = pixels
        .chunks_exact(4)
        .filter(|p| p[0] > 0.1 && p[0] > p[2] * 2. && p[3] > 0.5)
        .count();
    assert!(
        red > 30,
        "the extruded surface must render red, rather than the flat blue paint ({red} pixels)"
    );
}

#[test]
fn shape_extrusion_and_empty_text_use_the_shared_native_scene() {
    use studio::SceneSource;
    for empty in [false, true] {
        let mut project: Value = serde_json::from_str(&pm_model::json::to_canonical_string(
            &document(false, 100.).to_value().unwrap(),
        ))
        .unwrap();
        let comp = &mut project["comps"]["test"];
        comp["layers"]
            .as_array_mut()
            .unwrap()
            .retain(|l| l["id"] != "mirror");
        let mut content = if empty {
            json!({"text":" ","font":"Geist","size":30,"color":"#FFFFFF"})
        } else {
            json!({"shape":"rect","w":24,"h":20,"radius":0,"smoothing":0,"stroke":2,"strokeColor":"#00FF00","color":"#0000FF"})
        };
        content["extrusion"]=pm_scene3d::extrusion::parse(&json!({"enabled":true,"p":{"depth":8,"bevelStyle":"angular","bevelDepth":1},"surfaces":{"front":{"properties":{"emissionColor":"#FF0000","emissionIntensity":1}}}})).unwrap();
        comp["layers"].as_array_mut().unwrap().push(json!({"id":"art","name":"Art","type":if empty{"text"}else{"shape"},"on":true,"threeD":true,"from":0,"dur":1,"p":{"position.x":{"v":32,"kf":[]},"position.y":{"v":32,"kf":[]},"opacity":{"v":100,"kf":[]}},"d":content}));
        let doc = pm_kernel::DocumentStore::new_untitled(
            pm_model::json::parse(&project.to_string()).unwrap(),
        )
        .snapshot()
        .document;
        let source = studio::DocumentSource::new(doc.clone(), Default::default())
            .with_text_fonts(extrusion_test_fonts());
        let req = studio::SceneRequest {
            composition: "test".into(),
            revision: doc.revision(),
            ticks: 0,
            width: 64,
            height: 64,
        };
        let scene = source.scene(&req).unwrap();
        assert!(
            source.layer_failures(&req).unwrap().is_empty(),
            "{:?}",
            source.layer_failures(&req).unwrap()
        );
        assert_eq!(scene.instances.is_empty(), empty);
        let mut renderer = Renderer::headless().unwrap();
        renderer.set_text_fonts(Some(extrusion_test_fonts()));
        renderer.set_studio_source(
            Some(Arc::new(pm_studio3d_engine::compositor::Adapter(Arc::new(
                source,
            )))),
            pm_render::scene3d::provider::Settings {
                revision: doc.revision(),
                ..Default::default()
            },
        );
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
        if empty {
            assert!(pixels.chunks_exact(4).all(|p| p[3] < 0.001));
        } else {
            assert!(
                pixels
                    .chunks_exact(4)
                    .filter(|p| p[0] > 0.1 && p[3] > 0.5)
                    .count()
                    > 30
            );
        }
    }
}

#[test]
fn material_capture_uses_hidden_local_sources_and_stable_group_bounds() {
    use pm_render::MaterialCaptureRequest;
    let mut project: Value = serde_json::from_str(&pm_model::json::to_canonical_string(
        &document(false, 100.).to_value().unwrap(),
    ))
    .unwrap();
    let comp = &mut project["comps"]["test"];
    comp["fps"] = json!(4);
    comp["layers"].as_array_mut().unwrap().extend([
        json!({"id":"art","name":"Art","type":"group","on":false,"from":0,"dur":1,"p":{"position.x":{"v":5000,"kf":[]},"opacity":{"v":0,"kf":[]}}}),
        json!({"id":"paint","name":"Paint","type":"solid","group":"art","on":true,"from":0,"dur":1,"p":{"position.x":{"v":0,"kf":[{"t":0,"v":0},{"t":1,"v":40}]},"opacity":{"v":100,"kf":[]}},"d":{"w":16,"h":16,"color":"#FF0000"}}),
    ]);
    for layer in comp["layers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .filter(|l| l["id"] == "art" || l["id"] == "paint")
    {
        for (key, value) in [
            ("position.x", 0.),
            ("position.y", 0.),
            ("anchor.x", 0.),
            ("anchor.y", 0.),
            ("scale.x", 100.),
            ("scale.y", 100.),
            ("rotation", 0.),
            ("skew", 0.),
            ("opacity", 100.),
        ] {
            if layer["p"].get(key).is_none() {
                layer["p"][key] = json!({"v":value,"kf":[]});
            }
        }
    }
    let doc = pm_kernel::DocumentStore::new_untitled(
        pm_model::json::parse(&project.to_string()).unwrap(),
    )
    .snapshot()
    .document;
    let before = doc.to_value();
    let mut renderer = Renderer::headless().unwrap();
    let assets = pm_render::MapResolver::default();
    let mut req = MaterialCaptureRequest {
        composition: "test".into(),
        layer: Some("art".into()),
        time: Ticks::ZERO,
        width: 64,
        height: 32,
        source_key: "art-v1".into(),
        frame_key: "0".into(),
        cache_bytes: 128 * 1024 * 1024,
        final_output: false,
    };
    let a = renderer.capture_material(&doc, &req, &assets).unwrap();
    assert!(
        a.pixels
            .chunks_exact(4)
            .filter(|p| p[0] > 200 && p[3] > 200)
            .count()
            > 60,
        "hidden root must still supply its child's paint"
    );
    let cached = renderer.capture_material(&doc, &req, &assets).unwrap();
    assert!(Arc::ptr_eq(&a, &cached));
    assert_eq!(renderer.material_capture_stats().cache_hits, 1);
    req.time = Ticks::new(pm_protocol::common::TICKS_PER_SECOND).unwrap();
    req.frame_key = "1".into();
    let b = renderer.capture_material(&doc, &req, &assets).unwrap();
    let center = |p: &[u8]| {
        let mut sum = 0.;
        let mut count = 0.;
        for (i, p) in p.chunks_exact(4).enumerate() {
            if p[3] > 200 {
                sum += (i % 64) as f64;
                count += 1.;
            }
        }
        sum / count
    };
    assert!(
        center(&b.pixels) > center(&a.pixels) + 20.,
        "an animated group must move inside one fixed canvas"
    );
    req.cache_bytes = 0;
    renderer.capture_material(&doc, &req, &assets).unwrap();
    assert_eq!(renderer.material_capture_stats().retained_bytes, 0);
    assert_eq!(
        doc.to_value(),
        before,
        "capture must not edit the source document"
    );
    let captures = renderer.material_capture_stats().captures;
    req.width = 16384;
    req.height = 16384;
    assert!(
        renderer
            .capture_material(&doc, &req, &assets)
            .unwrap_err()
            .to_string()
            .contains("pixel and memory budget")
    );
    assert_eq!(renderer.material_capture_stats().captures, captures);
}

fn source_art(id: &str, kind: &str, group: Option<&str>, color: &str) -> Value {
    let mut layer = json!({"id":id,"name":id,"type":kind,"on":true,"from":0,"dur":1,"p":{},"d":{"w":32,"h":32,"color":color}});
    for (key, value) in [
        ("position.x", 0.),
        ("position.y", 0.),
        ("anchor.x", 0.),
        ("anchor.y", 0.),
        ("scale.x", 100.),
        ("scale.y", 100.),
        ("rotation", 0.),
        ("skew", 0.),
        ("opacity", 100.),
    ] {
        layer["p"][key] = json!({"v":value,"kf":[]});
    }
    if let Some(group) = group {
        layer["group"] = json!(group);
    }
    layer
}
fn captured_project(source: Value) -> Value {
    let doc = environment_geometry_document(
        json!({"source":{"primitive":"plane","parameters":{"width":2,"height":2}},"material":{"p":{"color":"#FFFFFF"},"maps":{"color":source}},"materialOptions":{"p":{"acceptsLights":false}}}),
        json!({}),
    );
    let mut project = pm_scene3d::from_model(&doc.to_value().unwrap()).unwrap();
    let mut group = source_art("art", "group", None, "#FFFFFF");
    group["on"] = json!(false);
    group["p"]["opacity"]["v"] = json!(0);
    group["p"]["position.x"]["v"] = json!(9000);
    project["comps"]["test"]["layers"]
        .as_array_mut()
        .unwrap()
        .extend([group, source_art("paint", "solid", Some("art"), "#FF0000")]);
    project
}
fn project_document(project: &Value) -> Arc<pm_model::Document> {
    pm_kernel::DocumentStore::new_untitled(pm_scene3d::to_model(project))
        .snapshot()
        .document
}
#[test]
fn material_capture_binds_hidden_groups_reuses_pixels_and_tracks_only_source_edits() {
    use pm_studio3d_engine::compositor::Source;
    use studio::SceneSource;
    let mut project = captured_project(json!({"layerId":"art"}));
    let doc = project_document(&project);
    let before = doc.to_value();
    let mut renderer = Renderer::headless().unwrap();
    let req = studio::SceneRequest {
        composition: "test".into(),
        revision: doc.revision(),
        ticks: 0,
        width: 64,
        height: 64,
    };
    let source = studio::DocumentSource::new(doc.clone(), Default::default());
    assert!(
        source
            .prepare_materials(&mut renderer, &req, studio::Tier::QualityLive)
            .unwrap()
    );
    let input = source.scene(&req).unwrap();
    assert!(source.layer_failures(&req).unwrap().is_empty());
    let pixels = &input.maps.as_ref().unwrap().layers[0];
    assert!(
        pixels
            .chunks_exact(4)
            .filter(|p| p[0] > 240 && p[1] < 10 && p[3] > 240)
            .count()
            > 900_000
    );
    assert_eq!(renderer.material_capture_stats().captures, 1);
    assert!(
        !source
            .prepare_materials(&mut renderer, &req, studio::Tier::QualityLive)
            .unwrap()
    );
    assert_eq!(renderer.material_capture_stats().captures, 1);
    // Receiver transforms, time and fresh immutable adapters do not redraw still artwork.
    project["comps"]["test"]["layers"][2]["p"]["position.x"]["v"] = json!(0.25);
    let moved = project_document(&project);
    let next = studio::DocumentSource::new(moved.clone(), Default::default());
    let next_req = studio::SceneRequest {
        revision: moved.revision(),
        ticks: pm_protocol::common::TICKS_PER_SECOND / 2,
        ..req.clone()
    };
    next.prepare_materials(&mut renderer, &next_req, studio::Tier::QualityLive)
        .unwrap();
    assert_eq!(renderer.material_capture_stats().captures, 1);
    project["comps"]["test"]["layers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|l| l["id"] == "paint")
        .unwrap()["d"]["color"] = json!("#00FF00");
    let edited = project_document(&project);
    let next = studio::DocumentSource::new(edited.clone(), Default::default());
    let next_req = studio::SceneRequest {
        revision: edited.revision(),
        ..next_req
    };
    next.prepare_materials(&mut renderer, &next_req, studio::Tier::QualityLive)
        .unwrap();
    let input = next.scene(&next_req).unwrap();
    assert!(
        input.maps.as_ref().unwrap().layers[0]
            .chunks_exact(4)
            .any(|p| p[1] > 240 && p[0] < 10)
    );
    assert_eq!(renderer.material_capture_stats().captures, 2);
    assert_eq!(doc.to_value(), before);
    // The actual native compositor adapter prepares captures before tracing.
    studio::attach(&mut renderer, doc.clone(), &Default::default());
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
    assert!(
        renderer
            .read_rgba(&frame)
            .unwrap()
            .chunks_exact(4)
            .filter(|p| p[0] > 0.1 && p[0] > 4. * p[1] && p[3] > 0.9)
            .count()
            > 100
    );
}
#[test]
fn material_capture_rejects_cycles_nested_3d_and_missing_sources_per_receiver() {
    use pm_studio3d_engine::compositor::Source;
    use studio::SceneSource;
    let mut renderer = Renderer::headless().unwrap();
    for (source, case, expected) in [
        (json!({"layerId":"terrain"}), "cycle", "cycle"),
        (json!({"compositionId":"test"}), "whole", "cycle"),
        (json!({"layerId":"missing"}), "missing", "missing"),
        (json!({"layerId":"art"}), "nested", "3D descendants"),
    ] {
        let mut project = captured_project(source);
        if case == "nested" {
            let mut precomp = source_art("nested", "precomp", Some("art"), "#000000");
            precomp["d"] = json!({"comp":"other"});
            project["comps"]["test"]["layers"]
                .as_array_mut()
                .unwrap()
                .push(precomp);
            project["comps"]["other"] = project["comps"]["test"].clone();
            project["comps"]["other"]["id"] = json!("other");
            project["comps"]["other"]["layers"]
                .as_array_mut()
                .unwrap()
                .retain(|l| l["id"] == "terrain");
        }
        let doc = project_document(&project);
        let req = studio::SceneRequest {
            composition: "test".into(),
            revision: doc.revision(),
            ticks: 0,
            width: 64,
            height: 64,
        };
        let source = studio::DocumentSource::new(doc, Default::default());
        source
            .prepare_materials(&mut renderer, &req, studio::Tier::QualityLive)
            .unwrap();
        let input = source.scene(&req).unwrap();
        assert!(input.instances.is_empty(), "{case}");
        let failures = source.layer_failures(&req).unwrap();
        assert_eq!(failures.len(), 1, "{case}: {failures:?}");
        assert!(
            failures[0].message.contains(expected),
            "{case}: {failures:?}"
        );
    }
}
#[test]
fn material_capture_screen_clock_uses_each_receiver_and_static_source_edges() {
    use pm_studio3d_engine::compositor::Source;
    use studio::SceneSource;
    let mut project = captured_project(json!({"layerId":"art"}));
    let layers = project["comps"]["test"]["layers"].as_array_mut().unwrap();
    let index = layers.iter().position(|l| l["id"] == "terrain").unwrap();
    layers[index]["d"]["data"]["object"]["materialOptions"]["p"]["acceptsLights"] = json!(true);
    layers[index]["d"]["data"]["object"]["material"] =
        json!({"screen":"screen","p":{"color":"#FFFFFF"}});
    let mut second = layers[index].clone();
    second["id"] = json!("second");
    second["from"] = json!(0.5);
    layers.push(second);
    let mut red = source_art("red", "solid", None, "#FF0000");
    red["dur"] = json!(0.5);
    let mut green = source_art("green", "solid", None, "#00FF00");
    green["from"] = json!(0.5);
    green["dur"] = json!(0.5);
    project["comps"]["screen"] = json!({"id":"screen","w":32,"h":32,"fps":30,"dur":1,"backgroundFill":{"type":"none"},"layers":[red,green]});
    let doc = project_document(&project);
    let mut renderer = Renderer::headless().unwrap();
    let req = studio::SceneRequest {
        composition: "test".into(),
        revision: doc.revision(),
        ticks: pm_protocol::common::TICKS_PER_SECOND * 3 / 4,
        width: 64,
        height: 64,
    };
    let source = studio::DocumentSource::new(doc, Default::default());
    source
        .prepare_materials(&mut renderer, &req, studio::Tier::QualityLive)
        .unwrap();
    let input = source.scene(&req).unwrap();
    assert!(source.layer_failures(&req).unwrap().is_empty());
    let maps = &input.maps.as_ref().unwrap().layers;
    assert_eq!(maps.len(), 2);
    assert!(
        maps.iter()
            .any(|p| p.chunks_exact(4).any(|p| p[0] > 240 && p[1] < 10))
    );
    assert!(
        maps.iter()
            .any(|p| p.chunks_exact(4).any(|p| p[1] > 240 && p[0] < 10))
    );
    assert_eq!(renderer.material_capture_stats().captures, 2);
    let next = studio::SceneRequest {
        ticks: pm_protocol::common::TICKS_PER_SECOND * 7 / 8,
        ..req
    };
    source
        .prepare_materials(&mut renderer, &next, studio::Tier::QualityLive)
        .unwrap();
    assert_eq!(
        renderer.material_capture_stats().captures,
        2,
        "still source intervals reuse across receiver clocks"
    );
}

#[test]
fn material_capture_preserves_source_masks_effects_and_child_visibility() {
    for root in ["paint","art"] {
        let mut project=captured_project(json!({"layerId":"art"}));
        let layers=project["comps"]["test"]["layers"].as_array_mut().unwrap();
        let layer=layers.iter_mut().find(|l|l["id"]==root).unwrap();
        layer["on"]=json!(false);layer["p"]["opacity"]["v"]=json!(0);layer["p"]["position.x"]["v"]=json!(5000);
        // A child stays inside the source group's local canvas.
        if root=="art"{layer["p"]["position.x"]["v"]=json!(9000);}
        layer["masks"]=json!([{"id":"cut","shape":"rect","mode":"add","on":true,"p":{"x":{"v":8,"kf":[]},"y":{"v":16,"kf":[]},"w":{"v":16,"kf":[]},"h":{"v":32,"kf":[]},"rotation":{"v":0,"kf":[]},"feather":{"v":0,"kf":[]}}}]);
        layer["fx"]=json!([{"id":"invert","type":"invert","version":2,"on":true,"p":{}}]);
        if root=="art"{let mut hidden=source_art("hidden","solid",Some("art"),"#FFFFFF");hidden["on"]=json!(false);layers.insert(0,hidden);}
        let doc=project_document(&project);let mut renderer=Renderer::headless().unwrap();renderer.set_main_effects_for_document(&doc).unwrap();
        let frame=renderer.capture_material(&doc,&pm_render::MaterialCaptureRequest{composition:"test".into(),layer:Some(root.into()),time:Ticks::ZERO,width:32,height:32,source_key:root.into(),frame_key:"still".into(),cache_bytes:0,final_output:true},&pm_render::MapResolver::default()).unwrap();
        let left=&frame.pixels[(16*32+8)*4..][..4];let right=&frame.pixels[(16*32+24)*4..][..4];assert!(left[0]<10 && left[1]>240 && left[2]>240 && left[3]>240,"{root}: {left:?}");assert!(right[3]<10,"{root}: {right:?}");
    }
}

#[test]
fn studio_view_cameras_select_named_layers_and_all_six_orthographic_axes() {
    use studio::{Axis, SceneSource, ViewCamera};
    let original = document(false, 100.);
    let mut project = pm_scene3d::from_model(&original.to_value().unwrap()).unwrap();
    let layers = project["comps"]["test"]["layers"].as_array_mut().unwrap();
    let mut camera = layers.iter().find(|l| l["id"] == "camera").unwrap().clone();
    camera["id"] = json!("side");
    camera["on"] = json!(false);
    camera["p"]["position.x"]["v"] = json!(5);
    camera["p"]["position.z"]["v"] = json!(0);
    layers.push(camera);
    let doc = project_document(&project);
    let before = doc.to_value();
    let req = studio::SceneRequest {
        composition: "test".into(),
        revision: doc.revision(),
        ticks: 0,
        width: 64,
        height: 64,
    };
    let gpu = pm_gpu::Gpu::new("Studio view axes").unwrap();
    let mut service = studio::StudioService::with_device(gpu.device, gpu.queue, 64, 64).unwrap();
    for (axis, normal) in [
        (Axis::Front, [0., 0., 1.]),
        (Axis::Back, [0., 0., -1.]),
        (Axis::Left, [-1., 0., 0.]),
        (Axis::Right, [1., 0., 0.]),
        (Axis::Top, [0., 1., 0.]),
        (Axis::Bottom, [0., -1., 0.]),
    ] {
        let source = studio::DocumentSource::new(doc.clone(), Default::default()).with_camera_for(
            "test",
            ViewCamera::Orthographic {
                axis,
                center: [0.; 3],
                span: 4.,
            },
        );
        let input = source.scene(&req).unwrap();
        assert_eq!(input.view.orthographic_span, Some(2.));
        assert_eq!(input.scene.camera.f_stop, 0.);
        let hit = service
            .pick_surface(&source, req.clone(), studio::Tier::QualityLive, 0.5, 0.5)
            .unwrap()
            .unwrap();
        assert_eq!(hit.layer, "mirror");
        let dot: f32 = hit.normal.iter().zip(normal).map(|(a, b)| a * b).sum();
        assert!(dot > 0.9, "{axis:?}: {:?}", hit.normal);
    }
    let source = studio::DocumentSource::new(doc.clone(), Default::default())
        .with_camera_for("test", ViewCamera::Layer { id: "side".into() });
    let input = source.scene(&req).unwrap();
    assert!((input.scene.camera.position.x - 5.).abs() < 1e-5);
    let hit = service
        .pick_surface(&source, req.clone(), studio::Tier::QualityLive, 0.5, 0.5)
        .unwrap()
        .unwrap();
    assert!(hit.normal[0] > 0.9);
    let invalid = studio::DocumentSource::new(doc.clone(), Default::default()).with_camera_for(
        "test",
        ViewCamera::Layer {
            id: "missing".into(),
        },
    );
    assert!(
        invalid
            .scene(&req)
            .err()
            .unwrap()
            .contains("View camera layer is missing")
    );
    assert_eq!(doc.to_value(), before);
}
#[test]
fn four_worker_views_keep_their_camera_tier_and_pick_state_independent() {
    use pm_engine_service::{EngineService, Output, SurfaceQueryKind, ViewerConfig};
    use studio::{Axis, Preview, ViewCamera, ViewCommand, Views};
    let doc = document(false, 100.);
    let snapshot = pm_kernel::DocumentStore::new_untitled(doc.to_value().unwrap()).snapshot();
    let before = snapshot.document.to_value();
    let service = EngineService::start(pm_gpu::Gpu::new("four Studio views").unwrap()).unwrap();
    let viewers: Vec<_> = (0..4)
        .map(|_| {
            service
                .viewer(ViewerConfig {
                    snapshot: snapshot.clone(),
                    composition: "test".into(),
                    media: Default::default(),
                    output: Output {
                        width: 64,
                        height: 64,
                        display: 1,
                        headroom: 1.,
                        crop: None,
                    },
                    layer: None,
                    rate: pm_time::Rate::FPS_30,
                    end: None,
                    audio: false,
                })
                .unwrap()
        })
        .collect();
    let mut views = Views::default();
    views.apply(ViewCommand::SetLayout { count: 4 }).unwrap();
    for (pane, axis) in [Axis::Front, Axis::Right, Axis::Top, Axis::Back]
        .into_iter()
        .enumerate()
    {
        views
            .apply(ViewCommand::SetCamera {
                pane: pane as u8,
                camera: ViewCamera::Orthographic {
                    axis,
                    center: [0.; 3],
                    span: 4.,
                },
            })
            .unwrap();
    }
    views
        .apply(ViewCommand::SetTier {
            pane: 1,
            tier: Preview::Refine,
        })
        .unwrap();
    views.apply_to(&viewers).unwrap();
    for (i, viewer) in viewers.iter().enumerate() {
        assert_eq!(viewer.studio_state().unwrap(), views.panes()[i]);
    }
    let stamps: Vec<_> = viewers.iter().map(|v| v.stamp()).collect();
    views.apply_to(&viewers).unwrap();
    assert_eq!(
        viewers.iter().map(|v| v.stamp()).collect::<Vec<_>>(),
        stamps,
        "unchanged pane state must not discard in-flight frames"
    );
    let replies: Vec<_> = viewers
        .iter()
        .map(|v| {
            v.pick_surface(
                v.stamp().unwrap(),
                Ticks::ZERO,
                [0.5, 0.5],
                SurfaceQueryKind::Click,
            )
            .unwrap()
        })
        .collect();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    for (i, reply) in replies.iter().enumerate() {
        let hit = loop {
            match reply.try_recv() {
                Ok(reply) => break reply.unwrap().hit.unwrap(),
                Err(async_channel::TryRecvError::Empty) => {
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(e) => panic!("{e}"),
            }
        };
        let (axis, sign) = [(2, 1.), (0, 1.), (1, 1.), (2, -1.)][i];
        assert!(hit.normal[axis] * sign > 0.9, "view {i}: {:?}", hit.normal);
    }
    assert_eq!(snapshot.document.to_value(), before);
    let stamp = viewers[0].stamp();
    assert!(
        viewers[0]
            .studio_view(Some(studio::FreeView {
                eye: [f64::NAN, 0., 1.],
                target: [0.; 3],
                zoom: 1.
            }))
            .is_err()
    );
    assert_eq!(viewers[0].stamp(), stamp);
}

#[test]
fn studio_view_override_does_not_change_a_supplying_compositions_camera() {
    use studio::{FreeView, SceneSource, ViewCamera};
    let doc = document(false, 100.);
    let mut project = pm_scene3d::from_model(&doc.to_value().unwrap()).unwrap();
    project["comps"]["source"] = project["comps"]["test"].clone();
    project["comps"]["source"]["id"] = json!("source");
    let doc = project_document(&project);
    let source = studio::DocumentSource::new(doc.clone(), Default::default()).with_camera_for(
        "test",
        ViewCamera::Custom {
            view: FreeView {
                eye: [5., 2., 1.],
                target: [0.; 3],
                zoom: 1.,
            },
        },
    );
    let req = studio::SceneRequest {
        composition: "source".into(),
        revision: doc.revision(),
        ticks: 0,
        width: 64,
        height: 64,
    };
    let input = source.scene(&req).unwrap();
    assert!(
        (input.scene.camera.position.z - 5.).abs() < 1e-6
            && input.scene.camera.position.x.abs() < 1e-6
    );
}

#[test]
fn studio_backdrop_and_shadow_catcher_reach_every_native_tier() {
    use studio::SceneSource;
    for (catcher, background, expected) in [
        (false, Some("#88AACC"), Some([136., 170., 204.])),
        (true, None, Some([246., 245., 248.])),
        (false, None, None),
        (false, Some("#000000"), Some([0.; 3])),
    ] {
        let original = document(false, 100.);
        let mut project = pm_scene3d::from_model(&original.to_value().unwrap()).unwrap();
        let camera = project["comps"]["test"]["layers"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|l| l["id"] == "camera")
            .unwrap();
        camera["d"]["data"]["environment"]["shadowCatcher"] = json!(catcher);
        camera["d"]["data"]["environment"]["background"] = json!(background);
        let doc = project_document(&project);
        let before = doc.to_value();
        let req = studio::SceneRequest {
            composition: "test".into(),
            revision: doc.revision(),
            ticks: 0,
            width: 64,
            height: 64,
        };
        let source = studio::DocumentSource::new(doc.clone(), Default::default());
        let input = source.scene(&req).unwrap();
        assert_eq!(input.view.backdrop_opaque, Some(expected.is_some()));
        let outputs = source.auxiliary_layers(&req).unwrap();
        assert_eq!(
            outputs.iter().filter(|l| l.include_void).count(),
            usize::from(expected.is_some())
        );
        let mut renderer = Renderer::headless().unwrap();
        for tier in [
            studio::Preview::Live,
            studio::Preview::Refine,
            studio::Preview::Final,
        ] {
            studio::attach_with_preview(
                &mut renderer,
                doc.clone(),
                &Default::default(),
                &req,
                tier,
            )
            .unwrap();
            let mut request = request();
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
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            let pixels = renderer.read_rgba(&frame).unwrap();
            let corner = &pixels[(2 * 64 + 2) * 4..][..4];
            if let Some(rgb) = expected {
                assert!(
                    corner[3] > 0.99,
                    "{catcher} {background:?} {tier:?}: {corner:?}"
                );
                for i in 0..3 {
                    assert!(
                        (corner[i] * 255. - rgb[i]).abs() < 3.,
                        "{catcher} {background:?} {tier:?}: {corner:?}"
                    );
                }
            } else {
                assert!(corner[3] < 0.01, "{tier:?}: {corner:?}");
            }
        }
        assert_eq!(doc.to_value(), before);
    }
}

#[test]
fn legacy_ambient_fog_keeps_a_visible_native_volume_and_foreground() {
    let mut valid = true;
    for surface in [false, true] {
        let old = document(false, 100.);
        let mut project = pm_scene3d::from_model(&old.to_value().unwrap()).unwrap();
        let comp = &mut project["comps"]["test"];
        comp["studioQuality"]=pm_scene3d::quality::resolve(&pm_scene3d::quality::preset("production").unwrap(),&json!({"minSamples":128,"maxSamples":128,"noiseThreshold":0,"denoise":false,"caustics":false})).unwrap();
        let fog=pm_scene3d::service::new_layer(comp,"fog","Fog",pm_scene3d::schema::Role::Object,json!({"object":{"source":{"primitive":"plane"},"fog":{"p":{"width":8,"height":8,"depth":8,"density":1.,"color":"#FFFFFF"}}}}),json!({}),true,None,0.).unwrap();
        let layers = comp["layers"].as_array_mut().unwrap();
        for layer in layers.iter_mut() {
            if layer["id"] == "camera" {
                layer["d"]["data"]["environment"]["p"]["ambient"]["v"] = json!(1.);
            }
            if layer["id"] == "mirror" {
                layer["on"] = json!(surface);
                layer["d"]["data"]["object"]["material"]["p"]["metalness"]["v"] = json!(0);
                layer["d"]["data"]["object"]["material"]["p"]["roughness"]["v"] = json!(0.7);
            }
        }
        layers.push(fog);
        let doc = project_document(&project);
        let mut renderer = Renderer::headless().unwrap();
        let req = studio::SceneRequest {
            composition: "test".into(),
            revision: doc.revision(),
            ticks: 0,
            width: 64,
            height: 64,
        };
        for tier in [
            studio::Preview::Live,
            studio::Preview::Refine,
            studio::Preview::Final,
        ] {
            studio::attach_with_preview(
                &mut renderer,
                doc.clone(),
                &Default::default(),
                &req,
                tier,
            )
            .unwrap();
            let mut r = request();
            r.export = false;
            r.output_kind = OutputKind::Preview;
            let frame = renderer
                .render(
                    &doc,
                    "test",
                    Ticks::ZERO,
                    &r,
                    &pm_render::MapResolver::default(),
                )
                .unwrap();
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            let p = renderer.read_rgba(&frame).unwrap();
            let center: Vec<_> = (20..44)
                .flat_map(|y| (20..44).map(move |x| (y * 64 + x) * 4))
                .collect();
            let rgb = center
                .iter()
                .map(|i| p[*i] + p[*i + 1] + p[*i + 2])
                .sum::<f32>()
                / (center.len() * 3) as f32;
            let alpha = center.iter().map(|i| p[*i + 3]).sum::<f32>() / center.len() as f32;
            eprintln!(
                "FOG_NATIVE surface={surface} tier={tier:?} mean_rgb={rgb:.5} alpha={alpha:.5}"
            );
            valid &= rgb > 0.1 && alpha > 0.1;
        }
    }
    assert!(
        valid,
        "fog must retain lit volume radiance in every native tier"
    );
}

#[test]
fn terrain_live_and_refine_keep_the_same_exposure() {
    let mut valid = true;
    for side in [64u32, 256] {
        for (natural, amplitude, sky) in [(false, 0., false), (true, 1., false), (true, 1., true)] {
            let base = environment_geometry_document(
                json!({"source":{"primitive":"plane"},"terrain":{"p":{"width":10,"depth":10,"resolution":16,"amplitude":amplitude,"natural":natural}},"material":{"p":{"color":"#CCCCCC","roughness":0.8,"metalness":0}}}),
                json!({}),
            );
            let mut project = pm_scene3d::from_model(&base.to_value().unwrap()).unwrap();
            let comp = &mut project["comps"]["test"];
            comp["w"] = json!(side);
            comp["h"] = json!(side);
            comp["studioQuality"] = pm_scene3d::quality::resolve(
                &pm_scene3d::quality::preset("production").unwrap(),
                &json!({"minSamples":128,"maxSamples":128,"noiseThreshold":0,"caustics":false}),
            )
            .unwrap();
            for layer in comp["layers"].as_array_mut().unwrap() {
                if layer["id"] == "camera" {
                    layer["p"]["position.y"]["v"] = json!(5);
                    layer["p"]["position.z"]["v"] = json!(6);
                    layer["d"]["data"]["environment"]["p"]["ambient"]["v"] = json!(1.);
                }
            }
            if sky {
                let world = pm_scene3d::service::new_layer(comp,"world","World",pm_scene3d::schema::Role::Object,json!({"object":{"source":{"primitive":"plane"},"world":{"mode":"sky","p":{"sunIntensity":0,"visible":false}}}}),json!({}),true,None,0.).unwrap();
                comp["layers"].as_array_mut().unwrap().push(world);
            }
            let doc = project_document(&project);
            let mut renderer = Renderer::headless().unwrap();
            let req = studio::SceneRequest {
                composition: "test".into(),
                revision: doc.revision(),
                ticks: 0,
                width: side,
                height: side,
            };
            let mut means = Vec::new();
            for tier in [
                studio::Preview::Live,
                studio::Preview::Refine,
                studio::Preview::Final,
            ] {
                studio::attach_with_preview(
                    &mut renderer,
                    doc.clone(),
                    &Default::default(),
                    &req,
                    tier,
                )
                .unwrap();
                let mut r = RenderRequest::sized(Size::new(side, side), Precision::Float16);
                r.background = false;
                r.export = false;
                r.output_kind = OutputKind::Preview;
                let frame = renderer
                    .render(
                        &doc,
                        "test",
                        Ticks::ZERO,
                        &r,
                        &pm_render::MapResolver::default(),
                    )
                    .unwrap();
                assert!(frame.failures.is_empty(), "{:?}", frame.failures);
                let pixels = renderer.read_rgba(&frame).unwrap();
                let lo = side as usize * 5 / 16;
                let hi = side as usize * 11 / 16;
                let mean = (lo..hi)
                    .flat_map(|y| (lo..hi).map(move |x| (y * side as usize + x) * 4))
                    .map(|i| pixels[i] * 0.2126 + pixels[i + 1] * 0.7152 + pixels[i + 2] * 0.0722)
                    .sum::<f32>()
                    / ((hi - lo) * (hi - lo)) as f32;
                means.push(mean);
            }
            eprintln!(
                "TERRAIN_NATIVE size={side} natural={natural} amplitude={amplitude} sky={sky} live={:.5} refine={:.5} final={:.5}",
                means[0], means[1], means[2]
            );
            valid &= means[1..]
                .iter()
                .all(|reference| (means[0] - reference).abs() / reference.max(0.01) < 0.05);
        }
    }
    assert!(
        valid,
        "Live terrain must not change its exposure when it refines"
    );
}

#[test]
fn viewer_export_estimates_measure_the_shot_and_publish_four_presets_without_edits() {
    use pm_engine_service::{EngineService, ExportEstimateState, Output, ViewerConfig};
    let doc = document(false, 100.);
    let original = doc.to_value();
    let snapshot = pm_kernel::DocumentStore::new_untitled(doc.to_value().unwrap()).snapshot();
    let service = EngineService::start(pm_gpu::Gpu::new("export estimates").unwrap()).unwrap();
    let viewer = service.viewer(ViewerConfig {
        snapshot, composition: "test".into(), media: Default::default(),
        output: Output { width: 64, height: 64, display: 1, headroom: 1., crop: None },
        layer: None, rate: pm_time::Rate::FPS_30, end: None, audio: false,
    }).unwrap();
    let stamp = viewer.stamp().unwrap();
    let response = viewer.estimate_studio_export(stamp, Ticks::ZERO, Size::new(1920,1080), 12).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let estimate = loop {
        match response.try_recv() {
            Ok(result) => break result.unwrap(),
            Err(async_channel::TryRecvError::Empty) => {
                assert!(std::time::Instant::now() < deadline, "estimate never settled: {:?}", viewer.status());
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(e) => panic!("estimate worker closed: {e}"),
        }
    };
    assert_eq!(estimate.presets.iter().map(|p|p.preset.as_str()).collect::<Vec<_>>(), ["draft","preview","production","final"]);
    assert_eq!((estimate.width, estimate.height, estimate.frames), (1920,1080,12));
    assert_eq!((estimate.calibration_width, estimate.calibration_height), (320,180));
    assert!(estimate.extrapolated);
    for p in &estimate.presets {
        assert!(p.frame_ms.min > 0. && p.frame_ms.max >= p.frame_ms.min, "{p:?}");
        assert_eq!(p.total_ms.max, p.frame_ms.max * 12.);
        assert_eq!((p.calibration.cold_samples, p.calibration.warm_samples), (8,16));
        // Draft denoises on the GPU; every other preset runs the denoiser,
        // when this build has it (the guided filter otherwise).
        if p.preset == "draft" || !studio::OIDN {
            assert_eq!((p.calibration.oidn_ms, p.oidn_ms), (0., 0.), "{p:?}");
        } else {
            assert!(p.calibration.oidn_ms > 0. && p.oidn_ms > 0., "{p:?}");
        }
    }
    assert!(matches!(viewer.status().export_estimate, ExportEstimateState::Ready { .. }));
    assert_eq!(viewer.stamp(), Some(stamp));
    assert_eq!(doc.to_value(), original);
    viewer.quality(pm_engine_service::Quality::Half).unwrap();
    assert!(matches!(viewer.status().export_estimate, ExportEstimateState::Idle));
    assert!(viewer.estimate_studio_export(stamp,Ticks::ZERO,Size::new(64,64),1).is_err());
    viewer.retire();
}

#[test]
fn captured_art_is_upright_on_every_cube_face_in_each_tier() {
    use studio::{Axis, ViewCamera};
    let mut project = captured_project(json!({"layerId":"art"}));
    let layers = project["comps"]["test"]["layers"].as_array_mut().unwrap();
    let cube = layers.iter_mut().find(|l| l["id"] == "terrain").unwrap();
    cube["d"]["data"]["object"]["source"] =
        json!({"primitive":"box","parameters":{"width":2,"height":2,"depth":2}});
    cube["d"]["data"]["object"]["material"]["maps"]["emissive"] = json!({"layerId":"art"});
    cube["d"]["data"]["object"]["material"]["p"]["emissive"] = json!("#FFFFFF");
    cube["d"]["data"]["object"]["material"]["p"]["emissiveIntensity"] = json!(1);
    layers.retain(|l| l["id"] != "paint");
    for (id, x, y, color) in [
        ("tl", 0, 0, "#FF0000"),
        ("tr", 16, 0, "#00FF00"),
        ("bl", 0, 16, "#0000FF"),
        ("br", 16, 16, "#FFFF00"),
    ] {
        let mut art = source_art(id, "solid", Some("art"), color);
        art["d"]["w"] = json!(16);
        art["d"]["h"] = json!(16);
        art["p"]["position.x"]["v"] = json!(x);
        art["p"]["position.y"]["v"] = json!(y);
        layers.push(art);
    }
    let doc = project_document(&project);
    let req = studio::SceneRequest {
        composition: "test".into(),
        revision: doc.revision(),
        ticks: 0,
        width: 64,
        height: 64,
    };
    let mut renderer = Renderer::headless().unwrap();
    let mut bad = vec![];
    for axis in [
        Axis::Front,
        Axis::Right,
        Axis::Back,
        Axis::Left,
        Axis::Top,
        Axis::Bottom,
    ] {
        for tier in [
            studio::Preview::Live,
            studio::Preview::Refine,
            studio::Preview::Final,
        ] {
            studio::attach_with_camera_fonts(
                &mut renderer,
                doc.clone(),
                &Default::default(),
                &req,
                tier,
                ViewCamera::Orthographic {
                    axis,
                    center: [0.; 3],
                    span: 4.,
                },
                None,
            )
            .unwrap();
            let mut r = request();
            r.export = false;
            r.output_kind = OutputKind::Preview;
            let frame = renderer
                .render(
                    &doc,
                    "test",
                    Ticks::ZERO,
                    &r,
                    &pm_render::MapResolver::default(),
                )
                .unwrap();
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            let pixels = renderer.read_rgba(&frame).unwrap();
            let colors: Vec<_> = [(24, 24), (40, 24), (24, 40), (40, 40)]
                .map(|(x, y)| pixels[(y * 64 + x) * 4..][..3].to_vec())
                .to_vec();
            let ok = colors
                .iter()
                .zip([
                    [true, false, false],
                    [false, true, false],
                    [false, false, true],
                    [true, true, false],
                ])
                .all(|(c, on)| {
                    c.iter()
                        .zip(on)
                        .all(|(v, on)| if on { *v > 0.6 } else { *v < 0.15 })
                });
            eprintln!("CUBE_UV axis={axis:?} tier={tier:?} corners={colors:?}");
            if !ok {
                bad.push(format!("{axis:?}/{tier:?}"));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "captured source orientation wrong on {bad:?}"
    );
}

#[test]
fn front_view_extrusion_live_has_no_dark_speckle_on_a_flat_face() {
    use studio::{Axis, ViewCamera};
    let mut project = pm_scene3d::from_model(&document(false, 100.).to_value().unwrap()).unwrap();
    let comp = &mut project["comps"]["test"];
    comp["studioQuality"]=pm_scene3d::quality::resolve(&pm_scene3d::quality::preset("final").unwrap(),&json!({"minSamples":512,"maxSamples":512,"noiseThreshold":0,"denoise":false,"caustics":false})).unwrap();
    let world=pm_scene3d::service::new_layer(comp,"world","World",pm_scene3d::schema::Role::Object,json!({"object":{"source":{"primitive":"plane"},"world":{"mode":"sky","p":{"visible":false,"sunIntensity":2,"elevation":45,"azimuth":0}}}}),json!({}),true,None,0.).unwrap();
    let extrusion=pm_scene3d::extrusion::parse(&json!({"enabled":true,"p":{"depth":8,"bevelStyle":"convex","bevelDepth":1},"surfaces":{"front":{"properties":{"color":"#FFD080","roughness":0.35,"metallic":0.15}},"bevel":{"properties":{"color":"#FFD080","roughness":0.2,"metallic":0.65}}}})).unwrap();
    let layers = comp["layers"].as_array_mut().unwrap();
    layers.retain(|l| l["id"] != "mirror");
    layers.extend([world,json!({"id":"art","name":"Art","type":"shape","on":true,"threeD":true,"from":0,"dur":1,"p":{"position.x":{"v":32,"kf":[]},"position.y":{"v":32,"kf":[]},"opacity":{"v":100,"kf":[]}},"d":{"shape":"rect","w":24,"h":24,"radius":0,"smoothing":0,"stroke":0,"color":"#FFD080","extrusion":extrusion}})]);
    let doc = project_document(&project);
    let req = studio::SceneRequest {
        composition: "test".into(),
        revision: doc.revision(),
        ticks: 0,
        width: 128,
        height: 128,
    };
    let mut renderer = Renderer::headless().unwrap();
    let mut deviations = vec![];
    for tier in [
        studio::Preview::Live,
        studio::Preview::Refine,
        studio::Preview::Final,
    ] {
        studio::attach_with_camera_fonts(
            &mut renderer,
            doc.clone(),
            &Default::default(),
            &req,
            tier,
            ViewCamera::Orthographic {
                axis: Axis::Front,
                center: [0.; 3],
                span: 4.,
            },
            None,
        )
        .unwrap();
        let mut r = RenderRequest::sized(Size::new(128, 128), Precision::Float16);
        r.export = false;
        r.output_kind = OutputKind::Preview;
        r.background = false;
        let frame = renderer
            .render(
                &doc,
                "test",
                Ticks::ZERO,
                &r,
                &pm_render::MapResolver::default(),
            )
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        let p = renderer.read_rgba(&frame).unwrap();
        let ids: Vec<_> = (48..80)
            .flat_map(|y| (48..80).map(move |x| (y * 128 + x) * 4))
            .collect();
        assert!(ids.iter().all(|i| p[*i + 3] > 0.99));
        let values: Vec<_> = ids
            .iter()
            .map(|i| p[*i] * 0.2126 + p[*i + 1] * 0.7152 + p[*i + 2] * 0.0722)
            .collect();
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        let sd =
            (values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32).sqrt();
        eprintln!("FRONT_EXTRUSION tier={tier:?} mean={mean:.5} noise_sd={sd:.5}");
        deviations.push(sd);
    }
    assert!(
        deviations[0] < 0.025,
        "Live flat-face speckle: {deviations:?}"
    );
}

#[test]
fn refine_reports_completed_batches_and_denoising_and_honors_supersession() {
    use studio::RefineStage;
    let doc = document(false, 100.);
    let req = studio::SceneRequest {
        composition: "test".into(),
        revision: doc.revision(),
        ticks: 0,
        width: 64,
        height: 64,
    };
    let updates = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    let seen = updates.clone();
    let source = studio::DocumentSource::new(doc, Default::default()).with_refine_observer(Some(
        std::sync::Arc::new(move |_, p| {
            seen.lock().unwrap().push(p);
            true
        }),
    ));
    let gpu = pm_gpu::Gpu::new("Refine progress").unwrap();
    let mut service = studio::StudioService::with_device(gpu.device, gpu.queue, 64, 64).unwrap();
    service
        .render(
            &source,
            req.clone(),
            studio::Tier::QualityRefine { samples: 32 },
        )
        .unwrap();
    let p = updates.lock().unwrap();
    let sampling: Vec<_> = p
        .iter()
        .filter(|p| p.stage == RefineStage::Sampling)
        .map(|p| p.samples_done)
        .collect();
    assert_eq!(sampling.first(), Some(&0));
    assert_eq!(sampling.last(), Some(&32));
    assert!(sampling.windows(2).all(|w| w[0] <= w[1]));
    assert!(p.iter().all(|p| p.samples_target == 32));
    assert!(p.iter().any(|p| p.stage == RefineStage::Denoising));
    assert_eq!(p.last().unwrap().stage, RefineStage::Complete);
    drop(p);
    let source = source.with_refine_observer(Some(std::sync::Arc::new(|_, p| p.samples_done < 8)));
    assert!(
        service
            .render(&source, req, studio::Tier::QualityRefine { samples: 32 })
            .err()
            .expect("superseded refinement returned a frame")
            .contains("superseded")
    );
}

#[test]
fn viewer_refine_progress_reaches_status_and_is_hidden_after_view_changes() {
    use pm_engine_service::{EngineService, Output, ViewerConfig};
    let doc = document(false, 100.);
    let snapshot = pm_kernel::DocumentStore::new_untitled(doc.to_value().unwrap()).snapshot();
    let service = EngineService::start(pm_gpu::Gpu::new("viewer Refine status").unwrap()).unwrap();
    let v = service
        .viewer(ViewerConfig {
            snapshot,
            composition: "test".into(),
            media: Default::default(),
            output: Output {
                width: 64,
                height: 64,
                display: 1,
                headroom: 1.,
                crop: None,
            },
            layer: None,
            rate: pm_time::Rate::FPS_30,
            end: None,
            audio: false,
        })
        .unwrap();
    v.studio_preview(studio::Preview::Refine).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Some(p) = v.status().refine_progress {
            assert_eq!(p.samples_target, 32);
            assert!(p.samples_done <= 32);
            if p.stage == studio::RefineStage::Complete {
                assert_eq!(p.samples_done, 32);
                break;
            }
        }
        assert!(std::time::Instant::now() < deadline, "{:?}", v.status());
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    v.studio_preview(studio::Preview::Live).unwrap();
    assert!(v.status().refine_progress.is_none());
    v.retire();
}

#[test]
fn orthographic_live_text_keeps_bright_bevel_edges_free_of_dark_outliers() {
    use studio::{Axis, ViewCamera};
    let mut project = pm_scene3d::from_model(&document(false, 100.).to_value().unwrap()).unwrap();
    let comp = &mut project["comps"]["test"];
    comp["w"] = json!(1920);
    comp["h"] = json!(1080);
    comp["studioQuality"] = pm_scene3d::quality::resolve(
        &pm_scene3d::quality::preset("final").unwrap(),
        // The reference is denoised by Open Image Denoise when this build has
        // it; otherwise it stays as traced, not smoothed by the guided filter.
        &json!({"minSamples":512,"maxSamples":512,"noiseThreshold":0,"caustics":false,"denoise":studio::OIDN}),
    )
    .unwrap();
    comp["layers"]
        .as_array_mut()
        .unwrap()
        .retain(|l| l["id"] == "camera");
    let key=pm_scene3d::service::new_layer(comp,"key","Key",pm_scene3d::schema::Role::Light,json!({"light":{"type":"area","p":{"intensity":8,"width":2,"height":1.2,"targetX":0,"targetY":0,"targetZ":0}}}),json!({"position.x":-3,"position.y":5,"position.z":4}),true,None,0.).unwrap();
    let world=pm_scene3d::service::new_layer(comp,"world","World",pm_scene3d::schema::Role::Object,json!({"object":{"source":{"primitive":"plane"},"world":{"mode":"sky","p":{"intensity":1,"sunIntensity":1,"elevation":6,"azimuth":0,"haze":3,"visible":false}}}}),json!({}),true,None,0.).unwrap();
    let bevel=pm_scene3d::extrusion::parse(&json!({"enabled":true,"p":{"depth":70,"bevelStyle":"convex","bevelDepth":8},"surfaces":{"front":{"properties":{"color":"#F2B33D","roughness":0.32,"metallic":0}},"bevel":{"properties":{"color":"#FFC142","roughness":0.16,"metallic":0.65}}}})).unwrap();
    comp["layers"].as_array_mut().unwrap().extend([key,world,json!({"id":"title","name":"Title","type":"text","on":true,"threeD":true,"from":0,"dur":1,"p":{"position.x":{"v":960,"kf":[]},"position.y":{"v":300,"kf":[]},"opacity":{"v":100,"kf":[]}},"d":{"text":"STUDIO","font":"Geist","size":220,"align":"center","weight":600,"color":"#F2B33D","extrusion":bevel}})]);
    let doc = project_document(&project);
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_text_fonts(Some(extrusion_test_fonts()));
    let mut frames = vec![];
    for tier in [
        studio::Tier::QualityLive,
        studio::Tier::Final {
            samples: 512,
            salt: 9,
            denoise: studio::Denoise::Oidn,
        },
    ] {
        let source = studio::DocumentSource::new(doc.clone(), Default::default())
            .with_text_fonts(extrusion_test_fonts())
            .with_camera_for(
                "test",
                ViewCamera::Orthographic {
                    axis: Axis::Front,
                    center: [0., 1.1, 0.],
                    span: 5.,
                },
            );
        renderer.set_studio_source(
            Some(Arc::new(pm_studio3d_engine::compositor::Adapter(Arc::new(
                source,
            )))),
            pm_render::scene3d::provider::Settings {
                revision: doc.revision(),
                preview: tier,
                ..Default::default()
            },
        );
        let mut r = RenderRequest::sized(Size::new(640, 360), Precision::Float16);
        r.export = false;
        r.output_kind = OutputKind::Preview;
        r.background = false;
        let f = renderer
            .render(
                &doc,
                "test",
                Ticks::ZERO,
                &r,
                &pm_render::MapResolver::default(),
            )
            .unwrap();
        assert!(f.failures.is_empty(), "{:?}", f.failures);
        frames.push(renderer.read_rgba(&f).unwrap());
    }
    let luma = |p: &[f32], i: usize| p[i] * 0.2126 + p[i + 1] * 0.7152 + p[i + 2] * 0.0722;
    let mut count = 0;
    let mut dark = 0;
    let mut squared = 0.;
    for y in 1..359 {
        for x in 1..639 {
            let i = (y * 640 + x) * 4;
            if frames[1][i + 3] > 0.99 && luma(&frames[1], i) > 0.6 {
                count += 1;
                let d = luma(&frames[1], i) - luma(&frames[0], i);
                squared += d * d;
                if d > 0.12 {
                    dark += 1;
                }
            }
        }
    }
    let fraction = dark as f32 / count as f32;
    let rms = (squared / count as f32).sqrt();
    eprintln!("ORTHO_TEXT pixels={count} dark_fraction={fraction:.6} rms={rms:.6}");
    assert!(count > 500);
    assert!(fraction < 0.01, "dark bevel outliers: {fraction:.4}");
    assert!(rms < 0.05, "orthographic text RMS {rms}");
}


#[test]
fn native_studio_export_scale_reduces_only_transport_and_preserves_document_and_output() {
    use pm_studio3d_engine::{compositor,extract};
    struct Source { inner:studio::DocumentSource, scale:f32, seen:Arc<std::sync::Mutex<Vec<(u32,u32)>>> }
    impl studio::SceneSource for Source {
        fn scene(&self,r:&studio::SceneRequest)->Result<studio::SceneInput,String> {
            self.seen.lock().unwrap().push((r.width,r.height));self.inner.scene(r)
        }
        fn final_quality(&self,r:&studio::SceneRequest)->Result<Option<extract::FinalQuality>,String> {self.inner.final_quality(r)}
    }
    impl compositor::Source for Source {
        fn final_samples(&self,r:&studio::SceneRequest)->Result<u32,String>{compositor::Source::final_samples(&self.inner,r)}
        fn final_render_scale(&self,_:&studio::SceneRequest)->Result<f32,String>{Ok(self.scale)}
    }
    let doc=document(true,100.);let before=doc.to_value();let mut renderer=Renderer::headless().unwrap();
    for scale in [1.,0.75] {
        let seen=Arc::new(std::sync::Mutex::new(vec![]));
        let source=Source{inner:studio::DocumentSource::new(doc.clone(),Default::default()),scale,seen:seen.clone()};
        renderer.set_studio_source(Some(Arc::new(compositor::Adapter(Arc::new(source)))),compositor::Settings{revision:doc.revision(),..Default::default()});
        let frame=renderer.render(&doc,"test",Ticks::ZERO,&request(),&pm_render::MapResolver::default()).unwrap();
        assert!(frame.failures.is_empty(),"{:?}",frame.failures);
        let pixels=renderer.read_rgba(&frame).unwrap();assert_eq!(pixels.len(),64*64*4);
        assert!(pixels.iter().any(|v|*v>0.1));
        let expected=if scale==1. {(64,64)}else{(48,48)};
        assert!(!seen.lock().unwrap().is_empty());
        let actual=seen.lock().unwrap().clone();
        assert!(actual.iter().all(|dims|*dims==expected),"{actual:?}");
        assert_eq!(doc.to_value(),before);
    }
}
