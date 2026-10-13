//! Blink's macOS character fallback cache (`FontCache::
//! PlatformFallbackFontForCharacter` with `MacCharacterFallbackCacheEnabled`).
//! For ideographs, the first substitute font found for a (font being
//! substituted, weight, style, size) key is reused for later ideographs that
//! font covers, for the life of the process. Results therefore depend on the
//! order text was measured: a CJK string measured after another one may keep
//! the earlier string's fallback font. Level 1 layout reproduces this; it
//! lives here, outside the registry, so native and WASM layout run the same
//! stateful logic over the same deterministic registry answers.

use crate::blink::chars;
use crate::font::{FaceId, FaceInfo, FontDesc, FontResolver};
use read_fonts::{FontRef, TableProvider};
use std::collections::HashMap;

type Key = (String, u32, bool, u32);

/// A resolver behind Blink's character fallback cache. Native and WASM
/// layout both wrap their resolver in it, so they run the same stateful
/// logic over the same registry answers.
pub struct FallbackCached<R> {
    pub inner: R,
    map: HashMap<Key, FaceId>,
}

impl<R: FontResolver> FallbackCached<R> {
    pub fn new(inner: R) -> Self {
        FallbackCached { inner, map: HashMap::new() }
    }

    /// Forgets cached substitutes (Blink's cache lives for the process; a
    /// host resets it when it starts a fresh measuring session, such as a
    /// corpus run or a document reopen).
    pub fn reset(&mut self) {
        self.map.clear();
    }
}

/// `typeface->unicharToGlyph(ch) != 0`.
fn has_glyph(info: &FaceInfo, ch: u32) -> bool {
    FontRef::from_index(info.data.as_slice(), info.index)
        .ok()
        .and_then(|f| f.cmap().ok())
        .and_then(|c| c.map_codepoint(ch))
        .is_some_and(|g| g.to_u32() != 0)
}

impl<R: FontResolver> FontResolver for FallbackCached<R> {
    fn face(&self, id: FaceId) -> &FaceInfo {
        self.inner.face(id)
    }
    fn family(&mut self, family: &str, desc: &FontDesc) -> Option<FaceId> {
        self.inner.family(family, desc)
    }
    fn standard(&mut self, desc: &FontDesc) -> Option<FaceId> {
        self.inner.standard(desc)
    }
    fn fallback_for_char(&mut self, desc: &FontDesc, ch: u32, primary: FaceId, emoji: bool) -> Option<FaceId> {
        if emoji {
            return self.inner.fallback_for_char(desc, ch, primary, emoji);
        }
        let key = if chars::is_ideographic(ch) {
            self.inner.face(primary).fallback_key.clone().map(|k| (k, desc.weight.to_bits(), desc.italic, desc.size.to_bits()))
        } else {
            None
        };
        if let Some(k) = &key
            && let Some(&cached) = self.map.get(k)
                && has_glyph(self.inner.face(cached), ch) {
                    return Some(cached);
                }
        let found = self.inner.fallback_for_char(desc, ch, primary, emoji);
        if let (Some(k), Some(f)) = (key, found) {
            // WTF::HashMap::insert keeps an existing entry.
            self.map.entry(k).or_insert(f);
        }
        found
    }
    fn last_resort(&mut self, desc: &FontDesc) -> Option<FaceId> {
        self.inner.last_resort(desc)
    }
    fn variable_face(&mut self, family: &str) -> Option<FaceId> {
        self.inner.variable_face(family)
    }
    fn em_advance(&mut self, face: FaceId, size: f32) -> Option<f32> {
        self.inner.em_advance(face, size)
    }
}
