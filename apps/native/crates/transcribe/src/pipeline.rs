//! One transcription job over decoded audio, as main's transcription worker
//! (`worker.ts`, `chunking.ts`, `decode.ts`, `words.ts`): 16 kHz mono samples
//! in overlapping windows cut at the quietest moment, each window led in
//! with silence, words kept by the window owning their midpoint and joined
//! at the seams, the model's reporting lag taken off, then grouped into
//! sentence-like segments. Pure apart from the model run: tested without the
//! native runtime.
use crate::runtime::{RunResult, SAMPLE_RATE};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub text: String,
    pub start: f64,
    pub end: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub words: Vec<Word>,
}
/// How finely a model times what it hears (the catalog's `timing`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timing {
    Word,
    Segment,
    None,
}
impl Timing {
    pub fn of(name: &str) -> Self {
        match name {
            "word" => Self::Word,
            "segment" => Self::Segment,
            _ => Self::None,
        }
    }
}
fn round(value: f64) -> f64 {
    (value * 1000.).round() / 1000.
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chunking {
    /// Shortest window before a cut is considered, seconds.
    pub min_seconds: f64,
    /// Longest window; the cut is found in [min, max].
    pub max_seconds: f64,
    /// Audio shared with each neighbour beyond the cut, seconds.
    pub overlap_seconds: f64,
}
/// Word-timed models get long overlapping windows (seam words are settled by
/// their times); the others meet at the cut, Whisper's under its 30 s input
/// and text-only models in phrase-sized windows.
pub fn chunking_for(timing: Timing) -> Chunking {
    match timing {
        Timing::Word => Chunking {
            min_seconds: 18.,
            max_seconds: 28.,
            overlap_seconds: 1.,
        },
        Timing::Segment => Chunking {
            min_seconds: 16.,
            max_seconds: 26.,
            overlap_seconds: 0.,
        },
        Timing::None => Chunking {
            min_seconds: 5.,
            max_seconds: 12.,
            overlap_seconds: 0.,
        },
    }
}
const FRAME: usize = SAMPLE_RATE / 100;
const SMOOTH_FRAMES: usize = 20;
fn samples(seconds: f64) -> usize {
    (seconds * SAMPLE_RATE as f64).round() as usize
}
/// Where to cut, as an absolute sample index: the middle of the quietest
/// 200 ms in [from + min, from + max] of `buffer` (starting at `start`).
pub fn find_cut(buffer: &[f32], start: usize, from: usize, o: Chunking) -> usize {
    let lo = (from + samples(o.min_seconds)).max(start);
    let hi = (from + samples(o.max_seconds)).min(start + buffer.len());
    let frames = hi.saturating_sub(lo) / FRAME;
    if frames <= SMOOTH_FRAMES {
        return (from + samples(o.max_seconds)).max(lo).min(hi);
    }
    let energy: Vec<f64> = (0..frames)
        .map(|f| {
            let at = lo - start + f * FRAME;
            buffer[at..at + FRAME]
                .iter()
                .map(|v| (*v as f64) * (*v as f64))
                .sum()
        })
        .collect();
    let mut window: f64 = energy[..SMOOTH_FRAMES].iter().sum();
    let (mut best, mut best_energy) = (0, window);
    for f in SMOOTH_FRAMES..frames {
        window += energy[f] - energy[f - SMOOTH_FRAMES];
        if window < best_energy {
            best_energy = window;
            best = f - SMOOTH_FRAMES + 1;
        }
    }
    lo + (best + SMOOTH_FRAMES / 2) * FRAME
}
/// Silence fed ahead of every window (Parakeet drops a phrase that starts
/// abruptly); whole 80 ms encoder frames, so word times do not shift.
pub const LEAD_IN_SECONDS: f64 = 0.48;
pub fn with_lead_in(samples_in: &[f32]) -> Vec<f32> {
    let mut out = vec![0.; samples(LEAD_IN_SECONDS)];
    out.extend_from_slice(samples_in);
    out
}
/// The stretch that holds sound, as sample indices, or `None` when silent: a
/// 10 ms frame counts within 40 dB of the loudest, above a floor.
pub fn speech_bounds(buffer: &[f32]) -> Option<(usize, usize)> {
    let frames = buffer.len() / FRAME;
    if frames == 0 {
        return None;
    }
    let energy: Vec<f64> = buffer
        .chunks_exact(FRAME)
        .map(|c| c.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / FRAME as f64)
        .collect();
    let peak = energy.iter().copied().fold(0., f64::max);
    if peak < 1e-6 {
        return None;
    }
    let threshold = (peak * 1e-4).max(1e-7);
    let first = energy.iter().position(|e| *e >= threshold)?;
    let last = energy
        .iter()
        .rposition(|e| *e >= threshold)
        .unwrap_or(first);
    Some((first * FRAME, ((last + 1) * FRAME).min(buffer.len())))
}
/// Words whose midpoint lies in [from, to).
pub fn words_within(words: Vec<Word>, from: f64, to: f64) -> Vec<Word> {
    words
        .into_iter()
        .filter(|w| {
            let middle = (w.start + w.end) / 2.;
            middle >= from && middle < to
        })
        .collect()
}
fn normal(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect()
}
/// Appends the next window's words, dropping those the previous window heard.
pub fn append_at_seam(kept: &mut Vec<Word>, incoming: Vec<Word>) {
    let mut skip = 0;
    if let Some(last) = kept.last() {
        for word in &incoming {
            let same_sound = word.start < (last.start + last.end) / 2.;
            let repeated =
                normal(&word.text) == normal(&last.text) && (word.start - last.start).abs() < 0.4;
            if !same_sound && !repeated {
                break;
            }
            skip += 1;
        }
    }
    kept.extend(incoming.into_iter().skip(skip));
}
/// Overlapping windows of streamed samples (`Chunker`).
pub struct AudioWindow {
    pub samples: Vec<f32>,
    /// Absolute sample index of `samples[0]`.
    pub start: usize,
    /// Seconds (from the decoded start) whose words this window owns.
    pub own_from: f64,
    pub own_to: f64,
}
pub struct Chunker {
    buffer: Vec<f32>,
    /// Absolute index of `buffer[0]`.
    start: usize,
    /// Absolute index where the next window's owned time begins.
    from: usize,
    options: Chunking,
}
impl Chunker {
    pub fn new(options: Chunking) -> Self {
        Self {
            buffer: vec![],
            start: 0,
            from: 0,
            options,
        }
    }
    /// Seconds received so far.
    pub fn received(&self) -> f64 {
        (self.start + self.buffer.len()) as f64 / SAMPLE_RATE as f64
    }
    pub fn push(&mut self, more: &[f32]) -> Vec<AudioWindow> {
        self.buffer.extend_from_slice(more);
        let overlap = samples(self.options.overlap_seconds);
        let mut out = vec![];
        while self.start + self.buffer.len()
            >= self.from + samples(self.options.max_seconds) + overlap
        {
            let cut = find_cut(&self.buffer, self.start, self.from, self.options);
            let begin = self.start.max(self.from.saturating_sub(overlap));
            let end = (cut + overlap).min(self.start + self.buffer.len());
            out.push(AudioWindow {
                samples: self.buffer[begin - self.start..end - self.start].to_vec(),
                start: begin,
                own_from: self.from as f64 / SAMPLE_RATE as f64,
                own_to: cut as f64 / SAMPLE_RATE as f64,
            });
            self.from = cut;
            let drop = cut.saturating_sub(overlap).saturating_sub(self.start);
            self.buffer.drain(..drop.min(self.buffer.len()));
            self.start += drop;
        }
        out
    }
    pub fn finish(&mut self) -> Option<AudioWindow> {
        let total = self.start + self.buffer.len();
        if total <= self.from {
            return None;
        }
        let begin = self.start.max(
            self.from
                .saturating_sub(samples(self.options.overlap_seconds)),
        );
        let window = AudioWindow {
            samples: self.buffer[begin - self.start..].to_vec(),
            start: begin,
            own_from: self.from as f64 / SAMPLE_RATE as f64,
            own_to: f64::INFINITY,
        };
        self.from = total;
        Some(window)
    }
}

/// A word after a gap at least this long is an onset after a pause.
const LAG_PAUSE: f64 = 0.2;
/// Takes a model's reporting delay `[onset, inner, end]` off its words: starts
/// move back first (kept in order), then ends move in, never past the next
/// word's new start and never under 20 ms where there is room.
pub fn remove_lag(words: Vec<Word>, [onset, inner, end]: [f64; 3]) -> Vec<Word> {
    let mut starts: Vec<f64> = Vec::with_capacity(words.len());
    for (i, w) in words.iter().enumerate() {
        let shift = match i.checked_sub(1).map(|p| &words[p]) {
            Some(prev) if w.start - prev.end < LAG_PAUSE => inner,
            _ => onset,
        };
        let floor = starts.last().copied().unwrap_or(f64::NEG_INFINITY);
        starts.push(floor.max(w.start - shift));
    }
    words
        .into_iter()
        .enumerate()
        .map(|(i, w)| {
            let from = starts[i];
            let next = starts.get(i + 1).copied().unwrap_or(f64::INFINITY);
            let to = next.min((from + 0.02).max(w.end - end));
            Word {
                start: round(from),
                end: round(from.max(to)),
                ..w
            }
        })
        .collect()
}
fn attached(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| !c.is_alphanumeric() && !c.is_whitespace())
}
fn cjk(c: char) -> bool {
    pm_cjk(c)
}
fn pm_cjk(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xac00..=0xd7af | 0xf900..=0xfaff)
}
/// The words of a phrase timed only as a whole (or a text-only window): the
/// span spread over the words by length; CJK splits per character.
pub fn spread_words(text: &str, start: f64, end: f64) -> Vec<Word> {
    let mut pieces: Vec<String> = vec![];
    for chunk in text.split_whitespace() {
        if attached(chunk) {
            if let Some(last) = pieces.last_mut() {
                last.push_str(chunk);
            }
            continue;
        }
        if !chunk.chars().any(cjk) {
            pieces.push(chunk.into());
            continue;
        }
        let mut run = String::new();
        for c in chunk.chars() {
            if cjk(c) {
                if !run.is_empty() {
                    pieces.push(std::mem::take(&mut run));
                }
                pieces.push(c.into());
            } else if attached(&c.to_string()) && run.is_empty() && !pieces.is_empty() {
                pieces.last_mut().unwrap().push(c);
            } else {
                run.push(c);
            }
        }
        if !run.is_empty() {
            pieces.push(run);
        }
    }
    if pieces.is_empty() {
        return vec![];
    }
    let span = (end - start).max(0.);
    let weights: Vec<f64> = pieces
        .iter()
        .map(|p| (p.chars().filter(|c| c.is_alphanumeric()).count() + 1) as f64)
        .collect();
    let total: f64 = weights.iter().sum();
    let mut at = start;
    let last = pieces.len() - 1;
    pieces
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            let from = at;
            at += span * weights[i] / total;
            Word {
                text,
                start: round(from),
                end: round(if i == last { end } else { at }),
                confidence: None,
            }
        })
        .collect()
}
/// Where a window's audio sits in the source.
pub struct WindowTiming {
    /// Source seconds of the run's first sample (lead-in included).
    pub offset: f64,
    /// Source seconds where the window's sound starts and ends.
    pub sound_from: f64,
    pub sound_to: f64,
}
/// One window's run as words in source seconds. A word-timed model's own
/// word table (transcribe.cpp merges its tokens) has its lag removed; a
/// phrase-timed one shares each phrase's span; text only shares the sound.
pub fn window_words(
    result: &RunResult,
    timing: Timing,
    window: &WindowTiming,
    lag: Option<[f64; 3]>,
) -> Vec<Word> {
    if timing == Timing::Word && !result.words.is_empty() {
        let mut words: Vec<Word> = vec![];
        for w in &result.words {
            let text = w.text.trim();
            if text.is_empty() {
                continue;
            }
            let start = window.offset + w.t0_ms as f64 / 1000.;
            let end = window.offset + w.t1_ms.max(w.t0_ms) as f64 / 1000.;
            // Punctuation the model emits as its own word belongs to the last.
            if attached(text)
                && let Some(last) = words.last_mut()
            {
                last.text.push_str(text);
                continue;
            }
            words.push(Word {
                text: text.into(),
                start: round(start),
                end: round(end),
                confidence: None,
            });
        }
        return match lag {
            Some(lag) => remove_lag(words, lag),
            None => words,
        };
    }
    let clamp = |v: f64| v.max(window.sound_from).min(window.sound_to);
    if timing != Timing::None {
        let timed: Vec<_> = result
            .segments
            .iter()
            .filter(|s| !s.text.trim().is_empty() && s.t1_ms > s.t0_ms)
            .collect();
        if !timed.is_empty() {
            return timed
                .into_iter()
                .flat_map(|s| {
                    let start = clamp(window.offset + s.t0_ms as f64 / 1000.);
                    let end = clamp(window.offset + s.t1_ms as f64 / 1000.);
                    spread_words(&s.text, start, end.max(start))
                })
                .collect();
        }
    }
    spread_words(&result.text, window.sound_from, window.sound_to)
}
fn sentence_end(text: &str) -> bool {
    text.trim_end_matches(['"', '\'', '”', '’', ')', ']'])
        .chars()
        .next_back()
        .is_some_and(|c| ".!?…。！？".contains(c))
}
fn clause_end(text: &str) -> bool {
    text.trim_end_matches(['"', '\'', '”', '’', ')', ']'])
        .chars()
        .next_back()
        .is_some_and(|c| ",;:，、；：".contains(c))
}
pub fn join_words(words: &[Word]) -> String {
    let mut text = String::new();
    for w in words {
        if text.is_empty() {
            text = w.text.clone();
        } else if w.text.chars().next().is_some_and(cjk)
            && text.chars().next_back().is_some_and(cjk)
        {
            text.push_str(&w.text);
        } else {
            text.push(' ');
            text.push_str(&w.text);
        }
    }
    text
}
fn split_long(words: Vec<Word>, max_duration: f64) -> Vec<Vec<Word>> {
    let (Some(first), Some(last)) = (words.first(), words.last()) else {
        return vec![words];
    };
    let span = last.end - first.start;
    if span <= max_duration || words.len() < 2 {
        return vec![words];
    }
    let middle = (first.start + last.end) / 2.;
    let mut best = 0;
    let mut best_score = f64::NEG_INFINITY;
    for i in 0..words.len() - 1 {
        let gap = words[i + 1].start - words[i].end;
        let clause = if clause_end(&words[i].text) { 1. } else { 0. };
        let centred = 1. - (words[i].end - middle).abs() / span;
        let score = clause * 2. + gap * 4. + centred;
        if score > best_score {
            best_score = score;
            best = i;
        }
    }
    let mut words = words;
    let rest = words.split_off(best + 1);
    let mut out = split_long(words, max_duration);
    out.extend(split_long(rest, max_duration));
    out
}
/// Sentence-like segments: a sentence's end or a 0.8 s pause ends one, and
/// none runs past 12 s (split at its best clause break or widest gap).
pub fn words_to_segments(words: Vec<Word>) -> Vec<Segment> {
    let mut groups: Vec<Vec<Word>> = vec![];
    let mut current: Vec<Word> = vec![];
    for word in words {
        if current.last().is_some_and(|p| word.start - p.end >= 0.8) {
            groups.push(std::mem::take(&mut current));
        }
        let ends = sentence_end(&word.text);
        current.push(word);
        if ends {
            groups.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
        .into_iter()
        .flat_map(|g| split_long(g, 12.))
        .filter(|g| !g.is_empty())
        .map(|words| Segment {
            text: join_words(&words),
            start: words[0].start,
            end: words[words.len() - 1].end,
            words,
        })
        .collect()
}

/// Seconds of real audio decoded on each side of a requested span, so its
/// edge words are heard whole; words are then kept by their midpoint.
pub const SPAN_CONTEXT_SECONDS: f64 = 1.;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    /// What to decode, in source seconds.
    pub start: f64,
    pub end: Option<f64>,
    /// The span asked for.
    pub keep_from: f64,
    pub keep_to: f64,
}
pub fn padded_span(start: Option<f64>, end: Option<f64>) -> Span {
    let keep_from = start.filter(|s| *s > 0.).unwrap_or(0.);
    let keep_to = end.filter(|e| *e > keep_from).unwrap_or(f64::INFINITY);
    Span {
        start: (keep_from - SPAN_CONTEXT_SECONDS).max(0.),
        end: keep_to
            .is_finite()
            .then_some(keep_to + SPAN_CONTEXT_SECONDS),
        keep_from,
        keep_to,
    }
}

/// Streamed 16 kHz mono samples of the span being transcribed.
pub trait Audio {
    /// Appends what is next; `false` at the end.
    fn read(&mut self, out: &mut Vec<f32>) -> Result<bool, String>;
}
/// What a run of the loaded model answers for one window.
pub trait Recognizer {
    fn run(&mut self, samples: &[f32]) -> Result<RunResult, crate::runtime::Error>;
}
pub struct Heard {
    pub duration: f64,
    pub segments: Vec<Segment>,
    /// The language the model detected, as its own tag.
    pub language: Option<String>,
}
#[derive(Debug)]
pub enum Failure {
    Cancelled,
    Audio(String),
    Model(String),
}
/// The whole job: `audio` holds the span from `span.start`, `seconds` its
/// expected length (for progress). Cancel is checked between windows; the
/// recognizer's own abort flag stops a window in progress.
#[allow(clippy::too_many_arguments)]
pub fn transcribe(
    recognizer: &mut dyn Recognizer,
    audio: &mut dyn Audio,
    span: Span,
    seconds: Option<f64>,
    timing: Timing,
    lag: Option<[f64; 3]>,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(f64),
) -> Result<Heard, Failure> {
    use std::sync::atomic::Ordering;
    let offset = span.start;
    let mut chunker = Chunker::new(chunking_for(timing));
    let mut words: Vec<Word> = vec![];
    let mut language = String::new();
    let mut last = -1.;
    let mut run = |window: AudioWindow,
                   words: &mut Vec<Word>,
                   language: &mut String|
     -> Result<(), Failure> {
        if cancel.load(Ordering::Relaxed) {
            return Err(Failure::Cancelled);
        }
        let Some((from, to)) = speech_bounds(&window.samples) else {
            return Ok(());
        };
        let result = match recognizer.run(&with_lead_in(&window.samples)) {
            Ok(result) => result,
            Err(crate::runtime::Error::Partial(result)) => *result,
            Err(crate::runtime::Error::Cancelled) => return Err(Failure::Cancelled),
            Err(e) => return Err(Failure::Model(e.to_string())),
        };
        let start = window.start as f64 / SAMPLE_RATE as f64;
        let heard = window_words(
            &result,
            timing,
            &WindowTiming {
                offset: start - LEAD_IN_SECONDS,
                sound_from: start + from as f64 / SAMPLE_RATE as f64,
                sound_to: start + to as f64 / SAMPLE_RATE as f64,
            },
            lag,
        );
        let owned = if timing == Timing::Word {
            let from = if window.own_from > 0. {
                window.own_from
            } else {
                f64::NEG_INFINITY
            };
            words_within(heard, from, window.own_to)
        } else {
            heard
        };
        let shifted: Vec<Word> = owned
            .into_iter()
            .map(|w| Word {
                start: round((w.start + offset).max(0.)),
                end: round((w.end + offset).max(0.)),
                ..w
            })
            .collect();
        if language.is_empty() && !shifted.is_empty() {
            *language = result.language.clone();
        }
        if timing == Timing::Word {
            append_at_seam(words, shifted);
        } else {
            words.extend(shifted);
        }
        Ok(())
    };
    let mut buffer = vec![];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Failure::Cancelled);
        }
        buffer.clear();
        let more = audio.read(&mut buffer).map_err(Failure::Audio)?;
        for window in chunker.push(&buffer) {
            let own_to = window.own_to;
            run(window, &mut words, &mut language)?;
            if let Some(total) = seconds.filter(|t| *t > 0.) {
                let value = (own_to / total).min(0.99);
                if value - last >= 0.01 {
                    last = value;
                    progress(value);
                }
            }
        }
        if !more {
            break;
        }
    }
    if let Some(rest) = chunker.finish()
        && rest.samples.len() >= SAMPLE_RATE / 10
    {
        run(rest, &mut words, &mut language)?;
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(Failure::Cancelled);
    }
    let duration =
        round(((offset + chunker.received()).min(span.keep_to) - span.keep_from).max(0.));
    let kept = if span.keep_from > 0. || span.keep_to.is_finite() {
        let from = if span.keep_from > 0. {
            span.keep_from
        } else {
            f64::NEG_INFINITY
        };
        words_within(words, from, span.keep_to)
    } else {
        words
    };
    progress(1.);
    Ok(Heard {
        duration,
        segments: words_to_segments(kept),
        language: (!language.is_empty()).then_some(language),
    })
}

/// Mono at 16 kHz from any rate: channels averaged, then a 32-tap
/// Blackman-windowed sinc low-pass at the new Nyquist, read at the output
/// rate. Streaming: feed source blocks in order.
pub struct Resampler {
    ratio: f64,
    cutoff: f64,
    history: Vec<f32>,
    /// Source index (into `history`) of the next output sample.
    position: f64,
}
const TAPS: usize = 32;
impl Resampler {
    pub fn new(source_rate: u32) -> Self {
        let ratio = source_rate.max(1) as f64 / SAMPLE_RATE as f64;
        Self {
            ratio,
            cutoff: if ratio > 1. { 0.95 / ratio } else { 0.95 },
            history: vec![0.; TAPS / 2],
            position: (TAPS / 2) as f64,
        }
    }
    /// Downmixes `channels` (planar) and appends the resampled block.
    pub fn push(&mut self, channels: &[Vec<f32>], out: &mut Vec<f32>) {
        let n = channels.first().map_or(0, Vec::len);
        let scale = 1. / channels.len().max(1) as f32;
        self.history.extend((0..n).map(|i| {
            channels
                .iter()
                .map(|c| c.get(i).copied().unwrap_or(0.))
                .sum::<f32>()
                * scale
        }));
        let half = TAPS as f64 / 2.;
        while self.position + half < self.history.len() as f64 {
            let centre = self.position.floor();
            let frac = self.position - centre;
            let mut sum = 0.;
            let mut weight = 0.;
            for k in 0..TAPS {
                let index = centre as isize + k as isize - (TAPS / 2 - 1) as isize;
                let x = (index as f64 - centre) - frac;
                let sinc = if x.abs() < 1e-9 {
                    1.
                } else {
                    let a = std::f64::consts::PI * x * self.cutoff;
                    a.sin() / a
                };
                let n = (x + half) / TAPS as f64;
                let window = 0.42 - 0.5 * (2. * std::f64::consts::PI * n).cos()
                    + 0.08 * (4. * std::f64::consts::PI * n).cos();
                let tap = sinc * window.max(0.);
                if let Some(v) = usize::try_from(index)
                    .ok()
                    .and_then(|i| self.history.get(i))
                {
                    sum += *v as f64 * tap;
                }
                weight += tap;
            }
            out.push((sum / weight.max(1e-9)) as f32);
            self.position += self.ratio;
        }
        // Keep what the next outputs still read.
        let keep_from = (self.position.floor() as usize).saturating_sub(TAPS);
        if keep_from > 0 {
            self.history.drain(..keep_from);
            self.position -= keep_from as f64;
        }
    }
    /// Flushes the tail with silence after the last block.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        self.push(&[vec![0.; TAPS]], out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{RunSegment, RunWord};
    fn w(text: &str, start: f64, end: f64) -> Word {
        Word {
            text: text.into(),
            start,
            end,
            confidence: None,
        }
    }
    #[test]
    fn windows_overlap_and_cut_at_the_quietest_moment() {
        let o = chunking_for(Timing::Word);
        let mut tone: Vec<f32> = (0..samples(40.))
            .map(|i| (i as f32 * 0.05).sin() * 0.5)
            .collect();
        // A breath at 22–22.3 s.
        for v in &mut tone[samples(22.)..samples(22.3)] {
            *v = 0.;
        }
        let mut chunker = Chunker::new(o);
        let mut windows = chunker.push(&tone);
        windows.extend(chunker.finish());
        assert_eq!(windows.len(), 2);
        assert!(
            (windows[0].own_to - 22.1).abs() < 0.02,
            "{}",
            windows[0].own_to
        );
        assert_eq!(windows[1].own_from, windows[0].own_to);
        // A second of shared audio each side of the cut.
        let first_end = windows[0].start + windows[0].samples.len();
        assert_eq!(first_end - windows[1].start, samples(2.));
        assert!(windows[1].own_to.is_infinite());
    }
    #[test]
    fn silence_has_no_speech_and_sound_is_bounded() {
        assert_eq!(speech_bounds(&vec![0.; 16_000]), None);
        let mut v = vec![0.; 16_000];
        for x in &mut v[4_000..8_000] {
            *x = 0.3;
        }
        assert_eq!(speech_bounds(&v), Some((4_000, 8_000)));
        assert_eq!(with_lead_in(&[1.]).len(), samples(LEAD_IN_SECONDS) + 1);
    }
    #[test]
    fn seam_words_heard_twice_are_kept_once() {
        let mut kept = vec![w("hello", 10., 10.4), w("world", 10.5, 10.9)];
        append_at_seam(
            &mut kept,
            vec![w("World", 10.52, 10.88), w("again", 11., 11.3)],
        );
        let texts: Vec<_> = kept.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["hello", "world", "again"]);
    }
    #[test]
    fn lag_moves_starts_back_and_keeps_order() {
        let words = remove_lag(
            vec![w("a", 1., 1.3), w("b", 1.35, 1.6), w("c", 3., 3.2)],
            [0.32, 0.2, 0.],
        );
        assert_eq!(words[0].start, 0.68);
        assert_eq!(words[1].start, 1.15);
        assert_eq!(words[0].end, 1.15);
        assert_eq!(words[2].start, 2.68);
    }
    #[test]
    fn phrase_timing_spreads_words_and_punctuation_joins() {
        let words = spread_words("warm . day", 0., 1.);
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "warm.");
        assert_eq!(words[1].end, 1.);
        assert_eq!(spread_words("你好", 0., 1.).len(), 2);
    }
    #[test]
    fn segments_end_at_sentences_pauses_and_twelve_seconds() {
        let segments = words_to_segments(vec![
            w("One.", 0., 0.5),
            w("Two", 0.6, 0.9),
            w("three", 2., 2.4),
        ]);
        let texts: Vec<_> = segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["One.", "Two", "three"]);
        let long: Vec<_> = (0..30)
            .map(|i| w("go", i as f64 * 0.5, i as f64 * 0.5 + 0.4))
            .collect();
        assert!(
            words_to_segments(long)
                .iter()
                .all(|s| s.end - s.start <= 12.)
        );
    }
    #[test]
    fn word_tables_become_source_words_without_the_lead_in() {
        let result = RunResult {
            text: "hi there".into(),
            language: "en".into(),
            segments: vec![RunSegment {
                text: "hi there".into(),
                t0_ms: 480,
                t1_ms: 1480,
            }],
            words: vec![
                RunWord {
                    text: " hi".into(),
                    t0_ms: 500,
                    t1_ms: 800,
                    segment: 0,
                },
                RunWord {
                    text: "there".into(),
                    t0_ms: 900,
                    t1_ms: 1400,
                    segment: 0,
                },
                RunWord {
                    text: "!".into(),
                    t0_ms: 1400,
                    t1_ms: 1420,
                    segment: 0,
                },
            ],
        };
        let timing = WindowTiming {
            offset: 10. - LEAD_IN_SECONDS,
            sound_from: 10.,
            sound_to: 11.,
        };
        let words = window_words(&result, Timing::Word, &timing, None);
        assert_eq!(
            words,
            vec![w("hi", 10.02, 10.32), w("there!", 10.42, 10.92)]
        );
        let phrase = window_words(&result, Timing::Segment, &timing, None);
        assert_eq!(phrase.first().map(|w| w.start), Some(10.));
        let text = window_words(&result, Timing::None, &timing, None);
        assert_eq!(text.last().map(|w| w.end), Some(11.));
    }
    struct Blocks(Vec<Vec<f32>>);
    impl Audio for Blocks {
        fn read(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
            if self.0.is_empty() {
                return Ok(false);
            }
            out.extend(self.0.remove(0));
            Ok(!self.0.is_empty())
        }
    }
    /// Answers every window with one word at its first sound.
    struct Echo(usize);
    impl Recognizer for Echo {
        fn run(&mut self, samples: &[f32]) -> Result<RunResult, crate::runtime::Error> {
            self.0 += 1;
            let (from, _) = speech_bounds(samples).unwrap();
            let t0 = (from * 1000 / SAMPLE_RATE) as i64;
            Ok(RunResult {
                text: "word".into(),
                language: "en".into(),
                segments: vec![],
                words: vec![RunWord {
                    text: format!("w{}", self.0),
                    t0_ms: t0,
                    t1_ms: t0 + 300,
                    segment: 0,
                }],
            })
        }
    }
    #[test]
    fn a_job_keeps_the_requested_span_and_reports_progress() {
        let mut audio: Vec<f32> = vec![0.; samples(60.)];
        // Bursts of sound at 2 s, 31 s and 50 s of the decoded audio.
        for at in [2., 31., 50.] {
            for v in &mut audio[samples(at)..samples(at + 0.3)] {
                *v = 0.4;
            }
        }
        let blocks = audio.chunks(16_000).map(<[f32]>::to_vec).collect();
        let span = padded_span(Some(3.), Some(52.));
        assert_eq!(span.start, 2.);
        let mut seen = vec![];
        let heard = transcribe(
            &mut Echo(0),
            &mut Blocks(blocks),
            span,
            Some(51.),
            Timing::Word,
            None,
            &std::sync::atomic::AtomicBool::new(false),
            &mut |p| seen.push(p),
        )
        .unwrap();
        let words: Vec<_> = heard.segments.iter().flat_map(|s| &s.words).collect();
        // The burst at 2 s of decoded audio is 4 s of source: inside [3, 52).
        assert!(
            words.iter().all(|w| w.start >= 3. && w.start < 52.),
            "{words:?}"
        );
        assert!(!words.is_empty());
        assert_eq!(seen.last(), Some(&1.));
        assert!(seen.windows(2).all(|p| p[0] <= p[1]));
        assert_eq!(heard.language.as_deref(), Some("en"));
        let cancelled = transcribe(
            &mut Echo(0),
            &mut Blocks(vec![vec![0.; 16_000]]),
            span,
            None,
            Timing::Word,
            None,
            &std::sync::atomic::AtomicBool::new(true),
            &mut |_| {},
        );
        assert!(matches!(cancelled, Err(Failure::Cancelled)));
    }
    #[test]
    fn resampling_keeps_a_low_tone_and_its_length() {
        let mut r = Resampler::new(48_000);
        let tone: Vec<f32> = (0..48_000)
            .map(|i| (i as f32 * 2. * std::f32::consts::PI * 440. / 48_000.).sin())
            .collect();
        let mut out = vec![];
        for block in tone.chunks(4_096) {
            r.push(&[block.to_vec(), block.to_vec()], &mut out);
        }
        r.finish(&mut out);
        assert!((out.len() as i64 - 16_000).abs() <= 20, "{}", out.len());
        let peak = out[1_000..15_000].iter().fold(0f32, |a, v| a.max(v.abs()));
        assert!((peak - 1.).abs() < 0.05, "{peak}");
        // A tone above the new Nyquist is filtered out.
        let mut r = Resampler::new(48_000);
        let high: Vec<f32> = (0..48_000)
            .map(|i| (i as f32 * 2. * std::f32::consts::PI * 12_000. / 48_000.).sin())
            .collect();
        let mut out = vec![];
        r.push(&[high], &mut out);
        let peak = out[1_000..15_000].iter().fold(0f32, |a, v| a.max(v.abs()));
        assert!(peak < 0.1, "{peak}");
    }
}
