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

use serde::{Deserialize, Serialize};

use crate::effects::program::Call;

/// One `glUniform*` call as the runner replays it (the shader corpus's
/// format, `tools/effects/webgl.ts`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FallbackCall {
    /// `1f`, `2f`, `3f`, `4f`, `1i`, or `m3` (column-major).
    #[serde(rename = "fn")]
    pub function: String,
    pub name: String,
    pub v: Vec<f64>,
}

impl FallbackCall {
    pub fn of(name: &str, call: &Call) -> FallbackCall {
        let (function, v): (&str, Vec<f64>) = match call {
            Call::F(v) => (
                ["1f", "2f", "3f", "4f"][v.len().clamp(1, 4) - 1],
                v.iter().map(|x| f64::from(*x)).collect(),
            ),
            Call::I(i) => ("1i", vec![f64::from(*i)]),
            Call::M3(m) => ("m3", m.iter().map(|x| f64::from(*x)).collect()),
        };
        FallbackCall {
            function: function.into(),
            name: name.into(),
            v,
        }
    }
}

/// What the runner draws for one layer at one frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FallbackRequest {
    /// SHA-256 of everything below that decides the pixels (hex).
    pub key: String,
    pub layer: String,
    /// `PM.VERT`.
    pub vertex: String,
    /// The wrapped fragment program (shader-layer header and code, or the
    /// kernel's extension-layer program).
    pub fragment: String,
    pub width: u32,
    pub height: u32,
    pub calls: Vec<FallbackCall>,
    /// The native pipeline's failure (reported with the layer).
    pub stage: String,
    pub message: String,
}

impl FallbackRequest {
    pub fn new(
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
            calls: calls.iter().map(|(n, c)| FallbackCall::of(n, c)).collect(),
            stage: failure.stage.name().into(),
            message: failure.message.lines().next().unwrap_or_default().into(),
        }
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
        let c = FallbackCall::of("iResolution", &Call::F(vec![320.0, 180.0]));
        assert_eq!(
            serde_json::to_value(&c).unwrap(),
            serde_json::json!({"fn": "2f", "name": "iResolution", "v": [320.0, 180.0]})
        );
        assert_eq!(FallbackCall::of("iFrame", &Call::I(7)).function, "1i");
    }
}
