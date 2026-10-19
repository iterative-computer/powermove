//! FONTS-R: exact residual snapshots, independent of the strict pixel bounds.
//! A regression, a new residual, or an improvement with a stale budget fails.
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

    fn strict_bound(&self) -> u8 {
        match self {
            Self::Raster(_) => 1,
            Self::Corpus { .. } => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Gate {
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

    pub fn from_json(json: &str) -> Result<Self, String> {
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
