//! wasm-bindgen exports for spike S5.
//!
//! The JS fast path (`js/evaluator.mjs`):
//! - numeric property ids from `intern`, answered by `ev` with no string
//!   crossing the boundary;
//! - batch results written into a `Float64Array` view of `out`, ids read
//!   from a `Uint32Array` view of `ids`, patches written into a `Uint8Array`
//!   view of `input`;
//! - every buffer only grows and keeps its contents when it does, and the JS
//!   side re-acquires its views whenever linear memory grows or a buffer moves.

use s5_core::Mirror;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Evaluator {
    mirror: Mirror,
    input: Vec<u8>,
    ids: Vec<u32>,
    out: Vec<f64>,
}

/// Grow-only buffer: length tracks capacity so contents survive a move.
fn reserve<T: Copy + Default>(buf: &mut Vec<T>, len: usize) -> *mut T {
    if buf.len() < len {
        let target = len.max(buf.len() * 2).max(64);
        buf.resize(target, T::default());
    }
    buf.as_mut_ptr()
}

#[wasm_bindgen]
impl Evaluator {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Evaluator {
        Evaluator {
            mirror: Mirror::new(),
            input: Vec::new(),
            ids: Vec::new(),
            out: Vec::new(),
        }
    }

    /// Address of an input buffer of at least `len` bytes. Bytes already
    /// written stay in place if the buffer has to move.
    pub fn input_ptr(&mut self, len: usize) -> *mut u8 {
        reserve(&mut self.input, len)
    }

    pub fn input_capacity(&self) -> usize {
        self.input.len()
    }

    /// Address of the id buffer (at least `len` u32s).
    pub fn ids_ptr(&mut self, len: usize) -> *mut u32 {
        reserve(&mut self.ids, len)
    }

    /// Address of the result buffer (at least `len` f64s).
    pub fn out_ptr(&mut self, len: usize) -> *mut f64 {
        reserve(&mut self.out, len)
    }

    /// Load a whole project from the first `len` input bytes (JSON).
    pub fn load(&mut self, len: usize) -> Result<(), JsError> {
        let bytes = self
            .input
            .get(..len)
            .ok_or_else(|| JsError::new("load length exceeds the input buffer"))?;
        self.mirror
            .load(bytes)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// Apply framed patches from the first `len` input bytes.
    pub fn apply(&mut self, len: usize) -> Result<(), JsError> {
        let bytes = self
            .input
            .get(..len)
            .ok_or_else(|| JsError::new("patch length exceeds the input buffer"))?;
        self.mirror
            .apply_framed(bytes)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    pub fn intern(&mut self, layer: &str, channel: &str) -> u32 {
        self.mirror.intern(layer, channel)
    }

    /// One property at global time `t` (seconds).
    pub fn ev(&mut self, id: u32, t: f64) -> f64 {
        self.mirror.ev(id, t)
    }

    /// `ids[0..n]` at time `t` into `out[0..n]`.
    pub fn ev_batch(&mut self, n: usize, t: f64) -> Result<(), JsError> {
        if n > self.ids.len() || n > self.out.len() {
            return Err(JsError::new("batch larger than the reserved buffers"));
        }
        self.mirror.ev_batch(&self.ids[..n], t, &mut self.out[..n]);
        Ok(())
    }

    /// World matrices of every layer (6 f64 each) into `out`. The caller
    /// reserves `out` for `6 * layer_count()` first.
    pub fn world_all(&mut self, t: f64) -> Result<usize, JsError> {
        let need = self.mirror.layer_count() * 6;
        if self.out.len() < need {
            return Err(JsError::new("out buffer smaller than 6 * layer_count"));
        }
        Ok(self.mirror.world_all(t, &mut self.out[..need]))
    }

    pub fn layer_count(&self) -> usize {
        self.mirror.layer_count()
    }

    pub fn handle_count(&self) -> usize {
        self.mirror.handle_count()
    }

    pub fn raw_bytes(&self) -> usize {
        self.mirror.raw_bytes()
    }

    /// Counters for the last load/apply:
    /// [patches, parsed, kept, added, removed, invalidated, bytesCompared].
    pub fn stats(&self) -> Vec<f64> {
        let s = self.mirror.stats();
        vec![
            s.patches as f64,
            s.layers_parsed as f64,
            s.layers_kept as f64,
            s.layers_added as f64,
            s.layers_removed as f64,
            s.handles_invalidated as f64,
            s.bytes_compared as f64,
        ]
    }

    /// The whole document as JSON (for the shadow-replay check).
    pub fn to_json(&self) -> String {
        String::from_utf8(self.mirror.to_json()).expect("mirror stores UTF-8")
    }

    /// Out-of-scope properties as "layer\tchannel\treason" lines.
    pub fn unsupported(&self) -> String {
        self.mirror
            .unsupported()
            .into_iter()
            .map(|(l, c, r)| format!("{l}\t{c}\t{r}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Default for Evaluator {
    fn default() -> Self {
        Self::new()
    }
}
