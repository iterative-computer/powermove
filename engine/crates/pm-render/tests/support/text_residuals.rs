//! FONTS-R: exact residual snapshots, independent of the strict pixel bounds.
//! A regression, a new residual, or an improvement with a stale budget fails.
//!
//! The approved ceilings live here, in code ([`APPROVED`]), not only in the
//! fixture: `fixtures/text-residuals.json` may lower or remove an entry, but
//! an entry that is not in [`APPROVED`], names another measurement, or
//! exceeds its approved pixels or max CV fails to load. Loosening the
//! ratchet therefore needs a visible change to this table.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Key {
    Raster(String),
    Corpus {
        project: String,
        isolated: bool,
        time: u64,
    },
}

impl Key {
    pub fn raster(id: &str) -> Self {
        Self::Raster(id.to_owned())
    }

    pub fn corpus(project: &str, isolated: bool, time: f64) -> Self {
        Self::Corpus {
            project: project.to_owned(),
            isolated,
            time: time.to_bits(),
        }
    }

    pub fn gate(&self) -> Gate {
        match self {
            Self::Raster(_) => Gate::Raster,
            Self::Corpus { .. } => Gate::Corpus,
        }
    }

    fn strict_bound(&self) -> u8 {
        match self {
            Self::Raster(_) => 1,
            Self::Corpus { .. } => 2,
        }
    }
}

/// One approved FONTS-R ceiling: the measurement it names and the exact
/// budget the orchestrator ruled visually identical (decision FONTS-R,
/// 2026-10-19; docs/engine/text-render.md, "FONTS-R").
#[derive(Clone, Copy, Debug)]
pub struct Approved {
    pub id: &'static str,
    /// `None`: a text raster case (`text_raster`); `Some`: a composed frame
    /// (`text_corpus`) as (project, compared in isolation, time in seconds).
    pub corpus: Option<(&'static str, bool, f64)>,
    pub pixels_over_bound: usize,
    pub max_cv: u8,
}

pub const fn raster(id: &'static str, pixels_over_bound: usize, max_cv: u8) -> Approved {
    Approved {
        id,
        corpus: None,
        pixels_over_bound,
        max_cv,
    }
}

pub const fn corpus(
    id: &'static str,
    project: &'static str,
    isolated: bool,
    time: f64,
    pixels_over_bound: usize,
    max_cv: u8,
) -> Approved {
    Approved {
        id,
        corpus: Some((project, isolated, time)),
        pixels_over_bound,
        max_cv,
    }
}

/// The 23 approved FONTS-R ceilings. Never raise a value or add a row here
/// without a new orchestrator decision; lower or remove rows (and the
/// fixture's entries) as residuals improve.
pub const APPROVED: [Approved; 23] = [
    raster("caption-paper-0460", 1, 2),
    raster("caption-paper-0461", 1, 2),
    raster("caption-paper-0462", 1, 2),
    raster("caption-pop-0455", 2, 2),
    raster("caption-pop-0456", 5, 2),
    raster("caption-pop-0458", 7, 13),
    raster("caption-pop-0459", 74, 13),
    raster("caption-stroke-0491", 5, 4),
    raster("caption-whisper-0442", 1, 2),
    raster("caption-whisper-0443", 1, 2),
    raster("caption-whisper-0444", 1, 2),
    raster("colr-0033", 1, 2),
    raster("colr-0034", 5, 2),
    raster("sdf-sizes-0004", 4, 2),
    raster("path-stroke-0000", 154, 4),
    raster("path-stroke-0001", 1, 64),
    raster("real-16-Lkkauewe", 6, 64),
    raster("real-18-Lkkauewe", 6, 64),
    raster("real-24-L3app4ie", 2, 5),
    corpus("real_powermove-ad-16", "real/powermove-ad", false, 16.0, 2, 39),
    corpus("real_powermove-ad-18", "real/powermove-ad", false, 18.0, 6, 34),
    corpus("real_powermove-ad-text-only-16", "real/powermove-ad", true, 16.0, 3, 44),
    corpus("real_powermove-ad-text-only-18", "real/powermove-ad", true, 18.0, 6, 44),
];

impl Approved {
    fn key(&self) -> Key {
        match self.corpus {
            None => Key::raster(self.id),
            Some((project, isolated, time)) => Key::corpus(project, isolated, time),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Gate {
    Raster,
    Corpus,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusCase {
    project: String,
    isolated: bool,
    time: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Residual {
    gate: Gate,
    id: String,
    pixels_over_bound: usize,
    max_cv: u8,
    cause: String,
    side_by_side: String,
    corpus: Option<CorpusCase>,
}

impl Residual {
    fn key(&self) -> Result<Key, String> {
        match (self.gate, &self.corpus) {
            (Gate::Raster, None) => Ok(Key::raster(&self.id)),
            (Gate::Corpus, Some(c))
                if !c.project.trim().is_empty() && c.time.is_finite() && c.time >= 0.0 =>
            {
                Ok(Key::corpus(&c.project, c.isolated, c.time))
            }
            _ => Err(format!("{}: invalid gate/corpus identity", self.id)),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    kind: String,
    version: u32,
    decision: String,
    policy: String,
    cases: Vec<Residual>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    Strict,
    Ratcheted,
}

pub struct Ratchet {
    budgets: BTreeMap<Key, Residual>,
    seen: BTreeSet<Key>,
}

impl Ratchet {
    pub fn load() -> Self {
        Self::from_json(include_str!("../fixtures/text-residuals.json"))
            .expect("valid FONTS-R text residual ratchet")
    }

    /// The fixture checked against the approved ceilings in [`APPROVED`].
    pub fn from_json(json: &str) -> Result<Self, String> {
        Self::from_json_with(json, &APPROVED)
    }

    /// The fixture checked against `approved` (the contract tests use
    /// synthetic tables; the gates use [`Ratchet::load`]).
    pub fn from_json_with(json: &str, approved: &[Approved]) -> Result<Self, String> {
        let fixture: Fixture = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if fixture.kind != "RATCHET"
            || fixture.version != 1
            || fixture.decision != "FONTS-R"
            || fixture.policy.trim().is_empty()
        {
            return Err("expected version 1 FONTS-R RATCHET metadata".into());
        }
        let mut budgets = BTreeMap::new();
        let mut ids = BTreeSet::new();
        for residual in fixture.cases {
            let key = residual.key()?;
            if residual.id.is_empty()
                || !residual
                    .id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                || residual.side_by_side != format!("{}.png", residual.id)
                || residual.cause.trim().is_empty()
                || residual.cause.lines().count() != 1
            {
                return Err(format!(
                    "{}: expected a case id, one-line cause and image filename only",
                    residual.id
                ));
            }
            if residual.pixels_over_bound == 0 || residual.max_cv <= key.strict_bound() {
                return Err(format!(
                    "{}: remove entries that meet the strict bound",
                    residual.id
                ));
            }
            let Some(ceiling) = approved.iter().find(|a| a.id == residual.id) else {
                return Err(format!(
                    "{}: not an approved FONTS-R residual (a new case needs a new decision and a row in APPROVED)",
                    residual.id
                ));
            };
            if ceiling.key() != key {
                return Err(format!(
                    "{}: names {key:?}, but the approved residual is {:?}",
                    residual.id,
                    ceiling.key()
                ));
            }
            if residual.pixels_over_bound > ceiling.pixels_over_bound
                || residual.max_cv > ceiling.max_cv
            {
                return Err(format!(
                    "{}: budget {} pixels / max {} exceeds the approved ceiling {} pixels / max {} (budgets may only be lowered)",
                    residual.id,
                    residual.pixels_over_bound,
                    residual.max_cv,
                    ceiling.pixels_over_bound,
                    ceiling.max_cv
                ));
            }
            if !ids.insert(residual.id.clone()) || budgets.insert(key, residual).is_some() {
                return Err("duplicate residual id or measurement identity".into());
            }
        }
        Ok(Self {
            budgets,
            seen: BTreeSet::new(),
        })
    }

    pub fn check(&mut self, key: Key, pixels: usize, max_cv: u8) -> Result<Outcome, String> {
        self.seen.insert(key.clone());
        let Some(budget) = self.budgets.get(&key) else {
            return if pixels == 0 && max_cv <= key.strict_bound() {
                Ok(Outcome::Strict)
            } else {
                Err(format!(
                    "unlisted {key:?}: {pixels} pixels over {} CV, max {max_cv}",
                    key.strict_bound()
                ))
            };
        };
        if pixels > budget.pixels_over_bound || max_cv > budget.max_cv {
            return Err(format!(
                "FONTS-R {} regressed: {pixels} pixels, max {max_cv}; budget {} pixels, max {}",
                budget.id, budget.pixels_over_bound, budget.max_cv
            ));
        }
        if pixels < budget.pixels_over_bound || max_cv < budget.max_cv {
            return Err(format!(
                "FONTS-R {} improved: {pixels} pixels, max {max_cv}; lower the {} pixel / max {} budget (remove if strict)",
                budget.id, budget.pixels_over_bound, budget.max_cv
            ));
        }
        Ok(Outcome::Ratcheted)
    }

    /// Every listed case of `gate` that no selected capture measured. A full
    /// gate run (nothing narrowed) fails on any: an entry cannot go unchecked
    /// because its capture is missing or was not selected.
    pub fn unmeasured(&self, gate: Gate) -> Vec<String> {
        self.budgets
            .iter()
            .filter(|(key, _)| key.gate() == gate && !self.seen.contains(*key))
            .map(|(_, r)| format!("FONTS-R {} has no selected capture (not measured)", r.id))
            .collect()
    }

    /// A listed case present in a selected capture cannot disappear through
    /// an empty shot, refusal, missing media or another early return.
    pub fn missing(&self, expected: impl IntoIterator<Item = Key>) -> Vec<String> {
        expected
            .into_iter()
            .filter_map(|key| {
                self.budgets
                    .get(&key)
                    .filter(|_| !self.seen.contains(&key))
                    .map(|r| format!("FONTS-R {} was not measured", r.id))
            })
            .collect()
    }
}
