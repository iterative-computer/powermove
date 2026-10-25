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
//! rules). The runner draws into the project's RGBA8, RGBA16F, or RGBA32F target
//! cleared to transparent with blending off, reads it back in GL row order (no flip, like the
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

pub(crate) fn color_depth(precision: pm_protocol::color::Precision) -> u8 {
    match precision {
        pm_protocol::color::Precision::Unorm8 => 8,
        pm_protocol::color::Precision::Float16 => 16,
        pm_protocol::color::Precision::Float32 => 32,
    }
}

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
        color_depth: 16,
        calls: calls.iter().map(|(n, c)| self::call(n, c)).collect(),
        stage: failure.stage.name().into(),
        message: failure.message.lines().next().unwrap_or_default().into(),
        textures: 0,
        units: Vec::new(),
        more_passes: Vec::new(),
    }
}

/// Binary16 values as little-endian bytes (the wire and file format), in
/// one pass without per-value allocation.
pub fn half_bytes(values: &[u16]) -> Vec<u8> {
    let mut out = vec![0u8; values.len() * 2];
    for (dst, v) in out.chunks_exact_mut(2).zip(values) {
        dst.copy_from_slice(&v.to_le_bytes());
    }
    out
}

/// Little-endian bytes as binary16 values (a trailing odd byte is ignored).
pub fn halves(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

/// Binary32 bit patterns as little-endian bytes without rounding or changing
/// NaN payloads.
pub fn float_bytes(values: &[u32]) -> Vec<u8> {
    let mut out = vec![0u8; values.len() * 4];
    for (dst, bits) in out.chunks_exact_mut(4).zip(values) {
        dst.copy_from_slice(&bits.to_le_bytes());
    }
    out
}

pub fn float_bits(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Feeds binary16 values to a hash as their little-endian bytes, a block at
/// a time (the digest of feeding them one by one).
fn hash_halves(h: &mut blake3::Hasher, values: &[u16]) {
    let mut block = [0u8; 64 * 1024];
    for chunk in values.chunks(block.len() / 2) {
        let bytes = &mut block[..chunk.len() * 2];
        for (dst, v) in bytes.chunks_exact_mut(2).zip(chunk) {
            dst.copy_from_slice(&v.to_le_bytes());
        }
        h.update(&*bytes);
    }
}

fn hash_floats(h: &mut blake3::Hasher, values: &[u32]) {
    let mut block = [0u8; 64 * 1024];
    for chunk in values.chunks(block.len() / 4) {
        let bytes = &mut block[..chunk.len() * 4];
        for (dst, bits) in bytes.chunks_exact_mut(4).zip(chunk) {
            dst.copy_from_slice(&bits.to_le_bytes());
        }
        h.update(&*bytes);
    }
}

/// Values per part hashed on a thread of its own (a full-HD texture is
/// four parts).
const PART: usize = 2 << 20;

/// A texture's digest: its values in parts of [`PART`] hashed side by side
/// (at most four threads), and the parts' digests hashed in order.
fn texture_digest(values: &[u16]) -> [u8; 32] {
    let parts: Vec<&[u16]> = values.chunks(PART).collect();
    let digests: Vec<[u8; 32]> = if parts.len() <= 1 {
        parts
            .iter()
            .map(|p| {
                let mut h = blake3::Hasher::new();
                hash_halves(&mut h, p);
                *h.finalize().as_bytes()
            })
            .collect()
    } else {
        let mut out = vec![[0u8; 32]; parts.len()];
        for (group, slots) in parts.chunks(4).zip(out.chunks_mut(4)) {
            std::thread::scope(|scope| {
                for (part, slot) in group.iter().zip(slots.iter_mut()) {
                    scope.spawn(move || {
                        let mut h = blake3::Hasher::new();
                        hash_halves(&mut h, part);
                        *slot = *h.finalize().as_bytes();
                    });
                }
            });
        }
        out
    };
    let mut h = blake3::Hasher::new();
    h.update(&(values.len() as u64).to_le_bytes());
    for d in &digests {
        h.update(d);
    }
    *h.finalize().as_bytes()
}

fn texture_digest32(values: &[u32]) -> [u8; 32] {
    let parts: Vec<&[u32]> = values.chunks(PART).collect();
    let digests: Vec<[u8; 32]> = if parts.len() <= 1 {
        parts
            .iter()
            .map(|p| {
                let mut h = blake3::Hasher::new();
                hash_floats(&mut h, p);
                *h.finalize().as_bytes()
            })
            .collect()
    } else {
        let mut out = vec![[0u8; 32]; parts.len()];
        for (group, slots) in parts.chunks(4).zip(out.chunks_mut(4)) {
            std::thread::scope(|scope| {
                for (part, slot) in group.iter().zip(slots.iter_mut()) {
                    scope.spawn(move || {
                        let mut h = blake3::Hasher::new();
                        hash_floats(&mut h, part);
                        *slot = *h.finalize().as_bytes();
                    });
                }
            });
        }
        out
    };
    let mut h = blake3::Hasher::new();
    h.update(&(values.len() as u64).to_le_bytes());
    for d in &digests {
        h.update(d);
    }
    *h.finalize().as_bytes()
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
    // BLAKE3 in parts on up to four threads: the key covers every texel of
    // every texture (two full-HD textures are 33 MB), hashed twice for
    // every frame of an animated step (asked for, then drawn); SHA-256 one
    // value at a time took 58 ms per request at full HD.
    let vertex = pm_glsl::VERTEX.to_owned();
    let mut h = blake3::Hasher::new();
    h.update(b"pm-render shader fallback with textures 3\0");
    h.update(vertex.as_bytes());
    h.update(&[0]);
    h.update(fragment.as_bytes());
    h.update(&[0]);
    h.update(&width.to_le_bytes());
    h.update(&height.to_le_bytes());
    for calls in passes {
        h.update(&(calls.len() as u32).to_le_bytes());
        h.update(&super::plan::calls_bytes(calls));
    }
    for u in &units {
        h.update(&u.unit.to_le_bytes());
        h.update(&u.texture.to_le_bytes());
        h.update(&[u8::from(u.chain)]);
    }
    for t in textures {
        h.update(&t.width.to_le_bytes());
        h.update(&t.height.to_le_bytes());
        if let Some(bits) = &t.rgba32f {
            h.update(&[32]);
            h.update(&texture_digest32(bits));
        } else {
            // Preserve the established 8/16-bit request keys and their cached corpus pixels.
            h.update(&texture_digest(&t.rgba16f));
        }
    }
    let key = h
        .finalize()
        .as_bytes()
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
        color_depth: 16,
        calls: passes.next().unwrap_or_default(),
        stage: failure.stage.name().into(),
        message: failure.message.lines().next().unwrap_or_default().into(),
        textures: textures.len() as u32,
        units,
        more_passes: passes.collect(),
    }
}

/// The plan does not know the project's depth. Bind it at execution, and
/// partition the fallback cache by depth because the same GLSL writes
/// different stored pixels into RGBA8 and RGBA16F targets.
pub fn at_color_depth(request: &FallbackRequest, depth: u8) -> FallbackRequest {
    if request.color_depth == depth {
        return request.clone();
    }
    let mut out = request.clone();
    out.color_depth = depth;
    let mut h = sha2::Sha256::new();
    use sha2::Digest;
    h.update(b"pm-render fallback color depth\0");
    h.update(request.key.as_bytes());
    h.update([depth]);
    out.key = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    out
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
    let row_bytes = pixels.width as usize * if pixels.rgba32f.is_some() { 16 } else { 8 };
    let per = (max_bytes / row_bytes.max(1)).max(1) as u32;
    let mut out = Vec::new();
    let mut row = 0;
    while row < pixels.height {
        let rows = per.min(pixels.height - row);
        let start = row as usize * pixels.width as usize * 4;
        let end = (row + rows) as usize * pixels.width as usize * 4;
        out.push((
            row,
            rows,
            match &pixels.rgba32f {
                Some(bits) => float_bytes(&bits[start..end]),
                None => half_bytes(&pixels.rgba16f[start..end]),
            },
        ));
        row += rows;
    }
    out
}

/// Why the runner could not draw a request (`ShaderFallbackMessage::Failed`,
/// or the host's error).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallbackFailure {
    /// `compile`, `deadline`, `contextLost`, `unavailable`, `size`,
    /// `pixelCode`, `tooLarge`, or `watchdog`.
    pub reason: String,
    pub message: String,
}

impl FallbackFailure {
    /// Whether the failure is deterministic, so it is not asked for again
    /// until the program's source changes: WebGL refused it (`compile`),
    /// its draw passed the runner's deadline (`watchdog`), or its target
    /// is larger than the GPU's WebGL texture limit (`tooLarge`; kept for
    /// the program at that size only, [`FallbackFailure::sized`]).
    /// Anything else (`contextLost`, the compile `deadline`, `unavailable`,
    /// `size`) is transient and retried.
    pub fn is_final(&self) -> bool {
        matches!(
            self.reason.as_str(),
            "compile" | "watchdog" | "tooLarge" | "pixelCode"
        )
    }

    /// Whether the failure holds only for the request's size (`tooLarge`):
    /// the same program at a smaller size (a lower preview resolution, a
    /// smaller export) is asked for.
    pub fn sized(&self) -> bool {
        self.reason == "tooLarge"
    }
}

/// The key a size-bound failure ([`FallbackFailure::sized`]) is kept under.
pub(crate) fn sized_program_id(program: &str, width: u32, height: u32) -> String {
    format!("{program}@{width}x{height}")
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
        let float32 = band.rgba32f.is_some();
        let fresh = || {
            (
                FallbackPixels {
                    width: w,
                    height: h,
                    rgba16f: if float32 {
                        Vec::new()
                    } else {
                        vec![0; w as usize * h as usize * 4]
                    },
                    rgba32f: float32.then(|| vec![0; w as usize * h as usize * 4]),
                },
                vec![false; h as usize],
            )
        };
        let entry = self.partial.entry(band.key.clone()).or_insert_with(&fresh);
        if entry.0.width != w || entry.0.height != h || entry.0.rgba32f.is_some() != float32 {
            *entry = fresh();
        }
        let row_values = w as usize * 4;
        let start = band.row as usize * row_values;
        let values = if float32 {
            band.rgba32f.as_ref()?.0.len() / 4
        } else {
            band.rgba16f.0.len() / 2
        };
        let end = start + values;
        if band.row + band.rows > h
            || values != band.rows as usize * row_values
            || end > w as usize * h as usize * 4
        {
            return None;
        }
        if let Some(bits) = &mut entry.0.rgba32f {
            bits[start..end].copy_from_slice(&float_bits(&band.rgba32f.as_ref()?.0));
        } else {
            entry.0.rgba16f[start..end].copy_from_slice(&halves(&band.rgba16f.0));
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

/// The runner's result: binary16 bits at depth 16, exact byte codes in u16
/// slots at depth 8, or binary32 bit patterns at depth 32, in GL row order.
#[derive(Debug, Clone, PartialEq)]
pub struct FallbackPixels {
    pub width: u32,
    pub height: u32,
    pub rgba16f: Vec<u16>,
    pub rgba32f: Option<Vec<u32>>,
}

impl FallbackPixels {
    pub fn bytes(&self) -> u64 {
        self.rgba16f.len() as u64 * 2 + self.rgba32f.as_ref().map_or(0, |v| v.len() as u64 * 4)
    }

    /// The first invalid RGBA8 code, if this request carries byte codes in
    /// u16 wire slots. A malformed answer must not be mistaken for a size
    /// mismatch or left pending indefinitely.
    pub fn invalid_rgba8_code(&self, request: &FallbackRequest) -> Option<u16> {
        (request.color_depth == 8)
            .then(|| self.rgba16f.iter().copied().find(|code| *code > 255))
            .flatten()
    }

    /// Checks the size against a request.
    pub fn fits(&self, request: &FallbackRequest) -> bool {
        let channels = u64::from(self.width) * u64::from(self.height) * 4;
        self.width == request.width
            && self.height == request.height
            && if request.color_depth == 32 {
                self.rgba16f.is_empty()
                    && self
                        .rgba32f
                        .as_ref()
                        .is_some_and(|bits| bits.len() as u64 == channels)
            } else {
                self.rgba32f.is_none() && self.rgba16f.len() as u64 == channels
            }
            && self.invalid_rgba8_code(request).is_none()
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
    /// Keys kept past the budget until [`FallbackStore::unpin`]: a job's
    /// frame keeps every result it was answered until it is delivered, so
    /// a chain of large steps (two 8K steps are past the budget) never
    /// drops one result for the next and asks for it again.
    pinned: Option<std::collections::HashSet<String>>,
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
            pinned: None,
        }
    }

    /// From now on, results inserted are kept past the budget (and are not
    /// dropped for later ones) until [`FallbackStore::unpin`].
    pub fn pin(&mut self) {
        self.pinned.get_or_insert_with(Default::default);
    }

    /// Ends [`FallbackStore::pin`]: the budget applies to every result
    /// again.
    pub fn unpin(&mut self) {
        self.pinned = None;
        self.trim(None);
    }

    pub fn insert(&mut self, key: String, pixels: FallbackPixels) {
        self.tick += 1;
        let size = pixels.bytes();
        if let Some(pinned) = &mut self.pinned {
            pinned.insert(key.clone());
        }
        if let Some((old, _)) = self
            .entries
            .insert(key.clone(), (Arc::new(pixels), self.tick))
        {
            self.bytes -= old.bytes();
        }
        self.bytes += size;
        self.trim(Some(&key));
    }

    /// Drops the least recently used results past the budget, never a
    /// pinned one or `keep` (the result just inserted).
    fn trim(&mut self, keep: Option<&str>) {
        while self.bytes > self.budget {
            let pinned = self.pinned.as_ref();
            let Some(oldest) = self
                .entries
                .iter()
                .filter(|(k, _)| {
                    Some(k.as_str()) != keep && !pinned.is_some_and(|p| p.contains(k.as_str()))
                })
                .min_by_key(|(_, (_, t))| *t)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
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

    #[test]
    fn rgba8_requests_partition_the_cache_and_accept_only_byte_codes() {
        let f16 = request(
            "layer".into(),
            "void main() {}".into(),
            1,
            1,
            &[],
            &crate::effects::exec::parity_failure(),
        );
        let u8_request = at_color_depth(&f16, 8);
        assert_eq!(f16.color_depth, 16);
        assert_eq!(u8_request.color_depth, 8);
        assert_ne!(u8_request.key, f16.key);
        assert_eq!(u8_request.key, at_color_depth(&f16, 8).key);
        let mut stored = pixels(1);
        stored.rgba16f = vec![0, 1, 127, 255];
        assert!(stored.fits(&u8_request));
        stored.rgba16f[0] = 256;
        assert!(!stored.fits(&u8_request));
        assert_eq!(stored.invalid_rgba8_code(&u8_request), Some(256));
        assert_eq!(stored.invalid_rgba8_code(&f16), None);
    }

    #[test]
    fn halves_are_hashed_and_converted_as_value_by_value() {
        let values: Vec<u16> = (0..200_003u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 16) as u16)
            .collect();
        let mut one = blake3::Hasher::new();
        for v in &values {
            one.update(&v.to_le_bytes());
        }
        let mut block = blake3::Hasher::new();
        hash_halves(&mut block, &values);
        assert_eq!(one.finalize(), block.finalize());
        let bytes = half_bytes(&values);
        assert_eq!(
            bytes,
            values
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<u8>>()
        );
        assert_eq!(halves(&bytes), values);
        // In parts: one value changed anywhere changes the digest.
        let big: Vec<u16> = (0..(PART * 5 + 7) as u32).map(|i| i as u16).collect();
        let d = texture_digest(&big);
        assert_eq!(d, texture_digest(&big), "deterministic");
        for at in [0, PART - 1, PART, PART * 4 + 3, big.len() - 1] {
            let mut other = big.clone();
            other[at] ^= 1;
            assert_ne!(texture_digest(&other), d, "{at}");
        }
        assert_ne!(texture_digest(&big[..big.len() - 1]), d);
    }

    #[test]
    fn float32_bits_survive_wire_bytes_and_partition_texture_keys() {
        let bits = [0x3f80_0008, 0xc010_0000, 0x38d1_b717, 0x3f80_0000];
        let bytes = float_bytes(&bits);
        assert_eq!(float_bits(&bytes), bits);
        let mut expected = blake3::Hasher::new();
        expected.update(&bytes);
        let mut actual = blake3::Hasher::new();
        hash_floats(&mut actual, &bits);
        assert_eq!(actual.finalize(), expected.finalize());
        let f16 = request(
            "layer".into(),
            "void main() {}".into(),
            1,
            1,
            &[],
            &crate::effects::exec::parity_failure(),
        );
        let f32 = at_color_depth(&f16, 32);
        let pixels = FallbackPixels {
            width: 1,
            height: 1,
            rgba16f: vec![],
            rgba32f: Some(bits.to_vec()),
        };
        assert!(pixels.fits(&f32));
        assert!(!pixels.fits(&f16));
        assert_ne!(f16.key, f32.key);
        let mut changed = pixels.clone();
        changed.rgba32f.as_mut().unwrap()[0] ^= 1;
        let key = |p: &FallbackPixels| {
            request_with_textures(
                "layer".into(),
                "void main() {}".into(),
                1,
                1,
                &[],
                vec![],
                &[p],
                &crate::effects::exec::parity_failure(),
            )
            .key
        };
        assert_ne!(key(&pixels), key(&changed));

        let large: Vec<u32> = (0..PART + 1).map(|i| i as u32).collect();
        let digest = texture_digest32(&large);
        let mut changed = large.clone();
        changed[PART] ^= 1;
        assert_ne!(texture_digest32(&changed), digest);
    }

    #[test]
    fn half_texture_request_key_matches_the_legacy_hash() {
        let pixels = FallbackPixels {
            width: 1,
            height: 1,
            rgba16f: vec![0x3c00, 0, 0, 0x3c00],
            rgba32f: None,
        };
        let fragment = "void main() {}";
        let request = request_with_textures(
            "layer".into(),
            fragment.into(),
            1,
            1,
            &[],
            vec![],
            &[&pixels],
            &crate::effects::exec::parity_failure(),
        );
        let mut legacy = blake3::Hasher::new();
        legacy.update(b"pm-render shader fallback with textures 3\0");
        legacy.update(pm_glsl::VERTEX.as_bytes());
        legacy.update(&[0]);
        legacy.update(fragment.as_bytes());
        legacy.update(&[0]);
        legacy.update(&1u32.to_le_bytes());
        legacy.update(&1u32.to_le_bytes());
        legacy.update(&pixels.width.to_le_bytes());
        legacy.update(&pixels.height.to_le_bytes());
        legacy.update(&texture_digest(&pixels.rgba16f));
        assert_eq!(request.key, legacy.finalize().to_hex().to_string());
    }

    fn pixels(n: u32) -> FallbackPixels {
        FallbackPixels {
            rgba32f: None,
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
    fn a_pinned_frame_keeps_every_result_past_the_budget_until_unpinned() {
        // Two results larger than the budget each (two 8K steps against
        // 512 MB): without pinning the second drops the first.
        let mut store = FallbackStore::with_budget(40);
        store.insert("old".into(), pixels(2));
        store.pin();
        store.insert("a".into(), pixels(4));
        store.insert("b".into(), pixels(4));
        assert!(store.contains("a") && store.contains("b"), "both kept");
        assert!(!store.contains("old"), "unpinned results still go first");
        store.unpin();
        assert!(store.contains("b") && !store.contains("a"));
        assert_eq!(store.bytes(), 32);
    }

    #[test]
    fn size_bound_failures_are_final_and_keyed_by_size() {
        let f = |reason: &str| FallbackFailure {
            reason: reason.into(),
            message: String::new(),
        };
        assert!(f("tooLarge").is_final() && f("tooLarge").sized());
        assert!(f("compile").is_final() && !f("compile").sized());
        assert!(f("pixelCode").is_final() && !f("pixelCode").sized());
        assert!(!f("size").is_final() && !f("contextLost").is_final());
        assert_eq!(sized_program_id("p", 8193, 4320), "p@8193x4320");
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
            rgba32f: None,
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
