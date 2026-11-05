mod common;
use pm_studio::extract::{MeshInstance, SceneInput, SceneRequest, SceneSource, Transform};
use pm_studio::{
    math::{Vec3, v3},
    mesh,
    scene::{Camera, Material, Softbox, preset},
    service::{Denoise, StudioService, Tier},
};
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

struct Source {
    mesh: Arc<mesh::Mesh>,
    changed: AtomicU32,
}
impl Source {
    fn new() -> Self {
        Self {
            mesh: Arc::new(mesh::sphere(Vec3::ZERO, 0.7, 12, 24)),
            changed: AtomicU32::new(0),
        }
    }
}
impl SceneSource for Source {
    fn scene(&self, _request: &SceneRequest) -> Result<SceneInput, String> {
        let mut scene = preset("hero", "space-gray", 0.).unwrap();
        scene.objects.clear();
        scene.materials = vec![Material::metal(
            if self.changed.load(Ordering::SeqCst) == 0 {
                v3(0.8, 0.2, 0.1)
            } else {
                v3(0.1, 0.2, 0.8)
            },
            0.3,
        )];
        scene.camera = Camera::lens(v3(0., 0., 5.), Vec3::ZERO, 32., 0.);
        scene.environment = (Vec3::splat(0.3), Vec3::splat(0.1));
        scene.shadow_catcher = false;
        scene.lights = vec![Softbox::aimed(
            Vec3::ZERO,
            -30.,
            40.,
            8.,
            4.,
            4.,
            Vec3::splat(4.),
        )];
        let instances = ["left", "right"]
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let mut world = Transform::default();
                world.0[12] = if i == 0 { -0.8 } else { 0.8 };
                MeshInstance {
                    layer: (*id).into(),
                    mesh: self.mesh.clone(),
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
fn request() -> SceneRequest {
    SceneRequest {
        composition: "comp".into(),
        revision: 1,
        ticks: 0,
        width: 64,
        height: 48,
    }
}
fn pixels(service: &StudioService, rendered: &pm_studio::service::RenderedScene) -> Vec<Vec<u8>> {
    rendered
        .layers
        .iter()
        .map(|l| {
            common::read(
                service.device().unwrap(),
                service.queue().unwrap(),
                &l.texture,
            )
        })
        .collect()
}
#[test]
fn provider_ids_layers_progress_and_resource_invalidation() {
    let source = Source::new();
    let mut service = StudioService::default();
    let final_tier = Tier::Final {
        samples: 4,
        salt: 1,
        denoise: Denoise::Guided,
    };
    let frame = service.render(&source, request(), final_tier).unwrap();
    assert_eq!(
        frame
            .layers
            .iter()
            .map(|l| l.layer.as_str())
            .collect::<Vec<_>>(),
        ["left", "right"]
    );
    assert!(
        !frame.hardware_rt,
        "local-space instances use the real compute BVH"
    );
    assert_eq!(service.blas_builds(), 1);
    let first = pixels(&service, &frame);
    for layer in &first {
        assert!(layer.chunks_exact(4).filter(|p| p[3] > 0).count() > 20);
    }
    assert!(
        first[0]
            .chunks_exact(4)
            .zip(first[1].chunks_exact(4))
            .all(|(a, b)| a[3] == 0 || b[3] == 0)
    );
    let frame = service.render(&source, request(), final_tier).unwrap();
    assert_eq!(first, pixels(&service, &frame));
    source.changed.store(1, Ordering::SeqCst); // Same revision: resources still invalidate.
    let frame = service.render(&source, request(), final_tier).unwrap();
    assert_ne!(first, pixels(&service, &frame));
    assert_eq!(service.blas_builds(), 1);
    let refine = Tier::QualityRefine { samples: 2 };
    assert_eq!(
        service.render(&source, request(), refine).unwrap().samples,
        2
    );
    assert_eq!(
        service.render(&source, request(), refine).unwrap().samples,
        4
    );
    let mut moved = request();
    moved.ticks = 29_400_000;
    assert_eq!(service.render(&source, moved, refine).unwrap().samples, 2);
    let live = service
        .render(&source, request(), Tier::QualityLive)
        .unwrap();
    assert_eq!(
        (
            live.layers[0].texture.width(),
            live.layers[0].texture.height()
        ),
        (32, 24)
    );
    assert_eq!(live.samples, 1);
    assert_eq!(
        service.render(&source, request(), refine).unwrap().samples,
        2
    );
    assert!(
        service
            .render(&source, request(), Tier::Fast)
            .unwrap_err_text()
            .contains("Fast compositor adapter")
    );
}
trait ErrorText {
    fn unwrap_err_text(self) -> String;
}
impl<T> ErrorText for Result<T, String> {
    fn unwrap_err_text(self) -> String {
        match self {
            Err(e) => e,
            Ok(_) => panic!("expected an explicit failure"),
        }
    }
}

#[test]
fn unsupported_or_zero_sample_final_never_claims_success() {
    let mut service = StudioService::default();
    let source = Source::new();
    assert!(
        service
            .render(
                &source,
                request(),
                Tier::Final {
                    samples: 0,
                    salt: 0,
                    denoise: Denoise::None
                }
            )
            .is_err()
    );
    #[cfg(not(feature = "oidn"))]
    assert!(
        service
            .render(
                &source,
                request(),
                Tier::Final {
                    samples: 4,
                    salt: 0,
                    denoise: Denoise::Oidn
                }
            )
            .unwrap_err_text()
            .contains("without optional OIDN")
    );
    assert!(service.device().is_none());
}
