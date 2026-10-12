//! The registry boundary as data (plan section 12).
//!
//! The native registry answers resolver queries with CoreText and records
//! every answer. `Answers` is that record in a serializable shape: the faces
//! it handed out (by blob key, never by bytes), each query with its answer,
//! the strike-sized emoji advances, and the variable-font sources.
//! `ReplayResolver` answers the same queries from `Answers` plus font bytes
//! (full fonts or metrics-and-shaping subsets). This is how `pm-wasm` lays
//! text out without platform font APIs: main owns font resolution, the
//! renderer owns layout.
//!
//! A replay can meet a query the answers lack (a new string needs a fallback
//! font nobody asked for yet) or a face whose bytes are still loading. Both
//! are reported (`misses`, `pending_blobs`) instead of being guessed: the
//! layout continues with the next font in Blink's order, the host asks main
//! for the missing answers and subsets, merges them (`add_answers`,
//! `add_font`), and lays the text out again.

use crate::font::{FaceFacts, FaceId, FaceInfo, FontBytes, FontDesc, FontResolver, WebDescriptors};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A resolver query, keyed exactly (floats by their bits) so a replay
/// answers precisely what the registry was asked.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Query {
    Family { name: String, weight: u32, italic: bool, size: u32 },
    Standard { weight: u32, italic: bool, size: u32 },
    Fallback { weight: u32, italic: bool, size: u32, ch: u32, primary: FaceId, emoji: bool },
    LastResort { weight: u32, italic: bool, size: u32 },
    Variable { family: String },
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
    pub fn variable(family: &str) -> Self {
        Query::Variable { family: family.to_string() }
    }
    pub fn em_advance(face: FaceId, size: f32) -> Self {
        Query::EmAdvance { face, size: size.to_bits() }
    }

    /// The font description the query was asked with, if it has one.
    pub fn desc(&self) -> Option<FontDesc> {
        let d = |weight: u32, italic: bool, size: u32| Some(FontDesc { weight: f32::from_bits(weight), italic, size: f32::from_bits(size) });
        match *self {
            Query::Family { weight, italic, size, .. }
            | Query::Standard { weight, italic, size }
            | Query::Fallback { weight, italic, size, .. }
            | Query::LastResort { weight, italic, size } => d(weight, italic, size),
            Query::Variable { .. } | Query::EmAdvance { .. } => None,
        }
    }
}

/// Serializable face description; `blob` and `index` name the bytes the face
/// reads (`subset_key` names its subset).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceRecord {
    pub id: FaceId,
    pub blob: String,
    pub index: u32,
    pub coords: Vec<(u32, f32)>,
    pub synthetic_bold: bool,
    pub synthetic_italic: bool,
    pub custom: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<WebDescriptors>,
    pub synthesized_hyphens: [Option<u16>; 2],
    pub postscript_name: String,
    pub typeface_key: String,
    #[serde(default)]
    pub fallback_key: Option<String>,
    pub facts: FaceFacts,
}

impl FaceRecord {
    pub fn of(id: FaceId, info: &FaceInfo) -> Self {
        FaceRecord {
            id,
            blob: info.blob.clone(),
            index: info.index,
            coords: info.coords.clone(),
            synthetic_bold: info.synthetic_bold,
            synthetic_italic: info.synthetic_italic,
            custom: info.custom,
            web: info.web,
            synthesized_hyphens: info.synthesized_hyphens,
            postscript_name: info.postscript_name.clone(),
            typeface_key: info.typeface_key.clone(),
            fallback_key: info.fallback_key.clone(),
            facts: info.facts,
        }
    }

    fn info(&self, data: FontBytes, index: u32) -> FaceInfo {
        FaceInfo {
            data,
            index,
            blob: self.blob.clone(),
            coords: self.coords.clone(),
            synthetic_bold: self.synthetic_bold,
            synthetic_italic: self.synthetic_italic,
            custom: self.custom,
            web: self.web,
            synthesized_hyphens: self.synthesized_hyphens,
            postscript_name: self.postscript_name.clone(),
            typeface_key: self.typeface_key.clone(),
            fallback_key: self.fallback_key.clone(),
            facts: self.facts,
        }
    }
}

/// The key a face's metrics-and-shaping subset is served under. A subset is
/// a single-face font, so it is always read at index 0.
pub fn subset_key(blob: &str, index: u32) -> String {
    format!("{blob}.{index}")
}

/// One query and its answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub query: Query,
    #[serde(default)]
    pub face: Option<FaceId>,
    /// `EmAdvance` answers carry a value instead of a face.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advance: Option<f32>,
}

/// Registry answers in a serializable shape. Faces are listed once; answers
/// are sorted by query, so two runs that asked the same questions serialize
/// identically.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Answers {
    pub faces: Vec<FaceRecord>,
    pub answers: Vec<Answer>,
}

impl Answers {
    /// Adds `other`'s faces and answers (an incremental reply from main).
    pub fn merge(&mut self, other: &Answers) {
        let known: BTreeSet<FaceId> = self.faces.iter().map(|f| f.id).collect();
        self.faces.extend(other.faces.iter().filter(|f| !known.contains(&f.id)).cloned());
        self.faces.sort_by_key(|f| f.id);
        let mut by_query: BTreeMap<Query, Answer> = self.answers.drain(..).map(|a| (a.query.clone(), a)).collect();
        for a in &other.answers {
            by_query.insert(a.query.clone(), a.clone());
        }
        self.answers = by_query.into_values().collect();
    }

    /// The distinct (blob, index) pairs the faces read.
    pub fn sources(&self) -> BTreeSet<(String, u32)> {
        self.faces.iter().map(|f| (f.blob.clone(), f.index)).collect()
    }
}

/// Answers resolver queries from `Answers` and the font bytes added so far.
pub struct ReplayResolver {
    records: Vec<Option<FaceRecord>>,
    faces: Vec<Option<FaceInfo>>,
    answers: HashMap<Query, Answer>,
    /// Loaded fonts by (blob, index): the bytes and the face index to read
    /// them at (0 for subsets).
    fonts: HashMap<(String, u32), (FontBytes, u32)>,
    /// Queries the answers did not contain, in the order they were met.
    pub misses: Vec<Query>,
    /// Faces an answer named whose bytes have not been added, by (blob, index).
    pub pending_blobs: BTreeSet<(String, u32)>,
}

impl ReplayResolver {
    pub fn new(answers: &Answers) -> Self {
        let mut r = ReplayResolver {
            records: Vec::new(),
            faces: Vec::new(),
            answers: HashMap::new(),
            fonts: HashMap::new(),
            misses: Vec::new(),
            pending_blobs: BTreeSet::new(),
        };
        r.add_answers(answers);
        r
    }

    /// Merges more answers (main's reply to `misses`).
    pub fn add_answers(&mut self, answers: &Answers) {
        for rec in &answers.faces {
            let id = rec.id as usize;
            if self.records.len() <= id {
                self.records.resize(id + 1, None);
                self.faces.resize(id + 1, None);
            }
            if self.records[id].is_none() {
                if let Some((data, index)) = self.fonts.get(&(rec.blob.clone(), rec.index)) {
                    self.faces[id] = Some(rec.info(data.clone(), *index));
                }
                self.records[id] = Some(rec.clone());
            }
        }
        for a in &answers.answers {
            self.answers.insert(a.query.clone(), a.clone());
        }
    }

    /// Adds the bytes of face `index` of font `blob`. `read_index` is the face
    /// index within `data`: 0 for a metrics-and-shaping subset, `index` for the
    /// full font.
    pub fn add_font(&mut self, blob: &str, index: u32, data: FontBytes, read_index: u32) {
        let key = (blob.to_string(), index);
        self.pending_blobs.remove(&key);
        for (rec, face) in self.records.iter().zip(self.faces.iter_mut()) {
            if let Some(rec) = rec {
                if rec.blob == blob && rec.index == index {
                    *face = Some(rec.info(data.clone(), read_index));
                }
            }
        }
        self.fonts.insert(key, (data, read_index));
    }

    /// Clears `misses` and `pending_blobs` before a new layout pass.
    pub fn clear_reports(&mut self) {
        self.misses.clear();
        self.pending_blobs.clear();
    }

    fn usable(&mut self, face: Option<FaceId>) -> Option<FaceId> {
        let id = face?;
        match self.faces.get(id as usize) {
            Some(Some(_)) => Some(id),
            _ => {
                if let Some(Some(rec)) = self.records.get(id as usize) {
                    self.pending_blobs.insert((rec.blob.clone(), rec.index));
                }
                None
            }
        }
    }

    fn answer(&mut self, q: Query) -> Option<FaceId> {
        match self.answers.get(&q) {
            Some(a) => {
                let face = a.face;
                self.usable(face)
            }
            None => {
                self.misses.push(q);
                None
            }
        }
    }
}

impl FontResolver for ReplayResolver {
    fn face(&self, id: FaceId) -> &FaceInfo {
        // Only ids `usable` returned reach the layout.
        self.faces[id as usize].as_ref().expect("face bytes are loaded")
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
    fn variable_face(&mut self, family: &str) -> Option<FaceId> {
        self.answer(Query::variable(family))
    }
    fn em_advance(&mut self, face: FaceId, size: f32) -> Option<f32> {
        let q = Query::em_advance(face, size);
        match self.answers.get(&q) {
            Some(a) => a.advance,
            None => {
                self.misses.push(q);
                None
            }
        }
    }
}

/// Wraps a resolver and records every answer it gives (the native side of
/// the boundary). `faces` lists the faces answers named, for `Answers`.
pub struct Recorder<R> {
    pub inner: R,
    answers: BTreeMap<Query, Answer>,
}

impl<R: FontResolver> Recorder<R> {
    pub fn new(inner: R) -> Self {
        Recorder { inner, answers: BTreeMap::new() }
    }

    fn record(&mut self, query: Query, face: Option<FaceId>) -> Option<FaceId> {
        self.answers.insert(query.clone(), Answer { query, face, advance: None });
        face
    }

    /// Everything recorded so far, with the records of every face named.
    pub fn answers(&self) -> Answers {
        let mut ids: BTreeSet<FaceId> = BTreeSet::new();
        for a in self.answers.values() {
            if let Some(f) = a.face {
                ids.insert(f);
            }
            if let Query::Fallback { primary, .. } = a.query {
                ids.insert(primary);
            }
            if let Query::EmAdvance { face, .. } = a.query {
                ids.insert(face);
            }
        }
        Answers {
            faces: ids.into_iter().map(|id| FaceRecord::of(id, self.inner.face(id))).collect(),
            answers: self.answers.values().cloned().collect(),
        }
    }

    /// Forgets recorded answers (the faces stay with the inner resolver).
    pub fn clear(&mut self) {
        self.answers.clear();
    }
}

impl<R: FontResolver> FontResolver for Recorder<R> {
    fn face(&self, id: FaceId) -> &FaceInfo {
        self.inner.face(id)
    }
    fn family(&mut self, family: &str, desc: &FontDesc) -> Option<FaceId> {
        let a = self.inner.family(family, desc);
        self.record(Query::family(family, desc), a)
    }
    fn standard(&mut self, desc: &FontDesc) -> Option<FaceId> {
        let a = self.inner.standard(desc);
        self.record(Query::standard(desc), a)
    }
    fn fallback_for_char(&mut self, desc: &FontDesc, ch: u32, primary: FaceId, emoji: bool) -> Option<FaceId> {
        let a = self.inner.fallback_for_char(desc, ch, primary, emoji);
        self.record(Query::fallback(desc, ch, primary, emoji), a)
    }
    fn last_resort(&mut self, desc: &FontDesc) -> Option<FaceId> {
        let a = self.inner.last_resort(desc);
        self.record(Query::last_resort(desc), a)
    }
    fn variable_face(&mut self, family: &str) -> Option<FaceId> {
        let a = self.inner.variable_face(family);
        self.record(Query::variable(family), a)
    }
    fn em_advance(&mut self, face: FaceId, size: f32) -> Option<f32> {
        let v = self.inner.em_advance(face, size);
        let query = Query::em_advance(face, size);
        self.answers.insert(query.clone(), Answer { query, face: None, advance: v });
        v
    }
}

/// Answers one query against a resolver (main answering a replay's misses).
pub fn answer_query(resolver: &mut dyn FontResolver, q: &Query) -> Answer {
    let face = match q {
        Query::Family { name, .. } => resolver.family(name, &q.desc().expect("family query")),
        Query::Standard { .. } => resolver.standard(&q.desc().expect("standard query")),
        Query::Fallback { ch, primary, emoji, .. } => resolver.fallback_for_char(&q.desc().expect("fallback query"), *ch, *primary, *emoji),
        Query::LastResort { .. } => resolver.last_resort(&q.desc().expect("last resort query")),
        Query::Variable { family } => resolver.variable_face(family),
        Query::EmAdvance { face, size } => {
            return Answer { query: q.clone(), face: None, advance: resolver.em_advance(*face, f32::from_bits(*size)) };
        }
    };
    Answer { query: q.clone(), face, advance: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: FaceId, blob: &str) -> FaceRecord {
        FaceRecord {
            id,
            blob: blob.into(),
            index: 0,
            coords: vec![],
            synthetic_bold: false,
            synthetic_italic: false,
            custom: false,
            web: None,
            synthesized_hyphens: [None, None],
            postscript_name: format!("Face{id}"),
            typeface_key: format!("Face{id}"),
            fallback_key: None,
            facts: FaceFacts::default(),
        }
    }

    fn desc() -> FontDesc {
        FontDesc { weight: 500.0, italic: false, size: 16.0 }
    }

    #[test]
    fn misses_and_pending_blobs_are_reported_then_resolved() {
        let answers = Answers {
            faces: vec![rec(0, "a")],
            answers: vec![Answer { query: Query::family("A", &desc()), face: Some(0), advance: None }],
        };
        let mut r = ReplayResolver::new(&answers);
        // Bytes not loaded yet: the layout must not see the face.
        assert_eq!(r.family("A", &desc()), None);
        assert!(r.pending_blobs.contains(&("a".to_string(), 0)));
        assert_eq!(r.family("B", &desc()), None);
        assert_eq!(r.misses, vec![Query::family("B", &desc())]);
        r.add_font("a", 0, FontBytes::new(vec![]), 0);
        r.clear_reports();
        assert_eq!(r.family("A", &desc()), Some(0));
        assert!(r.pending_blobs.is_empty());
        r.add_answers(&Answers {
            faces: vec![rec(1, "a")],
            answers: vec![Answer { query: Query::family("B", &desc()), face: Some(1), advance: None }],
        });
        assert_eq!(r.family("B", &desc()), Some(1));
    }

    #[test]
    fn answers_serialize_with_stable_order() {
        let mut a = Answers::default();
        a.merge(&Answers {
            faces: vec![rec(1, "b"), rec(0, "a")],
            answers: vec![
                Answer { query: Query::variable("Z"), face: None, advance: None },
                Answer { query: Query::family("A", &desc()), face: Some(0), advance: None },
            ],
        });
        assert_eq!(a.faces.iter().map(|f| f.id).collect::<Vec<_>>(), [0, 1]);
        let json = serde_json::to_string(&a).unwrap();
        let back: Answers = serde_json::from_str(&json).unwrap();
        assert_eq!(back, a);
        assert!(json.find("\"family\"").unwrap() < json.find("\"variable\"").unwrap());
    }
}
