use core::borrow::Borrow;
use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};

/// A JavaScript string: a sequence of UTF-16 code units.
///
/// Well-formed strings fit inline when short, or in a boxed UTF-8 slice.
/// Strings holding lone surrogates, which JavaScript allows and `JSON.stringify`
/// writes as `\udXXX` escapes, are stored as WTF-8: UTF-8 generalized to encode
/// unpaired surrogate code points as three bytes. The two forms never overlap,
/// so equality, ordering, and hashing work on the bytes across representations.
///
/// `Ord` is byte order, which equals code point order. JavaScript's default
/// sort compares UTF-16 code units instead; use [`JsString::cmp_utf16`].
#[derive(Clone)]
pub struct JsString(Repr);

const INLINE_CAP: usize = 2 * core::mem::size_of::<usize>() - 1;

#[derive(Clone)]
enum Repr {
    Inline {
        len: u8,
        bytes: [u8; INLINE_CAP],
    },
    Utf8(Box<str>),
    /// Contains at least one unpaired surrogate; never valid UTF-8.
    Wtf8(Box<[u8]>),
}

impl JsString {
    pub fn new() -> JsString {
        JsString(Repr::Inline {
            len: 0,
            bytes: [0; INLINE_CAP],
        })
    }

    /// The text when it is well-formed (no lone surrogates).
    pub fn as_str(&self) -> Option<&str> {
        match &self.0 {
            Repr::Inline { len, bytes } => {
                // Every inline constructor copies from a valid UTF-8 string.
                Some(core::str::from_utf8(&bytes[..usize::from(*len)]).expect("inline UTF-8"))
            }
            Repr::Utf8(s) => Some(s),
            Repr::Wtf8(_) => None,
        }
    }

    /// The text with lone surrogates replaced by U+FFFD.
    pub fn to_string_lossy(&self) -> String {
        match &self.0 {
            Repr::Inline { .. } | Repr::Utf8(_) => self.as_str().unwrap().to_owned(),
            Repr::Wtf8(_) => {
                let mut out = String::new();
                for cp in self.code_points() {
                    out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                }
                out
            }
        }
    }

    /// WTF-8 bytes (UTF-8 when well-formed).
    pub fn as_bytes(&self) -> &[u8] {
        match &self.0 {
            Repr::Inline { len, bytes } => &bytes[..usize::from(*len)],
            Repr::Utf8(s) => s.as_bytes(),
            Repr::Wtf8(b) => b,
        }
    }

    /// Whether the string contains unpaired surrogates.
    pub fn has_lone_surrogates(&self) -> bool {
        matches!(self.0, Repr::Wtf8(_))
    }

    /// Code points, with unpaired surrogates as values in `0xD800..=0xDFFF`.
    pub fn code_points(&self) -> CodePoints<'_> {
        CodePoints {
            bytes: self.as_bytes(),
            index: 0,
        }
    }

    /// UTF-16 code units, as JavaScript indexes the string.
    pub fn utf16_units(&self) -> impl Iterator<Item = u16> + '_ {
        self.code_points().flat_map(|cp| {
            if cp >= 0x1_0000 {
                let v = cp - 0x1_0000;
                [
                    Some(0xd800 | (v >> 10) as u16),
                    Some(0xdc00 | (v & 0x3ff) as u16),
                ]
            } else {
                [Some(cp as u16), None]
            }
            .into_iter()
            .flatten()
        })
    }

    /// `string.length` in JavaScript.
    pub fn utf16_len(&self) -> usize {
        if let Some(s) = self.as_str() {
            s.chars().map(char::len_utf16).sum()
        } else {
            self.code_points()
                .map(|cp| if cp >= 0x1_0000 { 2 } else { 1 })
                .sum()
        }
    }

    /// The code unit at `index` as a one-unit string (`s[index]` in JavaScript).
    pub fn unit_at(&self, index: usize) -> Option<JsString> {
        let unit = self.utf16_units().nth(index)?;
        let mut b = Wtf8Builder::new();
        b.push_unit(unit);
        Some(b.finish())
    }

    /// Compares by UTF-16 code units, the order of `Array.prototype.sort()`.
    pub fn cmp_utf16(&self, other: &JsString) -> Ordering {
        if utf16_order_matches_bytes(self.as_bytes()) && utf16_order_matches_bytes(other.as_bytes())
        {
            return self.as_bytes().cmp(other.as_bytes());
        }
        self.utf16_units().cmp(other.utf16_units())
    }

    /// Reads WTF-8 bytes (UTF-8 generalized to unpaired surrogates, as
    /// [`JsString::as_bytes`] writes them). Returns `None` for anything else:
    /// invalid or overlong sequences, code points past U+10FFFF, and a high
    /// surrogate directly followed by a low one (a pair must be written as one
    /// four-byte sequence), so every string has exactly one byte form.
    pub fn from_wtf8(bytes: &[u8]) -> Option<JsString> {
        if let Ok(s) = core::str::from_utf8(bytes) {
            return Some(JsString::from(s));
        }
        let cont = |i: usize| bytes.get(i).copied().filter(|b| b & 0xc0 == 0x80);
        let mut i = 0;
        let mut previous_high = false;
        while i < bytes.len() {
            let b0 = bytes[i];
            let (cp, len) = match b0 {
                0x00..=0x7f => (b0 as u32, 1),
                0xc2..=0xdf => (((b0 as u32) & 0x1f) << 6 | (cont(i + 1)? as u32 & 0x3f), 2),
                0xe0..=0xef => {
                    let (b1, b2) = (cont(i + 1)?, cont(i + 2)?);
                    if b0 == 0xe0 && b1 < 0xa0 {
                        return None; // overlong
                    }
                    let cp =
                        ((b0 as u32) & 0x0f) << 12 | (b1 as u32 & 0x3f) << 6 | (b2 as u32 & 0x3f);
                    (cp, 3)
                }
                0xf0..=0xf4 => {
                    let (b1, b2, b3) = (cont(i + 1)?, cont(i + 2)?, cont(i + 3)?);
                    if (b0 == 0xf0 && b1 < 0x90) || (b0 == 0xf4 && b1 > 0x8f) {
                        return None; // overlong or past U+10FFFF
                    }
                    let cp = ((b0 as u32) & 0x07) << 18
                        | (b1 as u32 & 0x3f) << 12
                        | (b2 as u32 & 0x3f) << 6
                        | (b3 as u32 & 0x3f);
                    (cp, 4)
                }
                _ => return None,
            };
            if previous_high && (0xdc00..=0xdfff).contains(&cp) {
                return None;
            }
            previous_high = (0xd800..=0xdbff).contains(&cp);
            i += len;
        }
        // Not UTF-8 yet well-formed WTF-8: at least one unpaired surrogate.
        Some(JsString(Repr::Wtf8(bytes.into())))
    }

    /// Builds a string from UTF-16 code units, pairing surrogates where possible.
    pub fn from_utf16_units(units: &[u16]) -> JsString {
        let mut b = Wtf8Builder::new();
        for &u in units {
            b.push_unit(u);
        }
        b.finish()
    }
}

/// True when the bytes contain no code point at or above U+E000 and no
/// surrogate, so byte order equals UTF-16 order. Lead bytes 0xED (U+D000 to
/// U+DFFF, including surrogates), 0xEE to 0xEF (U+E000 to U+FFFF), and 0xF0 and
/// above (supplementary planes) need the slow comparison.
pub(crate) fn utf16_order_matches_bytes(bytes: &[u8]) -> bool {
    bytes.iter().all(|&b| b < 0xed)
}

impl Default for JsString {
    fn default() -> JsString {
        JsString::new()
    }
}

impl PartialEq for JsString {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for JsString {}

impl Hash for JsString {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

impl PartialOrd for JsString {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for JsString {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

impl Borrow<[u8]> for JsString {
    fn borrow(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl From<&str> for JsString {
    fn from(s: &str) -> JsString {
        if s.len() <= INLINE_CAP {
            let mut bytes = [0; INLINE_CAP];
            bytes[..s.len()].copy_from_slice(s.as_bytes());
            JsString(Repr::Inline {
                len: s.len() as u8,
                bytes,
            })
        } else {
            JsString(Repr::Utf8(s.into()))
        }
    }
}

impl From<String> for JsString {
    fn from(s: String) -> JsString {
        if s.len() <= INLINE_CAP {
            JsString::from(s.as_str())
        } else {
            JsString(Repr::Utf8(s.into_boxed_str()))
        }
    }
}

impl From<&String> for JsString {
    fn from(s: &String) -> JsString {
        JsString::from(s.as_str())
    }
}

impl PartialEq<str> for JsString {
    fn eq(&self, other: &str) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl PartialEq<&str> for JsString {
    fn eq(&self, other: &&str) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl fmt::Debug for JsString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Repr::Inline { .. } => fmt::Debug::fmt(self.as_str().unwrap(), f),
            Repr::Utf8(s) => fmt::Debug::fmt(s, f),
            Repr::Wtf8(_) => {
                f.write_str("\"")?;
                for cp in self.code_points() {
                    match char::from_u32(cp) {
                        Some(c) => write!(f, "{}", c.escape_debug())?,
                        None => write!(f, "\\u{{{cp:x}}}")?,
                    }
                }
                f.write_str("\"")
            }
        }
    }
}

impl fmt::Display for JsString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Repr::Inline { .. } => f.write_str(self.as_str().unwrap()),
            Repr::Utf8(s) => f.write_str(s),
            Repr::Wtf8(_) => f.write_str(&self.to_string_lossy()),
        }
    }
}

/// Iterator over generalized code points of WTF-8 bytes.
pub struct CodePoints<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl Iterator for CodePoints<'_> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        let b = self.bytes;
        let i = self.index;
        let first = *b.get(i)? as u32;
        let (cp, len) = if first < 0x80 {
            (first, 1)
        } else if first < 0xe0 {
            (((first & 0x1f) << 6) | (b[i + 1] as u32 & 0x3f), 2)
        } else if first < 0xf0 {
            (
                ((first & 0x0f) << 12) | ((b[i + 1] as u32 & 0x3f) << 6) | (b[i + 2] as u32 & 0x3f),
                3,
            )
        } else {
            (
                ((first & 0x07) << 18)
                    | ((b[i + 1] as u32 & 0x3f) << 12)
                    | ((b[i + 2] as u32 & 0x3f) << 6)
                    | (b[i + 3] as u32 & 0x3f),
                4,
            )
        };
        self.index += len;
        Some(cp)
    }
}

/// Accumulates code units or code points into a [`JsString`], joining a high
/// surrogate followed by a low surrogate into one supplementary code point, as
/// JavaScript strings do.
#[derive(Default)]
pub(crate) struct Wtf8Builder {
    bytes: Vec<u8>,
    lone: bool,
    /// A high surrogate written last, which a following low surrogate joins.
    pending_high: Option<u16>,
}

impl Wtf8Builder {
    pub(crate) fn new() -> Wtf8Builder {
        Wtf8Builder::default()
    }

    pub(crate) fn with_capacity(capacity: usize) -> Wtf8Builder {
        Wtf8Builder {
            bytes: Vec::with_capacity(capacity),
            ..Default::default()
        }
    }

    fn flush_high(&mut self) {
        if let Some(high) = self.pending_high.take() {
            encode_generalized(high as u32, &mut self.bytes);
            self.lone = true;
        }
    }

    pub(crate) fn push_char(&mut self, c: char) {
        self.flush_high();
        let mut buf = [0u8; 4];
        self.bytes
            .extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }

    pub(crate) fn push_str(&mut self, s: &str) {
        self.flush_high();
        self.bytes.extend_from_slice(s.as_bytes());
    }

    pub(crate) fn push_unit(&mut self, unit: u16) {
        match unit {
            0xd800..=0xdbff => {
                self.flush_high();
                self.pending_high = Some(unit);
            }
            0xdc00..=0xdfff => {
                if let Some(high) = self.pending_high.take() {
                    let cp = 0x1_0000 + (((high as u32) - 0xd800) << 10) + (unit as u32 - 0xdc00);
                    encode_generalized(cp, &mut self.bytes);
                } else {
                    encode_generalized(unit as u32, &mut self.bytes);
                    self.lone = true;
                }
            }
            _ => {
                self.flush_high();
                encode_generalized(unit as u32, &mut self.bytes);
            }
        }
    }

    pub(crate) fn finish(mut self) -> JsString {
        self.flush_high();
        if self.lone {
            JsString(Repr::Wtf8(self.bytes.into_boxed_slice()))
        } else {
            match String::from_utf8(self.bytes) {
                Ok(s) => JsString::from(s),
                // Unreachable: without lone surrogates the bytes are UTF-8.
                Err(e) => JsString(Repr::Wtf8(e.into_bytes().into_boxed_slice())),
            }
        }
    }
}

fn encode_generalized(cp: u32, out: &mut Vec<u8>) {
    if cp < 0x80 {
        out.push(cp as u8);
    } else if cp < 0x800 {
        out.extend_from_slice(&[0xc0 | (cp >> 6) as u8, 0x80 | (cp & 0x3f) as u8]);
    } else if cp < 0x1_0000 {
        out.extend_from_slice(&[
            0xe0 | (cp >> 12) as u8,
            0x80 | ((cp >> 6) & 0x3f) as u8,
            0x80 | (cp & 0x3f) as u8,
        ]);
    } else {
        out.extend_from_slice(&[
            0xf0 | (cp >> 18) as u8,
            0x80 | ((cp >> 12) & 0x3f) as u8,
            0x80 | ((cp >> 6) & 0x3f) as u8,
            0x80 | (cp & 0x3f) as u8,
        ]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashMap, hash_map::DefaultHasher};

    #[test]
    fn short_utf8_is_inline_and_the_representation_is_three_words() {
        assert!(core::mem::size_of::<JsString>() <= 3 * core::mem::size_of::<usize>());
        for text in ["", "v", "inEase", "é", "😀", &"x".repeat(INLINE_CAP)] {
            let value = JsString::from(text);
            assert!(matches!(&value.0, Repr::Inline { .. }), "{text:?}");
            assert_eq!(value.as_str(), Some(text));
            assert_eq!(value.clone().as_bytes(), text.as_bytes());
            assert_eq!(value.to_string(), text);
        }
        let long = "x".repeat(INLINE_CAP + 1);
        let value = JsString::from(long.clone());
        assert!(matches!(&value.0, Repr::Utf8(_)));
        assert_eq!(value.as_str(), Some(long.as_str()));
        assert_eq!(JsString::from(&long), value);
        assert_eq!(JsString::from_wtf8(long.as_bytes()), Some(value));
    }

    #[test]
    fn byte_equality_hash_and_order_ignore_utf8_storage() {
        let inline = JsString::from("short");
        let boxed = JsString(Repr::Utf8("short".into()));
        assert_eq!(inline, boxed);
        let hash = |s: &JsString| {
            let mut state = DefaultHasher::new();
            s.hash(&mut state);
            state.finish()
        };
        assert_eq!(hash(&inline), hash(&boxed));
        assert_eq!(inline.cmp(&boxed), Ordering::Equal);
        assert_eq!(inline.borrow() as &[u8], b"short");
        let mut hashed = HashMap::from([(boxed.clone(), 1)]);
        assert_eq!(hashed.get(b"short".as_slice()), Some(&1));
        assert_eq!(hashed.insert(inline.clone(), 2), Some(1));
        let ordered = BTreeMap::from([(boxed, 1), (JsString::from("second"), 2)]);
        assert_eq!(ordered.get(&inline), Some(&1));
    }

    #[test]
    fn escaped_utf8_and_lone_surrogates_keep_json_semantics() {
        let value =
            crate::json::parse(r#"{"short":"\u0061","lone":"\ud800","long":"abcdefghijklmnop"}"#)
                .unwrap();
        let object = value.as_object().unwrap();
        let short = object.get("short").unwrap().as_js_string().unwrap();
        assert!(matches!(&short.0, Repr::Inline { .. }));
        assert_eq!(short.as_str(), Some("a"));
        let lone = object.get("lone").unwrap().as_js_string().unwrap();
        assert!(matches!(&lone.0, Repr::Wtf8(_)));
        assert_eq!(lone.utf16_units().collect::<Vec<_>>(), vec![0xd800]);
        let long = object.get("long").unwrap().as_js_string().unwrap();
        assert!(matches!(&long.0, Repr::Utf8(_)));
        assert_eq!(
            crate::json::to_canonical_string(&value),
            r#"{"lone":"\ud800","long":"abcdefghijklmnop","short":"a"}"#
        );
    }

    #[test]
    fn pairs_surrogates_and_keeps_lone_ones() {
        let pair = JsString::from_utf16_units(&[0xd83d, 0xde00]);
        assert_eq!(pair.as_str(), Some("😀"));
        let lone = JsString::from_utf16_units(&[0x61, 0xd800]);
        assert!(lone.has_lone_surrogates());
        assert_eq!(lone.as_str(), None);
        assert_eq!(lone.utf16_units().collect::<Vec<_>>(), vec![0x61, 0xd800]);
        assert_eq!(lone.utf16_len(), 2);
        let trailing_low = JsString::from_utf16_units(&[0xdc00, 0x62]);
        assert_eq!(
            trailing_low.utf16_units().collect::<Vec<_>>(),
            vec![0xdc00, 0x62]
        );
        let reversed = JsString::from_utf16_units(&[0xde00, 0xd83d]);
        assert_eq!(
            reversed.utf16_units().collect::<Vec<_>>(),
            vec![0xde00, 0xd83d]
        );
        assert_eq!(lone.to_string_lossy(), "a\u{fffd}");
    }

    #[test]
    fn utf16_order_differs_from_code_point_order() {
        let bmp_high = JsString::from("\u{ffff}");
        let astral = JsString::from("\u{10000}");
        assert_eq!(bmp_high.cmp(&astral), Ordering::Less); // code points
        assert_eq!(bmp_high.cmp_utf16(&astral), Ordering::Greater); // UTF-16: 0xD800 < 0xFFFF
        assert_eq!(
            JsString::from("a").cmp_utf16(&JsString::from("b")),
            Ordering::Less
        );
        assert_eq!(
            JsString::from("10").cmp_utf16(&JsString::from("2")),
            Ordering::Less
        );
    }

    #[test]
    fn unit_access_matches_javascript_indexing() {
        let s = JsString::from("a😀");
        assert_eq!(s.utf16_len(), 3);
        assert_eq!(s.unit_at(0).unwrap(), "a");
        assert_eq!(
            s.unit_at(1).unwrap().utf16_units().collect::<Vec<_>>(),
            vec![0xd83d]
        );
        assert!(s.unit_at(3).is_none());
    }

    #[test]
    fn wtf8_reads_back_every_form_and_rejects_the_rest() {
        for units in [
            vec![],
            vec![0x61, 0x62],
            vec![0xd83d, 0xde00],
            vec![0xd800],
            vec![0xdc00, 0xd800],
            vec![0x61, 0xdbff, 0x62, 0xdfff],
            vec![0xe000, 0xffff, 0x7ff, 0x80],
        ] {
            let s = JsString::from_utf16_units(&units);
            assert_eq!(
                JsString::from_wtf8(s.as_bytes()).as_ref(),
                Some(&s),
                "{units:x?}"
            );
        }
        for bad in [
            &[0x80][..],
            &[0xc0, 0x80],                         // overlong
            &[0xe0, 0x80, 0x80],                   // overlong
            &[0xf0, 0x80, 0x80, 0x80],             // overlong
            &[0xf4, 0x90, 0x80, 0x80],             // past U+10FFFF
            &[0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80], // a pair written as two surrogates
            &[0xe2, 0x82],                         // truncated
            &[0xff],
        ] {
            assert_eq!(JsString::from_wtf8(bad), None, "{bad:x?}");
        }
    }
}
