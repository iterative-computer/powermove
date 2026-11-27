use crate::{clean_cue_text, cue_words, join_words, normalize_caption_style};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

pub const MIN_CUE_DURATION: f64 = 0.04;
const EPS: f64 = 1e-6;
pub(crate) fn round(value: f64) -> f64 {
    if value.abs() > f64::MAX / 1e6 {
        value
    } else {
        (value * 1e6 + 0.5).floor() / 1e6
    }
}
pub(crate) fn clamp(v: f64, a: f64, b: f64) -> f64 {
    a.max(b.min(v))
}
/// JSON equivalent of the shipped Number coercion, with missing/blank fallback.
pub(crate) fn finite(value: Option<&Value>, fallback: f64) -> f64 {
    fn number_string(s: &str) -> Option<f64> {
        let s = crate::text::trim(s);
        if s.is_empty() {
            return Some(0.0);
        }
        for (prefix, radix) in [
            ("0x", 16),
            ("0X", 16),
            ("0b", 2),
            ("0B", 2),
            ("0o", 8),
            ("0O", 8),
        ] {
            if let Some(digits) = s.strip_prefix(prefix) {
                return u64::from_str_radix(digits, radix).ok().map(|n| n as f64);
            }
        }
        s.parse().ok()
    }
    let n = match value {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) if !crate::text::trim(s).is_empty() => number_string(s),
        Some(Value::Null) => Some(0.0),
        Some(Value::Bool(b)) => Some(if *b { 1.0 } else { 0.0 }),
        Some(v @ Value::Array(_)) => number_string(&crate::text::string(v)),
        _ => None,
    };
    n.filter(|n| n.is_finite()).unwrap_or(fallback)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptionWord {
    pub text: String,
    pub start: f64,
    pub end: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptionCue {
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<CaptionWord>>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptionsContent {
    pub cues: Vec<CaptionCue>,
    pub style: crate::CaptionStyle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

pub fn normalize_words(raw: &Value, start: f64, end: f64) -> Option<Vec<CaptionWord>> {
    let mut words = Vec::new();
    for w in raw.as_array()? {
        let Some(text) = w
            .get("text")
            .and_then(Value::as_str)
            .filter(|s| !crate::text::trim(s).is_empty())
        else {
            continue;
        };
        let (mut a, mut b) = (
            finite(w.get("start"), f64::NAN),
            finite(w.get("end"), f64::NAN),
        );
        if !a.is_finite() || !b.is_finite() {
            continue;
        }
        if b < a {
            std::mem::swap(&mut a, &mut b);
        }
        a = clamp(a, start, end);
        b = clamp(b, a, end);
        words.push(CaptionWord {
            text: crate::text::trim(text).into(),
            start: round(a),
            end: round(b),
        });
    }
    words.sort_by(|a, b| a.start.total_cmp(&b.start));
    (!words.is_empty()).then_some(words)
}
fn fit_words(cue: &mut CaptionCue) {
    if let Some(words) = &cue.words {
        cue.words = normalize_words(
            &serde_json::to_value(words).expect("finite word times"),
            cue.start,
            cue.end,
        );
    }
}

/// Normalizes raw v1–3 cue input: stable start order, same-start speaker merge,
/// preceding overlap trim, six decimal precision and minimum duration.
/// IDs returned by `make_id` are also checked against authored IDs (N3/N7).
pub fn normalize_cues(raw: &Value, make_id: &mut impl FnMut() -> String) -> Vec<CaptionCue> {
    let Some(items) = raw.as_array() else {
        return Vec::new();
    };
    let mut seen: HashSet<String> = items
        .iter()
        .filter_map(|v| v.get("id")?.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    let mut used = HashSet::new();
    let mut cues = Vec::new();
    for item in items {
        if !item.is_object() {
            continue;
        }
        let (mut start, mut end) = (
            finite(item.get("start"), f64::NAN),
            finite(item.get("end"), f64::NAN),
        );
        if !start.is_finite() || !end.is_finite() {
            continue;
        }
        if end < start {
            std::mem::swap(&mut start, &mut end);
        }
        start = start.max(0.0);
        end = end.max(0.0);
        if end - start < MIN_CUE_DURATION - EPS {
            continue;
        }
        let text = clean_cue_text(item.get("text").unwrap_or(&Value::Null));
        if text.is_empty() {
            continue;
        }
        let mut id = item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if id.is_empty() || used.contains(&id) {
            id = make_id();
            // A bad allocator must not hang the kernel. Suffix its proposal
            // deterministically instead of retrying an unbounded callback.
            if id.is_empty() {
                id = "c".into();
            }
            let base = id.clone();
            let mut suffix = 1;
            while seen.contains(&id) || used.contains(&id) {
                id = format!("{base}-{suffix}");
                suffix += 1;
            }
        }
        seen.insert(id.clone());
        used.insert(id.clone());
        let (start, end) = (round(start), round(end));
        let words = normalize_words(item.get("words").unwrap_or(&Value::Null), start, end);
        cues.push(CaptionCue {
            id,
            start,
            end,
            text,
            words,
        });
    }
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut out: Vec<CaptionCue> = Vec::new();
    for cue in cues {
        if let Some(previous) = out.last_mut().filter(|p| cue.start < p.end - EPS) {
            if cue.start - previous.start < MIN_CUE_DURATION {
                previous.text.push('\n');
                previous.text.push_str(&cue.text);
                previous.end = previous.end.max(cue.end);
                if previous.words.is_some() || cue.words.is_some() {
                    let mut words = previous.words.take().unwrap_or_default();
                    words.extend(cue.words.unwrap_or_default());
                    words.sort_by(|a, b| a.start.total_cmp(&b.start));
                    previous.words = Some(words);
                }
                continue;
            }
            previous.end = cue.start;
            if let Some(words) = &mut previous.words {
                words.retain(|w| w.start < previous.end);
                for word in words.iter_mut() {
                    word.end = word.end.min(previous.end);
                }
                if words.is_empty() {
                    previous.words = None;
                }
            }
        }
        out.push(cue);
    }
    out
}
fn normalized(cues: &[CaptionCue]) -> Vec<CaptionCue> {
    normalize_cues(
        &serde_json::to_value(cues).expect("finite cue times"),
        &mut || "c".into(),
    )
}

pub fn normalize_captions_content(
    raw: &Value,
    make_id: &mut impl FnMut() -> String,
) -> CaptionsContent {
    let mut extra = raw.as_object().cloned().unwrap_or_default();
    let cues = normalize_cues(extra.get("cues").unwrap_or(&Value::Null), make_id);
    let style = normalize_caption_style(extra.get("style").unwrap_or(&Value::Null));
    let language = extra
        .get("language")
        .and_then(Value::as_str)
        .filter(|s| valid_language(s))
        .map(str::to_owned);
    for key in ["cues", "style", "language"] {
        extra.remove(key);
    }
    CaptionsContent {
        cues,
        style,
        language,
        extra,
    }
}
pub fn valid_language(s: &str) -> bool {
    let mut parts = s.split('-');
    let first = parts.next().unwrap_or("");
    (2..=3).contains(&first.len())
        && first.bytes().all(|c| c.is_ascii_alphabetic())
        && parts.all(|p| (2..=8).contains(&p.len()) && p.bytes().all(|c| c.is_ascii_alphanumeric()))
}
pub fn cue_index_at(cues: &[CaptionCue], t: f64) -> Option<usize> {
    if !t.is_finite() {
        return None;
    }
    let i = cues.partition_point(|c| c.end <= t);
    cues.get(i).filter(|c| c.start <= t && t < c.end).map(|_| i)
}
pub fn first_cue_ending_after(cues: &[CaptionCue], t: f64) -> usize {
    cues.partition_point(|c| c.end <= t)
}
pub fn cue_range(cues: &[CaptionCue], t0: f64, t1: f64) -> [usize; 2] {
    let first = first_cue_ending_after(cues, t0);
    [
        first,
        first + cues[first..].partition_point(|c| c.start < t1),
    ]
}
pub fn captions_end(cues: &[CaptionCue]) -> f64 {
    cues.last().map_or(0.0, |c| c.end)
}

/// JSON patches preserve the distinction between missing `words` and null.
pub fn update_cues(cues: &[CaptionCue], patches: &Value) -> Vec<CaptionCue> {
    let Some(patches) = patches.as_array() else {
        return cues.to_vec();
    };
    let by_id: HashMap<_, _> = patches
        .iter()
        .filter_map(|p| Some((p.get("id")?.as_str()?, p)))
        .collect();
    let mut out = cues.to_vec();
    let mut touched = Vec::new();
    for index in 0..out.len() {
        let Some(patch) = by_id.get(out[index].id.as_str()) else {
            continue;
        };
        let lower = if index > 0 && !by_id.contains_key(out[index - 1].id.as_str()) {
            out[index - 1].end
        } else {
            0.0
        };
        let upper = if index + 1 < out.len() && !by_id.contains_key(out[index + 1].id.as_str()) {
            out[index + 1].start
        } else {
            f64::INFINITY
        };
        let cue = &mut out[index];
        touched.push(index);
        if let Some(text) = patch.get("text") {
            let text = clean_cue_text(text);
            if text != cue.text {
                cue.text = text;
                if patch.get("words").is_none() {
                    cue.words = None;
                }
            }
        }
        if patch.get("words").is_some_and(Value::is_null) {
            cue.words = None;
        }
        let start = clamp(
            finite(patch.get("start"), cue.start),
            lower,
            upper - MIN_CUE_DURATION,
        );
        let end = clamp(
            finite(patch.get("end"), cue.end),
            start + MIN_CUE_DURATION,
            (start + MIN_CUE_DURATION).max(upper),
        );
        let shift = start.max(0.0) - cue.start;
        if patch.get("words").is_none()
            && shift != 0.0
            && (end - cue.end - shift).abs() < EPS
            && let Some(words) = &mut cue.words
        {
            for w in words {
                w.start += shift;
                w.end += shift;
            }
        }
        cue.start = round(start.max(0.0));
        cue.end = round(end);
        if let Some(words) = patch.get("words").filter(|v| v.is_array()) {
            cue.words = normalize_words(words, cue.start, cue.end);
        }
        fit_words(cue);
    }
    if touched.iter().all(|&i| {
        !out[i].text.is_empty()
            && out[i].end - out[i].start >= MIN_CUE_DURATION - EPS
            && (i == 0 || out[i - 1].end <= out[i].start + EPS)
            && (i + 1 == out.len() || out[i].end <= out[i + 1].start + EPS)
    }) {
        out
    } else {
        normalized(&out)
    }
}
pub fn insert_cues(
    cues: &[CaptionCue],
    added: &Value,
    make_id: &mut impl FnMut() -> String,
) -> Vec<CaptionCue> {
    let mut raw = added.clone();
    if let Some(items) = raw.as_array_mut() {
        for item in items {
            if let Some(id) = item.get("id").and_then(Value::as_str)
                && cues.iter().any(|c| c.id == id)
            {
                item.as_object_mut().unwrap().remove("id");
            }
        }
    }
    let mut out = cues.to_vec();
    // Reserve existing ids as well as freshly allocated ones.
    let mut reserved: HashSet<_> = cues.iter().map(|c| c.id.clone()).collect();
    let mut allocator = || {
        let mut id = make_id();
        if id.is_empty() {
            id = "c".into();
        }
        let base = id.clone();
        let mut n = 1;
        while reserved.contains(&id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        reserved.insert(id.clone());
        id
    };
    for mut cue in normalize_cues(&raw, &mut allocator) {
        let [first, last] = cue_range(&out, cue.start, cue.end);
        let mut start = cue
            .start
            .max(if first > 0 { out[first - 1].end } else { 0.0 });
        let mut end = cue.end;
        if first < last {
            if out[first].start <= start + EPS {
                start = out[first].end;
            }
            if let Some(next) = out[first..].iter().find(|c| c.start >= start - EPS) {
                end = end.min(next.start);
            }
        }
        if end - start < MIN_CUE_DURATION - EPS {
            continue;
        }
        cue.start = round(start);
        cue.end = round(end);
        fit_words(&mut cue);
        let at = first_cue_ending_after(&out, cue.start);
        out.insert(at, cue);
    }
    normalized(&out)
}
pub fn delete_cues(cues: &[CaptionCue], ids: &[String]) -> Vec<CaptionCue> {
    cues.iter()
        .filter(|c| !ids.contains(&c.id))
        .cloned()
        .collect()
}

pub fn split_cue(
    cues: &[CaptionCue],
    id: &str,
    at: f64,
    tail_id: &str,
) -> (Vec<CaptionCue>, Option<String>) {
    let Some(index) = cues.iter().position(|c| c.id == id) else {
        return (cues.to_vec(), None);
    };
    let cue = &cues[index];
    if !at.is_finite()
        || at <= cue.start + MIN_CUE_DURATION - EPS
        || at >= cue.end - MIN_CUE_DURATION + EPS
        || tail_id.is_empty()
        || cues.iter().any(|c| c.id == tail_id)
    {
        return (cues.to_vec(), None);
    }
    let words = cue_words(cue);
    let mut cut = words
        .iter()
        .position(|w| (w.start + w.end) / 2.0 >= at)
        .unwrap_or(words.len());
    if words.len() > 1 {
        cut = cut.clamp(1, words.len() - 1);
    }
    let lines: Vec<_> = cue.text.split('\n').collect();
    let head_text = if words.len() > 1 {
        join_words(words[..cut].iter().map(|w| w.text.as_str()))
    } else {
        lines[0].to_string()
    };
    let tail_text = if words.len() > 1 {
        join_words(words[cut..].iter().map(|w| w.text.as_str()))
    } else {
        let s = join_words(lines.iter().skip(1).copied());
        if s.is_empty() { cue.text.clone() } else { s }
    };
    let mut head = CaptionCue {
        id: cue.id.clone(),
        start: cue.start,
        end: round(at),
        text: head_text,
        words: None,
    };
    let mut tail = CaptionCue {
        id: tail_id.into(),
        start: round(at),
        end: cue.end,
        text: tail_text,
        words: None,
    };
    if let Some(real) = &cue.words {
        head.words = Some(
            real.iter()
                .filter(|w| (w.start + w.end) / 2.0 < at)
                .cloned()
                .collect(),
        );
        tail.words = Some(
            real.iter()
                .filter(|w| (w.start + w.end) / 2.0 >= at)
                .cloned()
                .collect(),
        );
        fit_words(&mut head);
        fit_words(&mut tail);
    }
    let mut out = cues.to_vec();
    out.splice(index..=index, [head, tail]);
    (out, Some(tail_id.into()))
}
pub fn merge_cues(cues: &[CaptionCue], ids: &[String]) -> (Vec<CaptionCue>, Option<String>) {
    let indices: Vec<_> = cues
        .iter()
        .enumerate()
        .filter(|(_, c)| ids.contains(&c.id))
        .map(|(i, _)| i)
        .collect();
    if indices.len() < 2 {
        return (cues.to_vec(), None);
    }
    let (first, last) = (indices[0], *indices.last().unwrap());
    let run = &cues[first..=last];
    let text = join_words(
        run.iter()
            .map(|c| join_words(c.text.split('\n')))
            .collect::<Vec<_>>()
            .iter()
            .map(String::as_str),
    );
    let words = run
        .iter()
        .all(|c| c.words.as_ref().is_some_and(|w| !w.is_empty()))
        .then(|| {
            run.iter()
                .flat_map(|c| c.words.clone().unwrap_or_default())
                .collect()
        });
    let merged = CaptionCue {
        id: run[0].id.clone(),
        start: run[0].start,
        end: run.last().unwrap().end,
        text,
        words,
    };
    let id = merged.id.clone();
    let mut out = cues.to_vec();
    out.splice(first..=last, [merged]);
    (out, Some(id))
}
pub fn move_limits(cues: &[CaptionCue], ids: &[String]) -> [f64; 2] {
    let mut min = f64::NEG_INFINITY;
    let mut max = f64::INFINITY;
    for (i, c) in cues.iter().enumerate().filter(|(_, c)| ids.contains(&c.id)) {
        min = min.max(-c.start);
        if let Some(p) = cues[..i].iter().rev().find(|c| !ids.contains(&c.id)) {
            min = min.max(p.end - c.start);
        }
        if let Some(n) = cues[i + 1..].iter().find(|c| !ids.contains(&c.id)) {
            max = max.min(n.start - c.end);
        }
    }
    [min, max]
}
pub fn move_cues(cues: &[CaptionCue], ids: &[String], by: f64) -> Vec<CaptionCue> {
    if !by.is_finite() {
        return cues.to_vec();
    }
    let [min, max] = move_limits(cues, ids);
    let dt = clamp(by, min.min(0.0), max.max(0.0));
    cues.iter()
        .map(|c| {
            let mut c = c.clone();
            if ids.contains(&c.id) && dt != 0.0 {
                shift(&mut c, dt);
            }
            c
        })
        .collect()
}
#[derive(Clone, Copy, Debug)]
pub enum CueEdge {
    Start,
    End,
}
pub fn trim_cue(cues: &[CaptionCue], id: &str, edge: CueEdge, time: f64) -> Vec<CaptionCue> {
    let key = match edge {
        CueEdge::Start => "start",
        CueEdge::End => "end",
    };
    update_cues(cues, &serde_json::json!([{"id":id,key:time}]))
}
fn shift(c: &mut CaptionCue, by: f64) {
    c.start = round(c.start + by);
    c.end = round(c.end + by);
    if let Some(words) = &mut c.words {
        for w in words {
            w.start = round(w.start + by);
            w.end = round(w.end + by);
        }
    }
}
pub fn shift_all_cues(cues: &[CaptionCue], by: f64) -> Vec<CaptionCue> {
    if !by.is_finite() {
        return cues.to_vec();
    }
    let mut out = cues.to_vec();
    for c in &mut out {
        shift(c, by);
    }
    normalized(&out)
}
pub fn rebase_cues_for_start(cues: &[CaptionCue], delta: f64) -> Vec<CaptionCue> {
    if !delta.is_finite() || delta == 0.0 {
        cues.to_vec()
    } else {
        shift_all_cues(cues, -delta)
    }
}
pub fn clip_cues_to_end(cues: &[CaptionCue], end: f64) -> Vec<CaptionCue> {
    if !end.is_finite() {
        return cues.to_vec();
    }
    normalized(
        &cues
            .iter()
            .filter(|c| c.start < end - EPS)
            .map(|c| {
                let mut c = c.clone();
                c.end = c.end.min(end);
                c
            })
            .collect::<Vec<_>>(),
    )
}
