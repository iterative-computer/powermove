//! Worker-owned CPU execution of immutable recorder commands.
//!
//! Reuses the recorder's analytic float rasterizer (and pm-vector / pm-text),
//! avoiding a second, subtly different implementation of Canvas geometry.
//! See docs/native/recorder-raster.md for oracle tolerances and integration gates.
#![forbid(unsafe_code)]
mod shadow;
pub use pm_recorder as recorder;
use pm_recorder::{Command, CpuRasterizer, Error, ImageData, Result, SourceKey};
pub use shadow::{AlphaMask, Clip, Shadow};

/// A host owns one surface per stable canvas id, on its extension worker.
/// No main-thread callback, JavaScript engine, window or platform handle.
pub struct Rasterizer {
    key: SourceKey,
    width: u32,
    height: u32,
    surface: CpuRasterizer,
}
impl Rasterizer {
    pub fn new(id: u64, width: u32, height: u32) -> Result<Self> {
        Ok(Self {
            key: SourceKey { id, version: 0 },
            width,
            height,
            surface: CpuRasterizer::new(width, height)?,
        })
    }
    pub fn key(&self) -> SourceKey {
        self.key
    }
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    /// Append an unconsumed suffix. A stale producer cannot repeat or overwrite
    /// native work. Rejected identity/version/overflow leaves pixels unchanged.
    /// The resulting version counts drawing commands, as in pm-recorder.
    pub fn replay(&mut self, expected: SourceKey, commands: &[Command]) -> Result<SourceKey> {
        self.check(expected)?;
        let count = u64::try_from(commands.len()).map_err(|_| Error::Capacity)?;
        let next = self.key.version.checked_add(count).ok_or(Error::Capacity)?;
        self.surface.replay(commands)?;
        self.key.version = next;
        Ok(self.key)
    }
    /// Signed extents and transparent out-of-bounds pixels, independent of CTM,
    /// clipping, alpha and compositing. No float -> unorm conversion on replay.
    pub fn get_image_data(&self, sx: i32, sy: i32, sw: i32, sh: i32) -> Result<ImageData> {
        if sw == 0 || sh == 0 {
            return Err(Error::IndexSize);
        }
        if sx == 0
            && sy == 0
            && sw > 0
            && sh > 0
            && sw as u32 == self.width
            && sh as u32 == self.height
        {
            return self.surface.image_data();
        }
        let mut out = ImageData::new(sw.unsigned_abs(), sh.unsigned_abs())?;
        let x0 = i64::from(sx) + i64::from(sw.min(0));
        let y0 = i64::from(sy) + i64::from(sh.min(0));
        self.surface.read_into(x0, y0, &mut out);
        Ok(out)
    }
    pub fn image_data(&self) -> Result<ImageData> {
        self.surface.image_data()
    }
    /// Same-size resize clears the bitmap too. Failed admission retains it.
    pub fn resize(&mut self, expected: SourceKey, width: u32, height: u32) -> Result<SourceKey> {
        self.check(expected)?;
        let next = self.key.version.checked_add(1).ok_or(Error::Capacity)?;
        let surface = CpuRasterizer::new(width, height)?;
        self.surface = surface;
        self.width = width;
        self.height = height;
        self.key.version = next;
        Ok(self.key)
    }
    fn check(&self, expected: SourceKey) -> Result<()> {
        if expected != self.key {
            return Err(Error::VersionConflict);
        }
        Ok(())
    }
}
