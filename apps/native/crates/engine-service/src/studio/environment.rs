//! Evaluated Studio World and fog transport. No authored state or UI work.
use super::*;
use pm_studio3d_engine::environment::{AdaptiveSampling, Atmosphere, Sky, Volume, VolumeShape};

pub(super) fn apply(
    r: &pm_scene3d::service::ResolvedScene,
    scene: &mut Scene,
    composition: &str,
    failures: &mut Vec<pm_render::LayerFailure>,
    image: &dyn Fn(&Value) -> Result<pm_studio3d_engine::environment::EnvironmentMap, String>,
) -> Result<(), String> {
    let objects = r.scene["objects"]
        .as_array()
        .ok_or("Missing Studio objects")?;
    let lights = r.scene["lights"]
        .as_array()
        .ok_or("Missing Studio lights")?;
    let world = objects
        .iter()
        .find(|o| o.get("world").is_some() && val(&o["p"]["visible"]) != false);
    let light = lights
        .iter()
        .find(|l| l["type"] == "environment" && val(&l["p"]["visible"]) != false);
    let (node, multiplier) = if let Some(owner) = world {
        (&owner["world"], 1.)
    } else if let Some(light) = light {
        (&light["world"], n(&light["p"], "intensity", 1.))
    } else {
        (&Value::Null, 1.)
    };
    let mut candidate = scene.world.clone();
    let mut meters_per_unit = scene.meters_per_unit;
    let world_result = (|| -> Result<(), String> {
        if !node.is_null() {
            let p = &node["p"];
            meters_per_unit = n(p, "metersPerUnit", 0.1);
            match node["mode"].as_str().unwrap_or("sky") {
                "sky" => {
                    candidate.sky = Some(Sky {
                        elevation: n(p, "elevation", 45.),
                        azimuth: n(p, "azimuth", 0.) + n(p, "rotation", 0.),
                        turbidity: n(p, "haze", 2.),
                        ground_albedo: color(p, "groundColor", "#7C7C7C"),
                        intensity: n(p, "intensity", 1.) * multiplier,
                        sun_intensity: n(p, "sunIntensity", 1.),
                        sun_radius: n(p, "sunRadius", 0.266),
                        overcast: n(p, "overcast", 0.),
                        visible: val(&p["visible"]) != false,
                    })
                }
                "hdri" => {
                    let mut map = image(&node["source"])?;
                    map.intensity = n(p, "intensity", 1.) * multiplier;
                    map.rotation = n(p, "rotation", 0.);
                    map.visible = val(&p["visible"]) != false;
                    candidate.hdri = Some(map);
                }
                "none" => {}
                _ => return Err("Invalid Studio World mode".into()),
            }
            candidate.atmosphere = Atmosphere {
                density: n(p, "fogDensity", 0.),
                color: color(p, "fogColor", "#E6E6E6"),
                anisotropy: n(p, "fogAnisotropy", 0.),
                base_height: n(p, "fogBaseHeight", 0.),
                height_falloff: n(p, "fogHeightFalloff", 0.),
                distance: n(p, "fogDistance", 10000.),
            };
            candidate.adaptive = AdaptiveSampling {
                enabled: val(&p["adaptive"]) == true,
                min_samples: (n(p, "minSamples", 64.).round() as u32).max(4),
                relative_error: n(p, "relativeError", 0.025),
                absolute_error: n(p, "absoluteError", 0.001),
            };
        }
        if !meters_per_unit.is_finite() || meters_per_unit<=0. {return Err("World scale must be finite and positive".into());}
        candidate.validate()?;
        Ok(())
    })();
    match world_result {
        Ok(()) => {
            scene.world = candidate;
            scene.meters_per_unit = meters_per_unit;
        }
        Err(message) => {
            let owner = world
                .or(light)
                .and_then(|o| o["id"].as_str())
                .ok_or("Invalid Studio World owner")?;
            failures.push(pm_render::LayerFailure {
                composition: composition.into(),
                layer: owner.into(),
                message,
            });
        }
    }
    for object in objects
        .iter()
        .filter(|o| o.get("fog").is_some() && val(&o["p"]["visible"]) != false)
    {
        let id = object["id"].as_str().ok_or("Fog has no layer ID")?;
        let result = (|| -> Result<(), String> {
            let id = object["id"].as_str().ok_or("Fog has no layer ID")?;
            let matrix = r
                .model_worlds
                .get(id)
                .ok_or("Fog transform is unavailable")?;
            let p = &object["fog"]["p"];
            let half = [
                n(p, "width", 2.) / 2.,
                n(p, "height", 2.) / 2.,
                n(p, "depth", 2.) / 2.,
            ];
            let shape = if val(&p["shape"]) == "sphere" {
                VolumeShape::Sphere
            } else {
                VolumeShape::Box
            };
            let size = if matches!(shape, VolumeShape::Sphere) {
                let scale = [0, 4, 8].map(|i| {
                    ((matrix[i] * matrix[i]
                        + matrix[i + 1] * matrix[i + 1]
                        + matrix[i + 2] * matrix[i + 2]) as f32)
                        .sqrt()
                });
                Vec3::splat(half[0] * scale.into_iter().fold(0., f32::max))
            } else {
                let extent = |i: usize| {
                    (matrix[i].abs() as f32) * half[0]
                        + (matrix[i + 4].abs() as f32) * half[1]
                        + (matrix[i + 8].abs() as f32) * half[2]
                };
                v3(extent(0), extent(1), extent(2))
            };
            let volume = Volume {
                shape,
                center: v3(matrix[12] as f32, matrix[13] as f32, matrix[14] as f32),
                size,
                density: n(p, "density", 0.02),
                color: color(p, "color", "#E6E6E6"),
                anisotropy: n(p, "anisotropy", 0.),
            };
            if scene.world.volumes.len() >= 32 {
                return Err("Studio supports at most 32 fog volumes".into());
            }
            let mut probe = pm_studio3d_engine::environment::World::default();
            probe.volumes.push(volume.clone());
            probe.validate()?;
            scene.world.volumes.push(volume);
            Ok(())
        })();
        if let Err(message) = result {
            failures.push(pm_render::LayerFailure {
                composition: composition.into(),
                layer: id.into(),
                message,
            });
        }
    }
    scene.world.validate()
}
