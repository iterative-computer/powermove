//! Portable engine readback boundary shared by render-cache and scope workers.
//! The renderer adapter must decode engine E and unpremultiply in pm-color at
//! the explicit requested stage. It must not label E/premultiplied bytes as RGB.
use std::io;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphIdentity {
    /// Complete engine graph hash, including dependencies and versions.
    pub graph: [u8; 32],
    pub media: [u8; 32],
    pub renderer_version: String,
    /// Exact colour-stage/encoding identity; never inferred from the display.
    pub encoding: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameRequest {
    pub identity: GraphIdentity,
    pub ticks: i64,
    pub width: u32,
    pub height: u32,
}
impl FrameRequest {
    pub fn validate(&self) -> io::Result<()> {
        let bytes = u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .and_then(|n| n.checked_mul(16));
        if self.width == 0
            || self.height == 0
            || self.identity.renderer_version.is_empty()
            || self.identity.encoding.is_empty()
            || bytes.is_none_or(|n| n > 1024 * 1024 * 1024)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid frame request",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct EngineFrame {
    pub request: FrameRequest,
    /// Straight RGB at request.identity.encoding. Alpha is retained exactly.
    pub rgba: Vec<[f32; 4]>,
}
impl EngineFrame {
    pub fn validate(&self) -> io::Result<()> {
        self.request.validate()?;
        let r = &self.request;
        if r.width == 0
            || r.height == 0
            || r.identity.renderer_version.is_empty()
            || r.identity.encoding.is_empty()
            || (r.width as usize).checked_mul(r.height as usize) != Some(self.rgba.len())
            || self.rgba.len() > 1024 * 1024 * 1024 / 16
            || self
                .rgba
                .iter()
                .any(|p| p.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&p[3]))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid engine readback",
            ));
        }
        Ok(())
    }
}
/// Worker-only renderer/readback port. Identity must rotate on edits, resource
/// changes and relinks even without a document revision. Calls never wait on JS.
pub trait FrameSource {
    fn identity(&self) -> GraphIdentity;
    fn render(&mut self, request: &FrameRequest) -> io::Result<EngineFrame>;
}
