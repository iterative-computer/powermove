//! Job-scoped activity candidates and explicit infinite-loop termination fixture.
use crate::server::State;
use objc2::{
    MainThreadMarker, MainThreadOnly, msg_send,
    rc::Retained,
    runtime::{Bool, ProtocolObject},
    sel,
};
use objc2_app_kit::{NSApplication, NSBackingStoreType, NSWindow, NSWindowStyleMask};
use objc2_foundation::{
    NSActivityOptions, NSObjectProtocol, NSPoint, NSProcessInfo, NSRect, NSSize, NSString,
};
use objc2_web_kit::{WKInactiveSchedulingPolicy, WKPreferences, WKWebView, WKWebViewConfiguration};
use serde_json::json;
use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

pub struct Controller {
    case: String,
    stage: u32,
    since: Instant,
    offscreen: Option<Retained<NSWindow>>,
    preferences: Retained<WKPreferences>,
    old: Option<(bool, bool)>,
    policy: Option<WKInactiveSchedulingPolicy>,
    activity: Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    kill_started: Option<Instant>,
    pid: i32,
    kill_sent: Option<Instant>,
}
impl Controller {
    pub fn new(case: String, config: &WKWebViewConfiguration, state: &State) -> Self {
        let preferences = unsafe { config.preferences() };
        let mut old = None;
        let modern = case.starts_with("lifecycle-s5b-") || case.starts_with("s5b-");
        let combined = case.contains("combined") || case.starts_with("s5b-");
        let mut policy = None;
        let mut activity = None;
        if modern && (case.ends_with("policy") || combined) {
            if preferences.respondsToSelector(sel!(setInactiveSchedulingPolicy:)) {
                unsafe {
                    policy = Some(preferences.inactiveSchedulingPolicy());
                    preferences.setInactiveSchedulingPolicy(WKInactiveSchedulingPolicy::None);
                }
                state.record(json!({"test":"inactive_policy_none","supported":true}));
            } else {
                state.record(json!({"test":"inactive_policy_none","supported":false}));
            }
        }
        if modern && (case.ends_with("activity") || combined) {
            activity = Some(
                NSProcessInfo::processInfo().beginActivityWithOptions_reason(
                    NSActivityOptions::UserInitiated | NSActivityOptions::LatencyCritical,
                    &NSString::from_str("S5b bounded synthetic export"),
                ),
            );
            state.record(json!({"test":"host_process_activity","scope":"UI process; not a WebContent assertion"}));
        }
        if case == "lifecycle-spi" || (modern && (case.ends_with("spi") || combined)) {
            let supported = preferences
                .respondsToSelector(sel!(_setPageVisibilityBasedProcessSuppressionEnabled:))
                && preferences.respondsToSelector(sel!(_setHiddenPageDOMTimerThrottlingEnabled:));
            if supported {
                unsafe {
                    let a: Bool =
                        msg_send![&*preferences, _pageVisibilityBasedProcessSuppressionEnabled];
                    let b: Bool = msg_send![&*preferences, _hiddenPageDOMTimerThrottlingEnabled];
                    old = Some((a.as_bool(), b.as_bool()));
                    let _: () = msg_send![&*preferences,_setPageVisibilityBasedProcessSuppressionEnabled:false];
                    let _: () =
                        msg_send![&*preferences,_setHiddenPageDOMTimerThrottlingEnabled:false];
                }
            }
            state.record(json!({"test":"activity_spi","supported":supported,"job_scoped":true}));
        }
        Self {
            case,
            stage: 0,
            since: Instant::now(),
            offscreen: None,
            preferences,
            old,
            policy,
            activity,
            kill_started: None,
            pid: 0,
            kill_sent: None,
        }
    }
    fn host(&mut self, mtm: MainThreadMarker, web: &WKWebView, visible: bool) {
        if self.offscreen.is_none() {
            let r = NSRect::new(
                NSPoint::new(-20000., -20000.),
                NSSize::new(
                    if self.case.starts_with("lifecycle-s5b-") {
                        1.
                    } else {
                        960.
                    },
                    if self.case.starts_with("lifecycle-s5b-") {
                        1.
                    } else {
                        640.
                    },
                ),
            );
            let w = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(mtm),
                    r,
                    NSWindowStyleMask::Borderless,
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            unsafe {
                w.setReleasedWhenClosed(false);
            }
            w.setCanHide(false);
            w.setIgnoresMouseEvents(true);
            self.offscreen = Some(w);
        }
        let w = self.offscreen.as_ref().unwrap();
        w.setContentView(Some(web));
        if visible {
            w.orderFront(None);
        } else {
            w.orderOut(None);
        }
    }
    pub fn tick(
        &mut self,
        mtm: MainThreadMarker,
        app: &NSApplication,
        window: &NSWindow,
        web: &WKWebView,
        s: &Arc<State>,
    ) {
        if self.case.starts_with("lifecycle-") {
            let visible = self.case == "lifecycle-offscreen" || self.case.ends_with("offscreen");
            if self.stage == 0 && s.life_ready.swap(false, Ordering::SeqCst) {
                window.miniaturize(None);
                if visible {
                    self.host(mtm, web, true);
                }
                app.hide(None);
                self.since = Instant::now();
                self.stage = 1;
                s.record(json!({"test":"long_minimise_started","minimised":window.isMiniaturized(),"app_hidden":app.isHidden(),"host_visible":self.offscreen.as_ref().is_some_and(|w|w.isVisible()),"dwell_s":60}));
            } else if self.stage == 1 && self.since.elapsed() >= Duration::from_secs(60) {
                s.life_phase.store(2, Ordering::SeqCst);
                self.stage = 2;
                s.record(json!({"test":"hidden_job_started","dwell_s":self.since.elapsed().as_secs_f64(),"app_hidden":app.isHidden(),"editor_minimised":window.isMiniaturized()}));
            } else if self.stage == 2 && s.life_ready.swap(false, Ordering::SeqCst) {
                self.host(mtm, web, visible);
                window.close();
                self.since = Instant::now();
                self.stage = 3;
            } else if self.stage == 3 && self.since.elapsed() >= Duration::from_secs(2) {
                s.life_phase.store(3, Ordering::SeqCst);
                self.stage = 4;
                s.record(json!({"test":"windowless_job_started","editor_visible":window.isVisible(),"host_visible":self.offscreen.as_ref().is_some_and(|w|w.isVisible()),"app_hidden":app.isHidden()}));
            }
        } else if self.case.starts_with("kill-") {
            if s.loop_started.load(Ordering::SeqCst) && self.kill_started.is_none() {
                self.kill_started = Some(Instant::now());
                if web.respondsToSelector(sel!(_webProcessIdentifier)) {
                    self.pid = unsafe { msg_send![web, _webProcessIdentifier] };
                }
                s.record(json!({"test":"real_loop_observed","pid":self.pid,"mode":self.case}));
            }
            if let Some(t) = self.kill_started {
                if self.stage == 0 && t.elapsed() >= Duration::from_secs(2) {
                    self.stage = 1;
                    let os = NSProcessInfo::processInfo().operatingSystemVersion();
                    let supported = os.majorVersion >= 14
                        && web.respondsToSelector(sel!(_webProcessIdentifier))
                        && web.respondsToSelector(if self.case == "kill-reset" {
                            sel!(_killWebContentProcessAndResetState)
                        } else {
                            sel!(_killWebContentProcess)
                        });
                    if self.case == "kill-reset" && !supported {
                        s.record(json!({"test":"hang_stop_unavailable","code":"compat_hang_stop_unavailable","os_major":os.majorVersion}));
                        s.failures.fetch_add(1, Ordering::SeqCst);
                        s.done.store(true, Ordering::SeqCst);
                        self.stage = 99;
                        return;
                    }
                    if self.case != "kill-fallback" && supported {
                        self.kill_sent = Some(Instant::now());
                        unsafe {
                            if self.case == "kill-reset" {
                                let _: () = msg_send![web, _killWebContentProcessAndResetState];
                            } else {
                                let _: () = msg_send![web, _killWebContentProcess];
                            }
                        }
                    }
                    s.record(json!({"test":"kill_requested","os_major":os.majorVersion,"os_minor":os.minorVersion,"os_patch":os.patchVersion,"spi_supported":supported,"used_spi":self.case!="kill-fallback"&&supported,"mode":self.case}));
                }
                if self.stage == 1
                    && (s.terminated.load(Ordering::SeqCst) || process_gone(self.pid))
                {
                    s.restarted.store(true, Ordering::SeqCst);
                    self.stage = 2;
                    s.record(json!({"test":"termination_observed","loop_elapsed_ms":t.elapsed().as_secs_f64()*1000.,"pid_exited":process_gone(self.pid),"kill_to_observation_ms":self.kill_sent.map(|t|t.elapsed().as_secs_f64()*1000.)}));
                    unsafe {
                        web.reload();
                    }
                }
                if self.stage == 2 && s.done.load(Ordering::SeqCst) {
                    let pid: i32 = unsafe { msg_send![web, _webProcessIdentifier] };
                    s.record(
                        json!({"test":"restart_pid","changed":pid!=self.pid&&pid>0,"pid":pid,"old_pid_exited":process_gone(self.pid),"kill_to_restart_ms":self.kill_sent.map(|t|t.elapsed().as_secs_f64()*1000.)}),
                    );
                    if !process_gone(self.pid) || pid == self.pid {
                        s.failures.fetch_add(1, Ordering::SeqCst);
                    }
                    self.stage = 3;
                }
                if self.stage < 2 && t.elapsed() >= Duration::from_secs(20) {
                    s.record(json!({"test":"kill_unverified","reason":"no termination callback within 20 seconds","mode":self.case}));
                    self.stage = 99;
                    s.failures.fetch_add(1, Ordering::SeqCst);
                    s.done.store(true, Ordering::SeqCst);
                }
            }
        }
    }
    pub fn finish(&mut self, s: &State) {
        if let Some(p) = self.policy.take() {
            unsafe {
                self.preferences.setInactiveSchedulingPolicy(p);
            }
            s.record(json!({"test":"inactive_policy_restored"}));
        }
        if let Some(a) = self.activity.take() {
            unsafe {
                NSProcessInfo::processInfo().endActivity(&a);
            }
            s.record(json!({"test":"host_activity_released"}));
        }
        if let Some((a, b)) = self.old.take() {
            unsafe {
                let _: () = msg_send![&*self.preferences,_setPageVisibilityBasedProcessSuppressionEnabled:a];
                let _: () = msg_send![&*self.preferences,_setHiddenPageDOMTimerThrottlingEnabled:b];
            }
            s.record(json!({"test":"activity_spi_released"}));
        }
        if let Some(w) = &self.offscreen {
            w.orderOut(None);
        }
    }
}

// Probe only the content PID owned by this WKWebView. Signal 0 sends no signal.
fn process_gone(pid: i32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
        fn __error() -> *mut i32;
    }
    pid > 0 && unsafe { kill(pid, 0) == -1 && *__error() == 3 }
}
