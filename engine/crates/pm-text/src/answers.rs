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
    Family {
        name: String,
        weight: u32,
        italic: bool,
        size: u32,
    },
    Standard {
        weight: u32,
        italic: bool,
        size: u32,
    },
    Fallback {
        weight: u32,
        italic: bool,
        size: u32,
        ch: u32,
        primary: FaceId,
        emoji: bool,
    },
    LastResort {
        weight: u32,
        italic: bool,
        size: u32,
    },
    Variable {
        family: String,
    },
    EmAdvance {
        face: FaceId,
        size: u32,
    },
    SbixMetric {
        face: FaceId,
        glyph: u32,
        size: u32,
    },
}

impl Query {
    pub fn family(name: &str, d: &FontDesc) -> Self {
        Query::Family {
            name: name.to_string(),
            weight: d.weight.to_bits(),
            italic: d.italic,
            size: d.size.to_bits(),
        }
    }
    pub fn standard(d: &FontDesc) -> Self {
        Query::Standard {
            weight: d.weight.to_bits(),
            italic: d.italic,
            size: d.size.to_bits(),
        }
    }
    pub fn fallback(d: &FontDesc, ch: u32, primary: FaceId, emoji: bool) -> Self {
        Query::Fallback {
            weight: d.weight.to_bits(),
            italic: d.italic,
            size: d.size.to_bits(),
            ch,
            primary,
            emoji,
        }
    }
    pub fn last_resort(d: &FontDesc) -> Self {
        Query::LastResort {
            weight: d.weight.to_bits(),
            italic: d.italic,
            size: d.size.to_bits(),
        }
    }
    pub fn variable(family: &str) -> Self {
        Query::Variable {
            family: family.to_string(),
        }
    }
    pub fn em_advance(face: FaceId, size: f32) -> Self {
        Query::EmAdvance {
            face,
            size: size.to_bits(),
        }
    }
    pub fn sbix_metric(face: FaceId, glyph: u32, size: f32) -> Self {
        Query::SbixMetric {
            face,
            glyph,
            size: size.to_bits(),
        }
    }

    /// The font description the query was asked with, if it has one.
    pub fn desc(&self) -> Option<FontDesc> {
        let d = |weight: u32, italic: bool, size: u32| {
            Some(FontDesc {
                weight: f32::from_bits(weight),
                italic,
                size: f32::from_bits(size),
            })
        };
        match *self {
            Query::Family {
                weight,
                italic,
                size,
                ..
            }
            | Query::Standard {
                weight,
                italic,
                size,
            }
            | Query::Fallback {
                weight,
                italic,
                size,
                ..
            }
            | Query::LastResort {
                weight,
                italic,
                size,
            } => d(weight, italic, size),
            Query::Variable { .. } | Query::EmAdvance { .. } | Query::SbixMetric { .. } => None,
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

    pub(crate) fn info(&self, data: FontBytes, index: u32) -> FaceInfo {
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
    /// `SbixMetric` answers carry CoreText's rectangle instead of a face.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<[f64; 4]>,
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
        self.faces.extend(
            other
                .faces
                .iter()
                .filter(|f| !known.contains(&f.id))
                .cloned(),
        );
        self.faces.sort_by_key(|f| f.id);
        let mut by_query: BTreeMap<Query, Answer> = self
            .answers
            .drain(..)
            .map(|a| (a.query.clone(), a))
            .collect();
        for a in &other.answers {
            by_query.insert(a.query.clone(), a.clone());
        }
        self.answers = by_query.into_values().collect();
    }

    /// The registry's reply to the renderer: these answers with the registry
    /// generation as the **last** key, `{"faces":…,"answers":…,"generation":N}`.
    ///
    /// Main reads the generation off the end of the reply without parsing
    /// it (`answerGeneration` in apps/desktop/src/main/engine-fonts.ts), so
    /// the key order is part of the format. It is written here explicitly,
    /// not left to serde_json's map ordering (which `preserve_order` or a
    /// new field would change).
    pub fn reply_json(&self, generation: u64) -> Result<String, serde_json::Error> {
        let mut json = serde_json::to_string(self)?;
        // A struct with fields: `{…}`, never `{}`.
        debug_assert!(json.ends_with('}') && json.len() > 2);
        json.pop();
        json.push_str(",\"generation\":");
        json.push_str(&generation.to_string());
        json.push('}');
        Ok(json)
    }

    /// The distinct (blob, index) pairs the faces read.
    pub fn sources(&self) -> BTreeSet<(String, u32)> {
        self.faces
            .iter()
            .map(|f| (f.blob.clone(), f.index))
            .collect()
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
            if let Some(rec) = rec
                && rec.blob == blob
                && rec.index == index
            {
                *face = Some(rec.info(data.clone(), read_index));
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
        self.faces[id as usize]
            .as_ref()
            .expect("face bytes are loaded")
    }
    fn family(&mut self, family: &str, desc: &FontDesc) -> Option<FaceId> {
        self.answer(Query::family(family, desc))
    }
    fn standard(&mut self, desc: &FontDesc) -> Option<FaceId> {
        self.answer(Query::standard(desc))
    }
    fn fallback_for_char(
        &mut self,
        desc: &FontDesc,
        ch: u32,
        primary: FaceId,
        emoji: bool,
    ) -> Option<FaceId> {
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
    fn sbix_metric_rect(&mut self, face: FaceId, glyph: u32, size: f32) -> Option<[f64; 4]> {
        let q = Query::sbix_metric(face, glyph, size);
        match self.answers.get(&q) {
            Some(a) => a.rect,
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
        Recorder {
            inner,
            answers: BTreeMap::new(),
        }
    }

    fn record(&mut self, query: Query, face: Option<FaceId>) -> Option<FaceId> {
        self.answers.insert(
            query.clone(),
            Answer {
                query,
                face,
                advance: None,
                rect: None,
            },
        );
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
            if let Query::EmAdvance { face, .. } | Query::SbixMetric { face, .. } = a.query {
                ids.insert(face);
            }
        }
        Answers {
            faces: ids
                .into_iter()
                .map(|id| FaceRecord::of(id, self.inner.face(id)))
                .collect(),
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
    fn fallback_for_char(
        &mut self,
        desc: &FontDesc,
        ch: u32,
        primary: FaceId,
        emoji: bool,
    ) -> Option<FaceId> {
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
        self.answers.insert(
            query.clone(),
            Answer {
                query,
                face: None,
                advance: v,
                rect: None,
            },
        );
        v
    }
    fn sbix_metric_rect(&mut self, face: FaceId, glyph: u32, size: f32) -> Option<[f64; 4]> {
        let v = self.inner.sbix_metric_rect(face, glyph, size);
        let query = Query::sbix_metric(face, glyph, size);
        self.answers.insert(
            query.clone(),
            Answer {
                query,
                face: None,
                advance: None,
                rect: v,
            },
        );
        v
    }
}

/// Answers one query against a resolver (main answering a replay's misses).
pub fn answer_query(resolver: &mut dyn FontResolver, q: &Query) -> Answer {
    let face = match q {
        Query::Family { name, .. } => resolver.family(name, &q.desc().expect("family query")),
        Query::Standard { .. } => resolver.standard(&q.desc().expect("standard query")),
        Query::Fallback {
            ch, primary, emoji, ..
        } => resolver.fallback_for_char(&q.desc().expect("fallback query"), *ch, *primary, *emoji),
        Query::LastResort { .. } => resolver.last_resort(&q.desc().expect("last resort query")),
        Query::Variable { family } => resolver.variable_face(family),
        Query::EmAdvance { face, size } => {
            return Answer {
                query: q.clone(),
                face: None,
                advance: resolver.em_advance(*face, f32::from_bits(*size)),
                rect: None,
            };
        }
        Query::SbixMetric { face, glyph, size } => {
            return Answer {
                query: q.clone(),
                face: None,
                advance: None,
                rect: resolver.sbix_metric_rect(*face, *glyph, f32::from_bits(*size)),
            };
        }
    };
    Answer {
        query: q.clone(),
        face,
        advance: None,
        rect: None,
    }
}

/// Answers a batch of queries (main's reply to a replay's `misses`, or a
/// prefetch) as an incremental `Answers`: each query once, in query order,
/// with the records of every face the answers and queries name, so the
/// replay can `add_answers` it directly.
pub fn answer_batch(resolver: &mut dyn FontResolver, queries: &[Query]) -> Answers {
    let mut by_query: BTreeMap<Query, Answer> = BTreeMap::new();
    for q in queries {
        if !by_query.contains_key(q) {
            by_query.insert(q.clone(), answer_query(resolver, q));
        }
    }
    let mut ids: BTreeSet<FaceId> = BTreeSet::new();
    for a in by_query.values() {
        ids.extend(a.face);
        match a.query {
            Query::Fallback { primary, .. } => {
                ids.insert(primary);
            }
            Query::EmAdvance { face, .. } | Query::SbixMetric { face, .. } => {
                ids.insert(face);
            }
            _ => {}
        }
    }
    Answers {
        faces: ids
            .into_iter()
            .map(|id| FaceRecord::of(id, resolver.face(id)))
            .collect(),
        answers: by_query.into_values().collect(),
    }
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
        FontDesc {
            weight: 500.0,
            italic: false,
            size: 16.0,
        }
    }

    #[test]
    fn reply_json_ends_with_the_generation() {
        let answers = Answers {
            faces: vec![rec(0, "a")],
            answers: vec![Answer {
                query: Query::family("A", &desc()),
                face: Some(0),
                advance: None,
                rect: None,
            }],
        };
        for (value, generation) in [(&answers, 42), (&Answers::default(), 0)] {
            let json = value.reply_json(generation).unwrap();
            // Main's `answerGeneration` reads exactly this suffix.
            assert!(
                json.ends_with(&format!(",\"generation\":{generation}}}")),
                "{json}"
            );
            let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed["generation"], generation);
            let back: Answers = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, value);
        }
    }

    #[test]
    fn misses_and_pending_blobs_are_reported_then_resolved() {
        let answers = Answers {
            faces: vec![rec(0, "a")],
            answers: vec![Answer {
                query: Query::family("A", &desc()),
                face: Some(0),
                advance: None,
                rect: None,
            }],
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
            answers: vec![Answer {
                query: Query::family("B", &desc()),
                face: Some(1),
                advance: None,
                rect: None,
            }],
        });
        assert_eq!(r.family("B", &desc()), Some(1));
    }

    /// Resolves family "A" to face 0 and everything else to nothing.
    struct Fake(Vec<FaceInfo>);

    impl FontResolver for Fake {
        fn face(&self, id: FaceId) -> &FaceInfo {
            &self.0[id as usize]
        }
        fn family(&mut self, family: &str, _: &FontDesc) -> Option<FaceId> {
            (family == "A").then_some(0)
        }
        fn standard(&mut self, _: &FontDesc) -> Option<FaceId> {
            Some(1)
        }
        fn fallback_for_char(
            &mut self,
            _: &FontDesc,
            _: u32,
            _: FaceId,
            _: bool,
        ) -> Option<FaceId> {
            None
        }
        fn last_resort(&mut self, _: &FontDesc) -> Option<FaceId> {
            None
        }
        fn variable_face(&mut self, _: &str) -> Option<FaceId> {
            None
        }
        fn em_advance(&mut self, _: FaceId, size: f32) -> Option<f32> {
            Some(size)
        }
        fn sbix_metric_rect(&mut self, _: FaceId, glyph: u32, size: f32) -> Option<[f64; 4]> {
            (glyph == 7).then_some([0.0, f64::from(size) / -8.0, 40.0001, 40.0001])
        }
    }

    #[test]
    fn batches_answer_each_query_once_with_the_faces_they_name() {
        let face = |b: &str| rec(0, b).info(FontBytes::new(vec![]), 0);
        let mut r = Fake(vec![face("a"), face("t"), face("p")]);
        let queries = [
            Query::family("B", &desc()),
            Query::family("A", &desc()),
            Query::family("A", &desc()),
            Query::fallback(&desc(), 0x4e00, 2, false),
            Query::em_advance(1, 20.0),
            Query::sbix_metric(1, 7, 40.0),
            Query::sbix_metric(1, 8, 40.0),
        ];
        let a = answer_batch(&mut r, &queries);
        assert_eq!(a.answers.len(), 6);
        assert_eq!(
            a.faces
                .iter()
                .map(|f| (f.id, f.blob.as_str()))
                .collect::<Vec<_>>(),
            [(0, "a"), (1, "t"), (2, "p")]
        );
        let em = a
            .answers
            .iter()
            .find(|x| matches!(x.query, Query::EmAdvance { .. }))
            .unwrap();
        assert_eq!((em.face, em.advance), (None, Some(20.0)));
        // The replay takes the batch as is.
        let mut replay = ReplayResolver::new(&Answers::default());
        replay.add_answers(&a);
        replay.add_font("a", 0, FontBytes::new(vec![]), 0);
        assert_eq!(replay.family("A", &desc()), Some(0));
        assert_eq!(replay.family("B", &desc()), None);
        // `None` (CoreText has a path) is an answer, not a miss.
        assert_eq!(
            replay.sbix_metric_rect(1, 7, 40.0),
            Some([0.0, -5.0, 40.0001, 40.0001])
        );
        assert_eq!(replay.sbix_metric_rect(1, 8, 40.0), None);
        assert!(replay.misses.is_empty());
        assert_eq!(replay.sbix_metric_rect(1, 7, 41.0), None);
        assert_eq!(replay.misses, [Query::sbix_metric(1, 7, 41.0)]);
    }

    #[test]
    fn answers_serialize_with_stable_order() {
        let mut a = Answers::default();
        a.merge(&Answers {
            faces: vec![rec(1, "b"), rec(0, "a")],
            answers: vec![
                Answer {
                    query: Query::variable("Z"),
                    face: None,
                    advance: None,
                    rect: None,
                },
                Answer {
                    query: Query::family("A", &desc()),
                    face: Some(0),
                    advance: None,
                    rect: None,
                },
            ],
        });
        assert_eq!(a.faces.iter().map(|f| f.id).collect::<Vec<_>>(), [0, 1]);
        let json = serde_json::to_string(&a).unwrap();
        let back: Answers = serde_json::from_str(&json).unwrap();
        assert_eq!(back, a);
        assert!(json.find("\"family\"").unwrap() < json.find("\"variable\"").unwrap());
    }
}
