//! TR-gated feed metadata only. No downloads, install, quit, signing or publish API.
use crate::error::Result;
use crate::service_ensure as ensure;
use semver::Version;
use serde::{Deserialize, Serialize};

/// Verbatim update-sheet copy from design review item 4; rendering belongs to shell.
pub const UPDATE_SHEET_TITLE: &str = "A New Version of Powermove Is Available";
pub const INSTALL_UPDATE_LABEL: &str = "Install Update";
pub const REMIND_ME_LATER_LABEL: &str = "Remind Me Later";
pub const SKIP_THIS_VERSION_LABEL: &str = "Skip This Version";

pub const STARTUP_CHECK_MS: u64 = 30_000;
pub const CHECK_INTERVAL_MS: u64 = 4 * 60 * 60 * 1000;
pub use pm_update_policy::updater::{FeedRoute, TransitionGate};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedMetadata {
    pub version: String,
    /// Release display name supplied by the feed (e.g. "Powermove 2.1").
    pub name: String,
    /// Download size supplied by the feed; never derived from the notes length.
    pub size_bytes: u64,
    pub minimum_macos: u32,
    /// Untrusted Markdown, at most 64 KiB of UTF-8. Never execute HTML/scripts or
    /// load remote resources. Shell renders with pm_design::Tokens::typography.reading
    /// and reading_leading in the standard update sheet. Activate links through
    /// the host's safe external-browser opener, never by navigating an app panel.
    /// Install Update is the accent action; the other buttons use the labels above.
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
