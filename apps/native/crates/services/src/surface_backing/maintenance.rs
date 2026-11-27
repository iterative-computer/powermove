//! S112/S115. Tag: main/whats-new, panels/PerfPanel. Updates are Sparkle's (app `updates`).
use super::{Ticket, invalid, stale, ticket};
use crate::{Result, ServiceErrorCode};
use semver::Version;
use serde_json::Value;
use std::collections::BTreeMap;

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
