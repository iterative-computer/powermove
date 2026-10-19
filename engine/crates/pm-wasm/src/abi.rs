//! The module's exports (wasm32 only).
//!
//! A plain C ABI over one [`Engine`] per module instance (one per editor
//! window or headless project), with no generated bindings: the renderer's
//! loader (`apps/desktop/src/renderer/src/engine-doc/wasm.ts`) is a few
//! dozen lines and the same module runs in Electron's renderer and in Node.
//!
//! Buffers live in linear memory and only grow:
//!
//! * the input buffer (`pm_in`): request JSON, patch bodies, documents;
//! * the reply buffer (`pm_out_ptr`, `pm_out_len`): JSON replies, hashes,
//!   messages;
//! * the batch buffers (`pm_batch_ids`, `pm_batch_out`, `pm_batch_kinds`):
//!   ids in, numbers and kinds out, as typed-array views (the S5 fast path).
//!
//! JavaScript re-acquires every view when `memory.buffer` changes. No export
//! panics on any input: errors are return codes with a message in the reply
//! buffer.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};

use pm_eval::{PropId, ValueKind};

use crate::engine::{ApplyError, Engine};

/// The default allocator with live and peak counters, so the per-window
/// memory budget reports what the mirror holds (linear memory never shrinks).
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

// SAFETY: every call forwards to `System` with the caller's layout unchanged;
// the counters do not affect allocation.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            grew(new_size);
        }
        p
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[derive(Default)]
struct State {
    engine: Engine,
    input: Vec<u8>,
    out: Vec<u8>,
    out_special: bool,
    ids: Vec<u32>,
    values: Vec<f64>,
    kinds: Vec<u8>,
    last_kind: u32,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

/// Grow-only: keeps contents, so a view made before a call stays valid as
/// long as memory does not grow.
fn reserve<T: Copy + Default>(buf: &mut Vec<T>, len: usize) -> *mut T {
    if buf.len() < len {
        let target = len.max(buf.len().saturating_mul(2)).max(64);
        buf.resize(target, T::default());
    }
    buf.as_mut_ptr()
}

fn input(state: &State, len: usize) -> &[u8] {
    &state.input[..len.min(state.input.len())]
}

fn reply(state: &mut State, text: &str, special: bool) -> usize {
    state.out.clear();
    state.out.extend_from_slice(text.as_bytes());
    state.out_special = special;
    state.out.len()
}

fn revision(r: f64) -> Option<u64> {
    ((0.0..=9_007_199_254_740_991.0).contains(&r) && r.fract() == 0.0).then_some(r as u64)
}

/// An input buffer of at least `len` bytes.
#[unsafe(no_mangle)]
pub extern "C" fn pm_in(len: usize) -> *mut u8 {
    with(|s| reserve(&mut s.input, len))
}

/// Frees the input buffer (after a large resync).
#[unsafe(no_mangle)]
pub extern "C" fn pm_in_release() {
    with(|s| s.input = Vec::new())
}

#[unsafe(no_mangle)]
pub extern "C" fn pm_out_ptr() -> *const u8 {
    with(|s| s.out.as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn pm_out_len() -> usize {
    with(|s| s.out.len())
}

/// Whether the last JSON reply holds special-number markers (`crate::reply`).
#[unsafe(no_mangle)]
pub extern "C" fn pm_out_special() -> u32 {
    with(|s| u32::from(s.out_special))
}

/// Replaces the mirror with `input[..len]` (canonical project JSON; empty for
/// an undefined document) at `rev`. 0 on success; otherwise 1 and the
/// message in the reply buffer.
#[unsafe(no_mangle)]
pub extern "C" fn pm_resync(len: usize, rev: f64) -> u32 {
    with(|s| {
        let Some(rev) = revision(rev) else {
            reply(s, "invalid revision", false);
            return 1;
        };
        let bytes = input(s, len).to_vec();
        match s.engine.resync(&bytes, rev) {
            Ok(()) => 0,
            Err(e) => {
                reply(s, &e, false);
                1
            }
        }
    })
}

/// Applies the `canonicalV1` body in `input[..len]` as revision `rev` on top
/// of `base`. 0 applied; 1 gap; 2 bad revision; 3 malformed body; 4 the
/// mirror diverged (earlier operations applied). The message is in the reply
/// buffer; any non-zero code asks the renderer to resync.
#[unsafe(no_mangle)]
pub extern "C" fn pm_apply(len: usize, base: f64, rev: f64) -> u32 {
    with(|s| {
        let (Some(base), Some(rev)) = (revision(base), revision(rev)) else {
            reply(s, "invalid revision", false);
            return 2;
        };
        let body = input(s, len).to_vec();
        match s.engine.apply(&body, base, rev) {
            Ok(_) => 0,
            Err(e) => {
                let code = match e {
                    ApplyError::Gap { .. } => 1,
                    ApplyError::Revision => 2,
                    ApplyError::Decode(_) => 3,
                    ApplyError::Mismatch(_) => 4,
                };
                reply(s, &e.to_string(), false);
                code
            }
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn pm_revision() -> f64 {
    with(|s| s.engine.revision() as f64)
}

/// The canonical hash into the reply buffer (64 hex digits; 0 bytes for an
/// undefined document).
#[unsafe(no_mangle)]
pub extern "C" fn pm_hash() -> usize {
    with(|s| {
        let h = s.engine.hash().unwrap_or_default();
        reply(s, &h, false)
    })
}

/// The canonical serialization of the mirror into the reply buffer.
#[unsafe(no_mangle)]
pub extern "C" fn pm_to_json() -> usize {
    with(|s| {
        let json = s.engine.to_json();
        reply(s, &json, false)
    })
}

/// Interns the address in `input[..len]` (JSON); the id, or -1.
#[unsafe(no_mangle)]
pub extern "C" fn pm_intern(len: usize) -> i32 {
    with(|s| {
        let Ok(request) = pm_model::json::parse_bytes(input(s, len)) else {
            return -1;
        };
        s.engine
            .intern(&request)
            .map_or(-1, |id| i32::try_from(id.index()).unwrap_or(-1))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn pm_release(id: u32) {
    with(|s| s.engine.release(PropId::from_index(id)))
}

/// One interned id at `t`. A number answers directly; otherwise NaN, and
/// `pm_kind` says whether the value was something else (1, `pm_last` has its
/// JSON) or a throw (2, `pm_last` has the message as a JSON string).
#[unsafe(no_mangle)]
pub extern "C" fn pm_ev(id: u32, t: f64) -> f64 {
    with(|s| {
        let (kind, n) = s.engine.ev_id(PropId::from_index(id), t);
        s.last_kind = kind as u32;
        n
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn pm_kind() -> u32 {
    with(|s| s.last_kind)
}

#[unsafe(no_mangle)]
pub extern "C" fn pm_last() -> usize {
    with(|s| {
        let json = s.engine.last_json();
        let special = s.engine.last_special();
        reply(s, &json, special)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn pm_batch_ids(n: usize) -> *mut u32 {
    with(|s| reserve(&mut s.ids, n))
}

#[unsafe(no_mangle)]
pub extern "C" fn pm_batch_out(n: usize) -> *mut f64 {
    with(|s| reserve(&mut s.values, n))
}

#[unsafe(no_mangle)]
pub extern "C" fn pm_batch_kinds(n: usize) -> *mut u8 {
    with(|s| reserve(&mut s.kinds, n))
}

/// `ids[..n]` at `t` into `out[..n]` and `kinds[..n]` (0 number, 1 other,
/// 2 threw). 0 on success; 1 when a buffer is smaller than `n`.
#[unsafe(no_mangle)]
pub extern "C" fn pm_ev_batch(n: usize, t: f64) -> u32 {
    with(|s| {
        if n > s.ids.len() || n > s.values.len() || n > s.kinds.len() {
            return 1;
        }
        let State {
            engine,
            ids,
            values,
            kinds,
            ..
        } = s;
        engine.ev_batch(&ids[..n], t, &mut values[..n], &mut kinds[..n]);
        0
    })
}

/// The expression error of an id's channel into the reply buffer; -1 when
/// there is none.
#[unsafe(no_mangle)]
pub extern "C" fn pm_expr_error(id: u32) -> i32 {
    with(
        |s| match s.engine.expression_error(PropId::from_index(id)) {
            Some(m) => {
                let m = m.to_owned();
                i32::try_from(reply(s, &m, false)).unwrap_or(-1)
            }
            None => -1,
        },
    )
}

/// Answers the JSON request in `input[..len]`; the reply's length.
#[unsafe(no_mangle)]
pub extern "C" fn pm_query(len: usize) -> usize {
    with(|s| {
        let request = input(s, len).to_vec();
        let r = s.engine.query(&request);
        reply(s, &r.text, r.special)
    })
}

/// Sets an extension definition's labels from `input[..len]`
/// (`{id, label, params: {key: label}}`).
#[unsafe(no_mangle)]
pub extern "C" fn pm_define(len: usize) {
    with(|s| {
        if let Ok(request) = pm_model::json::parse_bytes(input(s, len)) {
            s.engine.set_definition(&request);
        }
    })
}

/// Merges registry answers from `input[..len]`. 0 on success.
#[unsafe(no_mangle)]
pub extern "C" fn pm_fonts_answers(len: usize) -> u32 {
    with(|s| {
        let bytes = input(s, len).to_vec();
        match s.engine.add_font_answers(&bytes) {
            Ok(()) => 0,
            Err(e) => {
                reply(s, &e, false);
                1
            }
        }
    })
}

/// Adds a font subset from `input[..len]`: a u32 blob key length, the key,
/// a u32 face index, then the bytes. 0 on success.
#[unsafe(no_mangle)]
pub extern "C" fn pm_fonts_add(len: usize) -> u32 {
    with(|s| {
        let bytes = input(s, len);
        let u32_at = |at: usize| -> Option<u32> {
            Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
        };
        let Some(key_len) = u32_at(0).map(|n| n as usize) else {
            return 1;
        };
        let Some(key) = bytes
            .get(4..4 + key_len)
            .and_then(|k| std::str::from_utf8(k).ok())
        else {
            return 1;
        };
        let Some(index) = u32_at(4 + key_len) else {
            return 1;
        };
        let Some(data) = bytes.get(8 + key_len..) else {
            return 1;
        };
        let (key, data) = (key.to_owned(), data.to_vec());
        s.engine.add_font(&key, index, data);
        0
    })
}

/// Adds a 3D model asset from `input[..len]`: a u32 id length, the id, a u8
/// kind (0 missing, 1 OBJ text, 2 glTF or GLB bytes), then the bytes. 0 on
/// success; 1 for a malformed input; 2 when the model did not load (kept as
/// missing; the reason is in the reply buffer).
#[unsafe(no_mangle)]
pub extern "C" fn pm_model_set(len: usize) -> u32 {
    with(|s| {
        let bytes = input(s, len).to_vec();
        let Some(id_len) = bytes
            .get(0..4)
            .and_then(|b| b.try_into().ok())
            .map(|b: [u8; 4]| u32::from_le_bytes(b) as usize)
        else {
            return 1;
        };
        let Some(id) = bytes
            .get(4..4 + id_len)
            .and_then(|k| std::str::from_utf8(k).ok())
        else {
            return 1;
        };
        let Some(kind) = bytes
            .get(4 + id_len)
            .and_then(|k| crate::geometry::ModelKind::from_u8(*k))
        else {
            return 1;
        };
        let id = id.to_owned();
        match s.engine.set_model(&id, kind, &bytes[5 + id_len..]) {
            Ok(()) => 0,
            Err(e) => {
                reply(s, &e, false);
                2
            }
        }
    })
}

/// Import glTF/GLB bytes with pm-geom3d and return the compatibility scene
/// as JSON in the output buffer. A failure returns 1 and its message there.
#[unsafe(no_mangle)]
pub extern "C" fn pm_model_import(len: usize) -> u32 {
    with(|s| {
        let bytes = input(s, len);
        let result = crate::imported_model::import(bytes);
        match result {
            Ok(scene) => {
                reply(s, &scene.to_string(), false);
                0
            }
            Err(message) => {
                reply(s, &message, false);
                1
            }
        }
    })
}

/// Bundle selected OBJ/MTL/texture files through pm-geom3d. Input is a u32
/// JSON-header length, a JSON array of {name,mime,length}, then file bytes.
/// Status: 0 GLB bytes in output; 2 unchanged model index (decimal text);
/// 1 an error message. The caller validates File instances before invoking.
#[unsafe(no_mangle)]
pub extern "C" fn pm_prepare_model_import(len: usize) -> u32 {
    with(|s| {
        let source = input(s, len).to_vec();
        let input = source.as_slice();
        let Some(header_len) = input
            .get(..4)
            .and_then(|b| b.try_into().ok())
            .map(|b: [u8; 4]| u32::from_le_bytes(b) as usize)
        else {
            reply(s, "Invalid model import bundle", false);
            return 1;
        };
        let Some(header) = input
            .get(4..4 + header_len)
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok())
        else {
            reply(s, "Invalid model import bundle", false);
            return 1;
        };
        let Some(entries) = header.as_array() else {
            reply(s, "Invalid model import bundle", false);
            return 1;
        };
        let mut offset = 4 + header_len;
        let mut files = Vec::with_capacity(entries.len());
        for entry in entries {
            let (Some(name), Some(mime), Some(length)) = (
                entry.get("name").and_then(|v| v.as_str()),
                entry.get("mime").and_then(|v| v.as_str()),
                entry
                    .get("length")
                    .and_then(|v| v.as_u64())
                    .and_then(|n| usize::try_from(n).ok()),
            ) else {
                reply(s, "Invalid model import bundle", false);
                return 1;
            };
            let Some(end) = offset.checked_add(length) else {
                reply(s, "Invalid model import bundle", false);
                return 1;
            };
            let Some(bytes) = input.get(offset..end) else {
                reply(s, "Invalid model import bundle", false);
                return 1;
            };
            files.push(pm_geom3d::obj::ImportFile {
                name: name.to_owned(),
                mime: mime.to_owned(),
                bytes: bytes.to_vec(),
            });
            offset = end;
        }
        if offset != input.len() {
            reply(s, "Invalid model import bundle", false);
            return 1;
        }
        match pm_geom3d::obj::prepare_import(&files) {
            Ok(pm_geom3d::obj::Prepared::Unchanged(index)) => {
                reply(s, &index.to_string(), false);
                2
            }
            Ok(pm_geom3d::obj::Prepared::Glb { bytes, .. }) => {
                s.out = bytes;
                s.out_special = false;
                0
            }
            Err(e) => {
                reply(s, &e.0, false);
                1
            }
        }
    })
}

/// Forgets the 3D model asset whose id is `input[..len]`.
#[unsafe(no_mangle)]
pub extern "C" fn pm_model_forget(len: usize) {
    with(|s| {
        if let Ok(id) = std::str::from_utf8(input(s, len)).map(str::to_owned) {
            s.engine.forget_model(&id);
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn pm_models_clear() {
    with(|s| s.engine.clear_models())
}

/// `{misses, pending}` of the last text layout into the reply buffer.
#[unsafe(no_mangle)]
pub extern "C" fn pm_fonts_report() -> usize {
    with(|s| {
        let r = s.engine.font_report();
        reply(s, &r, false)
    })
}

/// Declares whether the faces added carry outlines (1) or are subsets (0).
#[unsafe(no_mangle)]
pub extern "C" fn pm_fonts_outlines(outlines: u32) {
    with(|s| s.engine.set_font_outlines(outlines != 0))
}

/// The editor renderer's font cache seeds (JSON, main's
/// `engine-fonts:seeds`). Returns 0 unchanged, 1 changed (text lays out
/// again), 2 malformed (the reason is in the output buffer).
#[unsafe(no_mangle)]
pub extern "C" fn pm_fonts_seeds(len: usize) -> u32 {
    with(|s| {
        let bytes = input(s, len).to_vec();
        match s.engine.set_font_seeds(&bytes) {
            Ok(false) => 0,
            Ok(true) => 1,
            Err(e) => {
                reply(s, &e, false);
                2
            }
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn pm_fonts_clear() {
    with(|s| s.engine.clear_fonts())
}

#[unsafe(no_mangle)]
pub extern "C" fn pm_heap_live() -> f64 {
    LIVE.load(Ordering::Relaxed) as f64
}

#[unsafe(no_mangle)]
pub extern "C" fn pm_heap_peak() -> f64 {
    PEAK.load(Ordering::Relaxed) as f64
}

#[unsafe(no_mangle)]
pub extern "C" fn pm_heap_reset_peak() {
    PEAK.store(LIVE.load(Ordering::Relaxed), Ordering::Relaxed);
}

/// Counters as JSON into the reply buffer.
#[unsafe(no_mangle)]
pub extern "C" fn pm_stats() -> usize {
    with(|s| {
        let st = s.engine.stats;
        let text = format!(
            "{{\"batches\":{},\"ops\":{},\"resyncs\":{},\"queries\":{},\"unanswered\":{},\"detachedHits\":{},\"detachedMisses\":{},\"revision\":{}}}",
            st.batches,
            st.ops,
            st.resyncs,
            st.queries,
            st.unanswered,
            st.detached_hits,
            st.detached_misses,
            s.engine.revision()
        );
        reply(s, &text, false)
    })
}

// `ValueKind` values cross the boundary as u8.
const _: () = {
    assert!(ValueKind::Number as u8 == 0);
    assert!(ValueKind::Other as u8 == 1);
    assert!(ValueKind::Threw as u8 == 2);
};
