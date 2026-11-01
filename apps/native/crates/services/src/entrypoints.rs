//! LaunchServices hooks and cold-start deep-link routing, without AppKit on Linux.
use anyhow::{Context, Result, ensure};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};
use url::Url;

/// Deliberately has no Debug/Serialize: callback state/token must never enter logs.
pub struct AuthLink {
    state: String,
    token: String,
}
impl AuthLink {
    pub fn state(&self) -> &str {
        &self.state
    }
    pub fn token(&self) -> &str {
        &self.token
    }
    pub fn parse(raw: &str) -> Result<Self> {
        ensure!(raw.len() <= 16 * 1024, "deep link too long");
        let url = Url::parse(raw).map_err(|_| anyhow::anyhow!("invalid auth link"))?;
        ensure!(
            url.scheme() == "powermove"
                && url.host_str() == Some("auth")
                && url.username().is_empty()
                && url.password().is_none()
                && url.port().is_none()
                && url.path().is_empty()
                && url.fragment().is_none(),
            "invalid auth link target"
        );
        let mut state = None;
        let mut token = None;
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "state" => {
                    ensure!(state.is_none(), "duplicate auth state");
                    state = Some(value.into_owned());
                }
                "token" => {
                    ensure!(token.is_none(), "duplicate auth token");
                    token = Some(value.into_owned());
                }
                _ => (),
            }
        }
        let state = state.context("missing auth state")?;
        let token = token.context("missing auth token")?;
        ensure!(
            !state.is_empty()
                && state.chars().count() <= 128
                && !token.is_empty()
                && token.chars().count() <= 4096
                && !state.chars().any(char::is_control)
                && !token.chars().any(char::is_control),
            "invalid auth callback fields"
        );
        Ok(Self { state, token })
    }
}
pub trait EntryHook {
    /// Auth owner validates pending state, expiry and PKCE before exchange.
    fn auth(&mut self, link: &AuthLink) -> Result<()>;
    fn open_project(&mut self, path: &Path) -> Result<()>;
    fn register_associations(&mut self, extension: &str, scheme: &str) -> Result<()>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkAdmission {
    Queued,
    ReplacedOldestAuth,
}
#[derive(Default)]
pub struct EntryQueue {
    links: VecDeque<AuthLink>,
    files: VecDeque<PathBuf>,
}
impl EntryQueue {
    /// Same eight newest auth callback bound as v1.2.0; explicit overflow for inspection.
    pub fn push_link(&mut self, raw: &str) -> Result<LinkAdmission> {
        let link = AuthLink::parse(raw)?;
        let admission = if self.links.len() >= 8 {
            self.links.pop_front();
            LinkAdmission::ReplacedOldestAuth
        } else {
            LinkAdmission::Queued
        };
        self.links.push_back(link);
        Ok(admission)
    }
    pub fn push_file(&mut self, path: PathBuf) -> Result<()> {
        ensure!(
            path.is_absolute()
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("pmv")),
            "expected absolute .pmv file"
        );
        if !self.files.contains(&path) {
            self.files.push_back(path);
        }
        Ok(())
    }
    /// Start only after first-launch/migration completes. On hook failure leave the
    /// failing entry and later entries queued; kernel/auth hooks must be idempotent.
    pub fn drain(&mut self, hook: &mut impl EntryHook) -> Result<()> {
        // Independent streams: a rejected auth callback cannot prevent a .pmv open.
        let auth = self.drain_auth(hook);
        let files = self.drain_files(hook);
        auth?;
        files
    }
    pub fn drain_auth(&mut self, hook: &mut impl EntryHook) -> Result<()> {
        while let Some(link) = self.links.front() {
            hook.auth(link)?;
            self.links.pop_front();
        }
        Ok(())
    }
    pub fn drain_files(&mut self, hook: &mut impl EntryHook) -> Result<()> {
        while let Some(path) = self.files.front() {
            hook.open_project(path)?;
            self.files.pop_front();
        }
        Ok(())
    }
    pub fn pending(&self) -> (usize, usize) {
        (self.links.len(), self.files.len())
    }
    pub fn register(
        packaged_macos: bool,
        explicit_dev_override: bool,
        hook: &mut impl EntryHook,
    ) -> Result<bool> {
        if !packaged_macos && !explicit_dev_override {
            return Ok(false);
        }
        hook.register_associations("pmv", "powermove")?;
        Ok(true)
    }
}
