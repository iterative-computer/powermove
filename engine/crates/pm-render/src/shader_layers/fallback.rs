//! The decision 19 fallback runner, engine side (plan section 11: "a shader
//! that fails translation on a user's machine renders through a minimal
//! GLSL runner in the renderer (one WebGL2 context, the contract below, no
//! compositor), uploaded to the engine like the `raster` override; it is
//! reported and marked not headless-renderable").
//!
//! A program reaches the runner only after the WebGL-equivalent checks
//! passed (`pm_glsl`'s limits and ES validation: WebGL links it) and a later
//! stage failed (glslang, a SPIR-V rewrite, the bind check, naga, or Metal
//! pipeline creation), so the runner never renders a program WebGL refuses
//! (S3.md, "The runtime fallback"). Amendment 9's bind-check refusals
//! (samplers other than a float `sampler2D`, which WebGL samples as
//! (0, 0, 0, 1) because nothing is bound to them) are the known members.
//!
//! The engine describes the draw as a [`FallbackRequest`]: the compositor's
//! vertex shader, the wrapped fragment program, the target size, and the
//! uniform calls in issue order (the runner replays them with GL's own type
//! rules). The runner draws into an RGBA16F target cleared to transparent
//! with blending off, reads it back in GL row order (no flip, like the
//! engine's legacy-GLSL targets), and uploads [`FallbackPixels`]; the
//! renderer keeps them in its [`FallbackStore`] under the request's key and
//! draws them as the layer's content. A frame whose pixels have not arrived
//! fails with `RenderError::ShaderFallbackPending` and is retried once they
//! do. Hosts without a runner (the job engine, `pm-cli`, the hidden host)
//! never plan a fallback: the layer is `Feature::ShaderFallback` and the
//! frame is not headless-renderable.

use std::collections::HashMap;
use std::sync::Arc;

use crate::effects::program::Call;

pub use pm_protocol::shader_fallback::{
    ShaderFallbackCall as FallbackCall, ShaderFallbackInput as FallbackInputBand,
    ShaderFallbackPixels as FallbackBand, ShaderFallbackRequest as FallbackRequest,
    ShaderFallbackUnit as FallbackUnit,
};

/// The runner's form of one uniform call (the shader corpus's format,
/// `tools/effects/webgl.ts`).
pub fn call(name: &str, call: &Call) -> FallbackCall {
    let (function, v): (&str, Vec<f64>) = match call {
        Call::F(v) => (
            ["1f", "2f", "3f", "4f"][v.len().clamp(1, 4) - 1],
            v.iter().map(|x| f64::from(*x)).collect(),
        ),
        Call::I(i) => ("1i", vec![f64::from(*i)]),
        Call::M3(m) => ("m3", m.iter().map(|x| f64::from(*x)).collect()),
    };
    // `name|alias`: a parameter bound by name when that uniform is active,
    // else through its positional alias (`effects::exec::param_calls`).
    let (name, alias) = match name.split_once('|') {
        Some((n, a)) => (n, Some(a.to_owned())),
        None => (name, None),
    };
    FallbackCall {
        function: function.into(),
        name: name.into(),
        v,
        alias,
    }
}

/// The request for a program, its target, and its calls.
pub fn request(
    layer: String,
    fragment: String,
    width: u32,
    height: u32,
    calls: &[(String, Call)],
    failure: &pm_glsl::Failure,
) -> FallbackRequest {
    use sha2::{Digest, Sha256};
    let vertex = pm_glsl::VERTEX.to_owned();
    let mut h = Sha256::new();
    h.update(b"pm-render shader fallback 1\0");
    h.update(vertex.as_bytes());
    h.update([0]);
    h.update(fragment.as_bytes());
    h.update([0]);
    h.update(width.to_le_bytes());
    h.update(height.to_le_bytes());
    h.update(super::plan::calls_bytes(calls));
    let key = h
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    FallbackRequest {
        key,
        layer,
        vertex,
        fragment,
        width,
        height,
        calls: calls.iter().map(|(n, c)| self::call(n, c)).collect(),
        stage: failure.stage.name().into(),
        message: failure.message.lines().next().unwrap_or_default().into(),
        textures: 0,
        units: Vec::new(),
        more_passes: Vec::new(),
    }
}

/// The request for an effect's or a transition's program: one entry of
/// `passes` per pass (its calls), `units` naming which of `textures` each
/// texture unit samples, the textures the target's size. The key covers
/// the textures' pixels, so the same program on other pixels is another
/// request.
#[allow(clippy::too_many_arguments)]
pub fn request_with_textures(
    layer: String,
    fragment: String,
    width: u32,
    height: u32,
    passes: &[Vec<(String, Call)>],
    units: Vec<FallbackUnit>,
    textures: &[&FallbackPixels],
    failure: &pm_glsl::Failure,
) -> FallbackRequest {
    use sha2::{Digest, Sha256};
    let vertex = pm_glsl::VERTEX.to_owned();
    let mut h = Sha256::new();
    h.update(b"pm-render shader fallback with textures 1\0");
    h.update(vertex.as_bytes());
    h.update([0]);
    h.update(fragment.as_bytes());
    h.update([0]);
    h.update(width.to_le_bytes());
    h.update(height.to_le_bytes());
    for calls in passes {
        h.update((calls.len() as u32).to_le_bytes());
        h.update(super::plan::calls_bytes(calls));
    }
    for u in &units {
        h.update(u.unit.to_le_bytes());
        h.update(u.texture.to_le_bytes());
        h.update([u8::from(u.chain)]);
    }
    for t in textures {
        h.update(t.width.to_le_bytes());
        h.update(t.height.to_le_bytes());
        for v in &t.rgba16f {
            h.update(v.to_le_bytes());
        }
    }
    let key = h
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let mut passes = passes.iter().map(|calls| {
        calls
            .iter()
            .map(|(n, c)| self::call(n, c))
            .collect::<Vec<_>>()
    });
    FallbackRequest {
        key,
        layer,
        vertex,
        fragment,
        width,
        height,
        calls: passes.next().unwrap_or_default(),
        stage: failure.stage.name().into(),
        message: failure.message.lines().next().unwrap_or_default().into(),
        textures: textures.len() as u32,
        units,
        more_passes: passes.collect(),
    }
}

/// A program's identity for the runner's failures (its vertex and
/// fragment source): a program the runner could not draw is not asked for
/// again until its source changes.
pub fn program_id(request: &FallbackRequest) -> String {
    crate::effects::fallback::program_id_of(&request.vertex, &request.fragment)
}

/// Splits pixels into row bands that fit a message, at most `max_bytes`
/// each: `(row, rows, bytes)`.
pub fn bands(pixels: &FallbackPixels, max_bytes: usize) -> Vec<(u32, u32, Vec<u8>)> {
    let row_bytes = pixels.width as usize * 8;
    let per = (max_bytes / row_bytes.max(1)).max(1) as u32;
    let mut out = Vec::new();
    let mut row = 0;
    while row < pixels.height {
        let rows = per.min(pixels.height - row);
        let start = row as usize * pixels.width as usize * 4;
        let end = (row + rows) as usize * pixels.width as usize * 4;
        let bytes = pixels.rgba16f[start..end]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        out.push((row, rows, bytes));
        row += rows;
    }
    out
}

/// Why the runner could not draw a request (`ShaderFallbackMessage::Failed`,
/// or the host's error).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallbackFailure {
    /// `compile`, `deadline`, `contextLost`, `unavailable`, `size`, or
    /// `watchdog`.
    pub reason: String,
    pub message: String,
}

impl FallbackFailure {
    /// Whether the failure says something about the program, so it is not
    /// asked for again until its source changes: WebGL refused it
    /// (`compile`), or its draw passed the runner's deadline (`watchdog`).
    /// Anything else (`contextLost`, the compile `deadline`, `unavailable`,
    /// `size`) is transient and retried.
    pub fn is_final(&self) -> bool {
        matches!(self.reason.as_str(), "compile" | "watchdog")
    }
}

/// A request with the textures it samples, for a host that runs it
/// synchronously ([`FallbackHost`]).
#[derive(Debug, Clone)]
pub struct FallbackJob {
    pub request: FallbackRequest,
    pub textures: Vec<Arc<FallbackPixels>>,
}

/// A runner a renderer can wait on (the job engine's: the host runs the
/// request in a window's runner, contract 14). The interactive engine's
/// runner is asynchronous instead (`RenderError::ShaderFallbackPending`).
pub trait FallbackHost: Send + Sync {
    fn run(&self, job: &FallbackJob) -> Result<FallbackPixels, FallbackFailure>;
}

/// Collects a request's row bands (`ShaderFallbackMessage::Pixels`) into
/// its pixels.
#[derive(Debug, Default)]
pub struct Bands {
    partial: HashMap<String, (FallbackPixels, Vec<bool>)>,
}

impl Bands {
    /// Adds a band; returns the request's pixels once every row arrived.
    /// A band whose size disagrees with earlier ones restarts the request.
    pub fn add(&mut self, band: &FallbackBand) -> Option<(String, FallbackPixels)> {
        let (w, h) = (band.width, band.height);
        let entry = self.partial.entry(band.key.clone()).or_insert_with(|| {
            (
                FallbackPixels {
                    width: w,
                    height: h,
                    rgba16f: vec![0; w as usize * h as usize * 4],
                },
                vec![false; h as usize],
            )
        });
        if entry.0.width != w || entry.0.height != h {
            *entry = (
                FallbackPixels {
                    width: w,
                    height: h,
                    rgba16f: vec![0; w as usize * h as usize * 4],
                },
                vec![false; h as usize],
            );
        }
        let row_values = w as usize * 4;
        let start = band.row as usize * row_values;
        let values = band.rgba16f.0.len() / 2;
        let end = start + values;
        if band.row + band.rows > h
            || values != band.rows as usize * row_values
            || end > entry.0.rgba16f.len()
        {
            return None;
        }
        for (i, pair) in band.rgba16f.0.chunks_exact(2).enumerate() {
            entry.0.rgba16f[start + i] = u16::from_le_bytes([pair[0], pair[1]]);
        }
        for r in band.row..band.row + band.rows {
            entry.1[r as usize] = true;
        }
        if entry.1.iter().all(|r| *r) {
            let (pixels, _) = self.partial.remove(&band.key).expect("present");
            return Some((band.key.clone(), pixels));
        }
        None
    }

    /// Forgets a request (the runner failed it, or the session that asked
    /// for it closed).
    pub fn forget(&mut self, key: &str) {
        self.partial.remove(key);
    }

    /// Requests with some bands held.
    pub fn len(&self) -> usize {
        self.partial.len()
    }

    pub fn is_empty(&self) -> bool {
        self.partial.is_empty()
    }
}

/// The runner's result: RGBA16F (binary16 bits), `width × height × 4`
/// values, rows in GL order (row 0 at the bottom).
#[derive(Debug, Clone, PartialEq)]
pub struct FallbackPixels {
    pub width: u32,
    pub height: u32,
    pub rgba16f: Vec<u16>,
}

impl FallbackPixels {
    pub fn bytes(&self) -> u64 {
        self.rgba16f.len() as u64 * 2
    }

    /// Checks the size against a request.
    pub fn fits(&self, request: &FallbackRequest) -> bool {
        self.width == request.width
            && self.height == request.height
            && self.rgba16f.len() as u64 == u64::from(self.width) * u64::from(self.height) * 4
    }
}

/// Uploaded pixels by request key, least recently used dropped first past
/// the byte budget.
#[derive(Debug)]
pub struct FallbackStore {
    entries: HashMap<String, (Arc<FallbackPixels>, u64)>,
    bytes: u64,
    budget: u64,
    tick: u64,
}

impl Default for FallbackStore {
    fn default() -> Self {
        // A few seconds of a full-HD layer at 30 fps.
        FallbackStore::with_budget(1 << 30)
    }
}

impl FallbackStore {
    pub fn with_budget(budget: u64) -> FallbackStore {
        FallbackStore {
            entries: HashMap::new(),
            bytes: 0,
            budget,
            tick: 0,
        }
    }

    pub fn insert(&mut self, key: String, pixels: FallbackPixels) {
        self.tick += 1;
        let size = pixels.bytes();
        if let Some((old, _)) = self.entries.insert(key, (Arc::new(pixels), self.tick)) {
            self.bytes -= old.bytes();
        }
        self.bytes += size;
        while self.bytes > self.budget && self.entries.len() > 1 {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, (_, t))| *t)
                .map(|(k, _)| k.clone())
                .expect("not empty");
            if let Some((p, _)) = self.entries.remove(&oldest) {
                self.bytes -= p.bytes();
            }
        }
    }

    pub fn get(&mut self, key: &str) -> Option<Arc<FallbackPixels>> {
        self.tick += 1;
        let tick = self.tick;
        self.entries.get_mut(key).map(|(p, t)| {
            *t = tick;
            p.clone()
        })
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels(n: u32) -> FallbackPixels {
        FallbackPixels {
            width: n,
            height: 1,
            rgba16f: vec![0; n as usize * 4],
        }
    }

    #[test]
    fn the_store_drops_the_least_recently_used_past_its_budget() {
        let mut store = FallbackStore::with_budget(80);
        store.insert("a".into(), pixels(4));
        store.insert("b".into(), pixels(4));
        assert!(store.get("a").is_some());
        store.insert("c".into(), pixels(4));
        assert!(store.contains("a") && store.contains("c") && !store.contains("b"));
        assert_eq!(store.bytes(), 64);
    }

    #[test]
    fn calls_serialize_in_the_corpus_format() {
        let c = call("iResolution", &Call::F(vec![320.0, 180.0]));
        assert_eq!(
            serde_json::to_value(&c).unwrap(),
            serde_json::json!({"fn": "2f", "name": "iResolution", "v": [320.0, 180.0]})
        );
        assert_eq!(call("iFrame", &Call::I(7)).function, "1i");
    }

    #[test]
    fn bands_assemble_in_any_order() {
        let band = |row: u32, rows: u32, v: u8| FallbackBand {
            key: "k".into(),
            width: 2,
            height: 3,
            row,
            rows,
            rgba16f: pm_protocol::common::Bytes(vec![v; 2 * rows as usize * 8]),
        };
        let mut bands = Bands::default();
        assert!(bands.add(&band(2, 1, 3)).is_none());
        assert!(bands.add(&band(4, 1, 9)).is_none(), "past the last row");
        let (key, px) = bands.add(&band(0, 2, 1)).unwrap();
        assert_eq!(key, "k");
        assert_eq!(px.rgba16f.len(), 24);
        assert_eq!(px.rgba16f[0], 0x0101);
        assert_eq!(px.rgba16f[16], 0x0303);
        assert!(bands.is_empty());
        // A request forgotten mid-transfer lets its bands go.
        assert!(bands.add(&band(0, 1, 1)).is_none());
        assert_eq!(bands.len(), 1);
        bands.forget("k");
        assert!(bands.is_empty());
    }
}
