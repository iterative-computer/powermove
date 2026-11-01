//! Frame identities are local to a viewer; document ids never recycle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Output {
    pub width: u32,
    pub height: u32,
    pub display: u32,
    pub headroom: f64,
}
impl Output {
    pub fn validate(self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.width > 0 && self.height > 0 && self.width <= 16384 && self.height <= 16384,
            "invalid output size"
        );
        anyhow::ensure!(
            self.headroom.is_finite() && self.headroom >= 1.,
            "invalid EDR headroom"
        );
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub generation: u64,
    pub doc_id: u64,
    pub revision: u64,
    pub sequence: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheKey {
    pub content: u64,
    pub revision: u64,
    pub time: i64,
    pub width: u32,
    pub height: u32,
}
impl CacheKey {
    pub fn same_content(self, other: Self) -> bool {
        self.content == other.content
            && self.revision == other.revision
            && self.width == other.width
            && self.height == other.height
    }
}
