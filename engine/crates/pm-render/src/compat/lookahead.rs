//! Asking an out-of-process producer a frame ahead without changing what it
//! is asked or in which order (an export's raster overrides).
//!
//! The serial path asks the producer each layer as the plan reaches it,
//! through a cache that keeps one time's answers: a layer asked again at the
//! same time is not asked twice, and an ask at another time clears it (a
//! held precomp, a local time that comes back, are asked again). A frame
//! ahead is probed (`Renderer::compat_missing`) with that cache simulated
//! from the state the frame before left it in, so the asks it records are
//! exactly the serial path's, in its order; the real frame then replays them
//! through the same cache. A frame whose probe met a layer whose answer can
//! change what the plan asks next (a track matte's consumer) records nothing
//! and runs serially. A real frame that asks anything else than what was
//! asked for it ahead has diverged: the export stops, never drawing with a
//! producer that ran out of order.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::compat::CompatAnswer;

/// One layer's ask: composition, layer, composition-local time (seconds)
/// and output scale, as the plan gives them.
#[derive(Clone, Debug, PartialEq)]
pub struct AheadNeed {
    pub composition: String,
    pub layer: String,
    pub seconds: f64,
    pub scale: f64,
}

type Key = (String, String, u64, u64);

impl AheadNeed {
    fn key(&self) -> Key {
        (
            self.composition.clone(),
            self.layer.clone(),
            self.seconds.to_bits(),
            self.scale.to_bits(),
        )
    }
}

/// What the serial path does with an ask now.
#[derive(Debug)]
pub enum Step {
    /// Answered before at this time.
    Answer(CompatAnswer),
    /// Asked ahead: its answer is the next planned one
    /// ([`Lookahead::take_planned`]).
    Planned,
    /// Ask the producer now, then [`Lookahead::store`] the answer.
    Ask,
    /// Not what was asked ahead for this frame: stop.
    Diverged(String),
}

/// The serial cache, and what was asked ahead of it.
#[derive(Default)]
pub struct Lookahead {
    map: HashMap<Key, CompatAnswer>,
    time: Option<u64>,
    /// The probe of the frame ahead, while it runs.
    shadow: Option<Shadow>,
    /// Asked ahead, in serial order, not reached yet.
    planned: VecDeque<Key>,
    /// Their answers, as they arrived (`arrive`), in the same order.
    ready: VecDeque<CompatAnswer>,
    diverged: Option<String>,
}

struct Shadow {
    keys: HashSet<Key>,
    time: Option<u64>,
    asks: Vec<AheadNeed>,
    unsafe_order: bool,
}

const DIVERGED: &str = "The export stopped: its extensions' layers were asked ahead in another order than the frame asks them";

impl Lookahead {
    /// The serial path's step for `need`.
    pub fn answer(&mut self, need: &AheadNeed) -> Step {
        if let Some(why) = &self.diverged {
            return Step::Diverged(why.clone());
        }
        let key = need.key();
        if let Some(answer) = self.map.get(&key) {
            return Step::Answer(answer.clone());
        }
        if self.time != Some(key.2) {
            self.map.clear();
            self.time = Some(key.2);
        }
        match self.planned.front() {
            None => Step::Ask,
            Some(next) if *next == key => Step::Planned,
            Some(_) => self.diverge(),
        }
    }
    fn diverge(&mut self) -> Step {
        self.diverged = Some(DIVERGED.to_owned());
        self.planned.clear();
        self.ready.clear();
        Step::Diverged(DIVERGED.to_owned())
    }
    /// Whether a planned answer has arrived for the next planned ask.
    pub fn has_ready(&self) -> bool {
        !self.ready.is_empty()
    }
    /// Answers of the next request asked ahead, in its order.
    pub fn arrive(&mut self, answers: impl IntoIterator<Item = CompatAnswer>) {
        self.ready.extend(answers);
    }
    /// The answer for `need` after [`Step::Planned`] (with `has_ready`).
    pub fn take_planned(&mut self, need: &AheadNeed) -> CompatAnswer {
        self.planned.pop_front();
        let answer = self.ready.pop_front().unwrap_or(CompatAnswer::Missing);
        self.store(need, answer.clone());
        answer
    }
    /// Keeps an answer the producer gave for `need` (after [`Step::Ask`]).
    pub fn store(&mut self, need: &AheadNeed, answer: CompatAnswer) {
        self.map.insert(need.key(), answer);
    }
    /// The serial cache simulated for a probe of the frame ahead: an ask the
    /// serial path would make there is recorded.
    pub fn probe(&mut self, need: &AheadNeed) {
        let (map, time) = (&self.map, self.time);
        let shadow = self.shadow.get_or_insert_with(|| Shadow {
            keys: map.keys().cloned().collect(),
            time,
            asks: Vec::new(),
            unsafe_order: false,
        });
        let key = need.key();
        if shadow.keys.contains(&key) {
            return;
        }
        if shadow.time != Some(key.2) {
            shadow.keys.clear();
            shadow.time = Some(key.2);
        }
        shadow.keys.insert(key);
        shadow.asks.push(need.clone());
    }
    /// The probe met a layer whose answer can change what the frame asks
    /// next: that frame is asked serially.
    pub fn probe_unsafe(&mut self) {
        let (map, time) = (&self.map, self.time);
        self.shadow
            .get_or_insert_with(|| Shadow {
                keys: map.keys().cloned().collect(),
                time,
                asks: Vec::new(),
                unsafe_order: false,
            })
            .unsafe_order = true;
    }
    /// Ends the probe: the asks to make now, in order (none for a frame
    /// asked serially), planned for the frame. A frame before that did not
    /// reach everything asked ahead for it has diverged.
    pub fn ahead(&mut self) -> Result<Vec<AheadNeed>, String> {
        let shadow = self.shadow.take();
        if let Some(why) = &self.diverged {
            return Err(why.clone());
        }
        if !self.planned.is_empty() {
            self.diverge();
            return Err(DIVERGED.to_owned());
        }
        // Answers that arrived for nothing planned are dropped.
        self.ready.clear();
        let Some(shadow) = shadow.filter(|s| !s.unsafe_order) else {
            return Ok(Vec::new());
        };
        self.planned = shadow.asks.iter().map(AheadNeed::key).collect();
        Ok(shadow.asks)
    }
    /// Whether anything is asked ahead and not reached yet.
    pub fn planning(&self) -> bool {
        !self.planned.is_empty()
    }
}
