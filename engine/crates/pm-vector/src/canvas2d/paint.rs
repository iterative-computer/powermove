//! Premultiplied solid paint for the legacy text path rasterizer.

#[derive(Debug, Clone)]
pub enum Paint {
    Solid([f32; 4]),
}
