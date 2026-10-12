//! Run segmentation ported from Blink: `SymbolsIterator` (emoji presentation
//! scanner from `third_party/emoji-segmenter`), `ScriptRunIterator`, and
//! `RunSegmenter` (horizontal text only).

use super::chars::{self, EmojiCat};
use icu_properties::props::{BidiPairedBracketType, EastAsianWidth, Script};

/// Decodes one code point at `i` like ICU's `U16_NEXT` (a lone surrogate is
/// returned as itself).
pub fn next_cp(s: &[u16], i: usize) -> (u32, usize) {
    let u = s[i] as u32;
    if (0xD800..0xDC00).contains(&u) && i + 1 < s.len() {
        let v = s[i + 1] as u32;
        if (0xDC00..0xE000).contains(&v) {
            return (0x10000 + ((u - 0xD800) << 10) + (v - 0xDC00), 2);
        }
    }
    (u, 1)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FallbackPriority {
    Text,
    EmojiTextWithVs,
    EmojiEmoji,
    EmojiEmojiWithVs,
}

impl FallbackPriority {
    pub fn is_emoji_presentation(self) -> bool {
        matches!(self, Self::EmojiEmoji | Self::EmojiEmojiWithVs)
    }
    pub fn is_non_text(self) -> bool {
        !matches!(self, Self::Text)
    }
}

// ---------------------------------------------------------------- emoji scanner

fn cats(s: &[u16]) -> (Vec<EmojiCat>, Vec<usize>) {
    let mut cats = Vec::new();
    let mut offsets = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let (c, n) = next_cp(s, i);
        cats.push(chars::emoji_category(c));
        offsets.push(i);
        i += n;
    }
    offsets.push(s.len());
    (cats, offsets)
}

fn any_emoji(c: EmojiCat) -> bool {
    matches!(
        c,
        EmojiCat::EmojiTextPresentation
            | EmojiCat::EmojiEmojiPresentation
            | EmojiCat::KeycapBase
            | EmojiCat::EmojiModifierBase
            | EmojiCat::TagBase
            | EmojiCat::Emoji
    )
}

/// Longest match of each scanner token at code point index `p`; returns
/// (length in code points, is_emoji, has_vs). Ragel's scanner takes the
/// longest match and breaks ties by token order.
fn scan(c: &[EmojiCat], p: usize) -> (usize, bool, bool) {
    let at = |i: usize| c.get(i).copied();
    let is = |i: usize, k: EmojiCat| at(i) == Some(k);
    let ae = |i: usize| at(i).is_some_and(any_emoji);

    // text_emoji_run_with_vs
    let mut l1 = 0;
    if ae(p) && is(p + 1, EmojiCat::Vs15) {
        l1 = 2;
    }
    if is(p, EmojiCat::KeycapBase) && is(p + 1, EmojiCat::Vs15) && is(p + 2, EmojiCat::CombiningEnclosingKeycap) {
        l1 = 3;
    }
    // emoji_run_with_vs
    let mut l2 = 0;
    if ae(p) && is(p + 1, EmojiCat::Vs16) {
        l2 = 2;
    }
    if is(p, EmojiCat::KeycapBase) && is(p + 1, EmojiCat::Vs16) && is(p + 2, EmojiCat::CombiningEnclosingKeycap) {
        l2 = 3;
    }
    // emoji_run = emoji_presentation
    let elem_lengths = |q: usize| -> Vec<usize> {
        let mut v = Vec::new();
        if ae(q) {
            v.push(1);
            if is(q + 1, EmojiCat::Vs16) {
                v.push(2);
            }
        }
        if is(q, EmojiCat::EmojiModifierBase) && is(q + 1, EmojiCat::EmojiModifier) {
            v.push(2);
        }
        v
    };
    let mut l3 = 0;
    if matches!(
        at(p),
        // emoji_presentation's single code points; a lone modifier or
        // regional indicator is a text_run.
        Some(EmojiCat::EmojiEmojiPresentation | EmojiCat::TagBase | EmojiCat::EmojiModifierBase)
    ) {
        l3 = 1;
    }
    if ae(p) && is(p + 1, EmojiCat::Vs16) {
        l3 = l3.max(2);
    }
    if is(p, EmojiCat::EmojiModifierBase) && is(p + 1, EmojiCat::EmojiModifier) {
        l3 = l3.max(2);
    }
    if is(p, EmojiCat::RegionalIndicator) && is(p + 1, EmojiCat::RegionalIndicator) {
        l3 = l3.max(2);
    }
    if is(p, EmojiCat::TagBase) && is(p + 1, EmojiCat::TagSequence) {
        let mut q = p + 1;
        while is(q, EmojiCat::TagSequence) {
            q += 1;
        }
        if is(q, EmojiCat::TagTerm) {
            l3 = l3.max(q + 1 - p);
        }
    }
    if is(p, EmojiCat::KeycapBase) && is(p + 1, EmojiCat::Vs16) && is(p + 2, EmojiCat::CombiningEnclosingKeycap) {
        l3 = l3.max(3);
    }
    if ae(p) && is(p + 1, EmojiCat::CombiningEnclosingCircleBackslash) {
        l3 = l3.max(2);
    }
    // emoji_zwj_sequence = elem (ZWJ elem)+
    let mut frontier: Vec<usize> = elem_lengths(p).into_iter().map(|n| p + n).collect();
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for q in frontier {
            if is(q, EmojiCat::Zwj) {
                for n in elem_lengths(q + 1) {
                    let r = q + 1 + n;
                    l3 = l3.max(r - p);
                    if !next.contains(&r) {
                        next.push(r);
                    }
                }
            }
        }
        frontier = next;
    }
    // unqualified_keycap_sequence
    let l4 = if is(p, EmojiCat::KeycapBase) && is(p + 1, EmojiCat::CombiningEnclosingKeycap) { 2 } else { 0 };

    // Longest match wins; ties go to the token listed first in the grammar.
    let candidates = [(l1, false, true), (l2, true, true), (l3, true, false), (l4, true, false), (1, false, false)];
    let max = candidates.iter().map(|c| c.0).max().unwrap_or(1);
    *candidates.iter().find(|c| c.0 == max).expect("text_run always matches")
}

/// Emoji presentation segments in UTF-16 offsets, following
/// `SymbolsIterator::Consume` (including its single-character shortcut for
/// characters that cannot start an emoji sequence after a text token).
pub fn symbol_runs(s: &[u16]) -> Vec<(usize, FallbackPriority)> {
    let (c, off) = cats(s);
    let n = c.len();
    let mut out = Vec::new();
    if n == 0 {
        return out;
    }
    // Positions are in code point indices; converted to UTF-16 at the end.
    let mut it = 0usize; // buffer_iterator_
    let first = scan(&c, 0);
    let mut next_end = first.0;
    let mut next_emoji = first.1;
    let mut next_vs = first.2;
    let mut cursor = 0usize;
    while cursor < n {
        let mut cur_emoji;
        let mut cur_vs;
        loop {
            cursor = next_end;
            cur_emoji = next_emoji;
            cur_vs = next_vs;
            if cursor >= n {
                break;
            }
            // UTF16RagelIterator::PeekCodepoint: the code point after the
            // iterator's position, U+FFFD past the end.
            let peek = if it + 1 < n { next_cp(s, off[it + 1]).0 } else { 0xFFFD };
            if !cur_emoji && !chars::maybe_emoji_presentation(peek) {
                it += 1;
                next_end = it;
                next_vs = false;
            } else {
                it = cursor;
                let t = scan(&c, cursor);
                next_end = cursor + t.0;
                next_emoji = t.1;
                next_vs = t.2;
            }
            if !(cur_emoji == next_emoji && cur_vs == next_vs) {
                break;
            }
        }
        let priority = match (cur_vs, cur_emoji) {
            (true, true) => FallbackPriority::EmojiEmojiWithVs,
            (true, false) => FallbackPriority::EmojiTextWithVs,
            (false, true) => FallbackPriority::EmojiEmoji,
            (false, false) => FallbackPriority::Text,
        };
        out.push((off[cursor.min(n)], priority));
    }
    out
}

// ---------------------------------------------------------------- script runs

const MAX_SCRIPT_COUNT: usize = 32;
const MAX_BRACKETS: usize = 32;

fn script_for_opentype(ch: u32) -> Script {
    let s = chars::script(ch);
    if s == Script::Katakana || s == Script::KatakanaOrHiragana { Script::Hiragana } else { s }
}

fn is_han_script(s: Script) -> bool {
    s == Script::Han || s == Script::Hiragana || s == Script::Bopomofo
}

fn get_scripts(ch: u32, dst: &mut Vec<Script>) {
    dst.clear();
    let primary = script_for_opentype(ch);
    if primary == Script::Hiragana {
        dst.push(primary);
        return;
    }
    let mut ext = chars::script_extensions(ch);
    ext.truncate(MAX_SCRIPT_COUNT - 1);
    *dst = ext;
    if dst.is_empty() {
        return;
    }
    if primary == dst[0] {
        return;
    }
    if primary != Script::Inherited && primary != Script::Common && primary != Script::Unknown {
        if let Some(pos) = dst.iter().skip(1).position(|&s| s == primary) {
            dst.swap(0, pos + 1);
        } else {
            dst.push(primary);
            let last = dst.len() - 1;
            dst.swap(0, last);
        }
        return;
    }
    if primary == Script::Common {
        if dst.len() == 1 {
            dst.insert(0, primary);
            return;
        }
        for i in 1..dst.len() {
            if dst[0] == Script::Latin || code(dst[i]) < code(dst[0]) {
                dst.swap(0, i);
            }
        }
        return;
    }
    let first = dst[0];
    dst.push(first);
    dst[0] = primary;
    for i in 2..dst.len() {
        if dst[1] == Script::Latin || code(dst[i]) < code(dst[1]) {
            dst.swap(1, i);
        }
    }
}

#[allow(deprecated)]
fn code(s: Script) -> u16 {
    s.to_icu4c_value()
}

fn le_inherited(s: Script) -> bool {
    code(s) <= code(Script::Inherited)
}

struct BracketRec {
    ch: u32,
    script: Script,
}

struct ScriptRunIterator<'a> {
    text: &'a [u16],
    current: Vec<Script>,
    next: Vec<Script>,
    ahead: Vec<Script>,
    ahead_char: u32,
    ahead_pos: usize,
    common_preferred: Script,
    brackets: Vec<BracketRec>,
    fixup_depth: usize,
    han_extensions: Vec<Script>,
}

impl<'a> ScriptRunIterator<'a> {
    fn new(text: &'a [u16]) -> Self {
        let mut it = ScriptRunIterator {
            text,
            current: Vec::new(),
            next: Vec::new(),
            ahead: Vec::new(),
            ahead_char: 0,
            ahead_pos: 0,
            common_preferred: Script::Common,
            brackets: Vec::new(),
            fixup_depth: 0,
            han_extensions: chars::script_extensions(0x300C),
        };
        if !text.is_empty() {
            it.current = vec![Script::Common];
            let (c, n) = next_cp(text, 0);
            it.ahead_char = c;
            it.ahead_pos = n;
            get_scripts(c, &mut it.ahead);
        }
        it
    }

    fn fetch(&mut self) -> Option<(usize, u32)> {
        let len = self.text.len();
        if self.ahead_pos > len {
            return None;
        }
        let pos = self.ahead_pos - if self.ahead_char >= 0x10000 { 2 } else { 1 };
        let ch = self.ahead_char;
        std::mem::swap(&mut self.next, &mut self.ahead);
        if self.ahead_pos == len {
            self.ahead_pos += 1;
            return Some((pos, ch));
        }
        if !self.fetch_next_character() {
            return None;
        }
        Some((pos, ch))
    }

    fn fetch_next_character(&mut self) -> bool {
        let (c, n) = next_cp(self.text, self.ahead_pos);
        self.ahead_pos += n;
        self.ahead_char = c;
        get_scripts(c, &mut self.ahead);
        if self.ahead.is_empty() {
            return false;
        }
        if self.ahead[0] == Script::Inherited && self.ahead.len() > 1 {
            if self.next.first() == Some(&Script::Common) {
                self.next = self.ahead.clone();
                self.next.remove(0);
                self.ahead.truncate(1);
            } else {
                self.ahead.truncate(1);
            }
        }
        true
    }

    fn fix_scripts_by_east_asian_width(&self, ch: u32, set: &mut Vec<Script>) {
        if set.len() > 1 || set.first() != Some(&Script::Common) {
            return;
        }
        let eaw = chars::east_asian_width(ch);
        if eaw == EastAsianWidth::Wide || eaw == EastAsianWidth::Fullwidth || eaw == EastAsianWidth::Halfwidth {
            *set = self.han_extensions.clone();
        }
    }

    fn open_bracket(&mut self, ch: u32) {
        if self.brackets.len() == MAX_BRACKETS {
            self.brackets.remove(0);
            if self.fixup_depth == MAX_BRACKETS {
                self.fixup_depth -= 1;
            }
        }
        let mut next = std::mem::take(&mut self.next);
        self.fix_scripts_by_east_asian_width(ch, &mut next);
        self.next = next;
        self.brackets.push(BracketRec { ch, script: Script::Common });
        self.fixup_depth += 1;
    }

    fn close_bracket(&mut self, ch: u32) {
        if self.brackets.is_empty() {
            return;
        }
        let target = chars::paired_bracket(ch);
        for (depth, idx) in (0..self.brackets.len()).rev().enumerate() {
            if self.brackets[idx].ch == target {
                let mut script = self.brackets[idx].script;
                if is_han_script(script)
                    && let Some(&h) = self.current.iter().find(|&&s| is_han_script(s)) {
                        script = h;
                    }
                if script != Script::Common {
                    self.next = vec![script];
                }
                let num_popped = depth;
                for _ in 0..num_popped {
                    self.brackets.pop();
                }
                self.fixup_depth = self.fixup_depth.saturating_sub(num_popped);
                return;
            }
        }
    }

    fn merge_sets(&mut self) -> bool {
        if self.next.is_empty() || self.current.is_empty() {
            return false;
        }
        let mut priority = self.current[0];
        let rest: Vec<Script> = self.current[1..].to_vec();
        if le_inherited(self.next[0]) {
            if self.next.len() == 2 && le_inherited(priority) && self.common_preferred == Script::Common {
                self.common_preferred = self.next[1];
            }
            return true;
        }
        if le_inherited(priority) {
            self.current = self.next.clone();
            return true;
        }
        let mut have_priority = self.next.contains(&priority);
        if rest.is_empty() {
            return have_priority;
        }
        let mut next_slice: &[Script] = &self.next;
        if !have_priority {
            priority = next_slice[0];
            next_slice = &next_slice[1..];
            have_priority = rest.contains(&priority);
        }
        let mut written = Vec::new();
        if have_priority {
            written.push(priority);
        }
        if !next_slice.is_empty() {
            for &sc in &rest {
                if next_slice.contains(&sc) {
                    written.push(sc);
                }
            }
        }
        if !written.is_empty() {
            self.current = written;
            true
        } else {
            false
        }
    }

    fn fixup_stack(&mut self, resolved: Script, exclude_last: bool) {
        let mut count = self.fixup_depth;
        if count == 0 {
            return;
        }
        count = count.min(self.brackets.len());
        let mut idx = self.brackets.len();
        if exclude_last {
            idx -= 1;
            count -= 1;
            self.fixup_depth = 1;
        } else {
            self.fixup_depth = 0;
        }
        while count > 0 {
            idx -= 1;
            self.brackets[idx].script = resolved;
            count -= 1;
        }
    }

    fn resolve(&self) -> Script {
        let r = self.current[0];
        if r == Script::Common { self.common_preferred } else { r }
    }

    fn consume(&mut self) -> Option<(usize, Script)> {
        if self.current.is_empty() {
            return None;
        }
        while let Some((pos, ch)) = self.fetch() {
            let paired = chars::paired_bracket_type(ch);
            match paired {
                BidiPairedBracketType::Open => self.open_bracket(ch),
                BidiPairedBracketType::Close => self.close_bracket(ch),
                _ => {}
            }
            if !self.merge_sets() {
                let script = self.resolve();
                self.fixup_stack(script, paired == BidiPairedBracketType::Open);
                self.current = self.next.clone();
                return Some((pos, script));
            }
        }
        let script = self.resolve();
        self.current.clear();
        Some((self.text.len(), script))
    }
}

/// Script runs as (end offset, script).
pub fn script_runs(s: &[u16]) -> Vec<(usize, Script)> {
    let mut it = ScriptRunIterator::new(s);
    let mut out = Vec::new();
    while let Some(r) = it.consume() {
        out.push(r);
    }
    out
}

#[derive(Clone, Copy, Debug)]
pub struct Segment {
    pub start: usize,
    pub end: usize,
    pub script: Script,
    pub priority: FallbackPriority,
}

/// `RunSegmenter` for horizontal text: the intersection of script runs and
/// emoji presentation runs.
pub fn run_segments(s: &[u16]) -> Vec<Segment> {
    let scripts = script_runs(s);
    let symbols = symbol_runs(s);
    let mut out = Vec::new();
    if s.is_empty() {
        return out;
    }
    let (mut si, mut yi) = (0usize, 0usize);
    let mut start = 0usize;
    while start < s.len() {
        while si < scripts.len() && scripts[si].0 <= start {
            si += 1;
        }
        while yi < symbols.len() && symbols[yi].0 <= start {
            yi += 1;
        }
        let (send, script) = scripts.get(si).copied().unwrap_or((s.len(), Script::Common));
        let (yend, priority) = symbols.get(yi).copied().unwrap_or((s.len(), FallbackPriority::Text));
        let end = send.min(yend);
        out.push(Segment { start, end, script, priority });
        start = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn latin_is_one_run() {
        let r = script_runs(&u("Hello world"));
        assert_eq!(r, vec![(11, Script::Latin)]);
    }

    #[test]
    fn mixed_scripts_split() {
        let r = script_runs(&u("abc 编辑 def"));
        assert_eq!(r.len(), 3);
        assert_eq!(r[1].1, Script::Han);
    }

    #[test]
    fn emoji_segments() {
        let s = u("Hi 👋🏽 x");
        let runs = symbol_runs(&s);
        let kinds: Vec<_> = runs.iter().map(|r| r.1).collect();
        assert_eq!(kinds, vec![FallbackPriority::Text, FallbackPriority::EmojiEmoji, FallbackPriority::Text]);
        assert_eq!(runs[0].0, 3);
        assert_eq!(runs[1].0, 7);
    }

    #[test]
    fn keycap_and_flag() {
        let s = u("1\u{FE0F}\u{20E3}🇰🇷");
        let runs = symbol_runs(&s);
        assert_eq!(runs[0], (3, FallbackPriority::EmojiEmojiWithVs));
        assert_eq!(runs[1], (7, FallbackPriority::EmojiEmoji));
    }
}
