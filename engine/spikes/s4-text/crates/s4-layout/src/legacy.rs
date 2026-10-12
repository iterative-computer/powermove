//! Level 1 compatibility layout: a line-for-line port of today's
//! `legacy/core/text-layout.ts` (`textSourceLines`, `glyphLayout`) and the
//! caret boundaries of `legacy/gl/raster.ts` (`textCaretLayout`), including
//! JavaScript's UTF-16 string semantics. Widths come from a `Measure`
//! implementation that reproduces `CanvasRenderingContext2D.measureText`.

use crate::blink::chars::is_js_space;
use serde::{Deserialize, Serialize};

/// The measuring context: `measureText(text).width` for UTF-16 text.
pub trait Measure {
    fn width(&mut self, text: &[u16]) -> f64;
}

/// The text layer content fields the layout reads (`layer.d` after
/// `resolvedTextContent`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TextContent {
    pub text: String,
    #[serde(default)]
    pub font: Option<String>,
    #[serde(default)]
    pub weight: Option<f64>,
    #[serde(default)]
    pub size: Option<f64>,
    #[serde(default)]
    pub tracking: Option<f64>,
    #[serde(default)]
    pub leading: Option<f64>,
    #[serde(default)]
    pub align: Option<String>,
    #[serde(default)]
    pub italic: Option<bool>,
    #[serde(rename = "boxWidth", default)]
    pub box_width: Option<f64>,
    #[serde(rename = "boxHeight", default)]
    pub box_height: Option<f64>,
    #[serde(default)]
    pub paragraph: Option<bool>,
    /// `fontAxis.<tag>` entries, sorted by tag as `variationEntries` does.
    #[serde(skip)]
    pub axes: Vec<(String, f64)>,
}

impl TextContent {
    /// `Math.max(1, Number(d.size) || 16)`.
    pub fn size(&self) -> f64 {
        let s = self.size.filter(|s| *s != 0.0 && !s.is_nan()).unwrap_or(16.0);
        s.max(1.0)
    }
    /// `size * (d.leading || 1.15)`.
    pub fn line_height(&self) -> f64 {
        let l = self.leading.filter(|l| *l != 0.0 && !l.is_nan()).unwrap_or(1.15);
        self.size() * l
    }
    pub fn align(&self) -> Align {
        match self.align.as_deref() {
            Some("center") => Align::Center,
            Some("right") => Align::Right,
            _ => Align::Left,
        }
    }
    pub fn utf16(&self) -> Vec<u16> {
        self.text.encode_utf16().collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLine {
    pub text: Vec<u16>,
    pub start: usize,
}

fn trim_end(s: &[u16]) -> &[u16] {
    let mut end = s.len();
    while end > 0 && is_js_space(s[end - 1]) {
        end -= 1;
    }
    &s[..end]
}

fn trim_start(s: &[u16]) -> &[u16] {
    let mut start = 0;
    while start < s.len() && is_js_space(s[start]) {
        start += 1;
    }
    &s[start..]
}

/// `paragraph.split(/(\s+)/u).filter(Boolean)`: alternating runs of
/// non-space and space. Every JS `\s` character is in the BMP, so code-unit
/// classification equals code-point classification.
fn tokens(s: &[u16]) -> Vec<&[u16]> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < s.len() {
        let space = is_js_space(s[start]);
        let mut end = start + 1;
        while end < s.len() && is_js_space(s[end]) == space {
            end += 1;
        }
        out.push(&s[start..end]);
        start = end;
    }
    out
}

fn concat(a: &[u16], b: &[u16]) -> Vec<u16> {
    let mut v = Vec::with_capacity(a.len() + b.len());
    v.extend_from_slice(a);
    v.extend_from_slice(b);
    v
}

/// `textSourceLines(d, context, lineHeight)`.
pub fn text_source_lines(d: &TextContent, m: &mut dyn Measure, line_height: f64) -> Vec<SourceLine> {
    let text = d.utf16();
    let paragraphs: Vec<&[u16]> = text.split(|&u| u == 0x0A).collect();
    let box_width = d.box_width.unwrap_or(f64::NAN);
    let mut lines = Vec::new();
    let mut offset = 0usize;
    if !d.paragraph.unwrap_or(false) || !box_width.is_finite() || box_width <= 0.0 {
        for p in paragraphs {
            lines.push(SourceLine { text: p.to_vec(), start: offset });
            offset += p.len() + 1;
        }
        return lines;
    }
    for p in paragraphs {
        let paragraph_start = offset;
        offset += p.len() + 1;
        if p.is_empty() {
            lines.push(SourceLine { text: Vec::new(), start: paragraph_start });
            continue;
        }
        let mut line: Vec<u16> = Vec::new();
        let mut line_start = paragraph_start;
        let mut token_start = paragraph_start;
        for token in tokens(p) {
            let candidate = concat(&line, token);
            if !line.is_empty() && m.width(&candidate) > box_width {
                lines.push(SourceLine { text: trim_end(&line).to_vec(), start: line_start });
                line = trim_start(token).to_vec();
                line_start = token_start + (token.len() - line.len());
            } else {
                if line.is_empty() {
                    line_start = token_start;
                }
                line = candidate;
            }
            token_start += token.len();
            while !line.is_empty() && m.width(&line) > box_width {
                let mut cut = 1;
                while cut < line.len() && m.width(&line[..cut + 1]) <= box_width {
                    cut += 1;
                }
                lines.push(SourceLine { text: line[..cut].to_vec(), start: line_start });
                line = line[cut..].to_vec();
                line_start += cut;
            }
        }
        lines.push(SourceLine { text: trim_end(&line).to_vec(), start: line_start });
    }
    let box_height = d.box_height.unwrap_or(f64::NAN);
    if !box_height.is_finite() || box_height <= 0.0 {
        return lines;
    }
    let keep = ((box_height / line_height.max(1.0)).floor()).max(1.0) as usize;
    lines.truncate(keep);
    lines
}

/// Grapheme clusters as `Intl.Segmenter(undefined, {granularity: 'grapheme'})`
/// yields them, as UTF-16 ranges.
pub fn graphemes(s: &[u16]) -> Vec<std::ops::Range<usize>> {
    let seg = icu_segmenter::GraphemeClusterSegmenter::new();
    let breaks: Vec<usize> = seg.segment_utf16(s).collect();
    breaks.windows(2).map(|w| w[0]..w[1]).collect()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Unit {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub line: usize,
    #[serde(rename = "sourceStart")]
    pub source_start: usize,
    pub index: usize,
    #[serde(rename = "lineUnit", default, skip_serializing_if = "Option::is_none")]
    pub line_unit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word: Option<usize>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GlyphLayout {
    pub characters: Vec<Unit>,
    pub words: Vec<Unit>,
    pub lines: Vec<Unit>,
}

fn js_string(s: &[u16]) -> String {
    String::from_utf16_lossy(s)
}

fn index_of(hay: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    if from > hay.len() {
        return if needle.is_empty() { Some(hay.len()) } else { None };
    }
    if needle.is_empty() {
        return Some(from);
    }
    (from..=hay.len().saturating_sub(needle.len())).find(|&i| hay[i..].starts_with(needle))
}

fn is_all_space(s: &[u16]) -> bool {
    !s.is_empty() && s.iter().all(|&u| is_js_space(u))
}

/// `glyphLayout(d, meas)`.
pub fn glyph_layout(d: &TextContent, m: &mut dyn Measure) -> GlyphLayout {
    let lh = d.line_height();
    let align = d.align();
    let lines: Vec<Vec<u16>> = text_source_lines(d, m, lh).into_iter().map(|l| l.text).collect();
    let source = d.utf16();
    let mut out = GlyphLayout::default();
    let (mut character_index, mut word_index, mut line_index, mut source_offset) = (0usize, 0usize, 0usize, 0usize);
    for (row, line) in lines.iter().enumerate() {
        let line_width = m.width(line);
        let start_x = match align {
            Align::Center => -line_width / 2.0,
            Align::Right => -line_width,
            Align::Left => 0.0,
        };
        let y = row as f64 * lh;
        // Math.max(sourceOffset, text.indexOf(line, sourceOffset)); indexOf
        // returns -1 when absent.
        let found = index_of(&source, line, source_offset).map(|i| i as i64).unwrap_or(-1);
        let line_source = (source_offset as i64).max(found) as usize;
        let line_unit = line_index;
        if !line.is_empty() {
            out.lines.push(Unit {
                text: js_string(line),
                x: start_x,
                y,
                w: line_width,
                line: row,
                source_start: line_source,
                index: line_index,
                line_unit: None,
                word: None,
            });
            line_index += 1;
        }
        let mut prefix: Vec<u16> = Vec::new();
        let mut local_word: i64 = -1;
        let mut was_space = true;
        for r in graphemes(line) {
            let segment = &line[r];
            let x = start_x + m.width(&concat(&prefix, segment)) - m.width(segment);
            let space = is_all_space(segment);
            if !space && was_space {
                local_word += 1;
            }
            if !space {
                out.characters.push(Unit {
                    text: js_string(segment),
                    x,
                    y,
                    w: m.width(segment),
                    line: row,
                    source_start: line_source + prefix.len(),
                    index: character_index,
                    line_unit: Some(line_unit),
                    word: Some((word_index as i64 + local_word) as usize),
                });
                character_index += 1;
            }
            was_space = space;
            prefix.extend_from_slice(segment);
        }
        source_offset = line_source + line.len();
        // /\S+/gu over code points; every \s is BMP so unit scanning matches.
        let mut i = 0;
        while i < line.len() {
            if is_js_space(line[i]) {
                i += 1;
                continue;
            }
            let start = i;
            while i < line.len() && !is_js_space(line[i]) {
                i += 1;
            }
            let word = &line[start..i];
            let pre = &line[..start];
            out.words.push(Unit {
                text: js_string(word),
                x: start_x + m.width(&concat(pre, word)) - m.width(word),
                y,
                w: m.width(word),
                line: row,
                source_start: line_source + start,
                index: word_index,
                line_unit: None,
                word: None,
            });
            word_index += 1;
        }
    }
    out
}

/// Caret boundaries of `textCaretLayout`: per line, `[sourceIndex, x]` with x
/// relative to the line's alignment start (`width(prefix)`).
pub fn caret_boundaries(d: &TextContent, m: &mut dyn Measure) -> Vec<Vec<(usize, f64)>> {
    let lh = d.line_height();
    text_source_lines(d, m, lh)
        .into_iter()
        .map(|line| {
            let mut out = vec![(line.start, 0.0)];
            let mut prefix_len = 0;
            for r in graphemes(&line.text) {
                prefix_len = r.end;
                out.push((line.start + prefix_len, m.width(&line.text[..prefix_len])));
            }
            let _ = prefix_len;
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monospace measurer: every UTF-16 unit is 10 px wide.
    struct Mono;
    impl Measure for Mono {
        fn width(&mut self, t: &[u16]) -> f64 {
            t.len() as f64 * 10.0
        }
    }

    fn d(text: &str, box_width: f64) -> TextContent {
        TextContent { text: text.into(), size: Some(10.0), box_width: Some(box_width), paragraph: Some(box_width > 0.0), ..Default::default() }
    }

    fn texts(lines: &[SourceLine]) -> Vec<String> {
        lines.iter().map(|l| String::from_utf16_lossy(&l.text)).collect()
    }

    #[test]
    fn no_box_splits_on_newlines() {
        let l = text_source_lines(&d("ab\n\ncd", 0.0), &mut Mono, 10.0);
        assert_eq!(texts(&l), ["ab", "", "cd"]);
        assert_eq!(l.iter().map(|l| l.start).collect::<Vec<_>>(), [0, 3, 4]);
    }

    #[test]
    fn wraps_at_spaces_and_trims() {
        let l = text_source_lines(&d("aaa bbb ccc", 65.0), &mut Mono, 10.0);
        assert_eq!(texts(&l), ["aaa", "bbb", "ccc"]);
        assert_eq!(l.iter().map(|l| l.start).collect::<Vec<_>>(), [0, 4, 8]);
    }

    #[test]
    fn breaks_long_words_per_code_unit() {
        let l = text_source_lines(&d("abcdefg", 30.0), &mut Mono, 10.0);
        assert_eq!(texts(&l), ["abc", "def", "g"]);
    }

    #[test]
    fn splits_surrogate_pairs_like_javascript() {
        // One emoji is two UTF-16 units; a 10 px box cuts between them.
        let l = text_source_lines(&d("😀", 10.0), &mut Mono, 10.0);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].text, vec![0xD83D]);
        assert_eq!(l[1].text, vec![0xDE00]);
    }

    #[test]
    fn box_height_clips_lines() {
        let mut c = d("a b c d", 15.0);
        c.box_height = Some(25.0);
        let l = text_source_lines(&c, &mut Mono, 10.0);
        assert_eq!(texts(&l), ["a", "b"]);
    }

    #[test]
    fn glyph_layout_positions() {
        let mut c = d("ab cd", 0.0);
        c.align = Some("center".into());
        let g = glyph_layout(&c, &mut Mono);
        assert_eq!(g.lines[0].x, -25.0);
        assert_eq!(g.characters.len(), 4);
        assert_eq!(g.characters[2].x, 5.0);
        assert_eq!(g.characters[2].word, Some(1));
        assert_eq!(g.words[1].x, 5.0);
    }
}
