//! A renderer-only compatibility view of the engine's imported glTF data.
//! Geometry and hierarchy come from pm-geom3d; no GLTFLoader runs in the
//! new-engine import lane. The JSON is consumed once when an asset loads.
use pm_geom3d::model_import::{self as gltf, AlphaMode, Interpolation, LightKind, Path};
use serde_json::{Value, json};

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        out.push(T[(a >> 2) as usize] as char);
        out.push(T[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[(((b & 15) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn texture_ref(r: &Option<gltf::TextureRef>) -> Value {
    r.as_ref().map_or(Value::Null, |r| json!({
        "texture":r.texture,"texCoord":r.tex_coord,
        "transform":r.transform.map(|t|json!({"offset":t.offset,"rotation":t.rotation,"scale":t.scale,"texCoord":t.tex_coord}))
    }))
}

fn metadata(def: &Value) -> Value {
    json!({"name":def.get("name"),"extras":def.get("extras"),"extensions":def.get("extensions")})
}

fn metadata_list(raw: &Value, key: &str) -> Vec<Value> {
    raw.get(key)
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |list| list.iter().map(metadata).collect())
}

enum Bulk {
    Json,
    Packed(Vec<u8>),
}

impl Bulk {
    fn f32_rows<const N: usize>(&mut self, rows: &[[f32; N]]) -> Value {
        match self {
            Self::Json => Value::Array(
                rows.iter()
                    .map(|row| Value::Array(row.iter().map(|value| json!(value)).collect()))
                    .collect(),
            ),
            Self::Packed(payload) => {
                let offset = payload.len();
                for row in rows {
                    for &value in row {
                        // JSON serializes non-finite components as null;
                        // Float32BufferAttribute converts null to +0.
                        payload.extend_from_slice(
                            &if value.is_finite() { value } else { 0.0 }.to_le_bytes(),
                        );
                    }
                }
                json!({"packed":{"offset":offset,"length":rows.len() * N,"components":N,"type":"f32"}})
            }
        }
    }

    fn optional_f32_rows<const N: usize>(&mut self, rows: Option<&[[f32; N]]>) -> Value {
        rows.map_or(Value::Null, |rows| self.f32_rows(rows))
    }

    fn u16_rows<const N: usize>(&mut self, rows: Option<&[[u16; N]]>) -> Value {
        match (self, rows) {
            (_, None) => Value::Null,
            (Self::Json, Some(rows)) => Value::Array(
                rows.iter()
                    .map(|row| Value::Array(row.iter().map(|value| json!(value)).collect()))
                    .collect(),
            ),
            (Self::Packed(payload), Some(rows)) => {
                let offset = payload.len();
                for row in rows {
                    for &value in row {
                        payload.extend_from_slice(&u32::from(value).to_le_bytes());
                    }
                }
                json!({"packed":{"offset":offset,"length":rows.len() * N,"components":N,"type":"u32"}})
            }
        }
    }

    fn u32_scalars(&mut self, values: Option<&[u32]>) -> Value {
        match (self, values) {
            (_, None) => Value::Null,
            (Self::Json, Some(values)) => json!(values),
            (Self::Packed(payload), Some(values)) => {
                let offset = payload.len();
                for &value in values {
                    payload.extend_from_slice(&value.to_le_bytes());
                }
                json!({"packed":{"offset":offset,"length":values.len(),"components":1,"type":"u32"}})
            }
        }
    }

    fn finish(self, scene: Value) -> Result<Vec<u8>, String> {
        let Self::Packed(mut payload) = self else {
            unreachable!("only packed imports are finalized as a packet")
        };
        let header = serde_json::to_vec(&scene).map_err(|e| e.to_string())?;
        let header_len =
            u32::try_from(header.len()).map_err(|_| "Model metadata is too large".to_owned())?;
        let prefix = (4 + header.len()).next_multiple_of(4);
        let payload_len = payload.len();
        payload.resize(prefix + payload_len, 0);
        payload.copy_within(..payload_len, prefix);
        payload[..4].copy_from_slice(&header_len.to_le_bytes());
        payload[4..4 + header.len()].copy_from_slice(&header);
        Ok(payload)
    }
}

/// Import with the exact checks and normalized scene facts used by the engine.
pub fn import(bytes: &[u8]) -> Result<Value, String> {
    import_with(bytes, &mut Bulk::Json)
}

/// Same scene as [`import`], with large mesh numeric attributes in a binary
/// payload: u32 LE header length, JSON metadata, align-to-four padding, then
/// f32/u32 LE scalars at marker offsets relative to the payload start.
pub fn import_packed(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut bulk = Bulk::Packed(Vec::new());
    let scene = import_with(bytes, &mut bulk)?;
    bulk.finish(scene)
}

fn import_with(bytes: &[u8], bulk: &mut Bulk) -> Result<Value, String> {
    let (model, scene, facts) = gltf::import(bytes).map_err(|e| e.0)?;
    let raw = gltf::json_of(bytes).map_err(|e| e.0)?;
    let raw_meshes: Vec<Value> =
        raw.get("meshes")
            .and_then(Value::as_array)
            .map_or_else(Vec::new, |list| {
                list.iter().map(|mesh| {
        json!({"meta":metadata(mesh),"primitives":mesh.get("primitives").and_then(Value::as_array)
            .map_or_else(Vec::new, |primitives| primitives.iter().map(metadata).collect())})
    }).collect()
            });
    let scene_index = raw.get("scene").and_then(Value::as_u64).unwrap_or(0) as usize;
    let raw_scene = raw
        .get("scenes")
        .and_then(Value::as_array)
        .and_then(|list| list.get(scene_index))
        .map(metadata);
    let raw_lights = raw
        .get("extensions")
        .and_then(|v| v.get("KHR_lights_punctual"))
        .map(|v| metadata_list(v, "lights"))
        .unwrap_or_default();
    let objects: Vec<Value> = scene
        .objects
        .iter()
        .map(|o| {
            json!({
                "kind":format!("{:?}",o.kind),"name":o.name,"parent":o.parent,"children":o.children,
                "matrix":o.matrix,"node":o.node,"primitive":o.primitive,"instances":o.instances,
                "trs":o.node.map(|n|json!({"translation":model.nodes[n].translation,"rotation":model.nodes[n].rotation,"scale":model.nodes[n].scale})),
                "skin":o.skin,"light":o.light,"camera":o.camera
            })
        })
        .collect();
    let meshes: Vec<Value> = model
        .meshes
        .iter()
        .map(|m| {
            json!({
                "weights":m.weights,
                "primitives":m.primitives.iter().map(|p| json!({
                    "topology":format!("{:?}",p.topology),
                    "positions":bulk.f32_rows(&p.positions),
                    "normals":bulk.optional_f32_rows(p.normals.as_deref()),
                    "uv":bulk.optional_f32_rows(p.uv.as_deref()),
                    "uv1":bulk.optional_f32_rows(p.uv1.as_deref()),
                    "uv2":bulk.optional_f32_rows(p.uv2.as_deref()),
                    "uv3":bulk.optional_f32_rows(p.uv3.as_deref()),
                    "colors":bulk.optional_f32_rows(p.colors.as_deref()),
                    "colorComponents":p.color_components,
                    "tangents":bulk.optional_f32_rows(p.tangents.as_deref()),
                    "joints":bulk.u16_rows(p.joints.as_deref()),
                    "weights":bulk.optional_f32_rows(p.weights.as_deref()),
                    "indices":bulk.u32_scalars(p.indices.as_deref()),
                    "targets":p.targets.iter().map(|t|json!({
                        "position":bulk.optional_f32_rows(t.position.as_deref()),
                        "normal":bulk.optional_f32_rows(t.normal.as_deref())
                    })).collect::<Vec<_>>(),
                    "material":p.material,"accessorBounds":p.accessor_bounds
                })).collect::<Vec<_>>()
            })
        })
        .collect();
    let materials: Vec<Value> = model.materials.iter().map(|m| json!({
        "name":m.name,"alphaMode":match m.alpha_mode{AlphaMode::Opaque=>"OPAQUE",AlphaMode::Mask=>"MASK",AlphaMode::Blend=>"BLEND"},
        "alphaCutoff":m.alpha_cutoff,"doubleSided":m.double_sided,"baseColor":m.base_color,
        "baseColorTexture":texture_ref(&m.base_color_texture),"metallic":m.metallic,"roughness":m.roughness,
        "metallicRoughnessTexture":texture_ref(&m.metallic_roughness_texture),
        "normalTexture":texture_ref(&m.normal_texture),"normalScale":m.normal_scale,
        "occlusionTexture":texture_ref(&m.occlusion_texture),"occlusionStrength":m.occlusion_strength,
        "emissive":m.emissive,"emissiveTexture":texture_ref(&m.emissive_texture),
        "emissiveStrength":m.emissive_strength,"unlit":m.unlit,"extensions":m.extensions
    })).collect();
    let images: Vec<Value> = model
        .images
        .iter()
        .map(|i| json!({"mime":i.mime,"base64":base64(&i.bytes)}))
        .collect();
    let textures: Vec<Value> = model.textures.iter().map(|t|json!({
        "source":t.source,"sampler":{"mag":t.sampler.mag_filter,"min":t.sampler.min_filter,"wrapS":t.sampler.wrap_s,"wrapT":t.sampler.wrap_t}
    })).collect();
    let skins: Vec<Value> = model
        .skins
        .iter()
        .map(|s| json!({"joints":s.joints,"inverseBind":s.inverse_bind}))
        .collect();
    let instances: Vec<Value> = model.nodes.iter().map(|n| json!(n.instances)).collect();
    let node_weights: Vec<Value> = model.nodes.iter().map(|n| json!(n.weights)).collect();
    let clips: Vec<Value> = model.clips.iter().map(|c|json!({
        "name":c.name,"duration":c.duration,
        "channels":c.channels.iter().map(|ch|json!({
            "node":ch.node,"path":match ch.path{Path::Translation=>"translation",Path::Rotation=>"rotation",Path::Scale=>"scale",Path::Weights=>"weights"},
            "interpolation":match ch.interpolation{Interpolation::Linear=>"LINEAR",Interpolation::Step=>"STEP",Interpolation::CubicSpline=>"CUBICSPLINE"},
            "times":ch.times,"values":ch.values,"size":ch.size
        })).collect::<Vec<_>>()
    })).collect();
    let lights: Vec<Value> = model.lights.iter().map(|l|json!({
        "color":l.color,"intensity":l.intensity,"range":l.range,
        "kind":match l.kind{LightKind::Directional=>"directional",LightKind::Point=>"point",LightKind::Spot{..}=>"spot"},
        "spot":match l.kind{LightKind::Spot{inner_cone,outer_cone}=>json!([inner_cone,outer_cone]),_=>Value::Null}
    })).collect();
    let cameras: Vec<Value> = model
        .cameras
        .iter()
        .map(|c| match c {
            gltf::Camera::Perspective {
                yfov,
                aspect,
                znear,
                zfar,
            } => {
                json!({"kind":"perspective","yfov":yfov,"aspect":aspect,"znear":znear,"zfar":zfar})
            }
            gltf::Camera::Orthographic {
                xmag,
                ymag,
                znear,
                zfar,
            } => json!({"kind":"orthographic","xmag":xmag,"ymag":ymag,"znear":znear,"zfar":zfar}),
        })
        .collect();
    Ok(
        json!({"objects":objects,"nodeObjects":scene.node_objects,"meshes":meshes,
        "materials":materials,"images":images,"textures":textures,"skins":skins,"instances":instances,"nodeWeights":node_weights,
        "nodeMeshes":model.nodes.iter().map(|n|n.mesh).collect::<Vec<_>>(),"clips":clips,
        "lights":lights,"cameras":cameras,
        "metadata":{"scene":raw_scene,"nodes":metadata_list(&raw,"nodes"),"meshes":raw_meshes,
          "materials":metadata_list(&raw,"materials"),"images":metadata_list(&raw,"images"),
          "textures":metadata_list(&raw,"textures"),
          "cameras":metadata_list(&raw,"cameras"),"lights":raw_lights,"animations":metadata_list(&raw,"animations")},
        "facts":{"vertices":facts.vertices,"triangles":facts.triangles,"center":facts.center,"radius":facts.radius}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_geom3d::model_import::glb::GlbWriter;
    use serde_json::Map;

    fn unpack(packet: &[u8]) -> (Value, &[u8]) {
        let len = u32::from_le_bytes(packet[..4].try_into().unwrap()) as usize;
        let header = serde_json::from_slice(&packet[4..4 + len]).unwrap();
        let start = (4 + len).next_multiple_of(4);
        (header, &packet[start..])
    }

    fn floats(marker: &Value, payload: &[u8]) -> Vec<f32> {
        let p = &marker["packed"];
        assert_eq!(p["type"], "f32");
        let start = p["offset"].as_u64().unwrap() as usize;
        let len = p["length"].as_u64().unwrap() as usize;
        payload[start..start + len * 4]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect()
    }

    fn ints(marker: &Value, payload: &[u8]) -> Vec<u32> {
        let p = &marker["packed"];
        assert_eq!(p["type"], "u32");
        let start = p["offset"].as_u64().unwrap() as usize;
        let len = p["length"].as_u64().unwrap() as usize;
        payload[start..start + len * 4]
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect()
    }

    #[test]
    fn packed_fields_match_json_to_float32_and_integer_conversion() {
        let rows = [
            [-0.0, 1.0, f32::NAN],
            [f32::INFINITY, -2.5, f32::from_bits(1)],
        ];
        let mut packed = Bulk::Packed(Vec::new());
        let positions = packed.f32_rows(&rows);
        let short_normals = packed.optional_f32_rows(Some(&[[1.0, 0.0, 0.0]]));
        let missing_uv: Value = packed.optional_f32_rows::<2>(None);
        let joints = packed.u16_rows(Some(&[[0, 2, u16::MAX, 4]]));
        let indices = packed.u32_scalars(Some(&[0, 65_536, u32::MAX]));
        let morph_position = packed.optional_f32_rows(Some(&[[0.25, 0.0, -0.25]]));
        let morph_normal: Value = packed.optional_f32_rows::<3>(None);
        let packet = packed
            .finish(json!({
                "positions":positions,"normals":short_normals,"uv":missing_uv,
                "joints":joints,"indices":indices,
                "targets":[{"position":morph_position,"normal":morph_normal}]
            }))
            .unwrap();
        let (header, payload) = unpack(&packet);
        assert_eq!(header["uv"], Value::Null);
        assert_eq!(header["targets"][0]["normal"], Value::Null);
        assert_eq!(header["normals"]["packed"]["components"], 3);
        assert_eq!(header["normals"]["packed"]["length"], 3);
        let old = json!(rows);
        let old_float32: Vec<f32> = old
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|row| {
                row.as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            })
            .collect();
        assert_eq!(
            floats(&header["positions"], payload)
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            old_float32.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        );
        assert_eq!(floats(&header["normals"], payload), [1.0, 0.0, 0.0]);
        assert_eq!(
            floats(&header["targets"][0]["position"], payload),
            [0.25, 0.0, -0.25]
        );
        assert_eq!(ints(&header["joints"], payload), [0, 2, 65_535, 4]);
        assert_eq!(ints(&header["indices"], payload), [0, 65_536, u32::MAX]);
    }

    #[test]
    fn packed_import_preserves_scene_metadata_and_short_attribute_counts() {
        let mut glb = GlbWriter::default();
        let positions = glb.floats(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], true);
        let normals = glb.floats(&[[0.0, 0.0, 1.0]], false);
        let indices = glb.indices(&[0, 1, 2]);
        let mut document = Map::new();
        document.insert("asset".into(), json!({"version":"2.0"}));
        document.insert(
            "meshes".into(),
            json!([{"name":"tiny","primitives":[{
                "attributes":{"POSITION":positions,"NORMAL":normals},"indices":indices
            }]}]),
        );
        document.insert("nodes".into(), json!([{"mesh":0}]));
        document.insert("scenes".into(), json!([{"nodes":[0]}]));
        let bytes = glb.finish(document);
        let old = import(&bytes).unwrap();
        let packet = import_packed(&bytes).unwrap();
        let (mut header, payload) = unpack(&packet);
        let prim = &header["meshes"][0]["primitives"][0];
        assert_eq!(floats(&prim["positions"], payload).len(), 9);
        assert_eq!(floats(&prim["normals"], payload).len(), 3);
        assert_eq!(ints(&prim["indices"], payload), [0, 1, 2]);
        header["meshes"][0]["primitives"][0]["positions"] =
            old["meshes"][0]["primitives"][0]["positions"].clone();
        header["meshes"][0]["primitives"][0]["normals"] =
            old["meshes"][0]["primitives"][0]["normals"].clone();
        header["meshes"][0]["primitives"][0]["indices"] =
            old["meshes"][0]["primitives"][0]["indices"].clone();
        assert_eq!(header, old);
    }

    #[test]
    fn six_hundred_thousand_vertices_keep_metadata_small() {
        let rows = vec![[0.0f32, 1.0, 2.0]; 600_000];
        let mut packed = Bulk::Packed(Vec::new());
        let marker = packed.f32_rows(&rows);
        let packet = packed.finish(json!({"positions":marker})).unwrap();
        let header_len = u32::from_le_bytes(packet[..4].try_into().unwrap()) as usize;
        assert!(header_len < 256, "bulk vertices must not enter JSON");
        let (header, payload) = unpack(&packet);
        assert_eq!(header["positions"]["packed"]["length"], 1_800_000);
        assert_eq!(payload.len(), 600_000 * 3 * 4);
    }
}
