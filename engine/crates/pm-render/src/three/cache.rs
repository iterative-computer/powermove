//! The disk cache of translated 3D programs (amendment 11: "compiled
//! through the naga pipeline with a disk program cache").
//!
//! Translating one three.js program (glslang, the SPIR-V rewrites, naga,
//! the MSL back end) takes about 25 ms (S7); a scene's first frame would pay
//! it for every program variant. The cache stores what the GPU needs to
//! build a program without translating it again: both stages' MSL with
//! their entry points, and the reflection the pass uses (the default
//! uniform block's slots and the vertex attribute locations). Entries are
//! keyed by SHA-256 of the program sources, the math mode, and the
//! translation revision (`pm_gpu::msl::TRANSLATION_REVISION`) with naga's
//! version, so a changed translation never reads an old entry. Files are
//! written atomically (temporary file, then rename); an unreadable or
//! mismatched entry is a miss and is rewritten.
//!
//! Only the passthrough (safe math) path is cached: wgpu's own naga path
//! needs the naga module, which is not serialized.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use pm_gpu::glsl::{ScalarKind, UniformSlot};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Bumped when the entry layout changes.
const FORMAT: u32 = 1;

/// What a cached program holds.
#[derive(Debug, Clone, PartialEq)]
pub struct CachedProgram {
    pub block_size: u32,
    pub uniforms: HashMap<String, UniformSlot>,
    /// GL attribute name to shader location.
    pub attributes: HashMap<String, u32>,
    pub vertex_msl: String,
    pub vertex_entry: String,
    pub fragment_msl: String,
    pub fragment_entry: String,
}

/// A directory of cached programs.
#[derive(Debug, Clone)]
pub struct ProgramCache {
    dir: PathBuf,
}

/// The key of a program: hex SHA-256.
pub fn key(vertex: &str, fragment: &str, math: &str) -> String {
    let mut h = Sha256::new();
    for part in [
        format!("pm-render three program cache {FORMAT}").as_bytes(),
        format!("translation {}", pm_gpu::msl::TRANSLATION_REVISION).as_bytes(),
        b"naga 30.0.1",
        math.as_bytes(),
        vertex.as_bytes(),
        fragment.as_bytes(),
    ] {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn kind_name(k: ScalarKind) -> &'static str {
    match k {
        ScalarKind::Float => "f",
        ScalarKind::Sint => "i",
        ScalarKind::Uint => "u",
        ScalarKind::Bool => "b",
    }
}

fn kind_of(s: &str) -> Option<ScalarKind> {
    Some(match s {
        "f" => ScalarKind::Float,
        "i" => ScalarKind::Sint,
        "u" => ScalarKind::Uint,
        "b" => ScalarKind::Bool,
        _ => return None,
    })
}

impl CachedProgram {
    fn to_json(&self, key: &str) -> Value {
        let mut uniforms: Vec<(&String, &UniformSlot)> = self.uniforms.iter().collect();
        uniforms.sort_by(|a, b| a.0.cmp(b.0));
        let mut attributes: Vec<(&String, &u32)> = self.attributes.iter().collect();
        attributes.sort();
        json!({
            "format": FORMAT,
            "key": key,
            "blockSize": self.block_size,
            "uniforms": uniforms.into_iter().map(|(n, s)| json!([
                n, s.offset, kind_name(s.kind), s.columns, s.rows, s.column_stride, s.count, s.array_stride
            ])).collect::<Vec<_>>(),
            "attributes": attributes.into_iter().map(|(n, l)| json!([n, l])).collect::<Vec<_>>(),
            "vertex": { "msl": self.vertex_msl, "entry": self.vertex_entry },
            "fragment": { "msl": self.fragment_msl, "entry": self.fragment_entry },
        })
    }

    fn from_json(v: &Value, key: &str) -> Option<CachedProgram> {
        if v["format"].as_u64()? != u64::from(FORMAT) || v["key"].as_str()? != key {
            return None;
        }
        let u32_of = |v: &Value| v.as_u64().and_then(|n| u32::try_from(n).ok());
        let mut uniforms = HashMap::new();
        for u in v["uniforms"].as_array()? {
            let a = u.as_array()?;
            uniforms.insert(
                a.first()?.as_str()?.to_owned(),
                UniformSlot {
                    offset: u32_of(a.get(1)?)?,
                    kind: kind_of(a.get(2)?.as_str()?)?,
                    columns: u32_of(a.get(3)?)?,
                    rows: u32_of(a.get(4)?)?,
                    column_stride: u32_of(a.get(5)?)?,
                    count: u32_of(a.get(6)?)?,
                    array_stride: u32_of(a.get(7)?)?,
                },
            );
        }
        let mut attributes = HashMap::new();
        for a in v["attributes"].as_array()? {
            let a = a.as_array()?;
            attributes.insert(a.first()?.as_str()?.to_owned(), u32_of(a.get(1)?)?);
        }
        Some(CachedProgram {
            block_size: u32_of(&v["blockSize"])?,
            uniforms,
            attributes,
            vertex_msl: v["vertex"]["msl"].as_str()?.to_owned(),
            vertex_entry: v["vertex"]["entry"].as_str()?.to_owned(),
            fragment_msl: v["fragment"]["msl"].as_str()?.to_owned(),
            fragment_entry: v["fragment"]["entry"].as_str()?.to_owned(),
        })
    }
}

impl ProgramCache {
    /// A cache in `dir` (created on the first store).
    pub fn new(dir: impl Into<PathBuf>) -> ProgramCache {
        ProgramCache { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }

    /// The entry for `key`, or `None` (missing, unreadable, or another
    /// translation's).
    pub fn load(&self, key: &str) -> Option<CachedProgram> {
        let text = std::fs::read_to_string(self.path(key)).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        CachedProgram::from_json(&v, key)
    }

    /// Stores an entry atomically.
    pub fn store(&self, key: &str, program: &CachedProgram) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join(format!(".{key}.{}.tmp", std::process::id()));
        std::fs::write(&tmp, program.to_json(key).to_string())?;
        std::fs::rename(&tmp, self.path(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_round_trip_and_reject_other_keys() {
        let dir = std::env::temp_dir().join(format!("pm-render-cache-test-{}", std::process::id()));
        let cache = ProgramCache::new(&dir);
        let mut uniforms = HashMap::new();
        uniforms.insert(
            "directionalLights[0].color".to_owned(),
            UniformSlot {
                offset: 48,
                kind: ScalarKind::Float,
                columns: 1,
                rows: 3,
                column_stride: 16,
                count: 1,
                array_stride: 32,
            },
        );
        let p = CachedProgram {
            block_size: 640,
            uniforms,
            attributes: HashMap::from([("position".to_owned(), 0), ("normal".to_owned(), 1)]),
            vertex_msl: "#pragma METAL fp math_mode(safe)\nvertex".into(),
            vertex_entry: "main_".into(),
            fragment_msl: "fragment".into(),
            fragment_entry: "main_".into(),
        };
        let k = key("vs", "fs", "Safe");
        assert_ne!(k, key("vs", "fs", "Fast"));
        cache.store(&k, &p).unwrap();
        assert_eq!(cache.load(&k), Some(p));
        // A file under another key's name is not trusted.
        std::fs::copy(cache.path(&k), cache.path("other")).unwrap();
        assert_eq!(cache.load("other"), None);
        std::fs::write(cache.path(&k), "{not json").unwrap();
        assert_eq!(cache.load(&k), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
