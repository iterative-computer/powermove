//! `v1_ingest`: float seconds from the document to engine ticks
//! (docs/rust-engine.md, section 5, "At the document boundary"; contract 5).
//!
//! The document and the public API keep float seconds; the engine works in
//! [`Tick`]s. This module is the one place that converts.
//!
//! # Rates
//!
//! A composition's rate comes from `fpsRational` (`[num, den]`) when it is a
//! valid rate, otherwise from snapping the stored float `fps`
//! ([`pm_time::rate_from_document`], boundary rule 3). [`IngestRate`] keeps the
//! exact [`Rate`] and the float used for the frame-index rule below.
//!
//! # Times
//!
//! For a time `t` in seconds in a composition with exact rate `R` and stored
//! float rate `fps_f64` ([`IngestRate::time`]):
//!
//! 1. `x = t × fps_f64` (one f64 multiplication) and `n = round(x)`, halves
//!    away from zero (`f64::round`). If `|x − n| < 1e-9 × max(1, |n|)` the
//!    time is frame-aligned: its ticks are `R.frame_start(n)`.
//! 2. Otherwise its ticks are the nearest tick to `t`, ties to even, computed
//!    exactly from the float's binary value ([`pm_time::ticks_from_seconds`]).
//!
//! `fps_f64` is the document's `fps` when that float is a valid rate that
//! snaps to `R`; otherwise (an `fps` that is missing, invalid, or disagrees
//! with `fpsRational`) it is `R` as the nearest f64. So a document saved with
//! the float 29.97 keeps every frame-aligned time on its frame of the exact
//! 30000/1001 grid at any length.
//!
//! The tolerance is section 26's amendment 1 (it was `1e-6` in section 5).
//! The float error of a frame-aligned time written by JavaScript (`n / fps`,
//! `n × (1 / fps)`, a step from the previous frame) is a few ulps, about
//! `1e-15 × n` relative, so `1e-9 × |n|` still captures every one of them up
//! to 10⁸ frames, while it stays a tenth of a frame or less up to 10⁸
//! frames: off-grid times far from zero keep their offset instead of being
//! pulled onto the nearest frame (with `1e-6` that happened beyond 500,000
//! frames).
//!
//! # Writing back
//!
//! Every [`IngestedTime`] keeps the original f64. [`IngestedTime::write_back`]
//! echoes it for a time the engine did not change, so ingest never perturbs
//! untouched values in the document. A time the engine created or moved is
//! written by [`IngestRate::seconds_of`] (amendment 1 to section 5, rule 4):
//! on a frame boundary as `n / fps_f64`, which re-ingests to the same frame at
//! every `n`, and off the grid as `ticks / TICKS` (correctly rounded).
//! `ticks / TICKS` for a frame start does not round-trip in a document that
//! stores a float NTSC rate: 29.97 is one part in a million slower than
//! 30000/1001, so frame 10⁶ written that way re-ingests as frame 999,999.
//!
//! # Which document members are times
//!
//! [`ingest_project`] walks the canonical form ([`crate::canonical`]) and
//! converts every time member of every composition at that composition's rate.
//! [`classify_time_path`] recognizes the same members from a canonical patch
//! path, so a leaf patch can be ingested without a walk. Composition time and
//! layer time share the composition's rate:
//!
//! | Member | [`TimeKind`] |
//! |---|---|
//! | composition `dur` | `CompositionDuration` |
//! | composition `work[0]`, `work[1]` | `WorkArea` |
//! | `markers[i].t` | `Marker` |
//! | layer `from` | `LayerStart` |
//! | layer `dur` | `LayerDuration` |
//! | `t` of every key of every channel anywhere below a layer (`…kf[k].t`) | `Keyframe` |
//! | `transitionIn.dur`, `transitionOut.dur` | `TransitionDuration` |
//! | captions `d.cues[i].start`/`end` and `…words[j].start`/`end` | `Caption` |
//! | numeric `d.trim` (video and audio source offset) | `SourceOffset` |
//! | numeric `d.fadeIn`, `d.fadeOut` (audio) | `Fade` |
//!
//! A channel is any object below a layer with a `kf` array; that covers
//! transforms, effects, masks, paths, text animators and styles, shader
//! uniforms, extension parameters, and the 3D channels in `d.data` and
//! `d.modeling`. Channel *values* that happen to be times (a precomp's
//! `trim` or `sourceTime` channel) are values, converted by evaluation.
//! `trim` counts in source seconds; edits create it in composition frames
//! (split at the playhead, at speed 1), so it uses the composition's rate, and
//! any other value takes rule 2 unchanged. Asset `fps` and `dur` are media
//! hints and the edit log is history; neither is ingested.

use pm_time::{
    Rate, RateSource, Tick, TimeError, rate_from_document, seconds_from_ticks, snap_float_rate,
    ticks_from_seconds,
};

use crate::json::{JsString, Value};
use crate::patch::PathPart;

/// The relative tolerance of the frame-index rule (section 26, amendment 1).
pub const FRAME_TOLERANCE: f64 = 1e-9;

/// A composition rate as ingested.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IngestRate {
    /// The exact rate.
    pub rate: Rate,
    /// Which rule produced it.
    pub source: RateSource,
    /// The float the frame-index rule multiplies by.
    pub fps: f64,
}

/// One time from the document, in ticks, with the original float.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IngestedTime {
    pub ticks: Tick,
    /// The document's value, echoed when the time is unchanged.
    pub original: f64,
    /// The frame index when rule 1 applied.
    pub frame: Option<i64>,
}

impl IngestedTime {
    /// The float to store for `ticks`: the original when unchanged, otherwise
    /// [`IngestRate::seconds_of`] at the composition's rate `rate`.
    pub fn write_back(&self, rate: &IngestRate, ticks: Tick) -> f64 {
        if ticks == self.ticks {
            self.original
        } else {
            rate.seconds_of(ticks)
        }
    }
}

/// `[num, den]` from a document `fpsRational` when both are safe integers.
pub fn fps_rational_of(value: Option<&Value>) -> Option<[i64; 2]> {
    let items = value?.as_array()?;
    if items.len() != 2 {
        return None;
    }
    let int = |v: &Value| {
        let x = v.as_f64()?;
        (x.fract() == 0.0 && x.abs() <= 9_007_199_254_740_991.0).then_some(x as i64)
    };
    Some([int(&items[0])?, int(&items[1])?])
}

impl IngestRate {
    /// The rate of a composition from its `fpsRational` and `fps` members.
    pub fn from_document(
        fps_rational: Option<[i64; 2]>,
        fps: Option<f64>,
    ) -> Result<IngestRate, TimeError> {
        let float = fps.unwrap_or(f64::NAN);
        let resolved = rate_from_document(fps_rational, float)?;
        let consistent = snap_float_rate(float).is_ok_and(|s| s.rate == resolved.rate);
        Ok(IngestRate {
            rate: resolved.rate,
            source: resolved.source,
            fps: if consistent {
                float
            } else {
                resolved.rate.to_f64()
            },
        })
    }

    /// The rate of a composition object.
    pub fn of_composition(comp: &Value) -> Result<IngestRate, TimeError> {
        IngestRate::from_document(
            fps_rational_of(comp.get("fpsRational")),
            comp.get("fps").and_then(Value::as_f64),
        )
    }

    /// Ingests one time (rules 1 and 2 in the module documentation).
    pub fn time(&self, t: f64) -> Result<IngestedTime, TimeError> {
        if !t.is_finite() {
            return Err(TimeError::NonFiniteSeconds);
        }
        let x = t * self.fps;
        if !x.is_finite() {
            return Err(TimeError::Overflow);
        }
        let n = x.round();
        if (x - n).abs() < FRAME_TOLERANCE * n.abs().max(1.0) {
            // |n| < 2^63 whenever the frame start fits in a tick; larger
            // values fail in frame_start or here.
            if n.abs() >= 9.223_372_036_854_775e18 {
                return Err(TimeError::Overflow);
            }
            let frame = n as i64;
            return Ok(IngestedTime {
                ticks: self.rate.frame_start(frame)?,
                original: t,
                frame: Some(frame),
            });
        }
        Ok(IngestedTime {
            ticks: ticks_from_seconds(t)?,
            original: t,
            frame: None,
        })
    }

    /// The float stored for a time the engine created or moved (section 5,
    /// rule 4, as amended): for a frame start, `n / fps_f64` (the document's
    /// own float grid); off the grid, `ticks / TICKS`.
    ///
    /// [`IngestRate::time`] maps the result back to `ticks` for every frame
    /// start whose float `n / fps_f64` round-trips (every `n` the `ingest`
    /// tests try, to ±10⁸ at every supported rate and the float NTSC rates),
    /// and for every off-grid tick whose `ticks / TICKS` is farther than the
    /// tolerance from the document's float grid. An off-grid tick within the
    /// tolerance of that grid has no float that maps back to it (rule 1 claims
    /// every such float); it re-ingests as that frame's start. With a float
    /// NTSC rate the float grid drifts from the exact one by `1e-6 × n`
    /// frames, so this can happen for ticks a fraction of a frame from a
    /// frame start once `|n|` passes about 200,000.
    pub fn seconds_of(&self, ticks: Tick) -> f64 {
        let n = self.rate.frame_of(ticks);
        if self.rate.frame_start(n) == Ok(ticks) {
            let candidate = n as f64 / self.fps;
            if self.time(candidate).is_ok_and(|t| t.ticks == ticks) {
                return candidate;
            }
        }
        seconds_from_ticks(ticks)
    }
}

/// What a time member is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimeKind {
    CompositionDuration,
    WorkArea,
    Marker,
    LayerStart,
    LayerDuration,
    Keyframe,
    TransitionDuration,
    Caption,
    SourceOffset,
    Fade,
}

/// One ingested time member.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeField {
    /// Path from the canonical project root.
    pub path: Vec<PathPart>,
    pub kind: TimeKind,
    pub time: IngestedTime,
}

/// One composition and its times.
#[derive(Clone, Debug, PartialEq)]
pub struct IngestedComposition {
    /// Path from the canonical project root (`[]` for a root that is itself
    /// the composition, else `["comps", id, ...]`).
    pub path: Vec<PathPart>,
    pub id: Option<JsString>,
    pub rate: IngestRate,
    pub times: Vec<TimeField>,
}

/// Why a project could not be ingested.
#[derive(Clone, Debug, PartialEq)]
pub struct IngestError {
    /// The composition or member that failed.
    pub path: Vec<PathPart>,
    pub error: TimeError,
}

impl core::fmt::Display for IngestError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} at {}", self.error, display_path(&self.path))
    }
}

impl std::error::Error for IngestError {}

fn display_path(path: &[PathPart]) -> String {
    let mut out = String::new();
    for part in path {
        match part {
            PathPart::Key(k) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(&k.to_string_lossy());
            }
            PathPart::Index(i) => out.push_str(&format!("[{i}]")),
        }
    }
    out
}

/// A whole project as ingested.
#[derive(Clone, Debug, PartialEq)]
pub struct IngestedProject {
    pub compositions: Vec<IngestedComposition>,
}

impl IngestedProject {
    /// Every time field, across compositions.
    pub fn times(&self) -> impl Iterator<Item = (&IngestedComposition, &TimeField)> {
        self.compositions
            .iter()
            .flat_map(|c| c.times.iter().map(move |t| (c, t)))
    }
}

/// Whether the canonical root is itself a composition: [`canonical_project`]
/// leaves a project unchanged when it has no open composition pointer
/// (`compId`, as in projects that predate compositions) or its `comps` is not
/// an object.
///
/// [`canonical_project`]: crate::canonical::canonical_project
pub fn root_is_composition(canonical: &Value) -> bool {
    canonical.as_object().is_some()
        && (crate::canonical::open_comp_id(canonical).is_none()
            || canonical
                .get("comps")
                .is_some_and(|comps| comps.as_object().is_none()))
}

/// Ingests every composition of a project in canonical form (see
/// [`crate::canonical::canonical_project`]). Compositions are listed in
/// depth-first order of their paths, ids in code-point order; nested `comps`
/// maps from older files are visited too.
pub fn ingest_project(canonical: &Value) -> Result<IngestedProject, IngestError> {
    let mut compositions = Vec::new();
    let mut path = Vec::new();
    if root_is_composition(canonical) {
        ingest_composition(canonical, &mut path, &mut compositions)?;
    } else if let Some(comps) = canonical.get("comps").and_then(Value::as_object) {
        for (id, comp) in comps {
            if comp.as_object().is_none() {
                continue;
            }
            path.push(PathPart::from("comps"));
            path.push(PathPart::Key(id.clone()));
            ingest_composition(comp, &mut path, &mut compositions)?;
            path.truncate(path.len() - 2);
        }
    }
    Ok(IngestedProject { compositions })
}

/// Ingests a stored project (open composition at the top level) by taking
/// its canonical form first; paths are canonical.
pub fn ingest_stored_project(project: &Value) -> Result<IngestedProject, IngestError> {
    ingest_project(&crate::canonical::canonical_project(project.clone()))
}

fn ingest_composition(
    comp: &Value,
    path: &mut Vec<PathPart>,
    out: &mut Vec<IngestedComposition>,
) -> Result<(), IngestError> {
    let rate = IngestRate::of_composition(comp).map_err(|error| IngestError {
        path: path.clone(),
        error,
    })?;
    let mut walker = Walker {
        rate,
        path: path.clone(),
        times: Vec::new(),
    };
    walker.composition(comp)?;
    out.push(IngestedComposition {
        path: path.clone(),
        id: comp.get("id").and_then(Value::as_js_string).cloned(),
        rate,
        times: walker.times,
    });
    if let Some(nested) = comp.get("comps").and_then(Value::as_object) {
        for (id, child) in nested {
            if child.as_object().is_none() {
                continue;
            }
            path.push(PathPart::from("comps"));
            path.push(PathPart::Key(id.clone()));
            ingest_composition(child, path, out)?;
            path.truncate(path.len() - 2);
        }
    }
    Ok(())
}

struct Walker {
    rate: IngestRate,
    path: Vec<PathPart>,
    times: Vec<TimeField>,
}

impl Walker {
    fn push(&mut self, part: impl Into<PathPart>) {
        self.path.push(part.into());
    }

    fn pop(&mut self) {
        self.path.pop();
    }

    /// Ingests `value` at the current path when it is a number.
    fn time(&mut self, value: Option<&Value>, kind: TimeKind) -> Result<(), IngestError> {
        let Some(Value::Number(t)) = value else {
            return Ok(());
        };
        let time = self.rate.time(*t).map_err(|error| IngestError {
            path: self.path.clone(),
            error,
        })?;
        self.times.push(TimeField {
            path: self.path.clone(),
            kind,
            time,
        });
        Ok(())
    }

    fn member(&mut self, obj: &Value, key: &str, kind: TimeKind) -> Result<(), IngestError> {
        self.push(key);
        let result = self.time(obj.get(key), kind);
        self.pop();
        result
    }

    fn composition(&mut self, comp: &Value) -> Result<(), IngestError> {
        self.member(comp, "dur", TimeKind::CompositionDuration)?;
        if let Some(work) = comp.get("work").and_then(Value::as_array) {
            self.push("work");
            for (i, item) in work.iter().enumerate().take(2) {
                self.push(i);
                self.time(Some(item), TimeKind::WorkArea)?;
                self.pop();
            }
            self.pop();
        }
        if let Some(markers) = comp.get("markers").and_then(Value::as_array) {
            self.push("markers");
            for (i, marker) in markers.iter().enumerate() {
                self.push(i);
                self.member(marker, "t", TimeKind::Marker)?;
                self.pop();
            }
            self.pop();
        }
        if let Some(layers) = comp.get("layers").and_then(Value::as_array) {
            self.push("layers");
            for (i, layer) in layers.iter().enumerate() {
                if layer.as_object().is_some() {
                    self.push(i);
                    self.layer(layer)?;
                    self.pop();
                }
            }
            self.pop();
        }
        Ok(())
    }

    fn layer(&mut self, layer: &Value) -> Result<(), IngestError> {
        self.member(layer, "from", TimeKind::LayerStart)?;
        self.member(layer, "dur", TimeKind::LayerDuration)?;
        for key in ["transitionIn", "transitionOut"] {
            if let Some(transition) = layer.get(key).filter(|t| t.as_object().is_some()) {
                self.push(key);
                self.member(transition, "dur", TimeKind::TransitionDuration)?;
                self.pop();
            }
        }
        if let Some(d) = layer.get("d").filter(|d| d.as_object().is_some()) {
            self.push("d");
            self.member(d, "trim", TimeKind::SourceOffset)?;
            self.member(d, "fadeIn", TimeKind::Fade)?;
            self.member(d, "fadeOut", TimeKind::Fade)?;
            if let Some(cues) = d.get("cues").and_then(Value::as_array) {
                self.push("cues");
                for (i, cue) in cues.iter().enumerate() {
                    self.push(i);
                    self.member(cue, "start", TimeKind::Caption)?;
                    self.member(cue, "end", TimeKind::Caption)?;
                    if let Some(words) = cue.get("words").and_then(Value::as_array) {
                        self.push("words");
                        for (j, word) in words.iter().enumerate() {
                            self.push(j);
                            self.member(word, "start", TimeKind::Caption)?;
                            self.member(word, "end", TimeKind::Caption)?;
                            self.pop();
                        }
                        self.pop();
                    }
                    self.pop();
                }
                self.pop();
            }
            self.pop();
        }
        self.keys_below(layer)
    }

    /// Every `kf[k].t` below `value`.
    fn keys_below(&mut self, value: &Value) -> Result<(), IngestError> {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    self.push(PathPart::Key(key.clone()));
                    if key.as_str() == Some("kf")
                        && let Value::Array(keys) = child
                    {
                        for (k, keyframe) in keys.iter().enumerate() {
                            self.push(k);
                            if keyframe.as_object().is_some() {
                                self.member(keyframe, "t", TimeKind::Keyframe)?;
                            }
                            self.keys_below(keyframe)?;
                            self.pop();
                        }
                    } else {
                        self.keys_below(child)?;
                    }
                    self.pop();
                }
            }
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    self.push(i);
                    self.keys_below(item)?;
                    self.pop();
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// The time member a canonical patch path addresses, with the path of its
/// composition, or `None` when the path is not a time member.
///
/// `root_is_composition` is [`root_is_composition`] of the project. A path
/// that replaces a whole subtree (a layer, a key) is not a time member; the
/// mirror re-ingests the subtree instead.
pub fn classify_time_path(
    path: &[PathPart],
    root_is_composition: bool,
) -> Option<(TimeKind, Vec<PathPart>)> {
    let key = |i: usize| match path.get(i) {
        Some(PathPart::Key(k)) => k.as_str(),
        Some(PathPart::Index(_)) => None,
        None => None,
    };
    let is_index = |i: usize| matches!(path.get(i), Some(p) if as_index(p).is_some());
    // Strip ("comps", id) pairs: the deepest composition owns the member.
    let mut start = 0;
    while key(start) == Some("comps") && path.get(start + 1).is_some() {
        start += 2;
    }
    if start == 0 && !root_is_composition {
        return None;
    }
    let comp = path[..start].to_vec();
    let rest = &path[start..];
    let rkey = |i: usize| key(start + i);
    let ridx = |i: usize| is_index(start + i);
    let kind = match rest.len() {
        1 if rkey(0) == Some("dur") => TimeKind::CompositionDuration,
        2 if rkey(0) == Some("work") && matches!(as_index(&rest[1]), Some(0 | 1)) => {
            TimeKind::WorkArea
        }
        3 if rkey(0) == Some("markers") && ridx(1) && rkey(2) == Some("t") => TimeKind::Marker,
        _ if rest.len() >= 3 && rkey(0) == Some("layers") && ridx(1) => {
            classify_layer_member(&rest[2..])?
        }
        _ => return None,
    };
    Some((kind, comp))
}

fn as_index(part: &PathPart) -> Option<u32> {
    match part {
        PathPart::Index(n) if *n >= 0.0 && n.fract() == 0.0 && *n < 4_294_967_295.0 => {
            Some(*n as u32)
        }
        PathPart::Index(_) => None,
        PathPart::Key(k) => k.as_str().and_then(crate::json::array_index),
    }
}

fn classify_layer_member(rest: &[PathPart]) -> Option<TimeKind> {
    let key = |i: usize| match rest.get(i) {
        Some(PathPart::Key(k)) => k.as_str(),
        _ => None,
    };
    let idx = |i: usize| rest.get(i).is_some_and(|p| as_index(p).is_some());
    let n = rest.len();
    // `…kf[k].t` anywhere below the layer.
    if n >= 3 && key(n - 1) == Some("t") && idx(n - 2) && key(n - 3) == Some("kf") {
        return Some(TimeKind::Keyframe);
    }
    match (n, key(0)) {
        (1, Some("from")) => Some(TimeKind::LayerStart),
        (1, Some("dur")) => Some(TimeKind::LayerDuration),
        (2, Some("transitionIn" | "transitionOut")) if key(1) == Some("dur") => {
            Some(TimeKind::TransitionDuration)
        }
        (2, Some("d")) => match key(1) {
            Some("trim") => Some(TimeKind::SourceOffset),
            Some("fadeIn" | "fadeOut") => Some(TimeKind::Fade),
            _ => None,
        },
        (4, Some("d"))
            if key(1) == Some("cues") && idx(2) && matches!(key(3), Some("start" | "end")) =>
        {
            Some(TimeKind::Caption)
        }
        (6, Some("d"))
            if key(1) == Some("cues")
                && idx(2)
                && key(3) == Some("words")
                && idx(4)
                && matches!(key(5), Some("start" | "end")) =>
        {
            Some(TimeKind::Caption)
        }
        _ => None,
    }
}
