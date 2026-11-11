//! Evaluated material options bind per timeline owner, preserving shared meshes.
use super::*;

pub(super) fn apply(
    r: &pm_scene3d::service::ResolvedScene,
    instances: &mut [MeshInstance],
    scene: &mut Scene,
    materials: &mut Vec<GpuMaterial>,
) -> Result<(), String> {
    let objects = r.scene["objects"]
        .as_array()
        .ok_or("Missing Studio objects")?;
    let by: HashMap<_, _> = objects
        .iter()
        .filter_map(|o| o["id"].as_str().map(|id| (id, o)))
        .collect();
    let original_len = materials.len();
    let mut bindings = HashMap::<(String, usize), usize>::new();
    let mut palettes = HashMap::<(String, Arc<Vec<usize>>), Arc<Vec<usize>>>::new();
    for instance in instances {
        let owner = by
            .get(instance.layer.as_str())
            .ok_or("Material owner is unavailable")?;
        let prototype = owner["scatter"]["model"]
            .as_str()
            .and_then(|id| by.get(id))
            .copied()
            .unwrap_or(owner);
        let source_options = &prototype["materialOptions"]["p"];
        let options = if owner.get("materialOptions").is_some() {
            &owner["materialOptions"]["p"]
        } else {
            source_options
        };
        let casts = if owner.get("materialOptions").is_some() {
            val(&options["castsShadows"]) != "off"
        } else {
            owner["castShadow"] != false
                && prototype["castShadow"] != false
                && val(&source_options["castsShadows"]) != "off"
        };
        let receives = if owner.get("materialOptions").is_some() {
            val(&options["acceptsShadows"]) != false
        } else {
            owner["receiveShadow"] != false
                && prototype["receiveShadow"] != false
                && val(&source_options["acceptsShadows"]) != false
        };
        instance.primary_visible = val(&owner["materialOptions"]["p"]["castsShadows"]) != "only";
        let mut bind = |old: usize| -> Result<usize, String> {
            if old >= original_len {
                return Err("Material binding out of range".into());
            }
            let key = (instance.layer.clone(), old);
            if let Some(index) = bindings.get(&key) {
                return Ok(*index);
            }
            let mut gpu = materials[old];
            let mut flags = gpu.policy[1] as u32 & !(1 | 2 | 32);
            if casts {
                flags |= 1;
            }
            if receives {
                flags |= 2;
            }
            if val(&options["appearsInReflections"]) == false {
                flags |= 32;
            }
            let unlit = val(&options["acceptsLights"]) == false || flags & 8 != 0;
            if flags == gpu.policy[1] as u32 && !unlit {
                bindings.insert(key, old);
                return Ok(old);
            }
            if unlit {
                flags |= 8;
                gpu.base[3] = 0.;
                gpu.spec[0] = 1.;
                gpu.emit[..3].copy_from_slice(&gpu.base[..3]);
                // Keep base alpha sampling as well as unlit color emission.
                gpu.maps[1] = gpu.maps[0];
            }
            gpu.policy[1] = flags as f32;
            let index = materials.len();
            materials.push(gpu);
            scene.materials.push(scene.materials[old]);
            bindings.insert(key, index);
            Ok(index)
        };
        instance.material = bind(instance.material)?;
        if let Some(palette) = &instance.material_palette {
            let key = (instance.layer.clone(), palette.clone());
            let bound = if let Some(bound) = palettes.get(&key) {
                bound.clone()
            } else {
                let bound = Arc::new(
                    palette
                        .iter()
                        .map(|&old| bind(old))
                        .collect::<Result<Vec<_>, _>>()?,
                );
                palettes.insert(key, bound.clone());
                bound
            };
            instance.material_palette = Some(bound);
        } else if !instance.mesh.triangle_materials.is_empty() {
            let mut palette = vec![instance.material; original_len];
            for &old in &instance.mesh.triangle_materials {
                let slot = palette
                    .get_mut(old)
                    .ok_or("Face material binding out of range")?;
                *slot = bind(old)?;
            }
            instance.material_palette = Some(Arc::new(palette));
        }
    }
    Ok(())
}
