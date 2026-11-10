//! Captions from speech (today's `captions/time-map.ts` and `segment.ts`):
//! transcript words in source seconds are placed on the composition through a
//! clip's timing, then grouped into readable cues. Pure: the caller supplies
//! the clip's `composition time → source time` samples and new cue IDs.
use crate::{CaptionCue, CaptionWord, MIN_CUE_DURATION, display_length, join_words};

/// A clip's composition timing: `from`/`dur` in composition seconds and the
/// source time it shows at evenly spaced composition times from `from` to
/// `from + dur` (two samples for a constant-rate clip).
#[derive(Clone, Debug, PartialEq)]
pub struct ClipTiming {
    pub from: f64,
    pub dur: f64,
    pub samples: Vec<f64>,
}
impl ClipTiming {
    /// Constant-rate clips, the common case (`linearClip`).
    pub fn linear(from: f64, dur: f64, trim: f64, speed: f64) -> Self {
        Self {
            from,
            dur,
            samples: vec![trim, dur * speed + trim],
        }
    }
    fn segments(&self) -> Vec<[f64; 4]> {
        let steps = self.samples.len().saturating_sub(1);
        (0..steps)
            .map(|i| {
                let t = |i: usize| self.from + self.dur * i as f64 / steps as f64;
                [t(i), t(i + 1), self.samples[i], self.samples[i + 1]]
            })
            .collect()
    }
    /// The source span the clip shows, to transcribe only what is used.
    pub fn visible_source_range(&self) -> [f64; 2] {
        let (lo, hi) = self
            .samples
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), s| {
                (lo.min(*s), hi.max(*s))
            });
        if lo > hi {
            return [0., 0.];
        }
        [lo.max(0.), hi.max(0.)]
    }
}
fn at([t0, t1, s0, s1]: [f64; 4], s: f64) -> f64 {
    let span = s1 - s0;
    if span.abs() < 1e-9 {
        t0
    } else {
        t0 + (s - s0) / span * (t1 - t0)
    }
}
/// First composition time at which the clip shows source time `s`.
fn first_crossing(list: &[[f64; 4]], s: f64, forward: bool) -> Option<f64> {
    if forward {
        let last = list.last()?;
        if s < list[0][2] || s > last[3] {
            return None;
        }
        let i = list.partition_point(|seg| seg[3] < s);
        return list.get(i).map(|seg| at(*seg, s));
    }
    list.iter()
        .find(|seg| s >= seg[2].min(seg[3]) && s <= seg[2].max(seg[3]))
        .map(|seg| at(*seg, s))
}
/// Words in composition time, in order. A word the clip shows more than once
/// (a remap that loops back) is placed at its first appearance; words cut by
/// the clip's edges are clipped to them and words it never shows are dropped.
pub fn map_words_to_composition(words: &[CaptionWord], clip: &ClipTiming) -> Vec<CaptionWord> {
    let list = clip.segments();
    if list.is_empty() {
        return vec![];
    }
    let [lo, hi] = {
        let (lo, hi) = clip
            .samples
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), s| {
                (a.min(*s), b.max(*s))
            });
        [lo, hi]
    };
    let forward = list.iter().all(|seg| seg[3] >= seg[2]);
    let (start, end) = (clip.from, clip.from + clip.dur);
    let mut out: Vec<CaptionWord> = words
        .iter()
        .filter(|w| w.end >= lo && w.start <= hi)
        .filter_map(|w| {
            let ta = first_crossing(&list, w.start.max(lo), forward)?;
            let tb = first_crossing(&list, w.end.min(hi), forward)?;
            let (a, b) = (start.max(ta.min(tb)), end.min(ta.max(tb)));
            (b >= a).then(|| CaptionWord {
                text: w.text.clone(),
                start: a,
                end: b,
            })
        })
        .collect();
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    out
}

/// Readable-cue rules (`DEFAULT_SEGMENT_OPTIONS`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SegmentOptions {
    pub max_chars_per_line: usize,
    pub max_lines: usize,
    /// Seconds a cue may stay on screen.
    pub max_duration: f64,
    /// Seconds of silence that always start a new cue.
    pub pause: f64,
    /// Seconds a cue lingers after its last word when nothing follows.
    pub linger: f64,
    /// Shortest cue; short cues extend into the following gap.
    pub min_duration: f64,
}
impl Default for SegmentOptions {
    fn default() -> Self {
        Self {
            max_chars_per_line: 42,
            max_lines: 2,
            max_duration: 6.,
            pause: 0.7,
            linger: 0.4,
            min_duration: 0.8,
        }
    }
}
const CLOSERS: &[char] = &['"', '\'', '”', '’', ')', ']'];
fn ends_with_any(text: &str, marks: &[char]) -> bool {
    text.trim_end_matches(CLOSERS)
        .chars()
        .next_back()
        .is_some_and(|c| marks.contains(&c))
}
fn sentence_end(text: &str) -> bool {
    ends_with_any(text, &['.', '!', '?', '…', '。', '！', '？'])
}
fn clause_end(text: &str) -> bool {
    ends_with_any(text, &[',', ';', ':', '—', '–', '、', '，', '；', '：'])
}
fn text_of(words: &[CaptionWord]) -> String {
    join_words(words.iter().map(|w| w.text.as_str()))
}
/// Timed words to cues: at most two lines of about 42 characters, at most six
/// seconds on screen, a new cue after a pause or a sentence's end, and an
/// overfull cue broken at its last clause break in the back half.
pub fn segment_words(
    input: &[CaptionWord],
    options: SegmentOptions,
    make_id: &mut impl FnMut() -> String,
) -> Vec<CaptionCue> {
    let capacity = options.max_chars_per_line.max(8) * options.max_lines.max(1);
    let mut words: Vec<CaptionWord> = input
        .iter()
        .map(|w| CaptionWord {
            text: w.text.trim().to_owned(),
            start: w.start,
            end: w.end.max(w.start),
        })
        .filter(|w| !w.text.is_empty() && w.start.is_finite() && w.end.is_finite())
        .collect();
    words.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut groups: Vec<Vec<CaptionWord>> = vec![];
    let mut current: Vec<CaptionWord> = vec![];
    for word in words {
        if let Some(last) = current.last() {
            let gap = word.start - last.end;
            let mut grown = current.clone();
            grown.push(word.clone());
            let length = display_length(&text_of(&grown));
            let duration = word.end - current[0].start;
            if gap >= options.pause
                || sentence_end(&last.text) && display_length(&text_of(&current)) >= 12
            {
                groups.push(std::mem::take(&mut current));
            } else if length > capacity || duration > options.max_duration {
                let half = current.len().div_ceil(2);
                let clause = (half..current.len())
                    .rev()
                    .find(|&i| clause_end(&current[i].text));
                match clause {
                    Some(i) if i < current.len() - 1 => {
                        let rest = current.split_off(i + 1);
                        groups.push(std::mem::replace(&mut current, rest));
                    }
                    _ => groups.push(std::mem::take(&mut current)),
                }
            }
        }
        current.push(word);
    }
    if !current.is_empty() {
        groups.push(current);
    }
    let mut cues: Vec<CaptionCue> = groups
        .into_iter()
        .map(|group| {
            let start = group[0].start;
            let end = group[group.len() - 1].end.max(start + MIN_CUE_DURATION);
            CaptionCue {
                id: make_id(),
                start,
                end,
                text: text_of(&group),
                words: Some(group),
            }
        })
        .collect();
    // Linger and the minimum duration only ever extend into free time.
    for i in 0..cues.len() {
        let next = cues.get(i + 1).map(|c| c.start);
        let limit = next.unwrap_or(f64::INFINITY);
        let lingers = next.is_none_or(|n| n - cues[i].end >= options.pause);
        let wanted = (cues[i].end + if lingers { options.linger } else { 0. })
            .max(cues[i].start + options.min_duration);
        cues[i].end = cues[i].end.max(wanted.min(limit));
    }
    let raw = serde_json::to_value(&cues).unwrap_or_default();
    crate::normalize_cues(&raw, make_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn w(text: &str, start: f64, end: f64) -> CaptionWord {
        CaptionWord {
            text: text.into(),
            start,
            end,
        }
    }
    fn ids() -> impl FnMut() -> String {
        let mut n = 0;
        move || {
            n += 1;
            format!("c{n}")
        }
    }
    #[test]
    fn a_trimmed_faster_clip_places_words_on_the_composition() {
        // Starts at 10 s showing source 4 s onward at 2×.
        let clip = ClipTiming::linear(10., 3., 4., 2.);
        assert_eq!(clip.visible_source_range(), [4., 10.]);
        let words = [
            w("before", 1., 2.),
            w("edge", 3.5, 4.5),
            w("in", 6., 7.),
            w("after", 11., 12.),
        ];
        let out = map_words_to_composition(&words, &clip);
        assert_eq!(out, vec![w("edge", 10., 10.25), w("in", 11., 11.5)]);
    }
    #[test]
    fn a_reversed_clip_keeps_each_word_at_its_first_appearance() {
        let clip = ClipTiming {
            from: 0.,
            dur: 4.,
            samples: vec![4., 3., 2., 1., 0.],
        };
        let out = map_words_to_composition(&[w("late", 3., 3.5), w("early", 0.5, 1.)], &clip);
        assert_eq!(out, vec![w("late", 0.5, 1.), w("early", 3., 3.5)]);
    }
    #[test]
    fn pauses_and_sentences_start_cues_and_short_cues_linger() {
        let words = [
            w("Hello", 0., 0.4),
            w("there.", 0.45, 0.9),
            w("This", 1.0, 1.2),
            w("is", 1.25, 1.4),
            w("a", 1.45, 1.5),
            w("test", 1.55, 1.9),
            w("after", 3.0, 3.3),
        ];
        let cues = segment_words(&words, SegmentOptions::default(), &mut ids());
        let texts: Vec<_> = cues.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["Hello there.", "This is a test", "after"]);
        // Continuous speech never lingers; a cue before a pause does.
        assert_eq!(cues[0].end, 0.9);
        assert_eq!(cues[1].end, 2.3);
        assert_eq!(cues[2].end, 3.8);
        assert_eq!(cues[1].words.as_ref().unwrap().len(), 4);
        // A short sentence does not end a cue on its own.
        let short = segment_words(
            &[w("Hi.", 0., 0.3), w("You", 0.4, 0.6)],
            SegmentOptions::default(),
            &mut ids(),
        );
        assert_eq!(short[0].text, "Hi. You");
    }
    #[test]
    fn an_overfull_cue_breaks_at_its_last_clause_in_the_back_half() {
        let text = "one two three four five six, seven eight nine ten eleven twelve thirteen";
        let words: Vec<_> = text
            .split(' ')
            .enumerate()
            .map(|(i, t)| w(t, i as f64 * 0.3, i as f64 * 0.3 + 0.25))
            .collect();
        let options = SegmentOptions {
            max_chars_per_line: 20,
            ..Default::default()
        };
        let cues = segment_words(&words, options, &mut ids());
        assert_eq!(cues[0].text, "one two three four five six,");
        assert!(cues.iter().all(|c| display_length(&c.text) <= 40));
    }
}
