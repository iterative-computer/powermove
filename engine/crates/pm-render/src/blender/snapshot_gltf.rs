//! Imported models in `renderSnapshot`: CPU-posed geometry and the source
//! materials of the runtime clone, including Canvas2D texture snapshots.
use std::collections::VecDeque;
use std::sync::Arc;

use pm_blender::snapshot::{AssetBytes, SnapshotAsset};
use pm_geom3d::gltf::{Mode, ObjectKind, pose};
use pm_model::json::{Map, Value};

use crate::scene3d::gltf::{GltfAsset, GltfMap, MapSource, material};
use crate::scene3d::{Material3d, MaterialKind, Object3d};
use crate::three::math::{self, Mat4};

fn numbers(xs: impl IntoIterator<Item = f64>) -> Value {
    Value::Array(xs.into_iter().map(Value::Number).collect())
}
fn channel(v: Value) -> Value {
    let mut m = Map::new();
    m.insert("v", v);
    Value::Object(m)
}
fn color(c: [f64; 3]) -> Value {
    let bytes = c.map(|x| (math::linear_to_srgb(x).clamp(0.0, 1.0) * 255.0).round() as u8);
    Value::String(format!("#{:02x}{:02x}{:02x}", bytes[0], bytes[1], bytes[2]).into())
}

/// One asynchronous source-material snapshot. `renderSnapshot` starts all
/// of them before awaiting any: empty maps resume on microtasks, a new
/// texture resumes after `toBlob`. Preserve that completion order, since
/// it decides both texture ids and mesh order in the Blender snapshot.
struct Pending {
    object: usize,
    mesh: Value,
    maps: Vec<(&'static str, Option<GltfMap>)>,
    next: usize,
    material: Map,
    assigned: Map,
}
#[derive(Default)]
pub(super) struct Textures {
    ids: Vec<GltfMap>,
    pending: Vec<Pending>,
    pub assets: Vec<SnapshotAsset>,
}

impl Textures {
    fn texture(&mut self, map: &GltfMap) -> Result<(String, bool), String> {
        if let Some(i) = self.ids.iter().position(|m| m == map) {
            return Ok((format!("texture_{i}"), false));
        }
        let id = format!("texture_{}", self.ids.len());
        self.ids.push(map.clone());
        // GLTFLoader's images have no color conversion and straight alpha.
        // drawImage premultiplies into RGBA8, then toBlob unpremultiplies.
        let mut rgba = map.image.rgba.clone();
        for p in rgba.chunks_exact_mut(4) {
            for i in 0..3 {
                p[i] = ((u32::from(p[i]) * u32::from(p[3]) + 127) / 255) as u8;
            }
        }
        super::snapshot::unpremultiply(&mut rgba);
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, map.image.width, map.image.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .and_then(|mut w| w.write_image_data(&rgba))
                .map_err(|e| e.to_string())?;
        }
        self.assets.push(
            SnapshotAsset::new(
                id.clone(),
                format!("{id}.png"),
                AssetBytes::Bytes(Arc::new(png)),
            )
            .map_err(|e| e.to_string())?,
        );
        Ok((id, true))
    }

    fn start(&mut self, object: usize, mesh: Value, source: &Material3d) {
        let basic = source.kind == MaterialKind::Basic;
        let map = |slot| match material::map(source, slot) {
            Some(MapSource::Gltf(m)) => Some(m),
            _ => None,
        };
        let maps = if basic {
            vec![("emissive", map("map"))]
        } else {
            vec![
                ("color", map("map")),
                ("normal", map("normalMap")),
                ("roughness", map("roughnessMap")),
                ("metalness", map("metalnessMap")),
                ("emissive", map("emissiveMap")),
                ("ao", map("aoMap")),
            ]
        };
        let mut p = Map::new();
        p.insert(
            "color",
            channel(if basic {
                Value::String("#000000".into())
            } else {
                color(source.color)
            }),
        );
        p.insert(
            "roughness",
            channel(Value::Number(if basic { 1.0 } else { source.roughness })),
        );
        p.insert(
            "metalness",
            channel(Value::Number(if basic { 0.0 } else { source.metalness })),
        );
        p.insert(
            "emissive",
            channel(if basic {
                if maps[0].1.is_some() {
                    Value::String("#FFFFFF".into())
                } else {
                    color(source.color)
                }
            } else {
                color(source.emissive)
            }),
        );
        p.insert(
            "emissiveIntensity",
            channel(Value::Number(if basic {
                1.0
            } else {
                source.emissive_intensity
            })),
        );
        p.insert("opacity", channel(Value::Number(source.opacity)));
        let mut m = Map::new();
        m.insert("p", Value::Object(p));
        self.pending.push(Pending {
            object,
            mesh,
            maps,
            next: 0,
            material: m,
            assigned: Map::new(),
        });
    }

    pub fn finish(&mut self, objects: &mut [Value]) -> Result<(), String> {
        let mut tasks = std::mem::take(&mut self.pending);
        let mut ready: VecDeque<usize> = (0..tasks.len()).collect();
        let mut blobs = VecDeque::new();
        loop {
            while let Some(i) = ready.pop_front() {
                let task = &mut tasks[i];
                if task.next == task.maps.len() {
                    task.material
                        .insert("maps", Value::Object(std::mem::take(&mut task.assigned)));
                    task.mesh.as_object_mut().unwrap().insert(
                        "material",
                        Value::Object(std::mem::take(&mut task.material)),
                    );
                    objects[task.object]
                        .as_object_mut()
                        .unwrap()
                        .get_mut("meshes")
                        .unwrap()
                        .as_array_mut()
                        .unwrap()
                        .push(std::mem::replace(&mut task.mesh, Value::Null));
                    continue;
                }
                let (name, map) = &task.maps[task.next];
                task.next += 1;
                let waits = if let Some(map) = map {
                    let (id, fresh) = self.texture(map)?;
                    task.assigned.insert(*name, Value::String(id.into()));
                    fresh
                } else {
                    false
                };
                if waits {
                    blobs.push_back(i);
                } else {
                    ready.push_back(i);
                }
            }
            match blobs.pop_front() {
                Some(i) => ready.push_back(i),
                None => break,
            }
        }
        Ok(())
    }
}

/// Meshes of an imported instance, using the exact same pose and world
/// matrices as the native renderer. InstancedMesh is traversed once, as
/// today's snapshot does; its instance matrices are not expanded there.
pub(super) fn meshes(
    asset: &GltfAsset,
    world: &Mat4,
    time: f64,
    object: &Object3d,
    node: &Value,
    object_index: usize,
    textures: &mut Textures,
) -> Vec<Value> {
    if !object.visible {
        return Vec::new();
    }
    let model = &asset.model;
    let (parent, posed) = pose::placed(model, pose::Pose::at(model, time), world);
    let worlds = pose::world(model, &posed, Some(&parent));
    let use_source = node.get("useSourceMaterials").is_some_and(Value::is_truthy);
    let slot = node
        .get("slots")
        .and_then(Value::as_array)
        .and_then(|s| s.first());
    let mut out = Vec::new();
    let mut mesh_index = 0;
    for i in model.traversal() {
        let ObjectKind::Drawable(d) = &model.objects[i].kind else {
            continue;
        };
        let mesh = &object.meshes[mesh_index];
        mesh_index += 1;
        let g = &model.geometries[d.geometry];
        if g.mode != Mode::Triangles {
            continue;
        }
        let poser = pose::Poser::new(model, i, d, &posed.influences[i], &worlds);
        let count = g.index.as_ref().map_or(g.vertex_count(), Vec::len);
        let mut positions = Vec::with_capacity(count * 3);
        let mut normals = Vec::new();
        let mut uvs = Vec::with_capacity(count * 2);
        for k in 0..count {
            let v = g.index.as_ref().map_or(k, |ix| ix[k] as usize);
            if v >= g.vertex_count() {
                // `getVertexPosition` and `fromBufferAttribute` past the
                // last vertex read `undefined`, which the snapshot's JSON
                // writes as null (NaN here); `uv?.getX(index) || 0` is 0.
                positions.extend([f64::NAN; 3]);
                if g.normal.is_some() {
                    normals.extend([f64::NAN; 3]);
                }
                uvs.extend([0.0, 0.0]);
                continue;
            }
            let point = poser.vertex(v);
            positions.extend(point);
            if let Some(normal) = &g.normal {
                let mut dir = std::array::from_fn(|c| f64::from(normal.data[v * 3 + c]));
                if d.skinned {
                    let origin: [f64; 3] =
                        std::array::from_fn(|c| f64::from(g.position.data[v * 3 + c]));
                    dir = poser.apply_bone(v, std::array::from_fn(|c| dir[c] + origin[c]));
                    dir = std::array::from_fn(|c| dir[c] - point[c]);
                }
                normals.extend(math::normalize(dir));
            }
            for c in 0..2 {
                let v = g.uv[0]
                    .as_ref()
                    .map_or(0.0, |uv| f64::from(uv.data[v * 2 + c]));
                uvs.push(if v.is_nan() { 0.0 } else { v });
            }
        }
        let mut m = Map::new();
        m.insert("positions", numbers(positions));
        m.insert("normals", numbers(normals));
        m.insert("uvs", numbers(uvs));
        m.insert("matrix", numbers(mesh.world));
        m.insert("materialIndex", Value::Number(0.0));
        m.insert("smooth", Value::Bool(true));
        if use_source && slot.is_none() {
            textures.start(
                object_index,
                Value::Object(m),
                mesh.materials.iter().next().unwrap(),
            );
        } else {
            m.insert(
                "material",
                slot.and_then(|s| s.get("material"))
                    .or_else(|| node.get("material"))
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            out.push(Value::Object(m));
        }
    }
    out
}
