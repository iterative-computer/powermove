//! Manual native-worker diagnostic. Never a presented-fps or image-quality gate.
#![cfg(feature = "oidn")]
mod common;
use pm_studio3d_engine::{
    extract::{
        FinalQuality, MeshInstance, SceneInput, SceneRequest, SceneSource, TextureMaps, Transform,
    },
    scene,
    service::{Denoise, StudioService, Tier},
};
use std::{sync::Arc, time::Instant};
struct Product {
    scene: scene::Scene,
    instances: Vec<MeshInstance>,
    screen: Vec<u8>,
    production: bool,
    animate: bool,
}
impl Product {
    fn new(name: &str) -> Self {
        let mut scene = scene::preset(name, "starlight", 0.).unwrap();
        let instances = scene
            .objects
            .drain(..)
            .map(|o| MeshInstance {
                layer: "product".into(),
                mesh: Arc::new(o.mesh),
                material: o.material,
                material_palette: None,
                attributes: None,
                primary_visible: true,
                world: Transform::default(),
            })
            .collect();
        Self {
            scene,
            instances,
            screen: scene::screen_wallpaper(512, 744, 0.),
            production: false,
            animate: false,
        }
    }
}
impl SceneSource for Product {
    fn scene(&self, request: &SceneRequest) -> Result<SceneInput, String> {
        let mut instances = self.instances.clone();
        if self.animate {
            let angle = request.ticks as f32 * 0.01;
            let (s, c) = angle.sin_cos();
            for instance in &mut instances {
                instance.world = Transform([
                    c, 0., -s, 0., 0., 1., 0., 0., s, 0., c, 0., 0., 0., 0., 1.,
                ]);
            }
        }
        Ok(SceneInput {
            scene: self.scene.clone(),
            instances,
            materials: None,
            maps: Some(TextureMaps {
                width: 512,
                height: 744,
                layers: vec![self.screen.clone()],
            }),
            view: Default::default(),
        })
    }
    fn final_quality(&self, _: &SceneRequest) -> Result<Option<FinalQuality>, String> {
        Ok(self.production.then_some(FinalQuality {
            sampling: pm_studio3d_engine::sampling::SamplingPolicy::PRODUCTION,
            min_samples: 8,
            max_samples: 128,
            noise_threshold: 0.12,
            max_bounces: 8,
            caustics: true,
            denoise: true,
            fast_denoise: false,
            half_resolution_guides: true,
        }))
    }
}

#[test]
#[ignore = "1080p manual Live diagnostic; run headless under pm-build.lock with >=5 GB free"]
fn native_worker_live_timing_split() {
    for name in ["hero", "chrome"] {
        let mut product = Product::new(name);
        let mut service = StudioService::default();
        let mut request = SceneRequest {
            composition: "live-timing".into(), revision: 1, ticks: 0,
            width: 1920, height: 1080,
        };
        for animated in [false, true] {
            product.animate = animated;
            for frame in 0..34 {
                request.ticks += 1;
                let rendered = service.render(&product, request.clone(), Tier::QualityLive).unwrap();
                // Complete pending layer copies without copying pixels to the CPU.
                service.device().unwrap().poll(wgpu::PollType::wait_indefinitely()).unwrap();
                if frame >= 4 {
                    println!("NATIVE_LIVE_TIMING {}", serde_json::json!({
                        "scene": name, "animated": animated, "frame": frame,
                        "width": request.width, "height": request.height,
                        "prepare_ms": rendered.timings.prepare_ms, "total_ms": rendered.timings.total_ms,
                        "pipeline": rendered.timings.pipeline, "hardware_rt": rendered.hardware_rt,
                        "geometry_uploads": service.geometry_uploads(), "scene_updates": service.scene_updates(),
                        "texture_uploads": service.texture_uploads(),
                        "blas_builds": service.blas_builds(),
                    }));
                }
            }
        }
    }
}
#[test]
#[ignore = "1080p manual timing; run under pm-build.lock with >=5 GB free"]
fn native_worker_product_timing_split() {
    let out = std::env::var_os("PM_STUDIO_TIMING_OUT").map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("pm-studio-timing-{}", std::process::id())));
    std::fs::create_dir_all(&out).unwrap();
    for name in ["hero", "chrome"] {
        let mut product = Product::new(name);
        let mut service = StudioService::default();
        let mut request = SceneRequest {
            composition: "timing".into(),
            revision: 1,
            ticks: 0,
            width: 1920,
            height: 1080,
        };
        service
            .render(&product, request.clone(), Tier::QualityLive)
            .unwrap();
        for preset in ["refine", "production"] {
            product.production = preset == "production";
            for run in 0..2 {
                request.revision += 1;
                let frame = service
                    .render(
                        &product,
                        request.clone(),
                        if product.production {
                            Tier::Final {
                                samples: 256,
                                salt: 0,
                                denoise: Denoise::Oidn,
                            }
                        } else {
                            Tier::QualityRefine { samples: 32 }
                        },
                    )
                    .unwrap();
                let read_start = Instant::now();
                let pixels = common::read(
                    service.device().unwrap(),
                    service.queue().unwrap(),
                    &frame.layers[0].texture,
                );
                let readback_ms = read_start.elapsed().as_secs_f64() * 1000.;
                assert!(pixels.chunks_exact(4).any(|p| p[3] > 0));
                let encode_start = Instant::now();
                crate::pm_studio_png(&out.join(format!("{name}-{preset}-{run}.png")), &pixels);
                let png_ms = encode_start.elapsed().as_secs_f64()*1000.;
                println!(
                    "NATIVE_TIMING {}",
                    serde_json::json!({"scene":name,"preset":preset,"run":run,"width":1920,"height":1080,"samples":frame.samples,"hardware_rt":frame.hardware_rt,"prepare_ms":frame.timings.prepare_ms,"render_ms":frame.timings.total_ms,"pipeline":frame.timings.pipeline,"oidn":frame.timings.oidn,"oidn_init_ms":frame.timings.oidn_init_ms,"oidn_reused":frame.timings.oidn_reused,"readback_ms":readback_ms,"png_encode_write_ms":png_ms,"scene_updates":service.scene_updates(),"geometry_uploads":service.geometry_uploads(),"texture_uploads":service.texture_uploads()})
                );
            }
        }
    }
}
fn pm_studio_png(path: &std::path::Path, pixels: &[u8]) {
    pm_studio3d_engine::gpu::write_png(path,1920,1080,pixels).unwrap();
}

#[test]
fn draft_spatial_denoise_keeps_targets_and_reuses_geometry() {
    struct Draft(Product);
    impl SceneSource for Draft {
        fn scene(&self,r:&SceneRequest)->Result<SceneInput,String>{self.0.scene(r)}
        fn final_quality(&self,_:&SceneRequest)->Result<Option<FinalQuality>,String>{Ok(Some(FinalQuality{sampling:Default::default(),min_samples:4,max_samples:16,noise_threshold:0.12,max_bounces:4,caustics:false,denoise:true,fast_denoise:true,half_resolution_guides:true}))}
    }
    let source=Draft(Product::new("hero"));let mut service=StudioService::default();
    for ticks in [0,1] {
        let request=SceneRequest{composition:"draft-filter".into(),revision:1,ticks,width:96,height:64};
        let frame=service.render(&source,request,Tier::Final{samples:16,salt:0,denoise:Denoise::Oidn}).unwrap();
        service.device().unwrap().poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert_eq!(frame.timings.oidn.total_ms,0.);assert_eq!(frame.timings.pipeline.guide_readback_ms,0.);
        assert!((4..=16).contains(&frame.samples));assert!(!frame.layers.is_empty());
        assert_eq!((frame.layers[0].texture.width(),frame.layers[0].texture.height()),(96,64));
    }
    assert_eq!(service.geometry_uploads(),1);assert_eq!(service.texture_uploads(),1);
}
