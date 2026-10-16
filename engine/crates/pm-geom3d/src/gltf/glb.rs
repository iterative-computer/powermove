//! A GLB writer (glTF 2.0 binary container): one BIN buffer, buffer views
//! aligned to 4 bytes, accessors with min and max where glTF requires them.

use serde_json::{Map, Value, json};

/// Builds the JSON and BIN chunks of a GLB.
#[derive(Default)]
pub struct GlbWriter {
    bin: Vec<u8>,
    pub views: Vec<Value>,
    pub accessors: Vec<Value>,
}

/// glTF component types.
pub const FLOAT: u32 = 5126;
pub const UNSIGNED_SHORT: u32 = 5123;
pub const UNSIGNED_INT: u32 = 5125;
/// Buffer view targets.
pub const ARRAY_BUFFER: u32 = 34962;
pub const ELEMENT_ARRAY_BUFFER: u32 = 34963;

impl GlbWriter {
    /// A buffer view over `bytes`, 4-byte aligned.
    pub fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let mut v = Map::new();
        v.insert("buffer".into(), json!(0));
        v.insert("byteOffset".into(), json!(self.bin.len()));
        v.insert("byteLength".into(), json!(bytes.len()));
        if let Some(t) = target {
            v.insert("target".into(), json!(t));
        }
        self.bin.extend_from_slice(bytes);
        self.views.push(Value::Object(v));
        self.views.len() - 1
    }

    /// A float accessor of `N`-component items (with min and max when asked).
    pub fn floats<const N: usize>(&mut self, items: &[[f32; N]], bounds: bool) -> usize {
        let bytes: Vec<u8> = items
            .iter()
            .flat_map(|v| v.iter().flat_map(|c| c.to_le_bytes()))
            .collect();
        let view = self.view(&bytes, Some(ARRAY_BUFFER));
        let kind = match N {
            1 => "SCALAR",
            2 => "VEC2",
            3 => "VEC3",
            4 => "VEC4",
            16 => "MAT4",
            _ => panic!("no glTF type has {N} components"),
        };
        let mut a = Map::new();
        a.insert("bufferView".into(), json!(view));
        a.insert("componentType".into(), json!(FLOAT));
        a.insert("count".into(), json!(items.len()));
        a.insert("type".into(), json!(kind));
        if bounds && !items.is_empty() {
            let min: Vec<f32> = (0..N)
                .map(|c| items.iter().map(|v| v[c]).fold(f32::INFINITY, f32::min))
                .collect();
            let max: Vec<f32> = (0..N)
                .map(|c| items.iter().map(|v| v[c]).fold(f32::NEG_INFINITY, f32::max))
                .collect();
            a.insert("min".into(), json!(min));
            a.insert("max".into(), json!(max));
        }
        self.accessors.push(Value::Object(a));
        self.accessors.len() - 1
    }

    /// An index accessor (u16 when every index fits, else u32).
    pub fn indices(&mut self, indices: &[u32]) -> usize {
        let small = indices.iter().all(|&i| i < 65535);
        let bytes: Vec<u8> = if small {
            indices
                .iter()
                .flat_map(|&i| (i as u16).to_le_bytes())
                .collect()
        } else {
            indices.iter().flat_map(|i| i.to_le_bytes()).collect()
        };
        let view = self.view(&bytes, Some(ELEMENT_ARRAY_BUFFER));
        self.accessors.push(json!({
            "bufferView": view,
            "componentType": if small { UNSIGNED_SHORT } else { UNSIGNED_INT },
            "count": indices.len(),
            "type": "SCALAR",
        }));
        self.accessors.len() - 1
    }

    /// The GLB file for `document` (its `buffers`, `bufferViews`, and
    /// `accessors` are set here).
    pub fn finish(mut self, mut document: Map<String, Value>) -> Vec<u8> {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        if !self.bin.is_empty() {
            document.insert("buffers".into(), json!([{ "byteLength": self.bin.len() }]));
            document.insert("bufferViews".into(), Value::Array(self.views));
        }
        if !self.accessors.is_empty() {
            document.insert("accessors".into(), Value::Array(self.accessors));
        }
        let mut text = serde_json::to_vec(&Value::Object(document)).expect("JSON");
        while !text.len().is_multiple_of(4) {
            text.push(b' ');
        }
        let total = 12 + 8 + text.len() + if self.bin.is_empty() { 0 } else { 8 + self.bin.len() };
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(text.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&text);
        if !self.bin.is_empty() {
            out.extend_from_slice(&(self.bin.len() as u32).to_le_bytes());
            out.extend_from_slice(b"BIN\0");
            out.extend_from_slice(&self.bin);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_glb_the_loader_reads() {
        let mut w = GlbWriter::default();
        let pos = w.floats(&[[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], true);
        let idx = w.indices(&[0, 1, 2]);
        let mut doc = Map::new();
        doc.insert("asset".into(), json!({ "version": "2.0" }));
        doc.insert(
            "meshes".into(),
            json!([{ "primitives": [{ "attributes": { "POSITION": pos }, "indices": idx }] }]),
        );
        doc.insert("nodes".into(), json!([{ "mesh": 0 }]));
        doc.insert("scenes".into(), json!([{ "nodes": [0] }]));
        let glb = w.finish(doc);
        assert_eq!(glb.len() % 4, 0);
        let (model, _, facts) = crate::gltf::import(&glb).unwrap();
        assert_eq!(model.meshes[0].primitives[0].indices.as_deref(), Some(&[0, 1, 2][..]));
        assert_eq!((facts.vertices, facts.triangles), (3, 1));
    }
}
