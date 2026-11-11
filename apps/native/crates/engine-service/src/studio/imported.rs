//! The pinned 24-scalar / 18-map physical-material contract, without UI state.
use super::*;
use pm_scene3d::render::gltf::{MapRef, Material as Imported, MaterialType, Wrap};
use pm_studio3d_engine::scene::GpuTextureMap;

/// Bounded decoded-pixel reuse survives immutable document-source replacement.
pub(super) fn image(
    resource: &pm_scene3d::render::Resource,
    texture: usize,
) -> Result<Arc<Vec<u8>>, String> {
    static CACHE: std::sync::OnceLock<Mutex<Vec<((String, usize), Arc<Vec<u8>>)>>> =
        std::sync::OnceLock::new();
    let pm_scene3d::import::Model::Gltf(model) = &resource.model else {
        return Err("Imported maps require glTF".into());
    };
    let image = model
        .textures
        .get(texture)
        .ok_or("Missing imported texture")?
        .image;
    let key = (resource.key.clone(), image);
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "Imported image cache poisoned")?;
    if !resource.key.is_empty() {
        if let Some(i) = cache.iter().position(|(k, _)| *k == key) {
            let entry = cache.remove(i);
            let pixels = entry.1.clone();
            cache.push(entry);
            return Ok(pixels);
        }
    }
    let pixels = Arc::new(resource.texture(texture).map_err(|e| e.to_string())?);
    if !resource.key.is_empty() {
        cache.push((key, pixels.clone()));
        if cache.len() > 16 {
            cache.remove(0);
        }
    }
    Ok(pixels)
}

pub(super) fn pack(
    m: &Imported,
    gpu: &mut GpuMaterial,
    mut image: impl FnMut(usize) -> Result<(usize, Wrap, Wrap), String>,
) -> Result<(), String> {
    gpu.imported = [
        [
            m.specular_color[0] as f32,
            m.specular_color[1] as f32,
            m.specular_color[2] as f32,
            m.sheen_roughness as f32,
        ],
        [
            m.sheen_color[0] as f32,
            m.sheen_color[1] as f32,
            m.sheen_color[2] as f32,
            m.iridescence as f32,
        ],
        [
            m.attenuation_color[0] as f32,
            m.attenuation_color[1] as f32,
            m.attenuation_color[2] as f32,
            if m.attenuation_distance.is_finite() {
                m.attenuation_distance as f32
            } else {
                0.
            },
        ],
        [
            m.thickness as f32,
            m.anisotropy_rotation as f32,
            m.iridescence_ior as f32,
            m.iridescence_thickness_range[0] as f32,
        ],
        [
            m.iridescence_thickness_range[1] as f32,
            m.clearcoat_normal_scale[0] as f32,
            m.clearcoat_normal_scale[1] as f32,
            m.ao_map_intensity as f32,
        ],
        [m.normal_scale[1] as f32, m.normal_scale[0] as f32, 0., 1.],
    ];
    let basic = m.kind == MaterialType::Basic;
    let maps: [Option<&MapRef>; 18] = [
        m.map.as_ref(),
        if basic {
            m.map.as_ref()
        } else {
            m.emissive_map.as_ref()
        },
        m.roughness_map.as_ref(),
        m.metalness_map.as_ref(),
        m.normal_map.as_ref(),
        m.ao_map.as_ref(),
        m.clearcoat_map.as_ref(),
        m.clearcoat_roughness_map.as_ref(),
        m.clearcoat_normal_map.as_ref(),
        m.transmission_map.as_ref(),
        m.thickness_map.as_ref(),
        m.specular_intensity_map.as_ref(),
        m.specular_color_map.as_ref(),
        m.sheen_color_map.as_ref(),
        m.sheen_roughness_map.as_ref(),
        m.iridescence_map.as_ref(),
        m.iridescence_thickness_map.as_ref(),
        m.anisotropy_map.as_ref(),
    ];
    let wrap = |w| match w {
        Wrap::Repeat => 0.,
        Wrap::Clamp => 1.,
        Wrap::Mirror => 2.,
    };
    for (slot, map) in maps.into_iter().enumerate() {
        gpu.physical_maps[slot] = GpuTextureMap {
            info: [-1., 0., 0., 0.],
            u: [1., 0., 0., 0.],
            v: [0., 1., 0., 0.],
        };
        let Some(map) = map else {
            continue;
        };
        if map.channel > 1 {
            return Err("Studio supports TEXCOORD_0 and TEXCOORD_1 only".into());
        }
        let (index, s, t) = image(map.texture)?;
        let e = map.transform;
        gpu.physical_maps[slot] = GpuTextureMap {
            info: [index as f32, map.channel as f32, wrap(s), wrap(t)],
            u: [e[0] as f32, e[3] as f32, e[6] as f32, 0.],
            v: [e[1] as f32, e[4] as f32, e[7] as f32, 0.],
        };
    }
    let index = |slot: usize| gpu.physical_maps[slot].info[0];
    gpu.maps = [
        index(0),
        index(1),
        if index(2) >= 0. { index(2) } else { index(3) },
        index(4),
    ];
    gpu.maps2 = [index(2), index(3), index(5), m.normal_scale[0] as f32];
    gpu.policy[2] = m.alpha_test as f32;
    if basic {
        gpu.policy[1] = (gpu.policy[1] as u32 | 8) as f32;
    }
    if !m.double_sided {
        gpu.policy[1] = (gpu.policy[1] as u32 & !4) as f32;
    }
    gpu.validate()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_eighteen_slots_keep_sampler_uv_and_physical_scalars() {
        let map = MapRef {
            texture: 3,
            channel: 1,
            transform: [2., 0., 0., 0., 3., 0., 0.25, 0.75, 1.],
        };
        let mut m = Imported::standard();
        m.map = Some(map.clone());
        m.emissive_map = Some(map.clone());
        m.roughness_map = Some(map.clone());
        m.metalness_map = Some(map.clone());
        m.normal_map = Some(map.clone());
        m.ao_map = Some(map.clone());
        m.clearcoat_map = Some(map.clone());
        m.clearcoat_roughness_map = Some(map.clone());
        m.clearcoat_normal_map = Some(map.clone());
        m.transmission_map = Some(map.clone());
        m.thickness_map = Some(map.clone());
        m.specular_intensity_map = Some(map.clone());
        m.specular_color_map = Some(map.clone());
        m.sheen_color_map = Some(map.clone());
        m.sheen_roughness_map = Some(map.clone());
        m.iridescence_map = Some(map.clone());
        m.iridescence_thickness_map = Some(map.clone());
        m.anisotropy_map = Some(map);
        m.normal_scale = [0.4, -0.7];
        m.clearcoat_normal_scale = [0.2, 0.3];
        m.attenuation_color = [0.1, 0.2, 0.3];
        m.attenuation_distance = 2.;
        m.thickness = 0.8;
        m.iridescence = 0.4;
        m.iridescence_thickness_range = [130., 420.];
        m.specular_color = [0.7, 0.8, 0.9];
        m.sheen_color = [0.1, 0.3, 0.5];
        m.sheen_roughness = 0.6;
        let mut gpu = imported_material(&m).gpu();
        pack(&m, &mut gpu, |index| {
            assert_eq!(index, 3);
            Ok((7, Wrap::Clamp, Wrap::Mirror))
        })
        .unwrap();
        assert!(
            gpu.physical_maps
                .iter()
                .all(|map| map.info == [7., 1., 1., 2.]
                    && map.u == [2., 0., 0.25, 0.]
                    && map.v == [0., 3., 0.75, 0.])
        );
        assert_eq!(gpu.imported[0], [0.7, 0.8, 0.9, 0.6]);
        assert_eq!(gpu.imported[2], [0.1, 0.2, 0.3, 2.]);
        assert_eq!(gpu.imported[4], [420., 0.2, 0.3, 1.]);
        assert_eq!(gpu.imported[5], [-0.7, 0.4, 0., 1.]);
    }
    #[test]
    fn unlit_uses_base_color_map_and_missing_maps_never_sample_layer_zero() {
        let mut m = Imported::standard();
        m.kind = MaterialType::Basic;
        m.map = Some(MapRef {
            texture: 4,
            channel: 0,
            transform: [1., 0., 0., 0., 1., 0., 0., 0., 1.],
        });
        let mut gpu = imported_material(&m).gpu();
        pack(&m, &mut gpu, |_| Ok((2, Wrap::Repeat, Wrap::Repeat))).unwrap();
        assert_eq!(gpu.physical_maps[0].info[0], 2.);
        assert_eq!(gpu.physical_maps[1].info[0], 2.);
        assert!(gpu.physical_maps[2..].iter().all(|map| map.info[0] == -1.));
        m.map.as_mut().unwrap().channel = 2;
        assert!(pack(&m, &mut gpu, |_| Ok((2, Wrap::Repeat, Wrap::Repeat))).is_err());
    }
}
