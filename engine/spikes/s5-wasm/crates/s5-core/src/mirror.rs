//! The canonical mirror of one composition document.
//!
//! Layers are stored as their canonical JSON bytes plus the derived
//! evaluable data. Structural patches are reconciled by layer id: an element
//! whose bytes equal the stored bytes for its id is kept without parsing;
//! only changed, new, and removed ids invalidate cached answers.
//!
//! Property handles are numeric ids interned per (layer id, channel). They
//! survive reordering and reconciliation, re-resolve lazily, and cache the
//! last answer per time.

use crate::json_scan::{object_id_and_end, skip_value, skip_ws};
use crate::model::{LayerData, PropEval};
use crate::patch::{FramedPatch, Op, PatchError, PathPart, apply_value_patch, decode_frames, perr};
use rustc_hash::FxHashMap;
use serde_json::Value;

const UNRESOLVED: u32 = u32::MAX;
const MISSING: u32 = u32::MAX - 1;

struct Slot {
    data: LayerData,
    raw: Vec<u8>,
    claimed: u32,
}

#[derive(Clone)]
struct Handle {
    layer: Box<str>,
    channel: Box<str>,
    slot: u32,
    chan: u32,
    valid: bool,
    t_bits: u64,
    value: f64,
}

enum Field {
    Layers,
    /// Validated JSON kept as bytes until a patch reaches below it. Fields
    /// the evaluator never reads (the edit log, assets) stay opaque.
    Raw(Vec<u8>),
    Value(Value),
}

/// Validate JSON without building it.
fn validated(json: &[u8]) -> Result<Vec<u8>> {
    serde_json::from_slice::<serde::de::IgnoredAny>(json).map_err(json_err)?;
    Ok(json.to_vec())
}

/// Counters for the last `apply`/`load` call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ApplyStats {
    pub patches: u32,
    pub layers_parsed: u32,
    pub layers_kept: u32,
    pub layers_added: u32,
    pub layers_removed: u32,
    pub handles_invalidated: u32,
    pub bytes_compared: u64,
}

#[derive(Default)]
pub struct Mirror {
    slots: Vec<Option<Slot>>,
    free: Vec<u32>,
    order: Vec<u32>,
    by_id: FxHashMap<Box<str>, u32>,
    fields: Vec<(String, Field)>,
    handles: Vec<Handle>,
    handle_index: FxHashMap<Box<str>, u32>,
    handles_by_layer: FxHashMap<Box<str>, Vec<u32>>,
    pass: u32,
    stats: ApplyStats,
    locals: Vec<[f64; 6]>,
    locals_valid: Vec<u32>,
    locals_pass: u32,
}

type Result<T> = std::result::Result<T, PatchError>;

fn json_err(e: impl std::fmt::Display) -> PatchError {
    PatchError(format!("invalid JSON: {e}"))
}

/// Collects a top-level object as ordered (key, raw value) pairs.
struct TopLevel<'a>(Vec<(String, &'a serde_json::value::RawValue)>);

impl<'de: 'a, 'a> serde::Deserialize<'de> for TopLevel<'a> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V<'a>(std::marker::PhantomData<&'a ()>);
        impl<'de: 'a, 'a> serde::de::Visitor<'de> for V<'a> {
            type Value = TopLevel<'a>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a project object")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut m: M,
            ) -> std::result::Result<Self::Value, M::Error> {
                let mut out: Vec<(String, &'a serde_json::value::RawValue)> = Vec::new();
                while let Some((k, v)) =
                    m.next_entry::<String, &'a serde_json::value::RawValue>()?
                {
                    // JSON.parse keeps the last duplicate at the first key's position.
                    if let Some(existing) = out.iter_mut().find(|(key, _)| *key == k) {
                        existing.1 = v;
                    } else {
                        out.push((k, v));
                    }
                }
                Ok(TopLevel(out))
            }
        }
        d.deserialize_map(V(std::marker::PhantomData))
    }
}

impl Mirror {
    pub fn new() -> Mirror {
        Mirror::default()
    }

    pub fn stats(&self) -> ApplyStats {
        self.stats
    }

    pub fn layer_count(&self) -> usize {
        self.order.len()
    }

    pub fn handle_count(&self) -> usize {
        self.handles.len()
    }

    /// Layer ids in document order (tests and harnesses).
    pub fn layer_ids(&self) -> Vec<Option<String>> {
        self.order
            .iter()
            .map(|&s| self.slot(s).data.id.as_deref().map(String::from))
            .collect()
    }

    /// Out-of-scope properties: (layer id, channel, reason).
    pub fn unsupported(&self) -> Vec<(String, String, &'static str)> {
        let mut out = Vec::new();
        for &s in &self.order {
            let d = &self.slot(s).data;
            for (k, r) in d.unsupported() {
                out.push((d.id.as_deref().unwrap_or("").to_string(), k.to_string(), r));
            }
        }
        out
    }

    /// Bytes held by the layer store (raw JSON plus capacity slack).
    pub fn raw_bytes(&self) -> usize {
        self.slots.iter().flatten().map(|s| s.raw.capacity()).sum()
    }

    fn slot(&self, s: u32) -> &Slot {
        self.slots[s as usize].as_ref().expect("live slot")
    }

    // ── loading ──────────────────────────────────────────────────────────

    /// Replace the whole document.
    pub fn load(&mut self, json: &[u8]) -> Result<()> {
        self.stats = ApplyStats::default();
        self.load_inner(json)
    }

    fn load_inner(&mut self, json: &[u8]) -> Result<()> {
        let top: TopLevel = serde_json::from_slice(json).map_err(json_err)?;
        let mut fields = Vec::with_capacity(top.0.len());
        let mut layers: Option<&[u8]> = None;
        for (k, raw) in top.0 {
            if k == "layers" {
                layers = Some(raw.get().as_bytes());
                fields.push((k, Field::Layers));
            } else {
                fields.push((k, Field::Raw(raw.get().as_bytes().to_vec())));
            }
        }
        self.fields = fields;
        match layers {
            Some(bytes) => self.reconcile_layers(bytes)?,
            None => self.remove_all_layers(),
        }
        Ok(())
    }

    fn remove_all_layers(&mut self) {
        let old = std::mem::take(&mut self.order);
        for s in old {
            self.free_slot(s);
        }
        self.by_id.clear();
    }

    // ── handles ──────────────────────────────────────────────────────────

    /// Numeric id for (layer id, channel). Stable for the mirror's lifetime.
    pub fn intern(&mut self, layer: &str, channel: &str) -> u32 {
        let mut key = String::with_capacity(layer.len() + channel.len() + 1);
        key.push_str(layer);
        key.push('\0');
        key.push_str(channel);
        if let Some(&h) = self.handle_index.get(key.as_str()) {
            return h;
        }
        let h = self.handles.len() as u32;
        self.handles.push(Handle {
            layer: layer.into(),
            channel: channel.into(),
            slot: UNRESOLVED,
            chan: UNRESOLVED,
            valid: false,
            t_bits: 0,
            value: 0.0,
        });
        self.handle_index.insert(key.into_boxed_str(), h);
        self.handles_by_layer
            .entry(layer.into())
            .or_default()
            .push(h);
        h
    }

    fn invalidate_layer(&mut self, id: &str) {
        if let Some(list) = self.handles_by_layer.get(id) {
            for &h in list {
                let handle = &mut self.handles[h as usize];
                if handle.valid || handle.slot != UNRESOLVED {
                    self.stats.handles_invalidated += 1;
                }
                handle.valid = false;
                handle.slot = UNRESOLVED;
                handle.chan = UNRESOLVED;
            }
        }
    }

    /// After `slot` changed from `old` to its current data, invalidate only
    /// handles whose answers can differ.
    fn invalidate_changed(&mut self, slot: u32, old: &LayerData) {
        let new = &self.slots[slot as usize].as_ref().expect("live slot").data;
        if new.id != old.id {
            let new_id = new.id.clone();
            if let Some(id) = old.id.as_deref() {
                self.invalidate_layer(id);
            }
            if let Some(id) = new_id.as_deref() {
                self.invalidate_layer(id);
            }
            return;
        }
        let Some(id) = new.id.as_deref() else { return };
        let Some(list) = self.handles_by_layer.get(id) else {
            return;
        };
        let from_changed = new.from.to_bits() != old.from.to_bits();
        for &h in list {
            let handle = &mut self.handles[h as usize];
            if handle.slot == UNRESOLVED {
                continue;
            }
            if handle.slot != slot {
                continue; // a handle bound to another slot with the same id (duplicate ids)
            }
            let new_idx = new.channel_index(&handle.channel);
            let old_prop = match handle.chan {
                MISSING | UNRESOLVED => None,
                i => old.channels.get(i as usize).map(|c| &c.1),
            };
            let new_prop = new_idx.map(|i| &new.channels[i].1);
            let same = !from_changed
                && match (old_prop, new_prop) {
                    (None, None) => true,
                    (Some(a), Some(b)) => props_identical(a, b),
                    _ => false,
                };
            if same {
                handle.chan = new_idx.map(|i| i as u32).unwrap_or(MISSING);
            } else {
                if handle.valid {
                    self.stats.handles_invalidated += 1;
                }
                handle.valid = false;
                handle.slot = UNRESOLVED;
                handle.chan = UNRESOLVED;
            }
        }
    }

    #[inline]
    fn resolve(&mut self, h: usize) {
        let handle = &self.handles[h];
        match self.by_id.get(&*handle.layer) {
            Some(&slot) => {
                let data = &self.slots[slot as usize].as_ref().expect("live slot").data;
                let chan = data
                    .channel_index(&handle.channel)
                    .map(|i| i as u32)
                    .unwrap_or(MISSING);
                let handle = &mut self.handles[h];
                handle.slot = slot;
                handle.chan = chan;
            }
            None => {
                let handle = &mut self.handles[h];
                handle.slot = MISSING;
                handle.chan = MISSING;
            }
        }
    }

    /// `PM.ev(L, channel, T)` through a handle. A missing channel answers 0
    /// (as `PM.ev`), a missing layer or out-of-scope property NaN.
    #[inline]
    pub fn ev(&mut self, h: u32, t: f64) -> f64 {
        let h = h as usize;
        let Some(handle) = self.handles.get(h) else {
            return f64::NAN;
        };
        if handle.valid && handle.t_bits == t.to_bits() {
            return handle.value;
        }
        if handle.slot == UNRESOLVED {
            self.resolve(h);
        }
        let handle = &self.handles[h];
        let value = match (handle.slot, handle.chan) {
            (MISSING, _) => f64::NAN,
            (_, MISSING) => 0.0,
            (slot, chan) => {
                let data = &self.slots[slot as usize].as_ref().expect("live slot").data;
                data.channels[chan as usize].1.value(t - data.from)
            }
        };
        let handle = &mut self.handles[h];
        handle.valid = true;
        handle.t_bits = t.to_bits();
        handle.value = value;
        value
    }

    /// Evaluate many handles at one time into `out`.
    pub fn ev_batch(&mut self, ids: &[u32], t: f64, out: &mut [f64]) {
        for (i, &h) in ids.iter().enumerate() {
            out[i] = self.ev(h, t);
        }
    }

    // ── matrices (PM.localMatrix, PM.transformParentMatrix, PM.worldMatrix) ──

    fn local_uncached(&self, slot: u32, t: f64) -> [f64; 6] {
        let d = &self.slot(slot).data;
        let px = d.ev("position.x", t);
        let py = d.ev("position.y", t);
        let ax = d.ev("anchor.x", t);
        let ay = d.ev("anchor.y", t);
        let sx = d.ev("scale.x", t) / 100.0;
        let sy = d.ev("scale.y", t) / 100.0;
        let r = d.ev("rotation", t) * std::f64::consts::PI / 180.0;
        let sk = crate::math::tan(d.ev("skew", t) * std::f64::consts::PI / 180.0);
        let c = crate::math::cos(r);
        let s = crate::math::sin(r);
        let a = c * sx;
        let b = s * sx;
        let cc = (c * sk - s) * sy;
        let dd = (s * sk + c) * sy;
        [a, b, cc, dd, px - a * ax - cc * ay, py - b * ax - dd * ay]
    }

    fn local(&mut self, slot: u32, t: f64) -> [f64; 6] {
        let i = slot as usize;
        if self.locals_valid.len() <= i {
            self.locals_valid.resize(self.slots.len().max(i + 1), 0);
            self.locals.resize(self.slots.len().max(i + 1), [0.0; 6]);
        }
        if self.locals_valid[i] == self.locals_pass {
            return self.locals[i];
        }
        let m = self.local_uncached(slot, t);
        self.locals[i] = m;
        self.locals_valid[i] = self.locals_pass;
        m
    }

    fn parent_of(&self, slot: u32) -> Option<u32> {
        let parent = self.slot(slot).data.parent.as_deref()?;
        self.by_id.get(parent).copied()
    }

    fn group_ancestors(&self, slot: u32) -> Vec<u32> {
        let d = &self.slot(slot).data;
        let mut out = Vec::new();
        let Some(mut id) = d.group.as_deref() else {
            return out;
        };
        let mut seen: Vec<&str> = vec![d.id.as_deref().unwrap_or("\u{0}undefined")];
        while !seen.contains(&id) {
            seen.push(id);
            let Some(&g) = self.by_id.get(id) else { break };
            let gd = &self.slot(g).data;
            if !gd.is_group {
                break;
            }
            out.push(g);
            match gd.group.as_deref() {
                Some(next) => id = next,
                None => break,
            }
        }
        out
    }

    fn transform_parent_matrix(&mut self, slot: u32, t: f64) -> [f64; 6] {
        let mut groups: Vec<u32> = Vec::new();
        let mut matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        self.append_groups(slot, t, &mut groups, &mut matrix);
        let first = self.parent_of(slot);
        self.append_parents(slot, first, t, &mut groups, &mut matrix);
        matrix
    }

    fn append_parents(
        &mut self,
        layer: u32,
        first: Option<u32>,
        t: f64,
        groups: &mut Vec<u32>,
        matrix: &mut [f64; 6],
    ) {
        let mut chain: Vec<u32> = Vec::new();
        let mut seen = vec![layer];
        let mut cur = first;
        while let Some(c) = cur {
            if seen.contains(&c) || chain.len() >= 256 {
                break;
            }
            seen.push(c);
            chain.push(c);
            self.append_groups(c, t, groups, matrix);
            cur = self.parent_of(c);
        }
        for &c in chain.iter().rev() {
            let l = self.local(c, t);
            *matrix = mul(matrix, &l);
        }
    }

    fn append_groups(&mut self, layer: u32, t: f64, groups: &mut Vec<u32>, matrix: &mut [f64; 6]) {
        let ancestors = self.group_ancestors(layer);
        for &g in ancestors.iter().rev() {
            // groups are keyed by id in TS; slots map 1:1 to the ids resolved here
            if groups.contains(&g) {
                continue;
            }
            groups.push(g);
            let first = self.parent_of(g);
            self.append_parents(g, first, t, groups, matrix);
            let l = self.local(g, t);
            *matrix = mul(matrix, &l);
        }
    }

    fn world(&mut self, slot: u32, t: f64) -> [f64; 6] {
        let d = &self.slot(slot).data;
        if d.parent.is_none() && d.group.is_none() {
            return self.local(slot, t);
        }
        let mut chain: Vec<u32> = Vec::new();
        let mut cur = Some(slot);
        while let Some(c) = cur {
            if chain.len() >= 256 {
                break;
            }
            if chain.len() > 8 && chain.contains(&c) {
                break;
            }
            chain.push(c);
            cur = self.parent_of(c);
        }
        if chain.iter().any(|&c| self.slot(c).data.group.is_some()) {
            let p = self.transform_parent_matrix(slot, t);
            let l = self.local(slot, t);
            return mul(&p, &l);
        }
        let mut m = self.local(*chain.last().expect("non-empty"), t);
        for i in (0..chain.len().saturating_sub(1)).rev() {
            let l = self.local(chain[i], t);
            m = mul(&m, &l);
        }
        m
    }

    fn begin_matrices(&mut self) {
        self.locals_pass = self.locals_pass.wrapping_add(1);
        if self.locals_pass == 0 {
            self.locals_valid.iter_mut().for_each(|v| *v = u32::MAX);
            self.locals_pass = 1;
        }
    }

    /// World matrix of the layer at document index `index`.
    pub fn world_matrix(&mut self, index: usize, t: f64) -> Option<[f64; 6]> {
        let slot = *self.order.get(index)?;
        self.begin_matrices();
        Some(self.world(slot, t))
    }

    /// World matrices of every layer in document order, 6 values each.
    pub fn world_all(&mut self, t: f64, out: &mut [f64]) -> usize {
        self.begin_matrices();
        let n = self.order.len().min(out.len() / 6);
        for i in 0..n {
            let slot = self.order[i];
            let m = self.world(slot, t);
            out[i * 6..i * 6 + 6].copy_from_slice(&m);
        }
        n
    }

    // ── slots ────────────────────────────────────────────────────────────

    fn alloc_slot(&mut self, slot: Slot) -> u32 {
        if let Some(s) = self.free.pop() {
            self.slots[s as usize] = Some(slot);
            s
        } else {
            self.slots.push(Some(slot));
            (self.slots.len() - 1) as u32
        }
    }

    fn free_slot(&mut self, s: u32) {
        if let Some(slot) = self.slots[s as usize].take() {
            self.stats.layers_removed += 1;
            if let Some(id) = slot.data.id.as_deref() {
                if self.by_id.get(id) == Some(&s) {
                    self.by_id.remove(id);
                }
                self.invalidate_layer(id);
            }
            self.free.push(s);
        }
    }

    fn rebuild_index(&mut self) {
        self.by_id.clear();
        for &s in &self.order {
            if let Some(id) = self.slots[s as usize]
                .as_ref()
                .and_then(|x| x.data.id.clone())
            {
                self.by_id.entry(id).or_insert(s);
            }
        }
        // Handles may now resolve to different slots.
        for h in &mut self.handles {
            h.valid = false;
            h.slot = UNRESOLVED;
            h.chan = UNRESOLVED;
        }
    }

    /// Parse one layer element and store it in `target` (reusing the slot
    /// when given). Returns the slot.
    fn store_layer(&mut self, bytes: &[u8], target: Option<u32>) -> Result<u32> {
        let value: Value = serde_json::from_slice(bytes).map_err(json_err)?;
        self.store_parsed(bytes, &value, target)
    }

    fn store_parsed(&mut self, bytes: &[u8], value: &Value, target: Option<u32>) -> Result<u32> {
        let data = LayerData::from_json(value);
        self.stats.layers_parsed += 1;
        match target {
            Some(s) => {
                let slot = self.slots[s as usize].as_mut().expect("live slot");
                let old = std::mem::replace(&mut slot.data, data);
                slot.raw.clear();
                slot.raw.extend_from_slice(bytes);
                if slot.data != old {
                    self.invalidate_changed(s, &old);
                }
                Ok(s)
            }
            None => {
                let id = data.id.clone();
                let s = self.alloc_slot(Slot {
                    data,
                    raw: bytes.to_vec(),
                    claimed: 0,
                });
                self.stats.layers_added += 1;
                if let Some(id) = id {
                    self.invalidate_layer(&id);
                }
                Ok(s)
            }
        }
    }

    // ── structural reconciliation ────────────────────────────────────────

    /// Replace the layers array with `arr` (JSON), keyed by layer id.
    pub fn reconcile_layers(&mut self, arr: &[u8]) -> Result<()> {
        self.pass = self.pass.wrapping_add(1).max(1);
        let pass = self.pass;
        let mut order = Vec::with_capacity(self.order.len() + 4);
        let mut duplicate = false;
        let mut i = skip_ws(arr, 0);
        if arr.get(i) != Some(&b'[') {
            return perr("layers must be an array");
        }
        i = skip_ws(arr, i + 1);
        if arr.get(i) == Some(&b']') {
            i += 1;
        } else {
            loop {
                let start = i;
                let mut kept = None;
                if let Some(id) = quick_id(arr, start)
                    && let Some(&s) = self.by_id.get(id)
                {
                    let slot = self.slots[s as usize].as_ref().expect("live slot");
                    let end = start + slot.raw.len();
                    if slot.claimed != pass && end <= arr.len() && arr[start..end] == slot.raw[..] {
                        self.stats.bytes_compared += slot.raw.len() as u64;
                        kept = Some((s, end));
                    }
                }
                let end = match kept {
                    Some((s, end)) => {
                        self.slots[s as usize].as_mut().expect("live slot").claimed = pass;
                        self.stats.layers_kept += 1;
                        order.push(s);
                        end
                    }
                    None => {
                        let end = if arr.get(start) == Some(&b'{') {
                            object_id_and_end(arr, start)
                                .map_err(|e| PatchError(e.to_string()))?
                                .1
                        } else {
                            skip_value(arr, start).map_err(|e| PatchError(e.to_string()))?
                        };
                        let bytes = &arr[start..end];
                        let s = self.claim_or_create(bytes, pass, &mut duplicate)?;
                        order.push(s);
                        end
                    }
                };
                i = skip_ws(arr, end);
                match arr.get(i) {
                    Some(b',') => i = skip_ws(arr, i + 1),
                    Some(b']') => {
                        i += 1;
                        break;
                    }
                    _ => return perr("expected ',' or ']' in layers"),
                }
            }
        }
        if skip_ws(arr, i) != arr.len() {
            return perr("trailing bytes after layers");
        }
        let old = std::mem::replace(&mut self.order, order);
        for s in old {
            if self.slots[s as usize]
                .as_ref()
                .is_some_and(|x| x.claimed != pass)
            {
                self.free_slot(s);
            }
        }
        if duplicate {
            self.rebuild_index();
        }
        Ok(())
    }

    fn claim_or_create(&mut self, bytes: &[u8], pass: u32, duplicate: &mut bool) -> Result<u32> {
        // The authoritative id comes from a full parse (escapes, duplicate keys).
        let value: Value = serde_json::from_slice(bytes).map_err(json_err)?;
        let id = value.get("id").and_then(Value::as_str);
        let existing = id.and_then(|id| self.by_id.get(id).copied());
        match existing {
            Some(s)
                if self.slots[s as usize]
                    .as_ref()
                    .is_some_and(|x| x.claimed != pass) =>
            {
                let s = self.store_parsed(bytes, &value, Some(s))?;
                self.slots[s as usize].as_mut().expect("live slot").claimed = pass;
                Ok(s)
            }
            Some(_) => {
                *duplicate = true;
                let s = self.store_parsed(bytes, &value, None)?;
                self.slots[s as usize].as_mut().expect("live slot").claimed = pass;
                Ok(s)
            }
            None => {
                let s = self.store_parsed(bytes, &value, None)?;
                let slot = self.slots[s as usize].as_mut().expect("live slot");
                slot.claimed = pass;
                if let Some(id) = slot.data.id.clone() {
                    self.by_id.insert(id, s);
                }
                Ok(s)
            }
        }
    }

    /// Id-keyed structural patch: the new order as ids plus the JSON of
    /// changed or new layers. Unlisted layers are removed.
    pub fn apply_layer_order(&mut self, ids: &[String], layers: &[&[u8]]) -> Result<()> {
        self.pass = self.pass.wrapping_add(1).max(1);
        let pass = self.pass;
        let mut parsed: Vec<(String, Value)> = Vec::with_capacity(layers.len());
        for bytes in layers {
            let value: Value = serde_json::from_slice(bytes).map_err(json_err)?;
            match value.get("id").and_then(Value::as_str) {
                Some(id) => parsed.push((id.to_string(), value)),
                None => return perr("layer-order patch layers need string ids"),
            }
        }
        let mut provided: FxHashMap<&str, (&[u8], &Value)> = FxHashMap::default();
        for ((id, value), bytes) in parsed.iter().zip(layers) {
            if provided.insert(id.as_str(), (bytes, value)).is_some() {
                return perr("duplicate layer in layer-order patch");
            }
        }
        let mut order = Vec::with_capacity(ids.len());
        for id in ids {
            let existing = self.by_id.get(id.as_str()).copied();
            let s = match (provided.get(id.as_str()), existing) {
                (Some(&(bytes, value)), Some(s)) => {
                    let same = self.slots[s as usize]
                        .as_ref()
                        .is_some_and(|x| x.raw[..] == bytes[..]);
                    if same {
                        self.stats.layers_kept += 1;
                        s
                    } else {
                        self.store_parsed(bytes, value, Some(s))?
                    }
                }
                (Some(&(bytes, value)), None) => {
                    let s = self.store_parsed(bytes, value, None)?;
                    self.by_id.insert(id.as_str().into(), s);
                    s
                }
                (None, Some(s)) => {
                    self.stats.layers_kept += 1;
                    s
                }
                (None, None) => return perr(format!("layer-order patch names unknown layer {id}")),
            };
            let slot = self.slots[s as usize].as_mut().expect("live slot");
            if slot.claimed == pass {
                return perr("duplicate id in layer-order patch");
            }
            slot.claimed = pass;
            order.push(s);
        }
        let old = std::mem::replace(&mut self.order, order);
        for s in old {
            if self.slots[s as usize]
                .as_ref()
                .is_some_and(|x| x.claimed != pass)
            {
                self.free_slot(s);
            }
        }
        Ok(())
    }

    // ── patches ──────────────────────────────────────────────────────────

    /// Apply a framed patch list (see `patch.rs`). On error the mirror may
    /// hold a prefix of the list; the caller resynchronizes with `load`.
    pub fn apply_framed(&mut self, bytes: &[u8]) -> Result<()> {
        self.stats = ApplyStats::default();
        let frames = decode_frames(bytes)?;
        for frame in frames {
            self.stats.patches += 1;
            match frame {
                FramedPatch::Delete { path } => self.apply_patch(&path, None)?,
                FramedPatch::Set { path, value } => self.apply_patch(&path, Some(value))?,
                FramedPatch::LayerOrder { path, ids, layers } => {
                    if path != [PathPart::Key("layers".into())] {
                        return perr("layer-order patches target [\"layers\"]");
                    }
                    if !self.fields.iter().any(|(k, _)| k == "layers") {
                        self.fields.push(("layers".into(), Field::Layers));
                    }
                    self.apply_layer_order(&ids, &layers)?;
                }
            }
        }
        Ok(())
    }

    /// One leaf patch; `value` is JSON for a set, `None` for a delete.
    pub fn apply_patch(&mut self, path: &[PathPart], value: Option<&[u8]>) -> Result<()> {
        let Some((first, rest)) = path.split_first() else {
            return match value {
                Some(json) => self.load_inner(json),
                None => {
                    self.fields.clear();
                    self.remove_all_layers();
                    Ok(())
                }
            };
        };
        let key = match first {
            PathPart::Key(k) => k.as_str(),
            PathPart::Index(_) => return Ok(()), // numeric key on the root object: not part of the schema
        };
        if key == "layers" {
            return self.apply_layers_patch(rest, value);
        }
        let position = self.fields.iter().position(|(k, _)| k == key);
        if rest.is_empty() {
            match (value, position) {
                (Some(json), Some(p)) => {
                    self.fields[p].1 = Field::Raw(validated(json)?);
                }
                (Some(json), None) => {
                    self.fields.push((key.into(), Field::Raw(validated(json)?)));
                }
                (None, Some(p)) => {
                    self.fields.remove(p);
                }
                (None, None) => {}
            }
            return Ok(());
        }
        let Some(p) = position else { return Ok(()) };
        if let Field::Raw(bytes) = &self.fields[p].1 {
            let value: Value = serde_json::from_slice(bytes).map_err(json_err)?;
            self.fields[p].1 = Field::Value(value);
        }
        let Field::Value(root) = &mut self.fields[p].1 else {
            unreachable!("layers handled above, raw converted")
        };
        match value {
            Some(json) => {
                let v: Value = serde_json::from_slice(json).map_err(json_err)?;
                apply_value_patch(root, rest, Op::Set(&v))
            }
            None => apply_value_patch(root, rest, Op::Delete),
        }
    }

    fn apply_layers_patch(&mut self, rest: &[PathPart], value: Option<&[u8]>) -> Result<()> {
        let has_layers = self.fields.iter().any(|(k, _)| k == "layers");
        let Some((index_part, below)) = rest.split_first() else {
            // The whole array: structural edit, reconciled by id.
            return match value {
                Some(json) => {
                    if !has_layers {
                        self.fields.push(("layers".into(), Field::Layers));
                    }
                    self.reconcile_layers(json)
                }
                None => {
                    self.fields.retain(|(k, _)| k != "layers");
                    self.remove_all_layers();
                    Ok(())
                }
            };
        };
        if !has_layers {
            return Ok(()); // parent missing: JS skips
        }
        let Some(index) = index_part.as_index() else {
            return Ok(());
        };
        if below.is_empty() {
            return match value {
                Some(json) => self.replace_layer_at(index, json),
                None => {
                    if index < self.order.len() {
                        let s = self.order.remove(index);
                        let id = self.slot(s).data.id.clone();
                        self.free_slot(s);
                        if let Some(id) = id {
                            self.reindex_id(&id);
                        }
                    }
                    Ok(())
                }
            };
        }
        let Some(&s) = self.order.get(index) else {
            return Ok(());
        };
        let slot = self.slots[s as usize].as_mut().expect("live slot");
        let mut layer: Value = serde_json::from_slice(&slot.raw).map_err(json_err)?;
        match value {
            Some(json) => {
                let v: Value = serde_json::from_slice(json).map_err(json_err)?;
                apply_value_patch(&mut layer, below, Op::Set(&v))?;
            }
            None => apply_value_patch(&mut layer, below, Op::Delete)?,
        }
        let data = LayerData::from_json(&layer);
        slot.raw.clear();
        serde_json::to_writer(&mut slot.raw, &layer).map_err(json_err)?;
        self.stats.layers_parsed += 1;
        let old = std::mem::replace(&mut slot.data, data);
        let id_changed = slot.data.id != old.id;
        let new_id = slot.data.id.clone();
        if slot.data != old {
            self.invalidate_changed(s, &old);
        }
        if id_changed {
            if let Some(id) = old.id.as_deref() {
                self.reindex_id(id);
            }
            if let Some(id) = new_id.as_deref() {
                self.reindex_id(id);
            }
        }
        Ok(())
    }

    fn replace_layer_at(&mut self, index: usize, json: &[u8]) -> Result<()> {
        if index > self.order.len() {
            return perr("a layer index past the end would leave a hole in layers");
        }
        if index == self.order.len() {
            let s = self.store_layer(json, None)?;
            self.order.push(s);
            if let Some(id) = self.slot(s).data.id.clone() {
                self.reindex_id(&id);
            }
            return Ok(());
        }
        let s = self.order[index];
        let old_id = self.slot(s).data.id.clone();
        if self.slot(s).raw[..] == json[..] {
            self.stats.layers_kept += 1;
            return Ok(());
        }
        self.store_layer(json, Some(s))?;
        let new_id = self.slot(s).data.id.clone();
        if new_id != old_id {
            if let Some(id) = old_id.as_deref() {
                self.reindex_id(id);
            }
            if let Some(id) = new_id.as_deref() {
                self.reindex_id(id);
            }
        }
        Ok(())
    }

    /// Point `id` at its first occurrence in document order (or drop it).
    fn reindex_id(&mut self, id: &str) {
        let first = self.order.iter().copied().find(|&s| {
            self.slots[s as usize]
                .as_ref()
                .is_some_and(|x| x.data.id.as_deref() == Some(id))
        });
        let before = self.by_id.get(id).copied();
        match first {
            Some(s) => {
                self.by_id.insert(id.into(), s);
            }
            None => {
                self.by_id.remove(id);
            }
        }
        if before != first {
            self.invalidate_layer(id);
        }
    }

    // ── serialization ────────────────────────────────────────────────────

    /// The whole document as JSON (layers from their canonical bytes).
    pub fn to_json(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.raw_bytes() + 4096);
        out.push(b'{');
        for (n, (k, field)) in self.fields.iter().enumerate() {
            if n > 0 {
                out.push(b',');
            }
            serde_json::to_writer(&mut out, k).expect("string serializes");
            out.push(b':');
            match field {
                Field::Layers => {
                    out.push(b'[');
                    for (i, &s) in self.order.iter().enumerate() {
                        if i > 0 {
                            out.push(b',');
                        }
                        out.extend_from_slice(&self.slot(s).raw);
                    }
                    out.push(b']');
                }
                Field::Raw(bytes) => out.extend_from_slice(bytes),
                Field::Value(v) => serde_json::to_writer(&mut out, v).expect("value serializes"),
            }
        }
        out.push(b'}');
        out
    }
}

/// `{"id":"...` with no escapes: the common case, read without scanning.
#[inline]
fn quick_id(arr: &[u8], start: usize) -> Option<&str> {
    const PREFIX: &[u8] = b"{\"id\":\"";
    let rest = arr.get(start..)?;
    if !rest.starts_with(PREFIX) {
        return None;
    }
    let body = &rest[PREFIX.len()..];
    let end = memchr::memchr2(b'"', b'\\', body)?;
    if body[end] != b'"' {
        return None;
    }
    std::str::from_utf8(&body[..end]).ok()
}

/// Bit-level equality, so a NaN-valued property compares equal to itself.
fn props_identical(a: &PropEval, b: &PropEval) -> bool {
    match (a, b) {
        (PropEval::Static(x), PropEval::Static(y)) => x.to_bits() == y.to_bits(),
        _ => a == b,
    }
}

#[inline]
pub fn mul(m: &[f64; 6], n: &[f64; 6]) -> [f64; 6] {
    [
        m[0] * n[0] + m[2] * n[1],
        m[1] * n[0] + m[3] * n[1],
        m[0] * n[2] + m[2] * n[3],
        m[1] * n[2] + m[3] * n[3],
        m[0] * n[4] + m[2] * n[5] + m[4],
        m[1] * n[4] + m[3] * n[5] + m[5],
    ]
}
