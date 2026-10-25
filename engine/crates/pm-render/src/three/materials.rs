//! `WebGLPrograms.getParameters` for the materials the 3D pass draws:
//! `MeshStandardMaterial` (primitives, meshes, slots), `MeshBasicMaterial`
//! (the missing-model box), and the shadow pass's `MeshDepthMaterial`
//! and `MeshDistanceMaterial` (point lights). Each returns
//! the [`Parameters`] three.js r180 derives for the same material, object,
//! light state, and render target, so [`super::program::sources`] gives
//! the program text WebGL receives.

use super::program::{Parameters, ToneMapping, chunk};

/// `material.side` (after `WebGLShadowMap`'s `shadowSide` mapping for
/// shadow materials).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Front,
    Back,
    Double,
}

impl Side {
    /// `shadowSide[side]` in `WebGLShadowMap.getDepthMaterial`.
    pub fn shadow(self) -> Side {
        match self {
            Side::Front => Side::Back,
            Side::Back => Side::Front,
            Side::Double => Side::Double,
        }
    }
}

/// The scene's light state as `WebGLLights` counts it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct LightCounts {
    pub directional: u32,
    pub point: u32,
    pub spot: u32,
    pub rect_area: u32,
    pub hemi: u32,
    pub directional_shadows: u32,
    pub point_shadows: u32,
    pub spot_shadows: u32,
    /// `renderer.shadowMap.enabled && shadows.length > 0`.
    pub shadow_map: bool,
}

/// Which `MeshStandardMaterial` maps are set (all on UV channel 0).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Maps {
    pub color: bool,
    pub normal: bool,
    pub roughness: bool,
    pub metalness: bool,
    pub emissive: bool,
    pub ao: bool,
}

impl Maps {
    pub fn any(&self) -> bool {
        self.color || self.normal || self.roughness || self.metalness || self.emissive || self.ao
    }
}

fn uv(on: bool) -> Option<String> {
    on.then(|| "uv".to_owned())
}

fn apply_lights(p: &mut Parameters, l: &LightCounts) {
    p.num_dir_lights = l.directional;
    p.num_point_lights = l.point;
    p.num_spot_lights = l.spot;
    p.num_rect_area_lights = l.rect_area;
    p.num_hemi_lights = l.hemi;
    p.num_dir_light_shadows = l.directional_shadows;
    p.num_point_light_shadows = l.point_shadows;
    p.num_spot_light_shadows = l.spot_shadows;
    p.shadow_map_enabled = l.shadow_map;
}

fn apply_side(p: &mut Parameters, side: Side) {
    p.double_sided = side == Side::Double;
    p.flip_sided = side == Side::Back;
}

/// A `MeshStandardMaterial` drawn in linear light; the scene output pass
/// applies ACES Filmic and encoding after translucency and MSAA resolve.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Standard {
    pub name: String,
    pub side: Side,
    pub transparent: bool,
    pub maps: Maps,
    pub lights: LightCounts,
}

pub fn standard(m: &Standard) -> Result<Parameters, String> {
    let mut p = Parameters {
        shader_type: "MeshStandardMaterial".into(),
        shader_name: m.name.clone(),
        defines: vec![("STANDARD".into(), String::new())],
        vertex_shader: chunk("meshphysical_vert")
            .ok_or("meshphysical_vert")?
            .into(),
        fragment_shader: chunk("meshphysical_frag")
            .ok_or("meshphysical_frag")?
            .into(),
        tone_mapping: ToneMapping::None,
        opaque: !m.transparent,
        output_srgb: false,
        map: m.maps.color,
        map_uv: uv(m.maps.color),
        normal_map: m.maps.normal,
        normal_map_tangent_space: m.maps.normal,
        normal_map_uv: uv(m.maps.normal),
        roughness_map: m.maps.roughness,
        roughness_map_uv: uv(m.maps.roughness),
        metalness_map: m.maps.metalness,
        metalness_map_uv: uv(m.maps.metalness),
        emissive_map: m.maps.emissive,
        emissive_map_uv: uv(m.maps.emissive),
        ao_map: m.maps.ao,
        ao_map_uv: uv(m.maps.ao),
        ..Parameters::default()
    };
    apply_side(&mut p, m.side);
    apply_lights(&mut p, &m.lights);
    Ok(p)
}

/// A `MeshBasicMaterial` drawn to the canvas.
pub fn basic(
    name: &str,
    side: Side,
    transparent: bool,
    lights: &LightCounts,
) -> Result<Parameters, String> {
    let mut p = Parameters {
        shader_type: "MeshBasicMaterial".into(),
        shader_name: name.into(),
        vertex_shader: chunk("meshbasic_vert").ok_or("meshbasic_vert")?.into(),
        fragment_shader: chunk("meshbasic_frag").ok_or("meshbasic_frag")?.into(),
        tone_mapping: ToneMapping::None,
        opaque: !transparent,
        output_srgb: false,
        ..Parameters::default()
    };
    apply_side(&mut p, side);
    apply_lights(&mut p, lights);
    Ok(p)
}

/// Depth-only caster fragments retain r180's map and alpha rejection; the
/// hardware depth attachment stores full-precision depth.
pub fn shadow_fragment(point: bool) -> &'static str {
    if point {
        "#define DISTANCE\nuniform vec3 referencePosition;\nuniform float nearDistance;\nuniform float farDistance;\nvarying vec3 vWorldPosition;\n#include <common>\n#include <uv_pars_fragment>\n#include <map_pars_fragment>\n#include <alphamap_pars_fragment>\n#include <alphatest_pars_fragment>\n#include <alphahash_pars_fragment>\n#include <clipping_planes_pars_fragment>\nvoid main() {\n vec4 diffuseColor = vec4(1.0);\n #include <clipping_planes_fragment>\n #include <map_fragment>\n #include <alphamap_fragment>\n #include <alphatest_fragment>\n #include <alphahash_fragment>\n float dist = length(vWorldPosition - referencePosition);\n gl_FragDepth = clamp((dist - nearDistance) / (farDistance - nearDistance), 0.0, 1.0);\n}\n"
    } else {
        "#include <common>\n#include <uv_pars_fragment>\n#include <map_pars_fragment>\n#include <alphamap_pars_fragment>\n#include <alphatest_pars_fragment>\n#include <alphahash_pars_fragment>\n#include <clipping_planes_pars_fragment>\nvoid main() {\n vec4 diffuseColor = vec4(1.0);\n #include <clipping_planes_fragment>\n #include <map_fragment>\n #include <alphamap_fragment>\n #include <alphatest_fragment>\n #include <alphahash_fragment>\n gl_FragDepth = gl_FragCoord.z;\n}\n"
    }
}

/// `MeshDepthMaterial` drawn into a depth-only shadow map. `side` is
/// already the shadow side; `map` is whether the caster's material has a
/// color map (`getDepthMaterial` copies `material.map`; the depth fragment
/// samples it but packs only depth).
pub fn depth_rgba(side: Side, map: bool, lights: &LightCounts) -> Result<Parameters, String> {
    let mut p = Parameters {
        shader_type: "MeshDepthMaterial".into(),
        shader_name: String::new(),
        vertex_shader: chunk("depth_vert").ok_or("depth_vert")?.into(),
        fragment_shader: shadow_fragment(false).into(),
        tone_mapping: ToneMapping::None,
        opaque: true,
        output_srgb: false,
        map,
        map_uv: uv(map),
        ..Parameters::default()
    };
    apply_side(&mut p, side);
    apply_lights(&mut p, lights);
    Ok(p)
}

/// `MeshDistanceMaterial` drawn into a point light's shadow atlas (`map` as
/// for [`depth_rgba`]).
pub fn distance(side: Side, map: bool, lights: &LightCounts) -> Result<Parameters, String> {
    let mut p = Parameters {
        shader_type: "MeshDistanceMaterial".into(),
        shader_name: String::new(),
        vertex_shader: chunk("distanceRGBA_vert")
            .ok_or("distanceRGBA_vert")?
            .into(),
        fragment_shader: shadow_fragment(true).into(),
        tone_mapping: ToneMapping::None,
        opaque: true,
        output_srgb: false,
        map,
        map_uv: uv(map),
        ..Parameters::default()
    };
    apply_side(&mut p, side);
    apply_lights(&mut p, lights);
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::three::program::sources;

    #[test]
    fn every_material_resolves() {
        let lights = LightCounts {
            directional: 2,
            point: 1,
            spot: 1,
            rect_area: 1,
            hemi: 1,
            directional_shadows: 1,
            point_shadows: 1,
            spot_shadows: 1,
            shadow_map: true,
        };
        let m = Standard {
            name: "Surface".into(),
            side: Side::Back,
            transparent: true,
            maps: Maps {
                color: true,
                normal: true,
                roughness: true,
                metalness: true,
                emissive: true,
                ao: true,
            },
            lights,
        };
        let (v, f) = sources(&standard(&m).unwrap()).unwrap();
        assert!(v.contains("#define USE_SHADOWMAP"));
        assert!(f.contains("#define FLIP_SIDED"));
        assert!(!f.contains("#define OPAQUE"));
        assert!(f.contains("#define MAP_UV uv") || v.contains("#define MAP_UV uv"));
        assert!(f.contains("rectAreaLights[ 0 ]"));
        for p in [
            basic("", Side::Front, false, &lights).unwrap(),
            depth_rgba(Side::Double, true, &lights).unwrap(),
            distance(Side::Back, false, &lights).unwrap(),
        ] {
            let (v, f) = sources(&p).unwrap();
            assert!(!v.contains("#include") && !f.contains("#include"));
        }
    }
}
