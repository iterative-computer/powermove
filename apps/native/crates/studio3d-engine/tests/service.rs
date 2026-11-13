use pm_studio3d_engine::SceneExt;
mod common;
use pm_studio3d_engine::extract::{MeshInstance, SceneInput, SceneRequest, SceneSource, Transform};
use pm_studio3d_engine::{
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
                    material_palette: None,
                    attributes: None,
                    primary_visible: true,
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
fn pixels(
    service: &StudioService,
    rendered: &pm_studio3d_engine::service::RenderedScene,
) -> Vec<Vec<u8>> {
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
        frame.hardware_rt
            == (service
                .device()
                .unwrap()
                .features()
                .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
                && std::env::var_os("STUDIO_NO_RT").is_none()),
        "instances use hardware traversal when the device supports it"
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
    assert_eq!(service.geometry_uploads(), 1);
    assert_eq!(service.scene_updates(), 1);
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
    assert_eq!(
        service.render(&source, request(), refine).unwrap().samples,
        2
    );
    let live = service
        .render(&source, request(), Tier::QualityLive)
        .unwrap();
    assert_eq!(
        (
            live.layers[0].texture.width(),
            live.layers[0].texture.height()
        ),
        (64, 48)
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
    assert!(service.device().is_none());
}

/// A build without Open Image Denoise cleans what would have gone to it with
/// the engine's own guided filter, rather than refusing the frame.
#[cfg(not(feature = "oidn"))]
#[test]
fn without_the_denoiser_its_frames_use_the_guided_filter() {
    assert!(!pm_studio3d_engine::OIDN);
    let mut service = StudioService::default();
    let source = Source::new();
    let tier = |denoise| Tier::Final { samples: 4, salt: 1, denoise };
    let frame = service.render(&source, request(), tier(Denoise::Guided)).unwrap();
    let guided = pixels(&service, &frame);
    let frame = service.render(&source, request(), tier(Denoise::Oidn)).unwrap();
    assert_eq!(guided, pixels(&service, &frame));
    let frame = service.render(&source, request(), tier(Denoise::None)).unwrap();
    assert_ne!(guided, pixels(&service, &frame), "the guided filter ran");
    let frame = service
        .render(&source, request(), Tier::QualityRefine { samples: 2 })
        .unwrap();
    assert!(pixels(&service, &frame).iter().all(|image| image.chunks_exact(4).any(|p| p[3] > 0)));
}

struct Screens {
    source: Source,
    swapped: std::sync::atomic::AtomicBool,
}
impl SceneSource for Screens {
    fn scene(&self, request: &SceneRequest) -> Result<SceneInput, String> {
        let mut input = self.source.scene(request)?;
        input.scene.lights.clear();
        input.scene.extra_lights.clear();
        input.scene.environment = (Vec3::ZERO, Vec3::ZERO);
        let mut display = Material::dielectric(Vec3::ZERO, 1.);
        display.emission = Vec3::splat(0.5);
        display.specular = 0.;
        input.scene.materials = vec![display, display];
        let mut materials = input.scene.gpu_materials();
        materials[0].maps[1] = 0.;
        materials[1].maps[1] = 1.;
        input.materials = Some(materials);
        input.instances[1].material = 1;
        let mut layers = vec![vec![255, 0, 0, 255], vec![0, 0, 255, 255]];
        if self.swapped.load(Ordering::SeqCst) {
            layers.swap(0, 1);
        }
        input.maps = Some(pm_studio3d_engine::extract::TextureMaps {
            width: 1,
            height: 1,
            layers,
        });
        Ok(input)
    }
}
#[test]
fn multiple_live_screens_update_at_the_same_document_revision() {
    let source = Screens {
        source: Source::new(),
        swapped: std::sync::atomic::AtomicBool::new(false),
    };
    let mut service = StudioService::default();
    let tier = Tier::Final {
        samples: 4,
        salt: 19,
        denoise: Denoise::None,
    };
    let frame = service.render(&source, request(), tier).unwrap();
    let before = pixels(&service, &frame);
    assert!(before[0].chunks_exact(4).filter(|p| p[0] > p[2]).count() > 20);
    assert!(before[1].chunks_exact(4).filter(|p| p[2] > p[0]).count() > 20);
    source.swapped.store(true, Ordering::SeqCst);
    let frame = service.render(&source, request(), tier).unwrap();
    let after = pixels(&service, &frame);
    assert!(after[0].chunks_exact(4).filter(|p| p[2] > p[0]).count() > 20);
    assert!(after[1].chunks_exact(4).filter(|p| p[0] > p[2]).count() > 20);
    assert_eq!(service.geometry_uploads(), 1);
}

#[cfg(feature = "oidn")]
#[test]
fn final_can_use_the_optional_oidn_device() {
    let mut service = StudioService::default();
    let source = Source::new();
    let frame = service
        .render(
            &source,
            request(),
            Tier::Final {
                samples: 4,
                salt: 1,
                denoise: Denoise::Oidn,
            },
        )
        .unwrap();
    let images = pixels(&service, &frame);
    assert_eq!(images.len(), 2);
    assert!(
        images
            .iter()
            .all(|image| image.chunks_exact(4).any(|p| p[3] > 0 && p[0] > 0))
    );
    let tier = Tier::Final {
        samples: 4,
        salt: 1,
        denoise: Denoise::Oidn,
    };
    let repeated = service.render(&source, request(), tier).unwrap();
    assert!(repeated.timings.oidn_reused);
    assert_eq!(images, pixels(&service, &repeated));
    let mut resized = request();
    resized.width = 80;
    service
        .render(&source, resized, Tier::QualityRefine { samples: 2 })
        .unwrap();
    let restored = service.render(&source, request(), tier).unwrap();
    assert!(restored.timings.oidn_reused);
    assert_eq!(
        images,
        pixels(&service, &restored),
        "cached filters survive size and guide-quality changes"
    );
}

struct GreyScreen {
    source: Source,
    textured: bool,
}
impl SceneSource for GreyScreen {
    fn scene(&self, request: &SceneRequest) -> Result<SceneInput, String> {
        let mut input = self.source.scene(request)?;
        input.scene.lights.clear();
        input.scene.extra_lights.clear();
        input.scene.environment = (Vec3::ZERO, Vec3::ZERO);
        // Independent sRGB EOTF for the authored 8-bit mid-grey source.
        let linear = ((128f32 / 255. + 0.055) / 1.055).powf(2.4);
        let mut display = Material::dielectric(Vec3::ZERO, 1.);
        display.specular = 0.;
        display.emission = Vec3::splat(if self.textured { 0.5 } else { 0.5 * linear });
        input.scene.materials = vec![display];
        if self.textured {
            let mut materials = input.scene.gpu_materials();
            materials[0].maps[1] = 0.;
            input.materials = Some(materials);
            input.maps = Some(pm_studio3d_engine::extract::TextureMaps {
                width: 1,
                height: 1,
                layers: vec![vec![128, 128, 128, 255]],
            });
        }
        Ok(input)
    }
}
#[test]
fn srgb_screens_are_decoded_exactly_once() {
    let mut service = StudioService::default();
    let tier = Tier::Final {
        samples: 4,
        salt: 29,
        denoise: Denoise::None,
    };
    let encoded = GreyScreen {
        source: Source::new(),
        textured: true,
    };
    let linear = GreyScreen {
        source: Source::new(),
        textured: false,
    };
    let frame = service.render(&encoded, request(), tier).unwrap();
    let textured = pixels(&service, &frame);
    let frame = service.render(&linear, request(), tier).unwrap();
    let explicit = pixels(&service, &frame);
    assert!(
        explicit
            .iter()
            .all(|p| p.chunks_exact(4).any(|p| p[0] > 20 && p[3] > 0))
    );
    for (layer, (actual, expected)) in textured.iter().zip(&explicit).enumerate() {
        for (pixel, (actual, expected)) in actual
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .enumerate()
        {
            assert_eq!(
                actual, expected,
                "mapped sRGB must match explicitly decoded emission: layer {layer}, pixel {pixel}"
            );
        }
    }
}

#[test]
fn changing_instance_capacity_reallocates_gpu_buffers_safely() {
    struct Growing {
        source: Source,
        count: AtomicU32,
    }
    impl SceneSource for Growing {
        fn scene(&self, request: &SceneRequest) -> Result<SceneInput, String> {
            let mut input = self.source.scene(request)?;
            for i in 2..self.count.load(Ordering::SeqCst) {
                input.instances.push(MeshInstance {
                    layer: format!("copy-{i}"),
                    mesh: self.source.mesh.clone(),
                    material: 0,
                    material_palette: None,
                    attributes: None,
                    primary_visible: true,
                    world: Transform::default(),
                });
            }
            Ok(input)
        }
    }
    let source = Growing {
        source: Source::new(),
        count: AtomicU32::new(2),
    };
    let mut service = StudioService::default();
    for count in [2, 12, 3, 2] {
        source.count.store(count, Ordering::SeqCst);
        let frame = service
            .render(
                &source,
                request(),
                Tier::Final {
                    samples: 4,
                    salt: 0,
                    denoise: Denoise::None,
                },
            )
            .unwrap();
        assert_eq!(frame.layers.len(), count as usize);
        assert!(pixels(&service, &frame).iter().flatten().any(|p| *p > 0));
    }
}

struct MovingCamera {
    source: Source,
    mode: AtomicU32,
}
impl SceneSource for MovingCamera {
    fn scene(&self, request: &SceneRequest) -> Result<SceneInput, String> {
        let mut input = self.source.scene(request)?;
        match self.mode.load(Ordering::SeqCst) {
            1 => input.scene.camera.position.x = 2.,
            2 => input.scene.meters_per_unit = 1.,
            _ => {}
        }
        Ok(input)
    }
}
#[test]
fn camera_motion_changes_pixels_without_scene_or_geometry_uploads() {
    let source = MovingCamera {
        source: Source::new(),
        mode: AtomicU32::new(0),
    };
    let mut service = StudioService::default();
    let tier = Tier::Final {
        samples: 4,
        salt: 7,
        denoise: Denoise::None,
    };
    let first = service.render(&source, request(), tier).unwrap();
    let before = pixels(&service, &first);
    source.mode.store(1, Ordering::SeqCst);
    let moved = service.render(&source, request(), tier).unwrap();
    assert_ne!(before, pixels(&service, &moved));
    assert_eq!(service.geometry_uploads(), 1);
    assert_eq!(
        service.scene_updates(),
        1,
        "camera motion must not rebuild the RT scene"
    );
}
struct MappedMotion {
    source: Source,
    image: AtomicU32,
}
impl SceneSource for MappedMotion {
    fn scene(&self, request: &SceneRequest) -> Result<SceneInput, String> {
        let mut input = self.source.scene(request)?;
        input.scene.materials = vec![Material::dielectric(Vec3::splat(0.8),0.7)];
        let mut materials = input.scene.gpu_materials();
        let image = self.image.load(Ordering::SeqCst);
        if image < 2 {
            materials[0].maps[0] = 0.;
            input.maps = Some(pm_studio3d_engine::extract::TextureMaps {
                width: 1, height: 1,
                layers: vec![if image == 0 {vec![255,0,0,255]} else {vec![0,0,255,255]}],
            });
        }
        input.materials = Some(materials);
        for instance in &mut input.instances {
            instance.world.0[13] += request.ticks as f32 * 0.25;
        }
        Ok(input)
    }
}
#[test]
fn moving_instances_retain_images_and_same_revision_image_edits_still_land() {
    let source = MappedMotion { source: Source::new(), image: AtomicU32::new(0) };
    let mut service = StudioService::default();
    let tier = Tier::Final { samples: 8, salt: 0, denoise: Denoise::None };
    let mut request = request();
    let first = service.render(&source, request.clone(), tier).unwrap();
    let before = pixels(&service, &first);
    request.ticks = 1;
    let moved = service.render(&source, request.clone(), tier).unwrap();
    let red = pixels(&service, &moved);
    assert_ne!(before, red, "motion must still update the scene");
    assert_eq!(service.geometry_uploads(), 1);
    assert_eq!(service.texture_uploads(), 1, "moving an instance must retain its image allocation");
    source.image.store(1, Ordering::SeqCst);
    let edited = service.render(&source, request.clone(), tier).unwrap();
    assert_ne!(red, pixels(&service, &edited), "same-size pixel changes must replace the image");
    assert_eq!(service.texture_uploads(), 2);
    source.image.store(2, Ordering::SeqCst);
    let removed = service.render(&source, request.clone(), tier).unwrap();
    assert_ne!(red, pixels(&service, &removed));
    assert_eq!(service.texture_uploads(), 3);
    source.image.store(0, Ordering::SeqCst);
    let restored = service.render(&source, request.clone(), tier).unwrap();
    assert_eq!(red, pixels(&service, &restored));
    assert_eq!(service.texture_uploads(), 4);
    request.width *= 2;request.height *= 2;
    let resized = service.render(&source, request, tier).unwrap();
    assert!(pixels(&service, &resized)[0].chunks_exact(4).any(|p|p[3]>0));
    assert_eq!(service.texture_uploads(), 4, "resolution changes retain source images");
}
#[test]
fn aperture_units_reset_refine_accumulation_at_the_same_revision() {
    let source = MovingCamera {
        source: Source::new(),
        mode: AtomicU32::new(0),
    };
    let mut service = StudioService::default();
    let tier = Tier::QualityRefine { samples: 2 };
    assert_eq!(service.render(&source, request(), tier).unwrap().samples, 2);
    assert_eq!(service.render(&source, request(), tier).unwrap().samples, 4);
    source.mode.store(2, Ordering::SeqCst);
    assert_eq!(service.render(&source, request(), tier).unwrap().samples, 2);
    assert_eq!(service.scene_updates(), 1);
}
#[test]
fn surface_queries_share_rendered_layers_world_geometry_and_refine_accumulation() {
    let source=Source::new();
    let mut service=StudioService::default();
    let req=request();
    let tier=Tier::QualityRefine {samples:4};
    // A query is useful before the first frame and does not trace samples.
    let left=service.pick_surface(&source,req.clone(),tier,0.29,0.5).unwrap().unwrap();
    assert_eq!(left.layer,"left");
    assert!((left.point[0]+0.8).abs()<0.2 && left.point[2]>0.5);
    assert!((left.normal.iter().map(|n|n*n).sum::<f32>()-1.).abs()<0.001);
    assert!(left.normal[2]>0.8);
    assert!(service.pick_surface(&source,req.clone(),tier,0.5,0.02).unwrap().is_none());
    assert!(service.pick_surface(&source,req.clone(),tier,f32::NAN,0.5).is_err());
    assert!(service.pick_surface(&source,req.clone(),tier,1.1,0.5).is_err());
    assert_eq!(service.render(&source,req.clone(),tier).unwrap().samples,4);
    let right=service.pick_surface(&source,req.clone(),tier,0.71,0.5).unwrap().unwrap();
    assert_eq!(right.layer,"right");
    assert!(right.point[0]>0.6);
    assert_eq!(service.render(&source,req,tier).unwrap().samples,8,"hover must retain refinement");
    assert_eq!(service.geometry_uploads(),1);
    assert_eq!(service.texture_uploads(),1);
}

struct BackdropSource {
    source: Source,
    owners: Vec<String>,
}
impl SceneSource for BackdropSource {
    fn scene(&self, request: &SceneRequest) -> Result<SceneInput, String> {
        let mut input = self.source.scene(request)?;
        input.scene.backdrop = v3(0.25, 0.5, 0.75);
        input.view.backdrop_opaque = Some(true);
        Ok(input)
    }
    fn auxiliary_layers(
        &self,
        _request: &SceneRequest,
    ) -> Result<Vec<pm_studio3d_engine::extract::AuxiliaryLayer>, String> {
        Ok(self
            .owners
            .iter()
            .map(|id| pm_studio3d_engine::extract::AuxiliaryLayer {
                id: id.clone(),
                include_void: true,
            })
            .collect())
    }
}
#[test]
fn one_surface_output_can_own_the_backdrop_and_duplicate_owners_are_rejected() {
    let mut source = BackdropSource {
        source: Source::new(),
        owners: vec!["right".into()],
    };
    let mut service = StudioService::default();
    let frame = service
        .render(
            &source,
            request(),
            Tier::Final {
                samples: 4,
                salt: 1,
                denoise: Denoise::None,
            },
        )
        .unwrap();
    let output = pixels(&service, &frame);
    for (layer, pixels) in frame.layers.iter().zip(output) {
        let corner = &pixels[..4];
        if layer.layer == "right" {
            assert!(corner[3] > 250);
            for (actual, expected) in corner[..3].iter().zip([64, 128, 191]) {
                assert!((*actual as i32 - expected).abs() <= 2, "{corner:?}");
            }
        } else {
            assert_eq!(corner[3], 0);
        }
    }
    source.owners.push("left".into());
    assert!(
        service
            .render(&source, request(), Tier::QualityLive)
            .err()
            .unwrap()
            .contains("multiple output owners")
    );
}
