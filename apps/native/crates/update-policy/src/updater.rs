//! Pure feed-route admission, shared by services and signed policy.
#[derive(Debug, Clone, Copy, Default)]
pub struct TransitionGate {
    pub packaged_macos: bool,
    pub macos_major: u32,
    pub migration_complete: bool,
    /// Verified host receipt, never inferred from the OS or a feed response.
    pub transition_release_verified: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedRoute {
    ElectronCompatible,
    NativeCutover,
}
impl TransitionGate {
    pub fn route(self) -> Option<FeedRoute> {
        if !self.packaged_macos {
            return None;
        }
        if self.macos_major == 13 {
            return Some(FeedRoute::ElectronCompatible);
        }
        (self.macos_major >= 14 && self.migration_complete && self.transition_release_verified)
            .then_some(FeedRoute::NativeCutover)
    }
}
