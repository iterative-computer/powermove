//! Hidden tabs give their realms back when the shared heap account is full
//! (PM-513). Every window's built-in panels and extensions reserve from one
//! account; seventeen restored tabs held more than it has, and the tab the
//! person looked at could not start its panels. While anything waits for
//! room, the hidden tab least recently shown hibernates: its panels' and
//! extensions' realms close and their heap goes back. Shown again, it starts
//! them as its project's open does.
use super::Controller;
use gpui::{App, Entity};
use std::time::{Duration, Instant};

/// How often the account is looked at, and the least time between two
/// hibernations (a closing realm gives its heap back off the main thread).
const LOOK: Duration = Duration::from_millis(300);
const BETWEEN: Duration = Duration::from_millis(900);

/// Watches the account for the app's life (once, whoever calls first).
pub(crate) fn watch(cx: &mut App) {
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return;
    }
    cx.spawn(async move |cx| {
        let mut last: Option<Instant> = None;
        loop {
            cx.background_executor().timer(LOOK).await;
            if pm_runtime_v8::Admission::cage().waiting() == 0
                || last.is_some_and(|at| at.elapsed() < BETWEEN)
            {
                continue;
            }
            let Some(chosen) = cx.update(|cx| relieve(cx)) else {
                continue;
            };
            crate::diag::line(serde_json::json!({"hibernated": chosen}));
            last = Some(Instant::now());
        }
    })
    .detach();
}

/// One hidden tab hibernates: the least recently shown of those that can.
fn relieve(cx: &mut App) -> Option<u64> {
    let editors = crate::entry::editors(cx);
    let candidates: Vec<(Entity<Controller>, Option<Instant>)> = editors
        .into_iter()
        .filter(|e| e.read(cx).can_hibernate(cx))
        .map(|e| {
            let shown = e.read(cx).last_shown;
            (e, shown)
        })
        .collect();
    let chosen = least_recently_shown(candidates)?;
    let key = chosen.read(cx).window_key;
    chosen.update(cx, |v, cx| v.hibernate(cx)).then_some(key)
}

use pm_runtime_v8::least_recently_shown;

impl Controller {
    /// Whether this window may give its realms back now: hidden, its project
    /// open and settled, awake, and nothing in it that a restart would cut
    /// short (an export, a live agent run, a sheet, a panel window).
    pub(crate) fn can_hibernate(&self, cx: &App) -> bool {
        if self.hibernated
            || self.session.is_none()
            || self.opening
            || self.recovery_pending
            || pm_shell::window_shown(&self.host.borrow().window)
            || self.host.borrow().window.attachedSheet().is_some()
            || self.builtins.export_job.is_some()
            || !self.builtins.windows.is_empty()
        {
            return false;
        }
        let _ = cx;
        let Some(runtime) = &self.builtins.runtime else {
            return self.extensions.host.is_some();
        };
        let (working, attention, _, background) = runtime.agent_status();
        !(working || attention || background > 0)
    }
    /// Closes this hidden window's panels' and extensions' realms: their heap
    /// goes back to the account. Its project stays open, its viewer paused;
    /// its panels keep their places (the shell hears they are loading).
    pub(crate) fn hibernate(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        if !self.can_hibernate(cx) {
            return false;
        }
        self.tell_panels_ready(false);
        {
            let mut shell = self.host.borrow_mut();
            shell.native_present = None;
            shell.native_accessibility = vec![];
        }
        // Dropping the host closes the extensions' realm off the main thread.
        self.reset_extensions();
        if let Some(runtime) = self.builtins.runtime.take() {
            self.builtins.keep_agent(runtime.carry_agent());
            runtime.close();
            let carry = self.builtins.take_agent();
            self.builtins = crate::builtins::Host::default();
            self.builtins.keep_agent(carry);
        }
        self.hibernated = true;
        cx.notify();
        true
    }
    /// Shown again: its panels and extensions start as its project's open
    /// starts them.
    pub(crate) fn wake(&mut self, cx: &mut gpui::Context<Self>) {
        if !std::mem::take(&mut self.hibernated) {
            return;
        }
        crate::diag::line(serde_json::json!({"woke": self.window_key}));
        self.update_builtins(cx);
        cx.notify();
    }
}
