//! Durable image-backed material maps. Live capture is a separate resource lane.
use super::*;
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
        }
        Ok(())
    }
}
