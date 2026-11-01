//! Shipped cue text normalization and CJK token/timing rules.
use crate::{CaptionCue, CaptionWord, model::round};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) fn string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => ryu_js::Buffer::new()
            .format(n.as_f64().unwrap())
            .to_string(),
    }
}
pub(crate) fn js_space(c: char) -> bool {
    matches!(c,'\u{0009}'..='\u{000d}'|'\u{0020}'|'\u{00a0}'|'\u{1680}'|'\u{2000}'..='\u{200a}'|'\u{2028}'|'\u{2029}'|'\u{202f}'|'\u{205f}'|'\u{3000}'|'\u{feff}')
}
pub(crate) fn trim(s: &str) -> &str {
    s.trim_matches(js_space)
}
pub fn clean_cue_text(value: &Value) -> String {
    string(value)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .filter_map(|line| {
            let mut out = String::new();
            let mut space = false;
            for c in line.chars() {
                if c == ' ' || c == '\t' {
                    if !space {
                        out.push(' ');
                    }
                    space = true;
                } else {
                    out.push(c);
                    space = false;
                }
            }
            let out = trim(&out);
            (!out.is_empty()).then(|| out.to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
}
pub fn apply_text_case(text: &str, text_case: &str) -> String {
    match text_case {
        "upper" => text.to_uppercase(),
        "lower" => text.to_lowercase(),
        _ => text.to_owned(),
    }
}
pub fn is_cjk_char(c: char) -> bool {
    matches!(c as u32,0x2e80..=0x303f|0x3040..=0x30ff|0x31f0..=0x31ff|0x3400..=0x4dbf|0x4e00..=0x9fff|0xf900..=0xfaff|0xff00..=0xffef|0x20000..=0x2ffff)
}
const CLOSING: &str = "、。，．！？：；」』）】〕〉》〙〗〛…‥・ーぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮヵヶ々〻!?.,;:)]}%'\"”’";
const OPENING: &str = "「『（【〔〈《〘〖〚([{“‘";
pub fn join_words<'a>(texts: impl IntoIterator<Item = &'a str>) -> String {
    let mut out = String::new();
    for text in texts {
        let text = trim(text);
        if text.is_empty() {
            continue;
        }
        if !out.is_empty()
            && !out.chars().next_back().is_some_and(is_cjk_char)
            && !text.chars().next().is_some_and(is_cjk_char)
        {
            out.push(' ');
        }
        out.push_str(text);
    }
    out
}
pub fn display_length(text: &str) -> usize {
    text.chars()
        .map(|c| if is_cjk_char(c) { 2 } else { 1 })
        .sum()
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextToken {
    pub text: String,
    pub gap: bool,
}
pub fn text_tokens(paragraph: &str) -> Vec<TextToken> {
    let mut tokens = Vec::new();
    for chunk in paragraph.split(js_space).filter(|s| !s.is_empty()) {
        let mut current = String::new();
        let mut gap = !tokens.is_empty();
        let mut opening = false;
        for c in chunk.chars() {
            let last = current.chars().next_back();
            let cjk = is_cjk_char(c);
            if current.is_empty()
                || opening
                || (CLOSING.contains(c) && (cjk || last.is_some_and(is_cjk_char)))
            {
                current.push(c);
            } else if cjk || last.is_some_and(is_cjk_char) {
                tokens.push(TextToken {
                    text: std::mem::take(&mut current),
                    gap,
                });
                gap = false;
                current.push(c);
            } else {
                current.push(c);
            }
            opening = OPENING.contains(c);
        }
        if !current.is_empty() {
            tokens.push(TextToken { text: current, gap });
        }
    }
    tokens
}
pub fn cue_tokens(text: &str) -> Vec<TextToken> {
    text.split('\n').flat_map(text_tokens).collect()
}
pub fn cue_words(cue: &CaptionCue) -> Vec<CaptionWord> {
    let tokens: Vec<_> = cue_tokens(&cue.text).into_iter().map(|t| t.text).collect();
    if let Some(words) = cue.words.as_ref().filter(|w| !w.is_empty()) {
        if words.len() == tokens.len() {
            return words
                .iter()
                .zip(tokens)
                .map(|(w, text)| CaptionWord { text, ..w.clone() })
                .collect();
        }
        if let Some(mapped) = words_by_character(words, &tokens) {
            return mapped;
        }
    }
    let total = tokens
        .iter()
        .map(|t| t.encode_utf16().count() + 1)
        .sum::<usize>()
        .max(1) as f64;
    let span = cue.end - cue.start;
    let mut at = cue.start;
    tokens
        .into_iter()
        .map(|text| {
            let length = span * (text.encode_utf16().count() + 1) as f64 / total;
            let word = CaptionWord {
                text,
                start: round(at),
                end: round(at + length),
            };
            at += length;
            word
        })
        .collect()
}
fn squash(s: &str) -> Vec<char> {
    s.chars()
        .filter(|c| !js_space(*c))
        .flat_map(char::to_lowercase)
        .collect()
}
fn words_by_character(words: &[CaptionWord], tokens: &[String]) -> Option<Vec<CaptionWord>> {
    let source: Vec<_> = words.iter().flat_map(|w| squash(&w.text)).collect();
    let text: Vec<_> = tokens.iter().flat_map(|t| squash(t)).collect();
    if source != text {
        return None;
    }
    let mut owner = Vec::new();
    let mut shares = Vec::new();
    for (i, w) in words.iter().enumerate() {
        let n = squash(&w.text).len();
        owner.extend(std::iter::repeat_n(i, n));
        shares.push(n);
    }
    let mut seen = vec![0; words.len()];
    let mut position = 0;
    let mut out = Vec::new();
    for token in tokens {
        let mut start = f64::INFINITY;
        let mut end = f64::NEG_INFINITY;
        for _ in squash(token) {
            let i = owner[position];
            position += 1;
            let w = &words[i];
            let k = seen[i];
            seen[i] += 1;
            let step = (w.end - w.start) / shares[i] as f64;
            start = start.min(w.start + step * k as f64);
            end = end.max(w.start + step * (k + 1) as f64);
        }
        out.push(CaptionWord {
            text: token.clone(),
            start: round(start),
            end: round(end),
        });
    }
    Some(out)
}
