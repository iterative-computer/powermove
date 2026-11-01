//! TR-gated feed metadata only. No downloads, install, quit, signing or publish API.
use crate::error::Result;
use crate::service_ensure as ensure;
use semver::Version;
use serde::{Deserialize, Serialize};

pub const STARTUP_CHECK_MS: u64 = 30_000;
pub const CHECK_INTERVAL_MS: u64 = 4 * 60 * 60 * 1000;
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedMetadata {
    pub version: String,
    pub minimum_macos: u32,
    pub notes: Option<String>,
}
pub trait UpdateFeed {
    fn metadata(&mut self, route: FeedRoute) -> Result<Option<FeedMetadata>>;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    Gated,
    Current,
    Available(FeedMetadata),
}
pub fn check(
    gate: TransitionGate,
    current: &str,
    feed: &mut impl UpdateFeed,
) -> Result<UpdateStatus> {
    let Some(route) = gate.route() else {
        return Ok(UpdateStatus::Gated);
    };
    let current = Version::parse(current)?;
    let Some(metadata) = feed.metadata(route)? else {
        return Ok(UpdateStatus::Current);
    };
    ensure!(
        metadata
            .notes
            .as_ref()
            .is_none_or(|notes| notes.len() <= 64 * 1024),
        TooLarge; "feed notes too large"
    );
    let offered = Version::parse(&metadata.version)?;
    let prerelease_allowed = current.pre.as_str().starts_with("beta.");
    if metadata.minimum_macos > gate.macos_major
        || offered <= current
        || (!offered.pre.is_empty() && !prerelease_allowed)
    {
        return Ok(UpdateStatus::Current);
    }
    Ok(UpdateStatus::Available(metadata))
}
