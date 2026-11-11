//! Durable image-backed material maps. Live capture is a separate resource lane.
use super::*;
pub(super) fn overrides(object: &Value, index: Option<usize>, basic: bool) -> Value {
    let mut maps = object["material"]["maps"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    if let Some(indexed) = index.and_then(|i| object["sourceMaterials"][i.to_string()].as_object())
    {
        maps.extend(indexed.clone());
    }
    if basic {
        let source = maps.get("emissive").or_else(|| maps.get("color")).cloned();
        maps.clear();
        if let Some(source) = source {
            maps.insert("color".into(), source.clone());
            maps.insert("emissive".into(), source);
        }
    }
    serde_json::json!({"maps": maps})
}
impl DocumentSource {
    fn material_image(&self, source: &Value) -> Result<Arc<Vec<u8>>, String> {
        let id = source
            .as_str()
            .ok_or("Live layer/group material capture is not connected yet")?;
        let path = self
            .media
            .paths
            .get(id)
            .ok_or_else(|| format!("Material image is not loaded: {id}"))?;
        let mut cache = self
            .material_images
            .lock()
            .map_err(|_| "Material image cache poisoned")?;
        if let Some(index) = cache.iter().position(|(key, _)| key == id) {
            let entry = cache.remove(index);
            let pixels = entry.1.clone();
            cache.push(entry);
            return Ok(pixels);
        }
        use std::io::Read;
        let mut bytes = vec![];
        std::fs::File::open(path)
            .map_err(|e| format!("Material image: {e}"))?
            .take(pm_scene3d::import::MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > pm_scene3d::import::MAX_BYTES {
            return Err("Material image exceeds the import byte budget".into());
        }
        let pixels = pm_scene3d::render::texture_rgba(&bytes).map_err(|e| e.to_string())?;
        cache.push((id.into(), pixels.clone()));
        if cache.len() > 16 {
            cache.remove(0);
        }
        Ok(pixels)
    }
    pub(super) fn bind_material_maps(
        &self,
        material: &Value,
        gpu: &mut GpuMaterial,
        layers: &mut Vec<Vec<u8>>,
        indices: &mut HashMap<(String, usize), usize>,
    ) -> Result<(), String> {
        for (slot, source) in material["maps"]
            .as_object()
            .into_iter()
            .flat_map(|m| m.iter())
        {
            if source.is_null() {
                continue;
            }
            let id = source
                .as_str()
                .ok_or("Live layer/group material capture is not connected yet")?;
            let key = (format!("image:{id}"), 0);
            let index = if let Some(index) = indices.get(&key) {
                *index
            } else {
                if layers.len() >= 256 {
                    return Err("Studio map array exceeds 256 layers".into());
                }
                let pixels = self.material_image(source)?;
                let index = layers.len();
                layers.push((*pixels).clone());
                indices.insert(key, index);
                index
            } as f32;
            match slot.as_str() {
                "color" => gpu.maps[0] = index,
                "emissive" => gpu.maps[1] = index,
                "normal" => gpu.maps[3] = index,
                "roughness" => gpu.maps2[0] = index,
                "metalness" => gpu.maps2[1] = index,
                "ao" => gpu.maps2[2] = index,
                _ => return Err("Unknown Studio material map".into()),
            }
            if gpu.imported[5][3] > 0.5 {
                let slot = match slot.as_str() {
                    "color" => 0,
                    "emissive" => 1,
                    "roughness" => 2,
                    "metalness" => 3,
                    "normal" => 4,
                    "ao" => 5,
                    _ => unreachable!(),
                };
                // Image replacement inherits the original slot's coordinates,
                // matching textureCoordinates(original) in the source runtime.
                let mut map = gpu.physical_maps[slot];
                if map.info[0] < 0. {
                    map = pm_studio3d_engine::scene::GpuTextureMap {
                        info: [index, 0., 0., 0.],
                        u: [1., 0., 0., 0.],
                        v: [0., 1., 0., 0.],
                    };
                }
                map.info[0] = index;
                gpu.physical_maps[slot] = map;
            }
        }
        Ok(())
    }
}
