//! Worker render path: exact engine frames -> pm-color LUT -> immutable cache.
//! Scopes always analyze the explicitly requested RGB stage, including on hits.
use crate::{
    cache::{RenderCache, RenderKey},
    cache_frame::{EngineFrame, FrameRequest, FrameSource},
    lut::CubeLut,
    scopes::{self, ScopeConfig, Scopes},
    storage::invalid,
};
use std::io;

#[derive(Clone, Debug)]
pub struct LutTransform {
    lut: CubeLut,
    digest: [u8; 32],
    encoding: String,
}
impl LutTransform {
    /// The host supplies the authored LUT stage. No implicit transfer conversion
    /// or display-dependent choice. Content bytes, not a filename, identify it.
    pub fn parse(text: &str, encoding: String) -> io::Result<Self> {
        if encoding.is_empty() {
            return Err(invalid("missing LUT colour stage"));
        }
        Ok(Self {
            lut: CubeLut::parse(text).map_err(|e| invalid(&e.to_string()))?,
            digest: *blake3::hash(text.as_bytes()).as_bytes(),
            encoding,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisStage {
    BeforeLut,
    AfterLut,
}
#[derive(Clone, Copy, Debug)]
pub struct ScopeRequest {
    pub stage: AnalysisStage,
    pub config: ScopeConfig,
}
#[derive(Debug)]
pub struct RenderResult {
    pub frame: EngineFrame,
    pub key: RenderKey,
    pub cache_hit: bool,
    pub scopes: Option<Scopes>,
}
/// A frozen worker request. Kernel/host must also compare its identity on receipt
/// before presentation. Old keys remain archived/kept and cannot hit a new graph.
#[derive(Clone, Debug)]
pub struct RenderWork {
    request: FrameRequest,
    lut: Option<LutTransform>,
}
impl RenderWork {
    pub fn new(request: FrameRequest, lut: Option<LutTransform>) -> io::Result<Self> {
        request.validate()?;
        if lut
            .as_ref()
            .is_some_and(|lut| lut.encoding != request.identity.encoding)
        {
            return Err(invalid("LUT stage disagrees with engine readback"));
        }
        let work = Self { request, lut };
        work.key()?.digest()?;
        Ok(work)
    }
    pub fn key(&self) -> io::Result<RenderKey> {
        let mut graph = blake3::Hasher::new_derive_key("powermove pro-media render v1");
        graph.update(&self.request.identity.graph);
        let stage = serde_json::to_vec(&(
            &self.request.identity.encoding,
            self.lut.as_ref().map(|lut| lut.digest),
        ))?;
        graph.update(&stage);
        Ok(RenderKey {
            graph: *graph.finalize().as_bytes(),
            media: self.request.identity.media,
            ticks: self.request.ticks,
            width: self.request.width,
            height: self.request.height,
            format: "straight-rgba-f32le-v1".into(),
            renderer_version: self.request.identity.renderer_version.clone(),
        })
    }
    /// Kernel-side receipt fence. Pass the current engine identity and current
    /// RenderWork (including the current LUT), never the frozen worker version.
    pub fn accept(
        &self,
        result: RenderResult,
        current: &crate::cache_frame::GraphIdentity,
    ) -> io::Result<RenderResult> {
        if current != &self.request.identity
            || result.key != self.key()?
            || result.frame.request != self.request
        {
            return Err(invalid("stale render result at actor receipt"));
        }
        result.frame.validate()?;
        Ok(result)
    }
    pub fn is_current(&self, source: &dyn FrameSource) -> bool {
        source.identity() == self.request.identity
    }
    /// Fill only from a matching engine graph. A stale render cannot publish
    /// after the engine reports a changed identity; callers additionally fence
    /// late results at the actor boundary using their current publication.
    pub fn run(
        &self,
        source: &mut dyn FrameSource,
        cache: &RenderCache,
        scope: Option<ScopeRequest>,
    ) -> io::Result<RenderResult> {
        if !self.is_current(source) {
            return Err(invalid("stale render graph before work"));
        }
        let key = self.key()?;
        let cached = cache.get(&key)?;
        let cache_hit = cached.is_some();
        let needs_input = !cache_hit || scope.is_some_and(|s| s.stage == AnalysisStage::BeforeLut);
        let mut input = if needs_input {
            let frame = source.render(&self.request)?;
            frame.validate()?;
            if frame.request != self.request {
                return Err(invalid("engine returned a different frame"));
            }
            Some(frame)
        } else {
            None
        };
        let before = if let Some(s) = scope.filter(|s| s.stage == AnalysisStage::BeforeLut) {
            Some(analyze(input.as_ref().expect("requested input"), s.config)?)
        } else {
            None
        };
        let frame = if let Some(bytes) = cached {
            decode(&bytes, self.request.clone())?
        } else {
            let mut frame = input.take().expect("rendered input");
            if let Some(lut) = &self.lut {
                for p in &mut frame.rgba {
                    let rgb = lut
                        .lut
                        .apply([f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
                        .map_err(|e| invalid(&e.to_string()))?;
                    for c in 0..3 {
                        p[c] = rgb[c] as f32;
                    }
                }
            }
            frame.validate()?; // refuse f64 -> f32 overflow, preserve extended RGB
            frame
        };
        let scopes = if let Some(s) = scope.filter(|s| s.stage == AnalysisStage::AfterLut) {
            Some(analyze(&frame, s.config)?)
        } else {
            before
        };
        if !self.is_current(source) {
            return Err(invalid("stale render graph after work"));
        }
        if !cache_hit {
            let bytes: Vec<u8> = frame
                .rgba
                .iter()
                .flatten()
                .flat_map(|v| v.to_le_bytes())
                .collect();
            cache.put(&key, &bytes)?;
        }
        Ok(RenderResult {
            frame,
            key,
            cache_hit,
            scopes,
        })
    }
}
fn analyze(frame: &EngineFrame, config: ScopeConfig) -> io::Result<Scopes> {
    let rgb: Vec<_> = frame
        .rgba
        .iter()
        .map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
        .collect();
    scopes::compute(
        &rgb,
        frame.request.width as usize,
        frame.request.height as usize,
        config,
    )
    .map_err(|e| invalid(&format!("scope input: {e:?}")))
}
fn decode(bytes: &[u8], request: FrameRequest) -> io::Result<EngineFrame> {
    let expected = u64::from(request.width) * u64::from(request.height) * 16;
    if bytes.len() as u64 != expected {
        return Err(invalid("cached frame size mismatch"));
    }
    let rgba = bytes
        .chunks_exact(16)
        .map(|p| {
            std::array::from_fn(|i| {
                f32::from_le_bytes(p[i * 4..i * 4 + 4].try_into().expect("four bytes"))
            })
        })
        .collect();
    let frame = EngineFrame { request, rgba };
    frame.validate()?;
    Ok(frame)
}
