//! S111–S113, design/surfaces/updates.md. Pure state projection; the gated
//! distribution/Sparkle owner supplies update facts and performs every effect.
use super::{Button, View};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Preferences {
    #[serde(default)]
    pub dismissed: BTreeSet<String>,
    #[serde(default)]
    pub skipped: BTreeSet<String>,
    #[serde(flatten)]
    pub unknown: BTreeMap<String, serde_json::Value>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Verifying,
    Ready,
    Busy,
    CheckFailed,
    Offline,
    DownloadFailed,
    VerificationFailed,
}
#[derive(Debug)]
pub struct Model {
    pub state: State,
    pub manual: bool,
    pub current: String,
    pub offered: String,
    pub done: u64,
    pub total: Option<u64>,
    pub preferences: Preferences,
}
impl Model {
    pub fn new(current: String) -> Self {
        Self {
            state: State::Idle,
            manual: false,
            current,
            offered: String::new(),
            done: 0,
            total: None,
            preferences: Preferences::default(),
        }
    }
    pub fn dismiss_banner(&mut self) {
        if self.state == State::Ready {
            self.preferences.dismissed.insert(self.offered.clone());
        }
    }
    pub fn skip_version(&mut self) {
        if self.state == State::Available {
            self.preferences.skipped.insert(self.offered.clone());
            self.state = State::Idle;
        }
    }
    pub fn skipped(&self, version: &str) -> bool {
        self.preferences.skipped.contains(version)
    }
    pub fn library_notice(&self) -> bool {
        self.state == State::Ready
    }
    pub fn viewer_banner(&self) -> bool {
        self.state == State::Ready && !self.preferences.dismissed.contains(&self.offered)
    }
    pub fn menu(&self) -> &'static str {
        if self.state == State::Ready {
            "Restart to Install Update…"
        } else {
            "Check for Updates…"
        }
    }
    pub fn restart_after_barrier(
        &mut self,
        save_or_export_active: bool,
        quit_approved: bool,
    ) -> bool {
        self.state == State::Ready && !save_or_export_active && quit_approved
    }
    /// Dispatch the visible Cancel button. The verifying view has no button.
    pub fn cancel(&mut self) -> bool {
        if matches!(self.state, State::Checking | State::Downloading) {
            self.state = State::Idle;
            true
        } else {
            false
        }
    }
    /// Software Update ⌘W cancels, including the Verifying window, per S111.
    /// The host cancels its worker and retains staging bytes; this never un-stages
    /// a Ready update or bypasses archive/signature verification.
    pub fn close_window(&mut self) -> bool {
        if matches!(
            self.state,
            State::Checking | State::Downloading | State::Verifying
        ) {
            self.state = State::Idle;
            true
        } else {
            false
        }
    }
    pub fn view(&self) -> Option<View> {
        if self.state == State::Idle || (!self.manual && self.state != State::Ready) {
            return None;
        }
        let (title,line,buttons,default)=match self.state {
            State::Idle=>return None,
            State::Checking=>("Software Update".into(),"Checking for updates…".into(),vec![Button::new("cancel","Cancel")],Some("cancel")),
            State::UpToDate=>("Powermove is up to date.".into(),format!("You’re running {}.",self.current),vec![Button::new("ok","OK")],Some("ok")),
            State::Available=>("A New Version of Powermove Is Available".into(),format!("{} is now available. You have {}.",self.offered,self.current),vec![Button::new("skip_version","Skip This Version"),Button::new("remind_later","Remind Me Later"),Button::new("install","Install Update")],Some("install")),
            State::Downloading=>("Software Update".into(),format!("Downloading {}…",self.offered),vec![Button::new("cancel","Cancel")],Some("cancel")),
            State::Verifying=>("Software Update".into(),"Verifying…".into(),vec![],None),
            State::Ready if self.manual=>("An update is ready.".into(),"Quit Powermove normally to install it. Your session will be saved before quitting.".into(),vec![Button::new("ok","OK")],Some("ok")),
            State::Ready=>(format!("Powermove {} is ready to install",self.offered),String::new(),vec![Button::new("restart","Restart to Update")],Some("restart")),
            State::Busy=>("An update is already in progress.".into(),"Powermove will let you know when it’s ready.".into(),vec![Button::new("ok","OK")],Some("ok")),
            State::CheckFailed|State::Offline=>("Couldn’t check for updates.".into(),if self.state==State::Offline {"Check your internet connection, then try again."}else{"Try again later. You can keep using Powermove."}.into(),vec![Button::new("ok","OK")],Some("ok")),
            State::DownloadFailed=>("Couldn’t download the update.".into(),"Try again later. You can keep using Powermove.".into(),vec![Button::new("ok","OK")],Some("ok")),
            State::VerificationFailed=>("This update couldn’t be verified, so it wasn’t installed.".into(),"Try again later. You can keep using Powermove.".into(),vec![Button::new("ok","OK")],Some("ok")),
        };
        Some(View {
            surface: "S111",
            title,
            line,
            status: None,
            buttons,
            default,
        })
    }
    pub fn progress(&self) -> Option<(u64, u64)> {
        self.total
            .filter(|total| *total > 0 && self.done <= *total)
            .map(|total| (self.done, total))
    }
}
pub const MISSING_NOTES: &str = "Fixes and improvements.";
#[derive(Debug, Default)]
pub struct WhatsNew {
    pub seen: BTreeSet<String>,
    pub showing: Option<String>,
    pub pending: Option<String>,
}
impl WhatsNew {
    pub fn changed_version(&mut self, previous: Option<&str>, current: &str) {
        if previous.is_some_and(|previous| previous != current) && !self.seen.contains(current) {
            self.pending = Some(current.into());
        }
    }
    /// Mark when actually shown, once in one free project window. Unavailable
    /// notes retain the previous marker so another launch can retry.
    pub fn show(
        &mut self,
        version: &str,
        notes_available: bool,
        project_window: bool,
        onboarding_or_migration: bool,
        recovery_or_sheet: bool,
    ) -> bool {
        if self.pending.as_deref() != Some(version)
            || !notes_available
            || !project_window
            || onboarding_or_migration
            || recovery_or_sheet
            || self.showing.is_some()
            || self.seen.contains(version)
        {
            return false;
        }
        self.seen.insert(version.into());
        self.showing = Some(version.into());
        true
    }
    pub fn dismiss(&mut self) {
        self.showing = None;
    }
}
/// Dispatch intent only; it is not a claim that the macOS host is connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AboutAction {
    OrderFrontStandardAboutPanel,
}
pub fn about_action() -> AboutAction {
    AboutAction::OrderFrontStandardAboutPanel
}
