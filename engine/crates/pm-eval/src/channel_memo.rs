//! What [`Evaluator::ev`](crate::Evaluator::ev) found for a layer's channel,
//! kept across the frames of one unchanged document (docs/engine/perf.md,
//! "Channel memo").
//!
//! `PM.ev(L, key, T)` reads `L.p`, `L.p[key]`, its `expr`, its `kf` and its
//! length, `L.from`, and prepares the keyframe track (every key's time,
//! value, and easing) on every call: about a third of a 500-layer frame's
//! plan was string-keyed member lookups and track preparation, repeated
//! unchanged from frame to frame while the document stayed the same. The
//! memo keeps, per layer and channel, the part of that work that does not
//! depend on the time:
//!
//! - the channel is falsy: the answer is `0`;
//! - no `expr` and no keyframes: the answer is `prop.v`;
//! - no `expr` and a temporal track: the answer is the prepared track at
//!   `T - L.from`, for a finite local time.
//!
//! It also keeps each layer's group ancestry (`PM.groupAncestors(L)`, read
//! for every member's activity and transform), which reads only the
//! composition's layers.
//! And a layer's local matrix when every channel `PM.localMatrix` reads
//! was found time-independent: it is the same at every time then.
//!
//! Anything else (an expression, a discrete or throwing track, a read that
//! throws, a non-finite time) takes the ordinary path, which this mirrors
//! read for read, so every answer is the one the ordinary path gives.
//!
//! # Validity
//!
//! A renderer opens the memo for one document around a frame's plan
//! ([`Evaluator::open_channel_memo`](crate::Evaluator::open_channel_memo))
//! and closes it after; it is never consulted otherwise. Entries are keyed
//! by the layer node's address and the channel key, and kept only while the
//! document's [`edition`](pm_model::Document::edition) is the one they were
//! found at: a document value whose edition did not change had no node
//! created, dropped, moved, or changed, so an address names one node with one
//! content. The first plan at an edition only notes it: a document edited
//! between every two frames never pays for entries it cannot reuse. Only
//! layers of the composition being evaluated (an element of its
//! `layers` array, by address) are kept, so nodes made during a plan (which
//! can reuse a freed address) never reach the memo.

use std::collections::HashMap;
use std::sync::Arc;

use pm_model::Node;

use crate::keyframes::TemporalTrack;

/// What a layer's channel answers without its nodes.
#[derive(Clone, Debug)]
pub(crate) enum Found {
    /// `L.p[key]` is falsy: `0`.
    Zero,
    /// No `expr`, no keyframes: `prop.v` (`undefined` when the channel is
    /// not an object).
    Static(crate::JsValue),
    /// No `expr`, a temporal track: its value at `T - from`.
    Track {
        from: f64,
        track: Arc<TemporalTrack>,
    },
    /// The ordinary path, every time.
    Ordinary,
}

/// The memo an [`Evaluator`](crate::Evaluator) keeps (see the module
/// documentation).
#[derive(Default)]
pub(crate) struct ChannelMemo {
    /// While the memo is open: the document's root node (address) and
    /// revision, which every composition evaluated in it shares (a guard:
    /// a composition of any other document is never answered).
    open: Option<(usize, u64)>,
    edition: Option<u64>,
    /// By layer address and key hash: the key (a span of `keys`) and
    /// what was found.
    entries: HashMap<(usize, u64), (u32, u32, Found), crate::scope::BuildAddressHasher>,
    /// The entries' keys, end to end (cleared with them; no allocation per
    /// entry).
    keys: Vec<u8>,
    /// `PM.groupAncestors(L)` by layer address, as positions in the
    /// composition's `layers` (`Evaluator::group_ancestors`).
    ancestors: HashMap<usize, crate::JsResult<Box<[u32]>>, crate::scope::BuildAddressHasher>,
    /// `PM.localMatrix(L, T)` by layer address when every channel it reads
    /// is time-independent (`None`: some channel is not; it is computed).
    locals: HashMap<usize, Option<crate::transform::Mat2d>, crate::scope::BuildAddressHasher>,
    pub(crate) hits: u64,
    pub(crate) misses: u64,
}

/// FNV-1a over the key's bytes.
fn key_hash(key: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in key {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    h
}

impl ChannelMemo {
    /// Opens the memo for a document at `edition` (dropping entries found
    /// at another one).
    pub(crate) fn open(&mut self, doc: &pm_model::Document) {
        let edition = doc.edition();
        if self.edition != Some(edition) {
            self.entries.clear();
            self.keys.clear();
            self.ancestors.clear();
            self.locals.clear();
            self.edition = Some(edition);
            // The first plan at an edition only notes it (see above).
            self.open = None;
            return;
        }
        self.open = doc.root().map(|root| (address(root), doc.revision()));
    }

    pub(crate) fn close(&mut self) {
        self.open = None;
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Forgets every entry.
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.keys.clear();
        self.ancestors.clear();
        self.locals.clear();
        self.edition = None;
    }

    /// The entry of `layer`'s channel `key`, when the memo is open and the
    /// layer is an element of `layers`.
    pub(crate) fn get(&mut self, cx: &crate::Comp<'_>, layer: &Node, key: &[u8]) -> Option<&Found> {
        if self.open != Some((address(cx.root()), cx.revision())) {
            return None;
        }
        let layers = cx.layers();
        if !is_element(layers, layer) {
            return None;
        }
        let k = (address(layer), key_hash(key));
        let keys = &self.keys;
        match self.entries.get(&k) {
            Some((at, len, found))
                if keys.get(*at as usize..(*at + *len) as usize) == Some(key) =>
            {
                self.hits += 1;
                Some(found)
            }
            _ => None,
        }
    }

    /// Keeps what the channel was found to be (when the memo is open, the
    /// layer is an element of `layers`, and no other key holds the slot).
    pub(crate) fn insert(&mut self, cx: &crate::Comp<'_>, layer: &Node, key: &[u8], found: Found) {
        if self.open != Some((address(cx.root()), cx.revision())) || !is_element(cx.layers(), layer)
        {
            return;
        }
        self.misses += 1;
        let bytes = key;
        let (Ok(at), Ok(len)) = (u32::try_from(self.keys.len()), u32::try_from(bytes.len())) else {
            return;
        };
        if at.checked_add(len).is_none() {
            return;
        }
        let k = (address(layer), key_hash(bytes));
        if let std::collections::hash_map::Entry::Vacant(slot) = self.entries.entry(k) {
            self.keys.extend_from_slice(bytes);
            slot.insert((at, len, found));
        }
    }
}

impl ChannelMemo {
    /// Whether the memo answers for compositions of `cx`'s document.
    fn answers(&self, cx: &crate::Comp<'_>) -> bool {
        self.open == Some((address(cx.root()), cx.revision()))
    }

    /// The group ancestors kept for `layer`, an element of `cx`'s layers.
    pub(crate) fn ancestors<'d>(
        &mut self,
        cx: &crate::Comp<'d>,
        layer: &Node,
    ) -> Option<crate::JsResult<Vec<&'d Node>>> {
        if !self.answers(cx) || !is_element(cx.layers(), layer) {
            return None;
        }
        let found = self.ancestors.get(&address(layer))?;
        self.hits += 1;
        let layers = cx.layers();
        Some(match found {
            Ok(at) => Ok(at.iter().map(|&i| &layers[i as usize]).collect()),
            Err(e) => Err(e.clone()),
        })
    }

    /// Keeps `layer`'s group ancestors (each an element of `cx`'s layers).
    pub(crate) fn keep_ancestors(
        &mut self,
        cx: &crate::Comp<'_>,
        layer: &Node,
        found: &crate::JsResult<Vec<&Node>>,
    ) {
        let layers = cx.layers();
        if !self.answers(cx) || !is_element(layers, layer) {
            return;
        }
        let kept = match found {
            Ok(groups) => {
                let mut at = Vec::with_capacity(groups.len());
                for g in groups {
                    let Some(i) = position(layers, g) else {
                        return;
                    };
                    at.push(i);
                }
                Ok(at.into_boxed_slice())
            }
            Err(e) => Err(e.clone()),
        };
        self.misses += 1;
        self.ancestors.insert(address(layer), kept);
    }
}

impl ChannelMemo {
    /// Whether `layer`'s channel `key` was found time-independent (falsy,
    /// or a static value).
    pub(crate) fn is_static(&self, cx: &crate::Comp<'_>, layer: &Node, key: &[u8]) -> bool {
        if !self.answers(cx) || !is_element(cx.layers(), layer) {
            return false;
        }
        let keys = &self.keys;
        matches!(
            self.entries.get(&(address(layer), key_hash(key))),
            Some((at, len, Found::Zero | Found::Static(_)))
                if keys.get(*at as usize..(*at + *len) as usize) == Some(key)
        )
    }

    /// What was kept of `layer`'s local matrix: `Some(Some(m))` a
    /// time-independent matrix, `Some(None)` one that is not.
    pub(crate) fn local(
        &mut self,
        cx: &crate::Comp<'_>,
        layer: &Node,
    ) -> Option<Option<crate::transform::Mat2d>> {
        if !self.answers(cx) || !is_element(cx.layers(), layer) {
            return None;
        }
        let found = *self.locals.get(&address(layer))?;
        self.hits += 1;
        Some(found)
    }

    pub(crate) fn keep_local(
        &mut self,
        cx: &crate::Comp<'_>,
        layer: &Node,
        m: Option<crate::transform::Mat2d>,
    ) {
        if !self.answers(cx) || !is_element(cx.layers(), layer) {
            return;
        }
        self.misses += 1;
        self.locals.insert(address(layer), m);
    }
}

/// `node`'s position in `layers`, when it is one of its elements.
fn position(layers: &[Node], node: &Node) -> Option<u32> {
    if !is_element(layers, node) {
        return None;
    }
    let i = (address(node) - layers.as_ptr() as usize) / std::mem::size_of::<Node>();
    u32::try_from(i).ok()
}

fn address(node: &Node) -> usize {
    std::ptr::from_ref(node) as usize
}

/// Whether `node` is one of the elements of `layers` (by address).
fn is_element(layers: &[Node], node: &Node) -> bool {
    let start = layers.as_ptr() as usize;
    let size = std::mem::size_of::<Node>();
    let at = address(node);
    size != 0
        && at >= start
        && at < start + std::mem::size_of_val(layers)
        && (at - start).is_multiple_of(size)
}
