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
    use pm_engine_service::jobs::{ExportHandle, ExportRequest};
    use pm_protocol::{
        color::OutputColor,
        common::{FrameRate, SessionId, TickRange},
        priority::{AlphaOutput, ExportFormat, ExportSettings, JobSpec, JobState, QualityTier},
    };
    let doc = document(true, 100.);
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
            Default::default(),
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
                    assert!(
                        image
                            .rgba
                            .chunks_exact(4)
                            .filter(|p| p[0] > 20 && p[1] == 0 && p[2] == 0)
                            .count()
                            > 100
                    );
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
