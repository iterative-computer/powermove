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
    production: bool,
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
                primary_visible: true,
                world: Transform::default(),
            })
            .collect();
        Self {
            scene,
            instances,
            production: false,
        }
    }
}
impl SceneSource for Product {
    fn scene(&self, _: &SceneRequest) -> Result<SceneInput, String> {
        Ok(SceneInput {
            scene: self.scene.clone(),
            instances: self.instances.clone(),
            materials: None,
            maps: Some(TextureMaps {
                width: 512,
                height: 744,
                layers: vec![scene::screen_wallpaper(512, 744, 0.)],
            }),
            view: Default::default(),
        })
    }
    fn final_quality(&self, _: &SceneRequest) -> Result<Option<FinalQuality>, String> {
        Ok(self.production.then_some(FinalQuality {
            min_samples: 32,
            max_samples: 256,
            noise_threshold: 0.03,
            max_bounces: 8,
            caustics: true,
            denoise: true,
            half_resolution_guides: true,
        }))
    }
}
#[test]
#[ignore = "1080p manual timing; run under pm-build.lock with >=5 GB free"]
fn native_worker_product_timing_split() {
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
                println!(
                    "NATIVE_TIMING {}",
                    serde_json::json!({"scene":name,"preset":preset,"run":run,"width":1920,"height":1080,"samples":frame.samples,"hardware_rt":frame.hardware_rt,"prepare_ms":frame.timings.prepare_ms,"render_ms":frame.timings.total_ms,"pipeline":frame.timings.pipeline,"oidn":frame.timings.oidn,"oidn_init_ms":frame.timings.oidn_init_ms,"oidn_reused":frame.timings.oidn_reused,"readback_ms":readback_ms,"scene_updates":service.scene_updates(),"geometry_uploads":service.geometry_uploads()})
                );
            }
        }
    }
}
