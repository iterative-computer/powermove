//! Forgiving SubRip/WebVTT import, canonical sidecar export. JSON cue serialization
//! is lossless; sidecars carry display text/times, not the project cue identity,
//! caption styles or original file metadata (the caller retains source bytes).
use crate::{CaptionCue, CaptionWord, MIN_CUE_DURATION, model::normalize_words, normalize_cues};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptionFormat {
    Srt,
    Vtt,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParsedCaptions {
    pub format: CaptionFormat,
    pub cues: Vec<CaptionCue>,
    pub skipped: usize,
}
struct Patterns {
    time: Regex,
    timing: Regex,
    inline: Regex,
    ruby: Regex,
    tags: Regex,
    classes: Regex,
    ass: Regex,
    entities: Regex,
}
fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| {
        let time = r"(?:([0-9]+):)?([0-9]{1,2}):([0-9]{1,2})(?:[.,:]([0-9]{1,3}))?";
        let regex = |s: &str| Regex::new(s).unwrap();
        Patterns {
            time: regex(&format!(r"^\s*{time}\s*$")),
            timing: regex(&format!(r"^\s*{time}\s*-+>\s*{time}(.*)$")),
            inline: regex(r"<((?:[0-9]+:)?[0-9]{1,2}:[0-9]{1,2}[.,][0-9]{1,3})>"),
            ruby: regex(r"(?is)<rt>.*?</rt>"),
            tags: regex(r"(?i)</?[a-z][^>]*>"),
            classes: regex(r"(?i)</?[a-z]*(?:\.[^>\s]*)?>"),
            ass: regex(r"\{\\[^}]*\}"),
            entities: regex(r"(?i)&(#x[0-9a-f]+|#[0-9]+|[a-z]+);"),
        }
    })
}
fn seconds(c: &regex::Captures<'_>, i: usize) -> Option<f64> {
    let num = |j| {
        c.get(j)
            .map_or(Some(0.0), |m| m.as_str().parse::<f64>().ok())
    };
    let ms = c.get(i + 3).map_or(Some(0.0), |m| {
        m.as_str()
            .parse::<f64>()
            .ok()
            .map(|n| n * 10f64.powi(3 - m.as_str().len() as i32))
    })?;
    let seconds = num(i)? * 3600.0 + num(i + 1)? * 60.0 + num(i + 2)? + ms / 1000.0;
    seconds.is_finite().then_some(seconds)
}
pub fn parse_timestamp(value: &str) -> Option<f64> {
    patterns().time.captures(value).and_then(|c| seconds(&c, 1))
}
fn decode_entities(text: &str) -> String {
    patterns()
        .entities
        .replace_all(text, |c: &regex::Captures<'_>| {
            let name = &c[1];
            if let Some(number) = name.strip_prefix('#') {
                let code = if number.starts_with(['x', 'X']) {
                    u32::from_str_radix(&number[1..], 16).ok()
                } else {
                    number.parse().ok()
                };
                code.filter(|n| *n > 0)
                    .and_then(char::from_u32)
                    .map(|c| c.to_string())
                    .unwrap_or_default()
            } else {
                match name.to_ascii_lowercase().as_str() {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "nbsp" => " ",
                    "quot" => "\"",
                    "apos" => "'",
                    "lrm" => "\u{200e}",
                    "rlm" => "\u{200f}",
                    _ => c.get(0).unwrap().as_str(),
                }
                .to_owned()
            }
        })
        .into_owned()
}
pub fn strip_markup(text: &str) -> String {
    let p = patterns();
    let no_ruby = p.ruby.replace_all(text, "");
    let tags = p.tags.replace_all(&no_ruby, "");
    let classes = p.classes.replace_all(&tags, "");
    decode_entities(&p.ass.replace_all(&classes, ""))
}
fn blocks(text: &str) -> Vec<Vec<String>> {
    let text = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut out = Vec::new();
    let mut current = Vec::new();
    for line in text.split('\n') {
        if line.trim().is_empty() {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
        } else {
            current.push(line.into());
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}
fn inline_words(raw: &str, start: f64, end: f64) -> Option<Vec<CaptionWord>> {
    let p = patterns();
    if !p.inline.is_match(raw) {
        return None;
    }
    let mut parts = Vec::new();
    let mut last = 0;
    let mut at = start;
    for m in p.inline.captures_iter(raw) {
        let whole = m.get(0).unwrap();
        parts.push((at, &raw[last..whole.start()]));
        at = parse_timestamp(&m[1]).unwrap_or(at);
        last = whole.end();
    }
    parts.push((at, &raw[last..]));
    let mut words: Vec<_> = parts
        .into_iter()
        .flat_map(|(at, text)| {
            strip_markup(text)
                .split_whitespace()
                .map(|t| CaptionWord {
                    text: t.into(),
                    start: at,
                    end: at,
                })
                .collect::<Vec<_>>()
        })
        .collect();
    for i in 0..words.len() {
        words[i].end = words[i].start.max(
            end.min(
                words[i + 1..]
                    .iter()
                    .find(|w| w.start > words[i].start)
                    .map_or(end, |w| w.start),
            ),
        );
    }
    normalize_words(&serde_json::to_value(words).unwrap(), start, end)
}
fn read_cue(block: &[String], vtt: bool) -> Option<serde_json::Value> {
    let i = block.iter().position(|s| patterns().timing.is_match(s))?;
    if i > 1 {
        return None;
    }
    let c = patterns().timing.captures(&block[i])?;
    let (start, end) = (seconds(&c, 1)?, seconds(&c, 5)?);
    let raw = block[i + 1..].join("\n");
    let text = strip_markup(&patterns().inline.replace_all(&raw, ""))
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        return None;
    }
    let mut cue = serde_json::json!({"start":start,"end":end,"text":text});
    if vtt && let Some(words) = inline_words(&raw, start, end) {
        cue["words"] = serde_json::to_value(words).unwrap();
    }
    Some(cue)
}
pub fn detect_caption_format(text: &str, name: &str) -> CaptionFormat {
    let name = name.to_ascii_lowercase();
    if name.ends_with(".vtt") {
        return CaptionFormat::Vtt;
    }
    if name.ends_with(".srt") {
        return CaptionFormat::Srt;
    }
    if text
        .trim_start()
        .trim_start_matches('\u{feff}')
        .starts_with("WEBVTT")
    {
        CaptionFormat::Vtt
    } else {
        CaptionFormat::Srt
    }
}
pub fn parse_srt(text: &str) -> ParsedCaptions {
    parse_with_ids(text, CaptionFormat::Srt, &mut local_ids())
}
pub fn parse_vtt(text: &str) -> ParsedCaptions {
    parse_with_ids(text, CaptionFormat::Vtt, &mut local_ids())
}
fn local_ids() -> impl FnMut() -> String {
    let mut n = 0;
    move || {
        n += 1;
        format!("c{n}")
    }
}
/// `None` selects auto detection; default parser IDs are local to this import.
/// Inserting into an existing layer must use its ID allocator (`parse_with_ids`
/// or `insert_cues`), as IDs are only unique within a layer.
pub fn parse_captions(text: &str, format: Option<CaptionFormat>, name: &str) -> ParsedCaptions {
    let resolved = format.unwrap_or_else(|| detect_caption_format(text, name));
    let parsed = parse_with_ids(text, resolved, &mut local_ids());
    if parsed.cues.is_empty() && format.is_none() {
        let other = match resolved {
            CaptionFormat::Srt => CaptionFormat::Vtt,
            CaptionFormat::Vtt => CaptionFormat::Srt,
        };
        let other = parse_with_ids(text, other, &mut local_ids());
        if !other.cues.is_empty() {
            return other;
        }
    }
    parsed
}
pub fn parse_with_ids(
    text: &str,
    format: CaptionFormat,
    make_id: &mut impl FnMut() -> String,
) -> ParsedCaptions {
    let mut cues = Vec::new();
    let mut skipped = 0;
    for (i, mut block) in blocks(text).into_iter().enumerate() {
        let head = block[0].trim();
        if format == CaptionFormat::Vtt {
            if i == 0
                && head.starts_with("WEBVTT")
                && head[6..]
                    .chars()
                    .next()
                    .is_none_or(|c| !c.is_alphanumeric() && c != '_')
            {
                if block.iter().any(|s| patterns().timing.is_match(s)) {
                    block.remove(0);
                    if let Some(c) = read_cue(&block, true) {
                        cues.push(c);
                    }
                }
                continue;
            }
            if ["NOTE", "STYLE", "REGION"].iter().any(|k| {
                head.strip_prefix(k)
                    .is_some_and(|s| s.is_empty() || s.starts_with(char::is_whitespace))
            }) {
                continue;
            }
        }
        if let Some(c) = read_cue(&block, format == CaptionFormat::Vtt) {
            cues.push(c);
        } else {
            skipped += 1;
        }
    }
    ParsedCaptions {
        format,
        cues: normalize_cues(&serde_json::Value::Array(cues), make_id),
        skipped,
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WriteOptions {
    #[serde(default)]
    pub offset: f64,
    #[serde(default)]
    pub from: Option<f64>,
    #[serde(default)]
    pub to: Option<f64>,
    #[serde(default)]
    pub rebase: f64,
    #[serde(default)]
    pub language: Option<String>,
}
fn stamp(value: f64, separator: char) -> String {
    let total = (value * 1000.0 + 0.5).floor().max(0.0) as u64;
    format!(
        "{:02}:{:02}:{:02}{}{:03}",
        total / 3600000,
        total / 60000 % 60,
        total / 1000 % 60,
        separator,
        total % 1000
    )
}
fn timed<'a>(cues: &'a [CaptionCue], o: &WriteOptions) -> Vec<(&'a str, f64, f64)> {
    if !o.offset.is_finite() || !o.rebase.is_finite() {
        return Vec::new();
    }
    cues.iter()
        .filter_map(|c| {
            if !c.start.is_finite() || !c.end.is_finite() {
                return None;
            }
            let start = (c.start + o.offset).max(o.from.unwrap_or(f64::NEG_INFINITY));
            let end = (c.end + o.offset).min(o.to.unwrap_or(f64::INFINITY));
            if end - start < MIN_CUE_DURATION / 2.0 {
                return None;
            }
            // Clip against the nonnegative sidecar timeline after rebasing.
            // A cue wholly before zero must not become an empty/invented cue.
            let start = (start - o.rebase).max(0.0);
            let end = end - o.rebase;
            if end - start < MIN_CUE_DURATION / 2.0 {
                return None;
            }
            let start = (start * 1000.0 + 0.5).floor() / 1000.0;
            let end = (start + 0.001).max((end * 1000.0 + 0.5).floor() / 1000.0);
            Some((c.text.as_str(), start, end))
        })
        .collect()
}
fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
pub fn format_srt(cues: &[CaptionCue], options: &WriteOptions) -> String {
    timed(cues, options)
        .iter()
        .enumerate()
        .map(|(i, (text, start, end))| {
            format!(
                "{}\n{} --> {}\n{}\n",
                i + 1,
                stamp(*start, ','),
                stamp(*end, ','),
                escaped(text)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
pub fn format_vtt(cues: &[CaptionCue], options: &WriteOptions) -> String {
    let header = options.language.as_ref().map_or_else(
        || "WEBVTT\n".to_string(),
        |s| format!("WEBVTT\nLanguage: {}\n", s.replace(['\r', '\n'], "")),
    );
    let mut blocks = vec![header];
    blocks.extend(timed(cues, options).iter().map(|(text, start, end)| {
        format!(
            "{} --> {}\n{}\n",
            stamp(*start, '.'),
            stamp(*end, '.'),
            escaped(text)
        )
    }));
    blocks.join("\n")
}
pub fn format_captions(
    format: CaptionFormat,
    cues: &[CaptionCue],
    options: &WriteOptions,
) -> String {
    match format {
        CaptionFormat::Srt => format_srt(cues, options),
        CaptionFormat::Vtt => format_vtt(cues, options),
    }
}
