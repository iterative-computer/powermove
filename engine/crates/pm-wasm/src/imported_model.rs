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

/// Import with the exact checks and normalized scene facts used by the engine.
pub fn import(bytes: &[u8]) -> Result<Value, String> {
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
    let meshes: Vec<Value> = model.meshes.iter().map(|m| json!({
        "weights":m.weights,
        "primitives":m.primitives.iter().map(|p| json!({
            "topology":format!("{:?}",p.topology),"positions":p.positions,"normals":p.normals,
            "uv":p.uv,"uv1":p.uv1,"uv2":p.uv2,"uv3":p.uv3,
            "colors":p.colors,"colorComponents":p.color_components,
            "tangents":p.tangents,"joints":p.joints,"weights":p.weights,"indices":p.indices,
            "targets":p.targets.iter().map(|t|json!({"position":t.position,"normal":t.normal})).collect::<Vec<_>>(),
            "material":p.material,"accessorBounds":p.accessor_bounds
        })).collect::<Vec<_>>()
    })).collect();
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
