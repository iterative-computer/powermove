//! Lazy Parley font collection from the existing registry/replay boundary.
//! Resolving the requested typography first uses the same font queries in
//! native and WASM hosts. Parley owns the final shaping and line breaking.
//!
//! Every request registers exactly the faces it resolved, under exactly the
//! families a fresh collection would give them, so a reused collection can
//! never offer Parley a face (another weight, a static face next to a
//! variable one) that the request did not ask for. A collection is cached
//! per registration signature and swapped into one layout context, whose
//! shaping caches follow [`FaceBlob`] identity across collections.
//!
//! Bounded memory: split faces form a least-recently-used cache with a hard
//! byte and entry ceiling, enforced whenever a request returns, and a face
//! that leaves it also leaves every cached collection (the active one
//! included) and layout that used it. Parley retains no font bytes outside
//! that cache. A request whose own faces exceed the ceiling uses them for
//! its layout and leaves them to the returned frame alone.

use super::lru::Lru;
use pm_text::{
    compat::{Compat, TextContent},
    font::{FontBytes, FontDesc, FontResolver},
    level2::{FaceBlob, FontSet, Level2, Level2Layout, StyleSpan},
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::mem::size_of;

/// Split faces retained between requests. Room for one split Apple Color
/// Emoji face (about 190 MB) beside ordinary faces.
pub(super) const FACE_LIMIT_BYTES: usize = 256 * 1024 * 1024;
pub(super) const FACE_LIMIT_ENTRIES: usize = 32;
/// Collections, one per registration signature (faces and their families).
pub(super) const SET_LIMIT_ENTRIES: usize = 32;
const SET_KEY_LIMIT_BYTES: usize = 2 * 1024 * 1024;
const FACE_ALIAS_LIMIT_ENTRIES: usize = 32;
const FACE_ALIAS_LIMIT_BYTES: usize = 32 * 1024;
pub(super) const LAYOUT_LIMIT_BYTES: usize = 2 * 1024 * 1024;
pub(super) const LAYOUT_LIMIT_ENTRIES: usize = 32;

/// A resolver face: its bytes' identity and its index in them.
type Identity = (u64, u32);

/// One face registration, in request order.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Registration {
    /// The registry key `Run::font` reports (`blob#index`).
    key: String,
    face: Identity,
    /// The split face's identity, which changes when the face is split again.
    font: u64,
    alias: String,
}

/// Everything a request puts in a collection.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Signature {
    registrations: Vec<Registration>,
    fallback: Vec<String>,
}

impl Signature {
    fn uses(&self, font: u64) -> bool {
        self.registrations.iter().any(|r| r.font == font)
    }
    fn bytes(&self) -> usize {
        self.registrations.capacity() * size_of::<Registration>()
            + self
                .registrations
                .iter()
                .map(|r| heap(r.key.capacity()) + heap(r.alias.capacity()))
                .sum::<usize>()
            + heap(self.fallback.capacity() * size_of::<String>())
            + self
                .fallback
                .iter()
                .map(|f| heap(f.capacity()))
                .sum::<usize>()
    }
}

struct Face {
    key: String,
    font: FontBytes,
    /// Whether `font` is a copy (a face split out of a collection file);
    /// otherwise it shares the resolver's bytes.
    copied: bool,
    /// One blob per family the face is registered under, as a fresh
    /// collection registers it, kept so shaping data is reused.
    blobs: Vec<(String, FaceBlob)>,
}

struct LayoutEntry {
    signature: Signature,
    content: TextContent,
    spans: Vec<StyleSpan>,
    layout: Level2Layout,
    bytes: usize,
}

/// Cache counters since creation.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Counters {
    pub face_hits: u64,
    pub face_splits: u64,
    pub face_evictions: u64,
    pub set_hits: u64,
    pub set_builds: u64,
    pub set_evictions: u64,
    pub registrations: u64,
    pub layouts: u64,
    pub layout_hits: u64,
}

pub(super) struct Level2Cache {
    engine: Level2,
    faces: Lru<Identity, Face>,
    /// The face each registry key currently resolves to.
    by_key: HashMap<String, Identity>,
    /// Collections not in the engine; the active one's entry is `None`.
    sets: Lru<Signature, Option<FontSet>>,
    active: Option<Signature>,
    layouts: VecDeque<LayoutEntry>,
    layout_bytes: usize,
    pub(super) counters: Counters,
    /// [`FACE_LIMIT_BYTES`] and [`FACE_LIMIT_ENTRIES`] (tests lower them).
    pub(super) face_limit_bytes: usize,
    pub(super) face_limit_entries: usize,
}

impl Default for Level2Cache {
    fn default() -> Self {
        Self {
            engine: Level2::new(),
            faces: Lru::default(),
            by_key: HashMap::new(),
            sets: Lru::default(),
            active: None,
            layouts: VecDeque::new(),
            layout_bytes: 0,
            counters: Counters::default(),
            face_limit_bytes: FACE_LIMIT_BYTES,
            face_limit_entries: FACE_LIMIT_ENTRIES,
        }
    }
}

/// A heap allocation's charge: its size plus allocator bookkeeping.
fn heap(bytes: usize) -> usize {
    if bytes == 0 { 0 } else { bytes + 16 }
}

fn content_heap(c: &TextContent) -> usize {
    heap(c.text.capacity())
        + c.font.as_ref().map_or(0, |f| heap(f.capacity()))
        + c.align.as_ref().map_or(0, |a| heap(a.capacity()))
        + heap(c.axes.capacity() * size_of::<(String, f64)>())
        + c.axes
            .iter()
            .map(|(t, _)| heap(t.capacity()))
            .sum::<usize>()
}

fn layout_heap(l: &Level2Layout) -> usize {
    use pm_text::compat::{Boundary, PlacedGlyph, Unit};
    use pm_text::level2::{Line, Run};
    let units = |u: &Vec<Unit>| {
        heap(u.capacity() * size_of::<Unit>())
            + u.iter().map(|u| heap(u.text.capacity())).sum::<usize>()
    };
    heap(l.lines.capacity() * size_of::<Line>())
        + l.lines
            .iter()
            .map(|l| heap(l.text.capacity()))
            .sum::<usize>()
        + heap(l.runs.capacity() * size_of::<Run>())
        + l.runs
            .iter()
            .map(|r| {
                heap(r.font.capacity())
                    + heap(r.coords.capacity() * size_of::<i16>())
                    + heap(r.glyphs.capacity() * size_of::<PlacedGlyph>())
                    + heap(r.sources.capacity() * size_of::<usize>())
            })
            .sum::<usize>()
        + heap(l.caret.capacity() * size_of::<Vec<Boundary>>())
        + l.caret
            .iter()
            .map(|c| heap(c.capacity() * size_of::<Boundary>()))
            .sum::<usize>()
        + units(&l.units.characters)
        + units(&l.units.words)
        + units(&l.units.lines)
}

/// What a cached layout retains: the entry, its key (each span carries a
/// whole `TextContent`, text included) and the layout's own allocations.
fn layout_entry_bytes(entry: &LayoutEntry) -> usize {
    size_of::<LayoutEntry>()
        + entry.signature.bytes()
        + content_heap(&entry.content)
        + heap(entry.spans.capacity() * size_of::<StyleSpan>())
        + entry
            .spans
            .iter()
            .map(|s| content_heap(&s.content))
            .sum::<usize>()
        + layout_heap(&entry.layout)
}

impl Level2Cache {
    pub(super) fn face_entries(&self) -> usize {
        self.faces.len()
    }
    pub(super) fn face_bytes(&self) -> usize {
        self.faces.bytes()
    }
    /// Bytes split out of collection files (the rest share resolver bytes).
    pub(super) fn copied_bytes(&self) -> usize {
        self.faces
            .values()
            .filter(|f| f.copied)
            .map(|f| f.font.as_slice().len())
            .sum()
    }
    pub(super) fn set_entries(&self) -> usize {
        self.sets.len()
    }
    /// Registrations held by every cached collection.
    pub(super) fn set_registrations(&self) -> usize {
        // The active collection's registrations are its signature's.
        self.active.as_ref().map_or(0, |s| s.registrations.len())
            + self
                .sets
                .values()
                .flatten()
                .map(FontSet::registrations)
                .sum::<usize>()
    }
    pub(super) fn layout_entries(&self) -> usize {
        self.layouts.len()
    }
    pub(super) fn layout_bytes(&self) -> usize {
        self.layout_bytes
    }

    /// The single-face font for `index` of `source`, split once and kept
    /// while it is used.
    fn face(
        &mut self,
        key: &str,
        source: &FontBytes,
        index: u32,
        used: &mut HashSet<Identity>,
    ) -> Result<FontBytes, String> {
        let identity = (source.id(), index);
        used.insert(identity);
        // A registry key that now serves new bytes (a font replaced, a
        // replay subset grown) never reaches the old face again.
        if let Some(&old) = self.by_key.get(key)
            && old != identity
            && !used.contains(&old)
        {
            self.evict(old);
        }
        if let Some(face) = self.faces.get(&identity) {
            self.counters.face_hits += 1;
            return Ok(face.font.clone());
        }
        let (font, copied) = single_face(source, index)?;
        self.counters.face_splits += 1;
        let bytes = font.as_slice().len();
        let face = Face {
            key: key.to_owned(),
            font: font.clone(),
            copied,
            blobs: Vec::new(),
        };
        self.faces.insert(identity, face, bytes);
        self.by_key.insert(key.to_owned(), identity);
        Ok(font)
    }

    fn blob(&mut self, identity: Identity, alias: &str) -> FaceBlob {
        // Faces are only evicted between requests (or superseded when not
        // used by this one), so every face this request resolved is here.
        let face = self
            .faces
            .get(&identity)
            .expect("a request's faces stay cached until it returns");
        if let Some(index) = face.blobs.iter().position(|(a, _)| a == alias) {
            let entry = face.blobs.remove(index);
            let blob = entry.1.clone();
            face.blobs.push(entry);
            return blob;
        }
        let blob = FaceBlob::new(&face.font);
        if alias.len() <= FACE_ALIAS_LIMIT_BYTES {
            face.blobs.push((alias.to_owned(), blob.clone()));
            while face.blobs.len() > FACE_ALIAS_LIMIT_ENTRIES
                || face.blobs.iter().map(|(a, _)| a.capacity()).sum::<usize>()
                    > FACE_ALIAS_LIMIT_BYTES
            {
                face.blobs.remove(0);
            }
        }
        blob
    }

    /// Drops a face and everything that holds it.
    fn evict(&mut self, identity: Identity) {
        if let Some(face) = self.faces.remove(&identity) {
            self.forget(identity, face);
        }
    }

    /// Releases an evicted face from Parley collections (the active one
    /// included) and from layouts shaped with it.
    fn forget(&mut self, identity: Identity, face: Face) {
        self.counters.face_evictions += 1;
        if self.by_key.get(&face.key) == Some(&identity) {
            self.by_key.remove(&face.key);
        }
        let font = face.font.id();
        if self.active.as_ref().is_some_and(|s| s.uses(font)) {
            self.engine.replace_fonts(FontSet::new());
            self.active = None;
        }
        let dropped = self.sets.retain(|signature, _| !signature.uses(font));
        self.counters.set_evictions += dropped.len() as u64;
        let mut freed = 0;
        self.layouts.retain(|entry| {
            let keep = !entry.signature.uses(font);
            if !keep {
                freed += entry.bytes;
            }
            keep
        });
        self.layout_bytes -= freed;
    }

    /// Evicts least recently used faces until the cache is within its
    /// ceiling. Runs when a request returns, so a request's own faces go too
    /// when they alone exceed it: the frame keeps the only references.
    fn trim(&mut self) {
        while self.faces.len() > self.face_limit_entries
            || self.faces.bytes() > self.face_limit_bytes
        {
            let Some((identity, face)) = self.faces.pop_lru(|_| false) else {
                break;
            };
            self.forget(identity, face);
        }
        // Family aliases can change while the underlying face stays hot.
        // Bound their collection keys too, including an oversized active
        // request; the returned frame needs no Parley collection.
        while self.sets.len() > SET_LIMIT_ENTRIES || self.sets.bytes() > SET_KEY_LIMIT_BYTES {
            let Some((signature, _)) = self.sets.pop_lru(|_| false) else {
                break;
            };
            if self.active.as_ref() == Some(&signature) {
                self.engine.replace_fonts(FontSet::new());
                self.active = None;
            }
            self.counters.set_evictions += 1;
        }
    }

    /// Makes `signature` the engine's collection, registering its faces only
    /// when no cached collection has exactly these registrations.
    fn activate(&mut self, signature: &Signature) {
        if self.active.as_ref() == Some(signature) {
            self.sets.get(signature);
            self.counters.set_hits += 1;
            return;
        }
        let cached = self.sets.get(signature).and_then(Option::take);
        let fresh = cached.is_none();
        let previous = self.engine.replace_fonts(cached.unwrap_or_default());
        if let Some(active) = self.active.take()
            && let Some(slot) = self.sets.get(&active)
        {
            *slot = Some(previous);
        }
        if fresh {
            for r in &signature.registrations {
                let blob = self.blob(r.face, &r.alias);
                self.engine.add_face(&r.key, &blob, &r.alias);
            }
            self.engine.set_fallback_families(&signature.fallback);
            self.counters.set_builds += 1;
            self.counters.registrations += signature.registrations.len() as u64;
            // Hash-map and recency-index keys each own the signature.
            self.sets
                .insert(signature.clone(), None, 2 * signature.bytes());
        } else {
            self.counters.set_hits += 1;
        }
        self.active = Some(signature.clone());
        while self.sets.len() > SET_LIMIT_ENTRIES {
            let active = self.active.as_ref();
            if self.sets.pop_lru(|k| Some(k) == active).is_none() {
                break;
            }
            self.counters.set_evictions += 1;
        }
    }

    fn layout(
        &mut self,
        signature: Signature,
        d: &TextContent,
        spans: &[StyleSpan],
    ) -> Level2Layout {
        if let Some(index) = self.layouts.iter().position(|entry| {
            entry.signature == signature && entry.content == *d && entry.spans == spans
        }) {
            let entry = self.layouts.remove(index).expect("layout cache index");
            let layout = entry.layout.clone();
            self.layouts.push_back(entry);
            self.counters.layout_hits += 1;
            return layout;
        }
        let layout = self.engine.layout_styled(d, spans);
        self.counters.layouts += 1;
        if layout.missing_glyphs == 0 {
            let mut entry = LayoutEntry {
                signature,
                content: d.clone(),
                spans: spans.to_vec(),
                layout: layout.clone(),
                bytes: 0,
            };
            entry.bytes = layout_entry_bytes(&entry);
            if entry.bytes <= LAYOUT_LIMIT_BYTES {
                self.layout_bytes += entry.bytes;
                self.layouts.push_back(entry);
                while self.layouts.len() > LAYOUT_LIMIT_ENTRIES
                    || self.layout_bytes > LAYOUT_LIMIT_BYTES
                {
                    let Some(old) = self.layouts.pop_front() else {
                        break;
                    };
                    self.layout_bytes -= old.bytes;
                }
            }
        }
        layout
    }
}

pub(super) fn shape<R: FontResolver>(
    compat: &mut Compat<R>,
    cache: &mut Level2Cache,
    d: &TextContent,
    spans: &[StyleSpan],
) -> Result<crate::text_level2::Shaped, String> {
    let mut faces: HashMap<String, (FontBytes, Identity)> = HashMap::new();
    let mut used = HashSet::new();
    let mut registrations: Vec<Registration> = Vec::new();
    let mut fallback = Vec::new();
    let mut typography = vec![d];
    for span in spans {
        if !typography.contains(&&span.content) {
            typography.push(&span.content);
        }
    }
    for content in typography {
        let family = content.font.as_deref().unwrap_or("system-ui");
        let desc = FontDesc {
            size: content.size() as f32,
            weight: content
                .weight
                .filter(|w| *w != 0.0 && !w.is_nan())
                .unwrap_or(500.0)
                .clamp(1.0, 1000.0) as f32,
            italic: content.italic.unwrap_or(false),
        };
        let variable = (!content.axes.is_empty())
            .then(|| compat.resolver_mut().variable_face(family))
            .flatten();
        let primary = variable
            .or_else(|| compat.resolver_mut().family(family, &desc))
            .or_else(|| compat.resolver_mut().family("system-ui", &desc))
            .or_else(|| compat.resolver_mut().standard(&desc));
        // Resolve fallback for all characters, not just those in the span:
        // fonts can affect shaping on either side of a script boundary.
        let runs = compat.text_runs(content, &d.text, 0.0).0;
        let ids = primary.into_iter().chain(runs.iter().map(|r| r.face));
        for id in ids {
            let info = compat.resolver().face(id);
            let key = format!("{}#{}", info.blob, info.index);
            if !faces.contains_key(&key) {
                let font = match cache.face(&key, &info.data, info.index, &mut used) {
                    Ok(font) => font,
                    Err(why) => {
                        cache.trim();
                        return Err(why);
                    }
                };
                faces.insert(key.clone(), (font, (info.data.id(), info.index)));
            }
            let alias = if Some(id) == primary {
                family.to_owned()
            } else {
                format!("pm-fallback-{key}")
            };
            let (font, face) = &faces[&key];
            let registration = Registration {
                key: key.clone(),
                face: *face,
                font: font.id(),
                alias: alias.clone(),
            };
            if !registrations.contains(&registration) {
                registrations.push(registration);
            }
            if !fallback.contains(&alias) {
                fallback.push(alias);
            }
        }
    }
    if faces.is_empty() && !d.text.is_empty() {
        cache.trim();
        return Err("level 2 font bytes unavailable".into());
    }
    let signature = Signature {
        registrations,
        fallback,
    };
    cache.activate(&signature);
    let layout = cache.layout(signature, d, spans);
    cache.trim();
    if layout.missing_glyphs > 0 {
        return Err("level 2 text whose fonts are not loaded".into());
    }
    let faces = faces.into_iter().map(|(k, (font, _))| (k, font)).collect();
    Ok(crate::text_level2::Shaped { layout, faces })
}

/// A single-face font for `index`, and whether it had to be copied out of a
/// collection file (a single font is shared as it is).
fn single_face(data: &FontBytes, index: u32) -> Result<(FontBytes, bool), String> {
    use skrifa::raw::FileRef;
    let file = FileRef::new(data.as_slice()).map_err(|e| format!("level 2 font: {e}"))?;
    if matches!(file, FileRef::Font(_)) {
        return Ok((data.clone(), false));
    }
    let font = file
        .fonts()
        .nth(index as usize)
        .ok_or("level 2 font collection index")?
        .map_err(|e| e.to_string())?;
    let mut tables: Vec<(u32, Vec<u8>)> = font
        .table_directory()
        .table_records()
        .iter()
        .filter_map(|r| {
            Some((
                u32::from_be_bytes(r.tag().to_be_bytes()),
                font.table_data(r.tag())?.as_bytes().to_vec(),
            ))
        })
        .collect();
    Ok((FontBytes::new(pm_text::sfnt::build_sfnt(&mut tables)), true))
}

#[cfg(test)]
mod tests;
