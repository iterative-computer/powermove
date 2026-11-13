//! FBX conversion shared by Electron and native Studio. Hosts run this on a
//! worker with an 8 MiB stack, never in a UI callback. No external file reads.
//! The durable result is ordinary glTF: parts, PBR materials, skins and baked TRS.
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Instant;

const MAX_BYTES: usize = 256 * 1024 * 1024;
const MAX_TRIANGLES: usize = 10_000_000;
const MAX_NODES: usize = 65_536;

pub struct Imported {
    pub glb: Vec<u8>,
    pub stats: Value,
}

#[derive(Default)]
struct Gltf {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
    images: Vec<Value>,
    textures: Vec<Value>,
    texture_indices: HashMap<u32, usize>,
    warnings: Vec<String>,
}
impl Gltf {
    fn view(&mut self, bytes: &[u8]) -> Result<usize, String> {
        while self.bin.len() % 4 != 0 {
            self.bin.push(0);
        }
        if self.bin.len().saturating_add(bytes.len()) > MAX_BYTES {
            return Err("Converted FBX exceeds 256 MB".into());
        }
        let index = self.views.len();
        self.views
            .push(json!({"buffer":0,"byteOffset":self.bin.len(),"byteLength":bytes.len()}));
        self.bin.extend_from_slice(bytes);
        Ok(index)
    }
    fn floats(&mut self, data: &[f32], kind: &str, width: usize) -> Result<usize, String> {
        if data.is_empty() || data.len() % width != 0 || data.iter().any(|v| !v.is_finite()) {
            return Err("FBX contains invalid numeric data".into());
        }
        let view = self.view(bytemuck::cast_slice(data))?;
        let index = self.accessors.len();
        let lo: Vec<f32> = (0..width)
            .map(|i| {
                data.chunks_exact(width)
                    .map(|p| p[i])
                    .fold(f32::INFINITY, f32::min)
            })
            .collect();
        let hi: Vec<f32> = (0..width)
            .map(|i| {
                data.chunks_exact(width)
                    .map(|p| p[i])
                    .fold(f32::NEG_INFINITY, f32::max)
            })
            .collect();
        self.accessors.push(json!({"bufferView":view,"componentType":5126,"count":data.len()/width,"type":kind,"min":lo,"max":hi}));
        Ok(index)
    }
    fn joints(&mut self, data: &[u16]) -> Result<usize, String> {
        let view = self.view(bytemuck::cast_slice(data))?;
        let index = self.accessors.len();
        self.accessors.push(
            json!({"bufferView":view,"componentType":5123,"count":data.len()/4,"type":"VEC4"}),
        );
        Ok(index)
    }
    fn texture(
        &mut self,
        map: &ufbx::MaterialMap,
        mesh: Option<&ufbx::Mesh>,
    ) -> Result<Option<Value>, String> {
        let Some(texture) = map.texture.as_ref().filter(|_| map.texture_enabled) else {
            return Ok(None);
        };
        let content: &[u8] = if !texture.content.is_empty() {
            &texture.content
        } else if let Some(video) = &texture.video {
            &video.content
        } else {
            &[]
        };
        if content.is_empty() {
            let warning = "External FBX textures are not read; embed PNG/JPEG images before import"
                .to_string();
            if !self.warnings.contains(&warning) {
                self.warnings.push(warning);
            }
            return Ok(None);
        }
        let index = if let Some(index) = self.texture_indices.get(&texture.element.typed_id) {
            *index
        } else {
            let mime = if content.starts_with(b"\x89PNG\r\n\x1a\n") {
                "image/png"
            } else if content.starts_with(&[0xff, 0xd8, 0xff]) {
                "image/jpeg"
            } else {
                self.warnings
                    .push("Unsupported embedded FBX image; use PNG or JPEG".into());
                return Ok(None);
            };
            let view = self.view(content)?;
            let index = self.textures.len();
            self.images.push(json!({"bufferView":view,"mimeType":mime}));
            self.textures
                .push(json!({"source":self.images.len()-1,"sampler":0}));
            self.texture_indices.insert(texture.element.typed_id, index);
            index
        };
        let uv = mesh
            .and_then(|m| {
                m.uv_sets
                    .iter()
                    .position(|uv| uv.name.as_ref() == texture.uv_set.as_ref())
            })
            .unwrap_or(0)
            .min(1);
        let mut info = json!({"index":index,"texCoord":uv});
        if texture.has_uv_transform {
            let t = &texture.uv_transform;
            let q = &t.rotation;
            let angle = 2.0 * q.z.atan2(q.w);
            info["extensions"] = json!({"KHR_texture_transform":{"offset":[t.translation.x,t.translation.y],"scale":[t.scale.x,t.scale.y],"rotation":angle}});
        }
        Ok(Some(info))
    }
    fn material(&mut self, m: &ufbx::Material) -> Result<Value, String> {
        let p = &m.pbr;
        let scalar = |v: &ufbx::MaterialMap, default: f64| {
            if v.has_value { v.value_vec4.x } else { default }
        };
        let color = |v: &ufbx::MaterialMap, default: [f64; 3]| {
            if v.has_value {
                [v.value_vec4.x, v.value_vec4.y, v.value_vec4.z]
            } else {
                default
            }
        };
        let base = color(&p.base_color, [0.75, 0.75, 0.75]);
        let base_factor = scalar(&p.base_factor, 1.0);
        let alpha = scalar(&p.opacity, 1.0).clamp(0.0, 1.0);
        let mut out = json!({"name":m.element.name.as_ref(),"doubleSided":m.features.double_sided.enabled,"pbrMetallicRoughness":{"baseColorFactor":[base[0]*base_factor,base[1]*base_factor,base[2]*base_factor,alpha],"metallicFactor":scalar(&p.metalness,0.0).clamp(0.0,1.0),"roughnessFactor":scalar(&p.roughness,0.5).clamp(0.0,1.0)}});
        if alpha < 1.0 {
            out["alphaMode"] = json!("BLEND");
        }
        if let Some(info) = self.texture(&p.base_color, None)? {
            out["pbrMetallicRoughness"]["baseColorTexture"] = info;
        }
        if let Some(info) = self.texture(&p.normal_map, None)? {
            out["normalTexture"] = info;
        }
        let e = color(&p.emission_color, [0.0, 0.0, 0.0]);
        let emission = scalar(&p.emission_factor, 1.0);
        let emission_scale = e.into_iter().fold(1.0, f64::max);
        out["emissiveFactor"] = json!([
            (e[0] / emission_scale).max(0.0),
            (e[1] / emission_scale).max(0.0),
            (e[2] / emission_scale).max(0.0)
        ]);
        if let Some(info) = self.texture(&p.emission_color, None)? {
            out["emissiveTexture"] = info;
        }
        out["extensions"] = json!({
            "KHR_materials_ior":{"ior":scalar(&p.specular_ior,1.5).max(1.0)},
            "KHR_materials_specular":{"specularFactor":scalar(&p.specular_factor,1.0).clamp(0.0,1.0),"specularColorFactor":color(&p.specular_color,[1.0,1.0,1.0])},
            "KHR_materials_transmission":{"transmissionFactor":scalar(&p.transmission_factor,0.0).clamp(0.0,1.0)},
            "KHR_materials_clearcoat":{"clearcoatFactor":scalar(&p.coat_factor,0.0).clamp(0.0,1.0),"clearcoatRoughnessFactor":scalar(&p.coat_roughness,0.03).clamp(0.0,1.0)},
            "KHR_materials_sheen":{"sheenColorFactor":color(&p.sheen_color,[1.0,1.0,1.0]).map(|v|v*scalar(&p.sheen_factor,0.0)),"sheenRoughnessFactor":scalar(&p.sheen_roughness,0.5).clamp(0.0,1.0)},
            "KHR_materials_emissive_strength":{"emissiveStrength":emission.max(0.0)*emission_scale}
        });
        Ok(out)
    }
}

fn v3(v: ufbx::Vec3) -> [f32; 3] {
    [v.x as f32, v.y as f32, v.z as f32]
}
fn matrix(m: ufbx::Matrix) -> [f32; 16] {
    [
        m.m00 as f32,
        m.m10 as f32,
        m.m20 as f32,
        0.0,
        m.m01 as f32,
        m.m11 as f32,
        m.m21 as f32,
        0.0,
        m.m02 as f32,
        m.m12 as f32,
        m.m22 as f32,
        0.0,
        m.m03 as f32,
        m.m13 as f32,
        m.m23 as f32,
        1.0,
    ]
}

pub fn convert(bytes: &[u8]) -> Result<Imported, String> {
    let start = Instant::now();
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err("FBX files must be between 1 byte and 256 MB".into());
    }
    let axes = ufbx::CoordinateAxes {
        right: ufbx::CoordinateAxis::PositiveX,
        up: ufbx::CoordinateAxis::PositiveY,
        front: ufbx::CoordinateAxis::PositiveZ,
    };
    let opts = ufbx::LoadOpts {
        file_format: ufbx::FileFormat::Fbx,
        load_external_files: false,
        ignore_missing_external_files: true,
        generate_missing_normals: true,
        clean_skin_weights: true,
        normalize_normals: true,
        target_axes: axes,
        target_unit_meters: 1.0,
        target_camera_axes: axes,
        target_light_axes: axes,
        geometry_transform_handling: ufbx::GeometryTransformHandling::HelperNodes,
        inherit_mode_handling: ufbx::InheritModeHandling::HelperNodes,
        node_depth_limit: 128,
        temp_allocator: ufbx::AllocatorOpts {
            memory_limit: 512 * 1024 * 1024,
            ..Default::default()
        },
        result_allocator: ufbx::AllocatorOpts {
            memory_limit: 512 * 1024 * 1024,
            ..Default::default()
        },
        ..Default::default()
    };
    let scene = ufbx::load_memory(bytes, opts).map_err(|e| format!("Cannot read FBX: {e:?}"))?;
    if scene.nodes.len() > MAX_NODES || scene.materials.len() > 4096 || scene.anim_stacks.len() > 16
    {
        return Err("FBX exceeds the node, material or animation budget".into());
    }
    if scene.meshes.iter().map(|m| m.num_triangles).sum::<usize>() > MAX_TRIANGLES {
        return Err("FBX exceeds 10 million triangles".into());
    }
    let mut g = Gltf::default();
    let mut materials = scene
        .materials
        .iter()
        .map(|m| g.material(m))
        .collect::<Result<Vec<_>, _>>()?;
    let default_material = materials.len();
    materials.push(
        json!({"name":"Default","pbrMetallicRoughness":{"metallicFactor":0,"roughnessFactor":0.5}}),
    );
    let mut nodes = Vec::new();
    let mut meshes = Vec::new();
    let mut skins = Vec::new();
    let mut cameras = Vec::new();
    let mut lights = Vec::new();
    let mut triangle_count = 0usize;
    for node in &scene.nodes {
        let t = &node.local_transform;
        let mut out = json!({"name":node.element.name.as_ref(),"translation":v3(t.translation),"rotation":[t.rotation.x,t.rotation.y,t.rotation.z,t.rotation.w],"scale":v3(t.scale),"children":node.children.iter().map(|n|n.element.typed_id).collect::<Vec<_>>()});
        if !node.visible {
            out["extras"] = json!({"studioFbxVisible":false});
        }
        if let Some(mesh) = &node.mesh {
            let skin = mesh.skin_deformers.first();
            if skin.is_some_and(|s| s.clusters.len() > u16::MAX as usize) {
                return Err("FBX skin has too many joints".into());
            }
            let mut primitives = Vec::new();
            let mut groups: Vec<Vec<usize>> = vec![Vec::new(); node.materials.len().max(1)];
            for face in 0..mesh.faces.len() {
                let mat = mesh.face_material.get(face).copied().unwrap_or(0) as usize;
                let mat = mat.min(groups.len() - 1);
                groups[mat].push(face);
            }
            for (material, faces) in groups.iter().enumerate() {
                let mut positions = Vec::new();
                let mut normals = Vec::new();
                let mut uv = Vec::new();
                let mut uv1 = Vec::new();
                let mut colors = Vec::new();
                let mut joints = Vec::new();
                let mut weights = Vec::new();
                let mut corners = vec![0u32; mesh.max_face_triangles * 3];
                for &face in faces {
                    let count =
                        ufbx::triangulate_face(&mut corners, mesh, mesh.faces[face]) as usize * 3;
                    triangle_count += count / 3;
                    if triangle_count > MAX_TRIANGLES {
                        return Err("FBX instances exceed 10 million triangles".into());
                    }
                    for &corner in &corners[..count] {
                        let i = corner as usize;
                        positions.extend(v3(mesh.vertex_position[i]));
                        normals.extend(v3(mesh.vertex_normal[i]));
                        if mesh.vertex_uv.exists {
                            let p = mesh.vertex_uv[i];
                            uv.extend([p.x as f32, 1.0 - p.y as f32]);
                        }
                        if let Some(set) = mesh.uv_sets.get(1) {
                            let p = set.vertex_uv[i];
                            uv1.extend([p.x as f32, 1.0 - p.y as f32]);
                        }
                        if mesh.vertex_color.exists {
                            let c = mesh.vertex_color[i];
                            colors.extend([c.x as f32, c.y as f32, c.z as f32, c.w as f32]);
                        }
                        if let Some(skin) = skin {
                            let vertex = skin.vertices[mesh.vertex_indices[i] as usize];
                            let influences = &skin.weights.as_ref()[vertex.weight_begin as usize
                                ..(vertex.weight_begin + vertex.num_weights) as usize];
                            let mut js = [0u16; 4];
                            let mut ws = [0f32; 4];
                            for (j, w) in influences.iter().take(4).enumerate() {
                                js[j] = w.cluster_index as u16;
                                ws[j] = w.weight as f32;
                            }
                            let sum: f32 = ws.iter().sum();
                            if sum <= 0.0 {
                                return Err("FBX skin contains an unweighted vertex".into());
                            }
                            for w in &mut ws {
                                *w /= sum;
                            }
                            joints.extend(js);
                            weights.extend(ws);
                            if influences.len() > 4
                                && !g.warnings.iter().any(|w| w.starts_with("Skin weights"))
                            {
                                g.warnings.push(
                                    "Skin weights use the four strongest influences per vertex"
                                        .into(),
                                );
                            }
                        }
                    }
                }
                if positions.is_empty() {
                    continue;
                }
                let mut attributes = json!({"POSITION":g.floats(&positions,"VEC3",3)?,"NORMAL":g.floats(&normals,"VEC3",3)?});
                if !uv.is_empty() {
                    attributes["TEXCOORD_0"] = json!(g.floats(&uv, "VEC2", 2)?);
                }
                if !uv1.is_empty() {
                    attributes["TEXCOORD_1"] = json!(g.floats(&uv1, "VEC2", 2)?);
                }
                if !colors.is_empty() {
                    attributes["COLOR_0"] = json!(g.floats(&colors, "VEC4", 4)?);
                }
                if skin.is_some() {
                    attributes["JOINTS_0"] = json!(g.joints(&joints)?);
                    attributes["WEIGHTS_0"] = json!(g.floats(&weights, "VEC4", 4)?);
                }
                let mat = node
                    .materials
                    .get(material)
                    .map(|m| m.element.typed_id as usize)
                    .unwrap_or(default_material);
                primitives.push(json!({"attributes":attributes,"material":mat,"mode":4}));
            }
            if !primitives.is_empty() {
                out["mesh"] = json!(meshes.len());
                meshes.push(json!({"name":mesh.element.name.as_ref(),"primitives":primitives}));
                if let Some(skin) = skin {
                    let matrices: Vec<f32> = skin
                        .clusters
                        .iter()
                        .flat_map(|c| matrix(c.geometry_to_bone))
                        .collect();
                    let joints: Vec<u32> = skin
                        .clusters
                        .iter()
                        .map(|c| {
                            c.bone_node
                                .as_ref()
                                .map(|n| n.element.typed_id)
                                .ok_or_else(|| "FBX skin is missing a joint".to_string())
                        })
                        .collect::<Result<_, _>>()?;
                    let accessor = g.floats(&matrices, "MAT4", 16)?;
                    out["skin"] = json!(skins.len());
                    skins.push(json!({"joints":joints,"inverseBindMatrices":accessor}));
                }
            }
        }
        if let Some(camera) = &node.camera {
            out["camera"] = json!(cameras.len());
            let near = camera.near_plane.max(1e-5);
            let far = camera.far_plane.max(near + 1.0);
            cameras.push(if camera.projection_mode==ufbx::ProjectionMode::Orthographic {json!({"type":"orthographic","orthographic":{"xmag":camera.orthographic_size.x.max(1e-5),"ymag":camera.orthographic_size.y.max(1e-5),"znear":near,"zfar":far}})} else {json!({"type":"perspective","perspective":{"yfov":camera.field_of_view_deg.y.to_radians().clamp(0.01,3.0),"aspectRatio":camera.aspect_ratio.max(1e-5),"znear":near,"zfar":far}})});
        }
        if let Some(light) = &node.light {
            out["extras"]["studioFbxCastShadow"] = json!(light.cast_shadows);
            if matches!(light.type_, ufbx::LightType::Area | ufbx::LightType::Volume)
                && !g.warnings.iter().any(|w| w.starts_with("Area/volume"))
            {
                g.warnings
                    .push("Area/volume FBX lights use a point-light approximation".into());
            }
            let kind = match light.type_ {
                ufbx::LightType::Directional => "directional",
                ufbx::LightType::Spot => "spot",
                _ => "point",
            };
            let mut l = json!({"name":light.element.name.as_ref(),"type":kind,"color":v3(light.color),"intensity":if light.cast_light {light.intensity.max(0.0)}else{0.0}});
            if kind == "spot" {
                l["spot"] = json!({"innerConeAngle":(light.inner_angle*0.5).to_radians().clamp(0.0,1.56),"outerConeAngle":(light.outer_angle*0.5).to_radians().clamp(0.001,1.57)});
            }
            out["extensions"] = json!({"KHR_lights_punctual":{"light":lights.len()}});
            lights.push(l);
        }
        nodes.push(out);
    }
    if meshes.is_empty() {
        return Err("FBX has no triangle geometry".into());
    }
    let mut animations = Vec::new();
    for stack in &scene.anim_stacks {
        if stack.time_end - stack.time_begin > 600.0 {
            return Err("FBX animations must be shorter than 10 minutes per take".into());
        }
        let baked = ufbx::bake_anim(
            &scene,
            &stack.anim,
            ufbx::BakeOpts {
                trim_start_time: true,
                resample_rate: 30.0,
                maximum_sample_rate: 120.0,
                max_keyframe_segments: 1_000_000,
                key_reduction_enabled: true,
                key_reduction_rotation: true,
                key_reduction_threshold: 1e-5,
                temp_allocator: ufbx::AllocatorOpts {
                    memory_limit: 256 * 1024 * 1024,
                    ..Default::default()
                },
                result_allocator: ufbx::AllocatorOpts {
                    memory_limit: 256 * 1024 * 1024,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .map_err(|e| format!("Cannot bake FBX animation: {e:?}"))?;
        let mut channels = Vec::new();
        let mut samplers = Vec::new();
        for node in &baked.nodes {
            let mut track = |path: &str,
                             times: Vec<f32>,
                             values: Vec<f32>,
                             width: usize|
             -> Result<(), String> {
                if times.is_empty() {
                    return Ok(());
                }
                let input = g.floats(&times, "SCALAR", 1)?;
                let output = g.floats(&values, if width == 4 { "VEC4" } else { "VEC3" }, width)?;
                channels.push(
                    json!({"sampler":samplers.len(),"target":{"node":node.typed_id,"path":path}}),
                );
                samplers.push(json!({"input":input,"output":output,"interpolation":"LINEAR"}));
                Ok(())
            };
            track(
                "translation",
                node.translation_keys
                    .iter()
                    .map(|k| k.time as f32)
                    .collect(),
                node.translation_keys
                    .iter()
                    .flat_map(|k| v3(k.value))
                    .collect(),
                3,
            )?;
            track(
                "rotation",
                node.rotation_keys.iter().map(|k| k.time as f32).collect(),
                node.rotation_keys
                    .iter()
                    .flat_map(|k| {
                        [
                            k.value.x as f32,
                            k.value.y as f32,
                            k.value.z as f32,
                            k.value.w as f32,
                        ]
                    })
                    .collect(),
                4,
            )?;
            track(
                "scale",
                node.scale_keys.iter().map(|k| k.time as f32).collect(),
                node.scale_keys.iter().flat_map(|k| v3(k.value)).collect(),
                3,
            )?;
        }
        if !channels.is_empty() {
            animations.push(
                json!({"name":stack.element.name.as_ref(),"channels":channels,"samplers":samplers}),
            );
        }
    }
    let stats = json!({"importMs":start.elapsed().as_secs_f64()*1000.0,"triangles":triangle_count,"nodes":nodes.len(),"meshes":meshes.len(),"skins":skins.len(),"clips":animations.len(),"warnings":g.warnings});
    let mut doc = json!({"asset":{"version":"2.0","generator":"Powermove ufbx"},"scene":0,"scenes":[{"nodes":[scene.root_node.element.typed_id]}],"nodes":nodes,"meshes":meshes,"materials":materials,"skins":skins,"cameras":cameras,"animations":animations,"accessors":g.accessors,"bufferViews":g.views,"buffers":[{"byteLength":g.bin.len()}],"images":g.images,"textures":g.textures,"samplers":[{"wrapS":10497,"wrapT":10497}],"extensionsUsed":["KHR_materials_ior","KHR_materials_specular","KHR_materials_transmission","KHR_materials_clearcoat","KHR_materials_sheen","KHR_materials_emissive_strength","KHR_texture_transform"]});
    if !lights.is_empty() {
        doc["extensions"] = json!({"KHR_lights_punctual":{"lights":lights}});
        doc["extensionsUsed"]
            .as_array_mut()
            .unwrap()
            .push(json!("KHR_lights_punctual"));
    }
    let mut json = serde_json::to_vec(&doc).map_err(|e| e.to_string())?;
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    while g.bin.len() % 4 != 0 {
        g.bin.push(0);
    }
    let total = 12 + 8 + json.len() + 8 + g.bin.len();
    if total > MAX_BYTES {
        return Err("Converted FBX exceeds 256 MB".into());
    }
    let mut glb = Vec::with_capacity(total);
    for word in [
        0x46546c67u32,
        2,
        total as u32,
        json.len() as u32,
        0x4e4f534a,
    ] {
        glb.extend(word.to_le_bytes());
    }
    glb.extend(json);
    glb.extend((g.bin.len() as u32).to_le_bytes());
    glb.extend(0x004e4942u32.to_le_bytes());
    glb.extend(g.bin);
    Ok(Imported { glb, stats })
}

#[cfg(test)]
mod tests {
    use super::*;
    const ANIMATED: &[u8] = include_bytes!("../fixtures/tiny-animated.fbx");
    const STATIC: &[u8] = include_bytes!("../fixtures/tiny-static.fbx");
    fn document(glb: &[u8]) -> Value {
        assert_eq!(&glb[..4], b"glTF");
        assert_eq!(
            u32::from_le_bytes(glb[8..12].try_into().unwrap()) as usize,
            glb.len()
        );
        let size = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        serde_json::from_slice(&glb[20..20 + size]).unwrap()
    }
    #[test]
    fn imports_geometry_materials_cameras_and_lights() {
        let imported = convert(STATIC).unwrap();
        let d = document(&imported.glb);
        assert_eq!(d["meshes"].as_array().unwrap().len(), 1);
        assert_eq!(d["cameras"].as_array().unwrap().len(), 1);
        assert_eq!(
            d["extensions"]["KHR_lights_punctual"]["lights"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(d["materials"][0]["name"], "Coral");
        assert!(
            (d["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"][0]
                .as_f64()
                .unwrap()
                - 0.8)
                .abs()
                < 0.001
        );
        assert_eq!(imported.stats["triangles"], 1);
        assert!(d["skins"].as_array().unwrap().is_empty());
        assert!(d["animations"].as_array().unwrap().is_empty());
    }
    #[test]
    fn preserves_joints_bind_matrices_and_bakes_node_animation() {
        let imported = convert(ANIMATED).unwrap();
        let d = document(&imported.glb);
        assert_eq!(d["skins"].as_array().unwrap().len(), 1);
        assert_eq!(d["skins"][0]["joints"].as_array().unwrap().len(), 1);
        let attrs = &d["meshes"][0]["primitives"][0]["attributes"];
        assert!(attrs["JOINTS_0"].is_number());
        assert!(attrs["WEIGHTS_0"].is_number());
        let animation = &d["animations"][0];
        assert_eq!(animation["name"], "Move");
        assert!(
            animation["channels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["target"]["path"] == "translation")
        );
        let output = animation["samplers"][0]["output"].as_u64().unwrap() as usize;
        let a = &d["accessors"][output];
        assert_eq!(a["type"], "VEC3");
        assert!((a["max"][0].as_f64().unwrap() - 2.0).abs() < 0.001);
        assert_eq!(imported.stats["clips"], 1);
    }
    #[test]
    fn rejects_invalid_and_empty_files_without_panicking() {
        for data in [
            &[][..],
            b"not an FBX",
            b"FBXHeaderExtension: { FBXVersion: 7400 } Objects: {}",
        ] {
            assert!(convert(data).is_err());
        }
    }
}
