//! The registry boundary as data. The native registry records every
//! resolver query with its answer; `ReplayResolver` answers the same queries
//! from that log plus font bytes (full fonts or metrics-and-shaping subsets).
//! This is how `pm-wasm` lays out text without platform font APIs: main owns
//! font resolution, the renderer owns layout.

use crate::font::{FaceId, FaceInfo, FontBytes, FontDesc, FontResolver};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Query {
    Family { name: String, weight: u32, italic: bool, size: u32 },
    Standard { weight: u32, italic: bool, size: u32 },
    Fallback { weight: u32, italic: bool, size: u32, ch: u32, primary: FaceId, emoji: bool },
    LastResort { weight: u32, italic: bool, size: u32 },
    EmAdvance { face: FaceId, size: u32 },
}

impl Query {
    pub fn family(name: &str, d: &FontDesc) -> Self {
        Query::Family { name: name.to_string(), weight: d.weight.to_bits(), italic: d.italic, size: d.size.to_bits() }
    }
    pub fn standard(d: &FontDesc) -> Self {
        Query::Standard { weight: d.weight.to_bits(), italic: d.italic, size: d.size.to_bits() }
    }
    pub fn fallback(d: &FontDesc, ch: u32, primary: FaceId, emoji: bool) -> Self {
        Query::Fallback { weight: d.weight.to_bits(), italic: d.italic, size: d.size.to_bits(), ch, primary, emoji }
    }
    pub fn last_resort(d: &FontDesc) -> Self {
        Query::LastResort { weight: d.weight.to_bits(), italic: d.italic, size: d.size.to_bits() }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Answer {
    pub face: Option<FaceId>,
}

#[derive(Clone, Debug, Default)]
pub struct ResolverLog {
    pub answers: HashMap<Query, Answer>,
    /// `em_advance` answers by (face, size bits).
    pub em_advances: HashMap<(FaceId, u32), Option<f32>>,
    /// Faces registered from bytes (`FontFace`), in registration order.
    pub custom: Vec<FaceId>,
}

/// Serializable face description; `blob` names the bytes the face reads.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FaceRecord {
    pub id: FaceId,
    pub blob: String,
    pub index: u32,
    pub coords: Vec<(u32, f32)>,
    pub synthetic_bold: bool,
    pub synthetic_italic: bool,
    pub custom: bool,
    pub synthesized_hyphens: [Option<u16>; 2],
    pub postscript_name: String,
    pub typeface_key: String,
    #[serde(default)]
    pub fallback_key: Option<String>,
}

/// The log in a serializable shape.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LogFile {
    pub faces: Vec<FaceRecord>,
    pub answers: Vec<(Query, Option<FaceId>)>,
    pub custom: Vec<FaceId>,
    /// Variable-font families registered as `FontFace`s, by family name.
    #[serde(default)]
    pub custom_names: Vec<(String, FaceId)>,
    #[serde(default)]
    pub em_advances: Vec<(FaceId, u32, Option<f32>)>,
}

impl LogFile {
    pub fn from_log(log: &ResolverLog, faces: &[(FaceId, FaceRecord)]) -> Self {
        let mut answers: Vec<(Query, Option<FaceId>)> = log.answers.iter().map(|(q, a)| (q.clone(), a.face)).collect();
        answers.sort_by(|a, b| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)));
        let mut em_advances: Vec<(FaceId, u32, Option<f32>)> = log.em_advances.iter().map(|(&(f, s), &v)| (f, s, v)).collect();
        em_advances.sort_by_key(|e| (e.0, e.1));
        LogFile { faces: faces.iter().map(|(_, r)| r.clone()).collect(), answers, custom: log.custom.clone(), custom_names: Vec::new(), em_advances }
    }
}

/// Answers resolver queries from a recorded log.
pub struct ReplayResolver {
    faces: Vec<FaceInfo>,
    answers: HashMap<Query, Option<FaceId>>,
    em_advances: HashMap<(FaceId, u32), Option<f32>>,
    custom: Vec<FaceId>,
    next_custom: usize,
    /// Queries the log did not contain (a replay miss is a bug in the
    /// prefetch, never silently answered).
    pub misses: Vec<Query>,
}

impl ReplayResolver {
    /// `blob` maps a record's blob name to its bytes.
    pub fn new(file: &LogFile, mut blob: impl FnMut(&str) -> FontBytes) -> Self {
        let mut faces: Vec<Option<FaceInfo>> = vec![None; file.faces.iter().map(|f| f.id as usize + 1).max().unwrap_or(0)];
        let mut cache: HashMap<String, FontBytes> = HashMap::new();
        for r in &file.faces {
            let data = cache.entry(r.blob.clone()).or_insert_with(|| blob(&r.blob)).clone();
            faces[r.id as usize] = Some(FaceInfo {
                data,
                index: r.index,
                coords: r.coords.clone(),
                synthetic_bold: r.synthetic_bold,
                synthetic_italic: r.synthetic_italic,
                custom: r.custom,
                synthesized_hyphens: r.synthesized_hyphens,
                postscript_name: r.postscript_name.clone(),
                typeface_key: r.typeface_key.clone(),
                fallback_key: r.fallback_key.clone(),
            });
        }
        ReplayResolver {
            faces: faces.into_iter().map(|f| f.expect("face ids are dense")).collect(),
            answers: file.answers.iter().cloned().collect(),
            em_advances: file.em_advances.iter().map(|&(f, s, v)| ((f, s), v)).collect(),
            custom: file.custom.clone(),
            next_custom: 0,
            misses: Vec::new(),
        }
    }

    fn answer(&mut self, q: Query) -> Option<FaceId> {
        match self.answers.get(&q) {
            Some(a) => *a,
            None => {
                self.misses.push(q);
                None
            }
        }
    }
}

impl FontResolver for ReplayResolver {
    fn face(&self, id: FaceId) -> &FaceInfo {
        &self.faces[id as usize]
    }
    fn family(&mut self, family: &str, desc: &FontDesc) -> Option<FaceId> {
        self.answer(Query::family(family, desc))
    }
    fn standard(&mut self, desc: &FontDesc) -> Option<FaceId> {
        self.answer(Query::standard(desc))
    }
    fn fallback_for_char(&mut self, desc: &FontDesc, ch: u32, primary: FaceId, emoji: bool) -> Option<FaceId> {
        self.answer(Query::fallback(desc, ch, primary, emoji))
    }
    fn last_resort(&mut self, desc: &FontDesc) -> Option<FaceId> {
        self.answer(Query::last_resort(desc))
    }
    fn em_advance(&mut self, face: FaceId, size: f32) -> Option<f32> {
        match self.em_advances.get(&(face, size.to_bits())) {
            Some(v) => *v,
            None => {
                self.misses.push(Query::EmAdvance { face, size: size.to_bits() });
                None
            }
        }
    }
    fn register_custom(&mut self, _data: FontBytes, _index: u32) -> FaceId {
        // Custom faces were registered natively in the same order.
        let id = self.custom[self.next_custom % self.custom.len().max(1)];
        self.next_custom += 1;
        id
    }
}
