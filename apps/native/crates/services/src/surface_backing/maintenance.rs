//! S111/S112/S115. Tag: main/updates, main/whats-new, panels/PerfPanel.
use super::{Ticket, invalid, stale, ticket, unavailable};
use crate::{
    Result, ServiceErrorCode,
    updater::{self, FeedMetadata, TransitionGate, UpdateFeed, UpdateStatus},
};
use semver::Version;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateState {
    Idle,
    Gated,
    Checking,
    Current,
    Available(FeedMetadata),
    Downloading(FeedMetadata),
    Verifying(FeedMetadata),
    Cancelled,
    Ready {
        metadata: FeedMetadata,
        receipt: String,
    },
    Failed(ServiceErrorCode),
    DownloadFailed(ServiceErrorCode),
    VerificationFailed(ServiceErrorCode),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateStage {
    Download { done: u64, total: u64 },
    Verifying,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateRequest {
    pub ticket: Ticket,
    pub interactive: bool,
}
pub trait UpdateHost {
    /// Return only after native signature/version verification AND installer
    /// staging. A download event/optimistic flag is insufficient (N1/N4).
    fn stage_verified(
        &mut self,
        metadata: &FeedMetadata,
        report: &mut dyn FnMut(UpdateStage) -> Result<()>,
    ) -> Result<String>;
    /// Normal application quit/save barrier. false means cancelled; never force quit.
    fn request_normal_quit(&mut self) -> Result<bool>;
}
pub struct Updates {
    pub state: UpdateState,
    active: Option<UpdateRequest>,
    pub dismissed_version: Option<String>,
    pub skipped_version: Option<String>,
    pub download_progress: Option<(u64, u64)>,
}
impl Default for Updates {
    fn default() -> Self {
        Self {
            state: UpdateState::Idle,
            active: None,
            dismissed_version: None,
            skipped_version: None,
            download_progress: None,
        }
    }
}
impl Updates {
    pub fn begin_check(&mut self, interactive: bool) -> Result<UpdateRequest> {
        if self.active.is_some()
            || matches!(
                self.state,
                UpdateState::Ready { .. } | UpdateState::Downloading(_) | UpdateState::Verifying(_)
            )
        {
            return Err(unavailable("update work already active or ready"));
        }
        let request = UpdateRequest {
            ticket: ticket(),
            interactive,
        };
        self.active = Some(request.clone());
        self.state = UpdateState::Checking;
        Ok(request)
    }
    pub fn check(
        &mut self,
        request: &UpdateRequest,
        gate: TransitionGate,
        current: &str,
        feed: &mut impl UpdateFeed,
    ) -> Result<()> {
        if self.active.as_ref() != Some(request) {
            return Err(stale());
        }
        let result = updater::check(gate, current, feed);
        self.active = None;
        match result {
            Ok(UpdateStatus::Gated) => self.state = UpdateState::Gated,
            Ok(UpdateStatus::Current) => self.state = UpdateState::Current,
            Ok(UpdateStatus::Available(metadata)) => {
                self.state = if !request.interactive
                    && self.skipped_version.as_ref() == Some(&metadata.version)
                {
                    UpdateState::Current
                } else {
                    UpdateState::Available(metadata)
                }
            }
            Err(e) => {
                self.state = UpdateState::Failed(e.code);
                return Err(e);
            }
        }
        Ok(())
    }
    /// Runs on service worker; state is Downloading until verified staging replies.
    pub fn stage(&mut self, host: &mut impl UpdateHost) -> Result<()> {
        let UpdateState::Available(metadata) = &self.state else {
            return Err(invalid("no update available to stage"));
        };
        let metadata = metadata.clone();
        self.state = UpdateState::Downloading(metadata.clone());
        self.download_progress = None;
        let result = host.stage_verified(&metadata, &mut |stage| {
            match stage {
                UpdateStage::Download { done, total } => {
                    if !matches!(self.state, UpdateState::Downloading(_))
                        || (total != 0 && done > total)
                        || self.download_progress.is_some_and(|(old, old_total)| {
                            done < old || (old_total != 0 && total != old_total)
                        })
                    {
                        return Err(invalid("invalid update download progress"));
                    }
                    self.download_progress = Some((done, total));
                }
                UpdateStage::Verifying => self.state = UpdateState::Verifying(metadata.clone()),
            }
            Ok(())
        });
        match result {
            Ok(receipt)
                if !receipt.is_empty() && matches!(self.state, UpdateState::Verifying(_)) =>
            {
                self.state = UpdateState::Ready { metadata, receipt }
            }
            Ok(_) => {
                self.state = UpdateState::VerificationFailed(ServiceErrorCode::InvalidData);
                return Err(crate::ServiceError::message(
                    ServiceErrorCode::InvalidData,
                    "missing verified installer staging receipt",
                ));
            }
            Err(e) => {
                self.state = if matches!(self.state, UpdateState::Verifying(_)) {
                    UpdateState::VerificationFailed(e.code)
                } else {
                    UpdateState::DownloadFailed(e.code)
                };
                return Err(e);
            }
        }
        Ok(())
    }
    pub fn skip_available(&mut self) -> Result<String> {
        let UpdateState::Available(metadata) = &self.state else {
            return Err(invalid("no available update to skip"));
        };
        let version = metadata.version.clone();
        self.skipped_version = Some(version.clone());
        self.state = UpdateState::Idle;
        Ok(version)
    }
    pub fn remind_later(&mut self) -> Result<()> {
        if !matches!(self.state, UpdateState::Available(_)) {
            return Err(invalid("no update sheet"));
        }
        self.state = UpdateState::Idle;
        Ok(())
    }
    /// Returns a worker cancellation intent; verifying is deliberately not cancellable.
    /// Native host must stop/join the actual job before acknowledging cancellation.
    pub fn cancellation_allowed(&self) -> bool {
        matches!(
            self.state,
            UpdateState::Checking | UpdateState::Downloading(_)
        )
    }
    pub fn install(&self, host: &mut impl UpdateHost) -> Result<bool> {
        if !matches!(self.state, UpdateState::Ready { .. }) {
            return Err(invalid("update is not staged"));
        }
        host.request_normal_quit()
    }
    pub fn dismiss(&mut self) -> Result<String> {
        let UpdateState::Ready { metadata, .. } = &self.state else {
            return Err(invalid("no ready update notice"));
        };
        self.dismissed_version = Some(metadata.version.clone());
        Ok(metadata.version.clone())
    }
    pub fn notice(&self, home_open: bool) -> bool {
        matches!(&self.state, UpdateState::Ready { metadata, .. } if home_open || self.dismissed_version.as_ref() != Some(&metadata.version))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub draft: bool,
    pub prerelease: bool,
    pub metadata: Value,
}
fn app_version(raw: &str) -> Result<Version> {
    let version = Version::parse(raw.trim().strip_prefix('v').unwrap_or(raw.trim()))?;
    if !version.build.is_empty()
        || (!version.pre.is_empty()
            && !(version
                .pre
                .as_str()
                .strip_prefix("beta.")
                .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))))
    {
        return Err(invalid("unsupported app version"));
    }
    Ok(version)
}
pub fn releases_between(releases: &[Release], since: &str, current: &str) -> Result<Vec<Release>> {
    let since = app_version(since)?;
    let current = app_version(current)?;
    let mut accepted = Vec::new();
    for release in releases {
        if release.draft || (current.pre.is_empty() && release.prerelease) {
            continue;
        }
        let Ok(version) = app_version(&release.version) else {
            continue;
        };
        if version > since && version <= current {
            accepted.push((version, release.clone()));
        }
    }
    accepted.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(accepted.into_iter().map(|(_, r)| r).collect())
}
pub trait ReleaseHost {
    fn seen_version(&mut self) -> Result<Option<String>>;
    fn releases(&mut self) -> Result<Vec<Release>>;
    fn mark_seen(&mut self, version: &str) -> Result<()>;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseClaim {
    pub ticket: Ticket,
    pub window_id: String,
    pub current: String,
    pub releases: Vec<Release>,
}
#[derive(Default)]
pub struct WhatsNew {
    claim: Option<ReleaseClaim>,
}
impl WhatsNew {
    pub fn pending(
        &mut self,
        host: &mut impl ReleaseHost,
        window_id: &str,
        current: &str,
    ) -> Result<Option<ReleaseClaim>> {
        if self.claim.is_some() {
            return Ok(None);
        }
        app_version(current)?;
        let Some(since) = host.seen_version()? else {
            host.mark_seen(current)?;
            return Ok(None);
        };
        if app_version(current)? <= app_version(&since)? {
            return Ok(None);
        }
        let releases = releases_between(&host.releases()?, &since, current)?;
        if releases.is_empty() {
            host.mark_seen(current)?;
            return Ok(None);
        }
        let claim = ReleaseClaim {
            ticket: ticket(),
            window_id: window_id.into(),
            current: current.into(),
            releases,
        };
        self.claim = Some(claim.clone());
        Ok(Some(claim))
    }
    /// S112 marks seen when the owning sheet is actually shown, even if a later
    /// quit dismisses it. A failed marker write keeps the claim retryable.
    pub fn shown(&mut self, host: &mut impl ReleaseHost, claim: &ReleaseClaim) -> Result<()> {
        if self.claim.as_ref() != Some(claim) {
            return Err(stale());
        }
        host.mark_seen(&claim.current)?;
        self.claim = None;
        Ok(())
    }
    pub fn window_lost(&mut self, window_id: &str) {
        if self
            .claim
            .as_ref()
            .is_some_and(|c| c.window_id == window_id)
        {
            self.claim = None;
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Metric {
    Count(u64),
    Milliseconds(f64),
    PerSecond(f64),
    Unavailable(ServiceErrorCode),
}
#[derive(Clone, Debug, PartialEq)]
pub struct PerformanceSample {
    pub at_ms: u64,
    pub metrics: BTreeMap<String, Metric>,
}
#[derive(Default)]
pub struct Performance {
    pub sample: Option<PerformanceSample>,
    /// Set by panel lifecycle; S115 has no Pause control.
    pub visible: bool,
}
impl Performance {
    /// At most 2 Hz, as in the tag; no fake zeros/GL names for missing native counters.
    pub fn observe(&mut self, sample: PerformanceSample) -> Result<bool> {
        for metric in sample.metrics.values() {
            if matches!(metric, Metric::Milliseconds(n) | Metric::PerSecond(n) if !n.is_finite() || *n < 0.)
            {
                return Err(invalid("invalid native metric"));
            }
        }
        if !self.visible
            || self
                .sample
                .as_ref()
                .is_some_and(|old| sample.at_ms.saturating_sub(old.at_ms) < 500)
        {
            return Ok(false);
        }
        self.sample = Some(sample);
        Ok(true)
    }
}
