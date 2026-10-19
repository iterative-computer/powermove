//! `WebGLPrograms.getParameters` and `WebGLMaterials` for glTF draws:
//! `MeshStandardMaterial`, `MeshPhysicalMaterial`, `MeshBasicMaterial`,
//! `PointsMaterial`, and `LineBasicMaterial` on meshes, skinned meshes,
//! instanced meshes, points, and lines, with every map on its UV channel,
//! vertex colors, tangents, flat shading, alpha test, morph targets, and
//! the shadow pass's depth and distance materials for such objects.

use pm_geom3d::gltf::Mode;

use super::{MapSource, MeshExt, VertexExtra};
use crate::scene3d::describe::TextureRef;
use crate::scene3d::model::{Material3d, MaterialKind, Mesh3d};
use crate::three::materials::{LightCounts, Side};
use crate::three::program::{Parameters, ToneMapping, chunk};

/// Where a draw renders: the canvas (ACES Filmic, sRGB output) or the
/// transmission render target (no tone mapping, linear output).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Canvas,
    Transmission,
}

/// A material's map in `slot`: the object's scene map for the six slots
/// `sync` sets, else the file's.
pub fn map(material: &Material3d, slot: &str) -> Option<MapSource> {
    let scene = match slot {
        "map" => material.maps.color.as_ref(),
        "normalMap" => material.maps.normal.as_ref(),
        "roughnessMap" => material.maps.roughness.as_ref(),
        "metalnessMap" => material.maps.metalness.as_ref(),
        "emissiveMap" => material.maps.emissive.as_ref(),
        "aoMap" => material.maps.ao.as_ref(),
        _ => None,
    };
    if let Some(t) = scene {
        return Some(MapSource::Scene(t.clone()));
    }
    let ext = material.ext.as_ref()?;
    ext.map(slot).map(|m| MapSource::Gltf(m.clone()))
}

fn extra(mesh: &Mesh3d) -> Option<&VertexExtra> {
    mesh.geometry.extra.as_deref()
}

fn ext(mesh: &Mesh3d) -> Option<&MeshExt> {
    mesh.gltf.as_deref()
}

/// The material's physical state (`source` of a glTF material; three.js
/// defaults otherwise).
fn source(material: &Material3d) -> std::borrow::Cow<'_, pm_geom3d::gltf::Material> {
    match &material.ext {
        Some(e) => std::borrow::Cow::Borrowed(&e.source),
        None => std::borrow::Cow::Owned(pm_geom3d::gltf::Material::standard()),
    }
}

struct Channels(Vec<u32>);

impl Channels {
    /// `getChannel`.
    fn uv(&mut self, m: &Option<MapSource>) -> Option<String> {
        let c = m.as_ref()?.channel();
        if c == 0 {
            Some("uv".into())
        } else {
            if !self.0.contains(&c) {
                self.0.push(c);
            }
            Some(format!("uv{c}"))
        }
    }
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

/// The object-dependent parameters (skinning, morphs, instancing, vertex
/// attributes).
fn apply_object(p: &mut Parameters, mesh: &Mesh3d) {
    let Some(e) = ext(mesh) else {
        return;
    };
    p.skinning = e.skinned;
    p.instancing = e.instances.is_some();
    p.instancing_color = e
        .instances
        .as_ref()
        .is_some_and(|(_, i)| i.colors.is_some());
    if let Some(x) = extra(mesh)
        && let Some((position, normal, color, count, _)) = &x.morph
    {
        p.morph_targets = *position;
        p.morph_normals = *normal;
        p.morph_colors = *color;
        p.morph_targets_count = *count;
        p.morph_texture_stride = if *color {
            3
        } else if *normal {
            2
        } else if *position {
            1
        } else {
            0
        };
    }
}

/// `getParameters` for a glTF draw of `material` on `mesh`.
pub fn parameters(
    mesh: &Mesh3d,
    material: &Material3d,
    side: Side,
    lights: &LightCounts,
    target: Target,
) -> Result<Parameters, String> {
    let src = source(material);
    let x = extra(mesh);
    let points = matches!(x.map(|x| x.mode), Some(Mode::Points));
    let (shader_type, vs, fs, defines): (&str, &str, &str, Vec<(String, String)>) =
        match material.kind {
            MaterialKind::Standard => (
                "MeshStandardMaterial",
                "meshphysical_vert",
                "meshphysical_frag",
                vec![("STANDARD".into(), String::new())],
            ),
            MaterialKind::Physical => (
                "MeshPhysicalMaterial",
                "meshphysical_vert",
                "meshphysical_frag",
                vec![
                    ("STANDARD".into(), String::new()),
                    ("PHYSICAL".into(), String::new()),
                ],
            ),
            MaterialKind::Basic => (
                "MeshBasicMaterial",
                "meshbasic_vert",
                "meshbasic_frag",
                Vec::new(),
            ),
            MaterialKind::Points => ("PointsMaterial", "points_vert", "points_frag", Vec::new()),
            MaterialKind::Line => (
                "LineBasicMaterial",
                "meshbasic_vert",
                "meshbasic_frag",
                Vec::new(),
            ),
        };
    let canvas = target == Target::Canvas;
    let mut p = Parameters {
        shader_type: shader_type.into(),
        shader_name: material.name.clone(),
        defines,
        vertex_shader: chunk(vs).ok_or(vs)?.into(),
        fragment_shader: chunk(fs).ok_or(fs)?.into(),
        tone_mapping: if canvas {
            ToneMapping::AcesFilmic
        } else {
            ToneMapping::None
        },
        opaque: !material.transparent,
        output_srgb: canvas,
        ..Parameters::default()
    };
    let lit = matches!(
        material.kind,
        MaterialKind::Standard | MaterialKind::Physical
    );
    let physical = material.kind == MaterialKind::Physical;
    let mut ch = Channels(Vec::new());
    let m = |slot: &str| map(material, slot);
    let base = m("map");
    p.map = base.is_some();
    p.map_uv = ch.uv(&base);
    let alpha_test = src.alpha_test > 0.0;
    p.alpha_test = alpha_test;
    if lit {
        let set =
            |flag: &mut bool, uv: &mut Option<String>, ch: &mut Channels, mm: Option<MapSource>| {
                *flag = mm.is_some();
                *uv = ch.uv(&mm);
            };
        set(&mut p.ao_map, &mut p.ao_map_uv, &mut ch, m("aoMap"));
        set(&mut p.bump_map, &mut p.bump_map_uv, &mut ch, m("bumpMap"));
        set(
            &mut p.normal_map,
            &mut p.normal_map_uv,
            &mut ch,
            m("normalMap"),
        );
        p.normal_map_tangent_space = p.normal_map;
        set(
            &mut p.emissive_map,
            &mut p.emissive_map_uv,
            &mut ch,
            m("emissiveMap"),
        );
        set(
            &mut p.metalness_map,
            &mut p.metalness_map_uv,
            &mut ch,
            m("metalnessMap"),
        );
        set(
            &mut p.roughness_map,
            &mut p.roughness_map_uv,
            &mut ch,
            m("roughnessMap"),
        );
        if physical {
            p.anisotropy = src.anisotropy > 0.0;
            p.clearcoat = src.clearcoat > 0.0;
            p.dispersion = src.dispersion > 0.0;
            p.iridescence = src.iridescence > 0.0;
            p.sheen = src.sheen > 0.0;
            p.transmission = src.transmission > 0.0;
            let gated = |on: bool, mm: Option<MapSource>| if on { mm } else { None };
            set(
                &mut p.anisotropy_map,
                &mut p.anisotropy_map_uv,
                &mut ch,
                gated(p.anisotropy, m("anisotropyMap")),
            );
            set(
                &mut p.clearcoat_map,
                &mut p.clearcoat_map_uv,
                &mut ch,
                gated(p.clearcoat, m("clearcoatMap")),
            );
            set(
                &mut p.clearcoat_normal_map,
                &mut p.clearcoat_normal_map_uv,
                &mut ch,
                gated(p.clearcoat, m("clearcoatNormalMap")),
            );
            set(
                &mut p.clearcoat_roughness_map,
                &mut p.clearcoat_roughness_map_uv,
                &mut ch,
                gated(p.clearcoat, m("clearcoatRoughnessMap")),
            );
            set(
                &mut p.iridescence_map,
                &mut p.iridescence_map_uv,
                &mut ch,
                gated(p.iridescence, m("iridescenceMap")),
            );
            set(
                &mut p.iridescence_thickness_map,
                &mut p.iridescence_thickness_map_uv,
                &mut ch,
                gated(p.iridescence, m("iridescenceThicknessMap")),
            );
            set(
                &mut p.sheen_color_map,
                &mut p.sheen_color_map_uv,
                &mut ch,
                gated(p.sheen, m("sheenColorMap")),
            );
            set(
                &mut p.sheen_roughness_map,
                &mut p.sheen_roughness_map_uv,
                &mut ch,
                gated(p.sheen, m("sheenRoughnessMap")),
            );
            set(
                &mut p.specular_color_map,
                &mut p.specular_color_map_uv,
                &mut ch,
                m("specularColorMap"),
            );
            set(
                &mut p.specular_intensity_map,
                &mut p.specular_intensity_map_uv,
                &mut ch,
                m("specularIntensityMap"),
            );
            set(
                &mut p.transmission_map,
                &mut p.transmission_map_uv,
                &mut ch,
                gated(p.transmission, m("transmissionMap")),
            );
            set(
                &mut p.thickness_map,
                &mut p.thickness_map_uv,
                &mut ch,
                gated(p.transmission, m("thicknessMap")),
            );
        }
    }
    // The uv defines follow `getParameters`' order: mapUv, aoMapUv, ...
    // (`Channels` records which secondary sets are active).
    p.vertex_uv1s = ch.0.contains(&1);
    p.vertex_uv2s = ch.0.contains(&2);
    p.vertex_uv3s = ch.0.contains(&3);
    let has_tangent = x.is_some_and(|x| x.tangent.is_some());
    p.vertex_tangents = has_tangent && (p.normal_map || p.anisotropy);
    p.vertex_colors = src.vertex_colors;
    p.vertex_alphas =
        src.vertex_colors && x.and_then(|x| x.color.as_ref()).is_some_and(|c| c.1 == 4);
    p.points_uvs = points && x.is_some_and(|x| x.has_uv) && p.map;
    // `flatShading && wireframe === false`: points and line materials have no
    // `wireframe` property, so the comparison is false for them.
    p.flat_shading = src.flat_shading
        && !material.wireframe
        && !matches!(material.kind, MaterialKind::Points | MaterialKind::Line);
    p.size_attenuation = material.kind == MaterialKind::Points && src.size_attenuation;
    p.double_sided = side == Side::Double;
    p.flip_sided = side == Side::Back;
    apply_lights(&mut p, lights);
    apply_object(&mut p, mesh);
    Ok(p)
}

/// Whether three r180's *draw program* binds a source attribute. A color
/// material and its shadow depth/distance material can consume different
/// inputs even when both draws use the same mesh.
pub fn consumes_short_attribute(p: &Parameters, name: &str) -> bool {
    let shadow = matches!(
        p.shader_type.as_str(),
        "MeshDepthMaterial" | "MeshDistanceMaterial"
    );
    // Chromium keeps the copied map's vertex UV input active in a depth
    // program even without alpha testing. Fragment-only dead-code reasoning
    // does not predict the linked WebGL attributes (capture regression).
    let uv0 = if shadow {
        p.map_uv.as_deref() == Some("uv")
            || p.alpha_map_uv.as_deref() == Some("uv")
            || p.displacement_map_uv.as_deref() == Some("uv")
    } else {
        [
            &p.map_uv,
            &p.alpha_map_uv,
            &p.light_map_uv,
            &p.ao_map_uv,
            &p.emissive_map_uv,
            &p.bump_map_uv,
            &p.normal_map_uv,
            &p.displacement_map_uv,
            &p.metalness_map_uv,
            &p.roughness_map_uv,
            &p.anisotropy_map_uv,
            &p.clearcoat_map_uv,
            &p.clearcoat_normal_map_uv,
            &p.clearcoat_roughness_map_uv,
            &p.iridescence_map_uv,
            &p.iridescence_thickness_map_uv,
            &p.sheen_color_map_uv,
            &p.sheen_roughness_map_uv,
            &p.specular_map_uv,
            &p.specular_color_map_uv,
            &p.specular_intensity_map_uv,
            &p.transmission_map_uv,
            &p.thickness_map_uv,
        ]
        .into_iter()
        .any(|uv| uv.as_deref() == Some("uv"))
    };
    let secondary_uv = |channel: &str| {
        if shadow {
            p.map_uv.as_deref() == Some(channel)
                || p.alpha_map_uv.as_deref() == Some(channel)
                || p.displacement_map_uv.as_deref() == Some(channel)
        } else {
            match channel {
                "uv1" => p.vertex_uv1s,
                "uv2" => p.vertex_uv2s,
                "uv3" => p.vertex_uv3s,
                _ => false,
            }
        }
    };
    match name {
        "NORMAL" => {
            if shadow {
                p.displacement_map
            } else {
                matches!(
                    p.shader_type.as_str(),
                    "MeshStandardMaterial" | "MeshPhysicalMaterial"
                ) && !p.flat_shading
            }
        }
        "TANGENT" => !shadow && p.vertex_tangents && !p.flat_shading,
        // The depth and distance shaders include no color chunks, including
        // when alpha testing their copied map/alphaMap.
        "COLOR_0" => !shadow && p.vertex_colors,
        "TEXCOORD_0" => uv0 || p.points_uvs,
        "TEXCOORD_1" => secondary_uv("uv1"),
        "TEXCOORD_2" => secondary_uv("uv2"),
        "TEXCOORD_3" => secondary_uv("uv3"),
        _ => false,
    }
}

fn push_map(
    u: &mut Vec<(String, Vec<f64>)>,
    t: &mut Vec<(String, TextureRef)>,
    name: &str,
    transform_name: &str,
    m: &MapSource,
) {
    t.push((
        name.into(),
        match m {
            MapSource::Scene(s) => TextureRef::Map(s.clone()),
            MapSource::Gltf(g) => TextureRef::Gltf(g.clone()),
        },
    ));
    u.push((transform_name.into(), m.transform().to_vec()));
}

/// `refreshMaterialUniforms` for a glTF draw. `size` is the transmission
/// target size, `height` the canvas height.
pub fn uniforms(
    u: &mut Vec<(String, Vec<f64>)>,
    t: &mut Vec<(String, TextureRef)>,
    material: &Material3d,
    side: Side,
    height: u32,
    size: (u32, u32),
) {
    let src = source(material);
    let back = side == Side::Back;
    let m = |slot: &str| map(material, slot);
    match material.kind {
        MaterialKind::Points => {
            u.push(("diffuse".into(), material.color.to_vec()));
            u.push(("opacity".into(), vec![material.opacity]));
            u.push(("size".into(), vec![1.0]));
            u.push(("scale".into(), vec![f64::from(height) * 0.5]));
            if let Some(mm) = m("map") {
                push_map(u, t, "map", "uvTransform", &mm);
            }
            if src.alpha_test > 0.0 {
                u.push(("alphaTest".into(), vec![src.alpha_test]));
            }
            return;
        }
        MaterialKind::Line => {
            u.push(("diffuse".into(), material.color.to_vec()));
            u.push(("opacity".into(), vec![material.opacity]));
            if let Some(mm) = m("map") {
                push_map(u, t, "map", "mapTransform", &mm);
            }
            return;
        }
        _ => {}
    }
    // `refreshUniformsCommon`.
    u.push(("opacity".into(), vec![material.opacity]));
    u.push(("diffuse".into(), material.color.to_vec()));
    let lit = matches!(
        material.kind,
        MaterialKind::Standard | MaterialKind::Physical
    );
    if lit {
        u.push((
            "emissive".into(),
            material
                .emissive
                .iter()
                .map(|v| v * material.emissive_intensity)
                .collect(),
        ));
    }
    if let Some(mm) = m("map") {
        push_map(u, t, "map", "mapTransform", &mm);
    }
    if lit {
        if let Some(mm) = m("bumpMap") {
            push_map(u, t, "bumpMap", "bumpMapTransform", &mm);
            let s = if back {
                -src.bump_scale
            } else {
                src.bump_scale
            };
            u.push(("bumpScale".into(), vec![s]));
        }
        if let Some(mm) = m("normalMap") {
            push_map(u, t, "normalMap", "normalMapTransform", &mm);
            let ns = if material.maps.normal.is_some() && material.ext.is_none() {
                [1.0, 1.0]
            } else {
                src.normal_scale
            };
            let ns = if back { [-ns[0], -ns[1]] } else { ns };
            u.push(("normalScale".into(), ns.to_vec()));
        }
        if let Some(mm) = m("emissiveMap") {
            push_map(u, t, "emissiveMap", "emissiveMapTransform", &mm);
        }
    }
    if src.alpha_test > 0.0 {
        u.push(("alphaTest".into(), vec![src.alpha_test]));
    }
    if lit && let Some(mm) = m("aoMap") {
        u.push(("aoMapIntensity".into(), vec![src.ao_map_intensity]));
        push_map(u, t, "aoMap", "aoMapTransform", &mm);
    }
    if !lit {
        return;
    }
    // `refreshUniformsStandard`.
    u.push(("metalness".into(), vec![material.metalness]));
    if let Some(mm) = m("metalnessMap") {
        push_map(u, t, "metalnessMap", "metalnessMapTransform", &mm);
    }
    u.push(("roughness".into(), vec![material.roughness]));
    if let Some(mm) = m("roughnessMap") {
        push_map(u, t, "roughnessMap", "roughnessMapTransform", &mm);
    }
    u.push(("envMapIntensity".into(), vec![1.0]));
    if material.kind != MaterialKind::Physical {
        return;
    }
    // `refreshUniformsPhysical`.
    u.push(("ior".into(), vec![src.ior]));
    if src.sheen > 0.0 {
        u.push((
            "sheenColor".into(),
            src.sheen_color.iter().map(|c| c * src.sheen).collect(),
        ));
        u.push(("sheenRoughness".into(), vec![src.sheen_roughness]));
        if let Some(mm) = m("sheenColorMap") {
            push_map(u, t, "sheenColorMap", "sheenColorMapTransform", &mm);
        }
        if let Some(mm) = m("sheenRoughnessMap") {
            push_map(u, t, "sheenRoughnessMap", "sheenRoughnessMapTransform", &mm);
        }
    }
    if src.clearcoat > 0.0 {
        u.push(("clearcoat".into(), vec![src.clearcoat]));
        u.push(("clearcoatRoughness".into(), vec![src.clearcoat_roughness]));
        if let Some(mm) = m("clearcoatMap") {
            push_map(u, t, "clearcoatMap", "clearcoatMapTransform", &mm);
        }
        if let Some(mm) = m("clearcoatRoughnessMap") {
            push_map(
                u,
                t,
                "clearcoatRoughnessMap",
                "clearcoatRoughnessMapTransform",
                &mm,
            );
        }
        if let Some(mm) = m("clearcoatNormalMap") {
            push_map(
                u,
                t,
                "clearcoatNormalMap",
                "clearcoatNormalMapTransform",
                &mm,
            );
            let s = src.clearcoat_normal_scale;
            let s = if back { [-s[0], -s[1]] } else { s };
            u.push(("clearcoatNormalScale".into(), s.to_vec()));
        }
    }
    if src.dispersion > 0.0 {
        u.push(("dispersion".into(), vec![src.dispersion]));
    }
    if src.iridescence > 0.0 {
        u.push(("iridescence".into(), vec![src.iridescence]));
        u.push(("iridescenceIOR".into(), vec![src.iridescence_ior]));
        u.push((
            "iridescenceThicknessMinimum".into(),
            vec![src.iridescence_thickness_range[0]],
        ));
        u.push((
            "iridescenceThicknessMaximum".into(),
            vec![src.iridescence_thickness_range[1]],
        ));
        if let Some(mm) = m("iridescenceMap") {
            push_map(u, t, "iridescenceMap", "iridescenceMapTransform", &mm);
        }
        if let Some(mm) = m("iridescenceThicknessMap") {
            push_map(
                u,
                t,
                "iridescenceThicknessMap",
                "iridescenceThicknessMapTransform",
                &mm,
            );
        }
    }
    if src.transmission > 0.0 {
        u.push(("transmission".into(), vec![src.transmission]));
        t.push(("transmissionSamplerMap".into(), TextureRef::Transmission));
        u.push((
            "transmissionSamplerSize".into(),
            vec![f64::from(size.0), f64::from(size.1)],
        ));
        if let Some(mm) = m("transmissionMap") {
            push_map(u, t, "transmissionMap", "transmissionMapTransform", &mm);
        }
        u.push(("thickness".into(), vec![src.thickness]));
        if let Some(mm) = m("thicknessMap") {
            push_map(u, t, "thicknessMap", "thicknessMapTransform", &mm);
        }
        u.push(("attenuationDistance".into(), vec![src.attenuation_distance]));
        u.push(("attenuationColor".into(), src.attenuation_color.to_vec()));
    }
    if src.anisotropy > 0.0 {
        u.push((
            "anisotropyVector".into(),
            vec![
                src.anisotropy * libm::cos(src.anisotropy_rotation),
                src.anisotropy * libm::sin(src.anisotropy_rotation),
            ],
        ));
        if let Some(mm) = m("anisotropyMap") {
            push_map(u, t, "anisotropyMap", "anisotropyMapTransform", &mm);
        }
    }
    u.push(("specularIntensity".into(), vec![src.specular_intensity]));
    u.push(("specularColor".into(), src.specular_color.to_vec()));
    if let Some(mm) = m("specularColorMap") {
        push_map(u, t, "specularColorMap", "specularColorMapTransform", &mm);
    }
    if let Some(mm) = m("specularIntensityMap") {
        push_map(
            u,
            t,
            "specularIntensityMap",
            "specularIntensityMapTransform",
            &mm,
        );
    }
}

/// Skinning and morph uniforms of an object (`setProgram`,
/// `WebGLMorphtargets.update`).
pub fn object_uniforms(
    u: &mut Vec<(String, Vec<f64>)>,
    t: &mut Vec<(String, TextureRef)>,
    mesh: &Mesh3d,
) {
    let Some(e) = ext(mesh) else {
        return;
    };
    if let Some(s) = &e.skinning {
        u.push(("bindMatrix".into(), s.bind_matrix.to_vec()));
        u.push(("bindMatrixInverse".into(), s.bind_matrix_inverse.to_vec()));
        t.push((
            "boneTexture".into(),
            TextureRef::Data {
                key: s.key,
                width: s.size,
                height: s.size,
                layers: 1,
                array: false,
                data: s.bones.clone(),
            },
        ));
    }
    if let Some(x) = extra(mesh)
        && let Some((_, _, _, count, texture)) = &x.morph
    {
        let influences: Vec<f64> = (0..*count as usize)
            .map(|k| e.influences.get(k).copied().unwrap_or(0.0))
            .collect();
        // Relative targets: the base influence is 1.
        u.push(("morphTargetBaseInfluence".into(), vec![1.0]));
        for (k, v) in influences.iter().enumerate() {
            u.push((format!("morphTargetInfluences[{k}]"), vec![*v]));
        }
        u.push((
            "morphTargetsTextureSize".into(),
            vec![f64::from(texture.width), f64::from(texture.height)],
        ));
        t.push((
            "morphTargetsTexture".into(),
            TextureRef::Data {
                key: texture.key,
                width: texture.width,
                height: texture.height,
                layers: texture.layers,
                array: true,
                data: texture.data.clone(),
            },
        ));
    }
}

/// `WebGLShadowMap.getDepthMaterial` for a glTF object: the shared depth
/// (or distance) material with the caster's side, alpha test, and map,
/// and the object's skinning, morphs, and instancing.
pub fn shadow_parameters(
    mesh: &Mesh3d,
    material: &Material3d,
    side: Side,
    point: bool,
    lights: &LightCounts,
) -> Result<Parameters, String> {
    let src = source(material);
    let base = map(material, "map");
    let mut p = if point {
        Parameters {
            shader_type: "MeshDistanceMaterial".into(),
            vertex_shader: chunk("distanceRGBA_vert")
                .ok_or("distanceRGBA_vert")?
                .into(),
            fragment_shader: chunk("distanceRGBA_frag")
                .ok_or("distanceRGBA_frag")?
                .into(),
            ..Parameters::default()
        }
    } else {
        Parameters {
            shader_type: "MeshDepthMaterial".into(),
            vertex_shader: chunk("depth_vert").ok_or("depth_vert")?.into(),
            fragment_shader: chunk("depth_frag").ok_or("depth_frag")?.into(),
            use_depth_packing: true,
            depth_packing: 3201,
            ..Parameters::default()
        }
    };
    p.tone_mapping = ToneMapping::None;
    p.opaque = true;
    p.output_srgb = false;
    p.map = base.is_some();
    let mut ch = Channels(Vec::new());
    p.map_uv = ch.uv(&base);
    p.alpha_test = src.alpha_test > 0.0;
    p.vertex_uv1s = ch.0.contains(&1);
    p.vertex_uv2s = ch.0.contains(&2);
    p.vertex_uv3s = ch.0.contains(&3);
    p.double_sided = side == Side::Double;
    p.flip_sided = side == Side::Back;
    apply_lights(&mut p, lights);
    apply_object(&mut p, mesh);
    Ok(p)
}

/// The shadow material's uniforms for a glTF caster (`alphaTest`, `map`).
pub fn shadow_uniforms(
    u: &mut Vec<(String, Vec<f64>)>,
    t: &mut Vec<(String, TextureRef)>,
    material: &Material3d,
) {
    let src = source(material);
    if let Some(mm) = map(material, "map") {
        push_map(u, t, "map", "mapTransform", &mm);
    }
    if src.alpha_test > 0.0 {
        u.push(("alphaTest".into(), vec![src.alpha_test]));
    }
}
