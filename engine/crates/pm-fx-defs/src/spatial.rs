//! Pure sampling/resource calculations from main's effect-sampling contract.
//! These functions plan work; they do not assert that a GPU host binds it.
use crate::catalog::CatalogError;

pub fn sample_lod(
    radius: f64,
    support: f64,
    max_radius: f64,
    scale: [f64; 2],
    width: u32,
    height: u32,
) -> Result<u32, CatalogError> {
    if !support.is_finite()
        || support <= 0.
        || support > 8.
        || !max_radius.is_finite()
        || !(1. ..=64.).contains(&max_radius)
        || scale.iter().any(|s| !s.is_finite())
        || width == 0
        || height == 0
    {
        return Err(CatalogError("invalid spatial sampling contract".into()));
    }
    let extent = if radius.is_finite() {
        radius.abs() * support * scale[0].abs().max(scale[1].abs())
    } else {
        0.
    };
    let cap = f64::from(width.max(height)).log2().floor();
    Ok((extent / max_radius).max(1.).log2().ceil().max(0.).min(cap) as u32)
}

pub fn grid_size(columns: f64, rows: f64, max_size: u32) -> Result<[u32; 2], CatalogError> {
    if max_size == 0 {
        return Err(CatalogError("zero cell atlas limit".into()));
    }
    let limit = max_size.min(2048);
    Ok([columns, rows].map(|v| {
        let v = if v.is_finite() { (v + 0.5).floor() } else { 1. };
        v.max(1.).min(f64::from(limit)) as u32
    }))
}

/// Incremental mip allocation (base surface is already charged).
pub fn mip_bytes(width: u32, height: u32, depth: u8, levels: u32) -> Result<u64, CatalogError> {
    if !matches!(depth, 8 | 16 | 32) || width == 0 || height == 0 || levels > 31 {
        return Err(CatalogError("invalid mip allocation".into()));
    }
    let mut bytes = 0_u64;
    for level in 1..=levels {
        let w = u64::from((width >> level).max(1));
        let h = u64::from((height >> level).max(1));
        bytes = w
            .checked_mul(h)
            .and_then(|n| n.checked_mul(u64::from(depth / 2)))
            .and_then(|n| bytes.checked_add(n))
            .ok_or_else(|| CatalogError("mip allocation overflow".into()))?;
    }
    Ok(bytes)
}

/// Layer-local kernels measure affine basis lengths in output pixels. Source
/// owners use an identity world matrix before transform and opacity.
pub fn kernel_scale(world: Option<[f64; 6]>, scale: [f64; 2]) -> [f64; 2] {
    match world {
        Some(m) => [
            (m[0] * scale[0]).hypot(m[1] * scale[1]),
            (m[2] * scale[0]).hypot(m[3] * scale[1]),
        ],
        None => scale,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extent {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Extent {
    fn valid(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            && self.width >= 0.
            && self.height >= 0.
    }
    pub fn pad(self, pixels: f64) -> Result<Self, CatalogError> {
        if !self.valid() || !pixels.is_finite() || pixels < 0. {
            return Err(CatalogError("invalid source extent".into()));
        }
        let out = Self {
            x: self.x - pixels,
            y: self.y - pixels,
            width: self.width + 2. * pixels,
            height: self.height + 2. * pixels,
        };
        if out.valid() {
            Ok(out)
        } else {
            Err(CatalogError("source extent overflow".into()))
        }
    }
    /// Temporal samples and brush hulls expand the domain even off-composition.
    pub fn union(self, other: Self) -> Result<Self, CatalogError> {
        if !self.valid() || !other.valid() {
            return Err(CatalogError("invalid source extent".into()));
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let out = Self {
            x,
            y,
            width: (self.x + self.width).max(other.x + other.width) - x,
            height: (self.y + self.height).max(other.y + other.height) - y,
        };
        if out.valid() {
            Ok(out)
        } else {
            Err(CatalogError("source extent overflow".into()))
        }
    }
}
