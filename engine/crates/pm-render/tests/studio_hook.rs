//! Counter qualification uses the engine workspace's instrumented wgpu build.
mod support;
use pm_protocol::{
    color::Precision,
    common::{Size, Ticks},
};
use pm_render::{OutputKind, RenderRequest, Renderer};
use pm_studio::{
    extract::{MeshInstance, SceneInput, SceneRequest, SceneSource, Transform},
    math::{Vec3, v3},
    mesh,
    scene::{Camera, Material},
    service::Denoise,
};
use serde_json::json;
use std::sync::Arc;
use support::doc::*;
struct Source(Arc<mesh::Mesh>);
impl SceneSource for Source {
    fn scene(&self, _: &SceneRequest) -> Result<SceneInput, String> {
        let mut scene = pm_studio::swatch::scene(Material::metal(Vec3::splat(0.8), 0.1));
        scene.objects.clear();
        scene.camera = Camera::lens(v3(0., 0., 5.), Vec3::ZERO, 32., 0.);
        let instances = [("left", -0.9), ("right", 0.9)]
            .into_iter()
            .map(|(layer, x)| {
                let mut world = Transform::default();
                world.0[12] = x;
                MeshInstance {
                    layer: layer.into(),
                    mesh: self.0.clone(),
                    material: 0,
                    world,
                }
            })
            .collect();
        Ok(SceneInput {
            scene,
            instances,
            materials: None,
            maps: None,
            view: Default::default(),
        })
    }
}
impl pm_render::scene3d::provider::Source for Source {
    fn final_samples(&self, _: &SceneRequest) -> Result<u32, String> {
        Ok(4)
    }
}
#[test]
fn shared_scene_trace_has_measured_gpu_cost_and_cached_final_does_not_retrace() {
    let mut renderer = Renderer::headless().unwrap();
    let document = load(&project(
        64,
        64,
        1.,
        no_fill(),
        vec![
            camera("camera", [0., 0., 5.], false, 1.),
            object3d("left", "sphere", json!({}), "#FFFFFF", 64, 64, 1.),
            object3d("right", "sphere", json!({}), "#FFFFFF", 64, 64, 1.),
        ],
    ));
    renderer.set_studio_source(
        Some(Arc::new(Source(Arc::new(mesh::sphere(
            Vec3::ZERO,
            0.7,
            12,
            24,
        ))))),
        pm_render::scene3d::provider::Settings {
            revision: document.revision(),
            denoise: Denoise::None,
            ..Default::default()
        },
    );
    let mut request = RenderRequest::sized(Size::new(64, 64), Precision::Float16);
    request.output_kind = OutputKind::Final;
    let before = pm_perf::snapshot();
    let frame = renderer
        .render(
            &document,
            "Ptest-root",
            Ticks::ZERO,
            &request,
            &pm_render::MapResolver::default(),
        )
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let pixels = renderer.read_rgba(&frame).unwrap();
    assert!(pixels.chunks_exact(4).any(|p| p[3] > 0.));
    let cost = pm_perf::snapshot().since(&before);
    assert!(
        cost.get(pm_perf::Counter::ComputePasses) > 0,
        "must use instrumented GPU path"
    );
    assert_eq!(renderer.studio_counters().traces, 1);
    assert_eq!(renderer.studio_counters().geometry_uploads, 1);
    assert_eq!(renderer.studio_counters().blas_builds, 1);
    let before = pm_perf::snapshot();
    let frame = renderer
        .render(
            &document,
            "Ptest-root",
            Ticks::ZERO,
            &request,
            &pm_render::MapResolver::default(),
        )
        .unwrap();
    assert!(frame.failures.is_empty());
    assert_eq!(pixels, renderer.read_rgba(&frame).unwrap());
    let cached = pm_perf::snapshot().since(&before);
    assert_eq!(cached.get(pm_perf::Counter::ComputePasses), 0);
    assert_eq!(renderer.studio_counters().traces, 1);
    let mesh = Arc::new(mesh::sphere(Vec3::ZERO, 0.7, 12, 24));
    renderer.set_studio_source(
        Some(Arc::new(Source(mesh))),
        pm_render::scene3d::provider::Settings {
            revision: document.revision(),
            preview: pm_studio::service::Tier::QualityRefine { samples: 2 },
            ..Default::default()
        },
    );
    request.output_kind = OutputKind::Preview;
    let traces = renderer.studio_counters().traces;
    for samples in [2, 4] {
        let frame = renderer
            .render(
                &document,
                "Ptest-root",
                Ticks::ZERO,
                &request,
                &pm_render::MapResolver::default(),
            )
            .unwrap();
        assert!(frame.failures.is_empty());
        assert_eq!(
            renderer.studio_counters().samples,
            samples,
            "Refine accumulates once per frame across both layers"
        );
    }
    assert_eq!(renderer.studio_counters().traces, traces + 2);
    eprintln!("Studio cold GPU work {cost:?}; cached Final work {cached:?}");
}
